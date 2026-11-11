//! Projects, deliveries and part versions: the version 2 project contract (package SB-10).
//!
//! One customer project owns its products, delivery packages, product revisions and shared
//! project part versions. A delivery may revise selected products and add new ones; omitted
//! products keep their current revision. A revised product revision is a draft until the owning
//! activation package (SB-19) records an activation; this contract stores and validates the
//! `draft`/`active`/`historical` identities and publishes no activation route.
//!
//! Identity rules:
//!
//! - `project_id`, `product_id`, `product_revision_id` and `part_version_id` are persistent
//!   business identities. `package_id` is one customer delivery of product data.
//! - A project part version is the pair of customer part number and customer revision; different
//!   customer revisions coexist as different part versions and are never merged by number alone.
//! - Customer identifiers propose a shared part identity; geometry validates the proposal within
//!   [`PART_MATCH_RELATIVE_TOLERANCE`]. Missing identifiers, missing geometry or conflicting
//!   geometry leave the delivered part unassociated until an engineer confirms the match.
//! - Every write is an append-only project record version. Exclusion/reinstatement changes the
//!   quotation scope and appends a recorded event; it never deletes packages, revisions or files.
//!
//! Wire rules follow version 1: decimals are strings, a missing value is `null` and an explicit
//! zero is separate. Unknown JSON fields are tolerated; the stored document names its schema.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{ContractError, Result};
use crate::types::SourceRef;

/// The project contract version this build reads and writes.
pub const PROJECTS_CONTRACT_VERSION: u32 = 2;

/// The schema version written into a stored project document.
pub const PROJECT_DOCUMENT_SCHEMA_VERSION: u32 = 1;

/// The relative tolerance geometry must stay within to validate a proposed identity match:
/// `width = 0.001` means 0.1 %, the same relative tolerance the measurement requirements use.
pub const PART_MATCH_RELATIVE_TOLERANCE: u64 = 1_000;

/// Resource bounds for hostile input. A project document is not a dump of arbitrary size.
pub const MAX_PRODUCTS: usize = 10_000;
pub const MAX_PACKAGES: usize = 10_000;
pub const MAX_REVISIONS_PER_PRODUCT: usize = 1_000;
pub const MAX_PARTS_PER_REVISION: usize = 10_000;
pub const MAX_FORECAST_YEARS: usize = 200;
pub const MAX_QUOTATION_EVENTS: usize = 100_000;
pub const MAX_NAME_CHARS: usize = 255;
pub const MAX_KEY_CHARS: usize = 255;

/// How often the revision of a product is treated while comparing and activating it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProductRevisionState {
    /// Prepared from a delivery; not the costing basis yet.
    Draft,
    /// The accepted costing basis of the product.
    Active,
    /// Previously active, kept for history.
    Historical,
}

/// Whether a delivered product is quoted.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QuotationState {
    Included,
    Excluded,
}

/// A simple part or a complex assembly in the project scope.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProductKind {
    Part,
    Assembly,
}

/// One inclusive calendar-year production range. SOP and EOP use calendar years only.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct YearRange {
    pub sop_year: u16,
    pub eop_year: u16,
}

impl YearRange {
    /// The range a product uses: its own override, else the project default.
    pub fn effective(project_default: YearRange, product_override: Option<YearRange>) -> Self {
        product_override.unwrap_or(project_default)
    }

    /// Validate the range and the bound on its length.
    pub fn validate(&self, field: &str) -> Result<()> {
        if !(1900..=9999).contains(&self.sop_year) || !(1900..=9999).contains(&self.eop_year) {
            return Err(ContractError::invalid(field, "a production year is a four-digit calendar year"));
        }
        if self.sop_year > self.eop_year {
            return Err(ContractError::invalid(field, "production ends before it starts"));
        }
        let years = usize::from(self.eop_year) - usize::from(self.sop_year) + 1;
        if years > MAX_FORECAST_YEARS {
            return Err(ContractError::invalid(field, format!("a production range is at most {MAX_FORECAST_YEARS} years")));
        }
        Ok(())
    }

    /// Iterate the inclusive years for issue reporting.
    pub fn years(&self) -> impl Iterator<Item = u16> {
        self.sop_year..=self.eop_year
    }
}

/// One positive annual production forecast. Zero-volume production years are not allowed.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnnualForecast {
    pub year: u16,
    pub quantity: u64,
}

impl AnnualForecast {
    pub(crate) fn validate(&self, field: &str) -> Result<()> {
        if !(1900..=9999).contains(&self.year) {
            return Err(ContractError::invalid(field, "a forecast year is a four-digit calendar year"));
        }
        if self.quantity == 0 {
            return Err(ContractError::invalid(field, "a production year needs a positive forecast; leave a year out instead of zero"));
        }
        Ok(())
    }
}

/// One definition delivered inside a product revision. `delivered_key` is the client's stable key
/// for the delivered definition (for a STEP import: the analysis definition name and path).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeliveredPart {
    pub delivered_key: String,
    pub customer_part_number: Option<String>,
    pub customer_revision: Option<String>,
    pub volume_mm3: Option<Decimal>,
    /// The shared project part version this definition is associated with; `None` means the match
    /// is pending engineer confirmation.
    pub part_version_id: Option<Uuid>,
}

impl DeliveredPart {
    fn validate(&self, field: &str) -> Result<()> {
        validate_key(&self.delivered_key, &format!("{field}.delivered_key"))?;
        if let Some(number) = &self.customer_part_number {
            validate_text(number, &format!("{field}.customer_part_number"))?;
        }
        if let Some(revision) = &self.customer_revision {
            validate_text(revision, &format!("{field}.customer_revision"))?;
        }
        if let Some(volume) = self.volume_mm3
            && volume.is_sign_negative()
        {
            return Err(ContractError::invalid(format!("{field}.volume_mm3"), "a volume cannot be negative"));
        }
        Ok(())
    }

    /// The customer identifiers that propose a shared identity, when both are present.
    pub fn identity_keys(&self) -> Option<(&str, &str)> {
        match (&self.customer_part_number, &self.customer_revision) {
            (Some(number), Some(revision)) if !number.trim().is_empty() && !revision.trim().is_empty() => Some((number, revision)),
            _ => None,
        }
    }
}

/// One delivered revision of a product. New deliveries create drafts; activation belongs to SB-19.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProductRevision {
    pub product_revision_id: Uuid,
    pub package_id: Uuid,
    pub state: ProductRevisionState,
    pub parts: Vec<DeliveredPart>,
}

