//! Canonical import schema `mizan.import/1`.
//!
//! Adapters emit a [`Batch`] of [`ImportRecord`]s, balance [`Assertion`]s and
//! [`Diagnostic`]s. [`parse_batch`] checks the schema version, then validates
//! every item on its own.

pub mod record;
pub mod schema;
pub mod types;
pub mod validate;

pub use record::*;
pub use types::{Code, Date, Dec, Money, Sha256Hex};
pub use validate::{
    parse_batch, parse_batch_value, validate_assertion, validate_record, BatchError, Indexed,
    ItemError, ItemKind, ParsedBatch,
};
