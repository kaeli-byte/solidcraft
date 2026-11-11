//! Tooling investment and recovery (contract version 1, additive to API version 2, package SB-17).
//!
//! The portable vocabulary of the project's tooling definitions and of one tooling
//! investment/allocation/recovery evaluation. The rules are locked in
//! `docs/solidbom/contracts/v2/tooling-investment-and-recovery.md`; the arithmetic lives in the
//! `solidbom-costing` crate's `tooling` module and the service adapter in
//! `services/solidbom-server/src/tooling.rs`.
//!
//! # Missing, zero and invalid values
//!
//! Money, quantities and shares are [`Decimal`] serialized as decimal strings, and calendar-year
//! demand is `u64`. A missing value is `None`/`null`; an explicit zero is `Some(Decimal::ZERO)` and
//! serializes as `"0"`. The two are never interchangeable: a missing input is a named issue and an
//! unavailable amount, never a zero.
//!
//! # Boundaries
//!
//! This module carries the project tooling settings, the resolved input of one tooling evaluation
//! and its explained output. It stores no database identity, no wall clock and no reference
//! lookup. The service resolves the demand from an immutable SB-16 baseline snapshot; the
//! calculator is pure and never re-derives demand from a raw manifest.
//!
//! # Investment is counted once
//!
//! A tool entry's `tool_count × cost_per_tool` is its whole investment. Demand, occurrence counts
//! and allocation shares never multiply it. Allocation explains each product's share of that one
//! investment; the scenario recovery quantity separately turns it into a per-unit contribution.
//! One evaluation selects one treatment, so a single investment can never be recovered through both
//! a separate charge and full amortization.
//!
//! # Versioning
//!
//! The calculator version string is published by the calculator crate
//! (`solidbom-tooling-recovery/1`). Changing a formula, a rounding rule, a unit or the supported
//! scope needs a new calculator version; adding an optional field or a named issue for an input
//! that already had no usable result does not change a valid input's arithmetic.

use std::collections::BTreeSet;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::costing::{ExchangeRate, MAX_EXCHANGE_RATES, check_currency_code, check_request_id};
use crate::error::{ContractError, Result};
use crate::projects::AnnualForecast;
use crate::types::{CostLine, ResultState, check_text};

/// The most tool entries one project may hold.
pub const MAX_TOOLING_ENTRIES: usize = 10_000;
/// The most scenario recovery choices one evaluation may carry.
pub const MAX_TOOLING_RECOVERY_CHOICES: usize = 10_000;
/// The most quoted products one tooling demand may describe.
pub const MAX_TOOLING_PRODUCTS: usize = 1_000;
/// The most part quantities one product demand may carry.
pub const MAX_TOOLING_PART_QUANTITIES: usize = 10_000;
/// The most characters an optional tooling label or reference may carry.
pub const MAX_TOOLING_TEXT_CHARS: usize = 200;

// ---------------------------------------------------------------------------
// Project tooling settings (additive to the stored project document)
// ---------------------------------------------------------------------------

/// The commercial treatment of tooling: one separate charge or a per-unit amortized contribution.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ToolingTreatment {
    /// The whole investment is charged once as a separate line, outside the unit price.
    Separate,
    /// The investment is divided by a recovery quantity into a per-unit contribution.
    Amortized,
}

/// The single part version or product that owns one tool entry.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolingOwner {
    /// A shared project part version; the entry covers a component or an assembly.
    PartVersion { part_version_id: Uuid },
    /// One product; a product-root tool uses that product's own lifetime demand.
    Product { product_id: Uuid },
}

impl ToolingOwner {
    /// The part version this owner names, if any.
    pub fn part_version_id(&self) -> Option<Uuid> {
        match self {
            Self::PartVersion { part_version_id } => Some(*part_version_id),
            Self::Product { .. } => None,
        }
    }

    /// The product this owner names, if any.
    pub fn product_id(&self) -> Option<Uuid> {
        match self {
            Self::Product { product_id } => Some(*product_id),
            Self::PartVersion { .. } => None,
        }
    }

    fn validate(&self, field: &str) -> Result<()> {
        match self {
            Self::PartVersion { part_version_id } if part_version_id.is_nil() => {
                Err(ContractError::invalid(format!("{field}.part_version_id"), "the part version id is empty"))
            }
            Self::Product { product_id } if product_id.is_nil() => {
                Err(ContractError::invalid(format!("{field}.product_id"), "the product id is empty"))
            }
            _ => Ok(()),
        }
    }
}

