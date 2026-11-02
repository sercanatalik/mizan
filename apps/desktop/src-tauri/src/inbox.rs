//! The Inbox: every condition that needs the user before the numbers can be
//! trusted, computed from stored data and the open import review. Items are
//! never stored; only snoozes are (see `mizan_storage::inbox`).

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use mizan_core::portfolio::{AttentionItem, FailedCheckLine};
use mizan_core::ReviewItem;
use mizan_import::Date;
use serde::Serialize;
use tauri::AppHandle;

use crate::commands::{blocking, holdings, overview, store_err};
use crate::session::AppError;
use crate::state::AppState;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum InboxKind {
    Import,
    Check,
    Price,
    Cost,
}

/// Where the item's primary action takes the user.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InboxAction {
    /// The open import review.
    ImportReview,
    /// The account page, at its balance checks.
    AccountChecks {
        #[cfg_attr(test, ts(type = "number"))]
        account_id: i64,
    },
    /// The activity form for a `position_in` of this instrument into this account.
    AddPositionIn {
        #[cfg_attr(test, ts(type = "number"))]
        account_id: i64,
        #[cfg_attr(test, ts(type = "number | null"))]
        asset_id: Option<i64>,
    },
    /// The asset's price source and manual price controls.
    AssetPrice {
        #[cfg_attr(test, ts(type = "number | null"))]
        asset_id: Option<i64>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct InboxFact {
    pub label: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct InboxItem {
    /// Stable per condition, e.g. `check:3` or `price:12`.
    pub key: String,
    pub kind: InboxKind,
    pub title: String,
    pub subtitle: String,
    /// What is wrong, in a sentence or two.
    pub body: String,
    pub facts: Vec<InboxFact>,
    /// Failing lines of a balance check: statement, computed and difference.
    pub check_lines: Vec<FailedCheckLine>,
    /// When the condition arose (a date), if known; newest first.
    pub since: Option<String>,
    pub action: InboxAction,
    /// The figures behind the item; a snooze ends when they change.
    pub fingerprint: String,
    /// End of the snooze, for snoozed items.
    pub snoozed_until: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct InboxView {
    /// Items to act on, newest condition first.
    pub items: Vec<InboxItem>,
    /// Snoozed items, soonest to return first.
    pub snoozed: Vec<InboxItem>,
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Every current condition, unsorted and without snoozes applied.
pub fn conditions(state: &AppState, today: Date) -> Result<Vec<InboxItem>, AppError> {
    let mut items = Vec::new();

    // The open import review.
    if let Some(session) = state.session.lock().map_err(store_err)?.as_ref() {
        let view = session.view();
        let pending = session.pending_decisions();
        if pending > 0 {
            items.push(InboxItem {
                key: "import:decisions".into(),
                kind: InboxKind::Import,
                title: format!("{} awaiting a decision", plural(pending, "record", "records")),
                subtitle: view.file_name.clone(),
                body: "These records may duplicate records already stored. Keep them, or treat them as the same records, before committing the import.".into(),
                facts: vec![
                    InboxFact { label: "Statement".into(), value: view.file_name.clone() },
                    InboxFact { label: "Awaiting a decision".into(), value: pending.to_string() },
                    InboxFact { label: "New records".into(), value: view.report.new_records.len().to_string() },
                ],
                check_lines: vec![],
                since: Some(today.to_string()),
                action: InboxAction::ImportReview,
                fingerprint: format!("{}|{pending}", view.file_name),
                snoozed_until: None,
            });
        }
        let unresolved = &view.report.unresolved;
        if !unresolved.is_empty() {
            let names: Vec<String> = unresolved
                .iter()
                .map(|u| match u {
                    ReviewItem::UnresolvedAccount { hint } => {
                        format!(
                            "account {}",
                            hint.name_hint.clone().unwrap_or_else(|| hint.hint.clone())
                        )
                    }
                    ReviewItem::UnresolvedInstrument { hint, key } => format!(
                        "asset {}",
                        hint.symbol
                            .clone()
                            .or_else(|| hint.name.clone())
                            .or_else(|| hint.isin.clone())
                            .or_else(|| key.clone())
                            .unwrap_or_else(|| "without a name".into())
                    ),
                })
                .collect();
            items.push(InboxItem {
                key: "import:hints".into(),
                kind: InboxKind::Import,
                title: format!("{} not linked", plural(unresolved.len(), "account or asset", "accounts or assets")),
                subtitle: view.file_name.clone(),
                body: "The statement names accounts or assets Mizan does not know yet. Link each to an existing one or create it.".into(),
                facts: names
                    .iter()
                    .map(|n| InboxFact { label: "Not linked".into(), value: n.clone() })
                    .collect(),
                check_lines: vec![],
                since: Some(today.to_string()),
                action: InboxAction::ImportReview,
                fingerprint: format!("{}|{}", view.file_name, names.join(",")),
                snoozed_until: None,
            });
        }
    }

    // Failed balance checks and unknown cost basis, as Overview computes them.
    for a in overview(state, today)?.attention {
        match a {
            AttentionItem::FailedChecks {
                account_id,
                account_name,
                as_of,
                failed,
            } => {
                let names: Vec<&str> = failed.iter().map(|f| f.display.as_str()).collect();
                items.push(InboxItem {
                    key: format!("check:{account_id}"),
                    kind: InboxKind::Check,
                    title: format!("Balance check failed · {account_name}"),
                    subtitle: format!("{} on {as_of}", names.join(", ")),
                    body: "The balance printed on the statement differs from the balance Mizan computes from stored activity. A transaction is probably missing or recorded twice.".into(),
                    facts: vec![
                        InboxFact { label: "Account".into(), value: account_name.clone() },
                        InboxFact { label: "Statement date".into(), value: as_of.clone() },
                    ],
                    fingerprint: format!(
                        "{as_of}|{}",
                        failed
                            .iter()
                            .map(|f| format!("{}:{}:{}", f.label, f.expected, f.computed))
                            .collect::<Vec<_>>()
                            .join(",")
                    ),
                    check_lines: failed,
                    since: Some(as_of),
                    action: InboxAction::AccountChecks { account_id },
                    snoozed_until: None,
                });
            }
            AttentionItem::UnknownBasis {
                account_id,
                account_name,
                instrument_key,
                display,
                units,
            } => {
                let asset_id = state
                    .store
                    .lock()
                    .map_err(store_err)?
                    .list_assets()?
                    .into_iter()
                    .find(|x| x.instrument_key == instrument_key)
                    .map(|x| x.id);
                items.push(InboxItem {
                    key: format!("cost:{account_id}:{instrument_key}"),
                    kind: InboxKind::Cost,
                    title: format!("Unknown cost basis · {units} {display} in {account_name}"),
                    subtitle: format!("{display} · {account_name}"),
                    body: "These units arrived without a cost. Unrealized gain and tax reports leave them out until a cost is recorded.".into(),
                    facts: vec![
                        InboxFact { label: "Units without cost".into(), value: units.clone() },
                        InboxFact { label: "Account".into(), value: account_name.clone() },
                    ],
                    check_lines: vec![],
                    since: None,
                    action: InboxAction::AddPositionIn { account_id, asset_id },
                    fingerprint: units,
                    snoozed_until: None,
                });
            }
            AttentionItem::PendingDecisions { .. } => {}
        }
    }

    // Held assets whose price is stale or missing, one item per asset.
    let mut prices: BTreeMap<String, (InboxItem, Vec<String>)> = BTreeMap::new();
    for acct in holdings(state, today)?.accounts {
        for p in acct.positions {
            let name = p
                .symbol
                .clone()
                .or_else(|| p.name.clone())
                .unwrap_or_else(|| p.instrument_key.clone());
            let item = match &p.price {
                Some(pr) if pr.stale => {
                    let days = Date::parse(&pr.date)
                        .ok()
                        .map(|d| (today.0 - d.0).num_days())
                        .unwrap_or(0);
                    InboxItem {
                        key: format!("price:{}", p.instrument_key),
                        kind: InboxKind::Price,
                        title: format!("No price for {days} days · {name}"),
                        subtitle: format!("{name} · {}", pr.source),
                        body: format!("The latest price is from {}. Values use it meanwhile and are marked stale.", pr.date),
                        facts: vec![
                            InboxFact { label: "Last price".into(), value: format!("{} {} on {}", pr.value, pr.currency, pr.date) },
                            InboxFact { label: "Source".into(), value: pr.source.clone() },
                        ],
                        check_lines: vec![],
                        since: Some(pr.date.clone()),
                        action: InboxAction::AssetPrice { asset_id: p.asset_id },
                        fingerprint: format!("stale|{}", pr.date),
                        snoozed_until: None,
                    }
                }
                None => InboxItem {
                    key: format!("price:{}", p.instrument_key),
                    kind: InboxKind::Price,
                    title: format!("No price · {name}"),
                    subtitle: name.clone(),
                    body: "This asset has no stored price, so it is left out of values. Choose a price source or add a manual price.".into(),
                    facts: vec![],
                    check_lines: vec![],
                    since: None,
                    action: InboxAction::AssetPrice { asset_id: p.asset_id },
                    fingerprint: "missing".into(),
                    snoozed_until: None,
                },
                _ => continue,
            };
            prices
                .entry(item.key.clone())
                .or_insert_with(|| (item, Vec::new()))
                .1
                .push(acct.name.clone());
        }
    }
    for (_, (mut item, accounts)) in prices {
        item.facts.push(InboxFact {
            label: "Held in".into(),
            value: accounts.join(", "),
        });
        items.push(item);
    }
    Ok(items)
}

fn stamp(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// The Inbox at `now`: snoozes that ended, or whose item resolved or changed, are dropped.
pub fn inbox_at(state: &AppState, today: Date, now: DateTime<Utc>) -> Result<InboxView, AppError> {
    let mut all = conditions(state, today)?;
    // Newest condition first; conditions without a date go last, then by kind and key.
    all.sort_by(|a, b| {
        b.since
            .is_some()
            .cmp(&a.since.is_some())
            .then_with(|| b.since.cmp(&a.since))
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.key.cmp(&b.key))
    });
    let live: Vec<(String, String)> = all
        .iter()
        .map(|i| (i.key.clone(), i.fingerprint.clone()))
        .collect();
    let snoozes = state
        .store
        .lock()
        .map_err(store_err)?
        .purge_snoozes(&stamp(now), &live)?;
    let (mut snoozed, items): (Vec<_>, Vec<_>) = all
        .into_iter()
        .partition(|i| snoozes.iter().any(|s| s.key == i.key));
    for i in &mut snoozed {
        i.snoozed_until = snoozes
            .iter()
            .find(|s| s.key == i.key)
            .map(|s| s.until.clone());
    }
    snoozed.sort_by(|a, b| a.snoozed_until.cmp(&b.snoozed_until));
    Ok(InboxView { items, snoozed })
}

/// Snooze an item for `days` (1 or 7) from `now`.
pub fn snooze_at(
    state: &AppState,
    key: &str,
    days: u32,
    today: Date,
    now: DateTime<Utc>,
) -> Result<InboxView, AppError> {
    if days != 1 && days != 7 {
        return Err(AppError::Invalid(
            "an item is snoozed for 1 or 7 days".into(),
        ));
    }
    let item = conditions(state, today)?
        .into_iter()
        .find(|i| i.key == key)
        .ok_or_else(|| AppError::Invalid(format!("no Inbox item {key}")))?;
    let until = stamp(now + Duration::days(days.into()));
    state
        .store
        .lock()
        .map_err(store_err)?
        .snooze(key, &item.fingerprint, &until)?;
    inbox_at(state, today, now)
}

pub fn unsnooze(state: &AppState, key: &str) -> Result<(), AppError> {
    state.store.lock().map_err(store_err)?.unsnooze(key)?;
    Ok(())
}

fn today() -> Date {
    Date(chrono::Local::now().date_naive())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_inbox(app: AppHandle) -> Result<InboxView, AppError> {
    blocking(app, |s| inbox_at(s, today(), Utc::now())).await
}

/// Items to act on, without snoozed ones: the sidebar badge.
#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_inbox_count(app: AppHandle) -> Result<usize, AppError> {
    blocking(app, |s| Ok(inbox_at(s, today(), Utc::now())?.items.len())).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_inbox_snooze(
    app: AppHandle,
    key: String,
    days: u32,
) -> Result<InboxView, AppError> {
    blocking(app, move |s| snooze_at(s, &key, days, today(), Utc::now())).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cmd_inbox_unsnooze(app: AppHandle, key: String) -> Result<(), AppError> {
    blocking(app, move |s| unsnooze(s, &key)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{
        import_close, import_commit, import_create_account, import_create_asset, import_open,
    };
    use mizan_storage::{PriceRow, Store};
    use std::path::{Path, PathBuf};

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../crates/mizan-core/tests/data/e2e")
            .join(name)
    }

    fn day() -> Date {
        Date::parse("2026-11-01").unwrap()
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-01T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    /// Seed and the overlapping batch committed without deciding: a failed
    /// check on ASML and cash, 20 VWCE with unknown basis, and no prices.
    fn state() -> AppState {
        let state = AppState::new(Store::open_in_memory().unwrap());
        import_open(&state, fixture("seed.json")).unwrap();
        import_create_account(&state, "DEG-4417".into()).unwrap();
        import_create_asset(&state, "isin:NL0010273215".into()).unwrap();
        import_create_asset(&state, "isin:IE00BK5BQT80".into()).unwrap();
        import_commit(&state).unwrap();
        import_open(&state, fixture("batch.json")).unwrap();
        link_all(&state);
        import_commit(&state).unwrap();
        state
    }

    /// Create every account and asset the open review does not know yet.
    fn link_all(state: &AppState) {
        let unresolved = state
            .session
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .view()
            .report
            .unresolved;
        for u in unresolved {
            match u {
                ReviewItem::UnresolvedAccount { hint } => {
                    import_create_account(state, hint.hint).unwrap();
                }
                ReviewItem::UnresolvedInstrument { key: Some(k), .. } => {
                    import_create_asset(state, k).unwrap();
                }
                ReviewItem::UnresolvedInstrument { key: None, .. } => {}
            }
        }
    }

    fn keys(v: &InboxView) -> Vec<&str> {
        v.items.iter().map(|i| i.key.as_str()).collect()
    }

    fn asset_id(st: &AppState, key: &str) -> i64 {
        st.store
            .lock()
            .unwrap()
            .list_assets()
            .unwrap()
            .into_iter()
            .find(|a| a.instrument_key == key)
            .unwrap()
            .id
    }

    fn price(st: &AppState, asset: i64, date: &str) {
        st.store
            .lock()
            .unwrap()
            .upsert_provider_price(&PriceRow {
                asset_id: asset,
                date: date.into(),
                source: "yahoo".into(),
                value: "100".into(),
                currency: "EUR".into(),
                fetched_at: "t".into(),
            })
            .unwrap();
        crate::market::bump_generation(st);
    }

    #[test]
    fn items_from_checks_basis_and_prices() {
        let st = state();
        let v = inbox_at(&st, day(), now()).unwrap();
        let check = v
            .items
            .iter()
            .find(|i| i.kind == InboxKind::Check)
            .expect("failed check");
        assert!(check.title.starts_with("Balance check failed · "));
        let asml = check
            .check_lines
            .iter()
            .find(|l| l.display == "ASML Holding")
            .unwrap();
        assert_eq!(
            (
                asml.expected.as_str(),
                asml.computed.as_str(),
                asml.difference.as_str()
            ),
            ("3", "1", "2")
        );
        assert!(matches!(check.action, InboxAction::AccountChecks { .. }));

        let cost = v
            .items
            .iter()
            .find(|i| i.kind == InboxKind::Cost)
            .expect("unknown basis");
        assert_eq!(
            cost.title,
            "Unknown cost basis · 20 Vanguard FTSE All-World in Degiro"
        );
        assert!(matches!(
            cost.action,
            InboxAction::AddPositionIn {
                asset_id: Some(_),
                ..
            }
        ));

        let missing: Vec<&InboxItem> = v
            .items
            .iter()
            .filter(|i| i.kind == InboxKind::Price)
            .collect();
        assert!(!missing.is_empty());
        assert!(missing.iter().all(|i| i.title.starts_with("No price · ")));
        assert!(missing
            .iter()
            .all(|i| i.facts.iter().any(|f| f.label == "Held in")));

        // Dated conditions come first.
        let first_undated = v.items.iter().position(|i| i.since.is_none()).unwrap();
        assert!(v.items[..first_undated].iter().all(|i| i.since.is_some()));
        assert!(v.items[first_undated..].iter().all(|i| i.since.is_none()));
    }

    #[test]
    fn open_review_with_decisions_is_an_item_until_closed() {
        let st = state();
        import_open(&st, fixture("batch.json")).unwrap();
        let v = inbox_at(&st, day(), now()).unwrap();
        let item = v
            .items
            .iter()
            .find(|i| i.key == "import:decisions")
            .expect("pending decisions");
        assert_eq!(item.action, InboxAction::ImportReview);
        assert_eq!(
            v.items[0].kind,
            InboxKind::Import,
            "today's review is newest"
        );
        import_close(&st).unwrap();
        assert!(!keys(&inbox_at(&st, day(), now()).unwrap()).contains(&"import:decisions"));
    }

    #[test]
    fn stale_price_item_and_it_resolves_with_a_fresh_price() {
        let st = state();
        let asml = asset_id(&st, "isin:NL0010273215");
        price(&st, asml, "2026-10-20");
        let v = inbox_at(&st, day(), now()).unwrap();
        let item = v
            .items
            .iter()
            .find(|i| i.key == "price:isin:NL0010273215")
            .unwrap();
        assert_eq!(item.title, "No price for 12 days · ASML Holding");
        assert_eq!(
            item.action,
            InboxAction::AssetPrice {
                asset_id: Some(asml)
            }
        );
        price(&st, asml, "2026-10-31");
        let v = inbox_at(&st, day(), now()).unwrap();
        assert!(
            !keys(&v).contains(&"price:isin:NL0010273215"),
            "a fresh price resolves it"
        );
    }

    #[test]
    fn snooze_hides_until_it_ends() {
        let st = state();
        let before = inbox_at(&st, day(), now()).unwrap();
        let cost = before
            .items
            .iter()
            .find(|i| i.kind == InboxKind::Cost)
            .unwrap()
            .key
            .clone();
        let v = snooze_at(&st, &cost, 7, day(), now()).unwrap();
        assert!(!keys(&v).contains(&cost.as_str()));
        assert_eq!(v.items.len(), before.items.len() - 1);
        assert_eq!(v.snoozed[0].key, cost);
        assert_eq!(
            v.snoozed[0].snoozed_until.as_deref(),
            Some("2026-11-08T10:00:00Z")
        );
        let later = now() + Duration::days(8);
        let v = inbox_at(&st, day(), later).unwrap();
        assert!(
            keys(&v).contains(&cost.as_str()),
            "back after the snooze ends"
        );
        assert!(v.snoozed.is_empty());
        assert!(
            snooze_at(&st, &cost, 3, day(), now()).is_err(),
            "only 1 or 7 days"
        );
        assert!(snooze_at(&st, "nope", 1, day(), now()).is_err());
    }

    #[test]
    fn snoozed_item_returns_when_its_figures_change() {
        let st = state();
        let check = inbox_at(&st, day(), now())
            .unwrap()
            .items
            .into_iter()
            .find(|i| i.kind == InboxKind::Check)
            .unwrap();
        st.store
            .lock()
            .unwrap()
            .snooze(&check.key, "older figures", "2026-12-01T00:00:00Z")
            .unwrap();
        let v = inbox_at(&st, day(), now()).unwrap();
        assert!(keys(&v).contains(&check.key.as_str()));
        assert!(
            st.store.lock().unwrap().snoozes().unwrap().is_empty(),
            "the stale snooze is dropped"
        );
    }

    #[test]
    fn unsnooze_brings_it_back() {
        let st = state();
        let key = inbox_at(&st, day(), now()).unwrap().items[0].key.clone();
        snooze_at(&st, &key, 1, day(), now()).unwrap();
        unsnooze(&st, &key).unwrap();
        assert!(keys(&inbox_at(&st, day(), now()).unwrap()).contains(&key.as_str()));
    }
}
