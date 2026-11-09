//! Exact tube and protection geometry from a reviewed source's B-rep.
//!
//! A constant-diameter tube is measured from its own exact B-rep record, never from a display
//! mesh: the outer and inner radius groups, the ordered centerline, every bend's radius and angle,
//! the developed centerline length (straight runs plus bend arcs), the straight runs between bend
//! tangents and the straight lengths at both ends. Wear and thermal protection are tubular
//! sleeves in the same supported subset; the reviewed category says what the part is, not what
//! geometry may stand in for it, and a part outside the subset is reported unavailable with a
//! named issue rather than guessed.
//!
//! # Supported subset
//!
//! One closed solid body per definition whose faces are, together:
//!
//! - exactly two planar annular end faces, each two concentric circles (outer radius `Ro`,
//!   inner radius `Ri`) in one plane square to the centerline, and
//! - one connected outer surface of cylindrical (straight) and toroidal (bend) faces at radius
//!   `Ro`, and the matching inner surface at radius `Ri`.
//!
//! The centerline is reconstructed from the exact circular boundary edges: a cylinder contributes
//! the segment between its end circles' centers, a torus contributes the arc between them around
//! the torus center at the major radius. Tangency between consecutive segments is checked, and the
//! inner and outer layers must reconstruct the same centerline. The two radius groups must agree
//! with the end faces' circles. The result is cross-checked against the body's exact volume: for
//! a tubular sweep, volume is annular area times centerline length, so a centerline that does not
//! reproduce the volume is refused rather than reported.
//!
//! Anything outside the subset - a surface (open) body, a mesh body, several bodies, a cone or
//! spline surface, a capped or non-square end, more or fewer than two radius groups, a wall split
//! into co-extensive faces along the tube, a branching or closed centerline, or a centerline that
//! disagrees with the volume - is `Unsupported`: no measurements are reported and a stable issue
//! code names why. The code separates unsupported geometry (`open_body`, `unsupported_surface`,
//! `unsupported_face_topology`, ...,) from a failed or self-inconsistent measurement
//! (`measure_failed`, `inconsistent_volume`), so a status-only consumer is not the whole story.
//! Missing values are `None`, never zero; an explicit zero is a measured zero (for example a bend
//! tangent that touches an end, or a straight run of length zero between two bends).
//!
//! # Identity and ordering
//!
//! Every result carries the exact analysis key of the source it describes, this extractor's
//! version and the definition it measures; an engineer's manual completion is a different value
//! with its own provenance, never part of this result. The definition's own body is measured, so
//! points and directions are in the definition frame the import gives that body, not in an
//! occurrence's world placement: a later placement never changes the measurement, and all
//! placements of a shared definition must resolve to that one definition body. The start end is
//! chosen deterministically: the end face whose circle center is lexicographically smallest (x,
//! then y, then z, within the extraction tolerance), so the same bytes always give the same start,
//! the same bend order and the same per-bend identities (0-based from the start end). Bend angles
//! are in degrees; every length is in millimetres.
//!
//! The extraction is pure and bounded: it reads the evaluated record and performs one exact
//! volume measure. It has no cancellation state and no window-thread state, so it can run inside
//! the existing background analysis worker; the exact SMLib measure itself is not interruptible,
//! so cancellation applies between jobs or requests, not in the middle of one measurement. A
//! complete result is only published after all of it has been measured.
//!
//! A consumer comparing a saved result with a fresh analysis must compare the whole analysis key
//! and require [`TubeExtraction::algorithm_is_current`]: the extraction version is deliberately
//! not part of the analysis key, because the manifest extraction that key names does not change
//! with this algorithm.

use std::collections::BTreeSet;
use std::f64::consts::{PI, TAU};

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use solidbom_contract::{AnalysisKey, AnalysisManifest, decimal_from_f64};
use solidcore_record::Vec3;
use solidcore_record::brep::{Brep, Frame, Loop, Surface};
use solidcraft_doc::ModelBody;
use solidcraft_geom::prelude::*;
use solidcraft_kernel::{Body, edge_circle};
use uuid::Uuid;

use crate::{EngineError, Result, Session};

/// Version of the extraction algorithm and of [`TubeExtraction`]. Bump it whenever a measurement
/// for the same B-rep could change, so a saved result is never silently attached to another
/// algorithm.
pub const EXTRACTION_VERSION: &str = "1";
/// The schema version written into [`TubeExtraction`] and checked on the way in.
pub const EXTRACTION_SCHEMA_VERSION: u32 = 1;

/// The most faces one definition may have before the extraction gives up (a reviewed STEP import
/// is hostile input; the record's own caps are larger than any tube needs).
const MAX_FACES: usize = 4096;
/// Two unit directions are parallel when their cosine is at least this: exact geometry agrees to
/// rounding, so a real kink is never accepted as a tangent join.
const PARALLEL_COS: f64 = 1.0 - 1e-9;
/// Two bend angles belong to the same centerline when they differ by at most this (radians).
const ANGLE_TOL: f64 = 1e-6;
/// How much the volume check may differ from the centerline length, relative to the length.
const VOLUME_TOL: f64 = 1e-6;

/// Which reviewed category asked for the extraction.
///
/// Recognition is not part of this extraction: the category is the engineer's reviewed
/// classification, and a part whose geometry is not in the supported subset is reported
/// unavailable whatever its category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionCategory {
    /// A constant-diameter tube: outer and inner diameter, developed centerline length, bend
    /// count, each bend's centerline radius, tangent-to-tangent straight lengths and straight ends.
    Tube,
    /// A wear-protection sleeve: developed centerline length including its curves.
    WearProtection,
    /// A thermal-protection sleeve: developed centerline length including its curves.
    ThermalProtection,
}

/// Whether the extraction produced every value, or the geometry is outside the supported subset.
/// An unsupported result carries no measurements and at least one issue; an incomplete result is
/// never presented as a successful extraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionStatus {
    Complete,
    Unsupported,
}

/// Which end of the measured tube a value describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TubeEnd {
    /// The start end: the end chosen as the deterministic start of the centerline.
    Start,
    /// The other end.
    End,
}

/// One stable diagnostic of an unavailable extraction. `code` is a stable name, never display
/// text; `message` explains the concrete cause for a person reading the record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractionIssue {
    pub code: String,
    pub message: String,
}

impl ExtractionIssue {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self { code: code.to_string(), message: message.into() }
    }
}

/// One bend of the centerline, in start-to-end order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TubeBend {
    /// 0-based position from the start end; the per-bend identity of this definition.
    pub index: u32,
    /// The centerline radius of this bend.
    pub centerline_radius_mm: Decimal,
    /// The angle the centerline turns through in this bend.
    pub angle_deg: Decimal,
    /// The developed arc length of this bend (`radius * angle`).
    pub developed_length_mm: Decimal,
    /// The bend's tangent point nearest the start end: the centerline point where this bend
    /// begins.
    pub start_mm: [f64; 3],
    /// The bend's tangent point nearest the tube's end.
    pub end_mm: [f64; 3],
}

