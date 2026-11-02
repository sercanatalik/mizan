//! Settings, price sources, prices, exchange rates, asset merges and the refresh log.

use std::collections::HashMap;

use rusqlite::{params, OptionalExtension};

use crate::{AssetId, Result, StorageError, Store};

pub const MANUAL: &str = "manual";

/// Where an asset's prices come from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PriceSourceKind {
    Tefas {
        fund_code: String,
    },
    /// Yahoo Finance, by Yahoo symbol (for example `VWCE.DE`).
    Yahoo {
        symbol: String,
    },
    /// Twelve Data, by Twelve Data symbol.
    TwelveData {
        symbol: String,
    },
    Manual,
}

impl PriceSourceKind {
    /// Provider id as stored (`tefas`, `yahoo`, `twelvedata`, `manual`).
    pub fn provider(&self) -> &'static str {
        match self {
            PriceSourceKind::Tefas { .. } => "tefas",
            PriceSourceKind::Yahoo { .. } => "yahoo",
            PriceSourceKind::TwelveData { .. } => "twelvedata",
            PriceSourceKind::Manual => MANUAL,
        }
    }

    /// Fund code or provider symbol; `None` for manual.
    pub fn symbol(&self) -> Option<&str> {
        match self {
            PriceSourceKind::Tefas { fund_code } => Some(fund_code),
            PriceSourceKind::Yahoo { symbol } | PriceSourceKind::TwelveData { symbol } => {
                Some(symbol)
            }
            PriceSourceKind::Manual => None,
        }
    }

    /// For listed providers: (provider id, symbol).
    pub fn listed(&self) -> Option<(&'static str, &str)> {
        match self {
            PriceSourceKind::Yahoo { symbol } | PriceSourceKind::TwelveData { symbol } => {
                Some((self.provider(), symbol))
            }
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PriceRow {
    pub asset_id: AssetId,
    pub date: String,
    pub source: String,
    pub value: String,
    pub currency: String,
    pub fetched_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FxRow {
    pub currency: String,
    pub date: String,
    pub source: String,
    pub try_per_unit: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeRow {
    pub id: i64,
    pub from_asset: AssetId,
    pub into_asset: AssetId,
    pub created_at: String,
}

impl Store {
    // ---- settings ----

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    pub fn set_setting(&mut self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    // ---- price sources ----

    pub fn price_source(&self, asset_id: AssetId) -> Result<Option<PriceSourceKind>> {
        let row: Option<(String, Option<String>)> = self
            .conn
            .query_row(
                "SELECT provider, symbol FROM price_sources WHERE asset_id = ?1",
                [asset_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(row.map(|(p, s)| match p.as_str() {
            "tefas" => PriceSourceKind::Tefas {
                fund_code: s.unwrap_or_default(),
            },
            "yahoo" => PriceSourceKind::Yahoo {
                symbol: s.unwrap_or_default(),
            },
            "twelvedata" => PriceSourceKind::TwelveData {
                symbol: s.unwrap_or_default(),
            },
            _ => PriceSourceKind::Manual,
        }))
    }

    /// Set (or with `None`, clear) an asset's price source. Stored prices are kept.
    pub fn set_price_source(
        &mut self,
        asset_id: AssetId,
        source: Option<&PriceSourceKind>,
    ) -> Result<()> {
        match source {
            None => {
                self.conn
                    .execute("DELETE FROM price_sources WHERE asset_id = ?1", [asset_id])?;
            }
            Some(s) => {
                let provider = s.provider();
                let symbol = s.symbol().map(|v| v.trim().to_uppercase());
                if symbol.as_deref() == Some("") {
                    return Err(StorageError::Invalid(format!(
                        "a {provider} price source needs a symbol"
                    )));
                }
                self.conn.execute(
                    "INSERT INTO price_sources (asset_id, provider, symbol) VALUES (?1, ?2, ?3)
                     ON CONFLICT(asset_id) DO UPDATE SET provider = excluded.provider, symbol = excluded.symbol",
                    params![asset_id, provider, symbol],
                )?;
            }
        }
        Ok(())
    }

    pub fn price_sources(&self) -> Result<HashMap<AssetId, PriceSourceKind>> {
        let mut stmt = self.conn.prepare("SELECT asset_id FROM price_sources")?;
        let ids: Vec<AssetId> = stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let mut out = HashMap::new();
        for id in ids {
            if let Some(s) = self.price_source(id)? {
                out.insert(id, s);
            }
        }
        Ok(out)
    }

    // ---- prices ----

    /// Insert or replace a provider price for (asset, date, source). Never touches manual rows.
    pub fn upsert_provider_price(&mut self, row: &PriceRow) -> Result<()> {
        if row.source == MANUAL {
            return Err(StorageError::Invalid(
                "manual prices are written with add_manual_price".into(),
            ));
        }
        self.conn.execute(
            "INSERT INTO prices (asset_id, date, source, value, currency, fetched_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(asset_id, date, source) DO UPDATE SET
                value = excluded.value, currency = excluded.currency, fetched_at = excluded.fetched_at",
            params![row.asset_id, row.date, row.source, row.value, row.currency, row.fetched_at],
        )?;
        Ok(())
    }

    pub fn add_manual_price(
        &mut self,
        asset_id: AssetId,
        date: &str,
        value: &str,
        currency: &str,
        now: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO prices (asset_id, date, source, value, currency, fetched_at) VALUES (?1, ?2, 'manual', ?3, ?4, ?5)
             ON CONFLICT(asset_id, date, source) DO UPDATE SET
                value = excluded.value, currency = excluded.currency, fetched_at = excluded.fetched_at",
            params![asset_id, date, value, currency, now],
        )?;
        Ok(())
    }

    pub fn delete_manual_price(&mut self, asset_id: AssetId, date: &str) -> Result<()> {
        let n = self.conn.execute(
            "DELETE FROM prices WHERE asset_id = ?1 AND date = ?2 AND source = 'manual'",
            params![asset_id, date],
        )?;
        if n == 0 {
            return Err(StorageError::NotFound(format!(
                "manual price for asset {asset_id} on {date}"
            )));
        }
        Ok(())
    }

    /// The newest price dated on or before `date`; on a date tie a manual price wins.
    pub fn latest_price(&self, asset_id: AssetId, on_or_before: &str) -> Result<Option<PriceRow>> {
        Ok(self
            .conn
            .query_row(
                "SELECT asset_id, date, source, value, currency, fetched_at FROM prices
                 WHERE asset_id = ?1 AND date <= ?2
                 ORDER BY date DESC, (source = 'manual') DESC, source
                 LIMIT 1",
                params![asset_id, on_or_before],
                |r| {
                    Ok(PriceRow {
                        asset_id: r.get(0)?,
                        date: r.get(1)?,
                        source: r.get(2)?,
                        value: r.get(3)?,
                        currency: r.get(4)?,
                        fetched_at: r.get(5)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn prices_for(&self, asset_id: AssetId) -> Result<Vec<PriceRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT asset_id, date, source, value, currency, fetched_at FROM prices WHERE asset_id = ?1
             ORDER BY date DESC, source",
        )?;
        let rows = stmt.query_map([asset_id], |r| {
            Ok(PriceRow {
                asset_id: r.get(0)?,
                date: r.get(1)?,
                source: r.get(2)?,
                value: r.get(3)?,
                currency: r.get(4)?,
                fetched_at: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // ---- exchange rates ----

    pub fn upsert_fx_rate(&mut self, row: &FxRow, fetched_at: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO fx_rates (currency, date, source, try_per_unit, fetched_at) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(currency, date, source) DO UPDATE SET
                try_per_unit = excluded.try_per_unit, fetched_at = excluded.fetched_at",
            params![row.currency, row.date, row.source, row.try_per_unit, fetched_at],
        )?;
        Ok(())
    }

    /// The newest TRY-per-unit rate for `currency` dated on or before `date`.
    pub fn latest_fx_rate(&self, currency: &str, on_or_before: &str) -> Result<Option<FxRow>> {
        Ok(self
            .conn
            .query_row(
                "SELECT currency, date, source, try_per_unit FROM fx_rates
                 WHERE currency = ?1 AND date <= ?2 ORDER BY date DESC, source LIMIT 1",
                params![currency, on_or_before],
                |r| {
                    Ok(FxRow {
                        currency: r.get(0)?,
                        date: r.get(1)?,
                        source: r.get(2)?,
                        try_per_unit: r.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    /// Every stored rate of `currency`, oldest first (one per date and source).
    pub fn fx_rates_for(&self, currency: &str) -> Result<Vec<FxRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT currency, date, source, try_per_unit FROM fx_rates WHERE currency = ?1
             ORDER BY date, source",
        )?;
        let rows = stmt.query_map([currency], |r| {
            Ok(FxRow {
                currency: r.get(0)?,
                date: r.get(1)?,
                source: r.get(2)?,
                try_per_unit: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Currencies with at least one stored rate.
    pub fn fx_currencies(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT currency FROM fx_rates ORDER BY currency")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // ---- asset merges ----

    /// Merge `from` into `into`. Fails if either is already merged away or the merge would form a cycle.
    pub fn merge_assets(&mut self, from: AssetId, into: AssetId) -> Result<i64> {
        if from == into {
            return Err(StorageError::Invalid(
                "an asset cannot be merged into itself".into(),
            ));
        }
        if self.canonical_asset(into)? == from || self.canonical_asset(from)? != from {
            return Err(StorageError::Invalid(format!(
                "asset {from} is already merged, or merging it into {into} would form a cycle"
            )));
        }
        let into = self.canonical_asset(into)?;
        self.conn.execute(
            "INSERT INTO asset_merges (from_asset, into_asset) VALUES (?1, ?2)",
            params![from, into],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn undo_merge(&mut self, merge_id: i64) -> Result<()> {
        let n = self.conn.execute(
            "UPDATE asset_merges SET undone_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?1 AND undone_at IS NULL",
            [merge_id],
        )?;
        if n == 0 {
            return Err(StorageError::NotFound(format!("active merge {merge_id}")));
        }
        Ok(())
    }

    pub fn active_merges(&self) -> Result<Vec<MergeRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, from_asset, into_asset, created_at FROM asset_merges WHERE undone_at IS NULL ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(MergeRow {
                id: r.get(0)?,
                from_asset: r.get(1)?,
                into_asset: r.get(2)?,
                created_at: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Follow active merges from `asset` to the asset that survives.
    pub fn canonical_asset(&self, asset: AssetId) -> Result<AssetId> {
        let merges: HashMap<AssetId, AssetId> = self
            .active_merges()?
            .into_iter()
            .map(|m| (m.from_asset, m.into_asset))
            .collect();
        let mut cur = asset;
        for _ in 0..merges.len() + 1 {
            match merges.get(&cur) {
                Some(next) => cur = *next,
                None => return Ok(cur),
            }
        }
        Ok(cur)
    }

    /// Raw instrument key → instrument key of the surviving asset, for every merged-away asset.
    pub fn key_map(&self) -> Result<HashMap<String, String>> {
        let keys: HashMap<AssetId, String> = self
            .list_assets()?
            .into_iter()
            .map(|a| (a.id, a.instrument_key))
            .collect();
        let mut out = HashMap::new();
        for m in self.active_merges()? {
            let target = self.canonical_asset(m.into_asset)?;
            if let (Some(k), Some(t)) = (keys.get(&m.from_asset), keys.get(&target)) {
                out.insert(k.clone(), t.clone());
            }
        }
        Ok(out)
    }

    // ---- refresh log ----

    pub fn log_refresh(
        &mut self,
        started_at: &str,
        finished_at: &str,
        report_json: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO refresh_log (started_at, finished_at, report_json) VALUES (?1, ?2, ?3)",
            params![started_at, finished_at, report_json],
        )?;
        Ok(())
    }

    /// The most recent refresh reports, newest first: (finished_at, report_json).
    pub fn refresh_reports(&self, limit: usize) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT finished_at, report_json FROM refresh_log ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn last_refresh_report(&self) -> Result<Option<(String, String)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT finished_at, report_json FROM refresh_log ORDER BY id DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }
}
