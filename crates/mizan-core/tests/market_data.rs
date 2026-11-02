//! The Market data screen's read model (market-data-browser spec).

use mizan_core::market_data::*;
use mizan_core::*;
use mizan_import::{parse_batch_value, Code, Date};
use mizan_storage::{
    CachedFundPrice, CachedIndexValue, CachedListedPrice, CachedRate, HistoryCache,
    PriceSourceKind, Store,
};
use rust_decimal::Decimal;
use serde_json::{json, Value};

const CREATE: CreatePolicy = CreatePolicy {
    create_accounts: true,
    create_assets: true,
};

fn day(s: &str) -> Date {
    Date::parse(s).unwrap()
}

fn code(s: &str) -> Code {
    Code::parse(s).unwrap()
}

/// Commit records; each gets account `hint` unless it has one.
fn import(store: &mut Store, file: char, hint: &str, records: Vec<Value>) {
    let records: Vec<Value> = records
        .into_iter()
        .enumerate()
        .map(|(i, mut r)| {
            r["source"] = json!({"adapter": "t", "adapter_version": "1", "file_sha256": file.to_string().repeat(64), "locator": format!("row {}", i + 1)});
            if r.get("account").is_none() {
                r["account"] = json!({"hint": hint, "name_hint": hint});
            }
            r
        })
        .collect();
    let parsed =
        parse_batch_value(json!({"schema": "mizan.import/1", "records": records})).unwrap();
    assert!(parsed.is_clean(), "{:?}", parsed.errors);
    let rb = {
        let mut r = MappingResolver {
            store,
            policy: CREATE,
        };
        resolve_batch(parsed, &mut r).unwrap()
    };
    let p = prepare(store, rb).unwrap();
    commit(store, &p, &Decisions::new()).unwrap();
}

fn money(a: &str, c: &str) -> Value {
    json!({"amount": a, "currency": c})
}

const TODAY: &str = "2025-06-20";

fn fund_price(c: &mut HistoryCache, code: &str, date: &str, p: &str) {
    c.upsert_fund_price(
        &CachedFundPrice {
            code: code.into(),
            date: date.into(),
            price: p.into(),
        },
        "f",
    )
    .unwrap();
}

fn rate(c: &mut HistoryCache, ccy: &str, date: &str, r: &str) {
    c.upsert_rate(
        &CachedRate {
            currency: ccy.into(),
            date: date.into(),
            try_per_unit: r.into(),
            source: "tcmb".into(),
        },
        "f",
    )
    .unwrap();
}

/// A database holding fund AFT (TEFAS) and VWCE (Yahoo); the cache also holds
/// fund TTE, which nothing uses, a stale listed symbol, USD rates and an index.
fn setup() -> (Store, HistoryCache) {
    let mut s = Store::open_in_memory().unwrap();
    import(
        &mut s,
        'a',
        "P1",
        vec![
            json!({"kind": "buy", "date": "2025-06-02", "instrument": {"symbol": "AFT", "name": "Ak Portfoy Teknoloji", "class_hint": "fund", "currency": "TRY"}, "quantity": "100", "net_cash": money("100.00", "TRY")}),
            json!({"kind": "buy", "date": "2025-06-02", "instrument": {"symbol": "VWCE", "currency": "EUR"}, "quantity": "1", "net_cash": money("100.00", "EUR")}),
        ],
    );
    for a in s.list_assets().unwrap() {
        let src = match a.hint.symbol.as_deref() {
            Some("AFT") => PriceSourceKind::Tefas {
                fund_code: "AFT".into(),
            },
            _ => PriceSourceKind::Yahoo {
                symbol: "VWCE.DE".into(),
            },
        };
        s.set_price_source(a.id, Some(&src)).unwrap();
    }
    let mut c = HistoryCache::open_in_memory().unwrap();
    // AFT: every weekday of June 2025 up to the 19th except the 10th.
    let mut day0 = Date::parse("2025-06-02").unwrap().0;
    while day0 <= Date::parse("2025-06-19").unwrap().0 {
        let d = day0.to_string();
        let wd = chrono::Datelike::weekday(&day0);
        if !matches!(wd, chrono::Weekday::Sat | chrono::Weekday::Sun) && d != "2025-06-10" {
            fund_price(&mut c, "AFT", &d, "1.00");
        }
        day0 += chrono::Duration::days(1);
    }
    fund_price(&mut c, "TTE", "2025-06-19", "2.50");
    c.upsert_listed_price(
        &CachedListedPrice {
            source: "yahoo".into(),
            symbol: "VWCE.DE".into(),
            date: "2025-06-06".into(),
            close: "131.42".into(),
            currency: "EUR".into(),
        },
        "f",
    )
    .unwrap();
    rate(&mut c, "USD", "2025-06-10", "39.00");
    rate(&mut c, "USD", "2025-06-19", "39.40");
    c.commit_inflation(
        "eurostat",
        "TR:I15",
        &[CachedIndexValue {
            month: "2025-05".into(),
            value: "1500".into(),
        }],
        true,
        "f",
    )
    .unwrap();
    (s, c)
}

