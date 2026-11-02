//! EVDS producer price index: parsing, the request contract and the refresh
//! step (market-data spec, "EVDS producer price index").

use std::sync::Mutex;
use std::time::Duration;

use mizan_core::*;
use mizan_import::{parse_batch_value, Date};
use mizan_market::inflation::{InflationError, YearMonth};
use mizan_market::*;
use mizan_storage::{CachedIndexValue, HistoryCache, Store};

const KEY: &str = "evds-private-key-41d9";
const NOW: &str = "2026-11-02T10:00:00Z";

fn fixture(path: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/evds/{path}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn ym(s: &str) -> YearMonth {
    YearMonth::parse(s).unwrap()
}

fn day(s: &str) -> Date {
    Date::parse(s).unwrap()
}

/// A recorded call: URL and request headers.
type Call = (String, Vec<(String, String)>);

/// Serves one response for EVDS and 404 for everything else; records each
/// call's URL and headers.
struct Fake {
    status: u16,
    body: String,
    calls: Mutex<Vec<Call>>,
}

impl Fake {
    fn new(status: u16, body: &str) -> Self {
        Fake {
            status,
            body: body.into(),
            calls: Mutex::new(vec![]),
        }
    }
    fn evds_calls(&self) -> Vec<Call> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(u, _)| u.starts_with(evds::BASE))
            .cloned()
            .collect()
    }
    fn serve(&self, url: &str, headers: &[(&str, &str)]) -> Result<Response, HttpError> {
        self.calls.lock().unwrap().push((
            url.into(),
            headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        ));
        Ok(if url.starts_with(evds::BASE) {
            Response {
                status: self.status,
                body: self.body.clone(),
            }
        } else {
            Response {
                status: 404,
                body: String::new(),
            }
        })
    }
}

impl Http for Fake {
    fn get(&self, url: &str) -> Result<Response, HttpError> {
        self.serve(url, &[])
    }
    fn get_with_headers(&self, url: &str, headers: &[(&str, &str)]) -> Result<Response, HttpError> {
        self.serve(url, headers)
    }
    fn post_json(&self, url: &str, _body: &str) -> Result<Response, HttpError> {
        self.serve(url, &[])
    }
    fn pause(&self, _d: Duration) {}
}

const CREATE: CreatePolicy = CreatePolicy {
    create_accounts: true,
    create_assets: true,
};

/// A database whose first activity is 2025-03-05.
fn pension_store() -> Store {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../adapters/anadolu-hayat/fixtures/basic/expected.json"
    );
    let batch: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut store = Store::open_in_memory().unwrap();
    let parsed = parse_batch_value(batch).unwrap();
    let rb = {
        let mut r = MappingResolver {
            store: &mut store,
            policy: CREATE,
        };
        resolve_batch(parsed, &mut r).unwrap()
    };
    let p = prepare(&store, rb).unwrap();
    commit(&mut store, &p, &Decisions::new()).unwrap();
    store
}

fn run(s: &mut Store, cache: &mut HistoryCache, http: &Fake, now: &str) -> RefreshReport {
    refresh(s, Some(cache), http, day("2026-11-02"), now, || now.into()).unwrap()
}

fn evds_report(rep: &RefreshReport) -> &SourceReport {
    rep.sources.iter().find(|s| s.id == "evds").unwrap()
}

#[test]
fn parses_monthly_values_and_skips_unpublished_months() {
    let v = evds::parse(&fixture("yiufe.json"), evds::SERIES.id).unwrap();
    assert_eq!(v.len(), 6);
    assert_eq!(v[0].month, ym("2025-02"));
    assert_eq!(v[5].month, ym("2025-07"));
    assert_eq!(v[5].value.to_string(), "4129.91");
    let numeric = r#"{"items":[{"Tarih":"2024-12","TP_TUFE1YI_T1":3500.5}]}"#;
    assert_eq!(
        evds::parse(numeric, evds::SERIES.id).unwrap()[0].month,
        ym("2024-12")
    );
}

#[test]
fn unknown_series_and_error_bodies() {
    let other = evds::parse(&fixture("yiufe.json"), "TP.XX.YY").unwrap_err();
    assert!(other.to_string().contains("no values"), "{other}");
    assert_eq!(
        evds::parse(&fixture("no_key.json"), evds::SERIES.id),
        Err(InflationError::Api(
            "Required request header 'key' is not present".into()
        ))
    );
}

#[test]
fn request_sends_the_key_only_in_a_header() {
    let http = Fake::new(200, &fixture("yiufe.json"));
    evds::fetch(&http, KEY, evds::SERIES.id, ym("2025-02"), ym("2025-07")).unwrap();
    let calls = http.evds_calls();
    assert_eq!(calls.len(), 1);
    let (url, headers) = &calls[0];
    assert_eq!(
        url,
        &format!(
            "{}series=TP.TUFE1YI.T1&startDate=01-02-2025&endDate=28-07-2025&type=json",
            evds::BASE
        )
    );
    assert!(!url.contains(KEY));
    assert_eq!(headers, &[("key".to_string(), KEY.to_string())]);
}

