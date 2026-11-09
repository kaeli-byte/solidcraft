//! Store-level tests for project records, deliveries and part versions (contract version 2, SB-10).
//!
//! `#[tokio::test]` is not recognised as a test function by Clippy's test allowance, so this file
//! allows the panicking helpers it needs. The library and binary still deny them.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

mod support;

use std::str::FromStr;

use rust_decimal::Decimal;
use solidbom_contract::{
    AnnualForecast, ConfirmPartMatch, CreateProject, DeliveredPartInput, DeliveredProductInput, DeliveryPackage, ExcludeProducts, Product,
    ProductKind, ProductRevisionState, ProjectDetails, ProjectFact, ProjectRecordView, QuotationState, RecordDelivery, ReinstateProducts, SourceRef,
    UpdateProduct, YearRange,
};
use solidbom_server::Store;
use support::TestDb;
use uuid::Uuid;

fn d(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a decimal fixture")
}

fn project_id() -> Uuid {
    Uuid::from_u128(0x5100)
}

fn details() -> ProjectDetails {
    ProjectDetails {
        name: "Inquiry".into(),
        customer: "Customer".into(),
        internal_team: Some("Team".into()),
        platform: Some("Platform".into()),
        application: None,
        currency: "CNY".into(),
        default_schedule: YearRange { sop_year: 2026, eop_year: 2028 },
    }
}

fn forecast_years() -> Vec<AnnualForecast> {
    vec![AnnualForecast { year: 2026, quantity: 100 }, AnnualForecast { year: 2027, quantity: 200 }, AnnualForecast { year: 2028, quantity: 300 }]
}

fn part(key: &str, number: Option<&str>, revision: Option<&str>, volume: Option<&str>) -> DeliveredPartInput {
    DeliveredPartInput {
        delivered_key: key.into(),
        customer_part_number: number.map(str::to_string),
        customer_revision: revision.map(str::to_string),
        volume_mm3: volume.map(d),
    }
}

fn delivered(name: &str, kind: ProductKind, number: Option<&str>, product_id: Option<Uuid>, parts: Vec<DeliveredPartInput>) -> DeliveredProductInput {
    DeliveredProductInput { product_id, name: name.into(), kind, customer_part_number: number.map(str::to_string), parts }
}

async fn upload(store: &Store, name: &str) -> SourceRef {
    let bytes = format!("ISO-10303-21;\n/* {name} */\nENDSEC;\nEND-ISO-10303-21;\n").into_bytes();
    store.put_source(&support::digest(&bytes), name, &bytes).await.expect("store the package source")
}

async fn view(store: &Store, project_id: Uuid) -> ProjectRecordView {
    store.project_record_view(project_id).await.expect("the project record view")
}

async fn create(store: &Store) -> ProjectRecordView {
    let request = CreateProject { request_id: "create-1".into(), project_id: project_id(), details: details() };
    store.create_project(&request).await.expect("create the project");
    view(store, project_id()).await
}

fn product<'a>(view: &'a ProjectRecordView, name: &str) -> &'a Product {
    view.document.products.iter().find(|product| product.name == name).expect("the named product")
}

async fn set_forecasts(store: &Store, request_id: &str, name: &str) -> u64 {
    let current = view(store, project_id()).await;
    let product = product(&current, name);
    let request = UpdateProduct {
        request_id: request_id.into(),
        expected_version: current.version,
        product_id: product.product_id,
        name: product.name.clone(),
        kind: product.kind,
        customer_part_number: product.customer_part_number.clone(),
        schedule_override: product.schedule_override,
        forecasts: forecast_years(),
    };
    store.update_product(project_id(), &request).await.expect("update the product forecasts").receipt.version
}

async fn delivery(
    store: &Store,
    request_id: &str,
    expected_version: u64,
    package_id: u128,
    name: &str,
    source: SourceRef,
    products: Vec<DeliveredProductInput>,
) -> u64 {
    let request = RecordDelivery {
        request_id: request_id.into(),
        expected_version,
        package: DeliveryPackage { package_id: Uuid::from_u128(package_id), name: name.into(), source },
        products,
    };
    store.record_delivery(project_id(), &request).await.expect("record the delivery").receipt.version
}

