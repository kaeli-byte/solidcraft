//! Sourcing and manufacturing definitions: the manufacturing contract of package SB-15.
//!
//! One project records how each shared project part version is sourced and made, beside its
//! engineering review and its project record:
//!
//! - A [`SourcedPart`] fixes the explicit purchased/manufactured choice of one project part
//!   version, with the material selection, net consumption and allowance of a made part. The
//!   record is keyed by `part_version_id`, so every occurrence and every product that references
//!   that shared part version reads the same choice while another customer revision stays a
//!   separate part version with its own record.
//! - A [`RoutingTemplateVersion`] is one published, immutable version of a reusable company
//!   routing. Templates are maintained by an Administrator, follow the append-only version chain
//!   used by the shared reference records and are never edited in place.
//! - An [`AppliedRouting`] is the local copy an engineer adopts from one exact template version
//!   (or enters without a template). It keeps the source/template-version link, is edited on its
//!   own and never writes back to the template; another instance of the same template is
//!   unaffected by an edit. Its [`RoutingOperation`]s are ordered, name their cost center, pin
//!   the separate production and setup rate versions, and declare the cycle basis and cavity
//!   conversion they mean.
//! - An [`AttachedCostLine`] adds packaging, adhesive, consumables or other non-CAD costs to a
//!   part or assembly with an explicit category, quantity and unit basis. It creates no CAD node
//!   and does not restructure the imported BOM.
//! - A [`ManufacturingRecordDocument`] is the append-only project document holding those records.
//!   A later write appends one immutable version, so accepted versions and earlier accepted
//!   snapshots stay unchanged.
//!
//! Rules mirrored from the confirmed requirements and the package brief:
//!
//! - A missing value stays `None`, never zero. A missing cycle time, cavity count, rate version,
//!   consumption or attached unit cost is a computed [`ManufacturingIssue`], not a silent zero.
//! - The cycle basis is explicit. `per_piece` seconds already describe one finished piece and are
//!   never divided by cavities; `per_cycle` seconds describe one machine cycle shared by its
//!   cavities, and only that declared basis divides by the declared cavity count.
//! - A purchased part records no manufacturing material consumption; its price and descendant
//!   cutoff belong to the costing package.
//! - Setup is a duration with its own dedicated setup rate version, separate from production.
//! - Assembly installation/joining/testing operations stay on the assembly's own routing; they are
//!   not copied into the child parts.
//! - The version 1 prototype calculator (`OperationInputs`) keeps its published behavior; this
//!   contract defines the routing operation the later costing package evaluates.
//!
//! Wire rules follow version 1 of the contract: decimals serialize as strings, a missing value is
//! `null`, unknown JSON fields are tolerated, and a stored document names the schema version it was
//! written with.

use std::collections::BTreeSet;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{ContractError, Result};
use crate::projects::ProjectDocument;
use crate::types::{AnalysisKey, CycleBasis, MeasurementSource, ReviewState, Sourcing, Unit, check_text};

/// The manufacturing contract version this build reads and writes. It extends the published
/// version 2 HTTP API additively; it does not change a version 1 or version 2 meaning.
pub const MANUFACTURING_CONTRACT_VERSION: u32 = 1;

/// The schema version written into a stored manufacturing record document.
pub const MANUFACTURING_DOCUMENT_SCHEMA_VERSION: u32 = 1;

/// The schema version written into a stored routing template version document.
pub const ROUTING_TEMPLATE_DOCUMENT_SCHEMA_VERSION: u32 = 1;

/// Resource bounds for hostile input. A manufacturing record is not a dump of arbitrary size.
pub const MAX_SOURCED_PARTS: usize = 100_000;
pub const MAX_APPLIED_ROUTINGS: usize = 100_000;
pub const MAX_OPERATIONS: usize = 10_000;
pub const MAX_ATTACHED_COST_LINES: usize = 100_000;
pub const MAX_TEMPLATE_VERSIONS: usize = 10_000;
pub const MAX_ROUTING_LEVELS: usize = 3;
pub const MAX_AFFECTED_REFS: usize = 100_000;
pub const MAX_MANUFACTURING_NAME_CHARS: usize = 255;
pub const MAX_DESCRIPTION_CHARS: usize = 1024;
pub const MAX_UNIT_CHARS: usize = 64;
pub const MAX_REFERENCE_CHARS: usize = 1024;
pub const MAX_MANUFACTURING_REQUEST_ID_CHARS: usize = 128;

/// At which manufacturing level a routing applies: one part, one subassembly or one product root.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum RoutingLevel {
    Part,
    Subassembly,
    ProductRoot,
}

impl RoutingLevel {
    /// Whether this level is the product root rather than a project part version.
    pub fn is_product_root(self) -> bool {
        matches!(self, Self::ProductRoot)
    }
}

/// Who owns an applied routing or an attached cost line: one shared project part version (a part or
/// subassembly) or one product root.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CostOwner {
    PartVersion { part_version_id: Uuid },
    ProductRoot { product_id: Uuid },
}

impl CostOwner {
    /// The shared part version this owner names, when it owns a part rather than a product root.
    pub fn part_version_id(self) -> Option<Uuid> {
        match self {
            Self::PartVersion { part_version_id } => Some(part_version_id),
            Self::ProductRoot { .. } => None,
        }
    }

    /// The product this owner names, when it owns a product root.
    pub fn product_id(self) -> Option<Uuid> {
        match self {
            Self::PartVersion { .. } => None,
            Self::ProductRoot { product_id } => Some(product_id),
        }
    }

    fn validate(self, field: &str) -> Result<()> {
        match self {
            Self::PartVersion { part_version_id } if part_version_id.is_nil() => {
                Err(ContractError::invalid(format!("{field}.part_version_id"), "the part version id is empty"))
            }
            Self::ProductRoot { product_id } if product_id.is_nil() => {
                Err(ContractError::invalid(format!("{field}.product_id"), "the product id is empty"))
            }
            _ => Ok(()),
        }
    }

    /// Whether this owner is allowed at the declared routing level.
    fn matches_level(self, level: RoutingLevel) -> bool {
        match self {
            Self::PartVersion { .. } => !level.is_product_root(),
            Self::ProductRoot { .. } => level.is_product_root(),
        }
    }
}

/// How a cycle figure came about. A calculated figure names the formula/version that produced it,
/// so a later geometry change can be told apart from a manual entry.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CycleOrigin {
    Manual,
    Calculated,
}

/// The production cycle of one operation. Every figure stays optional until entered.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationCycle {
    /// `None` is a missing cycle time, distinct from an explicit zero.
    pub seconds: Option<Decimal>,
    /// The declared basis. Only [`CycleBasis::PerCycle`] divides by the cavity count.
    pub basis: CycleBasis,
    /// The cavities this operation runs. Required for the `per_cycle` basis; informational and at
    /// most one for `per_piece`.
    pub cavities: Option<u32>,
    /// The selected production cost-center rate version; updating the cost center never changes it.
    pub production_rate_version_id: Option<Uuid>,
    pub origin: CycleOrigin,
    /// The formula/version a calculated figure came from; not used for a manual entry.
    pub source_reference: Option<String>,
}

impl OperationCycle {
    pub fn validate(&self, field: &str) -> Result<()> {
        if let Some(seconds) = self.seconds
            && seconds.is_sign_negative()
        {
            return Err(ContractError::invalid(format!("{field}.seconds"), "a cycle time cannot be negative"));
        }
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
        if self.origin == CycleOrigin::Calculated {
            match self.source_reference.as_deref() {
                Some(reference) if !reference.trim().is_empty() => {}
                _ => return Err(ContractError::invalid(format!("{field}.source_reference"), "a calculated cycle names its formula/version")),
            }
        }
        check_optional_reference(&self.source_reference, &format!("{field}.source_reference"))
    }

    /// The per-piece seconds the declared basis gives. Missing data stays `None`: this never
    /// substitutes a zero or an implied cavity count.
    pub fn per_piece_seconds(&self) -> Option<Decimal> {
        let seconds = self.seconds?;
        if self.basis.adjusts_for_cavities() {
            let cavities = self.cavities?;
            if cavities == 0 {
                return None;
            }
            seconds.checked_div(Decimal::from(cavities))
        } else {
            Some(seconds)
        }
    }
}

/// The setup of one operation: a duration and the dedicated setup rate version that prices it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SetupTime {
    /// `None` is a missing setup duration, distinct from an explicit zero.
    pub hours: Option<Decimal>,
    /// The selected setup cost-center rate version, kept separate from the production rate.
    pub setup_rate_version_id: Option<Uuid>,
    pub source_reference: Option<String>,
}

impl SetupTime {
    pub fn validate(&self, field: &str) -> Result<()> {
        if let Some(hours) = self.hours
            && hours.is_sign_negative()
        {
            return Err(ContractError::invalid(format!("{field}.hours"), "setup time cannot be negative"));
        }
        check_optional_reference(&self.source_reference, &format!("{field}.source_reference"))
    }
}

/// One ordered manufacturing operation of a routing or routing template.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoutingOperation {
    pub operation_id: Uuid,
    /// The operation's place in the routing, starting at 1 and strictly increasing.
    pub sequence: u32,
    pub name: String,
    pub description: Option<String>,
    /// The cost center the operation runs in; its rate versions are pinned by the cycle and setup.
    pub cost_center_id: Option<Uuid>,
    /// `None` means no cycle has been recorded yet (a computed issue), never a zero cycle.
    pub cycle: Option<OperationCycle>,
    /// `None` means no setup applies to this operation; a recorded setup with missing figures is
    /// an incomplete setup and stays an issue.
    pub setup: Option<SetupTime>,
    /// The applicable tooling/cavity reference, when the operation names one.
    pub tooling_reference: Option<String>,
}

impl RoutingOperation {
    pub fn validate(&self, field: &str) -> Result<()> {
        if self.operation_id.is_nil() {
            return Err(ContractError::invalid(format!("{field}.operation_id"), "the operation id is empty"));
        }
        if self.sequence == 0 {
            return Err(ContractError::invalid(format!("{field}.sequence"), "an operation sequence starts at 1"));
        }
        check_name(&self.name, &format!("{field}.name"))?;
        check_optional_description(&self.description, &format!("{field}.description"))?;
        check_optional_reference(&self.tooling_reference, &format!("{field}.tooling_reference"))?;
        if let Some(cycle) = &self.cycle {
            cycle.validate(&format!("{field}.cycle"))?;
        }
        if let Some(setup) = &self.setup {
            setup.validate(&format!("{field}.setup"))?;
        }
        Ok(())
    }
}

/// The material selection, net consumption and allowance of one made shared part version.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MaterialConsumption {
    /// The selected immutable material version. `None` is missing, not a default material.
    pub material_version_id: Option<Uuid>,
    /// The net required consumption before the allowance. `None` stays missing; SB-16 derives
    /// mass from the selected material and this quantity.
    pub quantity: Option<Decimal>,
    /// The explicit consumption unit: a volume in `mm3` or `cm3`.
    pub unit: Unit,
    pub source: MeasurementSource,
    pub review: ReviewState,
    /// The authenticated engineer who confirmed the consumption. The service fills this from the
    /// caller identity, never from the payload; a stored confirmed consumption always carries it.
    pub confirmed_by: Option<String>,
    /// The exact analysis an extracted consumption was derived from. A manual override does not
    /// name one.
    pub analysis_key: Option<AnalysisKey>,
    /// Why a manual value was entered; required for manual values.
    pub reason: Option<String>,
    /// The simple material allowance rate; `Some(0)` means no allowance, `None` means not entered.
    pub allowance_rate: Option<Decimal>,
    pub source_reference: Option<String>,
}

