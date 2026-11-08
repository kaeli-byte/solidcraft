//! Background local analysis: open a reviewed STEP source and extract its manifest on a worker
//! thread, then publish the answer only while it still describes the record that asked for it.
//!
//! The worker owns a scratch session, so the design on screen is never touched while the source
//! is read and measured. The window thread only starts the job and takes the answer on a poll: it
//! never waits for the analysis, and a worker that cannot start is a reported capability error
//! rather than synchronous work on the window thread. A completion for a design that has since
//! been replaced, or for a newer reviewed binding, is discarded instead of published.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

use solidcraft_doc::progress::{Progress, watch};

use super::extract::extract;
use crate::{EngineError, Result, Session};
use serde_json::json;
use solidbom_contract::{AnalysisKey, AnalysisManifest, SourceRef};

/// What a finished analysis will be published as.
#[derive(Debug, Clone)]
pub enum AnalysisPurpose {
    /// A first import: publish the analysed design as a new reviewed project.
    Import {
        source: SourceRef,
        /// The original STEP file's canonical path: the reviewed session must never write it.
        source_path: PathBuf,
    },
    /// A reopen: the saved project is already loaded and the design must verify against the
    /// source it names.
    Reopen { source: SourceRef },
}

/// Everything the worker needs. `bytes` is the exact source the caller read and hashed, so the
/// analysis and the later upload describe the same file even if the original path changes.
pub struct AnalysisRequest {
    pub bytes: Vec<u8>,
    /// The reviewed source's original name: names the design and selects the STEP suffix.
    pub original_name: String,
    pub key: AnalysisKey,
    pub purpose: AnalysisPurpose,
    pub command_check: Option<crate::CommandCheck>,
    pub validation: solidcraft_kernel::Validation,
    pub param_check: crate::schema::ParamCheck,
    pub auto_project: bool,
}

/// A finished scratch analysis, ready to be published when it still matches its session.
pub struct PreparedAnalysis {
    pub session: Session,
    pub manifest: AnalysisManifest,
    /// The verified source bytes, for the upload that follows a first import.
    pub bytes: Vec<u8>,
}

/// The kind of worker spawn the job uses; tests inject a failing one through [`AnalysisJob::start_with`].
pub(crate) type Spawn = Box<dyn FnOnce(Box<dyn FnOnce() + Send>) -> io::Result<()> + Send>;

/// One analysis running beside the window.
pub struct AnalysisJob {
    purpose: AnalysisPurpose,
    key: AnalysisKey,
    /// The session's design-replacement counter, reviewed-binding generation and reviewed-input
    /// generation when the job started: an answer for a record that has since changed is stale and
    /// is not published, and a reviewed edit made meanwhile leaves the reopened project dirty.
    replacements: u64,
    binding_generation: u64,
    inputs_generation: u64,
    /// Cooperative stop for the worker's kernel work; cancelling the job drops the answer.
    stop: Arc<Progress>,
    rx: Receiver<std::result::Result<PreparedAnalysis, String>>,
}

impl std::fmt::Debug for AnalysisJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnalysisJob")
            .field("purpose", &self.purpose)
            .field("key", &self.key)
            .field("replacements", &self.replacements)
            .field("binding_generation", &self.binding_generation)
            .field("inputs_generation", &self.inputs_generation)
            .finish_non_exhaustive()
    }
}

impl AnalysisJob {
    /// Start the worker. `replacements` and `binding_generation` are the session's counters at
    /// submission; the caller publishes the answer only when they are unchanged.
    pub(crate) fn start(request: AnalysisRequest, replacements: u64, binding_generation: u64, inputs_generation: u64) -> Result<Self> {
        Self::start_with(request, replacements, binding_generation, inputs_generation, Box::new(spawn_worker))
    }

    /// The same start with the spawn seam exposed: a test can refuse the worker without relying on
    /// the machine's thread limits.
    pub(crate) fn start_with(
        request: AnalysisRequest,
        replacements: u64,
        binding_generation: u64,
        inputs_generation: u64,
        spawn: Spawn,
    ) -> Result<Self> {
        let purpose = request.purpose.clone();
        let key = request.key.clone();
        let stop = Arc::new(Progress::default());
        let worker_stop = stop.clone();
        let channel = mpsc::channel();
        let tx: Sender<std::result::Result<PreparedAnalysis, String>> = channel.0;
        let rx = channel.1;
        let work: Box<dyn FnOnce() + Send> = Box::new(move || {
            // A panic in the worker must not take the process down: it becomes a failed analysis,
            // and the session keeps the design it already had.
            let answer = if worker_stop.is_cancelled() {
                Err("the SolidBOM analysis was cancelled".into())
            } else {
                // The kernel watches this token, so a cancelled or replaced analysis stops at its
                // next kernel check instead of finishing work nobody will use.
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| watch(worker_stop.clone(), || analyse(request))))
                    .unwrap_or_else(|_| Err("the SolidBOM analysis worker panicked; the design is unchanged".into()))
            };
            let _ = tx.send(answer);
        });
        spawn(work).map_err(|error| EngineError::Capability(format!("the SolidBOM analysis worker did not start: {error}")))?;
        Ok(Self { purpose, key, replacements, binding_generation, inputs_generation, stop, rx })
    }

    pub(crate) fn purpose(&self) -> &AnalysisPurpose {
        &self.purpose
    }

    pub(crate) fn key(&self) -> &AnalysisKey {
        &self.key
    }

    pub(crate) fn replacements(&self) -> u64 {
        self.replacements
    }

    pub(crate) fn binding_generation(&self) -> u64 {
        self.binding_generation
    }

    pub(crate) fn inputs_generation(&self) -> u64 {
        self.inputs_generation
    }

    /// Stop working on this analysis cooperatively: the kernel checks the stop token at its next
    /// check point, and the answer is dropped by the caller anyway.
    pub(crate) fn cancel(&self) {
        self.stop.cancel();
    }

    /// The worker's answer once it has one, `None` while it still runs. A worker that stopped
    /// without answering (a panic outside the catch, a dropped process) is a visible failure.
    pub(crate) fn take_answer(&mut self) -> Option<std::result::Result<PreparedAnalysis, String>> {
        match self.rx.try_recv() {
            Ok(answer) => Some(answer),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err("the SolidBOM analysis worker stopped before it answered".into())),
        }
    }

    /// A pending job whose answer is delivered by the caller: deterministic tests drive the
    /// publication path through the same channel the worker uses, without timing.
    #[cfg(test)]
    pub(crate) fn pending(
        purpose: AnalysisPurpose,
        key: AnalysisKey,
        replacements: u64,
        binding_generation: u64,
        inputs_generation: u64,
        rx: Receiver<std::result::Result<PreparedAnalysis, String>>,
    ) -> Self {
        Self { purpose, key, replacements, binding_generation, inputs_generation, stop: Arc::new(Progress::default()), rx }
    }
}

