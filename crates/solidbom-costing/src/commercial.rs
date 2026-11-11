//! Pure commercial scenario calculator (package SB-18).
//!
//! The arithmetic version is [`COMMERCIAL_SCENARIO_CALCULATOR_VERSION`]. The published rules are
//! locked in `docs/solidbom/contracts/v2/commercial-scenarios.md`; the portable input and output
//! types live in `solidbom_contract::commercial`. The calculator consumes the SB-17
//! [`ToolingOutcome`] and never repeats the investment, allocation or recovery arithmetic.
//!
//! # Arithmetic in version `solidbom-commercial-scenarios/1`
//!
//! ```text
//! suggested_first_year[p] = C[p] / (1 − t)                        # t is the project EBIT target
//! factor[n]               = (1 − r)^(n−1)                         # n counts production years from SOP
//! suggested_base[p,n]     = suggested_first_year[p] × factor[n]
//! manual_base[p,n]        = manual_price[p] × factor[n]           # blank manual price: unavailable
//! contribution[p,n]       = tooling_contribution[p] × factor[n]   # productivity applies to tooling
//! contribution[p,n]       = tooling_contribution[p]               # productivity is exempt
//! total_price[p,n]        = base_price[p,n] + contribution[p,n]
//! unit_ebit_amount[p,n]   = base_price[p,n] − C[p]
//! unit_ebit_rate[p,n]     = (base_price[p,n] − C[p]) / base_price[p,n]
//! revenue[p,n]            = manual_base[p,n] × forecast[p,n]
//! ebit_amount[p,n]        = (manual_base[p,n] − C[p]) × forecast[p,n]
//! recovery_revenue[p,n]   = contribution[p,n] × forecast[p,n]     # amortized only
//! ```
//!
//! The manufacturing cost `C` stays fixed across the production years: the annual productivity
//! reduction applies only to selling prices, never to the assumed manufacturing cost. Tooling
//! recovery revenue and the separate setup charge stay outside the recurring EBIT. Under the
//! separate tooling treatment the unit price carries no contribution and the whole investment is
//! one separate charge; the amortized treatment carries the contribution and no separate charge.
//!
//! # Missing, zero and invalid values
//!
//! `None` is missing, `Some(Decimal::ZERO)` is an explicit zero; they are never interchangeable. A
//! blank manual price is a deliberately absent value: it has no manual-price EBIT and no quoted
//! revenue, and it produces no issue because the scenario is still a valid suggestion-only view. A
//! missing project rate, recurring cost, forecast, setup total or incomplete tooling answer is a
//! named issue and makes the result incomplete. Checked decimal arithmetic turns overflow into an
//! `arithmetic_overflow` issue and an unavailable amount, never a panic.
//!
//! # Determinism and ordering
//!
//! Products are ordered by product id and each product's years ascending; project years are the
//! ascending union of the product schedules; issues are sorted by `(code, field_path)`. Every
//! business decimal is normalised at the output boundary, so the same [`CommercialInputs`] always
//! produce the same [`CommercialOutcome`] bytes.

use std::collections::{BTreeMap, BTreeSet};

use rust_decimal::Decimal;
use solidbom_contract::{
    CommercialInputs, CommercialIssue, CommercialOutcome, CommercialProductLine, CommercialProjectTotals, CommercialYearLine, CommercialYearTotal,
    CostLine, ResultState, ToolingProductLine, ToolingTreatment,
};
use uuid::Uuid;

/// The arithmetic version this module publishes.
pub const COMMERCIAL_SCENARIO_CALCULATOR_VERSION: &str = "solidbom-commercial-scenarios/1";

/// The calculator version recorded by the service with one calculation.
pub fn commercial_scenario_calculator_version() -> &'static str {
    COMMERCIAL_SCENARIO_CALCULATOR_VERSION
}

