//! Store-level tests for sourcing, routing templates and attached costs (contract version 1 of the
//! SB-15 manufacturing contract, additive to API version 2).
//!
//! `#[tokio::test]` is not recognised as a test function by Clippy's test allowance, so this file
//! allows the panicking helpers it needs. The library and binary still deny them.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

mod support;

use solidbom_contract::{
    AdoptRoutingTemplate, AnalysisKey, AppliedRouting, AttachedCostLine, CostOwner, CreateProject, CycleBasis, CycleOrigin, DeliveredPartInput,
    DeliveredProductInput, DeliveryPackage, EditAttachedCosts, EditRouting, ManualRouting, ManufacturingFact, ManufacturingRecordView,
    MaterialConsumption, MeasurementSource, OperationCycle, ProductKind, ProjectDetails, PublishRoutingTemplate, RecordDelivery, ReviewState,
    RoutingLevel, RoutingOperation, RoutingSource, RoutingWrite, SetSourcing, SetupTime, SourcedPart, Sourcing, TemplateRef, Unit, YearRange,
};
use solidbom_server::{Store, StoreError};
use support::TestDb;
use uuid::Uuid;

const SOURCE: &[u8] =
    b"ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('sourcing manufacturing fixture'),'2;1');\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n";
const ROUTING_A: u128 = 0x8a01;
const ROUTING_B: u128 = 0x8a02;
const ROUTING_SUB: u128 = 0x8a03;
const ROUTING_ROOT: u128 = 0x8a04;
const TEMPLATE: u128 = 0x7e01;

fn project_id() -> Uuid {
    Uuid::from_u128(0xb151)
}

fn rate_id() -> Uuid {
    support::rate_id()
}

fn setup_rate_id() -> Uuid {
    Uuid::parse_str("00000000-0000-4000-8000-000000000102").expect("the seeded setup rate fixture")
}

fn details() -> ProjectDetails {
    ProjectDetails {
        name: "Manufacturing inquiry".into(),
        customer: "Customer".into(),
        internal_team: None,
        platform: None,
        application: None,
        currency: "CNY".into(),
        default_schedule: YearRange { sop_year: 2026, eop_year: 2028 },
    }
}

/// The exact analysis key an extracted material consumption names.
fn analysis_key() -> AnalysisKey {
    AnalysisKey { source_sha256: support::digest(SOURCE), engine_build: "0.40.0-rc.2".into(), extractor_version: "1".into() }
}

/// The project fixture: three shared part versions and three products.
///
/// - `part_a` is the shared `P-1` revision A; the part product and the assembly both reference it,
///   so one record serves both occurrences.
/// - `part_b` is the other customer revision `P-1` revision B of the same part number.
/// - `part_sub` is the subassembly `SUB-1` revision A.
struct Fixture {
    part_a: Uuid,
    part_b: Uuid,
    part_sub: Uuid,
    product_assembly: Uuid,
}

fn delivered_part(key: &str, number: &str, revision: &str) -> DeliveredPartInput {
    DeliveredPartInput {
        delivered_key: key.into(),
        customer_part_number: Some(number.into()),
        customer_revision: Some(revision.into()),
        volume_mm3: Some(support::d("2400")),
    }
}

fn delivered_product(name: &str, kind: ProductKind, parts: Vec<DeliveredPartInput>) -> DeliveredProductInput {
    DeliveredProductInput { product_id: None, name: name.into(), kind, customer_part_number: None, parts }
}

async fn setup(store: &Store) -> Fixture {
    store
        .create_project(&CreateProject { request_id: "create-1".into(), project_id: project_id(), details: details() })
        .await
        .expect("create the project");
    let source = store.put_source(&support::digest(SOURCE), "manufacturing.step", SOURCE).await.expect("store the source");
    store
        .record_delivery(
            project_id(),
            &RecordDelivery {
                request_id: "delivery-1".into(),
                expected_version: 1,
                package: DeliveryPackage { package_id: Uuid::from_u128(0xb152), name: "Package 1".into(), source },
                products: vec![
                    delivered_product("part-a", ProductKind::Part, vec![delivered_part("a-body", "P-1", "A")]),
                    delivered_product("part-b", ProductKind::Part, vec![delivered_part("b-body", "P-1", "B")]),
                    delivered_product(
                        "assembly",
                        ProductKind::Assembly,
                        vec![delivered_part("a-body-again", "P-1", "A"), delivered_part("sub-body", "SUB-1", "A")],
                    ),
                ],
            },
        )
        .await
        .expect("record the delivery");
    let view = store.project_record_view(project_id()).await.expect("the project record view");
    let part_version = |number: &str, revision: &str| -> Uuid {
        view.document.part_version_by_identity(number, revision).expect("the delivered part version").part_version_id
    };
    let product =
        |name: &str| -> Uuid { view.document.products.iter().find(|product| product.name == name).expect("the delivered product").product_id };
    Fixture {
        part_a: part_version("P-1", "A"),
        part_b: part_version("P-1", "B"),
        part_sub: part_version("SUB-1", "A"),
        product_assembly: product("assembly"),
    }
}

fn manufactured(part_version_id: Uuid) -> SourcedPart {
    SourcedPart {
        part_version_id,
        sourcing: Sourcing::Manufactured,
        material: Some(MaterialConsumption {
            material_version_id: Some(support::material_id()),
            quantity: Some(support::d("2400")),
            unit: Unit::MillimetreCubed,
            source: MeasurementSource::Extracted,
            review: ReviewState::Confirmed,
            confirmed_by: None,
            analysis_key: Some(analysis_key()),
            reason: None,
            allowance_rate: Some(support::d("0.02")),
            source_reference: None,
        }),
        source_reference: None,
    }
}

fn purchased(part_version_id: Uuid) -> SourcedPart {
    SourcedPart { part_version_id, sourcing: Sourcing::Purchased, material: None, source_reference: None }
}

fn cycle(seconds: Option<&str>, basis: CycleBasis, cavities: Option<u32>, production_rate_version_id: Option<Uuid>) -> OperationCycle {
    OperationCycle {
        seconds: seconds.map(support::d),
        basis,
        cavities,
        production_rate_version_id,
        origin: CycleOrigin::Manual,
        source_reference: None,
    }
}

fn operation(
    operation_id: u128,
    sequence: u32,
    name: &str,
    cost_center_id: Option<Uuid>,
    cycle: Option<OperationCycle>,
    setup: Option<SetupTime>,
) -> RoutingOperation {
    RoutingOperation {
        operation_id: Uuid::from_u128(operation_id),
        sequence,
        name: name.into(),
        description: None,
        cost_center_id,
        cycle,
        setup,
        tooling_reference: None,
    }
}

