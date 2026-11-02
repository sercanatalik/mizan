//! Dividends, distributions and interest: which records count as income and
//! how gross and tax withheld are derived. Shared by the tax reports and the
//! Income screen so the two cannot disagree; each converts amounts its own way.

use mizan_import::{ChargeType, Date, Fx, ImportRecord, Kind, LinkRole, Money};
use mizan_storage::{AccountId, StoredActivity};
use rust_decimal::Decimal;

use crate::replay::KeyMap;
use crate::resolve::instrument_key;
use crate::tax::events::money;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IncomeKind {
    /// Dividends and fund distributions.
    Dividend,
    Interest,
}

impl IncomeKind {
    pub fn label(&self) -> &'static str {
        match self {
            IncomeKind::Dividend => "dividends",
            IncomeKind::Interest => "interest",
        }
    }
}

/// One income record with its amounts in the currencies the statement used.
#[derive(Clone, Debug, PartialEq)]
pub struct Payment {
    pub account: AccountId,
    pub date: Date,
    pub kind: IncomeKind,
    /// Canonical instrument key; `None` for interest on cash.
    pub key: Option<String>,
    /// The record's gross, else net cash plus withholding in its currency.
    pub gross: Option<Money>,
    /// Withholding charges and linked `tax` activities.
    pub withheld: Vec<Money>,
    pub net: Option<Money>,
    /// Rate printed on the statement, if any.
    pub fx: Option<Fx>,
}

fn withholding(r: &ImportRecord) -> Vec<Money> {
    r.charges
        .iter()
        .filter(|c| c.charge_type == ChargeType::WithholdingTax)
        .map(|c| c.amount.clone())
        .collect()
}

/// Gross: the record's gross, else net cash plus withholding in its currency.
pub fn gross(r: &ImportRecord) -> Option<Money> {
    if let Some(g) = &r.gross {
        return Some(g.clone());
    }
    let n = r.net_cash.as_ref()?;
    let w: Decimal = withholding(r)
        .iter()
        .filter(|m| m.currency == n.currency)
        .map(|m| m.amount.value())
        .sum();
    Some(money(n.amount.value() + w, n.currency.clone()))
}

fn is_linked(a: &ImportRecord, b: &ImportRecord) -> bool {
    let points = |from: &ImportRecord, to: &ImportRecord| {
        to.external_id.as_deref().is_some_and(|id| {
            from.links
                .iter()
                .any(|l| l.role != LinkRole::TransferPair && l.target == id)
        })
    };
    points(a, b) || points(b, a)
}

/// Every income record of `accounts`, oldest first.
pub fn payments(accounts: &[(AccountId, Vec<StoredActivity>)], keys: &KeyMap) -> Vec<Payment> {
    let canon = |k: String| keys.get(&k).cloned().unwrap_or(k);
    let mut out = Vec::new();
    for (account, stored) in accounts {
        let taxes: Vec<&ImportRecord> = stored
            .iter()
            .map(|s| &s.record)
            .filter(|r| r.kind == Kind::Tax)
            .collect();
        for r in stored.iter().map(|s| &s.record) {
            let kind = match r.kind {
                Kind::Dividend | Kind::Distribution => IncomeKind::Dividend,
                Kind::Interest => IncomeKind::Interest,
                _ => continue,
            };
            let mut withheld = withholding(r);
            withheld.extend(
                taxes
                    .iter()
                    .filter(|x| is_linked(r, x))
                    .filter_map(|x| x.net_cash.clone().or_else(|| x.gross.clone())),
            );
            out.push(Payment {
                account: *account,
                date: r.date,
                kind,
                key: r.instrument.as_ref().and_then(instrument_key).map(canon),
                gross: gross(r),
                withheld,
                net: r.net_cash.clone(),
                fx: r.fx.clone(),
            });
        }
    }
    out.sort_by_key(|p| (p.date, p.account));
    out
}

