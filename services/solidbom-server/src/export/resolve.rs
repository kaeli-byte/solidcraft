//! Resolving one selected saved result into the shared export model (package SB-21).
//!
//! The projection reads the exact records the selected result pins and copies their stored values
//! into [`super::model::ExportModel`]. It performs no arithmetic of its own: every cost, demand and
//! price comes from the immutable SB-16 snapshot output. The tooling recovery and the commercial
//! scenario prices/EBIT are the deterministic pure SB-17/SB-18 evaluations of that same pinned
//! snapshot and project version, not a recomputation from current master data, and the renderers
//! draw only the resulting model.
//!
//! The accepted-baseline selection preserves the accepted history: its project, engineering,
//! manufacturing and feasibility record versions are the ones activation pinned. A draft selection
//! is the project's current draft result and carries the SB-20 stale statement; its feasibility
//! record is read at export time because no costing snapshot pins one.
//!
//! # Rules of the projection
//!
//! - **One selected saved result, never "the newest".** An accepted baseline is read by the exact
//!   pair `(accepted_baseline_id, sequence)`; a sequence mismatch is invalid. A draft selection must
//!   name the newest `baseline_calculations` row that no accepted baseline pins: an older (or
//!   accepted) snapshot is a conflict, exactly like an expired client read.
//! - **The snapshot is preserved.** The project document, the engineering record and the
//!   feasibility record are read at the exact versions the snapshot (or the accepted baseline)
//!   recorded, in one repeatable-read transaction. The only value read at export time is the
//!   feasibility record of a draft result, and the metadata labels it as unpinned.
//! - **The freshness labels follow the stored result, they are never invented.** A draft is
//!   labelled `draft_result` and `unpinned_feasibility_record`; an incomplete result, a listed
//!   missing input, a non-passing (or review-required-exception) feasibility finding, a
//!   costing-relevant shared-data change and a stale compared scenario each add their own label.
//!   An accepted baseline is labelled `superseded_accepted_baseline` when a newer acceptance heads
//!   the project. The labels are sorted by the contract's canonical order and deduplicated.
//! - **The bundle describes exactly the bytes it carries**: the rendered media type, byte length
//!   and SHA-256 digest are computed here, and the bundle is validated against its metadata before
//!   it is returned.
//! - **An export is a pure read.** Nothing is appended: no project version, record version,
//!   snapshot, accepted baseline, decision or request receipt. The comparison of saved commercial
//!   scenarios is the SB-18 read-only evaluation of the same snapshot.

use sha2::{Digest, Sha256};
use solidbom_contract::{
    BaselineOperationInput, BaselineSnapshot, CommercialScenarioComparison, CompareCommercialScenarios, DraftStaleReason, EngineeringRecordDocument,
    ExceptionState, ExportBaselineRef, ExportBaselineSelection, ExportBundle, ExportCalculation, ExportFormat, ExportLabel, ExportMetadata,
    ExportRecordVersions, ExportRequest, ExportStaleReason, FeasibilityRecordDocument, FeasibilityRecordView, FindingDisposition, FindingState,
    ProductKind, ProjectDocument, QuotationState, ResultState, ReviewState, ToolingOutcome, ToolingRecoveryInputs,
};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::Result;
use crate::costing::{baseline_snapshot_in, engineering_record, manufacturing_record, project_version_document};
use crate::feasibility::current_feasibility_record;
use crate::store::{RFC3339_UTC, Store, StoreError};
use crate::tooling::resolve_tooling_inputs;

use super::model::{
    ExportFeasibility, ExportIssue, ExportIssueSource, ExportModel, ExportOperation, ExportPart, ExportProduct, ExportScenario, ExportScope,
    ExportScopedProduct,
};

/// The conflict message of a draft selection that is not the project's current draft result.
const NOT_CURRENT_DRAFT: &str =
    "the selected calculation is not the project's current draft result; export the current draft result or an accepted baseline";

/// The longest project-name stem the suggested file name keeps, so the name stays a base name of at
/// most [`solidbom_contract::MAX_EXPORT_FILE_NAME_CHARS`] characters with its suffix and extension.
const FILE_NAME_BASE_MAX_CHARS: usize = 72;

