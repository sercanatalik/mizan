//! Read models for the app screens, computed from stored activities only.

pub mod cost;

use std::collections::{BTreeMap, HashMap};

use mizan_import::{ClassHint, Code, Date, ImportRecord, Kind};
use mizan_storage::{AccountId, AccountRow, AssetRow, StorageError, Store, StoredActivity};
use rust_decimal::Decimal;
use serde::Serialize;

use crate::consistency::minor_digits;
use crate::ordering::{order, OrderInput};
use crate::replay::{AssertionLine, Balances};
use crate::resolve::instrument_key;
use cost::{CostBook, CostStatus};

/// An account's activities in deterministic processing order.
pub fn ordered(stored: &[StoredActivity]) -> Vec<&ImportRecord> {
    let inputs: Vec<OrderInput> = stored
        .iter()
        .map(|s| OrderInput {
            account: s.account_id.to_string(),
            date: s.record.date,
            file: s.record.source.file_sha256.as_str().to_owned(),
            source_order: s.source_order,
            kind: s.record.kind,
            instrument_key: s.record.instrument.as_ref().and_then(instrument_key),
            locator: s.record.source.locator.clone(),
        })
        .collect();
    order(&inputs)
        .0
        .into_iter()
        .map(|i| &stored[i].record)
        .collect()
}

fn money_str(d: Decimal, ccy: &Code) -> String {
    d.round_dp(minor_digits(ccy)).to_string()
}

fn class_name(c: Option<ClassHint>) -> String {
    match c {
        Some(c) => serde_json::to_value(c)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default(),
        None => "other".into(),
    }
}

