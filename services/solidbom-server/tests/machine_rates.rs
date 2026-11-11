//! PostgreSQL integration tests for the machine hourly-rate store (gated package SB-X01).
//!
//! Each test runs in its own schema on `SOLIDBOM_TEST_DATABASE_URL` and drops it afterwards. The
//! suite covers the append-only version chain, the exact recommended arithmetic of the complete
//! fixture, the incomplete-draft refusals, publication into the SB-09 cost-center chain, the
//! read-only loading sensitivity and the stored-snapshot read rule.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

mod support;

use rust_decimal::Decimal;
use solidbom_contract::{
    CalculateRequest, CostCenterPurpose, DiagnosticSeverity, MachineRateStatus, MachineRateVersionView, RateDiagnosticCode, RateScenario,
    RateVersion, RateVersionState, ReferenceData, SaveDraft, SharedRecordKind, Unit,
};
use solidbom_server::{Store, StoreError};
use support::{
    TestDb, create as create_project, d, fixture_draft, machine_rate_create, machine_rate_input, machine_rate_input_for, machine_rate_machine_id,
    machine_rate_publish, row_count, seeded_rate_version_id, upload,
};
use uuid::Uuid;

/// The stored text of one machine rate version column, for the byte-identity assertions.
async fn column_text(store: &Store, machine_rate_version_id: Uuid, column: &str) -> Option<String> {
    // `column` is always a literal in the test source, never client input.
    sqlx::query_scalar(&format!("SELECT {column}::text FROM machine_rate_versions WHERE machine_rate_version_id = $1"))
        .bind(machine_rate_version_id)
        .fetch_one(store.pool())
        .await
        .expect("the stored column")
}

async fn create(db: &TestDb, request_id: &str, expected_version: u64, input: solidbom_contract::MachineRateInput) -> MachineRateVersionView {
    db.store
        .create_machine_rate_version(&machine_rate_create(request_id, expected_version, input), "administrator")
        .await
        .expect("create the machine rate version")
}

fn diagnostic_codes(view: &MachineRateVersionView) -> Vec<RateDiagnosticCode> {
    view.outcome.diagnostics.iter().map(|diagnostic| diagnostic.code).collect()
}

/// The seeded production-rate chain of the project fixture, ordered by version, so a byte-identity
/// assertion can show that a machine publication appends no version to the adopted chain.
fn seeded_production_rates(reference: &ReferenceData) -> Vec<RateVersion> {
    let mut rates: Vec<RateVersion> = reference.rates.iter().filter(|rate| rate.cost_center_id == seeded_rate_version_id()).cloned().collect();
    rates.sort_by_key(|rate| rate.version);
    rates
}

#[tokio::test]
async fn a_machine_rate_chain_is_append_only_and_idempotent() {
    let db = TestDb::new().await;
    let first = create(&db, "mr-1", 0, machine_rate_input()).await;
    assert_eq!((first.version.version, first.version.supersedes), (1, None));
    assert_eq!(first.version.state, RateVersionState::Draft);
    assert_eq!(first.version.created_by.as_deref(), Some("administrator"));
    assert_eq!(first.version.machine_rate_version_id, column_uuid(&db, "mr-1").await);
    assert!(first.version.created_at.is_some());
    assert!(first.version.published_at.is_none());
    assert!(first.version.published_rate_version_id.is_none());
    assert!(first.version.calculator_version.is_none());

    // An identical retry returns the recorded view and appends nothing.
    let retried = create(&db, "mr-1", 0, machine_rate_input()).await;
    assert_eq!(first, retried);
    assert_eq!(row_count(&db.store, "machine_rate_versions").await, 1);
    assert_eq!(row_count(&db.store, "requests").await, 1);

    // A stale expectation with a new request id is a conflict and writes nothing.
    let error = db
        .store
        .create_machine_rate_version(&machine_rate_create("mr-2", 0, machine_rate_input()), "administrator")
        .await
        .expect_err("a stale expected version is refused");
    assert!(matches!(error, StoreError::Conflict(_)), "{error}");
    assert_eq!(row_count(&db.store, "machine_rate_versions").await, 1);
    assert_eq!(row_count(&db.store, "requests").await, 1);

    // A changed payload under the same request id is a conflict.
    let mut changed = machine_rate_input();
    changed.annual_maintenance = Some(d("7000"));
    let error = db
        .store
        .create_machine_rate_version(&machine_rate_create("mr-1", 0, changed), "administrator")
        .await
        .expect_err("a reused request id with another payload conflicts");
    assert!(matches!(error, StoreError::Conflict(_)), "{error}");
    assert_eq!(row_count(&db.store, "machine_rate_versions").await, 1);

    // The next version supersedes the previous one; the earlier row is untouched.
    let second = create(&db, "mr-3", 1, machine_rate_input()).await;
    assert_eq!(second.version.version, 2);
    assert_eq!(second.version.supersedes, Some(first.version.machine_rate_version_id));
    assert_eq!(row_count(&db.store, "machine_rate_versions").await, 2);
    let stored_first = db.store.machine_rate_version(first.version.machine_rate_version_id).await.expect("read the first version");
    assert_eq!(stored_first.version.version, 1);
    assert_eq!(stored_first.version.supersedes, None);
    db.cleanup().await;
}

