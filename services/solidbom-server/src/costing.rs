//! Project costing settings and immutable baseline snapshots (package SB-16, additive to contract
//! version 2).
//!
//! The project document gains one additive optional `costing` field: the representative annual
//! volume policy, the saved manual exchange rates and the agreed purchase price tiers. An accepted
//! settings write is an ordinary append-only project record version (`Store::write_document`), so
//! editing the settings never rewrites an earlier version and never touches a saved baseline.
//!
//! [`Store::calculate_baseline`] reads the exact project, engineering and manufacturing record
//! versions and the published reference data inside one transaction, resolves the portable
//! [`solidbom_contract::BaselineInputs`], runs the pure calculator from `solidbom_costing` and
//! appends one immutable snapshot row. The snapshot retains the exact record versions, the
//! canonical input hash and the calculator version, so a historical answer stays reproducible after
//! a later edit. Issues are computed by the calculator on read; they are part of the stored output
//! because the immutable snapshot is the answer that was accepted.
//!
//! # Resolution rules of the service adapter
//!
//! These are deterministic and documented here because they turn stored records into the portable
//! inputs; the calculator never reads a record or a reference table itself.
//!
//! - `project_currency` is the project document currency; the volume policy, exchange rates and
//!   purchase tiers come from `document.costing` (`None` stays missing, never an implicit default).
//! - `category_config` is the latest published app cost configuration, or `None` when none exists.
//! - One part input exists for **every** project part version, so a part version without an explicit
//!   sourcing choice becomes a computed `missing_sourcing` issue instead of disappearing.
//! - A made part's material input keeps every missing piece missing: consumption, unit, density,
//!   price and allowance are copied from the manufacturing consumption and the selected immutable
//!   material version, and a version without a price keeps `None`.
//! - A purchased part's purchase input comes from the engineer-confirmed catalog reference (single
//!   price, its own currency and component version) plus the project's tier table for that part
//!   version (the tiers keep the table's own currency). It is never zero-substituted: with neither
//!   source the input carries a missing `single_price` and no tiers. Both sources keep their own
//!   currency, so the calculator converts only the one it selects. The purchase unit is `pcs`: the
//!   MVP prices one piece per occurrence.
//! - A routing operation's pinned production/setup rate version is resolved through the published
//!   rates to its hourly rate and currency; a missing version keeps `None`.
//! - Every quoted or excluded project product is an input. `quoted` is
//!   `quotation_state == included`; the schedule is the product's effective SOP–EOP; forecasts are
//!   copied as stored.
//! - The costed product revision is the one the request explicitly selects; a product without a
//!   selection uses its active revision, and no revision is ever read implicitly as "the latest".
//!   A quoted product with neither is left without a revision, which the calculator reports as a
//!   missing revision selection. An explicit selection that names a revision the product does not
//!   hold is refused. The occurrence tree comes from the newest registered exact analysis of the
//!   selected revision; no analysis means an empty occurrence list, which the calculator reports
//!   as a missing structure.
//! - An occurrence's part version is, first, the part version of the reviewed component (same
//!   product revision and analysis key) that owns its source path and carries one, and otherwise
//!   the project part version whose customer number and revision the manifest definition names,
//!   when exactly one matches. Everything else stays unresolved and the calculator reports it;
//!   children stay nested.

use std::collections::BTreeMap;

use solidbom_contract::{
    AppliedRouting, BaselineAttachedCostInput, BaselineCalculationList, BaselineCalculationSummary, BaselineCatalogPrice, BaselineCategoryConfig,
    BaselineCycleInput, BaselineInputs, BaselineMaterialInput, BaselineOccurrenceInput, BaselineOperationInput, BaselinePartInput,
    BaselineProductInput, BaselinePurchaseInput, BaselineSetupInput, BaselineSnapshot, CalculateBaseline, CostOwner, EngineeringRecordDocument,
    ManufacturingRecordDocument, MaterialConsumption, OccurrenceRef, Product, ProductRevision, ProductRevisionSelection, ProjectDocument,
    ProjectWriteOutcome, QuotationState, ReferenceData, ResultState, Sourcing, UpdateProjectCosting,
};
use sqlx::PgPool;
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

use crate::Result;
use crate::engineering::{RegisteredAnalysis, registered_analyses};
use crate::store::{RFC3339_UTC, Store, StoreError, check_request_id, lock_request, payload_hash, record_reply, recorded_reply, reference_data_in};

