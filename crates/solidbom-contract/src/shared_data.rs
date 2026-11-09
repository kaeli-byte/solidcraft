//! Shared company reference data and the Engineer/Administrator permissions (package SB-09).
//!
//! One organization shares versioned material catalogs, cost-center rates, catalog components and
//! the app cost configuration. Every record is a chain of append-only versions: publishing a new
//! version never edits an older one, and a project keeps referencing the exact version ids it
//! selected. Money is a decimal string; `None` means the value is missing, never zero.
//!
//! Two permission sets authorize the work. An engineer works on projects; an administrator
//! maintains the shared records. One identity can hold both, as [`Identity`] records. The service
//! enforces these rules on every route; a hidden button in a client is never the check.

use std::collections::{BTreeMap, BTreeSet};

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{ContractError, Result};
use crate::types::{CatalogComponentVersion, MaterialVersion, RateVersion, ReferenceData, check_text};

/// The longest shared-record name the contract accepts.
pub const MAX_SHARED_NAME_CHARS: usize = 255;
/// The longest source/version reference a shared record may retain.
pub const MAX_SOURCE_REFERENCE_CHARS: usize = 1024;
/// The most categories one app configuration may define.
pub const MAX_COST_CATEGORIES: usize = 256;
/// The most category codes one percentage basis may name.
pub const MAX_BASIS_CATEGORIES: usize = 256;
/// The most reference changes one shared-data view returns.
pub const MAX_REFERENCE_CHANGES: usize = 100;
/// The longest request id an administrative write accepts.
pub const MAX_SHARED_REQUEST_ID_CHARS: usize = 128;

/// One of the two permission sets. One identity can hold both.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    /// Project work: reviewed inputs, saves, calculations and activation.
    Engineer,
    /// Shared catalogs, cost centers, feasibility rules, routing templates and app configuration.
    Administrator,
}

impl Permission {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Engineer => "engineer",
            Self::Administrator => "administrator",
        }
    }
}

/// Who is calling and which permission sets that identity holds.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Identity {
    pub identity_id: Uuid,
    pub name: String,
    /// Every permission set this identity holds, unique and sorted by the wire name.
    pub permissions: Vec<Permission>,
}

impl Identity {
    /// Whether this identity holds one permission set.
    pub fn has(&self, permission: Permission) -> bool {
        self.permissions.contains(&permission)
    }

    /// Whether this identity may work on projects.
    pub fn can_work_on_projects(&self) -> bool {
        self.has(Permission::Engineer)
    }

    /// Whether this identity may maintain shared records.
    pub fn can_manage_shared_data(&self) -> bool {
        self.has(Permission::Administrator)
    }

    pub fn validate(&self) -> Result<()> {
        check_text(&self.name, "identity.name")?;
        if self.name.chars().count() > MAX_SHARED_NAME_CHARS {
            return Err(ContractError::invalid("identity.name", "the identity name is too long"));
        }
        if self.permissions.is_empty() {
            return Err(ContractError::invalid("identity.permissions", "an identity needs at least one permission"));
        }
        let mut seen = BTreeSet::new();
        for permission in &self.permissions {
            if !seen.insert(permission.as_str()) {
                return Err(ContractError::invalid("identity.permissions", "a permission is listed twice"));
            }
        }
        Ok(())
    }
}

/// Which kind of shared record a version belongs to. Used by change facts.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SharedRecordKind {
    Material,
    CostCenter,
    Component,
    AppConfig,
}

impl SharedRecordKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Material => "material",
            Self::CostCenter => "cost_center",
            Self::Component => "component",
            Self::AppConfig => "app_config",
        }
    }
}

/// How a cost center rate is used. Setup uses its own dedicated cost center.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CostCenterPurpose {
    Production,
    Setup,
}

/// One published change to a shared record, emitted for later packages (SB-20) to consume.
///
/// The fact records what was published and by whom. It is not a project notification and it does
/// not adopt anything into a project.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReferenceChange {
    pub change_id: Uuid,
    pub kind: SharedRecordKind,
    /// The logical record id; it is stable across its versions.
    pub record_id: Uuid,
    pub version_id: Uuid,
    pub version: u64,
    pub previous_version_id: Option<Uuid>,
    pub summary: String,
    pub changed_by: String,
    /// RFC 3339 UTC timestamp the service recorded.
    pub changed_at: String,
}

