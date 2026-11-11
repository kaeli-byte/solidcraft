//! The snapshot-resolved export model shared by the two internal-export renderers.
//!
//! The service resolves one selected saved result (an accepted baseline or the project's current
//! draft calculation snapshot) into this model once, and both renderers draw it: the Excel workbook
//! and the summary PDF never read the database, never recompute a cost and never decide which
//! values are valid. Every cost, demand and price value here is copied from the stored snapshot
//! output or a stored record; every identifier here was read from the record versions the snapshot
//! pinned (and from the feasibility record, whose own version the metadata records).
//!
//! A renderer prints the values it finds. It formats numbers, wraps labels, paginates and names the
//! stable labels; it does not add, convert or re-resolve anything.

use rust_decimal::Decimal;
use solidbom_contract::{
    AnnualForecast, BaselineAnnualDemand, BaselineAnnualProjectCost, BaselineAttachedCostInput, BaselineCategoryLine, BaselineCycleInput,
    BaselineMaterialCost, BaselineProductPartLine, BaselineSelectedPurchasePrice, BaselineSetupInput, BaselineSetupLine, CommercialOutcome,
    CommercialStaleReason, ExportMetadata, FeasibilityRecordView, ProductKind, ProductRevisionState, QuotationState, Sourcing, ToolingOutcome,
    YearRange,
};
use uuid::Uuid;

/// One resolved export, ready for either renderer.
pub struct ExportModel {
    /// What the documents state about the saved result they render.
    pub metadata: ExportMetadata,
    /// The project and product scope the selected result covers.
    pub scope: ExportScope,
    /// One row per shared part version in the selected scope, in part-version-id order.
    pub parts: Vec<ExportPart>,
    /// One row per product the selected result costs, in product-id order.
    pub products: Vec<ExportProduct>,
    /// The snapshot's separate setup lines, in the stored order.
    pub setup: Vec<BaselineSetupLine>,
    /// The project setup-scope categories the snapshot evaluated once.
    pub setup_categories: Vec<BaselineCategoryLine>,
    /// The separate setup total; `None` is missing, never zero.
    pub setup_total: Option<Decimal>,
    /// The stored actual calendar-year project cost series, ascending years.
    pub annual: Vec<BaselineAnnualProjectCost>,
    /// The stored lifetime manufacturing total including the separate setup total.
    pub project_total_manufacturing_cost: Option<Decimal>,
    /// The tooling investment, allocation and recovery of the same selected result.
    pub tooling: Option<ToolingOutcome>,
    /// The saved commercial scenario comparisons, in the requested scenario order.
    pub scenarios: Vec<ExportScenario>,
    /// The feasibility record the documents carry.
    pub feasibility: Option<ExportFeasibility>,
    /// Every missing input, unresolved finding, issue and stale-cost statement the documents list.
    pub issues: Vec<ExportIssue>,
}

/// The project and product scope one export describes.
pub struct ExportScope {
    pub project_name: String,
    pub customer: String,
    pub internal_team: Option<String>,
    pub platform: Option<String>,
    pub application: Option<String>,
    /// The project currency every amount in the documents is stated in.
    pub currency: String,
    pub default_schedule: YearRange,
    /// One row per product in the selected result's scope, in product-id order.
    pub products: Vec<ExportScopedProduct>,
}

/// One product as the selected result sees it: the revision it costed and the stored schedule.
pub struct ExportScopedProduct {
    pub product_id: Uuid,
    pub name: String,
    pub kind: ProductKind,
    pub customer_part_number: Option<String>,
    pub quotation_state: QuotationState,
    /// The revision the selected result costed; `None` when the snapshot resolved no revision.
    pub product_revision_id: Option<Uuid>,
    /// The stored state of that revision; `None` when no revision was resolved.
    pub revision_state: Option<ProductRevisionState>,
    pub schedule: YearRange,
    pub forecasts: Vec<AnnualForecast>,
}

impl ExportScopedProduct {
    /// The product's stored name, or its identity when it is no longer in the project document.
    pub fn display_name(&self) -> String {
        if self.name.trim().is_empty() { self.product_id.to_string() } else { self.name.clone() }
    }
}

