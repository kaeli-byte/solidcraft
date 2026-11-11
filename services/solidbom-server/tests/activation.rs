//! Store-level tests for activation readiness, coordinated activation and the accepted-baseline
//! history (package SB-19, activation contract version 1 additive to API version 2).
//!
//! `#[tokio::test]` is not recognised as a test function by Clippy's test allowance, so this file
//! allows the panicking helpers it needs. The library and binary still deny them.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

mod support;

use std::collections::BTreeSet;

use solidbom_contract::{
    AcceptFeasibilityException, ActivateRevisions, ActivationAction, ActivationFact, ActivationReadinessReport, ActivationReadinessRequest,
    ActivationReceipt, AnalysisKey, AnalysisManifest, AnnualForecast, ClassificationConfirmation, CompleteMeasurement, ConfirmParts, CostOwner,
    CostingVolumePolicy, CreateFeasibilityRuleTableVersion, CreateProject, CycleBasis, CycleOrigin, DefinitionChange, DefinitionRef,
    DeliveredPartInput, DeliveredProductInput, DeliveryPackage, EditRouting, FeasibilityComponentEvidence, FeasibilityConnector, FeasibilityRule,
    FeasibilityRuleKind, FeasibilityTubeAttributes, FeasibilityTubeEnd, FeasibilityTubeMeasurements, GeometryBinding, GroupSolids, ManualRouting,
    MaterialConsumption, Measurement, MeasurementSource, NumericRangeMm, OccurrenceRef, OperationCycle, PartConfirmation, ProductKind,
    ProductRevisionSelection, ProductRevisionState, ProjectDetails, RecordDelivery, RecordFeasibilityAssessment, RegisterAnalysis, ReviewState,
    ReviewedGeometry, RoutingLevel, RoutingOperation, RoutingWrite, RuleApplicability, RuleLimit, SCHEMA_VERSION, SetSourcing, SetupTime,
    SourcedPart, Sourcing, TubeEnd, TubeKind, Unit, UpdateProduct, UpdateProjectCosting, YearRange,
};
use solidbom_server::{Store, StoreError};
use support::TestDb;
use uuid::Uuid;

const SOURCE: &[u8] = b"ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('activation fixture'),'2;1');\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n";
const IDENTITY: [[f64; 4]; 4] = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];

/// The fixture connector the feasibility rules and evidence name.
const CONNECTOR_ID: u128 = 0xc0;

fn analysis_key() -> AnalysisKey {
    AnalysisKey { source_sha256: support::digest(SOURCE), engine_build: "0.40.0-rc.2".into(), extractor_version: "1".into() }
}

fn production_rate_id() -> Uuid {
    support::seeded_rate_version_id()
}

fn setup_rate_id() -> Uuid {
    Uuid::parse_str("00000000-0000-4000-8000-000000000102").expect("the seeded setup rate fixture")
}

#[derive(Clone, Copy)]
enum Feasibility {
    Passing,
    Failing,
}

/// One delivered product of the fixture.
struct FixtureProduct {
    product_id: Uuid,
    revision_id: Uuid,
    part_version_id: Uuid,
    component_id: Uuid,
    routing_id: Uuid,
    revision_label: String,
}

impl FixtureProduct {
    fn selection(&self) -> ProductRevisionSelection {
        ProductRevisionSelection { product_id: self.product_id, product_revision_id: self.revision_id }
    }
}

/// A fully priced, feasible project: every product is quoted with a positive forecast, the shared
/// part versions are reviewed, sourced and routed, the costing settings and app configuration are
/// published, and one feasibility assessment is recorded.
struct Fixture {
    project_id: Uuid,
    products: Vec<FixtureProduct>,
}

impl Fixture {
    fn part_version(&self, revision_label: &str) -> Uuid {
        self.products.iter().find(|product| product.revision_label == revision_label).expect("the fixture part version").part_version_id
    }

    fn selections(&self) -> Vec<ProductRevisionSelection> {
        self.products.iter().map(FixtureProduct::selection).collect()
    }
}

async fn current_project_version(store: &Store, project_id: Uuid) -> u64 {
    store.project_record_view(project_id).await.expect("the project record view").version
}

async fn engineering_version(store: &Store, project_id: Uuid) -> u64 {
    store.engineering_record_view(project_id).await.expect("the engineering record view").version
}

async fn manufacturing_version(store: &Store, project_id: Uuid) -> u64 {
    store.manufacturing_record_view(project_id).await.expect("the manufacturing record view").version
}

async fn feasibility_version(store: &Store, project_id: Uuid) -> u64 {
    store.feasibility_record_view(project_id).await.expect("the feasibility record view").version
}

async fn accepted_head(store: &Store, project_id: Uuid) -> Option<Uuid> {
    store.accepted_baselines(project_id).await.expect("the accepted history").baselines.first().map(|baseline| baseline.accepted_baseline_id)
}

async fn count(store: &Store, table: &str) -> i64 {
    support::row_count(store, table).await
}

