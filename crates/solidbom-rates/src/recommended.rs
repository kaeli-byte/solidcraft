//! The recommended mode: strict validation of the authoritative rate-version input.
//!
//! Every numeric assumption is required and explicit; a blank workbook cell is never coerced to
//! zero and the 20 % residual default is not applied. A missing or invalid value is a structured
//! hard error, and an input with any hard error returns `invalid` with no result at all, so an
//! apparently usable number can never come from corrected arithmetic.

use rust_decimal::Decimal;
use solidbom_contract::{MAX_PLANNING_YEAR, MIN_PLANNING_YEAR, MachineRateInput, MachineRateOutcome, RateDiagnostic, RateDiagnosticCode};

use crate::engine::{Resolved, arithmetic_overflow, finish, invalid, missing_input, negative_input};

/// Calculate the production outcome of one input with the recommended mode's strict validation.
pub(crate) fn calculate(input: &MachineRateInput) -> MachineRateOutcome {
    let errors = validate(input);
    if !errors.is_empty() {
        return invalid(errors);
    }
    match assemble(input) {
        Some(resolved) => finish(&resolved, Vec::new()),
        None => invalid(vec![arithmetic_overflow("machine_rate")]),
    }
}

/// Collect every hard error of the contract's recommended mode.
fn validate(input: &MachineRateInput) -> Vec<RateDiagnostic> {
    let mut errors = Vec::new();

    // Planning years: the financial year is required and both years must be plausible.
    let financial_year = match input.financial_year {
        Some(year) if !(MIN_PLANNING_YEAR..=MAX_PLANNING_YEAR).contains(&year) => {
            errors.push(RateDiagnostic::error(
                RateDiagnosticCode::InvalidYear,
                "financial_year",
                format!("the financial year must be between {MIN_PLANNING_YEAR} and {MAX_PLANNING_YEAR}"),
            ));
            None
        }
        Some(year) => Some(year),
        None => {
            errors.push(RateDiagnostic::error(
                RateDiagnosticCode::MissingFinancialYear,
                "financial_year",
                "a financial year is required: it dates the rate and derives the remaining depreciation life",
            ));
            None
        }
    };
    let acquisition_year = match input.machine.acquisition_year {
        Some(year) if !(MIN_PLANNING_YEAR..=MAX_PLANNING_YEAR).contains(&year) => {
            errors.push(RateDiagnostic::error(
                RateDiagnosticCode::InvalidYear,
                "acquisition_year",
                format!("the acquisition year must be between {MIN_PLANNING_YEAR} and {MAX_PLANNING_YEAR}"),
            ));
            None
        }
        Some(year) => Some(year),
        None => None,
    };
    if let (Some(financial_year), Some(acquisition_year)) = (financial_year, acquisition_year)
        && financial_year < acquisition_year
    {
        errors.push(RateDiagnostic::error(
            RateDiagnosticCode::FinancialYearBeforeAcquisitionYear,
            "financial_year",
            "the financial year is earlier than the acquisition year; the rate cannot depreciate a machine it does not own yet",
        ));
    }

    // A required numeric input is an explicit value. A negative one is never a usable assumption.
    macro_rules! required {
        ($errors:ident, $field:ident) => {{
            let name = stringify!($field);
            match input.$field {
                None => {
                    $errors.push(missing_input(name));
                    None
                }
                Some(value) if value.is_sign_negative() => {
                    $errors.push(negative_input(name));
                    None
                }
                Some(value) => Some(value),
            }
        }};
    }
    // The bounded fields carry a more specific diagnostic than a plain negative-input one.
    macro_rules! present {
        ($errors:ident, $field:ident) => {{
            let name = stringify!($field);
            match input.$field {
                None => {
                    $errors.push(missing_input(name));
                    None
                }
                Some(value) => Some(value),
            }
        }};
    }
    // For inputs whose value is not used again after validation.
    macro_rules! check_required {
        ($errors:ident, $field:ident) => {{
            let name = stringify!($field);
            match input.$field {
                None => $errors.push(missing_input(name)),
                Some(value) if value.is_sign_negative() => $errors.push(negative_input(name)),
                Some(_) => {}
            }
        }};
    }

    // Capacity.
    let working_days_per_year = required!(errors, working_days_per_year);
    let shifts_per_day = required!(errors, shifts_per_day);
    let hours_per_shift = required!(errors, hours_per_shift);
    let loading = present!(errors, loading);
    let availability = present!(errors, availability);

    // CAPEX and life.
    let machine_purchase_capex = required!(errors, machine_purchase_capex);
    let installation_capex = required!(errors, installation_capex);
    let dedicated_auxiliary_capex = required!(errors, dedicated_auxiliary_capex);
    let residual_value = required!(errors, residual_value);
    let useful_life_years = required!(errors, useful_life_years);

    // Fixed operating costs, excluding depreciation.
    check_required!(errors, annual_maintenance);
    check_required!(errors, annual_insurance);
    check_required!(errors, floor_space_m2);
    check_required!(errors, annual_premises_rate_per_m2);
    check_required!(errors, other_fixed_machine_overhead);
    check_required!(errors, dedicated_annual_labor);
    check_required!(errors, committed_operators);
    check_required!(errors, committed_labor_base_hourly_pay);
    let committed_labor_burden_rate = present!(errors, committed_labor_burden_rate);

    // Utilities, wear, consumables and run-only labor.
    check_required!(errors, installed_running_power_kw);
    let average_running_power_factor = present!(errors, average_running_power_factor);
    check_required!(errors, electricity_tariff);
    check_required!(errors, standby_power_kw);
    check_required!(errors, compressed_air_nm3_per_running_hour);
    check_required!(errors, compressed_air_tariff);
    check_required!(errors, cooling_water_m3_per_running_hour);
    check_required!(errors, cooling_water_tariff);
    check_required!(errors, other_utilities_per_running_hour);
    check_required!(errors, wear_parts_per_running_hour);
    check_required!(errors, operating_consumables_per_running_hour);
    check_required!(errors, run_only_operators);
    check_required!(errors, run_only_labor_base_hourly_pay);
    let run_only_labor_burden_rate = present!(errors, run_only_labor_burden_rate);
    check_required!(errors, other_variable_cost_per_running_hour);

    // Capacity ranges. Zero running hours is a valid scenario later; a zero-capacity input is not.
    if let Some(value) = working_days_per_year
        && (value <= Decimal::ZERO || value > Decimal::from(366))
    {
        errors.push(RateDiagnostic::error(
            RateDiagnosticCode::InvalidCapacity,
            "working_days_per_year",
            "working_days_per_year must be positive and at most 366",
        ));
    }
    if let Some(value) = shifts_per_day
        && (value <= Decimal::ZERO || value > Decimal::from(4))
    {
        errors.push(RateDiagnostic::error(RateDiagnosticCode::InvalidCapacity, "shifts_per_day", "shifts_per_day must be within (0, 4]"));
    }
    if let Some(value) = hours_per_shift
        && (value <= Decimal::ZERO || value > Decimal::from(24))
    {
        errors.push(RateDiagnostic::error(RateDiagnosticCode::InvalidCapacity, "hours_per_shift", "hours_per_shift must be within (0, 24]"));
    }
    if let (Some(shifts), Some(hours)) = (shifts_per_day, hours_per_shift)
        && shifts.checked_mul(hours).is_some_and(|shift_hours| shift_hours > Decimal::from(24))
    {
        errors.push(RateDiagnostic::error(
            RateDiagnosticCode::InvalidCapacity,
            "shifts_per_day",
            "shifts_per_day × hours_per_shift cannot exceed 24 hours in a day",
        ));
    }

    // Loading and availability accept zero (a valid no-run-hours scenario) but not an impossible one.
    if let Some(value) = loading
        && (value < Decimal::ZERO || value > Decimal::ONE)
    {
        errors.push(RateDiagnostic::error(RateDiagnosticCode::InvalidLoading, "loading", "loading must be within [0, 1]"));
    }
    if let Some(value) = availability
        && (value < Decimal::ZERO || value > Decimal::ONE)
    {
        errors.push(RateDiagnostic::error(RateDiagnosticCode::InvalidAvailability, "availability", "availability must be within [0, 1]"));
    }
    if let Some(value) = useful_life_years
        && value <= Decimal::ZERO
    {
        errors.push(RateDiagnostic::error(RateDiagnosticCode::InvalidUsefulLife, "useful_life_years", "useful_life_years must be positive"));
    }
    if let Some(value) = average_running_power_factor
        && (value < Decimal::ZERO || value > Decimal::ONE)
    {
        errors.push(RateDiagnostic::error(
            RateDiagnosticCode::InvalidPowerFactor,
            "average_running_power_factor",
            "average_running_power_factor must be within [0, 1]",
        ));
    }
    if let Some(value) = committed_labor_burden_rate
        && (value < Decimal::ZERO || value > Decimal::ONE)
    {
        errors.push(RateDiagnostic::error(
            RateDiagnosticCode::InvalidBurden,
            "committed_labor_burden_rate",
            "a labor burden rate must be within [0, 1]",
        ));
    }
    if let Some(value) = run_only_labor_burden_rate
        && (value < Decimal::ZERO || value > Decimal::ONE)
    {
        errors.push(RateDiagnostic::error(
            RateDiagnosticCode::InvalidBurden,
            "run_only_labor_burden_rate",
            "a labor burden rate must be within [0, 1]",
        ));
    }

    // The remaining depreciation life: an explicit override must stay within the useful life; a
    // derived one needs the acquisition year. Given the checks above the derived value can never
    // exceed the useful life, because the elapsed years are non-negative and the formula clamps at
    // zero.
    match input.remaining_depreciation_years {
        Some(value) if value.is_sign_negative() => {
            errors.push(RateDiagnostic::error(
                RateDiagnosticCode::InvalidRemainingLife,
                "remaining_depreciation_years",
                "an explicit remaining depreciation life cannot be negative",
            ));
        }
        Some(value) => {
            if let Some(useful_life) = useful_life_years
                && value > useful_life
            {
                errors.push(RateDiagnostic::error(
                    RateDiagnosticCode::InvalidRemainingLife,
                    "remaining_depreciation_years",
                    "an explicit remaining depreciation life cannot exceed the useful life",
                ));
            }
        }
        None => {
            if let (Some(_), Some(_), None) = (useful_life_years, financial_year, acquisition_year) {
                errors.push(RateDiagnostic::error(
                    RateDiagnosticCode::MissingAcquisitionYear,
                    "acquisition_year",
                    "the acquisition year is required to derive the remaining depreciation life",
                ));
            }
        }
    }

    // The residual value can never exceed the installed CAPEX: that would depreciate more than the
    // machine cost.
    if let (Some(machine_purchase), Some(installation), Some(auxiliary), Some(residual)) =
        (machine_purchase_capex, installation_capex, dedicated_auxiliary_capex, residual_value)
    {
        match machine_purchase.checked_add(installation).and_then(|installed| installed.checked_add(auxiliary)) {
            Some(installed) if residual > installed => {
                errors.push(RateDiagnostic::error(
                    RateDiagnosticCode::ResidualExceedsInstalledCapex,
                    "residual_value",
                    "the residual value cannot exceed the installed CAPEX",
                ));
            }
            Some(_) => {}
            None => errors.push(arithmetic_overflow("installed_capex")),
        }
    }

    errors
}