impl ProductRevision {
    fn validate(&self, field: &str, package_ids: &[Uuid], part_versions: &[PartVersion]) -> Result<()> {
        if !package_ids.contains(&self.package_id) {
            return Err(ContractError::invalid(format!("{field}.package_id"), "the revision names a package this project does not have"));
        }
        if self.parts.len() > MAX_PARTS_PER_REVISION {
            return Err(ContractError::invalid(
                format!("{field}.parts"),
                format!("a revision holds at most {MAX_PARTS_PER_REVISION} delivered parts"),
            ));
        }
        let mut keys = Vec::with_capacity(self.parts.len());
        for part in &self.parts {
            part.validate(field)?;
            if keys.contains(&part.delivered_key) {
                return Err(ContractError::invalid(format!("{field}.parts"), "two delivered parts share one delivered key"));
            }
            if let Some(part_version_id) = part.part_version_id {
                let part_version = part_versions.iter().find(|part_version| part_version.part_version_id == part_version_id).ok_or_else(|| {
                    ContractError::invalid(
                        format!("{field}.parts.part_version_id"),
                        "a delivered part names a part version this project does not have",
                    )
                })?;
                if let Some((number, revision)) = part.identity_keys()
                    && (number.trim() != part_version.customer_part_number.trim() || revision.trim() != part_version.customer_revision.trim())
                {
                    return Err(ContractError::invalid(
                        format!("{field}.parts.part_version_id"),
                        "the delivered customer number and revision differ from the associated part version; separate customer revisions stay separate",
                    ));
                }
            }
            keys.push(part.delivered_key.clone());
        }
        Ok(())
    }
}

/// One product in the project. Its identity persists across customer deliveries.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Product {
    pub product_id: Uuid,
    pub name: String,
    pub kind: ProductKind,
    pub customer_part_number: Option<String>,
    /// The product's own SOP/EOP; `None` inherits the project default.
    pub schedule_override: Option<YearRange>,
    pub forecasts: Vec<AnnualForecast>,
    pub revisions: Vec<ProductRevision>,
    pub active_revision_id: Option<Uuid>,
    pub quotation_state: QuotationState,
}

impl Product {
    /// The schedule this product uses.
    pub fn schedule(&self, project_default: YearRange) -> YearRange {
        YearRange::effective(project_default, self.schedule_override)
    }

    fn validate(&self, field: &str, default_schedule: YearRange, package_ids: &[Uuid], part_versions: &[PartVersion]) -> Result<()> {
        validate_text(&self.name, &format!("{field}.name"))?;
        if let Some(number) = &self.customer_part_number {
            validate_text(number, &format!("{field}.customer_part_number"))?;
        }
        let schedule = self.schedule(default_schedule);
        schedule.validate(&format!("{field}.schedule"))?;
        if self.revisions.len() > MAX_REVISIONS_PER_PRODUCT {
            return Err(ContractError::invalid(
                format!("{field}.revisions"),
                format!("a product holds at most {MAX_REVISIONS_PER_PRODUCT} revisions"),
            ));
        }
        if self.forecasts.len() > MAX_FORECAST_YEARS {
            return Err(ContractError::invalid(format!("{field}.forecasts"), format!("a product holds at most {MAX_FORECAST_YEARS} forecast years")));
        }
        let mut years = Vec::with_capacity(self.forecasts.len());
        for forecast in &self.forecasts {
            forecast.validate(&format!("{field}.forecasts"))?;
            if years.contains(&forecast.year) {
                return Err(ContractError::invalid(format!("{field}.forecasts"), "two forecasts name one calendar year"));
            }
            if forecast.year < schedule.sop_year || forecast.year > schedule.eop_year {
                return Err(ContractError::invalid(
                    format!("{field}.forecasts"),
                    "a forecast year is outside the product schedule; do not pad years",
                ));
            }
            years.push(forecast.year);
        }
        let mut revision_ids = Vec::with_capacity(self.revisions.len());
        let mut active = Vec::new();
        for revision in &self.revisions {
            if revision_ids.contains(&revision.product_revision_id) {
                return Err(ContractError::invalid(format!("{field}.revisions"), "two revisions share one product revision id"));
            }
            revision.validate(&format!("{field}.revisions"), package_ids, part_versions)?;
            if revision.state == ProductRevisionState::Active {
                active.push(revision.product_revision_id);
            }
            revision_ids.push(revision.product_revision_id);
        }
        if active.len() > 1 {
            return Err(ContractError::invalid(format!("{field}.revisions"), "a product has more than one active revision"));
        }
        match (self.active_revision_id, active.first()) {
            (None, Some(_)) => Err(ContractError::invalid(format!("{field}.active_revision_id"), "the product names no active revision but has one")),
            (Some(named), Some(active)) if named != *active => {
                Err(ContractError::invalid(format!("{field}.active_revision_id"), "the active revision pointer names another revision"))
            }
            (Some(_), None) => {
                Err(ContractError::invalid(format!("{field}.active_revision_id"), "the active revision pointer names a revision that is not active"))
            }
            _ => Ok(()),
        }
    }

    /// The missing production-year forecasts, in schedule order.
    pub fn missing_forecast_years(&self, project_default: YearRange) -> Vec<u16> {
        let schedule = self.schedule(project_default);
        let mut years = Vec::new();
        for year in schedule.years() {
            if !self.forecasts.iter().any(|forecast| forecast.year == year) {
                years.push(year);
            }
        }
        years
    }

    /// The delivered part with `delivered_key` in one revision, when present.
    pub fn delivered_part(&self, product_revision_id: Uuid, delivered_key: &str) -> Option<&DeliveredPart> {
        self.revisions
            .iter()
            .find(|revision| revision.product_revision_id == product_revision_id)
            .and_then(|revision| revision.parts.iter().find(|part| part.delivered_key == delivered_key))
    }
}

/// One customer delivery of product data. A package belongs to exactly one project and keeps the
/// original source it was imported from.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeliveryPackage {
    pub package_id: Uuid,
    pub name: String,
    pub source: SourceRef,
}

impl DeliveryPackage {
    fn validate(&self, field: &str) -> Result<()> {
        validate_text(&self.name, &format!("{field}.name"))?;
        self.source.validate().map_err(|error| ContractError::invalid(format!("{field}.source"), error.to_string()))?;
        Ok(())
    }
}

