//! SolidBOM's command contracts: import, reviewed inputs, save/calculate, retry and reopen.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Import one STEP file and start its review.
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportParams {
    /// The local STEP file to import and review.
    pub path: String,
}

/// What an import produced.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ImportReply {
    pub project_id: String,
    pub definitions: u32,
    pub occurrences: u32,
    pub part_version_ids: Vec<String>,
    pub issues: Vec<String>,
    pub needs_service: bool,
}

/// Edit one reviewed part. Decimal values are exact strings; an empty string clears a value.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct EmptyParams {}

/// Reopen a project by id.
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReopenParams {
    pub project_id: String,
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