fn page(s: &Store, c: &HistoryCache, kind: &str, f: SeriesFilter) -> SeriesPage {
    series_page(s, Some(c), kind, &f, day(TODAY)).unwrap()
}

fn row<'a>(p: &'a SeriesPage, key: &str) -> &'a SeriesRow {
    p.rows
        .iter()
        .find(|r| r.key == key)
        .unwrap_or_else(|| panic!("no {key}"))
}

#[test]
fn every_cached_series_is_listed_with_its_quality() {
    let (s, c) = setup();
    let p = page(&s, &c, "price", SeriesFilter::default());
    assert_eq!(p.total, 3);
    let tte = row(&p, "tefas:TTE");
    assert!(!tte.used, "a fund no asset uses");
    assert_eq!(tte.held_gaps, None, "nobody holds TTE");
    assert_eq!((tte.latest.as_deref(), tte.count), (Some("2.50"), 1));
    let aft = row(&p, "tefas:AFT");
    assert_eq!(aft.name, "Ak Portfoy Teknoloji");
    assert!(aft.used);
    assert_eq!(
        (
            aft.first_date.as_deref(),
            aft.latest_date.as_deref(),
            aft.series_gaps
        ),
        (Some("2025-06-02"), Some("2025-06-19"), 1)
    );
    assert!(!aft.stale);
    let vwce = row(&p, "yahoo:VWCE.DE");
    assert!(vwce.stale, "14 days old");
    assert_eq!(
        vwce.held_gaps,
        Some(4),
        "held from 2025-06-02, first close 2025-06-06"
    );
    assert_eq!(vwce.currency.as_deref(), Some("EUR"));
    assert_eq!(p.sources, ["tefas", "yahoo"]);

    let r = page(&s, &c, "rate", SeriesFilter::default());
    assert_eq!(row(&r, "USD").latest.as_deref(), Some("39.40"));
    let i = page(&s, &c, "index", SeriesFilter::default());
    let tr = row(&i, "eurostat:TR:I15");
    assert_eq!(tr.name, "HICP Türkiye (Eurostat)");
    assert!(tr.used && !tr.stale && tr.series_gaps == 0 && tr.held_gaps.is_none());
}

#[test]
fn filters_combine() {
    let (mut s, c) = setup();
    let n = |s: &Store, f: SeriesFilter, kind: &str| page(s, &c, kind, f).rows.len();
    assert_eq!(
        n(
            &s,
            SeriesFilter {
                used: true,
                ..Default::default()
            },
            "price"
        ),
        2
    );
    assert_eq!(
        n(
            &s,
            SeriesFilter {
                source: Some("tefas".into()),
                ..Default::default()
            },
            "price"
        ),
        2
    );
    assert_eq!(
        n(
            &s,
            SeriesFilter {
                currency: Some("EUR".into()),
                ..Default::default()
            },
            "price"
        ),
        1
    );
    assert_eq!(
        n(
            &s,
            SeriesFilter {
                text: Some("teknol".into()),
                ..Default::default()
            },
            "price"
        ),
        1
    );
    assert_eq!(
        n(
            &s,
            SeriesFilter {
                attention: true,
                ..Default::default()
            },
            "price"
        ),
        2,
        "AFT has a gap, VWCE is stale"
    );
    assert_eq!(
        n(
            &s,
            SeriesFilter {
                edited: true,
                ..Default::default()
            },
            "rate"
        ),
        0
    );
    set_value(
        &mut s,
        "rate",
        "USD",
        "2025-06-10",
        "39.05",
        None,
        "2025-06-20T09:00:00Z",
    )
    .unwrap();
    assert_eq!(
        n(
            &s,
            SeriesFilter {
                edited: true,
                ..Default::default()
            },
            "rate"
        ),
        1
    );
    assert_eq!(
        n(
            &s,
            SeriesFilter {
                edited: true,
                source: Some("tefas".into()),
                ..Default::default()
            },
            "price"
        ),
        0
    );
}

