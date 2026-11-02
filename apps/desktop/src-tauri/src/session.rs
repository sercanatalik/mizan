//! The open import review.
//!
//! A session holds the batch JSON (read from a batch file, or produced by an
//! adapter from a statement file) and the user's decisions. Every view
//! is rebuilt from those with a read-only resolver, so nothing is written
//! until [`ImportSession::commit`]. Creating or mapping an account, or
//! creating an asset, writes only that mapping and then rebuilds.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use mizan_adapters::{detect, run_adapter, Detection, FileInput, Origin, Registry};
use mizan_core::{
    commit, prepare, report, resolve_batch, CommitError, CreatePolicy, Decision, Decisions,
    MappingResolver, Prepared, PreviewResolver, ReviewItem, ReviewReport,
};
use mizan_import::{parse_batch, BatchError, InstrumentHint};
use mizan_storage::{AccountId, StorageError, Store};
use serde::Serialize;

/// Errors shown to the user. Serialized as `{ kind, message }`.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("cannot read {path}: {message}")]
    Io { path: String, message: String },
    #[error("{0}")]
    Batch(#[from] BatchError),
    #[error("{0}")]
    Storage(#[from] StorageError),
    #[error("{0}")]
    Commit(String),
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Adapter(String),
}

impl AppError {
    fn kind(&self) -> &'static str {
        match self {
            AppError::Io { .. } => "io",
            AppError::Batch(_) => "batch",
            AppError::Storage(_) => "storage",
            AppError::Commit(_) => "commit",
            AppError::Invalid(_) => "invalid",
            AppError::Adapter(_) => "adapter",
        }
    }
}

impl Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("AppError", 2)?;
        st.serialize_field("kind", self.kind())?;
        st.serialize_field("message", &self.to_string())?;
        st.end()
    }
}

impl From<CommitError> for AppError {
    fn from(e: CommitError) -> Self {
        match e {
            CommitError::Storage(s) => AppError::Storage(s),
            other => AppError::Commit(other.to_string()),
        }
    }
}

/// An adapter that recognizes the open file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AdapterInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    /// 1–100; higher is a better match.
    pub score: u8,
    /// A mapping the user saved, rather than one shipped with Mizan.
    pub user: bool,
}

impl From<mizan_adapters::Match> for AdapterInfo {
    fn from(m: mizan_adapters::Match) -> Self {
        AdapterInfo {
            id: m.id,
            name: m.name,
            version: m.version,
            score: m.score,
            user: m.origin == Origin::User,
        }
    }
}

/// What the batch says about itself, for the review header.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct SourceInfo {
    /// The adapter that read the statement; `None` for a batch file.
    pub adapter: Option<AdapterInfo>,
    /// Every adapter that recognizes the file, best first (empty for a batch file).
    pub matches: Vec<AdapterInfo>,
    /// Adapter names and versions recorded in the batch's provenance.
    pub adapters: Vec<String>,
    pub file_sha256: Option<String>,
    pub accounts: Vec<String>,
    pub first_date: Option<String>,
    pub last_date: Option<String>,
}

/// Everything the Imports screen shows for an open review.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ImportView {
    pub file_name: String,
    pub source: SourceInfo,
    pub report: ReviewReport,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct CommitView {
    #[cfg_attr(test, ts(type = "number"))]
    pub batch_id: i64,
    #[cfg_attr(test, ts(type = "number"))]
    pub committed: usize,
    #[cfg_attr(test, ts(type = "number"))]
    pub skipped_undecided: usize,
}

/// Result of opening a file: a review, or a file no adapter recognizes.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OpenResult {
    Review {
        view: Box<ImportView>,
    },
    Unrecognized {
        file_name: String,
        file_kind: String,
    },
}

/// How [`ImportSession::open_file`] went.
pub enum Opened {
    Review(Box<ImportSession>),
    Unrecognized {
        file_name: String,
        file_kind: String,
    },
}

