//! Stable fingerprints for import records, used to recognize re-imports.
//!
//! - With an external identifier: `x:` + hash of (adapter family, account hint, external id).
//! - Without: `h:` + hash of (account hint, kind, date, instrument key, quantity, net cash),
//!   followed by `#n`, the occurrence index among identical records in the same batch.
//!
//! Because the occurrence index is part of the fingerprint, comparing
//! fingerprints compares counts: if a batch holds three identical trades and
//! two are already stored, only the third is new.

use std::collections::HashMap;

use mizan_import::ImportRecord;
use sha2::{Digest, Sha256};

use crate::resolve::instrument_key;

fn hash(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update([0x1f]);
    }
    hex::encode(&h.finalize()[..16])
}

fn base(r: &ImportRecord) -> String {
    if let Some(ext) = r.external_id.as_deref().filter(|e| !e.trim().is_empty()) {
        return format!(
            "x:{}",
            hash(&["x", &r.source.adapter, &r.account.hint, ext.trim()])
        );
    }
    let key = r
        .instrument
        .as_ref()
        .and_then(instrument_key)
        .unwrap_or_default();
    let qty = r
        .quantity
        .map(|q| q.value().normalize().to_string())
        .unwrap_or_default();
    let cash = r
        .net_cash
        .as_ref()
        .map(|m| format!("{} {}", m.amount.value().normalize(), m.currency))
        .unwrap_or_default();
    format!(
        "h:{}",
        hash(&[
            "h",
            &r.account.hint,
            r.kind.as_str(),
            &r.date.to_string(),
            &key,
            &qty,
            &cash
        ])
    )
}

/// Fingerprints for a batch's records, in the same order.
pub fn fingerprints<'a>(records: impl IntoIterator<Item = &'a ImportRecord>) -> Vec<String> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    records
        .into_iter()
        .map(|r| {
            let b = base(r);
            if b.starts_with("x:") {
                return b;
            }
            let n = seen.entry(b.clone()).or_insert(0);
            let fp = format!("{b}#{n}");
            *n += 1;
            fp
        })
        .collect()
}

/// True when the fingerprint came from a broker transaction identifier.
pub fn is_external(fingerprint: &str) -> bool {
    fingerprint.starts_with("x:")
}
