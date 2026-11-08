//! The approved prototype fixture and its edge cases, in exact decimals.

use rust_decimal::Decimal;
use solidbom_contract::{CalculationInputs, CalculationOutcome, ResolvedPartInput, ResultState};
use uuid::Uuid;

use super::{calculate, calculator_version};

fn d(text: &str) -> Decimal {
    text.parse().expect("a decimal in the fixture")
}

fn part() -> ResolvedPartInput {
    ResolvedPartInput {
        part_version_id: Uuid::nil(),
        quantity: 3,
        volume_mm3: Some(d("2400")),
        density_g_cm3: Some(d("2.7")),
        material_price_per_kg: Some(d("20")),
        allowance_rate: Some(d("0.02")),
        cycle_seconds: Some(d("30")),
        cavities: Some(1),
        production_rate_per_hour: Some(d("120")),
        setup_hours: Some(d("0.5")),
        setup_rate_per_hour: Some(d("120")),
    }
}

fn fixture() -> CalculationInputs {
    CalculationInputs { currency: "CNY".into(), material_overhead_rate: Some(d("0.05")), parts: vec![part()] }
}

fn codes(outcome: &CalculationOutcome) -> Vec<&str> {
    outcome.issues.iter().map(|issue| issue.code.as_str()).collect()
}

fn has(outcome: &CalculationOutcome, code: &str) -> bool {
    outcome.issues.iter().any(|issue| issue.code == code)
}

fn field_of<'a>(outcome: &'a CalculationOutcome, code: &str) -> Option<&'a str> {
    outcome.issues.iter().find(|issue| issue.code == code).map(|issue| issue.field_path.as_str())
}

fn complete_fixture() -> CalculationOutcome {
    let outcome = calculate(&fixture());
    assert_eq!(outcome.state, ResultState::Complete, "{:?}", outcome.issues);
    assert!(outcome.issues.is_empty(), "{:?}", outcome.issues);
    outcome
}

#[test]
fn fixture_cost() {
    let outcome = complete_fixture();
    let part = &outcome.parts[0];
    assert_eq!(part.net_mass_kg, Some(d("0.00648")));
    assert_eq!(part.allowed_material_cost, Some(d("0.132192")));
    assert_eq!(part.material_overhead, Some(d("0.0066096")));
    assert_eq!(part.production_cost, Some(d("1")));
    assert_eq!(part.per_part_subtotal, Some(d("1.1388016")));
    assert_eq!(part.line_total, Some(d("3.4164048")));
    assert_eq!(outcome.assembly_subtotal, Some(d("3.4164048")));
    assert_eq!(outcome.setup_total, Some(d("60")));
}

#[test]
fn the_calculator_version_is_pinned() {
    assert_eq!(calculator_version(), "solidbom-direct-cost/1");
}

#[test]
fn repeat_quantity() {
    let outcome = complete_fixture();
    let part = &outcome.parts[0];
    assert_eq!(part.quantity, 3);
    assert_eq!(part.line_total, part.per_part_subtotal.map(|subtotal| subtotal * Decimal::from(3)));
    assert_eq!(outcome.assembly_subtotal, Some(d("3.4164048")));
}

#[test]
fn assembly_totals_come_from_unrounded_values() {
    let outcome = complete_fixture();
    // 3 × the displayed 1.14 would be 3.42, not the exact 3.4164048.
    assert_eq!(outcome.assembly_subtotal, Some(d("3.4164048")));
    assert_ne!(outcome.assembly_subtotal, Some(d("3.42")));
    assert_eq!(outcome.assembly_subtotal, outcome.parts[0].line_total);
}

#[test]
fn sixty_second_cycle() {
    let mut input = fixture();
    input.parts[0].cycle_seconds = Some(d("60"));
    let outcome = calculate(&input);
    assert_eq!(outcome.state, ResultState::Complete);
    assert_eq!(outcome.parts[0].production_cost, Some(d("2")));
    assert_eq!(outcome.parts[0].per_part_subtotal, Some(d("2.1388016")));
    assert_eq!(outcome.assembly_subtotal, Some(d("6.4164048")));
    assert_eq!(outcome.setup_total, Some(d("60")));
}

