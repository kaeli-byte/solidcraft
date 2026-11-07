//! The portable host seam: how the desktop, a test and the browser submit work to the SolidBOM
//! service and receive completions, without any transport in this crate.
//!
//! A request carries a [`JobEnvelope`] with the project, analysis and local generations it
//! describes. The host echoes the envelope on the completion, so a caller can apply an answer
//! only to the record it was submitted for. The host never mutates a document and never retries a
//! mutation on its own; `poll` is the only wakeup path in version 1.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{ContractError, Result};
use crate::review::StaleReason;
use crate::types::{
    AnalysisKey, CalculateRequest, CalculationSnapshot, ProjectView, ReferenceData, SCHEMA_VERSION, SaveDraft, SaveReceipt, SourceRef,
};

/// What the host calls one submitted request. Hosts number jobs locally; it is not a business id.
pub type JobId = u64;

/// The most requests one host accepts before [`SolidBomHost::submit`] answers
/// [`HostError::QueueFull`]. A bound keeps a hostile or looping caller from spawning unbounded
/// worker threads.
pub const MAX_PENDING_JOBS: u32 = 32;

/// One request to the SolidBOM service.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ClientReply {
    Source(SourceRef),
    SourceBytes(SourceRef, Vec<u8>),
    Receipt(SaveReceipt),
    ReferenceData(ReferenceData),
    Project(ProjectView),
    Calculation(CalculationSnapshot),
}

/// The classes of failure the controller treats differently: a conflict needs a person's
/// decision, a network failure is retried explicitly, invalid input is shown as an issue, and a
/// cancelled request is the user's own decision.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClientFailure {
    Network(String),
    Conflict(String),
    Invalid(String),
    NotFound,
    Unauthorized,
    Server(String),
    Cancelled,
}

impl ClientFailure {
    /// Whether sending the same request again can succeed without a person changing anything.
    /// A conflict needs the project reopened, an invalid request needs new inputs, a missing or
    /// unauthorized project needs a different configuration and a cancelled request was the
    /// user's decision, so none of those are retried.
    pub fn retryable(&self) -> bool {
        matches!(self, Self::Network(_) | Self::Server(_))
    }
}

/// What a submitted request describes. The host echoes it on the completion; a caller applies an
/// answer only to the record that still matches. Optional identity fields are `None` for requests
/// submitted before a project exists (`ReferenceData`, `LoadProject`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct JobEnvelope {
    pub request_id: Option<String>,
    pub project_id: Option<Uuid>,
    pub analysis_key: Option<AnalysisKey>,
    /// The reviewed-input generation when the request was submitted.
    pub local_edit_generation: u64,
    /// The reviewed-source binding generation when the request was submitted.
    pub binding_generation: u64,
}

impl JobEnvelope {
    /// Why an answer to this request is stale for the record as it is now, most fundamental
    /// cause first. `None` means the answer still describes the current record.
    pub fn stale_reason(&self, current: Option<&AnalysisKey>, local_edit_generation: u64, binding_generation: u64) -> Option<StaleReason> {
        let Some(submitted) = &self.analysis_key else {
            return if self.binding_generation != binding_generation {
                Some(StaleReason::BindingChanged)
            } else if self.local_edit_generation != local_edit_generation {
                Some(StaleReason::InputsChanged)
            } else {
                None
            };
        };
        let Some(now) = current else { return Some(StaleReason::SourceChanged) };
        if now.source_sha256 != submitted.source_sha256 {
            Some(StaleReason::SourceChanged)
        } else if now.engine_build != submitted.engine_build {
            Some(StaleReason::EngineChanged)
        } else if now.extractor_version != submitted.extractor_version {
            Some(StaleReason::ExtractorChanged)
        } else if self.binding_generation != binding_generation {
            Some(StaleReason::BindingChanged)
        } else if self.local_edit_generation != local_edit_generation {
            Some(StaleReason::InputsChanged)
        } else {
            None
        }
    }
}

/// One submitted request and the record it describes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobRequest {
    pub envelope: JobEnvelope,
    pub request: ClientRequest,
}

impl JobRequest {
    /// Pair a request with the record it describes.
    pub fn new(envelope: JobEnvelope, request: ClientRequest) -> Self {
        Self { envelope, request }
    }
}

/// A finished request. The job id is the one `submit` returned; the envelope is the one the
/// request carried.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobCompletion {
    pub job_id: JobId,
    pub envelope: JobEnvelope,
    pub result: std::result::Result<ClientReply, ClientFailure>,
}

