//! Application state: the database and the open import review.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

use mizan_storage::Store;

use crate::session::ImportSession;

pub const DB_FILE: &str = "mizan.db";

pub struct AppState {
    pub store: Mutex<Store>,
    pub session: Mutex<Option<ImportSession>>,
    /// Database file; a refresh opens its own connection to it so screens stay responsive.
    /// `None` for in-memory stores (tests), which then share `store`.
    pub db_path: Option<PathBuf>,
    /// Set while a price refresh runs; only one runs at a time.
    pub refreshing: AtomicBool,
}

impl AppState {
    pub fn new(store: Store) -> Self {
        AppState {
            store: Mutex::new(store),
            session: Mutex::new(None),
            db_path: None,
            refreshing: AtomicBool::new(false),
        }
    }

    pub fn with_path(store: Store, db_path: PathBuf) -> Self {
        AppState {
            db_path: Some(db_path),
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
