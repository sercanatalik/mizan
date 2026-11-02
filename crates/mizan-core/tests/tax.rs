//! Tax reports from a real database and history cache (tax-reports spec).

use mizan_core::tax::report::{report, years, TaxInputs};
use mizan_core::tax::{Country, TaxTreatment, TaxYear};
use mizan_core::*;
use mizan_import::{parse_batch_value, Date};
use mizan_storage::{CachedIndexValue, CachedRate, HistoryCache, PriceSourceKind, Store};
use serde_json::{json, Value};

const CREATE: CreatePolicy = CreatePolicy {
    create_accounts: true,
    create_assets: true,
};

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

fn rate(cache: &mut HistoryCache, ccy: &str, date: &str, value: &str) {
    cache
        .upsert_rate(
            &CachedRate {
                currency: ccy.into(),
                date: date.into(),
                try_per_unit: value.into(),
                source: "tcmb".into(),
            },
            "t",
        )
        .unwrap();
}

fn store_and_cache() -> (Store, HistoryCache) {
    let mut s = Store::open_in_memory().unwrap();
    import(
        &mut s,
        'a',
        "IB1",
        vec![
            json!({"kind": "buy", "date": "2023-03-01", "instrument": {"isin": "US0378331005", "symbol": "AAPL", "name": "Apple Inc.", "currency": "USD"}, "quantity": "10", "net_cash": money("1500.00", "USD")}),
            json!({"kind": "sell", "date": "2025-06-10", "instrument": {"isin": "US0378331005", "symbol": "AAPL", "currency": "USD"}, "quantity": "10", "net_cash": money("2200.00", "USD")}),
            json!({"kind": "dividend", "date": "2025-05-15", "instrument": {"isin": "US0378331005", "symbol": "AAPL", "currency": "USD"}, "net_cash": money("8.50", "USD"), "charges": [{"type": "withholding_tax", "amount": money("1.50", "USD")}]}),
            json!({"kind": "buy", "date": "2025-01-06", "instrument": {"isin": "IE00B4L5Y983", "symbol": "IWDA", "name": "iShares Core MSCI World, Acc", "currency": "USD"}, "quantity": "5", "net_cash": money("500.00", "USD")}),
            json!({"kind": "sell", "date": "2025-06-10", "instrument": {"isin": "IE00B4L5Y983", "symbol": "IWDA", "currency": "USD"}, "quantity": "5", "net_cash": money("450.00", "USD")}),
            json!({"kind": "buy", "date": "2024-02-01", "instrument": fund("AFT"), "quantity": "1000", "net_cash": money("1000.00", "TRY")}),
            json!({"kind": "sell", "date": "2025-02-03", "instrument": fund("AFT"), "quantity": "1000", "net_cash": money("1400.00", "TRY")}),
        ],
    );
    import(
        &mut s,
        'b',
        "AH1",
        vec![
            json!({"kind": "buy", "date": "2024-02-01", "instrument": fund("AH2"), "quantity": "100", "net_cash": money("100.00", "TRY")}),
            json!({"kind": "sell", "date": "2025-02-03", "instrument": fund("AH2"), "quantity": "100", "net_cash": money("150.00", "TRY")}),
        ],
    );
    let aft = s
        .list_assets()
        .unwrap()
        .into_iter()
        .find(|a| a.hint.symbol.as_deref() == Some("AFT"))
        .unwrap();
    s.set_price_source(
        aft.id,
        Some(&PriceSourceKind::Tefas {
            fund_code: "AFT".into(),
        }),
    )
    .unwrap();
    let pension = s
        .list_accounts()
        .unwrap()
        .into_iter()
        .find(|a| a.hints.contains(&"AH1".to_string()))
        .unwrap();
    s.set_tax_treatment(pension.id, TaxTreatment::Pension)
        .unwrap();

    let mut c = HistoryCache::open_in_memory().unwrap();
    rate(&mut c, "USD", "2023-03-01", "18.90");
    rate(&mut c, "USD", "2025-06-10", "39.00");
    rate(&mut c, "USD", "2025-05-15", "38.80");
    rate(&mut c, "USD", "2025-01-06", "35.40");
    for (date, v) in [
        ("2023-03-01", "22.70"),
        ("2025-01-06", "44.25"),
        ("2025-05-15", "51.60"),
        ("2025-06-10", "52.65"),
    ] {
        rate(&mut c, "GBP", date, v);
    }
    c.commit_inflation(
        "evds",
        "TP.TUFE1YI.T1",
        &[
            CachedIndexValue {
                month: "2023-02".into(),
                value: "1000.00".into(),
            },
            CachedIndexValue {
                month: "2025-05".into(),
                value: "1800.00".into(),
            },
        ],
        true,
        "t",
    )
    .unwrap();
    (s, c)
}

