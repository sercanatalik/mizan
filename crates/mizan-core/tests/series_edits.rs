//! The order of sources with per-database series edits (market-data-edits spec).

use mizan_core::returns::*;
use mizan_core::*;
use mizan_import::{parse_batch_value, Code, Date};
use mizan_storage::{
    CachedFundPrice, CachedListedPrice, CachedRate, HistoryCache, PriceSourceKind, SeriesKind,
    Store,
};
use rust_decimal::Decimal;
use serde_json::{json, Value};
use std::str::FromStr;

const CREATE: CreatePolicy = CreatePolicy {
    create_accounts: true,
    create_assets: true,
};

fn day(s: &str) -> Date {
    Date::parse(s).unwrap()
}

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
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

fn fund(sym: &str) -> Value {
    json!({"symbol": sym, "class_hint": "fund", "currency": "TRY"})
}

fn key(store: &Store, symbol: &str) -> String {
    store
        .list_assets()
        .unwrap()
        .into_iter()
        .find(|a| a.hint.symbol.as_deref() == Some(symbol))
        .unwrap()
        .instrument_key
}

fn listed(cache: &mut HistoryCache, symbol: &str, date: &str, close: &str) {
    cache
        .upsert_listed_price(
            &CachedListedPrice {
                source: "yahoo".into(),
                symbol: symbol.into(),
                date: date.into(),
                close: close.into(),
                currency: "EUR".into(),
            },
            "t",
        )
        .unwrap();
}

fn fund_price(cache: &mut HistoryCache, code: &str, date: &str, p: &str) {
    cache
        .upsert_fund_price(
            &CachedFundPrice {
                code: code.into(),
                date: date.into(),
                price: p.into(),
            },
            "t",
        )
        .unwrap();
}

/// Two accounts holding VWCE (Yahoo) and two assets priced from fund AFT.
fn setup() -> (Store, HistoryCache) {
    let mut s = Store::open_in_memory().unwrap();
    import(
        &mut s,
        'a',
        "P1",
        vec![
            json!({"kind": "buy", "date": "2025-03-03", "instrument": {"symbol": "VWCE", "currency": "EUR"}, "quantity": "10", "net_cash": money("460.00", "EUR")}),
            json!({"kind": "buy", "date": "2025-03-03", "instrument": fund("AFT"), "quantity": "100", "net_cash": money("100.00", "TRY")}),
            json!({"kind": "buy", "date": "2025-03-03", "instrument": fund("AFT2"), "quantity": "100", "net_cash": money("100.00", "TRY")}),
        ],
    );
    for a in s.list_assets().unwrap() {
        let src = match a.hint.symbol.as_deref() {
            Some("VWCE") => PriceSourceKind::Yahoo {
                symbol: "VWCE.DE".into(),
            },
            // Two assets share one fund series.
            _ => PriceSourceKind::Tefas {
                fund_code: "AFT".into(),
            },
        };
        s.set_price_source(a.id, Some(&src)).unwrap();
    }
    let mut c = HistoryCache::open_in_memory().unwrap();
    listed(&mut c, "VWCE.DE", "2025-03-13", "46.90");
    listed(&mut c, "VWCE.DE", "2025-03-14", "4712.00");
    fund_price(&mut c, "AFT", "2025-03-14", "1.20");
    for (d, r) in [("2025-03-31", "37.90"), ("2025-04-02", "38.20")] {
        c.upsert_rate(
            &CachedRate {
                currency: "USD".into(),
                date: d.into(),
                try_per_unit: r.into(),
                source: "tcmb".into(),
            },
            "t",
        )
        .unwrap();
    }
    (s, c)
}

fn price(s: &Store, c: &HistoryCache, symbol: &str, date: &str) -> Decimal {
    let book = PriceBook::load(s, Some(c)).unwrap();
    book.price_on(&key(s, symbol), day(date)).unwrap().value
}

