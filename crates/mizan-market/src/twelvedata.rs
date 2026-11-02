//! Twelve Data daily closes for listed stocks and ETFs.
//!
//! Needs a free API key, sent in the `Authorization: apikey <key>` header so it
//! never appears in a URL. Prices are requested unadjusted (`adjust=none`).
//! The free plan covers US listings and allows 8 requests a minute.

use std::time::Duration;

use chrono::{Datelike, Duration as Days, NaiveDate};
use mizan_import::Date;
use rust_decimal::RoundingStrategy;
use serde::Deserialize;

use crate::http::Http;
use crate::listed::{decimal, encode, major_unit, Close, ListedError};

pub const SOURCE: &str = "twelvedata";
pub const BASE: &str = "https://api.twelvedata.com/time_series";
/// The free plan allows 8 requests a minute.
pub const MIN_INTERVAL: Duration = Duration::from_secs(8);
/// At most 5,000 daily points per request: stay under it with 19-year spans.
pub const MAX_SPAN_YEARS: i32 = 19;
/// Twelve Data quotes up to 5 decimals with float noise; keep 4.
const DECIMALS: u32 = 4;

/// The last 30 daily closes (for the latest price).
pub fn latest_url(symbol: &str) -> String {
    format!(
        "{BASE}?symbol={}&interval=1day&outputsize=30&adjust=none&order=asc",
        encode(symbol)
    )
}

/// Daily closes from `from` through `to`.
pub fn span_url(symbol: &str, from: Date, to: Date) -> String {
    format!(
        "{BASE}?symbol={}&interval=1day&start_date={}&end_date={}&outputsize=5000&adjust=none&order=asc",
        encode(symbol),
        from,
        Date(to.0 + Days::days(1))
    )
}

/// Split `from..=to` into spans of at most [`MAX_SPAN_YEARS`] calendar years.
pub fn spans(from: Date, to: Date) -> Vec<(Date, Date)> {
    let mut out = Vec::new();
    let mut start = from.0;
    while start <= to.0 {
        let year_end =
            NaiveDate::from_ymd_opt(start.year() + MAX_SPAN_YEARS - 1, 12, 31).expect("valid date");
        let end = year_end.min(to.0);
        out.push((Date(start), Date(end)));
        start = end + Days::days(1);
    }
    out
}

/// Replace the key with dots wherever it appears.
pub fn scrub(text: &str, key: &str) -> String {
    let k = key.trim();
    if k.is_empty() {
        text.to_owned()
    } else {
        text.replace(k, "•••")
    }
}

#[derive(Deserialize)]
struct Envelope {
    #[serde(default)]
    status: String,
    #[serde(default)]
    code: Option<u16>,
    #[serde(default)]
    message: String,
    meta: Option<Meta>,
    #[serde(default)]
    values: Vec<Row>,
}

#[derive(Deserialize)]
struct Meta {
    currency: Option<String>,
}

#[derive(Deserialize)]
struct Row {
    datetime: String,
    close: String,
}

/// Parse a `time_series` response into closes, oldest first.
pub fn parse(symbol: &str, status: u16, body: &str) -> Result<Vec<Close>, ListedError> {
    let env: Envelope = match serde_json::from_str(body) {
        Ok(e) => e,
        Err(_) if status == 429 => return Err(ListedError::RateLimited),
        Err(_) if status == 401 || status == 403 => return Err(ListedError::KeyRejected),
        Err(_) if status != 200 => return Err(ListedError::Status(status)),
        Err(e) => return Err(ListedError::Parse(e.to_string())),
    };
    if env.status == "error" || status != 200 {
        let code = env.code.unwrap_or(status);
        let m = env.message.to_lowercase();
        return match code {
            429 => Err(ListedError::RateLimited),
            401 | 403 => Err(ListedError::KeyRejected),
            400 if m.contains("no data is available") => Ok(Vec::new()),
            400 | 404 if m.contains("symbol") || code == 404 => {
                Err(ListedError::NotFound(symbol.into()))
            }
            _ => Err(ListedError::Parse(format!("{code}: {}", env.message))),
        };
    }
    let currency = env
        .meta
        .and_then(|m| m.currency)
        .ok_or_else(|| ListedError::Parse("no currency".into()))?;
    let mut out = Vec::new();
    for r in env.values {
        let date = Date::parse(r.datetime.get(..10).unwrap_or(&r.datetime))
            .map_err(|_| ListedError::Parse(format!("bad date {}", r.datetime)))?;
        let v = decimal(&r.close)
            .ok_or_else(|| ListedError::Parse(format!("unreadable close {}", r.close)))?
            .round_dp_with_strategy(DECIMALS, RoundingStrategy::MidpointNearestEven)
            .normalize();
        let (code, value) = major_unit(&currency, v)
            .ok_or_else(|| ListedError::Parse(format!("unknown currency {currency}")))?;
        let value = value.normalize();
        out.push(Close {
            date,
            value,
            currency: code,
        });
    }
    out.sort_by_key(|c| c.date);
    out.dedup_by_key(|c| c.date);
    Ok(out)
}

fn fetch(http: &dyn Http, key: &str, symbol: &str, url: &str) -> Result<Vec<Close>, ListedError> {
    let auth = format!("apikey {}", key.trim());
    let r = http
        .get_with_headers(url, &[("Authorization", auth.as_str())])
        .map_err(|e| ListedError::Http(scrub(&e.0, key)))?;
    parse(symbol, r.status, &r.body).map_err(|e| match e {
        ListedError::Parse(m) => ListedError::Parse(scrub(&m, key)),
        other => other,
    })
}

/// The last 30 closes (the newest is the latest price).
pub fn fetch_latest(http: &dyn Http, key: &str, symbol: &str) -> Result<Vec<Close>, ListedError> {
    fetch(http, key, symbol, &latest_url(symbol))
}

/// Closes from `from` through `to`, in as many requests as [`spans`] needs,
/// waiting [`MIN_INTERVAL`] between them.
pub fn fetch_span(
    http: &dyn Http,
    key: &str,
    symbol: &str,
    from: Date,
    to: Date,
) -> Result<Vec<Close>, ListedError> {
    let mut out = Vec::new();
    for (i, (a, b)) in spans(from, to).into_iter().enumerate() {
        if i > 0 {
            http.pause(MIN_INTERVAL);
        }
        out.extend(fetch(http, key, symbol, &span_url(symbol, a, b))?);
    }
    out.sort_by_key(|c| c.date);
    out.dedup_by_key(|c| c.date);
    Ok(out)
}
