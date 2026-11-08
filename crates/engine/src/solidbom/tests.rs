//! Extraction tests and the generator for the checked-in SolidBOM STEP fixtures.

use std::path::PathBuf;

use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use solidbom_contract::{AnalysisKey, AnalysisManifest, ReviewState, SourceRef};

use super::extract::{EXTRACTOR_VERSION, RuntimeOccurrenceMap, analysis_matches, engine_build, extract, resolve_definition_pick, resolve_pick};
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
    /// The source path of the occurrence's parent, when the manifest expects one.
    #[serde(default)]
    parent_path: Option<Vec<u64>>,
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
fn a_partly_measurable_definition_has_no_volume() {
    let (session, spec, key) = prepared("mixed_open_closed.step");
    let manifest = extract(&session, key).expect("extract");
    let expected = &spec.definitions[0];
    assert_eq!(manifest.definitions.len(), 1);
    assert_eq!(manifest.occurrences.len(), 1);
    assert_eq!(manifest.geometry_bindings.len(), 2, "the closed box and the open surface");
    let definition = &manifest.definitions[0];
    assert_eq!(definition.volume_mm3.value, None, "a partial closed-body sum is not the definition's volume");
    assert_eq!(definition.volume_mm3.review, ReviewState::Pending);
    assert_eq!(definition.volume_mm3.value, expected.volume_mm3.as_deref().map(decimal));
    assert_eq!(definition_issues(&manifest, definition.definition_id), expected.issues);
    let codes: Vec<&str> = manifest.issues.iter().map(|issue| issue.code.as_str()).collect();
    for expected_code in &spec.manifest_issues {
        assert!(codes.contains(&expected_code.as_str()), "{codes:?}");
    }
    assert!(!codes.contains(&"unresolved_grouping"), "the mixed part sits in a component: {codes:?}");
    for issue in &manifest.issues {
        assert_eq!(issue.field_path, "analysis.definitions.volume_mm3");
    }
    assert_eq!(manifest.occurrences[0].source_path, expected.occurrences[0].source_path);
    let found = resolve_pick(&session, &manifest, &Sel::Occurrence { path: vec![placing_occurrence(&session)] }).expect("resolve the one placement");
    assert_eq!(found.definition_id, definition.definition_id);
}

#[test]
fn nested_occurrences_compose_and_share_one_definition() {
    let (session, spec, key) = prepared("nested_repeated.step");
    let manifest = extract(&session, key).expect("extract");
    assert_eq!(manifest.definitions.len(), spec.definitions.len());
    assert_eq!(manifest.occurrences.len(), 6);
    assert_eq!(manifest.geometry_bindings.len(), 6);
    assert!(manifest.issues.is_empty(), "{:?}", manifest.issues);
    let map = RuntimeOccurrenceMap::new(&session).expect("the runtime/source map");
    let mut clip = None;
    for expected in &spec.definitions {
        let definition = manifest
            .definitions
            .iter()
            .find(|definition| {
                definition.customer_part_number.as_deref() == expected.part_number.as_deref()
                    && definition.customer_revision.as_deref() == expected.revision.as_deref()
            })
            .expect("the authored definition");
        let volume = definition.volume_mm3.value.expect("a measurable definition");
        let expected_volume = decimal(expected.volume_mm3.as_deref().expect("expected volume"));
        let tolerance = expected.tolerance_mm3.as_deref().map(decimal).unwrap_or(Decimal::ZERO);
        assert!((volume - expected_volume).abs() <= tolerance, "{volume}");
        assert_eq!(definition_issues(&manifest, definition.definition_id), expected.issues);
        if expected.part_number.as_deref() == Some("PL-200") {
            clip = Some(definition.definition_id);
        }
        for expected_occurrence in &expected.occurrences {
            let occurrence = manifest
                .occurrences
                .iter()
                .find(|occurrence| occurrence.source_path == expected_occurrence.source_path)
                .expect("the authored occurrence");
            assert_eq!(occurrence.definition_id, definition.definition_id, "{:?}", expected_occurrence.source_path);
            assert!(
                same_transform(occurrence.transform, expected_occurrence.transform),
                "{:?}: {:?}",
                expected_occurrence.source_path,
                occurrence.transform
            );
            let parent = expected_occurrence
                .parent_path
                .as_ref()
                .map(|path| manifest.occurrences.iter().find(|occurrence| &occurrence.source_path == path).expect("the authored parent occurrence"));
            assert_eq!(occurrence.parent_occurrence_id, parent.map(|occurrence| occurrence.occurrence_id), "{:?}", expected_occurrence.source_path);
            let runtime = map.runtime_occurrence(&occurrence.source_path).expect("the runtime occurrence");
            let picked = resolve_pick(&session, &manifest, &Sel::Occurrence { path: vec![runtime] }).expect("resolve the occurrence");
            assert_eq!(picked.occurrence_id, occurrence.occurrence_id);
            assert_eq!(picked.source_path, occurrence.source_path);
        }
    }
    let clip = clip.expect("the shared clip definition");
    let placements: Vec<_> = manifest.occurrences.iter().filter(|occurrence| occurrence.definition_id == clip).collect();
    assert_eq!(placements.len(), 4, "one shared definition: twice under Frame, once under Cover, once at the root");
    assert_eq!(
        placements.iter().filter(|occurrence| occurrence.parent_occurrence_id.is_some()).count(),
        3,
        "three placements sit below a subassembly"
    );
    // The body belongs to the part placed four times, so a body pick cannot choose an occurrence.
    let body_of = |component_name: &str| {
        let component = session.doc.components.iter().find(|component| component.name == component_name).expect("the imported component").id;
        session.doc.body_components.iter().find(|(_, owner)| **owner == component).map(|(body, _)| body.clone()).expect("the imported body")
    };
    assert!(resolve_pick(&session, &manifest, &Sel::Body { name: body_of("Clip") }).is_err());
    let frame_pick = resolve_pick(&session, &manifest, &Sel::Body { name: body_of("Frame") }).expect("the frame is placed once");
    assert_eq!(frame_pick.source_path, vec![0]);
    // The import's own component is not a reviewed definition, so it never picks.
    let root_occurrence =
        session.doc.occurrences.iter().find(|occurrence| occurrence.parent == 0).map(|occurrence| occurrence.id).expect("the import placement");
    assert!(resolve_pick(&session, &manifest, &Sel::Occurrence { path: vec![root_occurrence] }).is_err());
}

