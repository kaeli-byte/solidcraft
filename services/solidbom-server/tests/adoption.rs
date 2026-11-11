//! Store-level tests for dependency updates, impact reports, explicit adoption and draft
//! publication (package SB-20, adoption contract version 1 additive to API version 2).
//!
//! `#[tokio::test]` is not recognised as a test function by Clippy's test allowance, so this file
//! allows the panicking helpers it needs. The library and binary still deny them.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

mod support;

use solidbom_contract::{
    ActivateRevisions, ActivationReceipt, AdoptRoutingTemplate, AnalysisKey, AnalysisManifest, AnnualForecast, AppConfigVersionPublished,
    BaselineSnapshot, CatalogComponentVersion, CatalogConfirmation, ClassificationConfirmation, CommercialScenarioContent, CompleteMeasurement,
    ConfirmParts, CostCenterPurpose, CostOwner, CostingVolumePolicy, CreateCommercialScenario, CreateFeasibilityRuleTableVersion, CreateProject,
    CycleBasis, CycleOrigin, DefinitionRef, DeliveredPartInput, DeliveredProductInput, DeliveryPackage, DependencyDecisionAction,
    DependencyDecisionItem, DependencyDecisionReceipt, DependencyFact, DependencyRefusal, DraftPublication, DraftStaleReason,
    FeasibilityComponentEvidence, FeasibilityConnector, FeasibilityRule, FeasibilityRuleKind, FeasibilityTubeAttributes, FeasibilityTubeEnd,
    FeasibilityTubeMeasurements, GeometryBinding, GroupSolids, ManualRouting, MaterialConsumption, Measurement, MeasurementSource, NumericRangeMm,
    OccurrenceRef, OperationCycle, PartConfirmation, ProductKind, ProductRevisionSelection, ProjectDependencyReport, ProjectDetails,
    ProjectWriteOutcome, PublishRoutingTemplate, RecordDelivery, RecordDependencyDecisions, RecordFeasibilityAssessment, RegisterAnalysis,
    ReviewState, RoutingLevel, RoutingOperation, RoutingWrite, RuleApplicability, RuleLimit, SCHEMA_VERSION, SetSourcing, SetupTime, SourcedPart,
    Sourcing, ToolingRecoveryInputs, TubeEnd, TubeKind, Unit, UpdateProduct, UpdateProject, UpdateProjectCosting, YearRange,
};
use solidbom_server::{Store, StoreError};
use support::TestDb;
use uuid::Uuid;

const SOURCE: &[u8] = b"ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('adoption fixture'),'2;1');\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n";
const IDENTITY: [[f64; 4]; 4] = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];

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
enum SourcingKind {
    Manufactured,
    Purchased,
}

#[derive(Clone, Copy)]
enum RoutingKind {
    Manual,
    Template,
    None,
}

struct FixtureProduct {
    product_id: Uuid,
    revision_id: Uuid,
    part_version_id: Uuid,
    component_id: Uuid,
    routing_id: Uuid,
}

struct Fixture {
    project_id: Uuid,
    component_id: Uuid,
    config_id: Uuid,
    config_version_id: Uuid,
    template_id: Uuid,
    products: Vec<FixtureProduct>,
}

async fn project_version(store: &Store, project_id: Uuid) -> u64 {
    store.project_record_view(project_id).await.expect("the project record view").version
}

async fn engineering_version(store: &Store, project_id: Uuid) -> u64 {
    store.engineering_record_view(project_id).await.expect("the engineering record view").version
}

async fn manufacturing_version(store: &Store, project_id: Uuid) -> u64 {
    store.manufacturing_record_view(project_id).await.expect("the manufacturing record view").version
}

async fn count(store: &Store, table: &str) -> i64 {
    support::row_count(store, table).await
}

async fn counts(store: &Store, tables: &[&str]) -> Vec<i64> {
    let mut values = Vec::with_capacity(tables.len());
    for table in tables {
        values.push(count(store, table).await);
    }
    values
}

const READ_ONLY_TABLES: &[&str] = &[
    "project_records",
    "project_record_versions",
    "engineering_records",
    "engineering_record_versions",
    "manufacturing_records",
    "manufacturing_record_versions",
    "baseline_calculations",
    "dependency_decisions",
    "requests",
    "commercial_scenarios",
    "accepted_baselines",
    "reference_changes",
];

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
    let definition_id = Uuid::from_u128(0xde20_0000 + seed * 10);
    let occurrence_id = Uuid::from_u128(0x0cc0_2000 + seed * 10);
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

