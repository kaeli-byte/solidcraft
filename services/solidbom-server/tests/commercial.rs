//! Store-level tests for the project commercial settings, the append-only named scenarios and the
//! read-only comparison (contract version 1 of the SB-18 commercial-scenarios contract, additive to
//! API version 2).
//!
//! The recurring unit cost, forecasts, setup charge and volume policy of every comparison are those
//! of a real immutable SB-16 baseline snapshot: every fixture owns a delivered product, a registered
//! exact analysis, the reviewed material and routing records and the published reference data, and
//! the baseline is calculated through `Store::calculate_baseline`. No hand-inserted snapshot or
//! scenario row exists.
//!
//! `#[tokio::test]` is not recognised as a test function by Clippy's test allowance, so this file
//! allows the panicking helpers it needs. The library and binary still deny them.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

mod support;

use solidbom_contract::{
    AnalysisKey, AnalysisManifest, AnnualForecast, BaselineSnapshot, CalculateBaseline, CatalogConfirmation, ConfirmParts, CostOwner,
    CostingVolumePolicy, CreateCommercialScenario, CreateProject, CycleBasis, CycleOrigin, DefinitionRef, DeliveredPartInput, DeliveredProductInput,
    DeliveryPackage, EditAttachedCosts, EngineeringWriteOutcome, GeometryBinding, GroupSolids, ManualRouting, MaterialConsumption, Measurement,
    MeasurementSource, OperationCycle, PartConfirmation, ProductKind, ProjectCommercial, ProjectCosting, ProjectDetails, RegisterAnalysis,
    ResultState, ReviewState, ReviewedGeometry, RoutingLevel, RoutingOperation, RoutingWrite, SCHEMA_VERSION, SetSourcing, SetupTime, SourcedPart,
    Sourcing, Unit, UpdateCommercialScenario, UpdateProduct, UpdateProjectCommercial, UpdateProjectCosting, YearRange,
};
use solidbom_server::{Store, StoreError};
use support::TestDb;
use uuid::Uuid;

const SOURCE: &[u8] =
    b"ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('commercial scenarios fixture'),'2;1');\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n";
const IDENTITY: [[f64; 4]; 4] = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];

fn analysis_key() -> AnalysisKey {
    AnalysisKey { source_sha256: support::digest(SOURCE), engine_build: "0.40.0-rc.2".into(), extractor_version: "1".into() }
}

fn measurement(value: &str) -> Measurement {
    Measurement {
        value: Some(support::d(value)),
        unit: Unit::MillimetreCubed,
        source: MeasurementSource::Extracted,
        review: ReviewState::Confirmed,
        reason: None,
        analysis_key: analysis_key(),
    }
}

/// The exact analysis of the delivered product revision: one made part and one purchased part, both
/// resolving their shared part version through the customer identifiers.
fn manifest() -> AnalysisManifest {
    let body = Uuid::from_u128(0xde01);
    let bolt = Uuid::from_u128(0xde02);
    let body_occurrence = Uuid::from_u128(0x0c01);
    let bolt_occurrence = Uuid::from_u128(0x0c02);
    AnalysisManifest {
        schema_version: SCHEMA_VERSION,
        key: analysis_key(),
        definitions: vec![
            DefinitionRef {
                definition_id: body,
                source_key: analysis_key(),
                customer_part_number: Some("M-1".into()),
                customer_revision: Some("A".into()),
                volume_mm3: measurement("2400"),
            },
            DefinitionRef {
                definition_id: bolt,
                source_key: analysis_key(),
                customer_part_number: Some("P-1".into()),
                customer_revision: Some("A".into()),
                volume_mm3: measurement("2400"),
            },
        ],
        occurrences: vec![
            solidbom_contract::OccurrenceRef {
                occurrence_id: body_occurrence,
                definition_id: body,
                parent_occurrence_id: None,
                source_path: Vec::new(),
                transform: IDENTITY,
            },
            solidbom_contract::OccurrenceRef {
                occurrence_id: bolt_occurrence,
                definition_id: bolt,
                parent_occurrence_id: None,
                source_path: vec![1],
                transform: IDENTITY,
            },
        ],
        geometry_bindings: vec![
            GeometryBinding { source_path: Vec::new(), source_body: 0, definition_id: body, occurrence_id: body_occurrence },
            GeometryBinding { source_path: vec![1], source_body: 0, definition_id: bolt, occurrence_id: bolt_occurrence },
        ],
        issues: Vec::new(),
    }
}