impl ReferenceChange {
    pub fn validate(&self) -> Result<()> {
        check_text(&self.summary, "reference_change.summary")?;
        check_text(&self.changed_by, "reference_change.changed_by")?;
        check_text(&self.changed_at, "reference_change.changed_at")?;
        if self.version == 0 {
            return Err(ContractError::invalid("reference_change.version", "a published version starts at 1"));
        }
        Ok(())
    }
}

/// Which built-in direct-cost subtotal a category names.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DirectCostInput {
    Material,
    Production,
    Setup,
}

/// The explicit subtotal a percentage category is calculated from.
///
/// A basis is either the engine's direct-cost subtotal, a set of named categories, or both. At
/// least one term is required: a percentage without a declared base is ambiguous and refused.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CategoryBasis {
    /// Include the engine's direct-cost subtotal (material and production, never setup).
    #[serde(default)]
    pub direct_cost: bool,
    /// Include these category codes. Each code is listed once.
    #[serde(default)]
    pub categories: Vec<String>,
}

impl CategoryBasis {
    fn terms(&self) -> usize {
        usize::from(self.direct_cost) + self.categories.len()
    }
}

/// How one configured cost category is calculated. There is no formula language.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CategoryCalculation {
    /// A calculated direct cost from a supported engine.
    Direct { input: DirectCostInput },
    /// A fixed amount per unit, in the configuration's currency.
    Fixed { amount: Decimal },
    /// A percentage of an explicitly defined cost subtotal.
    Percentage { basis: CategoryBasis, rate: Decimal },
}

/// One configured cost category. An administrator maintains the categories in a shared version.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CostCategory {
    /// The stable code that other categories and scenarios name.
    pub code: String,
    pub name: String,
    pub calculation: CategoryCalculation,
    /// Whether this operating category is included in recurring EBIT.
    ///
    /// Only fixed and percentage categories are operating categories. A calculated direct-cost
    /// category is already part of the recurring unit cost and cannot be selected here; the setup
    /// subtotal stays outside recurring EBIT.
    pub included_in_ebit: bool,
}

/// One append-only version of the app cost configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppConfigVersion {
    pub config_version_id: Uuid,
    pub config_id: Uuid,
    pub version: u64,
    pub supersedes: Option<Uuid>,
    pub currency: String,
    pub categories: Vec<CostCategory>,
    pub source_reference: Option<String>,
    /// RFC 3339 UTC timestamp the service recorded.
    pub created_at: Option<String>,
}

