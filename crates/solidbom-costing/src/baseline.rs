//! Pure shared-demand and manufacturing-cost baseline calculator (package SB-16).
//!
//! The arithmetic version is [`BASELINE_CALCULATOR_VERSION`]. The published rules are locked in
//! `docs/solidbom/contracts/v2/shared-demand-and-manufacturing-costs.md`; the portable input and
//! output types live in `solidbom_contract::costing`.
//!
//! # Arithmetic in version `solidbom-manufacturing-baseline/1`
//!
//! ```text
//! expanded_quantity[c,p] = non-suppressed occurrences of c in p's resolved structure
//! part_demand[c,y]       = Σ quoted p covering y  volume[p,y] × expanded_quantity[c,p]
//! lifetime_demand[c]     = Σ part_demand[c,y] over the applicable years
//! peak_demand[c]         = maximum part_demand[c, applicable years]
//! costing_volume[c]      = average | maximum | median of part_demand[c, applicable years]
//!
//! net_mass_kg      = net_volume_mm3 × density_g_cm3 / 1_000_000
//! allowed_mass_kg  = net_mass_kg × (1 + allowance_rate)
//! net_material     = allowed_mass_kg × material_price_per_kg (converted)
//! production_cost  = Σ per_piece_seconds × production_rate_per_hour (converted) / 3_600
//! attached_cost    = Σ quantity × unit_cost
//! purchase_price   = covering tier price or the confirmed catalog single price (converted)
//!
//! rolled[c]        = own[c] + Σ children rolled[i] × quantity_in_one_c
//! product_total    = rolled roots + product-root own production/attached
//!                    + fixed/percentage unit categories (once per finished product)
//! setup_charge     = Σ setup_hours × setup_rate_per_hour, once per applicable routing
//! annual_cost[y]   = Σ quoted products unit_total[p] × forecast[p,y]
//! project_total    = Σ annual_cost[y] + setup_total
//! ```
//!
//! A different currency converts through a saved manual rate: project units per source unit. The
//! project currency itself is the identity. No live exchange-rate integration exists.
//!
//! # Missing, zero and invalid values
//!
//! `None` is missing, `Some(Decimal::ZERO)` is an explicit zero; they are never interchangeable. A
//! missing value makes its affected amounts unavailable and produces a named issue whose code and
//! field path are stable contract names. Checked decimal arithmetic turns overflow into an
//! `arithmetic_overflow` issue and an unavailable amount, never a panic. A part version that no
//! quoted product demands may keep unavailable purchase/category values without making the result
//! incomplete: nothing needs that price.
//!
//! # Determinism
//!
//! Parts are ordered by part version id, products by product id, demand years by calendar year and
//! categories by configuration order; issue order is sorted. Decimal values are normalised at the
//! output boundary, so the same [`BaselineInputs`] always produce the same [`BaselineOutcome`]
//! bytes. No hash-map iteration order, clock, locale or environment is read.

use std::collections::{BTreeMap, BTreeSet};

use rust_decimal::Decimal;
use solidbom_contract::{
    AnnualForecast, BaselineAnnualDemand, BaselineAnnualProjectCost, BaselineAttachedCostInput, BaselineCategoryConfig, BaselineCategoryLine,
    BaselineInputs, BaselineIssue, BaselineMaterialCost, BaselineOccurrenceInput, BaselineOperationInput, BaselineOutcome, BaselinePartInput,
    BaselinePartLine, BaselineProductInput, BaselineProductLine, BaselineProductPartLine, BaselinePurchaseInput, BaselinePurchasePriceSource,
    BaselineSelectedPurchasePrice, BaselineSetupLine, BaselineSetupOwner, CategoryBasis, CategoryCalculation, CostLine, CostingVolumePolicy,
    DirectCostInput, ResultState, Sourcing, Unit, YearRange,
};
use uuid::Uuid;

/// The arithmetic version this module publishes, recorded in every immutable snapshot.
pub const BASELINE_CALCULATOR_VERSION: &str = "solidbom-manufacturing-baseline/1";

/// One million, from mm³ and grams to kilograms: 1 cm³ = 1000 mm³, 1 kg = 1000 g.
const MM3_TIMES_GRAMS_PER_KG: u32 = 1_000_000;
/// One cubic centimetre in cubic millimetres.
const MM3_PER_CM3: u32 = 1_000;
const SECONDS_PER_HOUR: u32 = 3_600;
/// The supported purchase unit: one piece per occurrence.
const PURCHASE_UNIT: &str = "pcs";

/// The calculator version recorded in an immutable baseline snapshot.
pub fn baseline_calculator_version() -> &'static str {
    BASELINE_CALCULATOR_VERSION
}

