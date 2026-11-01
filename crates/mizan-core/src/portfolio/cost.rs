//! Provisional average cost per instrument within one account.
//!
//! Acquisitions add the cash they settled for; disposals remove cost in
//! proportion to quantity; splits change quantity but not cost; spinoff and
//! exchange legs carry cost-basis shares; a return of capital reduces cost.
//! Units without a known basis are counted separately and never valued at zero.
//!
//! This is deliberately simple and will be replaced by the valuation engine's lots.

use std::collections::BTreeMap;

use mizan_import::{Code, ImportRecord, Kind, Money};
use rust_decimal::Decimal;

use crate::resolve::instrument_key;

/// Running cost state of one instrument.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lot {
    pub quantity: Decimal,
    /// Cost of the units whose basis is known, in `currency`.
    pub cost: Decimal,
    pub currency: Option<Code>,
    /// Units that arrived without a cost basis.
    pub unknown_units: Decimal,
    /// Acquisitions were paid in more than one currency.
    pub mixed_currency: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CostStatus {
    Known,
    UnknownBasis,
    MixedCurrency,
    Short,
}

impl Lot {
    pub fn status(&self) -> CostStatus {
        if self.quantity < Decimal::ZERO {
            CostStatus::Short
        } else if self.mixed_currency {
            CostStatus::MixedCurrency
        } else if self.unknown_units > Decimal::ZERO {
            CostStatus::UnknownBasis
        } else {
            CostStatus::Known
        }
    }

    fn add_cost(&mut self, m: &Money) {
        match &self.currency {
            None => self.currency = Some(m.currency.clone()),
            Some(c) if *c != m.currency => self.mixed_currency = true,
            _ => {}
        }
        self.cost += m.amount.value();
    }

    fn acquire(&mut self, qty: Decimal, cost: Option<&Money>) {
        if self.quantity < Decimal::ZERO {
            // Covering a short: no cost tracking for shorts yet.
            self.quantity += qty;
            return;
        }
        self.quantity += qty;
        match cost {
            Some(m) => self.add_cost(m),
            None => self.unknown_units += qty,
        }
    }

    /// Remove `qty` units; cost and unknown units shrink pro rata.
    /// Returns the cost removed.
    fn dispose(&mut self, qty: Decimal) -> Decimal {
        if self.quantity <= Decimal::ZERO || qty <= Decimal::ZERO {
            self.quantity -= qty;
            return Decimal::ZERO;
        }
        let fraction = (qty / self.quantity).min(Decimal::ONE);
        let removed = self.cost * fraction;
        self.cost -= removed;
        self.unknown_units -= self.unknown_units * fraction;
        self.quantity -= qty;
        if self.quantity <= Decimal::ZERO {
            if self.quantity.is_zero() {
                *self = Lot::default();
            } else {
                // Sold more than held: the remainder is a short position.
                self.cost = Decimal::ZERO;
                self.unknown_units = Decimal::ZERO;
            }
        }
        removed
    }
}

/// What a record settled in cash for an acquisition: net cash, else gross plus
/// charges, else quantity × price × multiplier plus charges.
fn settled_cost(r: &ImportRecord) -> Option<Money> {
    if let Some(n) = &r.net_cash {
        return Some(n.clone());
    }
    let (amount, currency) = match (&r.gross, r.quantity, &r.price) {
        (Some(g), _, _) => (g.amount.value(), g.currency.clone()),
        (None, Some(q), Some(p)) => {
            let mult = r
                .instrument
                .as_ref()
                .and_then(|i| i.multiplier)
                .map(|m| m.value())
                .unwrap_or(Decimal::ONE);
            (q.value() * p.amount.value() * mult, p.currency.clone())
        }
        _ => return None,
    };
    let charges: Decimal = r
        .charges
        .iter()
        .filter(|c| c.amount.currency == currency)
        .map(|c| c.amount.amount.value())
        .sum();
    Some(Money {
        amount: mizan_import::Dec(amount + charges),
        currency,
    })
}

/// Average-cost state for every instrument of one account.
#[derive(Clone, Debug, Default)]
pub struct CostBook {
    /// Keyed by canonical instrument key.
    pub lots: BTreeMap<String, Lot>,
    pub keys: crate::replay::KeyMap,
}

impl CostBook {
    pub fn with_keys(keys: crate::replay::KeyMap) -> Self {
        CostBook {
            keys,
            ..Default::default()
        }
    }

    fn canon(&self, key: String) -> String {
        self.keys.get(&key).cloned().unwrap_or(key)
    }

    fn lot(&mut self, key: &str) -> &mut Lot {
        self.lots.entry(key.to_owned()).or_default()
    }