/// One shared project part definition: a customer part number with one customer revision.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PartVersion {
    pub part_version_id: Uuid,
    pub customer_part_number: String,
    pub customer_revision: String,
    /// The validated volume the identity was proposed from; `None` when geometry was unavailable.
    pub volume_mm3: Option<Decimal>,
    pub created_in_package_id: Uuid,
}

impl PartVersion {
    fn validate(&self, field: &str, package_ids: &[Uuid]) -> Result<()> {
        validate_text(&self.customer_part_number, &format!("{field}.customer_part_number"))?;
        validate_text(&self.customer_revision, &format!("{field}.customer_revision"))?;
        if !package_ids.contains(&self.created_in_package_id) {
            return Err(ContractError::invalid(
                format!("{field}.created_in_package_id"),
                "the part version names a package this project does not have",
            ));
        }
        if let Some(volume) = self.volume_mm3
            && volume.is_sign_negative()
        {
            return Err(ContractError::invalid(format!("{field}.volume_mm3"), "a volume cannot be negative"));
        }
        Ok(())
    }
}

/// One recorded quotation-scope change. The event is kept after the product is reinstated.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QuotationEvent {
    pub sequence: u64,
    pub product_ids: Vec<Uuid>,
    pub state: QuotationState,
}

impl QuotationEvent {
    fn validate(&self, field: &str, product_ids: &[Uuid]) -> Result<()> {
        if self.sequence == 0 {
            return Err(ContractError::invalid(format!("{field}.sequence"), "a quotation event sequence starts at 1"));
        }
        if self.product_ids.is_empty() {
            return Err(ContractError::invalid(format!("{field}.product_ids"), "a quotation event names at least one product"));
        }
        for product_id in &self.product_ids {
            if !product_ids.contains(product_id) {
                return Err(ContractError::invalid(format!("{field}.product_ids"), "a quotation event names a product this project does not have"));
            }
        }
        Ok(())
    }
}

/// The stored, appended project record. History lives in the immutable project record versions;
/// this document is the current one.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProjectDocument {
    pub schema_version: u32,
    pub project_id: Uuid,
    pub name: String,
    pub customer: String,
    pub internal_team: Option<String>,
    pub platform: Option<String>,
    pub application: Option<String>,
    pub currency: String,
    pub default_schedule: YearRange,
    pub products: Vec<Product>,
    pub packages: Vec<DeliveryPackage>,
    pub part_versions: Vec<PartVersion>,
    pub quotation_events: Vec<QuotationEvent>,
    /// The project's costing settings (package SB-16), additive and defaulted. A document written
    /// before that contract has no field and reads as `None`, never as an implicit policy.
    #[serde(default)]
    pub costing: Option<crate::costing::ProjectCosting>,
    /// The project's tooling settings (package SB-17), additive and defaulted. A document written
    /// before that contract has no field and reads as `None`, never as an implicit treatment.
    #[serde(default)]
    pub tooling: Option<crate::tooling::ProjectTooling>,
    /// The project's commercial settings (package SB-18), additive and defaulted. A document written
    /// before that contract has no field and reads as `None`, never as an implicit EBIT target.
    #[serde(default)]
    pub commercial: Option<crate::commercial::ProjectCommercial>,
}

/// The business details a project is created with and later updated.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProjectDetails {
    pub name: String,
    pub customer: String,
    pub internal_team: Option<String>,
    pub platform: Option<String>,
    pub application: Option<String>,
    pub currency: String,
    pub default_schedule: YearRange,
}

impl ProjectDetails {
    /// Validate the editable project details.
    pub fn validate(&self) -> Result<()> {
        validate_text(&self.name, "project.name")?;
        validate_text(&self.customer, "project.customer")?;
        for (field, value) in
            [("project.internal_team", &self.internal_team), ("project.platform", &self.platform), ("project.application", &self.application)]
        {
            if let Some(value) = value {
                validate_text(value, field)?;
            }
        }
        validate_currency(&self.currency)?;
        self.default_schedule.validate("project.default_schedule")
    }
}

impl ProjectDocument {
    /// A new empty project document for `details`.
    pub fn new(project_id: Uuid, details: &ProjectDetails) -> Self {
        Self {
            schema_version: PROJECT_DOCUMENT_SCHEMA_VERSION,
            project_id,
            name: details.name.clone(),
            customer: details.customer.clone(),
            internal_team: details.internal_team.clone(),
            platform: details.platform.clone(),
            application: details.application.clone(),
            currency: details.currency.clone(),
            default_schedule: details.default_schedule,
            products: Vec::new(),
            packages: Vec::new(),
            part_versions: Vec::new(),
            quotation_events: Vec::new(),
            costing: None,
            tooling: None,
            commercial: None,
        }
    }

    /// Replace the editable details, keeping every product, package and recorded event.
    pub fn apply_details(&mut self, details: &ProjectDetails) {
        self.name = details.name.clone();
        self.customer = details.customer.clone();
        self.internal_team = details.internal_team.clone();
        self.platform = details.platform.clone();
        self.application = details.application.clone();
        self.currency = details.currency.clone();
        self.default_schedule = details.default_schedule;
    }