fn refusal_message(error: StoreError) -> String {
    match error {
        StoreError::Conflict(message) | StoreError::Invalid(message) => message,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn details(name: &str) -> ProjectDetails {
    ProjectDetails {
        name: name.into(),
        customer: "Customer".into(),
        internal_team: None,
        platform: None,
        application: None,
        currency: "CNY".into(),
        default_schedule: YearRange { sop_year: 2027, eop_year: 2027 },
    }
}

fn delivered_part(key: &str, number: &str, revision: &str) -> DeliveredPartInput {
    DeliveredPartInput {
        delivered_key: key.into(),
        customer_part_number: Some(number.into()),
        customer_revision: Some(revision.into()),
        volume_mm3: Some(support::d("2400")),
    }
}

fn manifest(number: &str, revision: &str, seed: u128) -> AnalysisManifest {
    let definition_id = Uuid::from_u128(0xde00_0000 + seed * 10);
    let occurrence_id = Uuid::from_u128(0x0cc0_0000 + seed * 10);
    AnalysisManifest {
        schema_version: SCHEMA_VERSION,
        key: analysis_key(),
        definitions: vec![DefinitionRef {
            definition_id,
            source_key: analysis_key(),
            customer_part_number: Some(number.into()),
            customer_revision: Some(revision.into()),
            volume_mm3: Measurement {
                value: Some(support::d("2400")),
                unit: Unit::MillimetreCubed,
                source: MeasurementSource::Extracted,
                review: ReviewState::Confirmed,
                reason: None,
                analysis_key: analysis_key(),
            },
        }],
        occurrences: vec![OccurrenceRef { occurrence_id, definition_id, parent_occurrence_id: None, source_path: Vec::new(), transform: IDENTITY }],
        geometry_bindings: vec![GeometryBinding { source_path: Vec::new(), source_body: 0, definition_id, occurrence_id }],
        issues: Vec::new(),
    }
}

fn manufactured(part_version_id: Uuid, quantity: &str, allowance: Option<&str>) -> SourcedPart {
    SourcedPart {
        part_version_id,
        sourcing: Sourcing::Manufactured,
        material: Some(MaterialConsumption {
            material_version_id: Some(support::material_id()),
            quantity: Some(support::d(quantity)),
            unit: Unit::CentimetreCubed,
            source: MeasurementSource::Extracted,
            review: ReviewState::Confirmed,
            confirmed_by: None,
            analysis_key: Some(analysis_key()),
            reason: None,
            allowance_rate: allowance.map(support::d),
            source_reference: None,
        }),
        source_reference: None,
    }
}

fn operation(operation_id: u128, sequence: u32, seconds: &str) -> RoutingOperation {
    RoutingOperation {
        operation_id: Uuid::from_u128(operation_id),
        sequence,
        name: "cut tube".into(),
        description: None,
        cost_center_id: None,
        cycle: Some(OperationCycle {
            seconds: Some(support::d(seconds)),
            basis: CycleBasis::PerPiece,
            cavities: None,
            production_rate_version_id: Some(production_rate_id()),
            origin: CycleOrigin::Manual,
            source_reference: None,
        }),
        setup: Some(SetupTime { hours: Some(support::d("0.5")), setup_rate_version_id: Some(setup_rate_id()), source_reference: None }),
        tooling_reference: None,
    }
}

fn rules(seed: u128) -> Vec<FeasibilityRule> {
    let source_reference = "design rules/".to_string();
    vec![
        FeasibilityRule {
            rule_id: Uuid::from_u128(seed * 100 + 1),
            kind: FeasibilityRuleKind::MinimumBendRadius,
            applicability: RuleApplicability {
                tube_kind: Some(TubeKind::Smooth),
                material_name: Some("PA12".into()),
                supporting_spring_required: true,
                ..RuleApplicability::default()
            },
            limit: RuleLimit::MinimumRadiusMm { radius_mm: support::d("30") },
            source_reference: source_reference.clone(),
            note: None,
        },
        FeasibilityRule {
            rule_id: Uuid::from_u128(seed * 100 + 2),
            kind: FeasibilityRuleKind::BendSpacing,
            applicability: RuleApplicability::default(),
            limit: RuleLimit::MinimumLengthMm { length_mm: support::d("20") },
            source_reference: source_reference.clone(),
            note: None,
        },
        FeasibilityRule {
            rule_id: Uuid::from_u128(seed * 100 + 3),
            kind: FeasibilityRuleKind::TubeConnectorCompatibility,
            applicability: RuleApplicability { connector_component_id: Some(Uuid::from_u128(CONNECTOR_ID)), ..RuleApplicability::default() },
            limit: RuleLimit::CompatibleTubeRange {
                outer_diameter_mm: NumericRangeMm { min_mm: support::d("8"), max_mm: support::d("10") },
                inner_diameter_mm: NumericRangeMm { min_mm: support::d("5"), max_mm: support::d("8") },
            },
            source_reference: source_reference.clone(),
            note: None,
        },
        FeasibilityRule {
            rule_id: Uuid::from_u128(seed * 100 + 4),
            kind: FeasibilityRuleKind::EndAssemblyLength,
            applicability: RuleApplicability { connector_component_id: Some(Uuid::from_u128(CONNECTOR_ID)), ..RuleApplicability::default() },
            limit: RuleLimit::MinimumLengthMm { length_mm: support::d("25") },
            source_reference,
            note: None,
        },
    ]
}

fn evidence(revision: Uuid, component_id: Uuid, feasibility: Feasibility) -> FeasibilityComponentEvidence {
    let radii = match feasibility {
        Feasibility::Passing => vec![support::d("35"), support::d("40")],
        Feasibility::Failing => vec![support::d("35"), support::d("25")],
    };
    let connector =
        FeasibilityConnector { catalog_component_id: Uuid::from_u128(CONNECTOR_ID), catalog_component_version_id: Uuid::from_u128(CONNECTOR_ID + 1) };
    FeasibilityComponentEvidence {
        component_id,
        product_revision_id: revision,
        analysis_key: analysis_key(),
        extraction_version: "1".into(),
        attributes: FeasibilityTubeAttributes {
            tube_kind: Some(TubeKind::Smooth),
            material_name: Some("PA12".into()),
            supporting_spring: Some(true),
            nominal_outer_diameter_mm: Some(support::d("10")),
            nominal_wall_thickness_mm: Some(support::d("1")),
            nominal_connection_size_mm: None,
        },
        measurements: Some(FeasibilityTubeMeasurements {
            outer_diameter_mm: support::d("10"),
            inner_diameter_mm: support::d("8"),
            bend_radii_mm: radii,
            tangent_straight_lengths_mm: vec![support::d("30")],
            straight_end_lengths_mm: vec![support::d("40"), support::d("40")],
        }),
        ends: vec![
            FeasibilityTubeEnd { end: TubeEnd::Start, connector: Some(connector) },
            FeasibilityTubeEnd { end: TubeEnd::End, connector: Some(connector) },
        ],
    }
}

/// Build the fixture. `specs` names each product with the customer part number and revision of its
/// delivered part; products naming one identity share one project part version.
async fn fixture(store: &Store, seed: u128, specs: &[(&str, &str, &str)], feasibility: Feasibility, confirm: bool) -> Fixture {
    let project_id = Uuid::from_u128(0xa190_0000 + seed);
    let source = store.put_source(&support::digest(SOURCE), "activation.step", SOURCE).await.expect("store the source");
    store
        .create_project(&CreateProject { request_id: format!("{seed}-create"), project_id, details: details("Activation inquiry") })
        .await
        .expect("create the project");
    let mut project_version = 1;
    let delivered: Vec<DeliveredProductInput> = specs
        .iter()
        .map(|(name, number, revision)| DeliveredProductInput {
            product_id: None,
            name: (*name).into(),
            kind: ProductKind::Part,
            customer_part_number: None,
            parts: vec![delivered_part(&format!("{name}-body"), number, revision)],
        })
        .collect();
    store
        .record_delivery(
            project_id,
            &RecordDelivery {
                request_id: format!("{seed}-delivery-1"),
                expected_version: project_version,
                package: DeliveryPackage { package_id: Uuid::from_u128(0xaa00_0000 + seed), name: "Package 1".into(), source },
                products: delivered,
            },
        )
        .await
        .expect("record the delivery");
    project_version += 1;

    let view = store.project_record_view(project_id).await.expect("the project record");
    let mut products = Vec::with_capacity(specs.len());
    for (index, (name, number, revision)) in specs.iter().enumerate() {
        let product = view.document.products.iter().find(|product| product.name == *name).expect("the delivered product");
        let product_id = product.product_id;
        let revision_id = product.revisions.first().expect("the delivered revision").product_revision_id;
        let part_version_id = view.document.part_version_by_identity(number, revision).expect("the delivered part version").part_version_id;
        store
            .update_product(
                project_id,
                &UpdateProduct {
                    request_id: format!("{seed}-product-{index}"),
                    expected_version: project_version,
                    product_id,
                    name: (*name).into(),
                    kind: ProductKind::Part,
                    customer_part_number: None,
                    schedule_override: Some(YearRange { sop_year: 2027, eop_year: 2027 }),
                    forecasts: vec![AnnualForecast { year: 2027, quantity: 100 }],
                },
            )
            .await
            .expect("set the forecast");
        project_version += 1;
        store
            .register_analysis(
                project_id,
                &RegisterAnalysis {
                    request_id: format!("{seed}-analysis-{index}"),
                    product_revision_id: revision_id,
                    manifest: manifest(number, revision, seed * 10 + index as u128),
                },
            )
            .await
            .expect("register the analysis");
        let component_id = Uuid::from_u128(0xcc00_0000 + seed * 100 + index as u128);
        store
            .group_solids(
                project_id,
                &GroupSolids {
                    request_id: format!("{seed}-group-{index}"),
                    expected_version: engineering_version(store, project_id).await,
                    component_id,
                    product_revision_id: revision_id,
                    analysis_key: analysis_key(),
                    geometry: vec![ReviewedGeometry { source_path: Vec::new(), source_body: 0 }],
                    part_version_id: Some(part_version_id),
                },
            )
            .await
            .expect("group the reviewed component");
        products.push(FixtureProduct {
            product_id,
            revision_id,
            part_version_id,
            component_id,
            routing_id: Uuid::from_u128(0x7700_0000 + seed * 100 + index as u128),
            revision_label: (*revision).to_string(),
        });
    }

    // One reviewed measurement and confirmation per distinct shared part version.
    let distinct_parts: Vec<Uuid> = products.iter().map(|product| product.part_version_id).collect::<BTreeSet<_>>().into_iter().collect();
    for (index, part_version_id) in distinct_parts.iter().enumerate() {
        store
            .complete_measurement(
                project_id,
                &CompleteMeasurement {
                    request_id: format!("{seed}-measurement-{index}"),
                    expected_version: engineering_version(store, project_id).await,
                    part_version_id: *part_version_id,
                    volume_mm3: Measurement {
                        value: Some(support::d("2400")),
                        unit: Unit::MillimetreCubed,
                        source: MeasurementSource::Manual,
                        review: ReviewState::Pending,
                        reason: Some("open surface; measured on the bench".into()),
                        analysis_key: analysis_key(),
                    },
                },
            )
            .await
            .expect("record the reviewed measurement");
    }
    if confirm {
        let confirmations = distinct_parts
            .iter()
            .map(|part_version_id| PartConfirmation {
                part_version_id: *part_version_id,
                classification: Some(ClassificationConfirmation { name: "bracket".into() }),
                confirm_measurement: true,
                catalog: None,
            })
            .collect();
        store
            .confirm_parts(
                project_id,
                &ConfirmParts {
                    request_id: format!("{seed}-confirm"),
                    expected_version: engineering_version(store, project_id).await,
                    confirmations,
                },
                "engineer",
            )
            .await
            .expect("confirm the reviewed parts");
    }

    // Sourcing for every distinct shared part version and one part routing per distinct part version.
    store
        .set_sourcing(
            project_id,
            &SetSourcing {
                request_id: format!("{seed}-sourcing"),
                expected_version: manufacturing_version(store, project_id).await,
                parts: distinct_parts.iter().map(|part_version_id| manufactured(*part_version_id, "100", Some("0.02"))).collect(),
                affected_part_versions: distinct_parts.clone(),
            },
            "engineer",
        )
        .await
        .expect("set the sourcing");
    for (index, part_version_id) in distinct_parts.iter().enumerate() {
        let routing = products.iter().find(|product| product.part_version_id == *part_version_id).expect("a product for the part version");
        store
            .write_routing(
                project_id,
                &RoutingWrite::Manual(ManualRouting {
                    request_id: format!("{seed}-routing-{index}"),
                    expected_version: manufacturing_version(store, project_id).await,
                    routing_id: routing.routing_id,
                    level: RoutingLevel::Part,
                    owner: CostOwner::PartVersion { part_version_id: *part_version_id },
                    operations: vec![operation(0x7100_0000 + seed * 100 + index as u128, 1, "30")],
                    affected_part_versions: vec![*part_version_id],
                    affected_products: Vec::new(),
                }),
            )
            .await
            .expect("write the routing");
    }
    store
        .create_app_config_version(
            &support::app_config_version(&format!("{seed}-config"), 0, Uuid::from_u128(0xcf00_0000 + seed), Vec::new()),
            "administrator",
        )
        .await
        .expect("publish the app configuration");
    store
        .update_project_costing(
            project_id,
            &UpdateProjectCosting {
                request_id: format!("{seed}-costing"),
                expected_version: project_version,
                costing: solidbom_contract::ProjectCosting {
                    volume_policy: Some(CostingVolumePolicy::Average),
                    exchange_rates: Vec::new(),
                    purchase_prices: Vec::new(),
                },
            },
        )
        .await
        .expect("write the costing settings");
    project_version += 1;
    assert_eq!(current_project_version(store, project_id).await, project_version);

    // One feasibility rule table and one assessment. The evidence names the first product's reviewed
    // component; assessment coverage is the client's obligation, as the SB-14 contract says.
    let rule_table_id = Uuid::from_u128(0xfa00_0000 + seed);
    let published = store
        .publish_feasibility_rules(
            rule_table_id,
            &CreateFeasibilityRuleTableVersion {
                request_id: format!("{seed}-rules"),
                expected_version: 0,
                rule_table_id,
                name: "Reviewed design rules".into(),
                rules: rules(seed),
                source_reference: Some("design rules/".into()),
            },
            "administrator",
        )
        .await
        .expect("publish the feasibility rules");
    let first = &products[0];
    store
        .record_feasibility_assessment(
            project_id,
            &RecordFeasibilityAssessment {
                request_id: format!("{seed}-assessment"),
                expected_version: 0,
                rule_table_version_id: published.rule_table.rule_table_version_id,
                components: vec![evidence(first.revision_id, first.component_id, feasibility)],
            },
            "engineer",
        )
        .await
        .expect("record the feasibility assessment");
    Fixture { project_id, products }
}

async fn readiness(store: &Store, fixture: &Fixture, selections: &[ProductRevisionSelection]) -> ActivationReadinessReport {
    store
        .activation_readiness(
            fixture.project_id,
            &ActivationReadinessRequest {
                project_version: current_project_version(store, fixture.project_id).await,
                selections: selections.to_vec(),
            },
        )
        .await
        .expect("the readiness report")
}

async fn activate(store: &Store, fixture: &Fixture, request_id: &str, selections: &[ProductRevisionSelection]) -> ActivationReceipt {
    store
        .activate_revisions(
            fixture.project_id,
            &ActivateRevisions {
                request_id: request_id.into(),
                expected_project_version: current_project_version(store, fixture.project_id).await,
                expected_accepted_baseline_id: accepted_head(store, fixture.project_id).await,
                product_revisions: selections.to_vec(),
            },
        )
        .await
        .expect("the activation")
}

#[tokio::test]
async fn readiness_is_read_only_and_reports_the_selected_revisions() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = fixture(store, 1, &[("A", "PL-100", "B"), ("B", "PL-100", "B")], Feasibility::Passing, true).await;
    let before = (
        count(store, "project_record_versions").await,
        count(store, "baseline_calculations").await,
        count(store, "accepted_baselines").await,
        count(store, "requests").await,
    );
    let report = readiness(store, &fixture, &fixture.selections()).await;
    assert_eq!(report.project_id, fixture.project_id);
    assert_eq!(report.project_version, current_project_version(store, fixture.project_id).await);
    assert!(report.accepted_baseline_id.is_none(), "the first activation has no accepted baseline");
    assert_eq!(report.members.len(), 2);
    assert!(report.members.iter().all(|member| member.state == solidbom_contract::ReadinessState::Ready), "{:?}", report.members);
    assert!(report.members.iter().all(|member| member.blockers.is_empty()));
    assert!(report.coordination.changed_definitions.is_empty());
    assert!(report.coordination.required_product_ids.is_empty());
    let after = (
        count(store, "project_record_versions").await,
        count(store, "baseline_calculations").await,
        count(store, "accepted_baselines").await,
        count(store, "requests").await,
    );
    assert_eq!(before, after, "readiness appends nothing");
    db.cleanup().await;
}

#[tokio::test]
async fn a_shared_material_change_refuses_one_product_and_the_coordinated_activation_changes_both() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = fixture(store, 2, &[("A", "PL-100", "B"), ("B", "PL-100", "B")], Feasibility::Passing, true).await;
    let first = activate(store, &fixture, "2-activation-1", &fixture.selections()).await;
    assert_eq!(first.members.len(), 2);
    assert!(first.members.iter().all(|member| member.action == ActivationAction::Activated));
    assert_eq!(first.costing_state, solidbom_contract::ResultState::Complete);
    assert_eq!(accepted_head(store, fixture.project_id).await, Some(first.accepted_baseline_id), "the first accepted baseline heads the project");
    assert!(matches!(
        first.facts.first(),
        Some(ActivationFact::BaselineAccepted { accepted_baseline_id, previous_accepted_baseline_id: None, .. })
            if *accepted_baseline_id == first.accepted_baseline_id
    ));
    let accepted_count = count(store, "accepted_baselines").await;
    let snapshot_count = count(store, "baseline_calculations").await;
    let project_version = current_project_version(store, fixture.project_id).await;

    // A shared sourcing/material change on the part both products cost.
    store
        .set_sourcing(
            fixture.project_id,
            &SetSourcing {
                request_id: "2-sourcing-change".into(),
                expected_version: manufacturing_version(store, fixture.project_id).await,
                parts: vec![manufactured(fixture.products[0].part_version_id, "110", Some("0.02"))],
                affected_part_versions: vec![fixture.products[0].part_version_id],
            },
            "engineer",
        )
        .await
        .expect("change the shared sourcing");
    // A product-only delivery: product A gets a new draft revision.
    let source = store.put_source(&support::digest(SOURCE), "activation.step", SOURCE).await.expect("the source");
    store
        .record_delivery(
            fixture.project_id,
            &RecordDelivery {
                request_id: "2-delivery-2".into(),
                expected_version: current_project_version(store, fixture.project_id).await,
                package: DeliveryPackage { package_id: Uuid::from_u128(0xbb00_0002), name: "Package 2".into(), source },
                products: vec![DeliveredProductInput {
                    product_id: Some(fixture.products[0].product_id),
                    name: "A".into(),
                    kind: ProductKind::Part,
                    customer_part_number: None,
                    parts: vec![delivered_part("A-body-2", "PL-100", "B")],
                }],
            },
        )
        .await
        .expect("deliver the new draft");
    let view = store.project_record_view(fixture.project_id).await.expect("the project record");
    let product = view.document.product(fixture.products[0].product_id).expect("the product");
    let draft = product.revisions.last().expect("the new draft revision").product_revision_id;
    // The new draft needs the same exact review the accepted revision had.
    store
        .register_analysis(
            fixture.project_id,
            &RegisterAnalysis { request_id: "2-analysis-draft".into(), product_revision_id: draft, manifest: manifest("PL-100", "B", 299) },
        )
        .await
        .expect("register the analysis");
    store
        .group_solids(
            fixture.project_id,
            &GroupSolids {
                request_id: "2-group-draft".into(),
                expected_version: engineering_version(store, fixture.project_id).await,
                component_id: Uuid::from_u128(0x00cc_0000_0299),
                product_revision_id: draft,
                analysis_key: analysis_key(),
                geometry: vec![ReviewedGeometry { source_path: Vec::new(), source_body: 0 }],
                part_version_id: Some(fixture.products[0].part_version_id),
            },
        )
        .await
        .expect("group the draft component");

    let a_draft = ProductRevisionSelection { product_id: fixture.products[0].product_id, product_revision_id: draft };
    let report = readiness(store, &fixture, &[a_draft]).await;
    assert_eq!(report.coordination.changed_definitions.len(), 1);
    assert_eq!(report.coordination.changed_definitions[0].changes, vec![DefinitionChange::Material]);
    let required = report.coordination.required_product_ids.clone();
    assert!(required.contains(&fixture.products[0].product_id));
    assert!(required.contains(&fixture.products[1].product_id), "the other product using the shared part is required: {required:?}");

    // An A-only activation is refused and writes nothing.
    let refused = store
        .activate_revisions(
            fixture.project_id,
            &ActivateRevisions {
                request_id: "2-activation-refused".into(),
                expected_project_version: current_project_version(store, fixture.project_id).await,
                expected_accepted_baseline_id: accepted_head(store, fixture.project_id).await,
                product_revisions: vec![a_draft],
            },
        )
        .await
        .expect_err("a coordinated activation is required");
    let message = refusal_message(refused);
    assert!(message.starts_with("coordinated_activation_required: "), "{message}");
    assert!(message.contains(&fixture.products[1].product_id.to_string()), "{message}");
    assert_eq!(count(store, "accepted_baselines").await, accepted_count);
    assert_eq!(count(store, "baseline_calculations").await, snapshot_count);
    assert_eq!(current_project_version(store, fixture.project_id).await, project_version + 1, "the delivery is the only project write");

    // The complete coordinated activation changes both in one project version and one baseline.
    let coordinated = activate(store, &fixture, "2-activation-2", &[a_draft, fixture.products[1].selection()]).await;
    assert_eq!(coordinated.members.len(), 2);
    let activated = coordinated.members.iter().find(|member| member.product_id == fixture.products[0].product_id).expect("the activated member");
    assert_eq!(activated.action, ActivationAction::Activated);
    assert_eq!(activated.product_revision_id, draft);
    assert_eq!(activated.previous_active_revision_id, Some(fixture.products[0].revision_id));
    let repinned = coordinated.members.iter().find(|member| member.product_id == fixture.products[1].product_id).expect("the re-pinned member");
    assert_eq!(repinned.action, ActivationAction::Repinned);
    assert_eq!(coordinated.pinned.project_version, project_version + 2);
    assert_eq!(count(store, "accepted_baselines").await, accepted_count + 1);
    assert_eq!(count(store, "baseline_calculations").await, snapshot_count + 1);
    let baseline = store.accepted_baseline(fixture.project_id, coordinated.accepted_baseline_id).await.expect("the accepted baseline");
    assert_eq!(baseline.sequence, 2);
    assert_eq!(baseline.shared_definitions.len(), 1);
    assert_eq!(baseline.shared_definitions[0].material.as_ref().and_then(|material| material.quantity), Some(support::d("110")));
    assert!(
        coordinated
            .facts
            .iter()
            .any(|fact| matches!(fact, ActivationFact::SharedDefinitionChanged { changes, .. } if changes == &vec![DefinitionChange::Material]))
    );
    let final_view = store.project_record_view(fixture.project_id).await.expect("the project record");
    let product = final_view.document.product(fixture.products[0].product_id).expect("the product");
    assert_eq!(product.active_revision_id, Some(draft));
    assert_eq!(
        product.revisions.iter().find(|revision| revision.product_revision_id == fixture.products[0].revision_id).expect("the first revision").state,
        ProductRevisionState::Historical
    );
    db.cleanup().await;
}

