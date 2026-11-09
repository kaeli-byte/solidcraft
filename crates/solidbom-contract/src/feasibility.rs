//! Feasibility rules and justified exceptions (package SB-14).
//!
//! Structured, sourced rule tables are evaluated against reviewed tube geometry. The evaluator is a
//! pure function of an immutable rule-table version and the exact reviewed measurements, so the
//! same answer is produced by the service and by any later client preview:
//!
//! - **Rules** come from administrator-published, append-only rule-table versions. Every rule keeps
//!   its source reference, a structured applicability and one typed limit. There is no formula
//!   language, no priority and no interpolation: an inapplicable rule never passes a check, an
//!   overlapping conflicting limit leaves the check unresolved for an Administrator, and a missing
//!   or unusable limit is never turned into a default.
//! - **Measurements** are the reviewed exact geometry of one tube component: the outer and inner
//!   diameter, every bend's centerline radius tested independently, the straight centerline lengths
//!   between consecutive bend tangents (tangent to tangent, never chord or nominal length), the
//!   straight length at each end, and the confirmed connector association of each end. A value that
//!   is not available is `None`; a recorded value is exact.
//! - **Findings** are computed, never stored as passes. Each failing, conflicting or unresolved
//!   finding names the rule version, the required limit and the supporting measurements, so a
//!   person can read why it did not pass.
//! - **Exceptions** are engineer decisions with a recorded justification. An exception changes the
//!   disposition of a non-passing finding, never the geometric result. It keeps the exact evidence
//!   it was accepted against and becomes visible as `review_required` when that supporting geometry
//!   or the current assessment changes; it is never silently dropped and never silently kept.
//!
//! Feasibility is independent of the selected manufacturing cost center: no value in this module
//! names a cost center or a rate.
//!
//! Wire rules follow version 1 of the contract: decimals serialize as strings, a missing value is
//! `null`, unknown JSON fields are tolerated, and a stored document names the schema version it was
//! written with. Lengths in this module are millimetres; angles elsewhere in the contract are
//! degrees.

use std::collections::BTreeSet;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::engineering::{EngineeringRecordDocument, ReviewedGeometry};
use crate::error::{ContractError, Result};
use crate::shared_data::Identity;
use crate::types::{AnalysisKey, check_text};

/// The feasibility contract version this build reads and writes. It extends the published version 2
/// HTTP API additively; it does not change a version 1 or version 2 meaning.
pub const FEASIBILITY_CONTRACT_VERSION: u32 = 1;

/// The schema version written into a stored feasibility record document.
pub const FEASIBILITY_DOCUMENT_SCHEMA_VERSION: u32 = 1;

/// Resource bounds for hostile input. A rule table or a review is not a dump of arbitrary size.
pub const MAX_RULES_PER_TABLE: usize = 4096;
pub const MAX_ASSESSED_COMPONENTS: usize = 10_000;
/// The most findings one recorded assessment may hold.
pub const MAX_FINDINGS_PER_ASSESSMENT: usize = 1_000_000;
/// The most rule-finding evaluations one assessment may perform, so a hostile request cannot ask the
/// service to build an unbounded number of findings with every rule applied to each.
pub const MAX_EVALUATION_WORK: usize = 4_000_000;
pub const MAX_TUBE_BENDS: usize = 10_000;
pub const MAX_ASSESSMENTS: usize = 1_000;
pub const MAX_EXCEPTIONS: usize = 10_000;
pub const MAX_RULE_NAME_CHARS: usize = 255;
pub const MAX_MATERIAL_NAME_CHARS: usize = 255;
pub const MAX_EXTRACTION_VERSION_CHARS: usize = 255;
pub const MAX_FEASIBILITY_SOURCE_REFERENCE_CHARS: usize = 1024;
pub const MAX_JUSTIFICATION_CHARS: usize = 4_000;
pub const MAX_FEASIBILITY_REQUEST_ID_CHARS: usize = 128;
/// The most rule-change facts a rules view returns.
pub const MAX_FEASIBILITY_CHANGES: usize = 100;

/// Which kind of tube the reviewed classification names. Corrugated measurement support is not part
/// of the SB-12 extraction scope; a corrugated rule may be retained as data but is only applicable
/// when the reviewed evidence actually names a corrugated tube.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TubeKind {
    Smooth,
    Corrugated,
}

/// Which end of a measured tube a value or a check describes, start first.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TubeEnd {
    Start,
    End,
}

/// An inclusive numeric range in millimetres. Both ends are required; an open-ended or missing
/// range is not a rule limit and is refused at publish time.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct NumericRangeMm {
    pub min_mm: Decimal,
    pub max_mm: Decimal,
}

impl NumericRangeMm {
    /// The range must be ordered and non-negative.
    pub fn validate(&self, field: &str) -> Result<()> {
        if self.min_mm.is_sign_negative() || self.max_mm.is_sign_negative() {
            return Err(ContractError::invalid(field, "a range cannot be negative"));
        }
        if self.min_mm > self.max_mm {
            return Err(ContractError::invalid(field, "the range minimum is above its maximum"));
        }
        Ok(())
    }

    /// Whether `value` is inside the inclusive range.
    pub fn contains(&self, value: Decimal) -> bool {
        value >= self.min_mm && value <= self.max_mm
    }
}

/// The reviewed attributes a rule may select on. Every attribute is explicit evidence: a rule whose
/// applicability needs an attribute the review does not prove cannot apply, so the check stays
/// unresolved instead of passing. Nominal sizes are engineering data; the extractor never rounds a
/// measured size into a nominal one.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeasibilityTubeAttributes {
    /// The reviewed tube kind. `None` means the review has not established one.
    pub tube_kind: Option<TubeKind>,
    /// The reviewed material name, exactly as the engineering source spells it. `None` means the
    /// review has not proved a material.
    pub material_name: Option<String>,
    /// Whether the reviewed specification requires a supporting spring inside the bend. `None`
    /// means the review has not established the condition either way.
    pub supporting_spring: Option<bool>,
    /// The reviewed nominal outer diameter, when a nominal size was associated.
    pub nominal_outer_diameter_mm: Option<Decimal>,
    /// The reviewed nominal wall thickness, when a nominal size was associated.
    pub nominal_wall_thickness_mm: Option<Decimal>,
    /// The reviewed nominal connection size: the inside diameter in the connection area a
    /// corrugated source keys its radius table by. It is never the generic outer diameter.
    pub nominal_connection_size_mm: Option<Decimal>,
}

impl FeasibilityTubeAttributes {
    /// Validate the optional evidence fields.
    pub fn validate(&self, field: &str) -> Result<()> {
        if let Some(name) = &self.material_name {
            check_text(name, &format!("{field}.material_name"))?;
            if name.chars().count() > MAX_MATERIAL_NAME_CHARS {
                return Err(ContractError::invalid(format!("{field}.material_name"), "the material name is too long"));
            }
        }
        check_optional_positive(self.nominal_outer_diameter_mm, &format!("{field}.nominal_outer_diameter_mm"))?;
        check_optional_positive(self.nominal_wall_thickness_mm, &format!("{field}.nominal_wall_thickness_mm"))?;
        check_optional_positive(self.nominal_connection_size_mm, &format!("{field}.nominal_connection_size_mm"))
    }
}

/// The four feasibility rules the MVP keeps. The kind is part of a finding's identity, so a check
/// never changes meaning silently.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum FeasibilityRuleKind {
    /// Compare each bend's centerline radius with the applicable minimum.
    MinimumBendRadius,
    /// Compare each straight centerline length between consecutive bend tangent points.
    BendSpacing,
    /// Compare the tube's outer and inner diameter with the connector's compatible range.
    TubeConnectorCompatibility,
    /// Compare each end's available straight length with the connector's required assembly length.
    EndAssemblyLength,
}

impl FeasibilityRuleKind {
    /// The stable wire/diagnostic name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MinimumBendRadius => "minimum_bend_radius",
            Self::BendSpacing => "bend_spacing",
            Self::TubeConnectorCompatibility => "tube_connector_compatibility",
            Self::EndAssemblyLength => "end_assembly_length",
        }
    }

    /// Whether a connector-specific applicability may be attached to this kind. A connector rule on
    /// a radius or spacing check is inconsistent engineering data and is refused at publish time
    /// rather than silently ignored.
    fn is_connector_check(self) -> bool {
        matches!(self, Self::TubeConnectorCompatibility | Self::EndAssemblyLength)
    }
}

/// The structured limit of one rule. The variant must match the rule kind.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuleLimit {
    /// A minimum radius in millimetres.
    MinimumRadiusMm { radius_mm: Decimal },
    /// A minimum radius expressed as a multiple of the measured outer diameter, as the owner's
    /// smooth-tube source does. This is one supported arithmetic form, not a formula language.
    MinimumRadiusMultipleOfOuterDiameter { multiple: Decimal },
    /// A minimum straight length in millimetres, used for bend spacing and end assembly length.
    MinimumLengthMm { length_mm: Decimal },
    /// The connector's compatible tube ranges; both ranges are required.
    CompatibleTubeRange { outer_diameter_mm: NumericRangeMm, inner_diameter_mm: NumericRangeMm },
}

impl RuleLimit {
    fn validate(&self, kind: FeasibilityRuleKind, field: &str) -> Result<()> {
        match (self, kind) {
            (Self::MinimumRadiusMm { radius_mm }, FeasibilityRuleKind::MinimumBendRadius) => {
                check_positive(*radius_mm, &format!("{field}.radius_mm"))
            }
            (Self::MinimumRadiusMultipleOfOuterDiameter { multiple }, FeasibilityRuleKind::MinimumBendRadius) => {
                check_positive(*multiple, &format!("{field}.multiple"))
            }
            (Self::MinimumLengthMm { length_mm }, FeasibilityRuleKind::BendSpacing | FeasibilityRuleKind::EndAssemblyLength) => {
                check_non_negative(*length_mm, &format!("{field}.length_mm"))
            }
            (Self::CompatibleTubeRange { outer_diameter_mm, inner_diameter_mm }, FeasibilityRuleKind::TubeConnectorCompatibility) => {
                outer_diameter_mm.validate(&format!("{field}.outer_diameter_mm"))?;
                inner_diameter_mm.validate(&format!("{field}.inner_diameter_mm"))
            }
            _ => Err(ContractError::invalid(field, format!("a `{}` rule needs its matching limit", kind.as_str()))),
        }
    }
}

/// The structured applicability of one rule. Every condition that is set must be proven by the
/// reviewed evidence; a rule with no condition applies to every check of its kind.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuleApplicability {
    /// Only this reviewed tube kind.
    pub tube_kind: Option<TubeKind>,
    /// Only this reviewed material name, matched exactly (no normalisation or case folding).
    pub material_name: Option<String>,
    /// Only when the reviewed specification records a supporting spring.
    pub supporting_spring_required: bool,
    /// Only when the reviewed nominal outer diameter is exactly this value.
    pub nominal_outer_diameter_mm: Option<Decimal>,
    /// Only when the reviewed nominal wall thickness is exactly this value.
    pub nominal_wall_thickness_mm: Option<Decimal>,
    /// Only when the reviewed nominal connection size is exactly this value. This is the corrugated
    /// source's connection-area inside diameter, never the generic outer diameter.
    pub nominal_connection_size_mm: Option<Decimal>,
    /// Only when the measured outer diameter is inside this inclusive range.
    pub measured_outer_diameter_mm: Option<NumericRangeMm>,
    /// Only when the measured wall thickness `(OD - ID) / 2` is inside this inclusive range.
    pub measured_wall_thickness_mm: Option<NumericRangeMm>,
    /// Only for an end associated with this catalog component.
    pub connector_component_id: Option<Uuid>,
}

impl RuleApplicability {
    fn validate(&self, kind: FeasibilityRuleKind, field: &str) -> Result<()> {
        if let Some(name) = &self.material_name {
            check_text(name, &format!("{field}.material_name"))?;
            if name.chars().count() > MAX_MATERIAL_NAME_CHARS {
                return Err(ContractError::invalid(format!("{field}.material_name"), "the material name is too long"));
            }
        }
        check_optional_positive(self.nominal_outer_diameter_mm, &format!("{field}.nominal_outer_diameter_mm"))?;
        check_optional_positive(self.nominal_wall_thickness_mm, &format!("{field}.nominal_wall_thickness_mm"))?;
        check_optional_positive(self.nominal_connection_size_mm, &format!("{field}.nominal_connection_size_mm"))?;
        if let Some(range) = &self.measured_outer_diameter_mm {
            range.validate(&format!("{field}.measured_outer_diameter_mm"))?;
        }
        if let Some(range) = &self.measured_wall_thickness_mm {
            range.validate(&format!("{field}.measured_wall_thickness_mm"))?;
        }
        if self.connector_component_id.is_some() && !kind.is_connector_check() {
            return Err(ContractError::invalid(
                format!("{field}.connector_component_id"),
                "a radius or spacing rule cannot be conditioned on a connector",
            ));
        }
        Ok(())
    }
}

/// One sourced feasibility rule. The source reference is required: an administrator-entered limit
/// without its engineering source would be an invented default.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeasibilityRule {
    pub rule_id: Uuid,
    pub kind: FeasibilityRuleKind,
    pub applicability: RuleApplicability,
    pub limit: RuleLimit,
    /// The engineering source and revision this limit was transcribed from.
    pub source_reference: String,
    /// Optional short note for the reviewer; never an inferred limit of its own.
    pub note: Option<String>,
}

