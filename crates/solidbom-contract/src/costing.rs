//! Shared demand and manufacturing cost baseline (contract version 1, additive to API version 2,
//! package SB-16).
//!
//! The portable vocabulary of the project's costing settings and of the pure shared-demand and
//! manufacturing-cost baseline. The rules are locked in
//! `docs/solidbom/contracts/v2/shared-demand-and-manufacturing-costs.md`; the arithmetic lives in
//! the `solidbom-costing` crate's `baseline` module and the service adapter in
//! `services/solidbom-server/src/costing.rs`.
//!
//! # Missing, zero and invalid values
//!
//! Money, rates, volumes and quantities are [`Decimal`] where a fraction is possible and `u64` for
//! calendar-year demand. A missing value is `None`/`null`; an explicit zero is `Some(Decimal::ZERO)`
//! and serializes as `"0"`. The two are never interchangeable: a missing input is a named issue and
//! an unavailable amount, never a zero.
//!
//! # Boundaries
//!
//! This module carries only the resolved inputs and outputs of one baseline calculation and the
//! project costing settings. It stores no database identity, no wall clock and no reference lookup.
//! The service resolves stored records into [`BaselineInputs`]; the calculator is pure.
//!
//! # Versioning
//!
//! The calculator version string is published by the calculator crate
//! (`solidbom-manufacturing-baseline/1`). Changing a formula, a rounding rule, a unit or the
//! supported scope needs a new calculator version; adding an optional field or a named issue for an
//! input that already had no usable result does not change a valid input's arithmetic.

use std::collections::BTreeSet;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{ContractError, Result};
use crate::projects::{AnnualForecast, YearRange};
use crate::shared_data::{AppConfigVersion, CostCategory};
use crate::types::{CostLine, CycleBasis, ResultState, Sourcing, Unit, check_text};

/// The most saved exchange rates one project costing record may hold.
pub const MAX_EXCHANGE_RATES: usize = 64;
/// The most purchase price tables one project costing record may hold.
pub const MAX_PURCHASE_PRICE_TABLES: usize = 10_000;
/// The most tiers one purchase price table may hold.
pub const MAX_PURCHASE_TIERS: usize = 1_000;
/// The most shared part versions one baseline input may cost.
pub const MAX_BASELINE_PARTS: usize = 10_000;
/// The most products one baseline input may roll up.
pub const MAX_BASELINE_PRODUCTS: usize = 1_000;
/// The most operations one part or product routing may hold.
pub const MAX_BASELINE_OPERATIONS: usize = 10_000;
/// The most attached non-CAD cost lines one owner may hold.
pub const MAX_BASELINE_ATTACHED_LINES: usize = 10_000;
/// The most occurrences one resolved product structure may hold.
pub const MAX_STRUCTURE_OCCURRENCES: usize = 100_000;
/// The deepest resolved product structure this contract accepts.
pub const MAX_STRUCTURE_DEPTH: usize = 64;
/// The most characters a calculation request id may carry.
pub const MAX_BASELINE_REQUEST_ID_CHARS: usize = 128;
/// The most calendar years one product schedule may cover, as the version 2 project contract says.
pub const MAX_BASELINE_SCHEDULE_YEARS: usize = 200;

// ---------------------------------------------------------------------------
// Project costing settings (additive to the stored project document)
// ---------------------------------------------------------------------------

/// The representative annual costing-volume method the project selects. Scenarios cannot override
/// it; the value is applied to the summed annual demand, never to individual product peaks.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum CostingVolumePolicy {
    Average,
    Maximum,
    Median,
}

/// One saved manual exchange rate: project-currency units per one source-currency unit.
///
/// The converted amount is `source amount × project_units_per_source_unit`. A positive rate is
/// required for a currency other than the project's; the same currency is the identity and needs no
/// rate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExchangeRate {
    /// The three-letter upper-case currency the rate converts from.
    pub source_currency: String,
    /// Project-currency units per one source-currency unit; strictly positive.
    pub project_units_per_source_unit: Decimal,
}

impl ExchangeRate {
    fn validate(&self, field: &str) -> Result<()> {
        check_currency_code(&self.source_currency, &format!("{field}.source_currency"))?;
        if self.project_units_per_source_unit <= Decimal::ZERO {
            return Err(ContractError::invalid(
                format!("{field}.project_units_per_source_unit"),
                "an exchange rate must be strictly positive (project units per source unit)",
            ));
        }
        Ok(())
    }
}

/// One purchase price tier: a minimum annual quantity and the unit price that covers it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct PurchasePriceTier {
    /// The smallest representative annual volume this tier covers; at least one.
    pub minimum_annual_quantity: u64,
    /// The unit price in the table's currency; zero is an explicit price (a free or included item).
    pub unit_price: Decimal,
}

impl PurchasePriceTier {
    fn validate(&self, field: &str) -> Result<()> {
        if self.minimum_annual_quantity == 0 {
            return Err(ContractError::invalid(format!("{field}.minimum_annual_quantity"), "a purchase tier needs a minimum quantity of at least 1"));
        }
        if self.unit_price.is_sign_negative() {
            return Err(ContractError::invalid(format!("{field}.unit_price"), "a purchase tier price cannot be negative"));
        }
        Ok(())
    }
}

/// The project's agreed purchase price tiers of one part version.
///
/// The confirmed catalog single price stays the catalog's value; a tier table adds the agreed
/// price breaks without rewriting the catalog. The tier with the greatest minimum annual quantity
/// not exceeding the representative costing volume selects the price; an uncovered volume falls
/// back to the catalog single price, and a purchase without either is missing pricing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PurchasePriceTable {
    pub part_version_id: Uuid,
    /// The three-letter upper-case currency of the tier prices.
    pub currency: String,
    pub tiers: Vec<PurchasePriceTier>,
    pub source_reference: Option<String>,
}

impl PurchasePriceTable {
    fn validate(&self, field: &str) -> Result<()> {
        if self.part_version_id.is_nil() {
            return Err(ContractError::invalid(format!("{field}.part_version_id"), "the part version id is empty"));
        }
        check_currency_code(&self.currency, &format!("{field}.currency"))?;
        if self.tiers.len() > MAX_PURCHASE_TIERS {
            return Err(ContractError::invalid(format!("{field}.tiers"), format!("a purchase price table holds at most {MAX_PURCHASE_TIERS} tiers")));
        }
        if let Some(reference) = &self.source_reference {
            check_text(reference, &format!("{field}.source_reference"))?;
        }
        let mut previous: Option<u64> = None;
        for (index, tier) in self.tiers.iter().enumerate() {
            tier.validate(&format!("{field}.tiers[{index}]"))?;
            if let Some(previous) = previous
                && tier.minimum_annual_quantity <= previous
            {
                return Err(ContractError::invalid(
                    format!("{field}.tiers"),
                    "purchase tiers are ordered by a strictly increasing unique minimum quantity",
                ));
            }
            previous = Some(tier.minimum_annual_quantity);
        }
        Ok(())
    }

    /// The tier with the greatest minimum quantity not exceeding `volume`, if one covers it.
    pub fn covering_tier(&self, volume: Decimal) -> Option<&PurchasePriceTier> {
        self.tiers.iter().rev().find(|tier| Decimal::from(tier.minimum_annual_quantity) <= volume)
    }
}

