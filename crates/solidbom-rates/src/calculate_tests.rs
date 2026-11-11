//! Golden, parity, edge-case, invariant and publication-validation tests for the machine-rate
//! engine.
//!
//! The parity and synthetic fixtures live in [`crate::fixtures`], together with the reconstruction
//! notes and tolerance definitions. These tests reproduce every arithmetic reference of the
//! handover's test plan (04 §4–§6) and the contract's reconciliation invariants.

use rust_decimal::Decimal;
use solidbom_contract::{
    CalculationMode, DiagnosticSeverity, MachineRateInput, MachineRateOutcome, MachineRateResult, MachineRateStatus, ProcessType, RateDiagnosticCode,
};

use crate::fixtures::*;
use crate::{calculate, validate_for_publication};

fn result(outcome: &MachineRateOutcome) -> &MachineRateResult {
    outcome.result.as_ref().expect("a calculable outcome carries its result")
}

fn per_hour_rates(result: &MachineRateResult) -> [Option<Decimal>; 7] {
    [
        result.standby_electricity_per_running_hour,
        result.variable_operating_rate,
        result.fixed_operating_rate,
        result.depreciation_rate,
        result.total_machine_hourly_rate,
        result.cash_operating_rate,
        result.annual_cost_per_loaded_scheduled_hour,
    ]
}

fn assert_invalid_with(input: &MachineRateInput, code: RateDiagnosticCode) {
    let outcome = outcome(input, CalculationMode::Recommended);
    assert_eq!(outcome.status, MachineRateStatus::Invalid, "expected an invalid outcome for {:?}", codes(&outcome));
    assert!(outcome.result.is_none(), "an invalid outcome carries no result");
    assert!(has_code(&outcome, code), "expected {code:?} in {:?}", codes(&outcome));
}

// -------------------------------------------------------------------------------------------------
// Synthetic full-branch golden vector (04 §5)
// -------------------------------------------------------------------------------------------------

#[test]
fn synthetic_golden_vector_reproduces_every_documented_component() {
    let outcome = outcome(&synthetic_input(), CalculationMode::Recommended);
    assert_eq!(outcome.status, MachineRateStatus::OkWithWarnings);
    let computed = result(&outcome);

    assert_eq!(computed.scheduled_capacity_hours, d("1600"));
    assert_eq!(computed.loaded_scheduled_hours, d("1200"));
    assert_eq!(computed.loaded_downtime_hours, d("240"));
    assert_eq!(computed.actual_running_hours, d("960"));
    assert_eq!(computed.installed_capex, d("120000"));
    assert_eq!(computed.residual_value, d("20000"));
    assert_eq!(computed.remaining_depreciation_years, d("0.5"));
    assert_eq!(computed.annual_depreciation, d("5000"));
    assert_eq!(computed.annual_premises_cost, d("12000"));
    assert_eq!(computed.loaded_committed_labor_rate, d("50"));
    assert_eq!(computed.annual_committed_shift_labor, d("30000"));
    assert_eq!(computed.annual_fixed_operating_cost, d("54000"));
    assert_eq!(computed.running_electricity_per_hour, d("24"));
    assert_eq!(computed.annual_standby_electricity, d("960"));
    assert_eq!(computed.compressed_air_per_hour, d("2"));
    assert_eq!(computed.cooling_water_per_hour, d("2"));
    assert_eq!(computed.run_only_labor_per_hour, d("12.5"));
    assert_eq!(computed.variable_cost_excluding_standby, d("54"));
    assert_eq!(computed.annual_variable_operating_cost, d("52800"));
    assert_eq!(computed.annual_fully_absorbed_cost, d("111800"));

    assert_eq!(computed.fixed_operating_rate, Some(d("56.25")));
    assert_eq!(computed.variable_operating_rate, Some(d("55")));
    assert_eq!(computed.cash_operating_rate, Some(d("111.25")));
    assert_parity_close(computed.standby_electricity_per_running_hour.expect("a per-hour rate"), d("1"));
    assert_parity_close(computed.depreciation_rate.expect("a per-hour rate"), d("5.208333333333333333"));
    assert_parity_close(computed.total_machine_hourly_rate.expect("the total rate"), d("116.458333333333333333"));
    assert_parity_close(computed.annual_cost_per_loaded_scheduled_hour.expect("a loaded-hour rate"), d("93.166666666666666667"));
}

