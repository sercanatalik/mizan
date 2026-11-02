//! The inflation series Mizan knows, one per supported base currency, and the
//! month type their values are keyed by. Shared by the providers that fetch the
//! series and the returns engine that reads them from the history cache.

use std::fmt;

use chrono::{Datelike, NaiveDate};

/// A consumer price index Mizan can fetch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InflationSeries {
    /// Base currency the index deflates.
    pub currency: &'static str,
    /// Provider id (`eurostat`, `bls`, `ons`).
    pub source: &'static str,
    /// Series id at that provider. Includes the index base year where the
    /// provider offers several, so different base years never mix.
    pub id: &'static str,
    /// Display name, for example "HICP Türkiye".
    pub label: &'static str,
    /// Display name of the provider.
    pub provider: &'static str,
}

impl InflationSeries {
    /// For example "HICP Türkiye (Eurostat)".
    pub fn full_label(&self) -> String {
        format!("{} ({})", self.label, self.provider)
    }
}

/// Turkish domestic producer price index (Yİ-ÜFE, 2003=100), used to index
/// the cost of foreign securities in Turkish tax reports. Not a base-currency
/// deflator, so it is not in [`SERIES`].
pub const PPI_TR: InflationSeries = InflationSeries {
    currency: "TRY",
    source: "evds",
    id: "TP.TUFE1YI.T1",
    label: "Yİ-ÜFE",
    provider: "TCMB EVDS",
};

pub const SERIES: [InflationSeries; 4] = [
    InflationSeries {
        currency: "TRY",
        source: "eurostat",
        id: "TR:I15",
        label: "HICP Türkiye",
        provider: "Eurostat",
    },
    InflationSeries {
        currency: "EUR",
        source: "eurostat",
        id: "EA:I15",
        label: "Euro-area HICP",
        provider: "Eurostat",
    },
    InflationSeries {
        currency: "USD",
        source: "bls",
        id: "CUUR0000SA0",
        label: "US CPI-U",
        provider: "BLS",
    },
    InflationSeries {
        currency: "GBP",
        source: "ons",
        id: "D7BT",
        label: "UK CPI",
        provider: "ONS",
    },
];

/// The index used for a base currency, if Mizan has one.
pub fn series_for(base: &str) -> Option<&'static InflationSeries> {
    SERIES
        .iter()
        .find(|s| s.currency.eq_ignore_ascii_case(base))
}

/// A calendar month.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct YearMonth {
    pub year: i32,
    /// 1 to 12.
    pub month: u32,
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

impl YearMonth {
    pub fn new(year: i32, month: u32) -> Option<Self> {
        (1..=12)
            .contains(&month)
            .then_some(YearMonth { year, month })
    }

    pub fn of(day: NaiveDate) -> Self {
        YearMonth {
            year: day.year(),
            month: day.month(),
        }
    }

    /// Parse `YYYY-MM`.
    pub fn parse(s: &str) -> Option<Self> {
        let (y, m) = s.split_once('-')?;
        if y.len() != 4 || m.len() != 2 {
            return None;
        }
        YearMonth::new(y.parse().ok()?, m.parse().ok()?)
    }

    /// Parse a three-letter English month name, any case.
    pub fn month_from_name(name: &str) -> Option<u32> {
        MONTHS
            .iter()
            .position(|m| m.eq_ignore_ascii_case(name))
            .map(|i| i as u32 + 1)
    }

    /// This month moved by `n` months (negative for earlier).
    pub fn shift(self, n: i32) -> Self {
        let i = self.year * 12 + self.month as i32 - 1 + n;
        YearMonth {
            year: i.div_euclid(12),
            month: i.rem_euclid(12) as u32 + 1,
        }
    }

    /// The 15th, where a monthly index value applies.
    pub fn anchor(self) -> NaiveDate {
        NaiveDate::from_ymd_opt(self.year, self.month, 15).expect("valid month")
    }

    /// For example "Sep 2026".
    pub fn label(self) -> String {
        format!("{} {}", MONTHS[self.month as usize - 1], self.year)
    }
}

impl fmt::Display for YearMonth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}", self.year, self.month)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn months() {
        let m = YearMonth::parse("2026-01").unwrap();
        assert_eq!(m.shift(-1).to_string(), "2025-12");
        assert_eq!(m.shift(11).to_string(), "2026-12");
        assert_eq!(m.shift(12).to_string(), "2027-01");
        assert_eq!(m.label(), "Jan 2026");
        assert_eq!(YearMonth::parse("2026-13"), None);
        assert_eq!(YearMonth::parse("26-01"), None);
        assert_eq!(YearMonth::month_from_name("SEP"), Some(9));
    }

    #[test]
    fn series_per_currency() {
        assert_eq!(series_for("TRY").unwrap().id, "TR:I15");
        assert_eq!(series_for("eur").unwrap().id, "EA:I15");
        assert_eq!(series_for("USD").unwrap().source, "bls");
        assert_eq!(series_for("GBP").unwrap().source, "ons");
        assert_eq!(series_for("CHF"), None);
    }
}
