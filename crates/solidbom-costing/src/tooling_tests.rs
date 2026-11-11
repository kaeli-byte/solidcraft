//! Exact arithmetic tests for the SB-17 tooling investment, allocation and recovery calculator.
//!
//! The fixtures follow the acceptance vectors of the package requirements: two entries of 10,000 are
//! 20,000 of project investment counted once, 10,000 divided by a 5,000 recovery quantity is 2 per
//! owner unit, lifetime shares allocate that one investment, the assembly quantity doubles the
//! finished-product contribution, the contribution is never capped, the treatment switches the
//! recovery channel, a purchased-branch descendant has no demand, saved FX converts, missing and
//! invalid inputs are named, an overflow is an issue, and repeated runs are byte-identical. Money
//! and rates are decimal strings; a missing value is distinct from an explicit zero.

use std::collections::{BTreeMap, BTreeSet};

use rust_decimal::Decimal;
use solidbom_contract::{
    AnnualForecast, CostLine, ExchangeRate, ProjectTooling, ResultState, ToolingDemand, ToolingEntry, ToolingInputs, ToolingOutcome, ToolingOwner,
    ToolingPartQuantity, ToolingProductDemand, ToolingProductLine, ToolingRecoveryBasis, ToolingRecoveryChoice, ToolingRecoveryInputs,
    ToolingToolLine, ToolingTreatment,
};
use uuid::Uuid;

use super::{TOOLING_RECOVERY_CALCULATOR_VERSION, calculate_tooling, tooling_recovery_calculator_version};

fn d(text: &str) -> Decimal {
    text.parse().expect("a decimal literal in the test fixture")
}