/// The machine rate version a create request recorded, read through the request receipt.
async fn column_uuid(db: &TestDb, request_id: &str) -> Uuid {
    let response: serde_json::Value = sqlx::query_scalar("SELECT response FROM requests WHERE request_id = $1")
        .bind(request_id)
        .fetch_one(db.store.pool())
        .await
        .expect("the recorded reply");
    serde_json::from_value(response["version"]["machine_rate_version_id"].clone()).expect("a version id")
}

#[tokio::test]
async fn concurrent_identical_machine_rate_creates_commit_once() {
    let db = TestDb::new().await;
    let request = machine_rate_create("mr-concurrent", 0, machine_rate_input());
    let (first, second) = tokio::join!(
        db.store.create_machine_rate_version(&request, "administrator"),
        db.store.create_machine_rate_version(&request, "administrator")
    );
    let first = first.expect("the first concurrent copy");
    let second = second.expect("the retried concurrent copy");
    assert_eq!(first, second);
    assert_eq!(row_count(&db.store, "machine_rate_versions").await, 1);
    assert_eq!(row_count(&db.store, "requests").await, 1);
    db.cleanup().await;
}

#[tokio::test]
async fn a_complete_draft_calculates_the_exact_recommended_numbers() {
    let db = TestDb::new().await;
    let view = create(&db, "mr-exact", 0, machine_rate_input()).await;
    let outcome = &view.outcome;
    assert!(matches!(outcome.status, MachineRateStatus::Ok | MachineRateStatus::OkWithWarnings), "{:?}", outcome.status);
    assert!(outcome.is_publishable());
    let result = outcome.result.as_ref().expect("a calculable outcome");
    assert_eq!(result.scheduled_capacity_hours, d("4000"));
    assert_eq!(result.loaded_scheduled_hours, d("3200"));
    assert_eq!(result.loaded_downtime_hours, d("800"));
    assert_eq!(result.actual_running_hours, d("2400"));
    assert_eq!(result.installed_capex, d("130000"));
    assert_eq!(result.residual_value, d("10000"));
    assert_eq!(result.remaining_depreciation_years, d("4"));
    assert_eq!(result.annual_depreciation, d("12000"));
    assert_eq!(result.annual_premises_cost, d("1200"));
    assert_eq!(result.loaded_committed_labor_rate, d("37.5"));
    assert_eq!(result.annual_committed_shift_labor, d("120000"));
    assert_eq!(result.annual_fixed_operating_cost, d("129000"));
    assert_eq!(result.running_electricity_per_hour, d("40"));
    assert_eq!(result.annual_standby_electricity, d("4800"));
    assert_eq!(result.compressed_air_per_hour, d("2"));
    assert_eq!(result.cooling_water_per_hour, d("2"));
    assert_eq!(result.run_only_labor_per_hour, d("36"));
    assert_eq!(result.variable_cost_excluding_standby, d("84"));
    assert_eq!(result.annual_variable_operating_cost, d("206400"));
    assert_eq!(result.annual_fully_absorbed_cost, d("347400"));
    assert_eq!(result.standby_electricity_per_running_hour, Some(d("2")));
    assert_eq!(result.fixed_operating_rate, Some(d("53.75")));
    assert_eq!(result.depreciation_rate, Some(d("5")));
    assert_eq!(result.variable_operating_rate, Some(d("86")));
    assert_eq!(result.total_machine_hourly_rate, Some(d("144.75")));
    assert_eq!(result.cash_operating_rate, Some(d("139.75")));
    assert_eq!(result.annual_cost_per_loaded_scheduled_hour, Some(d("108.5625")));

    // Reconciliation invariants the contract guarantees.
    assert_eq!(
        result.loaded_downtime_hours + result.actual_running_hours,
        result.loaded_scheduled_hours,
        "loaded downtime plus running equals the loaded hours"
    );
    assert_eq!(
        result.fixed_operating_rate.expect("a rate") + result.variable_operating_rate.expect("a rate") + result.depreciation_rate.expect("a rate"),
        result.total_machine_hourly_rate.expect("a rate")
    );
    assert_eq!(result.annual_fully_absorbed_cost, result.total_machine_hourly_rate.expect("a rate") * result.actual_running_hours);
    assert_eq!(result.annual_variable_operating_cost, result.variable_operating_rate.expect("a rate") * result.actual_running_hours);
    assert_eq!(
        result.cash_operating_rate.expect("a rate"),
        result.total_machine_hourly_rate.expect("a rate") - result.depreciation_rate.expect("a rate")
    );

    // The read route recalculates a draft with the same deterministic answer.
    let read = db.store.machine_rate_version(view.version.machine_rate_version_id).await.expect("read the draft");
    assert_eq!(read, view);
    db.cleanup().await;
}

