//! PostgreSQL integration tests for the SolidBOM store.
//!
//! Each test runs in its own schema on `SOLIDBOM_TEST_DATABASE_URL` and drops it afterwards. The
//! tests never reset the database they connect to.

mod support;

use solidbom_contract::{CalculateRequest, SaveDraft, SourceRef};
use solidbom_server::StoreError;
use support::{FIXTURE_SOURCE, TestDb, create, d, digest, fixture_draft, row_count, two_revision_draft, upload};
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
    assert_eq!(material.price_per_kg, d("20"));
    assert!(data.rates.iter().any(|rate| rate.rate_per_hour == d("120")));
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
