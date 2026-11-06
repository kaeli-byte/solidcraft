//! Extraction tests and the generator for the checked-in SolidBOM STEP fixtures.

use std::path::PathBuf;

use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use solidbom_contract::{AnalysisKey, AnalysisManifest, ReviewState, SourceRef};

use super::extract::{EXTRACTOR_VERSION, analysis_matches, engine_build, extract, resolve_pick};
use super::policy::ReviewPolicy;
use crate::{Sel, Session};

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/solidbom/fixtures")
}

fn fixture(name: &str) -> PathBuf {
    fixture_dir().join(name)
}

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session.execute(id, &params).expect(id)
}

fn sha256(path: &std::path::Path) -> String {
    hex(&std::fs::read(path).expect("read the fixture"))
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

fn open(path: &std::path::Path) -> Session {
    let mut session = Session::default();
    run(&mut session, "doc.open", json!({"path": path.to_string_lossy()}));
    session
}

fn key(path: &std::path::Path) -> AnalysisKey {
    AnalysisKey { source_sha256: sha256(path), engine_build: engine_build().to_string(), extractor_version: EXTRACTOR_VERSION.to_string() }
}

fn extracted_manifest(path: &std::path::Path) -> AnalysisManifest {
    let session = open(path);
    extract(&session, key(path)).expect("extract")
}

#[derive(Deserialize)]
struct FixtureManifest {
    fixtures: Vec<FixtureSpec>,
}

#[derive(Deserialize)]
struct FixtureSpec {
    source: String,
    sha256: String,
    metadata: Vec<MetadataSpec>,
    definitions: Vec<DefinitionSpec>,
    manifest_issues: Vec<String>,
}

#[derive(Deserialize)]
struct MetadataSpec {
    component: String,
    part_number: String,
    revision: String,
}

#[derive(Deserialize)]
struct DefinitionSpec {
    part_number: Option<String>,
    revision: Option<String>,
    volume_mm3: Option<String>,
    tolerance_mm3: Option<String>,
    issues: Vec<String>,
    occurrences: Vec<OccurrenceSpec>,
}

#[derive(Deserialize)]
struct OccurrenceSpec {
    source_path: Vec<u64>,
    transform: [[f64; 4]; 4],
}

fn spec(source: &str) -> FixtureSpec {
    let text = std::fs::read_to_string(fixture_dir().parent().expect("fixtures folder").join("manifest.json")).expect("read manifest.json");
    let manifest: FixtureManifest = serde_json::from_str(&text).expect("manifest.json");
    manifest.fixtures.into_iter().find(|fixture| fixture.source == source).expect("the fixture in manifest.json")
}

fn prepared(source: &str) -> (Session, FixtureSpec, AnalysisKey) {
    let spec = spec(source);
    let path = fixture(source);
    assert_eq!(sha256(&path), spec.sha256, "{source}: the checked-in file changed; update manifest.json");
    let mut session = open(&path);
    for metadata in &spec.metadata {
        run(
            &mut session,
            "component.metadata.set",
            json!({"component": metadata.component, "part_number": metadata.part_number, "revision": metadata.revision}),
        );
    }
    let key = key(&path);
    (session, spec, key)
}

fn same_transform(a: [[f64; 4]; 4], b: [[f64; 4]; 4]) -> bool {
    a.iter().flatten().zip(b.iter().flatten()).all(|(left, right)| (left - right).abs() <= 1e-6 * (1.0 + right.abs()))
}

fn decimal(text: &str) -> Decimal {
    text.parse().expect("a decimal in manifest.json")
}

fn definition_issues(manifest: &AnalysisManifest, definition_id: uuid::Uuid) -> Vec<String> {
    manifest.issues.iter().filter(|issue| issue.definition_id == Some(definition_id)).map(|issue| issue.code.clone()).collect()
}

/// The occurrence that places the body's component; flat imports have no `body_components` entry.
fn placing_occurrence(session: &Session) -> u64 {
    let state = session.model.state();
    let component = session.doc.model_body_components(&state)[0];
    session.doc.occurrence_of(component).expect("the import placement").id
}

#[test]
fn three_boxes_match_the_authored_manifest() {
    let (session, spec, key) = prepared("three_boxes.step");
    let manifest = extract(&session, key).expect("extract");
    assert_eq!(manifest.definitions.len(), 1);
    assert_eq!(manifest.occurrences.len(), 3);
    assert_eq!(manifest.geometry_bindings.len(), 3);
    assert!(manifest.issues.is_empty(), "{:?}", manifest.issues);
    let definition = &manifest.definitions[0];
    let expected = &spec.definitions[0];
    assert_eq!(definition_issues(&manifest, definition.definition_id), expected.issues);
    assert_eq!(definition.customer_part_number.as_deref(), expected.part_number.as_deref());
    assert_eq!(definition.customer_revision.as_deref(), expected.revision.as_deref());
    assert_eq!(definition.volume_mm3.review, ReviewState::Pending);
    let volume = definition.volume_mm3.value.expect("the box volume");
    let tolerance = expected.tolerance_mm3.as_deref().map(decimal).unwrap_or(Decimal::ZERO);
    assert!((volume - decimal(expected.volume_mm3.as_deref().expect("expected volume"))).abs() <= tolerance, "{volume}");
    for expected in &expected.occurrences {
        let occurrence =
            manifest.occurrences.iter().find(|occurrence| occurrence.source_path == expected.source_path).expect("the authored occurrence");
        assert_eq!(occurrence.definition_id, definition.definition_id);
        assert!(same_transform(occurrence.transform, expected.transform), "{:?}", occurrence.transform);
    }
}

#[test]
fn six_selections_resolve_to_the_same_shared_part() {
    let (session, _, key) = prepared("three_boxes.step");
    let manifest = extract(&session, key).expect("extract");
    let definition = &manifest.definitions[0];
    let component = session.doc.body_components.get("Clip").copied().expect("the imported part");
    let occurrences: Vec<u64> =
        session.doc.occurrences.iter().filter(|occurrence| occurrence.component == component).map(|occurrence| occurrence.id).collect();
    assert_eq!(occurrences.len(), 3);
    for (index, occurrence) in occurrences.iter().enumerate() {
        // The tree and the viewport both produce an occurrence selection; both must bind alike.
        for _ in 0..2 {
            let found = resolve_pick(&session, &manifest, &Sel::Occurrence { path: vec![*occurrence] }).expect("resolve");
            assert_eq!(found.definition_id, definition.definition_id);
            assert_eq!(found.source_path, vec![index as u64]);
        }
    }
    // A body pick cannot choose between three placements.
    let ambiguous = resolve_pick(&session, &manifest, &Sel::Body { name: "Clip".into() });
    assert!(ambiguous.is_err());
}

#[test]
fn an_open_surface_has_no_volume() {
    let (session, spec, key) = prepared("open_surface.step");
    let manifest = extract(&session, key).expect("extract");
    assert_eq!(manifest.definitions.len(), 1);
    let definition = &manifest.definitions[0];
    assert_eq!(definition.volume_mm3.value, None);
    let issues: Vec<&str> = manifest.issues.iter().map(|issue| issue.code.as_str()).collect();
    for expected in &spec.manifest_issues {
        assert!(issues.contains(&expected.as_str()), "{issues:?}");
    }
    assert_eq!(definition_issues(&manifest, definition.definition_id), spec.definitions[0].issues);
    let found =
        resolve_pick(&session, &manifest, &Sel::Occurrence { path: vec![placing_occurrence(&session)] }).expect("resolve the single occurrence");
    assert_eq!(found.definition_id, definition.definition_id);
}

#[test]
fn two_customer_revisions_stay_separate() {
    let (session, spec, key) = prepared("revisions_bc.step");
    let manifest = extract(&session, key).expect("extract");
    assert_eq!(manifest.definitions.len(), 2);
    assert_eq!(spec.definitions.len(), 2);
    for expected in &spec.definitions {
        let definition = manifest
            .definitions
            .iter()
            .find(|definition| {
                definition.customer_part_number.as_deref() == expected.part_number.as_deref()
                    && definition.customer_revision.as_deref() == expected.revision.as_deref()
            })
            .expect("the authored revision");
        let expected_volume = decimal(expected.volume_mm3.as_deref().expect("expected volume"));
        let tolerance = expected.tolerance_mm3.as_deref().map(decimal).unwrap_or(Decimal::ZERO);
        let volume = definition.volume_mm3.value.expect("volume");
        assert!((volume - expected_volume).abs() <= tolerance, "{volume}");
        assert_eq!(definition_issues(&manifest, definition.definition_id), expected.issues);
    }
    assert_ne!(manifest.definitions[0].definition_id, manifest.definitions[1].definition_id);
    for (component_name, expected_revision) in [("ClipB", "B"), ("ClipC", "C")] {
        let component = session.doc.body_components.get(component_name).copied().expect("the imported revision");
        let occurrence = session.doc.occurrences.iter().find(|occurrence| occurrence.component == component).expect("one placement");
        let found = resolve_pick(&session, &manifest, &Sel::Occurrence { path: vec![occurrence.id] }).expect("resolve");
        let definition = manifest.definitions.iter().find(|definition| definition.definition_id == found.definition_id).expect("the definition");
        assert_eq!(definition.customer_revision.as_deref(), Some(expected_revision));
    }
}

#[test]
fn a_flat_input_stays_review_required() {
    let (session, spec, key) = prepared("flat_multi_body.step");
    let manifest = extract(&session, key).expect("extract");
    let issues: Vec<&str> = manifest.issues.iter().map(|issue| issue.code.as_str()).collect();
    for expected in &spec.manifest_issues {
        assert!(issues.contains(&expected.as_str()), "{issues:?}");
    }
    assert_eq!(manifest.definitions.len(), 1);
    assert_eq!(manifest.geometry_bindings.len(), 3);
    assert_eq!(definition_issues(&manifest, manifest.definitions[0].definition_id), spec.definitions[0].issues);
    let found = resolve_pick(&session, &manifest, &Sel::Occurrence { path: vec![placing_occurrence(&session)] }).expect("the one placement resolves");
    assert_eq!(found.source_path, Vec::<u64>::new());
}

#[test]
fn a_second_import_restores_the_same_mapping() {
    let path = fixture("three_boxes.step");
    let key = key(&path);
    let first = extract(&open(&path), key.clone()).expect("extract");
    let second = extract(&open(&path), key).expect("extract again");
    assert_eq!(first, second);
}

#[test]
fn a_changed_analysis_key_does_not_match() {
    let path = fixture("three_boxes.step");
    let session = open(&path);
    let key = key(&path);
    let manifest = extract(&session, key.clone()).expect("extract");
    assert!(analysis_matches(&manifest, &key));
    let mut other = key.clone();
    other.extractor_version = "a later extractor".into();
    let changed = extract(&session, other.clone()).expect("extract with the other key");
    assert!(!analysis_matches(&manifest, &other));
    // The ids follow the source path, so an unchanged source restores the same identities.
    assert_eq!(manifest.definitions[0].definition_id, changed.definitions[0].definition_id);
    assert_ne!(manifest.key, changed.key);
}

#[test]
fn extraction_leaves_the_source_file_unchanged() {
    let path = fixture("three_boxes.step");
    let before = sha256(&path);
    let _ = extracted_manifest(&path);
    assert_eq!(sha256(&path), before);
}

#[test]
#[ignore = "writes the checked-in SolidBOM STEP fixtures"]
fn write_fixtures() {
    let dir = fixture_dir();
    std::fs::create_dir_all(&dir).expect("create the fixture folder");

    let mut three = Session::default();
    run(&mut three, "solid.box", json!({"length": 40, "width": 20, "height": 3, "body_name": "Clip"}));
    let created = run(&mut three, "component.from_bodies", json!({"bodies": ["Clip"]}));
    let component = created["components"][0]["component"].as_u64().expect("one component");
    run(&mut three, "occurrence.copy", json!({"component": component, "translate": [50, 0, 0]}));
    run(&mut three, "occurrence.copy", json!({"component": component, "translate": [100, 0, 0]}));
    run(&mut three, "component.metadata.set", json!({"component": component, "part_number": "PL-100", "revision": "B"}));
    run(&mut three, "file.export", json!({"path": fixture("three_boxes.step").to_string_lossy()}));

    let mut surface = Session::default();
    run(&mut surface, "sketch.create", json!({"plane": "XY", "name": "Outline"}));
    run(&mut surface, "sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [40, 20]}));
    run(&mut surface, "sketch.finish", json!({}));
    run(&mut surface, "surface.extrude", json!({"sketch": "Outline", "distance": 10, "body_name": "OpenSurface"}));
    run(&mut surface, "file.export", json!({"path": fixture("open_surface.step").to_string_lossy()}));

    let mut revisions = Session::default();
    run(&mut revisions, "solid.box", json!({"length": 40, "width": 20, "height": 3, "body_name": "ClipB"}));
    let b = run(&mut revisions, "component.from_bodies", json!({"bodies": ["ClipB"]}));
    let component_b = b["components"][0]["component"].as_u64().expect("component B");
    run(&mut revisions, "component.metadata.set", json!({"component": component_b, "part_number": "PL-100", "revision": "B"}));
    run(&mut revisions, "solid.box", json!({"length": 40, "width": 20, "height": 6, "body_name": "ClipC"}));
    let c = run(&mut revisions, "component.from_bodies", json!({"bodies": ["ClipC"]}));
    let component_c = c["components"][0]["component"].as_u64().expect("component C");
    run(&mut revisions, "component.metadata.set", json!({"component": component_c, "part_number": "PL-100", "revision": "C"}));
    run(&mut revisions, "file.export", json!({"path": fixture("revisions_bc.step").to_string_lossy()}));

    let mut flat = Session::default();
    for (name, x) in [("A", 0), ("B", 50), ("C", 100)] {
        run(&mut flat, "solid.box", json!({"length": 40, "width": 20, "height": 3, "corner": [x, 0, 0], "operation": "new", "body_name": name}));
    }
    run(&mut flat, "file.export", json!({"path": fixture("flat_multi_body.step").to_string_lossy()}));

    for name in ["three_boxes.step", "open_surface.step", "revisions_bc.step", "flat_multi_body.step"] {
        println!("{name} {}", sha256(&fixture(name)));
    }
}

#[test]
#[ignore = "prints the extracted fixture summaries while they are authored"]
fn inspect_fixtures() {
    for name in ["three_boxes.step", "open_surface.step", "revisions_bc.step", "flat_multi_body.step"] {
        let path = fixture(name);
        let manifest = extracted_manifest(&path);
        println!(
            "{name}: {} definitions, {} occurrences, {} bindings, issues {:?}",
            manifest.definitions.len(),
            manifest.occurrences.len(),
            manifest.geometry_bindings.len(),
            manifest.issues.iter().map(|issue| issue.code.as_str()).collect::<Vec<_>>()
        );
        for definition in &manifest.definitions {
            println!("  def {} volume {:?}", definition.definition_id, definition.volume_mm3.value);
        }
        for occurrence in &manifest.occurrences {
            println!("  occ {} path {:?}", occurrence.occurrence_id, occurrence.source_path);
        }
    }
}

fn reviewed_source() -> SourceRef {
    SourceRef { source_id: uuid::Uuid::nil(), sha256: "a".repeat(64), byte_len: 1, original_name: "three_boxes.step".into() }
}

fn bound_session() -> Session {
    let mut session = Session::default();
    run(&mut session, "solid.box", json!({"length": 10, "width": 10, "height": 10}));
    session.bind_review(ReviewPolicy::new(reviewed_source(), key(&fixture("three_boxes.step"))));
    session
}

#[test]
fn reviewed_geometry_is_read_only() {
    let mut session = bound_session();
    let document = session.doc.clone();
    let revision = session.revision;
    let undo = session.undo.len();
    for id in [
        "solid.extrude",
        "component.create",
        "occurrence.reparent",
        "occurrence.move",
        "occurrence.copy",
        "edit.undo",
        "edit.redo",
        "file.new",
        "doc.open",
        "file.insert_step",
        "component.metadata.set",
        "document.metadata.set",
        "file.save",
        "parameters.change",
    ] {
        let error = session.execute(id, &json!({})).expect_err(id);
        assert!(matches!(error, crate::EngineError::Refused { .. }), "{id}: {error}");
    }
    assert!(std::sync::Arc::ptr_eq(&document, &session.doc));
    assert_eq!(session.revision, revision);
    assert_eq!(session.undo.len(), undo);
}

#[test]
fn permitted_reads_and_a_local_save_still_work() {
    let mut session = bound_session();
    for (id, params) in [
        ("inspect.measure", json!({})),
        ("assembly.bom", json!({})),
        ("document.inspect", json!({})),
        ("engine.commands", json!({})),
        ("component.metadata", json!({"component": 0})),
        ("component.properties", json!({"component": 0})),
    ] {
        session.execute(id, &params).unwrap_or_else(|error| panic!("{id}: {error}"));
    }
    let dir = std::env::temp_dir().join(format!("solidbom-review-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a temporary folder");
    let path = dir.join("review.sc");
    session.execute("file.save_as", &json!({"path": path.to_string_lossy()})).expect("the local CAD save is allowed");
    assert!(path.exists());
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir(&dir);
}

#[test]
fn a_script_and_a_nested_run_cannot_bypass_the_policy() {
    let mut session = bound_session();
    let script = json!([{"command": "solid.extrude", "params": {}}]);
    let error = session.run_script(&script).expect_err("the script is refused");
    assert!(error.to_string().contains("not allowed"), "{error}");
    let spec = crate::cmd::find_command("solid.extrude").expect("the command");
    let error = spec.run_nested(&mut session, &json!({})).expect_err("the nested run is refused");
    assert!(matches!(error, crate::EngineError::Refused { .. }), "{error}");
}

#[test]
fn the_policy_survives_copies_and_document_replacement() {
    let mut session = bound_session();
    let copy = session.scratch();
    assert!(!copy.review_allows("solid.extrude"));
    session.replace_design(Session::default());
    assert!(!session.review_allows("solid.extrude"));
    let error = session.execute("solid.extrude", &json!({})).expect_err("still bound");
    assert!(matches!(error, crate::EngineError::Refused { .. }), "{error}");
}

#[test]
fn an_unbound_session_still_models() {
    let mut session = Session::default();
    session.execute("solid.box", &json!({"length": 10, "width": 10, "height": 10})).expect("modeling works");
    assert!(session.review_allows("solid.extrude"));
}