fn manufactured(part_version_id: Uuid) -> SourcedPart {
    SourcedPart {
        part_version_id,
        sourcing: Sourcing::Manufactured,
        material: Some(MaterialConsumption {
            material_version_id: Some(support::material_id()),
            quantity: Some(support::d("100")),
            unit: Unit::CentimetreCubed,
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

/// Build a fully reviewed project whose products are priced from the manufacturing definition or a
/// confirmed catalog reference. One shared material, one production/setup rate pair and one
/// optional routing (manual or adopted from a template) are the shared dependencies a test updates.
async fn fixture(store: &Store, seed: u128, names: &[&str], sourcing: SourcingKind, routing: RoutingKind, confirm_catalog: bool) -> Fixture {
    let project_id = Uuid::from_u128(0xb200_0000 + seed);
    let source = store.put_source(&support::digest(SOURCE), "adoption.step", SOURCE).await.expect("store the source");
    store
        .create_project(&CreateProject { request_id: format!("{seed}-create"), project_id, details: details("Dependency inquiry") })
        .await
        .expect("create the project");
    let delivered: Vec<DeliveredProductInput> = names
        .iter()
        .map(|name| DeliveredProductInput {
            product_id: None,
            name: (*name).into(),
            kind: ProductKind::Part,
            customer_part_number: None,
            parts: vec![delivered_part(&format!("{name}-body"), &format!("P-{seed}-{name}"), "A")],
        })
        .collect();
    store
        .record_delivery(
            project_id,
            &RecordDelivery {
                request_id: format!("{seed}-delivery"),
                expected_version: 1,
                package: DeliveryPackage { package_id: Uuid::from_u128(0xaa20_0000 + seed), name: "Package 1".into(), source },
                products: delivered,
            },
        )
        .await
        .expect("record the delivery");
    let view = store.project_record_view(project_id).await.expect("the project record view");

    // The shared catalog version a confirmation names, and the app configuration the calculator
    // records in `category_config`.
    let component_id = Uuid::from_u128(0xc200_0000 + seed);
    let catalog: Option<CatalogComponentVersion> = if confirm_catalog {
        Some(
            store
                .create_component_version(&support::component_version(&format!("{seed}-catalog-1"), 0, component_id, Some("12.5")), "administrator")
                .await
                .expect("publish the catalog component")
                .component,
        )
    } else {
        None
    };
    let config_id = Uuid::from_u128(0xcf20_0000 + seed);
    let config: AppConfigVersionPublished = store
        .create_app_config_version(&support::app_config_version(&format!("{seed}-config-1"), 0, config_id, Vec::new()), "administrator")
        .await
        .expect("publish the app configuration");

    let mut products = Vec::with_capacity(names.len());
    for (index, name) in names.iter().enumerate() {
        let product = view.document.products.iter().find(|product| product.name == *name).expect("the delivered product");
        let product_id = product.product_id;
        let revision_id = product.revisions.first().expect("the delivered revision").product_revision_id;
        let number = format!("P-{seed}-{name}");
        let part_version_id = view.document.part_version_by_identity(&number, "A").expect("the delivered part version").part_version_id;
        let version = project_version(store, project_id).await;
        assert_eq!(version, 2 + index as u64, "each fixture product appends exactly one project version");
        store
            .update_product(
                project_id,
                &UpdateProduct {
                    request_id: format!("{seed}-product-{index}"),
                    expected_version: version,
                    product_id,
                    name: (*name).into(),
                    kind: ProductKind::Part,
                    customer_part_number: Some(number.clone()),
                    schedule_override: Some(YearRange { sop_year: 2027, eop_year: 2027 }),
                    forecasts: vec![AnnualForecast { year: 2027, quantity: 100 }],
                },
            )
            .await
            .expect("set the forecast");
        let mut engineering_version = engineering_version(store, project_id).await;
        store
            .register_analysis(
                project_id,
                &RegisterAnalysis {
                    request_id: format!("{seed}-analysis-{index}"),
                    product_revision_id: revision_id,
                    manifest: manifest(&number, "A", seed * 16 + index as u128),
                },
            )
            .await
            .expect("register the analysis");
        let component_id = Uuid::from_u128(0xcc20_0000 + seed * 16 + index as u128);
        store
            .group_solids(
                project_id,
                &GroupSolids {
                    request_id: format!("{seed}-group-{index}"),
                    expected_version: engineering_version,
                    component_id,
                    product_revision_id: revision_id,
                    analysis_key: analysis_key(),
                    geometry: vec![solidbom_contract::ReviewedGeometry { source_path: Vec::new(), source_body: 0 }],
                    part_version_id: Some(part_version_id),
                },
            )
            .await
            .expect("group the reviewed component");
        engineering_version += 1;
        store
            .complete_measurement(
                project_id,
                &CompleteMeasurement {
                    request_id: format!("{seed}-measurement-{index}"),
                    expected_version: engineering_version,
                    part_version_id,
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
        engineering_version += 1;
        store
            .confirm_parts(
                project_id,
                &ConfirmParts {
                    request_id: format!("{seed}-confirm-{index}"),
                    expected_version: engineering_version,
                    confirmations: vec![PartConfirmation {
                        part_version_id,
                        classification: Some(ClassificationConfirmation { name: "bracket".into() }),
                        confirm_measurement: true,
                        catalog: catalog
                            .as_ref()
                            .map(|catalog| CatalogConfirmation { component_version_id: catalog.component_version_id, project_override: None }),
                    }],
                },
                "engineer",
            )
            .await
            .expect("confirm the reviewed part");
        assert_eq!(project_version(store, project_id).await, version + 1, "the forecast write appended one project version");
        products.push(FixtureProduct {
            product_id,
            revision_id,
            part_version_id,
            component_id,
            routing_id: Uuid::from_u128(0x7720_0000 + seed * 16 + index as u128),
        });
    }

    let current_manufacturing = manufacturing_version(store, project_id).await;
    let parts: Vec<SourcedPart> = products
        .iter()
        .map(|product| match sourcing {
            SourcingKind::Manufactured => manufactured(product.part_version_id),
            SourcingKind::Purchased => purchased(product.part_version_id),
        })
        .collect();
    store
        .set_sourcing(
            project_id,
            &SetSourcing {
                request_id: format!("{seed}-sourcing"),
                expected_version: current_manufacturing,
                parts,
                affected_part_versions: products.iter().map(|product| product.part_version_id).collect(),
            },
            "engineer",
        )
        .await
        .expect("set the sourcing");

    let template_id = Uuid::from_u128(0x7120_0000 + seed);
    match routing {
        RoutingKind::Manual => {
            for (index, product) in products.iter().enumerate() {
                let version = manufacturing_version(store, project_id).await;
                store
                    .write_routing(
                        project_id,
                        &RoutingWrite::Manual(ManualRouting {
                            request_id: format!("{seed}-routing-{index}"),
                            expected_version: version,
                            routing_id: product.routing_id,
                            level: RoutingLevel::Part,
                            owner: CostOwner::PartVersion { part_version_id: product.part_version_id },
                            operations: vec![operation(0x7220_0000 + seed * 16 + index as u128, 1, "30")],
                            affected_part_versions: vec![product.part_version_id],
                            affected_products: Vec::new(),
                        }),
                    )
                    .await
                    .expect("write the routing");
            }
        }
        RoutingKind::Template => {
            store
                .publish_routing_template(&PublishRoutingTemplate {
                    request_id: format!("{seed}-template-1"),
                    expected_version: 0,
                    template_id,
                    name: "Reusable tube routing".into(),
                    description: None,
                    applies_to: vec![RoutingLevel::Part],
                    operations: vec![operation(0x7320_0000 + seed, 1, "30")],
                    source_reference: None,
                })
                .await
                .expect("publish the template");
            for (index, product) in products.iter().enumerate() {
                let version = manufacturing_version(store, project_id).await;
                store
                    .write_routing(
                        project_id,
                        &RoutingWrite::AdoptTemplate(AdoptRoutingTemplate {
                            request_id: format!("{seed}-routing-{index}"),
                            expected_version: version,
                            routing_id: product.routing_id,
                            level: RoutingLevel::Part,
                            owner: CostOwner::PartVersion { part_version_id: product.part_version_id },
                            template: solidbom_contract::TemplateRef { template_id, version: 1 },
                            affected_part_versions: vec![product.part_version_id],
                            affected_products: Vec::new(),
                        }),
                    )
                    .await
                    .expect("adopt the template");
            }
        }
        RoutingKind::None => {}
    }

    store
        .update_project_costing(
            project_id,
            &UpdateProjectCosting {
                request_id: format!("{seed}-costing"),
                expected_version: project_version(store, project_id).await,
                costing: solidbom_contract::ProjectCosting {
                    volume_policy: Some(CostingVolumePolicy::Average),
                    exchange_rates: Vec::new(),
                    purchase_prices: Vec::new(),
                },
            },
        )
        .await
        .expect("write the costing settings");

    Fixture { project_id, component_id, config_id, config_version_id: config.config.config_version_id, template_id, products }
}

async fn dependency_report(store: &Store, project_id: Uuid) -> ProjectDependencyReport {
    store.project_dependency_report(project_id).await.expect("the dependency report")
}

fn decisions(
    request_id: &str,
    expected_version: u64,
    action: DependencyDecisionAction,
    items: Vec<DependencyDecisionItem>,
) -> RecordDependencyDecisions {
    RecordDependencyDecisions { request_id: request_id.into(), expected_version, action, decisions: items }
}

fn item(kind: solidbom_contract::SharedUpdateKind, record_id: Uuid, version_id: Uuid) -> DependencyDecisionItem {
    DependencyDecisionItem { kind, record_id, version_id }
}

async fn adopt(
    store: &Store,
    project_id: Uuid,
    request_id: &str,
    items: Vec<DependencyDecisionItem>,
) -> Result<DependencyDecisionReceipt, StoreError> {
    let version = project_version(store, project_id).await;
    store.record_dependency_decisions(project_id, &decisions(request_id, version, DependencyDecisionAction::Adopted, items), "engineer").await
}

async fn acknowledge(
    store: &Store,
    project_id: Uuid,
    request_id: &str,
    items: Vec<DependencyDecisionItem>,
) -> Result<DependencyDecisionReceipt, StoreError> {
    let version = project_version(store, project_id).await;
    store.record_dependency_decisions(project_id, &decisions(request_id, version, DependencyDecisionAction::Acknowledged, items), "engineer").await
}

async fn recalculate(store: &Store, project_id: Uuid, request_id: &str) -> Result<DraftPublication, StoreError> {
    store
        .recalculate_draft(
            project_id,
            &solidbom_contract::RecalculateDraft {
                request_id: request_id.into(),
                expected_project_version: project_version(store, project_id).await,
                expected_engineering_version: engineering_version(store, project_id).await,
                expected_manufacturing_version: manufacturing_version(store, project_id).await,
                product_revisions: Vec::new(),
            },
        )
        .await
}

/// The fixture connector the feasibility rules and evidence name.
const CONNECTOR_ID: u128 = 0xc0;

/// The passing feasibility rule table of one project: every rule is satisfied by the fixture
/// evidence, so a recorded assessment yields passing findings.
fn passing_rules(seed: u128) -> Vec<FeasibilityRule> {
    let source_reference = "design rules/".to_string();
    vec![
        FeasibilityRule {
            rule_id: Uuid::from_u128(0xfe20_0000 + seed * 100 + 1),
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
            rule_id: Uuid::from_u128(0xfe20_0000 + seed * 100 + 2),
            kind: FeasibilityRuleKind::BendSpacing,
            applicability: RuleApplicability::default(),
            limit: RuleLimit::MinimumLengthMm { length_mm: support::d("20") },
            source_reference: source_reference.clone(),
            note: None,
        },
        FeasibilityRule {
            rule_id: Uuid::from_u128(0xfe20_0000 + seed * 100 + 3),
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
            rule_id: Uuid::from_u128(0xfe20_0000 + seed * 100 + 4),
            kind: FeasibilityRuleKind::EndAssemblyLength,
            applicability: RuleApplicability { connector_component_id: Some(Uuid::from_u128(CONNECTOR_ID)), ..RuleApplicability::default() },
            limit: RuleLimit::MinimumLengthMm { length_mm: support::d("25") },
            source_reference,
            note: None,
        },
    ]
}

/// The passing tube evidence of one reviewed fixture component.
fn tube_evidence(revision: Uuid, component_id: Uuid) -> FeasibilityComponentEvidence {
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
            bend_radii_mm: vec![support::d("35"), support::d("40")],
            tangent_straight_lengths_mm: vec![support::d("30")],
            straight_end_lengths_mm: vec![support::d("40"), support::d("40")],
        }),
        ends: vec![
            FeasibilityTubeEnd { end: TubeEnd::Start, connector: Some(connector) },
            FeasibilityTubeEnd { end: TubeEnd::End, connector: Some(connector) },
        ],
    }
}

/// Publish one passing feasibility rule-table version.
async fn publish_passing_rules(store: &Store, fixture: &Fixture, request_id: &str, expected_version: u64, version_seed: u128) -> Uuid {
    let rule_table_id = Uuid::from_u128(0xfa20_0000 + (fixture.project_id.as_u128() & 0xffff));
    store
        .publish_feasibility_rules(
            rule_table_id,
            &CreateFeasibilityRuleTableVersion {
                request_id: request_id.into(),
                expected_version,
                rule_table_id,
                name: "Reviewed design rules".into(),
                rules: passing_rules(version_seed),
                source_reference: Some("design rules/".into()),
            },
            "administrator",
        )
        .await
        .expect("publish the feasibility rule table")
        .rule_table
        .rule_table_version_id
}

/// Record one passing feasibility assessment of the fixture's first reviewed component.
async fn record_passing_assessment(store: &Store, fixture: &Fixture, request_id: &str, rule_table_version_id: Uuid) {
    let product = &fixture.products[0];
    let expected_version = store.feasibility_record_view(fixture.project_id).await.expect("the feasibility record view").version;
    store
        .record_feasibility_assessment(
            fixture.project_id,
            &RecordFeasibilityAssessment {
                request_id: request_id.into(),
                expected_version,
                rule_table_version_id,
                components: vec![tube_evidence(product.revision_id, product.component_id)],
            },
            "engineer",
        )
        .await
        .expect("record the feasibility assessment");
}

/// Publish version 1 of the passing rule table and record one passing assessment.
async fn seed_feasibility(store: &Store, fixture: &Fixture) {
    let rule_table_version_id = publish_passing_rules(store, fixture, "feasibility-rules", 0, 1).await;
    record_passing_assessment(store, fixture, "feasibility-assessment", rule_table_version_id).await;
}

/// Activate every fixture revision, which records one accepted baseline beside its snapshot.
async fn activate(store: &Store, fixture: &Fixture) -> ActivationReceipt {
    store
        .activate_revisions(
            fixture.project_id,
            &ActivateRevisions {
                request_id: "activate-1".into(),
                expected_project_version: project_version(store, fixture.project_id).await,
                expected_accepted_baseline_id: None,
                product_revisions: fixture
                    .products
                    .iter()
                    .map(|product| ProductRevisionSelection { product_id: product.product_id, product_revision_id: product.revision_id })
                    .collect(),
            },
        )
        .await
        .expect("activate the fixture revisions")
}

fn refusal_message(error: StoreError) -> String {
    match error {
        StoreError::Conflict(message) | StoreError::Invalid(message) => message,
        other => panic!("expected a typed refusal, got {other:?}"),
    }
}

fn purchase_price(snapshot: &BaselineSnapshot) -> Option<rust_decimal::Decimal> {
    snapshot
        .inputs
        .parts
        .first()
        .and_then(|part| part.purchase.as_ref())
        .and_then(|purchase| purchase.single_price.as_ref())
        .map(|price| price.amount)
}

#[tokio::test]
async fn a_catalog_update_flags_only_dependent_projects() {
    let db = TestDb::new().await;
    let store = &db.store;
    let dependent = fixture(store, 1, &["bracket"], SourcingKind::Purchased, RoutingKind::None, true).await;
    let independent = fixture(store, 2, &["bracket"], SourcingKind::Purchased, RoutingKind::None, true).await;

    // Publish version 2 of the dependent project's catalog component only.
    let catalog_v2 = store
        .create_component_version(&support::component_version("catalog-2", 1, dependent.component_id, Some("20")), "administrator")
        .await
        .expect("publish the catalog update")
        .component;

    let before = counts(store, READ_ONLY_TABLES).await;
    let dependent_report = dependency_report(store, dependent.project_id).await;
    let independent_report = dependency_report(store, independent.project_id).await;
    let _ = store.shared_updates().await.expect("the shared-update catalog");
    let filtered = store
        .dependency_projects(Some((solidbom_contract::SharedUpdateKind::CatalogComponent, dependent.component_id)))
        .await
        .expect("the filtered project list");
    let all = store.dependency_projects(None).await.expect("the global project list");
    assert_eq!(counts(store, READ_ONLY_TABLES).await, before, "reading updates, reports and project lists appends no row");

    assert_eq!(dependent_report.dependencies.len(), 1);
    assert_eq!(dependent_report.pending_count, 1, "the pending count names the true total");
    assert_eq!(independent_report.pending_count, 0);
    let dependency = &dependent_report.dependencies[0];
    assert_eq!(dependency.kind, solidbom_contract::SharedUpdateKind::CatalogComponent);
    assert_eq!(dependency.record_id, dependent.component_id);
    assert_eq!(dependency.referenced_version, 1);
    assert_eq!(dependency.available_version, 2);
    assert_eq!(dependency.available_version_id, catalog_v2.component_version_id);
    assert_eq!(dependency.part_version_ids, vec![dependent.products[0].part_version_id]);
    assert_eq!(dependency.product_ids, vec![dependent.products[0].product_id]);
    assert!(!dependency.acknowledged);
    assert!(independent_report.dependencies.is_empty(), "a catalog change flags only the projects that resolve it");
    assert!(independent_report.decisions.is_empty());

    assert_eq!(filtered.projects.len(), 1);
    assert_eq!(filtered.projects[0].project_id, dependent.project_id);
    assert_eq!(filtered.projects[0].pending_count, 1);
    assert_eq!(filtered.projects[0].kinds, vec![solidbom_contract::SharedUpdateKind::CatalogComponent]);
    assert_eq!(all.projects.len(), 1);
    assert_eq!(all.projects[0].project_id, dependent.project_id);

    // An unknown filter value is refused instead of looking like an empty list.
    let unknown = store
        .dependency_projects(Some((solidbom_contract::SharedUpdateKind::CatalogComponent, Uuid::from_u128(0xdead))))
        .await
        .expect_err("an unknown filter record is refused");
    assert!(matches!(unknown, StoreError::Invalid(message) if message.starts_with("dependency_projects.filter:")));

    // A project with no baseline result has no draft and flags no scenario.
    let bare_project_id = Uuid::from_u128(0xb200_0101);
    store
        .create_project(&CreateProject { request_id: "bare-create".into(), project_id: bare_project_id, details: details("Bare inquiry") })
        .await
        .expect("create the bare project");
    let bare = dependency_report(store, bare_project_id).await;
    assert!(bare.draft.is_none());
    assert!(bare.flagged_scenarios.is_empty());
    assert_eq!(bare.pending_count, 0);

    let updates = store.shared_updates().await.expect("the shared-update catalog");
    let catalog_update = updates
        .updates
        .iter()
        .find(|update| update.kind == solidbom_contract::SharedUpdateKind::CatalogComponent && update.record_id == dependent.component_id)
        .expect("the catalog update is listed");
    assert_eq!(catalog_update.version, 2);
    assert_eq!(catalog_update.version_id, catalog_v2.component_version_id);
}

#[tokio::test]
async fn acknowledgement_is_not_adoption() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = fixture(store, 3, &["bracket"], SourcingKind::Purchased, RoutingKind::None, true).await;
    let catalog_v2 = store
        .create_component_version(&support::component_version("catalog-3", 1, fixture.component_id, Some("20")), "administrator")
        .await
        .expect("publish the catalog update")
        .component;
    let first = recalculate(store, fixture.project_id, "first-draft").await.expect("the first draft");
    assert_eq!(
        purchase_price(&store.baseline_calculation(fixture.project_id, first.calculation.calculation_id).await.expect("the snapshot")),
        Some(support::d("12.5"))
    );
    let engineering_before = engineering_version(store, fixture.project_id).await;
    let project_before = project_version(store, fixture.project_id).await;

    let receipt = acknowledge(
        store,
        fixture.project_id,
        "ack-1",
        vec![item(solidbom_contract::SharedUpdateKind::CatalogComponent, fixture.component_id, catalog_v2.component_version_id)],
    )
    .await
    .expect("acknowledge the update");
    assert_eq!(receipt.project_version, project_before);
    assert_eq!(receipt.decisions.len(), 1);
    assert_eq!(receipt.decisions[0].sequence, 1);
    assert!(receipt.decisions[0].from_version_id.is_some(), "the acknowledgement names the referenced version");
    assert!(matches!(receipt.facts[0], DependencyFact::DependencyAcknowledged { .. }));
    assert_eq!(engineering_version(store, fixture.project_id).await, engineering_before, "an acknowledgement appends no record version");
    assert_eq!(project_version(store, fixture.project_id).await, project_before);

    let report = dependency_report(store, fixture.project_id).await;
    assert_eq!(report.dependencies.len(), 1, "acknowledgement is not adoption: the dependency stays pending");
    assert!(report.dependencies[0].acknowledged);
    assert_eq!(report.decisions.len(), 1);
    let draft = report.draft.as_ref().expect("the first draft is the newest result");
    assert!(draft.stale);
    assert!(draft.stale_reasons.contains(&DraftStaleReason::NewerSharedData));

    // A recalculation still uses the older, acknowledged-but-not-adopted catalog version.
    let second = recalculate(store, fixture.project_id, "second-draft").await.expect("the second draft");
    let snapshot = store.baseline_calculation(fixture.project_id, second.calculation.calculation_id).await.expect("the second snapshot");
    assert_eq!(purchase_price(&snapshot), Some(support::d("12.5")));
}

#[tokio::test]
async fn adoption_repins_the_catalog_then_recalculation_creates_a_new_result() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = fixture(store, 4, &["bracket"], SourcingKind::Purchased, RoutingKind::None, true).await;
    let first = recalculate(store, fixture.project_id, "first-draft").await.expect("the first draft");
    let first_snapshot = store.baseline_calculation(fixture.project_id, first.calculation.calculation_id).await.expect("the first snapshot");
    let catalog_v2_published = store
        .create_component_version(&support::component_version("catalog-4", 1, fixture.component_id, Some("20")), "administrator")
        .await
        .expect("publish the catalog update");
    let catalog_v2 = catalog_v2_published.component.clone();
    let project_before = project_version(store, fixture.project_id).await;
    let engineering_before = engineering_version(store, fixture.project_id).await;

    let receipt = adopt(
        store,
        fixture.project_id,
        "adopt-catalog",
        vec![item(solidbom_contract::SharedUpdateKind::CatalogComponent, fixture.component_id, catalog_v2.component_version_id)],
    )
    .await
    .expect("adopt the catalog update");
    assert_eq!(receipt.project_version, project_before);
    assert_eq!(project_version(store, fixture.project_id).await, project_before, "adoption appends no project version");
    assert_eq!(engineering_version(store, fixture.project_id).await, engineering_before + 1, "one engineering record version is appended");
    assert_eq!(receipt.decisions.len(), 1);
    assert_eq!(receipt.decisions[0].action, DependencyDecisionAction::Adopted);
    assert_eq!(receipt.decisions[0].to_version_id, catalog_v2.component_version_id);
    assert_eq!(receipt.engineering_version, engineering_before + 1, "the receipt names the engineering head after the transaction");
    assert_eq!(receipt.manufacturing_version, manufacturing_version(store, fixture.project_id).await);
    assert!(matches!(receipt.facts[0], DependencyFact::DependencyAdopted { .. }));

    // The new engineering record pins the v2 values; the old record version stays readable.
    let current = store.engineering_record_view(fixture.project_id).await.expect("the engineering record");
    let confirmed =
        current.document.part(fixture.products[0].part_version_id).and_then(|part| part.catalog.confirmed.clone()).expect("the confirmed reference");
    assert_eq!(confirmed.component_version_id, catalog_v2.component_version_id);
    assert_eq!(confirmed.version, 2);
    assert_eq!(confirmed.price, Some(support::d("20")));
    assert_eq!(confirmed.confirmed_by, "engineer");
    let old = store.engineering_record_version(fixture.project_id, engineering_before).await.expect("the old engineering record");
    let old_confirmed =
        old.document.part(fixture.products[0].part_version_id).and_then(|part| part.catalog.confirmed.clone()).expect("the old confirmed reference");
    assert_eq!(old_confirmed.component_version_id, catalog_v2_published.change.previous_version_id.expect("the superseded version"));

    let second = recalculate(store, fixture.project_id, "second-draft").await.expect("the second draft");
    let second_snapshot = store.baseline_calculation(fixture.project_id, second.calculation.calculation_id).await.expect("the second snapshot");
    assert_eq!(purchase_price(&second_snapshot), Some(support::d("20")), "the recalculation costs the adopted catalog price");
    assert_ne!(second_snapshot.calculation_id, first_snapshot.calculation_id);
    let reread = store.baseline_calculation(fixture.project_id, first_snapshot.calculation_id).await.expect("the first snapshot");
    assert_eq!(reread, first_snapshot, "the earlier snapshot stays byte-identical");
    assert!(dependency_report(store, fixture.project_id).await.dependencies.is_empty(), "the adopted dependency disappears from the report");
}

#[tokio::test]
async fn material_and_rate_adoption_repins_the_manufacturing_record() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = fixture(store, 5, &["tube"], SourcingKind::Manufactured, RoutingKind::Manual, false).await;
    let manufacturing_before = manufacturing_version(store, fixture.project_id).await;
    let old_document = store.manufacturing_record_version(fixture.project_id, manufacturing_before).await.expect("the old manufacturing record");
    let material_v2 = store
        .create_material_version(&support::material_version("material-v2", 1, support::material_id(), "2.7", Some("30")), "administrator")
        .await
        .expect("publish the material update")
        .material;
    let rate_v2 = store
        .create_rate_version(&support::rate_version("rate-v2", 1, production_rate_id(), Some("150"), CostCenterPurpose::Production), "administrator")
        .await
        .expect("publish the rate update")
        .cost_center;
    let project_before = project_version(store, fixture.project_id).await;

    let receipt = adopt(
        store,
        fixture.project_id,
        "adopt-manufacturing",
        vec![
            item(solidbom_contract::SharedUpdateKind::Material, support::material_id(), material_v2.material_version_id),
            item(solidbom_contract::SharedUpdateKind::CostCenterRate, production_rate_id(), rate_v2.rate_version_id),
        ],
    )
    .await
    .expect("adopt the material and rate updates");
    assert_eq!(receipt.decisions.len(), 2);
    assert_eq!(receipt.manufacturing_version, manufacturing_before + 1);
    assert_eq!(receipt.engineering_version, engineering_version(store, fixture.project_id).await);
    assert_eq!(project_version(store, fixture.project_id).await, project_before, "adoption appends no project version");
    assert_eq!(manufacturing_version(store, fixture.project_id).await, manufacturing_before + 1, "one manufacturing record version for both repins");

    let current = store.manufacturing_record_view(fixture.project_id).await.expect("the manufacturing record");
    let part = current.document.part(fixture.products[0].part_version_id).expect("the sourced part");
    let material_version_id = part.material.as_ref().and_then(|material| material.material_version_id);
    assert_eq!(material_version_id, Some(material_v2.material_version_id));
    let routing = current.document.routing(fixture.products[0].routing_id).expect("the applied routing");
    assert_eq!(routing.operations[0].cycle.as_ref().and_then(|cycle| cycle.production_rate_version_id), Some(rate_v2.rate_version_id));
    assert_eq!(routing.operations[0].setup.as_ref().and_then(|setup| setup.setup_rate_version_id), Some(setup_rate_id()));

    let reread_old = store.manufacturing_record_version(fixture.project_id, manufacturing_before).await.expect("the old manufacturing record");
    assert_eq!(reread_old, old_document, "the earlier manufacturing record version stays byte-identical");

    let publication = recalculate(store, fixture.project_id, "after-manufacturing-adoption").await.expect("the publication");
    let snapshot = store.baseline_calculation(fixture.project_id, publication.calculation.calculation_id).await.expect("the snapshot");
    let material = snapshot.inputs.parts[0].material.as_ref().expect("the material input");
    assert_eq!(material.material_version_id, Some(material_v2.material_version_id));
    assert_eq!(material.price_per_kg, Some(support::d("30")), "the number re-resolution uses the adopted material price");
    let operation = snapshot.inputs.parts[0].operations.first().expect("the operation input");
    assert_eq!(
        operation.cycle.as_ref().and_then(|cycle| cycle.rate_per_hour),
        Some(support::d("150")),
        "the number re-resolution uses the adopted rate"
    );
}