#[test]
fn synthetic_golden_vector_at_loading_zero_point_five() {
    let mut input = synthetic_input();
    input.loading = Some(d("0.50"));
    let outcome = outcome(&input, CalculationMode::Recommended);
    let computed = result(&outcome);

    assert_eq!(computed.loaded_scheduled_hours, d("800"));
    assert_eq!(computed.actual_running_hours, d("640"));
    assert_eq!(computed.annual_committed_shift_labor, d("20000"));
    assert_eq!(computed.annual_fixed_operating_cost, d("44000"));
    assert_eq!(computed.annual_standby_electricity, d("640"));
    assert_eq!(computed.annual_fully_absorbed_cost, d("84200"));
    assert_eq!(computed.total_machine_hourly_rate, Some(d("131.5625")));
}

// -------------------------------------------------------------------------------------------------
// Workbook baselines and Extrusion Line 1's component vector (04 §4)
// -------------------------------------------------------------------------------------------------

#[test]
fn every_workbook_baseline_matches_its_documented_aggregates_in_parity_mode() {
    for baseline in &WORKBOOK_BASELINES {
        let input = workbook_input(baseline);
        let outcome = outcome(&input, CalculationMode::ExcelParity);
        assert_eq!(outcome.status, MachineRateStatus::OkWithWarnings, "{} parity status", baseline.key);
        let computed = result(&outcome);
        assert_parity_close(computed.installed_capex, d(baseline.installed_capex));
        assert_parity_close(computed.annual_depreciation, d(baseline.annual_depreciation));
        assert_parity_close(computed.fixed_operating_rate.expect("a fixed rate"), d(baseline.fixed_rate));
        assert_parity_close(computed.variable_operating_rate.expect("a variable rate"), d(baseline.variable_rate_including_standby));
        assert_parity_close(computed.depreciation_rate.expect("a depreciation rate"), d(baseline.depreciation_rate));
        assert_parity_close(computed.total_machine_hourly_rate.expect("a total rate"), d(baseline.total_rate));
        assert_parity_close(computed.annual_fully_absorbed_cost, d(baseline.annual_full_cost));
        // A reconstructed fixture omits the residual value, so the workbook's 20 % default applies.
        if baseline.annual_fixed_remainder.is_some() {
            assert_parity_close(computed.residual_value, computed.installed_capex * Decimal::new(20, 2));
        }
        // The workbook's shared capacity state.
        assert_eq!(computed.scheduled_capacity_hours, d("4000"));
        assert_eq!(computed.loaded_scheduled_hours, d("3400"));
        assert_eq!(computed.loaded_downtime_hours, d("340"));
        assert_eq!(computed.actual_running_hours, d("3060"));
    }
}

#[test]
fn extrusion_line_one_component_vector_is_reproduced_exactly() {
    let outcome = outcome(&extrusion_line_one_input(), CalculationMode::ExcelParity);
    let computed = result(&outcome);

    assert_eq!(computed.scheduled_capacity_hours, d("4000"));
    assert_eq!(computed.loaded_scheduled_hours, d("3400"));
    assert_eq!(computed.loaded_downtime_hours, d("340"));
    assert_eq!(computed.actual_running_hours, d("3060"));
    assert_eq!(computed.installed_capex, d("3755065.50"));
    assert_eq!(computed.residual_value, d("751013.10"));
    assert_eq!(computed.remaining_depreciation_years, d("0"));
    assert_eq!(computed.annual_depreciation, d("0"));
    assert_eq!(computed.annual_premises_cost, d("121680"));
    assert_eq!(computed.loaded_committed_labor_rate, d("36.66"));
    assert_eq!(computed.annual_committed_shift_labor, d("12464.40"));
    assert_eq!(computed.annual_fixed_operating_cost, d("155905.40"));
    assert_eq!(computed.running_electricity_per_hour, d("127.50"));
    assert_eq!(computed.annual_standby_electricity, d("867"));
    assert_parity_close(computed.standby_electricity_per_running_hour.expect("standby per hour"), d("0.283333333333"));
    assert_eq!(computed.variable_cost_excluding_standby, d("150.46"));
    assert_parity_close(computed.variable_operating_rate.expect("variable rate"), d("150.743333333333"));
    assert_parity_close(computed.fixed_operating_rate.expect("fixed rate"), d("50.949477124183"));
    assert_eq!(computed.depreciation_rate, Some(Decimal::ZERO));
    assert_parity_close(computed.total_machine_hourly_rate.expect("total rate"), d("201.692810457516"));
    assert_parity_close(computed.cash_operating_rate.expect("cash rate"), d("201.692810457516"));
    assert_eq!(computed.annual_variable_operating_cost, d("461274.60"));
    assert_eq!(computed.annual_fully_absorbed_cost, d("617180"));
    assert_parity_close(computed.annual_cost_per_loaded_scheduled_hour.expect("loaded-hour rate"), d("181.523529411765"));
}

