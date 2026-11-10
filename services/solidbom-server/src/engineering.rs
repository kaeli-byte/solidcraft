//! Engineering review and flat grouping storage (package SB-11, additive to contract version 2).
//!
//! The imported CAD stays immutable. This module stores only engineer decisions, in one append-only
//! engineering record per project, and the exact analysis each decision was reviewed against:
//!
//! - [`Store::register_analysis`] stores the exact source hash, engine build and extractor version,
//!   plus the analysis manifest, so the review is bound to the bytes it describes. The manifest's
//!   source hash must match the source the product revision was delivered from.
//! - [`Store::group_solids`] records which source-local bodies are one reviewed component. It
//!   proves every body exists in the registered analysis and that each body is associated once, so
//!   a body is never silently treated as its own component and no nested hierarchy is invented.
//! - [`Store::associate_component_part`] binds a reviewed component to the project part version it
//!   shares; an unresolved association stays uncosted.
//! - [`Store::complete_measurement`] records a reviewed volume with its source, unit and reason.
//!   A manual completion stays pending until [`Store::confirm_parts`], the bulk engineer
//!   confirmation, accepts the label and the measured volume and captures the confirmed label as
//!   training-example metadata. Nothing is trained or deployed here.
//!
//! Every accepted write appends one immutable record version and advances the project's review
//! version under an expected-version check. Issues are computed on read; a missing input is an
//! issue, never a zero.

use std::collections::{BTreeMap, BTreeSet};

use rust_decimal::Decimal;
use solidbom_contract::{
    AnalysisKey, AnalysisManifest, AnalysisReceipt, AssociateComponentPart, CatalogMatchCandidate, CatalogRecord, ClassificationPrediction,
    ClassificationRecord, CompleteMeasurement, ConfirmParts, ConfirmedCatalogReference, ConfirmedClassification, ContractError, EngineeringFact,
    EngineeringIssue, EngineeringReceipt, EngineeringRecordDocument, EngineeringRecordView, EngineeringWriteOutcome, GroupSolids,
    MAX_CATALOG_SUGGESTIONS_PER_PART, PartConfirmation, ProductRevision, ProjectCatalogOverride, ProjectDocument, RecordSuggestions,
    RegisterAnalysis, ReviewState, ReviewedComponent, ReviewedGeometry, ReviewedPart, TrainingExampleMetadata, Unit, geometry_definition,
    unassociated_geometry,
};
use sqlx::postgres::PgPool;
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

use crate::Result;
use crate::store::{
    RFC3339_UTC, Store, StoreError, check_request_id, insert_request, lock_request, lookup_request, parse_optional_decimal, payload_hash,
    unit_from_db,
};

/// The prefix of the engineering record operations recorded in the shared request log.
const OPERATION_PREFIX: &str = "engineering_record";

impl Store {
    /// Store the exact analysis of one product revision. The analysis is immutable evidence: a
    /// different manifest under the same source/engine/extractor identity is a conflict, never a
    /// silent replacement.
    pub async fn register_analysis(&self, project_id: Uuid, request: &RegisterAnalysis) -> Result<AnalysisReceipt> {
        request.validate().map_err(invalid)?;
        let payload = payload_hash(&format!("{OPERATION_PREFIX}.analysis"), &(project_id, request))?;
        let mut tx = self.pool().begin().await?;
        lock_request(&mut tx, &request.request_id).await?;
        if let Some(receipt) = replay_analysis(&mut tx, &request.request_id, &payload).await? {
            tx.commit().await?;
            return Ok(receipt);
        }
        let project = current_project_document(&mut tx, project_id).await?;
        let revision = project_revision(&project, request.product_revision_id)?;
        let package = project
            .packages
            .iter()
            .find(|package| package.package_id == revision.package_id)
            .ok_or_else(|| StoreError::Invalid("the product revision names a delivery package the project does not hold".into()))?;
        if request.manifest.key.source_sha256 != package.source.sha256 {
            return Err(StoreError::Invalid("the analysis source hash differs from the delivered package source".into()));
        }
        let key = &request.manifest.key;
        let stored: Option<String> = sqlx::query_scalar(
            "SELECT manifest::text FROM engineering_analyses \
             WHERE project_id = $1 AND product_revision_id = $2 AND source_sha256 = $3 AND engine_build = $4 AND extractor_version = $5",
        )
        .bind(project_id)
        .bind(request.product_revision_id)
        .bind(&key.source_sha256)
        .bind(&key.engine_build)
        .bind(&key.extractor_version)
        .fetch_optional(&mut *tx)
        .await?;
        match stored {
            Some(text) => {
                let existing: AnalysisManifest = serde_json::from_str(&text).map_err(StoreError::codec)?;
                if existing != request.manifest {
                    return Err(StoreError::Conflict("a different analysis already claims this source, engine and extractor identity".into()));
                }
            }
            None => {
                // Two concurrent registrations of one identity are serialised by the primary key:
                // the loser writes nothing and then compares the committed evidence.
                let manifest = serde_json::to_string(&request.manifest).map_err(StoreError::codec)?;
                let inserted: Option<String> = sqlx::query_scalar(
                    "INSERT INTO engineering_analyses \
                     (project_id, product_revision_id, source_sha256, engine_build, extractor_version, manifest) \
                     VALUES ($1, $2, $3, $4, $5, $6::jsonb) \
                     ON CONFLICT (project_id, product_revision_id, source_sha256, engine_build, extractor_version) DO NOTHING \
                     RETURNING manifest::text",
                )
                .bind(project_id)
                .bind(request.product_revision_id)
                .bind(&key.source_sha256)
                .bind(&key.engine_build)
                .bind(&key.extractor_version)
                .bind(manifest)
                .fetch_optional(&mut *tx)
                .await?;
                if inserted.is_none() {
                    let text: String = sqlx::query_scalar(
                        "SELECT manifest::text FROM engineering_analyses \
                         WHERE project_id = $1 AND product_revision_id = $2 AND source_sha256 = $3 AND engine_build = $4 AND extractor_version = $5",
                    )
                    .bind(project_id)
                    .bind(request.product_revision_id)
                    .bind(&key.source_sha256)
                    .bind(&key.engine_build)
                    .bind(&key.extractor_version)
                    .fetch_one(&mut *tx)
                    .await?;
                    let existing: AnalysisManifest = serde_json::from_str(&text).map_err(StoreError::codec)?;
                    if existing != request.manifest {
                        return Err(StoreError::Conflict("a different analysis already claims this source, engine and extractor identity".into()));
                    }
                }
            }
        }
        let receipt = AnalysisReceipt::of(request.request_id.clone(), request.product_revision_id, &request.manifest);
        insert_request(&mut tx, &request.request_id, &format!("{OPERATION_PREFIX}.analysis"), &payload, &receipt).await?;
        tx.commit().await?;
        Ok(receipt)
    }

