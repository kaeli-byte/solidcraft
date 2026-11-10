//! Sourcing, routing and attached-cost storage (package SB-15, additive to contract version 2).
//!
//! One project records how each shared project part version is sourced and made, beside its
//! project record and its engineering review:
//!
//! - [`Store::set_sourcing`] writes the explicit purchased/manufactured choice of one or more
//!   shared part versions, with the material consumption and allowance of a made part. The record
//!   is keyed by `part_version_id`, so every occurrence of that version reads the same choice while
//!   another customer revision stays a separate part version with its own record.
//! - [`Store::publish_routing_template`] publishes the next immutable version of a reusable company
//!   routing; the template stays independent of every copy applied from it.
//! - [`Store::write_routing`] adopts one exact template version (copying its operations into the
//!   applied routing), edits an applied routing without touching the template, or enters a routing
//!   without a template. Adoption is the only write that changes a template source link.
//! - [`Store::edit_attached_costs`] upserts and removes non-CAD cost lines of a part version or
//!   product root.
//!
//! Every accepted project write appends one immutable manufacturing record version under an
//! expected-version check and the shared request log, so request-id idempotency and stale-version
//! conflicts behave exactly like the project and engineering writes. Facts name what changed;
//! issues are computed on read and are never stored. A missing cycle time, cavity count, rate
//! version, consumption or attached unit cost is an issue, never a zero.

use std::collections::BTreeSet;

use solidbom_contract::{
    AdoptRoutingTemplate, AppliedRouting, CostCenterPurpose, CostOwner, EditAttachedCosts, EditRouting, ManualRouting, ManufacturingFact,
    ManufacturingReceipt, ManufacturingRecordDocument, ManufacturingRecordView, ManufacturingWriteOutcome, ProjectDocument, PublishRoutingTemplate,
    ROUTING_TEMPLATE_DOCUMENT_SCHEMA_VERSION, ReviewState, RoutingOperation, RoutingSource, RoutingTemplateList, RoutingTemplateReceipt,
    RoutingTemplateVersion, RoutingWrite, SetSourcing, TemplateRef, check_affected_owner,
};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::Result;
use crate::store::{
    Store, StoreError, check_request_id, lock_logical_record, lock_request, material_versions_in, next_version, payload_hash, record_reply,
    recorded_reply,
};

/// The prefix of the manufacturing record operations recorded in the shared request log.
const OPERATION_PREFIX: &str = "manufacturing_record";

/// The RFC 3339 UTC spelling the service stores into a template document. A client never parses a
/// locale timestamp; the literal `Z` marks UTC.
const RFC3339_UTC: &str = "'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"'";

