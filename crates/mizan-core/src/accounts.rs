//! The Accounts screen and the account page: one account's value, positions,
//! value at current units, history, returns, income, activity and balance
//! checks, built from the same calculations as Holdings and Performance.

use std::collections::{BTreeMap, HashMap};

use chrono::{Datelike, Duration, Months, NaiveDate};
use mizan_import::{ChargeType, Code, Date, Kind};
use mizan_storage::{AccountId, StorageError, Store};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::activity::{page as activity_page, ActivityQuery, ActivityRow};
use crate::consistency::minor_digits;
use crate::portfolio::{holdings_in, AccountHoldings, Period, Position};
use crate::replay::AssertionLine;
use crate::returns::calc::{self, Point};
use crate::returns::view::{account_points, Figure};
use crate::returns::{value_history, AccountSeries, PriceBook, ReturnsView};
use crate::tax::income::gross as income_gross;
use crate::valuation::{base_currency, Market};

/// Setting holding the account page's second currency.
pub const SECOND_CURRENCY_KEY: &str = "account.second_currency";
/// Currencies offered as the second currency.
pub const SECOND_CURRENCIES: [&str; 4] = ["TRY", "USD", "EUR", "GBP"];
/// Most points per position in the current-units chart.
pub const CHART_POINTS: usize = 400;
/// Most points in the value history chart.
pub const HISTORY_POINTS: usize = 500;

fn fmt(d: Decimal, ccy: &Code) -> String {
    let mut r = d.round_dp(minor_digits(ccy));
    if r.is_zero() {
        r = Decimal::ZERO;
    }
    r.rescale(minor_digits(ccy));
    r.to_string()
}

fn signed_pct(r: Decimal) -> String {
    let p = (r * Decimal::ONE_HUNDRED).round_dp(1);
    if p.is_sign_negative() && !p.is_zero() {
        format!("−{:.1}%", p.abs())
    } else {
        format!("+{:.1}%", p.abs())
    }
}

/// The second currency: the saved setting, else USD (EUR when the base is USD).
pub fn second_currency(store: &Store) -> Result<Code, StorageError> {
    let base = base_currency(store)?;
    let saved = store
        .setting(SECOND_CURRENCY_KEY)?
        .and_then(|c| Code::parse(c.trim()).ok())
        .filter(|c| SECOND_CURRENCIES.contains(&c.as_str()));
    Ok(saved.unwrap_or_else(|| {
        Code::parse(if base.as_str() == "USD" { "EUR" } else { "USD" }).expect("valid")
    }))
}

pub fn set_second_currency(store: &mut Store, code: &str) -> Result<(), StorageError> {
    let c = code.trim().to_uppercase();
    if !SECOND_CURRENCIES.contains(&c.as_str()) {
        return Err(StorageError::Invalid(format!(
            "{code:?} is not offered as a second currency"
        )));
    }
    store.set_setting(SECOND_CURRENCY_KEY, &c)
}

