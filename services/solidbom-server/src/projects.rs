//! Project, delivery and part-version storage and behaviour (contract version 2, package SB-10).
//!
//! One customer project owns its products, delivery packages, product revisions and shared part
//! versions. Each accepted write appends one immutable project record version and advances the
//! current version, so earlier definitions, edits and quotation events stay readable.
//!
//! A delivery records a package with its original source and one draft revision per delivered
//! product. Omitted products keep their current revision. Customer part number and revision
//! propose a shared project part version; geometry validates the proposal and anything missing,
//! conflicting or ambiguous stays unassociated until an engineer confirms the match.
//!
//! Activation belongs to SB-19: this module stores and validates the `draft`/`active`/`historical`
//! identities but publishes no activation route. `record_revision_activation` is the internal
//! transition SB-19 will drive; it is not part of the HTTP contract.

use solidbom_contract::{
    ConfirmPartMatch, CreateProject, DeliveredPart, DeliveredPartInput, DeliveredProductInput, ExcludeProducts, PartVersion, Product,
    ProductRevision, ProductRevisionState, ProjectDocument, ProjectFact, ProjectReceipt, ProjectRecordView, ProjectWriteOutcome, QuotationEvent,
    QuotationState, RecordDelivery, ReinstateProducts, UpdateProduct, UpdateProject, geometry_validates,
};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::Result;
use crate::store::{Store, StoreError, check_request_id, ensure_source, insert_request, lock_request, lookup_request, payload_hash};

/// The prefix of the project record operations recorded in the shared request log.
const OPERATION_PREFIX: &str = "project_record";

impl Store {
    /// Create a project record with its business details.
    pub async fn create_project(&self, request: &CreateProject) -> Result<ProjectWriteOutcome> {
        request.validate().map_err(invalid)?;
        let payload = payload_hash(&format!("{OPERATION_PREFIX}.create"), request)?;
        let mut tx = self.pool().begin().await?;
        lock_request(&mut tx, &request.request_id).await?;
        if let Some(outcome) = replay(&mut tx, &request.request_id, &payload).await? {
            tx.commit().await?;
            return Ok(outcome);
        }
        let existing: Option<Uuid> = sqlx::query_scalar("SELECT project_id FROM project_records WHERE project_id = $1")
            .bind(request.project_id)
            .fetch_optional(&mut *tx)
            .await?;
        if existing.is_some() {
            return Err(StoreError::Conflict("the project id already exists".into()));
        }
        let document = ProjectDocument::new(request.project_id, &request.details);
        document.validate().map_err(invalid)?;
        sqlx::query("INSERT INTO project_records (project_id, current_version) VALUES ($1, 1)").bind(request.project_id).execute(&mut *tx).await?;
        append_document(&mut tx, request.project_id, 1, &document).await?;
        let outcome =
            finish(&mut tx, &request.request_id, &format!("{OPERATION_PREFIX}.create"), &payload, request.project_id, 1, Vec::new()).await?;
        tx.commit().await?;
        Ok(outcome)
    }

    /// Update the editable project details. Expected-version checked.
    pub async fn update_project(&self, project_id: Uuid, request: &UpdateProject) -> Result<ProjectWriteOutcome> {
        request.validate().map_err(invalid)?;
        let payload = payload_hash(&format!("{OPERATION_PREFIX}.update"), request)?;
        self.write_document(project_id, &request.request_id, request.expected_version, &payload, "update", |document| {
            document.apply_details(&request.details);
            Ok(Vec::new())
        })
        .await
    }

    /// Update the editable product fields: name, kind, customer number, schedule and forecasts.
    pub async fn update_product(&self, project_id: Uuid, request: &UpdateProduct) -> Result<ProjectWriteOutcome> {
        request.validate().map_err(invalid)?;
        let payload = payload_hash(&format!("{OPERATION_PREFIX}.update_product"), request)?;
        self.write_document(project_id, &request.request_id, request.expected_version, &payload, "update_product", |document| {
            let product = document.product_mut(request.product_id).map_err(invalid)?;
            product.name = request.name.clone();
            product.kind = request.kind;
            product.customer_part_number = request.customer_part_number.clone();
            product.schedule_override = request.schedule_override;
            product.forecasts = request.forecasts.clone();
            Ok(Vec::new())
        })
        .await
    }