pub struct ImportSession {
    pub path: PathBuf,
    /// The statement's bytes, kept to re-run another adapter. Empty for batch files.
    bytes: Vec<u8>,
    json: String,
    source: SourceInfo,
    decisions: Decisions,
    prepared: Prepared,
    report: ReviewReport,
}

fn source_info(json: &str) -> Result<SourceInfo, AppError> {
    let parsed = parse_batch(json)?;
    let mut adapters = BTreeSet::new();
    let mut accounts = BTreeSet::new();
    let mut file = None;
    let mut dates = BTreeSet::new();
    for r in &parsed.records {
        adapters.insert(format!(
            "{} {}",
            r.item.source.adapter, r.item.source.adapter_version
        ));
        accounts.insert(r.item.account.hint.clone());
        file.get_or_insert_with(|| r.item.source.file_sha256.as_str().to_owned());
        dates.insert(r.item.date);
    }
    for a in &parsed.assertions {
        adapters.insert(format!(
            "{} {}",
            a.item.source.adapter, a.item.source.adapter_version
        ));
        accounts.insert(a.item.account.hint.clone());
        file.get_or_insert_with(|| a.item.source.file_sha256.as_str().to_owned());
        dates.insert(a.item.as_of);
    }
    Ok(SourceInfo {
        adapter: None,
        matches: Vec::new(),
        adapters: adapters.into_iter().collect(),
        file_sha256: file,
        accounts: accounts.into_iter().collect(),
        first_date: dates.first().map(|d| d.to_string()),
        last_date: dates.last().map(|d| d.to_string()),
    })
}