fn write_manual(
    request_id: &str,
    expected_version: u64,
    routing_id: Uuid,
    level: RoutingLevel,
    owner: CostOwner,
    operations: Vec<RoutingOperation>,
    affected_part_versions: Vec<Uuid>,
    affected_products: Vec<Uuid>,
) -> RoutingWrite {
    RoutingWrite::Manual(ManualRouting {
        request_id: request_id.into(),
        expected_version,
        routing_id,
        level,
        owner,
        operations,
        affected_part_versions,
        affected_products,
    })
}

fn write_adopt(
    request_id: &str,
    expected_version: u64,
    routing_id: Uuid,
    level: RoutingLevel,
    owner: CostOwner,
    template: TemplateRef,
    affected_part_versions: Vec<Uuid>,
    affected_products: Vec<Uuid>,
) -> RoutingWrite {
    RoutingWrite::AdoptTemplate(AdoptRoutingTemplate {
        request_id: request_id.into(),
        expected_version,
        routing_id,
        level,
        owner,
        template,
        affected_part_versions,
        affected_products,
    })
}

fn write_edit(
    request_id: &str,
    expected_version: u64,
    routing_id: Uuid,
    operations: Vec<RoutingOperation>,
    affected_part_versions: Vec<Uuid>,
) -> RoutingWrite {
    RoutingWrite::Edit(EditRouting {
        request_id: request_id.into(),
        expected_version,
        routing_id,
        operations,
        affected_part_versions,
        affected_products: Vec::new(),
    })
}

fn publish_template(
    request_id: &str,
    expected_version: u64,
    applies_to: Vec<RoutingLevel>,
    operations: Vec<RoutingOperation>,
) -> PublishRoutingTemplate {
    PublishRoutingTemplate {
        request_id: request_id.into(),
        expected_version,
        template_id: Uuid::from_u128(TEMPLATE),
        name: "tube routing".into(),
        description: Some("the shared tube cutting and bending standard".into()),
        applies_to,
        operations,
        source_reference: Some("routing-standard/1".into()),
    }
}

async fn view(store: &Store) -> ManufacturingRecordView {
    store.manufacturing_record_view(project_id()).await.expect("the manufacturing record view")
}

fn codes(view: &ManufacturingRecordView) -> Vec<&str> {
    view.issues.iter().map(|issue| issue.code.as_str()).collect()
}

fn routing(view: &ManufacturingRecordView, routing_id: Uuid) -> &AppliedRouting {
    view.document.routing(routing_id).expect("the applied routing")
}

#[tokio::test]
async fn explicit_sourcing_is_shared_by_one_part_version_and_another_revision_stays_separate() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = setup(store).await;

    // Version 0: no sourcing is assumed, so every shared part version is a computed issue.
    let empty = view(store).await;
    assert_eq!(empty.version, 0);
    assert!(empty.document.parts.is_empty());
    assert!(empty.document.routings.is_empty());
    assert!(empty.document.attached_costs.is_empty());
    let unsourced: Vec<Uuid> =
        empty.issues.iter().filter(|issue| issue.code == "missing_sourcing").filter_map(|issue| issue.part_version_id).collect();
    assert_eq!(unsourced.len(), 3, "three shared part versions are unsourced: {:?}", empty.issues);
    for part_version_id in [fixture.part_a, fixture.part_b, fixture.part_sub] {
        assert!(unsourced.contains(&part_version_id));
    }

    let mut shared = manufactured(fixture.part_a);
    if let Some(material) = shared.material.as_mut() {
        material.confirmed_by = Some("forged client reviewer".into());
    }
    let outcome = store
        .set_sourcing(
            project_id(),
            &SetSourcing { request_id: "sourcing-1".into(), expected_version: 0, parts: vec![shared], affected_part_versions: vec![fixture.part_a] },
            "engineer",
        )
        .await
        .expect("set the explicit sourcing");
    assert_eq!(outcome.receipt.version, 1);
    assert_eq!(outcome.affected_part_versions, vec![fixture.part_a]);
    assert!(outcome.affected_products.is_empty());
    assert!(matches!(outcome.facts.as_slice(), [ManufacturingFact::SourcingRecorded { sourcing: Sourcing::Manufactured, .. }]));

    let current = view(store).await;
    // One record is keyed by the shared part version; both products that reference it read it.
    assert_eq!(current.document.parts.len(), 1);
    assert_eq!(current.document.part(fixture.part_a).map(|part| part.sourcing), Some(Sourcing::Manufactured));
    let stored_material = current.document.part(fixture.part_a).and_then(|part| part.material.as_ref()).expect("the stored material record");
    assert_eq!(stored_material.confirmed_by.as_deref(), Some("engineer"), "the service records the authenticated reviewer, never the payload's");
    assert!(stored_material.analysis_key.is_some(), "an extracted consumption names its analysis");
    // The other customer revision is a different part version with no record of its own.
    assert_eq!(current.document.part(fixture.part_b), None);
    assert!(!current.issues.iter().any(|issue| issue.code == "missing_sourcing" && issue.part_version_id == Some(fixture.part_a)));
    assert!(current.issues.iter().any(|issue| issue.code == "missing_sourcing" && issue.part_version_id == Some(fixture.part_b)));

    // A later write appends a version and leaves the accepted one exactly as it was.
    store
        .set_sourcing(
            project_id(),
            &SetSourcing {
                request_id: "sourcing-2".into(),
                expected_version: 1,
                parts: vec![purchased(fixture.part_b)],
                affected_part_versions: vec![fixture.part_b],
            },
            "engineer",
        )
        .await
        .expect("set the other revision purchased");
    let accepted = store.manufacturing_record_version(project_id(), 1).await.expect("the retained version");
    assert_eq!(accepted.version, 1);
    assert_eq!(accepted.document.parts.len(), 1);
    assert_eq!(accepted.document.part(fixture.part_b), None);
    let current = view(store).await;
    assert_eq!(current.version, 2);
    assert_eq!(current.document.parts.len(), 2);
    assert_eq!(current.document.part(fixture.part_b).map(|part| part.sourcing), Some(Sourcing::Purchased));
    assert_eq!(
        current.document.part(fixture.part_a).and_then(|part| part.material.as_ref()).and_then(|material| material.quantity),
        Some(support::d("2400"))
    );

    // An unknown project or record version is not found.
    assert!(matches!(store.manufacturing_record_view(Uuid::from_u128(0xdead)).await, Err(StoreError::NotFound)));
    assert!(matches!(store.manufacturing_record_version(project_id(), 9).await, Err(StoreError::NotFound)));
    db.cleanup().await;
}