    /// Validate a stored document before it is written and before clients read one.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != PROJECT_DOCUMENT_SCHEMA_VERSION {
            return Err(ContractError::SchemaVersion { found: self.schema_version, expected: PROJECT_DOCUMENT_SCHEMA_VERSION });
        }
        ProjectDetails {
            name: self.name.clone(),
            customer: self.customer.clone(),
            internal_team: self.internal_team.clone(),
            platform: self.platform.clone(),
            application: self.application.clone(),
            currency: self.currency.clone(),
            default_schedule: self.default_schedule,
        }
        .validate()?;
        if self.products.len() > MAX_PRODUCTS {
            return Err(ContractError::invalid("project.products", format!("a project holds at most {MAX_PRODUCTS} products")));
        }
        if self.packages.len() > MAX_PACKAGES {
            return Err(ContractError::invalid("project.packages", format!("a project holds at most {MAX_PACKAGES} packages")));
        }
        if self.quotation_events.len() > MAX_QUOTATION_EVENTS {
            return Err(ContractError::invalid(
                "project.quotation_events",
                format!("a project holds at most {MAX_QUOTATION_EVENTS} quotation events"),
            ));
        }
        let package_ids: Vec<Uuid> = self.packages.iter().map(|package| package.package_id).collect();
        let mut unique_packages = Vec::with_capacity(package_ids.len());
        for package in &self.packages {
            package.validate("project.packages")?;
            if unique_packages.contains(&package.package_id) {
                return Err(ContractError::invalid("project.packages", "two delivery packages share one package id"));
            }
            unique_packages.push(package.package_id);
        }
        let mut unique_part_versions = Vec::with_capacity(self.part_versions.len());
        let mut identities: Vec<(String, String)> = Vec::with_capacity(self.part_versions.len());
        for part in &self.part_versions {
            part.validate("project.part_versions", &package_ids)?;
            if unique_part_versions.contains(&part.part_version_id) {
                return Err(ContractError::invalid("project.part_versions", "two part versions share one part version id"));
            }
            let identity = (part.customer_part_number.trim().to_string(), part.customer_revision.trim().to_string());
            if identities.contains(&identity) {
                return Err(ContractError::invalid(
                    "project.part_versions",
                    "two part versions share one customer number and revision; different revisions must stay separate",
                ));
            }
            unique_part_versions.push(part.part_version_id);
            identities.push(identity);
        }
        let mut product_ids = Vec::with_capacity(self.products.len());
        let mut names = Vec::with_capacity(self.products.len());
        for product in &self.products {
            product.validate("project.products", self.default_schedule, &package_ids, &self.part_versions)?;
            if product_ids.contains(&product.product_id) {
                return Err(ContractError::invalid("project.products", "two products share one product id"));
            }
            let name = product.name.trim().to_lowercase();
            if names.contains(&name) {
                return Err(ContractError::invalid(
                    "project.products",
                    "two products share one name; delivery matching needs distinct product names",
                ));
            }
            product_ids.push(product.product_id);
            names.push(name);
        }
        let mut sequences = Vec::with_capacity(self.quotation_events.len());
        for event in &self.quotation_events {
            event.validate("project.quotation_events", &product_ids)?;
            if sequences.contains(&event.sequence) {
                return Err(ContractError::invalid("project.quotation_events", "two quotation events share one sequence"));
            }
            sequences.push(event.sequence);
        }
        for product in &self.products {
            if let Some(latest) = self.quotation_events.iter().rev().find(|event| event.product_ids.contains(&product.product_id)) {
                if product.quotation_state != latest.state {
                    return Err(ContractError::invalid(
                        "project.products.quotation_state",
                        "the product quotation state disagrees with its latest recorded event",
                    ));
                }
            } else if product.quotation_state != QuotationState::Included {
                return Err(ContractError::invalid("project.products.quotation_state", "a product has no recorded event for its excluded state"));
            }
        }
        if let Some(costing) = &self.costing {
            costing.validate(&self.currency, &unique_part_versions)?;
        }
        if let Some(tooling) = &self.tooling {
            tooling.validate(&self.currency, &unique_part_versions, &product_ids)?;
        }
        if let Some(commercial) = &self.commercial {
            commercial.validate()?;
        }
        Ok(())
    }

    /// Replace the project's costing settings, keeping every product, package, part version and
    /// recorded event. The caller validates the settings against this document before writing.
    pub fn apply_costing(&mut self, costing: crate::costing::ProjectCosting) {
        self.costing = Some(costing);
    }

    /// Replace the project's tooling settings, keeping every product, package, part version and
    /// recorded event. The caller validates the settings against this document before writing.
    pub fn apply_tooling(&mut self, tooling: crate::tooling::ProjectTooling) {
        self.tooling = Some(tooling);
    }

    /// Replace the project's commercial settings, keeping every product, package, part version and
    /// recorded event. The caller validates the settings before writing.
    pub fn apply_commercial(&mut self, commercial: crate::commercial::ProjectCommercial) {
        self.commercial = Some(commercial);
    }

    /// The next quotation event sequence.
    pub fn next_quotation_sequence(&self) -> u64 {
        self.quotation_events.iter().map(|event| event.sequence).max().unwrap_or(0) + 1
    }

    /// The shared part version with this customer identity, when one exists.
    pub fn part_version_by_identity(&self, number: &str, revision: &str) -> Option<&PartVersion> {
        self.part_versions.iter().find(|part| part.customer_part_number.trim() == number.trim() && part.customer_revision.trim() == revision.trim())
    }

    /// Record a delivery's new product revision on an existing product.
    pub fn push_revision(&mut self, product_id: Uuid, revision: ProductRevision) -> Result<()> {
        let product = self.product_mut(product_id)?;
        product.revisions.push(revision);
        Ok(())
    }

    /// The mutable product with `product_id`.
    pub fn product_mut(&mut self, product_id: Uuid) -> Result<&mut Product> {
        self.products
            .iter_mut()
            .find(|product| product.product_id == product_id)
            .ok_or_else(|| ContractError::invalid("project.products", "the project has no such product"))
    }

    /// The product with `product_id`.
    pub fn product(&self, product_id: Uuid) -> Option<&Product> {
        self.products.iter().find(|product| product.product_id == product_id)
    }

    /// The delivered part inside one product revision.
    pub fn delivered_part_mut(&mut self, product_revision_id: Uuid, delivered_key: &str) -> Result<&mut DeliveredPart> {
        for product in &mut self.products {
            for revision in &mut product.revisions {
                if revision.product_revision_id == product_revision_id {
                    return revision
                        .parts
                        .iter_mut()
                        .find(|part| part.delivered_key == delivered_key)
                        .ok_or_else(|| ContractError::invalid("project.delivered_key", "the revision has no such delivered part"));
                }
            }
        }
        Err(ContractError::invalid("project.product_revision_id", "the project has no such product revision"))
    }

    /// The current document's issues: everything an engineer still has to complete. They are
    /// computed, never stored, and a missing value is an issue rather than a zero.
    pub fn issues(&self) -> Vec<ProjectIssue> {
        let mut issues = Vec::new();
        for product in &self.products {
            for year in product.missing_forecast_years(self.default_schedule) {
                issues.push(
                    ProjectIssue::new(
                        "missing_forecast",
                        format!("products.{}.forecasts.{year}", product.product_id),
                        format!("{} needs a positive forecast for {year}", product.name),
                    )
                    .for_product(product.product_id),
                );
            }
            for revision in &product.revisions {
                for part in &revision.parts {
                    if part.part_version_id.is_some() {
                        continue;
                    }
                    let code = match (part.identity_keys(), part.volume_mm3) {
                        (None, _) => "missing_part_identity",
                        (Some(_), None) => "missing_geometry_for_match",
                        (Some(_), Some(_)) => "conflicting_geometry_match",
                    };
                    let message = match code {
                        "missing_part_identity" => "the delivered part has no customer number and revision to propose an identity",
                        "missing_geometry_for_match" => "geometry is unavailable, so the proposed identity match needs engineer confirmation",
                        _ => "geometry disagrees with the existing part identity, so the match needs engineer confirmation",
                    };
                    issues.push(
                        ProjectIssue::new(
                            code,
                            format!("products.{}.revisions.{}.parts.{}", product.product_id, revision.product_revision_id, part.delivered_key),
                            message,
                        )
                        .for_product(product.product_id),
                    );
                }
            }
        }
        issues
    }
}

