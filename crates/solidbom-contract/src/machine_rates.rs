//! Machine hourly-rate construction and loading sensitivity (gated package SB-X01).
//!
//! The contract carries the stable machine identity, the immutable rate-version input snapshot, the
//! explained calculation result and the diagnostics that keep invalid workbook data out of approved
//! company rates. Money and rates are [`rust_decimal::Decimal`] strings; `None` is a missing value,
//! never zero. A missing authoritative input is a diagnostic, not an implicit zero.
//!
//! Two calculation modes are explicit. [`CalculationMode::Recommended`] is the production path: it
//! refuses to calculate from incomplete or invalid inputs, so an apparently usable rate never comes
//! from corrected arithmetic. [`CalculationMode::ExcelParity`] reproduces the supplied workbook's
//! arithmetic, including its blank-cell-as-zero and 20 % residual-value defaults, only to keep the
//! documented parity references reproducible; a parity result is never publishable.
//!
//! A rate version is one machine, one financial year and one scenario. The version chain is
//! append-only: a later version supersedes an earlier one, and a published version and its result
//! snapshot never change. Publication appends an immutable fully absorbed cost-center rate through
//! the SB-09 shared-reference publication, with `ACTUAL_RUNNING_HOUR` as the denominator.
//!
//! Loading sensitivity recalculates copied inputs with only `loading` changed. It never mutates the
//! version it reads, and sensitivity at the base loading equals the base rate exactly.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{ContractError, Result};
use crate::shared_data::{MAX_SOURCE_REFERENCE_CHARS, ReferenceChange};
use crate::types::{RateVersion, check_text};

/// The contract version of the machine hourly-rate vocabulary.
pub const MACHINE_RATE_CONTRACT_VERSION: u32 = 1;
/// The longest machine name a version carries.
pub const MAX_MACHINE_NAME_CHARS: usize = 255;
/// The longest business cost-center code a machine identity carries.
pub const MAX_MACHINE_CODE_CHARS: usize = 64;
/// The longest descriptive machine text (process description, brand, model).
pub const MAX_MACHINE_TEXT_CHARS: usize = 255;
/// The longest assumption notes a version retains.
pub const MAX_ASSUMPTION_NOTES_CHARS: usize = 4096;
/// The most diagnostics one outcome carries.
pub const MAX_RATE_DIAGNOSTICS: usize = 256;
/// The most alternative loadings one sensitivity request calculates.
pub const MAX_SENSITIVITY_LOADINGS: usize = 64;
/// The most versions one inventory or chain response carries.
pub const MAX_RATE_VERSIONS: usize = 500;
/// The longest request id a machine-rate write accepts.
pub const MAX_MACHINE_RATE_REQUEST_ID_CHARS: usize = 128;
/// The lower bound of a plausible planning year, shared by the acquisition and financial years.
pub const MIN_PLANNING_YEAR: i32 = 1900;
/// The upper bound of a plausible planning year.
pub const MAX_PLANNING_YEAR: i32 = 2200;

/// The workbook's process types. A manual workstation is the same entity with zero CAPEX, power and
/// utilities.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ProcessType {
    Extrusion,
    Winding,
    Forming,
    Cutting,
    Testing,
    Injection,
    ManualWork,
}

impl ProcessType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Extrusion => "extrusion",
            Self::Winding => "winding",
            Self::Forming => "forming",
            Self::Cutting => "cutting",
            Self::Testing => "testing",
            Self::Injection => "injection",
            Self::ManualWork => "manual_work",
        }
    }
}

/// The planning scenario one rate version belongs to. A revised rate is a new version, never an
/// edit of an existing one.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum RateScenario {
    Base,
    Budget,
    Custom,
}

impl RateScenario {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Budget => "budget",
            Self::Custom => "custom",
        }
    }
}

/// How one calculation interprets missing and invalid inputs.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CalculationMode {
    /// The production path: strict validation, no blank-cell coercion, no implicit residual default.
    Recommended,
    /// The workbook reference path: reproduces Excel arithmetic, including the documented defects.
    /// A parity outcome keeps its numbers but is never publishable.
    ExcelParity,
}

/// Whether a rate version is still being prepared or has been approved and published.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RateVersionState {
    Draft,
    Published,
}

/// The calculation status of one rate version, independent of its publication state.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MachineRateStatus {
    /// Valid arithmetic with non-zero running hours.
    Ok,
    /// Valid arithmetic plus business-review warnings.
    OkWithWarnings,
    /// Inputs are otherwise valid but the per-hour rates are defined by zero running hours.
    NoRunHours,
    /// At least one hard error; no per-hour or annual result is returned.
    Invalid,
}

/// How seriously one diagnostic affects a calculation or a publication.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
}