#[tokio::test]
async fn a_shared_routing_or_dimension_change_requires_the_whole_group() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = fixture(store, 3, &[("A", "PL-100", "B"), ("B", "PL-100", "B")], Feasibility::Passing, true).await;
    activate(store, &fixture, "3-activation-1", &fixture.selections()).await;
    let part_version_id = fixture.products[0].part_version_id;

    // A routing change on the shared part.
    store
        .write_routing(
            fixture.project_id,
            &RoutingWrite::Edit(EditRouting {
                request_id: "3-routing-edit".into(),
                expected_version: manufacturing_version(store, fixture.project_id).await,
                routing_id: fixture.products[0].routing_id,
                operations: vec![operation(0x7100_0301, 1, "45")],
                affected_part_versions: vec![part_version_id],
                affected_products: Vec::new(),
            }),
        )
        .await
        .expect("edit the shared routing");
    let report = readiness(store, &fixture, &[fixture.products[0].selection()]).await;
    assert_eq!(report.coordination.changed_definitions.len(), 1);
    assert_eq!(report.coordination.changed_definitions[0].changes, vec![DefinitionChange::Routing]);
    let refused = store
        .activate_revisions(
            fixture.project_id,
            &ActivateRevisions {
                request_id: "3-activation-refused-routing".into(),
                expected_project_version: current_project_version(store, fixture.project_id).await,
                expected_accepted_baseline_id: accepted_head(store, fixture.project_id).await,
                product_revisions: vec![fixture.products[0].selection()],
            },
        )
        .await
        .expect_err("the routing change needs both products");
    assert!(refusal_message(refused).starts_with("coordinated_activation_required: "));
    let coordinated = activate(store, &fixture, "3-activation-2", &fixture.selections()).await;
    assert!(coordinated.members.iter().all(|member| member.action == ActivationAction::Repinned));
    assert!(coordinated.facts.iter().any(|fact| matches!(fact, ActivationFact::RevisionRepinned { .. })));
    let baseline = store.accepted_baseline(fixture.project_id, coordinated.accepted_baseline_id).await.expect("the accepted baseline");
    assert_eq!(baseline.sequence, 2);
    assert_eq!(
        baseline.shared_definitions[0].routing.as_ref().expect("the pinned routing").operations[0].cycle.as_ref().and_then(|cycle| cycle.seconds),
        Some(support::d("45"))
    );

    // A reviewed-dimension change on the shared part.
    store
        .complete_measurement(
            fixture.project_id,
            &CompleteMeasurement {
                request_id: "3-measurement".into(),
                expected_version: engineering_version(store, fixture.project_id).await,
                part_version_id,
                volume_mm3: Measurement {
                    value: Some(support::d("2500")),
                    unit: Unit::MillimetreCubed,
                    source: MeasurementSource::Manual,
                    review: ReviewState::Pending,
                    reason: Some("geometry re-measured on the bench".into()),
                    analysis_key: analysis_key(),
                },
            },
        )
        .await
        .expect("record the changed dimension");
    store
        .confirm_parts(
            fixture.project_id,
            &ConfirmParts {
                request_id: "3-measurement-confirm".into(),
                expected_version: engineering_version(store, fixture.project_id).await,
                confirmations: vec![PartConfirmation { part_version_id, classification: None, confirm_measurement: true, catalog: None }],
            },
            "engineer",
        )
        .await
        .expect("confirm the changed dimension");
    let report = readiness(store, &fixture, &[fixture.products[0].selection()]).await;
    assert_eq!(report.coordination.changed_definitions[0].changes, vec![DefinitionChange::Dimensions]);
    let refused = store
        .activate_revisions(
            fixture.project_id,
            &ActivateRevisions {
                request_id: "3-activation-refused-dimension".into(),
                expected_project_version: current_project_version(store, fixture.project_id).await,
                expected_accepted_baseline_id: accepted_head(store, fixture.project_id).await,
                product_revisions: vec![fixture.products[0].selection()],
            },
        )
        .await
        .expect_err("the dimension change needs both products");
    assert!(refusal_message(refused).starts_with("coordinated_activation_required: "));
    let final_activation = activate(store, &fixture, "3-activation-3", &fixture.selections()).await;
    let baseline = store.accepted_baseline(fixture.project_id, final_activation.accepted_baseline_id).await.expect("the accepted baseline");
    assert_eq!(baseline.sequence, 3);
    assert_eq!(baseline.shared_definitions[0].dimensions.as_ref().and_then(|dimensions| dimensions.volume_mm3), Some(support::d("2500")));
    db.cleanup().await;
}

