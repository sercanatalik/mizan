//! Gain rows shared by the country rules, and conversion of event amounts.

use std::collections::HashMap;

use mizan_import::{Code, Date, Fx, Money};
use mizan_storage::{AccountId, TaxTreatment};
use rust_decimal::Decimal;

use super::events::{Event, EventKind};
use super::rates::Rates;

/// Which UK identification rule matched a disposal; US and Turkish rows are
/// first in, first out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    SameDay,
    ThirtyDays,
    Pool,
    Fifo,
}

impl Rule {
    pub fn label(&self) -> &'static str {
        match self {
            Rule::SameDay => "same day",
            Rule::ThirtyDays => "30 days",
            Rule::Pool => "pool",
            Rule::Fifo => "first in, first out",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Term {
    Short,
    Long,
}

/// Why a row has no gain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Missing {
    CostUnknown,
    NoRate,
    NotCovered(&'static str),
}

impl Missing {
    pub fn reason(&self) -> String {
        match self {
            Missing::CostUnknown => "cost unknown".into(),
            Missing::NoRate => "no exchange rate".into(),
            Missing::NotCovered(what) => format!("not covered: {what}"),
        }
    }
}

/// One realized disposal (or part of one) in the report currency.
#[derive(Clone, Debug, PartialEq)]
pub struct GainRow {
    pub key: String,
    /// `None` when several accounts' disposals were matched together.
    pub account: Option<AccountId>,
    pub sold: Date,
    /// `None` for rows matched to the UK pool.
    pub acquired: Option<Date>,
    pub qty: Decimal,
    pub proceeds: Option<Decimal>,
    /// Cost before any indexation, including wash-sale basis adjustments.
    pub cost: Option<Decimal>,
    /// Turkish cost after Yİ-ÜFE indexation, when applied.
    pub indexed_cost: Option<Decimal>,
    /// US wash-sale disallowed loss, added back to the gain.
    pub adjustment: Option<Decimal>,
    pub code: Option<&'static str>,
    pub gain: Option<Decimal>,
    pub rule: Rule,
    pub term: Option<Term>,
    pub missing: Option<Missing>,
    pub notes: Vec<String>,
}

impl GainRow {
    pub fn new(key: &str, sold: Date, qty: Decimal, rule: Rule) -> Self {
        GainRow {
            key: key.to_owned(),
            account: None,
            sold,
            acquired: None,
            qty,
            proceeds: None,
            cost: None,
            indexed_cost: None,
            adjustment: None,
            code: None,
            gain: None,
            rule,
            term: None,
            missing: None,
            notes: vec![],
        }
    }

    /// Fill proceeds, cost and gain, or the reason they are missing.
    pub fn with_amounts(
        mut self,
        proceeds: Result<Decimal, Missing>,
        cost: Result<Decimal, Missing>,
    ) -> Self {
        match (proceeds, cost) {
            (Ok(p), Ok(c)) => {
                self.proceeds = Some(p);
                self.cost = Some(c);
                self.gain = Some(p - c);
            }
            (p, c) => {
                self.proceeds = p.as_ref().ok().copied();
                self.cost = c.as_ref().ok().copied();
                // Cost unknown is the more basic gap; report it first.
                self.missing = Some(match (p, c) {
                    (_, Err(Missing::CostUnknown)) => Missing::CostUnknown,
                    (Err(e), _) | (_, Err(e)) => e,
                    _ => unreachable!(),
                });
            }
        }
        self
    }

    pub fn not_covered(key: &str, sold: Date, account: AccountId, what: &'static str) -> Self {
        let mut r = GainRow::new(key, sold, Decimal::ZERO, Rule::Fifo);
        r.account = Some(account);
        r.missing = Some(Missing::NotCovered(what));
        r
    }
}

/// Convert an amount into `to` on `day`, preferring a statement rate.
pub fn convert(
    rates: &Rates,
    m: Option<&Money>,
    to: &Code,
    day: Date,
    fx: Option<&Fx>,
) -> Result<Decimal, Missing> {
    let m = m.ok_or(Missing::CostUnknown)?;
    rates
        .convert(m.amount.value(), &m.currency, to, day, fx)
        .ok_or(Missing::NoRate)
}

/// Account treatments by id; unknown accounts count as taxable.
#[derive(Clone, Debug, Default)]
pub struct Treatments(pub HashMap<AccountId, TaxTreatment>);

impl Treatments {
    pub fn of(&self, account: AccountId) -> TaxTreatment {
        self.0.get(&account).copied().unwrap_or_default()
    }

    pub fn taxable(&self, account: AccountId) -> bool {
        self.of(account).is_taxable()
    }
}

/// Transfer pairs whose two sides are both in taxable accounts.
pub fn taxable_pairs(events: &[Event], t: &Treatments) -> HashMap<usize, bool> {
    let mut sides: HashMap<usize, Vec<bool>> = HashMap::new();
    for e in events {
        if let EventKind::TransferOut { pair: Some(p), .. }
        | EventKind::TransferIn { pair: Some(p), .. } = &e.kind
        {
            sides.entry(*p).or_default().push(t.taxable(e.account));
        }
    }
    sides
        .into_iter()
        .map(|(p, s)| (p, s.len() == 2 && s.iter().all(|x| *x)))
        .collect()
}

/// Round a report figure to two decimals, half away from zero.
pub fn round2(d: Decimal) -> Decimal {
    d.round_dp_with_strategy(2, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
}