/// One shared part version as the selected result sees it: identity, review, manufacturing
/// definition and the stored cost values of one occurrence.
pub struct ExportPart {
    pub part_version_id: Uuid,
    pub customer_part_number: String,
    pub customer_revision: String,
    pub created_in_package_id: Uuid,
    /// The engineer-confirmed classification name, when one is confirmed.
    pub classification: Option<String>,
    /// Whether the classification is engineer-confirmed.
    pub classification_confirmed: bool,
    /// The reviewed volume, when one is recorded.
    pub reviewed_volume_mm3: Option<Decimal>,
    pub sourcing: Option<Sourcing>,
    /// The stored own-material record of a manufactured part; `None` for a purchased part.
    pub material: Option<BaselineMaterialCost>,
    /// The stored selected purchase price of a purchased part.
    pub purchase: Option<BaselineSelectedPurchasePrice>,
    /// The stored attached non-CAD cost lines this part version owns.
    pub attached_costs: Vec<BaselineAttachedCostInput>,
    /// The stored routing operations this part version owns; empty when it has no routing.
    pub operations: Vec<ExportOperation>,
    /// The applicable annual demand, ascending years.
    pub annual_demand: Vec<BaselineAnnualDemand>,
    pub lifetime_demand: Option<u64>,
    pub peak_demand: Option<u64>,
    pub costing_volume: Option<Decimal>,
    /// The rolled material content of one occurrence.
    pub material_cost: Option<Decimal>,
    /// The rolled production cost of one occurrence.
    pub production_cost: Option<Decimal>,
    /// The rolled attached cost of one occurrence.
    pub attached_cost: Option<Decimal>,
    /// The rolled unit cost of one occurrence; unit-scope categories apply at the product instead.
    pub unit_cost: Option<Decimal>,
    /// The part routing's separate setup charge, counted once per project quotation.
    pub setup_charge: Option<Decimal>,
}

impl ExportPart {
    /// The stored part number, or the identity when the project document keeps none.
    pub fn display_name(&self) -> String {
        if self.customer_part_number.trim().is_empty() { self.part_version_id.to_string() } else { self.customer_part_number.clone() }
    }
}

/// One routing operation of a part version or a product root.
pub struct ExportOperation {
    pub sequence: u32,
    pub operation_id: Uuid,
    /// The stored cycle and pinned production rate; `None` is a missing cycle.
    pub cycle: Option<BaselineCycleInput>,
    /// The stored setup and pinned setup rate; `None` means no setup applies.
    pub setup: Option<BaselineSetupInput>,
}

/// One product the selected result costs, with its stored cost values.
pub struct ExportProduct {
    pub product_id: Uuid,
    pub name: String,
    pub kind: ProductKind,
    pub customer_part_number: Option<String>,
    pub quotation_state: QuotationState,
    pub product_revision_id: Option<Uuid>,
    pub revision_state: Option<ProductRevisionState>,
    pub schedule: YearRange,
    pub forecasts: Vec<AnnualForecast>,
    /// Whether the selected result counted this product into demand and cost.
    pub quoted: bool,
    /// The product-root routing operations; empty when the root has none.
    pub root_operations: Vec<ExportOperation>,
    /// The attached non-CAD cost lines the product root owns.
    pub root_attached_costs: Vec<BaselineAttachedCostInput>,
    /// The expanded non-suppressed part quantities in one finished product, by part version id.
    pub parts: Vec<BaselineProductPartLine>,
    pub lifetime_demand: Option<u64>,
    pub material_cost: Option<Decimal>,
    pub production_cost: Option<Decimal>,
    pub attached_cost: Option<Decimal>,
    pub root_production_cost: Option<Decimal>,
    pub root_attached_cost: Option<Decimal>,
    pub categories: Vec<BaselineCategoryLine>,
    /// The per-unit manufacturing cost including fixed/percentage categories; setup excluded.
    pub unit_total: Option<Decimal>,
    /// The recurring unit cost a commercial scenario prices.
    pub recurring_unit_cost: Option<Decimal>,
}

impl ExportProduct {
    /// The stored product name, or its identity when the project document keeps none.
    pub fn display_name(&self) -> String {
        if self.name.trim().is_empty() { self.product_id.to_string() } else { self.name.clone() }
    }
}

/// One saved commercial scenario evaluated against the exported result.
pub struct ExportScenario {
    pub scenario_id: Uuid,
    pub version: u64,
    pub name: String,
    /// Whether the saved version was pinned to another result, project version or setting.
    pub stale: bool,
    pub stale_reasons: Vec<CommercialStaleReason>,
    /// The evaluated prices, revenues, EBIT and tooling answer over the export's snapshot.
    pub outcome: CommercialOutcome,
}

/// The stored feasibility record the documents carry.
pub struct ExportFeasibility {
    /// The feasibility record version read.
    pub version: u64,
    /// Whether an accepted baseline pinned the version, or it was read at export time.
    pub pinned: bool,
    /// The record's findings, exception dispositions and issues against the pinned review.
    pub view: FeasibilityRecordView,
}

/// Where one listed issue came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportIssueSource {
    Costing,
    Tooling,
    Commercial,
    Feasibility,
    Staleness,
}

impl ExportIssueSource {
    /// The stable contract name a renderer prints in the source column.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Costing => "costing",
            Self::Tooling => "tooling",
            Self::Commercial => "commercial",
            Self::Feasibility => "feasibility",
            Self::Staleness => "staleness",
        }
    }
}

/// One missing input, unresolved finding, issue or stale-cost statement.
pub struct ExportIssue {
    pub source: ExportIssueSource,
    /// The stable issue code the owning contract defined.
    pub code: String,
    /// The affected part version or product, when the issue names one.
    pub scope: Option<Uuid>,
    /// The stored explanation; the renderer prints it as written.
    pub message: String,
}
