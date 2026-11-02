//! Turkish realized gains on securities (GVK Mük. 80 and 81).
//!
//! Disposals are matched first in, first out across all taxable accounts. The
//! cost converts to TRY at the TCMB buying rate on the purchase date and the
//! proceeds at the rate on the sale date; rates printed on statements are not
//! used. The cost is raised by the Yİ-ÜFE change between the month before the
//! purchase and the month before the sale when that change is 10% or more.

use std::collections::BTreeMap;

use mizan_import::{Code, Date};
use mizan_storage::inflation::YearMonth;
use rust_decimal::Decimal;

use super::events::{Event, EventKind};
use super::fifo::FifoBook;
use super::rates::Rates;
use super::rows::{convert, GainRow, Missing, Rule, Treatments};
use super::{Country, TaxYear};

/// Yİ-ÜFE values by month.
pub type MonthlyIndex = BTreeMap<YearMonth, Decimal>;

/// The ratio at or above which cost is indexed.
pub fn threshold() -> Decimal {
    Decimal::new(110, 2)
}

pub const NOT_INDEXED: &str = "not indexed: Yİ-ÜFE not available";

/// The indexation ratio for a purchase and a sale, when both months are known.
pub fn ratio(index: &MonthlyIndex, acquired: Date, sold: Date) -> Option<Decimal> {
    let a = index.get(&YearMonth::of(acquired.0).shift(-1))?;
    let s = index.get(&YearMonth::of(sold.0).shift(-1))?;
    (!a.is_zero()).then(|| s / a)
}

