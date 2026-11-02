//! The history cache holds public market data only (price-history spec).

use std::sync::atomic::AtomicBool;
use std::time::Duration;

use mizan_core::*;
use mizan_import::{parse_batch_value, Date};
use mizan_market::backfill::{self, PlanInput};
use mizan_market::*;
use mizan_storage::{HistoryCache, PriceSourceKind, Store};

const CREATE: CreatePolicy = CreatePolicy {
    create_accounts: true,
    create_assets: true,
};

struct Fake;

impl Http for Fake {
    fn get(&self, url: &str) -> Result<Response, HttpError> {
        assert!(
            !url.contains(KEY) && !url.contains(EVDS_KEY),
            "an API key went into a URL: {url}"
        );
        if url.starts_with(evds::BASE) {
            return Ok(Response {
                status: 200,
                body: std::fs::read_to_string(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/fixtures/evds/yiufe.json"
                ))
                .unwrap(),
            });
        }
        if url.starts_with(yahoo::BASE) {
            return Ok(Response {
                status: 200,
                body: r#"{"chart":{"result":[{"meta":{"currency":"USD","priceHint":2,"gmtoffset":-14400},"timestamp":[1749562200],"indicators":{"quote":[{"close":[600.25]}]}}],"error":null}}"#.into(),
            });
        }
        if url.starts_with(twelvedata::BASE) {
            return Ok(Response {
                status: 200,
                body: r#"{"meta":{"symbol":"MSFT","currency":"USD"},"values":[{"datetime":"2025-06-09","close":"470.38000"}],"status":"ok"}"#.into(),
            });
        }
        if url == tcmb::TODAY_URL {
            return Ok(Response {
                status: 404,
                body: String::new(),
            });
        }
        let file = url.rsplit('/').next().unwrap().trim_end_matches(".xml");
        let (d, m, y) = (&file[..2], &file[2..4], &file[4..8]);
        Ok(Response {
            status: 200,
            body: format!(
                r#"<Tarih_Date Tarih="{d}.{m}.{y}"><Currency CurrencyCode="USD"><Unit>1</Unit><ForexBuying>34.5</ForexBuying></Currency></Tarih_Date>"#
            ),
        })
    }
    fn post_json(&self, url: &str, body: &str) -> Result<Response, HttpError> {
        if url == bls::URL {
            return Ok(Response {
                status: 200,
                body: std::fs::read_to_string(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/fixtures/bls/cpi_2017_2026.json"
                ))
                .unwrap(),
            });
        }
        let v: serde_json::Value = serde_json::from_str(body).unwrap();
        let from = v["basTarih"].as_str().unwrap();
        let date = format!("{}-{}-{}", &from[..4], &from[4..6], &from[6..8]);
        let codes: Vec<String> = match v["fonKodu"].as_str() {
            Some(c) => vec![c.to_string()],
            None => ["ABE", "AEA", "AET", "AH2", "AH5", "BGL", "GMF"]
                .map(String::from)
                .to_vec(),
        };
        let rows: Vec<String> = codes
            .iter()
            .map(|c| format!(r#"{{"fonKodu":"{c}","tarih":"{date}","fiyat":0.731}}"#))
            .collect();
        Ok(Response {
            status: 200,
            body: format!(
                r#"{{"errorMessage":null,"resultList":[{}]}}"#,
                rows.join(",")
            ),
        })
    }
    fn pause(&self, _d: Duration) {}
}

fn pension_store() -> (Store, serde_json::Value) {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../adapters/anadolu-hayat/fixtures/basic/expected.json"
    );
    let batch: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut store = Store::open_in_memory().unwrap();
    let parsed = parse_batch_value(batch.clone()).unwrap();
    assert!(parsed.is_clean(), "{:?}", parsed.errors);
    let rb = {
        let mut r = MappingResolver {
            store: &mut store,
            policy: CREATE,
        };
        resolve_batch(parsed, &mut r).unwrap()
    };
    let p = prepare(&store, rb).unwrap();
    commit(&mut store, &p, &Decisions::new()).unwrap();
    for a in store.list_assets().unwrap() {
        let code = a.hint.symbol.clone().unwrap();
        store
            .set_price_source(a.id, Some(&PriceSourceKind::Tefas { fund_code: code }))
            .unwrap();
    }
    (store, batch)
}

