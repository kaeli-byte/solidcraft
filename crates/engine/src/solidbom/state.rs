//! The reviewed-project controller a session carries.
//!
//! Jobs are submitted through the session's host and applied by [`Session::drain_solidbom`] only
//! to the record they were submitted for. A save records the edit generation it started with:
//! edits made meanwhile leave the project dirty after the older save commits. A calculate first
//! saves, then calculates the returned version; a late answer after an edit is stale, not current.

use std::collections::BTreeMap;

use solidbom_contract::{
    AnalysisKey, CalculationSnapshot, CalculationSummary, DraftProject, ReferenceData, ResultState as OutcomeState, SaveDraft, SourceRef,
};
use uuid::Uuid;

use super::extract::{EXTRACTOR_VERSION, analysis_matches, engine_build, extract};
use super::host::{ClientFailure, ClientReply, ClientRequest, JobCompletion, JobId};
use super::policy::ReviewPolicy;
use crate::{EngineError, Result, Session};

/// The service is not configured for this host (the browser, a test, or a missing environment).
pub const SERVICE_MISSING: &str = "the SolidBOM service is not configured";

/// Where the review is in its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Phase {
    #[default]
    Idle,
    Importing,
    Reviewing,
}

/// Whether the reviewed inputs are saved.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SaveState {
    #[default]
    Clean,
    Dirty,
    Saving {
        request_id: String,
    },
    SaveFailed {
        request_id: String,
        message: String,
    },
}

/// Whether the displayed cost reflects the current inputs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ResultState {
    #[default]
    NoResult,
    Calculating {
        request_id: String,
    },
    Current,
    Stale,
    Incomplete,
}

/// What a submitted job will do when it completes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingKind {
    UploadSource,
    CreateProject,
    ReferenceData,
    Save { calculate_after: bool },
    Calculate,
    LoadProject,
    DownloadSource,
    LoadCalculation,
}

/// One in-flight job, its exact request, and the record it belongs to.
#[derive(Debug, Clone)]
pub struct PendingJob {
    pub kind: PendingKind,
    pub request: ClientRequest,
    pub generation: u64,
    pub request_id: Option<String>,
    pub expected_version: Option<u64>,
}

/// A request that failed and may be retried with the same id and payload.
#[derive(Debug, Clone)]
pub struct Failed {
    pub kind: PendingKind,
    pub request: ClientRequest,
}

/// Everything the panel reads and every job it tracks.
#[derive(Debug, Clone, Default)]
pub struct State {
    pub phase: Phase,
    pub save: SaveState,
    pub result: ResultState,
    pub draft: Option<DraftProject>,
    pub reference: Option<ReferenceData>,
    pub summaries: Vec<CalculationSummary>,
    pub calculations: Vec<CalculationSnapshot>,
    pub current: Option<CalculationSnapshot>,
    pub pending: BTreeMap<JobId, PendingJob>,
    pub failed: Option<Failed>,
    /// Increments on every reviewed-input edit.
    pub generation: u64,
    pub next_request: u64,
    pub notice: Option<String>,
    pub error: Option<String>,
    pub needs_service: bool,
}

impl State {
    pub fn project_id(&self) -> Option<Uuid> {
        self.draft.as_ref().map(|draft| draft.project_id)
    }

    pub fn version(&self) -> u64 {
        self.draft.as_ref().map_or(0, |draft| draft.version)
    }

    pub fn issue_codes(&self) -> Vec<String> {
        self.current.as_ref().map(|snapshot| snapshot.issues.iter().map(|issue| issue.code.clone()).collect()).unwrap_or_default()
    }
}

impl Session {
    /// Submit a request through the installed host and remember what its answer belongs to.
    pub fn submit_solidbom(
        &mut self,
        request: ClientRequest,
        kind: PendingKind,
        request_id: Option<String>,
        expected_version: Option<u64>,
    ) -> Result<JobId> {
        let Some(host) = self.solidbom_host.clone() else {
            self.solidbom.needs_service = true;
            return Err(EngineError::Other(SERVICE_MISSING.into()));
        };
        let generation = self.solidbom.generation;
        let stored = request.clone();
        let job = host.submit(request).map_err(|error| EngineError::Other(format!("SolidBOM: {error}")))?;
        self.solidbom.pending.insert(job, PendingJob { kind, request: stored, generation, request_id, expected_version });
        Ok(job)
    }

    /// Take every finished job and apply only the ones that still match their record. The UI
    /// calls this each frame; it never blocks.
    pub fn drain_solidbom(&mut self) {
        let Some(host) = self.solidbom_host.clone() else { return };
        for completion in host.poll() {
            self.apply_solidbom_completion(completion);
        }
    }