#[tokio::test]
async fn template_adoption_copies_operations_and_one_edit_leaves_the_template_and_other_instance_unchanged() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = setup(store).await;
    let template = TemplateRef { template_id: Uuid::from_u128(TEMPLATE), version: 1 };
    let operations_v1 = vec![
        operation(0x11, 1, "cut tube", Some(rate_id()), Some(cycle(Some("30"), CycleBasis::PerPiece, None, Some(rate_id()))), None),
        operation(0x12, 2, "bend", Some(rate_id()), Some(cycle(Some("45"), CycleBasis::PerPiece, None, Some(rate_id()))), None),
    ];
    let receipt = store
        .publish_routing_template(&publish_template("template-1", 0, vec![RoutingLevel::Part, RoutingLevel::Subassembly], operations_v1.clone()))
        .await
        .expect("publish template version 1");
    assert_eq!(receipt.version, 1);
    assert!(!receipt.template_version_id.is_nil());

    // Listing reports the current version; the exact version stays readable.
    let list = store.routing_templates().await.expect("list the templates");
    assert_eq!(list.templates.len(), 1);
    assert_eq!(list.templates[0].template_version_id, receipt.template_version_id);
    assert_eq!(list.templates[0].operations, operations_v1);
    let read = store.routing_template_version(template.template_id, 1).await.expect("read template version 1");
    assert_eq!(read.template_version_id, receipt.template_version_id);
    assert_eq!(read.operations, operations_v1);
    assert!(matches!(store.routing_template_version(template.template_id, 2).await, Err(StoreError::NotFound)));

    // Adopt the same exact version for two owners: each copy gets its own operations and keeps
    // the retained source link.
    let adopted_a = store
        .write_routing(
            project_id(),
            &write_adopt(
                "adopt-1",
                0,
                Uuid::from_u128(ROUTING_A),
                RoutingLevel::Part,
                CostOwner::PartVersion { part_version_id: fixture.part_a },
                template,
                vec![fixture.part_a],
                Vec::new(),
            ),
        )
        .await
        .expect("adopt for part a");
    assert_eq!(adopted_a.receipt.version, 1);
    assert!(matches!(
        adopted_a.facts.as_slice(),
        [ManufacturingFact::RoutingWritten { part_version_id: Some(part_version_id), template: Some(found), .. }]
            if *part_version_id == fixture.part_a && *found == template
    ));
    assert_eq!(adopted_a.affected_part_versions, vec![fixture.part_a]);
    assert!(adopted_a.affected_products.is_empty());
    store
        .write_routing(
            project_id(),
            &write_adopt(
                "adopt-2",
                1,
                Uuid::from_u128(ROUTING_B),
                RoutingLevel::Part,
                CostOwner::PartVersion { part_version_id: fixture.part_b },
                template,
                vec![fixture.part_b],
                Vec::new(),
            ),
        )
        .await
        .expect("adopt for part b");
    let current = view(store).await;
    assert_eq!(current.version, 2);
    assert_eq!(current.document.routings.len(), 2);
    for routing in &current.document.routings {
        assert_eq!(routing.source, RoutingSource::Template(template));
        assert_eq!(routing.operations, operations_v1);
    }

    // Editing one applied copy changes only that copy: the template and the other instance stay.
    let edited_operations = vec![
        operation(0x11, 1, "cut tube, edited", Some(rate_id()), Some(cycle(Some("35"), CycleBasis::PerPiece, None, Some(rate_id()))), None),
        operation(0x12, 2, "bend, edited", Some(rate_id()), Some(cycle(Some("45"), CycleBasis::PerPiece, None, Some(rate_id()))), None),
    ];
    let edited = store
        .write_routing(project_id(), &write_edit("edit-1", 2, Uuid::from_u128(ROUTING_A), edited_operations.clone(), vec![fixture.part_a]))
        .await
        .expect("edit one applied routing");
    assert_eq!(edited.receipt.version, 3);
    assert!(matches!(
        edited.facts.as_slice(),
        [ManufacturingFact::RoutingWritten { template: Some(found), .. }] if *found == template
    ));
    let current = view(store).await;
    let edited_a = routing(&current, Uuid::from_u128(ROUTING_A));
    assert_eq!(edited_a.operations, edited_operations);
    assert_eq!(edited_a.source, RoutingSource::Template(template), "an edit keeps the source link");
    let untouched_b = routing(&current, Uuid::from_u128(ROUTING_B));
    assert_eq!(untouched_b.operations, operations_v1);
    assert_eq!(store.routing_template_version(template.template_id, 1).await.expect("the template is unchanged").operations, operations_v1);
    // An earlier accepted version keeps the operations the routing had before the edit.
    let accepted = store.manufacturing_record_version(project_id(), 2).await.expect("the retained version");
    assert_eq!(routing(&accepted, Uuid::from_u128(ROUTING_A)).operations, operations_v1);

    // Publishing a newer template version and adopting it changes only the named instance's link.
    let operations_v2 =
        vec![operation(0x21, 1, "cut tube, version 2", Some(rate_id()), Some(cycle(Some("28"), CycleBasis::PerPiece, None, Some(rate_id()))), None)];
    let receipt_v2 = store
        .publish_routing_template(&publish_template("template-2", 1, vec![RoutingLevel::Part, RoutingLevel::Subassembly], operations_v2.clone()))
        .await
        .expect("publish template version 2");
    assert_eq!(receipt_v2.version, 2);
    let version_two = store.routing_template_version(template.template_id, 2).await.expect("read template version 2");
    assert_eq!(version_two.supersedes, Some(receipt.template_version_id));
    let newer = TemplateRef { template_id: template.template_id, version: 2 };
    store
        .write_routing(
            project_id(),
            &write_adopt(
                "adopt-3",
                3,
                Uuid::from_u128(ROUTING_A),
                RoutingLevel::Part,
                CostOwner::PartVersion { part_version_id: fixture.part_a },
                newer,
                vec![fixture.part_a],
                Vec::new(),
            ),
        )
        .await
        .expect("adopt the newer template version");
    let current = view(store).await;
    assert_eq!(routing(&current, Uuid::from_u128(ROUTING_A)).source, RoutingSource::Template(newer));
    assert_eq!(routing(&current, Uuid::from_u128(ROUTING_A)).operations, operations_v2);
    assert_eq!(routing(&current, Uuid::from_u128(ROUTING_B)).source, RoutingSource::Template(template));
    // Re-adopting the same version is not a source change and is refused.
    let redundant = store
        .write_routing(
            project_id(),
            &write_adopt(
                "adopt-4",
                4,
                Uuid::from_u128(ROUTING_A),
                RoutingLevel::Part,
                CostOwner::PartVersion { part_version_id: fixture.part_a },
                newer,
                vec![fixture.part_a],
                Vec::new(),
            ),
        )
        .await
        .expect_err("a same-version re-adoption is not a source change");
    assert!(matches!(redundant, StoreError::Invalid(_)), "{redundant:?}");
    assert_eq!(view(store).await.version, 4, "the refused adoption appended nothing");
    // Publishing a version never edits an earlier one.
    assert_eq!(store.routing_template_version(template.template_id, 1).await.expect("version 1").operations, operations_v1);
    db.cleanup().await;
}

