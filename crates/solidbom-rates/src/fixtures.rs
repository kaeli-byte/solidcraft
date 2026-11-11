//! Parity fixtures and shared test helpers for the machine hourly-rate engine.
//!
//! # Workbook parity reconstruction
//!
//! The supplied handover publishes the eight workbook aggregates (installed CAPEX, annual
//! depreciation, fixed/variable/depreciation/total rates and annual fully absorbed cost) and
//! Extrusion Line 1's detailed component vector and sensitivity table. It does **not** publish the
//! workbook's per-cell inputs for the other seven machines, and the machine workbook itself is not
//! part of the imported reference package. Those seven fixtures are therefore reconstructions whose
//! documented aggregates match within the tolerance below:
//!
//! - Every machine uses the workbook's shared capacity inputs: 250 working days, 2 shifts, 8 hours a
//!   shift, loading 0.85, availability 0.90 — 4000 scheduled, 3400 loaded, 340 downtime and 3060
//!   running hours.
//! - Installed CAPEX sits in `machine_purchase_capex`; installation and dedicated auxiliary CAPEX
//!   are zero (the documented installed total is the only published CAPEX number).
//! - The fixed-cost remainder (the documented fixed rate × 3060 running hours) sits in
//!   `other_fixed_machine_overhead`; every other fixed component and all committed labor are zero.
//! - The variable-rate remainder (the documented variable rate, standby included) sits in
//!   `other_variable_cost_per_running_hour`; every other variable component and standby power are
//!   zero, so the documented variable rate is the per-running-hour remainder itself.
//! - Useful life is 10 years; the acquisition year is 2016 (remaining life 0) for the machines with
//!   zero annual depreciation and 2020 (remaining life 6) for the machines with a full annual
//!   depreciation amount.
//! - `residual_value` is omitted from the parity inputs so the workbook's 20 % default applies; the
//!   recommended-mode variant fills the same 20 % explicitly, which keeps the aggregates identical.
//! - Oven Forming keeps its missing financial year: Excel reads the blank as year zero, which turns
//!   the derived remaining life into 2034 against a useful life of 10.
//! - Extrusion Line 1 keeps its documented component vector exactly (it is also the contract crate's
//!   machine-rate portability fixture): its own CAPEX split, residual value, staffing, utilities and
//!   premises values are documented, not reconstructed.
//!
//! None of these fixtures is an approved company rate; they are arithmetic references only.
//!
//! # Tolerances
//!
//! The workbook publishes rates rounded to 10 decimal places and annual amounts to 4, and the
//! synthetic golden vector as repeating decimals, so parity comparisons allow an **absolute 1e-6 or
//! relative 1e-10** difference. Engine-internal reconciliation and closed-form sensitivity
//! comparisons allow **absolute 1e-12 or relative 1e-18**, because both sides are exact decimal
//! arithmetic apart from `rust_decimal`'s last-digit division rounding.

use rust_decimal::Decimal;
use solidbom_contract::{CalculationMode, MachineIdentity, MachineRateInput, MachineRateOutcome, ProcessType, RateDiagnosticCode, RateScenario};
use uuid::Uuid;

use crate::calculate;

/// Absolute tolerance for comparisons against the handover's published, rounded values.
pub(crate) const PARITY_ABSOLUTE_TOLERANCE: Decimal = Decimal::from_parts(1, 0, 0, false, 6);
/// Relative tolerance for comparisons against the handover's published, rounded values.
pub(crate) const PARITY_RELATIVE_TOLERANCE: Decimal = Decimal::from_parts(1, 0, 0, false, 10);
/// Absolute tolerance for engine-internal and closed-form reconciliation.
pub(crate) const ARITHMETIC_ABSOLUTE_TOLERANCE: Decimal = Decimal::from_parts(1, 0, 0, false, 12);
/// Relative tolerance for engine-internal and closed-form reconciliation.
pub(crate) const ARITHMETIC_RELATIVE_TOLERANCE: Decimal = Decimal::from_parts(1, 0, 0, false, 18);