#[tokio::test]
async fn an_incomplete_draft_is_invalid_and_is_never_calculated_as_zero() {
    let db = TestDb::new().await;

    // The Oven Forming case: a missing financial year makes the recommended mode refuse, even
    // though the workbook parity mode would produce a number. The service stores the draft but
    // returns no result at all.
    let mut missing_year = machine_rate_input();
    missing_year.financial_year = None;
    let view = create(&db, "mr-no-year", 0, missing_year).await;
    assert_eq!(view.outcome.status, MachineRateStatus::Invalid);
    assert!(view.outcome.result.is_none(), "an invalid outcome carries no result");
    assert!(diagnostic_codes(&view).contains(&RateDiagnosticCode::MissingFinancialYear));
    assert!(view.outcome.diagnostics.iter().any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error));
    assert!(!view.outcome.is_publishable());
    let error = db
        .store
        .publish_machine_rate_version(view.version.machine_rate_version_id, &machine_rate_publish("mr-publish-no-year", 1), "administrator")
        .await
        .expect_err("an invalid draft cannot be published");
    assert!(matches!(error, StoreError::Invalid(_)), "{error}");
    assert!(error.to_string().contains("missing_financial_year"), "{error}");
    assert_eq!(row_count(&db.store, "rate_versions").await, 2, "the two seeded fixtures and no publication");

    // A missing authoritative numeric input stays missing, never an implicit zero.
    let mut missing_residual = machine_rate_input();
    missing_residual.residual_value = None;
    let view = create(&db, "mr-no-residual", 0, missing_residual).await;
    assert_eq!(view.outcome.status, MachineRateStatus::Invalid);
    assert!(view.outcome.result.is_none());
    assert!(diagnostic_codes(&view).contains(&RateDiagnosticCode::MissingInput));

    // A negative assumption is a hard error, not clamped.
    let mut negative = machine_rate_input();
    negative.financial_year = Some(2027);
    negative.annual_insurance = Some(d("-1"));
    let view = create(&db, "mr-negative", 0, negative).await;
    assert_eq!(view.outcome.status, MachineRateStatus::Invalid);
    assert!(diagnostic_codes(&view).contains(&RateDiagnosticCode::NegativeInput));

    // A wire-level invalid identity is refused before anything is stored.
    let mut nil_machine = machine_rate_input();
    nil_machine.machine.machine_id = Uuid::nil();
    let error = db
        .store
        .create_machine_rate_version(&machine_rate_create("mr-nil", 0, nil_machine), "administrator")
        .await
        .expect_err("a nil machine id is refused");
    assert!(matches!(error, StoreError::Invalid(_)), "{error}");
    assert_eq!(row_count(&db.store, "machine_rate_versions").await, 3);
    db.cleanup().await;
}