#[test]
fn a_definition_only_pick_never_invents_an_occurrence() {
    let (session, _, key) = prepared("nested_repeated.step");
    let manifest = extract(&session, key).expect("extract");
    let clip = manifest
        .definitions
        .iter()
        .find(|definition| definition.customer_part_number.as_deref() == Some("PL-200"))
        .expect("the shared clip definition");
    let pick = resolve_definition_pick(&manifest, clip.definition_id).expect("the definition pick");
    assert_eq!(pick.definition.definition_id, clip.definition_id);
    assert_eq!(pick.definition.customer_revision.as_deref(), Some("B"));
    assert_eq!(pick.occurrences.len(), 4);
    assert!(pick.occurrences.iter().all(|occurrence| occurrence.definition_id == clip.definition_id));
    assert!(
        manifest.occurrences.iter().all(|occurrence| occurrence.occurrence_id != pick.definition.definition_id),
        "a definition is not an occurrence"
    );
    let unknown = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, b"not a definition");
    assert!(resolve_definition_pick(&manifest, unknown).is_err());
}

#[test]
fn display_quality_never_changes_extraction() {
    use solidcraft_doc::display_quality::{DisplayQuality, set_current};
    let path = fixture("three_boxes.step");
    let (session, _, key) = prepared("three_boxes.step");
    let before = sha256(&path);
    let mut manifests = Vec::new();
    for quality in [DisplayQuality::Coarse, DisplayQuality::Ultra] {
        set_current(quality);
        let manifest = extract(&session, key.clone()).expect("extract");
        let volume = manifest.definitions[0].volume_mm3.value.expect("the exact volume");
        assert!((volume - decimal("2400")).abs() <= decimal("2.4"), "{quality:?}: {volume}");
        manifests.push(manifest);
    }
    set_current(DisplayQuality::Normal);
    assert_eq!(manifests[0], manifests[1], "the business extraction is identical at any display quality");
    assert_eq!(manifests[0].occurrences.len(), 3);
    assert_eq!(sha256(&path), before, "extraction never rewrites the source");
}

#[test]
fn stale_source_keys_and_analyses_are_refused() {
    let path = fixture("three_boxes.step");
    let session = open(&path);
    let key = key(&path);
    let manifest = extract(&session, key.clone()).expect("extract");
    assert!(analysis_matches(&manifest, &key));
    for changed in [
        AnalysisKey { engine_build: "another engine build".into(), ..key.clone() },
        AnalysisKey { extractor_version: "another extractor".into(), ..key.clone() },
        AnalysisKey { source_sha256: "b".repeat(64), ..key.clone() },
    ] {
        assert!(!analysis_matches(&manifest, &changed), "{changed:?}");
    }

    // A definition or measurement bound to another analysis is refused by the manifest.
    let mut other = key.clone();
    other.engine_build = "another engine build".into();
    other.extractor_version = "another extractor".into();
    let mut bound_definition = manifest.clone();
    bound_definition.definitions[0].source_key = other.clone();
    assert!(bound_definition.validate().is_err());
    let mut bound_volume = manifest.clone();
    bound_volume.definitions[0].volume_mm3.analysis_key = other.clone();
    assert!(bound_volume.validate().is_err());
    let mut bound_key = manifest.clone();
    bound_key.key = other.clone();
    assert!(bound_key.validate().is_err());

    // A pick refuses an analysis that does not describe the session's reviewed source.
    let mut reviewed = open(&path);
    reviewed.bind_review(ReviewPolicy::new(reviewed_source(), key.clone()));
    let map = RuntimeOccurrenceMap::new(&reviewed).expect("the runtime/source map");
    let occurrence = map.runtime_occurrence(&[0]).expect("a runtime occurrence");
    let selection = Sel::Occurrence { path: vec![occurrence] };
    assert!(resolve_pick(&reviewed, &manifest, &selection).is_ok());
    let stale = extract(&reviewed, other).expect("extraction alone does not refuse the key");
    assert_eq!(stale.occurrences, manifest.occurrences, "the stale analysis still names the same placements");
    let error = resolve_pick(&reviewed, &stale, &selection).expect_err("a stale analysis must not resolve a pick");
    assert!(error.to_string().contains("reviewed source"), "{error}");
}

