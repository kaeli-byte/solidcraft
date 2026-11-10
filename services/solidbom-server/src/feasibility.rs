//! Feasibility rules and justified exceptions storage (package SB-14, additive to contract version 2).
//!
//! Two append-only stores sit beside the project record:
//!
//! - **Rule tables** are Administrator-maintained version chains. Publishing a version keeps its
//!   structured rules, their source references and the publisher; an earlier version is never
//!   edited, so a recorded finding always names the exact rules it was evaluated with.
//! - **A project's feasibility record** keeps the recorded assessments and the current justified
//!   exceptions. Recording an assessment verifies the exact reviewed component binding against the
//!   engineering review, evaluates the pure contract evaluator against one exact rule version, and
//!   appends the evidence and its computed findings. Accepting an exception verifies the recorded
//!   non-passing finding and the unchanged supporting geometry, then stores the engineer's
//!   justification.
//!
//! Issues are computed on read. A missing measurement, a missing connector, a missing limit and a
//! conflicting limit are never stored as a pass, and an accepted exception whose supporting inputs
//! changed becomes `review_required` instead of silently staying accepted.

use solidbom_contract::{
    AcceptFeasibilityException, AssessedComponent, CloseFeasibilityException, ContractError, CreateFeasibilityRuleTableVersion,
    EngineeringRecordDocument, ExceptionClosure, FeasibilityAssessment, FeasibilityAssessmentOutcome, FeasibilityException, FeasibilityFact,
    FeasibilityReceipt, FeasibilityRecordDocument, FeasibilityRecordView, FeasibilityRule, FeasibilityRuleChange, FeasibilityRuleTablePublished,
    FeasibilityRuleTableVersion, FeasibilityRulesView, FeasibilityWriteOutcome, Identity, MAX_FEASIBILITY_CHANGES, ReviewedComponent,
    assessment_view, evaluate_feasibility,
};
use sqlx::postgres::PgRow;
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

use crate::Result;
use crate::store::{Store, StoreError, insert_request, lock_request, lookup_request, payload_hash};

/// The prefix of the feasibility operations recorded in the shared request log.
const OPERATION_PREFIX: &str = "feasibility";

/// The RFC 3339 UTC spelling the service stores into a reply, so a client never parses a locale
/// timestamp. `MS` keeps milliseconds and the literal `Z` marks UTC.
const RFC3339_UTC: &str = "'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"'";