/// The stable reason one diagnostic exists. The wire names are part of the contract.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum RateDiagnosticCode {
    /// A required financial year is missing; derived remaining life is impossible.
    MissingFinancialYear,
    /// The acquisition year is missing while a derived remaining life is requested.
    MissingAcquisitionYear,
    /// The financial year is earlier than the acquisition year.
    FinancialYearBeforeAcquisitionYear,
    /// A planning year is outside the plausible range.
    InvalidYear,
    /// A required numeric assumption is missing. Under Excel parity the workbook blanks are zero.
    MissingInput,
    /// A numeric assumption is negative.
    NegativeInput,
    /// A capacity input is outside its valid range.
    InvalidCapacity,
    /// Loading is outside its valid range.
    InvalidLoading,
    /// Availability is outside its valid range.
    InvalidAvailability,
    /// The useful life is not positive.
    InvalidUsefulLife,
    /// The remaining depreciation years are negative or above the useful life.
    InvalidRemainingLife,
    /// The derived remaining depreciation years exceed the useful life.
    RemainingLifeExceedsUsefulLife,
    /// The residual value exceeds the installed CAPEX.
    ResidualExceedsInstalledCapex,
    /// The average running power factor is outside `[0, 1]`.
    InvalidPowerFactor,
    /// A labor burden rate is outside `[0, 1]`.
    InvalidBurden,
    /// Checked decimal arithmetic overflowed.
    ArithmeticOverflow,
    /// The per-hour rates are undefined because the actual running hours are zero.
    ZeroRunHours,
    /// The workbook's blank numeric cells were treated as zero (Excel parity only).
    BlankTreatedAsZero,
    /// The workbook's default residual value of 20 % of installed CAPEX was used (parity only).
    ResidualDefaultedToTwentyPercent,
    /// A publication was requested without a stable cost-center code.
    MissingMachineCode,
    /// A publication was requested without a stable machine identity.
    MissingMachineIdentity,
    /// CAPEX is positive while the residual value equals the installed CAPEX.
    ResidualEqualsInstalledCapex,
    /// Installed power is positive while the electricity tariff is zero.
    PowerWithoutTariff,
    /// The electricity tariff is positive while installed power is zero.
    TariffWithoutPower,
    /// Air or water consumption is positive while its tariff is zero.
    UtilityConsumptionWithoutTariff,
    /// Air or water tariff is positive while its consumption is zero.
    UtilityTariffWithoutConsumption,
    /// Committed staffing and dedicated annual labor are both non-zero; confirm no duplicate labor.
    CommittedAndDedicatedLabor,
    /// Premises cost is part of the machine rate; confirm the boundary with production SG&A.
    PremisesAllocationUnconfirmed,
    /// Compressed air is charged while machine power is also charged; confirm the compressor boundary.
    CompressedAirBoundaryUnconfirmed,
    /// Cooling water is charged; confirm the consumption basis (circulation versus net makeup).
    CoolingWaterBasisUnconfirmed,
    /// The workbook itself flags this machine's inputs for review.
    WorkbookInputsFlagged,
}

/// One structured explanation of a calculation or publication finding.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RateDiagnostic {
    pub code: RateDiagnosticCode,
    pub severity: DiagnosticSeverity,
    /// The input field the diagnostic names, when it names one.
    pub field: Option<String>,
    pub message: String,
}

impl RateDiagnostic {
    /// A hard error that prevents a production calculation or a publication.
    pub fn error(code: RateDiagnosticCode, field: impl Into<String>, message: impl Into<String>) -> Self {
        Self { code, severity: DiagnosticSeverity::Error, field: Some(field.into()), message: message.into() }
    }

    /// An error that does not name one input field.
    pub fn global_error(code: RateDiagnosticCode, message: impl Into<String>) -> Self {
        Self { code, severity: DiagnosticSeverity::Error, field: None, message: message.into() }
    }

    /// A business-review warning. It changes no arithmetic.
    pub fn warning(code: RateDiagnosticCode, field: impl Into<String>, message: impl Into<String>) -> Self {
        Self { code, severity: DiagnosticSeverity::Warning, field: Some(field.into()), message: message.into() }
    }

    /// A warning that does not name one input field.
    pub fn global_warning(code: RateDiagnosticCode, message: impl Into<String>) -> Self {
        Self { code, severity: DiagnosticSeverity::Warning, field: None, message: message.into() }
    }

    /// Validate the bounded message of a diagnostic.
    pub fn validate(&self) -> Result<()> {
        check_text(&self.message, "diagnostic.message")?;
        if self.field.as_ref().is_some_and(|field| field.is_empty()) {
            return Err(ContractError::invalid("diagnostic.field", "an empty field name is not valid"));
        }
        if self.message.chars().count() > MAX_ASSUMPTION_NOTES_CHARS {
            return Err(ContractError::invalid("diagnostic.message", "the diagnostic message is too long"));
        }
        Ok(())
    }
}

/// Stable machine identity. It holds no annually changing calculation assumption; a renamed or
/// re-coded machine does not rewrite a historical calculation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MachineIdentity {
    /// The stable logical machine or cost-center id; it is also the published cost-center id.
    pub machine_id: Uuid,
    /// The business cost-center code. Imported workbook sheets have none; publication needs one.
    pub code: Option<String>,
    pub name: String,
    pub process_type: ProcessType,
    pub process_description: Option<String>,
    pub brand: Option<String>,
    pub model: Option<String>,
    /// The acquisition year; required only when the remaining life is derived.
    pub acquisition_year: Option<i32>,
    /// Whether future rate versions may select this machine. It never hides history.
    pub active: bool,
}

