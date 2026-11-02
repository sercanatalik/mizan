//! US realized gains: first in, first out per account, short- and long-term
//! holding periods, and wash sales (IRS Publication 550).
//!
//! Each lot converts to USD at the date its cost was paid and each sale at its
//! own date. A loss is disallowed, in proportion to quantity, when the same
//! instrument is acquired in any account within 30 days before or after the
//! sale and those shares are still held after it. In a taxable account the
//! disallowed loss is added to the replacement shares' cost and the sold
//! shares' holding period carries over; in an ISA or pension account it is
//! simply lost.

use std::collections::{HashMap, HashSet};

use chrono::{Duration, Months};
use mizan_import::{Code, Date};
use rust_decimal::Decimal;

use super::events::{Event, EventKind};
use super::fifo::{FifoBook, Lot};
use super::rates::Rates;
use super::rows::{convert, GainRow, Missing, Rule, Term, Treatments};
use super::{Country, TaxYear};

/// Long-term when the sale is more than one year after the acquisition.
pub fn term(acquired: Date, sold: Date) -> Term {
    // Held more than a year: sold after the anniversary of the acquisition.
    // A 29 February acquisition has its anniversary on 28 February.
    let anniversary = acquired
        .0
        .checked_add_months(Months::new(12))
        .unwrap_or(acquired.0);
    if sold.0 > anniversary {
        Term::Long
    } else {
        Term::Short
    }
}

/// A disallowed loss waiting for its replacement acquisition to be processed.
#[derive(Clone, Debug)]
struct Pending {
    qty: Decimal,
    adjust: Decimal,
    /// Days the sold shares were held, carried over to the replacement.
    held: Duration,
}

/// Add a wash-sale adjustment to `qty` units of the lot created by `origin`.
fn apply_wash(book: &mut FifoBook, origin: usize, p: &Pending) {
    if let Some((group, key, mut lot)) = book.split_origin(origin, p.qty) {
        lot.adjust += p.adjust;
        lot.acquired = Date(lot.acquired.0 - p.held);
        book.insert(group, &key, lot);
    }
}