#[tokio::test]
async fn missing_cycle_rate_and_cavity_data_stay_issues_while_an_explicit_zero_cycle_is_a_value() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = setup(store).await;
    let routing_id = Uuid::from_u128(ROUTING_A);

    // A manufactured part whose consumption figures are not entered yet: missing is an issue.
    let mut incomplete = manufactured(fixture.part_a);
    if let Some(material) = incomplete.material.as_mut() {
        material.quantity = None;
        material.allowance_rate = None;
        material.review = ReviewState::Pending;
        material.confirmed_by = Some("forged client reviewer".into());
        material.analysis_key = Some(analysis_key());
    }
    store
        .set_sourcing(
            project_id(),
            &SetSourcing {
                request_id: "sourcing-1".into(),
                expected_version: 0,
                parts: vec![incomplete],
                affected_part_versions: vec![fixture.part_a],
            },
            "engineer",
        )
        .await
        .expect("set the incomplete manufactured sourcing");

    let operations = vec![
        operation(0x31, 1, "cycle time not entered", Some(rate_id()), Some(cycle(None, CycleBasis::PerPiece, None, None)), None),
        operation(0x32, 2, "explicit zero cycle", Some(rate_id()), Some(cycle(Some("0"), CycleBasis::PerPiece, None, Some(rate_id()))), None),
        operation(
            0x33,
            3,
            "per-cycle waiting for cavities",
            Some(rate_id()),
            Some(cycle(Some("12"), CycleBasis::PerCycle, None, Some(rate_id()))),
            None,
        ),
        operation(0x34, 4, "no cycle basis", Some(rate_id()), None, None),
        operation(
            0x35,
            5,
            "setup without figures",
            Some(rate_id()),
            Some(cycle(Some("30"), CycleBasis::PerPiece, None, Some(rate_id()))),
            Some(SetupTime { hours: None, setup_rate_version_id: None, source_reference: None }),
        ),
    ];
    store
        .write_routing(
            project_id(),
            &write_manual(
                "manual-1",
                1,
                routing_id,
                RoutingLevel::Part,
                CostOwner::PartVersion { part_version_id: fixture.part_a },
                operations,
                vec![fixture.part_a],
                Vec::new(),
            ),
        )
        .await
        .expect("write the incomplete manual routing");
    let current = view(store).await;
    let codes = codes(&current);
    for code in [
        "missing_consumption",
        "pending_consumption_review",
        "missing_allowance",
        "missing_cycle_time",
        "missing_production_rate",
        "missing_cycle_cavities",
        "missing_cycle",
        "missing_setup_hours",
        "missing_setup_rate",
    ] {
        assert!(codes.contains(&code), "expected `{code}` in {codes:?}");
    }
    // Missing figures stay missing; no zero is stored for them, and a pending consumption keeps no
    // reviewer even when the payload names one.
    assert_eq!(current.document.parts[0].material.as_ref().and_then(|material| material.confirmed_by.as_deref()), None);
    assert_eq!(current.document.parts[0].material.as_ref().and_then(|material| material.quantity), None);
    assert_eq!(current.document.parts[0].material.as_ref().and_then(|material| material.allowance_rate), None);
    // The explicit zero is a value: exactly the other operation reports a missing cycle time.
    let zero = current
        .document
        .routing(routing_id)
        .and_then(|routing| routing.operations.iter().find(|operation| operation.operation_id == Uuid::from_u128(0x32)))
        .expect("the explicit zero operation");
    let zero_cycle = zero.cycle.as_ref().expect("the explicit zero cycle");
    assert_eq!(zero_cycle.seconds, Some(support::d("0")));
    assert_eq!(zero_cycle.per_piece_seconds(), Some(support::d("0")));
    let missing_cycle_time: Vec<_> = current.issues.iter().filter(|issue| issue.code == "missing_cycle_time").collect();
    assert_eq!(missing_cycle_time.len(), 1, "an explicit zero cycle is not missing: {:?}", current.issues);
    assert_eq!(missing_cycle_time[0].operation_id, Some(Uuid::from_u128(0x31)));
    db.cleanup().await;
}

#[tokio::test]
async fn cavity_conversion_follows_the_declared_basis_and_a_per_piece_cycle_with_cavities_is_refused() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = setup(store).await;
    let routing_id = Uuid::from_u128(ROUTING_A);
    let operations = vec![
        operation(
            0x41,
            1,
            "per cycle shared by two cavities",
            Some(rate_id()),
            Some(cycle(Some("12"), CycleBasis::PerCycle, Some(2), Some(rate_id()))),
            None,
        ),
        operation(0x42, 2, "per piece, no cavity split", Some(rate_id()), Some(cycle(Some("12"), CycleBasis::PerPiece, None, Some(rate_id()))), None),
    ];
    store
        .write_routing(
            project_id(),
            &write_manual(
                "manual-1",
                0,
                routing_id,
                RoutingLevel::Part,
                CostOwner::PartVersion { part_version_id: fixture.part_a },
                operations,
                vec![fixture.part_a],
                Vec::new(),
            ),
        )
        .await
        .expect("write the per-cycle routing");
    let current = view(store).await;
    let operations = &routing(&current, routing_id).operations;
    assert_eq!(operations[0].cycle.as_ref().and_then(OperationCycle::per_piece_seconds), Some(support::d("6")));
    assert_eq!(operations[1].cycle.as_ref().and_then(OperationCycle::per_piece_seconds), Some(support::d("12")));

    // The contradictory per-piece cycle with two cavities is refused before anything is written.
    let contradictory = vec![operation(
        0x43,
        1,
        "per piece with cavities",
        Some(rate_id()),
        Some(cycle(Some("12"), CycleBasis::PerPiece, Some(2), Some(rate_id()))),
        None,
    )];
    let refused = store
        .write_routing(project_id(), &write_edit("edit-1", 1, routing_id, contradictory, vec![fixture.part_a]))
        .await
        .expect_err("a per-piece cycle is not divided by cavities");
    assert!(matches!(refused, StoreError::Invalid(_)), "{refused:?}");
    // A zero cavity count is refused too.
    let zero_cavities = vec![operation(
        0x44,
        1,
        "per cycle without cavities",
        Some(rate_id()),
        Some(cycle(Some("12"), CycleBasis::PerCycle, Some(0), Some(rate_id()))),
        None,
    )];
    assert!(matches!(
        store.write_routing(project_id(), &write_edit("edit-2", 1, routing_id, zero_cavities, vec![fixture.part_a])).await,
        Err(StoreError::Invalid(_))
    ));
    assert_eq!(view(store).await.version, 1, "a refused write appends no version");
    db.cleanup().await;
}