#[tokio::test]
async fn an_unconfirmed_classification_blocks_a_fully_priced_feasible_part_until_confirmed() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = fixture(store, 4, &[("A", "PL-100", "B")], Feasibility::Passing, false).await;
    let report = readiness(store, &fixture, &fixture.selections()).await;
    let member = &report.members[0];
    assert_eq!(member.state, solidbom_contract::ReadinessState::Blocked);
    assert!(
        member.blockers.iter().any(|blocker| blocker.owner == solidbom_contract::ReadinessOwner::Engineering
            && blocker.code == "pending_classification"
            && blocker.part_version_id == Some(fixture.products[0].part_version_id)),
        "{:?}",
        member.blockers
    );
    let refused = store
        .activate_revisions(
            fixture.project_id,
            &ActivateRevisions {
                request_id: "4-activation-refused".into(),
                expected_project_version: current_project_version(store, fixture.project_id).await,
                expected_accepted_baseline_id: None,
                product_revisions: fixture.selections(),
            },
        )
        .await
        .expect_err("the unconfirmed classification blocks activation");
    let message = refusal_message(refused);
    assert!(message.starts_with("member_not_ready: "), "{message}");
    assert!(message.contains("pending_classification"), "{message}");

    store
        .confirm_parts(
            fixture.project_id,
            &ConfirmParts {
                request_id: "4-confirm".into(),
                expected_version: engineering_version(store, fixture.project_id).await,
                confirmations: vec![PartConfirmation {
                    part_version_id: fixture.products[0].part_version_id,
                    classification: Some(ClassificationConfirmation { name: "bracket".into() }),
                    confirm_measurement: true,
                    catalog: None,
                }],
            },
            "engineer",
        )
        .await
        .expect("confirm the classification and measurement");
    let report = readiness(store, &fixture, &fixture.selections()).await;
    assert_eq!(report.members[0].state, solidbom_contract::ReadinessState::Ready, "{:?}", report.members[0].blockers);
    let receipt = activate(store, &fixture, "4-activation-1", &fixture.selections()).await;
    assert_eq!(receipt.pinned.project_version, current_project_version(store, fixture.project_id).await);
    db.cleanup().await;
}