/// One selected saved result, as the projection read it before the model is built.
struct SelectedResult {
    snapshot: BaselineSnapshot,
    versions: ExportRecordVersions,
    feasibility: ExportFeasibility,
    baseline: ExportBaselineSelection,
    superseded: bool,
}

impl Store {
    /// Render one internal export of one explicitly selected saved result of one project.
    ///
    /// This is a pure read: it appends nothing and stores no rendered artifact. The selected result
    /// is resolved once into [`ExportModel`], the model is rendered by the format's renderer, and the
    /// answer carries the metadata the documents record together with the rendered bytes, their
    /// media type, length and SHA-256 digest.
    ///
    /// An unknown project, accepted baseline or draft calculation is `not found`; a draft selection
    /// that is not the project's current draft result, and an accepted baseline whose sequence does
    /// not match the stored row, are refused.
    pub async fn export_bundle(&self, project_id: Uuid, request: &ExportRequest) -> Result<ExportBundle> {
        request.validate().map_err(invalid)?;
        if request.project_id != project_id {
            return Err(StoreError::Invalid("the export request names another project than the route".into()));
        }
        let model = self.resolve_export_model(project_id, request).await?;
        let file_name = suggested_file_name(&model.scope.project_name, request.format, model.metadata.baseline);
        let bytes = super::render(&model, request.format);
        let byte_len = u64::try_from(bytes.len()).map_err(StoreError::codec)?;
        let sha256 = hex::encode(Sha256::digest(&bytes));
        let bundle =
            ExportBundle { metadata: model.metadata, file_name, media_type: request.format.media_type().to_string(), byte_len, sha256, bytes };
        // A bundle that disagrees with its own metadata is a service defect, not a client error.
        bundle.validate().map_err(StoreError::codec)?;
        Ok(bundle)
    }

