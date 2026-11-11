//! Dependency updates, impact reports and explicit adoption (package SB-20, adoption contract
//! version 1 additive to API version 2).
//!
//! The contract vocabulary lives in [`solidbom_contract::adoption`] and the published rules in
//! `docs/solidbom/contracts/v2/dependency-updates-and-adoption.md`; this module is the service side.
//!
//! - **Shared-update catalog** ([`Store::shared_updates`]) is a pure read: the newest published
//!   version of every shared logical record, built from the six append-only version chains
//!   (material, cost-center rate, catalog component, routing template, feasibility rule table and
//!   app configuration). The append-only `reference_changes` fact table covers only four of the six
//!   kinds, so it is not the catalog source; the chains are. The answer is bounded by
//!   [`MAX_SHARED_UPDATES`] and truncated deterministically after the canonical sort, exactly like
//!   the other bounded history reads. Project reports never read this truncated list: they resolve
//!   their own dependency set.
//! - **Impact report and project list** ([`Store::project_dependency_report`],
//!   [`Store::dependency_projects`]) are point-in-time reads in one transaction. They resolve each
//!   kind's referenced version from the current project, engineering, manufacturing and feasibility
//!   records and the published reference data, and compare it with the newest published version of
//!   the same logical record. A report writes no row. The newest draft result is the newest
//!   baseline snapshot that no accepted baseline pins (an activation snapshot is accepted history);
//!   it is stale when a costing-kind update is pending or a shared-data adoption was recorded after
//!   it, and a project without one flags no scenario. `pending_count` keeps the bounded dependency
//!   window honest.
//! - **Explicit decisions** ([`Store::record_dependency_decisions`]) is one transaction: the project
//!   row, then the engineering and manufacturing record heads, are locked; the current referenced
//!   versions are checked; an acknowledgement appends decision rows only, an adoption first repins
//!   the owning document and then appends the decision rows and exactly one record version per
//!   document that changed. A refusal or failure writes nothing.
//! - **Draft publication** ([`Store::recalculate_draft`] and the automatic publication inside the
//!   project writes) resolves the SB-16 baseline, appends one immutable snapshot and answers with
//!   the snapshot summary, the still-pending dependencies and the scenarios the new result flags.
//!   A publication never appends a project or record version.
//!
//! # App-configuration resolution refinement
//!
//! SB-16 published "resolve the latest published app configuration". This package narrows that for
//! a project that resolved an older configuration: the project's latest adopted `app_config`
//! decision wins, then the configuration version its newest baseline snapshot recorded, then the
//! latest published configuration. The refinement lives in [`app_config_for_project`] and is called
//! from the SB-16 resolution (`costing::resolve_baseline`), so the calculator's `category_config` is
//! the refined version. A project with no adopted decision and no snapshot keeps the published
//! latest-version behaviour exactly. A decision naming an unreadable configuration version is an
//! error, never a silent fallback.
//!
//! # Dependency scope
//!
//! The affected product set of a dependency is the set of products whose draft rule revision (the
//! product's newest `draft` revision, else its `active` revision) resolves the referencing part
//! version; a dependency with no target part version (a product-root routing, a feasibility
//! assessment) names products directly. The scope therefore describes exactly what a draft
//! publication of this project would cost.

use std::collections::{BTreeMap, BTreeSet};

use solidbom_contract::{
    AppConfigVersion, BaselineCalculationSummary, BaselineOccurrenceInput, CatalogComponentVersion, CostCategory, CostOwner, DependencyDecision,
    DependencyDecisionAction, DependencyDecisionReceipt, DependencyFact, DependencyProjectList, DependencyRefusal, DraftPublication,
    DraftResultStatus, DraftStaleReason, EngineeringRecordDocument, FeasibilityRecordDocument, FlaggedScenario, ImpactedProject,
    MAX_AFFECTED_PROJECTS, MAX_DECISION_HISTORY, MAX_DEPENDENCY_TARGETS, MAX_FLAGGED_SCENARIOS, MAX_PROJECT_DEPENDENCIES, MAX_SHARED_UPDATES,
    ManufacturingRecordDocument, ProductRevision, ProductRevisionSelection, ProductRevisionState, ProjectDependency, ProjectDependencyReport,
    ProjectDocument, RecalculateDraft, RecordDependencyDecisions, ReferenceData, RoutingSource, SharedUpdate, SharedUpdateKind, SharedUpdateList,
    Sourcing,
};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

use crate::Result;
use crate::costing::{
    engineering_record, insert_baseline_snapshot, manufacturing_record, project_version_document, resolve_baseline, resolve_occurrences,
    result_state_from_db, version_from_i64, version_to_i64,
};
use crate::engineering::{
    RegisteredAnalysis, append_version as append_engineering_version, locked_current as locked_engineering, registered_analyses,
};
use crate::feasibility::current_feasibility_record;
use crate::manufacturing::{
    append_version as append_manufacturing_version, locked_current as locked_manufacturing, pinned_rates, validate_operation_rates,
};
use crate::projects::{current_document, invalid};
use crate::store::{RFC3339_UTC, Store, StoreError, check_request_id, lock_request, payload_hash, record_reply, recorded_reply, reference_data_in};

/// The prefix of the dependency-decision operations recorded in the shared request log.
const DECISION_OPERATION: &str = "dependency_decisions.record";

/// The prefix of the explicit draft-publication operations recorded in the shared request log.
const PUBLICATION_OPERATION: &str = "draft_publication.recalculate";

/// How many projects one global list scans; the locked contract caps the scan and the returned rows.
const MAX_SCANNED_PROJECTS: usize = 1_000;

/// The SB-18 stale-reason vocabulary a flagged scenario reuses.
const SCENARIO_BASELINE_OUTDATED_CODE: &str = "scenario_baseline_outdated";
const SCENARIO_PROJECT_OUTDATED_CODE: &str = "scenario_project_outdated";

impl Store {
    /// The newest published version of every shared logical record. A pure read: it appends no row.
    pub async fn shared_updates(&self) -> Result<SharedUpdateList> {
        let mut tx = self.pool().begin().await?;
        let rows = shared_chain_rows_in(&mut tx).await?;
        tx.commit().await?;
        let mut updates: Vec<SharedUpdate> = rows
            .into_iter()
            .map(|row| SharedUpdate {
                kind: row.kind,
                record_id: row.record_id,
                version_id: row.version_id,
                version: row.version,
                previous_version_id: row.previous_version_id,
                published_at: Some(row.published_at),
            })
            .collect();
        updates.sort_by_key(|update| (update.kind.as_str(), update.record_id, update.version));
        // The contract bounds the catalog. The chains are the source of truth and project reports
        // read their own dependency set, so a deterministic truncation here never hides a project's
        // pending dependency from that project's report.
        updates.truncate(MAX_SHARED_UPDATES);
        let list = SharedUpdateList { updates };
        list.validate().map_err(StoreError::codec)?;
        Ok(list)
    }

    /// The point-in-time dependency impact report of one project. A pure read in one transaction:
    /// it appends no project version, record version, snapshot, decision row or request receipt.
    pub async fn project_dependency_report(&self, project_id: Uuid) -> Result<ProjectDependencyReport> {
        let mut tx = self.pool().begin().await?;
        // One consistent view: the report names one project version and the exact records it read.
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ").execute(&mut *tx).await?;
        let records = project_records_in(&mut tx, project_id).await?;
        let support = shared_support_in(&mut tx).await?;
        let acknowledged = acknowledged_decisions_in(&mut tx, project_id).await?;
        let app_config = app_config_for_project(&mut tx, project_id, support.reference.app_config.as_ref()).await?;
        let all_dependencies = project_dependencies(&records, &support, app_config.as_ref(), &acknowledged)?;
        let pending_count = u64::try_from(all_dependencies.len()).map_err(StoreError::codec)?;
        let draft = newest_draft_in(&mut tx, project_id, &records, &all_dependencies).await?;
        let flagged_scenarios = flagged_scenarios_in(
            &mut tx,
            project_id,
            draft.as_ref().map(|draft| NewestDraft { calculation_id: draft.calculation_id, stale: draft.stale }),
            records.project_version,
        )
        .await?;
        let report = ProjectDependencyReport {
            project_id,
            project_version: records.project_version,
            engineering_version: records.engineering_version,
            manufacturing_version: records.manufacturing_version,
            pending_count,
            dependencies: bounded_dependencies(all_dependencies),
            decisions: decision_history_in(&mut tx, project_id).await?,
            draft,
            flagged_scenarios,
        };
        report.validate().map_err(StoreError::codec)?;
        tx.commit().await?;
        Ok(report)
    }

