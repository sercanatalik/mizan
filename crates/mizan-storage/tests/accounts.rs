//! Account management and merges (account-management spec).

use mizan_import::ImportRecord;
use mizan_storage::*;
use serde_json::json;

fn rec(hint: &str, i: usize) -> ImportRecord {
    serde_json::from_value(json!({
        "source": {"adapter": "t", "adapter_version": "1", "file_sha256": "a".repeat(64), "locator": format!("row {i}")},
        "account": {"hint": hint},
        "kind": "deposit",
        "date": "2025-01-02",
        "net_cash": {"amount": "1.00", "currency": "TRY"}
    }))
    .unwrap()
}

fn add(s: &mut Store, account: AccountId, hint: &str, n: usize, file: char) {
    let batch = NewBatch {
        adapter: "t".into(),
        adapter_version: "1".into(),
        file_sha256: file.to_string().repeat(64),
        activities: (0..n)
            .map(|i| NewActivity {
                account_id: account,
                asset_id: None,
                fingerprint: format!("fp-{file}-{i}"),
                source_order: i as i64,
                basis_unknown: false,
                record: rec(hint, i),
            })
            .collect(),
        assertions: vec![],
    };
    s.commit_batch(&batch).unwrap();
}

fn count(s: &Store, account: AccountId) -> usize {
    s.all_activities(account).unwrap().len()
}

#[test]
fn schema_and_hidden_merged_accounts() {
    let mut s = Store::open_in_memory().unwrap();
    assert_eq!(s.schema_version().unwrap(), 9);
    let main = s
        .create_account("Anadolu Hayat Emeklilik", "9000001")
        .unwrap();
    let dk = s
        .create_account("Anadolu Hayat (Devlet Katkısı)", "9000001-DK")
        .unwrap();
    add(&mut s, main, "9000001", 3, 'a');
    add(&mut s, dk, "9000001-DK", 2, 'b');
    assert_eq!(
        s.merge_preview(dk).unwrap(),
        MergeCounts {
            activities: 2,
            assertions: 0,
            hints: 1
        }
    );

    let m = s.merge_accounts(dk, main, "2026-11-02T10:00:00Z").unwrap();
    assert_eq!(count(&s, main), 5);
    assert_eq!(count(&s, dk), 0);
    let ids: Vec<i64> = s.list_accounts().unwrap().iter().map(|a| a.id).collect();
    assert_eq!(ids, [main], "merged accounts are hidden");
    assert_eq!(s.list_all_accounts().unwrap().len(), 2);
    let main_row = s.list_accounts().unwrap().remove(0);
    assert_eq!(main_row.hints, ["9000001", "9000001-DK"]);
    assert_eq!(
        s.account_for_hint("9000001-DK").unwrap(),
        Some(main),
        "future imports go to the target"
    );
    let log = s.account_merges().unwrap();
    assert_eq!(
        (log[0].id, log[0].moved.activities, log[0].moved.hints),
        (m, 2, 1)
    );
}

#[test]
fn undo_after_a_later_import_keeps_new_activities_in_the_target() {
    let mut s = Store::open_in_memory().unwrap();
    let a = s.create_account("Old", "A1").unwrap();
    let b = s.create_account("New", "B1").unwrap();
    add(&mut s, a, "A1", 2, 'a');
    let m = s.merge_accounts(a, b, "t").unwrap();
    // A statement for A1 imported after the merge lands in B.
    add(&mut s, b, "A1", 1, 'c');
    assert_eq!(count(&s, b), 3);
    let skipped = s.undo_account_merge(m, "t2").unwrap();
    assert!(skipped.is_empty());
    assert_eq!((count(&s, a), count(&s, b)), (2, 1));
    assert_eq!(s.account_for_hint("A1").unwrap(), Some(a));
    assert_eq!(s.list_accounts().unwrap().len(), 2);
    assert!(s.undo_account_merge(m, "t3").is_err(), "already undone");
    // Undone merges can be merged again.
    s.merge_accounts(a, b, "t4").unwrap();
}

#[test]
fn merge_refusals() {
    let mut s = Store::open_in_memory().unwrap();
    let a = s.create_account("A", "A1").unwrap();
    let b = s.create_account("B", "B1").unwrap();
    let c = s.create_account("C", "C1").unwrap();
    assert!(s.merge_accounts(a, a, "t").is_err());
    assert!(s.merge_accounts(a, 99, "t").is_err());
    let ab = s.merge_accounts(a, b, "t").unwrap();
    let err = s.merge_accounts(a, c, "t").unwrap_err();
    assert!(err.to_string().contains("already merged"), "{err}");
    assert!(
        s.merge_accounts(c, a, "t").is_err(),
        "cannot merge into a merged account"
    );
    // B merged into C: A→B can't be undone until B→C is.
    let bc = s.merge_accounts(b, c, "t").unwrap();
    let err = s.undo_account_merge(ab, "t").unwrap_err();
    assert!(err.to_string().contains("undo that merge first"), "{err}");
    s.undo_account_merge(bc, "t").unwrap();
    s.undo_account_merge(ab, "t").unwrap();
}

#[test]
fn moved_identifier_is_reported_on_undo() {
    let mut s = Store::open_in_memory().unwrap();
    let a = s.create_account("A", "A1").unwrap();
    let b = s.create_account("B", "B1").unwrap();
    let c = s.create_account("C", "C1").unwrap();
    let m = s.merge_accounts(a, b, "t").unwrap();
    s.move_account_hint("A1", c).unwrap();
    assert_eq!(s.undo_account_merge(m, "t").unwrap(), ["A1"]);
    assert_eq!(s.account_for_hint("A1").unwrap(), Some(c));
}

#[test]
fn rename_identifiers_and_delete() {
    let mut s = Store::open_in_memory().unwrap();
    let a = s.create_account("DEG-4417", "D1").unwrap();
    let b = s.create_account("TR Bank", "T1").unwrap();
    s.rename_account(a, "  Degiro ").unwrap();
    assert_eq!(s.list_accounts().unwrap()[0].name, "Degiro");
    assert!(s.rename_account(a, "  ").is_err());
    assert!(s.rename_account(99, "x").is_err());

    s.add_account_hint(a, "U1234567").unwrap();
    s.add_account_hint(a, "U1234567").unwrap(); // already its own
    let err = s.add_account_hint(b, "U1234567").unwrap_err();
    assert!(
        err.to_string().contains("already belongs to Degiro"),
        "{err}"
    );
    s.move_account_hint("U1234567", b).unwrap();
    assert_eq!(s.account_for_hint("U1234567").unwrap(), Some(b));
    assert!(s.move_account_hint("nope", b).is_err());

    add(&mut s, a, "D1", 1, 'a');
    let why = s.delete_blocker(a).unwrap().unwrap();
    assert!(why.contains("1 activities"), "{why}");
    assert!(s.delete_account(a).is_err());
    assert_eq!(s.delete_blocker(b).unwrap(), None);
    s.delete_account(b).unwrap();
    assert_eq!(s.account_for_hint("T1").unwrap(), None);
    assert_eq!(s.list_all_accounts().unwrap().len(), 1);
    assert!(s.assertion_history(a).unwrap().is_empty());
}
