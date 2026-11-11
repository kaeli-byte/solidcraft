//! Internal Excel and PDF exports (package SB-21, additive to API version 2).
//!
//! The service exports one explicitly selected saved result: an accepted baseline (SB-19) or the
//! project's current draft calculation snapshot (SB-16). The export request names that selection,
//! the saved commercial scenario versions to compare and the output format; the answer carries the
//! rendered bytes together with the metadata the documents record (the exact project, engineering,
//! manufacturing and feasibility record versions, the costing calculation identity, the freshness
//! labels and the stale reasons). The documents are internal review material, never a customer
//! quotation.
//!
//! The full snapshot-resolved export model and the two renderers are service-side
//! (`services/solidbom-server/src/export/`); this module is the portable wire vocabulary the
//! desktop host uses. It carries no rendering code and no arithmetic: the renderers draw stored
//! values, they never recompute a cost.
//!
//! # Rules that apply to every value
//!
//! The version 1 and version 2 rules still apply: money is a decimal string, a missing value is
//! `null` and an explicit zero is `"0"`, unknown JSON fields are tolerated, a newer stored schema is
//! refused and a value received from another program is validated before it is used. Additional
//! rules for this contract:
//!
//! - An export is a pure read. It names one exact saved result; it never appends a project version,
//!   a snapshot, an accepted baseline or a request receipt, and it never recalculates a cost.
//! - A draft export names the project's **current draft result**: the newest baseline calculation
//!   snapshot that no accepted baseline pins. A superseded draft snapshot is refused, exactly like
//!   an expired client read; exporting an earlier accepted baseline is always possible.
//! - The selected snapshot is preserved: every identifier the documents print comes from the
//!   snapshot and the record versions it pinned, never from the current master data. The only value
//!   read at export time for a draft result is the feasibility record, which no SB-16 snapshot
//!   pins; its version is recorded and the documents label it as read at export time.
//! - Labels are stable contract names in a canonical order, not display text. The renderer owns the
//!   words it prints for them.
//! - The bundled bytes are the rendered document for the named format. `media_type` and `byte_len`
//!   must match, and `sha256` is the digest a downloading client can check. The file name is a
//!   suggestion: one ASCII base name with the format's extension.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{ContractError, Result, check_sha256};
use crate::types::ResultState;

/// The internal-export contract version this build reads and writes.
pub const EXPORT_CONTRACT_VERSION: u32 = 1;

/// The most saved commercial scenario versions one export request compares.
pub const MAX_EXPORT_SCENARIOS: usize = 32;

/// The longest export file name suggestion this contract carries.
pub const MAX_EXPORT_FILE_NAME_CHARS: usize = 128;

/// The longest project, customer or calculator text this contract carries.
const MAX_TEXT_CHARS: usize = 255;

/// The longest calculator version text an export carries.
const MAX_CALCULATOR_VERSION_CHARS: usize = 64;

/// The longest RFC 3339 UTC timestamp text this contract carries.
const MAX_TIMESTAMP_CHARS: usize = 64;

/// The output format of one export.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    /// The detailed internal workbook.
    Xlsx,
    /// The internal summary document.
    Pdf,
}

impl ExportFormat {
    /// The media type the rendered document has.
    pub fn media_type(self) -> &'static str {
        match self {
            Self::Xlsx => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            Self::Pdf => "application/pdf",
        }
    }

    /// The file extension without the dot.
    pub fn file_extension(self) -> &'static str {
        match self {
            Self::Xlsx => "xlsx",
            Self::Pdf => "pdf",
        }
    }

    /// The stable contract name, as the JSON value spells it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Xlsx => "xlsx",
            Self::Pdf => "pdf",
        }
    }
}

/// Which saved result one export reads.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExportBaselineRef {
    /// One exact accepted baseline of the project, with the sequence it was accepted at.
    AcceptedBaseline { accepted_baseline_id: Uuid, sequence: u64 },
    /// The project's current draft result, by the immutable SB-16 calculation id.
    DraftCalculation { calculation_id: Uuid },
}