fn production_rate_id() -> Uuid {
    support::rate_id()
}

fn setup_rate_id() -> Uuid {
    Uuid::parse_str("00000000-0000-4000-8000-000000000102").expect("the seeded setup rate fixture")
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

fn operation(operation_id: u128, sequence: u32, name: &str, cycle: OperationCycle, setup: Option<SetupTime>) -> RoutingOperation {
    RoutingOperation {
        operation_id: Uuid::from_u128(operation_id),
        sequence,
        name: name.into(),
        description: None,
        cost_center_id: None,
        cycle: Some(cycle),
        setup,
        tooling_reference: None,
    }
}

/// The full fixture: a quoted product with one made part (material, production and a routing setup)
/// and one purchased part (confirmed catalog price plus a project tier), a published app cost
/// configuration and the project costing settings. It returns the immutable three-year baseline.
struct Fixture {
    project_id: Uuid,
    product_id: Uuid,
    baseline: BaselineSnapshot,
}

async fn setup(store: &Store) -> Fixture {
    let project_id = Uuid::from_u128(0xb301);
    let source = store.put_source(&support::digest(SOURCE), "commercial.step", SOURCE).await.expect("store the source");
    store
        .create_project(&CreateProject {
            request_id: "create-1".into(),
            project_id,
            details: ProjectDetails {
                name: "Commercial inquiry".into(),
                customer: "Customer".into(),
                internal_team: None,
                platform: None,
                application: None,
                currency: "CNY".into(),
                default_schedule: YearRange { sop_year: 2027, eop_year: 2029 },
            },
        })
        .await
        .expect("create the project");
    let delivered = |key: &str, number: &str, revision: &str| DeliveredPartInput {
        delivered_key: key.into(),
        customer_part_number: Some(number.into()),
        customer_revision: Some(revision.into()),
        volume_mm3: Some(support::d("2400")),
    };
    store
        .record_delivery(
            project_id,
            &solidbom_contract::RecordDelivery {
                request_id: "delivery-1".into(),
                expected_version: 1,
                package: DeliveryPackage { package_id: Uuid::from_u128(0xb302), name: "Package 1".into(), source },
                products: vec![DeliveredProductInput {
                    product_id: None,
                    name: "assembly".into(),
                    kind: ProductKind::Assembly,
                    customer_part_number: None,
                    parts: vec![delivered("body", "M-1", "A"), delivered("bolt", "P-1", "A")],
                }],
            },
        )
        .await
        .expect("record the delivery");
    let view = store.project_record_view(project_id).await.expect("the project record view");
    let product = view.document.products.first().expect("the delivered product");
    let product_id = product.product_id;
    let revision_id = product.revisions.first().expect("the delivered revision").product_revision_id;
    let body = view.document.part_version_by_identity("M-1", "A").expect("the made part version").part_version_id;
    let bolt = view.document.part_version_by_identity("P-1", "A").expect("the purchased part version").part_version_id;

    store
        .update_product(
            project_id,
            &UpdateProduct {
                request_id: "product-1".into(),
                expected_version: 2,
                product_id,
                name: "assembly".into(),
                kind: ProductKind::Assembly,
                customer_part_number: None,
                schedule_override: Some(YearRange { sop_year: 2027, eop_year: 2029 }),
                forecasts: vec![
                    AnnualForecast { year: 2027, quantity: 100 },
                    AnnualForecast { year: 2028, quantity: 200 },
                    AnnualForecast { year: 2029, quantity: 300 },
                ],
            },
        )
        .await
        .expect("set the schedule and the forecasts");

    store
        .register_analysis(project_id, &RegisterAnalysis { request_id: "analysis-1".into(), product_revision_id: revision_id, manifest: manifest() })
        .await
        .expect("register the exact analysis");
    let grouped: EngineeringWriteOutcome = store
        .group_solids(
            project_id,
            &GroupSolids {
                request_id: "group-1".into(),
                expected_version: 0,
                component_id: Uuid::from_u128(0xc301),
                product_revision_id: revision_id,
                analysis_key: analysis_key(),
                geometry: vec![ReviewedGeometry { source_path: Vec::new(), source_body: 0 }],
                part_version_id: Some(body),
            },
        )
        .await
        .expect("group the reviewed component");
    assert_eq!(grouped.receipt.version, 1);

    let component_id = Uuid::from_u128(0xca71);
    let component = store
        .create_component_version(&support::component_version("component-1", 0, component_id, Some("8")), "administrator")
        .await
        .expect("publish the catalog component");
    let component_version_id = component.component.component_version_id;
    store
        .confirm_parts(
            project_id,
            &ConfirmParts {
                request_id: "confirm-1".into(),
                expected_version: 1,
                confirmations: vec![PartConfirmation {
                    part_version_id: bolt,
                    classification: None,
                    confirm_measurement: false,
                    catalog: Some(CatalogConfirmation { component_version_id, project_override: None }),
                }],
            },
            "engineer",
        )
        .await
        .expect("confirm the catalog reference");

    store
        .set_sourcing(
            project_id,
            &SetSourcing {
                request_id: "sourcing-1".into(),
                expected_version: 0,
                parts: vec![manufactured(body), purchased(bolt)],
                affected_part_versions: vec![body, bolt],
            },
            "engineer",
        )
        .await
        .expect("set the explicit sourcing");
    store
        .write_routing(
            project_id,
            &RoutingWrite::Manual(ManualRouting {
                request_id: "routing-1".into(),
                expected_version: 1,
                routing_id: Uuid::from_u128(0x8a01),
                level: RoutingLevel::Part,
                owner: CostOwner::PartVersion { part_version_id: body },
                operations: vec![operation(
                    0x7101,
                    1,
                    "cut tube",
                    OperationCycle {
                        seconds: Some(support::d("30")),
                        basis: CycleBasis::PerPiece,
                        cavities: None,
                        production_rate_version_id: Some(production_rate_id()),
                        origin: CycleOrigin::Manual,
                        source_reference: None,
                    },
                    Some(SetupTime { hours: Some(support::d("0.5")), setup_rate_version_id: Some(setup_rate_id()), source_reference: None }),
                )],
                affected_part_versions: vec![body],
                affected_products: Vec::new(),
            }),
        )
        .await
        .expect("write the part routing");
    store
        .edit_attached_costs(
            project_id,
            &EditAttachedCosts {
                request_id: "attached-1".into(),
                expected_version: 2,
                upsert: vec![solidbom_contract::AttachedCostLine {
                    line_id: Uuid::from_u128(0x7830),
                    owner: CostOwner::PartVersion { part_version_id: body },
                    category: "packaging".into(),
                    description: Some("returnable tray".into()),
                    quantity: Some(support::d("2")),
                    unit: "pcs".into(),
                    unit_cost: Some(support::d("0.50")),
                    source_reference: None,
                }],
                remove: Vec::new(),
                affected_part_versions: vec![body],
                affected_products: Vec::new(),
            },
        )
        .await
        .expect("write the attached cost line");
    store
        .create_app_config_version(&support::app_config_version("config-1", 0, Uuid::from_u128(0xcf01), Vec::new()), "administrator")
        .await
        .expect("publish the app cost configuration");
    store
        .update_project_costing(
            project_id,
            &UpdateProjectCosting {
                request_id: "costing-1".into(),
                expected_version: 3,
                costing: ProjectCosting {
                    volume_policy: Some(CostingVolumePolicy::Average),
                    exchange_rates: Vec::new(),
                    purchase_prices: vec![solidbom_contract::PurchasePriceTable {
                        part_version_id: bolt,
                        currency: "CNY".into(),
                        tiers: vec![solidbom_contract::PurchasePriceTier { minimum_annual_quantity: 1, unit_price: support::d("10") }],
                        source_reference: Some("agreed purchase tiers 2027".into()),
                    }],
                },
            },
        )
        .await
        .expect("write the project costing settings");
    let activated = store.record_revision_activation(project_id, revision_id).await.expect("activate the delivered revision");
    assert_eq!(activated.version, 5);

    store
        .update_project_commercial(
            project_id,
            &UpdateProjectCommercial {
                request_id: "commercial-1".into(),
                expected_version: 5,
                commercial: ProjectCommercial { ebit_target_rate: support::d("0.2"), annual_productivity_rate: support::d("0.03") },
            },
        )
        .await
        .expect("write the project commercial settings");
    store
        .update_project_tooling(
            project_id,
            &solidbom_contract::UpdateProjectTooling {
                request_id: "tooling-1".into(),
                expected_version: 6,
                tooling: solidbom_contract::ProjectTooling {
                    default_treatment: Some(solidbom_contract::ToolingTreatment::Amortized),
                    entries: vec![solidbom_contract::ToolingEntry {
                        tool_id: Uuid::from_u128(0x7a01),
                        owner: solidbom_contract::ToolingOwner::PartVersion { part_version_id: body },
                        name: None,
                        tool_count: 1,
                        cost_per_tool: support::d("10000"),
                        currency: "CNY".into(),
                        source_reference: None,
                    }],
                },
            },
        )
        .await
        .expect("write the project tooling settings");
    let baseline = calculate(store, project_id, "baseline-1", 7).await;
    Fixture { project_id, product_id, baseline }
}

async fn calculate(store: &Store, project_id: Uuid, request_id: &str, project_version: u64) -> BaselineSnapshot {
    store
        .calculate_baseline(
            project_id,
            &CalculateBaseline {
                request_id: request_id.into(),
                project_version,
                engineering_version: None,
                manufacturing_version: None,
                product_revisions: Vec::new(),
            },
        )
        .await
        .expect("calculate the baseline the scenarios read")
}

fn content(baseline: &BaselineSnapshot, product_id: Uuid, price: &str) -> solidbom_contract::CommercialScenarioContent {
    solidbom_contract::CommercialScenarioContent {
        name: "baseline".into(),
        baseline_calculation_id: baseline.calculation_id,
        manual_prices: vec![solidbom_contract::ScenarioManualPrice { product_id, price: support::d(price) }],
        tooling: Default::default(),
        productivity_applies_to_tooling: false,
    }
}

async fn create(
    store: &Store,
    project_id: Uuid,
    scenario_id: Uuid,
    content: solidbom_contract::CommercialScenarioContent,
) -> solidbom_contract::CommercialScenario {
    store
        .create_commercial_scenario(project_id, &CreateCommercialScenario { request_id: format!("create-{scenario_id}"), scenario_id, content })
        .await
        .expect("create the scenario")
        .scenario
}

#[tokio::test]
async fn commercial_settings_are_append_only_idempotent_and_validated() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = setup(store).await;
    assert_eq!(store.project_record_version_count(fixture.project_id).await.expect("count"), 7);

    let current = store.project_record_view(fixture.project_id).await.expect("the current project record");
    assert_eq!(
        current.document.commercial,
        Some(ProjectCommercial { ebit_target_rate: support::d("0.2"), annual_productivity_rate: support::d("0.03") })
    );
    assert!(current.document.tooling.is_some());
    let before = store.project_record_version(fixture.project_id, 5).await.expect("the version before the settings");
    assert_eq!(before.document.commercial, None, "a document written before SB-18 reads as no settings, never an implicit target");

    let request = UpdateProjectCommercial {
        request_id: "commercial-1".into(),
        expected_version: 5,
        commercial: ProjectCommercial { ebit_target_rate: support::d("0.2"), annual_productivity_rate: support::d("0.03") },
    };
    let retried = store.update_project_commercial(fixture.project_id, &request).await.expect("retry the same request");
    assert_eq!(retried.receipt.version, 6);
    assert_eq!(store.project_record_version_count(fixture.project_id).await.expect("count"), 7, "an identical retry appends nothing");

    let stale = UpdateProjectCommercial { request_id: "commercial-2".into(), expected_version: 3, ..request };
    assert!(matches!(store.update_project_commercial(fixture.project_id, &stale).await, Err(StoreError::Conflict(_))));
    let refused = UpdateProjectCommercial {
        request_id: "commercial-3".into(),
        expected_version: 7,
        commercial: ProjectCommercial { ebit_target_rate: support::d("1"), annual_productivity_rate: support::d("0.03") },
    };
    assert!(matches!(store.update_project_commercial(fixture.project_id, &refused).await, Err(StoreError::Invalid(_))));
}

