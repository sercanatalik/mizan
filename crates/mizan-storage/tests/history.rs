use std::ffi::OsString;
use std::path::PathBuf;

use mizan_storage::history::{resolve_home, CACHE_FILE};
use mizan_storage::*;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mizan-history-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn price(code: &str, date: &str, price: &str) -> CachedFundPrice {
    CachedFundPrice {
        code: code.into(),
        date: date.into(),
        price: price.into(),
    }
}

#[test]
fn home_defaults_to_dot_mizan_in_the_user_home() {
    let home = resolve_home(None, Some(PathBuf::from("/Users/a")));
    assert_eq!(home, Some(PathBuf::from("/Users/a/.mizan")));
}

#[test]
fn mizan_home_overrides_the_default() {
    let home = resolve_home(
        Some(OsString::from("/data/mizan-test")),
        Some(PathBuf::from("/Users/a")),
    );
    assert_eq!(home, Some(PathBuf::from("/data/mizan-test")));
}

#[test]
fn empty_mizan_home_is_ignored() {
    let home = resolve_home(Some(OsString::new()), Some(PathBuf::from("/Users/a")));
    assert_eq!(home, Some(PathBuf::from("/Users/a/.mizan")));
}

#[test]
fn open_creates_the_folder_and_schema_and_reopens() {
    let dir = temp_dir("open").join("nested");
    {
        let mut c = HistoryCache::open(&dir).unwrap();
        assert_eq!(c.schema_version().unwrap(), 3);
        assert_eq!(c.path().unwrap(), dir.join(CACHE_FILE));
        c.upsert_fund_price(&price("AFT", "2026-10-30", "0.959756"), "t")
            .unwrap();
    }
    assert!(dir.join(CACHE_FILE).exists());
    let c = HistoryCache::open(&dir).unwrap();
    assert_eq!(c.schema_version().unwrap(), 3);
    assert_eq!(c.fund_prices("AFT").unwrap().len(), 1);
    let tables: Vec<String> = c
        .schema_columns()
        .unwrap()
        .into_iter()
        .map(|(t, _)| t)
        .collect();
    assert_eq!(
        tables,
        [
            "fetched",
            "fund_prices",
            "fund_types",
            "fx_rates",
            "inflation_checks",
            "inflation_index",
            "listed_prices"
        ]
    );
}

#[test]
fn decimals_round_trip_as_text() {
    let mut c = HistoryCache::open_in_memory().unwrap();
    let long = "12345678901.234567890123";
    c.upsert_fund_price(&price("AFT", "2026-10-30", long), "t")
        .unwrap();
    c.upsert_rate(
        &CachedRate {
            currency: "JPY".into(),
            date: "2024-05-02".into(),
            try_per_unit: "0.207".into(),
            source: "tcmb".into(),
        },
        "t",
    )
    .unwrap();
    assert_eq!(c.fund_prices("AFT").unwrap()[0].value, long);
    assert_eq!(c.rates("JPY").unwrap()[0].value, "0.207");
}

#[test]
fn repeated_upsert_keeps_one_row_with_the_latest_value() {
    let mut c = HistoryCache::open_in_memory().unwrap();
    c.upsert_fund_price(&price("AFT", "2026-10-30", "1.20"), "t1")
        .unwrap();
    c.upsert_fund_price(&price("AFT", "2026-10-30", "1.21"), "t2")
        .unwrap();
    let rows = c.fund_prices("AFT").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].value, "1.21");
}

#[test]
fn newest_on_or_before() {
    let mut c = HistoryCache::open_in_memory().unwrap();
    for (d, p) in [
        ("2026-10-22", "1.10"),
        ("2026-10-23", "1.20"),
        ("2026-10-26", "1.30"),
    ] {
        c.upsert_fund_price(&price("AFT", d, p), "t").unwrap();
    }
    let sunday = c.fund_price_on("AFT", "2026-10-25").unwrap().unwrap();
    assert_eq!(
        (sunday.date.as_str(), sunday.value.as_str()),
        ("2026-10-23", "1.20")
    );
    assert!(c.fund_price_on("AFT", "2026-10-01").unwrap().is_none());
}