/// The exact geometry of one tube end: where a connector associates and what it must fit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TubeEndEvidence {
    pub end: TubeEnd,
    /// The centerline point at the end (the end circle's center).
    pub center_mm: [f64; 3],
    /// The centerline direction pointing out of the tube through this end (the end face's
    /// outward normal); a unit vector.
    pub outward_axis: [f64; 3],
    /// The outer radius of the end face (`OD / 2`).
    pub outer_radius_mm: Option<Decimal>,
    /// The inner radius of the end face (`ID / 2`).
    pub inner_radius_mm: Option<Decimal>,
}

/// The exact tube or protection geometry of one reviewed definition.
///
/// Every measurement describes the source and extractor named by [`TubeExtraction::analysis_key`]
/// and [`TubeExtraction::extraction_version`]; a missing value is `None` and an explicit zero is
/// `Some(Decimal::ZERO)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TubeExtraction {
    pub schema_version: u32,
    pub analysis_key: AnalysisKey,
    pub extraction_version: String,
    /// The definition this result measures.
    pub definition_id: Uuid,
    /// The reviewed category that asked for it.
    pub category: ExtractionCategory,
    pub status: ExtractionStatus,
    /// Outer diameter (`2 * Ro`); `None` when unavailable.
    pub outer_diameter_mm: Option<Decimal>,
    /// Inner diameter (`2 * Ri`); `None` when unavailable.
    pub inner_diameter_mm: Option<Decimal>,
    /// Developed centerline length: every straight run plus every bend arc.
    pub developed_centerline_length_mm: Option<Decimal>,
    /// The exact number of bends (zero for a straight tube); `None` when unavailable.
    pub bend_count: Option<u32>,
    /// Every bend, in start-to-end order.
    pub bends: Vec<TubeBend>,
    /// Straight lengths between consecutive bends, tangent to tangent, in start-to-end order:
    /// exactly `bend_count - 1` entries when two or more bends exist (a zero where two bends meet
    /// at a shared tangent point), empty otherwise.
    pub tangent_straight_lengths_mm: Vec<Decimal>,
    /// The straight length at the start end and at the end end, in that order: two entries.
    /// For a straight tube both entries are the whole length, because either end offers the whole
    /// straight run; for a bent tube the length before the first bend and after the last bend
    /// (zero when the tube begins or ends with a bend).
    pub straight_end_lengths_mm: Vec<Decimal>,
    /// The two ends, start first.
    pub ends: Vec<TubeEndEvidence>,
    /// Why the extraction is unsupported; empty when complete. Stable codes, never display text.
    pub issues: Vec<ExtractionIssue>,
}

impl TubeExtraction {
    /// Check the versioned result's invariants: a complete result carries every value the
    /// categories need and its parts add up; an unsupported result carries no measurements and at
    /// least one issue.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != EXTRACTION_SCHEMA_VERSION {
            return Err(invalid(format!("the extraction schema version is {}, this build reads {EXTRACTION_SCHEMA_VERSION}", self.schema_version)));
        }
        self.analysis_key.validate().map_err(|error| EngineError::Other(error.to_string()))?;
        if self.extraction_version.trim().is_empty() {
            return Err(invalid("an extraction result needs its algorithm version"));
        }
        match self.status {
            ExtractionStatus::Unsupported => {
                if self.issues.is_empty() {
                    return Err(invalid("an unsupported extraction needs at least one issue"));
                }
                if self.outer_diameter_mm.is_some()
                    || self.inner_diameter_mm.is_some()
                    || self.developed_centerline_length_mm.is_some()
                    || self.bend_count.is_some()
                    || !self.bends.is_empty()
                    || !self.tangent_straight_lengths_mm.is_empty()
                    || !self.straight_end_lengths_mm.is_empty()
                    || !self.ends.is_empty()
                {
                    return Err(invalid("an unsupported extraction must not report measurements"));
                }
                return Ok(());
            }
            ExtractionStatus::Complete => {}
        }
        let outer = self.outer_diameter_mm.ok_or_else(|| invalid("a complete extraction needs the outer diameter"))?;
        let inner = self.inner_diameter_mm.ok_or_else(|| invalid("a complete extraction needs the inner diameter"))?;
        let developed = self.developed_centerline_length_mm.ok_or_else(|| invalid("a complete extraction needs the developed length"))?;
        let count = self.bend_count.ok_or_else(|| invalid("a complete extraction needs the bend count"))?;
        if !(outer > Decimal::ZERO && inner > Decimal::ZERO && outer > inner) {
            return Err(invalid("the outer diameter must be positive and larger than the inner diameter"));
        }
        if developed < Decimal::ZERO {
            return Err(invalid("the developed length cannot be negative"));
        }
        if self.bends.len() != usize::try_from(count).map_err(|_| invalid("the bend count does not fit this platform"))? {
            return Err(invalid("the bend count and the listed bends disagree"));
        }
        if self.tangent_straight_lengths_mm.len() != self.bends.len().saturating_sub(1) {
            return Err(invalid("there must be one tangent-to-tangent straight length between consecutive bends"));
        }
        if self.straight_end_lengths_mm.len() != 2 {
            return Err(invalid("a complete extraction needs one straight length per end"));
        }
        if self.ends.len() != 2 {
            return Err(invalid("a complete extraction needs both ends"));
        }
        if self.ends.first().map(|end| end.end) != Some(TubeEnd::Start) || self.ends.get(1).map(|end| end.end) != Some(TubeEnd::End) {
            return Err(invalid("the ends must be listed start first"));
        }
        for value in self.tangent_straight_lengths_mm.iter().chain(self.straight_end_lengths_mm.iter()) {
            if value.is_sign_negative() {
                return Err(invalid("a straight length cannot be negative"));
            }
        }
        let mut bend_total = 0.0f64;
        for (position, bend) in self.bends.iter().enumerate() {
            if usize::try_from(bend.index).ok() != Some(position) {
                return Err(invalid("the bends must be indexed from zero in start-to-end order"));
            }
            if bend.centerline_radius_mm <= Decimal::ZERO || bend.angle_deg <= Decimal::ZERO || bend.developed_length_mm <= Decimal::ZERO {
                return Err(invalid("a bend needs a positive radius, angle and developed length"));
            }
            if bend.angle_deg > Decimal::from(360) {
                return Err(invalid("a single bend cannot turn more than one whole turn"));
            }
            if !finite3(bend.start_mm) || !finite3(bend.end_mm) {
                return Err(invalid("a bend's tangent point must be finite"));
            }
            bend_total += decimal_to_f64(bend.developed_length_mm).ok_or_else(|| invalid("a bend's developed length does not fit a number"))?;
        }
        let straight_total: f64 = self
            .straight_end_lengths_mm
            .iter()
            .chain(self.tangent_straight_lengths_mm.iter())
            .try_fold(0.0f64, |sum, value| decimal_to_f64(*value).map(|value| sum + value))
            .ok_or_else(|| invalid("a straight length does not fit a number"))?;
        let developed_number = decimal_to_f64(developed).ok_or_else(|| invalid("the developed length does not fit a number"))?;
        let parts = if self.bends.is_empty() {
            // A straight tube: both ends report the whole straight run.
            let length = self.straight_end_lengths_mm.first().ok_or_else(|| invalid("a straight tube needs its length"))?;
            decimal_to_f64(*length).ok_or_else(|| invalid("a straight tube's length does not fit a number"))?
        } else {
            bend_total + straight_total
        };
        if relative_difference(parts, developed_number) > VOLUME_TOL {
            return Err(invalid("the developed length does not add up from its parts"));
        }
        for end in &self.ends {
            if !finite3(end.center_mm) || !finite3(end.outward_axis) {
                return Err(invalid("an end's center and direction must be finite"));
            }
            let axis = Vec3::from(end.outward_axis);
            if (axis.len() - 1.0).abs() > 1e-6 {
                return Err(invalid("an end's outward axis must be a unit vector"));
            }
            let outer_radius = end.outer_radius_mm.ok_or_else(|| invalid("an end needs its outer radius"))?;
            let inner_radius = end.inner_radius_mm.ok_or_else(|| invalid("an end needs its inner radius"))?;
            let doubled = |radius: Decimal| radius.checked_mul(Decimal::from(2));
            if !doubled(outer_radius).is_some_and(|diameter| same_measure(diameter, outer))
                || !doubled(inner_radius).is_some_and(|diameter| same_measure(diameter, inner))
            {
                return Err(invalid("an end's radii must be half the extracted diameters"));
            }
        }
        // The parts must also close geometrically: each bend's chord matches its radius and turn,
        // the start/end and tangent-to-tangent straight lengths join the end centers and bend
        // tangent points, and a straight tube's ends span its length. A tampered or inconsistent
        // result is refused, not just an arithmetically inconsistent total.
        let start_point = Vec3::from(self.ends[0].center_mm);
        let end_point = Vec3::from(self.ends[1].center_mm);
        let start_end_value = *self.straight_end_lengths_mm.first().ok_or_else(|| invalid("a complete extraction needs a start straight end"))?;
        let finish_end_value = *self.straight_end_lengths_mm.get(1).ok_or_else(|| invalid("a complete extraction needs an end straight end"))?;
        let start_end = decimal_to_f64(start_end_value).ok_or_else(|| invalid("a straight end does not fit a number"))?;
        let finish_end = decimal_to_f64(finish_end_value).ok_or_else(|| invalid("a straight end does not fit a number"))?;
        if self.bends.is_empty() {
            if !same_measure(start_end_value, finish_end_value) {
                return Err(invalid("a straight tube reports its whole length at both ends"));
            }
            check_distance(start_point, end_point, developed_number, "a straight tube's ends must span its developed length")?;
        } else {
            let first = self.bends.first().ok_or_else(|| invalid("a bend is missing"))?;
            let last = self.bends.last().ok_or_else(|| invalid("a bend is missing"))?;
            check_distance(start_point, Vec3::from(first.start_mm), start_end, "the first bend must start at the start straight end")?;
            check_distance(Vec3::from(last.end_mm), end_point, finish_end, "the last bend must end at the end straight end")?;
            for (position, pair) in self.bends.windows(2).enumerate() {
                let gap = self.tangent_straight_lengths_mm.get(position).ok_or_else(|| invalid("a tangent straight length is missing"))?;
                let gap = decimal_to_f64(*gap).ok_or_else(|| invalid("a tangent straight length does not fit a number"))?;
                check_distance(
                    Vec3::from(pair[0].end_mm),
                    Vec3::from(pair[1].start_mm),
                    gap,
                    "consecutive bends must be joined by their tangent straight length",
                )?;
            }
            for (position, bend) in self.bends.iter().enumerate() {
                let radius = decimal_to_f64(bend.centerline_radius_mm).ok_or_else(|| invalid("a bend radius does not fit a number"))?;
                let angle = decimal_to_f64(bend.angle_deg).ok_or_else(|| invalid("a bend angle does not fit a number"))?;
                let chord = 2.0 * radius * (angle.to_radians() / 2.0).sin();
                check_distance(
                    Vec3::from(bend.start_mm),
                    Vec3::from(bend.end_mm),
                    chord,
                    &format!("bend {position}'s chord must match its radius and angle"),
                )?;
            }
        }
        Ok(())
    }

    /// Whether this result came from the extraction algorithm and schema this build implements.
    /// A consumer comparing a saved result with a fresh analysis must require this in addition to
    /// the full analysis key; see the module documentation for why the version is separate.
    pub fn algorithm_is_current(&self) -> bool {
        self.schema_version == EXTRACTION_SCHEMA_VERSION && self.extraction_version == EXTRACTION_VERSION
    }
}

