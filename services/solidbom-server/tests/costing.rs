//! Store-level tests for the project costing settings and the immutable shared-demand and
//! manufacturing-cost baseline snapshots (contract version 1 of the SB-16 costing contract,
//! additive to API version 2).
//!
//! `#[tokio::test]` is not recognised as a test function by Clippy's test allowance, so this file
//! allows the panicking helpers it needs. The library and binary still deny them.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

mod support;

use solidbom_contract::{
    AnalysisKey, AnalysisManifest, AnnualForecast, BaselineAnnualDemand, BaselineAnnualProjectCost, BaselinePurchasePriceSource, BaselineSetupOwner,
    CalculateBaseline, CatalogConfirmation, ConfirmParts, CostOwner, CostingVolumePolicy, CreateProject, CycleBasis, CycleOrigin, DefinitionRef,
    DeliveredPartInput, DeliveredProductInput, DeliveryPackage, EditAttachedCosts, EngineeringWriteOutcome, ExchangeRate, GeometryBinding,
    GroupSolids, ManualRouting, MaterialConsumption, Measurement, MeasurementSource, OperationCycle, PartConfirmation, ProductKind,
    ProductRevisionSelection, ProjectCosting, ProjectDetails, PurchasePriceTable, PurchasePriceTier, RecordDelivery, RegisterAnalysis, ReviewState,
    ReviewedGeometry, RoutingLevel, RoutingOperation, RoutingWrite, SCHEMA_VERSION, SetSourcing, SetupTime, SourcedPart, Sourcing, Unit,
    UpdateProduct, UpdateProjectCosting, YearRange,
};
use solidbom_server::{Store, StoreError};
use support::TestDb;
use uuid::Uuid;

const SOURCE: &[u8] =
    b"ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('shared demand baseline fixture'),'2;1');\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n";
const IDENTITY: [[f64; 4]; 4] = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];

fn project_id() -> Uuid {
    Uuid::from_u128(0xb161)
}

fn other_project_id() -> Uuid {
    Uuid::from_u128(0xb169)
}

fn analysis_key() -> AnalysisKey {
    AnalysisKey { source_sha256: support::digest(SOURCE), engine_build: "0.40.0-rc.2".into(), extractor_version: "1".into() }
}

fn production_rate_id() -> Uuid {
    support::seeded_rate_version_id()
}

