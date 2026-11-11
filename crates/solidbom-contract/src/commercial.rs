//! Commercial scenarios (contract version 1, additive to API version 2, package SB-18).
//!
//! The portable vocabulary of the project's commercial settings, the named commercial scenarios,
//! their immutable saved versions and the pure per-product/per-year price, EBIT and revenue
//! answer. The rules are locked in `docs/solidbom/contracts/v2/commercial-scenarios.md`; the
//! arithmetic lives in the `solidbom-costing` crate's `commercial` module and the service adapter
//! in `services/solidbom-server/src/commercial.rs`.
//!
//! # Settings ownership
//!
//! The project owns the EBIT target rate and the annual selling-price productivity rate. A scenario
//! owns its name, its manual base unit selling prices (each excluding tooling), its SB-17 tooling
//! recovery assumptions and whether productivity applies to the tooling contribution. A scenario
//! cannot express a volume policy or an EBIT target, so it cannot override the project's. Each
//! saved scenario version pins one immutable SB-16 baseline snapshot; a comparison evaluates every
//! scenario against one named baseline and labels a scenario that was saved against another one.
//!
//! # Missing, zero and invalid values
//!
//! Money, rates and prices are [`Decimal`] serialized as decimal strings. A missing value is
//! `None`/`null`; an explicit zero is `Some(Decimal::ZERO)` and serializes as `"0"`. A blank
//! manual price is the absent entry, not a zero price: it has no valid manual-price EBIT and no
//! quoted revenue, and it never implies a zero revenue. A production year without a forecast stays
//! unavailable and produces a named issue; it is never treated as zero volume.
//!
//! # Boundaries
//!
//! This module stores no database identity, no wall clock and no reference lookup. The service
//! resolves the scenario record, the immutable baseline snapshot and the project settings of that
//! snapshot's exact project record version into [`CommercialInputs`]; the calculator is pure and
//! consumes the SB-17 [`ToolingOutcome`] without repeating the investment, allocation or recovery
//! arithmetic.
//!
//! # Versioning
//!
//! The calculator version string is published by the calculator crate
//! (`solidbom-commercial-scenarios/1`). Changing a formula, a rounding rule, a unit or the
//! supported scope needs a new calculator version; adding an optional field or a named issue for
//! an input that already had no usable result does not change a valid input's arithmetic.

use std::collections::BTreeSet;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::costing::check_request_id;
use crate::error::{ContractError, Result};
use crate::projects::{AnnualForecast, YearRange};
use crate::tooling::{ToolingOutcome, ToolingRecoveryInputs};
use crate::types::{CostLine, ResultState, check_text};

/// The most saved scenarios one project may hold.
pub const MAX_COMMERCIAL_SCENARIOS: usize = 1_000;
/// The most manual product prices one scenario may hold.
pub const MAX_COMMERCIAL_MANUAL_PRICES: usize = 1_000;
/// The most scenarios one comparison may name.
pub const MAX_COMMERCIAL_COMPARE_SCENARIOS: usize = 50;
/// The most products one commercial calculation may price.
pub const MAX_COMMERCIAL_PRODUCTS: usize = 1_000;
/// The longest schedule one product may price.
pub const MAX_COMMERCIAL_YEARS: usize = 200;
/// The most characters an optional scenario label may carry.
pub const MAX_COMMERCIAL_TEXT_CHARS: usize = 200;

// ---------------------------------------------------------------------------
// Project commercial settings (additive to the stored project document)
// ---------------------------------------------------------------------------

/// The project's commercial settings: the EBIT target and the annual selling-price productivity.
///
/// Both rates are project-owned and shared by every scenario. A scenario can never override them;
/// a comparison evaluates every scenario with the settings of the project record version its named
/// baseline snapshot recorded and labels a scenario saved under different settings.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectCommercial {
    /// The project EBIT target `t`: a margin on the base selling price, `0 ≤ t < 1`.
    pub ebit_target_rate: Decimal,
    /// The annual selling-price productivity rate, `0 ≤ rate < 1`. It reduces the selling price
    /// from each product's second production year and never reduces the manufacturing cost.
    pub annual_productivity_rate: Decimal,
}

impl ProjectCommercial {
    /// Validate the project settings: both rates are present and below 100%.
    pub fn validate(&self) -> Result<()> {
        validate_rate(self.ebit_target_rate, "project.commercial.ebit_target_rate", "the EBIT target")?;
        validate_rate(self.annual_productivity_rate, "project.commercial.annual_productivity_rate", "the annual productivity rate")
    }
}

/// Replace the project's commercial settings: an append-only, expected-version-checked project write.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateProjectCommercial {
    pub request_id: String,
    pub expected_version: u64,
    pub commercial: ProjectCommercial,
}

impl UpdateProjectCommercial {
    /// Validate the request envelope and the settings.
    pub fn validate(&self) -> Result<()> {
        check_request_id(&self.request_id)?;
        if self.expected_version == 0 {
            return Err(ContractError::invalid("update_project_commercial.expected_version", "the expected project version starts at 1"));
        }
        self.commercial.validate()
    }
}

