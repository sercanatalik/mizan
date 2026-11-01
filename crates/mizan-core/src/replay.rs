//! Minimal balance replay: cash per currency and quantity per instrument.
//!
//! This is not the valuation engine. It answers one question: what balances do
//! the activities imply at the end of a date, so statement balances can be
//! checked against them.
//!
//! Cash effects use `net_cash` when present; otherwise `gross` and charges, and
//! for trades quantity × price × multiplier. Amounts are magnitudes whose
//! direction comes from the kind, except `internal_transfer` and `adjustment`,
//! whose `net_cash` and `quantity` are signed (positive = into the account).

use std::collections::BTreeMap;

use mizan_import::{Assertion, Code, ImportRecord, Kind, Money};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::consistency::tolerance;
use crate::resolve::instrument_key;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Balances {
    pub cash: BTreeMap<Code, Decimal>,
    /// Keyed by instrument key.
    pub positions: BTreeMap<String, Decimal>,
}

impl Balances {
    fn cash_add(&mut self, m: &Money, sign: Decimal) {
        *self.cash.entry(m.currency.clone()).or_default() += m.amount.value() * sign;
    }

    fn cash_add_amount(&mut self, ccy: &Code, amount: Decimal) {
        *self.cash.entry(ccy.clone()).or_default() += amount;
    }

    fn pos_add(&mut self, key: &str, delta: Decimal) {
        *self.positions.entry(key.to_owned()).or_default() += delta;
    }

    pub fn position(&self, key: &str) -> Decimal {
        self.positions.get(key).copied().unwrap_or_default()
    }

    pub fn cash_in(&self, ccy: &Code) -> Decimal {
        self.cash.get(ccy).copied().unwrap_or_default()
    }

    /// Apply one record's cash and quantity effect.
    pub fn apply(&mut self, r: &ImportRecord) {
        use Kind::*;
        let one = Decimal::ONE;
        let key = r.instrument.as_ref().and_then(instrument_key);
        let qty = r.quantity.map(|q| q.value()).unwrap_or_default();

        // Cash direction for kinds whose amounts are magnitudes.
        let cash_sign = match r.kind {
            Deposit | Interest | Dividend | Distribution | Reward | ReturnOfCapital | Sell
            | ShortSell | Exchange | Spinoff => Some(one),
            Withdrawal | Fee | Tax | Buy | BuyToCover => Some(-one),
            _ => None,
        };
        if let Some(sign) = cash_sign {
            self.apply_cash(r, sign);
        }

        match r.kind {
            Buy | BuyToCover | PositionIn => {
                if let Some(k) = &key {
                    self.pos_add(k, qty);
                }
            }
            Sell | ShortSell | PositionOut => {
                if let Some(k) = &key {
                    self.pos_add(k, -qty);
                }
            }
            Split => {
                if let (Some(k), Some(s)) = (&key, &r.split) {
                    if !s.old.value().is_zero() {
                        let q = self.position(k);
                        self.positions
                            .insert(k.clone(), q * s.new.value() / s.old.value());
                    }
                }
            }
            Exchange | Spinoff => {
                for leg in &r.legs {
                    if let Some(k) = instrument_key(&leg.instrument) {
                        self.pos_add(&k, leg.quantity_delta.value());
                    }
                }
            }
            OptionExercise => {
                if let Some(k) = &key {
                    self.pos_add(k, -qty);
                }
            }
            OptionAssignment => {
                if let Some(k) = &key {
                    self.pos_add(k, qty);
                }
            }
            OptionExpiry => {
                if let Some(k) = &key {
                    self.positions.insert(k.clone(), Decimal::ZERO);
                }
            }
            FxConversion => {
                // `gross` leaves the account in one currency, `net_cash` arrives in another.
                if let Some(g) = &r.gross {
                    self.cash_add(g, -one);
                }
                if let Some(n) = &r.net_cash {
                    self.cash_add(n, one);
                }
            }
            InternalTransfer | Adjustment => {
                if let Some(n) = &r.net_cash {
                    self.cash_add(n, one);
                }
                if let (Some(k), Some(q)) = (&key, r.quantity) {
                    self.pos_add(k, q.value());
                }
            }
            _ => {}
        }

        // Position moves carry no cash except their charges.
        if matches!(r.kind, PositionIn | PositionOut) {
            for c in &r.charges {
                self.cash_add(&c.amount, -one);
            }
        }
    }

    fn apply_cash(&mut self, r: &ImportRecord, sign: Decimal) {
        if let Some(n) = &r.net_cash {
            self.cash_add(n, sign);
            return;
        }
        // No net cash on the statement: gross adjusted by charges, if both are in one currency.
        let outflow = sign.is_sign_negative();
        let gross = r.gross.clone().or_else(|| {
            let (q, p) = (r.quantity?, r.price.as_ref()?);
            let mult = r
                .instrument
                .as_ref()
                .and_then(|i| i.multiplier)
                .map(|m| m.value())
                .unwrap_or(Decimal::ONE);
            Some(Money {
                amount: mizan_import::Dec(q.value() * p.amount.value() * mult),
                currency: p.currency.clone(),
            })
        });
        let Some(g) = gross else {
            // Standalone charge without net cash: its charges are the amount.
            for c in &r.charges {
                self.cash_add(&c.amount, sign);
            }
            return;
        };
        let mut total = g.amount.value();
        for c in r.charges.iter().filter(|c| c.amount.currency == g.currency) {
            // Charges add to what leaves the account and reduce what arrives.
            total += c.amount.amount.value() * if outflow { Decimal::ONE } else { -Decimal::ONE };
        }
        self.cash_add_amount(&g.currency, total * sign);
    }
}

/// Replay records in the given order.
pub fn replay<'a>(records: impl IntoIterator<Item = &'a ImportRecord>) -> Balances {
    let mut b = Balances::default();
    for r in records {
        b.apply(r);
    }
    b
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AssertionLine {
    /// Currency code for cash, instrument key for positions.
    pub label: String,
    pub expected: String,
    pub computed: String,
    pub passed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AssertionCheck {
    pub account: String,
    pub as_of: String,
    pub locator: String,
    pub passed: bool,
    pub lines: Vec<AssertionLine>,
    /// Set when the assertion could not be checked, e.g. its account is unresolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Compare an assertion against balances computed to the end of its date.
///
/// Cash lines pass within half a minor unit; positions must match exactly.
pub fn check_assertion(a: &Assertion, computed: &Balances) -> AssertionCheck {
    let mut lines = Vec::new();
    for m in &a.cash {
        let got = computed.cash_in(&m.currency);
        lines.push(AssertionLine {
            label: m.currency.to_string(),
            expected: m.amount.to_string(),
            computed: got.round_dp(m.amount.value().scale().max(2)).to_string(),
            passed: (got - m.amount.value()).abs() <= tolerance(&m.currency),
        });
    }
    for p in &a.positions {
        let key = instrument_key(&p.instrument).unwrap_or_default();
        let got = computed.position(&key);
        lines.push(AssertionLine {
            label: key,
            expected: p.quantity.to_string(),
            computed: got.normalize().to_string(),
            passed: got == p.quantity.value(),
        });
    }
    AssertionCheck {
        account: a.account.hint.clone(),
        as_of: a.as_of.to_string(),
        locator: a.source.locator.clone(),
        passed: lines.iter().all(|l| l.passed),
        lines,
        note: None,
    }
}
