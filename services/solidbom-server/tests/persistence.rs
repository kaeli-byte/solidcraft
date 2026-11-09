//! PostgreSQL integration tests for the SolidBOM store.
//!
//! Each test runs in its own schema on `SOLIDBOM_TEST_DATABASE_URL` and drops it afterwards. The
//! tests never reset the database they connect to.

mod support;

use solidbom_contract::{CalculateRequest, CostCenterPurpose, DirectCostInput, SaveDraft, SharedRecordKind, SourceRef, Unit};
use solidbom_server::StoreError;
use support::{
    FIXTURE_SOURCE, TestDb, app_config_version, component_version, create, d, digest, direct_category, fixture_draft, material_version,
    percentage_category, rate_version, row_count, seeded_material_version_id, seeded_rate_version_id, two_revision_draft, upload,
};
use uuid::Uuid;

#[tokio::test]
async fn a_saved_project_survives_a_reconnect() {
    let db = TestDb::new().await;
    let source = upload(&db.store).await;
    let create = create(&db.store, &source).await;
    let save = SaveDraft { request_id: "save-1".into(), expected_version: 1, draft: fixture_draft(1, &source) };
    let receipt = db.store.save_draft(&save).await.expect("save version 2");
    assert_eq!(receipt.version, 2);

    let store = db.reconnect().await;
    let draft = store.load_project(create.draft.project_id).await.expect("load the project");
    assert_eq!(draft.version, 2);
    assert_eq!(draft.parts, save.draft.parts);
    assert_eq!(draft.source.sha256, digest(FIXTURE_SOURCE));
    assert_eq!(draft.analysis.occurrences.len(), 3);
    assert_eq!(draft.analysis.geometry_bindings.len(), 3);
    db.cleanup().await;
}

#[tokio::test]
async fn a_stale_version_is_refused_and_changes_nothing() {
    let db = TestDb::new().await;
    let source = upload(&db.store).await;
    let create = create(&db.store, &source).await;
    let save = SaveDraft { request_id: "save-1".into(), expected_version: 1, draft: fixture_draft(1, &source) };
    db.store.save_draft(&save).await.expect("save version 2");

    let stale = SaveDraft { request_id: "stale-1".into(), expected_version: 1, draft: fixture_draft(1, &source) };
    let error = db.store.save_draft(&stale).await.expect_err("a stale save is refused");
    assert!(matches!(error, StoreError::Conflict(_)), "{error}");
    assert_eq!(db.store.version_count(create.draft.project_id).await.expect("count"), 2);
    assert_eq!(db.store.load_project(create.draft.project_id).await.expect("load").version, 2);
    db.cleanup().await;
}

#[tokio::test]
async fn an_unknown_source_is_refused_and_leaves_no_project() {
    let db = TestDb::new().await;
    let mut draft = fixture_draft(
        0,
        &SourceRef {
            source_id: Uuid::new_v4(),
            sha256: digest(FIXTURE_SOURCE),
            byte_len: FIXTURE_SOURCE.len() as u64,
            original_name: "three_boxes.step".into(),
        },
    );
    draft.project_id = Uuid::new_v4();
    let save = SaveDraft { request_id: "create-1".into(), expected_version: 0, draft };
    let error = db.store.create_draft(&save).await.expect_err("the source was never uploaded");
    assert!(matches!(error, StoreError::Invalid(_)), "{error}");
    let projects: i64 = sqlx::query_scalar("SELECT count(*) FROM projects").fetch_one(db.store.pool()).await.expect("count projects");
    let versions: i64 = sqlx::query_scalar("SELECT count(*) FROM project_versions").fetch_one(db.store.pool()).await.expect("count versions");
    let requests: i64 = sqlx::query_scalar("SELECT count(*) FROM requests").fetch_one(db.store.pool()).await.expect("count requests");
    assert_eq!((projects, versions, requests), (0, 0, 0));
    db.cleanup().await;
}

