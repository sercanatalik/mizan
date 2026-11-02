//! Consumer price indices for real returns: which series a base currency uses,
//! when to ask for it, and fetching it from Eurostat, BLS or ONS.
//!
//! Only the base currency's series is ever requested. A full series is one
//! request (BLS: one per 10 years), so this runs inside the refresh rather than
//! the background backfill.

use chrono::{DateTime, Duration as Span, Utc};
use mizan_import::Date;
pub use mizan_storage::inflation::{series_for, InflationSeries, YearMonth, SERIES};
use rust_decimal::Decimal;

use crate::http::Http;
use crate::{bls, eurostat, ons};

/// Provider ids that supply inflation indices.
pub const PROVIDERS: [&str; 3] = [eurostat::SOURCE, bls::SOURCE, ons::SOURCE];
/// A series is asked for at most once in this window.
pub const RECHECK_AFTER: Span = Span::hours(24);
/// Cached months re-read on each fetch, to pick up revisions.
pub const REVISION_MONTHS: i32 = 3;

/// One published monthly value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexValue {
    pub month: YearMonth,
    pub value: Decimal,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum InflationError {
    #[error("{0}")]
    Http(String),
    #[error("HTTP {0}")]
    Status(u16),
    #[error("unexpected response: {0}")]
    Parse(String),
    #[error("{0}")]
    Api(String),
}

/// What a refresh should do for a series.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan {
    /// No request, with the reason.
    Skip(String),
    /// Request months from `from` through the current month.
    Fetch { from: YearMonth },
}

/// Decide whether to request a series. `cached` holds the cached months, oldest
/// first; `last_check` is the time of the last request for it, successful or not.
pub fn plan(
    cached: &[YearMonth],
    last_check: Option<DateTime<Utc>>,
    first_activity: Option<Date>,
    today: Date,
    now: DateTime<Utc>,
) -> Plan {
    let Some(first) = first_activity else {
        return Plan::Skip("no activities yet".into());
    };
    let needed_from = YearMonth::of(first.0).shift(-1);
    let complete_through = YearMonth::of(today.0).shift(-1);
    if let (Some(&oldest), Some(&newest)) = (cached.first(), cached.last()) {
        if newest >= complete_through && oldest <= needed_from {
            return Plan::Skip(format!("up to date through {}", newest.label()));
        }
    }
    if last_check.is_some_and(|t| now - t < RECHECK_AFTER) {
        return Plan::Skip("checked less than 24 hours ago".into());
    }
    let from = match (cached.first(), cached.last()) {
        (Some(&oldest), Some(&newest)) if oldest <= needed_from => {
            newest.shift(1 - REVISION_MONTHS)
        }
        _ => needed_from,
    };
    Plan::Fetch { from }
}

/// Fetch a series from `from` through the month of `today`, oldest first.
pub fn fetch(
    series: &InflationSeries,
    http: &dyn Http,
    from: YearMonth,
    today: Date,
) -> Result<Vec<IndexValue>, InflationError> {
    let to = YearMonth::of(today.0);
    let mut values = match series.source {
        eurostat::SOURCE => eurostat::fetch(http, series.id, from)?,
        bls::SOURCE => bls::fetch(http, series.id, from, to)?,
        ons::SOURCE => ons::fetch(http, series.id)?,
        other => return Err(InflationError::Api(format!("unknown provider {other}"))),
    };
    values.retain(|v| v.month >= from && v.month <= to);
    values.sort_by_key(|v| v.month);
    values.dedup_by_key(|v| v.month);
    Ok(values)
}

/// Parse a published index value; it must be a positive decimal.
pub(crate) fn index_value(text: &str) -> Result<Decimal, InflationError> {
    let v: Decimal = text
        .trim()
        .trim_matches('"')
        .parse()
        .map_err(|_| InflationError::Parse(format!("not a number: {text}")))?;
    if v <= Decimal::ZERO {
        return Err(InflationError::Parse(format!(
            "not a positive index: {text}"
        )));
    }
    Ok(v.normalize())
}

pub(crate) fn ok_body(r: crate::Response) -> Result<String, InflationError> {
    if r.status != 200 {
        return Err(InflationError::Status(r.status));
    }
    Ok(r.body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn ym(s: &str) -> YearMonth {
        YearMonth::parse(s).unwrap()
    }
    fn day(s: &str) -> Date {
        Date(NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap())
    }
    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }
    fn months(from: &str, to: &str) -> Vec<YearMonth> {
        let mut out = vec![];
        let mut m = ym(from);
        while m <= ym(to) {
            out.push(m);
            m = m.shift(1);
        }
        out
    }
    const NOW: &str = "2026-11-02T10:00:00Z";

    #[test]
    fn nothing_without_activities() {
        assert!(matches!(
            plan(&[], None, None, day("2026-11-02"), at(NOW)),
            Plan::Skip(_)
        ));
    }

    #[test]
    fn first_fetch_starts_the_month_before_the_first_activity() {
        assert_eq!(
            plan(
                &[],
                None,
                Some(day("2021-06-14")),
                day("2026-11-02"),
                at(NOW)
            ),
            Plan::Fetch {
                from: ym("2021-05")
            }
        );
    }

    #[test]
    fn complete_through_previous_month_is_skipped() {
        let cached = months("2021-05", "2026-10");
        assert_eq!(
            plan(
                &cached,
                None,
                Some(day("2021-06-14")),
                day("2026-11-02"),
                at(NOW)
            ),
            Plan::Skip("up to date through Oct 2026".into())
        );
    }

    #[test]
    fn recent_check_is_skipped_then_asked_again_after_a_day() {
        let cached = months("2021-05", "2026-09");
        let first = Some(day("2021-06-14"));
        assert!(matches!(
            plan(
                &cached,
                Some(at("2026-11-02T07:00:00Z")),
                first,
                day("2026-11-02"),
                at(NOW)
            ),
            Plan::Skip(_)
        ));
        assert_eq!(
            plan(
                &cached,
                Some(at("2026-11-01T09:00:00Z")),
                first,
                day("2026-11-02"),
                at(NOW)
            ),
            Plan::Fetch {
                from: ym("2026-07")
            }
        );
    }

    #[test]
    fn older_gap_is_filled_from_the_needed_month() {
        let cached = months("2024-01", "2026-09");
        assert_eq!(
            plan(
                &cached,
                None,
                Some(day("2021-06-14")),
                day("2026-11-02"),
                at(NOW)
            ),
            Plan::Fetch {
                from: ym("2021-05")
            }
        );
    }

    #[test]
    fn index_values_are_positive_decimals() {
        assert_eq!(index_value("134.310").unwrap().to_string(), "134.31");
        assert_eq!(index_value("\"144.1\"").unwrap().to_string(), "144.1");
        assert!(index_value("0").is_err());
        assert!(index_value("n/a").is_err());
    }
}
