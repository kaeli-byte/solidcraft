//! HTTP integration tests for the SolidBOM API.
//!
//! `#[tokio::test]` is not recognised as a test function by Clippy's test allowance, so this
//! file allows the panicking helpers it needs. The library and binary still deny them.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

mod support;

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header};
use solidbom_contract::{
    AcceptedBaseline, AcceptedBaselineList, ActivateRevisions, ActivationReadinessReport, ActivationReadinessRequest, ActivationReceipt, AnalysisKey,
    AnalysisManifest, AnalysisReceipt, AnnualForecast, AppConfigVersionPublished, AttachedCostLine, BaselineCalculationList, BaselineSnapshot,
    CalculateBaseline, CalculateRequest, CalculationSnapshot, CatalogConfirmation, CatalogMatchSuggestion, ClassificationConfirmation,
    ClassificationPrediction, ClassificationSuggestion, CommercialScenarioComparison, CommercialScenarioContent, CommercialScenarioList,
    CommercialScenarioWriteOutcome, CompareCommercialScenarios, CompleteMeasurement, ComponentVersionPublished, ConfirmParts, CostCenterPurpose,
    CostOwner, CostingVolumePolicy, CreateAppConfigVersion, CreateCommercialScenario, CreateComponentVersion, CreateMaterialVersion, CreateProject,
    CreateRateVersion, CycleBasis, CycleOrigin, DefinitionRef, DeliveredPartInput, DeliveredProductInput, DeliveryPackage, DependencyDecisionAction,
    DependencyDecisionItem, DependencyDecisionReceipt, DependencyProjectList, DirectCostInput, DraftProject, DraftPublication, EditAttachedCosts,
    EditRouting, EngineeringFact, EngineeringRecordView, EngineeringWriteOutcome, EvaluateToolingRecovery, ExportBaselineRef,
    ExportBaselineSelection, ExportBundle, ExportFormat, ExportLabel, ExportRequest, GeometryBinding, GroupSolids, Identity, ManualRouting,
    ManufacturingFact, ManufacturingRecordView, ManufacturingWriteOutcome, MaterialConsumption, MaterialVersionPublished, Measurement,
    MeasurementSource, OccurrenceRef, OperationCycle, PartConfirmation, Permission, ProductKind, ProjectCatalogOverride, ProjectCommercial,
    ProjectCosting, ProjectDependencyReport, ProjectDetails, ProjectFact, ProjectRecordView, ProjectTooling, ProjectView, ProjectWriteOutcome,
    PublishRoutingTemplate, PurchasePriceTable, RateVersionPublished, RecalculateDraft, RecordDelivery, RecordDependencyDecisions, RecordSuggestions,
    ReferenceData, RegisterAnalysis, ResultState, ReviewState, ReviewedGeometry, RoutingLevel, RoutingOperation, RoutingWrite, SCHEMA_VERSION,
    SaveDraft, SaveReceipt, SetSourcing, SharedDataView, SharedUpdateKind, SharedUpdateList, SourceRef, SourcedPart, Sourcing, SuggestionEvidence,
    TemplateRef, ToolingEntry, ToolingOutcome, ToolingOwner, ToolingTreatment, Unit, UpdateCommercialScenario, UpdateProduct,
    UpdateProjectCommercial, UpdateProjectCosting, UpdateProjectTooling, YearRange,
};

use solidbom_contract::{
    AcceptFeasibilityException, CloseFeasibilityException, CreateFeasibilityRuleTableVersion, ExceptionState, FeasibilityAssessmentOutcome,
    FeasibilityComponentEvidence, FeasibilityConnector, FeasibilityRecordView, FeasibilityRule, FeasibilityRuleKind, FeasibilityRuleTablePublished,
    FeasibilityRulesView, FeasibilityTubeAttributes, FeasibilityTubeEnd, FeasibilityTubeMeasurements, FeasibilityWriteOutcome, FindingCheck,
    FindingDisposition, FindingState, NumericRangeMm, RecordFeasibilityAssessment, RuleApplicability, RuleLimit, TubeEnd, TubeKind,
};
use solidbom_server::api::{ApiState, router};
use support::{
    ADMIN_TOKEN, ENGINEER_TOKEN, FIXTURE_SOURCE, TestDb, app_config_version, component_version, d, digest, direct_category, fixed_category,
    fixture_draft, material_version, percentage_category, rate_version, row_count, seeded_material_version_id, seeded_rate_version_id,
};
use tower::ServiceExt;
use uuid::Uuid;

const TOKEN: &str = "solidbom-test-token-0123456789";
const BODY_LIMIT: usize = 64 * 1024 * 1024;

fn api(db: &TestDb) -> Router {
    router(ApiState::new(db.store.clone(), TOKEN).with_engineer_token(ENGINEER_TOKEN).with_administrator_token(ADMIN_TOKEN))
}

fn authed(builder: axum::http::request::Builder) -> axum::http::request::Builder {
    authed_as(builder, TOKEN)
}

fn authed_as(builder: axum::http::request::Builder, token: &str) -> axum::http::request::Builder {
    builder.header(header::AUTHORIZATION, format!("Bearer {token}"))
}

fn get(path: &str) -> Request<Body> {
    authed(Request::builder().method("GET").uri(path)).body(Body::empty()).expect("a request")
}

fn json_request(method: &str, path: &str, value: &serde_json::Value) -> Request<Body> {
    authed_as(Request::builder().method(method).uri(path).header(header::CONTENT_TYPE, "application/json"), TOKEN)
        .body(Body::from(serde_json::to_vec(value).expect("the request body")))
        .expect("a request")
}

fn get_as(path: &str, token: &str) -> Request<Body> {
    authed_as(Request::builder().method("GET").uri(path), token).body(Body::empty()).expect("a request")
}

fn json_as(method: &str, path: &str, token: &str, value: &serde_json::Value) -> Request<Body> {
    authed_as(Request::builder().method(method).uri(path).header(header::CONTENT_TYPE, "application/json"), token)
        .body(Body::from(serde_json::to_vec(value).expect("the request body")))
        .expect("a request")
}

fn put_bytes(path: &str, bytes: &[u8]) -> Request<Body> {
    authed(Request::builder().method("PUT").uri(path)).body(Body::from(bytes.to_vec())).expect("a request")
}

async fn send(app: Router, request: Request<Body>) -> (StatusCode, HeaderMap, Vec<u8>) {
    let response = app.oneshot(request).await.expect("the router answers");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), BODY_LIMIT).await.expect("a body");
    (status, headers, bytes.to_vec())
}

fn value(bytes: &[u8]) -> serde_json::Value {
    serde_json::from_slice(bytes).expect("JSON")
}

async fn create_project(app: Router, source: &SourceRef) -> SaveReceipt {
    let save = SaveDraft { request_id: "create-1".into(), expected_version: 0, draft: fixture_draft(0, source) };
    let (status, _, body) = send(app, json_request("POST", "/v1/projects", &serde_json::to_value(&save).expect("save"))).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    serde_json::from_value(value(&body)).expect("a receipt")
}

