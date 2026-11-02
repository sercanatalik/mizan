//! The market-history cache: historical fund prices, listed-asset closes and
//! exchange rates kept
//! once per machine in `~/.mizan/market-history.sqlite` (or under
//! `$MIZAN_HOME`), shared by every database.
//!
//! It holds public market data only. Nothing from a user database (accounts,
//! hints, quantities, activities) is ever written here.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

use crate::Result;

pub const CACHE_FILE: &str = "market-history.sqlite";
pub const HOME_ENV: &str = "MIZAN_HOME";
pub const HOME_DIR: &str = ".mizan";

const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/history/0001_init.sql"),
    include_str!("../migrations/history/0002_listed.sql"),
    include_str!("../migrations/history/0003_inflation.sql"),
];

/// The folder for machine-wide Mizan data: `$MIZAN_HOME` when set and not
/// empty, otherwise `.mizan` in the user's home directory.
pub fn mizan_home() -> Option<PathBuf> {
    let user_home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    resolve_home(std::env::var_os(HOME_ENV), user_home)
}

/// [`mizan_home`] with its inputs passed in, for tests.
pub fn resolve_home(mizan_home: Option<OsString>, user_home: Option<PathBuf>) -> Option<PathBuf> {
    match mizan_home.filter(|v| !v.is_empty()) {
        Some(dir) => Some(PathBuf::from(dir)),
        None => user_home.map(|h| h.join(HOME_DIR)),
    }
}

/// Outcome of fetching one unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FetchStatus {
    /// Data was returned and stored.
    Ok,
    /// The provider published nothing for this unit (weekend, holiday, no prices yet).
    None,
    /// The provider refuses this range (for example TEFAS before five years ago).
    TooOld,
}

impl FetchStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            FetchStatus::Ok => "ok",
            FetchStatus::None => "none",
            FetchStatus::TooOld => "too_old",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "ok" => Some(FetchStatus::Ok),
            "none" => Some(FetchStatus::None),
            "too_old" => Some(FetchStatus::TooOld),
            _ => None,
        }
    }
}

/// A dated decimal value (price or rate) as stored text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dated {
    pub date: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedFundPrice {
    pub code: String,
    pub date: String,
    pub price: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedRate {
    pub currency: String,
    pub date: String,
    pub try_per_unit: String,
    pub source: String,
}

/// A cached daily close of a listed stock or ETF.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedListedPrice {
    /// Provider id (`yahoo`, `twelvedata`).
    pub source: String,
    pub symbol: String,
    pub date: String,
    pub close: String,
    pub currency: String,
}

/// One fetched unit and the data it returned, committed together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchedUnit {
    pub source: String,
    pub key: String,
    pub period: String,
    pub status: FetchStatus,
    pub fund_prices: Vec<CachedFundPrice>,
    pub rates: Vec<CachedRate>,
    pub fetched_at: String,
}

/// One cached monthly value of an inflation index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedIndexValue {
    /// Month as `YYYY-MM`.
    pub month: String,
    pub value: String,
}

/// One cached series summarised, for browsing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedSeries {
    /// Provider id: `tefas`, `yahoo`, `twelvedata`, `tcmb` or an index source.
    pub source: String,
    /// Fund code, provider symbol, currency code or index series id.
    pub id: String,
    /// Currency of the values, when the series has one.
    pub currency: Option<String>,
    pub count: i64,
    /// First and last date (or month).
    pub first: String,
    pub last: String,
    /// The value on `last`.
    pub latest: String,
    /// Values dated Monday to Friday (daily series only).
    pub weekday_values: i64,
}

/// One cached value of a series.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedPoint {
    /// Date, or month for indices.
    pub period: String,
    pub value: String,
    pub currency: Option<String>,
    pub fetched_at: String,
}

/// The last request for an inflation series.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InflationCheck {
    pub checked_at: String,
    /// The request succeeded.
    pub ok: bool,
}

pub struct HistoryCache {
    conn: Connection,
    path: Option<PathBuf>,
}

