//! Price-source suggestions for assets, and the assets view.

use mizan_import::{ClassHint, InstrumentHint};
use mizan_storage::PriceSourceKind;

/// Suggest a TEFAS fund code when an asset without a price source looks like a
/// TEFAS fund: class hint `fund` or a TRY quote currency, together with a
/// three-character upper-case code of letters and digits with at least one
/// letter (`AFT`, `AH1`). A suggestion is never applied automatically.
pub fn tefas_suggestion(hint: &InstrumentHint, source: Option<&PriceSourceKind>) -> Option<String> {
    if source.is_some() {
        return None;
    }
    let symbol = hint.symbol.as_deref()?.trim();
    let looks_like_code = is_tefas_code(symbol) && symbol.len() == 3;
    let fund_like = hint.class_hint == Some(ClassHint::Fund)
        || hint.currency.as_ref().is_some_and(|c| c.as_str() == "TRY");
    (looks_like_code && fund_like).then(|| symbol.to_owned())
}

/// A TEFAS fund code: up to six upper-case letters and digits, at least one letter.
pub fn is_tefas_code(code: &str) -> bool {
    (1..=6).contains(&code.len())
        && code
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        && code.bytes().any(|b| b.is_ascii_uppercase())
}

/// The source an account's default provider gives one of its assets: the
/// imported symbol as the TEFAS code or Twelve Data symbol, and for Yahoo the
/// suggested Yahoo symbol, else the plain symbol. `None` when the asset has no
/// usable symbol.
pub fn account_default_source(provider: &str, hint: &InstrumentHint) -> Option<PriceSourceKind> {
    let symbol = hint.symbol.as_deref()?.trim().to_uppercase();
    if symbol.is_empty() || symbol.contains(char::is_whitespace) {
        return None;
    }
    match provider {
        "tefas" => is_tefas_code(&symbol).then_some(PriceSourceKind::Tefas { fund_code: symbol }),
        "yahoo" => Some(PriceSourceKind::Yahoo {
            symbol: listed_suggestion(hint, None).unwrap_or(symbol),
        }),
        "twelvedata" => Some(PriceSourceKind::TwelveData { symbol }),
        _ => None,
    }
}

/// What applying the account defaults did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppliedSources {
    /// Assets that got a price source.
    pub set: Vec<mizan_storage::AssetId>,
    /// Assets left without one: no usable symbol for the provider.
    pub skipped: Vec<String>,
}

/// Give every asset held in an account with a default provider, and without
/// a price source, the source that provider implies. Sources the user chose
/// are never replaced. Run after imports, manual entries and setting a default.
pub fn apply_account_sources(store: &mut Store) -> Result<AppliedSources, StorageError> {
    let mut out = AppliedSources::default();
    let assets = store.list_assets()?;
    for (account, provider) in store.account_price_sources()? {
        for id in store.account_asset_ids(account)? {
            if out.set.contains(&id) || store.price_source(id)?.is_some() {
                continue;
            }
            let Some(asset) = assets.iter().find(|a| a.id == id) else {
                continue;
            };
            match account_default_source(&provider, &asset.hint) {
                Some(src) => {
                    store.set_price_source(id, Some(&src))?;
                    out.set.push(id);
                }
                None => out
                    .skipped
                    .push(label_of(&asset.hint, &asset.instrument_key)),
            }
        }
    }
    out.skipped.sort();
    out.skipped.dedup();
    Ok(out)
}

/// Yahoo suffix per ISO 10383 exchange code (MIC). US exchanges have none.
const YAHOO_SUFFIX: &[(&str, &str)] = &[
    ("XNAS", ""),
    ("XNYS", ""),
    ("ARCX", ""),
    ("BATS", ""),
    ("XASE", ""),
    ("XETR", ".DE"),
    ("XFRA", ".F"),
    ("XAMS", ".AS"),
    ("XPAR", ".PA"),
    ("XBRU", ".BR"),
    ("XLIS", ".LS"),
    ("XMIL", ".MI"),
    ("XMAD", ".MC"),
    ("XLON", ".L"),
    ("XSWX", ".SW"),
    ("XWBO", ".VI"),
    ("XSTO", ".ST"),
    ("XCSE", ".CO"),
    ("XHEL", ".HE"),
    ("XOSL", ".OL"),
    ("XDUB", ".IR"),
];

