//! SolidBOM's commands: import a reviewed STEP, edit its inputs, save, calculate, retry and
//! reopen. Every one is a normal command, so the panel, control channel, MCP and scripts reach
//! the same behaviour; none opens a dialog.

use std::str::FromStr;

use rust_decimal::Decimal;
use serde_json::{Value, json};
use solidbom_contract::{
    AnalysisKey, AnalysisManifest, CycleBasis, DraftProject, MeasurementSource, OperationInputs, PartInputs, ReviewState, SourceRef, Sourcing,
};
use uuid::Uuid;

use super::CommandSpec;
use crate::params::bad;
use crate::schema::solidbom::{EmptyParams, ImportParams, ImportReply, InputsParams, JobReply, ReopenParams, StateReply};
use crate::schema::{TypedContract, output_schema_for};
use crate::solidbom::EXTRACTOR_VERSION;
use crate::solidbom::extract::extract;
use crate::solidbom::host::ClientRequest;
use crate::solidbom::policy::ReviewPolicy;
use crate::solidbom::state::{Phase, ResultState, SaveState, State as SolidBomState};
use crate::{EngineError, Result, Session};

/// The service's configured upload limit; a prototype resource limit.
const MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;

const IMPORT_CONTRACT: TypedContract<ImportParams> = TypedContract::new();
const INPUTS_CONTRACT: TypedContract<InputsParams> = TypedContract::new();
const EMPTY_CONTRACT: TypedContract<EmptyParams> = TypedContract::new();
const REOPEN_CONTRACT: TypedContract<ReopenParams> = TypedContract::new();

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("solidbom.import", "Import STEP for Review", crate::checked!("solidbom.import", import))
        .icon("bom")
        .noundo()
        .contract(&IMPORT_CONTRACT)
        .output(output_schema_for::<ImportReply>)
        .params("path: a local STEP file to import and review → project_id, definitions, occurrences, issues, needs_service"),
    CommandSpec::new("solidbom.inputs", "Review SolidBOM Inputs", crate::checked!("solidbom.inputs", inputs))
        .icon("settings")
        .noundo()
        .contract(&INPUTS_CONTRACT)
        .output(output_schema_for::<StateReply>)
        .params(
            "part_version_id; classification?, classification_confirmed?, sourcing?, volume_mm3?, manual_reason?, volume_confirmed?, \
             material_version_id?, allowance_rate?, cycle_seconds?, cavities?, production_rate_version_id?, setup_hours?, \
             setup_rate_version_id?, material_overhead_rate? → the review state",
        ),
    CommandSpec::new("solidbom.save", "Save SolidBOM Project", crate::checked!("solidbom.save", save))
        .icon("save")
        .noundo()
        .contract(&EMPTY_CONTRACT)
        .output(output_schema_for::<JobReply>)
        .params("→ request_id, state"),
    CommandSpec::new("solidbom.calculate", "Calculate SolidBOM Direct Cost", crate::checked!("solidbom.calculate", calculate))
        .icon("measure")
        .noundo()
        .contract(&EMPTY_CONTRACT)
        .output(output_schema_for::<JobReply>)
        .params("saves unsaved inputs first → request_id, state"),
    CommandSpec::new("solidbom.retry", "Retry SolidBOM Request", crate::checked!("solidbom.retry", retry))
        .icon("activate")
        .noundo()
        .contract(&EMPTY_CONTRACT)
        .output(output_schema_for::<JobReply>)
        .params("resubmits the failed request with the same id and payload → request_id, state"),
    CommandSpec::new("solidbom.reopen", "Reopen SolidBOM Project", crate::checked!("solidbom.reopen", reopen))
        .icon("open")
        .noundo()
        .contract(&REOPEN_CONTRACT)
        .output(output_schema_for::<JobReply>)
        .params("project_id: the project to load and verify → request_id, state"),
    CommandSpec::new("solidbom.state", "SolidBOM Review State", crate::checked!("solidbom.state", state_command))
        .icon("list")
        .noundo()
        .contract(&EMPTY_CONTRACT)
        .output(output_schema_for::<StateReply>)
        .params("→ phase, save, result, project, version, issues, calculations"),
    CommandSpec::new("solidbom.review", "SolidBOM Review", crate::checked!("solidbom.review", review))
        .icon("bom")
        .noundo()
        .contract(&EMPTY_CONTRACT)
        .output(output_schema_for::<StateReply>)
        .params("opens the review panel → the review state"),
];

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "solidbom.import";
    let params: ImportParams = serde_json::from_value(p.clone()).map_err(|error| bad(cmd, error.to_string()))?;
    if s.review.is_some() {
        return Err(bad(cmd, "a reviewed source is already open in this session"));
    }
    let path = std::path::PathBuf::from(&params.path);
    let metadata = std::fs::metadata(&path).map_err(|error| bad(cmd, format!("{}: {error}", path.display())))?;
    if !metadata.is_file() {
        return Err(bad(cmd, format!("{} is not a file", path.display())));
    }
    if !solidcraft_io::is_step_path(&params.path) {
        return Err(bad(cmd, "only a STEP file (.step or .stp) can be imported for review"));
    }
    if metadata.len() > MAX_SOURCE_BYTES {
        return Err(bad(cmd, format!("the file exceeds the {MAX_SOURCE_BYTES}-byte development limit")));
    }
    let bytes = std::fs::read(&path).map_err(|error| bad(cmd, format!("{}: {error}", path.display())))?;
    let sha256 = crate::solidbom::state::hex(&bytes);
    s.execute("doc.open", &json!({"path": path.to_string_lossy()}))?;
    // The original bytes are the source; the opened design is only a scratch review.
    s.path = None;
    s.read_only = false;
    let key = AnalysisKey {
        source_sha256: sha256.clone(),
        engine_build: env!("CARGO_PKG_VERSION").to_string(),
        extractor_version: EXTRACTOR_VERSION.to_string(),
    };
    let manifest = extract(s, key.clone())?;
    let original_name = path.file_name().and_then(|name| name.to_str()).unwrap_or("source.step").to_string();
    let source = SourceRef {
        source_id: Uuid::new_v5(&Uuid::NAMESPACE_OID, format!("{sha256}:source").as_bytes()),
        sha256: sha256.clone(),
        byte_len: u64::try_from(bytes.len()).map_err(|_| bad(cmd, "the source is too large"))?,
        original_name: original_name.clone(),
    };
    let draft = draft_from(&manifest, &source, &sha256);
    draft.validate().map_err(|error| bad(cmd, error.to_string()))?;
    let project_id = draft.project_id;
    let part_version_ids: Vec<String> = draft.parts.iter().map(|part| part.part_version_id.to_string()).collect();
    let definitions = manifest.definitions.len();
    let occurrences = manifest.occurrences.len();
    let issues: Vec<String> = manifest.issues.iter().map(|issue| issue.code.clone()).collect();
    s.solidbom = SolidBomState { phase: Phase::Reviewing, save: SaveState::Dirty, draft: Some(draft), ..Default::default() };
    s.bind_review(ReviewPolicy::new(source, key));
    let mut needs_service = true;
    if s.solidbom_host.is_some() {
        let request = ClientRequest::UploadSource { sha256, original_name, bytes };
        match s.submit_solidbom(request, crate::solidbom::state::PendingKind::UploadSource, None, None) {
            Ok(_) => needs_service = false,
            Err(error) => s.solidbom.error = Some(error.to_string()),
        }
    }
    s.solidbom.needs_service = needs_service;
    serde_json::to_value(ImportReply {
        project_id: project_id.to_string(),
        definitions: u32::try_from(definitions).unwrap_or(u32::MAX),
        occurrences: u32::try_from(occurrences).unwrap_or(u32::MAX),
        part_version_ids,
        issues,
        needs_service,
    })
    .map_err(|error| EngineError::Other(error.to_string()))
}