impl FeasibilityRule {
    /// Validate one rule before it is published or applied.
    pub fn validate(&self, field: &str) -> Result<()> {
        check_text(&self.source_reference, &format!("{field}.source_reference"))?;
        if self.source_reference.chars().count() > MAX_FEASIBILITY_SOURCE_REFERENCE_CHARS {
            return Err(ContractError::invalid(format!("{field}.source_reference"), "the source reference is too long"));
        }
        if let Some(note) = &self.note {
            check_text(note, &format!("{field}.note"))?;
            if note.chars().count() > MAX_FEASIBILITY_SOURCE_REFERENCE_CHARS {
                return Err(ContractError::invalid(format!("{field}.note"), "the note is too long"));
            }
        }
        self.applicability.validate(self.kind, &format!("{field}.applicability"))?;
        self.limit.validate(self.kind, &format!("{field}.limit"))
    }
}

/// One append-only version of a feasibility rule table. A new version supersedes the previous one;
/// earlier versions stay readable, so a recorded finding always names the exact rules it used.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeasibilityRuleTableVersion {
    pub rule_table_id: Uuid,
    pub rule_table_version_id: Uuid,
    pub version: u64,
    pub supersedes: Option<Uuid>,
    pub name: String,
    pub rules: Vec<FeasibilityRule>,
    pub source_reference: Option<String>,
    /// The Administrator identity that published this version.
    pub created_by: Option<String>,
    /// RFC 3339 UTC timestamp the service recorded.
    pub created_at: Option<String>,
}

impl FeasibilityRuleTableVersion {
    /// Validate the version, its rules and their identities.
    pub fn validate(&self) -> Result<()> {
        if self.version == 0 {
            return Err(ContractError::invalid("feasibility_rule_table.version", "a published version starts at 1"));
        }
        check_text(&self.name, "feasibility_rule_table.name")?;
        if self.name.chars().count() > MAX_RULE_NAME_CHARS {
            return Err(ContractError::invalid("feasibility_rule_table.name", "the rule-table name is too long"));
        }
        if let Some(reference) = &self.source_reference {
            check_text(reference, "feasibility_rule_table.source_reference")?;
            if reference.chars().count() > MAX_FEASIBILITY_SOURCE_REFERENCE_CHARS {
                return Err(ContractError::invalid("feasibility_rule_table.source_reference", "the source reference is too long"));
            }
        }
        if let Some(name) = &self.created_by {
            check_text(name, "feasibility_rule_table.created_by")?;
            if name.chars().count() > MAX_RULE_NAME_CHARS {
                return Err(ContractError::invalid("feasibility_rule_table.created_by", "the publisher name is too long"));
            }
        }
        if self.rules.len() > MAX_RULES_PER_TABLE {
            return Err(ContractError::invalid("feasibility_rule_table.rules", format!("a rule table holds at most {MAX_RULES_PER_TABLE} rules")));
        }
        let mut rule_ids = BTreeSet::new();
        for (index, rule) in self.rules.iter().enumerate() {
            rule.validate(&format!("feasibility_rule_table.rules.{index}"))?;
            if !rule_ids.insert(rule.rule_id) {
                return Err(ContractError::invalid("feasibility_rule_table.rules", "two rules share one rule id"));
            }
        }
        Ok(())
    }
}

/// Publish the next rule-table version. `expected_version` 0 creates version 1 of a new table.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateFeasibilityRuleTableVersion {
    pub request_id: String,
    pub expected_version: u64,
    pub rule_table_id: Uuid,
    pub name: String,
    pub rules: Vec<FeasibilityRule>,
    pub source_reference: Option<String>,
}

impl CreateFeasibilityRuleTableVersion {
    /// Validate the request before the service writes it.
    pub fn validate(&self) -> Result<()> {
        check_request_id(&self.request_id, "create_feasibility_rule_table.request_id")?;
        // Reuse the published-version validator; the ids and timestamps are filled in on publish.
        FeasibilityRuleTableVersion {
            rule_table_id: self.rule_table_id,
            rule_table_version_id: Uuid::nil(),
            version: 1,
            supersedes: None,
            name: self.name.clone(),
            rules: self.rules.clone(),
            source_reference: self.source_reference.clone(),
            created_by: None,
            created_at: None,
        }
        .validate()
    }
}

/// One published version of a rule table.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeasibilityRuleTablePublished {
    pub rule_table: FeasibilityRuleTableVersion,
    pub change: FeasibilityRuleChange,
}

/// One append-only change of a rule table, for later packages (SB-20) that flag affected projects.
/// The fact names what was published; it never adopts a new rule into a project.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeasibilityRuleChange {
    pub rule_table_id: Uuid,
    pub rule_table_version_id: Uuid,
    pub version: u64,
    pub previous_version_id: Option<Uuid>,
    pub summary: String,
    pub changed_by: String,
    /// RFC 3339 UTC timestamp the service recorded.
    pub changed_at: String,
}

/// Every published rule-table version and the newest change facts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeasibilityRulesView {
    pub identity: Identity,
    /// Every published rule-table version, oldest version first per table.
    pub rule_tables: Vec<FeasibilityRuleTableVersion>,
    /// Newest change facts first, bounded by [`MAX_FEASIBILITY_CHANGES`].
    pub changes: Vec<FeasibilityRuleChange>,
}

impl FeasibilityRulesView {
    /// Validate the versions and the facts.
    pub fn validate(&self) -> Result<()> {
        self.identity.validate()?;
        if self.changes.len() > MAX_FEASIBILITY_CHANGES {
            return Err(ContractError::invalid("feasibility_rules.changes", "too many rule changes"));
        }
        let mut versions = BTreeSet::new();
        for table in &self.rule_tables {
            table.validate()?;
            if !versions.insert(table.rule_table_version_id) {
                return Err(ContractError::invalid("feasibility_rules.rule_tables", "two versions share one version id"));
            }
        }
        Ok(())
    }
}

/// The exact reviewed tube geometry of one component. Every length is millimetres and came from the
/// SB-12 extraction of the registered analysis (or from a later reviewed completion with its own
/// provenance). Missing values are refused here: an unavailable extraction is represented by
/// `measurements: None` on the component, not by zero lengths.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeasibilityTubeMeasurements {
    pub outer_diameter_mm: Decimal,
    pub inner_diameter_mm: Decimal,
    /// Every bend's centerline radius, start-to-end order. Each is checked independently.
    pub bend_radii_mm: Vec<Decimal>,
    /// The straight centerline lengths between consecutive bend tangent points, tangent to tangent,
    /// start-to-end order. Exactly `bend_radii_mm.len() - 1` entries.
    pub tangent_straight_lengths_mm: Vec<Decimal>,
    /// The straight length at the start end and at the end end, in that order; two entries.
    pub straight_end_lengths_mm: Vec<Decimal>,
}

impl FeasibilityTubeMeasurements {
    /// Validate the measured values and their required counts.
    pub fn validate(&self, field: &str) -> Result<()> {
        if !(self.outer_diameter_mm > Decimal::ZERO && self.inner_diameter_mm > Decimal::ZERO && self.outer_diameter_mm > self.inner_diameter_mm) {
            return Err(ContractError::invalid(
                format!("{field}.outer_diameter_mm"),
                "the outer diameter must be positive and above the inner diameter",
            ));
        }
        if self.bend_radii_mm.len() > MAX_TUBE_BENDS {
            return Err(ContractError::invalid(format!("{field}.bend_radii_mm"), "the tube has too many bends"));
        }
        for radius in &self.bend_radii_mm {
            check_positive(*radius, &format!("{field}.bend_radii_mm"))?;
        }
        if self.tangent_straight_lengths_mm.len() != self.bend_radii_mm.len().saturating_sub(1) {
            return Err(ContractError::invalid(
                format!("{field}.tangent_straight_lengths_mm"),
                "there must be one tangent-to-tangent straight length between consecutive bends",
            ));
        }
        for length in &self.tangent_straight_lengths_mm {
            check_non_negative(*length, &format!("{field}.tangent_straight_lengths_mm"))?;
        }
        if self.straight_end_lengths_mm.len() != 2 {
            return Err(ContractError::invalid(format!("{field}.straight_end_lengths_mm"), "a tube needs one straight length per end"));
        }
        for length in &self.straight_end_lengths_mm {
            check_non_negative(*length, &format!("{field}.straight_end_lengths_mm"))?;
        }
        if self.bend_radii_mm.is_empty() && self.straight_end_lengths_mm[0] != self.straight_end_lengths_mm[1] {
            return Err(ContractError::invalid(format!("{field}.straight_end_lengths_mm"), "a straight tube reports its whole length at both ends"));
        }
        Ok(())
    }

    /// The wall thickness derived from the two measured diameters. Exact arithmetic; the extractor
    /// never rounds a measured size into a nominal one.
    pub fn wall_thickness_mm(&self) -> Option<Decimal> {
        let wall = self.outer_diameter_mm.checked_sub(self.inner_diameter_mm)?;
        wall.checked_div(Decimal::from(2))
    }
}

/// One confirmed connector association of a tube end. The catalog identity and the exact published
/// catalog version are kept; feasibility never guesses compatibility from geometry alone.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeasibilityConnector {
    /// The logical catalog component id.
    pub catalog_component_id: Uuid,
    /// The exact published catalog version the association was confirmed against.
    pub catalog_component_version_id: Uuid,
}

/// One reviewed tube end: its end identity and the confirmed connector association, if any.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeasibilityTubeEnd {
    pub end: TubeEnd,
    /// `None` means no connector association is confirmed for this end; connector checks stay
    /// unresolved instead of passing.
    pub connector: Option<FeasibilityConnector>,
}

/// The reviewed feasibility evidence of one tube component: what the checks consume.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeasibilityComponentEvidence {
    /// The reviewed component of SB-11 whose body set this evidence describes.
    pub component_id: Uuid,
    /// The product revision the reviewed component belongs to.
    pub product_revision_id: Uuid,
    /// The exact source, engine build and extractor identity the measurements came from.
    pub analysis_key: AnalysisKey,
    /// The extraction algorithm version that produced the measurements. A consumer comparing a
    /// saved result with a fresh analysis must also require the extractor's own current version.
    pub extraction_version: String,
    pub attributes: FeasibilityTubeAttributes,
    /// `None` means no exact tube measurements are available; every dependent check is unresolved,
    /// never passed. `Some` carries every required value.
    pub measurements: Option<FeasibilityTubeMeasurements>,
    /// The reviewed ends, start first when measurements are available. Empty is allowed only when
    /// no measurements are available.
    pub ends: Vec<FeasibilityTubeEnd>,
}

impl FeasibilityComponentEvidence {
    /// Validate the evidence and its internal consistency.
    pub fn validate(&self, field: &str) -> Result<()> {
        self.analysis_key.validate()?;
        check_text(&self.extraction_version, &format!("{field}.extraction_version"))?;
        if self.extraction_version.chars().count() > MAX_EXTRACTION_VERSION_CHARS {
            return Err(ContractError::invalid(format!("{field}.extraction_version"), "the extraction version is too long"));
        }
        self.attributes.validate(&format!("{field}.attributes"))?;
        match &self.measurements {
            Some(measurements) => {
                measurements.validate(&format!("{field}.measurements"))?;
                if self.ends.len() != 2 {
                    return Err(ContractError::invalid(format!("{field}.ends"), "a measured tube needs both ends, start first"));
                }
                if self.ends.first().map(|end| end.end) != Some(TubeEnd::Start) || self.ends.get(1).map(|end| end.end) != Some(TubeEnd::End) {
                    return Err(ContractError::invalid(format!("{field}.ends"), "the ends must be listed start first"));
                }
            }
            None => {
                if !self.ends.is_empty() {
                    return Err(ContractError::invalid(format!("{field}.ends"), "ends need the exact measurements they belong to"));
                }
            }
        }
        Ok(())
    }
}

/// Record one feasibility assessment of a project: the exact reviewed evidence and the rule-table
/// version it is evaluated against. The findings are computed from that evidence; nothing is
/// inferred from a prior assessment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecordFeasibilityAssessment {
    pub request_id: String,
    pub expected_version: u64,
    /// The exact published rule-table version to evaluate against. A newer version is a different
    /// assessment, never a silent re-evaluation of a recorded one.
    pub rule_table_version_id: Uuid,
    /// One entry per reviewed tube component of the project. Assessment completeness is a client
    /// obligation: the service cannot infer which reviewed components are tubes from a free label.
    pub components: Vec<FeasibilityComponentEvidence>,
}

impl RecordFeasibilityAssessment {
    /// Validate the request and every piece of evidence before the service evaluates it.
    pub fn validate(&self) -> Result<()> {
        check_request_id(&self.request_id, "record_feasibility_assessment.request_id")?;
        if self.components.is_empty() {
            return Err(ContractError::invalid(
                "record_feasibility_assessment.components",
                "an assessment names at least one reviewed tube component",
            ));
        }
        if self.components.len() > MAX_ASSESSED_COMPONENTS {
            return Err(ContractError::invalid("record_feasibility_assessment.components", "the assessment names too many components"));
        }
        let mut component_ids = BTreeSet::new();
        for (index, component) in self.components.iter().enumerate() {
            component.validate(&format!("record_feasibility_assessment.components.{index}"))?;
            if !component_ids.insert(component.component_id) {
                return Err(ContractError::invalid("record_feasibility_assessment.components", "a component appears twice in one assessment"));
            }
        }
        Ok(())
    }
}

/// One reviewed component as the assessment recorded it: the exact evidence plus the reviewed
/// binding it was verified against, so a later review change is detectable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AssessedComponent {
    pub evidence: FeasibilityComponentEvidence,
    /// The shared project part version the reviewed component was associated with at assessment.
    pub part_version_id: Option<Uuid>,
    /// The reviewed body set at assessment. Compared with the current engineering review to detect
    /// a changed supporting geometry.
    pub reviewed_geometry: Vec<ReviewedGeometry>,
}

