//! The reviewed-project controller a session carries.
//!
//! Jobs are submitted through the session's host and applied only to the record they were
//! submitted for. Applications sharing a host route its completions to their owning session;
//! standalone sessions can use [`Session::drain_solidbom`]. A save records its edit generation:
//! edits made meanwhile leave the project dirty after the older save commits. A calculate first
//! saves, then calculates the returned version; a late answer after an edit is stale, not current.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{Value, json};
use solidbom_contract::{
    AnalysisKey, AnalysisManifest, CalculationSnapshot, CalculationSummary, DefinitionRef, DraftProject, MeasurementSource, PartInputs,
    ReferenceData, ResultState as OutcomeState, SaveDraft, SourceRef, Sourcing, part_version_for,
};
use uuid::Uuid;

use super::extract::{EXTRACTOR_VERSION, RuntimeOccurrenceMap, analysis_matches, engine_build};
use super::host::{ClientFailure, ClientReply, ClientRequest, JobCompletion, JobEnvelope, JobId, JobRequest};
use super::jobs::{AnalysisJob, AnalysisPurpose, AnalysisRequest, PreparedAnalysis};
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
    /// The project, analysis and generations the request described; the host echoes it back.
    pub envelope: JobEnvelope,
    pub expected_version: Option<u64>,
}

