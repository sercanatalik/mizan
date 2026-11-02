//! Choosing adapters for a file.

use serde::Serialize;

use crate::registry::{Origin, Registry};
use crate::FileInput;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Match {
    pub id: String,
    pub name: String,
    pub version: String,
    pub score: u8,
    pub origin: Origin,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Detection {
    /// Already a `mizan.import/1` batch: no adapter needed.
    Batch,
    /// Adapters that recognize the file, best first.
    Matches(Vec<Match>),
    /// No adapter recognizes it.
    Unrecognized,
}

/// A JSON object with a `"schema": "mizan.import/…"` field.
pub fn is_batch(file: &FileInput) -> bool {
    let head = String::from_utf8_lossy(&file.bytes[..file.bytes.len().min(4096)]);
    let t = head.trim_start_matches('\u{feff}').trim_start();
    t.starts_with('{') && t.contains("\"schema\"") && t.contains("mizan.import/")
}

pub fn detect(reg: &Registry, file: &FileInput) -> Detection {
    if is_batch(file) {
        return Detection::Batch;
    }
    let mut matches: Vec<Match> = reg
        .loaded
        .iter()
        .filter(|l| l.adapter.manifest().files.contains(&file.kind))
        .filter_map(|l| {
            let m = l.adapter.manifest();
            let score = l.adapter.detect(file);
            (score > 0).then(|| Match {
                id: m.id.clone(),
                name: m.name.clone(),
                version: m.version.clone(),
                score,
                origin: l.origin,
            })
        })
        .collect();
    // Best score first; on a tie user mappings win (they were made for this kind of file), then id.
    matches.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then((b.origin == Origin::User).cmp(&(a.origin == Origin::User)))
            .then(a.id.cmp(&b.id))
    });
    if matches.is_empty() {
        Detection::Unrecognized
    } else {
        Detection::Matches(matches)
    }
}