#[tokio::test]
async fn the_same_part_number_with_two_revisions_stays_separate() {
    let db = TestDb::new().await;
    let source = upload(&db.store).await;
    let save = SaveDraft { request_id: "create-1".into(), expected_version: 0, draft: two_revision_draft(0, &source) };
    let receipt = db.store.create_draft(&save).await.expect("create two revisions");
    let draft = db.store.load_project(receipt.project_id).await.expect("load");
    assert_eq!(draft.analysis.definitions.len(), 2);
    assert_eq!(draft.parts.len(), 2);
    let revisions: Vec<&str> = draft.analysis.definitions.iter().filter_map(|definition| definition.customer_revision.as_deref()).collect();
    assert_eq!(revisions, ["B", "C"]);
    assert!(draft.analysis.definitions.iter().all(|definition| definition.customer_part_number.as_deref() == Some("PL-100")));
    db.cleanup().await;
}

#[tokio::test]
async fn a_repeated_request_returns_its_receipt_and_a_changed_payload_conflicts() {
    let db = TestDb::new().await;
    let source = upload(&db.store).await;
    let create = create(&db.store, &source).await;
    let save = SaveDraft { request_id: "save-1".into(), expected_version: 1, draft: fixture_draft(1, &source) };
    let first = db.store.save_draft(&save).await.expect("save version 2");
    let retried = db.store.save_draft(&save).await.expect("the lost reply is retried");
    assert_eq!(first, retried);
    assert_eq!(db.store.version_count(create.draft.project_id).await.expect("count"), 2);

    let mut changed = save.clone();
    changed.draft.parts[0].operation.as_mut().expect("operation").cycle_seconds = Some(d("60"));
    let error = db.store.save_draft(&changed).await.expect_err("a changed payload under the same ID conflicts");
    assert!(matches!(error, StoreError::Conflict(_)), "{error}");
    assert_eq!(db.store.version_count(create.draft.project_id).await.expect("count"), 2);
    db.cleanup().await;
}

#[tokio::test]
async fn the_same_source_bytes_are_stored_once() {
    let db = TestDb::new().await;
    let first = upload(&db.store).await;
    let second = upload(&db.store).await;
    assert_eq!(first.source_id, second.source_id);
    let (source, bytes) = db.store.get_source(first.source_id).await.expect("read the source back");
    assert_eq!(source.original_name, "three_boxes.step");
    assert_eq!(bytes, FIXTURE_SOURCE);
    assert_eq!(source.sha256, digest(FIXTURE_SOURCE));
    let mismatched = db.store.put_source(&"0".repeat(64), "other.step", FIXTURE_SOURCE).await;
    assert!(matches!(mismatched, Err(StoreError::Invalid(_))));
    let missing = db.store.get_source(Uuid::new_v4()).await;
    assert!(matches!(missing, Err(StoreError::NotFound)));
    db.cleanup().await;
}

#[tokio::test]
async fn reference_data_is_seeded_with_the_explicit_test_values() {
    let db = TestDb::new().await;
    let data = db.store.reference_data().await.expect("reference data");
    data.validate().expect("the seed is valid");
    let material = data.materials.iter().find(|material| material.density_g_cm3 == d("2.7")).expect("the 2.7 g/cm3 fixture");
    assert_eq!(material.price_per_kg, Some(d("20")));
    assert!(data.rates.iter().any(|rate| rate.rate_per_hour == Some(d("120"))));
    db.cleanup().await;
}

