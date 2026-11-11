//! Store-level tests for the project tooling settings and the read-only tooling recovery
//! evaluation (contract version 1 of the SB-17 tooling contract, additive to API version 2).
//!
//! The demand of the evaluation is a real immutable SB-16 baseline snapshot: every fixture owns a
//! delivered product, a registered exact analysis, the quoted forecasts and, where a purchased
//! cutoff matters, the explicit sourced records. The baseline is calculated through
//! `Store::calculate_baseline`, so the evaluation reads exactly the records the service writes
//! rather than a hand-inserted row.
//!
//! `#[tokio::test]` is not recognised as a test function by Clippy's test allowance, so this file
//! allows the panicking helpers it needs. The library and binary still deny them.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

mod support;

use std::collections::BTreeMap;

use solidbom_contract::{
    AnalysisKey, AnalysisManifest, AnnualForecast, BaselineSnapshot, CalculateBaseline, CreateProject, DefinitionRef, DeliveredPartInput,
    DeliveredProductInput, DeliveryPackage, EvaluateToolingRecovery, GeometryBinding, Measurement, MeasurementSource, ProductKind,
    ProductRevisionSelection, ProjectDetails, ProjectTooling, ProjectWriteOutcome, RegisterAnalysis, ResultState, ReviewState, SCHEMA_VERSION,
    SetSourcing, SourcedPart, Sourcing, ToolingEntry, ToolingOwner, ToolingRecoveryBasis, ToolingRecoveryChoice, ToolingRecoveryInputs,
    ToolingTreatment, Unit, UpdateProduct, UpdateProjectTooling, YearRange,
};
use solidbom_server::{Store, StoreError};
use support::TestDb;
use uuid::Uuid;

const SOURCE: &[u8] = b"ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('tooling recovery fixture'),'2;1');\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n";
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

/// One delivered part definition and how often it occurs in the product structure. A part with a
/// parent is nested under that parent's first occurrence, which is how the purchased-branch cutoff
/// fixture builds a descendant.
struct PartSpec {
    number: &'static str,
    revision: &'static str,
    quantity: u64,
    parent: Option<&'static str>,
}

fn spec(number: &'static str, revision: &'static str, quantity: u64) -> PartSpec {
    PartSpec { number, revision, quantity, parent: None }
}

struct Fixture {
    project_id: Uuid,
    product_id: Uuid,
    revision_id: Uuid,
    parts: BTreeMap<&'static str, Uuid>,
}

/// The exact analysis of the product revision: every delivered definition carries the customer
/// identifiers that resolve its shared part version, so no reviewed component is needed for this
/// demand-only fixture.
fn manifest(parts: &[PartSpec]) -> AnalysisManifest {
    let mut definitions = Vec::with_capacity(parts.len());
    let mut occurrences = Vec::new();
    let mut bindings = Vec::new();
    let mut first_occurrence: BTreeMap<&'static str, Uuid> = BTreeMap::new();
    let mut counter: u128 = 0;
    for (index, part) in parts.iter().enumerate() {
        let definition_id = Uuid::from_u128(0xde00 + u128::try_from(index).expect("a fixture part index"));
        definitions.push(DefinitionRef {
            definition_id,
            source_key: analysis_key(),
            customer_part_number: Some(part.number.into()),
            customer_revision: Some(part.revision.into()),
            volume_mm3: measurement("2400"),
        });
        let parent_occurrence_id = part.parent.and_then(|parent| first_occurrence.get(parent).copied());
        for placement in 0..part.quantity {
            let occurrence_id = Uuid::from_u128(0x0c0000 + counter);
            let source_path = vec![u64::try_from(counter).expect("a fixture occurrence index")];
            counter += 1;
            if placement == 0 {
                first_occurrence.insert(part.number, occurrence_id);
            }
            occurrences.push(solidbom_contract::OccurrenceRef {
                occurrence_id,
                definition_id,
                parent_occurrence_id,
                source_path: source_path.clone(),
                transform: IDENTITY,
            });
            bindings.push(GeometryBinding { source_path, source_body: 0, definition_id, occurrence_id });
        }
    }
    AnalysisManifest {
        schema_version: SCHEMA_VERSION,
        key: analysis_key(),
        definitions,
        occurrences,
        geometry_bindings: bindings,
        issues: Vec::new(),
    }
}

