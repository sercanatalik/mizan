//! Account management: rename, identifiers, delete-when-empty, and merges
//! that move activities, balance checks and identifiers with an exact log.

use serde::{Deserialize, Serialize};

use crate::{AccountId, Result, StorageError, Store, StoredAssertionResult};

/// What a merge moves, or moved.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Moved {
    pub activities: Vec<i64>,
    pub assertions: Vec<i64>,
    pub hints: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MergeCounts {
    pub activities: i64,
    pub assertions: i64,
    pub hints: i64,
}

/// One merge from the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountMerge {
    pub id: i64,
    pub from_account: AccountId,
    pub into_account: AccountId,
    pub merged_at: String,
    pub undone_at: Option<String>,
    pub moved: MergeCounts,
}

fn ids(tx: &rusqlite::Transaction, sql: &str, account: AccountId) -> Result<Vec<i64>> {
    let mut stmt = tx.prepare(sql)?;
    let rows = stmt.query_map([account], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn exists(conn: &rusqlite::Connection, id: AccountId) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM accounts WHERE id = ?1)",
        [id],
        |r| r.get(0),
    )?)
}

fn active_merge_of(conn: &rusqlite::Connection, id: AccountId) -> Result<Option<i64>> {
    use rusqlite::OptionalExtension;
    Ok(conn
        .query_row(
            "SELECT id FROM account_merges WHERE from_account = ?1 AND undone_at IS NULL",
            [id],
            |r| r.get(0),
        )
        .optional()?)
}

impl Store {
    fn account_name(&self, id: AccountId) -> Result<String> {
        use rusqlite::OptionalExtension;
        self.conn
            .query_row("SELECT name FROM accounts WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .optional()?
            .ok_or_else(|| StorageError::NotFound(format!("account {id}")))
    }

    /// Rename an account; the name must not be empty.
    pub fn rename_account(&mut self, id: AccountId, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(StorageError::Invalid("an account needs a name".into()));
        }
        let n = self.conn.execute(
            "UPDATE accounts SET name = ?2 WHERE id = ?1",
            rusqlite::params![id, name],
        )?;
        if n == 0 {
            return Err(StorageError::NotFound(format!("account {id}")));
        }
        Ok(())
    }