#[test]
fn the_earlier_outcome_and_its_inputs_stay_unchanged_after_an_edit() {
    let input = fixture();
    let first = calculate(&input);

    let mut changed = input.clone();
    changed.parts[0].cycle_seconds = Some(d("60"));
    let second = calculate(&changed);

    assert_eq!(second.parts[0].per_part_subtotal, Some(d("2.1388016")));
    assert_eq!(second.assembly_subtotal, Some(d("6.4164048")));

    // P07: the earlier result remains readable with its original inputs.
    assert_eq!(first.parts[0].per_part_subtotal, Some(d("1.1388016")));
    assert_eq!(first.assembly_subtotal, Some(d("3.4164048")));
    assert_eq!(input.parts[0].cycle_seconds, Some(d("30")));
    assert_eq!(calculate(&input), first);
}

#[test]
fn allowance_once() {
    let with = complete_fixture();
    let mut without = fixture();
    without.parts[0].allowance_rate = Some(Decimal::ZERO);
    let without = calculate(&without);
    let with_part = &with.parts[0];
    let without_part = &without.parts[0];
    assert_eq!(with_part.production_cost, without_part.production_cost);
    assert_eq!(with_part.allowed_material_cost, without_part.allowed_material_cost.map(|cost| cost * d("1.02")));
    assert_eq!(with_part.material_overhead, without_part.material_overhead.map(|cost| cost * d("1.02")));
    assert_eq!(with.setup_total, without.setup_total);
}

#[test]
fn setup_once() {
    let mut one = fixture();
    one.parts[0].quantity = 1;
    let one = calculate(&one);
    let three = complete_fixture();
    assert_eq!(one.setup_total, Some(d("60")));
    assert_eq!(three.setup_total, Some(d("60")));
    assert_eq!(one.parts[0].per_part_subtotal, three.parts[0].per_part_subtotal);
    assert_eq!(one.parts[0].per_part_subtotal, Some(d("1.1388016")));
    assert_eq!(one.parts[0].line_total, Some(d("1.1388016")));
    assert_eq!(three.parts[0].line_total, Some(d("3.4164048")));
}

#[test]
fn cavities_divide_the_cycle() {
    let mut input = fixture();
    input.parts[0].cavities = Some(2);
    let outcome = calculate(&input);
    assert_eq!(outcome.state, ResultState::Complete);
    assert_eq!(outcome.parts[0].production_cost, Some(d("0.5")));
    assert_eq!(outcome.parts[0].per_part_subtotal, Some(d("0.6388016")));
    assert_eq!(outcome.assembly_subtotal, Some(d("1.9164048")));
}

#[test]
fn an_omitted_cavity_count_with_a_nonzero_cycle_is_missing() {
    let mut input = fixture();
    input.parts[0].cavities = None;
    let outcome = calculate(&input);
    assert_eq!(outcome.state, ResultState::Incomplete);
    assert_eq!(field_of(&outcome, "missing_cavities"), Some("parts[0].cavities"));
    assert_eq!(outcome.parts[0].production_cost, None);
    assert_eq!(outcome.parts[0].per_part_subtotal, None);
    assert_eq!(outcome.parts[0].line_total, None);
    assert_eq!(outcome.assembly_subtotal, None);
    // The setup charge does not depend on the cavities, so it stays available.
    assert_eq!(outcome.setup_total, Some(d("60")));
}

#[test]
fn missing_overhead_rate_is_incomplete() {
    let mut input = fixture();
    input.material_overhead_rate = None;
    let outcome = calculate(&input);
    assert_eq!(outcome.state, ResultState::Incomplete);
    assert_eq!(outcome.parts[0].allowed_material_cost, Some(d("0.132192")));
    assert_eq!(outcome.parts[0].material_overhead, None);
    assert_eq!(outcome.parts[0].per_part_subtotal, None);
    assert_eq!(outcome.assembly_subtotal, None);
    assert_eq!(outcome.setup_total, Some(d("60")));
    assert_eq!(field_of(&outcome, "missing_overhead_rate"), Some("material_overhead_rate"));
}

