//! Activation readiness, coordinated shared-definition activation and accepted baselines: the
//! version 1 activation contract of package SB-19, additive to API version 2.
//!
//! The stored project record already carries `draft`/`active`/`historical` product revisions, but it
//! publishes no activation decision. This module is the portable vocabulary that decision uses:
//!
//! - [`ActivationReadinessRequest`] and [`ActivationReadinessReport`] name the exact project record
//!   version and the selected member revisions, and report one ready/blocked state per selected
//!   member with typed [`ReadinessBlocker`]s grouped by their owning package. Readiness is a pure
//!   read: it never appends a project version, snapshot or accepted baseline.
//! - A member is one product plus one revision. The revision is either the product's current
//!   `draft` (the activation turns it `active`) or its current `active` revision (the activation
//!   re-pins the accepted shared definitions without a revision state transition). A `historical`
//!   or otherwise inactivatable revision is refused; it never silently joins a group.
//! - [`ActivateRevisions`] is the expected-version activation request; [`ActivationReceipt`] is its
//!   committed answer, including the new project version, the accepted baseline id, the members,
//!   the pinned record versions and the dependency facts for SB-20.
//! - [`AcceptedBaseline`] is an append-only record of one activation: the exact project,
//!   engineering, manufacturing and feasibility record versions read, the immutable SB-16 costing
//!   snapshot that resolved every reference version, and the project's full accepted shared-part
//!   definition set after the activation. [`AcceptedBaselineList`] and a historical read by id stay
//!   readable after later edits.
//! - [`ActivationRefusal`] names the stable refusal reasons. The HTTP error body keeps the version
//!   1 `ApiErrorBody` shape; the refusal code is the stable prefix of its message, because the
//!   version 2 error code set is closed and this contract does not extend it.
//!
//! # Readiness scoping
//!
//! A blocker blocks the member it is attributed to: a product-scoped issue blocks that product; a
//! part-scoped issue blocks every member whose resulting structure uses that part version; a
//! component- or revision-scoped issue blocks the member whose resulting revision holds it. An
//! issue scoped to no product or part is a project setting and blocks every member. The SB-16
//! costing snapshot is project-wide, so its product-scoped issues for another product do not block a
//! member, `no_quoted_products` does not block a quotation-excluded member, and the receipt still
//! reports the evaluation's project-wide `costing_state`. Negotiated selling prices and commercial
//! scenarios are never readiness inputs.
//!
//! # Coordination
//!
//! A coordinated activation is required when a shared part version's current sourcing, material,
//! dimensions or manufacturing routing definition differs from the definition the newest accepted
//! baseline recorded for that part version (the "accepted definition"). The comparison is
//! semantic, not a record-version comparison: re-saving an identical definition does not force
//! coordination, and changing only a comment, a provenance field or a rate version does not either.
//! [`accepted_shared_definition`] builds the pin from the stored records and
//! [`shared_definition_changes`] reports the differing aspects, so the service and this contract
//! cannot drift on what "differs" means.
//!
//! Everything is English, validated on the way in, and a missing value stays `None`; this module
//! never turns a missing input into a zero and never panics.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::costing::ProductRevisionSelection;
use crate::engineering::ReviewedPart;
use crate::error::{ContractError, Result, check_sha256};
use crate::manufacturing::{AppliedRouting, SourcedPart};
use crate::projects::PartVersion;
use crate::types::{CycleBasis, ResultState, Sourcing, Unit};

/// The activation contract version this build reads and writes.
pub const ACTIVATION_CONTRACT_VERSION: u32 = 1;

/// The most product revisions one readiness report or activation request may name. It matches the
/// SB-16 baseline product bound so an accepted state never exceeds one calculation input.
pub const MAX_ACTIVATION_MEMBERS: usize = 1_000;

/// The most shared-definition pins one accepted baseline may hold. It matches the SB-16 baseline
/// part bound: every activated revision's costed structure is resolved into the same snapshot.
pub const MAX_SHARED_DEFINITIONS: usize = 10_000;

/// The most typed blockers one selected member may carry.
pub const MAX_READINESS_BLOCKERS: usize = 100_000;

/// The most accepted baselines one history read returns; the service also stores them all.
pub const MAX_ACCEPTED_BASELINE_HISTORY: usize = 100;

/// The longest identity text (a customer part number or revision) a pin carries.
const MAX_IDENTITY_CHARS: usize = 255;

/// The longest stable code, field path and message a blocker or a refusal carries.
const MAX_CODE_CHARS: usize = 64;
const MAX_FIELD_PATH_CHARS: usize = 1_024;
const MAX_MESSAGE_CHARS: usize = 4_000;

/// The longest calculator version and tooling reference text this contract carries.
const MAX_CALCULATOR_VERSION_CHARS: usize = 64;
const MAX_TOOLING_REFERENCE_CHARS: usize = 1_024;

/// How many blockers a [`ActivationRefusal::MemberNotReady`] message names before summarising.
const REFUSAL_BLOCKER_NAMES: usize = 8;

// ---------------------------------------------------------------------------
// Readiness
// ---------------------------------------------------------------------------

/// Whether one selected member revision may be accepted now.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessState {
    /// Every readiness input is present and no finding is open.
    Ready,
    /// At least one typed blocker prevents activation.
    Blocked,
}

/// The package that owns resolving one readiness blocker. These names are contract vocabulary, not
/// display text; the contract document maps them to the owning packages (project SB-10, engineering
/// review SB-11, feasibility SB-14, manufacturing SB-15, costing SB-16, tooling SB-17).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessOwner {
    /// Project record runs and identities: forecasts and the revision state itself.
    Project,
    /// Reviewed classification, reviewed dimensions, component associations and part matches.
    Engineering,
    /// Feasibility findings, assessment staleness and exception dispositions.
    Feasibility,
    /// Sourcing and manufacturing definition issues.
    Manufacturing,
    /// Cost-baseline issues and required cost inputs.
    Costing,
    /// Tooling input completeness.
    Tooling,
}

impl ReadinessOwner {
    /// The canonical order of owner groups in a report.
    pub fn rank(self) -> u8 {
        match self {
            Self::Project => 0,
            Self::Engineering => 1,
            Self::Feasibility => 2,
            Self::Manufacturing => 3,
            Self::Costing => 4,
            Self::Tooling => 5,
        }
    }
}

/// One typed reason a selected revision is not ready. The code is a stable contract name copied from
/// the owning package's computed issue; the message is client-safe text.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReadinessBlocker {
    pub owner: ReadinessOwner,
    pub code: String,
    pub field_path: String,
    pub message: String,
    /// The product the blocker is about, when it is product-scoped.
    #[serde(default)]
    pub product_id: Option<Uuid>,
    /// The product revision the blocker is about, when it is revision-scoped.
    #[serde(default)]
    pub product_revision_id: Option<Uuid>,
    /// The shared part version the blocker is about, when it is part-scoped.
    #[serde(default)]
    pub part_version_id: Option<Uuid>,
    /// The reviewed component the blocker is about, when it is component-scoped.
    #[serde(default)]
    pub component_id: Option<Uuid>,
}

impl ReadinessBlocker {
    /// A blocker with no identity yet.
    pub fn new(owner: ReadinessOwner, code: impl Into<String>, field_path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            owner,
            code: code.into(),
            field_path: field_path.into(),
            message: message.into(),
            product_id: None,
            product_revision_id: None,
            part_version_id: None,
            component_id: None,
        }
    }

    /// Attach the affected product.
    pub fn for_product(mut self, product_id: Uuid) -> Self {
        self.product_id = Some(product_id);
        self
    }

    /// Attach the affected product revision (and its product).
    pub fn for_revision(mut self, product_id: Uuid, product_revision_id: Uuid) -> Self {
        self.product_id = Some(product_id);
        self.product_revision_id = Some(product_revision_id);
        self
    }

    /// Attach the affected shared part version.
    pub fn for_part(mut self, part_version_id: Uuid) -> Self {
        self.part_version_id = Some(part_version_id);
        self
    }

    /// The canonical ordering key: owner group, then stable code, then field path.
    fn sort_key(&self) -> (u8, &str, &str) {
        (self.owner.rank(), self.code.as_str(), self.field_path.as_str())
    }

    /// Validate the blocker's own fields.
    pub fn validate(&self) -> Result<()> {
        if !crate::types::is_stable_code(&self.code, MAX_CODE_CHARS) {
            return Err(ContractError::invalid(
                "blocker.code",
                "a blocker code is a stable lower-case code: a letter followed by lower-case letters, digits or underscores",
            ));
        }
        check_bounded_text(&self.field_path, "blocker.field_path", MAX_FIELD_PATH_CHARS)?;
        check_bounded_text(&self.message, "blocker.message", MAX_MESSAGE_CHARS)?;
        check_optional_named_id(self.product_id, "blocker.product_id")?;
        check_optional_named_id(self.product_revision_id, "blocker.product_revision_id")?;
        check_optional_named_id(self.part_version_id, "blocker.part_version_id")?;
        check_optional_named_id(self.component_id, "blocker.component_id")
    }
}

/// One selected member revision and its readiness. A ready member carries no blocker; a blocked
/// member carries at least one.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemberReadiness {
    pub product_id: Uuid,
    pub product_revision_id: Uuid,
    pub state: ReadinessState,
    /// The blockers in canonical order ([`ReadinessOwner::rank`], code, field path).
    pub blockers: Vec<ReadinessBlocker>,
}

