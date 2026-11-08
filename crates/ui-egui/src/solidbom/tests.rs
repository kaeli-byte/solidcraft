//! The desktop side of the read-only review policy.

use solidbom_contract::{AnalysisKey, SourceRef};
use solidcraft_engine::solidbom::ReviewPolicy;

fn policy() -> ReviewPolicy {
    ReviewPolicy::new(
        SourceRef { source_id: uuid::Uuid::nil(), sha256: "a".repeat(64), byte_len: 1, original_name: "three_boxes.step".into() },
        AnalysisKey { source_sha256: "a".repeat(64), engine_build: "test".into(), extractor_version: "1".into() },
    )
}

#[test]
fn a_bound_session_does_not_open_a_modelling_tool() {
    let mut harness = crate::scenario::Harness::new();
    harness.app.session.bind_review(policy());
    let revision = harness.app.session.revision;
    harness.app.start("solid.box");
    assert!(harness.app.dialog.is_none(), "no modelling dialog opens");
    assert_eq!(harness.app.session.revision, revision);
    assert!(harness.app.run("solid.box", serde_json::json!({})).is_err());
    assert_eq!(harness.app.session.revision, revision);
}

#[test]
fn an_unbound_session_still_opens_the_tool() {
    let mut harness = crate::scenario::Harness::new();
    harness.app.start("solid.box");
    assert!(harness.app.dialog.is_some());
}

/// Every interactive entry path refuses a reviewed session before it opens anything. The
/// toolbar's own buttons call `start_at`/`start_pinned` without going through `start`, so the
/// refusal belongs where they all meet.
#[test]
fn every_interactive_start_path_refuses_a_reviewed_source() {
    let mut harness = crate::scenario::Harness::new();
    harness.app.session.bind_review(policy());
    let revision = harness.app.session.revision;
    let undo = harness.app.session.undo.len();
    harness.app.start("solid.box");
    harness.app.start_at("solid.box", None);
    harness.app.start_pinned("solid.box", None);
    // The deferred, after-roll and view-confirm starts call this directly.
    harness.app.start_interactive("solid.box", None);
    // A tool, not a dialog.
    harness.app.start_at("sketch.line", None);
    harness.app.start_pinned("sketch.line", None);
    assert!(harness.app.dialog.is_none(), "no modelling dialog opens");
    assert!(harness.app.tool.is_none(), "no modelling tool starts");
    assert_eq!(harness.app.session.revision, revision, "the design does not change");
    assert_eq!(harness.app.session.undo.len(), undo, "no undo step goes in");
    let (text, _, error) = harness.app.status.as_ref().expect("the refusal is said");
    assert!(*error && text.contains("read-only while it is reviewed"), "{text}");
    assert!(text.contains("sketch.line"), "the refusal names the command: {text}");
}

/// A start that waited for the history marker to roll back goes through `start_interactive`:
/// it refuses a reviewed session and does not stay queued either.
#[test]
fn a_deferred_start_refuses_a_reviewed_source() {
    let mut harness = crate::scenario::Harness::new();
    harness.app.session.bind_review(policy());
    let revision = harness.app.session.revision;
    harness.app.start_after_roll = Some(("solid.box".into(), 1));
    harness.frames(2);
    assert!(harness.app.dialog.is_none(), "the deferred start opens nothing");
    assert!(harness.app.start_after_roll.is_none(), "it does not stay queued");
    assert_eq!(harness.app.session.revision, revision);
}

/// The policy belongs to one session. A reviewed tab does not lock an editable tab in the same
/// host, and the host's command check still applies to every tab.
#[test]
fn a_reviewed_tab_does_not_lock_an_editable_tab() {
    use std::sync::Arc;
    let mut harness = crate::scenario::Harness::new();
    harness.app.session.set_command_check(Some(Arc::new(
        |id: &str, _: &serde_json::Value, _: &solidcraft_engine::Session| {
            if id == "solid.cylinder" { Err("no cylinders".to_string()) } else { Ok(()) }
        },
    )));
    let mut reviewed = solidcraft_engine::Session::default();
    reviewed.bind_review(policy());
    crate::documents::open_tab(&mut harness.app, reviewed).expect("the reviewed tab opens");
    assert!(harness.app.session.review().is_some());
    // Showing the tab frames its model, which is view state: the revision is fixed after that.
    let reviewed_revision = harness.app.session.revision;
    harness.app.start("solid.box");
    assert!(harness.app.dialog.is_none(), "the reviewed tab opens no dialog");
    assert_eq!(harness.app.session.revision, reviewed_revision);
    crate::documents::switch(&mut harness.app, 0);
    assert!(harness.app.session.review().is_none(), "the other tab is ordinary");
    harness.app.start("solid.box");
    assert!(harness.app.dialog.is_some(), "the editable tab opens its dialog");
    crate::dialogs::cancel(&mut harness.app);
    harness.app.run("solid.box", serde_json::json!({"length": 10, "width": 10, "height": 10})).expect("the editable tab models");
    assert_eq!(
        harness.app.run("solid.cylinder", serde_json::json!({"radius": 2, "height": 3})),
        Err("no cylinders".to_string()),
        "the host's command check still applies"
    );
    crate::documents::switch(&mut harness.app, 1);
    assert!(harness.app.session.review().is_some(), "the reviewed tab stays reviewed");
    assert!(harness.app.run("solid.box", serde_json::json!({"length": 5, "width": 5, "height": 5})).is_err());
}

/// Editing a feature reopens its editor, which would write the source: a reviewed session
/// refuses before rolling the timeline back, and closes what was open.
#[test]
fn a_reviewed_session_does_not_reopen_a_feature_editor() {
    let mut harness = crate::scenario::Harness::new();
    harness.app.run("solid.box", serde_json::json!({"length": 10, "width": 10, "height": 10})).expect("the box");
    let feature = harness.app.session.doc.features[0].id;
    harness.app.start("solid.fillet");
    assert!(harness.app.dialog.is_some(), "an ordinary session opens the dialog");
    harness.app.session.bind_review(policy());
    let revision = harness.app.session.revision;
    let undo = harness.app.session.undo.len();
    let marker = harness.app.session.doc.marker;
    harness.app.edit_feature(feature);
    harness.app.edit_feature_at(feature, Some(solidcore_record::Vec3::new(1.0, 2.0, 3.0)));
    assert!(harness.app.dialog.is_none(), "the open dialog closes and no editor opens");
    assert!(harness.app.tool.is_none());
    assert_eq!(harness.app.session.revision, revision, "the design does not change");
    assert_eq!(harness.app.session.undo.len(), undo, "no undo step goes in");
    assert_eq!(harness.app.session.doc.marker, marker, "the timeline does not roll back");
    let (text, _, error) = harness.app.status.as_ref().expect("the refusal is said");
    assert!(*error && text.contains("read-only while it is reviewed"), "{text}");
}

/// Importing binds review mode while a dialog, a prepared editor and history work are pending:
/// all of it belongs to the design the import replaced, so all of it goes. A feature editor's
/// result must not restore the old design over the reviewed source.
#[test]
fn binding_the_review_policy_cancels_pending_authoring() {
    let mut harness = crate::scenario::Harness::new();
    harness.app.run("solid.box", serde_json::json!({"length": 10, "width": 10, "height": 10})).expect("the box");
    let feature = harness.app.session.doc.features[0].id;
    let marker = harness.app.session.doc.marker;
    harness.app.start("solid.fillet");
    harness.app.tool = crate::tools::Tool::for_command("sketch.line");
    assert!(harness.app.tool.is_some(), "a tool for the replaced design");
    harness.app.prepare_feature_edit(feature, 0, marker);
    let (job, _never_finishes) = crate::deferred::Deferred::never_done("timeline.roll_to", serde_json::json!({}), harness.app.session.revision);
    harness.app.deferred = Some(job);
    harness.app.deferred_queue.push(("solid.extrude".into(), serde_json::json!({})));
    harness.app.start_after_roll = Some(("solid.extrude".into(), 1));
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/solidbom/fixtures/three_boxes.step");
    harness.app.run("solidbom.import", serde_json::json!({"path": path.to_string_lossy()})).expect("the import");
    harness.settle();
    assert!(harness.app.session.review().is_some());
    assert!(harness.app.dialog.is_none(), "the dialog for the replaced design is gone");
    assert!(harness.app.tool.is_none());
    assert!(harness.app.pending_feature_edit.is_none(), "the prepared editor is dropped");
    assert!(harness.app.deferred.is_none(), "history work for the replaced design is dropped");
    assert!(harness.app.deferred_queue.is_empty());
    assert!(harness.app.start_after_roll.is_none(), "the waiting start is dropped");
    harness.frames(2);
}

