//! The version 2 HTTP API over the SolidBOM store.
//!
//! Two development identity sets protect the routes: Engineer for project work and Administrator for
//! shared company records. One identity can hold both; `SOLIDBOM_TOKEN` configures that combined
//! legacy identity. Every route except `/health` needs a bearer token. A known identity without the
//! route's permission set is `403 forbidden`; a missing or unknown token is `401 unauthorized`.
//!
//! Errors carry a stable code and a client-safe message, never a database error, path or token.
//! Uploads are bounded and hashed; every protected request has a bounded response time; a
//! calculation is only for a persisted version and resolves its reference values on this side.
//!
//! Routes, statuses, error codes and idempotency rules are the published version 2 contract in
//! `docs/solidbom/contracts/v2/shared-data-and-permissions.md`; this module only implements it.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, FromRequestParts, Path, Query, Request, State};
use axum::http::header;
use axum::http::request::Parts;
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use solidbom_contract::{
    ApiErrorBody, ApiErrorCode, CalculateRequest, CreateAppConfigVersion, CreateComponentVersion, CreateMaterialVersion, CreateRateVersion, Identity,
    Permission, SaveDraft, SharedDataView, SourceRef,
};
use uuid::Uuid;

use crate::auth::IdentityTable;
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
    identities: Arc<IdentityTable>,
    max_upload: usize,
    request_timeout: Duration,
    delay: Duration,
}

impl ApiState {
    /// The legacy single-token configuration: one combined identity, as before version 2.
    pub fn new(store: Store, token: impl Into<Arc<str>>) -> Self {
        let token: Arc<str> = token.into();
        Self::with_identities(store, IdentityTable::development(token.as_ref()))
    }

    /// A state with an explicit identity table (tests and the configured service use this).
    pub fn with_identities(store: Store, identities: IdentityTable) -> Self {
        Self {
            store,
            identities: Arc::new(identities),
            max_upload: MAX_UPLOAD_BYTES,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            delay: Duration::ZERO,
        }
    }

    /// Add the engineer-only development identity.
    pub fn with_engineer_token(mut self, token: impl Into<String>) -> Self {
        let token: String = token.into();
        self.identities = Arc::new(self.identities.as_ref().clone().with_engineer_token(&token));
        self
    }