    /// The bounded global list of projects a shared update flags, optionally narrowed by one
    /// (kind, record) pair. A pure read; a project with no pending dependency is never listed.
    pub async fn dependency_projects(&self, filter: Option<(SharedUpdateKind, Uuid)>) -> Result<DependencyProjectList> {
        let mut tx = self.pool().begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ").execute(&mut *tx).await?;
        let support = shared_support_in(&mut tx).await?;
        if let Some((kind, record_id)) = filter
            // An unknown filter value is `422 invalid` (never an empty list that looks current).
            && !support.rows.iter().any(|row| row.kind == kind && row.record_id == record_id)
        {
            return Err(StoreError::Invalid(format!(
                "dependency_projects.filter: nothing published matches the {} record `{record_id}`",
                kind.as_str()
            )));
        }
        let scan_limit = i64::try_from(MAX_SCANNED_PROJECTS + 1).map_err(StoreError::codec)?;
        let project_ids: Vec<Uuid> =
            sqlx::query_scalar("SELECT project_id FROM project_records ORDER BY project_id LIMIT $1").bind(scan_limit).fetch_all(&mut *tx).await?;
        let mut projects: Vec<ImpactedProject> = Vec::new();
        for project_id in project_ids.into_iter().take(MAX_SCANNED_PROJECTS) {
            let records = match project_records_in(&mut tx, project_id).await {
                Ok(records) => records,
                // A project deleted between the scan and the read is not a listed project.
                Err(StoreError::NotFound) => continue,
                Err(error) => return Err(error),
            };
            let acknowledged = acknowledged_decisions_in(&mut tx, project_id).await?;
            let app_config = app_config_for_project(&mut tx, project_id, support.reference.app_config.as_ref()).await?;
            let dependencies = project_dependencies(&records, &support, app_config.as_ref(), &acknowledged)?;
            let filtered: Vec<&ProjectDependency> = match filter {
                Some((kind, record_id)) => {
                    dependencies.iter().filter(|dependency| dependency.kind == kind && dependency.record_id == record_id).collect()
                }
                None => dependencies.iter().collect(),
            };
            if filtered.is_empty() {
                continue;
            }
            let mut kinds: Vec<SharedUpdateKind> = dependencies.iter().map(|dependency| dependency.kind).collect();
            kinds.sort_by_key(|kind| kind.as_str());
            kinds.dedup();
            projects.push(ImpactedProject {
                project_id,
                project_version: records.project_version,
                pending_count: u64::try_from(dependencies.len()).map_err(StoreError::codec)?,
                kinds,
            });
            if projects.len() >= MAX_AFFECTED_PROJECTS {
                break;
            }
        }
        let list = DependencyProjectList { projects };
        list.validate().map_err(StoreError::codec)?;
        tx.commit().await?;
        Ok(list)
    }

    /// Record one explicit acknowledgement or adoption. One transaction: request replay, the
    /// project-row and record-head locks, every identity/version/pending/duplicate check, then the
    /// repins, record versions, decision rows and receipt — or none of them.
    pub async fn record_dependency_decisions(
        &self,
        project_id: Uuid,
        request: &RecordDependencyDecisions,
        decided_by: &str,
    ) -> Result<DependencyDecisionReceipt> {
        request.validate().map_err(invalid)?;
        check_request_id(&request.request_id)?;
        let payload = payload_hash(DECISION_OPERATION, &(project_id, request))?;
        let mut tx = self.pool().begin().await?;
        lock_request(&mut tx, &request.request_id).await?;
        if let Some(receipt) = recorded_reply::<DependencyDecisionReceipt>(&mut tx, &request.request_id, &payload).await? {
            tx.commit().await?;
            return Ok(receipt);
        }
        // The project row first, then the engineering and manufacturing heads, in the one order
        // every SB-20 transaction uses, so the pins a decision reads are the pins it writes.
        let (current, project) = current_document(&mut tx, project_id).await?;
        if current != request.expected_version {
            return Err(refusal_error(DependencyRefusal::StaleProjectVersion { expected: request.expected_version, current }));
        }
        let (engineering_version, engineering) = locked_engineering(&mut tx, project_id).await?;
        let (manufacturing_version, manufacturing) = locked_manufacturing(&mut tx, project_id).await?;
        let support = shared_support_in(&mut tx).await?;
        let acknowledged = acknowledged_decisions_in(&mut tx, project_id).await?;
        let app_config = app_config_for_project(&mut tx, project_id, support.reference.app_config.as_ref()).await?;
        let mut records = ProjectRecords {
            project_version: current,
            project,
            engineering_version,
            engineering,
            manufacturing_version,
            manufacturing,
            feasibility: current_feasibility_record(&mut tx, project_id).await?.1,
            analyses: registered_analyses(&mut tx, project_id).await?,
        };
        let dependencies = project_dependencies(&records, &support, app_config.as_ref(), &acknowledged)?;
        let dependency_index: BTreeMap<(&'static str, Uuid), &ProjectDependency> =
            dependencies.iter().map(|dependency| ((dependency.kind.as_str(), dependency.record_id), dependency)).collect();

        // Validate every item before changing anything. The transaction rolls back on any refusal,
        // but keeping the check pass pure makes "a failed adoption writes nothing" structural.
        let mut planned: Vec<PlannedDecision> = Vec::with_capacity(request.decisions.len());
        for item in &request.decisions {
            let Some(latest) = support.latest_version(item.kind, item.record_id) else {
                return Err(refusal_error(DependencyRefusal::UnknownUpdate {
                    kind: item.kind,
                    record_id: item.record_id,
                    version_id: item.version_id,
                }));
            };
            let Some((record_id, _version)) = support.version_by_id(item.kind, item.version_id) else {
                return Err(refusal_error(DependencyRefusal::UnknownUpdate {
                    kind: item.kind,
                    record_id: item.record_id,
                    version_id: item.version_id,
                }));
            };
            if record_id != item.record_id {
                return Err(refusal_error(DependencyRefusal::UnknownUpdate {
                    kind: item.kind,
                    record_id: item.record_id,
                    version_id: item.version_id,
                }));
            }
            if latest.version_id != item.version_id {
                return Err(refusal_error(DependencyRefusal::StaleUpdate {
                    kind: item.kind,
                    record_id: item.record_id,
                    named_version_id: item.version_id,
                    current_version_id: latest.version_id,
                }));
            }
            let Some(dependency) = dependency_index.get(&(item.kind.as_str(), item.record_id)) else {
                return Err(refusal_error(DependencyRefusal::NoPendingDependency {
                    kind: item.kind,
                    record_id: item.record_id,
                    version_id: item.version_id,
                }));
            };
            if decision_recorded(&mut tx, project_id, item.kind, item.record_id, item.version_id, request.action).await? {
                return Err(refusal_error(DependencyRefusal::DuplicateDecision {
                    kind: item.kind,
                    record_id: item.record_id,
                    version_id: item.version_id,
                    action: request.action,
                }));
            }
            if request.action == DependencyDecisionAction::Adopted && !item.kind.is_adoptable() {
                return Err(refusal_error(DependencyRefusal::UnsupportedAdoption { kind: item.kind }));
            }
            planned.push(PlannedDecision {
                kind: item.kind,
                record_id: item.record_id,
                to_version_id: item.version_id,
                from_version_id: Some(dependency.referenced_version_id),
            });
        }
        // Canonical order: the receipt and the appended rows both use `(kind, record id)`.
        planned.sort_by_key(|decision| (decision.kind.as_str(), decision.record_id));

        let mut manufacturing_changed = false;
        let mut engineering_changed = false;
        if request.action == DependencyDecisionAction::Adopted {
            for decision in &planned {
                match decision.kind {
                    SharedUpdateKind::Material => {
                        manufacturing_changed |= repin_material(&mut records.manufacturing, decision.record_id, decision.to_version_id, &support)?;
                    }
                    SharedUpdateKind::CostCenterRate => {
                        manufacturing_changed |= repin_rates(&mut records.manufacturing, decision.record_id, decision.to_version_id, &support)?;
                    }
                    SharedUpdateKind::CatalogComponent => {
                        let component = support.component_version(decision.to_version_id).ok_or_else(|| {
                            StoreError::Invalid(format!(
                                "dependency_decisions.decisions: the catalog component version `{}` is not published",
                                decision.to_version_id
                            ))
                        })?;
                        engineering_changed |= repin_catalog(&mut records.engineering, decision.record_id, component, decided_by);
                    }
                    // The decision row is the pin the resolution refinement reads; no document changes.
                    SharedUpdateKind::AppConfig => {}
                    SharedUpdateKind::RoutingTemplate | SharedUpdateKind::FeasibilityRuleTable => {
                        return Err(refusal_error(DependencyRefusal::UnsupportedAdoption { kind: decision.kind }));
                    }
                }
            }
            if manufacturing_changed {
                // The repin must not store a document the manufacturing write path itself refuses.
                let rates = pinned_rates(&mut tx).await?;
                for routing in &records.manufacturing.routings {
                    validate_operation_rates(&routing.operations, &rates)?;
                }
                records.manufacturing.validate().map_err(invalid)?;
                let version = records
                    .manufacturing_version
                    .checked_add(1)
                    .ok_or_else(|| StoreError::Invalid("the manufacturing version is too large".into()))?;
                append_manufacturing_version(&mut tx, project_id, version, &records.manufacturing).await?;
            }
            if engineering_changed {
                records.engineering.validate().map_err(invalid)?;
                let version =
                    records.engineering_version.checked_add(1).ok_or_else(|| StoreError::Invalid("the engineering version is too large".into()))?;
                append_engineering_version(&mut tx, project_id, version, &records.engineering).await?;
            }
        }

        // The decision rows are append-only; the project row is locked, so the sequence is safe.
        let last_sequence: i64 = sqlx::query_scalar("SELECT coalesce(max(sequence), 0) FROM dependency_decisions WHERE project_id = $1")
            .bind(project_id)
            .fetch_one(&mut *tx)
            .await?;
        let mut sequence = u64::try_from(last_sequence).map_err(StoreError::codec)?;
        let mut decisions: Vec<DependencyDecision> = Vec::with_capacity(planned.len());
        for decision in &planned {
            sequence = sequence.checked_add(1).ok_or_else(|| StoreError::Invalid("the decision sequence is too large".into()))?;
            let decision_id = Uuid::new_v4();
            let decided_at: String = sqlx::query_scalar(&format!(
                "INSERT INTO dependency_decisions \
                 (decision_id, project_id, sequence, kind, record_id, from_version_id, to_version_id, action, decided_by) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
                 RETURNING to_char(decided_at AT TIME ZONE 'UTC', {RFC3339_UTC})"
            ))
            .bind(decision_id)
            .bind(project_id)
            .bind(version_to_i64(sequence)?)
            .bind(decision.kind.as_str())
            .bind(decision.record_id)
            .bind(decision.from_version_id)
            .bind(decision.to_version_id)
            .bind(request.action.as_str())
            .bind(decided_by)
            .fetch_one(&mut *tx)
            .await?;
            decisions.push(DependencyDecision {
                decision_id,
                project_id,
                sequence,
                kind: decision.kind,
                record_id: decision.record_id,
                from_version_id: decision.from_version_id,
                to_version_id: decision.to_version_id,
                action: request.action,
                decided_by: decided_by.to_string(),
                decided_at,
            });
        }
        let facts = decisions
            .iter()
            .map(|decision| match decision.action {
                DependencyDecisionAction::Acknowledged => {
                    DependencyFact::DependencyAcknowledged { kind: decision.kind, record_id: decision.record_id, version_id: decision.to_version_id }
                }
                DependencyDecisionAction::Adopted => DependencyFact::DependencyAdopted {
                    kind: decision.kind,
                    record_id: decision.record_id,
                    previous_version_id: decision.from_version_id,
                    version_id: decision.to_version_id,
                },
            })
            .collect();
        // The receipt names the record generations the transaction left behind, so a client can
        // prepare its next adoption or publication against the exact state it produced.
        let engineering_version = current_generation_in(&mut tx, project_id, RecordHead::Engineering).await?;
        let manufacturing_version = current_generation_in(&mut tx, project_id, RecordHead::Manufacturing).await?;
        let receipt = DependencyDecisionReceipt {
            request_id: request.request_id.clone(),
            project_id,
            project_version: current,
            engineering_version,
            manufacturing_version,
            decisions,
            facts,
        };
        receipt.validate().map_err(StoreError::codec)?;
        record_reply(&mut tx, &request.request_id, DECISION_OPERATION, &payload, &receipt).await?;
        tx.commit().await?;
        Ok(receipt)
    }

    /// Publish one explicit draft calculation. One transaction: the project row, then the engineering
    /// and manufacturing heads are locked `FOR SHARE`; the three expected generations must match
    /// exactly; the SB-16 baseline is resolved and one immutable snapshot appended. No project or
    /// record version is appended.
    pub async fn recalculate_draft(&self, project_id: Uuid, request: &RecalculateDraft) -> Result<DraftPublication> {
        request.validate().map_err(invalid)?;
        check_request_id(&request.request_id)?;
        let payload = payload_hash(PUBLICATION_OPERATION, &(project_id, request))?;
        let mut tx = self.pool().begin().await?;
        lock_request(&mut tx, &request.request_id).await?;
        if let Some(publication) = recorded_reply::<DraftPublication>(&mut tx, &request.request_id, &payload).await? {
            tx.commit().await?;
            return Ok(publication);
        }
        let (current, project) = current_document(&mut tx, project_id).await?;
        lock_record_heads_for_share(&mut tx, project_id).await?;
        let (engineering_version, _) = engineering_record(&mut tx, project_id, None).await?;
        let (manufacturing_version, _) = manufacturing_record(&mut tx, project_id, None).await?;
        let mut stale_reasons = Vec::new();
        if current != request.expected_project_version {
            stale_reasons.push(DraftStaleReason::NewerProjectVersion);
        }
        if engineering_version != request.expected_engineering_version {
            stale_reasons.push(DraftStaleReason::NewerEngineeringVersion);
        }
        if manufacturing_version != request.expected_manufacturing_version {
            stale_reasons.push(DraftStaleReason::NewerManufacturingVersion);
        }
        if !stale_reasons.is_empty() {
            stale_reasons.sort_by_key(|reason| reason.as_str());
            return Err(refusal_error(DependencyRefusal::StaleDraftGeneration { reasons: stale_reasons }));
        }
        let selections = if request.product_revisions.is_empty() { default_draft_selections(&project) } else { request.product_revisions.clone() };
        // A generation 0 means the project has no such record yet; the SB-16 reader returns the
        // empty record for it, so the exact current version is read through `None` instead of asking
        // it to read a version 0.
        let engineering_request = (engineering_version != 0).then_some(engineering_version);
        let manufacturing_request = (manufacturing_version != 0).then_some(manufacturing_version);
        let resolved = resolve_baseline(&mut tx, project_id, &project, engineering_request, manufacturing_request, &selections).await?;
        let snapshot = insert_baseline_snapshot(&mut tx, project_id, current, resolved).await?;
        let publication = draft_publication_in(&mut tx, project_id, snapshot_summary(&snapshot), snapshot.calculation_id).await?;
        publication.validate().map_err(StoreError::codec)?;
        record_reply(&mut tx, &request.request_id, PUBLICATION_OPERATION, &payload, &publication).await?;
        tx.commit().await?;
        Ok(publication)
    }
}

/// Publish the project's draft calculation inside the caller's transaction, after the project
/// version it reads was appended. The engineering and manufacturing heads are locked `FOR SHARE`,
/// the default draft revision rule is used, and one immutable snapshot is appended. A missing input
/// is an `incomplete` result, never an error; a genuine storage failure rolls the whole edit back.
pub(crate) async fn publish_draft_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    project_version: u64,
    project: &ProjectDocument,
) -> Result<DraftPublication> {
    lock_record_heads_for_share(tx, project_id).await?;
    let selections = default_draft_selections(project);
    let resolved = resolve_baseline(tx, project_id, project, None, None, &selections).await?;
    let snapshot = insert_baseline_snapshot(tx, project_id, project_version, resolved).await?;
    let publication = draft_publication_in(tx, project_id, snapshot_summary(&snapshot), snapshot.calculation_id).await?;
    publication.validate().map_err(StoreError::codec)?;
    Ok(publication)
}