#[test]
fn oven_forming_parity_reproduces_the_workbook_number_while_recommended_refuses_the_blank_year() {
    let oven = baseline("Oven Forming");
    let parity = outcome(&workbook_input(oven), CalculationMode::ExcelParity);
    assert_eq!(parity.status, MachineRateStatus::OkWithWarnings);
    let computed = result(&parity);
    assert_eq!(computed.remaining_depreciation_years, d("2034"));
    assert_eq!(computed.annual_depreciation, d("9549.0264"));
    assert_parity_close(computed.total_machine_hourly_rate.expect("the parity rate"), d("107.9716752941"));
    assert_parity_close(computed.annual_fully_absorbed_cost, d("330393.3264"));
    assert!(has_code(&parity, RateDiagnosticCode::BlankTreatedAsZero));
    assert!(has_code(&parity, RateDiagnosticCode::ResidualDefaultedToTwentyPercent));
    assert!(has_code(&parity, RateDiagnosticCode::RemainingLifeExceedsUsefulLife));
    assert!(has_code(&parity, RateDiagnosticCode::WorkbookInputsFlagged));

    let recommended = outcome(&recommended_workbook_input(oven), CalculationMode::Recommended);
    assert_eq!(recommended.status, MachineRateStatus::Invalid);
    assert!(recommended.result.is_none());
    assert!(has_code(&recommended, RateDiagnosticCode::MissingFinancialYear));
}

#[test]
fn recommended_mode_reproduces_the_workbook_aggregates_without_parity_defaults() {
    for baseline in &WORKBOOK_BASELINES {
        if baseline.key == "Oven Forming" {
            continue;
        }
        let input = recommended_workbook_input(baseline);
        let outcome = outcome(&input, CalculationMode::Recommended);
        assert!(
            matches!(outcome.status, MachineRateStatus::Ok | MachineRateStatus::OkWithWarnings),
            "{} recommended status {:?}",
            baseline.key,
            outcome.status
        );
        let computed = result(&outcome);
        assert_parity_close(computed.annual_depreciation, d(baseline.annual_depreciation));
        assert_parity_close(computed.total_machine_hourly_rate.expect("a total rate"), d(baseline.total_rate));
        assert_parity_close(computed.annual_fully_absorbed_cost, d(baseline.annual_full_cost));
        // No recommended outcome is explained by a workbook default.
        assert!(!has_code(&outcome, RateDiagnosticCode::BlankTreatedAsZero), "{}", baseline.key);
        assert!(!has_code(&outcome, RateDiagnosticCode::ResidualDefaultedToTwentyPercent), "{}", baseline.key);
    }
}

#[test]
fn parity_never_turns_an_input_quality_problem_into_an_invalid_outcome() {
    let mut input = synthetic_input();
    input.financial_year = None;
    input.residual_value = None;
    input.annual_insurance = None;
    input.electricity_tariff = None;
    input.shifts_per_day = Some(d("4"));
    input.hours_per_shift = Some(d("12"));
    input.loading = Some(d("1.5"));
    let parity = outcome(&input, CalculationMode::ExcelParity);
    assert_ne!(parity.status, MachineRateStatus::Invalid);
    assert!(result(&parity).total_machine_hourly_rate.is_some());
    assert!(has_code(&parity, RateDiagnosticCode::WorkbookInputsFlagged));
    assert!(has_code(&parity, RateDiagnosticCode::BlankTreatedAsZero));

    // The one honest parity exception: a non-positive useful life divides by zero in the workbook.
    let mut zero_life = synthetic_input();
    zero_life.useful_life_years = Some(Decimal::ZERO);
    let invalid = outcome(&zero_life, CalculationMode::ExcelParity);
    assert_eq!(invalid.status, MachineRateStatus::Invalid);
    assert!(invalid.result.is_none());
    assert!(has_code(&invalid, RateDiagnosticCode::InvalidUsefulLife));
}