    /// Add the administrator-only development identity.
    pub fn with_administrator_token(mut self, token: impl Into<String>) -> Self {
        let token: String = token.into();
        self.identities = Arc::new(self.identities.as_ref().clone().with_administrator_token(&token));
        self
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

/// Build the API. `/health` needs no token; every route under `/v1` is authenticated, and the
/// engineer and administrator route groups then check their permission set.
pub fn router(state: ApiState) -> Router {
    let max_upload = state.max_upload;
    let identities = state.identities.clone();
    let timeout = state.request_timeout;
    let delay = state.delay;
    let engineer_routes = Router::new()
        .route("/sources/{source}", put(upload_source))
        .route("/sources/{source}", get(get_source))
        .route("/reference-data", get(reference_data))
        .route("/projects", post(create_project))
        .route("/projects/{project_id}", get(get_project))
        .route("/projects/{project_id}/draft", put(save_draft))
        .route("/projects/{project_id}/calculations", post(calculate))
        .route("/calculations/{calculation_id}", get(get_calculation))
        .route_layer(middleware::from_fn_with_state(Permission::Engineer, require_permission));
    let administrator_routes = Router::new()
        .route("/shared", get(shared_data))
        .route("/shared/materials/{material_id}/versions", post(create_material_version))
        .route("/shared/cost-centers/{cost_center_id}/versions", post(create_rate_version))
        .route("/shared/components/{component_id}/versions", post(create_component_version))
        .route("/shared/app-config/{config_id}/versions", post(create_app_config_version))
        .route_layer(middleware::from_fn_with_state(Permission::Administrator, require_permission));
    let protected = Router::new()
        .route("/me", get(me))
        .merge(engineer_routes)
        .merge(administrator_routes)
        .layer(middleware::from_fn_with_state(delay, wait))
        .layer(middleware::from_fn_with_state(identities, authenticate))
        .layer(middleware::from_fn_with_state(timeout, bound_request));
    Router::new().route("/health", get(health)).nest("/v1", protected).layer(DefaultBodyLimit::max(max_upload)).with_state(state)
}

/// The caller of one authenticated request. The authentication layer inserts the identity.
struct Caller(Identity);

impl<S> FromRequestParts<S> for Caller
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts.extensions.get::<Identity>().cloned().map(Caller).ok_or(ApiError::Unauthorized)
    }
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

/// Resolve the bearer token to one identity and attach it for the route groups to check.
async fn authenticate(State(identities): State<Arc<IdentityTable>>, mut request: Request, next: Next) -> Response {
    let token = request.headers().get(header::AUTHORIZATION).and_then(|value| value.to_str().ok()).and_then(|value| value.strip_prefix("Bearer "));
    match token.and_then(|token| identities.authenticate(token)) {
        Some(identity) => {
            request.extensions_mut().insert(identity);
            next.run(request).await
        }
        None => ApiError::Unauthorized.into_response(),
    }
}

/// Check one permission set for a whole route group. A known identity without it is `403`.
async fn require_permission(State(permission): State<Permission>, request: Request, next: Next) -> Response {
    match request.extensions().get::<Identity>() {
        Some(identity) if identity.has(permission) => next.run(request).await,
        Some(_) => ApiError::Forbidden.into_response(),
        // The authentication layer always inserts an identity; this is only a safe fallback.
        None => ApiError::Unauthorized.into_response(),
    }
}

async fn health() -> impl IntoResponse {
    Json(json!({"ok": true}))
}

/// The caller's identity and permission sets, for any authenticated caller.
async fn me(Caller(identity): Caller) -> Json<Identity> {
    Json(identity)
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

async fn shared_data(State(state): State<ApiState>, Caller(identity): Caller) -> Result<Json<SharedDataView>, ApiError> {
    Ok(Json(state.store.shared_data(&identity).await?))
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

async fn create_material_version(
    State(state): State<ApiState>,
    Path(material_id): Path<String>,
    Caller(identity): Caller,
    Json(request): Json<CreateMaterialVersion>,
) -> Result<impl IntoResponse, ApiError> {
    let material_id = parse_id(&material_id, "material")?;
    if request.material_id != material_id {
        return Err(ApiError::Invalid("the version belongs to a different material".into()));
    }
    let published = state.store.create_material_version(&request, &identity.name).await?;
    Ok((StatusCode::CREATED, Json(published)))
}

async fn create_rate_version(
    State(state): State<ApiState>,
    Path(cost_center_id): Path<String>,
    Caller(identity): Caller,
    Json(request): Json<CreateRateVersion>,
) -> Result<impl IntoResponse, ApiError> {
    let cost_center_id = parse_id(&cost_center_id, "cost center")?;
    if request.cost_center_id != cost_center_id {
        return Err(ApiError::Invalid("the version belongs to a different cost center".into()));
    }
    let published = state.store.create_rate_version(&request, &identity.name).await?;
    Ok((StatusCode::CREATED, Json(published)))
}

async fn create_component_version(
    State(state): State<ApiState>,
    Path(component_id): Path<String>,
    Caller(identity): Caller,
    Json(request): Json<CreateComponentVersion>,
) -> Result<impl IntoResponse, ApiError> {
    let component_id = parse_id(&component_id, "component")?;
    if request.component_id != component_id {
        return Err(ApiError::Invalid("the version belongs to a different component".into()));
    }
    let published = state.store.create_component_version(&request, &identity.name).await?;
    Ok((StatusCode::CREATED, Json(published)))
}

async fn create_app_config_version(
    State(state): State<ApiState>,
    Path(config_id): Path<String>,
    Caller(identity): Caller,
    Json(request): Json<CreateAppConfigVersion>,
) -> Result<impl IntoResponse, ApiError> {
    let config_id = parse_id(&config_id, "app configuration")?;
    if request.config_id != config_id {
        return Err(ApiError::Invalid("the version belongs to a different app configuration".into()));
    }
    let published = state.store.create_app_config_version(&request, &identity.name).await?;
    Ok((StatusCode::CREATED, Json(published)))
}

fn parse_id(text: &str, what: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(text).map_err(|_| ApiError::Invalid(format!("`{text}` is not a {what} id")))
}

/// An API failure with a stable status and a client-safe message.
enum ApiError {
    Unauthorized,
    Forbidden,
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
            // The identity is known, so the token is valid; the route needs a permission set it
            // does not hold. This is not a retry and not a login problem.
            Self::Forbidden => {
                (StatusCode::FORBIDDEN, ApiErrorCode::Forbidden, "this identity does not hold the permission this route needs".to_string())
            }
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
