//! The Excel-parity mode: reproduce the supplied workbook's arithmetic and documented defects.
//!
//! Parity exists only to keep the published workbook references reproducible. Blank numeric cells
//! become zero, a missing residual value becomes 20 % of installed CAPEX, a missing financial year
//! becomes year zero for the derived remaining life, and the workbook's own `CHECK INPUTS` guard
//! becomes a warning. A parity outcome is never used to publish a rate.
//!
//! Parity never turns an input-quality problem into a hard error, because the documented reference
//! numbers must stay calculable. The one exception is arithmetic the workbook itself cannot
//! evaluate: a non-positive useful life divides by zero in the workbook, so no number exists to
//! preserve and the outcome is `invalid` rather than an invented value.

use rust_decimal::Decimal;
use solidbom_contract::{MachineRateInput, MachineRateOutcome, RateDiagnostic, RateDiagnosticCode};

use crate::engine::{Resolved, arithmetic_overflow, finish, invalid};

/// Calculate one input with the workbook's parity arithmetic.
pub(crate) fn calculate(input: &MachineRateInput) -> MachineRateOutcome {
    let mut warnings = Vec::new();
    let mut blank_cells = false;

    // Every numeric assumption: a blank workbook cell evaluates as zero in Excel.
    macro_rules! blank_or_zero {
        ($blank:ident, $field:ident) => {{
            match input.$field {
                Some(value) => value,
                None => {
                    $blank = true;
                    Decimal::ZERO
                }
            }
        }};
    }

    let working_days_per_year = blank_or_zero!(blank_cells, working_days_per_year);
    let shifts_per_day = blank_or_zero!(blank_cells, shifts_per_day);
    let hours_per_shift = blank_or_zero!(blank_cells, hours_per_shift);
    let loading = blank_or_zero!(blank_cells, loading);
    let availability = blank_or_zero!(blank_cells, availability);
    let machine_purchase_capex = blank_or_zero!(blank_cells, machine_purchase_capex);
    let installation_capex = blank_or_zero!(blank_cells, installation_capex);
    let dedicated_auxiliary_capex = blank_or_zero!(blank_cells, dedicated_auxiliary_capex);
    let useful_life_years = blank_or_zero!(blank_cells, useful_life_years);
    let annual_maintenance = blank_or_zero!(blank_cells, annual_maintenance);
    let annual_insurance = blank_or_zero!(blank_cells, annual_insurance);
    let floor_space_m2 = blank_or_zero!(blank_cells, floor_space_m2);
    let annual_premises_rate_per_m2 = blank_or_zero!(blank_cells, annual_premises_rate_per_m2);
    let other_fixed_machine_overhead = blank_or_zero!(blank_cells, other_fixed_machine_overhead);
    let dedicated_annual_labor = blank_or_zero!(blank_cells, dedicated_annual_labor);
    let committed_operators = blank_or_zero!(blank_cells, committed_operators);
    let committed_labor_base_hourly_pay = blank_or_zero!(blank_cells, committed_labor_base_hourly_pay);
    let committed_labor_burden_rate = blank_or_zero!(blank_cells, committed_labor_burden_rate);
    let installed_running_power_kw = blank_or_zero!(blank_cells, installed_running_power_kw);
    let average_running_power_factor = blank_or_zero!(blank_cells, average_running_power_factor);
    let electricity_tariff = blank_or_zero!(blank_cells, electricity_tariff);
    let standby_power_kw = blank_or_zero!(blank_cells, standby_power_kw);
    let compressed_air_nm3_per_running_hour = blank_or_zero!(blank_cells, compressed_air_nm3_per_running_hour);
    let compressed_air_tariff = blank_or_zero!(blank_cells, compressed_air_tariff);
    let cooling_water_m3_per_running_hour = blank_or_zero!(blank_cells, cooling_water_m3_per_running_hour);
    let cooling_water_tariff = blank_or_zero!(blank_cells, cooling_water_tariff);
    let other_utilities_per_running_hour = blank_or_zero!(blank_cells, other_utilities_per_running_hour);
    let wear_parts_per_running_hour = blank_or_zero!(blank_cells, wear_parts_per_running_hour);
    let operating_consumables_per_running_hour = blank_or_zero!(blank_cells, operating_consumables_per_running_hour);
    let run_only_operators = blank_or_zero!(blank_cells, run_only_operators);
    let run_only_labor_base_hourly_pay = blank_or_zero!(blank_cells, run_only_labor_base_hourly_pay);
    let run_only_labor_burden_rate = blank_or_zero!(blank_cells, run_only_labor_burden_rate);
    let other_variable_cost_per_running_hour = blank_or_zero!(blank_cells, other_variable_cost_per_running_hour);

    // Installed CAPEX before the residual default: the 20 % parity residual is 20 % of the installed
    // CAPEX, not of the machine purchase alone.
    let installed_capex =
        machine_purchase_capex.checked_add(installation_capex).and_then(|installed| installed.checked_add(dedicated_auxiliary_capex));
    let Some(installed_capex) = installed_capex else {
        return invalid(vec![arithmetic_overflow("installed_capex")]);
    };

    let residual_value = match input.residual_value {
        Some(value) => value,
        None => {
            warnings.push(RateDiagnostic::warning(
                RateDiagnosticCode::ResidualDefaultedToTwentyPercent,
                "residual_value",
                "the workbook's residual value is 20 % of installed CAPEX; it is a parity default, not an approved accounting policy",
            ));
            let Some(residual) = installed_capex.checked_mul(Decimal::new(20, 2)) else {
                return invalid(vec![arithmetic_overflow("residual_value")]);
            };
            residual
        }
    };

    // A blank financial year is year zero in the workbook's derived remaining life; a blank
    // acquisition year is year zero in the same formula.
    let financial_year = match input.financial_year {
        Some(year) => year,
        None => {
            blank_cells = true;
            0
        }
    };
    let acquisition_year = match input.machine.acquisition_year {
        Some(year) => year,
        None => {
            blank_cells = true;
            0
        }
    };

    if useful_life_years <= Decimal::ZERO {
        // Excel divides by zero here and its status column says CHECK INPUTS; there is no reference
        // number to preserve, so an invalid outcome is the honest parity answer.
        return invalid(vec![RateDiagnostic::error(
            RateDiagnosticCode::InvalidUsefulLife,
            "useful_life_years",
            "the workbook's useful life is not positive, so its depreciation formula divides by zero and has no number to reproduce",
        )]);
    }

    let remaining_depreciation_years = match input.remaining_depreciation_years {
        Some(value) => value,
        None => {
            let elapsed = Decimal::from(i64::from(financial_year) - i64::from(acquisition_year));
            let Some(derived) = useful_life_years.checked_sub(elapsed) else {
                return invalid(vec![arithmetic_overflow("remaining_depreciation_years")]);
            };
            derived.max(Decimal::ZERO)
        }
    };

    // The workbook's own status guard: shifts × hours > 24, useful life <= 0, residual value >
    // installed CAPEX, or remaining years > useful life. The last one is also the derived-life
    // warning of the contract. Oven Forming's missing financial year triggers it through the year
    // zero reading.
    let shift_hours = shifts_per_day.checked_mul(hours_per_shift);
    let shift_hours_above_a_day = shift_hours.is_some_and(|value| value > Decimal::from(24));
    let residual_above_installed_capex = residual_value > installed_capex;
    let remaining_above_useful_life = remaining_depreciation_years > useful_life_years;
    if remaining_above_useful_life {
        warnings.push(RateDiagnostic::warning(
            RateDiagnosticCode::RemainingLifeExceedsUsefulLife,
            "remaining_depreciation_years",
            format!(
                "the derived remaining depreciation life ({remaining_depreciation_years}) exceeds the useful life ({useful_life_years}); the workbook displays CHECK INPUTS"
            ),
        ));
    }
    if shift_hours_above_a_day || residual_above_installed_capex || remaining_above_useful_life {
        warnings.push(RateDiagnostic::warning(
            RateDiagnosticCode::WorkbookInputsFlagged,
            "machine_rate",
            "the workbook's own status column would display CHECK INPUTS for these inputs",
        ));
    }
    if blank_cells {
        warnings.push(RateDiagnostic::warning(
            RateDiagnosticCode::BlankTreatedAsZero,
            "machine_rate",
            "one or more blank workbook numeric cells (including a missing planning year) were treated as zero (Excel parity only)",
        ));
    }

    let resolved = Resolved {
        working_days_per_year,
        shifts_per_day,
        hours_per_shift,
        loading,
        availability,
        machine_purchase_capex,
        installation_capex,
        dedicated_auxiliary_capex,
        residual_value,
        useful_life_years,
        remaining_depreciation_years,
        annual_maintenance,
        annual_insurance,
        floor_space_m2,
        annual_premises_rate_per_m2,
        other_fixed_machine_overhead,
        dedicated_annual_labor,
        committed_operators,
        committed_labor_base_hourly_pay,
        committed_labor_burden_rate,
        installed_running_power_kw,
        average_running_power_factor,
        electricity_tariff,
        standby_power_kw,
        compressed_air_nm3_per_running_hour,
        compressed_air_tariff,
        cooling_water_m3_per_running_hour,
        cooling_water_tariff,
        other_utilities_per_running_hour,
        wear_parts_per_running_hour,
        operating_consumables_per_running_hour,
        run_only_operators,
        run_only_labor_base_hourly_pay,
        run_only_labor_burden_rate,
        other_variable_cost_per_running_hour,
    };
    finish(&resolved, warnings)
}