#[tokio::test]
async fn a_failed_save_keeps_every_earlier_version() {
    let db = TestDb::new().await;
    let source = upload(&db.store).await;
    let create = create(&db.store, &source).await;
    let save = SaveDraft { request_id: "save-1".into(), expected_version: 1, draft: fixture_draft(1, &source) };
    db.store.save_draft(&save).await.expect("save version 2");

    let mut bad = SaveDraft { request_id: "save-bad".into(), expected_version: 2, draft: fixture_draft(2, &source) };
    bad.draft.source.sha256 = "b".repeat(64);
    bad.draft.analysis.key.source_sha256 = "b".repeat(64);
    for definition in &mut bad.draft.analysis.definitions {
        definition.source_key.source_sha256 = "b".repeat(64);
    }
    bad.draft.parts[0].volume_mm3.analysis_key.source_sha256 = "b".repeat(64);
    bad.draft.version = 2;
    let error = db.store.save_draft(&bad).await.expect_err("the source reference is wrong");
    assert!(matches!(error, StoreError::Invalid(_)), "{error}");
    assert_eq!(db.store.version_count(create.draft.project_id).await.expect("count"), 2);
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM project_versions WHERE version = 3").fetch_one(db.store.pool()).await.expect("count version 3");
    assert_eq!(rows, 0);
    db.cleanup().await;
}

#[tokio::test]
async fn a_lost_create_reply_is_answered_once() {
    let db = TestDb::new().await;
    let source = upload(&db.store).await;
    let save = SaveDraft { request_id: "create-1".into(), expected_version: 0, draft: fixture_draft(0, &source) };
    let first = db.store.create_draft(&save).await.expect("create the project");
    let retried = db.store.create_draft(&save).await.expect("the lost reply is retried");
    assert_eq!(first, retried);
    assert_eq!(row_count(&db.store, "projects").await, 1);
    assert_eq!(row_count(&db.store, "project_versions").await, 1);
    assert_eq!(row_count(&db.store, "requests").await, 1);
    db.cleanup().await;
}

#[tokio::test]
async fn concurrent_identical_creates_commit_once() {
    let db = TestDb::new().await;
    let source = upload(&db.store).await;
    let save = SaveDraft { request_id: "create-1".into(), expected_version: 0, draft: fixture_draft(0, &source) };
    let (first, second) = tokio::join!(db.store.create_draft(&save), db.store.create_draft(&save));
    let first = first.expect("the first concurrent copy");
    let second = second.expect("the retried concurrent copy");
    assert_eq!(first, second);
    assert_eq!(row_count(&db.store, "projects").await, 1);
    assert_eq!(row_count(&db.store, "project_versions").await, 1);
    assert_eq!(row_count(&db.store, "requests").await, 1);
    db.cleanup().await;
}

#[tokio::test]
async fn a_changed_create_payload_under_a_reused_request_id_conflicts() {
    let db = TestDb::new().await;
    let source = upload(&db.store).await;
    let save = SaveDraft { request_id: "create-1".into(), expected_version: 0, draft: fixture_draft(0, &source) };
    db.store.create_draft(&save).await.expect("create the project");

    let mut changed = save.clone();
    changed.draft.parts[0].operation.as_mut().expect("operation").cycle_seconds = Some(d("60"));
    let error = db.store.create_draft(&changed).await.expect_err("a changed payload under the same ID conflicts");
    assert!(matches!(error, StoreError::Conflict(_)), "{error}");
    assert_eq!(row_count(&db.store, "projects").await, 1);
    assert_eq!(row_count(&db.store, "requests").await, 1);
    db.cleanup().await;
}

#[tokio::test]
async fn a_calculation_retry_returns_the_same_snapshot() {
    let db = TestDb::new().await;
    let source = upload(&db.store).await;
    let created = create(&db.store, &source).await;
    let project_id = created.draft.project_id;
    let request = CalculateRequest { request_id: "calc-1".into(), project_id, saved_version: 1 };
    let first = db.store.record_calculation(&request).await.expect("calculate version 1");
    let retried = db.store.record_calculation(&request).await.expect("the lost reply is retried");
    assert_eq!(first, retried);
    assert_eq!(row_count(&db.store, "calculations").await, 1);

    let save = SaveDraft { request_id: "save-1".into(), expected_version: 1, draft: fixture_draft(1, &source) };
    db.store.save_draft(&save).await.expect("save version 2");
    let changed = CalculateRequest { request_id: "calc-1".into(), project_id, saved_version: 2 };
    let error = db.store.record_calculation(&changed).await.expect_err("a changed payload under the same ID conflicts");
    assert!(matches!(error, StoreError::Conflict(_)), "{error}");
    assert_eq!(row_count(&db.store, "calculations").await, 1);
    db.cleanup().await;
}

