//! Conversion into a report currency on a transaction's own date.

use chrono::Duration;
use mizan_import::{Code, Date, Fx};
use rust_decimal::Decimal;

use crate::returns::PriceBook;

/// A stored rate older than this many days is not used.
pub const MAX_RATE_AGE_DAYS: i64 = 7;

/// TCMB rates crossed through TRY.
pub struct Rates<'a> {
    pub book: &'a PriceBook,
}

impl<'a> Rates<'a> {
    pub fn new(book: &'a PriceBook) -> Self {
        Rates { book }
    }

    /// TRY per unit of `currency`, newest on or before `day` within 7 days.
    pub fn try_per_unit(&self, currency: &Code, day: Date) -> Option<Decimal> {
        let (d, r) = self.book.dated_rate(currency, day)?;
        (day.0 - d.0 <= Duration::days(MAX_RATE_AGE_DAYS)).then_some(r)
    }

    /// Convert `amount` from `from` into `to` on `day`. When `statement` is
    /// given and converts directly between the two currencies, its rate wins.
    pub fn convert(
        &self,
        amount: Decimal,
        from: &Code,
        to: &Code,
        day: Date,
        statement: Option<&Fx>,
    ) -> Option<Decimal> {
        if from == to {
            return Some(amount);
        }
        if let Some(fx) = statement {
            if fx.from == *from && fx.to == *to {
                return Some(amount * fx.rate.value());
            }
            if fx.from == *to && fx.to == *from && !fx.rate.value().is_zero() {
                return Some(amount / fx.rate.value());
            }
        }
        let f = self.try_per_unit(from, day)?;
        let t = self.try_per_unit(to, day)?;
        (!t.is_zero()).then(|| amount * f / t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tax::testing::*;
    use mizan_import::Dec;
    use std::collections::HashMap;

    fn book() -> PriceBook {
        let mut rates = HashMap::new();
        rates.insert(code("USD"), vec![(day("2025-06-10"), d("34.00"))]);
        rates.insert(code("GBP"), vec![(day("2025-06-10"), d("42.50"))]);
        PriceBook::from_parts(HashMap::new(), rates)
    }

    #[test]
    fn usd_sale_in_a_uk_report_crosses_through_try() {
        let b = book();
        let r = Rates::new(&b);
        let gbp = r.convert(
            d("1000.00"),
            &code("USD"),
            &code("GBP"),
            day("2025-06-10"),
            None,
        );
        assert_eq!(gbp, Some(d("800")));
        // A weekend two days later still uses Friday's rates.
        assert!(r
            .convert(d("1"), &code("USD"), &code("GBP"), day("2025-06-12"), None)
            .is_some());
    }

    #[test]
    fn statement_rate_used_when_it_converts_directly() {
        let b = book();
        let r = Rates::new(&b);
        let fx = Fx {
            rate: Dec(d("0.79")),
            from: code("USD"),
            to: code("GBP"),
        };
        let gbp = r.convert(
            d("1000.00"),
            &code("USD"),
            &code("GBP"),
            day("2025-06-10"),
            Some(&fx),
        );
        assert_eq!(gbp, Some(d("790.0000")));
        let usd = r.convert(
            d("790"),
            &code("GBP"),
            &code("USD"),
            day("2025-06-10"),
            Some(&fx),
        );
        assert_eq!(usd, Some(d("1000")));
        // A rate between other currencies is ignored.
        let eur = Fx {
            rate: Dec(d("0.9")),
            from: code("USD"),
            to: code("EUR"),
        };
        let gbp = r.convert(
            d("1000.00"),
            &code("USD"),
            &code("GBP"),
            day("2025-06-10"),
            Some(&eur),
        );
        assert_eq!(gbp, Some(d("800")));
    }

    #[test]
    fn no_rate_near_the_date() {
        let b = book();
        let r = Rates::new(&b);
        assert!(r
            .convert(d("1"), &code("USD"), &code("GBP"), day("2025-06-18"), None)
            .is_none());
        assert!(r
            .convert(d("1"), &code("USD"), &code("GBP"), day("2025-06-09"), None)
            .is_none());
        assert_eq!(
            r.convert(d("5"), &code("TRY"), &code("TRY"), day("2001-01-01"), None),
            Some(d("5"))
        );
    }
}