#[test]
fn edited_value_in_the_table() {
    let (mut s, c) = setup();
    set_value(
        &mut s,
        "rate",
        "USD",
        "2025-06-10",
        "39.05",
        None,
        "2025-06-20T09:00:00Z",
    )
    .unwrap();
    let d = series_detail(&s, Some(&c), "rate", "USD", 0, day(TODAY)).unwrap();
    let p = d.points.iter().find(|p| p.period == "2025-06-10").unwrap();
    assert_eq!(
        (
            p.value.as_str(),
            p.source.as_str(),
            p.tag.as_deref(),
            p.original.as_deref()
        ),
        ("39.05", "edited", Some("edited"), Some("39.00"))
    );
    assert_eq!(d.row.edits, 1);
    assert_eq!(
        c.rate_points("USD").unwrap()[0].value,
        "39.00",
        "the cache is untouched"
    );

    ignore_value(&mut s, "rate", "USD", "2025-06-19", "t").unwrap();
    let d = series_detail(&s, Some(&c), "rate", "USD", 0, day(TODAY)).unwrap();
    let ig = &d.points[0];
    assert_eq!(
        (ig.period.as_str(), ig.tag.as_deref(), ig.in_use),
        ("2025-06-19", Some("ignored"), false)
    );
    assert_eq!(
        d.row.latest.as_deref(),
        Some("39.05"),
        "the latest value in use"
    );
    assert_eq!(d.chart.len(), 1);

    // An added month in an index, then reverted.
    set_value(
        &mut s,
        "index",
        "eurostat:TR:I15",
        "2025-06",
        "1520",
        None,
        "t",
    )
    .unwrap();
    assert_eq!(
        series_detail(&s, Some(&c), "index", "eurostat:TR:I15", 0, day(TODAY))
            .unwrap()
            .total,
        2
    );
    revert_value(&mut s, "index", "eurostat:TR:I15", "2025-06").unwrap();
    assert_eq!(
        series_detail(&s, Some(&c), "index", "eurostat:TR:I15", 0, day(TODAY))
            .unwrap()
            .total,
        1
    );
    assert!(revert_value(&mut s, "index", "eurostat:TR:I15", "2025-06").is_err());

    // Validation.
    assert!(set_value(
        &mut s,
        "index",
        "eurostat:TR:I15",
        "2025-06-01",
        "1",
        None,
        "t"
    )
    .is_err());
    assert!(
        set_value(
            &mut s,
            "price",
            "tefas:AFT",
            "2025-06-10",
            "1.01",
            None,
            "t"
        )
        .is_err(),
        "prices need a currency"
    );
    assert!(set_value(&mut s, "rate", "USD", "2025-06-10", "-1", None, "t").is_err());
    assert!(set_value(&mut s, "rate", "USD", "2025-06-10", "abc", None, "t").is_err());
}

#[test]
fn fund_history_is_paged_and_its_chart_thinned() {
    let mut s = Store::open_in_memory().unwrap();
    let mut c = HistoryCache::open_in_memory().unwrap();
    let start = Date::parse("2020-01-01").unwrap().0;
    for i in 0..1201 {
        fund_price(
            &mut c,
            "AFT",
            &(start + chrono::Duration::days(i)).to_string(),
            &format!("{}", 1 + i),
        );
    }
    set_value(
        &mut s,
        "price",
        "tefas:AFT",
        "2020-01-05",
        "99",
        Some("TRY"),
        "t",
    )
    .unwrap();
    let d = series_detail(&s, Some(&c), "price", "tefas:AFT", 0, day("2023-04-20")).unwrap();
    assert_eq!((d.total, d.points.len()), (1201, 200));
    assert_eq!(d.points[0].period, "2023-04-15", "newest first");
    assert!(
        d.chart.len() <= 1000 + 1 && d.chart.len() > 500,
        "{}",
        d.chart.len()
    );
    assert!(
        d.chart.iter().any(|p| p.period == "2020-01-05" && p.edited),
        "edited points are kept"
    );
    let next = series_detail(&s, Some(&c), "price", "tefas:AFT", 1200, day("2023-04-20")).unwrap();
    assert_eq!(
        (next.points.len(), next.points[0].period.as_str()),
        (1, "2020-01-01")
    );
    let _ = (Decimal::ONE, code("TRY"));
}