#[tokio::test]
async fn an_unknown_material_reference_is_invalid_and_writes_nothing() {
    let db = TestDb::new().await;
    let source = upload(&db.store).await;
    let mut draft = fixture_draft(0, &source);
    draft.parts[0].material_version_id = Some(Uuid::new_v4());
    let save = SaveDraft { request_id: "create-unknown".into(), expected_version: 0, draft };
    let error = db.store.create_draft(&save).await.expect_err("the selected material does not exist");
    assert!(matches!(error, StoreError::Invalid(_)), "{error}");
    assert_eq!(row_count(&db.store, "projects").await, 0);
    assert_eq!(row_count(&db.store, "project_versions").await, 0);
    assert_eq!(row_count(&db.store, "requests").await, 0);
    db.cleanup().await;
}

#[tokio::test]
async fn an_oversized_request_id_writes_nothing() {
    let db = TestDb::new().await;
    let source = upload(&db.store).await;
    let save = SaveDraft { request_id: "x".repeat(5_000), expected_version: 0, draft: fixture_draft(0, &source) };
    let error = db.store.create_draft(&save).await.expect_err("the request id is too long");
    assert!(matches!(error, StoreError::Invalid(_)), "{error}");
    assert_eq!(row_count(&db.store, "projects").await, 0);
    assert_eq!(row_count(&db.store, "requests").await, 0);
    db.cleanup().await;
}

#[tokio::test]
async fn a_save_retry_returns_its_receipt_after_the_project_moved_on() {
    let db = TestDb::new().await;
    let source = upload(&db.store).await;
    let create = create(&db.store, &source).await;
    let save_one = SaveDraft { request_id: "save-1".into(), expected_version: 1, draft: fixture_draft(1, &source) };
    let first = db.store.save_draft(&save_one).await.expect("save version 2");
    let save_two = SaveDraft { request_id: "save-2".into(), expected_version: 2, draft: fixture_draft(2, &source) };
    let second = db.store.save_draft(&save_two).await.expect("save version 3");
    assert_eq!((first.version, second.version), (2, 3));

    // The idempotency check runs before the optimistic-concurrency check: the lost reply to the
    // first save is still answered with its own receipt, without appending anything.
    let retried = db.store.save_draft(&save_one).await.expect("the lost reply is retried");
    assert_eq!(retried, first);
    assert_eq!(db.store.version_count(create.draft.project_id).await.expect("count"), 3);
    assert_eq!(db.store.load_project(create.draft.project_id).await.expect("load").version, 3);
    db.cleanup().await;
}

#[tokio::test]
async fn the_seeded_reference_rows_are_version_one_chains_with_explicit_units() {
    let db = TestDb::new().await;
    let data = db.store.reference_data().await.expect("reference data");
    data.validate().expect("the seed and its chain fields are valid");
    let material = data.materials.iter().find(|material| material.material_version_id == seeded_material_version_id()).expect("the PA12 fixture");
    assert_eq!(material.material_id, seeded_material_version_id());
    assert_eq!(material.version, 1);
    assert_eq!(material.supersedes, None);
    assert_eq!(material.price_per_kg, Some(d("20")));
    assert_eq!(material.density_g_cm3, d("2.7"));
    assert_eq!(material.density_unit, Unit::GramPerCentimetreCubed);
    assert_eq!(material.price_unit, Unit::CurrencyPerKilogram);
    assert_eq!(material.currency, "CNY");
    assert!(material.created_at.is_some());
    let production = data.rates.iter().find(|rate| rate.rate_version_id == seeded_rate_version_id()).expect("the machining fixture");
    assert_eq!(production.cost_center_id, seeded_rate_version_id());
    assert_eq!(production.version, 1);
    assert_eq!(production.purpose, CostCenterPurpose::Production);
    assert_eq!(production.rate_per_hour, Some(d("120")));
    assert_eq!(production.rate_unit, Unit::CurrencyPerHour);
    let setup = data.rates.iter().find(|rate| rate.rate_version_id == Uuid::parse_str("00000000-0000-4000-8000-000000000102").expect("the setup id"));
    assert_eq!(setup.map(|rate| rate.purpose), Some(CostCenterPurpose::Setup));
    assert!(data.components.is_empty());
    assert!(data.app_config.is_none());
    db.cleanup().await;
}

