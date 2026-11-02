//! US Bureau of Labor Statistics CPI (public API v1, no key). One request
//! covers at most 10 years; the keyless plan allows 25 requests a day.

use std::time::Duration;

use serde::Deserialize;

use crate::http::Http;
use crate::inflation::{index_value, ok_body, IndexValue, InflationError, YearMonth};

pub const SOURCE: &str = "bls";
pub const URL: &str = "https://api.bls.gov/publicAPI/v1/timeseries/data/";
pub const MAX_YEARS: i32 = 10;
pub const MIN_INTERVAL: Duration = Duration::from_secs(1);

/// Year windows of at most 10 years covering `from..=to`, newest first.
pub fn windows(from: i32, to: i32) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    let mut end = to;
    while end >= from {
        let start = (end - MAX_YEARS + 1).max(from);
        out.push((start, end));
        end = start - 1;
    }
    out
}

pub fn body(series: &str, start: i32, end: i32) -> String {
    serde_json::json!({
        "seriesid": [series],
        "startyear": start.to_string(),
        "endyear": end.to_string(),
    })
    .to_string()
}

#[derive(Deserialize)]
struct Envelope {
    status: String,
    #[serde(default)]
    message: Vec<String>,
    #[serde(rename = "Results", default)]
    results: Option<Results>,
}

#[derive(Deserialize)]
struct Results {
    #[serde(default)]
    series: Vec<Series>,
}

#[derive(Deserialize)]
struct Series {
    #[serde(default)]
    data: Vec<Point>,
}

#[derive(Deserialize)]
struct Point {
    year: String,
    period: String,
    value: String,
}

pub fn parse(body: &str) -> Result<Vec<IndexValue>, InflationError> {
    let e: Envelope =
        serde_json::from_str(body).map_err(|e| InflationError::Parse(e.to_string()))?;
    if e.status != "REQUEST_SUCCEEDED" {
        return Err(InflationError::Api(if e.message.is_empty() {
            e.status
        } else {
            e.message.join("; ")
        }));
    }
    let data: Vec<Point> = e
        .results
        .into_iter()
        .flat_map(|r| r.series)
        .flat_map(|s| s.data)
        .collect();
    if data.is_empty() && !e.message.is_empty() {
        return Err(InflationError::Api(e.message.join("; ")));
    }
    let mut out = Vec::new();
    for p in data {
        // M01..M12 are months; M13 is the annual average.
        // "-" marks a month with no published value (for example October 2025,
        // when US CPI was not collected); it is left out, not an error.
        if p.value.trim() == "-" {
            continue;
        }
        let Some(month) = p
            .period
            .strip_prefix('M')
            .and_then(|m| m.parse::<u32>().ok())
            .filter(|m| (1..=12).contains(m))
        else {
            continue;
        };
        let year = p
            .year
            .parse()
            .map_err(|_| InflationError::Parse(format!("bad year {}", p.year)))?;
        out.push(IndexValue {
            month: YearMonth::new(year, month).expect("checked"),
            value: index_value(&p.value)?,
        });
    }
    out.sort_by_key(|v| v.month);
    Ok(out)
}

pub fn fetch(
    http: &dyn Http,
    series: &str,
    from: YearMonth,
    to: YearMonth,
) -> Result<Vec<IndexValue>, InflationError> {
    let mut out = Vec::new();
    for (i, (start, end)) in windows(from.year, to.year).into_iter().enumerate() {
        if i > 0 {
            http.pause(MIN_INTERVAL);
        }
        let r = http
            .post_json(URL, &body(series, start, end))
            .map_err(|e| InflationError::Http(e.to_string()))?;
        out.extend(parse(&ok_body(r)?)?);
    }
    Ok(out)
}