impl MachineIdentity {
    pub fn validate(&self) -> Result<()> {
        if self.machine_id.is_nil() {
            return Err(ContractError::invalid("machine.machine_id", "a machine needs a stable non-nil id"));
        }
        check_text(&self.name, "machine.name")?;
        if self.name.chars().count() > MAX_MACHINE_NAME_CHARS {
            return Err(ContractError::invalid("machine.name", "the machine name is too long"));
        }
        if let Some(code) = &self.code {
            check_machine_code(code)?;
        }
        check_optional_machine_text(&self.process_description, "machine.process_description")?;
        check_optional_machine_text(&self.brand, "machine.brand")?;
        check_optional_machine_text(&self.model, "machine.model")?;
        if let Some(year) = self.acquisition_year {
            check_planning_year(year, "machine.acquisition_year")?;
        }
        Ok(())
    }
}

/// The immutable input snapshot of one machine rate version. Every numeric value is explicit; a
/// missing value is `None`. The recommended mode refuses to calculate unless the authoritative
/// inputs are present, so a blank cell never silently becomes zero.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MachineRateInput {
    pub machine: MachineIdentity,
    /// The planning year; required by the recommended mode and derived from the workbook only in
    /// parity mode.
    pub financial_year: Option<i32>,
    pub scenario: RateScenario,
    /// Scheduled capacity: working days per year, shifts per day and hours per shift.
    pub working_days_per_year: Option<Decimal>,
    pub shifts_per_day: Option<Decimal>,
    pub hours_per_shift: Option<Decimal>,
    /// Demand occupation of the scheduled capacity; `0` is a valid scenario with undefined rates.
    pub loading: Option<Decimal>,
    /// Time losses inside the loaded scheduled hours.
    pub availability: Option<Decimal>,
    /// Machine purchase, installation and dedicated auxiliary CAPEX.
    pub machine_purchase_capex: Option<Decimal>,
    pub installation_capex: Option<Decimal>,
    pub dedicated_auxiliary_capex: Option<Decimal>,
    /// The explicit residual value. The recommended mode has no implicit default.
    pub residual_value: Option<Decimal>,
    pub useful_life_years: Option<Decimal>,
    /// An explicit remaining-life override. `None` derives it from the two years.
    pub remaining_depreciation_years: Option<Decimal>,
    /// Annual fixed operating cost components, excluding depreciation.
    pub annual_maintenance: Option<Decimal>,
    pub annual_insurance: Option<Decimal>,
    pub floor_space_m2: Option<Decimal>,
    pub annual_premises_rate_per_m2: Option<Decimal>,
    pub other_fixed_machine_overhead: Option<Decimal>,
    pub dedicated_annual_labor: Option<Decimal>,
    /// Committed shift labor, paid during loaded hours including loaded downtime.
    pub committed_operators: Option<Decimal>,
    pub committed_labor_base_hourly_pay: Option<Decimal>,
    pub committed_labor_burden_rate: Option<Decimal>,
    /// Utilities, wear, consumables and run-only labor, all per actual running hour.
    pub installed_running_power_kw: Option<Decimal>,
    pub average_running_power_factor: Option<Decimal>,
    pub electricity_tariff: Option<Decimal>,
    pub standby_power_kw: Option<Decimal>,
    pub compressed_air_nm3_per_running_hour: Option<Decimal>,
    pub compressed_air_tariff: Option<Decimal>,
    pub cooling_water_m3_per_running_hour: Option<Decimal>,
    pub cooling_water_tariff: Option<Decimal>,
    pub other_utilities_per_running_hour: Option<Decimal>,
    pub wear_parts_per_running_hour: Option<Decimal>,
    pub operating_consumables_per_running_hour: Option<Decimal>,
    pub run_only_operators: Option<Decimal>,
    pub run_only_labor_base_hourly_pay: Option<Decimal>,
    pub run_only_labor_burden_rate: Option<Decimal>,
    pub other_variable_cost_per_running_hour: Option<Decimal>,
    /// Provenance: a workbook/document reference and free assumption notes.
    pub source_reference: Option<String>,
    pub assumption_notes: Option<String>,
}

impl MachineRateInput {
    /// Wire-level validation: bounded text, identity and plausible planning years. Semantic
    /// arithmetic validation happens in the calculation (it returns diagnostics, not parse errors).
    pub fn validate(&self) -> Result<()> {
        self.machine.validate()?;
        if let Some(year) = self.financial_year {
            check_planning_year(year, "financial_year")?;
        }
        if let Some(reference) = &self.source_reference {
            check_text(reference, "source_reference")?;
            if reference.chars().count() > MAX_SOURCE_REFERENCE_CHARS {
                return Err(ContractError::invalid("source_reference", "the source reference is too long"));
            }
        }
        if let Some(notes) = &self.assumption_notes {
            check_text(notes, "assumption_notes")?;
            if notes.chars().count() > MAX_ASSUMPTION_NOTES_CHARS {
                return Err(ContractError::invalid("assumption_notes", "the assumption notes are too long"));
            }
        }
        Ok(())
    }
}