/// The lifecycle vocabulary a transport may report. Version 1 only reports completions through
/// [`SolidBomHost::poll`]; this enum exists so a later transport can name the same states.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Running,
    Finished,
    Cancelled,
    Unknown,
}

/// What a host can do. A missing upload limit is `None`, never zero; zero would mean the host
/// accepts no uploads.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostCapabilities {
    pub service: bool,
    pub contract_version: u32,
    pub max_pending_jobs: u32,
    pub cancellation: bool,
    pub upload_bytes: Option<u64>,
}

impl Default for HostCapabilities {
    fn default() -> Self {
        Self { service: true, contract_version: SCHEMA_VERSION, max_pending_jobs: MAX_PENDING_JOBS, cancellation: false, upload_bytes: None }
    }
}

impl HostCapabilities {
    /// A host with no SolidBOM service (the browser, a test, a missing configuration).
    /// A host with no SolidBOM service.
    pub fn none() -> Self {
        Self { service: false, ..Self::default() }
    }

    /// Refuse a capability record that cannot describe a working host.
    pub fn validate(&self) -> Result<()> {
        if self.contract_version == 0 {
            return Err(ContractError::invalid("host.contract_version", "a host must name its contract version"));
        }
        if self.max_pending_jobs == 0 {
            return Err(ContractError::invalid("host.max_pending_jobs", "a host must accept at least one request"));
        }
        Ok(())
    }
}

/// Why a host could not accept a request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum HostError {
    /// The host or build lacks the capability (no service, wrong contract version).
    #[error("the SolidBOM host lacks the {0} capability")]
    Capability(String),
    /// The host already has its bounded number of requests in flight.
    #[error("the SolidBOM host already has {capacity} requests in flight")]
    QueueFull { capacity: u32 },
    /// The transport could not start or reach the service.
    #[error("the SolidBOM host transport failed: {0}")]
    Transport(String),
}

/// What happened when a caller asked to cancel a job.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CancelOutcome {
    /// The host accepted the cancellation, or the job had already finished without its answer
    /// being delivered.
    Cancelled,
    /// The host knows no job with that id (it finished, or was never submitted here).
    Unknown,
    /// This host cannot cancel a submitted request.
    Unsupported,
}

/// Starts work beside the window and answers on `poll`.
pub trait SolidBomHost: Send + Sync {
    /// Submit one request. A host admits at most [`HostCapabilities::max_pending_jobs`] at a
    /// time and never blocks the caller for the duration of the work.
    fn submit(&self, job: JobRequest) -> std::result::Result<JobId, HostError>;

    /// Take every finished request without blocking. Polling is the version 1 wakeup path: the
    /// app polls once per frame (or on its repaint timer) and routes each completion to the
    /// session that submitted it.
    fn poll(&self) -> Vec<JobCompletion>;

    /// Best-effort cancellation. A transport that cannot interrupt in-flight work may deliver
    /// the real answer instead; a host that cannot cancel at all returns
    /// [`CancelOutcome::Unsupported`].
    fn cancel(&self, _job_id: JobId) -> CancelOutcome {
        CancelOutcome::Unsupported
    }

    /// What this host can do. The default describes a working host at this contract version.
    fn capabilities(&self) -> HostCapabilities {
        HostCapabilities::default()
    }
}