fn draft_from(manifest: &AnalysisManifest, source: &SourceRef, sha256: &str) -> DraftProject {
    let namespace = Uuid::NAMESPACE_OID;
    DraftProject {
        project_id: Uuid::new_v5(&namespace, format!("{sha256}:project").as_bytes()),
        product_revision_id: Uuid::new_v5(&namespace, format!("{sha256}:revision").as_bytes()),
        version: 0,
        source: source.clone(),
        analysis: manifest.clone(),
        material_overhead_rate: None,
        parts: manifest
            .definitions
            .iter()
            .map(|definition| PartInputs {
                part_version_id: Uuid::new_v5(&namespace, format!("{sha256}:part:{}", definition.definition_id).as_bytes()),
                definition_id: definition.definition_id,
                classification: String::new(),
                classification_confirmed: false,
                sourcing: Sourcing::Manufactured,
                volume_mm3: definition.volume_mm3.clone(),
                material_version_id: None,
                allowance_rate: None,
                operation: None,
            })
            .collect(),
        forecast_year: 2027,
        forecast_quantity: 1000,
        currency: "CNY".into(),
    }
}

fn inputs(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "solidbom.inputs";
    let params: InputsParams = serde_json::from_value(p.clone()).map_err(|error| bad(cmd, error.to_string()))?;
    let part_version_id = Uuid::parse_str(&params.part_version_id).map_err(|_| bad(cmd, "`part_version_id` must be a UUID"))?;
    let state = &mut s.solidbom;
    let mut draft = state.draft.clone().ok_or_else(|| bad(cmd, "no SolidBOM project is open"))?;
    {
        let part =
            draft.parts.iter_mut().find(|part| part.part_version_id == part_version_id).ok_or_else(|| bad(cmd, "no reviewed part has that id"))?;
        if let Some(value) = &params.classification {
            part.classification.clone_from(value);
        }
        if let Some(value) = params.classification_confirmed {
            part.classification_confirmed = value;
        }
        if let Some(value) = &params.sourcing {
            part.sourcing = match value.as_str() {
                "manufactured" => Sourcing::Manufactured,
                "purchased" => Sourcing::Purchased,
                other => return Err(bad(cmd, format!("`sourcing` must be manufactured or purchased, not `{other}`"))),
            };
        }
        if let Some(value) = &params.volume_mm3 {
            let volume = decimal(cmd, "volume_mm3", value)?;
            part.volume_mm3.value = volume;
            part.volume_mm3.source = MeasurementSource::Extracted;
            part.volume_mm3.reason = None;
        }
        if let Some(reason) = &params.manual_reason {
            part.volume_mm3.source = MeasurementSource::Manual;
            part.volume_mm3.reason = Some(reason.clone());
        }
        if let Some(confirmed) = params.volume_confirmed {
            part.volume_mm3.review = if confirmed { ReviewState::Confirmed } else { ReviewState::Pending };
        }
        if let Some(value) = &params.material_version_id {
            part.material_version_id = optional_uuid(cmd, "material_version_id", value)?;
        }
        if let Some(value) = &params.allowance_rate {
            part.allowance_rate = optional_decimal(cmd, "allowance_rate", value)?;
        }
        if params.cycle_seconds.is_some()
            || params.cavities.is_some()
            || params.production_rate_version_id.is_some()
            || params.setup_hours.is_some()
            || params.setup_rate_version_id.is_some()
        {
            let operation = part.operation.get_or_insert(OperationInputs {
                cycle_seconds: None,
                cycle_basis: CycleBasis::PerPiece,
                cavities: None,
                production_rate_version_id: None,
                setup_hours: None,
                setup_rate_version_id: None,
            });
            if let Some(value) = &params.cycle_seconds {
                operation.cycle_seconds = optional_decimal(cmd, "cycle_seconds", value)?;
            }
            if let Some(value) = params.cavities {
                operation.cavities = (value > 0).then_some(value);
            }
            if let Some(value) = &params.production_rate_version_id {
                operation.production_rate_version_id = optional_uuid(cmd, "production_rate_version_id", value)?;
            }
            if let Some(value) = &params.setup_hours {
                operation.setup_hours = optional_decimal(cmd, "setup_hours", value)?;
            }
            if let Some(value) = &params.setup_rate_version_id {
                operation.setup_rate_version_id = optional_uuid(cmd, "setup_rate_version_id", value)?;
            }
        }
        if let Some(value) = &params.material_overhead_rate {
            draft.material_overhead_rate = optional_decimal(cmd, "material_overhead_rate", value)?;
        }
    }
    draft.validate().map_err(|error| bad(cmd, error.to_string()))?;
    state.draft = Some(draft);
    state.generation = state.generation.wrapping_add(1);
    state.save = SaveState::Dirty;
    if matches!(state.result, ResultState::Current | ResultState::Incomplete) {
        state.result = ResultState::Stale;
    }
    state.error = None;
    state_reply_value(s)
}