impl MaterialConsumption {
    pub fn validate(&self, field: &str) -> Result<()> {
        if !matches!(self.unit, Unit::MillimetreCubed | Unit::CentimetreCubed) {
            return Err(ContractError::invalid(format!("{field}.unit"), "consumption is a volume in mm3 or cm3"));
        }
        if let Some(quantity) = self.quantity
            && quantity.is_sign_negative()
        {
            return Err(ContractError::invalid(format!("{field}.quantity"), "consumption cannot be negative"));
        }
        if self.review == ReviewState::Confirmed && self.quantity.is_none() {
            return Err(ContractError::invalid(format!("{field}.review"), "a confirmed consumption needs a value"));
        }
        match self.source {
            MeasurementSource::Manual => match self.reason.as_deref() {
                Some(reason) if !reason.trim().is_empty() => {}
                _ => return Err(ContractError::invalid(format!("{field}.reason"), "a manual consumption needs a reason")),
            },
            MeasurementSource::Extracted => {
                if self.analysis_key.is_none() {
                    return Err(ContractError::invalid(
                        format!("{field}.analysis_key"),
                        "an extracted consumption names the analysis it was derived from",
                    ));
                }
            }
        }
        if let Some(analysis_key) = &self.analysis_key {
            analysis_key.validate()?;
        }
        if let Some(confirmed_by) = &self.confirmed_by {
            check_text(confirmed_by, &format!("{field}.confirmed_by"))?;
            if confirmed_by.chars().count() > MAX_MANUFACTURING_NAME_CHARS {
                return Err(ContractError::invalid(format!("{field}.confirmed_by"), "the reviewer name is too long"));
            }
        }
        if let Some(allowance) = self.allowance_rate
            && allowance.is_sign_negative()
        {
            return Err(ContractError::invalid(format!("{field}.allowance_rate"), "an allowance cannot be negative"));
        }
        check_optional_description(&self.reason, &format!("{field}.reason"))?;
        check_optional_reference(&self.source_reference, &format!("{field}.source_reference"))
    }
}

/// The explicit manufacturing definition of one shared project part version.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourcedPart {
    /// The shared project part version this record belongs to. Another customer revision is a
    /// different part version with its own record.
    pub part_version_id: Uuid,
    /// The explicit purchased or internally manufactured choice.
    pub sourcing: Sourcing,
    /// The material/consumption record of a made part. A purchased part carries none: its price
    /// covers its descendants and belongs to the costing package.
    pub material: Option<MaterialConsumption>,
    pub source_reference: Option<String>,
}

impl SourcedPart {
    pub fn validate(&self, field: &str) -> Result<()> {
        if self.part_version_id.is_nil() {
            return Err(ContractError::invalid(format!("{field}.part_version_id"), "the part version id is empty"));
        }
        if self.sourcing == Sourcing::Purchased && self.material.is_some() {
            return Err(ContractError::invalid(format!("{field}.material"), "a purchased part records no manufacturing material consumption"));
        }
        if let Some(material) = &self.material {
            material.validate(&format!("{field}.material"))?;
        }
        check_optional_reference(&self.source_reference, &format!("{field}.source_reference"))
    }
}

/// One exact published template version a routing was copied from.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct TemplateRef {
    pub template_id: Uuid,
    pub version: u64,
}

impl TemplateRef {
    pub fn validate(&self, field: &str) -> Result<()> {
        if self.template_id.is_nil() {
            return Err(ContractError::invalid(format!("{field}.template_id"), "the template id is empty"));
        }
        if self.version == 0 {
            return Err(ContractError::invalid(format!("{field}.version"), "a template version starts at 1"));
        }
        Ok(())
    }
}

/// Where an applied routing came from. A template link is retained and never updated silently.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RoutingSource {
    Template(TemplateRef),
    Manual,
}

impl RoutingSource {
    fn validate(&self, field: &str) -> Result<()> {
        match self {
            Self::Template(template) => template.validate(field),
            Self::Manual => Ok(()),
        }
    }

    /// The template version this routing was copied from, when it has one.
    pub fn template(self) -> Option<TemplateRef> {
        match self {
            Self::Template(template) => Some(template),
            Self::Manual => None,
        }
    }
}

/// The routing an engineer applies at one part, subassembly or product root.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppliedRouting {
    pub routing_id: Uuid,
    pub level: RoutingLevel,
    pub owner: CostOwner,
    /// The source template version the operations were copied from, or `Manual`.
    pub source: RoutingSource,
    /// The operations in sequence order. Editing them never changes the template.
    pub operations: Vec<RoutingOperation>,
}

impl AppliedRouting {
    pub fn validate(&self) -> Result<()> {
        if self.routing_id.is_nil() {
            return Err(ContractError::invalid("routing.routing_id", "the routing id is empty"));
        }
        self.owner.validate("routing.owner")?;
        if !self.owner.matches_level(self.level) {
            return Err(ContractError::invalid(
                "routing.level",
                "a product-root routing owns a product; a part or subassembly routing owns a part version",
            ));
        }
        self.source.validate("routing.source")?;
        validate_operations(&self.operations, "routing.operations", false)
    }
}

/// One additional non-CAD cost line attached to a part or assembly.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttachedCostLine {
    pub line_id: Uuid,
    pub owner: CostOwner,
    /// The explicit cost category, for example packaging, adhesive or consumables.
    pub category: String,
    pub description: Option<String>,
    /// How much of the item one unit of its owner consumes. `None` stays missing, never zero.
    pub quantity: Option<Decimal>,
    /// The explicit unit basis the quantity is stated in, for example `pcs`, `m` or `kg`.
    pub unit: String,
    /// The project-currency cost of one unit basis. `None` stays missing, never zero.
    pub unit_cost: Option<Decimal>,
    pub source_reference: Option<String>,
}

impl AttachedCostLine {
    pub fn validate(&self, field: &str) -> Result<()> {
        if self.line_id.is_nil() {
            return Err(ContractError::invalid(format!("{field}.line_id"), "the attached cost line id is empty"));
        }
        self.owner.validate(&format!("{field}.owner"))?;
        check_name(&self.category, &format!("{field}.category"))?;
        check_text(&self.unit, &format!("{field}.unit"))?;
        if self.unit.chars().count() > MAX_UNIT_CHARS {
            return Err(ContractError::invalid(format!("{field}.unit"), "the unit basis is too long"));
        }
        check_optional_description(&self.description, &format!("{field}.description"))?;
        check_optional_non_negative(self.quantity, &format!("{field}.quantity"), "a quantity cannot be negative")?;
        check_optional_non_negative(self.unit_cost, &format!("{field}.unit_cost"), "a unit cost cannot be negative")?;
        check_optional_reference(&self.source_reference, &format!("{field}.source_reference"))
    }
}

/// One computed problem of a manufacturing definition. Issues are never stored; a missing input is
/// an issue, not a zero, and an incomplete definition stays uncosted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManufacturingIssue {
    pub code: String,
    pub field_path: String,
    pub part_version_id: Option<Uuid>,
    pub product_id: Option<Uuid>,
    pub routing_id: Option<Uuid>,
    pub operation_id: Option<Uuid>,
    pub line_id: Option<Uuid>,
    pub message: String,
}

impl ManufacturingIssue {
    /// A named issue. `code` is a stable contract name, not display text.
    pub fn new(code: impl Into<String>, field_path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            field_path: field_path.into(),
            part_version_id: None,
            product_id: None,
            routing_id: None,
            operation_id: None,
            line_id: None,
            message: message.into(),
        }
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

    /// Attach the routing the issue is about when no single operation is.
    pub fn for_routing(mut self, routing_id: Uuid) -> Self {
        self.routing_id = Some(routing_id);
        self
    }

    /// Attach the routing and operation the issue is about.
    pub fn for_operation(mut self, routing_id: Uuid, operation_id: Uuid) -> Self {
        self.routing_id = Some(routing_id);
        self.operation_id = Some(operation_id);
        self
    }

    /// Attach the attached cost line the issue is about.
    pub fn for_line(mut self, line_id: Uuid) -> Self {
        self.line_id = Some(line_id);
        self
    }

    pub fn validate(&self) -> Result<()> {
        check_text(&self.code, "issue.code")?;
        check_text(&self.field_path, "issue.field_path")?;
        check_text(&self.message, "issue.message")
    }
}

/// The stored, appended manufacturing record of one project. History lives in the immutable record
/// versions; this document is the current one.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManufacturingRecordDocument {
    pub schema_version: u32,
    pub project_id: Uuid,
    pub parts: Vec<SourcedPart>,
    pub routings: Vec<AppliedRouting>,
    pub attached_costs: Vec<AttachedCostLine>,
}

impl ManufacturingRecordDocument {
    /// The empty record of a project with no manufacturing definition yet.
    pub fn empty(project_id: Uuid) -> Self {
        Self {
            schema_version: MANUFACTURING_DOCUMENT_SCHEMA_VERSION,
            project_id,
            parts: Vec::new(),
            routings: Vec::new(),
            attached_costs: Vec::new(),
        }
    }