    /// Resolve one selected saved result into the renderer-neutral model. The pinned reads happen in
    /// one repeatable-read transaction; the renderers run afterwards and read nothing.
    async fn resolve_export_model(&self, project_id: Uuid, request: &ExportRequest) -> Result<ExportModel> {
        // The accepted baseline row is immutable and its pair check is a pure read, so it is fetched
        // before the snapshot transaction opens; the transaction then reads the same immutable row
        // contents and cannot observe a different accepted state.
        let accepted = match request.baseline {
            ExportBaselineRef::AcceptedBaseline { accepted_baseline_id, sequence } => {
                let baseline = self.accepted_baseline(project_id, accepted_baseline_id).await?;
                if baseline.sequence != sequence {
                    return Err(StoreError::Invalid(format!(
                        "the accepted baseline `{accepted_baseline_id}` is at sequence {}, not {sequence}",
                        baseline.sequence
                    )));
                }
                Some(baseline)
            }
            ExportBaselineRef::DraftCalculation { .. } => None,
        };

        let mut tx = self.pool().begin().await?;
        // One consistent view of the records the selected result pinned: a concurrent write must not
        // mix two states into one document. `now()` is the transaction's start time, so the
        // `exported_at` timestamp belongs to the same read.
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ").execute(&mut *tx).await?;
        let exported_at: String = sqlx::query_scalar(&format!("SELECT to_char(now() AT TIME ZONE 'UTC', {RFC3339_UTC})")).fetch_one(&mut *tx).await?;

        let (snapshot, project_version, engineering_version, manufacturing_version, feasibility_pinned, feasibility_version) =
            if let Some(baseline) = &accepted {
                let snapshot = baseline_snapshot_in(&mut tx, project_id, baseline.costing_calculation_id).await?;
                (
                    snapshot,
                    baseline.pinned.project_version,
                    baseline.pinned.engineering_version,
                    baseline.pinned.manufacturing_version,
                    true,
                    baseline.pinned.feasibility_version,
                )
            } else {
                let ExportBaselineRef::DraftCalculation { calculation_id } = request.baseline else {
                    return Err(StoreError::codec("the export selection is missing"));
                };
                let snapshot = baseline_snapshot_in(&mut tx, project_id, calculation_id).await?;
                // A draft export names the current draft result: the newest immutable snapshot that
                // no accepted baseline pins. An earlier (or accepted) snapshot is refused like an
                // expired client read; exporting an accepted baseline is the alternative.
                let newest: Option<Uuid> = sqlx::query_scalar(
                    "SELECT bc.calculation_id FROM baseline_calculations bc \
                     WHERE bc.project_id = $1 \
                     AND NOT EXISTS (SELECT 1 FROM accepted_baselines ab \
                                     WHERE ab.project_id = bc.project_id AND ab.costing_calculation_id = bc.calculation_id) \
                     ORDER BY bc.created_at DESC, bc.calculation_id DESC LIMIT 1",
                )
                .bind(project_id)
                .fetch_optional(&mut *tx)
                .await?;
                if newest != Some(snapshot.calculation_id) {
                    return Err(StoreError::Conflict(NOT_CURRENT_DRAFT.into()));
                }
                let project_version = snapshot.project_version;
                let engineering_version = snapshot.engineering_version;
                let manufacturing_version = snapshot.manufacturing_version;
                (snapshot, project_version, engineering_version, manufacturing_version, false, 0)
            };

        // The exact project record version the snapshot read, and the exact engineering and
        // manufacturing versions it recorded. Version 0 is the empty record.
        let project = project_version_document(&mut tx, project_id, project_version).await?;
        let (_, engineering) = engineering_record(&mut tx, project_id, Some(engineering_version)).await?;
        // The manufacturing record is loaded and validated so a pinned version this build cannot read
        // refuses the export; every manufacturing value the documents print already comes from the
        // stored snapshot inputs.
        let (_manufacturing_version, _manufacturing) = manufacturing_record(&mut tx, project_id, Some(manufacturing_version)).await?;
        let (feasibility_version, feasibility_document) = if feasibility_pinned {
            if feasibility_version == 0 {
                (0, FeasibilityRecordDocument::empty(project_id))
            } else {
                (feasibility_version, feasibility_record_document_in(&mut tx, project_id, feasibility_version).await?)
            }
        } else {
            // No costing snapshot pins a feasibility record, so a draft reads the current one. Its
            // version is recorded and the documents label it as read at export time.
            current_feasibility_record(&mut tx, project_id).await?
        };
        let feasibility = ExportFeasibility {
            version: feasibility_version,
            pinned: feasibility_pinned,
            view: feasibility_document.record_view(feasibility_version, &engineering),
        };
        // An accepted baseline is accepted history, so stale markers never apply to it. A newer
        // acceptance is a label instead of a freshness statement.
        let superseded = match &accepted {
            Some(baseline) => {
                let newest_sequence: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(sequence), 0) FROM accepted_baselines WHERE project_id = $1")
                    .bind(project_id)
                    .fetch_one(&mut *tx)
                    .await?;
                u64::try_from(newest_sequence).map_err(StoreError::codec)? > baseline.sequence
            }
            None => false,
        };
        let baseline_selection = match &accepted {
            Some(baseline) => {
                ExportBaselineSelection::AcceptedBaseline { accepted_baseline_id: baseline.accepted_baseline_id, sequence: baseline.sequence }
            }
            None => ExportBaselineSelection::DraftCalculation { calculation_id: snapshot.calculation_id },
        };
        let versions = ExportRecordVersions {
            project_version,
            engineering_version,
            manufacturing_version,
            feasibility_version: feasibility.version,
            feasibility_pinned: feasibility.pinned,
        };
        let result = SelectedResult { snapshot, versions, feasibility, baseline: baseline_selection, superseded };
        tx.commit().await?;
        self.project_export_model(project_id, request, exported_at, project, engineering, result).await
    }

    /// Evaluate the parts of the model that need another read (the SB-17 tooling evaluation and the
    /// SB-18 scenario comparison) and build the model. Both reads are pure and read only the same
    /// immutable snapshot and project version, so the answer stays consistent with the pinned read.
    async fn project_export_model(
        &self,
        project_id: Uuid,
        request: &ExportRequest,
        exported_at: String,
        project: ProjectDocument,
        engineering: EngineeringRecordDocument,
        result: SelectedResult,
    ) -> Result<ExportModel> {
        let SelectedResult { snapshot, versions, feasibility, baseline, superseded } = result;
        // A draft result carries the SB-20 stale statement of the same project state; an accepted
        // baseline is accepted history and never stale.
        let (stale, stale_reasons) = match baseline {
            ExportBaselineSelection::DraftCalculation { calculation_id } => {
                let report = self.project_dependency_report(project_id).await?;
                let draft = report.draft.ok_or_else(|| StoreError::Conflict(NOT_CURRENT_DRAFT.into()))?;
                if draft.calculation_id != calculation_id {
                    return Err(StoreError::Conflict(NOT_CURRENT_DRAFT.into()));
                }
                let mut reasons: Vec<ExportStaleReason> = draft.stale_reasons.iter().map(|reason| export_stale_reason(*reason)).collect();
                reasons.sort_by_key(|reason| reason.as_str());
                reasons.dedup();
                if draft.stale != !reasons.is_empty() {
                    return Err(StoreError::codec("the stored draft freshness statement is inconsistent"));
                }
                (draft.stale, reasons)
            }
            ExportBaselineSelection::AcceptedBaseline { .. } => (false, Vec::new()),
        };
        // The tooling investment, allocation and recovery of the same selected result: the SB-17
        // evaluation of the immutable snapshot, never a second investment calculation.
        let tooling_inputs = resolve_tooling_inputs(&project, &snapshot, &ToolingRecoveryInputs::default());
        tooling_inputs.validate().map_err(invalid)?;
        let tooling = solidbom_costing::calculate_tooling(&tooling_inputs);
        // The saved commercial scenarios, evaluated against the same immutable snapshot. The
        // comparison is a read-only SB-18 evaluation and labels a scenario that pinned another
        // result, project version or setting.
        let comparison = if request.scenarios.is_empty() {
            None
        } else {
            Some(
                self.compare_commercial_scenarios(
                    project_id,
                    &CompareCommercialScenarios { baseline_calculation_id: snapshot.calculation_id, scenario_ids: request.scenarios.clone() },
                )
                .await?,
            )
        };
        Ok(build_export_model(
            project_id,
            request.format,
            exported_at,
            &snapshot,
            &project,
            &engineering,
            feasibility,
            baseline,
            versions,
            stale,
            stale_reasons,
            superseded,
            tooling,
            comparison.as_ref(),
        ))
    }
}