#[test]
fn no_key_means_no_request_and_needs_an_api_key() {
    let mut s = pension_store();
    let mut cache = HistoryCache::open_in_memory().unwrap();
    let http = Fake::new(200, &fixture("yiufe.json"));
    let rep = run(&mut s, &mut cache, &http, NOW);
    assert!(http.evds_calls().is_empty());
    let r = evds_report(&rep);
    assert_eq!(r.status, SourceStatus::NeedsKey);
    assert_eq!(r.messages, ["needs an API key"]);
}

#[test]
fn first_fetch_caches_from_the_month_before_the_first_activity() {
    let mut s = pension_store();
    set_api_key(&mut s, "evds", Some(KEY)).unwrap();
    let mut cache = HistoryCache::open_in_memory().unwrap();
    let http = Fake::new(200, &fixture("yiufe.json"));
    let rep = run(&mut s, &mut cache, &http, NOW);
    let calls = http.evds_calls();
    assert_eq!(calls.len(), 1);
    assert!(
        calls[0].0.contains("startDate=01-02-2025"),
        "{}",
        calls[0].0
    );
    let r = evds_report(&rep);
    assert_eq!(r.status, SourceStatus::Ok);
    assert_eq!(r.messages, ["Yİ-ÜFE through Jul 2025"]);
    let cached = cache.inflation_index("evds", "TP.TUFE1YI.T1").unwrap();
    assert_eq!(cached.len(), 6);

    // The key is in neither the cache nor the stored report.
    let dump = format!(
        "{:?}{:?}",
        cache.inflation_index("evds", "TP.TUFE1YI.T1").unwrap(),
        rep
    );
    assert!(!dump.contains(KEY));
}

#[test]
fn up_to_date_and_recently_checked_series_make_no_request() {
    let mut s = pension_store();
    set_api_key(&mut s, "evds", Some(KEY)).unwrap();
    let mut cache = HistoryCache::open_in_memory().unwrap();
    let mut rows = vec![];
    let mut m = ym("2025-01");
    while m <= ym("2026-10") {
        rows.push(CachedIndexValue {
            month: m.to_string(),
            value: "4000".into(),
        });
        m = m.shift(1);
    }
    cache
        .commit_inflation("evds", "TP.TUFE1YI.T1", &rows, true, "2026-10-01T00:00:00Z")
        .unwrap();
    let http = Fake::new(200, &fixture("yiufe.json"));
    let rep = run(&mut s, &mut cache, &http, NOW);
    assert!(http.evds_calls().is_empty());
    assert_eq!(evds_report(&rep).status, SourceStatus::Idle);
    assert_eq!(
        evds_report(&rep).messages,
        ["Yİ-ÜFE: up to date through Oct 2026"]
    );
}

#[test]
fn rejected_key_keeps_cached_values_and_other_sources() {
    let mut s = pension_store();
    set_api_key(&mut s, "evds", Some(KEY)).unwrap();
    let mut cache = HistoryCache::open_in_memory().unwrap();
    cache
        .commit_inflation(
            "evds",
            "TP.TUFE1YI.T1",
            &[CachedIndexValue {
                month: "2025-02".into(),
                value: "3712.59".into(),
            }],
            true,
            "2026-10-01T00:00:00Z",
        )
        .unwrap();
    let http = Fake::new(403, &fixture("no_key.json"));
    let rep = run(&mut s, &mut cache, &http, NOW);
    let r = evds_report(&rep);
    assert_eq!(r.status, SourceStatus::Failed);
    assert_eq!(r.messages, ["EVDS: API key rejected"]);
    assert_eq!(
        cache
            .inflation_index("evds", "TP.TUFE1YI.T1")
            .unwrap()
            .len(),
        1
    );
    assert!(
        !cache
            .inflation_check("evds", "TP.TUFE1YI.T1")
            .unwrap()
            .unwrap()
            .ok
    );
    assert_eq!(rep.sources.len(), 8, "every provider still reports");
}

#[test]
fn disabled_evds_makes_no_request() {
    let mut s = pension_store();
    set_api_key(&mut s, "evds", Some(KEY)).unwrap();
    set_enabled(&mut s, "evds", false).unwrap();
    let mut cache = HistoryCache::open_in_memory().unwrap();
    let http = Fake::new(200, &fixture("yiufe.json"));
    let rep = run(&mut s, &mut cache, &http, NOW);
    assert!(http.evds_calls().is_empty());
    assert_eq!(evds_report(&rep).status, SourceStatus::Disabled);
}