/// One tooling entry: one physical investment owned by one part version or one product.
///
/// The investment is `tool_count × cost_per_tool` in the entry's currency. It is counted once for
/// the project; no occurrence, product or allocation multiplies it. One entry never spans several
/// definitions (multi-part/family tools are excluded from the MVP).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolingEntry {
    /// The stable business identity of the entry.
    pub tool_id: Uuid,
    /// The single part version or product that owns it.
    pub owner: ToolingOwner,
    /// An optional explanation label.
    #[serde(default)]
    pub name: Option<String>,
    /// How many physical tools the entry buys; at least one.
    pub tool_count: u64,
    /// The cost of one tool in `currency`; zero is an explicit value.
    pub cost_per_tool: Decimal,
    /// The three-letter upper-case currency of `cost_per_tool`.
    pub currency: String,
    /// Optional provenance text.
    #[serde(default)]
    pub source_reference: Option<String>,
}

impl ToolingEntry {
    fn validate(&self, field: &str) -> Result<()> {
        if self.tool_id.is_nil() {
            return Err(ContractError::invalid(format!("{field}.tool_id"), "the tool id is empty"));
        }
        self.owner.validate(&format!("{field}.owner"))?;
        for (name, value) in [("name", &self.name), ("source_reference", &self.source_reference)] {
            if let Some(value) = value {
                check_text(value, &format!("{field}.{name}"))?;
                if value.chars().count() > MAX_TOOLING_TEXT_CHARS {
                    return Err(ContractError::invalid(
                        format!("{field}.{name}"),
                        format!("the value is at most {MAX_TOOLING_TEXT_CHARS} characters"),
                    ));
                }
            }
        }
        if self.tool_count == 0 {
            return Err(ContractError::invalid(format!("{field}.tool_count"), "a tool entry needs at least one tool"));
        }
        if self.cost_per_tool.is_sign_negative() {
            return Err(ContractError::invalid(format!("{field}.cost_per_tool"), "a tool cost cannot be negative"));
        }
        check_currency_code(&self.currency, &format!("{field}.currency"))
    }

    /// The entry's investment in its own currency, checked. `None` means the multiplication is out
    /// of the supported decimal range.
    pub fn source_investment(&self) -> Option<Decimal> {
        Decimal::from(self.tool_count).checked_mul(self.cost_per_tool)
    }
}

/// The project's tooling settings: one default commercial treatment and the tool entries.
///
/// The settings are additive to the stored project document (`#[serde(default)]`); a document
/// written before this contract reads as `None`, never as an implicit treatment. Replacing them
/// appends one project record version.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ProjectTooling {
    /// The project's default commercial treatment; a scenario may override it.
    #[serde(default)]
    pub default_treatment: Option<ToolingTreatment>,
    /// The project's tool entries, each with one owner.
    #[serde(default)]
    pub entries: Vec<ToolingEntry>,
}

