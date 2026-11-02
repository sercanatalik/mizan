//! TCMB EVDS: the monthly Turkish domestic producer price index (Yİ-ÜFE),
//! used to index the cost of foreign securities in Turkish tax reports.
//!
//! `GET https://evds3.tcmb.gov.tr/igmevdsms-dis/series=<code>&startDate=DD-MM-YYYY&endDate=DD-MM-YYYY&type=json`
//! with the user's API key in the `key` request header, never in the URL.
//! Without the header the service answers 403. Monthly values come back as
//! `{"items": [{"Tarih": "2024-1", "TP_TUFE1YI_T1": "3000.51"}, ...]}`; a month
//! not yet published has a null value.

use serde_json::Value;

use crate::http::Http;
use crate::inflation::{index_value, IndexValue, InflationError, YearMonth};

pub const SOURCE: &str = "evds";
pub const BASE: &str = "https://evds3.tcmb.gov.tr/igmevdsms-dis/";
pub use mizan_storage::inflation::PPI_TR as SERIES;

/// Request URL for months `from` through `to`. Holds no key.
pub fn url(series: &str, from: YearMonth, to: YearMonth) -> String {
    format!(
        "{BASE}series={series}&startDate=01-{:02}-{}&endDate=28-{:02}-{}&type=json",
        from.month, from.year, to.month, to.year
    )
}

/// `"2024-1"` or `"2024-01"` → 2024-01.
fn month(text: &str) -> Option<YearMonth> {
    let (y, m) = text.trim().split_once('-')?;
    YearMonth::new(y.parse().ok()?, m.parse().ok()?)
}

pub fn parse(body: &str, series: &str) -> Result<Vec<IndexValue>, InflationError> {
    let doc: Value =
        serde_json::from_str(body).map_err(|e| InflationError::Parse(e.to_string()))?;
    let items = doc.get("items").and_then(Value::as_array).ok_or_else(|| {
        match doc.get("message").and_then(Value::as_str) {
            Some(m) => InflationError::Api(m.to_owned()),
            None => InflationError::Parse("no items".into()),
        }
    })?;
    let field = series.replace('.', "_");
    let mut out = Vec::new();
    for item in items {
        let date = item.get("Tarih").and_then(Value::as_str).unwrap_or("");
        let m = month(date).ok_or_else(|| InflationError::Parse(format!("bad month {date:?}")))?;
        let value = match item.get(&field) {
            None | Some(Value::Null) => continue,
            Some(Value::String(s)) if s.trim().is_empty() => continue,
            Some(Value::String(s)) => index_value(s)?,
            Some(Value::Number(n)) => index_value(&n.to_string())?,
            Some(other) => return Err(InflationError::Parse(format!("bad value {other}"))),
        };
        out.push(IndexValue { month: m, value });
    }
    if out.is_empty() && !items.is_empty() {
        return Err(InflationError::Parse(format!("no values for {series}")));
    }
    out.sort_by_key(|v| v.month);
    Ok(out)
}

pub fn fetch(
    http: &dyn Http,
    key: &str,
    series: &str,
    from: YearMonth,
    to: YearMonth,
) -> Result<Vec<IndexValue>, InflationError> {
    let r = http
        .get_with_headers(&url(series, from, to), &[("key", key)])
        .map_err(|e| InflationError::Http(e.to_string()))?;
    match r.status {
        200 => {}
        401 | 403 => return Err(InflationError::Api("API key rejected".into())),
        s => return Err(InflationError::Status(s)),
    }
    let mut values = parse(&r.body, series)?;
    values.retain(|v| v.month >= from && v.month <= to);
    Ok(values)
}