/// Build the model from values that are already loaded. This is a pure projection: it copies stored
/// values, orders rows and derives the labels; it never recomputes a cost.
#[allow(clippy::too_many_arguments)]
fn build_export_model(
    project_id: Uuid,
    format: ExportFormat,
    exported_at: String,
    snapshot: &BaselineSnapshot,
    project: &ProjectDocument,
    engineering: &EngineeringRecordDocument,
    feasibility: ExportFeasibility,
    baseline: ExportBaselineSelection,
    versions: ExportRecordVersions,
    stale: bool,
    stale_reasons: Vec<ExportStaleReason>,
    superseded: bool,
    tooling: ToolingOutcome,
    comparison: Option<&CommercialScenarioComparison>,
) -> ExportModel {
    let scope = export_scope(snapshot, project);
    let parts = export_parts(snapshot, project, engineering);
    let products = export_products(snapshot, project);
    let scenarios = export_scenarios(comparison);
    let issues = export_issues(snapshot, &tooling, comparison, &feasibility.view, &stale_reasons);
    let mut labels = Vec::new();
    if matches!(baseline, ExportBaselineSelection::DraftCalculation { .. }) {
        labels.push(ExportLabel::DraftResult);
    }
    if snapshot.outputs.state == ResultState::Incomplete {
        labels.push(ExportLabel::IncompleteResult);
    }
    if issues.iter().any(|issue| missing_input_code(&issue.code)) {
        labels.push(ExportLabel::MissingInputs);
    }
    if has_unresolved_findings(&feasibility.view) {
        labels.push(ExportLabel::UnresolvedFindings);
    }
    // Any stale draft result is not current, so its costs are labelled outdated. The named reason
    // says what moved (a record generation or costing-relevant shared data); the label itself is
    // the general warning the contract requires on a stale result.
    if stale {
        labels.push(ExportLabel::OutdatedCosts);
    }
    if superseded {
        labels.push(ExportLabel::SupersededAcceptedBaseline);
    }
    if scenarios.iter().any(|scenario| scenario.stale) {
        labels.push(ExportLabel::StaleScenarios);
    }
    if !feasibility.pinned {
        labels.push(ExportLabel::UnpinnedFeasibilityRecord);
    }
    // `ExportLabel` derives `Ord` in the contract's canonical order, so sorting and deduplicating
    // yields exactly the order the bundle's validation requires.
    labels.sort();
    labels.dedup();
    let metadata = ExportMetadata {
        project_id,
        project_name: project.name.clone(),
        customer: project.customer.clone(),
        currency: project.currency.clone(),
        exported_at,
        format,
        baseline,
        versions,
        calculation: ExportCalculation {
            calculation_id: snapshot.calculation_id,
            calculator_version: snapshot.calculator_version.clone(),
            input_hash: snapshot.input_hash.clone(),
            state: snapshot.outputs.state,
        },
        labels,
        stale,
        stale_reasons,
    };
    ExportModel {
        metadata,
        scope,
        parts,
        products,
        setup: snapshot.outputs.setup.clone(),
        setup_categories: snapshot.outputs.setup_categories.clone(),
        setup_total: snapshot.outputs.setup_total,
        annual: snapshot.outputs.annual.clone(),
        project_total_manufacturing_cost: snapshot.outputs.project_total_manufacturing_cost,
        tooling: Some(tooling),
        scenarios,
        feasibility: Some(feasibility),
        issues,
    }
}