/// The project's costing settings. The volume policy is required before a representative volume
/// can be selected; the exchange rates and purchase tiers are optional.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectCosting {
    #[serde(default)]
    pub volume_policy: Option<CostingVolumePolicy>,
    #[serde(default)]
    pub exchange_rates: Vec<ExchangeRate>,
    #[serde(default)]
    pub purchase_prices: Vec<PurchasePriceTable>,
}

impl ProjectCosting {
    /// Validate the settings against the project currency and the project's part versions.
    ///
    /// An exchange rate for the project currency itself is refused: the conversion is the identity,
    /// so a saved identity rate would only invite a differing value. Every purchase tier table must
    /// name a part version the project holds.
    pub fn validate(&self, project_currency: &str, part_version_ids: &[Uuid]) -> Result<()> {
        check_currency_code(project_currency, "project.currency")?;
        if self.exchange_rates.len() > MAX_EXCHANGE_RATES {
            return Err(ContractError::invalid(
                "project.costing.exchange_rates",
                format!("a project holds at most {MAX_EXCHANGE_RATES} saved exchange rates"),
            ));
        }
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for (index, rate) in self.exchange_rates.iter().enumerate() {
            rate.validate(&format!("project.costing.exchange_rates[{index}]"))?;
            if rate.source_currency == project_currency {
                return Err(ContractError::invalid(
                    format!("project.costing.exchange_rates[{index}].source_currency"),
                    "the project currency converts to itself; remove the identity rate",
                ));
            }
            if !seen.insert(rate.source_currency.as_str()) {
                return Err(ContractError::invalid(
                    format!("project.costing.exchange_rates[{index}].source_currency"),
                    "two saved rates name one source currency",
                ));
            }
        }
        if self.purchase_prices.len() > MAX_PURCHASE_PRICE_TABLES {
            return Err(ContractError::invalid(
                "project.costing.purchase_prices",
                format!("a project holds at most {MAX_PURCHASE_PRICE_TABLES} purchase price tables"),
            ));
        }
        let mut parts: BTreeSet<Uuid> = BTreeSet::new();
        for (index, table) in self.purchase_prices.iter().enumerate() {
            table.validate(&format!("project.costing.purchase_prices[{index}]"))?;
            if !part_version_ids.contains(&table.part_version_id) {
                return Err(ContractError::invalid(
                    format!("project.costing.purchase_prices[{index}].part_version_id"),
                    "the purchase price table names a part version this project does not have",
                ));
            }
            if !parts.insert(table.part_version_id) {
                return Err(ContractError::invalid(
                    format!("project.costing.purchase_prices[{index}].part_version_id"),
                    "two purchase price tables name one part version",
                ));
            }
        }
        Ok(())
    }

    /// The saved rate converting `source_currency` to the project currency, when one is needed.
    ///
    /// The project currency itself is the identity and returns `Some(Decimal::ONE)`.
    pub fn exchange_rate(&self, project_currency: &str, source_currency: &str) -> Option<Decimal> {
        if source_currency == project_currency {
            return Some(Decimal::ONE);
        }
        self.exchange_rates.iter().find(|rate| rate.source_currency == source_currency).map(|rate| rate.project_units_per_source_unit)
    }

    /// The purchase price table of one part version, when the project saved one.
    pub fn purchase_table(&self, part_version_id: Uuid) -> Option<&PurchasePriceTable> {
        self.purchase_prices.iter().find(|table| table.part_version_id == part_version_id)
    }
}

/// Replace the project's costing settings. An accepted write appends one project record version.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateProjectCosting {
    pub request_id: String,
    /// The project record version the write expects; a stale value is a conflict.
    pub expected_version: u64,
    pub costing: ProjectCosting,
}

impl UpdateProjectCosting {
    /// Validate the request envelope. The store validates the settings against the project
    /// currency and part versions after loading the current document.
    pub fn validate(&self) -> Result<()> {
        check_request_id(&self.request_id)?;
        if self.expected_version == 0 {
            return Err(ContractError::invalid("update_project_costing.expected_version", "the expected version starts at 1"));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Resolved baseline inputs
// ---------------------------------------------------------------------------

/// The resolved category configuration one baseline reads.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaselineCategoryConfig {
    /// The published app-config version id the categories came from.
    pub config_version_id: Uuid,
    /// The configuration's currency; a fixed amount converts from it to the project currency.
    pub currency: String,
    pub categories: Vec<CostCategory>,
}

impl BaselineCategoryConfig {
    fn validate(&self) -> Result<()> {
        if self.config_version_id.is_nil() {
            return Err(ContractError::invalid("category_config.config_version_id", "the app-config version id is empty"));
        }
        check_currency_code(&self.currency, "category_config.currency")?;
        if self.categories.len() > MAX_PURCHASE_PRICE_TABLES {
            return Err(ContractError::invalid("category_config.categories", "too many cost categories"));
        }
        let config = AppConfigVersion {
            config_version_id: self.config_version_id,
            config_id: self.config_version_id,
            version: 1,
            supersedes: None,
            currency: self.currency.clone(),
            categories: self.categories.clone(),
            source_reference: None,
            created_at: None,
        };
        config.validate()
    }
}

/// The net material consumption of one made part version, with the resolved density and price.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaselineMaterialInput {
    /// The selected immutable material version, retained in the snapshot.
    #[serde(default)]
    pub material_version_id: Option<Uuid>,
    /// The net required consumption before the allowance; `None` is missing, never zero.
    pub net_consumption: Option<Decimal>,
    /// The declared unit of `net_consumption`: a volume in `mm3` or `cm3`.
    pub unit: Unit,
    pub density_g_cm3: Option<Decimal>,
    pub price_per_kg: Option<Decimal>,
    /// The currency of `price_per_kg`.
    pub price_currency: String,
    pub allowance_rate: Option<Decimal>,
}

impl BaselineMaterialInput {
    fn validate(&self, field: &str) -> Result<()> {
        if let Some(material_version_id) = self.material_version_id
            && material_version_id.is_nil()
        {
            return Err(ContractError::invalid(format!("{field}.material_version_id"), "the material version id is empty"));
        }
        if !matches!(self.unit, Unit::MillimetreCubed | Unit::CentimetreCubed) {
            return Err(ContractError::invalid(format!("{field}.unit"), "consumption is a volume in mm3 or cm3"));
        }
        check_non_negative(self.net_consumption, &format!("{field}.net_consumption"))?;
        check_non_negative(self.density_g_cm3, &format!("{field}.density_g_cm3"))?;
        check_non_negative(self.price_per_kg, &format!("{field}.price_per_kg"))?;
        check_non_negative(self.allowance_rate, &format!("{field}.allowance_rate"))?;
        check_currency_code(&self.price_currency, &format!("{field}.price_currency"))
    }
}

/// The resolved production cycle of one routing operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaselineCycleInput {
    /// The pinned production cost-center rate version, retained in the snapshot.
    #[serde(default)]
    pub production_rate_version_id: Option<Uuid>,
    pub seconds: Option<Decimal>,
    pub basis: CycleBasis,
    pub cavities: Option<u32>,
    pub rate_per_hour: Option<Decimal>,
    /// The currency of `rate_per_hour`.
    pub rate_currency: String,
}

