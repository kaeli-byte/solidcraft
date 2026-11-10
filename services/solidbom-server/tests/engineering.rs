//! Store-level tests for engineering review and flat grouping (contract version 1, package SB-11).
//!
//! `#[tokio::test]` is not recognised as a test function by Clippy's test allowance, so this file
//! allows the panicking helpers it needs. The library and binary still deny them.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

mod support;

use solidbom_contract::{
    AnalysisKey, AnalysisManifest, AssociateComponentPart, CatalogConfirmation, CatalogMatchSuggestion, ClassificationConfirmation,
    ClassificationPrediction, ClassificationSuggestion, CompleteMeasurement, ConfirmParts, CreateProject, DefinitionRef, DeliveredPartInput,
    DeliveredProductInput, DeliveryPackage, EngineeringFact, EngineeringRecordView, GeometryBinding, GroupSolids, Issue, Measurement,
    MeasurementSource, OccurrenceRef, PartConfirmation, ProductKind, ProjectCatalogOverride, ProjectDetails, RecordDelivery, RecordSuggestions,
    RegisterAnalysis, ReviewState, ReviewedGeometry, SCHEMA_VERSION, SuggestionEvidence, Unit, YearRange,
};
use solidbom_server::{Store, StoreError};
use support::TestDb;
use uuid::Uuid;

const SOURCE: &[u8] = b"ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('flat multi-body fixture'),'2;1');\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n";
const IDENTITY: [[f64; 4]; 4] = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
const DEFINITION: u128 = 0xdef1;
const OCCURRENCE: u128 = 0x0cc0;
fn project_id() -> Uuid {
    Uuid::from_u128(0xb111)
}

fn details() -> ProjectDetails {
    ProjectDetails {
        name: "Review inquiry".into(),
        customer: "Customer".into(),
        internal_team: None,
        platform: None,
        application: None,
        currency: "CNY".into(),
        default_schedule: YearRange { sop_year: 2026, eop_year: 2028 },
    }
}

fn key() -> AnalysisKey {
    AnalysisKey { source_sha256: support::digest(SOURCE), engine_build: "0.40.0-rc.2".into(), extractor_version: "2".into() }
}

fn body(source_body: u32) -> ReviewedGeometry {
    ReviewedGeometry { source_path: Vec::new(), source_body }
}

/// The flat fixture's exact analysis: one definition with three bodies, no usable hierarchy, which
/// is exactly the case the engineer reviews as flat components.
fn flat_manifest(key: AnalysisKey) -> AnalysisManifest {
    AnalysisManifest {
        schema_version: SCHEMA_VERSION,
        key: key.clone(),
        definitions: vec![DefinitionRef {
            definition_id: Uuid::from_u128(DEFINITION),
            source_key: key.clone(),
            customer_part_number: None,
            customer_revision: None,
            volume_mm3: Measurement {
                value: None,
                unit: Unit::MillimetreCubed,
                source: MeasurementSource::Extracted,
                review: ReviewState::Pending,
                reason: None,
                analysis_key: key.clone(),
            },
        }],
        occurrences: vec![OccurrenceRef {
            occurrence_id: Uuid::from_u128(OCCURRENCE),
            definition_id: Uuid::from_u128(DEFINITION),
            parent_occurrence_id: None,
            source_path: Vec::new(),
            transform: IDENTITY,
        }],
        geometry_bindings: (0u32..3)
            .map(|source_body| GeometryBinding {
                source_path: Vec::new(),
                source_body,
                definition_id: Uuid::from_u128(DEFINITION),
                occurrence_id: Uuid::from_u128(OCCURRENCE),
            })
            .collect(),
        issues: vec![Issue::new("unresolved_grouping", "analysis.definitions", "the flat input needs a component review")],
    }
}

fn manual_volume(value: &str) -> Measurement {
    Measurement {
        value: Some(support::d(value)),
        unit: Unit::MillimetreCubed,
        source: MeasurementSource::Manual,
        review: ReviewState::Pending,
        reason: Some("open surface; measured on the bench".into()),
        analysis_key: key(),
    }
}

fn delivered_product(name: &str) -> DeliveredProductInput {
    DeliveredProductInput {
        product_id: None,
        name: name.into(),
        kind: ProductKind::Part,
        customer_part_number: None,
        parts: vec![DeliveredPartInput {
            delivered_key: "body-set".into(),
            customer_part_number: Some("PL-100".into()),
            customer_revision: Some("B".into()),
            volume_mm3: Some(support::d("2400")),
        }],
    }
}

/// Create a project with one delivery of two products whose delivered parts share one customer
/// identity and geometry, so they share exactly one project part version.
async fn setup(store: &Store) -> (Uuid, Vec<Uuid>, Uuid) {
    store
        .create_project(&CreateProject { request_id: "create-1".into(), project_id: project_id(), details: details() })
        .await
        .expect("create the project");
    let source = store.put_source(&support::digest(SOURCE), "flat_multi_body.step", SOURCE).await.expect("store the source");
    store
        .record_delivery(
            project_id(),
            &RecordDelivery {
                request_id: "delivery-1".into(),
                expected_version: 1,
                package: DeliveryPackage { package_id: Uuid::from_u128(0xb112), name: "Package 1".into(), source },
                products: vec![delivered_product("A"), delivered_product("B")],
            },
        )
        .await
        .expect("record the delivery");
    let view = store.project_record_view(project_id()).await.expect("the project record view");
    assert_eq!(view.document.part_versions.len(), 1, "the two products share one part version");
    let part_version_id = view.document.part_versions[0].part_version_id;
    let revisions = view.document.products.iter().flat_map(|product| product.revisions.iter().map(|revision| revision.product_revision_id)).collect();
    (project_id(), revisions, part_version_id)
}

async fn register(store: &Store, request_id: &str, revision: Uuid, manifest: AnalysisManifest) {
    store
        .register_analysis(project_id(), &RegisterAnalysis { request_id: request_id.into(), product_revision_id: revision, manifest })
        .await
        .expect("register the exact analysis");
}

async fn view(store: &Store) -> EngineeringRecordView {
    store.engineering_record_view(project_id()).await.expect("the engineering review view")
}

fn codes(view: &EngineeringRecordView) -> Vec<&str> {
    view.issues.iter().map(|issue| issue.code.as_str()).collect()
}

#[tokio::test]
async fn a_registered_analysis_is_exact_immutable_evidence() {
    let db = TestDb::new().await;
    let store = &db.store;
    let (project, revisions, _) = setup(store).await;
    let manifest = flat_manifest(key());
    let receipt = store
        .register_analysis(
            project,
            &RegisterAnalysis { request_id: "analysis-1".into(), product_revision_id: revisions[0], manifest: manifest.clone() },
        )
        .await
        .expect("register the analysis");
    assert_eq!(receipt.analysis_key, key());

    // The same request id is an idempotent retry, not a second registration.
    let again = store
        .register_analysis(
            project,
            &RegisterAnalysis { request_id: "analysis-1".into(), product_revision_id: revisions[0], manifest: manifest.clone() },
        )
        .await
        .expect("the idempotent retry");
    assert_eq!(again, receipt);

    // A different manifest under the same source/engine/extractor identity is a conflict.
    let mut changed = manifest.clone();
    changed.issues.clear();
    let conflict = store
        .register_analysis(project, &RegisterAnalysis { request_id: "analysis-2".into(), product_revision_id: revisions[0], manifest: changed })
        .await
        .expect_err("a changed manifest under the same identity");
    assert!(matches!(conflict, StoreError::Conflict(_)), "{conflict:?}");

    // A review against a different source hash than the delivered package is refused.
    let mut foreign = flat_manifest(key());
    foreign.key.source_sha256 = support::digest(b"another source");
    let foreign = store
        .register_analysis(project, &RegisterAnalysis { request_id: "analysis-3".into(), product_revision_id: revisions[0], manifest: foreign })
        .await
        .expect_err("a foreign source hash");
    assert!(matches!(foreign, StoreError::Invalid(_)), "{foreign:?}");

    // A revision that is not in this project is refused.
    let unknown = store
        .register_analysis(
            project,
            &RegisterAnalysis { request_id: "analysis-4".into(), product_revision_id: Uuid::from_u128(0xdead), manifest: flat_manifest(key()) },
        )
        .await
        .expect_err("an unknown product revision");
    assert!(matches!(unknown, StoreError::Invalid(_)), "{unknown:?}");

    // With no reviewed grouping yet, the flat guard reports the unassociated bodies.
    let review = view(store).await;
    assert_eq!(review.version, 0);
    assert!(codes(&review).contains(&"unresolved_grouping"), "{:?}", review.issues);
}