/// The prefix of the baseline calculation operations recorded in the shared request log. The
/// settings write uses the project-record prefix because it appends a project record version.
const OPERATION_PREFIX: &str = "baseline_calculation";

/// The most snapshots one history read returns. The response is bounded instead of trusting a
/// project to hold a client-sized list.
const MAX_BASELINE_HISTORY: usize = 100;

impl Store {
    /// Replace the project's costing settings: an expected-version-checked append-only project
    /// record version, validated against the project currency and part versions by
    /// `ProjectDocument::validate`. The request id makes an identical retry return the recorded
    /// outcome and a changed payload under the same id a conflict.
    pub async fn update_project_costing(&self, project_id: Uuid, request: &UpdateProjectCosting) -> Result<ProjectWriteOutcome> {
        request.validate().map_err(invalid)?;
        let payload = payload_hash("project_record.costing", &(project_id, request))?;
        self.write_document(project_id, &request.request_id, request.expected_version, &payload, "costing", |document| {
            document.apply_costing(request.costing.clone());
            Ok(Vec::new())
        })
        .await
    }

    /// Calculate one baseline from an exact project record version and append one immutable
    /// snapshot. The optional record versions default to the current engineering and manufacturing
    /// versions and are recorded either way. An identical retry returns the recorded snapshot before
    /// any version or record check; a changed payload under the same request id is a conflict.
    pub async fn calculate_baseline(&self, project_id: Uuid, request: &CalculateBaseline) -> Result<BaselineSnapshot> {
        request.validate().map_err(invalid)?;
        check_request_id(&request.request_id)?;
        let operation = format!("{OPERATION_PREFIX}.calculate");
        let payload = payload_hash(&operation, &(project_id, request))?;
        let mut tx = self.pool().begin().await?;
        lock_request(&mut tx, &request.request_id).await?;
        if let Some(snapshot) = recorded_reply::<BaselineSnapshot>(&mut tx, &request.request_id, &payload).await? {
            tx.commit().await?;
            return Ok(snapshot);
        }
        let project = project_version_document(&mut tx, project_id, request.project_version).await?;
        let (engineering_version, engineering) = engineering_record(&mut tx, project_id, request.engineering_version).await?;
        let (manufacturing_version, manufacturing) = manufacturing_record(&mut tx, project_id, request.manufacturing_version).await?;
        let analyses = registered_analyses(&mut tx, project_id).await?;
        let reference = reference_data_in(&mut tx).await?;
        let inputs = resolve_inputs(&project, &engineering, &analyses, &manufacturing, &reference, &request.product_revisions)?;
        // A broken resolved input is refused before the calculator runs: an invalid input never
        // becomes a stored invalid snapshot.
        inputs.validate().map_err(invalid)?;
        let input_hash = payload_hash("baseline_inputs", &inputs)?;
        let outputs = solidbom_costing::calculate_baseline(&inputs);
        let calculation_id = Uuid::new_v4();
        let created_at: String = sqlx::query_scalar(&format!(
            "INSERT INTO baseline_calculations \
             (calculation_id, project_id, project_version, engineering_version, manufacturing_version, input_hash, calculator_version, inputs, outputs) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8::jsonb, $9::jsonb) \
             RETURNING to_char(created_at AT TIME ZONE 'UTC', {RFC3339_UTC})"
        ))
        .bind(calculation_id)
        .bind(project_id)
        .bind(version_to_i64(request.project_version)?)
        .bind(version_to_i64(engineering_version)?)
        .bind(version_to_i64(manufacturing_version)?)
        .bind(&input_hash)
        .bind(solidbom_costing::baseline_calculator_version())
        .bind(serde_json::to_string(&inputs).map_err(StoreError::codec)?)
        .bind(serde_json::to_string(&outputs).map_err(StoreError::codec)?)
        .fetch_one(&mut *tx)
        .await?;
        let snapshot = BaselineSnapshot {
            calculation_id,
            project_id,
            project_version: request.project_version,
            engineering_version,
            manufacturing_version,
            input_hash,
            calculator_version: solidbom_costing::baseline_calculator_version().to_string(),
            inputs,
            outputs,
            created_at: Some(created_at),
        };
        record_reply(&mut tx, &request.request_id, &operation, &payload, &snapshot).await?;
        tx.commit().await?;
        Ok(snapshot)
    }