impl ProjectTooling {
    /// Validate the settings against the project they belong to: the named part version or product
    /// must exist in the same document and every tool id is unique.
    pub fn validate(&self, project_currency: &str, part_version_ids: &[Uuid], product_ids: &[Uuid]) -> Result<()> {
        check_currency_code(project_currency, "project.currency")?;
        self.validate_entries("project.tooling")?;
        for (index, entry) in self.entries.iter().enumerate() {
            match entry.owner {
                ToolingOwner::PartVersion { part_version_id } if !part_version_ids.contains(&part_version_id) => {
                    return Err(ContractError::invalid(
                        format!("project.tooling.entries[{index}].owner.part_version_id"),
                        "the tool owner names a part version this project does not have",
                    ));
                }
                ToolingOwner::Product { product_id } if !product_ids.contains(&product_id) => {
                    return Err(ContractError::invalid(
                        format!("project.tooling.entries[{index}].owner.product_id"),
                        "the tool owner names a product this project does not have",
                    ));
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Validate the settings structurally, without requiring the project's part version and product
    /// lists. The evaluation input uses this form because it receives the demand, not the document.
    pub fn validate_entries(&self, field: &str) -> Result<()> {
        if self.entries.len() > MAX_TOOLING_ENTRIES {
            return Err(ContractError::invalid(format!("{field}.entries"), format!("the project holds at most {MAX_TOOLING_ENTRIES} tool entries")));
        }
        let mut tool_ids = BTreeSet::new();
        for (index, entry) in self.entries.iter().enumerate() {
            entry.validate(&format!("{field}.entries[{index}]"))?;
            if !tool_ids.insert(entry.tool_id) {
                return Err(ContractError::invalid(format!("{field}.entries"), "two tool entries share one tool id"));
            }
        }
        Ok(())
    }

    /// The entry with this tool id, if the settings hold one.
    pub fn entry(&self, tool_id: &Uuid) -> Option<&ToolingEntry> {
        self.entries.iter().find(|entry| entry.tool_id == *tool_id)
    }
}

// ---------------------------------------------------------------------------
// Scenario recovery assumptions (consumed and retained by SB-18)
// ---------------------------------------------------------------------------

/// How one tool's contribution converts to a per-unit amount.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolingRecoveryBasis {
    /// The owner's combined lifetime demand across quoted products.
    LifetimeDemand,
    /// A manually entered quantity; a missing entry is not a zero.
    ManualQuantity {
        #[serde(default)]
        quantity: Option<Decimal>,
    },
}

impl ToolingRecoveryBasis {
    fn validate(&self, field: &str) -> Result<()> {
        if let Self::ManualQuantity { quantity: Some(quantity) } = self
            && *quantity <= Decimal::ZERO
        {
            return Err(ContractError::invalid(format!("{field}.quantity"), "a manual recovery quantity must be strictly positive"));
        }
        Ok(())
    }
}

/// One explicit recovery basis choice for one tool.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolingRecoveryChoice {
    pub tool_id: Uuid,
    pub basis: ToolingRecoveryBasis,
}

impl ToolingRecoveryChoice {
    fn validate(&self, field: &str) -> Result<()> {
        if self.tool_id.is_nil() {
            return Err(ContractError::invalid(format!("{field}.tool_id"), "the tool id is empty"));
        }
        self.basis.validate(&format!("{field}.basis"))
    }
}

/// The scenario's tooling recovery assumptions: the selected treatment and per-tool recovery bases.
///
/// A tool without an explicit choice uses its owner's lifetime demand. A choice naming a tool the
/// project no longer holds is ignored by the evaluation; labelling a stale scenario is SB-18's
/// concern. The project owns the default treatment; a scenario may override it but never the
/// investment or the allocation rule.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolingRecoveryInputs {
    /// The scenario's treatment; `None` uses the project default.
    #[serde(default)]
    pub treatment: Option<ToolingTreatment>,
    /// Explicit recovery bases; a tool without an entry uses lifetime demand.
    #[serde(default)]
    pub recovery: Vec<ToolingRecoveryChoice>,
}

impl ToolingRecoveryInputs {
    /// Validate the scenario assumptions.
    pub fn validate(&self) -> Result<()> {
        if self.recovery.len() > MAX_TOOLING_RECOVERY_CHOICES {
            return Err(ContractError::invalid("recovery.recovery", format!("at most {MAX_TOOLING_RECOVERY_CHOICES} recovery choices")));
        }
        let mut tool_ids = BTreeSet::new();
        for (index, choice) in self.recovery.iter().enumerate() {
            choice.validate(&format!("recovery.recovery[{index}]"))?;
            if !tool_ids.insert(choice.tool_id) {
                return Err(ContractError::invalid("recovery.recovery", "two recovery choices name one tool"));
            }
        }
        Ok(())
    }

    /// The basis selected for one tool; lifetime demand when the scenario holds no choice.
    pub fn basis_for(&self, tool_id: &Uuid) -> ToolingRecoveryBasis {
        self.recovery.iter().find(|choice| choice.tool_id == *tool_id).map_or(ToolingRecoveryBasis::LifetimeDemand, |choice| choice.basis)
    }
}

// ---------------------------------------------------------------------------
// Demand consumed from an SB-16 baseline snapshot
// ---------------------------------------------------------------------------

/// One part version's expanded quantity inside one finished product.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolingPartQuantity {
    pub part_version_id: Uuid,
    /// The non-suppressed occurrence count in one finished product; at least one.
    pub quantity: u64,
}

/// One quoted product's demand as one tooling evaluation consumes it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolingProductDemand {
    pub product_id: Uuid,
    /// The product revision the baseline read, retained for the explanation.
    pub product_revision_id: Option<Uuid>,
    /// The baseline product line's summed positive forecast; `None` is missing, never zero.
    pub lifetime_demand: Option<u64>,
    /// The same stored positive annual forecasts; used only for the annual recovery series.
    #[serde(default)]
    pub forecasts: Vec<AnnualForecast>,
    /// The expanded non-suppressed part quantities, ordered by part version id.
    #[serde(default)]
    pub part_quantities: Vec<ToolingPartQuantity>,
}