// -------------------------------------------------------------------------------------------------
// Recommended-mode validation (04 §6 and the contract's hard errors)
// -------------------------------------------------------------------------------------------------

#[test]
fn recommended_mode_never_turns_a_missing_authoritative_input_into_zero() {
    for field in REQUIRED_NUMERIC_FIELDS {
        let mut input = synthetic_input();
        set_numeric(&mut input, field, None);
        let outcome = outcome(&input, CalculationMode::Recommended);
        assert_eq!(outcome.status, MachineRateStatus::Invalid, "{field} missing");
        assert!(outcome.result.is_none(), "{field} missing leaves no result");
        let named = outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == RateDiagnosticCode::MissingInput && diagnostic.field.as_deref() == Some(field));
        assert!(named, "{field} missing should be a named missing_input error: {:?}", codes(&outcome));
    }
}

#[test]
fn recommended_mode_rejects_a_negative_value_for_every_input() {
    for field in REQUIRED_NUMERIC_FIELDS {
        let mut input = synthetic_input();
        set_numeric(&mut input, field, Some(Decimal::NEGATIVE_ONE));
        let outcome = outcome(&input, CalculationMode::Recommended);
        assert_eq!(outcome.status, MachineRateStatus::Invalid, "{field} negative");
        assert!(outcome.result.is_none(), "{field} negative leaves no result");
        assert!(!outcome.diagnostics.is_empty(), "{field} negative needs an explanation");
    }
}

#[test]
fn the_contract_hard_errors_are_reported_with_their_codes() {
    let mut missing_year = synthetic_input();
    missing_year.financial_year = None;
    assert_invalid_with(&missing_year, RateDiagnosticCode::MissingFinancialYear);

    let mut missing_acquisition = synthetic_input();
    missing_acquisition.machine.acquisition_year = None;
    missing_acquisition.remaining_depreciation_years = None;
    assert_invalid_with(&missing_acquisition, RateDiagnosticCode::MissingAcquisitionYear);

    let mut year_before_acquisition = synthetic_input();
    year_before_acquisition.machine.acquisition_year = Some(2030);
    assert_invalid_with(&year_before_acquisition, RateDiagnosticCode::FinancialYearBeforeAcquisitionYear);

    let mut implausible_acquisition = synthetic_input();
    implausible_acquisition.machine.acquisition_year = Some(1500);
    assert_invalid_with(&implausible_acquisition, RateDiagnosticCode::InvalidYear);

    let mut implausible_financial_year = synthetic_input();
    implausible_financial_year.financial_year = Some(1500);
    assert_invalid_with(&implausible_financial_year, RateDiagnosticCode::InvalidYear);

    let mut zero_useful_life = synthetic_input();
    zero_useful_life.useful_life_years = Some(Decimal::ZERO);
    assert_invalid_with(&zero_useful_life, RateDiagnosticCode::InvalidUsefulLife);

    let mut residual_above_capex = synthetic_input();
    residual_above_capex.residual_value = Some(d("120000.01"));
    assert_invalid_with(&residual_above_capex, RateDiagnosticCode::ResidualExceedsInstalledCapex);

    let mut too_many_hours = synthetic_input();
    too_many_hours.shifts_per_day = Some(d("4"));
    too_many_hours.hours_per_shift = Some(d("7"));
    assert_invalid_with(&too_many_hours, RateDiagnosticCode::InvalidCapacity);

    let mut no_working_days = synthetic_input();
    no_working_days.working_days_per_year = Some(Decimal::ZERO);
    assert_invalid_with(&no_working_days, RateDiagnosticCode::InvalidCapacity);

    let mut too_many_shifts = synthetic_input();
    too_many_shifts.shifts_per_day = Some(d("5"));
    assert_invalid_with(&too_many_shifts, RateDiagnosticCode::InvalidCapacity);

    let mut too_long_a_shift = synthetic_input();
    too_long_a_shift.hours_per_shift = Some(d("25"));
    assert_invalid_with(&too_long_a_shift, RateDiagnosticCode::InvalidCapacity);

    let mut loading_above_one = synthetic_input();
    loading_above_one.loading = Some(d("1.01"));
    assert_invalid_with(&loading_above_one, RateDiagnosticCode::InvalidLoading);

    let mut availability_above_one = synthetic_input();
    availability_above_one.availability = Some(d("1.01"));
    assert_invalid_with(&availability_above_one, RateDiagnosticCode::InvalidAvailability);

    let mut power_factor_above_one = synthetic_input();
    power_factor_above_one.average_running_power_factor = Some(d("1.01"));
    assert_invalid_with(&power_factor_above_one, RateDiagnosticCode::InvalidPowerFactor);

    let mut burden_above_one = synthetic_input();
    burden_above_one.committed_labor_burden_rate = Some(d("1.01"));
    assert_invalid_with(&burden_above_one, RateDiagnosticCode::InvalidBurden);

    let mut run_only_burden_above_one = synthetic_input();
    run_only_burden_above_one.run_only_labor_burden_rate = Some(d("1.01"));
    assert_invalid_with(&run_only_burden_above_one, RateDiagnosticCode::InvalidBurden);

    let mut negative_remaining = synthetic_input();
    negative_remaining.remaining_depreciation_years = Some(Decimal::NEGATIVE_ONE);
    assert_invalid_with(&negative_remaining, RateDiagnosticCode::InvalidRemainingLife);

    let mut remaining_above_useful_life = synthetic_input();
    remaining_above_useful_life.remaining_depreciation_years = Some(d("11"));
    assert_invalid_with(&remaining_above_useful_life, RateDiagnosticCode::InvalidRemainingLife);
}