#[test]
fn plan_input_reads_codes_and_first_dates_from_the_database() {
    let (store, _) = pension_store();
    let inp = PlanInput::from_store(&store).unwrap();
    assert_eq!(inp.tefas.len(), 7);
    assert_eq!(inp.tefas["AH2"], Date::parse("2025-03-05").unwrap());
    assert_eq!(
        inp.fx_from, None,
        "a TRY-only pension needs no exchange rates"
    );
    assert!(inp.tefas_enabled && inp.tcmb_enabled);
}

#[test]
fn base_currency_other_than_try_needs_rates_from_the_first_activity() {
    let (mut store, _) = pension_store();
    store.set_setting("base_currency", "EUR").unwrap();
    let inp = PlanInput::from_store(&store).unwrap();
    assert_eq!(inp.fx_from, Some(Date::parse("2025-03-05").unwrap()));
}

#[test]
fn uk_or_us_tax_report_needs_rates_from_the_first_activity() {
    let (mut store, _) = pension_store();
    store.set_setting(backfill::TAX_RATES_KEY, "true").unwrap();
    let inp = PlanInput::from_store(&store).unwrap();
    assert_eq!(inp.fx_from, Some(Date::parse("2025-03-05").unwrap()));
}

const KEY: &str = "td-private-key-7c21";
const EVDS_KEY: &str = "evds-private-key-41d9";

#[test]
fn cache_contains_no_account_or_activity_data() {
    let (mut store, batch) = pension_store();
    store.set_setting("base_currency", "USD").unwrap();
    // Two of the assets priced as listed instruments, one needing an API key.
    let assets = store.list_assets().unwrap();
    store
        .set_price_source(
            assets[0].id,
            Some(&PriceSourceKind::Yahoo {
                symbol: "SPY".into(),
            }),
        )
        .unwrap();
    store
        .set_price_source(
            assets[1].id,
            Some(&PriceSourceKind::TwelveData {
                symbol: "MSFT".into(),
            }),
        )
        .unwrap();
    set_api_key(&mut store, "twelvedata", Some(KEY)).unwrap();
    set_api_key(&mut store, "evds", Some(EVDS_KEY)).unwrap();
    let mut cache = HistoryCache::open_in_memory().unwrap();
    let today = Date::parse("2025-06-10").unwrap();
    refresh(
        &mut store,
        Some(&mut cache),
        &Fake,
        today,
        "2025-06-10T10:00:00Z",
        || "2025-06-10T10:00:01Z".into(),
    )
    .unwrap();
    let plan = backfill::plan(&PlanInput::from_store(&store).unwrap(), &cache, today).unwrap();
    assert!(!plan.tefas.is_empty() && !plan.tcmb.is_empty());
    assert_eq!(plan.listed.len(), 2, "SPY and MSFT");
    backfill::run(
        &plan,
        &mut cache,
        &Fake,
        &AtomicBool::new(false),
        &|| "t".into(),
        &mut |_| {},
    );

    let columns: Vec<(String, Vec<String>)> = cache.schema_columns().unwrap();
    let expected = [
        (
            "fetched",
            vec!["source", "key", "period", "status", "fetched_at"],
        ),
        ("fund_prices", vec!["code", "date", "price", "fetched_at"]),
        ("fund_types", vec!["code", "fund_type"]),
        (
            "fx_rates",
            vec!["currency", "date", "try_per_unit", "source", "fetched_at"],
        ),
        (
            "inflation_checks",
            vec!["source", "series", "checked_at", "status"],
        ),
        (
            "inflation_index",
            vec!["source", "series", "month", "value", "fetched_at"],
        ),
        (
            "listed_prices",
            vec![
                "source",
                "symbol",
                "date",
                "close",
                "currency",
                "fetched_at",
            ],
        ),
    ];
    assert_eq!(columns.len(), expected.len());
    for ((t, cols), (et, ecols)) in columns.iter().zip(expected) {
        assert_eq!(t, et);
        assert_eq!(cols, &ecols);
    }

    let values = cache.all_text_values().unwrap();
    assert!(!values.is_empty());
    assert!(values.iter().any(|v| v == "SPY") && values.iter().any(|v| v == "MSFT"));
    assert!(!cache.listed_prices("yahoo", "SPY").unwrap().is_empty());
    // The base currency's inflation index was fetched into the cache.
    assert!(!cache
        .inflation_index("bls", "CUUR0000SA0")
        .unwrap()
        .is_empty());
    // The Turkish producer price index was fetched with the EVDS key.
    assert!(!cache
        .inflation_index("evds", "TP.TUFE1YI.T1")
        .unwrap()
        .is_empty());
    let mut private: Vec<String> = vec![
        "9000001".into(),
        "Anadolu".into(),
        KEY.into(),
        EVDS_KEY.into(),
    ];
    for r in batch["records"].as_array().unwrap() {
        if let Some(q) = r["quantity"].as_str() {
            private.push(q.into());
        }
        if let Some(a) = r["net_cash"]["amount"].as_str() {
            private.push(a.into());
        }
        if let Some(n) = r["instrument"]["name"].as_str() {
            private.push(n.into());
        }
        private.push(r["source"]["file_sha256"].as_str().unwrap().into());
    }
    for v in &values {
        for p in &private {
            assert!(
                !v.contains(p.as_str()),
                "cache value {v:?} contains private data {p:?}"
            );
        }
    }
}

