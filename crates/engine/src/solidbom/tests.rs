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
    let reply = session.execute(id, &params).expect(id);
    // The local analysis runs beside the window; tests that assert its pending state drive the
    // poll themselves, every other test waits here for the review it publishes.
    pump(session);
    reply
}

/// Wait for the session's background analysis to publish or fail. The worker is a real thread,
/// so the wait is bounded rather than a sleep chosen to be long enough.
fn pump(session: &mut Session) {
    let began = std::time::Instant::now();
    while session.solidbom_analysis.is_some() {
        session.drain_solidbom_analysis();
        assert!(began.elapsed() < std::time::Duration::from_secs(120), "the background analysis did not finish: {:?}", session.solidbom.error);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
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
    let expected = &spec.definitions[0];
    assert_eq!(definition_issues(&manifest, manifest.definitions[0].definition_id), expected.issues);
    // The grouping is unresolved but the geometry is fully measurable: a flat-volume regression
    // must fail against the authored 7200 mm3, never against the extractor's own output.
    let expected_volume = decimal(expected.volume_mm3.as_deref().expect("the authored flat volume"));
    let tolerance = expected.tolerance_mm3.as_deref().map(decimal).unwrap_or(Decimal::ZERO);
    let volume = manifest.definitions[0].volume_mm3.value.expect("the three flat boxes are measurable");
    assert!((volume - expected_volume).abs() <= tolerance, "{volume}");
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
}

#[test]
fn resolve_definition_pick_refuses_unknown_and_unplaced_definitions() {
    let (session, _, key) = prepared("nested_repeated.step");
    let manifest = extract(&session, key).expect("extract");
    let cover =
        manifest.definitions.iter().find(|definition| definition.customer_part_number.as_deref() == Some("CV-1")).expect("the cover definition");
    let unknown = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, b"not a definition");
    let error = resolve_definition_pick(&manifest, unknown).expect_err("an unknown definition is refused");
    assert!(error.to_string().contains("not a definition"), "{error}");

    // The contract does not require every definition to be placed, so a valid manifest can hold a
    // definition no occurrence places: the placed-refusal branch is testable without an API change.
    let mut unplaced = manifest.clone();
    unplaced.occurrences.retain(|occurrence| occurrence.definition_id != cover.definition_id);
    unplaced.geometry_bindings.retain(|binding| binding.definition_id != cover.definition_id);
    let placed: Vec<uuid::Uuid> = unplaced.occurrences.iter().map(|occurrence| occurrence.occurrence_id).collect();
    for occurrence in &mut unplaced.occurrences {
        if let Some(parent) = occurrence.parent_occurrence_id
            && !placed.contains(&parent)
        {
            occurrence.parent_occurrence_id = None;
        }
    }
    unplaced.validate().expect("a definition need not be placed for the manifest to be valid");
    let error = resolve_definition_pick(&unplaced, cover.definition_id).expect_err("an unplaced definition is refused");
    assert!(error.to_string().contains("not placed"), "{error}");
}

/// Serializes this test's reads and writes of the process-wide display quality.
///
/// The setting is one value for the whole test process, and the test harness runs tests on
/// threads; holding this lock keeps this test's section from interleaving with itself. Other
/// engine tests that change the same setting do not take this lock; giving them a shared one
/// changes their files, which are outside this package's ownership.
static DISPLAY_QUALITY_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Restores the process-wide display quality when it is dropped, on a normal return and on a panic.
struct DisplayQualityGuard(solidcraft_doc::display_quality::DisplayQuality);

impl DisplayQualityGuard {
    /// Capture the quality in effect now; dropping the guard puts it back.
    fn capture() -> Self {
        Self(solidcraft_doc::display_quality::current())
    }
}

impl Drop for DisplayQualityGuard {
    fn drop(&mut self) {
        solidcraft_doc::display_quality::set_current(self.0);
    }
}

#[test]
fn display_quality_never_changes_extraction() {
    use solidcraft_doc::display_quality::{DisplayQuality, set_current};
    // The display quality is global to the process: hold the lock while this test changes it and
    // reads display plans, and restore the previous value with a guard so a failed assertion cannot
    // leave another test at a different quality.
    let _quality = DISPLAY_QUALITY_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let _restore = DisplayQualityGuard::capture();
    // A display-quality setting changes the display tessellation, not the model the extractor
    // measures. Prove the switch is observable on a curved body first, then compare the business
    // extraction of the reviewed source at both levels.
    let mut curved = Session::default();
    run(&mut curved, "solid.cylinder", json!({"radius": 10, "height": 20}));
    let curved_state = curved.model.state();
    let curved_body = curved_state.bodies.first().expect("the cylinder body");
    let plan_vertices =
        |body: &solidcraft_doc::ModelBody, quality: DisplayQuality| body.display_plan_at(quality).expect("the display plan").vertex_count();
    let coarse_vertices = plan_vertices(curved_body, DisplayQuality::Coarse);
    let ultra_vertices = plan_vertices(curved_body, DisplayQuality::Ultra);
    assert!(ultra_vertices > coarse_vertices, "display quality must change the tessellation: {coarse_vertices} vs {ultra_vertices}");

    let path = fixture("three_boxes.step");
    let (session, _, key) = prepared("three_boxes.step");
    let imported_state = session.model.state();
    let imported_body = imported_state.bodies.first().expect("the imported body");
    let imported_vertices: Vec<usize> =
        [DisplayQuality::Coarse, DisplayQuality::Ultra].into_iter().map(|quality| plan_vertices(imported_body, quality)).collect();
    assert_eq!(imported_vertices[0], imported_vertices[1], "the planar imported box tessellates the same at both levels");

    let before = sha256(&path);
    let mut manifests = Vec::new();
    for quality in [DisplayQuality::Coarse, DisplayQuality::Ultra] {
        set_current(quality);
        let manifest = extract(&session, key.clone()).expect("extract");
        let volume = manifest.definitions[0].volume_mm3.value.expect("the exact volume");
        assert!((volume - decimal("2400")).abs() <= decimal("2.4"), "{quality:?}: {volume}");
        manifests.push(manifest);
    }
    assert_eq!(manifests[0], manifests[1], "the business extraction is identical at any display quality");
    assert_eq!(manifests[0].occurrences.len(), 3);
    assert_eq!(sha256(&path), before, "extraction never rewrites the source");
}