impl ToolingProductDemand {
    fn validate(&self, field: &str) -> Result<()> {
        if self.product_id.is_nil() {
            return Err(ContractError::invalid(format!("{field}.product_id"), "the product id is empty"));
        }
        if let Some(revision_id) = self.product_revision_id
            && revision_id.is_nil()
        {
            return Err(ContractError::invalid(format!("{field}.product_revision_id"), "the product revision id is empty"));
        }
        if self.lifetime_demand == Some(0) {
            return Err(ContractError::invalid(
                format!("{field}.lifetime_demand"),
                "a quoted product with a schedule has a positive lifetime demand; a missing value is null",
            ));
        }
        if self.forecasts.len() > crate::costing::MAX_BASELINE_SCHEDULE_YEARS {
            return Err(ContractError::invalid(format!("{field}.forecasts"), "too many forecast years"));
        }
        let mut years = BTreeSet::new();
        for (index, forecast) in self.forecasts.iter().enumerate() {
            forecast.validate(&format!("{field}.forecasts[{index}]"))?;
            if !years.insert(forecast.year) {
                return Err(ContractError::invalid(format!("{field}.forecasts"), "two forecasts name one calendar year"));
            }
        }
        if self.part_quantities.len() > MAX_TOOLING_PART_QUANTITIES {
            return Err(ContractError::invalid(
                format!("{field}.part_quantities"),
                format!("a product holds at most {MAX_TOOLING_PART_QUANTITIES} part quantities"),
            ));
        }
        let mut part_ids = BTreeSet::new();
        let mut previous_part: Option<Uuid> = None;
        for (index, part) in self.part_quantities.iter().enumerate() {
            if part.part_version_id.is_nil() {
                return Err(ContractError::invalid(format!("{field}.part_quantities[{index}].part_version_id"), "the part version id is empty"));
            }
            if part.quantity == 0 {
                return Err(ContractError::invalid(
                    format!("{field}.part_quantities[{index}].quantity"),
                    "an expanded part quantity is at least one; leave an unused part out",
                ));
            }
            if !part_ids.insert(part.part_version_id) {
                return Err(ContractError::invalid(format!("{field}.part_quantities"), "two quantities name one part version"));
            }
            if let Some(previous) = previous_part
                && part.part_version_id <= previous
            {
                return Err(ContractError::invalid(format!("{field}.part_quantities"), "part quantities are ordered by ascending part version id"));
            }
            previous_part = Some(part.part_version_id);
        }
        Ok(())
    }

    /// The expanded quantity of one part version in one finished product (zero when absent).
    pub fn quantity_of(&self, part_version_id: Uuid) -> u64 {
        self.part_quantities.iter().find(|part| part.part_version_id == part_version_id).map_or(0, |part| part.quantity)
    }
}

/// The quoted-product demand one tooling evaluation consumes, resolved from a baseline snapshot.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolingDemand {
    /// One entry per quoted product, ordered by product id.
    #[serde(default)]
    pub products: Vec<ToolingProductDemand>,
}

impl ToolingDemand {
    /// Validate the resolved demand.
    pub fn validate(&self) -> Result<()> {
        if self.products.len() > MAX_TOOLING_PRODUCTS {
            return Err(ContractError::invalid("demand.products", format!("at most {MAX_TOOLING_PRODUCTS} quoted products")));
        }
        let mut product_ids = BTreeSet::new();
        let mut previous_product: Option<Uuid> = None;
        for (index, product) in self.products.iter().enumerate() {
            product.validate(&format!("demand.products[{index}]"))?;
            if !product_ids.insert(product.product_id) {
                return Err(ContractError::invalid("demand.products", "two products share one product id"));
            }
            if let Some(previous) = previous_product
                && product.product_id <= previous
            {
                return Err(ContractError::invalid("demand.products", "products are ordered by ascending product id"));
            }
            previous_product = Some(product.product_id);
        }
        Ok(())
    }

    /// The quoted product with this id, if the demand holds one.
    pub fn product(&self, product_id: &Uuid) -> Option<&ToolingProductDemand> {
        self.products.iter().find(|product| product.product_id == *product_id)
    }
}