/// The app-configuration version one project resolves, per the locked refinement: the latest
/// adopted decision, else the configuration version the newest baseline snapshot recorded, else the
/// latest published configuration. An unreadable adopted or recorded version is an error, never a
/// silent fallback to the latest published one.
pub(crate) async fn app_config_for_project(
    tx: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    latest: Option<&AppConfigVersion>,
) -> Result<Option<AppConfigVersion>> {
    let adopted: Option<Uuid> = sqlx::query_scalar(
        "SELECT to_version_id FROM dependency_decisions \
         WHERE project_id = $1 AND kind = 'app_config' AND action = 'adopted' ORDER BY sequence DESC LIMIT 1",
    )
    .bind(project_id)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(config_version_id) = adopted {
        return app_config_version_in(tx, config_version_id).await?.map(Some).ok_or_else(|| {
            StoreError::Codec(format!("the adopted app configuration version `{config_version_id}` is not a readable published version"))
        });
    }
    let recorded: Option<Option<String>> = sqlx::query_scalar(
        "SELECT inputs->'category_config'->>'config_version_id' FROM baseline_calculations \
         WHERE project_id = $1 ORDER BY created_at DESC, calculation_id DESC LIMIT 1",
    )
    .bind(project_id)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(Some(text)) = recorded {
        let config_version_id = Uuid::parse_str(&text).map_err(StoreError::codec)?;
        return app_config_version_in(tx, config_version_id).await?.map(Some).ok_or_else(|| {
            StoreError::codec(format!("the recorded app configuration version `{config_version_id}` is not a readable published version"))
        });
    }
    Ok(latest.cloned())
}

// ---------------------------------------------------------------------------
// Shared version chains and lookups
// ---------------------------------------------------------------------------

/// One newest published version of one shared logical record, as the six chains store it.
struct SharedChainRow {
    kind: SharedUpdateKind,
    record_id: Uuid,
    version_id: Uuid,
    version: u64,
    previous_version_id: Option<Uuid>,
    published_at: String,
}

