//! The import pipeline: resolve, fingerprint and deduplicate, check, review, commit.
//!
//! ```text
//! ParsedBatch --resolve_batch--> ResolvedBatch --prepare--> Prepared
//!     Prepared + Decisions --report--> ReviewReport
//!     Prepared + Decisions --commit--> rows in the store
//! ```
//!
//! Nothing is written until [`commit`]. Records that need a user decision
//! (possible duplicates from another file) are committed only when the
//! [`Decisions`] mark them as new.

use std::collections::{BTreeMap, HashMap, HashSet};

use mizan_import::{Assertion, Diagnostic, ImportRecord, ItemError, Kind, ParsedBatch};
use mizan_storage::{
    AccountId, AssertionResult, BatchId, NewActivity, NewAssertion, NewBatch, StorageError, Store,
    StoredActivity,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::consistency::{check_record, ConsistencyWarning};
use crate::fingerprint::{fingerprints, is_external};
use crate::ordering::{order, OrderInput, OrderingDiagnostic};
use crate::replay::{check_assertion, replay_with, AssertionCheck};
use crate::resolve::{resolve_records, ResolvedRecord, Resolver};
use crate::review::ReviewItem;

/// A batch whose hints have been resolved as far as possible.
#[derive(Clone, Debug)]
pub struct ResolvedBatch {
    pub records: Vec<ResolvedRecord>,
    pub assertions: Vec<(Assertion, Option<AccountId>)>,
    pub diagnostics: Vec<Diagnostic>,
    pub validation_errors: Vec<ItemError>,
    pub unresolved: Vec<ReviewItem>,
}

/// Resolve records' and assertions' hints. Only the resolver touches the store here.
pub fn resolve_batch(
    parsed: ParsedBatch,
    resolver: &mut dyn Resolver,
) -> Result<ResolvedBatch, StorageError> {
    let (records, mut unresolved) = resolve_records(&parsed.records, resolver)?;
    let mut assertions = Vec::new();
    for a in parsed.assertions {
        let id = resolver.account(&a.item.account)?;
        let already = unresolved
            .iter()
            .any(|i| matches!(i, ReviewItem::UnresolvedAccount { hint } if hint.hint == a.item.account.hint));
        if id.is_none() && !already {
            unresolved.push(ReviewItem::UnresolvedAccount {
                hint: a.item.account.clone(),
            });
        }
        assertions.push((a.item, id));
    }
    Ok(ResolvedBatch {
        records,
        assertions,
        diagnostics: parsed.diagnostics,
        validation_errors: parsed.errors,
        unresolved,
    })
}

/// How a record relates to what is already stored.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Outcome {
    New,
    /// Already stored (or repeated within this batch, then `existing_activity_id` is `None`).
    Duplicate {
        existing_activity_id: Option<i64>,
    },
    /// Matches a stored record from a different file and has no external id: the user decides.
    NeedsConfirmation {
        existing_activity_id: i64,
        existing_file_sha256: String,
        existing_locator: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// A second, genuine record: commit it.
    New,
    /// The same record as the stored one: skip it.
    Duplicate,
}

/// User decisions for records that need confirmation, keyed by record index in the batch.
pub type Decisions = HashMap<usize, Decision>;

#[derive(Clone, Debug)]
pub struct PreparedRecord {
    pub resolved: ResolvedRecord,
    pub fingerprint: String,
    pub outcome: Outcome,
    /// A `position_in` without cost basis.
    pub basis_unknown: bool,
    pub warnings: Vec<ConsistencyWarning>,
}

impl PreparedRecord {
    fn record(&self) -> &ImportRecord {
        &self.resolved.record
    }

    fn index(&self) -> usize {
        self.resolved.index
    }

    /// Whether this record would be committed under `decisions`.
    pub fn will_commit(&self, decisions: &Decisions) -> bool {
        match self.outcome {
            Outcome::New => true,
            Outcome::Duplicate { .. } => false,
            Outcome::NeedsConfirmation { .. } => {
                decisions.get(&self.index()) == Some(&Decision::New)
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct Prepared {
    pub records: Vec<PreparedRecord>,
    pub assertions: Vec<(Assertion, Option<AccountId>)>,
    pub diagnostics: Vec<Diagnostic>,
    pub validation_errors: Vec<ItemError>,
    pub unresolved: Vec<ReviewItem>,
}

/// Fingerprint, deduplicate against the store, and run the consistency checks.
pub fn prepare(store: &Store, batch: ResolvedBatch) -> Result<Prepared, StorageError> {
    let fps = fingerprints(batch.records.iter().map(|r| &r.record));
    let mut seen_in_batch = HashSet::new();
    let mut records = Vec::with_capacity(batch.records.len());

    for (resolved, fp) in batch.records.into_iter().zip(fps) {
        let r = &resolved.record;
        let outcome = if !seen_in_batch.insert(fp.clone()) {
            Outcome::Duplicate {
                existing_activity_id: None,
            }
        } else {
            match store.activities_by_fingerprint(&fp)?.into_iter().next() {
                None => Outcome::New,
                Some(existing)
                    if is_external(&fp)
                        || existing.record.source.file_sha256 == r.source.file_sha256 =>
                {
                    Outcome::Duplicate {
                        existing_activity_id: Some(existing.id),
                    }
                }
                Some(existing) => Outcome::NeedsConfirmation {
                    existing_activity_id: existing.id,
                    existing_file_sha256: existing.record.source.file_sha256.as_str().to_owned(),
                    existing_locator: existing.record.source.locator.clone(),
                },
            }
        };
        let basis_unknown = r.kind == Kind::PositionIn && r.cost_basis.is_none();
        let warnings = check_record(resolved.index, r);
        records.push(PreparedRecord {
            resolved,
            fingerprint: fp,
            outcome,
            basis_unknown,
            warnings,
        });
    }

    Ok(Prepared {
        records,
        assertions: batch.assertions,
        diagnostics: batch.diagnostics,
        validation_errors: batch.validation_errors,
        unresolved: batch.unresolved,
    })
}

/// One record as shown in the review.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RecordSummary {
    pub index: usize,
    pub locator: String,
    pub kind: String,
    pub date: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instrument: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net_cash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
}

impl RecordSummary {
    fn of(p: &PreparedRecord) -> Self {
        let r = p.record();
        RecordSummary {
            index: p.index(),
            locator: r.source.locator.clone(),
            kind: r.kind.as_str().to_owned(),
            date: r.date.to_string(),
            instrument: p.resolved.instrument_key.clone(),
            quantity: r.quantity.map(|q| q.to_string()),
            net_cash: r
                .net_cash
                .as_ref()
                .map(|m| format!("{} {}", m.amount, m.currency)),
            memo: r.memo.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Confirmation {
    pub record: RecordSummary,
    pub existing_activity_id: i64,
    pub existing_file_sha256: String,
    pub existing_locator: String,
    /// `None` while the user has not decided.
    pub decision: Option<Decision>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ReviewSummary {
    pub new: usize,
    pub duplicates: usize,
    pub pending_confirmations: usize,
    pub consistency_warnings: usize,
    pub ordering_diagnostics: usize,
    pub adapter_diagnostics: usize,
    pub validation_errors: usize,
    pub unresolved: usize,
    pub assertions_failed: usize,
}

/// Everything the user sees before committing.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ReviewReport {
    pub summary: ReviewSummary,
    pub new_records: Vec<RecordSummary>,
    pub duplicates: Vec<RecordSummary>,
    pub confirmations: Vec<Confirmation>,
    pub consistency_warnings: Vec<ConsistencyWarning>,
    pub ordering_diagnostics: Vec<OrderingDiagnostic>,
    pub adapter_diagnostics: Vec<Diagnostic>,
    /// Records and assertions rejected by validation, with their locators.
    pub validation_errors: Vec<String>,
    pub unresolved: Vec<ReviewItem>,
    pub assertion_results: Vec<AssertionCheck>,
    pub flagged_adjustments: Vec<RecordSummary>,
    pub basis_unknown: Vec<RecordSummary>,
}

fn order_input(
    account: AccountId,
    r: &ImportRecord,
    source_order: i64,
    key: Option<String>,
) -> OrderInput {
    OrderInput {
        account: account.to_string(),
        date: r.date,
        file: r.source.file_sha256.as_str().to_owned(),
        source_order,
        kind: r.kind,
        instrument_key: key,
        locator: r.source.locator.clone(),
    }
}

fn stored_key(a: &StoredActivity) -> Option<String> {
    a.record
        .instrument
        .as_ref()
        .and_then(crate::resolve::instrument_key)
}

/// Records that would be committed, in deterministic processing order.
fn committed_in_order<'a>(p: &'a Prepared, decisions: &Decisions) -> Vec<&'a PreparedRecord> {
    let pending: Vec<&PreparedRecord> = p
        .records
        .iter()
        .filter(|r| r.will_commit(decisions))
        .collect();
    let inputs: Vec<OrderInput> = pending
        .iter()
        .map(|r| {
            order_input(
                r.resolved.account_id.unwrap_or(-1),
                r.record(),
                r.index() as i64,
                r.resolved.instrument_key.clone(),
            )
        })
        .collect();
    order(&inputs).0.into_iter().map(|i| pending[i]).collect()
}

/// Ordering diagnostics for pending records together with stored ones on the same accounts and dates.
fn ordering_diagnostics(
    store: &Store,
    pending: &[&PreparedRecord],
) -> Result<Vec<OrderingDiagnostic>, StorageError> {
    let mut inputs = Vec::new();
    let mut ranges: BTreeMap<AccountId, (String, String)> = BTreeMap::new();
    for r in pending {
        let Some(acct) = r.resolved.account_id else {
            continue;
        };
        let d = r.record().date.to_string();
        let e = ranges.entry(acct).or_insert_with(|| (d.clone(), d.clone()));
        if d < e.0 {
            e.0 = d.clone();
        }
        if d > e.1 {
            e.1 = d;
        }
        inputs.push(order_input(
            acct,
            r.record(),
            r.index() as i64,
            r.resolved.instrument_key.clone(),
        ));
    }
    for (acct, (from, to)) in ranges {
        for s in store.activities_between(acct, &from, &to)? {
            inputs.push(order_input(acct, &s.record, s.source_order, stored_key(&s)));
        }
    }
    Ok(order(&inputs).1)
}

fn check_assertions(
    store: &Store,
    p: &Prepared,
    committed: &[&PreparedRecord],
) -> Result<Vec<AssertionCheck>, StorageError> {
    let keys = store.key_map()?;
    let mut out = Vec::new();
    for (a, acct) in &p.assertions {
        let Some(acct) = *acct else {
            let mut c = check_assertion(a, &Default::default());
            c.passed = false;
            c.note =
                Some("account is not resolved; map it before this balance can be checked".into());
            out.push(c);
            continue;
        };
        let stored = store.activities_until(acct, &a.as_of.to_string())?;
        let pending: Vec<&PreparedRecord> = committed
            .iter()
            .copied()
            .filter(|r| r.resolved.account_id == Some(acct) && r.record().date <= a.as_of)
            .collect();
        let mut inputs: Vec<OrderInput> = stored
            .iter()
            .map(|s| order_input(acct, &s.record, s.source_order, stored_key(s)))
            .collect();
        inputs.extend(pending.iter().map(|r| {
            order_input(
                acct,
                r.record(),
                r.index() as i64,
                r.resolved.instrument_key.clone(),
            )
        }));
        let all: Vec<&ImportRecord> = stored
            .iter()
            .map(|s| &s.record)
            .chain(pending.iter().map(|r| r.record()))
            .collect();
        let (ord, _) = order(&inputs);
        let balances = replay_with(keys.clone(), ord.into_iter().map(|i| all[i]));
        out.push(check_assertion(a, &balances));
    }
    Ok(out)
}

/// Build the review report for the given decisions. Reads the store; writes nothing.
pub fn report(
    store: &Store,
    p: &Prepared,
    decisions: &Decisions,
) -> Result<ReviewReport, StorageError> {
    let committed = committed_in_order(p, decisions);
    let mut duplicates = Vec::new();
    let mut confirmations = Vec::new();
    for r in &p.records {
        match &r.outcome {
            Outcome::New => {}
            Outcome::Duplicate { .. } => duplicates.push(RecordSummary::of(r)),
            Outcome::NeedsConfirmation {
                existing_activity_id,
                existing_file_sha256,
                existing_locator,
            } => {
                let decision = decisions.get(&r.index()).copied();
                if decision == Some(Decision::Duplicate) {
                    duplicates.push(RecordSummary::of(r));
                }
                confirmations.push(Confirmation {
                    record: RecordSummary::of(r),
                    existing_activity_id: *existing_activity_id,
                    existing_file_sha256: existing_file_sha256.clone(),
                    existing_locator: existing_locator.clone(),
                    decision,
                });
            }
        }
    }
    let mut new_records: Vec<RecordSummary> =
        committed.iter().map(|r| RecordSummary::of(r)).collect();
    new_records.sort_by_key(|s| s.index);

    let consistency_warnings: Vec<ConsistencyWarning> = p
        .records
        .iter()
        .flat_map(|r| r.warnings.iter().cloned())
        .collect();
    let ordering_diagnostics = ordering_diagnostics(store, &committed)?;
    let assertion_results = check_assertions(store, p, &committed)?;
    let pick = |f: &dyn Fn(&PreparedRecord) -> bool| -> Vec<RecordSummary> {
        let mut v: Vec<RecordSummary> = committed
            .iter()
            .filter(|r| f(r))
            .map(|r| RecordSummary::of(r))
            .collect();
        v.sort_by_key(|s| s.index);
        v
    };
    let flagged_adjustments = pick(&|r| r.record().kind == Kind::Adjustment);
    let basis_unknown = pick(&|r| r.basis_unknown);

    let summary = ReviewSummary {
        new: new_records.len(),
        duplicates: duplicates.len(),
        pending_confirmations: confirmations
            .iter()
            .filter(|c| c.decision.is_none())
            .count(),
        consistency_warnings: consistency_warnings.len(),
        ordering_diagnostics: ordering_diagnostics.len(),
        adapter_diagnostics: p.diagnostics.len(),
        validation_errors: p.validation_errors.len(),
        unresolved: p.unresolved.len(),
        assertions_failed: assertion_results.iter().filter(|a| !a.passed).count(),
    };

    Ok(ReviewReport {
        summary,
        new_records,
        duplicates,
        confirmations,
        consistency_warnings,
        ordering_diagnostics,
        adapter_diagnostics: p.diagnostics.clone(),
        validation_errors: p.validation_errors.iter().map(|e| e.to_string()).collect(),
        unresolved: p.unresolved.clone(),
        assertion_results,
        flagged_adjustments,
        basis_unknown,
    })
}

#[derive(Debug, Error)]
pub enum CommitError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("cannot commit: hints are unresolved for {0}")]
    Unresolved(String),
    #[error("nothing to commit")]
    Empty,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitResult {
    pub batch_id: BatchId,
    pub committed: usize,
    /// Records needing confirmation that were left out because no decision was given.
    pub skipped_undecided: usize,
}

/// Persist the records that will commit under `decisions`, with provenance, plus
/// the batch's assertions and their results. All-or-nothing.
pub fn commit(
    store: &mut Store,
    p: &Prepared,
    decisions: &Decisions,
) -> Result<CommitResult, CommitError> {
    let committed = committed_in_order(p, decisions);
    let unresolved: Vec<String> = committed
        .iter()
        .filter(|r| !r.resolved.is_resolved())
        .map(|r| r.record().source.locator.clone())
        .chain(
            p.assertions
                .iter()
                .filter(|(_, acct)| acct.is_none())
                .map(|(a, _)| a.source.locator.clone()),
        )
        .collect();
    if !unresolved.is_empty() {
        return Err(CommitError::Unresolved(unresolved.join(", ")));
    }
    let first_source = committed
        .first()
        .map(|r| &r.record().source)
        .or_else(|| p.assertions.first().map(|(a, _)| &a.source))
        .ok_or(CommitError::Empty)?
        .clone();

    let results = check_assertions(store, p, &committed)?;
    let batch = NewBatch {
        adapter: first_source.adapter,
        adapter_version: first_source.adapter_version,
        file_sha256: first_source.file_sha256.as_str().to_owned(),
        activities: committed
            .iter()
            .map(|r| NewActivity {
                account_id: r.resolved.account_id.expect("checked above"),
                asset_id: r.resolved.asset_id,
                fingerprint: r.fingerprint.clone(),
                source_order: r.index() as i64,
                basis_unknown: r.basis_unknown,
                record: r.record().clone(),
            })
            .collect(),
        assertions: p
            .assertions
            .iter()
            .zip(results)
            .map(|((a, acct), res)| NewAssertion {
                account_id: acct.expect("checked above"),
                assertion: a.clone(),
                result: Some(AssertionResult {
                    passed: res.passed,
                    detail: serde_json::to_value(&res.lines).expect("serializable"),
                }),
            })
            .collect(),
    };
    let batch_id = store.commit_batch(&batch)?;
    let skipped_undecided = p
        .records
        .iter()
        .filter(|r| {
            matches!(r.outcome, Outcome::NeedsConfirmation { .. })
                && !decisions.contains_key(&r.index())
        })
        .count();
    Ok(CommitResult {
        batch_id,
        committed: batch.activities.len(),
        skipped_undecided,
    })
}