#[tokio::test]
async fn a_partial_delivery_keeps_omitted_products_and_the_old_active_revision() {
    let db = TestDb::new().await;
    let store = &db.store;
    let first = create(store).await;
    assert_eq!(first.version, 1);
    assert!(first.document.products.is_empty());

    let package_1 = upload(store, "package-1.step").await;
    let version = delivery(
        store,
        "delivery-1",
        1,
        0x6101,
        "Package 1",
        package_1.clone(),
        vec![
            delivered("A", ProductKind::Assembly, Some("PL-A"), None, vec![part("def-a", Some("PL-100"), Some("B"), Some("2400"))]),
            delivered("B", ProductKind::Assembly, Some("PL-B"), None, vec![part("def-b", Some("PL-200"), Some("B"), Some("2400"))]),
            delivered("C", ProductKind::Assembly, Some("PL-C"), None, vec![part("def-c", Some("PL-300"), Some("B"), Some("2400"))]),
        ],
    )
    .await;
    let after_first = view(store, project_id()).await;
    assert_eq!(after_first.version, version);
    assert_eq!(after_first.document.products.len(), 3);
    assert_eq!(after_first.document.packages.len(), 1);
    assert_eq!(after_first.document.packages[0].source.source_id, package_1.source_id);
    for name in ["A", "B", "C"] {
        let product = product(&after_first, name);
        assert_eq!(product.revisions.len(), 1);
        assert_eq!(product.revisions[0].state, ProductRevisionState::Draft);
        assert_eq!(product.active_revision_id, None);
        assert_eq!(product.revisions[0].package_id, Uuid::from_u128(0x6101));
    }
    // The engineer completes the forecasts and activates A, B and C (SB-19's transition is not a
    // published route; the store records the same transition).
    for (index, name) in ["A", "B", "C"].iter().enumerate() {
        set_forecasts(store, &format!("forecast-{index}"), name).await;
        let current = view(store, project_id()).await;
        let revision = product(&current, name).revisions[0].product_revision_id;
        let activated = store.record_revision_activation(project_id(), revision).await.expect("activate the revision");
        assert_eq!(product(&activated, name).active_revision_id, Some(revision));
    }

    // Package 2 revises B and adds D; A and C are omitted.
    let active = view(store, project_id()).await;
    let a_before = product(&active, "A").clone();
    let b_before = product(&active, "B").clone();
    let c_before = product(&active, "C").clone();
    let expected = active.version;
    let package_2 = upload(store, "package-2.step").await;
    let version = delivery(
        store,
        "delivery-2",
        expected,
        0x6102,
        "Package 2",
        package_2,
        vec![
            delivered(
                "B",
                ProductKind::Assembly,
                Some("PL-B"),
                Some(b_before.product_id),
                vec![part("def-b2", Some("PL-200"), Some("C"), Some("2400"))],
            ),
            delivered("D", ProductKind::Assembly, Some("PL-D"), None, vec![part("def-d", Some("PL-400"), Some("A"), Some("2400"))]),
        ],
    )
    .await;
    assert_eq!(version, expected + 1);
    let after_second = view(store, project_id()).await;
    assert_eq!(after_second.document.products.len(), 4);
    assert_eq!(after_second.document.packages.len(), 2);

    let b_after = product(&after_second, "B");
    assert_eq!(b_after.revisions.len(), 2);
    let old = &b_after.revisions[0];
    let new = &b_after.revisions[1];
    assert_eq!(old.product_revision_id, b_before.revisions[0].product_revision_id);
    assert_eq!(old.package_id, Uuid::from_u128(0x6101));
    assert_eq!(old.state, ProductRevisionState::Active);
    assert_eq!(b_after.active_revision_id, Some(old.product_revision_id));
    assert_eq!(new.state, ProductRevisionState::Draft);
    assert_eq!(new.package_id, Uuid::from_u128(0x6102));

    let a_after = product(&after_second, "A");
    let c_after = product(&after_second, "C");
    assert_eq!(a_after, &a_before);
    assert_eq!(c_after, &c_before);

    let d_after = product(&after_second, "D");
    assert_eq!(d_after.revisions.len(), 1);
    assert_eq!(d_after.revisions[0].state, ProductRevisionState::Draft);
    assert_eq!(d_after.active_revision_id, None);
    assert!(
        after_second.issues.iter().any(|issue| issue.code == "missing_forecast" && issue.product_id == Some(d_after.product_id)),
        "D has no forecasts and stays incomplete"
    );

    // The retained versions keep the earlier state and definitions.
    let version_one = store.project_record_version(project_id(), 1).await.expect("the first version");
    assert!(version_one.document.products.is_empty());
    let version_two = store.project_record_version(project_id(), 2).await.expect("the delivery version");
    assert_eq!(product(&version_two, "B").revisions.len(), 1);
    assert_eq!(product(&version_two, "B").revisions[0].state, ProductRevisionState::Draft);
}