impl Store {
    /// Publish the next immutable version of one reusable routing template. `expected_version` 0
    /// creates version 1; a later publish names the current version and appends the next one. The
    /// write is keyed by the request log, so an identical retry returns its recorded receipt.
    pub async fn publish_routing_template(&self, request: &PublishRoutingTemplate) -> Result<RoutingTemplateReceipt> {
        request.validate().map_err(invalid)?;
        let operation = format!("{OPERATION_PREFIX}.template_publish");
        let payload = payload_hash(&operation, request)?;
        let mut tx = self.pool().begin().await?;
        lock_request(&mut tx, &request.request_id).await?;
        lock_logical_record(&mut tx, "routing_template", request.template_id).await?;
        if let Some(receipt) = recorded_reply::<RoutingTemplateReceipt>(&mut tx, &request.request_id, &payload).await? {
            tx.commit().await?;
            return Ok(receipt);
        }
        // A pinned rate version must still exist and keep the purpose the operation assumes; a
        // template never invents a rate or a cost center.
        let rates = pinned_rates(&mut tx).await?;
        validate_operation_rates(&request.operations, &rates)?;
        let latest = latest_template(&mut tx, request.template_id).await?;
        let (version, supersedes) =
            next_version("routing template", request.expected_version, latest.as_ref().map(|row| (row.template_version_id, row.version)))?;
        let template_version_id = Uuid::new_v4();
        let created_at = service_timestamp(&mut tx).await?;
        let document = RoutingTemplateVersion {
            schema_version: ROUTING_TEMPLATE_DOCUMENT_SCHEMA_VERSION,
            template_version_id,
            template_id: request.template_id,
            version,
            supersedes,
            name: request.name.clone(),
            description: request.description.clone(),
            applies_to: request.applies_to.clone(),
            operations: request.operations.clone(),
            source_reference: request.source_reference.clone(),
            created_at: Some(created_at),
        };
        document.validate().map_err(invalid)?;
        let json = serde_json::to_string(&document).map_err(StoreError::codec)?;
        sqlx::query(
            "INSERT INTO routing_templates (template_id, current_version) VALUES ($1, $2) \
             ON CONFLICT (template_id) DO UPDATE SET current_version = EXCLUDED.current_version",
        )
        .bind(request.template_id)
        .bind(version_to_i64(version)?)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO routing_template_versions (template_id, version, template_version_id, document, created_at) \
             VALUES ($1, $2, $3, $4::jsonb, $5::timestamptz)",
        )
        .bind(request.template_id)
        .bind(version_to_i64(version)?)
        .bind(template_version_id)
        .bind(json)
        .bind(&document.created_at)
        .execute(&mut *tx)
        .await?;
        let receipt =
            RoutingTemplateReceipt { request_id: request.request_id.clone(), template_id: request.template_id, version, template_version_id };
        record_reply(&mut tx, &request.request_id, &operation, &payload, &receipt).await?;
        tx.commit().await?;
        Ok(receipt)
    }

    /// The current version of every published routing template, so an engineer can choose one to
    /// apply and an administrator can see the maintained set.
    pub async fn routing_templates(&self) -> Result<RoutingTemplateList> {
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT v.document::text FROM routing_templates t \
             JOIN routing_template_versions v ON v.template_id = t.template_id AND v.version = t.current_version \
             ORDER BY t.created_at, t.template_id",
        )
        .fetch_all(self.pool())
        .await?;
        let mut templates = Vec::with_capacity(rows.len());
        for text in rows {
            templates.push(read_template_document(&text)?);
        }
        let list = RoutingTemplateList { templates };
        list.validate().map_err(StoreError::codec)?;
        Ok(list)
    }

    /// One retained, exact published template version.
    pub async fn routing_template_version(&self, template_id: Uuid, version: u64) -> Result<RoutingTemplateVersion> {
        let text: Option<String> = sqlx::query_scalar("SELECT document::text FROM routing_template_versions WHERE template_id = $1 AND version = $2")
            .bind(template_id)
            .bind(version_to_i64(version)?)
            .fetch_optional(self.pool())
            .await?;
        let text = text.ok_or(StoreError::NotFound)?;
        read_template_document(&text)
    }

    /// The current manufacturing definition of a project: its append-only version, the sourced
    /// parts, applied routings and attached cost lines, and the computed issues. A project with no
    /// recorded definition reads as version 0 with every part version still unsourced.
    pub async fn manufacturing_record_view(&self, project_id: Uuid) -> Result<ManufacturingRecordView> {
        let mut tx = self.pool().begin().await?;
        let project = project_document(&mut tx, project_id).await?;
        let (version, document) = read_current(&mut tx, project_id).await?;
        tx.commit().await?;
        Ok(view(project_id, version, document, &project))
    }

    /// One retained manufacturing record version, which may be historical. Its document is exactly
    /// what was written; the issues are recomputed against the current project record.
    pub async fn manufacturing_record_version(&self, project_id: Uuid, version: u64) -> Result<ManufacturingRecordView> {
        let mut tx = self.pool().begin().await?;
        let project = project_document(&mut tx, project_id).await?;
        let text: Option<String> =
            sqlx::query_scalar("SELECT document::text FROM manufacturing_record_versions WHERE project_id = $1 AND version = $2")
                .bind(project_id)
                .bind(version_to_i64(version)?)
                .fetch_optional(&mut *tx)
                .await?;
        let document = read_manufacturing_document(&text.ok_or(StoreError::NotFound)?)?;
        tx.commit().await?;
        Ok(view(project_id, version, document, &project))
    }

    /// Set the explicit sourcing record of one or more shared project part versions. The whole
    /// record of each named part version is replaced; the affected part versions are exactly the
    /// written ones. A confirmed material consumption records the authenticated caller as its
    /// reviewer: the payload's `confirmed_by` is never trusted.
    pub async fn set_sourcing(&self, project_id: Uuid, request: &SetSourcing, reviewer: &str) -> Result<ManufacturingWriteOutcome> {
        let mut normalized = request.clone();
        for part in &mut normalized.parts {
            if let Some(material) = &mut part.material {
                material.confirmed_by = match material.review {
                    ReviewState::Confirmed => Some(reviewer.to_string()),
                    ReviewState::Pending => None,
                };
            }
        }
        normalized.validate().map_err(invalid)?;
        let operation = format!("{OPERATION_PREFIX}.sourcing");
        let payload = payload_hash(&operation, &(project_id, &normalized))?;
        let affected_part_versions = normalized.affected_part_versions.clone();
        self.write_manufacturing_document(
            project_id,
            &normalized.request_id,
            normalized.expected_version,
            &payload,
            &operation,
            &[],
            affected_part_versions,
            Vec::new(),
            |document, project, support| {
                for part in &normalized.parts {
                    ensure_part_version(project, part.part_version_id)?;
                    if let Some(material) = &part.material
                        && let Some(material_version_id) = material.material_version_id
                        && !support.materials.contains(&material_version_id)
                    {
                        return Err(StoreError::Invalid(format!(
                            "the selected material version `{material_version_id}` is not a published reference version"
                        )));
                    }
                }
                let mut facts = Vec::with_capacity(normalized.parts.len());
                for part in &normalized.parts {
                    match document.part_mut(part.part_version_id) {
                        Some(existing) => *existing = part.clone(),
                        None => document.parts.push(part.clone()),
                    }
                    facts.push(ManufacturingFact::SourcingRecorded { part_version_id: part.part_version_id, sourcing: part.sourcing });
                }
                Ok(facts)
            },
        )
        .await
    }

    /// One typed routing write. `adopt_template` copies an exact published template version,
    /// `edit` replaces the operations of an existing applied routing while keeping its owner, level
    /// and source link, and `manual` enters a routing without a template. Adoption is the only
    /// write that changes a template source link, so an edit can never silently adopt a newer one.
    pub async fn write_routing(&self, project_id: Uuid, request: &RoutingWrite) -> Result<ManufacturingWriteOutcome> {
        request.validate().map_err(invalid)?;
        let operation = format!("{OPERATION_PREFIX}.routing");
        let payload = payload_hash(&operation, &(project_id, request))?;
        let templates: Vec<TemplateRef> = match request {
            RoutingWrite::AdoptTemplate(write) => vec![write.template],
            RoutingWrite::Edit(_) | RoutingWrite::Manual(_) => Vec::new(),
        };
        let (affected_part_versions, affected_products) = match request {
            RoutingWrite::AdoptTemplate(write) => (write.affected_part_versions.clone(), write.affected_products.clone()),
            RoutingWrite::Edit(write) => (write.affected_part_versions.clone(), write.affected_products.clone()),
            RoutingWrite::Manual(write) => (write.affected_part_versions.clone(), write.affected_products.clone()),
        };
        self.write_manufacturing_document(
            project_id,
            request.request_id(),
            request.expected_version(),
            &payload,
            &operation,
            &templates,
            affected_part_versions,
            affected_products,
            |document, project, support| match request {
                RoutingWrite::AdoptTemplate(write) => adopt_template(document, project, support, write),
                RoutingWrite::Edit(write) => edit_routing(document, project, support, write),
                RoutingWrite::Manual(write) => manual_routing(document, project, support, write),
            },
        )
        .await
    }

    /// Upsert and remove attached non-CAD cost lines of a part version or product root. A removed
    /// line id must exist in the current draft and its owner must be named as affected; earlier
    /// record versions keep every line.
    pub async fn edit_attached_costs(&self, project_id: Uuid, request: &EditAttachedCosts) -> Result<ManufacturingWriteOutcome> {
        request.validate().map_err(invalid)?;
        let operation = format!("{OPERATION_PREFIX}.attached_costs");
        let payload = payload_hash(&operation, &(project_id, request))?;
        self.write_manufacturing_document(
            project_id,
            &request.request_id,
            request.expected_version,
            &payload,
            &operation,
            &[],
            request.affected_part_versions.clone(),
            request.affected_products.clone(),
            |document, project, _support| apply_attached_costs(document, project, request),
        )
        .await
    }

    /// A plain expected-version-checked manufacturing document write: lock the request and the
    /// project record, replay a recorded reply, load the current project and manufacturing
    /// documents, apply, validate, append and record the request. The named template versions are
    /// loaded after the replay check, so a retried adoption still returns its recorded outcome.
    #[allow(clippy::too_many_arguments)]
    async fn write_manufacturing_document<F>(
        &self,
        project_id: Uuid,
        request_id: &str,
        expected_version: u64,
        payload: &str,
        operation: &str,
        templates: &[TemplateRef],
        affected_part_versions: Vec<Uuid>,
        affected_products: Vec<Uuid>,
        apply: F,
    ) -> Result<ManufacturingWriteOutcome>
    where
        F: FnOnce(&mut ManufacturingRecordDocument, &ProjectDocument, &WriteSupport) -> Result<Vec<ManufacturingFact>>,
    {
        check_request_id(request_id)?;
        let mut tx = self.pool().begin().await?;
        lock_request(&mut tx, request_id).await?;
        lock_manufacturing(&mut tx, project_id).await?;
        if let Some(outcome) = recorded_reply::<ManufacturingWriteOutcome>(&mut tx, request_id, payload).await? {
            tx.commit().await?;
            return Ok(outcome);
        }
        let project = project_document(&mut tx, project_id).await?;
        let (current, mut document) = locked_current(&mut tx, project_id).await?;
        if current != expected_version {
            return Err(version_conflict(current, expected_version));
        }
        let rates = pinned_rates(&mut tx).await?;
        let materials = material_versions_in(&mut tx).await?.into_iter().map(|material| material.material_version_id).collect();
        let mut loaded = Vec::with_capacity(templates.len());
        for template in templates {
            loaded.push(load_template(&mut tx, *template).await?);
        }
        let support = WriteSupport { rates, materials, templates: loaded };
        let facts = apply(&mut document, &project, &support)?;
        document.validate().map_err(invalid)?;
        let version = current + 1;
        ensure_record_row(&mut tx, project_id).await?;
        append_document(&mut tx, project_id, version, &document).await?;
        advance_version(&mut tx, project_id, version).await?;
        let outcome = ManufacturingWriteOutcome {
            receipt: ManufacturingReceipt { request_id: request_id.to_string(), project_id, version },
            affected_part_versions,
            affected_products,
            facts,
        };
        record_reply(&mut tx, request_id, operation, payload, &outcome).await?;
        tx.commit().await?;
        Ok(outcome)
    }
}

