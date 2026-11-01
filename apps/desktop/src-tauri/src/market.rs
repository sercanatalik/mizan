//! Market data commands: refresh, providers, settings and asset price sources.
//! Plain functions over [`AppState`] (tested) wrapped by thin Tauri commands.

use std::str::FromStr;
use std::sync::atomic::Ordering;

use chrono::{DateTime, Utc};
use mizan_core::assets::{assets_view as core_assets_view, tefas_suggestion, AssetView};
use mizan_import::{Code, Date};
use mizan_market::{Http, RefreshReport, SourceStatus, PROVIDERS};
use mizan_storage::{PriceSourceKind, Store};
use rust_decimal::Decimal;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::session::AppError;
use crate::state::AppState;

pub const EVENT_REFRESH_STARTED: &str = "market:refresh-started";
pub const EVENT_REFRESH_FINISHED: &str = "market:refresh-finished";
const REFRESH_ON_START: &str = "refresh_on_start";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ProviderView {
    pub id: String,
    pub name: String,
    pub supplies: String,
    pub enabled: bool,
    /// When this provider last refreshed successfully.
    pub last_success: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct SettingsView {
    pub base_currency: String,
    /// Currencies that can be chosen as base currency.
    pub currencies: Vec<String>,
    pub refresh_on_start: bool,
    pub last_refresh_at: Option<String>,
    pub providers: Vec<ProviderView>,
    pub last_report: Option<RefreshReport>,
    pub refreshing: bool,
}

fn locked<T>(_: T) -> AppError {
    AppError::Invalid("the database is unavailable".into())
}

fn invalid(msg: impl Into<String>) -> AppError {
    AppError::Invalid(msg.into())
}

// ---- refresh ----

/// Resets the single-flight flag when the refresh ends, even on error.
struct Running<'a>(&'a AppState);

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.0.refreshing.store(false, Ordering::SeqCst);
    }
}

pub fn refresh_prices(
    state: &AppState,
    http: &dyn Http,
    today: Date,
    now: impl Fn() -> String,
) -> Result<RefreshReport, AppError> {
    if state.refreshing.swap(true, Ordering::SeqCst) {
        return Err(invalid("a price refresh is already running"));
    }
    let _running = Running(state);
    let started = now();
    match &state.db_path {
        // Own connection: screens keep reading while the refresh waits on the network.
        Some(path) => {
            let mut store = Store::open(path)?;
            Ok(mizan_market::refresh(
                &mut store, http, today, &started, &now,
            )?)
        }
        None => {
            let mut store = state.store.lock().map_err(locked)?;
            Ok(mizan_market::refresh(
                &mut store, http, today, &started, &now,
            )?)
        }
    }
}

/// Refresh on start when enabled and the last successful refresh is over 24 hours old (or never happened).
pub fn should_refresh_on_start(
    enabled: bool,
    last_success: Option<&str>,
    now: DateTime<Utc>,
) -> bool {
    if !enabled {
        return false;
    }
    match last_success.and_then(|s| DateTime::parse_from_rfc3339(s).ok()) {
        None => true,
        Some(t) => now.signed_duration_since(t.with_timezone(&Utc)) > chrono::Duration::hours(24),
    }
}

pub fn start_refresh_due(state: &AppState, now: DateTime<Utc>) -> Result<bool, AppError> {
    let store = state.store.lock().map_err(locked)?;
    let enabled = store
        .setting(REFRESH_ON_START)?
        .is_none_or(|v| v != "false");
    let last = store.setting(mizan_market::refresh::LAST_REFRESH_KEY)?;
    Ok(should_refresh_on_start(enabled, last.as_deref(), now))
}

// ---- providers and settings ----

fn last_success(store: &Store) -> Result<Vec<(String, String)>, AppError> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (finished, json) in store.refresh_reports(50)? {
        let Ok(rep) = serde_json::from_str::<RefreshReport>(&json) else {
            continue;
        };
        for s in rep.sources.iter().filter(|s| s.status == SourceStatus::Ok) {
            if !out.iter().any(|(id, _)| id == &s.id) {
                out.push((s.id.clone(), finished.clone()));
            }
        }
    }
    Ok(out)
}

