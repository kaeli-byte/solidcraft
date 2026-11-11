//! Dependency updates and adoption: the version 1 dependency-update and adoption contract of
//! package SB-20, additive to API version 2.
//!
//! The shared reference data is versioned and append-only (materials, cost-center rates, catalog
//! components, routing templates, feasibility rule tables and the app cost configuration). A
//! project pins the exact version ids it used. When an administrator publishes a newer version of
//! a shared logical record, every project that still resolves an older version of that record has a
//! pending dependency. This module is the portable vocabulary of that surface:
//!
//! - [`SharedUpdate`] and [`SharedUpdateList`] are the read-only catalog of version-change facts:
//!   one newest published version of one shared logical record.
//! - [`ProjectDependency`] and [`ProjectDependencyReport`] are the read-only impact answer for one
//!   project: the outdated dependency, the affected part versions and products, the append-only
//!   decision history, the newest draft result's stale markers and the saved commercial scenarios
//!   that pinned an older result. Reading a report stores nothing and changes no version.
//! - [`DependencyDecisionItem`]/[`RecordDependencyDecisions`] and [`DependencyDecisionReceipt`] are
//!   the explicit decision write. An `acknowledged` decision records that the engineer saw the
//!   update; it changes nothing else and does not adopt the update. An `adopted` decision repins the
//!   project's records to the exact newly published version. Adoption never recalculates by itself;
//!   recalculation is the separate [`RecalculateDraft`] command.
//! - [`RecalculateDraft`] and [`DraftPublication`] publish one new immutable draft calculation
//!   snapshot bound to the exact project/engineering/manufacturing versions it read. A publication
//!   whose expected generation moved is a conflict and writes nothing.
//! - [`DraftResultStatus`] and [`DraftStaleReason`] mark a stored draft result against the current
//!   heads; [`FlaggedScenario`] marks a saved commercial scenario whose pinned baseline or project
//!   version is no longer the newest.
//! - [`DependencyRefusal`] names the stable refusal reasons; each maps to one version 1
//!   [`ApiErrorCode`] and prefixes the version 1 error body message.
//!
//! The rules are locked in `docs/solidbom/contracts/v2/dependency-updates-and-adoption.md`; the
//! service surface is `services/solidbom-server/src/adoption.rs` with migration
//! `0012_dependency_decisions.sql`.
//!
//! # Rules this contract keeps
//!
//! Viewing is free: the catalog, the project report and the project list are pure reads. A missing
//! value stays `None` and is never turned into a zero. Acknowledgement is not adoption and adoption
//! is not recalculation. Shared updates enter a draft calculation only after explicit adoption, and
//! a newer published version is never silently selected for a project that already resolved an
//! older one (the app configuration included). Earlier results, accepted baselines and historical
//! snapshots are never mutated. A failed adoption or a stale generation writes nothing at all.
//!
//! # Ordering and validation
//!
//! Sorted lists stay sorted: values that the service derives carry the canonical order stated per
//! type, and [`validate`](SharedUpdateList::validate) refuses an unsorted or duplicated list rather
//! than silently trusting it. Every project version, decision sequence and published-record version
//! is at least 1; an engineering or manufacturing record generation may be 0, meaning the project
//! has no such record yet. Every identity is non-nil, every text is bounded and free of control
//! characters, and hostile input is refused before it reaches a store.
//!
//! There is no desktop surface in this increment: the typed engine commands, controller state and
//! UI are a later increment of SB-20.
//!
//! [`ApiErrorCode`]: crate::api::ApiErrorCode

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::commercial::CommercialStaleReason;
use crate::costing::{BaselineCalculationSummary, ProductRevisionSelection};
use crate::error::{ContractError, Result, check_sha256};
use crate::types::{ResultState, is_stable_code};

/// The dependency-update and adoption contract version this build reads and writes.
pub const ADOPTION_CONTRACT_VERSION: u32 = 1;

/// The most version-change facts one shared-update catalog read returns.
pub const MAX_SHARED_UPDATES: usize = 500;

/// The most pending dependencies one project impact report returns.
pub const MAX_PROJECT_DEPENDENCIES: usize = 200;

/// The most part versions or products one dependency names as its affected scope.
pub const MAX_DEPENDENCY_TARGETS: usize = 500;

/// The most decisions one decision request or receipt may carry.
pub const MAX_DEPENDENCY_DECISIONS: usize = 100;

/// The most decision-history rows one project impact report returns.
pub const MAX_DECISION_HISTORY: usize = 100;

/// The most affected projects one global dependency-project list returns.
pub const MAX_AFFECTED_PROJECTS: usize = 200;

/// The most flagged commercial scenarios one report or publication returns.
pub const MAX_FLAGGED_SCENARIOS: usize = 100;

/// The longest request id a decision or recalculation request accepts.
pub const MAX_DEPENDENCY_REQUEST_ID_CHARS: usize = 128;

/// The longest identity text (a decision author) or reason code this contract carries.
const MAX_TEXT_CHARS: usize = 255;

/// The longest saved commercial scenario name a flagged-scenario row carries.
const MAX_SCENARIO_NAME_CHARS: usize = 200;

/// The longest calculator version text a draft status carries.
const MAX_CALCULATOR_VERSION_CHARS: usize = 64;

/// The longest RFC 3339 UTC timestamp text this contract carries.
const MAX_TIMESTAMP_CHARS: usize = 64;

/// The longest stale-reason code a flagged scenario carries.
const MAX_REASON_CODE_CHARS: usize = 64;

/// The longest stale-reason message a flagged scenario carries.
const MAX_REASON_MESSAGE_CHARS: usize = 1_024;

/// How many stale reasons one flagged scenario may name (two are currently defined).
const MAX_SCENARIO_STALE_REASONS: usize = 8;

// ---------------------------------------------------------------------------
// Shared-update catalog
// ---------------------------------------------------------------------------

/// The kind of shared logical record a published version belongs to. These are contract vocabulary,
/// not display text.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SharedUpdateKind {
    /// A material version (logical id is the material id).
    Material,
    /// A cost-center rate version (logical id is the cost-center id).
    CostCenterRate,
    /// A catalog component version (logical id is the catalog component id).
    CatalogComponent,
    /// A routing template version (logical id is the template id). Flagged only: the owning package
    /// SB-15 performs the routing write.
    RoutingTemplate,
    /// A feasibility rule-table version (logical id is the rule-table id). Flagged only: the owning
    /// package SB-14 re-records the assessment.
    FeasibilityRuleTable,
    /// An app cost-configuration version (the logical record is the configuration).
    AppConfig,
}

impl SharedUpdateKind {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Material => "material",
            Self::CostCenterRate => "cost_center_rate",
            Self::CatalogComponent => "catalog_component",
            Self::RoutingTemplate => "routing_template",
            Self::FeasibilityRuleTable => "feasibility_rule_table",
            Self::AppConfig => "app_config",
        }
    }

    /// Whether a decision may adopt this kind. A routing-template or feasibility-rule-table update
    /// is flagged only; the owning package performs the change and the dependency disappears once
    /// the record pins the new version.
    pub fn is_adoptable(self) -> bool {
        matches!(self, Self::Material | Self::CostCenterRate | Self::CatalogComponent | Self::AppConfig)
    }
}

/// One newest published version of one shared logical record (a version-change fact).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SharedUpdate {
    pub kind: SharedUpdateKind,
    /// The logical shared-record id (material id, cost-center id, ...).
    pub record_id: Uuid,
    /// The identity of the newest published version of that record.
    pub version_id: Uuid,
    /// The newest published version number, at least 1.
    pub version: u64,
    /// The version this one succeeded, when the record has one.
    #[serde(default)]
    pub previous_version_id: Option<Uuid>,
    /// The RFC 3339 UTC timestamp the version was published.
    #[serde(default)]
    pub published_at: Option<String>,
}

impl SharedUpdate {
    /// Validate the version-change fact.
    pub fn validate(&self) -> Result<()> {
        check_named_id(self.record_id, "shared_update.record_id")?;
        check_named_id(self.version_id, "shared_update.version_id")?;
        check_version(self.version, "shared_update.version")?;
        check_optional_named_id(self.previous_version_id, "shared_update.previous_version_id")?;
        if let Some(previous_version_id) = self.previous_version_id
            && previous_version_id == self.version_id
        {
            return Err(ContractError::invalid("shared_update.previous_version_id", "a published version cannot be its own previous version"));
        }
        check_optional_bounded_text(&self.published_at, "shared_update.published_at", MAX_TIMESTAMP_CHARS)
    }

    /// The canonical ordering key of the catalog: kind wire name, record id, version.
    fn sort_key(&self) -> (&'static str, Uuid, u64) {
        (self.kind.as_str(), self.record_id, self.version)
    }
}

/// The read-only shared-update catalog, sorted by `(kind.as_str, record_id, version)`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SharedUpdateList {
    pub updates: Vec<SharedUpdate>,
}

impl SharedUpdateList {
    /// Validate the catalog: bounded, sorted and duplicate-free.
    pub fn validate(&self) -> Result<()> {
        if self.updates.len() > MAX_SHARED_UPDATES {
            return Err(ContractError::invalid("shared_update_list.updates", format!("the catalog returns at most {MAX_SHARED_UPDATES} updates")));
        }
        let mut previous: Option<(&'static str, Uuid, u64)> = None;
        for update in &self.updates {
            update.validate()?;
            let key = update.sort_key();
            if let Some(previous) = previous
                && key <= previous
            {
                return Err(ContractError::invalid(
                    "shared_update_list.updates",
                    "the updates are sorted by (kind, record id, version) without duplicates",
                ));
            }
            previous = Some(key);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Project impact
// ---------------------------------------------------------------------------

/// One current outdated shared dependency of one project: the version the project currently
/// resolves versus the newest published version of the same logical record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectDependency {
    pub kind: SharedUpdateKind,
    pub record_id: Uuid,
    /// The version id the project currently resolves.
    pub referenced_version_id: Uuid,
    /// The version number the project currently resolves, at least 1.
    pub referenced_version: u64,
    /// The newest published version id of the same logical record.
    pub available_version_id: Uuid,
    /// The newest published version number, greater than `referenced_version`.
    pub available_version: u64,
    /// Project part versions whose definition or costed input references the record, sorted.
    pub part_version_ids: Vec<Uuid>,
    /// Quoted or excluded project products using those part versions, sorted.
    pub product_ids: Vec<Uuid>,
    /// True when an acknowledgement decision for `available_version_id` is recorded; adoption
    /// removes the dependency from this list instead of raising this flag.
    pub acknowledged: bool,
}

impl ProjectDependency {
    /// Validate the dependency and its affected scope.
    pub fn validate(&self) -> Result<()> {
        check_named_id(self.record_id, "dependency.record_id")?;
        check_named_id(self.referenced_version_id, "dependency.referenced_version_id")?;
        check_named_id(self.available_version_id, "dependency.available_version_id")?;
        check_version(self.referenced_version, "dependency.referenced_version")?;
        check_version(self.available_version, "dependency.available_version")?;
        if self.referenced_version >= self.available_version {
            return Err(ContractError::invalid(
                "dependency.available_version",
                "a listed dependency names a strictly newer available version than the referenced one",
            ));
        }
        if self.part_version_ids.len() > MAX_DEPENDENCY_TARGETS {
            return Err(ContractError::invalid(
                "dependency.part_version_ids",
                format!("a dependency names at most {MAX_DEPENDENCY_TARGETS} part versions"),
            ));
        }
        check_sorted_unique_ids(&self.part_version_ids, "dependency.part_version_ids")?;
        if self.product_ids.len() > MAX_DEPENDENCY_TARGETS {
            return Err(ContractError::invalid("dependency.product_ids", format!("a dependency names at most {MAX_DEPENDENCY_TARGETS} products")));
        }
        check_sorted_unique_ids(&self.product_ids, "dependency.product_ids")
    }

    /// The canonical ordering key of a report: kind wire name, then record id.
    fn sort_key(&self) -> (&'static str, Uuid) {
        (self.kind.as_str(), self.record_id)
    }
}

/// Whether one explicit decision acknowledges an update or adopts it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DependencyDecisionAction {
    /// The engineer saw the update; nothing else changes and no draft calculation uses it.
    Acknowledged,
    /// The project pins the named published version; adoption never recalculates by itself.
    Adopted,
}

impl DependencyDecisionAction {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Acknowledged => "acknowledged",
            Self::Adopted => "adopted",
        }
    }
}