/// The newest published version of every shared logical record, per kind.
struct SharedSupport {
    rows: Vec<SharedChainRow>,
    latest: BTreeMap<(&'static str, Uuid), VersionRef>,
    by_version: BTreeMap<(&'static str, Uuid), (Uuid, u64)>,
    /// Every published routing-template version, because an applied routing may pin an older one.
    template_versions: BTreeMap<(Uuid, u64), Uuid>,
    /// Every published feasibility rule-table version, because a recorded assessment may name an
    /// older one.
    rule_versions: BTreeMap<Uuid, (Uuid, u64)>,
    reference: ReferenceData,
}

/// One published version identity: the row id and the chain position.
#[derive(Clone, Copy)]
struct VersionRef {
    version_id: Uuid,
    version: u64,
}

impl SharedSupport {
    /// The newest published version of one logical record.
    fn latest_version(&self, kind: SharedUpdateKind, record_id: Uuid) -> Option<VersionRef> {
        self.latest.get(&(kind.as_str(), record_id)).copied()
    }

    /// The logical record and chain position of one published version id.
    fn version_by_id(&self, kind: SharedUpdateKind, version_id: Uuid) -> Option<(Uuid, u64)> {
        self.by_version.get(&(kind.as_str(), version_id)).copied()
    }

    /// One published catalog component version.
    fn component_version(&self, version_id: Uuid) -> Option<&CatalogComponentVersion> {
        self.reference.components.iter().find(|component| component.component_version_id == version_id)
    }
}

/// Read the newest published version of every shared logical record from the six append-only
/// version chains, plus the published reference data the dependency resolution reads. The
/// `reference_changes` fact table is not read: it only records four of the six kinds.
async fn shared_support_in(tx: &mut Transaction<'_, Postgres>) -> Result<SharedSupport> {
    let rows = shared_chain_rows_in(tx).await?;
    let mut latest = BTreeMap::new();
    for row in &rows {
        latest.insert((row.kind.as_str(), row.record_id), VersionRef { version_id: row.version_id, version: row.version });
    }
    // Every published version id resolves to its logical record and chain position: a decision
    // naming an older published version is `stale_update`, not an unknown record.
    let mut by_version: BTreeMap<(&'static str, Uuid), (Uuid, u64)> = BTreeMap::new();
    for (kind, version_id, record_id, version) in all_version_rows_in(tx).await? {
        by_version.insert((kind.as_str(), version_id), (record_id, version));
    }
    let template_rows: Vec<(Uuid, i64, Uuid)> =
        sqlx::query_as("SELECT template_id, version, template_version_id FROM routing_template_versions").fetch_all(&mut **tx).await?;
    let mut template_versions: BTreeMap<(Uuid, u64), Uuid> = BTreeMap::new();
    for (template_id, version, template_version_id) in template_rows {
        template_versions.insert((template_id, version_from_i64(version)?), template_version_id);
    }
    let rule_rows: Vec<(Uuid, Uuid, i64)> =
        sqlx::query_as("SELECT rule_table_version_id, rule_table_id, version FROM feasibility_rule_versions").fetch_all(&mut **tx).await?;
    let mut rule_versions: BTreeMap<Uuid, (Uuid, u64)> = BTreeMap::new();
    for (version_id, rule_table_id, version) in rule_rows {
        rule_versions.insert(version_id, (rule_table_id, version_from_i64(version)?));
    }
    let reference = reference_data_in(tx).await?;
    Ok(SharedSupport { rows, latest, by_version, template_versions, rule_versions, reference })
}

async fn shared_chain_rows_in(tx: &mut Transaction<'_, Postgres>) -> Result<Vec<SharedChainRow>> {
    let mut rows = Vec::new();
    let material_rows = sqlx::query(&format!(
        "SELECT DISTINCT ON (material_id) material_id AS record_id, material_version_id AS version_id, version, supersedes AS previous_version_id, \
         to_char(created_at AT TIME ZONE 'UTC', {RFC3339_UTC}) AS published_at \
         FROM material_versions ORDER BY material_id, version DESC"
    ))
    .fetch_all(&mut **tx)
    .await?;
    for row in &material_rows {
        rows.push(chain_row(SharedUpdateKind::Material, row)?);
    }
    let rate_rows = sqlx::query(&format!(
        "SELECT DISTINCT ON (cost_center_id) cost_center_id AS record_id, rate_version_id AS version_id, version, \
         supersedes AS previous_version_id, to_char(created_at AT TIME ZONE 'UTC', {RFC3339_UTC}) AS published_at \
         FROM rate_versions ORDER BY cost_center_id, version DESC"
    ))
    .fetch_all(&mut **tx)
    .await?;
    for row in &rate_rows {
        rows.push(chain_row(SharedUpdateKind::CostCenterRate, row)?);
    }
    let component_rows = sqlx::query(&format!(
        "SELECT DISTINCT ON (component_id) component_id AS record_id, component_version_id AS version_id, version, \
         supersedes AS previous_version_id, to_char(created_at AT TIME ZONE 'UTC', {RFC3339_UTC}) AS published_at \
         FROM catalog_component_versions ORDER BY component_id, version DESC"
    ))
    .fetch_all(&mut **tx)
    .await?;
    for row in &component_rows {
        rows.push(chain_row(SharedUpdateKind::CatalogComponent, row)?);
    }
    // The routing-template chain has no `supersedes` column; the stored version document carries it.
    let template_rows = sqlx::query(&format!(
        "SELECT t.template_id AS record_id, v.template_version_id AS version_id, v.version, \
         (v.document ->> 'supersedes') AS previous_version_id, \
         to_char(v.created_at AT TIME ZONE 'UTC', {RFC3339_UTC}) AS published_at \
         FROM routing_templates t JOIN routing_template_versions v ON v.template_id = t.template_id AND v.version = t.current_version \
         ORDER BY t.template_id"
    ))
    .fetch_all(&mut **tx)
    .await?;
    for row in &template_rows {
        rows.push(chain_row(SharedUpdateKind::RoutingTemplate, row)?);
    }
    let rule_rows = sqlx::query(&format!(
        "SELECT DISTINCT ON (rule_table_id) rule_table_id AS record_id, rule_table_version_id AS version_id, version, \
         supersedes AS previous_version_id, to_char(created_at AT TIME ZONE 'UTC', {RFC3339_UTC}) AS published_at \
         FROM feasibility_rule_versions ORDER BY rule_table_id, version DESC"
    ))
    .fetch_all(&mut **tx)
    .await?;
    for row in &rule_rows {
        rows.push(chain_row(SharedUpdateKind::FeasibilityRuleTable, row)?);
    }
    let config_rows = sqlx::query(&format!(
        "SELECT DISTINCT ON (config_id) config_id AS record_id, config_version_id AS version_id, version, \
         supersedes AS previous_version_id, to_char(created_at AT TIME ZONE 'UTC', {RFC3339_UTC}) AS published_at \
         FROM app_config_versions ORDER BY config_id, version DESC"
    ))
    .fetch_all(&mut **tx)
    .await?;
    for row in &config_rows {
        rows.push(chain_row(SharedUpdateKind::AppConfig, row)?);
    }
    Ok(rows)
}

/// Every published shared version id, its logical record and its chain position. The catalog only
/// needs the newest version of each record, but a decision or a stored reference may name an older
/// published version, which is a `stale_update` rather than an unknown one.
async fn all_version_rows_in(tx: &mut Transaction<'_, Postgres>) -> Result<Vec<(SharedUpdateKind, Uuid, Uuid, u64)>> {
    let mut versions: Vec<(SharedUpdateKind, Uuid, Uuid, u64)> = Vec::new();
    let materials: Vec<(Uuid, Uuid, i64)> =
        sqlx::query_as("SELECT material_version_id, material_id, version FROM material_versions").fetch_all(&mut **tx).await?;
    for (version_id, record_id, version) in materials {
        versions.push((SharedUpdateKind::Material, version_id, record_id, version_from_i64(version)?));
    }
    let rates: Vec<(Uuid, Uuid, i64)> =
        sqlx::query_as("SELECT rate_version_id, cost_center_id, version FROM rate_versions").fetch_all(&mut **tx).await?;
    for (version_id, record_id, version) in rates {
        versions.push((SharedUpdateKind::CostCenterRate, version_id, record_id, version_from_i64(version)?));
    }
    let components: Vec<(Uuid, Uuid, i64)> =
        sqlx::query_as("SELECT component_version_id, component_id, version FROM catalog_component_versions").fetch_all(&mut **tx).await?;
    for (version_id, record_id, version) in components {
        versions.push((SharedUpdateKind::CatalogComponent, version_id, record_id, version_from_i64(version)?));
    }
    let templates: Vec<(Uuid, Uuid, i64)> =
        sqlx::query_as("SELECT template_version_id, template_id, version FROM routing_template_versions").fetch_all(&mut **tx).await?;
    for (version_id, record_id, version) in templates {
        versions.push((SharedUpdateKind::RoutingTemplate, version_id, record_id, version_from_i64(version)?));
    }
    let rules: Vec<(Uuid, Uuid, i64)> =
        sqlx::query_as("SELECT rule_table_version_id, rule_table_id, version FROM feasibility_rule_versions").fetch_all(&mut **tx).await?;
    for (version_id, record_id, version) in rules {
        versions.push((SharedUpdateKind::FeasibilityRuleTable, version_id, record_id, version_from_i64(version)?));
    }
    let configs: Vec<(Uuid, Uuid, i64)> =
        sqlx::query_as("SELECT config_version_id, config_id, version FROM app_config_versions").fetch_all(&mut **tx).await?;
    for (version_id, record_id, version) in configs {
        versions.push((SharedUpdateKind::AppConfig, version_id, record_id, version_from_i64(version)?));
    }
    Ok(versions)
}

/// One chain row, with the routing-template previous version parsed out of the stored document.
fn chain_row(kind: SharedUpdateKind, row: &sqlx::postgres::PgRow) -> Result<SharedChainRow> {
    let previous_version_id: Option<Uuid> = match row.try_get::<Option<Uuid>, _>("previous_version_id") {
        Ok(previous) => previous,
        Err(_) => {
            // The routing-template query returns the document's `supersedes` as text.
            let text: Option<String> = row.try_get("previous_version_id")?;
            text.map(|text| Uuid::parse_str(&text).map_err(StoreError::codec)).transpose()?
        }
    };
    Ok(SharedChainRow {
        kind,
        record_id: row.try_get("record_id")?,
        version_id: row.try_get("version_id")?,
        version: version_from_i64(row.try_get("version")?)?,
        previous_version_id,
        published_at: row.try_get("published_at")?,
    })
}

/// The newest published version of one app configuration version id, if it exists.
async fn app_config_version_in(tx: &mut Transaction<'_, Postgres>, config_version_id: Uuid) -> Result<Option<AppConfigVersion>> {
    let row = sqlx::query(&format!(
        "SELECT config_version_id, config_id, version, supersedes, currency, categories::text AS categories, source_reference, \
         to_char(created_at AT TIME ZONE 'UTC', {RFC3339_UTC}) AS created_at \
         FROM app_config_versions WHERE config_version_id = $1"
    ))
    .bind(config_version_id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else { return Ok(None) };
    let categories: Vec<CostCategory> = serde_json::from_str(&row.try_get::<String, _>("categories")?).map_err(StoreError::codec)?;
    Ok(Some(AppConfigVersion {
        config_version_id: row.try_get("config_version_id")?,
        config_id: row.try_get("config_id")?,
        version: version_from_i64(row.try_get("version")?)?,
        supersedes: row.try_get("supersedes")?,
        currency: row.try_get("currency")?,
        categories,
        source_reference: row.try_get("source_reference")?,
        created_at: Some(row.try_get("created_at")?),
    }))
}

// ---------------------------------------------------------------------------
// Per-project records read for a report
// ---------------------------------------------------------------------------

/// The current records one dependency answer resolves against.
struct ProjectRecords {
    project_version: u64,
    project: ProjectDocument,
    engineering_version: u64,
    engineering: EngineeringRecordDocument,
    manufacturing_version: u64,
    manufacturing: ManufacturingRecordDocument,
    feasibility: FeasibilityRecordDocument,
    analyses: Vec<RegisteredAnalysis>,
}

/// Read the current project record version and the exact engineering, manufacturing and feasibility
/// records the dependency answer resolves against, without locking any row.
async fn project_records_in(tx: &mut Transaction<'_, Postgres>, project_id: Uuid) -> Result<ProjectRecords> {
    let current_version: Option<i64> =
        sqlx::query_scalar("SELECT current_version FROM project_records WHERE project_id = $1").bind(project_id).fetch_optional(&mut **tx).await?;
    let project_version = version_from_i64(current_version.ok_or(StoreError::NotFound)?)?;
    let project = project_version_document(tx, project_id, project_version).await?;
    let (engineering_version, engineering) = engineering_record(tx, project_id, None).await?;
    let (manufacturing_version, manufacturing) = manufacturing_record(tx, project_id, None).await?;
    let (_, feasibility) = current_feasibility_record(tx, project_id).await?;
    let analyses = registered_analyses(tx, project_id).await?;
    Ok(ProjectRecords { project_version, project, engineering_version, engineering, manufacturing_version, manufacturing, feasibility, analyses })
}

/// The acknowledgement decisions of one project, keyed by `(kind, record, named version)`.
async fn acknowledged_decisions_in(tx: &mut Transaction<'_, Postgres>, project_id: Uuid) -> Result<BTreeSet<(String, Uuid, Uuid)>> {
    let rows: Vec<(String, Uuid, Uuid)> =
        sqlx::query_as("SELECT kind, record_id, to_version_id FROM dependency_decisions WHERE project_id = $1 AND action = 'acknowledged'")
            .bind(project_id)
            .fetch_all(&mut **tx)
            .await?;
    Ok(rows.into_iter().collect())
}

/// The bounded decision history of one project, newest first.
async fn decision_history_in(tx: &mut Transaction<'_, Postgres>, project_id: Uuid) -> Result<Vec<DependencyDecision>> {
    let limit = i64::try_from(MAX_DECISION_HISTORY).map_err(StoreError::codec)?;
    let rows = sqlx::query(&format!(
        "SELECT decision_id, project_id, sequence, kind, record_id, from_version_id, to_version_id, action, decided_by, \
         to_char(decided_at AT TIME ZONE 'UTC', {RFC3339_UTC}) AS decided_at \
         FROM dependency_decisions WHERE project_id = $1 ORDER BY sequence DESC LIMIT $2"
    ))
    .bind(project_id)
    .bind(limit)
    .fetch_all(&mut **tx)
    .await?;
    let mut decisions = Vec::with_capacity(rows.len());
    for row in &rows {
        let action = match row.try_get::<String, _>("action")?.as_str() {
            "acknowledged" => DependencyDecisionAction::Acknowledged,
            "adopted" => DependencyDecisionAction::Adopted,
            other => return Err(StoreError::Codec(format!("the stored decision action `{other}` is unknown"))),
        };
        decisions.push(DependencyDecision {
            decision_id: row.try_get("decision_id")?,
            project_id: row.try_get("project_id")?,
            sequence: version_from_i64(row.try_get("sequence")?)?,
            kind: shared_update_kind_from_db(row.try_get("kind")?)?,
            record_id: row.try_get("record_id")?,
            from_version_id: row.try_get("from_version_id")?,
            to_version_id: row.try_get("to_version_id")?,
            action,
            decided_by: row.try_get("decided_by")?,
            decided_at: row.try_get("decided_at")?,
        });
    }
    Ok(decisions)
}

// ---------------------------------------------------------------------------
// Dependency resolution
// ---------------------------------------------------------------------------

/// The dependency set of one project, sorted by `(kind, record id)` and bounded by the contract.
fn project_dependencies(
    records: &ProjectRecords,
    support: &SharedSupport,
    app_config: Option<&AppConfigVersion>,
    acknowledged: &BTreeSet<(String, Uuid, Uuid)>,
) -> Result<Vec<ProjectDependency>> {
    let usage = part_usage(records)?;
    // (kind, record id) -> referenced version number -> (version id, scope that references it)
    let mut referenced: BTreeMap<(String, Uuid), BTreeMap<u64, (Uuid, DependencyScope)>> = BTreeMap::new();
    material_dependencies(records, support, &usage, &mut referenced)?;
    rate_dependencies(records, support, &usage, &mut referenced)?;
    catalog_dependencies(records, support, &usage, &mut referenced)?;
    template_dependencies(records, support, &usage, &mut referenced)?;
    feasibility_dependencies(records, support, &mut referenced)?;
    app_config_dependency(support, app_config, &mut referenced)?;

    let mut dependencies = Vec::with_capacity(referenced.len());
    for ((kind_name, record_id), versions) in referenced {
        let Some(kind) = shared_update_kind_from_str(&kind_name) else {
            return Err(StoreError::Codec(format!("the resolved shared-update kind `{kind_name}` is unknown")));
        };
        let Some(newest) = support.latest_version(kind, record_id) else {
            return Err(StoreError::Invalid(format!(
                "dependency.{}: the project references the shared record `{record_id}` which has no published version",
                kind.as_str()
            )));
        };
        // The map is ordered by version number: the first entry is the oldest referenced version
        // and the last is the newest referenced one.
        let Some((oldest_version, (oldest_version_id, _))) = versions.iter().next() else { continue };
        let Some((newest_referenced_version, _)) = versions.iter().next_back() else { continue };
        if *newest_referenced_version > newest.version {
            return Err(StoreError::Invalid(format!(
                "dependency.{}: the project resolves version {newest_referenced_version} of record `{record_id}` which is newer than the newest published version {}",
                kind.as_str(),
                newest.version
            )));
        }
        // The scope is the union of every referencing part version and product of this record.
        let mut part_version_ids: BTreeSet<Uuid> = BTreeSet::new();
        let mut product_ids: BTreeSet<Uuid> = BTreeSet::new();
        for (_, scope) in versions.values() {
            part_version_ids.extend(&scope.part_version_ids);
            product_ids.extend(&scope.product_ids);
        }
        if *oldest_version_id == newest.version_id {
            // The project already resolves the newest published version: no pending dependency.
            continue;
        }
        let acknowledged = acknowledged.contains(&(kind.as_str().to_string(), record_id, newest.version_id));
        dependencies.push(ProjectDependency {
            kind,
            record_id,
            referenced_version_id: *oldest_version_id,
            referenced_version: *oldest_version,
            available_version_id: newest.version_id,
            available_version: newest.version,
            part_version_ids: part_version_ids.into_iter().collect(),
            product_ids: product_ids.into_iter().collect(),
            acknowledged,
        });
    }
    dependencies.sort_by_key(|dependency| (dependency.kind.as_str(), dependency.record_id));
    Ok(dependencies)
}

/// The scope one referenced version names.
#[derive(Clone, Default)]
struct DependencyScope {
    part_version_ids: BTreeSet<Uuid>,
    product_ids: BTreeSet<Uuid>,
}

/// Which products use each part version, under the default draft rule.
fn part_usage(records: &ProjectRecords) -> Result<BTreeMap<Uuid, BTreeSet<Uuid>>> {
    let mut usage: BTreeMap<Uuid, BTreeSet<Uuid>> = BTreeMap::new();
    for product in &records.project.products {
        let Some(revision) = scope_revision(product) else { continue };
        let occurrences = resolve_occurrences(&records.project, &records.engineering, &records.analyses, revision)?;
        let mut parts = BTreeSet::new();
        collect_occurrence_parts(&occurrences, &mut parts);
        for part_version_id in parts {
            usage.entry(part_version_id).or_default().insert(product.product_id);
        }
    }
    Ok(usage)
}

fn collect_occurrence_parts(occurrences: &[BaselineOccurrenceInput], parts: &mut BTreeSet<Uuid>) {
    for occurrence in occurrences {
        if let Some(part_version_id) = occurrence.part_version_id {
            parts.insert(part_version_id);
        }
        collect_occurrence_parts(&occurrence.children, parts);
    }
}

/// The revision a dependency scope describes: the product's newest `draft` revision, else its
/// `active` revision, else none. This is the default draft rule of a publication.
fn scope_revision(product: &solidbom_contract::Product) -> Option<&ProductRevision> {
    product
        .revisions
        .iter()
        .rev()
        .find(|revision| revision.state == ProductRevisionState::Draft)
        .or_else(|| product.active_revision_id.and_then(|id| product.revisions.iter().find(|revision| revision.product_revision_id == id)))
}

/// The default draft revision selection of every product that has a draft revision. A product
/// without one falls back to its active revision inside the SB-16 resolution, exactly like today.
fn default_draft_selections(project: &ProjectDocument) -> Vec<ProductRevisionSelection> {
    let mut selections: Vec<ProductRevisionSelection> = project
        .products
        .iter()
        .filter_map(|product| {
            product
                .revisions
                .iter()
                .rev()
                .find(|revision| revision.state == ProductRevisionState::Draft)
                .map(|revision| ProductRevisionSelection { product_id: product.product_id, product_revision_id: revision.product_revision_id })
        })
        .collect();
    selections.sort_by_key(|selection| selection.product_id);
    selections
}

fn material_dependencies(
    records: &ProjectRecords,
    support: &SharedSupport,
    usage: &BTreeMap<Uuid, BTreeSet<Uuid>>,
    referenced: &mut BTreeMap<(String, Uuid), BTreeMap<u64, (Uuid, DependencyScope)>>,
) -> Result<()> {
    let materials: BTreeMap<Uuid, (Uuid, u64)> =
        support.reference.materials.iter().map(|material| (material.material_version_id, (material.material_id, material.version))).collect();
    for part in &records.manufacturing.parts {
        if part.sourcing != Sourcing::Manufactured {
            continue;
        }
        let Some(material_version_id) = part.material.as_ref().and_then(|material| material.material_version_id) else { continue };
        let Some((material_id, version)) = materials.get(&material_version_id).copied() else {
            return Err(StoreError::Invalid(format!(
                "dependency.material: the manufactured part version `{}` selects the material version `{material_version_id}` which is not a published material version",
                part.part_version_id
            )));
        };
        let scope = referenced.entry((SharedUpdateKind::Material.as_str().to_string(), material_id)).or_default();
        let entry = scope.entry(version).or_insert_with(|| (material_version_id, DependencyScope::default()));
        entry.1.part_version_ids.insert(part.part_version_id);
        if let Some(products) = usage.get(&part.part_version_id) {
            entry.1.product_ids.extend(products.iter().copied());
        }
    }
    Ok(())
}

fn rate_dependencies(
    records: &ProjectRecords,
    support: &SharedSupport,
    usage: &BTreeMap<Uuid, BTreeSet<Uuid>>,
    referenced: &mut BTreeMap<(String, Uuid), BTreeMap<u64, (Uuid, DependencyScope)>>,
) -> Result<()> {
    let rates: BTreeMap<Uuid, (Uuid, u64)> =
        support.reference.rates.iter().map(|rate| (rate.rate_version_id, (rate.cost_center_id, rate.version))).collect();
    for routing in &records.manufacturing.routings {
        for operation in &routing.operations {
            let mut pinned = Vec::with_capacity(2);
            if let Some(cycle) = &operation.cycle
                && let Some(rate_version_id) = cycle.production_rate_version_id
            {
                pinned.push(rate_version_id);
            }
            if let Some(setup) = &operation.setup
                && let Some(rate_version_id) = setup.setup_rate_version_id
            {
                pinned.push(rate_version_id);
            }
            for rate_version_id in pinned {
                let Some((cost_center_id, version)) = rates.get(&rate_version_id).copied() else {
                    return Err(StoreError::Invalid(format!(
                        "dependency.cost_center_rate: the applied routing `{}` pins the rate version `{rate_version_id}` which is not a published rate version",
                        routing.routing_id
                    )));
                };
                let scope = referenced.entry((SharedUpdateKind::CostCenterRate.as_str().to_string(), cost_center_id)).or_default();
                let entry = scope.entry(version).or_insert_with(|| (rate_version_id, DependencyScope::default()));
                add_owner_scope(&mut entry.1, routing.owner, usage);
            }
        }
    }
    Ok(())
}

fn catalog_dependencies(
    records: &ProjectRecords,
    support: &SharedSupport,
    usage: &BTreeMap<Uuid, BTreeSet<Uuid>>,
    referenced: &mut BTreeMap<(String, Uuid), BTreeMap<u64, (Uuid, DependencyScope)>>,
) -> Result<()> {
    let components: BTreeMap<Uuid, (Uuid, u64)> =
        support.reference.components.iter().map(|component| (component.component_version_id, (component.component_id, component.version))).collect();
    for part in &records.engineering.parts {
        let Some(confirmed) = &part.catalog.confirmed else { continue };
        let Some((component_id, version)) = components.get(&confirmed.component_version_id).copied() else {
            return Err(StoreError::Invalid(format!(
                "dependency.catalog_component: the reviewed part version `{}` confirms the catalog version `{}` which is not a published catalog version",
                part.part_version_id, confirmed.component_version_id
            )));
        };
        if component_id != confirmed.component_id {
            return Err(StoreError::Codec(format!(
                "the confirmed catalog version `{}` belongs to component `{component_id}`, not `{}`",
                confirmed.component_version_id, confirmed.component_id
            )));
        }
        let scope = referenced.entry((SharedUpdateKind::CatalogComponent.as_str().to_string(), component_id)).or_default();
        let entry = scope.entry(version).or_insert_with(|| (confirmed.component_version_id, DependencyScope::default()));
        entry.1.part_version_ids.insert(part.part_version_id);
        if let Some(products) = usage.get(&part.part_version_id) {
            entry.1.product_ids.extend(products.iter().copied());
        }
    }
    Ok(())
}

fn template_dependencies(
    records: &ProjectRecords,
    support: &SharedSupport,
    usage: &BTreeMap<Uuid, BTreeSet<Uuid>>,
    referenced: &mut BTreeMap<(String, Uuid), BTreeMap<u64, (Uuid, DependencyScope)>>,
) -> Result<()> {
    for routing in &records.manufacturing.routings {
        let RoutingSource::Template(template) = routing.source else { continue };
        let Some(template_version_id) = support.template_versions.get(&(template.template_id, template.version)).copied() else {
            return Err(StoreError::Invalid(format!(
                "dependency.routing_template: the applied routing `{}` names template `{}` version {} which is not a published template version",
                routing.routing_id, template.template_id, template.version
            )));
        };
        let scope = referenced.entry((SharedUpdateKind::RoutingTemplate.as_str().to_string(), template.template_id)).or_default();
        let entry = scope.entry(template.version).or_insert_with(|| (template_version_id, DependencyScope::default()));
        add_owner_scope(&mut entry.1, routing.owner, usage);
    }
    Ok(())
}

fn feasibility_dependencies(
    records: &ProjectRecords,
    support: &SharedSupport,
    referenced: &mut BTreeMap<(String, Uuid), BTreeMap<u64, (Uuid, DependencyScope)>>,
) -> Result<()> {
    // Only the latest recorded assessment is the current evidence: the append-only record keeps
    // superseded assessments for history, and a superseded rule-table version is not a dependency.
    let Some(assessment) = records.feasibility.assessments.iter().max_by_key(|assessment| assessment.version) else {
        return Ok(());
    };
    let Some((rule_table_id, version)) = support.rule_versions.get(&assessment.rule_table_version_id).copied() else {
        return Err(StoreError::Invalid(format!(
            "dependency.feasibility_rule_table: the recorded assessment names the rule table version `{}` which is not a published rule table version",
            assessment.rule_table_version_id
        )));
    };
    let scope = referenced.entry((SharedUpdateKind::FeasibilityRuleTable.as_str().to_string(), rule_table_id)).or_default();
    let entry = scope.entry(version).or_insert_with(|| (assessment.rule_table_version_id, DependencyScope::default()));
    for component in &assessment.components {
        let component_id = component.evidence.component_id;
        let product_revision_id = records
            .engineering
            .component(component_id)
            .map(|component| component.product_revision_id)
            .unwrap_or(component.evidence.product_revision_id);
        if let Some(product) = records
            .project
            .products
            .iter()
            .find(|product| product.revisions.iter().any(|revision| revision.product_revision_id == product_revision_id))
        {
            entry.1.product_ids.insert(product.product_id);
        }
    }
    Ok(())
}

fn app_config_dependency(
    support: &SharedSupport,
    app_config: Option<&AppConfigVersion>,
    referenced: &mut BTreeMap<(String, Uuid), BTreeMap<u64, (Uuid, DependencyScope)>>,
) -> Result<()> {
    let Some(config) = app_config else { return Ok(()) };
    if support.latest_version(SharedUpdateKind::AppConfig, config.config_id).is_none() {
        return Err(StoreError::Invalid(format!(
            "dependency.app_config: the resolved app configuration version `{}` belongs to configuration `{}` which has no published version",
            config.config_version_id, config.config_id
        )));
    }
    referenced
        .entry((SharedUpdateKind::AppConfig.as_str().to_string(), config.config_id))
        .or_default()
        .entry(config.version)
        .or_insert_with(|| (config.config_version_id, DependencyScope::default()));
    Ok(())
}

fn add_owner_scope(scope: &mut DependencyScope, owner: CostOwner, usage: &BTreeMap<Uuid, BTreeSet<Uuid>>) {
    match owner {
        CostOwner::PartVersion { part_version_id } => {
            scope.part_version_ids.insert(part_version_id);
            if let Some(products) = usage.get(&part_version_id) {
                scope.product_ids.extend(products.iter().copied());
            }
        }
        CostOwner::ProductRoot { product_id } => {
            scope.product_ids.insert(product_id);
        }
    }
}

// ---------------------------------------------------------------------------
// Draft status and flagged scenarios
// ---------------------------------------------------------------------------

/// One newest draft result, reduced to what scenario flagging needs.
struct NewestDraft {
    calculation_id: Uuid,
    stale: bool,
}

/// The project's newest draft result with its stale markers, or `None` when every stored result is
/// accepted history. The newest draft result is the newest `baseline_calculations` row that no
/// `accepted_baselines` row pins: an activation snapshot is accepted evidence, not a draft.
async fn newest_draft_in(
    tx: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    records: &ProjectRecords,
    dependencies: &[ProjectDependency],
) -> Result<Option<DraftResultStatus>> {
    let row = sqlx::query(&format!(
        "SELECT calculation_id, project_version, engineering_version, manufacturing_version, input_hash, calculator_version, \
         outputs ->> 'state' AS state, to_char(created_at AT TIME ZONE 'UTC', {RFC3339_UTC}) AS created_at \
         FROM baseline_calculations bc WHERE bc.project_id = $1 \
         AND NOT EXISTS (SELECT 1 FROM accepted_baselines ab \
                         WHERE ab.project_id = bc.project_id AND ab.costing_calculation_id = bc.calculation_id) \
         ORDER BY bc.created_at DESC, bc.calculation_id DESC LIMIT 1"
    ))
    .bind(project_id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else { return Ok(None) };
    let project_version = version_from_i64(row.try_get("project_version")?)?;
    let engineering_version = version_from_i64(row.try_get("engineering_version")?)?;
    let manufacturing_version = version_from_i64(row.try_get("manufacturing_version")?)?;
    let created_at: String = row.try_get("created_at")?;
    let mut stale_reasons = Vec::new();
    if project_version != records.project_version {
        stale_reasons.push(DraftStaleReason::NewerProjectVersion);
    }
    if engineering_version != records.engineering_version {
        stale_reasons.push(DraftStaleReason::NewerEngineeringVersion);
    }
    if manufacturing_version != records.manufacturing_version {
        stale_reasons.push(DraftStaleReason::NewerManufacturingVersion);
    }
    // A shared-data update that changes a draft cost keeps the result stale. A feasibility rule
    // table is not a costing input, and an acknowledgement is not adoption, so neither flags it.
    if dependencies.iter().any(|dependency| costs_draft(dependency.kind)) {
        stale_reasons.push(DraftStaleReason::NewerSharedData);
    }
    // An adoption recorded after the draft result is a shared-data change too, even when no record
    // version moved (an adopted app configuration is the only pin the draft then uses).
    if adoption_recorded_after(tx, project_id, &created_at).await? {
        stale_reasons.push(DraftStaleReason::NewerSharedData);
    }
    stale_reasons.sort_by_key(|reason| reason.as_str());
    stale_reasons.dedup();
    let status = DraftResultStatus {
        calculation_id: row.try_get("calculation_id")?,
        project_version,
        engineering_version,
        manufacturing_version,
        state: result_state_from_db(row.try_get("state")?)?,
        input_hash: row.try_get("input_hash")?,
        calculator_version: row.try_get("calculator_version")?,
        stale: !stale_reasons.is_empty(),
        stale_reasons,
        created_at: Some(created_at),
    };
    status.validate().map_err(StoreError::codec)?;
    Ok(Some(status))
}

/// Whether one update kind is a draft costing input. A feasibility rule table is not: it is
/// flagged for the project but it never changes a draft cost.
fn costs_draft(kind: SharedUpdateKind) -> bool {
    kind != SharedUpdateKind::FeasibilityRuleTable
}

/// Whether the project recorded a shared-data adoption after one draft result was created.
async fn adoption_recorded_after(tx: &mut Transaction<'_, Postgres>, project_id: Uuid, created_at: &str) -> Result<bool> {
    let recorded: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM dependency_decisions \
         WHERE project_id = $1 AND action = 'adopted' AND decided_at > $2::timestamptz)",
    )
    .bind(project_id)
    .bind(created_at)
    .fetch_one(&mut **tx)
    .await?;
    Ok(recorded)
}

/// The bounded canonical dependency list: the first `MAX_PROJECT_DEPENDENCIES` rows after the sort,
/// with each row's targets truncated to the contract's target bound. The report keeps the true
/// pending count beside it, so a bounded list never claims to be the whole answer.
fn bounded_dependencies(dependencies: Vec<ProjectDependency>) -> Vec<ProjectDependency> {
    dependencies
        .into_iter()
        .take(MAX_PROJECT_DEPENDENCIES)
        .map(|mut dependency| {
            dependency.part_version_ids.truncate(MAX_DEPENDENCY_TARGETS);
            dependency.product_ids.truncate(MAX_DEPENDENCY_TARGETS);
            dependency
        })
        .collect()
}

/// The saved scenarios the newest draft result flags, sorted by scenario id. Only the latest saved
/// version per scenario is considered. The list is empty when the project has no draft result: an
/// accepted-only state flags nothing.
async fn flagged_scenarios_in(
    tx: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    newest: Option<NewestDraft>,
    current_project_version: u64,
) -> Result<Vec<FlaggedScenario>> {
    let Some(newest) = newest else { return Ok(Vec::new()) };
    let rows = sqlx::query(
        "SELECT DISTINCT ON (scenario_id) scenario_id, version, name, baseline_calculation_id, project_version \
         FROM commercial_scenarios WHERE project_id = $1 ORDER BY scenario_id, version DESC",
    )
    .bind(project_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut flagged = Vec::new();
    for row in &rows {
        let scenario_id: Uuid = row.try_get("scenario_id")?;
        let version = version_from_i64(row.try_get("version")?)?;
        let name: String = row.try_get("name")?;
        let baseline_calculation_id: Uuid = row.try_get("baseline_calculation_id")?;
        let pinned_project_version = version_from_i64(row.try_get("project_version")?)?;
        let mut stale_reasons = Vec::new();
        if baseline_calculation_id != newest.calculation_id {
            stale_reasons.push(solidbom_contract::CommercialStaleReason::new(
                SCENARIO_BASELINE_OUTDATED_CODE,
                "a newer baseline snapshot exists for this project",
            ));
        } else if newest.stale {
            stale_reasons.push(solidbom_contract::CommercialStaleReason::new(
                SCENARIO_BASELINE_OUTDATED_CODE,
                "the newest draft result is stale; re-publish it before comparing",
            ));
        }
        if pinned_project_version != current_project_version {
            stale_reasons.push(solidbom_contract::CommercialStaleReason::new(
                SCENARIO_PROJECT_OUTDATED_CODE,
                "a newer project record version exists than the scenario pinned",
            ));
        }
        if stale_reasons.is_empty() {
            continue;
        }
        stale_reasons.sort_by(|left, right| left.code.cmp(&right.code));
        flagged.push(FlaggedScenario { scenario_id, version, name, baseline_calculation_id, pinned_project_version, stale_reasons });
    }
    flagged.sort_by_key(|scenario| scenario.scenario_id);
    flagged.truncate(MAX_FLAGGED_SCENARIOS);
    Ok(flagged)
}

/// The answer of one publication: the snapshot summary, the dependencies still pending afterwards
/// and the scenarios the new result flags. The inserted snapshot is the newest draft result; it is
/// stale exactly when a pending dependency changes a draft cost.
async fn draft_publication_in(
    tx: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    calculation: BaselineCalculationSummary,
    calculation_id: Uuid,
) -> Result<DraftPublication> {
    let records = project_records_in(tx, project_id).await?;
    let support = shared_support_in(tx).await?;
    let acknowledged = acknowledged_decisions_in(tx, project_id).await?;
    let app_config = app_config_for_project(tx, project_id, support.reference.app_config.as_ref()).await?;
    let dependencies = project_dependencies(&records, &support, app_config.as_ref(), &acknowledged)?;
    let stale = dependencies.iter().any(|dependency| costs_draft(dependency.kind));
    let flagged_scenarios = flagged_scenarios_in(tx, project_id, Some(NewestDraft { calculation_id, stale }), records.project_version).await?;
    Ok(DraftPublication { calculation, dependencies: bounded_dependencies(dependencies), flagged_scenarios })
}

/// The summary of one inserted snapshot, as a publication returns it.
fn snapshot_summary(snapshot: &solidbom_contract::BaselineSnapshot) -> BaselineCalculationSummary {
    BaselineCalculationSummary {
        calculation_id: snapshot.calculation_id,
        project_id: snapshot.project_id,
        project_version: snapshot.project_version,
        engineering_version: snapshot.engineering_version,
        manufacturing_version: snapshot.manufacturing_version,
        calculator_version: snapshot.calculator_version.clone(),
        state: snapshot.outputs.state,
        created_at: snapshot.created_at.clone(),
    }
}

// ---------------------------------------------------------------------------
// Adoption repins
// ---------------------------------------------------------------------------

/// One explicit decision about to be recorded.
struct PlannedDecision {
    kind: SharedUpdateKind,
    record_id: Uuid,
    from_version_id: Option<Uuid>,
    to_version_id: Uuid,
}

/// Whether one identical decision row is already recorded.
async fn decision_recorded(
    tx: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    kind: SharedUpdateKind,
    record_id: Uuid,
    to_version_id: Uuid,
    action: DependencyDecisionAction,
) -> Result<bool> {
    let found: Option<Uuid> = sqlx::query_scalar(
        "SELECT decision_id FROM dependency_decisions \
         WHERE project_id = $1 AND kind = $2 AND record_id = $3 AND to_version_id = $4 AND action = $5 LIMIT 1",
    )
    .bind(project_id)
    .bind(kind.as_str())
    .bind(record_id)
    .bind(to_version_id)
    .bind(action.as_str())
    .fetch_optional(&mut **tx)
    .await?;
    Ok(found.is_some())
}

/// Repin every manufactured part whose current material version belongs to the named logical
/// record. A stored reference to an unknown material version is corrupt state and refuses the
/// whole adoption; the document is otherwise untouched.
fn repin_material(document: &mut ManufacturingRecordDocument, record_id: Uuid, to_version_id: Uuid, support: &SharedSupport) -> Result<bool> {
    let materials: BTreeMap<Uuid, Uuid> =
        support.reference.materials.iter().map(|material| (material.material_version_id, material.material_id)).collect();
    let mut changed = false;
    for part in &mut document.parts {
        if part.sourcing != Sourcing::Manufactured {
            continue;
        }
        let Some(material) = &mut part.material else { continue };
        let Some(material_version_id) = material.material_version_id else { continue };
        let Some(material_id) = materials.get(&material_version_id).copied() else {
            return Err(StoreError::Invalid(format!(
                "dependency_decisions.parts: the manufactured part version `{}` selects the material version `{material_version_id}` which is not a published material version",
                part.part_version_id
            )));
        };
        if material_id == record_id {
            material.material_version_id = Some(to_version_id);
            changed = true;
        }
    }
    Ok(changed)
}

/// Repin every routing operation cycle/setup rate whose current rate version belongs to the named
/// logical cost center.
fn repin_rates(document: &mut ManufacturingRecordDocument, record_id: Uuid, to_version_id: Uuid, support: &SharedSupport) -> Result<bool> {
    let rates: BTreeMap<Uuid, Uuid> = support.reference.rates.iter().map(|rate| (rate.rate_version_id, rate.cost_center_id)).collect();
    let mut changed = false;
    for routing in &mut document.routings {
        for operation in &mut routing.operations {
            if let Some(cycle) = &mut operation.cycle
                && let Some(rate_version_id) = cycle.production_rate_version_id
            {
                let Some(cost_center_id) = rates.get(&rate_version_id).copied() else {
                    return Err(corrupt_rate_version(rate_version_id));
                };
                if cost_center_id == record_id {
                    cycle.production_rate_version_id = Some(to_version_id);
                    changed = true;
                }
            }
            if let Some(setup) = &mut operation.setup
                && let Some(rate_version_id) = setup.setup_rate_version_id
            {
                let Some(cost_center_id) = rates.get(&rate_version_id).copied() else {
                    return Err(corrupt_rate_version(rate_version_id));
                };
                if cost_center_id == record_id {
                    setup.setup_rate_version_id = Some(to_version_id);
                    changed = true;
                }
            }
        }
    }
    Ok(changed)
}

fn corrupt_rate_version(rate_version_id: Uuid) -> StoreError {
    StoreError::Invalid(format!(
        "dependency_decisions.routings: the applied routing pins the rate version `{rate_version_id}` which is not a published rate version"
    ))
}

/// Replace the confirmed catalog reference of every reviewed part that confirmed the named logical
/// component with the values of the exact newly published version; the project override stays.
fn repin_catalog(document: &mut EngineeringRecordDocument, record_id: Uuid, component: &CatalogComponentVersion, confirmed_by: &str) -> bool {
    let mut changed = false;
    for part in &mut document.parts {
        let Some(confirmed) = &part.catalog.confirmed else { continue };
        if confirmed.component_id != record_id {
            continue;
        }
        part.catalog.confirmed = Some(solidbom_contract::ConfirmedCatalogReference {
            component_id: component.component_id,
            component_version_id: component.component_version_id,
            version: component.version,
            name: component.name.clone(),
            specification: component.specification.clone(),
            price: component.price,
            price_unit: component.price_unit,
            currency: component.currency.clone(),
            confirmed_by: confirmed_by.to_string(),
        });
        changed = true;
    }
    changed
}

// ---------------------------------------------------------------------------
// Locking and refusal helpers
// ---------------------------------------------------------------------------

/// One append-only record head a transaction reads its generation from.
#[derive(Clone, Copy)]
enum RecordHead {
    Engineering,
    Manufacturing,
}

/// The current generation of one record head inside the caller's transaction: 0 when the project has
/// no such record yet.
async fn current_generation_in(tx: &mut Transaction<'_, Postgres>, project_id: Uuid, head: RecordHead) -> Result<u64> {
    // The table names are fixed literals selected by the enum, never client input.
    let table = match head {
        RecordHead::Engineering => "engineering_records",
        RecordHead::Manufacturing => "manufacturing_records",
    };
    let statement = format!("SELECT current_version FROM {table} WHERE project_id = $1");
    let current: Option<i64> = sqlx::query_scalar(&statement).bind(project_id).fetch_optional(&mut **tx).await?;
    match current {
        Some(current) => version_from_i64(current),
        None => Ok(0),
    }
}

/// Lock the engineering and manufacturing record heads `FOR SHARE` for the rest of the transaction,
/// so a concurrent record write cannot supersede the versions a publication reads. The heads are
/// locked in the one order every SB-20 transaction uses: engineering, then manufacturing.
async fn lock_record_heads_for_share(tx: &mut Transaction<'_, Postgres>, project_id: Uuid) -> Result<()> {
    // The table names are fixed literals, never client input.
    for table in ["engineering_records", "manufacturing_records"] {
        let statement = format!("SELECT current_version FROM {table} WHERE project_id = $1 FOR SHARE");
        sqlx::query(&statement).bind(project_id).fetch_optional(&mut **tx).await?;
    }
    Ok(())
}

/// Build the store error for one typed dependency refusal. The version 1 error code decides the
/// class: `404` keeps the generic not-found body, `409` and `422` keep the refusal message.
fn refusal_error(refusal: DependencyRefusal) -> StoreError {
    match refusal.error_code() {
        solidbom_contract::ApiErrorCode::NotFound => StoreError::NotFound,
        solidbom_contract::ApiErrorCode::Invalid => StoreError::Invalid(refusal.message()),
        _ => StoreError::Conflict(refusal.message()),
    }
}

/// The wire name of one shared-update kind, for the query filter.
pub(crate) fn shared_update_kind_from_str(text: &str) -> Option<SharedUpdateKind> {
    match text {
        "material" => Some(SharedUpdateKind::Material),
        "cost_center_rate" => Some(SharedUpdateKind::CostCenterRate),
        "catalog_component" => Some(SharedUpdateKind::CatalogComponent),
        "routing_template" => Some(SharedUpdateKind::RoutingTemplate),
        "feasibility_rule_table" => Some(SharedUpdateKind::FeasibilityRuleTable),
        "app_config" => Some(SharedUpdateKind::AppConfig),
        _ => None,
    }
}

/// The stored wire name of one shared-update kind.
fn shared_update_kind_from_db(text: String) -> Result<SharedUpdateKind> {
    shared_update_kind_from_str(&text).ok_or_else(|| StoreError::Codec(format!("the stored shared-update kind `{text}` is unknown")))
}
