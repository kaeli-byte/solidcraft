//! SolidBOM's commands: import a reviewed STEP, edit its inputs, save, calculate, load a saved
//! calculation, export an internal document, retry and reopen. Every one is a normal command, so the panel, control channel, MCP and scripts reach
//! the same behaviour; none opens a dialog.

use std::collections::BTreeSet;
use std::str::FromStr;

use rust_decimal::Decimal;
use serde_json::Value;
use solidbom_contract::{
    AnalysisKey, CatalogAttribute, CategoryBasis, CategoryCalculation, CostCategory, CostCenterPurpose, CreateAppConfigVersion,
    CreateComponentVersion, CreateMaterialVersion, CreateRateVersion, CycleBasis, DirectCostInput, DraftProject, ExportBaselineRef, ExportFormat,
    ExportRequest, MAX_COST_CATEGORIES, MeasurementSource, OperationInputs, ReviewState, SourceRef, Sourcing,
};
use uuid::Uuid;

use super::CommandSpec;
use crate::params::bad;
use crate::schema::solidbom::{
    AppConfigVersionParams, BatchInputsParams, CalculationParams, CategoryCalculationParams, ComponentVersionParams, CostCategoryParams,
    CostCenterVersionParams, EmptyParams, ExportFormatParams, ExportParams, ImportParams, ImportReply, InputsParams, JobReply, MaterialVersionParams,
    PartsReply, ReopenParams, SharedReply, SharedVersionRow, StateReply,
};
use crate::schema::{TypedContract, output_schema_for};
use crate::solidbom::EXTRACTOR_VERSION;
use crate::solidbom::host::ClientRequest;
use crate::solidbom::jobs::{AnalysisPurpose, AnalysisRequest};
use crate::solidbom::state::{PendingKind, Phase, ResultState, SaveState};
use crate::{EngineError, Result, Session};

/// The service's configured upload limit; a prototype resource limit.
const MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;

const IMPORT_CONTRACT: TypedContract<ImportParams> = TypedContract::new();
const INPUTS_CONTRACT: TypedContract<InputsParams> = TypedContract::new();
const BATCH_INPUTS_CONTRACT: TypedContract<BatchInputsParams> = TypedContract::new();
const EMPTY_CONTRACT: TypedContract<EmptyParams> = TypedContract::new();
const REOPEN_CONTRACT: TypedContract<ReopenParams> = TypedContract::new();
const CALCULATION_CONTRACT: TypedContract<CalculationParams> = TypedContract::new();
const EXPORT_CONTRACT: TypedContract<ExportParams> = TypedContract::new();
const MATERIAL_VERSION_CONTRACT: TypedContract<MaterialVersionParams> = TypedContract::new();
const COST_CENTER_VERSION_CONTRACT: TypedContract<CostCenterVersionParams> = TypedContract::new();
const COMPONENT_VERSION_CONTRACT: TypedContract<ComponentVersionParams> = TypedContract::new();
const APP_CONFIG_VERSION_CONTRACT: TypedContract<AppConfigVersionParams> = TypedContract::new();