#[tokio::test]
async fn customer_revisions_stay_separate_and_conflicting_geometry_needs_confirmation() {
    let db = TestDb::new().await;
    let store = &db.store;
    create(store).await;
    let package_1 = upload(store, "revisions.step").await;
    let version = delivery(
        store,
        "delivery-1",
        1,
        0x6201,
        "Revisions",
        package_1,
        vec![delivered(
            "A",
            ProductKind::Part,
            Some("PL-100"),
            None,
            vec![
                part("def-b", Some("PL-100"), Some("B"), Some("2400")),
                part("def-c", Some("PL-100"), Some("C"), Some("2400")),
                // A repeated shared identity across two products shares one part version.
                part("def-b-copy", Some("PL-100"), Some("B"), Some("2400")),
            ],
        )],
    )
    .await;
    let mut first = view(store, project_id()).await;
    assert_eq!(first.version, version);
    assert_eq!(first.document.part_versions.len(), 2, "B and C are separate part versions");
    let b_part = first.document.part_versions.iter().find(|part| part.customer_revision == "B").expect("the B part version").clone();
    let c_part = first.document.part_versions.iter().find(|part| part.customer_revision == "C").expect("the C part version").clone();
    assert_ne!(b_part.part_version_id, c_part.part_version_id);
    let delivered_parts = &product(&first, "A").revisions[0].parts;
    assert_eq!(delivered_parts[0].part_version_id, Some(b_part.part_version_id));
    assert_eq!(delivered_parts[1].part_version_id, Some(c_part.part_version_id));
    assert_eq!(delivered_parts[2].part_version_id, Some(b_part.part_version_id), "the same identity shares one version");
    set_forecasts(store, "forecast-a", "A").await;
    first = view(store, project_id()).await;
    assert!(first.issues.is_empty(), "the delivery has no pending match and every forecast year is complete");

    // Geometry inside the relative tolerance validates the identity and keeps the same version.
    let within = upload(store, "within-tolerance.step").await;
    delivery(
        store,
        "delivery-2",
        first.version,
        0x6202,
        "Within tolerance",
        within,
        vec![delivered(
            "A",
            ProductKind::Part,
            Some("PL-100"),
            Some(product(&first, "A").product_id),
            vec![part("def-b2", Some("PL-100"), Some("B"), Some("2401"))],
        )],
    )
    .await;
    let second = view(store, project_id()).await;
    assert_eq!(product(&second, "A").revisions[1].parts[0].part_version_id, Some(b_part.part_version_id));

    // Geometry outside the tolerance leaves the delivered part unassociated and asks for review.
    let conflict = upload(store, "conflict.step").await;
    delivery(
        store,
        "delivery-3",
        second.version,
        0x6203,
        "Conflict",
        conflict,
        vec![delivered(
            "A",
            ProductKind::Part,
            Some("PL-100"),
            Some(product(&second, "A").product_id),
            vec![part("def-b3", Some("PL-100"), Some("B"), Some("4800"))],
        )],
    )
    .await;
    let third = view(store, project_id()).await;
    assert_eq!(third.document.part_versions.len(), 2, "a conflicting delivery never merges or invents a version");
    let pending = &product(&third, "A").revisions[2].parts[0];
    assert_eq!(pending.part_version_id, None);
    assert!(third.issues.iter().any(|issue| issue.code == "conflicting_geometry_match"));

    // The engineer confirms the match explicitly; the association is retained across a reopen.
    let current = view(store, project_id()).await;
    let request = ConfirmPartMatch {
        request_id: "match-1".into(),
        expected_version: current.version,
        product_revision_id: current.document.products[0].revisions[2].product_revision_id,
        delivered_key: "def-b3".into(),
        part_version_id: b_part.part_version_id,
    };
    let outcome = store.confirm_part_match(project_id(), &request).await.expect("confirm the match");
    assert!(matches!(outcome.facts.as_slice(), [ProjectFact::PartMatchConfirmed { .. }]));
    let after = view(store, project_id()).await;
    assert_eq!(product(&after, "A").revisions[2].parts[0].part_version_id, Some(b_part.part_version_id));
    assert!(!after.issues.iter().any(|issue| issue.code == "conflicting_geometry_match"));
    let reopened = view(store, project_id()).await;
    assert_eq!(product(&reopened, "A").revisions[2].parts[0].part_version_id, Some(b_part.part_version_id));
    assert_eq!(reopened.document.part_versions.len(), 2);
}

