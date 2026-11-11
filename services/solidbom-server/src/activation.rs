//! Activation readiness, coordinated activation and accepted baselines (package SB-19, activation
//! contract version 1 additive to API version 2).
//!
//! The project record already stores `draft`/`active`/`historical` product revisions but publishes
//! no activation decision. This module publishes it:
//!
//! - **Readiness** ([`Store::activation_readiness`]) is a pure read. It resolves the exact current
//!   project, engineering, manufacturing and feasibility record versions, the registered analyses,
//!   the published reference data, the project costing and tooling settings and the accepted-baseline
//!   head, then reports one ready/blocked state per selected member with typed blockers grouped by
//!   the package that owns the issue. It appends no project version, snapshot or accepted baseline.
//! - **Activation** ([`Store::activate_revisions`]) runs one database transaction: request replay,
//!   project and accepted-baseline concurrency checks, the coordination closure, the authoritative
//!   readiness recheck, the revision state transitions, one immutable project record version, one
//!   immutable SB-16 baseline snapshot evaluated from the new version, one immutable accepted
//!   baseline row and the recorded receipt — or none of them.
//! - **Accepted baselines** ([`Store::accepted_baselines`], [`Store::accepted_baseline`]) are the
//!   append-only accepted history; the row with the largest `sequence` is the project head.
//!
//! # Coordination and the accepted definition set
//!
//! A shared part version's accepted definition has four semantic aspects (sourcing, material,
//! reviewed dimension and manufacturing routing). The portable helpers in the contract
//! ([`solidbom_contract::accepted_shared_definition`] and
//! [`solidbom_contract::shared_definition_changes`]) are the only definition of that mapping; this
//! module never re-derives it.
//!
//! Each accepted baseline carries the project's **full** accepted definition set: one pin for every
//! shared part version used by the costed structures of the quotation-included products, resolved
//! from the same `BaselineInputs`/`BaselineOutcome` the pinned SB-16 snapshot costs. A changed
//! definition therefore always has an accepted value to differ from; an activation never drops a pin
//! for a part version it did not touch.
//!
//! Readiness and activation compute the same project-wide pending-change set: every pin in the
//! accepted-baseline head compared with its current definition, and every quotation-included product
//! whose resulting costed structure uses a pending changed part. While a change is pending, any
//! activation must include those products, whether or not the request's own members use the part;
//! an omitted required product refuses the activation (`coordinated_activation_required`) and the
//! readiness route reports them in `coordination.required_product_ids`. A newly used part version,
//! and a pending pin that no resulting structure uses, need no group.
//!
//! # Readiness scope of one blocker
//!
//! The coverage mirrors the locked contract: a product-scoped issue blocks that product, a
//! part-scoped issue blocks every member whose resulting costed structure uses the part version, a
//! component- or revision-scoped issue blocks the member whose revision holds it, and an issue with
//! neither a product nor a part is a project setting that blocks every member. A quotation-excluded
//! member has no costed product line, so its structure is read from the resolved occurrence tree and
//! `no_quoted_products` does not block it. Costing issues that belong to another product never block
//! a member; the project-wide costing snapshot still reports its complete or incomplete state
//! honestly.

use std::collections::{BTreeMap, BTreeSet};

use solidbom_contract::{
    AcceptedBaseline, AcceptedBaselineList, AcceptedBaselineSummary, AcceptedSharedDefinition, ActivateRevisions, ActivationAction, ActivationFact,
    ActivationMember, ActivationReadinessReport, ActivationReadinessRequest, ActivationReceipt, ActivationRefusal, BaselineOccurrenceInput,
    BaselineOutcome, ChangedSharedDefinition, CoordinationReport, CostOwner, DefinitionChange, EngineeringRecordDocument, FeasibilityRecordDocument,
    ManufacturingIssue, ManufacturingRecordDocument, MemberReadiness, PinnedRecordVersions, ProductRevisionSelection, ProductRevisionState,
    ProjectDocument, QuotationState, ReadinessBlocker, ReadinessOwner, ResultState, ToolingOwner, ToolingRecoveryInputs, accepted_shared_definition,
    pending_shared_definition_changes, shared_definition_changes,
};
use sqlx::postgres::PgRow;
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

use crate::Result;
use crate::costing::{
    ResolvedBaseline, engineering_record, insert_baseline_snapshot, manufacturing_record, project_version_document, resolve_baseline,
    version_from_i64,
};
use crate::engineering::{RegisteredAnalysis, engineering_issues, registered_analyses};
use crate::feasibility::current_feasibility_record;
use crate::projects::{advance_version, append_document, apply_revision_activation, current_document, invalid, version_to_i64};
use crate::store::{RFC3339_UTC, Store, StoreError, check_request_id, lock_request, payload_hash, record_reply, recorded_reply};
use crate::tooling::tooling_inputs_from;

/// The prefix of the activation operations recorded in the shared request log.
const OPERATION_PREFIX: &str = "project_record.activation";

/// The project issues that need an engineer-confirmed delivered-part match. The locked readiness
/// contract groups them under the engineering owner because they are review work, not a project run.
const PENDING_PART_MATCH_CODES: [&str; 3] = ["missing_part_identity", "missing_geometry_for_match", "conflicting_geometry_match"];

/// How many accepted baselines the history read returns, newest first.
const MAX_ACCEPTED_BASELINE_HISTORY: usize = solidbom_contract::MAX_ACCEPTED_BASELINE_HISTORY;

