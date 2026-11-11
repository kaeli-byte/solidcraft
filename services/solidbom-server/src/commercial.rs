//! Commercial scenario persistence and comparison (package SB-18, additive to contract version 2).
//!
//! The project document gains one additive optional `commercial` field: the EBIT target and the
//! annual selling-price productivity rate. An accepted settings write is an ordinary append-only
//! project record version, so the settings are versioned with the project and a scenario can never
//! override them.
//!
//! A named commercial scenario is its own append-only version chain in `commercial_scenarios`: a
//! create writes version 1 of a client-named identity, an edit checks the expected version under an
//! advisory lock and appends exactly one new immutable version in the same transaction as its
//! request receipt. A later version never rewrites an earlier one.
//!
//! [`Store::compare_commercial_scenarios`] is a pure read. It loads one immutable SB-16 baseline
//! snapshot, the exact project record version that snapshot names, the saved scenario versions and
//! the SB-17 tooling evaluation of the same snapshot with each scenario's recovery assumptions,
//! resolves the portable [`solidbom_contract::CommercialInputs`], runs the pure commercial
//! calculator from `solidbom_costing` and returns its answer per scenario with explicit stale
//! labels. It stores nothing and appends no scenario or project version.
//!
//! No arithmetic is repeated here: the tooling investment, allocation and recovery come from the
//! SB-17 calculator, and the price, EBIT and revenue series come from the SB-18 commercial
//! calculator.

use solidbom_contract::{
    BaselineSnapshot, CommercialComparisonScenario, CommercialInputs, CommercialIssue, CommercialProductInput, CommercialScenario,
    CommercialScenarioComparison, CommercialScenarioContent, CommercialScenarioList, CommercialScenarioWriteOutcome, CommercialStaleReason,
    CompareCommercialScenarios, CreateCommercialScenario, ProjectCommercial, ProjectDocument, ProjectWriteOutcome, ScenarioManualPrice,
    UpdateCommercialScenario, UpdateProjectCommercial,
};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

use crate::costing::{baseline_snapshot_in, project_version_document};
use crate::store::{RFC3339_UTC, Store, lock_logical_record, lock_request, payload_hash, record_reply, recorded_reply};
use crate::tooling::resolve_tooling_inputs;
use crate::{Result, StoreError};

impl Store {
    /// Replace the project's commercial settings: an expected-version-checked append-only project
    /// record version. The request id makes an identical retry return the recorded outcome and a
    /// changed payload under the same id a conflict.
    pub async fn update_project_commercial(&self, project_id: Uuid, request: &UpdateProjectCommercial) -> Result<ProjectWriteOutcome> {
        request.validate().map_err(invalid)?;
        let payload = payload_hash("project_record.commercial", &(project_id, request))?;
        self.write_document(project_id, &request.request_id, request.expected_version, &payload, "commercial", |document| {
            document.apply_commercial(request.commercial);
            Ok(Vec::new())
        })
        .await
    }