    /// The baseline history of one project, newest first and bounded.
    pub async fn baseline_calculations(&self, project_id: Uuid) -> Result<BaselineCalculationList> {
        ensure_project(self.pool(), project_id).await?;
        let limit = i64::try_from(MAX_BASELINE_HISTORY).map_err(StoreError::codec)?;
        let rows = sqlx::query(&format!(
            "SELECT calculation_id, project_id, project_version, engineering_version, manufacturing_version, calculator_version, \
             outputs->>'state' AS state, to_char(created_at AT TIME ZONE 'UTC', {RFC3339_UTC}) AS created_at \
             FROM baseline_calculations WHERE project_id = $1 ORDER BY created_at DESC, calculation_id DESC LIMIT $2"
        ))
        .bind(project_id)
        .bind(limit)
        .fetch_all(self.pool())
        .await?;
        let mut calculations = Vec::with_capacity(rows.len());
        for row in &rows {
            calculations.push(BaselineCalculationSummary {
                calculation_id: row.try_get("calculation_id")?,
                project_id: row.try_get("project_id")?,
                project_version: version_from_i64(row.try_get("project_version")?)?,
                engineering_version: version_from_i64(row.try_get("engineering_version")?)?,
                manufacturing_version: version_from_i64(row.try_get("manufacturing_version")?)?,
                calculator_version: row.try_get("calculator_version")?,
                state: result_state_from_db(row.try_get("state")?)?,
                created_at: Some(row.try_get("created_at")?),
            });
        }
        Ok(BaselineCalculationList { project_id, calculations })
    }

    /// One immutable baseline snapshot of one project. A calculation that does not exist, or that
    /// belongs to another project, is not found.
    pub async fn baseline_calculation(&self, project_id: Uuid, calculation_id: Uuid) -> Result<BaselineSnapshot> {
        let row = sqlx::query(&format!(
            "SELECT calculation_id, project_id, project_version, engineering_version, manufacturing_version, input_hash, calculator_version, \
             inputs::text AS inputs, outputs::text AS outputs, to_char(created_at AT TIME ZONE 'UTC', {RFC3339_UTC}) AS created_at \
             FROM baseline_calculations WHERE project_id = $1 AND calculation_id = $2"
        ))
        .bind(project_id)
        .bind(calculation_id)
        .fetch_optional(self.pool())
        .await?
        .ok_or(StoreError::NotFound)?;
        Ok(BaselineSnapshot {
            calculation_id: row.try_get("calculation_id")?,
            project_id: row.try_get("project_id")?,
            project_version: version_from_i64(row.try_get("project_version")?)?,
            engineering_version: version_from_i64(row.try_get("engineering_version")?)?,
            manufacturing_version: version_from_i64(row.try_get("manufacturing_version")?)?,
            input_hash: row.try_get("input_hash")?,
            calculator_version: row.try_get("calculator_version")?,
            inputs: serde_json::from_str(&row.try_get::<String, _>("inputs")?).map_err(StoreError::codec)?,
            outputs: serde_json::from_str(&row.try_get::<String, _>("outputs")?).map_err(StoreError::codec)?,
            created_at: Some(row.try_get("created_at")?),
        })
    }
}

/// The exact project record version to cost; a historical version is valid and an unknown one is
/// not found. Stored documents are validated on read, so a stored broken document stays an internal
/// codec failure instead of a client error.
async fn project_version_document(tx: &mut Transaction<'_, Postgres>, project_id: Uuid, version: u64) -> Result<ProjectDocument> {
    let text: Option<String> = sqlx::query_scalar("SELECT document::text FROM project_record_versions WHERE project_id = $1 AND version = $2")
        .bind(project_id)
        .bind(version_to_i64(version)?)
        .fetch_optional(&mut **tx)
        .await?;
    let document: ProjectDocument = serde_json::from_str(&text.ok_or(StoreError::NotFound)?).map_err(StoreError::codec)?;
    document.validate().map_err(StoreError::codec)?;
    Ok(document)
}

