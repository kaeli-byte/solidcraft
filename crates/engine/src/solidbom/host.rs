//! The portable boundary between the SolidBOM controller and the service transport.
//!
//! The engine owns the request and reply types; the native app implements the host with HTTP and
//! worker threads, and the browser or a test installs no host. A job is applied only to the
//! record it was submitted for; the host never retries a mutation on its own.

use std::sync::Arc;

use solidbom_contract::{CalculateRequest, CalculationSnapshot, ProjectView, ReferenceData, SaveDraft, SaveReceipt, SourceRef};
use uuid::Uuid;

/// What the host calls one submitted request.
pub type JobId = u64;

/// One request to the SolidBOM service.
#[derive(Debug, Clone)]
pub enum ClientRequest {
    UploadSource { sha256: String, original_name: String, bytes: Vec<u8> },
    CreateProject(SaveDraft),
    SaveDraft(SaveDraft),
    ReferenceData,
    LoadProject(Uuid),
    DownloadSource(Uuid),
    Calculate(CalculateRequest),
    LoadCalculation(Uuid),
}

/// The successful answer to one request.
#[derive(Debug, Clone, PartialEq)]
pub enum ClientReply {
    Source(SourceRef),
    SourceBytes(SourceRef, Vec<u8>),
    Receipt(SaveReceipt),
    ReferenceData(ReferenceData),
    Project(ProjectView),
    Calculation(CalculationSnapshot),
}

/// The classes of failure the controller treats differently: a conflict needs the user's
/// decision, a network failure is retried explicitly, and invalid input is shown as an issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientFailure {
    Network(String),
    Conflict(String),
    Invalid(String),
    NotFound,
    Unauthorized,
    Server(String),
}

/// A finished request. The job id is the one `submit` returned.
#[derive(Debug)]
pub struct JobCompletion {
    pub job_id: JobId,
    pub result: std::result::Result<ClientReply, ClientFailure>,
}

/// Starts work beside the window and answers on `poll`.
pub trait SolidBomHost: Send + Sync {
    fn submit(&self, request: ClientRequest) -> std::result::Result<JobId, String>;
    fn poll(&self) -> Vec<JobCompletion>;
}

/// The host a session holds, shared with its copies.
pub type HostHandle = Arc<dyn SolidBomHost>;