#[test]
fn committed_units_are_not_missing() {
    let mut c = HistoryCache::open_in_memory().unwrap();
    c.commit_unit(&FetchedUnit {
        source: "tefas".into(),
        key: "AFT".into(),
        period: "2022-01".into(),
        status: FetchStatus::Ok,
        fund_prices: vec![price("AFT", "2022-01-03", "0.5")],
        rates: vec![],
        fetched_at: "t".into(),
    })
    .unwrap();
    c.commit_unit(&FetchedUnit {
        source: "tcmb".into(),
        key: "*".into(),
        period: "2025-01-01".into(),
        status: FetchStatus::None,
        fund_prices: vec![],
        rates: vec![],
        fetched_at: "t".into(),
    })
    .unwrap();
    let planned: Vec<String> = ["2022-02", "2022-01", "2021-12"].map(String::from).to_vec();
    assert_eq!(
        c.missing("tefas", "AFT", &planned).unwrap(),
        ["2022-02", "2021-12"]
    );
    assert_eq!(
        c.fetched("tcmb", "*").unwrap().get("2025-01-01"),
        Some(&FetchStatus::None)
    );
    assert_eq!(c.fund_prices("AFT").unwrap()[0].value, "0.5");
}

#[test]
fn fund_types_are_remembered() {
    let mut c = HistoryCache::open_in_memory().unwrap();
    assert_eq!(c.fund_type("AH2").unwrap(), None);
    c.set_fund_type("AH2", "EMK").unwrap();
    c.set_fund_type("AH2", "EMK").unwrap();
    assert_eq!(c.fund_type("AH2").unwrap().as_deref(), Some("EMK"));
}

fn listed(symbol: &str, date: &str, close: &str) -> CachedListedPrice {
    CachedListedPrice {
        source: "yahoo".into(),
        symbol: symbol.into(),
        date: date.into(),
        close: close.into(),
        currency: "EUR".into(),
    }
}

#[test]
fn listed_prices_commit_years_together() {
    let mut c = HistoryCache::open_in_memory().unwrap();
    c.commit_listed(
        "yahoo",
        "VWCE.DE",
        &[
            ("2020".into(), FetchStatus::None),
            ("2021".into(), FetchStatus::Ok),
        ],
        &[
            listed("VWCE.DE", "2021-06-15", "98.12"),
            listed("VWCE.DE", "2021-06-14", "97.50"),
        ],
        "2026-11-02T10:00:00Z",
    )
    .unwrap();
    let rows = c.listed_prices("yahoo", "VWCE.DE").unwrap();
    assert_eq!(
        rows.iter()
            .map(|r| (r.date.as_str(), r.close.as_str()))
            .collect::<Vec<_>>(),
        [("2021-06-14", "97.50"), ("2021-06-15", "98.12")]
    );
    assert_eq!(rows[0].currency, "EUR");
    let done = c.fetched("yahoo", "VWCE.DE").unwrap();
    assert_eq!(done.get("2020"), Some(&FetchStatus::None));
    assert_eq!(done.get("2021"), Some(&FetchStatus::Ok));
    assert!(c.listed_prices("twelvedata", "VWCE.DE").unwrap().is_empty());

    // Re-fetching a year overwrites its closes, never duplicates.
    c.upsert_listed_price(&listed("VWCE.DE", "2021-06-15", "98.20"), "t2")
        .unwrap();
    assert_eq!(
        c.listed_prices("yahoo", "VWCE.DE").unwrap()[1].close,
        "98.20"
    );
}

fn idx(month: &str, value: &str) -> CachedIndexValue {
    CachedIndexValue {
        month: month.into(),
        value: value.into(),
    }
}

