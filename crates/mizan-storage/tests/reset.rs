//! Backups and resets (data-reset spec).

use mizan_storage::*;

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mizan-reset-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn filled() -> Store {
    let mut s = Store::open_in_memory().unwrap();
    let a = s.create_account("IBKR", "U1").unwrap();
    s.create_asset("isin:US0378331005", &Default::default())
        .unwrap();
    s.set_setting("base_currency", "USD").unwrap();
    s.set_setting("provider.twelvedata.api_key", "td-key")
        .unwrap();
    s.set_series_value(SeriesKind::Rate, "USD", "2025-04-01", "38.10", None, "t")
        .unwrap();
    let _ = a;
    s
}

#[test]
fn backup_holds_the_same_rows_and_refuses_a_missing_folder() {
    let s = filled();
    let dir = temp_dir("backup");
    let path = dir.join("mizan-backup.db");
    s.backup_to(&path).unwrap();
    let b = Store::open(&path).unwrap();
    assert_eq!(b.list_accounts().unwrap().len(), 1);
    assert_eq!(b.setting("base_currency").unwrap().as_deref(), Some("USD"));
    assert!(s.backup_to(&path).is_err(), "never overwrites");
    assert!(s.backup_to(&dir.join("missing").join("x.db")).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn portfolio_reset_keeps_settings_and_keys() {
    let mut s = filled();
    let version = s.schema_version().unwrap();
    s.reset(ResetScope::Portfolio).unwrap();
    assert!(s.list_accounts().unwrap().is_empty());
    assert!(s.list_assets().unwrap().is_empty());
    assert!(s.all_series_edits().unwrap().is_empty());
    assert_eq!(s.setting("base_currency").unwrap().as_deref(), Some("USD"));
    assert_eq!(
        s.setting("provider.twelvedata.api_key").unwrap().as_deref(),
        Some("td-key")
    );
    assert_eq!(s.schema_version().unwrap(), version);
    // The database still works.
    s.create_account("New", "N1").unwrap();
}

#[test]
fn everything_restores_default_settings() {
    let mut s = filled();
    s.reset(ResetScope::Everything).unwrap();
    assert!(s.list_accounts().unwrap().is_empty());
    assert_eq!(s.setting("base_currency").unwrap().as_deref(), Some("TRY"));
    assert_eq!(s.setting("provider.twelvedata.api_key").unwrap(), None);
    assert_eq!(
        s.setting("refresh_on_start").unwrap().as_deref(),
        Some("true")
    );
    assert_eq!(
        s.table_count("settings").unwrap(),
        DEFAULT_SETTINGS.len() as i64
    );
}

#[test]
fn cache_backup_and_clear() {
    let dir = temp_dir("cache");
    let mut c = HistoryCache::open(&dir).unwrap();
    c.upsert_fund_price(
        &CachedFundPrice {
            code: "AFT".into(),
            date: "2025-01-02".into(),
            price: "1.0".into(),
        },
        "t",
    )
    .unwrap();
    assert_eq!(c.value_count().unwrap(), 1);
    let backup = dir.join("market-history-backup.sqlite");
    c.backup_to(&backup).unwrap();
    c.clear().unwrap();
    assert_eq!(c.value_count().unwrap(), 0);
    assert_eq!(c.schema_version().unwrap(), 3);
    let b = HistoryCache::open(&dir).unwrap();
    assert_eq!(
        b.fund_prices("AFT").unwrap().len(),
        0,
        "the live cache is empty"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