/// Build the fully resolved input. Validation already guarantees every required value; a missing
/// one here can only come from an unrepresentable derived remaining life, which is overflow.
fn assemble(input: &MachineRateInput) -> Option<Resolved> {
    Some(Resolved {
        working_days_per_year: input.working_days_per_year?,
        shifts_per_day: input.shifts_per_day?,
        hours_per_shift: input.hours_per_shift?,
        loading: input.loading?,
        availability: input.availability?,
        machine_purchase_capex: input.machine_purchase_capex?,
        installation_capex: input.installation_capex?,
        dedicated_auxiliary_capex: input.dedicated_auxiliary_capex?,
        residual_value: input.residual_value?,
        useful_life_years: input.useful_life_years?,
        remaining_depreciation_years: remaining_depreciation_years(input)?,
        annual_maintenance: input.annual_maintenance?,
        annual_insurance: input.annual_insurance?,
        floor_space_m2: input.floor_space_m2?,
        annual_premises_rate_per_m2: input.annual_premises_rate_per_m2?,
        other_fixed_machine_overhead: input.other_fixed_machine_overhead?,
        dedicated_annual_labor: input.dedicated_annual_labor?,
        committed_operators: input.committed_operators?,
        committed_labor_base_hourly_pay: input.committed_labor_base_hourly_pay?,
        committed_labor_burden_rate: input.committed_labor_burden_rate?,
        installed_running_power_kw: input.installed_running_power_kw?,
        average_running_power_factor: input.average_running_power_factor?,
        electricity_tariff: input.electricity_tariff?,
        standby_power_kw: input.standby_power_kw?,
        compressed_air_nm3_per_running_hour: input.compressed_air_nm3_per_running_hour?,
        compressed_air_tariff: input.compressed_air_tariff?,
        cooling_water_m3_per_running_hour: input.cooling_water_m3_per_running_hour?,
        cooling_water_tariff: input.cooling_water_tariff?,
        other_utilities_per_running_hour: input.other_utilities_per_running_hour?,
        wear_parts_per_running_hour: input.wear_parts_per_running_hour?,
        operating_consumables_per_running_hour: input.operating_consumables_per_running_hour?,
        run_only_operators: input.run_only_operators?,
        run_only_labor_base_hourly_pay: input.run_only_labor_base_hourly_pay?,
        run_only_labor_burden_rate: input.run_only_labor_burden_rate?,
        other_variable_cost_per_running_hour: input.other_variable_cost_per_running_hour?,
    })
}

/// The explicit remaining-life override, or `max(useful_life − (financial_year − acquisition_year), 0)`.
fn remaining_depreciation_years(input: &MachineRateInput) -> Option<Decimal> {
    if let Some(remaining) = input.remaining_depreciation_years {
        return Some(remaining);
    }
    let useful_life = input.useful_life_years?;
    let financial_year = input.financial_year?;
    let acquisition_year = input.machine.acquisition_year?;
    let elapsed = Decimal::from(i64::from(financial_year) - i64::from(acquisition_year));
    let derived = useful_life.checked_sub(elapsed)?;
    Some(derived.max(Decimal::ZERO))
}
