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
    /// Fund type (`YAT`, `EMK`) each found code was listed under.
    pub fund_types: BTreeMap<String, String>,
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
                        out.fund_types.insert(p.code.clone(), fund_type.to_string());
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

// ---------------------------------------------------------------- history

/// TEFAS refuses start dates more than five years before today.
pub const HISTORY_YEARS: i32 = 5;

/// The earliest start date TEFAS accepts on `today` (five years back, plus one
/// day of margin so the boundary is never hit).
pub fn earliest_history_date(today: Date) -> Date {
    use chrono::Datelike;
    let t = today.0;
    let back = t
        .with_year(t.year() - HISTORY_YEARS)
        .or_else(|| {
            t.pred_opt()
                .and_then(|d| d.with_year(t.year() - HISTORY_YEARS))
        })
        .unwrap_or(t);
    Date(back.succ_opt().unwrap_or(back))
}

/// Outcome of a one-month history request for one fund.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MonthFetch {
    /// Prices published in the range (possibly none).
    Prices(Vec<FundPrice>),
    /// TEFAS refuses the range as more than five years old.
    TooOld,
}

fn is_too_old(msg: &str) -> bool {
    msg.to_lowercase().contains("yıldan eski")
}

/// Prices of one fund between `from` and `to` (at most one month), from a
/// single-fund request with the given fund type.
pub fn fetch_range(
    http: &dyn Http,
    fund_type: &str,
    code: &str,
    from: Date,
    to: Date,
) -> Result<MonthFetch, TefasError> {
    let mut body: serde_json::Value =
        serde_json::from_str(&request_body(fund_type, from, to)).expect("valid body");
    body["fonKodu"] = serde_json::Value::String(code.to_string());
    let r = http
        .post_json(URL, &body.to_string())
        .map_err(|e| TefasError::Http(e.0))?;
    if r.status != 200 {
        return Err(TefasError::Status(r.status));
    }
    match parse(&r.body) {
        Ok(rows) => Ok(MonthFetch::Prices(
            rows.into_iter().filter(|p| p.code == code).collect(),
        )),
        Err(TefasError::Api(msg)) if is_too_old(&msg) => Ok(MonthFetch::TooOld),
        Err(e) => Err(e),
    }
}