impl AppConfigVersion {
    /// Refuse an ambiguous percentage base, a circular or duplicate category inclusion, or an
    /// invalid monetary value. A saved configuration always passes this check.
    pub fn validate(&self) -> Result<()> {
        if self.version == 0 {
            return Err(ContractError::invalid("app_config.version", "a published version starts at 1"));
        }
        if self.currency.len() != 3 || !self.currency.bytes().all(|byte| byte.is_ascii_uppercase()) {
            return Err(ContractError::invalid("app_config.currency", "expected a three-letter upper-case currency code"));
        }
        check_optional_reference(&self.source_reference, "app_config.source_reference")?;
        if self.categories.len() > MAX_COST_CATEGORIES {
            return Err(ContractError::invalid("app_config.categories", "too many cost categories"));
        }

        // Pass one: unique codes and the direct subtotals each category names.
        let mut codes = BTreeSet::new();
        let mut direct_inputs = BTreeSet::new();
        for category in &self.categories {
            check_text(&category.code, "app_config.categories.code")?;
            if category.code.chars().count() > MAX_SHARED_NAME_CHARS {
                return Err(ContractError::invalid("app_config.categories.code", "the category code is too long"));
            }
            if !codes.insert(category.code.clone()) {
                return Err(ContractError::invalid("app_config.categories", format!("duplicate category code `{}`", category.code)));
            }
            check_text(&category.name, "app_config.categories.name")?;
            match &category.calculation {
                CategoryCalculation::Direct { input } => {
                    if !direct_inputs.insert(*input) {
                        return Err(ContractError::invalid(
                            "app_config.categories",
                            format!("the `{}` direct cost is included twice", direct_input_name(*input)),
                        ));
                    }
                    if category.included_in_ebit {
                        return Err(ContractError::invalid(
                            "app_config.categories.included_in_ebit",
                            format!("`{}` is a direct cost, not an operating category", category.code),
                        ));
                    }
                }
                CategoryCalculation::Fixed { amount } => {
                    if amount.is_sign_negative() {
                        return Err(ContractError::invalid(
                            "app_config.categories.amount",
                            format!("`{}` has a negative fixed amount", category.code),
                        ));
                    }
                }
                CategoryCalculation::Percentage { basis, rate } => {
                    if rate.is_sign_negative() {
                        return Err(ContractError::invalid(
                            "app_config.categories.rate",
                            format!("`{}` has a negative percentage rate", category.code),
                        ));
                    }
                    if basis.terms() == 0 {
                        return Err(ContractError::invalid(
                            "app_config.categories.basis",
                            format!("`{}` has an ambiguous percentage base", category.code),
                        ));
                    }
                    if basis.categories.len() > MAX_BASIS_CATEGORIES {
                        return Err(ContractError::invalid("app_config.categories.basis", "too many basis categories"));
                    }
                    let mut named = BTreeSet::new();
                    for code in &basis.categories {
                        if !named.insert(code.as_str()) {
                            return Err(ContractError::invalid(
                                "app_config.categories.basis",
                                format!("`{}` names `{code}` twice in its base", category.code),
                            ));
                        }
                    }
                }
            }
        }

        // Pass two: every named basis category exists, is not the category itself, and does not
        // contribute a cost the basis already includes. A base that names the direct-cost subtotal
        // and also names a Direct(material) or Direct(production) category would count that direct
        // cost twice.
        for category in &self.categories {
            let CategoryCalculation::Percentage { basis, .. } = &category.calculation else { continue };
            for code in &basis.categories {
                if !codes.contains(code) {
                    return Err(ContractError::invalid(
                        "app_config.categories.basis",
                        format!("`{}` names an unknown category `{code}` in its base", category.code),
                    ));
                }
                if code == &category.code {
                    return Err(ContractError::invalid(
                        "app_config.categories.basis",
                        format!("`{}` includes itself in its percentage base", category.code),
                    ));
                }
                let named = self.categories.iter().find(|named| &named.code == code);
                if basis.direct_cost
                    && let Some(named) = named
                    && let CategoryCalculation::Direct { input } = &named.calculation
                    && matches!(input, DirectCostInput::Material | DirectCostInput::Production)
                {
                    return Err(ContractError::invalid(
                        "app_config.categories.basis",
                        format!(
                            "`{}` includes the direct-cost subtotal and the direct-cost category `{code}`; that cost would be counted twice",
                            category.code
                        ),
                    ));
                }
            }
        }

        // Pass three: percentage categories may not depend on each other in a cycle.
        let mut edges: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for category in &self.categories {
            if let CategoryCalculation::Percentage { basis, .. } = &category.calculation {
                edges.insert(category.code.as_str(), basis.categories.iter().map(String::as_str).collect());
            }
        }
        let mut visiting = BTreeSet::new();
        let mut done = BTreeSet::new();
        for code in edges.keys() {
            if cycle_from(code, &edges, &mut visiting, &mut done) {
                return Err(ContractError::invalid("app_config.categories.basis", format!("the percentage base of `{code}` is circular")));
            }
        }
        Ok(())
    }

    /// The category with that code, if the configuration defines it.
    pub fn category(&self, code: &str) -> Option<&CostCategory> {
        self.categories.iter().find(|category| category.code == code)
    }
}

fn direct_input_name(input: DirectCostInput) -> &'static str {
    match input {
        DirectCostInput::Material => "material",
        DirectCostInput::Production => "production",
        DirectCostInput::Setup => "setup",
    }
}

/// Depth-first search for a cycle among percentage categories.
fn cycle_from<'a>(node: &'a str, edges: &BTreeMap<&'a str, Vec<&'a str>>, visiting: &mut BTreeSet<&'a str>, done: &mut BTreeSet<&'a str>) -> bool {
    if done.contains(node) {
        return false;
    }
    if !visiting.insert(node) {
        return true;
    }
    let mut found = false;
    for next in edges.get(node).into_iter().flatten() {
        if cycle_from(next, edges, visiting, done) {
            found = true;
            break;
        }
    }
    visiting.remove(node);
    done.insert(node);
    found
}

