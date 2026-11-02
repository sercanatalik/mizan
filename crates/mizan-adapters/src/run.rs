//! Running an adapter and adding provenance.

use mizan_import::{parse_batch_value, BatchError, ParsedBatch, SCHEMA_V1};
use serde_json::{Map, Value};

use crate::{Adapter, FileInput};

/// What an adapter returns: records and assertions without `source`, each
/// carrying a `locator` string; diagnostics as in the schema.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunOutput {
    pub records: Vec<Value>,
    pub assertions: Vec<Value>,
    pub diagnostics: Vec<Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("adapter {adapter} failed{}: {message}", locator.as_ref().map(|l| format!(" at {l}")).unwrap_or_default())]
pub struct AdapterError {
    pub adapter: String,
    pub message: String,
    pub locator: Option<String>,
}

impl AdapterError {
    pub fn new(adapter: &str, message: impl Into<String>) -> Self {
        AdapterError {
            adapter: adapter.into(),
            message: message.into(),
            locator: None,
        }
    }

    pub fn at(mut self, locator: impl Into<String>) -> Self {
        self.locator = Some(locator.into());
        self
    }
}

fn with_source(
    item: Value,
    adapter: &str,
    version: &str,
    sha: &str,
    what: &str,
) -> Result<Value, AdapterError> {
    let Value::Object(mut obj) = item else {
        return Err(AdapterError::new(
            adapter,
            format!("{what} must be an object"),
        ));
    };
    let locator = match obj.remove("locator") {
        Some(Value::String(l)) if !l.trim().is_empty() => l,
        _ => {
            return Err(AdapterError::new(
                adapter,
                format!("every {what} needs a non-empty `locator`"),
            ))
        }
    };
    if obj.contains_key("source") {
        return Err(AdapterError::new(
            adapter,
            format!("{what} must not set `source`; Mizan adds it"),
        )
        .at(locator));
    }
    let mut source = Map::new();
    source.insert("adapter".into(), Value::String(adapter.into()));
    source.insert("adapter_version".into(), Value::String(version.into()));
    source.insert("file_sha256".into(), Value::String(sha.into()));
    source.insert("locator".into(), Value::String(locator));
    obj.insert("source".into(), Value::Object(source));
    Ok(Value::Object(obj))
}

/// Run `adapter` on `file` and build a `mizan.import/1` batch with provenance.
pub fn run_adapter(adapter: &dyn Adapter, file: &FileInput) -> Result<Value, AdapterError> {
    let m = adapter.manifest();
    let out = adapter.run(file)?;
    let records = out
        .records
        .into_iter()
        .map(|r| with_source(r, &m.id, &m.version, &file.sha256, "record"))
        .collect::<Result<Vec<_>, _>>()?;
    let assertions = out
        .assertions
        .into_iter()
        .map(|a| with_source(a, &m.id, &m.version, &file.sha256, "assertion"))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(serde_json::json!({
        "schema": SCHEMA_V1,
        "records": records,
        "assertions": assertions,
        "diagnostics": out.diagnostics,
    }))
}