/// Parse a fixture decimal.
pub(crate) fn d(text: &str) -> Decimal {
    text.parse().expect("a decimal literal in a machine-rate fixture")
}

/// Assert two decimal values are equal within an absolute-or-relative tolerance.
pub(crate) fn assert_close(actual: Decimal, expected: Decimal, absolute: Decimal, relative: Decimal) {
    let difference = (actual - expected).abs();
    let allowed = absolute.max(expected.abs() * relative);
    assert!(difference <= allowed, "expected {expected}, got {actual} (difference {difference}, allowed {allowed})");
}

/// Assert a value equals a published workbook aggregate within the parity tolerance.
pub(crate) fn assert_parity_close(actual: Decimal, expected: Decimal) {
    assert_close(actual, expected, PARITY_ABSOLUTE_TOLERANCE, PARITY_RELATIVE_TOLERANCE);
}

/// Assert an engine-internal or closed-form identity within the arithmetic tolerance.
pub(crate) fn assert_arithmetic_close(actual: Decimal, expected: Decimal) {
    assert_close(actual, expected, ARITHMETIC_ABSOLUTE_TOLERANCE, ARITHMETIC_RELATIVE_TOLERANCE);
}

/// Calculate an outcome and assert that it satisfies the contract's outcome validation.
pub(crate) fn outcome(input: &MachineRateInput, mode: CalculationMode) -> MachineRateOutcome {
    let outcome = calculate(input, mode);
    outcome.validate().expect("every machine-rate engine outcome validates against the contract");
    outcome
}

/// The diagnostic codes of an outcome, in order.
pub(crate) fn codes(outcome: &MachineRateOutcome) -> Vec<RateDiagnosticCode> {
    outcome.diagnostics.iter().map(|diagnostic| diagnostic.code).collect()
}

/// Whether an outcome carries a diagnostic with this code.
pub(crate) fn has_code(outcome: &MachineRateOutcome, code: RateDiagnosticCode) -> bool {
    outcome.diagnostics.iter().any(|diagnostic| diagnostic.code == code)
}

/// Build a stable machine identity.
pub(crate) fn machine(machine_id: u128, code: Option<&str>, name: &str, process_type: ProcessType, acquisition_year: Option<i32>) -> MachineIdentity {
    MachineIdentity {
        machine_id: Uuid::from_u128(machine_id),
        code: code.map(str::to_string),
        name: name.to_string(),
        process_type,
        process_description: None,
        brand: None,
        model: None,
        acquisition_year,
        active: true,
    }
}

/// One published workbook baseline: the documented aggregates plus the fixture split that
/// reconstructs them.
pub(crate) struct WorkbookBaseline {
    pub(crate) key: &'static str,
    pub(crate) name: &'static str,
    pub(crate) machine_id: u128,
    pub(crate) process_type: ProcessType,
    pub(crate) financial_year: Option<i32>,
    pub(crate) acquisition_year: Option<i32>,
    /// Documented installed CAPEX.
    pub(crate) installed_capex: &'static str,
    /// Reconstructed fixed-cost remainder; `None` means the documented Extrusion Line 1 components.
    pub(crate) annual_fixed_remainder: Option<&'static str>,
    /// Reconstructed variable-rate remainder (standby included); `None` for Extrusion Line 1.
    pub(crate) variable_rate_remainder: Option<&'static str>,
    pub(crate) annual_depreciation: &'static str,
    pub(crate) fixed_rate: &'static str,
    pub(crate) variable_rate_including_standby: &'static str,
    pub(crate) depreciation_rate: &'static str,
    pub(crate) total_rate: &'static str,
    pub(crate) annual_full_cost: &'static str,
}

