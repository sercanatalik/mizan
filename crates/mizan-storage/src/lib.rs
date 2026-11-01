//! SQLite persistence for Mizan.
//!
//! [`Store`] owns one SQLite connection, applies migrations on open, and
//! exposes the small set of reads and writes the import pipeline needs.

use std::path::Path;

use mizan_import::{Assertion, ImportRecord, InstrumentHint};
use rusqlite::{params, Connection, OptionalExtension, Row};
use thiserror::Error;

pub type AccountId = i64;
pub type AssetId = i64;
pub type BatchId = i64;

/// Migrations in order. The database's `user_version` records how many have run.
const MIGRATIONS: &[&str] = &[include_str!("../migrations/0001_init.sql")];

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("stored JSON could not be read: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, StorageError>;

/// An activity ready to be persisted.
#[derive(Clone, Debug)]
pub struct NewActivity {
    pub account_id: AccountId,
    pub asset_id: Option<AssetId>,
    pub fingerprint: String,
    pub source_order: i64,
    pub basis_unknown: bool,
    pub record: ImportRecord,
}

/// A balance assertion ready to be persisted, with its verification result if known.
#[derive(Clone, Debug)]
pub struct NewAssertion {
    pub account_id: AccountId,
    pub assertion: Assertion,
    pub result: Option<AssertionResult>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssertionResult {
    pub passed: bool,
    pub detail: serde_json::Value,
}

/// One adapter run over one file, committed atomically.
#[derive(Clone, Debug)]
pub struct NewBatch {
    pub adapter: String,
    pub adapter_version: String,
    pub file_sha256: String,
    pub activities: Vec<NewActivity>,
    pub assertions: Vec<NewAssertion>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StoredActivity {
    pub id: i64,
    pub batch_id: BatchId,
    pub account_id: AccountId,
    pub asset_id: Option<AssetId>,
    pub fingerprint: String,
    pub source_order: i64,
    pub basis_unknown: bool,
    pub record: ImportRecord,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", true)?;
        let mut store = Store { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&mut self) -> Result<()> {
        let current: i64 = self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
            let tx = self.conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", (i + 1) as i64)?;
            tx.commit()?;
        }
        Ok(())
    }

    /// Number of migrations applied.
    pub fn schema_version(&self) -> Result<i64> {
        Ok(self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))?)
    }

    // ---- accounts ----

    pub fn account_for_hint(&self, hint: &str) -> Result<Option<AccountId>> {
        Ok(self
            .conn
            .query_row(
                "SELECT account_id FROM account_hints WHERE hint = ?1",
                [hint],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Create an account and map `hint` to it.
    pub fn create_account(&mut self, name: &str, hint: &str) -> Result<AccountId> {
        let tx = self.conn.transaction()?;
        tx.execute("INSERT INTO accounts (name) VALUES (?1)", [name])?;
        let id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO account_hints (hint, account_id) VALUES (?1, ?2)",
            params![hint, id],
        )?;
        tx.commit()?;
        Ok(id)
    }

    /// Map an additional statement identifier to an existing account.
    pub fn map_account_hint(&mut self, hint: &str, account_id: AccountId) -> Result<()> {
        self.conn.execute(
            "INSERT INTO account_hints (hint, account_id) VALUES (?1, ?2)
             ON CONFLICT(hint) DO UPDATE SET account_id = excluded.account_id",
            params![hint, account_id],
        )?;
        Ok(())
    }

    // ---- assets ----

    pub fn asset_for_key(&self, instrument_key: &str) -> Result<Option<AssetId>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM assets WHERE instrument_key = ?1",
                [instrument_key],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn create_asset(&mut self, instrument_key: &str, hint: &InstrumentHint) -> Result<AssetId> {
        self.conn.execute(
            "INSERT INTO assets (instrument_key, name, currency, hint_json) VALUES (?1, ?2, ?3, ?4)",
            params![
                instrument_key,
                hint.name.as_deref().or(hint.symbol.as_deref()),
                hint.currency.as_ref().map(|c| c.as_str()),
                serde_json::to_string(hint)?,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    // ---- batches ----

    /// Persist a batch, its activities and its assertions in one transaction.
    /// On any error nothing is written.
    pub fn commit_batch(&mut self, batch: &NewBatch) -> Result<BatchId> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO import_batches (adapter, adapter_version, file_sha256) VALUES (?1, ?2, ?3)",
            params![batch.adapter, batch.adapter_version, batch.file_sha256],
        )?;
        let batch_id = tx.last_insert_rowid();
        {
            let mut ins = tx.prepare(
                "INSERT INTO activities (batch_id, account_id, asset_id, kind, date, source_order, fingerprint,
                    external_id, adapter, adapter_version, file_sha256, locator, basis_unknown, record_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            )?;
            for a in &batch.activities {
                let r = &a.record;
                ins.execute(params![
                    batch_id,
                    a.account_id,
                    a.asset_id,
                    r.kind.as_str(),
                    r.date.to_string(),
                    a.source_order,
                    a.fingerprint,
                    r.external_id,
                    r.source.adapter,
                    r.source.adapter_version,
                    r.source.file_sha256.as_str(),
                    r.source.locator,
                    a.basis_unknown,
                    serde_json::to_string(r)?,
                ])?;
            }
            let mut ins_a = tx.prepare(
                "INSERT INTO assertions (batch_id, account_id, as_of, file_sha256, locator, assertion_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            let mut ins_r = tx.prepare(
                "INSERT INTO assertion_results (assertion_id, passed, result_json) VALUES (?1, ?2, ?3)",
            )?;
            for na in &batch.assertions {
                let a = &na.assertion;
                ins_a.execute(params![
                    batch_id,
                    na.account_id,
                    a.as_of.to_string(),
                    a.source.file_sha256.as_str(),
                    a.source.locator,
                    serde_json::to_string(a)?,
                ])?;
                if let Some(res) = &na.result {
                    ins_r.execute(params![
                        tx.last_insert_rowid(),
                        res.passed,
                        res.detail.to_string()
                    ])?;
                }
            }
        }
        tx.commit()?;
        Ok(batch_id)
    }

    // ---- activity queries ----

    const ACTIVITY_COLS: &'static str =
        "id, batch_id, account_id, asset_id, fingerprint, source_order, basis_unknown, record_json";

    fn collect(&self, sql: &str, p: impl rusqlite::Params) -> Result<Vec<StoredActivity>> {
        type RowTuple = (
            i64,
            BatchId,
            AccountId,
            Option<AssetId>,
            String,
            i64,
            bool,
            String,
        );
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(p, |r: &Row| -> rusqlite::Result<RowTuple> {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (
                id,
                batch_id,
                account_id,
                asset_id,
                fingerprint,
                source_order,
                basis_unknown,
                json,
            ) = row?;
            out.push(StoredActivity {
                id,
                batch_id,
                account_id,
                asset_id,
                fingerprint,
                source_order,
                basis_unknown,
                record: serde_json::from_str(&json)?,
            });
        }
        Ok(out)
    }

    /// Activities of an account with `from <= date <= to` (dates as `YYYY-MM-DD`), in date and source order.
    pub fn activities_between(
        &self,
        account_id: AccountId,
        from: &str,
        to: &str,
    ) -> Result<Vec<StoredActivity>> {
        self.collect(
            &format!(
                "SELECT {} FROM activities WHERE account_id = ?1 AND date >= ?2 AND date <= ?3
                 ORDER BY date, id",
                Self::ACTIVITY_COLS
            ),
            params![account_id, from, to],
        )
    }

    /// Activities of an account up to and including `date`.
    pub fn activities_until(
        &self,
        account_id: AccountId,
        date: &str,
    ) -> Result<Vec<StoredActivity>> {
        self.activities_between(account_id, "0000-01-01", date)
    }

    pub fn activities_by_fingerprint(&self, fingerprint: &str) -> Result<Vec<StoredActivity>> {
        self.collect(
            &format!(
                "SELECT {} FROM activities WHERE fingerprint = ?1 ORDER BY id",
                Self::ACTIVITY_COLS
            ),
            [fingerprint],
        )
    }

    /// Row counts per table, for tests and diagnostics.
    pub fn count(&self, table: &str) -> Result<i64> {
        const TABLES: [&str; 7] = [
            "accounts",
            "account_hints",
            "assets",
            "import_batches",
            "activities",
            "assertions",
            "assertion_results",
        ];
        assert!(TABLES.contains(&table), "unknown table {table}");
        Ok(self
            .conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?)
    }
}
