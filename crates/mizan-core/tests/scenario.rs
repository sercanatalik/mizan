//! Full pipeline over two overlapping statements, compared with a committed golden report.
//! Regenerate with: `MIZAN_BLESS=1 cargo test -p mizan-core --test scenario`

use std::path::PathBuf;

use mizan_core::*;
use mizan_import::parse_batch;
use mizan_storage::Store;
use serde_json::json;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/scenario")
}

fn review_and_commit(store: &mut Store, file: &str) -> ReviewReport {
    let parsed = parse_batch(&std::fs::read_to_string(dir().join(file)).unwrap()).unwrap();
    let rb = {
        let mut r = MappingResolver {
            store,
            policy: CreatePolicy {
                create_accounts: true,
                create_assets: true,
            },
        };
        resolve_batch(parsed, &mut r).unwrap()
    };
    let prepared = prepare(store, rb).unwrap();
    let rep = report(store, &prepared, &Decisions::new()).unwrap();
    commit(store, &prepared, &Decisions::new()).unwrap();
    rep
}

#[test]
fn two_overlapping_statements_match_golden_report() {
    let mut store = Store::open_in_memory().unwrap();
    let q1 = review_and_commit(&mut store, "q1.json");
    let q2 = review_and_commit(&mut store, "q2.json");

    // Spot checks that make the golden file's meaning explicit.
    assert_eq!(q1.basis_unknown.len(), 1, "transfer-in without basis");
    assert!(q1.assertion_results.iter().all(|a| a.passed));
    assert_eq!(q2.summary.duplicates, 1, "overlapping March transfer-in");
    assert_eq!(q2.summary.new, 3, "spinoff, dividend, buy");
    assert!(
        q2.assertion_results.iter().all(|a| a.passed),
        "{:?}",
        q2.assertion_results
    );
    assert_eq!(store.count("activities").unwrap(), 7);

    let mut actual = serde_json::to_string_pretty(&json!({"q1": q1, "q2": q2})).unwrap();
    actual.push('\n');
    let golden = dir().join("golden.report.json");
    if std::env::var_os("MIZAN_BLESS").is_some() {
        std::fs::write(&golden, &actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&golden).unwrap_or_default();
    if expected != actual {
        let diff = similar::TextDiff::from_lines(&expected, &actual)
            .unified_diff()
            .header("golden.report.json", "actual")
            .to_string();
        panic!("review report differs from the golden file. If the change is intended, run:\n  MIZAN_BLESS=1 cargo test -p mizan-core --test scenario\n\n{diff}");
    }
}
