//! SolidBOM's machine hourly-rate engine: pure, deterministic arithmetic over the portable SB-X01
//! contract, used by the service only.
//!
//! # Published contract
//!
//! The input is one [`solidbom_contract::MachineRateInput`], an immutable snapshot of one machine,
//! one financial year and one scenario. The answer is one
//! [`solidbom_contract::MachineRateOutcome`] with the explained components, the status and the
//! structured diagnostics. Money and rates are [`rust_decimal::Decimal`]; a missing input is
//! `None`, never an implicit zero. The engine is pure: it reads no clock, database, locale or file,
//! so one input and one mode always produce a byte-identical outcome.
//!
//! # Calculation in version `solidbom-machine-rate/1`
//!
//! ```text
//! scheduled_capacity_hours = working_days_per_year × shifts_per_day × hours_per_shift
//! loaded_scheduled_hours   = scheduled_capacity_hours × loading
//! loaded_downtime_hours    = loaded_scheduled_hours × (1 − availability)
//! actual_running_hours     = loaded_scheduled_hours × availability
//!
//! installed_capex          = machine + installation + dedicated_auxiliary CAPEX
//! annual_depreciation      = (installed_capex − residual_value) / useful_life_years
//!                            × min(1, remaining_depreciation_years)
//! annual_premises_cost     = floor_space_m2 × annual_premises_rate_per_m2
//! loaded_committed_labor_rate = committed_labor_base_hourly_pay × (1 + committed_labor_burden_rate)
//! annual_committed_shift_labor = committed_operators × loaded_committed_labor_rate
//!                                × loaded_scheduled_hours
//! annual_fixed_operating_cost  = maintenance + insurance + premises
//!                              + other_fixed_machine_overhead + dedicated_annual_labor
//!                              + annual_committed_shift_labor
//! running_electricity_per_hour = installed_running_power_kw × average_running_power_factor
//!                                × electricity_tariff
//! annual_standby_electricity   = standby_power_kw × electricity_tariff × loaded_downtime_hours
//! compressed_air_per_hour      = compressed_air_nm3_per_running_hour × compressed_air_tariff
//! cooling_water_per_hour       = cooling_water_m3_per_running_hour × cooling_water_tariff
//! run_only_labor_per_hour      = run_only_operators × run_only_labor_base_hourly_pay
//!                                × (1 + run_only_labor_burden_rate)
//! variable_cost_excluding_standby = running_electricity + compressed_air + cooling_water
//!                                + other_utilities + wear_parts + operating_consumables
//!                                + run_only_labor + other_variable_cost
//! annual_variable_operating_cost = variable_cost_excluding_standby × actual_running_hours
//!                                  + annual_standby_electricity
//! annual_fully_absorbed_cost   = annual_fixed_operating_cost + annual_depreciation
//!                                + annual_variable_operating_cost
//! ```
//!
//! For `actual_running_hours > 0`:
//!
//! ```text
//! standby_electricity_per_running_hour = annual_standby_electricity / actual_running_hours
//! fixed_operating_rate      = annual_fixed_operating_cost / actual_running_hours
//! depreciation_rate         = annual_depreciation / actual_running_hours
//! variable_operating_rate   = variable_cost_excluding_standby
//!                           + standby_electricity_per_running_hour
//! total_machine_hourly_rate = (annual_fixed_operating_cost + annual_depreciation
//!                              + annual_standby_electricity) / actual_running_hours
//!                             + variable_cost_excluding_standby
//! cash_operating_rate       = total_machine_hourly_rate − depreciation_rate
//! annual_cost_per_loaded_scheduled_hour = annual_fully_absorbed_cost / loaded_scheduled_hours
//! ```
//!
//! The total rate uses the direct formula; the decomposition into fixed, variable and depreciation
//! rates is a reconciliation invariant, not a second arithmetic path. All arithmetic is checked
//! decimal arithmetic: an unrepresentable operation becomes an `arithmetic_overflow` diagnostic and
//! an invalid outcome, never a panic and never an invented value. `rust_decimal` division is the
//! only rounding, at its last representable digit; this crate applies no display rounding.
//!
//! # Modes
//!
//! [`solidbom_contract::CalculationMode::Recommended`] is the production path. It requires every
//! authoritative numeric input, both planning years where the remaining life is derived, and every
//! documented range. A missing value is a `missing_input` error and leaves no result. Zero loading
//! or zero availability is a valid scenario: the outcome is `no_run_hours` with every per-hour rate
//! `None` and the annual costs still returned.
//!
//! [`solidbom_contract::CalculationMode::ExcelParity`] reproduces the supplied workbook, including
//! its documented defects: blank numeric cells become zero (`blank_treated_as_zero`), a missing
//! residual value becomes 20 % of installed CAPEX (`residual_defaulted_to_twenty_percent`), a
//! missing financial year becomes year zero for the derived remaining life
//! (`remaining_life_exceeds_useful_life` warns) and the workbook's own `CHECK INPUTS` guard becomes
//! a `workbook_inputs_flagged` warning. A parity outcome keeps its numbers so the documented
//! references stay reproducible. Parity never turns an input-quality problem into a hard error; the
//! only exception is arithmetic the workbook itself cannot evaluate (`useful_life_years <= 0`
//! divides by zero in the workbook), where no number exists to preserve and the outcome is
//! `invalid`. The service calculates drafts only in the recommended mode, so a parity outcome is
//! never published.
//!
//! # Statuses and diagnostics
//!
//! | Status | Result |
//! |---|---|
//! | `ok` | full result, no business warning |
//! | `ok_with_warnings` | full result plus business-review warnings |
//! | `no_run_hours` | annual costs returned, all seven per-hour fields `None` |
//! | `invalid` | no result, at least one error diagnostic |
//!
//! Every returned outcome satisfies [`solidbom_contract::MachineRateOutcome::validate`]. Business
//! warnings change no arithmetic: residual value equal to positive installed CAPEX, power without a
//! tariff or the reverse, air/water consumption without a tariff or the reverse, committed staffing
//! together with dedicated annual labor, premises cost, compressed air charged while machine power
//! is charged, and cooling water charged.
//!
//! # Loading sensitivity
//!
//! [`sensitivity`] copies the input, changes only `loading` and returns the base outcome plus one
//! outcome per alternative loading in the caller's order. It never mutates the input, and the
//! scenario at the input's own loading equals the base outcome exactly.
//!
//! # Publication
//!
//! [`validate_for_publication`] returns the recommended calculation's hard errors, or, when the
//! calculation is valid, the stable-identity errors a first publication needs: a nil machine id and
//! an absent or blank cost-center code. An empty answer means the input may be published; warnings
//! never block and are not returned here.
//!
//! # Version policy
//!
//! [`CALCULATOR_VERSION`] names the arithmetic an immutable publication snapshot used. Any change to
//! the formulas, the derived defaults, the statuses or the supported scope needs a new version and a
//! contract review; adding a diagnostic for an input that already had no usable result does not
//! change a valid input's arithmetic.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod engine;
mod parity;
mod recommended;