/// The eight workbook baselines of the handover's test plan (04 §4). The aggregates are documented;
/// only the seven non-Extrusion-Line-1 per-cell splits are reconstructed (see the module docs).
pub(crate) const WORKBOOK_BASELINES: &[WorkbookBaseline] = &[
    WorkbookBaseline {
        key: "Extrusion Line 1",
        name: "Extrusion Line 1",
        machine_id: 1001,
        process_type: ProcessType::Extrusion,
        financial_year: Some(2026),
        acquisition_year: Some(2016),
        installed_capex: "3755065.50",
        annual_fixed_remainder: None,
        variable_rate_remainder: None,
        annual_depreciation: "0.0000",
        fixed_rate: "50.9494771242",
        variable_rate_including_standby: "150.7433333333",
        depreciation_rate: "0.0000000000",
        total_rate: "201.6928104575",
        annual_full_cost: "617180.0000",
    },
    WorkbookBaseline {
        key: "Extrusion Line 2",
        name: "Extrusion Line 2",
        machine_id: 1002,
        process_type: ProcessType::Extrusion,
        financial_year: Some(2026),
        acquisition_year: Some(2020),
        installed_capex: "4796212.78",
        annual_fixed_remainder: Some("209872.4"),
        variable_rate_remainder: Some("180.4933333333"),
        annual_depreciation: "383697.0224",
        fixed_rate: "68.5857516340",
        variable_rate_including_standby: "180.4933333333",
        depreciation_rate: "125.3911837908",
        total_rate: "374.4702687582",
        annual_full_cost: "1145879.0224",
    },
    WorkbookBaseline {
        key: "ABC Winding",
        name: "ABC Winding",
        machine_id: 1003,
        process_type: ProcessType::Winding,
        financial_year: Some(2026),
        acquisition_year: Some(2016),
        installed_capex: "323600.00",
        annual_fixed_remainder: Some("125852.6"),
        variable_rate_remainder: Some("57.85"),
        annual_depreciation: "0.0000",
        fixed_rate: "41.1283006536",
        variable_rate_including_standby: "57.8500000000",
        depreciation_rate: "0.0000000000",
        total_rate: "98.9783006536",
        annual_full_cost: "302873.6000",
    },
    WorkbookBaseline {
        key: "Oven Forming",
        name: "Oven Forming",
        machine_id: 1004,
        process_type: ProcessType::Forming,
        // The workbook leaves the financial year blank; the parity fixture keeps that defect.
        financial_year: None,
        acquisition_year: Some(2024),
        installed_capex: "119362.83",
        annual_fixed_remainder: Some("69960.0"),
        variable_rate_remainder: Some("81.9883333333"),
        annual_depreciation: "9549.0264",
        fixed_rate: "22.8627450980",
        variable_rate_including_standby: "81.9883333333",
        depreciation_rate: "3.1205968627",
        total_rate: "107.9716752941",
        annual_full_cost: "330393.3264",
    },
    WorkbookBaseline {
        key: "Pipe Cutting",
        name: "Pipe Cutting",
        machine_id: 1005,
        process_type: ProcessType::Cutting,
        financial_year: Some(2026),
        acquisition_year: Some(2020),
        installed_capex: "189495.58",
        annual_fixed_remainder: Some("77150.0"),
        variable_rate_remainder: Some("3.9"),
        annual_depreciation: "15159.6464",
        fixed_rate: "25.2124183007",
        variable_rate_including_standby: "3.9000000000",
        depreciation_rate: "4.9541328105",
        total_rate: "34.0665511111",
        annual_full_cost: "104243.6464",
    },
    WorkbookBaseline {
        key: "Leakage Testing",
        name: "Leakage Testing",
        machine_id: 1006,
        process_type: ProcessType::Testing,
        financial_year: Some(2026),
        acquisition_year: Some(2016),
        installed_capex: "73982.16",
        annual_fixed_remainder: Some("128016.0"),
        variable_rate_remainder: Some("6.1515"),
        annual_depreciation: "0.0000",
        fixed_rate: "41.8352941176",
        variable_rate_including_standby: "6.1515000000",
        depreciation_rate: "0.0000000000",
        total_rate: "47.9867941176",
        annual_full_cost: "146839.5900",
    },
    WorkbookBaseline {
        key: "Overmolding",
        name: "Overmolding",
        machine_id: 1007,
        process_type: ProcessType::Injection,
        financial_year: Some(2026),
        acquisition_year: Some(2016),
        installed_capex: "362777.61",
        annual_fixed_remainder: Some("78444.0"),
        variable_rate_remainder: Some("33.564"),
        annual_depreciation: "0.0000",
        fixed_rate: "25.6352941176",
        variable_rate_including_standby: "33.5640000000",
        depreciation_rate: "0.0000000000",
        total_rate: "59.1992941176",
        annual_full_cost: "181149.8400",
    },
    WorkbookBaseline {
        key: "Manual operation",
        name: "Manual operation",
        machine_id: 1008,
        process_type: ProcessType::ManualWork,
        financial_year: Some(2026),
        acquisition_year: Some(2016),
        installed_capex: "0.00",
        annual_fixed_remainder: Some("125346.0"),
        variable_rate_remainder: Some("0"),
        annual_depreciation: "0.0000",
        fixed_rate: "40.9627450980",
        variable_rate_including_standby: "0.0000000000",
        depreciation_rate: "0.0000000000",
        total_rate: "40.9627450980",
        annual_full_cost: "125346.0000",
    },
];

