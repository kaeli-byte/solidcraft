//! Store-level tests for feasibility rules and justified exceptions (contract version 1, package
//! SB-14).
//!
//! `#[tokio::test]` is not recognised as a test function by Clippy's test allowance, so this file
//! allows the panicking helpers it needs. The library and binary still deny them.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

mod support;

use solidbom_contract::{
    AcceptFeasibilityException, AnalysisKey, AnalysisManifest, CloseFeasibilityException, CreateFeasibilityRuleTableVersion, CreateProject,
    DefinitionRef, DeliveredPartInput, DeliveredProductInput, DeliveryPackage, ExceptionState, FeasibilityAssessmentOutcome,
    FeasibilityComponentEvidence, FeasibilityConnector, FeasibilityFact, FeasibilityRecordView, FeasibilityRule, FeasibilityRuleKind,
    FeasibilityRuleTablePublished, FeasibilityTubeAttributes, FeasibilityTubeEnd, FeasibilityTubeMeasurements, FindingCheck, FindingDisposition,
    FindingState, GeometryBinding, Issue, Measurement, MeasurementSource, NumericRangeMm, OccurrenceRef, ProductKind, ProjectDetails, RecordDelivery,
    RecordFeasibilityAssessment, RegisterAnalysis, ReviewState, ReviewedGeometry, RuleApplicability, RuleLimit, SCHEMA_VERSION, TubeEnd, TubeKind,
    Unit, YearRange,
};
use solidbom_server::{Store, StoreError};
use support::TestDb;
use uuid::Uuid;

const SOURCE: &[u8] = b"ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('feasibility fixture'),'2;1');\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n";
const IDENTITY: [[f64; 4]; 4] = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
const DEFINITION: u128 = 0xdef1;
const OCCURRENCE: u128 = 0x0cc0;
const COMPONENT: u128 = 0xc001;
const CONNECTOR: u128 = 0xc0;

fn project_id() -> Uuid {
    Uuid::from_u128(0xf140)
}