/// The native worker spawn. The browser build has no threads: local analysis is a desktop
/// capability, and the absence is reported rather than hidden behind window-thread work.
#[cfg(not(target_arch = "wasm32"))]
fn spawn_worker(work: Box<dyn FnOnce() + Send>) -> io::Result<()> {
    std::thread::Builder::new().name("solidbom-analysis".into()).spawn(work).map(|_| ())
}

#[cfg(target_arch = "wasm32")]
fn spawn_worker(_work: Box<dyn FnOnce() + Send>) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "this build has no worker threads"))
}

/// Read and measure one source in a scratch session. The file is written under a generated name
/// first, because the original path may change or be withdrawn while the worker reads it and the
/// upload must carry exactly the analysed bytes.
fn analyse(mut request: AnalysisRequest) -> std::result::Result<PreparedAnalysis, String> {
    controlled_delay();
    let carrier = TempSource(temporary_source_path(&request.original_name));
    std::fs::write(&carrier.0, &request.bytes).map_err(|error| format!("the source could not be prepared for analysis: {error}"))?;
    let outcome = analyse_source(&request, &carrier.0);
    drop(carrier);
    let mut prepared = outcome?;
    // Hand the exact analysed bytes back for the upload that follows a first import.
    prepared.bytes = std::mem::take(&mut request.bytes);
    Ok(prepared)
}

/// The generated file that carries the source to `doc.open`; removed when it goes out of scope,
/// including when a panic escapes between writing it and opening it.
struct TempSource(PathBuf);

impl Drop for TempSource {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn analyse_source(request: &AnalysisRequest, path: &Path) -> std::result::Result<PreparedAnalysis, String> {
    let mut prepared = Session::default();
    prepared.set_command_check(request.command_check.clone());
    prepared.set_validation(request.validation);
    prepared.param_check = request.param_check;
    prepared.auto_project = request.auto_project;
    let opened = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| prepared.execute("doc.open", &json!({"path": path.to_string_lossy()}))))
        .map_err(|_| "the source could not be opened (the analysis worker stopped)".to_string())?;
    opened.map_err(|error| error.to_string())?;
    // The generated file was only a carrier: the session keeps no path to it.
    prepared.path = None;
    prepared.read_only = false;
    prepared.doc_mut().name = design_name(&request.original_name);
    let manifest = extract(&prepared, request.key.clone()).map_err(|error| error.to_string())?;
    Ok(PreparedAnalysis { session: prepared, manifest, bytes: Vec::new() })
}

/// A controlled delay for responsiveness measurements only: `SOLIDBOM_ANALYSIS_DELAY_MS` holds a
/// real analysis so a person (or SB-08's P13 run) can check that the window keeps drawing and
/// answering selection while it runs. It changes no result and is capped; unset or invalid is no
/// delay. Like the development service's `SOLIDBOM_DELAY_MS`, it exists for measurement, never as
/// a production behaviour.
#[cfg(not(target_arch = "wasm32"))]
fn controlled_delay() {
    let Some(milliseconds) = std::env::var("SOLIDBOM_ANALYSIS_DELAY_MS").ok().and_then(|value| value.parse::<u64>().ok()) else {
        return;
    };
    std::thread::sleep(std::time::Duration::from_millis(milliseconds.min(60_000)));
}

#[cfg(target_arch = "wasm32")]
fn controlled_delay() {}

/// A unique STEP file under the temporary directory. The suffix decides the importer, and the
/// generated name keeps two analyses from reading each other's file.
fn temporary_source_path(original_name: &str) -> PathBuf {
    static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|elapsed| elapsed.subsec_nanos()).unwrap_or(0);
    let suffix = if original_name.to_ascii_lowercase().ends_with(".stp") { "stp" } else { "step" };
    std::env::temp_dir().join(format!("solidbom-analysis-{}-{serial}-{nanos}.{suffix}", std::process::id()))
}

/// The design name a source's file stem gives, as `doc.open` would name it.
fn design_name(original_name: &str) -> String {
    Path::new(original_name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::to_string)
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| "reviewed source".into())
}
