//! Store-level tests for the internal-export projection and rendered bundles (package SB-21,
//! internal-exports contract version 1 additive to API version 2).
//!
//! The tests exercise the store method the export route calls: one explicitly selected saved result
//! (an accepted baseline or the project's current draft calculation snapshot) is resolved into the
//! rendered workbook or summary PDF, and the returned bundle is checked against its metadata. The
//! fixture is the activation-ready project: every product is quoted with a positive forecast, the
//! shared part versions are reviewed, sourced and routed, the costing settings and app configuration
//! are published and a feasibility assessment is recorded.
//!
//! `#[tokio::test]` is not recognised as a test function by Clippy's test allowance, so this file
//! allows the panicking helpers it needs. The library and binary still deny them.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

mod support;

use solidbom_contract::{
    ActivateRevisions, ActivationReceipt, AnalysisKey, AnalysisManifest, AnnualForecast, BaselineSnapshot, ClassificationConfirmation,
    CommercialScenarioContent, CompleteMeasurement, ConfirmParts, CostOwner, CostingVolumePolicy, CreateCommercialScenario,
    CreateFeasibilityRuleTableVersion, CreateProject, CycleBasis, CycleOrigin, DefinitionRef, DeliveredPartInput, DeliveredProductInput,
    DeliveryPackage, ExportBaselineRef, ExportBaselineSelection, ExportBundle, ExportFormat, ExportLabel, ExportRequest, ExportStaleReason,
    FeasibilityComponentEvidence, FeasibilityConnector, FeasibilityRule, FeasibilityRuleKind, FeasibilityTubeAttributes, FeasibilityTubeEnd,
    FeasibilityTubeMeasurements, GeometryBinding, GroupSolids, ManualRouting, MaterialConsumption, Measurement, MeasurementSource, NumericRangeMm,
    OccurrenceRef, OperationCycle, PartConfirmation, ProductKind, ProductRevisionSelection, ProjectDetails, RecordDelivery,
    RecordFeasibilityAssessment, RegisterAnalysis, ResultState, ReviewState, RoutingLevel, RoutingOperation, RoutingWrite, RuleApplicability,
    RuleLimit, SCHEMA_VERSION, SetSourcing, SetupTime, SourcedPart, Sourcing, ToolingRecoveryInputs, TubeEnd, TubeKind, Unit, UpdateProduct,
    UpdateProject, UpdateProjectCosting, YearRange,
};
use solidbom_server::{Store, StoreError};
use support::TestDb;
use uuid::Uuid;

const SOURCE: &[u8] = b"ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('export fixture'),'2;1');\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n";
const IDENTITY: [[f64; 4]; 4] = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];

/// The fixture connector the feasibility rules and evidence name.
const CONNECTOR_ID: u128 = 0xc0;

/// The tables an export must never append to.
const READ_ONLY_TABLES: &[&str] = &[
    "project_records",
    "project_record_versions",
    "engineering_records",
    "engineering_record_versions",
    "manufacturing_records",
    "manufacturing_record_versions",
    "feasibility_records",
    "feasibility_record_versions",
    "baseline_calculations",
    "accepted_baselines",
    "commercial_scenarios",
    "dependency_decisions",
    "requests",
];

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