/// A versioned extraction request: the exact source and algorithm (`analysis_key`), the reviewed
/// definition and the reviewed category. Nothing here is guessed from a display name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtractionInput {
    pub schema_version: u32,
    pub analysis_key: AnalysisKey,
    pub definition_id: Uuid,
    pub category: ExtractionCategory,
}

impl ExtractionInput {
    /// A request at the current schema version.
    pub fn new(analysis_key: AnalysisKey, definition_id: Uuid, category: ExtractionCategory) -> Self {
        Self { schema_version: EXTRACTION_SCHEMA_VERSION, analysis_key, definition_id, category }
    }

    /// The version and source identity are the request's whole point: refuse a newer schema and a
    /// malformed key rather than measure something else.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != EXTRACTION_SCHEMA_VERSION {
            return Err(invalid(format!(
                "the extraction request schema version is {}, this build reads {EXTRACTION_SCHEMA_VERSION}",
                self.schema_version
            )));
        }
        self.analysis_key.validate().map_err(|error| EngineError::Other(error.to_string()))
    }
}

/// Measure one reviewed definition from the exact B-rep of its own bodies.
///
/// The returned value is never a guess: a body outside the supported subset is [`ExtractionStatus::Unsupported`]
/// with a named issue and no measurements. The function is pure and performs one exact volume
/// measure; it can run on a background analysis worker.
///
/// `key` must describe the source and extraction the bodies came from: it is copied into the
/// result as the measurement identity. [`extract_definition`] validates the request and manifest
/// for a reviewed session; a caller measuring bodies directly must pass an already-validated key
/// (and is responsible for the same source scoping).
pub fn extract_tube(key: &AnalysisKey, definition_id: Uuid, category: ExtractionCategory, bodies: &[&ModelBody]) -> TubeExtraction {
    match measure(bodies) {
        Ok(measured) => finish(key, definition_id, category, &measured),
        Err(failure) => unsupported(key, definition_id, category, failure.issues),
    }
}