/// The project and product scope the selected result covers: every product the snapshot resolved, in
/// product-id order, named from the pinned project document.
fn export_scope(snapshot: &BaselineSnapshot, project: &ProjectDocument) -> ExportScope {
    let mut products: Vec<ExportScopedProduct> = Vec::with_capacity(snapshot.inputs.products.len());
    for input in &snapshot.inputs.products {
        let product = project.products.iter().find(|product| product.product_id == input.product_id);
        products.push(ExportScopedProduct {
            product_id: input.product_id,
            name: product.map(|product| product.name.clone()).unwrap_or_default(),
            kind: product.map(|product| product.kind).unwrap_or(ProductKind::Part),
            customer_part_number: product.and_then(|product| product.customer_part_number.clone()),
            quotation_state: product.map(|product| product.quotation_state).unwrap_or(QuotationState::Excluded),
            product_revision_id: input.product_revision_id,
            revision_state: product.and_then(|product| {
                input
                    .product_revision_id
                    .and_then(|revision_id| product.revisions.iter().find(|revision| revision.product_revision_id == revision_id))
                    .map(|revision| revision.state)
            }),
            schedule: product.map(|product| product.schedule(project.default_schedule)).unwrap_or(input.schedule),
            forecasts: product.map(|product| product.forecasts.clone()).unwrap_or_else(|| input.forecasts.clone()),
        });
    }
    products.sort_by_key(|product| product.product_id);
    ExportScope {
        project_name: project.name.clone(),
        customer: project.customer.clone(),
        internal_team: project.internal_team.clone(),
        platform: project.platform.clone(),
        application: project.application.clone(),
        currency: project.currency.clone(),
        default_schedule: project.default_schedule,
        products,
    }
}

