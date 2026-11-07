//! Native HTTP transport for SolidBOM.
//!
//! Every request runs on its own worker thread and answers on a channel the UI polls; the window
//! thread never waits for the service. Endpoints and the development token come from
//! `SOLIDBOM_ENDPOINT` and `SOLIDBOM_TOKEN`; the token is never logged. There is no automatic
//! retry of a mutation: a failed request is retried only when the user asks.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use solidbom_contract::{
    CalculationSnapshot, CancelOutcome, ClientFailure, ClientReply, ClientRequest, HostCapabilities, HostError, HostHandle, JobCompletion, JobId,
    JobRequest, MAX_PENDING_JOBS, ProjectView, ReferenceData, SCHEMA_VERSION, SaveReceipt, SolidBomHost, SourceRef,
};
const TIMEOUT: Duration = Duration::from_secs(30);
const BODY_LIMIT: u64 = 128 * 1024 * 1024;
/// The service's configured upload limit; a resource limit, not a supported-file claim.
const MAX_UPLOAD_BYTES: u64 = 64 * 1024 * 1024;

/// The host the desktop installs, or none when the endpoint or token is not configured.
pub fn from_env() -> Option<HostHandle> {
    let endpoint = std::env::var("SOLIDBOM_ENDPOINT").ok()?;
    let token = std::env::var("SOLIDBOM_TOKEN").ok()?;
    let endpoint = endpoint.trim().trim_end_matches('/').to_string();
    if endpoint.is_empty() || token.len() < 16 {
        return None;
    }
    Some(Arc::new(HttpHost::new(endpoint, token)))
}

struct HttpHost {
    endpoint: String,
    token: String,
    agent: ureq::Agent,
    budget: Arc<Budget>,
    tx: Sender<JobCompletion>,
    rx: Mutex<Receiver<JobCompletion>>,
}

/// The bounded set of jobs this host has admitted. Submission never runs unbounded work: a
/// caller that keeps submitting past [`MAX_PENDING_JOBS`] gets [`HostError::QueueFull`].
struct Budget {
    next: AtomicU64,
    in_flight: Mutex<BTreeMap<JobId, Arc<AtomicBool>>>,
}

impl Budget {
    fn new() -> Self {
        Self { next: AtomicU64::new(0), in_flight: Mutex::new(BTreeMap::new()) }
    }

    fn admit(self: &Arc<Self>) -> std::result::Result<Admission, HostError> {
        let mut in_flight = self.in_flight.lock().unwrap_or_else(PoisonError::into_inner);
        if in_flight.len() >= usize::try_from(MAX_PENDING_JOBS).unwrap_or(usize::MAX) {
            return Err(HostError::QueueFull { capacity: MAX_PENDING_JOBS });
        }
        let job = self.next.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        in_flight.insert(job, cancelled.clone());
        Ok(Admission { budget: Arc::clone(self), job, cancelled })
    }

    /// Best effort: the worker checks the flag once before queueing its answer, so a request
    /// already on the wire may still deliver its real result.
    fn cancel(&self, job: JobId) -> CancelOutcome {
        let in_flight = self.in_flight.lock().unwrap_or_else(PoisonError::into_inner);
        match in_flight.get(&job) {
            Some(cancelled) => {
                cancelled.store(true, Ordering::SeqCst);
                CancelOutcome::Cancelled
            }
            None => CancelOutcome::Unknown,
        }
    }
}

struct Admission {
    budget: Arc<Budget>,
    job: JobId,
    cancelled: Arc<AtomicBool>,
}