/// Measure the definition an analysis names, resolving it through the analysis manifest and the
/// session's reviewed source.
///
/// The manifest must describe the source the session reviews: a stale manifest is refused, and a
/// malformed one is validated, instead of measuring another source or another algorithm's
/// analysis. The definition must be part of the analysis and placed by it; every placement must
/// resolve to the one definition body, which is measured in its own definition frame (a placement
/// never changes the measurement).
pub fn extract_definition(session: &Session, manifest: &AnalysisManifest, input: &ExtractionInput) -> Result<TubeExtraction> {
    input.validate()?;
    manifest.validate().map_err(|error| EngineError::Other(error.to_string()))?;
    if manifest.key != input.analysis_key {
        return Err(invalid("the extraction request was not made for this analysis"));
    }
    if let Some(review) = &session.review
        && manifest.key != *review.analysis()
    {
        return Err(invalid("the analysis does not describe the reviewed source; review the imported source again"));
    }
    let definition = manifest
        .definitions
        .iter()
        .find(|definition| definition.definition_id == input.definition_id)
        .ok_or_else(|| invalid("the definition is not part of this analysis"))?;
    // A definition is one shared body placed many times; measure that body once, in its own
    // definition frame. A manifest that points the same definition at two bodies is refused
    // instead of silently measuring whichever placement happens to come first.
    let map = super::extract::RuntimeOccurrenceMap::new(session)?;
    let mut component = None;
    for occurrence in manifest.occurrences.iter().filter(|occurrence| occurrence.definition_id == definition.definition_id) {
        let runtime_occurrence = map.runtime_occurrence(&occurrence.source_path)?;
        let placed_component = session
            .doc
            .occurrences
            .iter()
            .find(|placed| placed.id == runtime_occurrence)
            .map(|placed| placed.component)
            .ok_or_else(|| invalid("an occurrence is not part of this import"))?;
        match component {
            Some(expected) if expected != placed_component => {
                return Err(invalid("the definition's placements do not share one definition body"));
            }
            _ => component = Some(placed_component),
        }
    }
    let component = component.ok_or_else(|| invalid("the definition is not placed in this analysis"))?;
    let state = session.model.state();
    let owners = session.doc.model_body_components(&state);
    if owners.len() != state.bodies.len() {
        return Err(invalid("the evaluated bodies and their components do not line up"));
    }
    let bodies: Vec<&ModelBody> = state.bodies.iter().zip(owners.iter()).filter_map(|(body, owner)| (*owner == component).then_some(body)).collect();
    Ok(extract_tube(&manifest.key, input.definition_id, input.category, &bodies))
}

/// An unsupported extraction: no measurements at all, and the reasons why.
fn unsupported(key: &AnalysisKey, definition_id: Uuid, category: ExtractionCategory, issues: Vec<ExtractionIssue>) -> TubeExtraction {
    TubeExtraction {
        schema_version: EXTRACTION_SCHEMA_VERSION,
        analysis_key: key.clone(),
        extraction_version: EXTRACTION_VERSION.to_string(),
        definition_id,
        category,
        status: ExtractionStatus::Unsupported,
        outer_diameter_mm: None,
        inner_diameter_mm: None,
        developed_centerline_length_mm: None,
        bend_count: None,
        bends: Vec::new(),
        tangent_straight_lengths_mm: Vec::new(),
        straight_end_lengths_mm: Vec::new(),
        ends: Vec::new(),
        issues,
    }
}

/// A measured tube in geometry numbers: converted to the contract's decimals once, in [`finish`].
struct MeasuredTube {
    outer_radius: f64,
    inner_radius: f64,
    runs: Vec<Run>,
    ends: [EndFace; 2],
}

impl MeasuredTube {
    fn developed_length(&self) -> f64 {
        self.runs.iter().map(Run::length).sum()
    }

    fn bend_runs(&self) -> impl Iterator<Item = &Run> {
        self.runs.iter().filter(|run| matches!(run, Run::Bend { .. }))
    }
}

/// One run of the ordered centerline: a straight stretch, or one bend (a torus patch and any
/// tangent-continuous pieces of the same geometric bend merged into it).
enum Run {
    Straight {
        length: f64,
    },
    Bend {
        center: Vec3,
        radius: f64,
        /// The turn angle, positive; the direction of turn is implicit in the traversal order.
        angle: f64,
        start: Vec3,
        end: Vec3,
    },
}

impl Run {
    fn length(&self) -> f64 {
        match self {
            Run::Straight { length } => *length,
            Run::Bend { radius, angle, .. } => radius * angle,
        }
    }
}

/// A face of the tube's surface, classified from its exact surface and boundary circles.
struct CurvedFace {
    /// The tube-wall radius this face belongs to: the cylinder radius or the torus tube radius.
    layer_radius: f64,
    kind: CurvedKind,
}

enum CurvedKind {
    Straight {
        a: Vec3,
        b: Vec3,
    },
    Bend {
        center: Vec3,
        /// The torus axis: the centerline arc lies in the plane through `center` square to it.
        axis: Vec3,
        /// The centerline (major) radius.
        radius: f64,
        a: Vec3,
        b: Vec3,
    },
}

impl CurvedFace {
    fn points(&self) -> (Vec3, Vec3) {
        match &self.kind {
            CurvedKind::Straight { a, b } | CurvedKind::Bend { a, b, .. } => (*a, *b),
        }
    }

    fn other_point(&self, side: usize) -> Vec3 {
        let (a, b) = self.points();
        if side == 0 { b } else { a }
    }
}

/// One planar annular end face.
struct EndFace {
    center: Vec3,
    outward: Vec3,
    outer_radius: f64,
    inner_radius: f64,
}

/// A failure of the supported-subset check, carrying the issues a reviewer reads.
#[derive(Debug)]
struct Failure {
    issues: Vec<ExtractionIssue>,
}

type Outcome<T> = std::result::Result<T, Failure>;

fn fail(code: &'static str, message: impl Into<String>) -> Failure {
    Failure { issues: vec![ExtractionIssue::new(code, message)] }
}

fn invalid(message: impl Into<String>) -> EngineError {
    EngineError::Other(message.into())
}

/// The geometric tolerance for "the same point/radius" decisions on this body: a millionth of its
/// size, so it scales with the part and never reaches a real wall thickness or bend radius.
fn tolerance(body: &Body) -> f64 {
    (body.size() * 1e-6).max(1e-9)
}

fn frame_axis(frame: &Frame) -> Option<Vec3> {
    Vec3::from(frame.x_axis).cross(Vec3::from(frame.y_axis)).normalized()
}

fn frame_origin(frame: &Frame) -> Vec3 {
    Vec3::from(frame.origin)
}

fn finite3(value: [f64; 3]) -> bool {
    value.iter().all(|coordinate| coordinate.is_finite())
}

fn finite_vec(value: Vec3) -> bool {
    value.x.is_finite() && value.y.is_finite() && value.z.is_finite()
}

/// A decimal as a finite `f64`, through the exact shortest text it was written from; `None` when
/// it does not fit a double.
fn decimal_to_f64(value: Decimal) -> Option<f64> {
    value.to_string().parse::<f64>().ok().filter(|number| number.is_finite())
}

/// Two amounts are the same measurement when they differ only by decimal round-off from different
/// geometry sources (relative 1e-9, far below the 0.001 acceptance tolerance).
fn same_measure(left: Decimal, right: Decimal) -> bool {
    let scale = left.abs().max(right.abs());
    let (Some(difference), Some(tolerance)) = (left.checked_sub(right), scale.checked_mul(Decimal::new(1, 9))) else {
        return false;
    };
    difference.abs() <= tolerance
}

fn relative_difference(left: f64, right: f64) -> f64 {
    let scale = left.abs().max(right.abs());
    if scale == 0.0 { 0.0 } else { (left - right).abs() / scale }
}

/// Two points must be `expected` apart, to representation accuracy: an inconsistent or tampered
/// result is refused rather than reported.
fn check_distance(from: Vec3, to: Vec3, expected: f64, message: &str) -> Result<()> {
    if !finite_vec(from) || !finite_vec(to) || !expected.is_finite() {
        return Err(invalid(message));
    }
    if (from.dist(to) - expected).abs() > 1e-6 * expected.abs().max(1.0) {
        return Err(invalid(message));
    }
    Ok(())
}