pub fn market_providers(state: &AppState) -> Result<Vec<ProviderView>, AppError> {
    let store = state.store.lock().map_err(locked)?;
    let last = last_success(&store)?;
    Ok(PROVIDERS
        .iter()
        .map(|p| ProviderView {
            id: p.id.into(),
            name: p.name.into(),
            supplies: p.supplies.into(),
            enabled: mizan_market::is_enabled(&store, p.id),
            last_success: last
                .iter()
                .find(|(id, _)| id == p.id)
                .map(|(_, t)| t.clone()),
        })
        .collect())
}

pub fn set_provider_enabled(state: &AppState, id: &str, enabled: bool) -> Result<(), AppError> {
    let mut store = state.store.lock().map_err(locked)?;
    Ok(mizan_market::set_enabled(&mut store, id, enabled)?)
}

fn base_choices(store: &Store) -> Result<Vec<String>, AppError> {
    let mut c = store.fx_currencies()?;
    if c.is_empty() {
        c = vec!["USD".into(), "EUR".into()];
    }
    c.retain(|x| x != "TRY");
    c.insert(0, "TRY".into());
    Ok(c)
}

pub fn settings_view(state: &AppState) -> Result<SettingsView, AppError> {
    let providers = market_providers(state)?;
    let store = state.store.lock().map_err(locked)?;
    Ok(SettingsView {
        base_currency: mizan_core::valuation::base_currency(&store)?.to_string(),
        currencies: base_choices(&store)?,
        refresh_on_start: store
            .setting(REFRESH_ON_START)?
            .is_none_or(|v| v != "false"),
        last_refresh_at: store.setting(mizan_market::refresh::LAST_REFRESH_KEY)?,
        providers,
        last_report: store
            .last_refresh_report()?
            .and_then(|(_, j)| serde_json::from_str(&j).ok()),
        refreshing: state.refreshing.load(Ordering::SeqCst),
    })
}

pub fn set_base_currency(state: &AppState, currency: &str) -> Result<(), AppError> {
    let mut store = state.store.lock().map_err(locked)?;
    if !base_choices(&store)?.iter().any(|c| c == currency) {
        return Err(invalid(format!(
            "{currency} is not an available base currency"
        )));
    }
    Ok(store.set_setting("base_currency", currency)?)
}

pub fn set_refresh_on_start(state: &AppState, enabled: bool) -> Result<(), AppError> {
    let mut store = state.store.lock().map_err(locked)?;
    Ok(store.set_setting(REFRESH_ON_START, if enabled { "true" } else { "false" })?)
}

// ---- assets ----

pub fn assets_view(state: &AppState, today: Date) -> Result<Vec<AssetView>, AppError> {
    let store = state.store.lock().map_err(locked)?;
    Ok(core_assets_view(&store, today)?)
}

fn ensure_asset(store: &Store, asset_id: i64) -> Result<mizan_storage::AssetRow, AppError> {
    store
        .list_assets()?
        .into_iter()
        .find(|a| a.id == asset_id)
        .ok_or_else(|| invalid(format!("asset {asset_id} does not exist")))
}

fn fund_code(code: &str) -> Result<String, AppError> {
    let c = code.trim().to_uppercase();
    if c.is_empty() || c.len() > 6 || !c.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(invalid(format!("{code:?} is not a TEFAS fund code")));
    }
    Ok(c)
}

/// `kind`: "tefas" (with `fund_code`), "manual" or "none". Stored prices are kept.
pub fn set_price_source(
    state: &AppState,
    asset_id: i64,
    kind: &str,
    code: Option<&str>,
) -> Result<(), AppError> {
    let mut store = state.store.lock().map_err(locked)?;
    ensure_asset(&store, asset_id)?;
    let source = match kind {
        "tefas" => Some(PriceSourceKind::Tefas {
            fund_code: fund_code(code.unwrap_or(""))?,
        }),
        "manual" => Some(PriceSourceKind::Manual),
        "none" => None,
        other => return Err(invalid(format!("unknown price source {other:?}"))),
    };
    Ok(store.set_price_source(asset_id, source.as_ref())?)
}