/// A reviewed session may keep its own working copy, but save and export never write over the
/// original it was read from, including through an equivalent path.
#[test]
fn save_and_export_refuse_to_overwrite_the_reviewed_source() {
    let mut harness = crate::scenario::Harness::new();
    harness.app.run("solid.box", serde_json::json!({"length": 10, "width": 10, "height": 10})).expect("the box");
    let source = harness.dir.join("reviewed.step");
    let original = b"the reviewed source\n".to_vec();
    std::fs::write(&source, &original).expect("write the reviewed source");
    harness.app.session.bind_review(policy().with_source_path(&source));
    for command in ["file.save_as", "file.export"] {
        let error = harness.app.run(command, serde_json::json!({ "path": source })).expect_err("the write is refused");
        assert!(error.contains("would overwrite the reviewed source"), "{command}: {error}");
        assert_eq!(std::fs::read(&source).expect("the source"), original, "{command} left the source unchanged");
    }
    let equivalent = harness.dir.join("sub").join("..").join("reviewed.step");
    let error = harness.app.run("file.save_as", serde_json::json!({ "path": equivalent })).expect_err("an equivalent path is refused");
    assert!(error.contains("would overwrite the reviewed source"), "{error}");
    assert_eq!(std::fs::read(&source).expect("the source"), original);
    let working = harness.dir.join("working.solidcraft");
    harness.app.run("file.save_as", serde_json::json!({ "path": working })).expect("a local working copy");
    let exported = harness.dir.join("exported.step");
    harness.app.run("file.export", serde_json::json!({ "path": exported })).expect("an export elsewhere");
    assert!(working.exists() && exported.exists());
    assert_eq!(std::fs::read(&source).expect("the source"), original, "the reviewed source is untouched");
}

#[test]
fn the_review_command_opens_the_panel() {
    let mut harness = crate::scenario::Harness::new();
    harness.app.run("solidbom.review", serde_json::json!({})).expect("the review command");
    assert!(harness.app.solidbom.is_some());
}

#[test]
fn importing_a_step_opens_the_panel_without_a_service() {
    let mut harness = crate::scenario::Harness::new();
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/solidbom/fixtures/three_boxes.step");
    harness.app.run("solidbom.import", serde_json::json!({"path": path.to_string_lossy()})).expect("the import");
    harness.settle();
    assert!(harness.app.solidbom.is_some());
    harness.frames(2);
}

/// The import answers while the analysis runs beside the window: the state reports `importing`,
/// a second source is refused without disturbing it, and the published review is the same
/// command path the panel, control channel and scripts use.
#[test]
fn an_import_answers_pending_and_the_review_publishes_later() {
    let mut harness = crate::scenario::Harness::new();
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/solidbom/fixtures/three_boxes.step");
    let reply = harness.app.run("solidbom.import", serde_json::json!({"path": path.to_string_lossy()})).expect("the import starts");
    assert_eq!(reply["state"], serde_json::json!("importing"), "{reply}");
    assert!(harness.app.session.solidbom.draft.is_none(), "the review is not published yet");
    assert!(harness.app.session.review().is_none());
    let state = harness.app.run("solidbom.state", serde_json::json!({})).expect("the review state");
    assert_eq!(state["phase"], serde_json::json!("importing"), "{state}");
    // A second source is refused while the first is analysed; the pending analysis stays.
    let error = harness.app.run("solidbom.import", serde_json::json!({"path": path.to_string_lossy()})).expect_err("refused");
    assert!(error.contains("already being opened"), "{error}");
    assert!(harness.app.session.solidbom_analysis.is_some());
    // The window keeps drawing while the analysis runs; the review publishes on a later poll.
    harness.frames(1);
    harness.settle();
    let state = harness.app.run("solidbom.state", serde_json::json!({})).expect("the review state");
    assert_eq!(state["phase"], serde_json::json!("reviewing"), "{state}");
    assert!(harness.app.session.review().is_some(), "the verified source bounds the review");
    assert_eq!(harness.app.session.solidbom.version(), 0);
}

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use solidbom_contract::{
    CalculateRequest, CalculationInputs, CalculationOutcome, CalculationSnapshot, CalculationSummary, CostLine, DraftProject, Issue, MaterialVersion,
    MeasurementSource, PartCost, ProjectView, RateVersion, ReferenceData, ResolvedPartInput, ResultState as OutcomeState, ReviewState, SaveDraft,
    SaveReceipt, Sourcing,
};
use solidcraft_engine::solidbom::{
    ClientFailure, ClientReply, ClientRequest, HostError, JobCompletion, JobId, JobRequest, Phase, SaveState, SolidBomHost, SolidBomResultState,
};
use uuid::Uuid;

use crate::scenario::Harness;

/// The material and rate the development service seeds for the SolidBOM fixtures: the direct-cost
/// arithmetic in `tests/solidbom/acceptance.md` comes from exactly these values.
const MATERIAL: &str = "00000000-0000-4000-8000-000000000001";
const RATE: &str = "00000000-0000-4000-8000-000000000101";

fn material_id() -> Uuid {
    Uuid::parse_str(MATERIAL).expect("the fixture material version id")
}

fn rate_id() -> Uuid {
    Uuid::parse_str(RATE).expect("the fixture rate version id")
}

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/solidbom/fixtures").join(name)
}

fn reference_data() -> ReferenceData {
    ReferenceData {
        materials: vec![MaterialVersion {
            material_version_id: material_id(),
            name: "PA12 fixture".into(),
            density_g_cm3: "2.7".parse().expect("the fixture density"),
            price_per_kg: "20".parse().expect("the fixture price"),
            currency: "CNY".into(),
        }],
        rates: vec![RateVersion {
            rate_version_id: rate_id(),
            name: "Machining fixture".into(),
            rate_per_hour: "120".parse().expect("the fixture rate"),
            currency: "CNY".into(),
        }],
    }
}

/// A `f64` as the exact decimal text the contract carries, without display rounding: the fixture
/// arithmetic divides exactly, and ten places are more than its deepest value.
fn exact(value: f64) -> String {
    let text = format!("{value:.10}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// A contract decimal as a number, for the fake service's arithmetic only.
fn num<T: std::fmt::Display>(value: Option<T>) -> Option<f64> {
    value.and_then(|value| value.to_string().parse().ok())
}

fn calculation_id(request_id: &str) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_OID, request_id.as_bytes())
}

/// The in-memory store one fake service answers from. A restart creates a new host over the same
/// store: the new window forgets every session, but the saved projects, sources and immutable
/// calculations stay exactly as the previous launch left them.
#[derive(Default)]
struct FakeStore {
    projects: BTreeMap<Uuid, StoredProject>,
    sources: BTreeMap<Uuid, (SourceRef, Vec<u8>)>,
}

#[derive(Default)]
struct StoredProject {
    version: u64,
    draft: Option<DraftProject>,
    /// Every persisted version, so a calculation can resolve the exact draft it names.
    versions: BTreeMap<u64, DraftProject>,
    /// The history rows `ProjectView` lists, in the order the service appends them.
    calculations: Vec<CalculationSummary>,
    snapshots: BTreeMap<Uuid, CalculationSnapshot>,
    /// Committed save receipts by request id: a repeated request returns the same receipt.
    receipts: BTreeMap<String, SaveReceipt>,
    /// The saves that actually committed, one per distinct request.
    committed_saves: Vec<SaveDraft>,
}

