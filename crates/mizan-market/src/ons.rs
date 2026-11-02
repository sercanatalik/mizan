//! UK Office for National Statistics CPI index (series D7BT, 2015=100). The
//! time-series page serves the whole history as one JSON document.

use serde::Deserialize;

use crate::http::Http;
use crate::inflation::{index_value, ok_body, IndexValue, InflationError, YearMonth};

pub const SOURCE: &str = "ons";
pub const BASE: &str = "https://www.ons.gov.uk/economy/inflationandpriceindices/timeseries/";

pub fn url(series: &str) -> String {
    format!("{BASE}{}/mm23/data", series.to_lowercase())
}

#[derive(Deserialize)]
struct Document {
    #[serde(default)]
    months: Vec<Point>,
}

#[derive(Deserialize)]
struct Point {
    date: String,
    value: String,
}

/// `"2026 SEP"` → 2026-09.
fn month(date: &str) -> Option<YearMonth> {
    let (y, m) = date.trim().split_once(' ')?;
    YearMonth::new(y.parse().ok()?, YearMonth::month_from_name(m)?)
}

pub fn parse(body: &str) -> Result<Vec<IndexValue>, InflationError> {
    let d: Document =
        serde_json::from_str(body).map_err(|e| InflationError::Parse(e.to_string()))?;
    if d.months.is_empty() {
        return Err(InflationError::Parse("no monthly values".into()));
    }
    let mut out = Vec::new();
    for p in d.months {
        let m =
            month(&p.date).ok_or_else(|| InflationError::Parse(format!("bad month {}", p.date)))?;
        out.push(IndexValue {
            month: m,
            value: index_value(&p.value)?,
        });
    }
    out.sort_by_key(|v| v.month);
    Ok(out)
}

pub fn fetch(http: &dyn Http, series: &str) -> Result<Vec<IndexValue>, InflationError> {
    let r = http
        .get(&url(series))
        .map_err(|e| InflationError::Http(e.to_string()))?;
    parse(&ok_body(r)?)
}