#[test]
fn an_overflowing_input_is_an_arithmetic_overflow_diagnostic_never_a_panic() {
    let mut input = synthetic_input();
    input.annual_maintenance = Some(Decimal::MAX);
    input.annual_insurance = Some(Decimal::MAX);
    let recommended = outcome(&input, CalculationMode::Recommended);
    assert_eq!(recommended.status, MachineRateStatus::Invalid);
    assert!(recommended.result.is_none());
    assert!(has_code(&recommended, RateDiagnosticCode::ArithmeticOverflow));

    // Parity keeps its numbers where numbers exist, but an unrepresentable operation is still an
    // explained overflow rather than a panic.
    let parity = outcome(&input, CalculationMode::ExcelParity);
    assert_eq!(parity.status, MachineRateStatus::Invalid);
    assert!(has_code(&parity, RateDiagnosticCode::ArithmeticOverflow));
}

// -------------------------------------------------------------------------------------------------
// Edge cases (04 §6)
// -------------------------------------------------------------------------------------------------

#[test]
fn zero_loading_is_a_valid_no_run_hours_scenario_with_annual_costs() {
    let mut input = synthetic_input();
    input.loading = Some(Decimal::ZERO);
    let outcome = outcome(&input, CalculationMode::Recommended);
    assert_eq!(outcome.status, MachineRateStatus::NoRunHours);
    let computed = result(&outcome);
    assert_eq!(computed.actual_running_hours, Decimal::ZERO);
    assert_eq!(computed.loaded_scheduled_hours, Decimal::ZERO);
    // The synthetic case at loading zero returns only the fixed allocations and depreciation.
    assert_eq!(computed.annual_fully_absorbed_cost, d("29000"));
    assert!(computed.has_undefined_per_hour_rates());
    assert!(per_hour_rates(computed).iter().all(Option::is_none));
}