/// One diagnosed project: its single product and shared part version.
struct Fixture {
    project_id: Uuid,
    product_id: Uuid,
    revision_id: Uuid,
    part_version_id: Uuid,
    component_id: Uuid,
    rule_table_version_id: Uuid,
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

async fn feasibility_version(store: &Store, project_id: Uuid) -> u64 {
    store.feasibility_record_view(project_id).await.expect("the feasibility record view").version
}

async fn accepted_head(store: &Store, project_id: Uuid) -> Option<Uuid> {
    store.accepted_baselines(project_id).await.expect("the accepted history").baselines.first().map(|baseline| baseline.accepted_baseline_id)
}

async fn count(store: &Store, table: &str) -> i64 {
    support::row_count(store, table).await
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

/// Build one quotation-included product with one manufactured shared part version. Every write the
/// activation readiness expects is made, so the same fixture also supports coordinated activation.
async fn fixture(store: &Store, seed: u128, feasibility: Feasibility) -> Fixture {
    let project_id = Uuid::from_u128(0xe210_0000 + seed);
    let component_id = Uuid::from_u128(0xcc00_0000 + seed);
    let routing_id = Uuid::from_u128(0x7700_0000 + seed);
    let source = store.put_source(&support::digest(SOURCE), "export.step", SOURCE).await.expect("store the source");
    store
        .create_project(&CreateProject { request_id: format!("{seed}-create"), project_id, details: details("Export inquiry") })
        .await
        .expect("create the project");
    store
        .record_delivery(
            project_id,
            &RecordDelivery {
                request_id: format!("{seed}-delivery"),
                expected_version: 1,
                package: DeliveryPackage { package_id: Uuid::from_u128(0xaa00_0000 + seed), name: "Package 1".into(), source },
                products: vec![DeliveredProductInput {
                    product_id: None,
                    name: "Housing".into(),
                    kind: ProductKind::Part,
                    customer_part_number: Some("PL-100".into()),
                    parts: vec![delivered_part("Housing-body", "PL-100", "B")],
                }],
            },
        )
        .await
        .expect("record the delivery");
    let view = store.project_record_view(project_id).await.expect("the project record");
    let product = view.document.products.iter().find(|product| product.name == "Housing").expect("the delivered product");
    let product_id = product.product_id;
    let revision_id = product.revisions.first().expect("the delivered revision").product_revision_id;
    let part_version_id = view.document.part_version_by_identity("PL-100", "B").expect("the delivered part version").part_version_id;
    store
        .update_product(
            project_id,
            &UpdateProduct {
                request_id: format!("{seed}-product"),
                expected_version: project_version(store, project_id).await,
                product_id,
                name: "Housing".into(),
                kind: ProductKind::Part,
                customer_part_number: Some("PL-100".into()),
                schedule_override: Some(YearRange { sop_year: 2027, eop_year: 2027 }),
                forecasts: vec![AnnualForecast { year: 2027, quantity: 100 }],
            },
        )
        .await
        .expect("set the forecast");
    store
        .register_analysis(
            project_id,
            &RegisterAnalysis { request_id: format!("{seed}-analysis"), product_revision_id: revision_id, manifest: manifest("PL-100", "B", seed) },
        )
        .await
        .expect("register the analysis");
    store
        .group_solids(
            project_id,
            &GroupSolids {
                request_id: format!("{seed}-group"),
                expected_version: engineering_version(store, project_id).await,
                component_id,
                product_revision_id: revision_id,
                analysis_key: analysis_key(),
                geometry: vec![solidbom_contract::ReviewedGeometry { source_path: Vec::new(), source_body: 0 }],
                part_version_id: Some(part_version_id),
            },
        )
        .await
        .expect("group the reviewed component");
    store
        .complete_measurement(
            project_id,
            &CompleteMeasurement {
                request_id: format!("{seed}-measurement"),
                expected_version: engineering_version(store, project_id).await,
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
    store
        .confirm_parts(
            project_id,
            &ConfirmParts {
                request_id: format!("{seed}-confirm"),
                expected_version: engineering_version(store, project_id).await,
                confirmations: vec![PartConfirmation {
                    part_version_id,
                    classification: Some(ClassificationConfirmation { name: "bracket".into() }),
                    confirm_measurement: true,
                    catalog: None,
                }],
            },
            "engineer",
        )
        .await
        .expect("confirm the reviewed part");
    store
        .set_sourcing(
            project_id,
            &SetSourcing {
                request_id: format!("{seed}-sourcing"),
                expected_version: manufacturing_version(store, project_id).await,
                parts: vec![manufactured(part_version_id, "100", Some("0.02"))],
                affected_part_versions: vec![part_version_id],
            },
            "engineer",
        )
        .await
        .expect("set the sourcing");
    store
        .write_routing(
            project_id,
            &RoutingWrite::Manual(ManualRouting {
                request_id: format!("{seed}-routing"),
                expected_version: manufacturing_version(store, project_id).await,
                routing_id,
                level: RoutingLevel::Part,
                owner: CostOwner::PartVersion { part_version_id },
                operations: vec![operation(0x7100_0000 + seed, 1, "30")],
                affected_part_versions: vec![part_version_id],
                affected_products: Vec::new(),
            }),
        )
        .await
        .expect("write the routing");
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
    store
        .record_feasibility_assessment(
            project_id,
            &RecordFeasibilityAssessment {
                request_id: format!("{seed}-assessment"),
                expected_version: 0,
                rule_table_version_id: published.rule_table.rule_table_version_id,
                components: vec![evidence(revision_id, component_id, feasibility)],
            },
            "engineer",
        )
        .await
        .expect("record the feasibility assessment");
    Fixture { project_id, product_id, revision_id, part_version_id, component_id, rule_table_version_id: published.rule_table.rule_table_version_id }
}

/// A project whose delivery, forecast and analysis exist but whose costing inputs are all missing:
/// the calculated result is incomplete and its stored issue list names the missing inputs.
async fn incomplete_fixture(store: &Store, seed: u128) -> Fixture {
    let project_id = Uuid::from_u128(0xe310_0000 + seed);
    let component_id = Uuid::from_u128(0xcd00_0000 + seed);
    let source = store.put_source(&support::digest(SOURCE), "export.step", SOURCE).await.expect("store the source");
    store
        .create_project(&CreateProject { request_id: format!("{seed}-create"), project_id, details: details("Incomplete inquiry") })
        .await
        .expect("create the project");
    store
        .record_delivery(
            project_id,
            &RecordDelivery {
                request_id: format!("{seed}-delivery"),
                expected_version: 1,
                package: DeliveryPackage { package_id: Uuid::from_u128(0xab00_0000 + seed), name: "Package 1".into(), source },
                products: vec![DeliveredProductInput {
                    product_id: None,
                    name: "Housing".into(),
                    kind: ProductKind::Part,
                    customer_part_number: Some("PL-200".into()),
                    parts: vec![delivered_part("Housing-body", "PL-200", "A")],
                }],
            },
        )
        .await
        .expect("record the delivery");
    let view = store.project_record_view(project_id).await.expect("the project record");
    let product = view.document.products.iter().find(|product| product.name == "Housing").expect("the delivered product");
    let product_id = product.product_id;
    let revision_id = product.revisions.first().expect("the delivered revision").product_revision_id;
    let part_version_id = view.document.part_version_by_identity("PL-200", "A").expect("the delivered part version").part_version_id;
    store
        .update_product(
            project_id,
            &UpdateProduct {
                request_id: format!("{seed}-product"),
                expected_version: project_version(store, project_id).await,
                product_id,
                name: "Housing".into(),
                kind: ProductKind::Part,
                customer_part_number: Some("PL-200".into()),
                schedule_override: Some(YearRange { sop_year: 2027, eop_year: 2027 }),
                forecasts: vec![AnnualForecast { year: 2027, quantity: 100 }],
            },
        )
        .await
        .expect("set the forecast");
    store
        .register_analysis(
            project_id,
            &RegisterAnalysis { request_id: format!("{seed}-analysis"), product_revision_id: revision_id, manifest: manifest("PL-200", "A", seed) },
        )
        .await
        .expect("register the analysis");
    store
        .group_solids(
            project_id,
            &GroupSolids {
                request_id: format!("{seed}-group"),
                expected_version: engineering_version(store, project_id).await,
                component_id,
                product_revision_id: revision_id,
                analysis_key: analysis_key(),
                geometry: vec![solidbom_contract::ReviewedGeometry { source_path: Vec::new(), source_body: 0 }],
                part_version_id: Some(part_version_id),
            },
        )
        .await
        .expect("group the reviewed component");
    Fixture { project_id, product_id, revision_id, part_version_id, component_id, rule_table_version_id: Uuid::nil() }
}

fn request(project_id: Uuid, baseline: ExportBaselineRef, format: ExportFormat, scenarios: Vec<Uuid>) -> ExportRequest {
    ExportRequest { project_id, baseline, scenarios, format }
}

fn draft(calculation_id: Uuid) -> ExportBaselineRef {
    ExportBaselineRef::DraftCalculation { calculation_id }
}

fn accepted(baseline_id: Uuid, sequence: u64) -> ExportBaselineRef {
    ExportBaselineRef::AcceptedBaseline { accepted_baseline_id: baseline_id, sequence }
}

async fn export(store: &Store, project_id: Uuid, baseline: ExportBaselineRef, format: ExportFormat, scenarios: Vec<Uuid>) -> ExportBundle {
    store.export_bundle(project_id, &request(project_id, baseline, format, scenarios)).await.expect("the export bundle")
}

async fn calculate(store: &Store, fixture: &Fixture, request_id: &str) -> BaselineSnapshot {
    store
        .calculate_baseline(
            fixture.project_id,
            &solidbom_contract::CalculateBaseline {
                request_id: request_id.into(),
                project_version: project_version(store, fixture.project_id).await,
                engineering_version: None,
                manufacturing_version: None,
                product_revisions: vec![ProductRevisionSelection { product_id: fixture.product_id, product_revision_id: fixture.revision_id }],
            },
        )
        .await
        .expect("the calculated baseline")
}

async fn activate(store: &Store, fixture: &Fixture, request_id: &str) -> ActivationReceipt {
    store
        .activate_revisions(
            fixture.project_id,
            &ActivateRevisions {
                request_id: request_id.into(),
                expected_project_version: project_version(store, fixture.project_id).await,
                expected_accepted_baseline_id: accepted_head(store, fixture.project_id).await,
                product_revisions: vec![ProductRevisionSelection { product_id: fixture.product_id, product_revision_id: fixture.revision_id }],
            },
        )
        .await
        .expect("the activation")
}

#[tokio::test]
async fn a_draft_export_carries_the_current_snapshot_and_renders_both_formats() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = fixture(store, 1, Feasibility::Passing).await;
    let snapshot = calculate(store, &fixture, "1-draft").await;
    assert_eq!(snapshot.outputs.state, ResultState::Complete, "the fixture is a complete result");

    let workbook = export(store, fixture.project_id, draft(snapshot.calculation_id), ExportFormat::Xlsx, Vec::new()).await;
    assert!(workbook.validate().is_ok(), "the bundle agrees with its metadata");
    assert_eq!(workbook.metadata.project_id, fixture.project_id);
    assert_eq!(workbook.metadata.project_name, "Export inquiry");
    assert_eq!(workbook.metadata.customer, "Customer");
    assert_eq!(workbook.metadata.currency, "CNY");
    assert!(workbook.metadata.exported_at.ends_with('Z'), "the export timestamp is RFC 3339 UTC: {}", workbook.metadata.exported_at);
    assert_eq!(workbook.metadata.format, ExportFormat::Xlsx);
    assert_eq!(
        workbook.metadata.baseline,
        ExportBaselineSelection::DraftCalculation { calculation_id: snapshot.calculation_id },
        "the documents name the exact draft snapshot"
    );
    assert_eq!(workbook.metadata.versions.project_version, snapshot.project_version);
    assert_eq!(workbook.metadata.versions.engineering_version, snapshot.engineering_version);
    assert_eq!(workbook.metadata.versions.manufacturing_version, snapshot.manufacturing_version);
    assert_eq!(workbook.metadata.versions.feasibility_version, 1);
    assert!(!workbook.metadata.versions.feasibility_pinned, "a draft reads its feasibility record at export time");
    assert_eq!(workbook.metadata.calculation.calculation_id, snapshot.calculation_id);
    assert_eq!(workbook.metadata.calculation.input_hash, snapshot.input_hash);
    assert_eq!(workbook.metadata.calculation.calculator_version, snapshot.calculator_version);
    assert_eq!(workbook.metadata.calculation.state, ResultState::Complete);
    assert!(!workbook.metadata.stale);
    assert!(workbook.metadata.stale_reasons.is_empty());
    assert_eq!(
        workbook.metadata.labels,
        vec![ExportLabel::DraftResult, ExportLabel::UnpinnedFeasibilityRecord],
        "a complete fresh draft carries no defect label"
    );
    assert_eq!(workbook.file_name, "export-inquiry-draft.xlsx");
    assert!(workbook.file_name.is_ascii());
    assert_eq!(workbook.media_type, ExportFormat::Xlsx.media_type());
    assert_eq!(workbook.byte_len, u64::try_from(workbook.bytes.len()).expect("a bounded length"));
    assert_eq!(workbook.sha256, support::digest(&workbook.bytes));
    assert!(workbook.bytes.starts_with(b"PK\x03\x04"), "the workbook is an Office Open XML ZIP package");

    let pdf = export(store, fixture.project_id, draft(snapshot.calculation_id), ExportFormat::Pdf, Vec::new()).await;
    assert!(pdf.validate().is_ok());
    assert_eq!(pdf.file_name, "export-inquiry-draft.pdf");
    assert_eq!(pdf.media_type, "application/pdf");
    assert_eq!(pdf.byte_len, u64::try_from(pdf.bytes.len()).expect("a bounded length"));
    assert_eq!(pdf.sha256, support::digest(&pdf.bytes));
    assert!(pdf.bytes.starts_with(b"%PDF-"), "the summary is a PDF document");
    assert_eq!(pdf.metadata.labels, workbook.metadata.labels);
    assert_eq!(pdf.metadata.versions, workbook.metadata.versions);
    db.cleanup().await;
}

#[tokio::test]
async fn a_draft_export_marks_newer_records_and_shared_data_as_stale() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = fixture(store, 2, Feasibility::Passing).await;
    let snapshot = calculate(store, &fixture, "2-draft").await;
    let before = export(store, fixture.project_id, draft(snapshot.calculation_id), ExportFormat::Pdf, Vec::new()).await;
    assert!(!before.metadata.stale);

    // The project record moves past the snapshot without publishing a new draft result: a
    // non-schedule project detail edit.
    let mut moved_details = details("Export inquiry");
    moved_details.customer = "Another customer".into();
    store
        .update_project(
            fixture.project_id,
            &UpdateProject {
                request_id: "2-project-move".into(),
                expected_version: project_version(store, fixture.project_id).await,
                details: moved_details,
            },
        )
        .await
        .expect("move the project record");
    // The manufacturing record moves past the snapshot: a changed material consumption.
    store
        .set_sourcing(
            fixture.project_id,
            &SetSourcing {
                request_id: "2-sourcing-move".into(),
                expected_version: manufacturing_version(store, fixture.project_id).await,
                parts: vec![manufactured(fixture.part_version_id, "110", Some("0.02"))],
                affected_part_versions: vec![fixture.part_version_id],
            },
            "engineer",
        )
        .await
        .expect("move the manufacturing record");
    // A costing-relevant shared record is published after the snapshot read its inputs.
    store
        .create_material_version(&support::material_version("2-material-move", 1, support::material_id(), "2.7", Some("21")), "administrator")
        .await
        .expect("publish a newer material version");

    let after = export(store, fixture.project_id, draft(snapshot.calculation_id), ExportFormat::Pdf, Vec::new()).await;
    assert!(after.metadata.stale, "the draft result is stale");
    assert_eq!(
        after.metadata.stale_reasons,
        vec![ExportStaleReason::NewerManufacturingVersion, ExportStaleReason::NewerProjectVersion, ExportStaleReason::NewerSharedData,],
        "the SB-20 stale reasons are copied and sorted by their stable names"
    );
    assert_eq!(
        after.metadata.labels,
        vec![ExportLabel::DraftResult, ExportLabel::OutdatedCosts, ExportLabel::UnpinnedFeasibilityRecord],
        "a stale draft result is labelled as outdated costs"
    );
    assert_eq!(after.metadata.calculation, before.metadata.calculation, "the selected snapshot is unchanged");
    assert_eq!(after.metadata.versions, before.metadata.versions, "the pinned record versions are unchanged");
    assert!(after.validate().is_ok());
    db.cleanup().await;
}

#[tokio::test]
async fn an_accepted_baseline_export_pins_its_versions_and_labels_a_newer_acceptance() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = fixture(store, 3, Feasibility::Passing).await;
    let first = activate(store, &fixture, "3-activation-1").await;
    assert_eq!(first.pinned.feasibility_version, 1);
    // The current feasibility record moves past the pinned version after the acceptance.
    store
        .record_feasibility_assessment(
            fixture.project_id,
            &RecordFeasibilityAssessment {
                request_id: "3-assessment-2".into(),
                expected_version: feasibility_version(store, fixture.project_id).await,
                rule_table_version_id: fixture.rule_table_version_id,
                components: vec![evidence(fixture.revision_id, fixture.component_id, Feasibility::Passing)],
            },
            "engineer",
        )
        .await
        .expect("record a newer feasibility assessment");
    assert_eq!(feasibility_version(store, fixture.project_id).await, 2);

    let baseline = export(store, fixture.project_id, accepted(first.accepted_baseline_id, 1), ExportFormat::Xlsx, Vec::new()).await;
    assert!(baseline.validate().is_ok());
    assert_eq!(
        baseline.metadata.baseline,
        ExportBaselineSelection::AcceptedBaseline { accepted_baseline_id: first.accepted_baseline_id, sequence: 1 }
    );
    assert_eq!(baseline.metadata.versions.project_version, first.pinned.project_version);
    assert_eq!(baseline.metadata.versions.engineering_version, first.pinned.engineering_version);
    assert_eq!(baseline.metadata.versions.manufacturing_version, first.pinned.manufacturing_version);
    assert_eq!(baseline.metadata.versions.feasibility_version, first.pinned.feasibility_version, "the accepted version is pinned");
    assert!(baseline.metadata.versions.feasibility_pinned);
    assert_eq!(baseline.metadata.calculation.calculation_id, first.costing_calculation_id);
    assert_eq!(baseline.metadata.calculation.input_hash, first.costing_input_hash);
    assert!(!baseline.metadata.stale, "accepted history is never a stale draft");
    assert!(baseline.metadata.stale_reasons.is_empty());
    assert!(baseline.metadata.labels.is_empty(), "a fresh accepted baseline carries no defect label");
    assert_eq!(baseline.file_name, "export-inquiry-accepted-baseline-1.xlsx");
    assert!(baseline.bytes.starts_with(b"PK\x03\x04"));

    // A later acceptance supersedes the first one; the stored baseline is labelled, not substituted.
    let second = activate(store, &fixture, "3-activation-2").await;
    assert_eq!(second.pinned.feasibility_version, 2);
    let superseded = export(store, fixture.project_id, accepted(first.accepted_baseline_id, 1), ExportFormat::Pdf, Vec::new()).await;
    assert_eq!(superseded.metadata.labels, vec![ExportLabel::SupersededAcceptedBaseline]);
    assert_eq!(superseded.metadata.calculation.calculation_id, first.costing_calculation_id, "nothing is re-resolved to the newer acceptance");
    assert!(superseded.bytes.starts_with(b"%PDF-"));
    let head = export(store, fixture.project_id, accepted(second.accepted_baseline_id, 2), ExportFormat::Pdf, Vec::new()).await;
    assert!(head.metadata.labels.is_empty());

    // A sequence that does not match the stored row is refused as invalid.
    let mismatch = store
        .export_bundle(fixture.project_id, &request(fixture.project_id, accepted(first.accepted_baseline_id, 2), ExportFormat::Pdf, Vec::new()))
        .await
        .expect_err("a mismatched sequence is refused");
    assert!(matches!(mismatch, StoreError::Invalid(_)), "a sequence mismatch is invalid, got {mismatch:?}");
    db.cleanup().await;
}

#[tokio::test]
async fn a_superseded_draft_and_an_unknown_baseline_are_refused() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = fixture(store, 4, Feasibility::Passing).await;
    let older = calculate(store, &fixture, "4-first").await;
    let current = calculate(store, &fixture, "4-second").await;
    assert_ne!(older.calculation_id, current.calculation_id);

