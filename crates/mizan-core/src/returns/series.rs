//! Daily value series: replay each account's activities once, then value every
//! calendar day from its first activity to today.

use std::collections::BTreeMap;

use mizan_import::{Code, Date, ImportRecord, Kind};
use mizan_storage::StoredActivity;
use rust_decimal::Decimal;

use super::pricebook::{on_or_before, PriceBook};
use crate::portfolio::cost::{CostBook, CostStatus};
use crate::portfolio::ordered;
use crate::replay::{Balances, KeyMap};
use crate::resolve::instrument_key;

/// What moved in or out of an account on one day. Positive means into the account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlowAmount {
    Cash {
        currency: Code,
        amount: Decimal,
    },
    /// Securities, valued at that day's value of the position.
    Units {
        key: String,
        quantity: Decimal,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flow {
    pub date: Date,
    pub amount: FlowAmount,
    /// Between the user's own accounts: a flow for the account, not the portfolio.
    pub internal: bool,
}

/// A per-unit amount in a currency.
pub type UnitCost = (Decimal, Code);

/// One account's activities reduced to dated balances, costs, trade prices and flows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountReplay {
    pub account_id: i64,
    pub name: String,
    pub first: Option<Date>,
    /// Quantity per instrument after each day it changed.
    pub qty: BTreeMap<String, Vec<(Date, Decimal)>>,
    /// Cash per currency after each day it changed.
    pub cash: BTreeMap<Code, Vec<(Date, Decimal)>>,
    /// Provisional average cost per unit after each day it changed (`None` when unknown).
    pub avg_cost: BTreeMap<String, Vec<(Date, Option<UnitCost>)>>,
    /// Unit price of each trade, in the trade's currency.
    pub trade_prices: BTreeMap<String, Vec<(Date, Decimal, Code)>>,
    pub flows: Vec<Flow>,
}

/// Unit price a trade implies: its price, otherwise its cash amount divided by quantity.
fn trade_price(r: &ImportRecord) -> Option<(Decimal, Code)> {
    if !matches!(
        r.kind,
        Kind::Buy | Kind::Sell | Kind::ShortSell | Kind::BuyToCover
    ) {
        return None;
    }
    let qty = r.quantity?.value().abs();
    if qty.is_zero() {
        return None;
    }
    if let Some(p) = &r.price {
        return Some((p.amount.value(), p.currency.clone()));
    }
    let cash = r.gross.as_ref().or(r.net_cash.as_ref())?;
    Some((cash.amount.value() / qty, cash.currency.clone()))
}

fn flows_of(r: &ImportRecord, key: Option<&String>) -> Vec<Flow> {
    let mut out = Vec::new();
    let mut cash = |sign: Decimal, internal: bool| {
        if let Some(n) = &r.net_cash {
            out.push(Flow {
                date: r.date,
                amount: FlowAmount::Cash {
                    currency: n.currency.clone(),
                    amount: n.amount.value() * sign,
                },
                internal,
            });
        }
    };
    match r.kind {
        Kind::Deposit => cash(Decimal::ONE, false),
        Kind::Withdrawal => cash(-Decimal::ONE, false),
        // Signed: positive is into the account.
        Kind::InternalTransfer => cash(Decimal::ONE, true),
        _ => {}
    }
    let units = |sign: Decimal, internal: bool| {
        key.zip(r.quantity).map(|(k, q)| Flow {
            date: r.date,
            amount: FlowAmount::Units {
                key: k.clone(),
                quantity: q.value() * sign,
            },
            internal,
        })
    };
    let u = match r.kind {
        Kind::PositionIn => units(Decimal::ONE, false),
        Kind::PositionOut => units(-Decimal::ONE, false),
        Kind::InternalTransfer => units(Decimal::ONE, true),
        _ => None,
    };
    out.extend(u);
    out
}

fn push_changed<K: Ord + Clone, V: PartialEq + Clone>(
    series: &mut BTreeMap<K, Vec<(Date, V)>>,
    key: &K,
    date: Date,
    v: V,
) {
    let s = series.entry(key.clone()).or_default();
    if s.last().map(|(_, last)| last) != Some(&v) {
        s.push((date, v));
    }
}

/// Replay one account. Records are processed in the deterministic processing order.
pub fn replay_account(
    account_id: i64,
    name: &str,
    stored: &[StoredActivity],
    keys: &KeyMap,
) -> AccountReplay {
    let recs = ordered(stored);
    let mut out = AccountReplay {
        account_id,
        name: name.to_string(),
        first: recs.iter().map(|r| r.date).min(),
        ..Default::default()
    };
    let mut balances = Balances::with_keys(keys.clone());
    let mut book = CostBook::with_keys(keys.clone());

    let snapshot = |out: &mut AccountReplay, balances: &Balances, book: &CostBook, date: Date| {
        for (k, q) in &balances.positions {
            push_changed(&mut out.qty, k, date, *q);
        }
        for (c, v) in &balances.cash {
            push_changed(&mut out.cash, c, date, *v);
        }
        for (k, lot) in &book.lots {
            let per_unit = match (&lot.currency, lot.status()) {
                (Some(c), CostStatus::Known) if !lot.quantity.is_zero() => {
                    Some((lot.cost / lot.quantity, c.clone()))
                }
                _ => None,
            };
            push_changed(&mut out.avg_cost, k, date, per_unit);
        }
    };

    let mut current: Option<Date> = None;
    for r in &recs {
        if let Some(d) = current {
            if d != r.date {
                snapshot(&mut out, &balances, &book, d);
            }
        }
        current = Some(r.date);
        balances.apply(r);
        book.apply(r);
        let key = r
            .instrument
            .as_ref()
            .and_then(instrument_key)
            .map(|k| balances.canon(k));
        if let (Some(k), Some((p, c))) = (&key, trade_price(r)) {
            out.trade_prices
                .entry(k.clone())
                .or_default()
                .push((r.date, p, c));
        }
        out.flows.extend(flows_of(r, key.as_ref()));
    }
    if let Some(d) = current {
        snapshot(&mut out, &balances, &book, d);
    }
    out
}

