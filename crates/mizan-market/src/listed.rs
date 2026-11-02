//! Types shared by the listed stock and ETF providers (Yahoo Finance, Twelve Data).

use std::str::FromStr;
use std::time::Duration;

use mizan_import::{Code, Date};
use rust_decimal::Decimal;

/// One daily close, unadjusted (as traded that day), in the major currency unit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Close {
    pub date: Date,
    pub value: Decimal,
    pub currency: Code,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ListedError {
    #[error("{0}: symbol not found")]
    NotFound(String),
    #[error("request limit reached")]
    RateLimited,
    #[error("API key rejected")]
    KeyRejected,
    #[error("request failed: {0}")]
    Http(String),
    #[error("returned HTTP {0}")]
    Status(u16),
    #[error("response could not be read: {0}")]
    Parse(String),
}

impl ListedError {
    /// Errors that stop the provider for the rest of the run.
    pub fn stops_provider(&self) -> bool {
        matches!(self, ListedError::RateLimited | ListedError::KeyRejected)
    }
}

/// An API key that never shows in debug output.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(pub String);

impl std::fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApiKey(•••)")
    }
}

/// A listed provider and its request spacing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListedProvider {
    pub id: &'static str,
    pub name: &'static str,
    pub interval: Duration,
}

pub const YAHOO: ListedProvider = ListedProvider {
    id: crate::yahoo::SOURCE,
    name: "Yahoo Finance",
    interval: crate::yahoo::MIN_INTERVAL,
};

pub const TWELVEDATA: ListedProvider = ListedProvider {
    id: crate::twelvedata::SOURCE,
    name: "Twelve Data",
    interval: crate::twelvedata::MIN_INTERVAL,
};

pub const LISTED: [ListedProvider; 2] = [YAHOO, TWELVEDATA];

pub fn listed_provider(id: &str) -> Option<ListedProvider> {
    LISTED.iter().copied().find(|p| p.id == id)
}

/// Minor currency units and their major unit; values are divided by 100.
const MINOR_UNITS: [(&str, &str); 4] = [
    ("GBp", "GBP"),
    ("GBX", "GBP"),
    ("ILA", "ILS"),
    ("ZAc", "ZAR"),
];

/// Convert a value quoted in a minor unit (GBp, GBX, ILA, ZAc) to its major
/// unit. Other currencies are upper-cased and returned unchanged.
pub fn major_unit(currency: &str, value: Decimal) -> Option<(Code, Decimal)> {
    let c = currency.trim();
    if let Some((_, major)) = MINOR_UNITS.iter().find(|(m, _)| *m == c) {
        return Some((Code::parse(major).ok()?, value / Decimal::ONE_HUNDRED));
    }
    Some((Code::parse(&c.to_uppercase()).ok()?, value))
}

/// Parse a JSON number or string as a decimal without binary floating point.
pub(crate) fn decimal(raw: &str) -> Option<Decimal> {
    let t = raw.trim().trim_matches('"');
    Decimal::from_str(t)
        .ok()
        .or_else(|| Decimal::from_scientific(t).ok())
}

/// Percent-encode a provider symbol for a URL path or query.
pub(crate) fn encode(symbol: &str) -> String {
    symbol
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'-' | b'_' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minor_units() {
        let (c, v) = major_unit("GBp", Decimal::from(7542)).unwrap();
        assert_eq!((c.as_str(), v.to_string().as_str()), ("GBP", "75.42"));
        let (c, _) = major_unit("ZAc", Decimal::ONE).unwrap();
        assert_eq!(c.as_str(), "ZAR");
        let (c, v) = major_unit("usd", Decimal::from(5)).unwrap();
        assert_eq!((c.as_str(), v), ("USD", Decimal::from(5)));
    }

    #[test]
    fn encodes_symbols() {
        assert_eq!(encode("VWCE.DE"), "VWCE.DE");
        assert_eq!(encode("^GSPC"), "%5EGSPC");
        assert_eq!(encode("BRK B"), "BRK%20B");
    }
}
