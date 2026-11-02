//! Application state: the database, the loaded adapters and the open import review.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Mutex, RwLock};

use mizan_adapters::Registry;
use mizan_market::backfill::BackfillProgress;
use mizan_storage::{HistoryCache, Store};

use crate::session::ImportSession;

pub const DB_FILE: &str = "mizan.db";
/// Folder in the app data directory holding the user's saved mappings.
pub const ADAPTERS_DIR: &str = "adapters";

pub struct AppState {
    pub store: Mutex<Store>,
    pub session: Mutex<Option<ImportSession>>,
    /// Database file; a refresh opens its own connection to it so screens stay responsive.
    /// `None` for in-memory stores (tests), which then share `store`.
    pub db_path: Option<PathBuf>,
    /// Set while a price refresh runs; only one runs at a time.
    pub refreshing: AtomicBool,
    /// Bundled adapters plus the user's saved mappings.
    pub registry: RwLock<Registry>,
    /// Where user mappings are saved; `None` in tests that never save.
    pub adapters_dir: Option<PathBuf>,
    /// Folder of the shared market-history cache (`~/.mizan`); `None` uses `history_mem`.
    pub history_dir: Option<PathBuf>,
    /// In-memory history cache for tests and when no home folder is known.
    pub history_mem: Mutex<HistoryCache>,
    /// Set while a history backfill runs; only one runs at a time.
    pub backfilling: AtomicBool,
    /// Progress of the backfill that ran (or runs) in this session.
    pub backfill: Mutex<Option<BackfillProgress>>,
    /// Set when the app quits; stops the backfill between units.
    pub cancel: AtomicBool,
    /// Bumped whenever data behind values and returns changes.
    pub generation: AtomicU64,
    /// Returns and value history for the last generation computed.
    pub returns_memo: Mutex<Option<crate::returns::Computed>>,
}

impl AppState {
    pub fn new(store: Store) -> Self {
        AppState {
            store: Mutex::new(store),
            session: Mutex::new(None),
            db_path: None,
            refreshing: AtomicBool::new(false),
            registry: RwLock::new(Registry::load(None)),
            adapters_dir: None,
            history_dir: None,
            history_mem: Mutex::new(HistoryCache::open_in_memory().expect("in-memory cache")),
            backfilling: AtomicBool::new(false),
            backfill: Mutex::new(None),
            cancel: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            returns_memo: Mutex::new(None),
        }
    }

    /// State for the app: the database file and user mappings live in `app_data_dir`,
    /// the market-history cache in `~/.mizan` (or `$MIZAN_HOME`).
    pub fn for_app_dir(store: Store, app_data_dir: &Path) -> Self {
        AppState {
            history_dir: mizan_storage::history::mizan_home(),
            ..Self::with_adapters_dir(
                store,
                Some(db_path(app_data_dir)),
                app_data_dir.join(ADAPTERS_DIR),
            )
        }
    }

    pub fn with_adapters_dir(
        store: Store,
        db_path: Option<PathBuf>,
        adapters_dir: PathBuf,
    ) -> Self {
        AppState {
            db_path,
            registry: RwLock::new(Registry::load(Some(&adapters_dir))),
            adapters_dir: Some(adapters_dir),
            ..AppState::new(store)
        }
    }
}

pub fn db_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(DB_FILE)
}

/// Open (creating and migrating on first launch) the database in the app data folder.
pub fn open_store(app_data_dir: &Path) -> Result<Store, String> {
    std::fs::create_dir_all(app_data_dir)
        .map_err(|e| format!("cannot create {}: {e}", app_data_dir.display()))?;
    Store::open(db_path(app_data_dir)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_launch_creates_and_migrates_then_keeps_data() {
        let dir = std::env::temp_dir()
            .join(format!("mizan-desktop-state-{}", std::process::id()))
            .join("nested");
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
        {
            let mut store = open_store(&dir).unwrap();
            assert!(db_path(&dir).exists());
            assert_eq!(store.schema_version().unwrap(), 2);
            store.create_account("IBKR", "U1").unwrap();
        }
        let store = open_store(&dir).unwrap();
        assert_eq!(store.count("accounts").unwrap(), 1);
        std::fs::remove_dir_all(dir.parent().unwrap()).unwrap();
    }
}