#[tokio::test]
async fn scenarios_are_append_only_versions_with_idempotency_and_baseline_validation() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = setup(store).await;
    let scenario_id = Uuid::from_u128(0x5c01);

    assert_eq!(store.commercial_scenarios(fixture.project_id).await.expect("the scenario list").scenarios, Vec::new());
    let mut first = content(&fixture.baseline, fixture.product_id, "20");
    first.tooling.treatment = Some(solidbom_contract::ToolingTreatment::Amortized);
    let created = create(store, fixture.project_id, scenario_id, first.clone()).await;
    assert_eq!(created.version, 1);
    assert_eq!(created.project_version, fixture.baseline.project_version);
    assert_eq!(created.baseline_calculation_id, fixture.baseline.calculation_id);
    assert_eq!(created.manual_prices.len(), 1);

    // The identical retry returns the recorded version and appends nothing.
    let retried = store
        .create_commercial_scenario(
            fixture.project_id,
            &CreateCommercialScenario { request_id: format!("create-{scenario_id}"), scenario_id, content: first.clone() },
        )
        .await
        .expect("retry the create");
    assert_eq!(retried.scenario, created);

    // A second scenario identity with the same id is refused, and a manual price for a product the
    // baseline does not price is refused.
    assert!(matches!(
        store
            .create_commercial_scenario(
                fixture.project_id,
                &CreateCommercialScenario { request_id: "create-other".into(), scenario_id, content: first.clone() }
            )
            .await,
        Err(StoreError::Conflict(_))
    ));
    let mut unknown_price = first.clone();
    unknown_price.manual_prices = vec![solidbom_contract::ScenarioManualPrice { product_id: Uuid::from_u128(0xdead), price: support::d("20") }];
    assert!(matches!(
        store
            .create_commercial_scenario(
                fixture.project_id,
                &CreateCommercialScenario { request_id: "create-unknown".into(), scenario_id: Uuid::from_u128(0x5c02), content: unknown_price }
            )
            .await,
        Err(StoreError::Invalid(_))
    ));

    // An edit appends exactly one version; a stale expectation and a changed payload under a used
    // request id are conflicts; the list returns only the current versions.
    let mut second = first.clone();
    second.name = "negotiated".into();
    second.manual_prices = vec![solidbom_contract::ScenarioManualPrice { product_id: fixture.product_id, price: support::d("21") }];
    let update = UpdateCommercialScenario { request_id: "update-1".into(), scenario_id, expected_version: 1, content: second.clone() };
    let updated = store.update_commercial_scenario(fixture.project_id, scenario_id, &update).await.expect("update the scenario");
    assert_eq!(updated.scenario.version, 2);
    assert_eq!(updated.scenario.name, "negotiated");
    let retained = store.commercial_scenarios(fixture.project_id).await.expect("the scenario list");
    assert_eq!(retained.scenarios, vec![updated.scenario.clone()]);

    let replay = store.update_commercial_scenario(fixture.project_id, scenario_id, &update).await.expect("retry the update");
    assert_eq!(replay, updated);
    let stale = UpdateCommercialScenario { request_id: "update-2".into(), scenario_id, expected_version: 1, content: second.clone() };
    assert!(matches!(store.update_commercial_scenario(fixture.project_id, scenario_id, &stale).await, Err(StoreError::Conflict(_))));
    let changed = UpdateCommercialScenario { request_id: "update-1".into(), scenario_id, expected_version: 2, content: first };
    assert!(matches!(store.update_commercial_scenario(fixture.project_id, scenario_id, &changed).await, Err(StoreError::Conflict(_))));
    assert!(matches!(
        store
            .update_commercial_scenario(
                fixture.project_id,
                Uuid::from_u128(0xdead),
                &UpdateCommercialScenario {
                    request_id: "update-3".into(),
                    scenario_id: Uuid::from_u128(0xdead),
                    expected_version: 1,
                    content: second
                }
            )
            .await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test]
async fn a_comparison_prices_every_scenario_against_the_common_baseline() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = setup(store).await;
    assert_eq!(fixture.baseline.outputs.state, ResultState::Complete, "{:?}", fixture.baseline.outputs.issues);
    let product_line = fixture.baseline.outputs.products.first().expect("the baseline product line");
    assert_eq!(product_line.recurring_unit_cost, Some(support::d("17.508")));
    assert_eq!(fixture.baseline.outputs.setup_total, Some(support::d("60")));

    // One amortized scenario with a manual recovery quantity: 10,000 / 5,000 = 2 per unit.
    let amortized_id = Uuid::from_u128(0x5c11);
    let mut amortized = content(&fixture.baseline, fixture.product_id, "20");
    amortized.tooling = solidbom_contract::ToolingRecoveryInputs {
        treatment: Some(solidbom_contract::ToolingTreatment::Amortized),
        recovery: vec![solidbom_contract::ToolingRecoveryChoice {
            tool_id: Uuid::from_u128(0x7a01),
            basis: solidbom_contract::ToolingRecoveryBasis::ManualQuantity { quantity: Some(support::d("5000")) },
        }],
    };
    create(store, fixture.project_id, amortized_id, amortized).await;

    let compare = store
        .compare_commercial_scenarios(
            fixture.project_id,
            &solidbom_contract::CompareCommercialScenarios {
                baseline_calculation_id: fixture.baseline.calculation_id,
                scenario_ids: vec![amortized_id],
            },
        )
        .await
        .expect("compare the scenario");
    assert_eq!(compare.project_id, fixture.project_id);
    assert_eq!(compare.project_version, fixture.baseline.project_version);
    assert_eq!(compare.ebit_target_rate, Some(support::d("0.2")));
    assert_eq!(compare.annual_productivity_rate, Some(support::d("0.03")));
    assert_eq!(compare.volume_policy, Some(CostingVolumePolicy::Average));
    assert_eq!(compare.scenarios.len(), 1);
    let line = &compare.scenarios[0];
    assert!(!line.stale, "the scenario was saved against the compared baseline: {:?}", line.stale_reasons);
    assert_eq!(line.version, 1);
    let outcome = &line.outcome;
    assert_eq!(outcome.state, ResultState::Complete, "{:?}", outcome.issues);
    let product = &outcome.products[0];
    assert_eq!(product.suggested_first_year_price, Some(support::d("21.885")), "C / (1 - 0.2)");
    assert_eq!(product.manual_price, Some(support::d("20")));
    assert_eq!(product.manual_first_year_ebit_amount, Some(support::d("2.492")));
    assert_eq!(product.manual_first_year_ebit_rate, Some(support::d("0.1246")));
    assert_eq!(product.tooling_initial_contribution, Some(support::d("2")));
    assert_eq!(product.years.len(), 3, "one price line per production year");
    assert_eq!(product.years[0].suggested_base_price, Some(support::d("21.885")));
    assert_eq!(product.years[1].suggested_base_price, Some(support::d("21.22845")));
    assert_eq!(product.years[2].suggested_base_price, Some(support::d("20.5915965")));
    assert_eq!(product.years[1].tooling_contribution, Some(support::d("2")), "productivity is exempt from the contribution");
    assert_eq!(product.years[0].manual_total_price, Some(support::d("22")));
    assert_eq!(product.years[1].manual_total_price, Some(support::d("21.4")));
    assert_eq!(product.years[0].recurring_unit_cost, Some(support::d("17.508")), "the manufacturing cost stays fixed");
    assert_eq!(product.years[2].recurring_unit_cost, Some(support::d("17.508")));
    assert_eq!(product.years[0].revenue, Some(support::d("2000")), "manual base price x forecast");
    assert_eq!(product.years[1].ebit_amount, Some(support::d("378.4")), "(19.4 - 17.508) x 200");
    assert_eq!(outcome.project.lifetime_revenue, Some(support::d("11525.4")), "2000 + 3880 + 5645.4");
    assert_eq!(outcome.project.tooling_investment, Some(support::d("10000")));
    assert_eq!(outcome.project.tooling_projected_recovery, Some(support::d("1200")), "2 x 600 lifetime units");
    assert_eq!(outcome.project.tooling_under_over_recovery, Some(support::d("-8800")));
    assert_eq!(outcome.tooling.treatment, Some(solidbom_contract::ToolingTreatment::Amortized));
    assert_eq!(outcome.setup_total, Some(support::d("60")), "the setup charge stays separate");

    // The separate scenario keeps the same investment as one separate charge and no unit price
    // contribution.
    let separate_id = Uuid::from_u128(0x5c12);
    let mut separate = content(&fixture.baseline, fixture.product_id, "20");
    separate.tooling =
        solidbom_contract::ToolingRecoveryInputs { treatment: Some(solidbom_contract::ToolingTreatment::Separate), recovery: Vec::new() };
    create(store, fixture.project_id, separate_id, separate).await;
    let compare = store
        .compare_commercial_scenarios(
            fixture.project_id,
            &solidbom_contract::CompareCommercialScenarios {
                baseline_calculation_id: fixture.baseline.calculation_id,
                scenario_ids: vec![amortized_id, separate_id],
            },
        )
        .await
        .expect("compare both scenarios");
    let separate_line = compare.scenarios.iter().find(|line| line.scenario_id == separate_id).expect("the separate scenario line");
    let separate_outcome = &separate_line.outcome;
    assert_eq!(separate_outcome.state, ResultState::Complete, "{:?}", separate_outcome.issues);
    assert_eq!(separate_outcome.products[0].tooling_initial_contribution, None);
    assert_eq!(separate_outcome.products[0].years[0].tooling_contribution, None);
    assert_eq!(separate_outcome.products[0].years[0].manual_total_price, Some(support::d("20")), "the separate charge is outside the unit price");
    assert_eq!(separate_outcome.project.tooling_separate_charge, Some(support::d("10000")));
    assert_eq!(separate_outcome.project.tooling_projected_recovery, None);
    assert_eq!(separate_outcome.tooling.project_investment, Some(support::d("10000")));
    assert_eq!(compare.scenarios.len(), 2);
}