/// The engineering review record version to read: the requested exact version, or the current one.
/// A project without a review reads as version 0 with the empty record, exactly like the engineering
/// view.
async fn engineering_record(
    tx: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    requested: Option<u64>,
) -> Result<(u64, EngineeringRecordDocument)> {
    let current: Option<i64> = sqlx::query_scalar("SELECT current_version FROM engineering_records WHERE project_id = $1")
        .bind(project_id)
        .fetch_optional(&mut **tx)
        .await?;
    let current = match current {
        Some(current) => version_from_i64(current)?,
        None => 0,
    };
    let version = requested.unwrap_or(current);
    if version == 0 {
        return Ok((0, EngineeringRecordDocument::empty(project_id)));
    }
    let text: Option<String> = sqlx::query_scalar("SELECT document::text FROM engineering_record_versions WHERE project_id = $1 AND version = $2")
        .bind(project_id)
        .bind(version_to_i64(version)?)
        .fetch_optional(&mut **tx)
        .await?;
    let document: EngineeringRecordDocument = serde_json::from_str(&text.ok_or(StoreError::NotFound)?).map_err(StoreError::codec)?;
    document.validate().map_err(StoreError::codec)?;
    Ok((version, document))
}

/// The manufacturing record version to read: the requested exact version, or the current one. A
/// project without a manufacturing definition reads as version 0 with the empty record.
async fn manufacturing_record(
    tx: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    requested: Option<u64>,
) -> Result<(u64, ManufacturingRecordDocument)> {
    let current: Option<i64> = sqlx::query_scalar("SELECT current_version FROM manufacturing_records WHERE project_id = $1")
        .bind(project_id)
        .fetch_optional(&mut **tx)
        .await?;
    let current = match current {
        Some(current) => version_from_i64(current)?,
        None => 0,
    };
    let version = requested.unwrap_or(current);
    if version == 0 {
        return Ok((0, ManufacturingRecordDocument::empty(project_id)));
    }
    let text: Option<String> = sqlx::query_scalar("SELECT document::text FROM manufacturing_record_versions WHERE project_id = $1 AND version = $2")
        .bind(project_id)
        .bind(version_to_i64(version)?)
        .fetch_optional(&mut **tx)
        .await?;
    let document: ManufacturingRecordDocument = serde_json::from_str(&text.ok_or(StoreError::NotFound)?).map_err(StoreError::codec)?;
    document.validate().map_err(StoreError::codec)?;
    Ok((version, document))
}

/// Resolve the portable calculator inputs from the exact stored records and the published reference
/// data. This is the whole adapter: the calculator never sees a record, a currency table or the
/// clock.
fn resolve_inputs(
    project: &ProjectDocument,
    engineering: &EngineeringRecordDocument,
    analyses: &[RegisteredAnalysis],
    manufacturing: &ManufacturingRecordDocument,
    reference: &ReferenceData,
    product_revisions: &[ProductRevisionSelection],
) -> Result<BaselineInputs> {
    let costing = project.costing.as_ref();
    let parts = project
        .part_versions
        .iter()
        .map(|part_version| resolve_part(part_version.part_version_id, project, engineering, manufacturing, reference))
        .collect::<Result<Vec<_>>>()?;
    let products = project
        .products
        .iter()
        .map(|product| resolve_product(product, project, engineering, analyses, manufacturing, reference, product_revisions))
        .collect::<Result<Vec<_>>>()?;
    Ok(BaselineInputs {
        project_currency: project.currency.clone(),
        category_config: reference.app_config.as_ref().map(|config| BaselineCategoryConfig {
            config_version_id: config.config_version_id,
            currency: config.currency.clone(),
            categories: config.categories.clone(),
        }),
        volume_policy: costing.and_then(|costing| costing.volume_policy),
        exchange_rates: costing.map(|costing| costing.exchange_rates.clone()).unwrap_or_default(),
        parts,
        products,
    })
}

/// One shared part version as the calculator receives it. Everything the manufacturing record does
/// not define stays missing and becomes a computed issue.
fn resolve_part(
    part_version_id: Uuid,
    project: &ProjectDocument,
    engineering: &EngineeringRecordDocument,
    manufacturing: &ManufacturingRecordDocument,
    reference: &ReferenceData,
) -> Result<BaselinePartInput> {
    let sourced = manufacturing.part(part_version_id);
    let sourcing = sourced.map(|part| part.sourcing);
    let operations = resolve_operations(manufacturing.routing_of(CostOwner::PartVersion { part_version_id }), reference, &project.currency);
    let attached_costs = resolve_attached_costs(manufacturing, CostOwner::PartVersion { part_version_id });
    let (material, purchase) = match sourcing {
        Some(Sourcing::Manufactured) => {
            (sourced.and_then(|part| part.material.as_ref()).map(|consumption| resolve_material(consumption, reference, &project.currency)), None)
        }
        Some(Sourcing::Purchased) => (None, Some(resolve_purchase(project, engineering, part_version_id))),
        // No explicit sourcing choice: no material and no purchase price source, never a guess.
        None => (None, None),
    };
    Ok(BaselinePartInput { part_version_id, sourcing, material, operations, attached_costs, purchase })
}

