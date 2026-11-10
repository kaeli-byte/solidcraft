//! The wire types the desktop app, the calculator and the service share.
//!
//! Identity is a UUID in a string, money and measurements are decimal strings, and a missing
//! value is `null` — never zero. A value here describes one exact source and analysis: editing
//! the source produces a new [`AnalysisKey`], and old measurements must not be attached to it.

use std::collections::{BTreeMap, BTreeSet};

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{ContractError, Result, check_sha256};
use crate::shared_data::{AppConfigVersion, CostCenterPurpose};

/// The contract version this build reads and writes.
pub const SCHEMA_VERSION: u32 = 1;
/// The deepest occurrence path the importer keeps (`kernel::step_in` caps it at 32).
pub const MAX_SOURCE_PATH_DEPTH: usize = 32;
/// Caps on the vectors a hostile draft may carry, before anything is allocated per item.
pub const MAX_DEFINITIONS: usize = 10_000;
pub const MAX_OCCURRENCES: usize = 100_000;
pub const MAX_GEOMETRY_BINDINGS: usize = 100_000;
pub const MAX_PARTS: usize = 10_000;
pub const MAX_ANALYSIS_ISSUES: usize = 10_000;
/// The longest catalog attribute key, matching `^[a-z][a-z0-9_]{0,63}$`.
pub const MAX_ATTRIBUTE_KEY_CHARS: usize = 64;
/// The longest catalog attribute value.
pub const MAX_ATTRIBUTE_VALUE_CHARS: usize = 255;
/// The most declared matching attributes one catalog component version may carry.
pub const MAX_CATALOG_ATTRIBUTES: usize = 32;

/// A stored original file: its identity, digest, size and the name the user gave it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceRef {
    pub source_id: Uuid,
    pub sha256: String,
    pub byte_len: u64,
    pub original_name: String,
}

impl SourceRef {
    pub fn validate(&self) -> Result<()> {
        check_sha256(&self.sha256, "source.sha256")?;
        if self.byte_len == 0 {
            return Err(ContractError::invalid("source.byte_len", "an empty file is not a source"));
        }
        if self.original_name.trim().is_empty() {
            return Err(ContractError::invalid("source.original_name", "a source needs the name it was imported as"));
        }
        Ok(())
    }
}

/// The exact source and extractor that produced an analysis.
///
/// `engine_build` is the engine crate version; bump `extractor_version` whenever extraction
/// changes, so saved measurements are never silently attached to a different algorithm.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct AnalysisKey {
    pub source_sha256: String,
    pub engine_build: String,
    pub extractor_version: String,
}

impl AnalysisKey {
    pub fn validate(&self) -> Result<()> {
        check_sha256(&self.source_sha256, "analysis.source_sha256")?;
        check_text(&self.engine_build, "analysis.engine_build")?;
        check_text(&self.extractor_version, "analysis.extractor_version")
    }
}

/// One shared part definition in an analysis.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DefinitionRef {
    pub definition_id: Uuid,
    pub source_key: AnalysisKey,
    pub customer_part_number: Option<String>,
    pub customer_revision: Option<String>,
    /// The extracted volume of this definition's own closed bodies. `None` when it has no
    /// enclosed volume (an open surface); pending until an engineer reviews it.
    pub volume_mm3: Measurement,
}

/// One placement of a definition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OccurrenceRef {
    pub occurrence_id: Uuid,
    pub definition_id: Uuid,
    pub parent_occurrence_id: Option<Uuid>,
    /// Child indices in the imported tree, not session-local component ids.
    pub source_path: Vec<u64>,
    /// Column-major 4 × 4 placement (`transform[column][row]`), as the engine's `Mat` and the
    /// STEP importer keep it. Converting through a row-major reading would transpose it.
    pub transform: [[f64; 4]; 4],
}

/// A source-local body bound to a definition and occurrence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GeometryBinding {
    pub source_path: Vec<u64>,
    pub source_body: u32,
    pub definition_id: Uuid,
    pub occurrence_id: Uuid,
}

/// The units the prototype's contracts carry.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Unit {
    #[serde(rename = "mm3")]
    MillimetreCubed,
    #[serde(rename = "cm3")]
    CentimetreCubed,
    #[serde(rename = "g_cm3")]
    GramPerCentimetreCubed,
    #[serde(rename = "cny_kg")]
    CurrencyPerKilogram,
    #[serde(rename = "s")]
    Second,
    #[serde(rename = "h")]
    Hour,
    #[serde(rename = "cny_h")]
    CurrencyPerHour,
    #[serde(rename = "cny")]
    Currency,
    #[serde(rename = "cny_piece")]
    CurrencyPerPiece,
}

/// Where a measurement came from.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementSource {
    Extracted,
    Manual,
}

/// Whether an engineer has reviewed a value.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    Pending,
    Confirmed,
}

/// A value with its unit, provenance and review state. Missing is `None`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Measurement {
    pub value: Option<Decimal>,
    pub unit: Unit,
    pub source: MeasurementSource,
    pub review: ReviewState,
    /// Why a manual value was entered; required for manual values.
    pub reason: Option<String>,
    pub analysis_key: AnalysisKey,
}

impl Measurement {
    pub fn validate(&self, field: &str) -> Result<()> {
        if let Some(value) = self.value
            && value.is_sign_negative()
        {
            return Err(ContractError::invalid(format!("{field}.value"), "a measurement cannot be negative"));
        }
        if self.source == MeasurementSource::Manual {
            match self.reason.as_deref() {
                Some(reason) if !reason.trim().is_empty() => {}
                _ => return Err(ContractError::invalid(format!("{field}.reason"), "a manual value needs a reason")),
            }
        }
        self.analysis_key.validate()
    }
}