impl Store {
    /// Publish the next version of a feasibility rule table. `expected_version` 0 creates version 1;
    /// any other value must equal the table's current latest version. The new version and its
    /// change fact commit in one transaction.
    pub async fn publish_feasibility_rules(
        &self,
        rule_table_id: Uuid,
        request: &CreateFeasibilityRuleTableVersion,
        changed_by: &str,
    ) -> Result<FeasibilityRuleTablePublished> {
        if request.rule_table_id != rule_table_id {
            return Err(StoreError::Invalid("the version belongs to a different rule table".into()));
        }
        request.validate().map_err(invalid)?;
        let payload = payload_hash(&format!("{OPERATION_PREFIX}.rules"), &(rule_table_id, request))?;
        let mut tx = self.pool().begin().await?;
        lock_request(&mut tx, &request.request_id).await?;
        if let Some(published) = replay::<FeasibilityRuleTablePublished>(&mut tx, &request.request_id, &payload).await? {
            tx.commit().await?;
            return Ok(published);
        }
        // Serialise publishes of one table so a concurrent publish cannot append two versions with
        // one number.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("feasibility_rules:{rule_table_id}"))
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO feasibility_rule_tables (rule_table_id, current_version) VALUES ($1, 0) ON CONFLICT (rule_table_id) DO NOTHING")
            .bind(rule_table_id)
            .execute(&mut *tx)
            .await?;
        let current: i64 = sqlx::query_scalar("SELECT current_version FROM feasibility_rule_tables WHERE rule_table_id = $1 FOR UPDATE")
            .bind(rule_table_id)
            .fetch_one(&mut *tx)
            .await?;
        let current = u64::try_from(current).map_err(StoreError::codec)?;
        if current != request.expected_version {
            return Err(StoreError::Conflict(format!("the rule table is at version {current}, not {}", request.expected_version)));
        }
        let supersedes: Option<Uuid> = if current == 0 {
            None
        } else {
            sqlx::query_scalar("SELECT rule_table_version_id FROM feasibility_rule_versions WHERE rule_table_id = $1 AND version = $2")
                .bind(rule_table_id)
                .bind(version_to_i64(current)?)
                .fetch_optional(&mut *tx)
                .await?
        };
        let version = current + 1;
        let version_id = Uuid::new_v4();
        let rules = serde_json::to_string(&request.rules).map_err(StoreError::codec)?;
        let created_at: String = sqlx::query_scalar(&format!(
            "INSERT INTO feasibility_rule_versions \
             (rule_table_version_id, rule_table_id, version, supersedes, name, rules, source_reference, created_by) \
             VALUES ($1, $2, $3, $4, $5, $6::jsonb, $7, $8) \
             RETURNING to_char(created_at AT TIME ZONE 'UTC', {RFC3339_UTC})"
        ))
        .bind(version_id)
        .bind(rule_table_id)
        .bind(version_to_i64(version)?)
        .bind(supersedes)
        .bind(&request.name)
        .bind(rules)
        .bind(&request.source_reference)
        .bind(changed_by)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query("UPDATE feasibility_rule_tables SET current_version = $2 WHERE rule_table_id = $1")
            .bind(rule_table_id)
            .bind(version_to_i64(version)?)
            .execute(&mut *tx)
            .await?;
        let table = FeasibilityRuleTableVersion {
            rule_table_id,
            rule_table_version_id: version_id,
            version,
            supersedes,
            name: request.name.clone(),
            rules: request.rules.clone(),
            source_reference: request.source_reference.clone(),
            created_by: Some(changed_by.to_string()),
            created_at: Some(created_at.clone()),
        };
        let change = FeasibilityRuleChange {
            rule_table_id,
            rule_table_version_id: version_id,
            version,
            previous_version_id: supersedes,
            summary: format!("version {version} publishes {} rules", table.rules.len()),
            changed_by: changed_by.to_string(),
            changed_at: created_at,
        };
        let published = FeasibilityRuleTablePublished { rule_table: table, change };
        insert_request(&mut tx, &request.request_id, &format!("{OPERATION_PREFIX}.rules"), &payload, &published).await?;
        tx.commit().await?;
        Ok(published)
    }

    /// Every published rule-table version, oldest version first per table, with the newest change
    /// facts. An Engineer reads the rules; only an Administrator publishes them.
    pub async fn feasibility_rules_view(&self, identity: &Identity) -> Result<FeasibilityRulesView> {
        let mut tx = self.pool().begin().await?;
        let rows = sqlx::query(&format!(
            "SELECT rule_table_id, rule_table_version_id, version, supersedes, name, rules::text AS rules, source_reference, created_by, \
             to_char(created_at AT TIME ZONE 'UTC', {RFC3339_UTC}) AS created_at \
             FROM feasibility_rule_versions ORDER BY rule_table_id, version"
        ))
        .fetch_all(&mut *tx)
        .await?;
        let mut rule_tables = Vec::with_capacity(rows.len());
        for row in &rows {
            rule_tables.push(rule_table_from_row(row)?);
        }
        tx.commit().await?;
        let mut changes: Vec<FeasibilityRuleChange> = rule_tables
            .iter()
            .map(|table| FeasibilityRuleChange {
                rule_table_id: table.rule_table_id,
                rule_table_version_id: table.rule_table_version_id,
                version: table.version,
                previous_version_id: table.supersedes,
                summary: format!("version {} publishes {} rules", table.version, table.rules.len()),
                changed_by: table.created_by.clone().unwrap_or_default(),
                changed_at: table.created_at.clone().unwrap_or_default(),
            })
            .collect();
        // Newest first; the stored timestamps are fixed-width RFC 3339 UTC strings.
        changes.sort_by(|left, right| {
            right
                .changed_at
                .cmp(&left.changed_at)
                .then_with(|| right.version.cmp(&left.version))
                .then_with(|| right.rule_table_id.cmp(&left.rule_table_id))
        });
        changes.truncate(MAX_FEASIBILITY_CHANGES);
        let view = FeasibilityRulesView { identity: identity.clone(), rule_tables, changes };
        view.validate().map_err(invalid)?;
        Ok(view)
    }