/// One made part's material input: the recorded net consumption, unit and allowance plus the
/// selected immutable material version's density and price. A missing selection keeps every derived
/// value missing.
fn resolve_material(consumption: &MaterialConsumption, reference: &ReferenceData, project_currency: &str) -> BaselineMaterialInput {
    let material = consumption.material_version_id.and_then(|id| reference.material(id));
    BaselineMaterialInput {
        material_version_id: consumption.material_version_id,
        net_consumption: consumption.quantity,
        unit: consumption.unit,
        density_g_cm3: material.map(|material| material.density_g_cm3),
        price_per_kg: material.and_then(|material| material.price_per_kg),
        price_currency: material.map(|material| material.currency.clone()).unwrap_or_else(|| project_currency.to_string()),
        allowance_rate: consumption.allowance_rate,
    }
}

/// One purchased part's price source: the engineer-confirmed catalog single price (with its own
/// currency and component version) plus the project's agreed tier table for that part version (with
/// the table's own currency). Neither is zero-substituted; the calculator converts only the source
/// it selects, so the two currencies may differ.
fn resolve_purchase(project: &ProjectDocument, engineering: &EngineeringRecordDocument, part_version_id: Uuid) -> BaselinePurchaseInput {
    let confirmed = engineering.part(part_version_id).and_then(|part| part.catalog.confirmed.as_ref());
    let table = project.costing.as_ref().and_then(|costing| costing.purchase_table(part_version_id));
    let single_price = confirmed.and_then(|confirmed| {
        confirmed.price.map(|amount| BaselineCatalogPrice {
            amount,
            currency: confirmed.currency.clone(),
            component_version_id: confirmed.component_version_id,
        })
    });
    // The tier currency is the table's own currency. With no table there is no tier to convert, so
    // the confirmed single price's currency (else the project currency) keeps the field a valid code.
    let currency = table
        .map(|table| table.currency.clone())
        .unwrap_or_else(|| confirmed.map(|confirmed| confirmed.currency.clone()).unwrap_or_else(|| project.currency.clone()));
    let tiers = table.map(|table| table.tiers.clone()).unwrap_or_default();
    BaselinePurchaseInput { single_price, currency, price_unit: "pcs".into(), tiers }
}

/// The operations of one applied routing, with each pinned rate version resolved to its hourly rate
/// and currency. A missing pinned version keeps the rate missing.
fn resolve_operations(routing: Option<&AppliedRouting>, reference: &ReferenceData, project_currency: &str) -> Vec<BaselineOperationInput> {
    let Some(routing) = routing else { return Vec::new() };
    routing
        .operations
        .iter()
        .map(|operation| {
            let cycle = operation.cycle.as_ref().map(|cycle| {
                let rate = cycle.production_rate_version_id.and_then(|id| reference.rate(id));
                BaselineCycleInput {
                    production_rate_version_id: cycle.production_rate_version_id,
                    seconds: cycle.seconds,
                    basis: cycle.basis,
                    cavities: cycle.cavities,
                    rate_per_hour: rate.and_then(|rate| rate.rate_per_hour),
                    rate_currency: rate.map(|rate| rate.currency.clone()).unwrap_or_else(|| project_currency.to_string()),
                }
            });
            let setup = operation.setup.as_ref().map(|setup| {
                let rate = setup.setup_rate_version_id.and_then(|id| reference.rate(id));
                BaselineSetupInput {
                    setup_rate_version_id: setup.setup_rate_version_id,
                    hours: setup.hours,
                    rate_per_hour: rate.and_then(|rate| rate.rate_per_hour),
                    rate_currency: rate.map(|rate| rate.currency.clone()).unwrap_or_else(|| project_currency.to_string()),
                }
            });
            BaselineOperationInput { operation_id: operation.operation_id, sequence: operation.sequence, cycle, setup }
        })
        .collect()
}