    /// A new request id, unique per launch and stable for one explicit retry.
    pub fn next_solidbom_request_id(&mut self) -> String {
        let serial = self.solidbom.next_request;
        self.solidbom.next_request = serial.wrapping_add(1);
        format!("{}-solidbom-{serial}", self.document_id)
    }

    /// Save the current inputs, then calculate the version the save returns.
    pub fn solidbom_save(&mut self, calculate_after: bool) -> Result<()> {
        if self.solidbom.phase != Phase::Reviewing {
            return Err(EngineError::Other("no SolidBOM project is open".into()));
        }
        let Some(draft) = self.solidbom.draft.clone() else {
            return Err(EngineError::Other("no SolidBOM project is open".into()));
        };
        if draft.version == 0 {
            return Err(EngineError::Other("the SolidBOM project is still being created".into()));
        }
        if matches!(self.solidbom.save, SaveState::Saving { .. }) {
            return Err(EngineError::Other("a SolidBOM save is already in progress".into()));
        }
        let request_id = self.next_solidbom_request_id();
        let expected_version = draft.version;
        let save = SaveDraft { request_id: request_id.clone(), expected_version, draft };
        self.solidbom.error = None;
        self.solidbom.failed = None;
        self.solidbom.save = SaveState::Saving { request_id: request_id.clone() };
        if let Err(error) = self.submit_solidbom(
            ClientRequest::SaveDraft(save),
            PendingKind::Save { calculate_after },
            Some(request_id.clone()),
            Some(expected_version),
        ) {
            self.solidbom.save = SaveState::SaveFailed { request_id, message: error.to_string() };
            return Err(error);
        }
        Ok(())
    }

    /// Calculate the saved version. Current inputs are saved first.
    pub fn solidbom_calculate(&mut self) -> Result<()> {
        if self.solidbom.phase != Phase::Reviewing {
            return Err(EngineError::Other("no SolidBOM project is open".into()));
        }
        if !matches!(self.solidbom.save, SaveState::Clean) {
            return self.solidbom_save(true);
        }
        let Some(draft) = self.solidbom.draft.clone() else {
            return Err(EngineError::Other("no SolidBOM project is open".into()));
        };
        if draft.version == 0 {
            return Err(EngineError::Other("the SolidBOM project has not been saved".into()));
        }
        let request_id = self.next_solidbom_request_id();
        let request =
            solidbom_contract::CalculateRequest { request_id: request_id.clone(), project_id: draft.project_id, saved_version: draft.version };
        self.solidbom.error = None;
        self.solidbom.failed = None;
        self.solidbom.result = ResultState::Calculating { request_id: request_id.clone() };
        if let Err(error) = self.submit_solidbom(ClientRequest::Calculate(request), PendingKind::Calculate, Some(request_id), None) {
            self.solidbom.result = ResultState::NoResult;
            self.solidbom.error = Some(error.to_string());
            return Err(error);
        }
        Ok(())
    }

    /// Resubmit the exact failed request (same request id and payload).
    pub fn solidbom_retry(&mut self) -> Result<()> {
        let Some(failed) = self.solidbom.failed.take() else {
            return Err(EngineError::Other("there is no failed SolidBOM request to retry".into()));
        };
        let (request_id, expected_version) = request_identity(&failed.request);
        self.solidbom.error = None;
        match &failed.kind {
            PendingKind::Save { .. } => {
                self.solidbom.save = SaveState::Saving { request_id: request_id.clone().unwrap_or_default() };
            }
            PendingKind::Calculate => {
                self.solidbom.result = ResultState::Calculating { request_id: request_id.clone().unwrap_or_default() };
            }
            _ => {}
        }
        let retry = failed.request.clone();
        if let Err(error) = self.submit_solidbom(failed.request, failed.kind.clone(), request_id, expected_version) {
            self.solidbom.failed = Some(Failed { kind: failed.kind, request: retry });
            return Err(error);
        }
        Ok(())
    }