#[tokio::test]
async fn a_flat_review_groups_several_solids_and_never_leaves_a_body_unassigned() {
    let db = TestDb::new().await;
    let store = &db.store;
    let (project, revisions, part_version_id) = setup(store).await;
    let revision = revisions[0];
    register(store, "analysis-1", revision, flat_manifest(key())).await;

    // Two solids are confirmed as one component and associated with the shared part version.
    let outcome = store
        .group_solids(
            project,
            &GroupSolids {
                request_id: "group-1".into(),
                expected_version: 0,
                component_id: Uuid::from_u128(0xc001),
                product_revision_id: revision,
                analysis_key: key(),
                geometry: vec![body(0), body(1)],
                part_version_id: Some(part_version_id),
            },
        )
        .await
        .expect("group two solids into one component");
    assert_eq!(outcome.receipt.version, 1);
    assert!(matches!(outcome.facts.as_slice(), [EngineeringFact::ComponentGrouped { geometry: 2, .. }]));

    // The remaining body is a second component without a confirmed part identity.
    let outcome = store
        .group_solids(
            project,
            &GroupSolids {
                request_id: "group-2".into(),
                expected_version: 1,
                component_id: Uuid::from_u128(0xc002),
                product_revision_id: revision,
                analysis_key: key(),
                geometry: vec![body(2)],
                part_version_id: None,
            },
        )
        .await
        .expect("group the remaining body");
    assert_eq!(outcome.receipt.version, 2);

    let review = view(store).await;
    assert_eq!(review.document.components.len(), 2);
    assert!(!codes(&review).contains(&"unresolved_grouping"), "every body is associated now: {:?}", review.issues);
    assert!(codes(&review).contains(&"unresolved_component_association"), "the part identity is still unresolved");

    // A body may belong to only one reviewed component: regrouping means rewriting the owner.
    let overlap = store
        .group_solids(
            project,
            &GroupSolids {
                request_id: "group-3".into(),
                expected_version: 2,
                component_id: Uuid::from_u128(0xc003),
                product_revision_id: revision,
                analysis_key: key(),
                geometry: vec![body(0)],
                part_version_id: None,
            },
        )
        .await
        .expect_err("a body already owned by another component");
    assert!(matches!(overlap, StoreError::Invalid(_)), "{overlap:?}");

    // A body that is not in the registered analysis is refused.
    let unknown_body = store
        .group_solids(
            project,
            &GroupSolids {
                request_id: "group-4".into(),
                expected_version: 2,
                component_id: Uuid::from_u128(0xc004),
                product_revision_id: revision,
                analysis_key: key(),
                geometry: vec![body(9)],
                part_version_id: None,
            },
        )
        .await
        .expect_err("a body the analysis does not contain");
    assert!(matches!(unknown_body, StoreError::Invalid(_)), "{unknown_body:?}");

    // Dissolving the reviewed component makes its bodies unassociated again.
    store
        .group_solids(
            project,
            &GroupSolids {
                request_id: "group-5".into(),
                expected_version: 2,
                component_id: Uuid::from_u128(0xc001),
                product_revision_id: revision,
                analysis_key: key(),
                geometry: Vec::new(),
                part_version_id: None,
            },
        )
        .await
        .expect("dissolve the reviewed component");
    let review = view(store).await;
    assert_eq!(review.version, 3);
    assert_eq!(review.document.components.len(), 1);
    assert!(codes(&review).contains(&"unresolved_grouping"), "the dissolved bodies are unassociated again");
}

#[tokio::test]
async fn a_component_is_bound_to_one_exact_analysis_and_one_revision() {
    let db = TestDb::new().await;
    let store = &db.store;
    let (project, revisions, part_version_id) = setup(store).await;
    let revision = revisions[0];
    register(store, "analysis-1", revision, flat_manifest(key())).await;
    let mut later_key = key();
    later_key.extractor_version = "3".into();
    register(store, "analysis-2", revision, flat_manifest(later_key.clone())).await;

    store
        .group_solids(
            project,
            &GroupSolids {
                request_id: "group-1".into(),
                expected_version: 0,
                component_id: Uuid::from_u128(0xc001),
                product_revision_id: revision,
                analysis_key: key(),
                geometry: vec![body(0)],
                part_version_id: Some(part_version_id),
            },
        )
        .await
        .expect("group against the first analysis");

    // The same component cannot be silently rebound to another extraction.
    let rebound = store
        .group_solids(
            project,
            &GroupSolids {
                request_id: "group-2".into(),
                expected_version: 1,
                component_id: Uuid::from_u128(0xc001),
                product_revision_id: revision,
                analysis_key: later_key,
                geometry: vec![body(0)],
                part_version_id: Some(part_version_id),
            },
        )
        .await
        .expect_err("a changed analysis identity");
    assert!(matches!(rebound, StoreError::Invalid(_)), "{rebound:?}");

    // A component cannot move to another product revision.
    let moved = store
        .group_solids(
            project,
            &GroupSolids {
                request_id: "group-3".into(),
                expected_version: 1,
                component_id: Uuid::from_u128(0xc001),
                product_revision_id: revisions[1],
                analysis_key: key(),
                geometry: vec![body(0)],
                part_version_id: Some(part_version_id),
            },
        )
        .await
        .expect_err("a component moved to another revision");
    assert!(matches!(moved, StoreError::Invalid(_)), "{moved:?}");
}

#[tokio::test]
async fn a_component_cannot_merge_different_definitions_into_one_component() {
    let db = TestDb::new().await;
    let store = &db.store;
    let (project, revisions, _) = setup(store).await;
    let revision = revisions[0];
    let mut manifest = flat_manifest(key());
    let second_definition = Uuid::from_u128(0xdef2);
    let mut second = manifest.definitions[0].clone();
    second.definition_id = second_definition;
    manifest.definitions.push(second);
    let mut binding = manifest.geometry_bindings[0].clone();
    binding.definition_id = second_definition;
    binding.source_body = 5;
    manifest.geometry_bindings.push(binding);
    register(store, "analysis-1", revision, manifest).await;

    let merged = store
        .group_solids(
            project,
            &GroupSolids {
                request_id: "group-1".into(),
                expected_version: 0,
                component_id: Uuid::from_u128(0xc001),
                product_revision_id: revision,
                analysis_key: key(),
                geometry: vec![body(0), ReviewedGeometry { source_path: Vec::new(), source_body: 5 }],
                part_version_id: None,
            },
        )
        .await
        .expect_err("merging two definitions");
    assert!(matches!(merged, StoreError::Invalid(_)), "{merged:?}");
}