/// Create a project with one quoted delivered product whose structure is `parts`, set its schedule
/// and forecasts, register the exact analysis and record the explicit purchased choices. The
/// project record is at version 3 when this returns: create, delivery and product edit.
async fn fixture(
    store: &Store,
    project_id: Uuid,
    schedule: YearRange,
    forecasts: Vec<AnnualForecast>,
    parts: &[PartSpec],
    purchased: &[&'static str],
) -> Fixture {
    let source = store.put_source(&support::digest(SOURCE), "tooling.step", SOURCE).await.expect("store the source");
    store
        .create_project(&CreateProject {
            request_id: "create-1".into(),
            project_id,
            details: ProjectDetails {
                name: "Tooling inquiry".into(),
                customer: "Customer".into(),
                internal_team: None,
                platform: None,
                application: None,
                currency: "CNY".into(),
                default_schedule: schedule,
            },
        })
        .await
        .expect("create the project");
    let delivered: Vec<DeliveredPartInput> = parts
        .iter()
        .map(|part| DeliveredPartInput {
            delivered_key: part.number.into(),
            customer_part_number: Some(part.number.into()),
            customer_revision: Some(part.revision.into()),
            volume_mm3: Some(support::d("2400")),
        })
        .collect();
    store
        .record_delivery(
            project_id,
            &solidbom_contract::RecordDelivery {
                request_id: "delivery-1".into(),
                expected_version: 1,
                package: DeliveryPackage { package_id: Uuid::new_v4(), name: "Package 1".into(), source },
                products: vec![DeliveredProductInput {
                    product_id: None,
                    name: "assembly".into(),
                    kind: ProductKind::Assembly,
                    customer_part_number: None,
                    parts: delivered,
                }],
            },
        )
        .await
        .expect("record the delivery");
    let view = store.project_record_view(project_id).await.expect("the project record view");
    let product = view.document.products.first().expect("the delivered product").clone();
    let revision_id = product.revisions.first().expect("the delivered revision").product_revision_id;
    let mut part_ids = BTreeMap::new();
    for part in parts {
        let part_version = view.document.part_version_by_identity(part.number, part.revision).expect("the delivered part version");
        part_ids.insert(part.number, part_version.part_version_id);
    }
    store
        .update_product(
            project_id,
            &UpdateProduct {
                request_id: "product-1".into(),
                expected_version: 2,
                product_id: product.product_id,
                name: "assembly".into(),
                kind: ProductKind::Assembly,
                customer_part_number: None,
                schedule_override: Some(schedule),
                forecasts,
            },
        )
        .await
        .expect("set the schedule and forecasts");
    store
        .register_analysis(
            project_id,
            &RegisterAnalysis { request_id: "analysis-1".into(), product_revision_id: revision_id, manifest: manifest(parts) },
        )
        .await
        .expect("register the exact analysis");
    if !purchased.is_empty() {
        let sourced: Vec<SourcedPart> = purchased
            .iter()
            .map(|number| SourcedPart {
                part_version_id: *part_ids.get(number).expect("the purchased part version"),
                sourcing: Sourcing::Purchased,
                material: None,
                source_reference: None,
            })
            .collect();
        let affected: Vec<Uuid> = sourced.iter().map(|part| part.part_version_id).collect();
        store
            .set_sourcing(
                project_id,
                &SetSourcing { request_id: "sourcing-1".into(), expected_version: 0, parts: sourced, affected_part_versions: affected },
                "engineer",
            )
            .await
            .expect("set the explicit sourcing");
    }
    Fixture { project_id, product_id: product.product_id, revision_id, parts: part_ids }
}

fn entry(tool: u128, owner: ToolingOwner, cost: &str) -> ToolingEntry {
    ToolingEntry {
        tool_id: Uuid::from_u128(tool),
        owner,
        name: None,
        tool_count: 1,
        cost_per_tool: support::d(cost),
        currency: "CNY".into(),
        source_reference: None,
    }
}

/// Write the tooling settings at the project's current version and return both the outcome and the
/// immutable baseline snapshot that records exactly that project version.
async fn write_settings_and_baseline(store: &Store, fixture: &Fixture, settings: ProjectTooling) -> (ProjectWriteOutcome, BaselineSnapshot) {
    let current = store.project_record_view(fixture.project_id).await.expect("the current project record").version;
    let written = store
        .update_project_tooling(
            fixture.project_id,
            &UpdateProjectTooling { request_id: "tooling-1".into(), expected_version: current, tooling: settings },
        )
        .await
        .expect("write the tooling settings");
    let baseline = store
        .calculate_baseline(
            fixture.project_id,
            &CalculateBaseline {
                request_id: "baseline-1".into(),
                project_version: written.receipt.version,
                engineering_version: None,
                manufacturing_version: None,
                product_revisions: vec![ProductRevisionSelection { product_id: fixture.product_id, product_revision_id: fixture.revision_id }],
            },
        )
        .await
        .expect("calculate the baseline the evaluation reads");
    (written, baseline)
}

async fn evaluate(
    store: &Store,
    project_id: Uuid,
    baseline: &BaselineSnapshot,
    recovery: ToolingRecoveryInputs,
) -> solidbom_contract::ToolingOutcome {
    store
        .evaluate_tooling_recovery(project_id, &EvaluateToolingRecovery { baseline_calculation_id: baseline.calculation_id, recovery })
        .await
        .expect("evaluate the tooling recovery")
}

fn manual(tool_year: &[ToolingRecoveryChoice]) -> ToolingRecoveryInputs {
    ToolingRecoveryInputs { treatment: Some(ToolingTreatment::Amortized), recovery: tool_year.to_vec() }
}

fn manual_choice(tool: u128, quantity: &str) -> ToolingRecoveryChoice {
    ToolingRecoveryChoice { tool_id: Uuid::from_u128(tool), basis: ToolingRecoveryBasis::ManualQuantity { quantity: Some(support::d(quantity)) } }
}

#[tokio::test]
async fn tooling_settings_are_append_only_idempotent_and_validated_against_the_project() {
    let db = TestDb::new().await;
    let store = &db.store;
    let project_id = Uuid::from_u128(0xb201);
    let fixture = fixture(store, project_id, YearRange { sop_year: 2027, eop_year: 2027 }, Vec::new(), &[spec("M-1", "A", 1)], &[]).await;
    let made = fixture.parts["M-1"];
    assert_eq!(store.project_record_version_count(project_id).await.expect("count the versions"), 3);

    let settings = ProjectTooling {
        default_treatment: Some(ToolingTreatment::Separate),
        entries: vec![entry(0x7a01, ToolingOwner::PartVersion { part_version_id: made }, "10000")],
    };
    let write = UpdateProjectTooling { request_id: "tooling-1".into(), expected_version: 3, tooling: settings.clone() };
    let accepted = store.update_project_tooling(project_id, &write).await.expect("write the tooling settings");
    assert_eq!(accepted.receipt.version, 4);
    assert_eq!(accepted.receipt.project_id, project_id);
    assert!(accepted.facts.is_empty());

    // The accepted write appended one immutable project record version; the earlier version stays
    // readable and shows the document before the tooling existed.
    assert_eq!(store.project_record_version_count(project_id).await.expect("count the versions"), 4);
    let current = store.project_record_view(project_id).await.expect("the current project record");
    assert_eq!(current.version, 4);
    assert_eq!(current.document.tooling, Some(settings.clone()));
    let before = store.project_record_version(project_id, 3).await.expect("the version before the settings");
    assert_eq!(before.document.tooling, None, "a document written before SB-17 reads as no settings, never an implicit treatment");

    // An identical retry returns the recorded outcome before the version check, even after a later
    // write has moved the project on.
    let retried = store.update_project_tooling(project_id, &write).await.expect("retry the same request");
    assert_eq!(retried, accepted);
    assert_eq!(store.project_record_version_count(project_id).await.expect("count the versions"), 4);

    // A second accepted write appends a version and leaves the accepted one exactly as it was.
    let second = ProjectTooling { default_treatment: Some(ToolingTreatment::Amortized), entries: Vec::new() };
    let rewritten = store
        .update_project_tooling(project_id, &UpdateProjectTooling { request_id: "tooling-2".into(), expected_version: 4, tooling: second.clone() })
        .await
        .expect("replace the settings");
    assert_eq!(rewritten.receipt.version, 5);
    let retained = store.project_record_version(project_id, 4).await.expect("the retained settings version");
    assert_eq!(retained.document.tooling, Some(settings));

    // A stale expected version and a changed payload under a used request id are conflicts.
    let stale = UpdateProjectTooling { request_id: "tooling-3".into(), expected_version: 3, tooling: second.clone() };
    assert!(matches!(store.update_project_tooling(project_id, &stale).await, Err(StoreError::Conflict(_))));
    let changed = UpdateProjectTooling {
        request_id: "tooling-1".into(),
        expected_version: 3,
        tooling: ProjectTooling { default_treatment: None, entries: Vec::new() },
    };
    assert!(matches!(store.update_project_tooling(project_id, &changed).await, Err(StoreError::Conflict(_))));

    // The settings are validated against the project currency, its part versions and its products
    // before anything is written.
    let unknown_part = ProjectTooling {
        default_treatment: None,
        entries: vec![entry(0x7a02, ToolingOwner::PartVersion { part_version_id: Uuid::new_v4() }, "10000")],
    };
    assert!(matches!(
        store
            .update_project_tooling(project_id, &UpdateProjectTooling { request_id: "tooling-4".into(), expected_version: 5, tooling: unknown_part })
            .await,
        Err(StoreError::Invalid(_))
    ));
    let unknown_product =
        ProjectTooling { default_treatment: None, entries: vec![entry(0x7a03, ToolingOwner::Product { product_id: Uuid::new_v4() }, "10000")] };
    assert!(matches!(
        store
            .update_project_tooling(
                project_id,
                &UpdateProjectTooling { request_id: "tooling-5".into(), expected_version: 5, tooling: unknown_product }
            )
            .await,
        Err(StoreError::Invalid(_))
    ));
    let mut lowercase_currency = entry(0x7a04, ToolingOwner::PartVersion { part_version_id: made }, "10000");
    lowercase_currency.currency = "cny".into();
    assert!(matches!(
        store
            .update_project_tooling(
                project_id,
                &UpdateProjectTooling {
                    request_id: "tooling-6".into(),
                    expected_version: 5,
                    tooling: ProjectTooling { default_treatment: None, entries: vec![lowercase_currency] },
                }
            )
            .await,
        Err(StoreError::Invalid(_))
    ));
    let mut zero_count = entry(0x7a05, ToolingOwner::PartVersion { part_version_id: made }, "10000");
    zero_count.tool_count = 0;
    assert!(matches!(
        store
            .update_project_tooling(
                project_id,
                &UpdateProjectTooling {
                    request_id: "tooling-7".into(),
                    expected_version: 5,
                    tooling: ProjectTooling { default_treatment: None, entries: vec![zero_count] },
                }
            )
            .await,
        Err(StoreError::Invalid(_))
    ));
    // A product-root owner that exists is accepted, so the owner check is a real project check and
    // not a blanket refusal.
    let product_tool = ProjectTooling {
        default_treatment: Some(ToolingTreatment::Separate),
        entries: vec![entry(0x7a06, ToolingOwner::Product { product_id: fixture.product_id }, "10000")],
    };
    let product_owned = store
        .update_project_tooling(project_id, &UpdateProjectTooling { request_id: "tooling-8".into(), expected_version: 5, tooling: product_tool })
        .await
        .expect("a product-root owner that the project holds");
    assert_eq!(product_owned.receipt.version, 6);
    assert_eq!(store.project_record_version_count(project_id).await.expect("count the versions"), 6, "a refused write appends nothing");
    db.cleanup().await;
}

#[tokio::test]
async fn a_recovery_evaluation_counts_investment_once_and_never_recovers_it_twice() {
    let db = TestDb::new().await;
    let store = &db.store;
    let project_id = Uuid::from_u128(0xb211);
    let fixture = fixture(
        store,
        project_id,
        YearRange { sop_year: 2027, eop_year: 2027 },
        vec![AnnualForecast { year: 2027, quantity: 1000 }],
        &[spec("M-1", "A", 1)],
        &[],
    )
    .await;
    let made = fixture.parts["M-1"];
    // Two entries of 10,000: one owned by the part version, one by the finished product. Both are
    // demanded once per finished product, and neither occurrence nor demand may multiply them.
    let settings = ProjectTooling {
        default_treatment: Some(ToolingTreatment::Amortized),
        entries: vec![
            entry(0x7a11, ToolingOwner::PartVersion { part_version_id: made }, "10000"),
            entry(0x7a12, ToolingOwner::Product { product_id: fixture.product_id }, "10000"),
        ],
    };
    let (_written, baseline) = write_settings_and_baseline(store, &fixture, settings).await;
    assert_eq!(baseline.outputs.products.first().expect("the product line").lifetime_demand, Some(1000));
    assert_eq!(baseline.outputs.products.first().expect("the product line").parts.len(), 1);

    let recovery = manual(&[manual_choice(0x7a11, "5000"), manual_choice(0x7a12, "5000")]);
    let outcome = evaluate(store, project_id, &baseline, recovery.clone()).await;
    assert_eq!(outcome.state, ResultState::Complete, "{:?}", outcome.issues);
    assert_eq!(outcome.treatment, Some(ToolingTreatment::Amortized));
    assert_eq!(outcome.project_investment, Some(support::d("20000")), "the project counts both 10,000 investments exactly once");
    assert_eq!(outcome.tools.len(), 2);
    for tool in &outcome.tools {
        assert_eq!(tool.investment, Some(support::d("10000")));
        assert_eq!(tool.recovery_quantity, Some(support::d("5000")));
        assert_eq!(tool.contribution_per_owner_unit, Some(support::d("2")), "10,000 / 5,000 = 2 per owner unit");
        assert_eq!(tool.projected_recovery, Some(support::d("2000")), "2 per owner unit × 1,000 owner units");
        assert_eq!(tool.under_over_recovery, Some(support::d("-8000")));
        assert_eq!(tool.separate_charge, None);
        assert_eq!(tool.allocation.len(), 1);
        assert_eq!(tool.allocation[0].product_id, fixture.product_id);
        assert_eq!(tool.allocation[0].expanded_quantity, 1);
        assert_eq!(tool.allocation[0].product_owner_demand, Some(1000));
        assert_eq!(tool.allocation[0].allocation_share, Some(support::d("1")));
        assert_eq!(tool.allocation[0].allocated_investment, Some(support::d("10000")));
        assert_eq!(tool.allocation[0].product_contribution, Some(support::d("2")));
        assert_eq!(tool.annual_recovery.len(), 1);
        assert_eq!(tool.annual_recovery[0].year, 2027);
        assert_eq!(tool.annual_recovery[0].recovered, Some(support::d("2000")));
    }
    assert_eq!(outcome.products.len(), 1);
    assert_eq!(outcome.products[0].allocated_investment, Some(support::d("20000")));
    assert_eq!(outcome.products[0].amortized_contribution, Some(support::d("4")), "two tools × 2 per finished product");
    assert_eq!(outcome.project_amortized_recovered, Some(support::d("4000")));
    assert_eq!(outcome.project_under_over_recovery, Some(support::d("-16000")));
    assert_eq!(outcome.project_separate_charge, None);
    assert_eq!(outcome.annual_recovery.len(), 1);
    assert_eq!(outcome.annual_recovery[0].recovered, Some(support::d("4000")));
    assert!(outcome.issues.is_empty(), "{:?}", outcome.issues);

    // Switching to the separate treatment preserves the investment exactly and populates only the
    // separate charge: the same investment is never recovered through both channels.
    let separate = evaluate(
        store,
        project_id,
        &baseline,
        ToolingRecoveryInputs { treatment: Some(ToolingTreatment::Separate), recovery: recovery.recovery.clone() },
    )
    .await;
    assert_eq!(separate.state, ResultState::Complete, "{:?}", separate.issues);
    assert_eq!(separate.treatment, Some(ToolingTreatment::Separate));
    assert_eq!(separate.project_investment, Some(support::d("20000")));
    assert_eq!(separate.project_separate_charge, Some(support::d("20000")));
    assert_eq!(separate.project_amortized_recovered, None);
    assert_eq!(separate.project_under_over_recovery, None);
    assert!(separate.annual_recovery.is_empty());
    for tool in &separate.tools {
        assert_eq!(tool.investment, Some(support::d("10000")));
        assert_eq!(tool.separate_charge, Some(support::d("10000")));
        assert_eq!(tool.contribution_per_owner_unit, None);
        assert_eq!(tool.projected_recovery, None);
        assert_eq!(tool.under_over_recovery, None);
        assert!(tool.annual_recovery.is_empty());
        assert_eq!(tool.allocation[0].allocated_investment, Some(support::d("10000")), "the allocation still explains the share of the charge");
    }
    assert_eq!(separate.products[0].allocated_investment, Some(support::d("20000")));
    assert_eq!(separate.products[0].amortized_contribution, None, "the amortized contribution does not apply to the separate treatment");
    db.cleanup().await;
}

#[tokio::test]
async fn an_assembly_quantity_and_the_stored_forecasts_drive_the_contribution_and_the_under_over() {
    let db = TestDb::new().await;
    let store = &db.store;
    let project_id = Uuid::from_u128(0xb221);
    // The assembly occurs twice per finished product and is forecast 1,000 units in each of the four
    // production years: 4,000 finished products, 8,000 owner units.
    let fixture = fixture(
        store,
        project_id,
        YearRange { sop_year: 2027, eop_year: 2030 },
        vec![
            AnnualForecast { year: 2027, quantity: 1000 },
            AnnualForecast { year: 2028, quantity: 1000 },
            AnnualForecast { year: 2029, quantity: 1000 },
            AnnualForecast { year: 2030, quantity: 1000 },
        ],
        &[spec("A-1", "A", 2)],
        &[],
    )
    .await;
    let assembly = fixture.parts["A-1"];
    let settings = ProjectTooling {
        default_treatment: Some(ToolingTreatment::Amortized),
        entries: vec![entry(0x7a21, ToolingOwner::PartVersion { part_version_id: assembly }, "10000")],
    };
    let (_written, baseline) = write_settings_and_baseline(store, &fixture, settings).await;
    let product_line = baseline.outputs.products.first().expect("the product line");
    assert_eq!(product_line.lifetime_demand, Some(4000));
    assert_eq!(product_line.parts.len(), 1);
    assert_eq!(product_line.parts[0].quantity, 2, "the expanded assembly quantity the baseline resolved");

    // A 10,000 investment with a manual recovery quantity of 5,000 contributes 2 per owner unit; the
    // assembly occurring twice contributes 4 per finished product.
    let outcome = evaluate(store, project_id, &baseline, manual(&[manual_choice(0x7a21, "5000")])).await;
    assert_eq!(outcome.state, ResultState::Complete, "{:?}", outcome.issues);
    assert_eq!(outcome.project_investment, Some(support::d("10000")));
    assert_eq!(outcome.tools.len(), 1);
    let tool = &outcome.tools[0];
    assert_eq!(tool.contribution_per_owner_unit, Some(support::d("2")));
    assert_eq!(tool.recovery_quantity, Some(support::d("5000")));
    assert_eq!(tool.allocation[0].expanded_quantity, 2);
    assert_eq!(tool.allocation[0].product_owner_demand, Some(8000));
    assert_eq!(tool.allocation[0].product_contribution, Some(support::d("4")));
    assert_eq!(tool.projected_recovery, Some(support::d("16000")), "2 per owner unit × 8,000 owner units");
    assert_eq!(tool.under_over_recovery, Some(support::d("6000")), "the projection recovers 6,000 more than the investment");
    assert_eq!(outcome.products[0].amortized_contribution, Some(support::d("4")));
    assert_eq!(outcome.project_amortized_recovered, Some(support::d("16000")));
    assert_eq!(outcome.project_under_over_recovery, Some(support::d("6000")));
    assert_eq!(outcome.annual_recovery.len(), 4);
    for year in &outcome.annual_recovery {
        assert_eq!(year.recovered, Some(support::d("4000")), "2 per owner unit × 1,000 products × 2 occurrences");
    }

    // A manual quantity of 10,000 recovers the investment exactly over the forecast lifetime; the
    // contribution stays at its initial value and the annual series never stops or caps.
    let exact = evaluate(store, project_id, &baseline, manual(&[manual_choice(0x7a21, "10000")])).await;
    assert_eq!(exact.tools[0].contribution_per_owner_unit, Some(support::d("1")));
    assert_eq!(exact.tools[0].projected_recovery, Some(support::d("8000")));
    assert_eq!(exact.tools[0].under_over_recovery, Some(support::d("-2000")));
    assert_eq!(
        exact.project_under_over_recovery,
        Some(support::d("-2000")),
        "using the lifetime basis would recover exactly; a manual 10,000 under-recovers"
    );

    // The lifetime basis equals the owner's combined lifetime demand, so the projection recovers the
    // investment exactly and the annual series uses the stored forecasts.
    let lifetime =
        evaluate(store, project_id, &baseline, ToolingRecoveryInputs { treatment: Some(ToolingTreatment::Amortized), recovery: Vec::new() }).await;
    assert_eq!(lifetime.tools[0].recovery_quantity, Some(support::d("8000")));
    assert_eq!(lifetime.tools[0].contribution_per_owner_unit, Some(support::d("1.25")));
    assert_eq!(lifetime.tools[0].projected_recovery, Some(support::d("10000")));
    assert_eq!(lifetime.tools[0].under_over_recovery, Some(support::d("0")));
    assert_eq!(lifetime.annual_recovery.len(), 4);
    assert_eq!(lifetime.annual_recovery[0].recovered, Some(support::d("2500")));
    db.cleanup().await;
}

#[tokio::test]
async fn a_tool_only_under_a_purchased_branch_has_zero_combined_demand_and_an_incomplete_result() {
    let db = TestDb::new().await;
    let store = &db.store;
    let project_id = Uuid::from_u128(0xb231);
    let mut descendant = spec("C-1", "A", 1);
    descendant.parent = Some("P-1");
    let fixture = fixture(
        store,
        project_id,
        YearRange { sop_year: 2027, eop_year: 2027 },
        vec![AnnualForecast { year: 2027, quantity: 1000 }],
        &[spec("P-1", "A", 1), descendant],
        &["P-1"],
    )
    .await;
    let child = fixture.parts["C-1"];
    let settings = ProjectTooling {
        default_treatment: Some(ToolingTreatment::Amortized),
        entries: vec![entry(0x7a31, ToolingOwner::PartVersion { part_version_id: child }, "10000")],
    };
    let (_written, baseline) = write_settings_and_baseline(store, &fixture, settings).await;
    // The purchased cutoffs are already applied in the immutable baseline: the purchased part itself
    // still carries its own demand, its descendant does not appear at all.
    let product_line = baseline.outputs.products.first().expect("the product line");
    assert_eq!(product_line.parts.len(), 1, "{:?}", product_line.parts);
    assert_eq!(product_line.parts[0].part_version_id, fixture.parts["P-1"]);
    assert_eq!(product_line.parts[0].quantity, 1);

    let outcome =
        evaluate(store, project_id, &baseline, ToolingRecoveryInputs { treatment: Some(ToolingTreatment::Amortized), recovery: Vec::new() }).await;
    assert_eq!(outcome.state, ResultState::Incomplete);
    assert_eq!(outcome.project_investment, Some(support::d("10000")), "the investment is counted once even without demand");
    assert_eq!(outcome.tools.len(), 1);
    let tool = &outcome.tools[0];
    assert_eq!(tool.tool_id, Uuid::from_u128(0x7a31));
    assert_eq!(tool.investment, Some(support::d("10000")));
    assert_eq!(tool.recovery_quantity, None, "no recovery quantity is invented from a zero combined demand");
    assert_eq!(tool.contribution_per_owner_unit, None);
    assert_eq!(tool.projected_recovery, None);
    let zero_demand = outcome.issues.iter().find(|issue| issue.code == "zero_combined_demand").expect("the zero combined demand issue");
    assert_eq!(zero_demand.tool_id, Some(Uuid::from_u128(0x7a31)));
    db.cleanup().await;
}

#[tokio::test]
async fn the_recovery_evaluation_is_read_only_and_refuses_unknown_inputs() {
    let db = TestDb::new().await;
    let store = &db.store;
    let project_id = Uuid::from_u128(0xb241);
    let fixture = fixture(
        store,
        project_id,
        YearRange { sop_year: 2027, eop_year: 2027 },
        vec![AnnualForecast { year: 2027, quantity: 1000 }],
        &[spec("M-1", "A", 1)],
        &[],
    )
    .await;
    let made = fixture.parts["M-1"];
    let settings = ProjectTooling {
        default_treatment: Some(ToolingTreatment::Amortized),
        entries: vec![entry(0x7a41, ToolingOwner::PartVersion { part_version_id: made }, "10000")],
    };
    let (_written, baseline) = write_settings_and_baseline(store, &fixture, settings).await;
    let versions = store.project_record_version_count(project_id).await.expect("count the versions");
    let snapshots = store.baseline_calculations(project_id).await.expect("the baseline history").calculations.len();

    let recovery = manual(&[manual_choice(0x7a41, "5000")]);
    let first = evaluate(store, project_id, &baseline, recovery.clone()).await;
    let second = evaluate(store, project_id, &baseline, recovery.clone()).await;
    assert_eq!(first, second, "the read-only evaluation is deterministic");
    assert_eq!(store.project_record_version_count(project_id).await.expect("count the versions"), versions, "no project version is appended");
    assert_eq!(store.baseline_calculations(project_id).await.expect("the baseline history").calculations.len(), snapshots, "no snapshot is stored");

    // A recovery choice for a tool the project no longer holds is ignored, not an error.
    let mut with_stale_choice = recovery.clone();
    with_stale_choice.recovery.push(manual_choice(0x7a99, "1"));
    assert_eq!(evaluate(store, project_id, &baseline, with_stale_choice).await, first);

    // Unknown snapshots, another project's snapshot and an unknown project are all not found.
    let unknown = EvaluateToolingRecovery { baseline_calculation_id: Uuid::new_v4(), recovery: recovery.clone() };
    assert!(matches!(store.evaluate_tooling_recovery(project_id, &unknown).await, Err(StoreError::NotFound)));
    assert!(matches!(
        store
            .evaluate_tooling_recovery(
                Uuid::new_v4(),
                &EvaluateToolingRecovery { baseline_calculation_id: baseline.calculation_id, recovery: recovery.clone() }
            )
            .await,
        Err(StoreError::NotFound)
    ));
    let other_project = Uuid::from_u128(0xb249);
    store
        .create_project(&CreateProject {
            request_id: "create-2".into(),
            project_id: other_project,
            details: ProjectDetails {
                name: "Other inquiry".into(),
                customer: "Customer".into(),
                internal_team: None,
                platform: None,
                application: None,
                currency: "CNY".into(),
                default_schedule: YearRange { sop_year: 2027, eop_year: 2027 },
            },
        })
        .await
        .expect("create the other project");
    assert!(matches!(
        store
            .evaluate_tooling_recovery(
                other_project,
                &EvaluateToolingRecovery { baseline_calculation_id: baseline.calculation_id, recovery: recovery.clone() }
            )
            .await,
        Err(StoreError::NotFound)
    ));

    // A malformed request envelope or a malformed recovery assumption is refused before the read.
    let nil = EvaluateToolingRecovery { baseline_calculation_id: Uuid::nil(), recovery: recovery.clone() };
    assert!(matches!(store.evaluate_tooling_recovery(project_id, &nil).await, Err(StoreError::Invalid(_))));
    let non_positive = manual(&[manual_choice(0x7a41, "0")]);
    let invalid = EvaluateToolingRecovery { baseline_calculation_id: baseline.calculation_id, recovery: non_positive };
    assert!(matches!(store.evaluate_tooling_recovery(project_id, &invalid).await, Err(StoreError::Invalid(_))));
    db.cleanup().await;
}

#[tokio::test]
async fn a_manual_recovery_quantity_below_the_lifetime_demand_under_recovers_the_investment() {
    let db = TestDb::new().await;
    let store = &db.store;
    let project_id = Uuid::from_u128(0xb251);
    // One component per finished product, forecast 1,000 units in each of four years: the owner's
    // combined lifetime demand is 4,000. A 10,000 tool with a manual recovery quantity of 5,000
    // contributes 2 per owner unit, projects 8,000 and under-recovers by 2,000.
    let fixture = fixture(
        store,
        project_id,
        YearRange { sop_year: 2027, eop_year: 2030 },
        vec![
            AnnualForecast { year: 2027, quantity: 1000 },
            AnnualForecast { year: 2028, quantity: 1000 },
            AnnualForecast { year: 2029, quantity: 1000 },
            AnnualForecast { year: 2030, quantity: 1000 },
        ],
        &[spec("M-1", "A", 1)],
        &[],
    )
    .await;
    let made = fixture.parts["M-1"];
    assert_eq!(fixture.parts.len(), 1);
    let settings = ProjectTooling {
        default_treatment: Some(ToolingTreatment::Amortized),
        entries: vec![entry(0x7a51, ToolingOwner::PartVersion { part_version_id: made }, "10000")],
    };
    let (_written, baseline) = write_settings_and_baseline(store, &fixture, settings).await;
    assert_eq!(baseline.outputs.products.first().expect("the product line").lifetime_demand, Some(4000));

    let outcome = evaluate(store, project_id, &baseline, manual(&[manual_choice(0x7a51, "5000")])).await;
    assert_eq!(outcome.state, ResultState::Complete, "{:?}", outcome.issues);
    assert_eq!(outcome.project_investment, Some(support::d("10000")));
    let tool = &outcome.tools[0];
    assert_eq!(tool.recovery_quantity, Some(support::d("5000")));
    assert_eq!(tool.contribution_per_owner_unit, Some(support::d("2")));
    assert_eq!(tool.projected_recovery, Some(support::d("8000")), "2 per owner unit × the 4,000 combined lifetime demand");
    assert_eq!(tool.under_over_recovery, Some(support::d("-2000")), "the projection recovers 2,000 less than the investment");
    assert_eq!(outcome.products[0].amortized_contribution, Some(support::d("2")));
    assert_eq!(outcome.project_amortized_recovered, Some(support::d("8000")));
    assert_eq!(outcome.project_under_over_recovery, Some(support::d("-2000")));
    assert_eq!(outcome.project_separate_charge, None);
    assert_eq!(outcome.annual_recovery.len(), 4);
    for year in &outcome.annual_recovery {
        assert_eq!(year.recovered, Some(support::d("2000")), "2 per owner unit × 1,000 products in every production year");
    }
    db.cleanup().await;
}
