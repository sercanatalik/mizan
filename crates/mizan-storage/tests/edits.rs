//! Per-database series edits (market-data-edits spec).

use mizan_storage::*;

#[test]
fn edit_ignore_and_revert() {
    let mut s = Store::open_in_memory().unwrap();
    assert_eq!(s.schema_version().unwrap(), 9);
    s.set_series_value(
        SeriesKind::Price,
        "yahoo:VWCE.DE",
        "2025-03-14",
        "47.12",
        Some("EUR"),
        "t1",
    )
    .unwrap();
    s.set_series_value(
        SeriesKind::Price,
        "yahoo:VWCE.DE",
        "2025-03-14",
        "47.20",
        Some("EUR"),
        "t2",
    )
    .unwrap();
    let e = s.series_edits(SeriesKind::Price, "yahoo:VWCE.DE").unwrap();
    assert_eq!(e.len(), 1, "upsert replaces");
    assert_eq!(
        (e[0].value.as_deref(), e[0].edited_at.as_str(), e[0].ignored),
        (Some("47.20"), "t2", false)
    );

    s.ignore_series_value(SeriesKind::Price, "yahoo:VWCE.DE", "2025-03-14", "t3")
        .unwrap();
    let e = &s.series_edits(SeriesKind::Price, "yahoo:VWCE.DE").unwrap()[0];
    assert!(e.ignored && e.value.is_none() && e.currency.is_none());

    s.set_series_value(SeriesKind::Rate, "USD", "2025-04-01", "38.10", None, "t")
        .unwrap();
    s.set_series_value(
        SeriesKind::Index,
        "eurostat:TR:I15",
        "2026-08",
        "1640.10",
        None,
        "t",
    )
    .unwrap();
    assert_eq!(s.all_series_edits().unwrap().len(), 3);
    assert!(s.series_edits(SeriesKind::Rate, "EUR").unwrap().is_empty());

    assert!(s
        .revert_series_value(SeriesKind::Rate, "USD", "2025-04-01")
        .unwrap());
    assert!(!s
        .revert_series_value(SeriesKind::Rate, "USD", "2025-04-01")
        .unwrap());
    assert_eq!(s.all_series_edits().unwrap().len(), 2);
    assert!(s
        .set_series_value(SeriesKind::Rate, " ", "2025-04-01", "1", None, "t")
        .is_err());
    assert_eq!(SeriesKind::parse("index"), Some(SeriesKind::Index));
}