    /// Record one customer delivery: the package, its original source and the product revisions it
    /// delivers. A delivered existing product gets a new draft; an omitted product is unchanged.
    pub async fn record_delivery(&self, project_id: Uuid, request: &RecordDelivery) -> Result<ProjectWriteOutcome> {
        request.validate().map_err(invalid)?;
        let payload = payload_hash(&format!("{OPERATION_PREFIX}.delivery"), request)?;
        let mut tx = self.pool().begin().await?;
        lock_request(&mut tx, &request.request_id).await?;
        if let Some(outcome) = replay(&mut tx, &request.request_id, &payload).await? {
            tx.commit().await?;
            return Ok(outcome);
        }
        let (current, mut document) = current_document(&mut tx, project_id).await?;
        if current != request.expected_version {
            return Err(version_conflict(current, request.expected_version));
        }
        if document.packages.iter().any(|package| package.package_id == request.package.package_id) {
            return Err(StoreError::Conflict("the delivery package id already exists in this project".into()));
        }
        ensure_source(&mut tx, &request.package.source).await?;
        document.packages.push(request.package.clone());
        let mut product_ids = Vec::with_capacity(request.products.len());
        let mut pending = 0u64;
        for delivered in &request.products {
            let product_id = resolve_product(&mut document, delivered)?;
            let (revision, product_pending) = delivered_revision(&mut document, product_id, request.package.package_id, &delivered.parts)?;
            pending += product_pending;
            document.push_revision(product_id, revision).map_err(invalid)?;
            product_ids.push(product_id);
        }
        document.validate().map_err(invalid)?;
        let version = current + 1;
        append_document(&mut tx, project_id, version, &document).await?;
        advance_version(&mut tx, project_id, version).await?;
        let facts = vec![ProjectFact::DeliveryRecorded { package_id: request.package.package_id, product_ids, pending_part_matches: pending }];
        let outcome = finish(&mut tx, &request.request_id, &format!("{OPERATION_PREFIX}.delivery"), &payload, project_id, version, facts).await?;
        tx.commit().await?;
        Ok(outcome)
    }

    /// Confirm one pending delivered-part match against a shared project part version.
    pub async fn confirm_part_match(&self, project_id: Uuid, request: &ConfirmPartMatch) -> Result<ProjectWriteOutcome> {
        request.validate().map_err(invalid)?;
        let payload = payload_hash(&format!("{OPERATION_PREFIX}.part_match"), request)?;
        self.write_document(project_id, &request.request_id, request.expected_version, &payload, "part_match", |document| {
            let part_version_id = document
                .part_versions
                .iter()
                .find(|part| part.part_version_id == request.part_version_id)
                .ok_or_else(|| StoreError::Invalid("the project has no such part version".into()))?
                .part_version_id;
            let part = document.delivered_part_mut(request.product_revision_id, &request.delivered_key).map_err(invalid)?;
            if part.part_version_id.is_some() {
                return Err(StoreError::Conflict("the delivered part already has a confirmed part version".into()));
            }
            part.part_version_id = Some(part_version_id);
            Ok(vec![ProjectFact::PartMatchConfirmed {
                product_revision_id: request.product_revision_id,
                delivered_key: request.delivered_key.clone(),
                part_version_id,
            }])
        })
        .await
    }

    /// Exclude products from the quotation scope; their packages, revisions and history stay.
    pub async fn exclude_products(&self, project_id: Uuid, request: &ExcludeProducts) -> Result<ProjectWriteOutcome> {
        request.validate().map_err(invalid)?;
        let payload = payload_hash(&format!("{OPERATION_PREFIX}.exclude"), request)?;
        self.write_document(project_id, &request.request_id, request.expected_version, &payload, "exclude", |document| {
            set_quotation_scope(document, &request.product_ids, QuotationState::Excluded)
        })
        .await
    }

    /// Reinstate previously excluded products into the quotation scope.
    pub async fn reinstate_products(&self, project_id: Uuid, request: &ReinstateProducts) -> Result<ProjectWriteOutcome> {
        request.validate().map_err(invalid)?;
        let payload = payload_hash(&format!("{OPERATION_PREFIX}.reinstate"), request)?;
        self.write_document(project_id, &request.request_id, request.expected_version, &payload, "reinstate", |document| {
            set_quotation_scope(document, &request.product_ids, QuotationState::Included)
        })
        .await
    }

    /// The current project record and its computed issues.
    pub async fn project_record_view(&self, project_id: Uuid) -> Result<ProjectRecordView> {
        let (version, document) = read_document(self.pool(), project_id).await?;
        Ok(view(project_id, version, document))
    }