#[tokio::test]
async fn exclusion_retains_files_history_and_emits_a_scope_fact() {
    let db = TestDb::new().await;
    let store = &db.store;
    create(store).await;
    let package_1 = upload(store, "scope.step").await;
    delivery(
        store,
        "delivery-1",
        1,
        0x6301,
        "Scope package",
        package_1,
        vec![
            delivered("A", ProductKind::Assembly, Some("PL-A"), None, vec![part("def-a", Some("PL-100"), Some("B"), Some("2400"))]),
            delivered("B", ProductKind::Assembly, Some("PL-B"), None, vec![part("def-b", Some("PL-200"), Some("B"), Some("2400"))]),
        ],
    )
    .await;
    let before = view(store, project_id()).await;
    let b_id = product(&before, "B").product_id;

    let request = ExcludeProducts { request_id: "exclude-1".into(), expected_version: before.version, product_ids: vec![b_id] };
    let outcome = store.exclude_products(project_id(), &request).await.expect("exclude the product");
    assert!(matches!(
        outcome.facts.as_slice(),
        [ProjectFact::QuotationScopeChanged { product_ids, state }] if product_ids == &vec![b_id] && *state == QuotationState::Excluded
    ));
    let excluded = view(store, project_id()).await;
    assert_eq!(product(&excluded, "B").quotation_state, QuotationState::Excluded);
    assert_eq!(product(&excluded, "A").quotation_state, QuotationState::Included);
    assert_eq!(excluded.document.packages.len(), 1, "the package and its file stay");
    assert_eq!(excluded.document.packages[0].source.sha256, before.document.packages[0].source.sha256);
    assert_eq!(product(&excluded, "B").revisions, product(&before, "B").revisions, "the revision history stays");
    assert_eq!(excluded.document.quotation_events.len(), 1);
    assert_eq!(excluded.document.quotation_events[0].state, QuotationState::Excluded);

    // The earlier retained version still shows B as included.
    let earlier = store.project_record_version(project_id(), before.version).await.expect("the earlier version");
    assert_eq!(product(&earlier, "B").quotation_state, QuotationState::Included);
    assert!(earlier.document.quotation_events.is_empty());

    let request = ReinstateProducts { request_id: "reinstate-1".into(), expected_version: excluded.version, product_ids: vec![b_id] };
    store.reinstate_products(project_id(), &request).await.expect("reinstate the product");
    let reinstated = view(store, project_id()).await;
    assert_eq!(product(&reinstated, "B").quotation_state, QuotationState::Included);
    assert_eq!(reinstated.document.quotation_events.len(), 2);
    assert_eq!(reinstated.document.quotation_events[0].state, QuotationState::Excluded);
    assert_eq!(reinstated.document.quotation_events[1].state, QuotationState::Included);
}

