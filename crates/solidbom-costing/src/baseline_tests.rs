//! Exact arithmetic tests for the SB-16 baseline calculator.
//!
//! The fixtures follow the acceptance vectors of the package requirements: aggregate demand by
//! calendar year before the project policy, purchase tiers, saved FX, allowance once, operation
//! costs, purchased-branch cutoff, setup once per project, nested rollups, product-level categories,
//! repeatability and incomplete results. Money and rates are decimal strings; a missing value is
//! distinct from an explicit zero.

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use solidbom_contract::{
    AnnualForecast, BaselineAttachedCostInput, BaselineCatalogPrice, BaselineCategoryConfig, BaselineCycleInput, BaselineInputs,
    BaselineMaterialInput, BaselineOccurrenceInput, BaselineOperationInput, BaselinePartInput, BaselineProductInput, BaselinePurchaseInput,
    BaselineSetupInput, BaselineSetupOwner, CategoryBasis, CategoryCalculation, CostCategory, CostingVolumePolicy, DirectCostInput, ExchangeRate,
    PurchasePriceTier, ResultState, Sourcing, Unit, YearRange,
};
use uuid::Uuid;

use super::{BASELINE_CALCULATOR_VERSION, calculate_baseline};

fn d(text: &str) -> Decimal {
    text.parse().unwrap_or_default()
}

