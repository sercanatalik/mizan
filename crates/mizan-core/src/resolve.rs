//! Turning account and instrument hints into internal ids.
//!
//! Resolution never fails a record: a hint that cannot be resolved becomes a
//! [`ReviewItem`] for the user to settle.

use std::collections::BTreeSet;

use mizan_import::{AccountHint, ImportRecord, Indexed, InstrumentHint, OptionRight};
use mizan_storage::{AccountId, AssetId, StorageError, Store};

use crate::review::ReviewItem;

/// Identity key for an instrument, from its strongest hint.
///
/// Priority: option terms, then ISIN > FIGI > CUSIP > (symbol, MIC) > normalized name.
/// Returns `None` when the hint has no identifier.
pub fn instrument_key(h: &InstrumentHint) -> Option<String> {
    fn clean(s: &Option<String>) -> Option<String> {
        s.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_uppercase)
    }
    if let Some(o) = &h.option {
        let right = match o.right {
            OptionRight::Call => "C",
            OptionRight::Put => "P",
        };
        return Some(format!(
            "opt:{}:{}:{}:{}",
            o.underlying.trim().to_uppercase(),
            o.expiry,
            right,
            o.strike.value().normalize()
        ));
    }
    if let Some(v) = clean(&h.isin) {
        return Some(format!("isin:{v}"));
    }
    if let Some(v) = clean(&h.figi) {
        return Some(format!("figi:{v}"));
    }
    if let Some(v) = clean(&h.cusip) {
        return Some(format!("cusip:{v}"));
    }
    if let Some(v) = clean(&h.symbol) {
        return Some(format!("sym:{v}@{}", clean(&h.mic).unwrap_or_default()));
    }
    let name = h.name.as_deref().unwrap_or("");
    let normalized = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!normalized.is_empty()).then(|| format!("name:{normalized}"))
}

/// Looks up (or creates) internal ids for hints.
pub trait Resolver {
    fn account(&mut self, hint: &AccountHint) -> Result<Option<AccountId>, StorageError>;
    fn instrument(&mut self, hint: &InstrumentHint) -> Result<Option<AssetId>, StorageError>;
}

/// Whether unknown hints create new accounts and assets, or are left for review.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CreatePolicy {
    pub create_accounts: bool,
    pub create_assets: bool,
}

/// Resolver backed by the user's mappings in the store.
///
/// Accounts resolve through the hint table the user maintains; assets resolve
/// by [`instrument_key`]. Unknown hints are created only when the policy allows.
pub struct MappingResolver<'a> {
    pub store: &'a mut Store,
    pub policy: CreatePolicy,
}

impl Resolver for MappingResolver<'_> {
    fn account(&mut self, hint: &AccountHint) -> Result<Option<AccountId>, StorageError> {
        if let Some(id) = self.store.account_for_hint(&hint.hint)? {
            return Ok(Some(id));
        }
        if self.policy.create_accounts {
            let name = hint.name_hint.as_deref().unwrap_or(&hint.hint);
            return self.store.create_account(name, &hint.hint).map(Some);
        }
        Ok(None)
    }

    fn instrument(&mut self, hint: &InstrumentHint) -> Result<Option<AssetId>, StorageError> {
        let Some(key) = instrument_key(hint) else {
            return Ok(None);
        };
        if let Some(id) = self.store.asset_for_key(&key)? {
            return Ok(Some(self.store.canonical_asset(id)?));
        }
        if self.policy.create_assets {
            return self.store.create_asset(&key, hint).map(Some);
        }
        Ok(None)
    }
}

/// A record with its hints resolved as far as possible.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedRecord {
    /// Position in the batch's `records` array.
    pub index: usize,
    pub record: ImportRecord,
    pub account_id: Option<AccountId>,
    /// `None` when the record names no instrument or it could not be resolved.
    pub asset_id: Option<AssetId>,
    pub instrument_key: Option<String>,
}

impl ResolvedRecord {
    pub fn is_resolved(&self) -> bool {
        self.account_id.is_some() && (self.record.instrument.is_none() || self.asset_id.is_some())
    }
}

/// Resolve every record's hints. Each distinct unresolved hint yields one review item.
pub fn resolve_records(
    records: &[Indexed<ImportRecord>],
    resolver: &mut dyn Resolver,
) -> Result<(Vec<ResolvedRecord>, Vec<ReviewItem>), StorageError> {
    let mut out = Vec::with_capacity(records.len());
    let mut items = Vec::new();
    let mut seen_accounts = BTreeSet::new();
    let mut seen_instruments = BTreeSet::new();

    for Indexed { index, item } in records {
        let account_id = resolver.account(&item.account)?;
        if account_id.is_none() && seen_accounts.insert(item.account.hint.clone()) {
            items.push(ReviewItem::UnresolvedAccount {
                hint: item.account.clone(),
            });
        }
        let (asset_id, key) = match &item.instrument {
            None => (None, None),
            Some(h) => {
                let key = instrument_key(h);
                let id = resolver.instrument(h)?;
                if id.is_none() && seen_instruments.insert(key.clone().unwrap_or_default()) {
                    items.push(ReviewItem::UnresolvedInstrument {
                        key: key.clone(),
                        hint: Box::new(h.clone()),
                    });
                }
                (id, key)
            }
        };
        out.push(ResolvedRecord {
            index: *index,
            record: item.clone(),
            account_id,
            asset_id,
            instrument_key: key,
        });
    }
    Ok((out, items))
}

/// Read-only resolver for previews: looks hints up in the store and, when the
/// policy allows creation, hands out temporary negative ids instead of writing.
pub struct PreviewResolver<'a> {
    pub store: &'a Store,
    pub policy: CreatePolicy,
    next_id: i64,
    accounts: std::collections::HashMap<String, AccountId>,
    assets: std::collections::HashMap<String, AssetId>,
}

impl<'a> PreviewResolver<'a> {
    pub fn new(store: &'a Store, policy: CreatePolicy) -> Self {
        PreviewResolver {
            store,
            policy,
            next_id: -1,
            accounts: Default::default(),
            assets: Default::default(),
        }
    }

    fn temp_id(&mut self) -> i64 {
        let id = self.next_id;
        self.next_id -= 1;
        id
    }
}

impl Resolver for PreviewResolver<'_> {
    fn account(&mut self, hint: &AccountHint) -> Result<Option<AccountId>, StorageError> {
        if let Some(id) = self.store.account_for_hint(&hint.hint)? {
            return Ok(Some(id));
        }
        if !self.policy.create_accounts {
            return Ok(None);
        }
        if let Some(id) = self.accounts.get(&hint.hint) {
            return Ok(Some(*id));
        }
        let id = self.temp_id();
        self.accounts.insert(hint.hint.clone(), id);
        Ok(Some(id))
    }

    fn instrument(&mut self, hint: &InstrumentHint) -> Result<Option<AssetId>, StorageError> {
        let Some(key) = instrument_key(hint) else {
            return Ok(None);
        };
        if let Some(id) = self.store.asset_for_key(&key)? {
            return Ok(Some(self.store.canonical_asset(id)?));
        }
        if !self.policy.create_assets {
            return Ok(None);
        }
        if let Some(id) = self.assets.get(&key) {
            return Ok(Some(*id));
        }
        let id = self.temp_id();
        self.assets.insert(key, id);
        Ok(Some(id))
    }
}