#[test]
fn missing_density() {
    let mut input = fixture();
    input.parts[0].density_g_cm3 = None;
    let outcome = calculate(&input);
    assert_eq!(outcome.state, ResultState::Incomplete);
    assert_eq!(outcome.parts[0].net_mass_kg, None);
    assert_eq!(outcome.parts[0].allowed_material_cost, None);
    assert_eq!(outcome.parts[0].per_part_subtotal, None);
    assert_eq!(outcome.assembly_subtotal, None);
    // The setup charge does not depend on the material, so it stays available.
    assert_eq!(outcome.setup_total, Some(d("60")));
    assert_eq!(field_of(&outcome, "missing_density"), Some("parts[0].density_g_cm3"));
    assert!(!has(&outcome, "arithmetic_overflow"));
}

#[test]
fn missing_material_price_is_named() {
    let mut input = fixture();
    input.parts[0].material_price_per_kg = None;
    let outcome = calculate(&input);
    assert_eq!(outcome.state, ResultState::Incomplete);
    assert_eq!(outcome.parts[0].net_mass_kg, Some(d("0.00648")));
    assert_eq!(outcome.parts[0].allowed_material_cost, None);
    assert_eq!(outcome.parts[0].per_part_subtotal, None);
    assert_eq!(field_of(&outcome, "missing_material_price"), Some("parts[0].material_price_per_kg"));
}

#[test]
fn missing_allowance_is_named() {
    let mut input = fixture();
    input.parts[0].allowance_rate = None;
    let outcome = calculate(&input);
    assert_eq!(outcome.state, ResultState::Incomplete);
    assert_eq!(outcome.parts[0].allowed_material_cost, None);
    assert_eq!(outcome.parts[0].per_part_subtotal, None);
    assert_eq!(field_of(&outcome, "missing_allowance_rate"), Some("parts[0].allowance_rate"));
}

#[test]
fn missing_cycle_time_is_incomplete() {
    let mut input = fixture();
    input.parts[0].cycle_seconds = None;
    let outcome = calculate(&input);
    assert_eq!(outcome.state, ResultState::Incomplete);
    assert_eq!(outcome.parts[0].production_cost, None);
    assert_eq!(outcome.parts[0].per_part_subtotal, None);
    assert_eq!(outcome.setup_total, Some(d("60")));
    assert_eq!(field_of(&outcome, "missing_cycle_time"), Some("parts[0].cycle_seconds"));
}

#[test]
fn missing_production_rate_is_named() {
    let mut input = fixture();
    input.parts[0].production_rate_per_hour = None;
    let outcome = calculate(&input);
    assert_eq!(outcome.state, ResultState::Incomplete);
    assert_eq!(outcome.parts[0].production_cost, None);
    assert_eq!(outcome.parts[0].per_part_subtotal, None);
    assert_eq!(field_of(&outcome, "missing_production_rate"), Some("parts[0].production_rate_per_hour"));
}

#[test]
fn missing_volume_is_missing_never_zero() {
    let mut input = fixture();
    input.parts[0].volume_mm3 = None;
    let outcome = calculate(&input);
    assert_eq!(outcome.state, ResultState::Incomplete);
    assert_eq!(outcome.parts[0].net_mass_kg, None);
    assert_eq!(outcome.parts[0].allowed_material_cost, None);
    assert_eq!(outcome.parts[0].per_part_subtotal, None);
    assert_eq!(outcome.assembly_subtotal, None);
    assert_eq!(outcome.setup_total, Some(d("60")));
    assert_eq!(field_of(&outcome, "missing_volume"), Some("parts[0].volume_mm3"));
}

