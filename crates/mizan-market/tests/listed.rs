//! Yahoo Finance and Twelve Data: parsing, refresh, errors and the API key.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use mizan_import::{Date, InstrumentHint};
use mizan_market::listed::{Close, ListedError};
use mizan_market::*;
use mizan_storage::{HistoryCache, PriceSourceKind, Store};

fn fixture(path: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/{path}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn day(s: &str) -> Date {
    Date::parse(s).unwrap()
}

const KEY: &str = "td-secret-key-3f9a";

#[derive(Debug, Clone)]
struct Call {
    at: u64,
    url: String,
    headers: Vec<(String, String)>,
}

/// Serves queued responses per URL prefix and records calls, headers and a simulated clock.
#[derive(Default)]
struct Fake {
    queue: Mutex<Vec<(String, VecDeque<Response>)>>,
    calls: Mutex<Vec<Call>>,
    clock: Mutex<u64>,
}

impl Fake {
    fn on(self, url: &str, status: u16, body: &str) -> Self {
        {
            let mut q = self.queue.lock().unwrap();
            let r = Response {
                status,
                body: body.into(),
            };
            match q.iter_mut().find(|(u, _)| u == url) {
                Some((_, v)) => v.push_back(r),
                None => q.push((url.into(), VecDeque::from([r]))),
            }
        }
        self
    }
    fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }
    fn serve(&self, url: &str, headers: &[(&str, &str)]) -> Result<Response, HttpError> {
        self.calls.lock().unwrap().push(Call {
            at: *self.clock.lock().unwrap(),
            url: url.into(),
            headers: headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        });
        let mut q = self.queue.lock().unwrap();
        q.iter_mut()
            .find(|(u, _)| u == url)
            .and_then(|(_, v)| v.pop_front())
            .ok_or_else(|| HttpError(format!("no response queued for {url}")))
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
    fn pause(&self, d: Duration) {
        *self.clock.lock().unwrap() += d.as_secs();
    }
}

fn shown(c: &[Close]) -> Vec<(String, String, String)> {
    c.iter()
        .map(|c| {
            (
                c.date.to_string(),
                c.value.to_string(),
                c.currency.to_string(),
            )
        })
        .collect()
}

// ---- Yahoo parsing ----

#[test]
fn yahoo_undoes_the_split_adjustment() {
    let closes = yahoo::parse("AAPL", 200, &fixture("yahoo/aapl_split_2020.json")).unwrap();
    let get = |d: &str| closes.iter().find(|c| c.date == day(d)).unwrap();
    // Yahoo reports 124.80750274658203 (adjusted); traded close was 4 × that.
    assert_eq!(get("2020-08-28").value.to_string(), "499.23");
    assert_eq!(get("2020-08-28").currency.as_str(), "USD");
    // On and after the split day, unchanged (rounded to priceHint 2).
    assert_eq!(get("2020-08-31").value.to_string(), "129.04");
    assert!(closes.windows(2).all(|w| w[0].date < w[1].date));
    assert_eq!(closes.first().unwrap().date, day("2020-08-24"));
}

#[test]
fn yahoo_european_listing_in_eur_with_trading_dates() {
    let closes = yahoo::parse("VWCE.DE", 200, &fixture("yahoo/vwce_de_5d.json")).unwrap();
    assert_eq!(closes.len(), 5);
    assert!(closes.iter().all(|c| c.currency.as_str() == "EUR"));
    let last = closes.last().unwrap();
    assert_eq!(last.value.to_string(), "168.9");
    assert!(
        !matches!(last.date.0.format("%a").to_string().as_str(), "Sat" | "Sun"),
        "a trading day"
    );
}

#[test]
fn yahoo_pence_become_pounds() {
    let closes = yahoo::parse("VOD.L", 200, &fixture("yahoo/vod_l_5d.json")).unwrap();
    let last = closes.last().unwrap();
    assert_eq!(last.currency.as_str(), "GBP");
    // 119.0999984741211 GBp, rounded to priceHint 2, then divided by 100.
    assert_eq!(last.value.to_string(), "1.191");
}

#[test]
fn yahoo_errors() {
    assert_eq!(
        yahoo::parse("NOSUCHXYZ", 404, &fixture("yahoo/not_found.json")),
        Err(ListedError::NotFound("NOSUCHXYZ".into()))
    );
    assert_eq!(
        ListedError::NotFound("NOSUCHXYZ".into()).to_string(),
        "NOSUCHXYZ: symbol not found"
    );
    assert_eq!(
        yahoo::parse("AAPL", 429, "Too Many Requests"),
        Err(ListedError::RateLimited)
    );
    assert!(matches!(
        yahoo::parse("AAPL", 200, "<html>"),
        Err(ListedError::Parse(_))
    ));
}

#[test]
fn yahoo_urls() {
    assert_eq!(
        yahoo::latest_url("VWCE.DE"),
        "https://query1.finance.yahoo.com/v8/finance/chart/VWCE.DE?range=1mo&interval=1d&events=split"
    );
    // From 2021-01-01 through 2026-11-02 inclusive (period2 is the next midnight UTC).
    assert_eq!(
        yahoo::span_url("AAPL", day("2021-01-01"), day("2026-11-02")),
        "https://query1.finance.yahoo.com/v8/finance/chart/AAPL?period1=1609459200&period2=1793664000&interval=1d&events=split"
    );
}

// ---- Twelve Data parsing ----

#[test]
fn twelvedata_parses_unadjusted_closes() {
    let closes = twelvedata::parse("AAPL", 200, &fixture("twelvedata/aapl_5d.json")).unwrap();
    assert_eq!(
        shown(&closes)[1..3],
        [
            ("2026-10-27".into(), "347.1".into(), "USD".into()),
            ("2026-10-28".into(), "343.83".into(), "USD".into()),
        ]
    );
    assert_eq!(closes.last().unwrap().value.to_string(), "344.63");
}

#[test]
fn twelvedata_errors() {
    assert_eq!(
        twelvedata::parse("AAPL", 200, &fixture("twelvedata/error_key.json")),
        Err(ListedError::KeyRejected)
    );
    assert_eq!(
        twelvedata::parse("AAPL", 200, &fixture("twelvedata/error_limit.json")),
        Err(ListedError::RateLimited)
    );
    assert_eq!(
        twelvedata::parse("NOSUCHXYZ", 200, &fixture("twelvedata/error_symbol.json")),
        Err(ListedError::NotFound("NOSUCHXYZ".into()))
    );
}

#[test]
fn twelvedata_spans_stay_under_the_point_limit() {
    assert_eq!(
        twelvedata::spans(day("2001-03-05"), day("2026-11-02")),
        vec![
            (day("2001-03-05"), day("2019-12-31")),
            (day("2020-01-01"), day("2026-11-02")),
        ]
    );
    assert_eq!(
        twelvedata::spans(day("2024-01-02"), day("2026-11-02")),
        vec![(day("2024-01-02"), day("2026-11-02"))]
    );
    assert_eq!(twelvedata::scrub(&format!("bad {KEY}!"), KEY), "bad •••!");
}

// ---- refresh ----

fn store_with(sources: &[(&str, PriceSourceKind)]) -> (Store, Vec<i64>) {
    let mut s = Store::open_in_memory().unwrap();
    for id in ["tefas", "tcmb"] {
        set_enabled(&mut s, id, false).unwrap();
    }
    let mut ids = Vec::new();
    for (key, src) in sources {
        let a = s
            .create_asset(
                key,
                &InstrumentHint {
                    symbol: Some(key.to_string()),
                    ..Default::default()
                },
            )
            .unwrap();
        s.set_price_source(a, Some(src)).unwrap();
        ids.push(a);
    }
    (s, ids)
}

fn yahoo_src(sym: &str) -> PriceSourceKind {
    PriceSourceKind::Yahoo { symbol: sym.into() }
}

fn td_src(sym: &str) -> PriceSourceKind {
    PriceSourceKind::TwelveData { symbol: sym.into() }
}

fn run(s: &mut Store, cache: Option<&mut HistoryCache>, http: &Fake) -> RefreshReport {
    refresh(
        s,
        cache,
        http,
        day("2026-11-02"),
        "2026-11-02T10:00:00Z",
        || "2026-11-02T10:00:09Z".into(),
    )
    .unwrap()
}

fn source<'a>(r: &'a RefreshReport, id: &str) -> &'a SourceReport {
    r.sources.iter().find(|x| x.id == id).unwrap()
}