#[tokio::test]
async fn assembly_installation_and_testing_stay_on_the_parent_routing() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = setup(store).await;
    // The subassembly's own routing carries the installation/testing operations that happen in
    // the assembly context.
    let assembly_operations = vec![operations_install_and_test()];
    let outcome = store
        .write_routing(
            project_id(),
            &write_manual(
                "manual-sub",
                0,
                Uuid::from_u128(ROUTING_SUB),
                RoutingLevel::Subassembly,
                CostOwner::PartVersion { part_version_id: fixture.part_sub },
                assembly_operations.clone(),
                vec![fixture.part_sub],
                Vec::new(),
            ),
        )
        .await
        .expect("write the subassembly routing");
    assert!(matches!(
        outcome.facts.as_slice(),
        [ManufacturingFact::RoutingWritten { part_version_id: Some(part_version_id), product_id: None, .. }] if *part_version_id == fixture.part_sub
    ));
    // A child part keeps only its own manufacturing operations.
    let child_operations =
        vec![operation(0x52, 1, "cut tube", Some(rate_id()), Some(cycle(Some("30"), CycleBasis::PerPiece, None, Some(rate_id()))), None)];
    store
        .write_routing(
            project_id(),
            &write_manual(
                "manual-child",
                1,
                Uuid::from_u128(ROUTING_A),
                RoutingLevel::Part,
                CostOwner::PartVersion { part_version_id: fixture.part_a },
                child_operations.clone(),
                vec![fixture.part_a],
                Vec::new(),
            ),
        )
        .await
        .expect("write the child part routing");

    let current = view(store).await;
    let parent = current.document.routing_of(CostOwner::PartVersion { part_version_id: fixture.part_sub }).expect("the subassembly routing");
    assert_eq!(parent.level, RoutingLevel::Subassembly);
    assert_eq!(parent.operations, assembly_operations);
    let child = current.document.routing_of(CostOwner::PartVersion { part_version_id: fixture.part_a }).expect("the child routing");
    assert_eq!(child.operations, child_operations);
    assert!(!child.operations.iter().any(|operation| operation.name.contains("install")));
    // The product root may carry its own final assembly operation without touching the children.
    let root_operations =
        vec![operation(0x53, 1, "final assembly test", Some(rate_id()), Some(cycle(Some("600"), CycleBasis::PerPiece, None, Some(rate_id()))), None)];
    let outcome = store
        .write_routing(
            project_id(),
            &write_manual(
                "manual-root",
                2,
                Uuid::from_u128(ROUTING_ROOT),
                RoutingLevel::ProductRoot,
                CostOwner::ProductRoot { product_id: fixture.product_assembly },
                root_operations.clone(),
                Vec::new(),
                vec![fixture.product_assembly],
            ),
        )
        .await
        .expect("write the product root routing");
    assert!(matches!(
        outcome.facts.as_slice(),
        [ManufacturingFact::RoutingWritten { part_version_id: None, product_id: Some(product_id), .. }] if *product_id == fixture.product_assembly
    ));
    assert!(outcome.affected_part_versions.is_empty());
    assert_eq!(outcome.affected_products, vec![fixture.product_assembly]);
    db.cleanup().await;
}

fn operations_install_and_test() -> RoutingOperation {
    operation(0x51, 1, "install and test", Some(rate_id()), Some(cycle(Some("300"), CycleBasis::PerPiece, None, Some(rate_id()))), None)
}