impl FakeStore {
    fn create(&mut self, save: &SaveDraft) -> std::result::Result<SaveReceipt, ClientFailure> {
        if save.expected_version != 0 {
            return Err(ClientFailure::Invalid("a new project starts at version 0".into()));
        }
        if let Some(existing) = self.projects.get(&save.draft.project_id).and_then(|project| project.receipts.get(&save.request_id)) {
            return Ok(existing.clone());
        }
        if self.projects.contains_key(&save.draft.project_id) {
            return Err(ClientFailure::Conflict("the project id already exists".into()));
        }
        let receipt = SaveReceipt { request_id: save.request_id.clone(), project_id: save.draft.project_id, version: 1 };
        let mut draft = save.draft.clone();
        draft.version = receipt.version;
        let mut project = StoredProject { version: receipt.version, draft: Some(draft.clone()), ..Default::default() };
        project.versions.insert(receipt.version, draft);
        project.receipts.insert(save.request_id.clone(), receipt.clone());
        self.projects.insert(save.draft.project_id, project);
        Ok(receipt)
    }

    fn save(&mut self, save: &SaveDraft) -> std::result::Result<SaveReceipt, ClientFailure> {
        let Some(project) = self.projects.get_mut(&save.draft.project_id) else { return Err(ClientFailure::NotFound) };
        if let Some(existing) = project.receipts.get(&save.request_id) {
            return Ok(existing.clone());
        }
        if project.version != save.expected_version {
            return Err(ClientFailure::Conflict(format!("the project is at version {}, not {}", project.version, save.expected_version)));
        }
        let receipt = SaveReceipt { request_id: save.request_id.clone(), project_id: save.draft.project_id, version: project.version + 1 };
        let mut draft = save.draft.clone();
        draft.version = receipt.version;
        project.version = receipt.version;
        project.versions.insert(receipt.version, draft.clone());
        project.draft = Some(draft);
        project.receipts.insert(save.request_id.clone(), receipt.clone());
        project.committed_saves.push(save.clone());
        Ok(receipt)
    }

    fn view(&self, project_id: Uuid) -> std::result::Result<ProjectView, ClientFailure> {
        let Some(project) = self.projects.get(&project_id) else { return Err(ClientFailure::NotFound) };
        let Some(draft) = project.draft.clone() else { return Err(ClientFailure::NotFound) };
        Ok(ProjectView { draft, calculations: project.calculations.clone() })
    }

    fn load_calculation(&self, calculation_id: Uuid) -> std::result::Result<CalculationSnapshot, ClientFailure> {
        self.projects.values().find_map(|project| project.snapshots.get(&calculation_id).cloned()).ok_or(ClientFailure::NotFound)
    }

    fn record_calculation(&mut self, request: &CalculateRequest) -> std::result::Result<CalculationSnapshot, ClientFailure> {
        let Some(project) = self.projects.get_mut(&request.project_id) else { return Err(ClientFailure::NotFound) };
        let calculation_id = calculation_id(&request.request_id);
        if let Some(existing) = project.snapshots.get(&calculation_id) {
            return Ok(existing.clone());
        }
        let Some(draft) = project.versions.get(&request.saved_version).cloned() else { return Err(ClientFailure::NotFound) };
        let snapshot = calculation_snapshot(request, &draft);
        project.snapshots.insert(calculation_id, snapshot.clone());
        project.calculations.push(CalculationSummary {
            calculation_id,
            saved_version: snapshot.saved_version,
            calculator_version: snapshot.calculator_version.clone(),
        });
        Ok(snapshot)
    }
}

/// A controllable `SolidBomHost`: it answers every request at once from an in-memory store, records
/// every request for identity comparisons, and can fail the next mutations with a chosen
/// `ClientFailure`. It never sleeps, starts no thread and reaches no process or database.
struct FakeHost {
    next: AtomicU64,
    queue: Mutex<Vec<JobCompletion>>,
    store: Arc<Mutex<FakeStore>>,
    requests: Mutex<Vec<JobRequest>>,
    fail_next: Mutex<VecDeque<ClientFailure>>,
}

impl FakeHost {
    fn new(store: Arc<Mutex<FakeStore>>) -> Self {
        Self {
            next: AtomicU64::new(0),
            queue: Mutex::new(Vec::new()),
            store,
            requests: Mutex::new(Vec::new()),
            fail_next: Mutex::new(VecDeque::new()),
        }
    }

    /// Fail the next save (or project create) with `failure`, without committing it.
    fn fail_next(&self, failure: ClientFailure) {
        if let Ok(mut failures) = self.fail_next.lock() {
            failures.push_back(failure);
        }
    }

    /// Every request this host was handed, in submission order.
    fn requests(&self) -> Vec<JobRequest> {
        self.requests.lock().map(|requests| requests.clone()).unwrap_or_default()
    }

    fn take_failure(&self) -> Option<ClientFailure> {
        self.fail_next.lock().ok().and_then(|mut failures| failures.pop_front())
    }

    fn answer(&self, request: ClientRequest) -> std::result::Result<ClientReply, ClientFailure> {
        let mut store = self.store.lock().map_err(|_| ClientFailure::Server("the fake project store is unavailable".into()))?;
        match request {
            ClientRequest::UploadSource { sha256, original_name, bytes } => {
                let source = SourceRef {
                    source_id: Uuid::new_v5(&Uuid::NAMESPACE_OID, format!("{sha256}:source").as_bytes()),
                    sha256,
                    byte_len: bytes.len() as u64,
                    original_name,
                };
                store.sources.insert(source.source_id, (source.clone(), bytes));
                Ok(ClientReply::Source(source))
            }
            ClientRequest::CreateProject(save) => {
                if let Some(failure) = self.take_failure() {
                    return Err(failure);
                }
                store.create(&save).map(ClientReply::Receipt)
            }
            ClientRequest::SaveDraft(save) => {
                if let Some(failure) = self.take_failure() {
                    return Err(failure);
                }
                store.save(&save).map(ClientReply::Receipt)
            }
            ClientRequest::ReferenceData => Ok(ClientReply::ReferenceData(reference_data())),
            ClientRequest::Calculate(request) => store.record_calculation(&request).map(ClientReply::Calculation),
            ClientRequest::LoadProject(project_id) => store.view(project_id).map(ClientReply::Project),
            ClientRequest::DownloadSource(source_id) => store
                .sources
                .get(&source_id)
                .map(|(source, bytes)| ClientReply::SourceBytes(source.clone(), bytes.clone()))
                .ok_or(ClientFailure::NotFound),
            ClientRequest::LoadCalculation(calculation_id) => store.load_calculation(calculation_id).map(ClientReply::Calculation),
        }
    }
}

impl SolidBomHost for FakeHost {
    fn submit(&self, job: JobRequest) -> std::result::Result<JobId, HostError> {
        let job_id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        if let Ok(mut requests) = self.requests.lock() {
            requests.push(job.clone());
        }
        let envelope = job.envelope;
        let result = self.answer(job.request);
        if let Ok(mut queue) = self.queue.lock() {
            queue.push(JobCompletion { job_id, envelope, result });
        }
        Ok(job_id)
    }

    fn poll(&self) -> Vec<JobCompletion> {
        self.queue.lock().map(|mut queue| std::mem::take(&mut *queue)).unwrap_or_default()
    }
}

/// The calculation the fake service records for one saved version. It resolves the reviewed inputs
/// and the analysis issues like the service, and applies the approved fixture arithmetic, so the
/// snapshot carries real explained amounts: 1.1388016 per part / 3.4164048 assembly / 60 setup at a
/// 30-second cycle, 2.1388016 / 6.4164048 after 60 seconds.
fn calculation_snapshot(request: &CalculateRequest, draft: &DraftProject) -> CalculationSnapshot {
    let reference = reference_data();
    let (inputs, mut issues) = resolve_inputs(draft, &reference);
    let mut outcome = direct_cost(&inputs);
    if !issues.is_empty() {
        // A resolution problem makes the result incomplete exactly as the service refuses to call
        // it complete: the amounts the calculator could compute stay readable.
        outcome.state = OutcomeState::Incomplete;
    }
    let mut all = std::mem::take(&mut issues);
    all.append(&mut outcome.issues);
    outcome.issues = all.clone();
    CalculationSnapshot {
        calculation_id: calculation_id(&request.request_id),
        project_id: request.project_id,
        saved_version: request.saved_version,
        input_hash: "a".repeat(64),
        calculator_version: "solidbom-direct-cost/1".into(),
        inputs,
        outputs: outcome,
        issues: all,
    }
}