fn save(s: &mut Session, _p: &Value) -> Result<Value> {
    s.solidbom_save(false)?;
    job_reply(s)
}

fn calculate(s: &mut Session, _p: &Value) -> Result<Value> {
    s.solidbom_calculate()?;
    job_reply(s)
}

fn retry(s: &mut Session, _p: &Value) -> Result<Value> {
    s.solidbom_retry()?;
    job_reply(s)
}

fn reopen(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "solidbom.reopen";
    let params: ReopenParams = serde_json::from_value(p.clone()).map_err(|error| bad(cmd, error.to_string()))?;
    if s.review.is_some() {
        return Err(bad(cmd, "close the current reviewed source before reopening another"));
    }
    let project_id = Uuid::parse_str(&params.project_id).map_err(|_| bad(cmd, "`project_id` must be a UUID"))?;
    s.solidbom = SolidBomState { phase: Phase::Importing, ..Default::default() };
    s.submit_solidbom(ClientRequest::LoadProject(project_id), crate::solidbom::state::PendingKind::LoadProject, None, None)?;
    job_reply(s)
}

fn state_command(s: &mut Session, _p: &Value) -> Result<Value> {
    state_reply_value(s)
}

fn review(s: &mut Session, _p: &Value) -> Result<Value> {
    state_reply_value(s)
}