/// One row per shared part version in the selected scope, in part-version-id order. Identity comes
/// from the pinned project record, the engineer-confirmed review from the pinned engineering record,
/// the resolved manufacturing definition from the snapshot inputs and the demand and rolled costs
/// from the stored snapshot outputs.
fn export_parts(snapshot: &BaselineSnapshot, project: &ProjectDocument, engineering: &EngineeringRecordDocument) -> Vec<ExportPart> {
    let mut parts: Vec<ExportPart> = Vec::with_capacity(snapshot.inputs.parts.len());
    for input in &snapshot.inputs.parts {
        let identity = project.part_versions.iter().find(|part| part.part_version_id == input.part_version_id);
        let reviewed = engineering.part(input.part_version_id);
        let line = snapshot.outputs.parts.iter().find(|part| part.part_version_id == input.part_version_id);
        parts.push(ExportPart {
            part_version_id: input.part_version_id,
            customer_part_number: identity.map(|part| part.customer_part_number.clone()).unwrap_or_default(),
            customer_revision: identity.map(|part| part.customer_revision.clone()).unwrap_or_default(),
            created_in_package_id: identity.map(|part| part.created_in_package_id).unwrap_or(Uuid::nil()),
            classification: reviewed.and_then(|part| part.classification.confirmed.as_ref()).map(|confirmed| confirmed.name.clone()),
            classification_confirmed: reviewed.is_some_and(|part| part.classification.is_confirmed()),
            reviewed_volume_mm3: reviewed
                .and_then(|part| part.volume_mm3.as_ref())
                .filter(|volume| volume.review == ReviewState::Confirmed)
                .and_then(|volume| volume.value),
            sourcing: input.sourcing,
            material: line.and_then(|line| line.material.clone()),
            purchase: line.and_then(|line| line.purchase_price.clone()),
            attached_costs: input.attached_costs.clone(),
            operations: input.operations.iter().map(export_operation).collect(),
            annual_demand: line.map(|line| line.annual_demand.clone()).unwrap_or_default(),
            lifetime_demand: line.and_then(|line| line.lifetime_demand),
            peak_demand: line.and_then(|line| line.peak_demand),
            costing_volume: line.and_then(|line| line.costing_volume),
            material_cost: line.and_then(|line| line.material_cost),
            production_cost: line.and_then(|line| line.production_cost),
            attached_cost: line.and_then(|line| line.attached_cost),
            unit_cost: line.and_then(|line| line.unit_cost),
            setup_charge: line.and_then(|line| line.setup_charge),
        });
    }
    parts.sort_by_key(|part| part.part_version_id);
    parts
}

/// One row per product the selected result costs, in product-id order. Product identity and stored
/// schedule come from the pinned project document; the expanded part quantities, categories and
/// cost values come from the stored snapshot outputs.
fn export_products(snapshot: &BaselineSnapshot, project: &ProjectDocument) -> Vec<ExportProduct> {
    let mut products: Vec<ExportProduct> = Vec::with_capacity(snapshot.outputs.products.len());
    for line in &snapshot.outputs.products {
        let input = snapshot.inputs.products.iter().find(|product| product.product_id == line.product_id);
        let product = project.products.iter().find(|product| product.product_id == line.product_id);
        products.push(ExportProduct {
            product_id: line.product_id,
            name: product.map(|product| product.name.clone()).unwrap_or_default(),
            kind: product.map(|product| product.kind).unwrap_or(ProductKind::Part),
            customer_part_number: product.and_then(|product| product.customer_part_number.clone()),
            quotation_state: product.map(|product| product.quotation_state).unwrap_or(QuotationState::Excluded),
            product_revision_id: line.product_revision_id,
            revision_state: product.and_then(|product| {
                line.product_revision_id
                    .and_then(|revision_id| product.revisions.iter().find(|revision| revision.product_revision_id == revision_id))
                    .map(|revision| revision.state)
            }),
            schedule: product
                .map(|product| product.schedule(project.default_schedule))
                .unwrap_or_else(|| input.map(|input| input.schedule).unwrap_or(project.default_schedule)),
            forecasts: product
                .map(|product| product.forecasts.clone())
                .unwrap_or_else(|| input.map(|input| input.forecasts.clone()).unwrap_or_default()),
            quoted: input.map(|input| input.quoted).unwrap_or(true),
            root_operations: input.map(|input| input.root_operations.iter().map(export_operation).collect()).unwrap_or_default(),
            root_attached_costs: input.map(|input| input.root_attached_costs.clone()).unwrap_or_default(),
            parts: line.parts.clone(),
            lifetime_demand: line.lifetime_demand,
            material_cost: line.material_cost,
            production_cost: line.production_cost,
            attached_cost: line.attached_cost,
            root_production_cost: line.root_production_cost,
            root_attached_cost: line.root_attached_cost,
            categories: line.categories.clone(),
            unit_total: line.unit_total,
            recurring_unit_cost: line.recurring_unit_cost,
        });
    }
    products.sort_by_key(|product| product.product_id);
    products
}

