//! The v1 HTTP API over the SolidBOM store.
//!
//! One explicit development bearer identity protects every route except `/health`. Errors carry a
//! code and a message, never a database error, path or token. Uploads are bounded and hashed; a
//! calculation is only for a persisted version and resolves its reference values on this side.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use solidbom_contract::{CalculateRequest, SaveDraft, SourceRef};
use uuid::Uuid;

use crate::store::{Store, StoreError};

/// The prototype's configured upload limit; a resource limit, not a supported-file claim.
pub const MAX_UPLOAD_BYTES: usize = 64 * 1024 * 1024;

/// What the API needs besides the store.
#[derive(Clone)]
pub struct ApiState {
    store: Store,
    token: Arc<str>,
    max_upload: usize,
}

impl ApiState {
    pub fn new(store: Store, token: impl Into<Arc<str>>) -> Self {
        Self { store, token: token.into(), max_upload: MAX_UPLOAD_BYTES }
    }

    /// Use a smaller limit in tests.
    pub fn with_max_upload(mut self, bytes: usize) -> Self {
        self.max_upload = bytes;
        self
    }
}

/// Build the API. `/health` needs no token; everything under `/v1` does.
pub fn router(state: ApiState) -> Router {
    let max_upload = state.max_upload;
    let token = state.token.clone();
    let protected = Router::new()
        .route("/sources/{source}", put(upload_source))
        .route("/sources/{source}", get(get_source))
        .route("/reference-data", get(reference_data))
        .route("/projects", post(create_project))
        .route("/projects/{project_id}", get(get_project))
        .route("/projects/{project_id}/draft", put(save_draft))
        .route("/projects/{project_id}/calculations", post(calculate))
        .route("/calculations/{calculation_id}", get(get_calculation))
        .layer(middleware::from_fn_with_state(token, require_token));
    Router::new().route("/health", get(health)).nest("/v1", protected).layer(DefaultBodyLimit::max(max_upload)).with_state(state)
}

async fn require_token(State(token): State<Arc<str>>, request: Request, next: Next) -> Response {
    let given = request.headers().get(header::AUTHORIZATION).and_then(|value| value.to_str().ok()).and_then(|value| value.strip_prefix("Bearer "));
    match given {
        Some(given) if given == token.as_ref() => next.run(request).await,
        _ => ApiError::Unauthorized.into_response(),
    }
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
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized", "a bearer token is required".to_string()),
            Self::Invalid(message) => (StatusCode::UNPROCESSABLE_ENTITY, "invalid", message),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found", "not found".to_string()),
            Self::Conflict(message) => (StatusCode::CONFLICT, "conflict", message),
            Self::Internal => (StatusCode::INTERNAL_SERVER_ERROR, "internal", "the service could not complete the request".to_string()),
        };
        (status, Json(json!({"error": {"code": code, "message": message}}))).into_response()
    }
}

impl From<StoreError> for ApiError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::NotFound => Self::NotFound,
            StoreError::Conflict(message) => Self::Conflict(message),
            StoreError::Invalid(message) => Self::Invalid(message),
            other @ (StoreError::Codec(_) | StoreError::Database(_) | StoreError::Migration(_)) => {
                // The log keeps the detail; the client gets a generic message.
                tracing::error!(error = %other, "store failure");
                Self::Internal
            }
        }
    }
}