#[tokio::test]
async fn attached_cost_lines_add_no_cad_node_and_are_removable() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = setup(store).await;
    let packaging = AttachedCostLine {
        line_id: Uuid::from_u128(0xc501),
        owner: CostOwner::PartVersion { part_version_id: fixture.part_a },
        category: "packaging".into(),
        description: Some("returnable tray".into()),
        quantity: Some(support::d("1")),
        unit: "pcs".into(),
        unit_cost: Some(support::d("0.35")),
        source_reference: None,
    };
    let adhesive = AttachedCostLine {
        line_id: Uuid::from_u128(0xc502),
        owner: CostOwner::ProductRoot { product_id: fixture.product_assembly },
        category: "adhesive".into(),
        description: None,
        quantity: Some(support::d("0.02")),
        unit: "kg".into(),
        unit_cost: Some(support::d("12")),
        source_reference: Some("supplier sheet".into()),
    };
    let outcome = store
        .edit_attached_costs(
            project_id(),
            &EditAttachedCosts {
                request_id: "attached-1".into(),
                expected_version: 0,
                upsert: vec![packaging.clone(), adhesive.clone()],
                remove: Vec::new(),
                affected_part_versions: vec![fixture.part_a],
                affected_products: vec![fixture.product_assembly],
            },
        )
        .await
        .expect("attach the non-CAD cost lines");
    assert_eq!(outcome.receipt.version, 1);
    assert!(matches!(
        outcome.facts.as_slice(),
        [ManufacturingFact::AttachedCostsWritten { written_line_ids, removed_line_ids }]
            if *written_line_ids == vec![packaging.line_id, adhesive.line_id] && removed_line_ids.is_empty()
    ));
    let current = view(store).await;
    assert_eq!(current.document.attached_costs.len(), 2);
    let lines: Vec<&AttachedCostLine> = current.document.attached_costs_of(CostOwner::PartVersion { part_version_id: fixture.part_a }).collect();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].unit, "pcs");
    assert_eq!(lines[0].quantity, Some(support::d("1")));
    // A line carries no CAD identity: no definition, occurrence or component field reaches a client.
    let json = serde_json::to_value(&current.document.attached_costs[0]).expect("the line JSON");
    for field in ["definition_id", "occurrence_id", "component_id", "product_revision_id"] {
        assert!(json.get(field).is_none(), "an attached cost line has no `{field}`");
    }

    // A line whose quantity and unit cost are not entered is an issue, not a zero.
    let incomplete = AttachedCostLine {
        line_id: Uuid::from_u128(0xc503),
        owner: CostOwner::PartVersion { part_version_id: fixture.part_b },
        category: "consumables".into(),
        description: None,
        quantity: None,
        unit: "pcs".into(),
        unit_cost: None,
        source_reference: None,
    };
    store
        .edit_attached_costs(
            project_id(),
            &EditAttachedCosts {
                request_id: "attached-2".into(),
                expected_version: 1,
                upsert: vec![incomplete.clone()],
                remove: Vec::new(),
                affected_part_versions: vec![fixture.part_b],
                affected_products: Vec::new(),
            },
        )
        .await
        .expect("attach the incomplete line");
    let current = view(store).await;
    assert!(current.issues.iter().any(|issue| issue.code == "missing_attached_quantity" && issue.line_id == Some(incomplete.line_id)));
    assert!(current.issues.iter().any(|issue| issue.code == "missing_attached_unit_cost" && issue.line_id == Some(incomplete.line_id)));
    assert_eq!(current.document.attached_costs.iter().find(|line| line.line_id == incomplete.line_id).and_then(|line| line.quantity), None);

    // Removing a line appends a version; the accepted history keeps it.
    let removed = store
        .edit_attached_costs(
            project_id(),
            &EditAttachedCosts {
                request_id: "attached-3".into(),
                expected_version: 2,
                upsert: Vec::new(),
                remove: vec![packaging.line_id],
                affected_part_versions: vec![fixture.part_a],
                affected_products: Vec::new(),
            },
        )
        .await
        .expect("remove the packaging line");
    assert!(matches!(
        removed.facts.as_slice(),
        [ManufacturingFact::AttachedCostsWritten { written_line_ids, removed_line_ids }]
            if written_line_ids.is_empty() && *removed_line_ids == vec![packaging.line_id]
    ));
    let current = view(store).await;
    assert_eq!(current.version, 3);
    assert!(current.document.attached_costs.iter().all(|line| line.line_id != packaging.line_id));
    assert_eq!(current.document.attached_costs.len(), 2);
    let accepted = store.manufacturing_record_version(project_id(), 2).await.expect("the retained version");
    assert!(accepted.document.attached_costs.iter().any(|line| line.line_id == packaging.line_id), "the accepted version keeps the removed line");

    // Removing a line that does not exist is refused.
    assert!(matches!(
        store
            .edit_attached_costs(
                project_id(),
                &EditAttachedCosts {
                    request_id: "attached-4".into(),
                    expected_version: 3,
                    upsert: Vec::new(),
                    remove: vec![Uuid::from_u128(0xdead)],
                    affected_part_versions: vec![fixture.part_a],
                    affected_products: Vec::new(),
                },
            )
            .await,
        Err(StoreError::Invalid(_))
    ));
    // A removed line's owner must be named as affected.
    assert!(matches!(
        store
            .edit_attached_costs(
                project_id(),
                &EditAttachedCosts {
                    request_id: "attached-5".into(),
                    expected_version: 3,
                    upsert: Vec::new(),
                    remove: vec![adhesive.line_id],
                    affected_part_versions: vec![fixture.part_a],
                    affected_products: Vec::new(),
                },
            )
            .await,
        Err(StoreError::Invalid(_))
    ));
    db.cleanup().await;
}

#[tokio::test]
async fn manufacturing_writes_are_idempotent_version_checked_and_history_readable() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = setup(store).await;
    let request = SetSourcing {
        request_id: "sourcing-1".into(),
        expected_version: 0,
        parts: vec![purchased(fixture.part_a)],
        affected_part_versions: vec![fixture.part_a],
    };
    let first = store.set_sourcing(project_id(), &request, "engineer").await.expect("the first write");
    let retry = store.set_sourcing(project_id(), &request, "engineer").await.expect("the idempotent retry");
    assert_eq!(retry, first);
    assert_eq!(view(store).await.version, 1, "the retry appends no version");

    let stale = store
        .set_sourcing(project_id(), &SetSourcing { request_id: "sourcing-2".into(), expected_version: 0, ..request.clone() }, "engineer")
        .await
        .expect_err("a stale expected version");
    assert!(matches!(stale, StoreError::Conflict(_)), "{stale:?}");

    let changed = store
        .set_sourcing(
            project_id(),
            &SetSourcing { parts: vec![manufactured(fixture.part_a)], affected_part_versions: vec![fixture.part_a], ..request.clone() },
            "engineer",
        )
        .await
        .expect_err("a reused request id with another payload");
    assert!(matches!(changed, StoreError::Conflict(_)), "{changed:?}");
    assert_eq!(view(store).await.version, 1);

    let retained = store.manufacturing_record_version(project_id(), 1).await.expect("the retained version");
    assert_eq!(retained.version, 1);
    assert_eq!(retained.document, view(store).await.document);
    assert!(matches!(store.manufacturing_record_version(project_id(), 2).await, Err(StoreError::NotFound)));
    db.cleanup().await;
}

#[tokio::test]
async fn an_applied_routing_with_no_operations_is_an_explicit_draft_with_a_computed_issue() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = setup(store).await;
    let routing_id = Uuid::from_u128(ROUTING_A);
    // A routing may be created before its operations are entered; the empty list is an explicit
    // draft, not an invalid payload.
    let outcome = store
        .write_routing(
            project_id(),
            &write_manual(
                "manual-empty",
                0,
                routing_id,
                RoutingLevel::Part,
                CostOwner::PartVersion { part_version_id: fixture.part_a },
                Vec::new(),
                vec![fixture.part_a],
                Vec::new(),
            ),
        )
        .await
        .expect("create an empty draft routing");
    assert_eq!(outcome.receipt.version, 1);
    let current = view(store).await;
    let missing = current.issues.iter().find(|issue| issue.code == "missing_routing_operations").expect("the computed draft issue");
    assert_eq!(missing.routing_id, Some(routing_id));
    assert_eq!(missing.operation_id, None, "the draft issue names the routing, not an operation");

    // Replacing the draft with another empty operation list is accepted and still a draft.
    store
        .write_routing(project_id(), &write_edit("edit-empty", 1, routing_id, Vec::new(), vec![fixture.part_a]))
        .await
        .expect("replace the empty draft");
    let current = view(store).await;
    assert_eq!(current.version, 2);
    assert!(current.document.routing(routing_id).is_some_and(|routing| routing.operations.is_empty()));
    assert!(codes(&current).contains(&"missing_routing_operations"));
    db.cleanup().await;
}