/// One append-only decision record (acknowledgement or adoption).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DependencyDecision {
    pub decision_id: Uuid,
    pub project_id: Uuid,
    /// Strictly increasing per project, starting at 1.
    pub sequence: u64,
    pub kind: SharedUpdateKind,
    pub record_id: Uuid,
    /// The referenced version at decision time, when the project resolved one.
    #[serde(default)]
    pub from_version_id: Option<Uuid>,
    /// The published version the decision names.
    pub to_version_id: Uuid,
    pub action: DependencyDecisionAction,
    /// The authenticated identity name, filled by the service.
    pub decided_by: String,
    /// The RFC 3339 UTC timestamp the service recorded.
    pub decided_at: String,
}

impl DependencyDecision {
    /// Validate the append-only decision record.
    pub fn validate(&self) -> Result<()> {
        check_named_id(self.decision_id, "decision.decision_id")?;
        check_named_id(self.project_id, "decision.project_id")?;
        check_version(self.sequence, "decision.sequence")?;
        check_named_id(self.record_id, "decision.record_id")?;
        check_named_id(self.to_version_id, "decision.to_version_id")?;
        check_optional_named_id(self.from_version_id, "decision.from_version_id")?;
        if let Some(from_version_id) = self.from_version_id
            && from_version_id == self.to_version_id
        {
            return Err(ContractError::invalid("decision.from_version_id", "a decision cannot name the version it already resolves"));
        }
        if self.action == DependencyDecisionAction::Adopted && !self.kind.is_adoptable() {
            return Err(ContractError::invalid("decision.kind", "a flagged-only update kind is never recorded as adopted"));
        }
        check_bounded_text(&self.decided_by, "decision.decided_by", MAX_TEXT_CHARS)?;
        check_bounded_text(&self.decided_at, "decision.decided_at", MAX_TIMESTAMP_CHARS)
    }
}

/// The read-only impact answer for one project at one point in time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectDependencyReport {
    pub project_id: Uuid,
    pub project_version: u64,
    pub engineering_version: u64,
    pub manufacturing_version: u64,
    /// Current outdated dependencies, sorted by `(kind.as_str, record_id)`.
    pub dependencies: Vec<ProjectDependency>,
    /// Decision history, newest first, bounded by [`MAX_DECISION_HISTORY`].
    pub decisions: Vec<DependencyDecision>,
    /// The newest draft (or any) baseline snapshot of the project, with its stale markers.
    #[serde(default)]
    pub draft: Option<DraftResultStatus>,
    /// Saved commercial scenarios that pinned an older result, sorted by scenario id.
    pub flagged_scenarios: Vec<FlaggedScenario>,
}

impl ProjectDependencyReport {
    /// Validate the report, its canonical order and the consistency between its flags and its
    /// decision history.
    pub fn validate(&self) -> Result<()> {
        check_named_id(self.project_id, "report.project_id")?;
        check_version(self.project_version, "report.project_version")?;
        check_record_version(self.engineering_version, "report.engineering_version")?;
        check_record_version(self.manufacturing_version, "report.manufacturing_version")?;
        check_project_dependencies(&self.dependencies, "report.dependencies")?;
        if self.decisions.len() > MAX_DECISION_HISTORY {
            return Err(ContractError::invalid("report.decisions", format!("the decision history returns at most {MAX_DECISION_HISTORY} decisions")));
        }
        let mut previous_sequence: Option<u64> = None;
        for decision in &self.decisions {
            decision.validate()?;
            if decision.project_id != self.project_id {
                return Err(ContractError::invalid("report.decisions.project_id", "a decision history row names another project"));
            }
            if let Some(previous) = previous_sequence
                && decision.sequence >= previous
            {
                return Err(ContractError::invalid("report.decisions", "the decision history is newest first with unique sequences"));
            }
            previous_sequence = Some(decision.sequence);
        }
        if let Some(draft) = &self.draft {
            draft.validate()?;
        }
        check_flagged_scenarios(&self.flagged_scenarios, "report.flagged_scenarios")?;
        check_flags_against_decisions(&self.dependencies, &self.decisions)
    }
}

/// One named reason a stored draft result is no longer current.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DraftStaleReason {
    /// The project record moved past the version the draft result read.
    NewerProjectVersion,
    /// The engineering record moved past the version the draft result read.
    NewerEngineeringVersion,
    /// The manufacturing record moved past the version the draft result read.
    NewerManufacturingVersion,
    /// At least one shared update is still pending adoption.
    NewerSharedData,
}

impl DraftStaleReason {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NewerProjectVersion => "newer_project_version",
            Self::NewerEngineeringVersion => "newer_engineering_version",
            Self::NewerManufacturingVersion => "newer_manufacturing_version",
            Self::NewerSharedData => "newer_shared_data",
        }
    }
}

/// One stored project draft result and why it may no longer be current. `stale` is exactly true
/// when `stale_reasons` is not empty.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DraftResultStatus {
    pub calculation_id: Uuid,
    pub project_version: u64,
    pub engineering_version: u64,
    pub manufacturing_version: u64,
    pub state: ResultState,
    /// The canonical hash of the resolved inputs.
    pub input_hash: String,
    pub calculator_version: String,
    pub stale: bool,
    /// Sorted by `as_str`, empty exactly when `stale` is false.
    pub stale_reasons: Vec<DraftStaleReason>,
    /// The RFC 3339 UTC timestamp the result was recorded.
    #[serde(default)]
    pub created_at: Option<String>,
}

impl DraftResultStatus {
    /// Validate the draft status and the stale-marker invariant.
    pub fn validate(&self) -> Result<()> {
        check_named_id(self.calculation_id, "draft.calculation_id")?;
        check_version(self.project_version, "draft.project_version")?;
        check_record_version(self.engineering_version, "draft.engineering_version")?;
        check_record_version(self.manufacturing_version, "draft.manufacturing_version")?;
        check_sha256(&self.input_hash, "draft.input_hash")?;
        check_bounded_text(&self.calculator_version, "draft.calculator_version", MAX_CALCULATOR_VERSION_CHARS)?;
        check_draft_stale_reasons(&self.stale_reasons, "draft.stale_reasons")?;
        if self.stale == self.stale_reasons.is_empty() {
            return Err(ContractError::invalid("draft.stale", "the stale flag is exactly true when the stale reasons are not empty"));
        }
        check_optional_bounded_text(&self.created_at, "draft.created_at", MAX_TIMESTAMP_CHARS)
    }
}

/// One saved commercial scenario that pinned an older result. The reasons reuse the SB-18
/// [`CommercialStaleReason`] code strings (`scenario_baseline_outdated`, `scenario_project_outdated`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FlaggedScenario {
    pub scenario_id: Uuid,
    pub version: u64,
    pub name: String,
    /// The immutable SB-16 baseline snapshot the saved scenario version pinned.
    pub baseline_calculation_id: Uuid,
    /// The project record version that pinned baseline recorded.
    pub pinned_project_version: u64,
    /// The named reasons, sorted by code, never empty.
    pub stale_reasons: Vec<CommercialStaleReason>,
}

impl FlaggedScenario {
    /// Validate the flagged scenario and its named reasons.
    pub fn validate(&self) -> Result<()> {
        check_named_id(self.scenario_id, "flagged_scenario.scenario_id")?;
        check_version(self.version, "flagged_scenario.version")?;
        check_bounded_text(&self.name, "flagged_scenario.name", MAX_SCENARIO_NAME_CHARS)?;
        check_named_id(self.baseline_calculation_id, "flagged_scenario.baseline_calculation_id")?;
        check_version(self.pinned_project_version, "flagged_scenario.pinned_project_version")?;
        if self.stale_reasons.is_empty() {
            return Err(ContractError::invalid("flagged_scenario.stale_reasons", "a flagged scenario names at least one stale reason"));
        }
        if self.stale_reasons.len() > MAX_SCENARIO_STALE_REASONS {
            return Err(ContractError::invalid(
                "flagged_scenario.stale_reasons",
                format!("a flagged scenario names at most {MAX_SCENARIO_STALE_REASONS} stale reasons"),
            ));
        }
        let mut previous: Option<&str> = None;
        for reason in &self.stale_reasons {
            if !is_stable_code(&reason.code, MAX_REASON_CODE_CHARS) {
                return Err(ContractError::invalid("flagged_scenario.stale_reasons.code", "a stale-reason code is a stable lower-case code"));
            }
            check_bounded_text(&reason.message, "flagged_scenario.stale_reasons.message", MAX_REASON_MESSAGE_CHARS)?;
            if let Some(previous) = previous
                && reason.code.as_str() <= previous
            {
                return Err(ContractError::invalid("flagged_scenario.stale_reasons", "the stale reasons are sorted by code without duplicates"));
            }
            previous = Some(reason.code.as_str());
        }
        Ok(())
    }
}

