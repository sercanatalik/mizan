//! Market data commands: refresh, providers, settings and asset price sources.
//! Plain functions over [`AppState`] (tested) wrapped by thin Tauri commands.

use std::str::FromStr;
use std::sync::atomic::Ordering;

use chrono::{DateTime, Utc};
use mizan_core::assets::{assets_view as core_assets_view, suggestion, AssetView};
use mizan_import::{Code, Date};
use mizan_market::backfill::{self, BackfillProgress, BackfillState, PlanInput};
use mizan_market::{Http, RefreshReport, SourceStatus, PROVIDERS};
use mizan_storage::{HistoryCache, PriceSourceKind, Store};
use rust_decimal::Decimal;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::session::AppError;
use crate::state::AppState;

pub const EVENT_REFRESH_STARTED: &str = "market:refresh-started";
pub const EVENT_REFRESH_FINISHED: &str = "market:refresh-finished";
pub const EVENT_BACKFILL_FINISHED: &str = "market:backfill-finished";
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
    /// Whether this provider needs an API key before it makes any request.
    pub needs_api_key: bool,
    /// The saved key's last four characters, masked ("••••3f9a"); never the key.
    pub api_key_hint: Option<String>,
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
    pub inflation: InflationStatusView,
}

/// The inflation index used for real returns, as Settings shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct InflationStatusView {
    /// For example "HICP Türkiye (Eurostat)"; `None` when the base currency has no index.
    pub label: Option<String>,
    /// Latest cached month, for example "Sep 2026".
    pub through: Option<String>,
    /// When the series was last requested.
    pub last_checked: Option<String>,
    /// The last request failed.
    pub last_check_failed: bool,
    /// Why the index is missing or not refreshed, when it is.
    pub note: Option<String>,
}

fn inflation_status(
    store: &Store,
    cache: Option<&HistoryCache>,
) -> Result<InflationStatusView, AppError> {
    let base = mizan_core::valuation::base_currency(store)?;
    let Some(series) = mizan_market::inflation::series_for(base.as_str()) else {
        return Ok(InflationStatusView {
            label: None,
            through: None,
            last_checked: None,
            last_check_failed: false,
            note: Some(format!(
                "No inflation index for {base}; real returns are not available."
            )),
        });
    };
    let (values, check) = match cache {
        Some(c) => (
            c.inflation_index(series.source, series.id)?,
            c.inflation_check(series.source, series.id)?,
        ),
        None => (vec![], None),
    };
    let through = values
        .last()
        .and_then(|v| mizan_market::inflation::YearMonth::parse(&v.month))
        .map(|m| m.label());
    let note = if !mizan_market::is_enabled(store, series.source) {
        Some(format!(
            "{} is not refreshed because {} is off{}.",
            series.label,
            series.provider,
            if through.is_some() {
                "; cached values are still used"
            } else {
                ""
            }
        ))
    } else if through.is_none() {
        Some("Not downloaded yet; the next refresh fetches it.".into())
    } else {
        None
    };
    Ok(InflationStatusView {
        label: Some(series.full_label()),
        through,
        last_checked: check.as_ref().map(|c| c.checked_at.clone()),
        last_check_failed: check.is_some_and(|c| !c.ok),
        note,
    })
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
    let report = with_history(state, |cache| {
        let cache = match cache {
            Ok(c) => Some(c),
            Err(e) => {
                eprintln!("mizan: history cache unavailable: {e}");
                None
            }
        };
        match &state.db_path {
            // Own connection: screens keep reading while the refresh waits on the network.
            Some(path) => {
                let mut store = Store::open(path)?;
                Ok(mizan_market::refresh(
                    &mut store, cache, http, today, &started, &now,
                )?)
            }
            None => {
                let mut store = state.store.lock().map_err(locked)?;
                Ok(mizan_market::refresh(
                    &mut store, cache, http, today, &started, &now,
                )?)
            }
        }
    })?;
    bump_generation(state);
    Ok(report)
}

// ---- history cache and backfill ----