#[test]
fn runtime_ids_change_but_the_source_map_is_stable() {
    let path = fixture("nested_repeated.step");
    let key = key(&path);
    let first = open(&path);
    let first_manifest = extract(&first, key.clone()).expect("extract");
    let first_map = RuntimeOccurrenceMap::new(&first).expect("the first map");

    // Import the same bytes into a session whose generated ids are not fresh, as a re-import is.
    let mut second = Session::default();
    run(&mut second, "solid.box", json!({"length": 5, "width": 5, "height": 5}));
    run(&mut second, "file.insert_step", json!({"path": path.to_string_lossy()}));
    let second_manifest = extract(&second, key).expect("extract");
    assert_eq!(second_manifest, first_manifest, "the analysis follows the source, not the runtime ids");
    let second_map = RuntimeOccurrenceMap::new(&second).expect("the second map");

    let mut differed = false;
    for occurrence in &first_manifest.occurrences {
        let runtime_first = first_map.runtime_occurrence(&occurrence.source_path).expect("the first runtime occurrence");
        let runtime_second = second_map.runtime_occurrence(&occurrence.source_path).expect("the second runtime occurrence");
        differed |= runtime_first != runtime_second;
        assert_eq!(first_map.source_path(runtime_first).expect("the first source path"), occurrence.source_path);
        assert_eq!(second_map.source_path(runtime_second).expect("the second source path"), occurrence.source_path);
        let picked_first = resolve_pick(&first, &first_manifest, &Sel::Occurrence { path: vec![runtime_first] }).expect("the first pick");
        let picked_second = resolve_pick(&second, &second_manifest, &Sel::Occurrence { path: vec![runtime_second] }).expect("the second pick");
        assert_eq!(picked_first.occurrence_id, occurrence.occurrence_id);
        assert_eq!(picked_second.occurrence_id, occurrence.occurrence_id);
    }
    assert!(differed, "the second import must allocate different runtime ids");
    assert!(first_map.source_path(u64::MAX).is_err(), "an unknown runtime id is an error");
    assert!(first_map.runtime_occurrence(&[9_999]).is_err(), "an unknown source path is an error");
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

    // A definition that is only partly measurable: one closed box and one open surface in the
    // same component, so neither the box volume nor a partial sum may stand as the volume.
    let mut mixed = Session::default();
    run(&mut mixed, "component.create", json!({"name": "Mixed"}));
    run(&mut mixed, "solid.box", json!({"length": 40, "width": 20, "height": 3, "body_name": "MixedBox"}));
    run(&mut mixed, "sketch.create", json!({"plane": "XY", "name": "MixedOutline"}));
    run(&mut mixed, "sketch.rectangle.two_point", json!({"p0": [100, 0], "p1": [140, 20]}));
    run(&mut mixed, "sketch.finish", json!({}));
    run(&mut mixed, "surface.extrude", json!({"sketch": "MixedOutline", "distance": 10, "body_name": "MixedSurface"}));
    run(&mut mixed, "file.export", json!({"path": fixture("mixed_open_closed.step").to_string_lossy()}));

    // A nested assembly: a Frame subassembly (60 x 20 x 5 plate) holding two placements of a
    // 40 x 20 x 3 Clip, a Cover subassembly (60 x 20 x 6 plate) holding one Clip, the Frame turned
    // a quarter turn and both subassemblies placed away from the origin, and one Clip placed
    // directly at the root. The Clip is one shared definition placed four times.
    let mut nested = Session::default();
    let frame = run(&mut nested, "component.create", json!({"name": "Frame", "grounded": false}));
    let frame_occurrence = frame["occurrence"].as_u64().expect("the frame placement");
    run(&mut nested, "solid.box", json!({"length": 60, "width": 20, "height": 5, "body_name": "FramePlate"}));
    run(&mut nested, "component.create", json!({"name": "Clip", "parent": "Frame"}));
    run(&mut nested, "solid.box", json!({"length": 40, "width": 20, "height": 3, "body_name": "NestedClip"}));
    run(&mut nested, "component.activate", json!({"component": 0}));
    run(&mut nested, "occurrence.move", json!({"occurrence": frame_occurrence, "translate": [10, 0, 0], "axis": [0, 0, 1], "angle": "90 deg"}));
    run(&mut nested, "occurrence.copy", json!({"component": "Clip", "parent": "Frame", "translate": [50, 0, 0]}));
    let cover = run(&mut nested, "component.create", json!({"name": "Cover", "grounded": false}));
    let cover_occurrence = cover["occurrence"].as_u64().expect("the cover placement");
    run(&mut nested, "solid.box", json!({"length": 60, "width": 20, "height": 6, "body_name": "CoverPlate"}));
    run(&mut nested, "component.activate", json!({"component": 0}));
    run(&mut nested, "occurrence.move", json!({"occurrence": cover_occurrence, "translate": [0, 100, 0]}));
    run(&mut nested, "occurrence.copy", json!({"component": "Clip", "parent": "Cover", "translate": [20, 0, 0]}));
    run(&mut nested, "occurrence.copy", json!({"component": "Clip", "parent": 0, "translate": [0, -50, 0]}));
    run(&mut nested, "file.export", json!({"path": fixture("nested_repeated.step").to_string_lossy()}));

    for name in
        ["three_boxes.step", "open_surface.step", "revisions_bc.step", "flat_multi_body.step", "mixed_open_closed.step", "nested_repeated.step"]
    {
        println!("{name} {}", sha256(&fixture(name)));
    }
}