/// The workbook baseline with this key.
pub(crate) fn baseline(key: &str) -> &'static WorkbookBaseline {
    WORKBOOK_BASELINES.iter().find(|baseline| baseline.key == key).expect("a documented workbook baseline")
}

/// A parity input for one baseline. Extrusion Line 1 uses its documented component vector; the other
/// seven use the reconstructed aggregate split described in the module docs.
pub(crate) fn workbook_input(baseline: &WorkbookBaseline) -> MachineRateInput {
    if baseline.annual_fixed_remainder.is_none() || baseline.variable_rate_remainder.is_none() {
        return extrusion_line_one_input();
    }
    let annual_fixed_remainder = d(baseline.annual_fixed_remainder.expect("a reconstructed fixed remainder"));
    let variable_rate_remainder = d(baseline.variable_rate_remainder.expect("a reconstructed variable remainder"));
    MachineRateInput {
        machine: machine(baseline.machine_id, Some("CC-1000"), baseline.name, baseline.process_type, baseline.acquisition_year),
        financial_year: baseline.financial_year,
        scenario: RateScenario::Base,
        working_days_per_year: Some(d("250")),
        shifts_per_day: Some(d("2")),
        hours_per_shift: Some(d("8")),
        loading: Some(d("0.85")),
        availability: Some(d("0.90")),
        machine_purchase_capex: Some(d(baseline.installed_capex)),
        installation_capex: Some(Decimal::ZERO),
        dedicated_auxiliary_capex: Some(Decimal::ZERO),
        // Omitted so the workbook's 20 % parity default applies; the recommended variant fills it.
        residual_value: None,
        useful_life_years: Some(d("10")),
        remaining_depreciation_years: None,
        annual_maintenance: Some(Decimal::ZERO),
        annual_insurance: Some(Decimal::ZERO),
        floor_space_m2: Some(Decimal::ZERO),
        annual_premises_rate_per_m2: Some(Decimal::ZERO),
        other_fixed_machine_overhead: Some(annual_fixed_remainder),
        dedicated_annual_labor: Some(Decimal::ZERO),
        committed_operators: Some(Decimal::ZERO),
        committed_labor_base_hourly_pay: Some(Decimal::ZERO),
        committed_labor_burden_rate: Some(Decimal::ZERO),
        installed_running_power_kw: Some(Decimal::ZERO),
        average_running_power_factor: Some(Decimal::ZERO),
        electricity_tariff: Some(Decimal::ZERO),
        standby_power_kw: Some(Decimal::ZERO),
        compressed_air_nm3_per_running_hour: Some(Decimal::ZERO),
        compressed_air_tariff: Some(Decimal::ZERO),
        cooling_water_m3_per_running_hour: Some(Decimal::ZERO),
        cooling_water_tariff: Some(Decimal::ZERO),
        other_utilities_per_running_hour: Some(Decimal::ZERO),
        wear_parts_per_running_hour: Some(Decimal::ZERO),
        operating_consumables_per_running_hour: Some(Decimal::ZERO),
        run_only_operators: Some(Decimal::ZERO),
        run_only_labor_base_hourly_pay: Some(Decimal::ZERO),
        run_only_labor_burden_rate: Some(Decimal::ZERO),
        other_variable_cost_per_running_hour: Some(variable_rate_remainder),
        source_reference: Some("machine hourly-rate workbook (parity reconstruction)".to_string()),
        assumption_notes: None,
    }
}