impl MemberReadiness {
    /// A member whose state follows its blockers, stored in canonical order.
    pub fn new(product_id: Uuid, product_revision_id: Uuid, mut blockers: Vec<ReadinessBlocker>) -> Self {
        blockers.sort_by(|left, right| left.sort_key().cmp(&right.sort_key()));
        let state = if blockers.is_empty() { ReadinessState::Ready } else { ReadinessState::Blocked };
        Self { product_id, product_revision_id, state, blockers }
    }

    /// Validate the member's identity, state and blocker scope. A blocker may name no product (a
    /// project-wide required setting); when it names one it must be this member's product, and when
    /// it names a revision that must be this member's revision.
    pub fn validate(&self) -> Result<()> {
        check_named_id(self.product_id, "member.product_id")?;
        check_named_id(self.product_revision_id, "member.product_revision_id")?;
        if self.blockers.len() > MAX_READINESS_BLOCKERS {
            return Err(ContractError::invalid("member.blockers", format!("a member holds at most {MAX_READINESS_BLOCKERS} blockers")));
        }
        let mut previous: Option<(u8, String, String)> = None;
        for blocker in &self.blockers {
            blocker.validate()?;
            if let Some(product_id) = blocker.product_id
                && product_id != self.product_id
            {
                return Err(ContractError::invalid(
                    "member.blockers.product_id",
                    "a member's blocker names another product; a shared part blocker names the part version instead",
                ));
            }
            if let Some(product_revision_id) = blocker.product_revision_id
                && product_revision_id != self.product_revision_id
            {
                return Err(ContractError::invalid("member.blockers.product_revision_id", "a member's blocker names another product revision"));
            }
            let key = (blocker.owner.rank(), blocker.code.clone(), blocker.field_path.clone());
            if let Some(previous) = previous
                && key <= previous
            {
                return Err(ContractError::invalid("member.blockers", "the blockers are sorted in canonical order without duplicates"));
            }
            previous = Some(key);
        }
        match self.state {
            ReadinessState::Ready if !self.blockers.is_empty() => Err(ContractError::invalid("member.state", "a ready member carries no blocker")),
            ReadinessState::Blocked if self.blockers.is_empty() => {
                Err(ContractError::invalid("member.state", "a blocked member names at least one blocker"))
            }
            _ => Ok(()),
        }
    }
}

// ---------------------------------------------------------------------------
// Shared-part definition pins
// ---------------------------------------------------------------------------

/// One shared part version's accepted material definition: the selected material version and the
/// net consumption it is costed from. Provenance and review fields are deliberately not part of the
/// definition: confirming an unchanged value is not a definition change.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SharedMaterialDefinition {
    #[serde(default)]
    pub material_version_id: Option<Uuid>,
    #[serde(default)]
    pub quantity: Option<Decimal>,
    pub unit: Unit,
    #[serde(default)]
    pub allowance_rate: Option<Decimal>,
}

impl SharedMaterialDefinition {
    /// Validate the material shape: a volume unit, non-negative values and a non-nil version.
    pub fn validate(&self) -> Result<()> {
        check_optional_named_id(self.material_version_id, "material.material_version_id")?;
        check_volume_unit(self.unit, "material.unit")?;
        check_optional_non_negative(self.quantity, "material.quantity")?;
        check_optional_non_negative(self.allowance_rate, "material.allowance_rate")
    }
}

/// One shared part version's accepted dimension: the reviewed volume the cost basis used. The
/// volume is the shared dimension the costing baseline consumes; more detailed feasibility
/// measurements are SB-14 evidence, not a cost definition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SharedDimensionDefinition {
    #[serde(default)]
    pub volume_mm3: Option<Decimal>,
}

impl SharedDimensionDefinition {
    /// Validate the dimension shape.
    pub fn validate(&self) -> Result<()> {
        check_optional_non_negative(self.volume_mm3, "dimensions.volume_mm3")
    }
}

/// The accepted cycle basis of one routing operation, without the cost-center rate version: a rate
/// version change is shared-data adoption (SB-20), not a shared-part definition change.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct SharedCycleDefinition {
    #[serde(default)]
    pub seconds: Option<Decimal>,
    pub basis: CycleBasis,
    #[serde(default)]
    pub cavities: Option<u32>,
}

impl SharedCycleDefinition {
    /// Validate the cycle shape with the same rules the sourcing contract applies.
    pub fn validate(&self) -> Result<()> {
        check_optional_non_negative(self.seconds, "operation.cycle.seconds")?;
        if let Some(cavities) = self.cavities {
            if cavities == 0 {
                return Err(ContractError::invalid("operation.cycle.cavities", "at least one cavity is needed"));
            }
            if !self.basis.adjusts_for_cavities() && cavities > 1 {
                return Err(ContractError::invalid(
                    "operation.cycle.cavities",
                    "a per-piece cycle is not adjusted by cavities; declare the per-cycle basis instead",
                ));
            }
        }
        Ok(())
    }
}

/// The accepted setup shape of one routing operation, without the setup rate version.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct SharedSetupDefinition {
    #[serde(default)]
    pub hours: Option<Decimal>,
}

impl SharedSetupDefinition {
    /// Validate the setup shape.
    pub fn validate(&self) -> Result<()> {
        check_optional_non_negative(self.hours, "operation.setup.hours")
    }
}

/// One accepted routing operation in its semantic costing shape. Names, descriptions and operation
/// ids are labels and identities, not cost definitions; a description edit does not force
/// coordination.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SharedOperationDefinition {
    pub sequence: u32,
    #[serde(default)]
    pub cost_center_id: Option<Uuid>,
    #[serde(default)]
    pub cycle: Option<SharedCycleDefinition>,
    #[serde(default)]
    pub setup: Option<SharedSetupDefinition>,
    #[serde(default)]
    pub tooling_reference: Option<String>,
}

impl SharedOperationDefinition {
    /// Validate the operation shape.
    pub fn validate(&self) -> Result<()> {
        if self.sequence == 0 {
            return Err(ContractError::invalid("operation.sequence", "an operation sequence starts at 1"));
        }
        check_optional_named_id(self.cost_center_id, "operation.cost_center_id")?;
        if let Some(cycle) = &self.cycle {
            cycle.validate()?;
        }
        if let Some(setup) = &self.setup {
            setup.validate()?;
        }
        check_optional_bounded_text(&self.tooling_reference, "operation.tooling_reference", MAX_TOOLING_REFERENCE_CHARS)
    }
}

/// One shared part version's accepted manufacturing routing definition: its operations in sequence
/// order.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SharedRoutingDefinition {
    pub operations: Vec<SharedOperationDefinition>,
}

impl SharedRoutingDefinition {
    /// Validate the routing shape: at least one operation with strictly increasing sequences.
    pub fn validate(&self) -> Result<()> {
        if self.operations.is_empty() {
            return Err(ContractError::invalid("routing.operations", "an accepted routing names at least one operation"));
        }
        if self.operations.len() > crate::manufacturing::MAX_OPERATIONS {
            return Err(ContractError::invalid(
                "routing.operations",
                format!("a routing holds at most {} operations", crate::manufacturing::MAX_OPERATIONS),
            ));
        }
        let mut previous: Option<u32> = None;
        for operation in &self.operations {
            operation.validate()?;
            if let Some(previous) = previous
                && operation.sequence <= previous
            {
                return Err(ContractError::invalid("routing.operations", "the operations are ordered by a strictly increasing sequence"));
            }
            previous = Some(operation.sequence);
        }
        Ok(())
    }
}

/// One shared part version's accepted definition pin: the customer identity plus the four aspects
/// the confirmed coordinated-activation rule covers (sourcing, material, dimensions and
/// manufacturing routing). `None` means the aspect was not defined when the baseline accepted it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AcceptedSharedDefinition {
    pub part_version_id: Uuid,
    pub customer_part_number: String,
    pub customer_revision: String,
    #[serde(default)]
    pub sourcing: Option<Sourcing>,
    #[serde(default)]
    pub material: Option<SharedMaterialDefinition>,
    #[serde(default)]
    pub dimensions: Option<SharedDimensionDefinition>,
    #[serde(default)]
    pub routing: Option<SharedRoutingDefinition>,
}

impl AcceptedSharedDefinition {
    /// Validate the pin. A purchased part never carries manufacturing material, exactly as the
    /// sourcing contract stores it.
    pub fn validate(&self) -> Result<()> {
        check_named_id(self.part_version_id, "shared_definition.part_version_id")?;
        check_bounded_text(&self.customer_part_number, "shared_definition.customer_part_number", MAX_IDENTITY_CHARS)?;
        check_bounded_text(&self.customer_revision, "shared_definition.customer_revision", MAX_IDENTITY_CHARS)?;
        if self.sourcing == Some(Sourcing::Purchased) && self.material.is_some() {
            return Err(ContractError::invalid("shared_definition.material", "a purchased part records no manufacturing material"));
        }
        if let Some(material) = &self.material {
            material.validate()?;
        }
        if let Some(dimensions) = &self.dimensions {
            dimensions.validate()?;
        }
        if let Some(routing) = &self.routing {
            routing.validate()?;
        }
        Ok(())
    }
}