#[path = "solidbom/parts.rs"]
mod parts;

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("solidbom.import", "Import STEP for Review", crate::checked!("solidbom.import", import))
        .icon("bom")
        .noundo()
        .contract(&IMPORT_CONTRACT)
        .output(output_schema_for::<ImportReply>)
        .params("path: a local STEP file to import and review → project_id, state (importing or reviewing), definitions, occurrences, issues, needs_service"),
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
    CommandSpec::new("solidbom.inputs.batch", "Apply SolidBOM Inputs", crate::checked!("solidbom.inputs.batch", batch_inputs))
        .icon("settings")
        .noundo()
        .contract(&BATCH_INPUTS_CONTRACT)
        .output(output_schema_for::<StateReply>)
        .params("parts: reviewed part patches; material_overhead_rate?: exact decimal string → the review state, applied atomically"),
    CommandSpec::new("solidbom.parts", "SolidBOM Parts", crate::checked!("solidbom.parts", parts_command))
        .icon("bom")
        .noundo()
        .contract(&EMPTY_CONTRACT)
        .output(output_schema_for::<PartsReply>)
        .params("→ typed reviewed part rows, exact occurrence paths, input provenance and optional costs"),
    CommandSpec::new("solidbom.save", "Save SolidBOM Project", crate::checked!("solidbom.save", save))
        .enabled(save_available)
        .icon("save")
        .noundo()
        .contract(&EMPTY_CONTRACT)
        .output(output_schema_for::<JobReply>)
        .params("→ request_id, state"),
    CommandSpec::new("solidbom.calculate", "Calculate SolidBOM Direct Cost", crate::checked!("solidbom.calculate", calculate))
        .enabled(calculate_available)
        .icon("measure")
        .noundo()
        .contract(&EMPTY_CONTRACT)
        .output(output_schema_for::<JobReply>)
        .params("saves unsaved inputs first → request_id, state"),
    CommandSpec::new("solidbom.calculation", "Load SolidBOM Calculation", crate::checked!("solidbom.calculation", calculation))
        .enabled(calculation_available)
        .icon("list")
        .noundo()
        .contract(&CALCULATION_CONTRACT)
        .output(output_schema_for::<JobReply>)
        .params("calculation_id: one saved calculation of the open project → request_id, state"),
    CommandSpec::new("solidbom.export", "Export SolidBOM Result", crate::checked!("solidbom.export", export))
        .icon("save")
        .noundo()
        .contract(&EXPORT_CONTRACT)
        .output(output_schema_for::<JobReply>)
        .params(
            "path: the local file the rendered document is written to; format?: xlsx (the default) or pdf; \
             accepted_baseline_id? and accepted_baseline_sequence? (both or neither); calculation_id?: one saved draft \
             calculation; scenarios?: saved commercial scenario ids, ascending → job id, state",
        ),
    CommandSpec::new("solidbom.retry", "Retry SolidBOM Request", crate::checked!("solidbom.retry", retry))
        .enabled(retry_available)
        .icon("activate")
        .noundo()
        .contract(&EMPTY_CONTRACT)
        .output(output_schema_for::<JobReply>)
        .params("resubmits the failed request with the same id and payload → request_id, state"),
    CommandSpec::new("solidbom.reopen", "Reopen SolidBOM Project", crate::checked!("solidbom.reopen", reopen))
        .enabled(reopen_available)
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
    CommandSpec::new("solidbom.shared", "SolidBOM Administration", crate::checked!("solidbom.shared", shared))
        .icon("list")
        .noundo()
        .contract(&EMPTY_CONTRACT)
        .output(output_schema_for::<SharedReply>)
        .params(
            "reloads the caller identity and every shared material, cost-center, component and app-configuration version → identity, \
             permissions, version chains, recent reference changes, pending jobs",
        ),
    CommandSpec::new(
        "solidbom.shared.material.version",
        "Publish Material Version",
        crate::checked!("solidbom.shared.material.version", material_version),
    )
    .icon("save")
    .noundo()
    .contract(&MATERIAL_VERSION_CONTRACT)
    .output(output_schema_for::<SharedReply>)
    .params(
        "material_id? (blank creates a record), name, density_g_cm3 (exact decimal > 0), price_per_kg? (blank = missing, 0 = explicit \
         zero), currency?, source_reference?, expected_version? → the shared administration state",
    ),
    CommandSpec::new(
        "solidbom.shared.cost_center.version",
        "Publish Cost-Center Rate",
        crate::checked!("solidbom.shared.cost_center.version", cost_center_version),
    )
    .icon("save")
    .noundo()
    .contract(&COST_CENTER_VERSION_CONTRACT)
    .output(output_schema_for::<SharedReply>)
    .params(
        "cost_center_id? (blank creates a record), name, purpose? (production or setup), rate_per_hour? (blank = missing, 0 = explicit \
         zero), currency?, source_reference?, expected_version? → the shared administration state",
    ),
    CommandSpec::new(
        "solidbom.shared.component.version",
        "Publish Catalog Component",
        crate::checked!("solidbom.shared.component.version", component_version),
    )
    .icon("save")
    .noundo()
    .contract(&COMPONENT_VERSION_CONTRACT)
    .output(output_schema_for::<SharedReply>)
    .params(
        "component_id? (blank creates a record), name, specification, price? (blank = missing, 0 = explicit zero), currency?, \
         source_reference?, attributes?: [{key, value}] administrator-declared matching attributes (optional; empty declares none), \
         expected_version? → the shared administration state",
    ),
    CommandSpec::new(
        "solidbom.shared.app_config.version",
        "Publish App Configuration",
        crate::checked!("solidbom.shared.app_config.version", app_config_version),
    )
    .icon("save")
    .noundo()
    .contract(&APP_CONFIG_VERSION_CONTRACT)
    .output(output_schema_for::<SharedReply>)
    .params(
        "config_id? (blank creates a record), currency, categories (code, name, calculation kind direct/fixed/percentage, \
         included_in_ebit), source_reference?, expected_version? → the shared administration state",
    ),
];