#[tokio::test]
async fn a_comparison_labels_a_scenario_saved_against_another_baseline() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = setup(store).await;
    let scenario_id = Uuid::from_u128(0x5c21);
    create(store, fixture.project_id, scenario_id, content(&fixture.baseline, fixture.product_id, "20")).await;

    // The same scenario against its own baseline, before a newer baseline exists, is not stale.
    let same = store
        .compare_commercial_scenarios(
            fixture.project_id,
            &solidbom_contract::CompareCommercialScenarios {
                baseline_calculation_id: fixture.baseline.calculation_id,
                scenario_ids: vec![scenario_id],
            },
        )
        .await
        .expect("compare against its own baseline");
    assert!(!same.scenarios[0].stale, "{:?}", same.scenarios[0].stale_reasons);

    // A newer baseline of the same project version exists; the scenario pinned the older one.
    let newer = calculate(store, fixture.project_id, "baseline-2", fixture.baseline.project_version).await;
    let compare = store
        .compare_commercial_scenarios(
            fixture.project_id,
            &solidbom_contract::CompareCommercialScenarios { baseline_calculation_id: newer.calculation_id, scenario_ids: vec![scenario_id] },
        )
        .await
        .expect("compare against the newer baseline");
    let line = &compare.scenarios[0];
    assert!(line.stale);
    let codes: Vec<&str> = line.stale_reasons.iter().map(|reason| reason.code.as_str()).collect();
    assert!(codes.contains(&"scenario_baseline_differs"), "{codes:?}");
    assert!(codes.contains(&"scenario_baseline_outdated"), "{codes:?}");
    assert!(!codes.contains(&"scenario_project_version_differs"), "{codes:?}");
    assert_eq!(line.saved_baseline_calculation_id, fixture.baseline.calculation_id);
    assert_eq!(line.outcome.state, ResultState::Complete, "{:?}", line.outcome.issues);

    // A changed project settings version and a baseline of it make the scenario stale under
    // different settings, and its own outcome stays unchanged.
    let current = store.project_record_view(fixture.project_id).await.expect("the current project record").version;
    let settings = store
        .update_project_commercial(
            fixture.project_id,
            &UpdateProjectCommercial {
                request_id: "commercial-2".into(),
                expected_version: current,
                commercial: ProjectCommercial { ebit_target_rate: support::d("0.25"), annual_productivity_rate: support::d("0.03") },
            },
        )
        .await
        .expect("change the project settings");
    assert_eq!(settings.receipt.version, current + 1);
    let newer_settings = calculate(store, fixture.project_id, "baseline-3", settings.receipt.version).await;
    let compare = store
        .compare_commercial_scenarios(
            fixture.project_id,
            &solidbom_contract::CompareCommercialScenarios {
                baseline_calculation_id: newer_settings.calculation_id,
                scenario_ids: vec![scenario_id],
            },
        )
        .await
        .expect("compare under different settings");
    let line = &compare.scenarios[0];
    let codes: Vec<&str> = line.stale_reasons.iter().map(|reason| reason.code.as_str()).collect();
    assert!(codes.contains(&"scenario_project_version_differs"), "{codes:?}");
    assert!(codes.contains(&"scenario_settings_differ"), "{codes:?}");
    assert!(codes.contains(&"scenario_project_outdated"), "{codes:?}");
    assert_eq!(compare.ebit_target_rate, Some(support::d("0.25")), "the comparison's shared settings are used");
    assert_eq!(line.outcome.ebit_target_rate, Some(support::d("0.25")), "no scenario overrides the project target");
}