fn setup_rate_id() -> Uuid {
    Uuid::parse_str("00000000-0000-4000-8000-000000000102").expect("the seeded setup rate fixture")
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

fn details(default_schedule: YearRange) -> ProjectDetails {
    ProjectDetails {
        name: "Shared demand inquiry".into(),
        customer: "Customer".into(),
        internal_team: None,
        platform: None,
        application: None,
        currency: "CNY".into(),
        default_schedule,
    }
}

fn delivered_part(delivered_key: &str, number: &str, revision: &str) -> DeliveredPartInput {
    DeliveredPartInput {
        delivered_key: delivered_key.into(),
        customer_part_number: Some(number.into()),
        customer_revision: Some(revision.into()),
        volume_mm3: Some(support::d("2400")),
    }
}

/// The exact analysis of the product revision: one placement of the made part (resolved through a
/// reviewed component in `setup`) and one placement of the purchased part (resolved through the
/// definition's customer identifiers). The part versions are resolved by the service from the
/// project part versions, never from the manifest.
fn manifest() -> AnalysisManifest {
    let body = Uuid::from_u128(0xde01);
    let bolt = Uuid::from_u128(0xde02);
    let body_occurrence = Uuid::from_u128(0x0cc1);
    let bolt_occurrence = Uuid::from_u128(0x0cc2);
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

fn cycle(seconds: &str, basis: CycleBasis, cavities: Option<u32>, rate_version_id: Uuid) -> OperationCycle {
    OperationCycle {
        seconds: Some(support::d(seconds)),
        basis,
        cavities,
        production_rate_version_id: Some(rate_version_id),
        origin: CycleOrigin::Manual,
        source_reference: None,
    }
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

fn attached_line(line_id: u128, owner: CostOwner, quantity: &str, unit_cost: &str) -> solidbom_contract::AttachedCostLine {
    solidbom_contract::AttachedCostLine {
        line_id: Uuid::from_u128(line_id),
        owner,
        category: "packaging".into(),
        description: Some("returnable tray".into()),
        quantity: Some(support::d(quantity)),
        unit: "pcs".into(),
        unit_cost: Some(support::d(unit_cost)),
        source_reference: None,
    }
}

/// The agreed purchase tier table of one part version: the greatest covered minimum selects.
fn purchase_table(part_version_id: Uuid) -> PurchasePriceTable {
    PurchasePriceTable {
        part_version_id,
        currency: "CNY".into(),
        tiers: vec![
            PurchasePriceTier { minimum_annual_quantity: 1, unit_price: support::d("10") },
            PurchasePriceTier { minimum_annual_quantity: 250, unit_price: support::d("8") },
        ],
        source_reference: Some("agreed purchase tiers 2027".into()),
    }
}

fn costing(part_version_id: Uuid) -> ProjectCosting {
    ProjectCosting {
        volume_policy: Some(CostingVolumePolicy::Average),
        exchange_rates: Vec::new(),
        purchase_prices: vec![purchase_table(part_version_id)],
    }
}

/// One fully resolved baseline fixture: a quoted product with one made part (material, routing
/// setup and an attached line), one purchased part (confirmed catalog price plus a project tier),
/// the project costing settings and a published category configuration.
struct Fixture {
    project_id: Uuid,
    product_id: Uuid,
    revision_id: Uuid,
    body: Uuid,
    bolt: Uuid,
}

async fn setup(store: &Store) -> Fixture {
    let source = store.put_source(&support::digest(SOURCE), "baseline.step", SOURCE).await.expect("store the source");
    store
        .create_project(&CreateProject {
            request_id: "create-1".into(),
            project_id: project_id(),
            details: details(YearRange { sop_year: 2027, eop_year: 2027 }),
        })
        .await
        .expect("create the project");
    store
        .record_delivery(
            project_id(),
            &RecordDelivery {
                request_id: "delivery-1".into(),
                expected_version: 1,
                package: DeliveryPackage { package_id: Uuid::from_u128(0xb162), name: "Package 1".into(), source },
                products: vec![DeliveredProductInput {
                    product_id: None,
                    name: "assembly".into(),
                    kind: ProductKind::Assembly,
                    customer_part_number: None,
                    parts: vec![delivered_part("body", "M-1", "A"), delivered_part("bolt", "P-1", "A")],
                }],
            },
        )
        .await
        .expect("record the delivery");
    let view = store.project_record_view(project_id()).await.expect("the project record view");
    let product = view.document.products.first().expect("the delivered product");
    let product_id = product.product_id;
    let revision_id = product.revisions.first().expect("the delivered revision").product_revision_id;
    let body = view.document.part_version_by_identity("M-1", "A").expect("the made part version").part_version_id;
    let bolt = view.document.part_version_by_identity("P-1", "A").expect("the purchased part version").part_version_id;

    // The quoted product schedule is exactly one positive forecast year, so no year is incomplete.
    store
        .update_product(
            project_id(),
            &UpdateProduct {
                request_id: "product-1".into(),
                expected_version: 2,
                product_id,
                name: "assembly".into(),
                kind: ProductKind::Assembly,
                customer_part_number: None,
                schedule_override: Some(YearRange { sop_year: 2027, eop_year: 2027 }),
                forecasts: vec![AnnualForecast { year: 2027, quantity: 100 }],
            },
        )
        .await
        .expect("set the schedule and forecast");

    // The exact analysis and the reviewed component that binds the made part's body to its shared
    // part version. The purchased part resolves through its manifest definition identifiers.
    store
        .register_analysis(
            project_id(),
            &RegisterAnalysis { request_id: "analysis-1".into(), product_revision_id: revision_id, manifest: manifest() },
        )
        .await
        .expect("register the analysis");
    let grouped: EngineeringWriteOutcome = store
        .group_solids(
            project_id(),
            &GroupSolids {
                request_id: "group-1".into(),
                expected_version: 0,
                component_id: Uuid::from_u128(0xc001),
                product_revision_id: revision_id,
                analysis_key: analysis_key(),
                geometry: vec![ReviewedGeometry { source_path: Vec::new(), source_body: 0 }],
                part_version_id: Some(body),
            },
        )
        .await
        .expect("group the reviewed component");
    assert_eq!(grouped.receipt.version, 1);

    // The engineer-confirmed catalog reference is the purchased part's single price.
    let component_id = Uuid::from_u128(0xca71);
    let component = store
        .create_component_version(&support::component_version("component-1", 0, component_id, Some("8")), "administrator")
        .await
        .expect("publish the catalog component");
    let component_version_id = component.component.component_version_id;
    let confirmed = store
        .confirm_parts(
            project_id(),
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
    assert_eq!(confirmed.receipt.version, 2);

    // Explicit sourcing: the made part records its net consumption and allowance, the purchased
    // part records no material at all.
    store
        .set_sourcing(
            project_id(),
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
    // The made part's routing: a 30-second per-piece cycle at the seeded production rate and a
    // dedicated 0.5-hour setup at the seeded setup rate.
    store
        .write_routing(
            project_id(),
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
                    cycle("30", CycleBasis::PerPiece, None, production_rate_id()),
                    Some(SetupTime { hours: Some(support::d("0.5")), setup_rate_version_id: Some(setup_rate_id()), source_reference: None }),
                )],
                affected_part_versions: vec![body],
                affected_products: Vec::new(),
            }),
        )
        .await
        .expect("write the part routing");
    // One attached non-CAD line of the made part: 2 pieces at 0.50 project-currency units each.
    store
        .edit_attached_costs(
            project_id(),
            &EditAttachedCosts {
                request_id: "attached-1".into(),
                expected_version: 2,
                upsert: vec![attached_line(0x7830, CostOwner::PartVersion { part_version_id: body }, "2", "0.50")],
                remove: Vec::new(),
                affected_part_versions: vec![body],
                affected_products: Vec::new(),
            },
        )
        .await
        .expect("write the attached cost line");
    // The published category configuration. Empty categories keep the fixture's arithmetic to the
    // direct manufacturing lines; the categories themselves are the calculator's tests.
    store
        .create_app_config_version(&support::app_config_version("config-1", 0, Uuid::from_u128(0xcf01), Vec::new()), "administrator")
        .await
        .expect("publish the app cost configuration");
    let written = store
        .update_project_costing(project_id(), &UpdateProjectCosting { request_id: "costing-1".into(), expected_version: 3, costing: costing(bolt) })
        .await
        .expect("write the project costing settings");
    assert_eq!(written.receipt.version, 4);
    // Activation is SB-19's transition; the baseline reads the active revision when the request
    // selects none. This makes the fallback path real instead of reaching for a latest draft.
    let activated = store.record_revision_activation(project_id(), revision_id).await.expect("activate the delivered revision");
    assert_eq!(activated.version, 5);
    Fixture { project_id: project_id(), product_id, revision_id, body, bolt }
}

#[tokio::test]
async fn a_resolved_baseline_keeps_exact_inputs_and_an_immutable_snapshot() {
    let db = TestDb::new().await;
    let store = &db.store;
    let fixture = setup(store).await;

    let request = CalculateBaseline {
        request_id: "baseline-1".into(),
        project_version: 5,
        engineering_version: None,
        manufacturing_version: None,
        product_revisions: Vec::new(),
    };
    let snapshot = store.calculate_baseline(fixture.project_id, &request).await.expect("calculate the baseline");
    assert_eq!(snapshot.project_id, fixture.project_id, "{snapshot:?}");
    assert_eq!(snapshot.project_version, 5);
    assert_eq!(snapshot.engineering_version, 2, "the optional versions default to the current records");
    assert_eq!(snapshot.manufacturing_version, 3);
    assert_eq!(snapshot.calculator_version, "solidbom-manufacturing-baseline/1");
    assert!(!snapshot.input_hash.is_empty());
    assert!(snapshot.created_at.is_some());

    // The resolved portable inputs tie every selected version and quantity to the stored records.
    let inputs = &snapshot.inputs;
    assert_eq!(inputs.project_currency, "CNY");
    assert_eq!(inputs.volume_policy, Some(CostingVolumePolicy::Average));
    assert!(inputs.exchange_rates.is_empty());
    let config = inputs.category_config.as_ref().expect("the published category configuration");
    assert_eq!(config.currency, "CNY");
    assert!(config.categories.is_empty());
    assert_eq!(inputs.parts.len(), 2, "one part input exists for every project part version: {:?}", inputs.parts);
    let body_input = inputs.part(fixture.body).expect("the made part input");
    assert_eq!(body_input.sourcing, Some(Sourcing::Manufactured));
    let material = body_input.material.as_ref().expect("the made part material record");
    assert_eq!(material.material_version_id, Some(support::material_id()), "the snapshot inputs retain the selected material version");
    assert_eq!(material.net_consumption, Some(support::d("100")));
    assert_eq!(material.unit, Unit::CentimetreCubed);
    assert_eq!(material.density_g_cm3, Some(support::d("2.7")));
    assert_eq!(material.price_per_kg, Some(support::d("20")));
    assert_eq!(material.price_currency, "CNY");
    assert_eq!(material.allowance_rate, Some(support::d("0.02")));
    assert_eq!(body_input.purchase, None);
    assert_eq!(body_input.operations.len(), 1);
    let operation = &body_input.operations[0];
    let cycle = operation.cycle.as_ref().expect("the pinned production cycle");
    assert_eq!(cycle.production_rate_version_id, Some(production_rate_id()), "the snapshot inputs retain the pinned production rate version");
    assert_eq!(cycle.seconds, Some(support::d("30")));
    assert_eq!(cycle.basis, CycleBasis::PerPiece);
    assert_eq!(cycle.rate_per_hour, Some(support::d("120")));
    assert_eq!(cycle.rate_currency, "CNY");
    let setup = operation.setup.as_ref().expect("the dedicated setup");
    assert_eq!(setup.setup_rate_version_id, Some(setup_rate_id()), "the snapshot inputs retain the pinned setup rate version");
    assert_eq!(setup.hours, Some(support::d("0.5")));
    assert_eq!(setup.rate_per_hour, Some(support::d("120")));
    assert_eq!(body_input.attached_costs.len(), 1);
    assert_eq!(body_input.attached_costs[0].quantity, Some(support::d("2")));
    assert_eq!(body_input.attached_costs[0].unit_cost, Some(support::d("0.50")));
    let bolt_input = inputs.part(fixture.bolt).expect("the purchased part input");
    assert_eq!(bolt_input.sourcing, Some(Sourcing::Purchased));
    assert_eq!(bolt_input.material, None, "a purchased part records no manufacturing material");
    let purchase = bolt_input.purchase.as_ref().expect("the purchased price source");
    assert_eq!(purchase.currency, "CNY", "the tier table keeps its own currency");
    assert_eq!(purchase.price_unit, "pcs");
    assert_eq!(purchase.tiers.len(), 2);
    let single = purchase.single_price.as_ref().expect("the confirmed catalog single price");
    assert_eq!(single.amount, support::d("8"));
    assert_eq!(single.currency, "CNY");
    // The product structure resolves once through the reviewed component and once through the
    // manifest definition's customer identifiers.
    assert_eq!(inputs.products.len(), 1);
    let product = &inputs.products[0];
    assert_eq!(product.product_id, fixture.product_id);
    assert!(product.quoted);
    assert_eq!(product.schedule, YearRange { sop_year: 2027, eop_year: 2027 });
    assert_eq!(product.forecasts, vec![AnnualForecast { year: 2027, quantity: 100 }]);
    assert_eq!(product.occurrences.len(), 2, "{:?}", product.occurrences);
    assert_eq!(product.occurrences[0].part_version_id, Some(fixture.body));
    assert_eq!(product.occurrences[1].part_version_id, Some(fixture.bolt));
    assert!(product.occurrences.iter().all(|occurrence| occurrence.children.is_empty()));

    // The explained answer carries the exact arithmetic: V6's made part (material 5.508, production
    // 1, attached 1, unit 7.508, setup 60 once) and the V4 tier price (greatest covered minimum).
    assert_eq!(snapshot.outputs.state, solidbom_contract::ResultState::Complete, "{:?}", snapshot.outputs.issues);
    let body_line = snapshot.outputs.parts.iter().find(|part| part.part_version_id == fixture.body).expect("the made part line");
    assert_eq!(body_line.annual_demand, vec![BaselineAnnualDemand { year: 2027, quantity: Some(100) }]);
    assert_eq!(body_line.lifetime_demand, Some(100));
    assert_eq!(body_line.peak_demand, Some(100));
    assert_eq!(body_line.costing_volume, Some(support::d("100")));
    assert_eq!(body_line.material_cost, Some(support::d("5.508")));
    assert_eq!(body_line.production_cost, Some(support::d("1")));
    assert_eq!(body_line.attached_cost, Some(support::d("1")));
    assert_eq!(body_line.unit_cost, Some(support::d("7.508")));
    assert_eq!(body_line.setup_charge, Some(support::d("60")));
    let bolt_line = snapshot.outputs.parts.iter().find(|part| part.part_version_id == fixture.bolt).expect("the purchased part line");
    assert_eq!(bolt_line.lifetime_demand, Some(100));
    assert_eq!(bolt_line.costing_volume, Some(support::d("100")));
    let selected = bolt_line.purchase_price.as_ref().expect("the selected purchase price");
    assert_eq!(selected.source, BaselinePurchasePriceSource::Tier { minimum_annual_quantity: 1 });
    assert_eq!(selected.currency, "CNY");
    assert_eq!(selected.source_unit_price, support::d("10"));
    assert_eq!(selected.project_unit_price, support::d("10"));
    assert_eq!(selected.fx_rate, None);
    assert_eq!(bolt_line.unit_cost, Some(support::d("10")));
    let product_line = snapshot.outputs.products.first().expect("the product line");
    assert_eq!(product_line.product_id, fixture.product_id);
    assert_eq!(product_line.material_cost, Some(support::d("15.508")));
    assert_eq!(product_line.production_cost, Some(support::d("1")));
    assert_eq!(product_line.attached_cost, Some(support::d("1")));
    assert_eq!(product_line.unit_total, Some(support::d("17.508")));
    assert_eq!(snapshot.outputs.setup_total, Some(support::d("60")));
    assert!(
        snapshot.outputs.setup.iter().any(
            |line| line.owner == BaselineSetupOwner::PartVersion { part_version_id: fixture.body } && line.setup_charge == Some(support::d("60"))
        ),
        "the body routing's setup is counted once: {:?}",
        snapshot.outputs.setup
    );
    assert!(snapshot.outputs.setup_categories.is_empty());
    assert_eq!(snapshot.outputs.annual, vec![BaselineAnnualProjectCost { year: 2027, manufacturing_cost: Some(support::d("1750.8")) }]);
    assert_eq!(snapshot.outputs.project_total_manufacturing_cost, Some(support::d("1810.8")));

    // An identical retry returns the recorded snapshot before any record or version check.
    let retried = store.calculate_baseline(fixture.project_id, &request).await.expect("retry the same request");
    assert_eq!(retried, snapshot, "the recorded answer is returned unchanged");

    // A changed payload under the same request id is a conflict.
    let changed = CalculateBaseline { project_version: 3, ..request.clone() };
    assert!(matches!(store.calculate_baseline(fixture.project_id, &changed).await, Err(StoreError::Conflict(_))));

    // An explicit revision selection for the same product revision is equivalent to the active
    // fallback and is recorded in the inputs, so a draft baseline never reads a revision implicitly.
    let selected_request = CalculateBaseline {
        request_id: "baseline-selected".into(),
        project_version: 5,
        engineering_version: None,
        manufacturing_version: None,
        product_revisions: vec![ProductRevisionSelection { product_id: fixture.product_id, product_revision_id: fixture.revision_id }],
    };
    let selected = store.calculate_baseline(fixture.project_id, &selected_request).await.expect("calculate with an explicit revision");
    assert_eq!(selected.inputs, snapshot.inputs);
    assert_eq!(selected.inputs.products[0].product_revision_id, Some(fixture.revision_id));
    // A revision the product does not hold is refused; a project version before activation has no
    // active revision, so the product carries no structure instead of a silently chosen latest one.
    let unknown_revision = CalculateBaseline {
        request_id: "baseline-unknown-revision".into(),
        project_version: 5,
        engineering_version: None,
        manufacturing_version: None,
        product_revisions: vec![ProductRevisionSelection { product_id: fixture.product_id, product_revision_id: Uuid::new_v4() }],
    };
    assert!(matches!(store.calculate_baseline(fixture.project_id, &unknown_revision).await, Err(StoreError::Invalid(_))));
    let not_activated = CalculateBaseline {
        request_id: "baseline-not-activated".into(),
        project_version: 3,
        engineering_version: None,
        manufacturing_version: None,
        product_revisions: Vec::new(),
    };
    let missing_revision = store.calculate_baseline(fixture.project_id, &not_activated).await.expect("a project version without an active revision");
    assert_eq!(missing_revision.inputs.products[0].product_revision_id, None);
    assert!(missing_revision.inputs.products[0].occurrences.is_empty());

    // A later quote edit appends a new project version and a new snapshot; the earlier answer is
    // still readable exactly as it was written and no row is rewritten.
    store
        .update_product(
            fixture.project_id,
            &UpdateProduct {
                request_id: "product-2".into(),
                expected_version: 5,
                product_id: fixture.product_id,
                name: "assembly".into(),
                kind: ProductKind::Assembly,
                customer_part_number: None,
                schedule_override: Some(YearRange { sop_year: 2027, eop_year: 2027 }),
                forecasts: vec![AnnualForecast { year: 2027, quantity: 200 }],
            },
        )
        .await
        .expect("edit the forecast");
    let later_request = CalculateBaseline {
        request_id: "baseline-2".into(),
        project_version: 6,
        engineering_version: None,
        manufacturing_version: None,
        product_revisions: Vec::new(),
    };
    let later = store.calculate_baseline(fixture.project_id, &later_request).await.expect("calculate the later baseline");
    assert_ne!(later.calculation_id, snapshot.calculation_id);
    assert_eq!(later.inputs.products[0].forecasts, vec![AnnualForecast { year: 2027, quantity: 200 }]);
    assert_eq!(later.outputs.project_total_manufacturing_cost, Some(support::d("3561.6")));
    // Reference data changes too: a newer material version and a newer app configuration are
    // published after the first answer. The saved inputs keep the exact versions they selected, so
    // the historical snapshot stays byte-for-byte the answer that was accepted.
    store
        .create_material_version(&support::material_version("material-2", 1, support::material_id(), "2.7", Some("30")), "administrator")
        .await
        .expect("publish a newer material version");
    store
        .create_app_config_version(
            &support::app_config_version("config-2", 1, Uuid::from_u128(0xcf01), vec![support::fixed_category("packaging", "5")]),
            "administrator",
        )
        .await
        .expect("publish a newer app configuration");
    let reread = store.baseline_calculation(fixture.project_id, snapshot.calculation_id).await.expect("re-read the historical snapshot");
    assert_eq!(reread, snapshot, "a later edit or reference change never rewrites a saved snapshot");
    let history = store.baseline_calculations(fixture.project_id).await.expect("the baseline history");
    assert_eq!(history.project_id, fixture.project_id);
    // The history holds six answers: the four explicit calculations plus one draft publication per
    // forecast edit, because a forecast/schedule edit is one of the two confirmed SB-20 automatic
    // publication triggers. The two forecast edits in this fixture are the delivery-time product
    // update and the later quote edit.
    assert_eq!(history.calculations.len(), 6);
    assert!(history.calculations.iter().any(|summary| summary.calculation_id == snapshot.calculation_id));
    assert!(history.calculations.iter().any(|summary| summary.calculation_id == selected.calculation_id));
    assert!(history.calculations.iter().any(|summary| summary.calculation_id == missing_revision.calculation_id));
    assert!(history.calculations.iter().any(|summary| summary.calculation_id == later.calculation_id));
    assert_eq!(history.calculations.iter().filter(|summary| summary.project_version == 5).count(), 2);
    // Project version 3 holds the delivery-time baseline and the draft publication the product
    // update published with it; the explicit calculation no longer has that version to itself.
    assert_eq!(history.calculations.iter().filter(|summary| summary.project_version == 3).count(), 2);

    // Unknown, historical and foreign reads: an unknown project or calculation is not found, and a
    // calculation of another project cannot be read through this project's route.
    store
        .create_project(&CreateProject {
            request_id: "create-2".into(),
            project_id: other_project_id(),
            details: details(YearRange { sop_year: 2027, eop_year: 2027 }),
        })
        .await
        .expect("create the other project");
    assert!(matches!(store.baseline_calculation(fixture.project_id, Uuid::new_v4()).await, Err(StoreError::NotFound)));
    assert!(matches!(store.baseline_calculation(other_project_id(), snapshot.calculation_id).await, Err(StoreError::NotFound)));
    assert!(matches!(store.baseline_calculations(Uuid::new_v4()).await, Err(StoreError::NotFound)));
    let unknown_project = CalculateBaseline {
        request_id: "baseline-3".into(),
        project_version: 1,
        engineering_version: None,
        manufacturing_version: None,
        product_revisions: Vec::new(),
    };
    assert!(matches!(store.calculate_baseline(Uuid::new_v4(), &unknown_project).await, Err(StoreError::NotFound)));
    let unknown_version = CalculateBaseline {
        request_id: "baseline-4".into(),
        project_version: 9,
        engineering_version: None,
        manufacturing_version: None,
        product_revisions: Vec::new(),
    };
    assert!(matches!(store.calculate_baseline(fixture.project_id, &unknown_version).await, Err(StoreError::NotFound)));
    let unknown_engineering = CalculateBaseline {
        request_id: "baseline-5".into(),
        project_version: 5,
        engineering_version: Some(9),
        manufacturing_version: None,
        product_revisions: Vec::new(),
    };
    assert!(matches!(store.calculate_baseline(fixture.project_id, &unknown_engineering).await, Err(StoreError::NotFound)));
    let unknown_manufacturing = CalculateBaseline {
        request_id: "baseline-6".into(),
        project_version: 5,
        engineering_version: None,
        manufacturing_version: Some(9),
        product_revisions: Vec::new(),
    };
    assert!(matches!(store.calculate_baseline(fixture.project_id, &unknown_manufacturing).await, Err(StoreError::NotFound)));
    db.cleanup().await;
}

#[tokio::test]
async fn costing_settings_are_append_only_idempotent_and_validated_against_the_project() {
    let db = TestDb::new().await;
    let store = &db.store;
    store
        .create_project(&CreateProject {
            request_id: "create-1".into(),
            project_id: project_id(),
            details: details(YearRange { sop_year: 2027, eop_year: 2028 }),
        })
        .await
        .expect("create the project");

    let settings = ProjectCosting {
        volume_policy: Some(CostingVolumePolicy::Maximum),
        exchange_rates: vec![ExchangeRate { source_currency: "USD".into(), project_units_per_source_unit: support::d("7.2") }],
        purchase_prices: Vec::new(),
    };
    let write = UpdateProjectCosting { request_id: "costing-1".into(), expected_version: 1, costing: settings.clone() };
    let accepted = store.update_project_costing(project_id(), &write).await.expect("write the costing settings");
    assert_eq!(accepted.receipt.version, 2);
    assert_eq!(accepted.receipt.project_id, project_id());
    assert!(accepted.facts.is_empty());

    // The accepted write appended one immutable project record version; the earlier version stays
    // readable and shows the document before the settings existed.
    assert_eq!(store.project_record_version_count(project_id()).await.expect("count the versions"), 2);
    let current = store.project_record_view(project_id()).await.expect("the current project record");
    assert_eq!(current.version, 2);
    assert_eq!(current.document.costing, Some(settings.clone()));
    let first = store.project_record_version(project_id(), 1).await.expect("the first project record");
    assert_eq!(first.document.costing, None, "a document written before SB-16 reads as no settings, never an implicit default");

    // An identical retry returns the recorded outcome before the version check: it replays even
    // after a later write has moved the project on.
    let retried = store.update_project_costing(project_id(), &write).await.expect("retry the same request");
    assert_eq!(retried, accepted);
    assert_eq!(store.project_record_version_count(project_id()).await.expect("count the versions"), 2);

    // A second accepted write appends a version and leaves the accepted one exactly as it was.
    let second = store
        .update_project_costing(
            project_id(),
            &UpdateProjectCosting { request_id: "costing-2".into(), expected_version: 2, costing: ProjectCosting::default() },
        )
        .await
        .expect("write the empty settings");
    assert_eq!(second.receipt.version, 3);
    let retained = store.project_record_version(project_id(), 2).await.expect("the retained settings version");
    assert_eq!(retained.document.costing, Some(settings));

    // A stale expected version and a changed payload under a used request id are conflicts.
    let stale = UpdateProjectCosting { request_id: "costing-3".into(), expected_version: 2, costing: ProjectCosting::default() };
    assert!(matches!(store.update_project_costing(project_id(), &stale).await, Err(StoreError::Conflict(_))));
    let changed = UpdateProjectCosting { request_id: "costing-1".into(), expected_version: 1, costing: ProjectCosting::default() };
    assert!(matches!(store.update_project_costing(project_id(), &changed).await, Err(StoreError::Conflict(_))));

    // Settings are validated against the project currency and part versions before anything is
    // written: an identity exchange rate and a table for an unknown part version are refused.
    let identity_rate = ProjectCosting {
        volume_policy: None,
        exchange_rates: vec![ExchangeRate { source_currency: "CNY".into(), project_units_per_source_unit: support::d("1") }],
        purchase_prices: Vec::new(),
    };
    assert!(matches!(
        store
            .update_project_costing(
                project_id(),
                &UpdateProjectCosting { request_id: "costing-4".into(), expected_version: 3, costing: identity_rate }
            )
            .await,
        Err(StoreError::Invalid(_))
    ));
    let foreign_table = ProjectCosting {
        volume_policy: None,
        exchange_rates: Vec::new(),
        purchase_prices: vec![PurchasePriceTable {
            part_version_id: Uuid::new_v4(),
            currency: "CNY".into(),
            tiers: Vec::new(),
            source_reference: None,
        }],
    };
    assert!(matches!(
        store
            .update_project_costing(
                project_id(),
                &UpdateProjectCosting { request_id: "costing-5".into(), expected_version: 3, costing: foreign_table }
            )
            .await,
        Err(StoreError::Invalid(_))
    ));
    assert_eq!(store.project_record_version_count(project_id()).await.expect("count the versions"), 3, "a refused write appends nothing");
    db.cleanup().await;
}

/// One flat occurrence of one definition whose customer identifiers resolve the part version.
fn single_manifest(definition_seed: u128, number: &str, revision: &str, source_path: &[u64]) -> AnalysisManifest {
    let definition_id = Uuid::from_u128(definition_seed);
    let occurrence_id = Uuid::from_u128(definition_seed + 0x100);
    AnalysisManifest {
        schema_version: SCHEMA_VERSION,
        key: analysis_key(),
        definitions: vec![DefinitionRef {
            definition_id,
            source_key: analysis_key(),
            customer_part_number: Some(number.into()),
            customer_revision: Some(revision.into()),
            volume_mm3: measurement("2400"),
        }],
        occurrences: vec![solidbom_contract::OccurrenceRef {
            occurrence_id,
            definition_id,
            parent_occurrence_id: None,
            source_path: source_path.to_vec(),
            transform: IDENTITY,
        }],
        geometry_bindings: vec![GeometryBinding { source_path: source_path.to_vec(), source_body: 0, definition_id, occurrence_id }],
        issues: Vec::new(),
    }
}

#[tokio::test]
async fn a_missing_forecast_an_unsourced_part_and_an_unpriced_purchase_stay_incomplete_with_named_issues() {
    let db = TestDb::new().await;
    let store = &db.store;
    let source = store.put_source(&support::digest(SOURCE), "baseline.step", SOURCE).await.expect("store the source");
    let project = Uuid::from_u128(0xb171);
    store
        .create_project(&CreateProject {
            request_id: "create-1".into(),
            project_id: project,
            details: details(YearRange { sop_year: 2027, eop_year: 2028 }),
        })
        .await
        .expect("create the project");
    store
        .record_delivery(
            project,
            &RecordDelivery {
                request_id: "delivery-1".into(),
                expected_version: 1,
                package: DeliveryPackage { package_id: Uuid::from_u128(0xb172), name: "Package 1".into(), source },
                products: vec![
                    DeliveredProductInput {
                        product_id: None,
                        name: "incomplete".into(),
                        kind: ProductKind::Assembly,
                        customer_part_number: None,
                        parts: vec![delivered_part("unsourced", "U-1", "A")],
                    },
                    DeliveredProductInput {
                        product_id: None,
                        name: "priced".into(),
                        kind: ProductKind::Assembly,
                        customer_part_number: None,
                        parts: vec![delivered_part("purchased", "P-2", "A")],
                    },
                ],
            },
        )
        .await
        .expect("record the delivery");
    let view = store.project_record_view(project).await.expect("the project record view");
    let product = |name: &str| view.document.products.iter().find(|product| product.name == name).cloned().expect("the delivered product");
    let incomplete = product("incomplete");
    let priced = product("priced");
    let purchased_part = view.document.part_version_by_identity("P-2", "A").expect("the purchased part version").part_version_id;
    // The default schedule is 2027–2028 and only 2027 is forecast: 2028 is a named missing input,
    // never a zero year. The second product has a complete one-year schedule.
    store
        .update_product(
            project,
            &UpdateProduct {
                request_id: "product-1".into(),
                expected_version: 2,
                product_id: incomplete.product_id,
                name: "incomplete".into(),
                kind: ProductKind::Assembly,
                customer_part_number: None,
                schedule_override: None,
                forecasts: vec![AnnualForecast { year: 2027, quantity: 10 }],
            },
        )
        .await
        .expect("set the incomplete forecast");
    store
        .update_product(
            project,
            &UpdateProduct {
                request_id: "product-2".into(),
                expected_version: 3,
                product_id: priced.product_id,
                name: "priced".into(),
                kind: ProductKind::Assembly,
                customer_part_number: None,
                schedule_override: Some(YearRange { sop_year: 2027, eop_year: 2027 }),
                forecasts: vec![AnnualForecast { year: 2027, quantity: 10 }],
            },
        )
        .await
        .expect("set the complete forecast");
    let incomplete_revision = incomplete.revisions.first().expect("the delivered revision").product_revision_id;
    let priced_revision = priced.revisions.first().expect("the delivered revision").product_revision_id;
    store
        .register_analysis(
            project,
            &RegisterAnalysis {
                request_id: "analysis-1".into(),
                product_revision_id: incomplete_revision,
                manifest: single_manifest(0xdf01, "U-1", "A", &[]),
            },
        )
        .await
        .expect("register the incomplete analysis");
    store
        .register_analysis(
            project,
            &RegisterAnalysis {
                request_id: "analysis-2".into(),
                product_revision_id: priced_revision,
                manifest: single_manifest(0xdf02, "P-2", "A", &[1]),
            },
        )
        .await
        .expect("register the priced analysis");
    // Only the purchased part has a sourcing record; the other part version stays unsourced.
    store
        .set_sourcing(
            project,
            &SetSourcing {
                request_id: "sourcing-1".into(),
                expected_version: 0,
                parts: vec![purchased(purchased_part)],
                affected_part_versions: vec![purchased_part],
            },
            "engineer",
        )
        .await
        .expect("set the purchased sourcing");
    store
        .create_app_config_version(&support::app_config_version("config-1", 0, Uuid::from_u128(0xcf02), Vec::new()), "administrator")
        .await
        .expect("publish the app cost configuration");
    store
        .update_project_costing(
            project,
            &UpdateProjectCosting {
                request_id: "costing-1".into(),
                expected_version: 4,
                costing: ProjectCosting {
                    volume_policy: Some(CostingVolumePolicy::Average),
                    exchange_rates: Vec::new(),
                    purchase_prices: Vec::new(),
                },
            },
        )
        .await
        .expect("write the project costing settings");

    let request = CalculateBaseline {
        request_id: "baseline-1".into(),
        project_version: 5,
        engineering_version: None,
        manufacturing_version: None,
        product_revisions: vec![
            ProductRevisionSelection { product_id: incomplete.product_id, product_revision_id: incomplete_revision },
            ProductRevisionSelection { product_id: priced.product_id, product_revision_id: priced_revision },
        ],
    };
    let snapshot = store.calculate_baseline(project, &request).await.expect("calculate the incomplete baseline");
    assert_eq!(snapshot.outputs.state, solidbom_contract::ResultState::Incomplete);
    let codes: Vec<&str> = snapshot.outputs.issues.iter().map(|issue| issue.code.as_str()).collect();
    assert!(codes.contains(&"missing_forecast"), "a missing production year is a named issue: {codes:?}");
    assert!(codes.contains(&"missing_sourcing"), "an unsourced part version is a named issue: {codes:?}");
    assert!(codes.contains(&"missing_purchase_price"), "an uncovered purchase with no single price is a named issue: {codes:?}");
    // The structure itself resolved, so the incomplete answer is about its inputs, not about a
    // missing tree.
    assert_eq!(snapshot.inputs.products.len(), 2);
    assert_eq!(snapshot.inputs.products[0].occurrences.len(), 1);
    assert_eq!(snapshot.inputs.products[1].occurrences.len(), 1);
    assert!(snapshot.inputs.products.iter().all(|product| product.occurrences.iter().all(|occurrence| occurrence.part_version_id.is_some())));
    assert!(snapshot.inputs.part(purchased_part).and_then(|part| part.purchase.as_ref()).is_some_and(|purchase| purchase.single_price.is_none()));
    // No missing amount became zero: the priced product's 2027 demand is known, the incomplete
    // product's lifetime is not.
    let purchased_line = snapshot.outputs.parts.iter().find(|part| part.part_version_id == purchased_part).expect("the purchased part line");
    assert_eq!(purchased_line.annual_demand.first().and_then(|year| year.quantity), Some(10));
    assert_eq!(purchased_line.lifetime_demand, Some(10));
    assert_eq!(purchased_line.costing_volume, Some(support::d("10")));
    assert_eq!(purchased_line.unit_cost, None);
    assert_eq!(purchased_line.purchase_price, None);
    let unsourced_part =
        snapshot.inputs.parts.iter().find(|part| part.sourcing.is_none()).map(|part| part.part_version_id).expect("the unsourced part input");
    let unsourced_line = snapshot.outputs.parts.iter().find(|part| part.part_version_id == unsourced_part).expect("the unsourced part line");
    assert_eq!(unsourced_line.lifetime_demand, None, "a partial year set never yields a lifetime value");
    assert_eq!(unsourced_line.unit_cost, None);
    db.cleanup().await;
}

/// A registered analysis may hold an arbitrarily deep parent chain (the manifest has no depth
/// bound), so the service must refuse the calculation before its own recursion overflows the
/// stack. The rejection is a clean invalid-input error, never a crash.
#[tokio::test]
async fn a_deep_registered_analysis_is_refused_before_the_calculation_recurses() {
    let db = TestDb::new().await;
    let store = &db.store;
    let source = store.put_source(&support::digest(SOURCE), "deep.step", SOURCE).await.expect("store the source");
    let project = Uuid::from_u128(0xdee9);
    store
        .create_project(&CreateProject {
            request_id: "create-deep".into(),
            project_id: project,
            details: details(YearRange { sop_year: 2027, eop_year: 2027 }),
        })
        .await
        .expect("create the project");
    store
        .record_delivery(
            project,
            &RecordDelivery {
                request_id: "delivery-deep".into(),
                expected_version: 1,
                package: DeliveryPackage { package_id: Uuid::from_u128(0xdeea), name: "Package".into(), source },
                products: vec![DeliveredProductInput {
                    product_id: None,
                    name: "deep".into(),
                    kind: ProductKind::Assembly,
                    customer_part_number: None,
                    parts: vec![delivered_part("deep", "D-1", "A")],
                }],
            },
        )
        .await
        .expect("record the delivery");
    let view = store.project_record_view(project).await.expect("the project view");
    let product = view.document.products.first().expect("the delivered product");
    let revision_id = product.revisions.first().expect("the delivered revision").product_revision_id;
    store
        .update_product(
            project,
            &UpdateProduct {
                request_id: "product-deep".into(),
                expected_version: 2,
                product_id: product.product_id,
                name: "deep".into(),
                kind: ProductKind::Assembly,
                customer_part_number: None,
                schedule_override: Some(YearRange { sop_year: 2027, eop_year: 2027 }),
                forecasts: vec![AnnualForecast { year: 2027, quantity: 1 }],
            },
        )
        .await
        .expect("set the schedule");

    // One definition placed 80 times, each placement the parent of the next: deeper than the
    // supported structure limit, and valid as far as the registered manifest schema is concerned.
    let definition_id = Uuid::from_u128(0xdeeb);
    let mut occurrences = Vec::with_capacity(80);
    for index in 0..80u64 {
        occurrences.push(solidbom_contract::OccurrenceRef {
            occurrence_id: Uuid::from_u128(0xdec0 + u128::from(index)),
            definition_id,
            parent_occurrence_id: (index > 0).then(|| Uuid::from_u128(0xdec0 + u128::from(index - 1))),
            source_path: vec![index],
            transform: IDENTITY,
        });
    }
    let manifest = AnalysisManifest {
        schema_version: SCHEMA_VERSION,
        key: analysis_key(),
        definitions: vec![DefinitionRef {
            definition_id,
            source_key: analysis_key(),
            customer_part_number: None,
            customer_revision: None,
            volume_mm3: measurement("2400"),
        }],
        occurrences,
        geometry_bindings: Vec::new(),
        issues: Vec::new(),
    };
    store
        .register_analysis(project, &RegisterAnalysis { request_id: "analysis-deep".into(), product_revision_id: revision_id, manifest })
        .await
        .expect("the hostile manifest is accepted as evidence");

    let result = store
        .calculate_baseline(
            project,
            &CalculateBaseline {
                request_id: "baseline-deep".into(),
                project_version: 3,
                engineering_version: None,
                manufacturing_version: None,
                // The delivered revision must be selected explicitly: an unselected product is
                // reported as a missing selection and its structure is never read.
                product_revisions: vec![ProductRevisionSelection { product_id: product.product_id, product_revision_id: revision_id }],
            },
        )
        .await;
    assert!(matches!(result, Err(StoreError::Invalid(_))), "a too-deep structure is refused, not walked: {result:?}");
    db.cleanup().await;
}

/// Reviewer round 1 regression: two product revisions can register the same analysis key (the
/// stored identity includes the revision). Occurrence resolution must use the reviewed component of
/// the selected revision, never the first matching component of another revision that shares the
/// same source geometry.
#[tokio::test]
async fn occurrence_resolution_is_scoped_to_the_selected_revision() {
    let db = TestDb::new().await;
    let store = &db.store;
    let source = store.put_source(&support::digest(SOURCE), "shared-analysis.step", SOURCE).await.expect("store the source");
    let project = Uuid::from_u128(0x5e1e);
    store
        .create_project(&CreateProject {
            request_id: "create-shared".into(),
            project_id: project,
            details: details(YearRange { sop_year: 2027, eop_year: 2027 }),
        })
        .await
        .expect("create the project");
    store
        .record_delivery(
            project,
            &RecordDelivery {
                request_id: "delivery-shared".into(),
                expected_version: 1,
                package: DeliveryPackage { package_id: Uuid::from_u128(0x5e1f), name: "Package".into(), source },
                products: vec![
                    DeliveredProductInput {
                        product_id: None,
                        name: "revision one".into(),
                        kind: ProductKind::Assembly,
                        customer_part_number: None,
                        parts: vec![delivered_part("body", "R1-1", "A")],
                    },
                    DeliveredProductInput {
                        product_id: None,
                        name: "revision two".into(),
                        kind: ProductKind::Assembly,
                        customer_part_number: None,
                        parts: vec![delivered_part("body", "R2-1", "A")],
                    },
                ],
            },
        )
        .await
        .expect("record the delivery");
    let view = store.project_record_view(project).await.expect("the project view");
    let first = view.document.products.iter().find(|product| product.name == "revision one").cloned().expect("the first product");
    let second = view.document.products.iter().find(|product| product.name == "revision two").cloned().expect("the second product");
    let first_revision = first.revisions.first().expect("the first revision").product_revision_id;
    let second_revision = second.revisions.first().expect("the second revision").product_revision_id;
    let first_part = view.document.part_version_by_identity("R1-1", "A").expect("the first part version").part_version_id;
    let second_part = view.document.part_version_by_identity("R2-1", "A").expect("the second part version").part_version_id;

    for (request, product, expected, quantity) in [("product-shared-1", &first, 2, 10), ("product-shared-2", &second, 3, 20)] {
        store
            .update_product(
                project,
                &UpdateProduct {
                    request_id: request.into(),
                    expected_version: expected,
                    product_id: product.product_id,
                    name: product.name.clone(),
                    kind: ProductKind::Assembly,
                    customer_part_number: None,
                    schedule_override: Some(YearRange { sop_year: 2027, eop_year: 2027 }),
                    forecasts: vec![AnnualForecast { year: 2027, quantity }],
                },
            )
            .await
            .expect("set the schedule");
    }

    // The same exact analysis key is registered for both revisions (they share one delivery
    // package and source), and each revision groups the same source body with its own part version.
    store
        .register_analysis(
            project,
            &RegisterAnalysis { request_id: "analysis-shared-1".into(), product_revision_id: first_revision, manifest: manifest() },
        )
        .await
        .expect("register the first analysis");
    store
        .register_analysis(
            project,
            &RegisterAnalysis { request_id: "analysis-shared-2".into(), product_revision_id: second_revision, manifest: manifest() },
        )
        .await
        .expect("register the second analysis");
    store
        .group_solids(
            project,
            &GroupSolids {
                request_id: "group-shared-1".into(),
                expected_version: 0,
                component_id: Uuid::from_u128(0x5e20),
                product_revision_id: first_revision,
                analysis_key: analysis_key(),
                geometry: vec![ReviewedGeometry { source_path: Vec::new(), source_body: 0 }],
                part_version_id: Some(first_part),
            },
        )
        .await
        .expect("group the first body");
    store
        .group_solids(
            project,
            &GroupSolids {
                request_id: "group-shared-2".into(),
                expected_version: 1,
                component_id: Uuid::from_u128(0x5e21),
                product_revision_id: second_revision,
                analysis_key: analysis_key(),
                geometry: vec![ReviewedGeometry { source_path: Vec::new(), source_body: 0 }],
                part_version_id: Some(second_part),
            },
        )
        .await
        .expect("group the second body");

    let snapshot = store
        .calculate_baseline(
            project,
            &CalculateBaseline {
                request_id: "baseline-shared".into(),
                project_version: 4,
                engineering_version: None,
                manufacturing_version: None,
                product_revisions: vec![
                    ProductRevisionSelection { product_id: first.product_id, product_revision_id: first_revision },
                    ProductRevisionSelection { product_id: second.product_id, product_revision_id: second_revision },
                ],
            },
        )
        .await
        .expect("calculate both revisions");
    let resolved =
        |product_id: Uuid| snapshot.inputs.products.iter().find(|candidate| candidate.product_id == product_id).expect("the resolved product");
    let body_of = |product_id: Uuid| {
        resolved(product_id)
            .occurrences
            .iter()
            .find(|occurrence| occurrence.part_version_id.is_some())
            .expect("a resolved root occurrence")
            .part_version_id
    };
    assert_eq!(body_of(first.product_id), Some(first_part), "the first revision keeps its own association, never the second revision's");
    assert_eq!(body_of(second.product_id), Some(second_part), "the second revision keeps its own association, never the first revision's");
    db.cleanup().await;
}