#[tokio::test]
async fn a_newer_app_configuration_is_not_used_until_adopted() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = fixture(store, 6, &["tube"], SourcingKind::Manufactured, RoutingKind::Manual, false).await;
    let first = recalculate(store, fixture.project_id, "first-draft").await.expect("the first draft");
    let first_snapshot = store.baseline_calculation(fixture.project_id, first.calculation.calculation_id).await.expect("the first snapshot");
    assert_eq!(first_snapshot.inputs.category_config.as_ref().map(|config| config.config_version_id), Some(fixture.config_version_id));
    let scenario_id = Uuid::from_u128(0x5200_0006);
    store
        .create_commercial_scenario(
            fixture.project_id,
            &CreateCommercialScenario {
                request_id: "config-scenario".into(),
                scenario_id,
                content: CommercialScenarioContent {
                    name: "config scenario".into(),
                    baseline_calculation_id: first.calculation.calculation_id,
                    manual_prices: Vec::new(),
                    tooling: ToolingRecoveryInputs::default(),
                    productivity_applies_to_tooling: false,
                },
            },
        )
        .await
        .expect("create the saved scenario");

    let config_v2 = store
        .create_app_config_version(
            &support::app_config_version("config-6-v2", 1, fixture.config_id, vec![support::fixed_category("packaging", "5")]),
            "administrator",
        )
        .await
        .expect("publish the app configuration update")
        .config;
    let report = dependency_report(store, fixture.project_id).await;
    let dependency = report
        .dependencies
        .iter()
        .find(|dependency| dependency.kind == solidbom_contract::SharedUpdateKind::AppConfig)
        .expect("the app configuration update is flagged");
    assert_eq!(dependency.record_id, fixture.config_id);
    assert_eq!(dependency.referenced_version, 1);
    assert_eq!(dependency.available_version_id, config_v2.config_version_id);

    let still_v1 = recalculate(store, fixture.project_id, "second-draft").await.expect("the recalculation");
    let still_v1_snapshot = store.baseline_calculation(fixture.project_id, still_v1.calculation.calculation_id).await.expect("the snapshot");
    assert_eq!(
        still_v1_snapshot.inputs.category_config.as_ref().map(|config| config.config_version_id),
        Some(fixture.config_version_id),
        "a newer published configuration is not used before it is adopted"
    );
    // The new draft is stale because a costing-relevant shared update is pending, and the scenario
    // pinned to it is flagged with the stale-draft reason.
    let pending_report = dependency_report(store, fixture.project_id).await;
    assert_eq!(pending_report.pending_count, 1);
    let pending_draft = pending_report.draft.as_ref().expect("a draft result exists");
    assert!(pending_draft.stale_reasons.contains(&DraftStaleReason::NewerSharedData));
    assert_eq!(pending_report.flagged_scenarios.len(), 1);
    assert_eq!(pending_report.flagged_scenarios[0].scenario_id, scenario_id);
    assert!(
        pending_report.flagged_scenarios[0]
            .stale_reasons
            .iter()
            .any(|reason| { reason.code == "scenario_baseline_outdated" && reason.message.contains("newer baseline snapshot") })
    );

    // A second scenario pins the newest draft result, which the pending app-configuration update
    // makes stale; after the adoption it is the stale-draft case.
    let newest_scenario_id = Uuid::from_u128(0x5200_0606);
    store
        .create_commercial_scenario(
            fixture.project_id,
            &CreateCommercialScenario {
                request_id: "config-scenario-2".into(),
                scenario_id: newest_scenario_id,
                content: CommercialScenarioContent {
                    name: "newest draft scenario".into(),
                    baseline_calculation_id: still_v1.calculation.calculation_id,
                    manual_prices: Vec::new(),
                    tooling: ToolingRecoveryInputs::default(),
                    productivity_applies_to_tooling: false,
                },
            },
        )
        .await
        .expect("create the newest-draft scenario");
    let engineering_before = engineering_version(store, fixture.project_id).await;
    let manufacturing_before = manufacturing_version(store, fixture.project_id).await;
    let project_before = project_version(store, fixture.project_id).await;
    adopt(
        store,
        fixture.project_id,
        "adopt-config",
        vec![item(solidbom_contract::SharedUpdateKind::AppConfig, fixture.config_id, config_v2.config_version_id)],
    )
    .await
    .expect("adopt the app configuration");
    assert_eq!(engineering_version(store, fixture.project_id).await, engineering_before);
    assert_eq!(manufacturing_version(store, fixture.project_id).await, manufacturing_before);
    assert_eq!(project_version(store, fixture.project_id).await, project_before);

    // Adopting the configuration moves no record version, but it was recorded after the newest
    // draft result read its inputs, so that result is stale and its scenario is flagged.
    let after_adoption = dependency_report(store, fixture.project_id).await;
    assert!(after_adoption.dependencies.is_empty());
    let stale_draft = after_adoption.draft.as_ref().expect("the earlier draft result is still the newest draft");
    assert_eq!(stale_draft.calculation_id, still_v1.calculation.calculation_id);
    assert!(stale_draft.stale);
    assert_eq!(stale_draft.stale_reasons, vec![DraftStaleReason::NewerSharedData]);
    assert_eq!(after_adoption.flagged_scenarios.len(), 2);
    let stale_scenario = after_adoption
        .flagged_scenarios
        .iter()
        .find(|scenario| scenario.scenario_id == newest_scenario_id)
        .expect("the scenario pinned to the now-stale newest draft");
    assert!(
        stale_scenario
            .stale_reasons
            .iter()
            .any(|reason| reason.code == "scenario_baseline_outdated" && reason.message.contains("newest draft result is stale"))
    );

    let adopted = recalculate(store, fixture.project_id, "third-draft").await.expect("the recalculation after adoption");
    let adopted_snapshot = store.baseline_calculation(fixture.project_id, adopted.calculation.calculation_id).await.expect("the snapshot");
    assert_eq!(adopted_snapshot.inputs.category_config.as_ref().map(|config| config.config_version_id), Some(config_v2.config_version_id));
    let fresh_report = dependency_report(store, fixture.project_id).await;
    let fresh_draft = fresh_report.draft.as_ref().expect("the publication is the newest draft result");
    assert!(!fresh_draft.stale, "the publication read the adopted configuration and no update is pending");
    assert!(!fresh_draft.stale_reasons.contains(&DraftStaleReason::NewerSharedData));
    assert_eq!(fresh_report.flagged_scenarios.len(), 2);
    assert!(fresh_report.flagged_scenarios.iter().all(|scenario| {
        scenario.stale_reasons.iter().all(|reason| reason.code != "scenario_baseline_outdated" || reason.message.contains("newer baseline snapshot"))
    }));

    // A project with no prior resolution keeps the SB-16 behaviour: the latest published version.
    let fresh_project_id = Uuid::from_u128(0xb200_0600);
    store
        .create_project(&CreateProject { request_id: "fresh-create".into(), project_id: fresh_project_id, details: details("Fresh inquiry") })
        .await
        .expect("create the fresh project");
    let fresh = store
        .calculate_baseline(
            fresh_project_id,
            &solidbom_contract::CalculateBaseline {
                request_id: "fresh-baseline".into(),
                project_version: 1,
                engineering_version: None,
                manufacturing_version: None,
                product_revisions: Vec::new(),
            },
        )
        .await
        .expect("calculate the fresh baseline");
    assert_eq!(fresh.inputs.category_config.as_ref().map(|config| config.config_version_id), Some(config_v2.config_version_id));
}