#[test]
fn yahoo_refresh_stores_latest_and_caches_closes() {
    let (mut s, ids) = store_with(&[("sym:VWCE@XETR", yahoo_src("VWCE.DE"))]);
    let http = Fake::default().on(
        &yahoo::latest_url("VWCE.DE"),
        200,
        &fixture("yahoo/vwce_de_5d.json"),
    );
    let mut cache = HistoryCache::open_in_memory().unwrap();
    let rep = run(&mut s, Some(&mut cache), &http);
    let y = source(&rep, "yahoo");
    assert_eq!((y.status, y.updated), (SourceStatus::Ok, 1));
    let prices = s.prices_for(ids[0]).unwrap();
    assert_eq!(prices.len(), 1);
    assert_eq!(
        (
            prices[0].source.as_str(),
            prices[0].value.as_str(),
            prices[0].currency.as_str()
        ),
        ("yahoo", "168.9", "EUR")
    );
    assert_eq!(cache.listed_prices("yahoo", "VWCE.DE").unwrap().len(), 5);
    assert!(
        cache.fetched("yahoo", "VWCE.DE").unwrap().is_empty(),
        "refresh records no years"
    );
}

#[test]
fn no_assigned_asset_or_disabled_provider_makes_no_request() {
    let (mut s, _) = store_with(&[]);
    let http = Fake::default();
    let rep = run(&mut s, None, &http);
    assert_eq!(source(&rep, "yahoo").status, SourceStatus::Idle);
    assert_eq!(source(&rep, "twelvedata").status, SourceStatus::Idle);

    let (mut s, ids) = store_with(&[("sym:AAPL@", yahoo_src("AAPL"))]);
    set_enabled(&mut s, "yahoo", false).unwrap();
    let rep = run(&mut s, None, &http);
    assert_eq!(source(&rep, "yahoo").status, SourceStatus::Disabled);
    assert!(http.calls().is_empty());
    assert!(s.prices_for(ids[0]).unwrap().is_empty());
}