impl Store {
    /// The readiness report for one selection at one exact project record version. This is a pure
    /// read with no request id: it appends nothing and stores nothing.
    pub async fn activation_readiness(&self, project_id: Uuid, request: &ActivationReadinessRequest) -> Result<ActivationReadinessReport> {
        request.validate().map_err(invalid)?;
        let mut tx = self.pool().begin().await?;
        // Read one consistent view: the report names one project version and the exact records it
        // read, so a concurrent write must not mix two states into one report.
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ").execute(&mut *tx).await?;
        let (current, project) = current_project(&mut tx, project_id).await?;
        // Refusal precedence: unknown identities (422), then the stale-version conflict (409), then a
        // revision that is not the product's current draft or active revision (409).
        validate_selection_identities(&project, &request.selections)?;
        if current != request.project_version {
            return refusal(ActivationRefusal::StaleProjectVersion { expected: request.project_version, current });
        }
        let selected = resolve_members(&project, &request.selections)?;
        let support = read_support(&mut tx, project_id).await?;
        let resolved = resolve_baseline(
            &mut tx,
            project_id,
            &project,
            Some(support.engineering_version),
            Some(support.manufacturing_version),
            &request.selections,
        )
        .await?;
        let tooling = tooling_issues(&project, &resolved)?;
        let mut members: Vec<MemberReadiness> = selected
            .iter()
            .map(|member| {
                let blockers = member_blockers(&project, &support, &resolved, &tooling, member.product_id, member.product_revision_id);
                MemberReadiness::new(member.product_id, member.product_revision_id, blockers)
            })
            .collect();
        members.sort_by_key(|member| member.product_id);
        let coordination = coordination_report(&project, &support, &resolved)?;
        let report = ActivationReadinessReport {
            project_id,
            project_version: current,
            accepted_baseline_id: support.head.as_ref().map(|head| head.baseline_id),
            members,
            coordination,
        };
        report.validate().map_err(StoreError::codec)?;
        tx.commit().await?;
        Ok(report)
    }

    /// Activate the selected product revisions and re-pin the current active revisions atomically,
    /// including every required member the coordinated shared-definition rule forces in. Exactly one
    /// project record version, one SB-16 baseline snapshot, one accepted baseline row and one request
    /// receipt commit together, or nothing does.
    pub async fn activate_revisions(&self, project_id: Uuid, request: &ActivateRevisions) -> Result<ActivationReceipt> {
        request.validate().map_err(invalid)?;
        check_request_id(&request.request_id)?;
        let operation = format!("{OPERATION_PREFIX}.activate");
        let payload = payload_hash(&operation, &(project_id, request))?;
        let mut tx = self.pool().begin().await?;
        lock_request(&mut tx, &request.request_id).await?;
        if let Some(receipt) = recorded_reply::<ActivationReceipt>(&mut tx, &request.request_id, &payload).await? {
            tx.commit().await?;
            return Ok(receipt);
        }
        let (current, project) = current_document(&mut tx, project_id).await?;
        // Refusal precedence: unknown identities and the request envelope (422), then the 409 order:
        // stale project version, stale accepted baseline, revision not activatable, coordination,
        // member readiness.
        validate_selection_identities(&project, &request.product_revisions)?;
        if current != request.expected_project_version {
            return refusal(ActivationRefusal::StaleProjectVersion { expected: request.expected_project_version, current });
        }
        let head = accepted_head(&mut tx, project_id).await?;
        let current_head = head.as_ref().map(|head| head.baseline_id);
        if current_head != request.expected_accepted_baseline_id {
            return refusal(ActivationRefusal::StaleAcceptedBaseline { expected: request.expected_accepted_baseline_id, current: current_head });
        }
        let selected = resolve_members(&project, &request.product_revisions)?;
        // Lock the supporting records for the rest of the transaction, so the versions the readiness
        // gate read are exactly the versions the accepted baseline pins.
        lock_support_records(&mut tx, project_id).await?;
        let support = read_support(&mut tx, project_id).await?;
        if support.engineering_version == 0 || support.manufacturing_version == 0 || support.feasibility_version == 0 {
            return refusal(ActivationRefusal::InvalidRequest {
                reason: "an accepted state needs an engineering review, a manufacturing definition and a feasibility record".into(),
            });
        }
        // Apply the member actions to the in-memory resulting document first. A re-pin keeps its
        // active revision. Nothing is written yet: every refusal below drops the transaction.
        let mut resulting = project.clone();
        let mut members = Vec::with_capacity(selected.len());
        for member in &selected {
            match member.action {
                ActivationAction::Activated => {
                    let previous = resulting
                        .product(member.product_id)
                        .and_then(|product| product.active_revision_id)
                        .filter(|previous| *previous != member.product_revision_id);
                    apply_revision_activation(&mut resulting, member.product_revision_id)?;
                    members.push(ActivationMember {
                        product_id: member.product_id,
                        product_revision_id: member.product_revision_id,
                        action: ActivationAction::Activated,
                        previous_active_revision_id: previous,
                    });
                }
                ActivationAction::Repinned => members.push(ActivationMember {
                    product_id: member.product_id,
                    product_revision_id: member.product_revision_id,
                    action: ActivationAction::Repinned,
                    previous_active_revision_id: None,
                }),
            }
        }
        members.sort_by_key(|member| member.product_id);
        resulting.validate().map_err(invalid)?;
        // Resolve the SB-16 baseline ONCE from the project version this transaction appends, with the
        // default (resulting active) revision selection. The one answer feeds the coordination check,
        // the authoritative readiness recheck, the accepted definition pins and the pinned snapshot,
        // so the gate and the snapshot cannot read different reference data.
        let resolved =
            resolve_baseline(&mut tx, project_id, &resulting, Some(support.engineering_version), Some(support.manufacturing_version), &[]).await?;
        // Every pending changed shared definition forces every quotation-included product whose
        // resulting costed structure uses it into this activation, whatever the request selected.
        let coordination = coordination_report(&resulting, &support, &resolved)?;
        let selected_products: BTreeSet<Uuid> = selected.iter().map(|member| member.product_id).collect();
        let missing: Vec<Uuid> = coordination.required_product_ids.iter().copied().filter(|id| !selected_products.contains(id)).collect();
        if !missing.is_empty() {
            return refusal(ActivationRefusal::CoordinatedActivationRequired { affected_products: missing });
        }
        // The authoritative readiness recheck. A readiness report is point-in-time; this gate is the
        // one the accepted state depends on.
        let tooling = tooling_issues(&resulting, &resolved)?;
        for member in &selected {
            let blockers = member_blockers(&resulting, &support, &resolved, &tooling, member.product_id, member.product_revision_id);
            if !blockers.is_empty() {
                return refusal(ActivationRefusal::MemberNotReady {
                    product_id: member.product_id,
                    product_revision_id: member.product_revision_id,
                    blockers,
                });
            }
        }
        // One immutable project record version, even for a pure re-pin.
        let version = current + 1;
        append_document(&mut tx, project_id, version, &resulting).await?;
        advance_version(&mut tx, project_id, version).await?;
        // One immutable SB-16 snapshot from that same single resolution. It records no second
        // request receipt.
        let snapshot = insert_baseline_snapshot(&mut tx, project_id, version, resolved).await?;
        // The full accepted definition set, taken from the same resolved structure the snapshot costs.
        let shared_definitions = accepted_pins(&resulting, &support, &snapshot.outputs)?;
        let active_revisions = resulting_active_revisions(&resulting);
        let accepted_baseline_id = Uuid::new_v4();
        let sequence = match &head {
            Some(head) => head.sequence.checked_add(1).ok_or_else(|| StoreError::Invalid("the accepted-baseline sequence is too large".into()))?,
            None => 1,
        };
        let facts = activation_facts(&members, &shared_definitions, &support, accepted_baseline_id, version);
        let accepted = AcceptedBaseline {
            accepted_baseline_id,
            project_id,
            sequence,
            request_id: request.request_id.clone(),
            pinned: PinnedRecordVersions {
                project_version: version,
                engineering_version: support.engineering_version,
                manufacturing_version: support.manufacturing_version,
                feasibility_version: support.feasibility_version,
            },
            costing_calculation_id: snapshot.calculation_id,
            costing_input_hash: snapshot.input_hash.clone(),
            calculator_version: snapshot.calculator_version.clone(),
            costing_state: snapshot.outputs.state,
            members: members.clone(),
            active_revisions,
            shared_definitions,
            created_at: None,
        };
        accepted.validate().map_err(invalid)?;
        insert_accepted_baseline(&mut tx, &accepted).await?;
        let receipt = ActivationReceipt {
            request_id: request.request_id.clone(),
            project_id,
            pinned: accepted.pinned,
            accepted_baseline_id,
            costing_calculation_id: snapshot.calculation_id,
            costing_input_hash: snapshot.input_hash.clone(),
            calculator_version: snapshot.calculator_version.clone(),
            costing_state: snapshot.outputs.state,
            members,
            facts,
        };
        receipt.validate().map_err(invalid)?;
        record_reply(&mut tx, &request.request_id, &operation, &payload, &receipt).await?;
        tx.commit().await?;
        Ok(receipt)
    }