#[test]
fn omitted_setup_is_missing_not_zero() {
    let mut input = fixture();
    input.parts[0].setup_hours = None;
    let outcome = calculate(&input);
    assert_eq!(outcome.state, ResultState::Incomplete);
    assert_eq!(outcome.setup_total, None);
    // The direct-cost subtotal does not depend on setup, so it stays available.
    assert_eq!(outcome.parts[0].per_part_subtotal, Some(d("1.1388016")));
    assert_eq!(outcome.assembly_subtotal, Some(d("3.4164048")));
    assert_eq!(field_of(&outcome, "missing_setup_hours"), Some("parts[0].setup_hours"));
}

#[test]
fn missing_setup_rate_is_named_only_when_setup_time_is_positive() {
    let mut input = fixture();
    input.parts[0].setup_rate_per_hour = None;
    let outcome = calculate(&input);
    assert_eq!(outcome.state, ResultState::Incomplete);
    assert_eq!(outcome.setup_total, None);
    assert_eq!(outcome.parts[0].per_part_subtotal, Some(d("1.1388016")));
    assert_eq!(field_of(&outcome, "missing_setup_rate"), Some("parts[0].setup_rate_per_hour"));

    input.parts[0].setup_hours = Some(Decimal::ZERO);
    let outcome = calculate(&input);
    assert_eq!(outcome.state, ResultState::Complete, "{:?}", outcome.issues);
    assert_eq!(outcome.setup_total, Some(Decimal::ZERO));
}

#[test]
fn zero_cycle_is_valid_and_gives_zero_production() {
    let mut input = fixture();
    input.parts[0].cycle_seconds = Some(Decimal::ZERO);
    input.parts[0].cavities = None;
    input.parts[0].production_rate_per_hour = None;
    let outcome = calculate(&input);
    assert_eq!(outcome.state, ResultState::Complete, "{:?}", outcome.issues);
    assert_eq!(outcome.parts[0].production_cost, Some(Decimal::ZERO));
    assert_eq!(outcome.parts[0].per_part_subtotal, Some(d("0.1388016")));
    assert_eq!(outcome.assembly_subtotal, Some(d("0.4164048")));
}

#[test]
fn explicit_zero_setup_is_allowed() {
    let mut input = fixture();
    input.parts[0].setup_hours = Some(Decimal::ZERO);
    input.parts[0].setup_rate_per_hour = None;
    let outcome = calculate(&input);
    assert_eq!(outcome.state, ResultState::Complete, "{:?}", outcome.issues);
    assert_eq!(outcome.setup_total, Some(Decimal::ZERO));
}

#[test]
fn explicit_zero_material_values_are_real_values() {
    let mut input = fixture();
    input.parts[0].material_price_per_kg = Some(Decimal::ZERO);
    input.material_overhead_rate = Some(Decimal::ZERO);
    let outcome = calculate(&input);
    assert_eq!(outcome.state, ResultState::Complete, "{:?}", outcome.issues);
    assert_eq!(outcome.parts[0].allowed_material_cost, Some(Decimal::ZERO));
    assert_eq!(outcome.parts[0].material_overhead, Some(Decimal::ZERO));
    assert_eq!(outcome.parts[0].per_part_subtotal, Some(d("1")));
    assert_eq!(outcome.assembly_subtotal, Some(d("3")));
}

#[test]
fn zero_volume_is_a_real_reviewed_measurement() {
    let mut input = fixture();
    input.parts[0].volume_mm3 = Some(Decimal::ZERO);
    let outcome = calculate(&input);
    assert_eq!(outcome.state, ResultState::Complete, "{:?}", outcome.issues);
    assert_eq!(outcome.parts[0].net_mass_kg, Some(Decimal::ZERO));
    assert_eq!(outcome.parts[0].allowed_material_cost, Some(Decimal::ZERO));
    assert_eq!(outcome.parts[0].per_part_subtotal, Some(d("1")));
}