/// One project a shared update flags, with the pending count and the kinds involved.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImpactedProject {
    pub project_id: Uuid,
    pub project_version: u64,
    /// How many dependencies are pending for this project; at least 1 for a listed project.
    pub pending_count: u64,
    /// The update kinds involved, sorted and duplicate-free.
    pub kinds: Vec<SharedUpdateKind>,
}

impl ImpactedProject {
    /// Validate the impacted-project row.
    pub fn validate(&self) -> Result<()> {
        check_named_id(self.project_id, "impacted_project.project_id")?;
        check_version(self.project_version, "impacted_project.project_version")?;
        if self.pending_count == 0 {
            return Err(ContractError::invalid("impacted_project.pending_count", "a listed project has at least one pending dependency"));
        }
        if self.kinds.is_empty() {
            return Err(ContractError::invalid("impacted_project.kinds", "a listed project names at least one update kind"));
        }
        check_sorted_unique_kinds(&self.kinds, "impacted_project.kinds")
    }
}

/// Which projects a shared update flags, sorted by project id.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DependencyProjectList {
    pub projects: Vec<ImpactedProject>,
}

impl DependencyProjectList {
    /// Validate the bounded, sorted project list.
    pub fn validate(&self) -> Result<()> {
        if self.projects.len() > MAX_AFFECTED_PROJECTS {
            return Err(ContractError::invalid(
                "dependency_project_list.projects",
                format!("the list returns at most {MAX_AFFECTED_PROJECTS} affected projects"),
            ));
        }
        let mut previous: Option<Uuid> = None;
        for project in &self.projects {
            project.validate()?;
            if let Some(previous) = previous
                && project.project_id <= previous
            {
                return Err(ContractError::invalid("dependency_project_list.projects", "the projects are sorted by project id without duplicates"));
            }
            previous = Some(project.project_id);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Explicit decisions
// ---------------------------------------------------------------------------

/// One explicit decision request item. `version_id` must be the newest published version of that
/// logical record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DependencyDecisionItem {
    pub kind: SharedUpdateKind,
    pub record_id: Uuid,
    pub version_id: Uuid,
}

impl DependencyDecisionItem {
    /// Validate the item's identities.
    pub fn validate(&self) -> Result<()> {
        check_named_id(self.record_id, "decision_item.record_id")?;
        check_named_id(self.version_id, "decision_item.version_id")
    }
}

/// One explicit decision request. Items are unique by `(kind, record_id)`; the request order is
/// kept on the wire and the receipt answers in canonical order.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecordDependencyDecisions {
    pub request_id: String,
    /// The current project record version the request was prepared against.
    pub expected_version: u64,
    pub action: DependencyDecisionAction,
    /// At least one item, bounded, unique by `(kind, record_id)`, in request order.
    pub decisions: Vec<DependencyDecisionItem>,
}

impl RecordDependencyDecisions {
    /// Validate the request envelope and its decision items.
    pub fn validate(&self) -> Result<()> {
        check_request_id(&self.request_id, "record_dependency_decisions.request_id")?;
        check_version(self.expected_version, "record_dependency_decisions.expected_version")?;
        if self.decisions.is_empty() {
            return Err(ContractError::invalid("record_dependency_decisions.decisions", "a decision request names at least one update"));
        }
        if self.decisions.len() > MAX_DEPENDENCY_DECISIONS {
            return Err(ContractError::invalid(
                "record_dependency_decisions.decisions",
                format!("a decision request names at most {MAX_DEPENDENCY_DECISIONS} updates"),
            ));
        }
        let mut keys: Vec<(&'static str, Uuid)> = Vec::with_capacity(self.decisions.len());
        for item in &self.decisions {
            item.validate()?;
            let key = (item.kind.as_str(), item.record_id);
            if keys.contains(&key) {
                return Err(ContractError::invalid(
                    "record_dependency_decisions.decisions",
                    "a decision request names one kind and record at most once",
                ));
            }
            keys.push(key);
        }
        Ok(())
    }
}

/// One dependency fact an explicit decision emits. Facts name what changed; they never rewrite an
/// accepted result. The discriminator is `fact`, because each variant already names the shared-update
/// `kind` it changed.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "fact", rename_all = "snake_case")]
pub enum DependencyFact {
    /// The engineer saw the update; no record was repinned.
    DependencyAcknowledged { kind: SharedUpdateKind, record_id: Uuid, version_id: Uuid },
    /// The project repinned the shared record to the named published version.
    DependencyAdopted {
        kind: SharedUpdateKind,
        record_id: Uuid,
        #[serde(default)]
        previous_version_id: Option<Uuid>,
        version_id: Uuid,
    },
}

impl DependencyFact {
    /// Validate the fact's identities.
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::DependencyAcknowledged { record_id, version_id, .. } => {
                check_named_id(*record_id, "fact.record_id")?;
                check_named_id(*version_id, "fact.version_id")
            }
            Self::DependencyAdopted { kind, record_id, previous_version_id, version_id } => {
                check_named_id(*record_id, "fact.record_id")?;
                check_named_id(*version_id, "fact.version_id")?;
                check_optional_named_id(*previous_version_id, "fact.previous_version_id")?;
                if let Some(previous_version_id) = previous_version_id
                    && previous_version_id == version_id
                {
                    return Err(ContractError::invalid("fact.previous_version_id", "an adopted fact cannot name the version it already resolves"));
                }
                if !kind.is_adoptable() {
                    return Err(ContractError::invalid("fact.kind", "a flagged-only update kind is never adopted"));
                }
                Ok(())
            }
        }
    }
}

/// The committed answer to one explicit decision request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DependencyDecisionReceipt {
    pub request_id: String,
    pub project_id: Uuid,
    /// The project record version the decisions were checked at.
    pub project_version: u64,
    /// The new decision rows in canonical order: `(kind.as_str, record_id)`.
    pub decisions: Vec<DependencyDecision>,
    /// One fact per decision, in the same canonical order.
    pub facts: Vec<DependencyFact>,
}

impl DependencyDecisionReceipt {
    /// Validate the receipt: canonical order, unique rows and one matching fact per decision.
    pub fn validate(&self) -> Result<()> {
        check_request_id(&self.request_id, "dependency_decision_receipt.request_id")?;
        check_named_id(self.project_id, "dependency_decision_receipt.project_id")?;
        check_version(self.project_version, "dependency_decision_receipt.project_version")?;
        if self.decisions.is_empty() {
            return Err(ContractError::invalid("dependency_decision_receipt.decisions", "a decision receipt records at least one decision"));
        }
        if self.decisions.len() > MAX_DEPENDENCY_DECISIONS {
            return Err(ContractError::invalid(
                "dependency_decision_receipt.decisions",
                format!("a decision receipt records at most {MAX_DEPENDENCY_DECISIONS} decisions"),
            ));
        }
        let mut previous_key: Option<(&'static str, Uuid)> = None;
        let mut sequences: Vec<u64> = Vec::with_capacity(self.decisions.len());
        for decision in &self.decisions {
            decision.validate()?;
            if decision.project_id != self.project_id {
                return Err(ContractError::invalid("dependency_decision_receipt.decisions.project_id", "a receipt decision names another project"));
            }
            if sequences.contains(&decision.sequence) {
                return Err(ContractError::invalid("dependency_decision_receipt.decisions", "two receipt decisions share one sequence"));
            }
            sequences.push(decision.sequence);
            let key = (decision.kind.as_str(), decision.record_id);
            if let Some(previous) = previous_key
                && key <= previous
            {
                return Err(ContractError::invalid(
                    "dependency_decision_receipt.decisions",
                    "the decisions are in canonical (kind, record id) order without duplicates",
                ));
            }
            previous_key = Some(key);
        }
        check_receipt_facts(&self.facts, &self.decisions)
    }
}

// ---------------------------------------------------------------------------
// Draft publication
// ---------------------------------------------------------------------------

/// Explicit draft publication. All three expected versions are mandatory: a publication must name
/// the exact generation it observed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecalculateDraft {
    pub request_id: String,
    pub expected_project_version: u64,
    pub expected_engineering_version: u64,
    pub expected_manufacturing_version: u64,
    /// One explicit revision per product, ascending by product id; empty means the default draft
    /// rule (each product's newest draft revision, else its active revision).
    pub product_revisions: Vec<ProductRevisionSelection>,
}

impl RecalculateDraft {
    /// Validate the publication request: bounded text, non-zero generations and ascending unique
    /// revision selections.
    pub fn validate(&self) -> Result<()> {
        check_request_id(&self.request_id, "recalculate_draft.request_id")?;
        check_version(self.expected_project_version, "recalculate_draft.expected_project_version")?;
        check_record_version(self.expected_engineering_version, "recalculate_draft.expected_engineering_version")?;
        check_record_version(self.expected_manufacturing_version, "recalculate_draft.expected_manufacturing_version")?;
        if self.product_revisions.len() > crate::costing::MAX_BASELINE_PRODUCTS {
            return Err(ContractError::invalid(
                "recalculate_draft.product_revisions",
                format!("a publication names at most {} product revisions", crate::costing::MAX_BASELINE_PRODUCTS),
            ));
        }
        let mut previous: Option<Uuid> = None;
        for selection in &self.product_revisions {
            check_named_id(selection.product_id, "recalculate_draft.product_revisions.product_id")?;
            check_named_id(selection.product_revision_id, "recalculate_draft.product_revisions.product_revision_id")?;
            if let Some(previous) = previous
                && selection.product_id <= previous
            {
                return Err(ContractError::invalid(
                    "recalculate_draft.product_revisions",
                    "the selections are sorted by product id without duplicates",
                ));
            }
            previous = Some(selection.product_id);
        }
        Ok(())
    }
}

/// The result of one explicit or automatic draft publication.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DraftPublication {
    /// The new immutable snapshot's summary (SB-16 type, reused).
    pub calculation: BaselineCalculationSummary,
    /// The dependencies still pending after the publication, sorted by `(kind.as_str, record_id)`.
    pub dependencies: Vec<ProjectDependency>,
    /// The saved scenarios the new result flags, sorted by scenario id.
    pub flagged_scenarios: Vec<FlaggedScenario>,
}

impl DraftPublication {
    /// Validate the publication: its snapshot summary, its sorted pending dependencies and its
    /// sorted flagged scenarios.
    pub fn validate(&self) -> Result<()> {
        check_calculation_summary(&self.calculation)?;
        check_project_dependencies(&self.dependencies, "draft_publication.dependencies")?;
        check_flagged_scenarios(&self.flagged_scenarios, "draft_publication.flagged_scenarios")
    }
}

// ---------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------