#[tokio::test]
async fn a_zero_run_hours_draft_keeps_its_annual_costs_but_is_never_publishable() {
    let db = TestDb::new().await;
    let mut zero_loading = machine_rate_input();
    zero_loading.loading = Some(Decimal::ZERO);
    let view = create(&db, "mr-zero-loading", 0, zero_loading).await;
    assert_eq!(view.outcome.status, MachineRateStatus::NoRunHours);
    let result = view.outcome.result.as_ref().expect("the annual costs are still returned");
    assert_eq!(result.actual_running_hours, Decimal::ZERO);
    assert_eq!(result.annual_fixed_operating_cost, d("9000"));
    assert_eq!(result.annual_depreciation, d("12000"));
    assert_eq!(result.annual_fully_absorbed_cost, d("21000"));
    assert_eq!(result.total_machine_hourly_rate, None);
    assert_eq!(result.fixed_operating_rate, None);
    assert_eq!(result.annual_cost_per_loaded_scheduled_hour, None);
    let error = db
        .store
        .publish_machine_rate_version(view.version.machine_rate_version_id, &machine_rate_publish("mr-publish-zero", 1), "administrator")
        .await
        .expect_err("a zero-running-hour draft cannot be published");
    assert!(matches!(error, StoreError::Invalid(_)), "{error}");
    assert!(error.to_string().contains("zero_run_hours"), "{error}");
    assert_eq!(row_count(&db.store, "rate_versions").await, 2);
    db.cleanup().await;
}

#[tokio::test]
async fn publishing_appends_one_sb09_cost_center_rate_and_freezes_the_snapshot() {
    let db = TestDb::new().await;
    let draft = create(&db, "mr-publish", 0, machine_rate_input()).await;
    let machine_rate_version_id = draft.version.machine_rate_version_id;
    let published = db
        .store
        .publish_machine_rate_version(machine_rate_version_id, &machine_rate_publish("mr-publish-1", 1), "administrator")
        .await
        .expect("publish the draft");

    // The immutable machine rate version and its approval snapshot.
    assert_eq!(published.version.state, RateVersionState::Published);
    assert_eq!(published.version.published_rate_version_id, Some(published.published_rate.rate_version_id));
    assert_eq!(published.version.calculator_version.as_deref(), Some("solidbom-machine-rate/1"));
    assert!(published.version.published_at.is_some());
    assert_eq!(published.outcome, draft.outcome, "publishing recalculates the same deterministic outcome");
    published.validate().expect("the publication answer is valid");

    // The SB-09 production rate row and its change fact.
    assert_eq!(published.published_rate.cost_center_id, machine_rate_machine_id());
    assert_eq!(published.published_rate.version, 1);
    assert_eq!(published.published_rate.supersedes, None);
    assert_eq!(published.published_rate.name, "Extrusion Line 1");
    assert_eq!(published.published_rate.purpose, CostCenterPurpose::Production);
    assert_eq!(published.published_rate.rate_per_hour, Some(d("144.75")));
    assert_eq!(published.published_rate.rate_unit, Unit::CurrencyPerHour);
    assert_eq!(published.published_rate.currency, "CNY");
    let reference = published.published_rate.source_reference.as_deref().expect("the publication provenance");
    assert!(reference.contains(&machine_rate_version_id.to_string()), "{reference}");
    assert!(reference.contains("ACTUAL_RUNNING_HOUR"), "{reference}");
    assert_eq!(published.change.kind, SharedRecordKind::CostCenter);
    assert_eq!(published.change.record_id, machine_rate_machine_id());
    assert_eq!(published.change.version_id, published.published_rate.rate_version_id);
    assert_eq!(published.change.previous_version_id, None);
    assert_eq!(published.change.changed_by, "administrator");

    // The stored row carries the frozen snapshot and calculator version.
    let snapshot = column_text(&db.store, machine_rate_version_id, "outcome_snapshot").await.expect("the stored snapshot");
    let stored_outcome: solidbom_contract::MachineRateOutcome = serde_json::from_str(&snapshot).expect("the snapshot JSON");
    assert_eq!(stored_outcome, published.outcome);
    assert_eq!(column_text(&db.store, machine_rate_version_id, "calculator_version").await.as_deref(), Some("solidbom-machine-rate/1"));
    assert_eq!(column_text(&db.store, machine_rate_version_id, "state").await.as_deref(), Some("published"));

    // A published version is immutable: a second publish is a conflict, an identical retry returns
    // the recorded reply, and every read answers from the stored snapshot.
    let error = db
        .store
        .publish_machine_rate_version(machine_rate_version_id, &machine_rate_publish("mr-publish-2", 1), "administrator")
        .await
        .expect_err("an already published version cannot be published again");
    assert!(matches!(error, StoreError::Conflict(_)), "{error}");
    let retried = db
        .store
        .publish_machine_rate_version(machine_rate_version_id, &machine_rate_publish("mr-publish-1", 1), "administrator")
        .await
        .expect("an identical retry returns the recorded publication");
    assert_eq!(retried, published);
    let read = db.store.machine_rate_version(machine_rate_version_id).await.expect("read the published version");
    assert_eq!(read.outcome, published.outcome);
    assert_eq!(read.version.state, RateVersionState::Published);
    assert_eq!(column_text(&db.store, machine_rate_version_id, "outcome_snapshot").await.as_deref(), Some(snapshot.as_str()));

    // A later version supersedes the published one and appends a newer cost-center rate.
    let next = create(&db, "mr-publish-next", 1, machine_rate_input()).await;
    assert_eq!(next.version.supersedes, Some(machine_rate_version_id));
    let second = db
        .store
        .publish_machine_rate_version(next.version.machine_rate_version_id, &machine_rate_publish("mr-publish-3", 2), "administrator")
        .await
        .expect("publish the second version");
    assert_eq!(second.published_rate.version, 2);
    assert_eq!(second.published_rate.supersedes, Some(published.published_rate.rate_version_id));
    assert_eq!(second.change.previous_version_id, Some(published.published_rate.rate_version_id));
    assert_eq!(row_count(&db.store, "rate_versions").await, 4, "the two seeded fixtures plus the two published machine cost centers");
    db.cleanup().await;
}