/// The extracted tree, definitions, placements and geometry bindings for one exact source.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnalysisManifest {
    pub schema_version: u32,
    pub key: AnalysisKey,
    pub definitions: Vec<DefinitionRef>,
    pub occurrences: Vec<OccurrenceRef>,
    pub geometry_bindings: Vec<GeometryBinding>,
    pub issues: Vec<Issue>,
}

impl AnalysisManifest {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ContractError::SchemaVersion { found: self.schema_version, expected: SCHEMA_VERSION });
        }
        self.key.validate()?;
        if self.definitions.len() > MAX_DEFINITIONS {
            return Err(ContractError::invalid("analysis.definitions", "too many definitions"));
        }
        if self.occurrences.len() > MAX_OCCURRENCES {
            return Err(ContractError::invalid("analysis.occurrences", "too many occurrences"));
        }
        if self.geometry_bindings.len() > MAX_GEOMETRY_BINDINGS {
            return Err(ContractError::invalid("analysis.geometry_bindings", "too many bindings"));
        }
        if self.issues.len() > MAX_ANALYSIS_ISSUES {
            return Err(ContractError::invalid("analysis.issues", "too many issues"));
        }

        let mut definition_ids = BTreeSet::new();
        for definition in &self.definitions {
            if !definition_ids.insert(definition.definition_id) {
                return Err(ContractError::invalid("analysis.definitions", "two definitions share one id"));
            }
            if definition.source_key != self.key {
                return Err(ContractError::invalid("analysis.definitions.source_key", "a definition belongs to a different analysis"));
            }
            definition.volume_mm3.validate("analysis.definitions.volume_mm3")?;
            if definition.volume_mm3.unit != Unit::MillimetreCubed {
                return Err(ContractError::invalid("analysis.definitions.volume_mm3.unit", "the reviewed volume is in mm3"));
            }
            if definition.volume_mm3.analysis_key != self.key {
                return Err(ContractError::invalid("analysis.definitions.volume_mm3.analysis_key", "the measurement describes a different analysis"));
            }
        }

        let mut occurrence_ids = BTreeSet::new();
        let mut source_paths = BTreeSet::new();
        for occurrence in &self.occurrences {
            if !occurrence_ids.insert(occurrence.occurrence_id) {
                return Err(ContractError::invalid("analysis.occurrences", "two occurrences share one id"));
            }
            if !definition_ids.contains(&occurrence.definition_id) {
                return Err(ContractError::invalid("analysis.occurrences.definition_id", "no such definition"));
            }
            if occurrence.source_path.len() > MAX_SOURCE_PATH_DEPTH {
                return Err(ContractError::invalid("analysis.occurrences.source_path", "the source path is too deep"));
            }
            if !source_paths.insert((occurrence.definition_id, occurrence.source_path.clone())) {
                return Err(ContractError::invalid("analysis.occurrences.source_path", "two placements share one path"));
            }
            check_transform(&occurrence.transform, "analysis.occurrences.transform")?;
        }
        for occurrence in &self.occurrences {
            if let Some(parent) = occurrence.parent_occurrence_id
                && !occurrence_ids.contains(&parent)
            {
                return Err(ContractError::invalid("analysis.occurrences.parent_occurrence_id", "no such parent occurrence"));
            }
        }

        let mut binding_keys = BTreeSet::new();
        for binding in &self.geometry_bindings {
            if !definition_ids.contains(&binding.definition_id) {
                return Err(ContractError::invalid("analysis.geometry_bindings.definition_id", "no such definition"));
            }
            if !occurrence_ids.contains(&binding.occurrence_id) {
                return Err(ContractError::invalid("analysis.geometry_bindings.occurrence_id", "no such occurrence"));
            }
            if binding.source_path.len() > MAX_SOURCE_PATH_DEPTH {
                return Err(ContractError::invalid("analysis.geometry_bindings.source_path", "the source path is too deep"));
            }
            if !binding_keys.insert((binding.occurrence_id, binding.source_body)) {
                return Err(ContractError::invalid("analysis.geometry_bindings", "two bindings share one body"));
            }
        }
        for issue in &self.issues {
            issue.validate()?;
        }
        Ok(())
    }
}

/// Whether a part is bought or made. The prototype calculates made parts only.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Sourcing {
    Purchased,
    Manufactured,
}

/// Which time a cycle figure describes.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CycleBasis {
    PerPiece,
}

/// One reviewed manufacturing operation. Every number stays optional until entered.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationInputs {
    pub cycle_seconds: Option<Decimal>,
    pub cycle_basis: CycleBasis,
    pub cavities: Option<u32>,
    pub production_rate_version_id: Option<Uuid>,
    pub setup_hours: Option<Decimal>,
    pub setup_rate_version_id: Option<Uuid>,
}

impl OperationInputs {
    pub fn validate(&self) -> Result<()> {
        if let Some(cycle) = self.cycle_seconds
            && cycle.is_sign_negative()
        {
            return Err(ContractError::invalid("operation.cycle_seconds", "a cycle time cannot be negative"));
        }
        if let Some(cavities) = self.cavities
            && cavities == 0
        {
            return Err(ContractError::invalid("operation.cavities", "at least one cavity is needed"));
        }
        if let Some(hours) = self.setup_hours
            && hours.is_sign_negative()
        {
            return Err(ContractError::invalid("operation.setup_hours", "setup time cannot be negative"));
        }
        Ok(())
    }
}

/// The reviewed engineering and manufacturing inputs for one part definition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PartInputs {
    pub part_version_id: Uuid,
    /// The analysis definition this record reviews; its occurrences decide the quantity.
    pub definition_id: Uuid,
    pub classification: String,
    pub classification_confirmed: bool,
    pub sourcing: Sourcing,
    pub volume_mm3: Measurement,
    pub material_version_id: Option<Uuid>,
    pub allowance_rate: Option<Decimal>,
    pub operation: Option<OperationInputs>,
}