/// Turn a reviewed draft into calculator inputs and the review problems the calculator cannot see:
/// the same resolution rules as the service store (`resolve_inputs`), including the flat guard. A
/// missing value stays `None`, never zero.
fn resolve_inputs(draft: &DraftProject, reference: &ReferenceData) -> (CalculationInputs, Vec<Issue>) {
    let mut issues = Vec::new();
    let mut parts = Vec::with_capacity(draft.parts.len());
    for (index, part) in draft.parts.iter().enumerate() {
        let path = |field: &str| format!("parts[{index}].{field}");
        let occurrences = draft.analysis.occurrences.iter().filter(|occurrence| occurrence.definition_id == part.definition_id).count();
        let quantity = u32::try_from(occurrences).unwrap_or(u32::MAX);
        if occurrences == 0 {
            issues.push(Issue::new("no_occurrences", path("quantity"), "the definition has no occurrence").for_definition(part.definition_id));
        }
        if part.sourcing != Sourcing::Manufactured {
            issues.push(
                Issue::new("unsupported_sourcing", path("sourcing"), "purchased parts are outside this prototype").for_definition(part.definition_id),
            );
        }
        if !part.classification_confirmed {
            issues.push(
                Issue::new("unconfirmed_classification", path("classification"), "the classification is still pending")
                    .for_definition(part.definition_id),
            );
        }
        if draft.analysis.issues.iter().any(|issue| issue.code == "unresolved_grouping" && issue.definition_id == Some(part.definition_id)) {
            issues.push(
                Issue::new("unresolved_grouping", path("definition_id"), "the component grouping is unresolved; review is required")
                    .for_definition(part.definition_id),
            );
        }
        let volume = if part.volume_mm3.review == ReviewState::Confirmed {
            part.volume_mm3.value
        } else {
            issues.push(
                Issue::new("unreviewed_volume", path("volume_mm3"), "the measured volume is still pending review").for_definition(part.definition_id),
            );
            None
        };
        let material = part.material_version_id.and_then(|id| reference.material(id));
        let operation = part.operation.as_ref();
        let rate = |id: Option<Uuid>| id.and_then(|id| reference.rate(id)).map(|rate| rate.rate_per_hour);
        parts.push(ResolvedPartInput {
            part_version_id: part.part_version_id,
            quantity,
            volume_mm3: volume,
            density_g_cm3: material.map(|material| material.density_g_cm3),
            material_price_per_kg: material.map(|material| material.price_per_kg),
            allowance_rate: part.allowance_rate,
            cycle_seconds: operation.and_then(|operation| operation.cycle_seconds),
            cavities: operation.and_then(|operation| operation.cavities),
            production_rate_per_hour: rate(operation.and_then(|operation| operation.production_rate_version_id)),
            setup_hours: operation.and_then(|operation| operation.setup_hours),
            setup_rate_per_hour: rate(operation.and_then(|operation| operation.setup_rate_version_id)),
        });
    }
    (CalculationInputs { currency: draft.currency.clone(), material_overhead_rate: draft.material_overhead_rate, parts }, issues)
}

/// The direct-cost arithmetic of `solidbom-costing` version 1, over `f64` because the desktop crate
/// does not depend on the decimal crate: the fixture values are exact to the emitted precision and
/// the test asserts them as exact decimal strings.
fn direct_cost(inputs: &CalculationInputs) -> CalculationOutcome {
    let mut issues = Vec::new();
    let overhead_rate = match num(inputs.material_overhead_rate) {
        Some(rate) => Some(rate),
        None => {
            issues.push(Issue::new("missing_overhead_rate", "material_overhead_rate", "no material overhead rate was entered"));
            None
        }
    };
    if inputs.parts.is_empty() {
        issues.push(Issue::new("no_parts", "parts", "there is no reviewed part to calculate"));
    }
    let mut parts = Vec::with_capacity(inputs.parts.len());
    let mut assembly: Option<f64> = Some(0.0);
    let mut setup_total: Option<f64> = Some(0.0);
    let mut every_part_complete = true;
    for (index, part) in inputs.parts.iter().enumerate() {
        let (cost, setup, part_issues, complete) = part_cost(index, part, overhead_rate);
        issues.extend(part_issues);
        every_part_complete &= complete;
        assembly = match (assembly, num(cost.line_total)) {
            (Some(total), Some(line)) => Some(total + line),
            _ => None,
        };
        setup_total = match (setup_total, setup) {
            (Some(total), Some(charge)) => Some(total + charge),
            _ => None,
        };
        parts.push(cost);
    }
    let complete = issues.is_empty() && every_part_complete && assembly.is_some() && setup_total.is_some();
    CalculationOutcome {
        state: if complete { OutcomeState::Complete } else { OutcomeState::Incomplete },
        parts,
        assembly_subtotal: assembly.map(exact).map(|text| text.parse().expect("the assembly subtotal")),
        setup_total: setup_total.map(exact).map(|text| text.parse().expect("the setup total")),
        issues,
    }
}