#[test]
fn unknown_symbol_is_isolated_and_requests_are_spaced() {
    let (mut s, ids) = store_with(&[
        ("sym:AAPL@", yahoo_src("AAPL")),
        ("sym:NOSUCHXYZ@", yahoo_src("NOSUCHXYZ")),
        ("sym:VWCE@XETR", yahoo_src("VWCE.DE")),
    ]);
    let http = Fake::default()
        .on(
            &yahoo::latest_url("AAPL"),
            200,
            &fixture("yahoo/aapl_split_2020.json"),
        )
        .on(
            &yahoo::latest_url("NOSUCHXYZ"),
            404,
            &fixture("yahoo/not_found.json"),
        )
        .on(
            &yahoo::latest_url("VWCE.DE"),
            200,
            &fixture("yahoo/vwce_de_5d.json"),
        );
    let rep = run(&mut s, None, &http);
    let y = source(&rep, "yahoo");
    assert_eq!((y.status, y.updated), (SourceStatus::Ok, 2));
    assert_eq!(y.messages, vec!["NOSUCHXYZ: symbol not found".to_string()]);
    assert_eq!(s.prices_for(ids[0]).unwrap().len(), 1);
    let times: Vec<u64> = http.calls().iter().map(|c| c.at).collect();
    assert_eq!(times, vec![0, 2, 4], "at least 2 seconds apart");
    assert!(http.calls().iter().all(|c| c.headers.is_empty()));
}