fn build(
    store: &Store,
    json: &str,
    decisions: &Decisions,
) -> Result<(Prepared, ReviewReport), AppError> {
    let parsed = parse_batch(json)?;
    let resolved = {
        let mut r = PreviewResolver::new(store, CreatePolicy::default());
        resolve_batch(parsed, &mut r)?
    };
    let prepared = prepare(store, resolved)?;
    let rep = report(store, &prepared, decisions)?;
    Ok((prepared, rep))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Run one adapter over the file and return its batch JSON.
fn run_one(reg: &Registry, id: &str, path: &Path, bytes: &[u8]) -> Result<String, AppError> {
    let loaded = reg
        .get(id)
        .ok_or_else(|| AppError::Invalid(format!("adapter {id} is not loaded")))?;
    let file = FileInput::new(file_name(path), bytes.to_vec());
    let value = run_adapter(loaded.adapter.as_ref(), &file)
        .map_err(|e| AppError::Adapter(e.to_string()))?;
    Ok(value.to_string())
}

impl ImportSession {
    /// Read and review a batch file. Writes nothing.
    pub fn open(store: &Store, path: &Path) -> Result<Self, AppError> {
        let json = std::fs::read_to_string(path).map_err(|e| AppError::Io {
            path: path.display().to_string(),
            message: e.to_string(),
        })?;
        Self::from_json(store, path, Vec::new(), json, None, Vec::new())
    }

    fn from_json(
        store: &Store,
        path: &Path,
        bytes: Vec<u8>,
        json: String,
        adapter: Option<AdapterInfo>,
        matches: Vec<AdapterInfo>,
    ) -> Result<Self, AppError> {
        let source = SourceInfo {
            adapter,
            matches,
            ..source_info(&json)?
        };
        let decisions = Decisions::new();
        let (prepared, report) = build(store, &json, &decisions)?;
        Ok(ImportSession {
            path: path.to_owned(),
            bytes,
            json,
            source,
            decisions,
            prepared,
            report,
        })
    }

    /// Open any supported file: a batch opens directly; a statement goes
    /// through detection and the best matching adapter. Writes nothing.
    pub fn open_file(store: &Store, reg: &Registry, path: &Path) -> Result<Opened, AppError> {
        let bytes = std::fs::read(path).map_err(|e| AppError::Io {
            path: path.display().to_string(),
            message: e.to_string(),
        })?;
        let file = FileInput::new(file_name(path), bytes);
        match detect(reg, &file) {
            Detection::Batch => {
                let json = String::from_utf8(file.bytes)
                    .map_err(|_| AppError::Invalid("the batch file is not UTF-8 text".into()))?;
                Ok(Opened::Review(Box::new(Self::from_json(
                    store,
                    path,
                    Vec::new(),
                    json,
                    None,
                    Vec::new(),
                )?)))
            }
            Detection::Unrecognized => Ok(Opened::Unrecognized {
                file_name: file.name.clone(),
                file_kind: serde_json::to_value(file.kind)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_default(),
            }),
            Detection::Matches(ms) => {
                let matches: Vec<AdapterInfo> = ms.into_iter().map(AdapterInfo::from).collect();
                let best = matches[0].clone();
                let json = run_one(reg, &best.id, path, &file.bytes)?;
                Ok(Opened::Review(Box::new(Self::from_json(
                    store,
                    path,
                    file.bytes,
                    json,
                    Some(best),
                    matches,
                )?)))
            }
        }
    }

    /// Re-read the statement with another matching adapter. Clears decisions,
    /// because record indexes differ between adapters.
    pub fn choose_adapter(
        &mut self,
        store: &Store,
        reg: &Registry,
        id: &str,
    ) -> Result<(), AppError> {
        let chosen = self
            .source
            .matches
            .iter()
            .find(|m| m.id == id)
            .cloned()
            .ok_or_else(|| AppError::Invalid(format!("adapter {id} does not match this file")))?;
        let json = run_one(reg, id, &self.path, &self.bytes)?;
        let matches = std::mem::take(&mut self.source.matches);
        *self = Self::from_json(
            store,
            &self.path,
            std::mem::take(&mut self.bytes),
            json,
            Some(chosen),
            matches,
        )?;
        Ok(())
    }

    fn rebuild(&mut self, store: &Store) -> Result<(), AppError> {
        let (prepared, report) = build(store, &self.json, &self.decisions)?;
        self.prepared = prepared;
        self.report = report;
        Ok(())
    }

    pub fn view(&self) -> ImportView {
        ImportView {
            file_name: file_name(&self.path),
            source: self.source.clone(),
            report: self.report.clone(),
        }
    }

    pub fn pending_decisions(&self) -> usize {
        self.report.summary.pending_confirmations
    }

    /// Record (or with `None`, clear) the decision for a record needing confirmation.
    pub fn decide(
        &mut self,
        store: &Store,
        index: usize,
        decision: Option<Decision>,
    ) -> Result<(), AppError> {
        let needs = self
            .report
            .confirmations
            .iter()
            .any(|c| c.record.index == index);
        if !needs {
            return Err(AppError::Invalid(format!(
                "record #{index} does not need a decision"
            )));
        }
        match decision {
            Some(d) => self.decisions.insert(index, d),
            None => self.decisions.remove(&index),
        };
        self.report = report(store, &self.prepared, &self.decisions)?;
        Ok(())
    }

    fn unresolved_account(&self, hint: &str) -> Result<mizan_import::AccountHint, AppError> {
        self.report
            .unresolved
            .iter()
            .find_map(|i| match i {
                ReviewItem::UnresolvedAccount { hint: h } if h.hint == hint => Some(h.clone()),
                _ => None,
            })
            .ok_or_else(|| {
                AppError::Invalid(format!(
                    "account hint {hint:?} is not unresolved in this review"
                ))
            })
    }

    fn unresolved_instrument(&self, key: &str) -> Result<InstrumentHint, AppError> {
        self.report
            .unresolved
            .iter()
            .find_map(|i| match i {
                ReviewItem::UnresolvedInstrument { key: Some(k), hint } if k == key => {
                    Some((**hint).clone())
                }
                _ => None,
            })
            .ok_or_else(|| {
                AppError::Invalid(format!(
                    "instrument {key:?} is not unresolved in this review"
                ))
            })
    }

    /// Create an account named after the hint and map the hint to it.
    pub fn create_account(&mut self, store: &mut Store, hint: &str) -> Result<AccountId, AppError> {
        let h = self.unresolved_account(hint)?;
        let id = store.create_account(h.name_hint.as_deref().unwrap_or(&h.hint), &h.hint)?;
        self.rebuild(store)?;
        Ok(id)
    }

    /// Map an unresolved hint to an existing account.
    pub fn map_account(
        &mut self,
        store: &mut Store,
        hint: &str,
        account_id: AccountId,
    ) -> Result<(), AppError> {
        self.unresolved_account(hint)?;
        if !store.list_accounts()?.iter().any(|a| a.id == account_id) {
            return Err(AppError::Invalid(format!(
                "account {account_id} does not exist"
            )));
        }
        store.map_account_hint(hint, account_id)?;
        self.rebuild(store)
    }

    /// Create an asset for an unresolved instrument.
    pub fn create_asset(&mut self, store: &mut Store, key: &str) -> Result<(), AppError> {
        let hint = self.unresolved_instrument(key)?;
        store.create_asset(key, &hint)?;
        self.rebuild(store)
    }

    /// Commit with the current decisions. Creates nothing: unresolved hints fail the commit.
    pub fn commit(&self, store: &mut Store) -> Result<CommitView, AppError> {
        let parsed = parse_batch(&self.json)?;
        let resolved = {
            let mut r = MappingResolver {
                store,
                policy: CreatePolicy::default(),
            };
            resolve_batch(parsed, &mut r)?
        };
        let prepared = prepare(store, resolved)?;
        let res = commit(store, &prepared, &self.decisions)?;
        Ok(CommitView {
            batch_id: res.batch_id,
            committed: res.committed,
            skipped_undecided: res.skipped_undecided,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../crates/mizan-core/tests/data/e2e")
            .join(name)
    }

    fn import_seed(store: &mut Store) {
        let mut s = ImportSession::open(store, &fixture("seed.json")).unwrap();
        s.create_account(store, "DEG-4417").unwrap();
        for key in ["isin:NL0010273215", "isin:IE00BK5BQT80"] {
            s.create_asset(store, key).unwrap();
        }
        s.commit(store).unwrap();
    }

    fn repo(rel: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .join(rel)
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mizan-session-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn review(o: Opened) -> ImportSession {
        match o {
            Opened::Review(s) => *s,
            Opened::Unrecognized { file_name, .. } => panic!("{file_name} unrecognized"),
        }
    }

    #[test]
    fn statement_opens_through_its_adapter() {
        let store = Store::open_in_memory().unwrap();
        let reg = Registry::load(None);
        let s = review(
            ImportSession::open_file(
                &store,
                &reg,
                &repo("adapters/ofx/fixtures/investment-xml/input.qfx"),
            )
            .unwrap(),
        );
        let v = s.view();
        let a = v.source.adapter.as_ref().unwrap();
        assert_eq!((a.id.as_str(), a.user), ("ofx", false));
        assert_eq!(v.source.matches.len(), 1);
        assert_eq!(v.source.adapters, vec![format!("ofx {}", a.version)]);
        assert_eq!(v.source.accounts, vec!["X0001234".to_string()]);
        assert!(v.report.summary.new > 0);
        for t in ["accounts", "assets", "activities", "import_batches"] {
            assert_eq!(store.count(t).unwrap(), 0, "{t}");
        }
    }

    #[test]
    fn batch_file_opens_without_adapter() {
        let store = Store::open_in_memory().unwrap();
        let s = review(
            ImportSession::open_file(&store, &Registry::load(None), &fixture("seed.json")).unwrap(),
        );
        assert_eq!(s.view().source.adapter, None);
        assert!(s.view().source.matches.is_empty());
        assert_eq!(
            s.view().source.adapters,
            vec!["degiro-csv 0.4.0".to_string()]
        );
    }

    #[test]
    fn unrecognized_csv_is_reported() {
        let dir = temp_dir("unrec");
        let f = dir.join("mystery.csv");
        std::fs::write(&f, "Datum,Omschrijving,Bedrag\n01-10-2026,Iets,5\n").unwrap();
        let store = Store::open_in_memory().unwrap();
        let o = ImportSession::open_file(&store, &Registry::load(None), &f).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        match o {
            Opened::Unrecognized {
                file_name,
                file_kind,
            } => assert_eq!(
                (file_name.as_str(), file_kind.as_str()),
                ("mystery.csv", "csv")
            ),
            Opened::Review(_) => panic!("should not be recognized"),
        }
    }

    #[test]
    fn switching_adapters_rebuilds_and_clears_decisions() {
        // A user mapping that reads the same bank CSV into another account.
        let dir = temp_dir("switch");
        let generic =
            std::fs::read_to_string(repo("adapters/generic-bank-csv-tr/adapter.toml")).unwrap();
        let mine = generic
            .replace("id = \"generic-bank-csv-tr\"", "id = \"my-bank\"")
            .replace(
                "name = \"Turkish bank account CSV (generic)\"",
                "name = \"My bank\"",
            )
            .replace("{ const = \"TR-BANK\" }", "{ const = \"MY-BANK\" }");
        std::fs::write(dir.join("my-bank.toml"), mine).unwrap();
        let reg = Registry::load(Some(&dir));
        assert!(
            reg.errors.is_empty(),
            "{:?}",
            reg.errors.iter().map(|e| &e.message).collect::<Vec<_>>()
        );
        let store = Store::open_in_memory().unwrap();
        let mut s = review(
            ImportSession::open_file(
                &store,
                &reg,
                &repo("adapters/generic-bank-csv-tr/fixtures/basic/input.csv"),
            )
            .unwrap(),
        );
        let ids: Vec<String> = s
            .view()
            .source
            .matches
            .iter()
            .map(|m| m.id.clone())
            .collect();
        assert_eq!(
            ids,
            ["my-bank", "generic-bank-csv-tr"],
            "user mapping wins the tie"
        );
        assert_eq!(s.view().source.accounts, vec!["MY-BANK".to_string()]);
        s.decisions.insert(0, Decision::New);

        s.choose_adapter(&store, &reg, "generic-bank-csv-tr")
            .unwrap();
        let v = s.view();
        assert_eq!(v.source.adapter.as_ref().unwrap().id, "generic-bank-csv-tr");
        assert_eq!(v.source.accounts, vec!["TR-BANK".to_string()]);
        assert_eq!(v.source.matches.len(), 2, "matches kept for switching back");
        assert!(s.decisions.is_empty());
        assert!(
            s.choose_adapter(&store, &reg, "ofx").is_err(),
            "not a match for this file"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn open_valid_batch_writes_nothing() {
        let store = Store::open_in_memory().unwrap();
        let s = ImportSession::open(&store, &fixture("seed.json")).unwrap();
        let v = s.view();
        assert_eq!(v.file_name, "seed.json");
        assert_eq!(v.source.adapters, vec!["degiro-csv 0.4.0".to_string()]);
        assert_eq!(v.source.accounts, vec!["DEG-4417".to_string()]);
        assert_eq!(
            (
                v.source.first_date.as_deref(),
                v.source.last_date.as_deref()
            ),
            (Some("2026-06-30"), Some("2026-07-03"))
        );
        assert_eq!(v.report.summary.new, 3);
        assert_eq!(
            v.report.summary.unresolved, 3,
            "account and two instruments"
        );
        for t in ["accounts", "assets", "activities", "import_batches"] {
            assert_eq!(store.count(t).unwrap(), 0, "{t}");
        }
    }

    #[test]
    fn unsupported_version_is_rejected() {
        let dir = std::env::temp_dir().join(format!("mizan-session-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("v9.json");
        std::fs::write(&f, r#"{"schema": "mizan.import/9", "records": []}"#).unwrap();
        let store = Store::open_in_memory().unwrap();
        let err = ImportSession::open(&store, &f).err().unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(err.to_string().contains("mizan.import/9"), "{err}");
        assert_eq!(serde_json::to_value(&err).unwrap()["kind"], "batch");
    }

    #[test]
    fn create_account_resolves_hint_and_keeps_decisions() {
        let mut store = Store::open_in_memory().unwrap();
        let mut s = ImportSession::open(&store, &fixture("seed.json")).unwrap();
        let id = s.create_account(&mut store, "DEG-4417").unwrap();
        assert_eq!(store.account_for_hint("DEG-4417").unwrap(), Some(id));
        assert_eq!(
            store.list_accounts().unwrap()[0].name,
            "Degiro",
            "named from the name hint"
        );
        assert!(s
            .view()
            .report
            .unresolved
            .iter()
            .all(|i| !matches!(i, ReviewItem::UnresolvedAccount { .. })));
        assert!(s.create_account(&mut store, "NOPE").is_err());
    }

    #[test]
    fn map_account_to_existing() {
        let mut store = Store::open_in_memory().unwrap();
        let existing = store.create_account("My Degiro", "OLD-HINT").unwrap();
        let mut s = ImportSession::open(&store, &fixture("seed.json")).unwrap();
        s.map_account(&mut store, "DEG-4417", existing).unwrap();
        assert_eq!(store.account_for_hint("DEG-4417").unwrap(), Some(existing));
        assert!(
            s.map_account(&mut store, "DEG-4417", 999).is_err(),
            "no longer unresolved"
        );
    }

    #[test]
    fn decision_fixes_balance_check_and_commit_stores_decided() {
        let mut store = Store::open_in_memory().unwrap();
        import_seed(&mut store);
        let mut s = ImportSession::open(&store, &fixture("batch.json")).unwrap();
        assert_eq!(s.pending_decisions(), 1);
        assert_eq!(s.view().report.summary.unresolved, 1, "SAP");
        assert!(
            s.commit(&mut store).is_err(),
            "unresolved SAP blocks commit"
        );
        assert_eq!(
            store.count("import_batches").unwrap(),
            1,
            "nothing written by the failed commit"
        );

        s.create_asset(&mut store, "isin:DE0007164600").unwrap();
        assert!(!s.view().report.assertion_results[0].passed);
        s.decide(&store, 0, Some(Decision::New)).unwrap();
        let v = s.view();
        assert!(v.report.assertion_results[0].passed);
        assert_eq!(
            (v.report.summary.new, v.report.summary.pending_confirmations),
            (5, 0)
        );

        s.decide(&store, 0, None).unwrap();
        assert_eq!(s.view().report.summary.new, 4, "undo choice reverts counts");
        assert!(
            s.decide(&store, 2, Some(Decision::New)).is_err(),
            "record 2 needs no decision"
        );

        // Commit with the record left undecided: it is skipped.
        let c = s.commit(&mut store).unwrap();
        assert_eq!((c.committed, c.skipped_undecided), (4, 1));
        assert_eq!(store.count("activities").unwrap(), 7);
    }

    #[test]
    fn open_commit_undo_open_again_sees_new() {
        let mut store = Store::open_in_memory().unwrap();
        import_seed(&mut store);
        let batches = store.list_batches().unwrap();
        assert_eq!(batches[0].activity_count, 3);
        store.undo_batch(batches[0].id).unwrap();
        let s = ImportSession::open(&store, &fixture("seed.json")).unwrap();
        assert_eq!(
            (
                s.view().report.summary.new,
                s.view().report.summary.duplicates
            ),
            (3, 0)
        );
    }
}