#[tokio::test]
async fn a_feasibility_reassessment_clears_the_dependency() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = fixture(store, 13, &["tube"], SourcingKind::Manufactured, RoutingKind::Manual, false).await;
    seed_feasibility(store, &fixture).await;
    assert!(
        dependency_report(store, fixture.project_id)
            .await
            .dependencies
            .iter()
            .all(|dependency| { dependency.kind != solidbom_contract::SharedUpdateKind::FeasibilityRuleTable }),
        "the assessment names the newest rule-table version, so it is not a dependency"
    );

    // A newer rule table flags the project from the latest recorded assessment only.
    let rule_table_version_id = publish_passing_rules(store, &fixture, "feasibility-rules-2", 1, 2).await;
    let report = dependency_report(store, fixture.project_id).await;
    let feasibility: Vec<&solidbom_contract::ProjectDependency> =
        report.dependencies.iter().filter(|dependency| dependency.kind == solidbom_contract::SharedUpdateKind::FeasibilityRuleTable).collect();
    assert_eq!(feasibility.len(), 1, "a superseded assessment is not a second phantom dependency");
    assert_eq!(feasibility[0].referenced_version, 1);
    assert_eq!(feasibility[0].available_version, 2);
    assert!(feasibility[0].part_version_ids.is_empty());
    assert_eq!(feasibility[0].product_ids, vec![fixture.products[0].product_id]);
    assert!(
        !report.draft.as_ref().is_some_and(|draft| draft.stale_reasons.contains(&DraftStaleReason::NewerSharedData)),
        "a pending feasibility rule table is flagged but never makes a draft result stale"
    );

    // Re-assessing the current review against the new rules clears the dependency.
    record_passing_assessment(store, &fixture, "feasibility-assessment-2", rule_table_version_id).await;
    assert!(
        dependency_report(store, fixture.project_id)
            .await
            .dependencies
            .iter()
            .all(|dependency| { dependency.kind != solidbom_contract::SharedUpdateKind::FeasibilityRuleTable }),
        "the re-assessment names the newest rule table, so the dependency disappears"
    );
}

