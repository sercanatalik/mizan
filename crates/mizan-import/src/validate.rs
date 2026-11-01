//! Batch parsing and validation.
//!
//! A batch with a missing or unsupported schema version is rejected whole.
//! Otherwise each record and assertion is validated on its own: invalid items
//! are reported as [`ItemError`]s and left out, valid items are kept.

use std::fmt;

use rust_decimal::Decimal;
use serde_json::{Map, Value};
use thiserror::Error;

use crate::record::{Assertion, Diagnostic, ImportRecord, InstrumentHint, Kind, SCHEMA_V1};

/// Field names that look like internal Mizan identifiers. Adapters must use hints instead.
const INTERNAL_ID_KEYS: [&str; 6] = [
    "id",
    "account_id",
    "asset_id",
    "activity_id",
    "instrument_id",
    "batch_id",
];

/// Errors that reject a whole batch.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum BatchError {
    #[error("invalid JSON: {0}")]
    Json(String),
    #[error("a batch must be a JSON object")]
    NotAnObject,
    #[error("the batch has no \"schema\" field; expected \"{SCHEMA_V1}\"")]
    MissingSchema,
    #[error("unsupported schema version {0:?}; this version of Mizan supports \"{SCHEMA_V1}\"")]
    UnsupportedVersion(String),
    #[error("malformed batch: {0}")]
    Malformed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Record,
    Assertion,
    Diagnostic,
}

/// A validation error for one record, assertion or diagnostic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemError {
    pub item: ItemKind,
    /// Zero-based position in its array.
    pub index: usize,
    /// The item's locator, when it could be read.
    pub locator: Option<String>,
    pub message: String,
}

impl fmt::Display for ItemError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let what = match self.item {
            ItemKind::Record => "record",
            ItemKind::Assertion => "assertion",
            ItemKind::Diagnostic => "diagnostic",
        };
        write!(f, "{what} #{}", self.index)?;
        if let Some(l) = &self.locator {
            write!(f, " ({l})")?;
        }
        write!(f, ": {}", self.message)
    }
}

/// An item together with its position in the batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Indexed<T> {
    pub index: usize,
    pub item: T,
}

/// The valid contents of a batch plus the errors for the items that were rejected.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParsedBatch {
    pub records: Vec<Indexed<ImportRecord>>,
    pub assertions: Vec<Indexed<Assertion>>,
    pub diagnostics: Vec<Diagnostic>,
    pub errors: Vec<ItemError>,
}