/// The shared records an administrator maintains, with the newest published versions and facts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SharedDataView {
    pub identity: Identity,
    /// Every published material version, oldest version first per material.
    pub materials: Vec<MaterialVersion>,
    /// Every published cost-center rate version.
    pub cost_centers: Vec<RateVersion>,
    /// Every published catalog component version.
    pub components: Vec<CatalogComponentVersion>,
    /// The latest published app cost configuration, if one exists.
    pub app_config: Option<AppConfigVersion>,
    /// Every published app configuration version, oldest version first, so history stays readable.
    #[serde(default)]
    pub app_config_versions: Vec<AppConfigVersion>,
    /// Newest reference-change facts first, bounded by [`MAX_REFERENCE_CHANGES`].
    pub changes: Vec<ReferenceChange>,
}

impl SharedDataView {
    pub fn validate(&self) -> Result<()> {
        self.identity.validate()?;
        if self.changes.len() > MAX_REFERENCE_CHANGES {
            return Err(ContractError::invalid("shared_data.changes", "too many reference changes"));
        }
        let data = ReferenceData {
            materials: self.materials.clone(),
            rates: self.cost_centers.clone(),
            components: self.components.clone(),
            app_config: self.app_config.clone(),
        };
        data.validate()?;
        let mut config_versions = BTreeSet::new();
        for config in &self.app_config_versions {
            if !config_versions.insert(config.config_version_id) {
                return Err(ContractError::invalid("shared_data.app_config_versions", "two configurations share one version id"));
            }
            config.validate()?;
        }
        if let Some(latest) = &self.app_config {
            if self.app_config_versions.is_empty() {
                return Err(ContractError::invalid("shared_data.app_config", "the latest configuration is missing from its versions"));
            }
            if !config_versions.contains(&latest.config_version_id) {
                return Err(ContractError::invalid("shared_data.app_config", "the latest configuration is not one of the versions"));
            }
        }
        for change in &self.changes {
            change.validate()?;
        }
        Ok(())
    }
}

/// Publish the next material version. `expected_version` 0 creates version 1 of a new record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateMaterialVersion {
    pub request_id: String,
    pub expected_version: u64,
    pub material_id: Uuid,
    pub name: String,
    /// The material density this version publishes. It must be positive; a catalog version is a
    /// complete record, so every version carries it and no value is inherited silently.
    pub density_g_cm3: Decimal,
    /// `None` is a missing price, distinct from an explicit zero.
    pub price_per_kg: Option<Decimal>,
    pub currency: String,
    pub source_reference: Option<String>,
}

impl CreateMaterialVersion {
    pub fn validate(&self) -> Result<()> {
        check_shared_request(&self.request_id)?;
        check_shared_name(&self.name, "material.name")?;
        if self.density_g_cm3 <= Decimal::ZERO {
            return Err(ContractError::invalid("material.density_g_cm3", "density must be positive"));
        }
        check_currency(&self.currency, "material.currency")?;
        check_optional_reference(&self.source_reference, "material.source_reference")?;
        check_optional_price(self.price_per_kg, "material.price_per_kg")?;
        Ok(())
    }
}

/// Publish the next cost-center rate version. `expected_version` 0 creates version 1.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateRateVersion {
    pub request_id: String,
    pub expected_version: u64,
    pub cost_center_id: Uuid,
    pub name: String,
    pub purpose: CostCenterPurpose,
    /// `None` is a missing rate, distinct from an explicit zero.
    pub rate_per_hour: Option<Decimal>,
    pub currency: String,
    pub source_reference: Option<String>,
}

impl CreateRateVersion {
    pub fn validate(&self) -> Result<()> {
        check_shared_request(&self.request_id)?;
        check_shared_name(&self.name, "cost_center.name")?;
        check_currency(&self.currency, "cost_center.currency")?;
        check_optional_reference(&self.source_reference, "cost_center.source_reference")?;
        check_optional_rate(self.rate_per_hour, "cost_center.rate_per_hour")?;
        Ok(())
    }
}

/// Publish the next catalog component version. `expected_version` 0 creates version 1.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateComponentVersion {
    pub request_id: String,
    pub expected_version: u64,
    pub component_id: Uuid,
    pub name: String,
    pub specification: String,
    /// `None` is a missing price, distinct from an explicit zero.
    pub price: Option<Decimal>,
    pub currency: String,
    pub source_reference: Option<String>,
}