// ---------------------------------------------------------------------------
// Evaluation input
// ---------------------------------------------------------------------------

/// The resolved input of one pure tooling evaluation. The service builds it from the project
/// tooling settings, the project's saved exchange rates and an immutable baseline snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolingInputs {
    /// One project currency; every investment converts to it.
    pub project_currency: String,
    /// The project's saved manual exchange rates.
    #[serde(default)]
    pub exchange_rates: Vec<ExchangeRate>,
    /// The project tooling definitions the evaluated project record version holds.
    pub tooling: ProjectTooling,
    /// The scenario's treatment and per-tool recovery assumptions.
    pub recovery: ToolingRecoveryInputs,
    /// The quoted demand and purchased cutoffs the baseline resolved.
    pub demand: ToolingDemand,
}

impl ToolingInputs {
    /// Validate the resolved input before a calculation. Owner existence is the project document's
    /// rule; this input only requires the part version or product identity to be structurally sound.
    pub fn validate(&self) -> Result<()> {
        check_currency_code(&self.project_currency, "project_currency")?;
        if self.exchange_rates.len() > MAX_EXCHANGE_RATES {
            return Err(ContractError::invalid("exchange_rates", format!("at most {MAX_EXCHANGE_RATES} saved exchange rates")));
        }
        let mut sources = BTreeSet::new();
        for (index, rate) in self.exchange_rates.iter().enumerate() {
            rate.validate(&format!("exchange_rates[{index}]"))?;
            if rate.source_currency == self.project_currency {
                return Err(ContractError::invalid(
                    format!("exchange_rates[{index}].source_currency"),
                    "the project currency converts to itself; remove the identity rate",
                ));
            }
            if !sources.insert(rate.source_currency.as_str()) {
                return Err(ContractError::invalid(format!("exchange_rates[{index}].source_currency"), "two saved rates name one source currency"));
            }
        }
        self.tooling.validate_entries("tooling")?;
        self.recovery.validate()?;
        self.demand.validate()
    }

    /// The explicit saved rate for one source currency, if the project holds one.
    pub fn exchange_rate(&self, source_currency: &str) -> Option<Decimal> {
        self.exchange_rates.iter().find(|rate| rate.source_currency == source_currency).map(|rate| rate.project_units_per_source_unit)
    }
}

// ---------------------------------------------------------------------------
// Evaluation output
// ---------------------------------------------------------------------------

/// One product's share of one tool's investment and contribution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolingAllocationLine {
    pub product_id: Uuid,
    /// The product's baseline lifetime demand; `None` is missing.
    pub product_lifetime_demand: Option<u64>,
    /// The expanded owner quantity in one finished product.
    pub expanded_quantity: u64,
    /// `product_lifetime_demand × expanded_quantity`; `None` when the demand is missing.
    pub product_owner_demand: Option<u64>,
    /// This product's lifetime share of the owner's combined demand.
    pub allocation_share: Option<Decimal>,
    /// This product's share of the tool's project-currency investment.
    pub allocated_investment: Option<Decimal>,
    /// This product's amortized contribution per finished unit (amortized treatment only).
    pub product_contribution: Option<Decimal>,
}

/// One calendar year's recovered amount of one tool (amortized treatment only).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolingAnnualRecovery {
    pub year: u16,
    /// The recovered amount in the project currency; `None` when unavailable.
    pub recovered: Option<Decimal>,
}

/// One tool entry's explained investment, allocation and recovery.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolingToolLine {
    pub tool_id: Uuid,
    pub owner: ToolingOwner,
    pub tool_count: u64,
    /// The cost of one tool in `currency`.
    pub cost_per_tool: Decimal,
    /// The entry currency of `cost_per_tool`.
    pub currency: String,
    /// `tool_count × cost_per_tool` in the entry currency.
    pub source_investment: Option<Decimal>,
    /// The saved rate used, or `None` when the currencies are identical or the rate is missing.
    pub fx_rate: Option<Decimal>,
    /// The whole investment in the project currency, counted once.
    pub investment: Option<Decimal>,
    /// The positive quantity the investment is divided by.
    pub recovery_quantity: Option<Decimal>,
    /// `investment / recovery_quantity` (amortized treatment only).
    pub contribution_per_owner_unit: Option<Decimal>,
    /// One line per quoted product, ordered by product id.
    pub allocation: Vec<ToolingAllocationLine>,
    /// The annual projected recovery, ascending years (amortized treatment only).
    pub annual_recovery: Vec<ToolingAnnualRecovery>,
    /// The lifetime projected recovery (amortized treatment only).
    pub projected_recovery: Option<Decimal>,
    /// `projected_recovery − investment`: positive is over-recovery, negative is under-recovery.
    pub under_over_recovery: Option<Decimal>,
    /// The whole investment as a separate charge (separate treatment only).
    pub separate_charge: Option<Decimal>,
    pub explanations: Vec<CostLine>,
}

