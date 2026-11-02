//! The examples in docs/adapters.md load and run with the real runtime.

use std::path::Path;

use mizan_adapters::input::TableOptions;
use mizan_adapters::script::ScriptAdapter;
use mizan_adapters::{run_and_parse, AdapterKind, FileInput, FileKind, Manifest, Registry};

fn docs() -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/adapters.md"))
        .unwrap()
}

/// The fenced block following an HTML comment marker.
fn block(text: &str, marker: &str) -> String {
    let start = text
        .find(&format!("<!-- {marker} -->"))
        .unwrap_or_else(|| panic!("marker {marker} missing"));
    let rest = &text[start..];
    let open = rest.find("```").unwrap();
    let body = &rest[open..];
    let body = &body[body.find('\n').unwrap() + 1..];
    body[..body.find("```").unwrap()].to_string()
}

#[test]
fn mapping_example_loads_and_reads_its_sample() {
    let text = docs();
    let dir = std::env::temp_dir().join(format!("mizan-docs-mapping-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("example.toml"), block(&text, "mapping-example")).unwrap();
    let reg = Registry::load(Some(&dir));
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(
        reg.errors.is_empty(),
        "{:?}",
        reg.errors.iter().map(|e| &e.message).collect::<Vec<_>>()
    );

    let file = FileInput::new(
        "transactions.csv",
        block(&text, "mapping-example-input").into_bytes(),
    );
    let a = reg.get("example-broker-csv").unwrap();
    assert_eq!(a.adapter.detect(&file), 90);
    let out = mizan_adapters::run_adapter(a.adapter.as_ref(), &file).unwrap();
    let parsed = run_and_parse(a.adapter.as_ref(), &file).unwrap().unwrap();
    assert!(parsed.is_clean(), "{:?}", parsed.errors);
    let r = &out["records"];
    assert_eq!(
        (r[0]["kind"].as_str(), r[0]["quantity"].as_str()),
        (Some("buy"), Some("10"))
    );
    assert_eq!(r[0]["net_cash"]["amount"], "985.00");
    assert_eq!(r[1]["kind"], "dividend");
    assert_eq!(r.as_array().unwrap().len(), 2, "the Saldo row is skipped");
}

#[test]
fn script_example_runs() {
    let text = docs();
    let manifest = Manifest {
        id: "docs-script".into(),
        version: "1.0.0".into(),
        name: "Docs".into(),
        kind: AdapterKind::Script,
        files: vec![FileKind::Csv],
        native: None,
        script: Some("script.ts".into()),
        description: None,
    };
    let a = ScriptAdapter::new(
        manifest,
        &block(&text, "script-example"),
        TableOptions::default(),
    )
    .unwrap();
    let file = FileInput::new(
        "x.csv",
        b"Trade Date,Action,Amount\n10/02/2026,Deposit,\"1,500.00\"\n".to_vec(),
    );
    use mizan_adapters::Adapter;
    assert_eq!(a.detect(&file), 80);
    let parsed = run_and_parse(&a, &file).unwrap().unwrap();
    assert!(parsed.is_clean(), "{:?}", parsed.errors);
    assert_eq!(
        parsed.records[0]
            .item
            .net_cash
            .as_ref()
            .unwrap()
            .amount
            .to_string(),
        "1500.00"
    );

    let bad = FileInput::new(
        "x.csv",
        b"Trade Date,Action,Amount\n10/02/2026,Buy,1\n".to_vec(),
    );
    let err = run_and_parse(&a, &bad).err().unwrap();
    assert_eq!(err.locator.as_deref(), Some("row 2"));
}