impl BaselineCycleInput {
    fn validate(&self, field: &str) -> Result<()> {
        if let Some(rate_version_id) = self.production_rate_version_id
            && rate_version_id.is_nil()
        {
            return Err(ContractError::invalid(format!("{field}.production_rate_version_id"), "the production rate version id is empty"));
        }
        check_non_negative(self.seconds, &format!("{field}.seconds"))?;
        if let Some(cavities) = self.cavities {
            if cavities == 0 {
                return Err(ContractError::invalid(format!("{field}.cavities"), "at least one cavity is needed"));
            }
            if !self.basis.adjusts_for_cavities() && cavities > 1 {
                return Err(ContractError::invalid(
                    format!("{field}.cavities"),
                    "a per-piece cycle is not adjusted by cavities; declare the per-cycle basis instead",
                ));
            }
        }
        check_non_negative(self.rate_per_hour, &format!("{field}.rate_per_hour"))?;
        check_currency_code(&self.rate_currency, &format!("{field}.rate_currency"))
    }
}

/// The resolved dedicated setup of one routing operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaselineSetupInput {
    /// The pinned dedicated setup cost-center rate version, retained in the snapshot.
    #[serde(default)]
    pub setup_rate_version_id: Option<Uuid>,
    pub hours: Option<Decimal>,
    pub rate_per_hour: Option<Decimal>,
    /// The currency of `rate_per_hour`.
    pub rate_currency: String,
}

impl BaselineSetupInput {
    fn validate(&self, field: &str) -> Result<()> {
        if let Some(rate_version_id) = self.setup_rate_version_id
            && rate_version_id.is_nil()
        {
            return Err(ContractError::invalid(format!("{field}.setup_rate_version_id"), "the setup rate version id is empty"));
        }
        check_non_negative(self.hours, &format!("{field}.hours"))?;
        check_non_negative(self.rate_per_hour, &format!("{field}.rate_per_hour"))?;
        check_currency_code(&self.rate_currency, &format!("{field}.rate_currency"))
    }
}

/// One resolved routing operation of a part, subassembly or product-root routing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaselineOperationInput {
    pub operation_id: Uuid,
    /// The operation's place in the routing, starting at 1.
    pub sequence: u32,
    /// `None` is a missing cycle (a computed issue), never a zero cycle.
    pub cycle: Option<BaselineCycleInput>,
    /// `None` means no setup applies to this operation.
    pub setup: Option<BaselineSetupInput>,
}

impl BaselineOperationInput {
    fn validate(&self, field: &str) -> Result<()> {
        if self.operation_id.is_nil() {
            return Err(ContractError::invalid(format!("{field}.operation_id"), "the operation id is empty"));
        }
        if self.sequence == 0 {
            return Err(ContractError::invalid(format!("{field}.sequence"), "an operation sequence starts at 1"));
        }
        if let Some(cycle) = &self.cycle {
            cycle.validate(&format!("{field}.cycle"))?;
        }
        if let Some(setup) = &self.setup {
            setup.validate(&format!("{field}.setup"))?;
        }
        Ok(())
    }
}

/// One resolved attached non-CAD cost line. Its unit cost is in the project currency.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaselineAttachedCostInput {
    pub line_id: Uuid,
    pub category: String,
    pub quantity: Option<Decimal>,
    pub unit_cost: Option<Decimal>,
}

impl BaselineAttachedCostInput {
    fn validate(&self, field: &str) -> Result<()> {
        if self.line_id.is_nil() {
            return Err(ContractError::invalid(format!("{field}.line_id"), "the attached cost line id is empty"));
        }
        check_text(&self.category, &format!("{field}.category"))?;
        check_non_negative(self.quantity, &format!("{field}.quantity"))?;
        check_non_negative(self.unit_cost, &format!("{field}.unit_cost"))
    }
}

/// One confirmed catalog single price, retained with its own currency and source version.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaselineCatalogPrice {
    pub amount: Decimal,
    /// The three-letter upper-case currency of `amount`.
    pub currency: String,
    /// The confirmed catalog component version the price came from.
    pub component_version_id: Uuid,
}

/// The resolved purchase price source of one purchased part version.
///
/// The project's agreed tiers and the confirmed catalog single price keep their own currencies: the
/// calculator converts only the source it selects, so a tier table and a catalog price may be stated
/// in different currencies without either being mislabelled.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaselinePurchaseInput {
    /// The confirmed catalog single price; `None` is missing, never zero.
    pub single_price: Option<BaselineCatalogPrice>,
    /// The three-letter upper-case currency of the tier prices.
    pub currency: String,
    /// The purchase unit the price is stated in. The MVP supports one piece per occurrence (`pcs`).
    pub price_unit: String,
    /// The project's agreed tiers; empty when none are saved.
    pub tiers: Vec<PurchasePriceTier>,
}

impl BaselinePurchaseInput {
    fn validate(&self, field: &str) -> Result<()> {
        check_currency_code(&self.currency, &format!("{field}.currency"))?;
        check_text(&self.price_unit, &format!("{field}.price_unit"))?;
        if let Some(single) = &self.single_price {
            check_currency_code(&single.currency, &format!("{field}.single_price.currency"))?;
            check_non_negative(Some(single.amount), &format!("{field}.single_price.amount"))?;
            if single.component_version_id.is_nil() {
                return Err(ContractError::invalid(
                    format!("{field}.single_price.component_version_id"),
                    "the catalog component version id is empty",
                ));
            }
        }
        if self.tiers.len() > MAX_PURCHASE_TIERS {
            return Err(ContractError::invalid(format!("{field}.tiers"), format!("a purchase price table holds at most {MAX_PURCHASE_TIERS} tiers")));
        }
        let mut previous: Option<u64> = None;
        for (index, tier) in self.tiers.iter().enumerate() {
            tier.validate(&format!("{field}.tiers[{index}]"))?;
            if let Some(previous) = previous
                && tier.minimum_annual_quantity <= previous
            {
                return Err(ContractError::invalid(
                    format!("{field}.tiers"),
                    "purchase tiers are ordered by a strictly increasing unique minimum quantity",
                ));
            }
            previous = Some(tier.minimum_annual_quantity);
        }
        Ok(())
    }

    /// The tier with the greatest minimum quantity not exceeding `volume`, if one covers it.
    pub fn covering_tier(&self, volume: Decimal) -> Option<&PurchasePriceTier> {
        self.tiers.iter().rev().find(|tier| Decimal::from(tier.minimum_annual_quantity) <= volume)
    }
}

/// One shared part version as the calculator receives it. Everything is resolved: the calculator
/// performs no reference lookup and no FX lookup beyond the saved rates it is given.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaselinePartInput {
    pub part_version_id: Uuid,
    /// The explicit purchased/manufactured choice; `None` is missing and the cost is unavailable.
    pub sourcing: Option<Sourcing>,
    /// A made part's own material record; `None` means the part has no own material cost (for an
    /// assembly, its content cost comes from its children).
    pub material: Option<BaselineMaterialInput>,
    /// The operations of the routing this part version owns, in any order.
    pub operations: Vec<BaselineOperationInput>,
    /// The attached non-CAD cost lines this part version owns.
    pub attached_costs: Vec<BaselineAttachedCostInput>,
    /// The purchase price source of a purchased part; `None` for a made part.
    pub purchase: Option<BaselinePurchaseInput>,
}