/// The recommended-mode variant of a workbook fixture: the same 20 % residual value made explicit,
/// so every aggregate stays identical while no authoritative input is blank.
pub(crate) fn recommended_workbook_input(baseline: &WorkbookBaseline) -> MachineRateInput {
    let mut input = workbook_input(baseline);
    if input.residual_value.is_none() {
        input.residual_value = Some(d(baseline.installed_capex) * Decimal::new(20, 2));
    }
    input
}

/// Extrusion Line 1's documented input vector (04 §4): the workbook's own component values.
pub(crate) fn extrusion_line_one_input() -> MachineRateInput {
    MachineRateInput {
        machine: machine(1001, Some("CC-1001"), "Extrusion Line 1", ProcessType::Extrusion, Some(2016)),
        financial_year: Some(2026),
        scenario: RateScenario::Base,
        working_days_per_year: Some(d("250")),
        shifts_per_day: Some(d("2")),
        hours_per_shift: Some(d("8")),
        loading: Some(d("0.85")),
        availability: Some(d("0.90")),
        machine_purchase_capex: Some(d("3700000")),
        installation_capex: Some(d("50000")),
        dedicated_auxiliary_capex: Some(d("5065.50")),
        residual_value: Some(d("751013.10")),
        useful_life_years: Some(d("10")),
        remaining_depreciation_years: None,
        annual_maintenance: Some(d("20000")),
        annual_insurance: Some(d("1000")),
        floor_space_m2: Some(d("1014")),
        annual_premises_rate_per_m2: Some(d("120")),
        other_fixed_machine_overhead: Some(d("761")),
        dedicated_annual_labor: Some(Decimal::ZERO),
        committed_operators: Some(d("0.1")),
        committed_labor_base_hourly_pay: Some(d("30")),
        committed_labor_burden_rate: Some(d("0.222")),
        installed_running_power_kw: Some(d("150")),
        average_running_power_factor: Some(d("0.85")),
        electricity_tariff: Some(Decimal::ONE),
        standby_power_kw: Some(d("2.55")),
        compressed_air_nm3_per_running_hour: Some(d("10")),
        compressed_air_tariff: Some(d("0.20")),
        cooling_water_m3_per_running_hour: Some(d("4")),
        cooling_water_tariff: Some(d("0.50")),
        other_utilities_per_running_hour: Some(d("3")),
        wear_parts_per_running_hour: Some(d("4")),
        operating_consumables_per_running_hour: Some(d("5")),
        run_only_operators: Some(d("0.1092")),
        run_only_labor_base_hourly_pay: Some(d("40")),
        run_only_labor_burden_rate: Some(d("0.25")),
        other_variable_cost_per_running_hour: Some(d("1.50")),
        source_reference: Some("machine hourly-rate workbook".to_string()),
        assumption_notes: None,
    }
}