impl ExportBaselineRef {
    fn validate(&self) -> Result<()> {
        match self {
            Self::AcceptedBaseline { accepted_baseline_id, sequence } => {
                check_named_id(*accepted_baseline_id, "export_request.baseline.accepted_baseline_id")?;
                check_version(*sequence, "export_request.baseline.sequence")
            }
            Self::DraftCalculation { calculation_id } => check_named_id(*calculation_id, "export_request.baseline.calculation_id"),
        }
    }
}

/// One request to render an internal export of one saved project result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExportRequest {
    /// The project whose saved result is exported. The request is sent to that project's route and
    /// both must agree; the id is carried here so a host can route the request without another
    /// lookup.
    pub project_id: Uuid,
    /// The exact saved result to export.
    pub baseline: ExportBaselineRef,
    /// The saved commercial scenario versions to compare against that result, in ascending
    /// scenario-id order; empty means the documents carry no scenario section.
    #[serde(default)]
    pub scenarios: Vec<Uuid>,
    /// The output format.
    pub format: ExportFormat,
}

impl ExportRequest {
    /// Validate the selection and the requested comparison.
    pub fn validate(&self) -> Result<()> {
        check_named_id(self.project_id, "export_request.project_id")?;
        self.baseline.validate()?;
        if self.scenarios.len() > MAX_EXPORT_SCENARIOS {
            return Err(ContractError::invalid(
                "export_request.scenarios",
                format!("at most {MAX_EXPORT_SCENARIOS} saved scenarios are compared in one export"),
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        for (index, scenario_id) in self.scenarios.iter().enumerate() {
            check_named_id(*scenario_id, &format!("export_request.scenarios[{index}]"))?;
            if !seen.insert(*scenario_id) {
                return Err(ContractError::invalid(format!("export_request.scenarios[{index}]"), "a scenario appears twice"));
            }
        }
        if self.scenarios.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(ContractError::invalid("export_request.scenarios", "scenario ids are in ascending order"));
        }
        Ok(())
    }
}

/// The freshness statement one export carries. These are stable contract names, not display text;
/// a renderer owns the words it prints.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ExportLabel {
    /// The exported result is the project's current draft, not an accepted baseline.
    DraftResult,
    /// The exported costing result is incomplete: at least one required input is missing.
    IncompleteResult,
    /// At least one issue names a missing input or an unavailable value.
    MissingInputs,
    /// At least one feasibility finding is unresolved, failed, conflicting or needs review.
    UnresolvedFindings,
    /// A costing-relevant shared record was published after the exported snapshot read its inputs.
    OutdatedCosts,
    /// A newer accepted baseline heads the project.
    SupersededAcceptedBaseline,
    /// At least one exported scenario was saved against another result, project version or setting.
    StaleScenarios,
    /// The feasibility record was read at export time because no costing snapshot pins it.
    UnpinnedFeasibilityRecord,
}

impl ExportLabel {
    /// The stable contract name, used for ordering and as the JSON value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DraftResult => "draft_result",
            Self::IncompleteResult => "incomplete_result",
            Self::MissingInputs => "missing_inputs",
            Self::UnresolvedFindings => "unresolved_findings",
            Self::OutdatedCosts => "outdated_costs",
            Self::SupersededAcceptedBaseline => "superseded_accepted_baseline",
            Self::StaleScenarios => "stale_scenarios",
            Self::UnpinnedFeasibilityRecord => "unpinned_feasibility_record",
        }
    }

    fn order(self) -> u8 {
        match self {
            Self::DraftResult => 0,
            Self::IncompleteResult => 1,
            Self::MissingInputs => 2,
            Self::UnresolvedFindings => 3,
            Self::OutdatedCosts => 4,
            Self::SupersededAcceptedBaseline => 5,
            Self::StaleScenarios => 6,
            Self::UnpinnedFeasibilityRecord => 7,
        }
    }
}

/// Why a draft result is stale, as the SB-20 adoption contract names it. An accepted baseline is
/// accepted history, so it carries no stale reason; a newer acceptance is a label instead.
///
/// The canonical order is alphabetical by [`ExportStaleReason::as_str`], not the declaration order;
/// consumers must sort with that key (or compare the names) rather than a derived `Ord`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ExportStaleReason {
    /// The project record moved past the version the result read.
    NewerProjectVersion,
    /// The engineering record moved past the version the result read.
    NewerEngineeringVersion,
    /// The manufacturing record moved past the version the result read.
    NewerManufacturingVersion,
    /// A costing-relevant shared-data update is pending adoption, or an adoption was recorded
    /// after the result read its inputs.
    NewerSharedData,
}