#[tokio::test]
async fn a_missing_cost_input_blocks_every_member_of_the_group() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = fixture(store, 5, &[("A", "PL-100", "B"), ("B", "PL-100", "B")], Feasibility::Passing, true).await;
    // Replace the shared sourcing record with one whose allowance is missing.
    store
        .set_sourcing(
            fixture.project_id,
            &SetSourcing {
                request_id: "5-sourcing-broken".into(),
                expected_version: manufacturing_version(store, fixture.project_id).await,
                parts: vec![manufactured(fixture.products[0].part_version_id, "100", None)],
                affected_part_versions: vec![fixture.products[0].part_version_id],
            },
            "engineer",
        )
        .await
        .expect("write the incomplete sourcing");
    let report = readiness(store, &fixture, &fixture.selections()).await;
    assert_eq!(report.members.len(), 2);
    for member in &report.members {
        assert_eq!(member.state, solidbom_contract::ReadinessState::Blocked);
        assert!(
            member
                .blockers
                .iter()
                .any(|blocker| blocker.code == "missing_allowance" && blocker.part_version_id == Some(fixture.products[0].part_version_id)),
            "{:?}",
            member.blockers
        );
    }
    let before = count(store, "project_record_versions").await;
    let refused = store
        .activate_revisions(
            fixture.project_id,
            &ActivateRevisions {
                request_id: "5-activation-refused".into(),
                expected_project_version: current_project_version(store, fixture.project_id).await,
                expected_accepted_baseline_id: None,
                product_revisions: fixture.selections(),
            },
        )
        .await
        .expect_err("the missing allowance blocks every member");
    assert!(refusal_message(refused).starts_with("member_not_ready: "));
    assert_eq!(count(store, "project_record_versions").await, before);
    // Fixing the shared input unblocks the same group.
    store
        .set_sourcing(
            fixture.project_id,
            &SetSourcing {
                request_id: "5-sourcing-fixed".into(),
                expected_version: manufacturing_version(store, fixture.project_id).await,
                parts: vec![manufactured(fixture.products[0].part_version_id, "100", Some("0.02"))],
                affected_part_versions: vec![fixture.products[0].part_version_id],
            },
            "engineer",
        )
        .await
        .expect("fix the sourcing");
    activate(store, &fixture, "5-activation-1", &fixture.selections()).await;
    db.cleanup().await;
}

#[tokio::test]
async fn an_unresolved_feasibility_finding_blocks_until_a_justified_exception_is_recorded() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = fixture(store, 6, &[("A", "PL-100", "B")], Feasibility::Failing, true).await;
    let report = readiness(store, &fixture, &fixture.selections()).await;
    assert_eq!(report.members[0].state, solidbom_contract::ReadinessState::Blocked);
    assert!(
        report.members[0].blockers.iter().any(|blocker| blocker.owner == solidbom_contract::ReadinessOwner::Feasibility
            && (blocker.code == "open_finding_failed" || blocker.code == "open_finding_conflict" || blocker.code == "open_finding_unresolved")),
        "{:?}",
        report.members[0].blockers
    );
    let refused = store
        .activate_revisions(
            fixture.project_id,
            &ActivateRevisions {
                request_id: "6-activation-refused".into(),
                expected_project_version: current_project_version(store, fixture.project_id).await,
                expected_accepted_baseline_id: None,
                product_revisions: fixture.selections(),
            },
        )
        .await
        .expect_err("the unresolved finding blocks activation");
    assert!(refusal_message(refused).starts_with("member_not_ready: "));

    // Record the justified exception for the failing finding, exactly as the SB-14 route does.
    let record = store.feasibility_record_view(fixture.project_id).await.expect("the feasibility record");
    let assessment = record.latest_assessment.as_ref().expect("the assessment");
    let failing = assessment
        .findings
        .iter()
        .find(|view| view.finding.state == solidbom_contract::FindingState::Fail)
        .expect("the failing finding")
        .finding
        .key
        .clone();
    store
        .accept_feasibility_exception(
            fixture.project_id,
            &AcceptFeasibilityException {
                request_id: "6-exception".into(),
                expected_version: record.version,
                assessment_version: assessment.assessment.version,
                finding_key: failing,
                justification: "the customer accepts the tighter bend; reviewed against the drawing".into(),
            },
            "engineer",
        )
        .await
        .expect("accept the justified exception");
    assert!(feasibility_version(store, fixture.project_id).await >= 2);
    let report = readiness(store, &fixture, &fixture.selections()).await;
    assert_eq!(report.members[0].state, solidbom_contract::ReadinessState::Ready, "{:?}", report.members[0].blockers);
    // A negotiated selling price or a commercial scenario is never a readiness input, so the
    // activation succeeds without one.
    let receipt = activate(store, &fixture, "6-activation-1", &fixture.selections()).await;
    assert_eq!(receipt.costing_state, solidbom_contract::ResultState::Complete);
    db.cleanup().await;
}

#[tokio::test]
async fn distinct_customer_revisions_activate_independently_and_an_invented_revision_is_refused() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = fixture(store, 7, &[("A", "PL-100", "B"), ("B", "PL-100", "C")], Feasibility::Passing, true).await;
    assert_ne!(fixture.part_version("B"), fixture.part_version("C"), "customer revisions stay distinct part versions");
    activate(store, &fixture, "7-activation-1", &fixture.selections()).await;
    // Change only the C-revision part's sourcing: the B-revision product is unaffected.
    store
        .set_sourcing(
            fixture.project_id,
            &SetSourcing {
                request_id: "7-sourcing-change".into(),
                expected_version: manufacturing_version(store, fixture.project_id).await,
                parts: vec![manufactured(fixture.part_version("C"), "110", Some("0.02"))],
                affected_part_versions: vec![fixture.part_version("C")],
            },
            "engineer",
        )
        .await
        .expect("change the C-revision sourcing");
    // The pending change on the C-revision part requires the product that uses that part, even
    // though the B-revision product does not cost it. The changed definitions are global.
    let report = readiness(store, &fixture, &[fixture.products[0].selection()]).await;
    assert_eq!(report.coordination.changed_definitions.len(), 1);
    assert_eq!(report.coordination.changed_definitions[0].part_version_id, fixture.part_version("C"));
    assert_eq!(report.coordination.required_product_ids, vec![fixture.products[1].product_id]);
    let independent = store
        .activate_revisions(
            fixture.project_id,
            &ActivateRevisions {
                request_id: "7-activation-2".into(),
                expected_project_version: current_project_version(store, fixture.project_id).await,
                expected_accepted_baseline_id: accepted_head(store, fixture.project_id).await,
                product_revisions: vec![fixture.products[0].selection()],
            },
        )
        .await
        .expect_err("an unrelated activation still has to include the affected product");
    let message = refusal_message(independent);
    assert!(message.starts_with("coordinated_activation_required: "), "{message}");
    assert!(message.contains(&fixture.products[1].product_id.to_string()), "{message}");
    // The C-revision product accepts the change alone; the B-revision product then activates alone.
    let concerning = activate(store, &fixture, "7-activation-3", &[fixture.products[1].selection()]).await;
    assert_eq!(concerning.members[0].action, ActivationAction::Repinned);
    let independent_after = activate(store, &fixture, "7-activation-4", &[fixture.products[0].selection()]).await;
    assert_eq!(independent_after.members[0].action, ActivationAction::Repinned);

    // An invented revision id is a malformed request; a foreign revision is not activatable.
    let baselines = count(store, "accepted_baselines").await;
    let requests = count(store, "requests").await;
    let invented = store
        .activate_revisions(
            fixture.project_id,
            &ActivateRevisions {
                request_id: "7-activation-invented".into(),
                expected_project_version: current_project_version(store, fixture.project_id).await,
                expected_accepted_baseline_id: accepted_head(store, fixture.project_id).await,
                product_revisions: vec![ProductRevisionSelection {
                    product_id: fixture.products[0].product_id,
                    product_revision_id: Uuid::from_u128(0xdead_beef),
                }],
            },
        )
        .await
        .expect_err("an invented revision is refused");
    let message = refusal_message(invented);
    assert!(message.starts_with("invalid_request: "), "{message}");
    let foreign = store
        .activate_revisions(
            fixture.project_id,
            &ActivateRevisions {
                request_id: "7-activation-foreign".into(),
                expected_project_version: current_project_version(store, fixture.project_id).await,
                expected_accepted_baseline_id: accepted_head(store, fixture.project_id).await,
                product_revisions: vec![ProductRevisionSelection {
                    product_id: fixture.products[0].product_id,
                    product_revision_id: fixture.products[1].revision_id,
                }],
            },
        )
        .await
        .expect_err("a foreign revision is refused");
    assert!(refusal_message(foreign).starts_with("revision_not_activatable: "));
    assert_eq!(count(store, "accepted_baselines").await, baselines);
    assert_eq!(count(store, "requests").await, requests);
    db.cleanup().await;
}