#[tokio::test]
async fn an_open_surface_volume_stays_missing_until_completed_and_confirmed() {
    let db = TestDb::new().await;
    let store = &db.store;
    let (project, revisions, part_version_id) = setup(store).await;
    let revision = revisions[0];
    register(store, "analysis-1", revision, flat_manifest(key())).await;
    store
        .group_solids(
            project,
            &GroupSolids {
                request_id: "group-1".into(),
                expected_version: 0,
                component_id: Uuid::from_u128(0xc001),
                product_revision_id: revision,
                analysis_key: key(),
                geometry: vec![body(0), body(1), body(2)],
                part_version_id: Some(part_version_id),
            },
        )
        .await
        .expect("group the flat component");

    // No volume is recorded: the part is uncosted, not zero-costed.
    let review = view(store).await;
    assert!(codes(&review).contains(&"missing_reviewed_volume"), "{:?}", review.issues);
    assert!(codes(&review).contains(&"pending_classification"), "{:?}", review.issues);

    // A volume cannot be confirmed before it is completed.
    let missing = store
        .confirm_parts(
            project,
            &ConfirmParts {
                request_id: "confirm-1".into(),
                expected_version: 1,
                confirmations: vec![PartConfirmation { part_version_id, classification: None, confirm_measurement: true, catalog: None }],
            },
            "engineer",
        )
        .await
        .expect_err("confirming a missing volume");
    assert!(matches!(missing, StoreError::Invalid(_)), "{missing:?}");

    // A completion cannot claim its own review.
    let mut claimed = manual_volume("2500");
    claimed.review = ReviewState::Confirmed;
    let claimed = store
        .complete_measurement(
            project,
            &CompleteMeasurement { request_id: "measure-0".into(), expected_version: 1, part_version_id, volume_mm3: claimed },
        )
        .await
        .expect_err("a completion claiming its own review");
    assert!(matches!(claimed, StoreError::Invalid(_)), "{claimed:?}");

    // A volume from an unrelated analysis is refused: it does not describe the reviewed geometry.
    let mut foreign = manual_volume("2500");
    foreign.analysis_key.extractor_version = "999".into();
    let foreign = store
        .complete_measurement(
            project,
            &CompleteMeasurement { request_id: "measure-foreign".into(), expected_version: 1, part_version_id, volume_mm3: foreign },
        )
        .await
        .expect_err("a volume from an analysis that describes no reviewed component");
    assert!(matches!(foreign, StoreError::Invalid(_)), "{foreign:?}");

    // The engineer completes the open surface manually with its reason; it stays pending.
    let outcome = store
        .complete_measurement(
            project,
            &CompleteMeasurement { request_id: "measure-1".into(), expected_version: 1, part_version_id, volume_mm3: manual_volume("2500") },
        )
        .await
        .expect("complete the measurement");
    assert_eq!(outcome.receipt.version, 2);
    assert!(matches!(
        outcome.facts.as_slice(),
        [EngineeringFact::MeasurementRecorded { source: MeasurementSource::Manual, review: ReviewState::Pending, .. }]
    ));
    let review = view(store).await;
    assert!(codes(&review).contains(&"pending_measurement_review"), "{:?}", review.issues);
    assert!(!codes(&review).contains(&"missing_reviewed_volume"), "{:?}", review.issues);

    // The bulk confirmation accepts the label and reviews the volume.
    let outcome = store
        .confirm_parts(
            project,
            &ConfirmParts {
                request_id: "confirm-2".into(),
                expected_version: 2,
                confirmations: vec![PartConfirmation {
                    part_version_id,
                    classification: Some(ClassificationConfirmation { name: "clip".into() }),
                    confirm_measurement: true,
                    catalog: None,
                }],
            },
            "engineer",
        )
        .await
        .expect("confirm the reviewed part");
    assert_eq!(outcome.receipt.version, 3);
    assert!(matches!(
        outcome.facts.as_slice(),
        [
            EngineeringFact::PartConfirmed { classification_confirmed: true, measurement_confirmed: true, .. },
            EngineeringFact::TrainingExamplesCaptured { count: 1, .. }
        ]
    ));
    let review = view(store).await;
    assert!(review.issues.is_empty(), "{:?}", review.issues);
    let part = &review.document.parts[0];
    assert_eq!(part.volume_mm3.as_ref().map(|volume| volume.review), Some(ReviewState::Confirmed));
    assert_eq!(part.volume_mm3.as_ref().and_then(|volume| volume.value), Some(support::d("2500")));
    assert_eq!(part.volume_confirmed_by.as_deref(), Some("engineer"));
    let retained = store.engineering_record_version(project, 3).await.expect("the retained confirmation version");
    assert_eq!(retained.version, 3);
    assert_eq!(retained.document.parts[0].volume_confirmed_by.as_deref(), Some("engineer"), "the reviewer stays readable in history");
    assert_eq!(
        retained.document.parts[0].volume_mm3.as_ref().map(|volume| volume.review),
        Some(ReviewState::Confirmed),
        "the historical version keeps its review state"
    );
    assert_eq!(part.classification.confirmed.as_ref().map(|label| label.name.as_str()), Some("clip"));
    assert_eq!(part.classification.confirmed.as_ref().map(|label| label.confirmed_by.as_str()), Some("engineer"));
    let example = &review.document.training_examples[0];
    assert_eq!(example.part_version_id, part_version_id);
    assert_eq!(example.geometry.len(), 3, "the confirmed label captures the component's bodies");
    assert_eq!(example.confirmed_label.name, "clip");
    assert_eq!(example.confirmed_label.confirmed_by, "engineer");
    assert!(example.prediction.is_none(), "the review records no model prediction in this write");
}

#[tokio::test]
async fn repeated_occurrences_share_reviewed_metadata_while_placements_stay_separate() {
    let db = TestDb::new().await;
    let store = &db.store;
    let (project, revisions, part_version_id) = setup(store).await;
    register(store, "analysis-1", revisions[0], flat_manifest(key())).await;
    register(store, "analysis-2", revisions[1], flat_manifest(key())).await;
    for (index, revision) in revisions.iter().enumerate() {
        let request_id = format!("group-{}", index + 1);
        store
            .group_solids(
                project,
                &GroupSolids {
                    request_id,
                    expected_version: u64::try_from(index).unwrap_or_default(),
                    component_id: Uuid::from_u128(0xc001 + index as u128),
                    product_revision_id: *revision,
                    analysis_key: key(),
                    geometry: vec![body(0), body(1), body(2)],
                    part_version_id: Some(part_version_id),
                },
            )
            .await
            .expect("group each revision's component");
    }
    store
        .complete_measurement(
            project,
            &CompleteMeasurement { request_id: "measure-1".into(), expected_version: 2, part_version_id, volume_mm3: manual_volume("2500") },
        )
        .await
        .expect("complete the shared measurement");
    let outcome = store
        .confirm_parts(
            project,
            &ConfirmParts {
                request_id: "confirm-1".into(),
                expected_version: 3,
                confirmations: vec![PartConfirmation {
                    part_version_id,
                    classification: Some(ClassificationConfirmation { name: "clip".into() }),
                    confirm_measurement: true,
                    catalog: None,
                }],
            },
            "engineer",
        )
        .await
        .expect("confirm the shared part");
    assert!(matches!(outcome.facts.as_slice(), [EngineeringFact::PartConfirmed { .. }, EngineeringFact::TrainingExamplesCaptured { count: 2, .. }]));
    let review = view(store).await;
    assert_eq!(review.document.parts.len(), 1, "one shared part record");
    assert_eq!(review.document.components.len(), 2);
    assert_eq!(review.document.training_examples.len(), 2, "one example per reviewed component");
    let mut revisions_in_examples: Vec<Uuid> = review.document.training_examples.iter().map(|example| example.product_revision_id).collect();
    revisions_in_examples.sort();
    let mut expected = revisions.clone();
    expected.sort();
    assert_eq!(revisions_in_examples, expected, "each placement keeps its own revision");
    let geometry: Vec<usize> = review.document.training_examples.iter().map(|example| example.geometry.len()).collect();
    assert_eq!(geometry, vec![3, 3], "the bodies are preserved per component");
}