    /// The accepted baselines of one project, newest first and bounded.
    pub async fn accepted_baselines(&self, project_id: Uuid) -> Result<AcceptedBaselineList> {
        ensure_project(self.pool(), project_id).await?;
        let limit = i64::try_from(MAX_ACCEPTED_BASELINE_HISTORY).map_err(StoreError::codec)?;
        let rows = sqlx::query(&format!(
            "SELECT accepted_baseline_id, project_id, sequence, project_version, engineering_version, manufacturing_version, \
             feasibility_version, costing_calculation_id, costing_state, to_char(created_at AT TIME ZONE 'UTC', {RFC3339_UTC}) AS created_at \
             FROM accepted_baselines WHERE project_id = $1 ORDER BY sequence DESC LIMIT $2"
        ))
        .bind(project_id)
        .bind(limit)
        .fetch_all(self.pool())
        .await?;
        let mut baselines = Vec::with_capacity(rows.len());
        for row in &rows {
            baselines.push(AcceptedBaselineSummary {
                accepted_baseline_id: row.try_get("accepted_baseline_id")?,
                project_id: row.try_get("project_id")?,
                sequence: u64::try_from(row.try_get::<i64, _>("sequence")?).map_err(StoreError::codec)?,
                project_version: u64::try_from(row.try_get::<i64, _>("project_version")?).map_err(StoreError::codec)?,
                engineering_version: u64::try_from(row.try_get::<i64, _>("engineering_version")?).map_err(StoreError::codec)?,
                manufacturing_version: u64::try_from(row.try_get::<i64, _>("manufacturing_version")?).map_err(StoreError::codec)?,
                feasibility_version: u64::try_from(row.try_get::<i64, _>("feasibility_version")?).map_err(StoreError::codec)?,
                costing_calculation_id: row.try_get("costing_calculation_id")?,
                costing_state: result_state_from_db(row.try_get("costing_state")?)?,
                created_at: Some(row.try_get("created_at")?),
            });
        }
        let list = AcceptedBaselineList { project_id, baselines };
        list.validate().map_err(StoreError::codec)?;
        Ok(list)
    }

    /// One immutable accepted baseline of one project. A baseline that does not exist, or that
    /// belongs to another project, is not found.
    pub async fn accepted_baseline(&self, project_id: Uuid, accepted_baseline_id: Uuid) -> Result<AcceptedBaseline> {
        let row = sqlx::query(&format!(
            "SELECT accepted_baseline_id, project_id, sequence, request_id, project_version, engineering_version, manufacturing_version, \
             feasibility_version, costing_calculation_id, costing_input_hash, calculator_version, costing_state, members::text AS members, \
             active_revisions::text AS active_revisions, shared_definitions::text AS shared_definitions, \
             to_char(created_at AT TIME ZONE 'UTC', {RFC3339_UTC}) AS created_at \
             FROM accepted_baselines WHERE project_id = $1 AND accepted_baseline_id = $2"
        ))
        .bind(project_id)
        .bind(accepted_baseline_id)
        .fetch_optional(self.pool())
        .await?
        .ok_or(StoreError::NotFound)?;
        let baseline = accepted_baseline_from_row(&row)?;
        baseline.validate().map_err(StoreError::codec)?;
        Ok(baseline)
    }
}

/// One selected revision resolved against the current project document: what the activation can
/// accept and what it will do to the product.
struct SelectedMember {
    product_id: Uuid,
    product_revision_id: Uuid,
    action: ActivationAction,
}