/// Map one stored sourcing record to the part's accepted material definition. `None` means the part
/// has no manufacturing definition or no material record, never an implicit material.
pub fn shared_material_definition(part: Option<&SourcedPart>) -> Option<SharedMaterialDefinition> {
    let material = part?.material.as_ref()?;
    Some(SharedMaterialDefinition {
        material_version_id: material.material_version_id,
        quantity: material.quantity,
        unit: material.unit,
        allowance_rate: material.allowance_rate,
    })
}

/// Map one reviewed part record to the accepted dimension definition. Only the reviewed volume is a
/// shared dimension; a missing reviewed volume stays `None`, never zero.
pub fn shared_dimension_definition(reviewed: Option<&ReviewedPart>) -> Option<SharedDimensionDefinition> {
    let volume = reviewed?.volume_mm3.as_ref()?;
    Some(SharedDimensionDefinition { volume_mm3: volume.value })
}

/// Map one stored routing to the accepted manufacturing routing definition. Operations are put in
/// sequence order; provenance, names, descriptions, operation ids and pinned rate versions are not
/// part of the definition.
pub fn shared_routing_definition(routing: Option<&AppliedRouting>) -> Option<SharedRoutingDefinition> {
    let routing = routing?;
    let mut operations: Vec<SharedOperationDefinition> = routing
        .operations
        .iter()
        .map(|operation| SharedOperationDefinition {
            sequence: operation.sequence,
            cost_center_id: operation.cost_center_id,
            cycle: operation.cycle.as_ref().map(|cycle| SharedCycleDefinition {
                seconds: cycle.seconds,
                basis: cycle.basis,
                cavities: cycle.cavities,
            }),
            setup: operation.setup.as_ref().map(|setup| SharedSetupDefinition { hours: setup.hours }),
            tooling_reference: operation.tooling_reference.clone(),
        })
        .collect();
    operations.sort_by_key(|operation| operation.sequence);
    Some(SharedRoutingDefinition { operations })
}

/// Build the accepted definition pin of one shared part version from the exact stored records an
/// activation read. The service calls this for every part version in the activation scope, so the
/// pin and the readiness gate cannot disagree about what the definition is.
pub fn accepted_shared_definition(
    part_version: &PartVersion,
    part: Option<&SourcedPart>,
    reviewed: Option<&ReviewedPart>,
    routing: Option<&AppliedRouting>,
) -> AcceptedSharedDefinition {
    AcceptedSharedDefinition {
        part_version_id: part_version.part_version_id,
        customer_part_number: part_version.customer_part_number.clone(),
        customer_revision: part_version.customer_revision.clone(),
        sourcing: part.map(|part| part.sourcing),
        material: shared_material_definition(part),
        dimensions: shared_dimension_definition(reviewed),
        routing: shared_routing_definition(routing),
    }
}

// ---------------------------------------------------------------------------
// Coordinated activation
// ---------------------------------------------------------------------------

/// One aspect of a shared part definition that differs from the accepted definition. The order is
/// canonical: sourcing, material, dimensions, routing.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DefinitionChange {
    Sourcing,
    Material,
    Dimensions,
    Routing,
}

impl DefinitionChange {
    /// The canonical position of the aspect.
    pub fn rank(self) -> u8 {
        match self {
            Self::Sourcing => 0,
            Self::Material => 1,
            Self::Dimensions => 2,
            Self::Routing => 3,
        }
    }
}

/// Compare the current definition pin of a shared part version with the accepted one and name the
/// differing aspects in canonical order. An empty result means the definition is unchanged.
pub fn shared_definition_changes(accepted: &AcceptedSharedDefinition, current: &AcceptedSharedDefinition) -> Vec<DefinitionChange> {
    let mut changes = Vec::new();
    if accepted.sourcing != current.sourcing {
        changes.push(DefinitionChange::Sourcing);
    }
    if accepted.material != current.material {
        changes.push(DefinitionChange::Material);
    }
    if accepted.dimensions != current.dimensions {
        changes.push(DefinitionChange::Dimensions);
    }
    if accepted.routing != current.routing {
        changes.push(DefinitionChange::Routing);
    }
    changes
}

/// One shared part version whose definition differs from the accepted definition, with the products
/// that must be activated together because their costed structures use it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChangedSharedDefinition {
    pub part_version_id: Uuid,
    pub customer_part_number: String,
    pub customer_revision: String,
    /// The differing aspects in canonical order; never empty.
    pub changes: Vec<DefinitionChange>,
    /// The quoted products whose resulting active revision costs this part version, sorted and
    /// unique; never empty.
    pub affected_product_ids: Vec<Uuid>,
}

impl ChangedSharedDefinition {
    /// A changed definition with its aspects and affected products in canonical order.
    pub fn new(
        part_version_id: Uuid,
        customer_part_number: impl Into<String>,
        customer_revision: impl Into<String>,
        changes: Vec<DefinitionChange>,
        affected_product_ids: Vec<Uuid>,
    ) -> Self {
        let mut changes = changes;
        changes.sort_by_key(|change| change.rank());
        changes.dedup();
        let mut affected_product_ids = affected_product_ids;
        affected_product_ids.sort_unstable();
        affected_product_ids.dedup();
        Self {
            part_version_id,
            customer_part_number: customer_part_number.into(),
            customer_revision: customer_revision.into(),
            changes,
            affected_product_ids,
        }
    }

    /// Validate one changed definition.
    pub fn validate(&self) -> Result<()> {
        check_named_id(self.part_version_id, "changed_definition.part_version_id")?;
        check_bounded_text(&self.customer_part_number, "changed_definition.customer_part_number", MAX_IDENTITY_CHARS)?;
        check_bounded_text(&self.customer_revision, "changed_definition.customer_revision", MAX_IDENTITY_CHARS)?;
        if self.changes.is_empty() {
            return Err(ContractError::invalid("changed_definition.changes", "a changed definition names at least one aspect"));
        }
        let mut previous: Option<u8> = None;
        for change in &self.changes {
            if let Some(previous) = previous
                && change.rank() <= previous
            {
                return Err(ContractError::invalid("changed_definition.changes", "the changes are unique and in canonical order"));
            }
            previous = Some(change.rank());
        }
        check_sorted_unique_ids(&self.affected_product_ids, "changed_definition.affected_product_ids")?;
        if self.affected_product_ids.is_empty() {
            return Err(ContractError::invalid(
                "changed_definition.affected_product_ids",
                "a changed definition is used by at least one quoted product",
            ));
        }
        Ok(())
    }
}

/// What a readiness report says about coordinated activation: the shared definitions whose change
/// is still pending and every quotation-included product that must join the activation because of
/// them.
///
/// The set is computed for the requested member set only. The activation route recomputes it from
/// the final request and refuses with the updated affected set, so one readiness read is never a
/// proof that the group has converged.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoordinationReport {
    /// The changed shared definitions ordered by part version id; empty when no definition changed.
    pub changed_definitions: Vec<ChangedSharedDefinition>,
    /// The sorted union of the changed definitions' affected products; empty when nothing is
    /// pending. Every affected product must be selected for the activation to be accepted.
    pub required_product_ids: Vec<Uuid>,
}

impl CoordinationReport {
    /// The report for one set of pending changes, with the definitions and the required set in
    /// canonical order.
    pub fn new(mut changed_definitions: Vec<ChangedSharedDefinition>) -> Self {
        changed_definitions.sort_by_key(|definition| definition.part_version_id);
        let mut required_product_ids: Vec<Uuid> =
            changed_definitions.iter().flat_map(|definition| definition.affected_product_ids.iter().copied()).collect();
        required_product_ids.sort_unstable();
        required_product_ids.dedup();
        Self { changed_definitions, required_product_ids }
    }