#[test]
fn twelvedata_without_key_needs_a_key_and_makes_no_request() {
    let (mut s, _) = store_with(&[("sym:MSFT@", td_src("MSFT"))]);
    let http = Fake::default();
    let rep = run(&mut s, None, &http);
    let t = source(&rep, "twelvedata");
    assert_eq!(t.status, SourceStatus::NeedsKey);
    assert_eq!(t.messages, vec!["needs an API key".to_string()]);
    assert!(!rep.any_failed());
    assert!(http.calls().is_empty());
}

#[test]
fn twelvedata_key_goes_in_a_header_and_never_into_reports() {
    let (mut s, ids) = store_with(&[("sym:AAPL@", td_src("AAPL"))]);
    set_api_key(&mut s, "twelvedata", Some(&format!("  {KEY} "))).unwrap();
    assert_eq!(api_key_hint(&s, "twelvedata").as_deref(), Some("••••3f9a"));
    let http = Fake::default().on(
        &twelvedata::latest_url("AAPL"),
        200,
        &fixture("twelvedata/aapl_5d.json"),
    );
    let mut cache = HistoryCache::open_in_memory().unwrap();
    let rep = run(&mut s, Some(&mut cache), &http);
    assert_eq!(source(&rep, "twelvedata").status, SourceStatus::Ok);
    assert_eq!(s.prices_for(ids[0]).unwrap()[0].value, "344.63");
    let call = &http.calls()[0];
    assert!(!call.url.contains(KEY));
    assert_eq!(
        call.headers,
        vec![("Authorization".to_string(), format!("apikey {KEY}"))]
    );
    let logged = serde_json::to_string(&rep).unwrap();
    assert!(!logged.contains(KEY));
    assert!(cache
        .all_text_values()
        .unwrap()
        .iter()
        .all(|v| !v.contains(KEY)));
}

#[test]
fn rejected_key_and_rate_limit_stop_only_that_provider() {
    let (mut s, _) = store_with(&[
        ("sym:AAPL@", td_src("AAPL")),
        ("sym:MSFT@", td_src("MSFT")),
        ("sym:VWCE@XETR", yahoo_src("VWCE.DE")),
    ]);
    set_api_key(&mut s, "twelvedata", Some(KEY)).unwrap();
    let http = Fake::default()
        .on(
            &twelvedata::latest_url("AAPL"),
            200,
            &fixture("twelvedata/error_key.json"),
        )
        .on(
            &yahoo::latest_url("VWCE.DE"),
            200,
            &fixture("yahoo/vwce_de_5d.json"),
        );
    let rep = run(&mut s, None, &http);
    let t = source(&rep, "twelvedata");
    assert_eq!(t.status, SourceStatus::Failed);
    assert_eq!(t.messages[0], "Twelve Data: API key rejected");
    assert!(
        !http.calls().iter().any(|c| c.url.contains("MSFT")),
        "stops after the rejected key"
    );
    assert_eq!(source(&rep, "yahoo").status, SourceStatus::Ok);

    let http = Fake::default()
        .on(
            &twelvedata::latest_url("AAPL"),
            429,
            &fixture("twelvedata/error_limit.json"),
        )
        .on(
            &yahoo::latest_url("VWCE.DE"),
            200,
            &fixture("yahoo/vwce_de_5d.json"),
        );
    let rep = run(&mut s, None, &http);
    assert_eq!(
        source(&rep, "twelvedata").messages[0],
        "Twelve Data: request limit reached"
    );
    assert_eq!(source(&rep, "yahoo").status, SourceStatus::Ok);
}

#[test]
fn api_key_helpers() {
    let mut s = Store::open_in_memory().unwrap();
    assert_eq!(api_key(&s, "twelvedata"), None);
    assert!(set_api_key(&mut s, "yahoo", Some("x")).is_err());
    assert!(set_api_key(&mut s, "twelvedata", Some("a b")).is_err());
    set_api_key(&mut s, "twelvedata", Some(KEY)).unwrap();
    assert_eq!(api_key(&s, "twelvedata").as_deref(), Some(KEY));
    set_api_key(&mut s, "twelvedata", None).unwrap();
    assert_eq!(api_key(&s, "twelvedata"), None);
    assert_eq!(api_key_hint(&s, "twelvedata"), None);
}