    /// Validate a stored document before it is written and before clients read one.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != MANUFACTURING_DOCUMENT_SCHEMA_VERSION {
            return Err(ContractError::SchemaVersion { found: self.schema_version, expected: MANUFACTURING_DOCUMENT_SCHEMA_VERSION });
        }
        if self.parts.len() > MAX_SOURCED_PARTS {
            return Err(ContractError::invalid("manufacturing.parts", format!("a record holds at most {MAX_SOURCED_PARTS} sourced parts")));
        }
        if self.routings.len() > MAX_APPLIED_ROUTINGS {
            return Err(ContractError::invalid("manufacturing.routings", format!("a record holds at most {MAX_APPLIED_ROUTINGS} routings")));
        }
        if self.attached_costs.len() > MAX_ATTACHED_COST_LINES {
            return Err(ContractError::invalid(
                "manufacturing.attached_costs",
                format!("a record holds at most {MAX_ATTACHED_COST_LINES} attached cost lines"),
            ));
        }
        let mut part_versions = BTreeSet::new();
        for part in &self.parts {
            part.validate(&format!("manufacturing.parts.{}", part.part_version_id))?;
            if !part_versions.insert(part.part_version_id) {
                return Err(ContractError::invalid("manufacturing.parts", "two sourced records share one part version id"));
            }
        }
        let mut routing_ids = BTreeSet::new();
        let mut owners = BTreeSet::new();
        for routing in &self.routings {
            routing.validate()?;
            if !routing_ids.insert(routing.routing_id) {
                return Err(ContractError::invalid("manufacturing.routings", "two applied routings share one routing id"));
            }
            if !owners.insert(routing.owner) {
                return Err(ContractError::invalid("manufacturing.routings", "two applied routings own the same part version or product root"));
            }
        }
        let mut line_ids = BTreeSet::new();
        for line in &self.attached_costs {
            line.validate(&format!("manufacturing.attached_costs.{}", line.line_id))?;
            if !line_ids.insert(line.line_id) {
                return Err(ContractError::invalid("manufacturing.attached_costs", "two attached cost lines share one line id"));
            }
        }
        Ok(())
    }

    /// The sourcing record of one shared part version.
    pub fn part(&self, part_version_id: Uuid) -> Option<&SourcedPart> {
        self.parts.iter().find(|part| part.part_version_id == part_version_id)
    }

    /// The mutable sourcing record of one shared part version.
    pub fn part_mut(&mut self, part_version_id: Uuid) -> Option<&mut SourcedPart> {
        self.parts.iter_mut().find(|part| part.part_version_id == part_version_id)
    }

    /// The applied routing with one routing id.
    pub fn routing(&self, routing_id: Uuid) -> Option<&AppliedRouting> {
        self.routings.iter().find(|routing| routing.routing_id == routing_id)
    }

    /// The mutable applied routing with one routing id.
    pub fn routing_mut(&mut self, routing_id: Uuid) -> Option<&mut AppliedRouting> {
        self.routings.iter_mut().find(|routing| routing.routing_id == routing_id)
    }

    /// The applied routing that owns a part version or product root.
    pub fn routing_of(&self, owner: CostOwner) -> Option<&AppliedRouting> {
        self.routings.iter().find(|routing| routing.owner == owner)
    }

    /// The attached cost lines of one owner, in document order.
    pub fn attached_costs_of(&self, owner: CostOwner) -> impl Iterator<Item = &AttachedCostLine> {
        self.attached_costs.iter().filter(move |line| line.owner == owner)
    }

    /// The computed issues of this record: everything an engineer still has to complete. Issues
    /// are derived against the project record, so a project part version without an explicit
    /// sourcing choice is reported here instead of being assumed manufactured.
    pub fn issues(&self, project: &ProjectDocument) -> Vec<ManufacturingIssue> {
        let mut issues = Vec::new();
        for part_version in &project.part_versions {
            if self.part(part_version.part_version_id).is_none() {
                issues.push(
                    ManufacturingIssue::new(
                        "missing_sourcing",
                        format!("parts.{}.sourcing", part_version.part_version_id),
                        "the part version has no explicit purchased or manufactured choice",
                    )
                    .for_part(part_version.part_version_id),
                );
            }
        }
        for part in &self.parts {
            if part.sourcing == Sourcing::Manufactured {
                match &part.material {
                    None => issues.push(
                        ManufacturingIssue::new(
                            "missing_material",
                            format!("parts.{}.material", part.part_version_id),
                            "a manufactured part needs a selected material and its consumption",
                        )
                        .for_part(part.part_version_id),
                    ),
                    Some(material) => {
                        if material.material_version_id.is_none() {
                            issues.push(
                                ManufacturingIssue::new(
                                    "missing_material_version",
                                    format!("parts.{}.material.material_version_id", part.part_version_id),
                                    "no material version is selected",
                                )
                                .for_part(part.part_version_id),
                            );
                        }
                        if material.quantity.is_none() {
                            issues.push(
                                ManufacturingIssue::new(
                                    "missing_consumption",
                                    format!("parts.{}.material.quantity", part.part_version_id),
                                    "no net material consumption is recorded",
                                )
                                .for_part(part.part_version_id),
                            );
                        }
                        if material.review != ReviewState::Confirmed {
                            issues.push(
                                ManufacturingIssue::new(
                                    "pending_consumption_review",
                                    format!("parts.{}.material.review", part.part_version_id),
                                    "the recorded consumption is not confirmed by an engineer yet",
                                )
                                .for_part(part.part_version_id),
                            );
                        }
                        if material.allowance_rate.is_none() {
                            issues.push(
                                ManufacturingIssue::new(
                                    "missing_allowance",
                                    format!("parts.{}.material.allowance_rate", part.part_version_id),
                                    "the material allowance is not entered; zero must be explicit",
                                )
                                .for_part(part.part_version_id),
                            );
                        }
                    }
                }
                if self.routing_of(CostOwner::PartVersion { part_version_id: part.part_version_id }).is_none() {
                    issues.push(
                        ManufacturingIssue::new(
                            "missing_routing",
                            format!("parts.{}.routing", part.part_version_id),
                            "a manufactured part needs an applied routing",
                        )
                        .for_part(part.part_version_id),
                    );
                }
            }
        }
        for routing in &self.routings {
            if routing.operations.is_empty() {
                issues.push(
                    ManufacturingIssue::new(
                        "missing_routing_operations",
                        format!("routings.{}.operations", routing.routing_id),
                        "the applied routing has no operations yet",
                    )
                    .for_routing(routing.routing_id),
                );
            }
            for operation in &routing.operations {
                let field = format!("routings.{}.operations.{}", routing.routing_id, operation.operation_id);
                match &operation.cycle {
                    None => issues.push(
                        ManufacturingIssue::new("missing_cycle", format!("{field}.cycle"), "no cycle basis is recorded for the operation")
                            .for_operation(routing.routing_id, operation.operation_id),
                    ),
                    Some(cycle) => {
                        if cycle.seconds.is_none() {
                            issues.push(
                                ManufacturingIssue::new("missing_cycle_time", format!("{field}.cycle.seconds"), "no cycle time is recorded")
                                    .for_operation(routing.routing_id, operation.operation_id),
                            );
                        }
                        if cycle.basis.adjusts_for_cavities() && cycle.cavities.is_none() {
                            issues.push(
                                ManufacturingIssue::new(
                                    "missing_cycle_cavities",
                                    format!("{field}.cycle.cavities"),
                                    "a per-cycle time needs its cavity count before it can be converted per piece",
                                )
                                .for_operation(routing.routing_id, operation.operation_id),
                            );
                        }
                        if cycle.production_rate_version_id.is_none() {
                            issues.push(
                                ManufacturingIssue::new(
                                    "missing_production_rate",
                                    format!("{field}.cycle.production_rate_version_id"),
                                    "no production rate version is pinned",
                                )
                                .for_operation(routing.routing_id, operation.operation_id),
                            );
                        }
                    }
                }
                if let Some(setup) = &operation.setup {
                    if setup.hours.is_none() {
                        issues.push(
                            ManufacturingIssue::new(
                                "missing_setup_hours",
                                format!("{field}.setup.hours"),
                                "the operation has a setup record but no duration",
                            )
                            .for_operation(routing.routing_id, operation.operation_id),
                        );
                    }
                    if setup.setup_rate_version_id.is_none() {
                        issues.push(
                            ManufacturingIssue::new(
                                "missing_setup_rate",
                                format!("{field}.setup.setup_rate_version_id"),
                                "no dedicated setup rate version is pinned",
                            )
                            .for_operation(routing.routing_id, operation.operation_id),
                        );
                    }
                }
            }
        }
        for line in &self.attached_costs {
            let field = format!("attached_costs.{}", line.line_id);
            if line.quantity.is_none() {
                issues.push(
                    ManufacturingIssue::new("missing_attached_quantity", format!("{field}.quantity"), "the attached cost line has no quantity")
                        .for_line(line.line_id),
                );
            }
            if line.unit_cost.is_none() {
                issues.push(
                    ManufacturingIssue::new("missing_attached_unit_cost", format!("{field}.unit_cost"), "the attached cost line has no unit cost")
                        .for_line(line.line_id),
                );
            }
        }
        issues
    }

    /// The routing issues an engineer must resolve before the product can activate, in the stable
    /// order clients display them. A definition without any issue is complete for its part.
    pub fn is_complete(&self, project: &ProjectDocument) -> bool {
        self.issues(project).is_empty()
    }
}

/// A published, immutable version of a reusable company routing template.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoutingTemplateVersion {
    /// The schema version written into the stored document.
    pub schema_version: u32,
    /// The identity of this published version.
    pub template_version_id: Uuid,
    /// The logical template this version belongs to; it is stable across its versions.
    pub template_id: Uuid,
    pub version: u64,
    /// The version this one supersedes, when it is not the first.
    pub supersedes: Option<Uuid>,
    pub name: String,
    pub description: Option<String>,
    /// The manufacturing levels this template may be applied at; at least one.
    pub applies_to: Vec<RoutingLevel>,
    /// The template's ordered operations. Applying it copies them; editing the copy never edits
    /// the template.
    pub operations: Vec<RoutingOperation>,
    pub source_reference: Option<String>,
    /// RFC 3339 UTC timestamp the service recorded.
    pub created_at: Option<String>,
}

impl RoutingTemplateVersion {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != ROUTING_TEMPLATE_DOCUMENT_SCHEMA_VERSION {
            return Err(ContractError::SchemaVersion { found: self.schema_version, expected: ROUTING_TEMPLATE_DOCUMENT_SCHEMA_VERSION });
        }
        if self.template_version_id.is_nil() {
            return Err(ContractError::invalid("template.template_version_id", "the template version id is empty"));
        }
        if self.template_id.is_nil() {
            return Err(ContractError::invalid("template.template_id", "the template id is empty"));
        }
        if self.version == 0 {
            return Err(ContractError::invalid("template.version", "a template version starts at 1"));
        }
        if self.supersedes == Some(self.template_version_id) {
            return Err(ContractError::invalid("template.supersedes", "a version cannot supersede itself"));
        }
        validate_template_fields(&self.name, &self.description, &self.applies_to, &self.operations, &self.source_reference)?;
        check_optional_description(&self.created_at, "template.created_at")
    }

    /// Whether this template may be applied at a level.
    pub fn applies_at(&self, level: RoutingLevel) -> bool {
        self.applies_to.contains(&level)
    }
}

/// The current version of each published routing template, as an administration surface lists
/// them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RoutingTemplateList {
    pub templates: Vec<RoutingTemplateVersion>,
}

impl RoutingTemplateList {
    /// Validate the current template versions. A list holds each logical template once and each
    /// published version id once.
    pub fn validate(&self) -> Result<()> {
        if self.templates.len() > MAX_TEMPLATE_VERSIONS {
            return Err(ContractError::invalid("templates", format!("a list holds at most {MAX_TEMPLATE_VERSIONS} template versions")));
        }
        let mut version_ids = BTreeSet::new();
        let mut template_ids = BTreeSet::new();
        for template in &self.templates {
            template.validate()?;
            if !version_ids.insert(template.template_version_id) {
                return Err(ContractError::invalid("templates", "two template versions share one template version id"));
            }
            if !template_ids.insert(template.template_id) {
                return Err(ContractError::invalid("templates", "a logical template appears more than once in the current-version list"));
            }
        }
        Ok(())
    }
}

/// The compact receipt of one committed routing template version.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoutingTemplateReceipt {
    pub request_id: String,
    pub template_id: Uuid,
    pub version: u64,
    pub template_version_id: Uuid,
}

/// One immutable manufacturing record version as a client reads it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ManufacturingRecordView {
    pub project_id: Uuid,
    pub version: u64,
    pub document: ManufacturingRecordDocument,
    pub issues: Vec<ManufacturingIssue>,
}