/// The attached non-CAD cost lines an owner holds. Their unit cost is already in the project
/// currency, as the manufacturing contract states.
fn resolve_attached_costs(manufacturing: &ManufacturingRecordDocument, owner: CostOwner) -> Vec<BaselineAttachedCostInput> {
    manufacturing
        .attached_costs_of(owner)
        .map(|line| BaselineAttachedCostInput {
            line_id: line.line_id,
            category: line.category.clone(),
            quantity: line.quantity,
            unit_cost: line.unit_cost,
        })
        .collect()
}

/// One project product as the calculator receives it: quoted flag, effective schedule, stored
/// forecasts, product-root operations and attached lines, and the resolved occurrence tree of the
/// revision whose structure is costed.
fn resolve_product(
    product: &Product,
    project: &ProjectDocument,
    engineering: &EngineeringRecordDocument,
    analyses: &[RegisteredAnalysis],
    manufacturing: &ManufacturingRecordDocument,
    reference: &ReferenceData,
    product_revisions: &[ProductRevisionSelection],
) -> Result<BaselineProductInput> {
    let revision = selected_revision(product, product_revisions)?;
    let occurrences = match revision {
        Some(revision) => resolve_occurrences(project, engineering, analyses, revision)?,
        None => Vec::new(),
    };
    Ok(BaselineProductInput {
        product_id: product.product_id,
        product_revision_id: revision.map(|revision| revision.product_revision_id),
        quoted: product.quotation_state == QuotationState::Included,
        schedule: product.schedule(project.default_schedule),
        forecasts: product.forecasts.clone(),
        root_operations: resolve_operations(
            manufacturing.routing_of(CostOwner::ProductRoot { product_id: product.product_id }),
            reference,
            &project.currency,
        ),
        root_attached_costs: resolve_attached_costs(manufacturing, CostOwner::ProductRoot { product_id: product.product_id }),
        occurrences,
    })
}

/// The revision whose structure is costed: the request's explicit selection for the product when it
/// names one, otherwise the product's active revision. The latest draft is never read implicitly, so
/// a product without either stays without a revision and the calculator reports the missing
/// selection. An explicit selection of a revision the product does not hold is refused.
fn selected_revision<'a>(product: &'a Product, selections: &[ProductRevisionSelection]) -> Result<Option<&'a ProductRevision>> {
    if let Some(selection) = selections.iter().find(|selection| selection.product_id == product.product_id) {
        return product.revisions.iter().find(|revision| revision.product_revision_id == selection.product_revision_id).map(Some).ok_or_else(|| {
            StoreError::Invalid(format!("the product `{}` has no product revision `{}`", product.product_id, selection.product_revision_id))
        });
    }
    Ok(product
        .active_revision_id
        .and_then(|active_revision_id| product.revisions.iter().find(|revision| revision.product_revision_id == active_revision_id)))
}

/// The resolved occurrence forest of one product revision. The registered analyses are ordered by
/// registration time, so the newest exact analysis of the revision is the structure the review is
/// bound to; a revision with no analysis has no occurrences, which the calculator reports as a
/// missing structure rather than inventing a tree.
fn resolve_occurrences(
    project: &ProjectDocument,
    engineering: &EngineeringRecordDocument,
    analyses: &[RegisteredAnalysis],
    revision: &ProductRevision,
) -> Result<Vec<BaselineOccurrenceInput>> {
    let Some(registered) = analyses.iter().rfind(|registered| registered.product_revision_id == revision.product_revision_id) else {
        return Ok(Vec::new());
    };
    let manifest = &registered.manifest;
    let mut children: BTreeMap<Option<Uuid>, Vec<&OccurrenceRef>> = BTreeMap::new();
    for occurrence in &manifest.occurrences {
        children.entry(occurrence.parent_occurrence_id).or_default().push(occurrence);
    }
    let Some(roots) = children.get(&None) else {
        return Ok(Vec::new());
    };
    roots.iter().map(|occurrence| build_occurrence(project, engineering, manifest, revision.product_revision_id, occurrence, &children, 0)).collect()
}