/// Every explained component of one machine rate calculation. All values are exact decimals;
/// per-hour values are `None` when they are mathematically undefined.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MachineRateResult {
    pub scheduled_capacity_hours: Decimal,
    pub loaded_scheduled_hours: Decimal,
    pub loaded_downtime_hours: Decimal,
    pub actual_running_hours: Decimal,
    pub installed_capex: Decimal,
    pub residual_value: Decimal,
    pub remaining_depreciation_years: Decimal,
    pub annual_depreciation: Decimal,
    pub annual_premises_cost: Decimal,
    pub loaded_committed_labor_rate: Decimal,
    pub annual_committed_shift_labor: Decimal,
    pub annual_fixed_operating_cost: Decimal,
    pub running_electricity_per_hour: Decimal,
    pub annual_standby_electricity: Decimal,
    pub compressed_air_per_hour: Decimal,
    pub cooling_water_per_hour: Decimal,
    pub run_only_labor_per_hour: Decimal,
    pub variable_cost_excluding_standby: Decimal,
    pub annual_variable_operating_cost: Decimal,
    pub annual_fully_absorbed_cost: Decimal,
    /// Standby electricity per actual running hour.
    pub standby_electricity_per_running_hour: Option<Decimal>,
    pub variable_operating_rate: Option<Decimal>,
    pub fixed_operating_rate: Option<Decimal>,
    pub depreciation_rate: Option<Decimal>,
    /// The fully absorbed production rate: total cost per actual running hour.
    pub total_machine_hourly_rate: Option<Decimal>,
    /// The cash operating rate: the total rate without depreciation.
    pub cash_operating_rate: Option<Decimal>,
    /// The alternative loaded-scheduled-hour reporting denominator, never a production rate.
    pub annual_cost_per_loaded_scheduled_hour: Option<Decimal>,
}

impl MachineRateResult {
    /// Whether every per-hour output is undefined (the zero-running-hours case).
    pub fn has_undefined_per_hour_rates(&self) -> bool {
        self.standby_electricity_per_running_hour.is_none()
            && self.variable_operating_rate.is_none()
            && self.fixed_operating_rate.is_none()
            && self.depreciation_rate.is_none()
            && self.total_machine_hourly_rate.is_none()
            && self.cash_operating_rate.is_none()
            && self.annual_cost_per_loaded_scheduled_hour.is_none()
    }
}

/// The deterministic answer of one calculation: the explained components, the status and the
/// structured diagnostics. An invalid calculation has no result at all.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MachineRateOutcome {
    pub status: MachineRateStatus,
    pub result: Option<MachineRateResult>,
    pub diagnostics: Vec<RateDiagnostic>,
}

impl MachineRateOutcome {
    /// Whether this outcome may be published as an approved fully absorbed rate.
    pub fn is_publishable(&self) -> bool {
        matches!(self.status, MachineRateStatus::Ok | MachineRateStatus::OkWithWarnings)
            && self.result.as_ref().is_some_and(|result| result.total_machine_hourly_rate.is_some())
    }

    pub fn validate(&self) -> Result<()> {
        if self.diagnostics.len() > MAX_RATE_DIAGNOSTICS {
            return Err(ContractError::invalid("outcome.diagnostics", "too many diagnostics"));
        }
        for diagnostic in &self.diagnostics {
            diagnostic.validate()?;
        }
        let errors = self.diagnostics.iter().filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error).count();
        match self.status {
            MachineRateStatus::Invalid => {
                if self.result.is_some() {
                    return Err(ContractError::invalid("outcome.result", "an invalid outcome carries no result"));
                }
                if errors == 0 {
                    return Err(ContractError::invalid("outcome.diagnostics", "an invalid outcome names at least one error"));
                }
            }
            MachineRateStatus::Ok | MachineRateStatus::OkWithWarnings | MachineRateStatus::NoRunHours => {
                let result =
                    self.result.as_ref().ok_or_else(|| ContractError::invalid("outcome.result", "a calculable outcome carries its result"))?;
                if errors != 0 {
                    return Err(ContractError::invalid("outcome.diagnostics", "a calculable outcome carries no hard error"));
                }
                if self.status == MachineRateStatus::NoRunHours && !result.has_undefined_per_hour_rates() {
                    return Err(ContractError::invalid("outcome.result", "the no-run-hours status leaves every per-hour rate undefined"));
                }
                if self.status != MachineRateStatus::NoRunHours && result.total_machine_hourly_rate.is_none() {
                    return Err(ContractError::invalid("outcome.result", "a non-zero-running-hours outcome defines the hourly rates"));
                }
            }
        }
        Ok(())
    }
}

/// One immutable rate version of one machine, financial year and scenario.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MachineRateVersion {
    pub machine_rate_version_id: Uuid,
    /// The append-only position inside the (machine, financial year, scenario) chain, starting at 1.
    pub version: u64,
    /// The previous version this one replaces, if any.
    pub supersedes: Option<Uuid>,
    pub state: RateVersionState,
    pub input: MachineRateInput,
    /// The SB-09 published cost-center rate version, set on publication.
    pub published_rate_version_id: Option<Uuid>,
    /// The calculator version that produced the publication snapshot.
    pub calculator_version: Option<String>,
    /// RFC 3339 UTC timestamp the service recorded.
    pub created_at: Option<String>,
    pub published_at: Option<String>,
    pub created_by: Option<String>,
}

