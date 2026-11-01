//! Fixture harness, exercised with a minimal test-only CSV adapter.

use std::path::{Path, PathBuf};

use mizan_core::fixture::{check_all, check_case, discover, AdapterInput, FixtureCase};
use mizan_import::Kind;
use serde_json::{json, Map, Value};

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Columns: id,account,date,kind,isin,quantity,amount,currency. No quoting.
fn sample_csv(input: &AdapterInput) -> Result<Value, String> {
    let text = std::str::from_utf8(input.bytes).map_err(|e| e.to_string())?;
    let mut records = Vec::new();
    let mut diagnostics = Vec::new();
    for (i, line) in text.lines().enumerate().skip(1) {
        let row = i + 1;
        let c: Vec<&str> = line.split(',').collect();
        if c.len() != 8 {
            return Err(format!("row {row}: expected 8 columns"));
        }
        if serde_json::from_value::<Kind>(json!(c[3])).is_err() {
            diagnostics.push(json!({
                "severity": "warning", "message": format!("unknown transaction kind {:?}", c[3]), "locator": format!("row {row}")
            }));
            continue;
        }
        let mut r = Map::new();
        r.insert(
            "source".into(),
            json!({"adapter": "sample-csv", "adapter_version": "0.1.0", "file_sha256": input.file_sha256, "locator": format!("row {row}")}),
        );
        if !c[0].is_empty() {
            r.insert("external_id".into(), json!(c[0]));
        }
        r.insert("account".into(), json!({"hint": c[1]}));
        r.insert("kind".into(), json!(c[3]));
        r.insert("date".into(), json!(c[2]));
        if !c[4].is_empty() {
            r.insert("instrument".into(), json!({"isin": c[4]}));
        }
        if !c[5].is_empty() {
            r.insert("quantity".into(), json!(c[5]));
        }
        r.insert("net_cash".into(), json!({"amount": c[6], "currency": c[7]}));
        records.push(Value::Object(r));
    }
    let mut batch = json!({"schema": "mizan.import/1", "records": records});
    if !diagnostics.is_empty() {
        batch["diagnostics"] = json!(diagnostics);
    }
    Ok(batch)
}

#[test]
fn discovers_cases() {
    let cases = discover(&fixtures_root()).unwrap();
    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0].name, "sample-csv/basic");
    assert!(cases[0].input.ends_with("input.csv"));
}

#[test]
fn sample_fixture_passes() {
    assert_eq!(check_all(&fixtures_root(), sample_csv), Ok(1));
}

fn temp_copy(case: &FixtureCase, tag: &str) -> FixtureCase {
    let dir = std::env::temp_dir().join(format!("mizan-fixture-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("input.csv");
    let expected = dir.join("expected.json");
    std::fs::copy(&case.input, &input).unwrap();
    std::fs::copy(&case.expected, &expected).unwrap();
    FixtureCase {
        name: format!("tmp/{tag}"),
        input,
        expected,
    }
}

#[test]
fn modified_fixture_fails_with_readable_diff() {
    let case = &discover(&fixtures_root()).unwrap()[0];
    let tmp = temp_copy(case, "diff");
    let text = std::fs::read_to_string(&tmp.expected)
        .unwrap()
        .replace("\"quantity\": \"10\"", "\"quantity\": \"12\"");
    std::fs::write(&tmp.expected, text).unwrap();

    let err = check_case(&tmp, sample_csv).unwrap_err();
    std::fs::remove_dir_all(tmp.input.parent().unwrap()).unwrap();
    assert_eq!(err.case, "tmp/diff");
    assert!(err.reason.contains("--- expected.json"), "{}", err.reason);
    assert!(err.reason.contains("+++ adapter output"), "{}", err.reason);
    assert!(
        err.reason.contains("-      \"quantity\": \"12\""),
        "{}",
        err.reason
    );
    assert!(
        err.reason.contains("+      \"quantity\": \"10\""),
        "{}",
        err.reason
    );
}

#[test]
fn invalid_adapter_output_fails() {
    let case = &discover(&fixtures_root()).unwrap()[0];
    let err = check_case(case, |i: &AdapterInput| {
        let mut v = sample_csv(i)?;
        v["records"][0]["quantity"] = json!(10.0);
        Ok(v)
    })
    .unwrap_err();
    assert!(err.reason.contains("invalid items"), "{}", err.reason);
}
