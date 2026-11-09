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
    CalculateRequest, CalculationSnapshot, CreateProject, DeliveredPartInput, DeliveredProductInput, DeliveryPackage, ProductKind, ProjectDetails,
    ProjectFact, ProjectRecordView, ProjectView, ProjectWriteOutcome, RecordDelivery, ReferenceData, ResultState, SaveDraft, SaveReceipt, SourceRef,
    YearRange,
};
use solidbom_server::api::{ApiState, router};
use support::{FIXTURE_SOURCE, TestDb, d, digest, fixture_draft, row_count};
use tower::ServiceExt;
use uuid::Uuid;

const TOKEN: &str = "solidbom-test-token-0123456789";
const BODY_LIMIT: usize = 64 * 1024 * 1024;

fn api(db: &TestDb) -> Router {
    router(ApiState::new(db.store.clone(), TOKEN))
}

fn authed(builder: axum::http::request::Builder) -> axum::http::request::Builder {
    builder.header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
}

fn get(path: &str) -> Request<Body> {
    authed(Request::builder().method("GET").uri(path)).body(Body::empty()).expect("a request")
}

fn json_request(method: &str, path: &str, value: &serde_json::Value) -> Request<Body> {
    authed(Request::builder().method(method).uri(path).header(header::CONTENT_TYPE, "application/json"))
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
    assert!(data.materials.iter().any(|material| material.density_g_cm3 == d("2.7") && material.price_per_kg == d("20")));
    assert!(data.rates.iter().any(|rate| rate.rate_per_hour == d("120")));
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
async fn version_two_project_records_are_authenticated_checked_and_never_activated_by_this_contract() {
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

    // A stale expected version is a conflict, and no activation route is published.
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
    db.cleanup().await;
}