#[tokio::test]
async fn publication_refuses_a_draft_without_a_cost_center_code() {
    let db = TestDb::new().await;
    let mut no_code = machine_rate_input();
    no_code.machine.code = None;
    let draft = create(&db, "mr-no-code", 0, no_code).await;
    assert!(draft.outcome.is_publishable(), "the arithmetic is valid; the identity is incomplete");
    let error = db
        .store
        .publish_machine_rate_version(draft.version.machine_rate_version_id, &machine_rate_publish("mr-publish-no-code", 1), "administrator")
        .await
        .expect_err("a machine without a cost-center code cannot be published");
    assert!(matches!(error, StoreError::Invalid(_)), "{error}");
    assert!(error.to_string().contains("missing_machine_code"), "{error}");
    let read = db.store.machine_rate_version(draft.version.machine_rate_version_id).await.expect("read the draft");
    assert_eq!(read.version.state, RateVersionState::Draft, "the refused publication changed nothing");
    assert_eq!(row_count(&db.store, "rate_versions").await, 2);
    assert_eq!(row_count(&db.store, "reference_changes").await, 0);
    db.cleanup().await;
}

#[tokio::test]
async fn sensitivity_is_a_pure_read_that_matches_the_base_rate() {
    let db = TestDb::new().await;
    let draft = create(&db, "mr-sensitivity", 0, machine_rate_input()).await;
    let machine_rate_version_id = draft.version.machine_rate_version_id;
    let input_before = column_text(&db.store, machine_rate_version_id, "input").await;
    let requests_before = row_count(&db.store, "requests").await;
    let reference_before = serde_json::to_vec(&db.store.reference_data().await.expect("reference data")).expect("JSON");
    let changes_before = row_count(&db.store, "reference_changes").await;

    let request = solidbom_contract::MachineRateSensitivityRequest { loadings: vec![d("0.8"), d("0.5"), Decimal::ZERO] };
    let sensitivity = db.store.machine_rate_sensitivity(machine_rate_version_id, &request).await.expect("calculate the sensitivity");
    sensitivity.validate().expect("the sensitivity answer is valid");
    assert_eq!(sensitivity.base, draft.outcome);
    assert_eq!(sensitivity.scenarios.len(), 3);
    assert_eq!(sensitivity.scenarios[0].loading, d("0.8"));
    assert_eq!(sensitivity.scenarios[0].outcome, draft.outcome, "the base loading reproduces the base outcome exactly");
    assert_eq!(sensitivity.scenarios[1].loading, d("0.5"));
    let half = sensitivity.scenarios[1].outcome.result.as_ref().expect("a calculable half-loading outcome");
    assert_eq!(half.actual_running_hours, d("1500"));
    assert_eq!(half.annual_fixed_operating_cost, d("84000"));
    assert_eq!(half.total_machine_hourly_rate, Some(d("150")));
    let zero = &sensitivity.scenarios[2];
    assert_eq!(zero.loading, Decimal::ZERO);
    assert_eq!(zero.outcome.status, MachineRateStatus::NoRunHours);
    assert_eq!(zero.outcome.result.as_ref().and_then(|result| result.total_machine_hourly_rate), None);

    // The stored draft, its snapshot column and the request log are byte-identical.
    assert_eq!(column_text(&db.store, machine_rate_version_id, "input").await, input_before);
    assert_eq!(column_text(&db.store, machine_rate_version_id, "outcome_snapshot").await, None);
    assert_eq!(row_count(&db.store, "requests").await, requests_before);
    assert_eq!(row_count(&db.store, "reference_changes").await, 0);
    // The contract says sensitivity leaves the SB-09 reference data and its change facts unchanged.
    assert_eq!(
        serde_json::to_vec(&db.store.reference_data().await.expect("reference data")).expect("JSON"),
        reference_before,
        "sensitivity leaves the SB-09 reference data byte-identical"
    );
    assert_eq!(row_count(&db.store, "reference_changes").await, changes_before, "sensitivity appends no reference change");

    // A published version stays byte-identical too, and its approved snapshot is the base answer.
    let published = db
        .store
        .publish_machine_rate_version(machine_rate_version_id, &machine_rate_publish("mr-publish-sensitivity", 1), "administrator")
        .await
        .expect("publish the draft");
    let snapshot_before = column_text(&db.store, machine_rate_version_id, "outcome_snapshot").await;
    let reference_published = serde_json::to_vec(&db.store.reference_data().await.expect("reference data")).expect("JSON");
    let changes_published = row_count(&db.store, "reference_changes").await;
    let sensitivity = db
        .store
        .machine_rate_sensitivity(machine_rate_version_id, &solidbom_contract::MachineRateSensitivityRequest { loadings: vec![d("0.8")] })
        .await
        .expect("calculate the sensitivity of the published version");
    assert_eq!(sensitivity.base, published.outcome);
    assert_eq!(sensitivity.scenarios[0].outcome, published.outcome);
    assert_eq!(column_text(&db.store, machine_rate_version_id, "outcome_snapshot").await, snapshot_before);
    assert_eq!(
        serde_json::to_vec(&db.store.reference_data().await.expect("reference data")).expect("JSON"),
        reference_published,
        "sensitivity leaves the published SB-09 reference data byte-identical"
    );
    assert_eq!(row_count(&db.store, "reference_changes").await, changes_published, "sensitivity appends no reference change");

    // Invalid loadings and an unknown version are refused without a request row.
    for loadings in [Vec::new(), vec![d("-0.1")], vec![d("1.1")], vec![d("0.5"); solidbom_contract::MAX_SENSITIVITY_LOADINGS + 1]] {
        let request = solidbom_contract::MachineRateSensitivityRequest { loadings };
        let error = db.store.machine_rate_sensitivity(machine_rate_version_id, &request).await.expect_err("invalid loadings are refused");
        assert!(matches!(error, StoreError::Invalid(_)), "{error}");
    }
    let missing =
        db.store.machine_rate_sensitivity(Uuid::new_v4(), &solidbom_contract::MachineRateSensitivityRequest { loadings: vec![d("0.5")] }).await;
    assert!(matches!(missing, Err(StoreError::NotFound)));
    db.cleanup().await;
}