/// `a` is lexicographically smaller than `b`, comparing x, then y, then z, and treating values
/// within `tol` as equal. Returns `None` when the points are equal within `tol`.
fn lexicographic_order(a: Vec3, b: Vec3, tol: f64) -> Option<std::cmp::Ordering> {
    for (left, right) in [(a.x, b.x), (a.y, b.y), (a.z, b.z)] {
        if (left - right).abs() > tol {
            return left.partial_cmp(&right);
        }
    }
    None
}

/// The exact circles on one face's boundary, from its exact curves.
fn face_circles(body: &Body, record: &Brep, face: usize) -> Vec<Circle> {
    let Some(face) = record.faces.get(face) else { return Vec::new() };
    let mut seen = BTreeSet::new();
    let mut circles = Vec::new();
    for boundary in &face.loops {
        let Loop::Edges { edges } = boundary else { continue };
        for oriented in edges {
            let edge = oriented.edge as usize;
            if !seen.insert(edge) {
                continue;
            }
            if let Some(circle) = edge_circle(body, edge) {
                circles.push(Circle { center: circle.center, axis: Vec3::from(circle.axis), radius: circle.radius });
            }
        }
    }
    circles
}

struct Circle {
    center: Vec3,
    axis: Vec3,
    radius: f64,
}

/// Classify a planar face as an annular tube end, or refuse it.
fn end_face(body: &Body, record: &Brep, face: usize, tol: f64) -> Outcome<EndFace> {
    let plane = body.planar_face(face).ok_or_else(|| fail("unsupported_end_face", format!("face {face} is planar but has no usable plane")))?;
    let outward = plane.x.cross(plane.y).normalized().ok_or_else(|| fail("unsupported_end_face", format!("face {face} has no plane normal")))?;
    let circles = face_circles(body, record, face);
    if circles.len() != 2 {
        return Err(fail(
            "unsupported_end_face",
            format!("a tube end must be one annular face with two circular boundaries; face {face} has {}", circles.len()),
        ));
    }
    let first = &circles[0];
    let second = &circles[1];
    if first.center.dist(second.center) > tol {
        return Err(fail("unsupported_end_face", format!("face {face}'s two circles are not concentric")));
    }
    let (inner, outer) = if first.radius <= second.radius { (first, second) } else { (second, first) };
    if outer.radius - inner.radius <= tol {
        return Err(fail("unsupported_end_face", format!("face {face}'s two circles are not an annulus")));
    }
    if (first.center - plane.origin).dot(outward).abs() > tol {
        return Err(fail("unsupported_end_face", format!("face {face}'s circles do not lie in its plane")));
    }
    for circle in [inner, outer] {
        if !finite_vec(circle.center) || !finite_vec(circle.axis) || !circle.radius.is_finite() {
            return Err(fail("unsupported_end_face", format!("face {face} has a non-finite boundary")));
        }
        if circle.axis.dot(outward).abs() < PARALLEL_COS {
            return Err(fail("unsupported_end_face", format!("face {face} is not square to the tube")));
        }
    }
    Ok(EndFace { center: first.center, outward, outer_radius: outer.radius, inner_radius: inner.radius })
}

/// Classify one non-planar face as a straight or bend face of the tube wall, from its exact
/// analytic surface and the exact circles on its boundary.
fn curved_face(body: &Body, record: &Brep, face: usize, tol: f64) -> Outcome<CurvedFace> {
    let Some(record_face) = record.faces.get(face) else {
        return Err(fail("unsupported_surface", format!("face {face} is gone")));
    };
    let circles = face_circles(body, record, face);
    match &record_face.surface {
        Surface::Cylinder { frame, radius } => {
            let axis = frame_axis(frame).ok_or_else(|| fail("unsupported_surface", format!("face {face}'s cylinder has no axis")))?;
            let origin = frame_origin(frame);
            if !radius.is_finite() || *radius <= tol {
                return Err(fail("unsupported_surface", format!("face {face}'s cylinder radius is not usable")));
            }
            let ends: Vec<&Circle> =
                circles.iter().filter(|circle| (circle.radius - radius).abs() <= tol && circle.axis.dot(axis).abs() >= PARALLEL_COS).collect();
            if ends.len() != 2 {
                return Err(fail(
                    "unsupported_face_topology",
                    format!("a straight tube face must be bounded by two tube circles; face {face} has {}", ends.len()),
                ));
            }
            for circle in &ends {
                let radial = circle.center - origin;
                if (radial - axis * radial.dot(axis)).len() > tol {
                    return Err(fail("unsupported_face_topology", format!("face {face}'s end circle is off its cylinder axis")));
                }
            }
            Ok(CurvedFace { layer_radius: *radius, kind: CurvedKind::Straight { a: ends[0].center, b: ends[1].center } })
        }
        Surface::Torus { frame, major_radius, minor_radius, .. } => {
            let axis = frame_axis(frame).ok_or_else(|| fail("unsupported_surface", format!("face {face}'s torus has no axis")))?;
            let center = frame_origin(frame);
            if !major_radius.is_finite() || !minor_radius.is_finite() || *major_radius <= tol || *minor_radius <= tol {
                return Err(fail("unsupported_surface", format!("face {face}'s torus radii are not usable")));
            }
            let ends: Vec<&Circle> =
                circles.iter().filter(|circle| (circle.radius - minor_radius).abs() <= tol && circle.axis.dot(axis).abs() < 1e-6).collect();
            if ends.len() != 2 {
                return Err(fail(
                    "unsupported_face_topology",
                    format!("a bend face must be bounded by two tube circles; face {face} has {}", ends.len()),
                ));
            }
            for circle in &ends {
                let radial = circle.center - center;
                if radial.dot(axis).abs() > tol {
                    return Err(fail("unsupported_face_topology", format!("face {face}'s bend circle is off the bend plane")));
                }
                if (radial.len() - major_radius).abs() > tol {
                    return Err(fail("unsupported_face_topology", format!("face {face}'s bend circle is off the centerline radius")));
                }
            }
            Ok(CurvedFace {
                layer_radius: *minor_radius,
                kind: CurvedKind::Bend { center, axis, radius: *major_radius, a: ends[0].center, b: ends[1].center },
            })
        }
        _ => Err(fail("unsupported_surface", format!("face {face} is not a plane, cylinder or torus"))),
    }
}

/// Classify every face: exactly two annular end faces and the tube's curved faces.
fn classify_faces(body: &Body, record: &Brep, tol: f64) -> Outcome<(Vec<CurvedFace>, [EndFace; 2])> {
    if record.faces.len() > MAX_FACES {
        return Err(fail("too_many_faces", format!("the definition has {} faces", record.faces.len())));
    }
    let mut curved = Vec::new();
    let mut ends = Vec::new();
    for face in 0..record.faces.len() {
        match record.faces.get(face).map(|face| &face.surface) {
            Some(Surface::Plane { .. } | Surface::PlaneThrough { .. }) => ends.push(end_face(body, record, face, tol)?),
            Some(Surface::Cylinder { .. } | Surface::Torus { .. }) => curved.push(curved_face(body, record, face, tol)?),
            Some(_) => return Err(fail("unsupported_surface", format!("face {face} is not a plane, cylinder or torus"))),
            None => return Err(fail("unsupported_surface", format!("face {face} is gone"))),
        }
    }
    if ends.len() != 2 {
        return Err(fail("unsupported_end_faces", format!("a tube needs exactly two annular end faces; the definition has {}", ends.len())));
    }
    if curved.is_empty() {
        return Err(fail("unsupported_tube", "the definition has no tube wall"));
    }
    let ends: [EndFace; 2] = ends.try_into().map_err(|_| fail("unsupported_end_faces", "the end faces do not pair up"))?;
    Ok((curved, ends))
}