impl PartInputs {
    pub fn validate(&self) -> Result<()> {
        if self.classification_confirmed && self.classification.trim().is_empty() {
            return Err(ContractError::invalid("part.classification", "a confirmed classification needs a name"));
        }
        self.volume_mm3.validate("part.volume_mm3")?;
        if self.volume_mm3.unit != Unit::MillimetreCubed {
            return Err(ContractError::invalid("part.volume_mm3.unit", "the reviewed volume is in mm3"));
        }
        if let Some(allowance) = self.allowance_rate
            && allowance.is_sign_negative()
        {
            return Err(ContractError::invalid("part.allowance_rate", "an allowance cannot be negative"));
        }
        if let Some(operation) = &self.operation {
            operation.validate()?;
        }
        Ok(())
    }
}

/// A project draft exactly as the desktop saves it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DraftProject {
    pub project_id: Uuid,
    pub product_revision_id: Uuid,
    /// The saved version this draft is based on; 0 for a project that has never been saved.
    pub version: u64,
    pub source: SourceRef,
    pub analysis: AnalysisManifest,
    /// The explicit project material-overhead rate; `None` until the engineer enters it.
    pub material_overhead_rate: Option<Decimal>,
    pub parts: Vec<PartInputs>,
    pub forecast_year: u16,
    pub forecast_quantity: u64,
    pub currency: String,
}

impl DraftProject {
    pub fn validate(&self) -> Result<()> {
        self.source.validate()?;
        self.analysis.validate()?;
        if self.source.sha256 != self.analysis.key.source_sha256 {
            return Err(ContractError::invalid("draft.source.sha256", "the analysis describes a different source"));
        }
        if !(1900..=9999).contains(&self.forecast_year) {
            return Err(ContractError::invalid("draft.forecast_year", "expected a four-digit calendar year"));
        }
        if self.forecast_quantity == 0 {
            return Err(ContractError::invalid("draft.forecast_quantity", "the fixture needs a positive forecast"));
        }
        if self.currency != "CNY" {
            return Err(ContractError::invalid("draft.currency", "this prototype records costs in CNY"));
        }
        if self.parts.len() > MAX_PARTS {
            return Err(ContractError::invalid("draft.parts", "too many parts"));
        }
        if let Some(rate) = self.material_overhead_rate
            && rate.is_sign_negative()
        {
            return Err(ContractError::invalid("draft.material_overhead_rate", "an overhead rate cannot be negative"));
        }
        let mut part_ids = BTreeSet::new();
        let mut part_definitions = BTreeSet::new();
        for part in &self.parts {
            if !part_ids.insert(part.part_version_id) {
                return Err(ContractError::invalid("draft.parts", "two parts share one id"));
            }
            if !part_definitions.insert(part.definition_id) {
                return Err(ContractError::invalid("draft.parts.definition_id", "two parts review one definition"));
            }
            if !self.analysis.definitions.iter().any(|definition| definition.definition_id == part.definition_id) {
                return Err(ContractError::invalid("draft.parts.definition_id", "no such definition in the analysis"));
            }
            part.validate()?;
            if part.volume_mm3.analysis_key != self.analysis.key {
                return Err(ContractError::invalid("draft.parts.volume_mm3.analysis_key", "the measurement describes a different analysis"));
            }
        }
        Ok(())
    }
}

/// A versioned save request. Reusing `request_id` with the same draft is a safe retry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SaveDraft {
    pub request_id: String,
    pub expected_version: u64,
    pub draft: DraftProject,
}

impl SaveDraft {
    pub fn validate(&self) -> Result<()> {
        check_text(&self.request_id, "save.request_id")?;
        if self.draft.version != self.expected_version {
            return Err(ContractError::invalid("save.expected_version", "the draft describes a different version"));
        }
        self.draft.validate()
    }
}

/// The receipt a committed save returns.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SaveReceipt {
    pub request_id: String,
    pub project_id: Uuid,
    pub version: u64,
}

/// Ask the server to calculate one saved version.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CalculateRequest {
    pub request_id: String,
    pub project_id: Uuid,
    pub saved_version: u64,
}

impl CalculateRequest {
    pub fn validate(&self) -> Result<()> {
        check_text(&self.request_id, "calculation.request_id")?;
        if self.saved_version == 0 {
            return Err(ContractError::invalid("calculation.saved_version", "a calculation needs a saved version"));
        }
        Ok(())
    }
}

/// Server-resolved inputs: versions have become the values they name.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CalculationInputs {
    pub currency: String,
    /// The project's explicit overhead rate; `None` is missing, not zero.
    pub material_overhead_rate: Option<Decimal>,
    pub parts: Vec<ResolvedPartInput>,
}

/// One part's values as the calculator receives them. `None` is missing, never zero.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolvedPartInput {
    pub part_version_id: Uuid,
    pub quantity: u32,
    pub volume_mm3: Option<Decimal>,
    pub density_g_cm3: Option<Decimal>,
    pub material_price_per_kg: Option<Decimal>,
    pub allowance_rate: Option<Decimal>,
    pub cycle_seconds: Option<Decimal>,
    pub cavities: Option<u32>,
    pub production_rate_per_hour: Option<Decimal>,
    pub setup_hours: Option<Decimal>,
    pub setup_rate_per_hour: Option<Decimal>,
}

/// Whether every supported direct-cost line is known.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResultState {
    Incomplete,
    Complete,
}

/// One explained arithmetic line.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CostLine {
    pub code: String,
    pub label: String,
    pub amount: Option<Decimal>,
    pub basis: String,
}

/// One part's direct-cost result. A missing input leaves its affected amounts null.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PartCost {
    pub part_version_id: Uuid,
    pub quantity: u32,
    pub net_mass_kg: Option<Decimal>,
    pub allowed_material_cost: Option<Decimal>,
    pub material_overhead: Option<Decimal>,
    pub production_cost: Option<Decimal>,
    pub per_part_subtotal: Option<Decimal>,
    /// The per-part subtotal expanded by quantity.
    pub line_total: Option<Decimal>,
    pub explanations: Vec<CostLine>,
}

