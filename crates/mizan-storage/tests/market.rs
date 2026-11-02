use mizan_import::InstrumentHint;
use mizan_storage::*;

fn asset(s: &mut Store, key: &str) -> AssetId {
    s.create_asset(
        key,
        &InstrumentHint {
            name: Some(key.into()),
            ..Default::default()
        },
    )
    .unwrap()
}

fn tefas_price(asset_id: AssetId, date: &str, value: &str) -> PriceRow {
    PriceRow {
        asset_id,
        date: date.into(),
        source: "tefas".into(),
        value: value.into(),
        currency: "TRY".into(),
        fetched_at: "2026-11-01T10:00:00Z".into(),
    }
}

#[test]
fn migration_0002_applies_on_a_database_created_before_it() {
    let dir = std::env::temp_dir().join(format!("mizan-market-mig-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("old.db");
    let _ = std::fs::remove_file(&path);
    {
        // A database as the previous version left it: only 0001 applied, with data.
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(include_str!("../migrations/0001_init.sql"))
            .unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        conn.execute("INSERT INTO accounts (name) VALUES ('IBKR')", [])
            .unwrap();
    }
    let s = Store::open(&path).unwrap();
    assert_eq!(s.schema_version().unwrap(), 9);
    assert_eq!(s.count("accounts").unwrap(), 1, "existing data kept");
    assert_eq!(s.setting("base_currency").unwrap().as_deref(), Some("TRY"));
    assert_eq!(
        s.setting("refresh_on_start").unwrap().as_deref(),
        Some("true")
    );
    assert_eq!(
        s.setting("provider.tefas.enabled").unwrap().as_deref(),
        Some("true")
    );
    assert_eq!(
        s.setting("provider.tcmb.enabled").unwrap().as_deref(),
        Some("true")
    );
    for t in [
        "asset_merges",
        "price_sources",
        "prices",
        "fx_rates",
        "refresh_log",
    ] {
        assert_eq!(s.count(t).unwrap(), 0, "{t}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn settings_round_trip() {
    let mut s = Store::open_in_memory().unwrap();
    assert_eq!(s.setting("missing").unwrap(), None);
    s.set_setting("base_currency", "EUR").unwrap();
    assert_eq!(s.setting("base_currency").unwrap().as_deref(), Some("EUR"));
}

#[test]
fn price_sources_set_clear_and_keep_prices() {
    let mut s = Store::open_in_memory().unwrap();
    let a = asset(&mut s, "sym:AFT@");
    assert_eq!(s.price_source(a).unwrap(), None);
    let tefas = PriceSourceKind::Tefas {
        fund_code: "AFT".into(),
    };
    s.set_price_source(a, Some(&tefas)).unwrap();
    assert_eq!(s.price_source(a).unwrap(), Some(tefas));
    s.upsert_provider_price(&tefas_price(a, "2026-10-30", "0.959756"))
        .unwrap();

    s.set_price_source(a, Some(&PriceSourceKind::Manual))
        .unwrap();
    assert_eq!(s.price_source(a).unwrap(), Some(PriceSourceKind::Manual));
    assert_eq!(
        s.prices_for(a).unwrap().len(),
        1,
        "switching source keeps stored prices"
    );

    s.set_price_source(a, None).unwrap();
    assert_eq!(s.price_source(a).unwrap(), None);
    assert!(s.price_sources().unwrap().is_empty());
}

#[test]
fn prices_latest_with_manual_tie_break_and_provider_never_touches_manual() {
    let mut s = Store::open_in_memory().unwrap();
    let a = asset(&mut s, "sym:AFT@");
    s.upsert_provider_price(&tefas_price(a, "2026-10-28", "0.955956"))
        .unwrap();
    s.upsert_provider_price(&tefas_price(a, "2026-10-30", "1.20"))
        .unwrap();
    assert_eq!(
        s.latest_price(a, "2026-11-01").unwrap().unwrap().value,
        "1.20"
    );
    assert_eq!(
        s.latest_price(a, "2026-10-29").unwrap().unwrap().date,
        "2026-10-28",
        "on or before"
    );
    assert!(s.latest_price(a, "2026-10-01").unwrap().is_none());

    s.add_manual_price(a, "2026-10-30", "1.25", "TRY", "2026-11-01T11:00:00Z")
        .unwrap();
    let latest = s.latest_price(a, "2026-11-01").unwrap().unwrap();
    assert_eq!(
        (latest.value.as_str(), latest.source.as_str()),
        ("1.25", "manual"),
        "manual wins the tie"
    );

    // A provider re-fetch updates its own row only.
    s.upsert_provider_price(&tefas_price(a, "2026-10-30", "1.21"))
        .unwrap();
    let rows = s.prices_for(a).unwrap();
    assert_eq!(
        rows.iter().find(|r| r.source == "manual").unwrap().value,
        "1.25"
    );
    assert_eq!(
        rows.iter()
            .find(|r| r.source == "tefas" && r.date == "2026-10-30")
            .unwrap()
            .value,
        "1.21"
    );
    let mut bad = tefas_price(a, "2026-10-30", "9");
    bad.source = "manual".into();
    assert!(matches!(
        s.upsert_provider_price(&bad),
        Err(StorageError::Invalid(_))
    ));

    s.delete_manual_price(a, "2026-10-30").unwrap();
    assert_eq!(
        s.latest_price(a, "2026-11-01").unwrap().unwrap().value,
        "1.21"
    );
    assert!(s.delete_manual_price(a, "2026-10-30").is_err());
}

#[test]
fn fx_rates_latest_on_or_before() {
    let mut s = Store::open_in_memory().unwrap();
    let row = |c: &str, d: &str, r: &str| FxRow {
        currency: c.into(),
        date: d.into(),
        source: "tcmb".into(),
        try_per_unit: r.into(),
    };
    s.upsert_fx_rate(&row("USD", "2026-10-30", "34.00"), "t")
        .unwrap();
    s.upsert_fx_rate(&row("USD", "2026-10-31", "34.10"), "t")
        .unwrap();
    s.upsert_fx_rate(&row("EUR", "2026-10-31", "37.00"), "t")
        .unwrap();
    assert_eq!(
        s.latest_fx_rate("USD", "2026-11-01")
            .unwrap()
            .unwrap()
            .try_per_unit,
        "34.10"
    );
    assert_eq!(
        s.latest_fx_rate("USD", "2026-10-30")
            .unwrap()
            .unwrap()
            .try_per_unit,
        "34.00"
    );
    assert!(s.latest_fx_rate("JPY", "2026-11-01").unwrap().is_none());
    s.upsert_fx_rate(&row("USD", "2026-10-31", "34.20"), "t2")
        .unwrap();
    assert_eq!(
        s.latest_fx_rate("USD", "2026-11-01")
            .unwrap()
            .unwrap()
            .try_per_unit,
        "34.20",
        "upsert"
    );
    assert_eq!(
        s.fx_currencies().unwrap(),
        vec!["EUR".to_string(), "USD".to_string()]
    );
}

#[test]
fn merge_undo_and_key_map() {
    let mut s = Store::open_in_memory().unwrap();
    let isin = asset(&mut s, "isin:IE00BK5BQT80");
    let sym = asset(&mut s, "sym:VWCE@XETR");
    let other = asset(&mut s, "sym:AAPL@");
    assert_eq!(s.canonical_asset(sym).unwrap(), sym, "never merged");
    assert!(s.key_map().unwrap().is_empty());

    let m = s.merge_assets(sym, isin).unwrap();
    assert_eq!(s.canonical_asset(sym).unwrap(), isin);
    assert_eq!(
        s.key_map()
            .unwrap()
            .get("sym:VWCE@XETR")
            .map(String::as_str),
        Some("isin:IE00BK5BQT80")
    );
    assert!(s.merge_assets(sym, other).is_err(), "already merged");
    assert!(s.merge_assets(isin, sym).is_err(), "would form a cycle");
    assert!(s.merge_assets(isin, isin).is_err());

    // Chained merge resolves to the final survivor.
    s.merge_assets(isin, other).unwrap();
    assert_eq!(s.canonical_asset(sym).unwrap(), other);
    assert_eq!(
        s.key_map()
            .unwrap()
            .get("sym:VWCE@XETR")
            .map(String::as_str),
        Some("sym:AAPL@")
    );

    s.undo_merge(m).unwrap();
    assert_eq!(s.canonical_asset(sym).unwrap(), sym);
    assert!(s.undo_merge(m).is_err(), "already undone");
    assert_eq!(s.active_merges().unwrap().len(), 1);
}

#[test]
fn refresh_log_keeps_latest() {
    let mut s = Store::open_in_memory().unwrap();
    assert!(s.last_refresh_report().unwrap().is_none());
    s.log_refresh("a", "2026-11-01T10:00:00Z", "{\"n\":1}")
        .unwrap();
    s.log_refresh("b", "2026-11-01T11:00:00Z", "{\"n\":2}")
        .unwrap();
    assert_eq!(
        s.last_refresh_report().unwrap().unwrap(),
        ("2026-11-01T11:00:00Z".to_string(), "{\"n\":2}".to_string())
    );
}

#[test]
fn migration_0003_keeps_existing_price_sources() {
    let dir = std::env::temp_dir().join(format!("mizan-market-mig3-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("v2.db");
    let _ = std::fs::remove_file(&path);
    {
        // A database as the previous version left it: 0001 and 0002 applied.
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(include_str!("../migrations/0001_init.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../migrations/0002_market.sql"))
            .unwrap();
        conn.pragma_update(None, "user_version", 2).unwrap();
        conn.execute_batch(
            "INSERT INTO assets (id, instrument_key, hint_json) VALUES (1, 'sym:AFT@', '{}'), (2, 'sym:X@', '{}');
             INSERT INTO price_sources (asset_id, provider, symbol) VALUES (1, 'tefas', 'AFT'), (2, 'manual', NULL);",
        )
        .unwrap();
    }
    let mut s = Store::open(&path).unwrap();
    assert_eq!(s.schema_version().unwrap(), 9);
    assert_eq!(
        s.price_source(1).unwrap(),
        Some(PriceSourceKind::Tefas {
            fund_code: "AFT".into()
        })
    );
    assert_eq!(s.price_source(2).unwrap(), Some(PriceSourceKind::Manual));
    let yahoo = PriceSourceKind::Yahoo {
        symbol: "VWCE.DE".into(),
    };
    s.set_price_source(2, Some(&yahoo)).unwrap();
    assert_eq!(s.price_source(2).unwrap(), Some(yahoo));
    drop(s);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn price_sources_round_trip_every_kind() {
    let mut s = Store::open_in_memory().unwrap();
    let a = asset(&mut s, "sym:A@");
    for (given, stored) in [
        (
            PriceSourceKind::Tefas {
                fund_code: " aft ".into(),
            },
            PriceSourceKind::Tefas {
                fund_code: "AFT".into(),
            },
        ),
        (
            PriceSourceKind::Yahoo {
                symbol: "vwce.de".into(),
            },
            PriceSourceKind::Yahoo {
                symbol: "VWCE.DE".into(),
            },
        ),
        (
            PriceSourceKind::TwelveData {
                symbol: "MSFT".into(),
            },
            PriceSourceKind::TwelveData {
                symbol: "MSFT".into(),
            },
        ),
        (PriceSourceKind::Manual, PriceSourceKind::Manual),
    ] {
        s.set_price_source(a, Some(&given)).unwrap();
        assert_eq!(s.price_source(a).unwrap(), Some(stored));
    }
    s.set_price_source(a, None).unwrap();
    assert_eq!(s.price_source(a).unwrap(), None);
}

#[test]
fn empty_symbol_is_refused_and_source_unchanged() {
    let mut s = Store::open_in_memory().unwrap();
    let a = asset(&mut s, "sym:A@");
    s.set_price_source(a, Some(&PriceSourceKind::Manual))
        .unwrap();
    let err = s
        .set_price_source(
            a,
            Some(&PriceSourceKind::TwelveData {
                symbol: "  ".into(),
            }),
        )
        .unwrap_err();
    assert!(err.to_string().contains("needs a symbol"), "{err}");
    assert_eq!(s.price_source(a).unwrap(), Some(PriceSourceKind::Manual));
}

#[test]
fn benchmark_candidates_are_kept_canonical_and_existing() {
    let mut s = Store::open_in_memory().unwrap();
    let hint = |sym: &str| mizan_import::InstrumentHint {
        symbol: Some(sym.into()),
        ..Default::default()
    };
    let a = s.create_asset("sym:A@", &hint("A")).unwrap();
    let b = s.create_asset("sym:B@", &hint("B")).unwrap();
    assert!(s.benchmark_candidates().unwrap().is_empty());
    s.set_benchmark_candidate(a, true).unwrap();
    s.set_benchmark_candidate(b, true).unwrap();
    s.set_benchmark_candidate(a, true).unwrap();
    assert_eq!(
        s.benchmark_candidates()
            .unwrap()
            .into_iter()
            .collect::<Vec<_>>(),
        [a, b]
    );
    s.set_benchmark_candidate(b, false).unwrap();
    assert_eq!(
        s.benchmark_candidates()
            .unwrap()
            .into_iter()
            .collect::<Vec<_>>(),
        [a]
    );
    s.set_setting(mizan_storage::BENCHMARK_CANDIDATES_KEY, "x, 999 ,")
        .unwrap();
    assert!(
        s.benchmark_candidates().unwrap().is_empty(),
        "junk and missing assets are ignored"
    );
}