/// One project issue: a stable code and a client-safe message, never display text.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectIssue {
    pub code: String,
    pub field_path: String,
    pub product_id: Option<Uuid>,
    pub message: String,
}

impl ProjectIssue {
    /// A named issue. `code` is a stable contract name, not display text.
    pub fn new(code: impl Into<String>, field_path: impl Into<String>, message: impl Into<String>) -> Self {
        Self { code: code.into(), field_path: field_path.into(), product_id: None, message: message.into() }
    }

    /// Attach the affected product.
    pub fn for_product(mut self, product_id: Uuid) -> Self {
        self.product_id = Some(product_id);
        self
    }
}

/// One dependency fact a write emits. Facts name what changed; they never rewrite accepted results.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProjectFact {
    /// A delivery appended product revisions. `pending_part_matches` counts the delivered parts
    /// that need engineer confirmation before they share a project part version.
    DeliveryRecorded { package_id: Uuid, product_ids: Vec<Uuid>, pending_part_matches: u64 },
    /// The quotation scope changed for products; accepted results stay unchanged.
    QuotationScopeChanged { product_ids: Vec<Uuid>, state: QuotationState },
    /// An engineer confirmed one pending delivered-part match.
    PartMatchConfirmed { product_revision_id: Uuid, delivered_key: String, part_version_id: Uuid },
}

/// The compact receipt of one committed project record version.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectReceipt {
    pub request_id: String,
    pub project_id: Uuid,
    pub version: u64,
}

/// One committed project write: its receipt and the dependency facts it emitted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProjectWriteOutcome {
    pub receipt: ProjectReceipt,
    pub facts: Vec<ProjectFact>,
    /// The draft result published in the same transaction when this write changed the draft demand
    /// (forecasts/schedule or quotation scope). `None` for other writes.
    #[serde(default)]
    pub draft: Option<crate::adoption::DraftPublication>,
}

/// A project record as a client reads it: the current document and its computed issues.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProjectRecordView {
    pub project_id: Uuid,
    pub version: u64,
    pub document: ProjectDocument,
    pub issues: Vec<ProjectIssue>,
}

/// Create a project with its business details.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CreateProject {
    pub request_id: String,
    pub project_id: Uuid,
    pub details: ProjectDetails,
}

impl CreateProject {
    /// Validate the request before the store writes it.
    pub fn validate(&self) -> Result<()> {
        validate_request_id(&self.request_id, "create_project.request_id")?;
        self.details.validate()
    }
}

/// Update the editable project details. Expected-version checked.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UpdateProject {
    pub request_id: String,
    pub expected_version: u64,
    pub details: ProjectDetails,
}

impl UpdateProject {
    /// Validate the request before the store writes it.
    pub fn validate(&self) -> Result<()> {
        validate_request_id(&self.request_id, "update_project.request_id")?;
        if self.expected_version == 0 {
            return Err(ContractError::invalid("update_project.expected_version", "an existing project starts at version 1"));
        }
        self.details.validate()
    }
}

/// Update the editable product fields: its name, kind, customer number, schedule and forecasts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UpdateProduct {
    pub request_id: String,
    pub expected_version: u64,
    pub product_id: Uuid,
    pub name: String,
    pub kind: ProductKind,
    pub customer_part_number: Option<String>,
    pub schedule_override: Option<YearRange>,
    pub forecasts: Vec<AnnualForecast>,
}

impl UpdateProduct {
    /// Validate the request's own fields; the store validates them against the project.
    pub fn validate(&self) -> Result<()> {
        validate_request_id(&self.request_id, "update_product.request_id")?;
        if self.expected_version == 0 {
            return Err(ContractError::invalid("update_product.expected_version", "an existing project starts at version 1"));
        }
        validate_text(&self.name, "update_product.name")?;
        if let Some(number) = &self.customer_part_number {
            validate_text(number, "update_product.customer_part_number")?;
        }
        if self.forecasts.len() > MAX_FORECAST_YEARS {
            return Err(ContractError::invalid("update_product.forecasts", format!("at most {MAX_FORECAST_YEARS} forecast years")));
        }
        let mut years = Vec::with_capacity(self.forecasts.len());
        for forecast in &self.forecasts {
            forecast.validate("update_product.forecasts")?;
            if years.contains(&forecast.year) {
                return Err(ContractError::invalid("update_product.forecasts", "two forecasts name one calendar year"));
            }
            years.push(forecast.year);
        }
        if let Some(schedule) = self.schedule_override {
            schedule.validate("update_product.schedule_override")?;
        }
        Ok(())
    }
}

/// One product delivered in a package. When `product_id` is absent the service proposes the
/// existing product by customer number (then by name) and creates a new identity when none
/// matches; an ambiguous proposal is refused until the client names the product.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeliveredProductInput {
    pub product_id: Option<Uuid>,
    pub name: String,
    pub kind: ProductKind,
    pub customer_part_number: Option<String>,
    pub parts: Vec<DeliveredPartInput>,
}

/// One definition of a delivered product, as the client's analysis describes it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeliveredPartInput {
    pub delivered_key: String,
    pub customer_part_number: Option<String>,
    pub customer_revision: Option<String>,
    pub volume_mm3: Option<Decimal>,
}

impl DeliveredPartInput {
    /// The customer identifiers that propose a shared identity, when both are present.
    pub fn identity_keys(&self) -> Option<(&str, &str)> {
        match (&self.customer_part_number, &self.customer_revision) {
            (Some(number), Some(revision)) if !number.trim().is_empty() && !revision.trim().is_empty() => Some((number, revision)),
            _ => None,
        }
    }