/// The calculator's answer: supported direct costs only, with the setup charge kept separate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CalculationOutcome {
    pub state: ResultState,
    pub parts: Vec<PartCost>,
    pub assembly_subtotal: Option<Decimal>,
    pub setup_total: Option<Decimal>,
    pub issues: Vec<Issue>,
}

/// An immutable calculation result for one saved version.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CalculationSnapshot {
    pub calculation_id: Uuid,
    pub project_id: Uuid,
    pub saved_version: u64,
    pub input_hash: String,
    pub calculator_version: String,
    pub inputs: CalculationInputs,
    pub outputs: CalculationOutcome,
    pub issues: Vec<Issue>,
}

/// A published material version. The development service seeds explicit test values only; they
/// are not company defaults. Versions are append-only: a later price leaves this one unchanged.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MaterialVersion {
    pub material_version_id: Uuid,
    /// The logical material this version belongs to; it is stable across its versions.
    pub material_id: Uuid,
    pub version: u64,
    pub supersedes: Option<Uuid>,
    pub name: String,
    pub density_g_cm3: Decimal,
    pub density_unit: Unit,
    /// `None` is a missing price, distinct from an explicit zero.
    pub price_per_kg: Option<Decimal>,
    pub price_unit: Unit,
    pub currency: String,
    pub source_reference: Option<String>,
    /// RFC 3339 UTC timestamp the service recorded.
    pub created_at: Option<String>,
}

/// A published hourly rate version of one cost center. A setup cost center is dedicated to setup.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RateVersion {
    pub rate_version_id: Uuid,
    /// The logical cost center this version belongs to; it is stable across its versions.
    pub cost_center_id: Uuid,
    pub version: u64,
    pub supersedes: Option<Uuid>,
    pub name: String,
    pub purpose: CostCenterPurpose,
    /// `None` is a missing rate, distinct from an explicit zero.
    pub rate_per_hour: Option<Decimal>,
    pub rate_unit: Unit,
    pub currency: String,
    pub source_reference: Option<String>,
    /// RFC 3339 UTC timestamp the service recorded.
    pub created_at: Option<String>,
}

/// One administrator-declared catalog matching attribute: a stable key and its value.
///
/// A matcher compares a reviewed attribute of the same key against this value, trimming the text
/// and folding ASCII case. The key is a stable lower snake_case code and the value is bounded
/// non-empty text, so a hostile catalog version cannot smuggle control characters or unbounded
/// blobs into matching.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CatalogAttribute {
    /// The stable attribute key, matching `^[a-z][a-z0-9_]{0,63}$`.
    pub key: String,
    /// The declared value: non-empty bounded text without control characters, compared trimmed.
    pub value: String,
}

impl CatalogAttribute {
    /// Validate the stable key and the bounded value.
    pub fn validate(&self, field: &str) -> Result<()> {
        let key_field = format!("{field}.key");
        if !is_stable_code(&self.key, MAX_ATTRIBUTE_KEY_CHARS) {
            return Err(ContractError::invalid(
                key_field.as_str(),
                "expected a stable attribute key: a lower-case letter followed by up to 63 lower-case letters, digits or underscores",
            ));
        }
        let value_field = format!("{field}.value");
        check_text(&self.value, &value_field)?;
        if self.value.chars().count() > MAX_ATTRIBUTE_VALUE_CHARS {
            return Err(ContractError::invalid(value_field.as_str(), "the attribute value is too long"));
        }
        if self.value.chars().any(char::is_control) {
            return Err(ContractError::invalid(value_field.as_str(), "the attribute value contains a control character"));
        }
        Ok(())
    }
}

/// Validate a catalog component version's declared attributes: bounded, unique keys, each valid.
pub(crate) fn validate_catalog_attributes(attributes: &[CatalogAttribute], field: &str) -> Result<()> {
    if attributes.len() > MAX_CATALOG_ATTRIBUTES {
        return Err(ContractError::invalid(field, format!("a catalog component version declares at most {MAX_CATALOG_ATTRIBUTES} attributes")));
    }
    let mut keys = BTreeSet::new();
    for (index, attribute) in attributes.iter().enumerate() {
        attribute.validate(&format!("{field}.{index}"))?;
        if !keys.insert(attribute.key.as_str()) {
            return Err(ContractError::invalid(field, format!("the attribute key `{}` is declared twice", attribute.key)));
        }
    }
    Ok(())
}

/// A published catalog component version: shared reference specifications and price.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CatalogComponentVersion {
    pub component_version_id: Uuid,
    /// The logical component this version belongs to; it is stable across its versions.
    pub component_id: Uuid,
    pub version: u64,
    pub supersedes: Option<Uuid>,
    pub name: String,
    pub specification: String,
    /// `None` is a missing price, distinct from an explicit zero.
    pub price: Option<Decimal>,
    pub price_unit: Unit,
    pub currency: String,
    pub source_reference: Option<String>,
    /// The administrator-declared matching attributes. Optional and empty for legacy records; a
    /// component with no declared attributes never matches a reviewed part as a wildcard.
    #[serde(default)]
    pub attributes: Vec<CatalogAttribute>,
    /// RFC 3339 UTC timestamp the service recorded.
    pub created_at: Option<String>,
}

/// The immutable published reference data a project may select from.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReferenceData {
    /// Every published material version. A project selects one version id.
    pub materials: Vec<MaterialVersion>,
    /// Every published cost-center rate version.
    pub rates: Vec<RateVersion>,
    /// Every published catalog component version.
    #[serde(default)]
    pub components: Vec<CatalogComponentVersion>,
    /// The latest published app cost configuration, if one exists.
    #[serde(default)]
    pub app_config: Option<AppConfigVersion>,
}

