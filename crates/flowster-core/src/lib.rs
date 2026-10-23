//! `flowster-core`: the engine shared by the WASM worker, the CLI and the server.
//!
//! Holds the engine ↔ UI protocol (§5.4), the command bus (§5.5), the
//! derived-state determinism probe (§20.3), and for M1 the document model (§6),
//! inheritance (§6.3), transforms (§6.4) and geometry (§8.1).

#![forbid(unsafe_code)]

pub mod calc;
pub mod command;
pub mod determinism;
pub mod formula;
pub mod geom;
pub mod library;
pub mod model;
pub mod paper;
pub mod protocol;
pub mod resolve;
pub mod rng;
pub mod theme;
pub mod transform;
pub mod value;

pub use command::{ChangeSummary, Command, CommandError, CommandTarget};