#[tokio::test]
async fn a_forecast_edit_publishes_a_recalculated_draft() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = fixture(store, 7, &["tube"], SourcingKind::Manufactured, RoutingKind::Manual, false).await;
    // Record an accepted state first, so the publication can be proven not to mutate it.
    seed_feasibility(store, &fixture).await;
    let activation = activate(store, &fixture).await;
    let accepted_before = store.accepted_baseline(fixture.project_id, activation.accepted_baseline_id).await.expect("the accepted baseline");
    // The activation snapshot is accepted history: the newest draft result is an earlier snapshot.
    let before_edit = dependency_report(store, fixture.project_id).await;
    assert_ne!(
        before_edit.draft.as_ref().map(|draft| draft.calculation_id),
        Some(accepted_before.costing_calculation_id),
        "an activation snapshot is not a draft result"
    );
    let pinned_snapshot_before =
        store.baseline_calculation(fixture.project_id, accepted_before.costing_calculation_id).await.expect("the accepted snapshot");
    let scenario_id = Uuid::from_u128(0x5200_0007);
    store
        .create_commercial_scenario(
            fixture.project_id,
            &CreateCommercialScenario {
                request_id: "scenario-1".into(),
                scenario_id,
                content: CommercialScenarioContent {
                    name: "negotiated".into(),
                    baseline_calculation_id: accepted_before.costing_calculation_id,
                    manual_prices: Vec::new(),
                    tooling: ToolingRecoveryInputs::default(),
                    productivity_applies_to_tooling: false,
                },
            },
        )
        .await
        .expect("create the saved scenario");

    let outcome: ProjectWriteOutcome = store
        .update_product(
            fixture.project_id,
            &UpdateProduct {
                request_id: "forecast-edit".into(),
                expected_version: project_version(store, fixture.project_id).await,
                product_id: fixture.products[0].product_id,
                name: "tube".into(),
                kind: ProductKind::Part,
                customer_part_number: Some("P-7-tube".into()),
                schedule_override: Some(YearRange { sop_year: 2027, eop_year: 2027 }),
                forecasts: vec![AnnualForecast { year: 2027, quantity: 200 }],
            },
        )
        .await
        .expect("edit the forecast");
    let publication = outcome.draft.expect("a forecast edit publishes a draft calculation in the same transaction");
    assert_eq!(publication.calculation.project_version, project_version(store, fixture.project_id).await);
    let snapshot = store.baseline_calculation(fixture.project_id, publication.calculation.calculation_id).await.expect("the new snapshot");
    assert_eq!(snapshot.inputs.products[0].forecasts, vec![AnnualForecast { year: 2027, quantity: 200 }]);
    assert_ne!(snapshot.outputs.project_total_manufacturing_cost, pinned_snapshot_before.outputs.project_total_manufacturing_cost);
    assert_eq!(publication.flagged_scenarios.len(), 1);
    assert_eq!(publication.flagged_scenarios[0].scenario_id, scenario_id);
    assert_eq!(publication.flagged_scenarios[0].stale_reasons[0].code, "scenario_baseline_outdated");

    // The accepted baseline row and the snapshot it pinned stay byte-identical.
    let accepted_after = store.accepted_baseline(fixture.project_id, activation.accepted_baseline_id).await.expect("the accepted baseline");
    assert_eq!(accepted_after, accepted_before, "a publication never mutates an accepted baseline");
    let pinned_snapshot_after =
        store.baseline_calculation(fixture.project_id, accepted_before.costing_calculation_id).await.expect("the accepted snapshot");
    assert_eq!(pinned_snapshot_after, pinned_snapshot_before, "a publication never mutates the snapshot an accepted baseline pinned");
    assert_eq!(count(store, "accepted_baselines").await, 1);
    let report = dependency_report(store, fixture.project_id).await;
    assert_eq!(report.draft.as_ref().map(|draft| draft.calculation_id), Some(snapshot.calculation_id));
    assert_eq!(report.flagged_scenarios.len(), 1);
}