#[test]
fn invalid_inputs_are_refused() {
    fn refused(code: &str, change: impl FnOnce(&mut CalculationInputs)) {
        let mut input = fixture();
        change(&mut input);
        let outcome = calculate(&input);
        assert_eq!(outcome.state, ResultState::Incomplete, "{code}");
        assert!(has(&outcome, code), "{code}: {:?}", codes(&outcome));
    }

    refused("invalid_cycle_time", |input| input.parts[0].cycle_seconds = Some(d("-1")));
    refused("invalid_volume", |input| input.parts[0].volume_mm3 = Some(d("-1")));
    refused("invalid_density", |input| input.parts[0].density_g_cm3 = Some(Decimal::ZERO));
    refused("invalid_material_price", |input| input.parts[0].material_price_per_kg = Some(d("-0.01")));
    refused("invalid_allowance_rate", |input| input.parts[0].allowance_rate = Some(d("-0.01")));
    refused("invalid_production_rate", |input| input.parts[0].production_rate_per_hour = Some(Decimal::ZERO));
    refused("invalid_cavities", |input| input.parts[0].cavities = Some(0));
    refused("invalid_setup_hours", |input| input.parts[0].setup_hours = Some(d("-0.5")));
    refused("invalid_setup_rate", |input| input.parts[0].setup_rate_per_hour = Some(Decimal::ZERO));
    refused("invalid_overhead_rate", |input| input.material_overhead_rate = Some(d("-0.05")));
}

#[test]
fn quantity_zero_is_named_and_expands_to_a_zero_line_total() {
    let mut input = fixture();
    input.parts[0].quantity = 0;
    let outcome = calculate(&input);
    assert_eq!(outcome.state, ResultState::Incomplete);
    assert_eq!(outcome.parts[0].per_part_subtotal, Some(d("1.1388016")));
    assert_eq!(outcome.parts[0].line_total, Some(Decimal::ZERO));
    assert_eq!(outcome.assembly_subtotal, Some(Decimal::ZERO));
    assert_eq!(outcome.setup_total, Some(d("60")));
    assert_eq!(field_of(&outcome, "invalid_quantity"), Some("parts[0].quantity"));
}

#[test]
fn net_mass_overflow_is_an_issue_not_a_panic() {
    let mut input = fixture();
    input.parts[0].volume_mm3 = Some(Decimal::MAX);
    input.parts[0].density_g_cm3 = Some(d("2"));
    let outcome = calculate(&input);
    assert_eq!(outcome.state, ResultState::Incomplete);
    assert_eq!(outcome.parts[0].net_mass_kg, None);
    assert_eq!(outcome.parts[0].per_part_subtotal, None);
    assert_eq!(outcome.assembly_subtotal, None);
    assert_eq!(outcome.setup_total, Some(d("60")));
    assert_eq!(field_of(&outcome, "arithmetic_overflow"), Some("parts[0].net_mass_kg"));
}

#[test]
fn quantity_expansion_overflow_is_an_issue() {
    let mut input = fixture();
    input.parts[0].volume_mm3 = Some(d("1000000"));
    input.parts[0].density_g_cm3 = Some(Decimal::ONE);
    input.parts[0].material_price_per_kg = Some(d("40000000000000000000000000000"));
    input.parts[0].allowance_rate = Some(Decimal::ZERO);
    input.material_overhead_rate = Some(Decimal::ZERO);
    input.parts[0].cycle_seconds = Some(Decimal::ZERO);
    input.parts[0].cavities = None;
    input.parts[0].production_rate_per_hour = None;
    input.parts[0].quantity = 2;
    let outcome = calculate(&input);
    assert_eq!(outcome.state, ResultState::Incomplete);
    assert_eq!(outcome.parts[0].per_part_subtotal, Some(d("40000000000000000000000000000")));
    assert_eq!(outcome.parts[0].line_total, None);
    assert_eq!(outcome.assembly_subtotal, None);
    assert_eq!(field_of(&outcome, "arithmetic_overflow"), Some("parts[0].line_total"));
}

