//! Market data for Mizan: built-in providers (TEFAS fund prices, TCMB exchange
//! rates) and the refresh that stores their results.
//!
//! This is the only crate that touches the network, and only inside
//! [`refresh`]. Everything else reads stored prices and rates.

pub mod http;
pub mod refresh;
pub mod tcmb;
pub mod tefas;

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
pub const PROVIDERS: [ProviderInfo; 2] = [
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
];

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