impl AssessedComponent {
    fn validate(&self) -> Result<()> {
        self.evidence.validate("assessed_component.evidence")?;
        if self.reviewed_geometry.is_empty() {
            return Err(ContractError::invalid("assessed_component.reviewed_geometry", "a reviewed component needs at least one body"));
        }
        if self.reviewed_geometry.len() > crate::engineering::MAX_COMPONENT_GEOMETRY {
            return Err(ContractError::invalid("assessed_component.reviewed_geometry", "the component holds too many bodies"));
        }
        Ok(())
    }

    /// Whether the current engineering review still describes exactly this component. A changed
    /// analysis, part association or body set is a changed supporting geometry.
    pub fn matches_review(&self, engineering: &EngineeringRecordDocument) -> bool {
        engineering.component(self.evidence.component_id).is_some_and(|current| {
            current.product_revision_id == self.evidence.product_revision_id
                && current.analysis_key == self.evidence.analysis_key
                && current.part_version_id == self.part_version_id
                && same_geometry(&current.geometry, &self.reviewed_geometry)
        })
    }
}

/// One check a finding is about. `WholeTube` is used when the tube measurements are unavailable, so
/// the finding says one check per kind is unresolved instead of inventing a bend count.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(tag = "check", rename_all = "snake_case")]
pub enum FindingCheck {
    WholeTube,
    BendRadius { bend_index: u32 },
    BendSpacing { first_bend_index: u32 },
    ConnectorCompatibility { end: TubeEnd },
    EndAssemblyLength { end: TubeEnd },
}

/// The stable identity of one finding inside one assessment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FindingKey {
    pub component_id: Uuid,
    pub product_revision_id: Uuid,
    pub kind: FeasibilityRuleKind,
    pub check: FindingCheck,
}

impl FindingKey {
    fn validate(&self) -> Result<()> {
        let matched = matches!(
            (&self.kind, &self.check),
            (FeasibilityRuleKind::MinimumBendRadius, FindingCheck::WholeTube | FindingCheck::BendRadius { .. })
                | (FeasibilityRuleKind::BendSpacing, FindingCheck::WholeTube | FindingCheck::BendSpacing { .. })
                | (FeasibilityRuleKind::TubeConnectorCompatibility, FindingCheck::WholeTube | FindingCheck::ConnectorCompatibility { .. })
                | (FeasibilityRuleKind::EndAssemblyLength, FindingCheck::WholeTube | FindingCheck::EndAssemblyLength { .. })
        );
        if matched { Ok(()) } else { Err(ContractError::invalid("finding_key", "the check does not belong to the rule kind")) }
    }
}

/// Why a check did not produce a geometric pass or fail.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UnresolvedReason {
    /// The exact reviewed measurement is unavailable (for example an unsupported extraction).
    MissingMeasurement,
    /// The end has no confirmed connector association, so its requirement is unknown.
    MissingConnectorAssociation,
    /// No applicable rule supplies a usable limit; the rule data is a prerequisite, not a default.
    NoApplicableLimit,
}

/// The geometric result of one check. A pass is only ever computed from an applicable limit and a
/// measured value.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FindingState {
    Pass,
    Fail,
    Unresolved,
    Conflict,
}

impl FindingState {
    /// Whether the check blocks a clean readiness result before an exception.
    pub fn blocks(self) -> bool {
        !matches!(self, Self::Pass)
    }
}

/// One measured value that supports a finding.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SupportingMeasurement {
    /// The stable field path of the value, such as `bends.2.centerline_radius_mm`.
    pub field_path: String,
    /// The exact measured value in millimetres.
    pub value_mm: Decimal,
}

/// One rule that applies to a finding, with the limit it supplied.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppliedRule {
    pub rule_id: Uuid,
    pub rule_table_id: Uuid,
    pub rule_table_version_id: Uuid,
    pub rule_table_version: u64,
    pub source_reference: String,
    pub limit: RuleLimit,
}

/// One computed feasibility finding. The finding names its rule source, its required limit and the
/// measurements it compared; it is never stored as a pass that a later input change could reuse.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeasibilityFinding {
    pub key: FindingKey,
    pub state: FindingState,
    /// Set exactly when `state` is `Unresolved`.
    pub unresolved_reason: Option<UnresolvedReason>,
    /// The measured values the check compared, in a stable order.
    pub measurements: Vec<SupportingMeasurement>,
    /// Every rule that applies. A conflict lists all of them; an agreed result lists the agreeing
    /// rules.
    pub applied_rules: Vec<AppliedRule>,
    /// The agreed required limit; `None` for a conflict or when no applicable limit exists.
    pub required_limit: Option<RuleLimit>,
}

impl FeasibilityFinding {
    /// Validate the finding's internal consistency.
    pub fn validate(&self, field: &str) -> Result<()> {
        self.key.validate()?;
        match self.state {
            FindingState::Unresolved => {
                if self.unresolved_reason.is_none() {
                    return Err(ContractError::invalid(field, "an unresolved finding needs its reason"));
                }
                if self.required_limit.is_some() {
                    return Err(ContractError::invalid(field, "an unresolved finding has no agreed required limit"));
                }
            }
            FindingState::Conflict => {
                if self.unresolved_reason.is_some() {
                    return Err(ContractError::invalid(field, "a conflict is not an unresolved reason"));
                }
                if self.required_limit.is_some() {
                    return Err(ContractError::invalid(field, "a conflict has no silently chosen limit"));
                }
                if self.applied_rules.len() < 2 {
                    return Err(ContractError::invalid(field, "a conflict names the overlapping rules"));
                }
            }
            FindingState::Pass | FindingState::Fail => {
                if self.unresolved_reason.is_some() || self.required_limit.is_none() || self.applied_rules.is_empty() {
                    return Err(ContractError::invalid(field, "a pass or fail needs its limit and applicable rules"));
                }
            }
        }
        Ok(())
    }

    /// Whether an engineer may accept a justified exception for this finding. A missing measurement
    /// or connector association is review work, not an engineering-limit decision; a pass needs no
    /// exception.
    pub fn exception_is_available(&self) -> bool {
        match self.state {
            FindingState::Fail | FindingState::Conflict => true,
            FindingState::Unresolved => matches!(self.unresolved_reason, Some(UnresolvedReason::NoApplicableLimit)),
            FindingState::Pass => false,
        }
    }
}

/// The stored, appended feasibility record of one project: assessments and the current exception
/// decisions. Earlier decisions stay readable in the immutable record versions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FeasibilityRecordDocument {
    pub schema_version: u32,
    pub project_id: Uuid,
    pub assessments: Vec<FeasibilityAssessment>,
    /// The current exception of each finding key. A replaced exception disappears from this list
    /// but stays readable in the earlier record version that held it.
    pub exceptions: Vec<FeasibilityException>,
}

/// One recorded assessment: the exact evidence, the rule version and the computed findings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FeasibilityAssessment {
    /// The feasibility record version this assessment was appended at.
    pub version: u64,
    pub rule_table_version_id: Uuid,
    /// The engineering review version the evidence was verified against.
    pub engineering_record_version: u64,
    pub created_by: String,
    /// RFC 3339 UTC timestamp the service recorded.
    pub created_at: String,
    pub components: Vec<AssessedComponent>,
    pub findings: Vec<FeasibilityFinding>,
}

impl FeasibilityAssessment {
    fn validate(&self) -> Result<()> {
        if self.version == 0 {
            return Err(ContractError::invalid("feasibility_assessment.version", "a recorded assessment names its version"));
        }
        check_text(&self.created_by, "feasibility_assessment.created_by")?;
        check_text(&self.created_at, "feasibility_assessment.created_at")?;
        if self.components.is_empty() {
            return Err(ContractError::invalid("feasibility_assessment.components", "an assessment names at least one reviewed tube component"));
        }
        if self.components.len() > MAX_ASSESSED_COMPONENTS {
            return Err(ContractError::invalid("feasibility_assessment.components", "the assessment names too many components"));
        }
        let mut component_ids = BTreeSet::new();
        for component in &self.components {
            component.validate()?;
            if !component_ids.insert(component.evidence.component_id) {
                return Err(ContractError::invalid("feasibility_assessment.components", "two assessed components share one component id"));
            }
        }
        if self.findings.len() > MAX_FINDINGS_PER_ASSESSMENT {
            return Err(ContractError::invalid("feasibility_assessment.findings", "the assessment holds too many findings"));
        }
        let mut keys = BTreeSet::new();
        for finding in &self.findings {
            finding.validate("feasibility_assessment.findings")?;
            if !keys.insert(finding.key.clone()) {
                return Err(ContractError::invalid("feasibility_assessment.findings", "two findings share one key"));
            }
        }
        Ok(())
    }

    /// The finding with `key`, when this assessment computed it.
    pub fn finding(&self, key: &FindingKey) -> Option<&FeasibilityFinding> {
        self.findings.iter().find(|finding| &finding.key == key)
    }

    /// The assessed component with `component_id`.
    pub fn component(&self, component_id: Uuid) -> Option<&AssessedComponent> {
        self.components.iter().find(|component| component.evidence.component_id == component_id)
    }
}

/// A justified engineer exception. It keeps the exact non-passing finding and supporting evidence it
/// was accepted against, so a later change is detectable instead of silently reusing the decision.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FeasibilityException {
    pub exception_id: Uuid,
    pub finding_key: FindingKey,
    /// The engineer's justification for accepting the non-passing result.
    pub justification: String,
    /// The authenticated engineer identity; the service fills it, never the payload.
    pub accepted_by: String,
    /// RFC 3339 UTC timestamp the service recorded.
    pub accepted_at: String,
    /// The assessment version whose finding was accepted.
    pub assessment_version: u64,
    /// The exact rule-table version the finding was computed with.
    pub rule_table_version_id: Uuid,
    /// The assessed component and its reviewed binding at acceptance.
    pub component: AssessedComponent,
    /// The accepted finding snapshot. Its geometric state stays visible; the exception only changes
    /// the disposition.
    pub finding: FeasibilityFinding,
}

impl FeasibilityException {
    fn validate(&self) -> Result<()> {
        self.finding_key.validate()?;
        check_text(&self.justification, "feasibility_exception.justification")?;
        if self.justification.chars().count() > MAX_JUSTIFICATION_CHARS {
            return Err(ContractError::invalid("feasibility_exception.justification", "the justification is too long"));
        }
        check_text(&self.accepted_by, "feasibility_exception.accepted_by")?;
        check_text(&self.accepted_at, "feasibility_exception.accepted_at")?;
        if self.assessment_version == 0 {
            return Err(ContractError::invalid("feasibility_exception.assessment_version", "an exception names its assessment"));
        }
        self.component.validate()?;
        self.finding.validate("feasibility_exception.finding")?;
        if !self.finding.exception_is_available() {
            return Err(ContractError::invalid(
                "feasibility_exception.finding",
                "an exception covers a failed, conflicting or missing-limit finding, not a pass or a missing measurement",
            ));
        }
        if self.finding.key != self.finding_key {
            return Err(ContractError::invalid("feasibility_exception.finding_key", "the exception key and its finding disagree"));
        }
        Ok(())
    }
}

/// Accept a justified exception for one recorded finding. The service verifies the finding, the
/// authenticated reviewer and the unchanged supporting evidence before it stores the decision.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AcceptFeasibilityException {
    pub request_id: String,
    pub expected_version: u64,
    pub assessment_version: u64,
    pub finding_key: FindingKey,
    pub justification: String,
}

impl AcceptFeasibilityException {
    /// Validate the request before the service verifies it against the record.
    pub fn validate(&self) -> Result<()> {
        check_request_id(&self.request_id, "accept_feasibility_exception.request_id")?;
        self.finding_key.validate()?;
        check_text(&self.justification, "accept_feasibility_exception.justification")?;
        if self.justification.chars().count() > MAX_JUSTIFICATION_CHARS {
            return Err(ContractError::invalid("accept_feasibility_exception.justification", "the justification is too long"));
        }
        Ok(())
    }
}

/// Whether the exception still covers the current supporting evidence.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExceptionState {
    /// The supporting review and the current assessment still match the accepted evidence.
    Active,
    /// A supporting input changed; the exception stays visible and asks for review again.
    ReviewRequired,
}

/// Why an accepted exception asks for review again.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExceptionInvalidationReason {
    /// The supporting geometry, analysis or part association changed in the engineering review.
    EngineeringReviewChanged,
    /// The current assessment changed the evidence, rule version or computed finding.
    AssessmentChanged,
}

/// An exception as a reader sees it: the decision, its current state and why it needs review again.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FeasibilityExceptionView {
    pub exception: FeasibilityException,
    pub state: ExceptionState,
    pub review_required_reasons: Vec<ExceptionInvalidationReason>,
}

/// The current disposition of one finding.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "disposition", rename_all = "snake_case")]
pub enum FindingDisposition {
    /// No accepted exception covers this finding.
    Open,
    /// An accepted exception covers it and still matches the current evidence.
    ExceptionAccepted { exception_id: Uuid },
    /// An accepted exception covered it, but a supporting input changed.
    ExceptionReviewRequired { exception_id: Uuid },
}

/// One finding with its current disposition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FeasibilityFindingView {
    pub finding: FeasibilityFinding,
    pub disposition: FindingDisposition,
}

/// One recorded assessment with the current disposition of each finding.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FeasibilityAssessmentView {
    pub assessment: FeasibilityAssessment,
    pub findings: Vec<FeasibilityFindingView>,
}

/// One computed readiness issue. Issues are never stored; a missing input or an open finding is an
/// issue, never a pass.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeasibilityIssue {
    pub code: String,
    pub field_path: String,
    pub message: String,
    pub component_id: Option<Uuid>,
    pub finding_key: Option<FindingKey>,
}