#[test]
fn display_quality_never_changes_tube_extraction() {
    use super::extraction::{ExtractionCategory, ExtractionStatus, extract_tube};
    use solidcraft_doc::display_quality::{DisplayQuality, set_current};
    // The display quality is process-wide: share the lock and restoration with the reviewed-source
    // test above so the two never race and neither leaves another test at a different quality.
    let _quality = DISPLAY_QUALITY_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let _restore = DisplayQualityGuard::capture();
    let mut session = Session::default();
    run(&mut session, "sketch.create", json!({"plane": "XY", "name": "Path"}));
    let arc = run(&mut session, "sketch.arc.center_point", json!({"center": [0.0, 20.0], "start": [0.0, 0.0], "sweep": 90.0}));
    let path: Vec<String> =
        arc["curves"].as_array().map(|curves| curves.iter().filter_map(|curve| curve.as_str().map(str::to_string)).collect()).unwrap_or_default();
    run(&mut session, "sketch.finish", json!({}));
    run(&mut session, "solid.pipe", json!({"path_sketch": "Path", "path": path, "diameter": 10.0, "wall": 2.0, "body_name": "Tube"}));
    let state = session.model.state();
    let body = state.bodies.first().expect("the tube body");
    // Prove the switch is observable on this curved body first, as the reviewed-source test does:
    // the two qualities must tessellate the tube differently, or the comparison below is vacuous.
    let plan_vertices = |quality: DisplayQuality| body.display_plan_at(quality).expect("the display plan").vertex_count();
    let (coarse_vertices, ultra_vertices) = (plan_vertices(DisplayQuality::Coarse), plan_vertices(DisplayQuality::Ultra));
    assert!(ultra_vertices > coarse_vertices, "display quality must change the tessellation: {coarse_vertices} vs {ultra_vertices}");
    let key =
        AnalysisKey { source_sha256: "a".repeat(64), engine_build: engine_build().to_string(), extractor_version: EXTRACTOR_VERSION.to_string() };
    let mut results = Vec::new();
    for quality in [DisplayQuality::Coarse, DisplayQuality::Ultra] {
        set_current(quality);
        let result = extract_tube(&key, uuid::Uuid::from_u128(1), ExtractionCategory::Tube, &[body]);
        assert_eq!(result.status, ExtractionStatus::Complete, "{quality:?}: {:?}", result.issues);
        results.push(result);
    }
    assert_eq!(results[0], results[1], "the business extraction is identical at any display quality");
    let developed: f64 = results[0].developed_centerline_length_mm.expect("the developed length").to_string().parse().expect("a number");
    assert!((developed - std::f64::consts::FRAC_PI_2 * 20.0).abs() <= 1e-6, "{developed}");
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
        "component.hide_history",
        "component.show_history",
        "document.metadata.set",
        "file.save",
        "parameters.change",
        "view.save",
        "view.rename",
        "view.delete",
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
        // Display-only tree folding changes no design state, so a reviewed source keeps it.
        ("browser.expand_all", json!({})),
        ("browser.collapse_all", json!({})),
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
fn a_reviewed_session_renames_bodies_and_components_without_changing_the_mapping() {
    // Set the reviewed session up the way `publish_solidbom_import` leaves it, from a directly
    // opened fixture. `solidbom.import` runs the background analysis worker, whose `analyse` step
    // writes a temporary carrier file first (jobs.rs); this regression needs neither the worker
    // nor a writable temporary folder to be reproducible.
    let (imported, source, key) = prepared_analysis("three_boxes.step");
    let draft = draft_from(&imported.manifest, &source);
    let runtime = RuntimeOccurrenceMap::new(&imported.session).expect("the runtime/source map");
    let analysis = draft.analysis.clone();
    let mut session = Session::default();
    session.replace_design(imported.session);
    session.solidbom = SolidBomState { phase: Phase::Reviewing, save: SaveState::Dirty, draft: Some(draft), ..Default::default() };
    session.solidbom.runtime = Some(runtime);
    session.bind_review(ReviewPolicy::new(source, key));
    let parts = session.solidbom_parts().expect("the reviewed parts").rows;
    let mapping = reviewed_mapping(&session, &analysis);
    let body = session.model.state().bodies.first().map(|body| body.name.clone()).expect("an imported body");
    let component = session.doc.components.first().map(|component| component.id).expect("an imported component");
    // A name is not geometry, so a reviewed session keeps these edits (issue #50).
    run(&mut session, "body.rename", json!({"body": body, "name": "Reviewed body"}));
    run(&mut session, "component.rename", json!({"component": component, "name": "Reviewed part"}));
    assert!(session.model.state().bodies.iter().any(|body| body.name == "Reviewed body"));
    assert_eq!(session.doc.components.first().map(|component| component.name.as_str()), Some("Reviewed part"));
    // Every reviewed occurrence still resolves to the definition the source gave it.
    assert_eq!(reviewed_mapping(&session, &analysis), mapping);
    // The reviewed rows keep their identity, quantity, occurrence paths and volume. A part with
    // no customer part number shows the component's name instead, so that text may follow the
    // rename; it is not the reviewed identity.
    let after = session.solidbom_parts().expect("the reviewed parts").rows;
    assert_eq!(after.len(), parts.len());
    for (before, after) in parts.iter().zip(&after) {
        assert_eq!(before.definition_id, after.definition_id);
        assert_eq!(before.quantity, after.quantity);
        assert_eq!(before.occurrence_paths, after.occurrence_paths);
        assert_eq!(before.volume_mm3, after.volume_mm3);
    }
    // The geometry, hierarchy and the rest of the document stay read-only.
    for id in ["solid.extrude", "occurrence.reparent", "document.metadata.set"] {
        let error = session.execute(id, &json!({})).expect_err(id);
        assert!(matches!(error, crate::EngineError::Refused { .. }), "{id}: {error}");
    }
}

/// Show/Hide, Physical Material and Appearance are document changes, but display attributes: the
/// reviewed source keeps them (issue #69) while geometry, hierarchy, history and source
/// replacement stay refused, and every reviewed occurrence keeps the definition it resolved to.
#[test]
fn a_reviewed_session_edits_display_attributes_without_changing_the_mapping() {
    let (imported, source, key) = prepared_analysis("three_boxes.step");
    let draft = draft_from(&imported.manifest, &source);
    let runtime = RuntimeOccurrenceMap::new(&imported.session).expect("the runtime/source map");
    let analysis = draft.analysis.clone();
    let mut session = Session::default();
    session.replace_design(imported.session);
    session.solidbom = SolidBomState { phase: Phase::Reviewing, save: SaveState::Dirty, draft: Some(draft), ..Default::default() };
    session.solidbom.runtime = Some(runtime);
    session.bind_review(ReviewPolicy::new(source, key));
    let parts = session.solidbom_parts().expect("the reviewed parts").rows;
    let mapping = reviewed_mapping(&session, &analysis);
    let body = session.model.state().bodies.first().map(|body| body.name.clone()).expect("an imported body");
    // A display attribute is not geometry or hierarchy, so a reviewed session keeps these edits.
    run(&mut session, "browser.visibility", json!({"items": [body.clone()], "visible": false}));
    assert!(session.visibility.hidden_bodies.contains(&body), "the reviewed body is hidden");
    run(&mut session, "material.assign", json!({"bodies": [body.clone()], "material": "Steel"}));
    assert_eq!(session.doc.body_material(&body), "Steel");
    run(&mut session, "appearance.assign", json!({"bodies": [body.clone()], "color": "#336699"}));
    assert_eq!(session.doc.appearances.bodies.get(&body).map(|look| look.color), Some([0x33, 0x66, 0x99]));
    // The display-attribute family the dialogs and their look tools drive is admitted, and the
    // commands that only read it stay available.
    for id in [
        "appearance.assign",
        "appearance.chain",
        "appearance.copy",
        "appearance.favorite",
        "appearance.library",
        "appearance.list",
        "appearance.paint",
        "appearance.paste",
        "appearance.pick_up",
        "appearance.replace",
        "browser.visibility",
        "material.assign",
        "material.inspect",
        "material.list",
    ] {
        assert!(session.review_allows(id), "{id} is a display-attribute command");
    }
    for (id, params) in [
        ("material.list", json!({})),
        ("material.inspect", json!({"material": "Steel"})),
        ("appearance.library", json!({})),
        ("appearance.list", json!({})),
        ("appearance.chain", json!({"target": {"body": body}})),
    ] {
        session.execute(id, &params).unwrap_or_else(|error| panic!("{id}: {error}"));
    }
    // Every reviewed occurrence still resolves to the definition the source gave it.
    assert_eq!(reviewed_mapping(&session, &analysis), mapping);
    let after = session.solidbom_parts().expect("the reviewed parts").rows;
    assert_eq!(after.len(), parts.len());
    for (before, after) in parts.iter().zip(&after) {
        assert_eq!(before.definition_id, after.definition_id);
        assert_eq!(before.quantity, after.quantity);
        assert_eq!(before.occurrence_paths, after.occurrence_paths);
        assert_eq!(before.volume_mm3, after.volume_mm3);
    }
    // Geometry, hierarchy, parameters, history and source replacement stay refused.
    for id in ["solid.extrude", "occurrence.reparent", "parameters.change", "edit.undo", "edit.redo", "file.new", "solidbom.import"] {
        let error = session.execute(id, &json!({})).expect_err(id);
        assert!(matches!(error, crate::EngineError::Refused { .. }), "{id}: {error}");
    }
}

/// Every saved review occurrence, through the runtime map, as the definition it resolves to.
fn reviewed_mapping(session: &Session, analysis: &AnalysisManifest) -> Vec<(Vec<u64>, uuid::Uuid)> {
    let runtime = RuntimeOccurrenceMap::new(session).expect("the runtime/source map");
    analysis
        .occurrences
        .iter()
        .map(|occurrence| {
            let runtime_id = runtime.runtime_occurrence(&occurrence.source_path).expect("the map places the saved path");
            let picked = resolve_pick(session, analysis, &Sel::Occurrence { path: vec![runtime_id] }).expect("the occurrence resolves");
            (occurrence.source_path.clone(), picked.definition_id)
        })
        .collect()
}

/// A sandbox that cannot rebuild the crate can still check a prebuilt test binary against the
/// checkout: this test embeds its own sources of this change at compile time and compares them
/// with the files on disk. A binary built from anything else fails it, so a passing run of this
/// test (with a clean checkout at the reviewed commit) shows the binary was built from this
/// revision. The test source embeds itself, so a binary whose check was altered cannot pass either.
#[test]
fn the_prebuilt_binary_matches_this_checkout_of_the_reviewed_change() {
    let changed = [("policy.rs", include_bytes!("policy.rs").as_slice()), ("tests.rs", include_bytes!("tests.rs").as_slice())];
    for (name, built) in changed {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/solidbom").join(name);
        let checkout = std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert_eq!(hex(&Sha256::digest(built)), hex(&Sha256::digest(&checkout)), "{name}: the prebuilt test binary was not built from this checkout");
    }
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

#[test]
fn each_review_binding_is_a_new_policy_generation() {
    let mut session = Session::default();
    assert_eq!(session.review_generation(), 0);
    assert!(session.review_locked_reason().is_none());
    session.bind_review(ReviewPolicy::new(reviewed_source(), key(&fixture("three_boxes.step"))));
    assert_eq!(session.review_generation(), 1);
    assert_eq!(session.review_locked_reason(), Some(super::policy::READ_ONLY_REASON));
    session.bind_review(ReviewPolicy::new(reviewed_source(), key(&fixture("three_boxes.step"))));
    assert_eq!(session.review_generation(), 2);
    // Scratch copies carry the policy and its generation: work prepared from them is held to it.
    let mut scratch = session.scratch();
    assert_eq!(scratch.review_generation(), 2);
    assert!(!scratch.review_allows("solid.extrude"));
    // A file worker's session is a scratch copy too: a replacement command is refused there
    // before it reaches the file.
    let error = scratch.execute("file.insert_step", &json!({"path": fixture("three_boxes.step")})).expect_err("the insert is refused");
    assert!(matches!(error, crate::EngineError::Refused { .. }), "{error}");
}

#[test]
fn feature_commands_prepared_before_or_while_bound_are_refused() {
    let mut session = Session::default();
    run(&mut session, "solid.box", json!({"length": 10, "width": 10, "height": 10}));
    let computed = session
        .scratch()
        .compute_commands(&[("solid.box".into(), json!({"length": 5, "width": 5, "height": 5}))], None)
        .expect("an unbound session prepares an authoring transaction");
    assert!(computed.is_current(&session));
    session.bind_review(ReviewPolicy::new(reviewed_source(), key(&fixture("three_boxes.step"))));
    let error = session.accept_commands(computed).expect_err("a binding invalidates the prepared transaction");
    assert!(matches!(error, crate::EngineError::Refused { .. }), "{error}");
    // The reviewed source is read-only, which is the most direct reason and comes first.
    assert!(error.to_string().contains("read-only while it is reviewed"), "{error}");
    // While bound, the transaction cannot even be prepared: its commands are refused at once.
    let error = session
        .scratch()
        .compute_commands(&[("solid.box".into(), json!({"length": 5, "width": 5, "height": 5}))], None)
        .err()
        .expect("no authoring transaction is prepared in a reviewed session");
    assert!(matches!(error, crate::EngineError::Refused { .. }), "{error}");
}

#[test]
fn a_reviewed_session_still_obeys_the_embedding_host_check() {
    let mut session = bound_session();
    session.set_command_check(Some(std::sync::Arc::new(
        |id: &str, _: &Value, _: &Session| {
            if id == "view.fit" { Err("the embedding host refuses Fit".to_string()) } else { Ok(()) }
        },
    )));
    let error = session.execute("view.fit", &json!({})).expect_err("the host refuses");
    assert!(matches!(error, crate::EngineError::Refused { .. }), "{error}");
    assert!(error.to_string().contains("the embedding host refuses Fit"), "{error}");
    // The reviewed-source refusal comes first, so a host check can never re-enable editing.
    session.set_command_check(Some(std::sync::Arc::new(|_: &str, _: &Value, _: &Session| Ok(()))));
    let error = session.execute("solid.extrude", &json!({})).expect_err("still reviewed");
    assert!(error.to_string().contains("not allowed"), "{error}");
}

#[test]
fn source_replacement_commands_are_refused_while_bound() {
    let mut session = bound_session();
    for (id, params) in
        [("solidbom.import", json!({"path": fixture("three_boxes.step")})), ("solidbom.reopen", json!({"project_id": uuid::Uuid::nil()}))]
    {
        let error = session.execute(id, &params).expect_err(id);
        assert!(matches!(error, crate::EngineError::Refused { .. }), "{id}: {error}");
        assert!(error.to_string().contains("not allowed"), "{id}: {error}");
    }
    assert!(session.review.is_some());
}

#[test]
fn a_failed_import_leaves_the_session_exactly_as_it_was() {
    let mut session = Session::default();
    run(&mut session, "solid.box", json!({"length": 10, "width": 10, "height": 10}));
    let doc = session.doc.clone();
    let revision = session.revision;
    let generation = session.review_generation();
    let dir = std::env::temp_dir().join(format!("solidbom-failed-import-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a temporary folder");
    let broken = dir.join("broken.step");
    std::fs::write(&broken, b"this is not a STEP file").expect("the broken source");
    let reply = session.execute("solidbom.import", &json!({"path": broken.to_string_lossy()})).expect("the analysis starts");
    assert_eq!(reply["state"], json!("importing"));
    assert_eq!(session.solidbom.phase, super::state::Phase::Importing);
    pump(&mut session);
    assert!(session.review.is_none(), "{:?}", session.solidbom.error);
    assert_eq!(session.review_generation(), generation);
    assert!(std::sync::Arc::ptr_eq(&doc, &session.doc), "the design is unchanged");
    assert_eq!(session.revision, revision);
    assert!(session.solidbom.draft.is_none());
    assert_eq!(session.solidbom.phase, super::state::Phase::Idle);
    assert!(session.solidbom.error.as_deref().is_some_and(|error| !error.is_empty()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_reviewed_session_never_writes_over_the_original_source() {
    let dir = std::env::temp_dir().join(format!("solidbom-review-source-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a temporary folder");
    let step = dir.join("source.step");
    std::fs::copy(fixture("three_boxes.step"), &step).expect("the reviewed source");
    let original = std::fs::read(&step).expect("the source bytes");
    let mut session = Session::default();
    run(&mut session, "solidbom.import", json!({"path": step.to_string_lossy()}));
    for id in ["file.save_as", "file.export", "file.export_stl", "file.save_mesh", "solidbom.export"] {
        let error = session.execute(id, &json!({"path": step.to_string_lossy()})).expect_err(id);
        assert!(matches!(error, crate::EngineError::Refused { .. }), "{id}: {error}");
        assert!(error.to_string().contains("would overwrite the reviewed source"), "{id}: {error}");
    }
    // An equivalent path names the same file: absolute + `..` and a symbolic link both count.
    let equivalent = dir.join("..").join(dir.file_name().expect("a folder name")).join("source.step");
    let error = session.execute("file.export", &json!({"path": equivalent.to_string_lossy()})).expect_err("the equivalent path");
    assert!(matches!(error, crate::EngineError::Refused { .. }), "{error}");
    #[cfg(unix)]
    {
        let link = dir.join("source-link.step");
        std::os::unix::fs::symlink(&step, &link).expect("the link");
        let error = session.execute("file.save_as", &json!({"path": link.to_string_lossy()})).expect_err("the link");
        assert!(matches!(error, crate::EngineError::Refused { .. }), "{error}");
    }
    // A separate local working file is exactly what a reviewed session may keep.
    let copy = dir.join("review.sc");
    run(&mut session, "file.save_as", json!({"path": copy.to_string_lossy()}));
    assert!(copy.exists());
    run(&mut session, "file.export_stl", json!({"path": dir.join("review.stl").to_string_lossy()}));
    assert!(dir.join("review.stl").exists());
    assert_eq!(std::fs::read(&step).expect("the source bytes"), original, "the reviewed source is untouched");
    let _ = std::fs::remove_dir_all(&dir);
}

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use solidbom_contract::{
    CalculationInputs, CalculationOutcome, CalculationSnapshot, CostCenterPurpose, ExportBaselineRef, ExportBaselineSelection, ExportBundle,
    ExportCalculation, ExportFormat, ExportLabel, ExportMetadata, ExportRecordVersions, ExportRequest, HostCapabilities, HostError, Identity,
    JobEnvelope, JobRequest, MaterialVersion, MaterialVersionPublished, Permission, RateVersion, RateVersionPublished, ReferenceChange,
    ReferenceData, ResultState as OutcomeState, SaveDraft, SaveReceipt, SharedDataView, SharedRecordKind, Unit,
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
            list.push((id, job.clone()));
        }
        // The identity request is a read the session always needs to know what to offer; the
        // fake answers it at once so project tests only drive the project requests themselves.
        if matches!(job.request, ClientRequest::Identity)
            && let Ok(mut list) = self.completions.lock()
        {
            let permissions = vec![Permission::Engineer, Permission::Administrator];
            let identity = Identity { identity_id: uuid::Uuid::from_u128(1), name: "test identity".into(), permissions };
            list.push(JobCompletion { job_id: id, envelope: job.envelope, result: Ok(ClientReply::Identity(identity)) });
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
    pump(session);
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
        fake.complete(
            *job,
            Ok(ClientReply::ReferenceData(ReferenceData { materials: Vec::new(), rates: Vec::new(), components: Vec::new(), app_config: None })),
        );
    }
    session.drain_solidbom();
    // The reference-data answer asks for the permission sets; take that answer too, so the
    // remaining request list only holds what a test submits itself.
    session.drain_solidbom();
    let _ = fake.take();
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

fn take_load_calculation(fake: &Arc<FakeHost>) -> JobId {
    fake.take()
        .into_iter()
        .find_map(|(job, submitted)| matches!(submitted.request, ClientRequest::LoadCalculation(_)).then_some(job))
        .expect("a load-calculation request")
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
fn a_retry_after_a_network_failure_stays_available_and_resubmits_the_same_request() {
    let (mut session, fake) = fake_session();
    import_three_boxes(&mut session, &fake);
    edit(&mut session);
    session.solidbom_save(false).expect("save");
    let (job, first) = take_save(&fake);
    fake.complete(job, Err(ClientFailure::Network("connection refused".into())));
    session.drain_solidbom();
    assert!(matches!(session.solidbom.save, SaveState::SaveFailed { .. }));
    assert!(session.solidbom.failed.as_ref().expect("failure").retryable);
    assert!(!session.solidbom.needs_service, "an unreachable configured service is not a missing service");
    // The service comes back: the same command must stay available so a person can retry.
    assert!(
        crate::find_command("solidbom.retry").expect("the retry command").info(&session).enabled,
        "a transient failure leaves the explicit retry available"
    );
    session.execute("solidbom.retry", &json!({})).expect("the retry is accepted");
    let (job, retried) = take_save(&fake);
    assert_eq!(retried.request_id, first.request_id, "a retry reuses the request identity");
    assert_eq!(retried.expected_version, first.expected_version, "a retry reuses the expected version");
    assert_eq!(retried.draft.project_id, first.draft.project_id);
    let receipt = SaveReceipt { request_id: retried.request_id.clone(), project_id: retried.draft.project_id, version: first.expected_version + 1 };
    fake.complete(job, Ok(ClientReply::Receipt(receipt)));
    session.drain_solidbom();
    assert_eq!(session.solidbom.save, SaveState::Clean, "the retried save commits once");
    assert!(!session.solidbom.needs_service);
}

#[test]
fn a_successful_request_clears_a_previous_missing_service_report() {
    let (mut session, fake) = fake_session();
    session.solidbom.needs_service = true;
    session
        .submit_solidbom(ClientRequest::ReferenceData, super::state::PendingKind::ReferenceData, None, None)
        .expect("the configured service accepts the request");
    assert!(!session.solidbom.needs_service, "an accepted request proves the service is configured");
    let (job, _) = fake.take().into_iter().find(|(_, job)| matches!(job.request, ClientRequest::ReferenceData)).expect("the reference request");
    fake.complete(
        job,
        Ok(ClientReply::ReferenceData(ReferenceData { materials: Vec::new(), rates: Vec::new(), components: Vec::new(), app_config: None })),
    );
    session.drain_solidbom();
    assert!(session.solidbom.reference.is_some(), "the reference data arrives");
    assert!(!session.solidbom.needs_service);
}

#[test]
fn a_saved_calculation_loads_through_the_solidbom_calculation_command() {
    let (mut session, fake) = fake_session();
    import_three_boxes(&mut session, &fake);
    edit(&mut session);
    session.solidbom_save(false).expect("save");
    let (job, save) = take_save(&fake);
    let receipt = SaveReceipt { request_id: save.request_id.clone(), project_id: save.draft.project_id, version: 2 };
    fake.complete(job, Ok(ClientReply::Receipt(receipt)));
    session.drain_solidbom();
    session.solidbom_calculate().expect("calculate");
    let (job, request) = take_calculate(&fake);
    let current = snapshot(request.project_id, 2);
    fake.complete(job, Ok(ClientReply::Calculation(current.clone())));
    session.drain_solidbom();
    assert_eq!(session.solidbom.result, ControllerResult::Current);

    // An older saved version is loaded as history; it never replaces the current result.
    session.execute("solidbom.calculation", &json!({"calculation_id": current.calculation_id.to_string()})).expect("the load command starts");
    let job = take_load_calculation(&fake);
    let older = snapshot(current.project_id, 1);
    fake.complete(job, Ok(ClientReply::Calculation(older.clone())));
    session.drain_solidbom();
    assert_eq!(
        session.solidbom.calculations.iter().filter(|snapshot| snapshot.calculation_id == older.calculation_id).count(),
        1,
        "the loaded historical snapshot is kept"
    );
    assert_eq!(
        session.solidbom.current.as_ref().map(|snapshot| snapshot.calculation_id),
        Some(current.calculation_id),
        "an older snapshot does not replace the current result"
    );
    assert_eq!(session.solidbom.result, ControllerResult::Current);

    // A snapshot for the saved version the project is on becomes the current result.
    session.execute("solidbom.calculation", &json!({"calculation_id": older.calculation_id.to_string()})).expect("the load command starts");
    let job = take_load_calculation(&fake);
    let latest = snapshot(current.project_id, session.solidbom.version());
    fake.complete(job, Ok(ClientReply::Calculation(latest.clone())));
    session.drain_solidbom();
    assert_eq!(session.solidbom.current.as_ref().map(|snapshot| snapshot.calculation_id), Some(latest.calculation_id));
}

/// A calculation loaded for the current saved version is history while the reviewed inputs are
/// dirty: an edit after Calculate must not be cleared back to Current by loading its snapshot.
#[test]
fn a_loaded_calculation_does_not_become_current_after_an_edit() {
    let (mut session, fake) = fake_session();
    import_three_boxes(&mut session, &fake);
    edit(&mut session);
    session.solidbom_save(false).expect("save");
    let (job, save) = take_save(&fake);
    let project_id = save.draft.project_id;
    let receipt = SaveReceipt { request_id: save.request_id.clone(), project_id, version: 2 };
    fake.complete(job, Ok(ClientReply::Receipt(receipt)));
    session.drain_solidbom();
    session.solidbom_calculate().expect("calculate");
    let (job, request) = take_calculate(&fake);
    let current = snapshot(request.project_id, 2);
    fake.complete(job, Ok(ClientReply::Calculation(current.clone())));
    session.drain_solidbom();
    assert_eq!(session.solidbom.result, ControllerResult::Current);

    edit(&mut session);
    assert_eq!(session.solidbom.result, ControllerResult::Stale, "the edit makes the result stale");
    session.execute("solidbom.calculation", &json!({"calculation_id": current.calculation_id.to_string()})).expect("the load command starts");
    let job = take_load_calculation(&fake);
    fake.complete(job, Ok(ClientReply::Calculation(current.clone())));
    session.drain_solidbom();
    assert_eq!(session.solidbom.result, ControllerResult::Stale, "a loaded snapshot never clears an edit made before it was asked for");
    assert_eq!(session.solidbom.save, SaveState::Dirty, "the reviewed inputs stay unsaved");
    assert_eq!(
        session.solidbom.calculations.iter().filter(|saved| saved.calculation_id == current.calculation_id).count(),
        1,
        "the loaded snapshot stays readable as history"
    );
}

/// An edit made while a saved calculation is loading also keeps the loaded answer historical.
#[test]
fn an_edit_while_a_calculation_loads_keeps_the_result_stale() {
    let (mut session, fake) = fake_session();
    import_three_boxes(&mut session, &fake);
    edit(&mut session);
    session.solidbom_save(false).expect("save");
    let (job, save) = take_save(&fake);
    let receipt = SaveReceipt { request_id: save.request_id.clone(), project_id: save.draft.project_id, version: 2 };
    fake.complete(job, Ok(ClientReply::Receipt(receipt)));
    session.drain_solidbom();
    session.solidbom_calculate().expect("calculate");
    let (job, request) = take_calculate(&fake);
    let current = snapshot(request.project_id, 2);
    fake.complete(job, Ok(ClientReply::Calculation(current.clone())));
    session.drain_solidbom();

    session.execute("solidbom.calculation", &json!({"calculation_id": current.calculation_id.to_string()})).expect("the load command starts");
    let job = take_load_calculation(&fake);
    edit(&mut session);
    fake.complete(job, Ok(ClientReply::Calculation(current.clone())));
    session.drain_solidbom();
    assert_eq!(session.solidbom.result, ControllerResult::Stale, "an edit during the load keeps the result stale");
    assert_eq!(session.solidbom.current.as_ref().map(|saved| saved.calculation_id), Some(current.calculation_id));
}

#[test]
fn loading_a_calculation_needs_an_open_reviewed_project_and_a_service() {
    let command = crate::find_command("solidbom.calculation").expect("the command is registered");
    assert!(!command.info(&Session::default()).enabled);
    assert!(!command.undoable, "loading a saved result is not a CAD edit");
    let (mut session, fake) = fake_session();
    import_three_boxes(&mut session, &fake);
    assert!(command.info(&session).enabled, "an open reviewed project with a service may load a saved result");
    let error = session.execute("solidbom.calculation", &json!({"calculation_id": "not-a-uuid"})).expect_err("not a UUID");
    assert!(error.to_string().contains("`calculation_id` must be a UUID"), "{error}");
}

#[test]
fn a_session_without_a_service_still_imports_and_says_so() {
    let mut session = Session::default();
    let path = fixture("three_boxes.step");
    let reply = session.execute("solidbom.import", &json!({"path": path.to_string_lossy()})).expect("the local analysis works");
    assert_eq!(reply["needs_service"], json!(true));
    assert_eq!(reply["state"], json!("importing"));
    pump(&mut session);
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
    fake.complete(
        999,
        Ok(ClientReply::ReferenceData(ReferenceData { materials: Vec::new(), rates: Vec::new(), components: Vec::new(), app_config: None })),
    );
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

/// A reopen whose downloaded bytes do not verify never opens them: the live design (identity,
/// document and revision) stays exactly as it was, and no review policy is bound. Only the
/// SolidBOM draft and phase already belong to the loaded project.
#[test]
fn a_reopen_that_fails_its_source_verification_keeps_the_live_design() {
    let (mut session, fake) = fake_session();
    session.execute("solid.box", &json!({"length": 10, "width": 10, "height": 10})).expect("a design");
    let doc = session.doc.clone();
    let (uid, revision) = (session.uid, session.revision);
    session.execute("solidbom.reopen", &json!({"project_id": "00000000-0000-4000-8000-000000000001"})).expect("the request is submitted");
    let (job, _) = fake.take().into_iter().find(|(_, job)| matches!(job.request, ClientRequest::LoadProject(_))).expect("a load request");
    let source = SourceRef { source_id: uuid::Uuid::nil(), sha256: "a".repeat(64), byte_len: 1, original_name: "reviewed.step".into() };
    let draft = solidbom_contract::DraftProject {
        project_id: uuid::Uuid::from_u128(9),
        product_revision_id: uuid::Uuid::from_u128(10),
        version: 1,
        source: source.clone(),
        analysis: AnalysisManifest {
            schema_version: solidbom_contract::SCHEMA_VERSION,
            key: AnalysisKey { source_sha256: source.sha256.clone(), engine_build: "test".into(), extractor_version: "1".into() },
            definitions: Vec::new(),
            occurrences: Vec::new(),
            geometry_bindings: Vec::new(),
            issues: Vec::new(),
        },
        material_overhead_rate: None,
        parts: Vec::new(),
        forecast_year: 2027,
        forecast_quantity: 1000,
        currency: "CNY".into(),
    };
    fake.complete(job, Ok(ClientReply::Project(solidbom_contract::ProjectView { draft, calculations: Vec::new() })));
    session.drain_solidbom();
    let (job, _) = fake.take().into_iter().find(|(_, job)| matches!(job.request, ClientRequest::DownloadSource(_))).expect("a download");
    fake.complete(job, Ok(ClientReply::SourceBytes(source, b"this is not the verified source".to_vec())));
    session.drain_solidbom();
    assert!(session.review.is_none(), "no policy is bound to an unverified source");
    assert!(std::sync::Arc::ptr_eq(&doc, &session.doc), "the design is unchanged");
    assert_eq!((session.uid, session.revision), (uid, revision));
    assert!(
        session.solidbom.error.as_deref().is_some_and(|error| error.contains("SHA-256")),
        "the verification failure is reported: {:?}",
        session.solidbom.error
    );
}

use std::sync::mpsc::{Sender, channel};

use super::jobs::{AnalysisJob, AnalysisPurpose, AnalysisRequest, PreparedAnalysis};
use super::state::{Phase, State as SolidBomState, draft_from};

/// A finished scratch analysis of a checked-in fixture, for the controlled-channel tests: the
/// publication path is driven through the same channel the worker answers on, with no timing.
fn prepared_analysis(name: &str) -> (PreparedAnalysis, SourceRef, AnalysisKey) {
    let path = fixture(name);
    let bytes = std::fs::read(&path).expect("the fixture bytes");
    let key = key(&path);
    let session = open(&path);
    let manifest = extract(&session, key.clone()).expect("extract");
    let source = SourceRef {
        source_id: uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, format!("{}:source", key.source_sha256).as_bytes()),
        sha256: key.source_sha256.clone(),
        byte_len: bytes.len() as u64,
        original_name: name.into(),
    };
    (PreparedAnalysis { session, manifest, bytes }, source, key)
}

/// A pending analysis job whose answer the test sends: the channel is the transport, so the
/// pending state is asserted without waiting on a thread.
fn pending_import(
    session: &mut Session,
    name: &str,
) -> (Sender<std::result::Result<PreparedAnalysis, String>>, PreparedAnalysis, SourceRef, AnalysisKey) {
    let (prepared, source, key) = prepared_analysis(name);
    let (tx, rx) = channel();
    let path = fixture(name);
    let purpose = AnalysisPurpose::Import { source: source.clone(), source_path: path };
    let replacements = session.replacements;
    let binding = session.solidbom.binding_generation;
    session.solidbom_analysis = Some(AnalysisJob::pending(purpose, key.clone(), replacements, binding, session.solidbom.generation, rx));
    session.solidbom = SolidBomState { phase: Phase::Importing, ..Default::default() };
    (tx, prepared, source, key)
}

#[test]
fn an_import_answers_pending_and_publishes_when_the_analysis_verifies() {
    let mut session = Session::default();
    let reply = session.execute("solidbom.import", &json!({"path": fixture("three_boxes.step").to_string_lossy()})).expect("the import starts");
    assert_eq!(reply["state"], json!("importing"));
    assert_eq!(session.solidbom.phase, Phase::Importing);
    assert!(session.solidbom.draft.is_none());
    // The source is read and measured beside the window; the review state publishes it later.
    pump(&mut session);
    assert_eq!(session.solidbom.phase, Phase::Reviewing);
    assert!(session.review.is_some());
    assert!(session.solidbom.runtime.is_some());
    let draft = session.solidbom.draft.as_ref().expect("the published draft");
    assert_eq!(draft.analysis.definitions.len(), 1);
    assert_eq!(draft.analysis.occurrences.len(), 3);
    let runtime = session.solidbom.runtime.as_ref().expect("the runtime map");
    for occurrence in &draft.analysis.occurrences {
        let runtime_id = runtime.runtime_occurrence(&occurrence.source_path).expect("the map places the saved path");
        assert_eq!(runtime.source_path(runtime_id).expect("the map's path"), occurrence.source_path);
    }
}

#[test]
fn a_pending_analysis_keeps_selection_and_navigation_available() {
    let mut session = Session::default();
    session.execute("solid.box", &json!({"length": 10, "width": 10, "height": 10})).expect("the box");
    let (tx, _prepared, _source, _key) = pending_import(&mut session, "three_boxes.step");
    // The window keeps answering navigation, selection and inspection while the analysis runs.
    session.execute("select.set", &json!({"items": []})).expect("selection is available");
    session.execute("document.inspect", &json!({})).expect("inspection is available");
    assert_eq!(session.solidbom.phase, Phase::Importing);
    assert!(session.review.is_none(), "the analysis is not bound yet");
    drop(tx);
}

#[test]
fn a_second_import_is_refused_while_one_is_being_analysed() {
    let mut session = Session::default();
    let (tx, _prepared, _source, _key) = pending_import(&mut session, "three_boxes.step");
    let error = session.execute("solidbom.import", &json!({"path": fixture("revisions_bc.step").to_string_lossy()})).expect_err("refused");
    assert!(error.to_string().contains("already being opened"), "{error}");
    // Reopen cannot start a second source either.
    let error = session.execute("solidbom.reopen", &json!({"project_id": "00000000-0000-4000-8000-000000000001"})).expect_err("refused");
    assert!(error.to_string().contains("already being opened"), "{error}");
    drop(tx);
}

#[test]
fn save_and_calculate_report_an_opening_project_rather_than_no_project() {
    let mut session = Session::default();
    let (tx, _prepared, _source, _key) = pending_import(&mut session, "three_boxes.step");
    for id in ["solidbom.save", "solidbom.calculate"] {
        let error = session.execute(id, &json!({})).expect_err(id);
        assert!(error.to_string().contains("still opening"), "{id}: {error}");
        let info = crate::find_command(id).expect("the command").info(&session);
        assert!(!info.enabled, "{id} is unavailable while the analysis runs");
        assert!(info.disabled_reason.as_ref().is_some_and(|reason| reason.contains("still opening")), "{id}: {info:?}");
    }
    drop(tx);
}

#[test]
fn a_completion_for_a_newer_binding_is_discarded() {
    let mut session = Session::default();
    let (tx, prepared, _source, _key) = pending_import(&mut session, "three_boxes.step");
    // A binding generation moved after submission: the answer describes an older record.
    session.solidbom.binding_generation = session.solidbom.binding_generation.wrapping_add(1);
    tx.send(Ok(prepared)).expect("deliver the answer");
    session.drain_solidbom_analysis();
    assert!(session.review.is_none(), "the stale analysis is not published");
    assert!(session.solidbom.draft.is_none());
    assert_eq!(session.solidbom.phase, Phase::Idle);
    assert!(session.solidbom.error.as_deref().is_some_and(|error| error.contains("binding changed")));
}

#[test]
fn a_replaced_design_discards_its_pending_analysis() {
    let mut session = Session::default();
    let (_tx, _prepared, _source, _key) = pending_import(&mut session, "three_boxes.step");
    assert!(session.solidbom_analysis.is_some());
    session.replace_design(Session::default());
    assert!(session.solidbom_analysis.is_none(), "the job belonged to the replaced design");
    assert_eq!(session.solidbom.phase, Phase::Idle);
}

#[test]
fn a_worker_that_cannot_start_is_a_visible_failure_without_synchronous_fallback() {
    let path = fixture("three_boxes.step");
    let bytes = std::fs::read(&path).expect("the fixture bytes");
    let key = key(&path);
    let source = SourceRef {
        source_id: uuid::Uuid::nil(),
        sha256: key.source_sha256.clone(),
        byte_len: bytes.len() as u64,
        original_name: "three_boxes.step".into(),
    };
    let request = AnalysisRequest {
        bytes,
        original_name: "three_boxes.step".into(),
        key,
        purpose: AnalysisPurpose::Import { source, source_path: path },
        command_check: None,
        validation: Session::default().validation(),
        param_check: Default::default(),
        auto_project: true,
    };
    let error = AnalysisJob::start_with(request, 0, 0, 0, Box::new(|_work| Err(std::io::Error::other("no worker threads"))))
        .expect_err("the worker cannot start");
    assert!(matches!(error, crate::EngineError::Capability(_)), "{error}");
    assert!(error.to_string().contains("did not start"), "{error}");
}

#[test]
fn a_reopen_verifies_the_saved_mapping_and_binds_the_policy() {
    let (imported, source, _key) = prepared_analysis("three_boxes.step");
    let mut draft = draft_from(&imported.manifest, &source);
    draft.version = 1;
    let mut session = Session { solidbom_host: None, ..Default::default() };
    let fake = Arc::new(FakeHost::default());
    session.solidbom_host = Some(fake.clone());
    session.execute("solidbom.reopen", &json!({"project_id": draft.project_id.to_string()})).expect("the reopen starts");
    let (job, _) = fake.take().into_iter().find(|(_, job)| matches!(job.request, ClientRequest::LoadProject(_))).expect("a load");
    fake.complete(job, Ok(ClientReply::Project(solidbom_contract::ProjectView { draft: draft.clone(), calculations: Vec::new() })));
    session.drain_solidbom();
    assert_eq!(session.solidbom.phase, Phase::Importing, "the downloaded source is still unverified");
    let (job, _) = fake.take().into_iter().find(|(_, job)| matches!(job.request, ClientRequest::DownloadSource(_))).expect("a download");
    fake.complete(job, Ok(ClientReply::SourceBytes(source.clone(), imported.bytes.clone())));
    session.drain_solidbom();
    pump(&mut session);
    assert_eq!(session.solidbom.phase, Phase::Reviewing, "{:?}", session.solidbom.error);
    assert!(session.review.is_some(), "the verified source is bound read-only");
    assert_eq!(session.solidbom.save, SaveState::Clean);
    let restored = session.solidbom.draft.as_ref().expect("the reopened draft");
    assert_eq!(restored.source.sha256, source.sha256);
    assert_eq!(restored.analysis, draft.analysis, "the verified mapping is the saved one");
    // Every saved occurrence path maps to a current runtime id and back: the restored mapping is
    // verified, not assumed.
    let runtime = session.solidbom.runtime.as_ref().expect("the rebuilt runtime map");
    for occurrence in &restored.analysis.occurrences {
        let runtime_id = runtime.runtime_occurrence(&occurrence.source_path).expect("the map places the saved path");
        let picked = resolve_pick(&session, &restored.analysis, &Sel::Occurrence { path: vec![runtime_id] }).expect("the occurrence resolves");
        assert_eq!(picked.occurrence_id, occurrence.occurrence_id);
    }
    // The reviewed source is read-only again.
    assert!(matches!(session.execute("solid.box", &json!({"length": 1, "width": 1, "height": 1})), Err(crate::EngineError::Refused { .. })));
    let _ = fake.take();
}

#[test]
fn a_reopen_from_an_older_extractor_re_analyses_and_keeps_the_reviewed_inputs() {
    let (imported, source, current_key) = prepared_analysis("three_boxes.step");
    let mut saved = draft_from(&imported.manifest, &source);
    saved.version = 1;
    // The saved analysis was made by an older extractor: same source, another analysis key.
    saved.analysis.key = AnalysisKey { engine_build: "0.0.1".into(), extractor_version: "1".into(), ..current_key.clone() };
    for definition in &mut saved.analysis.definitions {
        definition.source_key = saved.analysis.key.clone();
        definition.volume_mm3.analysis_key = saved.analysis.key.clone();
    }
    for part in &mut saved.parts {
        part.volume_mm3.analysis_key = saved.analysis.key.clone();
    }
    // A reviewed decision and a manual measurement must survive the re-analysis.
    let first = &mut saved.parts[0];
    first.classification = "clip".into();
    first.classification_confirmed = true;
    first.volume_mm3.source = solidbom_contract::MeasurementSource::Manual;
    first.volume_mm3.value = Some(decimal("2400"));
    first.volume_mm3.reason = Some("measured by hand".into());
    first.volume_mm3.review = ReviewState::Confirmed;
    first.allowance_rate = Some(decimal("0.02"));
    first.operation = Some(solidbom_contract::OperationInputs {
        cycle_seconds: Some(decimal("30")),
        cycle_basis: solidbom_contract::CycleBasis::PerPiece,
        cavities: Some(1),
        production_rate_version_id: None,
        setup_hours: Some(decimal("0.5")),
        setup_rate_version_id: None,
    });
    let mut session = Session::default();
    let fake = Arc::new(FakeHost::default());
    session.solidbom_host = Some(fake.clone());
    session.execute("solidbom.reopen", &json!({"project_id": saved.project_id.to_string()})).expect("the reopen starts");
    let (job, _) = fake.take().into_iter().find(|(_, job)| matches!(job.request, ClientRequest::LoadProject(_))).expect("a load");
    fake.complete(job, Ok(ClientReply::Project(solidbom_contract::ProjectView { draft: saved, calculations: Vec::new() })));
    session.drain_solidbom();
    let (job, _) = fake.take().into_iter().find(|(_, job)| matches!(job.request, ClientRequest::DownloadSource(_))).expect("a download");
    fake.complete(job, Ok(ClientReply::SourceBytes(source.clone(), imported.bytes.clone())));
    session.drain_solidbom();
    pump(&mut session);
    assert_eq!(session.solidbom.phase, Phase::Reviewing, "{:?}", session.solidbom.error);
    let draft = session.solidbom.draft.as_ref().expect("the reopened draft");
    assert_eq!(draft.analysis.key, current_key, "the analysis is the current extraction");
    assert_eq!(session.solidbom.save, SaveState::Dirty, "the re-analysed project needs a save");
    assert_eq!(session.solidbom.result, ControllerResult::Stale);
    let part = &draft.parts[0];
    assert_eq!(part.classification, "clip");
    assert!(part.classification_confirmed);
    assert_eq!(part.volume_mm3.value, Some(decimal("2400")), "a manual measurement stays reviewed");
    assert_eq!(part.volume_mm3.analysis_key, current_key, "it is re-anchored to the current analysis");
    assert_eq!(part.allowance_rate, Some(decimal("0.02")));
    assert_eq!(part.operation.as_ref().and_then(|operation| operation.cycle_seconds), Some(decimal("30")));
    // Reference data is loaded and calculate is available (it saves first).
    let (job, _) = fake.take().into_iter().find(|(_, job)| matches!(job.request, ClientRequest::ReferenceData)).expect("reference data");
    fake.complete(
        job,
        Ok(ClientReply::ReferenceData(ReferenceData { materials: Vec::new(), rates: Vec::new(), components: Vec::new(), app_config: None })),
    );
    session.drain_solidbom();
    session.execute("solidbom.calculate", &json!({})).expect("calculate saves the re-analysed inputs first");
    assert!(matches!(session.solidbom.save, SaveState::Saving { .. }));
}

#[test]
fn an_analysis_without_a_usable_mapping_is_not_published() {
    let mut session = Session::default();
    let (tx, prepared, source, key) = pending_import(&mut session, "three_boxes.step");
    // The manifest is valid, but the session it claims to describe has no imported source: the
    // runtime map cannot be built, so the source must not be bound or published.
    let unmapped = PreparedAnalysis { session: Session::default(), manifest: prepared.manifest, bytes: prepared.bytes };
    tx.send(Ok(unmapped)).expect("deliver the answer");
    session.drain_solidbom_analysis();
    assert!(session.review.is_none(), "no policy is bound to an unverifiable mapping");
    assert!(session.solidbom.draft.is_none());
    assert_eq!(session.solidbom.phase, Phase::Idle);
    assert!(session.solidbom.error.as_deref().is_some_and(|error| error.contains("occurrence mapping")), "{:?}", session.solidbom.error);
    assert!(key.validate().is_ok());
    assert_eq!(source.sha256.len(), 64);
}

#[test]
fn cancelling_a_session_stops_and_discards_its_pending_analysis() {
    let mut session = Session::default();
    let (tx, prepared, _source, _key) = pending_import(&mut session, "three_boxes.step");
    session.cancel_solidbom();
    assert!(session.solidbom_analysis.is_none(), "the job is dropped");
    assert_eq!(session.solidbom.phase, Phase::Idle);
    assert!(session.solidbom.notice.as_deref().is_some_and(|notice| notice.contains("cancelled")));
    // A late answer for the cancelled job has nowhere to go and publishes nothing.
    let _ = tx.send(Ok(prepared));
    session.drain_solidbom_analysis();
    assert!(session.review.is_none());
    assert!(session.solidbom.draft.is_none());
}

#[test]
fn a_reopen_keeps_an_input_edit_made_while_it_opened_dirty() {
    let (imported, source, _key) = prepared_analysis("three_boxes.step");
    let mut draft = draft_from(&imported.manifest, &source);
    draft.version = 1;
    let mut session = Session::default();
    let fake = Arc::new(FakeHost::default());
    session.solidbom_host = Some(fake.clone());
    session.execute("solidbom.reopen", &json!({"project_id": draft.project_id.to_string()})).expect("the reopen starts");
    let (job, _) = fake.take().into_iter().find(|(_, job)| matches!(job.request, ClientRequest::LoadProject(_))).expect("a load");
    fake.complete(job, Ok(ClientReply::Project(solidbom_contract::ProjectView { draft: draft.clone(), calculations: Vec::new() })));
    session.drain_solidbom();
    // The project is loaded and its source is still downloading: a reviewed edit made now must
    // not become saved when the verified source publishes a moment later.
    let part = draft.parts[0].part_version_id.to_string();
    session
        .execute("solidbom.inputs", &json!({"part_version_id": part, "classification": "clip", "classification_confirmed": true}))
        .expect("the edit");
    let after_edit = session.solidbom.generation;
    let (job, _) = fake.take().into_iter().find(|(_, job)| matches!(job.request, ClientRequest::DownloadSource(_))).expect("a download");
    fake.complete(job, Ok(ClientReply::SourceBytes(source.clone(), imported.bytes.clone())));
    session.drain_solidbom();
    pump(&mut session);
    assert_eq!(session.solidbom.phase, Phase::Reviewing, "{:?}", session.solidbom.error);
    assert_eq!(session.solidbom.save, SaveState::Dirty, "the edit made while opening stays unsaved");
    assert_eq!(session.solidbom.generation, after_edit, "the edit is kept");
    assert!(session.solidbom.current.is_none(), "no saved result is shown as current over newer inputs");
    assert!(session.solidbom.notice.as_deref().is_some_and(|notice| notice.contains("changed while it opened")));
}

fn administrator() -> Identity {
    Identity {
        identity_id: uuid::Uuid::from_u128(1),
        name: "administrator".into(),
        permissions: vec![Permission::Engineer, Permission::Administrator],
    }
}

fn material_version(version: u64, price: Option<&str>) -> MaterialVersion {
    MaterialVersion {
        material_version_id: uuid::Uuid::from_u128(100 + u128::from(version)),
        material_id: uuid::Uuid::from_u128(100),
        version,
        supersedes: (version > 1).then_some(uuid::Uuid::from_u128(100 + u128::from(version - 1))),
        name: "PA12".into(),
        density_g_cm3: decimal("1.01"),
        density_unit: Unit::GramPerCentimetreCubed,
        price_per_kg: price.map(decimal),
        price_unit: Unit::CurrencyPerKilogram,
        currency: "CNY".into(),
        source_reference: None,
        created_at: None,
    }
}

fn rate_version(version: u64, rate: Option<&str>) -> RateVersion {
    RateVersion {
        rate_version_id: uuid::Uuid::from_u128(200 + u128::from(version)),
        cost_center_id: uuid::Uuid::from_u128(200),
        version,
        supersedes: (version > 1).then_some(uuid::Uuid::from_u128(200 + u128::from(version - 1))),
        name: "Machining".into(),
        purpose: CostCenterPurpose::Production,
        rate_per_hour: rate.map(decimal),
        rate_unit: Unit::CurrencyPerHour,
        currency: "CNY".into(),
        source_reference: None,
        created_at: None,
    }
}

fn change(kind: SharedRecordKind, record_id: uuid::Uuid, version_id: uuid::Uuid, version: u64) -> ReferenceChange {
    ReferenceChange {
        change_id: uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, version_id.as_bytes()),
        kind,
        record_id,
        version_id,
        version,
        previous_version_id: None,
        summary: "published".into(),
        changed_by: "administrator".into(),
        changed_at: "2026-11-09T00:00:00Z".into(),
    }
}

fn shared_view(identity: Identity, material_versions: Vec<MaterialVersion>, rate_versions: Vec<RateVersion>) -> SharedDataView {
    SharedDataView {
        identity,
        materials: material_versions,
        cost_centers: rate_versions,
        components: Vec::new(),
        app_config: None,
        app_config_versions: Vec::new(),
        changes: Vec::new(),
    }
}

/// Take the one request of a kind the session submitted and complete the identity answer the
/// fake host queues by itself.
fn take_request(fake: &Arc<FakeHost>, want: impl Fn(&ClientRequest) -> bool) -> (JobId, ClientRequest) {
    fake.take()
        .into_iter()
        .find_map(|(job, submitted)| want(&submitted.request).then_some((job, submitted.request)))
        .expect("the request was submitted")
}

#[test]
fn the_identity_loads_with_the_reference_data() {
    let (mut session, fake) = fake_session();
    import_three_boxes(&mut session, &fake);
    let identity = session.solidbom.identity.as_ref().expect("the permission sets are loaded");
    assert!(identity.can_work_on_projects());
    assert!(identity.can_manage_shared_data());
}

#[test]
fn a_forbidden_shared_request_is_surfaced_and_never_retried() {
    let (mut session, fake) = fake_session();
    session.execute("solidbom.shared", &json!({})).expect("the shared data is requested");
    let (job, _) = take_request(&fake, |request| matches!(request, ClientRequest::SharedData));
    fake.complete(job, Err(ClientFailure::Forbidden));
    session.drain_solidbom();
    let error = session.solidbom.error.as_deref().expect("the refusal is shown");
    assert!(error.contains("administrator"), "the refusal names the permission: {error}");
    let failed = session.solidbom.failed.as_ref().expect("the failed request is retained");
    assert!(!failed.retryable, "a wrong permission set cannot be retried unchanged");
    assert!(session.solidbom.shared.is_none(), "nothing becomes the shared view");
    assert!(session.execute("solidbom.retry", &json!({})).is_err(), "the retry command refuses it");
}

#[test]
fn a_published_material_version_refreshes_the_shared_view() {
    let (mut session, fake) = fake_session();
    session.execute("solidbom.shared", &json!({})).expect("the shared data is requested");
    let (job, _) = take_request(&fake, |request| matches!(request, ClientRequest::SharedData));
    fake.complete(job, Ok(ClientReply::SharedData(shared_view(administrator(), vec![material_version(1, Some("20"))], vec![]))));
    session.drain_solidbom();
    assert!(session.solidbom.shared.is_some());
    assert!(session.solidbom.is_administrator());

    session
        .execute("solidbom.shared.material.version", &json!({"name": "PA12", "density_g_cm3": "1.01", "price_per_kg": "0"}))
        .expect("the material version is published");
    let (job, request) = take_request(&fake, |request| matches!(request, ClientRequest::CreateMaterialVersion(_)));
    let ClientRequest::CreateMaterialVersion(create) = request else { panic!("the request is a material version publish") };
    assert_eq!(create.expected_version, 0, "a blank record starts version 1");
    assert_eq!(create.price_per_kg, Some(decimal("0")), "an explicit zero stays a value");
    let version = material_version(2, Some("0"));
    fake.complete(
        job,
        Ok(ClientReply::MaterialVersion(MaterialVersionPublished {
            material: version.clone(),
            change: change(SharedRecordKind::Material, version.material_id, version.material_version_id, 2),
        })),
    );
    session.drain_solidbom();
    assert!(session.solidbom.notice.as_deref().is_some_and(|notice| notice.contains("published")), "{:?}", session.solidbom.notice);
    // The successful publish reloads the view from the service rather than trusting its own copy.
    let (refresh, _) = take_request(&fake, |request| matches!(request, ClientRequest::SharedData));
    let mut refreshed = shared_view(administrator(), vec![material_version(1, Some("20")), version], vec![]);
    refreshed.changes.push(change(SharedRecordKind::Material, uuid::Uuid::from_u128(100), uuid::Uuid::from_u128(102), 2));
    fake.complete(refresh, Ok(ClientReply::SharedData(refreshed)));
    session.drain_solidbom();
    let shared = session.solidbom.shared.as_ref().expect("the refreshed view");
    assert_eq!(shared.materials.len(), 2, "the refresh carries the whole version chain");
    assert!(!shared.changes.is_empty(), "the change facts come from the service");
}

#[test]
fn a_published_rate_version_refreshes_the_shared_view() {
    let (mut session, fake) = fake_session();
    session
        .execute("solidbom.shared.cost_center.version", &json!({"name": "Machining", "purpose": "setup", "rate_per_hour": "0"}))
        .expect("the rate version is published");
    let (job, request) = take_request(&fake, |request| matches!(request, ClientRequest::CreateRateVersion(_)));
    let ClientRequest::CreateRateVersion(create) = request else { panic!("the request is a rate version publish") };
    assert_eq!(create.purpose, CostCenterPurpose::Setup);
    assert_eq!(create.rate_per_hour, Some(decimal("0")));
    let mut version = rate_version(1, Some("0"));
    version.purpose = CostCenterPurpose::Setup;
    fake.complete(
        job,
        Ok(ClientReply::RateVersion(RateVersionPublished {
            cost_center: version.clone(),
            change: change(SharedRecordKind::CostCenter, version.cost_center_id, version.rate_version_id, 1),
        })),
    );
    session.drain_solidbom();
    let (refresh, _) = take_request(&fake, |request| matches!(request, ClientRequest::SharedData));
    fake.complete(refresh, Ok(ClientReply::SharedData(shared_view(administrator(), vec![], vec![version]))));
    session.drain_solidbom();
    let shared = session.solidbom.shared.as_ref().expect("the refreshed view");
    assert_eq!(shared.cost_centers.len(), 1);
    assert_eq!(shared.cost_centers[0].purpose, CostCenterPurpose::Setup);
    assert_eq!(shared.cost_centers[0].rate_per_hour, Some(decimal("0")), "an explicit zero rate survives the refresh");
}

#[test]
fn a_blank_price_is_missing_and_a_zero_price_is_explicit_for_every_publish_command() {
    let (mut session, fake) = fake_session();
    session
        .execute("solidbom.shared.material.version", &json!({"name": "PA12 missing", "density_g_cm3": "1.01", "price_per_kg": ""}))
        .expect("the material version is published");
    let (_, request) = take_request(&fake, |request| matches!(request, ClientRequest::CreateMaterialVersion(_)));
    let ClientRequest::CreateMaterialVersion(create) = request else { panic!("the request is a material version publish") };
    assert_eq!(create.price_per_kg, None, "a blank price is missing, not zero");

    session
        .execute("solidbom.shared.material.version", &json!({"name": "PA12 zero", "density_g_cm3": "1.01", "price_per_kg": "0"}))
        .expect("the material version is published");
    let (_, request) = take_request(&fake, |request| matches!(request, ClientRequest::CreateMaterialVersion(_)));
    let ClientRequest::CreateMaterialVersion(create) = request else { panic!("the request is a material version publish") };
    assert_eq!(create.price_per_kg, Some(decimal("0")), "an explicit zero stays a value");

    session
        .execute("solidbom.shared.cost_center.version", &json!({"name": "Machining missing", "rate_per_hour": ""}))
        .expect("the rate version is published");
    let (_, request) = take_request(&fake, |request| matches!(request, ClientRequest::CreateRateVersion(_)));
    let ClientRequest::CreateRateVersion(create) = request else { panic!("the request is a rate version publish") };
    assert_eq!(create.rate_per_hour, None);

    session
        .execute("solidbom.shared.cost_center.version", &json!({"name": "Machining zero", "rate_per_hour": "0"}))
        .expect("the rate version is published");
    let (_, request) = take_request(&fake, |request| matches!(request, ClientRequest::CreateRateVersion(_)));
    let ClientRequest::CreateRateVersion(create) = request else { panic!("the request is a rate version publish") };
    assert_eq!(create.rate_per_hour, Some(decimal("0")));
}

#[test]
fn the_shared_publish_commands_refuse_hostile_parameters_as_bad_params() {
    let (mut session, _fake) = fake_session();
    for (command, params) in [
        ("solidbom.shared.material.version", json!({"name": "PA12", "density_g_cm3": "not a number"})),
        ("solidbom.shared.material.version", json!({"name": "PA12", "density_g_cm3": "0"})),
        ("solidbom.shared.material.version", json!({"name": "PA12", "density_g_cm3": "-1"})),
        ("solidbom.shared.material.version", json!({"name": "", "density_g_cm3": "1.01"})),
        ("solidbom.shared.material.version", json!({"material_id": "not a uuid", "name": "PA12", "density_g_cm3": "1.01"})),
        ("solidbom.shared.material.version", json!({"name": "PA12", "density_g_cm3": "1.01", "price_per_kg": "-1"})),
        ("solidbom.shared.material.version", json!({"name": "PA12", "density_g_cm3": "1.01", "currency": "cny"})),
        ("solidbom.shared.cost_center.version", json!({"name": "Machining", "purpose": "assembly"})),
        ("solidbom.shared.cost_center.version", json!({"name": "Machining", "rate_per_hour": "0.1.2"})),
        ("solidbom.shared.component.version", json!({"name": "Bolt", "specification": "M6", "price": "free"})),
        (
            "solidbom.shared.app_config.version",
            json!({"currency": "CNY", "categories": [{"code": "overhead", "name": "Overhead", "calculation": {"kind": "percentage", "basis": {}, "rate": "0.05"}, "included_in_ebit": true}]}),
        ),
        (
            "solidbom.shared.app_config.version",
            json!({"currency": "CNY", "categories": [{"code": "setup", "name": "Setup", "calculation": {"kind": "direct", "input": "setup"}, "included_in_ebit": true}]}),
        ),
        ("solidbom.shared.app_config.version", json!({"currency": "CNY", "categories": "not a list"})),
    ] {
        let error = session.execute(command, &params).expect_err(command);
        assert!(matches!(error, crate::EngineError::BadParams { .. }), "{command}: {error}");
    }
    assert!(session.execute("solidbom.shared", &json!({"reload": true})).is_ok(), "the reload command takes no parameters");
}

#[test]
fn a_publish_retry_resends_the_same_request_id() {
    let (mut session, fake) = fake_session();
    session
        .execute("solidbom.shared.material.version", &json!({"name": "PA12", "density_g_cm3": "1.01", "price_per_kg": "20"}))
        .expect("the version is published");
    let (job, request) = take_request(&fake, |request| matches!(request, ClientRequest::CreateMaterialVersion(_)));
    let ClientRequest::CreateMaterialVersion(first) = request else { panic!("the request is a material version publish") };
    fake.complete(job, Err(ClientFailure::Network("offline".into())));
    session.drain_solidbom();
    assert!(session.solidbom.failed.as_ref().is_some_and(|failed| failed.retryable), "a network failure is retryable");
    session.solidbom_retry().expect("the retry resubmits the shared write");
    let (_, request) = take_request(&fake, |request| matches!(request, ClientRequest::CreateMaterialVersion(_)));
    let ClientRequest::CreateMaterialVersion(retried) = request else { panic!("the request is a material version publish") };
    assert_eq!(retried.request_id, first.request_id, "the service sees one request id, so the retry stays idempotent");
    assert_eq!(retried.price_per_kg, first.price_per_kg);
}

#[test]
fn a_shared_request_carries_no_project_or_analysis_in_its_envelope() {
    let (mut session, fake) = fake_session();
    import_three_boxes(&mut session, &fake);
    // A reviewed project is open, but the shared request belongs to the organization.
    session.execute("solidbom.shared", &json!({})).expect("the shared data is requested");
    let (_, submitted) = fake.take().into_iter().find(|(_, job)| matches!(job.request, ClientRequest::SharedData)).expect("a shared-data request");
    assert_eq!(submitted.envelope.project_id, None);
    assert_eq!(submitted.envelope.analysis_key, None);
}

/// A project with a saved and calculated current result, ready to export the draft.
fn project_with_current_calculation(session: &mut Session, fake: &Arc<FakeHost>) -> CalculationSnapshot {
    import_three_boxes(session, fake);
    edit(session);
    session.solidbom_calculate().expect("calculate saves first");
    let (save_job, save) = take_save(fake);
    let receipt = SaveReceipt { request_id: save.request_id.clone(), project_id: save.draft.project_id, version: 2 };
    fake.complete(save_job, Ok(ClientReply::Receipt(receipt)));
    session.drain_solidbom();
    let (job, request) = take_calculate(fake);
    let current = snapshot(request.project_id, 2);
    fake.complete(job, Ok(ClientReply::Calculation(current.clone())));
    session.drain_solidbom();
    current
}

/// One validated export bundle whose bytes and digest agree, as the service answers them.
fn export_bundle(project_id: uuid::Uuid, baseline: ExportBaselineSelection, format: ExportFormat, bytes: &[u8]) -> ExportBundle {
    let calculation_id = match baseline {
        ExportBaselineSelection::DraftCalculation { calculation_id } => calculation_id,
        ExportBaselineSelection::AcceptedBaseline { accepted_baseline_id, .. } => accepted_baseline_id,
    };
    let labels = match baseline {
        ExportBaselineSelection::DraftCalculation { .. } => vec![ExportLabel::DraftResult],
        ExportBaselineSelection::AcceptedBaseline { .. } => Vec::new(),
    };
    ExportBundle {
        metadata: ExportMetadata {
            project_id,
            project_name: "Three boxes".into(),
            customer: "Internal".into(),
            currency: "CNY".into(),
            exported_at: "2026-11-11T13:00:00Z".into(),
            format,
            baseline,
            versions: ExportRecordVersions {
                project_version: 1,
                engineering_version: 0,
                manufacturing_version: 0,
                feasibility_version: 0,
                // A draft calculation snapshot pins no feasibility record; an accepted baseline pins
                // the version activation read.
                feasibility_pinned: matches!(baseline, ExportBaselineSelection::AcceptedBaseline { .. }),
            },
            calculation: ExportCalculation {
                calculation_id,
                calculator_version: "test".into(),
                input_hash: "a".repeat(64),
                state: OutcomeState::Complete,
            },
            labels,
            stale: false,
            stale_reasons: Vec::new(),
        },
        file_name: format!("three-boxes-draft.{}", format.file_extension()),
        media_type: format.media_type().into(),
        byte_len: bytes.len() as u64,
        sha256: hex(bytes),
        bytes: bytes.to_vec(),
    }
}

/// A destination under the temporary folder that only one test uses.
fn export_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("solidbom-export-{}-{name}", std::process::id()))
}

#[test]
fn an_export_of_the_named_draft_calculation_writes_the_returned_bytes() {
    let (mut session, fake) = fake_session();
    let current = project_with_current_calculation(&mut session, &fake);
    let project_id = session.solidbom.project_id().expect("the project");
    let path = export_path("draft.xlsx");
    let reply = session
        .execute("solidbom.export", &json!({"path": path.to_string_lossy(), "format": "xlsx", "calculation_id": current.calculation_id.to_string()}))
        .expect("the export starts");
    assert!(reply["request_id"].as_str().is_some_and(|id| id.parse::<u64>().is_ok()), "the reply names the host job id: {reply}");

    let (job, request) = take_request(&fake, |request| matches!(request, ClientRequest::Export(_)));
    let ClientRequest::Export(request) = request else { panic!("the request is an export") };
    assert_eq!(request.project_id, project_id, "the export is sent for the open project");
    assert_eq!(
        request.baseline,
        ExportBaselineRef::DraftCalculation { calculation_id: current.calculation_id },
        "the named SB-16 calculation is exported"
    );
    assert_eq!(request.format, ExportFormat::Xlsx);
    assert!(request.scenarios.is_empty());

    let bytes = b"rendered workbook".to_vec();
    let bundle =
        export_bundle(project_id, ExportBaselineSelection::DraftCalculation { calculation_id: current.calculation_id }, ExportFormat::Xlsx, &bytes);
    fake.complete(job, Ok(ClientReply::Export(bundle)));
    session.drain_solidbom();

    assert_eq!(std::fs::read(&path).expect("the written document"), bytes, "the validated bytes are written to the destination");
    let Some(super::state::ExportOutcome::Written(file)) = &session.solidbom.export else {
        panic!("the export outcome: {:?}", session.solidbom.export)
    };
    assert_eq!(file.path, path);
    assert_eq!(file.byte_len, bytes.len() as u64);
    assert_eq!(file.sha256, hex(&bytes));
    assert_eq!(file.format, ExportFormat::Xlsx);
    assert_eq!(file.baseline, ExportBaselineSelection::DraftCalculation { calculation_id: current.calculation_id });
    assert_eq!(file.labels, vec![ExportLabel::DraftResult]);
    assert!(!file.stale);
    assert!(file.stale_reasons.is_empty());
    assert!(
        session.solidbom.notice.as_deref().is_some_and(|notice| notice.contains(&path.display().to_string())),
        "the notice names the written path: {:?}",
        session.solidbom.notice
    );
    assert!(session.solidbom.error.is_none());
    assert!(session.solidbom.pending.is_empty());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn an_export_of_an_accepted_baseline_carries_its_id_and_sequence() {
    let (mut session, fake) = fake_session();
    import_three_boxes(&mut session, &fake);
    // No calculation is current: an accepted baseline is exactly what makes this export possible.
    assert!(session.solidbom.current.is_none());
    let accepted_baseline_id = uuid::Uuid::from_u128(900);
    let path = export_path("accepted.xlsx");
    let reply = session
        .execute(
            "solidbom.export",
            &json!({"path": path.to_string_lossy(), "accepted_baseline_id": accepted_baseline_id.to_string(), "accepted_baseline_sequence": 3}),
        )
        .expect("the export starts");
    assert!(reply["request_id"].as_str().is_some(), "the reply names the job: {reply}");

    let (job, request) = take_request(&fake, |request| matches!(request, ClientRequest::Export(_)));
    let ClientRequest::Export(request) = request else { panic!("the request is an export") };
    assert_eq!(request.baseline, ExportBaselineRef::AcceptedBaseline { accepted_baseline_id, sequence: 3 });
    assert_eq!(request.format, ExportFormat::Xlsx, "the workbook is the default format");

    let project_id = request.project_id;
    let baseline = ExportBaselineSelection::AcceptedBaseline { accepted_baseline_id, sequence: 3 };
    let bytes = b"rendered accepted baseline".to_vec();
    fake.complete(job, Ok(ClientReply::Export(export_bundle(project_id, baseline, ExportFormat::Xlsx, &bytes))));
    session.drain_solidbom();
    assert_eq!(std::fs::read(&path).expect("the written document"), bytes);
    let Some(super::state::ExportOutcome::Written(file)) = &session.solidbom.export else {
        panic!("the export outcome: {:?}", session.solidbom.export)
    };
    assert_eq!(file.baseline, baseline, "the outcome keeps the exact baseline the documents state");
    assert!(file.labels.is_empty());
    assert!(!file.stale);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn an_export_with_a_bundle_that_does_not_match_its_digest_is_refused_and_writes_no_file() {
    let (mut session, fake) = fake_session();
    let current = project_with_current_calculation(&mut session, &fake);
    let project_id = session.solidbom.project_id().expect("the project");
    let path = export_path("bad-digest.xlsx");
    session
        .execute("solidbom.export", &json!({"path": path.to_string_lossy(), "format": "xlsx", "calculation_id": current.calculation_id.to_string()}))
        .expect("the export starts");
    let (job, request) = take_request(&fake, |request| matches!(request, ClientRequest::Export(_)));
    let ClientRequest::Export(_request) = request else { panic!("the request is an export") };
    let mut bundle = export_bundle(
        project_id,
        ExportBaselineSelection::DraftCalculation { calculation_id: current.calculation_id },
        ExportFormat::Xlsx,
        b"tampered document",
    );
    bundle.sha256 = "b".repeat(64);
    fake.complete(job, Ok(ClientReply::Export(bundle)));
    session.drain_solidbom();

    assert!(!path.exists(), "an answer that does not match its digest writes no file");
    let Some(super::state::ExportOutcome::Failed(super::state::ExportFailure::InvalidBundle(message))) = &session.solidbom.export else {
        panic!("the export outcome: {:?}", session.solidbom.export)
    };
    assert!(message.contains("SHA-256"), "the refusal names the digest: {message}");
    assert!(session.solidbom.error.as_deref().is_some_and(|error| error.contains("refused")), "{:?}", session.solidbom.error);
    assert!(session.solidbom.failed.is_none(), "an inconsistent answer is not retried unchanged");
    assert!(fake.take().is_empty(), "nothing else is submitted");
}

#[test]
#[cfg(unix)]
fn a_failed_export_write_preserves_the_previous_file() {
    // The controller writes atomically: the document goes to a temporary file beside the
    // destination and is renamed into place, so a write that cannot complete leaves the existing
    // export untouched instead of truncating it. A read-only directory makes the temporary file
    // creation fail, which is the closest a test can get to a full disk.
    use std::os::unix::fs::PermissionsExt;

    let (mut session, fake) = fake_session();
    let current = project_with_current_calculation(&mut session, &fake);
    let project_id = session.solidbom.project_id().expect("the project");
    let directory = std::env::temp_dir().join(format!("solidbom-export-atomic-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("the export directory");
    let path = directory.join("export.xlsx");
    std::fs::write(&path, b"previous export").expect("the previous export");
    let writable = std::fs::metadata(&directory).expect("the directory").permissions();
    let mut read_only = writable.clone();
    read_only.set_mode(0o555);
    std::fs::set_permissions(&directory, read_only).expect("a read-only directory");

    let request = ExportRequest {
        project_id,
        baseline: ExportBaselineRef::DraftCalculation { calculation_id: current.calculation_id },
        scenarios: Vec::new(),
        format: ExportFormat::Xlsx,
    };
    session.solidbom.export = None;
    let _ = session.solidbom_export(request, path.clone());
    let job = fake.take().into_iter().find_map(|(job, submitted)| matches!(submitted.request, ClientRequest::Export(_)).then_some(job));
    if let Some(job) = job {
        let bundle = export_bundle(
            project_id,
            ExportBaselineSelection::DraftCalculation { calculation_id: current.calculation_id },
            ExportFormat::Xlsx,
            b"rendered workbook",
        );
        fake.complete(job, Ok(ClientReply::Export(bundle)));
        session.drain_solidbom();
    }
    std::fs::set_permissions(&directory, writable).expect("a writable directory again");

    assert_eq!(std::fs::read(&path).expect("the previous export"), b"previous export", "the failed write kept the previous document");
    assert!(
        matches!(session.solidbom.export, Some(super::state::ExportOutcome::Failed(super::state::ExportFailure::Write { .. }))),
        "the failure is recorded: {:?}",
        session.solidbom.export
    );
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_failing_export_request_surfaces_the_typed_client_failure_and_retries_it() {
    let (mut session, fake) = fake_session();
    import_three_boxes(&mut session, &fake);
    let calculation_id = uuid::Uuid::from_u128(500);
    let scenarios = [uuid::Uuid::from_u128(10), uuid::Uuid::from_u128(11)];
    let path = export_path("failed.pdf");
    session
        .execute(
            "solidbom.export",
            &json!({
                "path": path.to_string_lossy(),
                "format": "pdf",
                "calculation_id": calculation_id.to_string(),
                "scenarios": [scenarios[0].to_string(), scenarios[1].to_string()],
            }),
        )
        .expect("the export starts");
    let (job, request) = take_request(&fake, |request| matches!(request, ClientRequest::Export(_)));
    let ClientRequest::Export(first) = request else { panic!("the request is an export") };
    assert_eq!(first.format, ExportFormat::Pdf);
    assert_eq!(first.scenarios, scenarios.to_vec());
    fake.complete(job, Err(ClientFailure::Network("offline".into())));
    session.drain_solidbom();

    assert!(!path.exists());
    assert!(
        matches!(session.solidbom.export, Some(super::state::ExportOutcome::Failed(super::state::ExportFailure::Request(ClientFailure::Network(_))))),
        "the typed failure is recorded: {:?}",
        session.solidbom.export
    );
    assert!(session.solidbom.error.as_deref().is_some_and(|error| error.contains("unreachable")), "{:?}", session.solidbom.error);
    assert!(session.solidbom.failed.as_ref().is_some_and(|failed| failed.retryable), "a network failure is retryable");
    session.solidbom_retry().expect("the retry resubmits the export");
    let (_, request) = take_request(&fake, |request| matches!(request, ClientRequest::Export(_)));
    let ClientRequest::Export(retried) = request else { panic!("the request is an export") };
    assert_eq!(retried, first, "a retry resends the exact export selection");
}

#[test]
fn export_parameters_are_validated_before_anything_is_submitted() {
    let (mut session, fake) = fake_session();
    import_three_boxes(&mut session, &fake);
    let path = export_path("invalid.xlsx");
    let accepted = uuid::Uuid::from_u128(900).to_string();
    let calculation = uuid::Uuid::from_u128(901).to_string();
    let scenarios = [uuid::Uuid::from_u128(11).to_string(), uuid::Uuid::from_u128(10).to_string()];
    for params in [
        json!({"path": "   ", "format": "xlsx"}),
        json!({"path": path.to_string_lossy(), "format": "step"}),
        json!({"path": path.to_string_lossy(), "accepted_baseline_id": accepted}),
        json!({"path": path.to_string_lossy(), "accepted_baseline_sequence": 2}),
        json!({"path": path.to_string_lossy(), "accepted_baseline_id": "not a uuid", "accepted_baseline_sequence": 2}),
        json!({"path": path.to_string_lossy(), "accepted_baseline_id": uuid::Uuid::nil().to_string(), "accepted_baseline_sequence": 2}),
        json!({"path": path.to_string_lossy(), "accepted_baseline_id": accepted, "accepted_baseline_sequence": 2, "calculation_id": calculation}),
        json!({"path": path.to_string_lossy(), "calculation_id": "not a uuid"}),
        json!({"path": path.to_string_lossy(), "scenarios": scenarios}),
        json!({"path": path.to_string_lossy(), "scenarios": ["not a uuid"]}),
    ] {
        let error = session.execute("solidbom.export", &params).expect_err("hostile parameters are refused");
        assert!(matches!(error, crate::EngineError::BadParams { ref cmd, .. } if cmd == "solidbom.export"), "{error}");
    }
    assert!(fake.take().is_empty(), "no invalid export reaches the host");
}

#[test]
fn an_export_needs_an_open_project_and_a_named_saved_result() {
    let path = export_path("no-project.xlsx");
    let error = Session::default().execute("solidbom.export", &json!({"path": path.to_string_lossy()})).expect_err("no project is open");
    assert!(matches!(error, crate::EngineError::BadParams { .. }), "{error}");
    assert!(error.to_string().contains("no SolidBOM project is open"), "{error}");

    // A desktop session carries only the v1 calculation id. Sending it to the SB-16 export route
    // would be a guaranteed 404, so the command refuses locally and submits nothing instead.
    let (mut session, fake) = fake_session();
    import_three_boxes(&mut session, &fake);
    let error = session.execute("solidbom.export", &json!({"path": path.to_string_lossy()})).expect_err("no saved result is named");
    assert!(matches!(error, crate::EngineError::BadParams { .. }), "{error}");
    assert!(error.to_string().contains("SB-16"), "the refusal names the snapshot kind: {error}");
    assert!(fake.take().is_empty(), "nothing is submitted without a selection");
}