    /// One retained project record version, which may be historical.
    pub async fn project_record_version(&self, project_id: Uuid, version: u64) -> Result<ProjectRecordView> {
        let text: String = sqlx::query_scalar("SELECT document::text FROM project_record_versions WHERE project_id = $1 AND version = $2")
            .bind(project_id)
            .bind(version_to_i64(version)?)
            .fetch_optional(self.pool())
            .await?
            .ok_or(StoreError::NotFound)?;
        let document: ProjectDocument = serde_json::from_str(&text).map_err(StoreError::codec)?;
        Ok(view(project_id, version, document))
    }

    /// How many immutable record versions a project holds (diagnostics and tests).
    pub async fn project_record_version_count(&self, project_id: Uuid) -> Result<u64> {
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM project_record_versions WHERE project_id = $1").bind(project_id).fetch_one(self.pool()).await?;
        u64::try_from(count).map_err(StoreError::codec)
    }

    /// Record the draft-to-active transition of one product revision and keep the previously
    /// active revision as history. This is the internal transition the activation package SB-19
    /// will drive; no HTTP route publishes it in contract version 2.
    pub async fn record_revision_activation(&self, project_id: Uuid, product_revision_id: Uuid) -> Result<ProjectRecordView> {
        let mut tx = self.pool().begin().await?;
        let (current, mut document) = current_document(&mut tx, project_id).await?;
        let position = document.products.iter().enumerate().find_map(|(product_index, product)| {
            product
                .revisions
                .iter()
                .position(|revision| revision.product_revision_id == product_revision_id)
                .map(|revision_index| (product_index, revision_index))
        });
        let (product_index, revision_index) = position.ok_or_else(|| StoreError::Invalid("the project has no such product revision".into()))?;
        if document.products[product_index].revisions[revision_index].state != ProductRevisionState::Draft {
            return Err(StoreError::Conflict("only a draft product revision can be activated".into()));
        }
        if let Some(previous) = document.products[product_index].active_revision_id
            && let Some(revision) = document.products[product_index].revisions.iter_mut().find(|revision| revision.product_revision_id == previous)
        {
            revision.state = ProductRevisionState::Historical;
        }
        document.products[product_index].revisions[revision_index].state = ProductRevisionState::Active;
        document.products[product_index].active_revision_id = Some(product_revision_id);
        document.validate().map_err(invalid)?;
        let version = current + 1;
        append_document(&mut tx, project_id, version, &document).await?;
        advance_version(&mut tx, project_id, version).await?;
        tx.commit().await?;
        Ok(view(project_id, version, document))
    }

    /// A plain expected-version-checked document write: load under the row lock, apply, validate,
    /// append and record the request. The closure returns the emitted dependency facts.
    async fn write_document<F>(
        &self,
        project_id: Uuid,
        request_id: &str,
        expected_version: u64,
        payload: &str,
        operation: &str,
        apply: F,
    ) -> Result<ProjectWriteOutcome>
    where
        F: FnOnce(&mut ProjectDocument) -> Result<Vec<ProjectFact>>,
    {
        check_request_id(request_id)?;
        let mut tx = self.pool().begin().await?;
        lock_request(&mut tx, request_id).await?;
        if let Some(outcome) = replay(&mut tx, request_id, payload).await? {
            tx.commit().await?;
            return Ok(outcome);
        }
        let (current, mut document) = current_document(&mut tx, project_id).await?;
        if current != expected_version {
            return Err(version_conflict(current, expected_version));
        }
        let facts = apply(&mut document)?;
        document.validate().map_err(invalid)?;
        let version = current + 1;
        append_document(&mut tx, project_id, version, &document).await?;
        advance_version(&mut tx, project_id, version).await?;
        let outcome = finish(&mut tx, request_id, &format!("{OPERATION_PREFIX}.{operation}"), payload, project_id, version, facts).await?;
        tx.commit().await?;
        Ok(outcome)
    }
}

fn invalid(error: solidbom_contract::ContractError) -> StoreError {
    StoreError::Invalid(error.to_string())
}

fn version_to_i64(version: u64) -> Result<i64> {
    i64::try_from(version).map_err(StoreError::codec)
}

fn version_conflict(current: u64, expected: u64) -> StoreError {
    StoreError::Conflict(format!("the project is at version {current}, not {expected}"))
}