/// The reference data one manufacturing write validated against inside its transaction.
struct WriteSupport {
    rates: Vec<PinnedRate>,
    materials: Vec<Uuid>,
    templates: Vec<RoutingTemplateVersion>,
}

/// One pinned cost-center rate version, reduced to what a routing operation validates.
struct PinnedRate {
    rate_version_id: Uuid,
    cost_center_id: Uuid,
    purpose: CostCenterPurpose,
}

/// The latest published version of one logical template.
struct LatestTemplate {
    template_version_id: Uuid,
    version: u64,
}

fn invalid(error: solidbom_contract::ContractError) -> StoreError {
    StoreError::Invalid(error.to_string())
}

fn version_to_i64(version: u64) -> Result<i64> {
    // A stored version is always an i64; a requested version outside that range cannot exist, so
    // it is an unknown version rather than an internal failure.
    i64::try_from(version).map_err(|_| StoreError::NotFound)
}

fn version_conflict(current: u64, expected: u64) -> StoreError {
    StoreError::Conflict(format!("the manufacturing definition is at version {current}, not {expected}"))
}

/// The service timestamp a published template version records, in the same UTC spelling the other
/// shared records use.
async fn service_timestamp(tx: &mut Transaction<'_, Postgres>) -> Result<String> {
    let created_at = sqlx::query_scalar(&format!("SELECT to_char(now() AT TIME ZONE 'UTC', {RFC3339_UTC})")).fetch_one(&mut **tx).await?;
    Ok(created_at)
}