    /// Record one feasibility assessment: verify every named reviewed component against the exact
    /// engineering review, evaluate the evidence against one exact rule-table version, and append
    /// the evidence and its computed findings. Nothing is inferred from a prior assessment.
    pub async fn record_feasibility_assessment(
        &self,
        project_id: Uuid,
        request: &solidbom_contract::RecordFeasibilityAssessment,
        reviewer: &str,
    ) -> Result<FeasibilityAssessmentOutcome> {
        request.validate().map_err(invalid)?;
        let payload = payload_hash(&format!("{OPERATION_PREFIX}.assessment"), &(project_id, request))?;
        let mut tx = self.pool().begin().await?;
        lock_request(&mut tx, &request.request_id).await?;
        lock_feasibility(&mut tx, project_id).await?;
        if let Some(outcome) = replay::<FeasibilityAssessmentOutcome>(&mut tx, &request.request_id, &payload).await? {
            tx.commit().await?;
            return Ok(outcome);
        }
        ensure_project_exists(&mut tx, project_id).await?;
        let (current, mut document) = locked_current(&mut tx, project_id).await?;
        if current != request.expected_version {
            return Err(version_conflict(current, request.expected_version));
        }
        let (engineering_version, engineering) = current_engineering(&mut tx, project_id).await?;
        let rule_table = rule_table_version(&mut tx, request.rule_table_version_id).await?;

        // The review is the authority on which reviewed components exist and what geometry they
        // contain; the evidence must describe exactly those components.
        let mut components = Vec::with_capacity(request.components.len());
        for evidence in &request.components {
            let reviewed = engineering.component(evidence.component_id).ok_or_else(|| {
                StoreError::Invalid("the project has no such reviewed component; review the component before assessing feasibility".into())
            })?;
            verify_review_binding(reviewed, evidence)?;
            components.push(AssessedComponent {
                evidence: evidence.clone(),
                part_version_id: reviewed.part_version_id,
                reviewed_geometry: reviewed.geometry.clone(),
            });
        }
        let findings = evaluate_feasibility(&rule_table, &request.components).map_err(invalid)?;
        let created_at = timestamp(&mut tx).await?;
        let version = current + 1;
        let finding_count = u64::try_from(findings.len()).map_err(StoreError::codec)?;
        let non_passing_finding_count = u64::try_from(findings.iter().filter(|finding| finding.state.blocks()).count()).map_err(StoreError::codec)?;
        let assessment = FeasibilityAssessment {
            version,
            rule_table_version_id: request.rule_table_version_id,
            engineering_record_version: engineering_version,
            created_by: reviewer.to_string(),
            created_at,
            components,
            findings,
        };
        document.assessments.push(assessment.clone());
        document.validate().map_err(invalid)?;
        let facts = vec![FeasibilityFact::AssessmentRecorded { version, finding_count, non_passing_finding_count }];
        ensure_record_row(&mut tx, project_id).await?;
        append_document(&mut tx, project_id, version, &document).await?;
        advance_version(&mut tx, project_id, version).await?;
        let exceptions = document.exception_views(&engineering);
        let assessment = assessment_view(&assessment, &exceptions);
        let issues = document.issues(&engineering);
        let outcome = FeasibilityAssessmentOutcome {
            receipt: FeasibilityReceipt { request_id: request.request_id.clone(), project_id, version },
            assessment,
            issues,
            facts,
        };
        insert_request(&mut tx, &request.request_id, &format!("{OPERATION_PREFIX}.assessment"), &payload, &outcome).await?;
        tx.commit().await?;
        Ok(outcome)
    }