/// One rendering operation of a stored snapshot input.
fn export_operation(operation: &BaselineOperationInput) -> ExportOperation {
    ExportOperation {
        sequence: operation.sequence,
        operation_id: operation.operation_id,
        cycle: operation.cycle.clone(),
        setup: operation.setup.clone(),
    }
}

/// The saved commercial scenarios the comparison evaluated, in the requested order.
fn export_scenarios(comparison: Option<&CommercialScenarioComparison>) -> Vec<ExportScenario> {
    let Some(comparison) = comparison else { return Vec::new() };
    comparison
        .scenarios
        .iter()
        .map(|scenario| ExportScenario {
            scenario_id: scenario.scenario_id,
            version: scenario.version,
            name: scenario.name.clone(),
            stale: scenario.stale,
            stale_reasons: scenario.stale_reasons.clone(),
            outcome: scenario.outcome.clone(),
        })
        .collect()
}

/// Every listed issue, grouped by its owning contract in the contract's canonical order and sorted
/// deterministically inside each group. Missing inputs and unresolved findings are printed, never
/// invented away.
fn export_issues(
    snapshot: &BaselineSnapshot,
    tooling: &ToolingOutcome,
    comparison: Option<&CommercialScenarioComparison>,
    feasibility: &FeasibilityRecordView,
    stale_reasons: &[ExportStaleReason],
) -> Vec<ExportIssue> {
    let mut issues = Vec::new();
    // The stored costing issues of the immutable snapshot.
    for issue in &snapshot.outputs.issues {
        issues.push(ExportIssue {
            source: ExportIssueSource::Costing,
            code: issue.code.clone(),
            scope: issue.part_version_id.or(issue.product_id),
            message: issue.message.clone(),
        });
    }
    // The SB-17 tooling issues of the same snapshot.
    for issue in &tooling.issues {
        issues.push(ExportIssue {
            source: ExportIssueSource::Tooling,
            code: issue.code.clone(),
            scope: issue.product_id,
            message: issue.message.clone(),
        });
    }
    // The SB-18 comparison issues and the evaluated scenarios' own issues.
    if let Some(comparison) = comparison {
        for issue in &comparison.issues {
            issues.push(ExportIssue {
                source: ExportIssueSource::Commercial,
                code: issue.code.clone(),
                scope: issue.product_id,
                message: issue.message.clone(),
            });
        }
        for scenario in &comparison.scenarios {
            for issue in &scenario.outcome.issues {
                issues.push(ExportIssue {
                    source: ExportIssueSource::Commercial,
                    code: issue.code.clone(),
                    scope: issue.product_id,
                    message: issue.message.clone(),
                });
            }
        }
    }
    // The computed feasibility issues of the record version the documents carry. A feasibility issue
    // names a component or a finding, neither of which is a part version or a product.
    for issue in &feasibility.issues {
        issues.push(ExportIssue { source: ExportIssueSource::Feasibility, code: issue.code.clone(), scope: None, message: issue.message.clone() });
    }
    // One staleness row per named stale reason, in the contract's reason order.
    for reason in stale_reasons {
        issues.push(ExportIssue {
            source: ExportIssueSource::Staleness,
            code: reason.as_str().to_string(),
            scope: None,
            message: staleness_message(*reason).to_string(),
        });
    }
    issues.sort_by(|left, right| {
        source_rank(left.source)
            .cmp(&source_rank(right.source))
            .then_with(|| left.code.cmp(&right.code))
            .then_with(|| left.scope.cmp(&right.scope))
            .then_with(|| left.message.cmp(&right.message))
    });
    issues
}

/// The canonical source order of the issue list, matching the contract order.
fn source_rank(source: ExportIssueSource) -> u8 {
    match source {
        ExportIssueSource::Costing => 0,
        ExportIssueSource::Tooling => 1,
        ExportIssueSource::Commercial => 2,
        ExportIssueSource::Feasibility => 3,
        ExportIssueSource::Staleness => 4,
    }
}

/// Whether a named issue reports a missing or unavailable value, as the `missing_inputs` label
/// states. Every issue contract names a missing value with a `missing_` code; the extra codes name
/// an unavailable input that is not spelled that way.
fn missing_input_code(code: &str) -> bool {
    code.starts_with("missing_")
        || matches!(code, "unresolved_occurrence" | "feasibility_not_assessed" | "open_finding_unresolved" | "tooling_incomplete")
}