impl BaselinePartInput {
    fn validate(&self, field: &str) -> Result<()> {
        if self.part_version_id.is_nil() {
            return Err(ContractError::invalid(format!("{field}.part_version_id"), "the part version id is empty"));
        }
        if self.sourcing == Some(Sourcing::Manufactured) && self.purchase.is_some() {
            return Err(ContractError::invalid(format!("{field}.purchase"), "a manufactured part has no purchase price source"));
        }
        if self.sourcing == Some(Sourcing::Purchased) && self.material.is_some() {
            return Err(ContractError::invalid(format!("{field}.material"), "a purchased part records no manufacturing material"));
        }
        if let Some(material) = &self.material {
            material.validate(&format!("{field}.material"))?;
        }
        if self.operations.len() > MAX_BASELINE_OPERATIONS {
            return Err(ContractError::invalid(
                format!("{field}.operations"),
                format!("a routing holds at most {MAX_BASELINE_OPERATIONS} operations"),
            ));
        }
        let mut operation_ids = BTreeSet::new();
        for (index, operation) in self.operations.iter().enumerate() {
            operation.validate(&format!("{field}.operations[{index}]"))?;
            if !operation_ids.insert(operation.operation_id) {
                return Err(ContractError::invalid(format!("{field}.operations"), "two operations share one operation id"));
            }
        }
        if self.attached_costs.len() > MAX_BASELINE_ATTACHED_LINES {
            return Err(ContractError::invalid(
                format!("{field}.attached_costs"),
                format!("an owner holds at most {MAX_BASELINE_ATTACHED_LINES} attached cost lines"),
            ));
        }
        let mut line_ids = BTreeSet::new();
        for (index, line) in self.attached_costs.iter().enumerate() {
            line.validate(&format!("{field}.attached_costs[{index}]"))?;
            if !line_ids.insert(line.line_id) {
                return Err(ContractError::invalid(format!("{field}.attached_costs"), "two attached cost lines share one line id"));
            }
        }
        if let Some(purchase) = &self.purchase {
            purchase.validate(&format!("{field}.purchase"))?;
        }
        Ok(())
    }
}

/// One occurrence of the resolved product structure. Each value is one placement; a part that
/// appears twice is two occurrences. Children are the placements directly under this occurrence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaselineOccurrenceInput {
    /// The stable occurrence identity, retained for issue explanations.
    pub occurrence_id: Option<Uuid>,
    /// The resolved shared part version; `None` when the reviewed association is still missing.
    pub part_version_id: Option<Uuid>,
    pub children: Vec<BaselineOccurrenceInput>,
}

impl BaselineOccurrenceInput {
    fn validate(&self, field: &str, depth: usize, count: &mut usize) -> Result<()> {
        if depth >= MAX_STRUCTURE_DEPTH {
            return Err(ContractError::invalid(field, format!("a product structure is at most {MAX_STRUCTURE_DEPTH} levels deep")));
        }
        *count += 1;
        if *count > MAX_STRUCTURE_OCCURRENCES {
            return Err(ContractError::invalid(field, format!("a product structure holds at most {MAX_STRUCTURE_OCCURRENCES} occurrences")));
        }
        if let Some(occurrence_id) = self.occurrence_id
            && occurrence_id.is_nil()
        {
            return Err(ContractError::invalid(format!("{field}.occurrence_id"), "the occurrence id is empty"));
        }
        if let Some(part_version_id) = self.part_version_id
            && part_version_id.is_nil()
        {
            return Err(ContractError::invalid(format!("{field}.part_version_id"), "the part version id is empty"));
        }
        for (index, child) in self.children.iter().enumerate() {
            child.validate(&format!("{field}.children[{index}]"), depth + 1, count)?;
        }
        Ok(())
    }
}

/// One quoted or excluded product as the calculator receives it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaselineProductInput {
    pub product_id: Uuid,
    /// The product revision whose resolved structure the baseline read, for the snapshot record.
    pub product_revision_id: Option<Uuid>,
    /// Only quoted products contribute demand, cost and setup.
    pub quoted: bool,
    /// The product's effective inclusive SOP–EOP calendar-year schedule.
    pub schedule: YearRange,
    /// The positive annual forecasts; a production year without one stays incomplete.
    pub forecasts: Vec<AnnualForecast>,
    /// The operations of the product-root routing this product owns.
    pub root_operations: Vec<BaselineOperationInput>,
    /// The attached non-CAD cost lines the product root owns.
    pub root_attached_costs: Vec<BaselineAttachedCostInput>,
    /// The resolved occurrence roots of the product structure.
    pub occurrences: Vec<BaselineOccurrenceInput>,
}

impl BaselineProductInput {
    fn validate(&self, field: &str, occurrence_count: &mut usize) -> Result<()> {
        if self.product_id.is_nil() {
            return Err(ContractError::invalid(format!("{field}.product_id"), "the product id is empty"));
        }
        if let Some(revision_id) = self.product_revision_id
            && revision_id.is_nil()
        {
            return Err(ContractError::invalid(format!("{field}.product_revision_id"), "the product revision id is empty"));
        }
        validate_schedule(&self.schedule, &format!("{field}.schedule"))?;
        if self.forecasts.len() > MAX_BASELINE_SCHEDULE_YEARS {
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
        for (index, operation) in self.root_operations.iter().enumerate() {
            operation.validate(&format!("{field}.root_operations[{index}]"))?;
        }
        for (index, line) in self.root_attached_costs.iter().enumerate() {
            line.validate(&format!("{field}.root_attached_costs[{index}]"))?;
        }
        for (index, occurrence) in self.occurrences.iter().enumerate() {
            occurrence.validate(&format!("{field}.occurrences[{index}]"), 0, occurrence_count)?;
        }
        Ok(())
    }
}

/// The resolved portable inputs of one baseline calculation.
///
/// The service builds them from the exact project, engineering and manufacturing record versions
/// and the published reference data; the calculator is pure. All selected version ids stay in the
/// inputs so an immutable snapshot remains reproducible.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaselineInputs {
    /// One costing currency per project.
    pub project_currency: String,
    /// The app cost configuration the categories come from; `None` is a missing configuration.
    pub category_config: Option<BaselineCategoryConfig>,
    /// The selected representative volume policy; `None` is missing.
    pub volume_policy: Option<CostingVolumePolicy>,
    /// The saved manual exchange rates.
    pub exchange_rates: Vec<ExchangeRate>,
    /// The shared part versions the products use and their resolved cost definitions.
    pub parts: Vec<BaselinePartInput>,
    /// The products whose demand and rollup are calculated.
    pub products: Vec<BaselineProductInput>,
}

impl BaselineInputs {
    /// Validate the resolved inputs before a calculation or a snapshot write. A broken input is
    /// refused here; the calculator additionally reports an invalid number as a named issue so a
    /// hostile caller never makes it crash.
    pub fn validate(&self) -> Result<()> {
        check_currency_code(&self.project_currency, "project_currency")?;
        if let Some(config) = &self.category_config {
            config.validate()?;
        }
        if self.exchange_rates.len() > MAX_EXCHANGE_RATES {
            return Err(ContractError::invalid("exchange_rates", format!("at most {MAX_EXCHANGE_RATES} saved exchange rates")));
        }
        let mut sources: BTreeSet<&str> = BTreeSet::new();
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
        if self.parts.len() > MAX_BASELINE_PARTS {
            return Err(ContractError::invalid("parts", format!("at most {MAX_BASELINE_PARTS} part versions")));
        }
        let mut part_ids = BTreeSet::new();
        for (index, part) in self.parts.iter().enumerate() {
            part.validate(&format!("parts[{index}]"))?;
            if !part_ids.insert(part.part_version_id) {
                return Err(ContractError::invalid(format!("parts[{index}].part_version_id"), "two part inputs name one part version"));
            }
        }
        if self.products.len() > MAX_BASELINE_PRODUCTS {
            return Err(ContractError::invalid("products", format!("at most {MAX_BASELINE_PRODUCTS} products")));
        }
        let mut product_ids = BTreeSet::new();
        let mut occurrence_count = 0usize;
        for (index, product) in self.products.iter().enumerate() {
            product.validate(&format!("products[{index}]"), &mut occurrence_count)?;
            if !product_ids.insert(product.product_id) {
                return Err(ContractError::invalid(format!("products[{index}].product_id"), "two product inputs name one product"));
            }
        }
        Ok(())
    }