    /// The current engineering review of a project: its append-only version, the reviewed decisions
    /// and the computed issues. A project with no recorded decision reads as version 0.
    pub async fn engineering_record_view(&self, project_id: Uuid) -> Result<EngineeringRecordView> {
        let mut tx = self.pool().begin().await?;
        ensure_project_exists(&mut tx, project_id).await?;
        let (version, document) = read_current(&mut tx, project_id).await?;
        let analyses = registered_analyses(&mut tx, project_id).await?;
        tx.commit().await?;
        Ok(view(project_id, version, document, &analyses))
    }

    /// One retained engineering review version, which may be historical.
    pub async fn engineering_record_version(&self, project_id: Uuid, version: u64) -> Result<EngineeringRecordView> {
        let mut tx = self.pool().begin().await?;
        ensure_project_exists(&mut tx, project_id).await?;
        let text: String = sqlx::query_scalar("SELECT document::text FROM engineering_record_versions WHERE project_id = $1 AND version = $2")
            .bind(project_id)
            .bind(version_to_i64(version)?)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(StoreError::NotFound)?;
        let document: EngineeringRecordDocument = serde_json::from_str(&text).map_err(StoreError::codec)?;
        document.validate().map_err(StoreError::codec)?;
        let analyses = registered_analyses(&mut tx, project_id).await?;
        tx.commit().await?;
        Ok(view(project_id, version, document, &analyses))
    }

    /// Group source-local bodies into one reviewed component, replace its grouping, or dissolve it
    /// by submitting no geometry. Every body must exist in the registered analysis and belong to
    /// the same definition; no body may be associated with two components.
    pub async fn group_solids(&self, project_id: Uuid, request: &GroupSolids) -> Result<EngineeringWriteOutcome> {
        request.validate().map_err(invalid)?;
        let payload = payload_hash(&format!("{OPERATION_PREFIX}.group"), &(project_id, request))?;
        self.write_engineering_document(
            project_id,
            &request.request_id,
            request.expected_version,
            &payload,
            "group",
            |document, project, analyses| {
                let revision_id = request.product_revision_id;
                let _revision = project_revision(project, revision_id)?;
                if let Some(part_version_id) = request.part_version_id {
                    ensure_part_version(project, part_version_id)?;
                }
                let registered = analyses
                    .iter()
                    .find(|registered| registered.product_revision_id == revision_id && registered.manifest.key == request.analysis_key)
                    .ok_or_else(|| StoreError::Invalid("register the exact analysis of the revision before grouping its bodies".into()))?;
                let mut definitions = BTreeSet::new();
                for geometry in &request.geometry {
                    let definition_id = geometry_definition(&registered.manifest, geometry)
                        .ok_or_else(|| StoreError::Invalid("a grouped body does not exist in the registered analysis".into()))?;
                    definitions.insert(definition_id);
                }
                if definitions.len() > 1 {
                    return Err(StoreError::Invalid(
                        "one reviewed component cannot merge bodies from different definitions; group each definition's bodies separately".into(),
                    ));
                }
                let mut facts = Vec::new();
                match document.component(request.component_id) {
                    Some(existing) => {
                        if existing.product_revision_id != revision_id {
                            return Err(StoreError::Invalid("a reviewed component cannot move to another product revision".into()));
                        }
                        if existing.analysis_key != request.analysis_key {
                            return Err(StoreError::Invalid(
                                "a reviewed component is bound to one exact analysis; a changed source, engine or extractor needs a new component"
                                    .into(),
                            ));
                        }
                        if request.geometry.is_empty() {
                            document.components.retain(|component| component.component_id != request.component_id);
                            facts.push(EngineeringFact::ComponentDissolved { component_id: request.component_id });
                        } else {
                            ensure_bodies_are_free(document, request.component_id, revision_id, &request.analysis_key, &request.geometry)?;
                            let component = document
                                .components
                                .iter_mut()
                                .find(|component| component.component_id == request.component_id)
                                .ok_or_else(|| StoreError::Invalid("the reviewed component disappeared while grouping".into()))?;
                            component.geometry = request.geometry.clone();
                            component.part_version_id = request.part_version_id;
                            facts.push(EngineeringFact::ComponentGrouped {
                                component_id: request.component_id,
                                product_revision_id: revision_id,
                                geometry: u64::try_from(component.geometry.len()).map_err(StoreError::codec)?,
                            });
                        }
                    }
                    None => {
                        if request.geometry.is_empty() {
                            return Err(StoreError::Invalid("a new reviewed component needs at least one body".into()));
                        }
                        ensure_bodies_are_free(document, request.component_id, revision_id, &request.analysis_key, &request.geometry)?;
                        document.components.push(ReviewedComponent {
                            component_id: request.component_id,
                            product_revision_id: revision_id,
                            analysis_key: request.analysis_key.clone(),
                            geometry: request.geometry.clone(),
                            part_version_id: request.part_version_id,
                        });
                        facts.push(EngineeringFact::ComponentGrouped {
                            component_id: request.component_id,
                            product_revision_id: revision_id,
                            geometry: u64::try_from(request.geometry.len()).map_err(StoreError::codec)?,
                        });
                    }
                }
                capture_component_example(document, request.component_id, &mut facts);
                Ok(facts)
            },
        )
        .await
    }

