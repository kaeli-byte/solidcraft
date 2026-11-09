//! The v1 HTTP API over the SolidBOM store.
//!
//! One explicit development bearer identity protects every route except `/health`. Errors carry a
//! stable code and a client-safe message, never a database error, path or token. Uploads are
//! bounded and hashed; every protected request has a bounded response time; a calculation is only
//! for a persisted version and resolves its reference values on this side.
//!
//! Routes, statuses, error codes and idempotency rules are the published version 1 contract in
//! `docs/solidbom/contracts/v1/http-api.md`; this module only implements it.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use solidbom_contract::{
    ApiErrorBody, ApiErrorCode, CalculateRequest, ConfirmPartMatch, CreateProject, ExcludeProducts, RecordDelivery, ReinstateProducts, SaveDraft,
    SourceRef, UpdateProduct, UpdateProject,
};
use uuid::Uuid;

use crate::store::{Store, StoreError};

/// The prototype's configured upload limit; a resource limit, not a supported-file claim.
pub const MAX_UPLOAD_BYTES: usize = 64 * 1024 * 1024;

/// The default bound on one protected request. It stays below the desktop client's 30-second
/// transport timeout, so a slow answer is an explicit retryable failure rather than a lost request.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(25);

/// What the API needs besides the store.
#[derive(Clone)]
pub struct ApiState {
    store: Store,
    token: Arc<str>,
    max_upload: usize,
    request_timeout: Duration,
    delay: Duration,
}

impl ApiState {
    pub fn new(store: Store, token: impl Into<Arc<str>>) -> Self {
        Self { store, token: token.into(), max_upload: MAX_UPLOAD_BYTES, request_timeout: DEFAULT_REQUEST_TIMEOUT, delay: Duration::ZERO }
    }

    /// Use a smaller limit in tests.
    pub fn with_max_upload(mut self, bytes: usize) -> Self {
        self.max_upload = bytes;
        self
    }

    /// Bound how long one protected request may take. The timeout is a retryable `internal`.
    pub fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    /// Delay every protected request. It exists only for controlled responsiveness measurements.
    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

/// Build the API. `/health` needs no token; everything under `/v1` does.
pub fn router(state: ApiState) -> Router {
    let max_upload = state.max_upload;
    let token = state.token.clone();
    let timeout = state.request_timeout;
    let delay = state.delay;
    let protected = Router::new()
        .route("/sources/{source}", put(upload_source))
        .route("/sources/{source}", get(get_source))
        .route("/reference-data", get(reference_data))
        .route("/projects", post(create_project))
        .route("/projects/{project_id}", get(get_project))
        .route("/projects/{project_id}/draft", put(save_draft))
        .route("/projects/{project_id}/calculations", post(calculate))
        .route("/calculations/{calculation_id}", get(get_calculation))
        .layer(middleware::from_fn_with_state(delay, wait))
        .layer(middleware::from_fn_with_state(token.clone(), require_token))
        .layer(middleware::from_fn_with_state(timeout, bound_request));
    let project_records = Router::new()
        .route("/projects", post(create_project_record))
        .route("/projects/{project_id}", get(get_project_record))
        .route("/projects/{project_id}", put(update_project_record))
        .route("/projects/{project_id}/products/{product_id}", put(update_product_record))
        .route("/projects/{project_id}/deliveries", post(record_delivery))
        .route("/projects/{project_id}/part-matches", post(confirm_part_match))
        .route("/projects/{project_id}/exclusions", post(exclude_products))
        .route("/projects/{project_id}/reinstatements", post(reinstate_products))
        .route("/projects/{project_id}/versions/{version}", get(get_project_record_version))
        .layer(middleware::from_fn_with_state(delay, wait))
        .layer(middleware::from_fn_with_state(token, require_token))
        .layer(middleware::from_fn_with_state(timeout, bound_request));
    Router::new()
        .route("/health", get(health))
        .nest("/v1", protected)
        .nest("/v2", project_records)
        .layer(DefaultBodyLimit::max(max_upload))
        .with_state(state)
}

async fn wait(State(delay): State<Duration>, request: Request, next: Next) -> Response {
    if !delay.is_zero() {
        tokio::time::sleep(delay).await;
    }
    next.run(request).await
}

async fn bound_request(State(timeout): State<Duration>, request: Request, next: Next) -> Response {
    match tokio::time::timeout(timeout, next.run(request)).await {
        Ok(response) => response,
        // The client sees the retryable internal class: the write may have committed, so the same
        // request id and payload are the safe retry. The transaction is dropped with the handler.
        Err(_) => ApiError::Timeout.into_response(),
    }
}

async fn require_token(State(token): State<Arc<str>>, request: Request, next: Next) -> Response {
    let given = request.headers().get(header::AUTHORIZATION).and_then(|value| value.to_str().ok()).and_then(|value| value.strip_prefix("Bearer "));
    match given {
        Some(given) if token_matches(given, token.as_ref()) => next.run(request).await,
        _ => ApiError::Unauthorized.into_response(),
    }
}

/// Compare the bearer token without an early exit, so a wrong prefix is not measurably faster.
fn token_matches(given: &str, expected: &str) -> bool {
    let given = given.as_bytes();
    let expected = expected.as_bytes();
    if given.len() != expected.len() {
        return false;
    }
    let mut difference = 0u8;
    for (left, right) in given.iter().zip(expected) {
        difference |= left ^ right;
    }
    difference == 0
}

async fn health() -> impl IntoResponse {
    Json(json!({"ok": true}))
}

#[derive(Deserialize)]
struct SourceName {
    name: String,
}

async fn upload_source(
    State(state): State<ApiState>,
    Path(sha256): Path<String>,
    Query(name): Query<SourceName>,
    body: Bytes,
) -> Result<Json<SourceRef>, ApiError> {
    let source = state.store.put_source(&sha256, &name.name, &body).await?;
    Ok(Json(source))
}

async fn get_source(State(state): State<ApiState>, Path(source_id): Path<String>) -> Result<Response, ApiError> {
    let source_id = parse_id(&source_id, "source")?;
    let (source, bytes) = state.store.get_source(source_id).await?;
    let mut response = Response::new(axum::body::Body::from(bytes));
    response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/step"));
    response.headers_mut().insert("x-solidbom-sha256", HeaderValue::from_str(&source.sha256).map_err(|_| ApiError::Internal)?);
    Ok(response)
}

async fn reference_data(State(state): State<ApiState>) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(state.store.reference_data().await?))
}