async fn replay(tx: &mut Transaction<'_, Postgres>, request_id: &str, payload: &str) -> Result<Option<ProjectWriteOutcome>> {
    if let Some((stored, response)) = lookup_request(&mut *tx, request_id).await? {
        if stored != payload {
            return Err(StoreError::Conflict("the request ID was reused with a different payload".into()));
        }
        return serde_json::from_value(response).map(Some).map_err(StoreError::codec);
    }
    Ok(None)
}

async fn finish(
    tx: &mut Transaction<'_, Postgres>,
    request_id: &str,
    operation: &str,
    payload: &str,
    project_id: Uuid,
    version: u64,
    facts: Vec<ProjectFact>,
) -> Result<ProjectWriteOutcome> {
    let outcome = ProjectWriteOutcome { receipt: ProjectReceipt { request_id: request_id.to_string(), project_id, version }, facts };
    insert_request(tx, request_id, operation, payload, &outcome).await?;
    Ok(outcome)
}

async fn current_document(tx: &mut Transaction<'_, Postgres>, project_id: Uuid) -> Result<(u64, ProjectDocument)> {
    let current: i64 = sqlx::query_scalar("SELECT current_version FROM project_records WHERE project_id = $1 FOR UPDATE")
        .bind(project_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(StoreError::NotFound)?;
    let current = u64::try_from(current).map_err(StoreError::codec)?;
    let text: String = sqlx::query_scalar("SELECT document::text FROM project_record_versions WHERE project_id = $1 AND version = $2")
        .bind(project_id)
        .bind(version_to_i64(current)?)
        .fetch_one(&mut **tx)
        .await?;
    let document: ProjectDocument = serde_json::from_str(&text).map_err(StoreError::codec)?;
    Ok((current, document))
}

async fn read_document(pool: &sqlx::PgPool, project_id: Uuid) -> Result<(u64, ProjectDocument)> {
    let current: i64 = sqlx::query_scalar("SELECT current_version FROM project_records WHERE project_id = $1")
        .bind(project_id)
        .fetch_optional(pool)
        .await?
        .ok_or(StoreError::NotFound)?;
    let current = u64::try_from(current).map_err(StoreError::codec)?;
    let text: String = sqlx::query_scalar("SELECT document::text FROM project_record_versions WHERE project_id = $1 AND version = $2")
        .bind(project_id)
        .bind(version_to_i64(current)?)
        .fetch_one(pool)
        .await?;
    let document: ProjectDocument = serde_json::from_str(&text).map_err(StoreError::codec)?;
    Ok((current, document))
}

async fn append_document(tx: &mut Transaction<'_, Postgres>, project_id: Uuid, version: u64, document: &ProjectDocument) -> Result<()> {
    let json = serde_json::to_string(document).map_err(StoreError::codec)?;
    sqlx::query("INSERT INTO project_record_versions (project_id, version, document) VALUES ($1, $2, $3::jsonb)")
        .bind(project_id)
        .bind(version_to_i64(version)?)
        .bind(json)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn advance_version(tx: &mut Transaction<'_, Postgres>, project_id: Uuid, version: u64) -> Result<()> {
    sqlx::query("UPDATE project_records SET current_version = $2 WHERE project_id = $1")
        .bind(project_id)
        .bind(version_to_i64(version)?)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

fn view(project_id: Uuid, version: u64, document: ProjectDocument) -> ProjectRecordView {
    let issues = document.issues();
    ProjectRecordView { project_id, version, document, issues }
}

/// Resolve the project product a delivered product is. An explicit id wins; otherwise the
/// customer part number proposes a unique product, then the name does. An ambiguous proposal is
/// refused instead of silently sharing one product's work.
fn resolve_product(document: &mut ProjectDocument, delivered: &DeliveredProductInput) -> Result<Uuid> {
    if let Some(product_id) = delivered.product_id {
        let product =
            document.product(product_id).ok_or_else(|| StoreError::Invalid("the delivery names a project product that does not exist".into()))?;
        if product.kind != delivered.kind {
            return Err(StoreError::Invalid("the delivered product kind differs from the project product".into()));
        }
        if let (Some(existing), Some(given)) = (&product.customer_part_number, &delivered.customer_part_number)
            && existing != given
        {
            return Err(StoreError::Invalid("the delivered customer part number differs from the project product".into()));
        }
        let fill_number = product.customer_part_number.is_none();
        if fill_number && let Some(number) = &delivered.customer_part_number {
            let product = document.product_mut(product_id).map_err(invalid)?;
            product.customer_part_number = Some(number.clone());
        }
        return Ok(product_id);
    }
    let candidates: Vec<&Product> = match &delivered.customer_part_number {
        Some(number) if !number.trim().is_empty() => {
            document.products.iter().filter(|product| product.customer_part_number.as_deref() == Some(number.as_str())).collect()
        }
        _ => document.products.iter().filter(|product| product.name.trim().eq_ignore_ascii_case(delivered.name.trim())).collect(),
    };
    match candidates.len() {
        0 => create_product(document, delivered),
        1 => {
            let product = candidates[0];
            if product.kind != delivered.kind {
                return Err(StoreError::Invalid("the delivered product kind differs from the project product".into()));
            }
            Ok(product.product_id)
        }
        _ => Err(StoreError::Invalid("more than one project product matches the delivered product; name the project product explicitly".into())),
    }
}

fn create_product(document: &mut ProjectDocument, delivered: &DeliveredProductInput) -> Result<Uuid> {
    let product_id = Uuid::new_v4();
    document.products.push(Product {
        product_id,
        name: delivered.name.clone(),
        kind: delivered.kind,
        customer_part_number: delivered.customer_part_number.clone(),
        schedule_override: None,
        forecasts: Vec::new(),
        revisions: Vec::new(),
        active_revision_id: None,
        quotation_state: QuotationState::Included,
    });
    Ok(product_id)
}

/// Build one delivered product revision: each delivered definition is associated with the shared
/// project part version its customer identifiers propose and geometry validates, or left pending.
fn delivered_revision(
    document: &mut ProjectDocument,
    product_id: Uuid,
    package_id: Uuid,
    parts: &[DeliveredPartInput],
) -> Result<(ProductRevision, u64)> {
    let product = document.product(product_id).ok_or_else(|| StoreError::Invalid("the project has no such product".into()))?;
    if product.revisions.len() >= solidbom_contract::MAX_REVISIONS_PER_PRODUCT {
        return Err(StoreError::Invalid("the product holds the maximum number of revisions".into()));
    }
    let mut delivered_parts = Vec::with_capacity(parts.len());
    let mut pending = 0u64;
    for part in parts {
        let part_version_id = match part.identity_keys() {
            Some((number, revision)) => {
                let existing = document.part_version_by_identity(number, revision).map(|part| (part.part_version_id, part.volume_mm3));
                match existing {
                    Some((part_version_id, stored_volume)) => match geometry_validates(stored_volume, part.volume_mm3) {
                        Some(true) => Some(part_version_id),
                        // Conflicting or unavailable geometry: the engineer confirms the match.
                        _ => {
                            pending += 1;
                            None
                        }
                    },
                    None => {
                        let part_version_id = Uuid::new_v4();
                        document.part_versions.push(PartVersion {
                            part_version_id,
                            customer_part_number: number.to_string(),
                            customer_revision: revision.to_string(),
                            volume_mm3: part.volume_mm3,
                            created_in_package_id: package_id,
                        });
                        Some(part_version_id)
                    }
                }
            }
            None => {
                pending += 1;
                None
            }
        };
        delivered_parts.push(DeliveredPart {
            delivered_key: part.delivered_key.clone(),
            customer_part_number: part.customer_part_number.clone(),
            customer_revision: part.customer_revision.clone(),
            volume_mm3: part.volume_mm3,
            part_version_id,
        });
    }
    Ok((ProductRevision { product_revision_id: Uuid::new_v4(), package_id, state: ProductRevisionState::Draft, parts: delivered_parts }, pending))
}

/// Set the quotation scope of products and append the recorded event. Nothing is removed.
fn set_quotation_scope(document: &mut ProjectDocument, product_ids: &[Uuid], state: QuotationState) -> Result<Vec<ProjectFact>> {
    let sequence = document.next_quotation_sequence();
    for product_id in product_ids {
        let product = document.product_mut(*product_id).map_err(invalid)?;
        if product.quotation_state == state {
            let word = match state {
                QuotationState::Excluded => "excluded",
                QuotationState::Included => "included",
            };
            return Err(StoreError::Invalid(format!("the product is already {word} in the quotation scope")));
        }
        product.quotation_state = state;
    }
    document.quotation_events.push(QuotationEvent { sequence, product_ids: product_ids.to_vec(), state });
    Ok(vec![ProjectFact::QuotationScopeChanged { product_ids: product_ids.to_vec(), state }])
}