/// Order the two ends deterministically and refuse a closed tube, whose ends coincide.
fn order_ends(ends: [EndFace; 2], tol: f64) -> Outcome<[EndFace; 2]> {
    let [first, second] = ends;
    match lexicographic_order(first.center, second.center, tol) {
        None => Err(fail("closed_centerline", "the tube's two ends coincide; a closed centerline is not supported")),
        Some(std::cmp::Ordering::Less) => Ok([first, second]),
        Some(_) => Ok([second, first]),
    }
}

/// The two wall radii of the tube, outer and inner, from the curved faces. The callers also check
/// that the end faces' circles agree with them.
fn radius_layers(curved: &[CurvedFace], tol: f64) -> Outcome<(f64, f64)> {
    let mut radii: Vec<f64> = Vec::new();
    for face in curved {
        if !radii.iter().any(|radius| (radius - face.layer_radius).abs() <= tol) {
            radii.push(face.layer_radius);
        }
    }
    if radii.len() != 2 {
        return Err(fail(
            "unsupported_diameter",
            format!("a constant-diameter tube needs exactly two wall radii (outer and inner); the definition has {}", radii.len()),
        ));
    }
    radii.sort_by(f64::total_cmp);
    let inner = radii[0];
    let outer = radii[1];
    if !(inner > tol) || outer - inner <= tol {
        return Err(fail("unsupported_diameter", "the wall radii do not form a tube"));
    }
    Ok((outer, inner))
}

/// A vertex of the layer's centerline graph: a tangent point where consecutive faces meet.
struct Node {
    point: Vec3,
    ends: Vec<(usize, usize)>,
}

fn push_node(nodes: &mut Vec<Node>, point: Vec3, face: usize, side: usize, tol: f64) {
    if let Some(node) = nodes.iter_mut().find(|node| node.point.dist(point) <= tol) {
        node.ends.push((face, side));
    } else {
        nodes.push(Node { point, ends: vec![(face, side)] });
    }
}

fn node_near(nodes: &[Node], point: Vec3, tol: f64) -> Option<usize> {
    nodes.iter().position(|node| node.point.dist(point) <= tol)
}

fn unique_free_node(nodes: &[Node], point: Vec3, tol: f64, what: &str) -> Outcome<usize> {
    let matches: Vec<usize> = nodes.iter().enumerate().filter(|(_, node)| node.point.dist(point) <= tol).map(|(index, _)| index).collect();
    match matches.as_slice() {
        [one] => Ok(*one),
        _ => Err(fail("ambiguous_centerline", format!("the {what} end does not meet the tube wall exactly once"))),
    }
}

/// Reconstruct one radius layer's ordered centerline, starting from the start end, and check that
/// it is one open chain whose direction runs from the start end's outward normal to the end end's.
fn chain(curved: &[CurvedFace], layer_radius: f64, start: &EndFace, end: &EndFace, tol: f64) -> Outcome<Vec<Seg>> {
    let faces: Vec<&CurvedFace> = curved.iter().filter(|face| (face.layer_radius - layer_radius).abs() <= tol).collect();
    if faces.is_empty() {
        return Err(fail("unsupported_diameter", "one wall radius has no faces"));
    }
    let mut nodes = Vec::new();
    for (index, face) in faces.iter().enumerate() {
        let (a, b) = face.points();
        if a.dist(b) <= tol {
            return Err(fail("unsupported_face_topology", "a tube face has no extent"));
        }
        push_node(&mut nodes, a, index, 0, tol);
        push_node(&mut nodes, b, index, 1, tol);
    }
    let free: Vec<usize> = nodes.iter().enumerate().filter(|(_, node)| node.ends.len() == 1).map(|(index, _)| index).collect();
    if free.len() != 2 {
        return Err(fail("ambiguous_centerline", format!("one wall radius must be one open chain; it has {} free tangent points", free.len())));
    }
    let start_node = unique_free_node(&nodes, start.center, tol, "start")?;
    let end_node = unique_free_node(&nodes, end.center, tol, "end")?;
    if start_node == end_node {
        return Err(fail("closed_centerline", "the tube's centerline is closed"));
    }
    // The tube's material lies on the inner side of the start end: the traversal leaves the end
    // inward. At the far end it leaves through the end's outward normal.
    let mut tangent = -start.outward;
    let mut current = start_node;
    let mut used = vec![false; faces.len()];
    let mut ordered = Vec::with_capacity(faces.len());
    loop {
        let Some(node) = nodes.get(current) else {
            return Err(fail("ambiguous_centerline", "the tube's centerline lost a tangent point"));
        };
        let candidates: Vec<(usize, usize)> = node.ends.iter().copied().filter(|(face, _)| !used.get(*face).copied().unwrap_or(true)).collect();
        let [(face_index, side)] = candidates.as_slice() else {
            if candidates.len() > 1 && co_extensive(&faces, &candidates, node.point, tol) {
                return Err(fail(
                    "unsupported_split_face",
                    "the tube wall is split into co-extensive faces along the tube; extraction needs unsplit cylinder/torus faces",
                ));
            }
            return Err(fail(
                "ambiguous_centerline",
                format!("the tube's centerline must pass each tangent point once; this one meets {} faces", candidates.len()),
            ));
        };
        let Some(face) = faces.get(*face_index) else {
            return Err(fail("ambiguous_centerline", "the tube's centerline names a missing face"));
        };
        if let Some(used) = used.get_mut(*face_index) {
            *used = true;
        }
        let point = node.point;
        let other = face.other_point(*side);
        let (segment, next_tangent) = orient_segment(face, point, tangent, other, *face_index, tol)?;
        ordered.push(segment);
        tangent = next_tangent;
        current = node_near(&nodes, other, tol).ok_or_else(|| fail("ambiguous_centerline", "the tube's centerline lost its next face"))?;
        if used.iter().all(|used| *used) {
            break;
        }
        if current == end_node {
            return Err(fail("ambiguous_centerline", "the tube wall continues past the far end"));
        }
    }
    if current != end_node {
        return Err(fail("ambiguous_centerline", "the tube wall does not end at the far end face"));
    }
    if tangent.dot(end.outward) < PARALLEL_COS {
        return Err(fail("tangent_discontinuity", "the tube's centerline does not leave through the far end square to it"));
    }
    Ok(ordered)
}

enum Seg {
    Straight { length: f64 },
    Bend { center: Vec3, radius: f64, angle: f64, start: Vec3, end: Vec3 },
}