/// A rate that is a fraction below one: zero is a real value, one and above are refused.
fn validate_rate(rate: Decimal, field: &str, what: &str) -> Result<()> {
    if rate.is_sign_negative() {
        return Err(ContractError::invalid(field, format!("{what} cannot be negative")));
    }
    if rate >= Decimal::ONE {
        return Err(ContractError::invalid(field, format!("{what} must stay below 100%")));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Named commercial scenarios
// ---------------------------------------------------------------------------

/// One manual base unit selling price: the entered quoted price of one product, excluding tooling.
///
/// Only present prices are stored; a product without an entry has a blank manual price. The entered
/// price is the base unit selling price, and the annual productivity reduction applies to it
/// exactly like the suggested price.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScenarioManualPrice {
    pub product_id: Uuid,
    /// The first-production-year base unit selling price, strictly positive.
    pub price: Decimal,
}

impl ScenarioManualPrice {
    fn validate(&self, field: &str) -> Result<()> {
        if self.product_id.is_nil() {
            return Err(ContractError::invalid(format!("{field}.product_id"), "the product id is empty"));
        }
        if self.price <= Decimal::ZERO {
            return Err(ContractError::invalid(
                format!("{field}.price"),
                "a manual base price must be strictly positive; leave it out for a blank price",
            ));
        }
        Ok(())
    }
}

/// The editable content of one named commercial scenario.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CommercialScenarioContent {
    /// The scenario's explanation label.
    pub name: String,
    /// The immutable baseline snapshot this scenario version is saved against.
    pub baseline_calculation_id: Uuid,
    /// The entered base unit prices, each excluding tooling; missing products stay blank.
    #[serde(default)]
    pub manual_prices: Vec<ScenarioManualPrice>,
    /// The scenario's tooling treatment override and per-tool recovery assumptions. A tool without
    /// a choice uses its owner's lifetime demand; the treatment falls back to the project default.
    #[serde(default)]
    pub tooling: ToolingRecoveryInputs,
    /// Whether the annual productivity reduction also applies to the amortized tooling contribution.
    pub productivity_applies_to_tooling: bool,
}

impl CommercialScenarioContent {
    /// Validate the content structurally. The baseline existence is the service's rule.
    pub fn validate(&self) -> Result<()> {
        check_text(&self.name, "commercial_scenario.name")?;
        if self.name.chars().count() > MAX_COMMERCIAL_TEXT_CHARS {
            return Err(ContractError::invalid(
                "commercial_scenario.name",
                format!("the scenario name is at most {MAX_COMMERCIAL_TEXT_CHARS} characters"),
            ));
        }
        if self.baseline_calculation_id.is_nil() {
            return Err(ContractError::invalid("commercial_scenario.baseline_calculation_id", "the baseline calculation id is empty"));
        }
        validate_manual_prices(&self.manual_prices)?;
        self.tooling.validate()
    }

    /// The manual price of one product, if the scenario holds one.
    pub fn manual_price(&self, product_id: &Uuid) -> Option<Decimal> {
        self.manual_prices.iter().find(|price| price.product_id == *product_id).map(|price| price.price)
    }
}

/// Validate manual prices: at most 1,000, a non-nil unique product id in ascending order and a
/// strictly positive price. Keeping the order makes an issue's `manual_prices[i]` path unambiguous.
fn validate_manual_prices(prices: &[ScenarioManualPrice]) -> Result<()> {
    if prices.len() > MAX_COMMERCIAL_MANUAL_PRICES {
        return Err(ContractError::invalid(
            "commercial_scenario.manual_prices",
            format!("a scenario holds at most {MAX_COMMERCIAL_MANUAL_PRICES} manual prices"),
        ));
    }
    let mut product_ids = BTreeSet::new();
    let mut previous: Option<Uuid> = None;
    for (index, price) in prices.iter().enumerate() {
        price.validate(&format!("commercial_scenario.manual_prices[{index}]"))?;
        if !product_ids.insert(price.product_id) {
            return Err(ContractError::invalid("commercial_scenario.manual_prices", "two manual prices name one product"));
        }
        if let Some(previous) = previous
            && price.product_id <= previous
        {
            return Err(ContractError::invalid("commercial_scenario.manual_prices", "manual prices are ordered by ascending product id"));
        }
        previous = Some(price.product_id);
    }
    Ok(())
}