/// The synthetic full-branch golden vector (04 §5): every branch exercised, including fractional
/// remaining life and committed labor during loaded downtime.
pub(crate) fn synthetic_input() -> MachineRateInput {
    MachineRateInput {
        machine: machine(2001, Some("CC-2001"), "Synthetic full branch", ProcessType::Forming, Some(2020)),
        financial_year: Some(2026),
        scenario: RateScenario::Base,
        working_days_per_year: Some(d("100")),
        shifts_per_day: Some(d("2")),
        hours_per_shift: Some(d("8")),
        loading: Some(d("0.75")),
        availability: Some(d("0.80")),
        machine_purchase_capex: Some(d("100000")),
        installation_capex: Some(d("10000")),
        dedicated_auxiliary_capex: Some(d("10000")),
        residual_value: Some(d("20000")),
        useful_life_years: Some(d("10")),
        // An explicit fractional override prorates the final annual depreciation amount.
        remaining_depreciation_years: Some(d("0.5")),
        annual_maintenance: Some(d("6000")),
        annual_insurance: Some(d("1000")),
        floor_space_m2: Some(d("100")),
        annual_premises_rate_per_m2: Some(d("120")),
        other_fixed_machine_overhead: Some(d("2000")),
        dedicated_annual_labor: Some(d("3000")),
        committed_operators: Some(d("0.5")),
        committed_labor_base_hourly_pay: Some(d("40")),
        committed_labor_burden_rate: Some(d("0.25")),
        installed_running_power_kw: Some(d("50")),
        average_running_power_factor: Some(d("0.60")),
        electricity_tariff: Some(d("0.80")),
        standby_power_kw: Some(d("5")),
        compressed_air_nm3_per_running_hour: Some(d("10")),
        compressed_air_tariff: Some(d("0.20")),
        cooling_water_m3_per_running_hour: Some(d("4")),
        cooling_water_tariff: Some(d("0.50")),
        other_utilities_per_running_hour: Some(d("3")),
        wear_parts_per_running_hour: Some(d("4")),
        operating_consumables_per_running_hour: Some(d("5")),
        run_only_operators: Some(d("0.25")),
        run_only_labor_base_hourly_pay: Some(d("40")),
        run_only_labor_burden_rate: Some(d("0.25")),
        other_variable_cost_per_running_hour: Some(d("1.50")),
        source_reference: Some("synthetic full-branch golden vector".to_string()),
        assumption_notes: None,
    }
}

/// A manual workstation: zero CAPEX, zero power and zero utilities; the rate consists of its
/// applicable fixed and committed-labor costs only.
pub(crate) fn manual_operation_input() -> MachineRateInput {
    MachineRateInput {
        machine: machine(3001, Some("CC-3001"), "Manual operation", ProcessType::ManualWork, Some(2016)),
        financial_year: Some(2026),
        scenario: RateScenario::Base,
        working_days_per_year: Some(d("250")),
        shifts_per_day: Some(d("2")),
        hours_per_shift: Some(d("8")),
        loading: Some(d("0.85")),
        availability: Some(d("0.90")),
        machine_purchase_capex: Some(Decimal::ZERO),
        installation_capex: Some(Decimal::ZERO),
        dedicated_auxiliary_capex: Some(Decimal::ZERO),
        residual_value: Some(Decimal::ZERO),
        useful_life_years: Some(d("10")),
        remaining_depreciation_years: Some(Decimal::ZERO),
        annual_maintenance: Some(d("1000")),
        annual_insurance: Some(Decimal::ZERO),
        floor_space_m2: Some(Decimal::ZERO),
        annual_premises_rate_per_m2: Some(Decimal::ZERO),
        other_fixed_machine_overhead: Some(Decimal::ZERO),
        dedicated_annual_labor: Some(Decimal::ZERO),
        committed_operators: Some(Decimal::ONE),
        committed_labor_base_hourly_pay: Some(d("20")),
        committed_labor_burden_rate: Some(Decimal::ZERO),
        installed_running_power_kw: Some(Decimal::ZERO),
        average_running_power_factor: Some(Decimal::ZERO),
        electricity_tariff: Some(Decimal::ZERO),
        standby_power_kw: Some(Decimal::ZERO),
        compressed_air_nm3_per_running_hour: Some(Decimal::ZERO),
        compressed_air_tariff: Some(Decimal::ZERO),
        cooling_water_m3_per_running_hour: Some(Decimal::ZERO),
        cooling_water_tariff: Some(Decimal::ZERO),
        other_utilities_per_running_hour: Some(Decimal::ZERO),
        wear_parts_per_running_hour: Some(Decimal::ZERO),
        operating_consumables_per_running_hour: Some(Decimal::ZERO),
        run_only_operators: Some(Decimal::ZERO),
        run_only_labor_base_hourly_pay: Some(Decimal::ZERO),
        run_only_labor_burden_rate: Some(Decimal::ZERO),
        other_variable_cost_per_running_hour: Some(Decimal::ZERO),
        source_reference: Some("manual operation reconstruction".to_string()),
        assumption_notes: None,
    }
}