/// Calculate one explained commercial answer.
///
/// Missing inputs stay missing: they produce a named issue and an unavailable amount, never a zero.
/// A blank manual price is the one deliberate absence that produces no issue. Arithmetic overflow
/// is a named issue, not a panic. The function applies no rounding beyond `rust_decimal`'s
/// last-representable-digit rounding for a non-terminating division.
pub fn calculate_commercial(inputs: &CommercialInputs) -> CommercialOutcome {
    let mut run = Run::new(inputs);
    let ebit_target = run.ebit_target();
    let productivity = run.productivity();
    let products = run.product_lines(ebit_target, productivity);
    let project = run.project_totals(&products);
    let explanations = project_explanations(inputs, &project);
    let mut outcome = CommercialOutcome {
        state: ResultState::Incomplete,
        ebit_target_rate: ebit_target,
        annual_productivity_rate: productivity,
        productivity_applies_to_tooling: inputs.productivity_applies_to_tooling,
        products,
        project,
        tooling: inputs.tooling.clone(),
        setup_total: inputs.setup_total,
        explanations,
        issues: std::mem::take(&mut run.issues.issues),
    };
    if outcome.issues.is_empty() {
        outcome.state = ResultState::Complete;
    }
    normalize_outcome(&mut outcome);
    outcome.issues.sort_by(|left, right| (left.code.as_str(), left.field_path.as_str()).cmp(&(right.code.as_str(), right.field_path.as_str())));
    outcome
}

// ---------------------------------------------------------------------------
// Issues and checked arithmetic
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Issues {
    issues: Vec<CommercialIssue>,
    seen: BTreeSet<(String, String, Option<Uuid>)>,
}

impl Issues {
    fn push(&mut self, issue: CommercialIssue) {
        if self.seen.insert((issue.code.clone(), issue.field_path.clone(), issue.product_id)) {
            self.issues.push(issue);
        }
    }

    fn named(&mut self, code: &str, field: &str, message: &str) {
        self.push(CommercialIssue::new(code, field, message));
    }
}

fn mul(issues: &mut Issues, left: Decimal, right: Decimal, field: &str) -> Option<Decimal> {
    match left.checked_mul(right) {
        Some(value) => Some(value),
        None => {
            issues.named("arithmetic_overflow", field, "the multiplication exceeds the supported numeric range");
            None
        }
    }
}

fn add(issues: &mut Issues, left: Decimal, right: Decimal, field: &str) -> Option<Decimal> {
    match left.checked_add(right) {
        Some(value) => Some(value),
        None => {
            issues.named("arithmetic_overflow", field, "the addition exceeds the supported numeric range");
            None
        }
    }
}

fn sub(issues: &mut Issues, left: Decimal, right: Decimal, field: &str) -> Option<Decimal> {
    match left.checked_sub(right) {
        Some(value) => Some(value),
        None => {
            issues.named("arithmetic_overflow", field, "the subtraction exceeds the supported numeric range");
            None
        }
    }
}

fn div(issues: &mut Issues, left: Decimal, right: Decimal, field: &str) -> Option<Decimal> {
    match left.checked_div(right) {
        Some(value) => Some(value),
        None => {
            issues.named("arithmetic_overflow", field, "the division exceeds the supported numeric range");
            None
        }
    }
}

/// `(1 − rate)^exponent`, checked. Both the rate and the exponent come from a validated input, so
/// the factor is a positive fraction; the checked multiplication still guards a hostile stored one.
fn factor(issues: &mut Issues, rate: Decimal, exponent: usize, field: &str) -> Option<Decimal> {
    let base = sub(issues, Decimal::ONE, rate, field)?;
    let mut result = Decimal::ONE;
    for _ in 0..exponent {
        result = mul(issues, result, base, field)?;
    }
    Some(result)
}

fn line(code: &str, label: &str, amount: Option<Decimal>, basis: &str) -> CostLine {
    CostLine { code: code.to_string(), label: label.to_string(), amount, basis: basis.to_string() }
}

// ---------------------------------------------------------------------------
// The run
// ---------------------------------------------------------------------------

struct Run<'a> {
    input: &'a CommercialInputs,
    issues: Issues,
    /// The SB-17 product lines by product id, for the allocation and amortized contribution.
    tooling_products: BTreeMap<Uuid, &'a ToolingProductLine>,
}

