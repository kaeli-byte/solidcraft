//! Arithmetic tests for the pure commercial scenario calculator (package SB-18).
//!
//! The accepted vectors come from the package acceptance and the app specification's scenarios
//! 14–17 and 19: 100 at 3% gives 100/97/94.09 while manufacturing cost stays fixed; an entered 100
//! with tooling contribution 2 totals 102 in year one, then 99 when productivity is exempt from the
//! contribution and 98.94 when it applies to both; a blank manual price has no manual-price EBIT;
//! the first-year suggested price is `C / (1 − t)`.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use rust_decimal::Decimal;
use solidbom_contract::{
    AnnualForecast, CommercialInputs, CommercialOutcome, CommercialProductInput, CostLine, ResultState, ScenarioManualPrice, ToolingOutcome,
    ToolingProductLine, ToolingTreatment, YearRange,
};
use uuid::Uuid;

use crate::commercial::calculate_commercial;

fn id(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn d(text: &str) -> Decimal {
    text.parse().expect("a decimal literal in the test fixture")
}

fn dec(text: &str) -> Option<Decimal> {
    Some(d(text))
}

fn product(product: u128, sop: u16, eop: u16, cost: Option<Decimal>) -> CommercialProductInput {
    CommercialProductInput {
        product_id: id(product),
        product_revision_id: None,
        schedule: YearRange { sop_year: sop, eop_year: eop },
        forecasts: (sop..=eop).map(|year| AnnualForecast { year, quantity: 1_000 }).collect(),
        recurring_unit_cost: cost,
    }
}

fn empty_tooling() -> ToolingOutcome {
    ToolingOutcome {
        state: ResultState::Complete,
        treatment: None,
        tools: Vec::new(),
        products: Vec::new(),
        project_investment: Some(Decimal::ZERO),
        project_amortized_recovered: None,
        project_under_over_recovery: None,
        project_separate_charge: None,
        annual_recovery: Vec::new(),
        explanations: Vec::new(),
        issues: Vec::new(),
    }
}

/// One amortized tool answer contributing `contribution` per finished product.
fn amortized_tooling(contribution: &str) -> ToolingOutcome {
    ToolingOutcome {
        state: ResultState::Complete,
        treatment: Some(ToolingTreatment::Amortized),
        tools: Vec::new(),
        products: vec![ToolingProductLine {
            product_id: id(1),
            product_revision_id: None,
            lifetime_demand: Some(3_000),
            allocated_investment: Some(d("10000")),
            amortized_contribution: Some(d(contribution)),
            explanations: Vec::new(),
        }],
        project_investment: Some(d("10000")),
        project_amortized_recovered: Some(d("10000")),
        project_under_over_recovery: Some(Decimal::ZERO),
        project_separate_charge: None,
        annual_recovery: Vec::new(),
        explanations: Vec::new(),
        issues: Vec::new(),
    }
}

fn separate_tooling() -> ToolingOutcome {
    ToolingOutcome {
        state: ResultState::Complete,
        treatment: Some(ToolingTreatment::Separate),
        tools: Vec::new(),
        products: vec![ToolingProductLine {
            product_id: id(1),
            product_revision_id: None,
            lifetime_demand: Some(3_000),
            allocated_investment: Some(d("10000")),
            amortized_contribution: None,
            explanations: Vec::new(),
        }],
        project_investment: Some(d("10000")),
        project_amortized_recovered: None,
        project_under_over_recovery: None,
        project_separate_charge: Some(d("10000")),
        annual_recovery: Vec::new(),
        explanations: Vec::new(),
        issues: Vec::new(),
    }
}

fn inputs(products: Vec<CommercialProductInput>, tooling: ToolingOutcome) -> CommercialInputs {
    CommercialInputs {
        project_currency: "CNY".into(),
        ebit_target_rate: Some(d("0.2")),
        annual_productivity_rate: Some(d("0.03")),
        productivity_applies_to_tooling: false,
        manual_prices: Vec::new(),
        tooling,
        products,
        setup_total: Some(Decimal::ZERO),
    }
}

fn codes(outcome: &CommercialOutcome) -> Vec<&str> {
    outcome.issues.iter().map(|issue| issue.code.as_str()).collect()
}

fn year_values(outcome: &CommercialOutcome, product: u128) -> Vec<(u16, Option<Decimal>, Option<Decimal>)> {
    outcome
        .products
        .iter()
        .find(|line| line.product_id == id(product))
        .expect("the fixture's product line")
        .years
        .iter()
        .map(|year| (year.year, year.suggested_base_price, year.recurring_unit_cost))
        .collect()
}

#[test]
fn base_price_100_with_three_percent_productivity_gives_100_97_94_09_and_cost_stays_fixed() {
    let inputs = inputs(vec![product(1, 2027, 2029, Some(d("80")))], empty_tooling());
    let outcome = calculate_commercial(&inputs);
    assert_eq!(outcome.state, ResultState::Complete, "issues: {:?}", outcome.issues);
    assert_eq!(
        year_values(&outcome, 1),
        vec![(2027, dec("100"), dec("80")), (2028, dec("97"), dec("80")), (2029, dec("94.09"), dec("80")),],
        "price falls by 3% a year while the manufacturing cost stays fixed"
    );
    let line = &outcome.products[0];
    assert_eq!(line.suggested_first_year_price, dec("100"));
    assert_eq!(outcome.project.lifetime_revenue, None, "a blank manual price has no quoted revenue");
    assert!(outcome.issues.is_empty());
}

#[test]
fn a_tooling_contribution_of_two_gives_102_then_99_or_98_94() {
    let exempt = inputs(vec![product(1, 2027, 2028, Some(d("80")))], amortized_tooling("2"));
    let exempt = calculate_commercial(&exempt);
    let line = &exempt.products[0];
    assert_eq!(line.tooling_initial_contribution, dec("2"));
    assert_eq!(line.years[0].tooling_contribution, dec("2"));
    assert_eq!(line.years[0].manual_total_price, None, "no manual price, so no manual total");
    assert_eq!(line.years[0].suggested_total_price, dec("102"));
    assert_eq!(line.years[1].tooling_contribution, dec("2"));
    assert_eq!(line.years[1].suggested_total_price, dec("99"));

    let mut applies_inputs = inputs(vec![product(1, 2027, 2028, Some(d("80")))], amortized_tooling("2"));
    applies_inputs.productivity_applies_to_tooling = true;
    let applies = calculate_commercial(&applies_inputs);
    let line = &applies.products[0];
    assert_eq!(line.years[0].suggested_total_price, dec("102"));
    assert_eq!(line.years[1].tooling_contribution, dec("1.94"));
    assert_eq!(line.years[1].suggested_total_price, dec("98.94"));
}

#[test]
fn a_blank_manual_price_has_no_manual_ebit_and_a_positive_price_shows_amount_and_rate() {
    let blank = calculate_commercial(&inputs(vec![product(1, 2027, 2027, Some(d("80")))], empty_tooling()));
    let line = &blank.products[0];
    assert_eq!(line.manual_price, None);
    assert_eq!(line.manual_first_year_ebit_amount, None);
    assert_eq!(line.manual_first_year_ebit_rate, None);
    assert_eq!(line.years[0].revenue, None);
    assert_eq!(line.years[0].ebit_amount, None);
    assert_eq!(line.years[0].suggested_base_price, dec("100"), "the suggested price stays visible");
    assert_eq!(blank.state, ResultState::Complete, "a blank manual price is not an error");

    let mut priced = inputs(vec![product(1, 2027, 2027, Some(d("80")))], empty_tooling());
    priced.manual_prices = vec![ScenarioManualPrice { product_id: id(1), price: d("100") }];
    let priced = calculate_commercial(&priced);
    let line = &priced.products[0];
    assert_eq!(line.manual_price, dec("100"));
    assert_eq!(line.manual_first_year_ebit_amount, dec("20"));
    assert_eq!(line.manual_first_year_ebit_rate, dec("0.2"));
    assert_eq!(line.years[0].manual_unit_ebit_amount, dec("20"));
    assert_eq!(line.years[0].manual_unit_ebit_rate, dec("0.2"));
    assert_eq!(line.years[0].revenue, dec("100000"));
    assert_eq!(line.years[0].ebit_amount, dec("20000"));
    assert_eq!(priced.project.lifetime_revenue, dec("100000"));
    assert_eq!(priced.project.lifetime_ebit_amount, dec("20000"));
    assert_eq!(priced.state, ResultState::Complete);
}

#[test]
fn the_suggested_first_year_price_is_cost_over_one_minus_the_target() {
    let mut inputs = inputs(vec![product(1, 2027, 2027, Some(d("80")))], empty_tooling());
    inputs.ebit_target_rate = Some(d("0.75"));
    let outcome = calculate_commercial(&inputs);
    assert_eq!(outcome.products[0].suggested_first_year_price, dec("320"));
    assert_eq!(outcome.products[0].years[0].suggested_unit_ebit_amount, dec("240"));
    assert_eq!(outcome.products[0].years[0].suggested_unit_ebit_rate, dec("0.75"));
}

#[test]
fn productivity_starts_at_each_products_own_first_production_year() {
    let mut first = product(1, 2027, 2028, Some(d("80")));
    first.forecasts = vec![AnnualForecast { year: 2027, quantity: 100 }, AnnualForecast { year: 2028, quantity: 200 }];
    let mut second = product(2, 2028, 2029, Some(d("80")));
    second.forecasts = vec![AnnualForecast { year: 2028, quantity: 300 }, AnnualForecast { year: 2029, quantity: 400 }];
    let outcome = calculate_commercial(&inputs(vec![first, second], empty_tooling()));
    assert_eq!(year_values(&outcome, 1), vec![(2027, dec("100"), dec("80")), (2028, dec("97"), dec("80"))]);
    assert_eq!(year_values(&outcome, 2), vec![(2028, dec("100"), dec("80")), (2029, dec("97"), dec("80"))]);
    assert_eq!(outcome.project.years.iter().map(|year| year.year).collect::<Vec<_>>(), vec![2027, 2028, 2029]);
}

#[test]
fn separate_tooling_keeps_the_investment_outside_the_unit_price() {
    let outcome = calculate_commercial(&inputs(vec![product(1, 2027, 2027, Some(d("80")))], separate_tooling()));
    let line = &outcome.products[0];
    assert_eq!(line.tooling_initial_contribution, None);
    assert_eq!(line.years[0].tooling_contribution, None);
    assert_eq!(line.years[0].suggested_total_price, dec("100"), "the separate charge is not in the unit price");
    assert_eq!(line.allocated_investment, dec("10000"));
    assert_eq!(outcome.project.tooling_separate_charge, dec("10000"));
    assert_eq!(outcome.project.tooling_projected_recovery, None);
    assert_eq!(outcome.project.tooling_under_over_recovery, None);
    assert_eq!(outcome.state, ResultState::Complete);
}

#[test]
fn missing_inputs_are_named_and_never_become_zero() {
    let mut missing_target = inputs(vec![product(1, 2027, 2027, Some(d("80")))], empty_tooling());
    missing_target.ebit_target_rate = None;
    let outcome = calculate_commercial(&missing_target);
    assert_eq!(outcome.state, ResultState::Incomplete);
    assert!(codes(&outcome).contains(&"missing_ebit_target"), "{:?}", outcome.issues);
    assert_eq!(outcome.products[0].suggested_first_year_price, None);

    let mut missing_productivity = inputs(vec![product(1, 2027, 2027, Some(d("80")))], empty_tooling());
    missing_productivity.annual_productivity_rate = None;
    let outcome = calculate_commercial(&missing_productivity);
    assert!(codes(&outcome).contains(&"missing_productivity"));
    assert_eq!(outcome.products[0].years[0].suggested_base_price, None);

    let outcome = calculate_commercial(&inputs(vec![product(1, 2027, 2027, None)], empty_tooling()));
    assert!(codes(&outcome).contains(&"missing_recurring_unit_cost"));
    assert_eq!(outcome.products[0].suggested_first_year_price, None);

    let mut missing_forecast = inputs(vec![product(1, 2027, 2028, Some(d("80")))], empty_tooling());
    missing_forecast.manual_prices = vec![ScenarioManualPrice { product_id: id(1), price: d("100") }];
    missing_forecast.products[0].forecasts = vec![AnnualForecast { year: 2027, quantity: 1_000 }];
    let outcome = calculate_commercial(&missing_forecast);
    assert!(codes(&outcome).contains(&"missing_forecast"), "{:?}", outcome.issues);
    assert_eq!(outcome.products[0].years[1].revenue, None);
    assert_eq!(outcome.products[0].years[1].ebit_amount, None);
    assert_eq!(outcome.project.lifetime_revenue, None, "no partial lifetime total");

    let mut missing_setup = inputs(vec![product(1, 2027, 2027, Some(d("80")))], empty_tooling());
    missing_setup.setup_total = None;
    let outcome = calculate_commercial(&missing_setup);
    assert!(codes(&outcome).contains(&"missing_setup_charge"));

    let mut incomplete_tooling = inputs(vec![product(1, 2027, 2027, Some(d("80")))], empty_tooling());
    incomplete_tooling.tooling.state = ResultState::Incomplete;
    let outcome = calculate_commercial(&incomplete_tooling);
    assert!(codes(&outcome).contains(&"tooling_incomplete"));
}

#[test]
fn hostile_stored_values_are_named_issues_and_never_panic() {
    let mut hostile = inputs(vec![product(1, 2027, 2027, Some(d("80")))], empty_tooling());
    hostile.ebit_target_rate = Some(Decimal::ONE);
    hostile.annual_productivity_rate = Some(d("-0.5"));
    hostile.manual_prices = vec![ScenarioManualPrice { product_id: id(1), price: Decimal::ZERO }];
    hostile.products[0].forecasts = vec![AnnualForecast { year: 2027, quantity: 0 }];
    let outcome = calculate_commercial(&hostile);
    assert_eq!(outcome.state, ResultState::Incomplete);
    let codes = codes(&outcome);
    assert!(codes.contains(&"invalid_ebit_target"), "{codes:?}");
    assert!(codes.contains(&"invalid_productivity"), "{codes:?}");
    assert!(codes.contains(&"invalid_manual_price"), "{codes:?}");
    assert!(codes.contains(&"invalid_forecast"), "{codes:?}");
    assert_eq!(outcome.products[0].manual_price, None);
    assert_eq!(outcome.products[0].suggested_first_year_price, None);
}

#[test]
fn an_unrepresentable_amount_is_an_arithmetic_overflow_issue() {
    let mut product = product(1, 2027, 2027, Some(d("1")));
    product.forecasts = vec![AnnualForecast { year: 2027, quantity: u64::MAX }];
    let mut inputs = inputs(vec![product], empty_tooling());
    inputs.manual_prices = vec![ScenarioManualPrice { product_id: id(1), price: Decimal::MAX }];
    let outcome = calculate_commercial(&inputs);
    assert!(codes(&outcome).contains(&"arithmetic_overflow"), "{:?}", outcome.issues);
    assert_eq!(outcome.state, ResultState::Incomplete);
    assert_eq!(outcome.products[0].years[0].revenue, None);
}

#[test]
fn the_same_inputs_always_produce_the_same_bytes() {
    let inputs = inputs(vec![product(1, 2027, 2029, Some(d("80")))], amortized_tooling("2"));
    let first = serde_json::to_string(&calculate_commercial(&inputs)).expect("the outcome serializes");
    let second = serde_json::to_string(&calculate_commercial(&inputs)).expect("the outcome serializes");
    assert_eq!(first, second);
    assert!(first.contains("solidbom") || !first.contains("created_at"), "no wall-clock value is written");
}

#[test]
fn explanations_name_the_cost_basis_and_the_separate_charges() {
    let mut inputs = inputs(vec![product(1, 2027, 2027, Some(d("80")))], amortized_tooling("2"));
    inputs.manual_prices = vec![ScenarioManualPrice { product_id: id(1), price: d("100") }];
    inputs.setup_total = Some(d("60"));
    let outcome = calculate_commercial(&inputs);
    let labels: Vec<&str> = outcome.explanations.iter().map(|line: &CostLine| line.label.as_str()).collect();
    assert!(labels.iter().any(|label| label.contains("setup")));
    assert!(labels.iter().any(|label| label.contains("tooling")));
    let product_labels: Vec<&str> = outcome.products[0].explanations.iter().map(|line| line.label.as_str()).collect();
    assert!(product_labels.iter().any(|label| label.contains("suggested first-year")));
    assert!(product_labels.iter().any(|label| label.contains("EBIT")));
}