#[tokio::test]
async fn a_corrected_label_captures_a_second_example_and_keeps_the_first() {
    let db = TestDb::new().await;
    let store = &db.store;
    let (project, revisions, part_version_id) = setup(store).await;
    let revision = revisions[0];
    register(store, "analysis-1", revision, flat_manifest(key())).await;
    store
        .group_solids(
            project,
            &GroupSolids {
                request_id: "group-1".into(),
                expected_version: 0,
                component_id: Uuid::from_u128(0xc001),
                product_revision_id: revision,
                analysis_key: key(),
                geometry: vec![body(0)],
                part_version_id: Some(part_version_id),
            },
        )
        .await
        .expect("group the component");
    store
        .complete_measurement(
            project,
            &CompleteMeasurement { request_id: "measure-1".into(), expected_version: 1, part_version_id, volume_mm3: manual_volume("2500") },
        )
        .await
        .expect("complete the measurement");
    for (request_id, expected_version, label) in [("confirm-1", 2u64, "clip"), ("confirm-2", 3, "bracket")] {
        store
            .confirm_parts(
                project,
                &ConfirmParts {
                    request_id: request_id.into(),
                    expected_version,
                    confirmations: vec![PartConfirmation {
                        part_version_id,
                        classification: Some(ClassificationConfirmation { name: label.into() }),
                        confirm_measurement: false,
                        catalog: None,
                    }],
                },
                "engineer",
            )
            .await
            .expect("correct the label");
    }
    let review = view(store).await;
    assert_eq!(review.document.parts[0].classification.confirmed.as_ref().map(|label| label.name.as_str()), Some("bracket"));
    let labels: Vec<&str> = review.document.training_examples.iter().map(|example| example.confirmed_label.name.as_str()).collect();
    assert_eq!(labels, vec!["clip", "bracket"], "the earlier example is never rewritten");
}

#[tokio::test]
async fn review_writes_are_idempotent_expected_version_checked_and_history_readable() {
    let db = TestDb::new().await;
    let store = &db.store;
    let (project, revisions, part_version_id) = setup(store).await;
    let revision = revisions[0];
    register(store, "analysis-1", revision, flat_manifest(key())).await;
    let group_request = GroupSolids {
        request_id: "group-1".into(),
        expected_version: 0,
        component_id: Uuid::from_u128(0xc001),
        product_revision_id: revision,
        analysis_key: key(),
        geometry: vec![body(0)],
        part_version_id: Some(part_version_id),
    };
    let first = store.group_solids(project, &group_request).await.expect("group");
    let retry = store.group_solids(project, &group_request).await.expect("the idempotent retry");
    assert_eq!(retry, first);
    assert_eq!(view(store).await.version, 1, "the retry appends no version");

    let stale = store
        .group_solids(project, &GroupSolids { request_id: "group-2".into(), expected_version: 0, ..group_request.clone() })
        .await
        .expect_err("a stale expected version");
    assert!(matches!(stale, StoreError::Conflict(_)), "{stale:?}");

    let reused = store
        .group_solids(project, &GroupSolids { expected_version: 1, geometry: vec![body(1)], ..group_request })
        .await
        .expect_err("a reused request id with another payload");
    assert!(matches!(reused, StoreError::Conflict(_)), "{reused:?}");

    let current = view(store).await;
    let version_one = store.engineering_record_version(project, 1).await.expect("the retained review version");
    assert_eq!(version_one.version, 1);
    assert_eq!(version_one.document, current.document);
    assert!(store.engineering_record_version(project, 9).await.is_err(), "a version that was never written");
}

#[tokio::test]
async fn the_review_refuses_unknown_identities_and_unknown_projects() {
    let db = TestDb::new().await;
    let store = &db.store;
    let (project, revisions, _) = setup(store).await;
    let revision = revisions[0];
    register(store, "analysis-1", revision, flat_manifest(key())).await;

    let unknown_revision = store
        .group_solids(
            project,
            &GroupSolids {
                request_id: "group-1".into(),
                expected_version: 0,
                component_id: Uuid::from_u128(0xc001),
                product_revision_id: Uuid::from_u128(0xdead),
                analysis_key: key(),
                geometry: vec![body(0)],
                part_version_id: None,
            },
        )
        .await
        .expect_err("an unknown product revision");
    assert!(matches!(unknown_revision, StoreError::Invalid(_)), "{unknown_revision:?}");

    let unknown_part = store
        .group_solids(
            project,
            &GroupSolids {
                request_id: "group-2".into(),
                expected_version: 0,
                component_id: Uuid::from_u128(0xc001),
                product_revision_id: revision,
                analysis_key: key(),
                geometry: vec![body(0)],
                part_version_id: Some(Uuid::from_u128(0xdead)),
            },
        )
        .await
        .expect_err("an unknown part version");
    assert!(matches!(unknown_part, StoreError::Invalid(_)), "{unknown_part:?}");

    let unknown_project = store
        .group_solids(
            Uuid::from_u128(0xdead),
            &GroupSolids {
                request_id: "group-3".into(),
                expected_version: 0,
                component_id: Uuid::from_u128(0xc001),
                product_revision_id: revision,
                analysis_key: key(),
                geometry: vec![body(0)],
                part_version_id: None,
            },
        )
        .await
        .expect_err("an unknown project");
    assert!(matches!(unknown_project, StoreError::NotFound), "{unknown_project:?}");

    // Clearing a part association makes the component unresolved again; the write is recorded.
    store
        .group_solids(
            project,
            &GroupSolids {
                request_id: "group-4".into(),
                expected_version: 0,
                component_id: Uuid::from_u128(0xc001),
                product_revision_id: revision,
                analysis_key: key(),
                geometry: vec![body(0)],
                part_version_id: None,
            },
        )
        .await
        .expect("group without a part identity");
    let cleared = store
        .associate_component_part(
            project,
            &AssociateComponentPart {
                request_id: "associate-1".into(),
                expected_version: 1,
                component_id: Uuid::from_u128(0xc001),
                part_version_id: None,
            },
        )
        .await
        .expect("clear the association");
    assert_eq!(cleared.receipt.version, 2);
    assert!(codes(&view(store).await).contains(&"unresolved_component_association"));
}

// ---------------------------------------------------------------------------
// SB-13: recognition and catalog suggestions
// ---------------------------------------------------------------------------

/// A recorded classification proposal: a label from the recognition rules with the exact analysis,
/// the prediction version and one evidence item the contract requires.
fn prediction(name: &str, confidence: Option<&str>) -> ClassificationPrediction {
    ClassificationPrediction {
        name: name.into(),
        model_version: "recognition-rules-1".into(),
        confidence: confidence.map(support::d),
        prediction_version: Some("1".into()),
        analysis_key: Some(key()),
        evidence: vec![SuggestionEvidence::new("tubular_sweep", "the exact geometry is one swept constant-diameter tube").expect("valid evidence")],
        suggested_at: None,
    }
}

fn classification(part_version_id: Uuid, name: &str, confidence: Option<&str>) -> ClassificationSuggestion {
    ClassificationSuggestion { part_version_id, prediction: prediction(name, confidence) }
}

fn catalog_match(part_version_id: Uuid, component_id: Uuid, component_version_id: Uuid) -> CatalogMatchSuggestion {
    CatalogMatchSuggestion {
        part_version_id,
        component_id,
        component_version_id,
        model_version: "catalog-text-match-1".into(),
        prediction_version: "1".into(),
        confidence: Some(support::d("1")),
        evidence: vec![
            SuggestionEvidence::new("catalog_attributes_match", "every declared catalog attribute matches the reviewed part")
                .expect("valid evidence"),
        ],
    }
}