// ---------------------------------------------------------------- holdings

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum CostState {
    Known,
    UnknownBasis,
    MixedCurrency,
    Short,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CostView {
    pub state: CostState,
    /// Provisional total cost; only when `state` is `known`.
    pub total: Option<String>,
    /// Provisional average cost per unit; only when `state` is `known`.
    pub average: Option<String>,
    pub currency: Option<String>,
    /// Units without a cost basis, when there are any.
    pub unknown_units: Option<String>,
}

/// The account's latest balance check disagreed with this position.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CheckFlag {
    pub statement_quantity: String,
    pub computed_quantity: String,
    pub as_of: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub batch_id: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Position {
    pub instrument_key: String,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub asset_id: Option<i64>,
    pub symbol: Option<String>,
    pub name: Option<String>,
    pub isin: Option<String>,
    /// Asset class from the class hint, `other` when absent.
    pub class: String,
    /// Instrument currency hint, otherwise the cost currency.
    pub currency: Option<String>,
    pub quantity: String,
    pub cost: CostView,
    pub check_flag: Option<CheckFlag>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CashLine {
    pub currency: String,
    pub amount: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AccountHoldings {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub account_id: i64,
    pub name: String,
    pub positions: Vec<Position>,
    pub cash: Vec<CashLine>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Holdings {
    pub accounts: Vec<AccountHoldings>,
}

fn failed_lines(
    store: &Store,
    account: AccountId,
) -> Result<Vec<(String, AssertionLine, i64)>, StorageError> {
    let mut out = Vec::new();
    for r in store.latest_assertion_results(account)? {
        let lines: Vec<AssertionLine> =
            serde_json::from_value(r.detail.clone()).unwrap_or_default();
        for l in lines.into_iter().filter(|l| !l.passed) {
            out.push((r.assertion.as_of.to_string(), l, r.batch_id));
        }
    }
    Ok(out)
}

/// Positions and cash per account from all stored activities.
pub fn holdings(store: &Store) -> Result<Holdings, StorageError> {
    let assets: HashMap<String, AssetRow> = store
        .list_assets()?
        .into_iter()
        .map(|a| (a.instrument_key.clone(), a))
        .collect();
    let mut accounts = Vec::new();

    for AccountRow { id, name, .. } in store.list_accounts()? {
        let stored = store.all_activities(id)?;
        let recs = ordered(&stored);
        let mut balances = Balances::default();
        let mut book = CostBook::default();
        let mut last_hint = HashMap::new();
        for r in &recs {
            balances.apply(r);
            book.apply(r);
            if let Some(h) = &r.instrument {
                if let Some(k) = instrument_key(h) {
                    last_hint.insert(k, h.clone());
                }
            }
        }
        let failed = failed_lines(store, id)?;

        let mut positions = Vec::new();
        for (key, qty) in &balances.positions {
            if qty.is_zero() {
                continue;
            }
            let lot = book.lots.get(key).cloned().unwrap_or_default();
            let asset = assets.get(key);
            // Stored asset hint first, gaps filled from the most recent activity's hint.
            let mut hint = asset.map(|a| a.hint.clone()).unwrap_or_default();
            if let Some(h) = last_hint.get(key) {
                hint.symbol = hint.symbol.or_else(|| h.symbol.clone());
                hint.name = hint.name.or_else(|| h.name.clone());
                hint.isin = hint.isin.or_else(|| h.isin.clone());
                hint.class_hint = hint.class_hint.or(h.class_hint);
                hint.currency = hint.currency.or_else(|| h.currency.clone());
            }
            let state = match lot.status() {
                CostStatus::Known => CostState::Known,
                CostStatus::UnknownBasis => CostState::UnknownBasis,
                CostStatus::MixedCurrency => CostState::MixedCurrency,
                CostStatus::Short => CostState::Short,
            };
            let known = state == CostState::Known && lot.currency.is_some();
            let cost = CostView {
                total: known.then(|| money_str(lot.cost, lot.currency.as_ref().expect("known"))),
                average: (known && !qty.is_zero())
                    .then(|| money_str(lot.cost / qty, lot.currency.as_ref().expect("known"))),
                currency: lot.currency.as_ref().map(|c| c.to_string()),
                unknown_units: (lot.unknown_units > Decimal::ZERO)
                    .then(|| lot.unknown_units.normalize().to_string()),
                state,
            };
            let check_flag =
                failed
                    .iter()
                    .find(|(_, l, _)| &l.label == key)
                    .map(|(as_of, l, b)| CheckFlag {
                        statement_quantity: l.expected.clone(),
                        computed_quantity: l.computed.clone(),
                        as_of: as_of.clone(),
                        batch_id: *b,
                    });
            positions.push(Position {
                instrument_key: key.clone(),
                asset_id: asset.map(|a| a.id),
                symbol: hint.symbol.clone(),
                name: hint.name.clone(),
                isin: hint.isin.clone(),
                class: class_name(hint.class_hint),
                currency: hint
                    .currency
                    .as_ref()
                    .map(|c| c.to_string())
                    .or_else(|| lot.currency.as_ref().map(|c| c.to_string())),
                quantity: qty.normalize().to_string(),
                cost,
                check_flag,
            });
        }
        let cash = balances
            .cash
            .iter()
            .filter(|(_, v)| !v.is_zero())
            .map(|(c, v)| CashLine {
                currency: c.to_string(),
                amount: money_str(*v, c),
            })
            .collect();
        accounts.push(AccountHoldings {
            account_id: id,
            name,
            positions,
            cash,
        });
    }
    Ok(Holdings { accounts })
}

// ---------------------------------------------------------------- overview

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FailedCheckLine {
    /// Currency code or instrument key.
    pub label: String,
    /// Short display name: "cash" for currencies, symbol or name for instruments.
    pub display: String,
    pub expected: String,
    pub computed: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CheckSummary {
    pub as_of: String,
    pub passed: bool,
    pub failed: Vec<FailedCheckLine>,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub batch_id: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AccountStatus {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub account_id: i64,
    pub name: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub activity_count: i64,
    pub latest_check: Option<CheckSummary>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AttentionItem {
    FailedChecks {
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        account_id: i64,
        account_name: String,
        as_of: String,
        failed: Vec<FailedCheckLine>,
    },
    UnknownBasis {
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        account_id: i64,
        account_name: String,
        instrument_key: String,
        display: String,
        units: String,
    },
    PendingDecisions {
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        count: i64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Overview {
    pub accounts: Vec<AccountStatus>,
    pub attention: Vec<AttentionItem>,
}

fn display_for(key: &str, assets: &HashMap<String, AssetRow>) -> String {
    if Code::parse(key).is_ok() {
        return "cash".into();
    }
    assets
        .get(key)
        .and_then(|a| a.hint.symbol.clone().or_else(|| a.hint.name.clone()))
        .unwrap_or_else(|| {
            key.split_once(':')
                .map(|(_, v)| v.to_owned())
                .unwrap_or_else(|| key.to_owned())
        })
}

/// Account import status and items needing attention.
pub fn overview(store: &Store) -> Result<Overview, StorageError> {
    let assets: HashMap<String, AssetRow> = store
        .list_assets()?
        .into_iter()
        .map(|a| (a.instrument_key.clone(), a))
        .collect();
    let hold = holdings(store)?;
    let mut accounts = Vec::new();
    let mut attention = Vec::new();

    for a in store.list_accounts()? {
        let results = store.latest_assertion_results(a.id)?;
        let latest_check = results.first().map(|first| {
            let failed: Vec<FailedCheckLine> = results
                .iter()
                .flat_map(|r| {
                    serde_json::from_value::<Vec<AssertionLine>>(r.detail.clone())
                        .unwrap_or_default()
                })
                .filter(|l| !l.passed)
                .map(|l| FailedCheckLine {
                    display: display_for(&l.label, &assets),
                    label: l.label,
                    expected: l.expected,
                    computed: l.computed,
                })
                .collect();
            CheckSummary {
                as_of: first.assertion.as_of.to_string(),
                passed: results.iter().all(|r| r.passed),
                failed,
                batch_id: first.batch_id,
            }
        });
        if let Some(c) = latest_check.as_ref().filter(|c| !c.passed) {
            attention.push(AttentionItem::FailedChecks {
                account_id: a.id,
                account_name: a.name.clone(),
                as_of: c.as_of.clone(),
                failed: c.failed.clone(),
            });
        }
        accounts.push(AccountStatus {
            account_id: a.id,
            name: a.name.clone(),
            activity_count: store.all_activities(a.id)?.len() as i64,
            latest_check,
        });
    }

    for acct in &hold.accounts {
        for p in &acct.positions {
            if let Some(units) = &p.cost.unknown_units {
                attention.push(AttentionItem::UnknownBasis {
                    account_id: acct.account_id,
                    account_name: acct.name.clone(),
                    instrument_key: p.instrument_key.clone(),
                    display: p
                        .symbol
                        .clone()
                        .or_else(|| p.name.clone())
                        .unwrap_or_else(|| display_for(&p.instrument_key, &assets)),
                    units: units.clone(),
                });
            }
        }
    }
    Ok(Overview {
        accounts,
        attention,
    })
}

// ---------------------------------------------------------------- contributions

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Period {
    Ytd,
    Last12Months,
    SinceStart,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AccountContribution {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub account_id: i64,
    pub name: String,
    /// First date counted for this account in the period.
    pub from: Option<String>,
    pub by_currency: Vec<CashLine>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PeriodContributions {
    pub period: Period,
    pub to: String,
    pub accounts: Vec<AccountContribution>,
    pub total: Vec<CashLine>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Contributions {
    pub periods: Vec<PeriodContributions>,
}

fn period_start(p: Period, today: Date) -> Option<Date> {
    use chrono::Datelike;
    let t = today.0;
    match p {
        Period::Ytd => chrono::NaiveDate::from_ymd_opt(t.year(), 1, 1).map(Date),
        Period::Last12Months => {
            let prev = t
                .with_year(t.year() - 1)
                .or_else(|| t.pred_opt().and_then(|d| d.with_year(t.year() - 1)));
            prev.and_then(|d| d.succ_opt()).map(Date)
        }
        Period::SinceStart => None,
    }
}

fn add_line(map: &mut BTreeMap<Code, Decimal>, ccy: &Code, v: Decimal) {
    *map.entry(ccy.clone()).or_default() += v;
}

fn lines(map: &BTreeMap<Code, Decimal>) -> Vec<CashLine> {
    map.iter()
        .map(|(c, v)| {
            let s = money_str(*v, c);
            CashLine {
                currency: c.to_string(),
                amount: if *v > Decimal::ZERO {
                    format!("+{s}")
                } else {
                    s
                },
            }
        })
        .collect()
}

/// Net contributions (deposits minus withdrawals) per period, account and currency.
/// Internal transfers are not contributions. Activities after `today` are ignored.
pub fn contributions(store: &Store, today: Date) -> Result<Contributions, StorageError> {
    let accounts = store.list_accounts()?;
    let mut activity: Vec<(AccountRow, Vec<StoredActivity>)> = Vec::new();
    for a in accounts {
        let acts = store.all_activities(a.id)?;
        activity.push((a, acts));
    }
    let mut periods = Vec::new();
    for p in [Period::Ytd, Period::Last12Months, Period::SinceStart] {
        let start = period_start(p, today);
        let mut total = BTreeMap::new();
        let mut per_account = Vec::new();
        for (a, acts) in &activity {
            let mut map = BTreeMap::new();
            let in_range = |r: &ImportRecord| r.date <= today && start.is_none_or(|s| r.date >= s);
            for s in acts.iter().filter(|s| in_range(&s.record)) {
                let r = &s.record;
                let sign = match r.kind {
                    Kind::Deposit => Decimal::ONE,
                    Kind::Withdrawal => -Decimal::ONE,
                    _ => continue,
                };
                if let Some(n) = &r.net_cash {
                    add_line(&mut map, &n.currency, n.amount.value() * sign);
                    add_line(&mut total, &n.currency, n.amount.value() * sign);
                }
            }
            let from = start
                .or_else(|| acts.iter().map(|s| s.record.date).min())
                .map(|d| d.to_string());
            per_account.push(AccountContribution {
                account_id: a.id,
                name: a.name.clone(),
                from,
                by_currency: lines(&map),
            });
        }
        periods.push(PeriodContributions {
            period: p,
            to: today.to_string(),
            accounts: per_account,
            total: lines(&total),
        });
    }
    Ok(Contributions { periods })
}