fn details() -> ProjectDetails {
    ProjectDetails {
        name: "Feasibility inquiry".into(),
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

/// The flat fixture's exact analysis: one definition with three bodies, which the engineer reviews
/// as components.
fn flat_manifest() -> AnalysisManifest {
    AnalysisManifest {
        schema_version: SCHEMA_VERSION,
        key: key(),
        definitions: vec![DefinitionRef {
            definition_id: Uuid::from_u128(DEFINITION),
            source_key: key(),
            customer_part_number: Some("PL-100".into()),
            customer_revision: Some("B".into()),
            volume_mm3: Measurement {
                value: None,
                unit: Unit::MillimetreCubed,
                source: MeasurementSource::Extracted,
                review: ReviewState::Pending,
                reason: None,
                analysis_key: key(),
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

fn delivered_product() -> DeliveredProductInput {
    DeliveredProductInput {
        product_id: None,
        name: "A".into(),
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

/// Create the project, deliver the flat product, register the exact analysis and review one
/// component containing body 0. Returns the project revision and part version.
async fn setup(store: &Store) -> (Uuid, Uuid) {
    store
        .create_project(&CreateProject { request_id: "create-1".into(), project_id: project_id(), details: details() })
        .await
        .expect("create the project");
    let source = store.put_source(&support::digest(SOURCE), "feasibility.step", SOURCE).await.expect("store the source");
    store
        .record_delivery(
            project_id(),
            &RecordDelivery {
                request_id: "delivery-1".into(),
                expected_version: 1,
                package: DeliveryPackage { package_id: Uuid::from_u128(0xf141), name: "Package 1".into(), source },
                products: vec![delivered_product()],
            },
        )
        .await
        .expect("record the delivery");
    let view = store.project_record_view(project_id()).await.expect("the project record view");
    let revision = view.document.products[0].revisions[0].product_revision_id;
    let part_version_id = view.document.part_versions[0].part_version_id;
    store
        .register_analysis(
            project_id(),
            &RegisterAnalysis { request_id: "analysis-1".into(), product_revision_id: revision, manifest: flat_manifest() },
        )
        .await
        .expect("register the exact analysis");
    store
        .group_solids(
            project_id(),
            &solidbom_contract::GroupSolids {
                request_id: "group-1".into(),
                expected_version: 0,
                component_id: Uuid::from_u128(COMPONENT),
                product_revision_id: revision,
                analysis_key: key(),
                geometry: vec![body(0)],
                part_version_id: Some(part_version_id),
            },
        )
        .await
        .expect("review one component");
    (revision, part_version_id)
}

fn rule(id: u128, kind: FeasibilityRuleKind, applicability: RuleApplicability, limit: RuleLimit) -> FeasibilityRule {
    FeasibilityRule { rule_id: Uuid::from_u128(id), kind, applicability, limit, source_reference: "ENG06517, 2010-11-05".into(), note: None }
}

fn radius_rule() -> FeasibilityRule {
    rule(
        1,
        FeasibilityRuleKind::MinimumBendRadius,
        RuleApplicability {
            tube_kind: Some(TubeKind::Smooth),
            material_name: Some("PA12".into()),
            supporting_spring_required: true,
            ..RuleApplicability::default()
        },
        RuleLimit::MinimumRadiusMm { radius_mm: support::d("30") },
    )
}

fn spacing_rule() -> FeasibilityRule {
    rule(2, FeasibilityRuleKind::BendSpacing, RuleApplicability::default(), RuleLimit::MinimumLengthMm { length_mm: support::d("20") })
}

fn compatibility_rule() -> FeasibilityRule {
    rule(
        3,
        FeasibilityRuleKind::TubeConnectorCompatibility,
        RuleApplicability { connector_component_id: Some(Uuid::from_u128(CONNECTOR)), ..RuleApplicability::default() },
        RuleLimit::CompatibleTubeRange {
            outer_diameter_mm: NumericRangeMm { min_mm: support::d("8"), max_mm: support::d("10") },
            inner_diameter_mm: NumericRangeMm { min_mm: support::d("5"), max_mm: support::d("8") },
        },
    )
}

fn end_length_rule() -> FeasibilityRule {
    rule(
        4,
        FeasibilityRuleKind::EndAssemblyLength,
        RuleApplicability { connector_component_id: Some(Uuid::from_u128(CONNECTOR)), ..RuleApplicability::default() },
        RuleLimit::MinimumLengthMm { length_mm: support::d("25") },
    )
}

fn rule_table(request_id: &str, expected_version: u64, rules: Vec<FeasibilityRule>) -> CreateFeasibilityRuleTableVersion {
    CreateFeasibilityRuleTableVersion {
        request_id: request_id.into(),
        expected_version,
        rule_table_id: Uuid::from_u128(0xfa01),
        name: "Reviewed design rules".into(),
        rules,
        source_reference: Some("design rules/".into()),
    }
}

fn attributes() -> FeasibilityTubeAttributes {
    FeasibilityTubeAttributes {
        tube_kind: Some(TubeKind::Smooth),
        material_name: Some("PA12".into()),
        supporting_spring: Some(true),
        nominal_outer_diameter_mm: Some(support::d("10")),
        nominal_wall_thickness_mm: Some(support::d("1")),
        nominal_connection_size_mm: None,
    }
}

fn measurements() -> FeasibilityTubeMeasurements {
    FeasibilityTubeMeasurements {
        outer_diameter_mm: support::d("10"),
        inner_diameter_mm: support::d("8"),
        bend_radii_mm: vec![support::d("35"), support::d("25")],
        tangent_straight_lengths_mm: vec![support::d("30")],
        straight_end_lengths_mm: vec![support::d("40"), support::d("40")],
    }
}

fn connector() -> FeasibilityConnector {
    FeasibilityConnector { catalog_component_id: Uuid::from_u128(CONNECTOR), catalog_component_version_id: Uuid::from_u128(0xc1) }
}

fn ends() -> Vec<FeasibilityTubeEnd> {
    vec![
        FeasibilityTubeEnd { end: TubeEnd::Start, connector: Some(connector()) },
        FeasibilityTubeEnd { end: TubeEnd::End, connector: Some(connector()) },
    ]
}

fn evidence(revision: Uuid) -> FeasibilityComponentEvidence {
    FeasibilityComponentEvidence {
        component_id: Uuid::from_u128(COMPONENT),
        product_revision_id: revision,
        analysis_key: key(),
        extraction_version: "1".into(),
        attributes: attributes(),
        measurements: Some(measurements()),
        ends: ends(),
    }
}

fn assessment(revision: Uuid, rule_table_version_id: Uuid, request_id: &str) -> RecordFeasibilityAssessment {
    RecordFeasibilityAssessment { request_id: request_id.into(), expected_version: 0, rule_table_version_id, components: vec![evidence(revision)] }
}

fn failing_finding(outcome: &FeasibilityAssessmentOutcome) -> solidbom_contract::FeasibilityFinding {
    outcome
        .assessment
        .findings
        .iter()
        .find(|view| {
            view.finding.key.kind == FeasibilityRuleKind::MinimumBendRadius && view.finding.key.check == FindingCheck::BendRadius { bend_index: 1 }
        })
        .map(|view| view.finding.clone())
        .expect("the tight bend finding")
}

async fn publish(store: &Store, request: &CreateFeasibilityRuleTableVersion) -> FeasibilityRuleTablePublished {
    store.publish_feasibility_rules(request.rule_table_id, request, "administrator").await.expect("publish the rule version")
}

async fn record(store: &Store, request: &RecordFeasibilityAssessment) -> FeasibilityAssessmentOutcome {
    store.record_feasibility_assessment(project_id(), request, "engineer").await.expect("record the assessment")
}

async fn view(store: &Store) -> FeasibilityRecordView {
    store.feasibility_record_view(project_id()).await.expect("the feasibility record view")
}

fn codes(view: &FeasibilityRecordView) -> Vec<&str> {
    view.issues.iter().map(|issue| issue.code.as_str()).collect()
}

#[tokio::test]
async fn a_rule_table_is_sourced_versions_are_append_only_and_retries_are_idempotent() {
    let db = TestDb::new().await;
    let store = &db.store;
    let request = rule_table("rules-1", 0, vec![radius_rule(), spacing_rule()]);
    let published = publish(store, &request).await;
    assert_eq!(published.rule_table.version, 1);
    assert_eq!(published.rule_table.supersedes, None);
    assert_eq!(published.rule_table.source_reference.as_deref(), Some("design rules/"));
    assert_eq!(published.rule_table.created_by.as_deref(), Some("administrator"));
    assert_eq!(published.change.previous_version_id, None);
    assert!(published.rule_table.created_at.is_some());

    // An identical retry returns the recorded reply; a reused id with another payload conflicts.
    let again = publish(store, &request).await;
    assert_eq!(again, published);
    let mut changed = request.clone();
    changed.name = "Changed".into();
    let reused = store.publish_feasibility_rules(request.rule_table_id, &changed, "administrator").await;
    assert!(matches!(reused, Err(StoreError::Conflict(_))), "{reused:?}");

    // A stale expected version writes nothing.
    let stale = rule_table("rules-2", 0, vec![radius_rule()]);
    let conflict = store.publish_feasibility_rules(stale.rule_table_id, &stale, "administrator").await;
    assert!(matches!(conflict, Err(StoreError::Conflict(_))), "{conflict:?}");

    // A new version supersedes the previous one and keeps its own rules.
    let second = rule_table("rules-3", 1, vec![radius_rule(), spacing_rule(), end_length_rule()]);
    let published_second = publish(store, &second).await;
    assert_eq!(published_second.rule_table.version, 2);
    assert_eq!(published_second.rule_table.supersedes, Some(published.rule_table.rule_table_version_id));
    assert_eq!(published_second.change.previous_version_id, Some(published.rule_table.rule_table_version_id));
    assert_eq!(published_second.rule_table.rules.len(), 3);

    // A rule without its engineering source is refused before anything is written.
    let mut unsourced = radius_rule();
    unsourced.source_reference = " ".into();
    let invalid = rule_table("rules-4", 2, vec![unsourced]);
    let refused = store.publish_feasibility_rules(invalid.rule_table_id, &invalid, "administrator").await;
    assert!(matches!(refused, Err(StoreError::Invalid(_))), "{refused:?}");

    // The Engineer reads every published version; history stays readable.
    let identity = solidbom_contract::Identity {
        identity_id: Uuid::from_u128(0x1),
        name: "engineer".into(),
        permissions: vec![solidbom_contract::Permission::Engineer],
    };
    let rules = store.feasibility_rules_view(&identity).await.expect("the rule view");
    assert_eq!(rules.rule_tables.len(), 2);
    assert_eq!(rules.rule_tables[0].version, 1);
    assert_eq!(rules.rule_tables[1].version, 2);
    assert_eq!(rules.changes.len(), 2);
    assert_eq!(rules.changes[0].version, 2, "newest change first");
    assert_eq!(rules.changes[0].changed_by, "administrator");
    db.cleanup().await;
}

#[tokio::test]
async fn an_assessment_evaluates_every_radius_and_the_tangent_to_tangent_spacing() {
    let db = TestDb::new().await;
    let store = &db.store;
    let (revision, _part) = setup(store).await;
    let rules = publish(store, &rule_table("rules-1", 0, vec![radius_rule(), spacing_rule(), compatibility_rule(), end_length_rule()])).await;

    let outcome = record(store, &assessment(revision, rules.rule_table.rule_table_version_id, "assessment-1")).await;
    assert_eq!(outcome.receipt.version, 1);
    assert_eq!(outcome.assessment.assessment.engineering_record_version, 1);
    assert!(outcome.assessment.assessment.created_at.contains('T'));
    assert_eq!(outcome.assessment.assessment.created_by, "engineer");

    // Each bend is its own finding: the first passes at 35 mm, the second fails at 25 mm.
    let first =
        outcome.assessment.findings.iter().find(|view| view.finding.key.check == FindingCheck::BendRadius { bend_index: 0 }).expect("the first bend");
    assert_eq!(first.finding.state, FindingState::Pass);
    assert_eq!(first.finding.measurements[0].value_mm, support::d("35"));
    let second = outcome
        .assessment
        .findings
        .iter()
        .find(|view| view.finding.key.check == FindingCheck::BendRadius { bend_index: 1 })
        .expect("the second bend");
    assert_eq!(second.finding.state, FindingState::Fail);
    assert_eq!(second.finding.required_limit, Some(RuleLimit::MinimumRadiusMm { radius_mm: support::d("30") }));
    assert_eq!(second.finding.applied_rules[0].source_reference, "ENG06517, 2010-11-05");

    // The spacing finding is the tangent-to-tangent measured value.
    let spacing =
        outcome.assessment.findings.iter().find(|view| view.finding.key.kind == FeasibilityRuleKind::BendSpacing).expect("the spacing finding");
    assert_eq!(spacing.finding.measurements[0].field_path, "tangent_straight_lengths_mm[0]");
    assert_eq!(spacing.finding.state, FindingState::Pass);

    // The open finding is a readiness issue; the record is readable and append-only.
    let view = view(store).await;
    assert_eq!(view.version, 1);
    assert!(codes(&view).contains(&"open_finding_failed"), "{:?}", view.issues);
    let latest = view.latest_assessment.expect("the latest assessment");
    assert!(latest.findings.iter().any(|view| view.disposition == FindingDisposition::Open));

    // An identical retry returns the recorded outcome; a stale expected version is a conflict.
    let again = record(store, &assessment(revision, rules.rule_table.rule_table_version_id, "assessment-1")).await;
    assert_eq!(again, outcome);
    let mut stale = assessment(revision, rules.rule_table.rule_table_version_id, "assessment-2");
    stale.expected_version = 0;
    let conflict = store.record_feasibility_assessment(project_id(), &stale, "engineer").await;
    assert!(matches!(conflict, Err(StoreError::Conflict(_))), "{conflict:?}");
    db.cleanup().await;
}

#[tokio::test]
async fn missing_measurements_connectors_and_limits_stay_unresolved() {
    let db = TestDb::new().await;
    let store = &db.store;
    let (revision, _part) = setup(store).await;

    // No published rules: the checks are unresolved for a missing limit, never passed.
    let empty = publish(store, &rule_table("rules-empty", 0, Vec::new())).await;
    let no_limits = record(store, &assessment(revision, empty.rule_table.rule_table_version_id, "assessment-1")).await;
    assert!(no_limits.assessment.findings.iter().all(|view| view.finding.state == FindingState::Unresolved));
    assert!(
        no_limits
            .assessment
            .findings
            .iter()
            .all(|view| view.finding.unresolved_reason == Some(solidbom_contract::UnresolvedReason::NoApplicableLimit))
    );
    assert!(codes(&view(store).await).contains(&"open_finding_unresolved"));

    // No exact measurements: one unresolved whole-tube finding per kept check kind.
    let mut unmeasured = assessment(revision, empty.rule_table.rule_table_version_id, "assessment-2");
    unmeasured.expected_version = 1;
    unmeasured.components[0].measurements = None;
    unmeasured.components[0].ends = Vec::new();
    let missing = record(store, &unmeasured).await;
    assert!(missing.assessment.findings.iter().all(|view| view.finding.state == FindingState::Unresolved));
    assert!(
        missing
            .assessment
            .findings
            .iter()
            .all(|view| view.finding.unresolved_reason == Some(solidbom_contract::UnresolvedReason::MissingMeasurement))
    );
    assert!(missing.assessment.findings.iter().all(|view| view.finding.key.check == FindingCheck::WholeTube));
    db.cleanup().await;
}

#[tokio::test]
async fn an_exception_is_visible_and_reopens_when_the_supporting_review_changes() {
    let db = TestDb::new().await;
    let store = &db.store;
    let (revision, part_version_id) = setup(store).await;
    let rules = publish(store, &rule_table("rules-1", 0, vec![radius_rule(), spacing_rule(), compatibility_rule(), end_length_rule()])).await;
    let outcome = record(store, &assessment(revision, rules.rule_table.rule_table_version_id, "assessment-1")).await;
    let failing = failing_finding(&outcome);

    let exception = store
        .accept_feasibility_exception(
            project_id(),
            &AcceptFeasibilityException {
                request_id: "exception-1".into(),
                expected_version: 1,
                assessment_version: 1,
                finding_key: failing.key.clone(),
                justification: "the customer accepts the tighter bend; reviewed on the bench".into(),
            },
            "engineer",
        )
        .await
        .expect("accept the exception");
    assert_eq!(exception.receipt.version, 2);
    assert!(matches!(exception.facts.as_slice(), [FeasibilityFact::ExceptionAccepted { replaced: None, .. }]));

    // The exception is visible and active; the geometric failure stays visible.
    let current = view(store).await;
    assert_eq!(current.version, 2);
    assert_eq!(current.exceptions.len(), 1);
    assert_eq!(current.exceptions[0].state, ExceptionState::Active);
    assert_eq!(current.exceptions[0].exception.accepted_by, "engineer");
    assert_eq!(current.exceptions[0].exception.justification, "the customer accepts the tighter bend; reviewed on the bench");
    assert!(current.issues.is_empty(), "{:?}", current.issues);
    let latest = current.latest_assessment.expect("the latest assessment");
    let failing_view = latest.findings.iter().find(|view| view.finding.key == failing.key).expect("the failing view");
    assert_eq!(failing_view.finding.state, FindingState::Fail, "the exception resolves disposition, not compliance");
    assert_eq!(failing_view.disposition, FindingDisposition::ExceptionAccepted { exception_id: current.exceptions[0].exception.exception_id });

    // Re-accepting the same finding replaces the decision and stays visible.
    let replaced = store
        .accept_feasibility_exception(
            project_id(),
            &AcceptFeasibilityException {
                request_id: "exception-2".into(),
                expected_version: 2,
                assessment_version: 1,
                finding_key: failing.key.clone(),
                justification: "the customer reconfirmed the tighter bend".into(),
            },
            "engineer",
        )
        .await
        .expect("replace the exception");
    assert_eq!(replaced.receipt.version, 3);
    assert!(matches!(replaced.facts.as_slice(), [FeasibilityFact::ExceptionAccepted { replaced: Some(_), .. }]));

    // Changed supporting geometry (the reviewed body set) makes the assessment stale and the
    // exception review-required; it is never silently kept.
    store
        .group_solids(
            project_id(),
            &solidbom_contract::GroupSolids {
                request_id: "group-2".into(),
                expected_version: 1,
                component_id: Uuid::from_u128(COMPONENT),
                product_revision_id: revision,
                analysis_key: key(),
                geometry: vec![body(0), body(1)],
                part_version_id: Some(part_version_id),
            },
        )
        .await
        .expect("change the reviewed body set");
    let changed = view(store).await;
    assert_eq!(changed.exceptions[0].state, ExceptionState::ReviewRequired);
    assert_eq!(changed.exceptions[0].review_required_reasons, vec![solidbom_contract::ExceptionInvalidationReason::EngineeringReviewChanged]);
    let issue_codes = codes(&changed);
    assert!(issue_codes.contains(&"assessment_stale"), "{issue_codes:?}");
    assert!(issue_codes.contains(&"exception_review_required"), "{issue_codes:?}");

    // A new decision cannot be accepted against the changed review: assess it again first.
    let refused = store
        .accept_feasibility_exception(
            project_id(),
            &AcceptFeasibilityException {
                request_id: "exception-3".into(),
                expected_version: 3,
                assessment_version: 1,
                finding_key: failing.key.clone(),
                justification: "accepted against changed geometry".into(),
            },
            "engineer",
        )
        .await;
    assert!(matches!(refused, Err(StoreError::Invalid(_))), "{refused:?}");

    // Retained versions still read; the earlier version has no exception and the record history is
    // not rewritten.
    let first = store.feasibility_record_version(project_id(), 1).await.expect("the first record version");
    assert!(first.exceptions.is_empty());
    assert_eq!(first.latest_assessment.expect("assessment").assessment.version, 1);
    db.cleanup().await;
}

#[tokio::test]
async fn a_conflict_or_missing_limit_can_be_excepted_but_a_pass_or_missing_measurement_cannot() {
    let db = TestDb::new().await;
    let store = &db.store;
    let (revision, _part) = setup(store).await;

    // Two overlapping radius rules disagree: the check is a conflict with no chosen limit.
    let mut second = radius_rule();
    second.rule_id = Uuid::from_u128(9);
    second.limit = RuleLimit::MinimumRadiusMm { radius_mm: support::d("20") };
    let rules = publish(store, &rule_table("rules-1", 0, vec![radius_rule(), second, spacing_rule()])).await;
    let outcome = record(store, &assessment(revision, rules.rule_table.rule_table_version_id, "assessment-1")).await;
    let conflict =
        outcome.assessment.findings.iter().find(|view| view.finding.key.kind == FeasibilityRuleKind::MinimumBendRadius).expect("the radius finding");
    assert_eq!(conflict.finding.state, FindingState::Conflict);
    assert_eq!(conflict.finding.required_limit, None);
    let exception = store
        .accept_feasibility_exception(
            project_id(),
            &AcceptFeasibilityException {
                request_id: "exception-1".into(),
                expected_version: 1,
                assessment_version: 1,
                finding_key: conflict.finding.key.clone(),
                justification: "the Administrator confirmed the tighter of the two source limits".into(),
            },
            "engineer",
        )
        .await
        .expect("the justified-exception route resolves the conflict disposition");
    assert_eq!(exception.receipt.version, 2);

    // A passing finding needs no exception and is refused.
    let pass = outcome.assessment.findings.iter().find(|view| view.finding.state == FindingState::Pass).expect("a passing finding").finding.clone();
    let refused = store
        .accept_feasibility_exception(
            project_id(),
            &AcceptFeasibilityException {
                request_id: "exception-2".into(),
                expected_version: 2,
                assessment_version: 1,
                finding_key: pass.key.clone(),
                justification: "not needed".into(),
            },
            "engineer",
        )
        .await;
    assert!(matches!(refused, Err(StoreError::Invalid(_))), "{refused:?}");

    // A missing measurement is review work, not an engineering-limit decision: it cannot be excepted.
    let mut unmeasured = assessment(revision, rules.rule_table.rule_table_version_id, "assessment-2");
    unmeasured.expected_version = 2;
    unmeasured.components[0].measurements = None;
    unmeasured.components[0].ends = Vec::new();
    let missing = record(store, &unmeasured).await;
    let missing_finding = missing.assessment.findings.first().expect("a missing finding").finding.clone();
    let refused = store
        .accept_feasibility_exception(
            project_id(),
            &AcceptFeasibilityException {
                request_id: "exception-3".into(),
                expected_version: missing.receipt.version,
                assessment_version: missing.receipt.version,
                finding_key: missing_finding.key.clone(),
                justification: "the geometry is unavailable; accepted".into(),
            },
            "engineer",
        )
        .await;
    assert!(matches!(refused, Err(StoreError::Invalid(_))), "{refused:?}");

    // A justification is required and bounded before anything is written.
    let empty = AcceptFeasibilityException {
        request_id: "exception-4".into(),
        expected_version: missing.receipt.version,
        assessment_version: missing.receipt.version,
        finding_key: missing_finding.key.clone(),
        justification: " ".into(),
    };
    let refused = store.accept_feasibility_exception(project_id(), &empty, "engineer").await;
    assert!(matches!(refused, Err(StoreError::Invalid(_))), "{refused:?}");

    // An exception is accepted against the latest recorded assessment only; a superseded assessment
    // is refused before anything is written.
    let superseded_finding =
        outcome.assessment.findings.iter().find(|view| view.finding.state == FindingState::Conflict).expect("the conflict finding").finding.clone();
    let refused = store
        .accept_feasibility_exception(
            project_id(),
            &AcceptFeasibilityException {
                request_id: "exception-5".into(),
                expected_version: missing.receipt.version,
                assessment_version: 1,
                finding_key: superseded_finding.key,
                justification: "accepted against an outdated assessment".into(),
            },
            "engineer",
        )
        .await;
    assert!(matches!(refused, Err(StoreError::Invalid(_))), "{refused:?}");
    db.cleanup().await;
}

#[tokio::test]
async fn an_assessment_must_describe_the_exact_reviewed_component_and_rule_version() {
    let db = TestDb::new().await;
    let store = &db.store;
    let (revision, _part) = setup(store).await;
    let rules = publish(store, &rule_table("rules-1", 0, vec![radius_rule()])).await;
    let rule_table_version_id = rules.rule_table.rule_table_version_id;

    // An unknown component, a different revision and a different analysis are refused.
    let mut unknown = assessment(revision, rule_table_version_id, "assessment-unknown");
    unknown.components[0].component_id = Uuid::from_u128(0xdead);
    assert!(matches!(store.record_feasibility_assessment(project_id(), &unknown, "engineer").await, Err(StoreError::Invalid(_))));
    let mut other_revision = assessment(revision, rule_table_version_id, "assessment-revision");
    other_revision.components[0].product_revision_id = Uuid::new_v4();
    assert!(matches!(store.record_feasibility_assessment(project_id(), &other_revision, "engineer").await, Err(StoreError::Invalid(_))));
    let mut other_analysis = assessment(revision, rule_table_version_id, "assessment-analysis");
    other_analysis.components[0].analysis_key.source_sha256 = support::digest(b"another source");
    assert!(matches!(store.record_feasibility_assessment(project_id(), &other_analysis, "engineer").await, Err(StoreError::Invalid(_))));

    // An unpublished rule version cannot be assessed.
    let unknown_rules = RecordFeasibilityAssessment {
        request_id: "assessment-rules".into(),
        expected_version: 0,
        rule_table_version_id: Uuid::new_v4(),
        components: vec![evidence(revision)],
    };
    assert!(matches!(store.record_feasibility_assessment(project_id(), &unknown_rules, "engineer").await, Err(StoreError::Invalid(_))));

    // A project that was never created has no feasibility record.
    let missing = store.feasibility_record_view(Uuid::new_v4()).await;
    assert!(matches!(missing, Err(StoreError::NotFound)), "{missing:?}");
    // A record version that was never written is not found.
    let version = store.feasibility_record_version(project_id(), 9).await;
    assert!(matches!(version, Err(StoreError::NotFound)), "{version:?}");
    db.cleanup().await;
}

#[tokio::test]
async fn an_obsolete_exception_is_closed_after_its_finding_passes() {
    let db = TestDb::new().await;
    let store = &db.store;
    let (revision, _part) = setup(store).await;
    let rules = publish(store, &rule_table("rules-1", 0, vec![radius_rule(), spacing_rule(), compatibility_rule(), end_length_rule()])).await;
    let rule_table_version_id = rules.rule_table.rule_table_version_id;
    let first = record(store, &assessment(revision, rule_table_version_id, "assessment-1")).await;
    let failing = failing_finding(&first);
    store
        .accept_feasibility_exception(
            project_id(),
            &AcceptFeasibilityException {
                request_id: "exception-1".into(),
                expected_version: 1,
                assessment_version: 1,
                finding_key: failing.key.clone(),
                justification: "the customer accepts the tighter bend".into(),
            },
            "engineer",
        )
        .await
        .expect("accept the exception");
    let exception_id = view(store).await.exceptions[0].exception.exception_id;

    // A finding that still fails cannot be closed; it must be re-accepted instead.
    let refused = store
        .close_feasibility_exception(
            project_id(),
            &CloseFeasibilityException { request_id: "close-early".into(), expected_version: 2, exception_id, reason: "not obsolete yet".into() },
            "engineer",
        )
        .await;
    assert!(matches!(refused, Err(StoreError::Invalid(_))), "{refused:?}");

    // A newer assessment of the same review makes the tight bend pass, so the decision is obsolete
    // and asks for review again.
    let mut passing = assessment(revision, rule_table_version_id, "assessment-2");
    passing.expected_version = 2;
    passing.components[0].measurements.as_mut().expect("the measured tube").bend_radii_mm = vec![support::d("35"), support::d("40")];
    let second = record(store, &passing).await;
    assert_eq!(second.receipt.version, 3);
    let review_required = view(store).await;
    assert_eq!(review_required.exceptions[0].state, ExceptionState::ReviewRequired);
    assert!(codes(&review_required).contains(&"exception_review_required"), "{:?}", review_required.issues);

    // The explicit closure records the closing Engineer, the timestamp and the reason.
    let close = CloseFeasibilityException {
        request_id: "close-1".into(),
        expected_version: 3,
        exception_id,
        reason: "the new assessment passes the bend".into(),
    };
    let closed = store.close_feasibility_exception(project_id(), &close, "engineer").await.expect("close the obsolete exception");
    assert_eq!(closed.receipt.version, 4);
    assert!(matches!(closed.facts.as_slice(), [FeasibilityFact::ExceptionClosed { .. }]));
    let after = view(store).await;
    assert_eq!(after.exceptions[0].state, ExceptionState::Closed);
    let closure = after.exceptions[0].exception.closed.as_ref().expect("the recorded closure");
    assert_eq!(closure.closed_by, "engineer");
    assert_eq!(closure.reason, "the new assessment passes the bend");
    assert!(closure.closed_at.contains('T'));
    assert!(after.issues.is_empty(), "{:?}", after.issues);
    let latest = after.latest_assessment.expect("the latest assessment");
    assert!(latest.findings.iter().all(|view| view.disposition == FindingDisposition::Open), "a closed decision covers nothing");

    // The closure write is idempotent on retry and a second distinct closure is refused.
    let retried = store.close_feasibility_exception(project_id(), &close, "engineer").await.expect("the identical retry");
    assert_eq!(retried, closed);
    let again = store
        .close_feasibility_exception(
            project_id(),
            &CloseFeasibilityException { request_id: "close-2".into(), expected_version: 4, ..close },
            "engineer",
        )
        .await;
    assert!(matches!(again, Err(StoreError::Invalid(_))), "{again:?}");

    // Earlier record versions keep the decision visible without its closure.
    let historical = store.feasibility_record_version(project_id(), 3).await.expect("the review-required version");
    assert!(historical.exceptions[0].exception.closed.is_none());
    assert_eq!(historical.exceptions[0].state, ExceptionState::ReviewRequired);
    db.cleanup().await;
}

#[tokio::test]
async fn an_obsolete_exception_can_be_closed_when_its_finding_is_no_longer_recorded() {
    let db = TestDb::new().await;
    let store = &db.store;
    let (revision, _part) = setup(store).await;
    let rules = publish(store, &rule_table("rules-1", 0, vec![radius_rule(), spacing_rule(), compatibility_rule(), end_length_rule()])).await;
    let rule_table_version_id = rules.rule_table.rule_table_version_id;
    let first = record(store, &assessment(revision, rule_table_version_id, "assessment-1")).await;
    let failing = failing_finding(&first);
    store
        .accept_feasibility_exception(
            project_id(),
            &AcceptFeasibilityException {
                request_id: "exception-1".into(),
                expected_version: 1,
                assessment_version: 1,
                finding_key: failing.key.clone(),
                justification: "the customer accepts the tighter bend".into(),
            },
            "engineer",
        )
        .await
        .expect("accept the exception");
    let exception_id = view(store).await.exceptions[0].exception.exception_id;

    // The tube is re-extracted as a straight tube: the bend finding no longer exists.
    let mut straight = assessment(revision, rule_table_version_id, "assessment-2");
    straight.expected_version = 2;
    let measured = straight.components[0].measurements.as_mut().expect("the measured tube");
    measured.bend_radii_mm.clear();
    measured.tangent_straight_lengths_mm.clear();
    measured.straight_end_lengths_mm = vec![support::d("100"), support::d("100")];
    let second = record(store, &straight).await;
    assert_eq!(second.receipt.version, 3);
    let obsolete = view(store).await;
    assert_eq!(obsolete.exceptions[0].state, ExceptionState::ReviewRequired);
    assert!(codes(&obsolete).contains(&"exception_review_required"), "{:?}", obsolete.issues);

    let closed = store
        .close_feasibility_exception(
            project_id(),
            &CloseFeasibilityException {
                request_id: "close-1".into(),
                expected_version: 3,
                exception_id,
                reason: "the bend is no longer present in the reviewed tube".into(),
            },
            "engineer",
        )
        .await
        .expect("close the obsolete exception");
    assert_eq!(closed.receipt.version, 4);
    let after = view(store).await;
    assert_eq!(after.exceptions[0].state, ExceptionState::Closed);
    assert!(after.issues.is_empty(), "{:?}", after.issues);
    db.cleanup().await;
}
