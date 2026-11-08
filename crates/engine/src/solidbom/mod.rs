//! SolidBOM's reviewed-source analysis: the imported STEP source, its definitions, occurrences
//! and the mapping from a tree or viewport pick to them. Exact geometry only; the manifest is
//! scoped to one source hash and extractor version.

pub mod extract;
pub mod host;
pub mod jobs;
pub mod policy;
pub mod state;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

pub use extract::{EXTRACTOR_VERSION, analysis_matches, extract, resolve_pick};
pub use host::{
    CancelOutcome, ClientFailure, ClientReply, ClientRequest, HostCapabilities, HostError, HostHandle, JobCompletion, JobEnvelope, JobId, JobRequest,
    JobStatus, MAX_PENDING_JOBS, SolidBomHost,
};
pub use jobs::{AnalysisJob, AnalysisPurpose, AnalysisRequest, PreparedAnalysis};
pub use policy::{READ_ONLY_REASON, ReviewPolicy};
pub use state::{Phase, ResultState as SolidBomResultState, SaveState, State as SolidBomState};
