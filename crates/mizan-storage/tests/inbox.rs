use mizan_storage::{Snooze, Store};

fn live(items: &[(&str, &str)]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|(k, f)| (k.to_string(), f.to_string()))
        .collect()
}

#[test]
fn snooze_list_and_unsnooze() {
    let mut s = Store::open_in_memory().unwrap();
    s.snooze("check:1", "abc", "2026-11-09T10:00:00Z").unwrap();
    s.snooze("price:7", "def", "2026-11-03T10:00:00Z").unwrap();
    assert_eq!(s.snoozes().unwrap().len(), 2);
    s.snooze("check:1", "abc", "2026-11-10T10:00:00Z").unwrap();
    assert_eq!(
        s.snoozes().unwrap()[0],
        Snooze {
            key: "check:1".into(),
            fingerprint: "abc".into(),
            until: "2026-11-10T10:00:00Z".into()
        }
    );
    s.unsnooze("check:1").unwrap();
    assert_eq!(s.snoozes().unwrap().len(), 1);
}

#[test]
fn purge_drops_expired_resolved_and_changed() {
    let mut s = Store::open_in_memory().unwrap();
    s.snooze("expired", "f", "2026-11-01T00:00:00Z").unwrap();
    s.snooze("resolved", "f", "2026-12-01T00:00:00Z").unwrap();
    s.snooze("changed", "old", "2026-12-01T00:00:00Z").unwrap();
    s.snooze("kept", "f", "2026-12-01T00:00:00Z").unwrap();
    let now = "2026-11-02T10:00:00Z";
    let kept = s
        .purge_snoozes(
            now,
            &live(&[("expired", "f"), ("changed", "new"), ("kept", "f")]),
        )
        .unwrap();
    assert_eq!(
        kept.iter().map(|k| k.key.as_str()).collect::<Vec<_>>(),
        ["kept"]
    );
    assert_eq!(s.snoozes().unwrap().len(), 1, "the others are deleted");
}

#[test]
fn reset_clears_snoozes() {
    let mut s = Store::open_in_memory().unwrap();
    s.snooze("check:1", "abc", "2026-11-09T10:00:00Z").unwrap();
    s.reset(mizan_storage::ResetScope::Portfolio).unwrap();
    assert!(s.snoozes().unwrap().is_empty());
}
