//! Stored activities as a flat, chronological stream of lot events.
//!
//! Amounts stay in their original currency; each country's rules convert them.
//! Buys carry their charges in the cost; sales carry proceeds net of charges.

use std::collections::HashMap;

use mizan_import::{ClassHint, Code, Date, Fx, ImportRecord, Kind, LinkRole, Money};
use mizan_storage::{AccountId, StoredActivity, TaxTreatment};
use rust_decimal::Decimal;

use crate::portfolio::ordered;
use crate::replay::KeyMap;
use crate::resolve::instrument_key;

/// An account as the tax rules see it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaxAccount {
    pub id: AccountId,
    pub name: String,
    pub treatment: TaxTreatment,
}

/// What a stream event does to a holding.
#[derive(Clone, Debug, PartialEq)]
pub enum EventKind {
    /// Units bought; `cost` is `None` when the basis is unknown.
    Acquire {
        qty: Decimal,
        cost: Option<Money>,
        fx: Option<Fx>,
    },
    /// Units sold; `proceeds` is net of charges, `None` when not stated.
    Dispose {
        qty: Decimal,
        proceeds: Option<Money>,
        fx: Option<Fx>,
    },
    /// Units removed without a sale (negative adjustment).
    Remove { qty: Decimal },
    /// Quantity multiplied by `ratio`; cost and dates unchanged.
    Split { ratio: Decimal },
    /// A spinoff (`parent` is the event's key, which keeps its units) or an
    /// exchange (`out` legs give up their units). Received legs take a share
    /// of the parent's or the removed units' cost.
    Reorg {
        spinoff: bool,
        out: Vec<(String, Decimal)>,
        received: Vec<(String, Decimal, Option<Decimal>)>,
    },
    /// Cost of the holding reduced by `amount`.
    ReturnOfCapital { amount: Money },
    /// Units leaving the account. `pair` links it to a [`EventKind::TransferIn`].
    TransferOut { qty: Decimal, pair: Option<usize> },
    /// Units arriving. Without a matching out, the record's own basis and
    /// acquisition date are used.
    TransferIn {
        qty: Decimal,
        cost: Option<Money>,
        acquired: Option<Date>,
        pair: Option<usize>,
    },
    /// A disposal or acquisition the tax rules do not cover yet.
    NotCovered { what: &'static str, disposal: bool },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    /// Position in the stream; stable identifier of the event.
    pub seq: usize,
    pub date: Date,
    pub account: AccountId,
    /// Canonical instrument key.
    pub key: String,
    pub kind: EventKind,
}

/// The cash a buy settled for: net cash, else gross plus charges, else
/// quantity × price × multiplier plus charges.
pub fn buy_cost(r: &ImportRecord) -> Option<Money> {
    if let Some(n) = &r.net_cash {
        return Some(n.clone());
    }
    let (amount, currency) = gross_of(r)?;
    Some(money(amount + charges_in(r, &currency), currency))
}

/// What a sale brought in: net cash, else gross minus charges, else
/// quantity × price × multiplier minus charges.
pub fn sale_proceeds(r: &ImportRecord) -> Option<Money> {
    if let Some(n) = &r.net_cash {
        return Some(n.clone());
    }
    let (amount, currency) = gross_of(r)?;
    Some(money(amount - charges_in(r, &currency), currency))
}

fn gross_of(r: &ImportRecord) -> Option<(Decimal, Code)> {
    match (&r.gross, r.quantity, &r.price) {
        (Some(g), _, _) => Some((g.amount.value(), g.currency.clone())),
        (None, Some(q), Some(p)) => {
            let mult = r
                .instrument
                .as_ref()
                .and_then(|i| i.multiplier)
                .map(|m| m.value())
                .unwrap_or(Decimal::ONE);
            Some((q.value() * p.amount.value() * mult, p.currency.clone()))
        }
        _ => None,
    }
}

fn charges_in(r: &ImportRecord, currency: &Code) -> Decimal {
    r.charges
        .iter()
        .filter(|c| c.amount.currency == *currency)
        .map(|c| c.amount.amount.value())
        .sum()
}

pub fn money(amount: Decimal, currency: Code) -> Money {
    Money {
        amount: mizan_import::Dec(amount),
        currency,
    }
}

fn is_crypto(r: &ImportRecord) -> bool {
    r.instrument
        .as_ref()
        .and_then(|i| i.class_hint)
        .is_some_and(|c| c == ClassHint::Crypto)
}

/// Transfer pairs: a `position_out` and a `position_in` joined by a
/// `transfer_pair` link from either side to the other's external id.
fn transfer_pairs(records: &[(AccountId, &ImportRecord)]) -> HashMap<usize, usize> {
    let by_ext: HashMap<&str, usize> = records
        .iter()
        .enumerate()
        .filter_map(|(i, (_, r))| r.external_id.as_deref().map(|e| (e, i)))
        .collect();
    let mut pair = HashMap::new();
    let mut next = 0;
    for (i, (_, r)) in records.iter().enumerate() {
        for l in r.links.iter().filter(|l| l.role == LinkRole::TransferPair) {
            let Some(&j) = by_ext.get(l.target.as_str()) else {
                continue;
            };
            let kinds = (r.kind, records[j].1.kind);
            let opposite = matches!(
                kinds,
                (Kind::PositionIn, Kind::PositionOut) | (Kind::PositionOut, Kind::PositionIn)
            );
            if opposite && !pair.contains_key(&i) && !pair.contains_key(&j) {
                pair.insert(i, next);
                pair.insert(j, next);
                next += 1;
            }
        }
    }
    pair
}

/// Build the event stream for the given accounts' stored activities.
pub fn events(accounts: &[(AccountId, Vec<StoredActivity>)], keys: &KeyMap) -> Vec<Event> {
    let canon = |k: String| keys.get(&k).cloned().unwrap_or(k);
    // Each account's records in processing order, then all accounts by date.
    let mut records: Vec<(AccountId, &ImportRecord)> = Vec::new();
    for (id, stored) in accounts {
        records.extend(ordered(stored).into_iter().map(|r| (*id, r)));
    }
    let pairs = transfer_pairs(&records);
    let mut order: Vec<usize> = (0..records.len()).collect();
    // Stable: by date, and a paired transfer in after the same day's out.
    order.sort_by_key(|&i| {
        let r = records[i].1;
        let late = r.kind == Kind::PositionIn && pairs.contains_key(&i);
        (r.date, late)
    });

    let mut out = Vec::new();
    for i in order {
        let (account, r) = records[i];
        let key = r.instrument.as_ref().and_then(instrument_key).map(canon);
        let qty = r.quantity.map(|q| q.value()).unwrap_or_default();
        let mut push = |key: String, kind: EventKind| {
            out.push(Event {
                seq: out.len(),
                date: r.date,
                account,
                key,
                kind,
            })
        };
        let Some(key) = key else { continue };
        let crypto = is_crypto(r);
        use Kind::*;
        let kind = match r.kind {
            Buy | Sell if crypto => EventKind::NotCovered {
                what: "crypto",
                disposal: r.kind == Sell,
            },
            Buy => EventKind::Acquire {
                qty,
                cost: buy_cost(r),
                fx: r.fx.clone(),
            },
            Sell => EventKind::Dispose {
                qty,
                proceeds: sale_proceeds(r),
                fx: r.fx.clone(),
            },
            ShortSell => EventKind::NotCovered {
                what: "short sale",
                disposal: true,
            },
            BuyToCover => EventKind::NotCovered {
                what: "short sale",
                disposal: false,
            },
            OptionExercise | OptionAssignment | OptionExpiry => EventKind::NotCovered {
                what: "option",
                disposal: true,
            },
            Reward => EventKind::Acquire {
                qty,
                cost: None,
                fx: None,
            },
            PositionIn => EventKind::TransferIn {
                qty,
                cost: r.cost_basis.clone(),
                acquired: r.acquisition_date,
                pair: pairs.get(&i).copied(),
            },
            PositionOut => EventKind::TransferOut {
                qty,
                pair: pairs.get(&i).copied(),
            },
            Split => match &r.split {
                Some(s) if !s.old.value().is_zero() => EventKind::Split {
                    ratio: s.new.value() / s.old.value(),
                },
                _ => continue,
            },
            ReturnOfCapital => match &r.net_cash {
                Some(n) => EventKind::ReturnOfCapital { amount: n.clone() },
                None => continue,
            },
            Spinoff | Exchange => {
                let mut outs = Vec::new();
                let mut received = Vec::new();
                for leg in &r.legs {
                    let Some(k) = instrument_key(&leg.instrument).map(canon) else {
                        continue;
                    };
                    let d = leg.quantity_delta.value();
                    if d < Decimal::ZERO {
                        outs.push((k, -d));
                    } else if d > Decimal::ZERO {
                        received.push((k, d, leg.cost_basis_share.map(|s| s.value())));
                    }
                }
                EventKind::Reorg {
                    spinoff: r.kind == Spinoff,
                    out: outs,
                    received,
                }
            }
            Adjustment => match r.quantity.map(|q| q.value()) {
                Some(q) if q > Decimal::ZERO => EventKind::Acquire {
                    qty: q,
                    cost: None,
                    fx: None,
                },
                Some(q) if q < Decimal::ZERO => EventKind::Remove { qty: -q },
                _ => continue,
            },
            _ => continue,
        };
        push(key, kind);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tax::testing::*;

    #[test]
    fn buy_with_commission_costs_price_plus_charges() {
        let r = trade(
            Kind::Buy,
            "2025-01-02",
            "AAPL",
            "10",
            "100.00",
            "USD",
            Some("1.00"),
        );
        assert_eq!(buy_cost(&r), Some(m("1001.00", "USD")));
    }

    #[test]
    fn sale_with_commission_brings_price_minus_charges() {
        let r = trade(
            Kind::Sell,
            "2025-01-02",
            "AAPL",
            "10",
            "120.00",
            "USD",
            Some("1.00"),
        );
        assert_eq!(sale_proceeds(&r), Some(m("1199.00", "USD")));
    }

    #[test]
    fn net_cash_wins_when_stated() {
        let mut r = trade(
            Kind::Buy,
            "2025-01-02",
            "AAPL",
            "10",
            "100.00",
            "USD",
            Some("1.00"),
        );
        r.net_cash = Some(m("1002.50", "USD"));
        assert_eq!(buy_cost(&r), Some(m("1002.50", "USD")));
    }

    #[test]
    fn stream_is_chronological_across_accounts_with_merged_keys() {
        let a = vec![stored(
            1,
            trade(Kind::Buy, "2025-03-01", "AAPL", "5", "10", "USD", None),
        )];
        let b = vec![
            stored(
                2,
                trade(Kind::Buy, "2025-01-01", "APPLE", "5", "10", "USD", None),
            ),
            stored(
                2,
                trade(Kind::Sell, "2025-05-01", "AAPL", "2", "12", "USD", None),
            ),
        ];
        let mut keys = KeyMap::new();
        keys.insert(key_of("APPLE"), key_of("AAPL"));
        let ev = events(&[(1, a), (2, b)], &keys);
        let dates: Vec<String> = ev.iter().map(|e| e.date.to_string()).collect();
        assert_eq!(dates, ["2025-01-01", "2025-03-01", "2025-05-01"]);
        assert!(ev.iter().all(|e| e.key == key_of("AAPL")));
        assert!(matches!(ev[2].kind, EventKind::Dispose { .. }));
    }

    #[test]
    fn linked_transfers_are_paired_and_in_follows_out() {
        let mut out = position(Kind::PositionOut, "2025-02-01", "AAPL", "50");
        out.external_id = Some("T1-out".into());
        let mut inn = position(Kind::PositionIn, "2025-02-01", "AAPL", "50");
        inn.links = vec![mizan_import::Link {
            role: LinkRole::TransferPair,
            target: "T1-out".into(),
        }];
        // The receiving account comes first in the input.
        let ev = events(
            &[(2, vec![stored(2, inn)]), (1, vec![stored(1, out)])],
            &KeyMap::new(),
        );
        assert!(matches!(
            ev[0].kind,
            EventKind::TransferOut { pair: Some(0), .. }
        ));
        assert!(matches!(
            ev[1].kind,
            EventKind::TransferIn { pair: Some(0), .. }
        ));
    }

    #[test]
    fn options_shorts_and_crypto_are_not_covered() {
        let mut c = trade(Kind::Sell, "2025-01-02", "BTC", "1", "100", "USD", None);
        c.instrument.as_mut().unwrap().class_hint = Some(ClassHint::Crypto);
        let ev = events(
            &[(
                1,
                vec![
                    stored(1, c),
                    stored(
                        1,
                        position(Kind::OptionExpiry, "2025-01-03", "AAPL 250117C", "1"),
                    ),
                    stored(1, position(Kind::ShortSell, "2025-01-04", "TSLA", "1")),
                ],
            )],
            &KeyMap::new(),
        );
        let what: Vec<&str> = ev
            .iter()
            .map(|e| match e.kind {
                EventKind::NotCovered { what, .. } => what,
                _ => "",
            })
            .collect();
        assert_eq!(what, ["crypto", "option", "short sale"]);
    }
}