impl MachineRateVersion {
    pub fn validate(&self) -> Result<()> {
        if self.machine_rate_version_id.is_nil() {
            return Err(ContractError::invalid("machine_rate_version.machine_rate_version_id", "a version needs a non-nil id"));
        }
        if self.version == 0 {
            return Err(ContractError::invalid("machine_rate_version.version", "a published version starts at 1"));
        }
        if self.supersedes == Some(self.machine_rate_version_id) {
            return Err(ContractError::invalid("machine_rate_version.supersedes", "a version cannot supersede itself"));
        }
        self.input.validate()?;
        match self.state {
            RateVersionState::Draft => {
                if self.published_rate_version_id.is_some() || self.published_at.is_some() || self.calculator_version.is_some() {
                    return Err(ContractError::invalid("machine_rate_version.state", "a draft carries no publication"));
                }
            }
            RateVersionState::Published => {
                if self.published_rate_version_id.is_none() {
                    return Err(ContractError::invalid(
                        "machine_rate_version.published_rate_version_id",
                        "a published version names its cost-center rate version",
                    ));
                }
                if self.calculator_version.is_none() {
                    return Err(ContractError::invalid(
                        "machine_rate_version.calculator_version",
                        "a published version names its calculator version",
                    ));
                }
                if self.published_at.is_none() {
                    return Err(ContractError::invalid("machine_rate_version.published_at", "a published version carries its publication time"));
                }
            }
        }
        Ok(())
    }
}

/// A rate version with its current calculation. A published version answers from its immutable
/// stored snapshot; a draft answers from a fresh deterministic calculation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MachineRateVersionView {
    pub version: MachineRateVersion,
    pub outcome: MachineRateOutcome,
}

impl MachineRateVersionView {
    pub fn validate(&self) -> Result<()> {
        self.version.validate()?;
        self.outcome.validate()?;
        Ok(())
    }
}

/// The append-only version chain of one machine and scenario, or the machine inventory, newest
/// first.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MachineRateVersionList {
    pub versions: Vec<MachineRateVersionView>,
}

impl MachineRateVersionList {
    pub fn validate(&self) -> Result<()> {
        if self.versions.len() > MAX_RATE_VERSIONS {
            return Err(ContractError::invalid("machine_rate_versions", "too many versions"));
        }
        for view in &self.versions {
            view.validate()?;
        }
        Ok(())
    }
}

/// Create the next draft rate version of one machine, financial year and scenario.
///
/// `expected_version` is the current latest version of that chain, `0` for the first one. The
/// service refuses a stale expectation instead of overwriting another writer's version.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateMachineRateVersion {
    pub request_id: String,
    pub expected_version: u64,
    pub input: MachineRateInput,
}

impl CreateMachineRateVersion {
    pub fn validate(&self) -> Result<()> {
        check_rate_request(&self.request_id)?;
        self.input.validate()?;
        Ok(())
    }
}

/// Publish one validated draft as an immutable cost-center rate through the SB-09 publication.
///
/// `expected_version` is the machine rate version the caller believes is current; a stale value is
/// refused rather than publishing an unknown version.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublishMachineRateVersion {
    pub request_id: String,
    pub expected_version: u64,
}

impl PublishMachineRateVersion {
    pub fn validate(&self) -> Result<()> {
        check_rate_request(&self.request_id)
    }
}

/// The answer to publishing one machine rate version: the immutable version, its approval
/// snapshot, the SB-09 cost-center rate version and its reference-change fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MachineRateVersionPublished {
    pub version: MachineRateVersion,
    pub outcome: MachineRateOutcome,
    pub published_rate: RateVersion,
    pub change: ReferenceChange,
}

impl MachineRateVersionPublished {
    pub fn validate(&self) -> Result<()> {
        self.version.validate()?;
        self.outcome.validate()?;
        if self.version.state != RateVersionState::Published {
            return Err(ContractError::invalid("machine_rate_version.state", "a publication answers with a published version"));
        }
        if self.version.published_rate_version_id != Some(self.published_rate.rate_version_id) {
            return Err(ContractError::invalid("published_rate", "the published rate is not the version the machine rate names"));
        }
        if !self.outcome.is_publishable() {
            return Err(ContractError::invalid("outcome", "the publication outcome is not publishable"));
        }
        Ok(())
    }
}

/// The alternative loadings one sensitivity request calculates.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MachineRateSensitivityRequest {
    pub loadings: Vec<Decimal>,
}

impl MachineRateSensitivityRequest {
    pub fn validate(&self) -> Result<()> {
        if self.loadings.is_empty() {
            return Err(ContractError::invalid("loadings", "a sensitivity request names at least one loading"));
        }
        if self.loadings.len() > MAX_SENSITIVITY_LOADINGS {
            return Err(ContractError::invalid("loadings", "too many alternative loadings"));
        }
        for loading in &self.loadings {
            if loading.is_sign_negative() || *loading > Decimal::ONE {
                return Err(ContractError::invalid("loadings", "a sensitivity loading must be within [0, 1]"));
            }
        }
        Ok(())
    }
}