impl FeasibilityIssue {
    fn new(code: impl Into<String>, field_path: impl Into<String>, message: impl Into<String>) -> Self {
        Self { code: code.into(), field_path: field_path.into(), message: message.into(), component_id: None, finding_key: None }
    }

    fn for_finding(mut self, finding: &FeasibilityFinding) -> Self {
        self.component_id = Some(finding.key.component_id);
        self.finding_key = Some(finding.key.clone());
        self
    }

    fn for_component(mut self, component_id: Uuid) -> Self {
        self.component_id = Some(component_id);
        self
    }
}

/// A compact receipt of one committed feasibility write.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeasibilityReceipt {
    pub request_id: String,
    pub project_id: Uuid,
    pub version: u64,
}

/// One dependency fact a feasibility write emits. Facts name what changed; they never rewrite an
/// accepted result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FeasibilityFact {
    /// One assessment was recorded with its computed findings. `non_passing_finding_count` counts
    /// every finding that is not a pass, including one an accepted exception already covers; it is
    /// not a count of open work.
    AssessmentRecorded { version: u64, finding_count: u64, non_passing_finding_count: u64 },
    /// One justified exception was accepted; `replaced` is the previous exception for the same
    /// finding key, when the decision replaced one.
    ExceptionAccepted { exception_id: Uuid, finding_key: FindingKey, replaced: Option<Uuid> },
}

/// One committed assessment write.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FeasibilityAssessmentOutcome {
    pub receipt: FeasibilityReceipt,
    pub assessment: FeasibilityAssessmentView,
    pub issues: Vec<FeasibilityIssue>,
    pub facts: Vec<FeasibilityFact>,
}

/// One committed exception write.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FeasibilityWriteOutcome {
    pub receipt: FeasibilityReceipt,
    pub facts: Vec<FeasibilityFact>,
}

/// What a reader of a project's feasibility record receives: the latest assessment with current
/// dispositions, the current exceptions with their state, and the readiness issues.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FeasibilityRecordView {
    pub project_id: Uuid,
    pub version: u64,
    pub latest_assessment: Option<FeasibilityAssessmentView>,
    pub exceptions: Vec<FeasibilityExceptionView>,
    pub issues: Vec<FeasibilityIssue>,
}

impl FeasibilityRecordDocument {
    /// The empty feasibility record of a project with no recorded decision yet.
    pub fn empty(project_id: Uuid) -> Self {
        Self { schema_version: FEASIBILITY_DOCUMENT_SCHEMA_VERSION, project_id, assessments: Vec::new(), exceptions: Vec::new() }
    }

    /// Validate a stored document before it is written and before clients read one.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != FEASIBILITY_DOCUMENT_SCHEMA_VERSION {
            return Err(ContractError::SchemaVersion { found: self.schema_version, expected: FEASIBILITY_DOCUMENT_SCHEMA_VERSION });
        }
        if self.assessments.len() > MAX_ASSESSMENTS {
            return Err(ContractError::invalid(
                "feasibility.assessments",
                format!("a feasibility record holds at most {MAX_ASSESSMENTS} assessments"),
            ));
        }
        if self.exceptions.len() > MAX_EXCEPTIONS {
            return Err(ContractError::invalid("feasibility.exceptions", format!("a feasibility record holds at most {MAX_EXCEPTIONS} exceptions")));
        }
        let mut versions = BTreeSet::new();
        for assessment in &self.assessments {
            assessment.validate()?;
            if !versions.insert(assessment.version) {
                return Err(ContractError::invalid("feasibility.assessments", "two assessments share one version"));
            }
        }
        let mut keys = BTreeSet::new();
        for exception in &self.exceptions {
            exception.validate()?;
            if !keys.insert(exception.finding_key.clone()) {
                return Err(ContractError::invalid("feasibility.exceptions", "two exceptions share one finding key"));
            }
            let assessment = self.assessment(exception.assessment_version).ok_or_else(|| {
                ContractError::invalid("feasibility.exceptions.assessment_version", "an exception names an assessment the record does not hold")
            })?;
            let finding = assessment.finding(&exception.finding_key).ok_or_else(|| {
                ContractError::invalid("feasibility.exceptions.finding_key", "an exception names a finding its assessment does not hold")
            })?;
            if finding != &exception.finding {
                return Err(ContractError::invalid("feasibility.exceptions.finding", "the accepted finding differs from its assessment's finding"));
            }
        }
        Ok(())
    }

    /// The recorded assessment with `version`.
    pub fn assessment(&self, version: u64) -> Option<&FeasibilityAssessment> {
        self.assessments.iter().find(|assessment| assessment.version == version)
    }

    /// The latest recorded assessment, if any.
    pub fn latest_assessment(&self) -> Option<&FeasibilityAssessment> {
        self.assessments.iter().max_by_key(|assessment| assessment.version)
    }

    /// The current exception of `key`, if one is accepted.
    pub fn exception(&self, key: &FindingKey) -> Option<&FeasibilityException> {
        self.exceptions.iter().find(|exception| &exception.finding_key == key)
    }

    /// Every current exception with its state computed against the current engineering review and
    /// the latest assessment.
    pub fn exception_views(&self, engineering: &EngineeringRecordDocument) -> Vec<FeasibilityExceptionView> {
        let latest = self.latest_assessment();
        self.exceptions.iter().map(|exception| exception_view(exception, engineering, latest)).collect()
    }

    /// The record as a reader sees it: latest assessment dispositions, exception states and the
    /// computed readiness issues.
    pub fn record_view(&self, version: u64, engineering: &EngineeringRecordDocument) -> FeasibilityRecordView {
        let exceptions = self.exception_views(engineering);
        let latest_assessment = self.latest_assessment().map(|assessment| assessment_view(assessment, &exceptions));
        let issues = self.issues_with(engineering, &exceptions);
        FeasibilityRecordView { project_id: self.project_id, version, latest_assessment, exceptions, issues }
    }

    /// The readiness issues of the project's feasibility record.
    pub fn issues(&self, engineering: &EngineeringRecordDocument) -> Vec<FeasibilityIssue> {
        let exceptions = self.exception_views(engineering);
        self.issues_with(engineering, &exceptions)
    }

    fn issues_with(&self, engineering: &EngineeringRecordDocument, exceptions: &[FeasibilityExceptionView]) -> Vec<FeasibilityIssue> {
        let mut issues = Vec::new();
        let Some(latest) = self.latest_assessment() else {
            issues.push(FeasibilityIssue::new(
                "feasibility_not_assessed",
                "feasibility",
                "no feasibility assessment is recorded; the tube checks are not evaluated",
            ));
            return issues;
        };
        // A changed supporting geometry makes the assessment stale. It stays visible with the
        // findings it recorded, but it is not evidence about the current review.
        for component in &latest.components {
            if !component.matches_review(engineering) {
                issues.push(
                    FeasibilityIssue::new(
                        "assessment_stale",
                        format!("assessments.{}.components.{}", latest.version, component.evidence.component_id),
                        "the supporting geometry or part association changed after this assessment; assess the current review again",
                    )
                    .for_component(component.evidence.component_id),
                );
            }
        }
        for view in exceptions {
            if view.state == ExceptionState::ReviewRequired {
                issues.push(
                    FeasibilityIssue::new(
                        "exception_review_required",
                        format!("exceptions.{}", view.exception.exception_id),
                        "the accepted exception's supporting inputs changed; review the exception again",
                    )
                    .for_finding(&view.exception.finding),
                );
            }
        }
        for finding in &latest.findings {
            if !finding.state.blocks() {
                continue;
            }
            // An accepted or re-review exception is the documented disposition; it is reported once
            // as the exception issue above instead of as another open finding.
            if exceptions.iter().any(|view| view.exception.finding_key == finding.key) {
                continue;
            }
            let (code, message) = match finding.state {
                FindingState::Fail => ("open_finding_failed", "a feasibility check fails and has no accepted exception"),
                FindingState::Conflict => ("open_finding_conflict", "overlapping rule limits conflict and an Administrator must resolve them"),
                FindingState::Unresolved => ("open_finding_unresolved", "a feasibility check is unresolved because its data is missing"),
                FindingState::Pass => continue,
            };
            issues.push(FeasibilityIssue::new(code, format!("assessments.{}.findings", latest.version), message).for_finding(finding));
        }
        issues
    }
}

/// The current view of one accepted exception.
fn exception_view(
    exception: &FeasibilityException,
    engineering: &EngineeringRecordDocument,
    latest_assessment: Option<&FeasibilityAssessment>,
) -> FeasibilityExceptionView {
    let mut review_required_reasons = Vec::new();
    if !exception.component.matches_review(engineering) {
        review_required_reasons.push(ExceptionInvalidationReason::EngineeringReviewChanged);
    }
    let assessment_changed = match latest_assessment {
        None => true,
        Some(latest) => {
            latest.rule_table_version_id != exception.rule_table_version_id
                || latest.finding(&exception.finding_key) != Some(&exception.finding)
                || latest.component(exception.finding_key.component_id) != Some(&exception.component)
        }
    };
    if assessment_changed {
        review_required_reasons.push(ExceptionInvalidationReason::AssessmentChanged);
    }
    let state = if review_required_reasons.is_empty() { ExceptionState::Active } else { ExceptionState::ReviewRequired };
    FeasibilityExceptionView { exception: exception.clone(), state, review_required_reasons }
}

/// Attach the current dispositions to one recorded assessment.
pub fn assessment_view(assessment: &FeasibilityAssessment, exceptions: &[FeasibilityExceptionView]) -> FeasibilityAssessmentView {
    let findings = assessment
        .findings
        .iter()
        .map(|finding| {
            let disposition = match exceptions.iter().find(|view| view.exception.finding_key == finding.key) {
                None => FindingDisposition::Open,
                Some(view) => match view.state {
                    ExceptionState::Active => FindingDisposition::ExceptionAccepted { exception_id: view.exception.exception_id },
                    ExceptionState::ReviewRequired => FindingDisposition::ExceptionReviewRequired { exception_id: view.exception.exception_id },
                },
            };
            FeasibilityFindingView { finding: finding.clone(), disposition }
        })
        .collect();
    FeasibilityAssessmentView { assessment: assessment.clone(), findings }
}

/// Evaluate one assessment's evidence against one immutable rule-table version.
///
/// The result is deterministic: components in input order, then within a component the bend radii in
/// order, the tangent-to-tangent spacings in order, the compatibility check of the start end and the
/// end end, and the end-assembly-length check of the start end and the end end. Every bend radius is
/// its own finding. A conflict or an unresolved finding is returned as such; a pass is only ever
/// computed from an applicable limit and a measured value.
pub fn evaluate_feasibility(
    rule_table: &FeasibilityRuleTableVersion,
    components: &[FeasibilityComponentEvidence],
) -> Result<Vec<FeasibilityFinding>> {
    rule_table.validate()?;
    if components.len() > MAX_ASSESSED_COMPONENTS {
        return Err(ContractError::invalid("feasibility.components", "the assessment names too many components"));
    }
    let mut seen = BTreeSet::new();
    for component in components {
        component.validate("feasibility.components")?;
        if !seen.insert(component.component_id) {
            return Err(ContractError::invalid("feasibility.components", "a component appears twice in one assessment"));
        }
    }
    // Bound the work before allocating: a hostile request cannot ask for an unbounded number of
    // findings with every rule applied to each.
    let projected_findings = components
        .iter()
        .map(projected_findings)
        .try_fold(0usize, |total, count| total.checked_add(count))
        .ok_or_else(|| ContractError::invalid("feasibility.components", "the assessment names more components than this build can evaluate"))?;
    if projected_findings > MAX_FINDINGS_PER_ASSESSMENT {
        return Err(ContractError::invalid(
            "feasibility.components",
            format!("the assessment would compute more than {MAX_FINDINGS_PER_ASSESSMENT} findings"),
        ));
    }
    let work = projected_findings.saturating_mul(rule_table.rules.len().max(1));
    if work > MAX_EVALUATION_WORK {
        return Err(ContractError::invalid("feasibility.components", "the assessment would evaluate more rule findings than this build accepts"));
    }
    let mut findings = Vec::new();
    for component in components {
        evaluate_component(rule_table, component, &mut findings);
    }
    Ok(findings)
}

/// How many findings one component's evidence produces: every bend, one spacing between consecutive
/// bends, and the two connector checks per end. Missing measurements produce one whole-tube finding
/// per kept check kind.
fn projected_findings(component: &FeasibilityComponentEvidence) -> usize {
    match &component.measurements {
        None => 4,
        Some(measurements) => {
            if measurements.bend_radii_mm.is_empty() {
                4
            } else {
                measurements.bend_radii_mm.len().saturating_mul(2).saturating_add(3)
            }
        }
    }
}

