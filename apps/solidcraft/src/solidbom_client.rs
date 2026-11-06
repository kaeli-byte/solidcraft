//! Native HTTP transport for SolidBOM.
//!
//! Every request runs on its own worker thread and answers on a channel the UI polls; the window
//! thread never waits for the service. Endpoints and the development token come from
//! `SOLIDBOM_ENDPOINT` and `SOLIDBOM_TOKEN`; the token is never logged. There is no automatic
//! retry of a mutation: a failed request is retried only when the user asks.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use solidbom_contract::{CalculationSnapshot, ProjectView, ReferenceData, SaveReceipt, SourceRef};
use solidcraft_engine::solidbom::{ClientFailure, ClientReply, ClientRequest, HostHandle, JobCompletion, JobId, SolidBomHost};
const TIMEOUT: Duration = Duration::from_secs(30);
const BODY_LIMIT: u64 = 128 * 1024 * 1024;

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
    next: AtomicU64,
    tx: Sender<JobCompletion>,
    rx: Mutex<Receiver<JobCompletion>>,
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
        Self { endpoint, token, agent, next: AtomicU64::new(0), tx, rx: Mutex::new(rx) }
    }
}

impl SolidBomHost for HttpHost {
    fn submit(&self, request: ClientRequest) -> std::result::Result<JobId, String> {
        let job = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let endpoint = self.endpoint.clone();
        let token = self.token.clone();
        let agent = self.agent.clone();
        let tx = self.tx.clone();
        std::thread::Builder::new()
            .name("solidbom-http".into())
            .spawn(move || {
                let result = perform(&agent, &endpoint, &token, request);
                let _ = tx.send(JobCompletion { job_id: job, result });
            })
            .map_err(|error| format!("the SolidBOM request thread did not start: {error}"))?;
        Ok(job)
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
    use super::encode;

    #[test]
    fn a_file_name_is_percent_encoded_for_the_query() {
        assert_eq!(encode("three_boxes.step"), "three_boxes.step");
        assert_eq!(encode("a b+c.step"), "a%20b%2Bc.step");
    }
}
