//! The column mapper: inspect an unrecognized CSV or XLSX file, preview a
//! mapping over it, and save or export the mapping as a user mapping file.

use std::path::{Path, PathBuf};

use mizan_adapters::input::TableOptions;
use mizan_adapters::mapping::{MappingAdapter, MappingDoc};
use mizan_adapters::registry::Origin;
use mizan_adapters::{run_adapter, FileInput, FileKind, Manifest, Registry};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::AppHandle;

use crate::commands::{blocking, store_err};
use crate::session::AppError;
use crate::state::AppState;

/// Rows shown when inspecting a file.
const INSPECT_ROWS: usize = 20;
/// Records shown in a preview.
const PREVIEW_RECORDS: usize = 50;

/// How to read the file's table; empty fields are guessed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct MapperOptions {
    pub delimiter: Option<String>,
    pub encoding: Option<String>,
    #[serde(default)]
    #[cfg_attr(test, ts(type = "number"))]
    pub skip_rows: usize,
    pub sheet: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct MapperRow {
    #[cfg_attr(test, ts(type = "number"))]
    pub row: usize,
    pub cells: Vec<String>,
}

/// The first rows of a file and how it was read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct MapperInspect {
    pub file_name: String,
    /// `csv` or `xlsx`.
    pub file_kind: String,
    /// The options used, with guessed values filled in.
    pub options: MapperOptions,
    pub sheets: Vec<String>,
    pub header: Vec<String>,
    pub rows: Vec<MapperRow>,
    #[cfg_attr(test, ts(type = "number"))]
    pub total_rows: usize,
}

/// One previewed record, flattened for a table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct PreviewRecord {
    pub locator: String,
    pub date: String,
    pub kind: String,
    pub account: String,
    pub instrument: Option<String>,
    pub quantity: Option<String>,
    pub amount: Option<String>,
    pub currency: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct PreviewDiagnostic {
    pub severity: String,
    pub message: String,
    pub locator: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct MapperPreview {
    pub records: Vec<PreviewRecord>,
    #[cfg_attr(test, ts(type = "number"))]
    pub total_records: usize,
    pub diagnostics: Vec<PreviewDiagnostic>,
    /// Set when the mapping itself is incomplete or invalid; no rows are read then.
    pub error: Option<String>,
}

/// A saved user mapping.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct SavedMapping {
    pub id: String,
    pub name: String,
    pub path: String,
}

fn read_file(path: &Path) -> Result<FileInput, AppError> {
    let bytes = std::fs::read(path).map_err(|e| AppError::Io {
        path: path.display().to_string(),
        message: e.to_string(),
    })?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let file = FileInput::new(name, bytes);
    if !matches!(file.kind, FileKind::Csv | FileKind::Xlsx) {
        return Err(AppError::Invalid(
            "the column mapper reads CSV and XLSX files".into(),
        ));
    }
    Ok(file)
}

