//! Activity browsing and manual entries: filtered, paged activity queries,
//! totals per currency, and the batches that hold manual entries.

use std::collections::{BTreeMap, HashSet};
use std::str::FromStr;

use rusqlite::types::Value as SqlValue;
use rust_decimal::Decimal;

use crate::{
    AccountId, AssetId, BatchId, BatchRow, NewBatch, Result, StorageError, Store, StoredActivity,
};

/// The adapter id of activities entered by hand.
pub const MANUAL_ADAPTER: &str = "manual";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceFilter {
    Imported,
    Manual,
}

/// Which activities to list. Empty or `None` fields do not filter.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ActivityFilter {
    pub accounts: Vec<AccountId>,
    /// Kind names as stored, for example `buy`.
    pub kinds: Vec<String>,
    /// `YYYY-MM-DD`, inclusive.
    pub from: Option<String>,
    pub to: Option<String>,
    pub source: Option<SourceFilter>,
    /// Matched against the symbol, name and ISIN of the activity's asset or
    /// the asset it was merged into, ignoring case.
    pub text: Option<String>,
}

/// One page of activities, newest first, and how many match in all.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredPage {
    pub rows: Vec<StoredActivity>,
    pub total: i64,
}

/// Cash moved by the matching activities in one currency.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CashTotals {
    pub cash_in: Decimal,
    pub cash_out: Decimal,
}

impl CashTotals {
    pub fn net(&self) -> Decimal {
        self.cash_in - self.cash_out
    }
}

/// Direction of a kind's net cash: +1 into the account, -1 out of it, 0 when
/// the record carries a signed amount or moves no cash of its own.
pub fn cash_direction(kind: &str) -> i8 {
    match kind {
        "deposit" | "sell" | "short_sell" | "dividend" | "interest" | "distribution" | "reward"
        | "return_of_capital" => 1,
        "withdrawal" | "buy" | "buy_to_cover" | "fee" | "tax" => -1,
        _ => 0,
    }
}

impl Store {
    /// Asset ids whose own symbol, name or ISIN contains `text`, or whose
    /// canonical asset's does.
    fn assets_matching(&self, text: &str) -> Result<Vec<AssetId>> {
        let needle = text.trim().to_lowercase();
        let assets = self.list_assets()?;
        let hit: HashSet<AssetId> = assets
            .iter()
            .filter(|a| {
                [&a.hint.symbol, &a.hint.name, &a.hint.isin, &a.name]
                    .iter()
                    .any(|f| {
                        f.as_deref()
                            .is_some_and(|s| s.to_lowercase().contains(&needle))
                    })
            })
            .map(|a| a.id)
            .collect();
        let mut out = Vec::new();
        for a in &assets {
            if hit.contains(&a.id) || hit.contains(&self.canonical_asset(a.id)?) {
                out.push(a.id);
            }
        }
        Ok(out)
    }

    /// `WHERE` clause and parameters for a filter.
    fn filter_sql(&self, f: &ActivityFilter) -> Result<(String, Vec<SqlValue>)> {
        let mut clauses = vec!["1 = 1".to_string()];
        let mut params: Vec<SqlValue> = Vec::new();
        let list = |col: &str,
                    values: Vec<SqlValue>,
                    clauses: &mut Vec<String>,
                    params: &mut Vec<SqlValue>| {
            let marks = vec!["?"; values.len()].join(", ");
            clauses.push(format!("{col} IN ({marks})"));
            params.extend(values);
        };
        if !f.accounts.is_empty() {
            list(
                "account_id",
                f.accounts.iter().map(|a| SqlValue::Integer(*a)).collect(),
                &mut clauses,
                &mut params,
            );
        }
        if !f.kinds.is_empty() {
            list(
                "kind",
                f.kinds.iter().map(|k| SqlValue::Text(k.clone())).collect(),
                &mut clauses,
                &mut params,
            );
        }
        if let Some(text) = f.text.as_deref().filter(|t| !t.trim().is_empty()) {
            let ids = self.assets_matching(text)?;
            if ids.is_empty() {
                clauses.push("0 = 1".into());
            } else {
                list(
                    "asset_id",
                    ids.into_iter().map(SqlValue::Integer).collect(),
                    &mut clauses,
                    &mut params,
                );
            }
        }
        if let Some(from) = &f.from {
            clauses.push("date >= ?".into());
            params.push(SqlValue::Text(from.clone()));
        }
        if let Some(to) = &f.to {
            clauses.push("date <= ?".into());
            params.push(SqlValue::Text(to.clone()));
        }
        match f.source {
            Some(SourceFilter::Manual) => clauses.push(format!("adapter = '{MANUAL_ADAPTER}'")),
            Some(SourceFilter::Imported) => clauses.push(format!("adapter <> '{MANUAL_ADAPTER}'")),
            None => {}
        }
        Ok((clauses.join(" AND "), params))
    }

