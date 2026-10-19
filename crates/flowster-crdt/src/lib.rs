//! `flowster-crdt`: the persistent half of the document model (SPEC §6.9) as a CRDT.
//!
//! M0 contains the §20.3 schema implemented on both candidate libraries and the
//! measurement harness behind `docs/adr/0001-crdt-library.md`. The engine uses
//! the Loro store ([`loro_store`]); yrs ([`yrs_store`]) stays behind a feature as the fallback.

#![forbid(unsafe_code)]

pub mod schema;
pub mod spike;
pub mod store;
pub mod synth;

#[cfg(feature = "loro")]
pub mod document;
#[cfg(feature = "loro")]
pub mod loro_store;
#[cfg(feature = "loro")]
pub mod model_io;
#[cfg(feature = "yrs")]
pub mod yrs_store;

pub use store::{Digest, SchemaStore};