#[test]
fn inflation_values_and_checks_round_trip() {
    let mut c = HistoryCache::open_in_memory().unwrap();
    assert!(c.inflation_check("eurostat", "TR:I15").unwrap().is_none());
    c.commit_inflation(
        "eurostat",
        "TR:I15",
        &[idx("2026-09", "1675.96"), idx("2026-08", "1640.10")],
        true,
        "2026-11-02T10:00:00Z",
    )
    .unwrap();
    assert_eq!(
        c.inflation_index("eurostat", "TR:I15").unwrap(),
        [idx("2026-08", "1640.10"), idx("2026-09", "1675.96")]
    );
    assert!(c.inflation_index("eurostat", "EA:I15").unwrap().is_empty());
    let check = c.inflation_check("eurostat", "TR:I15").unwrap().unwrap();
    assert!(check.ok);
    assert_eq!(check.checked_at, "2026-11-02T10:00:00Z");

    // A failed request keeps the values and records the failure.
    c.commit_inflation("eurostat", "TR:I15", &[], false, "2026-11-03T10:00:00Z")
        .unwrap();
    assert_eq!(c.inflation_index("eurostat", "TR:I15").unwrap().len(), 2);
    let check = c.inflation_check("eurostat", "TR:I15").unwrap().unwrap();
    assert!(!check.ok);
    assert_eq!(check.checked_at, "2026-11-03T10:00:00Z");

    // Re-fetching a month overwrites it.
    c.commit_inflation(
        "eurostat",
        "TR:I15",
        &[idx("2026-09", "1676.00")],
        true,
        "2026-11-04T10:00:00Z",
    )
    .unwrap();
    let rows = c.inflation_index("eurostat", "TR:I15").unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].value, "1676.00");
}

#[test]
fn series_summaries_and_points_are_read_only() {
    let mut c = HistoryCache::open_in_memory().unwrap();
    // 2025-03-14 is a Friday, 2025-03-15 a Saturday.
    for (d, p) in [
        ("2025-03-13", "1.10"),
        ("2025-03-14", "1.20"),
        ("2025-03-15", "1.25"),
    ] {
        c.upsert_fund_price(&price("AFT", d, p), "f1").unwrap();
    }
    c.upsert_listed_price(&listed("VWCE.DE", "2025-03-14", "47.12"), "f2")
        .unwrap();
    c.upsert_rate(
        &CachedRate {
            currency: "USD".into(),
            date: "2025-04-01".into(),
            try_per_unit: "38.10".into(),
            source: "tcmb".into(),
        },
        "f3",
    )
    .unwrap();
    c.commit_inflation(
        "eurostat",
        "TR:I15",
        &[idx("2026-08", "1640.10"), idx("2026-09", "1675.96")],
        true,
        "f4",
    )
    .unwrap();
    let before = c.all_text_values().unwrap();

    let f = &c.fund_series().unwrap()[0];
    assert_eq!(
        (f.source.as_str(), f.id.as_str(), f.currency.as_deref()),
        ("tefas", "AFT", Some("TRY"))
    );
    assert_eq!(
        (
            f.count,
            f.first.as_str(),
            f.last.as_str(),
            f.latest.as_str()
        ),
        (3, "2025-03-13", "2025-03-15", "1.25")
    );
    assert_eq!(f.weekday_values, 2);
    let l = &c.listed_series().unwrap()[0];
    assert_eq!(
        (l.source.as_str(), l.id.as_str(), l.currency.as_deref()),
        ("yahoo", "VWCE.DE", Some("EUR"))
    );
    assert_eq!(c.rate_series().unwrap()[0].latest, "38.10");
    let i = &c.index_series().unwrap()[0];
    assert_eq!(
        (i.source.as_str(), i.id.as_str(), i.count, i.latest.as_str()),
        ("eurostat", "TR:I15", 2, "1675.96")
    );

    let pts = c.fund_points("AFT").unwrap();
    assert_eq!(pts.len(), 3);
    assert_eq!(
        (pts[0].period.as_str(), pts[0].fetched_at.as_str()),
        ("2025-03-13", "f1")
    );
    assert_eq!(
        c.listed_points("yahoo", "VWCE.DE").unwrap()[0]
            .currency
            .as_deref(),
        Some("EUR")
    );
    assert_eq!(c.rate_points("USD").unwrap()[0].value, "38.10");
    assert_eq!(
        c.index_points("eurostat", "TR:I15").unwrap()[1].period,
        "2026-09"
    );
    assert_eq!(
        c.all_text_values().unwrap(),
        before,
        "browsing writes nothing"
    );
}
