//! Starting over: backups and clearing the user database.

use std::path::Path;

use crate::{Result, StorageError, Store};

/// Settings a new database starts with (migration 0002).
pub const DEFAULT_SETTINGS: [(&str, &str); 4] = [
    ("base_currency", "TRY"),
    ("refresh_on_start", "true"),
    ("provider.tefas.enabled", "true"),
    ("provider.tcmb.enabled", "true"),
];

/// What a reset clears.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetScope {
    /// Every table except `settings`.
    Portfolio,
    /// Every table, with settings back to their defaults.
    Everything,
}

/// User tables of a connection, without SQLite's own.
pub(crate) fn user_tables(conn: &rusqlite::Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type = 'table'
         AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    let names = stmt.query_map([], |r| r.get(0))?;
    Ok(names.collect::<rusqlite::Result<Vec<String>>>()?)
}

/// Write a consistent copy of the open database to `path` (`VACUUM INTO`).
pub(crate) fn vacuum_into(conn: &rusqlite::Connection, path: &Path) -> Result<()> {
    if path.exists() {
        return Err(StorageError::Invalid(format!(
            "{} already exists",
            path.display()
        )));
    }
    let p = path
        .to_str()
        .ok_or_else(|| StorageError::Invalid(format!("unusable path {}", path.display())))?;
    conn.execute("VACUUM INTO ?1", [p])?;
    Ok(())
}

impl Store {
    /// Copy the database to `path`, which must not exist yet.
    pub fn backup_to(&self, path: &Path) -> Result<()> {
        vacuum_into(&self.conn, path)
    }

    /// Clear the database in one transaction. The schema and its version stay.
    pub fn reset(&mut self, scope: ResetScope) -> Result<()> {
        let tables = user_tables(&self.conn)?;
        self.conn.pragma_update(None, "foreign_keys", false)?;
        let result = (|| -> Result<()> {
            let tx = self.conn.transaction()?;
            for t in &tables {
                if scope == ResetScope::Portfolio && t == "settings" {
                    continue;
                }
                tx.execute(&format!("DELETE FROM \"{t}\""), [])?;
            }
            if scope == ResetScope::Everything {
                for (k, v) in DEFAULT_SETTINGS {
                    tx.execute("INSERT INTO settings (key, value) VALUES (?1, ?2)", [k, v])?;
                }
            }
            let broken: i64 =
                tx.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| {
                    r.get(0)
                })?;
            if broken > 0 {
                return Err(StorageError::Invalid(
                    "the reset would leave broken references".into(),
                ));
            }
            tx.commit()?;
            Ok(())
        })();
        self.conn.pragma_update(None, "foreign_keys", true)?;
        result
    }

    /// Rows per table, for a reset preview.
    pub fn table_count(&self, table: &str) -> Result<i64> {
        if !user_tables(&self.conn)?.iter().any(|t| t == table) {
            return Ok(0);
        }
        Ok(self
            .conn
            .query_row(&format!("SELECT COUNT(*) FROM \"{table}\""), [], |r| {
                r.get(0)
            })?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_table_added_later_is_cleared_too() {
        let mut s = Store::open_in_memory().unwrap();
        s.conn
            .execute_batch("CREATE TABLE future_thing (id INTEGER PRIMARY KEY, note TEXT); INSERT INTO future_thing (note) VALUES ('x');")
            .unwrap();
        s.reset(ResetScope::Portfolio).unwrap();
        assert_eq!(s.table_count("future_thing").unwrap(), 0);
        assert_eq!(s.table_count("no_such_table").unwrap(), 0);
    }
}