#[tokio::test]
async fn stale_conflicting_and_failing_activations_write_nothing() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = fixture(store, 8, &[("A", "PL-100", "B"), ("B", "PL-100", "B")], Feasibility::Passing, true).await;
    let retry_request = ActivateRevisions {
        request_id: "8-activation-1".into(),
        expected_project_version: current_project_version(store, fixture.project_id).await,
        expected_accepted_baseline_id: accepted_head(store, fixture.project_id).await,
        product_revisions: fixture.selections(),
    };
    let first = store.activate_revisions(fixture.project_id, &retry_request).await.expect("the activation");
    let version = current_project_version(store, fixture.project_id).await;
    let head = accepted_head(store, fixture.project_id).await;
    let accepted_count = count(store, "accepted_baselines").await;
    let snapshot_count = count(store, "baseline_calculations").await;
    let request_count = count(store, "requests").await;

    // A stale expected project version writes nothing.
    let stale = store
        .activate_revisions(
            fixture.project_id,
            &ActivateRevisions {
                request_id: "8-activation-stale".into(),
                expected_project_version: version - 1,
                expected_accepted_baseline_id: head,
                product_revisions: fixture.selections(),
            },
        )
        .await
        .expect_err("the stale version is a conflict");
    assert!(refusal_message(stale).starts_with("stale_project_version: "));
    // A stale expected accepted baseline writes nothing.
    let stale_head = store
        .activate_revisions(
            fixture.project_id,
            &ActivateRevisions {
                request_id: "8-activation-stale-head".into(),
                expected_project_version: version,
                expected_accepted_baseline_id: None,
                product_revisions: fixture.selections(),
            },
        )
        .await
        .expect_err("the stale accepted baseline is a conflict");
    assert!(refusal_message(stale_head).starts_with("stale_accepted_baseline: "));
    assert_eq!(count(store, "accepted_baselines").await, accepted_count);
    assert_eq!(count(store, "baseline_calculations").await, snapshot_count);
    assert_eq!(count(store, "project_record_versions").await, i64::try_from(version).expect("the version"));
    assert_eq!(count(store, "requests").await, request_count);

    // An identical retry returns the recorded receipt without writing a second time.
    let retried = store.activate_revisions(fixture.project_id, &retry_request).await.expect("the identical retry");
    assert_eq!(retried, first);
    assert_eq!(count(store, "accepted_baselines").await, accepted_count);
    // A changed payload under the used request id is a conflict.
    let changed = store
        .activate_revisions(
            fixture.project_id,
            &ActivateRevisions {
                request_id: "8-activation-1".into(),
                expected_project_version: version,
                expected_accepted_baseline_id: head,
                product_revisions: vec![fixture.products[0].selection()],
            },
        )
        .await
        .expect_err("a changed payload conflicts");
    let changed_message = refusal_message(changed);
    assert!(changed_message.contains("request ID was reused"), "{changed_message}");

    // A failure after the checks and after the project-version insert rolls everything back. The test
    // plants an accepted-baseline row naming the project version the activation will append, so the
    // final immutable insert breaks a unique constraint inside the transaction.
    let fake_calculation_id = Uuid::new_v4();
    let fake_baseline_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO baseline_calculations \
         (calculation_id, project_id, project_version, engineering_version, manufacturing_version, input_hash, calculator_version, inputs, outputs) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, '{}'::jsonb, '{}'::jsonb)",
    )
    .bind(fake_calculation_id)
    .bind(fixture.project_id)
    .bind(i64::try_from(version + 1).expect("the version"))
    .bind(i64::try_from(engineering_version(store, fixture.project_id).await).expect("the engineering version"))
    .bind(i64::try_from(manufacturing_version(store, fixture.project_id).await).expect("the manufacturing version"))
    .bind("a".repeat(64))
    .bind("solidbom-manufacturing-baseline/1")
    .execute(store.pool())
    .await
    .expect("plant the fake snapshot row");
    sqlx::query(
        "INSERT INTO accepted_baselines \
         (accepted_baseline_id, project_id, sequence, request_id, project_version, engineering_version, manufacturing_version, \
          feasibility_version, costing_calculation_id, costing_input_hash, calculator_version, costing_state, members, active_revisions, shared_definitions) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, '[]'::jsonb, '[]'::jsonb, '[]'::jsonb)",
    )
    .bind(fake_baseline_id)
    .bind(fixture.project_id)
    .bind(2_i64)
    .bind("8-planted-baseline")
    .bind(i64::try_from(version + 1).expect("the version"))
    .bind(i64::try_from(engineering_version(store, fixture.project_id).await).expect("the engineering version"))
    .bind(i64::try_from(manufacturing_version(store, fixture.project_id).await).expect("the manufacturing version"))
    .bind(i64::try_from(feasibility_version(store, fixture.project_id).await).expect("the feasibility version"))
    .bind(fake_calculation_id)
    .bind("a".repeat(64))
    .bind("solidbom-manufacturing-baseline/1")
    .bind("complete")
    .execute(store.pool())
    .await
    .expect("plant the fake accepted row");
    let before_failure = (
        count(store, "project_record_versions").await,
        count(store, "accepted_baselines").await,
        count(store, "baseline_calculations").await,
        count(store, "requests").await,
    );
    let failed = store
        .activate_revisions(
            fixture.project_id,
            &ActivateRevisions {
                request_id: "8-activation-late-failure".into(),
                expected_project_version: version,
                expected_accepted_baseline_id: Some(fake_baseline_id),
                product_revisions: fixture.selections(),
            },
        )
        .await;
    assert!(failed.is_err(), "the planted version row makes the accepted insert fail");
    let after_failure = (
        count(store, "project_record_versions").await,
        count(store, "accepted_baselines").await,
        count(store, "baseline_calculations").await,
        count(store, "requests").await,
    );
    assert_eq!(before_failure, after_failure, "the failed transaction wrote nothing");
    let view = store.project_record_view(fixture.project_id).await.expect("the project record");
    assert_eq!(view.version, version, "the project head did not move");
    db.cleanup().await;
}