    /// Associate a reviewed component with the project part version it shares, or clear the
    /// association. An unresolved component stays uncosted.
    pub async fn associate_component_part(&self, project_id: Uuid, request: &AssociateComponentPart) -> Result<EngineeringWriteOutcome> {
        request.validate().map_err(invalid)?;
        let payload = payload_hash(&format!("{OPERATION_PREFIX}.component_part"), &(project_id, request))?;
        self.write_engineering_document(
            project_id,
            &request.request_id,
            request.expected_version,
            &payload,
            "component_part",
            |document, project, _| {
                if let Some(part_version_id) = request.part_version_id {
                    ensure_part_version(project, part_version_id)?;
                }
                let component = document
                    .components
                    .iter_mut()
                    .find(|component| component.component_id == request.component_id)
                    .ok_or_else(|| StoreError::Invalid("the project has no such reviewed component".into()))?;
                component.part_version_id = request.part_version_id;
                let mut facts =
                    vec![EngineeringFact::ComponentPartAssociated { component_id: request.component_id, part_version_id: request.part_version_id }];
                capture_component_example(document, request.component_id, &mut facts);
                Ok(facts)
            },
        )
        .await
    }

    /// Record or replace the reviewed volume of one project part version. A manual value carries
    /// its unit, source and reason and stays pending until an engineer confirms it.
    pub async fn complete_measurement(&self, project_id: Uuid, request: &CompleteMeasurement) -> Result<EngineeringWriteOutcome> {
        request.validate().map_err(invalid)?;
        let payload = payload_hash(&format!("{OPERATION_PREFIX}.measurement"), &(project_id, request))?;
        self.write_engineering_document(project_id, &request.request_id, request.expected_version, &payload, "measurement", |document, project, _| {
            ensure_part_version(project, request.part_version_id)?;
            // The value must describe geometry actually under review: the measurement's exact
            // analysis has to match a reviewed component that references this part version, so a
            // volume from unrelated source bytes cannot be recorded (and later confirmed).
            let analysis = &request.volume_mm3.analysis_key;
            let described = document
                .components
                .iter()
                .any(|component| component.part_version_id == Some(request.part_version_id) && component.analysis_key == *analysis);
            if !described {
                return Err(StoreError::Invalid("the measurement's analysis does not describe any reviewed component of this part version".into()));
            }
            let measurement = request.volume_mm3.clone();
            match document.part_mut(request.part_version_id) {
                Some(part) => {
                    part.volume_mm3 = Some(measurement.clone());
                    // A replaced value is review work again until an engineer confirms it.
                    part.volume_confirmed_by = None;
                }
                None => document.parts.push(ReviewedPart {
                    part_version_id: request.part_version_id,
                    classification: ClassificationRecord::pending(),
                    volume_mm3: Some(measurement.clone()),
                    volume_confirmed_by: None,
                    catalog: CatalogRecord::pending(),
                }),
            }
            Ok(vec![EngineeringFact::MeasurementRecorded {
                part_version_id: request.part_version_id,
                source: measurement.source,
                review: measurement.review,
            }])
        })
        .await
    }

    /// Record recognition and catalog-matching suggestions as **proposals**: one append-only
    /// engineering record version stores each classification prediction in
    /// `ReviewedPart.classification.prediction` and each resolved catalog candidate in
    /// `ReviewedPart.catalog.suggestions`. Nothing here confirms, clears or replaces an engineer's
    /// confirmed label, catalog reference or project override, and a confidence value never
    /// authorises a cost input. Recording a fresh matching run for a part version replaces that
    /// part's previous candidate list while the confirmed reference and override stay untouched.
    pub async fn record_suggestions(&self, project_id: Uuid, request: &RecordSuggestions) -> Result<EngineeringWriteOutcome> {
        request.validate().map_err(invalid)?;
        let payload = payload_hash(&format!("{OPERATION_PREFIX}.suggestions"), &(project_id, request))?;
        // Catalog versions are immutable evidence: a published row never changes, so resolving the
        // authoritative values before the write transaction reads exactly what the transaction
        // would. The payload never decides a stored catalog value.
        let mut resolved = BTreeMap::new();
        for suggestion in &request.catalog {
            let catalog = resolve_catalog_version(self.pool(), suggestion.component_version_id)
                .await?
                .ok_or_else(|| StoreError::Invalid("the catalog suggestion names a component version the catalog does not hold".into()))?;
            if catalog.component_id != suggestion.component_id {
                return Err(StoreError::Invalid("the catalog suggestion names a component version of a different component".into()));
            }
            resolved.insert(suggestion.component_version_id, catalog);
        }
        let recorded_at = service_timestamp(self.pool()).await?;
        self.write_engineering_document(
            project_id,
            &request.request_id,
            request.expected_version,
            &payload,
            "suggestions",
            |document, project, analyses| {
                record_classification_suggestions(document, project, analyses, request, &recorded_at)?;
                record_catalog_suggestions(document, project_id, project, request, &resolved, &recorded_at)?;
                Ok(vec![EngineeringFact::SuggestionsRecorded {
                    classification: u64::try_from(request.classification.len()).map_err(StoreError::codec)?,
                    catalog: u64::try_from(request.catalog.len()).map_err(StoreError::codec)?,
                }])
            },
        )
        .await
    }

