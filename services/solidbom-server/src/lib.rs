//! SolidBOM's development service: the central persistence and calculation API (packages SB-03
//! and SB-09).
//!
//! It stores original STEP bytes by SHA-256, append-only project versions, the immutable
//! reference data, append-only calculation snapshots, the append-only shared-record version
//! chains and the append-only project record with its deliveries and part versions, and serves the
//! version 2 HTTP routes published under `docs/solidbom/contracts/`. Two development permission
//! sets protect the routes: Engineer for project work and Administrator for shared company
//! records. The server resolves material and rate versions itself and runs the pure direct-cost
//! calculator from `crates/solidbom-costing`; the desktop only displays the answer.
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
//! Engineering review and flat grouping (package SB-11) are an append-only record beside the
//! project record: reviewed component associations bound to one registered source/analysis, the
//! shared reviewed metadata of each project part version, bulk engineer confirmation and the
//! persisted confirmed-label metadata. See `engineering.rs`.
//!
//! Sourcing and manufacturing definitions (package SB-15) are a second append-only record beside
//! the project record: the explicit purchased/manufactured choice and material consumption of each
//! shared project part version, the reusable versioned routing templates, the applied routings and
//! their operations, and the attached non-CAD cost lines. See `manufacturing.rs`.
//!
//! Feasibility rules and justified exceptions (package SB-14) are an Administrator-maintained
//! append-only rule-table chain and an append-only project feasibility record that keeps each
//! recorded assessment's exact reviewed evidence and computed findings plus the current justified
//! exceptions. See `feasibility.rs`.
//!
//! Project costing settings and immutable baseline snapshots (package SB-16) are an additive
//! optional field of the project record and a second append-only record beside it: the service
//! resolves the exact project, engineering and manufacturing record versions and the published
//! reference data into the portable baseline inputs, runs the pure calculator and keeps the
//! answer, its input hash, calculator version and selected versions unchanged. See `costing.rs`.
//!
//! This is a separate Cargo workspace from the CAD application. It is not built for wasm and its
//! dependencies are not shipped in the desktop program. See `README.md` for the configuration,
//! migration/seed and isolated test-database commands.

pub mod api;
pub mod auth;
mod costing;
mod engineering;
mod feasibility;
mod manufacturing;
mod projects;
pub mod store;

pub use store::{Store, StoreError};

/// A store operation's result.
pub type Result<T> = std::result::Result<T, StoreError>;