/// Group the flat fixture's three bodies into one reviewed component of the shared part version.
async fn group_reviewed_component(store: &Store, project: Uuid, revision: Uuid, part_version_id: Uuid) {
    store
        .group_solids(
            project,
            &GroupSolids {
                request_id: "group-1".into(),
                expected_version: 0,
                component_id: Uuid::from_u128(0xc001),
                product_revision_id: revision,
                analysis_key: key(),
                geometry: vec![body(0), body(1), body(2)],
                part_version_id: Some(part_version_id),
            },
        )
        .await
        .expect("group the reviewed component");
}

/// Publish one catalog component version and answer its exact version id.
async fn publish_catalog(store: &Store, request_id: &str, component_id: Uuid, name: &str, specification: &str, price: Option<&str>) -> Uuid {
    let request = support::component_version_with_attributes(request_id, 0, component_id, name, specification, price, &[("category", "tube")]);
    let published = store.create_component_version(&request, "administrator").await.expect("publish the catalog component version");
    published.component.component_version_id
}

fn reviewed_part(review: &EngineeringRecordView, part_version_id: Uuid) -> &solidbom_contract::ReviewedPart {
    review.document.parts.iter().find(|part| part.part_version_id == part_version_id).expect("the reviewed part record")
}

#[tokio::test]
async fn a_confident_prediction_stays_pending_and_confirmation_keeps_the_reviewed_geometry() {
    let db = TestDb::new().await;
    let store = &db.store;
    let (project, revisions, part_version_id) = setup(store).await;
    register(store, "analysis-1", revisions[0], flat_manifest(key())).await;
    group_reviewed_component(store, project, revisions[0], part_version_id).await;
    let reviewed = view(store).await;
    let geometry = reviewed.document.components[0].geometry.clone();
    let analysis = reviewed.document.components[0].analysis_key.clone();

    // A recognition rule proposes a tube with confidence 1 and a client-claimed timestamp. The
    // proposal is evidence, never an approval, and the service stamps its own recording time.
    let mut confident = classification(part_version_id, "tube", Some("1"));
    confident.prediction.suggested_at = Some("2000-01-01T00:00:00Z".into());
    let outcome = store
        .record_suggestions(
            project,
            &RecordSuggestions {
                catalog_evaluated_part_versions: Vec::new(),
                request_id: "suggest-1".into(),
                expected_version: 1,
                classification: vec![confident],
                catalog: Vec::new(),
            },
        )
        .await
        .expect("record the confident prediction");
    assert_eq!(outcome.receipt.version, 2);
    assert!(matches!(outcome.facts.as_slice(), [EngineeringFact::SuggestionsRecorded { classification: 1, catalog: 0 }]));

    let review = view(store).await;
    let part = reviewed_part(&review, part_version_id);
    assert!(part.classification.confirmed.is_none(), "confidence 1 never confirms a label");
    let proposal = part.classification.prediction.as_ref().expect("the proposal is stored");
    assert_eq!(proposal.name, "tube");
    assert_eq!(proposal.confidence, Some(support::d("1")));
    assert!(proposal.suggested_at.is_some(), "the service fills the recording time");
    let recorded_at = proposal.suggested_at.as_deref().expect("the service stamps the recording time");
    assert_ne!(recorded_at, "2000-01-01T00:00:00Z", "a client-supplied timestamp is overwritten, never stored");
    assert!(recorded_at.ends_with('Z') && recorded_at.contains('T'), "the service writes its own RFC 3339 UTC timestamp: {recorded_at}");
    assert!(codes(&review).contains(&"pending_classification"), "{:?}", review.issues);
    assert!(review.document.training_examples.is_empty(), "a proposal captures no training example");
    assert_eq!(review.document.components[0].geometry, geometry, "recording a prediction never changes reviewed geometry");
    assert_eq!(review.document.components[0].analysis_key, analysis);

    // The engineer confirms the proposed label; the reviewed geometry and analysis stay exactly as
    // they were reviewed.
    let outcome = store
        .confirm_parts(
            project,
            &ConfirmParts {
                request_id: "confirm-1".into(),
                expected_version: 2,
                confirmations: vec![PartConfirmation {
                    part_version_id,
                    classification: Some(ClassificationConfirmation { name: "tube".into() }),
                    confirm_measurement: false,
                    catalog: None,
                }],
            },
            "engineer",
        )
        .await
        .expect("confirm the proposed label");
    assert_eq!(outcome.receipt.version, 3);
    let review = view(store).await;
    let part = reviewed_part(&review, part_version_id);
    assert_eq!(part.classification.confirmed.as_ref().map(|label| label.name.as_str()), Some("tube"));
    assert_eq!(part.classification.confirmed.as_ref().map(|label| label.confirmed_by.as_str()), Some("engineer"));
    assert!(!codes(&review).contains(&"pending_classification"), "{:?}", review.issues);
    let example = &review.document.training_examples[0];
    assert_eq!(example.confirmed_label.name, "tube");
    assert_eq!(example.prediction.as_ref().map(|proposal| proposal.name.as_str()), Some("tube"), "the confirmed label captures the proposal");
    assert_eq!(review.document.components[0].geometry, geometry, "confirmation changes the decision, never the reviewed geometry");
    assert_eq!(review.document.components[0].analysis_key, analysis);
    assert_eq!(
        part.classification.prediction.as_ref().map(|proposal| proposal.name.as_str()),
        Some("tube"),
        "the proposal stays readable after confirmation"
    );
}

#[tokio::test]
async fn recording_a_prediction_keeps_the_confirmed_label_and_a_correction_replaces_it() {
    let db = TestDb::new().await;
    let store = &db.store;
    let (project, revisions, part_version_id) = setup(store).await;
    register(store, "analysis-1", revisions[0], flat_manifest(key())).await;
    group_reviewed_component(store, project, revisions[0], part_version_id).await;
    let geometry = view(store).await.document.components[0].geometry.clone();
    store
        .confirm_parts(
            project,
            &ConfirmParts {
                request_id: "confirm-1".into(),
                expected_version: 1,
                confirmations: vec![PartConfirmation {
                    part_version_id,
                    classification: Some(ClassificationConfirmation { name: "clip".into() }),
                    confirm_measurement: false,
                    catalog: None,
                }],
            },
            "engineer",
        )
        .await
        .expect("confirm the first label");

    // A later recognition run proposes another label. It is stored as a proposal only: no silent
    // reclassification of an existing project.
    store
        .record_suggestions(
            project,
            &RecordSuggestions {
                catalog_evaluated_part_versions: Vec::new(),
                request_id: "suggest-1".into(),
                expected_version: 2,
                classification: vec![classification(part_version_id, "tube", Some("1"))],
                catalog: Vec::new(),
            },
        )
        .await
        .expect("record the later prediction");
    let review = view(store).await;
    let part = reviewed_part(&review, part_version_id);
    assert_eq!(
        part.classification.confirmed.as_ref().map(|label| label.name.as_str()),
        Some("clip"),
        "a prediction never silently replaces the confirmed label"
    );
    assert_eq!(part.classification.prediction.as_ref().map(|proposal| proposal.name.as_str()), Some("tube"));
    assert_eq!(review.document.training_examples.len(), 1, "recording a proposal captures no example");
    assert_eq!(review.document.components[0].geometry, geometry, "recording a proposal leaves the reviewed geometry untouched");

    // The engineer's correction is the only path that replaces the confirmed label, and the
    // earlier confirmed example stays retained.
    store
        .confirm_parts(
            project,
            &ConfirmParts {
                request_id: "confirm-2".into(),
                expected_version: 3,
                confirmations: vec![PartConfirmation {
                    part_version_id,
                    classification: Some(ClassificationConfirmation { name: "bracket".into() }),
                    confirm_measurement: false,
                    catalog: None,
                }],
            },
            "engineer",
        )
        .await
        .expect("correct the confirmed label");
    let review = view(store).await;
    let part = reviewed_part(&review, part_version_id);
    assert_eq!(part.classification.confirmed.as_ref().map(|label| label.name.as_str()), Some("bracket"));
    let labels: Vec<&str> = review.document.training_examples.iter().map(|example| example.confirmed_label.name.as_str()).collect();
    assert_eq!(labels, vec!["clip", "bracket"], "each confirmed label keeps its own example");
    assert_eq!(review.document.training_examples[1].prediction.as_ref().map(|proposal| proposal.name.as_str()), Some("tube"));
    assert_eq!(review.document.components[0].geometry, geometry);
}

