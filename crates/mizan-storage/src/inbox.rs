//! Snoozed Inbox items.

use rusqlite::params;

use crate::{Result, Store};

/// One snoozed item: hidden until `until` (RFC 3339, UTC) while its figures
/// still match `fingerprint`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snooze {
    pub key: String,
    pub fingerprint: String,
    pub until: String,
}

impl Store {
    /// Snooze (or re-snooze) an item.
    pub fn snooze(&mut self, key: &str, fingerprint: &str, until: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO inbox_snoozes (key, fingerprint, until) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET fingerprint = excluded.fingerprint, until = excluded.until",
            params![key, fingerprint, until],
        )?;
        Ok(())
    }

    pub fn unsnooze(&mut self, key: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM inbox_snoozes WHERE key = ?1", [key])?;
        Ok(())
    }

    /// Every stored snooze, by key.
    pub fn snoozes(&self) -> Result<Vec<Snooze>> {
        let mut stmt = self
            .conn
            .prepare("SELECT key, fingerprint, until FROM inbox_snoozes ORDER BY key")?;
        let rows = stmt.query_map([], |r| {
            Ok(Snooze {
                key: r.get(0)?,
                fingerprint: r.get(1)?,
                until: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Drop snoozes that ended at or before `now`, and those whose key is not
    /// in `live` with the same fingerprint (the item resolved or its figures
    /// changed). Returns the snoozes still in force.
    pub fn purge_snoozes(&mut self, now: &str, live: &[(String, String)]) -> Result<Vec<Snooze>> {
        let mut keep = Vec::new();
        for s in self.snoozes()? {
            let current = live.iter().any(|(k, f)| *k == s.key && *f == s.fingerprint);
            if s.until.as_str() > now && current {
                keep.push(s);
            } else {
                self.unsnooze(&s.key)?;
            }
        }
        Ok(keep)
    }
}
