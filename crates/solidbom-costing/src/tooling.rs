//! Pure tooling investment, allocation and recovery calculator (package SB-17).
//!
//! The arithmetic version is [`TOOLING_RECOVERY_CALCULATOR_VERSION`]. The published rules are locked
//! in `docs/solidbom/contracts/v2/tooling-investment-and-recovery.md`; the portable input and output
//! types live in `solidbom_contract::tooling`.
//!
//! # Arithmetic in version `solidbom-tooling-recovery/1`
//!
//! ```text
//! source_investment[t] = tool_count[t] × cost_per_tool[t]              # entry currency
//! investment[t]        = source_investment[t] × saved rate             # project currency
//! project_investment   = Σ over entries of investment[t]              # counted once
//!
//! D[t,p]               = product_lifetime[p] × q[t,p]                  # q is the expanded owner quantity
//! combined_demand[t]   = Σ over quoted p of D[t,p]
//! allocation_share[t,p]= D[t,p] / combined_demand[t]
//! product_allocated_investment[p] = Σ over tools of allocation_share[t,p] × investment[t]
//!
//! contribution_per_owner_unit[t] = investment[t] / recovery_quantity[t]
//! product_contribution[t,p]      = contribution_per_owner_unit[t] × q[t,p]
//! recovered[t,y]                 = contribution_per_owner_unit[t] × owner_annual[t,y]
//! projected_recovery[t]          = contribution_per_owner_unit[t] × owner_lifetime[t]
//! under_over_recovery[t]         = projected_recovery[t] − investment[t]
//! separate_charge[t]             = investment[t]
//! ```
//!
//! The recovery quantity is the owner's combined lifetime demand by default, or a manually entered
//! strictly positive quantity selected by the scenario. The contribution is the tool's whole
//! investment divided by that quantity; it stays constant in every production year and reaching the
//! recovery quantity never stops, caps or reduces it. One evaluation selects one treatment, so the
//! same investment is never recovered through both a full separate charge and full amortization.
//!
//! # Missing, zero and invalid values
//!
//! `None` is missing, `Some(Decimal::ZERO)` is an explicit zero; they are never interchangeable. A
//! missing value makes its affected amounts unavailable and produces a named issue whose code and
//! field path are stable contract names. Checked decimal arithmetic turns overflow into an
//! `arithmetic_overflow` issue and an unavailable amount, never a panic. A missing product lifetime
//! demand leaves a demanding owner's allocation and projection unavailable; it never becomes zero.
//! A combined demand of zero leaves the shares unavailable without inventing a percentage.
//!
//! # Determinism and ordering
//!
//! Tools are ordered by tool id, allocation and product lines by product id and annual series by
//! ascending year; issues are sorted by `(code, field_path)`. Every business decimal is normalised
//! at the output boundary, so the same [`ToolingInputs`] always produce the same [`ToolingOutcome`]
//! bytes. No hash-map iteration order, clock, locale or environment is read.

use std::collections::{BTreeMap, BTreeSet};

use rust_decimal::Decimal;
use solidbom_contract::{
    AnnualForecast, CostLine, ResultState, ToolingAllocationLine, ToolingAnnualRecovery, ToolingEntry, ToolingInputs, ToolingIssue, ToolingOutcome,
    ToolingOwner, ToolingProductDemand, ToolingProductLine, ToolingRecoveryBasis, ToolingToolLine, ToolingTreatment,
};
use uuid::Uuid;

/// The arithmetic version this module publishes.
pub const TOOLING_RECOVERY_CALCULATOR_VERSION: &str = "solidbom-tooling-recovery/1";

/// The calculator version recorded by the service with one evaluation.
pub fn tooling_recovery_calculator_version() -> &'static str {
    TOOLING_RECOVERY_CALCULATOR_VERSION
}

