//! UK share identification (TCGA 1992 s.105, s.106A, s.104).
//!
//! Disposals are matched across all taxable accounts: first to acquisitions of
//! the same instrument on the same day, then to acquisitions in the following
//! 30 days (earliest first), then to the pool of everything else at average
//! cost. All acquisitions on one day count as one, and so do all disposals.
//! Amounts are converted to GBP on each transaction's date.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use chrono::Duration;
use mizan_import::{Code, Date};
use mizan_storage::AccountId;
use rust_decimal::Decimal;

use super::events::{Event, EventKind};
use super::rates::Rates;
use super::rows::{convert, taxable_pairs, GainRow, Missing, Rule, Treatments};
use super::{Country, TaxYear};

type Amount = Result<Decimal, Missing>;

fn add(a: &Amount, b: &Amount) -> Amount {
    match (a, b) {
        (Ok(x), Ok(y)) => Ok(x + y),
        (Err(Missing::CostUnknown), _) | (_, Err(Missing::CostUnknown)) => {
            Err(Missing::CostUnknown)
        }
        (Err(e), _) | (_, Err(e)) => Err(e.clone()),
    }
}

fn part(a: &Amount, f: Decimal) -> Amount {
    a.clone().map(|x| x * f)
}

/// `a × num ÷ den`, multiplying first to keep precision.
fn share(a: &Amount, num: Decimal, den: Decimal) -> Amount {
    a.clone().map(|x| {
        if den.is_zero() {
            Decimal::ZERO
        } else {
            x * num / den
        }
    })
}

/// All acquisitions of one instrument on one day.
#[derive(Clone, Debug)]
struct AcqDay {
    date: Date,
    qty: Decimal,
    cost: Amount,
    /// Units not yet matched to a disposal.
    remaining: Decimal,
}

impl AcqDay {
    fn cost_of(&self, qty: Decimal) -> Amount {
        share(&self.cost, qty, self.qty)
    }
}

/// All disposals of one instrument on one day.
#[derive(Clone, Debug)]
struct DispDay {
    qty: Decimal,
    proceeds: Amount,
    accounts: BTreeSet<AccountId>,
}

#[derive(Clone, Debug)]
struct Pool {
    qty: Decimal,
    cost: Amount,
}

impl Default for Pool {
    fn default() -> Self {
        Pool {
            qty: Decimal::ZERO,
            cost: Ok(Decimal::ZERO),
        }
    }
}

impl Pool {
    fn add(&mut self, qty: Decimal, cost: &Amount) {
        if self.qty.is_zero() {
            self.cost = Ok(Decimal::ZERO);
        }
        self.qty += qty;
        self.cost = add(&self.cost, cost);
    }

    /// Remove `qty` units at average cost; returns the cost removed.
    fn take(&mut self, qty: Decimal) -> Amount {
        if self.qty <= Decimal::ZERO {
            return Err(Missing::CostUnknown);
        }
        let qty = qty.min(self.qty);
        let out = share(&self.cost, qty, self.qty);
        self.cost = add(&self.cost, &out.clone().map(|x| -x));
        self.qty -= qty;
        if self.qty.is_zero() {
            self.cost = Ok(Decimal::ZERO);
        }
        out
    }
}

