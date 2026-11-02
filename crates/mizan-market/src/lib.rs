//! Market data for Mizan: built-in providers (TEFAS fund prices, TCMB exchange
//! rates, Yahoo Finance and Twelve Data listed-asset closes, Eurostat, BLS
//! and ONS inflation indices, and the EVDS producer price index) and the
//! refresh that stores their results.
//!
//! This is the only crate that touches the network, and only inside
//! [`refresh`]. Everything else reads stored prices and rates.

pub mod backfill;
pub mod bls;
pub mod eurostat;
pub mod evds;
pub mod http;
pub mod inflation;
pub mod listed;
pub mod ons;
pub mod refresh;
pub mod tcmb;
pub mod tefas;
pub mod twelvedata;
pub mod yahoo;

pub use http::{Http, HttpError, ReqwestHttp, Response};
pub use refresh::{refresh, RefreshReport, SourceReport, SourceStatus};

/// A market data provider that ships with Mizan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProviderInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub supplies: &'static str,
}

/// Built-in providers, enabled by default; each can be turned off in Settings.
pub const PROVIDERS: [ProviderInfo; 8] = [
    ProviderInfo {
        id: tefas::SOURCE,
        name: "TEFAS",
        supplies: "Turkish investment and pension fund prices (TRY)",
    },
    ProviderInfo {
        id: tcmb::SOURCE,
        name: "TCMB",
        supplies: "Exchange rates from the Turkish central bank",
    },
    ProviderInfo {
        id: yahoo::SOURCE,
        name: "Yahoo Finance",
        supplies:
            "US and European stock and ETF prices; unofficial source that may change without notice",
    },
    ProviderInfo {
        id: twelvedata::SOURCE,
        name: "Twelve Data",
        supplies: "Stock and ETF prices with your free API key; the free plan covers US listings",
    },
    ProviderInfo {
        id: eurostat::SOURCE,
        name: "Eurostat",
        supplies: "Inflation indices for TRY (HICP Türkiye) and EUR (euro-area HICP)",
    },
    ProviderInfo {
        id: bls::SOURCE,
        name: "BLS",
        supplies: "US consumer price index (USD inflation) from the Bureau of Labor Statistics",
    },
    ProviderInfo {
        id: ons::SOURCE,
        name: "ONS",
        supplies: "UK consumer price index (GBP inflation) from the Office for National Statistics",
    },
    ProviderInfo {
        id: evds::SOURCE,
        name: "EVDS",
        supplies: "Turkish producer price index (Yİ-ÜFE) from the TCMB, for indexing costs in Turkish tax reports; needs your free API key",
    },
];

/// Providers that need an API key before they make any request.
pub const NEEDS_KEY: [&str; 2] = [twelvedata::SOURCE, evds::SOURCE];

pub fn api_key_setting(id: &str) -> String {
    format!("provider.{id}.api_key")
}

/// The saved API key of a provider, trimmed; `None` when not saved or empty.
pub fn api_key(store: &mizan_storage::Store, id: &str) -> Option<String> {
    store
        .setting(&api_key_setting(id))
        .ok()
        .flatten()
        .map(|k| k.trim().to_owned())
        .filter(|k| !k.is_empty())
}

/// Save (or with `None` or an empty key, clear) a provider's API key.
pub fn set_api_key(
    store: &mut mizan_storage::Store,
    id: &str,
    key: Option<&str>,
) -> Result<(), mizan_storage::StorageError> {
    if !NEEDS_KEY.contains(&id) {
        return Err(mizan_storage::StorageError::Invalid(format!(
            "provider {id:?} takes no API key"
        )));
    }
    let key = key.map(str::trim).unwrap_or("");
    if key.chars().any(char::is_whitespace) {
        return Err(mizan_storage::StorageError::Invalid(
            "an API key cannot contain spaces".into(),
        ));
    }
    store.set_setting(&api_key_setting(id), key)
}

/// The last four characters of a saved key, masked: for example "••••3f9a".
pub fn api_key_hint(store: &mizan_storage::Store, id: &str) -> Option<String> {
    api_key(store, id).map(|k| {
        let tail: String = k
            .chars()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("••••{tail}")
    })
}

pub fn enabled_key(id: &str) -> String {
    format!("provider.{id}.enabled")
}

/// Whether a provider is enabled. Missing or unreadable settings count as enabled.
pub fn is_enabled(store: &mizan_storage::Store, id: &str) -> bool {
    store
        .setting(&enabled_key(id))
        .ok()
        .flatten()
        .is_none_or(|v| v != "false")
}

pub fn set_enabled(
    store: &mut mizan_storage::Store,
    id: &str,
    enabled: bool,
) -> Result<(), mizan_storage::StorageError> {
    if !PROVIDERS.iter().any(|p| p.id == id) {
        return Err(mizan_storage::StorageError::Invalid(format!(
            "unknown provider {id:?}"
        )));
    }
    store.set_setting(&enabled_key(id), if enabled { "true" } else { "false" })
}

/// The base currency setting as text, `TRY` when unset.
pub fn base_currency_code(
    store: &mizan_storage::Store,
) -> Result<String, mizan_storage::StorageError> {
    Ok(store
        .setting("base_currency")?
        .map(|s| s.trim().to_uppercase())
        .filter(|s| s.len() == 3)
        .unwrap_or_else(|| "TRY".into()))
}
