//! Tests for analytic recognition, conservative catalog matching and the independent-reference
//! evaluation harness.
//!
//! Geometry fixtures are produced by SolidCraft's own commands and round-trip through the STEP
//! writer and importer, so the exact B-rep under test is the one a reviewed project would hold.
//! Every expected label, count and evidence code is derived from the authored fixture or from the
//! reference dataset below, never from the recognition output.
//!
//! The evaluation dataset is synthetic: `synthetic contract fixture, not customer data`. It proves
//! the harness's counting, refusal and reporting rules; it is not a claim about real parts.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use solidbom_contract::{
    AnalysisKey, AnalysisManifest, CatalogAttribute, CatalogComponentVersion, CategoryCode, SourceRef, SuggestionEvidence, Unit,
};
use uuid::Uuid;

use super::ReviewPolicy;
use super::extract::{EXTRACTOR_VERSION, engine_build, extract};
use super::extraction::{ExtractionCategory, ExtractionInput, ExtractionStatus, extract_definition};
use super::recognition::{
    CATALOG_MATCH_VERSION, CaseCatalog, CaseClassification, CatalogMatchOutcome, CatalogMatchPart, EvaluationInput, EvaluationReport,
    RECOGNITION_VERSION, REFERENCE_DATASET_SCHEMA_VERSION, RecognitionResult, RecognitionStatus, ReferenceCase, ReferenceDataset, evaluate,
    recognise_definition, suggest_catalog_matches,
};
use crate::Session;

// ---------------------------------------------------------------------------------------------
// Fixtures and helpers
// ---------------------------------------------------------------------------------------------

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session.execute(id, &params).expect(id)
}