/// One quoted product's explained tooling values.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolingProductLine {
    pub product_id: Uuid,
    pub product_revision_id: Option<Uuid>,
    pub lifetime_demand: Option<u64>,
    /// This product's share of the project investment; the separate-charge share under the
    /// separate treatment.
    pub allocated_investment: Option<Decimal>,
    /// The amortized contribution per finished product (amortized treatment only).
    pub amortized_contribution: Option<Decimal>,
    pub explanations: Vec<CostLine>,
}

/// One named problem of a tooling evaluation. Issues are computed on read, never stored as a value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolingIssue {
    pub code: String,
    pub field_path: String,
    pub tool_id: Option<Uuid>,
    pub product_id: Option<Uuid>,
    pub message: String,
}

impl ToolingIssue {
    /// A named issue. `code` is a stable contract name, not display text.
    pub fn new(code: impl Into<String>, field_path: impl Into<String>, message: impl Into<String>) -> Self {
        Self { code: code.into(), field_path: field_path.into(), tool_id: None, product_id: None, message: message.into() }
    }

    /// Attach the tool the issue is about.
    pub fn for_tool(mut self, tool_id: Uuid) -> Self {
        self.tool_id = Some(tool_id);
        self
    }

    /// Attach the product the issue is about.
    pub fn for_product(mut self, product_id: Uuid) -> Self {
        self.product_id = Some(product_id);
        self
    }
}

/// The calculator's explained answer: one line per tool, one per quoted product and the project
/// totals. Investment, amortized recovery revenue and the separate charge stay distinct.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolingOutcome {
    pub state: ResultState,
    /// The selected treatment; `None` when neither the scenario nor the project chose one.
    pub treatment: Option<ToolingTreatment>,
    /// One line per tool entry, ordered by tool id.
    pub tools: Vec<ToolingToolLine>,
    /// One line per quoted product, ordered by product id.
    pub products: Vec<ToolingProductLine>,
    /// `Σ investment[t]` counted once; `"0"` when the project has no tool entries.
    pub project_investment: Option<Decimal>,
    /// `Σ projected_recovery[t]` (amortized treatment only).
    pub project_amortized_recovered: Option<Decimal>,
    /// `Σ under_over_recovery[t]` (amortized treatment only).
    pub project_under_over_recovery: Option<Decimal>,
    /// `Σ separate_charge[t]` (separate treatment only).
    pub project_separate_charge: Option<Decimal>,
    /// The summed per-year recovery, ascending (amortized treatment only).
    pub annual_recovery: Vec<ToolingAnnualRecovery>,
    pub explanations: Vec<CostLine>,
    pub issues: Vec<ToolingIssue>,
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

/// Replace the project's tooling settings: an append-only, expected-version-checked project write.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UpdateProjectTooling {
    pub request_id: String,
    pub expected_version: u64,
    pub tooling: ProjectTooling,
}

impl UpdateProjectTooling {
    /// Validate the request envelope and the settings structurally. The owner references are
    /// validated against the project document by the service before it writes.
    pub fn validate(&self) -> Result<()> {
        check_request_id(&self.request_id)?;
        if self.expected_version == 0 {
            return Err(ContractError::invalid("expected_version", "the expected project version starts at 1"));
        }
        self.tooling.validate_entries("tooling")
    }
}

/// Evaluate tooling investment, allocation and recovery against one immutable baseline snapshot.
///
/// This is a read-only calculation: no request id is needed because nothing is stored and the
/// answer is a deterministic function of the named baseline, the tooling settings of that
/// baseline's project version and these recovery assumptions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvaluateToolingRecovery {
    pub baseline_calculation_id: Uuid,
    #[serde(default)]
    pub recovery: ToolingRecoveryInputs,
}

impl EvaluateToolingRecovery {
    /// Validate the request envelope and the recovery assumptions.
    pub fn validate(&self) -> Result<()> {
        if self.baseline_calculation_id.is_nil() {
            return Err(ContractError::invalid("baseline_calculation_id", "the baseline calculation id is empty"));
        }
        self.recovery.validate()
    }
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