// ---- The Income screen ----

use std::collections::{BTreeMap, HashMap};

use mizan_import::Code;
use mizan_storage::{StorageError, Store};
use serde::Serialize;

use crate::portfolio::{period_start, AccountScope, Period};
use crate::returns::PriceBook;
use crate::tax::rates::Rates;
use crate::valuation::{base_currency, money as base_money};

/// One payment, in its own currency and in the base currency.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct IncomePaymentView {
    pub date: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub account_id: i64,
    pub account: String,
    pub instrument_key: Option<String>,
    /// Symbol or name, or "Interest · USD" for interest on cash.
    pub label: String,
    /// `dividend` (dividends and distributions) or `interest`.
    pub kind: String,
    pub currency: Option<String>,
    pub gross: Option<String>,
    pub withheld: String,
    pub net: Option<String>,
    pub base_gross: Option<String>,
    pub base_withheld: Option<String>,
    pub base_net: Option<String>,
    /// Why the payment is left out of the totals, e.g. "not converted: no rate".
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct IncomeTotals {
    pub gross: String,
    pub withheld: String,
    pub net: String,
}

/// Net income of one calendar month in the base currency, by type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct IncomeMonth {
    /// `YYYY-MM`.
    pub month: String,
    pub dividends: String,
    pub interest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct IncomeInstrument {
    pub instrument_key: Option<String>,
    pub label: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub account_id: i64,
    pub account: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub payments: usize,
    pub gross: String,
    pub withheld: String,
    pub net: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct IncomeView {
    pub base_currency: String,
    /// First day counted; `None` before any income since start.
    pub from: Option<String>,
    pub to: String,
    pub totals: IncomeTotals,
    /// The same totals for the previous period (same dates last year for YTD).
    pub previous: Option<IncomeTotals>,
    pub previous_from: Option<String>,
    pub previous_to: Option<String>,
    /// Payments left out of the totals because they could not be converted.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub not_converted: usize,
    /// Every month from `from` to `to`, including months without income.
    pub months: Vec<IncomeMonth>,
    /// Largest net first.
    pub instruments: Vec<IncomeInstrument>,
    /// Oldest first.
    pub payments: Vec<IncomePaymentView>,
}

#[derive(Default, Clone, Copy)]
struct Sum {
    gross: Decimal,
    withheld: Decimal,
}

impl Sum {
    fn totals(&self, base: &Code) -> IncomeTotals {
        IncomeTotals {
            gross: base_money(self.gross, base),
            withheld: base_money(self.withheld, base),
            net: base_money(self.gross - self.withheld, base),
        }
    }
}

/// A payment converted to `base`: gross and withheld, or `None` without a rate.
fn converted(p: &Payment, rates: &Rates, base: &Code) -> Option<Sum> {
    let conv = |m: &Money| rates.convert(m.amount.value(), &m.currency, base, p.date, None);
    let gross = p.gross.as_ref().and_then(conv)?;
    let withheld = p
        .withheld
        .iter()
        .map(conv)
        .try_fold(Decimal::ZERO, |acc, x| x.map(|v| acc + v))?;
    Some(Sum { gross, withheld })
}

fn year_back(d: Date) -> Date {
    use chrono::Datelike;
    let t = d.0;
    Date(
        t.with_year(t.year() - 1)
            .or_else(|| t.pred_opt().and_then(|x| x.with_year(t.year() - 1)))
            .unwrap_or(t),
    )
}

/// The period before `from..=to`: the same dates last year for YTD, the
/// preceding period of the same kind otherwise; none since start.
fn previous(period: Period, from: Option<Date>, to: Date) -> Option<(Date, Date)> {
    let from = from?;
    match period {
        Period::SinceStart => None,
        Period::Ytd => Some((year_back(from), year_back(to))),
        _ => {
            let end = Date(from.0.pred_opt()?);
            Some((period_start(period, end)?, end))
        }
    }
}

fn months_between(from: Date, to: Date) -> Vec<String> {
    use chrono::Datelike;
    let mut out = Vec::new();
    let (mut y, mut m) = (from.0.year(), from.0.month());
    while (y, m) <= (to.0.year(), to.0.month()) {
        out.push(format!("{y:04}-{m:02}"));
        if m == 12 {
            y += 1;
            m = 1;
        } else {
            m += 1;
        }
    }
    out
}

/// Income of the accounts in `scope` for `period` ending `today`, in the base currency.
pub fn income_view(
    store: &Store,
    book: &PriceBook,
    scope: AccountScope,
    period: Period,
    today: Date,
) -> Result<IncomeView, StorageError> {
    let base = base_currency(store)?;
    let keys = store.key_map()?;
    let rates = Rates::new(book);
    let accounts: Vec<_> = store
        .list_accounts()?
        .into_iter()
        .filter(|a| scope.includes(a.id))
        .collect();
    let names: HashMap<i64, String> = accounts.iter().map(|a| (a.id, a.name.clone())).collect();
    let mut activity = Vec::new();
    for a in &accounts {
        activity.push((a.id, store.all_activities(a.id)?));
    }
    let labels: HashMap<String, String> = store
        .list_assets()?
        .into_iter()
        .map(|a| {
            let l = a
                .hint
                .symbol
                .clone()
                .or(a.hint.name.clone())
                .unwrap_or(a.instrument_key.clone());
            (a.instrument_key, l)
        })
        .collect();
    let all = payments(&activity, &keys);
    let from = period_start(period, today).or_else(|| all.first().map(|p| p.date));
    let in_range =
        |p: &&Payment, f: Option<Date>, t: Date| p.date <= t && f.is_none_or(|f| p.date >= f);

    let mut total = Sum::default();
    let mut not_converted = 0;
    let mut by_month: BTreeMap<String, (Decimal, Decimal)> = BTreeMap::new();
    let mut by_instrument: BTreeMap<(Option<String>, i64, String), (usize, Sum)> = BTreeMap::new();
    let mut views = Vec::new();
    for p in all.iter().filter(|p| in_range(p, from, today)) {
        let c = converted(p, &rates, &base);
        let cash_ccy = p
            .net
            .as_ref()
            .or(p.gross.as_ref())
            .map(|m| m.currency.clone());
        let label = match &p.key {
            Some(k) => labels.get(k).cloned().unwrap_or_else(|| k.clone()),
            None => format!(
                "Interest · {}",
                cash_ccy.as_ref().map(|c| c.to_string()).unwrap_or_default()
            ),
        };
        if let Some(s) = c {
            total.gross += s.gross;
            total.withheld += s.withheld;
            let m = by_month
                .entry(p.date.to_string()[..7].to_string())
                .or_default();
            match p.kind {
                IncomeKind::Dividend => m.0 += s.gross - s.withheld,
                IncomeKind::Interest => m.1 += s.gross - s.withheld,
            }
            let e = by_instrument
                .entry((p.key.clone(), p.account, label.clone()))
                .or_default();
            e.0 += 1;
            e.1.gross += s.gross;
            e.1.withheld += s.withheld;
        } else {
            not_converted += 1;
        }
        let orig = |m: &Money| base_money(m.amount.value(), &m.currency);
        let withheld_same: Decimal = p
            .withheld
            .iter()
            .filter(|w| Some(&w.currency) == cash_ccy.as_ref())
            .map(|w| w.amount.value())
            .sum();
        views.push(IncomePaymentView {
            date: p.date.to_string(),
            account_id: p.account,
            account: names.get(&p.account).cloned().unwrap_or_default(),
            instrument_key: p.key.clone(),
            label,
            kind: match p.kind {
                IncomeKind::Dividend => "dividend".into(),
                IncomeKind::Interest => "interest".into(),
            },
            currency: cash_ccy.as_ref().map(|c| c.to_string()),
            gross: p.gross.as_ref().map(orig),
            withheld: cash_ccy.as_ref().map_or_else(
                || withheld_same.to_string(),
                |c| base_money(withheld_same, c),
            ),
            net: p.net.as_ref().map(orig),
            base_gross: c.map(|s| base_money(s.gross, &base)),
            base_withheld: c.map(|s| base_money(s.withheld, &base)),
            base_net: c.map(|s| base_money(s.gross - s.withheld, &base)),
            note: c.is_none().then(|| "not converted: no rate".to_string()),
        });
    }

    let prev = previous(period, from, today).map(|(pf, pt)| {
        let mut s = Sum::default();
        for p in all.iter().filter(|p| in_range(p, Some(pf), pt)) {
            if let Some(c) = converted(p, &rates, &base) {
                s.gross += c.gross;
                s.withheld += c.withheld;
            }
        }
        (pf, pt, s)
    });

    let months = from
        .map(|f| months_between(f, today))
        .unwrap_or_default()
        .into_iter()
        .map(|m| {
            let (d, i) = by_month.get(&m).copied().unwrap_or_default();
            IncomeMonth {
                month: m,
                dividends: base_money(d, &base),
                interest: base_money(i, &base),
            }
        })
        .collect();
    let mut instruments: Vec<(Decimal, IncomeInstrument)> = by_instrument
        .into_iter()
        .map(|((key, account, label), (n, s))| {
            (
                s.gross - s.withheld,
                IncomeInstrument {
                    instrument_key: key,
                    label,
                    account_id: account,
                    account: names.get(&account).cloned().unwrap_or_default(),
                    payments: n,
                    gross: base_money(s.gross, &base),
                    withheld: base_money(s.withheld, &base),
                    net: base_money(s.gross - s.withheld, &base),
                },
            )
        })
        .collect();
    instruments.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.label.cmp(&b.1.label)));

    Ok(IncomeView {
        base_currency: base.to_string(),
        from: from.map(|d| d.to_string()),
        to: today.to_string(),
        totals: total.totals(&base),
        previous: prev.as_ref().map(|(_, _, s)| s.totals(&base)),
        previous_from: prev.as_ref().map(|(f, _, _)| f.to_string()),
        previous_to: prev.as_ref().map(|(_, t, _)| t.to_string()),
        not_converted,
        months,
        instruments: instruments.into_iter().map(|(_, i)| i).collect(),
        payments: views,
    })
}

