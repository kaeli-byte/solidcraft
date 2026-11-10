//! SolidBOM's portable contract: the wire types the desktop app, the calculator and the service
//! share. It has no UI, file access or database code, and it builds for the browser too.
//!
//! Money, rates and measurements use [`rust_decimal::Decimal`], serialized as strings. A missing
//! value is `None`; an explicit zero is `Some(Decimal::ZERO)`. Geometry becomes a decimal once,
//! through [`decimal_from_f64`], and never again from a displayed rounding.
//!
//! Version 1 is published by package SB-01 and version 2 extends it with the project, delivery
//! and part-version domain (package SB-10); package SB-11 adds the reviewed engineering record and
//! flat grouping in [`engineering`]; package SB-13 adds the recognition and catalog suggestion
//! contract in [`recognition`]; package SB-15 adds the sourcing, routing and attached-cost
//! definitions in [`manufacturing`]. The registry and the per-contract rules are under
//! `docs/solidbom/contracts/`. The host seam lives in [`host`], reviewed-input projections in
//! [`review`] and the HTTP vocabulary in [`api`].
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod api;
pub mod engineering;
pub mod error;
pub mod feasibility;
pub mod host;
pub mod manufacturing;
pub mod projects;
pub mod recognition;
pub mod review;
pub mod shared_data;
pub mod types;

pub use api::{API_VERSION, ApiErrorBody, ApiErrorCode, ApiErrorDetail, CalculationReceipt, ProjectVersionRef};
pub use engineering::*;
pub use error::{ContractError, Result, check_sha256, decimal_from_f64, is_sha256};
pub use feasibility::*;
pub use host::{
    CancelOutcome, ClientFailure, ClientReply, ClientRequest, HostCapabilities, HostError, HostHandle, JobCompletion, JobEnvelope, JobId, JobRequest,
    JobStatus, MAX_PENDING_JOBS, SolidBomHost,
};
pub use manufacturing::*;
pub use projects::*;
pub use recognition::*;
pub use review::{Annotation, MaterialOverheadPolicy, OverheadBasis, QuantityExpansion, ReviewedClassification, StaleReason, part_version_for};
pub use shared_data::*;
pub use types::*;

/// The contract version this build reads and writes.
pub const CONTRACT_VERSION: u32 = types::SCHEMA_VERSION;
/// The schema version written into manifests and checked on the way in.
pub const SCHEMA_VERSION: u32 = types::SCHEMA_VERSION;