/// One part's direct-cost lines, its separate setup charge, its named issues and whether every
/// supported line was known. The issue codes and the explanation lines match the calculator's.
fn part_cost(index: usize, part: &ResolvedPartInput, overhead_rate: Option<f64>) -> (PartCost, Option<f64>, Vec<Issue>, bool) {
    let mut issues = Vec::new();
    let issue =
        |code: &str, field: &str, message: &str| Issue::new(code, format!("parts[{index}].{field}"), message).for_definition(part.part_version_id);
    if part.quantity == 0 {
        issues.push(issue("invalid_quantity", "quantity", "a part must occur at least once"));
    }
    let volume = match num(part.volume_mm3) {
        Some(value) if value < 0.0 => {
            issues.push(issue("invalid_volume", "volume_mm3", "a reviewed volume cannot be negative"));
            None
        }
        Some(value) => Some(value),
        None => {
            issues.push(issue("missing_volume", "volume_mm3", "the part has no reviewed volume; an open surface needs a manual value"));
            None
        }
    };
    let density = match num(part.density_g_cm3) {
        Some(value) if value <= 0.0 => {
            issues.push(issue("invalid_density", "density_g_cm3", "the material density must be positive"));
            None
        }
        Some(value) => Some(value),
        None => {
            issues.push(issue("missing_density", "density_g_cm3", "no reviewed material density was selected"));
            None
        }
    };
    let price = match num(part.material_price_per_kg) {
        Some(value) if value < 0.0 => {
            issues.push(issue("invalid_material_price", "material_price_per_kg", "a material price cannot be negative"));
            None
        }
        Some(value) => Some(value),
        None => {
            issues.push(issue("missing_material_price", "material_price_per_kg", "the selected material version has no price"));
            None
        }
    };
    let allowance = match num(part.allowance_rate) {
        Some(value) if value < 0.0 => {
            issues.push(issue("invalid_allowance_rate", "allowance_rate", "a material allowance cannot be negative"));
            None
        }
        Some(value) => Some(value),
        None => {
            issues.push(issue("missing_allowance_rate", "allowance_rate", "no material allowance was entered"));
            None
        }
    };
    let cycle = match num(part.cycle_seconds) {
        Some(value) if value < 0.0 => {
            issues.push(issue("invalid_cycle_time", "cycle_seconds", "a cycle time cannot be negative"));
            None
        }
        Some(value) => Some(value),
        None => {
            issues.push(issue("missing_cycle_time", "cycle_seconds", "no production cycle time was entered"));
            None
        }
    };
    let cavities = match part.cavities {
        Some(0) => {
            issues.push(issue("invalid_cavities", "cavities", "at least one cavity is needed"));
            None
        }
        Some(value) => Some(f64::from(value)),
        // An omitted count is usable only when the cycle is an explicit zero.
        None if cycle == Some(0.0) => Some(1.0),
        None => {
            issues.push(issue("missing_cavities", "cavities", "no cavity count was entered"));
            None
        }
    };
    let production_rate = match num(part.production_rate_per_hour) {
        Some(value) if value <= 0.0 => {
            issues.push(issue("invalid_production_rate", "production_rate_per_hour", "the production rate must be positive"));
            None
        }
        Some(value) => Some(value),
        None if cycle == Some(0.0) => None,
        None => {
            issues.push(issue("missing_production_rate", "production_rate_per_hour", "no production rate version was selected"));
            None
        }
    };
    let setup = match num(part.setup_hours) {
        Some(hours) if hours < 0.0 => {
            issues.push(issue("invalid_setup_hours", "setup_hours", "setup time cannot be negative"));
            None
        }
        Some(hours) if hours > 0.0 => match num(part.setup_rate_per_hour) {
            Some(rate) if rate <= 0.0 => {
                issues.push(issue("invalid_setup_rate", "setup_rate_per_hour", "the setup rate must be positive"));
                None
            }
            Some(rate) => Some(hours * rate),
            None => {
                issues.push(issue("missing_setup_rate", "setup_rate_per_hour", "no setup rate version was selected"));
                None
            }
        },
        Some(_) => Some(0.0),
        None => {
            issues.push(issue("missing_setup_hours", "setup_hours", "no setup duration was entered"));
            None
        }
    };
    let net_mass = match (volume, density) {
        (Some(volume), Some(density)) => Some(volume * density / 1_000_000.0),
        _ => None,
    };
    let allowed_material = match (net_mass, price, allowance) {
        (Some(mass), Some(price), Some(allowance)) => Some(mass * price * (1.0 + allowance)),
        _ => None,
    };
    let material_overhead = match (allowed_material, overhead_rate) {
        (Some(material), Some(rate)) => Some(material * rate),
        _ => None,
    };
    let production = match (cycle, cavities) {
        // A cycle time is for the tool, so a multi-cavity tool makes a piece faster.
        (Some(cycle), Some(cavities)) => match production_rate {
            Some(rate) => Some(cycle / cavities * rate / 3_600.0),
            None if cycle == 0.0 => Some(0.0),
            None => None,
        },
        _ => None,
    };
    let per_part = match (allowed_material, material_overhead, production) {
        (Some(material), Some(overhead), Some(production)) => Some(material + overhead + production),
        _ => None,
    };
    let line_total = per_part.map(|subtotal| subtotal * f64::from(part.quantity));
    let explanations = vec![
        cost_line("material.mass_kg", "Net mass", net_mass, "volume × density"),
        cost_line("material.allowed_cost", "Allowed material cost", allowed_material, "mass × price × (1 + allowance)"),
        cost_line("material.overhead", "Material overhead", material_overhead, "material cost × overhead rate"),
        cost_line("production.cost", "Production cost", production, "cycle seconds ÷ cavities × hourly rate ÷ 3600"),
        cost_line("part.subtotal", "Direct-cost subtotal per part", per_part, "material + overhead + production"),
        cost_line("part.line_total", "Direct-cost subtotal for the quantity", line_total, "per-part subtotal × quantity"),
        cost_line("setup.charge", "Setup charge (separate)", setup, "setup hours × setup rate"),
    ];
    let complete = issues.is_empty()
        && part.quantity > 0
        && volume.is_some()
        && density.is_some()
        && price.is_some()
        && allowance.is_some()
        && cycle.is_some()
        && cavities.is_some()
        && (production_rate.is_some() || cycle == Some(0.0))
        && setup.is_some()
        && per_part.is_some()
        && line_total.is_some();
    let cost = PartCost {
        part_version_id: part.part_version_id,
        quantity: part.quantity,
        net_mass_kg: net_mass.map(exact).map(|text| text.parse().expect("the net mass")),
        allowed_material_cost: allowed_material.map(exact).map(|text| text.parse().expect("the allowed material cost")),
        material_overhead: material_overhead.map(exact).map(|text| text.parse().expect("the material overhead")),
        production_cost: production.map(exact).map(|text| text.parse().expect("the production cost")),
        per_part_subtotal: per_part.map(exact).map(|text| text.parse().expect("the per-part subtotal")),
        line_total: line_total.map(exact).map(|text| text.parse().expect("the quantity line total")),
        explanations,
    };
    (cost, setup, issues, complete)
}

fn cost_line(code: &str, label: &str, amount: Option<f64>, basis: &str) -> CostLine {
    CostLine {
        code: code.to_string(),
        label: label.to_string(),
        amount: amount.map(exact).map(|text| text.parse().expect("a cost line amount")),
        basis: basis.to_string(),
    }
}

fn fake_harness(store: &Arc<Mutex<FakeStore>>) -> (Harness, Arc<FakeHost>) {
    let mut harness = Harness::new();
    let host = Arc::new(FakeHost::new(store.clone()));
    harness.app.session.solidbom_host = Some(host.clone());
    (harness, host)
}

/// Run frames without blocking until `done` holds. Polling is the only wakeup path, so an answer
/// arrives on a frame; the bound is a failure, never a sleep.
fn pump_until(harness: &mut Harness, what: &str, mut done: impl FnMut(&Harness) -> bool) {
    for _ in 0..600 {
        if done(harness) {
            return;
        }
        harness.frame();
    }
    let state = &harness.app.session.solidbom;
    panic!(
        "timed out waiting for {what}: phase {:?}, save {:?}, result {:?}, error {:?}, {} pending",
        state.phase,
        state.save,
        state.result,
        state.error,
        state.pending.len()
    );
}

fn import_fixture(harness: &mut Harness, name: &str) {
    let path = fixture(name);
    harness.app.run("solidbom.import", json!({"path": path.to_string_lossy()})).expect("the import starts");
    harness.settle();
    pump_until(harness, "the first saved project version and its reference data", |harness| {
        harness.app.session.solidbom.version() >= 1 && harness.app.session.solidbom.reference.is_some()
    });
    harness.frames(2);
}

/// Reopen a saved project by id in this window: the download, the verified re-analysis and the
/// current result all arrive on frames.
fn reopen_project(harness: &mut Harness, project_id: Uuid) {
    harness.app.run("solidbom.reopen", json!({"project_id": project_id.to_string()})).expect("the project starts reopening");
    pump_until(harness, "the downloaded source of the reopened project", |harness| harness.app.session.solidbom_analysis.is_some());
    harness.settle();
    pump_until(harness, "the reopened review", |harness| harness.app.session.solidbom.phase == Phase::Reviewing);
    harness.frames(2);
}

fn first_part(harness: &Harness) -> Uuid {
    harness.app.session.solidbom.draft.as_ref().expect("the reviewed draft").parts[0].part_version_id
}

fn review(harness: &mut Harness, part_version_id: Uuid, patch: Value) {
    let mut params = patch;
    params["part_version_id"] = json!(part_version_id.to_string());
    harness.app.run("solidbom.inputs", params).expect("the reviewed inputs");
}

/// The approved fixture review: the extracted 2400 mm3 volume confirmed, the PA12 material and the
/// machining rate selected, 2 % allowance, 5 % overhead and the fixture's separate setup.
fn review_three_boxes(harness: &mut Harness, cycle_seconds: &str) {
    let part = first_part(harness);
    review(
        harness,
        part,
        json!({
            "classification": "clip",
            "classification_confirmed": true,
            "volume_confirmed": true,
            "material_version_id": MATERIAL,
            "allowance_rate": "0.02",
            "cycle_seconds": cycle_seconds,
            "cavities": 1,
            "production_rate_version_id": RATE,
            "setup_hours": "0.5",
            "setup_rate_version_id": RATE,
            "material_overhead_rate": "0.05"
        }),
    );
}

/// Calculate and return the new snapshot's id once the answer is applied, so a test never mistakes
/// the previous (possibly stale) result for the new one.
fn calculate(harness: &mut Harness) -> Uuid {
    let previous = harness.app.session.solidbom.current.as_ref().map(|snapshot| snapshot.calculation_id);
    harness.app.run("solidbom.calculate", json!({})).expect("the calculation starts");
    pump_until(harness, "the calculated snapshot", |harness| {
        let state = &harness.app.session.solidbom;
        state.current.as_ref().is_some_and(|snapshot| Some(snapshot.calculation_id) != previous)
            && !matches!(state.result, SolidBomResultState::Calculating { .. })
    });
    harness.frames(2);
    harness.app.session.solidbom.current.as_ref().expect("the current snapshot").calculation_id
}