/// Calculate one explained shared-demand and manufacturing-cost baseline.
///
/// Missing inputs stay missing: they produce a named issue and an unavailable amount, never a zero.
/// Arithmetic overflow is a named issue, not a panic. This function performs no rounding beyond
/// `rust_decimal`'s last-representable-digit rounding for a non-terminating division.
pub fn calculate_baseline(input: &BaselineInputs) -> BaselineOutcome {
    let mut state = Run::new(input);
    let demand = state.demand();
    let own = state.own_subtotals(&demand);
    let rolled = state.rollups(&own);
    let setup = state.setup(&rolled);
    let categories = state.category_plan();
    let products = state.products(&rolled, &setup, &categories);
    let annual = state.annual_costs(&products);
    let setup_total = state.setup_total(&setup);
    let setup_categories = state.setup_categories(&categories, setup_total);
    let total = state.project_total(&annual, setup_total);

    let parts = state.part_lines(&demand, &own, &rolled, &setup);
    let every_revision_selected = state.quoted_products.iter().all(|product| product.product_revision_id.is_some());
    let mut outcome = BaselineOutcome {
        state: ResultState::Incomplete,
        parts,
        products,
        setup: setup.lines,
        setup_categories,
        setup_total,
        annual,
        project_total_manufacturing_cost: total,
        explanations: state.project_explanations(setup_total, total),
        issues: std::mem::take(&mut state.issues.issues),
    };
    let complete = outcome.issues.is_empty()
        && outcome.parts.iter().filter(|part| demand.contains_key(&part.part_version_id)).all(|part| part.unit_cost.is_some())
        && outcome
            .products
            .iter()
            .all(|product| product.unit_total.is_some() && product.recurring_unit_cost.is_some() && product.lifetime_demand.is_some())
        && outcome.setup_total.is_some()
        && every_revision_selected
        && outcome.annual.iter().all(|year| year.manufacturing_cost.is_some())
        && outcome.project_total_manufacturing_cost.is_some();
    if complete {
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
    issues: Vec<BaselineIssue>,
    seen: BTreeSet<(String, String)>,
}

impl Issues {
    fn push(&mut self, issue: BaselineIssue) {
        if self.seen.insert((issue.code.clone(), issue.field_path.clone())) {
            self.issues.push(issue);
        }
    }

    fn complex(&mut self, code: &str, field: &str, message: &str) {
        self.push(BaselineIssue::new(code, field, message));
    }

    fn complex_part(&mut self, code: &str, field: &str, message: &str, part_version_id: uuid::Uuid) {
        self.push(BaselineIssue::new(code, field, message).for_part(part_version_id));
    }

    fn complex_product(&mut self, code: &str, field: &str, message: &str, product_id: uuid::Uuid) {
        self.push(BaselineIssue::new(code, field, message).for_product(product_id));
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

fn add_u64(issues: &mut Issues, left: u64, right: u64, field: &str) -> Option<u64> {
    match left.checked_add(right) {
        Some(value) => Some(value),
        None => {
            issues.complex("arithmetic_overflow", field, "the annual quantity exceeds the supported numeric range");
            None
        }
    }
}

fn mul_u64(issues: &mut Issues, quantity: u64, count: u64, field: &str) -> Option<u64> {
    match quantity.checked_mul(count) {
        Some(value) => Some(value),
        None => {
            issues.complex("arithmetic_overflow", field, "the annual demand exceeds the supported numeric range");
            None
        }
    }
}

/// Sum contributions, reporting an overflow; any missing contribution makes the total unavailable.
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

fn saved_rate(issues: &mut Issues, input: &BaselineInputs, source_currency: &str, field: &str) -> Option<Decimal> {
    if source_currency == input.project_currency {
        return Some(Decimal::ONE);
    }
    match input.exchange_rates.iter().find(|rate| rate.source_currency == source_currency) {
        None => {
            issues.complex(
                "missing_exchange_rate",
                field,
                &format!("no saved rate converts {source_currency} to the project currency {}", input.project_currency),
            );
            None
        }
        Some(rate) if rate.project_units_per_source_unit <= Decimal::ZERO => {
            issues.complex("invalid_exchange_rate", field, "a saved exchange rate must be strictly positive");
            None
        }
        Some(rate) => Some(rate.project_units_per_source_unit),
    }
}

fn convert(issues: &mut Issues, input: &BaselineInputs, amount: Decimal, currency: &str, field: &str) -> Option<(Decimal, Option<Decimal>)> {
    if currency == input.project_currency {
        return Some((amount, None));
    }
    let rate = saved_rate(issues, input, currency, field)?;
    let converted = mul(issues, amount, rate, field)?;
    Some((converted, Some(rate)))
}

fn line(code: &str, label: &str, amount: Option<Decimal>, basis: &str) -> CostLine {
    CostLine { code: code.to_string(), label: label.to_string(), amount, basis: basis.to_string() }
}

// ---------------------------------------------------------------------------
// The run
// ---------------------------------------------------------------------------

/// One quoted product with its resolved revision and expansion.
#[derive(Clone)]
struct QuotedProduct {
    product_id: uuid::Uuid,
    product_revision_id: Option<uuid::Uuid>,
    schedule: YearRange,
    forecasts: Vec<AnnualForecast>,
    occurrences: Vec<BaselineOccurrenceInput>,
    counts: BTreeMap<uuid::Uuid, u64>,
    lifetime_demand: Option<u64>,
}

/// The child composition of one shared part version, from its occurrence.
#[derive(Clone)]
struct Composition {
    children: Option<Vec<uuid::Uuid>>,
}

/// One part version's own subtotals (one occurrence, without children).
#[derive(Clone, Default)]
struct OwnSubtotals {
    material: Option<Decimal>,
    production: Option<Decimal>,
    attached: Option<Decimal>,
    material_detail: Option<BaselineMaterialCost>,
    purchase: Option<BaselineSelectedPurchasePrice>,
    explanations: Vec<CostLine>,
}

/// The explanation facts of one computed operation.
struct OperationComputed {
    per_piece_seconds: Decimal,
    seconds: Decimal,
    cost: Decimal,
}

/// One part version's rolled subtotals (one occurrence, including children).
#[derive(Clone, Copy)]
struct Rolled {
    material: Option<Decimal>,
    production: Option<Decimal>,
    attached: Option<Decimal>,
}

impl Rolled {
    fn unit(self) -> Option<Decimal> {
        self.material?.checked_add(self.production?)?.checked_add(self.attached?)
    }
}

/// The setup section and the per-owner charges.
#[derive(Default)]
struct Setup {
    lines: Vec<BaselineSetupLine>,
    part_charges: BTreeMap<uuid::Uuid, Option<Decimal>>,
    root_charges: BTreeMap<uuid::Uuid, Option<Decimal>>,
}

/// The category plan of one run: which categories are unit scope and their computed values.
struct CategoryPlan {
    config: Option<BaselineCategoryConfig>,
    scopes: BTreeMap<String, CategoryScope>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CategoryScope {
    Unit,
    Setup,
    Ambiguous,
}

/// The unit-scope categories evaluated once for one finished product.
#[derive(Clone, Default)]
struct UnitCategories {
    lines: Vec<BaselineCategoryLine>,
    additional: Option<Decimal>,
    recurring_additional: Option<Decimal>,
}

struct Run<'a> {
    input: &'a BaselineInputs,
    issues: Issues,
    quoted_products: Vec<QuotedProduct>,
    compositions: BTreeMap<uuid::Uuid, Composition>,
    purchased: BTreeSet<uuid::Uuid>,
    part_ids: BTreeSet<uuid::Uuid>,
    category_config: Option<BaselineCategoryConfig>,
    category_scopes: BTreeMap<String, CategoryScope>,
}

impl<'a> Run<'a> {
    fn new(input: &'a BaselineInputs) -> Self {
        let purchased: BTreeSet<uuid::Uuid> =
            input.parts.iter().filter(|part| part.sourcing == Some(Sourcing::Purchased)).map(|part| part.part_version_id).collect();
        let mut part_ids: BTreeSet<uuid::Uuid> = input.parts.iter().map(|part| part.part_version_id).collect();
        let mut issues = Issues::default();
        if input.volume_policy.is_none() {
            issues.complex("missing_volume_policy", "volume_policy", "the project never selected a representative costing-volume policy");
        }
        if input.category_config.is_none() {
            issues.complex("missing_cost_config", "category_config", "no published app cost configuration exists");
        }

        // Only quoted products contribute. An explicit revision selection is required; a product
        // without one is incomplete and contributes no demand (never a silently chosen latest draft).
        let mut quoted_products = Vec::new();
        let mut any_quoted = false;
        for product in &input.products {
            if !product.quoted {
                continue;
            }
            any_quoted = true;
            if product.product_revision_id.is_none() {
                issues.complex_product(
                    "missing_revision_selection",
                    &format!("products.{}.product_revision_id", product.product_id),
                    "the quoted product has no selected revision for this calculation",
                    product.product_id,
                );
                quoted_products.push(QuotedProduct {
                    product_id: product.product_id,
                    product_revision_id: None,
                    schedule: product.schedule,
                    forecasts: product.forecasts.clone(),
                    occurrences: Vec::new(),
                    counts: BTreeMap::new(),
                    lifetime_demand: None,
                });
                continue;
            }
            let counts = count_occurrences(&mut issues, product, &purchased);
            part_ids.extend(counts.keys().copied());
            quoted_products.push(QuotedProduct {
                product_id: product.product_id,
                product_revision_id: product.product_revision_id,
                schedule: product.schedule,
                forecasts: product.forecasts.clone(),
                occurrences: product.occurrences.clone(),
                counts,
                lifetime_demand: None,
            });
        }
        if !any_quoted {
            issues.complex("no_quoted_products", "products", "no quoted product contributes to the baseline");
        }
        quoted_products.sort_by_key(|product| product.product_id);
        for (index, product) in input.products.iter().enumerate() {
            let _ = index;
            if product.quoted && product.product_revision_id.is_some() && product.occurrences.is_empty() {
                issues.complex_product(
                    "missing_product_structure",
                    &format!("products.{}.occurrences", product.product_id),
                    "the quoted product's selected revision has no resolved structure",
                    product.product_id,
                );
            }
        }
        let mut compositions: BTreeMap<uuid::Uuid, Composition> = BTreeMap::new();
        for product in &input.products {
            if product.quoted && product.product_revision_id.is_some() {
                collect_compositions(&mut issues, product, &mut compositions);
            }
        }
        let category_config = input.category_config.clone();
        let category_scopes = match &category_config {
            Some(config) => compute_category_scopes(config, &mut issues),
            None => BTreeMap::new(),
        };
        Run { input, issues, quoted_products, compositions, purchased, part_ids, category_config, category_scopes }
    }

    // ---- demand -----------------------------------------------------------

    fn demand(&mut self) -> BTreeMap<uuid::Uuid, PartDemand> {
        for index in 0..self.quoted_products.len() {
            let lifetime = self.product_lifetime(index);
            self.quoted_products[index].lifetime_demand = lifetime;
        }
        let mut years: BTreeMap<uuid::Uuid, BTreeMap<u16, Option<u64>>> = BTreeMap::new();
        for product in &self.quoted_products.clone() {
            for (part_version_id, count) in product.counts.clone() {
                if count == 0 {
                    continue;
                }
                for year in product.schedule.years() {
                    let entry = years.entry(part_version_id).or_default().entry(year).or_insert(Some(0));
                    let forecast = product.forecasts.iter().find(|forecast| forecast.year == year);
                    match forecast {
                        Some(forecast) => {
                            let increment =
                                mul_u64(&mut self.issues, forecast.quantity, count, &format!("products.{}.forecasts.{year}", product.product_id));
                            *entry = match (*entry, increment) {
                                (Some(total), Some(increment)) => {
                                    add_u64(&mut self.issues, total, increment, &format!("parts.{part_version_id}.annual_demand.{year}"))
                                }
                                _ => None,
                            };
                        }
                        None => {
                            self.issues.complex_product(
                                "missing_forecast",
                                &format!("products.{product_id}.forecasts.{year}", product_id = product.product_id),
                                "a production year inside the schedule has no positive forecast; the year's demand stays unavailable",
                                product.product_id,
                            );
                            *entry = None;
                        }
                    }
                }
            }
        }
        let mut result = BTreeMap::new();
        for (part_version_id, year_map) in years {
            let annual: Vec<BaselineAnnualDemand> =
                year_map.iter().map(|(year, quantity)| BaselineAnnualDemand { year: *year, quantity: *quantity }).collect();
            let lifetime = {
                let mut total: Option<u64> = Some(0);
                for year in &annual {
                    total = match (total, year.quantity) {
                        (Some(total), Some(quantity)) => {
                            add_u64(&mut self.issues, total, quantity, &format!("parts.{part_version_id}.lifetime_demand"))
                        }
                        _ => None,
                    };
                }
                total
            };
            let peak = {
                let mut peak: Option<u64> = None;
                let mut every_known = true;
                for year in &annual {
                    match year.quantity {
                        Some(quantity) => peak = Some(peak.map_or(quantity, |current| current.max(quantity))),
                        None => every_known = false,
                    }
                }
                if every_known { peak } else { None }
            };
            let volume = self.representative_volume(part_version_id, &annual, lifetime);
            result.insert(part_version_id, PartDemand { annual, lifetime, peak, volume });
        }
        result
    }

    fn product_lifetime(&mut self, index: usize) -> Option<u64> {
        let product = self.quoted_products.get(index).cloned()?;
        let mut total: Option<u64> = Some(0);
        for year in product.schedule.years() {
            match product.forecasts.iter().find(|forecast| forecast.year == year) {
                Some(forecast) => {
                    total = match total {
                        Some(current) => {
                            add_u64(&mut self.issues, current, forecast.quantity, &format!("products.{}.lifetime_demand", product.product_id))
                        }
                        None => None,
                    };
                }
                None => {
                    self.issues.complex_product(
                        "missing_forecast",
                        &format!("products.{}.forecasts.{year}", product.product_id),
                        "a production year inside the schedule has no positive forecast",
                        product.product_id,
                    );
                    total = None;
                }
            }
        }
        total
    }

    fn representative_volume(&mut self, part_version_id: uuid::Uuid, annual: &[BaselineAnnualDemand], lifetime: Option<u64>) -> Option<Decimal> {
        let policy = self.input.volume_policy?;
        if annual.is_empty() {
            return None;
        }
        let lifetime = lifetime?;
        let mut values: Vec<Decimal> = Vec::with_capacity(annual.len());
        for year in annual {
            values.push(Decimal::from(year.quantity?));
        }
        match policy {
            CostingVolumePolicy::Average => {
                let count = Decimal::from(u64::try_from(values.len()).ok()?);
                if count.is_zero() {
                    return None;
                }
                Decimal::from(lifetime).checked_div(count)
            }
            CostingVolumePolicy::Maximum => values.iter().copied().max(),
            CostingVolumePolicy::Median => {
                values.sort_unstable();
                let middle = values.len() / 2;
                if values.len() % 2 == 1 {
                    values.get(middle).copied()
                } else {
                    let left = values.get(middle.checked_sub(1)?).copied()?;
                    let right = values.get(middle).copied()?;
                    let sum = add(&mut self.issues, left, right, &format!("parts.{part_version_id}.costing_volume"))?;
                    sum.checked_div(Decimal::TWO)
                }
            }
        }
    }

    // ---- own subtotals and rollups ---------------------------------------

    fn own_subtotals(&mut self, demand: &BTreeMap<uuid::Uuid, PartDemand>) -> BTreeMap<uuid::Uuid, OwnSubtotals> {
        let mut result = BTreeMap::new();
        let part_ids: Vec<uuid::Uuid> = self.part_ids.iter().copied().collect();
        for part_version_id in part_ids {
            let Some(part) = self.input.part(part_version_id).cloned() else {
                self.issues.complex_part(
                    "missing_sourcing",
                    &format!("parts.{part_version_id}.sourcing"),
                    "the quoted structure names a part version with no manufacturing record",
                    part_version_id,
                );
                result.insert(part_version_id, OwnSubtotals::default());
                continue;
            };
            let own = self.part_own(&part, demand.get(&part_version_id));
            result.insert(part_version_id, own);
        }
        result
    }

    fn part_own(&mut self, part: &BaselinePartInput, demand: Option<&PartDemand>) -> OwnSubtotals {
        let part_version_id = part.part_version_id;
        let field = |suffix: &str| format!("parts.{part_version_id}.{suffix}");
        if part.sourcing.is_none() {
            self.issues.complex_part(
                "missing_sourcing",
                &field("sourcing"),
                "the part version has no explicit purchased/manufactured choice",
                part_version_id,
            );
        }
        let mut own = OwnSubtotals::default();
        match part.sourcing {
            Some(Sourcing::Manufactured) => match &part.material {
                None => own.material = Some(Decimal::ZERO),
                Some(material) => {
                    let detail = self.material_cost(part_version_id, material, &field);
                    let contribution = detail.allowed_material_cost;
                    let material_unit = detail.net_volume_unit;
                    if let Some(volume) = detail.net_volume {
                        own.explanations.push(line(
                            "material.net_volume",
                            "net consumption",
                            Some(volume),
                            if material_unit == Unit::CentimetreCubed { "net volume in cm3 as recorded" } else { "net volume in mm3 as recorded" },
                        ));
                    }
                    if let Some(mass) = detail.net_mass_kg {
                        own.explanations.push(line("material.net_mass_kg", "net mass", Some(mass), "net volume mm3 × density g/cm3 ÷ 1 000 000"));
                    }
                    if let Some(price) = detail.price_per_kg {
                        own.explanations.push(line(
                            "material.unit_price",
                            "material price per kg",
                            Some(price),
                            "selected material price per kg, source currency",
                        ));
                    }
                    if let Some(net_cost) = detail.net_material_cost {
                        own.explanations.push(line(
                            "material.net_cost",
                            "net material cost",
                            Some(net_cost),
                            "net mass kg × material price per kg, converted",
                        ));
                    }
                    if let Some(allowance) = detail.allowance_rate {
                        own.explanations.push(line(
                            "material.allowance_factor",
                            "allowance factor",
                            Some(allowance + Decimal::ONE),
                            "1 + material allowance rate",
                        ));
                    }
                    if let Some(allowed) = detail.allowed_material_cost {
                        own.explanations.push(line(
                            "material.allowed_cost",
                            "allowed material cost",
                            Some(allowed),
                            "net material cost × allowance factor, once",
                        ));
                    }
                    own.material = contribution;
                    own.material_detail = Some(detail);
                }
            },
            Some(Sourcing::Purchased) => {
                let price = self.purchase_price(part_version_id, part.purchase.as_ref(), demand);
                if let Some(price) = &price {
                    own.explanations.push(line(
                        "purchase.unit_price",
                        "selected purchase price",
                        Some(price.project_unit_price),
                        "tier price or confirmed catalog single price, converted",
                    ));
                    if let Some(rate) = price.fx_rate {
                        own.explanations.push(line(
                            "purchase.fx_rate",
                            "exchange rate",
                            Some(rate),
                            "project-currency units per source-currency unit",
                        ));
                    }
                }
                own.material = price.as_ref().map(|price| price.project_unit_price);
                own.purchase = price;
            }
            None => {}
        }
        let (production, production_lines) = self.operation_production(part_version_id, &part.operations, &field);
        own.production = production;
        own.explanations.extend(production_lines);
        let (attached, attached_lines) = self.attached_cost(part_version_id, &part.attached_costs, &field);
        own.attached = attached;
        own.explanations.extend(attached_lines);
        own
    }

    fn material_cost(
        &mut self,
        part_version_id: Uuid,
        material: &solidbom_contract::BaselineMaterialInput,
        field: &dyn Fn(&str) -> String,
    ) -> BaselineMaterialCost {
        let net_consumption = field_required(
            &mut self.issues,
            part_version_id,
            "missing_consumption",
            "invalid_consumption",
            material.net_consumption,
            &field("material.net_consumption"),
        );
        let mut density = field_required(
            &mut self.issues,
            part_version_id,
            "missing_density",
            "invalid_density",
            material.density_g_cm3,
            &field("material.density_g_cm3"),
        );
        if density.is_some_and(|value| value.is_zero()) {
            self.issues.complex_part("invalid_density", &field("material.density_g_cm3"), "density must be positive", part_version_id);
            density = None;
        }
        let price = field_required(
            &mut self.issues,
            part_version_id,
            "missing_material_price",
            "invalid_material_price",
            material.price_per_kg,
            &field("material.price_per_kg"),
        );
        let allowance = field_required(
            &mut self.issues,
            part_version_id,
            "missing_allowance",
            "invalid_allowance",
            material.allowance_rate,
            &field("material.allowance_rate"),
        );
        let volume_mm3 = match (net_consumption, material.unit) {
            (Some(quantity), Unit::MillimetreCubed) => Some(quantity),
            (Some(quantity), Unit::CentimetreCubed) => {
                mul(&mut self.issues, quantity, Decimal::from(MM3_PER_CM3), &field("material.net_consumption"))
            }
            (Some(_), _) => {
                self.issues.complex_part("invalid_consumption", &field("material.unit"), "consumption is a volume in mm3 or cm3", part_version_id);
                None
            }
            (None, _) => None,
        };
        let net_mass = match (volume_mm3, density) {
            (Some(volume_mm3), Some(density)) => mul(&mut self.issues, volume_mm3, density, &field("material.net_mass_kg"))
                .and_then(|value| value.checked_div(Decimal::from(MM3_TIMES_GRAMS_PER_KG))),
            _ => None,
        };
        let converted_price = match price {
            Some(price) => convert(&mut self.issues, self.input, price, &material.price_currency, &field("material.price_currency")),
            None => None,
        };
        let (project_price, fx_rate) = match converted_price {
            Some((amount, rate)) => (Some(amount), rate),
            None => (None, None),
        };
        let net_cost = match (net_mass, project_price) {
            (Some(net_mass), Some(project_price)) => mul(&mut self.issues, net_mass, project_price, &field("material.net_cost")),
            _ => None,
        };
        let allowed = match (net_cost, allowance) {
            (Some(net_cost), Some(allowance)) => {
                let factor = add(&mut self.issues, Decimal::ONE, allowance, &field("material.allowance_factor"));
                factor.and_then(|factor| mul(&mut self.issues, net_cost, factor, &field("material.allowed_cost")))
            }
            _ => None,
        };
        BaselineMaterialCost {
            net_volume: net_consumption,
            net_volume_unit: material.unit,
            net_volume_mm3: volume_mm3,
            net_mass_kg: net_mass,
            net_material_cost: net_cost,
            allowance_rate: allowance,
            price_per_kg: price,
            price_currency: material.price_currency.clone(),
            fx_rate,
            allowed_material_cost: allowed,
        }
    }

    fn operation_production(
        &mut self,
        part_version_id: Uuid,
        operations: &[BaselineOperationInput],
        field: &dyn Fn(&str) -> String,
    ) -> (Option<Decimal>, Vec<CostLine>) {
        if operations.is_empty() {
            return (Some(Decimal::ZERO), Vec::new());
        }
        let mut sorted: Vec<BaselineOperationInput> = operations.to_vec();
        sorted.sort_by_key(|operation| operation.sequence);
        let mut total: Option<Decimal> = Some(Decimal::ZERO);
        let mut explanations = Vec::new();
        for (position, operation) in sorted.iter().enumerate() {
            let operation_field = |suffix: &str| field(&format!("operations[{position}].{suffix}"));
            match self.operation_cost(part_version_id, operation, &operation_field) {
                Some(computed) => {
                    explanations.push(line(
                        "production.per_piece_seconds",
                        "per-piece cycle",
                        Some(computed.per_piece_seconds),
                        "declared cycle basis; a per-cycle cycle is divided by its cavities",
                    ));
                    explanations.push(line(
                        "production.cost",
                        "operation production cost",
                        Some(computed.cost),
                        "per-piece seconds × production rate ÷ 3 600",
                    ));
                    explanations.push(line("production.cycle_seconds", "declared cycle", Some(computed.seconds), "cycle as recorded"));
                    total = match total {
                        Some(current) => add(&mut self.issues, current, computed.cost, &field("production_cost")),
                        None => None,
                    };
                }
                None => total = None,
            }
        }
        (total, explanations)
    }

    fn operation_cost(
        &mut self,
        part_version_id: Uuid,
        operation: &BaselineOperationInput,
        field: &dyn Fn(&str) -> String,
    ) -> Option<OperationComputed> {
        let cycle = operation.cycle.as_ref()?;
        let seconds = match cycle.seconds {
            Some(seconds) if seconds.is_sign_negative() => {
                self.issues.complex_part("invalid_cycle", &field("cycle.seconds"), "a cycle time cannot be negative", part_version_id);
                return None;
            }
            Some(seconds) => seconds,
            None => {
                self.issues.complex_part("missing_cycle", &field("cycle.seconds"), "the routed operation has no cycle time", part_version_id);
                return None;
            }
        };
        if seconds.is_zero() {
            return Some(OperationComputed { per_piece_seconds: Decimal::ZERO, seconds, cost: Decimal::ZERO });
        }
        let per_piece = if cycle.basis.adjusts_for_cavities() {
            let cavities = match cycle.cavities {
                Some(0) => {
                    self.issues.complex_part(
                        "invalid_cavities",
                        &field("cycle.cavities"),
                        "a per-cycle operation needs at least one cavity",
                        part_version_id,
                    );
                    return None;
                }
                Some(cavities) => Decimal::from(cavities),
                None => {
                    self.issues.complex_part(
                        "missing_cavities",
                        &field("cycle.cavities"),
                        "a per-cycle operation has no cavity count",
                        part_version_id,
                    );
                    return None;
                }
            };
            seconds.checked_div(cavities)?
        } else {
            seconds
        };
        let rate = match cycle.rate_per_hour {
            Some(rate) if rate <= Decimal::ZERO => {
                self.issues.complex_part(
                    "invalid_production_rate",
                    &field("cycle.rate_per_hour"),
                    "a production rate must be positive",
                    part_version_id,
                );
                return None;
            }
            Some(rate) => rate,
            None => {
                self.issues.complex_part(
                    "missing_production_rate",
                    &field("cycle.rate_per_hour"),
                    "the operation has no pinned production rate",
                    part_version_id,
                );
                return None;
            }
        };
        let (project_rate, _) = convert(&mut self.issues, self.input, rate, &cycle.rate_currency, &field("cycle.rate_currency"))?;
        let seconds_cost = mul(&mut self.issues, per_piece, project_rate, &field("cycle"))?;
        let cost = seconds_cost.checked_div(Decimal::from(SECONDS_PER_HOUR))?;
        Some(OperationComputed { per_piece_seconds: per_piece, seconds, cost })
    }

    fn attached_cost(
        &mut self,
        part_version_id: Uuid,
        lines: &[BaselineAttachedCostInput],
        field: &dyn Fn(&str) -> String,
    ) -> (Option<Decimal>, Vec<CostLine>) {
        if lines.is_empty() {
            return (Some(Decimal::ZERO), Vec::new());
        }
        let mut total: Option<Decimal> = Some(Decimal::ZERO);
        let mut explanations = Vec::new();
        for (index, line_input) in lines.iter().enumerate() {
            let line_field = |suffix: &str| field(&format!("attached_costs[{index}].{suffix}"));
            let quantity = match line_input.quantity {
                Some(quantity) if quantity.is_sign_negative() => {
                    self.issues.complex_part("invalid_quantity", &line_field("quantity"), "an attached quantity cannot be negative", part_version_id);
                    None
                }
                Some(quantity) => Some(quantity),
                None => {
                    self.issues.complex_part(
                        "missing_attached_quantity",
                        &line_field("quantity"),
                        "the attached line has no quantity",
                        part_version_id,
                    );
                    None
                }
            };
            let unit_cost = match line_input.unit_cost {
                Some(unit_cost) if unit_cost.is_sign_negative() => {
                    self.issues.complex_part(
                        "invalid_attached_unit_cost",
                        &line_field("unit_cost"),
                        "an attached unit cost cannot be negative",
                        part_version_id,
                    );
                    None
                }
                Some(unit_cost) => Some(unit_cost),
                None => {
                    self.issues.complex_part(
                        "missing_attached_unit_cost",
                        &line_field("unit_cost"),
                        "the attached line has no unit cost",
                        part_version_id,
                    );
                    None
                }
            };
            let contribution = quantity.zip(unit_cost).and_then(|(quantity, unit_cost)| {
                let amount = mul(&mut self.issues, quantity, unit_cost, &line_field("line_total"))?;
                explanations.push(line(
                    &format!("attached.{}", line_input.line_id),
                    &line_input.category,
                    Some(amount),
                    "attached non-CAD line: quantity × unit cost, per owner unit",
                ));
                Some(amount)
            });
            total = match (total, contribution) {
                (Some(current), Some(contribution)) => add(&mut self.issues, current, contribution, &field("attached_cost")),
                _ => None,
            };
        }
        (total, explanations)
    }

    fn purchase_price(
        &mut self,
        part_version_id: uuid::Uuid,
        purchase: Option<&BaselinePurchaseInput>,
        demand: Option<&PartDemand>,
    ) -> Option<BaselineSelectedPurchasePrice> {
        let volume = demand.and_then(|demand| demand.volume);
        let volume = volume?;
        let Some(purchase) = purchase else {
            self.issues.complex_part(
                "missing_purchase_price",
                &format!("parts.{part_version_id}.purchase"),
                "a demanded purchased part has no price source",
                part_version_id,
            );
            return None;
        };
        if purchase.price_unit != PURCHASE_UNIT {
            self.issues.complex_part(
                "unsupported_purchase_unit",
                &format!("parts.{part_version_id}.purchase.price_unit"),
                "the MVP purchase price is stated per one piece occurrence",
                part_version_id,
            );
            return None;
        }
        let (source, amount, currency) = match purchase
            .tiers
            .iter()
            .filter(|tier| Decimal::from(tier.minimum_annual_quantity) <= volume)
            .max_by_key(|tier| tier.minimum_annual_quantity)
        {
            Some(tier) => (
                BaselinePurchasePriceSource::Tier { minimum_annual_quantity: tier.minimum_annual_quantity },
                Some(tier.unit_price),
                purchase.currency.clone(),
            ),
            None => match &purchase.single_price {
                Some(single) => (
                    BaselinePurchasePriceSource::CatalogComponent { component_version_id: single.component_version_id },
                    Some(single.amount),
                    single.currency.clone(),
                ),
                None => {
                    self.issues.complex_part(
                        "missing_purchase_price",
                        &format!("parts.{part_version_id}.purchase"),
                        "no purchase tier covers the representative volume and there is no confirmed catalog single price",
                        part_version_id,
                    );
                    return None;
                }
            },
        };
        let amount = amount?;
        let (project_unit_price, fx_rate) =
            convert(&mut self.issues, self.input, amount, &currency, &format!("parts.{part_version_id}.purchase.currency"))?;
        Some(BaselineSelectedPurchasePrice { source, currency, source_unit_price: amount, project_unit_price, fx_rate })
    }

    fn rollups(&mut self, own: &BTreeMap<uuid::Uuid, OwnSubtotals>) -> BTreeMap<uuid::Uuid, Rolled> {
        let mut memo: BTreeMap<uuid::Uuid, Option<Rolled>> = BTreeMap::new();
        let mut visiting: BTreeSet<uuid::Uuid> = BTreeSet::new();
        for part_version_id in self.part_ids.clone() {
            self.rolled(part_version_id, own, &mut memo, &mut visiting, false);
        }
        memo.into_iter().filter_map(|(key, value)| value.map(|value| (key, value))).collect()
    }

    fn rolled(
        &mut self,
        part_version_id: uuid::Uuid,
        own: &BTreeMap<uuid::Uuid, OwnSubtotals>,
        memo: &mut BTreeMap<uuid::Uuid, Option<Rolled>>,
        visiting: &mut BTreeSet<uuid::Uuid>,
        report_cycle: bool,
    ) -> Option<Rolled> {
        if let Some(value) = memo.get(&part_version_id) {
            return *value;
        }
        if !visiting.insert(part_version_id) {
            if report_cycle {
                self.issues.complex_part(
                    "part_structure_cycle",
                    &format!("parts.{part_version_id}"),
                    "the resolved structure would make one shared part version contain itself",
                    part_version_id,
                );
            }
            return None;
        }
        let result = (|| {
            let own_values = own.get(&part_version_id)?;
            if self.purchased.contains(&part_version_id) {
                return Some(Rolled { material: own_values.material, production: own_values.production, attached: own_values.attached });
            }
            let mut material = own_values.material;
            let mut production = own_values.production;
            let mut attached = own_values.attached;
            let composition = self.compositions.get(&part_version_id).cloned();
            let children = match composition {
                Some(Composition { children: Some(children) }) => children,
                Some(Composition { children: None }) => {
                    material = None;
                    production = None;
                    attached = None;
                    Vec::new()
                }
                None => Vec::new(),
            };
            let mut quantities: BTreeMap<uuid::Uuid, u64> = BTreeMap::new();
            for child in children {
                *quantities.entry(child).or_insert(0) += 1;
            }
            for (child, quantity) in quantities {
                let child_rolled = self.rolled(child, own, memo, visiting, true);
                let (child_material, child_production, child_attached) = match child_rolled {
                    Some(child) => (child.material, child.production, child.attached),
                    None => (None, None, None),
                };
                material = sum_scaled(&mut self.issues, material, child_material, quantity, &format!("parts.{part_version_id}.material_cost"));
                production =
                    sum_scaled(&mut self.issues, production, child_production, quantity, &format!("parts.{part_version_id}.production_cost"));
                attached = sum_scaled(&mut self.issues, attached, child_attached, quantity, &format!("parts.{part_version_id}.attached_cost"));
            }
            Some(Rolled { material, production, attached })
        })();
        visiting.remove(&part_version_id);
        memo.insert(part_version_id, result);
        result
    }

    // ---- categories ------------------------------------------------------

    fn category_plan(&mut self) -> CategoryPlan {
        CategoryPlan { config: self.category_config.clone(), scopes: self.category_scopes.clone() }
    }

    fn unit_categories(&mut self, material: Decimal, production: Decimal) -> UnitCategories {
        let Some(config) = self.category_config.clone() else {
            return UnitCategories { lines: Vec::new(), additional: None, recurring_additional: None };
        };
        let mut values: BTreeMap<String, Option<Decimal>> = BTreeMap::new();
        let direct = Direct { material: Some(material), production: Some(production), setup: None };
        for category in &config.categories {
            self.category_value(&config, &direct, &mut values, &category.code);
        }
        let mut lines = Vec::new();
        let mut additional: Option<Decimal> = Some(Decimal::ZERO);
        let mut recurring: Option<Decimal> = Some(Decimal::ZERO);
        for category in &config.categories {
            if self.category_scopes.get(&category.code).copied().unwrap_or(CategoryScope::Ambiguous) == CategoryScope::Setup {
                continue;
            }
            let amount = values.get(&category.code).copied().flatten();
            lines.push(BaselineCategoryLine { code: category.code.clone(), name: category.name.clone(), amount });
            if !matches!(category.calculation, CategoryCalculation::Direct { .. }) {
                additional = match (additional, amount) {
                    (Some(total), Some(amount)) => Some(total + amount),
                    _ => None,
                };
                if category.included_in_ebit {
                    recurring = match (recurring, amount) {
                        (Some(total), Some(amount)) => Some(total + amount),
                        _ => None,
                    };
                }
            }
        }
        UnitCategories { lines, additional, recurring_additional: recurring }
    }

    fn category_value(
        &mut self,
        config: &BaselineCategoryConfig,
        direct: &Direct,
        values: &mut BTreeMap<String, Option<Decimal>>,
        code: &str,
    ) -> Option<Decimal> {
        if let Some(value) = values.get(code) {
            return *value;
        }
        let category = config.categories.iter().find(|category| category.code == code)?;
        let value = match &category.calculation {
            CategoryCalculation::Direct { input } => match input {
                DirectCostInput::Material => direct.material,
                DirectCostInput::Production => direct.production,
                DirectCostInput::Setup => direct.setup,
            },
            CategoryCalculation::Fixed { amount } => {
                convert(&mut self.issues, self.input, *amount, &config.currency, &format!("category_config.categories.{code}.amount"))
                    .map(|(converted, _)| converted)
            }
            CategoryCalculation::Percentage { basis, rate } => {
                let mut base = if basis.direct_cost {
                    match (direct.material, direct.production) {
                        (Some(material), Some(production)) => {
                            add(&mut self.issues, material, production, &format!("category_config.categories.{code}.basis"))
                        }
                        _ => return None,
                    }
                } else {
                    Some(Decimal::ZERO)
                };
                for named in &basis.categories {
                    let named_value = self.category_value(config, direct, values, named);
                    base = match (base, named_value) {
                        (Some(base), Some(named_value)) => {
                            add(&mut self.issues, base, named_value, &format!("category_config.categories.{code}.basis"))
                        }
                        _ => None,
                    };
                }
                match base {
                    Some(base) => mul(&mut self.issues, base, *rate, &format!("category_config.categories.{code}")),
                    None => None,
                }
            }
        };
        values.insert(code.to_string(), value);
        value
    }

    fn setup_categories(&mut self, plan: &CategoryPlan, setup_total: Option<Decimal>) -> Vec<BaselineCategoryLine> {
        let Some(config) = plan.config.clone() else { return Vec::new() };
        let Some(setup_total) = setup_total else {
            return config
                .categories
                .iter()
                .filter(|category| plan.scopes.get(&category.code).copied() == Some(CategoryScope::Setup))
                .map(|category| BaselineCategoryLine { code: category.code.clone(), name: category.name.clone(), amount: None })
                .collect();
        };
        let direct = Direct { material: None, production: None, setup: Some(setup_total) };
        let mut values: BTreeMap<String, Option<Decimal>> = BTreeMap::new();
        for category in &config.categories {
            self.category_value(&config, &direct, &mut values, &category.code);
        }
        config
            .categories
            .iter()
            .filter(|category| plan.scopes.get(&category.code).copied() == Some(CategoryScope::Setup))
            .map(|category| BaselineCategoryLine {
                code: category.code.clone(),
                name: category.name.clone(),
                amount: values.get(&category.code).copied().flatten(),
            })
            .collect()
    }

    // ---- setup -----------------------------------------------------------

    fn setup(&mut self, rolled: &BTreeMap<uuid::Uuid, Rolled>) -> Setup {
        let mut section = Setup::default();
        let quoted = self.quoted_products.clone();
        let mut applicable: BTreeSet<uuid::Uuid> = BTreeSet::new();
        for product in &quoted {
            applicable.extend(product.counts.keys().copied());
        }
        for part_version_id in &applicable {
            let Some(part) = self.input.part(*part_version_id).cloned() else { continue };
            let field = |suffix: &str| format!("parts.{part_version_id}.{suffix}");
            let charge = self.setup_charge(*part_version_id, &part.operations, &field);
            section.part_charges.insert(*part_version_id, charge);
            section.lines.push(BaselineSetupLine {
                owner: BaselineSetupOwner::PartVersion { part_version_id: *part_version_id },
                setup_charge: charge,
                explanations: vec![line("setup.charge", "setup charge", charge, "Σ setup hours × dedicated setup rate, once per project quotation")],
            });
        }
        let _ = rolled;
        for product in &quoted {
            if product.product_revision_id.is_none() {
                continue;
            }
            let Some(input_product) = self.input.products.iter().find(|candidate| candidate.product_id == product.product_id) else { continue };
            let field = |suffix: &str| format!("products.{}.root_{suffix}", product.product_id);
            let charge = self.setup_charge(product.product_id, &input_product.root_operations, &field);
            section.root_charges.insert(product.product_id, charge);
            section.lines.push(BaselineSetupLine {
                owner: BaselineSetupOwner::Product { product_id: product.product_id },
                setup_charge: charge,
                explanations: vec![line(
                    "setup.charge",
                    "product-root setup charge",
                    charge,
                    "Σ setup hours × dedicated setup rate, once per quoted product",
                )],
            });
        }
        section
    }

    fn setup_charge(&mut self, owner_id: uuid::Uuid, operations: &[BaselineOperationInput], field: &dyn Fn(&str) -> String) -> Option<Decimal> {
        let mut total: Option<Decimal> = Some(Decimal::ZERO);
        let mut sorted: Vec<&BaselineOperationInput> = operations.iter().collect();
        sorted.sort_by_key(|operation| operation.sequence);
        for (position, operation) in sorted.iter().enumerate() {
            let Some(setup) = &operation.setup else { continue };
            let setup_field = |suffix: &str| field(&format!("operations[{position}].setup.{suffix}"));
            let hours = match setup.hours {
                Some(hours) if hours.is_sign_negative() => {
                    self.issues.push(BaselineIssue::new("invalid_setup_hours", setup_field("hours"), "setup time cannot be negative"));
                    None
                }
                Some(hours) => Some(hours),
                None => {
                    self.issues.push(BaselineIssue::new("missing_setup_hours", setup_field("hours"), "the recorded setup has no duration"));
                    None
                }
            };
            let Some(hours) = hours else {
                total = None;
                continue;
            };
            if hours.is_zero() {
                continue;
            }
            let rate = match setup.rate_per_hour {
                Some(rate) if rate <= Decimal::ZERO => {
                    self.issues.push(BaselineIssue::new("invalid_setup_rate", setup_field("rate_per_hour"), "a setup rate must be positive"));
                    None
                }
                Some(rate) => Some(rate),
                None => {
                    self.issues.push(BaselineIssue::new("missing_setup_rate", setup_field("rate_per_hour"), "the setup has no dedicated rate"));
                    None
                }
            };
            let Some(rate) = rate else {
                total = None;
                continue;
            };
            let Some((project_rate, _)) = convert(&mut self.issues, self.input, rate, &setup.rate_currency, &setup_field("rate_currency")) else {
                total = None;
                continue;
            };
            match mul(&mut self.issues, hours, project_rate, &setup_field("charge")) {
                Some(charge) => {
                    total = match total {
                        Some(current) => add(&mut self.issues, current, charge, &field("setup_charge")),
                        None => None,
                    };
                }
                None => total = None,
            }
        }
        let _ = owner_id;
        total
    }

    fn setup_total(&mut self, setup: &Setup) -> Option<Decimal> {
        sum_all(&mut self.issues, setup.lines.iter().map(|line| line.setup_charge), "setup_total")
    }

    // ---- products, annual series and project total -----------------------

    fn products(&mut self, rolled: &BTreeMap<uuid::Uuid, Rolled>, setup: &Setup, categories: &CategoryPlan) -> Vec<BaselineProductLine> {
        let mut products = Vec::new();
        for quoted in &self.quoted_products.clone() {
            let Some(input_product) = self.input.products.iter().find(|candidate| candidate.product_id == quoted.product_id).cloned() else {
                continue;
            };
            let mut explanations = Vec::new();
            let mut roots_material: Option<Decimal> = Some(Decimal::ZERO);
            let mut roots_production: Option<Decimal> = Some(Decimal::ZERO);
            let mut roots_attached: Option<Decimal> = Some(Decimal::ZERO);
            for root in &quoted.occurrences {
                let Some(part_version_id) = root.part_version_id else {
                    roots_material = None;
                    roots_production = None;
                    roots_attached = None;
                    continue;
                };
                let Some(value) = rolled.get(&part_version_id) else {
                    roots_material = None;
                    roots_production = None;
                    roots_attached = None;
                    continue;
                };
                roots_material = match (roots_material, value.material) {
                    (Some(current), Some(amount)) => add(&mut self.issues, current, amount, &format!("products.{}.material", quoted.product_id)),
                    _ => None,
                };
                roots_production = match (roots_production, value.production) {
                    (Some(current), Some(amount)) => add(&mut self.issues, current, amount, &format!("products.{}.production", quoted.product_id)),
                    _ => None,
                };
                roots_attached = match (roots_attached, value.attached) {
                    (Some(current), Some(amount)) => add(&mut self.issues, current, amount, &format!("products.{}.attached", quoted.product_id)),
                    _ => None,
                };
            }
            if quoted.product_revision_id.is_none() {
                roots_material = None;
                roots_production = None;
                roots_attached = None;
            }
            let root_field = |suffix: &str| format!("products.{}.root_{suffix}", quoted.product_id);
            let root_material = if quoted.product_revision_id.is_some() { Some(Decimal::ZERO) } else { None };
            let root_production = if quoted.product_revision_id.is_some() {
                self.operation_production(quoted.product_id, &input_product.root_operations, &|suffix: &str| root_field(suffix)).0
            } else {
                None
            };
            let root_attached = if quoted.product_revision_id.is_some() {
                self.attached_cost(quoted.product_id, &input_product.root_attached_costs, &|suffix: &str| root_field(suffix)).0
            } else {
                None
            };
            let material = zip_sum(&mut self.issues, roots_material, root_material, &format!("products.{}.material", quoted.product_id));
            let production = zip_sum(&mut self.issues, roots_production, root_production, &format!("products.{}.production", quoted.product_id));
            let attached = zip_sum(&mut self.issues, roots_attached, root_attached, &format!("products.{}.attached", quoted.product_id));

            let (category_lines, additional, recurring) = match (material, production, categories.config.is_some()) {
                (Some(material), Some(production), true) => {
                    let unit_categories = self.unit_categories(material, production);
                    (unit_categories.lines, unit_categories.additional, unit_categories.recurring_additional)
                }
                (_, _, false) => (Vec::new(), None, None),
                _ => (Vec::new(), None, None),
            };
            if let Some(material) = material {
                explanations.push(line(
                    "rollup.material",
                    "rolled material",
                    Some(material),
                    "own material + child rolled material × occurrence quantity",
                ));
            }
            if let Some(production) = production {
                explanations.push(line(
                    "rollup.production",
                    "rolled production",
                    Some(production),
                    "own operations + child rolled production × occurrence quantity",
                ));
            }
            if let Some(attached) = attached {
                explanations.push(line(
                    "rollup.attached",
                    "rolled attached cost",
                    Some(attached),
                    "own attached lines + child rolled attached × occurrence quantity",
                ));
            }
            for category in &category_lines {
                explanations.push(line(
                    &format!("category.{code}", code = category.code),
                    &category.name,
                    category.amount,
                    "configured cost category, once per finished product unit",
                ));
            }
            let base = sum_all(&mut self.issues, [material, production, attached], &format!("products.{}.unit_total", quoted.product_id));
            let unit_total = match (base, additional) {
                (Some(base), Some(additional)) => add(&mut self.issues, base, additional, &format!("products.{}.unit_total", quoted.product_id)),
                _ => None,
            };
            let recurring_unit_cost = match (base, recurring) {
                (Some(base), Some(recurring)) => {
                    add(&mut self.issues, base, recurring, &format!("products.{}.recurring_unit_cost", quoted.product_id))
                }
                _ => None,
            };
            if let Some(unit_total) = unit_total {
                explanations.push(line(
                    "product.unit_total",
                    "product unit cost",
                    Some(unit_total),
                    "material + production + attached + fixed/percentage categories",
                ));
            }
            if let Some(recurring_unit_cost) = recurring_unit_cost {
                explanations.push(line(
                    "product.recurring_unit_cost",
                    "recurring unit cost",
                    Some(recurring_unit_cost),
                    "material + production + attached + EBIT-included fixed/percentage categories",
                ));
            }
            let parts = quoted
                .counts
                .iter()
                .filter(|(_, count)| **count > 0)
                .map(|(part_version_id, count)| BaselineProductPartLine { part_version_id: *part_version_id, quantity: *count })
                .collect();
            let _ = setup;
            products.push(BaselineProductLine {
                product_id: quoted.product_id,
                product_revision_id: quoted.product_revision_id,
                lifetime_demand: quoted.lifetime_demand,
                material_cost: material,
                production_cost: production,
                attached_cost: attached,
                root_production_cost: root_production,
                root_attached_cost: root_attached,
                categories: category_lines,
                unit_total,
                recurring_unit_cost,
                parts,
                explanations,
            });
        }
        products
    }

    fn annual_costs(&mut self, products: &[BaselineProductLine]) -> Vec<BaselineAnnualProjectCost> {
        let mut years: BTreeSet<u16> = BTreeSet::new();
        for quoted in &self.quoted_products {
            for year in quoted.schedule.years() {
                years.insert(year);
            }
        }
        let mut annual = Vec::new();
        for year in years {
            let mut cost: Option<Decimal> = Some(Decimal::ZERO);
            for quoted in &self.quoted_products {
                let covers = year >= quoted.schedule.sop_year && year <= quoted.schedule.eop_year;
                if !covers {
                    continue;
                }
                let Some(product) = products.iter().find(|product| product.product_id == quoted.product_id) else {
                    cost = None;
                    continue;
                };
                let Some(unit_total) = product.unit_total else {
                    cost = None;
                    continue;
                };
                let forecast = quoted.forecasts.iter().find(|forecast| forecast.year == year).map(|forecast| forecast.quantity);
                let forecast = match forecast {
                    Some(forecast) => forecast,
                    None => {
                        cost = None;
                        continue;
                    }
                };
                let amount = mul(&mut self.issues, unit_total, Decimal::from(forecast), &format!("annual.{year}"));
                cost = match (cost, amount) {
                    (Some(current), Some(amount)) => add(&mut self.issues, current, amount, &format!("annual.{year}")),
                    _ => None,
                };
            }
            annual.push(BaselineAnnualProjectCost { year, manufacturing_cost: cost });
        }
        annual
    }

    fn project_total(&mut self, annual: &[BaselineAnnualProjectCost], setup_total: Option<Decimal>) -> Option<Decimal> {
        if self.quoted_products.is_empty() {
            return Some(Decimal::ZERO);
        }
        if self.quoted_products.iter().any(|product| product.product_revision_id.is_none()) {
            return None;
        }
        let mut total = sum_all(&mut self.issues, annual.iter().map(|year| year.manufacturing_cost), "project_total_manufacturing_cost")?;
        let setup_total = setup_total?;
        total = add(&mut self.issues, total, setup_total, "project_total_manufacturing_cost")?;
        Some(total)
    }

    // ---- explained part lines -------------------------------------------

    fn part_lines(
        &mut self,
        demand: &BTreeMap<uuid::Uuid, PartDemand>,
        own: &BTreeMap<uuid::Uuid, OwnSubtotals>,
        rolled: &BTreeMap<uuid::Uuid, Rolled>,
        setup: &Setup,
    ) -> Vec<BaselinePartLine> {
        let mut lines = Vec::new();
        for part_version_id in self.part_ids.clone() {
            let demand_line = demand.get(&part_version_id);
            let own_line = own.get(&part_version_id);
            let rolled_line = rolled.get(&part_version_id);
            let mut explanations = Vec::new();
            if let Some(demand) = demand_line {
                for year in &demand.annual {
                    explanations.push(line(
                        &format!("demand.{}", year.year),
                        "annual demand",
                        year.quantity.map(Decimal::from),
                        "Σ quoted product forecast × expanded occurrence quantity, aggregated before policy",
                    ));
                }
                explanations.push(line("demand.lifetime", "lifetime demand", demand.lifetime.map(Decimal::from), "Σ applicable annual demand"));
                explanations.push(line("demand.peak", "peak annual demand", demand.peak.map(Decimal::from), "maximum applicable annual demand"));
                explanations.push(line(
                    "demand.costing_volume",
                    "representative costing volume",
                    demand.volume,
                    "project volume policy over the applicable annual demand",
                ));
            }
            if let Some(own) = own_line {
                explanations.extend(own.explanations.clone());
            }
            if let Some(rolled) = rolled_line {
                explanations.push(line("rollup.material", "rolled material", rolled.material, "own material + child rolled material"));
                explanations.push(line("rollup.production", "rolled production", rolled.production, "own operations + child rolled production"));
                explanations.push(line("rollup.attached", "rolled attached cost", rolled.attached, "own attached lines + child rolled attached"));
                let direct = match (rolled.material, rolled.production) {
                    (Some(material), Some(production)) => {
                        add(&mut self.issues, material, production, &format!("parts.{part_version_id}.direct_cost"))
                    }
                    _ => None,
                };
                explanations.push(line("part.direct_cost", "part direct cost", direct, "rolled material + rolled production"));
            }
            let unit_cost = rolled_line.and_then(|rolled| rolled.unit());
            let setup_charge = setup.part_charges.get(&part_version_id).copied().flatten();
            if let Some(charge) = setup_charge {
                explanations.push(line(
                    "setup.charge",
                    "setup charge",
                    Some(charge),
                    "Σ setup hours × dedicated setup rate, once per project quotation",
                ));
            }
            lines.push(BaselinePartLine {
                part_version_id,
                sourcing: self.input.part(part_version_id).and_then(|part| part.sourcing),
                annual_demand: demand_line.map(|demand| demand.annual.clone()).unwrap_or_default(),
                lifetime_demand: demand_line.and_then(|demand| demand.lifetime),
                peak_demand: demand_line.and_then(|demand| demand.peak),
                costing_volume: demand_line.and_then(|demand| demand.volume),
                purchase_price: own_line.and_then(|own| own.purchase.clone()),
                material: own_line.and_then(|own| own.material_detail.clone()),
                material_cost: rolled_line.and_then(|rolled| rolled.material),
                production_cost: rolled_line.and_then(|rolled| rolled.production),
                attached_cost: rolled_line.and_then(|rolled| rolled.attached),
                unit_cost,
                setup_charge,
                explanations,
            });
        }
        lines
    }

    fn project_explanations(&mut self, setup_total: Option<Decimal>, total: Option<Decimal>) -> Vec<CostLine> {
        let mut lines = Vec::new();
        if let Some(setup_total) = setup_total {
            lines.push(line("setup.total", "setup total", Some(setup_total), "Σ applicable routing setup charges, once per project quotation"));
        }
        if let Some(total) = total {
            lines.push(line("project.total", "project manufacturing total", Some(total), "Σ annual quoted demand × product unit cost + setup total"));
        }
        lines
    }
}

/// One part's demand result.
#[derive(Clone, Default)]
struct PartDemand {
    annual: Vec<BaselineAnnualDemand>,
    lifetime: Option<u64>,
    peak: Option<u64>,
    volume: Option<Decimal>,
}

/// The direct subtotals a category graph is calculated from.
#[derive(Clone, Copy, Default)]
struct Direct {
    material: Option<Decimal>,
    production: Option<Decimal>,
    setup: Option<Decimal>,
}

/// One optional required value: `None` becomes a missing-input issue.
fn field_required(
    issues: &mut Issues,
    part_version_id: uuid::Uuid,
    missing_code: &str,
    invalid_code: &str,
    value: Option<Decimal>,
    field: &str,
) -> Option<Decimal> {
    match value {
        Some(value) if value.is_sign_negative() => {
            issues.complex_part(invalid_code, field, "a value cannot be negative", part_version_id);
            None
        }
        Some(value) => Some(value),
        None => {
            issues.complex_part(missing_code, field, "a required value is missing; it is not zero", part_version_id);
            None
        }
    }
}

/// Add `child × quantity` to a running total, reporting an overflow; a missing term is unavailable.
fn sum_scaled(issues: &mut Issues, total: Option<Decimal>, child: Option<Decimal>, quantity: u64, field: &str) -> Option<Decimal> {
    let child = child?;
    let total = total?;
    let scaled = mul(issues, child, Decimal::from(quantity), field)?;
    add(issues, total, scaled, field)
}

/// Add two optional totals; a missing term is unavailable.
fn zip_sum(issues: &mut Issues, left: Option<Decimal>, right: Option<Decimal>, field: &str) -> Option<Decimal> {
    match (left, right) {
        (Some(left), Some(right)) => add(issues, left, right, field),
        _ => None,
    }
}

/// Compute the scope of every category once: unit scope, setup scope or ambiguous when a
/// percentage basis mixes them.
fn compute_category_scopes(config: &BaselineCategoryConfig, issues: &mut Issues) -> BTreeMap<String, CategoryScope> {
    let mut scopes: BTreeMap<String, CategoryScope> = BTreeMap::new();
    for category in &config.categories {
        category_scope(config, &mut scopes, &category.code, issues);
    }
    scopes
}

fn category_scope(config: &BaselineCategoryConfig, scopes: &mut BTreeMap<String, CategoryScope>, code: &str, issues: &mut Issues) -> CategoryScope {
    if let Some(scope) = scopes.get(code) {
        return *scope;
    }
    let Some(category) = config.categories.iter().find(|category| category.code == code) else {
        issues.complex(
            "ambiguous_category_basis",
            &format!("category_config.categories.{code}"),
            "a category basis names a category the configuration does not define",
        );
        scopes.insert(code.to_string(), CategoryScope::Ambiguous);
        return CategoryScope::Ambiguous;
    };
    let scope = match &category.calculation {
        CategoryCalculation::Direct { input } => match input {
            DirectCostInput::Material | DirectCostInput::Production => CategoryScope::Unit,
            DirectCostInput::Setup => CategoryScope::Setup,
        },
        CategoryCalculation::Fixed { .. } => CategoryScope::Unit,
        CategoryCalculation::Percentage { basis, .. } => percentage_scope(config, scopes, code, basis, issues),
    };
    scopes.insert(code.to_string(), scope);
    scope
}

fn percentage_scope(
    config: &BaselineCategoryConfig,
    scopes: &mut BTreeMap<String, CategoryScope>,
    code: &str,
    basis: &CategoryBasis,
    issues: &mut Issues,
) -> CategoryScope {
    let mut scope = if basis.direct_cost { Some(CategoryScope::Unit) } else { None };
    let mut stack = vec![code.to_string()];
    for named in &basis.categories {
        if stack.contains(named) {
            issues.complex("ambiguous_category_basis", &format!("category_config.categories.{code}"), "the percentage basis is circular");
            return CategoryScope::Ambiguous;
        }
        stack.push(named.clone());
        let named_scope = category_scope(config, scopes, named, issues);
        scope = match (scope, named_scope) {
            (None, named_scope) => Some(named_scope),
            (Some(left), right) if left == right => Some(left),
            _ => Some(CategoryScope::Ambiguous),
        };
    }
    scope.unwrap_or(CategoryScope::Ambiguous)
}

/// Count the non-suppressed occurrences of each part version in one product structure.
fn count_occurrences(issues: &mut Issues, product: &BaselineProductInput, purchased: &BTreeSet<uuid::Uuid>) -> BTreeMap<uuid::Uuid, u64> {
    let mut counts = BTreeMap::new();
    count_nodes(issues, &product.occurrences, false, purchased, &mut counts, product.product_id);
    counts
}

fn count_nodes(
    issues: &mut Issues,
    nodes: &[BaselineOccurrenceInput],
    suppressed: bool,
    purchased: &BTreeSet<uuid::Uuid>,
    counts: &mut BTreeMap<uuid::Uuid, u64>,
    product_id: uuid::Uuid,
) {
    for node in nodes {
        let mut suppressed_below = suppressed;
        match node.part_version_id {
            Some(part_version_id) => {
                if !suppressed {
                    let entry = counts.entry(part_version_id).or_insert(0);
                    *entry = entry.saturating_add(1);
                }
                suppressed_below = suppressed || purchased.contains(&part_version_id);
            }
            None => {
                issues.complex_product(
                    "unresolved_occurrence",
                    &format!("products.{product_id}.occurrences"),
                    "one occurrence's reviewed part association is unresolved; its branch is uncosted",
                    product_id,
                );
            }
        }
        count_nodes(issues, &node.children, suppressed_below, purchased, counts, product_id);
    }
}

/// Collect each shared part version's child composition from its first occurrence in the resolved
/// structures. Two occurrences that disagree leave the composition unavailable.
fn collect_compositions(issues: &mut Issues, product: &BaselineProductInput, compositions: &mut BTreeMap<uuid::Uuid, Composition>) {
    for occurrence in &product.occurrences {
        collect_occurrence_composition(issues, occurrence, compositions, product.product_id);
    }
}

fn collect_occurrence_composition(
    issues: &mut Issues,
    occurrence: &BaselineOccurrenceInput,
    compositions: &mut BTreeMap<uuid::Uuid, Composition>,
    product_id: uuid::Uuid,
) {
    if let Some(part_version_id) = occurrence.part_version_id {
        let mut children = Vec::with_capacity(occurrence.children.len());
        let mut resolved = true;
        for child in &occurrence.children {
            match child.part_version_id {
                Some(child_part) => {
                    children.push(child_part);
                    collect_occurrence_composition(issues, child, compositions, product_id);
                }
                None => {
                    resolved = false;
                    issues.complex_product(
                        "unresolved_occurrence",
                        &format!("products.{product_id}.occurrences.children"),
                        "one occurrence's reviewed part association is unresolved; the parent's composition is unavailable",
                        product_id,
                    );
                }
            }
        }
        let composition = Composition { children: if resolved { Some(children.clone()) } else { None } };
        match compositions.get(&part_version_id) {
            None => {
                compositions.insert(part_version_id, composition);
            }
            Some(existing) => {
                let same = match (&existing.children, &composition.children) {
                    (Some(left), Some(right)) => {
                        let mut left = left.clone();
                        let mut right = right.clone();
                        left.sort_unstable();
                        right.sort_unstable();
                        left == right
                    }
                    _ => existing.children.is_none() && composition.children.is_none(),
                };
                if !same {
                    issues.complex_part(
                        "inconsistent_part_structure",
                        &format!("parts.{part_version_id}"),
                        "two occurrences of one shared part version disagree about their child composition",
                        part_version_id,
                    );
                    compositions.insert(part_version_id, Composition { children: None });
                }
            }
        }
    }
}

fn norm(value: Decimal) -> Decimal {
    value.normalize()
}

fn norm_opt(value: Option<Decimal>) -> Option<Decimal> {
    value.map(norm)
}

/// Normalise every business decimal at the output boundary so repeated runs agree byte for byte.
fn normalize_outcome(outcome: &mut BaselineOutcome) {
    for part in &mut outcome.parts {
        if let Some(material) = &mut part.material {
            material.net_volume = norm_opt(material.net_volume);
            material.net_volume_mm3 = norm_opt(material.net_volume_mm3);
            material.net_mass_kg = norm_opt(material.net_mass_kg);
            material.net_material_cost = norm_opt(material.net_material_cost);
            material.allowance_rate = norm_opt(material.allowance_rate);
            material.price_per_kg = norm_opt(material.price_per_kg);
            material.fx_rate = norm_opt(material.fx_rate);
            material.allowed_material_cost = norm_opt(material.allowed_material_cost);
        }
        if let Some(purchase) = &mut part.purchase_price {
            purchase.source_unit_price = norm(purchase.source_unit_price);
            purchase.project_unit_price = norm(purchase.project_unit_price);
            purchase.fx_rate = norm_opt(purchase.fx_rate);
        }
        part.costing_volume = norm_opt(part.costing_volume);
        part.material_cost = norm_opt(part.material_cost);
        part.production_cost = norm_opt(part.production_cost);
        part.attached_cost = norm_opt(part.attached_cost);
        part.unit_cost = norm_opt(part.unit_cost);
        part.setup_charge = norm_opt(part.setup_charge);
        for explanation in &mut part.explanations {
            explanation.amount = norm_opt(explanation.amount);
        }
    }
    for product in &mut outcome.products {
        product.material_cost = norm_opt(product.material_cost);
        product.production_cost = norm_opt(product.production_cost);
        product.attached_cost = norm_opt(product.attached_cost);
        product.root_production_cost = norm_opt(product.root_production_cost);
        product.root_attached_cost = norm_opt(product.root_attached_cost);
        product.unit_total = norm_opt(product.unit_total);
        product.recurring_unit_cost = norm_opt(product.recurring_unit_cost);
        for category in &mut product.categories {
            category.amount = norm_opt(category.amount);
        }
        for explanation in &mut product.explanations {
            explanation.amount = norm_opt(explanation.amount);
        }
    }
    for line in &mut outcome.setup {
        line.setup_charge = norm_opt(line.setup_charge);
        for explanation in &mut line.explanations {
            explanation.amount = norm_opt(explanation.amount);
        }
    }
    for category in &mut outcome.setup_categories {
        category.amount = norm_opt(category.amount);
    }
    outcome.setup_total = norm_opt(outcome.setup_total);
    for year in &mut outcome.annual {
        year.manufacturing_cost = norm_opt(year.manufacturing_cost);
    }
    outcome.project_total_manufacturing_cost = norm_opt(outcome.project_total_manufacturing_cost);
    for explanation in &mut outcome.explanations {
        explanation.amount = norm_opt(explanation.amount);
    }
}

#[cfg(test)]
#[path = "baseline_tests.rs"]
mod baseline_tests;