/// One immutable saved version of a named commercial scenario.
///
/// History lives in the append-only scenario versions; a later edit appends a new version and never
/// rewrites an earlier one. The recorded `project_version` is the project record version the saved
/// baseline snapshot names, retained so a comparison can label the scenario explicitly.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CommercialScenario {
    /// The stable business identity of the scenario across its versions.
    pub scenario_id: Uuid,
    pub project_id: Uuid,
    /// The append-only scenario version, starting at 1.
    pub version: u64,
    /// The scenario's explanation label.
    pub name: String,
    /// The immutable baseline snapshot this version is saved against.
    pub baseline_calculation_id: Uuid,
    /// The project record version that baseline snapshot recorded.
    pub project_version: u64,
    #[serde(default)]
    pub manual_prices: Vec<ScenarioManualPrice>,
    #[serde(default)]
    pub tooling: ToolingRecoveryInputs,
    pub productivity_applies_to_tooling: bool,
    /// The RFC 3339 UTC timestamp the service recorded.
    pub created_at: Option<String>,
}

impl CommercialScenario {
    /// Validate a saved scenario version.
    pub fn validate(&self) -> Result<()> {
        if self.scenario_id.is_nil() {
            return Err(ContractError::invalid("commercial_scenario.scenario_id", "the scenario id is empty"));
        }
        if self.project_id.is_nil() {
            return Err(ContractError::invalid("commercial_scenario.project_id", "the project id is empty"));
        }
        if self.version == 0 {
            return Err(ContractError::invalid("commercial_scenario.version", "a saved scenario version starts at 1"));
        }
        if self.project_version == 0 {
            return Err(ContractError::invalid("commercial_scenario.project_version", "the saved project version starts at 1"));
        }
        self.content().validate()
    }

    /// The editable content of this saved version.
    pub fn content(&self) -> CommercialScenarioContent {
        CommercialScenarioContent {
            name: self.name.clone(),
            baseline_calculation_id: self.baseline_calculation_id,
            manual_prices: self.manual_prices.clone(),
            tooling: self.tooling.clone(),
            productivity_applies_to_tooling: self.productivity_applies_to_tooling,
        }
    }
}

// ---------------------------------------------------------------------------
// Requests and responses
// ---------------------------------------------------------------------------

/// Create one named commercial scenario: version 1 of a new scenario identity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CreateCommercialScenario {
    pub request_id: String,
    /// The client-generated stable business identity of the new scenario.
    pub scenario_id: Uuid,
    pub content: CommercialScenarioContent,
}

impl CreateCommercialScenario {
    /// Validate the request envelope and its content.
    pub fn validate(&self) -> Result<()> {
        check_request_id(&self.request_id)?;
        if self.scenario_id.is_nil() {
            return Err(ContractError::invalid("create_commercial_scenario.scenario_id", "the scenario id is empty"));
        }
        self.content.validate()
    }
}

/// Save a new immutable version of one named commercial scenario. Expected-version checked.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UpdateCommercialScenario {
    pub request_id: String,
    pub scenario_id: Uuid,
    /// The scenario version the edit was based on; the next version is `expected_version + 1`.
    pub expected_version: u64,
    pub content: CommercialScenarioContent,
}

impl UpdateCommercialScenario {
    /// Validate the request envelope and its content.
    pub fn validate(&self) -> Result<()> {
        check_request_id(&self.request_id)?;
        if self.scenario_id.is_nil() {
            return Err(ContractError::invalid("update_commercial_scenario.scenario_id", "the scenario id is empty"));
        }
        if self.expected_version == 0 {
            return Err(ContractError::invalid("update_commercial_scenario.expected_version", "a saved scenario starts at version 1"));
        }
        self.content.validate()
    }
}

/// One committed scenario write: the saved immutable version.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CommercialScenarioWriteOutcome {
    pub scenario: CommercialScenario,
}

/// The current immutable version of every named scenario of one project, ordered by scenario id.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CommercialScenarioList {
    pub project_id: Uuid,
    pub scenarios: Vec<CommercialScenario>,
}

/// Compare named scenarios against one common immutable baseline.
///
/// Every scenario is evaluated against `baseline_calculation_id`; a scenario saved against another
/// baseline or project version is still evaluated but explicitly labelled stale, so comparisons
/// from different baselines are never presented as equivalent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompareCommercialScenarios {
    /// The common immutable SB-16 baseline snapshot the comparison uses.
    pub baseline_calculation_id: Uuid,
    /// The compared scenarios in ascending order; at least one.
    pub scenario_ids: Vec<Uuid>,
}

impl CompareCommercialScenarios {
    /// Validate the request envelope and the scenario selection.
    pub fn validate(&self) -> Result<()> {
        if self.baseline_calculation_id.is_nil() {
            return Err(ContractError::invalid("compare_commercial_scenarios.baseline_calculation_id", "the baseline calculation id is empty"));
        }
        if self.scenario_ids.is_empty() {
            return Err(ContractError::invalid("compare_commercial_scenarios.scenario_ids", "a comparison needs at least one scenario"));
        }
        if self.scenario_ids.len() > MAX_COMMERCIAL_COMPARE_SCENARIOS {
            return Err(ContractError::invalid(
                "compare_commercial_scenarios.scenario_ids",
                format!("a comparison names at most {MAX_COMMERCIAL_COMPARE_SCENARIOS} scenarios"),
            ));
        }
        let mut previous: Option<Uuid> = None;
        for (index, scenario_id) in self.scenario_ids.iter().enumerate() {
            if scenario_id.is_nil() {
                return Err(ContractError::invalid(format!("compare_commercial_scenarios.scenario_ids[{index}]"), "the scenario id is empty"));
            }
            if let Some(previous) = previous
                && *scenario_id <= previous
            {
                return Err(ContractError::invalid(
                    "compare_commercial_scenarios.scenario_ids",
                    "scenarios are named once each, in ascending id order",
                ));
            }
            previous = Some(*scenario_id);
        }
        Ok(())
    }
}

