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
//! Project tooling settings and the tooling recovery evaluation (package SB-17) are a third
//! additive optional field of the project record plus a read-only route: the service resolves the
//! tooling definitions of the exact project record version a baseline snapshot names, the demand
//! that snapshot already resolved and the saved exchange rates into the portable tooling inputs,
//! and runs the pure tooling calculator. A tooling settings write appends one project record
//! version; the evaluation stores nothing. See `tooling.rs`.
//!
//! Activation readiness, coordinated activation and accepted baselines (package SB-19) are the
//! version 2 activation surface: a read-only readiness report with typed per-member blockers, one
//! database transaction that appends the project record version, the SB-16 costing snapshot and the
//! accepted baseline together, and the append-only accepted-baseline history with its full accepted
//! shared-definition set. See `activation.rs`.
//!
//! Dependency updates and adoption (package SB-20) are the version 2 update surface: a read-only
//! catalog of the newest published version of every shared record, read-only per-project and global
//! dependency impact reports, one transaction per explicit acknowledgement or adoption, and draft
//! publication that is explicit or triggered by a forecast/quotation-scope edit. See `adoption.rs`.
//!
//! Internal exports (package SB-21) are the version 2 export surface: one read-only route renders
//! the detailed Excel workbook or the summary PDF of one explicitly selected saved result (an
//! accepted baseline or the current draft calculation snapshot). The projection reads only the
//! records that result pinned, so the documents preserve the selected snapshot, label a draft,
//! incomplete or stale result, and list the missing inputs, unresolved findings and stale costs.
//! See `export.rs`.
//!
//! This is a separate Cargo workspace from the CAD application. It is not built for wasm and its
//! dependencies are not shipped in the desktop program. See `README.md` for the configuration,
//! migration/seed and isolated test-database commands.

mod activation;
mod adoption;
pub mod api;
pub mod auth;
mod commercial;
mod costing;
mod engineering;
mod export;
mod feasibility;
mod machine_rates;
mod manufacturing;
mod projects;
pub mod store;
mod tooling;

pub use store::{Store, StoreError};

/// A store operation's result.
pub type Result<T> = std::result::Result<T, StoreError>;