/// Run `f` with the history cache: a fresh connection to `~/.mizan/market-history.sqlite`,
/// or the in-memory cache in tests. `f` receives the open error instead when the
/// cache cannot be opened, so callers can carry on without it.
pub fn with_history<T>(
    state: &AppState,
    f: impl FnOnce(Result<&mut HistoryCache, String>) -> Result<T, AppError>,
) -> Result<T, AppError> {
    match &state.history_dir {
        Some(dir) => match HistoryCache::open(dir) {
            Ok(mut c) => f(Ok(&mut c)),
            Err(e) => f(Err(e.to_string())),
        },
        None => {
            let mut c = state.history_mem.lock().map_err(locked)?;
            f(Ok(&mut c))
        }
    }
}

fn history_path(state: &AppState) -> Option<String> {
    state.history_dir.as_ref().map(|d| {
        d.join(mizan_storage::history::CACHE_FILE)
            .display()
            .to_string()
    })
}

/// Mark values and returns as stale (see the returns memo).
pub fn bump_generation(state: &AppState) {
    state.generation.fetch_add(1, Ordering::SeqCst);
}

fn plan_input(state: &AppState) -> Result<PlanInput, AppError> {
    match &state.db_path {
        Some(path) => Ok(PlanInput::from_store(&Store::open(path)?)?),
        None => Ok(PlanInput::from_store(
            &*state.store.lock().map_err(locked)?,
        )?),
    }
}

/// Resets the backfill single-flight flag when the backfill ends.
struct Backfilling<'a>(&'a AppState);

impl Drop for Backfilling<'_> {
    fn drop(&mut self) {
        self.0.backfilling.store(false, Ordering::SeqCst);
    }
}

/// Fill missing history into the cache. Runs after a successful refresh; at most
/// one at a time (a second call returns the current progress). Values and returns
/// are marked stale at most every 5 seconds while it runs, and at the end.
pub fn backfill_history(
    state: &AppState,
    http: &dyn Http,
    today: Date,
    now: &dyn Fn() -> String,
) -> Result<BackfillProgress, AppError> {
    if state.backfilling.swap(true, Ordering::SeqCst) {
        return backfill_status(state, today);
    }
    let _running = Backfilling(state);
    let input = plan_input(state)?;
    let path = history_path(state);
    let progress = with_history(state, |cache| {
        let cache = cache.map_err(invalid)?;
        let plan = backfill::plan(&input, cache, today)?;
        let mut last_bump = std::time::Instant::now();
        let p = backfill::run(&plan, cache, http, &state.cancel, now, &mut |p| {
            let mut p = p.clone();
            p.cache_path = path.clone();
            if let Ok(mut slot) = state.backfill.lock() {
                *slot = Some(p);
            }
            if last_bump.elapsed() >= std::time::Duration::from_secs(5) {
                bump_generation(state);
                last_bump = std::time::Instant::now();
            }
        });
        Ok(p)
    })?;
    let mut progress = progress;
    progress.cache_path = path;
    *state.backfill.lock().map_err(locked)? = Some(progress.clone());
    bump_generation(state);
    Ok(progress)
}

/// Backfill progress for Settings. While a backfill runs (or after one failed in
/// this session), its live progress; otherwise what is still missing, computed
/// from the cache without any request.
pub fn backfill_status(state: &AppState, today: Date) -> Result<BackfillProgress, AppError> {
    let last = state.backfill.lock().map_err(locked)?.clone();
    if let Some(p) = &last {
        let failed = p.sources.iter().any(|s| s.state == BackfillState::Failed);
        if state.backfilling.load(Ordering::SeqCst) || failed {
            return Ok(p.clone());
        }
    }
    let input = plan_input(state)?;
    let path = history_path(state);
    with_history(state, |cache| match cache {
        Ok(cache) => {
            let plan = backfill::plan(&input, cache, today)?;
            Ok(BackfillProgress::at_rest(&plan, path.clone()))
        }
        Err(e) => {
            let mut p = BackfillProgress::waiting(path.clone());
            for s in p.sources.iter_mut() {
                s.state = BackfillState::Failed;
                s.error = Some(format!("history cache unavailable: {e}"));
            }
            Ok(p)
        }
    })
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
            needs_api_key: mizan_market::NEEDS_KEY.contains(&p.id),
            api_key_hint: mizan_market::api_key_hint(&store, p.id),
        })
        .collect())
}

