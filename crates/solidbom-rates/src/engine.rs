//! The one arithmetic both calculation modes share, and the diagnostics that explain it.
//!
//! A [`Resolved`] already has every assumption as an explicit decimal: the recommended resolver
//! refuses incomplete input, and the parity resolver fills the workbook's blanks and defaults. The
//! calculator then performs checked decimal arithmetic once, so the two modes cannot drift apart.
//! Every operation is checked; [`compute`] returns `None` when an operation has no representable
//! result, and [`finish`] turns that into an `arithmetic_overflow` diagnostic and an invalid outcome
//! instead of a panic.

use rust_decimal::Decimal;
use solidbom_contract::{MachineRateOutcome, MachineRateResult, MachineRateStatus, RateDiagnostic, RateDiagnosticCode};

/// Every machine assumption the arithmetic needs, already resolved to an explicit decimal.
pub(crate) struct Resolved {
    pub(crate) working_days_per_year: Decimal,
    pub(crate) shifts_per_day: Decimal,
    pub(crate) hours_per_shift: Decimal,
    pub(crate) loading: Decimal,
    pub(crate) availability: Decimal,
    pub(crate) machine_purchase_capex: Decimal,
    pub(crate) installation_capex: Decimal,
    pub(crate) dedicated_auxiliary_capex: Decimal,
    pub(crate) residual_value: Decimal,
    pub(crate) useful_life_years: Decimal,
    pub(crate) remaining_depreciation_years: Decimal,
    pub(crate) annual_maintenance: Decimal,
    pub(crate) annual_insurance: Decimal,
    pub(crate) floor_space_m2: Decimal,
    pub(crate) annual_premises_rate_per_m2: Decimal,
    pub(crate) other_fixed_machine_overhead: Decimal,
    pub(crate) dedicated_annual_labor: Decimal,
    pub(crate) committed_operators: Decimal,
    pub(crate) committed_labor_base_hourly_pay: Decimal,
    pub(crate) committed_labor_burden_rate: Decimal,
    pub(crate) installed_running_power_kw: Decimal,
    pub(crate) average_running_power_factor: Decimal,
    pub(crate) electricity_tariff: Decimal,
    pub(crate) standby_power_kw: Decimal,
    pub(crate) compressed_air_nm3_per_running_hour: Decimal,
    pub(crate) compressed_air_tariff: Decimal,
    pub(crate) cooling_water_m3_per_running_hour: Decimal,
    pub(crate) cooling_water_tariff: Decimal,
    pub(crate) other_utilities_per_running_hour: Decimal,
    pub(crate) wear_parts_per_running_hour: Decimal,
    pub(crate) operating_consumables_per_running_hour: Decimal,
    pub(crate) run_only_operators: Decimal,
    pub(crate) run_only_labor_base_hourly_pay: Decimal,
    pub(crate) run_only_labor_burden_rate: Decimal,
    pub(crate) other_variable_cost_per_running_hour: Decimal,
}

/// Sum a list of terms with checked addition.
fn sum(terms: &[Decimal]) -> Option<Decimal> {
    let mut total = Decimal::ZERO;
    for term in terms {
        total = total.checked_add(*term)?;
    }
    Some(total)
}