// ---------------------------------------------------------------- grid

/// How a unit was valued on a day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PriceKind {
    Market,
    TradePrice,
    AverageCost,
}

/// The value of one unit of an instrument on `day`: the newest market price,
/// otherwise the newest trade price, otherwise the provisional average cost.
pub fn unit_value(
    rep: &AccountReplay,
    book: &PriceBook,
    key: &str,
    day: Date,
) -> Option<(Decimal, Code, PriceKind)> {
    if let Some(p) = book.price_on(key, day) {
        return Some((p.value, p.currency.clone(), PriceKind::Market));
    }
    if let Some((_, p, c)) = rep
        .trade_prices
        .get(key)
        .and_then(|s| on_or_before(s, day, |(d, _, _)| *d))
    {
        return Some((*p, c.clone(), PriceKind::TradePrice));
    }
    rep.avg_cost
        .get(key)
        .and_then(|s| on_or_before(s, day, |(d, _)| *d))
        .and_then(|(_, v)| v.clone())
        .map(|(p, c)| (p, c, PriceKind::AverageCost))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DayValue {
    pub date: Date,
    /// Value in the base currency at the end of the day; `None` when an amount
    /// could not be converted (see `missing_rate`).
    pub value: Option<Decimal>,
    /// External flows (deposits, withdrawals, securities moved in or out), base currency.
    pub external_flow: Decimal,
    /// Transfers between the user's own accounts, base currency.
    pub internal_flow: Decimal,
    /// Positions valued at a trade price or average cost.
    pub estimated: u32,
    /// Positions with no price, trade price or cost, left out of the value.
    pub excluded: u32,
    /// A currency without a rate on or before this day.
    pub missing_rate: Option<Code>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountSeries {
    pub account_id: i64,
    pub name: String,
    /// One entry per calendar day from the first activity to today.
    pub days: Vec<DayValue>,
}

impl AccountSeries {
    pub fn first(&self) -> Option<Date> {
        self.days.first().map(|d| d.date)
    }

    /// The entry for `day`, if the series covers it.
    pub fn day(&self, day: Date) -> Option<&DayValue> {
        let first = self.first()?;
        let i = (day.0 - first.0).num_days();
        (i >= 0).then(|| self.days.get(i as usize)).flatten()
    }
}

fn at<T: Clone>(s: &[(Date, T)], day: Date) -> Option<T> {
    on_or_before(s, day, |(d, _)| *d).map(|(_, v)| v.clone())
}

/// Value every day of an account from its first activity to `today`.
pub fn account_series(
    rep: &AccountReplay,
    book: &PriceBook,
    base: &Code,
    today: Date,
) -> AccountSeries {
    let mut days = Vec::new();
    let mut flows = rep.flows.iter().peekable();
    if let Some(first) = rep.first {
        let mut day = first;
        while day <= today {
            let mut value = Decimal::ZERO;
            let mut missing: Option<Code> = None;
            let (mut estimated, mut excluded) = (0u32, 0u32);
            let convert = |amount: Decimal, ccy: &Code, missing: &mut Option<Code>| {
                let v = book.convert(amount, ccy, base, day);
                if v.is_none() && missing.is_none() {
                    *missing = Some(if book.try_per_unit(ccy, day).is_none() {
                        ccy.clone()
                    } else {
                        base.clone()
                    });
                }
                v.unwrap_or_default()
            };
            for (key, series) in &rep.qty {
                let q = at(series, day).unwrap_or_default();
                if q.is_zero() {
                    continue;
                }
                match unit_value(rep, book, key, day) {
                    Some((p, c, kind)) => {
                        if kind != PriceKind::Market {
                            estimated += 1;
                        }
                        value += convert(q * p, &c, &mut missing);
                    }
                    None => excluded += 1,
                }
            }
            for (ccy, series) in &rep.cash {
                let v = at(series, day).unwrap_or_default();
                if !v.is_zero() {
                    value += convert(v, ccy, &mut missing);
                }
            }
            let (mut external, mut internal) = (Decimal::ZERO, Decimal::ZERO);
            while let Some(f) = flows.peek().filter(|f| f.date == day) {
                let amount = match &f.amount {
                    FlowAmount::Cash { currency, amount } => {
                        convert(*amount, currency, &mut missing)
                    }
                    FlowAmount::Units { key, quantity } => match unit_value(rep, book, key, day) {
                        Some((p, c, _)) => convert(*quantity * p, &c, &mut missing),
                        None => Decimal::ZERO,
                    },
                };
                if f.internal {
                    internal += amount;
                } else {
                    external += amount;
                }
                flows.next();
            }
            days.push(DayValue {
                date: day,
                value: missing.is_none().then_some(value),
                external_flow: external,
                internal_flow: internal,
                estimated,
                excluded,
                missing_rate: missing,
            });
            day = Date(day.0.succ_opt().expect("valid day"));
        }
    }
    AccountSeries {
        account_id: rep.account_id,
        name: rep.name.clone(),
        days,
    }
}