/// One calculated alternative loading.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MachineRateSensitivityScenario {
    pub loading: Decimal,
    pub outcome: MachineRateOutcome,
}

/// The base calculation plus every alternative loading. It is a pure read: no input, draft or
/// published version changes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MachineRateSensitivity {
    pub base: MachineRateOutcome,
    pub scenarios: Vec<MachineRateSensitivityScenario>,
}

impl MachineRateSensitivity {
    pub fn validate(&self) -> Result<()> {
        self.base.validate()?;
        if self.scenarios.len() > MAX_SENSITIVITY_LOADINGS {
            return Err(ContractError::invalid("sensitivity.scenarios", "too many alternative loadings"));
        }
        for scenario in &self.scenarios {
            if scenario.loading.is_sign_negative() || scenario.loading > Decimal::ONE {
                return Err(ContractError::invalid("sensitivity.scenarios.loading", "a sensitivity loading must be within [0, 1]"));
            }
            scenario.outcome.validate()?;
        }
        Ok(())
    }
}

fn check_rate_request(request_id: &str) -> Result<()> {
    check_text(request_id, "request_id")?;
    if request_id.chars().count() > MAX_MACHINE_RATE_REQUEST_ID_CHARS {
        return Err(ContractError::invalid("request_id", "the request id is too long"));
    }
    Ok(())
}

fn check_machine_code(code: &str) -> Result<()> {
    check_text(code, "machine.code")?;
    if code.chars().count() > MAX_MACHINE_CODE_CHARS {
        return Err(ContractError::invalid("machine.code", "the cost-center code is too long"));
    }
    if code.chars().any(char::is_control) {
        return Err(ContractError::invalid("machine.code", "the cost-center code contains a control character"));
    }
    Ok(())
}

fn check_optional_machine_text(text: &Option<String>, field: &str) -> Result<()> {
    let Some(text) = text else { return Ok(()) };
    check_text(text, field)?;
    if text.chars().count() > MAX_MACHINE_TEXT_CHARS {
        return Err(ContractError::invalid(field, "the machine text is too long"));
    }
    if text.chars().any(char::is_control) {
        return Err(ContractError::invalid(field, "the machine text contains a control character"));
    }
    Ok(())
}