impl CreateComponentVersion {
    pub fn validate(&self) -> Result<()> {
        check_shared_request(&self.request_id)?;
        check_shared_name(&self.name, "component.name")?;
        check_text(&self.specification, "component.specification")?;
        if self.specification.chars().count() > MAX_SOURCE_REFERENCE_CHARS {
            return Err(ContractError::invalid("component.specification", "the specification is too long"));
        }
        check_currency(&self.currency, "component.currency")?;
        check_optional_reference(&self.source_reference, "component.source_reference")?;
        check_optional_price(self.price, "component.price")?;
        Ok(())
    }
}

/// Publish the next app cost configuration version. `expected_version` 0 creates version 1.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateAppConfigVersion {
    pub request_id: String,
    pub expected_version: u64,
    pub config_id: Uuid,
    pub currency: String,
    pub categories: Vec<CostCategory>,
    pub source_reference: Option<String>,
}

impl CreateAppConfigVersion {
    pub fn validate(&self) -> Result<()> {
        check_shared_request(&self.request_id)?;
        check_currency(&self.currency, "app_config.currency")?;
        check_optional_reference(&self.source_reference, "app_config.source_reference")?;
        // Reuse the version validator for the category rules; the ids are filled in on publish.
        AppConfigVersion {
            config_version_id: Uuid::nil(),
            config_id: self.config_id,
            version: 1,
            supersedes: None,
            currency: self.currency.clone(),
            categories: self.categories.clone(),
            source_reference: None,
            created_at: None,
        }
        .validate()
    }
}

/// The answer to publishing one material version.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MaterialVersionPublished {
    pub material: MaterialVersion,
    pub change: ReferenceChange,
}

/// The answer to publishing one cost-center rate version.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RateVersionPublished {
    pub cost_center: RateVersion,
    pub change: ReferenceChange,
}

/// The answer to publishing one catalog component version.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ComponentVersionPublished {
    pub component: CatalogComponentVersion,
    pub change: ReferenceChange,
}

/// The answer to publishing one app configuration version.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppConfigVersionPublished {
    pub config: AppConfigVersion,
    pub change: ReferenceChange,
}

fn check_shared_request(request_id: &str) -> Result<()> {
    check_text(request_id, "request_id")?;
    if request_id.chars().count() > MAX_SHARED_REQUEST_ID_CHARS {
        return Err(ContractError::invalid("request_id", "the request id is too long"));
    }
    Ok(())
}

fn check_shared_name(name: &str, field: &str) -> Result<()> {
    check_text(name, field)?;
    if name.chars().count() > MAX_SHARED_NAME_CHARS {
        return Err(ContractError::invalid(field, "the name is too long"));
    }
    Ok(())
}

fn check_currency(currency: &str, field: &str) -> Result<()> {
    if currency.len() != 3 || !currency.bytes().all(|byte| byte.is_ascii_uppercase()) {
        return Err(ContractError::invalid(field, "expected a three-letter upper-case currency code"));
    }
    Ok(())
}

fn check_optional_reference(reference: &Option<String>, field: &str) -> Result<()> {
    let Some(reference) = reference else { return Ok(()) };
    check_text(reference, field)?;
    if reference.chars().count() > MAX_SOURCE_REFERENCE_CHARS {
        return Err(ContractError::invalid(field, "the source reference is too long"));
    }
    Ok(())
}

fn check_optional_price(price: Option<Decimal>, field: &str) -> Result<()> {
    if price.is_some_and(|price| price.is_sign_negative()) {
        return Err(ContractError::invalid(field, "a price cannot be negative"));
    }
    Ok(())
}