    /// Bulk confirmation of engineer decisions: accept or correct classifications, confirm recorded
    /// volumes, confirm the catalog version an engineer selected, and capture the confirmed labels
    /// as training-example metadata. A confidence value never confirms anything by itself.
    pub async fn confirm_parts(&self, project_id: Uuid, request: &ConfirmParts, reviewer: &str) -> Result<EngineeringWriteOutcome> {
        request.validate().map_err(invalid)?;
        let payload = payload_hash(&format!("{OPERATION_PREFIX}.confirmation"), &(project_id, request))?;
        // The confirmed catalog reference quotes the catalog's own values, resolved here from the
        // immutable catalog version rather than trusted from the payload.
        let mut resolved_catalog = BTreeMap::new();
        for confirmation in &request.confirmations {
            let Some(catalog) = &confirmation.catalog else { continue };
            let resolved = resolve_catalog_version(self.pool(), catalog.component_version_id)
                .await?
                .ok_or_else(|| StoreError::Invalid("the confirmation names a catalog component version the catalog does not hold".into()))?;
            resolved_catalog.insert(catalog.component_version_id, resolved);
        }
        self.write_engineering_document(
            project_id,
            &request.request_id,
            request.expected_version,
            &payload,
            "confirmation",
            |document, project, _| {
                for confirmation in &request.confirmations {
                    ensure_part_version(project, confirmation.part_version_id)?;
                }
                let mut facts = Vec::new();
                for confirmation in &request.confirmations {
                    confirm_one(document, confirmation, reviewer, &resolved_catalog, &mut facts)?;
                }
                Ok(facts)
            },
        )
        .await
    }

    /// A plain expected-version-checked document write: lock, load, apply, validate, append and
    /// record the request. The closure returns the emitted dependency facts.
    async fn write_engineering_document<F>(
        &self,
        project_id: Uuid,
        request_id: &str,
        expected_version: u64,
        payload: &str,
        operation: &str,
        apply: F,
    ) -> Result<EngineeringWriteOutcome>
    where
        F: FnOnce(&mut EngineeringRecordDocument, &ProjectDocument, &[RegisteredAnalysis]) -> Result<Vec<EngineeringFact>>,
    {
        check_request_id(request_id)?;
        let mut tx = self.pool().begin().await?;
        lock_request(&mut tx, request_id).await?;
        lock_engineering(&mut tx, project_id).await?;
        if let Some(outcome) = replay_write(&mut tx, request_id, payload).await? {
            tx.commit().await?;
            return Ok(outcome);
        }
        let project = current_project_document(&mut tx, project_id).await?;
        let (current, mut document) = locked_current(&mut tx, project_id).await?;
        if current != expected_version {
            return Err(version_conflict(current, expected_version));
        }
        let analyses = registered_analyses(&mut tx, project_id).await?;
        let facts = apply(&mut document, &project, &analyses)?;
        document.validate().map_err(invalid)?;
        let version = current + 1;
        ensure_record_row(&mut tx, project_id).await?;
        append_document(&mut tx, project_id, version, &document).await?;
        advance_version(&mut tx, project_id, version).await?;
        let outcome = finish(&mut tx, request_id, &format!("{OPERATION_PREFIX}.{operation}"), payload, project_id, version, facts).await?;
        tx.commit().await?;
        Ok(outcome)
    }
}

/// One registered analysis of one product revision, as the write and read paths see it.
pub struct RegisteredAnalysis {
    pub product_revision_id: Uuid,
    pub manifest: AnalysisManifest,
}

/// The authoritative values of one catalog component version, resolved from the immutable catalog
/// row at record time. A suggestion or a confirmed reference quotes these values; the payload is
/// never its own source of truth.
struct ResolvedCatalogVersion {
    component_id: Uuid,
    component_version_id: Uuid,
    version: u64,
    name: String,
    specification: String,
    price: Option<Decimal>,
    price_unit: Unit,
    currency: String,
}