    /// Accept a justified exception for one recorded non-passing finding. The service verifies the
    /// finding, the authenticated reviewer and the unchanged supporting geometry; a pass or a
    /// missing measurement cannot be excepted, and a conflict or missing limit follows the same
    /// recorded-justification route.
    pub async fn accept_feasibility_exception(
        &self,
        project_id: Uuid,
        request: &AcceptFeasibilityException,
        reviewer: &str,
    ) -> Result<FeasibilityWriteOutcome> {
        request.validate().map_err(invalid)?;
        let payload = payload_hash(&format!("{OPERATION_PREFIX}.exception"), &(project_id, request))?;
        let mut tx = self.pool().begin().await?;
        lock_request(&mut tx, &request.request_id).await?;
        lock_feasibility(&mut tx, project_id).await?;
        if let Some(outcome) = replay::<FeasibilityWriteOutcome>(&mut tx, &request.request_id, &payload).await? {
            tx.commit().await?;
            return Ok(outcome);
        }
        ensure_project_exists(&mut tx, project_id).await?;
        let (current, mut document) = locked_current(&mut tx, project_id).await?;
        if current != request.expected_version {
            return Err(version_conflict(current, request.expected_version));
        }
        let (_, engineering) = current_engineering(&mut tx, project_id).await?;
        // An accepted decision always refers to the assessment a later read treats as current.
        let latest = document.latest_assessment().map(|assessment| assessment.version);
        if latest != Some(request.assessment_version) {
            return Err(StoreError::Invalid("accept the exception against the latest recorded assessment; assess the current review again".into()));
        }
        let assessment = document
            .assessment(request.assessment_version)
            .cloned()
            .ok_or_else(|| StoreError::Invalid("the project has no such feasibility assessment version".into()))?;
        let finding =
            assessment.finding(&request.finding_key).cloned().ok_or_else(|| StoreError::Invalid("the assessment has no such finding".into()))?;
        if !finding.exception_is_available() {
            return Err(StoreError::Invalid(
                "an exception covers a failed, conflicting or missing-limit finding, not a pass or a missing measurement".into(),
            ));
        }
        let component = assessment
            .component(request.finding_key.component_id)
            .cloned()
            .ok_or_else(|| StoreError::Invalid("the assessment has no such assessed component".into()))?;
        // The accepted decision names the exact supporting geometry; a changed review means the
        // engineer must assess the current review again before accepting.
        if !component.matches_review(&engineering) {
            return Err(StoreError::Invalid(
                "the supporting geometry or part association changed; assess the current review again before accepting an exception".into(),
            ));
        }
        let accepted_at = timestamp(&mut tx).await?;
        let exception = FeasibilityException {
            exception_id: Uuid::new_v4(),
            finding_key: request.finding_key.clone(),
            justification: request.justification.clone(),
            accepted_by: reviewer.to_string(),
            accepted_at,
            assessment_version: request.assessment_version,
            rule_table_version_id: assessment.rule_table_version_id,
            component,
            finding,
            closed: None,
        };
        let replaced =
            document.exceptions.iter().find(|existing| existing.finding_key == exception.finding_key).map(|existing| existing.exception_id);
        document.exceptions.retain(|existing| existing.finding_key != exception.finding_key);
        document.exceptions.push(exception.clone());
        document.validate().map_err(invalid)?;
        let version = current + 1;
        let facts =
            vec![FeasibilityFact::ExceptionAccepted { exception_id: exception.exception_id, finding_key: exception.finding_key.clone(), replaced }];
        ensure_record_row(&mut tx, project_id).await?;
        append_document(&mut tx, project_id, version, &document).await?;
        advance_version(&mut tx, project_id, version).await?;
        let outcome = FeasibilityWriteOutcome { receipt: FeasibilityReceipt { request_id: request.request_id.clone(), project_id, version }, facts };
        insert_request(&mut tx, &request.request_id, &format!("{OPERATION_PREFIX}.exception"), &payload, &outcome).await?;
        tx.commit().await?;
        Ok(outcome)
    }