/// The supporting record versions one readiness report or activation reads. The activation holds
/// `FOR SHARE` locks on the three record heads while it uses them, so a concurrent record write
/// cannot supersede a version between the readiness gate and the accepted row.
struct Support {
    engineering_version: u64,
    engineering: EngineeringRecordDocument,
    manufacturing_version: u64,
    manufacturing: ManufacturingRecordDocument,
    feasibility_version: u64,
    feasibility: FeasibilityRecordDocument,
    analyses: Vec<RegisteredAnalysis>,
    head: Option<AcceptedHead>,
}

/// The accepted-baseline head as readiness and activation compare it.
struct AcceptedHead {
    baseline_id: Uuid,
    sequence: u64,
    shared_definitions: Vec<AcceptedSharedDefinition>,
}

/// One typed tooling readiness issue with the scope its contract assigns it.
struct ScopedToolingIssue {
    scope: BlockerScope,
    blocker: ReadinessBlocker,
}

/// The scope one readiness issue covers.
enum BlockerScope {
    Project,
    Product(Uuid),
    Part(Uuid),
}

/// One budget of typed readiness blockers, deduplicated by the contract's canonical order key so a
/// member's blocker list validates.
type BlockerSet = BTreeMap<(u8, String, String), ReadinessBlocker>;

/// Read the current project record version and document without locking it (and within the
/// readiness transaction's repeatable-read snapshot).
async fn current_project(tx: &mut Transaction<'_, Postgres>, project_id: Uuid) -> Result<(u64, ProjectDocument)> {
    let current: Option<i64> =
        sqlx::query_scalar("SELECT current_version FROM project_records WHERE project_id = $1").bind(project_id).fetch_optional(&mut **tx).await?;
    let current = match current {
        Some(current) => version_from_i64(current)?,
        None => return Err(StoreError::NotFound),
    };
    let document = project_version_document(tx, project_id, current).await?;
    Ok((current, document))
}

/// Lock the three supporting record heads for the rest of the activation transaction. Every record
/// writer takes `FOR UPDATE` on the same row before it appends, so a version read here cannot be
/// superseded before the transaction commits.
async fn lock_support_records(tx: &mut Transaction<'_, Postgres>, project_id: Uuid) -> Result<()> {
    // The table names are fixed literals, never client input.
    for table in ["engineering_records", "manufacturing_records", "feasibility_records"] {
        let statement = format!("SELECT current_version FROM {table} WHERE project_id = $1 FOR SHARE");
        sqlx::query(&statement).bind(project_id).fetch_optional(&mut **tx).await?;
    }
    Ok(())
}

/// The current engineering, manufacturing and feasibility records, the registered analyses and the
/// accepted-baseline head. A project without a review or a manufacturing definition reads as the
/// empty record at version 0, exactly like the corresponding route views.
async fn read_support(tx: &mut Transaction<'_, Postgres>, project_id: Uuid) -> Result<Support> {
    let (engineering_version, engineering) = engineering_record(tx, project_id, None).await?;
    let (manufacturing_version, manufacturing) = manufacturing_record(tx, project_id, None).await?;
    let (feasibility_version, feasibility) = current_feasibility_record(tx, project_id).await?;
    let analyses = registered_analyses(tx, project_id).await?;
    let head = accepted_head(tx, project_id).await?;
    Ok(Support { engineering_version, engineering, manufacturing_version, manufacturing, feasibility_version, feasibility, analyses, head })
}