async fn resolve_catalog_version(pool: &PgPool, component_version_id: Uuid) -> Result<Option<ResolvedCatalogVersion>> {
    let row = sqlx::query(
        "SELECT component_id, version, name, specification, price::text AS price, price_unit, currency \
         FROM catalog_component_versions WHERE component_version_id = $1",
    )
    .bind(component_version_id)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else { return Ok(None) };
    Ok(Some(ResolvedCatalogVersion {
        component_id: row.try_get("component_id")?,
        component_version_id,
        version: u64::try_from(row.try_get::<i64, _>("version")?).map_err(StoreError::codec)?,
        name: row.try_get("name")?,
        specification: row.try_get("specification")?,
        price: parse_optional_decimal(row.try_get("price")?, "component price")?,
        price_unit: unit_from_db(row.try_get("price_unit")?, Unit::CurrencyPerPiece)?,
        currency: row.try_get("currency")?,
    }))
}

/// The service's RFC 3339 UTC timestamp. It always stamps a recorded suggestion; a client-supplied
/// `suggested_at` is overwritten, so a client never decides when a suggestion was made.
async fn service_timestamp(pool: &PgPool) -> Result<String> {
    let timestamp = sqlx::query_scalar(&format!("SELECT to_char(now() AT TIME ZONE 'UTC', {RFC3339_UTC})")).fetch_one(pool).await?;
    Ok(timestamp)
}

/// Store each classification prediction as the reviewed part's proposal, creating the reviewed part
/// record when the prediction arrives before any other review of that part version. A confirmed
/// label is never set, cleared or replaced here: only the engineer's confirmation can change it.
fn record_classification_suggestions(
    document: &mut EngineeringRecordDocument,
    project: &ProjectDocument,
    analyses: &[RegisteredAnalysis],
    request: &RecordSuggestions,
    recorded_at: &str,
) -> Result<()> {
    for suggestion in &request.classification {
        let part_version_id = suggestion.part_version_id;
        ensure_part_version(project, part_version_id)?;
        let mut prediction = suggestion.prediction.clone();
        let analysis_key = prediction
            .analysis_key
            .clone()
            .ok_or_else(|| StoreError::Invalid("a recorded classification suggestion must name the exact analysis it was recognised from".into()))?;
        if !analyses.iter().any(|registered| registered.manifest.key == analysis_key) {
            return Err(StoreError::Invalid("a classification suggestion's analysis is not registered for the project".into()));
        }
        // The same binding rule as a completed measurement: the prediction's analysis must describe
        // a reviewed component that references this part version, so unrelated source bytes cannot
        // carry a prediction onto the part.
        let described =
            document.components.iter().any(|component| component.part_version_id == Some(part_version_id) && component.analysis_key == analysis_key);
        if !described {
            return Err(StoreError::Invalid(
                "a classification suggestion's analysis does not describe any reviewed component of this part version".into(),
            ));
        }
        // The service stamps the recording time and overwrites any client-supplied timestamp: a
        // proposal says when the service received it, never when a client claims it was made.
        prediction.suggested_at = Some(recorded_at.to_string());
        match document.part_mut(part_version_id) {
            Some(part) => part.classification.prediction = Some(prediction),
            None => document.parts.push(ReviewedPart {
                part_version_id,
                classification: ClassificationRecord { prediction: Some(prediction), confirmed: None },
                volume_mm3: None,
                volume_confirmed_by: None,
                catalog: CatalogRecord::pending(),
            }),
        }
    }
    Ok(())
}

/// Store one matching run's catalog candidates. Every evaluated part version is replaced by the
/// candidates this run carries for it (possibly none), so a no-match or ambiguous result clears the
/// previous proposals instead of leaving stale matches behind. The engineer-confirmed reference and
/// project override are never changed by a matching run; a part version named only by a candidate
/// is treated as evaluated too, which keeps a payload written before the evaluated-part field
/// working.
fn record_catalog_suggestions(
    document: &mut EngineeringRecordDocument,
    project_id: Uuid,
    project: &ProjectDocument,
    request: &RecordSuggestions,
    resolved: &BTreeMap<Uuid, ResolvedCatalogVersion>,
    recorded_at: &str,
) -> Result<()> {
    for part_version_id in &request.catalog_evaluated_part_versions {
        ensure_part_version(project, *part_version_id)?;
    }
    let mut per_part: BTreeMap<Uuid, Vec<CatalogMatchCandidate>> = BTreeMap::new();
    for suggestion in &request.catalog {
        ensure_part_version(project, suggestion.part_version_id)?;
        let catalog = resolved
            .get(&suggestion.component_version_id)
            .ok_or_else(|| StoreError::Invalid("the catalog suggestion's component version was not resolved".into()))?;
        per_part.entry(suggestion.part_version_id).or_default().push(CatalogMatchCandidate {
            suggestion_id: catalog_suggestion_id(
                project_id,
                suggestion.part_version_id,
                suggestion.component_version_id,
                &suggestion.model_version,
                &suggestion.prediction_version,
            ),
            part_version_id: suggestion.part_version_id,
            component_id: catalog.component_id,
            component_version_id: catalog.component_version_id,
            catalog_version: catalog.version,
            catalog_name: catalog.name.clone(),
            specification: catalog.specification.clone(),
            price: catalog.price,
            price_unit: catalog.price_unit,
            currency: catalog.currency.clone(),
            model_version: suggestion.model_version.clone(),
            prediction_version: suggestion.prediction_version.clone(),
            confidence: suggestion.confidence,
            evidence: suggestion.evidence.clone(),
            suggested_at: Some(recorded_at.to_string()),
        });
    }
    let mut evaluated: BTreeSet<Uuid> = request.catalog_evaluated_part_versions.iter().copied().collect();
    evaluated.extend(per_part.keys().copied());
    for part_version_id in evaluated {
        let candidates = per_part.remove(&part_version_id).unwrap_or_default();
        if candidates.len() > MAX_CATALOG_SUGGESTIONS_PER_PART {
            return Err(StoreError::Invalid(format!(
                "a part version accepts at most {MAX_CATALOG_SUGGESTIONS_PER_PART} catalog suggestions in one matching run"
            )));
        }
        match document.part_mut(part_version_id) {
            // A fresh matching run replaces only the proposals; the confirmed reference and the
            // project override stay exactly as the engineer recorded them.
            Some(part) => part.catalog.suggestions = candidates,
            None => {
                // Nothing was stored for this part version yet, so an empty evaluation writes no
                // empty record; a candidate creates the reviewed part record as before.
                if !candidates.is_empty() {
                    document.parts.push(ReviewedPart {
                        part_version_id,
                        classification: ClassificationRecord::pending(),
                        volume_mm3: None,
                        volume_confirmed_by: None,
                        catalog: CatalogRecord { suggestions: candidates, confirmed: None, project_override: None },
                    });
                }
            }
        }
    }
    Ok(())
}

