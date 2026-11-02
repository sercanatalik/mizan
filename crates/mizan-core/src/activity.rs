//! The Activity screen's read model and manual entries.
//!
//! Rows come from stored activities only. A manual entry is an ordinary
//! `mizan.import/1` record with adapter `manual`, committed as its own batch
//! through the same validation, resolution, fingerprinting and commit as an
//! import, so every other screen treats it exactly like imported data.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};

use mizan_import::{parse_batch_value, ChargeType, Code, Dec, ImportRecord, Kind, Money};
use mizan_storage::{
    AccountId, ActivityFilter, AssetId, BatchId, SourceFilter, StorageError, Store, StoredActivity,
    MANUAL_ADAPTER,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::consistency::minor_digits;
use crate::ordering::{order, OrderInput};
use crate::pipeline::{
    build_batch, commit, prepare, resolve_batch, CommitError, Decisions, Outcome,
};
use crate::resolve::{instrument_key, CreatePolicy, MappingResolver, PreviewResolver};

/// Rows per page.
pub const PAGE_SIZE: i64 = 200;
/// Locator of every manual entry.
pub const MANUAL_LOCATOR: &str = "manual entry";

// ---------------------------------------------------------------- views

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ActivityQuery {
    #[cfg_attr(feature = "ts", ts(type = "number[]"))]
    pub accounts: Vec<i64>,
    pub kinds: Vec<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    /// `imported` or `manual`; anything else lists both.
    pub source: Option<String>,
    pub text: Option<String>,
}

impl ActivityQuery {
    fn filter(&self) -> ActivityFilter {
        let blank = |s: &Option<String>| {
            s.as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        ActivityFilter {
            accounts: self.accounts.clone(),
            kinds: self.kinds.clone(),
            from: blank(&self.from),
            to: blank(&self.to),
            source: match self.source.as_deref() {
                Some("imported") => Some(SourceFilter::Imported),
                Some("manual") => Some(SourceFilter::Manual),
                _ => None,
            },
            text: blank(&self.text),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MoneyView {
    pub amount: String,
    pub currency: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ActivitySource {
    pub manual: bool,
    pub adapter: String,
    pub adapter_version: String,
    /// First 12 characters of the file hash.
    pub file: String,
    pub locator: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub batch_id: i64,
    /// For manual entries, when the entry was added.
    pub added_at: Option<String>,
    /// An imported activity changed by hand; its imported values are kept.
    pub amended: bool,
    /// Changed after it was imported or first entered.
    pub edited: bool,
    /// When it was last changed (RFC 3339).
    pub edited_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ActivityRow {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub id: i64,
    pub date: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub account_id: i64,
    pub account: String,
    pub kind: String,
    pub instrument: Option<String>,
    pub symbol: Option<String>,
    pub isin: Option<String>,
    pub quantity: Option<String>,
    pub price: Option<MoneyView>,
    pub net_cash: Option<MoneyView>,
    /// Charges summed per currency.
    pub charges: Vec<MoneyView>,
    pub source: ActivitySource,
    /// Every activity can be edited (imports are amended).
    pub editable: bool,
    /// Only manual entries are deleted one by one; imports are undone whole.
    pub deletable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CashTotalView {
    pub currency: String,
    pub cash_in: String,
    pub cash_out: String,
    pub net: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ActivityPage {
    pub rows: Vec<ActivityRow>,
    /// Matching activities in all pages.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub total: i64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub offset: i64,
    pub totals: Vec<CashTotalView>,
    /// Whether any activity is stored at all, whatever the filters.
    pub has_any: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ChargeView {
    /// For example `commission` or `withholding_tax`.
    pub kind: String,
    pub amount: String,
    pub currency: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LegView {
    pub instrument: String,
    pub quantity_delta: String,
    pub cost_basis_share: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ImportLink {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub batch_id: i64,
    pub committed_at: String,
    pub adapter: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ActivityDetail {
    pub row: ActivityRow,
    pub settle_date: Option<String>,
    pub gross: Option<MoneyView>,
    pub charges: Vec<ChargeView>,
    /// For example `0.79 GBP per USD`.
    pub fx: Option<String>,
    pub legs: Vec<LegView>,
    /// For example `4 for 1`.
    pub split: Option<String>,
    pub cost_basis: Option<MoneyView>,
    pub acquisition_date: Option<String>,
    pub external_id: Option<String>,
    pub links: Vec<String>,
    pub memo: Option<String>,
    pub file_sha256: String,
    /// The import this activity came from; `None` for manual entries.
    pub import: Option<ImportLink>,
    /// When an imported activity was amended.
    pub amended_at: Option<String>,
    /// The form's values, for editing.
    pub form: ActivityForm,
}

// ---------------------------------------------------------------- reading

fn money_view(m: &Money) -> MoneyView {
    MoneyView {
        amount: m.amount.value().normalize().to_string(),
        currency: m.currency.to_string(),
    }
}

/// Lookups shared by every row of a page.
struct Names {
    accounts: HashMap<AccountId, String>,
    /// Canonical asset id → (label, symbol, isin).
    assets: HashMap<AssetId, (String, Option<String>, Option<String>)>,
    canonical: HashMap<AssetId, AssetId>,
    /// Amended imports and when they were last amended.
    amended: HashMap<i64, String>,
}

impl Names {
    fn load(store: &Store) -> Result<Self, StorageError> {
        let accounts = store
            .list_accounts()?
            .into_iter()
            .map(|a| (a.id, a.name))
            .collect();
        let mut assets = HashMap::new();
        let mut canonical = HashMap::new();
        let rows = store.list_assets()?;
        for a in &rows {
            let label = match (&a.hint.symbol, a.name.as_ref().or(a.hint.name.as_ref())) {
                (Some(s), Some(n)) if s != n => format!("{s} · {n}"),
                (Some(s), _) => s.clone(),
                (None, Some(n)) => n.clone(),
                (None, None) => a.instrument_key.clone(),
            };
            assets.insert(a.id, (label, a.hint.symbol.clone(), a.hint.isin.clone()));
            canonical.insert(a.id, store.canonical_asset(a.id)?);
        }
        Ok(Names {
            accounts,
            assets,
            canonical,
            amended: store.amendment_times()?,
        })
    }

    fn asset(&self, id: Option<AssetId>) -> Option<&(String, Option<String>, Option<String>)> {
        let id = id?;
        self.assets.get(self.canonical.get(&id).unwrap_or(&id))
    }
}

fn raw_time(r: &ImportRecord, key: &str) -> Option<String> {
    r.raw
        .as_ref()
        .and_then(|v| v.get(key))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn added_at(r: &ImportRecord) -> Option<String> {
    r.raw
        .as_ref()
        .and_then(|v| v.get("created_at"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn row(names: &Names, s: &StoredActivity) -> ActivityRow {
    let r = &s.record;
    let manual = r.source.adapter == MANUAL_ADAPTER;
    let edited_at = if manual {
        raw_time(r, "edited_at")
    } else {
        names.amended.get(&s.id).cloned()
    };
    let asset = names.asset(s.asset_id);
    let mut charges: BTreeMap<String, Decimal> = BTreeMap::new();
    for c in &r.charges {
        *charges.entry(c.amount.currency.to_string()).or_default() += c.amount.amount.value();
    }
    ActivityRow {
        id: s.id,
        date: r.date.to_string(),
        account_id: s.account_id,
        account: names
            .accounts
            .get(&s.account_id)
            .cloned()
            .unwrap_or_default(),
        kind: r.kind.as_str().into(),
        instrument: asset.map(|a| a.0.clone()).or_else(|| {
            r.instrument
                .as_ref()
                .and_then(|h| h.symbol.clone().or(h.name.clone()).or(h.isin.clone()))
        }),
        symbol: asset.and_then(|a| a.1.clone()),
        isin: asset.and_then(|a| a.2.clone()),
        quantity: r.quantity.map(|q| q.value().normalize().to_string()),
        price: r.price.as_ref().map(money_view),
        net_cash: r.net_cash.as_ref().map(money_view),
        charges: charges
            .into_iter()
            .map(|(currency, a)| MoneyView {
                amount: a.normalize().to_string(),
                currency,
            })
            .collect(),
        source: ActivitySource {
            manual,
            adapter: r.source.adapter.clone(),
            adapter_version: r.source.adapter_version.clone(),
            file: r.source.file_sha256.as_str().chars().take(12).collect(),
            locator: r.source.locator.clone(),
            batch_id: s.batch_id,
            added_at: if manual { added_at(r) } else { None },
            amended: names.amended.contains_key(&s.id),
            edited: edited_at.is_some(),
            edited_at,
        },
        editable: true,
        deletable: manual,
    }
}

/// Newest day first; within a day, processing order.
fn arrange(rows: Vec<StoredActivity>) -> Vec<StoredActivity> {
    let mut by_day: BTreeMap<mizan_import::Date, Vec<StoredActivity>> = BTreeMap::new();
    for r in rows {
        by_day.entry(r.record.date).or_default().push(r);
    }
    let mut out = Vec::new();
    for (_, day) in by_day.into_iter().rev() {
        let inputs: Vec<OrderInput> = day
            .iter()
            .map(|s| OrderInput {
                account: s.account_id.to_string(),
                date: s.record.date,
                file: s.record.source.file_sha256.as_str().to_owned(),
                source_order: s.source_order,
                kind: s.record.kind,
                instrument_key: s.record.instrument.as_ref().and_then(instrument_key),
                locator: s.record.source.locator.clone(),
            })
            .collect();
        let (idx, _) = order(&inputs);
        let mut slots: Vec<Option<StoredActivity>> = day.into_iter().map(Some).collect();
        out.extend(idx.into_iter().filter_map(|i| slots[i].take()));
    }
    out
}

fn fmt2(d: Decimal, ccy: &str) -> String {
    let digits = Code::parse(ccy).map(|c| minor_digits(&c)).unwrap_or(2);
    let mut r = d.round_dp(digits);
    r.rescale(digits);
    r.to_string()
}

/// One page of the Activity list.
pub fn page(store: &Store, q: &ActivityQuery, offset: i64) -> Result<ActivityPage, StorageError> {
    let f = q.filter();
    let names = Names::load(store)?;
    let p = store.query_activities(&f, offset.max(0), PAGE_SIZE)?;
    let (_, totals) = store.activity_totals(&f)?;
    let has_any = if f == ActivityFilter::default() {
        p.total > 0
    } else {
        store
            .query_activities(&ActivityFilter::default(), 0, 1)?
            .total
            > 0
    };
    Ok(ActivityPage {
        rows: arrange(p.rows).iter().map(|s| row(&names, s)).collect(),
        total: p.total,
        offset,
        totals: totals
            .into_iter()
            .map(|(c, t)| CashTotalView {
                cash_in: fmt2(t.cash_in, &c),
                cash_out: fmt2(t.cash_out, &c),
                net: fmt2(t.net(), &c),
                currency: c,
            })
            .collect(),
        has_any,
    })
}

fn kind_name<T: Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// The full record behind one row.
pub fn detail(store: &Store, id: i64) -> Result<ActivityDetail, StorageError> {
    let s = store
        .activity(id)?
        .ok_or_else(|| StorageError::NotFound(format!("activity {id}")))?;
    let names = Names::load(store)?;
    let r = &s.record;
    let manual = r.source.adapter == MANUAL_ADAPTER;
    let import = if manual {
        None
    } else {
        store.batch(s.batch_id)?.map(|b| ImportLink {
            batch_id: b.id,
            committed_at: b.committed_at,
            adapter: b.adapter,
        })
    };
    Ok(ActivityDetail {
        row: row(&names, &s),
        settle_date: r.settle_date.map(|d| d.to_string()),
        gross: r.gross.as_ref().map(money_view),
        charges: r
            .charges
            .iter()
            .map(|c| ChargeView {
                kind: kind_name(&c.charge_type),
                amount: c.amount.amount.value().normalize().to_string(),
                currency: c.amount.currency.to_string(),
            })
            .collect(),
        fx: r
            .fx
            .as_ref()
            .map(|f| format!("{} {} per {}", f.rate.value().normalize(), f.to, f.from)),
        legs: r
            .legs
            .iter()
            .map(|l| LegView {
                instrument: l
                    .instrument
                    .symbol
                    .clone()
                    .or(l.instrument.name.clone())
                    .or(l.instrument.isin.clone())
                    .unwrap_or_default(),
                quantity_delta: l.quantity_delta.value().normalize().to_string(),
                cost_basis_share: l
                    .cost_basis_share
                    .map(|s| s.value().normalize().to_string()),
            })
            .collect(),
        split: r.split.as_ref().map(|s| {
            format!(
                "{} for {}",
                s.new.value().normalize(),
                s.old.value().normalize()
            )
        }),
        cost_basis: r.cost_basis.as_ref().map(money_view),
        acquisition_date: r.acquisition_date.map(|d| d.to_string()),
        external_id: r.external_id.clone(),
        links: r
            .links
            .iter()
            .map(|l| format!("{} {}", kind_name(&l.role), l.target))
            .collect(),
        memo: r.memo.clone(),
        file_sha256: r.source.file_sha256.as_str().to_owned(),
        import,
        amended_at: store.amendment(id)?.map(|(_, at)| at),
        form: form_of(&s),
    })
}

// ---------------------------------------------------------------- manual entries

/// The kinds the manual form offers, in menu order.
pub const MANUAL_KINDS: [&str; 11] = [
    "buy",
    "sell",
    "deposit",
    "withdrawal",
    "dividend",
    "interest",
    "fee",
    "tax",
    "position_in",
    "position_out",
    "split",
];

/// A new asset typed into the form.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct NewAssetForm {
    pub symbol: Option<String>,
    pub name: Option<String>,
    pub isin: Option<String>,
    pub currency: Option<String>,
    /// `equity`, `etf`, `fund`, `bond`, `commodity` or `other`.
    pub class: Option<String>,
}

/// The manual entry form. Amounts are plain decimal text; empty means not given.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ActivityForm {
    pub kind: String,
    /// `YYYY-MM-DD`.
    pub date: String,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub account_id: Option<i64>,
    /// Name of a new account, when `account_id` is not given.
    pub new_account: Option<String>,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub asset_id: Option<i64>,
    pub new_asset: Option<NewAssetForm>,
    /// Currency of the amounts.
    pub currency: String,
    pub quantity: Option<String>,
    pub price: Option<String>,
    pub net_cash: Option<String>,
    pub gross: Option<String>,
    pub commission: Option<String>,
    pub withholding: Option<String>,
    pub cost_basis: Option<String>,
    pub acquisition_date: Option<String>,
    pub split_new: Option<String>,
    pub split_old: Option<String>,
    pub memo: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FieldError {
    /// The form field, or `form` for the entry as a whole.
    pub field: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DuplicateView {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub activity_id: i64,
    pub date: String,
    /// For example `ibkr-flex, row 12` or `manual entry`.
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FormCheck {
    pub errors: Vec<FieldError>,
    pub warnings: Vec<String>,
    pub duplicate: Option<DuplicateView>,
}

impl FormCheck {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("{0}")]
    Storage(#[from] StorageError),
    #[error("{0}")]
    Commit(String),
    #[error("the entry is not valid")]
    Invalid(FormCheck),
    #[error("this may be a duplicate; confirm to save it anyway")]
    NeedsConfirmation(FormCheck),
}

impl From<CommitError> for SaveError {
    fn from(e: CommitError) -> Self {
        SaveError::Commit(e.to_string())
    }
}

/// When and how this entry is stamped.
#[derive(Clone, Debug)]
pub struct ManualContext {
    pub app_version: String,
    /// RFC 3339 time of the save (kept in the record as `created_at`).
    pub now: String,
}

fn blank(s: &Option<String>) -> Option<&str> {
    s.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

fn parse_dec(field: &str, s: &Option<String>, errors: &mut Vec<FieldError>) -> Option<Decimal> {
    let t = blank(s)?;
    match t.replace(',', "").parse::<Decimal>() {
        Ok(d) => Some(d),
        Err(_) => {
            errors.push(FieldError {
                field: field.into(),
                message: format!("{t:?} is not a number"),
            });
            None
        }
    }
}

fn require(field: &str, present: bool, what: &str, errors: &mut Vec<FieldError>) {
    if !present {
        errors.push(FieldError {
            field: field.into(),
            message: format!("{what} is required"),
        });
    }
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A hash unique to one manual entry.
fn entry_hash(now: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    hex::encode(Sha256::digest(format!(
        "mizan-manual:{now}:{nanos}:{n}:{}",
        std::process::id()
    )))
}

/// What the form resolves to before it becomes a record.
struct Built {
    record: Option<ImportRecord>,
    errors: Vec<FieldError>,
    /// A hint for a new account, to create on save.
    new_account: Option<(String, String)>,
}

fn money(amount: Decimal, ccy: &str) -> Value {
    json!({"amount": amount.normalize().to_string(), "currency": ccy})
}

fn build(
    store: &Store,
    f: &ActivityForm,
    ctx: &ManualContext,
    created_at: &str,
    edited_at: Option<&str>,
) -> Result<Built, StorageError> {
    let mut errors = Vec::new();
    let kind = f.kind.trim();
    if !MANUAL_KINDS.contains(&kind) {
        errors.push(FieldError {
            field: "kind".into(),
            message: format!("{kind:?} cannot be entered by hand"),
        });
    }
    let date_ok = mizan_import::Date::parse(f.date.trim()).is_ok();
    if !date_ok {
        errors.push(FieldError {
            field: "date".into(),
            message: "a date (YYYY-MM-DD) is required".into(),
        });
    }
    let ccy = f.currency.trim().to_uppercase();
    let needs_ccy = !matches!(kind, "position_out" | "split");
    if needs_ccy && Code::parse(&ccy).is_err() {
        errors.push(FieldError {
            field: "currency".into(),
            message: "a three-letter currency code is required".into(),
        });
    }

    // Account: an existing one by its first hint, or a new one.
    let mut new_account = None;
    let account_hint = match (f.account_id, blank(&f.new_account)) {
        (Some(id), _) => match store.list_accounts()?.into_iter().find(|a| a.id == id) {
            Some(a) => a
                .hints
                .first()
                .cloned()
                .map(|h| json!({"hint": h, "name_hint": a.name})),
            None => {
                errors.push(FieldError {
                    field: "account".into(),
                    message: format!("account {id} does not exist"),
                });
                None
            }
        },
        (None, Some(name)) => {
            let hint = format!("manual:{}", &entry_hash(created_at)[..16]);
            new_account = Some((name.to_owned(), hint.clone()));
            Some(json!({"hint": hint, "name_hint": name}))
        }
        (None, None) => {
            require("account", false, "an account", &mut errors);
            None
        }
    };

    // Instrument: an existing asset's stored hint, or a typed new one.
    let wants_asset = !matches!(kind, "deposit" | "withdrawal" | "interest" | "fee" | "tax");
    let instrument: Option<Value> = match (f.asset_id, &f.new_asset) {
        (Some(id), _) => {
            let id = store.canonical_asset(id)?;
            match store.list_assets()?.into_iter().find(|a| a.id == id) {
                Some(a) => Some(serde_json::to_value(&a.hint).expect("serializable")),
                None => {
                    errors.push(FieldError {
                        field: "asset".into(),
                        message: format!("asset {id} does not exist"),
                    });
                    None
                }
            }
        }
        (None, Some(n)) => {
            let mut h = serde_json::Map::new();
            for (k, v) in [("symbol", &n.symbol), ("name", &n.name), ("isin", &n.isin)] {
                if let Some(v) = blank(v) {
                    h.insert(
                        k.into(),
                        json!(if k == "name" {
                            v.to_owned()
                        } else {
                            v.to_uppercase()
                        }),
                    );
                }
            }
            if let Some(c) = blank(&n.currency) {
                h.insert("currency".into(), json!(c.to_uppercase()));
            }
            if let Some(c) = blank(&n.class) {
                h.insert("class_hint".into(), json!(c.to_lowercase()));
            }
            if h.contains_key("symbol") || h.contains_key("isin") || h.contains_key("name") {
                Some(Value::Object(h))
            } else {
                errors.push(FieldError {
                    field: "asset".into(),
                    message: "a new asset needs a symbol, ISIN or name".into(),
                });
                None
            }
        }
        (None, None) => None,
    };
    if wants_asset && instrument.is_none() && !errors.iter().any(|e| e.field == "asset") {
        require("asset", false, "an asset", &mut errors);
    }

    let qty = parse_dec("quantity", &f.quantity, &mut errors);
    let price = parse_dec("price", &f.price, &mut errors);
    let net = parse_dec("net_cash", &f.net_cash, &mut errors);
    let gross = parse_dec("gross", &f.gross, &mut errors);
    let commission = parse_dec("commission", &f.commission, &mut errors);
    let withholding = parse_dec("withholding", &f.withholding, &mut errors);
    let cost = parse_dec("cost_basis", &f.cost_basis, &mut errors);
    let split_new = parse_dec("split_new", &f.split_new, &mut errors);
    let split_old = parse_dec("split_old", &f.split_old, &mut errors);
    let acquisition = blank(&f.acquisition_date);
    if let Some(a) = acquisition {
        if mizan_import::Date::parse(a).is_err() {
            errors.push(FieldError {
                field: "acquisition_date".into(),
                message: "use YYYY-MM-DD".into(),
            });
        }
    }

    let mut rec = json!({
        "source": {"adapter": MANUAL_ADAPTER, "adapter_version": ctx.app_version, "file_sha256": entry_hash(created_at), "locator": MANUAL_LOCATOR},
        "account": account_hint.unwrap_or(json!({"hint": ""})),
        "kind": kind,
        "date": f.date.trim(),
        "raw": match edited_at {
            Some(e) => json!({"created_at": created_at, "edited_at": e}),
            None => json!({"created_at": created_at}),
        },
    });
    if let Some(i) =
        instrument.filter(|_| wants_asset || f.asset_id.is_some() || f.new_asset.is_some())
    {
        rec["instrument"] = i;
    }
    if let Some(m) = blank(&f.memo) {
        rec["memo"] = json!(m);
    }
    let mut charges = Vec::new();
    if let Some(c) = commission.filter(|c| !c.is_zero()) {
        charges.push(json!({"type": "commission", "amount": money(c, &ccy)}));
    }
    if let Some(w) = withholding.filter(|w| !w.is_zero()) {
        charges.push(json!({"type": "withholding_tax", "amount": money(w, &ccy)}));
    }

    match kind {
        "buy" | "sell" => {
            require("quantity", qty.is_some(), "a quantity", &mut errors);
            require(
                "price",
                price.is_some() || net.is_some(),
                "a price or a total",
                &mut errors,
            );
            if let Some(q) = qty {
                rec["quantity"] = json!(q.normalize().to_string());
            }
            if let Some(p) = price {
                rec["price"] = money(p, &ccy);
            }
            let fees = commission.unwrap_or_default();
            let total = net.or_else(|| match (qty, price) {
                (Some(q), Some(p)) => Some(if kind == "buy" {
                    q * p + fees
                } else {
                    q * p - fees
                }),
                _ => None,
            });
            if let Some(t) = total {
                rec["net_cash"] = money(t, &ccy);
            }
        }
        "deposit" | "withdrawal" | "interest" | "fee" | "tax" => {
            require("net_cash", net.is_some(), "an amount", &mut errors);
            if let Some(n) = net {
                rec["net_cash"] = money(n, &ccy);
            }
        }
        "dividend" => {
            require(
                "net_cash",
                net.is_some() || gross.is_some(),
                "the amount received or the gross amount",
                &mut errors,
            );
            if let Some(g) = gross {
                rec["gross"] = money(g, &ccy);
            }
            let received = net.or_else(|| gross.map(|g| g - withholding.unwrap_or_default()));
            if let Some(n) = received {
                rec["net_cash"] = money(n, &ccy);
            }
        }
        "position_in" => {
            require("quantity", qty.is_some(), "a quantity", &mut errors);
            if let Some(q) = qty {
                rec["quantity"] = json!(q.normalize().to_string());
            }
            if let Some(c) = cost {
                rec["cost_basis"] = money(c, &ccy);
            }
            if let Some(a) = acquisition {
                rec["acquisition_date"] = json!(a);
            }
        }
        "position_out" => {
            require("quantity", qty.is_some(), "a quantity", &mut errors);
            if let Some(q) = qty {
                rec["quantity"] = json!(q.normalize().to_string());
            }
        }
        "split" => {
            require(
                "split_new",
                split_new.is_some(),
                "the new number of units",
                &mut errors,
            );
            require(
                "split_old",
                split_old.is_some(),
                "the old number of units",
                &mut errors,
            );
            if let (Some(n), Some(o)) = (split_new, split_old) {
                rec["split"] =
                    json!({"new": n.normalize().to_string(), "old": o.normalize().to_string()});
            }
        }
        _ => {}
    }
    if !charges.is_empty() && matches!(kind, "buy" | "sell" | "dividend" | "interest") {
        rec["charges"] = Value::Array(charges);
    }
    if !errors.is_empty() {
        return Ok(Built {
            record: None,
            errors,
            new_account,
        });
    }
    // Schema validation, exactly as for imports.
    match parse_batch_value(json!({"schema": mizan_import::SCHEMA_V1, "records": [rec]})) {
        Ok(p) if p.errors.is_empty() && p.records.len() == 1 => Ok(Built {
            record: Some(p.records[0].item.clone()),
            errors,
            new_account,
        }),
        Ok(p) => {
            for e in p.errors {
                errors.push(FieldError {
                    field: field_of(&e.message),
                    message: e.message,
                });
            }
            Ok(Built {
                record: None,
                errors,
                new_account,
            })
        }
        Err(e) => {
            errors.push(FieldError {
                field: "form".into(),
                message: e.to_string(),
            });
            Ok(Built {
                record: None,
                errors,
                new_account,
            })
        }
    }
}

/// The form field a validation message is about, by the field name it mentions.
fn field_of(message: &str) -> String {
    for f in [
        "quantity",
        "price",
        "net_cash",
        "gross",
        "cost_basis",
        "acquisition_date",
        "split",
        "currency",
        "date",
        "instrument",
    ] {
        if message.contains(f) {
            return match f {
                "instrument" => "asset".into(),
                "split" => "split_new".into(),
                other => other.into(),
            };
        }
    }
    "form".into()
}

fn source_label(store: &Store, id: i64) -> Result<Option<DuplicateView>, StorageError> {
    Ok(store.activity(id)?.map(|s| DuplicateView {
        activity_id: id,
        date: s.record.date.to_string(),
        source: if s.record.source.adapter == MANUAL_ADAPTER {
            MANUAL_LOCATOR.into()
        } else {
            format!("{}, {}", s.record.source.adapter, s.record.source.locator)
        },
    }))
}

/// A check, the record it would save, and a new account to create.
type Checked = (FormCheck, Option<ImportRecord>, Option<(String, String)>);

/// Validate, resolve and fingerprint a form without writing anything.
/// `replacing` is the activity being edited, never counted as its own duplicate.
fn run_check(
    store: &Store,
    f: &ActivityForm,
    ctx: &ManualContext,
    replacing: Option<i64>,
) -> Result<Checked, StorageError> {
    let created_at = match replacing.and_then(|id| store.activity(id).ok().flatten()) {
        Some(s) => added_at(&s.record).unwrap_or_else(|| ctx.now.clone()),
        None => ctx.now.clone(),
    };
    // Replacing an entry records when it was edited.
    let edited_at = replacing.map(|_| ctx.now.as_str());
    let built = build(store, f, ctx, &created_at, edited_at)?;
    let Some(record) = built.record else {
        return Ok((
            FormCheck {
                errors: built.errors,
                warnings: vec![],
                duplicate: None,
            },
            None,
            built.new_account,
        ));
    };
    let parsed = parse_batch_value(json!({"schema": mizan_import::SCHEMA_V1, "records": [record]}))
        .map_err(|e| StorageError::Invalid(e.to_string()))?;
    let mut resolver = PreviewResolver::new(
        store,
        CreatePolicy {
            create_accounts: true,
            create_assets: true,
        },
    );
    let rb = resolve_batch(parsed, &mut resolver)?;
    let p = prepare(store, rb)?;
    let pr = &p.records[0];
    let warnings = pr.warnings.iter().map(|w| w.message.clone()).collect();
    let dup_id = match &pr.outcome {
        Outcome::New => None,
        Outcome::Duplicate {
            existing_activity_id,
        } => *existing_activity_id,
        Outcome::NeedsConfirmation {
            existing_activity_id,
            ..
        } => Some(*existing_activity_id),
    };
    let duplicate = match dup_id.filter(|id| Some(*id) != replacing) {
        Some(id) => source_label(store, id)?,
        None => None,
    };
    Ok((
        FormCheck {
            errors: vec![],
            warnings,
            duplicate,
        },
        Some(pr.resolved.record.clone()),
        built.new_account,
    ))
}

/// Check a form: field errors, consistency warnings and a possible duplicate.
pub fn check_form(
    store: &Store,
    f: &ActivityForm,
    ctx: &ManualContext,
    replacing: Option<i64>,
) -> Result<FormCheck, StorageError> {
    if let Some(s) = replacing.and_then(|id| store.activity(id).ok().flatten()) {
        if s.record.source.adapter != MANUAL_ADAPTER && !uses_form(&s, f) {
            let (errors, record) = generic_patch(&s, f);
            return Ok(FormCheck {
                errors,
                warnings: record.map(|r| warnings_of(&r)).unwrap_or_default(),
                duplicate: None,
            });
        }
    }
    Ok(run_check(store, f, ctx, replacing)?.0)
}

/// Save a new manual entry, or replace the entry `replacing`. Returns the new
/// activity id.
pub fn save_form(
    store: &mut Store,
    f: &ActivityForm,
    ctx: &ManualContext,
    confirm_duplicate: bool,
    replacing: Option<i64>,
) -> Result<i64, SaveError> {
    let old_batch: Option<BatchId> = match replacing {
        Some(id) => {
            let s = store
                .activity(id)?
                .ok_or_else(|| StorageError::NotFound(format!("activity {id}")))?;
            if s.record.source.adapter != MANUAL_ADAPTER {
                return amend_imported(store, &s, f, ctx, confirm_duplicate);
            }
            Some(s.batch_id)
        }
        None => None,
    };
    let (check, record, new_account) = run_check(store, f, ctx, replacing)?;
    if !check.ok() {
        return Err(SaveError::Invalid(check));
    }
    if check.duplicate.is_some() && !confirm_duplicate {
        return Err(SaveError::NeedsConfirmation(check));
    }
    let record = record.expect("checked");
    if let Some((name, hint)) = new_account {
        store.create_account(&name, &hint)?;
    }
    let parsed = parse_batch_value(json!({"schema": mizan_import::SCHEMA_V1, "records": [record]}))
        .map_err(|e| StorageError::Invalid(e.to_string()))?;
    let rb = {
        let mut r = MappingResolver {
            store,
            policy: CreatePolicy {
                create_accounts: false,
                create_assets: true,
            },
        };
        resolve_batch(parsed, &mut r)?
    };
    let mut p = prepare(store, rb)?;
    // The user confirmed any possible duplicate (or it was the entry being
    // replaced): the entry is a record of its own.
    p.records[0].outcome = Outcome::New;
    let decisions = Decisions::new();
    let batch_id = match old_batch {
        None => commit(store, &p, &decisions)?.batch_id,
        Some(old) => {
            let batch = build_batch(store, &p, &decisions)?;
            store.replace_manual_batch(old, &batch)?
        }
    };
    let id = store
        .query_activities(
            &ActivityFilter {
                source: Some(SourceFilter::Manual),
                ..Default::default()
            },
            0,
            i64::MAX,
        )?
        .rows
        .into_iter()
        .find(|s| s.batch_id == batch_id)
        .map(|s| s.id)
        .ok_or_else(|| SaveError::Commit("the entry was not stored".into()))?;
    Ok(id)
}

/// Whether an imported activity is amended through the full form: both its
/// stored kind and the form's kind are kinds the form covers.
fn uses_form(s: &StoredActivity, f: &ActivityForm) -> bool {
    MANUAL_KINDS.contains(&s.record.kind.as_str()) && MANUAL_KINDS.contains(&f.kind.trim())
}

fn warnings_of(r: &ImportRecord) -> Vec<String> {
    crate::consistency::check_record(0, r)
        .into_iter()
        .map(|w| w.message)
        .collect()
}

/// Kinds the form does not cover: change date, quantity, price, gross, net
/// cash and memo on the stored record and keep everything else.
fn generic_patch(s: &StoredActivity, f: &ActivityForm) -> (Vec<FieldError>, Option<ImportRecord>) {
    let mut errors = Vec::new();
    let mut r = s.record.clone();
    match mizan_import::Date::parse(f.date.trim()) {
        Ok(d) => r.date = d,
        Err(_) => errors.push(FieldError {
            field: "date".into(),
            message: "a date (YYYY-MM-DD) is required".into(),
        }),
    }
    let ccy = |field: &Option<Money>| -> Option<Code> {
        Code::parse(f.currency.trim())
            .ok()
            .or_else(|| field.as_ref().map(|m| m.currency.clone()))
    };
    let qty = parse_dec("quantity", &f.quantity, &mut errors);
    let price = parse_dec("price", &f.price, &mut errors);
    let gross = parse_dec("gross", &f.gross, &mut errors);
    let net = parse_dec("net_cash", &f.net_cash, &mut errors);
    r.quantity = qty.map(Dec);
    let money_of = |v: Option<Decimal>, old: &Option<Money>| -> Option<Money> {
        v.and_then(|a| {
            ccy(old).map(|c| Money {
                amount: Dec(a),
                currency: c,
            })
        })
    };
    r.price = money_of(price, &s.record.price);
    r.gross = money_of(gross, &s.record.gross);
    r.net_cash = money_of(net, &s.record.net_cash);
    r.memo = blank(&f.memo).map(str::to_owned);
    if !errors.is_empty() {
        return (errors, None);
    }
    let invalid = mizan_import::validate_record(&r);
    if !invalid.is_empty() {
        return (
            invalid
                .into_iter()
                .map(|m| FieldError {
                    field: field_of(&m),
                    message: m,
                })
                .collect(),
            None,
        );
    }
    (errors, Some(r))
}

/// Amend an imported activity in place, keeping its provenance and the
/// record as first imported.
fn amend_imported(
    store: &mut Store,
    s: &StoredActivity,
    f: &ActivityForm,
    ctx: &ManualContext,
    confirm_duplicate: bool,
) -> Result<i64, SaveError> {
    let old = &s.record;
    let (record, account_id, asset_id) = if uses_form(s, f) {
        let (check, built, new_account) = run_check(store, f, ctx, Some(s.id))?;
        if !check.ok() {
            return Err(SaveError::Invalid(check));
        }
        if check.duplicate.is_some() && !confirm_duplicate {
            return Err(SaveError::NeedsConfirmation(check));
        }
        if let Some((name, hint)) = new_account {
            store.create_account(&name, &hint)?;
        }
        let built = built.expect("checked");
        let parsed =
            parse_batch_value(json!({"schema": mizan_import::SCHEMA_V1, "records": [built]}))
                .map_err(|e| StorageError::Invalid(e.to_string()))?;
        let rb = {
            let mut r = MappingResolver {
                store,
                policy: CreatePolicy {
                    create_accounts: false,
                    create_assets: true,
                },
            };
            resolve_batch(parsed, &mut r)?
        };
        let rr =
            rb.records.into_iter().next().ok_or_else(|| {
                SaveError::Commit("the amended record could not be resolved".into())
            })?;
        let account_id = rr
            .account_id
            .ok_or_else(|| SaveError::Commit("the account could not be found".into()))?;
        let mut r = rr.record;
        // The statement's own provenance and details stay.
        r.source = old.source.clone();
        r.external_id = old.external_id.clone();
        r.links = old.links.clone();
        r.fx = old.fx.clone();
        r.settle_date = old.settle_date;
        r.raw = old.raw.clone();
        if account_id == s.account_id {
            r.account = old.account.clone();
        }
        (r, account_id, rr.asset_id)
    } else {
        let (errors, patched) = generic_patch(s, f);
        let Some(r) = patched else {
            return Err(SaveError::Invalid(FormCheck {
                errors,
                warnings: vec![],
                duplicate: None,
            }));
        };
        (r, f.account_id.unwrap_or(s.account_id), s.asset_id)
    };
    let invalid = mizan_import::validate_record(&record);
    if !invalid.is_empty() {
        return Err(SaveError::Invalid(FormCheck {
            errors: invalid
                .into_iter()
                .map(|m| FieldError {
                    field: field_of(&m),
                    message: m,
                })
                .collect(),
            warnings: vec![],
            duplicate: None,
        }));
    }
    let basis_unknown = record.kind == Kind::PositionIn && record.cost_basis.is_none();
    store.amend_activity(
        &mizan_storage::AmendedActivity {
            id: s.id,
            account_id,
            asset_id,
            basis_unknown,
            record,
        },
        &ctx.now,
    )?;
    Ok(s.id)
}

/// Restore an amended imported activity's imported values.
pub fn revert_entry(store: &mut Store, id: i64) -> Result<(), StorageError> {
    let s = store
        .activity(id)?
        .ok_or_else(|| StorageError::NotFound(format!("activity {id}")))?;
    let (original, _) = store
        .amendment(id)?
        .ok_or_else(|| StorageError::Invalid("this activity has not been amended".into()))?;
    let account = store
        .account_for_hint(&original.account.hint)?
        .unwrap_or(s.account_id);
    let asset = match original.instrument.as_ref().and_then(instrument_key) {
        Some(key) => match store.asset_for_key(&key)? {
            Some(a) => Some(store.canonical_asset(a)?),
            None => s.asset_id,
        },
        None => None,
    };
    store.revert_activity(id, account, asset)
}

/// Delete a manual entry. Imported activities are refused.
pub fn delete_entry(store: &mut Store, id: i64) -> Result<(), StorageError> {
    let s = store
        .activity(id)?
        .ok_or_else(|| StorageError::NotFound(format!("activity {id}")))?;
    store.delete_manual_batch(s.batch_id)
}

/// A manual entry's record back as form values, for editing.
fn form_of(s: &StoredActivity) -> ActivityForm {
    let r = &s.record;
    let dec = |d: Option<Dec>| d.map(|d| d.value().normalize().to_string());
    let amt = |m: &Option<Money>| m.as_ref().map(|m| m.amount.value().normalize().to_string());
    let charge = |t: ChargeType| {
        r.charges
            .iter()
            .find(|c| c.charge_type == t)
            .map(|c| c.amount.amount.value().normalize().to_string())
    };
    let currency = [&r.net_cash, &r.price, &r.gross, &r.cost_basis]
        .into_iter()
        .flatten()
        .next()
        .map(|m| m.currency.to_string())
        .unwrap_or_default();
    let explicit_net = match r.kind {
        // Buys and sells computed their total; keep it only if it differs.
        Kind::Buy | Kind::Sell => match (r.quantity, &r.price, &r.net_cash) {
            (Some(q), Some(p), Some(n)) => {
                let fees: Decimal = r.charges.iter().map(|c| c.amount.amount.value()).sum();
                let computed = if r.kind == Kind::Buy {
                    q.value() * p.amount.value() + fees
                } else {
                    q.value() * p.amount.value() - fees
                };
                (computed != n.amount.value()).then(|| n.amount.value().normalize().to_string())
            }
            _ => amt(&r.net_cash),
        },
        _ => amt(&r.net_cash),
    };
    ActivityForm {
        kind: r.kind.as_str().into(),
        date: r.date.to_string(),
        account_id: Some(s.account_id),
        new_account: None,
        asset_id: s.asset_id,
        new_asset: None,
        currency,
        quantity: dec(r.quantity),
        price: amt(&r.price),
        net_cash: explicit_net,
        gross: amt(&r.gross),
        commission: charge(ChargeType::Commission),
        withholding: charge(ChargeType::WithholdingTax),
        cost_basis: amt(&r.cost_basis),
        acquisition_date: r.acquisition_date.map(|d| d.to_string()),
        split_new: r
            .split
            .as_ref()
            .map(|s| s.new.value().normalize().to_string()),
        split_old: r
            .split
            .as_ref()
            .map(|s| s.old.value().normalize().to_string()),
        memo: r.memo.clone(),
    }
}
