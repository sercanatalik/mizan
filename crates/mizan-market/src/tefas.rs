//! TEFAS fund prices.
//!
//! The request and response shape were verified on 2026-11-01 (see
//! `docs/market-data.md`). One bulk request per fund type returns every fund's
//! daily prices for a date range; mapped fund codes are matched locally.

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
use std::time::Duration;

use chrono::Duration as Days;
use mizan_import::Date;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::value::RawValue;

use crate::http::Http;

pub const URL: &str = "https://www.tefas.gov.tr/api/funds/fonGnlBlgSiraliGetir";
pub const SOURCE: &str = "tefas";
/// Investment funds, then pension funds.
pub const FUND_TYPES: [&str; 2] = ["YAT", "EMK"];
/// TEFAS allows about six requests a minute.
pub const MIN_INTERVAL: Duration = Duration::from_secs(10);
/// How far back a refresh looks for the latest price (covers weekends and short holidays).
pub const LOOKBACK_DAYS: i64 = 7;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TefasError {
    #[error("TEFAS request failed: {0}")]
    Http(String),
    #[error("TEFAS returned HTTP {0}")]
    Status(u16),
    #[error("TEFAS error: {0}")]
    Api(String),
    #[error("TEFAS response could not be read: {0}")]
    Parse(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FundPrice {
    pub code: String,
    pub date: Date,
    /// Unit price in TRY.
    pub price: Decimal,
}

#[derive(Deserialize)]
struct Envelope<'a> {
    #[serde(rename = "errorMessage")]
    error_message: Option<String>,
    #[serde(rename = "resultList", borrow)]
    result_list: Option<Vec<Row<'a>>>,
}

#[derive(Deserialize)]
struct Row<'a> {
    #[serde(rename = "fonKodu")]
    code: String,
    tarih: String,
    /// Kept as raw JSON text so the price never passes through binary floating point.
    #[serde(borrow)]
    fiyat: Option<&'a RawValue>,
}

/// TEFAS reports an empty range as an error message; treat those as no rows.
fn is_empty_marker(msg: &str) -> bool {
    let m = msg.to_lowercase();
    m.contains("out of bounds") || m.contains("veri bulunamad")
}

fn decimal(raw: &str) -> Option<Decimal> {
    let t = raw.trim().trim_matches('"');
    Decimal::from_str(t)
        .ok()
        .or_else(|| Decimal::from_scientific(t).ok())
}

/// Parse a `fonGnlBlgSiraliGetir` response body.
pub fn parse(body: &str) -> Result<Vec<FundPrice>, TefasError> {
    let env: Envelope = serde_json::from_str(body).map_err(|e| TefasError::Parse(e.to_string()))?;
    if let Some(msg) = env.error_message.filter(|m| !m.trim().is_empty()) {
        if is_empty_marker(&msg) {
            return Ok(Vec::new());
        }
        return Err(TefasError::Api(msg));
    }
    let mut out = Vec::new();
    for row in env.result_list.unwrap_or_default() {
        let (Some(raw), Ok(date)) = (
            row.fiyat,
            Date::parse(row.tarih.get(..10).unwrap_or(&row.tarih)),
        ) else {
            continue;
        };
        if let Some(price) = decimal(raw.get()) {
            out.push(FundPrice {
                code: row.code.trim().to_uppercase(),
                date,
                price,
            });
        }
    }
    Ok(out)
}

/// The JSON body for one bulk request (all funds of `fund_type` between two dates).
pub fn request_body(fund_type: &str, from: Date, to: Date) -> String {
    serde_json::json!({
        "fonTipi": fund_type, "fonKodu": null,
        "aramaMetni": null, "fonTurKod": null, "fonGrubu": null, "sfonTurKod": null,
        "fonTurAciklama": null, "kurucuKod": null,
        "basTarih": from.0.format("%Y%m%d").to_string(), "bitTarih": to.0.format("%Y%m%d").to_string(),
        "basSira": 1, "bitSira": 100000,
        "dil": "TR", "sFonTurKod": "", "fonKod": "", "fonGrup": "", "fonUnvanTip": ""
    })
    .to_string()
}

/// Result of fetching the latest prices for a set of fund codes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fetched {
    /// Latest price per fund code.
    pub latest: BTreeMap<String, FundPrice>,
    /// Requested codes TEFAS did not list in any fund type.
    pub not_found: Vec<String>,
    /// Set when a request failed; prices from earlier successful requests are kept.
    pub error: Option<String>,
}

/// Latest price for each of `codes`, looking back [`LOOKBACK_DAYS`] from `today`.
pub fn fetch_latest(http: &dyn Http, codes: &BTreeSet<String>, today: Date) -> Fetched {
    let mut out = Fetched::default();
    let mut remaining: BTreeSet<String> = codes.iter().map(|c| c.trim().to_uppercase()).collect();
    let from = Date(today.0 - Days::days(LOOKBACK_DAYS - 1));
    for (i, fund_type) in FUND_TYPES.iter().enumerate() {
        if remaining.is_empty() {
            break;
        }
        if i > 0 {
            http.pause(MIN_INTERVAL);
        }
        let result = http
            .post_json(URL, &request_body(fund_type, from, today))
            .map_err(|e| TefasError::Http(e.0))
            .and_then(|r| {
                if r.status == 200 {
                    parse(&r.body)
                } else {
                    Err(TefasError::Status(r.status))
                }
            });
        match result {
            Ok(rows) => {
                for p in rows.into_iter().filter(|p| remaining.contains(&p.code)) {
                    let keep = out.latest.get(&p.code).is_none_or(|q| p.date > q.date);
                    if keep {
                        out.latest.insert(p.code.clone(), p);
                    }
                }
                remaining.retain(|c| !out.latest.contains_key(c));
            }
            Err(e) => {
                out.error = Some(e.to_string());
                return out;
            }
        }
    }
    out.not_found = remaining.into_iter().collect();
    out
}