    /// Matching activities, newest first, `limit` rows from `offset`.
    pub fn query_activities(
        &self,
        f: &ActivityFilter,
        offset: i64,
        limit: i64,
    ) -> Result<StoredPage> {
        let (filter, params) = self.filter_sql(f)?;
        let total: i64 = self.conn.query_row(
            &format!("SELECT COUNT(*) FROM activities WHERE {filter}"),
            rusqlite::params_from_iter(params.iter()),
            |r| r.get(0),
        )?;
        let mut p = params;
        p.push(SqlValue::Integer(limit));
        p.push(SqlValue::Integer(offset));
        let rows = self.collect(
            &format!(
                "SELECT {} FROM activities WHERE {filter}
                 ORDER BY date DESC, id DESC LIMIT ? OFFSET ?",
                Self::ACTIVITY_COLS
            ),
            rusqlite::params_from_iter(p.iter()),
        )?;
        Ok(StoredPage { rows, total })
    }

    /// Count and cash per currency over the matching activities.
    pub fn activity_totals(
        &self,
        f: &ActivityFilter,
    ) -> Result<(i64, BTreeMap<String, CashTotals>)> {
        let (filter, params) = self.filter_sql(f)?;
        let mut stmt = self.conn.prepare(&format!(
            "SELECT kind, json_extract(record_json, '$.net_cash.amount'), json_extract(record_json, '$.net_cash.currency')
             FROM activities WHERE {filter}"
        ))?;
        let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?;
        let mut count = 0;
        let mut out: BTreeMap<String, CashTotals> = BTreeMap::new();
        for row in rows {
            let (kind, amount, ccy) = row?;
            count += 1;
            let (Some(amount), Some(ccy)) = (amount, ccy) else {
                continue;
            };
            let Ok(v) = Decimal::from_str(&amount) else {
                continue;
            };
            let signed = match cash_direction(&kind) {
                1 => v,
                -1 => -v,
                // internal transfers and adjustments carry their own sign;
                // others (fx conversion, splits, position moves) are left out.
                _ if kind == "internal_transfer" || kind == "adjustment" => v,
                _ => continue,
            };
            let t = out.entry(ccy).or_default();
            if signed >= Decimal::ZERO {
                t.cash_in += signed;
            } else {
                t.cash_out -= signed;
            }
        }
        Ok((count, out))
    }

    /// One activity by id.
    pub fn activity(&self, id: i64) -> Result<Option<StoredActivity>> {
        Ok(self
            .collect(
                &format!(
                    "SELECT {} FROM activities WHERE id = ?1",
                    Self::ACTIVITY_COLS
                ),
                [id],
            )?
            .pop())
    }

    /// One batch, imports and manual entries alike.
    pub fn batch(&self, id: BatchId) -> Result<Option<BatchRow>> {
        Ok(self.batches_where("b.id = ?1", [id])?.pop())
    }

    /// Replace a manual entry's batch with `new` in one transaction. Refuses
    /// imported batches; on any error the old entry stays.
    pub fn replace_manual_batch(&mut self, old: BatchId, new: &NewBatch) -> Result<BatchId> {
        let tx = self.conn.transaction()?;
        let adapter: Option<String> = tx
            .query_row(
                "SELECT adapter FROM import_batches WHERE id = ?1",
                [old],
                |r| r.get(0),
            )
            .ok();
        match adapter.as_deref() {
            None => return Err(StorageError::NotFound(format!("entry {old}"))),
            Some(MANUAL_ADAPTER) => {}
            Some(_) => {
                return Err(StorageError::Invalid(
                    "imported activities are corrected by undoing or re-importing their file"
                        .into(),
                ))
            }
        }
        if new.adapter != MANUAL_ADAPTER {
            return Err(StorageError::Invalid(
                "a replacement must be a manual entry".into(),
            ));
        }
        tx.execute(
            "DELETE FROM activity_amendments WHERE activity_id IN (SELECT id FROM activities WHERE batch_id = ?1)",
            [old],
        )?;
        tx.execute("DELETE FROM activities WHERE batch_id = ?1", [old])?;
        tx.execute("DELETE FROM import_batches WHERE id = ?1", [old])?;
        let id = Self::insert_batch(&tx, new)?;
        tx.commit()?;
        Ok(id)
    }