fn id(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn part_owner(part_version_id: u128) -> ToolingOwner {
    ToolingOwner::PartVersion { part_version_id: id(part_version_id) }
}

fn product_owner(product_id: u128) -> ToolingOwner {
    ToolingOwner::Product { product_id: id(product_id) }
}

fn entry(tool_id: u128, owner: ToolingOwner, tool_count: u64, cost_per_tool: &str, currency: &str) -> ToolingEntry {
    ToolingEntry {
        tool_id: id(tool_id),
        owner,
        name: None,
        tool_count,
        cost_per_tool: d(cost_per_tool),
        currency: currency.into(),
        source_reference: None,
    }
}

fn product(product_id: u128, lifetime_demand: Option<u64>, quantities: &[(u128, u64)], forecasts: &[(u16, u64)]) -> ToolingProductDemand {
    ToolingProductDemand {
        product_id: id(product_id),
        product_revision_id: Some(id(product_id + 10_000)),
        lifetime_demand,
        forecasts: forecasts.iter().map(|(year, quantity)| AnnualForecast { year: *year, quantity: *quantity }).collect(),
        part_quantities: quantities
            .iter()
            .map(|(part_version_id, quantity)| ToolingPartQuantity { part_version_id: id(*part_version_id), quantity: *quantity })
            .collect(),
    }
}

fn manual(tool_id: u128, quantity: Option<&str>) -> ToolingRecoveryChoice {
    ToolingRecoveryChoice { tool_id: id(tool_id), basis: ToolingRecoveryBasis::ManualQuantity { quantity: quantity.map(d) } }
}

fn inputs(entries: Vec<ToolingEntry>, products: Vec<ToolingProductDemand>, treatment: Option<ToolingTreatment>) -> ToolingInputs {
    ToolingInputs {
        project_currency: "CNY".into(),
        exchange_rates: Vec::new(),
        tooling: ProjectTooling { default_treatment: treatment, entries },
        recovery: ToolingRecoveryInputs::default(),
        demand: ToolingDemand { products },
    }
}

/// One 10,000 CNY tool owned by part version 5, used once in one product whose lifetime demand is
/// 5,000 with the forecasts 2,000 and 3,000.
fn standard_input() -> ToolingInputs {
    inputs(
        vec![entry(1, part_owner(5), 1, "10000", "CNY")],
        vec![product(10, Some(5000), &[(5, 1)], &[(2027, 2000), (2028, 3000)])],
        Some(ToolingTreatment::Amortized),
    )
}

fn tool_line(outcome: &ToolingOutcome, tool_id: u128) -> &ToolingToolLine {
    outcome.tools.iter().find(|line| line.tool_id == id(tool_id)).expect("the tool line exists")
}

fn product_line(outcome: &ToolingOutcome, product_id: u128) -> &ToolingProductLine {
    outcome.products.iter().find(|line| line.product_id == id(product_id)).expect("the product line exists")
}

fn issue_codes(outcome: &ToolingOutcome) -> Vec<&str> {
    outcome.issues.iter().map(|issue| issue.code.as_str()).collect()
}

fn explanation_codes(lines: &[CostLine]) -> BTreeSet<&str> {
    lines.iter().map(|line| line.code.as_str()).collect()
}

fn explanation_amounts(lines: &[CostLine]) -> BTreeMap<&str, Decimal> {
    lines.iter().filter_map(|line| line.amount.map(|amount| (line.code.as_str(), amount))).collect()
}

fn json(outcome: &ToolingOutcome) -> String {
    serde_json::to_string(outcome).expect("the outcome serialises")
}

// ---------------------------------------------------------------------------
// The version and the zero-investment answer
// ---------------------------------------------------------------------------

#[test]
fn the_version_is_the_published_calculator_version() {
    assert_eq!(TOOLING_RECOVERY_CALCULATOR_VERSION, "solidbom-tooling-recovery/1");
    assert_eq!(tooling_recovery_calculator_version(), TOOLING_RECOVERY_CALCULATOR_VERSION);
}

#[test]
fn a_project_with_no_tool_entries_is_a_complete_zero_investment_answer() {
    let outcome = calculate_tooling(&inputs(Vec::new(), Vec::new(), None));
    assert_eq!(outcome.state, ResultState::Complete);
    assert_eq!(outcome.project_investment, Some(Decimal::ZERO));
    assert_eq!(outcome.treatment, None);
    assert!(outcome.tools.is_empty());
    assert!(outcome.products.is_empty());
    assert!(outcome.issues.is_empty());
}

#[test]
fn no_tool_entries_keep_the_products_with_explicit_zero_allocations() {
    // No treatment is chosen, yet no recovery channel applies: the answer is complete.
    let mut input = inputs(Vec::new(), vec![product(10, Some(100), &[], &[(2027, 100)])], None);
    let outcome = calculate_tooling(&input);
    assert_eq!(outcome.state, ResultState::Complete);
    assert_eq!(outcome.treatment, None);
    assert_eq!(outcome.project_investment, Some(Decimal::ZERO));
    assert_eq!(product_line(&outcome, 10).allocated_investment, Some(Decimal::ZERO));
    assert_eq!(product_line(&outcome, 10).amortized_contribution, None);

    // Under the amortized treatment the amortized contribution is the explicit zero of an empty sum.
    input.tooling.default_treatment = Some(ToolingTreatment::Amortized);
    let outcome = calculate_tooling(&input);
    assert_eq!(outcome.state, ResultState::Complete);
    assert_eq!(outcome.treatment, Some(ToolingTreatment::Amortized));
    assert_eq!(product_line(&outcome, 10).allocated_investment, Some(Decimal::ZERO));
    assert_eq!(product_line(&outcome, 10).amortized_contribution, Some(Decimal::ZERO));
    assert_eq!(outcome.project_amortized_recovered, Some(Decimal::ZERO));
    assert_eq!(outcome.project_under_over_recovery, Some(Decimal::ZERO));
    assert!(outcome.annual_recovery.is_empty());
}

// ---------------------------------------------------------------------------
// Investment counted once
// ---------------------------------------------------------------------------

#[test]
fn two_entries_of_ten_thousand_are_twenty_thousand_counted_once() {
    // Two tools of 10,000 each; the product uses the owner twice and has a lifetime demand of 100,
    // which must not multiply the investment.
    let input = inputs(
        vec![entry(1, part_owner(5), 1, "10000", "CNY"), entry(2, part_owner(5), 2, "5000", "CNY")],
        vec![product(10, Some(100), &[(5, 2)], &[(2027, 100)])],
        Some(ToolingTreatment::Amortized),
    );
    let outcome = calculate_tooling(&input);
    assert_eq!(outcome.state, ResultState::Complete);
    assert_eq!(outcome.project_investment, Some(d("20000")), "two entries of 10,000 are 20,000, not multiplied by demand");
    assert_eq!(tool_line(&outcome, 1).source_investment, Some(d("10000")));
    assert_eq!(tool_line(&outcome, 1).investment, Some(d("10000")));
    assert_eq!(tool_line(&outcome, 2).source_investment, Some(d("10000")));
    assert_eq!(tool_line(&outcome, 2).investment, Some(d("10000")));
    assert_eq!(product_line(&outcome, 10).allocated_investment, Some(d("20000")), "the whole project investment once");
    assert_ne!(product_line(&outcome, 10).allocated_investment, Some(d("4000000")), "demand never multiplies the investment");
    assert_eq!(outcome.project_amortized_recovered, Some(d("20000")));
    assert_eq!(outcome.annual_recovery, vec![solidbom_contract::ToolingAnnualRecovery { year: 2027, recovered: Some(d("20000")) }]);
}

#[test]
fn the_amortized_contribution_is_the_investment_divided_by_the_manual_quantity() {
    // The locked acceptance example: 10,000 / 5,000 = 2 per owner unit and per finished product.
    let mut input = standard_input();
    input.recovery.recovery = vec![manual(1, Some("5000"))];
    let outcome = calculate_tooling(&input);
    assert_eq!(outcome.state, ResultState::Complete);
    let tool = tool_line(&outcome, 1);
    assert_eq!(tool.investment, Some(d("10000")));
    assert_eq!(tool.recovery_quantity, Some(d("5000")));
    assert_eq!(tool.contribution_per_owner_unit, Some(d("2")));
    assert_eq!(tool.projected_recovery, Some(d("10000")));
    assert_eq!(tool.under_over_recovery, Some(Decimal::ZERO));
    assert_eq!(tool.allocation[0].product_contribution, Some(d("2")));
    assert_eq!(
        tool.annual_recovery,
        vec![
            solidbom_contract::ToolingAnnualRecovery { year: 2027, recovered: Some(d("4000")) },
            solidbom_contract::ToolingAnnualRecovery { year: 2028, recovered: Some(d("6000")) },
        ]
    );
    assert_eq!(product_line(&outcome, 10).amortized_contribution, Some(d("2")));
    assert_eq!(outcome.project_amortized_recovered, Some(d("10000")));
    assert_eq!(outcome.project_under_over_recovery, Some(Decimal::ZERO));
}

#[test]
fn a_manual_quantity_of_four_thousand_divides_exactly() {
    let mut input = standard_input();
    input.recovery.recovery = vec![manual(1, Some("4000"))];
    let outcome = calculate_tooling(&input);
    assert_eq!(outcome.state, ResultState::Complete);
    let tool = tool_line(&outcome, 1);
    assert_eq!(tool.contribution_per_owner_unit, Some(d("2.5")));
    assert_eq!(tool.projected_recovery, Some(d("12500")), "2.5 per owner unit over the 5,000 lifetime owner units");
    assert_eq!(tool.under_over_recovery, Some(d("2500")));
    assert_eq!(product_line(&outcome, 10).amortized_contribution, Some(d("2.5")));
}

// ---------------------------------------------------------------------------
// Allocation by lifetime product demand
// ---------------------------------------------------------------------------

#[test]
fn lifetime_shares_allocate_the_single_investment_across_products() {
    // Product 10: lifetime 100, one owner unit -> 100. Product 20: lifetime 200, two units -> 400.
    // Combined 500: shares 0.2 and 0.8 of the 10,000 investment.
    let input = inputs(
        vec![entry(1, part_owner(5), 1, "10000", "CNY")],
        vec![product(10, Some(100), &[(5, 1)], &[]), product(20, Some(200), &[(5, 2)], &[])],
        Some(ToolingTreatment::Amortized),
    );
    let outcome = calculate_tooling(&input);
    assert_eq!(outcome.state, ResultState::Complete);
    let tool = tool_line(&outcome, 1);
    assert_eq!(tool.allocation.len(), 2);
    assert_eq!(tool.allocation[0].product_id, id(10));
    assert_eq!(tool.allocation[0].expanded_quantity, 1);
    assert_eq!(tool.allocation[0].product_owner_demand, Some(100));
    assert_eq!(tool.allocation[0].allocation_share, Some(d("0.2")));
    assert_eq!(tool.allocation[0].allocated_investment, Some(d("2000")));
    assert_eq!(tool.allocation[0].product_contribution, Some(d("20")));
    assert_eq!(tool.allocation[1].product_id, id(20));
    assert_eq!(tool.allocation[1].expanded_quantity, 2);
    assert_eq!(tool.allocation[1].product_owner_demand, Some(400));
    assert_eq!(tool.allocation[1].allocation_share, Some(d("0.8")));
    assert_eq!(tool.allocation[1].allocated_investment, Some(d("8000")));
    assert_eq!(tool.allocation[1].product_contribution, Some(d("40")));
    assert_eq!(product_line(&outcome, 10).allocated_investment, Some(d("2000")));
    assert_eq!(product_line(&outcome, 20).allocated_investment, Some(d("8000")));
    assert_eq!(product_line(&outcome, 10).amortized_contribution, Some(d("20")));
    assert_eq!(product_line(&outcome, 20).amortized_contribution, Some(d("40")));
    assert_eq!(outcome.project_amortized_recovered, Some(d("10000")), "20 × 500 owner units");

    let total: Decimal = tool.allocation.iter().filter_map(|line| line.allocated_investment).sum();
    assert_eq!(total, d("10000"), "the shares partition the one investment");
}

#[test]
fn products_that_do_not_use_the_owner_are_omitted_and_sum_to_zero() {
    let input = inputs(
        vec![entry(1, part_owner(5), 1, "10000", "CNY")],
        vec![product(10, Some(500), &[(5, 1)], &[]), product(20, Some(200), &[(9, 1)], &[])],
        Some(ToolingTreatment::Amortized),
    );
    let outcome = calculate_tooling(&input);
    assert_eq!(outcome.state, ResultState::Complete);
    assert_eq!(tool_line(&outcome, 1).allocation.len(), 1, "only the product that uses the owner has an allocation line");
    assert_eq!(tool_line(&outcome, 1).allocation[0].product_id, id(10));
    assert_eq!(product_line(&outcome, 20).allocated_investment, Some(Decimal::ZERO));
    assert_eq!(product_line(&outcome, 20).amortized_contribution, Some(Decimal::ZERO));
}

#[test]
fn a_product_root_tool_allocates_one_hundred_percent_to_its_own_product() {
    let input = inputs(
        vec![entry(1, product_owner(10), 1, "10000", "CNY")],
        vec![product(10, Some(800), &[], &[]), product(20, Some(500), &[], &[])],
        Some(ToolingTreatment::Amortized),
    );
    let outcome = calculate_tooling(&input);
    assert_eq!(outcome.state, ResultState::Complete);
    let tool = tool_line(&outcome, 1);
    assert_eq!(tool.allocation.len(), 1);
    assert_eq!(tool.allocation[0].product_id, id(10));
    assert_eq!(tool.allocation[0].expanded_quantity, 1, "a product-root tool is used once per finished product");
    assert_eq!(tool.allocation[0].allocation_share, Some(Decimal::ONE));
    assert_eq!(tool.allocation[0].allocated_investment, Some(d("10000")));
    assert_eq!(tool.recovery_quantity, Some(d("800")), "the product's own lifetime demand is the combined demand");
    assert_eq!(tool.contribution_per_owner_unit, Some(d("12.5")));
    assert_eq!(product_line(&outcome, 10).allocated_investment, Some(d("10000")));
    assert_eq!(product_line(&outcome, 20).allocated_investment, Some(Decimal::ZERO));
}

#[test]
fn part_version_and_product_root_tools_sum_into_one_product_line() {
    let input = inputs(
        vec![entry(1, product_owner(10), 1, "10000", "CNY"), entry(2, part_owner(5), 1, "10000", "CNY")],
        vec![product(10, Some(800), &[(5, 1)], &[])],
        Some(ToolingTreatment::Amortized),
    );
    let mut input = input;
    input.recovery.recovery = vec![manual(1, Some("5000")), manual(2, Some("5000"))];
    let outcome = calculate_tooling(&input);
    assert_eq!(outcome.state, ResultState::Complete);
    assert_eq!(outcome.project_investment, Some(d("20000")));
    assert_eq!(product_line(&outcome, 10).allocated_investment, Some(d("20000")));
    assert_eq!(product_line(&outcome, 10).amortized_contribution, Some(d("4")));
    assert_eq!(outcome.project_amortized_recovered, Some(d("3200")));
}

// ---------------------------------------------------------------------------
// Treatment: separate versus amortized
// ---------------------------------------------------------------------------

#[test]
fn the_separate_treatment_is_one_charge_and_never_populates_amortization() {
    let mut input = standard_input();
    input.tooling.default_treatment = Some(ToolingTreatment::Separate);
    let outcome = calculate_tooling(&input);
    assert_eq!(outcome.state, ResultState::Complete, "amortized-only fields are not an incompleteness under the separate treatment");
    assert_eq!(outcome.treatment, Some(ToolingTreatment::Separate));
    assert_eq!(outcome.project_investment, Some(d("10000")));
    assert_eq!(outcome.project_separate_charge, Some(d("10000")));
    assert_eq!(outcome.project_amortized_recovered, None);
    assert_eq!(outcome.project_under_over_recovery, None);
    assert!(outcome.annual_recovery.is_empty());
    let tool = tool_line(&outcome, 1);
    assert_eq!(tool.separate_charge, Some(d("10000")));
    assert_eq!(tool.contribution_per_owner_unit, None);
    assert_eq!(tool.projected_recovery, None);
    assert_eq!(tool.under_over_recovery, None);
    assert!(tool.annual_recovery.is_empty());
    assert_eq!(tool.allocation[0].product_contribution, None);
    assert_eq!(tool.allocation[0].allocation_share, Some(Decimal::ONE), "allocation still explains the separate charge");
    assert_eq!(tool.allocation[0].allocated_investment, Some(d("10000")));
    assert_eq!(product_line(&outcome, 10).allocated_investment, Some(d("10000")));
    assert_eq!(product_line(&outcome, 10).amortized_contribution, None);
    assert!(issue_codes(&outcome).is_empty());
}

#[test]
fn switching_the_treatment_preserves_the_investment_and_changes_only_the_channel() {
    let mut input = standard_input();
    let amortized = calculate_tooling(&input);
    input.tooling.default_treatment = Some(ToolingTreatment::Separate);
    let separate = calculate_tooling(&input);

    assert_eq!(amortized.project_investment, separate.project_investment);
    assert_eq!(amortized.project_investment, Some(d("10000")));
    assert_eq!(amortized.tools[0].investment, separate.tools[0].investment);
    assert_eq!(separate.project_separate_charge, Some(d("10000")));
    assert_eq!(separate.project_amortized_recovered, None);
    assert_eq!(amortized.project_amortized_recovered, Some(d("10000")));
    assert_eq!(amortized.project_separate_charge, None);
    // One evaluation can never report both a full separate charge and a full amortized contribution.
    assert_eq!(amortized.tools[0].separate_charge, None);
    assert_eq!(separate.tools[0].contribution_per_owner_unit, None);
    assert_eq!(amortized.tools[0].contribution_per_owner_unit, Some(d("2")));
}

#[test]
fn the_scenario_treatment_overrides_the_project_default() {
    let mut input = standard_input();
    input.tooling.default_treatment = Some(ToolingTreatment::Separate);
    input.recovery.treatment = Some(ToolingTreatment::Amortized);
    let outcome = calculate_tooling(&input);
    assert_eq!(outcome.treatment, Some(ToolingTreatment::Amortized));
    assert_eq!(outcome.project_separate_charge, None);
    assert_eq!(outcome.project_amortized_recovered, Some(d("10000")));

    let mut from_scenario = inputs(vec![entry(1, part_owner(5), 1, "10000", "CNY")], vec![product(10, Some(5000), &[(5, 1)], &[])], None);
    from_scenario.recovery.treatment = Some(ToolingTreatment::Separate);
    let outcome = calculate_tooling(&from_scenario);
    assert_eq!(outcome.state, ResultState::Complete);
    assert_eq!(outcome.treatment, Some(ToolingTreatment::Separate));
    assert_eq!(outcome.project_separate_charge, Some(d("10000")));
}

#[test]
fn a_missing_treatment_is_named_and_keeps_the_investment() {
    let input = inputs(vec![entry(1, part_owner(5), 1, "10000", "CNY")], vec![product(10, Some(5000), &[(5, 1)], &[])], None);
    let outcome = calculate_tooling(&input);
    assert_eq!(outcome.state, ResultState::Incomplete);
    assert_eq!(outcome.treatment, None);
    assert_eq!(issue_codes(&outcome), vec!["missing_treatment"]);
    assert_eq!(outcome.issues[0].field_path, "recovery.treatment");
    assert_eq!(outcome.project_investment, Some(d("10000")), "investment does not depend on the treatment");
    assert_eq!(outcome.project_amortized_recovered, None);
    assert_eq!(outcome.project_separate_charge, None);
    let tool = tool_line(&outcome, 1);
    assert_eq!(tool.investment, Some(d("10000")));
    assert_eq!(tool.contribution_per_owner_unit, None);
    assert_eq!(tool.separate_charge, None);
    assert_eq!(tool.recovery_quantity, Some(d("5000")));
}

// ---------------------------------------------------------------------------
// Amortized expansion and no cap
// ---------------------------------------------------------------------------

#[test]
fn an_assembly_used_twice_doubles_the_finished_product_contribution() {
    let input =
        inputs(vec![entry(1, part_owner(5), 1, "10000", "CNY")], vec![product(10, Some(1000), &[(5, 2)], &[])], Some(ToolingTreatment::Amortized));
    let outcome = calculate_tooling(&input);
    assert_eq!(outcome.state, ResultState::Complete);
    let tool = tool_line(&outcome, 1);
    assert_eq!(tool.recovery_quantity, Some(d("2000")), "1,000 finished products × 2 assemblies");
    assert_eq!(tool.contribution_per_owner_unit, Some(d("5")));
    assert_eq!(tool.allocation[0].product_contribution, Some(d("10")), "twice the owner-unit contribution per finished product");
    assert_eq!(product_line(&outcome, 10).amortized_contribution, Some(d("10")));
    assert_eq!(tool.projected_recovery, Some(d("10000")));
    assert_eq!(tool.under_over_recovery, Some(Decimal::ZERO));
}

#[test]
fn reaching_the_recovery_quantity_never_caps_or_reduces_the_contribution() {
    // 10,000 over a manual quantity of 2,000 contributes 5 forever; the lifetime demand of 3,000
    // recovers 15,000, an over-recovery of 5,000.
    let mut input = inputs(
        vec![entry(1, part_owner(5), 1, "10000", "CNY")],
        vec![product(10, Some(3000), &[(5, 1)], &[(2027, 1000), (2028, 2000)])],
        Some(ToolingTreatment::Amortized),
    );
    input.recovery.recovery = vec![manual(1, Some("2000"))];
    let outcome = calculate_tooling(&input);
    assert_eq!(outcome.state, ResultState::Complete);
    let tool = tool_line(&outcome, 1);
    assert_eq!(tool.contribution_per_owner_unit, Some(d("5")), "the contribution stays at its initial value");
    assert_eq!(tool.projected_recovery, Some(d("15000")));
    assert_eq!(tool.under_over_recovery, Some(d("5000")), "positive is over-recovery");
    assert_eq!(
        tool.annual_recovery,
        vec![
            solidbom_contract::ToolingAnnualRecovery { year: 2027, recovered: Some(d("5000")) },
            solidbom_contract::ToolingAnnualRecovery { year: 2028, recovered: Some(d("10000")) },
        ],
        "each year is the same contribution × that year's owner demand; no year is capped or reduced"
    );
    assert_eq!(outcome.project_amortized_recovered, Some(d("15000")));
    assert_eq!(outcome.project_under_over_recovery, Some(d("5000")));
    assert_eq!(
        outcome.annual_recovery,
        vec![
            solidbom_contract::ToolingAnnualRecovery { year: 2027, recovered: Some(d("5000")) },
            solidbom_contract::ToolingAnnualRecovery { year: 2028, recovered: Some(d("10000")) },
        ]
    );
}

#[test]
fn under_recovery_is_negative_projected_minus_investment() {
    let mut input = inputs(
        vec![entry(1, part_owner(5), 1, "10000", "CNY")],
        vec![product(10, Some(3000), &[(5, 1)], &[(2027, 1000), (2028, 2000)])],
        Some(ToolingTreatment::Amortized),
    );
    input.recovery.recovery = vec![manual(1, Some("5000"))];
    let outcome = calculate_tooling(&input);
    assert_eq!(outcome.state, ResultState::Complete);
    let tool = tool_line(&outcome, 1);
    assert_eq!(tool.contribution_per_owner_unit, Some(d("2")));
    assert_eq!(tool.projected_recovery, Some(d("6000")));
    assert_eq!(tool.under_over_recovery, Some(d("-4000")), "negative is under-recovery");
    assert_eq!(outcome.project_under_over_recovery, Some(d("-4000")));
    assert_eq!(outcome.project_amortized_recovered, Some(d("6000")));
}

#[test]
fn two_tools_sum_the_project_annual_recovery_present_only_when_all_are_available() {
    let mut input = inputs(
        vec![entry(1, part_owner(5), 1, "10000", "CNY"), entry(2, part_owner(5), 1, "10000", "CNY")],
        vec![product(10, Some(5000), &[(5, 1)], &[(2027, 2000), (2028, 3000)])],
        Some(ToolingTreatment::Amortized),
    );
    input.recovery.recovery = vec![manual(1, Some("5000")), manual(2, Some("2000"))];
    let outcome = calculate_tooling(&input);
    assert_eq!(outcome.state, ResultState::Complete);
    assert_eq!(tool_line(&outcome, 1).contribution_per_owner_unit, Some(d("2")));
    assert_eq!(tool_line(&outcome, 2).contribution_per_owner_unit, Some(d("5")));
    assert_eq!(outcome.project_investment, Some(d("20000")));
    assert_eq!(outcome.project_amortized_recovered, Some(d("35000")));
    assert_eq!(outcome.project_under_over_recovery, Some(d("15000")));
    assert_eq!(
        outcome.annual_recovery,
        vec![
            solidbom_contract::ToolingAnnualRecovery { year: 2027, recovered: Some(d("14000")) },
            solidbom_contract::ToolingAnnualRecovery { year: 2028, recovered: Some(d("21000")) },
        ]
    );
    assert_eq!(product_line(&outcome, 10).allocated_investment, Some(d("20000")));
    assert_eq!(product_line(&outcome, 10).amortized_contribution, Some(d("7")));

    // When one tool's annual series is unavailable the project series is empty, not a partial sum.
    input.demand.products[0].lifetime_demand = None;
    let outcome = calculate_tooling(&input);
    assert_eq!(outcome.state, ResultState::Incomplete);
    assert!(outcome.annual_recovery.is_empty());
}

// ---------------------------------------------------------------------------
// Demand cutoffs, missing and invalid inputs
// ---------------------------------------------------------------------------

#[test]
fn a_purchased_branch_descendant_has_zero_combined_demand_and_no_invented_share() {
    // Part 5 sits only inside the purchased part 2, whose descendants the baseline already stops
    // counting; part version 5 therefore has no expanded quantity in any quoted product.
    let input = inputs(
        vec![entry(1, part_owner(5), 1, "10000", "CNY"), entry(2, part_owner(2), 1, "10000", "CNY")],
        vec![product(10, Some(600), &[(2, 1)], &[]), product(20, Some(400), &[(2, 1)], &[])],
        Some(ToolingTreatment::Amortized),
    );
    let outcome = calculate_tooling(&input);
    assert_eq!(outcome.state, ResultState::Incomplete);
    let descendant = tool_line(&outcome, 1);
    assert_eq!(descendant.investment, Some(d("10000")), "the investment is still reported");
    assert!(descendant.allocation.is_empty(), "the owner is demanded by no quoted product");
    assert_eq!(descendant.recovery_quantity, None);
    assert_eq!(descendant.contribution_per_owner_unit, None);
    assert_eq!(descendant.projected_recovery, None);
    assert_eq!(descendant.under_over_recovery, None);
    assert!(descendant.annual_recovery.is_empty());
    assert!(outcome.issues.iter().any(|issue| issue.code == "zero_combined_demand" && issue.tool_id == Some(id(1))));
    assert_eq!(
        outcome.issues.iter().find(|issue| issue.code == "zero_combined_demand").map(|issue| issue.field_path.as_str()),
        Some("tools[0].allocation")
    );
    // The purchased part's own non-suppressed demand still evaluates normally.
    let purchased = tool_line(&outcome, 2);
    assert_eq!(purchased.allocation[0].product_owner_demand, Some(600));
    assert_eq!(purchased.allocation[0].allocation_share, Some(d("0.6")));
    assert_eq!(purchased.allocation[0].allocated_investment, Some(d("6000")));
    assert_eq!(outcome.project_investment, Some(d("20000")), "the total is still known, while the shares are not");
    assert_eq!(product_line(&outcome, 10).allocated_investment, Some(d("6000")), "the zero-demand tool adds nothing");
    assert_eq!(product_line(&outcome, 20).allocated_investment, Some(d("4000")));
}

#[test]
fn a_missing_product_lifetime_leaves_allocation_and_projection_unavailable() {
    let input = inputs(
        vec![entry(1, part_owner(5), 1, "10000", "CNY")],
        vec![product(10, None, &[(5, 1)], &[(2027, 100)])],
        Some(ToolingTreatment::Amortized),
    );
    let outcome = calculate_tooling(&input);
    assert_eq!(outcome.state, ResultState::Incomplete);
    assert!(outcome.issues.iter().any(|issue| issue.code == "missing_product_demand" && issue.product_id == Some(id(10))));
    assert_eq!(
        outcome.issues.iter().find(|issue| issue.code == "missing_product_demand").map(|issue| issue.field_path.as_str()),
        Some("demand.products[0].lifetime_demand")
    );
    let tool = tool_line(&outcome, 1);
    assert_eq!(tool.investment, Some(d("10000")));
    assert_eq!(tool.allocation.len(), 1);
    assert_eq!(tool.allocation[0].product_owner_demand, None);
    assert_eq!(tool.allocation[0].allocation_share, None);
    assert_eq!(tool.allocation[0].allocated_investment, None);
    assert_eq!(tool.recovery_quantity, None);
    assert_eq!(tool.contribution_per_owner_unit, None);
    assert_eq!(tool.projected_recovery, None);
    assert!(tool.annual_recovery.is_empty());
    assert_eq!(outcome.project_investment, Some(d("10000")));
    assert_eq!(product_line(&outcome, 10).allocated_investment, None);
}

#[test]
fn a_missing_lifetime_of_a_product_no_tool_demands_does_not_make_the_answer_incomplete() {
    let input = inputs(
        vec![entry(1, part_owner(5), 1, "10000", "CNY")],
        vec![product(10, Some(500), &[(5, 1)], &[]), product(20, None, &[(7, 1)], &[])],
        Some(ToolingTreatment::Amortized),
    );
    let outcome = calculate_tooling(&input);
    assert_eq!(outcome.state, ResultState::Complete, "issues: {:?}", issue_codes(&outcome));
    assert!(outcome.issues.is_empty());
    assert_eq!(product_line(&outcome, 20).lifetime_demand, None);
    assert_eq!(product_line(&outcome, 20).allocated_investment, Some(Decimal::ZERO));
    assert_eq!(product_line(&outcome, 20).amortized_contribution, Some(Decimal::ZERO));
}

#[test]
fn the_default_recovery_basis_is_the_combined_lifetime_demand() {
    let input = standard_input();
    let outcome = calculate_tooling(&input);
    assert_eq!(outcome.state, ResultState::Complete);
    let tool = tool_line(&outcome, 1);
    assert_eq!(tool.recovery_quantity, Some(d("5000")));
    assert_eq!(tool.contribution_per_owner_unit, Some(d("2")));
    assert_eq!(tool.projected_recovery, Some(d("10000")), "the lifetime basis projects exactly the investment");
    assert_eq!(tool.under_over_recovery, Some(Decimal::ZERO));
}

#[test]
fn a_recovery_choice_for_an_unknown_tool_is_ignored() {
    let mut input = standard_input();
    input.recovery.recovery = vec![manual(999, Some("1"))];
    let outcome = calculate_tooling(&input);
    assert_eq!(outcome.state, ResultState::Complete);
    assert!(outcome.issues.is_empty());
    assert_eq!(tool_line(&outcome, 1).recovery_quantity, Some(d("5000")), "the existing tool keeps the lifetime basis");
}

#[test]
fn a_missing_manual_recovery_quantity_is_named_and_never_zero() {
    let mut input = standard_input();
    input.recovery.recovery = vec![manual(1, None)];
    let outcome = calculate_tooling(&input);
    assert_eq!(outcome.state, ResultState::Incomplete);
    assert_eq!(issue_codes(&outcome), vec!["missing_recovery_quantity"]);
    assert_eq!(outcome.issues[0].field_path, "recovery.recovery[0].basis.quantity");
    assert_eq!(outcome.issues[0].tool_id, Some(id(1)));
    let tool = tool_line(&outcome, 1);
    assert_eq!(tool.recovery_quantity, None);
    assert_eq!(tool.contribution_per_owner_unit, None);
    assert_eq!(tool.projected_recovery, None);
    assert_eq!(tool.investment, Some(d("10000")));
}

#[test]
fn a_non_positive_manual_recovery_quantity_is_named() {
    for quantity in ["0", "-5"] {
        let mut input = standard_input();
        input.recovery.recovery = vec![manual(1, Some(quantity))];
        let outcome = calculate_tooling(&input);
        assert_eq!(outcome.state, ResultState::Incomplete);
        assert_eq!(issue_codes(&outcome), vec!["invalid_recovery_quantity"], "quantity {quantity}");
        assert_eq!(outcome.issues[0].field_path, "recovery.recovery[0].basis.quantity");
        assert_eq!(tool_line(&outcome, 1).contribution_per_owner_unit, None);
    }
}

#[test]
fn a_hostile_tool_count_or_cost_is_a_named_issue_not_a_panic() {
    for (entry, code) in
        [(entry(1, part_owner(5), 0, "10000", "CNY"), "invalid_tool_count"), (entry(1, part_owner(5), 1, "-1", "CNY"), "invalid_cost_per_tool")]
    {
        let input = inputs(vec![entry], vec![product(10, Some(5000), &[(5, 1)], &[])], Some(ToolingTreatment::Amortized));
        let outcome = calculate_tooling(&input);
        assert_eq!(outcome.state, ResultState::Incomplete);
        assert!(issue_codes(&outcome).contains(&code), "issues: {:?}", issue_codes(&outcome));
        assert_eq!(tool_line(&outcome, 1).source_investment, None);
        assert_eq!(tool_line(&outcome, 1).investment, None);
        assert_eq!(outcome.project_investment, None);
        assert_eq!(product_line(&outcome, 10).allocated_investment, None);
    }
}

// ---------------------------------------------------------------------------
// Saved exchange rates
// ---------------------------------------------------------------------------

#[test]
fn a_saved_rate_converts_the_source_investment_and_the_identity_needs_no_rate() {
    let mut input =
        inputs(vec![entry(1, part_owner(5), 2, "100", "USD")], vec![product(10, Some(5000), &[(5, 1)], &[])], Some(ToolingTreatment::Amortized));
    input.exchange_rates = vec![ExchangeRate { source_currency: "USD".into(), project_units_per_source_unit: d("7.20") }];
    let outcome = calculate_tooling(&input);
    assert_eq!(outcome.state, ResultState::Complete);
    let tool = tool_line(&outcome, 1);
    assert_eq!(tool.source_investment, Some(d("200")));
    assert_eq!(tool.fx_rate, Some(d("7.2")));
    assert_eq!(tool.investment, Some(d("1440")));
    assert_eq!(outcome.project_investment, Some(d("1440")));
    assert_eq!(tool.contribution_per_owner_unit, Some(d("0.288")));

    // Same currency is the identity and needs no saved rate.
    let identity =
        inputs(vec![entry(1, part_owner(5), 1, "200", "CNY")], vec![product(10, Some(5000), &[(5, 1)], &[])], Some(ToolingTreatment::Amortized));
    let outcome = calculate_tooling(&identity);
    assert_eq!(outcome.state, ResultState::Complete);
    assert_eq!(tool_line(&outcome, 1).fx_rate, None);
    assert_eq!(tool_line(&outcome, 1).investment, Some(d("200")));
}

#[test]
fn a_missing_exchange_rate_never_becomes_one_and_withholds_the_project_total() {
    let input =
        inputs(vec![entry(1, part_owner(5), 1, "100", "USD")], vec![product(10, Some(5000), &[(5, 1)], &[])], Some(ToolingTreatment::Amortized));
    let outcome = calculate_tooling(&input);
    assert_eq!(outcome.state, ResultState::Incomplete);
    assert_eq!(issue_codes(&outcome), vec!["missing_exchange_rate"]);
    assert_eq!(outcome.issues[0].field_path, "exchange_rates");
    let tool = tool_line(&outcome, 1);
    assert_eq!(tool.source_investment, Some(d("100")));
    assert_eq!(tool.fx_rate, None);
    assert_eq!(tool.investment, None, "a missing rate is never the identity rate one");
    assert_eq!(tool.allocation[0].allocation_share, Some(Decimal::ONE), "the share is still explainable");
    assert_eq!(tool.allocation[0].allocated_investment, None);
    assert_eq!(outcome.project_investment, None, "the project total is never a partial sum");
    assert_eq!(product_line(&outcome, 10).allocated_investment, None);

    // A non-positive saved rate is invalid, never coerced.
    let mut invalid =
        inputs(vec![entry(1, part_owner(5), 1, "100", "USD")], vec![product(10, Some(5000), &[(5, 1)], &[])], Some(ToolingTreatment::Amortized));
    invalid.exchange_rates = vec![ExchangeRate { source_currency: "USD".into(), project_units_per_source_unit: Decimal::ZERO }];
    let outcome = calculate_tooling(&invalid);
    assert_eq!(issue_codes(&outcome), vec!["invalid_exchange_rate"]);
    assert_eq!(tool_line(&outcome, 1).investment, None);
    assert_eq!(outcome.state, ResultState::Incomplete);
}

// ---------------------------------------------------------------------------
// Ordering, determinism and normalisation
// ---------------------------------------------------------------------------

#[test]
fn tools_and_products_are_ordered_by_identity() {
    let input = inputs(
        vec![entry(2, part_owner(5), 1, "10000", "CNY"), entry(1, part_owner(5), 1, "10000", "CNY")],
        vec![product(20, Some(200), &[(5, 1)], &[]), product(10, Some(100), &[(5, 1)], &[])],
        Some(ToolingTreatment::Amortized),
    );
    let outcome = calculate_tooling(&input);
    assert_eq!(outcome.tools.iter().map(|line| line.tool_id).collect::<Vec<_>>(), vec![id(1), id(2)]);
    assert_eq!(outcome.products.iter().map(|line| line.product_id).collect::<Vec<_>>(), vec![id(10), id(20)]);
    assert_eq!(tool_line(&outcome, 1).allocation.iter().map(|line| line.product_id).collect::<Vec<_>>(), vec![id(10), id(20)]);
}

#[test]
fn repeated_runs_and_permuted_inputs_are_byte_identical() {
    let mut input = inputs(
        vec![entry(1, part_owner(5), 1, "10000", "CNY"), entry(2, part_owner(7), 2, "1000", "CNY")],
        vec![
            product(10, Some(5000), &[(5, 1), (7, 2)], &[(2027, 2000), (2028, 3000)]),
            product(20, Some(1200), &[(5, 2)], &[(2027, 700), (2028, 500)]),
        ],
        Some(ToolingTreatment::Amortized),
    );
    input.recovery.recovery = vec![manual(2, Some("4000"))];
    let first = calculate_tooling(&input);
    let second = calculate_tooling(&input);
    assert_eq!(first, second);
    assert_eq!(json(&first), json(&second));

    let mut permuted = input.clone();
    permuted.tooling.entries.reverse();
    permuted.demand.products.reverse();
    for product in &mut permuted.demand.products {
        product.forecasts.reverse();
        product.part_quantities.reverse();
    }
    let reordered = calculate_tooling(&permuted);
    assert_eq!(json(&first), json(&reordered), "tool, product, forecast and part-quantity order must not change the answer");
}

#[test]
fn trailing_zeros_are_normalised_at_the_output_boundary() {
    let mut input =
        inputs(vec![entry(1, part_owner(5), 2, "100.00", "USD")], vec![product(10, Some(5000), &[(5, 1)], &[])], Some(ToolingTreatment::Amortized));
    input.exchange_rates = vec![ExchangeRate { source_currency: "USD".into(), project_units_per_source_unit: d("7.200") }];
    let outcome = calculate_tooling(&input);
    assert_eq!(outcome.state, ResultState::Complete);
    let text = json(&outcome);
    assert!(text.contains("\"cost_per_tool\":\"100\""), "cost is normalised: {text}");
    assert!(text.contains("\"source_investment\":\"200\""), "source investment is normalised: {text}");
    assert!(text.contains("\"fx_rate\":\"7.2\""), "the saved rate is normalised: {text}");
    assert!(text.contains("\"investment\":\"1440\""), "the converted investment is normalised: {text}");
}

#[test]
fn issues_are_ordered_by_code_and_field_path() {
    let mut input = inputs(
        vec![entry(1, part_owner(5), 1, "100", "USD"), entry(2, part_owner(5), 0, "100", "CNY"), entry(3, part_owner(99), 1, "100", "CNY")],
        vec![product(10, Some(600), &[(5, 1)], &[])],
        None,
    );
    input.recovery.treatment = None;
    let outcome = calculate_tooling(&input);
    let codes = issue_codes(&outcome);
    assert_eq!(codes, vec!["invalid_tool_count", "missing_exchange_rate", "missing_treatment", "zero_combined_demand"]);
    let keys: Vec<(&str, &str)> = outcome.issues.iter().map(|issue| (issue.code.as_str(), issue.field_path.as_str())).collect();
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    assert_eq!(keys, sorted);
}

#[test]
fn every_reported_amount_has_an_explanation_line() {
    let mut input = standard_input();
    input.recovery.recovery = vec![manual(1, Some("5000"))];
    let outcome = calculate_tooling(&input);
    let tool = explanation_codes(&tool_line(&outcome, 1).explanations);
    for expected in [
        "tool.cost_per_tool",
        "tool.source_investment",
        "tool.investment",
        "tool.combined_demand",
        "tool.recovery_quantity",
        "tool.contribution",
        "tool.allocation_share",
        "tool.allocation",
        "tool.product_contribution",
        "tool.annual_recovery.2027",
        "tool.annual_recovery.2028",
        "tool.projected_recovery",
        "tool.under_over_recovery",
    ] {
        assert!(tool.contains(expected), "missing tool explanation {expected}: {tool:?}");
    }
    assert_eq!(explanation_amounts(&tool_line(&outcome, 1).explanations).get("tool.contribution"), Some(&d("2")));
    let product = explanation_codes(&product_line(&outcome, 10).explanations);
    for expected in ["product.lifetime_demand", "product.allocated_investment", "product.amortized_contribution"] {
        assert!(product.contains(expected), "missing product explanation {expected}: {product:?}");
    }
    let project = explanation_codes(&outcome.explanations);
    for expected in ["project.investment", "project.projected_recovery", "project.under_over_recovery", "project.annual_recovery.2027"] {
        assert!(project.contains(expected), "missing project explanation {expected}: {project:?}");
    }

    // The separate treatment explains its own channel and no amortized one.
    input.tooling.default_treatment = Some(ToolingTreatment::Separate);
    let outcome = calculate_tooling(&input);
    let tool = explanation_codes(&tool_line(&outcome, 1).explanations);
    assert!(tool.contains("tool.separate_charge"), "{tool:?}");
    assert!(!tool.contains("tool.contribution"), "{tool:?}");
    assert!(!tool.contains("tool.projected_recovery"), "{tool:?}");
    assert!(!tool.contains("tool.annual_recovery.2027"), "{tool:?}");
    let project = explanation_codes(&outcome.explanations);
    assert!(project.contains("project.separate_charge"), "{project:?}");
    assert!(!project.contains("project.projected_recovery"), "{project:?}");
}

// ---------------------------------------------------------------------------
// Arithmetic overflow
// ---------------------------------------------------------------------------

#[test]
fn an_overflowing_investment_is_a_named_issue_not_a_panic() {
    let input = inputs(
        vec![entry(1, part_owner(5), 2, &Decimal::MAX.to_string(), "CNY")],
        vec![product(10, Some(5000), &[(5, 1)], &[])],
        Some(ToolingTreatment::Amortized),
    );
    let outcome = calculate_tooling(&input);
    assert_eq!(outcome.state, ResultState::Incomplete);
    assert!(issue_codes(&outcome).contains(&"arithmetic_overflow"), "issues: {:?}", issue_codes(&outcome));
    let tool = tool_line(&outcome, 1);
    assert_eq!(tool.source_investment, None);
    assert_eq!(tool.investment, None);
    assert_eq!(outcome.project_investment, None);
    assert_eq!(outcome.project_amortized_recovered, None);
}

#[test]
fn an_overflowing_owner_lifetime_demand_is_a_named_issue_not_a_panic() {
    let input = inputs(
        vec![entry(1, part_owner(5), 1, "10000", "CNY")],
        vec![product(10, Some(u64::MAX), &[(5, 2)], &[])],
        Some(ToolingTreatment::Amortized),
    );
    let outcome = calculate_tooling(&input);
    assert_eq!(outcome.state, ResultState::Incomplete);
    assert!(issue_codes(&outcome).contains(&"arithmetic_overflow"), "issues: {:?}", issue_codes(&outcome));
    assert_eq!(tool_line(&outcome, 1).allocation[0].product_owner_demand, None);
    assert_eq!(tool_line(&outcome, 1).recovery_quantity, None);
    assert_eq!(outcome.project_investment, Some(d("10000")), "the investment itself is still known");
}