    // The current draft result exports.
    assert!(export(store, fixture.project_id, draft(current.calculation_id), ExportFormat::Xlsx, Vec::new()).await.validate().is_ok());
    // An earlier draft snapshot is refused like an expired client read.
    let refused = store
        .export_bundle(fixture.project_id, &request(fixture.project_id, draft(older.calculation_id), ExportFormat::Xlsx, Vec::new()))
        .await
        .expect_err("a superseded draft snapshot is refused");
    match refused {
        StoreError::Conflict(message) => assert!(message.contains("current draft result"), "{message}"),
        other => panic!("expected a conflict, got {other:?}"),
    }
    // An unknown accepted baseline, and an unknown project, are not found.
    let unknown = store
        .export_bundle(fixture.project_id, &request(fixture.project_id, accepted(Uuid::from_u128(0xdead_beef), 1), ExportFormat::Xlsx, Vec::new()))
        .await
        .expect_err("an unknown accepted baseline is not found");
    assert!(matches!(unknown, StoreError::NotFound), "got {unknown:?}");
    let unknown_project = store
        .export_bundle(
            Uuid::from_u128(0xfeed_face),
            &request(Uuid::from_u128(0xfeed_face), draft(current.calculation_id), ExportFormat::Xlsx, Vec::new()),
        )
        .await
        .expect_err("an unknown project is not found");
    assert!(matches!(unknown_project, StoreError::NotFound), "got {unknown_project:?}");
    // A request that names another project than the route is invalid.
    let wrong_project = store
        .export_bundle(fixture.project_id, &request(Uuid::from_u128(0x9999), draft(current.calculation_id), ExportFormat::Xlsx, Vec::new()))
        .await
        .expect_err("a mismatched project id is refused");
    assert!(matches!(wrong_project, StoreError::Invalid(_)), "got {wrong_project:?}");
    db.cleanup().await;
}