#[tokio::test]
async fn a_new_shared_record_starts_at_version_one_with_a_change_fact() {
    let db = TestDb::new().await;
    let identity = solidbom_server::auth::administrator_identity();
    let material_id = Uuid::new_v4();
    let published = db
        .store
        .create_material_version(&material_version("shared-material", 0, material_id, "1.2", None), &identity.name)
        .await
        .expect("publish material version 1");
    assert_eq!((published.material.version, published.material.supersedes), (1, None));
    assert_eq!(published.material.material_id, material_id);
    assert!(published.material.created_at.is_some());
    assert_eq!(published.change.kind, SharedRecordKind::Material);
    assert_eq!(published.change.previous_version_id, None);
    assert_eq!(published.change.changed_by, "administrator");

    let cost_center_id = Uuid::new_v4();
    let rate = db
        .store
        .create_rate_version(&rate_version("shared-rate", 0, cost_center_id, Some("90"), CostCenterPurpose::Setup), &identity.name)
        .await
        .expect("publish rate version 1");
    assert_eq!((rate.cost_center.version, rate.cost_center.supersedes), (1, None));
    assert_eq!(rate.cost_center.purpose, CostCenterPurpose::Setup);

    let component_id = Uuid::new_v4();
    let component = db
        .store
        .create_component_version(&component_version("shared-component", 0, component_id, Some("0")), &identity.name)
        .await
        .expect("publish component version 1");
    assert_eq!((component.component.version, component.component.supersedes), (1, None));
    assert_eq!(component.component.price, Some(d("0")));

    let config_id = Uuid::new_v4();
    let categories = vec![direct_category("material-cost", DirectCostInput::Material), percentage_category("overhead", true, &[], "0.05")];
    let config = db
        .store
        .create_app_config_version(&app_config_version("shared-config", 0, config_id, categories), &identity.name)
        .await
        .expect("publish app configuration version 1");
    assert_eq!((config.config.version, config.config.supersedes), (1, None));

    let data = db.store.reference_data().await.expect("reference data");
    data.validate().expect("the published data is valid");
    assert_eq!(data.material(published.material.material_version_id).and_then(|material| material.price_per_kg), None);
    assert_eq!(data.rate(rate.cost_center.rate_version_id).and_then(|rate| rate.rate_per_hour), Some(d("90")));
    assert_eq!(data.component(component.component.component_version_id).map(|component| component.component_id), Some(component_id));
    assert_eq!(data.app_config.as_ref().map(|config| config.version), Some(1));

    let view = db.store.shared_data(&identity).await.expect("shared data");
    view.validate().expect("the shared view is valid");
    assert_eq!(view.identity.name, "administrator");
    // The latest configuration and its complete version list are both returned.
    assert_eq!(view.app_config.as_ref().map(|config| config.version), Some(1));
    assert_eq!(view.app_config_versions.len(), 1);
    assert_eq!(view.app_config_versions[0].config_version_id, config.config.config_version_id);
    assert_eq!(view.changes.len(), 4);
    for change in &view.changes {
        assert_eq!(change.previous_version_id, None);
        assert_eq!(change.changed_by, "administrator");
        assert!(!change.summary.is_empty());
    }
    assert!(view.changes.iter().any(|change| change.kind == SharedRecordKind::CostCenter));
    db.cleanup().await;
}