pub fn accept_suggestion(state: &AppState, asset_id: i64) -> Result<(), AppError> {
    let mut store = state.store.lock().map_err(locked)?;
    let a = ensure_asset(&store, asset_id)?;
    let current = store.price_source(asset_id)?;
    let code = tefas_suggestion(&a.hint, current.as_ref())
        .ok_or_else(|| invalid("this asset has no suggestion"))?;
    Ok(store.set_price_source(asset_id, Some(&PriceSourceKind::Tefas { fund_code: code }))?)
}

pub fn add_manual_price(
    state: &AppState,
    asset_id: i64,
    date: &str,
    value: &str,
    currency: &str,
    now: &str,
) -> Result<(), AppError> {
    let mut store = state.store.lock().map_err(locked)?;
    ensure_asset(&store, asset_id)?;
    let date = Date::parse(date).map_err(invalid)?;
    let value = Decimal::from_str(value.trim())
        .map_err(|_| invalid(format!("{value:?} is not a number")))?;
    if value <= Decimal::ZERO {
        return Err(invalid("a price must be positive"));
    }
    let currency = Code::parse(currency.trim()).map_err(invalid)?;
    Ok(store.add_manual_price(
        asset_id,
        &date.to_string(),
        &value.normalize().to_string(),
        currency.as_str(),
        now,
    )?)
}

pub fn delete_manual_price(state: &AppState, asset_id: i64, date: &str) -> Result<(), AppError> {
    let mut store = state.store.lock().map_err(locked)?;
    Ok(store.delete_manual_price(asset_id, date)?)
}

pub fn merge_assets(state: &AppState, from: i64, into: i64) -> Result<i64, AppError> {
    let mut store = state.store.lock().map_err(locked)?;
    ensure_asset(&store, from)?;
    ensure_asset(&store, into)?;
    Ok(store.merge_assets(from, into)?)
}

pub fn undo_merge(state: &AppState, merge_id: i64) -> Result<(), AppError> {
    let mut store = state.store.lock().map_err(locked)?;
    Ok(store.undo_merge(merge_id)?)
}

// ---- Tauri wrappers ----

fn today() -> Date {
    Date(chrono::Local::now().date_naive())
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

async fn blocking<T: Send + 'static>(
    app: AppHandle,
    f: impl FnOnce(&AppState) -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
    tauri::async_runtime::spawn_blocking(move || f(&app.state::<AppState>()))
        .await
        .map_err(|e| invalid(format!("background task failed: {e}")))?
}

fn run_refresh(app: &AppHandle) -> Result<RefreshReport, AppError> {
    let http = mizan_market::ReqwestHttp::new().map_err(|e| invalid(e.0))?;
    let _ = app.emit(EVENT_REFRESH_STARTED, ());
    let result = refresh_prices(&app.state::<AppState>(), &http, today(), now);
    let _ = app.emit(EVENT_REFRESH_FINISHED, result.as_ref().ok());
    result
}