impl<'a> Run<'a> {
    fn new(input: &'a CommercialInputs) -> Self {
        let tooling_products = input.tooling.products.iter().map(|line| (line.product_id, line)).collect();
        Self { input, issues: Issues::default(), tooling_products }
    }

    fn ebit_target(&mut self) -> Option<Decimal> {
        let rate = self.input.ebit_target_rate;
        self.rate(rate, "ebit_target_rate", "missing_ebit_target", "invalid_ebit_target", "the project EBIT target")
    }

    fn productivity(&mut self) -> Option<Decimal> {
        let rate = self.input.annual_productivity_rate;
        self.rate(rate, "annual_productivity_rate", "missing_productivity", "invalid_productivity", "the annual productivity rate")
    }

    /// Re-check a project rate the stored input could hold: a missing one and a hostile one are both
    /// named. A valid rate is a fraction below one.
    fn rate(&mut self, rate: Option<Decimal>, field: &str, missing: &str, invalid: &str, what: &str) -> Option<Decimal> {
        match rate {
            None => {
                self.issues.named(missing, field, &format!("{what} is missing; the prices that need it stay unavailable"));
                None
            }
            Some(rate) if rate.is_sign_negative() || rate >= Decimal::ONE => {
                self.issues.named(invalid, field, &format!("{what} must be a fraction below 100%"));
                None
            }
            Some(rate) => Some(rate),
        }
    }

    fn product_lines(&mut self, ebit_target: Option<Decimal>, productivity: Option<Decimal>) -> Vec<CommercialProductLine> {
        if self.input.tooling.state == ResultState::Incomplete {
            self.issues.named("tooling_incomplete", "tooling", "the tooling answer this price uses is incomplete");
        }
        if self.input.setup_total.is_none() {
            self.issues.named("missing_setup_charge", "setup_total", "the baseline has no separate setup total");
        }
        let mut lines = Vec::with_capacity(self.input.products.len());
        for index in 0..self.input.products.len() {
            lines.push(self.product_line(index, ebit_target, productivity));
        }
        lines
    }

    fn product_line(&mut self, index: usize, ebit_target: Option<Decimal>, productivity: Option<Decimal>) -> CommercialProductLine {
        let product = self.input.products[index].clone();
        let product_id = product.product_id;
        let field = |suffix: &str| format!("products[{index}].{suffix}");

        let recurring = self.recurring_unit_cost(index, &product);
        let manual = self.manual_price(&product_id);
        let suggested_first = match (recurring, ebit_target) {
            (Some(cost), Some(target)) => match sub(&mut self.issues, Decimal::ONE, target, &field("suggested_first_year_price")) {
                Some(one_minus) if one_minus > Decimal::ZERO => div(&mut self.issues, cost, one_minus, &field("suggested_first_year_price")),
                _ => None,
            },
            _ => None,
        };

        let treatment = self.input.tooling.treatment;
        let tooling_line = self.tooling_products.get(&product_id).copied();
        let initial_contribution = match (treatment, tooling_line) {
            (Some(ToolingTreatment::Amortized), Some(line)) => line.amortized_contribution,
            _ => None,
        };
        let allocated_investment = tooling_line.and_then(|line| line.allocated_investment);

        let mut years = Vec::with_capacity((product.schedule.eop_year - product.schedule.sop_year) as usize + 1);
        for (offset, year) in (product.schedule.sop_year..=product.schedule.eop_year).enumerate() {
            years.push(self.year_line(
                index,
                &product,
                offset,
                year,
                recurring,
                manual,
                suggested_first,
                initial_contribution,
                treatment,
                productivity,
            ));
        }

        let manual_first_year_ebit_amount = match (manual, recurring) {
            (Some(price), Some(cost)) => sub(&mut self.issues, price, cost, &field("manual_first_year_ebit_amount")),
            _ => None,
        };
        let manual_first_year_ebit_rate = match (manual_first_year_ebit_amount, manual) {
            (Some(amount), Some(price)) if price > Decimal::ZERO => div(&mut self.issues, amount, price, &field("manual_first_year_ebit_rate")),
            _ => None,
        };

        let mut explanations = vec![
            line("recurring_unit_cost", "recurring unit cost used by EBIT", recurring, "baseline product line; tooling and setup excluded"),
            line("suggested_first_year_price", "suggested first-year base price C / (1 - target)", suggested_first, "project EBIT target"),
            line("manual_base_price", "entered first-year base price", manual, "manual price; tooling excluded"),
        ];
        if let Some(contribution) = initial_contribution {
            explanations.push(line(
                "tooling_contribution",
                "amortized tooling contribution per finished product",
                Some(contribution),
                "SB-17 recovery answer; productivity applied per year",
            ));
        }
        explanations.push(line("allocated_investment", "product share of the project tooling investment", allocated_investment, "SB-17 allocation"));

        CommercialProductLine {
            product_id,
            product_revision_id: product.product_revision_id,
            schedule: product.schedule,
            manual_price: manual,
            suggested_first_year_price: suggested_first,
            manual_first_year_ebit_amount,
            manual_first_year_ebit_rate,
            tooling_initial_contribution: initial_contribution,
            allocated_investment,
            years,
            explanations,
        }
    }

