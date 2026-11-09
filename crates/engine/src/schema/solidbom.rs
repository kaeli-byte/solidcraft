//! SolidBOM's command contracts: import, reviewed inputs, save/calculate, retry and reopen.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Import one STEP file and start its review.
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct ImportParams {
    /// The local STEP file to import and review.
    pub path: String,
}

/// What an import produced. A background analysis answers before it has definitions: the
/// `state` names the review phase (`importing`), and the counts and ids follow once the analysis
/// verifies the source and the review state reports `reviewing`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ImportReply {
    pub project_id: String,
    pub definitions: u32,
    pub occurrences: u32,
    pub part_version_ids: Vec<String>,
    pub issues: Vec<String>,
    pub needs_service: bool,
    /// The review phase at the time of the reply: `importing` or `reviewing`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

/// Edit one reviewed part. Decimal values are exact strings; an empty string clears a value.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct InputsParams {
    pub part_version_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classification: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classification_confirmed: Option<bool>,
    /// `manufactured` or `purchased`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sourcing: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume_mm3: Option<String>,
    /// Why a manual volume was entered; marks the measurement manual.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume_confirmed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material_version_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowance_rate: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle_seconds: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cavities: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub production_rate_version_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup_hours: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup_rate_version_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material_overhead_rate: Option<String>,
}

/// A reviewed-part patch; project-wide values are accepted only by the batch itself.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct PartInputPatch {
    pub part_version_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classification: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classification_confirmed: Option<bool>,
    /// `manufactured` or `purchased`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sourcing: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume_mm3: Option<String>,
    /// Why a manual volume was entered; marks the measurement manual.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume_confirmed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material_version_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowance_rate: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle_seconds: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cavities: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub production_rate_version_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup_hours: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup_rate_version_id: Option<String>,
}

impl From<PartInputPatch> for InputsParams {
    fn from(patch: PartInputPatch) -> Self {
        Self {
            part_version_id: patch.part_version_id,
            classification: patch.classification,
            classification_confirmed: patch.classification_confirmed,
            sourcing: patch.sourcing,
            volume_mm3: patch.volume_mm3,
            manual_reason: patch.manual_reason,
            volume_confirmed: patch.volume_confirmed,
            material_version_id: patch.material_version_id,
            allowance_rate: patch.allowance_rate,
            cycle_seconds: patch.cycle_seconds,
            cavities: patch.cavities,
            production_rate_version_id: patch.production_rate_version_id,
            setup_hours: patch.setup_hours,
            setup_rate_version_id: patch.setup_rate_version_id,
            material_overhead_rate: None,
        }
    }
}

/// Apply every pending part edit together. Project overhead belongs here, not in a part patch.
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct BatchInputsParams {
    pub parts: Vec<PartInputPatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material_overhead_rate: Option<String>,
}

/// Source or calculation finding associated with one reviewed definition.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PartIssue {
    pub code: String,
    pub field_path: String,
    pub message: String,
}

/// One shared reviewed definition, independently of table order or display tessellation.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PartRow {
    pub part_version_id: String,
    pub definition_id: String,
    pub part: String,
    pub revision: Option<String>,
    pub quantity: usize,
    pub classification: String,
    pub classification_confirmed: bool,
    pub sourcing: String,
    pub material: Option<String>,
    pub material_version_id: Option<String>,
    pub volume_mm3: Option<String>,
    pub volume_source: String,
    pub volume_review: String,
    pub manual_reason: Option<String>,
    pub needs_review: bool,
    pub missing_inputs: Vec<String>,
    pub issues: Vec<PartIssue>,
    pub per_part_subtotal: Option<String>,
    pub line_total: Option<String>,
    pub result_state: String,
    /// Full session occurrence paths, resolved from the exact imported source mapping.
    pub occurrence_paths: Vec<Vec<u64>>,
    pub inputs: InputsParams,
}

/// Source identity used to produce every measurement in a parts reply.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PartsAnalysisKey {
    pub source_sha256: String,
    pub engine_build: String,
    pub extractor_version: String,
}

/// Typed table projection shared by the workspace, CLI, control channel and MCP.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PartsReply {
    pub rows: Vec<PartRow>,
    pub currency: Option<String>,
    pub material_overhead_rate: Option<String>,
    pub generation: u64,
    pub analysis_key: Option<PartsAnalysisKey>,
}

/// A command with no parameters.
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct EmptyParams {}

/// The shared-data surface has no parameters: every call reloads the organization's records.
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct SharedParams {}

/// Publish the next material version. A blank id creates a new material record; the version's own
/// density is required, because no value is inherited.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct MaterialVersionParams {
    /// The logical material id; blank creates a new material record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material_id: Option<String>,
    pub name: String,
    /// This version's density, an exact decimal string greater than zero.
    pub density_g_cm3: String,
    /// A blank string is a missing price, distinct from `0` for an explicit zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_per_kg: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_reference: Option<String>,
    /// The version this publish replaces; 0 creates version 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_version: Option<u64>,
}

