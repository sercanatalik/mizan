//! Settings › Reset data: back up, then clear portfolio data or everything.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use mizan_storage::{HistoryCache, ResetScope};
use serde::Serialize;
use tauri::AppHandle;

use crate::commands::{blocking, store_err};
use crate::market::bump_generation;
use crate::session::AppError;
use crate::state::AppState;

/// What a reset would clear.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ResetPreview {
    #[cfg_attr(test, ts(type = "number"))]
    pub accounts: i64,
    #[cfg_attr(test, ts(type = "number"))]
    pub activities: i64,
    #[cfg_attr(test, ts(type = "number"))]
    pub imports: i64,
    #[cfg_attr(test, ts(type = "number"))]
    pub assets: i64,
    /// Prices, rates and index values in the shared cache (Everything only).
    #[cfg_attr(test, ts(type = "number | null"))]
    pub cache_values: Option<i64>,
    pub cache_path: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ResetOutcome {
    /// Backup files written before clearing.
    pub backups: Vec<String>,
}

/// The exact text the user must type.
pub const CONFIRMATION: &str = "RESET";
/// Longest wait for background work to stop.
const WAIT: Duration = Duration::from_secs(10);

fn scope(s: &str) -> Result<ResetScope, AppError> {
    match s {
        "portfolio" => Ok(ResetScope::Portfolio),
        "everything" => Ok(ResetScope::Everything),
        other => Err(AppError::Invalid(format!("unknown reset scope {other:?}"))),
    }
}

fn cache_file(state: &AppState) -> Option<PathBuf> {
    state
        .history_dir
        .as_ref()
        .map(|d| d.join(mizan_storage::history::CACHE_FILE))
}

pub fn reset_preview(state: &AppState, s: &str) -> Result<ResetPreview, AppError> {
    let sc = scope(s)?;
    let store = state.store.lock().map_err(store_err)?;
    let cache_values = if sc == ResetScope::Everything {
        Some(crate::market::with_history(state, |c| match c {
            Ok(c) => Ok(c.value_count()?),
            Err(_) => Ok(0),
        })?)
    } else {
        None
    };
    Ok(ResetPreview {
        accounts: store.table_count("accounts")?,
        activities: store.table_count("activities")?,
        imports: store.list_batches()?.len() as i64,
        assets: store.table_count("assets")?,
        cache_values,
        cache_path: if sc == ResetScope::Everything {
            cache_file(state).map(|p| p.display().to_string())
        } else {
            None
        },
    })
}

