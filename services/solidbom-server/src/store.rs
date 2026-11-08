//! PostgreSQL storage for the SolidBOM prototype.
//!
//! Sources are stored once by SHA-256. Project drafts are append-only versions: a save locks the
//! project row, checks the request ID before the expected version, validates the source, inserts
//! the new immutable version and advances the current version in one transaction. A lost reply
//! answered by an explicit retry returns its recorded receipt; a reused request ID with a
//! different payload is a conflict.

use std::collections::HashMap;
use std::str::FromStr;

use rust_decimal::Decimal;
use serde::Serialize;
use sha2::{Digest, Sha256};
use solidbom_contract::{
    CalculateRequest, CalculationInputs, CalculationOutcome, CalculationSnapshot, CalculationSummary, DraftProject, Issue, MaterialVersion,
    ProjectView, RateVersion, ReferenceData, ResolvedPartInput, ResultState, ReviewState, SaveDraft, SaveReceipt, SourceRef, Sourcing,
};
use sqlx::Row;
use sqlx::postgres::{PgConnectOptions, PgConnection, PgPool, PgPoolOptions, PgRow};
use uuid::Uuid;

/// The longest request id the service accepts. It is a client-supplied primary key, so it is
/// bounded instead of trusted.
pub const MAX_REQUEST_ID_CHARS: usize = 128;

/// The longest source name the service accepts.
pub const MAX_SOURCE_NAME_CHARS: usize = 255;

/// What can go wrong in the store. The API maps these; database detail never reaches a client.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Invalid(String),
    #[error("a stored value could not be read: {0}")]
    Codec(String),
    #[error("database error")]
    Database(#[from] sqlx::Error),
    #[error("migrations failed: {0}")]
    Migration(String),
}

impl StoreError {
    fn codec(error: impl std::fmt::Display) -> Self {
        Self::Codec(error.to_string())
    }
}

type Result<T> = std::result::Result<T, StoreError>;

/// A handle on the development database.
#[derive(Clone)]
pub struct Store {
    pool: PgPool,
}

impl Store {
    /// Connect to a database and apply the migrations.
    pub async fn connect(url: &str) -> Result<Self> {
        let options = PgConnectOptions::from_str(url).map_err(|e| StoreError::Invalid(format!("database URL: {e}")))?;
        Self::connect_with_options(options).await
    }