#[test]
fn zero_availability_is_a_valid_no_run_hours_scenario_without_dividing_by_zero() {
    let mut input = synthetic_input();
    input.availability = Some(Decimal::ZERO);
    let outcome = outcome(&input, CalculationMode::Recommended);
    assert_eq!(outcome.status, MachineRateStatus::NoRunHours);
    let computed = result(&outcome);
    assert_eq!(computed.actual_running_hours, Decimal::ZERO);
    assert_eq!(computed.installed_capex, d("120000"));
    assert_eq!(computed.annual_fully_absorbed_cost, d("63800"));
    assert!(computed.has_undefined_per_hour_rates());
}

#[test]
fn fully_depreciated_and_half_year_remaining_life_prorate_the_charge() {
    let mut half_year = synthetic_input();
    half_year.remaining_depreciation_years = Some(d("0.5"));
    assert_eq!(result(&outcome(&half_year, CalculationMode::Recommended)).annual_depreciation, d("5000"));

    let mut fully_depreciated = synthetic_input();
    fully_depreciated.remaining_depreciation_years = Some(Decimal::ZERO);
    let fully_depreciated_outcome = outcome(&fully_depreciated, CalculationMode::Recommended);
    let computed = result(&fully_depreciated_outcome);
    assert_eq!(computed.annual_depreciation, Decimal::ZERO);
    assert_eq!(computed.depreciation_rate, Some(Decimal::ZERO));

    // A derived remaining life that has already elapsed clamps at zero.
    let mut elapsed = synthetic_input();
    elapsed.remaining_depreciation_years = None;
    elapsed.machine.acquisition_year = Some(2010);
    let elapsed_outcome = outcome(&elapsed, CalculationMode::Recommended);
    let derived = result(&elapsed_outcome);
    assert_eq!(derived.remaining_depreciation_years, Decimal::ZERO);
    assert_eq!(derived.annual_depreciation, Decimal::ZERO);
}

#[test]
fn residual_equal_to_installed_capex_is_valid_with_a_warning() {
    let mut input = synthetic_input();
    input.residual_value = Some(d("120000"));
    let outcome = outcome(&input, CalculationMode::Recommended);
    assert_eq!(outcome.status, MachineRateStatus::OkWithWarnings);
    assert!(has_code(&outcome, RateDiagnosticCode::ResidualEqualsInstalledCapex));
    assert_eq!(result(&outcome).annual_depreciation, Decimal::ZERO);
}

#[test]
fn committed_operators_zero_removes_only_committed_labor() {
    let mut input = synthetic_input();
    input.committed_operators = Some(Decimal::ZERO);
    let committed_outcome = outcome(&input, CalculationMode::Recommended);
    let computed = result(&committed_outcome);
    assert_eq!(computed.annual_committed_shift_labor, Decimal::ZERO);
    assert_eq!(computed.annual_fixed_operating_cost, d("24000"));
    assert_eq!(computed.variable_cost_excluding_standby, d("54"));
}

#[test]
fn standby_power_zero_removes_only_the_standby_cost() {
    let mut input = synthetic_input();
    input.standby_power_kw = Some(Decimal::ZERO);
    let standby_outcome = outcome(&input, CalculationMode::Recommended);
    let computed = result(&standby_outcome);
    assert_eq!(computed.annual_standby_electricity, Decimal::ZERO);
    assert_eq!(computed.standby_electricity_per_running_hour, Some(Decimal::ZERO));
    assert_eq!(computed.variable_operating_rate, Some(d("54")));
    assert_parity_close(computed.total_machine_hourly_rate.expect("a total rate"), d("115.458333333333333333"));
}

#[test]
fn a_power_factor_of_one_uses_the_measured_average_kw() {
    let mut input = synthetic_input();
    input.average_running_power_factor = Some(Decimal::ONE);
    let factor_outcome = outcome(&input, CalculationMode::Recommended);
    let computed = result(&factor_outcome);
    assert_eq!(computed.running_electricity_per_hour, d("40"));
}

#[test]
fn fractional_operators_are_supported_without_rounding() {
    let input = synthetic_input();
    let input_outcome = outcome(&input, CalculationMode::Recommended);
    let computed = result(&input_outcome);
    assert_eq!(computed.annual_committed_shift_labor, d("30000"));
    assert_eq!(computed.run_only_labor_per_hour, d("12.5"));
}