    fn entry(tool: u128, owner: ToolingOwner) -> ToolingEntry {
        ToolingEntry {
            tool_id: id(tool),
            owner,
            name: None,
            tool_count: 2,
            cost_per_tool: decimal("10000"),
            currency: "CNY".into(),
            source_reference: None,
        }
    }

    fn product(product: u128, lifetime: Option<u64>, quantities: Vec<(u128, u64)>) -> ToolingProductDemand {
        ToolingProductDemand {
            product_id: id(product),
            product_revision_id: None,
            lifetime_demand: lifetime,
            forecasts: Vec::new(),
            part_quantities: quantities.into_iter().map(|(part, quantity)| ToolingPartQuantity { part_version_id: id(part), quantity }).collect(),
        }
    }

    #[test]
    fn a_tool_entry_investment_is_count_times_unit_cost() {
        let line = entry(1, ToolingOwner::PartVersion { part_version_id: id(5) });
        assert_eq!(line.source_investment(), Some(decimal("20000")));
    }

    #[test]
    fn an_entry_with_no_tool_or_a_negative_cost_is_refused() {
        let mut zero_count = entry(1, ToolingOwner::PartVersion { part_version_id: id(5) });
        zero_count.tool_count = 0;
        assert!(zero_count.validate("entry").is_err());
        let mut negative = entry(1, ToolingOwner::PartVersion { part_version_id: id(5) });
        negative.cost_per_tool = decimal("-1");
        assert!(negative.validate("entry").is_err());
        let mut bad_currency = entry(1, ToolingOwner::PartVersion { part_version_id: id(5) });
        bad_currency.currency = "cny".into();
        assert!(bad_currency.validate("entry").is_err());
    }

    #[test]
    fn duplicate_tool_ids_and_unknown_owners_are_refused() {
        let first = entry(1, ToolingOwner::PartVersion { part_version_id: id(5) });
        let second = entry(1, ToolingOwner::Product { product_id: id(7) });
        let mut tooling = ProjectTooling { default_treatment: Some(ToolingTreatment::Amortized), entries: vec![first, second] };
        assert!(tooling.validate_entries("tooling").is_err());

        let unknown_owner = entry(2, ToolingOwner::Product { product_id: id(9) });
        tooling.entries = vec![unknown_owner];
        assert!(tooling.validate("CNY", &[id(5)], &[id(7)]).is_err());
        tooling.entries[0].owner = ToolingOwner::PartVersion { part_version_id: id(5) };
        assert!(tooling.validate("CNY", &[id(5)], &[id(7)]).is_ok());
    }

    #[test]
    fn the_optional_tooling_field_reads_as_missing_on_an_earlier_document() {
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
        let document: crate::projects::ProjectDocument = serde_json::from_value(earlier).expect("a document written before SB-17 parses");
        assert!(document.tooling.is_none());
        assert!(document.validate().is_ok());
    }

    #[test]
    fn the_owner_wire_shape_is_tagged_and_reads_back() {
        let part = serde_json::to_value(ToolingOwner::PartVersion { part_version_id: id(5) }).expect("the owner wire value");
        assert_eq!(part["kind"], "part_version");
        let product = serde_json::to_value(ToolingOwner::Product { product_id: id(7) }).expect("the owner wire value");
        assert_eq!(product["kind"], "product");
        let manual = serde_json::to_value(ToolingRecoveryBasis::ManualQuantity { quantity: Some(decimal("5000")) }).expect("the basis wire value");
        assert_eq!(manual["kind"], "manual_quantity");
        let missing: ToolingRecoveryBasis =
            serde_json::from_value(serde_json::json!({ "kind": "manual_quantity" })).expect("a manual basis without a quantity parses");
        assert_eq!(missing, ToolingRecoveryBasis::ManualQuantity { quantity: None });
    }

    #[test]
    fn the_recovery_input_defaults_to_lifetime_demand_and_refuses_a_non_positive_manual_quantity() {
        let mut recovery = ToolingRecoveryInputs::default();
        assert_eq!(recovery.basis_for(&id(1)), ToolingRecoveryBasis::LifetimeDemand);
        recovery.recovery =
            vec![ToolingRecoveryChoice { tool_id: id(1), basis: ToolingRecoveryBasis::ManualQuantity { quantity: Some(decimal("0")) } }];
        assert!(recovery.validate().is_err());
        recovery.recovery[0].basis = ToolingRecoveryBasis::ManualQuantity { quantity: Some(decimal("5000")) };
        assert!(recovery.validate().is_ok());
        assert_eq!(recovery.basis_for(&id(1)), ToolingRecoveryBasis::ManualQuantity { quantity: Some(decimal("5000")) });
        recovery.recovery.push(recovery.recovery[0]);
        assert!(recovery.validate().is_err());
    }