    /// Validate the report: sorted definitions, sorted unique required products and a required set
    /// that is exactly the union of the affected products.
    pub fn validate(&self) -> Result<()> {
        if self.changed_definitions.len() > MAX_SHARED_DEFINITIONS {
            return Err(ContractError::invalid(
                "coordination.changed_definitions",
                format!("at most {MAX_SHARED_DEFINITIONS} changed definitions are reported"),
            ));
        }
        check_sorted_unique_ids(&self.required_product_ids, "coordination.required_product_ids")?;
        let mut affected: Vec<Uuid> = Vec::new();
        let mut previous: Option<Uuid> = None;
        for definition in &self.changed_definitions {
            definition.validate()?;
            if let Some(previous) = previous
                && definition.part_version_id <= previous
            {
                return Err(ContractError::invalid(
                    "coordination.changed_definitions",
                    "the definitions are sorted by part version id without duplicates",
                ));
            }
            previous = Some(definition.part_version_id);
            affected.extend(definition.affected_product_ids.iter().copied());
        }
        affected.sort_unstable();
        affected.dedup();
        if affected != self.required_product_ids {
            return Err(ContractError::invalid(
                "coordination.required_product_ids",
                "the required set is exactly the union of the affected products",
            ));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Readiness request and report
// ---------------------------------------------------------------------------

/// Ask for the readiness of the selected member revisions at one exact project record version. The
/// request changes nothing.
///
/// A selection names the product's current `draft` (the activation would activate it) or its
/// current `active` revision (the activation would re-pin the accepted shared definitions without a
/// revision state transition). Any other revision is refused with
/// [`ActivationRefusal::RevisionNotActivatable`] (`409`), exactly as the activation route refuses it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActivationReadinessRequest {
    /// The exact project record version to read; it must be the current version.
    pub project_version: u64,
    /// One current draft or current active revision per product, unique by product.
    pub selections: Vec<ProductRevisionSelection>,
}

impl ActivationReadinessRequest {
    /// Validate the request envelope.
    pub fn validate(&self) -> Result<()> {
        check_version(self.project_version, "readiness.project_version")?;
        validate_selections(&self.selections, "readiness.selections")
    }
}

/// The readiness answer for one selection at one project record version.
///
/// This is a point-in-time report. Adding a required member can expose a further changed shared
/// definition and further affected products, so the activation route recomputes the group from the
/// final request; a client must not assume one readiness call has stabilised it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActivationReadinessReport {
    pub project_id: Uuid,
    pub project_version: u64,
    /// The current accepted baseline, or `None` before the project accepted its first activation.
    /// The client echoes it in [`ActivateRevisions::expected_accepted_baseline_id`].
    #[serde(default)]
    pub accepted_baseline_id: Option<Uuid>,
    /// One entry per selected revision, sorted by product id.
    pub members: Vec<MemberReadiness>,
    pub coordination: CoordinationReport,
}

impl ActivationReadinessReport {
    /// Validate the report's structure and invariants.
    pub fn validate(&self) -> Result<()> {
        check_named_id(self.project_id, "report.project_id")?;
        check_version(self.project_version, "report.project_version")?;
        check_optional_named_id(self.accepted_baseline_id, "report.accepted_baseline_id")?;
        if self.members.len() > MAX_ACTIVATION_MEMBERS {
            return Err(ContractError::invalid("report.members", format!("at most {MAX_ACTIVATION_MEMBERS} members are reported")));
        }
        let mut previous: Option<Uuid> = None;
        for member in &self.members {
            member.validate()?;
            if let Some(previous) = previous
                && member.product_id <= previous
            {
                return Err(ContractError::invalid("report.members", "the members are sorted by product id without duplicates"));
            }
            previous = Some(member.product_id);
        }
        self.coordination.validate()
    }
}

// ---------------------------------------------------------------------------
// Activation request, receipt and facts
// ---------------------------------------------------------------------------

/// Activate draft product revisions and re-pin current active revisions atomically, including every
/// member the coordinated shared-definition rule requires.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActivateRevisions {
    pub request_id: String,
    /// The current project record version the caller read; a different one is `409 conflict`.
    pub expected_project_version: u64,
    /// The current accepted baseline the caller read. `None` (absent or `null`) means the caller
    /// read a project with no accepted baseline yet; a different current head is `409 conflict`.
    #[serde(default)]
    pub expected_accepted_baseline_id: Option<Uuid>,
    /// One current draft (activate it) or current active revision (re-pin it) per product, unique by
    /// product. A historical, superseded or foreign revision is refused with
    /// [`ActivationRefusal::RevisionNotActivatable`].
    pub product_revisions: Vec<ProductRevisionSelection>,
}

impl ActivateRevisions {
    /// Validate the request envelope.
    pub fn validate(&self) -> Result<()> {
        check_activation_request_id(&self.request_id)?;
        check_version(self.expected_project_version, "activation.expected_project_version")?;
        check_optional_named_id(self.expected_accepted_baseline_id, "activation.expected_accepted_baseline_id")?;
        validate_selections(&self.product_revisions, "activation.product_revisions")
    }
}

/// What one member contributed to the accepted state.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActivationAction {
    /// The selected current draft became the product's active revision; the previous active
    /// revision became historical.
    Activated,
    /// The selected current active revision stays active; the activation only re-pins the accepted
    /// shared definitions. There is no revision state transition.
    Repinned,
}

/// One product revision accepted by an activation, and what the activation did to it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActivationMember {
    pub product_id: Uuid,
    pub product_revision_id: Uuid,
    pub action: ActivationAction,
    /// The revision that became `historical` (`Activated` only), or `None` when the product
    /// activates its first revision or the member is a re-pin.
    #[serde(default)]
    pub previous_active_revision_id: Option<Uuid>,
}

impl ActivationMember {
    /// Validate the member's identity and action.
    pub fn validate(&self) -> Result<()> {
        check_named_id(self.product_id, "member.product_id")?;
        check_named_id(self.product_revision_id, "member.product_revision_id")?;
        check_optional_named_id(self.previous_active_revision_id, "member.previous_active_revision_id")?;
        if self.previous_active_revision_id == Some(self.product_revision_id) {
            return Err(ContractError::invalid("member.previous_active_revision_id", "the replaced revision cannot be the accepted revision"));
        }
        if self.action == ActivationAction::Repinned && self.previous_active_revision_id.is_some() {
            return Err(ContractError::invalid(
                "member.previous_active_revision_id",
                "a re-pinned member keeps its active revision; no revision became historical",
            ));
        }
        Ok(())
    }
}

/// The exact immutable record versions one activation read and pinned.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct PinnedRecordVersions {
    /// The project record version the activation appended.
    pub project_version: u64,
    pub engineering_version: u64,
    pub manufacturing_version: u64,
    pub feasibility_version: u64,
}

impl PinnedRecordVersions {
    /// Validate the pinned versions: every immutable record starts at version 1.
    pub fn validate(&self) -> Result<()> {
        check_version(self.project_version, "pinned.project_version")?;
        check_version(self.engineering_version, "pinned.engineering_version")?;
        check_version(self.manufacturing_version, "pinned.manufacturing_version")?;
        check_version(self.feasibility_version, "pinned.feasibility_version")
    }
}

/// One dependency fact an activation emits for SB-20. Facts name what changed; they never rewrite an
/// accepted result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActivationFact {
    /// A new accepted baseline heads the project.
    BaselineAccepted {
        accepted_baseline_id: Uuid,
        #[serde(default)]
        previous_accepted_baseline_id: Option<Uuid>,
        project_version: u64,
    },
    /// One product revision became the product's active revision.
    RevisionActivated {
        product_id: Uuid,
        product_revision_id: Uuid,
        #[serde(default)]
        previous_active_revision_id: Option<Uuid>,
    },
    /// One current active revision was re-pinned without a revision state transition.
    RevisionRepinned { product_id: Uuid, product_revision_id: Uuid },
    /// One shared part version's definition was accepted with the aspects that differ from the
    /// previously accepted definition.
    SharedDefinitionChanged { part_version_id: Uuid, customer_part_number: String, customer_revision: String, changes: Vec<DefinitionChange> },
}

impl ActivationFact {
    /// Validate the fact's values.
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::BaselineAccepted { accepted_baseline_id, previous_accepted_baseline_id, project_version } => {
                check_named_id(*accepted_baseline_id, "fact.accepted_baseline_id")?;
                check_optional_named_id(*previous_accepted_baseline_id, "fact.previous_accepted_baseline_id")?;
                check_version(*project_version, "fact.project_version")
            }
            Self::RevisionActivated { product_id, product_revision_id, previous_active_revision_id } => ActivationMember {
                product_id: *product_id,
                product_revision_id: *product_revision_id,
                action: ActivationAction::Activated,
                previous_active_revision_id: *previous_active_revision_id,
            }
            .validate(),
            Self::RevisionRepinned { product_id, product_revision_id } => ActivationMember {
                product_id: *product_id,
                product_revision_id: *product_revision_id,
                action: ActivationAction::Repinned,
                previous_active_revision_id: None,
            }
            .validate(),
            Self::SharedDefinitionChanged { part_version_id, customer_part_number, customer_revision, changes } => {
                check_named_id(*part_version_id, "fact.part_version_id")?;
                check_bounded_text(customer_part_number, "fact.customer_part_number", MAX_IDENTITY_CHARS)?;
                check_bounded_text(customer_revision, "fact.customer_revision", MAX_IDENTITY_CHARS)?;
                check_changes(changes, "fact.changes")
            }
        }
    }
}

/// The committed answer to one activation request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActivationReceipt {
    pub request_id: String,
    pub project_id: Uuid,
    /// The new project record version and the exact record versions the activation pinned.
    pub pinned: PinnedRecordVersions,
    pub accepted_baseline_id: Uuid,
    /// The immutable SB-16 costing snapshot written for the accepted project version.
    pub costing_calculation_id: Uuid,
    /// The canonical hash of the resolved [`crate::costing::BaselineInputs`]; the snapshot keeps
    /// every referenced record and reference version it hashed.
    pub costing_input_hash: String,
    pub calculator_version: String,
    /// The state of the pinned costing calculation. An activation accepts complete activated
    /// revisions; an unrelated product's pre-existing incompleteness is visible here, never hidden.
    pub costing_state: ResultState,
    /// The accepted members sorted by product id; never empty.
    pub members: Vec<ActivationMember>,
    /// The dependency facts for SB-20, with the baseline fact first.
    pub facts: Vec<ActivationFact>,
}

impl ActivationReceipt {
    /// The project record version this receipt created.
    pub fn project_version(&self) -> u64 {
        self.pinned.project_version
    }