fn state_of(harness: &mut Harness) -> Value {
    harness.app.run("solidbom.state", json!({})).expect("the review state")
}

/// A saved review survives a full restart: a new window and a new fake host over the same in-memory
/// store reopen the project by id. The recovered source hash, occurrence mapping, reviewed values
/// and the original saved calculation must all describe what was saved, with the result shown as
/// the saved one and never as a new calculation.
#[test]
fn a_saved_review_reopens_after_a_restart_from_the_same_store() {
    let store = Arc::new(Mutex::new(FakeStore::default()));
    let (mut first, _host) = fake_harness(&store);
    import_fixture(&mut first, "three_boxes.step");
    review_three_boxes(&mut first, "30");
    let first_calculation = calculate(&mut first);
    first.frames(2);

    let draft = first.app.session.solidbom.draft.clone().expect("the reviewed draft");
    let project_id = draft.project_id;
    let saved_version = draft.version;
    let source = draft.source.clone();
    let occurrences = draft.analysis.occurrences.clone();
    let reviewed_parts = draft.parts.clone();
    let calculation = first.app.session.solidbom.current.clone().expect("the saved calculation");
    assert_eq!(calculation.saved_version, saved_version);
    assert_eq!(first.app.session.solidbom.calculations.len(), 1);
    assert!(crate::scenario::rect_of("bom:table").is_some(), "the BOM workspace drew the review");
    // The cost group is a disclosure; open it the way the Cost tab does, then read the drawn line.
    first.app.run("view.bom.cost_breakdown", json!({})).expect("the cost breakdown section");
    first.frames(2);
    // The summary amounts are displayed at two places; the explanation lines keep the exact
    // backend amounts. Both come from the saved snapshot, never from the current draft.
    assert!(!first.widgets("Direct cost per part: 1.14 CNY").is_empty(), "the panel drew the two-place per-part subtotal");
    assert!(!first.widgets("Assembly direct-cost subtotal: 3.42 CNY").is_empty(), "the panel drew the two-place assembly subtotal");
    assert!(!first.widgets("Setup charge (separate): 60.00 CNY").is_empty(), "the panel drew the separate setup charge");
    assert!(!first.widgets("Production cost: 1").is_empty(), "the panel drew the exact explanation line");

    // A relaunch: a new window and a new host, but the store the first process left behind.
    drop(first);
    let (mut second, _host) = fake_harness(&store);
    reopen_project(&mut second, project_id);
    pump_until(&mut second, "the reopened saved calculation", |harness| harness.app.session.solidbom.current.is_some());
    second.frames(2);

    let reopened = second.app.session.solidbom.draft.as_ref().expect("the reopened draft");
    assert_eq!(reopened.project_id, project_id);
    assert_eq!(reopened.version, saved_version);
    assert_eq!(reopened.source, source, "the recovered source identity is the saved one");
    assert_eq!(reopened.source.sha256.len(), 64);
    assert_eq!(reopened.analysis.occurrences, occurrences, "the restored occurrence mapping is the saved one");
    assert_eq!(reopened.parts, reviewed_parts, "the reviewed values came back");
    let runtime = second.app.session.solidbom.runtime.as_ref().expect("the verified runtime mapping");
    for occurrence in &reopened.analysis.occurrences {
        assert!(runtime.runtime_occurrence(&occurrence.source_path).is_ok(), "the saved path {:?} maps to a live occurrence", occurrence.source_path);
    }
    let state = &second.app.session.solidbom;
    let recovered = state.current.as_ref().expect("the saved result is current");
    assert_eq!(recovered.calculation_id, first_calculation, "the saved calculation is shown as itself, never as new");
    assert_eq!(recovered.saved_version, saved_version);
    assert_eq!(state.summaries.len(), 1);
    assert_eq!(state.summaries[0].calculation_id, calculation.calculation_id);
    assert_eq!(state.result, SolidBomResultState::Current);
    assert_eq!(state.save, SaveState::Clean);
    second.app.run("view.bom.cost_breakdown", json!({})).expect("the cost breakdown section");
    second.frames(2);
    assert!(!second.widgets("Assembly direct-cost subtotal: 3.42 CNY").is_empty(), "the reopened panel drew the saved result");
    assert!(!second.widgets("Direct cost per part: 1.14 CNY").is_empty(), "the reopened panel drew the saved per-part subtotal");
}

/// The 60-second change: the earlier result becomes stale but stays readable, the earlier snapshot
/// is retained, and the recalculation uses the new cycle. Setup stays a separate 60.
#[test]
fn a_cycle_change_marks_the_result_stale_and_recalculates_over_the_retained_snapshot() {
    let store = Arc::new(Mutex::new(FakeStore::default()));
    let (mut harness, _host) = fake_harness(&store);
    import_fixture(&mut harness, "three_boxes.step");
    review_three_boxes(&mut harness, "30");
    let first_calculation = calculate(&mut harness);
    let first = harness.app.session.solidbom.current.clone().expect("the first calculation");
    assert_eq!(first.outputs.parts[0].per_part_subtotal, Some("1.1388016".parse().expect("the first per-part subtotal")));
    assert_eq!(first.outputs.assembly_subtotal, Some("3.4164048".parse().expect("the first assembly subtotal")));
    assert_eq!(first.outputs.setup_total, Some("60".parse().expect("the separate setup")));

    review_three_boxes(&mut harness, "60");
    {
        let state = &harness.app.session.solidbom;
        assert_eq!(state.result, SolidBomResultState::Stale, "the shown result no longer describes the inputs");
        assert_eq!(state.save, SaveState::Dirty);
        assert_eq!(state.current.as_ref().map(|snapshot| snapshot.calculation_id), Some(first_calculation), "the stale snapshot stays shown");
        assert!(
            state.calculations.iter().any(|snapshot| snapshot.calculation_id == first_calculation),
            "the earlier snapshot stays readable in the history"
        );
    }

    let second_calculation = calculate(&mut harness);
    assert_ne!(second_calculation, first_calculation);
    let state = &harness.app.session.solidbom;
    assert_eq!(state.result, SolidBomResultState::Current);
    assert_eq!(state.version(), 3);
    let second = state.current.as_ref().expect("the recalculated result");
    assert_eq!(second.saved_version, 3);
    assert_eq!(second.outputs.parts[0].per_part_subtotal, Some("2.1388016".parse().expect("the 60-second per-part subtotal")));
    assert_eq!(second.outputs.assembly_subtotal, Some("6.4164048".parse().expect("the 60-second assembly subtotal")));
    assert_eq!(second.outputs.setup_total, Some("60".parse().expect("the separate setup")));
    assert_eq!(state.calculations.len(), 2);
    let retained = state.calculations.iter().find(|snapshot| snapshot.calculation_id == first_calculation).expect("the first snapshot is retained");
    assert_eq!(retained.saved_version, 2);
    assert_eq!(retained.outputs.parts[0].per_part_subtotal, Some("1.1388016".parse().expect("the retained per-part subtotal")));
    harness.frames(2);
}

/// Missing inputs never become zero: without a material and a cycle time the calculation is
/// incomplete, names `missing_cycle_time` and withholds the complete subtotals.
#[test]
fn missing_material_and_cycle_time_make_the_result_incomplete() {
    let store = Arc::new(Mutex::new(FakeStore::default()));
    let (mut harness, _host) = fake_harness(&store);
    import_fixture(&mut harness, "three_boxes.step");
    let part = first_part(&harness);
    review(&mut harness, part, json!({"classification": "clip", "classification_confirmed": true, "volume_confirmed": true}));
    calculate(&mut harness);
    let state = &harness.app.session.solidbom;
    assert_eq!(state.result, SolidBomResultState::Incomplete);
    let snapshot = state.current.as_ref().expect("the incomplete result is readable");
    assert_eq!(snapshot.outputs.state, OutcomeState::Incomplete);
    assert!(snapshot.issues.iter().any(|issue| issue.code == "missing_cycle_time"), "{:?}", snapshot.issues);
    assert_eq!(snapshot.outputs.assembly_subtotal, None, "no complete subtotal");
    assert_eq!(snapshot.outputs.setup_total, None);
    assert!(snapshot.outputs.parts.iter().all(|part| part.per_part_subtotal.is_none()));
    let reply = state_of(&mut harness);
    assert_eq!(reply["result"], json!("incomplete"));
    assert_eq!(reply["assembly_subtotal"], Value::Null);
    assert_eq!(reply["per_part_subtotal"], Value::Null);
    assert!(reply["issues"].as_array().is_some_and(|issues| issues.iter().any(|issue| issue == "missing_cycle_time")), "{reply}");
}