#[test]
fn benchmark_fund_is_backfilled_from_the_first_activity_even_when_not_held() {
    let (mut store, _) = pension_store();
    let other = store
        .create_asset(
            "sym:TCD@",
            &mizan_import::InstrumentHint {
                symbol: Some("TCD".into()),
                ..Default::default()
            },
        )
        .unwrap();
    store
        .set_price_source(
            other,
            Some(&PriceSourceKind::Tefas {
                fund_code: "TCD".into(),
            }),
        )
        .unwrap();
    assert!(
        !PlanInput::from_store(&store)
            .unwrap()
            .tefas
            .contains_key("TCD"),
        "not held, not the benchmark"
    );
    store
        .set_setting(backfill::BENCHMARK_KEY, &other.to_string())
        .unwrap();
    let inp = PlanInput::from_store(&store).unwrap();
    assert_eq!(inp.tefas["TCD"], Date::parse("2025-03-05").unwrap());
}

#[test]
fn listed_benchmark_is_backfilled_from_the_first_activity_even_when_not_held() {
    let (mut store, _) = pension_store();
    let spy = store
        .create_asset(
            "sym:SPY@ARCX",
            &mizan_import::InstrumentHint {
                symbol: Some("SPY".into()),
                ..Default::default()
            },
        )
        .unwrap();
    store
        .set_price_source(
            spy,
            Some(&PriceSourceKind::Yahoo {
                symbol: "spy".into(),
            }),
        )
        .unwrap();
    let key = ("yahoo".to_string(), "SPY".to_string());
    assert!(!PlanInput::from_store(&store)
        .unwrap()
        .listed
        .contains_key(&key));
    store
        .set_setting(backfill::BENCHMARK_KEY, &spy.to_string())
        .unwrap();
    let inp = PlanInput::from_store(&store).unwrap();
    assert_eq!(inp.listed[&key], Date::parse("2025-03-05").unwrap());
}

#[test]
fn nothing_to_backfill_without_activities() {
    let mut store = Store::open_in_memory().unwrap();
    let a = store
        .create_asset(
            "sym:AFT@",
            &mizan_import::InstrumentHint {
                symbol: Some("AFT".into()),
                ..Default::default()
            },
        )
        .unwrap();
    store
        .set_price_source(
            a,
            Some(&PriceSourceKind::Tefas {
                fund_code: "AFT".into(),
            }),
        )
        .unwrap();
    store
        .set_setting(backfill::BENCHMARK_KEY, &a.to_string())
        .unwrap();
    let inp = PlanInput::from_store(&store).unwrap();
    assert!(inp.tefas.is_empty() && inp.fx_from.is_none());
}
