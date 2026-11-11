//! Loading-sensitivity tests: the 64 built-in workbook cases against the independent closed form,
//! Extrusion Line 1's documented sensitivity table, ordering, non-mutation and the contract's
//! request validation.
//!
//! The closed form of the handover (03 §2) is
//! `F / (S × l × a) + c × w / a + V + p × t × (1 − a) / a`, with `F` excluding committed labor. It is
//! a test reference only; the engine remains the one production implementation.

use rust_decimal::Decimal;
use solidbom_contract::{CalculationMode, MachineRateInput, MachineRateOutcome, MachineRateResult, MachineRateSensitivityRequest, MachineRateStatus};

use crate::fixtures::*;
use crate::{calculate, sensitivity};

fn result(outcome: &MachineRateOutcome) -> &MachineRateResult {
    outcome.result.as_ref().expect("a calculable outcome carries its result")
}

/// The handover's eight built-in scenario loadings.
const BUILT_IN_LOADINGS: [&str; 8] = ["0.20", "0.40", "0.50", "0.60", "0.70", "0.80", "0.90", "1.00"];

/// The independent closed form for a scenario loading, with `F` excluding committed labor.
fn closed_form_total_rate(input: &MachineRateInput, base: &MachineRateResult, loading: Decimal) -> Decimal {
    let scheduled = base.scheduled_capacity_hours;
    let availability = input.availability.expect("the fixture has an availability");
    let annual_fixed_excluding_committed = base.annual_fixed_operating_cost - base.annual_committed_shift_labor + base.annual_depreciation;
    let committed_operators = input.committed_operators.expect("the fixture has committed operators");
    let committed_rate = base.loaded_committed_labor_rate;
    let variable = base.variable_cost_excluding_standby;
    let standby_power = input.standby_power_kw.expect("the fixture has a standby power");
    let tariff = input.electricity_tariff.expect("the fixture has an electricity tariff");
    annual_fixed_excluding_committed / (scheduled * loading * availability)
        + committed_operators * committed_rate / availability
        + variable
        + standby_power * tariff * (Decimal::ONE - availability) / availability
}

#[test]
fn sixty_four_builtin_loading_cases_match_the_independent_closed_form() {
    let mut comparisons = 0;
    for baseline in WORKBOOK_BASELINES {
        let input = workbook_input(baseline);
        let base = outcome(&input, CalculationMode::ExcelParity);
        let base_result = result(&base);
        let loadings: Vec<Decimal> = BUILT_IN_LOADINGS.iter().map(|loading| d(loading)).collect();
        let scenarios = sensitivity(&input, &loadings, CalculationMode::ExcelParity).expect("the built-in loadings are valid");
        scenarios.validate().expect("a sensitivity answer validates against the contract");
        assert_eq!(scenarios.scenarios.len(), BUILT_IN_LOADINGS.len());
        for scenario in &scenarios.scenarios {
            let computed = result(&scenario.outcome);
            let expected = closed_form_total_rate(&input, base_result, scenario.loading);
            assert_arithmetic_close(computed.total_machine_hourly_rate.expect("a total rate at a positive loading"), expected);
            assert_ne!(computed.actual_running_hours, Decimal::ZERO);
            comparisons += 1;
        }
    }
    assert_eq!(comparisons, 64, "eight machines × eight built-in loadings");
}

#[test]
fn extrusion_line_one_reproduces_the_documented_sensitivity_table() {
    let table = [
        ("0.20", "354.040277777778"),
        ("0.40", "254.428472222222"),
        ("0.50", "234.506111111111"),
        ("0.60", "221.224537037037"),
        ("0.70", "211.737698412698"),
        ("0.80", "204.622569444444"),
        ("0.85", "201.692810457516"),
        ("0.90", "199.088580246914"),
        ("1.00", "194.661388888889"),
    ];
    let input = extrusion_line_one_input();
    let loadings: Vec<Decimal> = table.iter().map(|(loading, _)| d(loading)).collect();
    let scenarios = sensitivity(&input, &loadings, CalculationMode::ExcelParity).expect("the documented loadings are valid");
    for ((loading, expected), scenario) in table.iter().zip(&scenarios.scenarios) {
        assert_eq!(scenario.loading, d(loading));
        assert_parity_close(result(&scenario.outcome).total_machine_hourly_rate.expect("a total rate"), d(expected));
    }
}