#[tokio::test]
async fn missing_forecasts_stay_incomplete_until_edited() {
    let db = TestDb::new().await;
    let store = &db.store;
    create(store).await;
    let package_1 = upload(store, "forecasts.step").await;
    delivery(
        store,
        "delivery-1",
        1,
        0x6401,
        "Forecast package",
        package_1,
        vec![delivered("D", ProductKind::Assembly, Some("PL-D"), None, vec![part("def-d", Some("PL-400"), Some("A"), Some("2400"))])],
    )
    .await;
    let incomplete = view(store, project_id()).await;
    let d_id = product(&incomplete, "D").product_id;
    let missing: Vec<&str> = incomplete.issues.iter().filter(|issue| issue.product_id == Some(d_id)).map(|issue| issue.code.as_str()).collect();
    assert_eq!(missing, vec!["missing_forecast", "missing_forecast", "missing_forecast"], "each production year is named");

    set_forecasts(store, "forecast-d", "D").await;
    let complete = view(store, project_id()).await;
    assert!(!complete.issues.iter().any(|issue| issue.product_id == Some(d_id)), "the completed years clear D's issues");
}

#[tokio::test]
async fn writes_are_version_checked_and_exactly_once() {
    let db = TestDb::new().await;
    let store = &db.store;
    let create_request = CreateProject { request_id: "create-1".into(), project_id: project_id(), details: details() };
    let first = store.create_project(&create_request).await.expect("create");
    let retry = store.create_project(&create_request).await.expect("an identical retry answers from the request log");
    assert_eq!(first, retry);
    assert_eq!(store.project_record_version_count(project_id()).await.expect("count"), 1);
    let changed =
        CreateProject { request_id: "create-1".into(), project_id: project_id(), details: ProjectDetails { name: "Other".into(), ..details() } };
    assert!(store.create_project(&changed).await.is_err(), "a reused request id with another payload is a conflict");

    let package = upload(store, "exactly-once.step").await;
    let request = RecordDelivery {
        request_id: "delivery-1".into(),
        expected_version: 1,
        package: DeliveryPackage { package_id: Uuid::from_u128(0x6501), name: "Exactly once".into(), source: package },
        products: vec![delivered("A", ProductKind::Part, Some("PL-A"), None, vec![part("def-a", Some("PL-100"), Some("B"), Some("2400"))])],
    };
    let committed = store.record_delivery(project_id(), &request).await.expect("deliver");
    let replayed = store.record_delivery(project_id(), &request).await.expect("the retry answers from the request log");
    assert_eq!(committed, replayed);
    assert_eq!(store.project_record_version_count(project_id()).await.expect("count"), 2, "no duplicate version");

    let stale = RecordDelivery {
        request_id: "delivery-stale".into(),
        expected_version: 1,
        package: DeliveryPackage { package_id: Uuid::from_u128(0x6502), name: "Stale".into(), source: upload(store, "stale.step").await },
        products: vec![delivered("B", ProductKind::Part, Some("PL-B"), None, vec![])],
    };
    assert!(store.record_delivery(project_id(), &stale).await.is_err(), "a stale expected version is a conflict");
    let unchanged = view(store, project_id()).await;
    assert_eq!(unchanged.version, 2);
    assert_eq!(unchanged.document.products.len(), 1);
}