/// An open surface has no volume at all, never zero. Completing it is explicit: a manual value and
/// a reason, then review confirmation, and the calculation uses the manual value.
#[test]
fn an_open_surface_needs_an_explicit_manual_volume_used_by_the_calculation() {
    let store = Arc::new(Mutex::new(FakeStore::default()));
    let (mut harness, _host) = fake_harness(&store);
    import_fixture(&mut harness, "open_surface.step");
    let part = first_part(&harness);
    let reply = state_of(&mut harness);
    assert_eq!(reply["volume_mm3"], Value::Null, "a missing volume is null, never zero");
    assert_eq!(reply["volume_source"], json!("extracted"));
    assert_eq!(reply["volume_review"], json!("pending"));
    assert!(harness.app.session.solidbom.draft.as_ref().expect("the draft").parts[0].volume_mm3.value.is_none());

    review(
        &mut harness,
        part,
        json!({
            "classification": "open housing",
            "classification_confirmed": true,
            "volume_mm3": "2400",
            "manual_reason": "measured on the workshop bench",
            "volume_confirmed": true,
            "material_version_id": MATERIAL,
            "allowance_rate": "0.02",
            "cycle_seconds": "30",
            "cavities": 1,
            "production_rate_version_id": RATE,
            "setup_hours": "0.5",
            "setup_rate_version_id": RATE,
            "material_overhead_rate": "0.05"
        }),
    );
    let reply = state_of(&mut harness);
    assert_eq!(reply["volume_mm3"], json!("2400"));
    assert_eq!(reply["volume_source"], json!("manual"));
    assert_eq!(reply["volume_review"], json!("confirmed"));
    let measurement = &harness.app.session.solidbom.draft.as_ref().expect("the draft").parts[0].volume_mm3;
    assert_eq!(measurement.source, MeasurementSource::Manual);
    assert_eq!(measurement.review, ReviewState::Confirmed);
    assert_eq!(measurement.reason.as_deref(), Some("measured on the workshop bench"));

    calculate(&mut harness);
    let snapshot = harness.app.session.solidbom.current.as_ref().expect("the calculated result");
    assert_eq!(snapshot.outputs.state, OutcomeState::Complete);
    assert_eq!(
        snapshot.inputs.parts[0].volume_mm3,
        Some("2400".parse().expect("the manual volume")),
        "the manual volume is what the calculation used"
    );
    assert_eq!(snapshot.outputs.parts[0].per_part_subtotal, Some("1.1388016".parse().expect("the per-part subtotal")));
    harness.frames(2);
}

/// Two customer revisions are two reviewed records. Editing B leaves C untouched, and both survive
/// a save and a restart reopen with their own definition and part identity.
#[test]
fn two_customer_revisions_keep_separate_part_records_across_a_restart() {
    let store = Arc::new(Mutex::new(FakeStore::default()));
    let (mut harness, _host) = fake_harness(&store);
    import_fixture(&mut harness, "revisions_bc.step");
    let draft = harness.app.session.solidbom.draft.clone().expect("the reviewed draft");
    assert_eq!(draft.parts.len(), 2);
    assert_eq!(draft.analysis.definitions.len(), 2);
    let b = draft.parts[0].part_version_id;
    let c = draft.parts[1].part_version_id;
    assert_ne!(b, c, "each source definition has its own part record");
    assert_ne!(draft.parts[0].definition_id, draft.parts[1].definition_id);
    let untouched = draft.parts[1].clone();
    review(
        &mut harness,
        b,
        json!({
            "classification": "clip B",
            "classification_confirmed": true,
            "volume_confirmed": true,
            "material_version_id": MATERIAL,
            "allowance_rate": "0.02",
            "cycle_seconds": "30",
            "cavities": 1,
            "production_rate_version_id": RATE,
            "setup_hours": "0.5",
            "setup_rate_version_id": RATE,
            "material_overhead_rate": "0.05"
        }),
    );
    let edited = harness.app.session.solidbom.draft.clone().expect("the edited draft");
    let b_now = edited.parts.iter().find(|part| part.part_version_id == b).expect("part B");
    assert_eq!(b_now.classification, "clip B");
    assert_eq!(b_now.material_version_id, Some(material_id()));
    let c_now = edited.parts.iter().find(|part| part.part_version_id == c).expect("part C");
    assert_eq!(c_now, &untouched, "editing B leaves C unchanged");
    harness.app.run("solidbom.save", json!({})).expect("the reviewed revision project saves");
    pump_until(&mut harness, "the saved revision project", |harness| {
        harness.app.session.solidbom.save == SaveState::Clean && harness.app.session.solidbom.version() == 2
    });
    harness.frames(2);
    let project_id = edited.project_id;
    let occurrences = edited.analysis.occurrences.clone();

    drop(harness);
    let (mut reopened, _host) = fake_harness(&store);
    reopen_project(&mut reopened, project_id);
    let draft = reopened.app.session.solidbom.draft.as_ref().expect("the reopened draft");
    assert_eq!(draft.parts.len(), 2);
    assert_eq!(draft.analysis.definitions.len(), 2);
    assert_eq!(draft.analysis.occurrences, occurrences);
    let b_now = draft.parts.iter().find(|part| part.part_version_id == b).expect("part B survived the restart");
    assert_eq!(b_now.classification, "clip B");
    assert_eq!(b_now.material_version_id, Some(material_id()));
    assert!(draft.analysis.definitions.iter().any(|definition| definition.definition_id == b_now.definition_id));
    let c_now = draft.parts.iter().find(|part| part.part_version_id == c).expect("part C survived the restart");
    assert_eq!(c_now, &untouched, "the untouched revision is unchanged");
    assert_ne!(b_now.part_version_id, c_now.part_version_id);
    let runtime = reopened.app.session.solidbom.runtime.as_ref().expect("the verified runtime mapping");
    for occurrence in &draft.analysis.occurrences {
        assert!(runtime.runtime_occurrence(&occurrence.source_path).is_ok(), "every saved path still maps: {:?}", occurrence.source_path);
    }
}

/// A flat file has no usable product hierarchy: the guard reports `unresolved_grouping`, the result
/// is incomplete even with complete numeric inputs, and no component identity is invented.
#[test]
fn a_flat_file_without_a_hierarchy_is_never_a_complete_cost() {
    let store = Arc::new(Mutex::new(FakeStore::default()));
    let (mut harness, _host) = fake_harness(&store);
    import_fixture(&mut harness, "flat_multi_body.step");
    let draft = harness.app.session.solidbom.draft.clone().expect("the reviewed draft");
    assert_eq!(draft.analysis.definitions.len(), 1, "no component identity is invented");
    assert_eq!(draft.parts.len(), 1);
    assert_eq!(draft.analysis.occurrences.len(), 1);
    assert_eq!(draft.parts[0].definition_id, draft.analysis.definitions[0].definition_id);
    let issue = draft.analysis.issues.iter().find(|issue| issue.code == "unresolved_grouping").expect("the flat guard reports the grouping");
    assert_eq!(issue.definition_id, Some(draft.analysis.definitions[0].definition_id));
    let part = draft.parts[0].part_version_id;
    review(
        &mut harness,
        part,
        json!({
            "classification": "flat body",
            "classification_confirmed": true,
            "volume_confirmed": true,
            "material_version_id": MATERIAL,
            "allowance_rate": "0.02",
            "cycle_seconds": "30",
            "cavities": 1,
            "production_rate_version_id": RATE,
            "setup_hours": "0.5",
            "setup_rate_version_id": RATE,
            "material_overhead_rate": "0.05"
        }),
    );
    calculate(&mut harness);
    let state = &harness.app.session.solidbom;
    assert_eq!(state.result, SolidBomResultState::Incomplete, "a flat unresolved input stays review-required, never a complete cost");
    let snapshot = state.current.as_ref().expect("the incomplete result is readable");
    assert_eq!(snapshot.outputs.state, OutcomeState::Incomplete);
    assert!(snapshot.issues.iter().any(|issue| issue.code == "unresolved_grouping"), "{:?}", snapshot.issues);
    assert_eq!(state.issue_codes().iter().filter(|code| code.as_str() == "unresolved_grouping").count(), 1);
    assert_eq!(state.draft.as_ref().expect("the reviewed draft").parts.len(), 1, "still exactly the one source component");
    harness.frames(2);
}