    /// Close a justified exception that no longer applies: the latest recorded assessment's finding
    /// for its key now passes or is no longer recorded. The service records the closing Engineer, the
    /// timestamp and the required reason, so the obsolete decision has an audit trail instead of
    /// silently blocking readiness; a finding that still does not pass must be re-accepted rather
    /// than closed.
    pub async fn close_feasibility_exception(
        &self,
        project_id: Uuid,
        request: &CloseFeasibilityException,
        reviewer: &str,
    ) -> Result<FeasibilityWriteOutcome> {
        request.validate().map_err(invalid)?;
        let payload = payload_hash(&format!("{OPERATION_PREFIX}.exception_closure"), &(project_id, request))?;
        let mut tx = self.pool().begin().await?;
        lock_request(&mut tx, &request.request_id).await?;
        lock_feasibility(&mut tx, project_id).await?;
        if let Some(outcome) = replay::<FeasibilityWriteOutcome>(&mut tx, &request.request_id, &payload).await? {
            tx.commit().await?;
            return Ok(outcome);
        }
        ensure_project_exists(&mut tx, project_id).await?;
        let (current, mut document) = locked_current(&mut tx, project_id).await?;
        if current != request.expected_version {
            return Err(version_conflict(current, request.expected_version));
        }
        let exception = document
            .exceptions
            .iter()
            .find(|exception| exception.exception_id == request.exception_id)
            .cloned()
            .ok_or_else(|| StoreError::Invalid("the project has no such feasibility exception".into()))?;
        if exception.closed.is_some() {
            return Err(StoreError::Invalid("the feasibility exception is already closed".into()));
        }
        // Only an obsolete decision may be closed; a finding that still does not pass must be
        // re-accepted instead of silently losing its disposition.
        let still_required = document
            .latest_assessment()
            .is_some_and(|assessment| assessment.finding(&exception.finding_key).is_some_and(|finding| finding.state.blocks()));
        if still_required {
            return Err(StoreError::Invalid("the finding still does not pass; re-accept the exception instead of closing it".into()));
        }
        let closed_at = timestamp(&mut tx).await?;
        let stored = document
            .exceptions
            .iter_mut()
            .find(|stored| stored.exception_id == request.exception_id)
            .ok_or_else(|| StoreError::Invalid("the feasibility exception disappeared while closing it".into()))?;
        stored.closed = Some(ExceptionClosure { closed_by: reviewer.to_string(), closed_at, reason: request.reason.clone() });
        let finding_key = exception.finding_key.clone();
        document.validate().map_err(invalid)?;
        let version = current + 1;
        let facts = vec![FeasibilityFact::ExceptionClosed { exception_id: exception.exception_id, finding_key }];
        ensure_record_row(&mut tx, project_id).await?;
        append_document(&mut tx, project_id, version, &document).await?;
        advance_version(&mut tx, project_id, version).await?;
        let outcome = FeasibilityWriteOutcome { receipt: FeasibilityReceipt { request_id: request.request_id.clone(), project_id, version }, facts };
        insert_request(&mut tx, &request.request_id, &format!("{OPERATION_PREFIX}.exception_closure"), &payload, &outcome).await?;
        tx.commit().await?;
        Ok(outcome)
    }

    /// The current feasibility record of a project: the latest assessment with current
    /// dispositions, the current exceptions with their state, and the computed readiness issues. A
    /// project with no recorded decision reads as version 0 with a not-assessed issue.
    pub async fn feasibility_record_view(&self, project_id: Uuid) -> Result<FeasibilityRecordView> {
        let mut tx = self.pool().begin().await?;
        ensure_project_exists(&mut tx, project_id).await?;
        let (version, document) = read_current(&mut tx, project_id).await?;
        let (_, engineering) = current_engineering(&mut tx, project_id).await?;
        tx.commit().await?;
        Ok(document.record_view(version, &engineering))
    }

    /// One retained feasibility record version. The retained evidence and findings are the version's
    /// own; dispositions and exception states are computed against the current engineering review.
    pub async fn feasibility_record_version(&self, project_id: Uuid, version: u64) -> Result<FeasibilityRecordView> {
        let mut tx = self.pool().begin().await?;
        ensure_project_exists(&mut tx, project_id).await?;
        let text: String = sqlx::query_scalar("SELECT document::text FROM feasibility_record_versions WHERE project_id = $1 AND version = $2")
            .bind(project_id)
            .bind(version_to_i64(version)?)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(StoreError::NotFound)?;
        let document: FeasibilityRecordDocument = serde_json::from_str(&text).map_err(StoreError::codec)?;
        document.validate().map_err(StoreError::codec)?;
        let (_, engineering) = current_engineering(&mut tx, project_id).await?;
        tx.commit().await?;
        Ok(document.record_view(version, &engineering))
    }
}