#[test]
fn series_csv_matches_the_golden_file() {
    let (mut s, c) = setup();
    set_value(&mut s, "rate", "USD", "2025-06-10", "39.05", None, "t").unwrap();
    ignore_value(&mut s, "rate", "USD", "2025-06-19", "t").unwrap();
    set_value(&mut s, "rate", "USD", "2025-06-11", "39.10", None, "t").unwrap();
    let body = series_csv(&s, Some(&c), "rate", "USD").unwrap();
    assert_eq!(csv_file_name("rate", "USD"), "mizan-rate-USD.csv");
    assert_eq!(
        csv_file_name("index", "eurostat:TR:I15"),
        "mizan-index-eurostat-TR-I15.csv"
    );
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/market/mizan-rate-USD.csv");
    if std::env::var_os("MIZAN_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &body).unwrap();
    }
    assert_eq!(body, std::fs::read_to_string(&path).unwrap_or_default());
}

#[test]
fn gaps_while_held_count_only_held_days_and_days_before_the_history() {
    let mut s = Store::open_in_memory().unwrap();
    import(
        &mut s,
        'b',
        "P1",
        vec![
            // Bought before the cached history starts; sold in early 2022.
            json!({"kind": "buy", "date": "2021-12-27", "instrument": {"symbol": "AFT", "class_hint": "fund", "currency": "TRY"}, "quantity": "10", "net_cash": money("10.00", "TRY")}),
            json!({"kind": "sell", "date": "2022-01-07", "instrument": {"symbol": "AFT", "class_hint": "fund", "currency": "TRY"}, "quantity": "10", "net_cash": money("10.00", "TRY")}),
            // Bought again in 2025.
            json!({"kind": "buy", "date": "2025-01-06", "instrument": {"symbol": "AFT", "class_hint": "fund", "currency": "TRY"}, "quantity": "10", "net_cash": money("10.00", "TRY")}),
        ],
    );
    let a = s.list_assets().unwrap()[0].id;
    s.set_price_source(
        a,
        Some(&PriceSourceKind::Tefas {
            fund_code: "AFT".into(),
        }),
    )
    .unwrap();
    let mut c = HistoryCache::open_in_memory().unwrap();
    // Every weekday 2022-01-03..2022-01-14 except 01-04 and 01-12, and 2025-01-06..01-10 except 01-08.
    for d in [
        "2022-01-03",
        "2022-01-05",
        "2022-01-06",
        "2022-01-07",
        "2022-01-10",
        "2022-01-11",
        "2022-01-13",
        "2022-01-14",
    ] {
        fund_price(&mut c, "AFT", d, "1.00");
    }
    for d in ["2025-01-06", "2025-01-07", "2025-01-09", "2025-01-10"] {
        fund_price(&mut c, "AFT", d, "1.00");
    }
    let p = series_page(
        &s,
        Some(&c),
        "price",
        &SeriesFilter::default(),
        day("2025-01-10"),
    )
    .unwrap();
    let r = row(&p, "tefas:AFT");
    // Held 2021-12-27..2022-01-06 (sold on the 7th) and from 2025-01-06.
    // Gaps while held: 5 weekdays before the history (2021-12-27..31), 2022-01-04, and 2025-01-08.
    assert_eq!(r.held_gaps, Some(7));
    // The series' own gaps: weekdays 2022-01-03..2025-01-10 without a value; includes 2022-01-12 (not held).
    assert!(r.series_gaps > 700, "{}", r.series_gaps);
    // Ignoring a held day adds a gap; adding a missing held day removes one.
    set_value(
        &mut s,
        "price",
        "tefas:AFT",
        "2025-01-08",
        "1.00",
        Some("TRY"),
        "t",
    )
    .unwrap();
    let p = series_page(
        &s,
        Some(&c),
        "price",
        &SeriesFilter::default(),
        day("2025-01-10"),
    )
    .unwrap();
    assert_eq!(row(&p, "tefas:AFT").held_gaps, Some(6));
    ignore_value(&mut s, "price", "tefas:AFT", "2025-01-09", "t").unwrap();
    let p = series_page(
        &s,
        Some(&c),
        "price",
        &SeriesFilter::default(),
        day("2025-01-10"),
    )
    .unwrap();
    assert_eq!(row(&p, "tefas:AFT").held_gaps, Some(7));
}