/// The deterministic identity of one catalog candidate, so a repeated matching run replaces a
/// candidate instead of duplicating it. The identity is the project, the part version, the catalog
/// component version and the model/prediction versions that produced the suggestion.
fn catalog_suggestion_id(project_id: Uuid, part_version_id: Uuid, component_version_id: Uuid, model_version: &str, prediction_version: &str) -> Uuid {
    let key = format!("catalog_match:{project_id}:{part_version_id}:{component_version_id}:{model_version}:{prediction_version}");
    Uuid::new_v5(&Uuid::NAMESPACE_OID, key.as_bytes())
}

fn invalid(error: ContractError) -> StoreError {
    StoreError::Invalid(error.to_string())
}

fn version_to_i64(version: u64) -> Result<i64> {
    i64::try_from(version).map_err(StoreError::codec)
}

fn version_conflict(current: u64, expected: u64) -> StoreError {
    StoreError::Conflict(format!("the engineering review is at version {current}, not {expected}"))
}

async fn replay_analysis(tx: &mut Transaction<'_, Postgres>, request_id: &str, payload: &str) -> Result<Option<AnalysisReceipt>> {
    if let Some((stored, response)) = lookup_request(&mut *tx, request_id).await? {
        if stored != payload {
            return Err(StoreError::Conflict("the request ID was reused with a different payload".into()));
        }
        return serde_json::from_value(response).map(Some).map_err(StoreError::codec);
    }
    Ok(None)
}

async fn replay_write(tx: &mut Transaction<'_, Postgres>, request_id: &str, payload: &str) -> Result<Option<EngineeringWriteOutcome>> {
    if let Some((stored, response)) = lookup_request(&mut *tx, request_id).await? {
        if stored != payload {
            return Err(StoreError::Conflict("the request ID was reused with a different payload".into()));
        }
        return serde_json::from_value(response).map(Some).map_err(StoreError::codec);
    }
    Ok(None)
}

async fn finish(
    tx: &mut Transaction<'_, Postgres>,
    request_id: &str,
    operation: &str,
    payload: &str,
    project_id: Uuid,
    version: u64,
    facts: Vec<EngineeringFact>,
) -> Result<EngineeringWriteOutcome> {
    let outcome = EngineeringWriteOutcome { receipt: EngineeringReceipt { request_id: request_id.to_string(), project_id, version }, facts };
    insert_request(&mut *tx, request_id, operation, payload, &outcome).await?;
    Ok(outcome)
}

/// Serialise review writes of one project, including the first write that creates the record row.
async fn lock_engineering(tx: &mut Transaction<'_, Postgres>, project_id: Uuid) -> Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))").bind(format!("engineering_record:{project_id}")).execute(&mut **tx).await?;
    Ok(())
}

async fn ensure_project_exists(tx: &mut Transaction<'_, Postgres>, project_id: Uuid) -> Result<()> {
    let found: Option<Uuid> =
        sqlx::query_scalar("SELECT project_id FROM project_records WHERE project_id = $1").bind(project_id).fetch_optional(&mut **tx).await?;
    found.map(|_| ()).ok_or(StoreError::NotFound)
}