    /// Validate the receipt's structure and invariants.
    pub fn validate(&self) -> Result<()> {
        check_activation_request_id(&self.request_id)?;
        check_named_id(self.project_id, "receipt.project_id")?;
        self.pinned.validate()?;
        check_named_id(self.accepted_baseline_id, "receipt.accepted_baseline_id")?;
        check_named_id(self.costing_calculation_id, "receipt.costing_calculation_id")?;
        check_sha256(&self.costing_input_hash, "receipt.costing_input_hash")?;
        check_bounded_text(&self.calculator_version, "receipt.calculator_version", MAX_CALCULATOR_VERSION_CHARS)?;
        validate_activation_members(&self.members, "receipt.members")?;
        if self.facts.is_empty() {
            return Err(ContractError::invalid("receipt.facts", "an activation emits its dependency facts"));
        }
        for fact in &self.facts {
            fact.validate()?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Accepted baselines
// ---------------------------------------------------------------------------

/// One append-only accepted baseline: the immutable record of a committed activation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AcceptedBaseline {
    pub accepted_baseline_id: Uuid,
    pub project_id: Uuid,
    /// The accepted baseline order within the project, starting at 1; the largest sequence is the
    /// current accepted baseline.
    pub sequence: u64,
    /// The activation request this baseline accepted.
    pub request_id: String,
    /// The project record version the activation appended and the exact records it read.
    pub pinned: PinnedRecordVersions,
    /// The immutable SB-16 costing snapshot of the accepted project version.
    pub costing_calculation_id: Uuid,
    pub costing_input_hash: String,
    pub calculator_version: String,
    pub costing_state: ResultState,
    /// The members this activation accepted, sorted by product id.
    pub members: Vec<ActivationMember>,
    /// The resulting active revision of every product that has one, sorted by product id.
    pub active_revisions: Vec<ProductRevisionSelection>,
    /// The project's full accepted shared-definition set, sorted by part version id: one pin for
    /// every shared part version used by the resulting active structures of all quotation-included
    /// products, computed at this activation and not only for the members it touched. Carrying the
    /// full set is what keeps a changed definition of an untouched part coordinated: a newest
    /// baseline that dropped an unactivated part's pin would silently accept a different definition
    /// later. Empty only when the accepted state uses no shared part version at all.
    pub shared_definitions: Vec<AcceptedSharedDefinition>,
    /// The RFC 3339 UTC timestamp the service recorded.
    #[serde(default)]
    pub created_at: Option<String>,
}

impl AcceptedBaseline {
    /// Validate the record's structure and invariants.
    pub fn validate(&self) -> Result<()> {
        check_named_id(self.accepted_baseline_id, "accepted_baseline.accepted_baseline_id")?;
        check_named_id(self.project_id, "accepted_baseline.project_id")?;
        check_version(self.sequence, "accepted_baseline.sequence")?;
        check_activation_request_id(&self.request_id)?;
        self.pinned.validate()?;
        check_named_id(self.costing_calculation_id, "accepted_baseline.costing_calculation_id")?;
        check_sha256(&self.costing_input_hash, "accepted_baseline.costing_input_hash")?;
        check_bounded_text(&self.calculator_version, "accepted_baseline.calculator_version", MAX_CALCULATOR_VERSION_CHARS)?;
        validate_activation_members(&self.members, "accepted_baseline.members")?;
        validate_selections_sorted(&self.active_revisions, "accepted_baseline.active_revisions")?;
        if self.active_revisions.is_empty() {
            return Err(ContractError::invalid("accepted_baseline.active_revisions", "an activation leaves at least one active revision"));
        }
        if self.shared_definitions.len() > MAX_SHARED_DEFINITIONS {
            return Err(ContractError::invalid(
                "accepted_baseline.shared_definitions",
                format!("at most {MAX_SHARED_DEFINITIONS} shared definitions are pinned"),
            ));
        }
        check_sorted_shared_definitions(&self.shared_definitions)?;
        check_optional_bounded_text(&self.created_at, "accepted_baseline.created_at", MAX_MESSAGE_CHARS)
    }
}

/// The compact history row of one accepted baseline.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AcceptedBaselineSummary {
    pub accepted_baseline_id: Uuid,
    pub project_id: Uuid,
    pub sequence: u64,
    pub project_version: u64,
    pub engineering_version: u64,
    pub manufacturing_version: u64,
    pub feasibility_version: u64,
    pub costing_calculation_id: Uuid,
    pub costing_state: ResultState,
    #[serde(default)]
    pub created_at: Option<String>,
}

impl AcceptedBaselineSummary {
    /// Validate the history row.
    pub fn validate(&self) -> Result<()> {
        check_named_id(self.accepted_baseline_id, "summary.accepted_baseline_id")?;
        check_named_id(self.project_id, "summary.project_id")?;
        check_version(self.sequence, "summary.sequence")?;
        check_version(self.project_version, "summary.project_version")?;
        check_version(self.engineering_version, "summary.engineering_version")?;
        check_version(self.manufacturing_version, "summary.manufacturing_version")?;
        check_version(self.feasibility_version, "summary.feasibility_version")?;
        check_named_id(self.costing_calculation_id, "summary.costing_calculation_id")?;
        check_optional_bounded_text(&self.created_at, "summary.created_at", MAX_MESSAGE_CHARS)
    }
}

/// The accepted-baseline history of one project, newest first.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AcceptedBaselineList {
    pub project_id: Uuid,
    pub baselines: Vec<AcceptedBaselineSummary>,
}

impl AcceptedBaselineList {
    /// Validate the history: every row names this project and the sequences strictly decrease.
    pub fn validate(&self) -> Result<()> {
        check_named_id(self.project_id, "list.project_id")?;
        if self.baselines.len() > MAX_ACCEPTED_BASELINE_HISTORY {
            return Err(ContractError::invalid(
                "history.baselines",
                format!("a history read returns at most {MAX_ACCEPTED_BASELINE_HISTORY} baselines"),
            ));
        }
        let mut previous: Option<u64> = None;
        for baseline in &self.baselines {
            baseline.validate()?;
            if baseline.project_id != self.project_id {
                return Err(ContractError::invalid("history.baselines.project_id", "a history row names another project"));
            }
            if let Some(previous) = previous
                && baseline.sequence >= previous
            {
                return Err(ContractError::invalid("history.baselines", "the history is newest first with unique sequences"));
            }
            previous = Some(baseline.sequence);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------

/// A stable refusal reason the service renders as the version 1 error body message prefix. The
/// version 2 error code set stays closed, so the typed reason lives in the message and in the
/// structured readiness report; [`ActivationRefusal::http_status`] gives the status the service
/// returns. [`CoordinatedActivationRequired`](Self::CoordinatedActivationRequired),
/// [`MemberNotReady`](Self::MemberNotReady),
/// [`RevisionNotActivatable`](Self::RevisionNotActivatable), [`StaleProjectVersion`](Self::StaleProjectVersion)
/// and [`StaleAcceptedBaseline`](Self::StaleAcceptedBaseline) are state conflicts (`409`); only a
/// malformed or unrepresentable payload is [`InvalidRequest`](Self::InvalidRequest) (`422`). The
/// structured affected set, blockers and readiness states are the readiness report response, not the
/// error body.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActivationRefusal {
    /// The activation would accept a changed shared definition without every required product.
    /// `affected_products` holds the required products the request omits, sorted by id.
    CoordinatedActivationRequired { affected_products: Vec<Uuid> },
    /// One selected member has open readiness blockers.
    MemberNotReady { product_id: Uuid, product_revision_id: Uuid, blockers: Vec<ReadinessBlocker> },
    /// One selected revision is neither the product's current draft nor its current active
    /// revision (a historical or otherwise inactivatable revision).
    RevisionNotActivatable { product_id: Uuid, product_revision_id: Uuid },
    /// The request's expected project record version is stale.
    StaleProjectVersion { expected: u64, current: u64 },
    /// The request's expected accepted baseline is stale.
    StaleAcceptedBaseline {
        #[serde(default)]
        expected: Option<Uuid>,
        #[serde(default)]
        current: Option<Uuid>,
    },
    /// The request itself is malformed or names a revision the project does not hold.
    InvalidRequest { reason: String },
}

impl ActivationRefusal {
    /// The stable refusal code that prefixes the error message.
    pub fn code(&self) -> &'static str {
        match self {
            Self::CoordinatedActivationRequired { .. } => "coordinated_activation_required",
            Self::MemberNotReady { .. } => "member_not_ready",
            Self::RevisionNotActivatable { .. } => "revision_not_activatable",
            Self::StaleProjectVersion { .. } => "stale_project_version",
            Self::StaleAcceptedBaseline { .. } => "stale_accepted_baseline",
            Self::InvalidRequest { .. } => "invalid_request",
        }
    }

    /// The HTTP status the service returns: `409` for a state conflict, `422` for a malformed
    /// request.
    pub fn http_status(&self) -> u16 {
        match self {
            Self::InvalidRequest { .. } => 422,
            _ => 409,
        }
    }

    /// The client-safe message the version 1 error body carries: the stable code, then a colon and
    /// the detail. The coordinated refusal names every affected product; the not-ready refusal
    /// names the blockers.
    pub fn message(&self) -> String {
        let detail = match self {
            Self::CoordinatedActivationRequired { affected_products } => {
                format!("the changed shared definition requires activating these products together: {}", join_ids(affected_products))
            }
            Self::MemberNotReady { product_id, product_revision_id, blockers } => {
                let mut names: Vec<String> =
                    blockers.iter().take(REFUSAL_BLOCKER_NAMES).map(|blocker| format!("{} at {}", blocker.code, blocker.field_path)).collect();
                if blockers.len() > REFUSAL_BLOCKER_NAMES {
                    names.push(format!("and {} more", blockers.len() - REFUSAL_BLOCKER_NAMES));
                }
                format!("product {product_id} revision {product_revision_id} has open readiness blockers: {}", names.join("; "))
            }
            Self::RevisionNotActivatable { product_id, product_revision_id } => {
                format!("product {product_id} revision {product_revision_id} is neither the current draft nor the current active revision")
            }
            Self::StaleProjectVersion { expected, current } => format!("the project is at version {current}, not {expected}"),
            Self::StaleAcceptedBaseline { expected, current } => {
                format!("the current accepted baseline is {}, not {}", describe_baseline(*current), describe_baseline(*expected))
            }
            Self::InvalidRequest { reason } => reason.clone(),
        };
        let message = format!("{}: {detail}", self.code());
        truncate_message(message)
    }

    /// Validate the refusal's values. The service builds refusals, so this also guards a coding
    /// mistake from reaching a client.
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::CoordinatedActivationRequired { affected_products } => {
                check_sorted_unique_ids(affected_products, "refusal.affected_products")?;
                if affected_products.is_empty() {
                    return Err(ContractError::invalid("refusal.affected_products", "the refusal names at least one affected product"));
                }
                Ok(())
            }
            Self::MemberNotReady { product_id, product_revision_id, blockers } => {
                check_named_id(*product_id, "refusal.product_id")?;
                check_named_id(*product_revision_id, "refusal.product_revision_id")?;
                if blockers.is_empty() {
                    return Err(ContractError::invalid("refusal.blockers", "a not-ready refusal names at least one blocker"));
                }
                for blocker in blockers {
                    blocker.validate()?;
                }
                Ok(())
            }
            Self::RevisionNotActivatable { product_id, product_revision_id } => {
                check_named_id(*product_id, "refusal.product_id")?;
                check_named_id(*product_revision_id, "refusal.product_revision_id")
            }
            Self::StaleProjectVersion { expected, current } => {
                check_version(*expected, "refusal.expected")?;
                check_version(*current, "refusal.current")
            }
            Self::StaleAcceptedBaseline { expected, current } => {
                check_optional_named_id(*expected, "refusal.expected")?;
                check_optional_named_id(*current, "refusal.current")
            }
            Self::InvalidRequest { reason } => check_bounded_text(reason, "refusal.reason", MAX_MESSAGE_CHARS),
        }
    }
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