    /// Apply one record. Records must arrive in processing order.
    pub fn apply(&mut self, r: &ImportRecord) {
        use Kind::*;
        let key = r
            .instrument
            .as_ref()
            .and_then(instrument_key)
            .map(|k| self.canon(k));
        let qty = r.quantity.map(|q| q.value()).unwrap_or_default();

        match r.kind {
            Buy => {
                if let Some(k) = &key {
                    let cost = settled_cost(r);
                    self.lot(k).acquire(qty, cost.as_ref());
                }
            }
            BuyToCover | OptionAssignment => {
                if let Some(k) = &key {
                    let lot = self.lot(k);
                    lot.quantity += qty;
                }
            }
            PositionIn => {
                if let Some(k) = &key {
                    self.lot(k).acquire(qty, r.cost_basis.as_ref());
                }
            }
            Sell | PositionOut | OptionExercise => {
                if let Some(k) = &key {
                    self.lot(k).dispose(qty);
                }
            }
            ShortSell => {
                if let Some(k) = &key {
                    let lot = self.lot(k);
                    if lot.quantity > Decimal::ZERO {
                        lot.dispose(qty);
                    } else {
                        lot.quantity -= qty;
                    }
                }
            }
            OptionExpiry => {
                if let Some(k) = &key {
                    *self.lot(k) = Lot::default();
                }
            }
            Split => {
                if let (Some(k), Some(s)) = (&key, &r.split) {
                    if !s.old.value().is_zero() {
                        let ratio = s.new.value() / s.old.value();
                        let lot = self.lot(k);
                        lot.quantity *= ratio;
                        lot.unknown_units *= ratio;
                    }
                }
            }
            ReturnOfCapital => {
                if let (Some(k), Some(n)) = (&key, &r.net_cash) {
                    let lot = self.lot(k);
                    match &lot.currency {
                        Some(c) if *c != n.currency => lot.mixed_currency = true,
                        _ => lot.cost -= n.amount.value(),
                    }
                }
            }
            Spinoff => self.spinoff(r, key.as_deref()),
            Exchange => self.exchange(r),
            Adjustment => {
                if let (Some(k), Some(q)) = (&key, r.quantity) {
                    let q = q.value();
                    if q >= Decimal::ZERO {
                        self.lot(k).acquire(q, None);
                    } else {
                        self.lot(k).dispose(-q);
                    }
                }
            }
            _ => {}
        }
    }

    fn spinoff(&mut self, r: &ImportRecord, parent: Option<&str>) {
        let (parent_cost, parent_ccy, parent_mixed) = match parent.and_then(|p| self.lots.get(p)) {
            Some(l) => (l.cost, l.currency.clone(), l.mixed_currency),
            None => (Decimal::ZERO, None, false),
        };
        let mut moved_total = Decimal::ZERO;
        for leg in &r.legs {
            let Some(k) = instrument_key(&leg.instrument).map(|k| self.canon(k)) else {
                continue;
            };
            let delta = leg.quantity_delta.value();
            if delta <= Decimal::ZERO {
                self.lot(&k).dispose(-delta);
                continue;
            }
            match (leg.cost_basis_share, &parent_ccy) {
                (Some(share), Some(ccy)) => {
                    let moved = parent_cost * share.value();
                    moved_total += moved;
                    let lot = self.lot(&k);
                    lot.acquire(
                        delta,
                        Some(&Money {
                            amount: mizan_import::Dec(moved),
                            currency: ccy.clone(),
                        }),
                    );
                    lot.mixed_currency |= parent_mixed;
                }
                _ => self.lot(&k).acquire(delta, None),
            }
        }
        if let Some(p) = parent {
            if let Some(l) = self.lots.get_mut(p) {
                l.cost -= moved_total;
            }
        }
    }

    fn exchange(&mut self, r: &ImportRecord) {
        // Removed legs give up their cost pro rata; received legs share it.
        let mut pool = Decimal::ZERO;
        let mut pool_ccy: Option<Code> = None;
        let mut pool_unknown = false;
        let mut received = Vec::new();
        for leg in &r.legs {
            let Some(k) = instrument_key(&leg.instrument).map(|k| self.canon(k)) else {
                continue;
            };
            let delta = leg.quantity_delta.value();
            if delta < Decimal::ZERO {
                let lot = self.lot(&k);
                pool_unknown |= lot.unknown_units > Decimal::ZERO || lot.mixed_currency;
                if pool_ccy.is_none() {
                    pool_ccy = lot.currency.clone();
                }
                pool += lot.dispose(-delta);
            } else if delta > Decimal::ZERO {
                received.push((k, delta, leg.cost_basis_share.map(|s| s.value())));
            }
        }
        let all_shared = received.iter().all(|(_, _, s)| s.is_some());
        let single = received.len() == 1;
        for (k, delta, share) in received {
            let share = if single {
                Some(share.unwrap_or(Decimal::ONE))
            } else {
                share
            };
            match (share, &pool_ccy, pool_unknown, all_shared || single) {
                (Some(s), Some(ccy), false, true) => {
                    let m = Money {
                        amount: mizan_import::Dec(pool * s),
                        currency: ccy.clone(),
                    };
                    self.lot(&k).acquire(delta, Some(&m));
                }
                _ => self.lot(&k).acquire(delta, None),
            }
        }
    }
}