    /// The provider an account gives its assets without a price source
    /// (`tefas`, `yahoo` or `twelvedata`), if set.
    pub fn account_price_source(&self, id: AccountId) -> Result<Option<String>> {
        use rusqlite::OptionalExtension;
        self.conn
            .query_row(
                "SELECT price_source FROM accounts WHERE id = ?1",
                [id],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .ok_or_else(|| StorageError::NotFound(format!("account {id}")))
    }

    /// Set (or with `None`, clear) an account's default price provider.
    pub fn set_account_price_source(
        &mut self,
        id: AccountId,
        provider: Option<&str>,
    ) -> Result<()> {
        if let Some(p) = provider {
            if !matches!(p, "tefas" | "yahoo" | "twelvedata") {
                return Err(StorageError::Invalid(format!(
                    "unknown price provider {p:?}"
                )));
            }
        }
        let n = self.conn.execute(
            "UPDATE accounts SET price_source = ?2 WHERE id = ?1",
            rusqlite::params![id, provider],
        )?;
        if n == 0 {
            return Err(StorageError::NotFound(format!("account {id}")));
        }
        Ok(())
    }

    /// Accounts with a default price provider, as (account, provider).
    pub fn account_price_sources(&self) -> Result<Vec<(AccountId, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, price_source FROM accounts WHERE price_source IS NOT NULL ORDER BY id",
        )?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// The assets an account's activities refer to, with asset merges followed
    /// to the surviving asset.
    pub fn account_asset_ids(&self, id: AccountId) -> Result<Vec<crate::AssetId>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT COALESCE(m.into_asset, a.asset_id)
               FROM activities a
               LEFT JOIN asset_merges m ON m.from_asset = a.asset_id AND m.undone_at IS NULL
              WHERE a.account_id = ?1 AND a.asset_id IS NOT NULL
              ORDER BY 1",
        )?;
        let ids = stmt
            .query_map([id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(ids)
    }

    /// Add a statement identifier. Refuses one that belongs to another account.
    pub fn add_account_hint(&mut self, id: AccountId, hint: &str) -> Result<()> {
        let hint = hint.trim();
        if hint.is_empty() {
            return Err(StorageError::Invalid(
                "an identifier must not be empty".into(),
            ));
        }
        self.account_name(id)?;
        match self.account_for_hint(hint)? {
            Some(owner) if owner == id => Ok(()),
            Some(owner) => Err(StorageError::Invalid(format!(
                "{hint} already belongs to {}; move it instead",
                self.account_name(owner)?
            ))),
            None => self.map_account_hint(hint, id),
        }
    }

    /// Move an identifier to another account (future imports only).
    pub fn move_account_hint(&mut self, hint: &str, to: AccountId) -> Result<()> {
        self.account_name(to)?;
        if self.account_for_hint(hint)?.is_none() {
            return Err(StorageError::NotFound(format!("identifier {hint}")));
        }
        self.map_account_hint(hint, to)
    }

    /// Why an account cannot be deleted, or `None` when it can.
    pub fn delete_blocker(&self, id: AccountId) -> Result<Option<String>> {
        let count =
            |sql: &str| -> Result<i64> { Ok(self.conn.query_row(sql, [id], |r| r.get(0))?) };
        let activities = count("SELECT COUNT(*) FROM activities WHERE account_id = ?1")?;
        if activities > 0 {
            return Ok(Some(format!(
                "it has {activities} activities; merge it into another account instead"
            )));
        }
        if count("SELECT COUNT(*) FROM assertions WHERE account_id = ?1")? > 0 {
            return Ok(Some(
                "it has balance checks; undo their import first".into(),
            ));
        }
        if count(
            "SELECT COUNT(*) FROM account_merges WHERE from_account = ?1 OR into_account = ?1",
        )? > 0
        {
            return Ok(Some("it is part of a merge".into()));
        }
        Ok(None)
    }

    /// Delete an empty account and its identifiers.
    pub fn delete_account(&mut self, id: AccountId) -> Result<()> {
        self.account_name(id)?;
        if let Some(why) = self.delete_blocker(id)? {
            return Err(StorageError::Invalid(format!(
                "cannot delete this account: {why}"
            )));
        }
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM account_hints WHERE account_id = ?1", [id])?;
        tx.execute("DELETE FROM accounts WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    /// Every balance check of an account with its result, newest first.
    pub fn assertion_history(&self, account: AccountId) -> Result<Vec<StoredAssertionResult>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.batch_id, s.account_id, s.assertion_json, r.passed, r.result_json
             FROM assertions s JOIN assertion_results r ON r.assertion_id = s.id
             WHERE s.account_id = ?1 ORDER BY s.as_of DESC, s.id",
        )?;
        let rows = stmt.query_map([account], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
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

    /// What merging `from` would move.
    pub fn merge_preview(&self, from: AccountId) -> Result<MergeCounts> {
        let count =
            |sql: &str| -> Result<i64> { Ok(self.conn.query_row(sql, [from], |r| r.get(0))?) };
        Ok(MergeCounts {
            activities: count("SELECT COUNT(*) FROM activities WHERE account_id = ?1")?,
            assertions: count("SELECT COUNT(*) FROM assertions WHERE account_id = ?1")?,
            hints: count("SELECT COUNT(*) FROM account_hints WHERE account_id = ?1")?,
        })
    }

    /// Move `from`'s activities, balance checks and identifiers into `into`,
    /// logging exactly what moved. Returns the merge id.
    pub fn merge_accounts(&mut self, from: AccountId, into: AccountId, now: &str) -> Result<i64> {
        if from == into {
            return Err(StorageError::Invalid(
                "an account cannot be merged into itself".into(),
            ));
        }
        for id in [from, into] {
            if !exists(&self.conn, id)? {
                return Err(StorageError::NotFound(format!("account {id}")));
            }
            if active_merge_of(&self.conn, id)?.is_some() {
                return Err(StorageError::Invalid(format!(
                    "{} is already merged into another account",
                    self.account_name(id)?
                )));
            }
        }
        let tx = self.conn.transaction()?;
        let moved = Moved {
            activities: ids(
                &tx,
                "SELECT id FROM activities WHERE account_id = ?1 ORDER BY id",
                from,
            )?,
            assertions: ids(
                &tx,
                "SELECT id FROM assertions WHERE account_id = ?1 ORDER BY id",
                from,
            )?,
            hints: {
                let mut stmt = tx.prepare(
                    "SELECT hint FROM account_hints WHERE account_id = ?1 ORDER BY hint",
                )?;
                let rows = stmt.query_map([from], |r| r.get(0))?;
                rows.collect::<rusqlite::Result<_>>()?
            },
        };
        tx.execute(
            "UPDATE activities SET account_id = ?2 WHERE account_id = ?1",
            rusqlite::params![from, into],
        )?;
        tx.execute(
            "UPDATE assertions SET account_id = ?2 WHERE account_id = ?1",
            rusqlite::params![from, into],
        )?;
        tx.execute(
            "UPDATE account_hints SET account_id = ?2 WHERE account_id = ?1",
            rusqlite::params![from, into],
        )?;
        tx.execute(
            "INSERT INTO account_merges (from_account, into_account, merged_at, moved_json) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![from, into, now, serde_json::to_string(&moved)?],
        )?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }

    /// Move back what a merge moved, if it is still in the target. Returns the
    /// identifiers that had since moved elsewhere and were left alone.
    pub fn undo_account_merge(&mut self, merge_id: i64, now: &str) -> Result<Vec<String>> {
        use rusqlite::OptionalExtension;
        let row: Option<(AccountId, AccountId, Option<String>, String)> = self
            .conn
            .query_row(
                "SELECT from_account, into_account, undone_at, moved_json FROM account_merges WHERE id = ?1",
                [merge_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let Some((from, into, undone, json)) = row else {
            return Err(StorageError::NotFound(format!("merge {merge_id}")));
        };
        if undone.is_some() {
            return Err(StorageError::Invalid(
                "this merge was already undone".into(),
            ));
        }
        if active_merge_of(&self.conn, into)?.is_some() {
            return Err(StorageError::Invalid(format!(
                "{} was merged into another account since; undo that merge first",
                self.account_name(into)?
            )));
        }
        let moved: Moved = serde_json::from_str(&json)?;
        let tx = self.conn.transaction()?;
        for id in &moved.activities {
            tx.execute(
                "UPDATE activities SET account_id = ?1 WHERE id = ?2 AND account_id = ?3",
                rusqlite::params![from, id, into],
            )?;
        }
        for id in &moved.assertions {
            tx.execute(
                "UPDATE assertions SET account_id = ?1 WHERE id = ?2 AND account_id = ?3",
                rusqlite::params![from, id, into],
            )?;
        }
        let mut skipped = Vec::new();
        for h in &moved.hints {
            let n = tx.execute(
                "UPDATE account_hints SET account_id = ?1 WHERE hint = ?2 AND account_id = ?3",
                rusqlite::params![from, h, into],
            )?;
            if n == 0 {
                skipped.push(h.clone());
            }
        }
        tx.execute(
            "UPDATE account_merges SET undone_at = ?2 WHERE id = ?1",
            rusqlite::params![merge_id, now],
        )?;
        tx.commit()?;
        Ok(skipped)
    }

    /// Every merge, newest first.
    pub fn account_merges(&self) -> Result<Vec<AccountMerge>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, from_account, into_account, merged_at, undone_at, moved_json FROM account_merges ORDER BY id DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, AccountId>(1)?,
                r.get::<_, AccountId>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, from_account, into_account, merged_at, undone_at, json) = row?;
            let m: Moved = serde_json::from_str(&json)?;
            out.push(AccountMerge {
                id,
                from_account,
                into_account,
                merged_at,
                undone_at,
                moved: MergeCounts {
                    activities: m.activities.len() as i64,
                    assertions: m.assertions.len() as i64,
                    hints: m.hints.len() as i64,
                },
            });
        }
        Ok(out)
    }
}
