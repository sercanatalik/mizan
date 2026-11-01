use mizan_import::{Assertion, ImportRecord, InstrumentHint};
use mizan_storage::*;
use serde_json::json;

fn sha() -> String {
    format!("{}{}", "ab", "0".repeat(62))
}

fn record(kind: &str, date: &str, locator: &str) -> ImportRecord {
    serde_json::from_value(json!({
        "source": {"adapter": "test", "adapter_version": "1", "file_sha256": sha(), "locator": locator},
        "external_id": format!("ext-{locator}"),
        "account": {"hint": "U1"},
        "kind": kind,
        "date": date,
        "instrument": {"isin": "US0378331005"},
        "quantity": "10",
        "net_cash": {"amount": "1501.00", "currency": "USD"}
    }))
    .unwrap()
}

fn assertion() -> Assertion {
    serde_json::from_value(json!({
        "source": {"adapter": "test", "adapter_version": "1", "file_sha256": sha(), "locator": "page 2"},
        "account": {"hint": "U1"}, "as_of": "2026-03-31",
        "cash": [{"amount": "1520.33", "currency": "USD"}]
    }))
    .unwrap()
}

fn new_batch(account_id: AccountId, asset_id: AssetId) -> NewBatch {
    NewBatch {
        adapter: "test".into(),
        adapter_version: "1".into(),
        file_sha256: sha(),
        activities: vec![
            NewActivity {
                account_id,
                asset_id: Some(asset_id),
                fingerprint: "fp-a".into(),
                source_order: 0,
                basis_unknown: false,
                record: record("buy", "2026-03-02", "row 2"),
            },
            NewActivity {
                account_id,
                asset_id: Some(asset_id),
                fingerprint: "fp-b".into(),
                source_order: 1,
                basis_unknown: true,
                record: record("position_in", "2026-03-20", "row 3"),
            },
        ],
        assertions: vec![NewAssertion {
            account_id,
            assertion: assertion(),
            result: Some(AssertionResult { passed: true, detail: json!({"lines": []}) }),
        }],
    }
}

#[test]
fn migrations_apply_on_empty_database() {
    let store = Store::open_in_memory().unwrap();
    assert_eq!(store.schema_version().unwrap(), 1);
    for t in ["accounts", "account_hints", "assets", "import_batches", "activities", "assertions", "assertion_results"] {
        assert_eq!(store.count(t).unwrap(), 0, "{t}");
    }
}

#[test]
fn reopening_a_file_database_does_not_rerun_migrations() {
    let dir = std::env::temp_dir().join(format!("mizan-store-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("m.db");
    let _ = std::fs::remove_file(&path);
    {
        let mut s = Store::open(&path).unwrap();
        s.create_account("IBKR", "U1").unwrap();
    }
    let s = Store::open(&path).unwrap();
    assert_eq!(s.schema_version().unwrap(), 1);
    assert_eq!(s.count("accounts").unwrap(), 1);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn accounts_and_assets_by_hint() {
    let mut s = Store::open_in_memory().unwrap();
    assert_eq!(s.account_for_hint("U1").unwrap(), None);
    let a = s.create_account("Interactive Brokers", "U1").unwrap();
    assert_eq!(s.account_for_hint("U1").unwrap(), Some(a));
    s.map_account_hint("IBKR-U1", a).unwrap();
    assert_eq!(s.account_for_hint("IBKR-U1").unwrap(), Some(a));

    let hint = InstrumentHint { isin: Some("US0378331005".into()), ..Default::default() };
    let x = s.create_asset("isin:US0378331005", &hint).unwrap();
    assert_eq!(s.asset_for_key("isin:US0378331005").unwrap(), Some(x));
    assert_eq!(s.asset_for_key("isin:XX").unwrap(), None);
}

#[test]
fn committed_batch_round_trips() {
    let mut s = Store::open_in_memory().unwrap();
    let acct = s.create_account("IBKR", "U1").unwrap();
    let hint = InstrumentHint { isin: Some("US0378331005".into()), ..Default::default() };
    let asset = s.create_asset("isin:US0378331005", &hint).unwrap();

    let batch = new_batch(acct, asset);
    let id = s.commit_batch(&batch).unwrap();
    assert_eq!(s.count("import_batches").unwrap(), 1);
    assert_eq!(s.count("assertions").unwrap(), 1);
    assert_eq!(s.count("assertion_results").unwrap(), 1);

    let all = s.activities_until(acct, "2026-12-31").unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].batch_id, id);
    assert_eq!(all[0].record, batch.activities[0].record, "record stored exactly as imported");
    assert!(all[1].basis_unknown);

    let march_first_half = s.activities_between(acct, "2026-03-01", "2026-03-15").unwrap();
    assert_eq!(march_first_half.len(), 1);
    assert_eq!(march_first_half[0].record.source.locator, "row 2");

    let by_fp = s.activities_by_fingerprint("fp-b").unwrap();
    assert_eq!(by_fp.len(), 1);
    assert_eq!(by_fp[0].record.kind.as_str(), "position_in");
}

#[test]
fn failed_commit_leaves_no_partial_rows() {
    let mut s = Store::open_in_memory().unwrap();
    let acct = s.create_account("IBKR", "U1").unwrap();
    let hint = InstrumentHint { isin: Some("US0378331005".into()), ..Default::default() };
    let asset = s.create_asset("isin:US0378331005", &hint).unwrap();

    let mut batch = new_batch(acct, asset);
    // Second activity references an account that does not exist: the insert fails mid-batch.
    batch.activities[1].account_id = 999;
    assert!(s.commit_batch(&batch).is_err());

    for t in ["import_batches", "activities", "assertions", "assertion_results"] {
        assert_eq!(s.count(t).unwrap(), 0, "{t} must be empty after a failed commit");
    }
}
