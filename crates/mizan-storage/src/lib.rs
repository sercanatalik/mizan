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

pub mod history;
pub mod market;
pub use history::{
    CachedFundPrice, CachedListedPrice, CachedRate, Dated, FetchStatus, FetchedUnit, HistoryCache,
};
pub use market::*;

/// Migrations in order. The database's `user_version` records how many have run.
const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_init.sql"),
    include_str!("../migrations/0002_market.sql"),
    include_str!("../migrations/0003_listed.sql"),
];

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("stored JSON could not be read: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0} not found")]
    NotFound(String),
    #[error("invalid request: {0}")]
    Invalid(String),
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

/// An account with the statement identifiers mapped to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountRow {
    pub id: AccountId,
    pub name: String,
    pub hints: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssetRow {
    pub id: AssetId,
    pub instrument_key: String,
    pub name: Option<String>,
    pub currency: Option<String>,
    pub hint: InstrumentHint,
}

/// One committed import, as listed in the import history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchRow {
    pub id: BatchId,
    pub committed_at: String,
    pub adapter: String,
    pub adapter_version: String,
    pub file_sha256: String,
    pub activity_count: i64,
    pub assertion_count: i64,
    /// `None` when the import had no balance checks.
    pub all_checks_passed: Option<bool>,
}

/// A stored balance assertion with its recorded result.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredAssertionResult {
    pub assertion_id: i64,
    pub batch_id: BatchId,
    pub account_id: AccountId,
    pub assertion: Assertion,
    pub passed: bool,
    pub detail: serde_json::Value,
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
        const TABLES: [&str; 13] = [
            "accounts",
            "account_hints",
            "assets",
            "import_batches",
            "activities",
            "assertions",
            "assertion_results",
            "settings",
            "asset_merges",
            "price_sources",
            "prices",
            "fx_rates",
            "refresh_log",
        ];
        assert!(TABLES.contains(&table), "unknown table {table}");
        Ok(self
            .conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?)
    }

    // ---- listings for the app ----

    pub fn list_accounts(&self) -> Result<Vec<AccountRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT a.id, a.name, h.hint FROM accounts a LEFT JOIN account_hints h ON h.account_id = a.id
             ORDER BY a.id, h.hint",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, AccountId>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?;
        let mut out: Vec<AccountRow> = Vec::new();
        for row in rows {
            let (id, name, hint) = row?;
            if out.last().map(|a| a.id) != Some(id) {
                out.push(AccountRow {
                    id,
                    name,
                    hints: Vec::new(),
                });
            }
            if let Some(h) = hint {
                out.last_mut().expect("pushed above").hints.push(h);
            }
        }
        Ok(out)
    }

    pub fn list_assets(&self) -> Result<Vec<AssetRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, instrument_key, name, currency, hint_json FROM assets ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, instrument_key, name, currency, json) = row?;
            out.push(AssetRow {
                id,
                instrument_key,
                name,
                currency,
                hint: serde_json::from_str(&json)?,
            });
        }
        Ok(out)
    }

    /// Every activity of an account, in date and insertion order.
    pub fn all_activities(&self, account_id: AccountId) -> Result<Vec<StoredActivity>> {
        self.activities_between(account_id, "0000-01-01", "9999-12-31")
    }

    /// Committed imports, newest first.
    pub fn list_batches(&self) -> Result<Vec<BatchRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT b.id, b.committed_at, b.adapter, b.adapter_version, b.file_sha256,
                (SELECT COUNT(*) FROM activities WHERE batch_id = b.id),
                (SELECT COUNT(*) FROM assertions WHERE batch_id = b.id),
                (SELECT MIN(r.passed) FROM assertion_results r JOIN assertions s ON s.id = r.assertion_id
                  WHERE s.batch_id = b.id)
             FROM import_batches b ORDER BY b.id DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(BatchRow {
                id: r.get(0)?,
                committed_at: r.get(1)?,
                adapter: r.get(2)?,
                adapter_version: r.get(3)?,
                file_sha256: r.get(4)?,
                activity_count: r.get(5)?,
                assertion_count: r.get(6)?,
                all_checks_passed: r.get::<_, Option<i64>>(7)?.map(|v| v == 1),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Results of an account's balance checks on its most recent check date.
    pub fn latest_assertion_results(
        &self,
        account_id: AccountId,
    ) -> Result<Vec<StoredAssertionResult>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.batch_id, s.account_id, s.assertion_json, r.passed, r.result_json
             FROM assertions s JOIN assertion_results r ON r.assertion_id = s.id
             WHERE s.account_id = ?1
               AND s.as_of = (SELECT MAX(as_of) FROM assertions WHERE account_id = ?1)
             ORDER BY s.id",
        )?;
        let rows = stmt.query_map([account_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, BatchId>(1)?,
                r.get::<_, AccountId>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, bool>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (assertion_id, batch_id, account_id, a, passed, detail) = row?;
            out.push(StoredAssertionResult {
                assertion_id,
                batch_id,
                account_id,
                assertion: serde_json::from_str(&a)?,
                passed,
                detail: serde_json::from_str(&detail)?,
            });
        }
        Ok(out)
    }

    /// Remove one import: its assertion results, assertions, activities and the batch row.
    /// Accounts, hints and assets are kept. All-or-nothing.
    pub fn undo_batch(&mut self, batch_id: BatchId) -> Result<()> {
        let tx = self.conn.transaction()?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM import_batches WHERE id = ?1)",
            [batch_id],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(StorageError::NotFound(format!("import {batch_id}")));
        }
        tx.execute(
            "DELETE FROM assertion_results WHERE assertion_id IN (SELECT id FROM assertions WHERE batch_id = ?1)",
            [batch_id],
        )?;
        tx.execute("DELETE FROM assertions WHERE batch_id = ?1", [batch_id])?;
        tx.execute("DELETE FROM activities WHERE batch_id = ?1", [batch_id])?;
        tx.execute("DELETE FROM import_batches WHERE id = ?1", [batch_id])?;
        tx.commit()?;
        Ok(())
    }
}