/// A request that failed and may be retried with the same id and payload.
#[derive(Debug, Clone)]
pub struct Failed {
    pub kind: PendingKind,
    pub request: ClientRequest,
    /// False when sending the same request again cannot help: a conflict, an invalid request, or
    /// an unknown or unauthorized project needs a person to change something first.
    pub retryable: bool,
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
    /// Increments each time the session binds a reviewed source. A completion carrying an older
    /// value describes a previous binding and is stale.
    pub binding_generation: u64,
    pub next_request: u64,
    pub notice: Option<String>,
    pub error: Option<String>,
    pub needs_service: bool,
    /// The runtime-occurrence map of the bound import, rebuilt after every import or reopen.
    /// Only the stable source path or a contract id may be persisted; runtime CAD ids change.
    pub runtime: Option<RuntimeOccurrenceMap>,
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
            return Err(EngineError::Capability(SERVICE_MISSING.into()));
        };
        let capabilities = host.capabilities();
        capabilities.validate().map_err(|error| EngineError::Capability(error.to_string()))?;
        if !capabilities.service {
            self.solidbom.needs_service = true;
            return Err(EngineError::Capability(SERVICE_MISSING.into()));
        }
        if capabilities.contract_version != solidbom_contract::SCHEMA_VERSION {
            return Err(EngineError::Capability(format!(
                "the SolidBOM service speaks contract version {}; this build reads version {}",
                capabilities.contract_version,
                solidbom_contract::SCHEMA_VERSION
            )));
        }
        let envelope = JobEnvelope {
            request_id: request_id.clone(),
            project_id: request_project_id(&request).or_else(|| self.solidbom.project_id()),
            analysis_key: self.solidbom.draft.as_ref().map(|draft| draft.analysis.key.clone()),
            local_edit_generation: self.solidbom.generation,
            binding_generation: self.solidbom.binding_generation,
        };
        let stored = request.clone();
        let job = host.submit(JobRequest::new(envelope.clone(), request)).map_err(EngineError::from)?;
        // The configured service accepted the request: an earlier transient failure no longer
        // means the service is missing, and the command stays available for an explicit retry.
        self.solidbom.needs_service = false;
        self.solidbom.pending.insert(job, PendingJob { kind, request: stored, envelope, expected_version });
        Ok(job)
    }

    /// Cancel every request this session has in flight, including a background analysis. The
    /// session keeps its last saved state; a cancelled analysis publishes nothing.
    pub fn cancel_solidbom(&mut self) {
        let pending = std::mem::take(&mut self.solidbom.pending);
        let host = self.solidbom_host.clone();
        for (job, pending) in pending {
            if let Some(host) = &host {
                let _ = host.cancel(job);
            }
            self.solidbom_cancelled(pending.kind);
        }
        if let Some(job) = self.solidbom_analysis.take() {
            job.cancel();
            if self.review.is_none() {
                self.solidbom.phase = Phase::Idle;
            }
            if self.solidbom.draft.is_none() {
                self.solidbom.save = SaveState::Clean;
            }
            self.solidbom.error = None;
            self.solidbom.failed = None;
            self.solidbom.notice = Some("the SolidBOM analysis was cancelled".into());
        }
    }

    fn solidbom_cancelled(&mut self, kind: PendingKind) {
        match kind {
            PendingKind::Save { .. } | PendingKind::CreateProject => self.solidbom.save = SaveState::Dirty,
            PendingKind::Calculate => self.solidbom.result = ResultState::NoResult,
            PendingKind::LoadProject => self.solidbom.phase = Phase::Idle,
            _ => {}
        }
        self.solidbom.error = None;
        self.solidbom.failed = None;
        self.solidbom.notice = Some("the SolidBOM request was cancelled".into());
    }

    /// Take the background analysis' answer when it has one, then poll a host owned by this
    /// session and apply matching completions, without blocking.
    /// Applications sharing a host across sessions must poll once and route completions through
    /// [`Session::apply_solidbom_completion`] instead.
    pub fn drain_solidbom(&mut self) {
        self.drain_solidbom_analysis();
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
            return Err(EngineError::Other(if self.solidbom.phase == Phase::Importing {
                "the SolidBOM project is still opening".into()
            } else {
                "no SolidBOM project is open".into()
            }));
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
            return Err(EngineError::Other(if self.solidbom.phase == Phase::Importing {
                "the SolidBOM project is still opening".into()
            } else {
                "no SolidBOM project is open".into()
            }));
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
        let Some(failed) = self.solidbom.failed.clone() else {
            return Err(EngineError::Other("there is no failed SolidBOM request to retry".into()));
        };
        if !failed.retryable {
            return Err(EngineError::Other("this SolidBOM request cannot be retried; reopen the project or correct the inputs".into()));
        }
        self.solidbom.failed = None;
        let (request_id, expected_version) = request_identity(&failed.request);
        let previous_phase = self.solidbom.phase;
        self.solidbom.error = None;
        match &failed.kind {
            PendingKind::Save { .. } => {
                self.solidbom.save = SaveState::Saving { request_id: request_id.clone().unwrap_or_default() };
            }
            PendingKind::Calculate => {
                self.solidbom.result = ResultState::Calculating { request_id: request_id.clone().unwrap_or_default() };
            }
            // A retried reopen is in flight again; keep the reopen guard closed until it answers.
            PendingKind::LoadProject => self.solidbom.phase = Phase::Importing,
            _ => {}
        }
        let retry = failed.request.clone();
        if let Err(error) = self.submit_solidbom(failed.request, failed.kind.clone(), request_id, expected_version) {
            self.solidbom.phase = previous_phase;
            self.solidbom.failed = Some(Failed { kind: failed.kind, request: retry, retryable: failed.retryable });
            return Err(error);
        }
        Ok(())
    }

    /// Apply one completion after its host and document owner have been resolved.
    /// Shared hosts must be polled once by the application, rather than once per session.
    pub fn apply_solidbom_completion(&mut self, completion: JobCompletion) {
        let Some(stored) = self.solidbom.pending.get(&completion.job_id) else { return };
        if completion.envelope != stored.envelope {
            self.solidbom.error = Some("SolidBOM: a completion did not echo its request's envelope".into());
            return;
        }
        let Some(pending) = self.solidbom.pending.remove(&completion.job_id) else { return };
        if matches!(completion.result, Err(ClientFailure::Cancelled)) {
            self.solidbom_cancelled(pending.kind);
            return;
        }
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
                let stale = pending
                    .envelope
                    .stale_reason(
                        self.solidbom.draft.as_ref().map(|draft| &draft.analysis.key),
                        self.solidbom.generation,
                        self.solidbom.binding_generation,
                    )
                    .is_some();
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
                let stale = pending
                    .envelope
                    .stale_reason(
                        self.solidbom.draft.as_ref().map(|draft| &draft.analysis.key),
                        self.solidbom.generation,
                        self.solidbom.binding_generation,
                    )
                    .is_some();
                if let Some(draft) = &mut self.solidbom.draft {
                    draft.version = receipt.version;
                }
                self.solidbom.save = if stale { SaveState::Dirty } else { SaveState::Clean };
                self.solidbom.notice = Some(format!("SolidBOM project saved as version {}", receipt.version));
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
                let id = pending.envelope.request_id.clone().unwrap_or_default();
                self.solidbom.save = SaveState::SaveFailed { request_id: id, message: failure_message(&error) };
                self.failed(pending.kind, pending.request, error);
            }
            (PendingKind::Calculate, Ok(ClientReply::Calculation(snapshot))) => {
                let stale = pending
                    .envelope
                    .stale_reason(
                        self.solidbom.draft.as_ref().map(|draft| &draft.analysis.key),
                        self.solidbom.generation,
                        self.solidbom.binding_generation,
                    )
                    .is_some()
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
                // The saved project is loaded, but the design is not verified or bound yet: the
                // source download and its analysis run in the background.
                self.solidbom.phase = Phase::Importing;
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
                // Verify the digest and analyse the source beside the window; the design is bound
                // only when the analysis verifies against the loaded project.
                if let Err(error) = self.start_solidbom_reopen_analysis(source, bytes) {
                    self.solidbom.phase = Phase::Idle;
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

    /// Start the background analysis a first import publishes when it verifies its source.
    /// The session keeps its current design until the analysed source is published.
    pub(crate) fn start_solidbom_import_analysis(&mut self, request: AnalysisRequest) -> Result<()> {
        if self.review.is_some() {
            return Err(EngineError::Other("a reviewed source is already open in this session".into()));
        }
        if self.solidbom_analysis.is_some() || self.solidbom.phase == Phase::Importing {
            return Err(EngineError::Other("a SolidBOM source is already being opened".into()));
        }
        let job = AnalysisJob::start(request, self.replacements, self.solidbom.binding_generation, self.solidbom.generation)?;
        self.solidbom = State { phase: Phase::Importing, ..Default::default() };
        self.solidbom_analysis = Some(job);
        Ok(())
    }

    /// Start the background analysis a reopen verifies its loaded project against. The project
    /// stays loaded, but its design is not bound until the downloaded source verifies.
    pub(crate) fn start_solidbom_reopen_analysis(&mut self, source: SourceRef, bytes: Vec<u8>) -> Result<()> {
        if self.review.is_some() {
            return Err(EngineError::Other("close the current reviewed source before reopening another".into()));
        }
        if self.solidbom.draft.is_none() {
            return Err(EngineError::Other("no SolidBOM project is open".into()));
        }
        if self.solidbom_analysis.is_some() {
            return Err(EngineError::Other("a SolidBOM source is already being opened".into()));
        }
        let digest = hex(&bytes);
        if digest != source.sha256 {
            return Err(EngineError::Other("the downloaded source does not match its SHA-256 digest".into()));
        }
        let key = AnalysisKey { source_sha256: digest, engine_build: engine_build().to_string(), extractor_version: EXTRACTOR_VERSION.to_string() };
        let request = AnalysisRequest {
            bytes,
            original_name: source.original_name.clone(),
            key,
            purpose: AnalysisPurpose::Reopen { source },
            command_check: self.command_check(),
            validation: self.validation(),
            param_check: self.param_check,
            auto_project: self.auto_project,
        };
        let job = AnalysisJob::start(request, self.replacements, self.solidbom.binding_generation, self.solidbom.generation)?;
        self.solidbom.phase = Phase::Importing;
        self.solidbom_analysis = Some(job);
        Ok(())
    }

    /// Wait for a background local analysis to publish or fail. The window never calls this: it
    /// polls per frame. A headless embedder (the MCP server's own session, a CLI script) has no
    /// frame to poll, so it waits here and then sees the published review. `false`: the analysis
    /// did not finish within `timeout` (it is still running; the window's pending state returns).
    pub fn wait_solidbom_analysis(&mut self, timeout: std::time::Duration) -> bool {
        let began = std::time::Instant::now();
        while self.solidbom_analysis.is_some() {
            self.drain_solidbom_analysis();
            if self.solidbom_analysis.is_none() {
                return true;
            }
            if began.elapsed() >= timeout {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        true
    }

    /// Take the background analysis' answer when it has one. A worker that failed publishes
    /// nothing: the session keeps the design it already had and reports why.
    pub fn drain_solidbom_analysis(&mut self) {
        let Some(job) = self.solidbom_analysis.as_mut() else { return };
        let Some(answer) = job.take_answer() else { return };
        let Some(job) = self.solidbom_analysis.take() else { return };
        match answer {
            Ok(prepared) => self.adopt_solidbom_analysis(job, prepared),
            Err(message) => self.fail_solidbom_analysis(job, message),
        }
    }

    fn fail_solidbom_analysis(&mut self, job: AnalysisJob, message: String) {
        match job.purpose() {
            AnalysisPurpose::Import { .. } => {
                self.solidbom = State { error: Some(message), ..Default::default() };
            }
            AnalysisPurpose::Reopen { source } => {
                // The loaded project stays loaded; downloading and verifying the source again is
                // an explicit retry, never an automatic one.
                self.solidbom.phase = Phase::Idle;
                self.solidbom.error = Some(message);
                self.solidbom.failed =
                    Some(Failed { kind: PendingKind::DownloadSource, request: ClientRequest::DownloadSource(source.source_id), retryable: true });
            }
        }
    }

    /// Publish an answer only while it still describes the record that asked for it: the design
    /// must not have been replaced and the reviewed binding must not have moved since submission.
    fn adopt_solidbom_analysis(&mut self, job: AnalysisJob, prepared: PreparedAnalysis) {
        if self.review.is_some() {
            self.solidbom.phase = Phase::Idle;
            self.solidbom.error = Some("the SolidBOM analysis finished after another source was bound; open the source again".into());
            return;
        }
        if self.replacements != job.replacements() {
            self.solidbom.phase = Phase::Idle;
            self.solidbom.error = Some("the SolidBOM analysis finished after its design was replaced; open the source again".into());
            return;
        }
        if self.solidbom.binding_generation != job.binding_generation() {
            self.solidbom.phase = Phase::Idle;
            self.solidbom.error = Some("the SolidBOM analysis finished after the reviewed binding changed; open the source again".into());
            return;
        }
        if prepared.manifest.key != *job.key() {
            self.solidbom.phase = Phase::Idle;
            self.solidbom.error = Some("the SolidBOM analysis does not describe its source; open the source again".into());
            return;
        }
        match job.purpose().clone() {
            AnalysisPurpose::Import { source, source_path } => self.publish_solidbom_import(prepared, source, source_path),
            AnalysisPurpose::Reopen { source } => self.publish_solidbom_reopen(prepared, source, job.inputs_generation()),
        }
    }

    /// Fill an `ImportReply` from the review a headless wait published, or say why it did not.
    /// A headless embedder (the MCP server's own session, a CLI script) has no window frame to
    /// poll: it waits and then reports the published counts instead of the pending zeros, and a
    /// failed analysis is a visible error, never a reply that still claims to be importing.
    pub fn complete_headless_import_reply(&self, reply: &mut Value) -> std::result::Result<(), String> {
        let Some(object) = reply.as_object_mut() else { return Ok(()) };
        if object.get("state").and_then(Value::as_str) != Some("importing") {
            return Ok(());
        }
        let Some(draft) = &self.solidbom.draft else {
            return Err(self.solidbom.error.clone().unwrap_or_else(|| "the SolidBOM import did not publish a review".into()));
        };
        object.insert("state".into(), json!("reviewing"));
        object.insert("definitions".into(), json!(draft.analysis.definitions.len()));
        object.insert("occurrences".into(), json!(draft.analysis.occurrences.len()));
        object.insert("part_version_ids".into(), json!(draft.parts.iter().map(|part| part.part_version_id.to_string()).collect::<Vec<_>>()));
        object.insert("issues".into(), json!(draft.analysis.issues.iter().map(|issue| issue.code.clone()).collect::<Vec<_>>()));
        object.insert("needs_service".into(), json!(self.solidbom.needs_service));
        Ok(())
    }

    /// Publish a verified import. Everything that can refuse the source is checked on the
    /// scratch session first — the draft validates and the runtime map resolves every occurrence
    /// path — so a failure never binds the policy or replaces the live design.
    fn publish_solidbom_import(&mut self, prepared: PreparedAnalysis, source: SourceRef, source_path: PathBuf) {
        let PreparedAnalysis { session, manifest, bytes } = prepared;
        let draft = draft_from(&manifest, &source);
        if let Err(error) = draft.validate() {
            self.solidbom = State { error: Some(format!("the analysed source cannot be reviewed: {error}")), ..Default::default() };
            return;
        }
        let runtime = match RuntimeOccurrenceMap::new(&session) {
            Ok(runtime) => runtime,
            Err(error) => {
                self.solidbom = State { error: Some(format!("the analysed source has no usable occurrence mapping: {error}")), ..Default::default() };
                return;
            }
        };
        let key = manifest.key.clone();
        let sha256 = source.sha256.clone();
        let original_name = source.original_name.clone();
        // The import binds review mode atomically, after the source verified: the design, the
        // manifest, the verified mapping and the policy all arrive together.
        self.replace_design(session);
        let mut state = State { phase: Phase::Reviewing, save: SaveState::Dirty, draft: Some(draft), ..Default::default() };
        state.runtime = Some(runtime);
        self.solidbom = state;
        self.bind_review(ReviewPolicy::new(source, key).with_source_path(source_path));
        self.solidbom.binding_generation = self.solidbom.binding_generation.wrapping_add(1);
        let host_missing = self.solidbom_host.is_none();
        self.solidbom.needs_service = host_missing;
        if host_missing {
            return;
        }
        let request = ClientRequest::UploadSource { sha256, original_name, bytes };
        if let Err(error) = self.submit_solidbom(request, PendingKind::UploadSource, None, None) {
            self.solidbom.error = Some(error.to_string());
            self.solidbom.needs_service = true;
        }
    }

    /// Publish a verified reopen. The saved analysis is compared with the fresh extraction and
    /// the runtime mapping resolves every occurrence path before the design is replaced or the
    /// policy is bound; an older extractor or engine build is re-analysed in place on a draft
    /// copy that validates, so a failure leaves the live session as it was. Reviewed edits made
    /// while the project was opening keep it dirty.
    fn publish_solidbom_reopen(&mut self, prepared: PreparedAnalysis, source: SourceRef, inputs_generation: u64) {
        let PreparedAnalysis { session, manifest, bytes: _ } = prepared;
        let Some(mut draft) = self.solidbom.draft.clone() else {
            self.solidbom.phase = Phase::Idle;
            self.solidbom.error = Some("no SolidBOM project is open".into());
            return;
        };
        if draft.source.sha256 != source.sha256 {
            self.solidbom.phase = Phase::Idle;
            self.solidbom.error = Some("the reopened source does not describe the loaded project".into());
            return;
        }
        let runtime = match RuntimeOccurrenceMap::new(&session) {
            Ok(runtime) => runtime,
            Err(error) => {
                self.solidbom.phase = Phase::Idle;
                self.solidbom.error = Some(format!("the reopened source has no usable occurrence mapping: {error}"));
                return;
            }
        };
        let key = manifest.key.clone();
        let exact = analysis_matches(&draft.analysis, &key) && draft.analysis == manifest;
        if !exact && let Err(error) = rebind_analysis(&mut draft, manifest) {
            self.solidbom.phase = Phase::Idle;
            self.solidbom.error = Some(format!("the re-analysed source cannot be reviewed: {error}"));
            return;
        }
        self.replace_design(session);
        self.bind_review(ReviewPolicy::new(source, key));
        self.solidbom.binding_generation = self.solidbom.binding_generation.wrapping_add(1);
        self.solidbom.runtime = Some(runtime);
        self.solidbom.draft = Some(draft);
        self.solidbom.phase = Phase::Reviewing;
        self.solidbom.result = ResultState::NoResult;
        if exact {
            // The saved analysis still describes this exact source, engine build and extractor:
            // the restored mappings and reviewed inputs are the saved ones. An edit made while
            // the project was opening means the inputs moved, so the project stays dirty and no
            // saved result is shown as current.
            let edited = self.solidbom.generation != inputs_generation || self.solidbom.save != SaveState::Clean;
            self.solidbom.save = if edited { SaveState::Dirty } else { SaveState::Clean };
            self.solidbom.notice = Some(if edited {
                "the SolidBOM project was reopened, but its inputs changed while it opened; save them".into()
            } else {
                "the SolidBOM project was reopened and its source verified".into()
            });
            let _ = self.submit_solidbom(ClientRequest::ReferenceData, PendingKind::ReferenceData, None, None);
            if !edited
                && let Some(latest) = self.solidbom.summaries.iter().max_by_key(|summary| summary.saved_version).map(|summary| summary.calculation_id)
            {
                let _ = self.submit_solidbom(ClientRequest::LoadCalculation(latest), PendingKind::LoadCalculation, None, None);
            }
            return;
        }
        // A saved analysis from another source, engine or extractor version cannot be presented
        // as current. Keep the reviewed decisions, re-anchor them to the new analysis and ask for
        // a save; a later calculation appends a new result.
        self.solidbom.save = SaveState::Dirty;
        self.solidbom.result = ResultState::Stale;
        self.solidbom.notice = Some("the source was analysed again; check the updated measurements and save".into());
        let _ = self.submit_solidbom(ClientRequest::ReferenceData, PendingKind::ReferenceData, None, None);
    }
    fn failed(&mut self, kind: PendingKind, request: ClientRequest, error: ClientFailure) {
        // A failed reopen must not leave the session permanently in `Importing`; the reopen
        // availability guard would then refuse every later attempt.
        if matches!(kind, PendingKind::LoadProject | PendingKind::DownloadSource) && self.solidbom.phase == Phase::Importing {
            self.solidbom.phase = Phase::Idle;
        }
        self.solidbom.error = Some(failure_message(&error));
        let retryable = error.retryable();
        // A failed request describes its own failure; it does not make a configured service
        // "missing". Only a host without the capability reports that (see `submit_solidbom`).
        self.solidbom.failed = Some(Failed { kind, request, retryable });
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
        ClientFailure::Cancelled => "the SolidBOM request was cancelled".into(),
    }
}

/// The project a request names, before a draft exists.
fn request_project_id(request: &ClientRequest) -> Option<Uuid> {
    match request {
        ClientRequest::CreateProject(save) | ClientRequest::SaveDraft(save) => Some(save.draft.project_id),
        ClientRequest::LoadProject(project_id) => Some(*project_id),
        ClientRequest::Calculate(request) => Some(request.project_id),
        _ => None,
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

/// The project identity a source hash names: the same bytes always reopen the same project.
pub(crate) fn project_id(sha256: &str) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_OID, format!("{sha256}:project").as_bytes())
}

/// Rebuild a loaded project's analysis from a fresh extraction without losing the engineer's
/// decisions. A reviewed manual measurement stays; an extracted measurement is re-read because it
/// belongs to the analysis that produced it. The caller validates the result before publishing.
fn rebind_analysis(draft: &mut DraftProject, manifest: AnalysisManifest) -> Result<()> {
    let mut parts = Vec::with_capacity(manifest.definitions.len());
    for definition in &manifest.definitions {
        let id = part_version_for(manifest.key.clone(), definition.definition_id);
        let mut part = draft.parts.iter().find(|part| part.part_version_id == id).cloned().unwrap_or_else(|| default_part(&manifest.key, definition));
        if part.volume_mm3.source == MeasurementSource::Extracted {
            part.volume_mm3 = definition.volume_mm3.clone();
        } else {
            part.volume_mm3.analysis_key = manifest.key.clone();
        }
        parts.push(part);
    }
    draft.analysis = manifest;
    draft.parts = parts;
    draft.validate().map_err(|error| EngineError::Other(error.to_string()))
}

/// A part that has not been reviewed yet: the extracted measurement stays pending.
fn default_part(key: &AnalysisKey, definition: &DefinitionRef) -> PartInputs {
    PartInputs {
        part_version_id: part_version_for(key.clone(), definition.definition_id),
        definition_id: definition.definition_id,
        classification: String::new(),
        classification_confirmed: false,
        sourcing: Sourcing::Manufactured,
        volume_mm3: definition.volume_mm3.clone(),
        material_version_id: None,
        allowance_rate: None,
        operation: None,
    }
}

/// The draft a freshly analysed source starts as: no reviewed decision, version zero, dirty.
pub(crate) fn draft_from(manifest: &AnalysisManifest, source: &SourceRef) -> DraftProject {
    DraftProject {
        project_id: project_id(&source.sha256),
        product_revision_id: Uuid::new_v5(&Uuid::NAMESPACE_OID, format!("{}:revision", source.sha256).as_bytes()),
        version: 0,
        source: source.clone(),
        analysis: manifest.clone(),
        material_overhead_rate: None,
        parts: manifest.definitions.iter().map(|definition| default_part(&manifest.key, definition)).collect(),
        forecast_year: 2027,
        forecast_quantity: 1000,
        currency: "CNY".into(),
    }
}

/// The SHA-256 of the downloaded bytes, in the contract's lower-case hexadecimal form.
pub fn hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}