#[test]
fn a_manual_operation_uses_the_same_engine() {
    let outcome = outcome(&manual_operation_input(), CalculationMode::Recommended);
    assert_eq!(outcome.status, MachineRateStatus::Ok);
    let computed = result(&outcome);
    assert_eq!(computed.installed_capex, Decimal::ZERO);
    assert_eq!(computed.annual_depreciation, Decimal::ZERO);
    assert_eq!(computed.annual_fixed_operating_cost, d("69000"));
    assert_eq!(computed.variable_cost_excluding_standby, Decimal::ZERO);
    assert_parity_close(computed.total_machine_hourly_rate.expect("a total rate"), d("22.549019607843137255"));
}

#[test]
fn explicit_zero_is_distinct_from_missing_but_both_are_required_to_be_explicit() {
    let mut zero_insurance = synthetic_input();
    zero_insurance.annual_insurance = Some(Decimal::ZERO);
    assert_ne!(outcome(&zero_insurance, CalculationMode::Recommended).status, MachineRateStatus::Invalid);

    let mut missing_insurance = synthetic_input();
    missing_insurance.annual_insurance = None;
    assert_eq!(outcome(&missing_insurance, CalculationMode::Recommended).status, MachineRateStatus::Invalid);
}

// -------------------------------------------------------------------------------------------------
// Reconciliation invariants (contract §Calculation and 04 §7)
// -------------------------------------------------------------------------------------------------

fn assert_reconciliation(input: &MachineRateInput, mode: CalculationMode) {
    let outcome = outcome(input, mode);
    let computed = result(&outcome);
    if computed.actual_running_hours <= Decimal::ZERO {
        return;
    }
    let total = computed.total_machine_hourly_rate.expect("a defined total rate");
    let fixed = computed.fixed_operating_rate.expect("a defined fixed rate");
    let variable = computed.variable_operating_rate.expect("a defined variable rate");
    let depreciation = computed.depreciation_rate.expect("a defined depreciation rate");
    let cash = computed.cash_operating_rate.expect("a defined cash rate");

    assert_eq!(computed.loaded_downtime_hours + computed.actual_running_hours, computed.loaded_scheduled_hours);
    assert_arithmetic_close(total, fixed + variable + depreciation);
    assert_arithmetic_close(cash, total - depreciation);
    assert_arithmetic_close(computed.annual_fully_absorbed_cost, total * computed.actual_running_hours);
    assert_arithmetic_close(computed.annual_variable_operating_cost, variable * computed.actual_running_hours);
    assert_arithmetic_close(
        computed.annual_cost_per_loaded_scheduled_hour.expect("a defined loaded-hour rate") * computed.loaded_scheduled_hours,
        computed.annual_fully_absorbed_cost,
    );
    assert_arithmetic_close(
        computed.annual_fully_absorbed_cost,
        computed.annual_fixed_operating_cost + computed.annual_depreciation + computed.annual_variable_operating_cost,
    );
}

#[test]
fn reconciliation_invariants_hold_for_every_fixture() {
    assert_reconciliation(&synthetic_input(), CalculationMode::Recommended);
    assert_reconciliation(&manual_operation_input(), CalculationMode::Recommended);
    for baseline in &WORKBOOK_BASELINES {
        assert_reconciliation(&workbook_input(baseline), CalculationMode::ExcelParity);
    }
}

