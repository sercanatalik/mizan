//! Eurostat harmonised consumer price indices (dataset `prc_hicp_minr`), for
//! the euro area and Türkiye. Public dissemination API, no key. The response is
//! JSON-stat: values are keyed by their flat position across all dimensions.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::value::RawValue;

use crate::http::Http;
use crate::inflation::{index_value, ok_body, IndexValue, InflationError, YearMonth};

pub const SOURCE: &str = "eurostat";
pub const BASE: &str =
    "https://ec.europa.eu/eurostat/api/dissemination/statistics/1.0/data/prc_hicp_minr";

/// Series ids are `GEO:UNIT`, for example `TR:I15`.
fn split(series: &str) -> Result<(&str, &str), InflationError> {
    series
        .split_once(':')
        .ok_or_else(|| InflationError::Api(format!("bad Eurostat series {series}")))
}

pub fn url(series: &str, from: YearMonth) -> Result<String, InflationError> {
    let (geo, unit) = split(series)?;
    Ok(format!(
        "{BASE}?format=JSON&geo={geo}&unit={unit}&coicop18=TOTAL&sinceTimePeriod={from}"
    ))
}

#[derive(Deserialize)]
struct Dataset<'a> {
    #[serde(default)]
    id: Vec<String>,
    #[serde(default)]
    size: Vec<usize>,
    #[serde(default)]
    dimension: BTreeMap<String, Dimension>,
    #[serde(borrow, default)]
    value: BTreeMap<String, &'a RawValue>,
    #[serde(default)]
    error: Option<Vec<ApiError>>,
}

#[derive(Deserialize)]
struct Dimension {
    category: Category,
}

#[derive(Deserialize)]
struct Category {
    #[serde(default)]
    index: BTreeMap<String, usize>,
}

#[derive(Deserialize)]
struct ApiError {
    #[serde(default)]
    label: String,
}

/// Values of one geo from a JSON-stat body. Other dimensions must hold one category.
pub fn parse(body: &str, geo: &str) -> Result<Vec<IndexValue>, InflationError> {
    let d: Dataset =
        serde_json::from_str(body).map_err(|e| InflationError::Parse(e.to_string()))?;
    if let Some(errors) = d.error.filter(|e| !e.is_empty()) {
        let labels: Vec<String> = errors.into_iter().map(|e| e.label).collect();
        return Err(InflationError::Api(labels.join("; ")));
    }
    if d.id.len() != d.size.len() {
        return Err(InflationError::Parse("dimension sizes do not match".into()));
    }
    // Row-major strides: the last dimension varies fastest.
    let mut strides = vec![1usize; d.size.len()];
    for i in (0..d.size.len().saturating_sub(1)).rev() {
        strides[i] = strides[i + 1] * d.size[i + 1];
    }
    let mut base = 0;
    let mut time = None;
    for (i, name) in d.id.iter().enumerate() {
        let dim = d
            .dimension
            .get(name)
            .ok_or_else(|| InflationError::Parse(format!("missing dimension {name}")))?;
        match name.as_str() {
            "time" => time = Some((i, &dim.category.index)),
            "geo" => match dim.category.index.get(geo) {
                Some(&p) => base += p * strides[i],
                None => return Ok(vec![]),
            },
            _ if d.size[i] == 1 => {}
            _ => {
                return Err(InflationError::Parse(format!(
                    "dimension {name} has {} categories",
                    d.size[i]
                )))
            }
        }
    }
    let (ti, months) = time.ok_or_else(|| InflationError::Parse("no time dimension".into()))?;
    let mut out = Vec::new();
    for (label, &p) in months {
        let Some(raw) = d.value.get(&(base + p * strides[ti]).to_string()) else {
            continue;
        };
        let month = YearMonth::parse(label)
            .ok_or_else(|| InflationError::Parse(format!("bad month {label}")))?;
        out.push(IndexValue {
            month,
            value: index_value(raw.get())?,
        });
    }
    out.sort_by_key(|v| v.month);
    Ok(out)
}

pub fn fetch(
    http: &dyn Http,
    series: &str,
    from: YearMonth,
) -> Result<Vec<IndexValue>, InflationError> {
    let (geo, _) = split(series)?;
    let r = http
        .get(&url(series, from)?)
        .map_err(|e| InflationError::Http(e.to_string()))?;
    if r.status != 200 {
        // Error bodies carry a readable label; fall back to the status.
        return match parse(&r.body, geo) {
            Err(e @ InflationError::Api(_)) => Err(e),
            _ => Err(InflationError::Status(r.status)),
        };
    }
    parse(&ok_body(r)?, geo)
}