#[tokio::test]
async fn publishing_a_new_version_keeps_the_project_version_and_calculation_bytes_identical() {
    let db = TestDb::new().await;
    let source = upload(&db.store).await;
    let created = create(&db.store, &source).await;
    let project_id = created.draft.project_id;
    let request = CalculateRequest { request_id: "calc-1".into(), project_id, saved_version: 1 };
    let snapshot = db.store.record_calculation(&request).await.expect("calculate version 1");
    let project_before = serde_json::to_vec(&db.store.project_view(project_id).await.expect("project view")).expect("JSON");
    let calculation_before = serde_json::to_vec(&db.store.load_calculation(snapshot.calculation_id).await.expect("calculation")).expect("JSON");

    let identity = solidbom_server::auth::administrator_identity();
    db.store
        .create_material_version(&material_version("shared-material-2", 1, seeded_material_version_id(), "2.7", Some("25")), &identity.name)
        .await
        .expect("publish material version 2");
    db.store
        .create_rate_version(&rate_version("shared-rate-2", 1, seeded_rate_version_id(), Some("150"), CostCenterPurpose::Production), &identity.name)
        .await
        .expect("publish rate version 2");

    let project_after = serde_json::to_vec(&db.store.project_view(project_id).await.expect("project view")).expect("JSON");
    let calculation_after = serde_json::to_vec(&db.store.load_calculation(snapshot.calculation_id).await.expect("calculation")).expect("JSON");
    assert_eq!(project_before, project_after, "a new reference version never edits a project version");
    assert_eq!(calculation_before, calculation_after, "a new reference version never rewrites a calculation snapshot");
    let loaded = db.store.load_calculation(snapshot.calculation_id).await.expect("calculation");
    assert_eq!(loaded.inputs.parts[0].material_price_per_kg, Some(d("20")));
    assert_eq!(loaded.inputs.parts[0].production_rate_per_hour, Some(d("120")));

    // Historical versions stay readable after a newer one is published.
    let data = db.store.reference_data().await.expect("reference data");
    let chain: Vec<_> = data.materials.iter().filter(|material| material.material_id == seeded_material_version_id()).collect();
    assert_eq!(chain.len(), 2);
    assert!(chain.iter().any(|material| material.version == 1 && material.price_per_kg == Some(d("20"))));
    assert!(chain.iter().any(|material| material.version == 2 && material.price_per_kg == Some(d("25"))));
    db.cleanup().await;
}

#[tokio::test]
async fn a_stale_expected_version_and_a_reused_request_id_are_conflicts() {
    let db = TestDb::new().await;
    let identity = solidbom_server::auth::administrator_identity();
    let first = material_version("shared-1", 1, seeded_material_version_id(), "2.7", Some("25"));
    let published = db.store.create_material_version(&first, &identity.name).await.expect("publish material version 2");
    let retried = db.store.create_material_version(&first, &identity.name).await.expect("retry the recorded publish");
    assert_eq!(published, retried);
    assert_eq!(row_count(&db.store, "material_versions").await, 3);
    assert_eq!(row_count(&db.store, "reference_changes").await, 1);
    assert_eq!(row_count(&db.store, "requests").await, 1);

    let mut stale = first.clone();
    stale.request_id = "shared-2".into();
    stale.price_per_kg = Some(d("30"));
    let error = db.store.create_material_version(&stale, &identity.name).await.expect_err("a stale expected version is refused");
    assert!(matches!(error, StoreError::Conflict(_)), "{error}");
    assert_eq!(row_count(&db.store, "material_versions").await, 3);
    assert_eq!(row_count(&db.store, "reference_changes").await, 1);

    let mut reused = first.clone();
    reused.price_per_kg = Some(d("99"));
    let error = db.store.create_material_version(&reused, &identity.name).await.expect_err("a reused request id with another payload conflicts");
    assert!(matches!(error, StoreError::Conflict(_)), "{error}");

    let mut next = stale;
    next.request_id = "shared-3".into();
    next.expected_version = 2;
    let second = db.store.create_material_version(&next, &identity.name).await.expect("publish material version 3");
    assert_eq!(second.material.version, 3);
    assert_eq!(second.material.supersedes, Some(published.material.material_version_id));
    assert_eq!(second.change.previous_version_id, Some(published.material.material_version_id));
    db.cleanup().await;
}