#[tokio::test]
async fn saved_scenarios_are_compared_and_a_stale_one_is_labelled() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = fixture(store, 5, Feasibility::Passing).await;
    let first = calculate(store, &fixture, "5-draft-a").await;
    let scenario_id = Uuid::from_u128(0x05ce_0005);
    store
        .create_commercial_scenario(
            fixture.project_id,
            &CreateCommercialScenario {
                request_id: "5-scenario".into(),
                scenario_id,
                content: CommercialScenarioContent {
                    name: "Base case".into(),
                    baseline_calculation_id: first.calculation_id,
                    manual_prices: Vec::new(),
                    tooling: ToolingRecoveryInputs::default(),
                    productivity_applies_to_tooling: true,
                },
            },
        )
        .await
        .expect("save the scenario");
    // The scenario is evaluated against its own baseline and is directly comparable.
    let comparable = export(store, fixture.project_id, draft(first.calculation_id), ExportFormat::Pdf, vec![scenario_id]).await;
    assert!(comparable.validate().is_ok());
    assert!(!comparable.metadata.labels.contains(&ExportLabel::StaleScenarios), "the scenario pinned the exported result");
    // A newer draft result needs a fresh draft as the selected result.
    let second = calculate(store, &fixture, "5-draft-b").await;
    let stale = export(store, fixture.project_id, draft(second.calculation_id), ExportFormat::Pdf, vec![scenario_id]).await;
    assert!(stale.metadata.labels.contains(&ExportLabel::StaleScenarios), "the scenario pinned another result: {:?}", stale.metadata.labels);
    // An unknown scenario id is not found: a comparison never silently drops a requested scenario.
    let unknown = store
        .export_bundle(
            fixture.project_id,
            &request(fixture.project_id, draft(second.calculation_id), ExportFormat::Pdf, vec![Uuid::from_u128(0xdead_05ce)]),
        )
        .await
        .expect_err("an unknown scenario is not found");
    assert!(matches!(unknown, StoreError::NotFound), "got {unknown:?}");
    // Out-of-order scenario ids are an invalid request.
    let unordered = store
        .export_bundle(
            fixture.project_id,
            &request(fixture.project_id, draft(second.calculation_id), ExportFormat::Pdf, vec![Uuid::from_u128(0x2), Uuid::from_u128(0x1)]),
        )
        .await
        .expect_err("scenario ids are ascending");
    assert!(matches!(unordered, StoreError::Invalid(_)), "got {unordered:?}");
    db.cleanup().await;
}