    /// Create version 1 of a named commercial scenario. The scenario id is a client-generated
    /// business identity; an existing one is a conflict. The content is validated against the
    /// baseline snapshot it names, so a manual price only names a product the baseline prices.
    pub async fn create_commercial_scenario(&self, project_id: Uuid, request: &CreateCommercialScenario) -> Result<CommercialScenarioWriteOutcome> {
        request.validate().map_err(invalid)?;
        let payload = payload_hash("commercial_scenario.create", &(project_id, request))?;
        let mut tx = self.pool().begin().await?;
        lock_request(&mut tx, &request.request_id).await?;
        if let Some(outcome) = recorded_reply(&mut tx, &request.request_id, &payload).await? {
            tx.commit().await?;
            return Ok(outcome);
        }
        let snapshot = baseline_snapshot_in(&mut tx, project_id, request.content.baseline_calculation_id).await?;
        validate_content_against_snapshot(&request.content, &snapshot)?;
        lock_logical_record(&mut tx, "commercial_scenario", request.scenario_id).await?;
        // The per-project cap is checked under a per-project lock, so two concurrent creates cannot
        // both observe room for the last scenario.
        lock_logical_record(&mut tx, "commercial_scenarios", project_id).await?;
        let existing: Option<i64> =
            sqlx::query_scalar("SELECT version FROM commercial_scenarios WHERE scenario_id = $1 ORDER BY version DESC LIMIT 1")
                .bind(request.scenario_id)
                .fetch_optional(&mut *tx)
                .await?;
        if existing.is_some() {
            return Err(StoreError::Conflict("a scenario with this id already exists".into()));
        }
        let current: i64 = sqlx::query_scalar("SELECT count(DISTINCT scenario_id) FROM commercial_scenarios WHERE project_id = $1")
            .bind(project_id)
            .fetch_one(&mut *tx)
            .await?;
        if usize::try_from(current).unwrap_or(usize::MAX) >= solidbom_contract::MAX_COMMERCIAL_SCENARIOS {
            return Err(StoreError::Invalid("commercial_scenario.scenario_id: the project already holds the maximum number of scenarios".into()));
        }
        insert_version(&mut tx, project_id, request.scenario_id, 1, &request.content, &snapshot).await?;
        let scenario = scenario_in(&mut tx, project_id, request.scenario_id, 1).await?;
        let outcome = CommercialScenarioWriteOutcome { scenario };
        record_reply(&mut tx, &request.request_id, "commercial_scenario.create", &payload, &outcome).await?;
        tx.commit().await?;
        Ok(outcome)
    }

    /// Append the next immutable version of one named commercial scenario. The expected version is
    /// checked under the scenario's advisory lock; an unknown scenario is not found and a stale
    /// expectation is a conflict.
    pub async fn update_commercial_scenario(
        &self,
        project_id: Uuid,
        scenario_id: Uuid,
        request: &UpdateCommercialScenario,
    ) -> Result<CommercialScenarioWriteOutcome> {
        request.validate().map_err(invalid)?;
        if request.scenario_id != scenario_id {
            return Err(StoreError::Invalid("the request belongs to a different scenario".into()));
        }
        let payload = payload_hash("commercial_scenario.update", &(project_id, scenario_id, request))?;
        let mut tx = self.pool().begin().await?;
        lock_request(&mut tx, &request.request_id).await?;
        if let Some(outcome) = recorded_reply(&mut tx, &request.request_id, &payload).await? {
            tx.commit().await?;
            return Ok(outcome);
        }
        lock_logical_record(&mut tx, "commercial_scenario", scenario_id).await?;
        let current: Option<i64> =
            sqlx::query_scalar("SELECT version FROM commercial_scenarios WHERE project_id = $1 AND scenario_id = $2 ORDER BY version DESC LIMIT 1")
                .bind(project_id)
                .bind(scenario_id)
                .fetch_optional(&mut *tx)
                .await?;
        let current = current.map(version_from_i64).transpose()?.ok_or(StoreError::NotFound)?;
        if current != request.expected_version {
            return Err(StoreError::Conflict(format!("the scenario is at version {current}, not {}", request.expected_version)));
        }
        let snapshot = baseline_snapshot_in(&mut tx, project_id, request.content.baseline_calculation_id).await?;
        validate_content_against_snapshot(&request.content, &snapshot)?;
        let next = current.checked_add(1).ok_or_else(|| StoreError::Conflict("the scenario version cannot advance further".into()))?;
        insert_version(&mut tx, project_id, scenario_id, next, &request.content, &snapshot).await?;
        let scenario = scenario_in(&mut tx, project_id, scenario_id, next).await?;
        let outcome = CommercialScenarioWriteOutcome { scenario };
        record_reply(&mut tx, &request.request_id, "commercial_scenario.update", &payload, &outcome).await?;
        tx.commit().await?;
        Ok(outcome)
    }

    /// The current immutable version of every scenario of one project, ordered by scenario id.
    pub async fn commercial_scenarios(&self, project_id: Uuid) -> Result<CommercialScenarioList> {
        ensure_project(self.pool(), project_id).await?;
        let rows = sqlx::query(&format!(
            "SELECT DISTINCT ON (scenario_id) scenario_id, project_id, version, name, baseline_calculation_id::text AS baseline_calculation_id, \
             project_version, manual_prices::text AS manual_prices, tooling::text AS tooling, productivity_applies_to_tooling, \
             to_char(created_at AT TIME ZONE 'UTC', {RFC3339_UTC}) AS created_at \
             FROM commercial_scenarios WHERE project_id = $1 ORDER BY scenario_id, version DESC"
        ))
        .bind(project_id)
        .fetch_all(self.pool())
        .await?;
        let scenarios = rows.iter().map(scenario_from_row).collect::<Result<Vec<_>>>()?;
        Ok(CommercialScenarioList { project_id, scenarios })
    }