    /// The saved rate converting `source_currency` to the project currency, when one is needed.
    /// The project currency itself is the identity.
    pub fn exchange_rate(&self, source_currency: &str) -> Option<Decimal> {
        if source_currency == self.project_currency {
            return Some(Decimal::ONE);
        }
        self.exchange_rates.iter().find(|rate| rate.source_currency == source_currency).map(|rate| rate.project_units_per_source_unit)
    }

    /// The part input of one shared part version, when the service resolved one.
    pub fn part(&self, part_version_id: Uuid) -> Option<&BaselinePartInput> {
        self.parts.iter().find(|part| part.part_version_id == part_version_id)
    }
}

// ---------------------------------------------------------------------------
// Explained baseline output
// ---------------------------------------------------------------------------

/// One calendar year of a shared part's demand. `None` means the year's demand is unavailable
/// (a missing forecast), never zero.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaselineAnnualDemand {
    pub year: u16,
    pub quantity: Option<u64>,
}

/// One computed problem of a baseline. Issues are computed on read, never stored as a value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaselineIssue {
    pub code: String,
    pub field_path: String,
    pub part_version_id: Option<Uuid>,
    pub product_id: Option<Uuid>,
    pub message: String,
}

impl BaselineIssue {
    /// A named issue. `code` is a stable contract name, not display text.
    pub fn new(code: impl Into<String>, field_path: impl Into<String>, message: impl Into<String>) -> Self {
        Self { code: code.into(), field_path: field_path.into(), part_version_id: None, product_id: None, message: message.into() }
    }

    /// Attach the shared part version the issue is about.
    pub fn for_part(mut self, part_version_id: Uuid) -> Self {
        self.part_version_id = Some(part_version_id);
        self
    }

    /// Attach the product the issue is about.
    pub fn for_product(mut self, product_id: Uuid) -> Self {
        self.product_id = Some(product_id);
        self
    }
}

/// The selected purchase price source of one part version.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BaselinePurchasePriceSource {
    /// The confirmed catalog component version's single price.
    CatalogComponent { component_version_id: Uuid },
    /// The project tier with this minimum annual quantity.
    Tier { minimum_annual_quantity: u64 },
}

/// One selected purchase price, with its source and conversion.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaselineSelectedPurchasePrice {
    pub source: BaselinePurchasePriceSource,
    /// The currency of `source_unit_price`.
    pub currency: String,
    pub source_unit_price: Decimal,
    /// The unit price in the project currency.
    pub project_unit_price: Decimal,
    /// The saved rate used, or `None` when the currencies are identical.
    pub fx_rate: Option<Decimal>,
}

/// One made part's own material cost, with its conversion.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaselineMaterialCost {
    /// The net consumption as recorded, in its declared unit; `None` is missing.
    pub net_volume: Option<Decimal>,
    /// The declared unit of `net_volume` (`mm3` or `cm3`).
    pub net_volume_unit: Unit,
    /// The net consumption converted to `mm3`; `None` is missing.
    pub net_volume_mm3: Option<Decimal>,
    pub net_mass_kg: Option<Decimal>,
    /// The net material cost before the allowance, in the project currency.
    pub net_material_cost: Option<Decimal>,
    pub allowance_rate: Option<Decimal>,
    /// The material price before conversion.
    pub price_per_kg: Option<Decimal>,
    /// The currency of `price_per_kg`.
    pub price_currency: String,
    pub fx_rate: Option<Decimal>,
    /// The allowed material cost in the project currency.
    pub allowed_material_cost: Option<Decimal>,
}

/// One configured cost category's computed value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaselineCategoryLine {
    pub code: String,
    pub name: String,
    /// `None` when the category or one of its basis terms is unavailable.
    pub amount: Option<Decimal>,
}

/// One shared part version's explained baseline.
///
/// The production, attached and total costs are rolled: one occurrence of the part version includes
/// the content of its own non-suppressed children exactly once. The shared values are reused with
/// the occurrence quantity, so a product never receives the part's whole project demand.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaselinePartLine {
    pub part_version_id: Uuid,
    pub sourcing: Option<Sourcing>,
    /// The applicable demand years, sorted; an empty list means the part is not demanded.
    pub annual_demand: Vec<BaselineAnnualDemand>,
    pub lifetime_demand: Option<u64>,
    /// The largest applicable annual demand, when every year is known.
    pub peak_demand: Option<u64>,
    /// The representative annual costing volume under the project's policy.
    pub costing_volume: Option<Decimal>,
    pub purchase_price: Option<BaselineSelectedPurchasePrice>,
    /// The part's own material record and conversion (manufactured parts only).
    pub material: Option<BaselineMaterialCost>,
    /// The rolled material content of one occurrence: own material (or purchase price) plus children.
    pub material_cost: Option<Decimal>,
    /// The rolled production cost of one occurrence: own operations plus children.
    pub production_cost: Option<Decimal>,
    /// The rolled attached non-CAD cost of one occurrence.
    pub attached_cost: Option<Decimal>,
    /// The rolled unit cost of one occurrence: material + production + attached.
    /// Unit-scope configured categories are applied once on the finished product, not here.
    pub unit_cost: Option<Decimal>,
    /// The setup charge of this part version's routing, counted once per project quotation.
    pub setup_charge: Option<Decimal>,
    pub explanations: Vec<CostLine>,
}

/// One part version's expanded quantity inside one product.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaselineProductPartLine {
    pub part_version_id: Uuid,
    /// The number of non-suppressed occurrences in that product's resolved structure.
    pub quantity: u64,
}

/// One quoted product's explained baseline.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaselineProductLine {
    pub product_id: Uuid,
    pub product_revision_id: Option<Uuid>,
    /// The summed positive annual forecasts over the product's schedule.
    pub lifetime_demand: Option<u64>,
    /// The rolled material content of one finished product.
    pub material_cost: Option<Decimal>,
    /// The rolled production content of one finished product, including the product-root routing.
    pub production_cost: Option<Decimal>,
    /// The rolled attached non-CAD cost of one finished product, including the product root.
    pub attached_cost: Option<Decimal>,
    /// The product-root routing's own production cost, retained for the explanation.
    pub root_production_cost: Option<Decimal>,
    /// The product-root attached lines' own cost, retained for the explanation.
    pub root_attached_cost: Option<Decimal>,
    /// The unit-scope configured categories, evaluated once on this finished product unit.
    pub categories: Vec<BaselineCategoryLine>,
    /// The per-unit manufacturing cost: material + production + attached + fixed/percentage
    /// categories. Setup stays outside it.
    pub unit_total: Option<Decimal>,
    /// The recurring unit cost an EBIT calculation uses: material + production + attached +
    /// fixed/percentage categories flagged `included_in_ebit`. Direct categories are always inside.
    pub recurring_unit_cost: Option<Decimal>,
    /// The expanded part quantities inside one finished product, ordered by part version id.
    pub parts: Vec<BaselineProductPartLine>,
    pub explanations: Vec<CostLine>,
}

