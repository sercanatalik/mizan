//! Built-in native adapters.

use std::sync::Arc;

use crate::manifest::Manifest;
use crate::Adapter;

pub mod ibkr;
pub mod ofx;
pub mod xml;

pub fn build(manifest: Manifest) -> Result<Arc<dyn Adapter>, String> {
    match manifest.native.as_deref() {
        Some("ofx") => Ok(Arc::new(ofx::OfxAdapter { manifest })),
        Some("ibkr-flex") => Ok(Arc::new(ibkr::IbkrFlexAdapter { manifest })),
        other => Err(format!("unknown native adapter {other:?}")),
    }
}