fn state_reply_value(s: &Session) -> Result<Value> {
    serde_json::to_value(state_reply(s)).map_err(|error| EngineError::Other(error.to_string()))
}

fn job_reply(s: &Session) -> Result<Value> {
    let request_id = match &s.solidbom.save {
        SaveState::Saving { request_id } => Some(request_id.clone()),
        _ => match &s.solidbom.result {
            ResultState::Calculating { request_id } => Some(request_id.clone()),
            _ => None,
        },
    };
    serde_json::to_value(JobReply { request_id, state: state_reply(s) }).map_err(|error| EngineError::Other(error.to_string()))
}

fn state_reply(s: &Session) -> StateReply {
    let state = &s.solidbom;
    StateReply {
        phase: match state.phase {
            Phase::Idle => "idle",
            Phase::Importing => "importing",
            Phase::Reviewing => "reviewing",
        }
        .into(),
        save: match &state.save {
            SaveState::Clean => "clean",
            SaveState::Dirty => "dirty",
            SaveState::Saving { .. } => "saving",
            SaveState::SaveFailed { .. } => "save_failed",
        }
        .into(),
        result: match &state.result {
            ResultState::NoResult => "no_result",
            ResultState::Calculating { .. } => "calculating",
            ResultState::Current => "current",
            ResultState::Stale => "stale",
            ResultState::Incomplete => "incomplete",
        }
        .into(),
        project_id: state.project_id().map(|id| id.to_string()),
        version: state.version(),
        definitions: state.draft.as_ref().map_or(0, |draft| draft.analysis.definitions.len()),
        occurrences: state.draft.as_ref().map_or(0, |draft| draft.analysis.occurrences.len()),
        issues: state.issue_codes(),
        calculations: state.summaries.iter().map(|summary| summary.calculation_id.to_string()).collect(),
        current_calculation: state.current.as_ref().map(|snapshot| snapshot.calculation_id.to_string()),
        assembly_subtotal: state.current.as_ref().and_then(|snapshot| snapshot.outputs.assembly_subtotal).map(|amount| amount.to_string()),
        per_part_subtotal: state
            .current
            .as_ref()
            .and_then(|snapshot| snapshot.outputs.parts.first())
            .and_then(|part| part.per_part_subtotal)
            .map(|amount| amount.to_string()),
        setup_total: state.current.as_ref().and_then(|snapshot| snapshot.outputs.setup_total).map(|amount| amount.to_string()),
        error: state.error.clone(),
        notice: state.notice.clone(),
        needs_service: state.needs_service,
    }
}

fn decimal(cmd: &str, field: &str, text: &str) -> Result<Option<Decimal>> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    Decimal::from_str(text).map(Some).map_err(|_| bad(cmd, format!("`{field}` must be an exact decimal string")))
}

fn optional_decimal(cmd: &str, field: &str, text: &str) -> Result<Option<Decimal>> {
    decimal(cmd, field, text)
}

fn optional_uuid(cmd: &str, field: &str, text: &str) -> Result<Option<Uuid>> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    Uuid::parse_str(text).map(Some).map_err(|_| bad(cmd, format!("`{field}` must be a UUID")))
}