/// CSV of the payments behind an Income view: one row per payment.
pub fn income_csv(v: &IncomeView) -> String {
    use crate::tax::csv::line;
    let base = &v.base_currency;
    let mut out = line(&[
        "date",
        "account",
        "instrument",
        "type",
        "currency",
        "gross",
        "withheld",
        "net",
        &format!("gross_{base}"),
        &format!("withheld_{base}"),
        &format!("net_{base}"),
        "note",
    ]);
    let o = |x: &Option<String>| x.clone().unwrap_or_default();
    for p in &v.payments {
        out.push_str(&line(&[
            &p.date,
            &p.account,
            &p.label,
            &p.kind,
            &o(&p.currency),
            &o(&p.gross),
            &p.withheld,
            &o(&p.net),
            &o(&p.base_gross),
            &o(&p.base_withheld),
            &o(&p.base_net),
            &o(&p.note),
        ]));
    }
    out
}

/// `mizan-income-<scope>-<from>-<to>.csv`, with the scope slugged.
pub fn income_file_name(v: &IncomeView, scope_name: &str) -> String {
    let slug: String = scope_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug = if slug.is_empty() {
        "account".to_string()
    } else {
        slug
    };
    format!(
        "mizan-income-{slug}-{}-{}.csv",
        v.from.as_deref().unwrap_or("start"),
        v.to
    )
}