    fn apply_solidbom_completion(&mut self, completion: JobCompletion) {
        let Some(pending) = self.solidbom.pending.remove(&completion.job_id) else { return };
        match (pending.kind.clone(), completion.result) {
            (PendingKind::UploadSource, Ok(ClientReply::Source(source))) => {
                let request_id = self.next_solidbom_request_id();
                let Some(mut draft) = self.solidbom.draft.clone() else { return };
                draft.source = source;
                self.solidbom.draft = Some(draft.clone());
                let save = SaveDraft { request_id: request_id.clone(), expected_version: 0, draft };
                if let Err(error) = self.submit_solidbom(ClientRequest::CreateProject(save), PendingKind::CreateProject, Some(request_id), Some(0)) {
                    self.solidbom.error = Some(error.to_string());
                }
                let _ = self.submit_solidbom(ClientRequest::ReferenceData, PendingKind::ReferenceData, None, None);
            }
            (PendingKind::CreateProject, Ok(ClientReply::Receipt(receipt))) => {
                let stale = pending.generation != self.solidbom.generation;
                if let Some(draft) = &mut self.solidbom.draft {
                    draft.version = receipt.version;
                }
                self.solidbom.save = if stale { SaveState::Dirty } else { SaveState::Clean };
                self.solidbom.notice = Some(format!("SolidBOM project saved as version {}", receipt.version));
            }
            (PendingKind::ReferenceData, Ok(ClientReply::ReferenceData(data))) => {
                self.solidbom.reference = Some(data);
            }
            (PendingKind::Save { calculate_after }, Ok(ClientReply::Receipt(receipt))) => {
                let stale = pending.generation != self.solidbom.generation;
                if let Some(draft) = &mut self.solidbom.draft {
                    draft.version = receipt.version;
                }
                self.solidbom.save = if stale { SaveState::Dirty } else { SaveState::Clean };
                if calculate_after && !stale {
                    let request_id = self.next_solidbom_request_id();
                    let Some(draft) = self.solidbom.draft.clone() else { return };
                    let request = solidbom_contract::CalculateRequest {
                        request_id: request_id.clone(),
                        project_id: draft.project_id,
                        saved_version: receipt.version,
                    };
                    self.solidbom.result = ResultState::Calculating { request_id: request_id.clone() };
                    if let Err(error) = self.submit_solidbom(ClientRequest::Calculate(request), PendingKind::Calculate, Some(request_id), None) {
                        self.solidbom.error = Some(error.to_string());
                        self.solidbom.result = ResultState::NoResult;
                    }
                } else if stale {
                    self.solidbom.result = ResultState::Stale;
                }
            }
            (PendingKind::Save { .. }, Err(error)) => {
                let id = pending.request_id.clone().unwrap_or_default();
                self.solidbom.save = SaveState::SaveFailed { request_id: id, message: failure_message(&error) };
                self.failed(pending.kind, pending.request, error);
            }
            (PendingKind::Calculate, Ok(ClientReply::Calculation(snapshot))) => {
                let stale = pending.generation != self.solidbom.generation
                    || Some(snapshot.saved_version) != self.solidbom.draft.as_ref().map(|draft| draft.version);
                let incomplete = snapshot.outputs.state == OutcomeState::Incomplete;
                self.solidbom.result = if stale {
                    ResultState::Stale
                } else if incomplete {
                    ResultState::Incomplete
                } else {
                    ResultState::Current
                };
                self.solidbom.calculations.retain(|saved| saved.calculation_id != snapshot.calculation_id);
                self.solidbom.calculations.push(snapshot.clone());
                self.solidbom.current = Some(snapshot);
            }
            (PendingKind::Calculate, Err(error)) => {
                self.solidbom.result = ResultState::NoResult;
                self.failed(pending.kind, pending.request, error);
            }
            (PendingKind::LoadProject, Ok(ClientReply::Project(view))) => {
                self.solidbom.draft = Some(view.draft.clone());
                self.solidbom.summaries = view.calculations.clone();
                self.solidbom.calculations.clear();
                self.solidbom.current = None;
                self.solidbom.save = SaveState::Clean;
                self.solidbom.result = ResultState::NoResult;
                self.solidbom.phase = Phase::Reviewing;
                let _ = self.submit_solidbom(ClientRequest::DownloadSource(view.draft.source.source_id), PendingKind::DownloadSource, None, None);
            }
            (PendingKind::LoadCalculation, Ok(ClientReply::Calculation(snapshot))) => {
                self.solidbom.calculations.retain(|saved| saved.calculation_id != snapshot.calculation_id);
                self.solidbom.calculations.push(snapshot.clone());
                if self.solidbom.draft.as_ref().is_some_and(|draft| draft.version == snapshot.saved_version) {
                    self.solidbom.result =
                        if snapshot.outputs.state == OutcomeState::Incomplete { ResultState::Incomplete } else { ResultState::Current };
                    self.solidbom.current = Some(snapshot);
                }
            }
            (PendingKind::DownloadSource, Ok(ClientReply::SourceBytes(source, bytes))) => {
                if let Err(error) = self.restore_reviewed_source(source, bytes) {
                    self.solidbom.error = Some(error.to_string());
                }
            }
            (
                PendingKind::UploadSource
                | PendingKind::CreateProject
                | PendingKind::ReferenceData
                | PendingKind::LoadProject
                | PendingKind::DownloadSource
                | PendingKind::LoadCalculation,
                Err(error),
            ) => {
                self.failed(pending.kind, pending.request, error);
            }
            // A reply that does not match its pending kind is a host programming error: keep the
            // state usable and say so.
            (kind, _) => {
                self.solidbom.error = Some(format!("SolidBOM: an unexpected reply arrived for {kind:?}"));
            }
        }
    }