/// Import a STEP file and start its background analysis. The command answers at once with the
/// project identity it will publish under; the review state reports `importing` until the
/// analysis verifies the source, and a failure is reported in the state rather than the reply.
fn import(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "solidbom.import";
    let params: ImportParams = serde_json::from_value(p.clone()).map_err(|error| bad(cmd, error.to_string()))?;
    if s.review.is_some() {
        return Err(bad(cmd, "a reviewed source is already open in this session"));
    }
    if s.solidbom_analysis.is_some() || s.solidbom.phase == Phase::Importing {
        return Err(bad(cmd, "a SolidBOM source is already being opened"));
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
    let key = AnalysisKey {
        source_sha256: sha256.clone(),
        engine_build: env!("CARGO_PKG_VERSION").to_string(),
        extractor_version: EXTRACTOR_VERSION.to_string(),
    };
    let original_name = path.file_name().and_then(|name| name.to_str()).unwrap_or("source.step").to_string();
    let source = SourceRef {
        source_id: Uuid::new_v5(&Uuid::NAMESPACE_OID, format!("{sha256}:source").as_bytes()),
        sha256: sha256.clone(),
        byte_len: u64::try_from(bytes.len()).map_err(|_| bad(cmd, "the source is too large"))?,
        original_name,
    };
    let project_id = crate::solidbom::state::project_id(&sha256);
    // The import binds review mode atomically, after the source verified: the design, the
    // manifest and the policy all arrive together. A replaced design or a failed analysis
    // publishes nothing, so the live session is exactly as it was.
    let reviewed_path = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
    let request = AnalysisRequest {
        bytes,
        original_name: source.original_name.clone(),
        key,
        purpose: AnalysisPurpose::Import { source, source_path: reviewed_path },
        command_check: s.command_check(),
        validation: s.validation(),
        param_check: s.param_check,
        auto_project: s.auto_project,
    };
    s.start_solidbom_import_analysis(request).map_err(|error| bad(cmd, error.to_string()))?;
    serde_json::to_value(ImportReply {
        project_id: project_id.to_string(),
        definitions: 0,
        occurrences: 0,
        part_version_ids: Vec::new(),
        issues: Vec::new(),
        needs_service: s.solidbom_host.is_none(),
        state: Some("importing".into()),
    })
    .map_err(|error| EngineError::Other(error.to_string()))
}

fn inputs(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "solidbom.inputs";
    let params: InputsParams = serde_json::from_value(p.clone()).map_err(|error| bad(cmd, error.to_string()))?;
    let mut draft = s.solidbom.draft.clone().ok_or_else(|| bad(cmd, "no SolidBOM project is open"))?;
    apply_inputs(cmd, &mut draft, &params)?;
    commit_inputs(s, cmd, draft)
}

fn batch_inputs(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "solidbom.inputs.batch";
    let parts = p.get("parts").and_then(Value::as_array);
    if parts.is_some_and(|parts| parts.len() > solidbom_contract::MAX_PARTS) {
        return Err(bad(cmd, "too many part patches"));
    }
    // Project overhead belongs to the batch, not a part patch. The patch's serde reader tolerates
    // unknown keys in the default check mode, so the rule is checked here explicitly.
    if parts.is_some_and(|parts| parts.iter().any(|patch| patch.get("material_overhead_rate").is_some())) {
        return Err(bad(cmd, "`material_overhead_rate` belongs to the batch, not a part patch"));
    }
    let params: BatchInputsParams = serde_json::from_value(p.clone()).map_err(|error| bad(cmd, error.to_string()))?;
    if params.parts.is_empty() && params.material_overhead_rate.is_none() {
        return Err(bad(cmd, "provide part patches or project material overhead"));
    }
    let mut draft = s.solidbom.draft.clone().ok_or_else(|| bad(cmd, "no SolidBOM project is open"))?;
    let mut ids = BTreeSet::new();
    for patch in params.parts {
        let id = Uuid::parse_str(&patch.part_version_id).map_err(|_| bad(cmd, "`part_version_id` must be a UUID"))?;
        if !ids.insert(id) {
            return Err(bad(cmd, format!("duplicate part patch: {id}")));
        }
        apply_inputs(cmd, &mut draft, &patch.into())?;
    }
    if let Some(value) = &params.material_overhead_rate {
        draft.material_overhead_rate = optional_decimal(cmd, "material_overhead_rate", value)?;
    }
    commit_inputs(s, cmd, draft)
}

fn apply_inputs(cmd: &str, draft: &mut DraftProject, params: &InputsParams) -> Result<()> {
    let part_version_id = Uuid::parse_str(&params.part_version_id).map_err(|_| bad(cmd, "`part_version_id` must be a UUID"))?;
    let part = draft.parts.iter_mut().find(|part| part.part_version_id == part_version_id).ok_or_else(|| bad(cmd, "no reviewed part has that id"))?;
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
    Ok(())
}

fn commit_inputs(s: &mut Session, cmd: &str, draft: DraftProject) -> Result<Value> {
    draft.validate().map_err(|error| bad(cmd, error.to_string()))?;
    let state = &mut s.solidbom;
    state.draft = Some(draft);
    state.generation = state.generation.wrapping_add(1);
    state.save = SaveState::Dirty;
    if matches!(state.result, ResultState::Current | ResultState::Incomplete) {
        state.result = ResultState::Stale;
    }
    state.error = None;
    state_reply_value(s)
}

fn parts_command(s: &mut Session, _p: &Value) -> Result<Value> {
    serde_json::to_value(s.solidbom_parts()?).map_err(|error| EngineError::Other(error.to_string()))
}

impl Session {
    /// Read the same typed parts projection as `solidbom.parts`, without executing a command.
    pub fn solidbom_parts(&self) -> Result<PartsReply> {
        parts::project(self)
    }
}

fn service_available(s: &Session) -> std::result::Result<(), String> {
    let Some(host) = &s.solidbom_host else {
        return Err(crate::solidbom::state::SERVICE_MISSING.into());
    };
    let capabilities = host.capabilities();
    if !capabilities.service {
        return Err(crate::solidbom::state::SERVICE_MISSING.into());
    }
    if capabilities.contract_version != solidbom_contract::SCHEMA_VERSION {
        return Err(format!(
            "the SolidBOM service speaks contract version {}; this build reads version {}",
            capabilities.contract_version,
            solidbom_contract::SCHEMA_VERSION
        ));
    }
    Ok(())
}

fn reopen_available(s: &Session) -> std::result::Result<(), String> {
    if s.review.is_some() {
        return Err("close the current reviewed source before reopening another".into());
    }
    if s.solidbom.phase == Phase::Importing {
        return Err("a SolidBOM project is already being opened".into());
    }
    service_available(s)
}

fn save_available(s: &Session) -> std::result::Result<(), String> {
    if s.solidbom.phase == Phase::Importing {
        return Err("the SolidBOM project is still opening".into());
    }
    if s.solidbom.phase != Phase::Reviewing || s.solidbom.draft.is_none() {
        return Err("no SolidBOM project is open".into());
    }
    service_available(s)?;
    if s.solidbom.version() == 0 {
        return Err("the SolidBOM project is still being created".into());
    }
    if matches!(s.solidbom.save, SaveState::Saving { .. }) {
        return Err("a SolidBOM save is already in progress".into());
    }
    Ok(())
}

fn calculate_available(s: &Session) -> std::result::Result<(), String> {
    save_available(s)?;
    if matches!(s.solidbom.result, ResultState::Calculating { .. }) {
        return Err("a SolidBOM calculation is already in progress".into());
    }
    if s.solidbom.reference.is_none() {
        return Err("SolidBOM reference data is not available yet".into());
    }
    Ok(())
}

fn calculation_available(s: &Session) -> std::result::Result<(), String> {
    if s.solidbom.phase != Phase::Reviewing || s.solidbom.draft.is_none() {
        return Err("no SolidBOM project is open".into());
    }
    service_available(s)?;
    if s.solidbom.pending.values().any(|pending| matches!(pending.kind, crate::solidbom::state::PendingKind::LoadCalculation)) {
        return Err("a SolidBOM calculation is already being loaded".into());
    }
    Ok(())
}

fn retry_available(s: &Session) -> std::result::Result<(), String> {
    service_available(s)?;
    let Some(failed) = &s.solidbom.failed else {
        return Err("there is no failed SolidBOM request to retry".into());
    };
    if !failed.retryable {
        return Err("this SolidBOM request cannot be retried; reopen the project or correct the inputs".into());
    }
    Ok(())
}

fn save(s: &mut Session, _p: &Value) -> Result<Value> {
    s.solidbom_save(false)?;
    job_reply(s)
}

fn calculate(s: &mut Session, _p: &Value) -> Result<Value> {
    s.solidbom_calculate()?;
    job_reply(s)
}

fn calculation(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "solidbom.calculation";
    let params: CalculationParams = serde_json::from_value(p.clone()).map_err(|error| bad(cmd, error.to_string()))?;
    let calculation_id = Uuid::parse_str(&params.calculation_id).map_err(|_| bad(cmd, "`calculation_id` must be a UUID"))?;
    s.solidbom.error = None;
    s.solidbom.failed = None;
    s.submit_solidbom(ClientRequest::LoadCalculation(calculation_id), crate::solidbom::state::PendingKind::LoadCalculation, None, None)?;
    job_reply(s)
}

/// Render one internal export of one explicitly selected saved result. The command answers with
/// the host job id at once; the controller writes the file when the host answers, and the
/// completion notice names the written path. An export is a pure read: it appends nothing.
fn export(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "solidbom.export";
    let params: ExportParams = serde_json::from_value(p.clone()).map_err(|error| bad(cmd, error.to_string()))?;
    if params.path.trim().is_empty() {
        return Err(bad(cmd, "`path` is the local file the export is written to"));
    }
    let path = std::path::PathBuf::from(&params.path);
    if path.is_dir() {
        return Err(bad(cmd, format!("{} is a folder, not a file", path.display())));
    }
    let project_id = s.solidbom.project_id().ok_or_else(|| bad(cmd, "no SolidBOM project is open"))?;
    let format = match params.format {
        ExportFormatParams::Xlsx => ExportFormat::Xlsx,
        ExportFormatParams::Pdf => ExportFormat::Pdf,
    };
    let baseline = export_baseline(s, cmd, &params)?;
    let scenarios = export_scenarios(cmd, &params.scenarios)?;
    let request = ExportRequest { project_id, baseline, scenarios, format };
    request.validate().map_err(|error| bad(cmd, error.to_string()))?;
    let job = s.solidbom_export(request, path)?;
    // The job id is the only identity this request has: the contract's export request carries no
    // request id, because an export appends nothing and needs no idempotency key.
    serde_json::to_value(JobReply { request_id: Some(job.to_string()), state: state_reply(s) }).map_err(|error| EngineError::Other(error.to_string()))
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
    if s.solidbom_analysis.is_some() || s.solidbom.phase == Phase::Importing {
        return Err(bad(cmd, "a SolidBOM source is already being opened"));
    }
    let project_id = Uuid::parse_str(&params.project_id).map_err(|_| bad(cmd, "`project_id` must be a UUID"))?;
    // An absent service is a capability error: the open review stays as it was.
    service_available(s).map_err(EngineError::Capability)?;
    let previous = std::mem::take(&mut s.solidbom);
    s.solidbom.phase = Phase::Importing;
    if let Err(error) = s.submit_solidbom(ClientRequest::LoadProject(project_id), crate::solidbom::state::PendingKind::LoadProject, None, None) {
        s.solidbom = previous;
        return Err(error);
    }
    job_reply(s)
}

fn state_command(s: &mut Session, _p: &Value) -> Result<Value> {
    state_reply_value(s)
}

fn review(s: &mut Session, _p: &Value) -> Result<Value> {
    state_reply_value(s)
}

/// Load (or reload) the shared records and open the administration surface in the window.
fn shared(s: &mut Session, _p: &Value) -> Result<Value> {
    s.solidbom_load_shared()?;
    shared_reply_value(s)
}

/// Publish one material version. A blank id starts a new material record; every version carries
/// its own density, and a blank price stays missing rather than becoming zero.
fn material_version(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "solidbom.shared.material.version";
    let params: MaterialVersionParams = serde_json::from_value(p.clone()).map_err(|error| bad(cmd, error.to_string()))?;
    let request_id = s.next_solidbom_request_id();
    let create = CreateMaterialVersion {
        material_id: record_id(cmd, "material_id", params.material_id.as_deref(), &request_id)?,
        expected_version: params.expected_version.unwrap_or(0),
        name: params.name,
        density_g_cm3: required_decimal(cmd, "density_g_cm3", &params.density_g_cm3)?,
        price_per_kg: optional_decimal(cmd, "price_per_kg", params.price_per_kg.as_deref().unwrap_or_default())?,
        currency: currency(cmd, params.currency.as_deref()),
        source_reference: source_reference(params.source_reference),
        request_id,
    };
    create.validate().map_err(|error| bad(cmd, error.to_string()))?;
    s.solidbom_publish_material(create)?;
    shared_reply_value(s)
}

/// Publish one cost-center rate version.
fn cost_center_version(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "solidbom.shared.cost_center.version";
    let params: CostCenterVersionParams = serde_json::from_value(p.clone()).map_err(|error| bad(cmd, error.to_string()))?;
    let purpose = match params.purpose.as_deref().map(str::trim).filter(|value| !value.is_empty()).unwrap_or("production") {
        "production" => CostCenterPurpose::Production,
        "setup" => CostCenterPurpose::Setup,
        other => return Err(bad(cmd, format!("`purpose` must be production or setup, not `{other}`"))),
    };
    let request_id = s.next_solidbom_request_id();
    let create = CreateRateVersion {
        cost_center_id: record_id(cmd, "cost_center_id", params.cost_center_id.as_deref(), &request_id)?,
        expected_version: params.expected_version.unwrap_or(0),
        name: params.name,
        purpose,
        rate_per_hour: optional_decimal(cmd, "rate_per_hour", params.rate_per_hour.as_deref().unwrap_or_default())?,
        currency: currency(cmd, params.currency.as_deref()),
        source_reference: source_reference(params.source_reference),
        request_id,
    };
    create.validate().map_err(|error| bad(cmd, error.to_string()))?;
    s.solidbom_publish_rate(create)?;
    shared_reply_value(s)
}

/// Publish one catalog component version.
fn component_version(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "solidbom.shared.component.version";
    let params: ComponentVersionParams = serde_json::from_value(p.clone()).map_err(|error| bad(cmd, error.to_string()))?;
    let request_id = s.next_solidbom_request_id();
    let attributes = params
        .attributes
        .into_iter()
        .map(|attribute| CatalogAttribute { key: attribute.key.trim().to_string(), value: attribute.value.trim().to_string() })
        .collect();
    let create = CreateComponentVersion {
        component_id: record_id(cmd, "component_id", params.component_id.as_deref(), &request_id)?,
        expected_version: params.expected_version.unwrap_or(0),
        name: params.name,
        specification: params.specification,
        price: optional_decimal(cmd, "price", params.price.as_deref().unwrap_or_default())?,
        currency: currency(cmd, params.currency.as_deref()),
        source_reference: source_reference(params.source_reference),
        attributes,
        request_id,
    };
    create.validate().map_err(|error| bad(cmd, error.to_string()))?;
    s.solidbom_publish_component(create)?;
    shared_reply_value(s)
}

/// Publish one app cost configuration version. The contract's validator refuses an ambiguous
/// percentage base, a duplicate or circular inclusion, and a direct category selected for EBIT.
fn app_config_version(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "solidbom.shared.app_config.version";
    let params: AppConfigVersionParams = serde_json::from_value(p.clone()).map_err(|error| bad(cmd, error.to_string()))?;
    if params.categories.len() > MAX_COST_CATEGORIES {
        return Err(bad(cmd, format!("at most {MAX_COST_CATEGORIES} cost categories are allowed")));
    }
    let categories = params.categories.into_iter().map(|category| category_from(cmd, category)).collect::<Result<Vec<_>>>()?;
    let request_id = s.next_solidbom_request_id();
    let create = CreateAppConfigVersion {
        config_id: record_id(cmd, "config_id", params.config_id.as_deref(), &request_id)?,
        expected_version: params.expected_version.unwrap_or(0),
        currency: currency(cmd, Some(params.currency.as_str())),
        categories,
        source_reference: source_reference(params.source_reference),
        request_id,
    };
    create.validate().map_err(|error| bad(cmd, error.to_string()))?;
    s.solidbom_publish_app_config(create)?;
    shared_reply_value(s)
}

/// A shared record id: an explicit UUID, or a stable new one derived from the request id. The
/// derived id is kept in the submitted request, so an explicit retry publishes under the same id.
fn record_id(cmd: &str, field: &str, text: Option<&str>, request_id: &str) -> Result<Uuid> {
    let Some(text) = text.map(str::trim).filter(|text| !text.is_empty()) else {
        return Ok(Uuid::new_v5(&Uuid::NAMESPACE_OID, format!("{request_id}:{field}").as_bytes()));
    };
    Uuid::parse_str(text).map_err(|_| bad(cmd, format!("`{field}` must be a UUID")))
}

/// The publishing currency: an explicit three-letter code, or the prototype's CNY.
fn currency(_cmd: &str, text: Option<&str>) -> String {
    text.map(str::trim).filter(|text| !text.is_empty()).unwrap_or("CNY").to_string()
}

/// An optional source/version reference: blank means none, never an empty string.
fn source_reference(text: Option<String>) -> Option<String> {
    text.map(|text| text.trim().to_string()).filter(|text| !text.is_empty())
}

/// An exact decimal parameter that is required: density, a fixed amount or a percentage rate.
fn required_decimal(cmd: &str, field: &str, text: &str) -> Result<Decimal> {
    if text.trim().is_empty() {
        return Err(bad(cmd, format!("`{field}` is required and must be an exact decimal string")));
    }
    Decimal::from_str(text).map_err(|_| bad(cmd, format!("`{field}` must be an exact decimal string")))
}

/// One category in an app-configuration publish, converted from its wire form.
fn category_from(cmd: &str, category: CostCategoryParams) -> Result<CostCategory> {
    let calculation = match category.calculation {
        CategoryCalculationParams::Direct { input } => {
            let input = match input.trim() {
                "material" => DirectCostInput::Material,
                "production" => DirectCostInput::Production,
                "setup" => DirectCostInput::Setup,
                other => return Err(bad(cmd, format!("`categories.calculation.input` must be material, production or setup, not `{other}`"))),
            };
            CategoryCalculation::Direct { input }
        }
        CategoryCalculationParams::Fixed { amount } => {
            CategoryCalculation::Fixed { amount: required_decimal(cmd, "categories.calculation.amount", &amount)? }
        }
        CategoryCalculationParams::Percentage { basis, rate } => CategoryCalculation::Percentage {
            basis: CategoryBasis { direct_cost: basis.direct_cost, categories: basis.categories },
            rate: required_decimal(cmd, "categories.calculation.rate", &rate)?,
        },
    };
    Ok(CostCategory { code: category.code, name: category.name, calculation, included_in_ebit: category.included_in_ebit })
}

fn shared_reply_value(s: &Session) -> Result<Value> {
    serde_json::to_value(shared_reply(s)).map_err(|error| EngineError::Other(error.to_string()))
}

/// What the administration surface shows: the identity, every version chain and the newest
/// reference-change facts, all read from the engine state rather than from a second request.
fn shared_reply(s: &Session) -> SharedReply {
    let state = &s.solidbom;
    let identity = state.identity.as_ref();
    let mut pending: Vec<String> = state
        .pending
        .values()
        .filter_map(|pending| match pending.kind {
            PendingKind::Identity => Some("identity"),
            PendingKind::SharedData => Some("shared_data"),
            PendingKind::PublishMaterial => Some("publish_material"),
            PendingKind::PublishRate => Some("publish_rate"),
            PendingKind::PublishComponent => Some("publish_component"),
            PendingKind::PublishAppConfig => Some("publish_app_config"),
            _ => None,
        })
        .map(str::to_string)
        .collect();
    pending.sort();
    pending.dedup();
    let (materials, cost_centers, components, app_config, changes) = match &state.shared {
        Some(view) => (
            view.materials.iter().map(material_row).collect(),
            view.cost_centers.iter().map(rate_row).collect(),
            view.components.iter().map(component_row).collect(),
            view.app_config.as_ref().map(app_config_row),
            view.changes.iter().map(change_row).collect(),
        ),
        None => (Vec::new(), Vec::new(), Vec::new(), None, Vec::new()),
    };
    SharedReply {
        identity_id: identity.map(|identity| identity.identity_id.to_string()),
        identity_name: identity.map(|identity| identity.name.clone()),
        permissions: identity
            .map(|identity| identity.permissions.iter().map(|permission| permission.as_str().to_string()).collect())
            .unwrap_or_default(),
        administrator: state.is_administrator(),
        materials,
        cost_centers,
        components,
        app_config,
        changes,
        pending,
        error: state.error.clone(),
        notice: state.notice.clone(),
    }
}

fn material_row(version: &solidbom_contract::MaterialVersion) -> SharedVersionRow {
    SharedVersionRow {
        record_id: version.material_id.to_string(),
        version_id: version.material_version_id.to_string(),
        version: version.version,
        name: version.name.clone(),
        detail: format!("{} g/cm3", version.density_g_cm3),
        price: version.price_per_kg.map(|price| price.to_string()),
        currency: version.currency.clone(),
        source_reference: version.source_reference.clone(),
        created_at: version.created_at.clone(),
    }
}

fn rate_row(version: &solidbom_contract::RateVersion) -> SharedVersionRow {
    let purpose = match version.purpose {
        CostCenterPurpose::Production => "production",
        CostCenterPurpose::Setup => "setup",
    };
    SharedVersionRow {
        record_id: version.cost_center_id.to_string(),
        version_id: version.rate_version_id.to_string(),
        version: version.version,
        name: version.name.clone(),
        detail: purpose.to_string(),
        price: version.rate_per_hour.map(|rate| rate.to_string()),
        currency: version.currency.clone(),
        source_reference: version.source_reference.clone(),
        created_at: version.created_at.clone(),
    }
}

fn component_row(version: &solidbom_contract::CatalogComponentVersion) -> SharedVersionRow {
    SharedVersionRow {
        record_id: version.component_id.to_string(),
        version_id: version.component_version_id.to_string(),
        version: version.version,
        name: version.name.clone(),
        detail: version.specification.clone(),
        price: version.price.map(|price| price.to_string()),
        currency: version.currency.clone(),
        source_reference: version.source_reference.clone(),
        created_at: version.created_at.clone(),
    }
}

fn app_config_row(config: &solidbom_contract::AppConfigVersion) -> crate::schema::solidbom::SharedAppConfigRow {
    crate::schema::solidbom::SharedAppConfigRow {
        config_id: config.config_id.to_string(),
        version_id: config.config_version_id.to_string(),
        version: config.version,
        currency: config.currency.clone(),
        categories: config.categories.iter().map(|category| category.code.clone()).collect(),
        source_reference: config.source_reference.clone(),
        created_at: config.created_at.clone(),
    }
}

fn change_row(change: &solidbom_contract::ReferenceChange) -> crate::schema::solidbom::SharedChangeRow {
    crate::schema::solidbom::SharedChangeRow {
        change_id: change.change_id.to_string(),
        kind: change.kind.as_str().to_string(),
        record_id: change.record_id.to_string(),
        version: change.version,
        summary: change.summary.clone(),
        changed_by: change.changed_by.clone(),
        changed_at: change.changed_at.clone(),
    }
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
        calculations: {
            let mut ids: std::collections::BTreeSet<String> = state.summaries.iter().map(|summary| summary.calculation_id.to_string()).collect();
            ids.extend(state.calculations.iter().map(|calculation| calculation.calculation_id.to_string()));
            ids.into_iter().collect()
        },
        current_calculation: state.current.as_ref().map(|snapshot| snapshot.calculation_id.to_string()),
        assembly_subtotal: state.current.as_ref().and_then(|snapshot| snapshot.outputs.assembly_subtotal).map(|amount| amount.to_string()),
        per_part_subtotal: state
            .current
            .as_ref()
            .and_then(|snapshot| snapshot.outputs.parts.first())
            .and_then(|part| part.per_part_subtotal)
            .map(|amount| amount.to_string()),
        setup_total: state.current.as_ref().and_then(|snapshot| snapshot.outputs.setup_total).map(|amount| amount.to_string()),
        volume_mm3: state.draft.as_ref().and_then(|draft| draft.parts.first()).and_then(|part| part.volume_mm3.value).map(|value| value.to_string()),
        volume_source: state.draft.as_ref().and_then(|draft| draft.parts.first()).map(|part| {
            match part.volume_mm3.source {
                MeasurementSource::Extracted => "extracted",
                MeasurementSource::Manual => "manual",
            }
            .to_string()
        }),
        volume_review: state.draft.as_ref().and_then(|draft| draft.parts.first()).map(|part| {
            match part.volume_mm3.review {
                ReviewState::Pending => "pending",
                ReviewState::Confirmed => "confirmed",
            }
            .to_string()
        }),
        part_costs: state
            .current
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .outputs
                    .parts
                    .iter()
                    .map(|part| format!("{}:{}", part.part_version_id, part.per_part_subtotal.map(|amount| amount.to_string()).unwrap_or_default()))
                    .collect()
            })
            .unwrap_or_default(),
        error: state.error.clone(),
        notice: state.notice.clone(),
        needs_service: state.needs_service,
        retryable: state.failed.as_ref().is_some_and(|failed| failed.retryable),
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