/// The host a session holds, shared with its copies and its application.
pub type HostHandle = Arc<dyn SolidBomHost>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DefinitionRef;

    fn key(engine: &str, extractor: &str) -> AnalysisKey {
        AnalysisKey { source_sha256: "a".repeat(64), engine_build: engine.into(), extractor_version: extractor.into() }
    }

    fn envelope() -> JobEnvelope {
        JobEnvelope {
            request_id: Some("request-1".into()),
            project_id: Some(Uuid::nil()),
            analysis_key: Some(key("0.40.0", "1")),
            local_edit_generation: 3,
            binding_generation: 1,
        }
    }

    #[test]
    fn an_envelope_keeps_its_request_project_and_generations_through_json() {
        let text = serde_json::to_string(&envelope()).unwrap_or_default();
        assert!(text.contains("request-1"));
        assert_eq!(serde_json::from_str::<JobEnvelope>(&text).ok(), Some(envelope()));
    }

    #[test]
    fn staleness_names_the_most_fundamental_change_first() {
        let submitted = envelope();
        let current = key("0.40.0", "1");
        assert_eq!(submitted.stale_reason(Some(&current), 3, 1), None);
        assert_eq!(submitted.stale_reason(Some(&current), 4, 1), Some(StaleReason::InputsChanged));
        assert_eq!(submitted.stale_reason(Some(&current), 3, 2), Some(StaleReason::BindingChanged));
        assert_eq!(submitted.stale_reason(Some(&key("0.41.0", "1")), 3, 1), Some(StaleReason::EngineChanged));
        assert_eq!(submitted.stale_reason(Some(&key("0.40.0", "2")), 3, 1), Some(StaleReason::ExtractorChanged));
        assert_eq!(submitted.stale_reason(Some(&key("0.40.0", "1")), 3, 2), Some(StaleReason::BindingChanged));
        assert_eq!(submitted.stale_reason(None, 3, 1), Some(StaleReason::SourceChanged));
    }

    #[test]
    fn an_envelope_without_an_analysis_still_checks_its_generations() {
        let mut submitted = envelope();
        submitted.analysis_key = None;
        assert_eq!(submitted.stale_reason(None, 3, 1), None);
        assert_eq!(submitted.stale_reason(None, 4, 1), Some(StaleReason::InputsChanged));
        assert_eq!(submitted.stale_reason(None, 3, 2), Some(StaleReason::BindingChanged));
    }

    #[test]
    fn a_capability_default_is_a_working_host_and_none_is_absent() {
        let working = HostCapabilities::default();
        assert!(working.service);
        assert_eq!(working.contract_version, SCHEMA_VERSION);
        assert_eq!(working.max_pending_jobs, MAX_PENDING_JOBS);
        assert_eq!(working.upload_bytes, None);
        assert!(working.validate().is_ok());
        assert!(!HostCapabilities::none().service);
        assert!(HostCapabilities { max_pending_jobs: 0, ..working }.validate().is_err());
        assert!(HostCapabilities { contract_version: 0, ..working }.validate().is_err());
    }

    #[test]
    fn a_cancelled_request_is_not_retryable() {
        assert!(!ClientFailure::Cancelled.retryable());
        assert!(ClientFailure::Network("offline".into()).retryable());
        assert!(ClientFailure::Server("500".into()).retryable());
        assert!(!ClientFailure::Conflict("stale".into()).retryable());
    }

    #[test]
    fn a_queue_full_error_names_its_bound() {
        let error = HostError::QueueFull { capacity: MAX_PENDING_JOBS };
        assert!(error.to_string().contains("32"));
    }

    #[test]
    fn a_completion_round_trips_with_its_envelope() {
        let completion = JobCompletion { job_id: 7, envelope: envelope(), result: Err(ClientFailure::Cancelled) };
        let text = serde_json::to_string(&completion).unwrap_or_default();
        assert_eq!(serde_json::from_str::<JobCompletion>(&text).ok(), Some(completion));
    }

    #[test]
    fn a_job_request_round_trips_a_save_with_decimal_strings() {
        let source = SourceRef { source_id: Uuid::nil(), sha256: "a".repeat(64), byte_len: 10, original_name: "three_boxes.step".into() };
        let manifest = crate::types::AnalysisManifest {
            schema_version: SCHEMA_VERSION,
            key: key("0.40.0", "1"),
            definitions: vec![DefinitionRef {
                definition_id: Uuid::nil(),
                source_key: key("0.40.0", "1"),
                customer_part_number: None,
                customer_revision: None,
                volume_mm3: crate::types::Measurement {
                    value: Some("2400".parse().unwrap_or_default()),
                    unit: crate::types::Unit::MillimetreCubed,
                    source: crate::types::MeasurementSource::Extracted,
                    review: crate::types::ReviewState::Pending,
                    reason: None,
                    analysis_key: key("0.40.0", "1"),
                },
            }],
            occurrences: Vec::new(),
            geometry_bindings: Vec::new(),
            issues: Vec::new(),
        };
        let draft = crate::types::DraftProject {
            project_id: Uuid::nil(),
            product_revision_id: Uuid::nil(),
            version: 0,
            source,
            analysis: manifest,
            material_overhead_rate: Some("0.05".parse().unwrap_or_default()),
            parts: Vec::new(),
            forecast_year: 2027,
            forecast_quantity: 1,
            currency: "CNY".into(),
        };
        let save = SaveDraft { request_id: "request-1".into(), expected_version: 0, draft };
        let job = JobRequest::new(envelope(), ClientRequest::CreateProject(save));
        let text = serde_json::to_string(&job).unwrap_or_default();
        assert!(text.contains("\"0.05\""));
        assert_eq!(serde_json::from_str::<JobRequest>(&text).ok(), Some(job));
    }

    #[test]
    fn a_capability_error_names_what_is_missing() {
        assert!(HostError::Capability("service".into()).to_string().contains("service"));
        assert!(HostCapabilities::none().validate().is_ok());
    }
}