/// The compact receipt of one committed manufacturing write.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManufacturingReceipt {
    pub request_id: String,
    pub project_id: Uuid,
    pub version: u64,
}

/// One committed manufacturing write: its receipt, the explicit affected references it wrote and
/// the dependency facts it emitted. The service echoes the references it validated, so a readiness
/// gate reads them from the write result instead of searching the project record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ManufacturingWriteOutcome {
    pub receipt: ManufacturingReceipt,
    pub affected_part_versions: Vec<Uuid>,
    pub affected_products: Vec<Uuid>,
    pub facts: Vec<ManufacturingFact>,
}

/// One dependency fact a manufacturing write emits. Facts name what changed; they never rewrite
/// accepted results, and later packages own any recalculation or activation they imply.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ManufacturingFact {
    /// The explicit sourcing choice (and material record) of one part version was set.
    SourcingRecorded { part_version_id: Uuid, sourcing: Sourcing },
    /// One applied routing was created, adopted or edited. `part_version_id` and `product_id`
    /// name the affected occurrence/product owners.
    RoutingWritten { routing_id: Uuid, part_version_id: Option<Uuid>, product_id: Option<Uuid>, template: Option<TemplateRef> },
    /// Attached non-CAD cost lines were added, replaced or removed from the draft.
    AttachedCostsWritten { written_line_ids: Vec<Uuid>, removed_line_ids: Vec<Uuid> },
}

/// Set the explicit sourcing record of one or more shared project part versions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SetSourcing {
    pub request_id: String,
    pub expected_version: u64,
    /// The complete records to write, one per part version. A purchased part carries no material
    /// block; a manufactured one keeps its material, consumption and allowance here.
    pub parts: Vec<SourcedPart>,
    /// The shared part versions this write affects; every written part version must be named once.
    pub affected_part_versions: Vec<Uuid>,
}

impl SetSourcing {
    pub fn validate(&self) -> Result<()> {
        check_request_id(&self.request_id, "set_sourcing.request_id")?;
        if self.parts.is_empty() {
            return Err(ContractError::invalid("set_sourcing.parts", "the write names no part"));
        }
        if self.parts.len() > MAX_SOURCED_PARTS {
            return Err(ContractError::invalid("set_sourcing.parts", format!("the write names at most {MAX_SOURCED_PARTS} parts")));
        }
        let mut part_version_ids = BTreeSet::new();
        for (index, part) in self.parts.iter().enumerate() {
            part.validate(&format!("set_sourcing.parts[{index}]"))?;
            if !part_version_ids.insert(part.part_version_id) {
                return Err(ContractError::invalid("set_sourcing.parts", "two written parts share one part version id"));
            }
        }
        check_affected_refs(&self.affected_part_versions, &[], "set_sourcing.affected_part_versions")?;
        let affected: BTreeSet<Uuid> = self.affected_part_versions.iter().copied().collect();
        if affected != part_version_ids {
            return Err(ContractError::invalid(
                "set_sourcing.affected_part_versions",
                "the affected part versions must name exactly the written part versions",
            ));
        }
        Ok(())
    }
}

/// Adopt one exact published template version as an applied routing. The service copies the
/// template's operations; the source link is retained on the copy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdoptRoutingTemplate {
    pub request_id: String,
    pub expected_version: u64,
    pub routing_id: Uuid,
    pub level: RoutingLevel,
    pub owner: CostOwner,
    pub template: TemplateRef,
    pub affected_part_versions: Vec<Uuid>,
    pub affected_products: Vec<Uuid>,
}

impl AdoptRoutingTemplate {
    pub fn validate(&self) -> Result<()> {
        check_request_id(&self.request_id, "adopt_template.request_id")?;
        check_routing_header(self.routing_id, self.level, self.owner, "adopt_template")?;
        self.template.validate("adopt_template.template")?;
        check_affected_owner(self.owner, &self.affected_part_versions, &self.affected_products, "adopt_template.affected_refs")
    }
}

/// Edit the operations of an existing applied routing. The routing's source/template link is kept;
/// editing the applied copy never changes the template or any other instance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EditRouting {
    pub request_id: String,
    pub expected_version: u64,
    pub routing_id: Uuid,
    pub operations: Vec<RoutingOperation>,
    pub affected_part_versions: Vec<Uuid>,
    pub affected_products: Vec<Uuid>,
}

impl EditRouting {
    pub fn validate(&self) -> Result<()> {
        check_request_id(&self.request_id, "edit_routing.request_id")?;
        if self.routing_id.is_nil() {
            return Err(ContractError::invalid("edit_routing.routing_id", "the routing id is empty"));
        }
        validate_operations(&self.operations, "edit_routing.operations", false)?;
        check_affected_refs(&self.affected_part_versions, &self.affected_products, "edit_routing.affected_refs")
    }
}

/// Enter an applied routing without a template. The operations are stored as given.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManualRouting {
    pub request_id: String,
    pub expected_version: u64,
    pub routing_id: Uuid,
    pub level: RoutingLevel,
    pub owner: CostOwner,
    pub operations: Vec<RoutingOperation>,
    pub affected_part_versions: Vec<Uuid>,
    pub affected_products: Vec<Uuid>,
}

impl ManualRouting {
    pub fn validate(&self) -> Result<()> {
        check_request_id(&self.request_id, "manual_routing.request_id")?;
        check_routing_header(self.routing_id, self.level, self.owner, "manual_routing")?;
        validate_operations(&self.operations, "manual_routing.operations", false)?;
        check_affected_owner(self.owner, &self.affected_part_versions, &self.affected_products, "manual_routing.affected_refs")
    }
}

/// One typed routing write. The action tag keeps adoption, edit and manual entry distinct, so an
/// edit can never silently adopt a newer template version.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum RoutingWrite {
    AdoptTemplate(AdoptRoutingTemplate),
    Edit(EditRouting),
    Manual(ManualRouting),
}

impl RoutingWrite {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::AdoptTemplate(write) => write.validate(),
            Self::Edit(write) => write.validate(),
            Self::Manual(write) => write.validate(),
        }
    }

    /// The routing id this write targets.
    pub fn routing_id(&self) -> Uuid {
        match self {
            Self::AdoptTemplate(write) => write.routing_id,
            Self::Edit(write) => write.routing_id,
            Self::Manual(write) => write.routing_id,
        }
    }

    pub fn request_id(&self) -> &str {
        match self {
            Self::AdoptTemplate(write) => &write.request_id,
            Self::Edit(write) => &write.request_id,
            Self::Manual(write) => &write.request_id,
        }
    }

    pub fn expected_version(&self) -> u64 {
        match self {
            Self::AdoptTemplate(write) => write.expected_version,
            Self::Edit(write) => write.expected_version,
            Self::Manual(write) => write.expected_version,
        }
    }
}

/// Add, replace or remove attached non-CAD cost lines of a project draft.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EditAttachedCosts {
    pub request_id: String,
    pub expected_version: u64,
    /// The lines to add or replace, keyed by their stable line id.
    pub upsert: Vec<AttachedCostLine>,
    /// The line ids to remove from the current draft. Earlier record versions keep them.
    pub remove: Vec<Uuid>,
    pub affected_part_versions: Vec<Uuid>,
    pub affected_products: Vec<Uuid>,
}

impl EditAttachedCosts {
    pub fn validate(&self) -> Result<()> {
        check_request_id(&self.request_id, "edit_attached_costs.request_id")?;
        if self.upsert.is_empty() && self.remove.is_empty() {
            return Err(ContractError::invalid("edit_attached_costs", "the write changes no attached cost line"));
        }
        if self.upsert.len() > MAX_ATTACHED_COST_LINES {
            return Err(ContractError::invalid("edit_attached_costs.upsert", format!("the write changes at most {MAX_ATTACHED_COST_LINES} lines")));
        }
        let mut line_ids = BTreeSet::new();
        for (index, line) in self.upsert.iter().enumerate() {
            line.validate(&format!("edit_attached_costs.upsert[{index}]"))?;
            if !line_ids.insert(line.line_id) {
                return Err(ContractError::invalid("edit_attached_costs.upsert", "two upserted lines share one line id"));
            }
        }
        let mut removed = BTreeSet::new();
        for line_id in &self.remove {
            if line_id.is_nil() {
                return Err(ContractError::invalid("edit_attached_costs.remove", "a removed line id is empty"));
            }
            if !removed.insert(*line_id) {
                return Err(ContractError::invalid("edit_attached_costs.remove", "a line id is removed twice"));
            }
            if line_ids.contains(line_id) {
                return Err(ContractError::invalid("edit_attached_costs", "one line id is both upserted and removed"));
            }
        }
        check_affected_refs(&self.affected_part_versions, &self.affected_products, "edit_attached_costs.affected_refs")?;
        // The owners of the upserted lines are explicit affected references; a removed line's
        // owner is validated by the service against the stored document.
        for line in &self.upsert {
            if let Some(part_version_id) = line.owner.part_version_id()
                && !self.affected_part_versions.contains(&part_version_id)
            {
                return Err(ContractError::invalid(
                    "edit_attached_costs.affected_part_versions",
                    "an upserted line's part version is not named as affected",
                ));
            }
            if let Some(product_id) = line.owner.product_id()
                && !self.affected_products.contains(&product_id)
            {
                return Err(ContractError::invalid("edit_attached_costs.affected_products", "an upserted line's product is not named as affected"));
            }
        }
        Ok(())
    }
}

/// Publish the next version of a reusable routing template. `expected_version` 0 creates version 1
/// of a new template. Only the managed write path may publish; applying and editing never do.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublishRoutingTemplate {
    pub request_id: String,
    pub expected_version: u64,
    pub template_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub applies_to: Vec<RoutingLevel>,
    pub operations: Vec<RoutingOperation>,
    pub source_reference: Option<String>,
}

impl PublishRoutingTemplate {
    pub fn validate(&self) -> Result<()> {
        check_request_id(&self.request_id, "publish_template.request_id")?;
        if self.template_id.is_nil() {
            return Err(ContractError::invalid("publish_template.template_id", "the template id is empty"));
        }
        validate_template_fields(&self.name, &self.description, &self.applies_to, &self.operations, &self.source_reference)
    }
}

/// The affected references a routing template may be applied to. At least one reference is named,
/// so a later readiness gate can list the affected occurrences/products without searching.
pub fn check_affected_refs(part_versions: &[Uuid], products: &[Uuid], field: &str) -> Result<()> {
    if part_versions.is_empty() && products.is_empty() {
        return Err(ContractError::invalid(field, "the write names no affected part version or product"));
    }
    if part_versions.len() > MAX_AFFECTED_REFS || products.len() > MAX_AFFECTED_REFS {
        return Err(ContractError::invalid(field, format!("the write names at most {MAX_AFFECTED_REFS} affected references")));
    }
    let mut seen = BTreeSet::new();
    for part_version_id in part_versions {
        if part_version_id.is_nil() {
            return Err(ContractError::invalid(field, "an affected part version id is empty"));
        }
        if !seen.insert(*part_version_id) {
            return Err(ContractError::invalid(field, "an affected part version is named twice"));
        }
    }
    for product_id in products {
        if product_id.is_nil() {
            return Err(ContractError::invalid(field, "an affected product id is empty"));
        }
        if !seen.insert(*product_id) {
            return Err(ContractError::invalid(field, "an affected reference is named twice"));
        }
    }
    Ok(())
}