impl Admission {
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

impl Drop for Admission {
    fn drop(&mut self) {
        let mut in_flight = self.budget.in_flight.lock().unwrap_or_else(PoisonError::into_inner);
        in_flight.remove(&self.job);
    }
}

impl HttpHost {
    fn new(endpoint: String, token: String) -> Self {
        let tls = ureq::tls::TlsConfig::builder()
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .build();
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .tls_config(tls)
            .https_only(false)
            .http_status_as_error(false)
            .max_redirects(0)
            .timeout_global(Some(TIMEOUT))
            .timeout_connect(Some(Duration::from_secs(5)))
            .user_agent("SolidCraft-SolidBOM")
            .build()
            .into();
        let (tx, rx) = channel();
        Self { endpoint, token, agent, budget: Arc::new(Budget::new()), tx, rx: Mutex::new(rx) }
    }
}

impl SolidBomHost for HttpHost {
    fn submit(&self, job: JobRequest) -> std::result::Result<JobId, HostError> {
        let admission = self.budget.admit()?;
        let job_id = admission.job;
        let endpoint = self.endpoint.clone();
        let token = self.token.clone();
        let agent = self.agent.clone();
        let tx = self.tx.clone();
        let envelope = job.envelope;
        let request = job.request;
        std::thread::Builder::new()
            .name("solidbom-http".into())
            .spawn(move || {
                let result = if admission.is_cancelled() {
                    Err(ClientFailure::Cancelled)
                } else {
                    let result = perform(&agent, &endpoint, &token, request);
                    if admission.is_cancelled() { Err(ClientFailure::Cancelled) } else { result }
                };
                let _ = tx.send(JobCompletion { job_id, envelope, result });
                // Dropping the admission frees the queue slot, including on an early return.
            })
            .map_err(|error| HostError::Transport(format!("the SolidBOM request thread did not start: {error}")))?;
        Ok(job_id)
    }

    fn capabilities(&self) -> HostCapabilities {
        HostCapabilities {
            service: true,
            contract_version: SCHEMA_VERSION,
            max_pending_jobs: MAX_PENDING_JOBS,
            cancellation: true,
            upload_bytes: Some(MAX_UPLOAD_BYTES),
        }
    }

    fn cancel(&self, job_id: JobId) -> CancelOutcome {
        self.budget.cancel(job_id)
    }

