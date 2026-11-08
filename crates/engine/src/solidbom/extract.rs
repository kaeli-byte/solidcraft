//! Extract a reviewed source's definitions, occurrences and exact volumes, and resolve picks.
//!
//! The manifest is scoped to one source hash and extractor version. Definition and occurrence ids
//! are deterministic UUIDv5 values over the source hash and the source-local path, so importing
//! the same bytes again restores the same identities. Volumes come from the exact B-rep measure
//! of each definition's own closed bodies; a definition with any open or unmeasurable body has no
//! volume at all, never zero and never a partial sum.

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use solidbom_contract::{
    AnalysisKey, AnalysisManifest, DefinitionRef, GeometryBinding, Issue, Measurement, MeasurementSource, OccurrenceRef, ReviewState, SCHEMA_VERSION,
    Unit, decimal_from_f64,
};
use solidcraft_doc::{Document, FeatureKind, ModelState, Occurrence, mat_mul};
use uuid::Uuid;

use crate::cmd::inspect::encloses_volume;
use crate::{EngineError, Result, Sel, Session};

/// Bump this whenever extraction changes; a saved measurement is then not reused silently.
/// Version 2: a definition with any open or otherwise unmeasurable body has no volume, so a mixed
/// closed/open component is unavailable until an engineer completes it manually.
pub const EXTRACTOR_VERSION: &str = "2";
/// The namespace of the deterministic definition and occurrence ids.
const NAMESPACE: Uuid = Uuid::from_u128(0x73_6f_6c_69_64_62_6f_6d_5f_61_6e_61_6c_79_73_69);
/// A definition's own bodies beyond this are left to review rather than guessed apart.
const FLAT_BODY_LIMIT: usize = 1;

/// The build whose extraction produced a manifest.
pub fn engine_build() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Whether a manifest was produced by exactly this source and extractor.
pub fn analysis_matches(manifest: &AnalysisManifest, key: &AnalysisKey) -> bool {
    manifest.key == *key
}