    fn validate(&self, field: &str) -> Result<()> {
        validate_text(&self.delivered_key, &format!("{field}.delivered_key"))?;
        if let Some(number) = &self.customer_part_number {
            validate_text(number, &format!("{field}.customer_part_number"))?;
        }
        if let Some(revision) = &self.customer_revision {
            validate_text(revision, &format!("{field}.customer_revision"))?;
        }
        if let Some(volume) = self.volume_mm3
            && volume.is_sign_negative()
        {
            return Err(ContractError::invalid(format!("{field}.volume_mm3"), "a volume cannot be negative"));
        }
        Ok(())
    }
}

/// Record one customer delivery: a package with an original source and the product revisions it
/// delivers. Omitted products are unchanged; a delivered existing product gets a new draft.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecordDelivery {
    pub request_id: String,
    pub expected_version: u64,
    pub package: DeliveryPackage,
    pub products: Vec<DeliveredProductInput>,
}

impl RecordDelivery {
    /// Validate the request's own fields; the store validates them against the project.
    pub fn validate(&self) -> Result<()> {
        validate_request_id(&self.request_id, "record_delivery.request_id")?;
        if self.expected_version == 0 {
            return Err(ContractError::invalid("record_delivery.expected_version", "a delivery saves from version 1"));
        }
        self.package.validate("record_delivery.package")?;
        if self.products.is_empty() {
            return Err(ContractError::invalid("record_delivery.products", "a delivery delivers at least one product"));
        }
        if self.products.len() > MAX_PRODUCTS {
            return Err(ContractError::invalid("record_delivery.products", format!("a delivery holds at most {MAX_PRODUCTS} products")));
        }
        let mut names = Vec::with_capacity(self.products.len());
        let mut explicit_ids = Vec::with_capacity(self.products.len());
        for product in &self.products {
            validate_text(&product.name, "record_delivery.products.name")?;
            if !names.contains(&product.name.trim().to_lowercase()) {
                names.push(product.name.trim().to_lowercase());
            } else {
                return Err(ContractError::invalid("record_delivery.products", "two delivered products share one name"));
            }
            if let Some(product_id) = product.product_id {
                if explicit_ids.contains(&product_id) {
                    return Err(ContractError::invalid("record_delivery.products", "two delivered products name one project product"));
                }
                explicit_ids.push(product_id);
            }
            if let Some(number) = &product.customer_part_number {
                validate_text(number, "record_delivery.products.customer_part_number")?;
            }
            if product.parts.len() > MAX_PARTS_PER_REVISION {
                return Err(ContractError::invalid(
                    "record_delivery.products.parts",
                    format!("a revision holds at most {MAX_PARTS_PER_REVISION} parts"),
                ));
            }
            let mut keys = Vec::with_capacity(product.parts.len());
            for part in &product.parts {
                part.validate("record_delivery.products.parts")?;
                if keys.contains(&part.delivered_key) {
                    return Err(ContractError::invalid("record_delivery.products.parts", "two delivered parts share one delivered key"));
                }
                keys.push(part.delivered_key.clone());
            }
        }
        Ok(())
    }
}

/// Confirm one pending delivered-part match against a shared project part version.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConfirmPartMatch {
    pub request_id: String,
    pub expected_version: u64,
    pub product_revision_id: Uuid,
    pub delivered_key: String,
    pub part_version_id: Uuid,
}

impl ConfirmPartMatch {
    /// Validate the request's own fields; the store validates them against the project.
    pub fn validate(&self) -> Result<()> {
        validate_request_id(&self.request_id, "confirm_part_match.request_id")?;
        if self.expected_version == 0 {
            return Err(ContractError::invalid("confirm_part_match.expected_version", "an existing project starts at version 1"));
        }
        validate_text(&self.delivered_key, "confirm_part_match.delivered_key")
    }
}

/// Exclude products from the quotation scope. Files, revisions and history are retained.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExcludeProducts {
    pub request_id: String,
    pub expected_version: u64,
    pub product_ids: Vec<Uuid>,
}

impl ExcludeProducts {
    /// Validate the request's own fields; the store validates them against the project.
    pub fn validate(&self) -> Result<()> {
        validate_scope_request(&self.request_id, self.expected_version, &self.product_ids)
    }
}

/// Reinstate previously excluded products into the quotation scope.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReinstateProducts {
    pub request_id: String,
    pub expected_version: u64,
    pub product_ids: Vec<Uuid>,
}

impl ReinstateProducts {
    /// Validate the request's own fields; the store validates them against the project.
    pub fn validate(&self) -> Result<()> {
        validate_scope_request(&self.request_id, self.expected_version, &self.product_ids)
    }
}

fn validate_scope_request(request_id: &str, expected_version: u64, product_ids: &[Uuid]) -> Result<()> {
    validate_request_id(request_id, "quotation_scope.request_id")?;
    if expected_version == 0 {
        return Err(ContractError::invalid("quotation_scope.expected_version", "an existing project starts at version 1"));
    }
    if product_ids.is_empty() {
        return Err(ContractError::invalid("quotation_scope.product_ids", "name at least one product"));
    }
    let mut unique = Vec::with_capacity(product_ids.len());
    for product_id in product_ids {
        if unique.contains(product_id) {
            return Err(ContractError::invalid("quotation_scope.product_ids", "one product is named twice"));
        }
        unique.push(*product_id);
    }
    Ok(())
}

fn validate_request_id(request_id: &str, field: &str) -> Result<()> {
    if request_id.trim().is_empty() || request_id.chars().count() > 128 {
        return Err(ContractError::invalid(field, "a request id is non-blank and at most 128 characters"));
    }
    Ok(())
}

fn validate_key(value: &str, field: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(ContractError::invalid(field, "the value must not be blank"));
    }
    if value.chars().count() > MAX_KEY_CHARS {
        return Err(ContractError::invalid(field, format!("the value is at most {MAX_KEY_CHARS} characters")));
    }
    Ok(())
}

fn validate_text(value: &str, field: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(ContractError::invalid(field, "the value must not be blank"));
    }
    if value.chars().count() > MAX_NAME_CHARS {
        return Err(ContractError::invalid(field, format!("the value is at most {MAX_NAME_CHARS} characters")));
    }
    Ok(())
}

fn validate_currency(currency: &str) -> Result<()> {
    if currency.trim().is_empty() || currency.chars().count() > 8 {
        return Err(ContractError::invalid("project.currency", "the project currency is a non-blank short code"));
    }
    Ok(())
}