#[tokio::test]
async fn a_published_version_answers_from_its_stored_snapshot_even_when_it_is_corrupt() {
    let db = TestDb::new().await;
    let draft = create(&db, "mr-snapshot", 0, machine_rate_input()).await;
    let machine_rate_version_id = draft.version.machine_rate_version_id;
    db.store
        .publish_machine_rate_version(machine_rate_version_id, &machine_rate_publish("mr-publish-snapshot", 1), "administrator")
        .await
        .expect("publish the draft");

    // A corrupt stored approval snapshot is an internal codec failure, never a silent recalculation.
    sqlx::query("UPDATE machine_rate_versions SET outcome_snapshot = '{\"status\":\"ok\",\"result\":null,\"diagnostics\":[]}'::jsonb WHERE machine_rate_version_id = $1")
        .bind(machine_rate_version_id)
        .execute(db.store.pool())
        .await
        .expect("corrupt the stored snapshot");
    let error = db.store.machine_rate_version(machine_rate_version_id).await.expect_err("a corrupt snapshot is a codec error");
    assert!(matches!(error, StoreError::Codec(_)), "{error}");
    let error = db
        .store
        .machine_rate_sensitivity(machine_rate_version_id, &solidbom_contract::MachineRateSensitivityRequest { loadings: vec![d("0.5")] })
        .await
        .expect_err("a corrupt snapshot is a codec error on the sensitivity read too");
    assert!(matches!(error, StoreError::Codec(_)), "{error}");

    // A corrupt stored input is a codec error as well.
    sqlx::query("UPDATE machine_rate_versions SET input = '{}'::jsonb WHERE machine_rate_version_id = $1")
        .bind(machine_rate_version_id)
        .execute(db.store.pool())
        .await
        .expect("corrupt the stored input");
    let error = db.store.machine_rate_version(machine_rate_version_id).await.expect_err("a corrupt input is a codec error");
    assert!(matches!(error, StoreError::Codec(_)), "{error}");
    db.cleanup().await;
}