    #[test]
    fn a_duplicate_product_or_part_quantity_is_refused() {
        let mut demand = ToolingDemand { products: vec![product(1, Some(100), vec![(5, 2)]), product(1, Some(100), vec![])] };
        assert!(demand.validate().is_err());
        demand.products.pop();
        demand.products[0].part_quantities = vec![ToolingPartQuantity { part_version_id: id(5), quantity: 0 }];
        assert!(demand.validate().is_err());
        demand.products[0].part_quantities = vec![ToolingPartQuantity { part_version_id: id(5), quantity: 2 }];
        assert!(demand.validate().is_ok());
        demand.products[0].part_quantities =
            vec![ToolingPartQuantity { part_version_id: id(6), quantity: 1 }, ToolingPartQuantity { part_version_id: id(5), quantity: 2 }];
        assert!(demand.validate().is_err(), "part quantities are ordered by ascending part version id");
        demand.products[0].part_quantities = vec![ToolingPartQuantity { part_version_id: id(5), quantity: 2 }];
        demand.products = vec![product(2, Some(10), vec![]), product(1, Some(10), vec![])];
        assert!(demand.validate().is_err(), "products are ordered by ascending product id");
        demand.products = vec![product(1, Some(10), vec![]), product(2, Some(10), vec![])];
        assert!(demand.validate().is_ok());
        demand.products[0].lifetime_demand = Some(0);
        assert!(demand.validate().is_err());
    }

    #[test]
    fn the_identity_rate_and_duplicate_rates_are_refused() {
        let mut inputs = ToolingInputs {
            project_currency: "CNY".into(),
            exchange_rates: vec![ExchangeRate { source_currency: "CNY".into(), project_units_per_source_unit: decimal("1") }],
            tooling: ProjectTooling::default(),
            recovery: ToolingRecoveryInputs::default(),
            demand: ToolingDemand::default(),
        };
        assert!(inputs.validate().is_err());
        inputs.exchange_rates = vec![
            ExchangeRate { source_currency: "EUR".into(), project_units_per_source_unit: decimal("7.2") },
            ExchangeRate { source_currency: "EUR".into(), project_units_per_source_unit: decimal("7.3") },
        ];
        assert!(inputs.validate().is_err());
        inputs.exchange_rates.truncate(1);
        assert!(inputs.validate().is_ok());
        assert_eq!(inputs.exchange_rate("EUR"), Some(decimal("7.2")));
    }

    #[test]
    fn a_request_envelope_and_a_recovery_request_are_validated() {
        let request = UpdateProjectTooling { request_id: "r".into(), expected_version: 1, tooling: ProjectTooling::default() };
        assert!(request.validate().is_ok());
        let mut zero_version = request.clone();
        zero_version.expected_version = 0;
        assert!(zero_version.validate().is_err());
        let evaluate = EvaluateToolingRecovery { baseline_calculation_id: id(3), recovery: ToolingRecoveryInputs::default() };
        assert!(evaluate.validate().is_ok());
        let nil = EvaluateToolingRecovery { baseline_calculation_id: Uuid::nil(), recovery: ToolingRecoveryInputs::default() };
        assert!(nil.validate().is_err());
    }

    #[test]
    fn the_outcome_round_trips_with_missing_values_as_null() {
        let outcome = ToolingOutcome {
            state: ResultState::Incomplete,
            treatment: None,
            tools: Vec::new(),
            products: Vec::new(),
            project_investment: Some(decimal("0")),
            project_amortized_recovered: None,
            project_under_over_recovery: None,
            project_separate_charge: None,
            annual_recovery: Vec::new(),
            explanations: Vec::new(),
            issues: vec![ToolingIssue::new("missing_treatment", "recovery.treatment", "no treatment")],
        };
        let text = serde_json::to_string(&outcome).expect("the outcome serializes");
        assert!(text.contains("\"treatment\":null"));
        let back: ToolingOutcome = serde_json::from_str(&text).expect("the outcome round trips");
        assert_eq!(back, outcome);
    }
}
