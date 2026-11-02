//! Tax lots and working-paper tax reports for the UK, the US and Turkey.
//!
//! Lots are built once from stored activities in their original currency
//! ([`events`]); each country's rules match disposals and convert amounts
//! into the report currency ([`uk`], [`us`], [`tr`]). Nothing here computes
//! tax owed, allowances or filing thresholds.

use chrono::{Datelike, NaiveDate};
use mizan_import::{Code, Date};
use serde::Serialize;

pub mod csv;
pub mod events;
pub mod fifo;
pub mod income;
pub mod rates;
pub mod report;
pub mod rows;
#[cfg(test)]
pub(crate) mod testing;
pub mod tr;
pub mod uk;
pub mod us;

pub use mizan_storage::TaxTreatment;

/// A country whose tax rules Mizan can report on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Country {
    Uk,
    Us,
    Tr,
}

impl Country {
    pub const ALL: [Country; 3] = [Country::Uk, Country::Us, Country::Tr];

    pub fn as_str(&self) -> &'static str {
        match self {
            Country::Uk => "uk",
            Country::Us => "us",
            Country::Tr => "tr",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Country::ALL
            .into_iter()
            .find(|c| c.as_str() == s.to_ascii_lowercase())
    }

    /// The currency a report for this country is shown in.
    pub fn currency(&self) -> Code {
        let c = match self {
            Country::Uk => "GBP",
            Country::Us => "USD",
            Country::Tr => "TRY",
        };
        Code::parse(c).expect("valid currency code")
    }

    /// The tax year containing `day`.
    pub fn year_of(&self, day: Date) -> TaxYear {
        let d = day.0;
        let start = match self {
            Country::Uk if (d.month(), d.day()) < (4, 6) => d.year() - 1,
            _ => d.year(),
        };
        TaxYear {
            country: *self,
            start_year: start,
        }
    }
}

/// One tax year of one country. UK years run 6 April to 5 April; US and
/// Turkish years are calendar years.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TaxYear {
    pub country: Country,
    /// The calendar year the tax year starts in.
    pub start_year: i32,
}

impl TaxYear {
    pub fn new(country: Country, start_year: i32) -> Self {
        TaxYear {
            country,
            start_year,
        }
    }

    /// Parse a label such as `2025-26` (UK) or `2025` (US, Turkey).
    pub fn parse(country: Country, s: &str) -> Option<Self> {
        let s = s.trim();
        match country {
            Country::Uk => {
                let (a, b) = s.split_once('-')?;
                let start: i32 = a.parse().ok()?;
                let end: i32 = b.parse().ok()?;
                (b.len() == 2 && end == (start + 1) % 100).then_some(TaxYear::new(country, start))
            }
            _ => (s.len() == 4)
                .then(|| s.parse().ok())
                .flatten()
                .map(|y| TaxYear::new(country, y)),
        }
    }

    pub fn label(&self) -> String {
        match self.country {
            Country::Uk => format!("{}-{:02}", self.start_year, (self.start_year + 1) % 100),
            _ => self.start_year.to_string(),
        }
    }

    pub fn first_day(&self) -> Date {
        let (m, d) = match self.country {
            Country::Uk => (4, 6),
            _ => (1, 1),
        };
        Date(NaiveDate::from_ymd_opt(self.start_year, m, d).expect("valid date"))
    }

    pub fn last_day(&self) -> Date {
        let d = match self.country {
            Country::Uk => NaiveDate::from_ymd_opt(self.start_year + 1, 4, 5),
            _ => NaiveDate::from_ymd_opt(self.start_year, 12, 31),
        };
        Date(d.expect("valid date"))
    }