/// Match every disposal under the Turkish rules and return the rows dated in `year`.
pub fn match_disposals(
    events: &[Event],
    t: &Treatments,
    rates: &Rates,
    index: &MonthlyIndex,
    year: TaxYear,
) -> Vec<GainRow> {
    let tl: Code = Country::Tr.currency();
    // All taxable accounts share one queue; others keep their own.
    let group = |account: i64| if t.taxable(account) { 0 } else { account };
    let mut book = FifoBook::default();
    let mut rows = Vec::new();
    for e in events {
        match &e.kind {
            EventKind::Dispose { qty, proceeds, .. } if t.taxable(e.account) => {
                let taken = book.apply(e, 0).unwrap_or_default();
                if !year.contains(e.date) {
                    continue;
                }
                let total = convert(rates, proceeds.as_ref(), &tl, e.date, None);
                let share = |q: Decimal| total.clone().map(|p| p * q / *qty);
                for lot in &taken.lots {
                    let cost = convert(rates, lot.cost.as_ref(), &tl, lot.cost_date, None);
                    let mut row = GainRow::new(&e.key, e.date, lot.qty, Rule::Fifo)
                        .with_amounts(share(lot.qty), cost);
                    row.account = Some(e.account);
                    row.acquired = Some(lot.acquired);
                    if lot.date_assumed {
                        row.notes.push("acquisition date assumed".into());
                    }
                    if let (Some(c), Some(p)) = (row.cost, row.proceeds) {
                        match ratio(index, lot.cost_date, e.date) {
                            Some(r) if r >= threshold() => {
                                let indexed = c * r;
                                row.indexed_cost = Some(indexed);
                                row.gain = Some(p - indexed);
                            }
                            Some(_) => {}
                            None => row.notes.push(NOT_INDEXED.into()),
                        }
                    }
                    rows.push(row);
                }
                if taken.shortfall > Decimal::ZERO {
                    let mut r = GainRow::new(&e.key, e.date, taken.shortfall, Rule::Fifo)
                        .with_amounts(share(taken.shortfall), Err(Missing::CostUnknown));
                    r.account = Some(e.account);
                    rows.push(r);
                }
            }
            EventKind::NotCovered {
                what,
                disposal: true,
            } if t.taxable(e.account) && year.contains(e.date) => {
                rows.push(GainRow::not_covered(&e.key, e.date, e.account, what));
            }
            _ => {
                book.apply(e, group(e.account));
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
    use mizan_import::{Dec, Fx, Kind};
    use mizan_storage::StoredActivity;
    use std::collections::HashMap;

    fn book() -> PriceBook {
        let mut rates = HashMap::new();
        rates.insert(
            code("USD"),
            vec![
                (day("2023-03-01"), d("18.90")),
                (day("2025-06-10"), d("39.00")),
            ],
        );
        PriceBook::from_parts(HashMap::new(), rates)
    }

    fn records() -> Vec<StoredActivity> {
        let mut sell = deal(Kind::Sell, "2025-06-10", "AAPL", "10", "2200", "USD");
        // A statement rate is ignored in Turkish reports.
        sell.fx = Some(Fx {
            rate: Dec(d("40")),
            from: code("USD"),
            to: code("TRY"),
        });
        vec![
            stored(
                1,
                deal(Kind::Buy, "2023-03-01", "AAPL", "10", "1500", "USD"),
            ),
            stored(1, sell),
        ]
    }

    fn run(index: &MonthlyIndex) -> Vec<GainRow> {
        let ev = events(&[(1, records())], &KeyMap::new());
        let b = book();
        match_disposals(
            &ev,
            &Treatments::default(),
            &Rates::new(&b),
            index,
            TaxYear::parse(Country::Tr, "2025").unwrap(),
        )
    }

    fn index(acq: &str, sale: &str) -> MonthlyIndex {
        [
            (YearMonth::parse("2023-02").unwrap(), d(acq)),
            (YearMonth::parse("2025-05").unwrap(), d(sale)),
        ]
        .into_iter()
        .collect()
    }

    #[test]
    fn foreign_share_sold_converts_at_tcmb_buying_rates() {
        let rows = run(&MonthlyIndex::new());
        assert_eq!(rows[0].cost, Some(d("28350.00")));
        assert_eq!(rows[0].proceeds, Some(d("85800.00")));
        assert_eq!(rows[0].acquired, Some(day("2023-03-01")));
    }

    #[test]
    fn index_rose_more_than_10_percent() {
        let rows = run(&index("1000.00", "1800.00"));
        assert_eq!(rows[0].indexed_cost, Some(d("51030.0000")));
        assert_eq!(rows[0].gain, Some(d("34770.0000")));
        assert!(rows[0].notes.is_empty());
    }

    #[test]
    fn index_rose_less_than_10_percent() {
        let rows = run(&index("1000.00", "1050.00"));
        assert_eq!(rows[0].indexed_cost, None);
        assert_eq!(rows[0].gain, Some(d("57450.00")));
    }

    #[test]
    fn exactly_10_percent_is_indexed() {
        let rows = run(&index("1000.00", "1100.00"));
        assert!(rows[0].indexed_cost.is_some());
    }

    #[test]
    fn no_index_values() {
        let rows = run(&MonthlyIndex::new());
        assert_eq!(rows[0].gain, Some(d("57450.00")));
        assert_eq!(rows[0].notes, [NOT_INDEXED]);
    }

    #[test]
    fn fifo_across_taxable_accounts() {
        let a = vec![stored(
            1,
            deal(Kind::Buy, "2023-01-02", "X", "10", "100", "TRY"),
        )];
        let b = vec![
            stored(2, deal(Kind::Buy, "2024-01-02", "X", "10", "300", "TRY")),
            stored(2, deal(Kind::Sell, "2025-03-01", "X", "10", "500", "TRY")),
        ];
        let ev = events(&[(1, a), (2, b)], &KeyMap::new());
        let b = PriceBook::default();
        let rows = match_disposals(
            &ev,
            &Treatments::default(),
            &Rates::new(&b),
            &MonthlyIndex::new(),
            TaxYear::parse(Country::Tr, "2025").unwrap(),
        );
        assert_eq!(
            rows[0].acquired,
            Some(day("2023-01-02")),
            "oldest lot of the person"
        );
        assert_eq!(rows[0].cost, Some(d("100")));
    }
}