#[tokio::test]
async fn the_accepted_history_and_a_historical_baseline_stay_readable_after_later_edits() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = fixture(store, 9, &[("A", "PL-100", "B")], Feasibility::Passing, true).await;
    let first = activate(store, &fixture, "9-activation-1", &fixture.selections()).await;
    let stored = store.accepted_baseline(fixture.project_id, first.accepted_baseline_id).await.expect("the first accepted baseline");
    assert_eq!(stored.sequence, 1);
    assert_eq!(stored.members.len(), 1);
    assert_eq!(stored.active_revisions[0].product_revision_id, fixture.products[0].revision_id);

    // A later routing edit and re-pin activation appends a second baseline and a later project version.
    store
        .write_routing(
            fixture.project_id,
            &RoutingWrite::Edit(EditRouting {
                request_id: "9-routing-edit".into(),
                expected_version: manufacturing_version(store, fixture.project_id).await,
                routing_id: fixture.products[0].routing_id,
                operations: vec![operation(0x7100_0901, 1, "60")],
                affected_part_versions: vec![fixture.products[0].part_version_id],
                affected_products: Vec::new(),
            }),
        )
        .await
        .expect("edit the routing");
    let second = activate(store, &fixture, "9-activation-2", &fixture.selections()).await;
    assert_eq!(second.members[0].action, ActivationAction::Repinned);
    let history = store.accepted_baselines(fixture.project_id).await.expect("the history");
    assert_eq!(history.baselines.iter().map(|baseline| baseline.sequence).collect::<Vec<_>>(), vec![2, 1]);
    // The historical read is the row that was accepted; later edits never rewrite it.
    let historical = store.accepted_baseline(fixture.project_id, first.accepted_baseline_id).await.expect("the historical baseline");
    assert_eq!(historical, stored);
    let later = store.accepted_baseline(fixture.project_id, second.accepted_baseline_id).await.expect("the later baseline");
    assert_eq!(
        later.shared_definitions[0].routing.as_ref().expect("the pinned routing").operations[0].cycle.as_ref().and_then(|cycle| cycle.seconds),
        Some(support::d("60"))
    );
    assert!(store.accepted_baseline(fixture.project_id, Uuid::new_v4()).await.is_err(), "an unknown baseline is not found");
    db.cleanup().await;
}

#[tokio::test]
async fn the_readiness_report_names_the_blocker_evidence_and_the_required_products() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = fixture(store, 10, &[("A", "PL-100", "B"), ("B", "PL-100", "B")], Feasibility::Passing, true).await;
    activate(store, &fixture, "10-activation-1", &fixture.selections()).await;
    store
        .set_sourcing(
            fixture.project_id,
            &SetSourcing {
                request_id: "10-sourcing-change".into(),
                expected_version: manufacturing_version(store, fixture.project_id).await,
                parts: vec![manufactured(fixture.products[0].part_version_id, "120", Some("0.02"))],
                affected_part_versions: vec![fixture.products[0].part_version_id],
            },
            "engineer",
        )
        .await
        .expect("change the sourcing");
    let report = readiness(store, &fixture, &[fixture.products[0].selection()]).await;
    assert_eq!(report.accepted_baseline_id, accepted_head(store, fixture.project_id).await);
    let required = report.coordination.required_product_ids.clone();
    let mut expected = vec![fixture.products[0].product_id, fixture.products[1].product_id];
    expected.sort_unstable();
    assert_eq!(required, expected);
    let changed = &report.coordination.changed_definitions[0];
    assert_eq!(changed.part_version_id, fixture.products[0].part_version_id);
    assert_eq!(changed.customer_part_number, "PL-100");
    assert_eq!(changed.customer_revision, "B");
    assert_eq!(changed.changes, vec![DefinitionChange::Material]);
    assert_eq!(changed.affected_product_ids, required);
    db.cleanup().await;
}

#[tokio::test]
async fn an_unrelated_activation_cannot_accept_a_pending_shared_change() {
    let db = TestDb::new().await;
    let store = &db.store;
    // A and B cost the shared part; C is independent.
    let fixture = fixture(store, 11, &[("A", "PL-100", "B"), ("B", "PL-100", "B"), ("C", "PL-200", "A")], Feasibility::Passing, true).await;
    activate(store, &fixture, "11-activation-1", &fixture.selections()).await;
    let shared = fixture.products[0].part_version_id;
    assert_eq!(fixture.products[1].part_version_id, shared);
    assert_ne!(fixture.products[2].part_version_id, shared);

    // The shared part's material definition changes; the pending change is visible from the whole
    // accepted pin set, not from whatever a request happens to select.
    store
        .set_sourcing(
            fixture.project_id,
            &SetSourcing {
                request_id: "11-sourcing-change".into(),
                expected_version: manufacturing_version(store, fixture.project_id).await,
                parts: vec![manufactured(shared, "130", Some("0.02"))],
                affected_part_versions: vec![shared],
            },
            "engineer",
        )
        .await
        .expect("change the shared sourcing");
    let report = readiness(store, &fixture, &[fixture.products[2].selection()]).await;
    assert_eq!(report.coordination.changed_definitions.len(), 1);
    let mut required = vec![fixture.products[0].product_id, fixture.products[1].product_id];
    required.sort_unstable();
    assert_eq!(report.coordination.required_product_ids, required);
    assert!(!report.coordination.required_product_ids.contains(&fixture.products[2].product_id));

    // C's independent activation must not silently accept the change for A and B.
    let counts =
        (count(store, "project_record_versions").await, count(store, "baseline_calculations").await, count(store, "accepted_baselines").await);
    let c_only = store
        .activate_revisions(
            fixture.project_id,
            &ActivateRevisions {
                request_id: "11-activation-c-only".into(),
                expected_project_version: current_project_version(store, fixture.project_id).await,
                expected_accepted_baseline_id: accepted_head(store, fixture.project_id).await,
                product_revisions: vec![fixture.products[2].selection()],
            },
        )
        .await
        .expect_err("an unrelated activation cannot accept a pending shared change");
    let message = refusal_message(c_only);
    assert!(message.starts_with("coordinated_activation_required: "), "{message}");
    assert!(message.contains(&fixture.products[0].product_id.to_string()), "{message}");
    assert!(message.contains(&fixture.products[1].product_id.to_string()), "{message}");
    assert!(!message.contains(&fixture.products[2].product_id.to_string()), "C is not required: {message}");
    // The affected group alone is still refused when it omits a member.
    let a_only = store
        .activate_revisions(
            fixture.project_id,
            &ActivateRevisions {
                request_id: "11-activation-a-only".into(),
                expected_project_version: current_project_version(store, fixture.project_id).await,
                expected_accepted_baseline_id: accepted_head(store, fixture.project_id).await,
                product_revisions: vec![fixture.products[0].selection()],
            },
        )
        .await
        .expect_err("A alone is still refused");
    assert!(refusal_message(a_only).contains(&fixture.products[1].product_id.to_string()));
    assert_eq!(
        (count(store, "project_record_versions").await, count(store, "baseline_calculations").await, count(store, "accepted_baselines").await),
        counts,
        "the refused activations wrote nothing"
    );

    // The complete affected group accepts the change; C is not needed.
    let accepted = activate(store, &fixture, "11-activation-ab", &[fixture.products[0].selection(), fixture.products[1].selection()]).await;
    assert_eq!(accepted.members.len(), 2);
    assert!(accepted.members.iter().all(|member| member.action == ActivationAction::Repinned));
    // Once the change is accepted, A can activate alone again.
    let later = activate(store, &fixture, "11-activation-a-after", &[fixture.products[0].selection()]).await;
    assert_eq!(later.members.len(), 1);
    assert_eq!(later.members[0].action, ActivationAction::Repinned);
    db.cleanup().await;
}