/// One named reason a saved scenario is not directly comparable with the comparison baseline.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommercialStaleReason {
    /// A stable contract name, not display text.
    pub code: String,
    pub message: String,
}

impl CommercialStaleReason {
    /// A named stale reason.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into() }
    }
}

/// One compared scenario: its saved identity plus the answer evaluated against the common baseline.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CommercialComparisonScenario {
    pub scenario_id: Uuid,
    /// The compared saved scenario version.
    pub version: u64,
    pub name: String,
    /// The baseline the saved scenario version pinned.
    pub saved_baseline_calculation_id: Uuid,
    /// The project record version that saved baseline pinned.
    pub saved_project_version: u64,
    /// Whether the saved scenario was pinned to another baseline, project version or settings.
    pub stale: bool,
    /// The named reasons; empty when the scenario is directly comparable.
    pub stale_reasons: Vec<CommercialStaleReason>,
    /// The evaluated price, EBIT, revenue and tooling answer over the comparison baseline.
    pub outcome: CommercialOutcome,
}

/// The shared project settings of one comparison and one line per compared scenario.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CommercialScenarioComparison {
    pub project_id: Uuid,
    /// The common immutable baseline snapshot every scenario was evaluated against.
    pub baseline_calculation_id: Uuid,
    /// The project record version that baseline snapshot recorded.
    pub project_version: u64,
    /// The project's volume policy read from the baseline snapshot, shared by every scenario.
    pub volume_policy: Option<crate::costing::CostingVolumePolicy>,
    /// The project EBIT target read from the baseline's project record version.
    pub ebit_target_rate: Option<Decimal>,
    /// The project annual productivity rate read from the baseline's project record version.
    pub annual_productivity_rate: Option<Decimal>,
    /// One line per compared scenario, in the requested ascending order.
    pub scenarios: Vec<CommercialComparisonScenario>,
    pub issues: Vec<CommercialIssue>,
}

// ---------------------------------------------------------------------------
// Pure calculator input and output
// ---------------------------------------------------------------------------

/// One quoted product the commercial calculation prices.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommercialProductInput {
    pub product_id: Uuid,
    /// The product revision the baseline read, retained for the explanation.
    pub product_revision_id: Option<Uuid>,
    /// The product's effective inclusive SOP–EOP calendar-year schedule.
    pub schedule: YearRange,
    /// The stored positive annual forecasts; a production year without one stays incomplete.
    #[serde(default)]
    pub forecasts: Vec<AnnualForecast>,
    /// The baseline product line's recurring unit cost `C`: material + production + attached +
    /// the configured operating categories included in EBIT. Setup and tooling are outside it.
    pub recurring_unit_cost: Option<Decimal>,
}

impl CommercialProductInput {
    fn validate(&self, field: &str) -> Result<()> {
        if self.product_id.is_nil() {
            return Err(ContractError::invalid(format!("{field}.product_id"), "the product id is empty"));
        }
        if let Some(revision_id) = self.product_revision_id
            && revision_id.is_nil()
        {
            return Err(ContractError::invalid(format!("{field}.product_revision_id"), "the product revision id is empty"));
        }
        self.schedule.validate(&format!("{field}.schedule"))?;
        if self.forecasts.len() > MAX_COMMERCIAL_YEARS {
            return Err(ContractError::invalid(format!("{field}.forecasts"), "too many forecast years"));
        }
        let mut years = BTreeSet::new();
        for (index, forecast) in self.forecasts.iter().enumerate() {
            let forecast_field = format!("{field}.forecasts[{index}]");
            if !(1900..=9999).contains(&forecast.year) {
                return Err(ContractError::invalid(format!("{forecast_field}.year"), "a forecast year is a four-digit calendar year"));
            }
            if forecast.quantity == 0 {
                return Err(ContractError::invalid(
                    format!("{forecast_field}.quantity"),
                    "a production year needs a positive forecast; leave a year out instead of zero",
                ));
            }
            if forecast.year < self.schedule.sop_year || forecast.year > self.schedule.eop_year {
                return Err(ContractError::invalid(
                    format!("{forecast_field}.year"),
                    "a forecast year is outside the product schedule; do not pad years",
                ));
            }
            if !years.insert(forecast.year) {
                return Err(ContractError::invalid(format!("{field}.forecasts"), "two forecasts name one calendar year"));
            }
        }
        if let Some(cost) = self.recurring_unit_cost
            && cost.is_sign_negative()
        {
            return Err(ContractError::invalid(format!("{field}.recurring_unit_cost"), "the recurring unit cost cannot be negative"));
        }
        Ok(())
    }