use rust_decimal::Decimal;
use solidbom_contract::{
    CalculationMode, DiagnosticSeverity, MachineRateInput, MachineRateOutcome, MachineRateSensitivity, MachineRateSensitivityRequest,
    MachineRateSensitivityScenario, RateDiagnostic, RateDiagnosticCode,
};

/// The calculator version recorded beside an immutable publication snapshot.
pub const CALCULATOR_VERSION: &str = "solidbom-machine-rate/1";

/// Calculate one machine rate version's fully absorbed rate.
///
/// The function is pure and deterministic. It never panics: a missing or invalid input is an
/// explained outcome, and an unrepresentable operation is an `arithmetic_overflow` diagnostic with
/// no result. The crate docs state the arithmetic, the statuses and the two calculation modes.
pub fn calculate(input: &MachineRateInput, mode: CalculationMode) -> MachineRateOutcome {
    match mode {
        CalculationMode::Recommended => recommended::calculate(input),
        CalculationMode::ExcelParity => parity::calculate(input),
    }
}

/// Recalculate the base rate and one scenario per alternative loading, changing only `loading`.
///
/// The loadings are validated exactly like `MachineRateSensitivityRequest::validate`: at least one,
/// at most 64, each within `[0, 1]`. The input is copied, never mutated, and the scenarios keep the
/// caller's order. The scenario at the input's own loading equals the base outcome exactly, and a
/// loading of zero is a valid `no_run_hours` scenario.
pub fn sensitivity(input: &MachineRateInput, loadings: &[Decimal], mode: CalculationMode) -> solidbom_contract::Result<MachineRateSensitivity> {
    MachineRateSensitivityRequest { loadings: loadings.to_vec() }.validate()?;
    let base = calculate(input, mode);
    let mut scenarios = Vec::with_capacity(loadings.len());
    for loading in loadings {
        let mut scenario_input = input.clone();
        scenario_input.loading = Some(*loading);
        scenarios.push(MachineRateSensitivityScenario { loading: *loading, outcome: calculate(&scenario_input, mode) });
    }
    let answer = MachineRateSensitivity { base, scenarios };
    answer.validate()?;
    Ok(answer)
}

/// List the hard errors that keep this input from being published as an approved cost-center rate.
///
/// The recommended calculation's errors come first; when the calculation is valid, the identity
/// errors a first publication needs are added: `missing_machine_identity` for a nil machine id and
/// `missing_machine_code` for an absent or blank cost-center code. An empty answer means the input
/// may be published; business warnings do not block and are not returned.
pub fn validate_for_publication(input: &MachineRateInput) -> Vec<RateDiagnostic> {
    let outcome = calculate(input, CalculationMode::Recommended);
    let mut errors: Vec<RateDiagnostic> =
        outcome.diagnostics.into_iter().filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error).collect();
    if !errors.is_empty() {
        return errors;
    }
    if input.machine.machine_id.is_nil() {
        errors.push(RateDiagnostic::error(
            RateDiagnosticCode::MissingMachineIdentity,
            "machine.machine_id",
            "a publication needs a stable non-nil machine identity",
        ));
    }
    let code_missing = input.machine.code.as_deref().is_none_or(|code| code.trim().is_empty());
    if code_missing {
        errors.push(RateDiagnostic::error(
            RateDiagnosticCode::MissingMachineCode,
            "machine.code",
            "a publication needs a non-empty cost-center code; the imported workbook leaves it blank",
        ));
    }
    errors
}

#[cfg(test)]
mod calculate_tests;
#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod sensitivity_tests;
