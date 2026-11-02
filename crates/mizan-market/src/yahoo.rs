//! Yahoo Finance daily closes for listed stocks and ETFs.
//!
//! Uses the unofficial chart endpoint, verified on 2026-11-02 (see
//! `docs/market-data.md`). No key is needed. Its `close` series is adjusted for
//! later splits; Mizan undoes that with the split events of the same response,
//! so stored closes are the prices traded on each day.

use std::collections::BTreeMap;
use std::time::Duration;

use chrono::{DateTime, Duration as Days, NaiveTime};
use mizan_import::Date;
use rust_decimal::{Decimal, RoundingStrategy};
use serde::Deserialize;
use serde_json::value::RawValue;

use crate::http::Http;
use crate::listed::{decimal, encode, major_unit, Close, ListedError};

pub const SOURCE: &str = "yahoo";
pub const BASE: &str = "https://query1.finance.yahoo.com/v8/finance/chart/";
/// At most one request every 2 seconds.
pub const MIN_INTERVAL: Duration = Duration::from_secs(2);
/// Decimal places when the response declares none.
const DEFAULT_PRICE_HINT: u32 = 4;

/// The last month of daily closes, with split events (for the latest price).
pub fn latest_url(symbol: &str) -> String {
    format!(
        "{BASE}{}?range=1mo&interval=1d&events=split",
        encode(symbol)
    )
}

fn unix(d: Date) -> i64 {
    d.0.and_time(NaiveTime::MIN).and_utc().timestamp()
}

/// Daily closes from `from` through `today`, with every split after them.
pub fn span_url(symbol: &str, from: Date, today: Date) -> String {
    format!(
        "{BASE}{}?period1={}&period2={}&interval=1d&events=split",
        encode(symbol),
        unix(from),
        unix(Date(today.0 + Days::days(1)))
    )
}

#[derive(Deserialize)]
struct Envelope<'a> {
    #[serde(borrow)]
    chart: Chart<'a>,
}

#[derive(Deserialize)]
struct Chart<'a> {
    #[serde(borrow, default)]
    result: Option<Vec<Series<'a>>>,
    #[serde(default)]
    error: Option<ApiError>,
}

#[derive(Deserialize)]
struct ApiError {
    #[serde(default)]
    code: String,
    #[serde(default)]
    description: String,
}

#[derive(Deserialize)]
struct Series<'a> {
    meta: Meta,
    #[serde(default)]
    timestamp: Vec<i64>,
    #[serde(borrow)]
    indicators: Indicators<'a>,
    #[serde(borrow, default)]
    events: Option<Events<'a>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Meta {
    currency: Option<String>,
    price_hint: Option<u32>,
    #[serde(default)]
    gmtoffset: i64,
}

#[derive(Deserialize)]
struct Indicators<'a> {
    #[serde(borrow)]
    quote: Vec<Quote<'a>>,
}

#[derive(Deserialize)]
struct Quote<'a> {
    /// Raw JSON text, so prices never pass through binary floating point.
    #[serde(borrow, default)]
    close: Vec<Option<&'a RawValue>>,
}

#[derive(Deserialize)]
struct Events<'a> {
    #[serde(borrow, default)]
    splits: BTreeMap<String, Split<'a>>,
}

#[derive(Deserialize)]
struct Split<'a> {
    date: i64,
    #[serde(borrow)]
    numerator: &'a RawValue,
    #[serde(borrow)]
    denominator: &'a RawValue,
}

/// Parse a chart response into unadjusted closes, oldest first.
pub fn parse(symbol: &str, status: u16, body: &str) -> Result<Vec<Close>, ListedError> {
    if status == 429 {
        return Err(ListedError::RateLimited);
    }
    let env: Envelope = match serde_json::from_str(body) {
        Ok(e) => e,
        Err(_) if status == 404 => return Err(ListedError::NotFound(symbol.into())),
        Err(_) if status != 200 => return Err(ListedError::Status(status)),
        Err(e) => return Err(ListedError::Parse(e.to_string())),
    };
    if let Some(err) = env.chart.error {
        let d = err.description.to_lowercase();
        if err.code == "Not Found" || d.contains("no data found") || d.contains("delisted") {
            return Err(ListedError::NotFound(symbol.into()));
        }
        return Err(ListedError::Parse(format!(
            "{}: {}",
            err.code, err.description
        )));
    }
    if status != 200 {
        return Err(ListedError::Status(status));
    }
    let Some(series) = env.chart.result.and_then(|r| r.into_iter().next()) else {
        return Err(ListedError::NotFound(symbol.into()));
    };
    let currency = series
        .meta
        .currency
        .ok_or_else(|| ListedError::Parse("no currency".into()))?;
    let hint = series.meta.price_hint.unwrap_or(DEFAULT_PRICE_HINT);
    let mut splits: Vec<(i64, Decimal)> = Vec::new();
    for s in series
        .events
        .map(|e| e.splits)
        .unwrap_or_default()
        .into_values()
    {
        let (Some(n), Some(d)) = (decimal(s.numerator.get()), decimal(s.denominator.get())) else {
            return Err(ListedError::Parse("unreadable split".into()));
        };
        if d.is_zero() || n.is_zero() {
            return Err(ListedError::Parse("unreadable split".into()));
        }
        splits.push((s.date, n / d));
    }
    let closes = series
        .indicators
        .quote
        .into_iter()
        .next()
        .map(|q| q.close)
        .unwrap_or_default();
    let mut out: BTreeMap<Date, Close> = BTreeMap::new();
    for (t, raw) in series.timestamp.iter().zip(closes) {
        let Some(raw) = raw else { continue };
        if raw.get() == "null" {
            continue;
        }
        let adjusted = decimal(raw.get())
            .ok_or_else(|| ListedError::Parse(format!("unreadable close {}", raw.get())))?;
        // Undo the adjustment for every split after this bar.
        let factor: Decimal = splits
            .iter()
            .filter(|(at, _)| at > t)
            .map(|(_, r)| *r)
            .product();
        let traded =
            (adjusted * factor).round_dp_with_strategy(hint, RoundingStrategy::MidpointNearestEven);
        let (code, value) = major_unit(&currency, traded)
            .ok_or_else(|| ListedError::Parse(format!("unknown currency {currency}")))?;
        let value = value.normalize();
        let date = DateTime::from_timestamp(t + series.meta.gmtoffset, 0)
            .ok_or_else(|| ListedError::Parse(format!("bad timestamp {t}")))?
            .date_naive();
        out.insert(
            Date(date),
            Close {
                date: Date(date),
                value,
                currency: code,
            },
        );
    }
    Ok(out.into_values().collect())
}

fn fetch(http: &dyn Http, symbol: &str, url: &str) -> Result<Vec<Close>, ListedError> {
    let r = http.get(url).map_err(|e| ListedError::Http(e.0))?;
    parse(symbol, r.status, &r.body)
}

/// The last month of closes (the newest is the latest price).
pub fn fetch_latest(http: &dyn Http, symbol: &str) -> Result<Vec<Close>, ListedError> {
    fetch(http, symbol, &latest_url(symbol))
}

/// Closes from `from` through `today`.
pub fn fetch_span(
    http: &dyn Http,
    symbol: &str,
    from: Date,
    today: Date,
) -> Result<Vec<Close>, ListedError> {
    fetch(http, symbol, &span_url(symbol, from, today))
}