async fn create_project(State(state): State<ApiState>, Json(save): Json<SaveDraft>) -> Result<impl IntoResponse, ApiError> {
    let receipt = state.store.create_draft(&save).await?;
    Ok((StatusCode::CREATED, Json(receipt)))
}

async fn get_project(State(state): State<ApiState>, Path(project_id): Path<String>) -> Result<impl IntoResponse, ApiError> {
    let project_id = parse_id(&project_id, "project")?;
    Ok(Json(state.store.project_view(project_id).await?))
}

async fn save_draft(
    State(state): State<ApiState>,
    Path(project_id): Path<String>,
    Json(save): Json<SaveDraft>,
) -> Result<impl IntoResponse, ApiError> {
    let project_id = parse_id(&project_id, "project")?;
    if save.draft.project_id != project_id {
        return Err(ApiError::Invalid("the draft belongs to a different project".into()));
    }
    Ok(Json(state.store.save_draft(&save).await?))
}

async fn calculate(
    State(state): State<ApiState>,
    Path(project_id): Path<String>,
    Json(request): Json<CalculateRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let project_id = parse_id(&project_id, "project")?;
    if request.project_id != project_id {
        return Err(ApiError::Invalid("the calculation belongs to a different project".into()));
    }
    let snapshot = state.store.record_calculation(&request).await?;
    Ok((StatusCode::CREATED, Json(snapshot)))
}

async fn get_calculation(State(state): State<ApiState>, Path(calculation_id): Path<String>) -> Result<impl IntoResponse, ApiError> {
    let calculation_id = parse_id(&calculation_id, "calculation")?;
    Ok(Json(state.store.load_calculation(calculation_id).await?))
}

async fn create_project_record(State(state): State<ApiState>, Json(request): Json<CreateProject>) -> Result<impl IntoResponse, ApiError> {
    let outcome = state.store.create_project(&request).await?;
    Ok((StatusCode::CREATED, Json(outcome)))
}

async fn get_project_record(State(state): State<ApiState>, Path(project_id): Path<String>) -> Result<impl IntoResponse, ApiError> {
    let project_id = parse_id(&project_id, "project")?;
    Ok(Json(state.store.project_record_view(project_id).await?))
}