/// Run and validate: invalid items are reported in the parsed batch, not as an error.
pub fn run_and_parse(
    adapter: &dyn Adapter,
    file: &FileInput,
) -> Result<Result<ParsedBatch, BatchError>, AdapterError> {
    Ok(parse_batch_value(run_adapter(adapter, file)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AdapterKind, FileKind, Manifest};
    use serde_json::json;

    struct Stub(Manifest, RunOutput);

    impl Adapter for Stub {
        fn manifest(&self) -> &Manifest {
            &self.0
        }
        fn detect(&self, _: &FileInput) -> u8 {
            100
        }
        fn run(&self, _: &FileInput) -> Result<RunOutput, AdapterError> {
            Ok(self.1.clone())
        }
    }

    fn stub(out: RunOutput) -> Stub {
        let m = Manifest {
            id: "degiro-csv".into(),
            version: "0.4.0".into(),
            name: "Degiro".into(),
            kind: AdapterKind::Mapping,
            files: vec![FileKind::Csv],
            native: None,
            script: None,
            description: None,
        };
        Stub(m, out)
    }

    #[test]
    fn host_fills_provenance_and_output_validates() {
        let file = FileInput::new("statement.csv", b"date,amount\n".to_vec());
        let out = RunOutput {
            records: vec![
                json!({"locator": "row 2", "account": {"hint": "D1"}, "kind": "deposit", "date": "2026-03-01",
                                 "net_cash": {"amount": "100.00", "currency": "EUR"}}),
            ],
            assertions: vec![
                json!({"locator": "summary", "account": {"hint": "D1"}, "as_of": "2026-03-31",
                                    "cash": [{"amount": "100.00", "currency": "EUR"}]}),
            ],
            diagnostics: vec![
                json!({"severity": "info", "message": "skipped header", "locator": "row 1"}),
            ],
        };
        let parsed = run_and_parse(&stub(out), &file).unwrap().unwrap();
        assert!(parsed.is_clean(), "{:?}", parsed.errors);
        let src = &parsed.records[0].item.source;
        assert_eq!(
            (
                src.adapter.as_str(),
                src.adapter_version.as_str(),
                src.locator.as_str()
            ),
            ("degiro-csv", "0.4.0", "row 2")
        );
        assert_eq!(src.file_sha256.as_str(), file.sha256);
        assert_eq!(parsed.assertions[0].item.source.locator, "summary");
        assert_eq!(parsed.diagnostics.len(), 1);
    }

    #[test]
    fn adapters_cannot_set_source_or_omit_locators() {
        let file = FileInput::new("s.csv", vec![]);
        let err = run_adapter(
            &stub(RunOutput {
                records: vec![json!({"kind": "deposit"})],
                ..Default::default()
            }),
            &file,
        )
        .unwrap_err();
        assert!(err.message.contains("locator"), "{err}");
        let err = run_adapter(
            &stub(RunOutput {
                records: vec![json!({"locator": "row 1", "source": {"adapter": "fake"}})],
                ..Default::default()
            }),
            &file,
        )
        .unwrap_err();
        assert!(err.message.contains("must not set `source`"), "{err}");
        assert_eq!(
            err.to_string(),
            "adapter degiro-csv failed at row 1: record must not set `source`; Mizan adds it"
        );
    }

    #[test]
    fn invalid_records_are_reported_not_fatal() {
        let file = FileInput::new("s.csv", vec![]);
        let out = RunOutput {
            records: vec![json!({"locator": "row 2", "kind": "nonsense"})],
            ..Default::default()
        };
        let parsed = run_and_parse(&stub(out), &file).unwrap().unwrap();
        assert_eq!(parsed.errors.len(), 1);
        assert_eq!(parsed.errors[0].locator.as_deref(), Some("row 2"));
    }

    #[test]
    fn sniffs_kinds() {
        use crate::input::sniff_kind;
        assert_eq!(sniff_kind("a.bin", b"%PDF-1.7"), FileKind::Pdf);
        assert_eq!(sniff_kind("a.bin", b"PK\x03\x04rest"), FileKind::Xlsx);
        assert_eq!(
            sniff_kind("a.ofx", b"OFXHEADER:100\nDATA:OFXSGML"),
            FileKind::Ofx
        );
        assert_eq!(
            sniff_kind(
                "a.qfx",
                b"<?xml version=\"1.0\"?><?OFX OFXHEADER=\"200\"?><OFX>"
            ),
            FileKind::Ofx
        );
        assert_eq!(
            sniff_kind("flex.xml", b"<?xml version=\"1.0\"?><FlexQueryResponse>"),
            FileKind::Xml
        );
        assert_eq!(sniff_kind("x.csv", b"a;b\n1;2"), FileKind::Csv);
        assert_eq!(sniff_kind("notes", b"hello"), FileKind::Text);
    }
}