    /// Connect with explicit options (tests use a private schema in the search path).
    pub async fn connect_with_options(options: PgConnectOptions) -> Result<Self> {
        let pool = PgPoolOptions::new().max_connections(8).connect_with(options).await?;
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    /// Apply the embedded migrations. Idempotent: SQLx records them.
    pub async fn migrate(&self) -> Result<()> {
        sqlx::migrate!("./migrations").run(&self.pool).await.map_err(|e| StoreError::Migration(e.to_string()))
    }

    /// The connection pool, for the API layer and integration tests.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Store the original bytes once. The digest must match the bytes; a repeat is idempotent.
    pub async fn put_source(&self, expected_sha256: &str, original_name: &str, bytes: &[u8]) -> Result<SourceRef> {
        let sha256 = hex::encode(Sha256::digest(bytes));
        if sha256 != expected_sha256 {
            return Err(StoreError::Invalid("the uploaded bytes do not match their SHA-256 digest".into()));
        }
        if bytes.is_empty() {
            return Err(StoreError::Invalid("an empty file is not a source".into()));
        }
        if original_name.trim().is_empty() {
            return Err(StoreError::Invalid("a source needs the name it was imported as".into()));
        }
        if original_name.chars().count() > MAX_SOURCE_NAME_CHARS {
            return Err(StoreError::Invalid("the source name is too long".into()));
        }
        if let Some(existing) = self.source_by_sha256(&sha256).await? {
            return Ok(existing);
        }
        let source = SourceRef {
            source_id: Uuid::new_v4(),
            sha256,
            byte_len: u64::try_from(bytes.len()).map_err(|_| StoreError::Invalid("the source is too large".into()))?,
            original_name: original_name.to_string(),
        };
        let inserted = sqlx::query("INSERT INTO sources (source_id, sha256, byte_len, original_name, bytes) VALUES ($1, $2, $3, $4, $5)")
            .bind(source.source_id)
            .bind(&source.sha256)
            .bind(i64::try_from(source.byte_len).map_err(|_| StoreError::Invalid("the source is too large".into()))?)
            .bind(&source.original_name)
            .bind(bytes)
            .execute(&self.pool)
            .await;
        match inserted {
            Ok(_) => Ok(source),
            // A concurrent upload of the same bytes won: return the stored row.
            Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
                self.source_by_sha256(&source.sha256).await?.ok_or(StoreError::NotFound)
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Read the original bytes and verify they still match the stored digest.
    pub async fn get_source(&self, source_id: Uuid) -> Result<(SourceRef, Vec<u8>)> {
        let row = sqlx::query("SELECT source_id, sha256, byte_len, original_name, bytes FROM sources WHERE source_id = $1")
            .bind(source_id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(StoreError::NotFound)?;
        let source = row_to_source(&row)?;
        let bytes: Vec<u8> = row.try_get("bytes")?;
        if hex::encode(Sha256::digest(&bytes)) != source.sha256 {
            return Err(StoreError::Codec("the stored source no longer matches its digest".into()));
        }
        Ok((source, bytes))
    }

    /// The seeded material and rate versions.
    pub async fn reference_data(&self) -> Result<ReferenceData> {
        let mut connection = self.pool.acquire().await?;
        reference_data_in(&mut connection).await
    }

    /// Create a project and its first immutable version.
    pub async fn create_draft(&self, save: &SaveDraft) -> Result<SaveReceipt> {
        save.validate().map_err(|e| StoreError::Invalid(e.to_string()))?;
        check_request_id(&save.request_id)?;
        if save.expected_version != 0 {
            return Err(StoreError::Invalid("a new project starts at version 0".into()));
        }
        let payload = payload_hash("create_project", &save.draft)?;
        let mut tx = self.pool.begin().await?;
        // Serialise retries that share one request id, so two concurrent copies cannot both write.
        lock_request(&mut tx, &save.request_id).await?;
        if let Some((stored, response)) = lookup_request(&mut tx, &save.request_id).await? {
            tx.commit().await?;
            if stored != payload {
                return Err(StoreError::Conflict("the request ID was reused with a different payload".into()));
            }
            return serde_json::from_value(response).map_err(StoreError::codec);
        }
        ensure_source(&mut tx, &save.draft.source).await?;
        validate_references(&mut tx, &save.draft).await?;

        let mut stored_draft = save.draft.clone();
        stored_draft.version = 1;
        let draft_json = serde_json::to_string(&stored_draft).map_err(StoreError::codec)?;
        let inserted =
            sqlx::query("INSERT INTO projects (project_id, product_revision_id, current_version, source_id, currency) VALUES ($1, $2, 1, $3, $4)")
                .bind(stored_draft.project_id)
                .bind(stored_draft.product_revision_id)
                .bind(stored_draft.source.source_id)
                .bind(&stored_draft.currency)
                .execute(&mut *tx)
                .await;
        match inserted {
            Ok(_) => {}
            // A new request id cannot take over a project id that already exists.
            Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
                return Err(StoreError::Conflict("the project id already exists".into()));
            }
            Err(error) => return Err(error.into()),
        }
        insert_version(&mut tx, &stored_draft, &draft_json).await?;

        let receipt = SaveReceipt { request_id: save.request_id.clone(), project_id: stored_draft.project_id, version: 1 };
        insert_request(&mut tx, &save.request_id, "create_project", &payload, &receipt).await?;
        tx.commit().await?;
        Ok(receipt)
    }

    /// Save a new immutable version, if the client's expected version is still current.
    pub async fn save_draft(&self, save: &SaveDraft) -> Result<SaveReceipt> {
        save.validate().map_err(|e| StoreError::Invalid(e.to_string()))?;
        check_request_id(&save.request_id)?;
        if save.expected_version == 0 {
            return Err(StoreError::Invalid("an existing project saves from version 1".into()));
        }
        let payload = payload_hash("save_draft", &save.draft)?;
        let mut tx = self.pool.begin().await?;
        lock_request(&mut tx, &save.request_id).await?;
        if let Some((stored, response)) = lookup_request(&mut tx, &save.request_id).await? {
            tx.commit().await?;
            if stored != payload {
                return Err(StoreError::Conflict("the request ID was reused with a different payload".into()));
            }
            return serde_json::from_value(response).map_err(StoreError::codec);
        }

        let project = sqlx::query("SELECT current_version, source_id, product_revision_id FROM projects WHERE project_id = $1 FOR UPDATE")
            .bind(save.draft.project_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(StoreError::NotFound)?;
        let current: i64 = project.try_get("current_version")?;
        let current = u64::try_from(current).map_err(StoreError::codec)?;
        let project_source: Uuid = project.try_get("source_id")?;
        let product_revision: Uuid = project.try_get("product_revision_id")?;
        if save.draft.source.source_id != project_source {
            return Err(StoreError::Conflict("the draft belongs to a different source revision".into()));
        }
        if save.draft.product_revision_id != product_revision {
            return Err(StoreError::Conflict("the draft belongs to a different product revision".into()));
        }
        if current != save.expected_version {
            return Err(StoreError::Conflict(format!("the project is at version {current}, not {}", save.expected_version)));
        }
        validate_references(&mut tx, &save.draft).await?;
        ensure_source(&mut tx, &save.draft.source).await?;

        let version = current + 1;
        let mut stored_draft = save.draft.clone();
        stored_draft.version = version;
        let draft_json = serde_json::to_string(&stored_draft).map_err(StoreError::codec)?;
        insert_version(&mut tx, &stored_draft, &draft_json).await?;
        sqlx::query("UPDATE projects SET current_version = $2 WHERE project_id = $1")
            .bind(stored_draft.project_id)
            .bind(i64::try_from(version).map_err(StoreError::codec)?)
            .execute(&mut *tx)
            .await?;

        let receipt = SaveReceipt { request_id: save.request_id.clone(), project_id: stored_draft.project_id, version };
        insert_request(&mut tx, &save.request_id, "save_draft", &payload, &receipt).await?;
        tx.commit().await?;
        Ok(receipt)
    }

    /// The current draft.
    pub async fn load_project(&self, project_id: Uuid) -> Result<DraftProject> {
        let row = sqlx::query(
            "SELECT pv.draft::text AS draft FROM projects p \
             JOIN project_versions pv ON pv.project_id = p.project_id AND pv.version = p.current_version \
             WHERE p.project_id = $1",
        )
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StoreError::NotFound)?;
        let text: String = row.try_get("draft")?;
        serde_json::from_str(&text).map_err(StoreError::codec)
    }

    /// The current draft and the immutable calculations saved for the project.
    pub async fn project_view(&self, project_id: Uuid) -> Result<ProjectView> {
        let draft = self.load_project(project_id).await?;
        let rows = sqlx::query(
            "SELECT calculation_id, saved_version, calculator_version FROM calculations \
             WHERE project_id = $1 ORDER BY created_at, calculation_id",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        let mut calculations = Vec::with_capacity(rows.len());
        for row in &rows {
            calculations.push(CalculationSummary {
                calculation_id: row.try_get("calculation_id")?,
                saved_version: u64::try_from(row.try_get::<i64, _>("saved_version")?).map_err(StoreError::codec)?,
                calculator_version: row.try_get("calculator_version")?,
            });
        }
        Ok(ProjectView { draft, calculations })
    }

    /// How many immutable versions a project holds (diagnostics and tests).
    pub async fn version_count(&self, project_id: Uuid) -> Result<u64> {
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM project_versions WHERE project_id = $1").bind(project_id).fetch_one(&self.pool).await?;
        u64::try_from(count).map_err(StoreError::codec)
    }

    /// One persisted version, which may be historical.
    pub async fn load_version(&self, project_id: Uuid, version: u64) -> Result<DraftProject> {
        let row = sqlx::query("SELECT draft::text AS draft FROM project_versions WHERE project_id = $1 AND version = $2")
            .bind(project_id)
            .bind(i64::try_from(version).map_err(StoreError::codec)?)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(StoreError::NotFound)?;
        let text: String = row.try_get("draft")?;
        serde_json::from_str(&text).map_err(StoreError::codec)
    }

    /// Calculate one persisted version, resolve reference values server-side, and keep the result
    /// immutable. A repeated request ID returns its recorded snapshot.
    pub async fn record_calculation(&self, request: &CalculateRequest) -> Result<CalculationSnapshot> {
        request.validate().map_err(|e| StoreError::Invalid(e.to_string()))?;
        check_request_id(&request.request_id)?;
        let payload = payload_hash("calculate", request)?;
        let mut tx = self.pool.begin().await?;
        lock_request(&mut tx, &request.request_id).await?;
        if let Some((stored, response)) = lookup_request(&mut tx, &request.request_id).await? {
            tx.commit().await?;
            if stored != payload {
                return Err(StoreError::Conflict("the request ID was reused with a different payload".into()));
            }
            return serde_json::from_value(response).map_err(StoreError::codec);
        }
        let row = sqlx::query("SELECT draft::text AS draft FROM project_versions WHERE project_id = $1 AND version = $2")
            .bind(request.project_id)
            .bind(i64::try_from(request.saved_version).map_err(StoreError::codec)?)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(StoreError::NotFound)?;
        let text: String = row.try_get("draft")?;
        let draft: DraftProject = serde_json::from_str(&text).map_err(StoreError::codec)?;
        let reference = reference_data_in(&mut tx).await?;
        let (inputs, mut resolution_issues) = resolve_inputs(&draft, &reference);
        let mut outcome = solidbom_costing::calculate(&inputs);
        // The calculator names a part by its resolved part version; a client names the reviewed
        // definition. Map the calculator's issues back before the server-only resolution issues
        // (which already name the definition) are appended.
        map_issue_definitions(&mut outcome, &draft);
        if !resolution_issues.is_empty() {
            outcome.state = ResultState::Incomplete;
        }
        resolution_issues.append(&mut outcome.issues);
        outcome.issues = resolution_issues;
        let issues = outcome.issues.clone();
        let input_hash = payload_hash("inputs", &inputs)?;
        let snapshot = CalculationSnapshot {
            calculation_id: Uuid::new_v4(),
            project_id: request.project_id,
            saved_version: request.saved_version,
            input_hash,
            calculator_version: solidbom_costing::calculator_version().to_string(),
            inputs,
            outputs: outcome,
            issues,
        };
        sqlx::query(
            "INSERT INTO calculations (calculation_id, project_id, saved_version, input_hash, calculator_version, inputs, outputs, issues) \
             VALUES ($1, $2, $3, $4, $5, $6::jsonb, $7::jsonb, $8::jsonb)",
        )
        .bind(snapshot.calculation_id)
        .bind(snapshot.project_id)
        .bind(i64::try_from(snapshot.saved_version).map_err(StoreError::codec)?)
        .bind(&snapshot.input_hash)
        .bind(&snapshot.calculator_version)
        .bind(serde_json::to_string(&snapshot.inputs).map_err(StoreError::codec)?)
        .bind(serde_json::to_string(&snapshot.outputs).map_err(StoreError::codec)?)
        .bind(serde_json::to_string(&snapshot.issues).map_err(StoreError::codec)?)
        .execute(&mut *tx)
        .await?;
        insert_request(&mut tx, &request.request_id, "calculate", &payload, &snapshot).await?;
        tx.commit().await?;
        Ok(snapshot)
    }

    /// One saved calculation snapshot, unchanged by later edits.
    pub async fn load_calculation(&self, calculation_id: Uuid) -> Result<CalculationSnapshot> {
        let row = sqlx::query(
            "SELECT calculation_id, project_id, saved_version, input_hash, calculator_version, \
             inputs::text AS inputs, outputs::text AS outputs, issues::text AS issues \
             FROM calculations WHERE calculation_id = $1",
        )
        .bind(calculation_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StoreError::NotFound)?;
        let inputs = serde_json::from_str(&row.try_get::<String, _>("inputs")?).map_err(StoreError::codec)?;
        let outputs = serde_json::from_str(&row.try_get::<String, _>("outputs")?).map_err(StoreError::codec)?;
        let issues = serde_json::from_str(&row.try_get::<String, _>("issues")?).map_err(StoreError::codec)?;
        Ok(CalculationSnapshot {
            calculation_id: row.try_get("calculation_id")?,
            project_id: row.try_get("project_id")?,
            saved_version: u64::try_from(row.try_get::<i64, _>("saved_version")?).map_err(StoreError::codec)?,
            input_hash: row.try_get("input_hash")?,
            calculator_version: row.try_get("calculator_version")?,
            inputs,
            outputs,
            issues,
        })
    }

    async fn source_by_sha256(&self, sha256: &str) -> Result<Option<SourceRef>> {
        let row = sqlx::query("SELECT source_id, sha256, byte_len, original_name FROM sources WHERE sha256 = $1")
            .bind(sha256)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(row_to_source).transpose()
    }
}

fn row_to_source(row: &PgRow) -> Result<SourceRef> {
    Ok(SourceRef {
        source_id: row.try_get("source_id")?,
        sha256: row.try_get("sha256")?,
        byte_len: u64::try_from(row.try_get::<i64, _>("byte_len")?).map_err(StoreError::codec)?,
        original_name: row.try_get("original_name")?,
    })
}

fn parse_decimal(text: String, what: &str) -> Result<Decimal> {
    Decimal::from_str(&text).map_err(|e| StoreError::Codec(format!("{what}: {e}")))
}

async fn ensure_source(tx: &mut PgConnection, source: &SourceRef) -> Result<()> {
    source.validate().map_err(|e| StoreError::Invalid(e.to_string()))?;
    let row = sqlx::query("SELECT sha256, byte_len FROM sources WHERE source_id = $1")
        .bind(source.source_id)
        .fetch_optional(tx)
        .await?
        .ok_or_else(|| StoreError::Invalid("the source was not uploaded before the project".into()))?;
    let sha256: String = row.try_get("sha256")?;
    let byte_len = u64::try_from(row.try_get::<i64, _>("byte_len")?).map_err(StoreError::codec)?;
    if sha256 != source.sha256 || byte_len != source.byte_len {
        return Err(StoreError::Invalid("the source reference does not match the stored file".into()));
    }
    Ok(())
}

async fn insert_version(tx: &mut PgConnection, draft: &DraftProject, draft_json: &str) -> Result<()> {
    sqlx::query("INSERT INTO project_versions (project_id, version, draft, source_id) VALUES ($1, $2, $3::jsonb, $4)")
        .bind(draft.project_id)
        .bind(i64::try_from(draft.version).map_err(StoreError::codec)?)
        .bind(draft_json)
        .bind(draft.source.source_id)
        .execute(tx)
        .await?;
    Ok(())
}

fn check_request_id(request_id: &str) -> Result<()> {
    if request_id.chars().count() > MAX_REQUEST_ID_CHARS {
        return Err(StoreError::Invalid("the request id is too long".into()));
    }
    Ok(())
}

/// Serialise requests that share one request id inside the transaction. The advisory lock is
/// released at commit or rollback, so two concurrent copies of a retried write cannot both append.
async fn lock_request(tx: &mut PgConnection, request_id: &str) -> Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))").bind(request_id).execute(&mut *tx).await?;
    Ok(())
}

/// Every selected material and rate version must exist before a version is written. A missing
/// selection (`None`) is valid and becomes a named calculation issue later; an unknown id is an
/// invalid reference and the save is refused without writing anything.
async fn validate_references(tx: &mut PgConnection, draft: &DraftProject) -> Result<()> {
    for part in &draft.parts {
        if let Some(material_version_id) = part.material_version_id {
            let found: Option<Uuid> = sqlx::query_scalar("SELECT material_version_id FROM material_versions WHERE material_version_id = $1")
                .bind(material_version_id)
                .fetch_optional(&mut *tx)
                .await?;
            if found.is_none() {
                return Err(StoreError::Invalid(format!("the material version `{material_version_id}` does not exist")));
            }
        }
        let Some(operation) = &part.operation else { continue };
        for (what, rate_version_id) in [("production", operation.production_rate_version_id), ("setup", operation.setup_rate_version_id)] {
            let Some(rate_version_id) = rate_version_id else { continue };
            let found: Option<Uuid> = sqlx::query_scalar("SELECT rate_version_id FROM rate_versions WHERE rate_version_id = $1")
                .bind(rate_version_id)
                .fetch_optional(&mut *tx)
                .await?;
            if found.is_none() {
                return Err(StoreError::Invalid(format!("the {what} rate version `{rate_version_id}` does not exist")));
            }
        }
    }
    Ok(())
}

/// The calculator names the part it could not cost by its resolved `part_version_id` (in
/// `Issue::definition_id`); the reviewed definition is what the client shows. Replace it.
fn map_issue_definitions(outcome: &mut CalculationOutcome, draft: &DraftProject) {
    let definitions: HashMap<Uuid, Uuid> = draft.parts.iter().map(|part| (part.part_version_id, part.definition_id)).collect();
    for issue in &mut outcome.issues {
        if let Some(part_version_id) = issue.definition_id
            && let Some(definition_id) = definitions.get(&part_version_id)
        {
            issue.definition_id = Some(*definition_id);
        }
    }
}

async fn lookup_request(tx: &mut PgConnection, request_id: &str) -> Result<Option<(String, serde_json::Value)>> {
    let row = sqlx::query("SELECT payload_hash, response::text AS response FROM requests WHERE request_id = $1")
        .bind(request_id)
        .fetch_optional(tx)
        .await?;
    let Some(row) = row else { return Ok(None) };
    let hash: String = row.try_get("payload_hash")?;
    let text: String = row.try_get("response")?;
    let response = serde_json::from_str(&text).map_err(StoreError::codec)?;
    Ok(Some((hash, response)))
}

async fn insert_request<T: Serialize>(tx: &mut PgConnection, request_id: &str, operation: &str, payload: &str, response: &T) -> Result<()> {
    let response = serde_json::to_string(response).map_err(StoreError::codec)?;
    sqlx::query("INSERT INTO requests (request_id, operation, payload_hash, response) VALUES ($1, $2, $3, $4::jsonb)")
        .bind(request_id)
        .bind(operation)
        .bind(payload)
        .bind(response)
        .execute(tx)
        .await?;
    Ok(())
}

/// The canonical hash of a validated value, not of the raw request bytes: key order and transport
/// details do not decide whether a retry is the same request.
fn payload_hash<T: Serialize>(operation: &str, value: &T) -> Result<String> {
    let bytes = serde_json::to_vec(&(operation, value)).map_err(StoreError::codec)?;
    Ok(hex::encode(Sha256::digest(&bytes)))
}

async fn reference_data_in(tx: &mut PgConnection) -> Result<ReferenceData> {
    let material_rows = sqlx::query(
        "SELECT material_version_id, name, density_g_cm3::text AS density, price_per_kg::text AS price, currency \
         FROM material_versions ORDER BY name, material_version_id",
    )
    .fetch_all(&mut *tx)
    .await?;
    let mut materials = Vec::with_capacity(material_rows.len());
    for row in &material_rows {
        materials.push(MaterialVersion {
            material_version_id: row.try_get("material_version_id")?,
            name: row.try_get("name")?,
            density_g_cm3: parse_decimal(row.try_get("density")?, "material density")?,
            price_per_kg: parse_decimal(row.try_get("price")?, "material price")?,
            currency: row.try_get("currency")?,
        });
    }
    let rate_rows =
        sqlx::query("SELECT rate_version_id, name, rate_per_hour::text AS rate, currency FROM rate_versions ORDER BY name, rate_version_id")
            .fetch_all(&mut *tx)
            .await?;
    let mut rates = Vec::with_capacity(rate_rows.len());
    for row in &rate_rows {
        rates.push(RateVersion {
            rate_version_id: row.try_get("rate_version_id")?,
            name: row.try_get("name")?,
            rate_per_hour: parse_decimal(row.try_get("rate")?, "rate")?,
            currency: row.try_get("currency")?,
        });
    }
    let data = ReferenceData { materials, rates };
    data.validate().map_err(|e| StoreError::Codec(e.to_string()))?;
    Ok(data)
}

/// Turn a draft into calculator inputs, recording the review problems the calculator cannot see.
/// A missing value stays `None`; it never becomes zero here.
fn resolve_inputs(draft: &DraftProject, reference: &ReferenceData) -> (CalculationInputs, Vec<Issue>) {
    let mut issues = Vec::new();
    let mut parts = Vec::with_capacity(draft.parts.len());
    for (index, part) in draft.parts.iter().enumerate() {
        let path = |field: &str| format!("parts[{index}].{field}");
        let occurrences = draft.analysis.occurrences.iter().filter(|occurrence| occurrence.definition_id == part.definition_id).count();
        let quantity = match u32::try_from(occurrences) {
            Ok(quantity) => quantity,
            Err(_) => {
                issues.push(
                    Issue::new("too_many_occurrences", path("quantity"), "the occurrence count is too large").for_definition(part.definition_id),
                );
                u32::MAX
            }
        };
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
    let inputs = CalculationInputs { currency: draft.currency.clone(), material_overhead_rate: draft.material_overhead_rate, parts };
    (inputs, issues)
}
