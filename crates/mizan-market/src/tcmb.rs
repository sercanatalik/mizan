//! TCMB (Turkish central bank) daily exchange rates.
//!
//! `https://www.tcmb.gov.tr/kurlar/today.xml` lists currencies with `Unit` and
//! `ForexBuying` (TRY per `Unit` units). Mizan stores the forex buying rate per
//! one unit, dated by the file's own date. No file is published on weekends and
//! holidays; then the stored rates simply stay as they are.

use std::str::FromStr;

use mizan_import::{Code, Date};
use quick_xml::events::Event;
use quick_xml::Reader;
use rust_decimal::Decimal;

use crate::http::Http;

pub const TODAY_URL: &str = "https://www.tcmb.gov.tr/kurlar/today.xml";
pub const SOURCE: &str = "tcmb";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rate {
    pub currency: Code,
    pub date: Date,
    /// TRY per one unit of `currency`.
    pub try_per_unit: Decimal,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TcmbError {
    #[error("TCMB request failed: {0}")]
    Http(String),
    #[error("TCMB returned HTTP {0}")]
    Status(u16),
    #[error("TCMB response could not be read: {0}")]
    Parse(String),
}

/// Parse a TCMB daily XML document.
pub fn parse(xml: &str) -> Result<Vec<Rate>, TcmbError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut date: Option<Date> = None;
    let mut out = Vec::new();
    let mut code: Option<String> = None;
    let mut field: Option<String> = None;
    let mut unit: Option<Decimal> = None;
    let mut buying: Option<Decimal> = None;

    loop {
        match reader
            .read_event()
            .map_err(|e| TcmbError::Parse(e.to_string()))?
        {
            Event::Start(e) | Event::Empty(e) if e.name().as_ref() == b"Tarih_Date" => {
                for a in e.attributes().flatten() {
                    if a.key.as_ref() == b"Tarih" {
                        let v = String::from_utf8_lossy(&a.value).to_string();
                        let p: Vec<&str> = v.split('.').collect();
                        if let [d, m, y] = p.as_slice() {
                            date = Date::parse(&format!("{y}-{m}-{d}")).ok();
                        }
                    }
                }
            }
            Event::Start(e) if e.name().as_ref() == b"Currency" => {
                code = e
                    .attributes()
                    .flatten()
                    .find(|a| a.key.as_ref() == b"CurrencyCode")
                    .map(|a| String::from_utf8_lossy(&a.value).to_string());
                unit = None;
                buying = None;
            }
            Event::Start(e) => field = Some(String::from_utf8_lossy(e.name().as_ref()).to_string()),
            Event::Text(t) => {
                let text = t
                    .decode()
                    .map_err(|e| TcmbError::Parse(e.to_string()))?
                    .trim()
                    .to_string();
                match field.as_deref() {
                    Some("Unit") => unit = Decimal::from_str(&text).ok(),
                    Some("ForexBuying") => buying = Decimal::from_str(&text).ok(),
                    _ => {}
                }
            }
            Event::End(e) if e.name().as_ref() == b"Currency" => {
                let (Some(c), Some(u), Some(b), Some(d)) = (code.take(), unit, buying, date) else {
                    continue;
                };
                if u.is_zero() {
                    continue;
                }
                if let Ok(currency) = Code::parse(&c) {
                    out.push(Rate {
                        currency,
                        date: d,
                        try_per_unit: (b / u).normalize(),
                    });
                }
            }
            Event::End(_) => field = None,
            Event::Eof => break,
            _ => {}
        }
    }
    if date.is_none() {
        return Err(TcmbError::Parse("no Tarih_Date date in document".into()));
    }
    Ok(out)
}

/// Fetch today's rates. `Ok(None)` when TCMB published no file (weekend or holiday).
pub fn fetch(http: &dyn Http) -> Result<Option<Vec<Rate>>, TcmbError> {
    let r = http.get(TODAY_URL).map_err(|e| TcmbError::Http(e.0))?;
    match r.status {
        200 => parse(&r.body).map(Some),
        404 => Ok(None),
        s => Err(TcmbError::Status(s)),
    }
}

/// The archive file for one day: `https://www.tcmb.gov.tr/kurlar/YYYYMM/DDMMYYYY.xml`.
pub fn archive_url(date: Date) -> String {
    format!(
        "https://www.tcmb.gov.tr/kurlar/{}/{}.xml",
        date.0.format("%Y%m"),
        date.0.format("%d%m%Y")
    )
}

/// Rates published for `date`. `Ok(None)` when TCMB published no file that day
/// (weekend or holiday).
pub fn fetch_day(http: &dyn Http, date: Date) -> Result<Option<Vec<Rate>>, TcmbError> {
    let r = http
        .get(&archive_url(date))
        .map_err(|e| TcmbError::Http(e.0))?;
    match r.status {
        200 => parse(&r.body).map(Some),
        404 => Ok(None),
        s => Err(TcmbError::Status(s)),
    }
}