fn invalid(error: ContractError) -> StoreError {
    StoreError::Invalid(error.to_string())
}

fn version_to_i64(version: u64) -> Result<i64> {
    i64::try_from(version).map_err(StoreError::codec)
}

fn version_conflict(current: u64, expected: u64) -> StoreError {
    StoreError::Conflict(format!("the feasibility record is at version {current}, not {expected}"))
}

/// Verify that the submitted evidence describes exactly the reviewed component: same revision and
/// exact analysis. The reviewed body set is the service's own evidence, never the payload's.
fn verify_review_binding(reviewed: &ReviewedComponent, evidence: &solidbom_contract::FeasibilityComponentEvidence) -> Result<()> {
    if reviewed.product_revision_id != evidence.product_revision_id {
        return Err(StoreError::Invalid("the evidence names a different product revision than the reviewed component".into()));
    }
    if reviewed.analysis_key != evidence.analysis_key {
        return Err(StoreError::Invalid(
            "the evidence names a different analysis than the reviewed component; review the exact source before assessing".into(),
        ));
    }
    Ok(())
}

async fn replay<T: serde::de::DeserializeOwned>(tx: &mut Transaction<'_, Postgres>, request_id: &str, payload: &str) -> Result<Option<T>> {
    if let Some((stored, response)) = lookup_request(tx, request_id).await? {
        if stored != payload {
            return Err(StoreError::Conflict("the request ID was reused with a different payload".into()));
        }
        return serde_json::from_value(response).map(Some).map_err(StoreError::codec);
    }
    Ok(None)
}

/// Serialise feasibility writes of one project, including the first write that creates the record
/// row.
async fn lock_feasibility(tx: &mut Transaction<'_, Postgres>, project_id: Uuid) -> Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))").bind(format!("feasibility_record:{project_id}")).execute(&mut **tx).await?;
    Ok(())
}

async fn ensure_project_exists(tx: &mut Transaction<'_, Postgres>, project_id: Uuid) -> Result<()> {
    let found: Option<Uuid> =
        sqlx::query_scalar("SELECT project_id FROM project_records WHERE project_id = $1").bind(project_id).fetch_optional(&mut **tx).await?;
    found.map(|_| ()).ok_or(StoreError::NotFound)
}

/// The current engineering review of the project, or the empty review when none is recorded. The
/// feasibility evidence is verified against these reviewed components.
async fn current_engineering(tx: &mut Transaction<'_, Postgres>, project_id: Uuid) -> Result<(u64, EngineeringRecordDocument)> {
    let current: Option<i64> = sqlx::query_scalar("SELECT current_version FROM engineering_records WHERE project_id = $1")
        .bind(project_id)
        .fetch_optional(&mut **tx)
        .await?;
    match current {
        None => Ok((0, EngineeringRecordDocument::empty(project_id))),
        Some(current) => {
            let current = u64::try_from(current).map_err(StoreError::codec)?;
            let text: String = sqlx::query_scalar("SELECT document::text FROM engineering_record_versions WHERE project_id = $1 AND version = $2")
                .bind(project_id)
                .bind(version_to_i64(current)?)
                .fetch_one(&mut **tx)
                .await?;
            let document: EngineeringRecordDocument = serde_json::from_str(&text).map_err(StoreError::codec)?;
            document.validate().map_err(StoreError::codec)?;
            Ok((current, document))
        }
    }
}

async fn locked_current(tx: &mut Transaction<'_, Postgres>, project_id: Uuid) -> Result<(u64, FeasibilityRecordDocument)> {
    let current: Option<i64> = sqlx::query_scalar("SELECT current_version FROM feasibility_records WHERE project_id = $1 FOR UPDATE")
        .bind(project_id)
        .fetch_optional(&mut **tx)
        .await?;
    match current {
        None => Ok((0, FeasibilityRecordDocument::empty(project_id))),
        Some(current) => {
            let current = u64::try_from(current).map_err(StoreError::codec)?;
            let text: String = sqlx::query_scalar("SELECT document::text FROM feasibility_record_versions WHERE project_id = $1 AND version = $2")
                .bind(project_id)
                .bind(version_to_i64(current)?)
                .fetch_one(&mut **tx)
                .await?;
            let document: FeasibilityRecordDocument = serde_json::from_str(&text).map_err(StoreError::codec)?;
            document.validate().map_err(StoreError::codec)?;
            Ok((current, document))
        }
    }
}