    /// The stored forecast of one calendar year, if the product has one.
    pub fn forecast(&self, year: u16) -> Option<u64> {
        self.forecasts.iter().find(|forecast| forecast.year == year).map(|forecast| forecast.quantity)
    }
}

/// The resolved input of one pure commercial calculation.
///
/// The service builds it from one saved scenario version, the immutable SB-16 baseline snapshot it
/// names, that snapshot's exact project record version and the SB-17 tooling evaluation of the same
/// snapshot with the scenario's recovery assumptions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CommercialInputs {
    pub project_currency: String,
    /// The project EBIT target; `None` is missing and leaves the suggested price unavailable.
    pub ebit_target_rate: Option<Decimal>,
    /// The project annual productivity rate; `None` is missing and leaves every price unavailable.
    pub annual_productivity_rate: Option<Decimal>,
    pub productivity_applies_to_tooling: bool,
    /// The scenario's entered manual prices; a product without an entry has a blank manual price.
    #[serde(default)]
    pub manual_prices: Vec<ScenarioManualPrice>,
    /// The SB-17 tooling answer for the same baseline and the scenario's recovery assumptions.
    pub tooling: ToolingOutcome,
    #[serde(default)]
    pub products: Vec<CommercialProductInput>,
    /// The baseline's separate setup total; `None` is missing.
    pub setup_total: Option<Decimal>,
}

impl CommercialInputs {
    /// Validate the resolved input before a calculation.
    pub fn validate(&self) -> Result<()> {
        check_currency(&self.project_currency)?;
        for (name, rate) in [("ebit_target_rate", self.ebit_target_rate), ("annual_productivity_rate", self.annual_productivity_rate)] {
            if let Some(rate) = rate {
                validate_rate(rate, name, if name == "ebit_target_rate" { "the EBIT target" } else { "the annual productivity rate" })?;
            }
        }
        if let Some(setup_total) = self.setup_total
            && setup_total.is_sign_negative()
        {
            return Err(ContractError::invalid("setup_total", "the separate setup total cannot be negative"));
        }
        if self.products.len() > MAX_COMMERCIAL_PRODUCTS {
            return Err(ContractError::invalid("products", format!("at most {MAX_COMMERCIAL_PRODUCTS} products")));
        }
        let mut product_ids = BTreeSet::new();
        let mut previous_product: Option<Uuid> = None;
        for (index, product) in self.products.iter().enumerate() {
            product.validate(&format!("products[{index}]"))?;
            if !product_ids.insert(product.product_id) {
                return Err(ContractError::invalid("products", "two products share one product id"));
            }
            if let Some(previous) = previous_product
                && product.product_id <= previous
            {
                return Err(ContractError::invalid("products", "products are ordered by ascending product id"));
            }
            previous_product = Some(product.product_id);
        }
        validate_manual_prices(&self.manual_prices)?;
        for (index, price) in self.manual_prices.iter().enumerate() {
            if !product_ids.contains(&price.product_id) {
                return Err(ContractError::invalid(
                    format!("manual_prices[{index}].product_id"),
                    "a manual price names a product the calculation does not price",
                ));
            }
        }
        Ok(())
    }

    /// The manual price of one product, if the scenario holds one.
    pub fn manual_price(&self, product_id: &Uuid) -> Option<Decimal> {
        self.manual_prices.iter().find(|price| price.product_id == *product_id).map(|price| price.price)
    }
}

/// One product's prices, EBIT and revenue in one production year.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommercialYearLine {
    pub year: u16,
    /// The stored positive forecast; `None` is missing, never zero.
    pub forecast: Option<u64>,
    /// The recurring unit cost `C`, fixed across the production years.
    pub recurring_unit_cost: Option<Decimal>,
    /// The suggested base unit selling price: first year `C / (1 − t)`, multiplied by
    /// `(1 − productivity)` for every later production year.
    pub suggested_base_price: Option<Decimal>,
    /// The manual base unit selling price series; `None` when the manual price is blank.
    pub manual_base_price: Option<Decimal>,
    /// The amortized tooling contribution in this year (after the scenario's productivity
    /// treatment); `None` under the separate treatment because it does not apply there.
    pub tooling_contribution: Option<Decimal>,
    /// `suggested_base_price + tooling_contribution`.
    pub suggested_total_price: Option<Decimal>,
    /// `manual_base_price + tooling_contribution`; `None` when the manual price is blank.
    pub manual_total_price: Option<Decimal>,
    /// `suggested_base_price − C`.
    pub suggested_unit_ebit_amount: Option<Decimal>,
    /// `(suggested_base_price − C) / suggested_base_price`.
    pub suggested_unit_ebit_rate: Option<Decimal>,
    /// `manual_base_price − C`; `None` when the manual price is blank.
    pub manual_unit_ebit_amount: Option<Decimal>,
    /// `(manual_base_price − C) / manual_base_price`; `None` when the manual price is blank.
    pub manual_unit_ebit_rate: Option<Decimal>,
    /// Quoted revenue: `manual_base_price × forecast`; `None` when the manual price is blank.
    pub revenue: Option<Decimal>,
    /// Recurring EBIT from the quoted price: `(manual_base_price − C) × forecast`.
    pub ebit_amount: Option<Decimal>,
    /// Amortized tooling recovery revenue: `tooling_contribution × forecast` (amortized only).
    pub tooling_recovery_revenue: Option<Decimal>,
}

