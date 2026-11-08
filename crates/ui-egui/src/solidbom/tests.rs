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
    assert!(harness.app.solidbom.is_some());
    harness.frames(2);
}

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use solidbom_contract::{
    CalculationInputs, CalculationOutcome, CalculationSnapshot, MaterialVersion, RateVersion, ReferenceData, ResultState as OutcomeState, SaveReceipt,
};
use solidcraft_engine::solidbom::{ClientFailure, ClientReply, ClientRequest, HostError, JobCompletion, JobId, JobRequest, SolidBomHost};

/// A host that answers every request at once, so a whole review can run without a service.
#[derive(Default)]
struct AutoHost {
    next: AtomicU64,
    queue: Mutex<Vec<JobCompletion>>,
}

impl SolidBomHost for AutoHost {
    fn submit(&self, job: JobRequest) -> Result<JobId, HostError> {
        let job_id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let envelope = job.envelope;
        let result = match job.request {
            ClientRequest::UploadSource { sha256, original_name, bytes } => Ok(ClientReply::Source(SourceRef {
                source_id: uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, sha256.as_bytes()),
                sha256,
                byte_len: bytes.len() as u64,
                original_name,
            })),
            ClientRequest::CreateProject(save) => {
                Ok(ClientReply::Receipt(SaveReceipt { request_id: save.request_id, project_id: save.draft.project_id, version: 1 }))
            }
            ClientRequest::SaveDraft(save) => Ok(ClientReply::Receipt(SaveReceipt {
                request_id: save.request_id,
                project_id: save.draft.project_id,
                version: save.expected_version + 1,
            })),
            ClientRequest::ReferenceData => Ok(ClientReply::ReferenceData(ReferenceData {
                materials: vec![MaterialVersion {
                    material_version_id: uuid::Uuid::parse_str("00000000-0000-4000-8000-000000000001").expect("material"),
                    name: "PA12 test fixture".into(),
                    density_g_cm3: "2.7".parse().expect("density"),
                    price_per_kg: "20".parse().expect("price"),
                    currency: "CNY".into(),
                }],
                rates: vec![RateVersion {
                    rate_version_id: uuid::Uuid::parse_str("00000000-0000-4000-8000-000000000101").expect("rate"),
                    name: "Machining test fixture".into(),
                    rate_per_hour: "120".parse().expect("rate"),
                    currency: "CNY".into(),
                }],
            })),
            ClientRequest::Calculate(request) => Ok(ClientReply::Calculation(CalculationSnapshot {
                calculation_id: uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, request.request_id.as_bytes()),
                project_id: request.project_id,
                saved_version: request.saved_version,
                input_hash: "a".repeat(64),
                calculator_version: "test".into(),
                inputs: CalculationInputs { currency: "CNY".into(), material_overhead_rate: Some("0.05".parse().expect("rate")), parts: Vec::new() },
                outputs: CalculationOutcome {
                    state: OutcomeState::Complete,
                    parts: Vec::new(),
                    assembly_subtotal: Some("3.4164048".parse().expect("total")),
                    setup_total: Some("60".parse().expect("setup")),
                    issues: Vec::new(),
                },
                issues: Vec::new(),
            })),
            other => Err(ClientFailure::Network(format!("{other:?} is not used in this test"))),
        };
        if let Ok(mut queue) = self.queue.lock() {
            queue.push(JobCompletion { job_id, envelope, result });
        }
        Ok(job_id)
    }

    fn poll(&self) -> Vec<JobCompletion> {
        self.queue.lock().map(|mut queue| std::mem::take(&mut *queue)).unwrap_or_default()
    }
}

#[test]
fn a_fake_service_round_trip_reaches_a_current_cost() {
    let mut harness = crate::scenario::Harness::new();
    let host = Arc::new(AutoHost::default());
    harness.app.session.solidbom_host = Some(host);
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/solidbom/fixtures/three_boxes.step");
    harness.app.run("solidbom.import", serde_json::json!({"path": path.to_string_lossy()})).expect("the import");
    for _ in 0..4 {
        harness.app.session.drain_solidbom();
    }
    assert_eq!(harness.app.session.solidbom.version(), 1);
    let part = harness.app.session.solidbom.draft.as_ref().expect("the draft").parts[0].part_version_id.to_string();
    harness
        .app
        .run(
            "solidbom.inputs",
            serde_json::json!({
                "part_version_id": part,
                "classification": "clip",
                "classification_confirmed": true,
                "volume_confirmed": true,
                "material_version_id": "00000000-0000-4000-8000-000000000001",
                "allowance_rate": "0.02",
                "cycle_seconds": "30",
                "cavities": 1,
                "production_rate_version_id": "00000000-0000-4000-8000-000000000101",
                "setup_hours": "0.5",
                "setup_rate_version_id": "00000000-0000-4000-8000-000000000101",
                "material_overhead_rate": "0.05"
            }),
        )
        .expect("the review inputs");
    harness.app.run("solidbom.calculate", serde_json::json!({})).expect("calculate");
    for _ in 0..4 {
        harness.app.session.drain_solidbom();
    }
    assert_eq!(harness.app.session.solidbom.result, solidcraft_engine::solidbom::SolidBomResultState::Current);
    assert_eq!(harness.app.session.solidbom.notice.as_deref(), Some("SolidBOM project saved as version 2"));
    let snapshot = harness.app.session.solidbom.current.as_ref().expect("the current result");
    assert_eq!(snapshot.outputs.assembly_subtotal, Some("3.4164048".parse().expect("total")));
    harness.frames(2);
}

#[test]
fn a_failed_import_keeps_the_previous_review_and_adds_no_tab() {
    let mut h = crate::scenario::Harness::new();
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/solidbom/fixtures/three_boxes.step");
    h.app.run("solidbom.import", serde_json::json!({"path": path.to_string_lossy()})).expect("the first import");
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