/// A failed save is never silently retried: the panel offers an explicit retry, which resubmits the
/// identical request id and payload and commits once. A conflict is not offered as a retry.
#[test]
fn a_failed_save_is_retried_with_the_same_identity_and_commits_once() {
    let store = Arc::new(Mutex::new(FakeStore::default()));
    let (mut harness, host) = fake_harness(&store);
    import_fixture(&mut harness, "three_boxes.step");
    review_three_boxes(&mut harness, "30");
    host.fail_next(ClientFailure::Network("the central write was not acknowledged".into()));
    harness.app.run("solidbom.save", json!({})).expect("the save is submitted");
    pump_until(&mut harness, "the failed save", |harness| matches!(harness.app.session.solidbom.save, SaveState::SaveFailed { .. }));
    let failed = harness.app.session.solidbom.failed.clone().expect("the failed request is retained");
    assert!(failed.retryable, "a network failure may be retried unchanged");
    assert_eq!(harness.app.session.solidbom.version(), 1, "nothing was committed");
    assert!(!harness.app.session.solidbom.needs_service, "an unreachable service report is not sticky for this session");
    assert_eq!(state_of(&mut harness)["needs_service"], json!(false));
    let failed_save = match &failed.request {
        ClientRequest::SaveDraft(save) => save.clone(),
        other => panic!("not a save: {other:?}"),
    };
    assert_eq!(store.lock().expect("the store").projects[&failed_save.draft.project_id].version, 1);
    assert!(
        solidcraft_engine::find_command("solidbom.retry").expect("the retry command").info(&harness.app.session).enabled,
        "an explicit retry is offered for a retryable failure"
    );

    harness.app.run("solidbom.retry", json!({})).expect("the retry is submitted");
    pump_until(&mut harness, "the retried save", |harness| {
        harness.app.session.solidbom.save == SaveState::Clean && harness.app.session.solidbom.version() == 2
    });
    assert_eq!(harness.app.session.solidbom.notice.as_deref(), Some("SolidBOM project saved as version 2"));
    assert!(!harness.app.session.solidbom.needs_service, "the successful retry clears the unreachable-service report");
    let saves: Vec<SaveDraft> = host
        .requests()
        .into_iter()
        .filter_map(|job| match job.request {
            ClientRequest::SaveDraft(save) => Some(save),
            _ => None,
        })
        .collect();
    assert_eq!(saves.len(), 2, "the failed request and its one explicit retry");
    assert_eq!(saves[0], failed_save);
    assert_eq!(saves[1], failed_save, "the retry carries the same request id and payload");
    let saved = store.lock().expect("the store");
    let project = &saved.projects[&failed_save.draft.project_id];
    assert_eq!(saved.projects.len(), 1, "no duplicate project record");
    assert_eq!(project.version, 2);
    assert_eq!(project.committed_saves.len(), 1, "the save committed exactly once");
    drop(saved);

    // A conflict needs a person: it is not retryable and the retry command stays unavailable.
    review_three_boxes(&mut harness, "60");
    host.fail_next(ClientFailure::Conflict("the project changed while saving".into()));
    harness.app.run("solidbom.save", json!({})).expect("the conflicting save is submitted");
    pump_until(&mut harness, "the conflicting save", |harness| matches!(harness.app.session.solidbom.save, SaveState::SaveFailed { .. }));
    let failed = harness.app.session.solidbom.failed.clone().expect("the failed request is retained");
    assert!(!failed.retryable, "a conflict cannot be retried unchanged");
    assert!(harness.app.run("solidbom.retry", json!({})).is_err(), "the retry command refuses a conflict");
    assert!(!solidcraft_engine::find_command("solidbom.retry").expect("the retry command").info(&harness.app.session).enabled);
    assert!(!state_of(&mut harness)["retryable"].as_bool().expect("the retryable flag"));
    assert_eq!(store.lock().expect("the store").projects[&failed_save.draft.project_id].version, 2, "the refused write changed nothing");
}

#[test]
fn a_fake_service_round_trip_reaches_a_current_cost() {
    let store = Arc::new(Mutex::new(FakeStore::default()));
    let (mut harness, _host) = fake_harness(&store);
    import_fixture(&mut harness, "three_boxes.step");
    review_three_boxes(&mut harness, "30");
    calculate(&mut harness);
    assert_eq!(harness.app.session.solidbom.result, SolidBomResultState::Current);
    assert_eq!(harness.app.session.solidbom.notice.as_deref(), Some("SolidBOM project saved as version 2"));
    let snapshot = harness.app.session.solidbom.current.as_ref().expect("the current result");
    assert_eq!(snapshot.outputs.assembly_subtotal, Some("3.4164048".parse().expect("the assembly subtotal")));
    assert_eq!(snapshot.outputs.setup_total, Some("60".parse().expect("the separate setup")));
    harness.frames(2);
}

#[test]
fn a_failed_import_keeps_the_previous_review_and_adds_no_tab() {
    let mut h = crate::scenario::Harness::new();
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/solidbom/fixtures/three_boxes.step");
    h.app.run("solidbom.import", serde_json::json!({"path": path.to_string_lossy()})).expect("the first import");
    h.settle();
    let tabs = crate::documents::tabs_info(&h.app).len();
    let source = h.app.session.solidbom.draft.as_ref().expect("the draft").source.original_name.clone();
    let error = super::panel::open_review_tab(&mut h.app, "solidbom.import", serde_json::json!({"path": "/no/such/file.step"}));
    assert!(error.is_err(), "a missing file is refused");
    assert_eq!(crate::documents::tabs_info(&h.app).len(), tabs, "no tab is added");
    let kept = h.app.session.solidbom.draft.as_ref().expect("the review stays");
    assert_eq!(kept.source.original_name, source);
}

#[test]
fn the_layout_page_exposes_the_legacy_window_preference() {
    let mut h = crate::scenario::Harness::new();
    h.app.home.open = false;
    h.call("ui.prefs", serde_json::json!({"open": "Layout"}));
    h.frames(4);
    assert!(crate::scenario::handle_at("prefs:solidbom_legacy_window").is_some());
}

#[test]
fn review_opens_in_the_workspace_unless_the_legacy_window_is_chosen() {
    let mut workspace = crate::scenario::Harness::new();
    workspace.app.run("solidbom.review", serde_json::json!({})).unwrap();
    workspace.frames(2);
    assert_eq!(crate::workspace::kind(&workspace.app), crate::workspace::WorkspaceKind::Bom);
    assert_eq!(crate::workspace::visible_tabs(&workspace.app), crate::workspace::BOM_TABS);

    let mut legacy = crate::scenario::Harness::new();
    crate::prefs::set(&mut legacy.app, &serde_json::json!({"solidbom_legacy_window": true})).unwrap();
    legacy.app.run("solidbom.review", serde_json::json!({})).unwrap();
    legacy.frames(2);
    assert_eq!(crate::workspace::kind(&legacy.app), crate::workspace::WorkspaceKind::Design);
    assert!(legacy.widget("SolidBOM Review").is_some());
}

#[test]
fn legacy_window_preference_round_trips() {
    let mut h = crate::scenario::Harness::new();
    let reply = crate::prefs::set(&mut h.app, &serde_json::json!({"solidbom_legacy_window": true})).unwrap();
    assert_eq!(reply["solidbom_legacy_window"], true);
    h.app.run("solidbom.review", serde_json::json!({})).unwrap();
    h.frames(2);
    assert!(h.widget("SolidBOM Review").is_some());
}