    /// Compare named scenarios against one common immutable baseline. This is a read-only
    /// calculation: the baseline snapshot, the project record version it recorded and the saved
    /// scenario versions fully determine the answer, so nothing is stored.
    pub async fn compare_commercial_scenarios(&self, project_id: Uuid, request: &CompareCommercialScenarios) -> Result<CommercialScenarioComparison> {
        request.validate().map_err(invalid)?;
        // One transaction reads the baseline, the project versions and the scenarios, so a
        // comparison never mixes records written by different edits.
        let mut tx = self.pool().begin().await?;
        let snapshot = baseline_snapshot_in(&mut tx, project_id, request.baseline_calculation_id).await?;
        let project = project_version_document(&mut tx, project_id, snapshot.project_version).await?;
        let current_project_version: i64 = sqlx::query_scalar("SELECT current_version FROM project_records WHERE project_id = $1")
            .bind(project_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(StoreError::NotFound)?;
        let current_project_version = version_from_i64(current_project_version)?;
        let latest_baseline: Option<Uuid> = sqlx::query_scalar(
            "SELECT calculation_id FROM baseline_calculations WHERE project_id = $1 ORDER BY created_at DESC, calculation_id DESC LIMIT 1",
        )
        .bind(project_id)
        .fetch_optional(&mut *tx)
        .await?;

        let commercial = project.commercial;
        let mut scenarios = Vec::with_capacity(request.scenario_ids.len());
        for scenario_id in &request.scenario_ids {
            scenarios
                .push(self.comparison_line(&mut tx, project_id, *scenario_id, &snapshot, &project, &latest_baseline, current_project_version).await?);
        }
        tx.commit().await?;
        Ok(CommercialScenarioComparison {
            project_id,
            baseline_calculation_id: request.baseline_calculation_id,
            project_version: snapshot.project_version,
            volume_policy: snapshot.inputs.volume_policy,
            ebit_target_rate: commercial.map(|settings| settings.ebit_target_rate),
            annual_productivity_rate: commercial.map(|settings| settings.annual_productivity_rate),
            scenarios,
            issues: Vec::<CommercialIssue>::new(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn comparison_line(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        project_id: Uuid,
        scenario_id: Uuid,
        snapshot: &BaselineSnapshot,
        project: &ProjectDocument,
        latest_baseline: &Option<Uuid>,
        current_project_version: u64,
    ) -> Result<CommercialComparisonScenario> {
        let scenario = current_scenario_in(tx, project_id, scenario_id).await?.ok_or(StoreError::NotFound)?;
        let (inputs, unpriced_products) = resolve_commercial_inputs(project, snapshot, &scenario)?;
        inputs.validate().map_err(invalid)?;
        let outcome = solidbom_costing::calculate_commercial(&inputs);

        let mut stale_reasons = Vec::new();
        if !unpriced_products.is_empty() {
            stale_reasons.push(CommercialStaleReason::new(
                "scenario_manual_price_unpriced",
                "a saved manual price names a product the comparison baseline does not price; that price is ignored",
            ));
        }
        if scenario.baseline_calculation_id != snapshot.calculation_id {
            stale_reasons.push(CommercialStaleReason::new(
                "scenario_baseline_differs",
                "the scenario was saved against another baseline and is evaluated against the comparison's baseline",
            ));
        }
        if scenario.project_version != snapshot.project_version {
            stale_reasons
                .push(CommercialStaleReason::new("scenario_project_version_differs", "the scenario's saved baseline names another project version"));
        }
        if scenario.baseline_calculation_id != snapshot.calculation_id {
            let saved_snapshot = baseline_snapshot_in(tx, project_id, scenario.baseline_calculation_id).await?;
            let saved_project = project_version_document(tx, project_id, saved_snapshot.project_version).await?;
            if saved_project.commercial != project.commercial {
                stale_reasons.push(CommercialStaleReason::new(
                    "scenario_settings_differ",
                    "the scenario was saved under different EBIT target or productivity settings; the comparison's settings are used",
                ));
            }
        }
        if latest_baseline.is_some_and(|latest| latest != scenario.baseline_calculation_id) {
            stale_reasons.push(CommercialStaleReason::new("scenario_baseline_outdated", "a newer baseline snapshot exists for this project"));
        }
        if current_project_version > scenario.project_version {
            stale_reasons
                .push(CommercialStaleReason::new("scenario_project_outdated", "a newer project record version exists than the scenario pinned"));
        }
        let stale = !stale_reasons.is_empty();
        Ok(CommercialComparisonScenario {
            scenario_id: scenario.scenario_id,
            version: scenario.version,
            name: scenario.name,
            saved_baseline_calculation_id: scenario.baseline_calculation_id,
            saved_project_version: scenario.project_version,
            stale,
            stale_reasons,
            outcome,
        })
    }
}

/// The commercial settings of one project version, by its exact version. The comparison reads the
/// settings of the project record version the baseline snapshot recorded, never the current one.
///
/// A scenario may have been saved against another baseline whose quoted products differ from the
/// comparison baseline's (for example a product was excluded). Its saved manual price for a product
/// the comparison baseline no longer prices is ignored and reported as a stale reason, so the
/// scenario is still evaluated instead of refusing the whole comparison.
fn resolve_commercial_inputs(
    project: &ProjectDocument,
    snapshot: &BaselineSnapshot,
    scenario: &CommercialScenario,
) -> Result<(CommercialInputs, Vec<Uuid>)> {
    let settings: Option<ProjectCommercial> = project.commercial;
    let products: Vec<CommercialProductInput> = snapshot
        .outputs
        .products
        .iter()
        .map(|line| {
            let input = snapshot
                .inputs
                .products
                .iter()
                .find(|input| input.product_id == line.product_id)
                .ok_or_else(|| StoreError::Codec("the baseline snapshot prices a product it has no input for".into()))?;
            Ok(CommercialProductInput {
                product_id: line.product_id,
                product_revision_id: line.product_revision_id,
                schedule: input.schedule,
                forecasts: input.forecasts.clone(),
                recurring_unit_cost: line.recurring_unit_cost,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let tooling_inputs = resolve_tooling_inputs(project, snapshot, &scenario.tooling);
    let tooling = solidbom_costing::calculate_tooling(&tooling_inputs);
    let mut unpriced_products = Vec::new();
    let manual_prices: Vec<ScenarioManualPrice> = scenario
        .manual_prices
        .iter()
        .filter(|price| {
            let priced = products.iter().any(|product| product.product_id == price.product_id);
            if !priced {
                unpriced_products.push(price.product_id);
            }
            priced
        })
        .map(|price| ScenarioManualPrice { product_id: price.product_id, price: price.price })
        .collect();
    Ok((
        CommercialInputs {
            project_currency: project.currency.clone(),
            ebit_target_rate: settings.map(|settings| settings.ebit_target_rate),
            annual_productivity_rate: settings.map(|settings| settings.annual_productivity_rate),
            productivity_applies_to_tooling: scenario.productivity_applies_to_tooling,
            manual_prices,
            tooling,
            products,
            setup_total: snapshot.outputs.setup_total,
        },
        unpriced_products,
    ))
}

/// Validate scenario content against the baseline snapshot it is saved against: a manual price names
/// a quoted product of that snapshot.
fn validate_content_against_snapshot(content: &CommercialScenarioContent, snapshot: &BaselineSnapshot) -> Result<()> {
    for (index, price) in content.manual_prices.iter().enumerate() {
        if !snapshot.outputs.products.iter().any(|line| line.product_id == price.product_id) {
            return Err(StoreError::Invalid(format!(
                "commercial_scenario.manual_prices[{index}].product_id names a product the baseline does not price"
            )));
        }
    }
    Ok(())
}

/// Insert one immutable scenario version. The project version recorded is the one the pinned
/// baseline snapshot recorded, so the settings a later comparison reads stay reproducible.
async fn insert_version(
    tx: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    scenario_id: Uuid,
    version: u64,
    content: &CommercialScenarioContent,
    snapshot: &BaselineSnapshot,
) -> Result<()> {
    let manual_prices = serde_json::to_string(&content.manual_prices).map_err(StoreError::codec)?;
    let tooling = serde_json::to_string(&content.tooling).map_err(StoreError::codec)?;
    sqlx::query(
        "INSERT INTO commercial_scenarios (scenario_id, project_id, version, name, baseline_calculation_id, project_version, manual_prices, tooling, \
         productivity_applies_to_tooling) VALUES ($1, $2, $3, $4, $5, $6, $7::jsonb, $8::jsonb, $9)",
    )
    .bind(scenario_id)
    .bind(project_id)
    .bind(version_to_i64(version)?)
    .bind(&content.name)
    .bind(snapshot.calculation_id)
    .bind(version_to_i64(snapshot.project_version)?)
    .bind(manual_prices)
    .bind(tooling)
    .bind(content.productivity_applies_to_tooling)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn scenario_in(tx: &mut Transaction<'_, Postgres>, project_id: Uuid, scenario_id: Uuid, version: u64) -> Result<CommercialScenario> {
    let row = sqlx::query(&format!(
        "SELECT scenario_id, project_id, version, name, baseline_calculation_id::text AS baseline_calculation_id, project_version, \
         manual_prices::text AS manual_prices, tooling::text AS tooling, productivity_applies_to_tooling, \
         to_char(created_at AT TIME ZONE 'UTC', {RFC3339_UTC}) AS created_at \
         FROM commercial_scenarios WHERE project_id = $1 AND scenario_id = $2 AND version = $3"
    ))
    .bind(project_id)
    .bind(scenario_id)
    .bind(version_to_i64(version)?)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(StoreError::NotFound)?;
    scenario_from_row(&row)
}

/// The current (greatest) immutable version of one scenario, if the project has it.
async fn current_scenario_in(tx: &mut Transaction<'_, Postgres>, project_id: Uuid, scenario_id: Uuid) -> Result<Option<CommercialScenario>> {
    let row = sqlx::query(&format!(
        "SELECT scenario_id, project_id, version, name, baseline_calculation_id::text AS baseline_calculation_id, project_version, \
         manual_prices::text AS manual_prices, tooling::text AS tooling, productivity_applies_to_tooling, \
         to_char(created_at AT TIME ZONE 'UTC', {RFC3339_UTC}) AS created_at \
         FROM commercial_scenarios WHERE project_id = $1 AND scenario_id = $2 ORDER BY version DESC LIMIT 1"
    ))
    .bind(project_id)
    .bind(scenario_id)
    .fetch_optional(&mut **tx)
    .await?;
    row.as_ref().map(scenario_from_row).transpose()
}

fn scenario_from_row(row: &sqlx::postgres::PgRow) -> Result<CommercialScenario> {
    let manual_prices: String = row.try_get("manual_prices")?;
    let tooling: String = row.try_get("tooling")?;
    let scenario = CommercialScenario {
        scenario_id: row.try_get("scenario_id")?,
        project_id: row.try_get("project_id")?,
        version: version_from_i64(row.try_get("version")?)?,
        name: row.try_get("name")?,
        baseline_calculation_id: Uuid::parse_str(&row.try_get::<String, _>("baseline_calculation_id")?).map_err(StoreError::codec)?,
        project_version: version_from_i64(row.try_get("project_version")?)?,
        manual_prices: serde_json::from_str(&manual_prices).map_err(StoreError::codec)?,
        tooling: serde_json::from_str(&tooling).map_err(StoreError::codec)?,
        productivity_applies_to_tooling: row.try_get("productivity_applies_to_tooling")?,
        created_at: Some(row.try_get("created_at")?),
    };
    scenario.validate().map_err(StoreError::codec)?;
    Ok(scenario)
}

async fn ensure_project(pool: &sqlx::PgPool, project_id: Uuid) -> Result<()> {
    let exists: Option<i64> =
        sqlx::query_scalar("SELECT current_version FROM project_records WHERE project_id = $1").bind(project_id).fetch_optional(pool).await?;
    exists.map(|_| ()).ok_or(StoreError::NotFound)
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