impl ExportStaleReason {
    /// The stable contract name, used for ordering.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NewerProjectVersion => "newer_project_version",
            Self::NewerEngineeringVersion => "newer_engineering_version",
            Self::NewerManufacturingVersion => "newer_manufacturing_version",
            Self::NewerSharedData => "newer_shared_data",
        }
    }
}

/// Which saved result the export states it rendered.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExportBaselineSelection {
    /// One exact accepted baseline, with its append-only project sequence.
    AcceptedBaseline { accepted_baseline_id: Uuid, sequence: u64 },
    /// The project's current draft result, by calculation id.
    DraftCalculation { calculation_id: Uuid },
}

impl ExportBaselineSelection {
    fn validate(&self) -> Result<()> {
        match self {
            Self::AcceptedBaseline { accepted_baseline_id, sequence } => {
                check_named_id(*accepted_baseline_id, "export_metadata.baseline.accepted_baseline_id")?;
                check_version(*sequence, "export_metadata.baseline.sequence")
            }
            Self::DraftCalculation { calculation_id } => check_named_id(*calculation_id, "export_metadata.baseline.calculation_id"),
        }
    }
}

/// The exact record versions the exported snapshot read.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExportRecordVersions {
    /// The immutable project record version the costing snapshot recorded.
    pub project_version: u64,
    /// The engineering record version the costing snapshot recorded; 0 is the empty record.
    pub engineering_version: u64,
    /// The manufacturing record version the costing snapshot recorded; 0 is the empty record.
    pub manufacturing_version: u64,
    /// The feasibility record version the documents read; 0 is the empty record.
    pub feasibility_version: u64,
    /// Whether the feasibility record is part of the selected saved result (an accepted baseline
    /// pins it) or was read at export time (a draft costing snapshot does not pin it).
    pub feasibility_pinned: bool,
}

impl ExportRecordVersions {
    fn validate(&self) -> Result<()> {
        check_version(self.project_version, "export_metadata.versions.project_version")
    }
}

/// The immutable costing calculation the documents render.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExportCalculation {
    /// The immutable SB-16 snapshot identity.
    pub calculation_id: Uuid,
    /// The arithmetic version that produced the stored cost values.
    pub calculator_version: String,
    /// The canonical hash of the stored resolved inputs.
    pub input_hash: String,
    /// The stored result state; incomplete results stay exportable.
    pub state: ResultState,
}

impl ExportCalculation {
    fn validate(&self) -> Result<()> {
        check_named_id(self.calculation_id, "export_metadata.calculation.calculation_id")?;
        check_text(&self.calculator_version, "export_metadata.calculation.calculator_version", MAX_CALCULATOR_VERSION_CHARS)?;
        check_sha256(&self.input_hash, "export_metadata.calculation.input_hash")
    }
}

/// What the rendered documents say about the result they carry. The renders embed this record and
/// the desktop host receives the same value beside the bytes, so a reader can identify the exact
/// saved result without opening the document.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExportMetadata {
    pub project_id: Uuid,
    /// The project's stored name.
    pub project_name: String,
    /// The stored customer text.
    pub customer: String,
    /// The project currency the amounts are stated in.
    pub currency: String,
    /// The RFC 3339 UTC timestamp the service recorded for this export.
    pub exported_at: String,
    pub format: ExportFormat,
    pub baseline: ExportBaselineSelection,
    pub versions: ExportRecordVersions,
    pub calculation: ExportCalculation,
    /// The freshness labels in canonical order.
    #[serde(default)]
    pub labels: Vec<ExportLabel>,
    /// Whether the exported result is a stale draft result.
    pub stale: bool,
    /// The named stale reasons, sorted and empty exactly when `stale` is false.
    #[serde(default)]
    pub stale_reasons: Vec<ExportStaleReason>,
}