/// The request id rule of the version 1/2 write contracts: non-blank, bounded, no control
/// characters.
fn check_activation_request_id(request_id: &str) -> Result<()> {
    check_bounded_text(request_id, "request_id", 128)?;
    if request_id.chars().any(char::is_control) {
        return Err(ContractError::invalid("request_id", "a request id must not contain control characters"));
    }
    Ok(())
}

/// A selected revision list: bounded, non-empty, unique by product, non-nil ids.
fn validate_selections(selections: &[ProductRevisionSelection], field: &str) -> Result<()> {
    if selections.is_empty() {
        return Err(ContractError::invalid(field, "name at least one product revision"));
    }
    if selections.len() > MAX_ACTIVATION_MEMBERS {
        return Err(ContractError::invalid(field, format!("at most {MAX_ACTIVATION_MEMBERS} product revisions are named")));
    }
    let mut products = Vec::with_capacity(selections.len());
    for selection in selections {
        check_named_id(selection.product_id, &format!("{field}.product_id"))?;
        check_named_id(selection.product_revision_id, &format!("{field}.product_revision_id"))?;
        if products.contains(&selection.product_id) {
            return Err(ContractError::invalid(field, "two selections name one product"));
        }
        products.push(selection.product_id);
    }
    Ok(())
}

/// A selection list sorted by product id without duplicates.
fn validate_selections_sorted(selections: &[ProductRevisionSelection], field: &str) -> Result<()> {
    let mut previous: Option<Uuid> = None;
    for selection in selections {
        check_named_id(selection.product_id, &format!("{field}.product_id"))?;
        check_named_id(selection.product_revision_id, &format!("{field}.product_revision_id"))?;
        if let Some(previous) = previous
            && selection.product_id <= previous
        {
            return Err(ContractError::invalid(field, "the selections are sorted by product id without duplicates"));
        }
        previous = Some(selection.product_id);
    }
    Ok(())
}

/// An activation member list: bounded, non-empty, sorted by product id without duplicates.
fn validate_activation_members(members: &[ActivationMember], field: &str) -> Result<()> {
    if members.is_empty() {
        return Err(ContractError::invalid(field, "an activation names at least one member"));
    }
    if members.len() > MAX_ACTIVATION_MEMBERS {
        return Err(ContractError::invalid(field, format!("at most {MAX_ACTIVATION_MEMBERS} members are named")));
    }
    let mut previous: Option<Uuid> = None;
    for member in members {
        member.validate()?;
        if let Some(previous) = previous
            && member.product_id <= previous
        {
            return Err(ContractError::invalid(field, "the members are sorted by product id without duplicates"));
        }
        previous = Some(member.product_id);
    }
    Ok(())
}

/// The shared definition pins sorted by part version id without duplicates.
fn check_sorted_shared_definitions(definitions: &[AcceptedSharedDefinition]) -> Result<()> {
    let mut previous: Option<Uuid> = None;
    for definition in definitions {
        definition.validate()?;
        if let Some(previous) = previous
            && definition.part_version_id <= previous
        {
            return Err(ContractError::invalid("accepted_baseline.shared_definitions", "the pins are sorted by part version id without duplicates"));
        }
        previous = Some(definition.part_version_id);
    }
    Ok(())
}

/// The definition changes in canonical order without duplicates.
fn check_changes(changes: &[DefinitionChange], field: &str) -> Result<()> {
    if changes.is_empty() {
        return Err(ContractError::invalid(field, "name at least one changed aspect"));
    }
    let mut previous: Option<u8> = None;
    for change in changes {
        if let Some(previous) = previous
            && change.rank() <= previous
        {
            return Err(ContractError::invalid(field, "the changes are unique and in canonical order"));
        }
        previous = Some(change.rank());
    }
    Ok(())
}

fn check_named_id(id: Uuid, field: &str) -> Result<()> {
    if id.is_nil() { Err(ContractError::invalid(field, "the id is empty")) } else { Ok(()) }
}

fn check_optional_named_id(id: Option<Uuid>, field: &str) -> Result<()> {
    match id {
        Some(id) => check_named_id(id, field),
        None => Ok(()),
    }
}

fn check_version(version: u64, field: &str) -> Result<()> {
    if version == 0 { Err(ContractError::invalid(field, "a record version starts at 1")) } else { Ok(()) }
}

fn check_sorted_unique_ids(ids: &[Uuid], field: &str) -> Result<()> {
    let mut previous: Option<Uuid> = None;
    for id in ids {
        check_named_id(*id, field)?;
        if let Some(previous) = previous
            && *id <= previous
        {
            return Err(ContractError::invalid(field, "the ids are sorted without duplicates"));
        }
        previous = Some(*id);
    }
    Ok(())
}

fn check_bounded_text(text: &str, field: &str, max_chars: usize) -> Result<()> {
    if text.trim().is_empty() {
        return Err(ContractError::invalid(field, "the value must not be blank"));
    }
    if text.chars().count() > max_chars {
        return Err(ContractError::invalid(field, format!("the value is at most {max_chars} characters")));
    }
    if text.chars().any(char::is_control) {
        return Err(ContractError::invalid(field, "the value must not contain control characters"));
    }
    Ok(())
}

fn check_optional_bounded_text(text: &Option<String>, field: &str, max_chars: usize) -> Result<()> {
    match text {
        Some(text) => check_bounded_text(text, field, max_chars),
        None => Ok(()),
    }
}

fn check_optional_non_negative(value: Option<Decimal>, field: &str) -> Result<()> {
    if let Some(value) = value
        && value.is_sign_negative()
    {
        return Err(ContractError::invalid(field, "the value cannot be negative"));
    }
    Ok(())
}

fn check_volume_unit(unit: Unit, field: &str) -> Result<()> {
    if matches!(unit, Unit::MillimetreCubed | Unit::CentimetreCubed) {
        Ok(())
    } else {
        Err(ContractError::invalid(field, "consumption is a volume in mm3 or cm3"))
    }
}

fn join_ids(ids: &[Uuid]) -> String {
    ids.iter().map(Uuid::to_string).collect::<Vec<String>>().join(", ")
}

fn describe_baseline(baseline: Option<Uuid>) -> String {
    match baseline {
        Some(id) => id.to_string(),
        None => "none".to_string(),
    }
}