fn check_optional_rate(rate: Option<Decimal>, field: &str) -> Result<()> {
    if rate.is_some_and(|rate| rate.is_sign_negative()) {
        return Err(ContractError::invalid(field, "a rate cannot be negative"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use rust_decimal::Decimal;

    use super::*;

    fn identity(permissions: &[Permission]) -> Identity {
        Identity { identity_id: Uuid::nil(), name: "someone".into(), permissions: permissions.to_vec() }
    }

    fn material(price: Option<&str>) -> MaterialVersion {
        MaterialVersion {
            material_version_id: Uuid::from_u128(1),
            material_id: Uuid::from_u128(10),
            version: 1,
            supersedes: None,
            name: "PA12".into(),
            density_g_cm3: "1.01".parse().unwrap_or_default(),
            density_unit: crate::Unit::GramPerCentimetreCubed,
            price_per_kg: price.map(|price| price.parse().unwrap_or_default()),
            price_unit: crate::Unit::CurrencyPerKilogram,
            currency: "CNY".into(),
            source_reference: None,
            created_at: None,
        }
    }

    fn category(code: &str, calculation: CategoryCalculation) -> CostCategory {
        CostCategory { code: code.into(), name: code.into(), calculation, included_in_ebit: false }
    }

    fn config(categories: Vec<CostCategory>) -> AppConfigVersion {
        AppConfigVersion {
            config_version_id: Uuid::nil(),
            config_id: Uuid::from_u128(1),
            version: 1,
            supersedes: None,
            currency: "CNY".into(),
            categories,
            source_reference: None,
            created_at: None,
        }
    }

    fn percentage(code: &str, basis: CategoryBasis, rate: &str) -> CostCategory {
        category(code, CategoryCalculation::Percentage { basis, rate: rate.parse().unwrap_or_default() })
    }

    #[test]
    fn an_identity_can_hold_both_permission_sets_and_combine_them() {
        let engineer = identity(&[Permission::Engineer]);
        assert!(engineer.can_work_on_projects());
        assert!(!engineer.can_manage_shared_data());
        let admin = identity(&[Permission::Administrator]);
        assert!(!admin.can_work_on_projects());
        assert!(admin.can_manage_shared_data());
        let both = identity(&[Permission::Engineer, Permission::Administrator]);
        assert!(both.can_work_on_projects());
        assert!(both.can_manage_shared_data());
        assert!(both.validate().is_ok());
        assert!(identity(&[Permission::Engineer, Permission::Engineer]).validate().is_err());
        assert!(identity(&[]).validate().is_err());
    }

    #[test]
    fn a_missing_price_is_null_and_an_explicit_zero_is_a_string() {
        let missing = serde_json::to_value(material(None)).unwrap_or_default();
        let zero = serde_json::to_value(material(Some("0"))).unwrap_or_default();
        assert_eq!(missing["price_per_kg"], serde_json::Value::Null);
        assert_eq!(zero["price_per_kg"], serde_json::json!("0"));
        assert_ne!(missing["price_per_kg"], zero["price_per_kg"]);
    }

    #[test]
    fn a_percentage_without_a_declared_base_is_ambiguous() {
        let ambiguous = config(vec![percentage("overhead", CategoryBasis::default(), "0.05")]);
        assert!(matches!(ambiguous.validate(), Err(ContractError::Invalid { .. })));
        let direct = config(vec![percentage("overhead", CategoryBasis { direct_cost: true, categories: Vec::new() }, "0.05")]);
        assert!(direct.validate().is_ok());
    }

    #[test]
    fn a_percentage_base_that_names_an_unknown_or_repeated_category_is_refused() {
        let unknown = config(vec![percentage("overhead", CategoryBasis { direct_cost: false, categories: vec!["missing".into()] }, "0.05")]);
        assert!(unknown.validate().is_err());
        let repeated = config(vec![
            category("consumables", CategoryCalculation::Fixed { amount: Decimal::from(2) }),
            percentage("overhead", CategoryBasis { direct_cost: false, categories: vec!["consumables".into(), "consumables".into()] }, "0.05"),
        ]);
        assert!(repeated.validate().is_err());
    }

    #[test]
    fn a_circular_or_self_inclusive_percentage_base_is_refused() {
        let itself = config(vec![percentage("overhead", CategoryBasis { direct_cost: false, categories: vec!["overhead".into()] }, "0.05")]);
        assert!(itself.validate().is_err());

        let first = percentage("a", CategoryBasis { direct_cost: false, categories: vec!["b".into()] }, "0.01");
        let second = percentage("b", CategoryBasis { direct_cost: false, categories: vec!["a".into()] }, "0.02");
        assert!(config(vec![first, second]).validate().is_err());
    }

    #[test]
    fn a_percentage_base_cannot_count_a_direct_cost_twice() {
        // The direct-cost subtotal already contains material and production, so naming a
        // Direct(material) category next to it would add the same cost again.
        let twice = config(vec![
            category("material-cost", CategoryCalculation::Direct { input: DirectCostInput::Material }),
            percentage("overhead", CategoryBasis { direct_cost: true, categories: vec!["material-cost".into()] }, "0.05"),
        ]);
        assert!(matches!(twice.validate(), Err(ContractError::Invalid { .. })));
        let twice_production = config(vec![
            category("production-cost", CategoryCalculation::Direct { input: DirectCostInput::Production }),
            percentage("overhead", CategoryBasis { direct_cost: true, categories: vec!["production-cost".into()] }, "0.05"),
        ]);
        assert!(twice_production.validate().is_err());
        // The setup subtotal stays outside the direct-cost subtotal, so naming it next to the
        // subtotal counts setup once and is accepted.
        let setup_once = config(vec![
            category("setup-cost", CategoryCalculation::Direct { input: DirectCostInput::Setup }),
            percentage("overhead", CategoryBasis { direct_cost: true, categories: vec!["setup-cost".into()] }, "0.05"),
        ]);
        assert!(setup_once.validate().is_ok());
    }

    #[test]
    fn a_duplicate_category_or_direct_inclusion_is_refused() {
        let duplicated = config(vec![
            category("consumables", CategoryCalculation::Fixed { amount: Decimal::ONE }),
            category("consumables", CategoryCalculation::Fixed { amount: Decimal::ONE }),
        ]);
        assert!(duplicated.validate().is_err());
        let direct_twice = config(vec![
            category("material-cost", CategoryCalculation::Direct { input: DirectCostInput::Material }),
            category("material-cost-2", CategoryCalculation::Direct { input: DirectCostInput::Material }),
        ]);
        assert!(direct_twice.validate().is_err());
    }

    #[test]
    fn a_direct_category_cannot_be_selected_for_recurring_ebit() {
        let mut direct = category("setup", CategoryCalculation::Direct { input: DirectCostInput::Setup });
        direct.included_in_ebit = true;
        assert!(config(vec![direct]).validate().is_err());
    }

    #[test]
    fn a_non_circular_percentage_chain_is_accepted() {
        let base = category("direct-material", CategoryCalculation::Direct { input: DirectCostInput::Material });
        let first = percentage("machine-overhead", CategoryBasis { direct_cost: true, categories: Vec::new() }, "0.1");
        let second = percentage("factory-overhead", CategoryBasis { direct_cost: false, categories: vec!["machine-overhead".into()] }, "0.05");
        assert!(config(vec![base, first, second]).validate().is_ok());
    }

    #[test]
    fn a_published_version_request_is_validated_before_the_service_writes_it() {
        let create = CreateMaterialVersion {
            request_id: "request-1".into(),
            expected_version: 0,
            material_id: Uuid::from_u128(10),
            name: "PA12".into(),
            density_g_cm3: "1.01".parse().unwrap_or_default(),
            price_per_kg: Some("0".parse().unwrap_or_default()),
            currency: "CNY".into(),
            source_reference: Some("supplier sheet 2026-11".into()),
        };
        assert!(create.validate().is_ok());
        let negative = CreateMaterialVersion { price_per_kg: Some("-1".parse().unwrap_or_default()), ..create.clone() };
        assert!(negative.validate().is_err());
        let long_name = CreateMaterialVersion { name: "x".repeat(MAX_SHARED_NAME_CHARS + 1), ..create };
        assert!(long_name.validate().is_err());
    }

    #[test]
    fn a_shared_data_view_refuses_duplicate_versions() {
        let mut view = SharedDataView {
            identity: identity(&[Permission::Administrator]),
            materials: vec![material(Some("20")), material(Some("21"))],
            cost_centers: Vec::new(),
            components: Vec::new(),
            app_config: None,
            app_config_versions: Vec::new(),
            changes: Vec::new(),
        };
        assert!(view.validate().is_err());
        view.materials[1].material_version_id = Uuid::from_u128(2);
        view.materials[1].version = 2;
        view.materials[1].supersedes = Some(Uuid::from_u128(1));
        assert!(view.validate().is_ok());
    }

    #[test]
    fn a_reference_change_round_trips_with_its_fact() {
        let change = ReferenceChange {
            change_id: Uuid::from_u128(3),
            kind: SharedRecordKind::Material,
            record_id: Uuid::from_u128(10),
            version_id: Uuid::from_u128(2),
            version: 2,
            previous_version_id: Some(Uuid::from_u128(1)),
            summary: "price 20 CNY/kg -> 21 CNY/kg".into(),
            changed_by: "administrator".into(),
            changed_at: "2026-11-09T00:00:00Z".into(),
        };
        let value = serde_json::to_value(&change).unwrap_or_default();
        assert_eq!(value["kind"], serde_json::json!("material"));
        assert_eq!(serde_json::from_value::<ReferenceChange>(value).ok(), Some(change));
    }
}