/// One occurrence node with its resolved part version and its nested children, in manifest order.
fn build_occurrence(
    project: &ProjectDocument,
    engineering: &EngineeringRecordDocument,
    manifest: &solidbom_contract::AnalysisManifest,
    product_revision_id: Uuid,
    occurrence: &OccurrenceRef,
    children: &BTreeMap<Option<Uuid>, Vec<&OccurrenceRef>>,
    depth: usize,
) -> Result<BaselineOccurrenceInput> {
    // The portable structure has a bounded depth, and the recursion that builds it must respect
    // the same bound *while building*: a hostile analysis manifest would otherwise overflow the
    // stack before the portable-input validation ever runs.
    if depth >= solidbom_contract::MAX_STRUCTURE_DEPTH {
        return Err(StoreError::Invalid(format!(
            "the registered analysis structure is deeper than the supported {} levels",
            solidbom_contract::MAX_STRUCTURE_DEPTH
        )));
    }
    let nested = match children.get(&Some(occurrence.occurrence_id)) {
        Some(nested) => {
            let mut built = Vec::with_capacity(nested.len());
            for child in nested {
                built.push(build_occurrence(project, engineering, manifest, product_revision_id, child, children, depth + 1)?);
            }
            built
        }
        None => Vec::new(),
    };
    Ok(BaselineOccurrenceInput {
        occurrence_id: Some(occurrence.occurrence_id),
        part_version_id: resolve_occurrence_part(project, engineering, manifest, product_revision_id, occurrence),
        children: nested,
    })
}

/// The shared part version one occurrence places. A reviewed component of the **selected product
/// revision** and exact analysis that owns the occurrence's source path takes precedence; otherwise
/// the manifest definition's customer number and revision must match exactly one project part
/// version. Anything else stays unresolved.
///
/// The revision filter matters: one analysis key may be registered for more than one revision (the
/// stored analysis identity includes the revision), and two revisions can share identical source
/// geometry. Matching components without the revision filter would let one revision borrow the
/// other revision's association and produce wrong demand and costs.
fn resolve_occurrence_part(
    project: &ProjectDocument,
    engineering: &EngineeringRecordDocument,
    manifest: &solidbom_contract::AnalysisManifest,
    product_revision_id: Uuid,
    occurrence: &OccurrenceRef,
) -> Option<Uuid> {
    let component = engineering
        .components_of(product_revision_id, &manifest.key)
        .find(|component| component.geometry.iter().any(|geometry| geometry.source_path == occurrence.source_path));
    if let Some(part_version_id) = component.and_then(|component| component.part_version_id) {
        return Some(part_version_id);
    }
    let definition = manifest.definitions.iter().find(|definition| definition.definition_id == occurrence.definition_id)?;
    let (number, revision) = match (&definition.customer_part_number, &definition.customer_revision) {
        (Some(number), Some(revision)) if !number.trim().is_empty() && !revision.trim().is_empty() => (number.trim(), revision.trim()),
        _ => return None,
    };
    let matches: Vec<Uuid> = project
        .part_versions
        .iter()
        .filter(|part_version| part_version.customer_part_number.trim() == number && part_version.customer_revision.trim() == revision)
        .map(|part_version| part_version.part_version_id)
        .collect();
    match matches.as_slice() {
        [only] => Some(*only),
        _ => None,
    }
}

/// Refuse an unknown project for a read that otherwise has an empty result.
async fn ensure_project(pool: &PgPool, project_id: Uuid) -> Result<()> {
    let found: Option<Uuid> =
        sqlx::query_scalar("SELECT project_id FROM project_records WHERE project_id = $1").bind(project_id).fetch_optional(pool).await?;
    found.map(|_| ()).ok_or(StoreError::NotFound)
}

fn invalid(error: solidbom_contract::ContractError) -> StoreError {
    StoreError::Invalid(error.to_string())
}

fn version_to_i64(version: u64) -> Result<i64> {
    i64::try_from(version).map_err(StoreError::codec)
}

fn version_from_i64(version: i64) -> Result<u64> {
    u64::try_from(version).map_err(StoreError::codec)
}

/// The wire `state` of a stored outcome, read for the compact history row.
fn result_state_from_db(state: String) -> Result<ResultState> {
    match state.as_str() {
        "complete" => Ok(ResultState::Complete),
        "incomplete" => Ok(ResultState::Incomplete),
        other => Err(StoreError::Codec(format!("the stored baseline state `{other}` is unknown"))),
    }
}