/// Require that an applied routing or attached cost write names its owner as affected.
pub fn check_affected_owner(owner: CostOwner, part_versions: &[Uuid], products: &[Uuid], field: &str) -> Result<()> {
    check_affected_refs(part_versions, products, field)?;
    match owner {
        CostOwner::PartVersion { part_version_id } if !part_versions.contains(&part_version_id) => {
            Err(ContractError::invalid(format!("{field}.part_versions"), "the routing's part version is not named as affected"))
        }
        CostOwner::ProductRoot { product_id } if !products.contains(&product_id) => {
            Err(ContractError::invalid(format!("{field}.products"), "the routing's product is not named as affected"))
        }
        _ => Ok(()),
    }
}

/// One ordered operation list: unique ids, strictly increasing sequences, valid entries.
pub fn validate_operations(operations: &[RoutingOperation], field: &str, required: bool) -> Result<()> {
    if required && operations.is_empty() {
        return Err(ContractError::invalid(field, "the routing needs at least one operation"));
    }
    if operations.len() > MAX_OPERATIONS {
        return Err(ContractError::invalid(field, format!("a routing holds at most {MAX_OPERATIONS} operations")));
    }
    let mut ids = BTreeSet::new();
    let mut previous = 0;
    for (index, operation) in operations.iter().enumerate() {
        operation.validate(&format!("{field}[{index}]"))?;
        if !ids.insert(operation.operation_id) {
            return Err(ContractError::invalid(field, "two operations share one operation id"));
        }
        if operation.sequence <= previous {
            return Err(ContractError::invalid(field, "operations are stored in strictly increasing sequence order"));
        }
        if index == 0 && operation.sequence != 1 {
            return Err(ContractError::invalid(field, "the first operation sequence is 1"));
        }
        previous = operation.sequence;
    }
    Ok(())
}

fn validate_template_fields(
    name: &str,
    description: &Option<String>,
    applies_to: &[RoutingLevel],
    operations: &[RoutingOperation],
    source_reference: &Option<String>,
) -> Result<()> {
    check_name(name, "template.name")?;
    check_optional_description(description, "template.description")?;
    if applies_to.is_empty() {
        return Err(ContractError::invalid("template.applies_to", "a template names at least one manufacturing level"));
    }
    if applies_to.len() > MAX_ROUTING_LEVELS {
        return Err(ContractError::invalid("template.applies_to", "a template repeats a manufacturing level"));
    }
    let mut levels = BTreeSet::new();
    for level in applies_to {
        if !levels.insert(*level) {
            return Err(ContractError::invalid("template.applies_to", "a template repeats a manufacturing level"));
        }
    }
    if operations.is_empty() {
        return Err(ContractError::invalid("template.operations", "a published template has at least one operation"));
    }
    validate_operations(operations, "template.operations", true)?;
    check_optional_reference(source_reference, "template.source_reference")
}

fn check_routing_header(routing_id: Uuid, level: RoutingLevel, owner: CostOwner, prefix: &str) -> Result<()> {
    if routing_id.is_nil() {
        return Err(ContractError::invalid(format!("{prefix}.routing_id"), "the routing id is empty"));
    }
    owner.validate(&format!("{prefix}.owner"))?;
    if !owner.matches_level(level) {
        return Err(ContractError::invalid(
            format!("{prefix}.level"),
            "a product-root routing owns a product; a part or subassembly routing owns a part version",
        ));
    }
    Ok(())
}

fn check_request_id(text: &str, field: &str) -> Result<()> {
    check_text(text, field)?;
    if text.chars().count() > MAX_MANUFACTURING_REQUEST_ID_CHARS {
        return Err(ContractError::invalid(field, "the request id is too long"));
    }
    Ok(())
}

fn check_name(text: &str, field: &str) -> Result<()> {
    check_text(text, field)?;
    if text.chars().count() > MAX_MANUFACTURING_NAME_CHARS {
        return Err(ContractError::invalid(field, "the name is too long"));
    }
    Ok(())
}

fn check_optional_description(text: &Option<String>, field: &str) -> Result<()> {
    if let Some(text) = text {
        if text.chars().count() > MAX_DESCRIPTION_CHARS {
            return Err(ContractError::invalid(field, "the description is too long"));
        }
        check_text(text, field)?;
    }
    Ok(())
}

fn check_optional_reference(text: &Option<String>, field: &str) -> Result<()> {
    if let Some(text) = text {
        if text.chars().count() > MAX_REFERENCE_CHARS {
            return Err(ContractError::invalid(field, "the reference is too long"));
        }
        check_text(text, field)?;
    }
    Ok(())
}

