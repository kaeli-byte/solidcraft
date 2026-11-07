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