#[tokio::test]
async fn an_exclusion_publishes_a_recalculated_draft() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = fixture(store, 8, &["left", "right"], SourcingKind::Manufactured, RoutingKind::Manual, false).await;
    let first = recalculate(store, fixture.project_id, "first-draft").await.expect("the first draft");
    let first_snapshot = store.baseline_calculation(fixture.project_id, first.calculation.calculation_id).await.expect("the first snapshot");
    assert_eq!(first_snapshot.outputs.products.len(), 2);
    let scenario_id = Uuid::from_u128(0x5200_0008);
    store
        .create_commercial_scenario(
            fixture.project_id,
            &CreateCommercialScenario {
                request_id: "scenario-1".into(),
                scenario_id,
                content: CommercialScenarioContent {
                    name: "negotiated".into(),
                    baseline_calculation_id: first.calculation.calculation_id,
                    manual_prices: Vec::new(),
                    tooling: ToolingRecoveryInputs::default(),
                    productivity_applies_to_tooling: false,
                },
            },
        )
        .await
        .expect("create the saved scenario");
    let accepted_before = count(store, "accepted_baselines").await;

    let outcome = store
        .exclude_products(
            fixture.project_id,
            &solidbom_contract::ExcludeProducts {
                request_id: "exclude-right".into(),
                expected_version: project_version(store, fixture.project_id).await,
                product_ids: vec![fixture.products[1].product_id],
            },
        )
        .await
        .expect("exclude the product");
    let publication = outcome.draft.expect("an exclusion publishes a draft calculation in the same transaction");
    let snapshot = store.baseline_calculation(fixture.project_id, publication.calculation.calculation_id).await.expect("the new snapshot");
    assert_eq!(snapshot.outputs.products.len(), 1, "the new draft recomputes demand without the excluded product");
    assert_eq!(snapshot.outputs.products[0].product_id, fixture.products[0].product_id);
    assert_eq!(publication.flagged_scenarios.len(), 1);
    let reread = store.baseline_calculation(fixture.project_id, first_snapshot.calculation_id).await.expect("the earlier snapshot");
    assert_eq!(reread, first_snapshot);
    assert_eq!(count(store, "accepted_baselines").await, accepted_before);
}

#[tokio::test]
async fn missing_inputs_produce_an_incomplete_draft_result() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = fixture(store, 9, &["tube"], SourcingKind::Manufactured, RoutingKind::Manual, false).await;
    // Clear the forecast: a missing input is an `incomplete` result with a named issue, never a
    // zero-substituted current-looking number, and the automatic publication still happens.
    let outcome = store
        .update_product(
            fixture.project_id,
            &UpdateProduct {
                request_id: "clear-forecast".into(),
                expected_version: project_version(store, fixture.project_id).await,
                product_id: fixture.products[0].product_id,
                name: "tube".into(),
                kind: ProductKind::Part,
                customer_part_number: Some("P-9-tube".into()),
                schedule_override: None,
                forecasts: Vec::new(),
            },
        )
        .await
        .expect("clear the forecast");
    let automatic = outcome.draft.expect("the forecast edit publishes an incomplete draft");
    let automatic_snapshot =
        store.baseline_calculation(fixture.project_id, automatic.calculation.calculation_id).await.expect("the automatic snapshot");
    assert_eq!(automatic_snapshot.outputs.state, solidbom_contract::ResultState::Incomplete);
    assert!(automatic_snapshot.outputs.issues.iter().any(|issue| issue.code == "missing_forecast"));

    let explicit = recalculate(store, fixture.project_id, "explicit-incomplete").await.expect("the explicit publication");
    let explicit_snapshot = store.baseline_calculation(fixture.project_id, explicit.calculation.calculation_id).await.expect("the explicit snapshot");
    assert_eq!(explicit_snapshot.outputs.state, solidbom_contract::ResultState::Incomplete);
    assert!(explicit_snapshot.outputs.issues.iter().any(|issue| issue.code == "missing_forecast"));
}