    pub fn contains(&self, day: Date) -> bool {
        self.first_day() <= day && day <= self.last_day()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(s: &str) -> Date {
        Date::parse(s).unwrap()
    }

    #[test]
    fn uk_year_runs_6_april_to_5_april() {
        let y = TaxYear::parse(Country::Uk, "2025-26").unwrap();
        assert_eq!(y.first_day(), day("2025-04-06"));
        assert_eq!(y.last_day(), day("2026-04-05"));
        assert_eq!(y.label(), "2025-26");
        assert!(!y.contains(day("2025-04-05")));
        assert!(y.contains(day("2025-04-06")));
        assert!(y.contains(day("2026-04-05")));
        assert!(!y.contains(day("2026-04-06")));
        assert_eq!(Country::Uk.year_of(day("2025-04-05")).label(), "2024-25");
        assert_eq!(Country::Uk.year_of(day("2025-04-06")).label(), "2025-26");
        assert_eq!(
            TaxYear::parse(Country::Uk, "1999-00").unwrap().start_year,
            1999
        );
        assert!(TaxYear::parse(Country::Uk, "2025-27").is_none());
        assert!(TaxYear::parse(Country::Uk, "2025").is_none());
    }

    #[test]
    fn us_and_turkish_years_are_calendar_years() {
        for c in [Country::Us, Country::Tr] {
            let y = TaxYear::parse(c, "2025").unwrap();
            assert_eq!(y.first_day(), day("2025-01-01"));
            assert_eq!(y.last_day(), day("2025-12-31"));
            assert_eq!(y.label(), "2025");
            assert_eq!(c.year_of(day("2025-04-05")), y);
            assert!(TaxYear::parse(c, "2025-26").is_none());
        }
    }

    mod not_computed {
        use crate::replay::KeyMap;
        use crate::returns::PriceBook;
        use crate::tax::events::events;
        use crate::tax::rates::Rates;
        use crate::tax::rows::{Missing, Treatments};
        use crate::tax::testing::*;
        use crate::tax::{tr, uk, us, Country, TaxYear};
        use mizan_import::Kind;

        fn all(records: Vec<mizan_storage::StoredActivity>) -> Vec<Vec<crate::tax::rows::GainRow>> {
            let ev = events(&[(1, records)], &KeyMap::new());
            let b = PriceBook::default();
            let r = Rates::new(&b);
            let t = Treatments::default();
            vec![
                uk::match_disposals(&ev, &t, &r, TaxYear::parse(Country::Uk, "2025-26").unwrap()),
                us::match_disposals(&ev, &t, &r, TaxYear::parse(Country::Us, "2025").unwrap()),
                tr::match_disposals(
                    &ev,
                    &t,
                    &r,
                    &Default::default(),
                    TaxYear::parse(Country::Tr, "2025").unwrap(),
                ),
            ]
        }

        #[test]
        fn sale_of_shares_with_unknown_cost() {
            let inn = position(Kind::PositionIn, "2024-01-02", "X", "30");
            for rows in all(vec![
                stored(1, inn),
                stored(1, deal(Kind::Sell, "2025-06-02", "X", "30", "900", "TRY")),
            ]) {
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].missing, Some(Missing::CostUnknown));
                assert_eq!(rows[0].gain, None);
            }
        }

        #[test]
        fn option_expiry_is_not_covered() {
            for rows in all(vec![stored(
                1,
                position(Kind::OptionExpiry, "2025-06-20", "AAPL 250620C", "1"),
            )]) {
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].missing, Some(Missing::NotCovered("option")));
                assert_eq!(
                    rows[0].missing.as_ref().unwrap().reason(),
                    "not covered: option"
                );
            }
        }

        #[test]
        fn no_exchange_rate() {
            for (i, rows) in all(vec![
                stored(1, deal(Kind::Buy, "2025-05-02", "X", "1", "100", "EUR")),
                stored(1, deal(Kind::Sell, "2025-06-02", "X", "1", "120", "EUR")),
            ])
            .into_iter()
            .enumerate()
            {
                assert_eq!(rows[0].missing, Some(Missing::NoRate), "country {i}");
            }
        }
    }

    #[test]
    fn countries() {
        assert_eq!(Country::parse("UK"), Some(Country::Uk));
        assert_eq!(Country::Tr.currency().as_str(), "TRY");
        assert_eq!(Country::Us.currency().as_str(), "USD");
        assert_eq!(Country::Uk.currency().as_str(), "GBP");
    }
}