/// Serialise manufacturing writes of one project, including the first write that creates the
/// record row.
async fn lock_manufacturing(tx: &mut Transaction<'_, Postgres>, project_id: Uuid) -> Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("manufacturing_record:{project_id}"))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// The latest version of one logical template and its version id, used for the next-version check.
async fn latest_template(tx: &mut Transaction<'_, Postgres>, template_id: Uuid) -> Result<Option<LatestTemplate>> {
    let current: Option<i64> = sqlx::query_scalar("SELECT current_version FROM routing_templates WHERE template_id = $1")
        .bind(template_id)
        .fetch_optional(&mut **tx)
        .await?;
    let Some(current) = current else { return Ok(None) };
    let current = u64::try_from(current).map_err(StoreError::codec)?;
    let template_version_id: Uuid =
        sqlx::query_scalar("SELECT template_version_id FROM routing_template_versions WHERE template_id = $1 AND version = $2")
            .bind(template_id)
            .bind(version_to_i64(current)?)
            .fetch_one(&mut **tx)
            .await?;
    Ok(Some(LatestTemplate { template_version_id, version: current }))
}

/// The stored project document, read only for identity validation. Project identities are
/// append-only, so a reference this write validates stays valid.
async fn project_document(tx: &mut Transaction<'_, Postgres>, project_id: Uuid) -> Result<ProjectDocument> {
    let current: i64 = sqlx::query_scalar("SELECT current_version FROM project_records WHERE project_id = $1")
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
    document.validate().map_err(StoreError::codec)?;
    Ok(document)
}

/// The locked current manufacturing document. The project record was already proved to exist, so
/// a first write starts at version 0 with the empty record.
async fn locked_current(tx: &mut Transaction<'_, Postgres>, project_id: Uuid) -> Result<(u64, ManufacturingRecordDocument)> {
    let current: Option<i64> = sqlx::query_scalar("SELECT current_version FROM manufacturing_records WHERE project_id = $1 FOR UPDATE")
        .bind(project_id)
        .fetch_optional(&mut **tx)
        .await?;
    match current {
        None => Ok((0, ManufacturingRecordDocument::empty(project_id))),
        Some(current) => {
            let current = u64::try_from(current).map_err(StoreError::codec)?;
            let text: String = sqlx::query_scalar("SELECT document::text FROM manufacturing_record_versions WHERE project_id = $1 AND version = $2")
                .bind(project_id)
                .bind(version_to_i64(current)?)
                .fetch_one(&mut **tx)
                .await?;
            Ok((current, read_manufacturing_document(&text)?))
        }
    }
}

async fn read_current(tx: &mut Transaction<'_, Postgres>, project_id: Uuid) -> Result<(u64, ManufacturingRecordDocument)> {
    let current: Option<i64> = sqlx::query_scalar("SELECT current_version FROM manufacturing_records WHERE project_id = $1")
        .bind(project_id)
        .fetch_optional(&mut **tx)
        .await?;
    match current {
        None => Ok((0, ManufacturingRecordDocument::empty(project_id))),
        Some(current) => {
            let current = u64::try_from(current).map_err(StoreError::codec)?;
            let text: String = sqlx::query_scalar("SELECT document::text FROM manufacturing_record_versions WHERE project_id = $1 AND version = $2")
                .bind(project_id)
                .bind(version_to_i64(current)?)
                .fetch_one(&mut **tx)
                .await?;
            Ok((current, read_manufacturing_document(&text)?))
        }
    }
}

/// Every published cost-center rate version, reduced to the fields a pinned operation validates.
async fn pinned_rates(tx: &mut Transaction<'_, Postgres>) -> Result<Vec<PinnedRate>> {
    let rows: Vec<(Uuid, Uuid, String)> =
        sqlx::query_as("SELECT rate_version_id, cost_center_id, purpose FROM rate_versions ORDER BY rate_version_id").fetch_all(&mut **tx).await?;
    let mut rates = Vec::with_capacity(rows.len());
    for (rate_version_id, cost_center_id, purpose) in rows {
        let purpose = match purpose.as_str() {
            "production" => CostCenterPurpose::Production,
            "setup" => CostCenterPurpose::Setup,
            other => return Err(StoreError::Codec(format!("the stored cost-center purpose `{other}` is unknown"))),
        };
        rates.push(PinnedRate { rate_version_id, cost_center_id, purpose });
    }
    Ok(rates)
}

/// One exact published template version, read inside the write transaction.
async fn load_template(tx: &mut Transaction<'_, Postgres>, template: TemplateRef) -> Result<RoutingTemplateVersion> {
    let text: Option<String> = sqlx::query_scalar("SELECT document::text FROM routing_template_versions WHERE template_id = $1 AND version = $2")
        .bind(template.template_id)
        .bind(version_to_i64(template.version)?)
        .fetch_optional(&mut **tx)
        .await?;
    let text = text.ok_or_else(|| {
        StoreError::Invalid(format!("the routing template `{}` has no published version {}", template.template_id, template.version))
    })?;
    read_template_document(&text)
}

fn read_template_document(text: &str) -> Result<RoutingTemplateVersion> {
    let document: RoutingTemplateVersion = serde_json::from_str(text).map_err(StoreError::codec)?;
    document.validate().map_err(StoreError::codec)?;
    Ok(document)
}

fn read_manufacturing_document(text: &str) -> Result<ManufacturingRecordDocument> {
    let document: ManufacturingRecordDocument = serde_json::from_str(text).map_err(StoreError::codec)?;
    document.validate().map_err(StoreError::codec)?;
    Ok(document)
}

async fn ensure_record_row(tx: &mut Transaction<'_, Postgres>, project_id: Uuid) -> Result<()> {
    sqlx::query("INSERT INTO manufacturing_records (project_id, current_version) VALUES ($1, 0) ON CONFLICT (project_id) DO NOTHING")
        .bind(project_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn append_document(tx: &mut Transaction<'_, Postgres>, project_id: Uuid, version: u64, document: &ManufacturingRecordDocument) -> Result<()> {
    let json = serde_json::to_string(document).map_err(StoreError::codec)?;
    sqlx::query("INSERT INTO manufacturing_record_versions (project_id, version, document) VALUES ($1, $2, $3::jsonb)")
        .bind(project_id)
        .bind(version_to_i64(version)?)
        .bind(json)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn advance_version(tx: &mut Transaction<'_, Postgres>, project_id: Uuid, version: u64) -> Result<()> {
    sqlx::query("UPDATE manufacturing_records SET current_version = $2 WHERE project_id = $1")
        .bind(project_id)
        .bind(version_to_i64(version)?)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

fn view(project_id: Uuid, version: u64, document: ManufacturingRecordDocument, project: &ProjectDocument) -> ManufacturingRecordView {
    let issues = document.issues(project);
    ManufacturingRecordView { project_id, version, document, issues }
}

fn ensure_part_version(project: &ProjectDocument, part_version_id: Uuid) -> Result<()> {
    if project.part_versions.iter().any(|part| part.part_version_id == part_version_id) {
        Ok(())
    } else {
        Err(StoreError::Invalid(format!("the project has no shared part version `{part_version_id}`")))
    }
}

fn ensure_product(project: &ProjectDocument, product_id: Uuid) -> Result<()> {
    if project.product(product_id).is_some() { Ok(()) } else { Err(StoreError::Invalid(format!("the project has no product `{product_id}`"))) }
}

/// Every shared part version and product a write names must exist in the current project record.
fn ensure_affected_refs(project: &ProjectDocument, part_versions: &[Uuid], products: &[Uuid]) -> Result<()> {
    for part_version_id in part_versions {
        ensure_part_version(project, *part_version_id)?;
    }
    for product_id in products {
        ensure_product(project, *product_id)?;
    }
    Ok(())
}

/// The affected references of a routing write must name the routing's owner and every named
/// reference must exist in the current project record.
fn validate_routing_refs(project: &ProjectDocument, owner: CostOwner, part_versions: &[Uuid], products: &[Uuid]) -> Result<()> {
    check_affected_owner(owner, part_versions, products, "routing.affected_refs").map_err(invalid)?;
    ensure_affected_refs(project, part_versions, products)
}

/// Every pinned production/setup rate version must exist, keep the purpose its field assumes and,
/// when the operation names a cost center, belong to that cost center. A missing pin is not an
/// error: it stays missing and becomes an issue on read.
fn validate_operation_rates(operations: &[RoutingOperation], rates: &[PinnedRate]) -> Result<()> {
    for operation in operations {
        if let Some(cycle) = &operation.cycle
            && let Some(rate_version_id) = cycle.production_rate_version_id
        {
            let rate = find_pinned_rate(rates, rate_version_id, "production")?;
            require_purpose(rate, CostCenterPurpose::Production, rate_version_id)?;
            require_cost_center(operation, rate, rate_version_id)?;
        }
        if let Some(setup) = &operation.setup
            && let Some(rate_version_id) = setup.setup_rate_version_id
        {
            let rate = find_pinned_rate(rates, rate_version_id, "setup")?;
            require_purpose(rate, CostCenterPurpose::Setup, rate_version_id)?;
            require_cost_center(operation, rate, rate_version_id)?;
        }
    }
    Ok(())
}

fn find_pinned_rate<'a>(rates: &'a [PinnedRate], rate_version_id: Uuid, what: &str) -> Result<&'a PinnedRate> {
    rates
        .iter()
        .find(|rate| rate.rate_version_id == rate_version_id)
        .ok_or_else(|| StoreError::Invalid(format!("the pinned {what} rate version `{rate_version_id}` is not a published reference version")))
}

fn require_purpose(rate: &PinnedRate, expected: CostCenterPurpose, rate_version_id: Uuid) -> Result<()> {
    if rate.purpose != expected {
        let what = match expected {
            CostCenterPurpose::Production => "production",
            CostCenterPurpose::Setup => "setup",
        };
        return Err(StoreError::Invalid(format!("the pinned {what} rate version `{rate_version_id}` has a different cost-center purpose")));
    }
    Ok(())
}

fn require_cost_center(operation: &RoutingOperation, rate: &PinnedRate, rate_version_id: Uuid) -> Result<()> {
    if let Some(cost_center_id) = operation.cost_center_id
        && cost_center_id != rate.cost_center_id
    {
        return Err(StoreError::Invalid(format!(
            "the pinned rate version `{rate_version_id}` belongs to a different cost center than operation `{}`",
            operation.operation_id
        )));
    }
    Ok(())
}

/// Adopt one exact published template version: create the routing when its id is new, or replace
/// the source link and the copied operations of an existing template-sourced routing. A manual
/// routing is never silently converted, and a same-version adoption is not a change.
fn adopt_template(
    document: &mut ManufacturingRecordDocument,
    project: &ProjectDocument,
    support: &WriteSupport,
    write: &AdoptRoutingTemplate,
) -> Result<Vec<ManufacturingFact>> {
    validate_routing_refs(project, write.owner, &write.affected_part_versions, &write.affected_products)?;
    let template =
        support.templates.first().ok_or_else(|| StoreError::Invalid("the adopted template version was not loaded inside the write".into()))?;
    if !template.applies_at(write.level) {
        return Err(StoreError::Invalid(format!("the routing template `{}` does not apply at the requested level", write.template.template_id)));
    }
    validate_operation_rates(&template.operations, &support.rates)?;
    let existing = document.routing(write.routing_id).map(|routing| (routing.owner, routing.level, routing.source));
    match existing {
        None => document.routings.push(AppliedRouting {
            routing_id: write.routing_id,
            level: write.level,
            owner: write.owner,
            source: RoutingSource::Template(write.template),
            operations: template.operations.clone(),
        }),
        Some((owner, level, source)) => {
            if owner != write.owner || level != write.level {
                return Err(StoreError::Invalid("adoption cannot change the owner or level of an existing applied routing".into()));
            }
            match source.template() {
                None => {
                    return Err(StoreError::Invalid(
                        "a manually entered applied routing cannot become template-sourced; create a routing by adoption to keep a template link"
                            .into(),
                    ));
                }
                Some(current) if current == write.template => {
                    return Err(StoreError::Invalid(
                        "the applied routing already adopts this template version; adoption is only the explicit change to a different version"
                            .into(),
                    ));
                }
                Some(_) => {}
            }
            let routing =
                document.routing_mut(write.routing_id).ok_or_else(|| StoreError::Invalid("the applied routing disappeared while adopting".into()))?;
            routing.source = RoutingSource::Template(write.template);
            routing.operations = template.operations.clone();
        }
    }
    Ok(vec![ManufacturingFact::RoutingWritten {
        routing_id: write.routing_id,
        part_version_id: write.owner.part_version_id(),
        product_id: write.owner.product_id(),
        template: Some(write.template),
    }])
}

/// Replace the operations of one existing applied routing. The owner, level and source link stay
/// exactly as they were, so editing a copy never edits its template or another instance.
fn edit_routing(
    document: &mut ManufacturingRecordDocument,
    project: &ProjectDocument,
    support: &WriteSupport,
    write: &EditRouting,
) -> Result<Vec<ManufacturingFact>> {
    let existing = document.routing(write.routing_id).map(|routing| (routing.owner, routing.source));
    let Some((owner, source)) = existing else {
        return Err(StoreError::Invalid(format!("the project has no applied routing `{}`", write.routing_id)));
    };
    validate_routing_refs(project, owner, &write.affected_part_versions, &write.affected_products)?;
    validate_operation_rates(&write.operations, &support.rates)?;
    let routing =
        document.routing_mut(write.routing_id).ok_or_else(|| StoreError::Invalid("the applied routing disappeared while editing".into()))?;
    routing.operations = write.operations.clone();
    Ok(vec![ManufacturingFact::RoutingWritten {
        routing_id: write.routing_id,
        part_version_id: owner.part_version_id(),
        product_id: owner.product_id(),
        template: source.template(),
    }])
}

/// Enter an applied routing without a template. A new routing id is required: an existing routing
/// is edited or adopted, never re-created, so its source kind cannot change silently.
fn manual_routing(
    document: &mut ManufacturingRecordDocument,
    project: &ProjectDocument,
    support: &WriteSupport,
    write: &ManualRouting,
) -> Result<Vec<ManufacturingFact>> {
    validate_routing_refs(project, write.owner, &write.affected_part_versions, &write.affected_products)?;
    validate_operation_rates(&write.operations, &support.rates)?;
    if document.routing(write.routing_id).is_some() {
        return Err(StoreError::Invalid(
            "the applied routing id already exists; an existing routing is edited or adopts another template version".into(),
        ));
    }
    document.routings.push(AppliedRouting {
        routing_id: write.routing_id,
        level: write.level,
        owner: write.owner,
        source: RoutingSource::Manual,
        operations: write.operations.clone(),
    });
    Ok(vec![ManufacturingFact::RoutingWritten {
        routing_id: write.routing_id,
        part_version_id: write.owner.part_version_id(),
        product_id: write.owner.product_id(),
        template: None,
    }])
}

/// Upsert and remove attached non-CAD cost lines. The line owns a part version or product root,
/// never a CAD identity, so the write changes no imported node or BOM structure.
fn apply_attached_costs(
    document: &mut ManufacturingRecordDocument,
    project: &ProjectDocument,
    request: &EditAttachedCosts,
) -> Result<Vec<ManufacturingFact>> {
    ensure_affected_refs(project, &request.affected_part_versions, &request.affected_products)?;
    for line in &request.upsert {
        if let Some(part_version_id) = line.owner.part_version_id() {
            ensure_part_version(project, part_version_id)?;
        }
        if let Some(product_id) = line.owner.product_id() {
            ensure_product(project, product_id)?;
        }
    }
    let removed: BTreeSet<Uuid> = request.remove.iter().copied().collect();
    for line_id in &request.remove {
        let line = document
            .attached_costs
            .iter()
            .find(|line| line.line_id == *line_id)
            .ok_or_else(|| StoreError::Invalid(format!("the project draft has no attached cost line `{line_id}`")))?;
        check_affected_owner(line.owner, &request.affected_part_versions, &request.affected_products, "edit_attached_costs.affected_refs")
            .map_err(invalid)?;
    }
    for line in &request.upsert {
        match document.attached_costs.iter_mut().find(|existing| existing.line_id == line.line_id) {
            Some(existing) => *existing = line.clone(),
            None => document.attached_costs.push(line.clone()),
        }
    }
    document.attached_costs.retain(|line| !removed.contains(&line.line_id));
    Ok(vec![ManufacturingFact::AttachedCostsWritten {
        written_line_ids: request.upsert.iter().map(|line| line.line_id).collect(),
        removed_line_ids: request.remove.clone(),
    }])
}