#[tokio::test]
async fn the_machine_inventory_and_chain_filters_list_the_newest_versions() {
    let db = TestDb::new().await;
    let machine_a = machine_rate_machine_id();
    let machine_b = Uuid::from_u128(0x5b01_0002);
    let base_2026 = create(&db, "mr-a-base-2026", 0, machine_rate_input()).await;
    let mut budget_2026 = machine_rate_input();
    budget_2026.scenario = RateScenario::Budget;
    let budget = create(&db, "mr-a-budget-2026", 0, budget_2026).await;
    let mut base_2027 = machine_rate_input();
    base_2027.financial_year = Some(2027);
    let newer_base = create(&db, "mr-a-base-2027", 0, base_2027).await;
    let machine_b_version = create(&db, "mr-b-base-2026", 0, machine_rate_input_for(machine_b)).await;

    // The inventory has one row per machine, ordered by machine id. Machine A's newest recorded
    // version is the 2027 chain.
    let inventory = db.store.machine_rate_inventory().await.expect("the machine inventory");
    inventory.validate().expect("a valid inventory");
    assert_eq!(inventory.versions.len(), 2);
    assert_eq!(inventory.versions[0].version.input.machine.machine_id, machine_a);
    assert_eq!(inventory.versions[1].version.input.machine.machine_id, machine_b);
    let machine_a_latest = &inventory.versions[0].version;
    assert!(
        [newer_base.version.machine_rate_version_id, budget.version.machine_rate_version_id].contains(&machine_a_latest.machine_rate_version_id),
        "the inventory lists the newest recorded chain of the machine"
    );
    assert_eq!(inventory.versions[1].version.machine_rate_version_id, machine_b_version.version.machine_rate_version_id);

    // The chain list has every version of one machine, newest first, and the optional filters.
    let all = db.store.machine_rate_versions_of(machine_a, None, None).await.expect("the chains");
    assert_eq!(all.versions.len(), 3);
    let filtered = db.store.machine_rate_versions_of(machine_a, Some(2026), None).await.expect("the 2026 chains");
    assert_eq!(filtered.versions.len(), 2);
    let base_only = db.store.machine_rate_versions_of(machine_a, None, Some(RateScenario::Base)).await.expect("the base chains");
    assert_eq!(base_only.versions.len(), 2);
    let both = db.store.machine_rate_versions_of(machine_a, Some(2026), Some(RateScenario::Base)).await.expect("the base 2026 chain");
    assert_eq!(both.versions.len(), 1);
    assert_eq!(both.versions[0].version.machine_rate_version_id, base_2026.version.machine_rate_version_id);
    let budget_only = db.store.machine_rate_versions_of(machine_a, None, Some(RateScenario::Budget)).await.expect("the budget chain");
    assert_eq!(budget_only.versions[0].version.machine_rate_version_id, budget.version.machine_rate_version_id);
    let unknown = db.store.machine_rate_versions_of(Uuid::new_v4(), None, None).await.expect("an unknown machine lists no versions");
    assert!(unknown.versions.is_empty());
    db.cleanup().await;
}

