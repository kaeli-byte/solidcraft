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
        Self::with_timeout(endpoint, token, TIMEOUT)
    }

    /// The same host with an explicit request bound: the controlled-channel tests use a short
    /// one so a stalled service fails visibly without a real 30-second wait.
    fn with_timeout(endpoint: String, token: String, timeout: Duration) -> Self {
        let tls = ureq::tls::TlsConfig::builder()
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .build();
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .tls_config(tls)
            .https_only(false)
            .http_status_as_error(false)
            .max_redirects(0)
            .timeout_global(Some(timeout))
            .timeout_connect(Some(Duration::from_secs(5).min(timeout)))
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
        400 => ClientFailure::Invalid("the service did not accept the JSON body".into()),
        401 => ClientFailure::Unauthorized,
        404 => ClientFailure::NotFound,
        409 => ClientFailure::Conflict(message),
        413 => ClientFailure::Invalid("the source is larger than the service accepts".into()),
        415 => ClientFailure::Invalid("the service did not accept the body type".into()),
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
    use super::{Budget, CancelOutcome, ClientFailure, HostError, MAX_PENDING_JOBS, encode, failure};
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
    fn body_and_extractor_rejections_are_invalid_and_not_retryable() {
        for status in [400, 413, 415, 422] {
            let error = failure(status, b"plain text");
            assert!(matches!(error, ClientFailure::Invalid(_)), "{status}: {error:?}");
            assert!(!error.retryable(), "{status} must not offer a retry");
        }
        assert!(matches!(failure(500, b""), ClientFailure::Server(_)));
        assert!(matches!(failure(409, b"{\"error\":{\"message\":\"moved\"}}"), ClientFailure::Conflict(_)));
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

#[cfg(test)]
mod service_tests {
    //! A real development service, driven from the desktop side: the controller imports a
    //! checked-in STEP source, saves and calculates it, refuses a stale save, reopens the project
    //! in a fresh session and verifies the restored mapping, reviewed inputs and saved result.
    //!
    //! The test runs only when `SOLIDBOM_TEST_ENDPOINT` and `SOLIDBOM_TEST_TOKEN` name a fresh
    //! development database; without them it reports that it skipped. It never mints or commits a
    //! token.

    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use serde_json::json;
    use solidbom_contract::{ClientFailure, ClientRequest, DraftProject, JobEnvelope, JobRequest, SaveDraft};
    use solidcraft_engine::solidbom::extract::RuntimeOccurrenceMap;
    use solidcraft_engine::solidbom::state::{ResultState, SaveState, hex};
    use solidcraft_engine::{EngineError, Session};

    use super::HttpHost;

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/solidbom/fixtures/three_boxes.step")
    }

    /// An exact decimal of the type the compared field uses (the app crate does not depend on
    /// the decimal library directly).
    fn decimal<T: std::str::FromStr>(text: &str) -> T
    where
        T::Err: std::fmt::Debug,
    {
        text.parse().expect("an exact decimal")
    }

    /// The development service from the environment, or `None` when this machine has none.
    fn test_service() -> Option<solidcraft_engine::solidbom::HostHandle> {
        let endpoint = std::env::var("SOLIDBOM_TEST_ENDPOINT").ok().filter(|value| !value.trim().is_empty())?;
        let token = std::env::var("SOLIDBOM_TEST_TOKEN").ok().filter(|value| !value.trim().is_empty())?;
        Some(Arc::new(HttpHost::new(endpoint, token)))
    }

    /// Take every finished request and analysis until `done`. Polling is the only wakeup path, so
    /// the test drives the same loop the window does, with a bound instead of a fixed sleep.
    fn pump(session: &mut Session, what: &str, done: impl Fn(&Session) -> bool) {
        let began = Instant::now();
        loop {
            session.drain_solidbom();
            if done(session) {
                return;
            }
            assert!(
                began.elapsed() < Duration::from_secs(120),
                "timed out waiting for {what}: phase {:?}, save {:?}, result {:?}, error {:?}",
                session.solidbom.phase,
                session.solidbom.save,
                session.solidbom.result,
                session.solidbom.error
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// The review state after an import has created its project and loaded the reference data.
    fn imported(session: &Session) -> bool {
        session.review.is_some()
            && session.solidbom.draft.is_some()
            && session.solidbom.version() >= 1
            && session.solidbom.save == SaveState::Clean
            && session.solidbom.reference.is_some()
            && session.solidbom.pending.is_empty()
    }

    #[test]
    fn a_real_service_round_trip_recovers_the_reviewed_project() {
        let Some(host) = test_service() else {
            eprintln!("skipped: set SOLIDBOM_TEST_ENDPOINT and SOLIDBOM_TEST_TOKEN to a fresh development database");
            return;
        };
        let path = fixture();
        let bytes = std::fs::read(&path).expect("the fixture bytes");
        let sha = hex(&bytes);

        // Import: the local analysis runs beside the window and publishes the reviewed source.
        let mut session = Session::default();
        session.solidbom_host = Some(host.clone());
        let reply = session.execute("solidbom.import", &json!({"path": path.to_string_lossy()})).expect("the import starts");
        assert_eq!(reply["state"], json!("importing"));
        pump(&mut session, "the imported analysis", |session| session.review.is_some());
        pump(&mut session, "the created project", imported);
        let project_id = session.solidbom.draft.as_ref().expect("the draft").project_id;
        let source_sha = session.solidbom.draft.as_ref().expect("the draft").source.sha256.clone();
        assert_eq!(source_sha, sha, "the verified source is the imported file");
        let reference = session.solidbom.reference.clone().expect("reference data");
        let material = reference.materials[0].material_version_id;
        let rate = reference.rates[0].rate_version_id;
        let part = session.solidbom.draft.as_ref().expect("the draft").parts[0].part_version_id;
        assert!(session.solidbom.runtime.is_some(), "the import has a rebuilt runtime map");

        // Review the fixture and calculate a saved version through the real service.
        session
            .execute(
                "solidbom.inputs",
                &json!({
                    "part_version_id": part.to_string(),
                    "classification": "clip",
                    "classification_confirmed": true,
                    "volume_confirmed": true,
                    "material_version_id": material.to_string(),
                    "allowance_rate": "0.02",
                    "cycle_seconds": "30",
                    "cavities": 1,
                    "production_rate_version_id": rate.to_string(),
                    "setup_hours": "0.5",
                    "setup_rate_version_id": rate.to_string(),
                    "material_overhead_rate": "0.05"
                }),
            )
            .expect("the reviewed inputs");
        session.execute("solidbom.calculate", &json!({})).expect("calculate saves first");
        pump(&mut session, "the first calculation", |session| session.solidbom.result == ResultState::Current);
        let first = session.solidbom.current.clone().expect("the current calculation");
        assert_eq!(first.saved_version, 2);
        assert_eq!(first.outputs.state, solidbom_contract::ResultState::Complete);
        assert_eq!(first.outputs.assembly_subtotal, Some(decimal("3.4164048")));
        assert_eq!(first.outputs.setup_total, Some(decimal("60")));
        assert_eq!(first.outputs.parts[0].per_part_subtotal, Some(decimal("1.1388016")));

        // P07: a changed cycle is a new result; the earlier calculation stays readable.
        session.execute("solidbom.inputs", &json!({"part_version_id": part.to_string(), "cycle_seconds": "60"})).expect("the edit");
        session.execute("solidbom.calculate", &json!({})).expect("recalculate");
        pump(&mut session, "the recalculated version", |session| session.solidbom.result == ResultState::Current && session.solidbom.version() == 3);
        let second = session.solidbom.current.clone().expect("the new current calculation");
        assert_eq!(second.outputs.parts[0].per_part_subtotal, Some(decimal("2.1388016")));
        assert_eq!(second.outputs.assembly_subtotal, Some(decimal("6.4164048")));
        assert!(
            session.solidbom.calculations.iter().any(|snapshot| snapshot.calculation_id == first.calculation_id),
            "the earlier result remains readable"
        );

        // A stale expected version is refused, is not retryable, and never appears saved.
        let draft = session.solidbom.draft.clone().expect("the draft");
        let stale = SaveDraft { request_id: "sb06-e2e-stale-save".into(), expected_version: 1, draft };
        session
            .submit_solidbom(
                ClientRequest::SaveDraft(stale),
                solidcraft_engine::solidbom::state::PendingKind::Save { calculate_after: false },
                Some("sb06-e2e-stale-save".into()),
                Some(1),
            )
            .expect("the stale save is submitted");
        pump(&mut session, "the refused save", |session| session.solidbom.failed.is_some());
        assert!(matches!(session.solidbom.save, SaveState::SaveFailed { .. }), "a refused write is not saved");
        assert_eq!(session.solidbom.version(), 3, "a refused write moves no version");
        assert!(!session.solidbom.failed.as_ref().expect("the failure").retryable, "a conflict waits for a person");
        let kept = session.solidbom.draft.as_ref().expect("the draft").parts[0].operation.as_ref().expect("operation");
        assert_eq!(kept.cycle_seconds, Some(decimal("60")), "the unsaved edit is retained");

        // Reopen in a fresh session: source hash, mapping, reviewed inputs and the saved result.
        let mut reopened = Session::default();
        reopened.solidbom_host = Some(host);
        reopened.execute("solidbom.reopen", &json!({"project_id": project_id.to_string()})).expect("the reopen starts");
        pump(&mut reopened, "the verified reopen", |session| {
            session.review.is_some() && session.solidbom.reference.is_some() && session.solidbom.result == ResultState::Current
        });
        let restored = reopened.solidbom.draft.clone().expect("the reopened draft");
        assert_eq!(restored.project_id, project_id);
        assert_eq!(restored.source.sha256, source_sha, "the reopened source verifies its digest");
        assert_eq!(restored.analysis, session.solidbom.draft.as_ref().expect("the saved draft").analysis, "the verified mapping is the saved one");
        let reviewed = &restored.parts[0];
        assert!(reviewed.classification_confirmed);
        assert_eq!(reviewed.classification, "clip");
        assert_eq!(reviewed.material_version_id, Some(material));
        assert_eq!(reviewed.allowance_rate, Some(decimal("0.02")));
        assert_eq!(reviewed.operation.as_ref().and_then(|operation| operation.cycle_seconds), Some(decimal("60")));
        let current = reopened.solidbom.current.as_ref().expect("the saved result is shown");
        assert_eq!(current.calculation_id, second.calculation_id);
        assert_eq!(current.outputs.assembly_subtotal, Some(decimal("6.4164048")));
        assert!(
            reopened.solidbom.summaries.iter().any(|summary| summary.calculation_id == first.calculation_id)
                && reopened.solidbom.summaries.iter().any(|summary| summary.calculation_id == second.calculation_id),
            "both saved results remain readable in the history"
        );

        // The restored mapping is verified against the runtime session, and selection works.
        let runtime = RuntimeOccurrenceMap::new(&reopened).expect("the rebuilt runtime map");
        for occurrence in &restored.analysis.occurrences {
            let runtime_id = runtime.runtime_occurrence(&occurrence.source_path).expect("the saved path is placed");
            assert_eq!(runtime.source_path(runtime_id).expect("the runtime id maps back"), occurrence.source_path);
        }
        let rows = reopened.solidbom_parts().expect("the parts projection").rows;
        let items: Vec<_> = rows[0].occurrence_paths.iter().map(|path| json!({"type": "occurrence", "path": path})).collect();
        let selected = reopened.execute("select.set", &json!({"items": items})).expect("selection works on the reviewed source");
        assert_eq!(selected["selected"], 3);

        // The reopened source is read-only through the same policy.
        assert!(
            matches!(reopened.execute("solid.box", &json!({"length": 1, "width": 1, "height": 1})), Err(EngineError::Refused { .. })),
            "the reopened source stays read-only"
        );
        assert!(reopened.execute("document.inspect", &json!({})).is_ok(), "inspection stays available");
    }

    #[test]
    fn a_stalled_service_request_times_out_as_a_visible_network_failure() {
        use std::net::TcpListener;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let listener = TcpListener::bind("127.0.0.1:0").expect("a local listener");
        let address = listener.local_addr().expect("the listener address");
        let accepted = Arc::new(AtomicUsize::new(0));
        let counter = accepted.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                if counter.fetch_add(1, Ordering::Relaxed) > 4 {
                    break;
                }
                // Accept the request and never answer it: the client's own bound must fire.
                std::thread::sleep(Duration::from_secs(5));
                drop(stream);
            }
        });
        let host: solidcraft_engine::solidbom::HostHandle =
            Arc::new(HttpHost::with_timeout(format!("http://{address}"), "t".repeat(32), Duration::from_millis(200)));
        let job = host.submit(JobRequest::new(JobEnvelope::default(), ClientRequest::ReferenceData)).expect("the request is submitted");
        let began = Instant::now();
        loop {
            if let Some(completion) = host.poll().into_iter().find(|completion| completion.job_id == job) {
                match completion.result {
                    Err(ClientFailure::Network(message)) => {
                        assert!(!message.is_empty(), "the timeout names its cause");
                        break;
                    }
                    other => panic!("a stalled request must fail as a network failure, not {other:?}"),
                }
            }
            assert!(began.elapsed() < Duration::from_secs(10), "the request never timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(accepted.load(Ordering::Relaxed), 1, "the request reached the service once");
    }

    #[test]
    fn a_save_request_keeps_its_payload_identical_for_an_explicit_retry() {
        // The retry contract is the same request id and payload; the reusable `SaveDraft` a
        // conflict retains must serialize identically, so a retried write cannot become a new
        // record. This is a wire-level check of the type, not a service call.
        let nil = "00000000-0000-4000-8000-000000000000".parse().expect("a nil uuid");
        let source = solidbom_contract::SourceRef { source_id: nil, sha256: "a".repeat(64), byte_len: 10, original_name: "source.step".into() };
        let draft = DraftProject {
            project_id: nil,
            product_revision_id: nil,
            version: 2,
            source,
            analysis: solidbom_contract::AnalysisManifest {
                schema_version: solidbom_contract::SCHEMA_VERSION,
                key: solidbom_contract::AnalysisKey { source_sha256: "a".repeat(64), engine_build: "b".into(), extractor_version: "2".into() },
                definitions: Vec::new(),
                occurrences: Vec::new(),
                geometry_bindings: Vec::new(),
                issues: Vec::new(),
            },
            material_overhead_rate: Some(decimal("0.05")),
            parts: Vec::new(),
            forecast_year: 2027,
            forecast_quantity: 1000,
            currency: "CNY".into(),
        };
        let save = SaveDraft { request_id: "retry-me".into(), expected_version: 2, draft };
        let first = serde_json::to_vec(&save).expect("the save request");
        let retried = serde_json::to_vec(&save.clone()).expect("the retried save request");
        assert_eq!(first, retried);
    }
}