/// Calculate one explained tooling investment, allocation and recovery answer.
///
/// Missing inputs stay missing: they produce a named issue and an unavailable amount, never a zero.
/// Arithmetic overflow is a named issue, not a panic. The function performs no rounding beyond
/// `rust_decimal`'s last-representable-digit rounding for a non-terminating division.
pub fn calculate_tooling(inputs: &ToolingInputs) -> ToolingOutcome {
    let mut run = Run::new(inputs);
    let tools = run.tool_lines();
    let products = run.product_lines(&tools);
    let totals = run.project_totals(&tools);
    let explanations = run.project_explanations(&totals);
    let treatment = run.treatment;
    let mut outcome = ToolingOutcome {
        state: ResultState::Incomplete,
        treatment,
        tools: tools.into_iter().map(|tool| tool.line).collect(),
        products,
        project_investment: totals.investment,
        project_amortized_recovered: totals.amortized_recovered,
        project_under_over_recovery: totals.under_over_recovery,
        project_separate_charge: totals.separate_charge,
        annual_recovery: totals.annual,
        explanations,
        issues: std::mem::take(&mut run.issues.issues),
    };
    if is_complete(&outcome, treatment) {
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
    issues: Vec<ToolingIssue>,
    seen: BTreeSet<(String, String, Option<Uuid>, Option<Uuid>)>,
}

impl Issues {
    fn push(&mut self, issue: ToolingIssue) {
        if self.seen.insert((issue.code.clone(), issue.field_path.clone(), issue.tool_id, issue.product_id)) {
            self.issues.push(issue);
        }
    }

    fn complex(&mut self, code: &str, field: &str, message: &str) {
        self.push(ToolingIssue::new(code, field, message));
    }
}

fn mul(issues: &mut Issues, left: Decimal, right: Decimal, field: &str) -> Option<Decimal> {
    match left.checked_mul(right) {
        Some(value) => Some(value),
        None => {
            issues.complex("arithmetic_overflow", field, "the multiplication exceeds the supported numeric range");
            None
        }
    }
}

fn add(issues: &mut Issues, left: Decimal, right: Decimal, field: &str) -> Option<Decimal> {
    match left.checked_add(right) {
        Some(value) => Some(value),
        None => {
            issues.complex("arithmetic_overflow", field, "the addition exceeds the supported numeric range");
            None
        }
    }
}

fn sub(issues: &mut Issues, left: Decimal, right: Decimal, field: &str) -> Option<Decimal> {
    match left.checked_sub(right) {
        Some(value) => Some(value),
        None => {
            issues.complex("arithmetic_overflow", field, "the subtraction exceeds the supported numeric range");
            None
        }
    }
}

fn add_u64(issues: &mut Issues, left: u64, right: u64, field: &str) -> Option<u64> {
    match left.checked_add(right) {
        Some(value) => Some(value),
        None => {
            issues.complex("arithmetic_overflow", field, "the quantity exceeds the supported numeric range");
            None
        }
    }
}

fn mul_u64(issues: &mut Issues, quantity: u64, count: u64, field: &str) -> Option<u64> {
    match quantity.checked_mul(count) {
        Some(value) => Some(value),
        None => {
            issues.complex("arithmetic_overflow", field, "the quantity exceeds the supported numeric range");
            None
        }
    }
}

/// Sum decimal contributions; an empty sum is an explicit zero and any missing term makes the total
/// unavailable. No partial total is ever reported.
fn sum_decimals(issues: &mut Issues, values: impl IntoIterator<Item = Option<Decimal>>, field: &str) -> Option<Decimal> {
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

fn line(code: &str, label: &str, amount: Option<Decimal>, basis: &str) -> CostLine {
    CostLine { code: code.to_string(), label: label.to_string(), amount, basis: basis.to_string() }
}

// ---------------------------------------------------------------------------
// The run
// ---------------------------------------------------------------------------

/// One quoted product's demand, ordered for deterministic allocation.
#[derive(Clone)]
struct ProductView {
    product_id: Uuid,
    product_revision_id: Option<Uuid>,
    lifetime_demand: Option<u64>,
    forecasts: Vec<AnnualForecast>,
    quantities: BTreeMap<Uuid, u64>,
}

impl ProductView {
    fn of(product: &ToolingProductDemand) -> Self {
        Self {
            product_id: product.product_id,
            product_revision_id: product.product_revision_id,
            lifetime_demand: product.lifetime_demand,
            forecasts: product.forecasts.clone(),
            quantities: product.part_quantities.iter().map(|part| (part.part_version_id, part.quantity)).collect(),
        }
    }

    fn quantity_of(&self, part_version_id: Uuid) -> u64 {
        self.quantities.get(&part_version_id).copied().unwrap_or(0)
    }
}

/// One tool's computed line and whether its annual recovery series is fully available.
struct ToolComputed {
    line: ToolingToolLine,
    annual_available: bool,
}

/// The project-level totals of one treatment.
struct Totals {
    investment: Option<Decimal>,
    amortized_recovered: Option<Decimal>,
    under_over_recovery: Option<Decimal>,
    separate_charge: Option<Decimal>,
    annual: Vec<ToolingAnnualRecovery>,
}

/// One allocation row before the combined demand and the project investment are known.
struct AllocationDraft {
    product_id: Uuid,
    product_lifetime_demand: Option<u64>,
    expanded_quantity: u64,
    product_owner_demand: Option<u64>,
}

struct Run<'a> {
    input: &'a ToolingInputs,
    issues: Issues,
    treatment: Option<ToolingTreatment>,
    entries: Vec<ToolingEntry>,
    products: Vec<ProductView>,
}

impl<'a> Run<'a> {
    fn new(input: &'a ToolingInputs) -> Self {
        let mut entries = input.tooling.entries.clone();
        entries.sort_by_key(|entry| entry.tool_id);
        let mut products: Vec<ProductView> = input.demand.products.iter().map(ProductView::of).collect();
        products.sort_by_key(|product| product.product_id);
        let treatment = input.recovery.treatment.or(input.tooling.default_treatment);
        let mut issues = Issues::default();
        if !entries.is_empty() && treatment.is_none() {
            issues.complex("missing_treatment", "recovery.treatment", "neither the scenario nor the project selects a tooling treatment");
        }
        // A missing product lifetime is a property of the demand input: report it once per quoted
        // product that actually demands a tool, never once per tool.
        for (index, product) in products.iter().enumerate() {
            if product.lifetime_demand.is_none() && entries.iter().any(|entry| owner_quantity(&entry.owner, product) > 0) {
                issues.push(
                    ToolingIssue::new(
                        "missing_product_demand",
                        format!("demand.products[{index}].lifetime_demand"),
                        "the baseline has no lifetime demand for a quoted product that demands a tool; allocation and projection stay unavailable",
                    )
                    .for_product(product.product_id),
                );
            }
        }
        Self { input, issues, treatment, entries, products }
    }

    // ---- tool lines ------------------------------------------------------

    fn tool_lines(&mut self) -> Vec<ToolComputed> {
        let entries = self.entries.clone();
        let products = self.products.clone();
        let mut lines = Vec::with_capacity(entries.len());
        for (index, entry) in entries.iter().enumerate() {
            lines.push(self.tool_line(index, entry, &products));
        }
        lines
    }

    fn tool_line(&mut self, index: usize, entry: &ToolingEntry, products: &[ProductView]) -> ToolComputed {
        let tool_id = entry.tool_id;
        let field = |suffix: &str| format!("tools[{index}].{suffix}");

        // The entry is the sole statement of the investment; re-check its business values instead
        // of assuming the stored record was validated.
        let source_investment = if entry.tool_count == 0 {
            self.issues.push(ToolingIssue::new("invalid_tool_count", field("tool_count"), "a tool entry needs at least one tool").for_tool(tool_id));
            None
        } else if entry.cost_per_tool.is_sign_negative() {
            self.issues.push(ToolingIssue::new("invalid_cost_per_tool", field("cost_per_tool"), "a tool cost cannot be negative").for_tool(tool_id));
            None
        } else {
            mul(&mut self.issues, Decimal::from(entry.tool_count), entry.cost_per_tool, &field("source_investment"))
        };

        // Same currency is the identity; a different one needs a saved, strictly positive rate.
        let (fx_rate, investment) = match source_investment {
            None => (None, None),
            Some(source) if entry.currency == self.input.project_currency => (None, Some(source)),
            Some(source) => match self.input.exchange_rates.iter().find(|rate| rate.source_currency == entry.currency) {
                None => {
                    self.issues.push(
                        ToolingIssue::new(
                            "missing_exchange_rate",
                            "exchange_rates",
                            format!("no saved rate converts {} to the project currency {}", entry.currency, self.input.project_currency),
                        )
                        .for_tool(tool_id),
                    );
                    (None, None)
                }
                Some(rate) if rate.project_units_per_source_unit <= Decimal::ZERO => {
                    self.issues.push(
                        ToolingIssue::new("invalid_exchange_rate", "exchange_rates", "a saved exchange rate must be strictly positive")
                            .for_tool(tool_id),
                    );
                    (None, None)
                }
                Some(rate) => (
                    Some(rate.project_units_per_source_unit),
                    mul(&mut self.issues, source, rate.project_units_per_source_unit, &field("investment")),
                ),
            },
        };

        // Allocation: one row per quoted product that actually uses the owner; a product that does
        // not use it has a zero share and is omitted.
        let mut drafts = Vec::new();
        let mut combined_demand: Option<u64> = Some(0);
        for product in products {
            let quantity = owner_quantity(&entry.owner, product);
            if quantity == 0 {
                continue;
            }
            let owner_demand = match product.lifetime_demand {
                Some(lifetime) => mul_u64(&mut self.issues, lifetime, quantity, &field("allocation")),
                None => None,
            };
            combined_demand = match (combined_demand, owner_demand) {
                (Some(total), Some(owner_demand)) => add_u64(&mut self.issues, total, owner_demand, &field("allocation")),
                _ => None,
            };
            drafts.push(AllocationDraft {
                product_id: product.product_id,
                product_lifetime_demand: product.lifetime_demand,
                expanded_quantity: quantity,
                product_owner_demand: owner_demand,
            });
        }
        if combined_demand == Some(0) {
            self.issues.push(
                ToolingIssue::new(
                    "zero_combined_demand",
                    field("allocation"),
                    "the owner is demanded by no quoted product; no allocation share or lifetime recovery quantity is invented",
                )
                .for_tool(tool_id),
            );
        }

        // The selected recovery basis, and the positive quantity the investment is divided by.
        let basis = self.input.recovery.basis_for(&tool_id);
        let basis_index = self.input.recovery.recovery.iter().position(|choice| choice.tool_id == tool_id);
        let recovery_quantity = match basis {
            ToolingRecoveryBasis::LifetimeDemand => match combined_demand {
                Some(0) => None,
                Some(total) => Some(Decimal::from(total)),
                None => None,
            },
            ToolingRecoveryBasis::ManualQuantity { quantity } => match quantity {
                Some(quantity) if quantity <= Decimal::ZERO => {
                    let path = basis_path(basis_index);
                    self.issues.push(
                        ToolingIssue::new("invalid_recovery_quantity", path, "a manual recovery quantity must be strictly positive")
                            .for_tool(tool_id),
                    );
                    None
                }
                Some(quantity) => Some(quantity),
                None => {
                    let path = basis_path(basis_index);
                    self.issues.push(
                        ToolingIssue::new("missing_recovery_quantity", path, "the manual recovery basis has no entered quantity").for_tool(tool_id),
                    );
                    None
                }
            },
        };

        let amortized = self.treatment == Some(ToolingTreatment::Amortized);
        let separate = self.treatment == Some(ToolingTreatment::Separate);

        // The contribution is the whole investment divided by the positive recovery quantity.
        let contribution_per_owner_unit = if amortized {
            match (investment, recovery_quantity) {
                (Some(investment), Some(recovery_quantity)) => match investment.checked_div(recovery_quantity) {
                    Some(contribution) => Some(contribution),
                    None => {
                        self.issues.complex(
                            "arithmetic_overflow",
                            &field("contribution_per_owner_unit"),
                            "the division exceeds the supported numeric range",
                        );
                        None
                    }
                },
                _ => None,
            }
        } else {
            None
        };

        // Per-product shares and contributions; unavailable when the combined demand is not usable.
        let mut allocation = Vec::with_capacity(drafts.len());
        match combined_demand {
            Some(total) if total > 0 => {
                for draft in &drafts {
                    let share = match draft.product_owner_demand {
                        Some(owner_demand) => match Decimal::from(owner_demand).checked_div(Decimal::from(total)) {
                            Some(share) => Some(share),
                            None => {
                                self.issues.complex(
                                    "arithmetic_overflow",
                                    &field("allocation"),
                                    "the allocation share exceeds the supported numeric range",
                                );
                                None
                            }
                        },
                        None => None,
                    };
                    let allocated_investment = match (share, investment) {
                        (Some(share), Some(investment)) => mul(&mut self.issues, share, investment, &field("allocation")),
                        _ => None,
                    };
                    let product_contribution = match contribution_per_owner_unit {
                        Some(contribution) => mul(&mut self.issues, contribution, Decimal::from(draft.expanded_quantity), &field("allocation")),
                        None => None,
                    };
                    allocation.push(ToolingAllocationLine {
                        product_id: draft.product_id,
                        product_lifetime_demand: draft.product_lifetime_demand,
                        expanded_quantity: draft.expanded_quantity,
                        product_owner_demand: draft.product_owner_demand,
                        allocation_share: share,
                        allocated_investment,
                        product_contribution,
                    });
                }
            }
            _ => {
                for draft in &drafts {
                    allocation.push(ToolingAllocationLine {
                        product_id: draft.product_id,
                        product_lifetime_demand: draft.product_lifetime_demand,
                        expanded_quantity: draft.expanded_quantity,
                        product_owner_demand: draft.product_owner_demand,
                        allocation_share: None,
                        allocated_investment: None,
                        product_contribution: None,
                    });
                }
            }
        }

        // The annual projected recovery: only an amortized evaluation produces it, and only when the
        // owner's combined lifetime demand is available.
        let mut annual_recovery = Vec::new();
        let mut annual_available = false;
        if amortized && combined_demand.is_some() && contribution_per_owner_unit.is_some() {
            annual_recovery = self.owner_annual(entry, products, contribution_per_owner_unit, index);
            annual_available = annual_recovery.iter().all(|year| year.recovered.is_some());
            if !annual_available {
                annual_recovery.clear();
            }
        }

        let projected_recovery = if amortized {
            match (contribution_per_owner_unit, combined_demand) {
                (Some(contribution), Some(combined)) => mul(&mut self.issues, contribution, Decimal::from(combined), &field("projected_recovery")),
                _ => None,
            }
        } else {
            None
        };
        let under_over_recovery = if amortized {
            match (projected_recovery, investment) {
                (Some(projected), Some(investment)) => sub(&mut self.issues, projected, investment, &field("under_over_recovery")),
                _ => None,
            }
        } else {
            None
        };
        let separate_charge = if separate { investment } else { None };

        let mut explanations = Vec::new();
        explanations.push(line(
            "tool.cost_per_tool",
            "cost per tool",
            Some(entry.cost_per_tool),
            "as stored, in the entry currency; an explicit zero is a real value",
        ));
        if let Some(source_investment) = source_investment {
            explanations.push(line(
                "tool.source_investment",
                "source investment",
                Some(source_investment),
                "tool count × cost per tool in the entry currency, counted once",
            ));
        }
        if let Some(fx_rate) = fx_rate {
            explanations.push(line("tool.fx", "exchange rate", Some(fx_rate), "project-currency units per source-currency unit"));
        }
        if let Some(investment) = investment {
            explanations.push(line(
                "tool.investment",
                "project investment",
                Some(investment),
                if entry.currency == self.input.project_currency {
                    "same currency: the source investment is the project investment, counted once"
                } else {
                    "source investment × saved rate, counted once"
                },
            ));
        }
        if let Some(combined) = combined_demand {
            explanations.push(line(
                "tool.combined_demand",
                "owner combined lifetime demand",
                Some(Decimal::from(combined)),
                "Σ quoted product lifetime demand × expanded owner quantity",
            ));
        }
        if let Some(recovery_quantity) = recovery_quantity {
            explanations.push(line(
                "tool.recovery_quantity",
                "recovery quantity",
                Some(recovery_quantity),
                match basis {
                    ToolingRecoveryBasis::LifetimeDemand => "the owner's combined lifetime demand",
                    ToolingRecoveryBasis::ManualQuantity { .. } => "a manually entered positive recovery quantity",
                },
            ));
        }
        if let Some(contribution) = contribution_per_owner_unit {
            explanations.push(line(
                "tool.contribution",
                "contribution per owner unit",
                Some(contribution),
                "investment ÷ recovery quantity, constant in every production year and never capped",
            ));
        }
        for allocation_line in &allocation {
            if let Some(share) = allocation_line.allocation_share {
                explanations.push(line(
                    "tool.allocation_share",
                    "product allocation share",
                    Some(share),
                    "product lifetime demand ÷ the owner's combined lifetime demand",
                ));
            }
            if let Some(allocated) = allocation_line.allocated_investment {
                explanations.push(line(
                    "tool.allocation",
                    "product allocated investment",
                    Some(allocated),
                    "allocation share × this tool's single investment; allocation never multiplies it",
                ));
            }
            if let Some(contribution) = allocation_line.product_contribution {
                explanations.push(line(
                    "tool.product_contribution",
                    "product amortized contribution",
                    Some(contribution),
                    "contribution per owner unit × the expanded owner quantity in one finished product",
                ));
            }
        }
        if annual_available {
            for year in &annual_recovery {
                if let Some(recovered) = year.recovered {
                    explanations.push(line(
                        &format!("tool.annual_recovery.{}", year.year),
                        "annual recovered amount",
                        Some(recovered),
                        "contribution per owner unit × the owner's annual demand",
                    ));
                }
            }
        }
        if let Some(projected) = projected_recovery {
            explanations.push(line(
                "tool.projected_recovery",
                "projected lifetime recovery",
                Some(projected),
                "contribution per owner unit × the owner's combined lifetime demand",
            ));
        }
        if let Some(under_over) = under_over_recovery {
            explanations.push(line(
                "tool.under_over_recovery",
                "under/over recovery",
                Some(under_over),
                "projected recovery − investment; positive is over-recovery and negative is under-recovery",
            ));
        }
        if let Some(charge) = separate_charge {
            explanations.push(line(
                "tool.separate_charge",
                "separate charge",
                Some(charge),
                "the whole investment as one separate charge, outside the unit price and the amortized contribution",
            ));
        }

        ToolComputed {
            line: ToolingToolLine {
                tool_id,
                owner: entry.owner,
                tool_count: entry.tool_count,
                cost_per_tool: entry.cost_per_tool,
                currency: entry.currency.clone(),
                source_investment,
                fx_rate,
                investment,
                recovery_quantity,
                contribution_per_owner_unit,
                allocation,
                annual_recovery,
                projected_recovery,
                under_over_recovery,
                separate_charge,
                explanations,
            },
            annual_available,
        }
    }

    /// The owner's annual demand summed over the quoted products that use it, one row per calendar
    /// year with a forecast for such a product.
    fn owner_annual(
        &mut self,
        entry: &ToolingEntry,
        products: &[ProductView],
        contribution: Option<Decimal>,
        index: usize,
    ) -> Vec<ToolingAnnualRecovery> {
        let field = |suffix: &str| format!("tools[{index}].{suffix}");
        let mut years: BTreeSet<u16> = BTreeSet::new();
        for product in products {
            if owner_quantity(&entry.owner, product) == 0 {
                continue;
            }
            for forecast in &product.forecasts {
                years.insert(forecast.year);
            }
        }
        let mut annual = Vec::with_capacity(years.len());
        for year in years {
            let mut owner_annual: Option<u64> = Some(0);
            for product in products {
                let quantity = owner_quantity(&entry.owner, product);
                if quantity == 0 {
                    continue;
                }
                let Some(forecast) = product.forecasts.iter().find(|forecast| forecast.year == year) else {
                    continue;
                };
                let increment = mul_u64(&mut self.issues, forecast.quantity, quantity, &field(&format!("annual_recovery.{year}")));
                owner_annual = match (owner_annual, increment) {
                    (Some(total), Some(increment)) => add_u64(&mut self.issues, total, increment, &field(&format!("annual_recovery.{year}"))),
                    _ => None,
                };
            }
            let recovered = match (owner_annual, contribution) {
                (Some(owner_annual), Some(contribution)) => {
                    mul(&mut self.issues, contribution, Decimal::from(owner_annual), &field(&format!("annual_recovery.{year}")))
                }
                _ => None,
            };
            annual.push(ToolingAnnualRecovery { year, recovered });
        }
        annual
    }

    // ---- product lines and project totals --------------------------------

    fn product_lines(&mut self, tools: &[ToolComputed]) -> Vec<ToolingProductLine> {
        let products = self.products.clone();
        let amortized = self.treatment == Some(ToolingTreatment::Amortized);
        let mut lines = Vec::with_capacity(products.len());
        for (index, product) in products.iter().enumerate() {
            let field = |suffix: &str| format!("products[{index}].{suffix}");
            let mut allocated_investment: Option<Decimal> = Some(Decimal::ZERO);
            let mut amortized_contribution: Option<Decimal> = Some(Decimal::ZERO);
            for tool in tools {
                let Some(allocation) = tool.line.allocation.iter().find(|line| line.product_id == product.product_id) else {
                    continue;
                };
                allocated_investment = match (allocated_investment, allocation.allocated_investment) {
                    (Some(total), Some(amount)) => add(&mut self.issues, total, amount, &field("allocated_investment")),
                    _ => None,
                };
                if amortized {
                    amortized_contribution = match (amortized_contribution, allocation.product_contribution) {
                        (Some(total), Some(amount)) => add(&mut self.issues, total, amount, &field("amortized_contribution")),
                        _ => None,
                    };
                }
            }
            if !amortized {
                amortized_contribution = None;
            }
            let mut explanations = Vec::new();
            if let Some(lifetime_demand) = product.lifetime_demand {
                explanations.push(line(
                    "product.lifetime_demand",
                    "lifetime demand",
                    Some(Decimal::from(lifetime_demand)),
                    "the baseline product line's summed positive forecast",
                ));
            }
            if let Some(allocated) = allocated_investment {
                explanations.push(line(
                    "product.allocated_investment",
                    "allocated investment",
                    Some(allocated),
                    "Σ this product's tool allocation shares × the tool investments; the share of the single project investment",
                ));
            }
            if let Some(contribution) = amortized_contribution {
                explanations.push(line(
                    "product.amortized_contribution",
                    "amortized contribution per finished product",
                    Some(contribution),
                    "Σ tool contribution per owner unit × the expanded owner quantity in this finished product",
                ));
            }
            lines.push(ToolingProductLine {
                product_id: product.product_id,
                product_revision_id: product.product_revision_id,
                lifetime_demand: product.lifetime_demand,
                allocated_investment,
                amortized_contribution,
                explanations,
            });
        }
        lines
    }

    fn project_totals(&mut self, tools: &[ToolComputed]) -> Totals {
        let investment = sum_decimals(&mut self.issues, tools.iter().map(|tool| tool.line.investment), "project_investment");
        match self.treatment {
            Some(ToolingTreatment::Amortized) => {
                let amortized_recovered =
                    sum_decimals(&mut self.issues, tools.iter().map(|tool| tool.line.projected_recovery), "project_amortized_recovered");
                let under_over_recovery =
                    sum_decimals(&mut self.issues, tools.iter().map(|tool| tool.line.under_over_recovery), "project_under_over_recovery");
                let annual = if tools.iter().all(|tool| tool.annual_available) { self.merged_annual(tools) } else { Vec::new() };
                Totals { investment, amortized_recovered, under_over_recovery, separate_charge: None, annual }
            }
            Some(ToolingTreatment::Separate) => {
                let separate_charge = sum_decimals(&mut self.issues, tools.iter().map(|tool| tool.line.separate_charge), "project_separate_charge");
                Totals { investment, amortized_recovered: None, under_over_recovery: None, separate_charge, annual: Vec::new() }
            }
            None => Totals { investment, amortized_recovered: None, under_over_recovery: None, separate_charge: None, annual: Vec::new() },
        }
    }

    /// The summed per-year recovery of every tool, presented only when every tool's series is
    /// available. A tool without a row for a year recovers nothing in that year.
    fn merged_annual(&mut self, tools: &[ToolComputed]) -> Vec<ToolingAnnualRecovery> {
        let mut years: BTreeSet<u16> = BTreeSet::new();
        for tool in tools {
            for year in &tool.line.annual_recovery {
                years.insert(year.year);
            }
        }
        let mut annual = Vec::with_capacity(years.len());
        for year in years {
            let field = format!("annual_recovery.{year}");
            let mut total: Option<Decimal> = Some(Decimal::ZERO);
            for tool in tools {
                let recovered = tool.line.annual_recovery.iter().find(|row| row.year == year).map_or(Some(Decimal::ZERO), |row| row.recovered);
                total = match (total, recovered) {
                    (Some(current), Some(amount)) => add(&mut self.issues, current, amount, &field),
                    _ => None,
                };
            }
            annual.push(ToolingAnnualRecovery { year, recovered: total });
        }
        annual
    }

    fn project_explanations(&mut self, totals: &Totals) -> Vec<CostLine> {
        let mut lines = Vec::new();
        if let Some(investment) = totals.investment {
            lines.push(line(
                "project.investment",
                "project investment",
                Some(investment),
                "Σ tool investments, each entry counted once and never multiplied by demand",
            ));
        }
        match self.treatment {
            Some(ToolingTreatment::Amortized) => {
                if let Some(recovered) = totals.amortized_recovered {
                    lines.push(line(
                        "project.projected_recovery",
                        "project projected recovery",
                        Some(recovered),
                        "Σ tool projected lifetime recovery",
                    ));
                }
                if let Some(under_over) = totals.under_over_recovery {
                    lines.push(line(
                        "project.under_over_recovery",
                        "project under/over recovery",
                        Some(under_over),
                        "Σ projected recovery − Σ investment; positive is over-recovery and negative is under-recovery",
                    ));
                }
                for year in &totals.annual {
                    if let Some(recovered) = year.recovered {
                        lines.push(line(
                            &format!("project.annual_recovery.{}", year.year),
                            "project annual recovery",
                            Some(recovered),
                            "Σ tool recovered amounts for this calendar year",
                        ));
                    }
                }
            }
            Some(ToolingTreatment::Separate) => {
                if let Some(charge) = totals.separate_charge {
                    lines.push(line(
                        "project.separate_charge",
                        "project separate charge",
                        Some(charge),
                        "Σ tool investments as one separate charge, counted once",
                    ));
                }
            }
            None => {}
        }
        lines
    }
}

/// The expanded quantity of one owner inside one finished product.
fn owner_quantity(owner: &ToolingOwner, product: &ProductView) -> u64 {
    match owner {
        ToolingOwner::PartVersion { part_version_id } => product.quantity_of(*part_version_id),
        ToolingOwner::Product { product_id } => u64::from(*product_id == product.product_id),
    }
}

/// The stable field path of a manual recovery quantity, with the scenario's choice index.
fn basis_path(index: Option<usize>) -> String {
    match index {
        Some(index) => format!("recovery.recovery[{index}].basis.quantity"),
        None => "recovery.recovery.basis.quantity".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Completeness and normalisation
// ---------------------------------------------------------------------------

/// `complete` only when the treatment is known (unless no tool entry exists at all), every
/// applicable amount is available and no issue was raised.
fn is_complete(outcome: &ToolingOutcome, treatment: Option<ToolingTreatment>) -> bool {
    if !outcome.issues.is_empty() || outcome.project_investment.is_none() {
        return false;
    }
    if outcome.tools.is_empty() {
        // No recovery channel applies: the zero-investment answer is complete even without a
        // treatment, and each quoted product's allocated investment is an explicit zero.
        if outcome.products.iter().any(|product| product.allocated_investment.is_none()) {
            return false;
        }
        return treatment != Some(ToolingTreatment::Amortized) || outcome.products.iter().all(|product| product.amortized_contribution.is_some());
    }
    let Some(treatment) = treatment else {
        return false;
    };
    for tool in &outcome.tools {
        if tool.investment.is_none() || tool.source_investment.is_none() || tool.recovery_quantity.is_none() {
            return false;
        }
        if tool.allocation.iter().any(|line| line.allocation_share.is_none() || line.allocated_investment.is_none()) {
            return false;
        }
        match treatment {
            ToolingTreatment::Amortized => {
                if tool.contribution_per_owner_unit.is_none() || tool.projected_recovery.is_none() || tool.under_over_recovery.is_none() {
                    return false;
                }
                if tool.allocation.iter().any(|line| line.product_contribution.is_none()) {
                    return false;
                }
            }
            ToolingTreatment::Separate => {
                if tool.separate_charge.is_none() {
                    return false;
                }
            }
        }
    }
    for product in &outcome.products {
        if product.allocated_investment.is_none() {
            return false;
        }
        if treatment == ToolingTreatment::Amortized && product.amortized_contribution.is_none() {
            return false;
        }
    }
    match treatment {
        ToolingTreatment::Amortized => outcome.project_amortized_recovered.is_some() && outcome.project_under_over_recovery.is_some(),
        ToolingTreatment::Separate => outcome.project_separate_charge.is_some(),
    }
}

fn norm(value: Decimal) -> Decimal {
    value.normalize()
}

fn norm_opt(value: Option<Decimal>) -> Option<Decimal> {
    value.map(norm)
}

/// Normalise every business decimal at the output boundary so repeated runs agree byte for byte.
fn normalize_outcome(outcome: &mut ToolingOutcome) {
    for tool in &mut outcome.tools {
        tool.cost_per_tool = norm(tool.cost_per_tool);
        tool.source_investment = norm_opt(tool.source_investment);
        tool.fx_rate = norm_opt(tool.fx_rate);
        tool.investment = norm_opt(tool.investment);
        tool.recovery_quantity = norm_opt(tool.recovery_quantity);
        tool.contribution_per_owner_unit = norm_opt(tool.contribution_per_owner_unit);
        for allocation in &mut tool.allocation {
            allocation.allocation_share = norm_opt(allocation.allocation_share);
            allocation.allocated_investment = norm_opt(allocation.allocated_investment);
            allocation.product_contribution = norm_opt(allocation.product_contribution);
        }
        for year in &mut tool.annual_recovery {
            year.recovered = norm_opt(year.recovered);
        }
        tool.projected_recovery = norm_opt(tool.projected_recovery);
        tool.under_over_recovery = norm_opt(tool.under_over_recovery);
        tool.separate_charge = norm_opt(tool.separate_charge);
        for explanation in &mut tool.explanations {
            explanation.amount = norm_opt(explanation.amount);
        }
    }
    for product in &mut outcome.products {
        product.allocated_investment = norm_opt(product.allocated_investment);
        product.amortized_contribution = norm_opt(product.amortized_contribution);
        for explanation in &mut product.explanations {
            explanation.amount = norm_opt(explanation.amount);
        }
    }
    outcome.project_investment = norm_opt(outcome.project_investment);
    outcome.project_amortized_recovered = norm_opt(outcome.project_amortized_recovered);
    outcome.project_under_over_recovery = norm_opt(outcome.project_under_over_recovery);
    outcome.project_separate_charge = norm_opt(outcome.project_separate_charge);
    for year in &mut outcome.annual_recovery {
        year.recovered = norm_opt(year.recovered);
    }
    for explanation in &mut outcome.explanations {
        explanation.amount = norm_opt(explanation.amount);
    }
}

#[cfg(test)]
#[path = "tooling_tests.rs"]
mod tooling_tests;