/// Whether the feasibility record carries a finding that is not a pass, or an accepted exception
/// that needs review again. A not-assessed record has no finding; its issue is a missing input.
fn has_unresolved_findings(view: &FeasibilityRecordView) -> bool {
    if let Some(assessment) = &view.latest_assessment
        && assessment.findings.iter().any(|finding| {
            finding.finding.state != FindingState::Pass || matches!(finding.disposition, FindingDisposition::ExceptionReviewRequired { .. })
        })
    {
        return true;
    }
    if view.exceptions.iter().any(|exception| exception.state == ExceptionState::ReviewRequired) {
        return true;
    }
    view.issues.iter().any(|issue| issue.code == "assessment_stale")
}

/// The human-readable statement one staleness issue row carries. The code is the stable contract
/// name; the renderer prints this message as written.
fn staleness_message(reason: ExportStaleReason) -> &'static str {
    match reason {
        ExportStaleReason::NewerProjectVersion => "the project record moved past the version the exported snapshot read",
        ExportStaleReason::NewerEngineeringVersion => "the engineering record moved past the version the exported snapshot read",
        ExportStaleReason::NewerManufacturingVersion => "the manufacturing record moved past the version the exported snapshot read",
        ExportStaleReason::NewerSharedData => "a costing-relevant shared-data change was recorded after the exported snapshot read its inputs",
    }
}

/// Map the SB-20 draft stale reason to the export contract's reason vocabulary.
fn export_stale_reason(reason: DraftStaleReason) -> ExportStaleReason {
    match reason {
        DraftStaleReason::NewerProjectVersion => ExportStaleReason::NewerProjectVersion,
        DraftStaleReason::NewerEngineeringVersion => ExportStaleReason::NewerEngineeringVersion,
        DraftStaleReason::NewerManufacturingVersion => ExportStaleReason::NewerManufacturingVersion,
        DraftStaleReason::NewerSharedData => ExportStaleReason::NewerSharedData,
    }
}

/// The stored feasibility record version, validated as the contract requires. Version 0 is handled
/// by the caller as the empty record; an unknown version is `not found`.
async fn feasibility_record_document_in(tx: &mut Transaction<'_, Postgres>, project_id: Uuid, version: u64) -> Result<FeasibilityRecordDocument> {
    let text: Option<String> = sqlx::query_scalar("SELECT document::text FROM feasibility_record_versions WHERE project_id = $1 AND version = $2")
        .bind(project_id)
        .bind(i64::try_from(version).map_err(StoreError::codec)?)
        .fetch_optional(&mut **tx)
        .await?;
    let document: FeasibilityRecordDocument = serde_json::from_str(&text.ok_or(StoreError::NotFound)?).map_err(StoreError::codec)?;
    document.validate().map_err(StoreError::codec)?;
    Ok(document)
}

/// The suggested export file name: one ASCII base name with the format's extension. The project name
/// is folded to lower-case ASCII, and an accepted baseline names its sequence so two exports of one
/// project are distinguishable without opening the document.
fn suggested_file_name(project_name: &str, format: ExportFormat, baseline: ExportBaselineSelection) -> String {
    let mut base = String::with_capacity(FILE_NAME_BASE_MAX_CHARS);
    for character in project_name.chars() {
        if base.len() >= FILE_NAME_BASE_MAX_CHARS {
            break;
        }
        if character.is_ascii_alphanumeric() {
            base.push(character.to_ascii_lowercase());
        } else if !base.ends_with('-') {
            base.push('-');
        }
    }
    let base = base.trim_matches('-');
    let stem = if base.is_empty() { "solidbom-export" } else { base };
    let suffix = match baseline {
        ExportBaselineSelection::AcceptedBaseline { sequence, .. } => format!("accepted-baseline-{sequence}"),
        ExportBaselineSelection::DraftCalculation { .. } => "draft".to_string(),
    };
    format!("{stem}-{suffix}.{}", format.file_extension())
}

/// The store error of a contract validation failure of a request.
fn invalid(error: solidbom_contract::ContractError) -> StoreError {
    StoreError::Invalid(error.to_string())
}