/// Suggest a Yahoo symbol for an asset without a price source: the imported
/// symbol plus the Yahoo suffix of its exchange code, or the plain symbol for a
/// USD asset without an exchange code. A dot in a US symbol becomes a hyphen
/// (`BRK.B` → `BRK-B`). Computed from stored data only; never applied
/// automatically, and not offered when the TEFAS suggestion applies.
pub fn listed_suggestion(
    hint: &InstrumentHint,
    source: Option<&PriceSourceKind>,
) -> Option<String> {
    if source.is_some() || tefas_suggestion(hint, None).is_some() {
        return None;
    }
    let symbol = hint.symbol.as_deref()?.trim().to_uppercase();
    if symbol.is_empty() || symbol.contains(char::is_whitespace) {
        return None;
    }
    let suffix = match hint.mic.as_deref().map(|m| m.trim().to_uppercase()) {
        Some(mic) if !mic.is_empty() => YAHOO_SUFFIX
            .iter()
            .find(|(m, _)| *m == mic)
            .map(|(_, s)| *s)?,
        _ if hint.currency.as_ref().is_some_and(|c| c.as_str() == "USD") => "",
        _ => return None,
    };
    if suffix.is_empty() {
        Some(symbol.replace('.', "-"))
    } else {
        Some(format!("{symbol}{suffix}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn listed_suggestions() {
        let s = |v| listed_suggestion(&hint(v), None);
        assert_eq!(
            s(json!({"symbol": "VWCE", "mic": "XETR", "currency": "EUR"})).as_deref(),
            Some("VWCE.DE")
        );
        assert_eq!(
            s(json!({"symbol": "AAPL", "currency": "USD"})).as_deref(),
            Some("AAPL")
        );
        assert_eq!(
            s(json!({"symbol": "BRK.B", "mic": "XNYS"})).as_deref(),
            Some("BRK-B")
        );
        assert_eq!(
            s(json!({"symbol": "VUSA", "mic": "XLON", "currency": "GBP"})).as_deref(),
            Some("VUSA.L")
        );
        assert_eq!(
            s(json!({"symbol": "7203", "mic": "XTKS", "currency": "JPY"})),
            None,
            "unknown exchange"
        );
        assert_eq!(
            s(json!({"symbol": "SAP", "currency": "EUR"})),
            None,
            "no exchange and not USD"
        );
        assert_eq!(
            s(json!({"symbol": "AFT", "class_hint": "fund", "currency": "TRY"})),
            None,
            "TEFAS suggestion wins"
        );
        assert_eq!(
            listed_suggestion(
                &hint(json!({"symbol": "AAPL", "currency": "USD"})),
                Some(&PriceSourceKind::Manual)
            ),
            None,
            "already has a source"
        );
    }

    fn hint(v: serde_json::Value) -> InstrumentHint {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn suggests_tefas_for_fund_like_hints() {
        assert_eq!(
            tefas_suggestion(
                &hint(json!({"symbol": "AFT", "class_hint": "fund", "currency": "TRY"})),
                None
            )
            .as_deref(),
            Some("AFT")
        );
        assert_eq!(
            tefas_suggestion(&hint(json!({"symbol": "TCD", "currency": "TRY"})), None).as_deref(),
            Some("TCD")
        );
        assert_eq!(
            tefas_suggestion(&hint(json!({"symbol": "AAL", "class_hint": "fund"})), None)
                .as_deref(),
            Some("AAL")
        );
    }

    #[test]
    fn tefas_codes_may_contain_digits() {
        assert_eq!(
            tefas_suggestion(&hint(json!({"symbol": "AH1", "class_hint": "fund"})), None)
                .as_deref(),
            Some("AH1")
        );
        assert!(!is_tefas_code("123"));
        assert!(!is_tefas_code("ah1"));
    }

    #[test]
    fn account_default_uses_the_symbol() {
        let h = hint(json!({"symbol": "ah5", "class_hint": "fund"}));
        assert_eq!(
            account_default_source("tefas", &h),
            Some(PriceSourceKind::Tefas {
                fund_code: "AH5".into()
            })
        );
        let v = hint(json!({"symbol": "VWCE", "mic": "XETR"}));
        assert_eq!(
            account_default_source("yahoo", &v),
            Some(PriceSourceKind::Yahoo {
                symbol: "VWCE.DE".into()
            })
        );
        assert_eq!(
            account_default_source("tefas", &hint(json!({"name": "No symbol"}))),
            None
        );
    }

    #[test]
    fn no_suggestion_otherwise() {
        assert_eq!(
            tefas_suggestion(&hint(json!({"symbol": "AAPL", "currency": "USD"})), None),
            None
        );
        assert_eq!(
            tefas_suggestion(
                &hint(json!({"symbol": "SAP", "currency": "EUR", "class_hint": "equity"})),
                None
            ),
            None
        );
        assert_eq!(
            tefas_suggestion(&hint(json!({"symbol": "aft", "class_hint": "fund"})), None),
            None
        );
        assert_eq!(
            tefas_suggestion(
                &hint(json!({"isin": "TRYAKPT00001", "currency": "TRY"})),
                None
            ),
            None
        );
        let src = PriceSourceKind::Manual;
        assert_eq!(
            tefas_suggestion(
                &hint(json!({"symbol": "AFT", "class_hint": "fund"})),
                Some(&src)
            ),
            None,
            "already has a source"
        );
    }
}

// ---------------------------------------------------------------- assets view

use mizan_import::Date;
use mizan_storage::{StorageError, Store};
use serde::Serialize;

use crate::portfolio::PriceView;
use crate::valuation::{is_stale, Market};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Tefas,
    Yahoo,
    Twelvedata,
    Manual,
    None,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SourceView {
    pub kind: SourceKind,
    /// Fund code (TEFAS) or provider symbol (Yahoo, Twelve Data).
    pub symbol: Option<String>,
}

impl SourceView {
    pub fn of(source: Option<&PriceSourceKind>) -> Self {
        let kind = match source {
            Some(PriceSourceKind::Tefas { .. }) => SourceKind::Tefas,
            Some(PriceSourceKind::Yahoo { .. }) => SourceKind::Yahoo,
            Some(PriceSourceKind::TwelveData { .. }) => SourceKind::Twelvedata,
            Some(PriceSourceKind::Manual) => SourceKind::Manual,
            None => SourceKind::None,
        };
        SourceView {
            kind,
            symbol: source.and_then(|s| s.symbol()).map(str::to_owned),
        }
    }
}

/// A price source Mizan proposes for an asset; applied only when accepted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SuggestionView {
    pub kind: SourceKind,
    pub symbol: String,
    /// For example "TEFAS: AFT" or "Yahoo: VWCE.DE".
    pub label: String,
}

/// The suggestion for an asset, TEFAS first, then a Yahoo symbol.
pub fn suggestion(
    hint: &InstrumentHint,
    source: Option<&PriceSourceKind>,
) -> Option<PriceSourceKind> {
    if let Some(code) = tefas_suggestion(hint, source) {
        return Some(PriceSourceKind::Tefas { fund_code: code });
    }
    listed_suggestion(hint, source).map(|symbol| PriceSourceKind::Yahoo { symbol })
}

impl SuggestionView {
    pub fn of(s: &PriceSourceKind) -> Option<Self> {
        let symbol = s.symbol()?.to_owned();
        let (kind, name) = match s {
            PriceSourceKind::Tefas { .. } => (SourceKind::Tefas, "TEFAS"),
            PriceSourceKind::Yahoo { .. } => (SourceKind::Yahoo, "Yahoo"),
            PriceSourceKind::TwelveData { .. } => (SourceKind::Twelvedata, "Twelve Data"),
            PriceSourceKind::Manual => return None,
        };
        Some(SuggestionView {
            label: format!("{name}: {symbol}"),
            kind,
            symbol,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ManualPrice {
    pub date: String,
    pub value: String,
    pub currency: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MergedInto {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub merge_id: i64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub asset_id: i64,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AssetView {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub asset_id: i64,
    pub instrument_key: String,
    /// Symbol, else name, else the key.
    pub label: String,
    pub name: Option<String>,
    pub class: String,
    pub currency: Option<String>,
    pub source: SourceView,
    pub suggestion: Option<SuggestionView>,
    pub latest_price: Option<PriceView>,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub holders: usize,
    /// Set when this asset was merged into another.
    pub merged_into: Option<MergedInto>,
    pub manual_prices: Vec<ManualPrice>,
    /// Marked as a benchmark: offered on Performance, history kept in full.
    pub benchmark: bool,
}

fn label_of(h: &InstrumentHint, key: &str) -> String {
    h.symbol
        .clone()
        .or_else(|| h.name.clone())
        .unwrap_or_else(|| key.to_owned())
}

/// Every asset with its price source, suggestion, latest price and holders.
pub fn assets_view(store: &Store, today: Date) -> Result<Vec<AssetView>, StorageError> {
    assets_view_in(store, store, today)
}

/// The Assets screen valued through `market`.
pub fn assets_view_in(
    store: &Store,
    market: &dyn Market,
    today: Date,
) -> Result<Vec<AssetView>, StorageError> {
    let hold = crate::portfolio::holdings_in(store, market, today)?;
    let assets = store.list_assets()?;
    let merges = store.active_merges()?;
    let benchmarks = store.benchmark_candidates()?;
    let mut out = Vec::new();
    for a in &assets {
        let source = store.price_source(a.id)?;
        let holders = hold
            .accounts
            .iter()
            .filter(|acc| acc.positions.iter().any(|p| p.asset_id == Some(a.id)))
            .count();
        let latest_price = market.quote(a.id, today)?.map(|q| PriceView {
            value: q.value.normalize().to_string(),
            currency: q.currency.to_string(),
            date: q.date.to_string(),
            stale: is_stale(q.date, today),
            source: q.source,
        });
        let merged_into = merges.iter().find(|m| m.from_asset == a.id).map(|m| {
            let target = assets.iter().find(|t| t.id == m.into_asset);
            MergedInto {
                merge_id: m.id,
                asset_id: m.into_asset,
                label: target
                    .map(|t| label_of(&t.hint, &t.instrument_key))
                    .unwrap_or_default(),
            }
        });
        let manual_prices = store
            .prices_for(a.id)?
            .into_iter()
            .filter(|p| p.source == mizan_storage::MANUAL)
            .map(|p| ManualPrice {
                date: p.date,
                value: p.value,
                currency: p.currency,
            })
            .collect();
        out.push(AssetView {
            asset_id: a.id,
            instrument_key: a.instrument_key.clone(),
            label: label_of(&a.hint, &a.instrument_key),
            name: a.hint.name.clone(),
            class: a
                .hint
                .class_hint
                .and_then(|c| serde_json::to_value(c).ok())
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_else(|| "other".into()),
            currency: a.hint.currency.as_ref().map(|c| c.to_string()),
            suggestion: suggestion(&a.hint, source.as_ref())
                .as_ref()
                .and_then(SuggestionView::of),
            source: SourceView::of(source.as_ref()),
            latest_price,
            holders,
            merged_into,
            manual_prices,
            benchmark: benchmarks.contains(&a.id),
        });
    }
    Ok(out)
}