/// Assemble the complete result, or return `None` when a checked operation has no representable
/// result.
///
/// Every input is present in `resolved`, so `None` means overflow (the arithmetic is validated, so
/// there is no division by zero). Zero or negative running hours leave the seven per-hour fields
/// `None` instead of dividing by zero.
fn compute(resolved: &Resolved) -> Option<MachineRateResult> {
    let scheduled_capacity_hours = resolved.working_days_per_year.checked_mul(resolved.shifts_per_day)?.checked_mul(resolved.hours_per_shift)?;
    let loaded_scheduled_hours = scheduled_capacity_hours.checked_mul(resolved.loading)?;
    let loaded_downtime_hours = loaded_scheduled_hours.checked_mul(Decimal::ONE.checked_sub(resolved.availability)?)?;
    let actual_running_hours = loaded_scheduled_hours.checked_mul(resolved.availability)?;

    let installed_capex =
        resolved.machine_purchase_capex.checked_add(resolved.installation_capex)?.checked_add(resolved.dedicated_auxiliary_capex)?;
    // The remaining-life factor prorates a fractional final year and never exceeds one full year.
    let remaining_factor = if resolved.remaining_depreciation_years < Decimal::ONE { resolved.remaining_depreciation_years } else { Decimal::ONE };
    let annual_depreciation =
        installed_capex.checked_sub(resolved.residual_value)?.checked_div(resolved.useful_life_years)?.checked_mul(remaining_factor)?;

    let annual_premises_cost = resolved.floor_space_m2.checked_mul(resolved.annual_premises_rate_per_m2)?;
    let loaded_committed_labor_rate =
        resolved.committed_labor_base_hourly_pay.checked_mul(Decimal::ONE.checked_add(resolved.committed_labor_burden_rate)?)?;
    let annual_committed_shift_labor = resolved.committed_operators.checked_mul(loaded_committed_labor_rate)?.checked_mul(loaded_scheduled_hours)?;
    let annual_fixed_operating_cost = sum(&[
        resolved.annual_maintenance,
        resolved.annual_insurance,
        annual_premises_cost,
        resolved.other_fixed_machine_overhead,
        resolved.dedicated_annual_labor,
        annual_committed_shift_labor,
    ])?;

    let running_electricity_per_hour =
        resolved.installed_running_power_kw.checked_mul(resolved.average_running_power_factor)?.checked_mul(resolved.electricity_tariff)?;
    let annual_standby_electricity = resolved.standby_power_kw.checked_mul(resolved.electricity_tariff)?.checked_mul(loaded_downtime_hours)?;
    let compressed_air_per_hour = resolved.compressed_air_nm3_per_running_hour.checked_mul(resolved.compressed_air_tariff)?;
    let cooling_water_per_hour = resolved.cooling_water_m3_per_running_hour.checked_mul(resolved.cooling_water_tariff)?;
    let run_only_labor_per_hour = resolved
        .run_only_operators
        .checked_mul(resolved.run_only_labor_base_hourly_pay)?
        .checked_mul(Decimal::ONE.checked_add(resolved.run_only_labor_burden_rate)?)?;
    let variable_cost_excluding_standby = sum(&[
        running_electricity_per_hour,
        compressed_air_per_hour,
        cooling_water_per_hour,
        resolved.other_utilities_per_running_hour,
        resolved.wear_parts_per_running_hour,
        resolved.operating_consumables_per_running_hour,
        run_only_labor_per_hour,
        resolved.other_variable_cost_per_running_hour,
    ])?;

    let annual_variable_operating_cost =
        variable_cost_excluding_standby.checked_mul(actual_running_hours)?.checked_add(annual_standby_electricity)?;
    let annual_fully_absorbed_cost = sum(&[annual_fixed_operating_cost, annual_depreciation, annual_variable_operating_cost])?;

    let (
        standby_electricity_per_running_hour,
        variable_operating_rate,
        fixed_operating_rate,
        depreciation_rate,
        total_machine_hourly_rate,
        cash_operating_rate,
        annual_cost_per_loaded_scheduled_hour,
    ) = if actual_running_hours > Decimal::ZERO {
        let standby_electricity_per_running_hour = annual_standby_electricity.checked_div(actual_running_hours)?;
        let fixed_operating_rate = annual_fixed_operating_cost.checked_div(actual_running_hours)?;
        let variable_operating_rate = variable_cost_excluding_standby.checked_add(standby_electricity_per_running_hour)?;
        let depreciation_rate = annual_depreciation.checked_div(actual_running_hours)?;
        let total_machine_hourly_rate = sum(&[annual_fixed_operating_cost, annual_depreciation, annual_standby_electricity])?
            .checked_div(actual_running_hours)?
            .checked_add(variable_cost_excluding_standby)?;
        let cash_operating_rate = sum(&[annual_fixed_operating_cost, annual_standby_electricity])?
            .checked_div(actual_running_hours)?
            .checked_add(variable_cost_excluding_standby)?;
        let annual_cost_per_loaded_scheduled_hour = annual_fully_absorbed_cost.checked_div(loaded_scheduled_hours)?;
        (
            Some(standby_electricity_per_running_hour),
            Some(variable_operating_rate),
            Some(fixed_operating_rate),
            Some(depreciation_rate),
            Some(total_machine_hourly_rate),
            Some(cash_operating_rate),
            Some(annual_cost_per_loaded_scheduled_hour),
        )
    } else {
        (None, None, None, None, None, None, None)
    };

    Some(MachineRateResult {
        scheduled_capacity_hours,
        loaded_scheduled_hours,
        loaded_downtime_hours,
        actual_running_hours,
        installed_capex,
        residual_value: resolved.residual_value,
        remaining_depreciation_years: resolved.remaining_depreciation_years,
        annual_depreciation,
        annual_premises_cost,
        loaded_committed_labor_rate,
        annual_committed_shift_labor,
        annual_fixed_operating_cost,
        running_electricity_per_hour,
        annual_standby_electricity,
        compressed_air_per_hour,
        cooling_water_per_hour,
        run_only_labor_per_hour,
        variable_cost_excluding_standby,
        annual_variable_operating_cost,
        annual_fully_absorbed_cost,
        standby_electricity_per_running_hour,
        variable_operating_rate,
        fixed_operating_rate,
        depreciation_rate,
        total_machine_hourly_rate,
        cash_operating_rate,
        annual_cost_per_loaded_scheduled_hour,
    })
}