#[test]
fn assembly_sum_overflow_is_an_issue() {
    let huge = d("40000000000000000000000000000");
    let mut input = fixture();
    input.material_overhead_rate = Some(Decimal::ZERO);
    input.parts = vec![huge_part(Uuid::from_u128(1), huge), huge_part(Uuid::from_u128(2), huge)];
    let outcome = calculate(&input);
    assert_eq!(outcome.state, ResultState::Incomplete);
    assert_eq!(outcome.parts[0].line_total, Some(huge));
    assert_eq!(outcome.parts[1].line_total, Some(huge));
    assert_eq!(outcome.assembly_subtotal, None);
    assert_eq!(field_of(&outcome, "arithmetic_overflow"), Some("assembly_subtotal"));
}

fn huge_part(id: Uuid, price: Decimal) -> ResolvedPartInput {
    ResolvedPartInput {
        part_version_id: id,
        quantity: 1,
        volume_mm3: Some(d("1000000")),
        density_g_cm3: Some(Decimal::ONE),
        material_price_per_kg: Some(price),
        allowance_rate: Some(Decimal::ZERO),
        cycle_seconds: Some(Decimal::ZERO),
        cavities: None,
        production_rate_per_hour: None,
        setup_hours: Some(Decimal::ZERO),
        setup_rate_per_hour: None,
    }
}

#[test]
fn setup_sum_overflow_is_an_issue() {
    let huge = d("40000000000000000000000000000");
    let mut input = fixture();
    input.material_overhead_rate = Some(Decimal::ZERO);
    input.parts = vec![setup_huge_part(Uuid::from_u128(1), huge), setup_huge_part(Uuid::from_u128(2), huge)];
    let outcome = calculate(&input);
    assert_eq!(outcome.state, ResultState::Incomplete);
    assert_eq!(outcome.assembly_subtotal, Some(d("2")));
    assert_eq!(outcome.setup_total, None);
    assert_eq!(field_of(&outcome, "arithmetic_overflow"), Some("setup_total"));
}

fn setup_huge_part(id: Uuid, rate: Decimal) -> ResolvedPartInput {
    ResolvedPartInput {
        part_version_id: id,
        quantity: 1,
        volume_mm3: Some(d("1000000")),
        density_g_cm3: Some(Decimal::ONE),
        material_price_per_kg: Some(Decimal::ONE),
        allowance_rate: Some(Decimal::ZERO),
        cycle_seconds: Some(Decimal::ZERO),
        cavities: None,
        production_rate_per_hour: None,
        setup_hours: Some(Decimal::ONE),
        setup_rate_per_hour: Some(rate),
    }
}

#[test]
fn extreme_values_are_issues_not_panics() {
    let mut input = fixture();
    input.parts[0].volume_mm3 = Some(Decimal::MAX);
    input.parts[0].density_g_cm3 = Some(Decimal::MAX);
    input.parts[0].material_price_per_kg = Some(Decimal::MAX);
    input.parts[0].allowance_rate = Some(Decimal::MAX);
    input.parts[0].cycle_seconds = Some(Decimal::MAX);
    input.parts[0].cavities = Some(u32::MAX);
    input.parts[0].production_rate_per_hour = Some(Decimal::MAX);
    input.parts[0].setup_hours = Some(Decimal::MAX);
    input.parts[0].setup_rate_per_hour = Some(Decimal::MAX);
    let outcome = calculate(&input);
    assert_eq!(outcome.state, ResultState::Incomplete);
    assert!(has(&outcome, "arithmetic_overflow"));
    assert_eq!(outcome.assembly_subtotal, None);
    assert_eq!(outcome.setup_total, None);
}

