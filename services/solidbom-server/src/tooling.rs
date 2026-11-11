//! Project tooling settings and tooling investment/allocation/recovery evaluation (package SB-17,
//! additive to contract version 2).
//!
//! The project document gains one additive optional `tooling` field: the default commercial
//! treatment and the tool entries. An accepted settings write is an ordinary append-only project
//! record version (`Store::write_document`), so editing the settings never rewrites an earlier
//! version and never changes an earlier baseline snapshot. The owner of each entry is validated
//! against the project currency, the project's part versions and its products by
//! `ProjectDocument::validate` before anything is appended.
//!
//! [`Store::evaluate_tooling_recovery`] is a pure read. It loads one immutable SB-16 baseline
//! snapshot, reads the exact project record version that snapshot names, resolves the portable
//! [`solidbom_contract::ToolingInputs`] from that document's tooling settings and saved exchange
//! rates plus the demand the baseline already resolved, runs the pure calculator from
//! `solidbom_costing` and returns its answer. It stores nothing and appends no project version.
//!
//! # Resolution rules of the service adapter
//!
//! The demand is copied from the exact immutable baseline snapshot the request names, never
//! re-derived from a manifest:
//!
//! - `project_currency` is the currency of the evaluated project record version.
//! - `exchange_rates` are that version's saved manual rates from `document.costing`; a document
//!   written before SB-16 has no rates and stays empty, never an implicit identity rate.
//! - `tooling` is that version's settings; a version without settings reads as the empty settings
//!   (`None` is not an implicit treatment).
//! - One `ToolingProductDemand` exists for every product line of the immutable baseline output, in
//!   product id order. `lifetime_demand` and `product_revision_id` are the baseline's values,
//!   `forecasts` are that product's stored positive forecasts from the snapshot inputs (empty when
//!   the snapshot has no input for the product), and `part_quantities` are the baseline's expanded
//!   non-suppressed occurrence counts with zero quantities left out, ordered by part version id.
//! - The purchased-branch cutoff and the unresolved-occurrence rule are the baseline's: they are
//!   already applied to the recorded output, so a part that sits only under a purchased branch has
//!   no expanded quantity here.
//!
//! A resolved input is validated with `ToolingInputs::validate` before the calculator runs, so a
//! hostile stored record is refused with a client error instead of reaching the calculator as a
//! stored answer.

use solidbom_contract::{
    BaselineSnapshot, EvaluateToolingRecovery, ProjectDocument, ProjectWriteOutcome, ToolingDemand, ToolingInputs, ToolingOutcome,
    ToolingPartQuantity, ToolingProductDemand, ToolingRecoveryInputs, UpdateProjectTooling,
};
use uuid::Uuid;

use crate::Result;
use crate::costing::{baseline_snapshot_in, project_version_document};
use crate::store::{Store, StoreError, payload_hash};

impl Store {
    /// Replace the project's tooling settings: an expected-version-checked append-only project
    /// record version, validated against the project currency, part versions and products by
    /// `ProjectDocument::validate`. The request id makes an identical retry return the recorded
    /// outcome and a changed payload under the same id a conflict.
    pub async fn update_project_tooling(&self, project_id: Uuid, request: &UpdateProjectTooling) -> Result<ProjectWriteOutcome> {
        request.validate().map_err(invalid)?;
        let payload = payload_hash("project_record.tooling", &(project_id, request))?;
        self.write_document(project_id, &request.request_id, request.expected_version, &payload, "tooling", |document| {
            document.apply_tooling(request.tooling.clone());
            Ok(Vec::new())
        })
        .await
    }

    /// Evaluate the tooling investment, allocation and separate or amortized recovery of one exact
    /// project record version against one immutable baseline snapshot. This is a read-only
    /// calculation: the snapshot, the project version and the scenario assumptions fully determine
    /// the answer, so nothing is stored and no project version is appended.
    pub async fn evaluate_tooling_recovery(&self, project_id: Uuid, request: &EvaluateToolingRecovery) -> Result<ToolingOutcome> {
        request.validate().map_err(invalid)?;
        // One transaction reads the snapshot and the project record version it names, so an
        // evaluation never mixes a snapshot with a project version written by a different edit.
        let mut tx = self.pool().begin().await?;
        let snapshot = baseline_snapshot_in(&mut tx, project_id, request.baseline_calculation_id).await?;
        let project = project_version_document(&mut tx, project_id, snapshot.project_version).await?;
        let inputs = resolve_tooling_inputs(&project, &snapshot, &request.recovery);
        // A broken resolved input is refused before the calculator runs.
        inputs.validate().map_err(invalid)?;
        let outcome = solidbom_costing::calculate_tooling(&inputs);
        tx.commit().await?;
        Ok(outcome)
    }
}

/// Resolve the portable calculator inputs from the evaluated project record version and the
/// immutable baseline snapshot. This is the whole adapter: the calculator never sees a database row
/// and never re-derives demand from a raw manifest.
fn resolve_tooling_inputs(project: &ProjectDocument, snapshot: &BaselineSnapshot, recovery: &ToolingRecoveryInputs) -> ToolingInputs {
    let mut products: Vec<ToolingProductDemand> = snapshot
        .outputs
        .products
        .iter()
        .map(|line| {
            let forecasts = snapshot
                .inputs
                .products
                .iter()
                .find(|input| input.product_id == line.product_id)
                .map(|input| input.forecasts.clone())
                .unwrap_or_default();
            let mut part_quantities: Vec<ToolingPartQuantity> = line
                .parts
                .iter()
                .filter(|part| part.quantity > 0)
                .map(|part| ToolingPartQuantity { part_version_id: part.part_version_id, quantity: part.quantity })
                .collect();
            part_quantities.sort_by_key(|part| part.part_version_id);
            ToolingProductDemand {
                product_id: line.product_id,
                product_revision_id: line.product_revision_id,
                lifetime_demand: line.lifetime_demand,
                forecasts,
                part_quantities,
            }
        })
        .collect();
    products.sort_by_key(|product| product.product_id);
    ToolingInputs {
        project_currency: project.currency.clone(),
        exchange_rates: project.costing.as_ref().map(|costing| costing.exchange_rates.clone()).unwrap_or_default(),
        tooling: project.tooling.clone().unwrap_or_default(),
        recovery: recovery.clone(),
        demand: ToolingDemand { products },
    }
}

fn invalid(error: solidbom_contract::ContractError) -> StoreError {
    StoreError::Invalid(error.to_string())
}