#[tokio::test]
async fn an_invalid_app_configuration_writes_nothing() {
    let db = TestDb::new().await;
    let identity = solidbom_server::auth::administrator_identity();
    let config_id = Uuid::new_v4();
    let invalid = vec![
        vec![percentage_category("overhead", false, &[], "0.05")],
        vec![percentage_category("overhead", false, &["overhead"], "0.05")],
        vec![percentage_category("first", false, &["second"], "0.01"), percentage_category("second", false, &["first"], "0.02")],
        vec![direct_category("material-a", DirectCostInput::Material), direct_category("material-b", DirectCostInput::Material)],
    ];
    for (index, categories) in invalid.into_iter().enumerate() {
        let request = app_config_version(&format!("shared-config-invalid-{index}"), 0, config_id, categories);
        let error = db.store.create_app_config_version(&request, &identity.name).await.expect_err("the invalid configuration is refused");
        assert!(matches!(error, StoreError::Invalid(_)), "{error}");
    }
    assert_eq!(row_count(&db.store, "app_config_versions").await, 0);
    assert_eq!(row_count(&db.store, "reference_changes").await, 0);
    assert_eq!(row_count(&db.store, "requests").await, 0);
    db.cleanup().await;
}

#[tokio::test]
async fn a_non_positive_density_or_foreign_currency_is_refused_before_writing() {
    let db = TestDb::new().await;
    let identity = solidbom_server::auth::administrator_identity();
    let no_density = material_version("shared-density", 0, Uuid::new_v4(), "0", Some("20"));
    let error = db.store.create_material_version(&no_density, &identity.name).await.expect_err("a zero density is refused");
    assert!(matches!(error, StoreError::Invalid(_)), "{error}");
    let mut foreign = material_version("shared-currency", 0, Uuid::new_v4(), "2.7", Some("20"));
    foreign.currency = "EUR".into();
    let error = db.store.create_material_version(&foreign, &identity.name).await.expect_err("a foreign currency is refused");
    assert!(matches!(error, StoreError::Invalid(_)), "{error}");
    assert_eq!(row_count(&db.store, "material_versions").await, 2, "a refused publish wrote nothing");
    assert_eq!(row_count(&db.store, "reference_changes").await, 0);
    assert_eq!(row_count(&db.store, "requests").await, 0);
    db.cleanup().await;
}

#[tokio::test]
async fn a_new_version_with_a_missing_price_reaches_the_calculation_as_missing() {
    let db = TestDb::new().await;
    let identity = solidbom_server::auth::administrator_identity();
    let published = db
        .store
        .create_material_version(&material_version("shared-material-no-price", 1, seeded_material_version_id(), "2.7", None), &identity.name)
        .await
        .expect("publish a material version without a price");
    assert_eq!(published.material.price_per_kg, None);

    // A project selects the exact version id it reviewed, here the new version without a price.
    let source = upload(&db.store).await;
    let mut draft = fixture_draft(0, &source);
    draft.project_id = Uuid::new_v4();
    draft.parts[0].material_version_id = Some(published.material.material_version_id);
    let save = SaveDraft { request_id: "create-no-price".into(), expected_version: 0, draft };
    let receipt = db.store.create_draft(&save).await.expect("create the project");
    let request = CalculateRequest { request_id: "calc-no-price".into(), project_id: receipt.project_id, saved_version: 1 };
    let snapshot = db.store.record_calculation(&request).await.expect("calculate");
    assert_eq!(snapshot.inputs.parts[0].material_price_per_kg, None, "a missing price never becomes zero");
    assert_eq!(snapshot.outputs.parts[0].allowed_material_cost, None);
    assert!(snapshot.issues.iter().any(|issue| issue.code == "missing_material_price"), "{:?}", snapshot.issues);
    db.cleanup().await;
}