/// The current project document, read for identity validation. Project records are append-only, so
/// an identity the write validates stays valid while the review is written.
async fn current_project_document(tx: &mut Transaction<'_, Postgres>, project_id: Uuid) -> Result<ProjectDocument> {
    let current: i64 = sqlx::query_scalar("SELECT current_version FROM project_records WHERE project_id = $1")
        .bind(project_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(StoreError::NotFound)?;
    let current = u64::try_from(current).map_err(StoreError::codec)?;
    let text: String = sqlx::query_scalar("SELECT document::text FROM project_record_versions WHERE project_id = $1 AND version = $2")
        .bind(project_id)
        .bind(version_to_i64(current)?)
        .fetch_one(&mut **tx)
        .await?;
    let document: ProjectDocument = serde_json::from_str(&text).map_err(StoreError::codec)?;
    document.validate().map_err(StoreError::codec)?;
    Ok(document)
}

async fn locked_current(tx: &mut Transaction<'_, Postgres>, project_id: Uuid) -> Result<(u64, EngineeringRecordDocument)> {
    let current: Option<i64> = sqlx::query_scalar("SELECT current_version FROM engineering_records WHERE project_id = $1 FOR UPDATE")
        .bind(project_id)
        .fetch_optional(&mut **tx)
        .await?;
    match current {
        None => {
            // The identity check already proved the project exists; a first review starts at 0.
            Ok((0, EngineeringRecordDocument::empty(project_id)))
        }
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

async fn read_current(tx: &mut Transaction<'_, Postgres>, project_id: Uuid) -> Result<(u64, EngineeringRecordDocument)> {
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

async fn registered_analyses(tx: &mut Transaction<'_, Postgres>, project_id: Uuid) -> Result<Vec<RegisteredAnalysis>> {
    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT product_revision_id, manifest::text FROM engineering_analyses WHERE project_id = $1 ORDER BY registered_at, product_revision_id",
    )
    .bind(project_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut analyses = Vec::with_capacity(rows.len());
    for (product_revision_id, text) in rows {
        let manifest: AnalysisManifest = serde_json::from_str(&text).map_err(StoreError::codec)?;
        manifest.validate().map_err(StoreError::codec)?;
        analyses.push(RegisteredAnalysis { product_revision_id, manifest });
    }
    Ok(analyses)
}

async fn ensure_record_row(tx: &mut Transaction<'_, Postgres>, project_id: Uuid) -> Result<()> {
    sqlx::query("INSERT INTO engineering_records (project_id, current_version) VALUES ($1, 0) ON CONFLICT (project_id) DO NOTHING")
        .bind(project_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn append_document(tx: &mut Transaction<'_, Postgres>, project_id: Uuid, version: u64, document: &EngineeringRecordDocument) -> Result<()> {
    let json = serde_json::to_string(document).map_err(StoreError::codec)?;
    sqlx::query("INSERT INTO engineering_record_versions (project_id, version, document) VALUES ($1, $2, $3::jsonb)")
        .bind(project_id)
        .bind(version_to_i64(version)?)
        .bind(json)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn advance_version(tx: &mut Transaction<'_, Postgres>, project_id: Uuid, version: u64) -> Result<()> {
    sqlx::query("UPDATE engineering_records SET current_version = $2 WHERE project_id = $1")
        .bind(project_id)
        .bind(version_to_i64(version)?)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

fn project_revision(project: &ProjectDocument, product_revision_id: Uuid) -> Result<&ProductRevision> {
    project
        .products
        .iter()
        .flat_map(|product| product.revisions.iter())
        .find(|revision| revision.product_revision_id == product_revision_id)
        .ok_or_else(|| StoreError::Invalid("the project has no such product revision".into()))
}

fn ensure_part_version(project: &ProjectDocument, part_version_id: Uuid) -> Result<()> {
    if project.part_versions.iter().any(|part| part.part_version_id == part_version_id) {
        Ok(())
    } else {
        Err(StoreError::Invalid("the project has no such shared part version".into()))
    }
}

/// Refuse a body that another reviewed component of the same revision and analysis already owns.
/// Moving a body means rewriting the component that owns it first; the association is never
/// silently duplicated.
fn ensure_bodies_are_free(
    document: &EngineeringRecordDocument,
    component_id: Uuid,
    product_revision_id: Uuid,
    analysis_key: &AnalysisKey,
    geometry: &[ReviewedGeometry],
) -> Result<()> {
    let wanted: BTreeSet<&ReviewedGeometry> = geometry.iter().collect();
    for component in document.components_of(product_revision_id, analysis_key) {
        if component.component_id == component_id {
            continue;
        }
        if component.geometry.iter().any(|owned| wanted.contains(owned)) {
            return Err(StoreError::Invalid("a body is already part of another reviewed component; regroup the existing component instead".into()));
        }
    }
    Ok(())
}

/// Confirm one part version's decisions and capture its confirmed label as training-example
/// metadata. The engineer's decision is the only path to a confirmed label, volume or catalog
/// reference.
fn confirm_one(
    document: &mut EngineeringRecordDocument,
    confirmation: &PartConfirmation,
    reviewer: &str,
    resolved_catalog: &BTreeMap<Uuid, ResolvedCatalogVersion>,
    facts: &mut Vec<EngineeringFact>,
) -> Result<()> {
    let part_version_id = confirmation.part_version_id;
    if document.part(part_version_id).is_none() {
        document.parts.push(ReviewedPart {
            part_version_id,
            classification: ClassificationRecord::pending(),
            volume_mm3: None,
            volume_confirmed_by: None,
            catalog: CatalogRecord::pending(),
        });
    }
    let mut classification_confirmed = false;
    let mut measurement_confirmed = false;
    let mut catalog_confirmed = false;
    let mut captured = 0u64;
    if let Some(classification) = &confirmation.classification {
        let part =
            document.part_mut(part_version_id).ok_or_else(|| StoreError::Invalid("the reviewed part record disappeared while confirming".into()))?;
        part.classification.confirmed = Some(ConfirmedClassification { name: classification.name.clone(), confirmed_by: reviewer.to_string() });
        classification_confirmed = true;
        let components: Vec<ReviewedComponent> = document.components_for_part(part_version_id).cloned().collect();
        for component in components {
            captured += capture_example(document, &component);
        }
    }
    if confirmation.confirm_measurement {
        let part =
            document.part_mut(part_version_id).ok_or_else(|| StoreError::Invalid("the reviewed part record disappeared while confirming".into()))?;
        let volume = part
            .volume_mm3
            .as_mut()
            .ok_or_else(|| StoreError::Invalid("the part version has no recorded volume to confirm; complete the measurement first".into()))?;
        if volume.value.is_none() {
            return Err(StoreError::Invalid("a missing volume cannot be confirmed; complete the measurement first".into()));
        }
        volume.review = ReviewState::Confirmed;
        part.volume_confirmed_by = Some(reviewer.to_string());
        measurement_confirmed = true;
    }
    if let Some(catalog) = &confirmation.catalog {
        let resolved = resolved_catalog
            .get(&catalog.component_version_id)
            .ok_or_else(|| StoreError::Invalid("the confirmation's catalog component version was not resolved".into()))?;
        let part =
            document.part_mut(part_version_id).ok_or_else(|| StoreError::Invalid("the reviewed part record disappeared while confirming".into()))?;
        // The confirmed reference keeps the catalog values it was confirmed against, even when a
        // project override replaces them for this project's costing inputs.
        part.catalog.confirmed = Some(ConfirmedCatalogReference {
            component_id: resolved.component_id,
            component_version_id: resolved.component_version_id,
            version: resolved.version,
            name: resolved.name.clone(),
            specification: resolved.specification.clone(),
            price: resolved.price,
            price_unit: resolved.price_unit,
            currency: resolved.currency.clone(),
            confirmed_by: reviewer.to_string(),
        });
        // The project override is a separate engineer decision: its own values, reason and reviewer.
        // Confirming a catalog version without an override records no override for this part.
        part.catalog.project_override = catalog.project_override.as_ref().map(|project_override| ProjectCatalogOverride {
            specification: project_override.specification.clone(),
            price: project_override.price,
            price_unit: project_override.price_unit,
            currency: project_override.currency.clone(),
            reason: project_override.reason.clone(),
            recorded_by: reviewer.to_string(),
        });
        catalog_confirmed = true;
    }
    // The part-confirmed fact names a classification and/or a volume decision. A catalog-only
    // confirmation has its own fact below, so it does not emit a no-op part-confirmed signal.
    if classification_confirmed || measurement_confirmed {
        facts.push(EngineeringFact::PartConfirmed { part_version_id, classification_confirmed, measurement_confirmed });
    }
    if catalog_confirmed {
        facts.push(EngineeringFact::CatalogMatchConfirmed { part_version_id });
    }
    if captured > 0 {
        facts.push(EngineeringFact::TrainingExamplesCaptured { part_version_id, count: captured });
    }
    Ok(())
}

/// Capture the confirmed label of the component's shared part version as one self-contained
/// training-example record. An already captured example is not duplicated.
fn capture_component_example(document: &mut EngineeringRecordDocument, component_id: Uuid, facts: &mut Vec<EngineeringFact>) {
    let Some(component) = document.component(component_id).cloned() else { return };
    let Some(part_version_id) = component.part_version_id else { return };
    if capture_example(document, &component) == 0 {
        return;
    }
    facts.push(EngineeringFact::TrainingExamplesCaptured { part_version_id, count: 1 });
}

fn capture_example(document: &mut EngineeringRecordDocument, component: &ReviewedComponent) -> u64 {
    let Some(part_version_id) = component.part_version_id else { return 0 };
    let Some(part) = document.part(part_version_id) else { return 0 };
    let Some(confirmed_label) = part.classification.confirmed.clone() else { return 0 };
    let prediction = part.classification.prediction.clone();
    let example_id = training_example_id(part_version_id, component, &confirmed_label, prediction.as_ref());
    if document.training_examples.iter().any(|example| example.example_id == example_id) {
        return 0;
    }
    document.training_examples.push(TrainingExampleMetadata {
        example_id,
        part_version_id,
        component_id: component.component_id,
        product_revision_id: component.product_revision_id,
        analysis_key: component.analysis_key.clone(),
        geometry: component.geometry.clone(),
        prediction,
        confirmed_label,
    });
    1
}

/// The deterministic example identity: the same confirmed label for the same reviewed component is
/// one example, and a later corrected label is a separate one.
fn training_example_id(
    part_version_id: Uuid,
    component: &ReviewedComponent,
    label: &ConfirmedClassification,
    prediction: Option<&ClassificationPrediction>,
) -> Uuid {
    let mut key = format!("{part_version_id}:{}:{}", component.component_id, label.name);
    if let Some(prediction) = prediction {
        key.push_str(&format!(":{}:{}", prediction.model_version, prediction.name));
    }
    Uuid::new_v5(&Uuid::NAMESPACE_OID, key.as_bytes())
}

fn view(project_id: Uuid, version: u64, document: EngineeringRecordDocument, analyses: &[RegisteredAnalysis]) -> EngineeringRecordView {
    let mut issues = document.issues();
    // A flattened input carries the extraction's own `unresolved_grouping` guard. Its review is
    // complete only when every body of the registered analysis belongs to a reviewed component;
    // an unassociated body stays uncosted rather than becoming its own component.
    for registered in analyses {
        if !registered.manifest.issues.iter().any(|issue| issue.code == "unresolved_grouping") {
            continue;
        }
        let uncovered = unassociated_geometry(&document, registered.product_revision_id, &registered.manifest);
        if uncovered.is_empty() {
            continue;
        }
        let mut issue = EngineeringIssue::new(
            "unresolved_grouping",
            format!("components.{}", registered.product_revision_id),
            format!("the flat component review leaves {} source bodies unassociated, so they stay uncosted", uncovered.len()),
        );
        issue.product_revision_id = Some(registered.product_revision_id);
        issues.push(issue);
    }
    EngineeringRecordView { project_id, version, document, issues }
}