impl ReferenceData {
    pub fn validate(&self) -> Result<()> {
        let mut material_ids = BTreeSet::new();
        let mut material_records = BTreeMap::new();
        for material in &self.materials {
            if !material_ids.insert(material.material_version_id) {
                return Err(ContractError::invalid("reference.materials", "two materials share one version id"));
            }
            check_text(&material.name, "reference.materials.name")?;
            if material.version == 0 {
                return Err(ContractError::invalid("reference.materials.version", "a published version starts at 1"));
            }
            if let Some(previous) = material_records.insert(material.material_id, (material.version, material.material_version_id))
                && previous.0 == material.version
            {
                return Err(ContractError::invalid("reference.materials", "a material has two versions with the same number"));
            }
            if let Some(price) = material.price_per_kg
                && price.is_sign_negative()
            {
                return Err(ContractError::invalid("reference.materials.price_per_kg", "a price cannot be negative"));
            }
            if material.density_g_cm3 <= Decimal::ZERO {
                return Err(ContractError::invalid("reference.materials.density_g_cm3", "density must be positive"));
            }
            if material.density_unit != Unit::GramPerCentimetreCubed {
                return Err(ContractError::invalid("reference.materials.density_unit", "density is recorded in g/cm3"));
            }
            if material.price_unit != Unit::CurrencyPerKilogram {
                return Err(ContractError::invalid("reference.materials.price_unit", "a material price is recorded per kilogram"));
            }
            if material.currency != "CNY" {
                return Err(ContractError::invalid("reference.materials.currency", "this prototype records costs in CNY"));
            }
        }
        let mut rate_ids = BTreeSet::new();
        let mut rate_records = BTreeMap::new();
        for rate in &self.rates {
            if !rate_ids.insert(rate.rate_version_id) {
                return Err(ContractError::invalid("reference.rates", "two rates share one version id"));
            }
            check_text(&rate.name, "reference.rates.name")?;
            if rate.version == 0 {
                return Err(ContractError::invalid("reference.rates.version", "a published version starts at 1"));
            }
            if let Some(previous) = rate_records.insert(rate.cost_center_id, (rate.version, rate.rate_version_id))
                && previous.0 == rate.version
            {
                return Err(ContractError::invalid("reference.rates", "a cost center has two versions with the same number"));
            }
            if let Some(rate_per_hour) = rate.rate_per_hour
                && rate_per_hour.is_sign_negative()
            {
                return Err(ContractError::invalid("reference.rates.rate_per_hour", "a rate cannot be negative"));
            }
            if rate.rate_unit != Unit::CurrencyPerHour {
                return Err(ContractError::invalid("reference.rates.rate_unit", "a rate is recorded per hour"));
            }
            if rate.currency != "CNY" {
                return Err(ContractError::invalid("reference.rates.currency", "this prototype records costs in CNY"));
            }
        }
        let mut component_ids = BTreeSet::new();
        let mut component_records = BTreeMap::new();
        for component in &self.components {
            if !component_ids.insert(component.component_version_id) {
                return Err(ContractError::invalid("reference.components", "two components share one version id"));
            }
            check_text(&component.name, "reference.components.name")?;
            check_text(&component.specification, "reference.components.specification")?;
            if component.version == 0 {
                return Err(ContractError::invalid("reference.components.version", "a published version starts at 1"));
            }
            if let Some(previous) = component_records.insert(component.component_id, (component.version, component.component_version_id))
                && previous.0 == component.version
            {
                return Err(ContractError::invalid("reference.components", "a component has two versions with the same number"));
            }
            if let Some(price) = component.price
                && price.is_sign_negative()
            {
                return Err(ContractError::invalid("reference.components.price", "a price cannot be negative"));
            }
            if component.price_unit != Unit::CurrencyPerPiece {
                return Err(ContractError::invalid("reference.components.price_unit", "a component price is recorded per piece"));
            }
            if component.currency != "CNY" {
                return Err(ContractError::invalid("reference.components.currency", "this prototype records costs in CNY"));
            }
            validate_catalog_attributes(&component.attributes, "reference.components.attributes")?;
        }
        if let Some(config) = &self.app_config {
            config.validate()?;
        }
        Ok(())
    }

    pub fn material(&self, id: Uuid) -> Option<&MaterialVersion> {
        self.materials.iter().find(|material| material.material_version_id == id)
    }

    pub fn rate(&self, id: Uuid) -> Option<&RateVersion> {
        self.rates.iter().find(|rate| rate.rate_version_id == id)
    }

    pub fn component(&self, id: Uuid) -> Option<&CatalogComponentVersion> {
        self.components.iter().find(|component| component.component_version_id == id)
    }

    /// The newest published version of each logical material, in name order.
    pub fn latest_materials(&self) -> Vec<&MaterialVersion> {
        latest_versions(&self.materials, |material| material.material_id, |material| material.version, |material| &material.name)
    }

    /// The newest published version of each logical cost center, in name order.
    pub fn latest_rates(&self) -> Vec<&RateVersion> {
        latest_versions(&self.rates, |rate| rate.cost_center_id, |rate| rate.version, |rate| &rate.name)
    }

    /// The newest published version of each logical catalog component, in name order.
    pub fn latest_components(&self) -> Vec<&CatalogComponentVersion> {
        latest_versions(&self.components, |component| component.component_id, |component| component.version, |component| &component.name)
    }
}