/// Traverse one face from `point` toward `other`, resolving a bend's turn direction from the
/// direction the centerline must continue in, and return the segment plus its outgoing tangent.
fn orient_segment(face: &CurvedFace, point: Vec3, tangent: Vec3, other: Vec3, index: usize, tol: f64) -> Outcome<(Seg, Vec3)> {
    match &face.kind {
        CurvedKind::Straight { .. } => {
            let length = point.dist(other);
            if !(length > tol) {
                return Err(fail("unsupported_face_topology", format!("straight tube face {index} has no length")));
            }
            let direction = (other - point)
                .normalized()
                .ok_or_else(|| fail("unsupported_face_topology", format!("straight tube face {index} has no direction")))?;
            if direction.dot(tangent) < PARALLEL_COS {
                return Err(fail("tangent_discontinuity", "a straight tube face does not continue the centerline direction"));
            }
            Ok((Seg::Straight { length }, direction))
        }
        CurvedKind::Bend { center, axis, radius, .. } => {
            let u_in =
                (point - *center).normalized().ok_or_else(|| fail("unsupported_face_topology", format!("bend face {index} has no centerline")))?;
            let u_out =
                (other - *center).normalized().ok_or_else(|| fail("unsupported_face_topology", format!("bend face {index} has no centerline")))?;
            let theta = axis.dot(u_in.cross(u_out)).atan2(u_in.dot(u_out));
            if !theta.is_finite() {
                return Err(fail("unsupported_face_topology", format!("bend face {index} has no usable turn angle")));
            }
            // The face's two boundary circles leave two possible arcs: the short way round or the
            // rest of the turn. The centerline's incoming direction says which one the face is.
            let candidates = [theta, if theta > 0.0 { theta - TAU } else { theta + TAU }];
            let mut chosen = None;
            for candidate in candidates {
                if candidate.abs() <= ANGLE_TOL || candidate.abs() > TAU + ANGLE_TOL {
                    continue;
                }
                let turn = axis.cross(u_in);
                let start_direction = if candidate > 0.0 { turn } else { -turn };
                if start_direction.dot(tangent) >= PARALLEL_COS {
                    chosen = Some(candidate);
                    break;
                }
            }
            let angle = chosen.ok_or_else(|| fail("tangent_discontinuity", "a bend does not continue the tube's centerline direction"))?;
            let turn = axis.cross(u_out);
            let end_direction = if angle > 0.0 { turn } else { -turn };
            Ok((Seg::Bend { center: *center, radius: *radius, angle: angle.abs(), start: point, end: other }, end_direction))
        }
    }
}

/// Whether every candidate face runs between the same two tangent points (the point and one
/// other): the wall is one face split into co-extensive pieces, not a branch.
fn co_extensive(faces: &[&CurvedFace], candidates: &[(usize, usize)], point: Vec3, tol: f64) -> bool {
    let Some((first, first_side)) = candidates.first() else { return false };
    let Some(first_face) = faces.get(*first) else { return false };
    let other = first_face.other_point(*first_side);
    if other.dist(point) <= tol {
        return false;
    }
    candidates.iter().all(|(face, side)| faces.get(*face).is_some_and(|face| face.other_point(*side).dist(other) <= tol))
}

/// Merge tangent-continuous pieces of the same straight run or geometric bend, so a split face
/// does not count as an extra straight or bend.
fn merge_runs(segments: Vec<Seg>, tol: f64) -> Outcome<Vec<Run>> {
    let mut runs: Vec<Run> = Vec::new();
    for segment in segments {
        match segment {
            Seg::Straight { length } => match runs.last_mut() {
                Some(Run::Straight { length: total }) => *total += length,
                _ => runs.push(Run::Straight { length }),
            },
            Seg::Bend { center, radius, angle, start, end } => {
                let merged = match runs.last_mut() {
                    Some(Run::Bend { center: previous_center, radius: previous_radius, angle: total, end: previous_end, .. })
                        if center.dist(*previous_center) <= tol && (radius - *previous_radius).abs() <= tol =>
                    {
                        *total += angle;
                        *previous_end = end;
                        true
                    }
                    _ => false,
                };
                if !merged {
                    runs.push(Run::Bend { center, radius, angle, start, end });
                }
            }
        }
    }
    for run in &runs {
        if let Run::Bend { angle, .. } = run
            && *angle > TAU + ANGLE_TOL
        {
            return Err(fail("over_large_bend", "a bend turns more than one whole turn"));
        }
    }
    Ok(runs)
}

/// The two wall layers must describe the same centerline; otherwise the surface is not a
/// constant-diameter tube even when each layer looks like one.
fn runs_match(outer: &[Run], inner: &[Run], tol: f64) -> bool {
    if outer.len() != inner.len() {
        return false;
    }
    outer.iter().zip(inner).all(|(outer, inner)| match (outer, inner) {
        (Run::Straight { length: a }, Run::Straight { length: b }) => (a - b).abs() <= tol.max(a.abs() * 1e-9),
        (Run::Bend { radius: outer_radius, angle: outer_angle, .. }, Run::Bend { radius: inner_radius, angle: inner_angle, .. }) => {
            (outer_radius - inner_radius).abs() <= tol && (outer_angle - inner_angle).abs() <= ANGLE_TOL
        }
        _ => false,
    })
}

/// The straight lengths between consecutive bends, tangent to tangent.
fn tangent_straight_lengths(runs: &[Run]) -> Vec<f64> {
    let mut lengths = Vec::new();
    let mut pending = 0.0;
    let mut bent = false;
    for run in runs {
        match run {
            Run::Straight { length } => pending += length,
            Run::Bend { .. } => {
                if bent {
                    lengths.push(pending);
                }
                pending = 0.0;
                bent = true;
            }
        }
    }
    lengths
}

/// The straight length at the start end and at the end end. A straight tube offers its whole
/// length at either end; a bent tube offers the run before its first bend and after its last one.
fn straight_end_lengths(runs: &[Run]) -> [f64; 2] {
    let total: f64 = runs.iter().map(|run| if let Run::Straight { length } = run { *length } else { 0.0 }).sum();
    let first_bend = runs.iter().position(|run| matches!(run, Run::Bend { .. }));
    let last_bend = runs.iter().rposition(|run| matches!(run, Run::Bend { .. }));
    let (Some(first_bend), Some(last_bend)) = (first_bend, last_bend) else {
        return [total, total];
    };
    let start = runs.iter().take(first_bend).map(|run| if let Run::Straight { length } = run { *length } else { 0.0 }).sum();
    let end = runs.iter().skip(last_bend + 1).map(|run| if let Run::Straight { length } = run { *length } else { 0.0 }).sum();
    [start, end]
}

/// Measure one definition's single body, checking the whole supported subset.
fn measure(bodies: &[&ModelBody]) -> Outcome<MeasuredTube> {
    match bodies {
        [] => Err(fail("missing_geometry", "the definition has no body to measure")),
        [single] => {
            if single.body.is_mesh() {
                return Err(fail("mesh_body", "a mesh cannot decide the tube's exact geometry"));
            }
            if single.is_surface() {
                return Err(fail("open_body", "the definition is an open surface with no tube volume to measure"));
            }
            measure_body(single)
        }
        _ => Err(fail("multiple_bodies", format!("the definition has {} bodies; a tube is one connected solid body", bodies.len()))),
    }
}