/// Which saved result one export reads: an explicitly accepted baseline, an explicit draft
/// calculation, or else the session's current draft calculation.
fn export_baseline(s: &Session, cmd: &str, params: &ExportParams) -> Result<ExportBaselineRef> {
    let accepted_id = params.accepted_baseline_id.as_deref().map(str::trim).filter(|text| !text.is_empty());
    let accepted = match (accepted_id, params.accepted_baseline_sequence) {
        (Some(text), Some(sequence)) => {
            let accepted_baseline_id = Uuid::parse_str(text).map_err(|_| bad(cmd, "`accepted_baseline_id` must be a UUID"))?;
            Some(ExportBaselineRef::AcceptedBaseline { accepted_baseline_id, sequence })
        }
        (None, None) => None,
        _ => {
            return Err(bad(cmd, "an accepted baseline names both `accepted_baseline_id` and `accepted_baseline_sequence`, or neither"));
        }
    };
    let explicit_draft = params.calculation_id.as_deref().map(str::trim).filter(|text| !text.is_empty());
    let draft = match explicit_draft {
        Some(text) => Some(Uuid::parse_str(text).map_err(|_| bad(cmd, "`calculation_id` must be a UUID"))?),
        None => None,
    };
    match (accepted, draft) {
        (Some(baseline), None) => Ok(baseline),
        (None, Some(calculation_id)) => Ok(ExportBaselineRef::DraftCalculation { calculation_id }),
        (Some(_), Some(_)) => Err(bad(cmd, "name an accepted baseline or a draft calculation, not both")),
        (None, None) => match s.solidbom.current.as_ref() {
            Some(snapshot) => Ok(ExportBaselineRef::DraftCalculation { calculation_id: snapshot.calculation_id }),
            None => Err(bad(cmd, "no accepted baseline is named and the session has no current calculation to export")),
        },
    }
}

/// The saved commercial scenarios to compare, ascending and unique by scenario id.
fn export_scenarios(cmd: &str, scenarios: &[String]) -> Result<Vec<Uuid>> {
    let mut ids = Vec::with_capacity(scenarios.len());
    for text in scenarios {
        ids.push(Uuid::parse_str(text.trim()).map_err(|_| bad(cmd, "`scenarios` must be UUIDs"))?);
    }
    if ids.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(bad(cmd, "`scenarios` must be ascending and unique"));
    }
    Ok(ids)
}
