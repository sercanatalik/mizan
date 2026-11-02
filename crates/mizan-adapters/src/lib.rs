//! Import adapters: turn statement files into `mizan.import/1` batches.
//!
//! Three kinds share one contract ([`Adapter`]): declarative mapping files,
//! sandboxed TypeScript scripts, and native Rust parsers. Adapters return
//! records without provenance; [`run::run_adapter`] adds it, so no adapter can
//! misreport which adapter, version or file produced a record.

pub mod detect;
pub mod input;
pub mod manifest;
pub mod mapping;
pub mod native;
pub mod registry;
pub mod run;
pub mod script;

pub use detect::{detect, Detection, Match};
pub use input::FileInput;
pub use manifest::{AdapterKind, FileKind, Manifest};
pub use registry::{Origin, Registry};
pub use run::{run_adapter, run_and_parse, AdapterError, RunOutput};

/// One import adapter.
pub trait Adapter: Send + Sync {
    fn manifest(&self) -> &Manifest;
    /// How well this adapter recognizes the file, 0 (not at all) to 100.
    fn detect(&self, file: &FileInput) -> u8;
    fn run(&self, file: &FileInput) -> Result<RunOutput, AdapterError>;
}
