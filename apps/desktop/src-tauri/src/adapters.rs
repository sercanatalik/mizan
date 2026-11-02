//! The Adapters screen: every adapter Mizan can read statements with, and
//! management of the user's saved mappings.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use mizan_adapters::registry::Origin;
use mizan_adapters::Registry;
use serde::Serialize;
use tauri::AppHandle;

use crate::commands::{blocking, store_err};
use crate::session::AppError;
use crate::state::AppState;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AdapterRow {
    /// Empty for a file that did not load.
    pub id: String,
    pub name: String,
    pub version: String,
    /// `native`, `script` or `mapping`.
    pub kind: String,
    pub files: Vec<String>,
    /// `bundled` or `saved`.
    pub origin: String,
    pub description: Option<String>,
    /// Imports still stored that used this adapter.
    #[cfg_attr(test, ts(type = "number"))]
    pub imports: usize,
    /// Why the adapter did not load, when it did not.
    pub error: Option<String>,
    /// File name of a saved mapping.
    pub file: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AdaptersList {
    /// Bundled first, then saved, each by name.
    pub adapters: Vec<AdapterRow>,
    /// Whether saved mappings can be managed (a user mappings folder exists).
    pub can_save: bool,
}

fn dir(state: &AppState) -> Result<&PathBuf, AppError> {
    state
        .adapters_dir
        .as_ref()
        .ok_or_else(|| AppError::Invalid("user mappings are not available".into()))
}

fn file_name(location: &str) -> Option<String> {
    Path::new(location)
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
}

pub fn adapters_list(state: &AppState) -> Result<AdaptersList, AppError> {
    let mut uses: HashMap<String, usize> = HashMap::new();
    for b in state.store.lock().map_err(store_err)?.list_batches()? {
        *uses.entry(b.adapter).or_default() += 1;
    }
    let reg = state.registry.read().map_err(store_err)?;
    let mut rows: Vec<AdapterRow> = reg
        .loaded
        .iter()
        .map(|l| {
            let m = l.adapter.manifest();
            let saved = l.origin == Origin::User;
            AdapterRow {
                id: m.id.clone(),
                name: m.name.clone(),
                version: m.version.clone(),
                kind: serde_json::to_value(m.kind)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_default(),
                files: m.files.iter().map(|f| f.as_str().to_string()).collect(),
                origin: if saved { "saved" } else { "bundled" }.into(),
                description: m.description.clone(),
                imports: uses.get(&m.id).copied().unwrap_or(0),
                error: None,
                file: saved.then(|| file_name(&l.location)).flatten(),
            }
        })
        .collect();
    for e in &reg.errors {
        let saved = !e.location.starts_with("adapters/");
        rows.push(AdapterRow {
            id: String::new(),
            name: file_name(&e.location).unwrap_or_else(|| e.location.clone()),
            version: String::new(),
            kind: String::new(),
            files: vec![],
            origin: if saved { "saved" } else { "bundled" }.into(),
            description: None,
            imports: 0,
            error: Some(e.message.clone()),
            file: saved.then(|| file_name(&e.location)).flatten(),
        });
    }
    rows.sort_by(|a, b| {
        (a.origin != "bundled")
            .cmp(&(b.origin != "bundled"))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(AdaptersList {
        adapters: rows,
        can_save: state.adapters_dir.is_some(),
    })
}

/// The saved mapping file of `id`; bundled adapters are refused.
fn saved_file(state: &AppState, id: &str) -> Result<PathBuf, AppError> {
    let reg = state.registry.read().map_err(store_err)?;
    match reg.get(id) {
        Some(l) if l.origin == Origin::User => Ok(PathBuf::from(&l.location)),
        Some(_) => Err(AppError::Invalid(format!(
            "{id} is built in and cannot be changed"
        ))),
        None => Err(AppError::Invalid(format!("no saved mapping {id}"))),
    }
}

fn reload(state: &AppState) -> Result<(), AppError> {
    *state.registry.write().map_err(store_err)? = Registry::load(Some(dir(state)?));
    Ok(())
}

fn io(path: &Path, e: std::io::Error) -> AppError {
    AppError::Io {
        path: path.display().to_string(),
        message: e.to_string(),
    }
}

/// A TOML basic string.
fn toml_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Give a saved mapping a new display name; its id and file stay.
pub fn mapping_rename(state: &AppState, id: &str, name: &str) -> Result<AdaptersList, AppError> {
    let name = name.trim();
    if name.is_empty() || name.chars().any(char::is_control) {
        return Err(AppError::Invalid(
            "a mapping needs a name on one line".into(),
        ));
    }
    let path = saved_file(state, id)?;
    let text = std::fs::read_to_string(&path).map_err(|e| io(&path, e))?;
    let mut in_adapter = false;
    let mut done = false;
    let lines: Vec<String> = text
        .lines()
        .map(|l| {
            let t = l.trim_start();
            if t.starts_with('[') {
                in_adapter = t.starts_with("[adapter]");
            } else if in_adapter
                && !done
                && t.starts_with("name")
                && t[4..].trim_start().starts_with('=')
            {
                done = true;
                return format!("name = {}", toml_string(name));
            }
            l.to_string()
        })
        .collect();
    if !done {
        return Err(AppError::Invalid(format!(
            "{} has no name to change",
            path.display()
        )));
    }
    let mut new = lines.join("\n");
    if text.ends_with('\n') {
        new.push('\n');
    }
    std::fs::write(&path, &new).map_err(|e| io(&path, e))?;
    reload(state)?;
    if state.registry.read().map_err(store_err)?.get(id).is_none() {
        std::fs::write(&path, &text).map_err(|e| io(&path, e))?;
        reload(state)?;
        return Err(AppError::Invalid(
            "the renamed mapping did not load; nothing was changed".into(),
        ));
    }
    adapters_list(state)
}

/// Delete a saved mapping. Imports already committed with it keep their records.
pub fn mapping_delete(state: &AppState, id: &str) -> Result<AdaptersList, AppError> {
    let path = saved_file(state, id)?;
    std::fs::remove_file(&path).map_err(|e| io(&path, e))?;
    reload(state)?;
    adapters_list(state)
}

/// Write an adapter's manifest to `dest`: a saved mapping's file, or a
/// bundled adapter's embedded `adapter.toml`.
pub fn adapter_export(state: &AppState, id: &str, dest: &Path) -> Result<(), AppError> {
    let (origin, location) = {
        let reg = state.registry.read().map_err(store_err)?;
        let l = reg
            .get(id)
            .ok_or_else(|| AppError::Invalid(format!("no adapter {id}")))?;
        (l.origin, l.location.clone())
    };
    let text = match origin {
        Origin::User => {
            std::fs::read_to_string(&location).map_err(|e| io(Path::new(&location), e))?
        }
        Origin::Bundled => mizan_adapters::registry::BUNDLED
            .iter()
            .find(|b| format!("adapters/{}", b.dir) == location)
            .map(|b| b.manifest.to_string())
            .ok_or_else(|| AppError::Invalid(format!("{id} has no manifest to export")))?,
    };
    std::fs::write(dest, text).map_err(|e| io(dest, e))
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_adapter_export(app: AppHandle, id: String, dest: PathBuf) -> Result<(), AppError> {
    blocking(app, move |s| adapter_export(s, &id, &dest)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_adapters_list(app: AppHandle) -> Result<AdaptersList, AppError> {
    blocking(app, adapters_list).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_mapping_rename(
    app: AppHandle,
    id: String,
    name: String,
) -> Result<AdaptersList, AppError> {
    blocking(app, move |s| mapping_rename(s, &id, &name)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_mapping_delete(app: AppHandle, id: String) -> Result<AdaptersList, AppError> {
    blocking(app, move |s| mapping_delete(s, &id)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use mizan_storage::Store;

    fn repo(rel: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .join(rel)
    }

    /// The bundled generic bank mapping saved under another id and name.
    fn write_mapping(dir: &Path, id: &str, name: &str) {
        let text =
            std::fs::read_to_string(repo("adapters/generic-bank-csv-tr/adapter.toml")).unwrap();
        let text = text
            .replace("id = \"generic-bank-csv-tr\"", &format!("id = \"{id}\""))
            .replace(
                "name = \"Turkish bank account CSV (generic)\"",
                &format!("name = \"{name}\""),
            );
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(format!("{id}.toml")), text).unwrap();
    }

    fn state(tag: &str) -> (AppState, PathBuf) {
        let d = std::env::temp_dir().join(format!("mizan-adapters-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let dir = d.join("adapters");
        write_mapping(&dir, "my-bank-csv", "My Bank CSV");
        let st = AppState::with_adapters_dir(Store::open_in_memory().unwrap(), None, dir.clone());
        (st, dir)
    }

    fn row<'a>(l: &'a AdaptersList, name: &str) -> &'a AdapterRow {
        l.adapters
            .iter()
            .find(|a| a.name == name)
            .unwrap_or_else(|| panic!("{name} listed"))
    }

    #[test]
    fn lists_bundled_and_saved() {
        let (st, _) = state("list");
        let l = adapters_list(&st).unwrap();
        assert!(l.can_save);
        let ibkr = l.adapters.iter().find(|a| a.id == "ibkr-flex").unwrap();
        assert_eq!(
            (ibkr.origin.as_str(), ibkr.kind.as_str()),
            ("bundled", "native")
        );
        assert_eq!(ibkr.files, ["xml"]);
        let mine = row(&l, "My Bank CSV");
        assert_eq!(
            (
                mine.origin.as_str(),
                mine.kind.as_str(),
                mine.file.as_deref()
            ),
            ("saved", "mapping", Some("my-bank-csv.toml"))
        );
        assert_eq!(mine.files, ["csv"]);
        let first_saved = l.adapters.iter().position(|a| a.origin == "saved").unwrap();
        assert!(
            l.adapters[..first_saved]
                .iter()
                .all(|a| a.origin == "bundled"),
            "bundled first"
        );
    }

    #[test]
    fn a_saved_mapping_with_a_bundled_id_is_listed_as_not_loaded() {
        let (_, dir) = state("clash");
        write_mapping(&dir, "ofx", "Clash");
        let st = AppState::with_adapters_dir(Store::open_in_memory().unwrap(), None, dir);
        let l = adapters_list(&st).unwrap();
        let bad = l
            .adapters
            .iter()
            .find(|a| a.error.is_some())
            .expect("a load error");
        assert_eq!(bad.origin, "saved");
        assert!(
            bad.error.as_deref().unwrap().contains("ofx"),
            "{:?}",
            bad.error
        );
    }

    #[test]
    fn rename_keeps_id_and_reloads() {
        let (st, dir) = state("rename");
        let l = mapping_rename(&st, "my-bank-csv", "Garanti \"daily\" CSV").unwrap();
        let r = row(&l, "Garanti \"daily\" CSV");
        assert_eq!(r.id, "my-bank-csv");
        assert!(std::fs::read_to_string(dir.join("my-bank-csv.toml"))
            .unwrap()
            .contains("name = \"Garanti \\\"daily\\\" CSV\""));
        assert!(mapping_rename(&st, "my-bank-csv", "  ").is_err());
        assert!(
            mapping_rename(&st, "ibkr-flex", "Mine").is_err(),
            "bundled is read-only"
        );
    }

    #[test]
    fn export_saved_and_bundled() {
        let (st, dir) = state("export");
        let out = dir.parent().unwrap().join("out");
        std::fs::create_dir_all(&out).unwrap();
        adapter_export(&st, "my-bank-csv", &out.join("mine.toml")).unwrap();
        assert!(std::fs::read_to_string(out.join("mine.toml"))
            .unwrap()
            .contains("name = \"My Bank CSV\""));
        adapter_export(&st, "ofx", &out.join("ofx.toml")).unwrap();
        assert!(std::fs::read_to_string(out.join("ofx.toml"))
            .unwrap()
            .contains("id = \"ofx\""));
        assert!(adapter_export(&st, "nope", &out.join("x.toml")).is_err());
    }

    #[test]
    fn delete_removes_the_file_and_keeps_committed_imports() {
        let (st, dir) = state("delete");
        // An import made with the mapping.
        crate::commands::import_open(
            &st,
            repo("adapters/generic-bank-csv-tr/fixtures/basic/input.csv"),
        )
        .unwrap();
        let unresolved = st
            .session
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .view()
            .report
            .unresolved;
        for u in unresolved {
            if let mizan_core::ReviewItem::UnresolvedAccount { hint } = u {
                crate::commands::import_create_account(&st, hint.hint).unwrap();
            }
        }
        crate::commands::import_commit(&st).unwrap();
        let before = st.store.lock().unwrap().list_batches().unwrap();
        assert_eq!(row(&adapters_list(&st).unwrap(), "My Bank CSV").imports, 1);

        let l = mapping_delete(&st, "my-bank-csv").unwrap();
        assert!(!l.adapters.iter().any(|a| a.id == "my-bank-csv"));
        assert!(!dir.join("my-bank-csv.toml").exists());
        assert!(
            st.registry.read().unwrap().get("my-bank-csv").is_none(),
            "no longer offered"
        );
        assert_eq!(st.store.lock().unwrap().list_batches().unwrap(), before);
        assert!(mapping_delete(&st, "ofx").is_err(), "bundled is read-only");
    }
}