/// Build the manifest for an imported source. The session's model must be up to date.
pub fn extract(session: &Session, key: AnalysisKey) -> Result<AnalysisManifest> {
    key.validate().map_err(|error| EngineError::Other(error.to_string()))?;
    let doc = &session.doc;
    let state = session.model.state();
    let import_root = import_root(doc)?;
    let indices = occurrence_indices(doc);
    let owners = doc.model_body_components(&state);
    if owners.len() != state.bodies.len() {
        return Err(EngineError::Other("the evaluated bodies and their components do not line up".into()));
    }
    let mut bodies_by_component: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
    for (index, component) in owners.iter().enumerate() {
        bodies_by_component.entry(*component).or_default().push(index);
    }

    let mut parts: Vec<(Vec<u64>, u64)> = bodies_by_component
        .keys()
        .filter_map(|component| component_source_path(doc, *component, import_root, &indices).map(|path| (path, *component)))
        .collect();
    parts.sort_by_key(|(path, _)| path.clone());
    let has_children = parts.iter().any(|(path, component)| *component != import_root && !path.is_empty());
    let mut issues = Vec::new();

    let mut definitions = Vec::with_capacity(parts.len());
    let mut volumes = BTreeMap::new();
    for (path, component) in &parts {
        let definition_id = definition_id(&key, path);
        let body_indices = bodies_by_component.get(component).map(Vec::as_slice).unwrap_or(&[]);
        let (volume, body_issues) = definition_volume(&state, body_indices, &key, definition_id);
        issues.extend(body_issues);
        if *component == import_root && body_indices.len() > FLAT_BODY_LIMIT && !has_children {
            issues.push(
                Issue::new(
                    "unresolved_grouping",
                    "analysis.definitions",
                    "the file has no usable product hierarchy; review the bodies as components",
                )
                .for_definition(definition_id),
            );
        }
        let component_ref = doc.components.iter().find(|candidate| candidate.id == *component);
        definitions.push(DefinitionRef {
            definition_id,
            source_key: key.clone(),
            customer_part_number: component_ref.and_then(|component| component.metadata.part_number().map(str::to_string)),
            customer_revision: component_ref.and_then(|component| component.metadata.revision().map(str::to_string)),
            volume_mm3: volume,
        });
        volumes.insert(*component, definition_id);
    }

    let mut occurrences = Vec::new();
    let mut occurrence_ids: BTreeMap<u64, Uuid> = BTreeMap::new();
    let mut placements: Vec<(Vec<u64>, u64, &Occurrence)> = Vec::new();
    for occurrence in &doc.occurrences {
        if !volumes.contains_key(&occurrence.component) {
            continue;
        }
        let Some(path) = source_path_of(doc, occurrence.id, import_root, &indices) else {
            continue;
        };
        placements.push((path, occurrence.component, occurrence));
    }
    placements.sort_by_key(|(path, _, _)| path.clone());
    for (path, _, occurrence) in &placements {
        let id = occurrence_id(&key, path);
        occurrence_ids.insert(occurrence.id, id);
    }
    for (path, component, occurrence) in &placements {
        let definition_id = volumes.get(component).copied().ok_or_else(|| EngineError::Other("an occurrence has no definition".into()))?;
        let parent_occurrence_id = if occurrence.parent == import_root || occurrence.parent == 0 {
            None
        } else {
            doc.occurrence_of(occurrence.parent).and_then(|parent| occurrence_ids.get(&parent.id).copied())
        };
        let world = mat_mul(&doc.component_transform(occurrence.parent), &occurrence.transform);
        occurrences.push(OccurrenceRef {
            occurrence_id: occurrence_id(&key, path),
            definition_id,
            parent_occurrence_id,
            source_path: path.clone(),
            transform: world,
        });
    }

    let mut geometry_bindings = Vec::new();
    for (path, component, occurrence) in &placements {
        let definition_id = volumes.get(component).copied().ok_or_else(|| EngineError::Other("an occurrence has no definition".into()))?;
        let body_count = bodies_by_component.get(component).map(Vec::len).unwrap_or(0);
        for body in 0..body_count {
            geometry_bindings.push(GeometryBinding {
                source_path: path.clone(),
                source_body: u32::try_from(body).map_err(|_| EngineError::Other("too many bodies in one definition".into()))?,
                definition_id,
                occurrence_id: occurrence_ids.get(&occurrence.id).copied().ok_or_else(|| EngineError::Other("an occurrence has no id".into()))?,
            });
        }
    }

    let manifest = AnalysisManifest { schema_version: SCHEMA_VERSION, key, definitions, occurrences, geometry_bindings, issues };
    manifest.validate().map_err(|error| EngineError::Other(error.to_string()))?;
    Ok(manifest)
}

/// Resolve a tree or viewport selection to its reviewed occurrence. An ambiguous selection is an
/// error, never a guess. A manifest whose key does not describe the session's reviewed source is
/// refused: it would otherwise map the selection to another source or another extraction.
pub fn resolve_pick(session: &Session, manifest: &AnalysisManifest, selection: &Sel) -> Result<OccurrenceRef> {
    check_reviewed_analysis(session, manifest)?;
    let doc = &session.doc;
    let import_root = import_root(doc)?;
    let indices = occurrence_indices(doc);
    match selection {
        Sel::Occurrence { path } => {
            let last = path.last().copied().ok_or_else(|| EngineError::Other("the selection has no occurrence".into()))?;
            let source_path = source_path_of(doc, last, import_root, &indices)
                .ok_or_else(|| EngineError::Other("the selection is outside the reviewed source".into()))?;
            manifest
                .occurrences
                .iter()
                .find(|occurrence| occurrence.source_path == source_path)
                .cloned()
                .ok_or_else(|| EngineError::Other("the selection is not part of this analysis".into()))
        }
        Sel::Body { name } | Sel::Face { body: name, .. } | Sel::Edge { body: name, .. } => {
            let component =
                doc.body_components.get(name).copied().ok_or_else(|| EngineError::Other(format!("`{name}` is not a body of the reviewed source")))?;
            let source_path = component_source_path(doc, component, import_root, &indices)
                .ok_or_else(|| EngineError::Other("the body is outside the reviewed source".into()))?;
            let mut matches = manifest.occurrences.iter().filter(|occurrence| occurrence.source_path == source_path);
            let first = matches.next().ok_or_else(|| EngineError::Other("the body is not part of this analysis".into()))?.clone();
            if matches.next().is_some() {
                return Err(EngineError::Other("this body belongs to a part placed more than once; select an occurrence".into()));
            }
            let placements = manifest.occurrences.iter().filter(|occurrence| occurrence.definition_id == first.definition_id).count();
            if placements > 1 {
                return Err(EngineError::Other("this body belongs to a part placed more than once; select an occurrence".into()));
            }
            Ok(first)
        }
        _ => Err(EngineError::Other("select a component or one of its bodies".into())),
    }
}