#[tokio::test]
async fn only_the_newest_draft_revision_is_activatable() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = fixture(store, 12, &[("A", "PL-100", "B")], Feasibility::Passing, true).await;
    let product = fixture.products[0].product_id;
    // A second delivery appends a newer draft revision; the first draft is now superseded.
    let source = store.put_source(&support::digest(SOURCE), "activation.step", SOURCE).await.expect("the source");
    store
        .record_delivery(
            fixture.project_id,
            &RecordDelivery {
                request_id: "12-delivery-2".into(),
                expected_version: current_project_version(store, fixture.project_id).await,
                package: DeliveryPackage { package_id: Uuid::from_u128(0xbb00_0012), name: "Package 2".into(), source },
                products: vec![DeliveredProductInput {
                    product_id: Some(product),
                    name: "A".into(),
                    kind: ProductKind::Part,
                    customer_part_number: None,
                    parts: vec![delivered_part("A-body-2", "PL-100", "B")],
                }],
            },
        )
        .await
        .expect("deliver the newer draft");
    let view = store.project_record_view(fixture.project_id).await.expect("the project record");
    let revisions = &view.document.product(product).expect("the product").revisions;
    let older = fixture.products[0].revision_id;
    let newest = revisions.last().expect("the newest revision").product_revision_id;
    assert_eq!(revisions.len(), 2);
    assert_ne!(older, newest);

    // Both routes refuse the superseded draft on both routes and write nothing.
    let counts = (count(store, "accepted_baselines").await, count(store, "requests").await);
    let readiness_error = store
        .activation_readiness(
            fixture.project_id,
            &ActivationReadinessRequest {
                project_version: current_project_version(store, fixture.project_id).await,
                selections: vec![ProductRevisionSelection { product_id: product, product_revision_id: older }],
            },
        )
        .await
        .expect_err("the superseded draft is not ready to activate");
    assert!(refusal_message(readiness_error).starts_with("revision_not_activatable: "));
    let activation_error = store
        .activate_revisions(
            fixture.project_id,
            &ActivateRevisions {
                request_id: "12-activation-older".into(),
                expected_project_version: current_project_version(store, fixture.project_id).await,
                expected_accepted_baseline_id: accepted_head(store, fixture.project_id).await,
                product_revisions: vec![ProductRevisionSelection { product_id: product, product_revision_id: older }],
            },
        )
        .await
        .expect_err("the superseded draft cannot be activated");
    assert!(refusal_message(activation_error).starts_with("revision_not_activatable: "));
    assert_eq!((count(store, "accepted_baselines").await, count(store, "requests").await), counts);

    // The newest draft is reviewed and activates.
    store
        .register_analysis(
            fixture.project_id,
            &RegisterAnalysis { request_id: "12-analysis-newest".into(), product_revision_id: newest, manifest: manifest("PL-100", "B", 129) },
        )
        .await
        .expect("register the analysis");
    store
        .group_solids(
            fixture.project_id,
            &GroupSolids {
                request_id: "12-group-newest".into(),
                expected_version: engineering_version(store, fixture.project_id).await,
                component_id: Uuid::from_u128(0x00cc_0000_1299),
                product_revision_id: newest,
                analysis_key: analysis_key(),
                geometry: vec![ReviewedGeometry { source_path: Vec::new(), source_body: 0 }],
                part_version_id: Some(fixture.products[0].part_version_id),
            },
        )
        .await
        .expect("group the newest component");
    let report = readiness(store, &fixture, &[ProductRevisionSelection { product_id: product, product_revision_id: newest }]).await;
    assert_eq!(report.members[0].state, solidbom_contract::ReadinessState::Ready, "{:?}", report.members[0].blockers);
    let receipt =
        activate(store, &fixture, "12-activation-newest", &[ProductRevisionSelection { product_id: product, product_revision_id: newest }]).await;
    assert_eq!(receipt.members[0].product_revision_id, newest);
    db.cleanup().await;
}

/// Deliver one more product whose part is fully priced and whose structure resolves through the
/// registered analysis manifest, but which has no reviewed component and no reviewed-part record at
/// all. This is the "manifest fallback" path the SB-16 costing contract documents: the occurrence's
/// shared part version is resolved from the manifest definition's customer number and revision.
async fn deliver_unreviewed_product(store: &Store, fixture: &Fixture, seed: u128, number: &str, revision: &str) -> FixtureProduct {
    let project_id = fixture.project_id;
    let source = store.put_source(&support::digest(SOURCE), "activation.step", SOURCE).await.expect("store the source");
    store
        .record_delivery(
            project_id,
            &RecordDelivery {
                request_id: format!("{seed}-delivery-2"),
                expected_version: current_project_version(store, project_id).await,
                package: DeliveryPackage { package_id: Uuid::from_u128(0xab00_0000 + seed), name: "Package 2".into(), source },
                products: vec![DeliveredProductInput {
                    product_id: None,
                    name: "Unreviewed".into(),
                    kind: ProductKind::Part,
                    customer_part_number: None,
                    parts: vec![delivered_part("unreviewed-body", number, revision)],
                }],
            },
        )
        .await
        .expect("record the second delivery");
    let view = store.project_record_view(project_id).await.expect("the project record");
    let product = view.document.products.iter().find(|product| product.name == "Unreviewed").expect("the delivered product");
    let product_id = product.product_id;
    let revision_id = product.revisions.last().expect("the delivered revision").product_revision_id;
    let part_version_id = view.document.part_version_by_identity(number, revision).expect("the delivered part version").part_version_id;
    store
        .update_product(
            project_id,
            &UpdateProduct {
                request_id: format!("{seed}-unreviewed-product"),
                expected_version: current_project_version(store, project_id).await,
                product_id,
                name: "Unreviewed".into(),
                kind: ProductKind::Part,
                customer_part_number: None,
                schedule_override: Some(YearRange { sop_year: 2027, eop_year: 2027 }),
                forecasts: vec![AnnualForecast { year: 2027, quantity: 100 }],
            },
        )
        .await
        .expect("set the forecast");
    store
        .register_analysis(
            project_id,
            &RegisterAnalysis {
                request_id: format!("{seed}-unreviewed-analysis"),
                product_revision_id: revision_id,
                manifest: manifest(number, revision, seed * 10 + 99),
            },
        )
        .await
        .expect("register the analysis");
    // Complete manufacturing definition: the part is priced, but no engineer has reviewed it.
    store
        .set_sourcing(
            project_id,
            &SetSourcing {
                request_id: format!("{seed}-unreviewed-sourcing"),
                expected_version: manufacturing_version(store, project_id).await,
                parts: vec![manufactured(part_version_id, "100", Some("0.02"))],
                affected_part_versions: vec![part_version_id],
            },
            "engineer",
        )
        .await
        .expect("set the sourcing");
    let routing_id = Uuid::from_u128(0x7700_0000_0000_0000_0000_0000_0000_0000 + seed);
    store
        .write_routing(
            project_id,
            &RoutingWrite::Manual(ManualRouting {
                request_id: format!("{seed}-unreviewed-routing"),
                expected_version: manufacturing_version(store, project_id).await,
                routing_id,
                level: RoutingLevel::Part,
                owner: CostOwner::PartVersion { part_version_id },
                operations: vec![operation(0x7100_0000_0000_0000_0000_0000_0000_0000 + seed, 1, "30")],
                affected_part_versions: vec![part_version_id],
                affected_products: Vec::new(),
            }),
        )
        .await
        .expect("write the routing");
    FixtureProduct { product_id, revision_id, part_version_id, component_id: Uuid::nil(), routing_id, revision_label: revision.into() }
}

/// A shared part version used by a member's resulting costed structure needs an engineer-confirmed
/// classification and reviewed measurement even when the costing manifest fallback could resolve its
/// identity without a reviewed component. Without this explicit check the engineering record's
/// computed issues never visit a part that has no review at all, so a fully priced but never-reviewed
/// part would pass the gate and activate.
#[tokio::test]
async fn an_unreviewed_part_blocks_readiness_and_activation_writes_nothing() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = fixture(store, 131, &[("Reviewed", "AA-1", "A")], Feasibility::Passing, true).await;
    let unreviewed = deliver_unreviewed_product(store, &fixture, 131, "BB-1", "A").await;

    let report = readiness(store, &fixture, &[unreviewed.selection()]).await;
    let member = report.members.first().expect("one selected member");
    assert_eq!(member.state, solidbom_contract::ReadinessState::Blocked, "an unreviewed part cannot be ready: {:?}", member.blockers);
    let engineering_codes: BTreeSet<&str> = member
        .blockers
        .iter()
        .filter(|blocker| blocker.owner == solidbom_contract::ReadinessOwner::Engineering)
        .map(|blocker| blocker.code.as_str())
        .collect();
    assert!(engineering_codes.contains("pending_classification"), "the unreviewed part has no confirmed classification: {engineering_codes:?}");
    assert!(engineering_codes.contains("missing_reviewed_volume"), "the unreviewed part has no reviewed measurement: {engineering_codes:?}");
    assert!(
        member.blockers.iter().any(|blocker| blocker.part_version_id == Some(unreviewed.part_version_id)),
        "the blocker names the unreviewed part version: {:?}",
        member.blockers
    );

    let before = (
        current_project_version(store, fixture.project_id).await,
        count(store, "project_record_versions").await,
        count(store, "baseline_calculations").await,
        count(store, "accepted_baselines").await,
        count(store, "requests").await,
    );
    let error = store
        .activate_revisions(
            fixture.project_id,
            &ActivateRevisions {
                request_id: "131-activation-unreviewed".into(),
                expected_project_version: current_project_version(store, fixture.project_id).await,
                expected_accepted_baseline_id: accepted_head(store, fixture.project_id).await,
                product_revisions: vec![unreviewed.selection()],
            },
        )
        .await
        .expect_err("an unreviewed part cannot be activated");
    assert!(refusal_message(error).starts_with("member_not_ready: "), "the authoritative gate blocks the unreviewed part");
    let after = (
        current_project_version(store, fixture.project_id).await,
        count(store, "project_record_versions").await,
        count(store, "baseline_calculations").await,
        count(store, "accepted_baselines").await,
        count(store, "requests").await,
    );
    assert_eq!(before, after, "the refused activation writes nothing");
    db.cleanup().await;
}