/// Whether two measured volumes validate one identity within the relative tolerance.
pub fn volumes_match(left: Decimal, right: Decimal) -> bool {
    // Both sides are non-negative in this contract; refuse anything else instead of panicking.
    if left.is_sign_negative() || right.is_sign_negative() {
        return false;
    }
    if left == right {
        return true;
    }
    let Some(difference) = left.checked_sub(right) else { return false };
    let difference = if difference.is_sign_negative() { -difference } else { difference };
    let largest = if left > right { left } else { right };
    if largest.is_zero() {
        return difference.is_zero();
    }
    // Divide by the tolerance rather than multiplying the difference: a scaled difference at the
    // decimal limit would overflow, while a quotient cannot.
    difference <= largest / Decimal::from(PART_MATCH_RELATIVE_TOLERANCE)
}

/// Compare an already-stored volume with a delivered one, when both exist.
pub fn geometry_validates(stored: Option<Decimal>, delivered: Option<Decimal>) -> Option<bool> {
    match (stored, delivered) {
        (Some(stored), Some(delivered)) => Some(volumes_match(stored, delivered)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AnalysisKey, Measurement, MeasurementSource, ReviewState, Unit};

    fn source() -> SourceRef {
        SourceRef { source_id: Uuid::from_u128(1), sha256: "a".repeat(64), byte_len: 10, original_name: "three_boxes.step".into() }
    }

    fn details() -> ProjectDetails {
        ProjectDetails {
            name: "Inquiry".into(),
            customer: "Customer".into(),
            internal_team: None,
            platform: Some("P1".into()),
            application: None,
            currency: "CNY".into(),
            default_schedule: YearRange { sop_year: 2026, eop_year: 2028 },
        }
    }

    fn package(id: u128) -> DeliveryPackage {
        DeliveryPackage { package_id: Uuid::from_u128(id), name: "Package 1".into(), source: source() }
    }

    fn product(id: u128, revision_id: u128, package_id: u128) -> Product {
        Product {
            product_id: Uuid::from_u128(id),
            name: format!("Product {id}"),
            kind: ProductKind::Assembly,
            customer_part_number: None,
            schedule_override: None,
            forecasts: vec![
                AnnualForecast { year: 2026, quantity: 100 },
                AnnualForecast { year: 2027, quantity: 200 },
                AnnualForecast { year: 2028, quantity: 300 },
            ],
            revisions: vec![ProductRevision {
                product_revision_id: Uuid::from_u128(revision_id),
                package_id: Uuid::from_u128(package_id),
                state: ProductRevisionState::Draft,
                parts: vec![DeliveredPart {
                    delivered_key: "def-1".into(),
                    customer_part_number: Some("PL-100".into()),
                    customer_revision: Some("B".into()),
                    volume_mm3: Some(Decimal::from(2400)),
                    part_version_id: Some(Uuid::from_u128(30)),
                }],
            }],
            active_revision_id: None,
            quotation_state: QuotationState::Included,
        }
    }

    fn document() -> ProjectDocument {
        ProjectDocument {
            schema_version: PROJECT_DOCUMENT_SCHEMA_VERSION,
            project_id: Uuid::from_u128(10),
            name: "Inquiry".into(),
            customer: "Customer".into(),
            internal_team: None,
            platform: None,
            application: None,
            currency: "CNY".into(),
            default_schedule: YearRange { sop_year: 2026, eop_year: 2028 },
            products: vec![product(2, 20, 1)],
            packages: vec![package(1)],
            part_versions: vec![PartVersion {
                part_version_id: Uuid::from_u128(30),
                customer_part_number: "PL-100".into(),
                customer_revision: "B".into(),
                volume_mm3: Some(Decimal::from(2400)),
                created_in_package_id: Uuid::from_u128(1),
            }],
            quotation_events: Vec::new(),
            costing: None,
            tooling: None,
            commercial: None,
        }
    }

    #[test]
    fn a_complete_project_document_validates() {
        let document = document();
        assert!(document.validate().is_ok());
        assert!(document.issues().is_empty());
    }

    #[test]
    fn a_missing_forecast_is_an_issue_not_zero() {
        let mut document = document();
        document.products[0].forecasts.retain(|forecast| forecast.year != 2027);
        assert!(document.validate().is_ok());
        let issues = document.issues();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].code, "missing_forecast");
        assert_eq!(issues[0].product_id, Some(Uuid::from_u128(2)));
    }

    #[test]
    fn a_zero_forecast_year_is_invalid() {
        let mut document = document();
        document.products[0].forecasts[0].quantity = 0;
        assert!(document.validate().is_err());
    }

    #[test]
    fn years_outside_the_schedule_are_invalid_and_never_padded() {
        let mut document = document();
        document.products[0].forecasts.push(AnnualForecast { year: 2029, quantity: 1 });
        assert!(document.validate().is_err());
    }

    #[test]
    fn a_product_override_changes_the_schedule_only_for_that_product() {
        let mut document = document();
        document.products[0].schedule_override = Some(YearRange { sop_year: 2027, eop_year: 2028 });
        document.products[0].forecasts.retain(|forecast| forecast.year >= 2027);
        assert!(document.validate().is_ok());
        assert!(document.products[0].missing_forecast_years(document.default_schedule).is_empty());
    }

    #[test]
    fn different_customer_revisions_are_different_part_versions() {
        let mut document = document();
        document.part_versions.push(PartVersion {
            part_version_id: Uuid::from_u128(31),
            customer_part_number: "PL-100".into(),
            customer_revision: "C".into(),
            volume_mm3: Some(Decimal::from(2400)),
            created_in_package_id: Uuid::from_u128(1),
        });
        assert!(document.validate().is_ok());
        assert_eq!(document.part_version_by_identity("PL-100", "C").map(|part| part.part_version_id), Some(Uuid::from_u128(31)));
    }

    #[test]
    fn the_same_customer_identity_cannot_be_two_part_versions() {
        let mut document = document();
        document.part_versions.push(PartVersion {
            part_version_id: Uuid::from_u128(31),
            customer_part_number: " PL-100 ".into(),
            customer_revision: "B".into(),
            volume_mm3: None,
            created_in_package_id: Uuid::from_u128(1),
        });
        assert!(document.validate().is_err());
    }

    #[test]
    fn an_unassociated_delivered_part_is_reported_until_confirmed() {
        let mut document = document();
        document.products[0].revisions[0].parts[0].part_version_id = None;
        document.products[0].revisions[0].parts[0].volume_mm3 = Some(Decimal::from(4800));
        let issues = document.issues();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].code, "conflicting_geometry_match");

        document.products[0].revisions[0].parts[0].customer_part_number = None;
        let issues = document.issues();
        assert_eq!(issues[0].code, "missing_part_identity");

        document.products[0].revisions[0].parts[0].customer_part_number = Some("PL-100".into());
        document.products[0].revisions[0].parts[0].volume_mm3 = None;
        let issues = document.issues();
        assert_eq!(issues[0].code, "missing_geometry_for_match");
    }

    #[test]
    fn a_product_cannot_have_two_active_revisions() {
        let mut document = document();
        document.products[0].revisions[0].state = ProductRevisionState::Active;
        let mut second = document.products[0].revisions[0].clone();
        second.product_revision_id = Uuid::from_u128(21);
        second.state = ProductRevisionState::Active;
        document.products[0].revisions.push(second);
        assert!(document.validate().is_err());
    }

    #[test]
    fn the_active_revision_pointer_must_name_the_active_revision() {
        let mut document = document();
        document.products[0].active_revision_id = Some(Uuid::from_u128(20));
        assert!(document.validate().is_err());
        document.products[0].revisions[0].state = ProductRevisionState::Active;
        assert!(document.validate().is_ok());
    }

    #[test]
    fn an_excluded_product_needs_its_recorded_event() {
        let mut document = document();
        document.products[0].quotation_state = QuotationState::Excluded;
        assert!(document.validate().is_err());
        let sequence = document.next_quotation_sequence();
        document.quotation_events.push(QuotationEvent { sequence, product_ids: vec![Uuid::from_u128(2)], state: QuotationState::Excluded });
        assert!(document.validate().is_ok());
        assert_eq!(sequence, 1);
    }

    #[test]
    fn extreme_volumes_do_not_overflow_the_match() {
        assert!(!volumes_match(Decimal::MAX, Decimal::ZERO));
        assert!(volumes_match(Decimal::MAX, Decimal::MAX));
        assert!(!volumes_match(Decimal::ZERO, Decimal::MAX));
        assert!(volumes_match(Decimal::ZERO, Decimal::ZERO));
        assert!(!volumes_match(Decimal::from(1), Decimal::from(2)));
    }

    #[test]
    fn a_confirmed_match_cannot_cross_customer_revisions() {
        let mut document = document();
        document.part_versions.push(PartVersion {
            part_version_id: Uuid::from_u128(31),
            customer_part_number: "PL-100".into(),
            customer_revision: "C".into(),
            volume_mm3: Some(Decimal::from(2400)),
            created_in_package_id: Uuid::from_u128(1),
        });
        document.products[0].revisions[0].parts[0].part_version_id = Some(Uuid::from_u128(31));
        assert!(document.validate().is_err(), "B must not be associated with the C part version");
        document.products[0].revisions[0].parts[0].part_version_id = Some(Uuid::from_u128(30));
        assert!(document.validate().is_ok());
    }

    #[test]
    fn geometry_matches_inside_the_relative_tolerance_only() {
        assert!(volumes_match(Decimal::from(1000), Decimal::from(1000)));
        assert!(volumes_match(Decimal::from(1000), Decimal::from(1001)));
        assert!(!volumes_match(Decimal::from(1000), Decimal::from(1002)));
        assert!(volumes_match(Decimal::ZERO, Decimal::ZERO));
        assert!(!volumes_match(Decimal::ZERO, Decimal::from(1)));
        assert_eq!(geometry_validates(None, Some(Decimal::from(1))), None);
        assert_eq!(geometry_validates(Some(Decimal::from(1000)), Some(Decimal::from(1001))), Some(true));
    }

    #[test]
    fn the_requests_validate_their_own_fields() {
        let details = details();
        assert!(CreateProject { request_id: "r".into(), project_id: Uuid::from_u128(1), details: details.clone() }.validate().is_ok());
        assert!(CreateProject { request_id: " ".into(), project_id: Uuid::from_u128(1), details: details.clone() }.validate().is_err());
        assert!(UpdateProject { request_id: "r".into(), expected_version: 0, details }.validate().is_err());
        assert!(
            UpdateProduct {
                request_id: "r".into(),
                expected_version: 2,
                product_id: Uuid::from_u128(2),
                name: "Product".into(),
                kind: ProductKind::Part,
                customer_part_number: None,
                schedule_override: None,
                forecasts: vec![AnnualForecast { year: 2026, quantity: 1 }, AnnualForecast { year: 2026, quantity: 2 }],
            }
            .validate()
            .is_err()
        );
        assert!(ExcludeProducts { request_id: "r".into(), expected_version: 1, product_ids: vec![] }.validate().is_err());
        assert!(ReinstateProducts { request_id: "r".into(), expected_version: 1, product_ids: vec![Uuid::from_u128(2)] }.validate().is_ok());
    }

    #[test]
    fn a_stored_document_keeps_wire_compatibility_with_unknown_fields() {
        let value = serde_json::to_value(document()).unwrap_or_default();
        assert_eq!(value["schema_version"], serde_json::json!(1));
        assert_eq!(value["default_schedule"]["sop_year"], serde_json::json!(2026));
        let mut with_unknown = value.clone();
        with_unknown["future_field"] = serde_json::json!("tolerated");
        assert!(serde_json::from_value::<ProjectDocument>(with_unknown).is_ok());
        let round_trip = serde_json::from_value::<ProjectDocument>(value).ok();
        assert_eq!(round_trip.as_ref(), Some(&document()));
    }

    #[test]
    fn a_project_needs_a_supported_schema_version() {
        let mut document = document();
        document.schema_version = PROJECT_DOCUMENT_SCHEMA_VERSION + 1;
        match document.validate() {
            Err(ContractError::SchemaVersion { found, expected }) => {
                assert_eq!(found, PROJECT_DOCUMENT_SCHEMA_VERSION + 1);
                assert_eq!(expected, PROJECT_DOCUMENT_SCHEMA_VERSION);
            }
            other => assert!(other.is_ok(), "a newer schema must be refused"),
        }
    }

    #[test]
    fn the_source_measurement_types_stay_reusable_with_the_document() {
        let _ = Measurement {
            value: Some(Decimal::from(2400)),
            unit: Unit::MillimetreCubed,
            source: MeasurementSource::Extracted,
            review: ReviewState::Confirmed,
            reason: None,
            analysis_key: AnalysisKey { source_sha256: "a".repeat(64), engine_build: "b".into(), extractor_version: "1".into() },
        };
    }
}
