//! Items the user must see or settle before an import is committed.

use mizan_import::{AccountHint, InstrumentHint};
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ReviewItem {
    /// No account is mapped to this statement identifier.
    UnresolvedAccount { hint: AccountHint },
    /// No asset matches this instrument.
    UnresolvedInstrument {
        key: Option<String>,
        hint: Box<InstrumentHint>,
    },
}