/// Start a background refresh after launch when it is due. Never blocks the window.
pub fn spawn_start_refresh(app: AppHandle) {
    std::thread::spawn(move || {
        let due = start_refresh_due(&app.state::<AppState>(), Utc::now()).unwrap_or(false);
        if due {
            let _ = run_refresh(&app);
        }
    });
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_refresh_prices(app: AppHandle) -> Result<RefreshReport, AppError> {
    tauri::async_runtime::spawn_blocking(move || run_refresh(&app))
        .await
        .map_err(|e| invalid(format!("background task failed: {e}")))?
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_settings(app: AppHandle) -> Result<SettingsView, AppError> {
    blocking(app, settings_view).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_set_provider_enabled(
    app: AppHandle,
    id: String,
    enabled: bool,
) -> Result<(), AppError> {
    blocking(app, move |s| set_provider_enabled(s, &id, enabled)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_set_base_currency(app: AppHandle, currency: String) -> Result<(), AppError> {
    blocking(app, move |s| set_base_currency(s, &currency)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_set_refresh_on_start(app: AppHandle, enabled: bool) -> Result<(), AppError> {
    blocking(app, move |s| set_refresh_on_start(s, enabled)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_assets(app: AppHandle) -> Result<Vec<AssetView>, AppError> {
    blocking(app, |s| assets_view(s, today())).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_set_price_source(
    app: AppHandle,
    asset_id: i64,
    kind: String,
    fund_code: Option<String>,
) -> Result<(), AppError> {
    blocking(app, move |s| {
        set_price_source(s, asset_id, &kind, fund_code.as_deref())
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_accept_suggestion(app: AppHandle, asset_id: i64) -> Result<(), AppError> {
    blocking(app, move |s| accept_suggestion(s, asset_id)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_add_manual_price(
    app: AppHandle,
    asset_id: i64,
    date: String,
    value: String,
    currency: String,
) -> Result<(), AppError> {
    blocking(app, move |s| {
        add_manual_price(s, asset_id, &date, &value, &currency, &now())
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_delete_manual_price(
    app: AppHandle,
    asset_id: i64,
    date: String,
) -> Result<(), AppError> {
    blocking(app, move |s| delete_manual_price(s, asset_id, &date)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_merge_assets(app: AppHandle, from: i64, into: i64) -> Result<i64, AppError> {
    blocking(app, move |s| merge_assets(s, from, into)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_undo_merge(app: AppHandle, merge_id: i64) -> Result<(), AppError> {
    blocking(app, move |s| undo_merge(s, merge_id)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use mizan_import::InstrumentHint;
    use mizan_market::{HttpError, Response};
    use std::sync::Mutex;

    fn fixture(p: &str) -> String {
        std::fs::read_to_string(format!(
            "{}/../../../crates/mizan-market/tests/fixtures/{p}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    }

    /// Answers TEFAS and TCMB from recorded fixtures and counts requests.
    #[derive(Default)]
    struct Fixtures {
        calls: Mutex<Vec<String>>,
    }

    impl Http for Fixtures {
        fn get(&self, url: &str) -> Result<Response, HttpError> {
            self.calls.lock().unwrap().push(url.into());
            Ok(Response {
                status: 200,
                body: fixture("tcmb/today.xml"),
            })
        }
        fn post_json(&self, url: &str, _: &str) -> Result<Response, HttpError> {
            self.calls.lock().unwrap().push(url.into());
            Ok(Response {
                status: 200,
                body: fixture("tefas/fund_aft_week.json"),
            })
        }
        fn pause(&self, _: std::time::Duration) {}
    }

    fn day() -> Date {
        Date::parse("2026-11-01").unwrap()
    }

    fn clock() -> String {
        "2026-11-01T10:00:00Z".into()
    }

    fn state_with_fund() -> (AppState, i64) {
        let mut store = Store::open_in_memory().unwrap();
        let hint = InstrumentHint {
            symbol: Some("AFT".into()),
            class_hint: Some(mizan_import::ClassHint::Fund),
            currency: Some(Code::parse("TRY").unwrap()),
            ..Default::default()
        };
        let id = store.create_asset("sym:AFT@", &hint).unwrap();
        (AppState::new(store), id)
    }

    #[test]
    fn refresh_stores_prices_and_reports() {
        let (state, id) = state_with_fund();
        accept_suggestion(&state, id).unwrap();
        let http = Fixtures::default();
        let rep = refresh_prices(&state, &http, day(), clock).unwrap();
        assert!(!rep.any_failed(), "{rep:?}");
        assert_eq!(
            http.calls.lock().unwrap().len(),
            2,
            "one TEFAS bulk request, one TCMB request"
        );
        let a = &assets_view(&state, day()).unwrap()[0];
        assert_eq!(a.latest_price.as_ref().unwrap().value, "0.959756");
        let p = market_providers(&state).unwrap();
        assert!(p
            .iter()
            .all(|x| x.last_success.as_deref() == Some("2026-11-01T10:00:00Z")));
        let s = settings_view(&state).unwrap();
        assert_eq!(s.last_refresh_at.as_deref(), Some("2026-11-01T10:00:00Z"));
        assert!(s.last_report.is_some());
        assert!(!s.refreshing, "flag reset after the refresh");
    }

    #[test]
    fn second_refresh_is_rejected_while_one_runs() {
        let (state, _) = state_with_fund();
        state.refreshing.store(true, Ordering::SeqCst);
        let err = refresh_prices(&state, &Fixtures::default(), day(), clock).unwrap_err();
        assert!(err.to_string().contains("already running"));
        assert!(
            state.refreshing.load(Ordering::SeqCst),
            "the running refresh keeps its flag"
        );
    }

    #[test]
    fn disabled_provider_makes_no_requests() {
        let (state, id) = state_with_fund();
        accept_suggestion(&state, id).unwrap();
        set_provider_enabled(&state, "tefas", false).unwrap();
        let http = Fixtures::default();
        let rep = refresh_prices(&state, &http, day(), clock).unwrap();
        assert_eq!(
            *http.calls.lock().unwrap(),
            vec![mizan_market::tcmb::TODAY_URL.to_string()]
        );
        assert_eq!(rep.sources[0].status, SourceStatus::Disabled);
        assert!(!market_providers(&state).unwrap()[0].enabled);
        assert!(set_provider_enabled(&state, "nope", true).is_err());
    }

    #[test]
    fn settings_validation() {
        let (state, _) = state_with_fund();
        let s = settings_view(&state).unwrap();
        assert_eq!(
            (s.base_currency.as_str(), s.refresh_on_start),
            ("TRY", true)
        );
        assert_eq!(
            s.currencies,
            vec!["TRY", "USD", "EUR"],
            "before any rates are stored"
        );
        assert!(set_base_currency(&state, "XYZ").is_err());
        set_base_currency(&state, "EUR").unwrap();
        set_refresh_on_start(&state, false).unwrap();
        let s = settings_view(&state).unwrap();
        assert_eq!(
            (s.base_currency.as_str(), s.refresh_on_start),
            ("EUR", false)
        );
    }

    #[test]
    fn asset_actions() {
        let (state, id) = state_with_fund();
        let a = &assets_view(&state, day()).unwrap()[0];
        assert_eq!(a.suggestion.as_deref(), Some("AFT"));
        assert_eq!(a.source.kind, mizan_core::assets::SourceKind::None);

        assert!(set_price_source(&state, id, "tefas", Some("bad code!")).is_err());
        set_price_source(&state, id, "tefas", Some(" tcd ")).unwrap();
        let a = &assets_view(&state, day()).unwrap()[0];
        assert_eq!(a.source.fund_code.as_deref(), Some("TCD"));
        assert!(a.suggestion.is_none(), "no suggestion once a source is set");
        assert!(accept_suggestion(&state, id).is_err());

        assert!(add_manual_price(&state, id, "2026-10-30", "-1", "TRY", "now").is_err());
        assert!(add_manual_price(&state, id, "30.10.2026", "1", "TRY", "now").is_err());
        add_manual_price(&state, id, "2026-10-30", "1.25", "TRY", "now").unwrap();
        let a = &assets_view(&state, day()).unwrap()[0];
        assert_eq!(a.manual_prices.len(), 1);
        assert_eq!(a.latest_price.as_ref().unwrap().source, "manual");
        delete_manual_price(&state, id, "2026-10-30").unwrap();
        assert!(set_price_source(&state, 999, "manual", None).is_err());

        let other = state
            .store
            .lock()
            .unwrap()
            .create_asset("isin:TRAFT0000000", &InstrumentHint::default())
            .unwrap();
        let m = merge_assets(&state, other, id).unwrap();
        let v = assets_view(&state, day()).unwrap();
        assert_eq!(
            v.iter()
                .find(|x| x.asset_id == other)
                .unwrap()
                .merged_into
                .as_ref()
                .unwrap()
                .merge_id,
            m
        );
        undo_merge(&state, m).unwrap();
        assert!(assets_view(&state, day())
            .unwrap()
            .iter()
            .all(|x| x.merged_into.is_none()));
    }

    #[test]
    fn refresh_on_start_decision() {
        let now = DateTime::parse_from_rfc3339("2026-11-02T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(!should_refresh_on_start(false, None, now), "disabled");
        assert!(should_refresh_on_start(true, None, now), "never refreshed");
        assert!(
            !should_refresh_on_start(true, Some("2026-11-01T13:00:00Z"), now),
            "23 hours"
        );
        assert!(
            should_refresh_on_start(true, Some("2026-11-01T06:00:00Z"), now),
            "30 hours"
        );
        assert!(
            should_refresh_on_start(true, Some("garbage"), now),
            "unreadable counts as never"
        );

        let (state, _) = state_with_fund();
        assert!(start_refresh_due(&state, now).unwrap());
        set_refresh_on_start(&state, false).unwrap();
        assert!(!start_refresh_due(&state, now).unwrap());
    }
}