/// Who owns one setup line.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BaselineSetupOwner {
    PartVersion { part_version_id: Uuid },
    Product { product_id: Uuid },
}

/// One applicable routing's separate setup charge.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaselineSetupLine {
    pub owner: BaselineSetupOwner,
    pub setup_charge: Option<Decimal>,
    pub explanations: Vec<CostLine>,
}

/// One calendar year's project manufacturing cost: quoted product demand × product unit cost.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaselineAnnualProjectCost {
    pub year: u16,
    pub manufacturing_cost: Option<Decimal>,
}

/// The calculator's explained baseline: one line per shared part version, one per quoted product,
/// the separate setup section and the project total.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaselineOutcome {
    pub state: ResultState,
    pub parts: Vec<BaselinePartLine>,
    pub products: Vec<BaselineProductLine>,
    pub setup: Vec<BaselineSetupLine>,
    /// The setup-scope configured categories, evaluated once on the project setup total.
    pub setup_categories: Vec<BaselineCategoryLine>,
    /// The separate setup total, never folded into a unit cost.
    pub setup_total: Option<Decimal>,
    /// The actual calendar-year project cost series: quoted demand × product unit cost per year.
    pub annual: Vec<BaselineAnnualProjectCost>,
    /// The summed annual series plus the separate setup total; the lifetime manufacturing total.
    pub project_total_manufacturing_cost: Option<Decimal>,
    /// The project-level explanation lines (setup total and project total).
    pub explanations: Vec<CostLine>,
    pub issues: Vec<BaselineIssue>,
}

// ---------------------------------------------------------------------------
// Snapshot, calculation request and list
// ---------------------------------------------------------------------------

/// One explicit product revision selection for a baseline calculation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProductRevisionSelection {
    pub product_id: Uuid,
    pub product_revision_id: Uuid,
}

impl ProductRevisionSelection {
    fn validate(&self, field: &str) -> Result<()> {
        if self.product_id.is_nil() {
            return Err(ContractError::invalid(format!("{field}.product_id"), "the product id is empty"));
        }
        if self.product_revision_id.is_nil() {
            return Err(ContractError::invalid(format!("{field}.product_revision_id"), "the product revision id is empty"));
        }
        Ok(())
    }
}

/// Calculate one immutable baseline from an exact project record version.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CalculateBaseline {
    pub request_id: String,
    /// The exact project record version to cost; at least 1.
    pub project_version: u64,
    /// The engineering record version to read; `None` means the current one.
    pub engineering_version: Option<u64>,
    /// The manufacturing record version to read; `None` means the current one.
    pub manufacturing_version: Option<u64>,
    /// One explicit revision per product; a missing product entry uses its active revision. Never
    /// read "the latest" implicitly.
    #[serde(default)]
    pub product_revisions: Vec<ProductRevisionSelection>,
}

impl CalculateBaseline {
    /// Validate the request envelope.
    pub fn validate(&self) -> Result<()> {
        check_request_id(&self.request_id)?;
        if self.project_version == 0 {
            return Err(ContractError::invalid("calculate_baseline.project_version", "a saved project version starts at 1"));
        }
        for (field, version) in [("engineering_version", self.engineering_version), ("manufacturing_version", self.manufacturing_version)] {
            if version == Some(0) {
                return Err(ContractError::invalid(format!("calculate_baseline.{field}"), "a record version starts at 1"));
            }
        }
        let mut products = BTreeSet::new();
        for (index, selection) in self.product_revisions.iter().enumerate() {
            selection.validate(&format!("calculate_baseline.product_revisions[{index}]"))?;
            if !products.insert(selection.product_id) {
                return Err(ContractError::invalid(
                    format!("calculate_baseline.product_revisions[{index}].product_id"),
                    "two selections name one product",
                ));
            }
        }
        Ok(())
    }
}

/// One immutable baseline result. History lives in the snapshots; a later recalculation appends a
/// new one and never rewrites an existing one.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaselineSnapshot {
    pub calculation_id: Uuid,
    pub project_id: Uuid,
    pub project_version: u64,
    pub engineering_version: u64,
    pub manufacturing_version: u64,
    /// The canonical hash of the resolved inputs.
    pub input_hash: String,
    /// The arithmetic version that produced the outputs.
    pub calculator_version: String,
    pub inputs: BaselineInputs,
    pub outputs: BaselineOutcome,
    /// The RFC 3339 UTC timestamp the service recorded.
    pub created_at: Option<String>,
}

/// The compact history row of one baseline snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaselineCalculationSummary {
    pub calculation_id: Uuid,
    pub project_id: Uuid,
    pub project_version: u64,
    pub engineering_version: u64,
    pub manufacturing_version: u64,
    pub calculator_version: String,
    pub state: ResultState,
    pub created_at: Option<String>,
}

/// The baseline history of one project, newest first.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaselineCalculationList {
    pub project_id: Uuid,
    pub calculations: Vec<BaselineCalculationSummary>,
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

/// A three-letter upper-case currency code, as the shared-data contract spells it.
fn check_currency_code(currency: &str, field: &str) -> Result<()> {
    if currency.len() != 3 || !currency.bytes().all(|byte| byte.is_ascii_uppercase()) {
        return Err(ContractError::invalid(field, "expected a three-letter upper-case currency code"));
    }
    Ok(())
}

/// A request id the service can deduplicate: non-blank, bounded, without control characters.
fn check_request_id(request_id: &str) -> Result<()> {
    check_text(request_id, "request_id")?;
    if request_id.chars().count() > MAX_BASELINE_REQUEST_ID_CHARS {
        return Err(ContractError::invalid("request_id", format!("a request id holds at most {MAX_BASELINE_REQUEST_ID_CHARS} characters")));
    }
    if request_id.chars().any(char::is_control) {
        return Err(ContractError::invalid("request_id", "a request id must not contain control characters"));
    }
    Ok(())
}

/// An optional amount must not be negative; a zero is a real value.
fn check_non_negative(value: Option<Decimal>, field: &str) -> Result<()> {
    if let Some(value) = value
        && value.is_sign_negative()
    {
        return Err(ContractError::invalid(field, "a value cannot be negative"));
    }
    Ok(())
}