fn evaluate_component(rule_table: &FeasibilityRuleTableVersion, component: &FeasibilityComponentEvidence, findings: &mut Vec<FeasibilityFinding>) {
    let Some(measurements) = &component.measurements else {
        // No exact measurement: one unresolved finding per rule kind, naming the missing data.
        for kind in [
            FeasibilityRuleKind::MinimumBendRadius,
            FeasibilityRuleKind::BendSpacing,
            FeasibilityRuleKind::TubeConnectorCompatibility,
            FeasibilityRuleKind::EndAssemblyLength,
        ] {
            findings.push(unresolved(finding_key(component, kind, FindingCheck::WholeTube), UnresolvedReason::MissingMeasurement, Vec::new()));
        }
        return;
    };

    // Every bend radius is its own finding and is tested independently.
    for (index, radius) in measurements.bend_radii_mm.iter().enumerate() {
        let Ok(bend_index) = u32::try_from(index) else { continue };
        let key = finding_key(component, FeasibilityRuleKind::MinimumBendRadius, FindingCheck::BendRadius { bend_index });
        let supporting = vec![SupportingMeasurement { field_path: format!("bend_radii_mm[{index}]"), value_mm: *radius }];
        findings.push(evaluate_numeric(
            rule_table,
            component,
            FeasibilityRuleKind::MinimumBendRadius,
            None,
            measurements,
            supporting,
            *radius,
            minimum_radius,
            key,
        ));
    }

    // Straight centerline spacing tangent to tangent between consecutive bends.
    for (position, spacing) in measurements.tangent_straight_lengths_mm.iter().enumerate() {
        let Ok(first_bend_index) = u32::try_from(position) else { continue };
        let key = finding_key(component, FeasibilityRuleKind::BendSpacing, FindingCheck::BendSpacing { first_bend_index });
        let supporting = vec![SupportingMeasurement { field_path: format!("tangent_straight_lengths_mm[{position}]"), value_mm: *spacing }];
        findings.push(evaluate_numeric(
            rule_table,
            component,
            FeasibilityRuleKind::BendSpacing,
            None,
            measurements,
            supporting,
            *spacing,
            minimum_length,
            key,
        ));
    }

    // Connector checks for each reviewed end.
    for end in &component.ends {
        let compatibility_key =
            finding_key(component, FeasibilityRuleKind::TubeConnectorCompatibility, FindingCheck::ConnectorCompatibility { end: end.end });
        if end.connector.is_none() {
            // The connector requirement is unknown, so the check can never pass.
            findings.push(unresolved(compatibility_key, UnresolvedReason::MissingConnectorAssociation, Vec::new()));
        } else {
            let supporting = vec![
                SupportingMeasurement { field_path: "outer_diameter_mm".into(), value_mm: measurements.outer_diameter_mm },
                SupportingMeasurement { field_path: "inner_diameter_mm".into(), value_mm: measurements.inner_diameter_mm },
            ];
            findings.push(evaluate_compatibility(rule_table, component, end.connector.as_ref(), measurements, supporting, compatibility_key));
        }

        let length_key = finding_key(component, FeasibilityRuleKind::EndAssemblyLength, FindingCheck::EndAssemblyLength { end: end.end });
        let Some(connector) = end.connector.as_ref() else {
            findings.push(unresolved(length_key, UnresolvedReason::MissingConnectorAssociation, Vec::new()));
            continue;
        };
        let Some(length) = measurements.straight_end_lengths_mm.get(usize::from(end.end == TubeEnd::End)) else {
            continue;
        };
        let supporting =
            vec![SupportingMeasurement { field_path: format!("straight_end_lengths_mm[{}]", u8::from(end.end == TubeEnd::End)), value_mm: *length }];
        findings.push(evaluate_numeric(
            rule_table,
            component,
            FeasibilityRuleKind::EndAssemblyLength,
            Some(connector),
            measurements,
            supporting,
            *length,
            minimum_length,
            length_key,
        ));
    }
}

/// Evaluate one numeric minimum check. `compare` maps an agreed limit to its minimum value; a limit
/// that cannot supply one is not usable and does not pass.
#[allow(clippy::too_many_arguments)]
fn evaluate_numeric(
    rule_table: &FeasibilityRuleTableVersion,
    component: &FeasibilityComponentEvidence,
    kind: FeasibilityRuleKind,
    connector: Option<&FeasibilityConnector>,
    measurements: &FeasibilityTubeMeasurements,
    supporting: Vec<SupportingMeasurement>,
    measured: Decimal,
    compare: fn(&RuleLimit) -> Option<Decimal>,
    key: FindingKey,
) -> FeasibilityFinding {
    let applicable = applicable_rules(rule_table, component, kind, connector, Some(measurements));
    match resolve_limits(rule_table, &applicable) {
        LimitResolution::None => unresolved(key, UnresolvedReason::NoApplicableLimit, supporting),
        LimitResolution::Conflict(rules) => conflict(key, supporting, rules),
        LimitResolution::Agreed(limit, rules) => {
            let Some(required) = compare(&limit) else {
                return unresolved(key, UnresolvedReason::NoApplicableLimit, supporting);
            };
            let state = if measured >= required { FindingState::Pass } else { FindingState::Fail };
            FeasibilityFinding { key, state, unresolved_reason: None, measurements: supporting, applied_rules: rules, required_limit: Some(limit) }
        }
    }
}

/// Evaluate one connector-compatibility check against both compatible ranges.
fn evaluate_compatibility(
    rule_table: &FeasibilityRuleTableVersion,
    component: &FeasibilityComponentEvidence,
    connector: Option<&FeasibilityConnector>,
    measurements: &FeasibilityTubeMeasurements,
    supporting: Vec<SupportingMeasurement>,
    key: FindingKey,
) -> FeasibilityFinding {
    let kind = FeasibilityRuleKind::TubeConnectorCompatibility;
    let applicable = applicable_rules(rule_table, component, kind, connector, Some(measurements));
    match resolve_limits(rule_table, &applicable) {
        LimitResolution::None => unresolved(key, UnresolvedReason::NoApplicableLimit, supporting),
        LimitResolution::Conflict(rules) => conflict(key, supporting, rules),
        LimitResolution::Agreed(limit, rules) => {
            let pass = match &limit {
                RuleLimit::CompatibleTubeRange { outer_diameter_mm, inner_diameter_mm } => {
                    outer_diameter_mm.contains(measurements.outer_diameter_mm) && inner_diameter_mm.contains(measurements.inner_diameter_mm)
                }
                _ => false,
            };
            FeasibilityFinding {
                key,
                state: if pass { FindingState::Pass } else { FindingState::Fail },
                unresolved_reason: None,
                measurements: supporting,
                applied_rules: rules,
                required_limit: Some(limit),
            }
        }
    }
}

/// The rules of the table that apply to one check, with their usable limits.
struct ApplicableRule {
    rule: FeasibilityRule,
    limit: RuleLimit,
}

fn applicable_rules(
    rule_table: &FeasibilityRuleTableVersion,
    component: &FeasibilityComponentEvidence,
    kind: FeasibilityRuleKind,
    connector: Option<&FeasibilityConnector>,
    measurements: Option<&FeasibilityTubeMeasurements>,
) -> Vec<ApplicableRule> {
    let mut applicable = Vec::new();
    for rule in &rule_table.rules {
        if rule.kind != kind {
            continue;
        }
        if !rule_applies(rule, &component.attributes, measurements, connector) {
            continue;
        }
        let Some(limit) = resolved_limit(kind, rule, measurements) else {
            // A rule whose limit cannot be resolved from the measured evidence is not a usable
            // limit; it cannot make the check pass.
            continue;
        };
        applicable.push(ApplicableRule { rule: rule.clone(), limit });
    }
    applicable
}

/// Whether every set applicability condition is proven by the reviewed evidence.
fn rule_applies(
    rule: &FeasibilityRule,
    attributes: &FeasibilityTubeAttributes,
    measurements: Option<&FeasibilityTubeMeasurements>,
    connector: Option<&FeasibilityConnector>,
) -> bool {
    let applicability = &rule.applicability;
    if let Some(kind) = applicability.tube_kind
        && attributes.tube_kind != Some(kind)
    {
        return false;
    }
    if let Some(name) = &applicability.material_name
        && attributes.material_name.as_deref() != Some(name.as_str())
    {
        return false;
    }
    if applicability.supporting_spring_required && attributes.supporting_spring != Some(true) {
        return false;
    }
    if let Some(nominal) = applicability.nominal_outer_diameter_mm
        && attributes.nominal_outer_diameter_mm != Some(nominal)
    {
        return false;
    }
    if let Some(nominal) = applicability.nominal_wall_thickness_mm
        && attributes.nominal_wall_thickness_mm != Some(nominal)
    {
        return false;
    }
    if let Some(nominal) = applicability.nominal_connection_size_mm
        && attributes.nominal_connection_size_mm != Some(nominal)
    {
        return false;
    }
    if let Some(range) = &applicability.measured_outer_diameter_mm {
        let Some(measurements) = measurements else { return false };
        if !range.contains(measurements.outer_diameter_mm) {
            return false;
        }
    }
    if let Some(range) = &applicability.measured_wall_thickness_mm {
        let Some(measurements) = measurements else { return false };
        let Some(wall) = measurements.wall_thickness_mm() else { return false };
        if !range.contains(wall) {
            return false;
        }
    }
    if let Some(component_id) = applicability.connector_component_id {
        let Some(connector) = connector else { return false };
        if connector.catalog_component_id != component_id {
            return false;
        }
    }
    true
}

/// The usable limit of one applicable rule. A limit that needs a measurement to become a number is
/// resolved here, exactly once; no rule is interpolated.
fn resolved_limit(kind: FeasibilityRuleKind, rule: &FeasibilityRule, measurements: Option<&FeasibilityTubeMeasurements>) -> Option<RuleLimit> {
    let mut limit = rule.limit.clone();
    if kind == FeasibilityRuleKind::MinimumBendRadius
        && let RuleLimit::MinimumRadiusMultipleOfOuterDiameter { multiple } = &rule.limit
    {
        let outer_diameter = measurements?.outer_diameter_mm;
        limit = RuleLimit::MinimumRadiusMm { radius_mm: multiple.checked_mul(outer_diameter)? };
    }
    let usable = match &limit {
        RuleLimit::MinimumRadiusMm { radius_mm } => *radius_mm > Decimal::ZERO,
        RuleLimit::MinimumLengthMm { length_mm } => !length_mm.is_sign_negative(),
        RuleLimit::CompatibleTubeRange { .. } => true,
        RuleLimit::MinimumRadiusMultipleOfOuterDiameter { .. } => false,
    };
    usable.then_some(limit)
}

/// How the applicable rules resolve to one limit.
enum LimitResolution {
    None,
    Agreed(RuleLimit, Vec<AppliedRule>),
    Conflict(Vec<AppliedRule>),
}

fn resolve_limits(rule_table: &FeasibilityRuleTableVersion, applicable: &[ApplicableRule]) -> LimitResolution {
    let Some(first) = applicable.first() else { return LimitResolution::None };
    let agreed = first.limit.clone();
    if applicable.iter().any(|rule| rule.limit != agreed) {
        // Overlapping applicable rules with different limits: never silently pick one.
        return LimitResolution::Conflict(applicable.iter().map(|rule| applied_rule(rule_table, rule)).collect());
    }
    LimitResolution::Agreed(agreed, applicable.iter().map(|rule| applied_rule(rule_table, rule)).collect())
}

fn applied_rule(rule_table: &FeasibilityRuleTableVersion, rule: &ApplicableRule) -> AppliedRule {
    AppliedRule {
        rule_id: rule.rule.rule_id,
        rule_table_id: rule_table.rule_table_id,
        rule_table_version_id: rule_table.rule_table_version_id,
        rule_table_version: rule_table.version,
        source_reference: rule.rule.source_reference.clone(),
        limit: rule.limit.clone(),
    }
}

/// The minimum radius a `MinimumBendRadius` limit requires.
fn minimum_radius(limit: &RuleLimit) -> Option<Decimal> {
    match limit {
        RuleLimit::MinimumRadiusMm { radius_mm } => Some(*radius_mm),
        _ => None,
    }
}

/// The minimum length a `BendSpacing` or `EndAssemblyLength` limit requires.
fn minimum_length(limit: &RuleLimit) -> Option<Decimal> {
    match limit {
        RuleLimit::MinimumLengthMm { length_mm } => Some(*length_mm),
        _ => None,
    }
}

fn finding_key(component: &FeasibilityComponentEvidence, kind: FeasibilityRuleKind, check: FindingCheck) -> FindingKey {
    FindingKey { component_id: component.component_id, product_revision_id: component.product_revision_id, kind, check }
}

/// One unresolved finding: nothing was invented, and nothing passed.
fn unresolved(key: FindingKey, reason: UnresolvedReason, measurements: Vec<SupportingMeasurement>) -> FeasibilityFinding {
    FeasibilityFinding {
        key,
        state: FindingState::Unresolved,
        unresolved_reason: Some(reason),
        measurements,
        applied_rules: Vec::new(),
        required_limit: None,
    }
}

/// One conflicting finding: every overlapping rule is listed and no limit is chosen.
fn conflict(key: FindingKey, measurements: Vec<SupportingMeasurement>, rules: Vec<AppliedRule>) -> FeasibilityFinding {
    FeasibilityFinding { key, state: FindingState::Conflict, unresolved_reason: None, measurements, applied_rules: rules, required_limit: None }
}

fn same_geometry(left: &[ReviewedGeometry], right: &[ReviewedGeometry]) -> bool {
    let mut left = left.to_vec();
    let mut right = right.to_vec();
    left.sort();
    right.sort();
    left == right
}

fn check_request_id(request_id: &str, field: &str) -> Result<()> {
    check_text(request_id, field)?;
    if request_id.chars().count() > MAX_FEASIBILITY_REQUEST_ID_CHARS {
        return Err(ContractError::invalid(field, "the request id is too long"));
    }
    Ok(())
}

fn check_positive(value: Decimal, field: &str) -> Result<()> {
    if value <= Decimal::ZERO { Err(ContractError::invalid(field, "the value must be positive")) } else { Ok(()) }
}

fn check_non_negative(value: Decimal, field: &str) -> Result<()> {
    if value.is_sign_negative() { Err(ContractError::invalid(field, "the value cannot be negative")) } else { Ok(()) }
}