#[tokio::test]
async fn a_failed_delivery_does_not_erase_successful_products() {
    let db = TestDb::new().await;
    let store = &db.store;
    create(store).await;
    let package_1 = upload(store, "failed-step-1.step").await;
    delivery(
        store,
        "delivery-1",
        1,
        0x6601,
        "First",
        package_1,
        vec![delivered("A", ProductKind::Assembly, Some("PL-900"), None, vec![part("def-a", Some("PL-100"), Some("B"), Some("2400"))])],
    )
    .await;
    let after_first = view(store, project_id()).await;
    let a_id = product(&after_first, "A").product_id;

    // A second product shares the same customer part number, which makes a later number-only
    // match ambiguous.
    let current = view(store, project_id()).await;
    let package_2 = upload(store, "failed-step-2.step").await;
    let version = delivery(
        store,
        "delivery-2",
        current.version,
        0x6602,
        "Second",
        package_2,
        vec![delivered("B", ProductKind::Assembly, None, None, vec![part("def-b", Some("PL-200"), Some("B"), Some("2400"))])],
    )
    .await;
    let after_second = view(store, project_id()).await;
    assert_eq!(after_second.version, version);
    let b_id = product(&after_second, "B").product_id;
    let request = UpdateProduct {
        request_id: "b-number".into(),
        expected_version: after_second.version,
        product_id: b_id,
        name: "B".into(),
        kind: ProductKind::Assembly,
        customer_part_number: Some("PL-900".into()),
        schedule_override: None,
        forecasts: Vec::new(),
    };
    store.update_product(project_id(), &request).await.expect("B takes the shared customer number");

    // The ambiguous delivery is refused atomically, leaving A and B exactly as they were.
    let before_failure = view(store, project_id()).await;
    let package_3 = upload(store, "failed-step-3.step").await;
    let ambiguous = RecordDelivery {
        request_id: "delivery-ambiguous".into(),
        expected_version: before_failure.version,
        package: DeliveryPackage { package_id: Uuid::from_u128(0x6603), name: "Ambiguous".into(), source: package_3 },
        products: vec![delivered("X", ProductKind::Assembly, Some("PL-900"), None, vec![part("def-x", Some("PL-100"), Some("B"), Some("2400"))])],
    };
    assert!(store.record_delivery(project_id(), &ambiguous).await.is_err());
    let after_failure = view(store, project_id()).await;
    assert_eq!(after_failure.version, before_failure.version, "no version was written");
    assert_eq!(after_failure.document, before_failure.document, "no successful product changed");
    assert_eq!(after_failure.document.packages.len(), 2);
    assert_eq!(product(&after_failure, "A").product_id, a_id);

    // A delivery whose source was never uploaded is refused too, without erasing A or B.
    let missing_source = SourceRef { source_id: Uuid::from_u128(0x66ff), sha256: "b".repeat(64), byte_len: 12, original_name: "missing.step".into() };
    let missing = RecordDelivery {
        request_id: "delivery-missing-source".into(),
        expected_version: after_failure.version,
        package: DeliveryPackage { package_id: Uuid::from_u128(0x6604), name: "Missing source".into(), source: missing_source },
        products: vec![delivered("Y", ProductKind::Part, Some("PL-Y"), None, vec![])],
    };
    assert!(store.record_delivery(project_id(), &missing).await.is_err());
    assert_eq!(view(store, project_id()).await.version, after_failure.version);
}

#[tokio::test]
async fn excluded_products_keep_their_part_versions_and_events_in_history() {
    let db = TestDb::new().await;
    let store = &db.store;
    create(store).await;
    let package_1 = upload(store, "history.step").await;
    delivery(
        store,
        "delivery-1",
        1,
        0x6701,
        "History",
        package_1,
        vec![delivered("A", ProductKind::Part, Some("PL-A"), None, vec![part("def-a", Some("PL-100"), Some("B"), Some("2400"))])],
    )
    .await;
    let before = view(store, project_id()).await;
    let a_id = product(&before, "A").product_id;
    let request = ExcludeProducts { request_id: "exclude-1".into(), expected_version: before.version, product_ids: vec![a_id] };
    store.exclude_products(project_id(), &request).await.expect("exclude");
    let excluded = view(store, project_id()).await;
    assert_eq!(excluded.document.part_versions.len(), before.document.part_versions.len());
    assert_eq!(excluded.document.products[0].revisions, before.document.products[0].revisions);
    for version in 1..=excluded.version {
        assert!(store.project_record_version(project_id(), version).await.is_ok(), "every retained version stays readable");
    }
}
