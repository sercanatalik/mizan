//! JSON Schema for `mizan.import/1`, generated from the Rust types.

use crate::record::Batch;

/// Path of the committed schema file, relative to the workspace root.
pub const SCHEMA_FILE: &str = "schema/mizan.import.1.schema.json";

/// The JSON Schema document for a batch, pretty-printed with a trailing newline.
pub fn batch_schema_json() -> String {
    let mut schema = schemars::schema_for!(Batch);
    let meta = schema.schema.metadata();
    meta.title = Some("mizan.import/1 batch".into());
    meta.description = Some(
        "Output of one Mizan import adapter run: records, balance assertions and diagnostics."
            .into(),
    );
    let mut s = serde_json::to_string_pretty(&schema).expect("schema serializes");
    s.push('\n');
    s
}
