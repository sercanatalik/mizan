//! Acquisition lots kept oldest first, for first-in first-out matching.
//!
//! Lots keep their original date, quantity, cost and currency through splits,
//! spinoffs, exchanges, returns of capital and transfers. Lots are grouped by
//! a caller-chosen group (an account, or one group for all taxable accounts).

use std::collections::HashMap;

use mizan_import::{Date, Fx, Money};
use mizan_storage::AccountId;
use rust_decimal::Decimal;

use super::events::{money, Event, EventKind};

pub type Group = i64;

#[derive(Clone, Debug, PartialEq)]
pub struct Lot {
    /// Sequence number of the event that created the lot.
    pub origin: usize,
    pub account: AccountId,
    pub qty: Decimal,
    /// `None` when the basis is unknown.
    pub cost: Option<Money>,
    /// Start of the holding period (moved back by US wash-sale tacking).
    pub acquired: Date,
    /// Date the cost was paid, for currency conversion.
    pub cost_date: Date,
    /// The acquisition date was not stated and the activity date was used.
    pub date_assumed: bool,
    /// Rate printed on the acquiring record.
    pub fx: Option<Fx>,
    /// Extra basis in a report currency (US wash-sale adjustments).
    pub adjust: Decimal,
}

impl Lot {
    /// Split off `qty` units, moving cost and adjustment pro rata.
    fn take(&mut self, qty: Decimal) -> Lot {
        let of = |x: Decimal| {
            if self.qty.is_zero() {
                Decimal::ZERO
            } else {
                x * qty / self.qty
            }
        };
        let mut part = self.clone();
        part.qty = qty;
        part.cost = self
            .cost
            .as_ref()
            .map(|c| money(of(c.amount.value()), c.currency.clone()));
        part.adjust = of(self.adjust);
        self.qty -= qty;
        if let Some(c) = &mut self.cost {
            c.amount =
                mizan_import::Dec(c.amount.value() - part.cost.as_ref().unwrap().amount.value());
        }
        self.adjust -= part.adjust;
        part
    }

    fn scale_cost(&mut self, f: Decimal) {
        if let Some(c) = &mut self.cost {
            c.amount = mizan_import::Dec(c.amount.value() * f);
        }
        self.adjust *= f;
    }
}

/// Lots removed by a disposal, and the units sold beyond what was held.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Taken {
    pub lots: Vec<Lot>,
    pub shortfall: Decimal,
}

#[derive(Clone, Debug, Default)]
pub struct FifoBook {
    lots: HashMap<(Group, String), Vec<Lot>>,
    /// Lots in transit, by transfer pair.
    transit: HashMap<usize, Vec<Lot>>,
}