/// A stable refusal reason. [`DependencyRefusal::code`] is the stable contract name, the prefix of
/// the client-safe [`message`](DependencyRefusal::message), and [`error_code`](DependencyRefusal::error_code)
/// names the version 1 [`ApiErrorCode`](crate::api::ApiErrorCode) the service returns: `not_found`
/// (`404`) for [`UnknownUpdate`](Self::UnknownUpdate), `conflict` (`409`) for a stale or contradictory
/// state, and `invalid` (`422`) for [`UnsupportedAdoption`](Self::UnsupportedAdoption). The
/// discriminator is `refusal`, because each variant already names the shared-update `kind` it is
/// about.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "refusal", rename_all = "snake_case")]
pub enum DependencyRefusal {
    /// The named logical record or the named published version does not exist.
    UnknownUpdate { kind: SharedUpdateKind, record_id: Uuid, version_id: Uuid },
    /// The named version is no longer the newest published version of its logical record.
    StaleUpdate { kind: SharedUpdateKind, record_id: Uuid, named_version_id: Uuid, current_version_id: Uuid },
    /// A routing-template or feasibility-rule-table update is never adopted through this route.
    UnsupportedAdoption { kind: SharedUpdateKind },
    /// The project already resolves the named version, so there is nothing to decide.
    NoPendingDependency { kind: SharedUpdateKind, record_id: Uuid, version_id: Uuid },
    /// The same decision is already recorded.
    DuplicateDecision { kind: SharedUpdateKind, record_id: Uuid, version_id: Uuid, action: DependencyDecisionAction },
    /// The request's expected project record version is stale.
    StaleProjectVersion { expected: u64, current: u64 },
    /// A publication's expected project/engineering/manufacturing generation moved.
    StaleDraftGeneration { reasons: Vec<DraftStaleReason> },
}