    /// Delete a manual entry's batch. Refuses imported batches.
    pub fn delete_manual_batch(&mut self, id: BatchId) -> Result<()> {
        match self.batch(id)? {
            None => Err(StorageError::NotFound(format!("entry {id}"))),
            Some(b) if b.adapter == MANUAL_ADAPTER => self.undo_batch(id),
            Some(_) => Err(StorageError::Invalid(
                "imported activities are corrected by undoing or re-importing their file".into(),
            )),
        }
    }

    /// Amend an imported activity in place. The record as first imported is
    /// kept the first time; the fingerprint and provenance columns are not
    /// touched, so re-importing the statement still finds it.
    pub fn amend_activity(&mut self, a: &AmendedActivity, amended_at: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        let (adapter, original): (String, String) = tx
            .query_row(
                "SELECT adapter, record_json FROM activities WHERE id = ?1",
                [a.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| StorageError::NotFound(format!("activity {}", a.id)))?;
        if adapter == MANUAL_ADAPTER {
            return Err(StorageError::Invalid(
                "manual entries are edited by replacing them".into(),
            ));
        }
        tx.execute(
            "INSERT INTO activity_amendments (activity_id, original_json, amended_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(activity_id) DO UPDATE SET amended_at = excluded.amended_at",
            rusqlite::params![a.id, original, amended_at],
        )?;
        tx.execute(
            "UPDATE activities SET account_id = ?2, asset_id = ?3, kind = ?4, date = ?5, basis_unknown = ?6, record_json = ?7
             WHERE id = ?1",
            rusqlite::params![
                a.id,
                a.account_id,
                a.asset_id,
                a.record.kind.as_str(),
                a.record.date.to_string(),
                a.basis_unknown,
                serde_json::to_string(&a.record)?,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Restore an amended activity's imported record.
    pub fn revert_activity(
        &mut self,
        id: i64,
        account_id: AccountId,
        asset_id: Option<AssetId>,
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        let original: String = tx
            .query_row(
                "SELECT original_json FROM activity_amendments WHERE activity_id = ?1",
                [id],
                |r| r.get(0),
            )
            .map_err(|_| StorageError::NotFound(format!("amendment of activity {id}")))?;
        let record: mizan_import::ImportRecord = serde_json::from_str(&original)?;
        let basis_unknown =
            record.kind == mizan_import::Kind::PositionIn && record.cost_basis.is_none();
        tx.execute(
            "UPDATE activities SET account_id = ?2, asset_id = ?3, kind = ?4, date = ?5, basis_unknown = ?6, record_json = ?7
             WHERE id = ?1",
            rusqlite::params![id, account_id, asset_id, record.kind.as_str(), record.date.to_string(), basis_unknown, original],
        )?;
        tx.execute(
            "DELETE FROM activity_amendments WHERE activity_id = ?1",
            [id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The record as first imported and when it was amended, for an amended activity.
    pub fn amendment(&self, id: i64) -> Result<Option<(mizan_import::ImportRecord, String)>> {
        let row: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT original_json, amended_at FROM activity_amendments WHERE activity_id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();
        match row {
            Some((json, at)) => Ok(Some((serde_json::from_str(&json)?, at))),
            None => Ok(None),
        }
    }

    /// When each amended activity was last amended, by activity id.
    pub fn amendment_times(&self) -> Result<std::collections::HashMap<i64, String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT activity_id, amended_at FROM activity_amendments")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Ids of every amended activity.
    pub fn amended_ids(&self) -> Result<HashSet<i64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT activity_id FROM activity_amendments")?;
        let ids = stmt.query_map([], |r| r.get(0))?;
        Ok(ids.collect::<rusqlite::Result<HashSet<i64>>>()?)
    }
}

/// The new state of an amended imported activity.
#[derive(Clone, Debug)]
pub struct AmendedActivity {
    pub id: i64,
    pub account_id: AccountId,
    pub asset_id: Option<AssetId>,
    pub basis_unknown: bool,
    pub record: mizan_import::ImportRecord,
}