async fn read_current(tx: &mut Transaction<'_, Postgres>, project_id: Uuid) -> Result<(u64, FeasibilityRecordDocument)> {
    let current: Option<i64> = sqlx::query_scalar("SELECT current_version FROM feasibility_records WHERE project_id = $1")
        .bind(project_id)
        .fetch_optional(&mut **tx)
        .await?;
    match current {
        None => Ok((0, FeasibilityRecordDocument::empty(project_id))),
        Some(current) => {
            let current = u64::try_from(current).map_err(StoreError::codec)?;
            let text: String = sqlx::query_scalar("SELECT document::text FROM feasibility_record_versions WHERE project_id = $1 AND version = $2")
                .bind(project_id)
                .bind(version_to_i64(current)?)
                .fetch_one(&mut **tx)
                .await?;
            let document: FeasibilityRecordDocument = serde_json::from_str(&text).map_err(StoreError::codec)?;
            document.validate().map_err(StoreError::codec)?;
            Ok((current, document))
        }
    }
}

async fn ensure_record_row(tx: &mut Transaction<'_, Postgres>, project_id: Uuid) -> Result<()> {
    sqlx::query("INSERT INTO feasibility_records (project_id, current_version) VALUES ($1, 0) ON CONFLICT (project_id) DO NOTHING")
        .bind(project_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn append_document(tx: &mut Transaction<'_, Postgres>, project_id: Uuid, version: u64, document: &FeasibilityRecordDocument) -> Result<()> {
    let json = serde_json::to_string(document).map_err(StoreError::codec)?;
    sqlx::query("INSERT INTO feasibility_record_versions (project_id, version, document) VALUES ($1, $2, $3::jsonb)")
        .bind(project_id)
        .bind(version_to_i64(version)?)
        .bind(json)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn advance_version(tx: &mut Transaction<'_, Postgres>, project_id: Uuid, version: u64) -> Result<()> {
    sqlx::query("UPDATE feasibility_records SET current_version = $2 WHERE project_id = $1")
        .bind(project_id)
        .bind(version_to_i64(version)?)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn timestamp(tx: &mut Transaction<'_, Postgres>) -> Result<String> {
    sqlx::query_scalar(&format!("SELECT to_char(clock_timestamp() AT TIME ZONE 'UTC', {RFC3339_UTC})"))
        .fetch_one(&mut **tx)
        .await
        .map_err(StoreError::from)
}

async fn rule_table_version(tx: &mut Transaction<'_, Postgres>, rule_table_version_id: Uuid) -> Result<FeasibilityRuleTableVersion> {
    let row = sqlx::query(&format!(
        "SELECT rule_table_id, rule_table_version_id, version, supersedes, name, rules::text AS rules, source_reference, created_by, \
         to_char(created_at AT TIME ZONE 'UTC', {RFC3339_UTC}) AS created_at \
         FROM feasibility_rule_versions WHERE rule_table_version_id = $1"
    ))
    .bind(rule_table_version_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| StoreError::Invalid("the rule table version does not exist; publish the rules before assessing".into()))?;
    rule_table_from_row(&row)
}

fn rule_table_from_row(row: &PgRow) -> Result<FeasibilityRuleTableVersion> {
    let rules_text: String = row.try_get("rules")?;
    let rules: Vec<FeasibilityRule> = serde_json::from_str(&rules_text).map_err(StoreError::codec)?;
    let version: i64 = row.try_get("version")?;
    let table = FeasibilityRuleTableVersion {
        rule_table_id: row.try_get("rule_table_id")?,
        rule_table_version_id: row.try_get("rule_table_version_id")?,
        version: u64::try_from(version).map_err(StoreError::codec)?,
        supersedes: row.try_get("supersedes")?,
        name: row.try_get("name")?,
        rules,
        source_reference: row.try_get("source_reference")?,
        created_by: row.try_get("created_by")?,
        created_at: Some(row.try_get("created_at")?),
    };
    table.validate().map_err(invalid)?;
    Ok(table)
}