#[test]
fn correcting_a_bad_close() {
    let (mut s, c) = setup();
    assert_eq!(price(&s, &c, "VWCE", "2025-03-14"), d("4712.00"));
    s.set_series_value(
        SeriesKind::Price,
        "yahoo:VWCE.DE",
        "2025-03-14",
        "47.12",
        Some("EUR"),
        "t",
    )
    .unwrap();
    assert_eq!(price(&s, &c, "VWCE", "2025-03-14"), d("47.12"));
    assert_eq!(
        c.listed_prices("yahoo", "VWCE.DE").unwrap()[1].close,
        "4712.00",
        "the cache is untouched"
    );
}

#[test]
fn ignoring_a_spike_falls_back_to_the_day_before() {
    let (mut s, c) = setup();
    s.ignore_series_value(SeriesKind::Price, "yahoo:VWCE.DE", "2025-03-14", "t")
        .unwrap();
    assert_eq!(price(&s, &c, "VWCE", "2025-03-14"), d("46.90"));
    s.revert_series_value(SeriesKind::Price, "yahoo:VWCE.DE", "2025-03-14")
        .unwrap();
    assert_eq!(price(&s, &c, "VWCE", "2025-03-14"), d("4712.00"));
}

#[test]
fn a_missing_tcmb_day_can_be_added() {
    let (mut s, c) = setup();
    let usd = code("USD");
    let rate = |s: &Store| {
        PriceBook::load(s, Some(&c))
            .unwrap()
            .try_per_unit(&usd, day("2025-04-01"))
            .unwrap()
    };
    assert_eq!(rate(&s), d("37.90"));
    s.set_series_value(SeriesKind::Rate, "USD", "2025-04-01", "38.10", None, "t")
        .unwrap();
    assert_eq!(rate(&s), d("38.10"));
    s.revert_series_value(SeriesKind::Rate, "USD", "2025-04-01")
        .unwrap();
    assert_eq!(rate(&s), d("37.90"));
    // Ignoring a stored rate falls back to the day before.
    s.ignore_series_value(SeriesKind::Rate, "USD", "2025-04-02", "t")
        .unwrap();
    let book = PriceBook::load(&s, Some(&c)).unwrap();
    assert_eq!(book.try_per_unit(&usd, day("2025-04-02")), Some(d("37.90")));
}

#[test]
fn a_manual_price_beats_an_edit_and_one_edit_reaches_every_asset_of_the_series() {
    let (mut s, c) = setup();
    s.set_series_value(
        SeriesKind::Price,
        "tefas:AFT",
        "2025-03-14",
        "1.22",
        Some("TRY"),
        "t",
    )
    .unwrap();
    assert_eq!(price(&s, &c, "AFT", "2025-03-14"), d("1.22"));
    assert_eq!(
        price(&s, &c, "AFT2", "2025-03-14"),
        d("1.22"),
        "same series, same edit"
    );
    let a = s
        .list_assets()
        .unwrap()
        .into_iter()
        .find(|a| a.hint.symbol.as_deref() == Some("AFT"))
        .unwrap();
    s.add_manual_price(a.id, "2025-03-14", "1.25", "TRY", "t")
        .unwrap();
    assert_eq!(price(&s, &c, "AFT", "2025-03-14"), d("1.25"));
    assert_eq!(price(&s, &c, "AFT2", "2025-03-14"), d("1.22"));
    // An ignore does not hide a manual price.
    s.ignore_series_value(SeriesKind::Price, "tefas:AFT", "2025-03-14", "t")
        .unwrap();
    assert_eq!(price(&s, &c, "AFT", "2025-03-14"), d("1.25"));
}