/// Match every disposal under the UK rules and return the rows dated in `year`.
pub fn match_disposals(
    events: &[Event],
    t: &Treatments,
    rates: &Rates,
    year: TaxYear,
) -> Vec<GainRow> {
    let gbp: Code = Country::Uk.currency();
    let pairs = taxable_pairs(events, t);
    let inside = |e: &Event, pair: &Option<usize>| {
        pair.is_some_and(|p| pairs.get(&p) == Some(&true)) && t.taxable(e.account)
    };

    // Acquisitions per instrument and day, known up front for the 30-day rule.
    let mut acq: HashMap<String, Vec<AcqDay>> = HashMap::new();
    let mut add_acq = |key: &str, date: Date, qty: Decimal, cost: Amount| {
        let days = acq.entry(key.to_owned()).or_default();
        match days.last_mut() {
            Some(a) if a.date == date => {
                a.qty += qty;
                a.remaining += qty;
                a.cost = add(&a.cost, &cost);
            }
            _ => days.push(AcqDay {
                date,
                qty,
                cost,
                remaining: qty,
            }),
        }
    };
    for e in events.iter().filter(|e| t.taxable(e.account)) {
        match &e.kind {
            EventKind::Acquire { qty, cost, fx } => add_acq(
                &e.key,
                e.date,
                *qty,
                convert(rates, cost.as_ref(), &gbp, e.date, fx.as_ref()),
            ),
            EventKind::TransferIn {
                qty, cost, pair, ..
            } if !inside(e, pair) => add_acq(
                &e.key,
                e.date,
                *qty,
                convert(rates, cost.as_ref(), &gbp, e.date, None),
            ),
            _ => {}
        }
    }

    let mut by_day: BTreeMap<Date, Vec<&Event>> = BTreeMap::new();
    for e in events.iter().filter(|e| t.taxable(e.account)) {
        by_day.entry(e.date).or_default().push(e);
    }

    let mut pools: HashMap<String, Pool> = HashMap::new();
    let mut rows = Vec::new();
    for (date, day) in by_day {
        let emit = year.contains(date);
        // Corporate actions and removals first, in record order.
        for e in &day {
            match &e.kind {
                EventKind::Split { ratio } => {
                    pools.entry(e.key.clone()).or_default().qty *= *ratio;
                }
                EventKind::ReturnOfCapital { amount } => {
                    let cut = convert(rates, Some(amount), &gbp, date, None);
                    let p = pools.entry(e.key.clone()).or_default();
                    p.cost = add(&p.cost, &cut.map(|x| -x));
                }
                EventKind::Remove { qty } => {
                    let _ = pools.entry(e.key.clone()).or_default().take(*qty);
                }
                EventKind::TransferOut { qty, pair } if !inside(e, pair) => {
                    // Into an ISA or pension, or out of Mizan's view: no gain.
                    let _ = pools.entry(e.key.clone()).or_default().take(*qty);
                }
                EventKind::Reorg {
                    spinoff,
                    out,
                    received,
                } => {
                    if *spinoff {
                        let parent = pools.entry(e.key.clone()).or_default().clone();
                        let mut moved = Decimal::ZERO;
                        for (k, q, share) in received {
                            let cost = match share {
                                Some(s) if parent.qty > Decimal::ZERO => part(&parent.cost, *s),
                                _ => Err(Missing::CostUnknown),
                            };
                            pools.entry(k.clone()).or_default().add(*q, &cost);
                            moved += share.unwrap_or_default();
                        }
                        let p = pools.entry(e.key.clone()).or_default();
                        p.cost = part(&p.cost, Decimal::ONE - moved);
                    } else {
                        let mut removed: Amount = Ok(Decimal::ZERO);
                        for (k, q) in out {
                            let c = pools.entry(k.clone()).or_default().take(*q);
                            removed = add(&removed, &c);
                        }
                        let single = received.len() == 1;
                        for (k, q, share) in received {
                            let share = if single {
                                Some(share.unwrap_or(Decimal::ONE))
                            } else {
                                *share
                            };
                            let cost = match share {
                                Some(s) => part(&removed, s),
                                None => Err(Missing::CostUnknown),
                            };
                            pools.entry(k.clone()).or_default().add(*q, &cost);
                        }
                    }
                }
                _ => {}
            }
        }

        // Disposals of the day, one per instrument.
        let mut disp: BTreeMap<String, DispDay> = BTreeMap::new();
        for e in &day {
            match &e.kind {
                EventKind::Dispose { qty, proceeds, fx } => {
                    let p = convert(rates, proceeds.as_ref(), &gbp, date, fx.as_ref());
                    let d = disp.entry(e.key.clone()).or_insert(DispDay {
                        qty: Decimal::ZERO,
                        proceeds: Ok(Decimal::ZERO),
                        accounts: BTreeSet::new(),
                    });
                    d.qty += qty;
                    d.proceeds = add(&d.proceeds, &p);
                    d.accounts.insert(e.account);
                }
                EventKind::NotCovered {
                    what,
                    disposal: true,
                } if emit => {
                    rows.push(GainRow::not_covered(&e.key, date, e.account, what));
                }
                _ => {}
            }
        }
        for (key, d) in disp {
            let days = acq.entry(key.clone()).or_default();
            let mut left = d.qty;
            let mut push = |rule: Rule, qty: Decimal, cost: Amount, acquired: Option<Date>| {
                if !emit || qty.is_zero() {
                    return;
                }
                let mut r = GainRow::new(&key, date, qty, rule)
                    .with_amounts(share(&d.proceeds, qty, d.qty), cost);
                r.acquired = acquired;
                if d.accounts.len() == 1 {
                    r.account = d.accounts.first().copied();
                }
                rows.push(r);
            };
            // 1. Same day.
            if let Some(a) = days.iter_mut().find(|a| a.date == date) {
                let m = left.min(a.remaining);
                if m > Decimal::ZERO {
                    a.remaining -= m;
                    left -= m;
                    push(Rule::SameDay, m, a.cost_of(m), Some(date));
                }
            }
            // 2. The following 30 days, earliest first.
            let until = Date(date.0 + Duration::days(30));
            for a in days.iter_mut().filter(|a| a.date > date && a.date <= until) {
                if left.is_zero() {
                    break;
                }
                let m = left.min(a.remaining);
                if m > Decimal::ZERO {
                    a.remaining -= m;
                    left -= m;
                    push(Rule::ThirtyDays, m, a.cost_of(m), Some(a.date));
                }
            }
            // 3. The pool.
            if left > Decimal::ZERO {
                let pool = pools.entry(key.clone()).or_default();
                let held = pool.qty.min(left);
                if held > Decimal::ZERO {
                    let cost = pool.take(held);
                    push(Rule::Pool, held, cost, None);
                }
                let over = left - held;
                if over > Decimal::ZERO {
                    push(Rule::Pool, over, Err(Missing::CostUnknown), None);
                }
            }
        }

        // What is left of the day's acquisitions joins the pool.
        let keys: BTreeSet<&String> = day.iter().map(|e| &e.key).collect();
        for key in keys {
            if let Some(a) = acq
                .get_mut(key)
                .and_then(|v| v.iter_mut().find(|a| a.date == date))
            {
                if a.remaining > Decimal::ZERO {
                    let cost = a.cost_of(a.remaining);
                    let qty = a.remaining;
                    a.remaining = Decimal::ZERO;
                    pools.entry(key.clone()).or_default().add(qty, &cost);
                }
            }
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replay::KeyMap;
    use crate::returns::PriceBook;
    use crate::tax::events::events;
    use crate::tax::testing::*;
    use mizan_import::Kind;
    use mizan_storage::StoredActivity;

    fn uk(year: &str, records: Vec<StoredActivity>) -> Vec<GainRow> {
        let ev = events(&[(1, records)], &KeyMap::new());
        let book = PriceBook::default();
        let rates = Rates::new(&book);
        match_disposals(
            &ev,
            &Treatments::default(),
            &rates,
            TaxYear::parse(Country::Uk, year).unwrap(),
        )
    }

    fn gbp(kind: Kind, date: &str, qty: &str, total: &str) -> StoredActivity {
        stored(1, deal(kind, date, "VOD", qty, total, "GBP"))
    }

    fn amounts(r: &GainRow) -> (Rule, Decimal, Decimal, Decimal, Decimal) {
        (
            r.rule,
            r.qty,
            r.proceeds.unwrap(),
            r.cost.unwrap(),
            r.gain.unwrap(),
        )
    }

    #[test]
    fn thirty_day_rule_then_pool() {
        let rows = uk(
            "2025-26",
            vec![
                gbp(Kind::Buy, "2018-05-01", "1000", "4000"),
                gbp(Kind::Buy, "2020-05-01", "500", "3000"),
                gbp(Kind::Sell, "2025-08-30", "700", "6300"),
                gbp(Kind::Buy, "2025-09-11", "400", "3000"),
                // Selling the rest later shows what the pool kept.
                gbp(Kind::Sell, "2026-01-15", "1200", "6000"),
            ],
        );
        assert_eq!(rows.len(), 3);
        assert_eq!(
            amounts(&rows[0]),
            (Rule::ThirtyDays, d("400"), d("3600"), d("3000"), d("600"))
        );
        assert_eq!(rows[0].acquired, Some(day("2025-09-11")));
        assert_eq!(
            amounts(&rows[1]),
            (Rule::Pool, d("300"), d("2700"), d("1400"), d("1300"))
        );
        // The pool kept 1,200 shares costing £5,600.
        assert_eq!(
            amounts(&rows[2]),
            (Rule::Pool, d("1200"), d("6000"), d("5600"), d("400"))
        );
    }

    #[test]
    fn same_day_rule_leaves_the_pool_unchanged() {
        let rows = uk(
            "2025-26",
            vec![
                gbp(Kind::Buy, "2019-05-01", "100", "500"),
                gbp(Kind::Buy, "2025-07-01", "100", "1000"),
                gbp(Kind::Sell, "2025-07-01", "100", "1200"),
                gbp(Kind::Sell, "2025-12-01", "100", "900"),
            ],
        );
        assert_eq!(
            amounts(&rows[0]),
            (Rule::SameDay, d("100"), d("1200"), d("1000"), d("200"))
        );
        assert_eq!(
            amounts(&rows[1]),
            (Rule::Pool, d("100"), d("900"), d("500"), d("400"))
        );
    }

    #[test]
    fn rows_outside_the_tax_year_are_not_emitted_but_still_match() {
        let rows = uk(
            "2024-25",
            vec![
                gbp(Kind::Buy, "2024-04-10", "10", "100"),
                gbp(Kind::Sell, "2025-04-05", "5", "80"),
                gbp(Kind::Sell, "2025-04-06", "5", "90"),
            ],
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].sold, day("2025-04-05"));
    }

    #[test]
    fn unknown_cost_and_missing_rates_are_reasons() {
        let mut inn = position(Kind::PositionIn, "2020-01-02", "VOD", "10");
        inn.cost_basis = None;
        let rows = uk(
            "2025-26",
            vec![
                stored(1, inn),
                gbp(Kind::Sell, "2025-06-01", "10", "100"),
                stored(1, deal(Kind::Buy, "2025-07-01", "AAPL", "1", "200", "USD")),
                stored(1, deal(Kind::Sell, "2025-09-01", "AAPL", "1", "250", "USD")),
            ],
        );
        assert_eq!(rows[0].missing, Some(Missing::CostUnknown));
        assert_eq!(rows[0].proceeds, Some(d("100")));
        assert_eq!(rows[1].missing, Some(Missing::NoRate));
    }

    #[test]
    fn isa_accounts_are_left_out_and_transfers_between_taxable_accounts_do_nothing() {
        let a = vec![
            gbp(Kind::Buy, "2020-01-02", "10", "100"),
            gbp(Kind::Sell, "2025-06-01", "5", "100"),
        ];
        let isa = vec![
            stored(2, deal(Kind::Buy, "2020-01-02", "VOD", "10", "100", "GBP")),
            stored(2, deal(Kind::Sell, "2025-06-01", "VOD", "10", "200", "GBP")),
        ];
        let ev = events(&[(1, a), (2, isa)], &KeyMap::new());
        let mut t = Treatments::default();
        t.0.insert(2, mizan_storage::TaxTreatment::Isa);
        let book = PriceBook::default();
        let rows = match_disposals(
            &ev,
            &t,
            &Rates::new(&book),
            TaxYear::parse(Country::Uk, "2025-26").unwrap(),
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].account, Some(1));
        assert_eq!(rows[0].gain, Some(d("50")));
    }

    /// HMRC HS284 (2025), Example 1: five purchases of Wilson and Strickland
    /// plc between 1979 and 2006 form one Section 104 holding of 12,000 shares.
    /// The helpsheet gives no costs; the costs here are illustrative.
    #[test]
    fn hs284_example_1_section_104_holding() {
        let rows = uk(
            "2025-26",
            vec![
                gbp(Kind::Buy, "1979-06-07", "2000", "1000"),
                gbp(Kind::Buy, "1982-11-04", "2500", "1500"),
                gbp(Kind::Buy, "1987-08-26", "2500", "2500"),
                gbp(Kind::Buy, "1998-07-07", "3000", "6000"),
                gbp(Kind::Buy, "2006-05-14", "2000", "5000"),
                gbp(Kind::Sell, "2025-06-02", "12000", "30000"),
            ],
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(
            amounts(&rows[0]),
            (Rule::Pool, d("12000"), d("30000"), d("16000"), d("14000"))
        );
    }

    /// HMRC HS284 (2025), Example 2: Mr Schneider sells 4,000 Mesopotamia plc
    /// shares for £6,000 on 30 August 2024 and buys 500 for £850 on
    /// 11 September 2024. The 500 are matched under the 30-day rule
    /// (proceeds £750, cost £850, loss £100) and 3,500 come from the holding.
    #[test]
    fn hs284_example_2_bed_and_breakfast() {
        let rows = uk(
            "2024-25",
            vec![
                // The helpsheet does not give the holding's cost.
                gbp(Kind::Buy, "2015-01-05", "10000", "12000"),
                gbp(Kind::Sell, "2024-08-30", "4000", "6000"),
                gbp(Kind::Buy, "2024-09-11", "500", "850"),
            ],
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(
            amounts(&rows[0]),
            (Rule::ThirtyDays, d("500"), d("750"), d("850"), d("-100"))
        );
        assert_eq!((rows[1].rule, rows[1].qty), (Rule::Pool, d("3500")));
        assert_eq!(rows[1].proceeds, Some(d("5250")));
    }
}