async fn upload(app: Router) -> SourceRef {
    let path = format!("/v1/sources/{}?name=three_boxes.step", digest(FIXTURE_SOURCE));
    let (status, _, body) = send(app, put_bytes(&path, FIXTURE_SOURCE)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    serde_json::from_value(value(&body)).expect("a source")
}

async fn me(app: Router, token: &str) -> Identity {
    let (status, _, body) = send(app, get_as("/v1/me", token)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    serde_json::from_value(value(&body)).expect("an identity")
}

async fn upload_as_engineer(app: Router) -> SourceRef {
    let path = format!("/v1/sources/{}?name=three_boxes.step", digest(FIXTURE_SOURCE));
    let request =
        authed_as(Request::builder().method("PUT").uri(&path), ENGINEER_TOKEN).body(Body::from(FIXTURE_SOURCE.to_vec())).expect("a request");
    let (status, _, body) = send(app, request).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    serde_json::from_value(value(&body)).expect("a source")
}

async fn publish_material(app: Router, token: &str, request: &CreateMaterialVersion) -> (StatusCode, Vec<u8>) {
    let path = format!("/v1/shared/materials/{}/versions", request.material_id);
    let (status, _, body) = send(app, json_as("POST", &path, token, &serde_json::to_value(request).expect("a material version request"))).await;
    (status, body)
}

async fn publish_rate(app: Router, token: &str, request: &CreateRateVersion) -> (StatusCode, Vec<u8>) {
    let path = format!("/v1/shared/cost-centers/{}/versions", request.cost_center_id);
    let (status, _, body) = send(app, json_as("POST", &path, token, &serde_json::to_value(request).expect("a rate version request"))).await;
    (status, body)
}

async fn publish_component(app: Router, token: &str, request: &CreateComponentVersion) -> (StatusCode, Vec<u8>) {
    let path = format!("/v1/shared/components/{}/versions", request.component_id);
    let (status, _, body) = send(app, json_as("POST", &path, token, &serde_json::to_value(request).expect("a component version request"))).await;
    (status, body)
}

async fn publish_app_config(app: Router, token: &str, request: &CreateAppConfigVersion) -> (StatusCode, Vec<u8>) {
    let path = format!("/v1/shared/app-config/{}/versions", request.config_id);
    let (status, _, body) = send(app, json_as("POST", &path, token, &serde_json::to_value(request).expect("an app config request"))).await;
    (status, body)
}

async fn create_with_draft(app: Router, request_id: &str, mut draft: DraftProject) -> SaveReceipt {
    draft.project_id = Uuid::new_v4();
    let save = SaveDraft { request_id: request_id.into(), expected_version: 0, draft };
    let (status, _, body) = send(app, json_request("POST", "/v1/projects", &serde_json::to_value(&save).expect("save"))).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    serde_json::from_value(value(&body)).expect("a receipt")
}

async fn calculate(app: Router, request_id: &str, project_id: Uuid, saved_version: u64) -> CalculationSnapshot {
    let request = CalculateRequest { request_id: request_id.into(), project_id, saved_version };
    let (status, _, body) =
        send(app, json_request("POST", &format!("/v1/projects/{project_id}/calculations"), &serde_json::to_value(&request).expect("request"))).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    serde_json::from_value(value(&body)).expect("a snapshot")
}

async fn reference_data(app: Router, token: &str) -> ReferenceData {
    let (status, _, body) = send(app, get_as("/v1/reference-data", token)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    serde_json::from_value(value(&body)).expect("reference data")
}

async fn shared_view(app: Router, token: &str) -> SharedDataView {
    let (status, _, body) = send(app, get_as("/v1/shared", token)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    serde_json::from_value(value(&body)).expect("shared data")
}

#[tokio::test]
async fn health_needs_no_token() {
    let db = TestDb::new().await;
    let request = Request::builder().method("GET").uri("/health").body(Body::empty()).expect("a request");
    let (status, _, body) = send(api(&db), request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value(&body), serde_json::json!({"ok": true}));
    db.cleanup().await;
}

#[tokio::test]
async fn the_api_refuses_a_missing_or_wrong_token() {
    let db = TestDb::new().await;
    let app = api(&db);
    let no_token = Request::builder().method("GET").uri("/v1/reference-data").body(Body::empty()).expect("a request");
    assert_eq!(send(app.clone(), no_token).await.0, StatusCode::UNAUTHORIZED);
    let wrong = Request::builder()
        .method("GET")
        .uri("/v1/reference-data")
        .header(header::AUTHORIZATION, "Bearer not-the-token")
        .body(Body::empty())
        .expect("a request");
    assert_eq!(send(app.clone(), wrong).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(send(app, get("/v1/reference-data")).await.0, StatusCode::OK);
    db.cleanup().await;
}

#[tokio::test]
async fn an_upload_is_checked_stored_once_and_read_back() {
    let db = TestDb::new().await;
    let app = api(&db);
    let source = upload(app.clone()).await;
    assert_eq!(source.sha256, digest(FIXTURE_SOURCE));
    assert_eq!(source.byte_len, FIXTURE_SOURCE.len() as u64);
    assert_eq!(source.original_name, "three_boxes.step");
    let again = upload(app.clone()).await;
    assert_eq!(source.source_id, again.source_id);

    let (status, headers, bytes) = send(app.clone(), get(&format!("/v1/sources/{}", source.source_id))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, FIXTURE_SOURCE);
    assert_eq!(headers.get("x-solidbom-sha256").and_then(|v| v.to_str().ok()), Some(source.sha256.as_str()));

    let wrong = format!("/v1/sources/{}?name=three_boxes.step", "0".repeat(64));
    let (status, _, _) = send(app, put_bytes(&wrong, FIXTURE_SOURCE)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    db.cleanup().await;
}

#[tokio::test]
async fn an_oversized_upload_is_refused() {
    let db = TestDb::new().await;
    let app = router(ApiState::new(db.store.clone(), TOKEN).with_max_upload(8));
    let bytes = vec![0u8; 16];
    let path = format!("/v1/sources/{}?name=large.step", digest(&bytes));
    let (status, _, _) = send(app, put_bytes(&path, &bytes)).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    db.cleanup().await;
}

#[tokio::test]
async fn reference_data_serves_the_seeded_values() {
    let db = TestDb::new().await;
    let (status, _, body) = send(api(&db), get("/v1/reference-data")).await;
    assert_eq!(status, StatusCode::OK);
    let data: ReferenceData = serde_json::from_value(value(&body)).expect("reference data");
    data.validate().expect("the seed is valid");
    assert!(data.materials.iter().any(|material| material.density_g_cm3 == d("2.7") && material.price_per_kg == Some(d("20"))));
    assert!(data.rates.iter().any(|rate| rate.rate_per_hour == Some(d("120"))));
    db.cleanup().await;
}

#[tokio::test]
async fn a_project_is_created_saved_and_reopened_over_http() {
    let db = TestDb::new().await;
    let app = api(&db);
    let source = upload(app.clone()).await;
    let receipt = create_project(app.clone(), &source).await;
    assert_eq!(receipt.version, 1);
    let project_id = receipt.project_id;

    let (status, _, body) = send(app.clone(), get(&format!("/v1/projects/{project_id}"))).await;
    assert_eq!(status, StatusCode::OK);
    let view: ProjectView = serde_json::from_value(value(&body)).expect("a project view");
    assert_eq!(view.draft.version, 1);
    assert_eq!(view.draft.analysis.occurrences.len(), 3);
    assert_eq!(view.calculations.len(), 0);

    let save = SaveDraft { request_id: "save-1".into(), expected_version: 1, draft: fixture_draft(1, &source) };
    let (status, _, body) =
        send(app.clone(), json_request("PUT", &format!("/v1/projects/{project_id}/draft"), &serde_json::to_value(&save).expect("save"))).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let receipt: SaveReceipt = serde_json::from_value(value(&body)).expect("a receipt");
    assert_eq!(receipt.version, 2);
    let (_, _, body) = send(app, get(&format!("/v1/projects/{project_id}"))).await;
    let view: ProjectView = serde_json::from_value(value(&body)).expect("a project view");
    assert_eq!(view.draft.version, 2);
    db.cleanup().await;
}

#[tokio::test]
async fn a_stale_save_is_a_conflict() {
    let db = TestDb::new().await;
    let app = api(&db);
    let source = upload(app.clone()).await;
    let receipt = create_project(app.clone(), &source).await;
    let project_id = receipt.project_id;
    let save = SaveDraft { request_id: "save-1".into(), expected_version: 1, draft: fixture_draft(1, &source) };
    let body = serde_json::to_value(&save).expect("save");
    let (status, _, _) = send(app.clone(), json_request("PUT", &format!("/v1/projects/{project_id}/draft"), &body)).await;
    assert_eq!(status, StatusCode::OK);
    let stale = SaveDraft { request_id: "save-2".into(), expected_version: 1, draft: fixture_draft(1, &source) };
    let (status, _, _) =
        send(app, json_request("PUT", &format!("/v1/projects/{project_id}/draft"), &serde_json::to_value(&stale).expect("save"))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    db.cleanup().await;
}

#[tokio::test]
async fn a_lost_save_reply_returns_the_same_receipt() {
    let db = TestDb::new().await;
    let app = api(&db);
    let source = upload(app.clone()).await;
    let receipt = create_project(app.clone(), &source).await;
    let project_id = receipt.project_id;
    let save = SaveDraft { request_id: "save-1".into(), expected_version: 1, draft: fixture_draft(1, &source) };
    let body = serde_json::to_value(&save).expect("save");
    let path = format!("/v1/projects/{project_id}/draft");
    let (first_status, _, first) = send(app.clone(), json_request("PUT", &path, &body)).await;
    let (second_status, _, second) = send(app, json_request("PUT", &path, &body)).await;
    assert_eq!(first_status, StatusCode::OK);
    assert_eq!(second_status, StatusCode::OK);
    assert_eq!(value(&first), value(&second));
    assert_eq!(db.store.version_count(project_id).await.expect("count"), 2);
    db.cleanup().await;
}

#[tokio::test]
async fn a_calculation_returns_the_approved_fixture_numbers() {
    let db = TestDb::new().await;
    let app = api(&db);
    let source = upload(app.clone()).await;
    let receipt = create_project(app.clone(), &source).await;
    let project_id = receipt.project_id;
    let request = CalculateRequest { request_id: "calc-1".into(), project_id, saved_version: 1 };
    let (status, _, body) = send(
        app.clone(),
        json_request("POST", &format!("/v1/projects/{project_id}/calculations"), &serde_json::to_value(&request).expect("request")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let snapshot: CalculationSnapshot = serde_json::from_value(value(&body)).expect("a snapshot");
    assert_eq!(snapshot.outputs.state, ResultState::Complete, "{:?}", snapshot.issues);
    assert!(snapshot.issues.is_empty());
    let part = &snapshot.outputs.parts[0];
    assert_eq!(part.net_mass_kg, Some(d("0.00648")));
    assert_eq!(part.allowed_material_cost, Some(d("0.132192")));
    assert_eq!(part.material_overhead, Some(d("0.0066096")));
    assert_eq!(part.production_cost, Some(d("1")));
    assert_eq!(part.per_part_subtotal, Some(d("1.1388016")));
    assert_eq!(part.line_total, Some(d("3.4164048")));
    assert_eq!(snapshot.outputs.assembly_subtotal, Some(d("3.4164048")));
    assert_eq!(snapshot.outputs.setup_total, Some(d("60")));
    assert_eq!(snapshot.saved_version, 1);
    assert_eq!(snapshot.calculator_version, "solidbom-direct-cost/1");

    let (status, _, again) = send(app, get(&format!("/v1/calculations/{}", snapshot.calculation_id))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value(&again), value(&body));
    db.cleanup().await;
}

#[tokio::test]
async fn a_calculation_on_a_historical_version_stays_readable() {
    let db = TestDb::new().await;
    let app = api(&db);
    let source = upload(app.clone()).await;
    let receipt = create_project(app.clone(), &source).await;
    let project_id = receipt.project_id;
    let first_request = CalculateRequest { request_id: "calc-1".into(), project_id, saved_version: 1 };
    let (status, _, first_body) = send(
        app.clone(),
        json_request("POST", &format!("/v1/projects/{project_id}/calculations"), &serde_json::to_value(&first_request).expect("request")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let first: CalculationSnapshot = serde_json::from_value(value(&first_body)).expect("a snapshot");

    let mut second_draft = fixture_draft(1, &source);
    second_draft.parts[0].operation.as_mut().expect("operation").cycle_seconds = Some(d("60"));
    let save = SaveDraft { request_id: "save-2".into(), expected_version: 1, draft: second_draft };
    let (status, _, _) =
        send(app.clone(), json_request("PUT", &format!("/v1/projects/{project_id}/draft"), &serde_json::to_value(&save).expect("save"))).await;
    assert_eq!(status, StatusCode::OK);
    let second_request = CalculateRequest { request_id: "calc-2".into(), project_id, saved_version: 2 };
    let (status, _, second_body) = send(
        app.clone(),
        json_request("POST", &format!("/v1/projects/{project_id}/calculations"), &serde_json::to_value(&second_request).expect("request")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let second: CalculationSnapshot = serde_json::from_value(value(&second_body)).expect("a snapshot");
    assert_eq!(second.outputs.parts[0].production_cost, Some(d("2")));
    assert_eq!(second.outputs.assembly_subtotal, Some(d("6.4164048")));

    let (status, _, untouched) = send(app.clone(), get(&format!("/v1/calculations/{}", first.calculation_id))).await;
    assert_eq!(status, StatusCode::OK);
    let untouched: CalculationSnapshot = serde_json::from_value(value(&untouched)).expect("a snapshot");
    assert_eq!(untouched.outputs.assembly_subtotal, Some(d("3.4164048")));
    assert_eq!(untouched.saved_version, 1);

    let (_, _, body) = send(app, get(&format!("/v1/projects/{project_id}"))).await;
    let view: ProjectView = serde_json::from_value(value(&body)).expect("a project view");
    assert_eq!(view.draft.version, 2);
    assert_eq!(view.draft.source.source_id, source.source_id);
    assert_eq!(view.draft.source.sha256, digest(FIXTURE_SOURCE));
    assert_eq!(view.calculations.len(), 2);
    assert_eq!(view.calculations.iter().map(|summary| summary.saved_version).collect::<Vec<_>>(), [1, 2]);
    // The historical snapshot keeps the inputs and the reviewed definition it computed.
    assert_eq!(first.saved_version, 1);
    assert_eq!(first.inputs.parts[0].cycle_seconds, Some(d("30")));
    assert_eq!(second.inputs.parts[0].cycle_seconds, Some(d("60")));
    assert_eq!(first.inputs.parts[0].part_version_id, view.draft.parts[0].part_version_id);
    assert_eq!(first.calculator_version, "solidbom-direct-cost/1");
    assert_ne!(first.input_hash, second.input_hash);
    db.cleanup().await;
}

#[tokio::test]
async fn a_repeated_calculation_request_returns_its_snapshot() {
    let db = TestDb::new().await;
    let app = api(&db);
    let source = upload(app.clone()).await;
    let receipt = create_project(app.clone(), &source).await;
    let project_id = receipt.project_id;
    let request = CalculateRequest { request_id: "calc-1".into(), project_id, saved_version: 1 };
    let body = serde_json::to_value(&request).expect("request");
    let path = format!("/v1/projects/{project_id}/calculations");
    let (_, _, first) = send(app.clone(), json_request("POST", &path, &body)).await;
    let (_, _, second) = send(app, json_request("POST", &path, &body)).await;
    assert_eq!(value(&first), value(&second));
    let (_, _, view) = send(api(&db), get(&format!("/v1/projects/{project_id}"))).await;
    let view: ProjectView = serde_json::from_value(value(&view)).expect("a project view");
    assert_eq!(view.calculations.len(), 1);
    db.cleanup().await;
}

#[tokio::test]
async fn an_unconfirmed_classification_makes_the_result_incomplete() {
    let db = TestDb::new().await;
    let app = api(&db);
    let source = upload(app.clone()).await;
    let mut draft = fixture_draft(0, &source);
    draft.parts[0].classification_confirmed = false;
    let create = SaveDraft { request_id: "create-1".into(), expected_version: 0, draft };
    let (status, _, body) = send(app.clone(), json_request("POST", "/v1/projects", &serde_json::to_value(&create).expect("create"))).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let receipt: SaveReceipt = serde_json::from_value(value(&body)).expect("a receipt");
    let request = CalculateRequest { request_id: "calc-1".into(), project_id: receipt.project_id, saved_version: 1 };
    let (status, _, body) = send(
        app,
        json_request("POST", &format!("/v1/projects/{}/calculations", receipt.project_id), &serde_json::to_value(&request).expect("request")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let snapshot: CalculationSnapshot = serde_json::from_value(value(&body)).expect("a snapshot");
    assert_eq!(snapshot.outputs.state, ResultState::Incomplete);
    assert!(snapshot.issues.iter().any(|issue| issue.code == "unconfirmed_classification"), "{:?}", snapshot.issues);
    db.cleanup().await;
}

#[tokio::test]
async fn a_missing_material_version_makes_the_result_incomplete() {
    let db = TestDb::new().await;
    let app = api(&db);
    let source = upload(app.clone()).await;
    let mut draft = fixture_draft(0, &source);
    draft.parts[0].material_version_id = None;
    let create = SaveDraft { request_id: "create-1".into(), expected_version: 0, draft };
    let (status, _, body) = send(app.clone(), json_request("POST", "/v1/projects", &serde_json::to_value(&create).expect("create"))).await;
    assert_eq!(status, StatusCode::CREATED);
    let receipt: SaveReceipt = serde_json::from_value(value(&body)).expect("a receipt");
    let request = CalculateRequest { request_id: "calc-1".into(), project_id: receipt.project_id, saved_version: 1 };
    let (_, _, body) = send(
        app,
        json_request("POST", &format!("/v1/projects/{}/calculations", receipt.project_id), &serde_json::to_value(&request).expect("request")),
    )
    .await;
    let snapshot: CalculationSnapshot = serde_json::from_value(value(&body)).expect("a snapshot");
    assert_eq!(snapshot.outputs.state, ResultState::Incomplete);
    assert_eq!(snapshot.outputs.parts[0].allowed_material_cost, None);
    assert!(snapshot.issues.iter().any(|issue| issue.code == "missing_density"), "{:?}", snapshot.issues);
    db.cleanup().await;
}

#[tokio::test]
async fn an_invalid_contract_is_unprocessable() {
    let db = TestDb::new().await;
    let app = api(&db);
    let source = upload(app.clone()).await;
    let mut save = SaveDraft { request_id: "create-1".into(), expected_version: 3, draft: fixture_draft(0, &source) };
    save.draft.version = 0;
    let (status, _, _) = send(app, json_request("POST", "/v1/projects", &serde_json::to_value(&save).expect("save"))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    db.cleanup().await;
}

#[tokio::test]
async fn an_unresolved_grouping_makes_the_result_incomplete() {
    let db = TestDb::new().await;
    let app = api(&db);
    let source = upload(app.clone()).await;
    let save = SaveDraft { request_id: "create-unresolved".into(), expected_version: 0, draft: support::unresolved_draft(0, &source) };
    let (status, _, body) = send(app.clone(), json_request("POST", "/v1/projects", &serde_json::to_value(&save).expect("save"))).await;
    assert_eq!(status, StatusCode::CREATED);
    let receipt: SaveReceipt = serde_json::from_value(value(&body)).expect("a receipt");
    let request = CalculateRequest { request_id: "calc-unresolved".into(), project_id: receipt.project_id, saved_version: 1 };
    let (status, _, body) = send(
        app,
        json_request("POST", &format!("/v1/projects/{}/calculations", receipt.project_id), &serde_json::to_value(&request).expect("request")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let snapshot: CalculationSnapshot = serde_json::from_value(value(&body)).expect("a snapshot");
    assert_eq!(snapshot.outputs.state, ResultState::Incomplete);
    assert!(snapshot.issues.iter().any(|issue| issue.code == "unresolved_grouping"), "{:?}", snapshot.issues);
    db.cleanup().await;
}

#[tokio::test]
async fn a_lost_create_reply_returns_the_same_receipt_without_duplicates() {
    let db = TestDb::new().await;
    let app = api(&db);
    let source = upload(app.clone()).await;
    let save = SaveDraft { request_id: "create-1".into(), expected_version: 0, draft: fixture_draft(0, &source) };
    let body = serde_json::to_value(&save).expect("save");
    let (first_status, _, first) = send(app.clone(), json_request("POST", "/v1/projects", &body)).await;
    let (second_status, _, second) = send(app, json_request("POST", "/v1/projects", &body)).await;
    assert_eq!(first_status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&first));
    assert_eq!(second_status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&second));
    assert_eq!(value(&first), value(&second));
    assert_eq!(row_count(&db.store, "projects").await, 1);
    assert_eq!(row_count(&db.store, "project_versions").await, 1);
    assert_eq!(row_count(&db.store, "requests").await, 1);
    db.cleanup().await;
}

#[tokio::test]
async fn a_changed_create_payload_under_a_reused_request_id_conflicts() {
    let db = TestDb::new().await;
    let app = api(&db);
    let source = upload(app.clone()).await;
    let save = SaveDraft { request_id: "create-1".into(), expected_version: 0, draft: fixture_draft(0, &source) };
    let (status, _, body) = send(app.clone(), json_request("POST", "/v1/projects", &serde_json::to_value(&save).expect("save"))).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));

    let mut changed = save.clone();
    changed.draft.parts[0].operation.as_mut().expect("operation").cycle_seconds = Some(d("60"));
    let (status, _, body) = send(app, json_request("POST", "/v1/projects", &serde_json::to_value(&changed).expect("save"))).await;
    assert_eq!(status, StatusCode::CONFLICT, "{}", String::from_utf8_lossy(&body));
    assert_eq!(value(&body)["error"]["code"], "conflict");
    assert_eq!(row_count(&db.store, "projects").await, 1);
    assert_eq!(row_count(&db.store, "project_versions").await, 1);
    assert_eq!(row_count(&db.store, "requests").await, 1);
    db.cleanup().await;
}

#[tokio::test]
async fn a_second_create_for_an_existing_project_id_conflicts() {
    let db = TestDb::new().await;
    let app = api(&db);
    let source = upload(app.clone()).await;
    let save = SaveDraft { request_id: "create-1".into(), expected_version: 0, draft: fixture_draft(0, &source) };
    let (status, _, _) = send(app.clone(), json_request("POST", "/v1/projects", &serde_json::to_value(&save).expect("save"))).await;
    assert_eq!(status, StatusCode::CREATED);

    let mut second = save.clone();
    second.request_id = "create-2".into();
    let (status, _, body) = send(app, json_request("POST", "/v1/projects", &serde_json::to_value(&second).expect("save"))).await;
    assert_eq!(status, StatusCode::CONFLICT, "{}", String::from_utf8_lossy(&body));
    assert_eq!(row_count(&db.store, "projects").await, 1);
    db.cleanup().await;
}

#[tokio::test]
async fn a_changed_calculation_payload_under_a_reused_request_id_conflicts() {
    let db = TestDb::new().await;
    let app = api(&db);
    let source = upload(app.clone()).await;
    let receipt = create_project(app.clone(), &source).await;
    let project_id = receipt.project_id;
    let first = CalculateRequest { request_id: "calc-1".into(), project_id, saved_version: 1 };
    let (status, _, body) =
        send(app.clone(), json_request("POST", &format!("/v1/projects/{project_id}/calculations"), &serde_json::to_value(&first).expect("request")))
            .await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));

    let save = SaveDraft { request_id: "save-1".into(), expected_version: 1, draft: fixture_draft(1, &source) };
    let (status, _, _) =
        send(app.clone(), json_request("PUT", &format!("/v1/projects/{project_id}/draft"), &serde_json::to_value(&save).expect("save"))).await;
    assert_eq!(status, StatusCode::OK);

    // The same request id now names another version, so its payload hash differs.
    let changed = CalculateRequest { request_id: "calc-1".into(), project_id, saved_version: 2 };
    let (status, _, body) =
        send(app, json_request("POST", &format!("/v1/projects/{project_id}/calculations"), &serde_json::to_value(&changed).expect("request"))).await;
    assert_eq!(status, StatusCode::CONFLICT, "{}", String::from_utf8_lossy(&body));
    assert_eq!(row_count(&db.store, "calculations").await, 1);
    db.cleanup().await;
}

#[tokio::test]
async fn malformed_json_is_refused_before_any_write() {
    let db = TestDb::new().await;
    let request = authed(Request::builder().method("POST").uri("/v1/projects").header(header::CONTENT_TYPE, "application/json"))
        .body(Body::from(b"{\"request_id\": ".to_vec()))
        .expect("a request");
    let (status, _, body) = send(api(&db), request).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(!String::from_utf8_lossy(&body).contains(TOKEN), "the refusal must not echo the token");
    assert_eq!(row_count(&db.store, "projects").await, 0);
    assert_eq!(row_count(&db.store, "project_versions").await, 0);
    assert_eq!(row_count(&db.store, "requests").await, 0);
    db.cleanup().await;
}

#[tokio::test]
async fn a_request_that_outlasts_the_bound_returns_a_retryable_internal_error() {
    let db = TestDb::new().await;
    let app = router(ApiState::new(db.store.clone(), TOKEN).with_delay(Duration::from_millis(200)).with_request_timeout(Duration::from_millis(20)));
    let (status, _, body) = send(app, get("/v1/reference-data")).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(value(&body)["error"]["code"], "internal");
    assert!(!String::from_utf8_lossy(&body).contains(TOKEN), "the refusal must not echo the token");
    db.cleanup().await;
}

#[tokio::test]
async fn a_reconnected_service_reads_the_committed_project() {
    let db = TestDb::new().await;
    let app = api(&db);
    let source = upload(app.clone()).await;
    let receipt = create_project(app.clone(), &source).await;
    let project_id = receipt.project_id;
    let request = CalculateRequest { request_id: "calc-1".into(), project_id, saved_version: 1 };
    let (status, _, _) =
        send(app, json_request("POST", &format!("/v1/projects/{project_id}/calculations"), &serde_json::to_value(&request).expect("request"))).await;
    assert_eq!(status, StatusCode::CREATED);

    // A new pool on the same schema is what a restarted service does.
    let reopened = router(ApiState::new(db.reconnect().await, TOKEN));
    let (status, _, body) = send(reopened.clone(), get(&format!("/v1/projects/{project_id}"))).await;
    assert_eq!(status, StatusCode::OK);
    let view: ProjectView = serde_json::from_value(value(&body)).expect("a project view");
    assert_eq!(view.draft.version, 1);
    assert_eq!(view.draft.source.sha256, digest(FIXTURE_SOURCE));
    assert_eq!(view.calculations.len(), 1);
    let calculation_id = view.calculations[0].calculation_id;
    let (status, _, body) = send(reopened, get(&format!("/v1/calculations/{calculation_id}"))).await;
    assert_eq!(status, StatusCode::OK);
    let snapshot: CalculationSnapshot = serde_json::from_value(value(&body)).expect("a snapshot");
    assert_eq!(snapshot.saved_version, 1);
    assert_eq!(snapshot.outputs.assembly_subtotal, Some(d("3.4164048")));
    db.cleanup().await;
}

#[tokio::test]
async fn two_cavities_and_a_reviewed_manual_volume_follow_the_sb02_vectors() {
    let db = TestDb::new().await;
    let app = api(&db);
    let source = upload(app.clone()).await;

    let cavities = SaveDraft { request_id: "create-cavities".into(), expected_version: 0, draft: support::two_cavity_draft(0, &source) };
    let (status, _, body) = send(app.clone(), json_request("POST", "/v1/projects", &serde_json::to_value(&cavities).expect("save"))).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let cavities_receipt: SaveReceipt = serde_json::from_value(value(&body)).expect("a receipt");
    let request = CalculateRequest { request_id: "calc-cavities".into(), project_id: cavities_receipt.project_id, saved_version: 1 };
    let (status, _, body) = send(
        app.clone(),
        json_request(
            "POST",
            &format!("/v1/projects/{}/calculations", cavities_receipt.project_id),
            &serde_json::to_value(&request).expect("request"),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let snapshot: CalculationSnapshot = serde_json::from_value(value(&body)).expect("a snapshot");
    assert_eq!(snapshot.outputs.parts[0].production_cost, Some(d("0.5")));
    assert_eq!(snapshot.outputs.parts[0].per_part_subtotal, Some(d("0.6388016")));
    assert_eq!(snapshot.outputs.assembly_subtotal, Some(d("1.9164048")));

    let mut manual = support::manual_volume_draft(0, &source);
    manual.project_id = Uuid::new_v4();
    let save = SaveDraft { request_id: "create-manual".into(), expected_version: 0, draft: manual };
    let (status, _, body) = send(app.clone(), json_request("POST", "/v1/projects", &serde_json::to_value(&save).expect("save"))).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let manual_receipt: SaveReceipt = serde_json::from_value(value(&body)).expect("a receipt");
    let request = CalculateRequest { request_id: "calc-manual".into(), project_id: manual_receipt.project_id, saved_version: 1 };
    let (_, _, body) = send(
        app,
        json_request("POST", &format!("/v1/projects/{}/calculations", manual_receipt.project_id), &serde_json::to_value(&request).expect("request")),
    )
    .await;
    let snapshot: CalculationSnapshot = serde_json::from_value(value(&body)).expect("a snapshot");
    let part = &snapshot.outputs.parts[0];
    assert_eq!(part.net_mass_kg, Some(d("0.00675")));
    assert_eq!(part.allowed_material_cost, Some(d("0.1377")));
    assert_eq!(part.material_overhead, Some(d("0.006885")));
    assert_eq!(part.per_part_subtotal, Some(d("1.144585")));
    assert_eq!(snapshot.outputs.assembly_subtotal, Some(d("3.433755")));
    db.cleanup().await;
}

#[tokio::test]
async fn a_missing_cycle_time_is_named_and_withholds_a_complete_subtotal() {
    let db = TestDb::new().await;
    let app = api(&db);
    let source = upload(app.clone()).await;
    let save = SaveDraft { request_id: "create-missing".into(), expected_version: 0, draft: support::missing_cycle_draft(0, &source) };
    let (status, _, body) = send(app.clone(), json_request("POST", "/v1/projects", &serde_json::to_value(&save).expect("save"))).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let receipt: SaveReceipt = serde_json::from_value(value(&body)).expect("a receipt");
    let request = CalculateRequest { request_id: "calc-missing".into(), project_id: receipt.project_id, saved_version: 1 };
    let (status, _, body) = send(
        app,
        json_request("POST", &format!("/v1/projects/{}/calculations", receipt.project_id), &serde_json::to_value(&request).expect("request")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let snapshot: CalculationSnapshot = serde_json::from_value(value(&body)).expect("a snapshot");
    assert_eq!(snapshot.outputs.state, ResultState::Incomplete);
    for code in ["missing_cycle_time", "missing_cavities", "missing_production_rate"] {
        assert!(snapshot.issues.iter().any(|issue| issue.code == code), "no {code} in {:?}", snapshot.issues);
    }
    assert_eq!(snapshot.outputs.parts[0].production_cost, None);
    assert_eq!(snapshot.outputs.parts[0].per_part_subtotal, None);
    assert_eq!(snapshot.outputs.assembly_subtotal, None);
    // Setup is independent and stays available.
    assert_eq!(snapshot.outputs.setup_total, Some(d("60")));
    db.cleanup().await;
}

#[tokio::test]
async fn an_explicit_zero_cycle_is_valid_and_distinct_from_missing() {
    let db = TestDb::new().await;
    let app = api(&db);
    let source = upload(app.clone()).await;
    let save = SaveDraft { request_id: "create-zero".into(), expected_version: 0, draft: support::zero_cycle_draft(0, &source) };
    let (status, _, body) = send(app.clone(), json_request("POST", "/v1/projects", &serde_json::to_value(&save).expect("save"))).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let receipt: SaveReceipt = serde_json::from_value(value(&body)).expect("a receipt");
    let request = CalculateRequest { request_id: "calc-zero".into(), project_id: receipt.project_id, saved_version: 1 };
    let (_, _, body) = send(
        app,
        json_request("POST", &format!("/v1/projects/{}/calculations", receipt.project_id), &serde_json::to_value(&request).expect("request")),
    )
    .await;
    let snapshot: CalculationSnapshot = serde_json::from_value(value(&body)).expect("a snapshot");
    assert_eq!(snapshot.outputs.state, ResultState::Complete);
    assert_eq!(snapshot.outputs.parts[0].production_cost, Some(d("0")));
    assert_eq!(snapshot.outputs.parts[0].per_part_subtotal, Some(d("0.1388016")));
    assert_eq!(snapshot.outputs.assembly_subtotal, Some(d("0.4164048")));
    db.cleanup().await;
}

#[tokio::test]
async fn a_calculator_issue_names_the_reviewed_definition_not_the_part_version() {
    let db = TestDb::new().await;
    let app = api(&db);
    let source = upload(app.clone()).await;
    let save = SaveDraft { request_id: "create-missing".into(), expected_version: 0, draft: support::missing_cycle_draft(0, &source) };
    let (_, _, body) = send(app.clone(), json_request("POST", "/v1/projects", &serde_json::to_value(&save).expect("save"))).await;
    let receipt: SaveReceipt = serde_json::from_value(value(&body)).expect("a receipt");
    let request = CalculateRequest { request_id: "calc-missing".into(), project_id: receipt.project_id, saved_version: 1 };
    let (_, _, body) = send(
        app,
        json_request("POST", &format!("/v1/projects/{}/calculations", receipt.project_id), &serde_json::to_value(&request).expect("request")),
    )
    .await;
    let snapshot: CalculationSnapshot = serde_json::from_value(value(&body)).expect("a snapshot");
    let issue = snapshot.issues.iter().find(|issue| issue.code == "missing_cycle_time").expect("the missing cycle issue");
    assert_eq!(issue.definition_id, Some(Uuid::from_u128(0xdef1)), "the client names the reviewed definition");
    assert_ne!(issue.definition_id, Some(Uuid::from_u128(0xaa01)), "the part version id is server-side");
    db.cleanup().await;
}

#[tokio::test]
async fn an_unknown_reference_is_invalid_and_leaves_earlier_state_intact() {
    let db = TestDb::new().await;
    let app = api(&db);
    let source = upload(app.clone()).await;
    let receipt = create_project(app.clone(), &source).await;
    let project_id = receipt.project_id;

    let mut unknown_material = fixture_draft(1, &source);
    unknown_material.parts[0].material_version_id = Some(Uuid::new_v4());
    let save = SaveDraft { request_id: "save-material".into(), expected_version: 1, draft: unknown_material };
    let (status, _, body) =
        send(app.clone(), json_request("PUT", &format!("/v1/projects/{project_id}/draft"), &serde_json::to_value(&save).expect("save"))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{}", String::from_utf8_lossy(&body));
    assert_eq!(value(&body)["error"]["code"], "invalid");

    let mut unknown_rate = fixture_draft(1, &source);
    unknown_rate.parts[0].operation.as_mut().expect("operation").production_rate_version_id = Some(Uuid::new_v4());
    let save = SaveDraft { request_id: "save-rate".into(), expected_version: 1, draft: unknown_rate };
    let (status, _, _) =
        send(app, json_request("PUT", &format!("/v1/projects/{project_id}/draft"), &serde_json::to_value(&save).expect("save"))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    assert_eq!(db.store.version_count(project_id).await.expect("count"), 1);
    let (_, _, body) = send(api(&db), get(&format!("/v1/projects/{project_id}"))).await;
    let view: ProjectView = serde_json::from_value(value(&body)).expect("a project view");
    assert_eq!(view.draft.version, 1);
    db.cleanup().await;
}

#[tokio::test]
async fn an_oversized_request_id_or_source_name_is_refused() {
    let db = TestDb::new().await;
    let app = api(&db);
    let source = upload(app.clone()).await;
    let mut save = SaveDraft { request_id: "x".repeat(5_000), expected_version: 0, draft: fixture_draft(0, &source) };
    save.draft.project_id = Uuid::new_v4();
    let (status, _, body) = send(app.clone(), json_request("POST", "/v1/projects", &serde_json::to_value(&save).expect("save"))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{}", String::from_utf8_lossy(&body));
    assert_eq!(row_count(&db.store, "projects").await, 0);
    assert_eq!(row_count(&db.store, "requests").await, 0);

    let name = format!("/v1/sources/{}?name={}", digest(FIXTURE_SOURCE), "n".repeat(300));
    let (status, _, _) = send(app, put_bytes(&name, FIXTURE_SOURCE)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    db.cleanup().await;
}

#[tokio::test]
async fn the_engineering_and_administration_permissions_are_enforced_on_every_route_group() {
    let db = TestDb::new().await;
    let app = api(&db);

    // `GET /v1/me` answers any authenticated caller with its permission sets.
    let legacy = me(app.clone(), TOKEN).await;
    assert_eq!(legacy.name, "development");
    assert!(legacy.has(Permission::Engineer));
    assert!(legacy.has(Permission::Administrator));
    let engineer = me(app.clone(), ENGINEER_TOKEN).await;
    assert_eq!(engineer.name, "engineer");
    assert!(engineer.has(Permission::Engineer));
    assert!(!engineer.has(Permission::Administrator));
    let administrator = me(app.clone(), ADMIN_TOKEN).await;
    assert_eq!(administrator.name, "administrator");
    assert!(!administrator.has(Permission::Engineer));
    assert!(administrator.has(Permission::Administrator));

    // A missing or unknown token is 401; a known identity without the permission is 403.
    let no_token = Request::builder().method("GET").uri("/v1/me").body(Body::empty()).expect("a request");
    assert_eq!(send(app.clone(), no_token).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(send(app.clone(), get_as("/v1/shared", "not-a-known-token")).await.0, StatusCode::UNAUTHORIZED);
    let (status, _, body) = send(app.clone(), get_as("/v1/reference-data", ADMIN_TOKEN)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(value(&body)["error"]["code"], "forbidden");
    let (status, _, body) = send(app.clone(), get_as("/v1/shared", ENGINEER_TOKEN)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(value(&body)["error"]["code"], "forbidden");
    assert_eq!(send(app.clone(), get_as("/v1/reference-data", ENGINEER_TOKEN)).await.0, StatusCode::OK);
    assert_eq!(send(app.clone(), get_as("/v1/shared", ADMIN_TOKEN)).await.0, StatusCode::OK);

    // Administrators cannot write project routes; the refusal comes from the service.
    let source = upload_as_engineer(app.clone()).await;
    let admin_save = SaveDraft { request_id: "create-admin".into(), expected_version: 0, draft: fixture_draft(0, &source) };
    let (status, _, body) = send(app.clone(), json_as("POST", "/v1/projects", ADMIN_TOKEN, &serde_json::to_value(&admin_save).expect("save"))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(value(&body)["error"]["code"], "forbidden");
    assert_eq!(row_count(&db.store, "projects").await, 0);
    let engineer_save = SaveDraft { request_id: "create-engineer".into(), expected_version: 0, draft: fixture_draft(0, &source) };
    let (status, _, body) =
        send(app.clone(), json_as("POST", "/v1/projects", ENGINEER_TOKEN, &serde_json::to_value(&engineer_save).expect("save"))).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));

    // Engineers cannot publish shared versions; administrators cannot work on projects.
    let engineer_request = material_version("shared-by-engineer", 0, Uuid::new_v4(), "2.7", Some("20"));
    let (status, body) = publish_material(app.clone(), ENGINEER_TOKEN, &engineer_request).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(value(&body)["error"]["code"], "forbidden");
    assert_eq!(row_count(&db.store, "material_versions").await, 2, "the refused publish wrote nothing");
    assert_eq!(row_count(&db.store, "reference_changes").await, 0);
    let (status, body) = publish_material(app.clone(), ADMIN_TOKEN, &engineer_request).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let published: MaterialVersionPublished = serde_json::from_value(value(&body)).expect("a published material");
    assert_eq!(published.material.version, 1);
    assert_eq!(published.change.changed_by, "administrator");

    // The legacy combined identity still reaches both groups, as an existing setup expects.
    assert_eq!(send(app.clone(), get_as("/v1/reference-data", TOKEN)).await.0, StatusCode::OK);
    assert_eq!(send(app, get_as("/v1/shared", TOKEN)).await.0, StatusCode::OK);
    db.cleanup().await;
}

#[tokio::test]
async fn publishing_a_new_price_and_rate_leaves_an_existing_project_and_calculation_byte_identical() {
    let db = TestDb::new().await;
    let app = api(&db);
    let source = upload(app.clone()).await;
    let receipt = create_project(app.clone(), &source).await;
    let project_id = receipt.project_id;
    let snapshot = calculate(app.clone(), "calc-before", project_id, 1).await;
    assert_eq!(snapshot.inputs.parts[0].material_price_per_kg, Some(d("20")));
    assert_eq!(snapshot.inputs.parts[0].production_rate_per_hour, Some(d("120")));
    let (_, _, calculation_body) = send(app.clone(), get(&format!("/v1/calculations/{}", snapshot.calculation_id))).await;
    let (_, _, view_before) = send(app.clone(), get(&format!("/v1/projects/{project_id}"))).await;

    // Publish a newer material price and a newer cost-center rate.
    let material = material_version("shared-material-2", 1, seeded_material_version_id(), "2.7", Some("25"));
    let (status, body) = publish_material(app.clone(), ADMIN_TOKEN, &material).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let published: MaterialVersionPublished = serde_json::from_value(value(&body)).expect("a published material");
    assert_eq!((published.material.version, published.material.supersedes), (2, Some(seeded_material_version_id())));
    assert_eq!(published.material.price_per_kg, Some(d("25")));
    let rate = rate_version("shared-rate-2", 1, seeded_rate_version_id(), Some("150"), CostCenterPurpose::Production);
    let (status, body) = publish_rate(app.clone(), ADMIN_TOKEN, &rate).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let published: RateVersionPublished = serde_json::from_value(value(&body)).expect("a published rate");
    assert_eq!((published.cost_center.version, published.cost_center.supersedes), (2, Some(seeded_rate_version_id())));

    // The project version and the saved snapshot are byte-identical; only reference data moved.
    let (_, _, view_after) = send(app.clone(), get(&format!("/v1/projects/{project_id}"))).await;
    assert_eq!(view_before, view_after, "a new reference version never edits an existing project version");
    let (_, _, calculation_after) = send(app.clone(), get(&format!("/v1/calculations/{}", snapshot.calculation_id))).await;
    assert_eq!(calculation_body, calculation_after, "a new reference version never rewrites a calculation snapshot");
    let before = value(&view_before);
    assert_eq!(before["draft"]["parts"][0]["material_version_id"], serde_json::json!(seeded_material_version_id()));
    assert_eq!(before["draft"]["parts"][0]["operation"]["production_rate_version_id"], serde_json::json!(seeded_rate_version_id()));

    // Every published version stays readable, including the historical ones.
    let data = reference_data(app, ENGINEER_TOKEN).await;
    let mut versions: Vec<_> = data.materials.iter().filter(|material| material.material_id == seeded_material_version_id()).collect();
    versions.sort_by_key(|material| material.version);
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0].material_version_id, seeded_material_version_id());
    assert_eq!(versions[0].price_per_kg, Some(d("20")));
    assert_eq!(versions[1].price_per_kg, Some(d("25")));
    assert_eq!(versions[1].supersedes, Some(seeded_material_version_id()));
    assert_eq!(versions[1].density_unit, solidbom_contract::Unit::GramPerCentimetreCubed);
    assert_eq!(versions[1].price_unit, solidbom_contract::Unit::CurrencyPerKilogram);
    assert_eq!(versions[1].source_reference.as_deref(), Some("supplier sheet 2026-11"));
    assert!(versions[1].created_at.is_some());
    let latest = data.latest_materials();
    let latest = latest.iter().find(|material| material.material_id == seeded_material_version_id()).expect("the latest material");
    assert_eq!(latest.version, 2);
    let mut rate_versions: Vec<_> = data.rates.iter().filter(|rate| rate.cost_center_id == seeded_rate_version_id()).collect();
    rate_versions.sort_by_key(|rate| rate.version);
    assert_eq!(rate_versions.len(), 2);
    assert_eq!(rate_versions[1].rate_per_hour, Some(d("150")));
    assert_eq!(rate_versions[1].rate_unit, solidbom_contract::Unit::CurrencyPerHour);
    db.cleanup().await;
}

#[tokio::test]
async fn a_missing_price_and_rate_stay_missing_while_an_explicit_zero_stays_zero() {
    let db = TestDb::new().await;
    let app = api(&db);

    let missing_material = material_version("shared-material-missing", 0, Uuid::new_v4(), "2.7", None);
    let (status, body) = publish_material(app.clone(), ADMIN_TOKEN, &missing_material).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let missing_material: MaterialVersionPublished = serde_json::from_value(value(&body)).expect("a published material");
    let zero_material = material_version("shared-material-zero", 0, Uuid::new_v4(), "2.7", Some("0"));
    let (status, body) = publish_material(app.clone(), ADMIN_TOKEN, &zero_material).await;
    assert_eq!(status, StatusCode::CREATED);
    let zero_material: MaterialVersionPublished = serde_json::from_value(value(&body)).expect("a published material");
    let missing_center = rate_version("shared-rate-missing", 0, Uuid::new_v4(), None, CostCenterPurpose::Production);
    let (status, body) = publish_rate(app.clone(), ADMIN_TOKEN, &missing_center).await;
    assert_eq!(status, StatusCode::CREATED);
    let missing_rate: RateVersionPublished = serde_json::from_value(value(&body)).expect("a published rate");
    let zero_center = rate_version("shared-rate-zero", 0, Uuid::new_v4(), Some("0"), CostCenterPurpose::Production);
    let (status, body) = publish_rate(app.clone(), ADMIN_TOKEN, &zero_center).await;
    assert_eq!(status, StatusCode::CREATED);
    let zero_rate: RateVersionPublished = serde_json::from_value(value(&body)).expect("a published rate");

    // On the wire `null` and `"0"` are different values, not both zero.
    let data = reference_data(app.clone(), ENGINEER_TOKEN).await;
    let missing = data.material(missing_material.material.material_version_id).expect("the missing-price material");
    let zero = data.material(zero_material.material.material_version_id).expect("the zero-price material");
    assert_eq!(missing.price_per_kg, None);
    assert_eq!(zero.price_per_kg, Some(d("0")));
    assert_ne!(serde_json::to_value(missing.price_per_kg).expect("JSON"), serde_json::to_value(zero.price_per_kg).expect("JSON"));
    assert_eq!(data.rate(missing_rate.cost_center.rate_version_id).and_then(|rate| rate.rate_per_hour), None);
    assert_eq!(data.rate(zero_rate.cost_center.rate_version_id).and_then(|rate| rate.rate_per_hour), Some(d("0")));

    // A calculation with a missing price and rate names the missing inputs; it never subtracts zero.
    let source = upload(app.clone()).await;
    let mut draft = fixture_draft(0, &source);
    draft.parts[0].material_version_id = Some(missing_material.material.material_version_id);
    draft.parts[0].operation.as_mut().expect("operation").production_rate_version_id = Some(missing_rate.cost_center.rate_version_id);
    let receipt = create_with_draft(app.clone(), "create-missing", draft).await;
    let snapshot = calculate(app.clone(), "calc-missing", receipt.project_id, 1).await;
    assert_eq!(snapshot.outputs.state, ResultState::Incomplete);
    assert_eq!(snapshot.inputs.parts[0].material_price_per_kg, None);
    assert_eq!(snapshot.inputs.parts[0].production_rate_per_hour, None);
    assert_eq!(snapshot.outputs.parts[0].allowed_material_cost, None);
    assert_eq!(snapshot.outputs.parts[0].production_cost, None);
    for code in ["missing_material_price", "missing_production_rate"] {
        assert!(snapshot.issues.iter().any(|issue| issue.code == code), "no {code} in {:?}", snapshot.issues);
    }

    // An explicit zero is a stored value: the calculation reads it as zero and does not report a
    // missing input for the price.
    let mut draft = fixture_draft(0, &source);
    draft.parts[0].material_version_id = Some(zero_material.material.material_version_id);
    draft.parts[0].operation.as_mut().expect("operation").production_rate_version_id = Some(zero_rate.cost_center.rate_version_id);
    let receipt = create_with_draft(app.clone(), "create-zero", draft).await;
    let snapshot = calculate(app, "calc-zero", receipt.project_id, 1).await;
    assert_eq!(snapshot.inputs.parts[0].material_price_per_kg, Some(d("0")));
    assert_eq!(snapshot.outputs.parts[0].allowed_material_cost, Some(d("0")));
    assert!(!snapshot.issues.iter().any(|issue| issue.code == "missing_material_price"));
    assert!(snapshot.issues.iter().any(|issue| issue.code == "invalid_production_rate"), "{:?}", snapshot.issues);
    db.cleanup().await;
}

#[tokio::test]
async fn an_expected_version_conflict_and_an_identical_publish_retry() {
    let db = TestDb::new().await;
    let app = api(&db);
    let first = material_version("shared-1", 1, seeded_material_version_id(), "2.7", Some("25"));
    let (status, body) = publish_material(app.clone(), ADMIN_TOKEN, &first).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let first_reply: MaterialVersionPublished = serde_json::from_value(value(&body)).expect("a published material");
    assert_eq!(first_reply.material.version, 2);

    // The identical retry returns the recorded reply and appends nothing.
    let (status, retried) = publish_material(app.clone(), ADMIN_TOKEN, &first).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body, retried);
    assert_eq!(row_count(&db.store, "material_versions").await, 3);
    assert_eq!(row_count(&db.store, "reference_changes").await, 1);
    assert_eq!(row_count(&db.store, "requests").await, 1);

    // A stale expected version with a new request id is a 409 and writes nothing.
    let mut stale = first.clone();
    stale.request_id = "shared-2".into();
    stale.price_per_kg = Some(d("30"));
    let (status, body) = publish_material(app.clone(), ADMIN_TOKEN, &stale).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(value(&body)["error"]["code"], "conflict");
    assert_eq!(row_count(&db.store, "material_versions").await, 3);
    assert_eq!(row_count(&db.store, "reference_changes").await, 1);

    // A newer expected version publishes version 3 and supersedes version 2.
    let mut next = stale.clone();
    next.request_id = "shared-3".into();
    next.expected_version = 2;
    let (status, body) = publish_material(app.clone(), ADMIN_TOKEN, &next).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let second: MaterialVersionPublished = serde_json::from_value(value(&body)).expect("a published material");
    assert_eq!(second.material.version, 3);
    assert_eq!(second.material.supersedes, Some(first_reply.material.material_version_id));
    assert_eq!(second.change.previous_version_id, Some(first_reply.material.material_version_id));
    assert_eq!(row_count(&db.store, "material_versions").await, 4);
    assert_eq!(row_count(&db.store, "reference_changes").await, 2);

    // A reused request id with a different payload is a conflict even when the version moved on.
    let mut reused = first.clone();
    reused.price_per_kg = Some(d("99"));
    let (status, body) = publish_material(app, ADMIN_TOKEN, &reused).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(value(&body)["error"]["code"], "conflict");
    db.cleanup().await;
}

#[tokio::test]
async fn an_invalid_app_configuration_is_refused_and_a_valid_one_is_published_versioned() {
    let db = TestDb::new().await;
    let app = api(&db);
    let config_id = Uuid::new_v4();
    let mut direct_for_ebit = direct_category("material-cost", DirectCostInput::Material);
    direct_for_ebit.included_in_ebit = true;
    let invalid = vec![
        ("ambiguous base", vec![percentage_category("overhead", false, &[], "0.05")]),
        ("self base", vec![percentage_category("overhead", false, &["overhead"], "0.05")]),
        ("circular base", vec![percentage_category("first", false, &["second"], "0.01"), percentage_category("second", false, &["first"], "0.02")]),
        ("unknown basis category", vec![percentage_category("overhead", false, &["missing"], "0.05")]),
        (
            "duplicated basis category",
            vec![fixed_category("consumables", "2"), percentage_category("overhead", false, &["consumables", "consumables"], "0.05")],
        ),
        ("duplicated category code", vec![fixed_category("same", "1"), fixed_category("same", "2")]),
        (
            "duplicated direct inclusion",
            vec![direct_category("material-a", DirectCostInput::Material), direct_category("material-b", DirectCostInput::Material)],
        ),
        (
            "direct subtotal plus a named direct category",
            vec![direct_category("material-cost", DirectCostInput::Material), percentage_category("overhead", true, &["material-cost"], "0.05")],
        ),
        ("direct category for ebit", vec![direct_for_ebit]),
        ("negative fixed amount", vec![fixed_category("negative", "-1")]),
    ];
    for (index, (what, categories)) in invalid.into_iter().enumerate() {
        let request = app_config_version(&format!("shared-config-invalid-{index}"), 0, config_id, categories);
        let (status, body) = publish_app_config(app.clone(), ADMIN_TOKEN, &request).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{what}: {}", String::from_utf8_lossy(&body));
        assert_eq!(value(&body)["error"]["code"], "invalid", "{what}");
    }
    assert_eq!(row_count(&db.store, "app_config_versions").await, 0, "a refused configuration wrote nothing");
    assert_eq!(row_count(&db.store, "reference_changes").await, 0);
    assert_eq!(row_count(&db.store, "requests").await, 0);

    let categories = vec![
        direct_category("material-cost", DirectCostInput::Material),
        fixed_category("consumables", "2"),
        percentage_category("overhead", true, &[], "0.05"),
        percentage_category("factory-overhead", false, &["overhead"], "0.02"),
    ];
    let request = app_config_version("shared-config-1", 0, config_id, categories.clone());
    let (status, body) = publish_app_config(app.clone(), ADMIN_TOKEN, &request).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let published: AppConfigVersionPublished = serde_json::from_value(value(&body)).expect("a published configuration");
    assert_eq!((published.config.version, published.config.supersedes), (1, None));
    assert_eq!(published.config.categories.len(), 4);
    assert_eq!(published.change.kind, solidbom_contract::SharedRecordKind::AppConfig);

    let request = app_config_version("shared-config-2", 1, config_id, categories);
    let (status, body) = publish_app_config(app.clone(), ADMIN_TOKEN, &request).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let published: AppConfigVersionPublished = serde_json::from_value(value(&body)).expect("a published configuration");
    assert_eq!(published.config.version, 2);
    assert_eq!(published.config.supersedes, Some(published.change.previous_version_id.expect("a previous version")));

    let data = reference_data(app.clone(), ENGINEER_TOKEN).await;
    let config = data.app_config.expect("the latest configuration");
    assert_eq!(config.version, 2);
    assert_eq!(config.currency, "CNY");
    assert!(config.category("overhead").is_some());
    let view = shared_view(app.clone(), ADMIN_TOKEN).await;
    assert_eq!(view.app_config.map(|config| config.version), Some(2));
    assert_eq!(view.app_config_versions.iter().map(|config| config.version).collect::<Vec<_>>(), [1, 2]);
    db.cleanup().await;
}

#[tokio::test]
async fn the_shared_view_lists_version_chains_components_and_change_facts() {
    let db = TestDb::new().await;
    let app = api(&db);
    let component_id = Uuid::new_v4();
    let config_id = Uuid::new_v4();

    let material = material_version("shared-material", 1, seeded_material_version_id(), "2.7", Some("25"));
    let (status, body) = publish_material(app.clone(), ADMIN_TOKEN, &material).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let material: MaterialVersionPublished = serde_json::from_value(value(&body)).expect("a published material");
    let center = rate_version("shared-center", 0, Uuid::new_v4(), Some("90"), CostCenterPurpose::Setup);
    let (status, body) = publish_rate(app.clone(), ADMIN_TOKEN, &center).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let center: RateVersionPublished = serde_json::from_value(value(&body)).expect("a published rate");
    let component = component_version("shared-component-1", 0, component_id, None);
    let (status, body) = publish_component(app.clone(), ADMIN_TOKEN, &component).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let component: ComponentVersionPublished = serde_json::from_value(value(&body)).expect("a published component");
    let categories = vec![direct_category("material-cost", DirectCostInput::Material)];
    let config = app_config_version("shared-config", 0, config_id, categories);
    let (status, body) = publish_app_config(app.clone(), ADMIN_TOKEN, &config).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let config: AppConfigVersionPublished = serde_json::from_value(value(&body)).expect("a published configuration");

    // Publishing a newer configuration keeps the older one readable in the shared view.
    let next_categories = vec![direct_category("material-cost", DirectCostInput::Material), fixed_category("consumables", "2")];
    let next_config = app_config_version("shared-config-2", 1, config_id, next_categories);
    let (status, body) = publish_app_config(app.clone(), ADMIN_TOKEN, &next_config).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let next_config: AppConfigVersionPublished = serde_json::from_value(value(&body)).expect("a published configuration");
    assert_eq!(next_config.config.version, 2);
    assert_eq!(next_config.config.supersedes, Some(config.config.config_version_id));
    assert_eq!(next_config.change.previous_version_id, Some(config.config.config_version_id));

    let view = shared_view(app.clone(), ADMIN_TOKEN).await;
    view.validate().expect("the shared view is valid");
    assert_eq!(view.identity.name, "administrator");
    assert!(view.identity.has(Permission::Administrator));
    // Every published version is listed, oldest version first per logical record.
    let chain: Vec<_> = view.materials.iter().filter(|version| version.material_id == seeded_material_version_id()).collect();
    assert_eq!(chain.iter().map(|version| version.version).collect::<Vec<_>>(), [1, 2]);
    assert_eq!(chain[1].material_version_id, material.material.material_version_id);
    assert_eq!(chain[1].supersedes, Some(seeded_material_version_id()));
    let cost_centers: Vec<_> = view.cost_centers.iter().filter(|version| version.cost_center_id == center.cost_center.cost_center_id).collect();
    assert_eq!(cost_centers.len(), 1);
    assert_eq!(cost_centers[0].purpose, CostCenterPurpose::Setup);
    assert_eq!(cost_centers[0].rate_per_hour, Some(d("90")));
    let component_versions: Vec<_> = view.components.iter().filter(|version| version.component_id == component_id).collect();
    assert_eq!(component_versions.len(), 1);
    assert_eq!(component_versions[0].component_version_id, component.component.component_version_id);
    assert_eq!(component_versions[0].price, None);
    assert_eq!(component_versions[0].price_unit, solidbom_contract::Unit::CurrencyPerPiece);
    assert_eq!(view.app_config.as_ref().map(|config| config.version), Some(2));
    // The complete app-configuration history is readable, oldest version first.
    assert_eq!(view.app_config_versions.iter().map(|config| config.version).collect::<Vec<_>>(), [1, 2]);
    assert_eq!(view.app_config_versions[0].config_version_id, config.config.config_version_id);
    assert_eq!(view.app_config_versions[0].categories.len(), 1);
    assert_eq!(view.app_config_versions[1].config_version_id, next_config.config.config_version_id);
    assert_eq!(view.app_config_versions[1].supersedes, Some(config.config.config_version_id));
    assert_eq!(view.app_config_versions[1].categories.len(), 2);

    // Facts are new, complete and newest first; they adopt nothing into a project.
    assert_eq!(view.changes.len(), 5);
    assert!(view.changes.windows(2).all(|pair| pair[0].changed_at >= pair[1].changed_at), "facts newest first");
    let fact = view.changes.iter().find(|change| change.change_id == material.change.change_id).expect("the material fact");
    assert_eq!(fact.kind, solidbom_contract::SharedRecordKind::Material);
    assert_eq!(fact.record_id, seeded_material_version_id());
    assert_eq!(fact.version_id, material.material.material_version_id);
    assert_eq!(fact.version, 2);
    assert_eq!(fact.previous_version_id, Some(seeded_material_version_id()));
    assert_eq!(fact.changed_by, "administrator");
    assert!(!fact.changed_at.is_empty());
    assert!(view.changes.iter().any(|change| change.change_id == component.change.change_id));
    assert!(view.changes.iter().any(|change| change.change_id == config.change.change_id));
    let newer_config_fact =
        view.changes.iter().find(|change| change.change_id == next_config.change.change_id).expect("the newer configuration fact");
    assert_eq!(newer_config_fact.kind, solidbom_contract::SharedRecordKind::AppConfig);
    assert_eq!(newer_config_fact.version, 2);
    assert_eq!(newer_config_fact.previous_version_id, Some(config.config.config_version_id));

    // A publish path that contradicts the body is refused before anything is written.
    let wrong_path = material_version("shared-wrong-path", 2, seeded_material_version_id(), "2.7", Some("26"));
    let request = json_as(
        "POST",
        &format!("/v1/shared/materials/{}/versions", Uuid::new_v4()),
        ADMIN_TOKEN,
        &serde_json::to_value(&wrong_path).expect("request"),
    );
    let (status, _, body) = send(app, request).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(value(&body)["error"]["code"], "invalid");
    db.cleanup().await;
}

#[tokio::test]
async fn version_two_project_records_are_authenticated_and_version_checked() {
    let db = TestDb::new().await;
    let app = api(&db);
    let source = support::upload(&db.store).await;
    let project_id = Uuid::from_u128(0x7700);
    let create = CreateProject {
        request_id: "create-1".into(),
        project_id,
        details: ProjectDetails {
            name: "Inquiry".into(),
            customer: "Customer".into(),
            internal_team: None,
            platform: None,
            application: None,
            currency: "CNY".into(),
            default_schedule: YearRange { sop_year: 2026, eop_year: 2028 },
        },
    };
    let (status, _, body) = send(app.clone(), json_request("POST", "/v2/projects", &serde_json::to_value(&create).expect("create"))).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let outcome: ProjectWriteOutcome = serde_json::from_slice(&body).expect("the project outcome");
    assert_eq!(outcome.receipt.version, 1);

    // The same routes need the bearer token and never leak a version 1 view.
    let unauthorized = Request::builder().method("GET").uri(format!("/v2/projects/{project_id}")).body(Body::empty()).expect("a request");
    let (status, _, _) = send(app.clone(), unauthorized).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _, body) = send(app.clone(), get(&format!("/v2/projects/{project_id}"))).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let view: ProjectRecordView = serde_json::from_slice(&body).expect("the project view");
    assert_eq!(view.version, 1);
    assert!(view.document.products.is_empty());

    let delivery = RecordDelivery {
        request_id: "delivery-1".into(),
        expected_version: 1,
        package: DeliveryPackage { package_id: Uuid::from_u128(0x7701), name: "Package 1".into(), source },
        products: vec![DeliveredProductInput {
            product_id: None,
            name: "A".into(),
            kind: ProductKind::Assembly,
            customer_part_number: Some("PL-A".into()),
            parts: vec![DeliveredPartInput {
                delivered_key: "def-a".into(),
                customer_part_number: Some("PL-100".into()),
                customer_revision: Some("B".into()),
                volume_mm3: Some(d("2400")),
            }],
        }],
    };
    let (status, _, body) = send(
        app.clone(),
        json_request("POST", &format!("/v2/projects/{project_id}/deliveries"), &serde_json::to_value(&delivery).expect("delivery")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let outcome: ProjectWriteOutcome = serde_json::from_slice(&body).expect("the delivery outcome");
    assert_eq!(outcome.receipt.version, 2);
    assert!(matches!(outcome.facts.as_slice(), [ProjectFact::DeliveryRecorded { .. }]));

    // A stale expected version is a conflict. Activation is published under /activations by
    // package SB-19; the bare /activate path this contract never published stays absent.
    let mut stale = delivery.clone();
    stale.request_id = "delivery-2".into();
    stale.package.package_id = Uuid::from_u128(0x7702);
    stale.package.name = "Stale".into();
    let (status, _, _) =
        send(app.clone(), json_request("POST", &format!("/v2/projects/{project_id}/deliveries"), &serde_json::to_value(&stale).expect("stale")))
            .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _, _) = send(app.clone(), get(&format!("/v2/projects/{project_id}/activate"))).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "activation belongs to SB-19 and has no version 2 route");

    // A stored document this build cannot read is refused rather than returned.
    sqlx::query("UPDATE project_record_versions SET document = jsonb_set(document, '{schema_version}', '99') WHERE project_id = $1")
        .bind(project_id)
        .execute(db.store.pool())
        .await
        .expect("store a newer schema");
    let (status, _, _) = send(app.clone(), get(&format!("/v2/projects/{project_id}"))).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    db.cleanup().await;
}

/// The flat fixture's exact analysis: one definition with three bodies and no usable hierarchy.
fn flat_analysis() -> AnalysisManifest {
    let key = engineering_key();
    AnalysisManifest {
        schema_version: SCHEMA_VERSION,
        key: key.clone(),
        definitions: vec![DefinitionRef {
            definition_id: Uuid::from_u128(0xdef1),
            source_key: key.clone(),
            customer_part_number: None,
            customer_revision: None,
            volume_mm3: Measurement {
                value: None,
                unit: Unit::MillimetreCubed,
                source: MeasurementSource::Extracted,
                review: ReviewState::Pending,
                reason: None,
                analysis_key: key.clone(),
            },
        }],
        occurrences: vec![OccurrenceRef {
            occurrence_id: Uuid::from_u128(0x0cc0),
            definition_id: Uuid::from_u128(0xdef1),
            parent_occurrence_id: None,
            source_path: Vec::new(),
            transform: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]],
        }],
        geometry_bindings: (0u32..3)
            .map(|source_body| GeometryBinding {
                source_path: Vec::new(),
                source_body,
                definition_id: Uuid::from_u128(0xdef1),
                occurrence_id: Uuid::from_u128(0x0cc0),
            })
            .collect(),
        issues: vec![solidbom_contract::Issue::new("unresolved_grouping", "analysis.definitions", "the flat input needs a component review")],
    }
}

fn engineering_key() -> AnalysisKey {
    AnalysisKey { source_sha256: digest(FIXTURE_SOURCE), engine_build: "0.40.0-rc.2".into(), extractor_version: "2".into() }
}

#[tokio::test]
async fn engineering_review_routes_are_authenticated_bound_and_confirm_engineer_decisions() {
    let db = TestDb::new().await;
    let app = api(&db);
    let source = upload(app.clone()).await;
    let project_id = Uuid::from_u128(0x7700);
    let create = CreateProject {
        request_id: "create-1".into(),
        project_id,
        details: ProjectDetails {
            name: "Inquiry".into(),
            customer: "Customer".into(),
            internal_team: None,
            platform: None,
            application: None,
            currency: "CNY".into(),
            default_schedule: YearRange { sop_year: 2026, eop_year: 2028 },
        },
    };
    let (status, _, body) = send(app.clone(), json_request("POST", "/v2/projects", &serde_json::to_value(&create).expect("create"))).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));

    // The review reads as version 0 before any decision, needs a token and the Engineer set.
    let review_path = format!("/v2/projects/{project_id}/engineering");
    let (status, _, body) = send(app.clone(), get(&review_path)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let review: EngineeringRecordView = serde_json::from_slice(&body).expect("the review view");
    assert_eq!(review.version, 0);
    assert!(review.document.components.is_empty());
    assert!(review.issues.is_empty());
    let unauthorized = Request::builder().method("GET").uri(&review_path).body(Body::empty()).expect("a request");
    assert_eq!(send(app.clone(), unauthorized).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(send(app.clone(), get_as(&review_path, ADMIN_TOKEN)).await.0, StatusCode::FORBIDDEN);

    // Deliver the flat product so the revision and its shared part version exist.
    let delivery = RecordDelivery {
        request_id: "delivery-1".into(),
        expected_version: 1,
        package: DeliveryPackage { package_id: Uuid::from_u128(0x7701), name: "Package 1".into(), source },
        products: vec![DeliveredProductInput {
            product_id: None,
            name: "A".into(),
            kind: ProductKind::Part,
            customer_part_number: None,
            parts: vec![DeliveredPartInput {
                delivered_key: "body-set".into(),
                customer_part_number: Some("PL-100".into()),
                customer_revision: Some("B".into()),
                volume_mm3: Some(d("2400")),
            }],
        }],
    };
    let delivery_path = format!("/v2/projects/{project_id}/deliveries");
    let (status, _, body) = send(app.clone(), json_request("POST", &delivery_path, &serde_json::to_value(&delivery).expect("delivery"))).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let (status, _, body) = send(app.clone(), get(&format!("/v2/projects/{project_id}"))).await;
    assert_eq!(status, StatusCode::OK);
    let record: ProjectRecordView = serde_json::from_slice(&body).expect("the project record");
    let revision = record.document.products[0].revisions[0].product_revision_id;
    let part_version_id = record.document.products[0].revisions[0].parts[0].part_version_id.expect("the delivered part match");

    // The exact analysis binds the review to the delivered source hash.
    let register = RegisterAnalysis { request_id: "analysis-1".into(), product_revision_id: revision, manifest: flat_analysis() };
    let analysis_path = format!("/v2/projects/{project_id}/engineering/analyses");
    let (status, _, body) = send(app.clone(), json_request("POST", &analysis_path, &serde_json::to_value(&register).expect("analysis"))).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let receipt: AnalysisReceipt = serde_json::from_slice(&body).expect("the analysis receipt");
    assert_eq!(receipt.analysis_key.source_sha256, digest(FIXTURE_SOURCE));

    // A stale expected version writes nothing; the then-current version groups the component.
    let group = GroupSolids {
        request_id: "group-1".into(),
        expected_version: 0,
        component_id: Uuid::from_u128(0xc001),
        product_revision_id: revision,
        analysis_key: engineering_key(),
        geometry: vec![
            ReviewedGeometry { source_path: Vec::new(), source_body: 0 },
            ReviewedGeometry { source_path: Vec::new(), source_body: 1 },
            ReviewedGeometry { source_path: Vec::new(), source_body: 2 },
        ],
        part_version_id: Some(part_version_id),
    };
    let components_path = format!("/v2/projects/{project_id}/engineering/components");
    let mut stale = group.clone();
    stale.request_id = "group-stale".into();
    stale.expected_version = 2;
    let (status, _, _) = send(app.clone(), json_request("POST", &components_path, &serde_json::to_value(&stale).expect("stale group"))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _, body) = send(app.clone(), json_request("POST", &components_path, &serde_json::to_value(&group).expect("group"))).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let outcome: EngineeringWriteOutcome = serde_json::from_slice(&body).expect("the group outcome");
    assert_eq!(outcome.receipt.version, 1);
    assert!(matches!(outcome.facts.as_slice(), [EngineeringFact::ComponentGrouped { geometry: 3, .. }]));

    // The open-surface volume is missing, never zero.
    let (_, _, body) = send(app.clone(), get(&review_path)).await;
    let review: EngineeringRecordView = serde_json::from_slice(&body).expect("the review view");
    assert!(review.issues.iter().any(|issue| issue.code == "missing_reviewed_volume"), "{:?}", review.issues);

    // Record the manual completion and confirm it as the Engineer identity.
    let measurement = CompleteMeasurement {
        request_id: "measure-1".into(),
        expected_version: 1,
        part_version_id,
        volume_mm3: Measurement {
            value: Some(d("2500")),
            unit: Unit::MillimetreCubed,
            source: MeasurementSource::Manual,
            review: ReviewState::Pending,
            reason: Some("open surface; measured on the bench".into()),
            analysis_key: engineering_key(),
        },
    };
    let measurements_path = format!("/v2/projects/{project_id}/engineering/measurements");
    let (status, _, body) =
        send(app.clone(), json_request("POST", &measurements_path, &serde_json::to_value(&measurement).expect("measurement"))).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let confirmation = ConfirmParts {
        request_id: "confirm-1".into(),
        expected_version: 2,
        confirmations: vec![PartConfirmation {
            part_version_id,
            classification: Some(ClassificationConfirmation { name: "clip".into() }),
            confirm_measurement: true,
            catalog: None,
        }],
    };
    let confirmations_path = format!("/v2/projects/{project_id}/engineering/confirmations");
    let (status, _, _) =
        send(app.clone(), json_as("POST", &confirmations_path, ADMIN_TOKEN, &serde_json::to_value(&confirmation).expect("confirmation"))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, body) =
        send(app.clone(), json_as("POST", &confirmations_path, ENGINEER_TOKEN, &serde_json::to_value(&confirmation).expect("confirmation"))).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let outcome: EngineeringWriteOutcome = serde_json::from_slice(&body).expect("the confirmation outcome");
    assert_eq!(outcome.receipt.version, 3);

    // The authenticated identity is the reviewer; no payload field can claim it.
    let (status, _, body) = send(app.clone(), get_as(&review_path, ENGINEER_TOKEN)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let review: EngineeringRecordView = serde_json::from_slice(&body).expect("the review view");
    assert!(review.issues.is_empty(), "{:?}", review.issues);
    assert_eq!(review.document.parts[0].classification.confirmed.as_ref().map(|label| label.confirmed_by.as_str()), Some("engineer"));
    assert_eq!(review.document.parts[0].volume_confirmed_by.as_deref(), Some("engineer"), "the volume confirmation keeps its reviewer");
    assert_eq!(review.document.training_examples.len(), 1);

    // Retained versions stay readable; a version or project that was never written is not found.
    let (status, _, _) = send(app.clone(), get(&format!("/v2/projects/{project_id}/engineering/versions/1"))).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = send(app.clone(), get(&format!("/v2/projects/{project_id}/engineering/versions/9"))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = send(app.clone(), get(&format!("/v2/projects/{}/engineering", Uuid::new_v4()))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    db.cleanup().await;
}

/// One complete operation of the manufacturing route fixtures: a per-piece cycle pinned to the
/// seeded production rate version.
fn manufacturing_operation(operation_id: u128, sequence: u32, name: &str, seconds: &str) -> RoutingOperation {
    RoutingOperation {
        operation_id: Uuid::from_u128(operation_id),
        sequence,
        name: name.into(),
        description: None,
        cost_center_id: Some(seeded_rate_version_id()),
        cycle: Some(OperationCycle {
            seconds: Some(d(seconds)),
            basis: CycleBasis::PerPiece,
            cavities: None,
            production_rate_version_id: Some(seeded_rate_version_id()),
            origin: CycleOrigin::Manual,
            source_reference: None,
        }),
        setup: None,
        tooling_reference: None,
    }
}

fn manufacturing_purchased(part_version_id: Uuid) -> SourcedPart {
    SourcedPart { part_version_id, sourcing: Sourcing::Purchased, material: None, source_reference: None }
}

fn manufacturing_manufactured(part_version_id: Uuid, material_version_id: Uuid) -> SourcedPart {
    SourcedPart {
        part_version_id,
        sourcing: Sourcing::Manufactured,
        material: Some(MaterialConsumption {
            material_version_id: Some(material_version_id),
            quantity: Some(d("2400")),
            unit: Unit::MillimetreCubed,
            source: MeasurementSource::Extracted,
            review: ReviewState::Confirmed,
            confirmed_by: None,
            analysis_key: Some(AnalysisKey {
                source_sha256: digest(FIXTURE_SOURCE),
                engine_build: "0.40.0-rc.2".into(),
                extractor_version: "1".into(),
            }),
            reason: None,
            allowance_rate: Some(d("0.02")),
            source_reference: None,
        }),
        source_reference: None,
    }
}

fn manufacturing_line(line_id: u128, owner: CostOwner) -> AttachedCostLine {
    AttachedCostLine {
        line_id: Uuid::from_u128(line_id),
        owner,
        category: "packaging".into(),
        description: Some("returnable tray".into()),
        quantity: Some(d("1")),
        unit: "pcs".into(),
        unit_cost: Some(d("0.35")),
        source_reference: None,
    }
}

#[tokio::test]
async fn sourcing_and_manufacturing_routes_are_permission_checked_and_keep_history() {
    let db = TestDb::new().await;
    let app = api(&db);
    let source = upload(app.clone()).await;
    let project_id = Uuid::from_u128(0x7800);
    let create = CreateProject {
        request_id: "create-1".into(),
        project_id,
        details: ProjectDetails {
            name: "Inquiry".into(),
            customer: "Customer".into(),
            internal_team: None,
            platform: None,
            application: None,
            currency: "CNY".into(),
            default_schedule: YearRange { sop_year: 2026, eop_year: 2028 },
        },
    };
    let (status, _, body) = send(app.clone(), json_request("POST", "/v2/projects", &serde_json::to_value(&create).expect("create"))).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));

    // Deliver two products whose delivered parts are two customer revisions of one part number.
    let delivery = RecordDelivery {
        request_id: "delivery-1".into(),
        expected_version: 1,
        package: DeliveryPackage { package_id: Uuid::from_u128(0x7801), name: "Package 1".into(), source },
        products: vec![
            DeliveredProductInput {
                product_id: None,
                name: "part-a".into(),
                kind: ProductKind::Part,
                customer_part_number: None,
                parts: vec![DeliveredPartInput {
                    delivered_key: "a-body".into(),
                    customer_part_number: Some("P-1".into()),
                    customer_revision: Some("A".into()),
                    volume_mm3: Some(d("2400")),
                }],
            },
            DeliveredProductInput {
                product_id: None,
                name: "part-b".into(),
                kind: ProductKind::Part,
                customer_part_number: None,
                parts: vec![DeliveredPartInput {
                    delivered_key: "b-body".into(),
                    customer_part_number: Some("P-1".into()),
                    customer_revision: Some("B".into()),
                    volume_mm3: Some(d("2400")),
                }],
            },
        ],
    };
    let delivery_path = format!("/v2/projects/{project_id}/deliveries");
    let (status, _, body) = send(app.clone(), json_request("POST", &delivery_path, &serde_json::to_value(&delivery).expect("delivery"))).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let (status, _, body) = send(app.clone(), get(&format!("/v2/projects/{project_id}"))).await;
    assert_eq!(status, StatusCode::OK);
    let record: ProjectRecordView = serde_json::from_slice(&body).expect("the project record");
    let part_a = record.document.part_version_by_identity("P-1", "A").expect("the first revision").part_version_id;
    let part_b = record.document.part_version_by_identity("P-1", "B").expect("the second revision").part_version_id;

    // A project with no manufacturing definition reads as version 0 with its computed issues; the
    // route needs a token and the Engineer permission set.
    let manufacturing_path = format!("/v2/projects/{project_id}/manufacturing");
    let (status, _, body) = send(app.clone(), get(&manufacturing_path)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let view: ManufacturingRecordView = serde_json::from_slice(&body).expect("the manufacturing view");
    assert_eq!(view.version, 0);
    assert!(view.document.parts.is_empty());
    assert_eq!(view.issues.iter().filter(|issue| issue.code == "missing_sourcing").count(), 2);
    let unauthorized = Request::builder().method("GET").uri(&manufacturing_path).body(Body::empty()).expect("a request");
    assert_eq!(send(app.clone(), unauthorized).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(send(app.clone(), get_as(&manufacturing_path, ADMIN_TOKEN)).await.0, StatusCode::FORBIDDEN);

    // A template is published by an administrator only; engineers and administrators both read it.
    let template_id = Uuid::from_u128(0x7810);
    let publish = PublishRoutingTemplate {
        request_id: "template-1".into(),
        expected_version: 0,
        template_id,
        name: "tube routing".into(),
        description: None,
        applies_to: vec![RoutingLevel::Part],
        operations: vec![manufacturing_operation(0x7101, 1, "cut tube", "30")],
        source_reference: Some("routing-standard/1".into()),
    };
    let publish_body = serde_json::to_value(&publish).expect("the template request");
    let (status, _, body) = send(app.clone(), json_as("POST", "/v1/routing-templates", ENGINEER_TOKEN, &publish_body)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{}", String::from_utf8_lossy(&body));
    assert_eq!(value(&body)["error"]["code"], "forbidden");
    let (status, _, body) = send(app.clone(), json_as("POST", "/v1/routing-templates", ADMIN_TOKEN, &publish_body)).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let receipt: solidbom_contract::RoutingTemplateReceipt = serde_json::from_slice(&body).expect("the template receipt");
    assert_eq!(receipt.version, 1);

    let (status, _, body) = send(app.clone(), get_as("/v1/routing-templates", ENGINEER_TOKEN)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let list: solidbom_contract::RoutingTemplateList = serde_json::from_slice(&body).expect("the template list");
    assert_eq!(list.templates.len(), 1);
    assert_eq!(list.templates[0].template_version_id, receipt.template_version_id);
    let template_path = format!("/v1/routing-templates/{template_id}/versions/1");
    let (status, _, body) = send(app.clone(), get_as(&template_path, ADMIN_TOKEN)).await;
    assert_eq!(status, StatusCode::OK);
    let template: solidbom_contract::RoutingTemplateVersion = serde_json::from_slice(&body).expect("the template version");
    assert_eq!(template.operations, publish.operations);
    assert_eq!(send(app.clone(), get(&format!("/v1/routing-templates/{template_id}/versions/9"))).await.0, StatusCode::NOT_FOUND);

    // Sourcing is an Engineer write; an administrator is refused before anything is written.
    let sourcing = SetSourcing {
        request_id: "sourcing-1".into(),
        expected_version: 0,
        parts: vec![manufacturing_purchased(part_a)],
        affected_part_versions: vec![part_a],
    };
    let sourcing_body = serde_json::to_value(&sourcing).expect("the sourcing request");
    let sourcing_path = format!("/v2/projects/{project_id}/manufacturing/sourcing");
    assert_eq!(send(app.clone(), json_as("POST", &sourcing_path, ADMIN_TOKEN, &sourcing_body)).await.0, StatusCode::FORBIDDEN);
    let (status, _, body) = send(app.clone(), json_request("POST", &sourcing_path, &sourcing_body)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let outcome: ManufacturingWriteOutcome = serde_json::from_slice(&body).expect("the sourcing outcome");
    assert_eq!(outcome.receipt.version, 1);
    assert_eq!(outcome.affected_part_versions, vec![part_a]);
    assert!(outcome.affected_products.is_empty());
    assert!(matches!(outcome.facts.as_slice(), [ManufacturingFact::SourcingRecorded { sourcing: Sourcing::Purchased, .. }]));

    // A stale expected version is a conflict and appends nothing.
    let stale = SetSourcing { request_id: "sourcing-stale".into(), expected_version: 0, ..sourcing.clone() };
    let (status, _, body) = send(app.clone(), json_request("POST", &sourcing_path, &serde_json::to_value(&stale).expect("stale"))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(value(&body)["error"]["code"], "conflict");

    // Adopt one exact template version, edit the copy, then enter another routing manually.
    let routings_path = format!("/v2/projects/{project_id}/manufacturing/routings");
    let adopt = RoutingWrite::AdoptTemplate(solidbom_contract::AdoptRoutingTemplate {
        request_id: "adopt-1".into(),
        expected_version: 1,
        routing_id: Uuid::from_u128(0x7820),
        level: RoutingLevel::Part,
        owner: CostOwner::PartVersion { part_version_id: part_a },
        template: TemplateRef { template_id, version: 1 },
        affected_part_versions: vec![part_a],
        affected_products: Vec::new(),
    });
    let (status, _, body) = send(app.clone(), json_request("POST", &routings_path, &serde_json::to_value(&adopt).expect("adopt"))).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let outcome: ManufacturingWriteOutcome = serde_json::from_slice(&body).expect("the adoption outcome");
    assert_eq!(outcome.receipt.version, 2);
    assert_eq!(outcome.affected_part_versions, vec![part_a]);
    assert!(outcome.affected_products.is_empty());
    assert!(
        matches!(outcome.facts.as_slice(), [ManufacturingFact::RoutingWritten { template: Some(found), .. }] if *found == TemplateRef { template_id, version: 1 })
    );
    let edit = RoutingWrite::Edit(EditRouting {
        request_id: "edit-1".into(),
        expected_version: 2,
        routing_id: Uuid::from_u128(0x7820),
        operations: vec![manufacturing_operation(0x7102, 1, "cut tube, edited", "35")],
        affected_part_versions: vec![part_a],
        affected_products: Vec::new(),
    });
    let (status, _, body) = send(app.clone(), json_request("POST", &routings_path, &serde_json::to_value(&edit).expect("edit"))).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(serde_json::from_slice::<ManufacturingWriteOutcome>(&body).expect("the edit outcome").receipt.version, 3);
    let manual = RoutingWrite::Manual(ManualRouting {
        request_id: "manual-1".into(),
        expected_version: 3,
        routing_id: Uuid::from_u128(0x7821),
        level: RoutingLevel::Part,
        owner: CostOwner::PartVersion { part_version_id: part_b },
        operations: vec![manufacturing_operation(0x7103, 1, "cut tube for revision B", "30")],
        affected_part_versions: vec![part_b],
        affected_products: Vec::new(),
    });
    let (status, _, body) = send(app.clone(), json_request("POST", &routings_path, &serde_json::to_value(&manual).expect("manual"))).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(serde_json::from_slice::<ManufacturingWriteOutcome>(&body).expect("the manual outcome").receipt.version, 4);

    // Attached non-CAD cost lines are an Engineer write of the same record.
    let attached = EditAttachedCosts {
        request_id: "attached-1".into(),
        expected_version: 4,
        upsert: vec![manufacturing_line(0x7830, CostOwner::PartVersion { part_version_id: part_a })],
        remove: Vec::new(),
        affected_part_versions: vec![part_a],
        affected_products: Vec::new(),
    };
    let attached_path = format!("/v2/projects/{project_id}/manufacturing/attached-costs");
    let (status, _, body) = send(app.clone(), json_request("POST", &attached_path, &serde_json::to_value(&attached).expect("attached"))).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let attached_outcome: ManufacturingWriteOutcome = serde_json::from_slice(&body).expect("the attached outcome");
    assert_eq!(attached_outcome.receipt.version, 5);
    assert_eq!(attached_outcome.affected_part_versions, vec![part_a]);
    assert!(attached_outcome.affected_products.is_empty());

    // A material version the reference data does not hold is refused before anything is stored.
    let unknown_material = manufacturing_manufactured(part_b, Uuid::new_v4());
    let (status, _, body) = send(
        app.clone(),
        json_request(
            "POST",
            &sourcing_path,
            &serde_json::to_value(SetSourcing {
                request_id: "sourcing-unknown-material".into(),
                expected_version: 5,
                parts: vec![unknown_material],
                affected_part_versions: vec![part_b],
            })
            .expect("the unknown material request"),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{}", String::from_utf8_lossy(&body));

    // The current read reports every record, and a historical version stays readable unchanged.
    let (status, _, body) = send(app.clone(), get(&manufacturing_path)).await;
    assert_eq!(status, StatusCode::OK);
    let view: ManufacturingRecordView = serde_json::from_slice(&body).expect("the manufacturing view");
    assert_eq!(view.version, 5);
    assert_eq!(view.document.parts.len(), 1);
    assert_eq!(view.document.routings.len(), 2);
    assert_eq!(view.document.attached_costs.len(), 1);
    let (status, _, body) = send(app.clone(), get(&format!("{manufacturing_path}/versions/2"))).await;
    assert_eq!(status, StatusCode::OK);
    let historical: ManufacturingRecordView = serde_json::from_slice(&body).expect("the historical manufacturing view");
    assert_eq!(historical.version, 2);
    assert_eq!(historical.document.routings.len(), 1);
    assert!(historical.document.attached_costs.is_empty());
    assert_eq!(send(app.clone(), get(&format!("{manufacturing_path}/versions/99"))).await.0, StatusCode::NOT_FOUND);
    let huge = "18446744073709551615";
    assert_eq!(send(app.clone(), get(&format!("{manufacturing_path}/versions/{huge}"))).await.0, StatusCode::NOT_FOUND);
    assert_eq!(send(app.clone(), get(&format!("/v1/routing-templates/{template_id}/versions/{huge}"))).await.0, StatusCode::NOT_FOUND);
    assert_eq!(send(app.clone(), get(&format!("/v2/projects/{}/manufacturing", Uuid::new_v4()))).await.0, StatusCode::NOT_FOUND);
    db.cleanup().await;
}

/// Create a version 2 project with one delivered flat product, register its exact analysis and
/// group its three bodies into one reviewed component of the shared part version.
async fn reviewed_engineering_project(app: Router) -> (Uuid, Uuid) {
    let source = upload(app.clone()).await;
    let project_id = Uuid::from_u128(0x7800);
    let create = CreateProject {
        request_id: "create-1".into(),
        project_id,
        details: ProjectDetails {
            name: "Engineering suggestions".into(),
            customer: "Customer".into(),
            internal_team: None,
            platform: None,
            application: None,
            currency: "CNY".into(),
            default_schedule: YearRange { sop_year: 2026, eop_year: 2028 },
        },
    };
    let (status, _, body) = send(app.clone(), json_request("POST", "/v2/projects", &serde_json::to_value(&create).expect("create"))).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let delivery = RecordDelivery {
        request_id: "delivery-1".into(),
        expected_version: 1,
        package: DeliveryPackage { package_id: Uuid::from_u128(0x7801), name: "Package 1".into(), source },
        products: vec![DeliveredProductInput {
            product_id: None,
            name: "A".into(),
            kind: ProductKind::Part,
            customer_part_number: None,
            parts: vec![DeliveredPartInput {
                delivered_key: "body-set".into(),
                customer_part_number: Some("PL-100".into()),
                customer_revision: Some("B".into()),
                volume_mm3: Some(d("2400")),
            }],
        }],
    };
    let (status, _, body) = send(
        app.clone(),
        json_request("POST", &format!("/v2/projects/{project_id}/deliveries"), &serde_json::to_value(&delivery).expect("delivery")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let (_, _, body) = send(app.clone(), get(&format!("/v2/projects/{project_id}"))).await;
    let record: ProjectRecordView = serde_json::from_slice(&body).expect("the project record");
    let revision = record.document.products[0].revisions[0].product_revision_id;
    let part_version_id = record.document.products[0].revisions[0].parts[0].part_version_id.expect("the delivered part match");
    let register = RegisterAnalysis { request_id: "analysis-1".into(), product_revision_id: revision, manifest: flat_analysis() };
    let (status, _, body) = send(
        app.clone(),
        json_request("POST", &format!("/v2/projects/{project_id}/engineering/analyses"), &serde_json::to_value(&register).expect("analysis")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let group = GroupSolids {
        request_id: "group-1".into(),
        expected_version: 0,
        component_id: Uuid::from_u128(0xc001),
        product_revision_id: revision,
        analysis_key: engineering_key(),
        geometry: vec![
            ReviewedGeometry { source_path: Vec::new(), source_body: 0 },
            ReviewedGeometry { source_path: Vec::new(), source_body: 1 },
            ReviewedGeometry { source_path: Vec::new(), source_body: 2 },
        ],
        part_version_id: Some(part_version_id),
    };
    let (status, _, body) = send(
        app.clone(),
        json_request("POST", &format!("/v2/projects/{project_id}/engineering/components"), &serde_json::to_value(&group).expect("group")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    (project_id, part_version_id)
}

#[tokio::test]
async fn engineering_suggestion_routes_record_proposals_and_only_confirmation_approves_catalog_inputs() {
    let db = TestDb::new().await;
    let app = api(&db);
    let (project_id, part_version_id) = reviewed_engineering_project(app.clone()).await;

    // Publish one catalog component version as the Administrator identity.
    let component_id = Uuid::from_u128(0xca70);
    let component = support::component_version_with_attributes(
        "catalog-1",
        0,
        component_id,
        "Hose clip 12-20",
        "stainless steel, 12-20 mm",
        Some("0.42"),
        &[("category", "tube")],
    );
    let (status, _, body) = send(
        app.clone(),
        json_as(
            "POST",
            &format!("/v1/shared/components/{component_id}/versions"),
            ADMIN_TOKEN,
            &serde_json::to_value(&component).expect("component"),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let published: ComponentVersionPublished = serde_json::from_slice(&body).expect("the published component");
    let component_version_id = published.component.component_version_id;

    // A proposal recording needs the Engineer permission set.
    let request = RecordSuggestions {
        catalog_evaluated_part_versions: Vec::new(),
        request_id: "suggest-1".into(),
        expected_version: 1,
        classification: vec![ClassificationSuggestion {
            part_version_id,
            prediction: ClassificationPrediction {
                name: "tube".into(),
                model_version: "recognition-rules-1".into(),
                confidence: Some(d("1")),
                prediction_version: Some("1".into()),
                analysis_key: Some(engineering_key()),
                evidence: vec![SuggestionEvidence::new("tubular_sweep", "the exact geometry is one swept constant-diameter tube").expect("evidence")],
                suggested_at: None,
            },
        }],
        catalog: vec![CatalogMatchSuggestion {
            part_version_id,
            component_id,
            component_version_id,
            model_version: "catalog-text-match-1".into(),
            prediction_version: "1".into(),
            confidence: Some(d("1")),
            evidence: vec![
                SuggestionEvidence::new("catalog_attributes_match", "every declared catalog attribute matches the reviewed part").expect("evidence"),
            ],
        }],
    };
    let suggestions_path = format!("/v2/projects/{project_id}/engineering/suggestions");
    let body_json = serde_json::to_value(&request).expect("the suggestion request");
    let anonymous = Request::builder()
        .method("POST")
        .uri(&suggestions_path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&body_json).expect("the body")))
        .expect("a request");
    assert_eq!(send(app.clone(), anonymous).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(send(app.clone(), json_as("POST", &suggestions_path, ADMIN_TOKEN, &body_json)).await.0, StatusCode::FORBIDDEN);
    let (status, _, body) = send(app.clone(), json_as("POST", &suggestions_path, ENGINEER_TOKEN, &body_json)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let outcome: EngineeringWriteOutcome = serde_json::from_slice(&body).expect("the suggestion outcome");
    assert_eq!(outcome.receipt.version, 2);
    assert!(matches!(outcome.facts.as_slice(), [EngineeringFact::SuggestionsRecorded { classification: 1, catalog: 1 }]));

    // An identical retry replays the recorded outcome; a stale version and an unknown identity are
    // refused without appending a version.
    let (status, _, body) = send(app.clone(), json_as("POST", &suggestions_path, ENGINEER_TOKEN, &body_json)).await;
    assert_eq!(status, StatusCode::OK);
    let retry: EngineeringWriteOutcome = serde_json::from_slice(&body).expect("the replay");
    assert_eq!(retry, outcome);
    let mut stale = request.clone();
    stale.request_id = "suggest-stale".into();
    stale.expected_version = 0;
    let (status, _, _) = send(app.clone(), json_as("POST", &suggestions_path, ENGINEER_TOKEN, &serde_json::to_value(&stale).expect("stale"))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let mut unknown_part = request.clone();
    unknown_part.request_id = "suggest-unknown-part".into();
    unknown_part.expected_version = 2;
    unknown_part.catalog.clear();
    unknown_part.classification[0].part_version_id = Uuid::new_v4();
    let (status, _, _) =
        send(app.clone(), json_as("POST", &suggestions_path, ENGINEER_TOKEN, &serde_json::to_value(&unknown_part).expect("unknown part"))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let unknown_project_path = format!("/v2/projects/{}/engineering/suggestions", Uuid::new_v4());
    let mut unknown_project = request.clone();
    unknown_project.request_id = "suggest-unknown-project".into();
    unknown_project.expected_version = 0;
    let (status, _, _) =
        send(app.clone(), json_as("POST", &unknown_project_path, ENGINEER_TOKEN, &serde_json::to_value(&unknown_project).expect("unknown project")))
            .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The review keeps both proposals pending: a confidence-1 prediction never confirms a label and
    // an unconfirmed catalog candidate never supplies a price or specification.
    let review_path = format!("/v2/projects/{project_id}/engineering");
    let (status, _, body) = send(app.clone(), get_as(&review_path, ENGINEER_TOKEN)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let review: EngineeringRecordView = serde_json::from_slice(&body).expect("the engineering review");
    let part = review.document.parts.iter().find(|part| part.part_version_id == part_version_id).expect("the reviewed part");
    assert!(part.classification.confirmed.is_none(), "a 1.0 confidence proposal never confirms a label");
    assert_eq!(part.classification.prediction.as_ref().map(|proposal| proposal.name.as_str()), Some("tube"));
    assert!(part.catalog.confirmed.is_none());
    assert!(part.catalog.effective_inputs().is_none(), "an unconfirmed candidate supplies no cost input");
    let candidate = &part.catalog.suggestions[0];
    assert_eq!(candidate.component_version_id, component_version_id);
    assert_eq!(candidate.catalog_version, 1);
    assert_eq!(candidate.catalog_name, "Hose clip 12-20");
    assert_eq!(candidate.price, Some(d("0.42")));

    // Only the bulk confirmation approves the label and the catalog reference. The project override
    // is recorded separately and the service, not the payload, records the reviewer.
    let confirmation = ConfirmParts {
        request_id: "confirm-1".into(),
        expected_version: 2,
        confirmations: vec![PartConfirmation {
            part_version_id,
            classification: Some(ClassificationConfirmation { name: "tube".into() }),
            confirm_measurement: false,
            catalog: Some(CatalogConfirmation {
                component_version_id,
                project_override: Some(ProjectCatalogOverride {
                    specification: "project stock: stainless steel, 12-20 mm".into(),
                    price: Some(d("0.55")),
                    price_unit: Unit::CurrencyPerPiece,
                    currency: "CNY".into(),
                    reason: "project buys from a different supplier".into(),
                    recorded_by: "client-claimed reviewer".into(),
                }),
            }),
        }],
    };
    let confirmations_path = format!("/v2/projects/{project_id}/engineering/confirmations");
    let (status, _, body) =
        send(app.clone(), json_as("POST", &confirmations_path, ENGINEER_TOKEN, &serde_json::to_value(&confirmation).expect("confirmation"))).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let outcome: EngineeringWriteOutcome = serde_json::from_slice(&body).expect("the confirmation outcome");
    assert_eq!(outcome.receipt.version, 3);
    assert!(outcome.facts.iter().any(|fact| matches!(fact, EngineeringFact::CatalogMatchConfirmed { .. })));

    let (status, _, body) = send(app.clone(), get_as(&review_path, ENGINEER_TOKEN)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let review: EngineeringRecordView = serde_json::from_slice(&body).expect("the engineering review");
    let part = review.document.parts.iter().find(|part| part.part_version_id == part_version_id).expect("the reviewed part");
    assert_eq!(part.classification.confirmed.as_ref().map(|label| label.name.as_str()), Some("tube"));
    let confirmed = part.catalog.confirmed.as_ref().expect("the confirmed catalog reference");
    assert_eq!(confirmed.version, 1);
    assert_eq!(confirmed.specification, "stainless steel, 12-20 mm");
    assert_eq!(confirmed.price, Some(d("0.42")));
    assert_eq!(confirmed.confirmed_by, "engineer");
    let project_override = part.catalog.project_override.as_ref().expect("the recorded project override");
    assert_eq!(project_override.recorded_by, "engineer");
    let inputs = part.catalog.effective_inputs().expect("the override supplies the project inputs");
    assert!(inputs.override_applied);
    assert_eq!(inputs.specification, "project stock: stainless steel, 12-20 mm");
    assert_eq!(inputs.price, Some(d("0.55")));

    // A fresh matching run that evaluates the part and finds no candidate clears the old proposal
    // through the route, while the confirmed reference and the project override stay authoritative.
    let clearing = RecordSuggestions {
        catalog_evaluated_part_versions: vec![part_version_id],
        request_id: "suggest-clear".into(),
        expected_version: 3,
        classification: Vec::new(),
        catalog: Vec::new(),
    };
    let (status, _, body) =
        send(app.clone(), json_as("POST", &suggestions_path, ENGINEER_TOKEN, &serde_json::to_value(&clearing).expect("clearing run"))).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let (status, _, body) = send(app.clone(), get_as(&review_path, ENGINEER_TOKEN)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let review: EngineeringRecordView = serde_json::from_slice(&body).expect("the engineering review");
    let part = review.document.parts.iter().find(|part| part.part_version_id == part_version_id).expect("the reviewed part");
    assert!(part.catalog.suggestions.is_empty(), "a no-match run through the route clears the previous proposals");
    assert_eq!(part.catalog.confirmed.as_ref().map(|confirmed| confirmed.version), Some(1), "the confirmed reference survives");
    assert!(part.catalog.project_override.is_some(), "the project override survives");
    db.cleanup().await;
}

fn feasibility_rule(id: u128, kind: FeasibilityRuleKind, applicability: RuleApplicability, limit: RuleLimit) -> FeasibilityRule {
    FeasibilityRule { rule_id: Uuid::from_u128(id), kind, applicability, limit, source_reference: "ENG06517, 2010-11-05".into(), note: None }
}

fn feasibility_rules_request() -> CreateFeasibilityRuleTableVersion {
    CreateFeasibilityRuleTableVersion {
        request_id: "feasibility-rules-1".into(),
        expected_version: 0,
        rule_table_id: Uuid::from_u128(0xfa01),
        name: "Reviewed design rules".into(),
        rules: vec![
            feasibility_rule(
                1,
                FeasibilityRuleKind::MinimumBendRadius,
                RuleApplicability {
                    tube_kind: Some(TubeKind::Smooth),
                    material_name: Some("PA12".into()),
                    supporting_spring_required: true,
                    ..RuleApplicability::default()
                },
                RuleLimit::MinimumRadiusMm { radius_mm: d("30") },
            ),
            feasibility_rule(2, FeasibilityRuleKind::BendSpacing, RuleApplicability::default(), RuleLimit::MinimumLengthMm { length_mm: d("20") }),
            feasibility_rule(
                3,
                FeasibilityRuleKind::TubeConnectorCompatibility,
                RuleApplicability { connector_component_id: Some(Uuid::from_u128(0xc0)), ..RuleApplicability::default() },
                RuleLimit::CompatibleTubeRange {
                    outer_diameter_mm: NumericRangeMm { min_mm: d("8"), max_mm: d("10") },
                    inner_diameter_mm: NumericRangeMm { min_mm: d("5"), max_mm: d("8") },
                },
            ),
            feasibility_rule(
                4,
                FeasibilityRuleKind::EndAssemblyLength,
                RuleApplicability { connector_component_id: Some(Uuid::from_u128(0xc0)), ..RuleApplicability::default() },
                RuleLimit::MinimumLengthMm { length_mm: d("25") },
            ),
        ],
        source_reference: Some("design rules/".into()),
    }
}

fn feasibility_evidence(revision: Uuid) -> FeasibilityComponentEvidence {
    let connector = FeasibilityConnector { catalog_component_id: Uuid::from_u128(0xc0), catalog_component_version_id: Uuid::from_u128(0xc1) };
    FeasibilityComponentEvidence {
        component_id: Uuid::from_u128(0xc001),
        product_revision_id: revision,
        analysis_key: engineering_key(),
        extraction_version: "1".into(),
        attributes: FeasibilityTubeAttributes {
            tube_kind: Some(TubeKind::Smooth),
            material_name: Some("PA12".into()),
            supporting_spring: Some(true),
            nominal_outer_diameter_mm: Some(d("10")),
            nominal_wall_thickness_mm: Some(d("1")),
            nominal_connection_size_mm: None,
        },
        measurements: Some(FeasibilityTubeMeasurements {
            outer_diameter_mm: d("10"),
            inner_diameter_mm: d("8"),
            bend_radii_mm: vec![d("35"), d("25")],
            tangent_straight_lengths_mm: vec![d("30")],
            straight_end_lengths_mm: vec![d("40"), d("40")],
        }),
        ends: vec![
            FeasibilityTubeEnd { end: TubeEnd::Start, connector: Some(connector) },
            FeasibilityTubeEnd { end: TubeEnd::End, connector: Some(connector) },
        ],
    }
}

#[tokio::test]
async fn feasibility_routes_are_authenticated_permission_checked_and_record_justified_exceptions() {
    let db = TestDb::new().await;
    let app = api(&db);
    let source = upload(app.clone()).await;
    let project_id = Uuid::from_u128(0xf140);
    let create = CreateProject {
        request_id: "create-1".into(),
        project_id,
        details: ProjectDetails {
            name: "Feasibility inquiry".into(),
            customer: "Customer".into(),
            internal_team: None,
            platform: None,
            application: None,
            currency: "CNY".into(),
            default_schedule: YearRange { sop_year: 2026, eop_year: 2028 },
        },
    };
    let (status, _, body) = send(app.clone(), json_request("POST", "/v2/projects", &serde_json::to_value(&create).expect("create"))).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));

    // The record reads as not assessed, needs a token and the Engineer permission set.
    let record_path = format!("/v2/projects/{project_id}/feasibility");
    let unauthorized = Request::builder().method("GET").uri(&record_path).body(Body::empty()).expect("a request");
    assert_eq!(send(app.clone(), unauthorized).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(send(app.clone(), get_as(&record_path, ADMIN_TOKEN)).await.0, StatusCode::FORBIDDEN);
    let (status, _, body) = send(app.clone(), get(&record_path)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let record: FeasibilityRecordView = serde_json::from_slice(&body).expect("the feasibility record");
    assert_eq!(record.version, 0);
    assert!(record.latest_assessment.is_none());
    assert_eq!(record.issues.iter().map(|issue| issue.code.as_str()).collect::<Vec<_>>(), vec!["feasibility_not_assessed"]);

    // Only an Administrator publishes rule versions; only an Engineer reads them.
    let rules = feasibility_rules_request();
    let publish_path = format!("/v1/shared/feasibility-rules/{}/versions", rules.rule_table_id);
    let (status, _, _) = send(app.clone(), json_as("POST", &publish_path, ENGINEER_TOKEN, &serde_json::to_value(&rules).expect("rules"))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, body) = send(app.clone(), json_as("POST", &publish_path, ADMIN_TOKEN, &serde_json::to_value(&rules).expect("rules"))).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let published: FeasibilityRuleTablePublished = serde_json::from_slice(&body).expect("the published rules");
    assert_eq!(published.rule_table.version, 1);
    assert_eq!(published.rule_table.created_by.as_deref(), Some("administrator"));
    let (status, _, _) = send(app.clone(), get_as("/v1/feasibility-rules", ADMIN_TOKEN)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, body) = send(app.clone(), get_as("/v1/feasibility-rules", ENGINEER_TOKEN)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let rules_view: FeasibilityRulesView = serde_json::from_slice(&body).expect("the rule view");
    assert_eq!(rules_view.rule_tables.len(), 1);

    // Deliver the flat product so the revision and part version exist, then review the component.
    let delivery = RecordDelivery {
        request_id: "delivery-1".into(),
        expected_version: 1,
        package: DeliveryPackage { package_id: Uuid::from_u128(0xf141), name: "Package 1".into(), source },
        products: vec![DeliveredProductInput {
            product_id: None,
            name: "A".into(),
            kind: ProductKind::Part,
            customer_part_number: None,
            parts: vec![DeliveredPartInput {
                delivered_key: "body-set".into(),
                customer_part_number: Some("PL-100".into()),
                customer_revision: Some("B".into()),
                volume_mm3: Some(d("2400")),
            }],
        }],
    };
    let (status, _, body) = send(
        app.clone(),
        json_request("POST", &format!("/v2/projects/{project_id}/deliveries"), &serde_json::to_value(&delivery).expect("delivery")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let (_, _, body) = send(app.clone(), get(&format!("/v2/projects/{project_id}"))).await;
    let project: ProjectRecordView = serde_json::from_slice(&body).expect("the project record");
    let revision = project.document.products[0].revisions[0].product_revision_id;
    let part_version_id = project.document.products[0].revisions[0].parts[0].part_version_id.expect("the delivered part match");

    let register = RegisterAnalysis { request_id: "analysis-1".into(), product_revision_id: revision, manifest: flat_analysis() };
    let (status, _, _) = send(
        app.clone(),
        json_request("POST", &format!("/v2/projects/{project_id}/engineering/analyses"), &serde_json::to_value(&register).expect("analysis")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let group = GroupSolids {
        request_id: "group-1".into(),
        expected_version: 0,
        component_id: Uuid::from_u128(0xc001),
        product_revision_id: revision,
        analysis_key: engineering_key(),
        geometry: vec![ReviewedGeometry { source_path: Vec::new(), source_body: 0 }],
        part_version_id: Some(part_version_id),
    };
    let (status, _, body) = send(
        app.clone(),
        json_request("POST", &format!("/v2/projects/{project_id}/engineering/components"), &serde_json::to_value(&group).expect("group")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));

    // An assessment must name an actual reviewed component and a published rule version.
    let unknown = RecordFeasibilityAssessment {
        request_id: "assessment-unknown".into(),
        expected_version: 0,
        rule_table_version_id: published.rule_table.rule_table_version_id,
        components: vec![FeasibilityComponentEvidence { component_id: Uuid::from_u128(0xdead), ..feasibility_evidence(revision) }],
    };
    let (status, _, _) = send(
        app.clone(),
        json_request("POST", &format!("/v2/projects/{project_id}/feasibility/assessments"), &serde_json::to_value(&unknown).expect("assessment")),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    let assessment = RecordFeasibilityAssessment {
        request_id: "assessment-1".into(),
        expected_version: 0,
        rule_table_version_id: published.rule_table.rule_table_version_id,
        components: vec![feasibility_evidence(revision)],
    };
    let (status, _, body) = send(
        app.clone(),
        json_request("POST", &format!("/v2/projects/{project_id}/feasibility/assessments"), &serde_json::to_value(&assessment).expect("assessment")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let outcome: FeasibilityAssessmentOutcome = serde_json::from_slice(&body).expect("the assessment outcome");
    assert_eq!(outcome.receipt.version, 1);
    let failing = outcome
        .assessment
        .findings
        .iter()
        .find(|view| view.finding.key.check == FindingCheck::BendRadius { bend_index: 1 })
        .expect("the tight bend finding");
    assert_eq!(failing.finding.state, FindingState::Fail);

    // An identical retry returns the recorded assessment before the version check; a changed
    // request under a stale expected version is a conflict.
    let (status, _, body) = send(
        app.clone(),
        json_request("POST", &format!("/v2/projects/{project_id}/feasibility/assessments"), &serde_json::to_value(&assessment).expect("assessment")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let retried: FeasibilityAssessmentOutcome = serde_json::from_slice(&body).expect("the retried assessment outcome");
    assert_eq!(retried.receipt, outcome.receipt);
    let mut stale_assessment = assessment.clone();
    stale_assessment.request_id = "assessment-2".into();
    stale_assessment.expected_version = 0;
    let (status, _, _) = send(
        app.clone(),
        json_request("POST", &format!("/v2/projects/{project_id}/feasibility/assessments"), &serde_json::to_value(&stale_assessment).expect("stale")),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // An exception is an Engineer decision; an administrator-only identity is refused.
    let exception = AcceptFeasibilityException {
        request_id: "exception-1".into(),
        expected_version: 1,
        assessment_version: 1,
        finding_key: failing.finding.key.clone(),
        justification: "the customer accepts the tighter bend; reviewed on the bench".into(),
    };
    let exception_path = format!("/v2/projects/{project_id}/feasibility/exceptions");
    let (status, _, _) =
        send(app.clone(), json_as("POST", &exception_path, ADMIN_TOKEN, &serde_json::to_value(&exception).expect("exception"))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, body) =
        send(app.clone(), json_as("POST", &exception_path, ENGINEER_TOKEN, &serde_json::to_value(&exception).expect("exception"))).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let outcome: FeasibilityWriteOutcome = serde_json::from_slice(&body).expect("the exception outcome");
    assert_eq!(outcome.receipt.version, 2);

    // The exception write is idempotent on retry and checks the expected version for a new payload.
    let (status, _, body) =
        send(app.clone(), json_as("POST", &exception_path, ENGINEER_TOKEN, &serde_json::to_value(&exception).expect("exception"))).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let retried: FeasibilityWriteOutcome = serde_json::from_slice(&body).expect("the retried exception outcome");
    assert_eq!(retried.receipt, outcome.receipt);
    let mut stale_exception = exception.clone();
    stale_exception.request_id = "exception-2".into();
    stale_exception.expected_version = 1;
    let (status, _, _) =
        send(app.clone(), json_as("POST", &exception_path, ENGINEER_TOKEN, &serde_json::to_value(&stale_exception).expect("stale"))).await;
    assert_eq!(status, StatusCode::CONFLICT);

    // The visible exception resolves the disposition; it does not change the geometric state.
    let (status, _, body) = send(app.clone(), get_as(&record_path, ENGINEER_TOKEN)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let record: FeasibilityRecordView = serde_json::from_slice(&body).expect("the feasibility record");
    assert_eq!(record.version, 2);
    assert_eq!(record.exceptions.len(), 1);
    assert_eq!(record.exceptions[0].state, ExceptionState::Active);
    assert_eq!(record.exceptions[0].exception.accepted_by, "engineer");
    assert!(record.issues.is_empty(), "{:?}", record.issues);
    let latest = record.latest_assessment.expect("the latest assessment");
    let failing_view = latest.findings.iter().find(|view| view.finding.key == failing.finding.key).expect("the failing view");
    assert_eq!(failing_view.finding.state, FindingState::Fail);
    assert_eq!(failing_view.disposition, FindingDisposition::ExceptionAccepted { exception_id: record.exceptions[0].exception.exception_id });

    // A finding that still fails cannot be closed: closing is refused and an Administrator-only
    // identity cannot close it either.
    let exception_id = record.exceptions[0].exception.exception_id;
    let closure_path = format!("/v2/projects/{project_id}/feasibility/exceptions/{exception_id}/closures");
    let close = CloseFeasibilityException {
        request_id: "closure-1".into(),
        expected_version: 2,
        exception_id,
        reason: "the finding no longer applies".into(),
    };
    let (status, _, _) = send(app.clone(), json_as("POST", &closure_path, ADMIN_TOKEN, &serde_json::to_value(&close).expect("closure"))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, _) = send(app.clone(), json_as("POST", &closure_path, ENGINEER_TOKEN, &serde_json::to_value(&close).expect("closure"))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "the finding still fails, so it must be re-accepted instead of closed");

    // A newer assessment makes the tight bend pass; the accepted decision is now obsolete.
    let mut passing = assessment.clone();
    passing.request_id = "assessment-2b".into();
    passing.expected_version = 2;
    passing.components[0].measurements.as_mut().expect("the measured tube").bend_radii_mm = vec![d("35"), d("40")];
    let (status, _, body) = send(
        app.clone(),
        json_request("POST", &format!("/v2/projects/{project_id}/feasibility/assessments"), &serde_json::to_value(&passing).expect("passing")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));

    // A stale expected version is a conflict; the current version closes the obsolete decision.
    let mut stale_close = close.clone();
    stale_close.request_id = "closure-stale".into();
    let (status, _, _) = send(app.clone(), json_as("POST", &closure_path, ENGINEER_TOKEN, &serde_json::to_value(&stale_close).expect("stale"))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let close_now = CloseFeasibilityException { request_id: "closure-1b".into(), expected_version: 3, ..close };
    let (status, _, body) =
        send(app.clone(), json_as("POST", &closure_path, ENGINEER_TOKEN, &serde_json::to_value(&close_now).expect("close"))).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let closed: FeasibilityWriteOutcome = serde_json::from_slice(&body).expect("the closure outcome");
    assert_eq!(closed.receipt.version, 4);
    let (status, _, body) =
        send(app.clone(), json_as("POST", &closure_path, ENGINEER_TOKEN, &serde_json::to_value(&close_now).expect("close"))).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let retried: FeasibilityWriteOutcome = serde_json::from_slice(&body).expect("the retried closure outcome");
    assert_eq!(retried.receipt, closed.receipt);

    // The closed decision stays visible and is no longer a readiness issue or a disposition.
    let (status, _, body) = send(app.clone(), get_as(&record_path, ENGINEER_TOKEN)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let closed_record: FeasibilityRecordView = serde_json::from_slice(&body).expect("the closed record");
    assert_eq!(closed_record.version, 4);
    assert_eq!(closed_record.exceptions[0].state, ExceptionState::Closed);
    assert_eq!(closed_record.exceptions[0].exception.closed.as_ref().map(|closure| closure.closed_by.as_str()), Some("engineer"));
    assert!(closed_record.issues.is_empty(), "{:?}", closed_record.issues);

    // A retained version stays readable; a never-written version or project is not found.
    let (status, _, body) = send(app.clone(), get(&format!("/v2/projects/{project_id}/feasibility/versions/1"))).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let first: FeasibilityRecordView = serde_json::from_slice(&body).expect("the first record version");
    assert!(first.exceptions.is_empty());
    assert_eq!(send(app.clone(), get(&format!("/v2/projects/{project_id}/feasibility/versions/9"))).await.0, StatusCode::NOT_FOUND);
    assert_eq!(send(app.clone(), get(&format!("/v2/projects/{}/feasibility", Uuid::new_v4()))).await.0, StatusCode::NOT_FOUND);
    db.cleanup().await;
}

#[tokio::test]
async fn costing_and_baseline_routes_are_permission_checked_and_keep_history() {
    let db = TestDb::new().await;
    let app = api(&db);
    let project_id = Uuid::from_u128(0x7900);
    let details = ProjectDetails {
        name: "Costing inquiry".into(),
        customer: "Customer".into(),
        internal_team: None,
        platform: None,
        application: None,
        currency: "CNY".into(),
        default_schedule: YearRange { sop_year: 2027, eop_year: 2027 },
    };
    let create = CreateProject { request_id: "create-1".into(), project_id, details: details.clone() };
    let (status, _, body) = send(app.clone(), json_request("POST", "/v2/projects", &serde_json::to_value(&create).expect("create"))).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));

    // The costing settings route needs a token and the Engineer permission set.
    let costing_path = format!("/v2/projects/{project_id}/costing");
    let settings = UpdateProjectCosting {
        request_id: "costing-1".into(),
        expected_version: 1,
        costing: ProjectCosting { volume_policy: Some(CostingVolumePolicy::Average), exchange_rates: Vec::new(), purchase_prices: Vec::new() },
    };
    let settings_body = serde_json::to_value(&settings).expect("the costing request");
    let no_token = Request::builder()
        .method("PUT")
        .uri(&costing_path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&settings_body).expect("the body")))
        .expect("a request");
    assert_eq!(send(app.clone(), no_token).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(send(app.clone(), json_as("PUT", &costing_path, ADMIN_TOKEN, &settings_body)).await.0, StatusCode::FORBIDDEN);

    let (status, _, body) = send(app.clone(), json_request("PUT", &costing_path, &settings_body)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let written: ProjectWriteOutcome = serde_json::from_slice(&body).expect("the costing write outcome");
    assert_eq!(written.receipt.version, 2);
    assert_eq!(written.receipt.project_id, project_id);
    assert!(written.facts.is_empty());

    // An identical retry returns the recorded outcome; a stale version and a changed payload under
    // a used request id are conflicts.
    let (status, _, body) = send(app.clone(), json_request("PUT", &costing_path, &settings_body)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let retried: ProjectWriteOutcome = serde_json::from_slice(&body).expect("the retried costing outcome");
    assert_eq!(retried.receipt, written.receipt);
    let stale = UpdateProjectCosting { request_id: "costing-stale".into(), expected_version: 1, ..settings.clone() };
    let (status, _, body) = send(app.clone(), json_request("PUT", &costing_path, &serde_json::to_value(&stale).expect("stale"))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(value(&body)["error"]["code"], "conflict");
    let changed = UpdateProjectCosting { request_id: "costing-1".into(), expected_version: 1, costing: ProjectCosting::default() };
    assert_eq!(
        send(app.clone(), json_request("PUT", &costing_path, &serde_json::to_value(&changed).expect("changed"))).await.0,
        StatusCode::CONFLICT
    );

    // A purchase table for a part version the project does not hold is refused with 422.
    let refused = UpdateProjectCosting {
        request_id: "costing-refused".into(),
        expected_version: 2,
        costing: ProjectCosting {
            volume_policy: None,
            exchange_rates: Vec::new(),
            purchase_prices: vec![PurchasePriceTable {
                part_version_id: Uuid::new_v4(),
                currency: "CNY".into(),
                tiers: Vec::new(),
                source_reference: None,
            }],
        },
    };
    let (status, _, body) = send(app.clone(), json_request("PUT", &costing_path, &serde_json::to_value(&refused).expect("refused"))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{}", String::from_utf8_lossy(&body));
    assert_eq!(value(&body)["error"]["code"], "invalid");

    // The baseline route is an Engineer POST that answers 201 with the immutable snapshot.
    let baseline_path = format!("/v2/projects/{project_id}/baseline-calculations");
    let request = CalculateBaseline {
        request_id: "baseline-1".into(),
        project_version: 2,
        engineering_version: None,
        manufacturing_version: None,
        product_revisions: Vec::new(),
    };
    let request_body = serde_json::to_value(&request).expect("the baseline request");
    let unauthorized = Request::builder()
        .method("POST")
        .uri(&baseline_path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&request_body).expect("the body")))
        .expect("a request");
    assert_eq!(send(app.clone(), unauthorized).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(send(app.clone(), json_as("POST", &baseline_path, ADMIN_TOKEN, &request_body)).await.0, StatusCode::FORBIDDEN);
    let invalid = serde_json::to_value(&CalculateBaseline { project_version: 0, ..request.clone() }).expect("the invalid request");
    let (status, _, body) = send(app.clone(), json_request("POST", &baseline_path, &invalid)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{}", String::from_utf8_lossy(&body));
    assert_eq!(value(&body)["error"]["code"], "invalid");

    let (status, _, body) = send(app.clone(), json_request("POST", &baseline_path, &request_body)).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let snapshot: BaselineSnapshot = serde_json::from_slice(&body).expect("the baseline snapshot");
    assert_eq!(snapshot.project_id, project_id);
    assert_eq!(snapshot.project_version, 2);
    assert_eq!(snapshot.engineering_version, 0, "the default record version is recorded even when the record is absent");
    assert_eq!(snapshot.manufacturing_version, 0);
    assert_eq!(snapshot.calculator_version, "solidbom-manufacturing-baseline/1");
    assert_eq!(snapshot.inputs.project_currency, "CNY");
    assert_eq!(snapshot.inputs.volume_policy, Some(CostingVolumePolicy::Average));
    assert!(snapshot.inputs.parts.is_empty());
    assert!(snapshot.inputs.products.is_empty());

    // An identical retry returns the recorded snapshot; a changed payload under the same request id
    // is a conflict.
    let (status, _, body) = send(app.clone(), json_request("POST", &baseline_path, &request_body)).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let retried: BaselineSnapshot = serde_json::from_slice(&body).expect("the retried snapshot");
    assert_eq!(retried, snapshot);
    let changed_request = serde_json::to_value(&CalculateBaseline { project_version: 1, ..request.clone() }).expect("the changed request");
    assert_eq!(send(app.clone(), json_request("POST", &baseline_path, &changed_request)).await.0, StatusCode::CONFLICT);

    // The history route is a bounded Engineer GET, newest first.
    let no_token = Request::builder().method("GET").uri(&baseline_path).body(Body::empty()).expect("a request");
    assert_eq!(send(app.clone(), no_token).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(send(app.clone(), get_as(&baseline_path, ADMIN_TOKEN)).await.0, StatusCode::FORBIDDEN);
    let (status, _, body) = send(app.clone(), get(&baseline_path)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let history: BaselineCalculationList = serde_json::from_slice(&body).expect("the baseline history");
    assert_eq!(history.project_id, project_id);
    assert_eq!(history.calculations.len(), 1);
    assert_eq!(history.calculations[0].calculation_id, snapshot.calculation_id);
    assert_eq!(history.calculations[0].project_version, 2);

    // One snapshot by id; an unknown id and another project's id are not found.
    let snapshot_path = format!("{baseline_path}/{}", snapshot.calculation_id);
    let (status, _, body) = send(app.clone(), get(&snapshot_path)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let read: BaselineSnapshot = serde_json::from_slice(&body).expect("the read snapshot");
    assert_eq!(read, snapshot);
    assert_eq!(send(app.clone(), get(&format!("{baseline_path}/{}", Uuid::new_v4()))).await.0, StatusCode::NOT_FOUND);
    let other_project = Uuid::from_u128(0x7901);
    let other = CreateProject { request_id: "create-2".into(), project_id: other_project, details };
    let (status, _, body) = send(app.clone(), json_request("POST", "/v2/projects", &serde_json::to_value(&other).expect("create"))).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    assert_eq!(
        send(app.clone(), get(&format!("/v2/projects/{other_project}/baseline-calculations/{}", snapshot.calculation_id))).await.0,
        StatusCode::NOT_FOUND
    );
    let (status, _, body) = send(app.clone(), get(&format!("/v2/projects/{other_project}/baseline-calculations"))).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let empty: BaselineCalculationList = serde_json::from_slice(&body).expect("the other project history");
    assert!(empty.calculations.is_empty());
    assert_eq!(send(app.clone(), get(&format!("/v2/projects/{}/baseline-calculations", Uuid::new_v4()))).await.0, StatusCode::NOT_FOUND);
    db.cleanup().await;
}

#[tokio::test]
async fn tooling_and_recovery_routes_are_permission_checked_and_read_only() {
    let db = TestDb::new().await;
    let app = api(&db);
    let project_id = Uuid::from_u128(0x7b00);
    let details = ProjectDetails {
        name: "Tooling inquiry".into(),
        customer: "Customer".into(),
        internal_team: None,
        platform: None,
        application: None,
        currency: "CNY".into(),
        default_schedule: YearRange { sop_year: 2027, eop_year: 2027 },
    };
    let create = CreateProject { request_id: "create-1".into(), project_id, details };
    let (status, _, body) = send(app.clone(), json_request("POST", "/v2/projects", &serde_json::to_value(&create).expect("create"))).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));

    // The tooling settings route is an Engineer PUT: no token is 401, an administrator-only identity
    // is 403, and the accepted write appends one project record version.
    let tooling_path = format!("/v2/projects/{project_id}/tooling");
    let settings = UpdateProjectTooling {
        request_id: "tooling-1".into(),
        expected_version: 1,
        tooling: ProjectTooling { default_treatment: Some(ToolingTreatment::Amortized), entries: Vec::new() },
    };
    let settings_body = serde_json::to_value(&settings).expect("the tooling request");
    let no_token = Request::builder()
        .method("PUT")
        .uri(&tooling_path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&settings_body).expect("the body")))
        .expect("a request");
    assert_eq!(send(app.clone(), no_token).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(send(app.clone(), json_as("PUT", &tooling_path, ADMIN_TOKEN, &settings_body)).await.0, StatusCode::FORBIDDEN);
    let (status, _, body) = send(app.clone(), json_request("PUT", &tooling_path, &settings_body)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let written: ProjectWriteOutcome = serde_json::from_slice(&body).expect("the tooling write outcome");
    assert_eq!(written.receipt.version, 2);
    assert_eq!(written.receipt.project_id, project_id);
    assert!(written.facts.is_empty());

    // An identical retry returns the recorded outcome; a stale version and a changed payload under a
    // used request id are conflicts.
    let (status, _, body) = send(app.clone(), json_request("PUT", &tooling_path, &settings_body)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let retried: ProjectWriteOutcome = serde_json::from_slice(&body).expect("the retried tooling outcome");
    assert_eq!(retried.receipt, written.receipt);
    let stale = UpdateProjectTooling { request_id: "tooling-stale".into(), expected_version: 1, ..settings.clone() };
    let (status, _, body) = send(app.clone(), json_request("PUT", &tooling_path, &serde_json::to_value(&stale).expect("stale"))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(value(&body)["error"]["code"], "conflict");
    let changed = UpdateProjectTooling { request_id: "tooling-1".into(), expected_version: 1, tooling: ProjectTooling::default() };
    assert_eq!(
        send(app.clone(), json_request("PUT", &tooling_path, &serde_json::to_value(&changed).expect("changed"))).await.0,
        StatusCode::CONFLICT
    );

    // An entry that names a part version the project does not hold is refused with 422.
    let refused = UpdateProjectTooling {
        request_id: "tooling-refused".into(),
        expected_version: 2,
        tooling: ProjectTooling {
            default_treatment: None,
            entries: vec![ToolingEntry {
                tool_id: Uuid::new_v4(),
                owner: ToolingOwner::PartVersion { part_version_id: Uuid::new_v4() },
                name: None,
                tool_count: 1,
                cost_per_tool: d("10000"),
                currency: "CNY".into(),
                source_reference: None,
            }],
        },
    };
    let (status, _, body) = send(app.clone(), json_request("PUT", &tooling_path, &serde_json::to_value(&refused).expect("refused"))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{}", String::from_utf8_lossy(&body));
    assert_eq!(value(&body)["error"]["code"], "invalid");

    // The settings route appended a version; a baseline over it records the empty demand and the
    // project version that holds the settings.
    let baseline_path = format!("/v2/projects/{project_id}/baseline-calculations");
    let request = CalculateBaseline {
        request_id: "baseline-1".into(),
        project_version: 2,
        engineering_version: None,
        manufacturing_version: None,
        product_revisions: Vec::new(),
    };
    let (status, _, body) = send(app.clone(), json_request("POST", &baseline_path, &serde_json::to_value(&request).expect("baseline"))).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let snapshot: BaselineSnapshot = serde_json::from_slice(&body).expect("the baseline snapshot");
    assert_eq!(snapshot.project_version, 2);

    // The recovery route is an Engineer POST that stores nothing: no token is 401, an
    // administrator-only identity is 403, a malformed envelope is 422 and an unknown baseline is 404.
    let recovery_path = format!("/v2/projects/{project_id}/tooling-recovery");
    let recovery = EvaluateToolingRecovery { baseline_calculation_id: snapshot.calculation_id, recovery: Default::default() };
    let recovery_body = serde_json::to_value(&recovery).expect("the recovery request");
    let no_token = Request::builder()
        .method("POST")
        .uri(&recovery_path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&recovery_body).expect("the body")))
        .expect("a request");
    assert_eq!(send(app.clone(), no_token).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(send(app.clone(), json_as("POST", &recovery_path, ADMIN_TOKEN, &recovery_body)).await.0, StatusCode::FORBIDDEN);
    let malformed = EvaluateToolingRecovery { baseline_calculation_id: Uuid::nil(), recovery: Default::default() };
    let (status, _, body) = send(app.clone(), json_request("POST", &recovery_path, &serde_json::to_value(&malformed).expect("malformed"))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{}", String::from_utf8_lossy(&body));
    assert_eq!(value(&body)["error"]["code"], "invalid");
    let unknown = EvaluateToolingRecovery { baseline_calculation_id: Uuid::new_v4(), recovery: Default::default() };
    assert_eq!(
        send(app.clone(), json_request("POST", &recovery_path, &serde_json::to_value(&unknown).expect("unknown"))).await.0,
        StatusCode::NOT_FOUND
    );

    // The accepted evaluation reads the settings of the snapshot's project version and returns the
    // deterministic ToolingOutcome; the same request answers the same values and stores nothing.
    let (status, _, body) = send(app.clone(), json_request("POST", &recovery_path, &recovery_body)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let outcome: ToolingOutcome = serde_json::from_slice(&body).expect("the tooling outcome");
    assert_eq!(outcome.treatment, Some(ToolingTreatment::Amortized));
    assert_eq!(outcome.project_investment, Some(d("0")), "an empty tooling setting is a complete zero-investment answer");
    assert_eq!(outcome.state, ResultState::Complete, "{:?}", outcome.issues);
    let (status, _, body) = send(app.clone(), json_request("POST", &recovery_path, &recovery_body)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let repeated: ToolingOutcome = serde_json::from_slice(&body).expect("the repeated tooling outcome");
    assert_eq!(repeated, outcome);
    let (status, _, body) = send(app.clone(), get(&format!("/v2/projects/{project_id}/versions/2"))).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let record: ProjectRecordView = serde_json::from_slice(&body).expect("the project record version");
    assert_eq!(record.document.tooling, Some(ProjectTooling { default_treatment: Some(ToolingTreatment::Amortized), entries: Vec::new() }));
    let (status, _, body) = send(app.clone(), get(&baseline_path)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let history: BaselineCalculationList = serde_json::from_slice(&body).expect("the baseline history");
    assert_eq!(history.calculations.len(), 1, "the recovery evaluation appends no baseline snapshot");

    // An unknown project is not found for both routes. A fresh request id is required: the recorded
    // tooling request above is bound to this project, so replaying it against another path is the
    // changed-payload conflict, not a missing project.
    let missing = Uuid::new_v4();
    let missing_settings = UpdateProjectTooling { request_id: "tooling-missing".into(), expected_version: 1, tooling: ProjectTooling::default() };
    assert_eq!(
        send(
            app.clone(),
            json_request("PUT", &format!("/v2/projects/{missing}/tooling"), &serde_json::to_value(&missing_settings).expect("missing"))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(app.clone(), json_request("POST", &format!("/v2/projects/{missing}/tooling-recovery"), &recovery_body)).await.0,
        StatusCode::NOT_FOUND
    );
    db.cleanup().await;
}

#[tokio::test]
async fn commercial_routes_are_permission_checked_and_append_only() {
    let db = TestDb::new().await;
    let app = api(&db);
    let project_id = Uuid::from_u128(0x7c00);
    let details = ProjectDetails {
        name: "Commercial inquiry".into(),
        customer: "Customer".into(),
        internal_team: None,
        platform: None,
        application: None,
        currency: "CNY".into(),
        default_schedule: YearRange { sop_year: 2027, eop_year: 2027 },
    };
    let create = CreateProject { request_id: "create-1".into(), project_id, details };
    let (status, _, body) = send(app.clone(), json_request("POST", "/v2/projects", &serde_json::to_value(&create).expect("create"))).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));

    // The project commercial settings are an Engineer PUT: no token is 401, an administrator-only
    // identity is 403, and the accepted write appends one project record version.
    let settings_path = format!("/v2/projects/{project_id}/commercial");
    let settings = UpdateProjectCommercial {
        request_id: "commercial-1".into(),
        expected_version: 1,
        commercial: ProjectCommercial { ebit_target_rate: d("0.2"), annual_productivity_rate: d("0.03") },
    };
    let settings_body = serde_json::to_value(&settings).expect("the settings request");
    let no_token = Request::builder()
        .method("PUT")
        .uri(&settings_path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&settings_body).expect("the body")))
        .expect("a request");
    assert_eq!(send(app.clone(), no_token).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(send(app.clone(), json_as("PUT", &settings_path, ADMIN_TOKEN, &settings_body)).await.0, StatusCode::FORBIDDEN);
    let (status, _, body) = send(app.clone(), json_request("PUT", &settings_path, &settings_body)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let written: ProjectWriteOutcome = serde_json::from_slice(&body).expect("the settings outcome");
    assert_eq!(written.receipt.version, 2);
    let (status, _, body) = send(app.clone(), json_request("PUT", &settings_path, &settings_body)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let retried: ProjectWriteOutcome = serde_json::from_slice(&body).expect("the retried settings outcome");
    assert_eq!(retried.receipt, written.receipt, "an identical retry returns the recorded outcome");
    let refused = UpdateProjectCommercial {
        request_id: "commercial-refused".into(),
        expected_version: 2,
        commercial: ProjectCommercial { ebit_target_rate: d("1"), annual_productivity_rate: d("0.03") },
    };
    let (status, _, body) = send(app.clone(), json_request("PUT", &settings_path, &serde_json::to_value(&refused).expect("refused"))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{}", String::from_utf8_lossy(&body));
    assert_eq!(value(&body)["error"]["code"], "invalid");

    // A baseline over that project version is the immutable baseline every scenario pins.
    let baseline_path = format!("/v2/projects/{project_id}/baseline-calculations");
    let baseline = CalculateBaseline {
        request_id: "baseline-1".into(),
        project_version: 2,
        engineering_version: None,
        manufacturing_version: None,
        product_revisions: Vec::new(),
    };
    let (status, _, body) = send(app.clone(), json_request("POST", &baseline_path, &serde_json::to_value(&baseline).expect("baseline"))).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let snapshot: BaselineSnapshot = serde_json::from_slice(&body).expect("the baseline snapshot");

    // Scenario create: no token 401, an administrator-only identity 403, the Engineer write 200.
    let scenarios_path = format!("/v2/projects/{project_id}/commercial-scenarios");
    let scenario_id = Uuid::from_u128(0x7c01);
    let content = |name: &str| CommercialScenarioContent {
        name: name.into(),
        baseline_calculation_id: snapshot.calculation_id,
        manual_prices: Vec::new(),
        tooling: Default::default(),
        productivity_applies_to_tooling: false,
    };
    let create_scenario = CreateCommercialScenario { request_id: "scenario-1".into(), scenario_id, content: content("baseline") };
    let create_body = serde_json::to_value(&create_scenario).expect("the scenario request");
    let no_token = Request::builder()
        .method("PUT")
        .uri(&scenarios_path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&create_body).expect("the body")))
        .expect("a request");
    assert_eq!(send(app.clone(), no_token).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(send(app.clone(), json_as("PUT", &scenarios_path, ADMIN_TOKEN, &create_body)).await.0, StatusCode::FORBIDDEN);
    let (status, _, body) = send(app.clone(), json_request("PUT", &scenarios_path, &create_body)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let created: CommercialScenarioWriteOutcome = serde_json::from_slice(&body).expect("the created scenario");
    assert_eq!(created.scenario.version, 1);
    assert_eq!(created.scenario.project_version, 2);
    assert_eq!(created.scenario.baseline_calculation_id, snapshot.calculation_id);
    let (status, _, body) = send(app.clone(), json_request("PUT", &scenarios_path, &create_body)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let replayed: CommercialScenarioWriteOutcome = serde_json::from_slice(&body).expect("the replayed scenario");
    assert_eq!(replayed.scenario, created.scenario);
    let changed = CreateCommercialScenario { request_id: "scenario-1".into(), scenario_id, content: content("changed") };
    let (status, _, _) = send(app.clone(), json_request("PUT", &scenarios_path, &serde_json::to_value(&changed).expect("changed"))).await;
    assert_eq!(status, StatusCode::CONFLICT);

    // A scenario edit appends version 2; a stale expectation is a conflict.
    let scenario_path = format!("{scenarios_path}/{scenario_id}");
    let update = UpdateCommercialScenario { request_id: "scenario-2".into(), scenario_id, expected_version: 1, content: content("revised") };
    let (status, _, body) = send(app.clone(), json_request("PUT", &scenario_path, &serde_json::to_value(&update).expect("update"))).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let updated: CommercialScenarioWriteOutcome = serde_json::from_slice(&body).expect("the updated scenario");
    assert_eq!(updated.scenario.version, 2);
    assert_eq!(updated.scenario.name, "revised");
    let stale = UpdateCommercialScenario { request_id: "scenario-3".into(), scenario_id, expected_version: 1, content: content("stale") };
    let (status, _, body) = send(app.clone(), json_request("PUT", &scenario_path, &serde_json::to_value(&stale).expect("stale"))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(value(&body)["error"]["code"], "conflict");
    let missing_id = Uuid::new_v4();
    let missing =
        UpdateCommercialScenario { request_id: "scenario-missing".into(), scenario_id: missing_id, expected_version: 1, content: content("missing") };
    assert_eq!(
        send(app.clone(), json_request("PUT", &format!("{scenarios_path}/{missing_id}"), &serde_json::to_value(&missing).expect("missing"))).await.0,
        StatusCode::NOT_FOUND
    );

    // The list returns only the current immutable version.
    let (status, _, body) = send(app.clone(), get(&scenarios_path)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let list: CommercialScenarioList = serde_json::from_slice(&body).expect("the scenario list");
    assert_eq!(list.scenarios.len(), 1);
    assert_eq!(list.scenarios[0].version, 2);

    // The comparison route is an Engineer POST that stores nothing: a malformed envelope is 422,
    // an unknown scenario is 404 and the accepted comparison returns the deterministic answer.
    let compare_path = format!("/v2/projects/{project_id}/commercial-comparisons");
    let compare = CompareCommercialScenarios { baseline_calculation_id: snapshot.calculation_id, scenario_ids: vec![scenario_id] };
    let compare_body = serde_json::to_value(&compare).expect("the comparison request");
    let no_token = Request::builder()
        .method("POST")
        .uri(&compare_path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&compare_body).expect("the body")))
        .expect("a request");
    assert_eq!(send(app.clone(), no_token).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(send(app.clone(), json_as("POST", &compare_path, ADMIN_TOKEN, &compare_body)).await.0, StatusCode::FORBIDDEN);
    let malformed = CompareCommercialScenarios { baseline_calculation_id: Uuid::nil(), scenario_ids: vec![scenario_id] };
    let (status, _, body) = send(app.clone(), json_request("POST", &compare_path, &serde_json::to_value(&malformed).expect("malformed"))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{}", String::from_utf8_lossy(&body));
    assert_eq!(value(&body)["error"]["code"], "invalid");
    let unknown = CompareCommercialScenarios { baseline_calculation_id: snapshot.calculation_id, scenario_ids: vec![Uuid::new_v4()] };
    assert_eq!(
        send(app.clone(), json_request("POST", &compare_path, &serde_json::to_value(&unknown).expect("unknown"))).await.0,
        StatusCode::NOT_FOUND
    );
    let (status, _, body) = send(app.clone(), json_request("POST", &compare_path, &compare_body)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let comparison: CommercialScenarioComparison = serde_json::from_slice(&body).expect("the comparison");
    assert_eq!(comparison.project_id, project_id);
    assert_eq!(comparison.baseline_calculation_id, snapshot.calculation_id);
    assert_eq!(comparison.ebit_target_rate, Some(d("0.2")));
    assert_eq!(comparison.scenarios.len(), 1);
    assert!(!comparison.scenarios[0].stale, "{:?}", comparison.scenarios[0].stale_reasons);
    assert_eq!(comparison.scenarios[0].version, 2);

    // The comparison stored no scenario version, project version or baseline.
    let (status, _, body) = send(app.clone(), get(&scenarios_path)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let list: CommercialScenarioList = serde_json::from_slice(&body).expect("the scenario list");
    assert_eq!(list.scenarios.len(), 1);
    assert_eq!(list.scenarios[0].version, 2);
    let (status, _, body) = send(app.clone(), get(&format!("/v2/projects/{project_id}"))).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let view: ProjectRecordView = serde_json::from_slice(&body).expect("the project record view");
    assert_eq!(view.version, 2, "a comparison appends no project version");
    db.cleanup().await;
}

#[tokio::test]
async fn activation_routes_are_permission_checked_and_keep_the_accepted_history() {
    let db = TestDb::new().await;
    let app = api(&db);
    // Seed a fully reviewed, priced and feasible single-product project: reviewed project records
    // through the engineering routes, then the definition records their own route suites cover.
    let (project_id, part_version_id) = reviewed_engineering_project(app.clone()).await;
    let (_, _, body) = send(app.clone(), get(&format!("/v2/projects/{project_id}"))).await;
    let record: ProjectRecordView = serde_json::from_slice(&body).expect("the project record");
    let product_id = record.document.products[0].product_id;
    let revision = record.document.products[0].revisions[0].product_revision_id;

    // The quoted product needs one positive production year for a complete cost.
    let product = UpdateProduct {
        request_id: "activation-product".into(),
        expected_version: 2,
        product_id,
        name: "A".into(),
        kind: ProductKind::Part,
        customer_part_number: None,
        schedule_override: Some(YearRange { sop_year: 2027, eop_year: 2027 }),
        forecasts: vec![AnnualForecast { year: 2027, quantity: 100 }],
    };
    let (status, _, body) = send(
        app.clone(),
        json_request("PUT", &format!("/v2/projects/{project_id}/products/{product_id}"), &serde_json::to_value(&product).expect("product")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));

    // The flat analysis needs every body associated; the helper grouped only the first.
    let group = GroupSolids {
        request_id: "activation-group-all".into(),
        expected_version: 1,
        component_id: Uuid::from_u128(0xc001),
        product_revision_id: revision,
        analysis_key: engineering_key(),
        geometry: vec![
            ReviewedGeometry { source_path: Vec::new(), source_body: 0 },
            ReviewedGeometry { source_path: Vec::new(), source_body: 1 },
            ReviewedGeometry { source_path: Vec::new(), source_body: 2 },
        ],
        part_version_id: Some(part_version_id),
    };
    let (status, _, body) = send(
        app.clone(),
        json_request("POST", &format!("/v2/projects/{project_id}/engineering/components"), &serde_json::to_value(&group).expect("group")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));

    let measurement = CompleteMeasurement {
        request_id: "activation-measurement".into(),
        expected_version: 2,
        part_version_id,
        volume_mm3: Measurement {
            value: Some(d("2400")),
            unit: Unit::MillimetreCubed,
            source: MeasurementSource::Manual,
            review: ReviewState::Pending,
            reason: Some("open surface; measured on the bench".into()),
            analysis_key: engineering_key(),
        },
    };
    let (status, _, body) = send(
        app.clone(),
        json_request(
            "POST",
            &format!("/v2/projects/{project_id}/engineering/measurements"),
            &serde_json::to_value(&measurement).expect("measurement"),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let confirm = ConfirmParts {
        request_id: "activation-confirm".into(),
        expected_version: 3,
        confirmations: vec![PartConfirmation {
            part_version_id,
            classification: Some(ClassificationConfirmation { name: "bracket".into() }),
            confirm_measurement: true,
            catalog: None,
        }],
    };
    let (status, _, body) = send(
        app.clone(),
        json_request("POST", &format!("/v2/projects/{project_id}/engineering/confirmations"), &serde_json::to_value(&confirm).expect("confirm")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));

    // One manufactured sourcing record and one part routing at the seeded fixtures.
    let sourcing = SetSourcing {
        request_id: "activation-sourcing".into(),
        expected_version: 0,
        parts: vec![SourcedPart {
            part_version_id,
            sourcing: Sourcing::Manufactured,
            material: Some(MaterialConsumption {
                material_version_id: Some(seeded_material_version_id()),
                quantity: Some(d("100")),
                unit: Unit::CentimetreCubed,
                source: MeasurementSource::Extracted,
                review: ReviewState::Confirmed,
                confirmed_by: None,
                analysis_key: Some(engineering_key()),
                reason: None,
                allowance_rate: Some(d("0.02")),
                source_reference: None,
            }),
            source_reference: None,
        }],
        affected_part_versions: vec![part_version_id],
    };
    let (status, _, body) = send(
        app.clone(),
        json_request("POST", &format!("/v2/projects/{project_id}/manufacturing/sourcing"), &serde_json::to_value(&sourcing).expect("sourcing")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let routing = RoutingWrite::Manual(ManualRouting {
        request_id: "activation-routing".into(),
        expected_version: 1,
        routing_id: Uuid::from_u128(0x8a99),
        level: RoutingLevel::Part,
        owner: CostOwner::PartVersion { part_version_id },
        operations: vec![RoutingOperation {
            operation_id: Uuid::from_u128(0x7199),
            sequence: 1,
            name: "cut tube".into(),
            description: None,
            cost_center_id: None,
            cycle: Some(OperationCycle {
                seconds: Some(d("30")),
                basis: CycleBasis::PerPiece,
                cavities: None,
                production_rate_version_id: Some(seeded_rate_version_id()),
                origin: CycleOrigin::Manual,
                source_reference: None,
            }),
            setup: None,
            tooling_reference: None,
        }],
        affected_part_versions: vec![part_version_id],
        affected_products: Vec::new(),
    });
    let (status, _, body) = send(
        app.clone(),
        json_request("POST", &format!("/v2/projects/{project_id}/manufacturing/routings"), &serde_json::to_value(&routing).expect("routing")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));

    // The app cost configuration and the project costing settings.
    let config = app_config_version("activation-config", 0, Uuid::from_u128(0xcf99), Vec::new());
    let (status, _, body) = send(
        app.clone(),
        json_as(
            "POST",
            &format!("/v1/shared/app-config/{}/versions", config.config_id),
            ADMIN_TOKEN,
            &serde_json::to_value(&config).expect("config"),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let costing = UpdateProjectCosting {
        request_id: "activation-costing".into(),
        expected_version: 3,
        costing: ProjectCosting { volume_policy: Some(CostingVolumePolicy::Average), exchange_rates: Vec::new(), purchase_prices: Vec::new() },
    };
    let (status, _, body) =
        send(app.clone(), json_request("PUT", &format!("/v2/projects/{project_id}/costing"), &serde_json::to_value(&costing).expect("costing")))
            .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));

    // One published rule table and one passing feasibility assessment.
    let rules = feasibility_rules_request();
    let (status, _, body) = send(
        app.clone(),
        json_as(
            "POST",
            &format!("/v1/shared/feasibility-rules/{}/versions", rules.rule_table_id),
            ADMIN_TOKEN,
            &serde_json::to_value(&rules).expect("rules"),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let published: FeasibilityRuleTablePublished = serde_json::from_slice(&body).expect("the published rules");
    let mut evidence = feasibility_evidence(revision);
    evidence.measurements.as_mut().expect("the measured tube").bend_radii_mm = vec![d("35"), d("40")];
    let assessment = RecordFeasibilityAssessment {
        request_id: "activation-assessment".into(),
        expected_version: 0,
        rule_table_version_id: published.rule_table.rule_table_version_id,
        components: vec![evidence],
    };
    let (status, _, body) = send(
        app.clone(),
        json_request("POST", &format!("/v2/projects/{project_id}/feasibility/assessments"), &serde_json::to_value(&assessment).expect("assessment")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));

    // The readiness route needs a token and the Engineer permission set; it is a read.
    let readiness_path = format!("/v2/projects/{project_id}/activation-readiness");
    let request = ActivationReadinessRequest {
        project_version: 4,
        selections: vec![solidbom_contract::ProductRevisionSelection { product_id, product_revision_id: revision }],
    };
    let request_body = serde_json::to_value(&request).expect("the readiness request");
    let no_token = Request::builder()
        .method("POST")
        .uri(&readiness_path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&request_body).expect("the body")))
        .expect("a request");
    assert_eq!(send(app.clone(), no_token).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(send(app.clone(), json_as("POST", &readiness_path, ADMIN_TOKEN, &request_body)).await.0, StatusCode::FORBIDDEN);
    let (status, _, body) = send(app.clone(), json_request("POST", &readiness_path, &request_body)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let report: ActivationReadinessReport = serde_json::from_slice(&body).expect("the readiness report");
    assert_eq!(report.project_version, 4);
    assert!(report.accepted_baseline_id.is_none());
    assert_eq!(report.members.len(), 1);
    assert_eq!(report.members[0].state, solidbom_contract::ReadinessState::Ready, "{:?}", report.members[0].blockers);
    assert_eq!(
        send(app.clone(), json_request("POST", &format!("/v2/projects/{}/activation-readiness", Uuid::new_v4()), &request_body)).await.0,
        StatusCode::NOT_FOUND
    );

    // The activation route publishes the receipt; an identical retry returns the same receipt.
    let activation = ActivateRevisions {
        request_id: "activation-1".into(),
        expected_project_version: 4,
        expected_accepted_baseline_id: None,
        product_revisions: request.selections.clone(),
    };
    let activation_path = format!("/v2/projects/{project_id}/activations");
    let activation_body = serde_json::to_value(&activation).expect("the activation request");
    assert_eq!(send(app.clone(), json_as("POST", &activation_path, ADMIN_TOKEN, &activation_body)).await.0, StatusCode::FORBIDDEN);
    let (status, _, body) = send(app.clone(), json_request("POST", &activation_path, &activation_body)).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let receipt: ActivationReceipt = serde_json::from_slice(&body).expect("the activation receipt");
    assert_eq!(receipt.pinned.project_version, 5);
    assert_eq!(receipt.members.len(), 1);
    assert_eq!(receipt.members[0].action, solidbom_contract::ActivationAction::Activated);
    assert_eq!(receipt.costing_state, ResultState::Complete);
    let (status, _, body) = send(app.clone(), json_request("POST", &activation_path, &activation_body)).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let retried: ActivationReceipt = serde_json::from_slice(&body).expect("the retried receipt");
    assert_eq!(retried, receipt);
    // A stale expected project version is a 409 conflict and writes nothing.
    let stale = ActivateRevisions { request_id: "activation-stale".into(), expected_project_version: 4, ..activation.clone() };
    let (status, _, body) = send(app.clone(), json_request("POST", &activation_path, &serde_json::to_value(&stale).expect("stale"))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(value(&body)["error"]["code"], "conflict");
    assert!(value(&body)["error"]["message"].as_str().is_some_and(|message| message.starts_with("stale_project_version: ")));

    // The accepted history is readable newest first and by id; an unknown id is not found.
    let history_path = format!("/v2/projects/{project_id}/accepted-baselines");
    let (status, _, body) = send(app.clone(), get(&history_path)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let history: AcceptedBaselineList = serde_json::from_slice(&body).expect("the accepted history");
    assert_eq!(history.baselines.len(), 1);
    assert_eq!(history.baselines[0].accepted_baseline_id, receipt.accepted_baseline_id);
    assert_eq!(history.baselines[0].sequence, 1);
    let (status, _, body) = send(app.clone(), get(&format!("{history_path}/{}", receipt.accepted_baseline_id))).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let stored: AcceptedBaseline = serde_json::from_slice(&body).expect("the accepted baseline");
    assert_eq!(stored.costing_calculation_id, receipt.costing_calculation_id);
    assert_eq!(stored.active_revisions.len(), 1);
    assert!(!stored.shared_definitions.is_empty());
    assert_eq!(send(app.clone(), get(&format!("{history_path}/{}", Uuid::new_v4()))).await.0, StatusCode::NOT_FOUND);
    db.cleanup().await;
}

#[tokio::test]
async fn dependency_update_routes_are_permission_checked_and_append_only() {
    let db = TestDb::new().await;
    let app = api(&db);
    let store = db.store.clone();
    let project_id = Uuid::from_u128(0x5200_0900);
    store
        .create_project(&CreateProject {
            request_id: "dependency-create".into(),
            project_id,
            details: ProjectDetails {
                name: "Dependency inquiry".into(),
                customer: "Customer".into(),
                internal_team: None,
                platform: None,
                application: None,
                currency: "CNY".into(),
                default_schedule: YearRange { sop_year: 2027, eop_year: 2027 },
            },
        })
        .await
        .expect("create the project");
    let config_id = Uuid::from_u128(0xcf00_0900);
    let config_v1 = store
        .create_app_config_version(&support::app_config_version("route-config-1", 0, config_id, Vec::new()), "administrator")
        .await
        .expect("publish the first configuration");
    let baseline = store
        .calculate_baseline(
            project_id,
            &CalculateBaseline {
                request_id: "route-baseline".into(),
                project_version: 1,
                engineering_version: None,
                manufacturing_version: None,
                product_revisions: Vec::new(),
            },
        )
        .await
        .expect("calculate the baseline that records configuration v1");
    assert_eq!(baseline.inputs.category_config.as_ref().map(|config| config.config_version_id), Some(config_v1.config.config_version_id));
    let config_v2 = store
        .create_app_config_version(
            &support::app_config_version("route-config-2", 1, config_id, vec![support::fixed_category("packaging", "5")]),
            "administrator",
        )
        .await
        .expect("publish the second configuration")
        .config;

    // 401 without a token, 403 for an identity without the Engineer permission set.
    assert_eq!(
        send(app.clone(), Request::builder().method("GET").uri("/v2/shared-updates").body(Body::empty()).expect("a request")).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(send(app.clone(), get_as("/v2/shared-updates", ADMIN_TOKEN)).await.0, StatusCode::FORBIDDEN);
    let (status, _, body) = send(app.clone(), get("/v2/shared-updates")).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let updates: SharedUpdateList = serde_json::from_slice(&body).expect("the shared-update catalog");
    assert!(updates.updates.iter().any(|update| update.kind == SharedUpdateKind::AppConfig && update.version_id == config_v2.config_version_id));

    // The two project reads answer 200; unknown filter values and an unknown project are refused.
    let list_path = format!("/v2/dependency-projects?kind=app_config&record_id={config_id}");
    let (status, _, body) = send(app.clone(), get(&list_path)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let list: DependencyProjectList = serde_json::from_slice(&body).expect("the dependency project list");
    assert_eq!(list.projects.len(), 1);
    assert_eq!(list.projects[0].project_id, project_id);
    assert_eq!(send(app.clone(), get("/v2/dependency-projects?kind=app_config")).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(send(app.clone(), get(&format!("/v2/dependency-projects?kind=app_config&record_id={config_id}"))).await.0, StatusCode::OK);
    let report_path = format!("/v2/projects/{project_id}/dependency-report");
    let requests_before = row_count(&store, "requests").await;
    let snapshots_before = row_count(&store, "baseline_calculations").await;
    let (status, _, body) = send(app.clone(), get(&report_path)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let report: ProjectDependencyReport = serde_json::from_slice(&body).expect("the dependency report");
    assert_eq!(report.project_version, 1);
    assert_eq!(report.pending_count, 1);
    assert_eq!(report.dependencies.len(), 1);
    assert_eq!(report.dependencies[0].kind, SharedUpdateKind::AppConfig);
    assert_eq!(report.dependencies[0].referenced_version, 1);
    assert_eq!(report.dependencies[0].available_version_id, config_v2.config_version_id);
    assert_eq!(send(app.clone(), get(&format!("/v2/projects/{}/dependency-report", Uuid::new_v4()))).await.0, StatusCode::NOT_FOUND);
    assert_eq!(row_count(&store, "requests").await, requests_before, "the dependency reads append no request receipt");
    assert_eq!(row_count(&store, "baseline_calculations").await, snapshots_before, "the dependency reads append no snapshot");

    // A decision is 201, an identical replay is 201 with the same receipt, and a changed payload
    // under the same request id is a 409 conflict.
    let decision = RecordDependencyDecisions {
        request_id: "route-ack".into(),
        expected_version: 1,
        action: DependencyDecisionAction::Acknowledged,
        decisions: vec![DependencyDecisionItem { kind: SharedUpdateKind::AppConfig, record_id: config_id, version_id: config_v2.config_version_id }],
    };
    let decisions_path = format!("/v2/projects/{project_id}/dependency-decisions");
    let (status, _, body) = send(app.clone(), json_request("POST", &decisions_path, &serde_json::to_value(&decision).expect("decision"))).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let receipt: DependencyDecisionReceipt = serde_json::from_slice(&body).expect("the decision receipt");
    assert_eq!(receipt.decisions.len(), 1);
    assert_eq!(receipt.decisions[0].sequence, 1);
    assert_eq!(receipt.engineering_version, 0, "the project has no engineering record yet");
    assert_eq!(receipt.manufacturing_version, 0, "the project has no manufacturing record yet");
    let (status, _, body) = send(app.clone(), json_request("POST", &decisions_path, &serde_json::to_value(&decision).expect("decision"))).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    assert_eq!(serde_json::from_slice::<DependencyDecisionReceipt>(&body).expect("the retried receipt"), receipt);
    let changed = RecordDependencyDecisions { action: DependencyDecisionAction::Adopted, ..decision.clone() };
    let (status, _, body) =
        send(app.clone(), json_request("POST", &decisions_path, &serde_json::to_value(&changed).expect("changed decision"))).await;
    assert_eq!(status, StatusCode::CONFLICT, "{}", String::from_utf8_lossy(&body));
    assert_eq!(value(&body)["error"]["code"], "conflict");
    let (status, _, body) = send(app.clone(), get(&report_path)).await;
    assert_eq!(status, StatusCode::OK);
    let after_ack: ProjectDependencyReport = serde_json::from_slice(&body).expect("the report after acknowledgement");
    assert!(after_ack.dependencies[0].acknowledged);
    assert_eq!(after_ack.pending_count, 1, "acknowledgement keeps the dependency pending");

    // An explicit publication is 201; a stale expected generation is a 409 conflict and writes nothing.
    let recalculate_path = format!("/v2/projects/{project_id}/draft-recalculations");
    let publication = RecalculateDraft {
        request_id: "route-recalculate".into(),
        expected_project_version: 1,
        expected_engineering_version: 0,
        expected_manufacturing_version: 0,
        product_revisions: Vec::new(),
    };
    let (status, _, body) =
        send(app.clone(), json_request("POST", &recalculate_path, &serde_json::to_value(&publication).expect("publication"))).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let published: DraftPublication = serde_json::from_slice(&body).expect("the publication");
    assert_eq!(published.calculation.project_version, 1);
    assert_eq!(published.calculation.engineering_version, 0);
    assert_eq!(published.calculation.manufacturing_version, 0);
    assert_eq!(published.dependencies.len(), 1, "the acknowledged app configuration is still pending after the publication");
    let stale = RecalculateDraft { request_id: "route-stale".into(), expected_project_version: 2, ..publication.clone() };
    let (status, _, body) =
        send(app.clone(), json_request("POST", &recalculate_path, &serde_json::to_value(&stale).expect("stale publication"))).await;
    assert_eq!(status, StatusCode::CONFLICT, "{}", String::from_utf8_lossy(&body));
    assert!(value(&body)["error"]["message"].as_str().is_some_and(|message| message.starts_with("stale_draft_generation: ")));
    db.cleanup().await;
}

/// The internal-export route is authenticated, Engineer-only and a pure read. It refuses a body that
/// names another project than the route and an unknown saved result, and it renders the current
/// draft result for both formats without appending a project version or a snapshot.
#[tokio::test]
async fn export_routes_are_permission_checked_and_read_only() {
    let db = TestDb::new().await;
    let app = api(&db);
    let project_id = Uuid::from_u128(0x7A21);
    let details = ProjectDetails {
        name: "Export inquiry".into(),
        customer: "Customer".into(),
        internal_team: None,
        platform: None,
        application: None,
        currency: "CNY".into(),
        default_schedule: YearRange { sop_year: 2027, eop_year: 2027 },
    };
    let create = CreateProject { request_id: "export-create".into(), project_id, details };
    let (status, _, body) = send(app.clone(), json_request("POST", "/v2/projects", &serde_json::to_value(&create).expect("create"))).await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));

    // One immutable baseline snapshot of the empty project: the export route renders saved values
    // only, so it never costs anything itself.
    let calculate = CalculateBaseline {
        request_id: "export-baseline".into(),
        project_version: 1,
        engineering_version: None,
        manufacturing_version: None,
        product_revisions: Vec::new(),
    };
    let (status, _, body) = send(
        app.clone(),
        json_request("POST", &format!("/v2/projects/{project_id}/baseline-calculations"), &serde_json::to_value(&calculate).expect("baseline")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&body));
    let snapshot: BaselineSnapshot = serde_json::from_slice(&body).expect("the baseline snapshot");
    let calculation_id = snapshot.calculation_id;

    let path = format!("/v2/projects/{project_id}/exports");
    let request = ExportRequest {
        project_id,
        baseline: ExportBaselineRef::DraftCalculation { calculation_id },
        scenarios: Vec::new(),
        format: ExportFormat::Xlsx,
    };
    let request_json = serde_json::to_value(&request).expect("the export request");

    // A missing token is unauthorized and an administrator-only identity is forbidden.
    let no_token = Request::builder()
        .method("POST")
        .uri(&path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&request_json).expect("the body")))
        .expect("a request");
    assert_eq!(send(app.clone(), no_token).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(send(app.clone(), json_as("POST", &path, ADMIN_TOKEN, &request_json)).await.0, StatusCode::FORBIDDEN);

    // The route and the body must name the same project.
    let mut foreign = request_json.clone();
    foreign["project_id"] = serde_json::json!(Uuid::from_u128(0x7A29).to_string());
    let (status, _, body) = send(app.clone(), json_request("POST", &path, &foreign)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{}", String::from_utf8_lossy(&body));
    assert_eq!(value(&body)["error"]["code"], "invalid");

    // An unknown saved result is not found; nothing is rendered and nothing is written.
    let unknown = ExportRequest { baseline: ExportBaselineRef::DraftCalculation { calculation_id: Uuid::from_u128(0x7A22) }, ..request.clone() };
    let (status, _, body) = send(app.clone(), json_request("POST", &path, &serde_json::to_value(&unknown).expect("unknown draft"))).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{}", String::from_utf8_lossy(&body));
    let unknown_baseline = ExportRequest {
        baseline: ExportBaselineRef::AcceptedBaseline { accepted_baseline_id: Uuid::from_u128(0x7A23), sequence: 1 },
        ..request.clone()
    };
    let (status, _, body) = send(app.clone(), json_request("POST", &path, &serde_json::to_value(&unknown_baseline).expect("unknown baseline"))).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{}", String::from_utf8_lossy(&body));

    // The current draft result renders in both formats with its metadata and stable bytes.
    let (status, _, body) = send(app.clone(), json_request("POST", &path, &request_json)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let workbook: ExportBundle = serde_json::from_slice(&body).expect("the workbook bundle");
    assert!(workbook.validate().is_ok(), "the workbook bundle is valid");
    assert!(workbook.bytes.starts_with(b"PK"), "the workbook is an OOXML package");
    assert_eq!(workbook.metadata.baseline, ExportBaselineSelection::DraftCalculation { calculation_id });
    assert!(workbook.metadata.labels.contains(&ExportLabel::DraftResult));
    assert!(!workbook.metadata.versions.feasibility_pinned);
    assert!(workbook.file_name.ends_with(".xlsx"));

    let pdf_request = ExportRequest { format: ExportFormat::Pdf, ..request.clone() };
    let (status, _, body) = send(app.clone(), json_request("POST", &path, &serde_json::to_value(&pdf_request).expect("pdf request"))).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let document: ExportBundle = serde_json::from_slice(&body).expect("the PDF bundle");
    assert!(document.validate().is_ok(), "the PDF bundle is valid");
    assert!(document.bytes.starts_with(b"%PDF-"), "the summary is a PDF document");
    assert!(document.file_name.ends_with(".pdf"));

    // A pure read: the project version and the baseline history are unchanged by the exports.
    let (status, _, body) = send(app.clone(), get(&format!("/v2/projects/{project_id}/baseline-calculations"))).await;
    assert_eq!(status, StatusCode::OK);
    let history: BaselineCalculationList = serde_json::from_slice(&body).expect("the baseline history");
    assert_eq!(history.calculations.len(), 1);
    assert_eq!(history.calculations[0].calculation_id, calculation_id);
    db.cleanup().await;
}