fn import_root(doc: &Document) -> Result<u64> {
    doc.features
        .iter()
        .rev()
        .find_map(|feature| match &feature.kind {
            FeatureKind::Import { .. } => Some(feature.component),
            _ => None,
        })
        .ok_or_else(|| EngineError::Other("the session has no imported STEP source".into()))
}

/// A pick resolves only against the exact analysis the session reviews. A manifest with a different
/// source hash, engine build or extractor version is stale and must not be attached to a pick.
fn check_reviewed_analysis(session: &Session, manifest: &AnalysisManifest) -> Result<()> {
    if let Some(review) = &session.review
        && manifest.key != *review.analysis()
    {
        return Err(EngineError::Other("the analysis does not describe the reviewed source; review the imported source again".into()));
    }
    Ok(())
}

struct OccurrenceIndex<'a> {
    by_id: BTreeMap<u64, &'a Occurrence>,
    by_component: BTreeMap<u64, &'a Occurrence>,
    child_indices: BTreeMap<u64, u64>,
}

fn occurrence_indices(doc: &Document) -> OccurrenceIndex<'_> {
    let mut next: BTreeMap<u64, u64> = BTreeMap::new();
    let mut child_indices = BTreeMap::new();
    let mut by_id = BTreeMap::new();
    let mut by_component = BTreeMap::new();
    for occurrence in &doc.occurrences {
        let counter = next.entry(occurrence.parent).or_insert(0);
        child_indices.insert(occurrence.id, *counter);
        *counter += 1;
        by_id.insert(occurrence.id, occurrence);
        by_component.entry(occurrence.component).or_insert(occurrence);
    }
    OccurrenceIndex { by_id, by_component, child_indices }
}

/// The child-index path of an occurrence, relative to the import root. The import root itself is
/// the empty path.
fn source_path_of(_doc: &Document, occurrence_id: u64, import_root: u64, indices: &OccurrenceIndex<'_>) -> Option<Vec<u64>> {
    let mut path = Vec::new();
    let mut current = occurrence_id;
    for _ in 0..solidcraft_doc::MAX_PLACEMENT_DEPTH + 1 {
        let occurrence = indices.by_id.get(&current)?;
        if occurrence.parent == import_root {
            path.push(*indices.child_indices.get(&occurrence.id)?);
            path.reverse();
            return Some(path);
        }
        if occurrence.parent == 0 {
            // The import root's own placement (or a design outside the source).
            path.reverse();
            return Some(path);
        }
        path.push(*indices.child_indices.get(&occurrence.id)?);
        current = indices.by_component.get(&occurrence.parent)?.id;
    }
    None
}

fn component_source_path(doc: &Document, component: u64, import_root: u64, indices: &OccurrenceIndex<'_>) -> Option<Vec<u64>> {
    if component == import_root {
        return Some(Vec::new());
    }
    let occurrence = doc.occurrence_of(component)?;
    source_path_of(doc, occurrence.id, import_root, indices)
}