/// Validate an inclusive four-digit SOP–EOP schedule.
fn validate_schedule(schedule: &YearRange, field: &str) -> Result<()> {
    if !(1900..=9999).contains(&schedule.sop_year) || !(1900..=9999).contains(&schedule.eop_year) {
        return Err(ContractError::invalid(field, "a schedule uses four-digit calendar years"));
    }
    if schedule.eop_year < schedule.sop_year {
        return Err(ContractError::invalid(field, "the end of production is before its start"));
    }
    if schedule.years().count() > MAX_BASELINE_SCHEDULE_YEARS {
        return Err(ContractError::invalid(field, format!("a product schedule covers at most {MAX_BASELINE_SCHEDULE_YEARS} years")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rate(source: &str, value: &str) -> ExchangeRate {
        ExchangeRate { source_currency: source.into(), project_units_per_source_unit: value.parse().unwrap_or_default() }
    }

    fn table(part: Uuid, tiers: Vec<(u64, &str)>) -> PurchasePriceTable {
        PurchasePriceTable {
            part_version_id: part,
            currency: "EUR".into(),
            tiers: tiers
                .into_iter()
                .map(|(minimum, price)| PurchasePriceTier { minimum_annual_quantity: minimum, unit_price: price.parse().unwrap_or_default() })
                .collect(),
            source_reference: None,
        }
    }

    fn operation_input(sequence: u32) -> BaselineOperationInput {
        BaselineOperationInput { operation_id: Uuid::from_u128(u128::from(sequence)), sequence, cycle: None, setup: None }
    }

    fn occurrence(part: u128, children: Vec<BaselineOccurrenceInput>) -> BaselineOccurrenceInput {
        BaselineOccurrenceInput { occurrence_id: Some(Uuid::from_u128(part + 1_000)), part_version_id: Some(Uuid::from_u128(part)), children }
    }

    fn schedule() -> YearRange {
        YearRange { sop_year: 2027, eop_year: 2028 }
    }

    fn product_input(occurrences: Vec<BaselineOccurrenceInput>) -> BaselineProductInput {
        BaselineProductInput {
            product_id: Uuid::from_u128(7),
            product_revision_id: Some(Uuid::from_u128(8)),
            quoted: true,
            schedule: schedule(),
            forecasts: vec![AnnualForecast { year: 2027, quantity: 100 }, AnnualForecast { year: 2028, quantity: 200 }],
            root_operations: Vec::new(),
            root_attached_costs: Vec::new(),
            occurrences,
        }
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

    fn inputs(parts: Vec<BaselinePartInput>, products: Vec<BaselineProductInput>) -> BaselineInputs {
        BaselineInputs {
            project_currency: "CNY".into(),
            category_config: None,
            volume_policy: Some(CostingVolumePolicy::Average),
            exchange_rates: Vec::new(),
            parts,
            products,
        }
    }

    #[test]
    fn costing_settings_refuse_duplicate_identity_and_non_positive_rates() {
        let part_id = Uuid::from_u128(1);
        let empty = ProjectCosting::default();
        assert!(empty.validate("CNY", &[part_id]).is_ok());
        assert!(ProjectCosting { exchange_rates: vec![rate("EUR", "7.8"), rate("EUR", "7.9")], ..Default::default() }.validate("CNY", &[]).is_err());
        assert!(ProjectCosting { exchange_rates: vec![rate("CNY", "1")], ..Default::default() }.validate("CNY", &[]).is_err());
        assert!(ProjectCosting { exchange_rates: vec![rate("EUR", "0")], ..Default::default() }.validate("CNY", &[]).is_err());
        assert!(ProjectCosting { exchange_rates: vec![rate("eur", "7.8")], ..Default::default() }.validate("CNY", &[]).is_err());
    }

    #[test]
    fn costing_settings_require_ordered_tiers_for_a_known_part() {
        let part_id = Uuid::from_u128(1);
        assert!(
            ProjectCosting { purchase_prices: vec![table(part_id, vec![(1, "10"), (100, "8")])], ..Default::default() }
                .validate("CNY", &[part_id])
                .is_ok()
        );
        assert!(
            ProjectCosting { purchase_prices: vec![table(part_id, vec![(100, "8"), (1, "10")])], ..Default::default() }
                .validate("CNY", &[part_id])
                .is_err()
        );
        assert!(ProjectCosting { purchase_prices: vec![table(part_id, vec![(0, "10")])], ..Default::default() }.validate("CNY", &[part_id]).is_err());
        assert!(ProjectCosting { purchase_prices: vec![table(part_id, vec![(1, "-1")])], ..Default::default() }.validate("CNY", &[part_id]).is_err());
        assert!(
            ProjectCosting { purchase_prices: vec![table(part_id, vec![(1, "10")])], ..Default::default() }
                .validate("CNY", &[Uuid::from_u128(2)])
                .is_err()
        );
        assert!(
            ProjectCosting { purchase_prices: vec![table(part_id, vec![]), table(part_id, vec![])], ..Default::default() }
                .validate("CNY", &[part_id])
                .is_err()
        );
    }

    #[test]
    fn a_tier_covers_the_greatest_minimum_not_exceeding_the_volume() {
        let table = table(Uuid::from_u128(1), vec![(1, "10"), (100, "8"), (1_000, "6")]);
        assert_eq!(table.covering_tier("50".parse().unwrap_or_default()).map(|tier| tier.unit_price.to_string()), Some("10".into()));
        assert_eq!(table.covering_tier("100".parse().unwrap_or_default()).map(|tier| tier.unit_price.to_string()), Some("8".into()));
        assert_eq!(table.covering_tier("999.5".parse().unwrap_or_default()).map(|tier| tier.unit_price.to_string()), Some("8".into()));
        assert_eq!(table.covering_tier("1000".parse().unwrap_or_default()).map(|tier| tier.unit_price.to_string()), Some("6".into()));
        assert!(table.covering_tier("0.5".parse().unwrap_or_default()).is_none());
    }

    #[test]
    fn the_saved_rate_lookup_keeps_the_identity_and_the_missing_case_apart() {
        let costing = ProjectCosting { exchange_rates: vec![rate("EUR", "7.8")], ..Default::default() };
        assert_eq!(costing.exchange_rate("CNY", "CNY"), Some(Decimal::ONE));
        assert_eq!(costing.exchange_rate("CNY", "EUR"), Some("7.8".parse().unwrap_or_default()));
        assert_eq!(costing.exchange_rate("CNY", "USD"), None);
    }

    #[test]
    fn update_and_calculation_requests_need_a_request_id_and_a_saved_version() {
        let update = UpdateProjectCosting { request_id: "costing-1".into(), expected_version: 1, costing: ProjectCosting::default() };
        assert!(update.validate().is_ok());
        assert!(UpdateProjectCosting { request_id: " ".into(), ..update.clone() }.validate().is_err());
        assert!(UpdateProjectCosting { expected_version: 0, ..update }.validate().is_err());
        let calculate = CalculateBaseline {
            request_id: "baseline-1".into(),
            project_version: 1,
            engineering_version: None,
            manufacturing_version: None,
            product_revisions: Vec::new(),
        };
        assert!(calculate.validate().is_ok());
        assert!(CalculateBaseline { project_version: 0, ..calculate.clone() }.validate().is_err());
        assert!(CalculateBaseline { engineering_version: Some(0), ..calculate }.validate().is_err());
    }

    #[test]
    fn resolved_inputs_refuse_duplicate_parts_products_and_bad_currencies() {
        let part_id = Uuid::from_u128(1);
        let valid = inputs(vec![part(part_id)], vec![product_input(vec![occurrence(1, Vec::new())])]);
        assert!(valid.validate().is_ok());
        assert!(BaselineInputs { project_currency: "cny".into(), ..valid.clone() }.validate().is_err());
        assert!(BaselineInputs { parts: vec![part(part_id), part(part_id)], ..valid.clone() }.validate().is_err());
        assert!(BaselineInputs { products: vec![product_input(Vec::new()), product_input(Vec::new())], ..valid.clone() }.validate().is_err());
        assert!(BaselineInputs { exchange_rates: vec![rate("CNY", "1")], ..valid.clone() }.validate().is_err());
        assert!(BaselineInputs { exchange_rates: vec![rate("EUR", "7.8"), rate("EUR", "7.9")], ..valid }.validate().is_err());
    }

    #[test]
    fn resolved_part_inputs_refuse_cross_sourcing_fields() {
        let part_id = Uuid::from_u128(1);
        let purchase = BaselinePurchaseInput { single_price: None, currency: "EUR".into(), price_unit: "pcs".into(), tiers: Vec::new() };
        assert!(
            BaselinePartInput { sourcing: Some(Sourcing::Manufactured), purchase: Some(purchase.clone()), ..part(part_id) }
                .validate("parts[0]")
                .is_err()
        );
        let material = BaselineMaterialInput {
            material_version_id: None,
            net_consumption: None,
            unit: Unit::MillimetreCubed,
            density_g_cm3: None,
            price_per_kg: None,
            price_currency: "CNY".into(),
            allowance_rate: None,
        };
        assert!(BaselinePartInput { sourcing: Some(Sourcing::Purchased), material: Some(material), ..part(part_id) }.validate("parts[0]").is_err());
        assert!(BaselinePartInput { sourcing: Some(Sourcing::Purchased), purchase: Some(purchase), ..part(part_id) }.validate("parts[0]").is_ok());
    }

    #[test]
    fn resolved_operations_and_tiers_refuse_invalid_numbers() {
        let part_id = Uuid::from_u128(1);
        let cycle = BaselineCycleInput {
            production_rate_version_id: None,
            seconds: Some("-1".parse().unwrap_or_default()),
            basis: CycleBasis::PerPiece,
            cavities: None,
            rate_per_hour: None,
            rate_currency: "CNY".into(),
        };
        let operation = BaselineOperationInput { cycle: Some(cycle), ..operation_input(1) };
        let part_input = BaselinePartInput { operations: vec![operation], ..part(part_id) };
        assert!(part_input.validate("parts[0]").is_err());
        let cycle = BaselineCycleInput {
            production_rate_version_id: None,
            seconds: Some("10".parse().unwrap_or_default()),
            basis: CycleBasis::PerPiece,
            cavities: Some(2),
            rate_per_hour: None,
            rate_currency: "CNY".into(),
        };
        assert!(
            BaselinePartInput { operations: vec![BaselineOperationInput { cycle: Some(cycle), ..operation_input(1) }], ..part(part_id) }
                .validate("parts[0]")
                .is_err()
        );
        let cycle = BaselineCycleInput {
            production_rate_version_id: None,
            seconds: Some("10".parse().unwrap_or_default()),
            basis: CycleBasis::PerCycle,
            cavities: Some(2),
            rate_per_hour: None,
            rate_currency: "CNY".into(),
        };
        assert!(
            BaselinePartInput { operations: vec![BaselineOperationInput { cycle: Some(cycle), ..operation_input(1) }], ..part(part_id) }
                .validate("parts[0]")
                .is_ok()
        );
        assert!(BaselinePartInput { operations: vec![operation_input(1), operation_input(1)], ..part(part_id) }.validate("parts[0]").is_err());
        let purchase = BaselinePurchaseInput {
            single_price: None,
            currency: "EUR".into(),
            price_unit: "pcs".into(),
            tiers: vec![PurchasePriceTier { minimum_annual_quantity: 1, unit_price: Decimal::ZERO }],
        };
        assert!(BaselinePartInput { sourcing: Some(Sourcing::Purchased), purchase: Some(purchase), ..part(part_id) }.validate("parts[0]").is_ok());
    }

    #[test]
    fn product_validation_keeps_forecasts_inside_the_schedule_and_positive() {
        let mut product = product_input(Vec::new());
        assert!(product.validate("products[0]", &mut 0).is_ok());
        product.forecasts[0].quantity = 0;
        assert!(product.validate("products[0]", &mut 0).is_err());
        let mut product = product_input(Vec::new());
        product.forecasts.push(AnnualForecast { year: 2030, quantity: 1 });
        assert!(product.validate("products[0]", &mut 0).is_err());
        let mut product = product_input(Vec::new());
        product.forecasts[1].year = 2027;
        assert!(product.validate("products[0]", &mut 0).is_err());
    }

    #[test]
    fn a_product_structure_is_bounded_and_its_ids_are_not_empty() {
        let mut product =
            product_input(vec![BaselineOccurrenceInput { occurrence_id: None, part_version_id: Some(Uuid::nil()), children: Vec::new() }]);
        assert!(product.validate("products[0]", &mut 0).is_err());
        product = product_input(vec![occurrence(1, vec![occurrence(2, vec![occurrence(3, Vec::new())])])]);
        assert!(product.validate("products[0]", &mut 0).is_ok());
        let mut deep = occurrence(1, Vec::new());
        for _ in 0..MAX_STRUCTURE_DEPTH {
            deep = BaselineOccurrenceInput { children: vec![deep], ..occurrence(1, Vec::new()) };
        }
        let product = product_input(vec![deep]);
        assert!(product.validate("products[0]", &mut 0).is_err());
    }

    #[test]
    fn inputs_round_trip_decimal_strings_against_null() {
        let mut part_input = part(Uuid::from_u128(1));
        part_input.material = Some(BaselineMaterialInput {
            material_version_id: None,
            net_consumption: Some(Decimal::ZERO),
            unit: Unit::MillimetreCubed,
            density_g_cm3: Some("7.85".parse().unwrap_or_default()),
            price_per_kg: Some("6".parse().unwrap_or_default()),
            price_currency: "CNY".into(),
            allowance_rate: Some("0.02".parse().unwrap_or_default()),
        });
        let value = serde_json::to_value(inputs(vec![part_input], Vec::new())).unwrap_or_default();
        assert_eq!(value["parts"][0]["material"]["net_consumption"], serde_json::json!("0"));
        assert_eq!(value["parts"][0]["material"]["allowance_rate"], serde_json::json!("0.02"));
        assert_eq!(value["category_config"], serde_json::Value::Null);
        let parsed: BaselineInputs = serde_json::from_value(value).unwrap_or_else(|_| panic!("round trip"));
        assert!(parsed.validate().is_ok());
    }

    #[test]
    fn a_snapshot_round_trips_with_its_selected_versions_and_issues() {
        let snapshot = BaselineSnapshot {
            calculation_id: Uuid::from_u128(9),
            project_id: Uuid::from_u128(7),
            project_version: 3,
            engineering_version: 2,
            manufacturing_version: 4,
            input_hash: "a".repeat(64),
            calculator_version: CALCULATOR_VERSION.into(),
            inputs: inputs(vec![part(Uuid::from_u128(1))], Vec::new()),
            outputs: BaselineOutcome {
                state: ResultState::Incomplete,
                parts: Vec::new(),
                products: Vec::new(),
                setup: Vec::new(),
                setup_categories: Vec::new(),
                setup_total: None,
                annual: Vec::new(),
                project_total_manufacturing_cost: None,
                explanations: Vec::new(),
                issues: vec![BaselineIssue::new("missing_volume_policy", "volume_policy", "no policy").for_part(Uuid::from_u128(1))],
            },
            created_at: Some("2026-11-11T00:00:00.000Z".into()),
        };
        let value = serde_json::to_value(&snapshot).unwrap_or_default();
        let parsed: BaselineSnapshot = serde_json::from_value(value).unwrap_or_else(|_| panic!("round trip"));
        assert_eq!(parsed, snapshot);
    }

    /// The calculator version string is owned by the costing crate; the contract's fixtures only
    /// need a stable value.
    const CALCULATOR_VERSION: &str = "solidbom-manufacturing-baseline/1";
}