fn kind_name(k: FileKind) -> String {
    serde_json::to_value(k)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// Read the first rows of a file, guessing the delimiter and encoding unless given.
pub fn mapper_inspect(
    path: &Path,
    options: Option<MapperOptions>,
) -> Result<MapperInspect, AppError> {
    let file = read_file(path)?;
    let mut o = options.unwrap_or_default();
    if file.kind == FileKind::Csv {
        if o.encoding.is_none() {
            o.encoding = file.guess_encoding().map(str::to_owned);
        }
        let opt = TableOptions {
            delimiter: None,
            encoding: o.encoding.clone(),
            skip_rows: o.skip_rows,
            sheet: None,
        };
        if o.delimiter.is_none() {
            o.delimiter = file.guess_delimiter(&opt);
        }
    }
    let sheets = file.sheet_names();
    if file.kind == FileKind::Xlsx && o.sheet.is_none() {
        o.sheet = sheets.first().cloned();
    }
    let table = file
        .table(&TableOptions {
            delimiter: o.delimiter.clone(),
            encoding: o.encoding.clone(),
            skip_rows: o.skip_rows,
            sheet: o.sheet.clone(),
        })
        .map_err(AppError::Invalid)?;
    Ok(MapperInspect {
        file_name: file.name.clone(),
        file_kind: kind_name(file.kind),
        options: o,
        sheets,
        header: table.header.clone(),
        total_rows: table.rows.len(),
        rows: table
            .rows
            .into_iter()
            .take(INSPECT_ROWS)
            .map(|(row, cells)| MapperRow { row, cells })
            .collect(),
    })
}

/// Validate a mapping sent by the screen (the TOML document as JSON) and
/// return it with its TOML text and manifest.
fn parse_mapping(mapping: Value) -> Result<(MappingDoc, String, Manifest), String> {
    let doc: MappingDoc =
        serde_json::from_value(mapping).map_err(|e| format!("invalid mapping: {e}"))?;
    let text = doc.to_toml()?;
    let manifest = Manifest::from_toml(&text)?;
    if manifest.kind != mizan_adapters::AdapterKind::Mapping {
        return Err("a saved mapping must have type = \"mapping\"".into());
    }
    Ok((doc, text, manifest))
}

fn str_at(v: &Value, path: &[&str]) -> Option<String> {
    let mut cur = v;
    for p in path {
        cur = cur.get(p)?;
    }
    cur.as_str().map(str::to_owned)
}

/// Run a mapping over the file and show what it would import.
pub fn mapper_preview(path: &Path, mapping: Value) -> Result<MapperPreview, AppError> {
    let file = read_file(path)?;
    let empty = |error: String| MapperPreview {
        records: Vec::new(),
        total_records: 0,
        diagnostics: Vec::new(),
        error: Some(error),
    };
    let (doc, _, manifest) = match parse_mapping(mapping) {
        Ok(x) => x,
        Err(e) => return Ok(empty(e)),
    };
    let adapter = match MappingAdapter::from_doc(manifest, doc) {
        Ok(a) => a,
        Err(e) => return Ok(empty(e)),
    };
    let out = match run_adapter(&adapter, &file) {
        Ok(v) => v,
        Err(e) => return Ok(empty(e.to_string())),
    };
    let records = out["records"].as_array().cloned().unwrap_or_default();
    let diagnostics = out["diagnostics"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|d| PreviewDiagnostic {
            severity: str_at(d, &["severity"]).unwrap_or_default(),
            message: str_at(d, &["message"]).unwrap_or_default(),
            locator: str_at(d, &["locator"]).unwrap_or_default(),
        })
        .collect();
    Ok(MapperPreview {
        total_records: records.len(),
        records: records
            .iter()
            .take(PREVIEW_RECORDS)
            .map(|r| {
                let money = r.get("net_cash").or(r.get("gross"));
                let inst = r.get("instrument");
                PreviewRecord {
                    locator: str_at(r, &["source", "locator"]).unwrap_or_default(),
                    date: str_at(r, &["date"]).unwrap_or_default(),
                    kind: str_at(r, &["kind"]).unwrap_or_default(),
                    account: str_at(r, &["account", "hint"]).unwrap_or_default(),
                    instrument: inst.and_then(|i| {
                        ["symbol", "isin", "name", "cusip", "figi"]
                            .iter()
                            .find_map(|k| str_at(i, &[k]))
                    }),
                    quantity: str_at(r, &["quantity"]),
                    amount: money.and_then(|m| str_at(m, &["amount"])),
                    currency: money.and_then(|m| str_at(m, &["currency"])),
                }
            })
            .collect(),
        diagnostics,
        error: None,
    })
}

fn user_file(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.toml"))
}

/// Write a mapping to the user mappings folder and reload the adapters.
pub fn mapper_save(state: &AppState, mapping: Value) -> Result<SavedMapping, AppError> {
    let dir = state
        .adapters_dir
        .as_ref()
        .ok_or_else(|| AppError::Invalid("user mappings are not available".into()))?;
    let (_, text, manifest) = parse_mapping(mapping).map_err(AppError::Invalid)?;
    {
        let reg = state.registry.read().map_err(store_err)?;
        if reg
            .get(&manifest.id)
            .is_some_and(|l| l.origin == Origin::Bundled)
        {
            return Err(AppError::Invalid(format!(
                "{} is the id of a built-in adapter; choose another name",
                manifest.id
            )));
        }
    }
    std::fs::create_dir_all(dir).map_err(|e| AppError::Io {
        path: dir.display().to_string(),
        message: e.to_string(),
    })?;
    let path = user_file(dir, &manifest.id);
    let header = "# Saved by the Mizan column mapper. Edit freely; see docs/adapters.md.\n\n";
    std::fs::write(&path, format!("{header}{text}")).map_err(|e| AppError::Io {
        path: path.display().to_string(),
        message: e.to_string(),
    })?;
    let reg = Registry::load(Some(dir));
    if let Some(e) = reg
        .errors
        .iter()
        .find(|e| e.location.ends_with(&format!("{}.toml", manifest.id)))
    {
        let _ = std::fs::remove_file(&path);
        return Err(AppError::Invalid(format!(
            "the mapping did not load: {}",
            e.message
        )));
    }
    *state.registry.write().map_err(store_err)? = reg;
    Ok(SavedMapping {
        id: manifest.id,
        name: manifest.name,
        path: path.display().to_string(),
    })
}

/// Copy a saved user mapping to `dest`.
pub fn mapper_export(state: &AppState, id: &str, dest: &Path) -> Result<(), AppError> {
    let dir = state
        .adapters_dir
        .as_ref()
        .ok_or_else(|| AppError::Invalid("user mappings are not available".into()))?;
    let src = user_file(dir, id);
    if !src.exists() {
        return Err(AppError::Invalid(format!("no saved mapping {id}")));
    }
    std::fs::copy(&src, dest).map_err(|e| AppError::Io {
        path: dest.display().to_string(),
        message: e.to_string(),
    })?;
    Ok(())
}

// ---- Tauri wrappers ----

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_mapper_inspect(
    app: AppHandle,
    path: PathBuf,
    options: Option<MapperOptions>,
) -> Result<MapperInspect, AppError> {
    blocking(app, move |_| mapper_inspect(&path, options)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_mapper_preview(
    app: AppHandle,
    path: PathBuf,
    mapping: Value,
) -> Result<MapperPreview, AppError> {
    blocking(app, move |_| mapper_preview(&path, mapping)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_mapper_save(app: AppHandle, mapping: Value) -> Result<SavedMapping, AppError> {
    blocking(app, move |s| mapper_save(s, mapping)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_mapper_export(app: AppHandle, id: String, dest: PathBuf) -> Result<(), AppError> {
    blocking(app, move |s| mapper_export(s, &id, &dest)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::import_open;
    use crate::session::OpenResult;
    use mizan_storage::Store;

    fn repo(rel: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .join(rel)
    }

    fn bank_csv() -> PathBuf {
        repo("adapters/generic-bank-csv-tr/fixtures/basic/input.csv")
    }

    /// The bundled generic bank mapping as the screen would send it, renamed.
    fn mapping(id: &str, name: &str) -> Value {
        let text =
            std::fs::read_to_string(repo("adapters/generic-bank-csv-tr/adapter.toml")).unwrap();
        let mut v = serde_json::to_value(MappingDoc::from_toml(&text).unwrap()).unwrap();
        v["adapter"]["id"] = id.into();
        v["adapter"]["name"] = name.into();
        v
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mizan-mapper-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn inspect_guesses_encoding_and_delimiter() {
        let i = mapper_inspect(
            &bank_csv(),
            Some(MapperOptions {
                skip_rows: 1,
                ..Default::default()
            }),
        )
        .unwrap();
        assert_eq!(i.file_kind, "csv");
        assert_eq!(
            (
                i.options.encoding.as_deref(),
                i.options.delimiter.as_deref()
            ),
            (Some("windows-1254"), Some(";"))
        );
        assert_eq!(
            i.header,
            ["İşlem Tarihi", "Açıklama", "Tutar", "Bakiye", "Dekont No"]
        );
        assert_eq!(i.rows[0].cells[1], "Devir Bakiyesi");
        assert_eq!(i.rows[0].row, 3);
        assert!(i.total_rows >= i.rows.len());
    }

    #[test]
    fn preview_shows_records_and_bad_dates() {
        let p = mapper_preview(&bank_csv(), mapping("my-bank", "My bank")).unwrap();
        assert_eq!(p.error, None);
        assert!(p.total_records > 0);
        assert_eq!(p.records[0].date, "2026-10-02");
        assert_eq!(
            (p.records[0].kind.as_str(), p.records[0].currency.as_deref()),
            ("deposit", Some("TRY"))
        );

        let mut bad = mapping("my-bank", "My bank");
        bad["input"]["date_format"] = "%Y-%m-%d".into();
        let p = mapper_preview(&bank_csv(), bad).unwrap();
        assert_eq!(p.total_records, 0);
        assert!(p
            .diagnostics
            .iter()
            .any(|d| d.message.contains("date format") && d.locator.starts_with("row ")));

        let mut incomplete = mapping("my-bank", "My bank");
        incomplete["fields"].as_object_mut().unwrap().remove("date");
        assert!(mapper_preview(&bank_csv(), incomplete)
            .unwrap()
            .error
            .unwrap()
            .contains("date"));
    }

    /// The document shape the screen builds (see `toMappingDoc` in mapping.ts).
    #[test]
    fn preview_accepts_the_screens_document() {
        let doc = serde_json::json!({
            "adapter": { "id": "my-bank", "version": "1.0.0", "name": "My bank", "type": "mapping", "files": ["csv"] },
            "detect": { "required_columns": ["İşlem Tarihi", "Açıklama", "Tutar", "Dekont No"] },
            "input": { "delimiter": ";", "encoding": "windows-1254", "skip_rows": 1, "decimal": ",", "thousands": ".", "date_format": "%d.%m.%Y" },
            "account": { "hint": { "const": "bank" } },
            "rules": [
                { "column": "Açıklama", "equals": "Devir Bakiyesi", "kind": "skip" },
                { "column": "Tutar", "matches": "^-", "kind": "withdrawal" },
                { "column": "Tutar", "matches": ".", "kind": "deposit" }
            ],
            "fields": {
                "date": { "column": "İşlem Tarihi" },
                "net_cash": { "amount": { "column": "Tutar", "transform": ["abs"] }, "currency": { "const": "TRY" } },
                "external_id": { "column": "Dekont No", "optional": true },
                "memo": { "column": "Açıklama" }
            }
        });
        let p = mapper_preview(&bank_csv(), doc).unwrap();
        assert_eq!(p.error, None);
        assert_eq!(p.records[0].kind, "deposit");
        assert_eq!(p.records[0].account, "bank");
        assert_eq!(p.records[1].kind, "withdrawal");
        assert_eq!(p.records[1].amount.as_deref(), Some("12500.00"));
    }

    #[test]
    fn save_reloads_and_redetects_then_exports() {
        let dir = temp_dir("save");
        let state = AppState::with_adapters_dir(
            Store::open_in_memory().unwrap(),
            None,
            dir.join("adapters"),
        );
        let saved = mapper_save(&state, mapping("my-bank-csv", "My Bank CSV")).unwrap();
        assert_eq!(
            (saved.id.as_str(), saved.name.as_str()),
            ("my-bank-csv", "My Bank CSV")
        );
        assert!(Path::new(&saved.path).exists());

        match import_open(&state, bank_csv()).unwrap() {
            OpenResult::Review { view } => {
                let a = view.source.adapter.unwrap();
                assert_eq!(
                    (a.id.as_str(), a.user),
                    ("my-bank-csv", true),
                    "the saved mapping is preselected"
                );
            }
            other => panic!("{other:?}"),
        }

        // A fresh start loads it too.
        let again = AppState::with_adapters_dir(
            Store::open_in_memory().unwrap(),
            None,
            dir.join("adapters"),
        );
        assert!(again.registry.read().unwrap().get("my-bank-csv").is_some());

        let dest = dir.join("exported.toml");
        mapper_export(&state, "my-bank-csv", &dest).unwrap();
        let text = std::fs::read_to_string(&dest).unwrap();
        assert!(
            text.contains("id = \"my-bank-csv\"")
                && text.contains("[fields.date]")
                && !text.contains("optional = false"),
            "{text}"
        );
        assert!(mapper_export(&state, "nope", &dest).is_err());

        let err = mapper_save(&state, mapping("ofx", "Clash")).unwrap_err();
        assert!(err.to_string().contains("built-in"), "{err}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