/// Resolve every source definition back to full session occurrence paths without inspecting a mesh.
/// The index shares extraction's source-path policy and bounds ancestry traversal on hostile input.
pub fn definition_occurrence_paths(session: &Session, manifest: &AnalysisManifest) -> Result<BTreeMap<Uuid, Vec<Vec<u64>>>> {
    let doc = &session.doc;
    let root = import_root(doc)?;
    let index = occurrence_indices(doc);
    let mut definitions = BTreeMap::new();
    for occurrence in &manifest.occurrences {
        if definitions.insert(occurrence.source_path.as_slice(), occurrence.definition_id).is_some() {
            return Err(EngineError::Other("the reviewed source maps one occurrence path more than once".into()));
        }
    }
    let mut paths: BTreeMap<Uuid, Vec<Vec<u64>>> = BTreeMap::new();
    for occurrence in &doc.occurrences {
        let Some(source_path) = source_path_of(doc, occurrence.id, root, &index) else { continue };
        let Some(definition) = definitions.get(source_path.as_slice()) else { continue };
        let mut path = Vec::new();
        let mut current = occurrence.id;
        let mut complete = false;
        for _ in 0..=solidcraft_doc::MAX_PLACEMENT_DEPTH {
            let Some(placement) = index.by_id.get(&current) else { break };
            path.push(current);
            if placement.parent == 0 {
                complete = true;
                break;
            }
            let Some(parent) = index.by_component.get(&placement.parent) else { break };
            current = parent.id;
        }
        if !complete {
            return Err(EngineError::Other("the reviewed occurrence has an invalid or too-deep session path".into()));
        }
        path.reverse();
        paths.entry(*definition).or_default().push(path);
    }
    Ok(paths)
}

/// Bidirectional map between a session's runtime occurrence ids and the stable source paths an
/// [`AnalysisManifest`] uses. Runtime CAD ids are generated per import and may change; source paths
/// are derived from the source tree and are the only identity a consumer may persist.
#[derive(Debug, Clone, Default)]
pub struct RuntimeOccurrenceMap {
    by_runtime: BTreeMap<u64, Vec<u64>>,
    by_source: BTreeMap<Vec<u64>, u64>,
}

impl RuntimeOccurrenceMap {
    /// Map every occurrence the imported source places, in both directions. A session without an
    /// imported source is an error, and so is a source the import reaches along one path twice.
    pub fn new(session: &Session) -> Result<Self> {
        let doc = &session.doc;
        let root = import_root(doc)?;
        let index = occurrence_indices(doc);
        let mut map = Self::default();
        for occurrence in &doc.occurrences {
            // A design may hold geometry outside the reviewed source; it has no source path here.
            if !doc.component_within(occurrence.component, root) {
                continue;
            }
            let Some(path) = source_path_of(doc, occurrence.id, root, &index) else { continue };
            if map.by_source.insert(path.clone(), occurrence.id).is_some() {
                return Err(EngineError::Other("the imported source places one occurrence path twice".into()));
            }
            map.by_runtime.insert(occurrence.id, path);
        }
        Ok(map)
    }

    /// The stable source path a runtime occurrence id stands for. An id this source does not place
    /// is an error, never a guessed path.
    pub fn source_path(&self, runtime_occurrence: u64) -> Result<Vec<u64>> {
        self.by_runtime
            .get(&runtime_occurrence)
            .cloned()
            .ok_or_else(|| EngineError::Other(format!("{runtime_occurrence} is not an occurrence of the reviewed source")))
    }

    /// The runtime occurrence id that currently places a stable source path. Runtime ids are
    /// replaceable, so a consumer maps its stored path again after every import.
    pub fn runtime_occurrence(&self, source_path: &[u64]) -> Result<u64> {
        self.by_source
            .get(source_path)
            .copied()
            .ok_or_else(|| EngineError::Other(format!("the source path {} is not in this import", path_key(source_path))))
    }
}

/// A definition-only tree pick: the shared definition and every occurrence that places it. It holds
/// no occurrence of its own, because a definition is not itself placed.
#[derive(Debug, Clone)]
pub struct DefinitionPick<'a> {
    pub definition: &'a DefinitionRef,
    pub occurrences: Vec<&'a OccurrenceRef>,
}