#[tokio::test]
async fn catalog_suggestions_are_pending_evidence_until_the_engineer_confirms_the_reference() {
    let db = TestDb::new().await;
    let store = &db.store;
    let (project, revisions, part_version_id) = setup(store).await;
    register(store, "analysis-1", revisions[0], flat_manifest(key())).await;
    group_reviewed_component(store, project, revisions[0], part_version_id).await;
    let component_id = Uuid::from_u128(0xca70);
    let component_version_id = publish_catalog(store, "catalog-1", component_id, "Hose clip 12-20", "stainless steel, 12-20 mm", Some("0.42")).await;

    let outcome = store
        .record_suggestions(
            project,
            &RecordSuggestions {
                catalog_evaluated_part_versions: Vec::new(),
                request_id: "suggest-1".into(),
                expected_version: 1,
                classification: Vec::new(),
                catalog: vec![catalog_match(part_version_id, component_id, component_version_id)],
            },
        )
        .await
        .expect("record the catalog candidate");
    assert!(matches!(outcome.facts.as_slice(), [EngineeringFact::SuggestionsRecorded { classification: 0, catalog: 1 }]));

    // The candidate carries the catalog's resolved values, and is still only evidence.
    let review = view(store).await;
    let part = reviewed_part(&review, part_version_id);
    assert!(part.catalog.confirmed.is_none());
    assert!(part.catalog.effective_inputs().is_none(), "an unconfirmed candidate never supplies a price or specification");
    let candidate = &part.catalog.suggestions[0];
    assert_eq!(candidate.component_version_id, component_version_id);
    assert_eq!(candidate.catalog_version, 1);
    assert_eq!(candidate.catalog_name, "Hose clip 12-20");
    assert_eq!(candidate.specification, "stainless steel, 12-20 mm");
    assert_eq!(candidate.price, Some(support::d("0.42")));
    assert_eq!(candidate.price_unit, Unit::CurrencyPerPiece);
    assert_eq!(candidate.currency, "CNY");
    assert_eq!(candidate.model_version, "catalog-text-match-1");
    assert_eq!(candidate.prediction_version, "1");
    assert_eq!(candidate.confidence, Some(support::d("1")), "confidence stays evidence only");
    assert!(candidate.suggested_at.is_some());
    let suggestion_id = candidate.suggestion_id;

    // A repeated matching run replaces the candidate list with the same deterministic identity.
    store
        .record_suggestions(
            project,
            &RecordSuggestions {
                catalog_evaluated_part_versions: Vec::new(),
                request_id: "suggest-2".into(),
                expected_version: 2,
                classification: Vec::new(),
                catalog: vec![catalog_match(part_version_id, component_id, component_version_id)],
            },
        )
        .await
        .expect("record the same matching run again");
    let review = view(store).await;
    let part = reviewed_part(&review, part_version_id);
    assert_eq!(part.catalog.suggestions.len(), 1);
    assert_eq!(part.catalog.suggestions[0].suggestion_id, suggestion_id, "the suggestion identity is deterministic");

    // The engineer confirms the catalog version; only now does it become authoritative input.
    let outcome = store
        .confirm_parts(
            project,
            &ConfirmParts {
                request_id: "confirm-1".into(),
                expected_version: 3,
                confirmations: vec![PartConfirmation {
                    part_version_id,
                    classification: None,
                    confirm_measurement: false,
                    catalog: Some(CatalogConfirmation { component_version_id, project_override: None }),
                }],
            },
            "engineer",
        )
        .await
        .expect("confirm the catalog version");
    assert!(
        outcome
            .facts
            .iter()
            .any(|fact| matches!(fact, EngineeringFact::CatalogMatchConfirmed { part_version_id: confirmed } if *confirmed == part_version_id))
    );
    assert!(
        !outcome.facts.iter().any(|fact| matches!(fact, EngineeringFact::PartConfirmed { .. })),
        "a catalog-only confirmation records no no-op label or volume confirmation"
    );
    let review = view(store).await;
    let part = reviewed_part(&review, part_version_id);
    let confirmed = part.catalog.confirmed.as_ref().expect("the confirmed catalog reference");
    assert_eq!(confirmed.component_version_id, component_version_id);
    assert_eq!(confirmed.version, 1);
    assert_eq!(confirmed.name, "Hose clip 12-20");
    assert_eq!(confirmed.specification, "stainless steel, 12-20 mm");
    assert_eq!(confirmed.price, Some(support::d("0.42")));
    assert_eq!(confirmed.price_unit, Unit::CurrencyPerPiece);
    assert_eq!(confirmed.currency, "CNY");
    assert_eq!(confirmed.confirmed_by, "engineer", "the authenticated reviewer confirms the reference");
    let inputs = part.catalog.effective_inputs().expect("a confirmed reference supplies the catalog inputs");
    assert_eq!(inputs.component_version_id, component_version_id);
    assert_eq!(inputs.specification, "stainless steel, 12-20 mm");
    assert_eq!(inputs.price, Some(support::d("0.42")));
    assert!(!inputs.override_applied);
    assert_eq!(part.catalog.suggestions.len(), 1, "the confirmed reference keeps the proposals as evidence");

    // Confirming a project override records it separately: the original catalog value is retained.
    store
        .confirm_parts(
            project,
            &ConfirmParts {
                request_id: "confirm-2".into(),
                expected_version: 4,
                confirmations: vec![PartConfirmation {
                    part_version_id,
                    classification: None,
                    confirm_measurement: false,
                    catalog: Some(CatalogConfirmation {
                        component_version_id,
                        project_override: Some(ProjectCatalogOverride {
                            specification: "project stock: stainless steel, 12-20 mm".into(),
                            price: Some(support::d("0.55")),
                            price_unit: Unit::CurrencyPerPiece,
                            currency: "CNY".into(),
                            reason: "project buys from a different supplier".into(),
                            recorded_by: "client-claimed reviewer".into(),
                        }),
                    }),
                }],
            },
            "engineer",
        )
        .await
        .expect("confirm the project override");
    let review = view(store).await;
    let part = reviewed_part(&review, part_version_id);
    let confirmed = part.catalog.confirmed.as_ref().expect("the original catalog reference is retained");
    assert_eq!(confirmed.component_version_id, component_version_id);
    assert_eq!(confirmed.version, 1);
    assert_eq!(confirmed.specification, "stainless steel, 12-20 mm");
    assert_eq!(confirmed.price, Some(support::d("0.42")));
    let project_override = part.catalog.project_override.as_ref().expect("the project override is recorded separately");
    assert_eq!(project_override.reason, "project buys from a different supplier");
    assert_eq!(project_override.recorded_by, "engineer", "the service records the authenticated reviewer");
    let inputs = part.catalog.effective_inputs().expect("the override supplies the project inputs");
    assert!(inputs.override_applied);
    assert_eq!(inputs.specification, "project stock: stainless steel, 12-20 mm");
    assert_eq!(inputs.price, Some(support::d("0.55")));
    assert_eq!(inputs.component_version_id, component_version_id);
}