/// Finish a mode's calculation: add the business warnings, choose the status and explain overflow.
///
/// `diagnostics` are the warnings the mode's own resolution already produced (the parity defaults);
/// the recommended mode passes none.
pub(crate) fn finish(resolved: &Resolved, diagnostics: Vec<RateDiagnostic>) -> MachineRateOutcome {
    let Some(result) = compute(resolved) else {
        let mut diagnostics = diagnostics;
        diagnostics.push(arithmetic_overflow("machine_rate"));
        return invalid(diagnostics);
    };
    let mut diagnostics = diagnostics;
    diagnostics.extend(business_warnings(resolved, &result));
    let status = if result.actual_running_hours > Decimal::ZERO {
        if diagnostics.is_empty() { MachineRateStatus::Ok } else { MachineRateStatus::OkWithWarnings }
    } else {
        MachineRateStatus::NoRunHours
    };
    MachineRateOutcome { status, result: Some(result), diagnostics }
}

/// The business-review warnings of the contract. They change no arithmetic.
fn business_warnings(resolved: &Resolved, result: &MachineRateResult) -> Vec<RateDiagnostic> {
    let mut warnings = Vec::new();
    if result.installed_capex > Decimal::ZERO && result.residual_value == result.installed_capex {
        warnings.push(RateDiagnostic::warning(
            RateDiagnosticCode::ResidualEqualsInstalledCapex,
            "residual_value",
            "the residual value equals the installed CAPEX, so no depreciation is charged; confirm the accounting policy",
        ));
    }
    if resolved.installed_running_power_kw > Decimal::ZERO && resolved.electricity_tariff == Decimal::ZERO {
        warnings.push(RateDiagnostic::warning(
            RateDiagnosticCode::PowerWithoutTariff,
            "electricity_tariff",
            "installed power is positive while the electricity tariff is zero; confirm the tariff",
        ));
    }
    if resolved.electricity_tariff > Decimal::ZERO && resolved.installed_running_power_kw == Decimal::ZERO {
        warnings.push(RateDiagnostic::warning(
            RateDiagnosticCode::TariffWithoutPower,
            "installed_running_power_kw",
            "the electricity tariff is positive while no installed running power is entered; confirm the power",
        ));
    }
    if resolved.compressed_air_nm3_per_running_hour > Decimal::ZERO && resolved.compressed_air_tariff == Decimal::ZERO {
        warnings.push(RateDiagnostic::warning(
            RateDiagnosticCode::UtilityConsumptionWithoutTariff,
            "compressed_air_tariff",
            "compressed-air consumption is positive while its tariff is zero; confirm the tariff",
        ));
    }
    if resolved.cooling_water_m3_per_running_hour > Decimal::ZERO && resolved.cooling_water_tariff == Decimal::ZERO {
        warnings.push(RateDiagnostic::warning(
            RateDiagnosticCode::UtilityConsumptionWithoutTariff,
            "cooling_water_tariff",
            "cooling-water consumption is positive while its tariff is zero; confirm the tariff",
        ));
    }
    if resolved.compressed_air_tariff > Decimal::ZERO && resolved.compressed_air_nm3_per_running_hour == Decimal::ZERO {
        warnings.push(RateDiagnostic::warning(
            RateDiagnosticCode::UtilityTariffWithoutConsumption,
            "compressed_air_nm3_per_running_hour",
            "a compressed-air tariff is entered while no consumption is; confirm the consumption",
        ));
    }
    if resolved.cooling_water_tariff > Decimal::ZERO && resolved.cooling_water_m3_per_running_hour == Decimal::ZERO {
        warnings.push(RateDiagnostic::warning(
            RateDiagnosticCode::UtilityTariffWithoutConsumption,
            "cooling_water_m3_per_running_hour",
            "a cooling-water tariff is entered while no consumption is; confirm the consumption",
        ));
    }
    if resolved.committed_operators > Decimal::ZERO && resolved.dedicated_annual_labor > Decimal::ZERO {
        warnings.push(RateDiagnostic::warning(
            RateDiagnosticCode::CommittedAndDedicatedLabor,
            "dedicated_annual_labor",
            "committed shift staffing and dedicated annual labor are both non-zero; confirm no labor is charged twice",
        ));
    }
    if result.annual_premises_cost > Decimal::ZERO {
        warnings.push(RateDiagnostic::warning(
            RateDiagnosticCode::PremisesAllocationUnconfirmed,
            "annual_premises_rate_per_m2",
            "premises cost is part of the machine rate; confirm the boundary with production SG&A",
        ));
    }
    if result.compressed_air_per_hour > Decimal::ZERO && result.running_electricity_per_hour > Decimal::ZERO {
        warnings.push(RateDiagnostic::warning(
            RateDiagnosticCode::CompressedAirBoundaryUnconfirmed,
            "compressed_air_nm3_per_running_hour",
            "compressed air is charged while machine power is also charged; confirm whether compressor supply is counted twice",
        ));
    }
    if result.cooling_water_per_hour > Decimal::ZERO {
        warnings.push(RateDiagnostic::warning(
            RateDiagnosticCode::CoolingWaterBasisUnconfirmed,
            "cooling_water_m3_per_running_hour",
            "cooling water is charged; confirm the consumption basis (circulation versus net makeup)",
        ));
    }
    warnings
}

/// An invalid outcome with no result. Callers guarantee at least one error diagnostic.
pub(crate) fn invalid(diagnostics: Vec<RateDiagnostic>) -> MachineRateOutcome {
    MachineRateOutcome { status: MachineRateStatus::Invalid, result: None, diagnostics }
}

/// The diagnostic of an unrepresentable checked decimal operation.
pub(crate) fn arithmetic_overflow(field: &'static str) -> RateDiagnostic {
    RateDiagnostic::error(
        RateDiagnosticCode::ArithmeticOverflow,
        field,
        "checked decimal arithmetic overflowed; no representable value exists for this calculation",
    )
}

/// A diagnostic that a required input is missing.
pub(crate) fn missing_input(field: &'static str) -> RateDiagnostic {
    RateDiagnostic::error(RateDiagnosticCode::MissingInput, field, format!("the recommended mode requires an explicit value for {field}"))
}

/// A diagnostic that a numeric input is negative.
pub(crate) fn negative_input(field: &'static str) -> RateDiagnostic {
    RateDiagnostic::error(RateDiagnosticCode::NegativeInput, field, format!("{field} cannot be negative"))
}