/// Publish the next cost-center rate version. A blank id creates a new cost center.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct CostCenterVersionParams {
    /// The logical cost-center id; blank creates a new cost center.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_center_id: Option<String>,
    pub name: String,
    /// `production` or `setup`; a setup cost center is dedicated to setup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    /// A blank string is a missing rate, distinct from `0` for an explicit zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_per_hour: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_reference: Option<String>,
    /// The version this publish replaces; 0 creates version 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_version: Option<u64>,
}

/// Publish the next catalog component version. A blank id creates a new component.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct ComponentVersionParams {
    /// The logical component id; blank creates a new component.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component_id: Option<String>,
    pub name: String,
    pub specification: String,
    /// A blank string is a missing price, distinct from `0` for an explicit zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_reference: Option<String>,
    /// The version this publish replaces; 0 creates version 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_version: Option<u64>,
}

/// Publish the next app cost configuration version. A blank id creates a new configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct AppConfigVersionParams {
    /// The logical configuration id; blank creates a new configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_id: Option<String>,
    pub currency: String,
    pub categories: Vec<CostCategoryParams>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_reference: Option<String>,
    /// The version this publish replaces; 0 creates version 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_version: Option<u64>,
}

/// One app cost category in a publish request. Monetary values are exact decimal strings.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct CostCategoryParams {
    pub code: String,
    pub name: String,
    pub calculation: CategoryCalculationParams,
    #[serde(default)]
    pub included_in_ebit: bool,
}

/// How one cost category is calculated. There is no formula language.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CategoryCalculationParams {
    /// A calculated direct cost from a supported engine: `material`, `production` or `setup`.
    Direct { input: String },
    /// A fixed amount per unit, an exact decimal string.
    Fixed { amount: String },
    /// A percentage of an explicitly defined cost subtotal.
    Percentage { basis: CategoryBasisParams, rate: String },
}

/// The explicit subtotal a percentage category is calculated from.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct CategoryBasisParams {
    #[serde(default)]
    pub direct_cost: bool,
    #[serde(default)]
    pub categories: Vec<String>,
}

/// One published version of a shared record as the administration surface lists it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SharedVersionRow {
    /// The logical record id, stable across its versions.
    pub record_id: String,
    pub version_id: String,
    pub version: u64,
    pub name: String,
    /// The record's own detail: a density with unit, a rate purpose or a specification.
    pub detail: String,
    /// A missing value is `null`, never a zero amount.
    pub price: Option<String>,
    pub currency: String,
    pub source_reference: Option<String>,
    pub created_at: Option<String>,
}

/// The newest published app cost configuration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SharedAppConfigRow {
    pub config_id: String,
    pub version_id: String,
    pub version: u64,
    pub currency: String,
    /// The configured category codes, in their configuration order.
    pub categories: Vec<String>,
    pub source_reference: Option<String>,
    pub created_at: Option<String>,
}

/// One recent publish fact for a shared record, newest first.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SharedChangeRow {
    pub change_id: String,
    /// `material`, `cost_center`, `component` or `app_config`.
    pub kind: String,
    pub record_id: String,
    pub version: u64,
    pub summary: String,
    pub changed_by: String,
    pub changed_at: String,
}

/// What the administration surface and every shared-data command report.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SharedReply {
    pub identity_id: Option<String>,
    pub identity_name: Option<String>,
    /// The permission sets the loading identity holds, by their stable wire names.
    pub permissions: Vec<String>,
    /// Whether the identity may maintain shared records. The service still enforces every route.
    pub administrator: bool,
    /// Every published material version, oldest version first per material.
    pub materials: Vec<SharedVersionRow>,
    /// Every published cost-center rate version.
    pub cost_centers: Vec<SharedVersionRow>,
    /// Every published catalog component version.
    pub components: Vec<SharedVersionRow>,
    pub app_config: Option<SharedAppConfigRow>,
    /// The newest reference-change facts, newest first.
    pub changes: Vec<SharedChangeRow>,
    /// The shared-data requests still in flight, by kind.
    pub pending: Vec<String>,
    pub error: Option<String>,
    pub notice: Option<String>,
}

/// Reopen a project by id.
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct ReopenParams {
    pub project_id: String,
}

/// Load one saved calculation snapshot of the open project by its id.
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct CalculationParams {
    pub calculation_id: String,
}

/// What a save/calculate/retry/reopen request started.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct JobReply {
    pub request_id: Option<String>,
    pub state: StateReply,
}

/// The review state the panel and programmatic callers read.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StateReply {
    pub phase: String,
    pub save: String,
    pub result: String,
    pub project_id: Option<String>,
    pub version: u64,
    pub definitions: usize,
    pub occurrences: usize,
    pub issues: Vec<String>,
    pub calculations: Vec<String>,
    pub current_calculation: Option<String>,
    pub assembly_subtotal: Option<String>,
    pub per_part_subtotal: Option<String>,
    pub setup_total: Option<String>,
    pub volume_mm3: Option<String>,
    pub volume_source: Option<String>,
    pub volume_review: Option<String>,
    pub part_costs: Vec<String>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub needs_service: bool,
    /// Whether `solidbom.retry` can resend the failed request unchanged.
    pub retryable: bool,
}

#[cfg(test)]
#[path = "solidbom_tests.rs"]
mod tests;