#[tokio::test]
async fn a_fresh_matching_run_with_no_candidates_clears_old_suggestions_and_keeps_the_confirmed_reference() {
    let db = TestDb::new().await;
    let store = &db.store;
    let (project, revisions, part_version_id) = setup(store).await;
    register(store, "analysis-1", revisions[0], flat_manifest(key())).await;
    group_reviewed_component(store, project, revisions[0], part_version_id).await;
    let component_id = Uuid::from_u128(0xca70);
    let component_version_id = publish_catalog(store, "catalog-1", component_id, "Hose clip 12-20", "stainless steel, 12-20 mm", Some("0.42")).await;

    // The first matching run proposes one candidate.
    store
        .record_suggestions(
            project,
            &RecordSuggestions {
                catalog_evaluated_part_versions: Vec::new(),
                request_id: "suggest-1".into(),
                expected_version: 1,
                classification: Vec::new(),
                catalog: vec![catalog_match(part_version_id, component_id, component_version_id)],
            },
        )
        .await
        .expect("record the catalog candidate");

    // The engineer confirms the candidate and records a project override.
    store
        .confirm_parts(
            project,
            &ConfirmParts {
                request_id: "confirm-1".into(),
                expected_version: 2,
                confirmations: vec![PartConfirmation {
                    part_version_id,
                    classification: None,
                    confirm_measurement: false,
                    catalog: Some(CatalogConfirmation {
                        component_version_id,
                        project_override: Some(ProjectCatalogOverride {
                            specification: "project stock: stainless steel, 12-20 mm".into(),
                            price: Some(support::d("0.55")),
                            price_unit: Unit::CurrencyPerPiece,
                            currency: "CNY".into(),
                            reason: "project buys from a different supplier".into(),
                            recorded_by: "engineer".into(),
                        }),
                    }),
                }],
            },
            "engineer",
        )
        .await
        .expect("confirm the catalog version with the override");

    // The next matching run finds no candidate (for example an empty or ambiguous result). It names
    // the evaluated part version explicitly, so the stale proposal is cleared while the engineer's
    // confirmed reference and project override stay exactly as recorded.
    let outcome = store
        .record_suggestions(
            project,
            &RecordSuggestions {
                catalog_evaluated_part_versions: vec![part_version_id],
                request_id: "suggest-2".into(),
                expected_version: 3,
                classification: Vec::new(),
                catalog: Vec::new(),
            },
        )
        .await
        .expect("record the empty matching run");
    assert!(matches!(outcome.facts.as_slice(), [EngineeringFact::SuggestionsRecorded { classification: 0, catalog: 0 }]));
    let review = view(store).await;
    let part = reviewed_part(&review, part_version_id);
    assert!(part.catalog.suggestions.is_empty(), "a no-match run clears the previous proposals");
    let confirmed = part.catalog.confirmed.as_ref().expect("the confirmed reference survives a matching run");
    assert_eq!(confirmed.component_version_id, component_version_id);
    assert_eq!(confirmed.price, Some(support::d("0.42")), "the confirmed catalog value is retained");
    assert!(part.catalog.project_override.is_some(), "the project override survives a matching run");
    let inputs = part.catalog.effective_inputs().expect("the confirmed override still supplies the project inputs");
    assert!(inputs.override_applied);
    assert_eq!(inputs.price, Some(support::d("0.55")));

    // An empty evaluation of a part version this project has never reviewed creates no record.
    let unrelated = Uuid::from_u128(0xffff_0001);
    let outcome = store
        .record_suggestions(
            project,
            &RecordSuggestions {
                catalog_evaluated_part_versions: vec![unrelated],
                request_id: "suggest-3".into(),
                expected_version: 4,
                classification: Vec::new(),
                catalog: Vec::new(),
            },
        )
        .await;
    assert!(matches!(outcome, Err(StoreError::Invalid(_))), "an unknown part version is refused before any write");
}

#[tokio::test]
async fn a_published_multiline_catalog_specification_can_be_suggested_and_confirmed() {
    let db = TestDb::new().await;
    let store = &db.store;
    let (project, revisions, part_version_id) = setup(store).await;
    register(store, "analysis-1", revisions[0], flat_manifest(key())).await;
    group_reviewed_component(store, project, revisions[0], part_version_id).await;
    let component_id = Uuid::from_u128(0xca70);
    // The publication contract accepts a multi-line specification; freezing it into a suggestion and
    // a confirmed reference must accept the same value instead of rejecting its newline.
    let multiline = "steel\n12-20 mm";
    let component_version_id = publish_catalog(store, "catalog-1", component_id, "Hose clip 12-20", multiline, Some("0.42")).await;

    store
        .record_suggestions(
            project,
            &RecordSuggestions {
                catalog_evaluated_part_versions: Vec::new(),
                request_id: "suggest-1".into(),
                expected_version: 1,
                classification: Vec::new(),
                catalog: vec![catalog_match(part_version_id, component_id, component_version_id)],
            },
        )
        .await
        .expect("a published multi-line specification is suggestable");
    let review = view(store).await;
    let part = reviewed_part(&review, part_version_id);
    assert_eq!(part.catalog.suggestions[0].specification, multiline);

    store
        .confirm_parts(
            project,
            &ConfirmParts {
                request_id: "confirm-1".into(),
                expected_version: 2,
                confirmations: vec![PartConfirmation {
                    part_version_id,
                    classification: None,
                    confirm_measurement: false,
                    catalog: Some(CatalogConfirmation { component_version_id, project_override: None }),
                }],
            },
            "engineer",
        )
        .await
        .expect("a published multi-line specification is confirmable");
    let review = view(store).await;
    let part = reviewed_part(&review, part_version_id);
    let confirmed = part.catalog.confirmed.as_ref().expect("the confirmed reference");
    assert_eq!(confirmed.specification, multiline);
}

#[tokio::test]
async fn a_fresh_matching_run_replaces_candidates_while_the_confirmed_reference_stays() {
    let db = TestDb::new().await;
    let store = &db.store;
    let (project, revisions, part_version_id) = setup(store).await;
    register(store, "analysis-1", revisions[0], flat_manifest(key())).await;
    group_reviewed_component(store, project, revisions[0], part_version_id).await;
    let first_component = Uuid::from_u128(0xca70);
    let first_version = publish_catalog(store, "catalog-1", first_component, "Hose clip 12-20", "stainless steel, 12-20 mm", Some("0.42")).await;
    let second_component = Uuid::from_u128(0xca71);
    let second_version = publish_catalog(store, "catalog-2", second_component, "Hose clip 14-24", "stainless steel, 14-24 mm", Some("0.51")).await;

    store
        .record_suggestions(
            project,
            &RecordSuggestions {
                catalog_evaluated_part_versions: Vec::new(),
                request_id: "suggest-1".into(),
                expected_version: 1,
                classification: Vec::new(),
                catalog: vec![
                    catalog_match(part_version_id, first_component, first_version),
                    catalog_match(part_version_id, second_component, second_version),
                ],
            },
        )
        .await
        .expect("record the first matching run");
    let review = view(store).await;
    assert_eq!(reviewed_part(&review, part_version_id).catalog.suggestions.len(), 2);

    store
        .confirm_parts(
            project,
            &ConfirmParts {
                request_id: "confirm-1".into(),
                expected_version: 2,
                confirmations: vec![PartConfirmation {
                    part_version_id,
                    classification: None,
                    confirm_measurement: false,
                    catalog: Some(CatalogConfirmation { component_version_id: first_version, project_override: None }),
                }],
            },
            "engineer",
        )
        .await
        .expect("confirm the first catalog version");

    // A fresh matching run replaces only the proposal list: the engineer's confirmed reference and
    // the authoritative inputs it supplies stay exactly as confirmed.
    store
        .record_suggestions(
            project,
            &RecordSuggestions {
                catalog_evaluated_part_versions: Vec::new(),
                request_id: "suggest-2".into(),
                expected_version: 3,
                classification: Vec::new(),
                catalog: vec![catalog_match(part_version_id, second_component, second_version)],
            },
        )
        .await
        .expect("record a fresh matching run");
    let review = view(store).await;
    let part = reviewed_part(&review, part_version_id);
    assert_eq!(part.catalog.suggestions.len(), 1, "a fresh matching run replaces the candidate list");
    assert_eq!(part.catalog.suggestions[0].component_version_id, second_version);
    let confirmed = part.catalog.confirmed.as_ref().expect("the confirmed reference survives a fresh matching run");
    assert_eq!(confirmed.component_version_id, first_version);
    assert_eq!(confirmed.version, 1);
    assert_eq!(confirmed.price, Some(support::d("0.42")));
    assert_eq!(confirmed.confirmed_by, "engineer");
    let inputs = part.catalog.effective_inputs().expect("the confirmed reference stays authoritative");
    assert_eq!(inputs.component_version_id, first_version);
    assert_eq!(inputs.price, Some(support::d("0.42")));
    assert!(!inputs.override_applied);
}

