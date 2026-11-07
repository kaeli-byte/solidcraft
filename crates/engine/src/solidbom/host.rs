//! The portable host seam lives in `solidbom-contract` (version 1); this module keeps the
//! engine's existing import paths working.
//!
//! A host submits work beside the window and answers on `poll`; it never touches a document and
//! never retries a mutation on its own. Ownership, bounded queues and cancellation are specified
//! in `docs/solidbom/contracts/v1/host.md`.

pub use solidbom_contract::host::*;