#[tokio::test]
async fn a_machine_publication_preserves_project_snapshots_and_adds_no_double_charge() {
    let db = TestDb::new().await;

    // A real project: create version 1, save version 2, then calculate version 2 against the seeded
    // 20 CNY/kg material and 120 CNY/hour production-rate fixtures.
    let source = upload(&db.store).await;
    let created = create_project(&db.store, &source).await;
    let project_id = created.draft.project_id;
    let save = SaveDraft { request_id: "mr-project-save".into(), expected_version: 1, draft: fixture_draft(1, &source) };
    db.store.save_draft(&save).await.expect("save project version 2");
    let calculation = db
        .store
        .record_calculation(&CalculateRequest { request_id: "mr-project-calc".into(), project_id, saved_version: 2 })
        .await
        .expect("calculate project version 2");
    assert_eq!(calculation.saved_version, 2);
    assert_eq!(calculation.inputs.parts[0].material_price_per_kg, Some(d("20")));
    assert_eq!(calculation.inputs.parts[0].production_rate_per_hour, Some(d("120")));

    // Capture the project version, its immutable calculation snapshot and the reference data the
    // calculation resolved, as bytes.
    let project_before = serde_json::to_vec(&db.store.project_view(project_id).await.expect("project view")).expect("JSON");
    let calculation_before = serde_json::to_vec(&db.store.load_calculation(calculation.calculation_id).await.expect("calculation")).expect("JSON");
    let reference_before = db.store.reference_data().await.expect("reference data");
    let materials_before = serde_json::to_vec(&reference_before.materials).expect("JSON");
    let seeded_rates_before = serde_json::to_vec(&seeded_production_rates(&reference_before)).expect("JSON");
    assert_eq!(seeded_production_rates(&reference_before).len(), 1, "the seeded production chain is one version");

    // Publish a machine rate into its own new cost center.
    let draft = create(&db, "mr-project-rate", 0, machine_rate_input()).await;
    let published = db
        .store
        .publish_machine_rate_version(draft.version.machine_rate_version_id, &machine_rate_publish("mr-project-rate-publish", 1), "administrator")
        .await
        .expect("publish the machine rate version");
    assert_eq!(published.published_rate.cost_center_id, machine_rate_machine_id());
    assert_ne!(published.published_rate.cost_center_id, seeded_rate_version_id(), "the machine rate is its own cost center");

    // The earlier project version, its saved calculation snapshot and the fixture reference rows are
    // byte-identical: a machine publication adds no project CAPEX and no second depreciation or
    // availability charge, and it never inflates or replaces the adopted rate.
    let project_after = serde_json::to_vec(&db.store.project_view(project_id).await.expect("project view")).expect("JSON");
    assert_eq!(project_after, project_before, "a machine publication never edits a project version");
    let calculation_after = serde_json::to_vec(&db.store.load_calculation(calculation.calculation_id).await.expect("calculation")).expect("JSON");
    assert_eq!(calculation_after, calculation_before, "a machine publication never rewrites a calculation snapshot");
    let reference_after = db.store.reference_data().await.expect("reference data");
    assert_eq!(serde_json::to_vec(&reference_after.materials).expect("JSON"), materials_before, "the seeded material rows are unchanged");
    assert_eq!(
        serde_json::to_vec(&seeded_production_rates(&reference_after)).expect("JSON"),
        seeded_rates_before,
        "the adopted production-rate chain is unchanged"
    );
    let seeded_after = seeded_production_rates(&reference_after);
    assert_eq!(seeded_after[0].rate_per_hour, Some(d("120")), "the seeded 120 CNY/hour fixture keeps its value");
    assert_eq!(seeded_after[0].version, 1);
    assert_eq!(seeded_after[0].supersedes, None);
    assert_eq!(seeded_after[0].purpose, CostCenterPurpose::Production);

    // Exactly one rate row was appended, on the machine's distinct cost center; the adopted chain
    // gained no version, so the project cannot be charged twice.
    assert_eq!(reference_after.rates.len(), reference_before.rates.len() + 1, "only the machine rate row is added");
    assert_eq!(seeded_production_rates(&reference_after).len(), seeded_production_rates(&reference_before).len());
    let machine_rate =
        reference_after.rates.iter().find(|rate| rate.cost_center_id == machine_rate_machine_id()).expect("the published machine cost center");
    assert_eq!((machine_rate.version, machine_rate.supersedes), (1, None));
    assert_eq!(machine_rate.rate_per_hour, Some(d("144.75")));

    // The project document has no project-CAPEX field at all and the snapshot still names the
    // seeded version and values it adopted.
    let project_json: serde_json::Value = serde_json::from_slice(&project_before).expect("JSON");
    assert!(project_json["draft"].get("capex").is_none(), "the project document carries no project CAPEX");
    let loaded = db.store.load_calculation(calculation.calculation_id).await.expect("the saved snapshot");
    assert_eq!(loaded.inputs.parts[0].production_rate_per_hour, Some(d("120")));
    assert_eq!(loaded.inputs.parts[0].material_price_per_kg, Some(d("20")));
    assert_eq!(loaded.calculator_version, calculation.calculator_version);
    db.cleanup().await;
}