#[tokio::test]
async fn suggestion_writes_are_idempotent_version_checked_and_reject_unknown_identities() {
    let db = TestDb::new().await;
    let store = &db.store;
    let (project, revisions, part_version_id) = setup(store).await;
    register(store, "analysis-1", revisions[0], flat_manifest(key())).await;
    group_reviewed_component(store, project, revisions[0], part_version_id).await;
    let component_id = Uuid::from_u128(0xca70);
    let component_version_id = publish_catalog(store, "catalog-1", component_id, "Hose clip 12-20", "stainless steel, 12-20 mm", Some("0.42")).await;

    let request = RecordSuggestions {
        catalog_evaluated_part_versions: Vec::new(),
        request_id: "suggest-1".into(),
        expected_version: 1,
        classification: vec![classification(part_version_id, "tube", Some("1"))],
        catalog: vec![catalog_match(part_version_id, component_id, component_version_id)],
    };
    let first = store.record_suggestions(project, &request).await.expect("record the suggestions");
    assert_eq!(first.receipt.version, 2);
    let retry = store.record_suggestions(project, &request).await.expect("the idempotent retry");
    assert_eq!(retry, first);
    assert_eq!(view(store).await.version, 2, "the retry appends no version");

    let reused = store
        .record_suggestions(project, &RecordSuggestions { catalog_evaluated_part_versions: Vec::new(), catalog: Vec::new(), ..request.clone() })
        .await
        .expect_err("a reused request id with another payload");
    assert!(matches!(reused, StoreError::Conflict(_)), "{reused:?}");

    let stale = store
        .record_suggestions(
            project,
            &RecordSuggestions {
                catalog_evaluated_part_versions: Vec::new(),
                request_id: "suggest-2".into(),
                expected_version: 1,
                catalog: Vec::new(),
                ..request.clone()
            },
        )
        .await
        .expect_err("a stale expected version");
    assert!(matches!(stale, StoreError::Conflict(_)), "{stale:?}");

    let unknown_part = store
        .record_suggestions(
            project,
            &RecordSuggestions {
                catalog_evaluated_part_versions: Vec::new(),
                request_id: "suggest-3".into(),
                expected_version: 2,
                classification: vec![classification(Uuid::from_u128(0xdead), "tube", Some("1"))],
                catalog: Vec::new(),
            },
        )
        .await
        .expect_err("a part version the project does not hold");
    assert!(matches!(unknown_part, StoreError::Invalid(_)), "{unknown_part:?}");

    let mut unregistered = classification(part_version_id, "tube", Some("1"));
    unregistered.prediction.analysis_key = Some(AnalysisKey { extractor_version: "999".into(), ..key() });
    let unregistered = store
        .record_suggestions(
            project,
            &RecordSuggestions {
                catalog_evaluated_part_versions: Vec::new(),
                request_id: "suggest-4".into(),
                expected_version: 2,
                classification: vec![unregistered],
                catalog: Vec::new(),
            },
        )
        .await
        .expect_err("an analysis the project has not registered");
    assert!(matches!(unregistered, StoreError::Invalid(_)), "{unregistered:?}");

    // A registered analysis of another revision that describes no component of this part version is
    // not a valid binding either: the prediction cannot be attached to unrelated reviewed geometry.
    let later_key = AnalysisKey { extractor_version: "3".into(), ..key() };
    register(store, "analysis-2", revisions[1], flat_manifest(later_key.clone())).await;
    let mut unrelated = classification(part_version_id, "tube", Some("1"));
    unrelated.prediction.analysis_key = Some(later_key);
    let unrelated = store
        .record_suggestions(
            project,
            &RecordSuggestions {
                catalog_evaluated_part_versions: Vec::new(),
                request_id: "suggest-5".into(),
                expected_version: 2,
                classification: vec![unrelated],
                catalog: Vec::new(),
            },
        )
        .await
        .expect_err("an analysis that describes no reviewed component of the part");
    assert!(matches!(unrelated, StoreError::Invalid(_)), "{unrelated:?}");

    let unknown_catalog = store
        .record_suggestions(
            project,
            &RecordSuggestions {
                catalog_evaluated_part_versions: Vec::new(),
                request_id: "suggest-6".into(),
                expected_version: 2,
                classification: Vec::new(),
                catalog: vec![catalog_match(part_version_id, component_id, Uuid::from_u128(0xdead))],
            },
        )
        .await
        .expect_err("a catalog version the catalog does not hold");
    assert!(matches!(unknown_catalog, StoreError::Invalid(_)), "{unknown_catalog:?}");

    let mismatched_component = store
        .record_suggestions(
            project,
            &RecordSuggestions {
                catalog_evaluated_part_versions: Vec::new(),
                request_id: "suggest-7".into(),
                expected_version: 2,
                classification: Vec::new(),
                catalog: vec![catalog_match(part_version_id, Uuid::from_u128(0xdead), component_version_id)],
            },
        )
        .await
        .expect_err("a catalog version of a different logical component");
    assert!(matches!(mismatched_component, StoreError::Invalid(_)), "{mismatched_component:?}");

    // An unknown catalog version is also refused at confirmation time.
    let unknown_confirmation = store
        .confirm_parts(
            project,
            &ConfirmParts {
                request_id: "confirm-unknown".into(),
                expected_version: 2,
                confirmations: vec![PartConfirmation {
                    part_version_id,
                    classification: None,
                    confirm_measurement: false,
                    catalog: Some(CatalogConfirmation { component_version_id: Uuid::from_u128(0xdead), project_override: None }),
                }],
            },
            "engineer",
        )
        .await
        .expect_err("a confirmation of a catalog version the catalog does not hold");
    assert!(matches!(unknown_confirmation, StoreError::Invalid(_)), "{unknown_confirmation:?}");

    let unknown_project = store
        .record_suggestions(
            Uuid::from_u128(0xdead),
            &RecordSuggestions {
                catalog_evaluated_part_versions: Vec::new(),
                request_id: "suggest-unknown-project".into(),
                expected_version: 0,
                ..request
            },
        )
        .await
        .expect_err("an unknown project");
    assert!(matches!(unknown_project, StoreError::NotFound), "{unknown_project:?}");
    assert_eq!(view(store).await.version, 2, "a refused write appends no version");
}