impl HistoryCache {
    /// Open (creating if needed) the cache file in `dir`.
    pub fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir).map_err(|e| {
            crate::StorageError::Invalid(format!("cannot create {}: {e}", dir.display()))
        })?;
        let path = dir.join(CACHE_FILE);
        let conn = Connection::open(&path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let mut c = HistoryCache {
            conn,
            path: Some(path),
        };
        c.migrate()?;
        Ok(c)
    }

    pub fn open_in_memory() -> Result<Self> {
        let mut c = HistoryCache {
            conn: Connection::open_in_memory()?,
            path: None,
        };
        c.migrate()?;
        Ok(c)
    }

    /// The cache file, or `None` for an in-memory cache.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
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

    pub fn schema_version(&self) -> Result<i64> {
        Ok(self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))?)
    }

    // ---- writes ----

    pub fn upsert_fund_price(&mut self, p: &CachedFundPrice, fetched_at: &str) -> Result<()> {
        upsert_fund_price(&self.conn, p, fetched_at)
    }

    pub fn upsert_rate(&mut self, r: &CachedRate, fetched_at: &str) -> Result<()> {
        upsert_rate(&self.conn, r, fetched_at)
    }

    pub fn set_fund_type(&mut self, code: &str, fund_type: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO fund_types (code, fund_type) VALUES (?1, ?2)
             ON CONFLICT(code) DO UPDATE SET fund_type = excluded.fund_type",
            params![code, fund_type],
        )?;
        Ok(())
    }

    /// Store a unit's data and mark it fetched, in one transaction.
    pub fn commit_unit(&mut self, u: &FetchedUnit) -> Result<()> {
        let tx = self.conn.transaction()?;
        for p in &u.fund_prices {
            upsert_fund_price(&tx, p, &u.fetched_at)?;
        }
        for r in &u.rates {
            upsert_rate(&tx, r, &u.fetched_at)?;
        }
        tx.execute(
            "INSERT INTO fetched (source, key, period, status, fetched_at) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(source, key, period) DO UPDATE SET
                status = excluded.status, fetched_at = excluded.fetched_at",
            params![u.source, u.key, u.period, u.status.as_str(), u.fetched_at],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn upsert_listed_price(&mut self, p: &CachedListedPrice, fetched_at: &str) -> Result<()> {
        upsert_listed_price(&self.conn, p, fetched_at)
    }

    /// Store a listed symbol's closes and mark each of `years` (`YYYY`) fetched
    /// with its status, in one transaction.
    pub fn commit_listed(
        &mut self,
        source: &str,
        symbol: &str,
        years: &[(String, FetchStatus)],
        prices: &[CachedListedPrice],
        fetched_at: &str,
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        for p in prices {
            upsert_listed_price(&tx, p, fetched_at)?;
        }
        for (year, status) in years {
            tx.execute(
                "INSERT INTO fetched (source, key, period, status, fetched_at) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(source, key, period) DO UPDATE SET
                    status = excluded.status, fetched_at = excluded.fetched_at",
                params![source, symbol, year, status.as_str(), fetched_at],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Store an inflation series' monthly values and record the request, in one
    /// transaction. A failed request records only the check; values stay as they were.
    pub fn commit_inflation(
        &mut self,
        source: &str,
        series: &str,
        values: &[CachedIndexValue],
        ok: bool,
        checked_at: &str,
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        for v in values {
            tx.execute(
                "INSERT INTO inflation_index (source, series, month, value, fetched_at) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(source, series, month) DO UPDATE SET
                    value = excluded.value, fetched_at = excluded.fetched_at",
                params![source, series, v.month, v.value, checked_at],
            )?;
        }
        tx.execute(
            "INSERT INTO inflation_checks (source, series, checked_at, status) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(source, series) DO UPDATE SET
                checked_at = excluded.checked_at, status = excluded.status",
            params![source, series, checked_at, if ok { "ok" } else { "failed" }],
        )?;
        tx.commit()?;
        Ok(())
    }

    // ---- reads ----

    /// Every cached month of an inflation series, oldest first.
    pub fn inflation_index(&self, source: &str, series: &str) -> Result<Vec<CachedIndexValue>> {
        let mut stmt = self.conn.prepare(
            "SELECT month, value FROM inflation_index WHERE source = ?1 AND series = ?2 ORDER BY month",
        )?;
        let rows = stmt.query_map(params![source, series], |r| {
            Ok(CachedIndexValue {
                month: r.get(0)?,
                value: r.get(1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The last request for an inflation series, if any.
    pub fn inflation_check(&self, source: &str, series: &str) -> Result<Option<InflationCheck>> {
        Ok(self
            .conn
            .query_row(
                "SELECT checked_at, status FROM inflation_checks WHERE source = ?1 AND series = ?2",
                params![source, series],
                |r| {
                    Ok(InflationCheck {
                        checked_at: r.get(0)?,
                        ok: r.get::<_, String>(1)? == "ok",
                    })
                },
            )
            .optional()?)
    }

    /// Every cached close of a listed symbol from one provider, oldest first.
    pub fn listed_prices(&self, source: &str, symbol: &str) -> Result<Vec<CachedListedPrice>> {
        let mut stmt = self.conn.prepare(
            "SELECT date, close, currency FROM listed_prices WHERE source = ?1 AND symbol = ?2 ORDER BY date",
        )?;
        let rows = stmt.query_map(params![source, symbol], |r| {
            Ok(CachedListedPrice {
                source: source.to_owned(),
                symbol: symbol.to_owned(),
                date: r.get(0)?,
                close: r.get(1)?,
                currency: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn fund_type(&self, code: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT fund_type FROM fund_types WHERE code = ?1",
                [code],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Every cached price of a fund, oldest first.
    pub fn fund_prices(&self, code: &str) -> Result<Vec<Dated>> {
        self.dated(
            "SELECT date, price FROM fund_prices WHERE code = ?1 ORDER BY date",
            code,
        )
    }

    /// Every cached rate of a currency (TRY per unit), oldest first.
    pub fn rates(&self, currency: &str) -> Result<Vec<Dated>> {
        self.dated(
            "SELECT date, try_per_unit FROM fx_rates WHERE currency = ?1 ORDER BY date",
            currency,
        )
    }

    /// Currencies with at least one cached rate.
    pub fn rate_currencies(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT currency FROM fx_rates ORDER BY currency")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Newest cached fund price dated on or before `date`.
    pub fn fund_price_on(&self, code: &str, date: &str) -> Result<Option<Dated>> {
        Ok(self
            .conn
            .query_row(
                "SELECT date, price FROM fund_prices WHERE code = ?1 AND date <= ?2
                 ORDER BY date DESC LIMIT 1",
                params![code, date],
                |r| {
                    Ok(Dated {
                        date: r.get(0)?,
                        value: r.get(1)?,
                    })
                },
            )
            .optional()?)
    }

    /// Fetched units for one (source, key): period → status.
    pub fn fetched(&self, source: &str, key: &str) -> Result<BTreeMap<String, FetchStatus>> {
        let mut stmt = self
            .conn
            .prepare("SELECT period, status FROM fetched WHERE source = ?1 AND key = ?2")?;
        let rows = stmt.query_map(params![source, key], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut out = BTreeMap::new();
        for row in rows {
            let (period, status) = row?;
            if let Some(s) = FetchStatus::parse(&status) {
                out.insert(period, s);
            }
        }
        Ok(out)
    }

    /// The periods of `planned` not yet fetched for (source, key), in the given order.
    pub fn missing(&self, source: &str, key: &str, planned: &[String]) -> Result<Vec<String>> {
        let done = self.fetched(source, key)?;
        Ok(planned
            .iter()
            .filter(|p| !done.contains_key(*p))
            .cloned()
            .collect())
    }

    /// Every table and its columns, for inspection and the privacy test.
    pub fn schema_columns(&self) -> Result<Vec<(String, Vec<String>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )?;
        let tables: Vec<String> = stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let mut out = Vec::new();
        for t in tables {
            let mut s = self.conn.prepare(&format!("PRAGMA table_info({t})"))?;
            let cols: Vec<String> = s
                .query_map([], |r| r.get(1))?
                .collect::<rusqlite::Result<_>>()?;
            out.push((t, cols));
        }
        Ok(out)
    }

    /// Every text value stored in the cache, for the privacy test.
    pub fn all_text_values(&self) -> Result<Vec<String>> {
        let mut out = Vec::new();
        for (table, cols) in self.schema_columns()? {
            let sql = format!("SELECT {} FROM {table}", cols.join(", "));
            let mut stmt = self.conn.prepare(&sql)?;
            let n = cols.len();
            let rows = stmt.query_map([], |r| {
                (0..n)
                    .map(|i| r.get::<_, rusqlite::types::Value>(i))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })?;
            for row in rows {
                for v in row? {
                    if let rusqlite::types::Value::Text(s) = v {
                        out.push(s);
                    }
                }
            }
        }
        Ok(out)
    }

    fn dated(&self, sql: &str, key: &str) -> Result<Vec<Dated>> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map([key], |r| {
            Ok(Dated {
                date: r.get(0)?,
                value: r.get(1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

fn upsert_fund_price(conn: &Connection, p: &CachedFundPrice, fetched_at: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO fund_prices (code, date, price, fetched_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(code, date) DO UPDATE SET price = excluded.price, fetched_at = excluded.fetched_at",
        params![p.code, p.date, p.price, fetched_at],
    )?;
    Ok(())
}

fn upsert_listed_price(conn: &Connection, p: &CachedListedPrice, fetched_at: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO listed_prices (source, symbol, date, close, currency, fetched_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(source, symbol, date) DO UPDATE SET
            close = excluded.close, currency = excluded.currency, fetched_at = excluded.fetched_at",
        params![p.source, p.symbol, p.date, p.close, p.currency, fetched_at],
    )?;
    Ok(())
}

fn upsert_rate(conn: &Connection, r: &CachedRate, fetched_at: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO fx_rates (currency, date, try_per_unit, source, fetched_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(currency, date) DO UPDATE SET
            try_per_unit = excluded.try_per_unit, source = excluded.source, fetched_at = excluded.fetched_at",
        params![r.currency, r.date, r.try_per_unit, r.source, fetched_at],
    )?;
    Ok(())
}

const WEEKDAY: &str = "SUM(CASE WHEN strftime('%w', date) IN ('0', '6') THEN 0 ELSE 1 END)";

/// Browsing: series summaries and whole series. Read-only.
impl HistoryCache {
    fn summaries(&self, sql: &str) -> Result<Vec<CachedSeries>> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map([], |r| {
            Ok(CachedSeries {
                source: r.get(0)?,
                id: r.get(1)?,
                currency: r.get(2)?,
                count: r.get(3)?,
                first: r.get(4)?,
                last: r.get(5)?,
                weekday_values: r.get(6)?,
                latest: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Every cached TEFAS fund series.
    pub fn fund_series(&self) -> Result<Vec<CachedSeries>> {
        self.summaries(&format!(
            "SELECT 'tefas', code, 'TRY', COUNT(*), MIN(date), MAX(date), {WEEKDAY},
                (SELECT price FROM fund_prices l WHERE l.code = f.code ORDER BY date DESC LIMIT 1)
             FROM fund_prices f GROUP BY code ORDER BY code"
        ))
    }

    /// Every cached listed series (provider and symbol).
    pub fn listed_series(&self) -> Result<Vec<CachedSeries>> {
        self.summaries(&format!(
            "SELECT source, symbol,
                (SELECT currency FROM listed_prices l WHERE l.source = f.source AND l.symbol = f.symbol ORDER BY date DESC LIMIT 1),
                COUNT(*), MIN(date), MAX(date), {WEEKDAY},
                (SELECT close FROM listed_prices l WHERE l.source = f.source AND l.symbol = f.symbol ORDER BY date DESC LIMIT 1)
             FROM listed_prices f GROUP BY source, symbol ORDER BY symbol, source"
        ))
    }

    /// Every cached exchange-rate series (TRY per unit).
    pub fn rate_series(&self) -> Result<Vec<CachedSeries>> {
        self.summaries(&format!(
            "SELECT 'tcmb', currency, 'TRY', COUNT(*), MIN(date), MAX(date), {WEEKDAY},
                (SELECT try_per_unit FROM fx_rates l WHERE l.currency = f.currency ORDER BY date DESC LIMIT 1)
             FROM fx_rates f GROUP BY currency ORDER BY currency"
        ))
    }

    /// Every cached index series (monthly).
    pub fn index_series(&self) -> Result<Vec<CachedSeries>> {
        self.summaries(
            "SELECT source, series, NULL, COUNT(*), MIN(month), MAX(month), 0,
                (SELECT value FROM inflation_index l WHERE l.source = f.source AND l.series = f.series ORDER BY month DESC LIMIT 1)
             FROM inflation_index f GROUP BY source, series ORDER BY source, series",
        )
    }

    fn points(&self, sql: &str, p: impl rusqlite::Params) -> Result<Vec<CachedPoint>> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(p, |r| {
            Ok(CachedPoint {
                period: r.get(0)?,
                value: r.get(1)?,
                currency: r.get(2)?,
                fetched_at: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// A fund's cached prices with fetch times, oldest first.
    pub fn fund_points(&self, code: &str) -> Result<Vec<CachedPoint>> {
        self.points(
            "SELECT date, price, 'TRY', fetched_at FROM fund_prices WHERE code = ?1 ORDER BY date",
            [code],
        )
    }

    /// A listed symbol's cached closes with fetch times, oldest first.
    pub fn listed_points(&self, source: &str, symbol: &str) -> Result<Vec<CachedPoint>> {
        self.points(
            "SELECT date, close, currency, fetched_at FROM listed_prices WHERE source = ?1 AND symbol = ?2 ORDER BY date",
            [source, symbol],
        )
    }

    /// A currency's cached rates with fetch times, oldest first.
    pub fn rate_points(&self, currency: &str) -> Result<Vec<CachedPoint>> {
        self.points(
            "SELECT date, try_per_unit, 'TRY', fetched_at FROM fx_rates WHERE currency = ?1 ORDER BY date",
            [currency],
        )
    }

    /// An index series' cached months with fetch times, oldest first.
    pub fn index_points(&self, source: &str, series: &str) -> Result<Vec<CachedPoint>> {
        self.points(
            "SELECT month, value, NULL, fetched_at FROM inflation_index WHERE source = ?1 AND series = ?2 ORDER BY month",
            [source, series],
        )
    }
}

/// Starting over: a backup copy and clearing every row.
impl HistoryCache {
    /// Copy the cache to `path`, which must not exist yet.
    pub fn backup_to(&self, path: &Path) -> Result<()> {
        crate::reset::vacuum_into(&self.conn, path)
    }

    /// Delete every row of every table in one transaction; the schema stays.
    pub fn clear(&mut self) -> Result<()> {
        let tables = crate::reset::user_tables(&self.conn)?;
        let tx = self.conn.transaction()?;
        for t in tables {
            tx.execute(&format!("DELETE FROM \"{t}\""), [])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Rows in the cache's value tables, for a reset preview.
    pub fn value_count(&self) -> Result<i64> {
        let mut n = 0;
        for t in [
            "fund_prices",
            "listed_prices",
            "fx_rates",
            "inflation_index",
        ] {
            n += self
                .conn
                .query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| {
                    r.get::<_, i64>(0)
                })?;
        }
        Ok(n)
    }
}