    fn recurring_unit_cost(&mut self, index: usize, product: &solidbom_contract::CommercialProductInput) -> Option<Decimal> {
        match product.recurring_unit_cost {
            None => {
                self.issues.push(
                    CommercialIssue::new(
                        "missing_recurring_unit_cost",
                        format!("products[{index}].recurring_unit_cost"),
                        "the baseline has no recurring unit cost for this product; the suggested price and EBIT stay unavailable",
                    )
                    .for_product(product.product_id),
                );
                None
            }
            Some(cost) if cost.is_sign_negative() => {
                self.issues.push(
                    CommercialIssue::new(
                        "invalid_recurring_unit_cost",
                        format!("products[{index}].recurring_unit_cost"),
                        "the recurring unit cost cannot be negative",
                    )
                    .for_product(product.product_id),
                );
                None
            }
            Some(cost) => Some(cost),
        }
    }

    /// The scenario's manual price of one product. A price the stored record should not hold is
    /// named and treated as blank instead of being priced.
    fn manual_price(&mut self, product_id: &Uuid) -> Option<Decimal> {
        let index = self.input.manual_prices.iter().position(|price| price.product_id == *product_id)?;
        match self.input.manual_prices[index].price {
            price if price <= Decimal::ZERO => {
                self.issues.push(
                    CommercialIssue::new(
                        "invalid_manual_price",
                        format!("manual_prices[{index}].price"),
                        "a manual base price must be strictly positive; leave it out for a blank price",
                    )
                    .for_product(*product_id),
                );
                None
            }
            price => Some(price),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn year_line(
        &mut self,
        index: usize,
        product: &solidbom_contract::CommercialProductInput,
        offset: usize,
        year: u16,
        recurring: Option<Decimal>,
        manual: Option<Decimal>,
        suggested_first: Option<Decimal>,
        initial_contribution: Option<Decimal>,
        treatment: Option<ToolingTreatment>,
        productivity: Option<Decimal>,
    ) -> CommercialYearLine {
        let field = |suffix: &str| format!("products[{index}].years[{offset}].{suffix}");
        let forecast = self.forecast(index, offset, product, year);
        let factor = productivity.and_then(|rate| factor(&mut self.issues, rate, offset, &field("base_price")));

        let suggested_base_price =
            suggested_first.zip(factor).and_then(|(first, factor)| mul(&mut self.issues, first, factor, &field("suggested_base_price")));
        let manual_base_price = manual.zip(factor).and_then(|(price, factor)| mul(&mut self.issues, price, factor, &field("manual_base_price")));
        let contribution = match treatment {
            // When the scenario applies productivity to the contribution, the contribution series
            // needs the productivity factor; a missing or invalid rate withholds it rather than
            // silently treating the missing rate as an explicit zero.
            Some(ToolingTreatment::Amortized) => match initial_contribution {
                None => None,
                Some(contribution) if self.input.productivity_applies_to_tooling => {
                    factor.and_then(|factor| mul(&mut self.issues, contribution, factor, &field("tooling_contribution")))
                }
                Some(contribution) => Some(contribution),
            },
            _ => None,
        };

        let suggested_total_price = total_price(&mut self.issues, suggested_base_price, contribution, treatment, &field("suggested_total_price"));
        let manual_total_price = total_price(&mut self.issues, manual_base_price, contribution, treatment, &field("manual_total_price"));
        let suggested_unit_ebit_amount = match (suggested_base_price, recurring) {
            (Some(price), Some(cost)) => sub(&mut self.issues, price, cost, &field("suggested_unit_ebit_amount")),
            _ => None,
        };
        let suggested_unit_ebit_rate = self.ebit_rate(suggested_unit_ebit_amount, suggested_base_price, &field("suggested_unit_ebit_rate"));
        let manual_unit_ebit_amount = match (manual_base_price, recurring) {
            (Some(price), Some(cost)) => sub(&mut self.issues, price, cost, &field("manual_unit_ebit_amount")),
            _ => None,
        };
        let manual_unit_ebit_rate = self.ebit_rate(manual_unit_ebit_amount, manual_base_price, &field("manual_unit_ebit_rate"));

        let revenue = self.times_forecast(manual_base_price, forecast, &field("revenue"));
        let ebit_amount = self.times_forecast(manual_unit_ebit_amount, forecast, &field("ebit_amount"));
        let tooling_recovery_revenue = self.times_forecast(contribution, forecast, &field("tooling_recovery_revenue"));

        CommercialYearLine {
            year,
            forecast,
            recurring_unit_cost: recurring,
            suggested_base_price,
            manual_base_price,
            tooling_contribution: contribution,
            suggested_total_price,
            manual_total_price,
            suggested_unit_ebit_amount,
            suggested_unit_ebit_rate,
            manual_unit_ebit_amount,
            manual_unit_ebit_rate,
            revenue,
            ebit_amount,
            tooling_recovery_revenue,
        }
    }

    fn forecast(&mut self, index: usize, offset: usize, product: &solidbom_contract::CommercialProductInput, year: u16) -> Option<u64> {
        match product.forecast(year) {
            Some(quantity) if quantity > 0 => Some(quantity),
            Some(_) => {
                self.issues.push(
                    CommercialIssue::new(
                        "invalid_forecast",
                        format!("products[{index}].years[{offset}].forecast"),
                        "a production year needs a positive forecast; leave the year out instead of zero",
                    )
                    .for_product(product.product_id),
                );
                None
            }
            None => {
                self.issues.push(
                    CommercialIssue::new(
                        "missing_forecast",
                        format!("products[{index}].years[{offset}].forecast"),
                        "the baseline has no forecast for this production year; revenue and EBIT stay unavailable",
                    )
                    .for_product(product.product_id),
                );
                None
            }
        }
    }

    /// `(unit amount) / price`, undefined when the price is zero.
    fn ebit_rate(&mut self, amount: Option<Decimal>, price: Option<Decimal>, field: &str) -> Option<Decimal> {
        match (amount, price) {
            (Some(amount), Some(price)) if price > Decimal::ZERO => div(&mut self.issues, amount, price, field),
            _ => None,
        }
    }

    fn times_forecast(&mut self, unit: Option<Decimal>, forecast: Option<u64>, field: &str) -> Option<Decimal> {
        match (unit, forecast) {
            (Some(unit), Some(forecast)) => mul(&mut self.issues, unit, Decimal::from(forecast), field),
            _ => None,
        }
    }

    // ---- project totals --------------------------------------------------

    fn project_totals(&mut self, products: &[CommercialProductLine]) -> CommercialProjectTotals {
        let years = self.project_years(products);
        let mut totals = Vec::with_capacity(years.len());
        for year in years {
            let revenue = self.sum_year(products, year, |line| line.revenue, "project.revenue");
            let ebit = self.sum_year(products, year, |line| line.ebit_amount, "project.ebit_amount");
            let tooling_revenue = self.sum_year(products, year, |line| line.tooling_recovery_revenue, "project.tooling_recovery_revenue");
            totals.push(CommercialYearTotal { year, revenue, ebit_amount: ebit, tooling_recovery_revenue: tooling_revenue });
        }

        let lifetime_revenue = sum_all(&mut self.issues, totals.iter().map(|total| total.revenue), "project.lifetime_revenue");
        let lifetime_ebit_amount = sum_all(&mut self.issues, totals.iter().map(|total| total.ebit_amount), "project.lifetime_ebit_amount");
        // The commercial view applies the scenario's productivity treatment to the contribution, so
        // its projected recovery follows the adjusted annual series rather than the SB-17 answer's
        // unreduced projection. When the contribution is exempt the two are equal; when it applies
        // the projection and the under/over recovery differ from the unreduced ones. A missing yearly
        // value keeps both totals unavailable. Under the separate treatment neither applies.
        let (tooling_projected_recovery, tooling_under_over_recovery) = if self.input.tooling.treatment == Some(ToolingTreatment::Amortized) {
            let projected =
                sum_all(&mut self.issues, totals.iter().map(|total| total.tooling_recovery_revenue), "project.tooling_projected_recovery");
            let under_over = match (projected, self.input.tooling.project_investment) {
                (Some(projected), Some(investment)) => sub(&mut self.issues, projected, investment, "project.tooling_under_over_recovery"),
                _ => None,
            };
            (projected, under_over)
        } else {
            (None, None)
        };
        let tooling = &self.input.tooling;
        CommercialProjectTotals {
            lifetime_revenue,
            lifetime_ebit_amount,
            years: totals,
            tooling_investment: tooling.project_investment,
            tooling_separate_charge: tooling.project_separate_charge,
            tooling_projected_recovery,
            tooling_under_over_recovery,
        }
    }

    /// The ascending union of every quoted product's schedule years.
    fn project_years(&self, products: &[CommercialProductLine]) -> Vec<u16> {
        let mut years = BTreeSet::new();
        for product in products {
            for year in product.schedule.sop_year..=product.schedule.eop_year {
                years.insert(year);
            }
        }
        years.into_iter().collect()
    }

    /// Sum one value over the products that produce in `year`. A product outside its schedule
    /// contributes nothing; a product inside its schedule with an unavailable value makes the whole
    /// yearly total unavailable, so no partial sum is ever reported.
    fn sum_year<F>(&mut self, products: &[CommercialProductLine], year: u16, value: F, field: &str) -> Option<Decimal>
    where
        F: Fn(&CommercialYearLine) -> Option<Decimal>,
    {
        let mut total: Option<Decimal> = Some(Decimal::ZERO);
        for product in products {
            if year < product.schedule.sop_year || year > product.schedule.eop_year {
                continue;
            }
            let Some(line) = product.years.iter().find(|line| line.year == year) else {
                total = None;
                continue;
            };
            match (total, value(line)) {
                (Some(sum), Some(value)) => total = add(&mut self.issues, sum, value, field),
                _ => total = None,
            }
        }
        total
    }
}

/// A total customer unit price: the base price plus the amortized tooling contribution.
///
/// Under the amortized treatment the contribution is part of the total, so a missing base or a
/// missing contribution leaves the total unavailable. Under the separate treatment the contribution
/// does not apply to the unit price at all, so the total is the base price alone. A blank manual
/// base price always leaves the manual total unavailable, never a contribution-only amount.
fn total_price(
    issues: &mut Issues,
    base: Option<Decimal>,
    contribution: Option<Decimal>,
    treatment: Option<ToolingTreatment>,
    field: &str,
) -> Option<Decimal> {
    let base = base?;
    match treatment {
        Some(ToolingTreatment::Amortized) => match contribution {
            Some(contribution) => add(issues, base, contribution, field),
            None => None,
        },
        _ => Some(base),
    }
}

/// Sum an optional series; an empty sum is an explicit zero and any missing term makes the total
/// unavailable.
fn sum_all(issues: &mut Issues, values: impl IntoIterator<Item = Option<Decimal>>, field: &str) -> Option<Decimal> {
    let mut total: Option<Decimal> = Some(Decimal::ZERO);
    for value in values {
        let value = value?;
        total = match total {
            Some(sum) => add(issues, sum, value, field),
            None => None,
        };
    }
    total
}

fn project_explanations(inputs: &CommercialInputs, project: &CommercialProjectTotals) -> Vec<CostLine> {
    vec![
        line("project_tooling_investment", "project tooling investment, counted once", project.tooling_investment, "SB-17 answer"),
        line(
            "project_tooling_separate_charge",
            "separate tooling charge, outside the unit price",
            project.tooling_separate_charge,
            "separate treatment only",
        ),
        line("project_setup_total", "separate setup charge, outside recurring EBIT", inputs.setup_total, "baseline setup total"),
        line("lifetime_revenue", "lifetime quoted revenue, base prices only", project.lifetime_revenue, "manual base price × forecasts"),
        line(
            "lifetime_ebit_amount",
            "lifetime recurring EBIT, tooling and setup excluded",
            project.lifetime_ebit_amount,
            "quoted revenue − recurring cost",
        ),
    ]
}

// ---------------------------------------------------------------------------
// Output normalisation
// ---------------------------------------------------------------------------

fn norm(value: Decimal) -> Decimal {
    value.normalize()
}

fn norm_opt(value: Option<Decimal>) -> Option<Decimal> {
    value.map(norm)
}

fn normalize_outcome(outcome: &mut CommercialOutcome) {
    outcome.ebit_target_rate = norm_opt(outcome.ebit_target_rate);
    outcome.annual_productivity_rate = norm_opt(outcome.annual_productivity_rate);
    outcome.setup_total = norm_opt(outcome.setup_total);
    for product in &mut outcome.products {
        product.manual_price = norm_opt(product.manual_price);
        product.suggested_first_year_price = norm_opt(product.suggested_first_year_price);
        product.manual_first_year_ebit_amount = norm_opt(product.manual_first_year_ebit_amount);
        product.manual_first_year_ebit_rate = norm_opt(product.manual_first_year_ebit_rate);
        product.tooling_initial_contribution = norm_opt(product.tooling_initial_contribution);
        product.allocated_investment = norm_opt(product.allocated_investment);
        for year in &mut product.years {
            year.recurring_unit_cost = norm_opt(year.recurring_unit_cost);
            year.suggested_base_price = norm_opt(year.suggested_base_price);
            year.manual_base_price = norm_opt(year.manual_base_price);
            year.tooling_contribution = norm_opt(year.tooling_contribution);
            year.suggested_total_price = norm_opt(year.suggested_total_price);
            year.manual_total_price = norm_opt(year.manual_total_price);
            year.suggested_unit_ebit_amount = norm_opt(year.suggested_unit_ebit_amount);
            year.suggested_unit_ebit_rate = norm_opt(year.suggested_unit_ebit_rate);
            year.manual_unit_ebit_amount = norm_opt(year.manual_unit_ebit_amount);
            year.manual_unit_ebit_rate = norm_opt(year.manual_unit_ebit_rate);
            year.revenue = norm_opt(year.revenue);
            year.ebit_amount = norm_opt(year.ebit_amount);
            year.tooling_recovery_revenue = norm_opt(year.tooling_recovery_revenue);
        }
        for explanation in &mut product.explanations {
            explanation.amount = norm_opt(explanation.amount);
        }
    }
    for year in &mut outcome.project.years {
        year.revenue = norm_opt(year.revenue);
        year.ebit_amount = norm_opt(year.ebit_amount);
        year.tooling_recovery_revenue = norm_opt(year.tooling_recovery_revenue);
    }
    outcome.project.lifetime_revenue = norm_opt(outcome.project.lifetime_revenue);
    outcome.project.lifetime_ebit_amount = norm_opt(outcome.project.lifetime_ebit_amount);
    outcome.project.tooling_investment = norm_opt(outcome.project.tooling_investment);
    outcome.project.tooling_separate_charge = norm_opt(outcome.project.tooling_separate_charge);
    outcome.project.tooling_projected_recovery = norm_opt(outcome.project.tooling_projected_recovery);
    outcome.project.tooling_under_over_recovery = norm_opt(outcome.project.tooling_under_over_recovery);
    for explanation in &mut outcome.explanations {
        explanation.amount = norm_opt(explanation.amount);
    }
}

#[cfg(test)]
#[path = "commercial_tests.rs"]
mod commercial_tests;