#[test]
fn business_warnings_name_every_unconfirmed_cost_boundary() {
    let synthetic = outcome(&synthetic_input(), CalculationMode::Recommended);
    for code in [
        RateDiagnosticCode::CommittedAndDedicatedLabor,
        RateDiagnosticCode::PremisesAllocationUnconfirmed,
        RateDiagnosticCode::CompressedAirBoundaryUnconfirmed,
        RateDiagnosticCode::CoolingWaterBasisUnconfirmed,
    ] {
        assert!(has_code(&synthetic, code), "the synthetic full-branch vector should warn {code:?}");
    }
    assert!(synthetic.diagnostics.iter().all(|diagnostic| diagnostic.severity == DiagnosticSeverity::Warning));

    let mut power_without_tariff = synthetic_input();
    power_without_tariff.electricity_tariff = Some(Decimal::ZERO);
    assert!(has_code(&outcome(&power_without_tariff, CalculationMode::Recommended), RateDiagnosticCode::PowerWithoutTariff));

    let mut tariff_without_power = synthetic_input();
    tariff_without_power.installed_running_power_kw = Some(Decimal::ZERO);
    assert!(has_code(&outcome(&tariff_without_power, CalculationMode::Recommended), RateDiagnosticCode::TariffWithoutPower));

    let mut air_without_tariff = synthetic_input();
    air_without_tariff.compressed_air_tariff = Some(Decimal::ZERO);
    let air_outcome = outcome(&air_without_tariff, CalculationMode::Recommended);
    assert!(air_outcome.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == RateDiagnosticCode::UtilityConsumptionWithoutTariff && diagnostic.field.as_deref() == Some("compressed_air_tariff")
    }));

    let mut water_tariff_without_consumption = synthetic_input();
    water_tariff_without_consumption.cooling_water_m3_per_running_hour = Some(Decimal::ZERO);
    let water_outcome = outcome(&water_tariff_without_consumption, CalculationMode::Recommended);
    assert!(water_outcome.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == RateDiagnosticCode::UtilityTariffWithoutConsumption
            && diagnostic.field.as_deref() == Some("cooling_water_m3_per_running_hour")
    }));
}

#[test]
fn the_calculator_version_is_the_frozen_wire_value() {
    assert_eq!(crate::CALCULATOR_VERSION, "solidbom-machine-rate/1");
}

// -------------------------------------------------------------------------------------------------
// Publication validation and determinism
// -------------------------------------------------------------------------------------------------

#[test]
fn publication_validation_accepts_a_complete_identified_machine() {
    assert!(validate_for_publication(&synthetic_input()).is_empty());
}

#[test]
fn publication_validation_names_a_missing_identity_and_cost_center_code() {
    let mut nil_id = synthetic_input();
    nil_id.machine.machine_id = uuid::Uuid::nil();
    let errors = validate_for_publication(&nil_id);
    assert!(errors.iter().any(|diagnostic| diagnostic.code == RateDiagnosticCode::MissingMachineIdentity));

    let mut missing_code = synthetic_input();
    missing_code.machine.code = None;
    let errors = validate_for_publication(&missing_code);
    assert!(errors.iter().any(|diagnostic| diagnostic.code == RateDiagnosticCode::MissingMachineCode));

    let mut blank_code = synthetic_input();
    blank_code.machine.code = Some("   ".to_string());
    let errors = validate_for_publication(&blank_code);
    assert!(errors.iter().any(|diagnostic| diagnostic.code == RateDiagnosticCode::MissingMachineCode));

    // Calculation errors come first and are the whole answer when the calculation is invalid.
    let mut invalid = synthetic_input();
    invalid.financial_year = None;
    invalid.machine.code = None;
    let errors = validate_for_publication(&invalid);
    assert!(!errors.is_empty());
    assert!(errors.iter().all(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error));
    assert!(errors.iter().any(|diagnostic| diagnostic.code == RateDiagnosticCode::MissingFinancialYear));
}

#[test]
fn every_fixture_outcome_validates_and_serializes_deterministically() {
    for baseline in &WORKBOOK_BASELINES {
        for mode in [CalculationMode::Recommended, CalculationMode::ExcelParity] {
            let input = recommended_workbook_input(baseline);
            let first = outcome(&input, mode);
            let second = calculate(&input, mode);
            assert_eq!(first, second);
            assert_eq!(
                serde_json::to_string(&first).expect("an outcome serializes"),
                serde_json::to_string(&second).expect("an outcome serializes"),
                "{} must be deterministic in {mode:?}",
                baseline.key
            );
        }
    }
    let synthetic = synthetic_input();
    assert_eq!(calculate(&synthetic, CalculationMode::Recommended), calculate(&synthetic, CalculationMode::Recommended));
}

#[test]
fn a_machine_identity_name_change_does_not_change_the_numbers() {
    let plain = outcome(&synthetic_input(), CalculationMode::Recommended);
    let mut renamed = synthetic_input();
    renamed.machine.name = "A renamed display name".to_string();
    renamed.machine.brand = Some("Another brand".to_string());
    renamed.machine.process_type = ProcessType::Extrusion;
    assert_eq!(plain, outcome(&renamed, CalculationMode::Recommended));
}