fn curve_ids(value: &Value) -> Vec<String> {
    value["curves"].as_array().map(|array| array.iter().filter_map(|item| item.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

fn sha256(path: &Path) -> String {
    Sha256::digest(std::fs::read(path).expect("read the exported STEP")).iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A STEP fixture directory removed when the helper returns; the imported document stays in memory.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|elapsed| elapsed.as_nanos()).unwrap_or(0);
        let path = std::env::temp_dir().join(format!("solidbom-recognition-{name}-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create a temporary directory");
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Export a fixture and import it again, then build its analysis manifest. This is the same
/// reviewed-source path an imported project uses, so recognition describes imported bytes.
fn reviewed_source(built: &mut Session, name: &str) -> (Session, AnalysisManifest) {
    let directory = TempDir::new(name);
    let path = directory.0.join("fixture.step");
    run(built, "file.export", json!({"path": path.to_string_lossy()}));
    let mut imported = Session::default();
    run(&mut imported, "doc.open", json!({"path": path.to_string_lossy()}));
    let key =
        AnalysisKey { source_sha256: sha256(&path), engine_build: engine_build().to_string(), extractor_version: EXTRACTOR_VERSION.to_string() };
    let manifest = extract(&imported, key).expect("extract the imported source");
    (imported, manifest)
}

/// Bind the session to the analysis it reviews, as the application's import path does. SB-12 then
/// refuses a manifest that describes another source instead of measuring whatever is present.
fn bind_review(session: &mut Session, manifest: &AnalysisManifest) {
    let key = manifest.key.clone();
    let source =
        SourceRef { source_id: Uuid::from_u128(0x5b13), sha256: key.source_sha256.clone(), byte_len: 1, original_name: "fixture.step".to_string() };
    session.review = Some(Arc::new(ReviewPolicy::new(source, key)));
}

fn only_definition(manifest: &AnalysisManifest) -> Uuid {
    manifest.definitions.first().expect("one definition in the fixture analysis").definition_id
}

/// A straight tube of `length` mm, diameter 12 mm, 2 mm wall, along +X from the origin.
fn straight_tube(length: f64) -> Session {
    let mut session = Session::default();
    run(&mut session, "sketch.create", json!({"plane": "XY", "name": "Path"}));
    let line = run(&mut session, "sketch.line", json!({"points": [[0.0, 0.0], [length, 0.0]], "connect": false}));
    let path = curve_ids(&line);
    run(&mut session, "sketch.finish", json!({}));
    let reply = run(&mut session, "solid.pipe", json!({"path_sketch": "Path", "path": path, "diameter": 12.0, "wall": 2.0, "body_name": "Tube"}));
    assert!(reply["warning"].is_null(), "{reply}");
    session
}

/// One 90-degree bend of centerline radius `radius` mm, diameter 10 mm, 2 mm wall.
fn one_bend_tube(radius: f64) -> Session {
    let mut session = Session::default();
    run(&mut session, "sketch.create", json!({"plane": "XY", "name": "Path"}));
    let arc = run(&mut session, "sketch.arc.center_point", json!({"center": [0.0, radius], "start": [0.0, 0.0], "sweep": 90.0}));
    let path = curve_ids(&arc);
    run(&mut session, "sketch.finish", json!({}));
    let reply = run(&mut session, "solid.pipe", json!({"path_sketch": "Path", "path": path, "diameter": 10.0, "wall": 2.0, "body_name": "Tube"}));
    assert!(reply["warning"].is_null(), "{reply}");
    session
}

/// A closed, solid, volume-bearing box: the honest example of geometry that is *not* a tube and
/// must never be guessed into a connector, fitting or clip label.
fn a_box() -> Session {
    let mut session = Session::default();
    let reply = run(&mut session, "solid.box", json!({"length": 120.0, "width": 80.0, "height": 40.0, "body_name": "Box"}));
    assert!(reply["warning"].is_null(), "{reply}");
    session
}

/// A solid rod: a closed solid with no bore, so it is not in the annular tube subset.
fn a_rod() -> Session {
    let mut session = Session::default();
    run(&mut session, "sketch.create", json!({"plane": "XY", "name": "Path"}));
    let line = run(&mut session, "sketch.line", json!({"points": [[0.0, 0.0], [100.0, 0.0]], "connect": false}));
    let path = curve_ids(&line);
    run(&mut session, "sketch.finish", json!({}));
    let reply = run(&mut session, "solid.pipe", json!({"path_sketch": "Path", "path": path, "diameter": 12.0, "body_name": "Rod"}));
    assert!(reply["warning"].is_null(), "{reply}");
    session
}

fn recogniser(session: &Session, manifest: &AnalysisManifest, definition_id: Uuid) -> RecognitionResult {
    recognise_definition(session, manifest, definition_id).expect("recognise the reviewed definition")
}

fn codes(evidence: &[SuggestionEvidence]) -> Vec<&str> {
    evidence.iter().map(|item| item.code.as_str()).collect()
}

// ---------------------------------------------------------------------------------------------
// Recognition rule v1
// ---------------------------------------------------------------------------------------------

#[test]
fn a_straight_tube_is_suggested_as_a_tube_with_stable_evidence() {
    let mut built = straight_tube(1000.0);
    let (session, manifest) = reviewed_source(&mut built, "straight-tube");
    let definition = only_definition(&manifest);
    let result = recogniser(&session, &manifest, definition);

    assert_eq!(result.status, RecognitionStatus::Suggested);
    assert_eq!(result.recognition_version, RECOGNITION_VERSION);
    assert_eq!(result.analysis_key, manifest.key);
    assert_eq!(result.definition_id, definition);
    assert_eq!(result.category.as_ref().map(CategoryCode::as_str), Some("tube"));
    // An analytic rule reports no probability; a constant confidence would be an invented number.
    assert_eq!(result.confidence, None);
    assert!(result.issues.is_empty(), "{:?}", result.issues);
    // The evidence order is part of the result: the same geometry must list the same codes.
    assert_eq!(
        codes(&result.evidence),
        [
            "tubular_sweep",
            "annular_end_faces",
            "constant_diameter",
            "developed_length_measured",
            "bend_count_measured",
            "category_shared_with_protection"
        ]
    );
    let protection = result.evidence.iter().find(|item| item.code == "category_shared_with_protection").expect("the protection note");
    assert!(protection.detail.contains("protection"), "{protection:?}");
    assert!(protection.detail.contains("confirmation"), "{protection:?}");
    let developed = result.evidence.iter().find(|item| item.code == "developed_length_measured").expect("the developed length note");
    assert!(developed.detail.contains("mm"), "{developed:?}");

    let part_version_id = Uuid::from_u128(7);
    let suggestion = result.suggestion(part_version_id).expect("a proposed classification");
    assert_eq!(suggestion.part_version_id, part_version_id);
    assert_eq!(suggestion.prediction.name, "tube");
    assert_eq!(suggestion.prediction.confidence, None, "a prediction carries no probability");
    assert_eq!(suggestion.prediction.prediction_version.as_deref(), Some(RECOGNITION_VERSION));
    assert_eq!(suggestion.prediction.analysis_key.as_ref(), Some(&manifest.key));
    assert_eq!(suggestion.prediction.evidence, result.evidence);
    assert_eq!(suggestion.prediction.suggested_at, None, "the service timestamps a recorded suggestion");
    assert!(!suggestion.prediction.model_version.is_empty());
    // A suggestion is never a confirmation: the prediction has no confirmed field at all.
    let prediction = serde_json::to_value(&suggestion.prediction).expect("serialize the prediction");
    assert!(prediction.get("confirmed").is_none());
    assert!(prediction.get("confirmed_by").is_none());
}

#[test]
fn a_bent_tube_reports_its_bend_count_and_the_rule_is_deterministic() {
    let mut built = one_bend_tube(20.0);
    let (session, manifest) = reviewed_source(&mut built, "bent-tube");
    let definition = only_definition(&manifest);
    let first = recogniser(&session, &manifest, definition);
    assert_eq!(first.status, RecognitionStatus::Suggested);
    assert_eq!(first.category.as_ref().map(CategoryCode::as_str), Some("tube"));
    assert_eq!(codes(&first.evidence), codes(&recogniser(&session, &manifest, definition).evidence));
    let bends = first.evidence.iter().find(|item| item.code == "bend_count_measured").expect("the bend count note");
    assert!(bends.detail.contains('1'), "{bends:?}");
    // Calling the rule again on the same exact analysis gives the same result, evidence order included.
    assert_eq!(first, recogniser(&session, &manifest, definition));
    // The result serializes as a proposal with no confirmed label.
    let json = serde_json::to_value(&first).expect("serialize the recognition result");
    assert!(json.get("confirmed").is_none(), "{json}");
    assert!(json.get("confirmed_category").is_none(), "{json}");
}

#[test]
fn a_closed_box_is_unsupported_and_never_guessed_as_a_connector() {
    let mut built = a_box();
    let (session, manifest) = reviewed_source(&mut built, "box");
    let definition = only_definition(&manifest);
    // Read the extractor's own issue codes from the same exact analysis: the mapping below is
    // checked against SB-12's output, not against a hardcoded assumption.
    let input = ExtractionInput::new(manifest.key.clone(), definition, ExtractionCategory::Tube);
    let extraction = extract_definition(&session, &manifest, &input).expect("the box is measured");
    assert_eq!(extraction.status, ExtractionStatus::Unsupported);
    assert!(!extraction.issues.is_empty(), "unsupported extraction always names why");

    let result = recogniser(&session, &manifest, definition);
    assert_eq!(result.status, RecognitionStatus::Unsupported);
    assert_eq!(result.category, None, "no category is guessed");
    assert_eq!(result.confidence, None);
    assert!(result.evidence.is_empty(), "{:?}", result.evidence);
    assert!(result.suggestion(Uuid::from_u128(1)).is_none(), "unsupported geometry is never written as a proposal");
    let issue_codes = codes(&result.issues);
    assert!(issue_codes.contains(&"recognition_unsupported"), "{issue_codes:?}");
    assert!(issue_codes.contains(&"no_matching_analytic_rule"), "{issue_codes:?}");
    for issue in &extraction.issues {
        let mapped = format!("extraction_{}", issue.code);
        assert!(issue_codes.iter().any(|code| *code == mapped), "the mapped issue for `{}` is missing: {issue_codes:?}", issue.code);
    }
    let summary = result.issues.iter().find(|item| item.code == "recognition_unsupported").expect("the summary issue");
    for issue in &extraction.issues {
        assert!(summary.detail.contains(&issue.code), "the summary names `{}`: {:?}", issue.code, summary);
    }
    // The other initial categories have no analytic rule here: no label is proposed for them.
    for category in ["connector", "fitting", "clip", "wear_protection", "thermal_protection"] {
        assert!(!issue_codes.contains(&category), "no category label is issued: {issue_codes:?}");
    }
}

#[test]
fn a_solid_rod_is_unsupported_without_a_category() {
    let mut built = a_rod();
    let (session, manifest) = reviewed_source(&mut built, "rod");
    let definition = only_definition(&manifest);
    let result = recogniser(&session, &manifest, definition);
    assert_eq!(result.status, RecognitionStatus::Unsupported);
    assert_eq!(result.category, None);
    assert!(codes(&result.issues).contains(&"recognition_unsupported"));
    assert!(codes(&result.issues).contains(&"no_matching_analytic_rule"));
    assert!(result.suggestion(Uuid::from_u128(2)).is_none());
}

#[test]
fn a_stale_or_malformed_manifest_is_refused() {
    let mut first = straight_tube(100.0);
    let (mut first_session, first_manifest) = reviewed_source(&mut first, "first-source");
    bind_review(&mut first_session, &first_manifest);
    let definition = only_definition(&first_manifest);
    assert_eq!(recogniser(&first_session, &first_manifest, definition).status, RecognitionStatus::Suggested);

    let mut second = one_bend_tube(20.0);
    let (_, second_manifest) = reviewed_source(&mut second, "second-source");
    assert_ne!(first_manifest.key, second_manifest.key, "the fixtures are different sources");
    assert!(
        recognise_definition(&first_session, &second_manifest, only_definition(&second_manifest)).is_err(),
        "a manifest that describes another source is stale and refused"
    );

    let mut newer = first_manifest.clone();
    newer.schema_version += 1;
    assert!(recognise_definition(&first_session, &newer, definition).is_err(), "a future manifest schema is refused");

    assert!(
        recognise_definition(&first_session, &first_manifest, Uuid::from_u128(0xdead)).is_err(),
        "a definition that is not part of the analysis is refused"
    );
}

// ---------------------------------------------------------------------------------------------
// Catalog matching rule v1
// ---------------------------------------------------------------------------------------------

fn category(code: &str) -> CategoryCode {
    CategoryCode::new(code).expect("a valid category code")
}

fn attribute(key: &str, value: &str) -> CatalogAttribute {
    CatalogAttribute { key: key.to_string(), value: value.to_string() }
}

fn component(component_id: Uuid, component_version_id: Uuid, version: u64, attributes: Vec<CatalogAttribute>) -> CatalogComponentVersion {
    CatalogComponentVersion {
        component_version_id,
        component_id,
        version,
        supersedes: None,
        name: format!("Catalog component {version}"),
        specification: "fixture specification".to_string(),
        price: None,
        price_unit: Unit::CurrencyPerPiece,
        currency: "CNY".to_string(),
        source_reference: None,
        created_at: None,
        attributes,
    }
}

fn tube_part(part_version_id: Uuid, customer_part_number: &str) -> CatalogMatchPart {
    CatalogMatchPart {
        part_version_id,
        category: Some(category("tube")),
        customer_part_number: Some(customer_part_number.to_string()),
        customer_revision: None,
    }
}

#[test]
fn one_candidate_becomes_one_suggestion_with_per_attribute_evidence() {
    let component_id = Uuid::from_u128(10);
    let component_version_id = Uuid::from_u128(11);
    let catalog =
        vec![component(component_id, component_version_id, 3, vec![attribute("category", "tube"), attribute("customer_part_number", "PIPE-001")])];
    let part_version_id = Uuid::from_u128(1);
    let part = tube_part(part_version_id, "PIPE-001");
    let outcome = suggest_catalog_matches(&part, &catalog);

    assert!(outcome.issues.is_empty(), "{:?}", outcome.issues);
    assert_eq!(outcome.suggestions.len(), 1);
    let suggestion = &outcome.suggestions[0];
    assert_eq!(suggestion.part_version_id, part_version_id, "the part identity is the given one, never merged");
    assert_eq!(suggestion.component_id, component_id);
    assert_eq!(suggestion.component_version_id, component_version_id);
    assert_eq!(suggestion.prediction_version, CATALOG_MATCH_VERSION);
    assert_eq!(suggestion.confidence, None, "text matching reports no probability");
    assert_eq!(codes(&suggestion.evidence), ["catalog_attributes_match", "catalog_attribute_match", "catalog_attribute_match"]);
    assert!(suggestion.evidence.iter().any(|item| item.detail == "category=tube"), "{:?}", suggestion.evidence);
    assert!(suggestion.evidence.iter().any(|item| item.detail == "customer_part_number=PIPE-001"), "{:?}", suggestion.evidence);
    assert!(!suggestion.model_version.is_empty());
    // The outcome is a proposal, not a confirmed catalog reference.
    let json = serde_json::to_value(&outcome).expect("serialize the outcome");
    assert!(json.get("confirmed").is_none(), "{json}");
    assert!(json.get("confirmed_reference").is_none(), "{json}");
}

#[test]
fn a_pending_classification_never_sources_a_catalog_match() {
    let mut part = tube_part(Uuid::from_u128(1), "PIPE-001");
    part.category = None;
    let outcome = suggest_catalog_matches(&part, &[]);
    assert!(outcome.suggestions.is_empty());
    assert_eq!(codes(&outcome.issues), ["classification_not_confirmed"]);
}

#[test]
fn a_missing_customer_part_number_never_sources_a_catalog_match() {
    for number in [None, Some("".to_string()), Some("   ".to_string())] {
        let mut part = tube_part(Uuid::from_u128(1), "unused");
        part.customer_part_number = number;
        let outcome = suggest_catalog_matches(&part, &[]);
        assert!(outcome.suggestions.is_empty());
        assert_eq!(codes(&outcome.issues), ["missing_customer_part_number"]);
    }
}

#[test]
fn a_component_without_declared_attributes_is_not_a_wildcard() {
    let catalog = vec![component(Uuid::from_u128(20), Uuid::from_u128(21), 1, Vec::new())];
    let part = tube_part(Uuid::from_u128(1), "PIPE-001");
    let outcome = suggest_catalog_matches(&part, &catalog);
    assert!(outcome.suggestions.is_empty(), "{:?}", outcome.suggestions);
    assert_eq!(codes(&outcome.issues), ["catalog_component_has_no_attributes", "no_catalog_match"]);
}

#[test]
fn an_unknown_declared_attribute_never_matches() {
    let catalog = vec![component(
        Uuid::from_u128(30),
        Uuid::from_u128(31),
        1,
        vec![attribute("category", "tube"), attribute("customer_part_number", "PIPE-001"), attribute("diameter_class", "DN20")],
    )];
    let outcome = suggest_catalog_matches(&tube_part(Uuid::from_u128(1), "PIPE-001"), &catalog);
    assert!(outcome.suggestions.is_empty());
    assert_eq!(codes(&outcome.issues), ["no_catalog_match"]);
}

#[test]
fn declared_values_match_after_trimming_and_ascii_case_folding() {
    let mut part = tube_part(Uuid::from_u128(1), "  pipe-001 ");
    part.customer_revision = Some(" Rev-B ".to_string());
    let catalog = vec![component(
        Uuid::from_u128(40),
        Uuid::from_u128(41),
        2,
        vec![attribute("category", " Tube "), attribute("customer_part_number", "PIPE-001"), attribute("customer_revision", "rev-b")],
    )];
    let outcome = suggest_catalog_matches(&part, &catalog);
    assert_eq!(outcome.suggestions.len(), 1, "{:?}", outcome.issues);
    let evidence = &outcome.suggestions[0].evidence;
    assert!(evidence.iter().any(|item| item.detail == "customer_revision=rev-b"), "{evidence:?}");
}

#[test]
fn a_revision_that_the_part_does_not_have_never_matches() {
    let catalog = vec![component(
        Uuid::from_u128(50),
        Uuid::from_u128(51),
        1,
        vec![attribute("category", "tube"), attribute("customer_part_number", "PIPE-001"), attribute("customer_revision", "B")],
    )];
    // The declared revision is an attribute the part must share; a part without one never matches it.
    let outcome = suggest_catalog_matches(&tube_part(Uuid::from_u128(1), "PIPE-001"), &catalog);
    assert!(outcome.suggestions.is_empty());
    assert_eq!(codes(&outcome.issues), ["no_catalog_match"]);
}

#[test]
fn only_the_newest_version_of_a_logical_component_is_a_candidate() {
    let component_id = Uuid::from_u128(60);
    let old_version =
        component(component_id, Uuid::from_u128(61), 1, vec![attribute("category", "tube"), attribute("customer_part_number", "PIPE-001")]);
    // The newest version no longer matches this part; the older matching version must not be used.
    let new_version =
        component(component_id, Uuid::from_u128(62), 2, vec![attribute("category", "tube"), attribute("customer_part_number", "OTHER-999")]);
    let part = tube_part(Uuid::from_u128(1), "PIPE-001");
    let outcome = suggest_catalog_matches(&part, &[new_version.clone(), old_version.clone()]);
    assert!(outcome.suggestions.is_empty(), "{:?}", outcome.suggestions);
    assert_eq!(codes(&outcome.issues), ["no_catalog_match"]);
    // The catalog order never changes the answer.
    assert_eq!(outcome, suggest_catalog_matches(&part, &[old_version, new_version]));
}

#[test]
fn several_equally_matching_components_are_ambiguous_and_never_picked() {
    let first =
        component(Uuid::from_u128(70), Uuid::from_u128(71), 1, vec![attribute("category", "tube"), attribute("customer_part_number", "PIPE-001")]);
    let second =
        component(Uuid::from_u128(72), Uuid::from_u128(73), 1, vec![attribute("category", "tube"), attribute("customer_part_number", "PIPE-001")]);
    let part = tube_part(Uuid::from_u128(1), "PIPE-001");
    let outcome = suggest_catalog_matches(&part, &[first.clone(), second.clone()]);
    assert!(outcome.suggestions.is_empty(), "never pick one of several: {:?}", outcome.suggestions);
    assert_eq!(codes(&outcome.issues), ["ambiguous_catalog_match"]);
    let ambiguous = outcome.issues.first().expect("the ambiguity issue");
    assert!(ambiguous.detail.contains(&Uuid::from_u128(71).to_string()), "{ambiguous:?}");
    assert!(ambiguous.detail.contains(&Uuid::from_u128(73).to_string()), "{ambiguous:?}");
    assert_eq!(outcome, suggest_catalog_matches(&part, &[second, first]));
}

#[test]
fn the_matcher_never_proposes_between_different_part_identities() {
    let catalog = vec![component(
        Uuid::from_u128(80),
        Uuid::from_u128(81),
        1,
        vec![attribute("category", "tube"), attribute("customer_part_number", "PIPE-001")],
    )];
    let first_part = Uuid::from_u128(1);
    let second_part = Uuid::from_u128(2);
    let first = suggest_catalog_matches(&tube_part(first_part, "PIPE-001"), &catalog);
    let second = suggest_catalog_matches(&tube_part(second_part, "PIPE-001"), &catalog);
    assert_eq!(first.suggestions.len(), 1);
    assert_eq!(second.suggestions.len(), 1);
    assert_eq!(first.suggestions[0].part_version_id, first_part);
    assert_eq!(second.suggestions[0].part_version_id, second_part);
    // Each call proposes only for the part it was given; no identity is shared or merged.
    assert_ne!(first.suggestions[0].part_version_id, second.suggestions[0].part_version_id);
}

// ---------------------------------------------------------------------------------------------
// Evaluation harness
// ---------------------------------------------------------------------------------------------

fn analysis_key() -> AnalysisKey {
    AnalysisKey { source_sha256: "a".repeat(64), engine_build: "test-build".to_string(), extractor_version: "2".to_string() }
}

fn evidence(code: &str, detail: &str) -> SuggestionEvidence {
    SuggestionEvidence::new(code, detail).expect("a valid evidence item")
}

fn suggested(definition_id: Uuid, code: &str) -> RecognitionResult {
    RecognitionResult {
        recognition_version: RECOGNITION_VERSION.to_string(),
        analysis_key: analysis_key(),
        definition_id,
        status: RecognitionStatus::Suggested,
        category: Some(category(code)),
        confidence: None,
        evidence: vec![evidence("tubular_sweep", "the fixture geometry is a tubular sweep")],
        issues: Vec::new(),
    }
}

fn unsupported(definition_id: Uuid, issue_codes: &[&str]) -> RecognitionResult {
    RecognitionResult {
        recognition_version: RECOGNITION_VERSION.to_string(),
        analysis_key: analysis_key(),
        definition_id,
        status: RecognitionStatus::Unsupported,
        category: None,
        confidence: None,
        evidence: Vec::new(),
        issues: issue_codes.iter().map(|code| evidence(code, "a fixture issue")).collect(),
    }
}

fn definition(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

/// The synthetic reference dataset: clearly marked, not customer data, and built to exercise every
/// outcome the report has a count for.
fn synthetic_dataset() -> ReferenceDataset {
    ReferenceDataset {
        schema_version: REFERENCE_DATASET_SCHEMA_VERSION,
        dataset_id: "synthetic-sb13-fixture".to_string(),
        provenance: "synthetic contract fixture, not customer data".to_string(),
        cases: vec![
            ReferenceCase {
                case_id: "case-matched".to_string(),
                definition_id: definition(1),
                expected_category: Some(category("tube")),
                expected_catalog_component_version_id: Some(definition(101)),
            },
            ReferenceCase {
                case_id: "case-mismatched".to_string(),
                definition_id: definition(2),
                expected_category: Some(category("wear_protection")),
                expected_catalog_component_version_id: None,
            },
            ReferenceCase {
                case_id: "case-unsupported".to_string(),
                definition_id: definition(3),
                expected_category: Some(category("tube")),
                expected_catalog_component_version_id: None,
            },
            ReferenceCase {
                case_id: "case-unclassified".to_string(),
                definition_id: definition(5),
                expected_category: None,
                expected_catalog_component_version_id: None,
            },
            // There is deliberately no input for this case: it proves missing results are counted.
            ReferenceCase {
                case_id: "case-missing".to_string(),
                definition_id: definition(4),
                expected_category: Some(category("tube")),
                expected_catalog_component_version_id: None,
            },
        ],
    }
}

/// The catalog proposal of the matched case: a real matcher outcome over a fixture catalog.
fn matched_catalog_outcome(part_version_id: Uuid) -> CatalogMatchOutcome {
    let catalog =
        vec![component(definition(100), definition(101), 1, vec![attribute("category", "tube"), attribute("customer_part_number", "PIPE-001")])];
    suggest_catalog_matches(&tube_part(part_version_id, "PIPE-001"), &catalog)
}

/// The evaluation inputs of the synthetic dataset, in dataset-independent order.
fn synthetic_inputs() -> Vec<EvaluationInput> {
    vec![
        EvaluationInput {
            definition_id: definition(1),
            recognition: suggested(definition(1), "tube"),
            catalog: Some(matched_catalog_outcome(definition(1))),
        },
        EvaluationInput { definition_id: definition(2), recognition: suggested(definition(2), "tube"), catalog: None },
        EvaluationInput {
            definition_id: definition(3),
            recognition: unsupported(definition(3), &["recognition_unsupported", "extraction_unsupported_end_faces"]),
            catalog: None,
        },
        EvaluationInput { definition_id: definition(5), recognition: suggested(definition(5), "tube"), catalog: None },
    ]
}

fn synthetic_report() -> EvaluationReport {
    evaluate(&synthetic_dataset(), &synthetic_inputs()).expect("evaluate the synthetic dataset")
}

#[test]
fn the_report_counts_matched_mismatched_unsupported_and_missing() {
    let report = synthetic_report();
    assert_eq!(report.dataset_id, "synthetic-sb13-fixture");
    assert_eq!(report.dataset_provenance, "synthetic contract fixture, not customer data");
    assert_eq!(report.recognition_version, RECOGNITION_VERSION);
    assert_eq!(report.matched, 1);
    assert_eq!(report.mismatched, 2, "a wrong label and a proposal where the reference says unclassified");
    assert_eq!(report.unsupported, 1);
    assert_eq!(report.missing_results, 1);
    assert_eq!(report.outcomes.len(), 5, "every reference case is reported, in dataset order");
    let ids: Vec<&str> = report.outcomes.iter().map(|outcome| outcome.case_id.as_str()).collect();
    assert_eq!(ids, ["case-matched", "case-mismatched", "case-unsupported", "case-unclassified", "case-missing"]);

    assert_eq!(report.outcomes[0].classification, CaseClassification::Matched);
    assert_eq!(
        report.outcomes[1].classification,
        CaseClassification::Mismatched { expected: Some(category("wear_protection")), proposed: Some(category("tube")) }
    );
    assert_eq!(
        report.outcomes[2].classification,
        CaseClassification::Unsupported { codes: vec!["recognition_unsupported".to_string(), "extraction_unsupported_end_faces".to_string()] }
    );
    assert_eq!(report.outcomes[3].classification, CaseClassification::Mismatched { expected: None, proposed: Some(category("tube")) });
    assert_eq!(report.outcomes[4].classification, CaseClassification::Unsupported { codes: vec!["missing_result".to_string()] });
    assert_eq!(report.outcomes[4].catalog, CaseCatalog::NotEvaluated);
}

#[test]
fn catalog_outcomes_are_judged_per_case_only_where_the_reference_expects_one() {
    let report = synthetic_report();
    assert_eq!(report.outcomes[0].catalog, CaseCatalog::Matched, "the fixture matcher proposes the expected version");
    // The reference does not evaluate the catalog for this case, so no catalog comparison is claimed.
    assert_eq!(report.outcomes[1].catalog, CaseCatalog::NotEvaluated);
    assert_eq!(report.outcomes[2].catalog, CaseCatalog::NotEvaluated);

    let catalog =
        vec![component(definition(100), definition(101), 1, vec![attribute("category", "tube"), attribute("customer_part_number", "PIPE-001")])];
    let other_number_part = tube_part(definition(1), "OTHER-999");
    let mismatch = suggest_catalog_matches(&other_number_part, &catalog);
    assert!(mismatch.suggestions.is_empty());
    let dataset = ReferenceDataset {
        schema_version: REFERENCE_DATASET_SCHEMA_VERSION,
        dataset_id: "synthetic-catalog-fixture".to_string(),
        provenance: "synthetic contract fixture, not customer data".to_string(),
        cases: vec![ReferenceCase {
            case_id: "case-catalog".to_string(),
            definition_id: definition(1),
            expected_category: Some(category("tube")),
            expected_catalog_component_version_id: Some(definition(101)),
        }],
    };
    let inputs = vec![EvaluationInput { definition_id: definition(1), recognition: suggested(definition(1), "tube"), catalog: Some(mismatch) }];
    let report = evaluate(&dataset, &inputs).expect("evaluate a catalog case");
    assert_eq!(report.outcomes[0].catalog, CaseCatalog::Unsupported { codes: vec!["no_catalog_match".to_string()] });
}

#[test]
fn the_report_exposes_no_accuracy_ratio_or_percentage() {
    let report = synthetic_report();
    let json = serde_json::to_value(&report).expect("serialize the report");
    let object = json.as_object().expect("the report is an object");
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "dataset_id",
            "dataset_provenance",
            "limitations",
            "matched",
            "mismatched",
            "missing_results",
            "outcomes",
            "recognition_version",
            "unsupported"
        ]
    );
    for forbidden in ["accuracy", "accuracy_ratio", "ratio", "percentage", "percent", "precision", "recall", "f1", "score"] {
        assert!(!object.contains_key(forbidden), "the report must not expose `{forbidden}`: {json}");
    }
    assert!(
        report.limitations.iter().any(|limitation| limitation.contains("not an accuracy percentage")),
        "the report states the count is not an accuracy percentage: {:?}",
        report.limitations
    );
    assert!(
        report.limitations.iter().any(|limitation| limitation.contains("deployment claim")),
        "the report makes no deployment claim: {:?}",
        report.limitations
    );
}

#[test]
fn the_evaluation_is_deterministic_and_independent_of_input_order() {
    let dataset = synthetic_dataset();
    let inputs = synthetic_inputs();
    let first = evaluate(&dataset, &inputs).expect("evaluate");
    let second = evaluate(&dataset, &inputs).expect("evaluate again");
    assert_eq!(first, second);
    let mut reversed = inputs;
    reversed.reverse();
    assert_eq!(first, evaluate(&dataset, &reversed).expect("evaluate in the other input order"));
}

#[test]
fn an_invalid_reference_dataset_is_refused() {
    let inputs = synthetic_inputs();
    let mut future = synthetic_dataset();
    future.schema_version = REFERENCE_DATASET_SCHEMA_VERSION + 1;
    assert!(evaluate(&future, &inputs).is_err(), "a future dataset schema is refused");

    let mut no_provenance = synthetic_dataset();
    no_provenance.provenance = "   ".to_string();
    assert!(evaluate(&no_provenance, &inputs).is_err(), "a dataset must state where its labels came from");

    let mut no_id = synthetic_dataset();
    no_id.dataset_id = String::new();
    assert!(evaluate(&no_id, &inputs).is_err());

    let mut duplicate_case = synthetic_dataset();
    duplicate_case.cases[1].case_id = duplicate_case.cases[0].case_id.clone();
    assert!(evaluate(&duplicate_case, &inputs).is_err(), "case ids are unique");

    let dataset = synthetic_dataset();
    let duplicate_input = vec![
        EvaluationInput { definition_id: definition(1), recognition: suggested(definition(1), "tube"), catalog: None },
        EvaluationInput { definition_id: definition(1), recognition: suggested(definition(1), "tube"), catalog: None },
    ];
    assert!(evaluate(&dataset, &duplicate_input).is_err(), "one definition has at most one input");

    let wrong_definition = vec![EvaluationInput { definition_id: definition(1), recognition: suggested(definition(2), "tube"), catalog: None }];
    assert!(evaluate(&dataset, &wrong_definition).is_err(), "an input's result must describe its own definition");
}

#[test]
fn a_suggested_result_without_a_category_is_reported_as_unsupported() {
    let dataset = ReferenceDataset {
        schema_version: REFERENCE_DATASET_SCHEMA_VERSION,
        dataset_id: "synthetic-broken-result".to_string(),
        provenance: "synthetic contract fixture, not customer data".to_string(),
        cases: vec![ReferenceCase {
            case_id: "case-broken".to_string(),
            definition_id: definition(1),
            expected_category: Some(category("tube")),
            expected_catalog_component_version_id: None,
        }],
    };
    let mut broken = suggested(definition(1), "tube");
    broken.category = None;
    let report = evaluate(&dataset, &[EvaluationInput { definition_id: definition(1), recognition: broken, catalog: None }]).expect("evaluate");
    assert_eq!(report.unsupported, 1, "a broken suggestion is not counted as a match");
    assert_eq!(report.outcomes[0].classification, CaseClassification::Unsupported { codes: vec!["suggested_without_category".to_string()] });
}