fn id(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn schedule(sop: u16, eop: u16) -> YearRange {
    YearRange { sop_year: sop, eop_year: eop }
}

fn forecast(year: u16, quantity: u64) -> AnnualForecast {
    AnnualForecast { year, quantity }
}

fn category(code: &str, calculation: CategoryCalculation, included_in_ebit: bool) -> CostCategory {
    CostCategory { code: code.into(), name: code.into(), calculation, included_in_ebit }
}

fn config(categories: Vec<CostCategory>) -> BaselineCategoryConfig {
    BaselineCategoryConfig { config_version_id: id(900), currency: "CNY".into(), categories }
}

fn part(part_version_id: Uuid) -> BaselinePartInput {
    BaselinePartInput {
        part_version_id,
        sourcing: Some(Sourcing::Manufactured),
        material: None,
        operations: Vec::new(),
        attached_costs: Vec::new(),
        purchase: None,
    }
}

fn occurrence(part_version_id: Uuid, key: u128, children: Vec<BaselineOccurrenceInput>) -> BaselineOccurrenceInput {
    BaselineOccurrenceInput { occurrence_id: Some(id(key)), part_version_id: Some(part_version_id), children }
}

fn product(product_id: Uuid, sop: u16, eop: u16, forecasts: Vec<AnnualForecast>, roots: Vec<BaselineOccurrenceInput>) -> BaselineProductInput {
    BaselineProductInput {
        product_id,
        product_revision_id: Some(id(product_id.as_u128() + 1)),
        quoted: true,
        schedule: schedule(sop, eop),
        forecasts,
        root_operations: Vec::new(),
        root_attached_costs: Vec::new(),
        occurrences: roots,
    }
}

fn inputs(parts: Vec<BaselinePartInput>, products: Vec<BaselineProductInput>) -> BaselineInputs {
    BaselineInputs {
        project_currency: "CNY".into(),
        category_config: Some(config(Vec::new())),
        volume_policy: Some(CostingVolumePolicy::Average),
        exchange_rates: Vec::new(),
        parts,
        products,
    }
}

fn cycle_operation(sequence: u32, seconds: &str, basis: solidbom_contract::CycleBasis, cavities: Option<u32>, rate: &str) -> BaselineOperationInput {
    BaselineOperationInput {
        operation_id: id(u128::from(sequence) * 10),
        sequence,
        cycle: Some(BaselineCycleInput { seconds: Some(d(seconds)), basis, cavities, rate_per_hour: Some(d(rate)), rate_currency: "CNY".into() }),
        setup: None,
    }
}

fn with_setup(mut operation: BaselineOperationInput, hours: &str, rate: &str) -> BaselineOperationInput {
    operation.setup = Some(BaselineSetupInput { hours: Some(d(hours)), rate_per_hour: Some(d(rate)), rate_currency: "CNY".into() });
    operation
}

fn attached(line_id: u128, category: &str, quantity: &str, unit_cost: &str) -> BaselineAttachedCostInput {
    BaselineAttachedCostInput { line_id: id(line_id), category: category.into(), quantity: Some(d(quantity)), unit_cost: Some(d(unit_cost)) }
}

/// The V6 made part: 100 cm³ at 2.7 g/cm³ and 20 CNY/kg, 2% allowance, 30 s at 120 CNY/h.
fn made_part() -> BaselinePartInput {
    BaselinePartInput {
        part_version_id: id(3),
        sourcing: Some(Sourcing::Manufactured),
        material: Some(BaselineMaterialInput {
            net_consumption: Some(d("100")),
            unit: Unit::CentimetreCubed,
            density_g_cm3: Some(d("2.7")),
            price_per_kg: Some(d("20")),
            price_currency: "CNY".into(),
            allowance_rate: Some(d("0.02")),
        }),
        operations: vec![cycle_operation(1, "30", solidbom_contract::CycleBasis::PerPiece, Some(1), "120")],
        attached_costs: Vec::new(),
        purchase: None,
    }
}

fn purchased_part(part_version_id: Uuid, price: &str, currency: &str, tiers: Vec<(u64, &str)>) -> BaselinePartInput {
    BaselinePartInput {
        part_version_id,
        sourcing: Some(Sourcing::Purchased),
        material: None,
        operations: Vec::new(),
        attached_costs: Vec::new(),
        purchase: Some(BaselinePurchaseInput {
            single_price: Some(BaselineCatalogPrice { amount: d(price), currency: currency.into(), component_version_id: id(800) }),
            currency: currency.into(),
            price_unit: "pcs".into(),
            tiers: tiers
                .into_iter()
                .map(|(minimum, unit_price)| PurchasePriceTier { minimum_annual_quantity: minimum, unit_price: d(unit_price) })
                .collect(),
        }),
    }
}

fn part_line(outcome: &solidbom_contract::BaselineOutcome, part_version_id: Uuid) -> &solidbom_contract::BaselinePartLine {
    outcome.parts.iter().find(|line| line.part_version_id == part_version_id).expect("the part line exists")
}

fn product_line(outcome: &solidbom_contract::BaselineOutcome, product_id: Uuid) -> &solidbom_contract::BaselineProductLine {
    outcome.products.iter().find(|line| line.product_id == product_id).expect("the product line exists")
}

fn issue_codes(outcome: &solidbom_contract::BaselineOutcome) -> Vec<&str> {
    outcome.issues.iter().map(|issue| issue.code.as_str()).collect()
}

// ---------------------------------------------------------------------------
// V1 — aggregate by calendar year before the policy; peak 300, not 400
// ---------------------------------------------------------------------------

#[test]
fn shared_annual_demand_is_aggregated_before_the_volume_policy() {
    let input = inputs(
        vec![part(id(1))],
        vec![
            product(id(10), 2027, 2028, vec![forecast(2027, 100), forecast(2028, 200)], vec![occurrence(id(1), 11, Vec::new())]),
            product(id(20), 2027, 2028, vec![forecast(2027, 200), forecast(2028, 100)], vec![occurrence(id(1), 21, Vec::new())]),
        ],
    );
    let outcome = calculate_baseline(&input);
    let line = part_line(&outcome, id(1));
    assert_eq!(
        line.annual_demand,
        vec![
            solidbom_contract::BaselineAnnualDemand { year: 2027, quantity: Some(300) },
            solidbom_contract::BaselineAnnualDemand { year: 2028, quantity: Some(300) },
        ]
    );
    assert_eq!(line.lifetime_demand, Some(600));
    assert_eq!(line.peak_demand, Some(300));
    assert_eq!(line.costing_volume, Some(d("300")));
    assert_ne!(line.costing_volume, Some(d("400")), "the sum of product peaks is not the annual shared demand");

    let mut maximum = input.clone();
    maximum.volume_policy = Some(CostingVolumePolicy::Maximum);
    assert_eq!(part_line(&calculate_baseline(&maximum), id(1)).costing_volume, Some(d("300")));
    let mut median = input;
    median.volume_policy = Some(CostingVolumePolicy::Median);
    assert_eq!(part_line(&calculate_baseline(&median), id(1)).costing_volume, Some(d("300")));
}

// ---------------------------------------------------------------------------
// V2/V3 — no zero padding, even-count median, policy selection
// ---------------------------------------------------------------------------

#[test]
fn applicable_years_have_no_zero_padding_and_the_median_handles_even_counts() {
    let mut input = inputs(
        vec![part(id(1))],
        vec![
            product(id(10), 2027, 2028, vec![forecast(2027, 100), forecast(2028, 200)], vec![occurrence(id(1), 11, Vec::new())]),
            product(id(20), 2030, 2031, vec![forecast(2030, 700), forecast(2031, 800)], vec![occurrence(id(1), 21, Vec::new())]),
        ],
    );
    input.volume_policy = Some(CostingVolumePolicy::Median);
    let outcome = calculate_baseline(&input);
    let line = part_line(&outcome, id(1));
    assert_eq!(line.annual_demand.iter().map(|year| year.year).collect::<Vec<_>>(), vec![2027, 2028, 2030, 2031]);
    assert!(line.annual_demand.iter().all(|year| year.year != 2029), "a gap year is absent, not zero");
    assert_eq!(line.costing_volume, Some(d("450")), "the even median is (200 + 700) / 2");
    assert_eq!(line.lifetime_demand, Some(1800));
    input.volume_policy = Some(CostingVolumePolicy::Average);
    assert_eq!(part_line(&calculate_baseline(&input), id(1)).costing_volume, Some(d("450")), "average divides by four applicable years, not six");
    input.volume_policy = Some(CostingVolumePolicy::Maximum);
    assert_eq!(part_line(&calculate_baseline(&input), id(1)).costing_volume, Some(d("800")));
}

#[test]
fn the_policy_applies_to_three_years() {
    let base = inputs(
        vec![part(id(1))],
        vec![product(
            id(10),
            2027,
            2029,
            vec![forecast(2027, 100), forecast(2028, 200), forecast(2029, 600)],
            vec![occurrence(id(1), 11, Vec::new())],
        )],
    );
    let mut average = base.clone();
    average.volume_policy = Some(CostingVolumePolicy::Average);
    assert_eq!(part_line(&calculate_baseline(&average), id(1)).costing_volume, Some(d("300")));
    let mut maximum = base.clone();
    maximum.volume_policy = Some(CostingVolumePolicy::Maximum);
    assert_eq!(part_line(&calculate_baseline(&maximum), id(1)).costing_volume, Some(d("600")));
    let mut median = base;
    median.volume_policy = Some(CostingVolumePolicy::Median);
    assert_eq!(part_line(&calculate_baseline(&median), id(1)).costing_volume, Some(d("200")));
}

// ---------------------------------------------------------------------------
// V4 — purchase tiers: greatest covered minimum, fallback, uncovered = missing
// ---------------------------------------------------------------------------

#[test]
fn the_purchase_tier_is_the_greatest_covered_minimum() {
    let cases = [(300, "250", "8"), (250, "250", "8"), (249, "100", "9"), (500, "500", "7")];
    for (volume, selected, price) in cases {
        let input = inputs(
            vec![purchased_part(id(2), "10", "CNY", vec![(1, "10"), (100, "9"), (250, "8"), (500, "7")])],
            vec![product(id(10), 2027, 2027, vec![forecast(2027, volume)], vec![occurrence(id(2), 11, Vec::new())])],
        );
        let outcome = calculate_baseline(&input);
        let line = part_line(&outcome, id(2));
        assert_eq!(line.costing_volume, Some(d(&volume.to_string())), "volume {volume}");
        assert_eq!(line.unit_cost, Some(d(price)), "volume {volume}");
        let purchase = line.purchase_price.as_ref().expect("a selected purchase price");
        assert_eq!(
            purchase.source,
            solidbom_contract::BaselinePurchasePriceSource::Tier { minimum_annual_quantity: selected.parse().unwrap_or_default() }
        );
    }
}

#[test]
fn an_uncovered_tier_without_a_single_price_is_missing_pricing() {
    let mut input = inputs(
        vec![purchased_part(id(2), "9", "CNY", vec![(100, "9"), (250, "8")])],
        vec![product(id(10), 2027, 2027, vec![forecast(2027, 50)], vec![occurrence(id(2), 11, Vec::new())])],
    );
    input.parts[0].purchase.as_mut().expect("purchase").single_price = None;
    let outcome = calculate_baseline(&input);
    let line = part_line(&outcome, id(2));
    assert_eq!(line.unit_cost, None);
    assert!(issue_codes(&outcome).contains(&"missing_purchase_price"));
    assert_eq!(
        line.annual_demand,
        vec![solidbom_contract::BaselineAnnualDemand { year: 2027, quantity: Some(50) }],
        "demand does not depend on the price"
    );
    assert_eq!(outcome.state, ResultState::Incomplete);

    let mut single = input;
    single.parts[0].purchase.as_mut().expect("purchase").single_price =
        Some(BaselineCatalogPrice { amount: d("12"), currency: "CNY".into(), component_version_id: id(800) });
    assert_eq!(part_line(&calculate_baseline(&single), id(2)).unit_cost, Some(d("12")), "the single price is the fallback");
}

// ---------------------------------------------------------------------------
// V5 — FX direction, same-currency identity, missing rate, converted inputs
// ---------------------------------------------------------------------------

#[test]
fn exchange_rates_multiply_source_amounts_by_saved_project_units_per_source_unit() {
    let mut input = inputs(
        vec![purchased_part(id(2), "10", "USD", Vec::new())],
        vec![product(id(10), 2027, 2027, vec![forecast(2027, 5)], vec![occurrence(id(2), 11, Vec::new())])],
    );
    input.exchange_rates = vec![ExchangeRate { source_currency: "USD".into(), project_units_per_source_unit: d("7.20") }];
    let outcome = calculate_baseline(&input);
    assert_eq!(part_line(&outcome, id(2)).unit_cost, Some(d("72")));
    assert_eq!(part_line(&outcome, id(2)).purchase_price.as_ref().expect("price").fx_rate, Some(d("7.2")));

    // Same currency is the identity and needs no rate.
    let identity = inputs(
        vec![purchased_part(id(2), "72", "CNY", Vec::new())],
        vec![product(id(10), 2027, 2027, vec![forecast(2027, 5)], vec![occurrence(id(2), 11, Vec::new())])],
    );
    assert_eq!(part_line(&calculate_baseline(&identity), id(2)).unit_cost, Some(d("72")));

    // A different currency with no saved rate is missing.
    let mut missing = inputs(
        vec![purchased_part(id(2), "10", "USD", Vec::new())],
        vec![product(id(10), 2027, 2027, vec![forecast(2027, 5)], vec![occurrence(id(2), 11, Vec::new())])],
    );
    missing.exchange_rates = Vec::new();
    let outcome = calculate_baseline(&missing);
    assert_eq!(part_line(&outcome, id(2)).unit_cost, None);
    assert!(issue_codes(&outcome).contains(&"missing_exchange_rate"));
    assert_eq!(outcome.state, ResultState::Incomplete);
}

#[test]
fn converted_material_rate_and_config_amounts_reproduce_the_locked_vector() {
    let mut input =
        inputs(vec![made_part_with_setup()], vec![product(id(10), 2027, 2027, vec![forecast(2027, 100)], vec![occurrence(id(3), 11, Vec::new())])]);
    input.project_currency = "EUR".into();
    input.exchange_rates = vec![ExchangeRate { source_currency: "CNY".into(), project_units_per_source_unit: d("0.13") }];
    input.category_config = Some(BaselineCategoryConfig {
        config_version_id: id(900),
        currency: "CNY".into(),
        categories: vec![
            category("packaging", CategoryCalculation::Fixed { amount: d("5") }, true),
            category(
                "factory-overhead",
                CategoryCalculation::Percentage { basis: CategoryBasis { direct_cost: true, categories: Vec::new() }, rate: d("0.05") },
                true,
            ),
            category(
                "handling",
                CategoryCalculation::Percentage {
                    basis: CategoryBasis { direct_cost: false, categories: vec!["factory-overhead".into()] },
                    rate: d("0.10"),
                },
                true,
            ),
        ],
    });
    let outcome = calculate_baseline(&input);
    let line = part_line(&outcome, id(3));
    let material = line.material.as_ref().expect("material detail");
    assert_eq!(material.price_per_kg, Some(d("20")));
    assert_eq!(material.fx_rate, Some(d("0.13")));
    assert_eq!(material.net_mass_kg, Some(d("0.27")));
    assert_eq!(material.net_material_cost, Some(d("0.702")));
    assert_eq!(material.allowed_material_cost, Some(d("0.71604")));
    assert_eq!(line.production_cost, Some(d("0.13")));
    assert_eq!(line.unit_cost, Some(d("0.84604")));
    let product = product_line(&outcome, id(10));
    let amounts: BTreeMap<&str, Option<Decimal>> = product.categories.iter().map(|category| (category.code.as_str(), category.amount)).collect();
    assert_eq!(amounts.get("packaging"), Some(&Some(d("0.65"))));
    assert_eq!(amounts.get("factory-overhead"), Some(&Some(d("0.042302"))));
    assert_eq!(amounts.get("handling"), Some(&Some(d("0.0042302"))));
    assert_eq!(product.unit_total, Some(d("1.5425722")));
    assert_eq!(outcome.setup_total, Some(d("7.8")));
}

// ---------------------------------------------------------------------------
// V6/V14 — mass, allowance once, cavity conversion, explicit zeros, explanation lines
// ---------------------------------------------------------------------------

fn made_part_with_setup() -> BaselinePartInput {
    let mut part = made_part();
    part.operations[0] = with_setup(part.operations[0].clone(), "0.5", "120");
    part
}

#[test]
fn net_mass_allowance_and_production_follow_the_locked_arithmetic() {
    let input = inputs(vec![made_part()], vec![product(id(10), 2027, 2027, vec![forecast(2027, 100)], vec![occurrence(id(3), 11, Vec::new())])]);
    let outcome = calculate_baseline(&input);
    let line = part_line(&outcome, id(3));
    let material = line.material.as_ref().expect("material detail");
    assert_eq!(material.net_volume, Some(d("100")));
    assert_eq!(material.net_volume_unit, Unit::CentimetreCubed);
    assert_eq!(material.net_mass_kg, Some(d("0.27")));
    assert_eq!(material.net_material_cost, Some(d("5.4")));
    assert_eq!(material.allowance_rate, Some(d("0.02")));
    assert_eq!(material.allowed_material_cost, Some(d("5.508")));
    assert_eq!(line.production_cost, Some(d("1")));
    assert_eq!(line.unit_cost, Some(d("6.508")));
    assert_eq!(outcome.state, ResultState::Complete);

    // The mm³ variant is the same physical consumption.
    let mut millimetres = input.clone();
    millimetres.parts[0].material.as_mut().expect("material").net_consumption = Some(d("100000"));
    millimetres.parts[0].material.as_mut().expect("material").unit = Unit::MillimetreCubed;
    assert_eq!(part_line(&calculate_baseline(&millimetres), id(3)).material.as_ref().expect("material").net_mass_kg, Some(d("0.27")));

    // A per-cycle basis divides by its declared cavities.
    let mut per_cycle = input.clone();
    let operation = &mut per_cycle.parts[0].operations[0];
    operation.cycle.as_mut().expect("cycle").seconds = Some(d("60"));
    operation.cycle.as_mut().expect("cycle").basis = solidbom_contract::CycleBasis::PerCycle;
    operation.cycle.as_mut().expect("cycle").cavities = Some(2);
    assert_eq!(part_line(&calculate_baseline(&per_cycle), id(3)).production_cost, Some(d("1")));

    // Explicit zeros are values.
    let mut zero_allowance = input.clone();
    zero_allowance.parts[0].material.as_mut().expect("material").allowance_rate = Some(Decimal::ZERO);
    assert_eq!(part_line(&calculate_baseline(&zero_allowance), id(3)).material.as_ref().expect("material").allowed_material_cost, Some(d("5.4")));
    let mut zero_price = input.clone();
    zero_price.parts[0].material.as_mut().expect("material").price_per_kg = Some(Decimal::ZERO);
    let zero_price_outcome = calculate_baseline(&zero_price);
    assert_eq!(part_line(&zero_price_outcome, id(3)).unit_cost, Some(d("1")));
    assert_eq!(zero_price_outcome.state, ResultState::Complete);
    let mut zero_cycle = input.clone();
    zero_cycle.parts[0].operations[0].cycle.as_mut().expect("cycle").seconds = Some(Decimal::ZERO);
    zero_cycle.parts[0].operations[0].cycle.as_mut().expect("cycle").rate_per_hour = None;
    assert_eq!(part_line(&calculate_baseline(&zero_cycle), id(3)).production_cost, Some(Decimal::ZERO));

    // A missing allowance is incomplete, never zero.
    let mut missing = input;
    missing.parts[0].material.as_mut().expect("material").allowance_rate = None;
    let missing_outcome = calculate_baseline(&missing);
    assert_eq!(part_line(&missing_outcome, id(3)).unit_cost, None);
    assert!(issue_codes(&missing_outcome).contains(&"missing_allowance"));
    assert_eq!(part_line(&missing_outcome, id(3)).material.as_ref().expect("material").net_mass_kg, Some(d("0.27")));
}

#[test]
fn native_explanations_carry_the_locked_line_codes() {
    let input =
        inputs(vec![made_part_with_setup()], vec![product(id(10), 2027, 2027, vec![forecast(2027, 100)], vec![occurrence(id(3), 11, Vec::new())])]);
    let outcome = calculate_baseline(&input);
    let line = part_line(&outcome, id(3));
    let amounts: BTreeMap<&str, Option<Decimal>> = line.explanations.iter().map(|line| (line.code.as_str(), line.amount)).collect();
    assert_eq!(amounts.get("material.net_mass_kg"), Some(&Some(d("0.27"))));
    assert_eq!(amounts.get("material.net_cost"), Some(&Some(d("5.4"))));
    assert_eq!(amounts.get("material.allowance_factor"), Some(&Some(d("1.02"))));
    assert_eq!(amounts.get("material.allowed_cost"), Some(&Some(d("5.508"))));
    assert_eq!(amounts.get("production.cost"), Some(&Some(d("1"))));
    assert_eq!(amounts.get("part.direct_cost"), Some(&Some(d("6.508"))));
    assert_eq!(amounts.get("setup.charge"), Some(&Some(d("60"))));
    assert_eq!(outcome.setup_total, Some(d("60")));
}

// ---------------------------------------------------------------------------
// V7 — configured categories are applied once on the finished product
// ---------------------------------------------------------------------------

fn v7_config() -> BaselineCategoryConfig {
    BaselineCategoryConfig {
        config_version_id: id(900),
        currency: "CNY".into(),
        categories: vec![
            category("material", CategoryCalculation::Direct { input: DirectCostInput::Material }, false),
            category("production", CategoryCalculation::Direct { input: DirectCostInput::Production }, false),
            category("setup", CategoryCalculation::Direct { input: DirectCostInput::Setup }, false),
            category("packaging", CategoryCalculation::Fixed { amount: d("5") }, true),
            category(
                "factory-overhead",
                CategoryCalculation::Percentage { basis: CategoryBasis { direct_cost: true, categories: Vec::new() }, rate: d("0.05") },
                true,
            ),
            category(
                "handling",
                CategoryCalculation::Percentage {
                    basis: CategoryBasis { direct_cost: false, categories: vec!["factory-overhead".into()] },
                    rate: d("0.10"),
                },
                true,
            ),
        ],
    }
}

#[test]
fn categories_are_views_or_one_addition_per_finished_product_and_setup_stays_separate() {
    let mut input =
        inputs(vec![made_part_with_setup()], vec![product(id(10), 2027, 2027, vec![forecast(2027, 100)], vec![occurrence(id(3), 11, Vec::new())])]);
    input.category_config = Some(v7_config());
    let outcome = calculate_baseline(&input);
    let product = product_line(&outcome, id(10));
    let amounts: BTreeMap<&str, Option<Decimal>> = product.categories.iter().map(|category| (category.code.as_str(), category.amount)).collect();
    assert_eq!(amounts.get("material"), Some(&Some(d("5.508"))), "a direct category is a view of the rolled material");
    assert_eq!(amounts.get("production"), Some(&Some(d("1"))), "a direct category is a view of the rolled production");
    assert_eq!(amounts.get("factory-overhead"), Some(&Some(d("0.3254"))), "the direct-cost basis excludes setup");
    assert_eq!(amounts.get("handling"), Some(&Some(d("0.03254"))), "percentage categories evaluate in dependency order");
    assert_eq!(amounts.get("packaging"), Some(&Some(d("5"))));
    assert!(!amounts.contains_key("setup"), "a setup-scope category belongs to the setup section");
    assert_eq!(product.unit_total, Some(d("11.86594")));
    assert_eq!(product.recurring_unit_cost, Some(d("11.86594")));
    assert_eq!(outcome.setup_total, Some(d("60")));
    let setup_amounts: BTreeMap<&str, Option<Decimal>> =
        outcome.setup_categories.iter().map(|category| (category.code.as_str(), category.amount)).collect();
    assert_eq!(setup_amounts.get("setup"), Some(&Some(d("60"))));
}

// ---------------------------------------------------------------------------
// V8 — purchased parent suppresses descendants; the same child elsewhere counts
// ---------------------------------------------------------------------------

#[test]
fn a_purchased_parent_suppresses_descendant_demand_and_cost_but_not_the_same_child_elsewhere() {
    let mut child = made_part();
    child.part_version_id = id(4);
    let input = inputs(
        vec![purchased_part(id(2), "500", "CNY", Vec::new()), child],
        vec![
            product(
                id(10),
                2027,
                2028,
                vec![forecast(2027, 10), forecast(2028, 10)],
                vec![occurrence(id(2), 11, vec![occurrence(id(4), 12, Vec::new()), occurrence(id(4), 13, Vec::new())])],
            ),
            product(id(20), 2027, 2028, vec![forecast(2027, 20), forecast(2028, 20)], vec![occurrence(id(4), 21, Vec::new())]),
        ],
    );
    let outcome = calculate_baseline(&input);
    let purchased = part_line(&outcome, id(2));
    assert_eq!(
        purchased.annual_demand,
        vec![
            solidbom_contract::BaselineAnnualDemand { year: 2027, quantity: Some(10) },
            solidbom_contract::BaselineAnnualDemand { year: 2028, quantity: Some(10) },
        ]
    );
    let child_line = part_line(&outcome, id(4));
    assert_eq!(
        child_line.annual_demand,
        vec![
            solidbom_contract::BaselineAnnualDemand { year: 2027, quantity: Some(20) },
            solidbom_contract::BaselineAnnualDemand { year: 2028, quantity: Some(20) },
        ],
        "A's purchased branch is pruned, B's occurrence still counts"
    );
    assert_eq!(product_line(&outcome, id(10)).unit_total, Some(d("500")), "the purchase price covers the children");
    assert_eq!(product_line(&outcome, id(20)).unit_total, Some(d("6.508")));
    assert_eq!(
        outcome.annual,
        vec![
            solidbom_contract::BaselineAnnualProjectCost { year: 2027, manufacturing_cost: Some(d("5130.16")) },
            solidbom_contract::BaselineAnnualProjectCost { year: 2028, manufacturing_cost: Some(d("5130.16")) },
        ]
    );
}

// ---------------------------------------------------------------------------
// V9/V10 — nested rollups, shared unit costs, setup once per project
// ---------------------------------------------------------------------------

fn nested_fixture() -> BaselineInputs {
    let made = made_part();
    let mut subassembly = part(id(5));
    subassembly.operations = vec![with_setup(cycle_operation(1, "20", solidbom_contract::CycleBasis::PerPiece, Some(1), "180"), "0.25", "100")];
    let mut product_a =
        product(id(10), 2027, 2027, vec![forecast(2027, 100)], vec![occurrence(id(5), 11, Vec::new()), occurrence(id(5), 12, Vec::new())]);
    product_a.root_operations = vec![with_setup(cycle_operation(1, "45", solidbom_contract::CycleBasis::PerPiece, Some(1), "200"), "0.1", "200")];
    product_a.root_attached_costs = vec![attached(700, "tape", "2", "0.5")];
    let mut product_b = product(id(20), 2027, 2027, vec![forecast(2027, 50)], vec![occurrence(id(5), 21, Vec::new())]);
    product_b.root_operations = vec![with_setup(cycle_operation(1, "90", solidbom_contract::CycleBasis::PerPiece, Some(1), "120"), "0.05", "200")];
    // Give M its own setup once; the composition comes from the occurrence tree.
    let mut made = made;
    made.operations[0] = with_setup(made.operations[0].clone(), "0.5", "120");
    let mut input = inputs(vec![made, subassembly], vec![product_a, product_b]);
    input.category_config = Some(v7_config());
    input
}

#[test]
fn nested_rollups_reuse_each_shared_unit_once_per_occurrence() {
    let mut input = nested_fixture();
    // Place three M children under each S occurrence in both products.
    for product in &mut input.products {
        for root in &mut product.occurrences {
            root.children = vec![occurrence(id(3), 100, Vec::new()), occurrence(id(3), 101, Vec::new()), occurrence(id(3), 102, Vec::new())];
        }
    }
    let outcome = calculate_baseline(&input);
    let m = part_line(&outcome, id(3));
    assert_eq!(m.unit_cost, Some(d("6.508")));
    let s = part_line(&outcome, id(5));
    assert_eq!(s.material_cost, Some(d("16.524")), "rolled material is three child materials");
    assert_eq!(s.production_cost, Some(d("4")), "rolled production is own 1 + three child productions");
    assert_eq!(s.unit_cost, Some(d("20.524")));
    assert_eq!(s.annual_demand, vec![solidbom_contract::BaselineAnnualDemand { year: 2027, quantity: Some(250) }]);
    assert_eq!(m.annual_demand, vec![solidbom_contract::BaselineAnnualDemand { year: 2027, quantity: Some(750) }]);

    let a = product_line(&outcome, id(10));
    assert_eq!(a.material_cost, Some(d("33.048")));
    assert_eq!(a.production_cost, Some(d("10.5")));
    assert_eq!(a.attached_cost, Some(d("1")));
    assert_eq!(a.unit_total, Some(d("51.94314")));
    let b = product_line(&outcome, id(20));
    assert_eq!(b.material_cost, Some(d("16.524")));
    assert_eq!(b.production_cost, Some(d("7")));
    assert_eq!(b.unit_total, Some(d("29.81782")));

    // Setup: M 60, S 25, A root 20, B root 10 — once each, never per occurrence.
    let charges: BTreeMap<&str, Option<Decimal>> = outcome
        .setup
        .iter()
        .map(|line| match line.owner {
            BaselineSetupOwner::PartVersion { part_version_id } => (if part_version_id == id(3) { "M" } else { "S" }, line.setup_charge),
            BaselineSetupOwner::Product { product_id } => (if product_id == id(10) { "A" } else { "B" }, line.setup_charge),
        })
        .collect();
    assert_eq!(charges.get("M"), Some(&Some(d("60"))));
    assert_eq!(charges.get("S"), Some(&Some(d("25"))));
    assert_eq!(charges.get("A"), Some(&Some(d("20"))));
    assert_eq!(charges.get("B"), Some(&Some(d("10"))));
    assert_eq!(outcome.setup_total, Some(d("115")));
    assert_eq!(outcome.annual, vec![solidbom_contract::BaselineAnnualProjectCost { year: 2027, manufacturing_cost: Some(d("6685.205")) }]);
    assert_eq!(outcome.project_total_manufacturing_cost, Some(d("6800.205")), "the annual series plus the separate setup total");
    assert_eq!(outcome.state, ResultState::Complete);
}

// ---------------------------------------------------------------------------
// V11/V15 — missing values stay missing, independent amounts survive
// ---------------------------------------------------------------------------

#[test]
fn a_missing_forecast_keeps_its_year_unavailable_and_the_result_incomplete() {
    let input = inputs(
        vec![part(id(1)), part(id(2))],
        vec![
            product(id(10), 2027, 2028, vec![forecast(2027, 100)], vec![occurrence(id(1), 11, Vec::new())]),
            product(
                id(20),
                2027,
                2028,
                vec![forecast(2027, 200), forecast(2028, 100)],
                vec![occurrence(id(1), 21, Vec::new()), occurrence(id(2), 22, Vec::new())],
            ),
        ],
    );
    let outcome = calculate_baseline(&input);
    let shared = part_line(&outcome, id(1));
    assert_eq!(
        shared.annual_demand,
        vec![
            solidbom_contract::BaselineAnnualDemand { year: 2027, quantity: Some(300) },
            solidbom_contract::BaselineAnnualDemand { year: 2028, quantity: None },
        ]
    );
    assert_eq!(shared.lifetime_demand, None);
    assert_eq!(shared.peak_demand, None);
    assert_eq!(shared.costing_volume, None, "the policy is never computed from a partial year set");
    assert_eq!(outcome.state, ResultState::Incomplete);
    assert!(issue_codes(&outcome).contains(&"missing_forecast"));
    let unrelated = part_line(&outcome, id(2));
    assert_eq!(
        unrelated.annual_demand,
        vec![
            solidbom_contract::BaselineAnnualDemand { year: 2027, quantity: Some(200) },
            solidbom_contract::BaselineAnnualDemand { year: 2028, quantity: Some(100) },
        ],
        "a complete independent part keeps its own demand"
    );
}

#[test]
fn a_quoted_product_without_a_selected_revision_is_incomplete_not_silently_costed() {
    let mut input = inputs(vec![part(id(1))], vec![product(id(10), 2027, 2027, vec![forecast(2027, 100)], vec![occurrence(id(1), 11, Vec::new())])]);
    input.products[0].product_revision_id = None;
    let outcome = calculate_baseline(&input);
    assert_eq!(outcome.state, ResultState::Incomplete);
    assert!(issue_codes(&outcome).contains(&"missing_revision_selection"));
    assert_eq!(outcome.project_total_manufacturing_cost, None);
}

// ---------------------------------------------------------------------------
// V12 — out-of-schedule years are absent, never padded (defensive input path)
// ---------------------------------------------------------------------------

#[test]
fn a_forecast_outside_the_schedule_is_not_a_demand_row() {
    let mut input = inputs(
        vec![part(id(1))],
        vec![product(
            id(10),
            2027,
            2028,
            vec![forecast(2027, 100), forecast(2028, 200), forecast(2029, 999)],
            vec![occurrence(id(1), 11, Vec::new())],
        )],
    );
    input.volume_policy = Some(CostingVolumePolicy::Average);
    let outcome = calculate_baseline(&input);
    let line = part_line(&outcome, id(1));
    assert_eq!(line.annual_demand.iter().map(|year| year.year).collect::<Vec<_>>(), vec![2027, 2028]);
    assert_eq!(line.lifetime_demand, Some(300));
}

// ---------------------------------------------------------------------------
// V13 — repeatability and order/scale independence
// ---------------------------------------------------------------------------

#[test]
fn identical_inputs_produce_identical_bytes_and_order_and_scale_do_not_matter() {
    let mut input = nested_fixture();
    for product in &mut input.products {
        for root in &mut product.occurrences {
            root.children = vec![occurrence(id(3), 100, Vec::new()), occurrence(id(3), 101, Vec::new()), occurrence(id(3), 102, Vec::new())];
        }
    }
    let first = calculate_baseline(&input);
    let second = calculate_baseline(&input);
    assert_eq!(first, second);
    assert_eq!(serde_json::to_string(&first).unwrap_or_default(), serde_json::to_string(&second).unwrap_or_default());
    assert_eq!(first.project_total_manufacturing_cost, second.project_total_manufacturing_cost);

    let mut permuted = input.clone();
    permuted.products.reverse();
    for product in &mut permuted.products {
        product.forecasts.reverse();
    }
    permuted.parts.reverse();
    let reordered = calculate_baseline(&permuted);
    assert_eq!(
        serde_json::to_string(&first).unwrap_or_default(),
        serde_json::to_string(&reordered).unwrap_or_default(),
        "product, part and forecast order must not change the answer"
    );
    assert_eq!(BASELINE_CALCULATOR_VERSION, "solidbom-manufacturing-baseline/1");
}

// ---------------------------------------------------------------------------
// Explanation and incomplete-shape coverage
// ---------------------------------------------------------------------------

#[test]
fn a_missing_category_configuration_keeps_direct_amounts_but_withholds_the_unit_total() {
    let mut input = inputs(vec![made_part()], vec![product(id(10), 2027, 2027, vec![forecast(2027, 100)], vec![occurrence(id(3), 11, Vec::new())])]);
    input.category_config = None;
    let outcome = calculate_baseline(&input);
    let line = part_line(&outcome, id(3));
    assert_eq!(line.unit_cost, Some(d("6.508")), "direct part amounts are still reported");
    assert_eq!(product_line(&outcome, id(10)).material_cost, Some(d("5.508")));
    assert_eq!(product_line(&outcome, id(10)).unit_total, None);
    assert_eq!(outcome.state, ResultState::Incomplete);
    assert!(issue_codes(&outcome).contains(&"missing_cost_config"));
}

#[test]
fn an_attached_line_needs_its_quantity_and_unit_cost() {
    let mut input = inputs(vec![made_part()], vec![product(id(10), 2027, 2027, vec![forecast(2027, 100)], vec![occurrence(id(3), 11, Vec::new())])]);
    input.parts[0].attached_costs =
        vec![BaselineAttachedCostInput { line_id: id(600), category: "packaging".into(), quantity: Some(d("2")), unit_cost: None }];
    let outcome = calculate_baseline(&input);
    assert!(issue_codes(&outcome).contains(&"missing_attached_unit_cost"));
    assert_eq!(part_line(&outcome, id(3)).attached_cost, None);
    assert_eq!(outcome.state, ResultState::Incomplete);
}

#[test]
fn missing_demand_policy_withholds_the_representative_volume() {
    let mut input = inputs(vec![part(id(1))], vec![product(id(10), 2027, 2027, vec![forecast(2027, 100)], vec![occurrence(id(1), 11, Vec::new())])]);
    input.volume_policy = None;
    let outcome = calculate_baseline(&input);
    assert_eq!(part_line(&outcome, id(1)).costing_volume, None);
    assert_eq!(part_line(&outcome, id(1)).lifetime_demand, Some(100), "lifetime demand does not depend on the policy");
    assert!(issue_codes(&outcome).contains(&"missing_volume_policy"));
}

#[test]
fn an_excluded_product_contributes_no_demand_or_cost() {
    let mut excluded = product(id(30), 2027, 2027, vec![forecast(2027, 500)], vec![occurrence(id(1), 31, Vec::new())]);
    excluded.quoted = false;
    let input =
        inputs(vec![part(id(1))], vec![product(id(10), 2027, 2027, vec![forecast(2027, 100)], vec![occurrence(id(1), 11, Vec::new())]), excluded]);
    let outcome = calculate_baseline(&input);
    assert_eq!(part_line(&outcome, id(1)).annual_demand, vec![solidbom_contract::BaselineAnnualDemand { year: 2027, quantity: Some(100) }]);
    assert!(outcome.products.iter().all(|product| product.product_id != id(30)));
}

#[test]
fn an_unsupported_purchase_unit_is_missing_pricing_not_an_invented_factor() {
    let mut input = inputs(
        vec![purchased_part(id(2), "10", "CNY", Vec::new())],
        vec![product(id(10), 2027, 2027, vec![forecast(2027, 5)], vec![occurrence(id(2), 11, Vec::new())])],
    );
    input.parts[0].purchase.as_mut().expect("purchase").price_unit = "kg".into();
    let outcome = calculate_baseline(&input);
    assert!(issue_codes(&outcome).contains(&"unsupported_purchase_unit"));
    assert_eq!(part_line(&outcome, id(2)).unit_cost, None);
}