#[test]
fn unsupported_currency_withholds_every_amount() {
    let mut input = fixture();
    input.currency = "USD".into();
    let outcome = calculate(&input);
    assert_eq!(outcome.state, ResultState::Incomplete);
    assert_eq!(field_of(&outcome, "unsupported_currency"), Some("currency"));
    assert_eq!(outcome.assembly_subtotal, None);
    assert_eq!(outcome.setup_total, None);
    let part = &outcome.parts[0];
    assert_eq!(part.part_version_id, Uuid::nil());
    assert_eq!(part.quantity, 3);
    assert_eq!(part.net_mass_kg, None);
    assert_eq!(part.allowed_material_cost, None);
    assert_eq!(part.material_overhead, None);
    assert_eq!(part.production_cost, None);
    assert_eq!(part.per_part_subtotal, None);
    assert_eq!(part.line_total, None);
    assert!(part.explanations.iter().all(|line| line.amount.is_none()));
}

#[test]
fn manual_reviewed_volume_uses_the_same_formula() {
    // P09: the reviewed manual volume reaches the calculator through `volume_mm3`; provenance
    // (manual, unit, source) stays in the reviewed input, not in this arithmetic.
    let mut input = fixture();
    input.parts[0].volume_mm3 = Some(d("2500"));
    let outcome = calculate(&input);
    assert_eq!(outcome.state, ResultState::Complete, "{:?}", outcome.issues);
    let part = &outcome.parts[0];
    assert_eq!(part.net_mass_kg, Some(d("0.00675")));
    assert_eq!(part.allowed_material_cost, Some(d("0.1377")));
    assert_eq!(part.material_overhead, Some(d("0.006885")));
    assert_eq!(part.production_cost, Some(d("1")));
    assert_eq!(part.per_part_subtotal, Some(d("1.144585")));
    assert_eq!(part.line_total, Some(d("3.433755")));
    assert_eq!(outcome.assembly_subtotal, Some(d("3.433755")));
}

#[test]
fn only_the_approved_lines_are_explained() {
    let outcome = complete_fixture();
    let codes: Vec<&str> = outcome.parts[0].explanations.iter().map(|line| line.code.as_str()).collect();
    assert_eq!(
        codes,
        vec!["material.mass_kg", "material.allowed_cost", "material.overhead", "production.cost", "part.subtotal", "part.line_total", "setup.charge"]
    );
    for line in &outcome.parts[0].explanations {
        assert!(!line.label.is_empty());
        assert!(!line.basis.is_empty());
        assert!(line.amount.is_some());
    }
}

#[test]
fn incomplete_outcome_keeps_unaffected_totals_available() {
    let mut missing_density = fixture();
    missing_density.parts[0].density_g_cm3 = None;
    let outcome = calculate(&missing_density);
    assert_eq!(outcome.state, ResultState::Incomplete);
    assert_eq!(outcome.parts[0].allowed_material_cost, None);
    assert_eq!(outcome.setup_total, Some(d("60")));

    let mut missing_setup = fixture();
    missing_setup.parts[0].setup_hours = None;
    let outcome = calculate(&missing_setup);
    assert_eq!(outcome.state, ResultState::Incomplete);
    assert_eq!(outcome.parts[0].per_part_subtotal, Some(d("1.1388016")));
    assert_eq!(outcome.assembly_subtotal, Some(d("3.4164048")));
    assert_eq!(outcome.setup_total, None);
}

#[test]
fn an_empty_currency_or_part_list_is_incomplete() {
    let mut input = fixture();
    input.currency = String::new();
    input.parts.clear();
    let outcome = calculate(&input);
    assert_eq!(outcome.state, ResultState::Incomplete);
    assert!(has(&outcome, "invalid_currency"));
    assert!(has(&outcome, "no_parts"));
    assert_eq!(outcome.assembly_subtotal, None);
    assert_eq!(outcome.setup_total, None);

    let mut empty = fixture();
    empty.parts.clear();
    let outcome = calculate(&empty);
    assert_eq!(codes(&outcome), vec!["no_parts"]);
    assert_eq!(outcome.state, ResultState::Incomplete);
    // The zero totals of an empty reviewed list stay as they are; the issue makes the result
    // incomplete, so a consumer still cannot read them as a complete cost.
    assert_eq!(outcome.assembly_subtotal, Some(Decimal::ZERO));
    assert_eq!(outcome.setup_total, Some(Decimal::ZERO));
}