async fn get_project_record_version(
    State(state): State<ApiState>,
    Path((project_id, version)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let project_id = parse_id(&project_id, "project")?;
    let version = version.parse::<u64>().map_err(|_| ApiError::Invalid(format!("`{version}` is not a project record version")))?;
    Ok(Json(state.store.project_record_version(project_id, version).await?))
}

async fn update_project_record(
    State(state): State<ApiState>,
    Path(project_id): Path<String>,
    Json(request): Json<UpdateProject>,
) -> Result<impl IntoResponse, ApiError> {
    let project_id = parse_id(&project_id, "project")?;
    Ok(Json(state.store.update_project(project_id, &request).await?))
}

async fn update_product_record(
    State(state): State<ApiState>,
    Path((project_id, product_id)): Path<(String, String)>,
    Json(request): Json<UpdateProduct>,
) -> Result<impl IntoResponse, ApiError> {
    let project_id = parse_id(&project_id, "project")?;
    let product_id = parse_id(&product_id, "product")?;
    if request.product_id != product_id {
        return Err(ApiError::Invalid("the product update belongs to a different product".into()));
    }
    Ok(Json(state.store.update_product(project_id, &request).await?))
}

async fn record_delivery(
    State(state): State<ApiState>,
    Path(project_id): Path<String>,
    Json(request): Json<RecordDelivery>,
) -> Result<impl IntoResponse, ApiError> {
    let project_id = parse_id(&project_id, "project")?;
    let outcome = state.store.record_delivery(project_id, &request).await?;
    Ok((StatusCode::CREATED, Json(outcome)))
}

async fn confirm_part_match(
    State(state): State<ApiState>,
    Path(project_id): Path<String>,
    Json(request): Json<ConfirmPartMatch>,
) -> Result<impl IntoResponse, ApiError> {
    let project_id = parse_id(&project_id, "project")?;
    Ok(Json(state.store.confirm_part_match(project_id, &request).await?))
}

async fn exclude_products(
    State(state): State<ApiState>,
    Path(project_id): Path<String>,
    Json(request): Json<ExcludeProducts>,
) -> Result<impl IntoResponse, ApiError> {
    let project_id = parse_id(&project_id, "project")?;
    Ok(Json(state.store.exclude_products(project_id, &request).await?))
}

async fn reinstate_products(
    State(state): State<ApiState>,
    Path(project_id): Path<String>,
    Json(request): Json<ReinstateProducts>,
) -> Result<impl IntoResponse, ApiError> {
    let project_id = parse_id(&project_id, "project")?;
    Ok(Json(state.store.reinstate_products(project_id, &request).await?))
}

fn parse_id(text: &str, what: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(text).map_err(|_| ApiError::Invalid(format!("`{text}` is not a {what} id")))
}

/// An API failure with a stable status and a client-safe message.
enum ApiError {
    Unauthorized,
    Invalid(String),
    NotFound,
    Conflict(String),
    Internal,
    Timeout,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, ApiErrorCode::Unauthorized, "a bearer token is required".to_string()),
            Self::Invalid(message) => (StatusCode::UNPROCESSABLE_ENTITY, ApiErrorCode::Invalid, message),
            Self::NotFound => (StatusCode::NOT_FOUND, ApiErrorCode::NotFound, "not found".to_string()),
            Self::Conflict(message) => (StatusCode::CONFLICT, ApiErrorCode::Conflict, message),
            Self::Internal => (StatusCode::INTERNAL_SERVER_ERROR, ApiErrorCode::Internal, "the service could not complete the request".to_string()),
            // A lost write is answered from the recorded request on an identical retry.
            Self::Timeout => {
                (StatusCode::INTERNAL_SERVER_ERROR, ApiErrorCode::Internal, "the service did not answer in time; retry the same request".to_string())
            }
        };
        (status, Json(ApiErrorBody::new(code, message))).into_response()
    }
}

impl From<StoreError> for ApiError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::NotFound => Self::NotFound,
            StoreError::Conflict(message) => Self::Conflict(message),
            StoreError::Invalid(message) => Self::Invalid(message),
            other @ (StoreError::Codec(_) | StoreError::Database(_) | StoreError::Migration(_)) => {
                // The log keeps the detail for an operator; the client gets a generic message that
                // never contains the database error, a path or the token.
                tracing::error!(error = %other, "store failure");
                Self::Internal
            }
        }
    }
}