/// Resolve a definition-only tree pick to the shared definition it names, with the exact
/// occurrences that place it. This never invents an occurrence. An unknown definition is an error,
/// and so is a definition the analysis does not place.
pub fn resolve_definition_pick<'a>(manifest: &'a AnalysisManifest, definition_id: Uuid) -> Result<DefinitionPick<'a>> {
    let definition = manifest
        .definitions
        .iter()
        .find(|definition| definition.definition_id == definition_id)
        .ok_or_else(|| EngineError::Other("the selection is not a definition of this analysis".into()))?;
    let occurrences: Vec<&OccurrenceRef> = manifest.occurrences.iter().filter(|occurrence| occurrence.definition_id == definition_id).collect();
    if occurrences.is_empty() {
        return Err(EngineError::Other("the definition is not placed in this analysis".into()));
    }
    Ok(DefinitionPick { definition, occurrences })
}

fn definition_id(key: &AnalysisKey, path: &[u64]) -> Uuid {
    Uuid::new_v5(&NAMESPACE, format!("{}:def:{}", key.source_sha256, path_key(path)).as_bytes())
}

fn occurrence_id(key: &AnalysisKey, path: &[u64]) -> Uuid {
    Uuid::new_v5(&NAMESPACE, format!("{}:occ:{}", key.source_sha256, path_key(path)).as_bytes())
}

fn path_key(path: &[u64]) -> String {
    path.iter().map(u64::to_string).collect::<Vec<_>>().join(".")
}

/// The volume of a definition's own bodies. A definition is measurable only when every body is
/// present, closed and measured in full; one open or unmeasurable body leaves the whole definition
/// unavailable (`None` with named issues), because a partial sum is not the part's volume.
fn definition_volume(state: &ModelState, bodies: &[usize], key: &AnalysisKey, definition_id: Uuid) -> (Measurement, Vec<Issue>) {
    let mut issues = Vec::new();
    let mut total = Decimal::ZERO;
    let mut measurable = true;
    let mut closed = false;
    for index in bodies {
        let Some(body) = state.bodies.get(*index) else {
            measurable = false;
            issues.push(Issue::new("missing_body", "analysis.definitions.volume_mm3", "the evaluated body is gone").for_definition(definition_id));
            continue;
        };
        if !encloses_volume(&body.body) {
            measurable = false;
            issues.push(
                Issue::new("open_body", "analysis.definitions.volume_mm3", "an open surface has no enclosed volume").for_definition(definition_id),
            );
            continue;
        }
        match body.measure() {
            Ok(measure) => match decimal_from_f64(measure.volume, "volume") {
                Ok(volume) => match total.checked_add(volume) {
                    Some(sum) => {
                        total = sum;
                        closed = true;
                    }
                    None => {
                        measurable = false;
                        issues.push(
                            Issue::new("volume_overflow", "analysis.definitions.volume_mm3", "the summed volume is too large")
                                .for_definition(definition_id),
                        );
                    }
                },
                Err(error) => {
                    measurable = false;
                    issues.push(Issue::new("invalid_volume", "analysis.definitions.volume_mm3", error.to_string()).for_definition(definition_id))
                }
            },
            Err(error) => {
                measurable = false;
                issues.push(Issue::new("measure_failed", "analysis.definitions.volume_mm3", error.to_string()).for_definition(definition_id))
            }
        }
    }
    if (!measurable || !closed) && !bodies.is_empty() {
        issues.push(
            Issue::new("missing_volume", "analysis.definitions.volume_mm3", "no complete enclosed volume can be measured")
                .for_definition(definition_id),
        );
    }
    (
        Measurement {
            value: (measurable && closed).then_some(total),
            unit: Unit::MillimetreCubed,
            source: MeasurementSource::Extracted,
            review: ReviewState::Pending,
            reason: None,
            analysis_key: key.clone(),
        },
        issues,
    )
}