#[tokio::test]
async fn an_incomplete_result_is_exported_with_its_missing_inputs_labelled() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = incomplete_fixture(store, 6).await;
    let snapshot = calculate(store, &fixture, "6-draft").await;
    assert_eq!(snapshot.outputs.state, ResultState::Incomplete, "an unfinished result stays exportable");
    assert!(snapshot.outputs.issues.iter().any(|issue| issue.code.starts_with("missing_")), "{:?}", snapshot.outputs.issues);

    let bundle = export(store, fixture.project_id, draft(snapshot.calculation_id), ExportFormat::Pdf, Vec::new()).await;
    assert!(bundle.validate().is_ok());
    assert_eq!(bundle.metadata.calculation.state, ResultState::Incomplete);
    assert_eq!(
        bundle.metadata.labels,
        vec![ExportLabel::DraftResult, ExportLabel::IncompleteResult, ExportLabel::MissingInputs, ExportLabel::UnpinnedFeasibilityRecord,],
        "an unfinished draft is labelled and its missing inputs are named"
    );
    assert!(bundle.bytes.starts_with(b"%PDF-"));
    db.cleanup().await;
}

#[tokio::test]
async fn unresolved_feasibility_findings_are_labelled() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = fixture(store, 7, Feasibility::Failing).await;
    let snapshot = calculate(store, &fixture, "7-draft").await;
    assert_eq!(snapshot.outputs.state, ResultState::Complete, "the costing result itself is complete");
    let bundle = export(store, fixture.project_id, draft(snapshot.calculation_id), ExportFormat::Xlsx, Vec::new()).await;
    assert_eq!(
        bundle.metadata.labels,
        vec![ExportLabel::DraftResult, ExportLabel::UnresolvedFindings, ExportLabel::UnpinnedFeasibilityRecord],
        "a non-passing feasibility finding is labelled, not hidden"
    );
    assert!(bundle.validate().is_ok());
    db.cleanup().await;
}

#[tokio::test]
async fn an_export_is_a_pure_read() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = fixture(store, 8, Feasibility::Passing).await;
    let snapshot = calculate(store, &fixture, "8-draft").await;
    let mut before = Vec::with_capacity(READ_ONLY_TABLES.len());
    for table in READ_ONLY_TABLES {
        before.push(count(store, table).await);
    }
    let bundle = export(store, fixture.project_id, draft(snapshot.calculation_id), ExportFormat::Xlsx, Vec::new()).await;
    assert!(bundle.validate().is_ok());
    let _ = export(store, fixture.project_id, draft(snapshot.calculation_id), ExportFormat::Pdf, Vec::new()).await;
    let mut after = Vec::with_capacity(READ_ONLY_TABLES.len());
    for table in READ_ONLY_TABLES {
        after.push(count(store, table).await);
    }
    assert_eq!(before, after, "an export appends no record, snapshot, baseline, decision or request receipt");
    db.cleanup().await;
}