/// Match every disposal under the US rules and return the rows dated in `year`.
pub fn match_disposals(
    events: &[Event],
    t: &Treatments,
    rates: &Rates,
    year: TaxYear,
) -> Vec<GainRow> {
    let usd: Code = Country::Us.currency();
    let mut book = FifoBook::default();
    // Acquisitions per instrument, for replacement-share search.
    let mut acquisitions: HashMap<&str, Vec<&Event>> = HashMap::new();
    let mut capacity: HashMap<usize, Decimal> = HashMap::new();
    for e in events {
        if let EventKind::Acquire { qty, .. } = &e.kind {
            acquisitions.entry(e.key.as_str()).or_default().push(e);
            capacity.insert(e.seq, *qty);
        }
    }
    let mut pending: HashMap<usize, Vec<Pending>> = HashMap::new();
    let mut rows = Vec::new();

    for e in events {
        let group = e.account;
        match &e.kind {
            EventKind::Acquire { .. } => {
                book.apply(e, group);
                for p in pending.remove(&e.seq).unwrap_or_default() {
                    apply_wash(&mut book, e.seq, &p);
                }
            }
            EventKind::Dispose { qty, proceeds, fx } if t.taxable(e.account) => {
                let taken = book.apply(e, group).unwrap_or_default();
                let total = convert(rates, proceeds.as_ref(), &usd, e.date, fx.as_ref());
                let share = |q: Decimal| total.clone().map(|p| p * q / *qty);
                let sold_origins: HashSet<usize> = taken.lots.iter().map(|l| l.origin).collect();
                for lot in &taken.lots {
                    let mut row = lot_row(e, lot, share(lot.qty), rates, &usd);
                    if let Some(gain) = row.gain.filter(|g| *g < Decimal::ZERO) {
                        let disallowed = wash(
                            e,
                            lot,
                            -gain,
                            &acquisitions,
                            &sold_origins,
                            &mut capacity,
                            &mut book,
                            &mut pending,
                            t,
                        );
                        if disallowed > Decimal::ZERO {
                            row.adjustment = Some(disallowed);
                            row.code = Some("W");
                            row.gain = Some(gain + disallowed);
                        }
                    }
                    if year.contains(e.date) {
                        rows.push(row);
                    }
                }
                if taken.shortfall > Decimal::ZERO && year.contains(e.date) {
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
                book.apply(e, group);
            }
        }
    }
    rows
}

fn lot_row(
    e: &Event,
    lot: &Lot,
    proceeds: Result<Decimal, Missing>,
    rates: &Rates,
    usd: &Code,
) -> GainRow {
    let cost = convert(
        rates,
        lot.cost.as_ref(),
        usd,
        lot.cost_date,
        lot.fx.as_ref(),
    )
    .map(|c| c + lot.adjust);
    let mut row = GainRow::new(&e.key, e.date, lot.qty, Rule::Fifo).with_amounts(proceeds, cost);
    row.account = Some(e.account);
    row.acquired = Some(lot.acquired);
    row.term = Some(term(lot.acquired, e.date));
    if lot.date_assumed {
        row.notes.push("acquisition date assumed".into());
    }
    row
}

/// Disallow the part of `loss` covered by replacement shares; returns the
/// disallowed amount.
#[allow(clippy::too_many_arguments)]
fn wash(
    sale: &Event,
    lot: &Lot,
    loss: Decimal,
    acquisitions: &HashMap<&str, Vec<&Event>>,
    sold: &HashSet<usize>,
    capacity: &mut HashMap<usize, Decimal>,
    book: &mut FifoBook,
    pending: &mut HashMap<usize, Vec<Pending>>,
    t: &Treatments,
) -> Decimal {
    let from = Date(sale.date.0 - Duration::days(30));
    let until = Date(sale.date.0 + Duration::days(30));
    let held = sale.date.0 - lot.acquired.0;
    let mut left = lot.qty;
    let mut disallowed = Decimal::ZERO;
    for a in acquisitions.get(sale.key.as_str()).into_iter().flatten() {
        if left.is_zero() {
            break;
        }
        if a.date < from || a.date > until || sold.contains(&a.seq) || a.seq == sale.seq {
            continue;
        }
        let cap = capacity.get(&a.seq).copied().unwrap_or_default();
        // Shares bought before the sale must still be held after it.
        let available = if a.seq < sale.seq {
            cap.min(book.held_from(a.seq))
        } else {
            cap
        };
        let used = available.min(left);
        if used <= Decimal::ZERO {
            continue;
        }
        let part = loss * used / lot.qty;
        disallowed += part;
        left -= used;
        capacity.insert(a.seq, cap - used);
        if t.taxable(a.account) {
            let p = Pending {
                qty: used,
                adjust: part,
                held,
            };
            if a.seq < sale.seq {
                apply_wash(book, a.seq, &p);
            } else {
                pending.entry(a.seq).or_default().push(p);
            }
        }
    }
    disallowed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replay::KeyMap;
    use crate::returns::PriceBook;
    use crate::tax::events::events;
    use crate::tax::testing::*;
    use mizan_import::Kind;
    use mizan_storage::{StoredActivity, TaxTreatment};

    fn usd(account: i64, kind: Kind, date: &str, qty: &str, total: &str) -> StoredActivity {
        stored(account, deal(kind, date, "XYZ", qty, total, "USD"))
    }

    fn run(year: &str, accounts: Vec<(i64, Vec<StoredActivity>)>, t: &Treatments) -> Vec<GainRow> {
        let ev = events(&accounts, &KeyMap::new());
        let book = PriceBook::default();
        match_disposals(
            &ev,
            t,
            &Rates::new(&book),
            TaxYear::parse(Country::Us, year).unwrap(),
        )
    }

    fn one(year: &str, records: Vec<StoredActivity>) -> Vec<GainRow> {
        run(year, vec![(1, records)], &Treatments::default())
    }

    #[test]
    fn sale_spanning_two_lots() {
        let rows = one(
            "2025",
            vec![
                usd(1, Kind::Buy, "2023-01-10", "10", "1000"),
                usd(1, Kind::Buy, "2024-06-01", "10", "1500"),
                usd(1, Kind::Sell, "2025-02-01", "15", "3000"),
            ],
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].term, Some(Term::Long));
        assert_eq!(
            (rows[0].qty, rows[0].proceeds, rows[0].cost),
            (d("10"), Some(d("2000")), Some(d("1000")))
        );
        assert_eq!(rows[1].term, Some(Term::Short));
        assert_eq!(
            (rows[1].qty, rows[1].proceeds, rows[1].cost),
            (d("5"), Some(d("1000")), Some(d("750")))
        );
    }

    #[test]
    fn one_year_boundary() {
        assert_eq!(term(day("2024-03-15"), day("2025-03-15")), Term::Short);
        assert_eq!(term(day("2024-03-15"), day("2025-03-16")), Term::Long);
        // Bought on 29 February: the anniversary is 28 February.
        assert_eq!(term(day("2024-02-29"), day("2025-02-28")), Term::Short);
        assert_eq!(term(day("2024-02-29"), day("2025-03-01")), Term::Long);
    }

    #[test]
    fn full_wash_sale() {
        let rows = one(
            "2025",
            vec![
                usd(1, Kind::Buy, "2025-01-02", "10", "1000"),
                usd(1, Kind::Sell, "2025-06-02", "10", "800"),
                usd(1, Kind::Buy, "2025-06-20", "10", "850"),
                // Selling the replacement shows its adjusted cost and date.
                usd(1, Kind::Sell, "2025-12-01", "10", "1100"),
            ],
        );
        let w = &rows[0];
        assert_eq!(w.code, Some("W"));
        assert_eq!(w.adjustment, Some(d("200")));
        assert_eq!(w.gain, Some(d("0")));
        assert_eq!(
            w.cost.unwrap() - w.proceeds.unwrap(),
            d("200"),
            "the loss itself"
        );
        let r = &rows[1];
        assert_eq!(r.cost, Some(d("1050")));
        assert_eq!(r.acquired, Some(day("2025-01-20")));
        assert_eq!(r.gain, Some(d("50")));
    }

    #[test]
    fn partial_replacement() {
        let rows = one(
            "2025",
            vec![
                usd(1, Kind::Buy, "2025-01-02", "10", "1000"),
                usd(1, Kind::Sell, "2025-06-02", "10", "800"),
                usd(1, Kind::Buy, "2025-06-10", "4", "300"),
            ],
        );
        assert_eq!(rows[0].adjustment, Some(d("80")));
        assert_eq!(rows[0].gain, Some(d("-120")));
    }

    #[test]
    fn replacement_bought_before_the_sale_counts_if_still_held() {
        let rows = one(
            "2025",
            vec![
                usd(1, Kind::Buy, "2025-01-02", "10", "1000"),
                usd(1, Kind::Buy, "2025-05-20", "10", "800"),
                // FIFO sells the January lot at a loss; the May lot is the replacement.
                usd(1, Kind::Sell, "2025-06-02", "10", "800"),
            ],
        );
        assert_eq!(rows[0].adjustment, Some(d("200")));
    }

    #[test]
    fn purchase_more_than_30_days_away_is_not_a_wash() {
        let rows = one(
            "2025",
            vec![
                usd(1, Kind::Buy, "2025-01-02", "10", "1000"),
                usd(1, Kind::Sell, "2025-06-02", "10", "800"),
                usd(1, Kind::Buy, "2025-07-03", "10", "850"),
            ],
        );
        assert_eq!(rows[0].adjustment, None);
        assert_eq!(rows[0].gain, Some(d("-200")));
    }

    #[test]
    fn replacement_in_a_pension_account() {
        let mut t = Treatments::default();
        t.0.insert(2, TaxTreatment::Pension);
        let rows = run(
            "2025",
            vec![
                (
                    1,
                    vec![
                        usd(1, Kind::Buy, "2025-01-02", "10", "1000"),
                        usd(1, Kind::Sell, "2025-06-02", "10", "800"),
                    ],
                ),
                (
                    2,
                    vec![
                        usd(2, Kind::Buy, "2025-06-20", "10", "850"),
                        usd(2, Kind::Sell, "2025-12-01", "10", "1100"),
                    ],
                ),
            ],
            &t,
        );
        assert_eq!(rows.len(), 1, "pension sales are not reported");
        assert_eq!(
            (rows[0].code, rows[0].adjustment),
            (Some("W"), Some(d("200")))
        );
    }

    /// IRS Publication 550, "Wash Sales", Example 1: you buy 100 shares
    /// of X stock for $1,000, sell them for $750 and within 30 days buy 100
    /// shares of the same stock for $800. The $250 loss is disallowed and the
    /// basis of the new shares is $1,050 ($800 + $250).
    #[test]
    fn publication_550_example_1() {
        let rows = one(
            "2024",
            vec![
                usd(1, Kind::Buy, "2024-01-10", "100", "1000"),
                usd(1, Kind::Sell, "2024-03-01", "100", "750"),
                usd(1, Kind::Buy, "2024-03-15", "100", "800"),
                usd(1, Kind::Sell, "2024-09-03", "100", "1050"),
            ],
        );
        assert_eq!(
            (rows[0].adjustment, rows[0].gain),
            (Some(d("250")), Some(d("0")))
        );
        assert_eq!(rows[1].cost, Some(d("1050")));
        assert_eq!(rows[1].gain, Some(d("0")));
    }

    /// IRS Publication 550, "Wash Sales", "More or less stock bought than
    /// sold", Example 1: 100 M shares bought 2024-09-20 for $5,000; 50 bought
    /// 2024-12-13 for $2,750; 25 bought 2024-12-20 for $1,125; the September
    /// shares sold 2025-01-03 for $4,000. $750 of the $1,000 loss is
    /// disallowed and $250 is deductible. The 50 shares' basis becomes $3,250
    /// and the 25 shares' basis $1,375.
    #[test]
    fn publication_550_more_or_less_stock_example_1() {
        let rows = one(
            "2025",
            vec![
                usd(1, Kind::Buy, "2024-09-20", "100", "5000"),
                usd(1, Kind::Buy, "2024-12-13", "50", "2750"),
                usd(1, Kind::Buy, "2024-12-20", "25", "1125"),
                usd(1, Kind::Sell, "2025-01-03", "100", "4000"),
                // Selling the replacement shares shows their new bases.
                usd(1, Kind::Sell, "2025-06-02", "50", "3250"),
                usd(1, Kind::Sell, "2025-06-03", "25", "1375"),
            ],
        );
        assert_eq!(
            (rows[0].adjustment, rows[0].gain),
            (Some(d("750")), Some(d("-250")))
        );
        assert_eq!(rows[1].cost, Some(d("3250")));
        assert_eq!(rows[2].cost, Some(d("1375")));
    }

    #[test]
    fn selling_more_than_held_is_cost_unknown() {
        let rows = one(
            "2025",
            vec![
                usd(1, Kind::Buy, "2025-01-02", "5", "500"),
                usd(1, Kind::Sell, "2025-02-02", "8", "800"),
            ],
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].missing, Some(Missing::CostUnknown));
        assert_eq!(rows[1].proceeds, Some(d("300")));
    }
}