impl ExportMetadata {
    /// Validate the identity, the versions, the labels and the freshness statement.
    pub fn validate(&self) -> Result<()> {
        check_named_id(self.project_id, "export_metadata.project_id")?;
        check_text(&self.project_name, "export_metadata.project_name", MAX_TEXT_CHARS)?;
        check_optional_text(&self.customer, "export_metadata.customer", MAX_TEXT_CHARS)?;
        check_currency(&self.currency, "export_metadata.currency")?;
        check_text(&self.exported_at, "export_metadata.exported_at", MAX_TIMESTAMP_CHARS)?;
        self.baseline.validate()?;
        self.versions.validate()?;
        self.calculation.validate()?;
        check_labels(&self.labels)?;
        check_stale_reasons(&self.stale_reasons)?;
        if self.stale != !self.stale_reasons.is_empty() {
            return Err(ContractError::invalid("export_metadata.stale", "a stale export names at least one reason and a fresh one names none"));
        }
        if self.stale && matches!(self.baseline, ExportBaselineSelection::AcceptedBaseline { .. }) {
            return Err(ContractError::invalid("export_metadata.stale", "an accepted baseline is accepted history; it is not a stale draft"));
        }
        check_selection_and_labels(self)
    }
}

/// One rendered internal export.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExportBundle {
    /// What the rendered documents say about the result they carry.
    pub metadata: ExportMetadata,
    /// The suggested file name: one ASCII base name with the format's extension.
    pub file_name: String,
    /// The rendered document's media type; equals `metadata.format.media_type()`.
    pub media_type: String,
    /// The rendered document's length in bytes.
    pub byte_len: u64,
    /// The SHA-256 digest of the rendered bytes, 64 lower-case hexadecimal characters.
    pub sha256: String,
    /// The rendered document; the JSON wire form is base64.
    #[serde(with = "base64_bytes")]
    pub bytes: Vec<u8>,
}

impl ExportBundle {
    /// Validate the bundle's shape and its agreement with its metadata. The digest is a transport
    /// check for the client: a value received from another program cannot be recomputed here, so
    /// this method checks its form and the length but leaves the content check to the consumer.
    pub fn validate(&self) -> Result<()> {
        self.metadata.validate()?;
        check_file_name(&self.file_name)?;
        if self.file_name == "." || self.file_name == ".." || self.file_name.contains(['/', '\\', '\0']) {
            return Err(ContractError::invalid("export_bundle.file_name", "the file name is one base name, not a path"));
        }
        if !self.file_name.ends_with(&format!(".{}", self.metadata.format.file_extension())) {
            return Err(ContractError::invalid(
                "export_bundle.file_name",
                format!("the file name ends with the {} extension", self.metadata.format.file_extension()),
            ));
        }
        if self.media_type != self.metadata.format.media_type() {
            return Err(ContractError::invalid("export_bundle.media_type", "the media type is the selected format's"));
        }
        if self.byte_len != u64::try_from(self.bytes.len()).unwrap_or(u64::MAX) {
            return Err(ContractError::invalid("export_bundle.byte_len", "the byte length is the rendered length"));
        }
        check_sha256(&self.sha256, "export_bundle.sha256")
    }
}