#[test]
fn holdings_value_positions_with_the_edited_latest_price() {
    let (mut s, c) = setup();
    let today = day("2025-03-14");
    let value = |s: &Store| {
        let book = PriceBook::load(s, Some(&c)).unwrap();
        let m = mizan_core::valuation::BookMarket {
            store: s,
            book: &book,
        };
        let h = mizan_core::portfolio::holdings_in(s, &m, today).unwrap();
        let p = h.accounts[0]
            .positions
            .iter()
            .find(|p| p.symbol.as_deref() == Some("VWCE"))
            .unwrap()
            .clone();
        (p.value, p.price.map(|x| x.source))
    };
    assert_eq!(value(&s), (Some("47120.00".into()), Some("yahoo".into())));
    s.set_series_value(
        SeriesKind::Price,
        "yahoo:VWCE.DE",
        "2025-03-14",
        "47.12",
        Some("EUR"),
        "t",
    )
    .unwrap();
    assert_eq!(value(&s), (Some("471.20".into()), Some("edited".into())));
}

#[test]
fn real_returns_use_an_edited_index_month() {
    let s = Store::open_in_memory().unwrap();
    let mut c = HistoryCache::open_in_memory().unwrap();
    let v = |m: &str, x: &str| mizan_storage::CachedIndexValue {
        month: m.into(),
        value: x.into(),
    };
    c.commit_inflation(
        "eurostat",
        "TR:I15",
        &[
            v("2026-07", "1600"),
            v("2026-08", "1640"),
            v("2026-09", "1680"),
        ],
        true,
        "t",
    )
    .unwrap();
    let on = |s: &Store| match mizan_core::returns::Inflation::load(s, Some(&c), &code("TRY"))
        .unwrap()
    {
        mizan_core::returns::Inflation::Ready(i) => i.on(day("2026-08-15")).unwrap().value,
        other => panic!("{other:?}"),
    };
    assert_eq!(on(&s), d("1640"));
    let mut s = s;
    s.set_series_value(
        SeriesKind::Index,
        "eurostat:TR:I15",
        "2026-08",
        "1650",
        None,
        "t",
    )
    .unwrap();
    assert_eq!(on(&s), d("1650"));
    // Ignoring the month interpolates across it.
    s.ignore_series_value(SeriesKind::Index, "eurostat:TR:I15", "2026-08", "t")
        .unwrap();
    assert!(on(&s) > d("1600") && on(&s) < d("1680") && on(&s) != d("1640"));
}

#[test]
fn turkish_tax_uses_an_edited_rate() {
    use mizan_core::tax::report::{report, TaxInputs};
    use mizan_core::tax::{Country, TaxYear};
    let mut s = Store::open_in_memory().unwrap();
    import(
        &mut s,
        'b',
        "U1",
        vec![
            json!({"kind": "buy", "date": "2025-01-06", "instrument": {"isin": "US0378331005", "symbol": "AAPL", "currency": "USD"}, "quantity": "1", "net_cash": money("100.00", "USD")}),
            json!({"kind": "sell", "date": "2025-06-10", "instrument": {"isin": "US0378331005", "symbol": "AAPL", "currency": "USD"}, "quantity": "1", "net_cash": money("200.00", "USD")}),
        ],
    );
    let mut c = HistoryCache::open_in_memory().unwrap();
    for (dt, r) in [("2025-01-06", "35.00"), ("2025-06-10", "39.00")] {
        c.upsert_rate(
            &CachedRate {
                currency: "USD".into(),
                date: dt.into(),
                try_per_unit: r.into(),
                source: "tcmb".into(),
            },
            "t",
        )
        .unwrap();
    }
    let proceeds = |s: &Store| {
        let i = TaxInputs::load(s, Some(&c)).unwrap();
        report(
            &i,
            Country::Tr,
            TaxYear::parse(Country::Tr, "2025").unwrap(),
            "t",
        )
        .gains[0]
            .rows[0]
            .proceeds
            .clone()
    };
    assert_eq!(proceeds(&s).as_deref(), Some("7800.00"));
    s.set_series_value(SeriesKind::Rate, "USD", "2025-06-10", "39.05", None, "t")
        .unwrap();
    assert_eq!(proceeds(&s).as_deref(), Some("7810.00"));
}