impl DependencyRefusal {
    /// The stable refusal code that prefixes the error message.
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnknownUpdate { .. } => "unknown_update",
            Self::StaleUpdate { .. } => "stale_update",
            Self::UnsupportedAdoption { .. } => "unsupported_adoption",
            Self::NoPendingDependency { .. } => "no_pending_dependency",
            Self::DuplicateDecision { .. } => "duplicate_decision",
            Self::StaleProjectVersion { .. } => "stale_project_version",
            Self::StaleDraftGeneration { .. } => "stale_draft_generation",
        }
    }

    /// The version 1 error code the service returns for this refusal.
    pub fn error_code(&self) -> crate::api::ApiErrorCode {
        match self {
            Self::UnknownUpdate { .. } => crate::api::ApiErrorCode::NotFound,
            Self::UnsupportedAdoption { .. } => crate::api::ApiErrorCode::Invalid,
            Self::StaleUpdate { .. }
            | Self::NoPendingDependency { .. }
            | Self::DuplicateDecision { .. }
            | Self::StaleProjectVersion { .. }
            | Self::StaleDraftGeneration { .. } => crate::api::ApiErrorCode::Conflict,
        }
    }

    /// The client-safe message the version 1 error body carries: the stable code, then a colon and
    /// the detail.
    pub fn message(&self) -> String {
        let detail = match self {
            Self::UnknownUpdate { kind, record_id, version_id } => {
                format!("no published {} version {version_id} of record {record_id} is known", kind.as_str())
            }
            Self::StaleUpdate { kind, record_id, named_version_id, current_version_id } => {
                format!("{} record {record_id} has moved to version {current_version_id}, not the named {named_version_id}", kind.as_str())
            }
            Self::UnsupportedAdoption { kind } => {
                format!("a {} update is flagged only; the owning package performs the change, so it cannot be adopted here", kind.as_str())
            }
            Self::NoPendingDependency { kind, record_id, version_id } => {
                format!("the project already resolves {} record {record_id} at version {version_id}, so there is nothing to decide", kind.as_str())
            }
            Self::DuplicateDecision { kind, record_id, version_id, action } => {
                format!("a {} decision for {} record {record_id} version {version_id} is already recorded", action.as_str(), kind.as_str())
            }
            Self::StaleProjectVersion { expected, current } => format!("the project is at version {current}, not {expected}"),
            Self::StaleDraftGeneration { reasons } => {
                format!("the draft generation moved: {}", describe_draft_stale_reasons(reasons))
            }
        };
        format!("{}: {detail}", self.code())
    }

    /// Validate the refusal's values. The service builds refusals, so this also guards a coding
    /// mistake from reaching a client.
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::UnknownUpdate { record_id, version_id, .. } | Self::NoPendingDependency { record_id, version_id, .. } => {
                check_named_id(*record_id, "refusal.record_id")?;
                check_named_id(*version_id, "refusal.version_id")
            }
            Self::StaleUpdate { record_id, named_version_id, current_version_id, .. } => {
                check_named_id(*record_id, "refusal.record_id")?;
                check_named_id(*named_version_id, "refusal.named_version_id")?;
                check_named_id(*current_version_id, "refusal.current_version_id")?;
                if named_version_id == current_version_id {
                    return Err(ContractError::invalid("refusal.current_version_id", "a stale-update refusal names two different versions"));
                }
                Ok(())
            }
            Self::UnsupportedAdoption { kind } => {
                if kind.is_adoptable() {
                    return Err(ContractError::invalid("refusal.kind", "an unsupported-adoption refusal names a flagged-only update kind"));
                }
                Ok(())
            }
            Self::DuplicateDecision { record_id, version_id, .. } => {
                check_named_id(*record_id, "refusal.record_id")?;
                check_named_id(*version_id, "refusal.version_id")
            }
            Self::StaleProjectVersion { expected, current } => {
                check_version(*expected, "refusal.expected")?;
                check_version(*current, "refusal.current")?;
                if expected == current {
                    return Err(ContractError::invalid("refusal.current", "a stale-version refusal names two different versions"));
                }
                Ok(())
            }
            Self::StaleDraftGeneration { reasons } => {
                if reasons.is_empty() {
                    return Err(ContractError::invalid("refusal.reasons", "a stale-draft refusal names at least one reason"));
                }
                check_draft_stale_reasons(reasons, "refusal.reasons")
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

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

/// An engineering or manufacturing record generation. A project without that record yet reads as
/// generation `0`, which is a present value and a legal generation; every existing record version is
/// at least 1.
fn check_record_version(version: u64, field: &str) -> Result<()> {
    if version == 0 { Ok(()) } else { check_version(version, field) }
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

/// The write request-id rule: non-blank, bounded, no control characters.
fn check_request_id(request_id: &str, field: &str) -> Result<()> {
    check_bounded_text(request_id, field, MAX_DEPENDENCY_REQUEST_ID_CHARS)
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

fn check_sorted_unique_kinds(kinds: &[SharedUpdateKind], field: &str) -> Result<()> {
    let mut previous: Option<&'static str> = None;
    for kind in kinds {
        if let Some(previous) = previous
            && kind.as_str() <= previous
        {
            return Err(ContractError::invalid(field, "the update kinds are sorted by wire name without duplicates"));
        }
        previous = Some(kind.as_str());
    }
    Ok(())
}

fn check_draft_stale_reasons(reasons: &[DraftStaleReason], field: &str) -> Result<()> {
    let mut previous: Option<&'static str> = None;
    for reason in reasons {
        if let Some(previous) = previous
            && reason.as_str() <= previous
        {
            return Err(ContractError::invalid(field, "the stale reasons are sorted by wire name without duplicates"));
        }
        previous = Some(reason.as_str());
    }
    Ok(())
}

fn check_project_dependencies(dependencies: &[ProjectDependency], field: &str) -> Result<()> {
    if dependencies.len() > MAX_PROJECT_DEPENDENCIES {
        return Err(ContractError::invalid(field, format!("at most {MAX_PROJECT_DEPENDENCIES} pending dependencies are reported")));
    }
    let mut previous: Option<(&'static str, Uuid)> = None;
    for dependency in dependencies {
        dependency.validate()?;
        let key = dependency.sort_key();
        if let Some(previous) = previous
            && key <= previous
        {
            return Err(ContractError::invalid(field, "the dependencies are sorted by (kind, record id) without duplicates"));
        }
        previous = Some(key);
    }
    Ok(())
}

fn check_flagged_scenarios(scenarios: &[FlaggedScenario], field: &str) -> Result<()> {
    if scenarios.len() > MAX_FLAGGED_SCENARIOS {
        return Err(ContractError::invalid(field, format!("at most {MAX_FLAGGED_SCENARIOS} flagged scenarios are returned")));
    }
    let mut previous: Option<Uuid> = None;
    for scenario in scenarios {
        scenario.validate()?;
        if let Some(previous) = previous
            && scenario.scenario_id <= previous
        {
            return Err(ContractError::invalid(field, "the flagged scenarios are sorted by scenario id without duplicates"));
        }
        previous = Some(scenario.scenario_id);
    }
    Ok(())
}

/// One fact per decision, in the same canonical order, agreeing on the action and the versions.
fn check_receipt_facts(facts: &[DependencyFact], decisions: &[DependencyDecision]) -> Result<()> {
    if facts.len() != decisions.len() {
        return Err(ContractError::invalid("dependency_decision_receipt.facts", "a receipt carries exactly one fact per decision"));
    }
    for (fact, decision) in facts.iter().zip(decisions) {
        fact.validate()?;
        match (fact, decision.action) {
            (DependencyFact::DependencyAcknowledged { kind, record_id, version_id }, DependencyDecisionAction::Acknowledged) => {
                if *kind != decision.kind || *record_id != decision.record_id || *version_id != decision.to_version_id {
                    return Err(ContractError::invalid("dependency_decision_receipt.facts", "an acknowledged fact disagrees with its decision"));
                }
            }
            (DependencyFact::DependencyAdopted { kind, record_id, previous_version_id, version_id }, DependencyDecisionAction::Adopted) => {
                if *kind != decision.kind
                    || *record_id != decision.record_id
                    || *version_id != decision.to_version_id
                    || *previous_version_id != decision.from_version_id
                {
                    return Err(ContractError::invalid("dependency_decision_receipt.facts", "an adopted fact disagrees with its decision"));
                }
            }
            _ => {
                return Err(ContractError::invalid("dependency_decision_receipt.facts", "a fact's action disagrees with its decision"));
            }
        }
    }
    Ok(())
}

/// An acknowledgement flag and a pending dependency never contradict the decision history the same
/// answer carries: an adopted decision for the available version removes the dependency, and an
/// acknowledgement decision for it raises the flag.
fn check_flags_against_decisions(dependencies: &[ProjectDependency], decisions: &[DependencyDecision]) -> Result<()> {
    for dependency in dependencies {
        for decision in decisions {
            if decision.kind != dependency.kind
                || decision.record_id != dependency.record_id
                || decision.to_version_id != dependency.available_version_id
            {
                continue;
            }
            match decision.action {
                DependencyDecisionAction::Adopted => {
                    return Err(ContractError::invalid(
                        "report.dependencies",
                        "a listed dependency names a version that an adopted decision already records",
                    ));
                }
                DependencyDecisionAction::Acknowledged if !dependency.acknowledged => {
                    return Err(ContractError::invalid(
                        "report.dependencies.acknowledged",
                        "a dependency with a recorded acknowledgement decision is flagged acknowledged",
                    ));
                }
                DependencyDecisionAction::Acknowledged => {}
            }
        }
    }
    Ok(())
}

fn check_calculation_summary(calculation: &BaselineCalculationSummary) -> Result<()> {
    check_named_id(calculation.calculation_id, "draft_publication.calculation.calculation_id")?;
    check_named_id(calculation.project_id, "draft_publication.calculation.project_id")?;
    check_version(calculation.project_version, "draft_publication.calculation.project_version")?;
    check_record_version(calculation.engineering_version, "draft_publication.calculation.engineering_version")?;
    check_record_version(calculation.manufacturing_version, "draft_publication.calculation.manufacturing_version")?;
    check_bounded_text(&calculation.calculator_version, "draft_publication.calculation.calculator_version", MAX_CALCULATOR_VERSION_CHARS)?;
    check_optional_bounded_text(&calculation.created_at, "draft_publication.calculation.created_at", MAX_TIMESTAMP_CHARS)
}

fn describe_draft_stale_reasons(reasons: &[DraftStaleReason]) -> String {
    reasons.iter().map(|reason| reason.as_str()).collect::<Vec<&str>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ApiErrorCode;
    use crate::projects::{ProjectReceipt, ProjectWriteOutcome};
    use serde_json::json;

    fn id(value: u128) -> Uuid {
        Uuid::from_u128(value)
    }

    fn update(kind: SharedUpdateKind, record: u128, version: u64) -> SharedUpdate {
        SharedUpdate {
            kind,
            record_id: id(record),
            version_id: id(record * 100 + version as u128),
            version,
            previous_version_id: None,
            published_at: Some("2026-11-11T00:00:00.000Z".into()),
        }
    }

    fn dependency() -> ProjectDependency {
        ProjectDependency {
            kind: SharedUpdateKind::CatalogComponent,
            record_id: id(40),
            referenced_version_id: id(41),
            referenced_version: 1,
            available_version_id: id(42),
            available_version: 2,
            part_version_ids: vec![id(30), id(31)],
            product_ids: vec![id(2), id(3)],
            acknowledged: false,
        }
    }

    fn decision(sequence: u64, kind: SharedUpdateKind, action: DependencyDecisionAction, from: Option<u128>, to: u128) -> DependencyDecision {
        DependencyDecision {
            decision_id: id(1_000 + sequence as u128),
            project_id: id(10),
            sequence,
            kind,
            record_id: id(40),
            from_version_id: from.map(id),
            to_version_id: id(to),
            action,
            decided_by: "engineer".into(),
            decided_at: "2026-11-11T00:00:00.000Z".into(),
        }
    }

    fn draft() -> DraftResultStatus {
        DraftResultStatus {
            calculation_id: id(91),
            project_version: 6,
            engineering_version: 3,
            manufacturing_version: 5,
            state: ResultState::Complete,
            input_hash: "a".repeat(64),
            calculator_version: "solidbom-manufacturing-baseline/1".into(),
            stale: true,
            stale_reasons: vec![DraftStaleReason::NewerProjectVersion],
            created_at: Some("2026-11-11T00:00:00.000Z".into()),
        }
    }

    fn flagged() -> FlaggedScenario {
        FlaggedScenario {
            scenario_id: id(70),
            version: 2,
            name: "negotiated".into(),
            baseline_calculation_id: id(91),
            pinned_project_version: 6,
            stale_reasons: vec![CommercialStaleReason::new("scenario_baseline_outdated", "the saved scenario pinned an older baseline")],
        }
    }

    fn report() -> ProjectDependencyReport {
        ProjectDependencyReport {
            project_id: id(10),
            project_version: 7,
            engineering_version: 3,
            manufacturing_version: 5,
            dependencies: vec![ProjectDependency { acknowledged: true, ..dependency() }],
            decisions: vec![
                decision(2, SharedUpdateKind::CatalogComponent, DependencyDecisionAction::Adopted, Some(41), 43),
                decision(1, SharedUpdateKind::CatalogComponent, DependencyDecisionAction::Acknowledged, Some(41), 42),
            ],
            draft: Some(draft()),
            flagged_scenarios: vec![flagged()],
        }
    }

    fn item(kind: SharedUpdateKind, record: u128, version: u128) -> DependencyDecisionItem {
        DependencyDecisionItem { kind, record_id: id(record), version_id: id(version) }
    }

    fn request() -> RecordDependencyDecisions {
        RecordDependencyDecisions {
            request_id: "decisions-1".into(),
            expected_version: 7,
            action: DependencyDecisionAction::Adopted,
            decisions: vec![item(SharedUpdateKind::CatalogComponent, 40, 42)],
        }
    }

    fn receipt() -> DependencyDecisionReceipt {
        DependencyDecisionReceipt {
            request_id: "decisions-1".into(),
            project_id: id(10),
            project_version: 7,
            decisions: vec![decision(1, SharedUpdateKind::CatalogComponent, DependencyDecisionAction::Adopted, Some(41), 42)],
            facts: vec![DependencyFact::DependencyAdopted {
                kind: SharedUpdateKind::CatalogComponent,
                record_id: id(40),
                previous_version_id: Some(id(41)),
                version_id: id(42),
            }],
        }
    }

    fn publication() -> DraftPublication {
        DraftPublication {
            calculation: BaselineCalculationSummary {
                calculation_id: id(92),
                project_id: id(10),
                project_version: 7,
                engineering_version: 3,
                manufacturing_version: 5,
                calculator_version: "solidbom-manufacturing-baseline/1".into(),
                state: ResultState::Complete,
                created_at: Some("2026-11-11T00:00:00.000Z".into()),
            },
            dependencies: vec![dependency()],
            flagged_scenarios: vec![flagged()],
        }
    }

    #[test]
    fn only_the_four_adoptable_kinds_may_be_adopted() {
        for kind in [SharedUpdateKind::Material, SharedUpdateKind::CostCenterRate, SharedUpdateKind::CatalogComponent, SharedUpdateKind::AppConfig] {
            assert!(kind.is_adoptable(), "{}", kind.as_str());
        }
        for kind in [SharedUpdateKind::RoutingTemplate, SharedUpdateKind::FeasibilityRuleTable] {
            assert!(!kind.is_adoptable(), "{}", kind.as_str());
        }
    }

    #[test]
    fn enum_wire_names_are_stable_snake_case() {
        let kinds = [
            (SharedUpdateKind::Material, "material"),
            (SharedUpdateKind::CostCenterRate, "cost_center_rate"),
            (SharedUpdateKind::CatalogComponent, "catalog_component"),
            (SharedUpdateKind::RoutingTemplate, "routing_template"),
            (SharedUpdateKind::FeasibilityRuleTable, "feasibility_rule_table"),
            (SharedUpdateKind::AppConfig, "app_config"),
        ];
        for (kind, name) in kinds {
            assert_eq!(kind.as_str(), name);
            assert_eq!(serde_json::to_value(kind).expect("the kind serializes"), json!(name));
            let parsed: SharedUpdateKind = serde_json::from_value(json!(name)).expect("the kind parses");
            assert_eq!(parsed, kind);
        }
        for (action, name) in [(DependencyDecisionAction::Acknowledged, "acknowledged"), (DependencyDecisionAction::Adopted, "adopted")] {
            assert_eq!(action.as_str(), name);
            assert_eq!(serde_json::to_value(action).expect("the action serializes"), json!(name));
        }
        let reasons = [
            (DraftStaleReason::NewerProjectVersion, "newer_project_version"),
            (DraftStaleReason::NewerEngineeringVersion, "newer_engineering_version"),
            (DraftStaleReason::NewerManufacturingVersion, "newer_manufacturing_version"),
            (DraftStaleReason::NewerSharedData, "newer_shared_data"),
        ];
        for (reason, name) in reasons {
            assert_eq!(reason.as_str(), name);
            assert_eq!(serde_json::to_value(reason).expect("the reason serializes"), json!(name));
        }
        // Facts and refusals are internally tagged by their own discriminator, because each variant
        // already carries the shared-update `kind` it names.
        let fact = DependencyFact::DependencyAcknowledged { kind: SharedUpdateKind::AppConfig, record_id: id(60), version_id: id(61) };
        let fact = serde_json::to_value(fact).expect("the fact serializes");
        assert_eq!(fact["fact"], json!("dependency_acknowledged"));
        assert_eq!(fact["kind"], json!("app_config"));
        let refusal = DependencyRefusal::UnsupportedAdoption { kind: SharedUpdateKind::RoutingTemplate };
        assert_eq!(serde_json::to_value(refusal).expect("the refusal serializes")["refusal"], json!("unsupported_adoption"));
    }

    #[test]
    fn a_shared_update_list_round_trips_in_canonical_order() {
        let list = SharedUpdateList {
            updates: vec![
                update(SharedUpdateKind::AppConfig, 60, 2),
                update(SharedUpdateKind::Material, 50, 2),
                update(SharedUpdateKind::Material, 50, 3),
            ],
        };
        assert!(list.validate().is_ok());
        let text = serde_json::to_string(&list).expect("the catalog serializes");
        let value: serde_json::Value = serde_json::from_str(&text).expect("the catalog parses");
        assert_eq!(value["updates"][0]["kind"], json!("app_config"));
        assert_eq!(value["updates"][1]["version"], json!(2));
        let parsed: SharedUpdateList = serde_json::from_value(value).expect("the catalog round trips");
        assert_eq!(parsed, list);
    }

    #[test]
    fn a_shared_update_list_refuses_unsorted_or_duplicate_rows() {
        let unsorted = SharedUpdateList { updates: vec![update(SharedUpdateKind::Material, 50, 2), update(SharedUpdateKind::AppConfig, 60, 2)] };
        assert!(unsorted.validate().is_err());
        let duplicate = SharedUpdateList { updates: vec![update(SharedUpdateKind::Material, 50, 2), update(SharedUpdateKind::Material, 50, 2)] };
        assert!(duplicate.validate().is_err());
        let too_many =
            SharedUpdateList { updates: (1..=MAX_SHARED_UPDATES as u128).map(|record| update(SharedUpdateKind::Material, record, 2)).collect() };
        assert!(too_many.validate().is_ok());
        let mut oversized = too_many;
        oversized.updates.push(update(SharedUpdateKind::Material, 1_000, 2));
        assert!(oversized.validate().is_err());
    }

    #[test]
    fn every_shared_update_value_is_validated() {
        let valid = update(SharedUpdateKind::Material, 50, 2);
        assert!(valid.validate().is_ok());
        assert!(SharedUpdate { record_id: Uuid::nil(), ..valid.clone() }.validate().is_err());
        assert!(SharedUpdate { version_id: Uuid::nil(), ..valid.clone() }.validate().is_err());
        assert!(SharedUpdate { version: 0, ..valid.clone() }.validate().is_err());
        assert!(SharedUpdate { previous_version_id: Some(valid.version_id), ..valid.clone() }.validate().is_err());
        assert!(SharedUpdate { previous_version_id: Some(Uuid::nil()), ..valid.clone() }.validate().is_err());
        assert!(SharedUpdate { published_at: Some(" ".into()), ..valid.clone() }.validate().is_err());
        assert!(SharedUpdate { published_at: Some("a".repeat(MAX_TIMESTAMP_CHARS + 1)), ..valid }.validate().is_err());
        // A previous version is optional and the field tolerates its absence on the wire.
        let parsed: SharedUpdate = serde_json::from_value(json!({
            "kind": "material",
            "record_id": id(50),
            "version_id": id(51),
            "version": 2,
        }))
        .expect("the optional fields are defaulted");
        assert_eq!(parsed.previous_version_id, None);
        assert_eq!(parsed.published_at, None);
    }

    #[test]
    fn a_project_dependency_names_an_older_referenced_version_with_sorted_scope() {
        let dependency = dependency();
        assert!(dependency.validate().is_ok());
        assert!(ProjectDependency { referenced_version: 2, available_version: 2, ..dependency.clone() }.validate().is_err());
        assert!(ProjectDependency { referenced_version: 3, ..dependency.clone() }.validate().is_err());
        assert!(ProjectDependency { record_id: Uuid::nil(), ..dependency.clone() }.validate().is_err());
        assert!(ProjectDependency { referenced_version_id: Uuid::nil(), ..dependency.clone() }.validate().is_err());
        assert!(ProjectDependency { available_version_id: Uuid::nil(), ..dependency.clone() }.validate().is_err());
        assert!(ProjectDependency { referenced_version: 0, ..dependency.clone() }.validate().is_err());
        assert!(ProjectDependency { part_version_ids: vec![id(31), id(30)], ..dependency.clone() }.validate().is_err());
        assert!(ProjectDependency { part_version_ids: vec![id(30), id(30)], ..dependency.clone() }.validate().is_err());
        assert!(ProjectDependency { product_ids: vec![id(2), Uuid::nil()], ..dependency.clone() }.validate().is_err());
        let too_many: Vec<Uuid> = (1..=MAX_DEPENDENCY_TARGETS as u128).map(id).collect();
        assert!(ProjectDependency { part_version_ids: too_many.clone(), ..dependency.clone() }.validate().is_ok());
        let mut oversized: Vec<Uuid> = too_many;
        oversized.push(id(MAX_DEPENDENCY_TARGETS as u128 + 1));
        assert!(ProjectDependency { part_version_ids: oversized, ..dependency }.validate().is_err());
    }

    #[test]
    fn a_project_report_round_trips_with_its_history_draft_and_flags() {
        let report = report();
        assert!(report.validate().is_ok());
        let text = serde_json::to_string(&report).expect("the report serializes");
        let value: serde_json::Value = serde_json::from_str(&text).expect("the report parses");
        assert_eq!(value["dependencies"][0]["kind"], json!("catalog_component"));
        assert_eq!(value["decisions"][0]["action"], json!("adopted"));
        assert_eq!(value["draft"]["stale"], json!(true));
        assert_eq!(value["draft"]["stale_reasons"], json!(["newer_project_version"]));
        assert_eq!(value["flagged_scenarios"][0]["stale_reasons"][0]["code"], json!("scenario_baseline_outdated"));
        let parsed: ProjectDependencyReport = serde_json::from_value(value).expect("the report round trips");
        assert_eq!(parsed, report);
    }

    #[test]
    fn a_project_report_refuses_a_non_canonical_order_or_a_foreign_decision() {
        let report = report();
        assert!(ProjectDependencyReport { project_id: Uuid::nil(), ..report.clone() }.validate().is_err());
        assert!(ProjectDependencyReport { project_version: 0, ..report.clone() }.validate().is_err());
        // Generation 0 is a project that has no such record yet, not a malformed generation.
        assert!(ProjectDependencyReport { engineering_version: 0, ..report.clone() }.validate().is_ok());
        assert!(ProjectDependencyReport { manufacturing_version: 0, ..report.clone() }.validate().is_ok());

        let mut unsorted_dependencies = report.clone();
        unsorted_dependencies.dependencies =
            vec![dependency(), ProjectDependency { kind: SharedUpdateKind::AppConfig, record_id: id(60), ..dependency() }];
        assert!(unsorted_dependencies.validate().is_err());

        let mut older_first = report.clone();
        older_first.decisions.swap(0, 1);
        assert!(older_first.validate().is_err());

        let mut duplicate_sequence = report.clone();
        duplicate_sequence.decisions[0] = DependencyDecision { sequence: 1, ..duplicate_sequence.decisions[1].clone() };
        assert!(duplicate_sequence.validate().is_err());

        let mut foreign = report.clone();
        foreign.decisions[0].project_id = id(11);
        assert!(foreign.validate().is_err());

        let mut unsorted_flags = report.clone();
        unsorted_flags.flagged_scenarios = vec![flagged(), FlaggedScenario { scenario_id: id(69), ..flagged() }];
        assert!(unsorted_flags.validate().is_err());

        let mut duplicate_flags = report.clone();
        duplicate_flags.flagged_scenarios = vec![flagged(), flagged()];
        assert!(duplicate_flags.validate().is_err());

        let mut no_draft = report;
        no_draft.draft = None;
        assert!(no_draft.validate().is_ok());
    }

    #[test]
    fn an_acknowledgement_flag_never_contradicts_the_decision_history() {
        // The adopted decision names a version the report still lists as pending: a contradiction.
        let mut contradiction = report();
        contradiction.dependencies = vec![ProjectDependency { available_version_id: id(42), ..dependency() }];
        contradiction.decisions = vec![decision(1, SharedUpdateKind::CatalogComponent, DependencyDecisionAction::Adopted, Some(41), 42)];
        assert!(contradiction.validate().is_err());
        // The acknowledgement decision exists, so the flag must be raised.
        let mut unraised = report();
        unraised.dependencies = vec![dependency()];
        unraised.decisions = vec![decision(1, SharedUpdateKind::CatalogComponent, DependencyDecisionAction::Acknowledged, Some(41), 42)];
        assert!(unraised.validate().is_err());
        unraised.dependencies[0].acknowledged = true;
        assert!(unraised.validate().is_ok());
        // A decision for another record never contradicts the listed dependency.
        let mut unrelated = report();
        unrelated.decisions = vec![DependencyDecision {
            record_id: id(41),
            ..decision(1, SharedUpdateKind::CatalogComponent, DependencyDecisionAction::Adopted, Some(41), 42)
        }];
        assert!(unrelated.validate().is_ok());
    }

    #[test]
    fn a_draft_status_is_stale_exactly_when_it_names_reasons() {
        let draft = draft();
        assert!(draft.validate().is_ok());
        let mut stale_without_reason = draft.clone();
        stale_without_reason.stale_reasons = Vec::new();
        assert!(stale_without_reason.validate().is_err());
        let mut reason_without_stale = draft.clone();
        reason_without_stale.stale = false;
        assert!(reason_without_stale.validate().is_err());
        let clean = DraftResultStatus { stale: false, stale_reasons: Vec::new(), ..draft };
        assert!(clean.validate().is_ok());
    }

    #[test]
    fn a_draft_status_validates_its_generation_hash_and_reason_order() {
        let draft = draft();
        assert!(DraftResultStatus { calculation_id: Uuid::nil(), ..draft.clone() }.validate().is_err());
        assert!(DraftResultStatus { project_version: 0, ..draft.clone() }.validate().is_err());
        assert!(DraftResultStatus { engineering_version: 0, ..draft.clone() }.validate().is_ok());
        assert!(DraftResultStatus { manufacturing_version: 0, ..draft.clone() }.validate().is_ok());
        assert!(DraftResultStatus { input_hash: "z".repeat(64), ..draft.clone() }.validate().is_err());
        assert!(DraftResultStatus { calculator_version: " ".into(), ..draft.clone() }.validate().is_err());
        assert!(DraftResultStatus { created_at: Some(" ".into()), ..draft.clone() }.validate().is_err());
        assert!(
            DraftResultStatus { stale_reasons: vec![DraftStaleReason::NewerSharedData, DraftStaleReason::NewerProjectVersion], ..draft.clone() }
                .validate()
                .is_err()
        );
        assert!(
            DraftResultStatus { stale_reasons: vec![DraftStaleReason::NewerEngineeringVersion, DraftStaleReason::NewerEngineeringVersion], ..draft }
                .validate()
                .is_err()
        );
    }

    #[test]
    fn a_flagged_scenario_always_names_sorted_stale_reasons() {
        let scenario = flagged();
        assert!(scenario.validate().is_ok());
        assert!(FlaggedScenario { scenario_id: Uuid::nil(), ..scenario.clone() }.validate().is_err());
        assert!(FlaggedScenario { version: 0, ..scenario.clone() }.validate().is_err());
        assert!(FlaggedScenario { name: " ".into(), ..scenario.clone() }.validate().is_err());
        assert!(FlaggedScenario { baseline_calculation_id: Uuid::nil(), ..scenario.clone() }.validate().is_err());
        assert!(FlaggedScenario { pinned_project_version: 0, ..scenario.clone() }.validate().is_err());
        assert!(FlaggedScenario { stale_reasons: Vec::new(), ..scenario.clone() }.validate().is_err());
        assert!(
            FlaggedScenario { stale_reasons: vec![CommercialStaleReason::new("scenario_project_outdated", "the project moved")], ..scenario.clone() }
                .validate()
                .is_ok()
        );
        assert!(
            FlaggedScenario {
                stale_reasons: vec![
                    CommercialStaleReason::new("scenario_project_outdated", "the project moved"),
                    CommercialStaleReason::new("scenario_baseline_outdated", "the baseline moved"),
                ],
                ..scenario.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            FlaggedScenario { stale_reasons: vec![CommercialStaleReason::new("Scenario Baseline", "the baseline moved")], ..scenario.clone() }
                .validate()
                .is_err()
        );
        assert!(
            FlaggedScenario { stale_reasons: vec![CommercialStaleReason::new("scenario_baseline_outdated", " ")], ..scenario }.validate().is_err()
        );
    }

    #[test]
    fn an_impacted_project_needs_a_pending_count_and_sorted_kinds() {
        let project = ImpactedProject {
            project_id: id(10),
            project_version: 7,
            pending_count: 2,
            kinds: vec![SharedUpdateKind::AppConfig, SharedUpdateKind::CatalogComponent],
        };
        assert!(project.validate().is_ok());
        assert!(ImpactedProject { project_id: Uuid::nil(), ..project.clone() }.validate().is_err());
        assert!(ImpactedProject { project_version: 0, ..project.clone() }.validate().is_err());
        assert!(ImpactedProject { pending_count: 0, ..project.clone() }.validate().is_err());
        assert!(ImpactedProject { kinds: Vec::new(), ..project.clone() }.validate().is_err());
        assert!(
            ImpactedProject { kinds: vec![SharedUpdateKind::Material, SharedUpdateKind::CatalogComponent], ..project.clone() }.validate().is_err()
        );
        assert!(ImpactedProject { kinds: vec![SharedUpdateKind::Material, SharedUpdateKind::Material], ..project }.validate().is_err());
    }

    #[test]
    fn a_dependency_project_list_is_sorted_by_project_id() {
        let list = DependencyProjectList {
            projects: vec![
                ImpactedProject { project_id: id(10), project_version: 7, pending_count: 1, kinds: vec![SharedUpdateKind::Material] },
                ImpactedProject { project_id: id(11), project_version: 2, pending_count: 3, kinds: vec![SharedUpdateKind::AppConfig] },
            ],
        };
        assert!(list.validate().is_ok());
        let unsorted = DependencyProjectList { projects: list.projects.iter().rev().cloned().collect() };
        assert!(unsorted.validate().is_err());
        let duplicate = DependencyProjectList { projects: vec![list.projects[0].clone(), list.projects[0].clone()] };
        assert!(duplicate.validate().is_err());
        let too_many = DependencyProjectList {
            projects: (1..=MAX_AFFECTED_PROJECTS as u128)
                .map(|project| ImpactedProject {
                    project_id: id(project),
                    project_version: 1,
                    pending_count: 1,
                    kinds: vec![SharedUpdateKind::Material],
                })
                .collect(),
        };
        assert!(too_many.validate().is_ok());
        let mut oversized = too_many;
        oversized.projects.push(ImpactedProject {
            project_id: id(MAX_AFFECTED_PROJECTS as u128 + 1),
            project_version: 1,
            pending_count: 1,
            kinds: vec![SharedUpdateKind::Material],
        });
        assert!(oversized.validate().is_err());
    }

    #[test]
    fn a_decision_request_keeps_request_order_and_refuses_duplicate_keys() {
        let request = request();
        assert!(request.validate().is_ok());
        let two_items = RecordDependencyDecisions {
            decisions: vec![item(SharedUpdateKind::Material, 50, 52), item(SharedUpdateKind::AppConfig, 60, 62)],
            ..request.clone()
        };
        assert!(two_items.validate().is_ok(), "the request keeps request order, not canonical order");
        let duplicate = RecordDependencyDecisions {
            decisions: vec![item(SharedUpdateKind::Material, 50, 52), item(SharedUpdateKind::Material, 50, 53)],
            ..request.clone()
        };
        assert!(duplicate.validate().is_err());
    }

    #[test]
    fn a_decision_request_refuses_a_bad_envelope_or_item() {
        let request = request();
        assert!(RecordDependencyDecisions { request_id: " ".into(), ..request.clone() }.validate().is_err());
        assert!(RecordDependencyDecisions { request_id: "a".repeat(MAX_DEPENDENCY_REQUEST_ID_CHARS + 1), ..request.clone() }.validate().is_err());
        assert!(RecordDependencyDecisions { request_id: "bad\u{7}id".into(), ..request.clone() }.validate().is_err());
        assert!(RecordDependencyDecisions { expected_version: 0, ..request.clone() }.validate().is_err());
        assert!(RecordDependencyDecisions { decisions: Vec::new(), ..request.clone() }.validate().is_err());
        let mut nil_version = request.clone();
        nil_version.decisions[0].version_id = Uuid::nil();
        assert!(nil_version.validate().is_err());
        let mut nil_record = request.clone();
        nil_record.decisions[0].record_id = Uuid::nil();
        assert!(nil_record.validate().is_err());
        let too_many = RecordDependencyDecisions {
            decisions: (1..=MAX_DEPENDENCY_DECISIONS as u128).map(|record| item(SharedUpdateKind::Material, record, record)).collect(),
            ..request
        };
        assert!(too_many.validate().is_ok());
        let mut oversized = too_many;
        oversized.decisions.push(item(SharedUpdateKind::Material, 1_000, 1_000));
        assert!(oversized.validate().is_err());
    }

    #[test]
    fn a_decision_receipt_round_trips_with_one_fact_per_decision() {
        let receipt = receipt();
        assert!(receipt.validate().is_ok());
        let text = serde_json::to_string(&receipt).expect("the receipt serializes");
        let value: serde_json::Value = serde_json::from_str(&text).expect("the receipt parses");
        assert_eq!(value["facts"][0]["fact"], json!("dependency_adopted"));
        let parsed: DependencyDecisionReceipt = serde_json::from_value(value).expect("the receipt round trips");
        assert_eq!(parsed, receipt);
    }

    #[test]
    fn a_decision_receipt_requires_canonical_order_and_one_matching_fact_per_decision() {
        let receipt = receipt();
        assert!(DependencyDecisionReceipt { request_id: " ".into(), ..receipt.clone() }.validate().is_err());
        assert!(DependencyDecisionReceipt { project_id: Uuid::nil(), ..receipt.clone() }.validate().is_err());
        assert!(DependencyDecisionReceipt { project_version: 0, ..receipt.clone() }.validate().is_err());
        assert!(DependencyDecisionReceipt { decisions: Vec::new(), ..receipt.clone() }.validate().is_err());
        assert!(DependencyDecisionReceipt { facts: Vec::new(), ..receipt.clone() }.validate().is_err());

        let mut foreign = receipt.clone();
        foreign.decisions[0].project_id = id(11);
        assert!(foreign.validate().is_err());

        let mut duplicate_sequence = receipt.clone();
        duplicate_sequence.decisions = vec![
            decision(1, SharedUpdateKind::CatalogComponent, DependencyDecisionAction::Adopted, Some(41), 42),
            DependencyDecision {
                kind: SharedUpdateKind::Material,
                ..decision(1, SharedUpdateKind::CatalogComponent, DependencyDecisionAction::Adopted, Some(41), 42)
            },
        ];
        duplicate_sequence.facts = vec![
            DependencyFact::DependencyAdopted {
                kind: SharedUpdateKind::CatalogComponent,
                record_id: id(40),
                previous_version_id: Some(id(41)),
                version_id: id(42),
            },
            DependencyFact::DependencyAdopted {
                kind: SharedUpdateKind::Material,
                record_id: id(40),
                previous_version_id: Some(id(41)),
                version_id: id(42),
            },
        ];
        assert!(duplicate_sequence.validate().is_err());

        // Two decisions in the wrong canonical order.
        let mut unsorted = receipt.clone();
        unsorted.decisions = vec![
            decision(2, SharedUpdateKind::Material, DependencyDecisionAction::Adopted, Some(51), 52),
            decision(1, SharedUpdateKind::CatalogComponent, DependencyDecisionAction::Adopted, Some(41), 42),
        ];
        unsorted.facts = vec![
            DependencyFact::DependencyAdopted {
                kind: SharedUpdateKind::Material,
                record_id: id(40),
                previous_version_id: Some(id(51)),
                version_id: id(52),
            },
            DependencyFact::DependencyAdopted {
                kind: SharedUpdateKind::CatalogComponent,
                record_id: id(40),
                previous_version_id: Some(id(41)),
                version_id: id(42),
            },
        ];
        assert!(unsorted.validate().is_err());

        // An adopted fact whose previous version disagrees with the decision.
        let mut wrong_previous = receipt.clone();
        wrong_previous.facts = vec![DependencyFact::DependencyAdopted {
            kind: SharedUpdateKind::CatalogComponent,
            record_id: id(40),
            previous_version_id: Some(id(99)),
            version_id: id(42),
        }];
        assert!(wrong_previous.validate().is_err());

        // An acknowledged fact next to an adopted decision.
        let mut wrong_action = receipt;
        wrong_action.facts =
            vec![DependencyFact::DependencyAcknowledged { kind: SharedUpdateKind::CatalogComponent, record_id: id(40), version_id: id(42) }];
        assert!(wrong_action.validate().is_err());
    }

    #[test]
    fn an_acknowledgement_receipt_carries_its_own_fact() {
        let receipt = DependencyDecisionReceipt {
            request_id: "decisions-2".into(),
            project_id: id(10),
            project_version: 7,
            decisions: vec![decision(3, SharedUpdateKind::CatalogComponent, DependencyDecisionAction::Acknowledged, Some(41), 42)],
            facts: vec![DependencyFact::DependencyAcknowledged { kind: SharedUpdateKind::CatalogComponent, record_id: id(40), version_id: id(42) }],
        };
        assert!(receipt.validate().is_ok());
        let mut adopted_fact = receipt;
        adopted_fact.facts = vec![DependencyFact::DependencyAdopted {
            kind: SharedUpdateKind::CatalogComponent,
            record_id: id(40),
            previous_version_id: Some(id(41)),
            version_id: id(42),
        }];
        assert!(adopted_fact.validate().is_err());
    }

    #[test]
    fn an_adopted_decision_never_names_a_flagged_only_kind() {
        let adopted_decision = decision(1, SharedUpdateKind::RoutingTemplate, DependencyDecisionAction::Adopted, Some(41), 42);
        assert!(adopted_decision.validate().is_err());
        let acknowledged = decision(1, SharedUpdateKind::RoutingTemplate, DependencyDecisionAction::Acknowledged, Some(41), 42);
        assert!(acknowledged.validate().is_ok());
        let fact = DependencyFact::DependencyAdopted {
            kind: SharedUpdateKind::FeasibilityRuleTable,
            record_id: id(40),
            previous_version_id: Some(id(41)),
            version_id: id(42),
        };
        assert!(fact.validate().is_err());
    }

    #[test]
    fn a_recalculation_names_its_exact_generation_and_ascending_selections() {
        let request = RecalculateDraft {
            request_id: "recalculate-1".into(),
            expected_project_version: 7,
            expected_engineering_version: 3,
            expected_manufacturing_version: 5,
            product_revisions: vec![
                ProductRevisionSelection { product_id: id(2), product_revision_id: id(20) },
                ProductRevisionSelection { product_id: id(3), product_revision_id: id(30) },
            ],
        };
        assert!(request.validate().is_ok());
        // An empty selection list means the default draft rule.
        assert!(RecalculateDraft { product_revisions: Vec::new(), ..request.clone() }.validate().is_ok());
        assert!(RecalculateDraft { request_id: " ".into(), ..request.clone() }.validate().is_err());
        assert!(RecalculateDraft { expected_project_version: 0, ..request.clone() }.validate().is_err());
        assert!(RecalculateDraft { expected_engineering_version: 0, ..request.clone() }.validate().is_ok());
        assert!(RecalculateDraft { expected_manufacturing_version: 0, ..request.clone() }.validate().is_ok());
        assert!(
            RecalculateDraft {
                product_revisions: vec![
                    ProductRevisionSelection { product_id: id(3), product_revision_id: id(30) },
                    ProductRevisionSelection { product_id: id(2), product_revision_id: id(20) },
                ],
                ..request.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            RecalculateDraft {
                product_revisions: vec![
                    ProductRevisionSelection { product_id: id(2), product_revision_id: id(20) },
                    ProductRevisionSelection { product_id: id(2), product_revision_id: id(21) },
                ],
                ..request
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn a_draft_publication_round_trips_with_its_summary_dependencies_and_flags() {
        let publication = publication();
        assert!(publication.validate().is_ok());
        let text = serde_json::to_string(&publication).expect("the publication serializes");
        let value: serde_json::Value = serde_json::from_str(&text).expect("the publication parses");
        assert_eq!(value["calculation"]["state"], json!("complete"));
        assert_eq!(value["dependencies"][0]["available_version"], json!(2));
        let parsed: DraftPublication = serde_json::from_value(value).expect("the publication round trips");
        assert_eq!(parsed, publication);
    }

    #[test]
    fn a_draft_publication_validates_its_snapshot_and_sorted_lists() {
        let publication = publication();
        assert!(
            DraftPublication {
                calculation: BaselineCalculationSummary { calculation_id: Uuid::nil(), ..publication.calculation.clone() },
                ..publication.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            DraftPublication {
                calculation: BaselineCalculationSummary { project_version: 0, ..publication.calculation.clone() },
                ..publication.clone()
            }
            .validate()
            .is_err()
        );
        let mut unsorted = publication.clone();
        unsorted.dependencies = vec![dependency(), ProjectDependency { kind: SharedUpdateKind::AppConfig, record_id: id(60), ..dependency() }];
        assert!(unsorted.validate().is_err());
        let mut unsorted_flags = publication;
        unsorted_flags.flagged_scenarios = vec![flagged(), FlaggedScenario { scenario_id: id(69), ..flagged() }];
        assert!(unsorted_flags.validate().is_err());
    }

    #[test]
    fn refusal_codes_map_to_their_version_1_error_codes() {
        let refusals = [
            (
                DependencyRefusal::UnknownUpdate { kind: SharedUpdateKind::Material, record_id: id(50), version_id: id(52) },
                "unknown_update",
                ApiErrorCode::NotFound,
            ),
            (
                DependencyRefusal::StaleUpdate {
                    kind: SharedUpdateKind::Material,
                    record_id: id(50),
                    named_version_id: id(52),
                    current_version_id: id(53),
                },
                "stale_update",
                ApiErrorCode::Conflict,
            ),
            (DependencyRefusal::UnsupportedAdoption { kind: SharedUpdateKind::RoutingTemplate }, "unsupported_adoption", ApiErrorCode::Invalid),
            (
                DependencyRefusal::NoPendingDependency { kind: SharedUpdateKind::AppConfig, record_id: id(60), version_id: id(62) },
                "no_pending_dependency",
                ApiErrorCode::Conflict,
            ),
            (
                DependencyRefusal::DuplicateDecision {
                    kind: SharedUpdateKind::Material,
                    record_id: id(50),
                    version_id: id(52),
                    action: DependencyDecisionAction::Adopted,
                },
                "duplicate_decision",
                ApiErrorCode::Conflict,
            ),
            (DependencyRefusal::StaleProjectVersion { expected: 6, current: 7 }, "stale_project_version", ApiErrorCode::Conflict),
            (
                DependencyRefusal::StaleDraftGeneration { reasons: vec![DraftStaleReason::NewerProjectVersion, DraftStaleReason::NewerSharedData] },
                "stale_draft_generation",
                ApiErrorCode::Conflict,
            ),
        ];
        for (refusal, code, error_code) in refusals {
            assert_eq!(refusal.code(), code);
            assert_eq!(refusal.error_code(), error_code);
            assert!(refusal.message().starts_with(&format!("{code}: ")));
            assert!(refusal.validate().is_ok(), "{code}");
        }
        assert!(
            DependencyRefusal::StaleUpdate {
                kind: SharedUpdateKind::Material,
                record_id: id(50),
                named_version_id: id(52),
                current_version_id: id(53),
            }
            .message()
            .contains(&id(53).to_string())
        );
        assert!(
            DependencyRefusal::StaleDraftGeneration { reasons: vec![DraftStaleReason::NewerManufacturingVersion] }
                .message()
                .contains("newer_manufacturing_version")
        );
        assert!(
            DependencyRefusal::UnknownUpdate { kind: SharedUpdateKind::Material, record_id: id(50), version_id: id(52) }
                .message()
                .contains(&id(52).to_string())
        );
    }

    #[test]
    fn a_refusal_is_validated_before_it_reaches_a_client() {
        assert!(
            DependencyRefusal::UnknownUpdate { kind: SharedUpdateKind::Material, record_id: Uuid::nil(), version_id: id(52) }.validate().is_err()
        );
        assert!(
            DependencyRefusal::StaleUpdate {
                kind: SharedUpdateKind::Material,
                record_id: id(50),
                named_version_id: id(52),
                current_version_id: id(52),
            }
            .validate()
            .is_err()
        );
        assert!(DependencyRefusal::UnsupportedAdoption { kind: SharedUpdateKind::Material }.validate().is_err());
        assert!(
            DependencyRefusal::DuplicateDecision {
                kind: SharedUpdateKind::Material,
                record_id: id(50),
                version_id: Uuid::nil(),
                action: DependencyDecisionAction::Acknowledged,
            }
            .validate()
            .is_err()
        );
        assert!(DependencyRefusal::StaleProjectVersion { expected: 7, current: 7 }.validate().is_err());
        assert!(DependencyRefusal::StaleProjectVersion { expected: 0, current: 7 }.validate().is_err());
        assert!(DependencyRefusal::StaleDraftGeneration { reasons: Vec::new() }.validate().is_err());
        assert!(
            DependencyRefusal::StaleDraftGeneration { reasons: vec![DraftStaleReason::NewerSharedData, DraftStaleReason::NewerProjectVersion] }
                .validate()
                .is_err()
        );
    }

    #[test]
    fn a_project_write_outcome_carries_an_optional_draft_publication() {
        let outcome = ProjectWriteOutcome {
            receipt: ProjectReceipt { request_id: "write-1".into(), project_id: id(10), version: 7 },
            facts: Vec::new(),
            draft: Some(publication()),
        };
        let value = serde_json::to_value(&outcome).expect("the outcome serializes");
        assert_eq!(value["draft"]["calculation"]["project_version"], json!(7));
        let parsed: ProjectWriteOutcome = serde_json::from_value(value).expect("the outcome round trips");
        assert_eq!(parsed, outcome);
        // An outcome written before SB-20 has no `draft` field and reads as None, never a missing
        // publication invented as an empty one.
        let old: ProjectWriteOutcome = serde_json::from_value(json!({
            "receipt": {"request_id": "write-1", "project_id": id(10), "version": 7},
            "facts": [],
        }))
        .expect("the additive field is defaulted");
        assert_eq!(old.draft, None);
        assert_eq!(
            serde_json::to_value(ProjectWriteOutcome { receipt: outcome.receipt, facts: Vec::new(), draft: None }).expect("the outcome serializes")["draft"],
            json!(null)
        );
    }

    #[test]
    fn unknown_json_fields_are_tolerated_and_never_promoted() {
        let mut value = serde_json::to_value(report()).expect("the report serializes");
        value["future_report_field"] = json!("tolerated");
        value["dependencies"][0]["future_dependency_field"] = json!(3);
        value["draft"]["future_draft_field"] = json!(true);
        value["flagged_scenarios"][0]["future_scenario_field"] = json!(["ignored"]);
        let parsed: ProjectDependencyReport = serde_json::from_value(value).expect("unknown fields are tolerated");
        assert!(parsed.validate().is_ok());
        let text = serde_json::to_string(&parsed).expect("the report serializes");
        assert!(!text.contains("future_report_field"), "an unknown field is never promoted into the contract");

        let mut request = serde_json::to_value(request()).expect("the request serializes");
        request["future_request_field"] = json!(true);
        let parsed: RecordDependencyDecisions = serde_json::from_value(request).expect("unknown fields are tolerated");
        assert!(parsed.validate().is_ok());

        let mut publication = serde_json::to_value(publication()).expect("the publication serializes");
        publication["future_publication_field"] = json!("tolerated");
        let parsed: DraftPublication = serde_json::from_value(publication).expect("unknown fields are tolerated");
        assert!(parsed.validate().is_ok());
    }

    #[test]
    fn a_project_without_a_record_reads_as_generation_zero() {
        // A fresh project has no engineering or manufacturing record yet; the service reads that
        // generation as 0 and an automatic publication pins 0/0. Zero is a present value, not a
        // missing one, and only that pair may be 0; every project/sequence/record version stays >= 1.
        let report = ProjectDependencyReport { engineering_version: 0, manufacturing_version: 0, ..report() };
        assert!(report.validate().is_ok());
        let draft = DraftResultStatus { engineering_version: 0, manufacturing_version: 0, ..draft() };
        assert!(draft.validate().is_ok());
        let publication = DraftPublication {
            calculation: BaselineCalculationSummary { engineering_version: 0, manufacturing_version: 0, ..publication().calculation.clone() },
            ..publication()
        };
        assert!(publication.validate().is_ok());
        let request = RecalculateDraft {
            request_id: "recalculate-1".into(),
            expected_project_version: 7,
            expected_engineering_version: 0,
            expected_manufacturing_version: 0,
            product_revisions: Vec::new(),
        };
        assert!(request.validate().is_ok());
        // The project version itself still starts at 1.
        assert!(ProjectDependencyReport { project_version: 0, ..report }.validate().is_err());
        assert!(RecalculateDraft { expected_project_version: 0, ..request }.validate().is_err());
        assert!(DraftResultStatus { project_version: 0, ..draft }.validate().is_err());
    }
}