impl FifoBook {
    pub fn lots(&self, group: Group, key: &str) -> &[Lot] {
        self.lots
            .get(&(group, key.to_owned()))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn held(&self, group: Group, key: &str) -> Decimal {
        self.lots(group, key).iter().map(|l| l.qty).sum()
    }

    fn queue(&mut self, group: Group, key: &str) -> &mut Vec<Lot> {
        self.lots.entry((group, key.to_owned())).or_default()
    }

    /// Add a lot, keeping the queue ordered by acquisition date.
    pub fn insert(&mut self, group: Group, key: &str, lot: Lot) {
        if lot.qty <= Decimal::ZERO {
            return;
        }
        let q = self.queue(group, key);
        let at = q.partition_point(|l| (l.acquired, l.origin) <= (lot.acquired, lot.origin));
        q.insert(at, lot);
    }

    /// Remove `qty` units, oldest first.
    pub fn take(&mut self, group: Group, key: &str, qty: Decimal) -> Taken {
        let q = self.queue(group, key);
        let mut left = qty;
        let mut out = Vec::new();
        while left > Decimal::ZERO && !q.is_empty() {
            if q[0].qty <= left {
                let lot = q.remove(0);
                left -= lot.qty;
                out.push(lot);
            } else {
                out.push(q[0].take(left));
                left = Decimal::ZERO;
            }
        }
        Taken {
            lots: out,
            shortfall: left,
        }
    }

    /// Mutable access to lots created by event `origin`, in any group.
    pub fn lots_from_mut(&mut self, origin: usize) -> impl Iterator<Item = &mut Lot> {
        self.lots
            .values_mut()
            .flat_map(|q| q.iter_mut())
            .filter(move |l| l.origin == origin)
    }

    /// Split `qty` units off the first lot created by event `origin` and
    /// return them with their group and key; the caller re-inserts them.
    pub fn split_origin(&mut self, origin: usize, qty: Decimal) -> Option<(Group, String, Lot)> {
        for ((g, k), q) in self.lots.iter_mut() {
            if let Some(i) = q.iter().position(|l| l.origin == origin) {
                let part = if q[i].qty <= qty {
                    q.remove(i)
                } else {
                    q[i].take(qty)
                };
                return Some((*g, k.clone(), part));
            }
        }
        None
    }

    /// Units still held from lots created by event `origin`.
    pub fn held_from(&self, origin: usize) -> Decimal {
        self.lots
            .values()
            .flat_map(|q| q.iter())
            .filter(|l| l.origin == origin)
            .map(|l| l.qty)
            .sum()
    }

    /// Re-sort a queue after acquisition dates changed.
    pub fn resort(&mut self) {
        for q in self.lots.values_mut() {
            q.sort_by_key(|l| (l.acquired, l.origin));
        }
    }

    /// Apply one event. A disposal removes and returns its lots; every other
    /// event returns `None`.
    pub fn apply(&mut self, ev: &Event, group: Group) -> Option<Taken> {
        let key = ev.key.as_str();
        match &ev.kind {
            EventKind::Acquire { qty, cost, fx } => {
                self.insert(
                    group,
                    key,
                    Lot {
                        origin: ev.seq,
                        account: ev.account,
                        qty: *qty,
                        cost: cost.clone(),
                        acquired: ev.date,
                        cost_date: ev.date,
                        date_assumed: false,
                        fx: fx.clone(),
                        adjust: Decimal::ZERO,
                    },
                );
                None
            }
            EventKind::Dispose { qty, .. } => Some(self.take(group, key, *qty)),
            EventKind::Remove { qty } => {
                self.take(group, key, *qty);
                None
            }
            EventKind::Split { ratio } => {
                for l in self.queue(group, key) {
                    l.qty *= *ratio;
                }
                None
            }
            EventKind::ReturnOfCapital { amount } => {
                let q = self.queue(group, key);
                let total: Decimal = q
                    .iter()
                    .filter(|l| {
                        l.cost
                            .as_ref()
                            .is_some_and(|c| c.currency == amount.currency)
                    })
                    .map(|l| l.qty)
                    .sum();
                if total > Decimal::ZERO {
                    for l in q.iter_mut() {
                        if let Some(c) = &mut l.cost {
                            if c.currency == amount.currency {
                                let cut = amount.amount.value() * l.qty / total;
                                c.amount = mizan_import::Dec(c.amount.value() - cut);
                            }
                        }
                    }
                }
                None
            }
            EventKind::Reorg {
                spinoff,
                out,
                received,
            } => {
                if *spinoff {
                    self.spinoff(group, key, received, ev);
                } else {
                    self.exchange(group, out, received, ev);
                }
                None
            }
            EventKind::TransferOut { qty, pair } => {
                let taken = self.take(group, key, *qty);
                if let Some(p) = pair {
                    self.transit.insert(*p, taken.lots);
                }
                None
            }
            EventKind::TransferIn {
                qty,
                cost,
                acquired,
                pair,
            } => {
                match pair.and_then(|p| self.transit.remove(&p)) {
                    Some(lots) => {
                        for mut l in lots {
                            l.account = ev.account;
                            self.insert(group, key, l);
                        }
                    }
                    None => self.insert(
                        group,
                        key,
                        Lot {
                            origin: ev.seq,
                            account: ev.account,
                            qty: *qty,
                            cost: cost.clone(),
                            acquired: acquired.unwrap_or(ev.date),
                            cost_date: acquired.unwrap_or(ev.date),
                            date_assumed: acquired.is_none(),
                            fx: None,
                            adjust: Decimal::ZERO,
                        },
                    ),
                }
                None
            }
            EventKind::NotCovered { .. } => None,
        }
    }

    fn spinoff(
        &mut self,
        group: Group,
        parent: &str,
        received: &[(String, Decimal, Option<Decimal>)],
        ev: &Event,
    ) {
        let parents: Vec<Lot> = self.lots(group, parent).to_vec();
        let total: Decimal = parents.iter().map(|l| l.qty).sum();
        let mut moved = Decimal::ZERO;
        for (k, qty, share) in received {
            if total.is_zero() {
                self.insert(group, k, unknown_lot(ev, *qty));
                continue;
            }
            for p in &parents {
                let mut lot = p.clone();
                lot.qty = *qty * p.qty / total;
                lot.fx = None;
                match share {
                    Some(s) => lot.scale_cost(*s),
                    None => {
                        lot.cost = None;
                        lot.adjust = Decimal::ZERO;
                    }
                }
                self.insert(group, k, lot);
            }
            moved += share.unwrap_or_default();
        }
        for l in self.queue(group, parent) {
            l.scale_cost(Decimal::ONE - moved);
        }
    }

    fn exchange(
        &mut self,
        group: Group,
        out: &[(String, Decimal)],
        received: &[(String, Decimal, Option<Decimal>)],
        ev: &Event,
    ) {
        let mut removed = Vec::new();
        for (k, qty) in out {
            removed.extend(self.take(group, k, *qty).lots);
        }
        let total: Decimal = removed.iter().map(|l| l.qty).sum();
        let single = received.len() == 1;
        for (k, qty, share) in received {
            let share = if single {
                Some(share.unwrap_or(Decimal::ONE))
            } else {
                *share
            };
            if total.is_zero() {
                self.insert(group, k, unknown_lot(ev, *qty));
                continue;
            }
            for r in &removed {
                let mut lot = r.clone();
                lot.qty = *qty * r.qty / total;
                lot.fx = None;
                match share {
                    Some(s) => lot.scale_cost(s),
                    None => {
                        lot.cost = None;
                        lot.adjust = Decimal::ZERO;
                    }
                }
                self.insert(group, k, lot);
            }
        }
    }
}

fn unknown_lot(ev: &Event, qty: Decimal) -> Lot {
    Lot {
        origin: ev.seq,
        account: ev.account,
        qty,
        cost: None,
        acquired: ev.date,
        cost_date: ev.date,
        date_assumed: false,
        fx: None,
        adjust: Decimal::ZERO,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replay::KeyMap;
    use crate::tax::events::events;
    use crate::tax::testing::*;
    use mizan_import::{Dec, Kind, Leg, Link, LinkRole, SplitRatio};

    fn run(accounts: &[(AccountId, Vec<mizan_storage::StoredActivity>)]) -> (FifoBook, Vec<Taken>) {
        let mut book = FifoBook::default();
        let mut taken = Vec::new();
        for ev in events(accounts, &KeyMap::new()) {
            if let Some(t) = book.apply(&ev, ev.account) {
                taken.push(t);
            }
        }
        (book, taken)
    }

    #[test]
    fn buy_creates_a_lot_with_cost_and_date() {
        let (book, _) = run(&[(
            1,
            vec![stored(
                1,
                trade(
                    Kind::Buy,
                    "2025-01-02",
                    "AAPL",
                    "10",
                    "100.00",
                    "USD",
                    Some("1.00"),
                ),
            )],
        )]);
        let lot = &book.lots(1, &key_of("AAPL"))[0];
        assert_eq!(lot.qty, d("10"));
        assert_eq!(lot.cost, Some(m("1001.00", "USD")));
        assert_eq!(lot.acquired, day("2025-01-02"));
    }

    #[test]
    fn disposal_takes_oldest_lots_first_and_splits_cost() {
        let (book, taken) = run(&[(
            1,
            vec![
                stored(1, deal(Kind::Buy, "2023-01-10", "X", "10", "1000", "USD")),
                stored(1, deal(Kind::Buy, "2024-06-01", "X", "10", "1500", "USD")),
                stored(1, deal(Kind::Sell, "2025-02-01", "X", "15", "3000", "USD")),
            ],
        )]);
        let t = &taken[0];
        assert_eq!(t.lots.len(), 2);
        assert_eq!(t.lots[0].cost, Some(m("1000", "USD")));
        assert_eq!(t.lots[1].qty, d("5"));
        assert_eq!(t.lots[1].cost.as_ref().unwrap().amount.value(), d("750"));
        let left = &book.lots(1, &key_of("X"))[0];
        assert_eq!(
            (left.qty, left.cost.as_ref().unwrap().amount.value()),
            (d("5"), d("750"))
        );
    }

    #[test]
    fn split_changes_quantity_only() {
        let mut s = record(Kind::Split, "2024-01-05", "X");
        s.split = Some(SplitRatio {
            new: Dec(d("4")),
            old: Dec(d("1")),
        });
        let (book, _) = run(&[(
            1,
            vec![
                stored(
                    1,
                    deal(Kind::Buy, "2022-04-01", "X", "10", "1000.00", "USD"),
                ),
                stored(1, s),
            ],
        )]);
        let lot = &book.lots(1, &key_of("X"))[0];
        assert_eq!(lot.qty, d("40"));
        assert_eq!(lot.cost, Some(m("1000.00", "USD")));
        assert_eq!(lot.acquired, day("2022-04-01"));
    }

    #[test]
    fn spinoff_moves_cost_share_and_keeps_dates() {
        let mut s = record(Kind::Spinoff, "2024-03-01", "PARENT");
        s.legs = vec![Leg {
            instrument: hint("CHILD"),
            quantity_delta: Dec(d("5")),
            cost_basis_share: Some(Dec(d("0.2"))),
        }];
        let (book, _) = run(&[(
            1,
            vec![
                stored(
                    1,
                    deal(Kind::Buy, "2020-01-02", "PARENT", "10", "1000", "USD"),
                ),
                stored(
                    1,
                    deal(Kind::Buy, "2021-01-04", "PARENT", "10", "3000", "USD"),
                ),
                stored(1, s),
            ],
        )]);
        let child = book.lots(1, &key_of("CHILD"));
        assert_eq!(child.len(), 2);
        assert_eq!(child[0].acquired, day("2020-01-02"));
        assert_eq!(child[0].qty, d("2.5"));
        assert_eq!(child[0].cost.as_ref().unwrap().amount.value(), d("200"));
        assert_eq!(child[1].cost.as_ref().unwrap().amount.value(), d("600"));
        let parent = book.lots(1, &key_of("PARENT"));
        assert_eq!(parent[0].cost.as_ref().unwrap().amount.value(), d("800"));
        assert_eq!(parent[1].cost.as_ref().unwrap().amount.value(), d("2400"));
    }

    #[test]
    fn exchange_moves_cost_and_dates_to_the_new_instrument() {
        let mut x = record(Kind::Exchange, "2024-03-01", "OLD");
        x.legs = vec![
            Leg {
                instrument: hint("OLD"),
                quantity_delta: Dec(d("-10")),
                cost_basis_share: None,
            },
            Leg {
                instrument: hint("NEW"),
                quantity_delta: Dec(d("5")),
                cost_basis_share: None,
            },
        ];
        let (book, _) = run(&[(
            1,
            vec![
                stored(1, deal(Kind::Buy, "2020-01-02", "OLD", "10", "1000", "USD")),
                stored(1, x),
            ],
        )]);
        assert!(book.lots(1, &key_of("OLD")).is_empty());
        let new = &book.lots(1, &key_of("NEW"))[0];
        assert_eq!((new.qty, new.acquired), (d("5"), day("2020-01-02")));
        assert_eq!(new.cost.as_ref().unwrap().amount.value(), d("1000"));
    }

    #[test]
    fn return_of_capital_reduces_cost() {
        let mut r = record(Kind::ReturnOfCapital, "2024-03-01", "X");
        r.net_cash = Some(m("100", "USD"));
        let (book, _) = run(&[(
            1,
            vec![
                stored(1, deal(Kind::Buy, "2020-01-02", "X", "10", "1000", "USD")),
                stored(1, r),
            ],
        )]);
        assert_eq!(
            book.lots(1, &key_of("X"))[0]
                .cost
                .as_ref()
                .unwrap()
                .amount
                .value(),
            d("900")
        );
    }

    #[test]
    fn linked_transfer_moves_lots_with_dates_and_cost() {
        let mut out = position(Kind::PositionOut, "2025-02-01", "X", "50");
        out.external_id = Some("T1".into());
        let mut inn = position(Kind::PositionIn, "2025-02-01", "X", "50");
        inn.cost_basis = Some(m("9999", "USD"));
        inn.links = vec![Link {
            role: LinkRole::TransferPair,
            target: "T1".into(),
        }];
        let (book, taken) = run(&[
            (
                1,
                vec![
                    stored(1, deal(Kind::Buy, "2021-05-03", "X", "50", "2500", "USD")),
                    stored(1, out),
                ],
            ),
            (2, vec![stored(2, inn)]),
        ]);
        assert!(taken.is_empty(), "a transfer is not a disposal");
        assert!(book.lots(1, &key_of("X")).is_empty());
        let lot = &book.lots(2, &key_of("X"))[0];
        assert_eq!((lot.account, lot.acquired), (2, day("2021-05-03")));
        assert_eq!(lot.cost, Some(m("2500", "USD")));
    }

    #[test]
    fn unlinked_transfer_in_uses_its_own_basis_or_unknown() {
        let mut with = position(Kind::PositionIn, "2025-02-01", "X", "10");
        with.cost_basis = Some(m("500", "USD"));
        with.acquisition_date = Some(day("2019-07-01"));
        let without = position(Kind::PositionIn, "2025-02-01", "Y", "30");
        let (book, _) = run(&[(1, vec![stored(1, with), stored(1, without)])]);
        let x = &book.lots(1, &key_of("X"))[0];
        assert_eq!((x.acquired, x.date_assumed), (day("2019-07-01"), false));
        assert_eq!(x.cost, Some(m("500", "USD")));
        let y = &book.lots(1, &key_of("Y"))[0];
        assert_eq!(
            (y.qty, y.cost.clone(), y.date_assumed),
            (d("30"), None, true)
        );
    }

    #[test]
    fn selling_more_than_held_reports_a_shortfall() {
        let (_, taken) = run(&[(
            1,
            vec![
                stored(1, deal(Kind::Buy, "2025-01-02", "X", "5", "500", "USD")),
                stored(1, deal(Kind::Sell, "2025-02-02", "X", "8", "900", "USD")),
            ],
        )]);
        assert_eq!(taken[0].shortfall, d("3"));
    }
}