#[tokio::test]
async fn the_manufacturing_definition_refuses_unknown_identities_and_invalid_references() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = setup(store).await;
    let unknown = Uuid::from_u128(0xdead);
    let unknown_project = Uuid::from_u128(0xbeef);

    // An unknown project is not found; an unknown part version in the project is invalid.
    assert!(matches!(
        store
            .set_sourcing(
                unknown_project,
                &SetSourcing {
                    request_id: "sourcing-unknown-project".into(),
                    expected_version: 0,
                    parts: vec![purchased(fixture.part_a)],
                    affected_part_versions: vec![fixture.part_a],
                },
                "engineer",
            )
            .await,
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        store
            .set_sourcing(
                project_id(),
                &SetSourcing {
                    request_id: "sourcing-unknown-part".into(),
                    expected_version: 0,
                    parts: vec![purchased(unknown)],
                    affected_part_versions: vec![unknown]
                },
                "engineer",
            )
            .await,
        Err(StoreError::Invalid(_))
    ));

    // A confirmed consumption needs its value, and an extracted consumption names its analysis.
    let mut confirmed_without_value = manufactured(fixture.part_a);
    if let Some(material) = confirmed_without_value.material.as_mut() {
        material.quantity = None;
    }
    assert!(matches!(
        store
            .set_sourcing(
                project_id(),
                &SetSourcing {
                    request_id: "sourcing-no-value".into(),
                    expected_version: 0,
                    parts: vec![confirmed_without_value],
                    affected_part_versions: vec![fixture.part_a]
                },
                "engineer",
            )
            .await,
        Err(StoreError::Invalid(_))
    ));
    let mut extracted_without_analysis = manufactured(fixture.part_a);
    if let Some(material) = extracted_without_analysis.material.as_mut() {
        material.analysis_key = None;
    }
    assert!(matches!(
        store
            .set_sourcing(
                project_id(),
                &SetSourcing {
                    request_id: "sourcing-no-analysis".into(),
                    expected_version: 0,
                    parts: vec![extracted_without_analysis],
                    affected_part_versions: vec![fixture.part_a]
                },
                "engineer",
            )
            .await,
        Err(StoreError::Invalid(_))
    ));

    // Adoption of a template version that was never published is refused.
    assert!(matches!(
        store
            .write_routing(
                project_id(),
                &write_adopt(
                    "adopt-unknown",
                    0,
                    Uuid::from_u128(ROUTING_A),
                    RoutingLevel::Part,
                    CostOwner::PartVersion { part_version_id: fixture.part_a },
                    TemplateRef { template_id: Uuid::from_u128(TEMPLATE), version: 1 },
                    vec![fixture.part_a],
                    Vec::new(),
                ),
            )
            .await,
        Err(StoreError::Invalid(_))
    ));

    // A template that does not apply at the requested level is refused.
    let operations =
        vec![operation(0x61, 1, "cut tube", Some(rate_id()), Some(cycle(Some("30"), CycleBasis::PerPiece, None, Some(rate_id()))), None)];
    store.publish_routing_template(&publish_template("template-1", 0, vec![RoutingLevel::Part], operations)).await.expect("publish a part template");
    let template = TemplateRef { template_id: Uuid::from_u128(TEMPLATE), version: 1 };
    assert!(matches!(
        store
            .write_routing(
                project_id(),
                &write_adopt(
                    "adopt-wrong-level",
                    0,
                    Uuid::from_u128(ROUTING_SUB),
                    RoutingLevel::Subassembly,
                    CostOwner::PartVersion { part_version_id: fixture.part_sub },
                    template,
                    vec![fixture.part_sub],
                    Vec::new(),
                ),
            )
            .await,
        Err(StoreError::Invalid(_))
    ));

    // The owner must be named as affected, and every named reference must exist.
    let own_operations =
        vec![operation(0x62, 1, "cut tube", Some(rate_id()), Some(cycle(Some("30"), CycleBasis::PerPiece, None, Some(rate_id()))), None)];
    assert!(matches!(
        store
            .write_routing(
                project_id(),
                &write_manual(
                    "manual-owner",
                    0,
                    Uuid::from_u128(ROUTING_A),
                    RoutingLevel::Part,
                    CostOwner::PartVersion { part_version_id: fixture.part_a },
                    own_operations.clone(),
                    vec![fixture.part_b],
                    Vec::new(),
                ),
            )
            .await,
        Err(StoreError::Invalid(_))
    ));
    assert!(matches!(
        store
            .write_routing(
                project_id(),
                &write_manual(
                    "manual-ref",
                    0,
                    Uuid::from_u128(ROUTING_A),
                    RoutingLevel::Part,
                    CostOwner::PartVersion { part_version_id: fixture.part_a },
                    own_operations.clone(),
                    vec![fixture.part_a, unknown],
                    Vec::new(),
                ),
            )
            .await,
        Err(StoreError::Invalid(_))
    ));

    // The first operation sequence is exactly 1, then strictly increasing.
    let wrong_first_sequence = vec![operation(
        0x66,
        2,
        "not the first sequence",
        Some(rate_id()),
        Some(cycle(Some("30"), CycleBasis::PerPiece, None, Some(rate_id()))),
        None,
    )];
    assert!(matches!(
        store
            .write_routing(
                project_id(),
                &write_manual(
                    "manual-sequence",
                    0,
                    Uuid::from_u128(ROUTING_A),
                    RoutingLevel::Part,
                    CostOwner::PartVersion { part_version_id: fixture.part_a },
                    wrong_first_sequence,
                    vec![fixture.part_a],
                    Vec::new(),
                ),
            )
            .await,
        Err(StoreError::Invalid(_))
    ));

    // A pinned rate version must exist and keep the purpose its field assumes.
    let unknown_rate =
        vec![operation(0x63, 1, "unknown rate", Some(rate_id()), Some(cycle(Some("30"), CycleBasis::PerPiece, None, Some(unknown))), None)];
    assert!(matches!(
        store
            .write_routing(
                project_id(),
                &write_manual(
                    "manual-rate",
                    0,
                    Uuid::from_u128(ROUTING_A),
                    RoutingLevel::Part,
                    CostOwner::PartVersion { part_version_id: fixture.part_a },
                    unknown_rate,
                    vec![fixture.part_a],
                    Vec::new(),
                ),
            )
            .await,
        Err(StoreError::Invalid(_))
    ));
    let wrong_purpose = vec![operation(
        0x64,
        1,
        "setup rate as production",
        Some(rate_id()),
        Some(cycle(Some("30"), CycleBasis::PerPiece, None, Some(setup_rate_id()))),
        None,
    )];
    assert!(matches!(
        store
            .write_routing(
                project_id(),
                &write_manual(
                    "manual-purpose",
                    0,
                    Uuid::from_u128(ROUTING_A),
                    RoutingLevel::Part,
                    CostOwner::PartVersion { part_version_id: fixture.part_a },
                    wrong_purpose,
                    vec![fixture.part_a],
                    Vec::new(),
                ),
            )
            .await,
        Err(StoreError::Invalid(_))
    ));
    let mismatched_center = vec![operation(
        0x65,
        1,
        "setup in another center",
        Some(rate_id()),
        Some(cycle(Some("30"), CycleBasis::PerPiece, None, Some(rate_id()))),
        Some(SetupTime { hours: Some(support::d("0.5")), setup_rate_version_id: Some(setup_rate_id()), source_reference: None }),
    )];
    assert!(matches!(
        store
            .write_routing(
                project_id(),
                &write_manual(
                    "manual-center",
                    0,
                    Uuid::from_u128(ROUTING_A),
                    RoutingLevel::Part,
                    CostOwner::PartVersion { part_version_id: fixture.part_a },
                    mismatched_center,
                    vec![fixture.part_a],
                    Vec::new(),
                ),
            )
            .await,
        Err(StoreError::Invalid(_))
    ));

    // An edit of a routing that does not exist is refused, and a manual re-creation of an
    // existing routing id is refused: an existing routing is edited or adopts a newer template.
    store
        .write_routing(
            project_id(),
            &write_manual(
                "manual-1",
                0,
                Uuid::from_u128(ROUTING_A),
                RoutingLevel::Part,
                CostOwner::PartVersion { part_version_id: fixture.part_a },
                own_operations.clone(),
                vec![fixture.part_a],
                Vec::new(),
            ),
        )
        .await
        .expect("write the manual routing");
    assert!(matches!(
        store
            .write_routing(project_id(), &write_edit("edit-missing", 1, Uuid::from_u128(ROUTING_B), own_operations.clone(), vec![fixture.part_a]))
            .await,
        Err(StoreError::Invalid(_))
    ));
    assert!(matches!(
        store
            .write_routing(
                project_id(),
                &write_manual(
                    "manual-existing",
                    1,
                    Uuid::from_u128(ROUTING_A),
                    RoutingLevel::Part,
                    CostOwner::PartVersion { part_version_id: fixture.part_a },
                    own_operations.clone(),
                    vec![fixture.part_a],
                    Vec::new(),
                ),
            )
            .await,
        Err(StoreError::Invalid(_))
    ));
    // An existing manual routing cannot silently become template-sourced: it refuses the adoption.
    assert!(matches!(
        store
            .write_routing(
                project_id(),
                &write_adopt(
                    "adopt-manual",
                    1,
                    Uuid::from_u128(ROUTING_A),
                    RoutingLevel::Part,
                    CostOwner::PartVersion { part_version_id: fixture.part_a },
                    template,
                    vec![fixture.part_a],
                    Vec::new(),
                ),
            )
            .await,
        Err(StoreError::Invalid(_))
    ));

    // An attached cost line may not own a part version or product the project does not hold.
    assert!(matches!(
        store
            .edit_attached_costs(
                project_id(),
                &EditAttachedCosts {
                    request_id: "attached-unknown".into(),
                    expected_version: 1,
                    upsert: vec![AttachedCostLine {
                        line_id: Uuid::from_u128(0xc601),
                        owner: CostOwner::PartVersion { part_version_id: unknown },
                        category: "packaging".into(),
                        description: None,
                        quantity: Some(support::d("1")),
                        unit: "pcs".into(),
                        unit_cost: Some(support::d("1")),
                        source_reference: None,
                    }],
                    remove: Vec::new(),
                    affected_part_versions: vec![unknown],
                    affected_products: Vec::new(),
                },
            )
            .await,
        Err(StoreError::Invalid(_))
    ));
    assert_eq!(view(store).await.version, 1, "every refused write appended nothing");
    db.cleanup().await;
}