#[tokio::test]
async fn a_default_schedule_edit_publishes_a_recalculated_draft() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = fixture(store, 14, &["tube"], SourcingKind::Manufactured, RoutingKind::Manual, false).await;
    let project_version_before = project_version(store, fixture.project_id).await;
    let schedule = YearRange { sop_year: 2028, eop_year: 2028 };
    let outcome = store
        .update_project(
            fixture.project_id,
            &UpdateProject {
                request_id: "schedule-edit".into(),
                expected_version: project_version_before,
                details: ProjectDetails { default_schedule: schedule, ..details("Dependency inquiry") },
            },
        )
        .await
        .expect("edit the default schedule");
    let publication = outcome.draft.expect("a default-schedule edit publishes a draft calculation");
    assert_eq!(publication.calculation.project_version, project_version_before + 1);
    assert_eq!(publication.calculation.engineering_version, engineering_version(store, fixture.project_id).await);
    assert_eq!(publication.calculation.manufacturing_version, manufacturing_version(store, fixture.project_id).await);

    // Changing any other details field does not publish.
    let unchanged = store
        .update_project(
            fixture.project_id,
            &UpdateProject {
                request_id: "name-edit".into(),
                expected_version: project_version(store, fixture.project_id).await,
                details: ProjectDetails { name: "Renamed inquiry".into(), default_schedule: schedule, ..details("Dependency inquiry") },
            },
        )
        .await
        .expect("edit the name only");
    assert!(unchanged.draft.is_none(), "a details edit that keeps the default schedule publishes nothing");
}

#[tokio::test]
async fn a_stale_generation_conflict_writes_nothing() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = fixture(store, 10, &["tube"], SourcingKind::Manufactured, RoutingKind::Manual, false).await;
    let _ = recalculate(store, fixture.project_id, "first-draft").await.expect("the first draft");
    let observed_project = project_version(store, fixture.project_id).await;
    let observed_engineering = engineering_version(store, fixture.project_id).await;
    let observed_manufacturing = manufacturing_version(store, fixture.project_id).await;
    // A newer edit moves the project generation after the client observed it.
    store
        .update_product(
            fixture.project_id,
            &UpdateProduct {
                request_id: "newer-edit".into(),
                expected_version: observed_project,
                product_id: fixture.products[0].product_id,
                name: "tube".into(),
                kind: ProductKind::Part,
                customer_part_number: Some("P-10-tube".into()),
                schedule_override: Some(YearRange { sop_year: 2027, eop_year: 2027 }),
                forecasts: vec![AnnualForecast { year: 2027, quantity: 150 }],
            },
        )
        .await
        .expect("a newer forecast edit");
    let snapshots_before = count(store, "baseline_calculations").await;
    let requests_before = count(store, "requests").await;
    let stale = store
        .recalculate_draft(
            fixture.project_id,
            &solidbom_contract::RecalculateDraft {
                request_id: "stale-generation".into(),
                expected_project_version: observed_project,
                expected_engineering_version: observed_engineering,
                expected_manufacturing_version: observed_manufacturing,
                product_revisions: Vec::new(),
            },
        )
        .await
        .expect_err("the stale generation is refused");
    let message = refusal_message(stale);
    assert!(message.starts_with("stale_draft_generation:"), "{message}");
    assert!(message.contains("newer_project_version"), "{message}");
    assert_eq!(count(store, "baseline_calculations").await, snapshots_before, "a stale publication writes no snapshot");
    assert_eq!(count(store, "requests").await, requests_before, "a stale publication records no receipt");

    let valid = recalculate(store, fixture.project_id, "valid-draft").await.expect("the current generation publishes");
    assert!(valid.dependencies.is_empty());
}

#[tokio::test]
async fn a_failed_adoption_writes_nothing() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = fixture(store, 11, &["tube"], SourcingKind::Manufactured, RoutingKind::Template, false).await;
    // Two pending updates: a material update and a routing-template update (flagged only).
    let material_v2 = store
        .create_material_version(&support::material_version("material-v2", 1, support::material_id(), "2.7", Some("30")), "administrator")
        .await
        .expect("publish the material update")
        .material;
    store
        .publish_routing_template(&PublishRoutingTemplate {
            request_id: "template-2".into(),
            expected_version: 1,
            template_id: fixture.template_id,
            name: "Reusable tube routing".into(),
            description: None,
            applies_to: vec![RoutingLevel::Part],
            operations: vec![operation(0x7330_0011, 1, "25")],
            source_reference: None,
        })
        .await
        .expect("publish the routing-template update");
    let report = dependency_report(store, fixture.project_id).await;
    assert!(report.dependencies.iter().any(|dependency| dependency.kind == solidbom_contract::SharedUpdateKind::RoutingTemplate));
    assert!(report.dependencies.iter().any(|dependency| dependency.kind == solidbom_contract::SharedUpdateKind::Material));
    let template_update = report
        .dependencies
        .iter()
        .find(|dependency| dependency.kind == solidbom_contract::SharedUpdateKind::RoutingTemplate)
        .expect("the routing-template dependency");
    let decisions_before = count(store, "dependency_decisions").await;
    let manufacturing_before = manufacturing_version(store, fixture.project_id).await;
    let requests_before = count(store, "requests").await;

    // A two-item adoption whose second item is not adoptable writes nothing at all.
    let unsupported = adopt(
        store,
        fixture.project_id,
        "failed-adoption",
        vec![
            item(solidbom_contract::SharedUpdateKind::Material, support::material_id(), material_v2.material_version_id),
            item(solidbom_contract::SharedUpdateKind::RoutingTemplate, fixture.template_id, template_update.available_version_id),
        ],
    )
    .await
    .expect_err("a routing-template update is flagged only");
    let message = refusal_message(unsupported);
    assert!(message.starts_with("unsupported_adoption:"), "{message}");
    assert_eq!(count(store, "dependency_decisions").await, decisions_before);
    assert_eq!(manufacturing_version(store, fixture.project_id).await, manufacturing_before);
    assert_eq!(count(store, "requests").await, requests_before);

    // A stale named version is refused too, and still writes nothing.
    let material_v3 = store
        .create_material_version(&support::material_version("material-v3", 2, support::material_id(), "2.7", Some("40")), "administrator")
        .await
        .expect("publish a newer material version")
        .material;
    // Publishing a shared version records its own receipt in the shared request log.
    let requests_before = count(store, "requests").await;
    let stale = adopt(
        store,
        fixture.project_id,
        "stale-adoption",
        vec![item(solidbom_contract::SharedUpdateKind::Material, support::material_id(), material_v2.material_version_id)],
    )
    .await
    .expect_err("the named version is no longer the newest");
    let message = refusal_message(stale);
    assert!(message.starts_with("stale_update:"), "{message}");
    assert_eq!(count(store, "dependency_decisions").await, decisions_before);
    assert_eq!(manufacturing_version(store, fixture.project_id).await, manufacturing_before);
    assert_eq!(count(store, "requests").await, requests_before);

    // A valid adoption after the failed ones still works.
    let receipt = adopt(
        store,
        fixture.project_id,
        "valid-adoption",
        vec![item(solidbom_contract::SharedUpdateKind::Material, support::material_id(), material_v3.material_version_id)],
    )
    .await
    .expect("the valid adoption");
    assert_eq!(receipt.decisions.len(), 1);
    assert_eq!(count(store, "dependency_decisions").await, decisions_before + 1);
    assert_eq!(manufacturing_version(store, fixture.project_id).await, manufacturing_before + 1);
}

#[tokio::test]
async fn every_read_is_point_in_time_and_the_decision_history_is_append_only() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = fixture(store, 12, &["bracket"], SourcingKind::Purchased, RoutingKind::None, true).await;
    let catalog_v2 = store
        .create_component_version(&support::component_version("catalog-12", 1, fixture.component_id, Some("20")), "administrator")
        .await
        .expect("publish the catalog update")
        .component;
    acknowledge(
        store,
        fixture.project_id,
        "ack-1",
        vec![item(solidbom_contract::SharedUpdateKind::CatalogComponent, fixture.component_id, catalog_v2.component_version_id)],
    )
    .await
    .expect("acknowledge");
    let report = dependency_report(store, fixture.project_id).await;
    let engineering_versions = count(store, "engineering_record_versions").await;
    let manufacturing_versions = count(store, "manufacturing_record_versions").await;
    let second = dependency_report(store, fixture.project_id).await;
    assert_eq!(second, report, "a second read of an unchanged project answers the same report");
    assert_eq!(count(store, "engineering_record_versions").await, engineering_versions);
    assert_eq!(count(store, "manufacturing_record_versions").await, manufacturing_versions);
    assert_eq!(second.decisions.len(), 1);
    assert_eq!(second.decisions[0].sequence, 1, "the decision history is append-only and newest first");

    // The dependency stays listed, acknowledged but not adopted; no decision row disappears.
    assert_eq!(second.dependencies.len(), 1);
    assert!(second.dependencies[0].acknowledged);
    assert!(second.draft.is_some());
}