// ---------------------------------------------------------------- list

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CheckSummaryView {
    pub as_of: String,
    pub passed: bool,
    /// Failing lines, for example `ASML` or `cash EUR`.
    pub failed: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MergeInfo {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub merge_id: i64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub into_id: i64,
    pub into_name: String,
    pub merged_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AccountListRow {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub id: i64,
    pub name: String,
    /// Value in the base currency; `None` for merged accounts.
    pub value: Option<String>,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub activities: i64,
    pub identifiers: Vec<String>,
    /// `taxable`, `isa` or `pension`.
    pub tax_treatment: String,
    pub latest_check: Option<CheckSummaryView>,
    /// Set when the account was merged into another.
    pub merge: Option<MergeInfo>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AccountsList {
    pub base_currency: String,
    pub accounts: Vec<AccountListRow>,
    pub merged: Vec<AccountListRow>,
}

fn label_of(store: &Store, label: &str) -> Result<String, StorageError> {
    if Code::parse(label).is_ok() {
        return Ok(format!("cash {label}"));
    }
    Ok(store
        .list_assets()?
        .into_iter()
        .find(|a| a.instrument_key == label)
        .and_then(|a| a.hint.symbol.or(a.name))
        .unwrap_or_else(|| label.to_owned()))
}

fn check_view(
    store: &Store,
    r: &mizan_storage::StoredAssertionResult,
) -> Result<CheckSummaryView, StorageError> {
    let lines: Vec<AssertionLine> = serde_json::from_value(r.detail.clone()).unwrap_or_default();
    let mut failed = Vec::new();
    for l in lines.into_iter().filter(|l| !l.passed) {
        failed.push(label_of(store, &l.label)?);
    }
    Ok(CheckSummaryView {
        as_of: r.assertion.as_of.to_string(),
        passed: r.passed,
        failed,
    })
}

/// Every account, the ones merged into another listed apart.
pub fn accounts_list(
    store: &Store,
    market: &dyn Market,
    today: Date,
) -> Result<AccountsList, StorageError> {
    let base = base_currency(store)?;
    let hold = holdings_in(store, market, today)?;
    let values: HashMap<i64, String> = hold
        .accounts
        .iter()
        .map(|a| (a.account_id, a.total.amount.clone()))
        .collect();
    let merges = store.account_merges()?;
    let names: HashMap<i64, String> = store
        .list_all_accounts()?
        .into_iter()
        .map(|a| (a.id, a.name))
        .collect();
    let mut accounts = Vec::new();
    let mut merged = Vec::new();
    for a in store.list_all_accounts()? {
        // Latest check: newest date's results, all passed or not.
        let history = store.assertion_history(a.id)?;
        let latest_check = match history.first() {
            Some(first) => {
                let same: Vec<_> = history
                    .iter()
                    .filter(|r| r.assertion.as_of == first.assertion.as_of)
                    .collect();
                let mut v = check_view(store, first)?;
                v.passed = same.iter().all(|r| r.passed);
                for r in same.iter().skip(1) {
                    v.failed.extend(check_view(store, r)?.failed);
                }
                Some(v)
            }
            None => None,
        };
        let merge = merges
            .iter()
            .find(|m| m.from_account == a.id && m.undone_at.is_none())
            .map(|m| MergeInfo {
                merge_id: m.id,
                into_id: m.into_account,
                into_name: names.get(&m.into_account).cloned().unwrap_or_default(),
                merged_at: m.merged_at.clone(),
            });
        let row = AccountListRow {
            id: a.id,
            name: a.name.clone(),
            value: if merge.is_some() {
                None
            } else {
                Some(
                    values
                        .get(&a.id)
                        .cloned()
                        .unwrap_or_else(|| fmt(Decimal::ZERO, &base)),
                )
            },
            activities: store.all_activities(a.id)?.len() as i64,
            identifiers: a.hints.clone(),
            tax_treatment: a.tax_treatment.as_str().into(),
            latest_check,
            merge,
        };
        if row.merge.is_some() {
            merged.push(row);
        } else {
            accounts.push(row);
        }
    }
    Ok(AccountsList {
        base_currency: base.to_string(),
        accounts,
        merged,
    })
}

// ---------------------------------------------------------------- detail

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AmountView {
    pub amount: String,
    pub currency: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ChangeView {
    /// Signed amount in the base currency, for example `+9098.00`.
    pub amount: String,
    pub pct: String,
    /// The latest price date it runs to.
    pub date: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AccountSummaryView {
    pub base_currency: String,
    pub second_currency: String,
    /// Account value (positions plus cash) in the base currency.
    pub value: String,
    pub value_second: Option<String>,
    /// For example `USD/TRY 49.7872 on 2026-10-27`.
    pub rate_note: Option<String>,
    pub positions_value: String,
    /// Cash in the base currency (may be negative).
    pub cash_value: String,
    pub cash: Vec<AmountView>,
    pub cost: Option<String>,
    pub unrealized: Option<String>,
    pub last_day: Option<ChangeView>,
    pub one_year: Figure,
    pub one_year_second: Option<String>,
    pub ytd: Figure,
    /// For example "1 position left out: no price".
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct HoldingRow {
    pub key: String,
    pub symbol: Option<String>,
    pub name: Option<String>,
    pub units: String,
    pub price: Option<String>,
    pub price_currency: Option<String>,
    pub price_date: Option<String>,
    pub value: Option<String>,
    pub value_second: Option<String>,
    pub weight: Option<String>,
    pub cost: Option<String>,
    pub gain: Option<String>,
    pub day: Option<String>,
    pub month: Option<String>,
    pub year: Option<String>,
    /// Why a figure is missing.
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PeriodFigures {
    pub period: Period,
    pub from: String,
    pub twr: Figure,
    pub mwr: Figure,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct IncomeYear {
    pub year: i32,
    pub gross: String,
    pub withheld: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct HistoryPoint {
    pub date: String,
    pub value: Option<String>,
    pub contributions: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AccountChoice {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub id: i64,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AccountDetail {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub id: i64,
    pub name: String,
    pub identifiers: Vec<String>,
    pub tax_treatment: String,
    /// The provider this account gives its assets without a price source:
    /// `tefas`, `yahoo` or `twelvedata`; `None` leaves each asset to the user.
    pub price_source: Option<String>,
    /// Why Delete is not offered; `None` when it is.
    pub delete_blocker: Option<String>,
    /// Accounts this one can be merged into.
    pub merge_targets: Vec<AccountChoice>,
    pub summary: AccountSummaryView,
    pub holdings: Vec<HoldingRow>,
    /// Sum of the priced positions (cash excluded).
    pub total_value: String,
    pub total_value_second: Option<String>,
    pub returns: Vec<PeriodFigures>,
    pub history: Vec<HistoryPoint>,
    pub income: Vec<IncomeYear>,
    pub recent: Vec<ActivityRow>,
    pub checks: Vec<CheckSummaryView>,
}

/// What the account page needs, loaded once by the caller.
pub struct DetailInputs<'a> {
    pub store: &'a Store,
    pub market: &'a dyn Market,
    pub book: &'a PriceBook,
    /// Every account's daily series (from the returns engine).
    pub series: &'a [AccountSeries],
    pub returns: &'a ReturnsView,
    pub today: Date,
}

fn parse(s: &str) -> Option<Decimal> {
    s.parse().ok()
}

/// The newest price on or before the latest one shifted back by `back`, and
/// the latest; the return between them.
fn price_return(
    book: &PriceBook,
    key: &str,
    today: Date,
    back: impl Fn(NaiveDate) -> NaiveDate,
) -> Option<Decimal> {
    let latest = book.price_on(key, today)?;
    let earlier = book.price_on(key, Date(back(latest.date.0)))?;
    (earlier.date < latest.date && !earlier.value.is_zero())
        .then(|| latest.value / earlier.value - Decimal::ONE)
}

/// The price before the latest one (the previous price date).
fn previous_price(
    book: &PriceBook,
    key: &str,
    today: Date,
) -> Option<(Date, Decimal, Date, Decimal, Code)> {
    let prices = book.prices(key);
    let n = prices.partition_point(|p| p.date <= today);
    if n < 2 {
        return None;
    }
    let (p0, p1) = (&prices[n - 2], &prices[n - 1]);
    Some((p0.date, p0.value, p1.date, p1.value, p1.currency.clone()))
}

fn holding_row(
    i: &DetailInputs,
    p: &Position,
    base: &Code,
    second: &Code,
    keys: &crate::replay::KeyMap,
    positions_value: Decimal,
) -> HoldingRow {
    let key = keys
        .get(&p.instrument_key)
        .cloned()
        .unwrap_or_else(|| p.instrument_key.clone());
    let pct = |r: Option<Decimal>| r.map(signed_pct);
    let second_value = p.base_value.as_deref().and_then(parse).and_then(|v| {
        i.book
            .convert(v, base, second, i.today)
            .map(|x| fmt(x, second))
    });
    let note = p.value_note.clone().or(p.gain_note.clone());
    HoldingRow {
        key: key.clone(),
        symbol: p.symbol.clone(),
        name: p.name.clone(),
        units: p.quantity.clone(),
        price: p.price.as_ref().map(|x| x.value.clone()),
        price_currency: p.price.as_ref().map(|x| x.currency.clone()),
        price_date: p.price.as_ref().map(|x| x.date.clone()),
        value: p.base_value.clone(),
        value_second: second_value,
        // Share of the account's priced positions (cash left out).
        weight: p
            .base_value
            .as_deref()
            .and_then(parse)
            .filter(|_| !positions_value.is_zero())
            .map(|v| {
                format!(
                    "{:.1}%",
                    (v / positions_value * Decimal::ONE_HUNDRED).round_dp(1)
                )
            }),
        cost: p.cost.total.clone(),
        gain: p.gain.as_ref().map(|g| g.amount.clone()),
        day: pct(previous_price(i.book, &key, i.today)
            .and_then(|(_, p0, _, p1, _)| (!p0.is_zero()).then(|| p1 / p0 - Decimal::ONE))),
        month: pct(price_return(i.book, &key, i.today, |d| {
            d.checked_sub_months(Months::new(1)).unwrap_or(d)
        })),
        year: pct(price_return(i.book, &key, i.today, |d| {
            d.checked_sub_months(Months::new(12)).unwrap_or(d)
        })),
        note,
    }
}

fn period_row(
    returns: &ReturnsView,
    period: Period,
    account: AccountId,
) -> Option<&crate::returns::view::AccountReturns> {
    returns
        .periods
        .iter()
        .find(|p| p.period == period)
        .and_then(|p| p.accounts.iter().find(|a| a.account_id == account))
}

/// The account's time-weighted return in another currency over the same days.
fn twr_in(
    series: &AccountSeries,
    book: &PriceBook,
    base: &Code,
    to: &Code,
    from: Date,
    today: Date,
) -> Option<String> {
    let (start, points, _) = account_points(series, from, today);
    let conv = |v: Decimal, d: Date| book.convert(v, base, to, d);
    let start_day = Date(from.0 - Duration::days(1));
    let start = conv(start, start_day).unwrap_or(start * Decimal::ZERO);
    let mut out = Vec::new();
    for p in points {
        out.push(Point {
            date: p.date,
            value: conv(p.value, p.date)?,
            flow: conv(p.flow, p.date)?,
        });
    }
    match calc::twr(start, &out) {
        calc::Rate::Value { cumulative, .. } => Some(calc::pct(cumulative)),
        _ => None,
    }
}

fn income_years(
    i: &DetailInputs,
    id: AccountId,
    base: &Code,
) -> Result<Vec<IncomeYear>, StorageError> {
    let mut years: BTreeMap<i32, (Decimal, Decimal)> = BTreeMap::new();
    for s in i.store.all_activities(id)? {
        let r = &s.record;
        if !matches!(r.kind, Kind::Dividend | Kind::Distribution | Kind::Interest) {
            continue;
        }
        let Some(g) = income_gross(r) else { continue };
        let Some(gross) = i.book.convert(g.amount.value(), &g.currency, base, r.date) else {
            continue;
        };
        let withheld: Decimal = r
            .charges
            .iter()
            .filter(|c| c.charge_type == ChargeType::WithholdingTax)
            .filter_map(|c| {
                i.book
                    .convert(c.amount.amount.value(), &c.amount.currency, base, r.date)
            })
            .sum();
        let e = years.entry(r.date.0.year()).or_default();
        e.0 += gross;
        e.1 += withheld;
    }
    Ok(years
        .into_iter()
        .rev()
        .map(|(year, (g, w))| IncomeYear {
            year,
            gross: fmt(g, base),
            withheld: fmt(w, base),
        })
        .collect())
}

fn thin<T: Clone>(v: &[T], max: usize) -> Vec<T> {
    if v.len() <= max {
        return v.to_vec();
    }
    let step = v.len().div_ceil(max);
    let mut out: Vec<T> = v.iter().step_by(step).cloned().collect();
    if (v.len() - 1) % step != 0 {
        out.push(v[v.len() - 1].clone());
    }
    out
}

/// One account's page.
pub fn account_detail(i: &DetailInputs, id: AccountId) -> Result<AccountDetail, StorageError> {
    let store = i.store;
    let row = store
        .list_all_accounts()?
        .into_iter()
        .find(|a| a.id == id)
        .ok_or_else(|| StorageError::NotFound(format!("account {id}")))?;
    let base = base_currency(store)?;
    let second = second_currency(store)?;
    let keys = store.key_map()?;
    let hold = holdings_in(store, i.market, i.today)?;
    let empty = AccountHoldings {
        account_id: id,
        name: row.name.clone(),
        positions: vec![],
        cash: vec![],
        total: crate::portfolio::TotalView {
            currency: base.to_string(),
            amount: fmt(Decimal::ZERO, &base),
            excluded_no_price: 0,
            excluded_no_rate: 0,
            oldest_price: None,
        },
    };
    let acct = hold
        .accounts
        .iter()
        .find(|a| a.account_id == id)
        .unwrap_or(&empty);

    // Summary.
    let total = parse(&acct.total.amount).unwrap_or_default();
    let positions_value: Decimal = acct
        .positions
        .iter()
        .filter_map(|p| p.base_value.as_deref().and_then(parse))
        .sum();
    let cash_value: Decimal = acct
        .cash
        .iter()
        .filter_map(|c| c.base_value.as_deref().and_then(parse))
        .sum();
    let latest = acct
        .positions
        .iter()
        .filter_map(|p| p.price.as_ref().and_then(|x| Date::parse(&x.date).ok()))
        .max()
        .unwrap_or(i.today);
    let rate = i.book.convert(Decimal::ONE, &second, &base, latest);
    let value_second = i
        .book
        .convert(total, &base, &second, latest)
        .map(|v| fmt(v, &second));
    let rate_note = rate
        .filter(|_| second != base)
        .map(|r| format!("{second}/{base} {} on {latest}", r.round_dp(4).normalize()));
    let mut last_day_amount = Decimal::ZERO;
    let mut prev_total = Decimal::ZERO;
    let mut last_date: Option<Date> = None;
    for p in &acct.positions {
        let key = keys
            .get(&p.instrument_key)
            .cloned()
            .unwrap_or_else(|| p.instrument_key.clone());
        let Some(qty) = parse(&p.quantity) else {
            continue;
        };
        if let Some((_, p0, d1, p1, ccy)) = previous_price(i.book, &key, i.today) {
            if let (Some(a), Some(b)) = (
                i.book.convert(qty * (p1 - p0), &ccy, &base, d1),
                i.book.convert(qty * p0, &ccy, &base, d1),
            ) {
                last_day_amount += a;
                prev_total += b;
                last_date = Some(last_date.map_or(d1, |d| d.max(d1)));
            }
        }
    }
    let last_day = last_date
        .filter(|_| !prev_total.is_zero())
        .map(|d| ChangeView {
            amount: format!(
                "{}{}",
                if last_day_amount >= Decimal::ZERO {
                    "+"
                } else {
                    "−"
                },
                fmt(last_day_amount.abs(), &base)
            ),
            pct: signed_pct(last_day_amount / prev_total),
            date: d.to_string(),
        });
    let one_year_row = period_row(i.returns, Period::Last12Months, id);
    let ytd_row = period_row(i.returns, Period::Ytd, id);
    let na = |why: &str| Figure {
        value: None,
        annualized: None,
        reason: Some(why.into()),
        estimated_days: 0,
    };
    let series = i.series.iter().find(|s| s.account_id == id);
    let one_year_second = match (one_year_row, series) {
        (Some(r), Some(s)) if second != base => Date::parse(&r.from)
            .ok()
            .and_then(|from| twr_in(s, i.book, &base, &second, from, i.today)),
        _ => None,
    };
    let cost: Option<Decimal> = acct
        .positions
        .iter()
        .map(|p| {
            p.cost
                .total
                .as_deref()
                .and_then(parse)
                .filter(|_| p.cost.currency.as_deref() == Some(base.as_str()))
        })
        .sum();
    let unrealized = cost.map(|c| positions_value - c);
    let mut notes = Vec::new();
    if acct.total.excluded_no_price > 0 {
        let n = acct.total.excluded_no_price;
        notes.push(format!(
            "{n} position{} left out: no price",
            if n == 1 { "" } else { "s" }
        ));
    }
    if acct.total.excluded_no_rate > 0 {
        let n = acct.total.excluded_no_rate;
        notes.push(format!(
            "{n} amount{} left out: no exchange rate",
            if n == 1 { "" } else { "s" }
        ));
    }
    let summary = AccountSummaryView {
        base_currency: base.to_string(),
        second_currency: second.to_string(),
        value: fmt(total, &base),
        value_second: value_second.clone(),
        rate_note,
        positions_value: fmt(positions_value, &base),
        cash_value: fmt(cash_value, &base),
        cash: acct
            .cash
            .iter()
            .map(|c| AmountView {
                amount: c.amount.clone(),
                currency: c.currency.clone(),
            })
            .collect(),
        cost: cost.map(|c| fmt(c, &base)),
        unrealized: unrealized.map(|u| fmt(u, &base)),
        last_day,
        one_year: one_year_row
            .map(|r| r.twr.clone())
            .unwrap_or_else(|| na("no activity in the period")),
        one_year_second,
        ytd: ytd_row
            .map(|r| r.twr.clone())
            .unwrap_or_else(|| na("no activity in the period")),
        notes,
    };

    // Holdings, value history, returns, income, activity, checks.
    let holdings: Vec<HoldingRow> = acct
        .positions
        .iter()
        .map(|p| holding_row(i, p, &base, &second, &keys, positions_value))
        .collect();
    let history = match series {
        Some(s) => {
            let v = value_history(std::slice::from_ref(s), &base, i.today);
            let pts: Vec<HistoryPoint> = v
                .points
                .into_iter()
                .map(|p| HistoryPoint {
                    date: p.date,
                    value: p.value,
                    contributions: p.contributions,
                })
                .collect();
            thin(&pts, HISTORY_POINTS)
        }
        None => vec![],
    };
    let returns = Period::ALL
        .iter()
        .filter_map(|&p| {
            period_row(i.returns, p, id).map(|r| PeriodFigures {
                period: p,
                from: r.from.clone(),
                twr: r.twr.clone(),
                mwr: r.mwr.clone(),
            })
        })
        .collect();
    let recent = activity_page(
        store,
        &ActivityQuery {
            accounts: vec![id],
            ..Default::default()
        },
        0,
    )?
    .rows
    .into_iter()
    .take(20)
    .collect();
    let mut checks = Vec::new();
    for r in store.assertion_history(id)? {
        checks.push(check_view(store, &r)?);
    }
    let merge_targets = store
        .list_accounts()?
        .into_iter()
        .filter(|a| a.id != id)
        .map(|a| AccountChoice {
            id: a.id,
            name: a.name,
        })
        .collect();
    Ok(AccountDetail {
        id,
        name: row.name,
        identifiers: row.hints,
        tax_treatment: row.tax_treatment.as_str().into(),
        price_source: store.account_price_source(id)?,
        delete_blocker: store.delete_blocker(id)?,
        merge_targets,
        total_value: fmt(positions_value, &base),
        total_value_second: i
            .book
            .convert(positions_value, &base, &second, latest)
            .map(|v| fmt(v, &second)),
        summary,
        holdings,
        returns,
        history,
        income: income_years(i, id, &base)?,
        recent,
        checks,
    })
}

// ---------------------------------------------------------------- current units

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ChartPeriod {
    OneMonth,
    ThreeMonths,
    SixMonths,
    Ytd,
    OneYear,
    ThreeYears,
    FiveYears,
}

impl ChartPeriod {
    fn start(&self, today: Date) -> Date {
        let back = |m: u32| {
            Date(
                today
                    .0
                    .checked_sub_months(Months::new(m))
                    .unwrap_or(today.0),
            )
        };
        match self {
            ChartPeriod::OneMonth => back(1),
            ChartPeriod::ThreeMonths => back(3),
            ChartPeriod::SixMonths => back(6),
            ChartPeriod::Ytd => Date(NaiveDate::from_ymd_opt(today.0.year(), 1, 1).expect("valid")),
            ChartPeriod::OneYear => back(12),
            ChartPeriod::ThreeYears => back(36),
            ChartPeriod::FiveYears => back(60),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct UnitsBand {
    pub key: String,
    pub name: String,
    /// One value per date, `None` before the position's first price.
    pub values: Vec<Option<String>>,
    /// The position's prices start after the period does.
    pub short_history: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CurrentUnitsChart {
    pub currency: String,
    pub period: ChartPeriod,
    pub dates: Vec<String>,
    pub bands: Vec<UnitsBand>,
    /// Today's weight per position (same order as `bands`), in percent.
    pub weights: Vec<String>,
}

/// Today's units of each position valued at each day's prices, in `currency`.
pub fn current_units_chart(
    store: &Store,
    market: &dyn Market,
    book: &PriceBook,
    id: AccountId,
    period: ChartPeriod,
    currency: &str,
    today: Date,
) -> Result<CurrentUnitsChart, StorageError> {
    let base = base_currency(store)?;
    let ccy = Code::parse(currency.trim().to_uppercase().as_str())
        .ok()
        .filter(|c| *c == base || SECOND_CURRENCIES.contains(&c.as_str()))
        .ok_or_else(|| {
            StorageError::Invalid(format!("{currency:?} is not the base or a second currency"))
        })?;
    let keys = store.key_map()?;
    let hold = holdings_in(store, market, today)?;
    let positions: Vec<&Position> = hold
        .accounts
        .iter()
        .find(|a| a.account_id == id)
        .map(|a| {
            a.positions
                .iter()
                .filter(|p| parse(&p.quantity).is_some_and(|q| q > Decimal::ZERO))
                .collect()
        })
        .unwrap_or_default();
    let start = period.start(today);
    let mut days = Vec::new();
    let mut d = start;
    while d <= today {
        days.push(d);
        d = Date(d.0 + Duration::days(1));
    }
    let days = thin(&days, CHART_POINTS);
    let mut bands = Vec::new();
    let mut weights = Vec::new();
    for p in positions {
        let key = keys
            .get(&p.instrument_key)
            .cloned()
            .unwrap_or_else(|| p.instrument_key.clone());
        let qty = parse(&p.quantity).unwrap_or_default();
        let first = book.prices(&key).first().map(|x| x.date);
        let values = days
            .iter()
            .map(|day| {
                let price = book.price_on(&key, *day)?;
                book.convert(qty * price.value, &price.currency, &ccy, *day)
                    .map(|v| fmt(v, &ccy))
            })
            .collect();
        bands.push(UnitsBand {
            key,
            name: p.symbol.clone().or(p.name.clone()).unwrap_or_default(),
            values,
            short_history: first.is_none_or(|f| f > start),
        });
        weights.push(p.weight.clone().unwrap_or_default());
    }
    Ok(CurrentUnitsChart {
        currency: ccy.to_string(),
        period,
        dates: days.iter().map(|d| d.to_string()).collect(),
        bands,
        weights,
    })
}