#[test]
#[ignore = "prints the extracted fixture summaries while they are authored"]
fn inspect_fixtures() {
    for name in
        ["three_boxes.step", "open_surface.step", "revisions_bc.step", "flat_multi_body.step", "mixed_open_closed.step", "nested_repeated.step"]
    {
        let path = fixture(name);
        let manifest = extracted_manifest(&path);
        println!(
            "{name} sha256 {}: {} definitions, {} occurrences, {} bindings, issues {:?}",
            sha256(&path),
            manifest.definitions.len(),
            manifest.occurrences.len(),
            manifest.geometry_bindings.len(),
            manifest.issues.iter().map(|issue| issue.code.as_str()).collect::<Vec<_>>()
        );
        for definition in &manifest.definitions {
            println!(
                "  def {} part {:?} rev {:?} volume {:?} issues {:?}",
                definition.definition_id,
                definition.customer_part_number,
                definition.customer_revision,
                definition.volume_mm3.value,
                definition_issues(&manifest, definition.definition_id)
            );
        }
        for occurrence in &manifest.occurrences {
            println!(
                "  occ path {:?} parent {:?} definition {} transform {:?}",
                occurrence.source_path, occurrence.parent_occurrence_id, occurrence.definition_id, occurrence.transform
            );
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

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use solidbom_contract::{
    CalculationInputs, CalculationOutcome, CalculationSnapshot, HostCapabilities, HostError, JobEnvelope, JobRequest, ReferenceData,
    ResultState as OutcomeState, SaveDraft, SaveReceipt,
};

use super::host::{ClientFailure, ClientReply, ClientRequest, JobCompletion, JobId, SolidBomHost};
use super::state::{ResultState as ControllerResult, SaveState};

#[derive(Default)]
struct FakeHost {
    next: AtomicU64,
    submitted: Mutex<Vec<(JobId, JobRequest)>>,
    envelopes: Mutex<BTreeMap<JobId, JobEnvelope>>,
    completions: Mutex<Vec<JobCompletion>>,
    reject: Mutex<Option<HostError>>,
    capabilities: Mutex<HostCapabilities>,
    cancelled: Mutex<Vec<JobId>>,
}

impl FakeHost {
    fn take(&self) -> Vec<(JobId, JobRequest)> {
        self.submitted.lock().map(|mut list| std::mem::take(&mut *list)).unwrap_or_default()
    }

    fn complete(&self, job: JobId, result: Result<ClientReply, ClientFailure>) {
        let envelope = self.envelopes.lock().ok().and_then(|mut map| map.remove(&job)).unwrap_or_default();
        if let Ok(mut list) = self.completions.lock() {
            list.push(JobCompletion { job_id: job, envelope, result });
        }
    }

    fn complete_with_envelope(&self, job: JobId, envelope: JobEnvelope, result: Result<ClientReply, ClientFailure>) {
        if let Ok(mut list) = self.completions.lock() {
            list.push(JobCompletion { job_id: job, envelope, result });
        }
    }

    fn reject_with(&self, error: HostError) {
        if let Ok(mut reject) = self.reject.lock() {
            *reject = Some(error);
        }
    }

    fn set_capabilities(&self, capabilities: HostCapabilities) {
        if let Ok(mut current) = self.capabilities.lock() {
            *current = capabilities;
        }
    }

    fn cancelled_jobs(&self) -> Vec<JobId> {
        self.cancelled.lock().map(|jobs| jobs.clone()).unwrap_or_default()
    }
}

impl SolidBomHost for FakeHost {
    fn submit(&self, job: JobRequest) -> Result<JobId, HostError> {
        if let Ok(mut reject) = self.reject.lock()
            && let Some(error) = reject.take()
        {
            return Err(error);
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        if let Ok(mut envelopes) = self.envelopes.lock() {
            envelopes.insert(id, job.envelope.clone());
        }
        if let Ok(mut list) = self.submitted.lock() {
            list.push((id, job));
        }
        Ok(id)
    }

    fn poll(&self) -> Vec<JobCompletion> {
        self.completions.lock().map(|mut list| std::mem::take(&mut *list)).unwrap_or_default()
    }

    fn cancel(&self, job_id: JobId) -> solidbom_contract::CancelOutcome {
        if let Ok(mut cancelled) = self.cancelled.lock() {
            cancelled.push(job_id);
        }
        solidbom_contract::CancelOutcome::Cancelled
    }

    fn capabilities(&self) -> HostCapabilities {
        self.capabilities.lock().map(|capabilities| *capabilities).unwrap_or_default()
    }
}

fn fake_session() -> (Session, Arc<FakeHost>) {
    let fake = Arc::new(FakeHost::default());
    let session = Session { solidbom_host: Some(fake.clone()), ..Default::default() };
    (session, fake)
}

fn import_three_boxes(session: &mut Session, fake: &Arc<FakeHost>) {
    let path = fixture("three_boxes.step");
    session.execute("solidbom.import", &json!({"path": path.to_string_lossy()})).expect("import");
    let uploads = fake.take();
    let (job, _) = uploads.iter().find(|(_, job)| matches!(&job.request, ClientRequest::UploadSource { .. })).expect("the upload");
    let source = session.solidbom.draft.as_ref().expect("the draft").source.clone();
    fake.complete(*job, Ok(ClientReply::Source(source)));
    session.drain_solidbom();
    let created = fake.take();
    let (create_job, create) = created
        .iter()
        .find_map(|(job, submitted)| match &submitted.request {
            ClientRequest::CreateProject(save) => Some((*job, save.clone())),
            _ => None,
        })
        .expect("create");
    let receipt = SaveReceipt { request_id: create.request_id.clone(), project_id: create.draft.project_id, version: 1 };
    fake.complete(create_job, Ok(ClientReply::Receipt(receipt)));
    if let Some((job, _)) = created.iter().find(|(_, job)| matches!(&job.request, ClientRequest::ReferenceData)) {
        fake.complete(*job, Ok(ClientReply::ReferenceData(ReferenceData { materials: Vec::new(), rates: Vec::new() })));
    }
    session.drain_solidbom();
}

fn edit(session: &mut Session) {
    let part = session.solidbom.draft.as_ref().expect("the draft").parts[0].part_version_id.to_string();
    session
        .execute(
            "solidbom.inputs",
            &json!({
                "part_version_id": part,
                "classification": "clip",
                "classification_confirmed": true,
                "volume_confirmed": true,
                "material_version_id": "00000000-0000-4000-8000-000000000001",
                "allowance_rate": "0.02",
                "cycle_seconds": "30",
                "cavities": 1,
                "production_rate_version_id": "00000000-0000-4000-8000-000000000101",
                "setup_hours": "0.5",
                "setup_rate_version_id": "00000000-0000-4000-8000-000000000101",
                "material_overhead_rate": "0.05"
            }),
        )
        .expect("edit");
}

fn take_save(fake: &Arc<FakeHost>) -> (JobId, SaveDraft) {
    fake.take()
        .into_iter()
        .find_map(|(job, submitted)| match submitted.request {
            ClientRequest::SaveDraft(save) => Some((job, save)),
            _ => None,
        })
        .expect("a save")
}

fn take_calculate(fake: &Arc<FakeHost>) -> (JobId, solidbom_contract::CalculateRequest) {
    fake.take()
        .into_iter()
        .find_map(|(job, submitted)| match submitted.request {
            ClientRequest::Calculate(request) => Some((job, request)),
            _ => None,
        })
        .expect("a calculation")
}

fn snapshot(project_id: uuid::Uuid, saved_version: u64) -> CalculationSnapshot {
    CalculationSnapshot {
        calculation_id: uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, format!("calc:{saved_version}").as_bytes()),
        project_id,
        saved_version,
        input_hash: "a".repeat(64),
        calculator_version: "test".into(),
        inputs: CalculationInputs { currency: "CNY".into(), material_overhead_rate: Some(decimal("0.05")), parts: Vec::new() },
        outputs: CalculationOutcome {
            state: OutcomeState::Complete,
            parts: Vec::new(),
            assembly_subtotal: None,
            setup_total: None,
            issues: Vec::new(),
        },
        issues: Vec::new(),
    }
}

#[test]
fn an_import_chains_the_upload_and_project_create() {
    let (mut session, fake) = fake_session();
    import_three_boxes(&mut session, &fake);
    assert_eq!(session.solidbom.phase, super::state::Phase::Reviewing);
    assert!(session.review.is_some());
    assert!(!session.solidbom.needs_service);
    assert_eq!(session.solidbom.save, SaveState::Clean);
    assert_eq!(session.solidbom.version(), 1);
    assert!(fake.take().is_empty(), "the chain has nothing left to submit");
}

#[test]
fn edits_during_a_save_keep_the_project_dirty() {
    let (mut session, fake) = fake_session();
    import_three_boxes(&mut session, &fake);
    edit(&mut session);
    session.solidbom_save(false).expect("save");
    let (job, save) = take_save(&fake);
    edit(&mut session);
    let receipt = SaveReceipt { request_id: save.request_id.clone(), project_id: save.draft.project_id, version: 2 };
    fake.complete(job, Ok(ClientReply::Receipt(receipt)));
    session.drain_solidbom();
    assert_eq!(session.solidbom.version(), 2);
    assert_eq!(session.solidbom.save, SaveState::Dirty);
}

#[test]
fn a_calculation_completed_after_an_edit_is_stale_and_keeps_the_selection() {
    let (mut session, fake) = fake_session();
    import_three_boxes(&mut session, &fake);
    edit(&mut session);
    session.solidbom_calculate().expect("calculate saves first");
    let (save_job, save) = take_save(&fake);
    let receipt = SaveReceipt { request_id: save.request_id.clone(), project_id: save.draft.project_id, version: 2 };
    fake.complete(save_job, Ok(ClientReply::Receipt(receipt)));
    session.drain_solidbom();
    let (calculation_job, _) = take_calculate(&fake);
    session.selection = vec![Sel::Occurrence { path: vec![7] }];
    edit(&mut session);
    let project_id = session.solidbom.draft.as_ref().expect("the draft").project_id;
    fake.complete(calculation_job, Ok(ClientReply::Calculation(snapshot(project_id, 2))));
    session.drain_solidbom();
    assert_eq!(session.solidbom.result, ControllerResult::Stale);
    assert!(session.solidbom.current.is_some());
    assert_eq!(session.selection, vec![Sel::Occurrence { path: vec![7] }]);
}

#[test]
fn a_failed_save_keeps_the_inputs_and_retries_the_same_request() {
    let (mut session, fake) = fake_session();
    import_three_boxes(&mut session, &fake);
    edit(&mut session);
    session.solidbom_save(false).expect("save");
    let (job, save) = take_save(&fake);
    fake.complete(job, Err(ClientFailure::Network("offline".into())));
    session.drain_solidbom();
    assert!(matches!(session.solidbom.save, SaveState::SaveFailed { .. }));
    assert!(session.solidbom.failed.as_ref().expect("failure").retryable);
    let kept = session.solidbom.draft.as_ref().expect("the draft").parts[0].operation.as_ref().expect("operation");
    assert_eq!(kept.cycle_seconds, Some(decimal("30")));
    session.solidbom_retry().expect("retry");
    let (retried_job, retried) = take_save(&fake);
    assert_ne!(retried_job, job);
    assert_eq!(retried.request_id, save.request_id);
    assert_eq!(retried.draft.parts, save.draft.parts);
    assert!(fake.take().is_empty());
}

#[test]
fn a_conflict_is_not_retried_automatically() {
    let (mut session, fake) = fake_session();
    import_three_boxes(&mut session, &fake);
    edit(&mut session);
    session.solidbom_save(false).expect("save");
    let (job, _) = take_save(&fake);
    fake.complete(job, Err(ClientFailure::Conflict("the project moved".into())));
    session.drain_solidbom();
    assert!(matches!(session.solidbom.save, SaveState::SaveFailed { .. }));
    assert!(!session.solidbom.failed.as_ref().expect("failure").retryable);
    assert!(session.execute("solidbom.retry", &json!({})).is_err(), "a conflict needs the project reopened");
    assert!(fake.take().is_empty(), "a conflict waits for the user");
}

#[test]
fn a_session_without_a_service_still_imports_and_says_so() {
    let mut session = Session::default();
    let path = fixture("three_boxes.step");
    let reply = session.execute("solidbom.import", &json!({"path": path.to_string_lossy()})).expect("the local analysis works");
    assert_eq!(reply["needs_service"], json!(true));
    assert!(session.review.is_some());
    assert!(session.solidbom.draft.is_some());
    assert!(session.solidbom.needs_service);
    let error = session
        .submit_solidbom(ClientRequest::ReferenceData, super::state::PendingKind::ReferenceData, None, None)
        .expect_err("there is no service to submit to");
    assert!(matches!(error, crate::EngineError::Capability(_)), "{error}");
    assert!(session.solidbom_save(false).is_err(), "there is no service to save to");
}

#[test]
fn reopening_without_a_host_keeps_the_session_idle() {
    let mut session = Session::default();
    let error = session
        .execute("solidbom.reopen", &json!({"project_id": "00000000-0000-4000-8000-000000000001"}))
        .expect_err("there is no service to reopen from");
    assert!(matches!(error, crate::EngineError::Disabled(_, ref reason) if reason == super::state::SERVICE_MISSING), "{error}");
    assert_eq!(session.solidbom.phase, super::state::Phase::Idle);
    assert!(session.solidbom.pending.is_empty());
}

#[test]
fn a_failed_or_cancelled_reopen_returns_the_session_to_idle() {
    let (mut session, fake) = fake_session();
    for failure in [ClientFailure::NotFound, ClientFailure::Cancelled] {
        session.execute("solidbom.reopen", &json!({"project_id": "00000000-0000-4000-8000-000000000001"})).expect("the request is submitted");
        let (job, _) = fake.take().into_iter().find(|(_, job)| matches!(job.request, ClientRequest::LoadProject(_))).expect("a load request");
        assert_eq!(session.solidbom.phase, super::state::Phase::Importing);
        fake.complete(job, Err(failure));
        session.drain_solidbom();
        assert_eq!(session.solidbom.phase, super::state::Phase::Idle);
    }
}

#[test]
fn a_retried_reopen_stays_importing_until_it_finishes() {
    let (mut session, fake) = fake_session();
    session.execute("solidbom.reopen", &json!({"project_id": "00000000-0000-4000-8000-000000000001"})).expect("the request is submitted");
    let (job, _) = fake.take().into_iter().find(|(_, job)| matches!(job.request, ClientRequest::LoadProject(_))).expect("a load request");
    fake.complete(job, Err(ClientFailure::Server("500".into())));
    session.drain_solidbom();
    assert_eq!(session.solidbom.phase, super::state::Phase::Idle);
    assert!(session.solidbom.failed.as_ref().is_some_and(|failed| failed.retryable));
    session.solidbom_retry().expect("retry the load");
    assert_eq!(session.solidbom.phase, super::state::Phase::Importing);
    assert!(!crate::find_command("solidbom.reopen").expect("reopen").info(&session).enabled);
    let (job, _) = fake.take().into_iter().find(|(_, job)| matches!(job.request, ClientRequest::LoadProject(_))).expect("a retried load");
    fake.complete(job, Err(ClientFailure::NotFound));
    session.drain_solidbom();
    assert_eq!(session.solidbom.phase, super::state::Phase::Idle);
}

#[test]
fn a_host_without_the_service_capability_is_refused_before_any_submission() {
    let (mut session, fake) = fake_session();
    fake.set_capabilities(HostCapabilities::none());
    let error = session
        .submit_solidbom(ClientRequest::ReferenceData, super::state::PendingKind::ReferenceData, None, None)
        .expect_err("the host reports no service");
    assert!(matches!(error, crate::EngineError::Capability(_)), "{error}");
    assert!(session.solidbom.needs_service);
    assert!(fake.take().is_empty());
}

#[test]
fn a_host_that_speaks_another_contract_version_is_refused_before_any_submission() {
    let (mut session, fake) = fake_session();
    fake.set_capabilities(HostCapabilities { contract_version: solidbom_contract::SCHEMA_VERSION + 1, ..HostCapabilities::default() });
    let error = session
        .submit_solidbom(ClientRequest::ReferenceData, super::state::PendingKind::ReferenceData, None, None)
        .expect_err("a different contract version is not usable");
    assert!(error.to_string().contains("contract version"), "{error}");
    assert!(fake.take().is_empty());
}

#[test]
fn a_full_host_queue_is_a_capability_error() {
    let (mut session, fake) = fake_session();
    fake.reject_with(HostError::QueueFull { capacity: solidbom_contract::MAX_PENDING_JOBS });
    let error =
        session.submit_solidbom(ClientRequest::ReferenceData, super::state::PendingKind::ReferenceData, None, None).expect_err("the host is full");
    assert!(matches!(error, crate::EngineError::Capability(_)), "{error}");
    assert!(session.solidbom.pending.is_empty());
}

#[test]
fn a_cancelled_save_keeps_the_project_dirty_and_offers_no_retry() {
    let (mut session, fake) = fake_session();
    import_three_boxes(&mut session, &fake);
    edit(&mut session);
    session.solidbom_save(false).expect("save");
    let (job, _) = take_save(&fake);
    fake.complete(job, Err(ClientFailure::Cancelled));
    session.drain_solidbom();
    assert_eq!(session.solidbom.save, SaveState::Dirty);
    assert!(session.solidbom.failed.is_none());
    assert!(session.solidbom.notice.as_deref().unwrap_or_default().contains("cancelled"));
}

#[test]
fn a_cancelled_calculation_clears_the_pending_result() {
    let (mut session, fake) = fake_session();
    import_three_boxes(&mut session, &fake);
    edit(&mut session);
    session.solidbom_calculate().expect("calculate saves first");
    let (save_job, save) = take_save(&fake);
    let receipt = SaveReceipt { request_id: save.request_id.clone(), project_id: save.draft.project_id, version: 2 };
    fake.complete(save_job, Ok(ClientReply::Receipt(receipt)));
    session.drain_solidbom();
    let (calculation_job, _) = take_calculate(&fake);
    fake.complete(calculation_job, Err(ClientFailure::Cancelled));
    session.drain_solidbom();
    assert_eq!(session.solidbom.result, ControllerResult::NoResult);
    assert!(session.solidbom.failed.is_none());
}

#[test]
fn a_new_binding_generation_makes_a_completed_calculation_stale() {
    let (mut session, fake) = fake_session();
    import_three_boxes(&mut session, &fake);
    edit(&mut session);
    session.solidbom_calculate().expect("calculate saves first");
    let (save_job, save) = take_save(&fake);
    let receipt = SaveReceipt { request_id: save.request_id.clone(), project_id: save.draft.project_id, version: 2 };
    fake.complete(save_job, Ok(ClientReply::Receipt(receipt)));
    session.drain_solidbom();
    let (calculation_job, _) = take_calculate(&fake);
    session.solidbom.binding_generation = session.solidbom.binding_generation.wrapping_add(1);
    let project_id = session.solidbom.draft.as_ref().expect("the draft").project_id;
    fake.complete(calculation_job, Ok(ClientReply::Calculation(snapshot(project_id, 2))));
    session.drain_solidbom();
    assert_eq!(session.solidbom.result, ControllerResult::Stale);
}

#[test]
fn a_completion_that_does_not_echo_its_envelope_is_reported_and_the_job_stays_pending() {
    let (mut session, fake) = fake_session();
    import_three_boxes(&mut session, &fake);
    edit(&mut session);
    session.solidbom_save(false).expect("save");
    let (job, save) = take_save(&fake);
    let receipt = SaveReceipt { request_id: save.request_id.clone(), project_id: save.draft.project_id, version: 2 };
    fake.complete_with_envelope(job, JobEnvelope::default(), Ok(ClientReply::Receipt(receipt)));
    session.drain_solidbom();
    assert!(session.solidbom.error.as_deref().unwrap_or_default().contains("envelope"));
    assert!(session.solidbom.pending.contains_key(&job));
}

#[test]
fn cancelling_a_session_cancels_every_pending_job() {
    let (mut session, fake) = fake_session();
    import_three_boxes(&mut session, &fake);
    edit(&mut session);
    session.solidbom_save(false).expect("save");
    let (job, _) = take_save(&fake);
    session.cancel_solidbom();
    assert!(session.solidbom.pending.is_empty());
    assert_eq!(fake.cancelled_jobs(), vec![job]);
    assert_eq!(session.solidbom.save, SaveState::Dirty);
}

#[test]
fn a_completion_for_a_job_that_is_not_pending_is_ignored() {
    let (mut session, fake) = fake_session();
    fake.complete(999, Ok(ClientReply::ReferenceData(ReferenceData { materials: Vec::new(), rates: Vec::new() })));
    session.drain_solidbom();
    assert!(session.solidbom.reference.is_none());
}

#[test]
fn a_refused_input_edit_leaves_the_draft_unchanged() {
    let (mut session, fake) = fake_session();
    import_three_boxes(&mut session, &fake);
    let before = session.solidbom.draft.clone();
    let part = session.solidbom.draft.as_ref().expect("the draft").parts[0].part_version_id.to_string();
    let error = session
        .execute("solidbom.inputs", &json!({"part_version_id": part, "classification": "", "classification_confirmed": true}))
        .expect_err("a confirmed empty classification is refused");
    assert!(error.to_string().contains("classification"), "{error}");
    assert_eq!(session.solidbom.draft, before);
    assert_eq!(session.solidbom.save, SaveState::Clean);
}

#[test]
fn parts_projection_keeps_shared_identity_quantity_and_exact_occurrence_paths() {
    let mut session = Session::default();
    run(&mut session, "solidbom.import", json!({"path": fixture("three_boxes.step")}));
    let reply = run(&mut session, "solidbom.parts", json!({}));
    let rows = reply["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row["quantity"], 3);
    assert!(row["per_part_subtotal"].is_null());
    assert_eq!(row["volume_source"], "extracted");
    assert_eq!(row["volume_review"], "pending");
    assert_eq!(row["needs_review"], true);
    assert_eq!(row["inputs"]["part_version_id"], row["part_version_id"]);
    assert_eq!(row["occurrence_paths"].as_array().unwrap().len(), 3);
    let definition: uuid::Uuid = row["definition_id"].as_str().unwrap().parse().unwrap();
    let analysis = &session.solidbom.draft.as_ref().unwrap().analysis;
    for path in row["occurrence_paths"].as_array().unwrap() {
        let path: Vec<u64> = serde_json::from_value(path.clone()).unwrap();
        session.occurrence_placement(&path).unwrap();
        assert_eq!(resolve_pick(&session, analysis, &Sel::Occurrence { path }).unwrap().definition_id, definition);
    }
}

#[test]
fn parts_projection_preserves_missing_volume_and_source_issues() {
    let mut session = Session::default();
    run(&mut session, "solidbom.import", json!({"path": fixture("open_surface.step")}));
    let reply = run(&mut session, "solidbom.parts", json!({}));
    let row = &reply["rows"][0];
    assert!(row["volume_mm3"].is_null());
    assert!(row["material"].is_null());
    assert!(row["line_total"].is_null());
    assert!(row["missing_inputs"].as_array().unwrap().iter().any(|field| field == "volume_mm3"));
    assert!(!row["issues"].as_array().unwrap().is_empty());
}

#[test]
fn batch_inputs_validate_every_part_before_committing_once() {
    let mut session = Session::default();
    run(&mut session, "solidbom.import", json!({"path": fixture("revisions_bc.step")}));
    let draft = session.solidbom.draft.clone().unwrap();
    assert_eq!(draft.parts.len(), 2);
    let first = draft.parts[0].part_version_id.to_string();
    let second = draft.parts[1].part_version_id.to_string();
    let generation = session.solidbom.generation;
    let bad = json!({"parts": [
        {"part_version_id": first, "classification": "plate", "classification_confirmed": true},
        {"part_version_id": second, "allowance_rate": "-1"}
    ], "material_overhead_rate": "0.15"});
    assert!(session.execute("solidbom.inputs.batch", &bad).is_err());
    assert_eq!(session.solidbom.draft, Some(draft));
    assert_eq!(session.solidbom.generation, generation);
    run(
        &mut session,
        "solidbom.inputs.batch",
        json!({"parts": [
        {"part_version_id": first, "classification": "plate", "classification_confirmed": true},
        {"part_version_id": second, "classification": "clip", "classification_confirmed": true}
    ], "material_overhead_rate": "0.15"}),
    );
    let draft = session.solidbom.draft.as_ref().unwrap();
    assert!(draft.parts.iter().all(|part| part.classification_confirmed));
    assert_eq!(draft.material_overhead_rate, Some(decimal("0.15")));
    assert_eq!(session.solidbom.generation, generation + 1);
    assert_eq!(session.solidbom.save, SaveState::Dirty);
}

#[test]
fn batch_inputs_reject_duplicate_unknown_and_part_scoped_overhead_without_changes() {
    let mut session = Session::default();
    run(&mut session, "solidbom.import", json!({"path": fixture("three_boxes.step")}));
    let draft = session.solidbom.draft.clone().unwrap();
    let part = draft.parts[0].part_version_id.to_string();
    for params in [
        json!({"parts": [{"part_version_id": part}, {"part_version_id": part}]}),
        json!({"parts": [{"part_version_id": uuid::Uuid::nil().to_string()}]}),
        json!({"parts": [{"part_version_id": part, "material_overhead_rate": "0.1"}]}),
    ] {
        assert!(matches!(session.execute("solidbom.inputs.batch", &params), Err(crate::EngineError::BadParams { .. })));
        assert_eq!(session.solidbom.draft, Some(draft.clone()));
    }
}

#[test]
fn service_command_availability_requires_a_project_service_and_idle_request() {
    let mut session = Session::default();
    for id in ["solidbom.save", "solidbom.calculate", "solidbom.retry"] {
        let info = crate::find_command(id).unwrap().info(&session);
        assert!(!info.enabled, "{id} must explain why it is unavailable without a review");
        assert!(info.disabled_reason.is_some());
    }
    run(&mut session, "solidbom.import", json!({"path": fixture("three_boxes.step")}));
    let info = crate::find_command("solidbom.save").unwrap().info(&session);
    assert!(!info.enabled);
    assert!(info.disabled_reason.unwrap().contains("service"));
    let (mut session, fake) = fake_session();
    import_three_boxes(&mut session, &fake);
    for id in ["solidbom.save", "solidbom.calculate"] {
        assert!(crate::find_command(id).unwrap().info(&session).enabled);
    }
    assert!(!crate::find_command("solidbom.retry").unwrap().info(&session).enabled);
    session.solidbom_save(false).unwrap();
    assert!(!crate::find_command("solidbom.save").unwrap().info(&session).enabled);
    assert!(!crate::find_command("solidbom.calculate").unwrap().info(&session).enabled);
}

#[test]
fn reverse_projection_refuses_ambiguous_source_paths() {
    let mut session = Session::default();
    run(&mut session, "solidbom.import", json!({"path": fixture("revisions_bc.step")}));
    let manifest = &mut session.solidbom.draft.as_mut().unwrap().analysis;
    assert_eq!(manifest.occurrences.len(), 2);
    manifest.occurrences[1].source_path = manifest.occurrences[0].source_path.clone();
    assert!(super::extract::definition_occurrence_paths(&session, &session.solidbom.draft.as_ref().unwrap().analysis).is_err());
}

#[test]
fn reviewed_sources_allow_selection_commands_without_allowing_model_edits() {
    let mut session = Session::default();
    run(&mut session, "solidbom.import", json!({"path": fixture("three_boxes.step")}));
    let row = session.solidbom_parts().unwrap().rows.remove(0);
    let items: Vec<_> = row.occurrence_paths.into_iter().map(|path| Sel::Occurrence { path }).collect();
    let document = session.doc.clone();
    let undo = session.undo.len();
    let reply = run(&mut session, "select.set", json!({"items": items}));
    assert_eq!(reply["selected"], 3);
    run(&mut session, "select.filter", json!({"mode": "component"}));
    run(&mut session, "select.clear", json!({}));
    assert!(session.selection.is_empty());
    assert!(std::sync::Arc::ptr_eq(&document, &session.doc));
    assert_eq!(session.undo.len(), undo);
    assert!(matches!(session.execute("solid.box", &json!({"length": 1, "width": 1, "height": 1})), Err(crate::EngineError::Refused { .. })));
}