#[test]
fn sensitivity_at_the_input_loading_equals_the_base_outcome_exactly() {
    let input = extrusion_line_one_input();
    let base = outcome(&input, CalculationMode::ExcelParity);
    let loadings = [d("0.20"), d("0.85"), d("0.50")];
    let scenarios = sensitivity(&input, &loadings, CalculationMode::ExcelParity).expect("valid loadings");
    assert_eq!(scenarios.base, base);
    let at_base = &scenarios.scenarios[1];
    assert_eq!(at_base.loading, d("0.85"));
    assert_eq!(at_base.outcome, base);
    assert_eq!(
        result(&at_base.outcome).total_machine_hourly_rate.expect("the total rate"),
        result(&base).total_machine_hourly_rate.expect("the total rate")
    );
}

#[test]
fn sensitivity_preserves_the_caller_order_and_never_mutates_the_input() {
    let input = synthetic_input();
    let before = input.clone();
    let loadings = [d("0.90"), d("0.20"), d("0.50"), d("1.00")];
    let scenarios = sensitivity(&input, &loadings, CalculationMode::Recommended).expect("valid loadings");
    let order: Vec<Decimal> = scenarios.scenarios.iter().map(|scenario| scenario.loading).collect();
    assert_eq!(order, loadings);
    assert_eq!(input, before, "a sensitivity request changes no input");
    assert_eq!(scenarios.base, calculate(&before, CalculationMode::Recommended));
}

#[test]
fn a_loading_of_zero_is_a_valid_no_run_hours_scenario() {
    let input = synthetic_input();
    let scenarios = sensitivity(&input, &[Decimal::ZERO, d("0.50")], CalculationMode::Recommended).expect("zero is a valid loading");
    let zero = &scenarios.scenarios[0];
    assert_eq!(zero.outcome.status, MachineRateStatus::NoRunHours);
    let computed = result(&zero.outcome);
    assert_eq!(computed.actual_running_hours, Decimal::ZERO);
    assert_eq!(computed.annual_fully_absorbed_cost, d("29000"));
    assert!(computed.has_undefined_per_hour_rates());
    assert!(computed.total_machine_hourly_rate.is_none());
}

#[test]
fn sensitivity_validates_loadings_like_the_contract_request() {
    let input = synthetic_input();
    for invalid in [vec![], vec![Decimal::NEGATIVE_ONE], vec![d("1.01")], vec![d("0.5"); 65]] {
        let request = MachineRateSensitivityRequest { loadings: invalid.clone() };
        assert!(request.validate().is_err(), "the contract refuses this loading list");
        assert!(sensitivity(&input, &invalid, CalculationMode::Recommended).is_err(), "the engine refuses the same loading list");
    }
    assert!(MachineRateSensitivityRequest { loadings: vec![Decimal::ZERO, Decimal::ONE] }.validate().is_ok());
}

#[test]
fn sensitivity_reports_an_invalid_base_and_its_scenarios_without_panicking() {
    let mut input = synthetic_input();
    input.financial_year = None;
    let scenarios = sensitivity(&input, &[d("0.50"), d("1.00")], CalculationMode::Recommended).expect("loadings are valid");
    assert_eq!(scenarios.base.status, MachineRateStatus::Invalid);
    assert!(scenarios.base.result.is_none());
    for scenario in &scenarios.scenarios {
        assert_eq!(scenario.outcome.status, MachineRateStatus::Invalid);
        assert!(scenario.outcome.result.is_none());
    }
}

#[test]
fn sensitivity_works_on_a_parity_input_whose_blank_year_is_a_reference_only() {
    let input = workbook_input(baseline("Oven Forming"));
    let loadings = [d("0.40"), d("0.85"), d("1.00")];
    let scenarios = sensitivity(&input, &loadings, CalculationMode::ExcelParity).expect("valid loadings");
    assert_eq!(scenarios.base.status, MachineRateStatus::OkWithWarnings);
    let base_total = result(&scenarios.base).total_machine_hourly_rate.expect("the parity rate");
    assert_parity_close(result(&scenarios.scenarios[1].outcome).total_machine_hourly_rate.expect("the parity rate"), base_total);
    for scenario in &scenarios.scenarios {
        assert!(result(&scenario.outcome).total_machine_hourly_rate.is_some());
    }
}