#[test]
fn turkish_report_from_a_database() {
    let (s, c) = store_and_cache();
    let inputs = TaxInputs::load(&s, Some(&c)).unwrap();
    let r = report(
        &inputs,
        Country::Tr,
        TaxYear::parse(Country::Tr, "2025").unwrap(),
        "2026-11-02T10:00:00Z",
    );
    let declared = &r.gains[0];
    assert_eq!(declared.rows.len(), 2);
    let row = &declared.rows[0];
    assert_eq!(row.description, "Apple Inc. (AAPL)");
    assert_eq!(
        (row.cost.as_deref(), row.indexed_cost.as_deref()),
        (Some("28350.00"), Some("51030.00"))
    );
    assert_eq!(
        (row.proceeds.as_deref(), row.gain.as_deref()),
        (Some("85800.00"), Some("34770.00"))
    );
    // The TEFAS fund is taxed at source; the pension fund is left out.
    assert_eq!(r.gains[1].id, "taxed_at_source");
    assert_eq!(r.gains[1].rows.len(), 1);
    assert_eq!(r.gains[1].rows[0].symbol.as_deref(), Some("AFT"));
    assert_eq!(r.excluded.len(), 1);
    assert_eq!(r.excluded[0].treatment, "pension");
    assert_eq!(
        r.rate_source,
        "TCMB forex buying rates on each transaction date"
    );
}

#[test]
fn years_skip_pension_accounts() {
    let (s, c) = store_and_cache();
    let inputs = TaxInputs::load(&s, Some(&c)).unwrap();
    let y = years(&inputs, Country::Uk, Date::parse("2026-11-02").unwrap());
    let labels: Vec<&str> = y.years.iter().map(|x| x.label.as_str()).collect();
    assert_eq!(labels, ["2025-26", "2024-25"]);
    assert_eq!(y.default.as_deref(), Some("2025-26"));
}

/// Compare with a committed CSV; `MIZAN_BLESS=1` rewrites it.
fn golden(name: &str, body: &str) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tax")
        .join(name);
    if std::env::var_os("MIZAN_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, body).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        body, expected,
        "{name} differs; regenerate with MIZAN_BLESS=1 cargo test -p mizan-core --test tax"
    );
}

#[test]
fn csv_exports_match_golden_files() {
    use mizan_core::tax::csv::{file_names, gains_csv, income_csv};
    let (s, c) = store_and_cache();
    let inputs = TaxInputs::load(&s, Some(&c)).unwrap();
    for (country, year) in [
        (Country::Uk, "2025-26"),
        (Country::Us, "2025"),
        (Country::Tr, "2025"),
    ] {
        let r = report(
            &inputs,
            country,
            TaxYear::parse(country, year).unwrap(),
            "2026-11-02T10:00:00Z",
        );
        let (g, i) = file_names(&r);
        assert!(g.starts_with(&format!("mizan-{}-{year}", country.as_str())));
        let gains = gains_csv(&r);
        let income = income_csv(&r);
        assert!(gains.ends_with("\r\n") && income.lines().count() >= 2);
        golden(&g, &gains);
        golden(&i, &income);
    }
}
