//! The Exports screen: a database backup and CSV files of activities and holdings.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};
use mizan_core::exports::{activities_csv, all_activities, holdings_csv, slug};
use mizan_core::portfolio::AccountScope;
use mizan_import::Date;
use tauri::AppHandle;

use crate::commands::{blocking, holdings_scoped, store_err};
use crate::session::AppError;
use crate::state::AppState;

fn io(path: &Path, e: impl ToString) -> AppError {
    AppError::Io {
        path: path.display().to_string(),
        message: e.to_string(),
    }
}

/// Copy the database to `dir/mizan-backup-<YYYYMMDD-HHMMSS>.db`. The copy is
/// written under a temporary name and renamed, so a failure leaves no file.
pub fn backup_database(
    state: &AppState,
    dir: &Path,
    now: DateTime<Local>,
) -> Result<String, AppError> {
    let name = format!("mizan-backup-{}.db", now.format("%Y%m%d-%H%M%S"));
    let path = dir.join(&name);
    if path.exists() {
        return Err(AppError::Invalid(format!(
            "{} already exists",
            path.display()
        )));
    }
    let tmp = dir.join(format!(".{name}.part"));
    let _ = std::fs::remove_file(&tmp);
    let written = state.store.lock().map_err(store_err)?.backup_to(&tmp);
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(io(dir, e));
    }
    if let Err(e) = std::fs::rename(&tmp, &path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(io(&path, e));
    }
    Ok(path.display().to_string())
}

fn write(path: &Path, body: &str) -> Result<String, AppError> {
    std::fs::write(path, body).map_err(|e| io(path, e))?;
    Ok(path.display().to_string())
}

pub fn export_activities_csv(
    state: &AppState,
    dir: &Path,
    today: Date,
) -> Result<String, AppError> {
    let rows = all_activities(&*state.store.lock().map_err(store_err)?)?;
    write(
        &dir.join(format!("mizan-activities-{today}.csv")),
        &activities_csv(&rows),
    )
}

pub fn export_holdings_csv(
    state: &AppState,
    scope: AccountScope,
    dir: &Path,
    today: Date,
) -> Result<String, AppError> {
    let name = match scope {
        AccountScope::All => "all-accounts".to_string(),
        AccountScope::One(id) => slug(
            &state
                .store
                .lock()
                .map_err(store_err)?
                .list_accounts()?
                .into_iter()
                .find(|a| a.id == id)
                .map(|a| a.name)
                .unwrap_or_default(),
        ),
    };
    let h = holdings_scoped(state, today, scope)?;
    write(
        &dir.join(format!("mizan-holdings-{name}-{today}.csv")),
        &holdings_csv(&h),
    )
}

fn today() -> Date {
    Date(chrono::Local::now().date_naive())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_backup_database(app: AppHandle, dir: PathBuf) -> Result<String, AppError> {
    blocking(app, move |s| backup_database(s, &dir, Local::now())).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_export_activities_csv(app: AppHandle, dir: PathBuf) -> Result<String, AppError> {
    blocking(app, move |s| export_activities_csv(s, &dir, today())).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_export_holdings_csv(
    app: AppHandle,
    account_id: Option<i64>,
    dir: PathBuf,
) -> Result<String, AppError> {
    blocking(app, move |s| {
        export_holdings_csv(s, AccountScope::from_option(account_id), &dir, today())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{import_commit, import_create_account, import_create_asset, import_open};
    use chrono::TimeZone;
    use mizan_storage::Store;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../crates/mizan-core/tests/data/e2e")
            .join(name)
    }

    fn seeded() -> AppState {
        let state = AppState::new(Store::open_in_memory().unwrap());
        import_open(&state, fixture("seed.json")).unwrap();
        import_create_account(&state, "DEG-4417".into()).unwrap();
        import_create_asset(&state, "isin:NL0010273215".into()).unwrap();
        import_create_asset(&state, "isin:IE00BK5BQT80".into()).unwrap();
        import_commit(&state).unwrap();
        state
    }

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mizan-exports-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn day() -> Date {
        Date::parse("2026-11-02").unwrap()
    }

    #[test]
    fn backup_opens_as_a_database_with_the_same_data() {
        let st = seeded();
        let d = dir("backup");
        let now = Local.with_ymd_and_hms(2026, 11, 2, 21, 5, 9).unwrap();
        let path = backup_database(&st, &d, now).unwrap();
        assert!(path.ends_with("mizan-backup-20261102-210509.db"), "{path}");
        let copy = Store::open(Path::new(&path)).unwrap();
        assert_eq!(
            copy.list_accounts().unwrap().len(),
            st.store.lock().unwrap().list_accounts().unwrap().len()
        );
        let leftovers: Vec<_> = std::fs::read_dir(&d)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(leftovers.len(), 1, "no temporary file left: {leftovers:?}");
        assert!(backup_database(&st, &d, now).is_err(), "never overwrites");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn backup_to_an_unwritable_folder_leaves_no_file() {
        let st = seeded();
        let missing = std::env::temp_dir()
            .join(format!("mizan-exports-missing-{}", std::process::id()))
            .join("nope");
        assert!(backup_database(&st, &missing, Local::now()).is_err());
        assert!(!missing.exists());
    }

    #[test]
    fn activities_csv_lists_every_activity() {
        let st = seeded();
        let d = dir("activities");
        let path = export_activities_csv(&st, &d, day()).unwrap();
        assert!(path.ends_with("mizan-activities-2026-11-02.csv"));
        let body = std::fs::read_to_string(&path).unwrap();
        let n = st
            .store
            .lock()
            .unwrap()
            .query_activities(&Default::default(), 0, 10_000)
            .unwrap()
            .total;
        assert_eq!(
            body.split("\r\n").filter(|l| !l.is_empty()).count() as i64,
            n + 1
        );
        assert!(body.starts_with("date,account,kind,instrument"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn holdings_csv_for_one_account_names_it() {
        let st = seeded();
        let d = dir("holdings");
        let degiro = st
            .store
            .lock()
            .unwrap()
            .list_accounts()
            .unwrap()
            .into_iter()
            .find(|a| a.name == "Degiro")
            .unwrap()
            .id;
        let path = export_holdings_csv(&st, AccountScope::One(degiro), &d, day()).unwrap();
        assert!(
            path.ends_with("mizan-holdings-degiro-2026-11-02.csv"),
            "{path}"
        );
        let body = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = body.split("\r\n").filter(|l| !l.is_empty()).collect();
        assert!(lines.len() > 1);
        assert!(lines[1..].iter().all(|l| l.starts_with("Degiro,")));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