/// Bound a rendered refusal message to the contract's message limit without splitting a character.
fn truncate_message(mut message: String) -> String {
    if message.chars().count() <= MAX_MESSAGE_CHARS {
        return message;
    }
    let mut boundary = 0;
    for (index, _) in message.char_indices().take(MAX_MESSAGE_CHARS) {
        boundary = index;
    }
    message.truncate(boundary);
    message
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ReviewState;
    use serde_json::json;

    fn id(value: u128) -> Uuid {
        Uuid::from_u128(value)
    }

    fn blocker(owner: ReadinessOwner, code: &str, field_path: &str) -> ReadinessBlocker {
        ReadinessBlocker::new(owner, code, field_path, "the input is missing")
    }

    fn selection(product: u128, revision: u128) -> ProductRevisionSelection {
        ProductRevisionSelection { product_id: id(product), product_revision_id: id(revision) }
    }

    fn material() -> SharedMaterialDefinition {
        SharedMaterialDefinition {
            material_version_id: Some(id(7)),
            quantity: Some(Decimal::from(120)),
            unit: Unit::MillimetreCubed,
            allowance_rate: Some(Decimal::from(2)),
        }
    }

    fn routing() -> SharedRoutingDefinition {
        SharedRoutingDefinition {
            operations: vec![
                SharedOperationDefinition {
                    sequence: 1,
                    cost_center_id: Some(id(8)),
                    cycle: Some(SharedCycleDefinition { seconds: Some(Decimal::from(30)), basis: CycleBasis::PerPiece, cavities: Some(1) }),
                    setup: Some(SharedSetupDefinition { hours: Some(Decimal::from(1)) }),
                    tooling_reference: None,
                },
                SharedOperationDefinition {
                    sequence: 2,
                    cost_center_id: None,
                    cycle: None,
                    setup: None,
                    tooling_reference: Some("fixture-1".into()),
                },
            ],
        }
    }

    fn definition() -> AcceptedSharedDefinition {
        AcceptedSharedDefinition {
            part_version_id: id(30),
            customer_part_number: "PL-100".into(),
            customer_revision: "B".into(),
            sourcing: Some(Sourcing::Manufactured),
            material: Some(material()),
            dimensions: Some(SharedDimensionDefinition { volume_mm3: Some(Decimal::from(2400)) }),
            routing: Some(routing()),
        }
    }

    fn activated(product: u128, revision: u128, previous: Option<u128>) -> ActivationMember {
        ActivationMember {
            product_id: id(product),
            product_revision_id: id(revision),
            action: ActivationAction::Activated,
            previous_active_revision_id: previous.map(id),
        }
    }

    fn repinned(product: u128, revision: u128) -> ActivationMember {
        ActivationMember {
            product_id: id(product),
            product_revision_id: id(revision),
            action: ActivationAction::Repinned,
            previous_active_revision_id: None,
        }
    }

    fn pinned() -> PinnedRecordVersions {
        PinnedRecordVersions { project_version: 7, engineering_version: 3, manufacturing_version: 5, feasibility_version: 2 }
    }

    fn accepted_baseline() -> AcceptedBaseline {
        AcceptedBaseline {
            accepted_baseline_id: id(90),
            project_id: id(10),
            sequence: 2,
            request_id: "request-1".into(),
            pinned: pinned(),
            costing_calculation_id: id(91),
            costing_input_hash: "a".repeat(64),
            calculator_version: "solidbom-manufacturing-baseline/1".into(),
            costing_state: ResultState::Complete,
            members: vec![activated(2, 20, Some(19))],
            active_revisions: vec![selection(2, 20)],
            shared_definitions: vec![definition()],
            created_at: Some("2026-11-11T00:00:00.000Z".into()),
        }
    }

    #[test]
    fn a_ready_member_carries_no_blocker_and_a_blocked_member_names_one() {
        let ready = MemberReadiness::new(id(2), id(20), Vec::new());
        assert_eq!(ready.state, ReadinessState::Ready);
        assert!(ready.validate().is_ok());

        let blocked = MemberReadiness::new(id(2), id(20), vec![blocker(ReadinessOwner::Costing, "missing_volume_policy", "project.costing")]);
        assert_eq!(blocked.state, ReadinessState::Blocked);
        assert!(blocked.validate().is_ok());

        let wrong_state = MemberReadiness { state: ReadinessState::Ready, ..blocked };
        assert!(wrong_state.validate().is_err());
        let empty_blocked = MemberReadiness::new(id(2), id(20), Vec::new());
        assert!(MemberReadiness { state: ReadinessState::Blocked, ..empty_blocked }.validate().is_err());
    }

    #[test]
    fn a_members_blockers_are_stored_in_canonical_order() {
        let member = MemberReadiness::new(
            id(2),
            id(20),
            vec![
                blocker(ReadinessOwner::Tooling, "missing_treatment", "recovery.treatment"),
                blocker(ReadinessOwner::Engineering, "pending_classification", "parts.30.classification"),
                blocker(ReadinessOwner::Costing, "missing_purchase_price", "parts.30.purchase"),
            ],
        );
        let owners: Vec<ReadinessOwner> = member.blockers.iter().map(|blocker| blocker.owner).collect();
        assert_eq!(owners, vec![ReadinessOwner::Engineering, ReadinessOwner::Costing, ReadinessOwner::Tooling]);
        assert!(member.validate().is_ok());
        let mut unsorted = member.clone();
        unsorted.blockers.swap(0, 2);
        assert!(unsorted.validate().is_err());
    }

    #[test]
    fn a_member_blocker_cannot_name_another_product() {
        let member = MemberReadiness::new(id(2), id(20), vec![blocker(ReadinessOwner::Project, "missing_forecast", "products.3").for_product(id(3))]);
        assert!(member.validate().is_err());
    }

    #[test]
    fn readiness_blockers_are_grouped_by_owning_package_and_carry_stable_codes() {
        let blocker = blocker(ReadinessOwner::Engineering, "pending_classification", "parts.30.classification.confirmed").for_part(id(30));
        assert!(blocker.validate().is_ok());
        let mut invalid = blocker.clone();
        invalid.code = "Not Stable".into();
        assert!(invalid.validate().is_err());
        let value = serde_json::to_value(&blocker).unwrap_or_default();
        assert_eq!(value["owner"], json!("engineering"));
        assert_eq!(value["code"], json!("pending_classification"));
        assert_eq!(value["part_version_id"], json!(id(30).to_string()));
        assert_eq!(value["product_id"], json!(null));
    }

    #[test]
    fn a_changed_definition_names_its_aspects_in_canonical_order() {
        let accepted = definition();
        let mut current = definition();
        current.routing = None;
        current.material = None;
        assert_eq!(shared_definition_changes(&accepted, &current), vec![DefinitionChange::Material, DefinitionChange::Routing]);

        let mut current = definition();
        current.sourcing = Some(Sourcing::Purchased);
        current.material = None;
        assert_eq!(shared_definition_changes(&accepted, &current), vec![DefinitionChange::Sourcing, DefinitionChange::Material]);

        let unchanged = shared_definition_changes(&accepted, &definition());
        assert!(unchanged.is_empty());
    }

    #[test]
    fn a_changed_definition_keeps_its_affected_products_sorted_and_unique() {
        let changed = ChangedSharedDefinition::new(
            id(30),
            "PL-100",
            "B",
            vec![DefinitionChange::Routing, DefinitionChange::Sourcing, DefinitionChange::Routing],
            vec![id(3), id(2), id(3)],
        );
        assert_eq!(changed.changes, vec![DefinitionChange::Sourcing, DefinitionChange::Routing]);
        assert_eq!(changed.affected_product_ids, vec![id(2), id(3)]);
        assert!(changed.validate().is_ok());
        assert!(ChangedSharedDefinition::new(id(30), "PL-100", "B", Vec::new(), vec![id(2)]).validate().is_err());
    }

    #[test]
    fn a_coordinated_group_requires_every_affected_product() {
        let first = ChangedSharedDefinition::new(id(30), "PL-100", "B", vec![DefinitionChange::Sourcing], vec![id(2), id(3)]);
        let second = ChangedSharedDefinition::new(id(31), "PL-101", "A", vec![DefinitionChange::Material], vec![id(3), id(4)]);
        let report = CoordinationReport::new(vec![second, first]);
        assert_eq!(report.required_product_ids, vec![id(2), id(3), id(4)]);
        assert_eq!(report.changed_definitions[0].part_version_id, id(30));
        assert!(report.validate().is_ok());

        let mut wrong = report.clone();
        wrong.required_product_ids.pop();
        assert!(wrong.validate().is_err());
    }

    #[test]
    fn a_readiness_request_needs_unique_products_and_valid_ids() {
        let request = ActivationReadinessRequest { project_version: 7, selections: vec![selection(2, 20), selection(3, 21)] };
        assert!(request.validate().is_ok());
        assert!(ActivationReadinessRequest { project_version: 0, ..request.clone() }.validate().is_err());
        assert!(ActivationReadinessRequest { selections: Vec::new(), ..request.clone() }.validate().is_err());
        assert!(ActivationReadinessRequest { selections: vec![selection(2, 20), selection(2, 22)], ..request }.validate().is_err());
    }

    #[test]
    fn a_readiness_report_names_the_versions_it_read_and_its_required_products() {
        let changed = ChangedSharedDefinition::new(id(30), "PL-100", "B", vec![DefinitionChange::Sourcing], vec![id(2), id(3)]);
        let report = ActivationReadinessReport {
            project_id: id(10),
            project_version: 7,
            accepted_baseline_id: Some(id(90)),
            members: vec![
                MemberReadiness::new(id(2), id(20), Vec::new()),
                MemberReadiness::new(id(3), id(21), vec![blocker(ReadinessOwner::Costing, "missing_purchase_price", "parts.30.purchase")]),
            ],
            coordination: CoordinationReport::new(vec![changed]),
        };
        assert!(report.validate().is_ok());
        let mut unsorted = report.clone();
        unsorted.members.reverse();
        assert!(unsorted.validate().is_err());
        assert!(ActivationReadinessReport { project_version: 0, ..report }.validate().is_err());
    }

    #[test]
    fn an_activation_request_carries_the_expected_versions_and_unique_selections() {
        let request = ActivateRevisions {
            request_id: "activation-1".into(),
            expected_project_version: 7,
            expected_accepted_baseline_id: Some(id(90)),
            product_revisions: vec![selection(2, 20)],
        };
        assert!(request.validate().is_ok());
        assert!(ActivateRevisions { request_id: " ".into(), ..request.clone() }.validate().is_err());
        assert!(ActivateRevisions { expected_project_version: 0, ..request.clone() }.validate().is_err());
        assert!(ActivateRevisions { expected_accepted_baseline_id: Some(Uuid::nil()), ..request.clone() }.validate().is_err());
        assert!(ActivateRevisions { product_revisions: Vec::new(), ..request }.validate().is_err());
    }

    #[test]
    fn a_member_is_either_activated_from_a_draft_or_repinned_from_its_active_revision() {
        assert!(activated(2, 20, Some(19)).validate().is_ok());
        assert!(repinned(2, 20).validate().is_ok());
        assert_eq!(serde_json::to_value(repinned(2, 20)).unwrap_or_default()["action"], json!("repinned"));

        // A re-pin keeps the active revision; nothing becomes historical.
        let inconsistent = ActivationMember { previous_active_revision_id: Some(id(19)), ..repinned(2, 20) };
        assert!(inconsistent.validate().is_err());
        // The accepted revision cannot replace itself.
        assert!(ActivationMember { previous_active_revision_id: Some(id(20)), ..activated(2, 20, Some(19)) }.validate().is_err());

        // Both actions reach SB-20 as facts.
        assert!(ActivationFact::RevisionRepinned { product_id: id(2), product_revision_id: id(20) }.validate().is_ok());
        assert!(
            ActivationFact::RevisionActivated { product_id: id(2), product_revision_id: id(20), previous_active_revision_id: Some(id(19)) }
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn an_inactivatable_revision_is_refused_with_its_own_code() {
        let refusal = ActivationRefusal::RevisionNotActivatable { product_id: id(2), product_revision_id: id(19) };
        assert_eq!(refusal.code(), "revision_not_activatable");
        assert_eq!(refusal.http_status(), 409);
        assert!(refusal.message().starts_with("revision_not_activatable: "));
        assert!(refusal.validate().is_ok());
    }

    #[test]
    fn an_activation_receipt_pins_the_record_versions_it_read() {
        let receipt = ActivationReceipt {
            request_id: "activation-1".into(),
            project_id: id(10),
            pinned: pinned(),
            accepted_baseline_id: id(90),
            costing_calculation_id: id(91),
            costing_input_hash: "a".repeat(64),
            calculator_version: "solidbom-manufacturing-baseline/1".into(),
            costing_state: ResultState::Incomplete,
            members: vec![activated(2, 20, None)],
            facts: vec![ActivationFact::BaselineAccepted { accepted_baseline_id: id(90), previous_accepted_baseline_id: None, project_version: 7 }],
        };
        assert_eq!(receipt.project_version(), 7);
        assert!(receipt.validate().is_ok());
        assert!(ActivationReceipt { costing_input_hash: "A".repeat(64), ..receipt.clone() }.validate().is_err());
        assert!(ActivationReceipt { facts: Vec::new(), ..receipt }.validate().is_err());
    }

    #[test]
    fn an_accepted_baseline_round_trips_with_its_pins_and_members() {
        let baseline = accepted_baseline();
        assert!(baseline.validate().is_ok());
        let text = serde_json::to_string(&baseline).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        // Business decimals are strings and the enum wire names are snake_case.
        assert_eq!(value["shared_definitions"][0]["material"]["quantity"], json!("120"));
        assert_eq!(value["costing_state"], json!("complete"));
        let parsed: AcceptedBaseline = serde_json::from_value(value).unwrap();
        assert_eq!(parsed, baseline);
    }

    #[test]
    fn an_accepted_baseline_validates_its_scopes_order_and_identity() {
        let baseline = accepted_baseline();
        assert!(AcceptedBaseline { members: Vec::new(), ..baseline.clone() }.validate().is_err());
        // A ready accepted state may use no shared part version at all (for example an
        // excluded-only activation); an empty full definition set is a valid record.
        assert!(AcceptedBaseline { shared_definitions: Vec::new(), ..baseline.clone() }.validate().is_ok());
        assert!(AcceptedBaseline { sequence: 0, ..baseline.clone() }.validate().is_err());
        assert!(AcceptedBaseline { costing_input_hash: "z".repeat(64), ..baseline.clone() }.validate().is_err());

        let mut unsorted = baseline.clone();
        unsorted.active_revisions = vec![selection(3, 21), selection(2, 20)];
        assert!(unsorted.validate().is_err());

        let mut unsorted_pins = baseline;
        unsorted_pins.shared_definitions = vec![definition(), definition()];
        assert!(unsorted_pins.validate().is_err());
    }

    #[test]
    fn a_purchased_part_pin_never_carries_manufacturing_material() {
        let mut pin = definition();
        pin.sourcing = Some(Sourcing::Purchased);
        assert!(pin.clone().validate().is_err());
        pin.material = None;
        assert!(pin.validate().is_ok());
    }

    #[test]
    fn a_shared_definition_pin_compares_its_semantic_shape() {
        let current = definition();
        let mut accepted = definition();
        accepted.routing = Some(SharedRoutingDefinition { operations: routing().operations[..1].to_vec() });
        assert_eq!(shared_definition_changes(&accepted, &current), vec![DefinitionChange::Routing]);
        // Re-saving the same shape, with another routing identity nowhere in the pin, is unchanged.
        assert!(shared_definition_changes(&current, &definition()).is_empty());
    }

    #[test]
    fn a_stale_expected_version_is_a_conflict_and_a_bad_request_is_invalid() {
        let refusal = ActivationRefusal::StaleProjectVersion { expected: 6, current: 7 };
        assert_eq!(refusal.http_status(), 409);
        assert!(refusal.message().starts_with("stale_project_version: "));
        assert!(refusal.validate().is_ok());

        let refusal = ActivationRefusal::InvalidRequest { reason: "the project has no such revision".into() };
        assert_eq!(refusal.http_status(), 422);
        assert!(refusal.message().starts_with("invalid_request: "));

        let refusal = ActivationRefusal::StaleAcceptedBaseline { expected: None, current: Some(id(90)) };
        assert_eq!(refusal.http_status(), 409);
        assert!(refusal.message().starts_with("stale_accepted_baseline: "));
        assert!(refusal.message().contains(&id(90).to_string()));
    }

    #[test]
    fn a_coordinated_refusal_names_every_affected_product_and_a_member_refusal_names_its_blockers() {
        let refusal = ActivationRefusal::CoordinatedActivationRequired { affected_products: vec![id(2), id(3)] };
        let message = refusal.message();
        assert!(message.starts_with("coordinated_activation_required: "));
        assert!(message.contains(&id(2).to_string()));
        assert!(message.contains(&id(3).to_string()));
        assert!(refusal.validate().is_ok());

        let refusal = ActivationRefusal::MemberNotReady {
            product_id: id(2),
            product_revision_id: id(20),
            blockers: vec![blocker(ReadinessOwner::Feasibility, "open_finding_failed", "assessments.1.findings")],
        };
        let message = refusal.message();
        assert!(message.starts_with("member_not_ready: "));
        assert!(message.contains("open_finding_failed"));
        assert!(message.contains(&id(2).to_string()));
        assert!(refusal.validate().is_ok());
        assert!(ActivationRefusal::MemberNotReady { product_id: id(2), product_revision_id: id(20), blockers: Vec::new() }.validate().is_err());
    }

    #[test]
    fn an_accepted_baseline_history_is_newest_first() {
        let summary = |baseline: u128, sequence: u64| AcceptedBaselineSummary {
            accepted_baseline_id: id(baseline),
            project_id: id(10),
            sequence,
            project_version: sequence + 5,
            engineering_version: 1,
            manufacturing_version: 1,
            feasibility_version: 1,
            costing_calculation_id: id(91),
            costing_state: ResultState::Complete,
            created_at: None,
        };
        let list = AcceptedBaselineList { project_id: id(10), baselines: vec![summary(2, 2), summary(1, 1)] };
        assert!(list.validate().is_ok());
        let wrong = AcceptedBaselineList { project_id: id(10), baselines: vec![summary(1, 1), summary(2, 2)] };
        assert!(wrong.validate().is_err());
        let foreign = AcceptedBaselineList { project_id: id(11), baselines: vec![summary(1, 1)] };
        assert!(foreign.validate().is_err());
    }

    #[test]
    fn a_missing_reviewed_dimension_or_material_stays_missing() {
        assert!(shared_dimension_definition(None).is_none());
        assert!(shared_material_definition(None).is_none());
        assert!(shared_routing_definition(None).is_none());

        let reviewed = ReviewedPart {
            part_version_id: id(30),
            classification: crate::engineering::ClassificationRecord::pending(),
            volume_mm3: None,
            volume_confirmed_by: None,
            catalog: Default::default(),
        };
        assert!(shared_dimension_definition(Some(&reviewed)).is_none());
        let volume = crate::types::Measurement {
            value: Some(Decimal::from(2400)),
            unit: Unit::MillimetreCubed,
            source: crate::types::MeasurementSource::Manual,
            review: ReviewState::Confirmed,
            reason: Some("manual completion".into()),
            analysis_key: crate::types::AnalysisKey {
                source_sha256: "b".repeat(64),
                engine_build: "0.40".into(),
                extractor_version: "extract/1".into(),
            },
        };
        let reviewed = ReviewedPart { volume_mm3: Some(volume), ..reviewed };
        assert_eq!(shared_dimension_definition(Some(&reviewed)), Some(SharedDimensionDefinition { volume_mm3: Some(Decimal::from(2400)) }));
    }
}