fn check_optional_positive(value: Option<Decimal>, field: &str) -> Result<()> {
    match value {
        Some(value) => check_positive(value, field),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engineering::ReviewedComponent;
    use crate::types::Unit;

    fn d(text: &str) -> Decimal {
        text.parse().unwrap_or_default()
    }

    fn key() -> AnalysisKey {
        AnalysisKey { source_sha256: "a".repeat(64), engine_build: "0.40.0".into(), extractor_version: "2".into() }
    }

    fn range(min: &str, max: &str) -> NumericRangeMm {
        NumericRangeMm { min_mm: d(min), max_mm: d(max) }
    }

    fn rule(kind: FeasibilityRuleKind, applicability: RuleApplicability, limit: RuleLimit) -> FeasibilityRule {
        FeasibilityRule { rule_id: Uuid::from_u128(1), kind, applicability, limit, source_reference: "ENG06517, 2010-11-05".into(), note: None }
    }

    fn rule_with_id(id: u128, kind: FeasibilityRuleKind, applicability: RuleApplicability, limit: RuleLimit) -> FeasibilityRule {
        FeasibilityRule { rule_id: Uuid::from_u128(id), ..rule(kind, applicability, limit) }
    }

    fn radius_rule_with_id(id: u128) -> FeasibilityRule {
        rule_with_id(
            id,
            FeasibilityRuleKind::MinimumBendRadius,
            RuleApplicability { material_name: Some("PA12".into()), supporting_spring_required: true, ..RuleApplicability::default() },
            RuleLimit::MinimumRadiusMm { radius_mm: d("30") },
        )
    }

    fn radius_rule() -> FeasibilityRule {
        radius_rule_with_id(1)
    }

    fn spacing_rule() -> FeasibilityRule {
        rule_with_id(2, FeasibilityRuleKind::BendSpacing, RuleApplicability::default(), RuleLimit::MinimumLengthMm { length_mm: d("20") })
    }

    fn end_length_rule() -> FeasibilityRule {
        rule_with_id(
            4,
            FeasibilityRuleKind::EndAssemblyLength,
            RuleApplicability { connector_component_id: Some(Uuid::from_u128(0xc0)), ..RuleApplicability::default() },
            RuleLimit::MinimumLengthMm { length_mm: d("25") },
        )
    }

    fn compatibility_rule() -> FeasibilityRule {
        rule_with_id(
            3,
            FeasibilityRuleKind::TubeConnectorCompatibility,
            RuleApplicability { connector_component_id: Some(Uuid::from_u128(0xc0)), ..RuleApplicability::default() },
            RuleLimit::CompatibleTubeRange { outer_diameter_mm: range("8", "10"), inner_diameter_mm: range("5", "8") },
        )
    }

    fn table(rules: Vec<FeasibilityRule>) -> FeasibilityRuleTableVersion {
        FeasibilityRuleTableVersion {
            rule_table_id: Uuid::from_u128(7),
            rule_table_version_id: Uuid::from_u128(8),
            version: 1,
            supersedes: None,
            name: "Design rules".into(),
            rules,
            source_reference: Some("design rules/".into()),
            created_by: Some("administrator".into()),
            created_at: Some("2026-11-10T00:00:00.000Z".into()),
        }
    }

    fn connector() -> FeasibilityConnector {
        FeasibilityConnector { catalog_component_id: Uuid::from_u128(0xc0), catalog_component_version_id: Uuid::from_u128(0xc1) }
    }

    fn ends(connector: Option<FeasibilityConnector>) -> Vec<FeasibilityTubeEnd> {
        vec![FeasibilityTubeEnd { end: TubeEnd::Start, connector }, FeasibilityTubeEnd { end: TubeEnd::End, connector }]
    }

    fn measurements() -> FeasibilityTubeMeasurements {
        FeasibilityTubeMeasurements {
            outer_diameter_mm: d("10"),
            inner_diameter_mm: d("8"),
            bend_radii_mm: vec![d("35"), d("25")],
            tangent_straight_lengths_mm: vec![d("30")],
            straight_end_lengths_mm: vec![d("40"), d("40")],
        }
    }

    fn attributes() -> FeasibilityTubeAttributes {
        FeasibilityTubeAttributes {
            tube_kind: Some(TubeKind::Smooth),
            material_name: Some("PA12".into()),
            supporting_spring: Some(true),
            nominal_outer_diameter_mm: Some(d("10")),
            nominal_wall_thickness_mm: Some(d("1")),
            nominal_connection_size_mm: None,
        }
    }

    fn component_fixture(measurements: Option<FeasibilityTubeMeasurements>, ends: Vec<FeasibilityTubeEnd>) -> FeasibilityComponentEvidence {
        FeasibilityComponentEvidence {
            component_id: Uuid::from_u128(0x100),
            product_revision_id: Uuid::from_u128(0x200),
            analysis_key: key(),
            extraction_version: "1".into(),
            attributes: attributes(),
            measurements,
            ends,
        }
    }

    fn evaluate(rules: Vec<FeasibilityRule>, components: Vec<FeasibilityComponentEvidence>) -> Vec<FeasibilityFinding> {
        evaluate_feasibility(&table(rules), &components).unwrap_or_default()
    }

    fn find(findings: &[FeasibilityFinding], kind: FeasibilityRuleKind, check: FindingCheck) -> Option<&FeasibilityFinding> {
        findings.iter().find(|finding| finding.key.kind == kind && finding.key.check == check)
    }

    fn codes(issues: &[FeasibilityIssue]) -> Vec<&str> {
        issues.iter().map(|issue| issue.code.as_str()).collect()
    }

    fn review_document(geometry: Vec<ReviewedGeometry>) -> EngineeringRecordDocument {
        let mut document = EngineeringRecordDocument::empty(Uuid::from_u128(0x300));
        document.components.push(ReviewedComponent {
            component_id: Uuid::from_u128(0x100),
            product_revision_id: Uuid::from_u128(0x200),
            analysis_key: key(),
            geometry,
            part_version_id: Some(Uuid::from_u128(0x400)),
        });
        document
    }

    fn assessed(engineering: &EngineeringRecordDocument) -> AssessedComponent {
        let component = engineering.components[0].clone();
        AssessedComponent {
            evidence: component_evidence_from_review(&component),
            part_version_id: component.part_version_id,
            reviewed_geometry: component.geometry.clone(),
        }
    }

    fn component_evidence_from_review(component: &ReviewedComponent) -> FeasibilityComponentEvidence {
        FeasibilityComponentEvidence {
            component_id: component.component_id,
            product_revision_id: component.product_revision_id,
            analysis_key: component.analysis_key.clone(),
            extraction_version: "1".into(),
            attributes: attributes(),
            measurements: Some(measurements()),
            ends: ends(Some(connector())),
        }
    }

    fn assessment_fixture(assessed: AssessedComponent, findings: Vec<FeasibilityFinding>) -> FeasibilityAssessment {
        FeasibilityAssessment {
            version: 1,
            rule_table_version_id: Uuid::from_u128(8),
            engineering_record_version: 1,
            created_by: "engineer".into(),
            created_at: "2026-11-10T00:00:00.000Z".into(),
            components: vec![assessed],
            findings,
        }
    }

    fn record(assessment: FeasibilityAssessment, exceptions: Vec<FeasibilityException>) -> FeasibilityRecordDocument {
        FeasibilityRecordDocument {
            schema_version: FEASIBILITY_DOCUMENT_SCHEMA_VERSION,
            project_id: Uuid::from_u128(0x300),
            assessments: vec![assessment],
            exceptions,
        }
    }

    fn exception_for(assessment: &FeasibilityAssessment, component: AssessedComponent, finding: FeasibilityFinding) -> FeasibilityException {
        FeasibilityException {
            exception_id: Uuid::from_u128(0x500),
            finding_key: finding.key.clone(),
            justification: "the customer accepts the tighter bend; reviewed on the bench".into(),
            accepted_by: "engineer".into(),
            accepted_at: "2026-11-10T00:00:00.000Z".into(),
            assessment_version: assessment.version,
            rule_table_version_id: assessment.rule_table_version_id,
            component,
            finding,
        }
    }

    const BODY_ZERO: ReviewedGeometry = ReviewedGeometry { source_path: Vec::new(), source_body: 0 };

    #[test]
    fn a_rule_table_needs_sourced_rules_and_matching_limits() {
        assert!(table(vec![radius_rule()]).validate().is_ok());
        // A rule without its engineering source is an invented default.
        let mut unsourced = radius_rule();
        unsourced.source_reference = " ".into();
        assert!(table(vec![unsourced]).validate().is_err());
        // One rule id per table.
        assert!(table(vec![radius_rule(), radius_rule()]).validate().is_err());
        // A radius rule cannot carry a length limit or a connector condition.
        let wrong_limit =
            rule(FeasibilityRuleKind::MinimumBendRadius, RuleApplicability::default(), RuleLimit::MinimumLengthMm { length_mm: d("20") });
        assert!(table(vec![wrong_limit]).validate().is_err());
        let incorrect_applicability = rule(
            FeasibilityRuleKind::MinimumBendRadius,
            RuleApplicability { connector_component_id: Some(Uuid::from_u128(2)), ..RuleApplicability::default() },
            RuleLimit::MinimumRadiusMm { radius_mm: d("20") },
        );
        assert!(table(vec![incorrect_applicability]).validate().is_err());
        // A rule limit must be usable.
        let zero_radius =
            rule(FeasibilityRuleKind::MinimumBendRadius, RuleApplicability::default(), RuleLimit::MinimumRadiusMm { radius_mm: d("0") });
        assert!(table(vec![zero_radius]).validate().is_err());
        let unordered = rule(
            FeasibilityRuleKind::TubeConnectorCompatibility,
            RuleApplicability::default(),
            RuleLimit::CompatibleTubeRange { outer_diameter_mm: range("10", "8"), inner_diameter_mm: range("5", "8") },
        );
        assert!(table(vec![unordered]).validate().is_err());
        let mut newer = table(vec![radius_rule()]);
        newer.version = 0;
        assert!(newer.validate().is_err());
    }

    #[test]
    fn a_publish_request_is_validated_before_the_service_writes_it() {
        let request = CreateFeasibilityRuleTableVersion {
            request_id: "rules-1".into(),
            expected_version: 0,
            rule_table_id: Uuid::from_u128(7),
            name: "Design rules".into(),
            rules: vec![radius_rule()],
            source_reference: Some("design rules/".into()),
        };
        assert!(request.validate().is_ok());
        assert!(CreateFeasibilityRuleTableVersion { request_id: " ".into(), ..request.clone() }.validate().is_err());
        assert!(CreateFeasibilityRuleTableVersion { rules: Vec::new(), ..request.clone() }.validate().is_ok(), "an empty table is explicit data");
        assert!(CreateFeasibilityRuleTableVersion { rules: vec![radius_rule(), radius_rule()], ..request }.validate().is_err());
    }

    #[test]
    fn every_bend_radius_is_tested_independently() {
        let findings = evaluate(vec![radius_rule()], vec![component_fixture(Some(measurements()), ends(None))]);
        // Two bends, two findings, each with its own index and measured value.
        let first =
            find(&findings, FeasibilityRuleKind::MinimumBendRadius, FindingCheck::BendRadius { bend_index: 0 }).expect("the first bend finding");
        assert_eq!(first.state, FindingState::Pass);
        assert_eq!(first.measurements[0].value_mm, d("35"));
        let second =
            find(&findings, FeasibilityRuleKind::MinimumBendRadius, FindingCheck::BendRadius { bend_index: 1 }).expect("the second bend finding");
        assert_eq!(second.state, FindingState::Fail, "25 mm is below the 30 mm limit even though the first bend passes");
        assert_eq!(second.measurements[0].value_mm, d("25"));
        assert_eq!(second.required_limit, Some(RuleLimit::MinimumRadiusMm { radius_mm: d("30") }));
        assert_eq!(second.applied_rules.len(), 1);
    }

    #[test]
    fn bend_spacing_is_measured_tangent_to_tangent_and_compared_with_its_own_finding() {
        let findings = evaluate(vec![spacing_rule()], vec![component_fixture(Some(measurements()), ends(None))]);
        let spacing =
            find(&findings, FeasibilityRuleKind::BendSpacing, FindingCheck::BendSpacing { first_bend_index: 0 }).expect("the spacing finding");
        assert_eq!(spacing.state, FindingState::Pass);
        assert_eq!(spacing.measurements[0].field_path, "tangent_straight_lengths_mm[0]");
        assert_eq!(spacing.measurements[0].value_mm, d("30"));

        // A measured spacing below the source limit fails; there is no chord or nominal substitute.
        let mut short = measurements();
        short.tangent_straight_lengths_mm = vec![d("12")];
        let findings = evaluate(vec![spacing_rule()], vec![component_fixture(Some(short), ends(None))]);
        let spacing =
            find(&findings, FeasibilityRuleKind::BendSpacing, FindingCheck::BendSpacing { first_bend_index: 0 }).expect("the spacing finding");
        assert_eq!(spacing.state, FindingState::Fail);
    }

    #[test]
    fn an_inapplicable_material_or_dimension_limit_never_passes() {
        // The supplied smooth-tube table needs a standard PA12 material and a supporting spring.
        let mut wrong_material = attributes();
        wrong_material.material_name = Some("PA6".into());
        let findings = evaluate(vec![radius_rule()], vec![component_fixture(Some(measurements()), ends(None)).with_attributes(wrong_material)]);
        let bend = find(&findings, FeasibilityRuleKind::MinimumBendRadius, FindingCheck::BendRadius { bend_index: 0 }).expect("the bend finding");
        assert_eq!(bend.state, FindingState::Unresolved);
        assert_eq!(bend.unresolved_reason, Some(UnresolvedReason::NoApplicableLimit));
        assert!(bend.applied_rules.is_empty());

        // The review not proving the material cannot become a pass either.
        let mut unknown = attributes();
        unknown.material_name = None;
        unknown.supporting_spring = None;
        let findings = evaluate(vec![radius_rule()], vec![component_fixture(Some(measurements()), ends(None)).with_attributes(unknown)]);
        let bend = find(&findings, FeasibilityRuleKind::MinimumBendRadius, FindingCheck::BendRadius { bend_index: 0 }).expect("the bend finding");
        assert_eq!(bend.state, FindingState::Unresolved);

        // A measured-range rule that does not cover this tube is inapplicable, not failing and not
        // passing.
        let narrow = rule(
            FeasibilityRuleKind::BendSpacing,
            RuleApplicability { measured_outer_diameter_mm: Some(range("4", "6")), ..RuleApplicability::default() },
            RuleLimit::MinimumLengthMm { length_mm: d("20") },
        );
        let findings = evaluate(vec![narrow], vec![component_fixture(Some(measurements()), ends(None))]);
        let spacing =
            find(&findings, FeasibilityRuleKind::BendSpacing, FindingCheck::BendSpacing { first_bend_index: 0 }).expect("the spacing finding");
        assert_eq!(spacing.state, FindingState::Unresolved);
    }

    trait WithAttributes {
        fn with_attributes(self, attributes: FeasibilityTubeAttributes) -> Self;
    }

    impl WithAttributes for FeasibilityComponentEvidence {
        fn with_attributes(mut self, attributes: FeasibilityTubeAttributes) -> Self {
            self.attributes = attributes;
            self
        }
    }

    #[test]
    fn a_minimum_radius_multiple_of_the_outer_diameter_resolves_to_a_radius() {
        let multiple = rule(
            FeasibilityRuleKind::MinimumBendRadius,
            RuleApplicability::default(),
            RuleLimit::MinimumRadiusMultipleOfOuterDiameter { multiple: d("3.5") },
        );
        let findings = evaluate(vec![multiple], vec![component_fixture(Some(measurements()), ends(None))]);
        let first =
            find(&findings, FeasibilityRuleKind::MinimumBendRadius, FindingCheck::BendRadius { bend_index: 0 }).expect("the first bend finding");
        assert_eq!(first.state, FindingState::Pass, "3.5 x 10 mm = 35 mm");
        assert_eq!(first.required_limit, Some(RuleLimit::MinimumRadiusMm { radius_mm: d("35") }));
        let second =
            find(&findings, FeasibilityRuleKind::MinimumBendRadius, FindingCheck::BendRadius { bend_index: 1 }).expect("the second bend finding");
        assert_eq!(second.state, FindingState::Fail, "25 mm is below the resolved 35 mm");
    }

    #[test]
    fn missing_measurements_are_unresolved_never_passed() {
        let findings =
            evaluate(vec![radius_rule(), spacing_rule(), compatibility_rule(), end_length_rule()], vec![component_fixture(None, Vec::new())]);
        assert!(!findings.is_empty());
        for finding in &findings {
            assert_eq!(finding.state, FindingState::Unresolved);
            assert_eq!(finding.unresolved_reason, Some(UnresolvedReason::MissingMeasurement));
        }
        // One whole-tube finding per kept check kind; no bend count is invented.
        for kind in [
            FeasibilityRuleKind::MinimumBendRadius,
            FeasibilityRuleKind::BendSpacing,
            FeasibilityRuleKind::TubeConnectorCompatibility,
            FeasibilityRuleKind::EndAssemblyLength,
        ] {
            assert!(find(&findings, kind, FindingCheck::WholeTube).is_some(), "{kind:?}");
        }
    }

    #[test]
    fn missing_connector_data_is_unresolved_never_passed() {
        let findings = evaluate(vec![compatibility_rule(), end_length_rule()], vec![component_fixture(Some(measurements()), ends(None))]);
        let compatibility =
            find(&findings, FeasibilityRuleKind::TubeConnectorCompatibility, FindingCheck::ConnectorCompatibility { end: TubeEnd::Start })
                .expect("the start compatibility finding");
        assert_eq!(compatibility.state, FindingState::Unresolved);
        assert_eq!(compatibility.unresolved_reason, Some(UnresolvedReason::MissingConnectorAssociation));
        let length = find(&findings, FeasibilityRuleKind::EndAssemblyLength, FindingCheck::EndAssemblyLength { end: TubeEnd::End })
            .expect("the end length finding");
        assert_eq!(length.state, FindingState::Unresolved);
        assert_eq!(length.unresolved_reason, Some(UnresolvedReason::MissingConnectorAssociation));
    }

    #[test]
    fn a_connector_compatibility_check_compares_both_measured_diameters() {
        let findings = evaluate(vec![compatibility_rule()], vec![component_fixture(Some(measurements()), ends(Some(connector())))]);
        let compatibility =
            find(&findings, FeasibilityRuleKind::TubeConnectorCompatibility, FindingCheck::ConnectorCompatibility { end: TubeEnd::Start })
                .expect("the compatibility finding");
        assert_eq!(compatibility.state, FindingState::Pass);
        assert_eq!(compatibility.measurements.len(), 2);

        let mut too_thick = measurements();
        too_thick.outer_diameter_mm = d("12");
        too_thick.inner_diameter_mm = d("9");
        let findings = evaluate(vec![compatibility_rule()], vec![component_fixture(Some(too_thick), ends(Some(connector())))]);
        let compatibility =
            find(&findings, FeasibilityRuleKind::TubeConnectorCompatibility, FindingCheck::ConnectorCompatibility { end: TubeEnd::Start })
                .expect("the compatibility finding");
        assert_eq!(compatibility.state, FindingState::Fail);
    }

    #[test]
    fn an_end_assembly_length_check_uses_each_end_straight_length() {
        let findings = evaluate(vec![end_length_rule()], vec![component_fixture(Some(measurements()), ends(Some(connector())))]);
        let start = find(&findings, FeasibilityRuleKind::EndAssemblyLength, FindingCheck::EndAssemblyLength { end: TubeEnd::Start })
            .expect("the start end finding");
        assert_eq!(start.state, FindingState::Pass);
        assert_eq!(start.measurements[0].value_mm, d("40"));

        let mut short = measurements();
        short.straight_end_lengths_mm = vec![d("40"), d("10")];
        let findings = evaluate(vec![end_length_rule()], vec![component_fixture(Some(short), ends(Some(connector())))]);
        let end = find(&findings, FeasibilityRuleKind::EndAssemblyLength, FindingCheck::EndAssemblyLength { end: TubeEnd::End })
            .expect("the end length finding");
        assert_eq!(end.state, FindingState::Fail);
        assert_eq!(end.measurements[0].value_mm, d("10"));
    }

    #[test]
    fn overlapping_limits_conflict_instead_of_silently_choosing_one() {
        let mut second_rule = radius_rule();
        second_rule.rule_id = Uuid::from_u128(2);
        second_rule.limit = RuleLimit::MinimumRadiusMm { radius_mm: d("20") };
        let findings = evaluate(vec![radius_rule(), second_rule], vec![component_fixture(Some(measurements()), ends(None))]);
        let first =
            find(&findings, FeasibilityRuleKind::MinimumBendRadius, FindingCheck::BendRadius { bend_index: 0 }).expect("the first bend finding");
        assert_eq!(first.state, FindingState::Conflict);
        assert_eq!(first.unresolved_reason, None);
        assert_eq!(first.required_limit, None, "a conflict has no chosen limit");
        assert_eq!(first.applied_rules.len(), 2);
        assert!(first.exception_is_available(), "the justified-exception route remains available");

        // Two applicable rules that agree are one agreed limit.
        let mut same = radius_rule();
        same.rule_id = Uuid::from_u128(3);
        let findings = evaluate(vec![radius_rule(), same], vec![component_fixture(Some(measurements()), ends(None))]);
        let first =
            find(&findings, FeasibilityRuleKind::MinimumBendRadius, FindingCheck::BendRadius { bend_index: 0 }).expect("the first bend finding");
        assert_eq!(first.state, FindingState::Pass);
        assert_eq!(first.applied_rules.len(), 2);
    }

    #[test]
    fn a_finding_validates_its_state_and_exception_availability() {
        let findings = evaluate(vec![radius_rule()], vec![component_fixture(Some(measurements()), ends(None))]);
        for finding in &findings {
            assert!(finding.validate("finding").is_ok());
        }
        let failing = find(&findings, FeasibilityRuleKind::MinimumBendRadius, FindingCheck::BendRadius { bend_index: 1 }).expect("the failing bend");
        assert!(failing.exception_is_available());
        let passing = find(&findings, FeasibilityRuleKind::MinimumBendRadius, FindingCheck::BendRadius { bend_index: 0 }).expect("the passing bend");
        assert!(!passing.exception_is_available());
        let mut missing_reason = failing.clone();
        missing_reason.unresolved_reason = None;
        missing_reason.state = FindingState::Unresolved;
        assert!(missing_reason.validate("finding").is_err());
        let mut chosen = failing.clone();
        chosen.state = FindingState::Conflict;
        chosen.required_limit = Some(RuleLimit::MinimumRadiusMm { radius_mm: d("30") });
        assert!(chosen.validate("finding").is_err());
    }

    #[test]
    fn a_finding_key_must_match_its_check() {
        let key = FindingKey {
            component_id: Uuid::from_u128(1),
            product_revision_id: Uuid::from_u128(2),
            kind: FeasibilityRuleKind::MinimumBendRadius,
            check: FindingCheck::EndAssemblyLength { end: TubeEnd::Start },
        };
        assert!(key.validate().is_err(), "a radius finding cannot name an end-length check");
    }

    #[test]
    fn a_straight_tube_has_no_bend_or_spacing_findings() {
        let mut straight = measurements();
        straight.bend_radii_mm.clear();
        straight.tangent_straight_lengths_mm.clear();
        straight.straight_end_lengths_mm = vec![d("100"), d("100")];
        let findings = evaluate(vec![radius_rule(), spacing_rule()], vec![component_fixture(Some(straight), ends(None))]);
        assert!(findings.iter().all(|finding| finding.key.kind != FeasibilityRuleKind::MinimumBendRadius));
        assert!(findings.iter().all(|finding| finding.key.kind != FeasibilityRuleKind::BendSpacing));
    }

    #[test]
    fn evidence_validation_refuses_inconsistent_tube_measurements() {
        let mut bad_count = measurements();
        bad_count.tangent_straight_lengths_mm.clear();
        assert!(bad_count.validate("measurements").is_err());
        let mut no_ends = component_fixture(Some(measurements()), Vec::new());
        no_ends.measurements = Some(measurements());
        assert!(no_ends.validate("component").is_err());
        let mut end_without_measurements = component_fixture(None, ends(None));
        end_without_measurements.ends = ends(None);
        assert!(end_without_measurements.validate("component").is_err());
        let mut unordered = component_fixture(Some(measurements()), ends(None));
        unordered.ends.reverse();
        assert!(unordered.validate("component").is_err());
        let mut negative = measurements();
        negative.straight_end_lengths_mm = vec![d("-1"), d("-1")];
        assert!(negative.validate("measurements").is_err());
        assert!(component_fixture(Some(measurements()), ends(Some(connector()))).validate("component").is_ok());
    }

    #[test]
    fn an_assessment_request_names_its_exact_components_and_rule_version() {
        let request = RecordFeasibilityAssessment {
            request_id: "assessment-1".into(),
            expected_version: 0,
            rule_table_version_id: Uuid::from_u128(8),
            components: vec![component_fixture(Some(measurements()), ends(Some(connector())))],
        };
        assert!(request.validate().is_ok());
        assert!(RecordFeasibilityAssessment { components: Vec::new(), ..request.clone() }.validate().is_err());
        assert!(
            RecordFeasibilityAssessment { components: vec![request.components[0].clone(), request.components[0].clone()], ..request.clone() }
                .validate()
                .is_err()
        );
        assert!(RecordFeasibilityAssessment { request_id: " ".into(), ..request }.validate().is_err());
    }

    #[test]
    fn an_assessed_component_detects_a_changed_review() {
        let engineering = review_document(vec![BODY_ZERO]);
        let assessed = assessed(&engineering);
        assert!(assessed.matches_review(&engineering));

        // A changed body set is changed supporting geometry.
        let regrouped = review_document(vec![ReviewedGeometry { source_path: Vec::new(), source_body: 1 }]);
        assert!(!assessed.matches_review(&regrouped));
        // The same bodies in another order are the same geometry.
        let two_bodies = review_document(vec![BODY_ZERO, ReviewedGeometry { source_path: Vec::new(), source_body: 1 }]);
        let mut also_two = assessed.clone();
        also_two.reviewed_geometry = vec![ReviewedGeometry { source_path: Vec::new(), source_body: 1 }, BODY_ZERO];
        assert!(also_two.matches_review(&two_bodies));
        // A changed part association is a changed binding.
        let mut changed_part = review_document(vec![BODY_ZERO]);
        changed_part.components[0].part_version_id = Some(Uuid::from_u128(0x401));
        assert!(!assessed.matches_review(&changed_part));
        // A missing component no longer supports the assessment.
        let mut removed = review_document(vec![BODY_ZERO]);
        removed.components.clear();
        assert!(!assessed.matches_review(&removed));
    }

    #[test]
    fn an_active_exception_resolves_the_disposition_but_keeps_the_geometric_failure() {
        let engineering = review_document(vec![BODY_ZERO]);
        // Every kept check has its applicable rule and its exact evidence, so only the tight bend
        // is a non-passing finding.
        let findings = evaluate(
            vec![radius_rule(), spacing_rule(), compatibility_rule(), end_length_rule()],
            vec![component_fixture(Some(measurements()), ends(Some(connector())))],
        );
        let assessed = assessed(&engineering);
        let assessment = assessment_fixture(assessed.clone(), findings.clone());
        let failing =
            find(&findings, FeasibilityRuleKind::MinimumBendRadius, FindingCheck::BendRadius { bend_index: 1 }).expect("the failing bend").clone();
        let exception = exception_for(&assessment, assessed, failing);
        let document = record(assessment.clone(), vec![exception.clone()]);
        assert!(document.validate().is_ok());

        let view = document.record_view(1, &engineering);
        assert_eq!(view.exceptions.len(), 1);
        assert_eq!(view.exceptions[0].state, ExceptionState::Active);
        assert!(view.issues.is_empty(), "{:?}", view.issues);
        let latest = view.latest_assessment.expect("the latest assessment");
        let failing_view =
            latest.findings.iter().find(|view| view.finding.key.check == FindingCheck::BendRadius { bend_index: 1 }).expect("the failing view");
        assert_eq!(failing_view.disposition, FindingDisposition::ExceptionAccepted { exception_id: exception.exception_id });
        // The geometric state stays visible; the exception only changes the disposition.
        assert_eq!(failing_view.finding.state, FindingState::Fail);
    }

    #[test]
    fn an_exception_asks_for_review_when_its_supporting_geometry_changes() {
        let engineering = review_document(vec![BODY_ZERO]);
        let findings = evaluate(vec![radius_rule()], vec![component_fixture(Some(measurements()), ends(None))]);
        let assessed = assessed(&engineering);
        let assessment = assessment_fixture(assessed.clone(), findings.clone());
        let failing =
            find(&findings, FeasibilityRuleKind::MinimumBendRadius, FindingCheck::BendRadius { bend_index: 1 }).expect("the failing bend").clone();
        let exception = exception_for(&assessment, assessed, failing);
        let document = record(assessment, vec![exception]);

        let regrouped = review_document(vec![ReviewedGeometry { source_path: Vec::new(), source_body: 1 }]);
        let view = document.record_view(1, &regrouped);
        assert_eq!(view.exceptions[0].state, ExceptionState::ReviewRequired);
        assert_eq!(view.exceptions[0].review_required_reasons, vec![ExceptionInvalidationReason::EngineeringReviewChanged]);
        assert!(codes(&view.issues).contains(&"exception_review_required"), "{:?}", view.issues);
        assert!(codes(&view.issues).contains(&"assessment_stale"), "{:?}", view.issues);
    }

    #[test]
    fn an_exception_asks_for_review_when_the_current_assessment_changes() {
        let engineering = review_document(vec![BODY_ZERO]);
        let findings = evaluate(vec![radius_rule()], vec![component_fixture(Some(measurements()), ends(None))]);
        let assessed = assessed(&engineering);
        let assessment = assessment_fixture(assessed.clone(), findings.clone());
        let failing =
            find(&findings, FeasibilityRuleKind::MinimumBendRadius, FindingCheck::BendRadius { bend_index: 1 }).expect("the failing bend").clone();
        let exception = exception_for(&assessment, assessed.clone(), failing);
        let mut document = record(assessment.clone(), vec![exception]);

        // A newer assessment of the same evidence and rule version keeps the exception active.
        let mut again = assessment.clone();
        again.version = 2;
        let mut unchanged = record(again.clone(), Vec::new());
        unchanged.assessments.push(again.clone());
        unchanged.exceptions = document.exceptions.clone();
        let view = unchanged.record_view(2, &engineering);
        assert_eq!(view.exceptions[0].state, ExceptionState::Active);

        // A newer rule version is a different assessment: the accepted limit changed.
        let mut changed = again.clone();
        changed.version = 3;
        changed.rule_table_version_id = Uuid::from_u128(9);
        document.assessments.push(changed);
        document.exceptions = unchanged.exceptions;
        let view = document.record_view(3, &engineering);
        assert_eq!(view.exceptions[0].state, ExceptionState::ReviewRequired);
        assert_eq!(view.exceptions[0].review_required_reasons, vec![ExceptionInvalidationReason::AssessmentChanged]);
    }

    #[test]
    fn a_missing_assessment_is_a_readiness_issue() {
        let document = FeasibilityRecordDocument::empty(Uuid::from_u128(0x300));
        assert!(document.validate().is_ok());
        let engineering = review_document(vec![BODY_ZERO]);
        assert_eq!(codes(&document.issues(&engineering)), vec!["feasibility_not_assessed"]);
    }

    #[test]
    fn an_open_finding_is_a_readiness_issue_and_a_missing_limit_stays_visible() {
        let engineering = review_document(vec![BODY_ZERO]);
        // No rule applies: the check is unresolved for a missing limit, not passed.
        let findings = evaluate(Vec::new(), vec![component_fixture(Some(measurements()), ends(Some(connector())))]);
        assert!(findings.iter().any(|finding| finding.unresolved_reason == Some(UnresolvedReason::NoApplicableLimit)));
        let assessed = assessed(&engineering);
        let document = record(assessment_fixture(assessed, findings), Vec::new());
        let view = document.record_view(1, &engineering);
        let issue_codes = codes(&view.issues);
        assert!(issue_codes.contains(&"open_finding_unresolved"), "{issue_codes:?}");
        let latest = view.latest_assessment.expect("the assessment");
        assert!(latest.findings.iter().all(|finding| finding.disposition == FindingDisposition::Open));
    }

    #[test]
    fn an_exception_cannot_cover_a_pass_or_a_missing_measurement() {
        let engineering = review_document(vec![BODY_ZERO]);
        let passing_findings = evaluate(vec![radius_rule()], vec![component_fixture(Some(measurements()), ends(None))]);
        let assessed = assessed(&engineering);
        let assessment = assessment_fixture(assessed.clone(), passing_findings.clone());
        let passing = find(&passing_findings, FeasibilityRuleKind::MinimumBendRadius, FindingCheck::BendRadius { bend_index: 0 })
            .expect("the passing bend")
            .clone();
        let mut exception = exception_for(&assessment, assessed.clone(), passing);
        assert!(exception.validate().is_err(), "a pass needs no exception");

        let missing_findings = evaluate(vec![radius_rule(), spacing_rule()], vec![component_fixture(None, Vec::new())]);
        let assessment = assessment_fixture(assessed, missing_findings.clone());
        let missing = missing_findings.first().expect("a missing finding").clone();
        exception = exception_for(&assessment, assessment.components[0].clone(), missing);
        assert!(exception.validate().is_err(), "a missing measurement is review work, not an exception decision");
    }

    #[test]
    fn a_record_refuses_duplicate_keys_and_unknown_assessments() {
        let engineering = review_document(vec![BODY_ZERO]);
        let findings = evaluate(vec![radius_rule()], vec![component_fixture(Some(measurements()), ends(None))]);
        let assessed = assessed(&engineering);
        let assessment = assessment_fixture(assessed.clone(), findings.clone());
        let failing =
            find(&findings, FeasibilityRuleKind::MinimumBendRadius, FindingCheck::BendRadius { bend_index: 1 }).expect("the failing bend").clone();
        let exception = exception_for(&assessment, assessed, failing);
        let mut document = record(assessment, vec![exception.clone(), exception.clone()]);
        assert!(document.validate().is_err(), "one exception per finding key");
        document.exceptions.truncate(1);
        assert!(document.validate().is_ok());
        let mut unknown = document.clone();
        unknown.exceptions[0].assessment_version = 99;
        assert!(unknown.validate().is_err());
        let mut newer_schema = document.clone();
        newer_schema.schema_version = FEASIBILITY_DOCUMENT_SCHEMA_VERSION + 1;
        assert!(matches!(newer_schema.validate(), Err(ContractError::SchemaVersion { .. })));
        let mut duplicate_assessments = document.clone();
        duplicate_assessments.assessments.push(duplicate_assessments.assessments[0].clone());
        assert!(duplicate_assessments.validate().is_err());
    }

    #[test]
    fn the_rules_view_refuses_duplicate_versions() {
        let identity = Identity { identity_id: Uuid::from_u128(1), name: "engineer".into(), permissions: vec![crate::Permission::Engineer] };
        let view = FeasibilityRulesView {
            identity: identity.clone(),
            rule_tables: vec![table(vec![radius_rule()]), table(vec![radius_rule()])],
            changes: Vec::new(),
        };
        assert!(view.validate().is_err());
        let table = table(vec![radius_rule()]);
        let mut distinct = table.clone();
        distinct.rule_table_version_id = Uuid::from_u128(9);
        distinct.version = 2;
        distinct.supersedes = Some(table.rule_table_version_id);
        let view = FeasibilityRulesView {
            identity,
            rule_tables: vec![table, distinct],
            changes: vec![FeasibilityRuleChange {
                rule_table_id: Uuid::from_u128(7),
                rule_table_version_id: Uuid::from_u128(9),
                version: 2,
                previous_version_id: None,
                summary: "version 2".into(),
                changed_by: "administrator".into(),
                changed_at: "2026-11-10T00:00:00.000Z".into(),
            }],
        };
        assert!(view.validate().is_ok());
    }

    #[test]
    fn the_rule_and_finding_wire_values_are_decimal_strings_and_null() {
        let value = serde_json::to_value(table(vec![radius_rule()])).unwrap_or_default();
        assert_eq!(value["rules"][0]["limit"]["radius_mm"], serde_json::json!("30"));
        assert_eq!(value["rules"][0]["applicability"]["material_name"], serde_json::json!("PA12"));
        let component = component_fixture(Some(measurements()), ends(None));
        let value = serde_json::to_value(&component).unwrap_or_default();
        assert_eq!(value["measurements"]["outer_diameter_mm"], serde_json::json!("10"));
        let empty = component_fixture(None, Vec::new());
        let value = serde_json::to_value(&empty).unwrap_or_default();
        assert_eq!(value["measurements"], serde_json::Value::Null);
        assert_eq!(serde_json::from_value::<FeasibilityComponentEvidence>(value).unwrap_or(empty.clone()), empty);

        let findings = evaluate(vec![radius_rule()], vec![component]);
        assert!(findings.iter().all(|finding| finding.validate("wire").is_ok()));
        let round_trip: Vec<FeasibilityFinding> = serde_json::from_value(serde_json::to_value(&findings).unwrap_or_default()).unwrap_or_default();
        assert_eq!(round_trip, findings);
    }

    #[test]
    fn a_unit_is_not_needed_and_geometry_is_millimetres() {
        // A finding's supporting measurement is an exact millimetre value; the contract carries no
        // unit field for it, so a length is never re-interpreted through a display unit.
        let findings = evaluate(vec![spacing_rule()], vec![component_fixture(Some(measurements()), ends(None))]);
        let spacing =
            find(&findings, FeasibilityRuleKind::BendSpacing, FindingCheck::BendSpacing { first_bend_index: 0 }).expect("the spacing finding");
        assert_eq!(spacing.measurements[0].value_mm, d("30"));
        assert_eq!(Unit::MillimetreCubed, Unit::MillimetreCubed);
    }

    #[test]
    fn a_corrugated_rule_is_keyed_by_the_reviewed_nominal_connection_size() {
        // The corrugated source keys its radius table by the connection-area inside diameter, not by
        // the generic outer diameter.
        let corrugated = rule(
            FeasibilityRuleKind::MinimumBendRadius,
            RuleApplicability { tube_kind: Some(TubeKind::Corrugated), nominal_connection_size_mm: Some(d("10")), ..RuleApplicability::default() },
            RuleLimit::MinimumRadiusMm { radius_mm: d("25") },
        );
        let mut corrugated_attributes = FeasibilityTubeAttributes {
            tube_kind: Some(TubeKind::Corrugated),
            nominal_connection_size_mm: Some(d("10")),
            ..FeasibilityTubeAttributes::default()
        };
        let mut measured = measurements();
        measured.bend_radii_mm = vec![d("20"), d("30")];
        let findings = evaluate(
            vec![corrugated.clone()],
            vec![component_fixture(Some(measured.clone()), ends(None)).with_attributes(corrugated_attributes.clone())],
        );
        let first =
            find(&findings, FeasibilityRuleKind::MinimumBendRadius, FindingCheck::BendRadius { bend_index: 0 }).expect("the first bend finding");
        assert_eq!(first.state, FindingState::Fail, "20 mm is below the 25 mm corrugated limit");
        let second =
            find(&findings, FeasibilityRuleKind::MinimumBendRadius, FindingCheck::BendRadius { bend_index: 1 }).expect("the second bend finding");
        assert_eq!(second.state, FindingState::Pass);

        // Without the reviewed nominal connection size the rule cannot apply; the check stays
        // unresolved instead of guessing a size from the outer diameter.
        corrugated_attributes.nominal_connection_size_mm = None;
        let findings = evaluate(vec![corrugated], vec![component_fixture(Some(measured), ends(None)).with_attributes(corrugated_attributes)]);
        let first =
            find(&findings, FeasibilityRuleKind::MinimumBendRadius, FindingCheck::BendRadius { bend_index: 0 }).expect("the first bend finding");
        assert_eq!(first.state, FindingState::Unresolved);
        assert_eq!(first.unresolved_reason, Some(UnresolvedReason::NoApplicableLimit));
    }

    #[test]
    fn a_hostile_assessment_is_bounded_before_it_allocates_findings() {
        // Every rule applies to every bend, so a large rule table and a many-bend tube would ask for
        // far more rule-finding evaluations than the contract accepts.
        let rules: Vec<FeasibilityRule> = (1u128..=1_000)
            .map(|id| {
                rule_with_id(id, FeasibilityRuleKind::BendSpacing, RuleApplicability::default(), RuleLimit::MinimumLengthMm { length_mm: d("20") })
            })
            .collect();
        let mut many_bends = measurements();
        many_bends.bend_radii_mm = vec![d("35"); 2_000];
        many_bends.tangent_straight_lengths_mm = vec![d("30"); 1_999];
        let result = evaluate_feasibility(&table(rules), &[component_fixture(Some(many_bends), ends(None))]);
        assert!(matches!(result, Err(ContractError::Invalid { .. })), "the work bound refuses the assessment before allocating");
    }
}