/// One quoted product's whole commercial answer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommercialProductLine {
    pub product_id: Uuid,
    pub product_revision_id: Option<Uuid>,
    pub schedule: YearRange,
    /// The entered first-year base unit price; `None` is a blank manual price.
    pub manual_price: Option<Decimal>,
    /// The suggested first-year base unit price `C / (1 − t)`.
    pub suggested_first_year_price: Option<Decimal>,
    /// `manual_price − C`: the first-year manual-price EBIT amount.
    pub manual_first_year_ebit_amount: Option<Decimal>,
    /// `(manual_price − C) / manual_price`: the first-year manual-price EBIT rate.
    pub manual_first_year_ebit_rate: Option<Decimal>,
    /// The amortized tooling contribution before any productivity reduction (amortized only).
    pub tooling_initial_contribution: Option<Decimal>,
    /// The product's share of the project tooling investment (separate or amortized).
    pub allocated_investment: Option<Decimal>,
    /// One line per schedule year, ascending.
    pub years: Vec<CommercialYearLine>,
    pub explanations: Vec<CostLine>,
}

/// One calendar year's project totals over the quoted products.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommercialYearTotal {
    pub year: u16,
    /// `None` when a quoted product has no manual price or no forecast.
    pub revenue: Option<Decimal>,
    pub ebit_amount: Option<Decimal>,
    /// The summed amortized tooling recovery revenue (amortized only; empty under separate).
    pub tooling_recovery_revenue: Option<Decimal>,
}

/// The project totals of one commercial answer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommercialProjectTotals {
    /// The summed quoted revenue; `None` when a quoted product has no manual price or forecast.
    pub lifetime_revenue: Option<Decimal>,
    pub lifetime_ebit_amount: Option<Decimal>,
    /// One line per calendar year that any quoted product produces in, ascending.
    pub years: Vec<CommercialYearTotal>,
    /// The project tooling investment, counted once (from the SB-17 answer).
    pub tooling_investment: Option<Decimal>,
    /// The whole investment as a separate charge (separate treatment only).
    pub tooling_separate_charge: Option<Decimal>,
    /// The projected lifetime recovery (amortized treatment only).
    pub tooling_projected_recovery: Option<Decimal>,
    /// Positive over-recovery, negative under-recovery (amortized treatment only).
    pub tooling_under_over_recovery: Option<Decimal>,
}

/// One named problem of a commercial calculation. Issues are computed on read, never stored.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommercialIssue {
    pub code: String,
    pub field_path: String,
    pub product_id: Option<Uuid>,
    pub message: String,
}

impl CommercialIssue {
    /// A named issue. `code` is a stable contract name, not display text.
    pub fn new(code: impl Into<String>, field_path: impl Into<String>, message: impl Into<String>) -> Self {
        Self { code: code.into(), field_path: field_path.into(), product_id: None, message: message.into() }
    }

    /// Attach the product the issue is about.
    pub fn for_product(mut self, product_id: Uuid) -> Self {
        self.product_id = Some(product_id);
        self
    }
}

/// The calculator's explained commercial answer: one product line per quoted product, the project
/// totals and the SB-17 tooling answer it consumed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CommercialOutcome {
    pub state: ResultState,
    pub ebit_target_rate: Option<Decimal>,
    pub annual_productivity_rate: Option<Decimal>,
    pub productivity_applies_to_tooling: bool,
    /// One line per quoted product, ordered by product id.
    pub products: Vec<CommercialProductLine>,
    pub project: CommercialProjectTotals,
    /// The SB-17 tooling answer this calculation consumed; its investment is never recomputed here.
    pub tooling: ToolingOutcome,
    /// The baseline's separate setup charge, kept outside the recurring EBIT.
    pub setup_total: Option<Decimal>,
    pub explanations: Vec<CostLine>,
    pub issues: Vec<CommercialIssue>,
}

