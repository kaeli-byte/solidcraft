//! HTTP integration tests for the SolidBOM API.
//!
//! `#[tokio::test]` is not recognised as a test function by Clippy's test allowance, so this
//! file allows the panicking helpers it needs. The library and binary still deny them.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

mod support;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header};
use solidbom_contract::{CalculateRequest, CalculationSnapshot, ProjectView, ReferenceData, ResultState, SaveDraft, SaveReceipt, SourceRef};
use solidbom_server::api::{ApiState, router};
use support::{FIXTURE_SOURCE, TestDb, d, digest, fixture_draft};
use tower::ServiceExt;

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
    assert_eq!(view.calculations.len(), 2);
    assert_eq!(view.calculations.iter().map(|summary| summary.saved_version).collect::<Vec<_>>(), [1, 2]);
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