/// Keep the highest version of each logical record, then order by name.
fn latest_versions<'a, T, Id, Version, Name>(records: &'a [T], id: Id, version: Version, name: Name) -> Vec<&'a T>
where
    Id: Fn(&'a T) -> Uuid,
    Version: Fn(&'a T) -> u64,
    Name: Fn(&'a T) -> &'a String,
{
    let mut newest: BTreeMap<Uuid, &'a T> = BTreeMap::new();
    for record in records {
        let record_version = version(record);
        let slot = newest.entry(id(record)).or_insert(record);
        if record_version > version(slot) {
            newest.insert(id(record), record);
        }
    }
    let mut records: Vec<&'a T> = newest.into_values().collect();
    records.sort_by(|left, right| name(left).cmp(name(right)));
    records
}

/// The compact reference a project view lists for each saved calculation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CalculationSummary {
    pub calculation_id: Uuid,
    pub saved_version: u64,
    pub calculator_version: String,
}

/// What reopening a project returns: the current draft and the immutable results saved for it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProjectView {
    pub draft: DraftProject,
    pub calculations: Vec<CalculationSummary>,
}

/// A named problem with the field it affects. A missing value is an issue, not a zero.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Issue {
    pub code: String,
    pub field_path: String,
    pub definition_id: Option<Uuid>,
    pub message: String,
}

impl Issue {
    pub fn new(code: impl Into<String>, field_path: impl Into<String>, message: impl Into<String>) -> Self {
        Self { code: code.into(), field_path: field_path.into(), definition_id: None, message: message.into() }
    }

    pub fn for_definition(mut self, definition_id: Uuid) -> Self {
        self.definition_id = Some(definition_id);
        self
    }

    pub fn validate(&self) -> Result<()> {
        check_text(&self.code, "issue.code")?;
        check_text(&self.field_path, "issue.field_path")?;
        check_text(&self.message, "issue.message")
    }
}

/// A field this contract requires to be non-empty text.
pub(crate) fn check_text(text: &str, field: &str) -> Result<()> {
    if text.trim().is_empty() { Err(ContractError::invalid(field, "must not be empty")) } else { Ok(()) }
}