fn measure_body(model_body: &ModelBody) -> Outcome<MeasuredTube> {
    let body = &model_body.body;
    let record = body.record();
    let tol = tolerance(body);
    let (curved, ends) = classify_faces(body, record, tol)?;
    let (outer_radius, inner_radius) = radius_layers(&curved, tol)?;
    let ends = order_ends(ends, tol)?;
    let [start, end] = &ends;
    for end_face in ends.iter() {
        if (end_face.outer_radius - outer_radius).abs() > tol || (end_face.inner_radius - inner_radius).abs() > tol {
            return Err(fail("mismatched_diameter", "an end face's circles do not match the tube's wall radii"));
        }
    }
    let outer = chain(&curved, outer_radius, start, end, tol)?;
    let inner = chain(&curved, inner_radius, start, end, tol)?;
    let outer_runs = merge_runs(outer, tol)?;
    let inner_runs = merge_runs(inner, tol)?;
    if !runs_match(&outer_runs, &inner_runs, tol) {
        return Err(fail("mismatched_layers", "the outer and inner surfaces do not follow the same centerline"));
    }
    let measured = MeasuredTube { outer_radius, inner_radius, runs: outer_runs, ends };
    // The volume of a tubular sweep is its annular area times its developed length. A centerline
    // that does not reproduce the exact volume (a missed arc, a wrong turn, extra geometry) is
    // refused instead of reported.
    let volume = model_body.measure().map_err(|error| fail("measure_failed", error.to_string()))?.volume;
    let area = PI * (outer_radius * outer_radius - inner_radius * inner_radius);
    if !(area > 0.0 && area.is_finite() && volume.is_finite()) {
        return Err(fail("measure_failed", "the tube's annular area or volume is not usable"));
    }
    let from_volume = volume / area;
    if !(from_volume > 0.0 && from_volume.is_finite()) {
        return Err(fail("measure_failed", "the tube's volume does not describe a positive length"));
    }
    if relative_difference(measured.developed_length(), from_volume) > VOLUME_TOL {
        return Err(fail(
            "inconsistent_volume",
            format!("the centerline is {:.6} mm long but the exact volume implies {:.6} mm", measured.developed_length(), from_volume),
        ));
    }
    Ok(measured)
}

/// Convert the measured geometry into the versioned contract result, once.
fn finish(key: &AnalysisKey, definition_id: Uuid, category: ExtractionCategory, measured: &MeasuredTube) -> TubeExtraction {
    let mut issues = Vec::new();
    let outer_diameter = length(2.0 * measured.outer_radius, "outer_diameter_mm", &mut issues);
    let inner_diameter = length(2.0 * measured.inner_radius, "inner_diameter_mm", &mut issues);
    let developed = length(measured.developed_length(), "developed_centerline_length_mm", &mut issues);
    let mut bends = Vec::new();
    for (index, run) in measured.bend_runs().enumerate() {
        let Run::Bend { radius, angle, start, end, .. } = run else { continue };
        let Ok(index) = u32::try_from(index) else {
            issues.push(ExtractionIssue::new("too_many_bends", "the definition has more bends than the contract counts"));
            return unsupported(key, definition_id, category, issues);
        };
        let centerline_radius = length(*radius, "bend.centerline_radius_mm", &mut issues);
        let angle_degrees = length(angle.to_degrees(), "bend.angle_deg", &mut issues);
        let developed_length = length(radius * angle, "bend.developed_length_mm", &mut issues);
        let (Some(centerline_radius), Some(angle_degrees), Some(developed_length)) = (centerline_radius, angle_degrees, developed_length) else {
            return unsupported(key, definition_id, category, issues);
        };
        bends.push(TubeBend {
            index,
            centerline_radius_mm: centerline_radius,
            angle_deg: angle_degrees,
            developed_length_mm: developed_length,
            start_mm: (*start).into(),
            end_mm: (*end).into(),
        });
    }
    let Some(bend_count) = u32::try_from(bends.len()).ok() else {
        issues.push(ExtractionIssue::new("too_many_bends", "the definition has more bends than the contract counts"));
        return unsupported(key, definition_id, category, issues);
    };
    let tangent_straight_lengths_mm = tangent_straight_lengths(&measured.runs)
        .into_iter()
        .filter_map(|value| length(value, "tangent_straight_lengths_mm", &mut issues))
        .collect::<Vec<_>>();
    let straight_end_lengths_mm = straight_end_lengths(&measured.runs)
        .into_iter()
        .filter_map(|value| length(value, "straight_end_lengths_mm", &mut issues))
        .collect::<Vec<_>>();
    // The end evidence reports the constant wall radius (half the extracted diameter): the cap
    // circles and the wall surfaces are only equal within the extraction tolerance, and one
    // authoritative source keeps a complete result self-consistent.
    let end_outer_radius = outer_diameter.and_then(|diameter| diameter.checked_div(Decimal::from(2)));
    let end_inner_radius = inner_diameter.and_then(|diameter| diameter.checked_div(Decimal::from(2)));
    if end_outer_radius.is_none() || end_inner_radius.is_none() {
        issues.push(ExtractionIssue::new("invalid_measurement", "the extracted diameters could not be split into end radii"));
    }
    let mut ends = Vec::with_capacity(2);
    for (position, end_face) in measured.ends.iter().enumerate() {
        let end = if position == 0 { TubeEnd::Start } else { TubeEnd::End };
        ends.push(TubeEndEvidence {
            end,
            center_mm: end_face.center.into(),
            outward_axis: end_face.outward.into(),
            outer_radius_mm: end_outer_radius,
            inner_radius_mm: end_inner_radius,
        });
    }
    if !issues.is_empty()
        || outer_diameter.is_none()
        || inner_diameter.is_none()
        || developed.is_none()
        || tangent_straight_lengths_mm.len() != bends.len().saturating_sub(1)
        || straight_end_lengths_mm.len() != 2
        || ends.iter().any(|end| end.outer_radius_mm.is_none() || end.inner_radius_mm.is_none())
    {
        if issues.is_empty() {
            issues.push(ExtractionIssue::new("invalid_measurement", "a measured value could not be written as an exact decimal"));
        }
        return unsupported(key, definition_id, category, issues);
    }
    TubeExtraction {
        schema_version: EXTRACTION_SCHEMA_VERSION,
        analysis_key: key.clone(),
        extraction_version: EXTRACTION_VERSION.to_string(),
        definition_id,
        category,
        status: ExtractionStatus::Complete,
        outer_diameter_mm: outer_diameter,
        inner_diameter_mm: inner_diameter,
        developed_centerline_length_mm: developed,
        bend_count: Some(bend_count),
        bends,
        tangent_straight_lengths_mm,
        straight_end_lengths_mm,
        ends,
        issues: Vec::new(),
    }
}

/// Convert one exact geometry number to its decimal once, naming the field when it cannot be.
fn length(value: f64, field: &'static str, issues: &mut Vec<ExtractionIssue>) -> Option<Decimal> {
    match decimal_from_f64(value, field) {
        Ok(value) => Some(value),
        Err(error) => {
            issues.push(ExtractionIssue::new("invalid_measurement", format!("{field}: {error}")));
            None
        }
    }
}

#[cfg(test)]
#[path = "extraction_tests.rs"]
mod tests;
