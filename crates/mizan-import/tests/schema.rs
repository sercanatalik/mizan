//! Fails when the committed JSON Schema drifts from the Rust types.
//! Regenerate with: `MIZAN_BLESS=1 cargo test -p mizan-import --test schema`

use std::path::PathBuf;

use mizan_import::schema::{batch_schema_json, SCHEMA_FILE};

#[test]
fn committed_schema_matches_generated() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").join(SCHEMA_FILE);
    let generated = batch_schema_json();
    if std::env::var_os("MIZAN_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &generated).unwrap();
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == generated,
        "{SCHEMA_FILE} is out of date. Regenerate it with:\n  MIZAN_BLESS=1 cargo test -p mizan-import --test schema"
    );
}