/// A three-letter upper-case currency code, as the shared-data contract spells it.
fn check_currency(currency: &str) -> Result<()> {
    if currency.len() != 3 || !currency.bytes().all(|byte| byte.is_ascii_uppercase()) {
        return Err(ContractError::invalid("project_currency", "expected a three-letter upper-case currency code"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: u128) -> Uuid {
        Uuid::from_u128(value)
    }

    fn decimal(text: &str) -> Decimal {
        text.parse().expect("a decimal literal in the test fixture")
    }

    fn content() -> CommercialScenarioContent {
        CommercialScenarioContent {
            name: "baseline".into(),
            baseline_calculation_id: id(3),
            manual_prices: vec![ScenarioManualPrice { product_id: id(7), price: decimal("100") }],
            tooling: ToolingRecoveryInputs::default(),
            productivity_applies_to_tooling: true,
        }
    }

    #[test]
    fn the_project_rates_are_below_one_and_never_negative() {
        let ok = ProjectCommercial { ebit_target_rate: decimal("0.2"), annual_productivity_rate: decimal("0.03") };
        assert!(ok.validate().is_ok());
        let zero = ProjectCommercial { ebit_target_rate: Decimal::ZERO, annual_productivity_rate: Decimal::ZERO };
        assert!(zero.validate().is_ok());
        let negative = ProjectCommercial { ebit_target_rate: decimal("-0.1"), annual_productivity_rate: decimal("0.03") };
        assert!(negative.validate().is_err());
        let hundred = ProjectCommercial { ebit_target_rate: Decimal::ONE, annual_productivity_rate: decimal("0.03") };
        assert!(hundred.validate().is_err());
        let above = ProjectCommercial { ebit_target_rate: decimal("0.2"), annual_productivity_rate: decimal("1.2") };
        assert!(above.validate().is_err());
    }

    #[test]
    fn the_project_settings_write_is_an_expected_version_checked_request() {
        let request = UpdateProjectCommercial {
            request_id: "r".into(),
            expected_version: 1,
            commercial: ProjectCommercial { ebit_target_rate: decimal("0.2"), annual_productivity_rate: decimal("0.03") },
        };
        assert!(request.validate().is_ok());
        let mut zero_version = request.clone();
        zero_version.expected_version = 0;
        assert!(zero_version.validate().is_err());
        let mut invalid_rate = request;
        invalid_rate.commercial.ebit_target_rate = Decimal::ONE;
        assert!(invalid_rate.validate().is_err());
    }

    #[test]
    fn the_optional_commercial_field_reads_as_missing_on_an_earlier_document() {
        let earlier = serde_json::json!({
            "schema_version": 1,
            "project_id": "00000000-0000-0000-0000-000000000001",
            "name": "p",
            "customer": "c",
            "currency": "CNY",
            "default_schedule": { "sop_year": 2027, "eop_year": 2028 },
            "products": [],
            "packages": [],
            "part_versions": [],
            "quotation_events": []
        });
        let document: crate::projects::ProjectDocument = serde_json::from_value(earlier).expect("a document written before SB-18 parses");
        assert!(document.commercial.is_none());
        assert!(document.validate().is_ok());
    }

    #[test]
    fn a_scenario_content_needs_a_name_a_baseline_and_positive_ordered_prices() {
        let ok = content();
        assert!(ok.validate().is_ok());
        assert_eq!(ok.manual_price(&id(7)), Some(decimal("100")));
        assert_eq!(ok.manual_price(&id(8)), None);

        let mut blank_name = ok.clone();
        blank_name.name = "  ".into();
        assert!(blank_name.validate().is_err());

        let mut nil_baseline = ok.clone();
        nil_baseline.baseline_calculation_id = Uuid::nil();
        assert!(nil_baseline.validate().is_err());

        let mut zero_price = ok.clone();
        zero_price.manual_prices = vec![ScenarioManualPrice { product_id: id(7), price: Decimal::ZERO }];
        assert!(zero_price.validate().is_err());

        let mut duplicate = ok.clone();
        duplicate.manual_prices =
            vec![ScenarioManualPrice { product_id: id(7), price: decimal("100") }, ScenarioManualPrice { product_id: id(7), price: decimal("90") }];
        assert!(duplicate.validate().is_err());

        let mut unordered = ok.clone();
        unordered.manual_prices =
            vec![ScenarioManualPrice { product_id: id(8), price: decimal("100") }, ScenarioManualPrice { product_id: id(7), price: decimal("90") }];
        assert!(unordered.validate().is_err());
    }

    #[test]
    fn a_scenario_cannot_express_an_ebit_target_or_volume_policy() {
        // The scenario content has no such field; an unknown JSON field is tolerated and ignored,
        // never promoted into an override.
        let mut value = serde_json::to_value(content()).expect("the content serializes");
        value["ebit_target_rate"] = serde_json::json!("0.5");
        value["volume_policy"] = serde_json::json!("maximum");
        let content: CommercialScenarioContent = serde_json::from_value(value).expect("unknown fields are tolerated");
        assert!(content.validate().is_ok());
        let text = serde_json::to_string(&content).expect("the content serializes");
        assert!(!text.contains("ebit_target_rate"), "the scenario content never carries an EBIT target");
        assert!(!text.contains("volume_policy"), "the scenario content never carries a volume policy");
    }

    #[test]
    fn a_saved_scenario_version_is_append_only_and_validated() {
        let scenario = CommercialScenario {
            scenario_id: id(1),
            project_id: id(2),
            version: 1,
            name: "baseline".into(),
            baseline_calculation_id: id(3),
            project_version: 4,
            manual_prices: vec![ScenarioManualPrice { product_id: id(7), price: decimal("100") }],
            tooling: ToolingRecoveryInputs::default(),
            productivity_applies_to_tooling: false,
            created_at: None,
        };
        assert!(scenario.validate().is_ok());
        assert_eq!(scenario.content().manual_prices, scenario.manual_prices);
        let mut zero_version = scenario.clone();
        zero_version.version = 0;
        assert!(zero_version.validate().is_err());
        let mut zero_project = scenario.clone();
        zero_project.project_version = 0;
        assert!(zero_project.validate().is_err());
        let mut nil_scenario = scenario;
        nil_scenario.scenario_id = Uuid::nil();
        assert!(nil_scenario.validate().is_err());
    }

    #[test]
    fn the_write_requests_and_a_comparison_are_validated() {
        let create = CreateCommercialScenario { request_id: "r".into(), scenario_id: id(1), content: content() };
        assert!(create.validate().is_ok());
        let update = UpdateCommercialScenario { request_id: "r".into(), scenario_id: id(1), expected_version: 1, content: content() };
        assert!(update.validate().is_ok());
        let mut zero_expected = update;
        zero_expected.expected_version = 0;
        assert!(zero_expected.validate().is_err());
        let mut nil_scenario = create;
        nil_scenario.scenario_id = Uuid::nil();
        assert!(nil_scenario.validate().is_err());

        let compare = CompareCommercialScenarios { baseline_calculation_id: id(3), scenario_ids: vec![id(1), id(2)] };
        assert!(compare.validate().is_ok());
        let empty = CompareCommercialScenarios { baseline_calculation_id: id(3), scenario_ids: Vec::new() };
        assert!(empty.validate().is_err());
        let unordered = CompareCommercialScenarios { baseline_calculation_id: id(3), scenario_ids: vec![id(2), id(1)] };
        assert!(unordered.validate().is_err());
        let duplicate = CompareCommercialScenarios { baseline_calculation_id: id(3), scenario_ids: vec![id(1), id(1)] };
        assert!(duplicate.validate().is_err());
        let nil_baseline = CompareCommercialScenarios { baseline_calculation_id: Uuid::nil(), scenario_ids: vec![id(1)] };
        assert!(nil_baseline.validate().is_err());
    }

    #[test]
    fn the_commercial_input_refuses_unknown_manual_prices_and_unordered_products() {
        let product = CommercialProductInput {
            product_id: id(7),
            product_revision_id: None,
            schedule: YearRange { sop_year: 2027, eop_year: 2028 },
            forecasts: Vec::new(),
            recurring_unit_cost: Some(decimal("80")),
        };
        let mut inputs = CommercialInputs {
            project_currency: "CNY".into(),
            ebit_target_rate: Some(decimal("0.2")),
            annual_productivity_rate: Some(decimal("0.03")),
            productivity_applies_to_tooling: false,
            manual_prices: vec![ScenarioManualPrice { product_id: id(7), price: decimal("100") }],
            tooling: empty_tooling(),
            products: vec![product.clone()],
            setup_total: Some(Decimal::ZERO),
        };
        assert!(inputs.validate().is_ok());
        assert_eq!(inputs.manual_price(&id(7)), Some(decimal("100")));
        inputs.manual_prices = vec![ScenarioManualPrice { product_id: id(8), price: decimal("100") }];
        assert!(inputs.validate().is_err());
        let mut unordered = inputs.clone();
        unordered.manual_prices = Vec::new();
        unordered.products =
            vec![CommercialProductInput { product_id: id(8), ..product.clone() }, CommercialProductInput { product_id: id(7), ..product }];
        assert!(unordered.validate().is_err());
    }

    #[test]
    fn the_outcome_round_trips_with_missing_values_as_null() {
        let outcome = CommercialOutcome {
            state: ResultState::Incomplete,
            ebit_target_rate: None,
            annual_productivity_rate: Some(decimal("0.03")),
            productivity_applies_to_tooling: false,
            products: Vec::new(),
            project: CommercialProjectTotals {
                lifetime_revenue: None,
                lifetime_ebit_amount: None,
                years: Vec::new(),
                tooling_investment: Some(Decimal::ZERO),
                tooling_separate_charge: None,
                tooling_projected_recovery: None,
                tooling_under_over_recovery: None,
            },
            tooling: empty_tooling(),
            setup_total: None,
            explanations: Vec::new(),
            issues: vec![CommercialIssue::new("missing_ebit_target", "ebit_target_rate", "no target")],
        };
        let text = serde_json::to_string(&outcome).expect("the outcome serializes");
        assert!(text.contains("\"ebit_target_rate\":null"));
        let back: CommercialOutcome = serde_json::from_str(&text).expect("the outcome round trips");
        assert_eq!(back, outcome);
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
}