#[tokio::test]
async fn published_templates_are_a_current_list_and_an_append_only_chain() {
    let db = TestDb::new().await;
    let store = &db.store;
    let operations_v1 =
        vec![operation(0x71, 1, "cut tube", Some(rate_id()), Some(cycle(Some("30"), CycleBasis::PerPiece, None, Some(rate_id()))), None)];
    let first =
        store.publish_routing_template(&publish_template("template-1", 0, vec![RoutingLevel::Part], operations_v1)).await.expect("publish version 1");
    assert_eq!(first.version, 1);
    // A publish with a stale expected version is a conflict and appends nothing.
    let operations_v2 = vec![operation(
        0x72,
        1,
        "cut tube, second standard",
        Some(rate_id()),
        Some(cycle(Some("28"), CycleBasis::PerPiece, None, Some(rate_id()))),
        None,
    )];
    assert!(matches!(
        store.publish_routing_template(&publish_template("template-stale", 0, vec![RoutingLevel::Part], operations_v2.clone())).await,
        Err(StoreError::Conflict(_))
    ));
    let second = store
        .publish_routing_template(&publish_template("template-2", 1, vec![RoutingLevel::Part], operations_v2.clone()))
        .await
        .expect("publish version 2");
    assert_eq!(second.version, 2);
    let listed = store.routing_templates().await.expect("list the current templates");
    assert_eq!(listed.templates.len(), 1);
    assert_eq!(listed.templates[0].version, 2);
    assert_eq!(listed.templates[0].supersedes, Some(first.template_version_id));
    assert_eq!(listed.templates[0].operations, operations_v2);
    // The published version carries its service timestamp and the retained source reference.
    assert!(listed.templates[0].created_at.as_deref().is_some_and(|value| value.ends_with('Z')));
    assert_eq!(listed.templates[0].source_reference.as_deref(), Some("routing-standard/1"));
    db.cleanup().await;
}