impl ParsedBatch {
    pub fn is_clean(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Parse and validate a batch from JSON text.
pub fn parse_batch(json: &str) -> Result<ParsedBatch, BatchError> {
    let value: Value = serde_json::from_str(json).map_err(|e| BatchError::Json(e.to_string()))?;
    parse_batch_value(value)
}

/// Parse and validate a batch from an already-parsed JSON value.
pub fn parse_batch_value(value: Value) -> Result<ParsedBatch, BatchError> {
    let Value::Object(mut top) = value else {
        return Err(BatchError::NotAnObject);
    };
    match top.get("schema") {
        None => return Err(BatchError::MissingSchema),
        Some(Value::String(s)) if s == SCHEMA_V1 => {}
        Some(Value::String(s)) => return Err(BatchError::UnsupportedVersion(s.clone())),
        Some(other) => return Err(BatchError::UnsupportedVersion(other.to_string())),
    }
    for key in top.keys() {
        if !matches!(key.as_str(), "schema" | "records" | "assertions" | "diagnostics") {
            return Err(BatchError::Malformed(format!("unknown top-level field {key:?}")));
        }
    }

    let mut out = ParsedBatch::default();

    for (index, item) in take_array(&mut top, "records")?.into_iter().enumerate() {
        let locator = locator_of(&item);
        let parsed = check_internal_ids_record(&item)
            .and_then(|_| serde_json::from_value::<ImportRecord>(item).map_err(|e| e.to_string()))
            .and_then(|r| {
                let errs = validate_record(&r);
                if errs.is_empty() {
                    Ok(r)
                } else {
                    Err(errs.join("; "))
                }
            });
        match parsed {
            Ok(item) => out.records.push(Indexed { index, item }),
            Err(message) => out.errors.push(ItemError { item: ItemKind::Record, index, locator, message }),
        }
    }

    for (index, item) in take_array(&mut top, "assertions")?.into_iter().enumerate() {
        let locator = locator_of(&item);
        let parsed = check_internal_ids_assertion(&item)
            .and_then(|_| serde_json::from_value::<Assertion>(item).map_err(|e| e.to_string()))
            .and_then(|a| {
                let errs = validate_assertion(&a);
                if errs.is_empty() {
                    Ok(a)
                } else {
                    Err(errs.join("; "))
                }
            });
        match parsed {
            Ok(item) => out.assertions.push(Indexed { index, item }),
            Err(message) => out.errors.push(ItemError { item: ItemKind::Assertion, index, locator, message }),
        }
    }

    for (index, item) in take_array(&mut top, "diagnostics")?.into_iter().enumerate() {
        let locator = item.get("locator").and_then(Value::as_str).map(str::to_owned);
        match serde_json::from_value::<Diagnostic>(item) {
            Ok(d) if d.locator.trim().is_empty() => out.errors.push(ItemError {
                item: ItemKind::Diagnostic,
                index,
                locator,
                message: "diagnostic locator must not be empty".into(),
            }),
            Ok(d) => out.diagnostics.push(d),
            Err(e) => out.errors.push(ItemError { item: ItemKind::Diagnostic, index, locator, message: e.to_string() }),
        }
    }

    Ok(out)
}

fn take_array(top: &mut Map<String, Value>, key: &str) -> Result<Vec<Value>, BatchError> {
    match top.remove(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(a)) => Ok(a),
        Some(_) => Err(BatchError::Malformed(format!("\"{key}\" must be an array"))),
    }
}

fn locator_of(item: &Value) -> Option<String> {
    item.get("source")?.get("locator")?.as_str().map(str::to_owned)
}

fn internal_id_in(obj: Option<&Value>, path: &str) -> Result<(), String> {
    if let Some(Value::Object(map)) = obj {
        if let Some(k) = INTERNAL_ID_KEYS.iter().find(|k| map.contains_key(**k)) {
            let at = if path.is_empty() { String::new() } else { format!(" in \"{path}\"") };
            return Err(format!(
                "field \"{k}\"{at} looks like an internal Mizan identifier; adapters must reference accounts and instruments by hints"
            ));
        }
    }
    Ok(())
}

fn check_internal_ids_record(item: &Value) -> Result<(), String> {
    internal_id_in(Some(item), "")?;
    internal_id_in(item.get("account"), "account")?;
    internal_id_in(item.get("instrument"), "instrument")?;
    if let Some(Value::Array(legs)) = item.get("legs") {
        for leg in legs {
            internal_id_in(leg.get("instrument"), "legs[].instrument")?;
        }
    }
    Ok(())
}

fn check_internal_ids_assertion(item: &Value) -> Result<(), String> {
    internal_id_in(Some(item), "")?;
    internal_id_in(item.get("account"), "account")?;
    if let Some(Value::Array(ps)) = item.get("positions") {
        for p in ps {
            internal_id_in(p.get("instrument"), "positions[].instrument")?;
        }
    }
    Ok(())
}

fn check_instrument(h: &InstrumentHint, path: &str, errs: &mut Vec<String>) {
    if !h.has_identifier() {
        errs.push(format!("{path} needs at least one of isin, cusip, figi, symbol or name"));
    }
}

/// Semantic checks for one record. Returns human-readable problems; empty means valid.
pub fn validate_record(r: &ImportRecord) -> Vec<String> {
    let mut errs = Vec::new();
    let zero = Decimal::ZERO;

    if r.source.adapter.trim().is_empty() || r.source.adapter_version.trim().is_empty() {
        errs.push("source.adapter and source.adapter_version must not be empty".into());
    }
    if r.source.locator.trim().is_empty() {
        errs.push("source.locator must not be empty".into());
    }
    if r.account.hint.trim().is_empty() {
        errs.push("account.hint must not be empty".into());
    }
    if let Some(i) = &r.instrument {
        check_instrument(i, "instrument", &mut errs);
    }

    for c in &r.charges {
        if c.amount.amount.value() < zero {
            errs.push(format!("charge amount must not be negative, got {}", c.amount.amount));
        }
    }

    if r.kind.carries_legs() {
        if r.legs.is_empty() {
            errs.push(format!("{} records must carry at least one leg", r.kind.as_str()));
        }
        for (i, leg) in r.legs.iter().enumerate() {
            check_instrument(&leg.instrument, &format!("legs[{i}].instrument"), &mut errs);
            if let Some(s) = leg.cost_basis_share {
                if s.value() < zero || s.value() > Decimal::ONE {
                    errs.push(format!("legs[{i}].cost_basis_share must be between 0 and 1, got {s}"));
                }
            }
        }
    } else if !r.legs.is_empty() {
        errs.push(format!("only exchange and spinoff records may carry legs, not {}", r.kind.as_str()));
    }

    match (&r.kind, &r.split) {
        (Kind::Split, None) => errs.push("split records must carry a split ratio".into()),
        (Kind::Split, Some(s)) => {
            if s.new.value() <= zero || s.old.value() <= zero {
                errs.push(format!("split ratio must be positive, got new {} / old {}", s.new, s.old));
            }
        }
        (_, Some(_)) => errs.push(format!("only split records may carry a split ratio, not {}", r.kind.as_str())),
        _ => {}
    }

    if matches!(r.kind, Kind::PositionIn | Kind::PositionOut) {
        if r.instrument.is_none() {
            errs.push(format!("{} records must name an instrument", r.kind.as_str()));
        }
        match r.quantity {
            None => errs.push(format!("{} records must carry a quantity", r.kind.as_str())),
            Some(q) if q.value() <= zero => errs.push(format!("{} quantity must be positive, got {q}", r.kind.as_str())),
            _ => {}
        }
    }
    if r.kind != Kind::PositionIn && (r.cost_basis.is_some() || r.acquisition_date.is_some()) {
        errs.push(format!(
            "cost_basis and acquisition_date are only allowed on position_in, not {}",
            r.kind.as_str()
        ));
    }

    if r.kind == Kind::Adjustment && r.memo.as_deref().is_none_or(|m| m.trim().is_empty()) {
        errs.push("adjustment records must carry a memo explaining the adjustment".into());
    }

    for (i, l) in r.links.iter().enumerate() {
        if l.target.trim().is_empty() {
            errs.push(format!("links[{i}].target must name an external identifier"));
        }
    }

    errs
}

/// Semantic checks for one balance assertion.
pub fn validate_assertion(a: &Assertion) -> Vec<String> {
    let mut errs = Vec::new();
    if a.source.locator.trim().is_empty() {
        errs.push("source.locator must not be empty".into());
    }
    if a.account.hint.trim().is_empty() {
        errs.push("account.hint must not be empty".into());
    }
    if a.cash.is_empty() && a.positions.is_empty() {
        errs.push("an assertion must state at least one cash balance or position".into());
    }
    for (i, p) in a.positions.iter().enumerate() {
        check_instrument(&p.instrument, &format!("positions[{i}].instrument"), &mut errs);
    }
    errs
}