/// A publication binds the exact generation it read; the refusal vocabulary is the contract's.
#[test]
fn the_publication_refusal_names_the_moved_generation() {
    let refusal =
        DependencyRefusal::StaleDraftGeneration { reasons: vec![DraftStaleReason::NewerProjectVersion, DraftStaleReason::NewerEngineeringVersion] };
    assert_eq!(refusal.code(), "stale_draft_generation");
    assert_eq!(refusal.error_code(), solidbom_contract::ApiErrorCode::Conflict);
    let message = refusal.message();
    assert!(message.contains("newer_project_version"), "{message}");
    assert!(message.contains("newer_engineering_version"), "{message}");
}

/// An adoption is history, not a claim about the present reference state: a later owning-package
/// write may legitimately reference the older shared version again. The report must show that new
/// pending dependency instead of refusing its own history, and the update must be adoptable again.
#[tokio::test]
async fn an_adoption_can_be_repeated_after_a_later_older_reference() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = fixture(store, 21, &["tube"], SourcingKind::Manufactured, RoutingKind::Manual, true).await;
    let project_id = fixture.project_id;
    let part_version_id = fixture.products[0].part_version_id;
    let reference = store.reference_data().await.expect("the published reference data");
    let component_v1 = reference
        .components
        .iter()
        .find(|component| component.component_id == fixture.component_id && component.version == 1)
        .expect("the confirmed catalog version")
        .component_version_id;
    let material_v1 = reference
        .materials
        .iter()
        .find(|material| material.material_id == support::material_id() && material.version == 1)
        .expect("the selected material version")
        .material_version_id;

    // Publish and adopt the catalog and material updates.
    let component_v2 = store
        .create_component_version(&support::component_version("revert-catalog-2", 1, fixture.component_id, Some("18.0")), "administrator")
        .await
        .expect("publish the catalog update")
        .component
        .component_version_id;
    let material_v2 = store
        .create_material_version(&support::material_version("revert-material-2", 1, support::material_id(), "2.7", Some("30")), "administrator")
        .await
        .expect("publish the material update")
        .material
        .material_version_id;
    adopt(
        store,
        project_id,
        "revert-adopt-first",
        vec![
            item(solidbom_contract::SharedUpdateKind::CatalogComponent, fixture.component_id, component_v2),
            item(solidbom_contract::SharedUpdateKind::Material, support::material_id(), material_v2),
        ],
    )
    .await
    .expect("adopt both updates");
    assert_eq!(dependency_report(store, project_id).await.pending_count, 0);

    // The owning commands legitimately introduce references to the older versions again.
    store
        .confirm_parts(
            project_id,
            &ConfirmParts {
                request_id: "revert-confirm-old-catalog".into(),
                expected_version: engineering_version(store, project_id).await,
                confirmations: vec![PartConfirmation {
                    part_version_id,
                    classification: None,
                    confirm_measurement: false,
                    catalog: Some(CatalogConfirmation { component_version_id: component_v1, project_override: None }),
                }],
            },
            "engineer",
        )
        .await
        .expect("confirm the older catalog version again");
    store
        .set_sourcing(
            project_id,
            &SetSourcing {
                request_id: "revert-sourcing-old-material".into(),
                expected_version: manufacturing_version(store, project_id).await,
                parts: vec![manufactured(part_version_id)],
                affected_part_versions: vec![part_version_id],
            },
            "engineer",
        )
        .await
        .expect("reference the older material version again");

    // The report carries the earlier adoption rows and the newly pending dependencies together.
    let report = dependency_report(store, project_id).await;
    assert_eq!(report.pending_count, 2);
    let catalog = report
        .dependencies
        .iter()
        .find(|dependency| dependency.kind == solidbom_contract::SharedUpdateKind::CatalogComponent)
        .expect("the newly outdated catalog reference is pending again");
    assert_eq!(catalog.referenced_version_id, component_v1);
    assert_eq!(catalog.available_version_id, component_v2);
    let material = report
        .dependencies
        .iter()
        .find(|dependency| dependency.kind == solidbom_contract::SharedUpdateKind::Material)
        .expect("the newly outdated material reference is pending again");
    assert_eq!(material.referenced_version_id, material_v1);
    assert_eq!(material.available_version_id, material_v2);
    assert_eq!(
        report.decisions.iter().filter(|decision| decision.action == solidbom_contract::DependencyDecisionAction::Adopted).count(),
        2,
        "the historical adoptions stay readable"
    );

    // A genuinely pending update is adoptable again; only an identical acknowledgement is a
    // duplicate. The new rows append with higher sequences.
    let receipt = adopt(
        store,
        project_id,
        "revert-adopt-again",
        vec![
            item(solidbom_contract::SharedUpdateKind::CatalogComponent, fixture.component_id, component_v2),
            item(solidbom_contract::SharedUpdateKind::Material, support::material_id(), material_v2),
        ],
    )
    .await
    .expect("re-adopt the newly pending updates");
    assert_eq!(receipt.decisions.len(), 2);
    assert!(receipt.decisions.iter().all(|decision| decision.sequence > 2));
    assert_eq!(dependency_report(store, project_id).await.pending_count, 0);
}

/// Adoption ordering is the append-only decision sequence, never a wall-clock timestamp: an
/// adoption whose stored `decided_at` is earlier than the result it must invalidate still marks that
/// result stale, and a result that consumed the adoption records the sequence and stays current.
#[tokio::test]
async fn adoption_ordering_uses_the_sequence_not_the_timestamp() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = fixture(store, 22, &["tube"], SourcingKind::Manufactured, RoutingKind::Manual, false).await;
    let project_id = fixture.project_id;
    let first = recalculate(store, project_id, "sequence-first").await.expect("the first draft");
    let config_v2 = store
        .create_app_config_version(
            &support::app_config_version("sequence-config-2", 1, fixture.config_id, vec![support::fixed_category("packaging", "5")]),
            "administrator",
        )
        .await
        .expect("publish the app configuration update")
        .config;

    // Simulate the transaction-start anomaly: the adoption row carries a timestamp one second before
    // the snapshot it invalidates, but its append-only sequence is higher.
    sqlx::query(
        "INSERT INTO dependency_decisions \
         (decision_id, project_id, sequence, kind, record_id, from_version_id, to_version_id, action, decided_by, decided_at) \
         VALUES ($1, $2, (SELECT COALESCE(MAX(sequence), 0) + 1 FROM dependency_decisions WHERE project_id = $2), \
                 'app_config', $3, $4, $5, 'adopted', 'engineer', \
                 (SELECT created_at - interval '1 second' FROM baseline_calculations WHERE calculation_id = $6))",
    )
    .bind(Uuid::new_v4())
    .bind(project_id)
    .bind(fixture.config_id)
    .bind(fixture.config_version_id)
    .bind(config_v2.config_version_id)
    .bind(first.calculation.calculation_id)
    .execute(store.pool())
    .await
    .expect("record the adoption with an earlier wall-clock timestamp");

    let report = dependency_report(store, project_id).await;
    let stale = report.draft.as_ref().expect("the first draft is the newest draft result");
    assert_eq!(stale.calculation_id, first.calculation.calculation_id);
    assert!(stale.stale, "the sequence ordering marks the result stale even though the timestamp is older");
    assert!(stale.stale_reasons.contains(&DraftStaleReason::NewerSharedData));

    // A result recalculated after the adoption consumed the sequence and records it.
    let second = recalculate(store, project_id, "sequence-second").await.expect("the recalculation after adoption");
    let recorded: i64 = sqlx::query_scalar("SELECT adoption_sequence FROM baseline_calculations WHERE calculation_id = $1")
        .bind(second.calculation.calculation_id)
        .fetch_one(store.pool())
        .await
        .expect("the recorded adoption sequence");
    assert_eq!(recorded, 1, "the snapshot records the highest adopted sequence it consumed");
    let report = dependency_report(store, project_id).await;
    let current = report.draft.as_ref().expect("the recalculation is the newest draft result");
    assert_eq!(current.calculation_id, second.calculation.calculation_id);
    assert!(!current.stale);

    // The SB-16 calculation route records the consumed sequence too.
    let third = store
        .calculate_baseline(
            project_id,
            &solidbom_contract::CalculateBaseline {
                request_id: "sequence-sb16".into(),
                project_version: project_version(store, project_id).await,
                engineering_version: None,
                manufacturing_version: None,
                product_revisions: Vec::new(),
            },
        )
        .await
        .expect("the SB-16 baseline calculation");
    let recorded: i64 = sqlx::query_scalar("SELECT adoption_sequence FROM baseline_calculations WHERE calculation_id = $1")
        .bind(third.calculation_id)
        .fetch_one(store.pool())
        .await
        .expect("the recorded adoption sequence");
    assert_eq!(recorded, 1);
    let report = dependency_report(store, project_id).await;
    assert!(!report.draft.as_ref().expect("a draft result exists").stale);
}