/// The required numeric inputs of the recommended mode, in contract order (the remaining-life
/// override is optional and tested separately).
pub(crate) const REQUIRED_NUMERIC_FIELDS: &[&str] = &[
    "working_days_per_year",
    "shifts_per_day",
    "hours_per_shift",
    "loading",
    "availability",
    "machine_purchase_capex",
    "installation_capex",
    "dedicated_auxiliary_capex",
    "residual_value",
    "useful_life_years",
    "annual_maintenance",
    "annual_insurance",
    "floor_space_m2",
    "annual_premises_rate_per_m2",
    "other_fixed_machine_overhead",
    "dedicated_annual_labor",
    "committed_operators",
    "committed_labor_base_hourly_pay",
    "committed_labor_burden_rate",
    "installed_running_power_kw",
    "average_running_power_factor",
    "electricity_tariff",
    "standby_power_kw",
    "compressed_air_nm3_per_running_hour",
    "compressed_air_tariff",
    "cooling_water_m3_per_running_hour",
    "cooling_water_tariff",
    "other_utilities_per_running_hour",
    "wear_parts_per_running_hour",
    "operating_consumables_per_running_hour",
    "run_only_operators",
    "run_only_labor_base_hourly_pay",
    "run_only_labor_burden_rate",
    "other_variable_cost_per_running_hour",
];

/// Set one required numeric input by its contract field name, for the missing/negative sweep tests.
pub(crate) fn set_numeric(input: &mut MachineRateInput, field: &str, value: Option<Decimal>) {
    match field {
        "working_days_per_year" => input.working_days_per_year = value,
        "shifts_per_day" => input.shifts_per_day = value,
        "hours_per_shift" => input.hours_per_shift = value,
        "loading" => input.loading = value,
        "availability" => input.availability = value,
        "machine_purchase_capex" => input.machine_purchase_capex = value,
        "installation_capex" => input.installation_capex = value,
        "dedicated_auxiliary_capex" => input.dedicated_auxiliary_capex = value,
        "residual_value" => input.residual_value = value,
        "useful_life_years" => input.useful_life_years = value,
        "remaining_depreciation_years" => input.remaining_depreciation_years = value,
        "annual_maintenance" => input.annual_maintenance = value,
        "annual_insurance" => input.annual_insurance = value,
        "floor_space_m2" => input.floor_space_m2 = value,
        "annual_premises_rate_per_m2" => input.annual_premises_rate_per_m2 = value,
        "other_fixed_machine_overhead" => input.other_fixed_machine_overhead = value,
        "dedicated_annual_labor" => input.dedicated_annual_labor = value,
        "committed_operators" => input.committed_operators = value,
        "committed_labor_base_hourly_pay" => input.committed_labor_base_hourly_pay = value,
        "committed_labor_burden_rate" => input.committed_labor_burden_rate = value,
        "installed_running_power_kw" => input.installed_running_power_kw = value,
        "average_running_power_factor" => input.average_running_power_factor = value,
        "electricity_tariff" => input.electricity_tariff = value,
        "standby_power_kw" => input.standby_power_kw = value,
        "compressed_air_nm3_per_running_hour" => input.compressed_air_nm3_per_running_hour = value,
        "compressed_air_tariff" => input.compressed_air_tariff = value,
        "cooling_water_m3_per_running_hour" => input.cooling_water_m3_per_running_hour = value,
        "cooling_water_tariff" => input.cooling_water_tariff = value,
        "other_utilities_per_running_hour" => input.other_utilities_per_running_hour = value,
        "wear_parts_per_running_hour" => input.wear_parts_per_running_hour = value,
        "operating_consumables_per_running_hour" => input.operating_consumables_per_running_hour = value,
        "run_only_operators" => input.run_only_operators = value,
        "run_only_labor_base_hourly_pay" => input.run_only_labor_base_hourly_pay = value,
        "run_only_labor_burden_rate" => input.run_only_labor_burden_rate = value,
        "other_variable_cost_per_running_hour" => input.other_variable_cost_per_running_hour = value,
        other => panic!("unknown machine-rate field {other}"),
    }
}