    /// Restore a reopened source into a fresh session, verify it against the saved analysis and
    /// bind the read-only policy.
    fn restore_reviewed_source(&mut self, source: SourceRef, bytes: Vec<u8>) -> Result<()> {
        let sha = hex(&bytes);
        if sha != source.sha256 {
            return Err(EngineError::Other("the downloaded source does not match its SHA-256 digest".into()));
        }
        if self.review.is_some() {
            return Err(EngineError::Other("close the current reviewed source before reopening another".into()));
        }
        let path = std::env::temp_dir().join(format!("solidbom-reopen-{}.step", Uuid::new_v5(&Uuid::NAMESPACE_OID, sha.as_bytes())));
        std::fs::write(&path, &bytes).map_err(|error| EngineError::Other(format!("the reopened source: {error}")))?;
        let opened = self.execute("doc.open", &serde_json::json!({"path": path.to_string_lossy()}));
        let _ = std::fs::remove_file(&path);
        opened.map_err(|error| EngineError::Other(format!("the reopened source: {error}")))?;
        self.path = None;
        // The scratch file has a generated name; keep the design name from the saved source.
        let design_name =
            self.solidbom.draft.as_ref().map(|draft| draft.source.original_name.clone()).unwrap_or_else(|| source.original_name.clone());
        let stem = std::path::Path::new(&design_name).file_stem().and_then(|stem| stem.to_str()).unwrap_or("reviewed source").to_string();
        self.doc_mut().name = stem;
        let key = AnalysisKey { source_sha256: sha, engine_build: engine_build().to_string(), extractor_version: EXTRACTOR_VERSION.to_string() };
        let _fresh = extract(self, key.clone())?;
        let Some(draft) = self.solidbom.draft.clone() else {
            return Err(EngineError::Other("no SolidBOM project is open".into()));
        };
        self.review = Some(std::sync::Arc::new(ReviewPolicy::new(source, key.clone())));
        self.solidbom.phase = Phase::Reviewing;
        if !analysis_matches(&draft.analysis, &key) {
            // The saved measurements describe a different source or extractor. Keep them for
            // reference but do not claim a current cost; the source is still read-only.
            self.solidbom.error = Some("the saved analysis was made by a different source or extractor; review the parts again".into());
            self.solidbom.result = ResultState::Incomplete;
            return Ok(());
        }
        self.solidbom.save = SaveState::Clean;
        let _ = self.submit_solidbom(ClientRequest::ReferenceData, PendingKind::ReferenceData, None, None);
        if let Some(latest) = self.solidbom.summaries.iter().max_by_key(|summary| summary.saved_version).map(|summary| summary.calculation_id) {
            let _ = self.submit_solidbom(ClientRequest::LoadCalculation(latest), PendingKind::LoadCalculation, None, None);
        }
        Ok(())
    }

    fn failed(&mut self, kind: PendingKind, request: ClientRequest, error: ClientFailure) {
        self.solidbom.error = Some(failure_message(&error));
        self.solidbom.failed = Some(Failed { kind, request });
        if matches!(error, ClientFailure::Network(_) | ClientFailure::Unauthorized) {
            self.solidbom.needs_service = true;
        }
    }
}

fn failure_message(error: &ClientFailure) -> String {
    match error {
        ClientFailure::Network(message) => format!("the SolidBOM service is unreachable: {message}"),
        ClientFailure::Conflict(message) => format!("the SolidBOM project changed: {message}"),
        ClientFailure::Invalid(message) => format!("the SolidBOM service refused the request: {message}"),
        ClientFailure::NotFound => "the SolidBOM project was not found".into(),
        ClientFailure::Unauthorized => "the SolidBOM service refused the development token".into(),
        ClientFailure::Server(message) => format!("the SolidBOM service failed: {message}"),
    }
}

fn request_identity(request: &ClientRequest) -> (Option<String>, Option<u64>) {
    match request {
        ClientRequest::CreateProject(save) => (Some(save.request_id.clone()), Some(0)),
        ClientRequest::SaveDraft(save) => (Some(save.request_id.clone()), Some(save.expected_version)),
        ClientRequest::Calculate(request) => (Some(request.request_id.clone()), None),
        _ => (None, None),
    }
}

/// The SHA-256 of the downloaded bytes, in the contract's lower-case hexadecimal form.
pub fn hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}