    fn poll(&self) -> Vec<JobCompletion> {
        self.rx.lock().map(|rx| rx.try_iter().collect()).unwrap_or_default()
    }
}

fn perform(agent: &ureq::Agent, endpoint: &str, token: &str, request: ClientRequest) -> std::result::Result<ClientReply, ClientFailure> {
    let auth = format!("Bearer {token}");
    match request {
        ClientRequest::UploadSource { sha256, original_name, bytes } => {
            let url = format!("{endpoint}/v1/sources/{sha256}?name={}", encode(&original_name));
            let response =
                agent.put(&url).header("Authorization", auth.as_str()).header("Content-Type", "application/step").send(bytes).map_err(network)?;
            parse::<SourceRef>(response).map(ClientReply::Source)
        }
        ClientRequest::CreateProject(save) => {
            let response = agent
                .post(&format!("{endpoint}/v1/projects"))
                .header("Authorization", auth.as_str())
                .header("Content-Type", "application/json")
                .send(json(&save))
                .map_err(network)?;
            parse::<SaveReceipt>(response).map(ClientReply::Receipt)
        }
        ClientRequest::SaveDraft(save) => {
            let url = format!("{endpoint}/v1/projects/{}/draft", save.draft.project_id);
            let response = agent
                .put(&url)
                .header("Authorization", auth.as_str())
                .header("Content-Type", "application/json")
                .send(json(&save))
                .map_err(network)?;
            parse::<SaveReceipt>(response).map(ClientReply::Receipt)
        }
        ClientRequest::ReferenceData => {
            let response = agent.get(&format!("{endpoint}/v1/reference-data")).header("Authorization", auth.as_str()).call().map_err(network)?;
            parse::<ReferenceData>(response).map(ClientReply::ReferenceData)
        }
        ClientRequest::LoadProject(project_id) => {
            let response =
                agent.get(&format!("{endpoint}/v1/projects/{project_id}")).header("Authorization", auth.as_str()).call().map_err(network)?;
            parse::<ProjectView>(response).map(ClientReply::Project)
        }
        ClientRequest::DownloadSource(source_id) => {
            let response = agent.get(&format!("{endpoint}/v1/sources/{source_id}")).header("Authorization", auth.as_str()).call().map_err(network)?;
            let status = response.status().as_u16();
            let sha256 = response.headers().get("x-solidbom-sha256").and_then(|value| value.to_str().ok()).map(str::to_string);
            let mut response = response;
            let bytes =
                response.body_mut().with_config().limit(BODY_LIMIT).read_to_vec().map_err(|error| ClientFailure::Network(error.to_string()))?;
            if status != 200 {
                return Err(failure(status, &bytes));
            }
            let sha256 = sha256.ok_or_else(|| ClientFailure::Server("the source reply has no digest".into()))?;
            let source = SourceRef {
                source_id,
                sha256,
                byte_len: u64::try_from(bytes.len()).map_err(|_| ClientFailure::Server("the source is too large".into()))?,
                original_name: "source.step".into(),
            };
            Ok(ClientReply::SourceBytes(source, bytes))
        }
        ClientRequest::Calculate(request) => {
            let url = format!("{endpoint}/v1/projects/{}/calculations", request.project_id);
            let response = agent
                .post(&url)
                .header("Authorization", auth.as_str())
                .header("Content-Type", "application/json")
                .send(json(&request))
                .map_err(network)?;
            parse::<CalculationSnapshot>(response).map(ClientReply::Calculation)
        }
        ClientRequest::LoadCalculation(calculation_id) => {
            let response =
                agent.get(&format!("{endpoint}/v1/calculations/{calculation_id}")).header("Authorization", auth.as_str()).call().map_err(network)?;
            parse::<CalculationSnapshot>(response).map(ClientReply::Calculation)
        }
    }
}

fn json<T: serde::Serialize>(value: &T) -> Vec<u8> {
    serde_json::to_vec(value).unwrap_or_default()
}

fn parse<T: serde::de::DeserializeOwned>(response: ureq::http::Response<ureq::Body>) -> std::result::Result<T, ClientFailure> {
    let status = response.status().as_u16();
    let mut response = response;
    let bytes = response.body_mut().with_config().limit(BODY_LIMIT).read_to_vec().map_err(|error| ClientFailure::Network(error.to_string()))?;
    if status != 200 && status != 201 {
        return Err(failure(status, &bytes));
    }
    serde_json::from_slice(&bytes).map_err(|_| ClientFailure::Server("the service's reply could not be read".into()))
}

fn network(error: ureq::Error) -> ClientFailure {
    ClientFailure::Network(error.to_string())
}

fn failure(status: u16, body: &[u8]) -> ClientFailure {
    let message = serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value["error"]["message"].as_str().map(str::to_string))
        .unwrap_or_else(|| format!("HTTP {status}"));
    match status {
        401 => ClientFailure::Unauthorized,
        404 => ClientFailure::NotFound,
        409 => ClientFailure::Conflict(message),
        413 => ClientFailure::Invalid("the source is larger than the service accepts".into()),
        422 => ClientFailure::Invalid(message),
        _ => ClientFailure::Server(message),
    }
}

/// Percent-encode a query value (the source file's name).
fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(*byte));
        } else {
            out.push('%');
            out.push_str(&format!("{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Budget, CancelOutcome, HostError, MAX_PENDING_JOBS, encode};
    use std::sync::Arc;

    #[test]
    fn a_file_name_is_percent_encoded_for_the_query() {
        assert_eq!(encode("three_boxes.step"), "three_boxes.step");
        assert_eq!(encode("a b+c.step"), "a%20b%2Bc.step");
    }

    #[test]
    fn a_host_admits_a_bounded_number_of_requests() {
        let budget = Arc::new(Budget::new());
        let mut admissions: Vec<_> = (0..MAX_PENDING_JOBS).filter_map(|_| budget.admit().ok()).collect();
        assert_eq!(admissions.len(), MAX_PENDING_JOBS as usize);
        assert!(matches!(budget.admit(), Err(HostError::QueueFull { capacity }) if capacity == MAX_PENDING_JOBS));
        admissions.pop();
        assert!(budget.admit().is_ok());
    }

    #[test]
    fn cancelling_an_admitted_job_marks_it_and_a_finished_job_is_unknown() {
        let budget = Arc::new(Budget::new());
        let Ok(admission) = budget.admit() else { return };
        let job = admission.job;
        assert!(!admission.is_cancelled());
        assert_eq!(budget.cancel(job), CancelOutcome::Cancelled);
        assert!(admission.is_cancelled());
        drop(admission);
        assert_eq!(budget.cancel(job), CancelOutcome::Unknown);
    }
}