fn check_optional_non_negative(value: Option<Decimal>, field: &str, reason: &str) -> Result<()> {
    if let Some(value) = value
        && value.is_sign_negative()
    {
        return Err(ContractError::invalid(field, reason));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::{DeliveryPackage, PartVersion, YearRange};
    use crate::types::AnalysisKey;

    fn part_version_id(index: u128) -> Uuid {
        Uuid::from_u128(index)
    }

    fn key() -> AnalysisKey {
        AnalysisKey { source_sha256: "a".repeat(64), engine_build: "0.40.0".into(), extractor_version: "2".into() }
    }

    fn consumed(quantity: Option<&str>, allowance: Option<&str>) -> MaterialConsumption {
        MaterialConsumption {
            material_version_id: Some(Uuid::from_u128(500)),
            quantity: quantity.map(|text| text.parse().unwrap_or_default()),
            unit: Unit::MillimetreCubed,
            source: MeasurementSource::Extracted,
            review: if quantity.is_some() { ReviewState::Confirmed } else { ReviewState::Pending },
            confirmed_by: quantity.map(|_| "engineer".to_string()),
            analysis_key: Some(key()),
            reason: None,
            allowance_rate: allowance.map(|text| text.parse().unwrap_or_default()),
            source_reference: None,
        }
    }

    fn cycle(seconds: Option<&str>, basis: CycleBasis, cavities: Option<u32>) -> OperationCycle {
        OperationCycle {
            seconds: seconds.map(|text| text.parse().unwrap_or_default()),
            basis,
            cavities,
            production_rate_version_id: Some(Uuid::from_u128(600)),
            origin: CycleOrigin::Manual,
            source_reference: None,
        }
    }

    fn operation(sequence: u32, cycle: Option<OperationCycle>, setup: Option<SetupTime>) -> RoutingOperation {
        RoutingOperation {
            operation_id: Uuid::from_u128(sequence as u128),
            sequence,
            name: format!("operation {sequence}"),
            description: None,
            cost_center_id: Some(Uuid::from_u128(700)),
            cycle,
            setup,
            tooling_reference: None,
        }
    }

    fn sourced(part_version_id: Uuid, sourcing: Sourcing) -> SourcedPart {
        SourcedPart {
            part_version_id,
            sourcing,
            material: match sourcing {
                Sourcing::Manufactured => Some(consumed(Some("2400"), Some("0.02"))),
                Sourcing::Purchased => None,
            },
            source_reference: None,
        }
    }

    fn routing(owner: CostOwner, level: RoutingLevel, operations: Vec<RoutingOperation>) -> AppliedRouting {
        AppliedRouting { routing_id: Uuid::from_u128(800), level, owner, source: RoutingSource::Manual, operations }
    }

    fn line(owner: CostOwner, quantity: Option<&str>, unit_cost: Option<&str>) -> AttachedCostLine {
        AttachedCostLine {
            line_id: Uuid::from_u128(900),
            owner,
            category: "packaging".into(),
            description: Some("returnable tray".into()),
            quantity: quantity.map(|text| text.parse().unwrap_or_default()),
            unit: "pcs".into(),
            unit_cost: unit_cost.map(|text| text.parse().unwrap_or_default()),
            source_reference: None,
        }
    }

    fn template_version() -> RoutingTemplateVersion {
        RoutingTemplateVersion {
            schema_version: ROUTING_TEMPLATE_DOCUMENT_SCHEMA_VERSION,
            template_version_id: Uuid::from_u128(1000),
            template_id: Uuid::from_u128(1001),
            version: 1,
            supersedes: None,
            name: "tube routing".into(),
            description: None,
            applies_to: vec![RoutingLevel::Part],
            operations: vec![operation(1, Some(cycle(Some("30"), CycleBasis::PerPiece, None)), None)],
            source_reference: None,
            created_at: None,
        }
    }

    /// A project with two shared part versions: two customer revisions of one part number.
    fn project() -> ProjectDocument {
        let package_id = Uuid::from_u128(11);
        ProjectDocument {
            schema_version: 1,
            project_id: Uuid::from_u128(10),
            name: "project".into(),
            customer: "customer".into(),
            internal_team: None,
            platform: None,
            application: None,
            currency: "CNY".into(),
            default_schedule: YearRange { sop_year: 2027, eop_year: 2029 },
            products: Vec::new(),
            packages: vec![DeliveryPackage {
                package_id,
                name: "delivery".into(),
                source: crate::types::SourceRef {
                    source_id: Uuid::from_u128(12),
                    sha256: "a".repeat(64),
                    byte_len: 10,
                    original_name: "three_boxes.step".into(),
                },
            }],
            part_versions: vec![
                PartVersion {
                    part_version_id: part_version_id(1),
                    customer_part_number: "P-1".into(),
                    customer_revision: "A".into(),
                    volume_mm3: Some("2400".parse().unwrap_or_default()),
                    created_in_package_id: package_id,
                },
                PartVersion {
                    part_version_id: part_version_id(2),
                    customer_part_number: "P-1".into(),
                    customer_revision: "B".into(),
                    volume_mm3: Some("2400".parse().unwrap_or_default()),
                    created_in_package_id: package_id,
                },
            ],
            quotation_events: Vec::new(),
        }
    }

    #[test]
    fn an_empty_record_validates_and_reports_every_part_version_unsourced() {
        let document = ManufacturingRecordDocument::empty(Uuid::from_u128(10));
        assert_eq!(document.schema_version, MANUFACTURING_DOCUMENT_SCHEMA_VERSION);
        assert!(document.validate().is_ok());
        let issues = document.issues(&project());
        assert_eq!(issues.iter().filter(|issue| issue.code == "missing_sourcing").count(), 2);
        assert!(!document.is_complete(&project()));
    }

    #[test]
    fn a_later_schema_version_is_refused() {
        let mut document = ManufacturingRecordDocument::empty(Uuid::from_u128(10));
        document.schema_version = MANUFACTURING_DOCUMENT_SCHEMA_VERSION + 1;
        assert!(matches!(document.validate(), Err(ContractError::SchemaVersion { .. })));
    }

    #[test]
    fn a_purchased_part_records_no_manufacturing_material() {
        let purchased = SourcedPart {
            part_version_id: part_version_id(1),
            sourcing: Sourcing::Purchased,
            material: Some(consumed(Some("2400"), Some("0"))),
            source_reference: None,
        };
        assert!(purchased.validate("part").is_err());
        let plain = Sourcing::Purchased;
        assert!(
            SourcedPart { part_version_id: part_version_id(1), sourcing: plain, material: None, source_reference: None }.validate("part").is_ok()
        );
    }

    #[test]
    fn a_made_part_keeps_missing_values_as_issues_not_zero() {
        let owner = CostOwner::PartVersion { part_version_id: part_version_id(1) };
        let mut document = ManufacturingRecordDocument::empty(Uuid::from_u128(10));
        document.parts.push(SourcedPart {
            part_version_id: part_version_id(1),
            sourcing: Sourcing::Manufactured,
            material: Some(consumed(None, None)),
            source_reference: None,
        });
        let mut missing_cycle = cycle(None, CycleBasis::PerPiece, None);
        missing_cycle.production_rate_version_id = None;
        document.routings.push(routing(
            owner,
            RoutingLevel::Part,
            vec![operation(1, Some(missing_cycle), Some(SetupTime { hours: None, setup_rate_version_id: None, source_reference: None }))],
        ));
        assert!(document.validate().is_ok());
        let reported = document.issues(&project());
        let codes: Vec<&str> = reported.iter().map(|issue| issue.code.as_str()).collect();
        for code in
            ["missing_consumption", "missing_allowance", "missing_cycle_time", "missing_production_rate", "missing_setup_hours", "missing_setup_rate"]
        {
            assert!(codes.contains(&code), "expected `{code}` in {codes:?}");
        }
        // The second customer revision stays separately unsourced and uncosted.
        assert!(
            document.issues(&project()).iter().any(|issue| issue.code == "missing_sourcing" && issue.part_version_id == Some(part_version_id(2)))
        );
    }

    #[test]
    fn an_explicit_zero_cycle_is_complete_and_stays_distinct_from_missing() {
        let owner = CostOwner::PartVersion { part_version_id: part_version_id(1) };
        let mut document = ManufacturingRecordDocument::empty(Uuid::from_u128(10));
        document.parts.push(sourced(part_version_id(1), Sourcing::Manufactured));
        document.parts.push(sourced(part_version_id(2), Sourcing::Manufactured));
        let zero = cycle(Some("0"), CycleBasis::PerPiece, None);
        assert_eq!(zero.per_piece_seconds(), Some(Decimal::ZERO));
        let missing = cycle(None, CycleBasis::PerPiece, None);
        assert_eq!(missing.per_piece_seconds(), None);
        document.routings.push(routing(owner, RoutingLevel::Part, vec![operation(1, Some(zero), None)]));
        document.routings.push(AppliedRouting {
            routing_id: Uuid::from_u128(801),
            level: RoutingLevel::Part,
            owner: CostOwner::PartVersion { part_version_id: part_version_id(2) },
            source: RoutingSource::Manual,
            operations: vec![operation(1, Some(missing), None)],
        });
        let reported = document.issues(&project());
        let missing_cycle: Vec<&ManufacturingIssue> = reported.iter().filter(|issue| issue.code == "missing_cycle_time").collect();
        assert_eq!(missing_cycle.len(), 1, "an explicit zero cycle is a value, not a missing input");
        assert_eq!(missing_cycle[0].routing_id, Some(Uuid::from_u128(801)), "only the other revision's omitted cycle stays missing");
    }

    #[test]
    fn cavities_adjust_the_cycle_only_for_the_declared_per_cycle_basis() {
        let per_cycle = cycle(Some("12"), CycleBasis::PerCycle, Some(2));
        assert_eq!(per_cycle.per_piece_seconds(), Some("6".parse().unwrap_or_default()));
        assert!(per_cycle.validate("cycle").is_ok(), "a per-cycle time may wait for its cavity count");
        let waiting = cycle(Some("12"), CycleBasis::PerCycle, None);
        assert_eq!(waiting.per_piece_seconds(), None);
        let per_piece = cycle(Some("12"), CycleBasis::PerPiece, None);
        assert_eq!(per_piece.per_piece_seconds(), Some("12".parse().unwrap_or_default()));
        let contradictory = cycle(Some("12"), CycleBasis::PerPiece, Some(2));
        assert!(contradictory.validate("cycle").is_err(), "a per-piece cycle is not divided by cavities");
        let zero_cavities = cycle(Some("12"), CycleBasis::PerCycle, Some(0));
        assert!(zero_cavities.validate("cycle").is_err());
        assert_eq!(zero_cavities.per_piece_seconds(), None);
    }

    #[test]
    fn a_calculated_cycle_names_its_formula_version() {
        let mut calculated = cycle(Some("30"), CycleBasis::PerPiece, None);
        calculated.origin = CycleOrigin::Calculated;
        assert!(calculated.validate("cycle").is_err());
        calculated.source_reference = Some("cycle-formula/1".into());
        assert!(calculated.validate("cycle").is_ok());
        let mut manual = cycle(Some("30"), CycleBasis::PerPiece, None);
        manual.origin = CycleOrigin::Manual;
        manual.source_reference = None;
        assert!(manual.validate("cycle").is_ok());
    }

    #[test]
    fn operations_are_ordered_by_unique_positive_sequences() {
        let owner = CostOwner::PartVersion { part_version_id: part_version_id(1) };
        assert!(routing(owner, RoutingLevel::Part, vec![operation(1, None, None), operation(2, None, None)]).validate().is_ok());
        let repeated = routing(owner, RoutingLevel::Part, vec![operation(1, None, None), operation(1, None, None)]);
        assert!(repeated.validate().is_err());
        let out_of_order = routing(owner, RoutingLevel::Part, vec![operation(2, None, None), operation(1, None, None)]);
        assert!(out_of_order.validate().is_err());
        let zero = routing(owner, RoutingLevel::Part, vec![operation(0, None, None)]);
        assert!(zero.validate().is_err());
    }

    #[test]
    fn a_routing_owner_matches_its_level_and_owns_one_routing() {
        let part = CostOwner::PartVersion { part_version_id: part_version_id(1) };
        let root = CostOwner::ProductRoot { product_id: Uuid::from_u128(3) };
        assert!(routing(part, RoutingLevel::ProductRoot, Vec::new()).validate().is_err());
        assert!(routing(root, RoutingLevel::Part, Vec::new()).validate().is_err());
        assert!(routing(part, RoutingLevel::Subassembly, Vec::new()).validate().is_ok());
        assert!(routing(root, RoutingLevel::ProductRoot, Vec::new()).validate().is_ok());

        let mut document = ManufacturingRecordDocument::empty(Uuid::from_u128(10));
        document.routings.push(routing(part, RoutingLevel::Part, Vec::new()));
        let mut second = routing(part, RoutingLevel::Subassembly, Vec::new());
        second.routing_id = Uuid::from_u128(802);
        document.routings.push(second);
        assert!(document.validate().is_err(), "one part version owns one applied routing");
    }

    #[test]
    fn an_applied_routing_keeps_its_template_source_link_and_leaves_the_template_alone() {
        let template = template_version();
        let template_before = template.clone();
        let routing_source = RoutingSource::Template(TemplateRef { template_id: template.template_id, version: template.version });
        let mut adopted = routing(CostOwner::PartVersion { part_version_id: part_version_id(1) }, RoutingLevel::Part, template.operations.clone());
        adopted.source = routing_source;
        assert_eq!(adopted.source.template().map(|template| template.version), Some(1));
        // Editing the applied copy changes only the copy.
        adopted.operations[0].name = "operation 1, edited".into();
        assert_ne!(adopted.operations[0].name, template_before.operations[0].name);
        assert_eq!(template, template_before, "the shared template is untouched");
        assert_eq!(template.operations[0].name, "operation 1");
        // A second instance of the same template keeps its own copy.
        let second = routing(CostOwner::PartVersion { part_version_id: part_version_id(2) }, RoutingLevel::Part, template_before.operations.clone());
        assert_ne!(second.operations[0].name, adopted.operations[0].name);
    }

    #[test]
    fn a_routing_template_is_published_with_levels_and_operations() {
        let template = PublishRoutingTemplate {
            request_id: "request-1".into(),
            expected_version: 0,
            template_id: Uuid::from_u128(1001),
            name: "tube routing".into(),
            description: None,
            applies_to: vec![RoutingLevel::Part, RoutingLevel::Subassembly],
            operations: vec![operation(1, Some(cycle(Some("30"), CycleBasis::PerPiece, None)), None)],
            source_reference: Some("routing-standard/1".into()),
        };
        assert!(template.validate().is_ok());
        assert!(PublishRoutingTemplate { applies_to: vec![], ..template.clone() }.validate().is_err());
        assert!(PublishRoutingTemplate { operations: vec![], ..template.clone() }.validate().is_err());
        assert!(PublishRoutingTemplate { applies_to: vec![RoutingLevel::Part, RoutingLevel::Part], ..template }.validate().is_err());
        assert!(template_version().validate().is_ok());
        assert!(template_version().applies_at(RoutingLevel::Part));
        assert!(!template_version().applies_at(RoutingLevel::ProductRoot));
    }

    #[test]
    fn attached_cost_lines_need_their_explicit_category_unit_and_basis() {
        let owner = CostOwner::PartVersion { part_version_id: part_version_id(1) };
        let valid = line(owner, Some("1"), Some("0.35"));
        assert!(valid.validate("line").is_ok());
        assert!(AttachedCostLine { line_id: Uuid::from_u128(901), owner, category: " ".into(), ..valid.clone() }.validate("line").is_err());
        assert!(AttachedCostLine { line_id: Uuid::from_u128(902), owner, unit: "".into(), ..valid.clone() }.validate("line").is_err());
        assert!(
            AttachedCostLine { line_id: Uuid::from_u128(903), owner, quantity: Some("-1".parse().unwrap_or_default()), ..valid.clone() }
                .validate("line")
                .is_err()
        );

        let mut document = ManufacturingRecordDocument::empty(Uuid::from_u128(10));
        document.parts.push(sourced(part_version_id(1), Sourcing::Purchased));
        document.parts.push(sourced(part_version_id(2), Sourcing::Purchased));
        document.attached_costs.push(line(owner, None, None));
        let reported = document.issues(&project());
        let codes: Vec<&str> = reported.iter().map(|issue| issue.code.as_str()).collect();
        assert!(codes.contains(&"missing_attached_quantity"));
        assert!(codes.contains(&"missing_attached_unit_cost"));
        // The lines carry no CAD identity: no node, occurrence or definition field exists.
        let json = serde_json::to_value(&document.attached_costs[0]).unwrap_or_default();
        assert!(json.get("definition_id").is_none());
        assert!(json.get("occurrence_id").is_none());
    }

    #[test]
    fn a_shared_part_routing_is_one_record_for_all_occurrences_and_another_revision_stays_separate() {
        let mut document = ManufacturingRecordDocument::empty(Uuid::from_u128(10));
        document.parts.push(sourced(part_version_id(1), Sourcing::Manufactured));
        document.parts.push(sourced(part_version_id(2), Sourcing::Manufactured));
        document.routings.push(routing(
            CostOwner::PartVersion { part_version_id: part_version_id(1) },
            RoutingLevel::Part,
            vec![operation(1, Some(cycle(Some("30"), CycleBasis::PerPiece, None)), None)],
        ));
        // A shared part record is read once for every occurrence that references the same version.
        assert_eq!(document.part(part_version_id(1)).map(|part| part.sourcing), Some(Sourcing::Manufactured));
        assert!(document.routing_of(CostOwner::PartVersion { part_version_id: part_version_id(1) }).is_some());
        // The other customer revision has no routing of its own and stays separate.
        assert!(document.routing_of(CostOwner::PartVersion { part_version_id: part_version_id(2) }).is_none());
        // An accepted document version is independent of a later edit: the document is a value.
        let accepted = document.clone();
        document.parts[0].sourcing = Sourcing::Purchased;
        document.parts[0].material = None;
        assert_eq!(accepted.parts[0].sourcing, Sourcing::Manufactured);
        assert_eq!(accepted.parts[0].material.as_ref().and_then(|material| material.material_version_id), Some(Uuid::from_u128(500)));
    }

    #[test]
    fn the_sourcing_write_names_exactly_the_changed_part_versions() {
        let write = SetSourcing {
            request_id: "request-1".into(),
            expected_version: 3,
            parts: vec![sourced(part_version_id(1), Sourcing::Manufactured)],
            affected_part_versions: vec![part_version_id(1)],
        };
        assert!(write.validate().is_ok());
        let mismatched = SetSourcing { affected_part_versions: vec![part_version_id(2)], ..write.clone() };
        assert!(mismatched.validate().is_err());
        let unlisted =
            SetSourcing { parts: vec![sourced(part_version_id(1), Sourcing::Purchased), sourced(part_version_id(2), Sourcing::Purchased)], ..write };
        assert!(unlisted.validate().is_err());
    }

    #[test]
    fn the_routing_writes_keep_adoption_edit_and_manual_entry_distinct() {
        let owner = CostOwner::PartVersion { part_version_id: part_version_id(1) };
        let adopted = RoutingWrite::AdoptTemplate(AdoptRoutingTemplate {
            request_id: "request-1".into(),
            expected_version: 0,
            routing_id: Uuid::from_u128(800),
            level: RoutingLevel::Part,
            owner,
            template: TemplateRef { template_id: Uuid::from_u128(1001), version: 1 },
            affected_part_versions: vec![part_version_id(1)],
            affected_products: Vec::new(),
        });
        assert!(adopted.validate().is_ok());
        let edited = RoutingWrite::Edit(EditRouting {
            request_id: "request-2".into(),
            expected_version: 1,
            routing_id: Uuid::from_u128(800),
            operations: vec![operation(1, None, None)],
            affected_part_versions: vec![part_version_id(1)],
            affected_products: Vec::new(),
        });
        assert!(edited.validate().is_ok());
        let manual = RoutingWrite::Manual(ManualRouting {
            request_id: "request-3".into(),
            expected_version: 2,
            routing_id: Uuid::from_u128(801),
            level: RoutingLevel::Part,
            owner,
            operations: vec![operation(1, None, None)],
            affected_part_versions: vec![part_version_id(1)],
            affected_products: Vec::new(),
        });
        assert!(manual.validate().is_ok());
        // Adoption names a template version; an edit cannot name one.
        let wrong_level = RoutingWrite::AdoptTemplate(AdoptRoutingTemplate {
            request_id: "request-4".into(),
            expected_version: 0,
            routing_id: Uuid::from_u128(802),
            level: RoutingLevel::ProductRoot,
            owner,
            template: TemplateRef { template_id: Uuid::from_u128(1001), version: 1 },
            affected_part_versions: vec![part_version_id(1)],
            affected_products: Vec::new(),
        });
        assert!(wrong_level.validate().is_err());
        let missing_owner = RoutingWrite::Edit(EditRouting {
            request_id: "request-5".into(),
            expected_version: 1,
            routing_id: Uuid::from_u128(800),
            operations: vec![operation(1, None, None)],
            affected_part_versions: Vec::new(),
            affected_products: Vec::new(),
        });
        assert!(missing_owner.validate().is_err());
        // The response carries the request facts an activation gate needs.
        assert_eq!(adopted.routing_id(), Uuid::from_u128(800));
        assert_eq!(edited.expected_version(), 1);
    }

    #[test]
    fn attached_cost_writes_are_upserts_and_removals_with_disjoint_ids() {
        let owner = CostOwner::PartVersion { part_version_id: part_version_id(1) };
        let write = EditAttachedCosts {
            request_id: "request-1".into(),
            expected_version: 0,
            upsert: vec![line(owner, Some("1"), Some("0.35"))],
            remove: Vec::new(),
            affected_part_versions: vec![part_version_id(1)],
            affected_products: Vec::new(),
        };
        assert!(write.validate().is_ok());
        let mut remove = line(owner, Some("1"), Some("0.35"));
        remove.line_id = Uuid::from_u128(901);
        let remove_id = remove.line_id;
        let removed = EditAttachedCosts {
            request_id: "request-2".into(),
            expected_version: 1,
            upsert: Vec::new(),
            remove: vec![remove_id],
            affected_part_versions: vec![part_version_id(1)],
            affected_products: Vec::new(),
        };
        assert!(removed.validate().is_ok());
        let overlap = EditAttachedCosts {
            request_id: "request-3".into(),
            expected_version: 2,
            upsert: vec![line(owner, Some("1"), Some("0.35"))],
            remove: vec![Uuid::from_u128(900)],
            affected_part_versions: vec![part_version_id(1)],
            affected_products: Vec::new(),
        };
        assert!(overlap.validate().is_err());
        let empty = EditAttachedCosts {
            request_id: "request-4".into(),
            expected_version: 2,
            upsert: Vec::new(),
            remove: Vec::new(),
            affected_part_versions: vec![part_version_id(1)],
            affected_products: Vec::new(),
        };
        assert!(empty.validate().is_err());
        let unnamed = EditAttachedCosts {
            request_id: "request-5".into(),
            expected_version: 2,
            upsert: vec![line(owner, Some("1"), Some("0.35"))],
            remove: Vec::new(),
            affected_part_versions: Vec::new(),
            affected_products: Vec::new(),
        };
        assert!(unnamed.validate().is_err());
    }

    #[test]
    fn a_manual_consumption_needs_its_reason() {
        let mut material = consumed(Some("2400"), Some("0"));
        material.source = MeasurementSource::Manual;
        material.reason = None;
        assert!(material.validate("material").is_err());
        material.reason = Some("overridden from the workshop sheet".into());
        assert!(material.validate("material").is_ok());
        material.unit = Unit::Currency;
        assert!(material.validate("material").is_err());
    }

    #[test]
    fn a_policy_and_a_product_with_assembly_operations() {
        // Assembly installation/testing operations belong to the assembly's own routing; they are
        // stored once at the parent and never copied into the child parts.
        let mut document = ManufacturingRecordDocument::empty(Uuid::from_u128(10));
        document.parts.push(sourced(part_version_id(1), Sourcing::Manufactured));
        document.parts.push(sourced(part_version_id(2), Sourcing::Manufactured));
        document.routings.push(routing(
            CostOwner::PartVersion { part_version_id: part_version_id(1) },
            RoutingLevel::Part,
            vec![operation(1, Some(cycle(Some("30"), CycleBasis::PerPiece, None)), None)],
        ));
        let assembly = AppliedRouting {
            routing_id: Uuid::from_u128(803),
            level: RoutingLevel::Subassembly,
            owner: CostOwner::PartVersion { part_version_id: part_version_id(2) },
            source: RoutingSource::Manual,
            operations: vec![RoutingOperation {
                operation_id: Uuid::from_u128(31),
                sequence: 1,
                name: "install and test".into(),
                description: Some("assembly context only".into()),
                cost_center_id: Some(Uuid::from_u128(700)),
                cycle: Some(cycle(Some("300"), CycleBasis::PerPiece, None)),
                setup: None,
                tooling_reference: None,
            }],
        };
        document.routings.push(assembly.clone());
        assert!(document.validate().is_ok());
        let child_ops: Vec<&RoutingOperation> = document
            .routings
            .iter()
            .filter(|routing| routing.owner == CostOwner::PartVersion { part_version_id: part_version_id(1) })
            .flat_map(|routing| &routing.operations)
            .collect();
        assert_eq!(child_ops.len(), 1);
        assert!(!child_ops.iter().any(|operation| operation.name == "install and test"));
        assert_eq!(document.routing_of(assembly.owner).map(|routing| routing.level), Some(RoutingLevel::Subassembly));
    }

    #[test]
    fn decimals_serialize_as_strings_and_missing_values_as_null() {
        let present = cycle(Some("12.5"), CycleBasis::PerCycle, Some(2));
        let json = serde_json::to_value(&present).unwrap_or_default();
        assert_eq!(json["seconds"], serde_json::json!("12.5"));
        let absent = cycle(None, CycleBasis::PerPiece, None);
        let json = serde_json::to_value(&absent).unwrap_or_default();
        assert_eq!(json["seconds"], serde_json::Value::Null);
        let zero = cycle(Some("0"), CycleBasis::PerPiece, None);
        let json = serde_json::to_value(&zero).unwrap_or_default();
        assert_eq!(json["seconds"], serde_json::json!("0"));
    }

    #[test]
    fn unknown_fields_are_tolerated_and_the_document_round_trips() {
        let mut document = ManufacturingRecordDocument::empty(Uuid::from_u128(10));
        document.parts.push(sourced(part_version_id(1), Sourcing::Manufactured));
        document.routings.push(routing(
            CostOwner::PartVersion { part_version_id: part_version_id(1) },
            RoutingLevel::Part,
            vec![operation(1, Some(cycle(Some("30"), CycleBasis::PerPiece, None)), None)],
        ));
        let mut json = serde_json::to_value(&document).unwrap_or_default();
        json["written_by"] = serde_json::json!("a later build");
        let parsed: ManufacturingRecordDocument = serde_json::from_value(json).unwrap_or_else(|_| ManufacturingRecordDocument::empty(Uuid::nil()));
        assert!(parsed.validate().is_ok(), "unknown fields are tolerated");
        assert_eq!(parsed, document, "the document round trips");
    }

    #[test]
    fn the_manufacuring_view_reports_version_and_issues() {
        let document = ManufacturingRecordDocument::empty(Uuid::from_u128(10));
        let view = ManufacturingRecordView { project_id: Uuid::from_u128(10), version: 4, issues: document.issues(&project()), document };
        assert_eq!(view.version, 4);
        assert!(!view.issues.is_empty());
        assert!(view.issues.iter().all(|issue| issue.validate().is_ok()));
    }

    #[test]
    fn a_template_version_cannot_supersede_itself() {
        let mut template = template_version();
        template.supersedes = Some(template.template_version_id);
        assert!(template.validate().is_err());
        template.supersedes = Some(Uuid::from_u128(999));
        assert!(template.validate().is_ok());
    }

    #[test]
    fn the_project_document_in_the_fixture_is_valid() {
        assert!(project().validate().is_ok());
        let _ = key();
    }

    #[test]
    fn a_routing_level_knows_whether_it_is_the_product_root() {
        assert!(!RoutingLevel::Part.is_product_root());
        assert!(!RoutingLevel::Subassembly.is_product_root());
        assert!(RoutingLevel::ProductRoot.is_product_root());
    }

    #[test]
    fn the_owner_helpers_name_one_identity_only() {
        let part = CostOwner::PartVersion { part_version_id: part_version_id(1) };
        let root = CostOwner::ProductRoot { product_id: Uuid::from_u128(3) };
        assert_eq!(part.part_version_id(), Some(part_version_id(1)));
        assert_eq!(part.product_id(), None);
        assert_eq!(root.part_version_id(), None);
        assert_eq!(root.product_id(), Some(Uuid::from_u128(3)));
    }

    #[test]
    fn the_fact_kinds_name_the_affected_refs() {
        let fact = ManufacturingFact::RoutingWritten {
            routing_id: Uuid::from_u128(800),
            part_version_id: Some(part_version_id(1)),
            product_id: None,
            template: Some(TemplateRef { template_id: Uuid::from_u128(1001), version: 2 }),
        };
        let json = serde_json::to_value(&fact).unwrap_or_default();
        assert_eq!(json["kind"], serde_json::json!("routing_written"));
        assert_eq!(json["template"]["version"], serde_json::json!(2));
    }
    #[test]
    fn routing_writes_require_their_own_owner_in_the_affected_refs() {
        let owner = CostOwner::PartVersion { part_version_id: part_version_id(1) };
        let template = TemplateRef { template_id: Uuid::from_u128(1001), version: 1 };
        let unrelated_product = vec![Uuid::from_u128(3)];
        let misdirected = AdoptRoutingTemplate {
            request_id: "request-1".into(),
            expected_version: 0,
            routing_id: Uuid::from_u128(800),
            level: RoutingLevel::Part,
            owner,
            template,
            affected_part_versions: Vec::new(),
            affected_products: unrelated_product.clone(),
        };
        assert!(misdirected.validate().is_err(), "a part routing must name its part version as affected");
        let named = AdoptRoutingTemplate { affected_part_versions: vec![part_version_id(1)], ..misdirected.clone() };
        assert!(named.validate().is_ok());
        let manual = ManualRouting {
            request_id: "request-2".into(),
            expected_version: 0,
            routing_id: Uuid::from_u128(801),
            level: RoutingLevel::Part,
            owner,
            operations: Vec::new(),
            affected_part_versions: Vec::new(),
            affected_products: unrelated_product,
        };
        assert!(manual.validate().is_err());
    }

    #[test]
    fn a_confirmed_consumption_needs_a_value_the_reviewer_and_its_analysis() {
        let mut material = consumed(Some("2400"), Some("0"));
        material.quantity = None;
        assert!(material.validate("material").is_err(), "a confirmed consumption cannot have no value");
        let mut missing_analysis = consumed(Some("2400"), Some("0"));
        missing_analysis.analysis_key = None;
        assert!(missing_analysis.validate("material").is_err(), "an extracted consumption names its analysis");
        let mut manual = consumed(Some("2400"), Some("0"));
        manual.source = MeasurementSource::Manual;
        manual.analysis_key = None;
        manual.reason = Some("measured on the bench".into());
        assert!(manual.validate("material").is_ok(), "a manual override needs its reason, not an analysis");
        let mut blank_reviewer = consumed(Some("2400"), Some("0"));
        blank_reviewer.confirmed_by = Some("  ".into());
        assert!(blank_reviewer.validate("material").is_err());
    }

    #[test]
    fn a_complete_manufactured_part_has_no_issues() {
        let mut project = project();
        project.part_versions.truncate(1);
        let mut document = ManufacturingRecordDocument::empty(Uuid::from_u128(10));
        document.parts.push(sourced(part_version_id(1), Sourcing::Manufactured));
        let mut complete_cycle = cycle(Some("12"), CycleBasis::PerCycle, Some(2));
        complete_cycle.production_rate_version_id = Some(Uuid::from_u128(600));
        document.routings.push(routing(
            CostOwner::PartVersion { part_version_id: part_version_id(1) },
            RoutingLevel::Part,
            vec![operation(
                1,
                Some(complete_cycle),
                Some(SetupTime {
                    hours: Some("0.5".parse().unwrap_or_default()),
                    setup_rate_version_id: Some(Uuid::from_u128(601)),
                    source_reference: None,
                }),
            )],
        ));
        assert!(document.validate().is_ok());
        assert_eq!(document.issues(&project), Vec::new(), "a complete made part needs no follow-up");
        assert!(document.is_complete(&project));
    }

    #[test]
    fn the_unreached_issue_codes_are_reported_when_the_data_is_missing() {
        let mut project = project();
        project.part_versions.truncate(1);
        let mut document = ManufacturingRecordDocument::empty(Uuid::from_u128(10));
        // A made part with no material record and no routing.
        document.parts.push(SourcedPart {
            part_version_id: part_version_id(1),
            sourcing: Sourcing::Manufactured,
            material: None,
            source_reference: None,
        });
        let codes =
            |document: &ManufacturingRecordDocument| -> Vec<String> { document.issues(&project).iter().map(|issue| issue.code.clone()).collect() };
        let reported = codes(&document);
        assert!(reported.contains(&"missing_material".to_string()));
        assert!(reported.contains(&"missing_routing".to_string()));
        // A consumption record without a material version and still pending review.
        document.parts[0].material = Some(MaterialConsumption {
            material_version_id: None,
            quantity: Some("2400".parse().unwrap_or_default()),
            unit: Unit::MillimetreCubed,
            source: MeasurementSource::Extracted,
            review: ReviewState::Pending,
            confirmed_by: None,
            analysis_key: Some(key()),
            reason: None,
            allowance_rate: Some(Decimal::ZERO),
            source_reference: None,
        });
        document.routings.push(AppliedRouting {
            routing_id: Uuid::from_u128(800),
            level: RoutingLevel::Part,
            owner: CostOwner::PartVersion { part_version_id: part_version_id(1) },
            source: RoutingSource::Manual,
            operations: Vec::new(),
        });
        document.routings[0].operations.push(operation(1, Some(cycle(None, CycleBasis::PerCycle, None)), None));
        let reported = codes(&document);
        assert!(document.validate().is_ok(), "a draft routing waits for its operations");
        for code in ["missing_material_version", "pending_consumption_review", "missing_cycle_time", "missing_cycle_cavities"] {
            assert!(reported.contains(&code.to_string()), "expected `{code}` in {reported:?}");
        }
        // An applied routing with no operations yet is an explicit draft state.
        document.routings[0].operations.clear();
        assert!(document.validate().is_ok());
        assert!(codes(&document).contains(&"missing_routing_operations".to_string()));
        assert!(
            document.issues(&project).iter().find(|issue| issue.code == "missing_routing_operations").and_then(|issue| issue.operation_id).is_none()
        );
    }

    #[test]
    fn a_template_sourced_routing_and_its_write_round_trip_and_the_template_names_its_schema() {
        let template = template_version();
        let json = serde_json::to_value(&template).unwrap_or_default();
        assert_eq!(json["schema_version"], serde_json::json!(1));
        let parsed: RoutingTemplateVersion = serde_json::from_value(json.clone()).unwrap_or_else(|_| template.clone());
        assert_eq!(parsed, template);
        let mut later = template.clone();
        later.schema_version = ROUTING_TEMPLATE_DOCUMENT_SCHEMA_VERSION + 1;
        assert!(matches!(later.validate(), Err(ContractError::SchemaVersion { .. })));

        let adopted = RoutingWrite::AdoptTemplate(AdoptRoutingTemplate {
            request_id: "request-1".into(),
            expected_version: 0,
            routing_id: Uuid::from_u128(800),
            level: RoutingLevel::Part,
            owner: CostOwner::PartVersion { part_version_id: part_version_id(1) },
            template: TemplateRef { template_id: template.template_id, version: template.version },
            affected_part_versions: vec![part_version_id(1)],
            affected_products: Vec::new(),
        });
        let json = serde_json::to_value(&adopted).unwrap_or_default();
        assert_eq!(json["action"], serde_json::json!("adopt_template"));
        let parsed: RoutingWrite = serde_json::from_value(json).unwrap_or_else(|_| adopted.clone());
        assert_eq!(parsed, adopted);

        let mut applied = routing(CostOwner::PartVersion { part_version_id: part_version_id(1) }, RoutingLevel::Part, template.operations.clone());
        applied.source = RoutingSource::Template(TemplateRef { template_id: template.template_id, version: template.version });
        let json = serde_json::to_value(&applied).unwrap_or_default();
        let parsed: AppliedRouting = serde_json::from_value(json).unwrap_or_else(|_| applied.clone());
        assert_eq!(parsed.source, applied.source, "the retained template version survives the wire");
    }

    #[test]
    fn the_routing_template_list_validates_each_current_version_once() {
        let template = template_version();
        let list = RoutingTemplateList { templates: vec![template.clone()] };
        assert!(list.validate().is_ok());
        let repeated = RoutingTemplateList { templates: vec![template.clone(), template.clone()] };
        assert!(repeated.validate().is_err(), "a current-version list holds each logical template once");
        let mut second = template.clone();
        second.template_id = Uuid::from_u128(1002);
        second.template_version_id = Uuid::from_u128(1003);
        second.version = 2;
        let two = RoutingTemplateList { templates: vec![template, second] };
        assert!(two.validate().is_ok());
    }

    #[test]
    fn the_cycle_basis_round_trips_its_wire_name() {
        let per_piece = cycle(Some("30"), CycleBasis::PerPiece, None);
        assert_eq!(serde_json::to_value(&per_piece).unwrap_or_default()["basis"], serde_json::json!("per_piece"));
        let per_cycle = cycle(Some("12"), CycleBasis::PerCycle, Some(2));
        assert_eq!(serde_json::to_value(&per_cycle).unwrap_or_default()["basis"], serde_json::json!("per_cycle"));
        assert_eq!(per_cycle.per_piece_seconds(), Some("6".parse().unwrap_or_default()));
    }

    #[test]
    fn the_first_operation_sequence_is_one() {
        let owner = CostOwner::PartVersion { part_version_id: part_version_id(1) };
        assert!(routing(owner, RoutingLevel::Part, vec![operation(1, None, None), operation(2, None, None)]).validate().is_ok());
        assert!(routing(owner, RoutingLevel::Part, vec![operation(5, None, None), operation(6, None, None)]).validate().is_err());
    }

    #[test]
    fn the_affected_refs_survive_in_the_write_outcome() {
        let outcome = ManufacturingWriteOutcome {
            receipt: ManufacturingReceipt { request_id: "request-1".into(), project_id: Uuid::from_u128(10), version: 3 },
            affected_part_versions: vec![part_version_id(1)],
            affected_products: Vec::new(),
            facts: vec![ManufacturingFact::SourcingRecorded { part_version_id: part_version_id(1), sourcing: Sourcing::Manufactured }],
        };
        let json = serde_json::to_value(&outcome).unwrap_or_default();
        assert_eq!(json["affected_part_versions"][0], serde_json::json!(part_version_id(1).to_string()));
        let parsed: ManufacturingWriteOutcome = serde_json::from_value(json).unwrap_or_else(|_| outcome.clone());
        assert_eq!(parsed, outcome);
    }
}