#[tokio::test]
async fn a_comparison_stores_nothing_and_refuses_unknown_scenarios() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = setup(store).await;
    let scenario_id = Uuid::from_u128(0x5c31);
    create(store, fixture.project_id, scenario_id, content(&fixture.baseline, fixture.product_id, "20")).await;
    let versions = store.project_record_version_count(fixture.project_id).await.expect("count");
    let baselines = store.baseline_calculations(fixture.project_id).await.expect("the baseline list").calculations.len();

    store
        .compare_commercial_scenarios(
            fixture.project_id,
            &solidbom_contract::CompareCommercialScenarios {
                baseline_calculation_id: fixture.baseline.calculation_id,
                scenario_ids: vec![scenario_id],
            },
        )
        .await
        .expect("the read-only comparison");
    assert_eq!(store.project_record_version_count(fixture.project_id).await.expect("count"), versions, "a comparison appends no project version");
    assert_eq!(
        store.baseline_calculations(fixture.project_id).await.expect("the baseline list").calculations.len(),
        baselines,
        "a comparison appends no baseline"
    );
    assert_eq!(store.commercial_scenarios(fixture.project_id).await.expect("the scenario list").scenarios.len(), 1);

    assert!(matches!(
        store
            .compare_commercial_scenarios(
                fixture.project_id,
                &solidbom_contract::CompareCommercialScenarios {
                    baseline_calculation_id: fixture.baseline.calculation_id,
                    scenario_ids: vec![Uuid::from_u128(0xdead)],
                },
            )
            .await,
        Err(StoreError::NotFound)
    ));
}