/// Stop the refresh and history download, waiting up to `WAIT`.
fn stop_background(state: &AppState) -> Result<(), AppError> {
    state.cancel.store(true, Ordering::SeqCst);
    let start = Instant::now();
    while state.backfilling.load(Ordering::SeqCst) || state.refreshing.load(Ordering::SeqCst) {
        if start.elapsed() > WAIT {
            state.cancel.store(false, Ordering::SeqCst);
            return Err(AppError::Invalid(
                "a background download is still running; try again in a moment".into(),
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

fn stamp(now: chrono::DateTime<chrono::Local>) -> String {
    now.format("%Y%m%d-%H%M%S").to_string()
}

pub fn reset_data(
    state: &AppState,
    s: &str,
    confirm: &str,
    now: chrono::DateTime<chrono::Local>,
) -> Result<ResetOutcome, AppError> {
    let sc = scope(s)?;
    if confirm != CONFIRMATION {
        return Err(AppError::Invalid(format!(
            "type {CONFIRMATION} to confirm the reset"
        )));
    }
    stop_background(state)?;
    let result = (|| {
        *state.session.lock().map_err(store_err)? = None;
        let ts = stamp(now);
        let mut backups = Vec::new();
        let mut store = state.store.lock().map_err(store_err)?;
        // 1. Backups; a failure stops before anything is cleared.
        if let Some(db) = &state.db_path {
            let dir = db.parent().map(PathBuf::from).unwrap_or_default();
            let path = dir.join(format!("mizan-backup-{ts}.db"));
            store.backup_to(&path)?;
            backups.push(path.display().to_string());
        }
        let mut cache_dir_cache = None;
        if sc == ResetScope::Everything {
            if let Some(dir) = &state.history_dir {
                let cache = HistoryCache::open(dir)?;
                let path = dir.join(format!("market-history-backup-{ts}.sqlite"));
                cache.backup_to(&path)?;
                backups.push(path.display().to_string());
                cache_dir_cache = Some(cache);
            }
        }
        // 2. Clear.
        store.reset(sc)?;
        if sc == ResetScope::Everything {
            match cache_dir_cache {
                Some(mut c) => c.clear()?,
                None => state.history_mem.lock().map_err(store_err)?.clear()?,
            }
        }
        Ok::<_, AppError>(ResetOutcome { backups })
    })();
    // 3. Forget derived state and let background work run again.
    if let Ok(mut b) = state.backfill.lock() {
        *b = None;
    }
    if let Ok(mut m) = state.returns_memo.lock() {
        m.clear();
    }
    state.cancel.store(false, Ordering::SeqCst);
    bump_generation(state);
    result
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_reset_preview(app: AppHandle, scope: String) -> Result<ResetPreview, AppError> {
    blocking(app, move |s| reset_preview(s, &scope)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_reset_data(
    app: AppHandle,
    scope: String,
    confirm: String,
) -> Result<ResetOutcome, AppError> {
    blocking(app, move |s| {
        reset_data(s, &scope, &confirm, chrono::Local::now())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use mizan_core::{commit, prepare, resolve_batch, CreatePolicy, Decisions, MappingResolver};
    use mizan_import::Date;
    use mizan_storage::{CachedFundPrice, Store};

    fn import(store: &mut Store) {
        let recs = serde_json::json!([
            {"kind": "deposit", "date": "2026-01-05", "net_cash": {"amount": "1000.00", "currency": "TRY"}},
            {"kind": "buy", "date": "2026-01-05", "instrument": {"symbol": "AH2", "class_hint": "fund", "currency": "TRY"}, "quantity": "1000", "net_cash": {"amount": "1000.00", "currency": "TRY"}}
        ]);
        let mut recs = recs.as_array().unwrap().clone();
        for (i, r) in recs.iter_mut().enumerate() {
            r["source"] = serde_json::json!({"adapter": "t", "adapter_version": "1", "file_sha256": "a".repeat(64), "locator": format!("row {}", i + 1)});
            r["account"] = serde_json::json!({"hint": "P1", "name_hint": "Pension"});
        }
        let parsed = mizan_import::parse_batch_value(
            serde_json::json!({"schema": "mizan.import/1", "records": recs}),
        )
        .unwrap();
        let rb = {
            let mut r = MappingResolver {
                store: &mut *store,
                policy: CreatePolicy {
                    create_accounts: true,
                    create_assets: true,
                },
            };
            resolve_batch(parsed, &mut r).unwrap()
        };
        let p = prepare(store, rb).unwrap();
        commit(store, &p, &Decisions::new()).unwrap();
        store.set_setting("base_currency", "USD").unwrap();
        store
            .set_setting("provider.twelvedata.api_key", "td-key")
            .unwrap();
    }

    fn state_in(dir: Option<&std::path::Path>) -> AppState {
        let mut store = match dir {
            Some(d) => Store::open(d.join(crate::state::DB_FILE)).unwrap(),
            None => Store::open_in_memory().unwrap(),
        };
        import(&mut store);
        let st = match dir {
            Some(d) => AppState::with_adapters_dir(
                store,
                Some(d.join(crate::state::DB_FILE)),
                d.join("adapters"),
            ),
            None => AppState::new(store),
        };
        st.history_mem
            .lock()
            .unwrap()
            .upsert_fund_price(
                &CachedFundPrice {
                    code: "AH2".into(),
                    date: "2026-01-05".into(),
                    price: "1.00".into(),
                },
                "t",
            )
            .unwrap();
        st
    }

    fn temp(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("mizan-desktop-reset-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn now() -> chrono::DateTime<chrono::Local> {
        chrono::Local::now()
    }

    fn day() -> Date {
        Date::parse("2026-06-02").unwrap()
    }

    fn setting(st: &AppState, k: &str) -> Option<String> {
        st.store.lock().unwrap().setting(k).unwrap()
    }

    #[test]
    fn reset_portfolio_data_keeps_settings_cache_and_mappings() {
        let dir = temp("portfolio");
        std::fs::create_dir_all(dir.join("adapters")).unwrap();
        std::fs::write(dir.join("adapters").join("my-bank.toml"), "# saved mapping").unwrap();
        let st = state_in(Some(&dir));
        let p = reset_preview(&st, "portfolio").unwrap();
        assert_eq!(
            (
                p.accounts,
                p.activities,
                p.imports,
                p.assets,
                p.cache_values
            ),
            (1, 2, 1, 1, None)
        );

        let out = reset_data(&st, "portfolio", "RESET", now()).unwrap();
        assert_eq!(out.backups.len(), 1);
        assert!(std::path::Path::new(&out.backups[0]).exists());
        assert!(out.backups[0].contains("mizan-backup-"));
        // Screens after reset.
        assert!(crate::commands::holdings(&st, day())
            .unwrap()
            .accounts
            .is_empty());
        assert!(crate::commands::overview(&st, day())
            .unwrap()
            .accounts
            .is_empty());
        assert_eq!(
            crate::activity::activities(&st, &Default::default(), 0)
                .unwrap()
                .total,
            0
        );
        assert!(crate::tax::tax_years(&st, "tr", day())
            .unwrap()
            .years
            .is_empty());
        // Kept.
        assert_eq!(setting(&st, "base_currency").as_deref(), Some("USD"));
        assert_eq!(
            setting(&st, "provider.twelvedata.api_key").as_deref(),
            Some("td-key")
        );
        assert_eq!(st.history_mem.lock().unwrap().value_count().unwrap(), 1);
        assert!(
            dir.join("adapters").join("my-bank.toml").exists(),
            "column mappings survive"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reset_everything_restores_defaults_and_clears_the_cache() {
        let st = state_in(None);
        let p = reset_preview(&st, "everything").unwrap();
        assert_eq!(p.cache_values, Some(1));
        let out = reset_data(&st, "everything", "RESET", now()).unwrap();
        assert!(
            out.backups.is_empty(),
            "nothing on disk to back up in memory"
        );
        assert_eq!(setting(&st, "base_currency").as_deref(), Some("TRY"));
        assert_eq!(setting(&st, "provider.twelvedata.api_key"), None);
        assert_eq!(st.history_mem.lock().unwrap().value_count().unwrap(), 0);
        assert!(crate::commands::holdings(&st, day())
            .unwrap()
            .accounts
            .is_empty());
    }

    #[test]
    fn wrong_confirmation_clears_nothing() {
        let st = state_in(None);
        let g = st.generation.load(Ordering::SeqCst);
        let err = reset_data(&st, "everything", "reset", now()).unwrap_err();
        assert!(err.to_string().contains("RESET"));
        assert_eq!(reset_preview(&st, "portfolio").unwrap().activities, 2);
        assert_eq!(st.generation.load(Ordering::SeqCst), g);
        assert!(reset_data(&st, "partial", "RESET", now()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_failing_backup_clears_nothing() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp("readonly");
        let st = state_in(Some(&dir));
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let r = reset_data(&st, "portfolio", "RESET", now());
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(r.is_err());
        assert_eq!(reset_preview(&st, "portfolio").unwrap().activities, 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reset_waits_for_a_history_download_to_stop() {
        let st = std::sync::Arc::new(state_in(None));
        st.backfilling.store(true, Ordering::SeqCst);
        let bg = st.clone();
        let worker = std::thread::spawn(move || {
            // The download stops at the next unit once it sees the cancel flag.
            while !bg.cancel.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(10));
            }
            std::thread::sleep(Duration::from_millis(100));
            bg.backfilling.store(false, Ordering::SeqCst);
        });
        reset_data(&st, "everything", "RESET", now()).unwrap();
        worker.join().unwrap();
        assert!(!st.backfilling.load(Ordering::SeqCst));
        assert!(
            !st.cancel.load(Ordering::SeqCst),
            "background work may run again"
        );
        assert_eq!(st.history_mem.lock().unwrap().value_count().unwrap(), 0);
    }
}