/// Whether `text` is a stable lower snake_case code: a lower-case ASCII letter followed by at most
/// `max_chars - 1` lower-case ASCII letters, digits or underscores. Category codes and catalog
/// attribute keys share this shape, so a new code needs no schema change.
pub(crate) fn is_stable_code(text: &str, max_chars: usize) -> bool {
    let mut chars = text.chars();
    let Some(first) = chars.next() else { return false };
    if !first.is_ascii_lowercase() {
        return false;
    }
    text.chars().count() <= max_chars && chars.all(|character| character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_')
}

fn check_transform(transform: &[[f64; 4]; 4], field: &str) -> Result<()> {
    if transform.iter().flatten().any(|value| !value.is_finite()) {
        return Err(ContractError::invalid(field, "a placement must be finite"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use rust_decimal::Decimal;

    use super::*;
    use crate::error::decimal_from_f64;

    fn key() -> AnalysisKey {
        AnalysisKey { source_sha256: "a".repeat(64), engine_build: "0.40.0".into(), extractor_version: "1".into() }
    }

    fn source() -> SourceRef {
        SourceRef { source_id: Uuid::nil(), sha256: "a".repeat(64), byte_len: 10, original_name: "three_boxes.step".into() }
    }

    fn measured(value: Option<&str>) -> Measurement {
        Measurement {
            value: value.map(|v| v.parse().unwrap_or(Decimal::ZERO)),
            unit: Unit::MillimetreCubed,
            source: if value.is_some() { MeasurementSource::Extracted } else { MeasurementSource::Manual },
            review: ReviewState::Confirmed,
            reason: None,
            analysis_key: key(),
        }
    }

    #[test]
    fn a_decimal_is_written_as_a_string_and_a_missing_value_as_null() {
        let present = measured(Some("2400"));
        let absent = measured(None);
        assert_eq!(serde_json::to_value(&present).unwrap()["value"], serde_json::json!("2400"));
        assert_eq!(serde_json::to_value(&absent).unwrap()["value"], serde_json::Value::Null);
    }

    #[test]
    fn an_explicit_zero_stays_distinct_from_missing() {
        let zero = measured(Some("0"));
        let absent = measured(None);
        assert_eq!(serde_json::to_value(&zero).unwrap()["value"], serde_json::json!("0"));
        assert_ne!(serde_json::to_value(&zero).unwrap()["value"], serde_json::to_value(&absent).unwrap()["value"]);
    }

    #[test]
    fn unknown_fields_are_tolerated() {
        let json = serde_json::json!({
            "source_id": Uuid::nil(),
            "sha256": "a".repeat(64),
            "byte_len": 10,
            "original_name": "three_boxes.step",
            "written_by": "a later build",
        });
        assert!(serde_json::from_value::<SourceRef>(json).is_ok());
    }

    #[test]
    fn a_later_schema_version_is_refused() {
        let manifest = AnalysisManifest {
            schema_version: SCHEMA_VERSION + 1,
            key: key(),
            definitions: Vec::new(),
            occurrences: Vec::new(),
            geometry_bindings: Vec::new(),
            issues: Vec::new(),
        };
        assert!(matches!(manifest.validate(), Err(ContractError::SchemaVersion { .. })));
    }

    #[test]
    fn a_manual_measurement_needs_a_reason() {
        let mut measurement = measured(None);
        assert!(measurement.validate("volume").is_err());
        measurement.reason = Some("open surface measured in the workshop".into());
        assert!(measurement.validate("volume").is_ok());
    }

    #[test]
    fn a_definition_and_its_occurrences_must_share_the_analysis_key() {
        let definition_id = Uuid::nil();
        let manifest = AnalysisManifest {
            schema_version: SCHEMA_VERSION,
            key: key(),
            definitions: vec![DefinitionRef {
                definition_id,
                source_key: AnalysisKey { source_sha256: "b".repeat(64), ..key() },
                customer_part_number: None,
                customer_revision: None,
                volume_mm3: measured(Some("2400")),
            }],
            occurrences: Vec::new(),
            geometry_bindings: Vec::new(),
            issues: Vec::new(),
        };
        assert!(manifest.validate().is_err());
    }

    #[test]
    fn a_draft_must_describe_one_source_version_and_a_positive_forecast() {
        let draft = DraftProject {
            project_id: Uuid::nil(),
            product_revision_id: Uuid::nil(),
            version: 0,
            source: source(),
            analysis: AnalysisManifest {
                schema_version: SCHEMA_VERSION,
                key: key(),
                definitions: Vec::new(),
                occurrences: Vec::new(),
                geometry_bindings: Vec::new(),
                issues: Vec::new(),
            },
            material_overhead_rate: Some(Decimal::ZERO),
            parts: Vec::new(),
            forecast_year: 2027,
            forecast_quantity: 1,
            currency: "CNY".into(),
        };
        assert!(draft.validate().is_ok());
        assert!(DraftProject { forecast_quantity: 0, ..draft.clone() }.validate().is_err());
    }

    #[test]
    fn a_geometry_number_converts_through_its_round_trip_text() {
        assert_eq!(decimal_from_f64(2400.0, "volume").unwrap().to_string(), "2400");
    }

    #[test]
    fn every_unit_has_its_wire_name() {
        let names = [
            (Unit::MillimetreCubed, "mm3"),
            (Unit::CentimetreCubed, "cm3"),
            (Unit::GramPerCentimetreCubed, "g_cm3"),
            (Unit::CurrencyPerKilogram, "cny_kg"),
            (Unit::Second, "s"),
            (Unit::Hour, "h"),
            (Unit::CurrencyPerHour, "cny_h"),
            (Unit::Currency, "cny"),
            (Unit::CurrencyPerPiece, "cny_piece"),
        ];
        for (unit, name) in names {
            assert_eq!(serde_json::to_value(unit).ok(), Some(serde_json::Value::String(name.into())), "{unit:?}");
            assert_eq!(serde_json::from_value::<Unit>(serde_json::Value::String(name.into())).ok(), Some(unit));
        }
    }

    fn part(definition_id: Uuid, key: &AnalysisKey) -> PartInputs {
        PartInputs {
            part_version_id: Uuid::from_u128(100),
            definition_id,
            classification: "clip".into(),
            classification_confirmed: true,
            sourcing: Sourcing::Manufactured,
            volume_mm3: Measurement {
                value: Some("2400".parse().unwrap_or_default()),
                unit: Unit::MillimetreCubed,
                source: MeasurementSource::Extracted,
                review: ReviewState::Confirmed,
                reason: None,
                analysis_key: key.clone(),
            },
            material_version_id: Some(Uuid::from_u128(500)),
            allowance_rate: Some("0.02".parse().unwrap_or_default()),
            operation: Some(OperationInputs {
                cycle_seconds: Some("30".parse().unwrap_or_default()),
                cycle_basis: CycleBasis::PerPiece,
                cavities: Some(1),
                production_rate_version_id: Some(Uuid::from_u128(600)),
                setup_hours: Some("0.5".parse().unwrap_or_default()),
                setup_rate_version_id: Some(Uuid::from_u128(601)),
            }),
        }
    }

    fn reviewed_draft() -> DraftProject {
        let key = key();
        let definition_id = Uuid::from_u128(10);
        DraftProject {
            project_id: Uuid::from_u128(1),
            product_revision_id: Uuid::from_u128(2),
            version: 3,
            source: source(),
            analysis: AnalysisManifest {
                schema_version: SCHEMA_VERSION,
                key: key.clone(),
                definitions: vec![DefinitionRef {
                    definition_id,
                    source_key: key.clone(),
                    customer_part_number: Some("0727 8201 004".into()),
                    customer_revision: Some("B".into()),
                    volume_mm3: Measurement {
                        value: Some("2400".parse().unwrap_or_default()),
                        unit: Unit::MillimetreCubed,
                        source: MeasurementSource::Extracted,
                        review: ReviewState::Confirmed,
                        reason: None,
                        analysis_key: key.clone(),
                    },
                }],
                occurrences: vec![OccurrenceRef {
                    occurrence_id: Uuid::from_u128(20),
                    definition_id,
                    parent_occurrence_id: None,
                    source_path: vec![0],
                    transform: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]],
                }],
                geometry_bindings: vec![GeometryBinding { source_path: vec![0], source_body: 0, definition_id, occurrence_id: Uuid::from_u128(20) }],
                issues: Vec::new(),
            },
            material_overhead_rate: Some("0.05".parse().unwrap_or_default()),
            parts: vec![part(definition_id, &key)],
            forecast_year: 2027,
            forecast_quantity: 1000,
            currency: "CNY".into(),
        }
    }

    #[test]
    fn a_full_reviewed_project_round_trips_through_json_with_decimal_strings() {
        let draft = reviewed_draft();
        let value = serde_json::to_value(&draft).unwrap_or_default();
        assert_eq!(value["material_overhead_rate"], serde_json::json!("0.05"));
        assert_eq!(value["parts"][0]["allowance_rate"], serde_json::json!("0.02"));
        assert_eq!(value["parts"][0]["volume_mm3"]["value"], serde_json::json!("2400"));
        assert_eq!(value["parts"][0]["operation"]["cycle_seconds"], serde_json::json!("30"));
        assert_eq!(serde_json::from_value::<DraftProject>(value).ok(), Some(draft));
    }

    #[test]
    fn a_saved_project_view_round_trips() {
        let view = ProjectView {
            draft: reviewed_draft(),
            calculations: vec![CalculationSummary {
                calculation_id: Uuid::from_u128(7),
                saved_version: 3,
                calculator_version: "solidbom-direct-cost/1".into(),
            }],
        };
        let text = serde_json::to_string(&view).unwrap_or_default();
        assert_eq!(serde_json::from_str::<ProjectView>(&text).ok(), Some(view));
    }

    #[test]
    fn an_immutable_calculation_snapshot_round_trips_with_decimal_strings() {
        let snapshot = CalculationSnapshot {
            calculation_id: Uuid::from_u128(7),
            project_id: Uuid::from_u128(1),
            saved_version: 3,
            input_hash: "a".repeat(64),
            calculator_version: "solidbom-direct-cost/1".into(),
            inputs: CalculationInputs {
                currency: "CNY".into(),
                material_overhead_rate: Some("0.05".parse().unwrap_or_default()),
                parts: vec![ResolvedPartInput {
                    part_version_id: Uuid::from_u128(100),
                    quantity: 3,
                    volume_mm3: Some("2400".parse().unwrap_or_default()),
                    density_g_cm3: Some("2.7".parse().unwrap_or_default()),
                    material_price_per_kg: Some("20".parse().unwrap_or_default()),
                    allowance_rate: Some("0.02".parse().unwrap_or_default()),
                    cycle_seconds: Some("30".parse().unwrap_or_default()),
                    cavities: Some(1),
                    production_rate_per_hour: Some("120".parse().unwrap_or_default()),
                    setup_hours: Some("0.5".parse().unwrap_or_default()),
                    setup_rate_per_hour: Some("120".parse().unwrap_or_default()),
                }],
            },
            outputs: CalculationOutcome {
                state: ResultState::Complete,
                parts: vec![PartCost {
                    part_version_id: Uuid::from_u128(100),
                    quantity: 3,
                    net_mass_kg: Some("0.00648".parse().unwrap_or_default()),
                    allowed_material_cost: Some("0.132192".parse().unwrap_or_default()),
                    material_overhead: Some("0.0066096".parse().unwrap_or_default()),
                    production_cost: Some("1".parse().unwrap_or_default()),
                    per_part_subtotal: Some("1.1388016".parse().unwrap_or_default()),
                    line_total: Some("3.4164048".parse().unwrap_or_default()),
                    explanations: Vec::new(),
                }],
                assembly_subtotal: Some("3.4164048".parse().unwrap_or_default()),
                setup_total: Some("60".parse().unwrap_or_default()),
                issues: Vec::new(),
            },
            issues: Vec::new(),
        };
        let value = serde_json::to_value(&snapshot).unwrap_or_default();
        assert_eq!(value["outputs"]["assembly_subtotal"], serde_json::json!("3.4164048"));
        assert_eq!(value["inputs"]["material_overhead_rate"], serde_json::json!("0.05"));
        assert_eq!(serde_json::from_value::<CalculationSnapshot>(value).ok(), Some(snapshot));
    }

    fn catalog_component(attributes: Vec<CatalogAttribute>) -> CatalogComponentVersion {
        CatalogComponentVersion {
            component_version_id: Uuid::from_u128(700),
            component_id: Uuid::from_u128(70),
            version: 1,
            supersedes: None,
            name: "Hose clip 12-20".into(),
            specification: "stainless steel, 12-20 mm".into(),
            price: Some("0.42".parse().unwrap_or_default()),
            price_unit: Unit::CurrencyPerPiece,
            currency: "CNY".into(),
            source_reference: None,
            attributes,
            created_at: None,
        }
    }

    fn catalog_reference(attributes: Vec<CatalogAttribute>) -> ReferenceData {
        ReferenceData { materials: Vec::new(), rates: Vec::new(), components: vec![catalog_component(attributes)], app_config: None }
    }

    fn attribute(key: &str, value: &str) -> CatalogAttribute {
        CatalogAttribute { key: key.into(), value: value.into() }
    }

    #[test]
    fn catalog_attributes_are_bounded_unique_and_validated() {
        let reference = catalog_reference(vec![attribute("diameter_mm", "12"), attribute("material", "stainless_steel")]);
        assert!(reference.validate().is_ok());
        assert_eq!(reference.components[0].attributes.len(), 2);

        for attributes in [
            vec![attribute("Diameter", "12")],
            vec![attribute("1diameter", "12")],
            vec![attribute("diameter_mm", " ")],
            vec![attribute("diameter_mm", "line\nbreak")],
            vec![attribute("diameter_mm", &"x".repeat(MAX_ATTRIBUTE_VALUE_CHARS + 1))],
            vec![attribute("diameter_mm", "12"), attribute("diameter_mm", "16")],
        ] {
            assert!(catalog_reference(attributes).validate().is_err());
        }
        let too_many = (0..=MAX_CATALOG_ATTRIBUTES).map(|index| attribute(&format!("attribute_{index}"), "value")).collect();
        assert!(catalog_reference(too_many).validate().is_err());
    }

    #[test]
    fn an_old_catalog_component_version_parses_with_empty_attributes() {
        let old = serde_json::json!({
            "component_version_id": Uuid::from_u128(700),
            "component_id": Uuid::from_u128(70),
            "version": 1,
            "supersedes": null,
            "name": "Hose clip 12-20",
            "specification": "stainless steel, 12-20 mm",
            "price": "0.42",
            "price_unit": "cny_piece",
            "currency": "CNY",
            "source_reference": null,
            "created_at": null,
        });
        let component: CatalogComponentVersion = serde_json::from_value(old).unwrap();
        assert!(component.attributes.is_empty(), "a legacy record declares no matching attributes");
        assert!(ReferenceData { materials: Vec::new(), rates: Vec::new(), components: vec![component.clone()], app_config: None }.validate().is_ok());

        let declared = catalog_component(vec![attribute("diameter_mm", "12")]);
        let value = serde_json::to_value(&declared).unwrap();
        assert_eq!(value["attributes"], serde_json::json!([{ "key": "diameter_mm", "value": "12" }]));
        assert_eq!(serde_json::from_value::<CatalogComponentVersion>(value).ok(), Some(declared));
    }
}
