//! Per-database corrections to market data series: edited, added and ignored
//! values. The shared history cache is never written to.

use std::fmt;

use crate::{Result, StorageError, Store};

/// What a series holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SeriesKind {
    /// Daily prices: `tefas:<CODE>`, `yahoo:<SYMBOL>`, `twelvedata:<SYMBOL>`.
    Price,
    /// Daily TRY per unit of a currency, keyed by currency code.
    Rate,
    /// Monthly index values, keyed `<source>:<series>`.
    Index,
}

impl SeriesKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SeriesKind::Price => "price",
            SeriesKind::Rate => "rate",
            SeriesKind::Index => "index",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [SeriesKind::Price, SeriesKind::Rate, SeriesKind::Index]
            .into_iter()
            .find(|k| k.as_str() == s)
    }
}

impl fmt::Display for SeriesKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One edit: a value for a period, or the stored value ignored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeriesEdit {
    pub kind: SeriesKind,
    pub series: String,
    /// `YYYY-MM-DD`, or `YYYY-MM` for indices.
    pub period: String,
    /// `None` when ignored.
    pub value: Option<String>,
    pub currency: Option<String>,
    pub ignored: bool,
    pub edited_at: String,
}

/// One `series_edits` row as read: kind, series, period, value, currency,
/// ignored, edited_at.
type EditRow = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    bool,
    String,
);

fn row(r: &rusqlite::Row) -> rusqlite::Result<EditRow> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
    ))
}

const COLS: &str = "kind, series, period, value, currency, ignored, edited_at";

impl Store {
    fn edits_where(&self, filter: &str, p: impl rusqlite::Params) -> Result<Vec<SeriesEdit>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM series_edits WHERE {filter} ORDER BY kind, series, period"
        ))?;
        let rows = stmt.query_map(p, row)?;
        let mut out = Vec::new();
        for r in rows {
            let (kind, series, period, value, currency, ignored, edited_at) = r?;
            let Some(kind) = SeriesKind::parse(&kind) else {
                continue;
            };
            out.push(SeriesEdit {
                kind,
                series,
                period,
                value,
                currency,
                ignored,
                edited_at,
            });
        }
        Ok(out)
    }

    /// Edits of one series, oldest period first.
    pub fn series_edits(&self, kind: SeriesKind, series: &str) -> Result<Vec<SeriesEdit>> {
        self.edits_where(
            "kind = ?1 AND series = ?2",
            rusqlite::params![kind.as_str(), series],
        )
    }

    /// Every edit in this database.
    pub fn all_series_edits(&self) -> Result<Vec<SeriesEdit>> {
        self.edits_where("1 = 1", [])
    }

    /// Set (edit or add) a value.
    pub fn set_series_value(
        &mut self,
        kind: SeriesKind,
        series: &str,
        period: &str,
        value: &str,
        currency: Option<&str>,
        edited_at: &str,
    ) -> Result<()> {
        if series.trim().is_empty() || period.trim().is_empty() {
            return Err(StorageError::Invalid(
                "a series and a date are required".into(),
            ));
        }
        self.conn.execute(
            "INSERT INTO series_edits (kind, series, period, value, currency, ignored, edited_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6)
             ON CONFLICT(kind, series, period) DO UPDATE SET
               value = excluded.value, currency = excluded.currency, ignored = 0, edited_at = excluded.edited_at",
            rusqlite::params![kind.as_str(), series, period, value, currency, edited_at],
        )?;
        Ok(())
    }

    /// Ignore the stored value for a period.
    pub fn ignore_series_value(
        &mut self,
        kind: SeriesKind,
        series: &str,
        period: &str,
        edited_at: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO series_edits (kind, series, period, value, currency, ignored, edited_at)
             VALUES (?1, ?2, ?3, NULL, NULL, 1, ?4)
             ON CONFLICT(kind, series, period) DO UPDATE SET
               value = NULL, currency = NULL, ignored = 1, edited_at = excluded.edited_at",
            rusqlite::params![kind.as_str(), series, period, edited_at],
        )?;
        Ok(())
    }

    /// Drop the edit for a period. Returns whether there was one.
    pub fn revert_series_value(
        &mut self,
        kind: SeriesKind,
        series: &str,
        period: &str,
    ) -> Result<bool> {
        let n = self.conn.execute(
            "DELETE FROM series_edits WHERE kind = ?1 AND series = ?2 AND period = ?3",
            rusqlite::params![kind.as_str(), series, period],
        )?;
        Ok(n > 0)
    }
}

#[cfg(test)]
mod tests {
    use crate::Store;

    #[test]
    fn a_value_marked_ignored_is_rejected() {
        let s = Store::open_in_memory().unwrap();
        let r = s.conn.execute(
            "INSERT INTO series_edits (kind, series, period, value, ignored, edited_at) VALUES ('rate', 'USD', '2025-01-02', '1', 1, 't')",
            [],
        );
        assert!(r.is_err());
        let r = s.conn.execute(
            "INSERT INTO series_edits (kind, series, period, value, ignored, edited_at) VALUES ('rate', 'USD', '2025-01-02', NULL, 0, 't')",
            [],
        );
        assert!(r.is_err());
    }
}
