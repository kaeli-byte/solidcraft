//! SolidBOM's development service: the central persistence and calculation API (packages SB-03
//! and SB-09).
//!
//! It stores original STEP bytes by SHA-256, append-only project versions, the immutable
//! reference data, append-only calculation snapshots and the append-only shared-record version
//! chains, and serves the version 2 HTTP routes published under `docs/solidbom/contracts/`. Two
//! development permission sets protect the routes: Engineer for project work and Administrator for
//! shared company records. The server resolves material and rate versions itself and runs the pure
//! direct-cost calculator from `crates/solidbom-costing`; the desktop only displays the answer.
//!
//! Every write is keyed by `request_id` plus a canonical payload hash. An identical retry returns
//! the recorded receipt or snapshot; a reused request id with a different payload is a conflict.
//! A draft save checks the client's expected version under a row lock and appends one immutable
//! version in the same transaction as its receipt. A calculation names one persisted version and
//! keeps its resolved inputs, input hash and calculator version, so a later edit never rewrites it.
//! A shared-record publish checks the expected version under a per-record lock and appends the
//! version and its reference-change fact in one transaction; a newer version never edits an
//! existing project version or calculation snapshot.
//!
//! This is a separate Cargo workspace from the CAD application. It is not built for wasm and its
//! dependencies are not shipped in the desktop program. See `README.md` for the configuration,
//! migration/seed and isolated test-database commands.

pub mod api;
pub mod auth;
pub mod store;

pub use store::{Store, StoreError};

/// A store operation's result.
pub type Result<T> = std::result::Result<T, StoreError>;