/// The accepted-baseline head and the definition set it carries, or `None` before the first
/// activation.
async fn accepted_head(tx: &mut Transaction<'_, Postgres>, project_id: Uuid) -> Result<Option<AcceptedHead>> {
    let row = sqlx::query(
        "SELECT accepted_baseline_id, sequence, shared_definitions::text AS shared_definitions \
         FROM accepted_baselines WHERE project_id = $1 ORDER BY sequence DESC LIMIT 1",
    )
    .bind(project_id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else { return Ok(None) };
    Ok(Some(AcceptedHead {
        baseline_id: row.try_get("accepted_baseline_id")?,
        sequence: u64::try_from(row.try_get::<i64, _>("sequence")?).map_err(StoreError::codec)?,
        shared_definitions: serde_json::from_str(&row.try_get::<String, _>("shared_definitions")?).map_err(StoreError::codec)?,
    }))
}

/// The `422 invalid_request` identity checks that precede every `409`: the product must exist and
/// the revision id must belong to the project. Whether the revision is the product's current draft
/// or current active revision is the later `409 revision_not_activatable` check.
fn validate_selection_identities(project: &ProjectDocument, selections: &[ProductRevisionSelection]) -> Result<()> {
    for selection in selections {
        if project.product(selection.product_id).is_none() {
            return refusal(ActivationRefusal::InvalidRequest { reason: format!("unknown product id `{}`", selection.product_id) });
        }
        let known = project
            .products
            .iter()
            .any(|product| product.revisions.iter().any(|revision| revision.product_revision_id == selection.product_revision_id));
        if !known {
            return refusal(ActivationRefusal::InvalidRequest { reason: format!("unknown product revision id `{}`", selection.product_revision_id) });
        }
    }
    Ok(())
}

/// Resolve every selection's action against the current document. A revision that is not the
/// product's current draft or current active revision (including a foreign or historical revision)
/// is `409 revision_not_activatable`. Identity validity is the caller's earlier 422 check.
fn resolve_members(project: &ProjectDocument, selections: &[ProductRevisionSelection]) -> Result<Vec<SelectedMember>> {
    let mut members = Vec::with_capacity(selections.len());
    for selection in selections {
        let product = project
            .product(selection.product_id)
            .ok_or_else(|| refusal_error(ActivationRefusal::InvalidRequest { reason: format!("unknown product id `{}`", selection.product_id) }))?;
        let revision = product.revisions.iter().find(|revision| revision.product_revision_id == selection.product_revision_id);
        // The product's current draft is the newest revision appended to it. An older draft is
        // superseded by a later delivery and cannot be activated, so an earlier draft cannot be
        // revived while a newer one is pending.
        let newest = product.revisions.last().map(|revision| revision.product_revision_id);
        let action = match revision.map(|revision| (revision.state, product.active_revision_id)) {
            Some((ProductRevisionState::Draft, _)) if newest == Some(selection.product_revision_id) => ActivationAction::Activated,
            Some((ProductRevisionState::Active, Some(active))) if active == selection.product_revision_id => ActivationAction::Repinned,
            Some((ProductRevisionState::Draft | ProductRevisionState::Active | ProductRevisionState::Historical, _)) | None => {
                return Err(refusal_error(ActivationRefusal::RevisionNotActivatable {
                    product_id: selection.product_id,
                    product_revision_id: selection.product_revision_id,
                }));
            }
        };
        members.push(SelectedMember { product_id: selection.product_id, product_revision_id: selection.product_revision_id, action });
    }
    members.sort_by_key(|member| member.product_id);
    Ok(members)
}

/// Build one typed refusal as the store error the API maps to the contract's status and message.
fn refusal_error(refusal: ActivationRefusal) -> StoreError {
    match refusal.http_status() {
        422 => StoreError::Invalid(refusal.message()),
        _ => StoreError::Conflict(refusal.message()),
    }
}

/// Return a typed refusal from a function with a concrete `Result<T>`.
fn refusal<T>(refusal: ActivationRefusal) -> Result<T> {
    Err(refusal_error(refusal))
}

/// The coordination report for the project: every accepted-baseline head pin whose current shared
/// definition differs, and every quotation-included product whose resulting costed structure uses a
/// changed part version. The candidate set is the whole accepted state (the head pins), not the
/// request's member scope: while a definition change is pending, any activation has to include every
/// affected product, so an unrelated product-only activation cannot silently re-pin the change.
fn coordination_report(project: &ProjectDocument, support: &Support, resolved: &ResolvedBaseline) -> Result<CoordinationReport> {
    let Some(head) = &support.head else {
        return Ok(CoordinationReport::new(Vec::new()));
    };
    // The candidates are the accepted head's whole pin set, not the request's member scope: a
    // pending change must be included in any activation, so an unrelated product-only activation
    // cannot silently re-pin it.
    let current: Vec<AcceptedSharedDefinition> =
        head.shared_definitions.iter().map(|pin| current_shared_definition(project, support, pin.part_version_id)).collect::<Result<_>>()?;
    let pending = pending_shared_definition_changes(&head.shared_definitions, &current).map_err(StoreError::codec)?;
    let changed: Vec<(Uuid, Vec<DefinitionChange>)> = pending.into_iter().map(|change| (change.part_version_id, change.changes)).collect();
    let used_by_product = costed_structure_parts(resolved);
    Ok(CoordinationReport::new(report_changed_definitions(project, &changed, &used_by_product)?))
}

/// The `ChangedSharedDefinition` list for the final closure: each changed part version with every
/// quotation-included product whose resulting costed structure uses it.
fn report_changed_definitions(
    project: &ProjectDocument,
    changed: &[(Uuid, Vec<DefinitionChange>)],
    used_by_product: &BTreeMap<Uuid, BTreeSet<Uuid>>,
) -> Result<Vec<ChangedSharedDefinition>> {
    let mut definitions = Vec::with_capacity(changed.len());
    for (part_version_id, changes) in changed {
        let mut affected: Vec<Uuid> =
            used_by_product.iter().filter(|(_, parts)| parts.contains(part_version_id)).map(|(product_id, _)| *product_id).collect();
        affected.sort_unstable();
        affected.dedup();
        if affected.is_empty() {
            // Only a quotation-excluded member uses it; excluded products are outside the group.
            continue;
        }
        let part_version = project
            .part_versions
            .iter()
            .find(|part| part.part_version_id == *part_version_id)
            .ok_or_else(|| StoreError::Codec("a resolved structure names a part version the project does not hold".into()))?;
        definitions.push(ChangedSharedDefinition::new(
            *part_version_id,
            part_version.customer_part_number.clone(),
            part_version.customer_revision.clone(),
            changes.clone(),
            affected,
        ));
    }
    Ok(definitions)
}

/// The part versions the resulting costed structure of every quotation-included product uses. The
/// SB-16 outcome's product lines are exactly the costed, non-suppressed structures; a product with no
/// resulting revision has no line and no scope.
fn costed_structure_parts(resolved: &ResolvedBaseline) -> BTreeMap<Uuid, BTreeSet<Uuid>> {
    let mut used: BTreeMap<Uuid, BTreeSet<Uuid>> = BTreeMap::new();
    for line in &resolved.outputs.products {
        let parts: BTreeSet<Uuid> = line.parts.iter().map(|part| part.part_version_id).collect();
        used.insert(line.product_id, parts);
    }
    used
}

/// Collect every resolved part version of one occurrence tree.
fn collect_occurrence_parts(occurrences: &[BaselineOccurrenceInput], parts: &mut BTreeSet<Uuid>) {
    for occurrence in occurrences {
        if let Some(part_version_id) = occurrence.part_version_id {
            parts.insert(part_version_id);
        }
        collect_occurrence_parts(&occurrence.children, parts);
    }
}

/// The current accepted definition of one shared part version, mapped through the contract's own
/// helpers so the definition shape and the comparison cannot drift from the portable contract.
fn current_shared_definition(project: &ProjectDocument, support: &Support, part_version_id: Uuid) -> Result<AcceptedSharedDefinition> {
    let part_version = project
        .part_versions
        .iter()
        .find(|part| part.part_version_id == part_version_id)
        .ok_or_else(|| StoreError::Codec("a resolved structure names a part version the project does not hold".into()))?;
    Ok(accepted_shared_definition(
        part_version,
        support.manufacturing.part(part_version_id),
        support.engineering.part(part_version_id),
        support.manufacturing.routing_of(CostOwner::PartVersion { part_version_id }),
    ))
}

/// The full accepted definition set of one resulting project version: one pin for every part version
/// used by the costed structures of the quotation-included products, sorted by part version id. The
/// costed structures come from the same SB-16 outcome the pinned snapshot recorded, so the accepted
/// definitions and the accepted cost evidence cannot disagree.
fn accepted_pins(project: &ProjectDocument, support: &Support, outputs: &BaselineOutcome) -> Result<Vec<AcceptedSharedDefinition>> {
    let mut part_versions: BTreeSet<Uuid> = BTreeSet::new();
    for line in &outputs.products {
        part_versions.extend(line.parts.iter().map(|part| part.part_version_id));
    }
    part_versions.iter().map(|part_version_id| current_shared_definition(project, support, *part_version_id)).collect()
}

/// The resulting active revision of every product that has one, sorted by product id.
fn resulting_active_revisions(project: &ProjectDocument) -> Vec<ProductRevisionSelection> {
    let mut revisions: Vec<ProductRevisionSelection> = project
        .products
        .iter()
        .filter_map(|product| {
            product.active_revision_id.map(|product_revision_id| ProductRevisionSelection { product_id: product.product_id, product_revision_id })
        })
        .collect();
    revisions.sort_by_key(|selection| selection.product_id);
    revisions
}

/// The dependency facts for SB-20, in the contract's order: the baseline fact first, then one
/// revision fact per member in product order, then one change fact per accepted definition that
/// differs from the previously accepted one.
fn activation_facts(
    members: &[ActivationMember],
    shared_definitions: &[AcceptedSharedDefinition],
    support: &Support,
    accepted_baseline_id: Uuid,
    project_version: u64,
) -> Vec<ActivationFact> {
    let mut facts = vec![ActivationFact::BaselineAccepted {
        accepted_baseline_id,
        previous_accepted_baseline_id: support.head.as_ref().map(|head| head.baseline_id),
        project_version,
    }];
    for member in members {
        match member.action {
            ActivationAction::Activated => facts.push(ActivationFact::RevisionActivated {
                product_id: member.product_id,
                product_revision_id: member.product_revision_id,
                previous_active_revision_id: member.previous_active_revision_id,
            }),
            ActivationAction::Repinned => {
                facts.push(ActivationFact::RevisionRepinned { product_id: member.product_id, product_revision_id: member.product_revision_id })
            }
        }
    }
    for definition in shared_definitions {
        let Some(accepted) =
            support.head.as_ref().and_then(|head| head.shared_definitions.iter().find(|pin| pin.part_version_id == definition.part_version_id))
        else {
            continue;
        };
        let changes = shared_definition_changes(accepted, definition);
        if !changes.is_empty() {
            facts.push(ActivationFact::SharedDefinitionChanged {
                part_version_id: definition.part_version_id,
                customer_part_number: definition.customer_part_number.clone(),
                customer_revision: definition.customer_revision.clone(),
                changes,
            });
        }
    }
    facts
}

fn push_blocker(set: &mut BlockerSet, blocker: ReadinessBlocker) {
    set.entry((blocker.owner.rank(), blocker.code.clone(), blocker.field_path.clone())).or_insert(blocker);
}

/// The readiness blockers of one resulting member revision, scoped exactly as the locked contract
/// describes.
fn member_blockers(
    project: &ProjectDocument,
    support: &Support,
    resolved: &ResolvedBaseline,
    tooling: &[ScopedToolingIssue],
    product_id: Uuid,
    product_revision_id: Uuid,
) -> Vec<ReadinessBlocker> {
    let mut blockers: BlockerSet = BTreeMap::new();
    let Some(product) = project.product(product_id) else { return Vec::new() };
    let used_parts = member_used_parts(resolved, product_id);
    let revision_prefix = format!("products.{product_id}.revisions.{product_revision_id}.");
    // SB-10 project record issues. The pending delivered-part match codes are review work, so the
    // locked contract attributes them to the engineering owner.
    for issue in project.issues() {
        if issue.code == "missing_forecast" {
            if issue.product_id == Some(product_id) {
                let mut blocker = ReadinessBlocker::new(ReadinessOwner::Project, issue.code.clone(), issue.field_path.clone(), issue.message.clone());
                blocker.product_id = Some(product_id);
                push_blocker(&mut blockers, blocker);
            }
            continue;
        }
        if PENDING_PART_MATCH_CODES.contains(&issue.code.as_str())
            && issue.product_id == Some(product_id)
            && issue.field_path.starts_with(&revision_prefix)
        {
            let mut blocker = ReadinessBlocker::new(ReadinessOwner::Engineering, issue.code.clone(), issue.field_path.clone(), issue.message.clone());
            blocker.product_id = Some(product_id);
            blocker.product_revision_id = Some(product_revision_id);
            push_blocker(&mut blockers, blocker);
        }
    }
    // SB-11 engineering review issues: component/revision-scoped or part-scoped.
    for issue in engineering_issues(&support.engineering, &support.analyses) {
        let scoped = issue.product_revision_id == Some(product_revision_id)
            || issue
                .component_id
                .and_then(|component_id| support.engineering.component(component_id))
                .is_some_and(|component| component.product_revision_id == product_revision_id)
            || issue.part_version_id.is_some_and(|part_version_id| used_parts.contains(&part_version_id));
        if !scoped {
            continue;
        }
        let mut blocker = ReadinessBlocker::new(ReadinessOwner::Engineering, issue.code.clone(), issue.field_path.clone(), issue.message.clone());
        if let Some(part_version_id) = issue.part_version_id {
            blocker.part_version_id = Some(part_version_id);
        } else {
            blocker.product_id = Some(product_id);
            blocker.product_revision_id = Some(product_revision_id);
            blocker.component_id = issue.component_id;
        }
        push_blocker(&mut blockers, blocker);
    }
    // SB-11 review coverage of every shared part version the member's resulting costed structure
    // uses. The engineering record's computed issues only visit part versions that have a review
    // record or a reviewed component, so a part version the SB-16 manifest fallback resolved without
    // any engineer review would otherwise pass the gate: its occurrence identity came from the
    // analysis manifest, and costing needs no reviewed volume of its own. The confirmed
    // classification and the reviewed measurement are still required inputs, so a used part version
    // with no reviewed-part record blocks readiness exactly like an unconfirmed one.
    for part_version_id in &used_parts {
        if support.engineering.part(*part_version_id).is_some() {
            continue;
        }
        push_blocker(
            &mut blockers,
            ReadinessBlocker::new(
                ReadinessOwner::Engineering,
                "pending_classification",
                format!("parts.{part_version_id}.classification.confirmed"),
                "the classification is a proposal until an engineer confirms a label",
            )
            .for_part(*part_version_id),
        );
        push_blocker(
            &mut blockers,
            ReadinessBlocker::new(
                ReadinessOwner::Engineering,
                "missing_reviewed_volume",
                format!("parts.{part_version_id}.volume_mm3"),
                "no usable volume is recorded; complete it manually before costing",
            )
            .for_part(*part_version_id),
        );
    }
    // SB-15 manufacturing issues, scoped through the issue's own part, product, routing or line owner.
    for issue in support.manufacturing.issues(project) {
        let Some(scope) = manufacturing_issue_scope(support, &issue, product_id, &used_parts) else { continue };
        let mut blocker = ReadinessBlocker::new(ReadinessOwner::Manufacturing, issue.code.clone(), issue.field_path.clone(), issue.message.clone());
        match scope {
            BlockerScope::Part(part_version_id) => blocker.part_version_id = Some(part_version_id),
            BlockerScope::Product(issue_product_id) => blocker.product_id = Some(issue_product_id),
            BlockerScope::Project => {}
        }
        push_blocker(&mut blockers, blocker);
    }
    // SB-14 feasibility issues, scoped by the assessed component or the finding's product revision.
    for issue in support.feasibility.issues(&support.engineering) {
        let scoped = match (issue.component_id, &issue.finding_key) {
            (Some(component_id), _) => {
                support.engineering.component(component_id).map(|component| component.product_revision_id == product_revision_id).unwrap_or(false)
            }
            (None, Some(finding_key)) => finding_key.product_revision_id == product_revision_id,
            (None, None) => true,
        };
        if !scoped {
            continue;
        }
        let mut blocker = ReadinessBlocker::new(ReadinessOwner::Feasibility, issue.code.clone(), issue.field_path.clone(), issue.message.clone());
        blocker.component_id = issue.component_id;
        if issue.finding_key.is_some() {
            blocker.product_id = Some(product_id);
            blocker.product_revision_id = Some(product_revision_id);
        }
        push_blocker(&mut blockers, blocker);
    }
    // SB-16 costing issues: product-scoped, part-scoped or project setting.
    for issue in &resolved.outputs.issues {
        match (issue.product_id, issue.part_version_id) {
            (Some(issue_product_id), _) if issue_product_id == product_id => {
                let mut blocker = ReadinessBlocker::new(ReadinessOwner::Costing, issue.code.clone(), issue.field_path.clone(), issue.message.clone());
                blocker.product_id = Some(product_id);
                push_blocker(&mut blockers, blocker);
            }
            (None, Some(part_version_id)) if used_parts.contains(&part_version_id) => {
                let mut blocker = ReadinessBlocker::new(ReadinessOwner::Costing, issue.code.clone(), issue.field_path.clone(), issue.message.clone());
                blocker.part_version_id = Some(part_version_id);
                push_blocker(&mut blockers, blocker);
            }
            (None, None) => {
                // A project setting blocks every member, except that a project with no quoted product
                // does not block a quotation-excluded member: the excluded member has no quoted cost.
                if issue.code == "no_quoted_products" && product.quotation_state == QuotationState::Excluded {
                    continue;
                }
                let blocker = ReadinessBlocker::new(ReadinessOwner::Costing, issue.code.clone(), issue.field_path.clone(), issue.message.clone());
                push_blocker(&mut blockers, blocker);
            }
            _ => {}
        }
    }
    // SB-17 tooling issues, scoped by the tool's owner.
    for issue in tooling {
        let scoped = match issue.scope {
            BlockerScope::Project => true,
            BlockerScope::Product(issue_product_id) => issue_product_id == product_id,
            BlockerScope::Part(part_version_id) => used_parts.contains(&part_version_id),
        };
        if scoped {
            push_blocker(&mut blockers, issue.blocker.clone());
        }
    }
    blockers.into_values().collect()
}

/// The part versions one member's resulting revision uses for readiness attribution: the costed
/// structure of a quotation-included product, or the resolved occurrence tree of an excluded one.
fn member_used_parts(resolved: &ResolvedBaseline, product_id: Uuid) -> BTreeSet<Uuid> {
    let mut parts = BTreeSet::new();
    let Some(input) = resolved.inputs.products.iter().find(|input| input.product_id == product_id) else {
        return parts;
    };
    if input.quoted {
        if let Some(line) = resolved.outputs.products.iter().find(|line| line.product_id == product_id) {
            parts.extend(line.parts.iter().map(|part| part.part_version_id));
        }
    } else {
        collect_occurrence_parts(&input.occurrences, &mut parts);
    }
    parts
}

/// Map one manufacturing issue to the readiness scope it belongs to, or `None` when it belongs to
/// neither the member product nor a part the member uses.
fn manufacturing_issue_scope(support: &Support, issue: &ManufacturingIssue, product_id: Uuid, used_parts: &BTreeSet<Uuid>) -> Option<BlockerScope> {
    if let Some(part_version_id) = issue.part_version_id {
        return used_parts.contains(&part_version_id).then_some(BlockerScope::Part(part_version_id));
    }
    if let Some(issue_product_id) = issue.product_id {
        return (issue_product_id == product_id).then_some(BlockerScope::Product(product_id));
    }
    if let Some(routing_id) = issue.routing_id {
        let owner = support.manufacturing.routing(routing_id).map(|routing| routing.owner)?;
        return owner_scope(owner, product_id, used_parts);
    }
    if let Some(line_id) = issue.line_id {
        let owner = support.manufacturing.attached_costs.iter().find(|line| line.line_id == line_id).map(|line| line.owner)?;
        return owner_scope(owner, product_id, used_parts);
    }
    None
}

/// The readiness scope of one cost owner.
fn owner_scope(owner: CostOwner, product_id: Uuid, used_parts: &BTreeSet<Uuid>) -> Option<BlockerScope> {
    match owner {
        CostOwner::PartVersion { part_version_id } => used_parts.contains(&part_version_id).then_some(BlockerScope::Part(part_version_id)),
        CostOwner::ProductRoot { product_id: owner_product_id } => (owner_product_id == product_id).then_some(BlockerScope::Product(product_id)),
    }
}

/// Evaluate the project's tooling completeness for a readiness report: the project default treatment
/// and the lifetime-demand basis, never a scenario assumption or a selling price.
fn tooling_issues(project: &ProjectDocument, resolved: &ResolvedBaseline) -> Result<Vec<ScopedToolingIssue>> {
    let inputs = tooling_inputs_from(project, &resolved.inputs, &resolved.outputs, &ToolingRecoveryInputs::default());
    inputs.validate().map_err(invalid)?;
    let outcome = solidbom_costing::calculate_tooling(&inputs);
    let mut issues = Vec::with_capacity(outcome.issues.len());
    for issue in &outcome.issues {
        let scope = match issue.product_id {
            Some(product_id) => BlockerScope::Product(product_id),
            None => match issue.tool_id.and_then(|tool_id| {
                project.tooling.as_ref().and_then(|tooling| tooling.entries.iter().find(|entry| entry.tool_id == tool_id)).map(|entry| entry.owner)
            }) {
                Some(ToolingOwner::PartVersion { part_version_id }) => BlockerScope::Part(part_version_id),
                Some(ToolingOwner::Product { product_id }) => BlockerScope::Product(product_id),
                None => BlockerScope::Project,
            },
        };
        let mut blocker = ReadinessBlocker::new(ReadinessOwner::Tooling, issue.code.clone(), issue.field_path.clone(), issue.message.clone());
        match scope {
            BlockerScope::Product(product_id) => blocker.product_id = Some(product_id),
            BlockerScope::Part(part_version_id) => blocker.part_version_id = Some(part_version_id),
            BlockerScope::Project => {}
        }
        issues.push(ScopedToolingIssue { scope, blocker });
    }
    Ok(issues)
}

/// Build the store error for a project read that has no result.
async fn ensure_project(pool: &sqlx::PgPool, project_id: Uuid) -> Result<()> {
    let found: Option<Uuid> =
        sqlx::query_scalar("SELECT project_id FROM project_records WHERE project_id = $1").bind(project_id).fetch_optional(pool).await?;
    found.map(|_| ()).ok_or(StoreError::NotFound)
}

/// Insert one immutable accepted baseline row. No code path updates or deletes a row later.
async fn insert_accepted_baseline(tx: &mut Transaction<'_, Postgres>, baseline: &AcceptedBaseline) -> Result<()> {
    sqlx::query(
        "INSERT INTO accepted_baselines \
         (accepted_baseline_id, project_id, sequence, request_id, project_version, engineering_version, manufacturing_version, \
          feasibility_version, costing_calculation_id, costing_input_hash, calculator_version, costing_state, members, active_revisions, \
          shared_definitions) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13::jsonb, $14::jsonb, $15::jsonb)",
    )
    .bind(baseline.accepted_baseline_id)
    .bind(baseline.project_id)
    .bind(version_to_i64(baseline.sequence)?)
    .bind(&baseline.request_id)
    .bind(version_to_i64(baseline.pinned.project_version)?)
    .bind(version_to_i64(baseline.pinned.engineering_version)?)
    .bind(version_to_i64(baseline.pinned.manufacturing_version)?)
    .bind(version_to_i64(baseline.pinned.feasibility_version)?)
    .bind(baseline.costing_calculation_id)
    .bind(&baseline.costing_input_hash)
    .bind(&baseline.calculator_version)
    .bind(result_state_to_db(baseline.costing_state))
    .bind(serde_json::to_string(&baseline.members).map_err(StoreError::codec)?)
    .bind(serde_json::to_string(&baseline.active_revisions).map_err(StoreError::codec)?)
    .bind(serde_json::to_string(&baseline.shared_definitions).map_err(StoreError::codec)?)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Decode one stored accepted-baseline row.
fn accepted_baseline_from_row(row: &PgRow) -> Result<AcceptedBaseline> {
    Ok(AcceptedBaseline {
        accepted_baseline_id: row.try_get("accepted_baseline_id")?,
        project_id: row.try_get("project_id")?,
        sequence: u64::try_from(row.try_get::<i64, _>("sequence")?).map_err(StoreError::codec)?,
        request_id: row.try_get("request_id")?,
        pinned: PinnedRecordVersions {
            project_version: u64::try_from(row.try_get::<i64, _>("project_version")?).map_err(StoreError::codec)?,
            engineering_version: u64::try_from(row.try_get::<i64, _>("engineering_version")?).map_err(StoreError::codec)?,
            manufacturing_version: u64::try_from(row.try_get::<i64, _>("manufacturing_version")?).map_err(StoreError::codec)?,
            feasibility_version: u64::try_from(row.try_get::<i64, _>("feasibility_version")?).map_err(StoreError::codec)?,
        },
        costing_calculation_id: row.try_get("costing_calculation_id")?,
        costing_input_hash: row.try_get("costing_input_hash")?,
        calculator_version: row.try_get("calculator_version")?,
        costing_state: result_state_from_db(row.try_get("costing_state")?)?,
        members: serde_json::from_str(&row.try_get::<String, _>("members")?).map_err(StoreError::codec)?,
        active_revisions: serde_json::from_str(&row.try_get::<String, _>("active_revisions")?).map_err(StoreError::codec)?,
        shared_definitions: serde_json::from_str(&row.try_get::<String, _>("shared_definitions")?).map_err(StoreError::codec)?,
        created_at: Some(row.try_get("created_at")?),
    })
}

/// The wire spelling of a stored cost state.
fn result_state_to_db(state: ResultState) -> &'static str {
    match state {
        ResultState::Complete => "complete",
        ResultState::Incomplete => "incomplete",
    }
}

/// The stored `costing_state` as its contract value.
fn result_state_from_db(text: String) -> Result<ResultState> {
    match text.as_str() {
        "complete" => Ok(ResultState::Complete),
        "incomplete" => Ok(ResultState::Incomplete),
        other => Err(StoreError::Codec(format!("the stored accepted-baseline costing state `{other}` is unknown"))),
    }
}
