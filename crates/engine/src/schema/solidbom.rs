//! SolidBOM's command contracts: import, reviewed inputs, save/calculate, retry and reopen.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Import one STEP file and start its review.
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
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
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
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

/// A command with no parameters.
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct EmptyParams {}

/// Reopen a project by id.
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
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
    pub error: Option<String>,
    pub notice: Option<String>,
    pub needs_service: bool,
}