/// Encode bytes as standard base64 with padding.
pub fn encode_base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk.first().copied().unwrap_or(0);
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        let group = (u32::from(first) << 16) | (u32::from(second) << 8) | u32::from(third);
        for index in 0..4 {
            if index <= chunk.len() {
                let shift = 18 - 6 * index;
                let value = ((group >> shift) & 0b11_1111) as usize;
                out.push(ALPHABET.get(value).copied().unwrap_or(b'A') as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Decode standard base64 with padding, refusing anything that is not a canonical encoding.
pub fn decode_base64(text: &str) -> Result<Vec<u8>> {
    if !text.len().is_multiple_of(4) {
        return Err(ContractError::invalid("base64", "the text length is a multiple of four"));
    }
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    for (index, chunk) in text.as_bytes().chunks(4).enumerate() {
        let field = format!("base64[{index}]");
        let mut values = [0u32; 4];
        let mut padding = 0usize;
        for (position, byte) in chunk.iter().enumerate() {
            if *byte == b'=' {
                // Padding is only allowed in the last group, in its last two positions, and only
                // with a value that leaves the discarded bits zero.
                if index + 1 != text.len() / 4 || position < 2 {
                    return Err(ContractError::invalid(&field, "padding is allowed only at the end"));
                }
                padding += 1;
                continue;
            }
            if padding > 0 {
                return Err(ContractError::invalid(&field, "no data follows padding"));
            }
            values[position] = base64_value(*byte).ok_or_else(|| ContractError::invalid(&field, "expected a base64 character"))?;
        }
        let group = (values[0] << 18) | (values[1] << 12) | (values[2] << 6) | values[3];
        let bytes = [(group >> 16) as u8, (group >> 8) as u8, group as u8];
        let kept = 3 - padding;
        if kept >= 1 {
            out.push(bytes[0]);
        }
        if kept >= 2 {
            out.push(bytes[1]);
        }
        if kept >= 3 {
            out.push(bytes[2]);
        }
        // The bits that are dropped must be zero: a non-canonical encoding is refused instead of
        // silently accepted as another valid document.
        if padding == 1 && values[2] & 0b11 != 0 {
            return Err(ContractError::invalid(&field, "the padding leaves non-zero bits"));
        }
        if padding == 2 && values[1] & 0b1111 != 0 {
            return Err(ContractError::invalid(&field, "the padding leaves non-zero bits"));
        }
    }
    Ok(out)
}

/// The base64 JSON representation of [`ExportBundle::bytes`].
pub mod base64_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    use crate::error::ContractError;

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&super::encode_base64(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Vec<u8>, D::Error> {
        let text = String::deserialize(deserializer)?;
        super::decode_base64(&text).map_err(|error| serde::de::Error::custom(ContractError::to_string(&error)))
    }
}

fn base64_value(byte: u8) -> Option<u32> {
    match byte {
        b'A'..=b'Z' => Some(u32::from(byte - b'A')),
        b'a'..=b'z' => Some(u32::from(byte - b'a') + 26),
        b'0'..=b'9' => Some(u32::from(byte - b'0') + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

fn check_labels(labels: &[ExportLabel]) -> Result<()> {
    for (index, label) in labels.iter().enumerate() {
        if labels.get(index + 1).is_some_and(|next| next.order() <= label.order()) {
            return Err(ContractError::invalid("export_metadata.labels", "the labels are sorted and unique"));
        }
    }
    Ok(())
}

fn check_stale_reasons(reasons: &[ExportStaleReason]) -> Result<()> {
    for (index, reason) in reasons.iter().enumerate() {
        if reasons.get(index + 1).is_some_and(|next| next.as_str() <= reason.as_str()) {
            return Err(ContractError::invalid("export_metadata.stale_reasons", "the stale reasons are sorted and unique"));
        }
    }
    Ok(())
}

/// The stated selection, calculation, record versions and labels must describe one consistent
/// saved result: a draft names exactly the calculation it carries and is labelled as a draft, and an
/// accepted baseline is pinned history whose feasibility record came from the activation.
fn check_selection_and_labels(metadata: &ExportMetadata) -> Result<()> {
    let has = |label: ExportLabel| metadata.labels.contains(&label);
    match metadata.baseline {
        ExportBaselineSelection::DraftCalculation { calculation_id } => {
            if metadata.calculation.calculation_id != calculation_id {
                return Err(ContractError::invalid(
                    "export_metadata.baseline.calculation_id",
                    "the draft selection names the calculation the documents carry",
                ));
            }
            if !has(ExportLabel::DraftResult) {
                return Err(ContractError::invalid("export_metadata.labels", "a draft result carries the draft_result label"));
            }
            if metadata.versions.feasibility_pinned {
                return Err(ContractError::invalid(
                    "export_metadata.versions.feasibility_pinned",
                    "a draft calculation snapshot pins no feasibility record",
                ));
            }
        }
        ExportBaselineSelection::AcceptedBaseline { .. } => {
            if has(ExportLabel::DraftResult) {
                return Err(ContractError::invalid("export_metadata.labels", "an accepted baseline is not a draft result"));
            }
            if !metadata.versions.feasibility_pinned {
                return Err(ContractError::invalid(
                    "export_metadata.versions.feasibility_pinned",
                    "an accepted baseline pins the feasibility record activation read",
                ));
            }
        }
    }
    if has(ExportLabel::UnpinnedFeasibilityRecord) && metadata.versions.feasibility_pinned {
        return Err(ContractError::invalid(
            "export_metadata.labels",
            "an unpinned_feasibility_record label needs a feasibility record that was not pinned",
        ));
    }
    Ok(())
}

fn check_named_id(id: Uuid, field: &str) -> Result<()> {
    if id.is_nil() { Err(ContractError::invalid(field, "the id is empty")) } else { Ok(()) }
}

fn check_version(version: u64, field: &str) -> Result<()> {
    if version == 0 { Err(ContractError::invalid(field, "the version starts at 1")) } else { Ok(()) }
}

fn check_currency(currency: &str, field: &str) -> Result<()> {
    if currency.len() == 3 && currency.bytes().all(|byte| byte.is_ascii_uppercase()) {
        Ok(())
    } else {
        Err(ContractError::invalid(field, "expected a three-letter upper-case currency code"))
    }
}

fn check_text(text: &str, field: &str, max_chars: usize) -> Result<()> {
    if text.trim().is_empty() {
        return Err(ContractError::invalid(field, "the text is empty"));
    }
    check_char_count(text, field, max_chars)
}

fn check_optional_text(text: &str, field: &str, max_chars: usize) -> Result<()> {
    if text.trim().is_empty() { Ok(()) } else { check_char_count(text, field, max_chars) }
}

fn check_char_count(text: &str, field: &str, max_chars: usize) -> Result<()> {
    if text.chars().count() > max_chars {
        return Err(ContractError::invalid(field, format!("at most {max_chars} characters")));
    }
    Ok(())
}

/// Check one file-name suggestion: bounded, printable ASCII without a path separator. The
/// project name it is derived from may carry any text the project contract allows, so the service
/// sanitises it; a hostile value is refused here instead of reaching a client's file system.
fn check_file_name(file_name: &str) -> Result<()> {
    check_text(file_name, "export_bundle.file_name", MAX_EXPORT_FILE_NAME_CHARS)?;
    if !file_name.is_ascii() {
        return Err(ContractError::invalid("export_bundle.file_name", "the file name is ASCII"));
    }
    if file_name.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(ContractError::invalid("export_bundle.file_name", "the file name has no control character"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ResultState;

    fn id(value: u128) -> Uuid {
        Uuid::from_u128(value)
    }

    fn metadata() -> ExportMetadata {
        ExportMetadata {
            project_id: id(1),
            project_name: "Three boxes".into(),
            customer: "Internal".into(),
            currency: "CNY".into(),
            exported_at: "2026-11-11T13:00:00Z".into(),
            format: ExportFormat::Xlsx,
            baseline: ExportBaselineSelection::DraftCalculation { calculation_id: id(2) },
            versions: ExportRecordVersions {
                project_version: 4,
                engineering_version: 3,
                manufacturing_version: 2,
                feasibility_version: 1,
                feasibility_pinned: false,
            },
            calculation: ExportCalculation {
                calculation_id: id(2),
                calculator_version: "solidbom-manufacturing-baseline/1".into(),
                input_hash: "a".repeat(64),
                state: ResultState::Incomplete,
            },
            labels: vec![ExportLabel::DraftResult, ExportLabel::IncompleteResult],
            stale: true,
            stale_reasons: vec![ExportStaleReason::NewerProjectVersion, ExportStaleReason::NewerSharedData],
        }
    }

    fn bundle() -> ExportBundle {
        let bytes = b"rendered".to_vec();
        ExportBundle {
            metadata: metadata(),
            file_name: "three-boxes-draft.xlsx".into(),
            media_type: ExportFormat::Xlsx.media_type().into(),
            byte_len: 8,
            sha256: "b".repeat(64),
            bytes,
        }
    }

    #[test]
    fn a_draft_request_round_trips_and_checks_its_scenario_order() {
        let request = ExportRequest {
            project_id: id(1),
            baseline: ExportBaselineRef::DraftCalculation { calculation_id: id(2) },
            scenarios: vec![id(10), id(11)],
            format: ExportFormat::Pdf,
        };
        assert!(request.validate().is_ok());
        let text = serde_json::to_string(&request).expect("the request serializes");
        assert_eq!(serde_json::from_str::<ExportRequest>(&text).expect("the request reads"), request);

        let unordered = ExportRequest { scenarios: vec![id(11), id(10)], ..request.clone() };
        assert!(unordered.validate().is_err());
        let duplicate = ExportRequest { scenarios: vec![id(10), id(10)], ..request };
        assert!(duplicate.validate().is_err());
    }

    #[test]
    fn an_accepted_baseline_selection_names_its_sequence() {
        let request = ExportRequest {
            project_id: id(1),
            baseline: ExportBaselineRef::AcceptedBaseline { accepted_baseline_id: id(3), sequence: 7 },
            scenarios: Vec::new(),
            format: ExportFormat::Xlsx,
        };
        assert!(request.validate().is_ok());
        let mut bad = request.clone();
        bad.baseline = ExportBaselineRef::AcceptedBaseline { accepted_baseline_id: id(3), sequence: 0 };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn formats_name_their_media_type_and_extension() {
        assert_eq!(ExportFormat::Xlsx.file_extension(), "xlsx");
        assert_eq!(ExportFormat::Pdf.file_extension(), "pdf");
        assert_eq!(ExportFormat::Pdf.media_type(), "application/pdf");
        assert!(ExportFormat::Xlsx.media_type().ends_with("spreadsheetml.sheet"));
        assert_eq!(serde_json::to_string(&ExportFormat::Pdf).expect("the format serializes"), "\"pdf\"");
    }

    #[test]
    fn the_metadata_checks_its_versions_labels_and_freshness() {
        let metadata = metadata();
        assert!(metadata.validate().is_ok());

        let accepted_stale =
            ExportMetadata { baseline: ExportBaselineSelection::AcceptedBaseline { accepted_baseline_id: id(3), sequence: 1 }, ..metadata.clone() };
        assert!(accepted_stale.validate().is_err(), "accepted history is never a stale draft");

        let unsorted = ExportMetadata { labels: vec![ExportLabel::IncompleteResult, ExportLabel::DraftResult], ..metadata.clone() };
        assert!(unsorted.validate().is_err());
        let fresh_but_stale = ExportMetadata { stale: false, ..metadata.clone() };
        assert!(fresh_but_stale.validate().is_err());
        let stale_without_reason = ExportMetadata { stale_reasons: Vec::new(), ..metadata.clone() };
        assert!(stale_without_reason.validate().is_err());
        let unsorted_reasons =
            ExportMetadata { stale_reasons: vec![ExportStaleReason::NewerSharedData, ExportStaleReason::NewerProjectVersion], ..metadata };
        assert!(unsorted_reasons.validate().is_err());
    }

    #[test]
    fn stored_project_text_with_control_characters_is_exportable() {
        // The project contract accepts a newline or a tab in the stored name and customer, so the
        // export metadata cannot refuse them; the renderers turn them into printable text.
        let mut metadata = metadata();
        metadata.project_name = "Line one\nLine two".into();
        metadata.customer = "Tab\there".into();
        assert!(metadata.validate().is_ok());
        // The suggested file name is the service's printable sanitisation; a control character in
        // a received name is still refused.
        let mut bundle = bundle();
        bundle.file_name = "three-boxes\ndraft.xlsx".into();
        assert!(bundle.validate().is_err());
    }

    #[test]
    fn the_stated_selection_and_labels_must_describe_one_saved_result() {
        let draft = metadata();
        assert!(draft.validate().is_ok());
        let other_calculation =
            ExportMetadata { calculation: ExportCalculation { calculation_id: id(9), ..draft.calculation.clone() }, ..draft.clone() };
        assert!(other_calculation.validate().is_err());
        let missing_draft_label = ExportMetadata { labels: vec![ExportLabel::IncompleteResult], ..draft.clone() };
        assert!(missing_draft_label.validate().is_err());
        let pinned_draft = ExportMetadata {
            versions: ExportRecordVersions { feasibility_pinned: true, ..draft.versions },
            labels: vec![ExportLabel::DraftResult],
            ..draft.clone()
        };
        assert!(pinned_draft.validate().is_err());

        let accepted = ExportMetadata {
            baseline: ExportBaselineSelection::AcceptedBaseline { accepted_baseline_id: id(3), sequence: 1 },
            labels: Vec::new(),
            stale: false,
            stale_reasons: Vec::new(),
            versions: ExportRecordVersions { feasibility_pinned: true, ..draft.versions },
            ..draft.clone()
        };
        assert!(accepted.validate().is_ok());
        let accepted_draft_label = ExportMetadata { labels: vec![ExportLabel::DraftResult], ..accepted.clone() };
        assert!(accepted_draft_label.validate().is_err());
        let unpinned_accepted = ExportMetadata { versions: ExportRecordVersions { feasibility_pinned: false, ..accepted.versions }, ..accepted };
        assert!(unpinned_accepted.validate().is_err());
    }

    #[test]
    fn an_unknown_baseline_or_calculation_version_is_refused() {
        let mut bad_hash = metadata();
        bad_hash.calculation.input_hash = "A".repeat(64);
        assert!(bad_hash.validate().is_err());
        let mut bad_version = metadata();
        bad_version.versions.project_version = 0;
        assert!(bad_version.validate().is_err());
        let mut bad_currency = metadata();
        bad_currency.currency = "cny".into();
        assert!(bad_currency.validate().is_err());
        let mut bad_name = metadata();
        bad_name.project_name = "  ".into();
        assert!(bad_name.validate().is_err());
    }

    #[test]
    fn a_bundle_agrees_with_its_metadata_and_is_one_base_name() {
        assert!(bundle().validate().is_ok());
        let wrong_media = ExportBundle { media_type: "application/pdf".into(), ..bundle() };
        assert!(wrong_media.validate().is_err());
        let wrong_length = ExportBundle { byte_len: 9, ..bundle() };
        assert!(wrong_length.validate().is_err());
        let path = ExportBundle { file_name: "dir/three-boxes.xlsx".into(), ..bundle() };
        assert!(path.validate().is_err());
        let wrong_extension = ExportBundle { file_name: "three-boxes.pdf".into(), ..bundle() };
        assert!(wrong_extension.validate().is_err());
        let parent = ExportBundle { file_name: "..".into(), ..bundle() };
        assert!(parent.validate().is_err());
    }

    #[test]
    fn the_bundle_transports_its_bytes_as_base64() {
        let bundle = bundle();
        let value = serde_json::to_value(&bundle).expect("the bundle serializes");
        assert_eq!(value["bytes"], serde_json::json!(encode_base64(b"rendered")));
        assert!(!value["bytes"].as_str().unwrap_or_default().is_empty());
        let parsed: ExportBundle = serde_json::from_value(value).expect("the bundle reads");
        assert_eq!(parsed, bundle);
    }

    #[test]
    fn a_base64_transport_round_trips_every_byte_length_and_refuses_non_canonical_text() {
        let bytes: Vec<u8> = (0..=255).collect();
        for len in 0..=bytes.len() {
            let text = encode_base64(&bytes[..len]);
            assert_eq!(decode_base64(&text).ok(), Some(bytes[..len].to_vec()), "length {len}");
        }
        for bad in ["A", "AA", "AAA", "AAAA=", "A===", "====", "AB==", "AAB=", "~~~~"] {
            assert!(decode_base64(bad).is_err(), "{bad}");
        }
        // `QQ==` is `A`; `QR==` leaves non-zero bits and is not canonical.
        assert_eq!(decode_base64("QQ==").ok(), Some(vec![b'A']));
        assert!(decode_base64("QR==").is_err());
    }

    #[test]
    fn unknown_json_fields_are_tolerated_and_never_promoted() {
        let mut value = serde_json::to_value(metadata()).expect("the metadata serializes");
        value["future_metadata_field"] = serde_json::json!("tolerated");
        value["versions"]["future_version_field"] = serde_json::json!(9);
        let parsed: ExportMetadata = serde_json::from_value(value).expect("unknown fields are tolerated");
        assert!(parsed.validate().is_ok());
        let text = serde_json::to_string(&parsed).expect("the metadata serializes");
        assert!(!text.contains("future_metadata_field"));

        let mut request = serde_json::to_value(ExportRequest {
            project_id: id(1),
            baseline: ExportBaselineRef::DraftCalculation { calculation_id: id(2) },
            scenarios: Vec::new(),
            format: ExportFormat::Xlsx,
        })
        .expect("the request serializes");
        request["future_request_field"] = serde_json::json!(true);
        let parsed: ExportRequest = serde_json::from_value(request).expect("unknown fields are tolerated");
        assert!(parsed.validate().is_ok());
    }
}