pub fn set_provider_enabled(state: &AppState, id: &str, enabled: bool) -> Result<(), AppError> {
    let mut store = state.store.lock().map_err(locked)?;
    Ok(mizan_market::set_enabled(&mut store, id, enabled)?)
}

/// Save (or with `None` or an empty key, clear) a provider's API key. The key is
/// stored in the user database only and never returned.
pub fn set_api_key(state: &AppState, id: &str, key: Option<&str>) -> Result<(), AppError> {
    let mut store = state.store.lock().map_err(locked)?;
    Ok(mizan_market::set_api_key(&mut store, id, key)?)
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
    let inflation = with_history(state, |cache| {
        inflation_status(&store, cache.ok().map(|c| &*c))
    })?;
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
        inflation,
    })
}

pub fn set_base_currency(state: &AppState, currency: &str) -> Result<(), AppError> {
    let mut store = state.store.lock().map_err(locked)?;
    if !base_choices(&store)?.iter().any(|c| c == currency) {
        return Err(invalid(format!(
            "{currency} is not an available base currency"
        )));
    }
    store.set_setting("base_currency", currency)?;
    bump_generation(state);
    Ok(())
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

fn provider_symbol(kind: &str, symbol: &str) -> Result<String, AppError> {
    let s = symbol.trim().to_uppercase();
    if s.is_empty() {
        return Err(invalid("a symbol is required"));
    }
    if s.len() > 32 || s.chars().any(char::is_whitespace) {
        return Err(invalid(format!("{symbol:?} is not a {kind} symbol")));
    }
    Ok(s)
}

/// `kind`: "tefas" (fund code in `symbol`), "yahoo" or "twelvedata" (provider
/// symbol), "manual" or "none". Stored prices are kept.
pub fn set_price_source(
    state: &AppState,
    asset_id: i64,
    kind: &str,
    symbol: Option<&str>,
) -> Result<(), AppError> {
    let mut store = state.store.lock().map_err(locked)?;
    ensure_asset(&store, asset_id)?;
    let symbol = symbol.unwrap_or("");
    let source = match kind {
        "tefas" => Some(PriceSourceKind::Tefas {
            fund_code: fund_code(symbol)?,
        }),
        "yahoo" => Some(PriceSourceKind::Yahoo {
            symbol: provider_symbol("Yahoo", symbol)?,
        }),
        "twelvedata" => Some(PriceSourceKind::TwelveData {
            symbol: provider_symbol("Twelve Data", symbol)?,
        }),
        "manual" => Some(PriceSourceKind::Manual),
        "none" => None,
        other => return Err(invalid(format!("unknown price source {other:?}"))),
    };
    store.set_price_source(asset_id, source.as_ref())?;
    bump_generation(state);
    Ok(())
}

pub fn accept_suggestion(state: &AppState, asset_id: i64) -> Result<(), AppError> {
    let mut store = state.store.lock().map_err(locked)?;
    let a = ensure_asset(&store, asset_id)?;
    let current = store.price_source(asset_id)?;
    let s = suggestion(&a.hint, current.as_ref())
        .ok_or_else(|| invalid("this asset has no suggestion"))?;
    store.set_price_source(asset_id, Some(&s))?;
    bump_generation(state);
    Ok(())
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
    store.add_manual_price(
        asset_id,
        &date.to_string(),
        &value.normalize().to_string(),
        currency.as_str(),
        now,
    )?;
    bump_generation(state);
    Ok(())
}

pub fn delete_manual_price(state: &AppState, asset_id: i64, date: &str) -> Result<(), AppError> {
    let mut store = state.store.lock().map_err(locked)?;
    store.delete_manual_price(asset_id, date)?;
    bump_generation(state);
    Ok(())
}

pub fn merge_assets(state: &AppState, from: i64, into: i64) -> Result<i64, AppError> {
    let mut store = state.store.lock().map_err(locked)?;
    ensure_asset(&store, from)?;
    ensure_asset(&store, into)?;
    let r = store.merge_assets(from, into)?;
    bump_generation(state);
    Ok(r)
}

pub fn undo_merge(state: &AppState, merge_id: i64) -> Result<(), AppError> {
    let mut store = state.store.lock().map_err(locked)?;
    store.undo_merge(merge_id)?;
    bump_generation(state);
    Ok(())
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
    if result.is_ok() {
        spawn_backfill(app.clone());
    }
    result
}

/// Continue with the history backfill in the background after a refresh.
fn spawn_backfill(app: AppHandle) {
    std::thread::spawn(move || {
        let Ok(http) = mizan_market::ReqwestHttp::new() else {
            return;
        };
        let state = app.state::<AppState>();
        if let Err(e) = backfill_history(&state, &http, today(), &now) {
            eprintln!("mizan: history backfill failed: {e}");
        }
        let _ = app.emit(EVENT_BACKFILL_FINISHED, ());
    });
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_backfill_status(app: AppHandle) -> Result<BackfillProgress, AppError> {
    blocking(app, |s| backfill_status(s, today())).await
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
    symbol: Option<String>,
) -> Result<(), AppError> {
    blocking(app, move |s| {
        set_price_source(s, asset_id, &kind, symbol.as_deref())
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_set_api_key(
    app: AppHandle,
    id: String,
    key: Option<String>,
) -> Result<(), AppError> {
    blocking(app, move |s| set_api_key(s, &id, key.as_deref())).await
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
    fn inflation_status_for_each_state() {
        let (state, _) = state_with_fund();
        // TRY base, nothing cached yet.
        let s = settings_view(&state).unwrap().inflation;
        assert_eq!(s.label.as_deref(), Some("HICP Türkiye (Eurostat)"));
        assert_eq!(s.through, None);
        assert_eq!(
            s.note.as_deref(),
            Some("Not downloaded yet; the next refresh fetches it.")
        );

        // Cached through September 2026.
        with_history(&state, |c| {
            c.unwrap()
                .commit_inflation(
                    "eurostat",
                    "TR:I15",
                    &[mizan_storage::CachedIndexValue {
                        month: "2026-09".into(),
                        value: "1675.96".into(),
                    }],
                    true,
                    "2026-11-01T10:00:00Z",
                )
                .unwrap();
            Ok(())
        })
        .unwrap();
        let s = settings_view(&state).unwrap().inflation;
        assert_eq!(s.through.as_deref(), Some("Sep 2026"));
        assert_eq!(s.last_checked.as_deref(), Some("2026-11-01T10:00:00Z"));
        assert!(!s.last_check_failed);
        assert_eq!(s.note, None);

        // Provider turned off: cached values still used.
        {
            let mut store = state.store.lock().unwrap();
            mizan_market::set_enabled(&mut store, "eurostat", false).unwrap();
        }
        let s = settings_view(&state).unwrap().inflation;
        assert_eq!(
            s.note.as_deref(),
            Some("HICP Türkiye is not refreshed because Eurostat is off; cached values are still used.")
        );

        // A base currency without an index.
        state
            .store
            .lock()
            .unwrap()
            .set_setting("base_currency", "CHF")
            .unwrap();
        let s = settings_view(&state).unwrap().inflation;
        assert_eq!(s.label, None);
        assert_eq!(
            s.note.as_deref(),
            Some("No inflation index for CHF; real returns are not available.")
        );
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
        for x in &p {
            let expected = match x.id.as_str() {
                "tefas" | "tcmb" => Some("2026-11-01T10:00:00Z"),
                _ => None, // no asset uses them: idle, not a success
            };
            assert_eq!(x.last_success.as_deref(), expected, "{}", x.id);
        }
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
        assert_eq!(a.suggestion.as_ref().unwrap().label, "TEFAS: AFT");
        assert_eq!(a.source.kind, mizan_core::assets::SourceKind::None);

        assert!(set_price_source(&state, id, "tefas", Some("bad code!")).is_err());
        set_price_source(&state, id, "tefas", Some(" tcd ")).unwrap();
        let a = &assets_view(&state, day()).unwrap()[0];
        assert_eq!(a.source.symbol.as_deref(), Some("TCD"));
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
    fn listed_sources_and_suggestions() {
        let mut store = Store::open_in_memory().unwrap();
        let hint = InstrumentHint {
            symbol: Some("VWCE".into()),
            mic: Some("XETR".into()),
            currency: Some(Code::parse("EUR").unwrap()),
            ..Default::default()
        };
        let id = store.create_asset("sym:VWCE@XETR", &hint).unwrap();
        let state = AppState::new(store);
        let a = &assets_view(&state, day()).unwrap()[0];
        let sug = a.suggestion.as_ref().unwrap();
        assert_eq!(
            (sug.kind.clone(), sug.symbol.as_str(), sug.label.as_str()),
            (
                mizan_core::assets::SourceKind::Yahoo,
                "VWCE.DE",
                "Yahoo: VWCE.DE"
            )
        );
        accept_suggestion(&state, id).unwrap();
        let a = &assets_view(&state, day()).unwrap()[0];
        assert_eq!(a.source.kind, mizan_core::assets::SourceKind::Yahoo);
        assert_eq!(a.source.symbol.as_deref(), Some("VWCE.DE"));

        let err = set_price_source(&state, id, "twelvedata", Some("  ")).unwrap_err();
        assert!(err.to_string().contains("a symbol is required"), "{err}");
        assert_eq!(
            assets_view(&state, day()).unwrap()[0].source.kind,
            mizan_core::assets::SourceKind::Yahoo,
            "unchanged after a refused symbol"
        );
        set_price_source(&state, id, "twelvedata", Some("vwce")).unwrap();
        let a = &assets_view(&state, day()).unwrap()[0];
        assert_eq!(a.source.kind, mizan_core::assets::SourceKind::Twelvedata);
        assert_eq!(a.source.symbol.as_deref(), Some("VWCE"));
        assert!(set_price_source(&state, id, "yahoo", Some("A B")).is_err());
    }

    #[test]
    fn api_key_is_saved_but_never_returned() {
        let (state, _) = state_with_fund();
        let key = "td-secret-0000-3f9a";
        let td = |s: &SettingsView| {
            s.providers
                .iter()
                .find(|p| p.id == "twelvedata")
                .cloned()
                .unwrap()
        };
        let s = settings_view(&state).unwrap();
        assert!(td(&s).needs_api_key && td(&s).api_key_hint.is_none());
        assert!(
            !s.providers
                .iter()
                .find(|p| p.id == "yahoo")
                .unwrap()
                .needs_api_key
        );
        set_api_key(&state, "twelvedata", Some(key)).unwrap();
        let s = settings_view(&state).unwrap();
        assert_eq!(td(&s).api_key_hint.as_deref(), Some("••••3f9a"));
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains(key), "settings never carry the key");
        assert!(set_api_key(&state, "yahoo", Some("x")).is_err());
        set_api_key(&state, "twelvedata", None).unwrap();
        assert!(td(&settings_view(&state).unwrap()).api_key_hint.is_none());
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

#[cfg(test)]
mod backfill_tests {
    use super::*;
    use mizan_core::{commit, prepare, resolve_batch, CreatePolicy, Decisions, MappingResolver};
    use mizan_market::{HttpError, Response};
    use std::sync::Mutex;

    /// Answers every TEFAS request with one AFT price and every TCMB request with
    /// one USD rate, and records each URL.
    #[derive(Default)]
    struct Fake {
        calls: Mutex<Vec<String>>,
    }

    impl Http for Fake {
        fn get(&self, url: &str) -> Result<Response, HttpError> {
            self.calls.lock().unwrap().push(url.into());
            Ok(Response {
                status: 200,
                body: r#"<Tarih_Date Tarih="30.10.2026"><Currency CurrencyCode="USD"><Unit>1</Unit><ForexBuying>34.5</ForexBuying></Currency></Tarih_Date>"#.into(),
            })
        }
        fn post_json(&self, url: &str, body: &str) -> Result<Response, HttpError> {
            self.calls.lock().unwrap().push(url.into());
            let v: serde_json::Value = serde_json::from_str(body).unwrap();
            let from = v["basTarih"].as_str().unwrap().to_string();
            let date = format!("{}-{}-{}", &from[..4], &from[4..6], &from[6..8]);
            Ok(Response {
                status: 200,
                body: format!(
                    r#"{{"errorMessage":null,"resultList":[{{"fonKodu":"AFT","tarih":"{date}","fiyat":1.25}}]}}"#
                ),
            })
        }
        fn pause(&self, _: std::time::Duration) {}
    }

    fn day() -> Date {
        Date::parse("2026-11-20").unwrap()
    }

    /// A TRY fund bought in September, priced from TEFAS.
    fn state() -> AppState {
        let mut store = Store::open_in_memory().unwrap();
        let src = serde_json::json!({"adapter": "t", "adapter_version": "1", "file_sha256": "a".repeat(64), "locator": "row 1"});
        let parsed = mizan_import::parse_batch_value(serde_json::json!({
            "schema": "mizan.import/1",
            "records": [{
                "source": src, "account": {"hint": "P1"}, "kind": "buy", "date": "2026-09-15",
                "instrument": {"symbol": "AFT", "class_hint": "fund", "currency": "TRY"},
                "quantity": "100", "net_cash": {"amount": "120.00", "currency": "TRY"}
            }]
        }))
        .unwrap();
        let rb = {
            let mut r = MappingResolver {
                store: &mut store,
                policy: CreatePolicy {
                    create_accounts: true,
                    create_assets: true,
                },
            };
            resolve_batch(parsed, &mut r).unwrap()
        };
        let p = prepare(&store, rb).unwrap();
        commit(&mut store, &p, &Decisions::new()).unwrap();
        let asset = store.list_assets().unwrap()[0].id;
        store
            .set_price_source(
                asset,
                Some(&PriceSourceKind::Tefas {
                    fund_code: "AFT".into(),
                }),
            )
            .unwrap();
        AppState::new(store)
    }

    #[test]
    fn status_before_any_refresh_is_waiting_and_makes_no_requests() {
        let st = state();
        let p = backfill_status(&st, day()).unwrap();
        let t = p.source("tefas").unwrap();
        assert_eq!((t.state, t.planned), (BackfillState::Waiting, 3));
        assert_eq!(p.source("tcmb").unwrap().state, BackfillState::Idle);
    }

    #[test]
    fn refresh_then_backfill_fills_the_cache_and_status_reports_done() {
        let st = state();
        let http = Fake::default();
        refresh_prices(&st, &http, day(), clock).unwrap();
        let g = st.generation.load(Ordering::SeqCst);
        let p = backfill_history(&st, &http, day(), &clock).unwrap();
        let t = p.source("tefas").unwrap();
        assert_eq!((t.state, t.done, t.planned), (BackfillState::Done, 3, 3));
        assert!(
            st.generation.load(Ordering::SeqCst) > g,
            "values marked stale"
        );
        let cached = st.history_mem.lock().unwrap().fund_prices("AFT").unwrap();
        assert!(cached.len() >= 3);
        let rest = backfill_status(&st, day()).unwrap();
        assert_eq!(rest.source("tefas").unwrap().state, BackfillState::Done);
    }

    #[test]
    fn a_second_backfill_while_one_runs_returns_progress_without_requests() {
        let st = state();
        st.backfilling.store(true, Ordering::SeqCst);
        let http = Fake::default();
        backfill_history(&st, &http, day(), &clock).unwrap();
        assert!(http.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn cancel_stops_the_backfill_and_keeps_completed_units() {
        let st = state();
        st.cancel.store(true, Ordering::SeqCst);
        let http = Fake::default();
        let p = backfill_history(&st, &http, day(), &clock).unwrap();
        assert!(http.calls.lock().unwrap().is_empty());
        assert_eq!(p.source("tefas").unwrap().state, BackfillState::Waiting);
    }

    fn clock() -> String {
        "2026-11-20T10:00:00Z".into()
    }
}