fn check_planning_year(year: i32, field: &str) -> Result<()> {
    if !(MIN_PLANNING_YEAR..=MAX_PLANNING_YEAR).contains(&year) {
        return Err(ContractError::invalid(field, format!("expected a year between {MIN_PLANNING_YEAR} and {MAX_PLANNING_YEAR}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use rust_decimal::Decimal;

    use super::*;

    fn machine() -> MachineIdentity {
        MachineIdentity {
            machine_id: Uuid::from_u128(7),
            code: Some("CC-1001".into()),
            name: "Extrusion Line 1".into(),
            process_type: ProcessType::Extrusion,
            process_description: None,
            brand: None,
            model: None,
            acquisition_year: Some(2016),
            active: true,
        }
    }

    fn input() -> MachineRateInput {
        MachineRateInput {
            machine: machine(),
            financial_year: Some(2026),
            scenario: RateScenario::Base,
            working_days_per_year: Some(Decimal::from(250)),
            shifts_per_day: Some(Decimal::from(2)),
            hours_per_shift: Some(Decimal::from(8)),
            loading: Some(Decimal::new(85, 2)),
            availability: Some(Decimal::new(90, 2)),
            machine_purchase_capex: Some(Decimal::from(3_700_000)),
            installation_capex: Some(Decimal::from(50_000)),
            dedicated_auxiliary_capex: Some(Decimal::new(506_550, 2)),
            residual_value: Some(Decimal::new(75_101_310, 2)),
            useful_life_years: Some(Decimal::from(10)),
            remaining_depreciation_years: None,
            annual_maintenance: Some(Decimal::from(20_000)),
            annual_insurance: Some(Decimal::from(1_000)),
            floor_space_m2: Some(Decimal::from(1_014)),
            annual_premises_rate_per_m2: Some(Decimal::from(120)),
            other_fixed_machine_overhead: Some(Decimal::from(761)),
            dedicated_annual_labor: Some(Decimal::ZERO),
            committed_operators: Some(Decimal::new(1, 1)),
            committed_labor_base_hourly_pay: Some(Decimal::from(30)),
            committed_labor_burden_rate: Some(Decimal::new(222, 3)),
            installed_running_power_kw: Some(Decimal::from(150)),
            average_running_power_factor: Some(Decimal::new(85, 2)),
            electricity_tariff: Some(Decimal::ONE),
            standby_power_kw: Some(Decimal::new(255, 2)),
            compressed_air_nm3_per_running_hour: Some(Decimal::from(10)),
            compressed_air_tariff: Some(Decimal::new(20, 2)),
            cooling_water_m3_per_running_hour: Some(Decimal::from(4)),
            cooling_water_tariff: Some(Decimal::new(50, 2)),
            other_utilities_per_running_hour: Some(Decimal::from(3)),
            wear_parts_per_running_hour: Some(Decimal::from(4)),
            operating_consumables_per_running_hour: Some(Decimal::from(5)),
            run_only_operators: Some(Decimal::new(1092, 4)),
            run_only_labor_base_hourly_pay: Some(Decimal::from(40)),
            run_only_labor_burden_rate: Some(Decimal::new(25, 2)),
            other_variable_cost_per_running_hour: Some(Decimal::new(150, 2)),
            source_reference: Some("machine hourly-rate workbook".into()),
            assumption_notes: None,
        }
    }

    fn result() -> MachineRateResult {
        let zero = Decimal::ZERO;
        MachineRateResult {
            scheduled_capacity_hours: Decimal::from(4000),
            loaded_scheduled_hours: Decimal::from(3400),
            loaded_downtime_hours: Decimal::from(340),
            actual_running_hours: Decimal::from(3060),
            installed_capex: Decimal::new(375_506_550, 2),
            residual_value: Decimal::new(75_101_310, 2),
            remaining_depreciation_years: zero,
            annual_depreciation: zero,
            annual_premises_cost: Decimal::from(121_680),
            loaded_committed_labor_rate: Decimal::new(3666, 2),
            annual_committed_shift_labor: Decimal::new(1_246_440, 2),
            annual_fixed_operating_cost: Decimal::new(15_590_540, 2),
            running_electricity_per_hour: Decimal::new(12_750, 2),
            annual_standby_electricity: Decimal::from(867),
            compressed_air_per_hour: Decimal::from(2),
            cooling_water_per_hour: Decimal::from(2),
            run_only_labor_per_hour: Decimal::new(546, 2),
            variable_cost_excluding_standby: Decimal::new(15_046, 2),
            annual_variable_operating_cost: Decimal::new(46_127_460, 2),
            annual_fully_absorbed_cost: Decimal::from(617_180),
            standby_electricity_per_running_hour: Some(Decimal::new(28_333, 5)),
            variable_operating_rate: Some(Decimal::new(15_074_333, 5)),
            fixed_operating_rate: Some(Decimal::new(50_949_477, 6)),
            depreciation_rate: Some(zero),
            total_machine_hourly_rate: Some(Decimal::new(201_692_810, 6)),
            cash_operating_rate: Some(Decimal::new(201_692_810, 6)),
            annual_cost_per_loaded_scheduled_hour: Some(Decimal::new(181_523_529, 6)),
        }
    }

    fn outcome(status: MachineRateStatus, result: Option<MachineRateResult>) -> MachineRateOutcome {
        MachineRateOutcome { status, result, diagnostics: Vec::new() }
    }

    #[test]
    fn a_machine_identity_needs_a_stable_id_and_bounded_text() {
        assert!(machine().validate().is_ok());
        let mut blank_name = machine();
        blank_name.name = "  ".into();
        assert!(blank_name.validate().is_err());
        let mut nil = machine();
        nil.machine_id = Uuid::nil();
        assert!(nil.validate().is_err());
        let mut long_code = machine();
        long_code.code = Some("x".repeat(MAX_MACHINE_CODE_CHARS + 1));
        assert!(long_code.validate().is_err());
        let mut year = machine();
        year.acquisition_year = Some(1500);
        assert!(year.validate().is_err());
    }

    #[test]
    fn an_input_requires_bounded_provenance_and_a_plausible_year() {
        assert!(input().validate().is_ok());
        let mut missing_year = input();
        missing_year.financial_year = None;
        assert!(missing_year.validate().is_ok(), "a missing year is a calculation diagnostic, not a wire error");
        let mut long_notes = input();
        long_notes.assumption_notes = Some("x".repeat(MAX_ASSUMPTION_NOTES_CHARS + 1));
        assert!(long_notes.validate().is_err());
        let mut early_year = input();
        early_year.financial_year = Some(0);
        assert!(early_year.validate().is_err());
    }

    #[test]
    fn a_missing_value_stays_missing_and_an_explicit_zero_stays_zero_through_json() {
        let mut with_missing = input();
        with_missing.annual_insurance = None;
        with_missing.dedicated_annual_labor = Some(Decimal::ZERO);
        let json = serde_json::to_value(&with_missing).unwrap();
        let roundtrip: MachineRateInput = serde_json::from_value(json).unwrap();
        assert_eq!(roundtrip, with_missing);
        assert_eq!(roundtrip.annual_insurance, None);
        assert_eq!(roundtrip.dedicated_annual_labor, Some(Decimal::ZERO));
    }

    #[test]
    fn an_invalid_outcome_carries_no_result_and_at_least_one_error() {
        let invalid = MachineRateOutcome {
            status: MachineRateStatus::Invalid,
            result: None,
            diagnostics: vec![RateDiagnostic::error(RateDiagnosticCode::MissingFinancialYear, "financial_year", "missing")],
        };
        assert!(invalid.validate().is_ok());
        let mut with_result = invalid.clone();
        with_result.result = Some(result());
        assert!(with_result.validate().is_err());
        let mut without_error = invalid;
        without_error.diagnostics.clear();
        assert!(without_error.validate().is_err());
    }

    #[test]
    fn a_calculable_outcome_carries_no_hard_error() {
        let ok = outcome(MachineRateStatus::Ok, Some(result()));
        assert!(ok.validate().is_ok());
        assert!(ok.is_publishable());
        let mut warning = ok.clone();
        warning.status = MachineRateStatus::OkWithWarnings;
        warning.diagnostics = vec![RateDiagnostic::warning(RateDiagnosticCode::ResidualEqualsInstalledCapex, "residual_value", "zero depreciation")];
        assert!(warning.validate().is_ok());
        assert!(warning.is_publishable());
        let mut erroneous = ok;
        erroneous.diagnostics = vec![RateDiagnostic::error(RateDiagnosticCode::MissingInput, "annual_maintenance", "missing")];
        assert!(erroneous.validate().is_err());
    }

    #[test]
    fn the_no_run_hours_status_leaves_every_per_hour_rate_undefined() {
        let mut zero_hours = result();
        zero_hours.standby_electricity_per_running_hour = None;
        zero_hours.variable_operating_rate = None;
        zero_hours.fixed_operating_rate = None;
        zero_hours.depreciation_rate = None;
        zero_hours.total_machine_hourly_rate = None;
        zero_hours.cash_operating_rate = None;
        zero_hours.annual_cost_per_loaded_scheduled_hour = None;
        let valid = outcome(MachineRateStatus::NoRunHours, Some(zero_hours.clone()));
        assert!(valid.validate().is_ok());
        assert!(!valid.is_publishable());
        let still_defined = outcome(MachineRateStatus::NoRunHours, Some(result()));
        assert!(still_defined.validate().is_err());
    }

    #[test]
    fn a_published_version_names_its_publication_and_a_draft_has_none() {
        let mut draft = MachineRateVersion {
            machine_rate_version_id: Uuid::from_u128(11),
            version: 1,
            supersedes: None,
            state: RateVersionState::Draft,
            input: input(),
            published_rate_version_id: None,
            calculator_version: None,
            created_at: Some("2026-11-11T13:00:00.000Z".into()),
            published_at: None,
            created_by: Some("administrator".into()),
        };
        assert!(draft.validate().is_ok());
        let mut published = draft.clone();
        published.state = RateVersionState::Published;
        published.published_rate_version_id = Some(Uuid::from_u128(12));
        published.calculator_version = Some("solidbom-machine-rate/1".into());
        published.published_at = Some("2026-11-11T13:00:00.000Z".into());
        assert!(published.validate().is_ok());
        draft.published_at = Some("2026-11-11T13:00:00.000Z".into());
        assert!(draft.validate().is_err());
    }

    #[test]
    fn a_sensitivity_request_accepts_an_empty_loading_scenario_and_refuses_an_impossible_one() {
        let request = MachineRateSensitivityRequest { loadings: vec![Decimal::new(50, 2), Decimal::ZERO, Decimal::ONE] };
        assert!(request.validate().is_ok());
        let negative = MachineRateSensitivityRequest { loadings: vec![Decimal::new(-1, 1)] };
        assert!(negative.validate().is_err());
        let above_one = MachineRateSensitivityRequest { loadings: vec![Decimal::new(101, 2)] };
        assert!(above_one.validate().is_err());
        let empty = MachineRateSensitivityRequest { loadings: Vec::new() };
        assert!(empty.validate().is_err());
        let too_many = MachineRateSensitivityRequest { loadings: vec![Decimal::new(50, 2); MAX_SENSITIVITY_LOADINGS + 1] };
        assert!(too_many.validate().is_err());
    }

    #[test]
    fn a_create_request_checks_its_id_and_input() {
        let request = CreateMachineRateVersion { request_id: "req-1".into(), expected_version: 0, input: input() };
        assert!(request.validate().is_ok());
        let blank = CreateMachineRateVersion { request_id: " ".into(), ..request.clone() };
        assert!(blank.validate().is_err());
        let long = CreateMachineRateVersion { request_id: "x".repeat(MAX_MACHINE_RATE_REQUEST_ID_CHARS + 1), ..request };
        assert!(long.validate().is_err());
    }

    #[test]
    fn the_wire_names_are_stable() {
        assert_eq!(serde_json::to_string(&ProcessType::ManualWork).unwrap(), "\"manual_work\"");
        assert_eq!(serde_json::to_string(&RateScenario::Base).unwrap(), "\"base\"");
        assert_eq!(serde_json::to_string(&CalculationMode::ExcelParity).unwrap(), "\"excel_parity\"");
        assert_eq!(serde_json::to_string(&RateVersionState::Published).unwrap(), "\"published\"");
        assert_eq!(serde_json::to_string(&MachineRateStatus::NoRunHours).unwrap(), "\"no_run_hours\"");
        assert_eq!(serde_json::to_string(&DiagnosticSeverity::Warning).unwrap(), "\"warning\"");
        assert_eq!(serde_json::to_string(&RateDiagnosticCode::MissingMachineCode).unwrap(), "\"missing_machine_code\"");
    }

    #[test]
    fn a_result_round_trips_with_explicit_nulls() {
        let value = outcome(MachineRateStatus::Ok, Some(result()));
        let json = serde_json::to_value(&value).unwrap();
        assert_eq!(json["result"]["total_machine_hourly_rate"].as_str(), Some("201.692810"));
        let roundtrip: MachineRateOutcome = serde_json::from_value(json).unwrap();
        assert_eq!(roundtrip, value);
    }
}
