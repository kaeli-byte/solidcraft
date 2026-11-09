//! Engineering review and flat grouping: the reviewed component contract of package SB-11.
//!
//! The imported CAD stays immutable. A review records only engineer decisions, stored apart from
//! the original geometry and customer hierarchy:
//!
//! - A [`ReviewedComponent`] is one engineer-confirmed association of source-local bodies to one
//!   component of one product revision. On a flattened input several solids may be confirmed as
//!   one component; a preserved customer tree is reviewed one definition at a time. The
//!   association names the exact source and analysis it was reviewed against, so a changed source
//!   hash, engine build or extractor version never silently reuses it.
//! - A [`ReviewedPart`] holds the shared engineering metadata of one project part version: the
//!   proposed and confirmed classification and the reviewed volume. Every product revision and
//!   every occurrence that references that part version shares the record, while placements stay
//!   in the immutable import.
//! - A [`TrainingExampleMetadata`] is the persisted evidence of one confirmed label. It is
//!   metadata only: this contract claims no automatic training, model deployment or retroactive
//!   reclassification.
//!
//! Rules mirrored from the prototype and the confirmed requirements:
//!
//! - A missing value is `None`, never zero. An open surface has no volume until an engineer
//!   completes it manually; a manual value needs its reason and stays pending until a separate
//!   review confirmation.
//! - A proposed classification is pending until a person confirms a label. A confidence value is
//!   evidence, never an approval; nothing in this contract sets a confirmed label from a
//!   prediction.
//! - An association without a shared part version stays uncosted. This contract does not invent a
//!   nested hierarchy and does not treat one body as one component.
//!
//! Wire rules follow version 1 of the contract: decimals serialize as strings, a missing value is
//! `null`, unknown JSON fields are tolerated, and a stored document names the schema version it
//! was written with.

use std::collections::BTreeSet;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{ContractError, Result};
use crate::recognition::{
    CatalogRecord, ProjectCatalogOverride, SuggestionEvidence, check_bounded_text, check_optional_timestamp, validate_evidence,
};
use crate::types::{AnalysisKey, AnalysisManifest, MAX_SOURCE_PATH_DEPTH, Measurement, MeasurementSource, ReviewState, Unit, check_text};

/// The engineering review contract version this build reads and writes. It extends the published
/// version 2 HTTP API additively; it does not change a version 1 or version 2 meaning.
pub const ENGINEERING_REVIEW_CONTRACT_VERSION: u32 = 1;

/// The schema version written into a stored engineering record document.
pub const ENGINEERING_DOCUMENT_SCHEMA_VERSION: u32 = 1;

/// Resource bounds for hostile input. A review is not a dump of arbitrary size.
pub const MAX_REVIEWED_COMPONENTS: usize = 100_000;
pub const MAX_REVIEWED_PARTS: usize = 10_000;
pub const MAX_COMPONENT_GEOMETRY: usize = 100_000;
pub const MAX_TRAINING_EXAMPLES: usize = 100_000;
pub const MAX_CONFIRMATIONS: usize = 10_000;
pub const MAX_LABEL_CHARS: usize = 255;
pub const MAX_MODEL_VERSION_CHARS: usize = 255;
pub const MAX_REVIEWER_CHARS: usize = 255;
pub const MAX_REQUEST_ID_CHARS: usize = 128;

/// One source-local body: the imported tree path of its occurrence and the body index inside that
/// definition, exactly as the analysis manifest spells them. The owning component or analysis names
/// the source hash, engine build and extractor version, so the reference is never source-free.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct ReviewedGeometry {
    pub source_path: Vec<u64>,
    pub source_body: u32,
}

impl ReviewedGeometry {
    /// Bound the source path depth, as the manifest does.
    pub fn validate(&self, field: &str) -> Result<()> {
        if self.source_path.len() > MAX_SOURCE_PATH_DEPTH {
            return Err(ContractError::invalid(format!("{field}.source_path"), "the source path is too deep"));
        }
        Ok(())
    }
}

/// A proposed classification from recognition. A prediction is evidence, never an approval: only
/// a [`ConfirmedClassification`] enters costing, and nothing sets one from a confidence value.
///
/// Package SB-13 adds the optional provenance fields. They are defaulted, so a prediction stored
/// before SB-13 stays readable and valid; a prediction recorded through the SB-13 suggestion route
/// carries a prediction version, the exact analysis key and at least one evidence item.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClassificationPrediction {
    pub name: String,
    pub model_version: String,
    /// The model's own confidence, when it reports one. It never confirms the label.
    pub confidence: Option<Decimal>,
    /// The version of the suggestion algorithm or schema generation that produced the label.
    #[serde(default)]
    pub prediction_version: Option<String>,
    /// The exact analysis the proposal describes. A recorded suggestion binds one.
    #[serde(default)]
    pub analysis_key: Option<AnalysisKey>,
    /// The evidence the proposal shows. It never approves anything.
    #[serde(default)]
    pub evidence: Vec<SuggestionEvidence>,
    /// The RFC 3339 timestamp the service recorded, when it recorded one. Never client-authoritative.
    #[serde(default)]
    pub suggested_at: Option<String>,
}

impl ClassificationPrediction {
    /// The proposed label needs a name, the model that produced it, and a confidence in `0..=1`.
    /// The optional SB-13 provenance fields, when present, are bounded and valid.
    pub fn validate(&self, field: &str) -> Result<()> {
        validate_label(&self.name, &format!("{field}.name"))?;
        check_text(&self.model_version, &format!("{field}.model_version"))?;
        if self.model_version.chars().count() > MAX_MODEL_VERSION_CHARS {
            return Err(ContractError::invalid(format!("{field}.model_version"), "the model version is too long"));
        }
        if let Some(confidence) = self.confidence
            && (confidence.is_sign_negative() || confidence > Decimal::ONE)
        {
            return Err(ContractError::invalid(format!("{field}.confidence"), "a confidence is between 0 and 1"));
        }
        if let Some(prediction_version) = &self.prediction_version {
            check_bounded_text(prediction_version, &format!("{field}.prediction_version"), MAX_MODEL_VERSION_CHARS)?;
        }
        if let Some(analysis_key) = &self.analysis_key {
            analysis_key.validate()?;
        }
        validate_evidence(&self.evidence, &format!("{field}.evidence"))?;
        check_optional_timestamp(&self.suggested_at, &format!("{field}.suggested_at"))
    }
}

/// An engineer's confirmed classification label. Confirmation names the person whose decision it
/// is; the service fills the reviewer from the authenticated identity, never from the payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfirmedClassification {
    pub name: String,
    pub confirmed_by: String,
}

impl ConfirmedClassification {
    /// A confirmed label needs a name and the reviewer's identity.
    pub fn validate(&self, field: &str) -> Result<()> {
        validate_label(&self.name, &format!("{field}.name"))?;
        check_text(&self.confirmed_by, &format!("{field}.confirmed_by"))?;
        if self.confirmed_by.chars().count() > MAX_REVIEWER_CHARS {
            return Err(ContractError::invalid(format!("{field}.confirmed_by"), "the reviewer name is too long"));
        }
        Ok(())
    }
}

/// The classification state of one reviewed part version: an optional proposed label and the
/// engineer's confirmed label. A pending record is review work, not an approved classification.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClassificationRecord {
    pub prediction: Option<ClassificationPrediction>,
    pub confirmed: Option<ConfirmedClassification>,
}

impl ClassificationRecord {
    /// No proposed and no confirmed label yet.
    pub fn pending() -> Self {
        Self::default()
    }

    /// Whether an engineer has confirmed a label.
    pub fn is_confirmed(&self) -> bool {
        self.confirmed.is_some()
    }

    /// Validate the optional proposal and the optional confirmed label.
    pub fn validate(&self, field: &str) -> Result<()> {
        if let Some(prediction) = &self.prediction {
            prediction.validate(&format!("{field}.prediction"))?;
        }
        if let Some(confirmed) = &self.confirmed {
            confirmed.validate(&format!("{field}.confirmed"))?;
        }
        Ok(())
    }
}

/// The shared engineering metadata of one project part version. Every occurrence and product
/// revision that references the part version shares this record; placements stay in the import.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewedPart {
    pub part_version_id: Uuid,
    pub classification: ClassificationRecord,
    /// The reviewed volume, once one is recorded. A manual completion carries its source, unit and
    /// reason; an extracted value names the exact analysis it came from. Nothing recorded stays
    /// `None`, never zero, and an open surface is not given a volume.
    pub volume_mm3: Option<Measurement>,
    /// The authenticated identity that confirmed the recorded volume, when one is confirmed. The
    /// service fills it from the caller, never from the payload; a recorded or completed value
    /// carries no reviewer until the separate confirmation.
    pub volume_confirmed_by: Option<String>,
    /// The catalog match suggestions and the engineer-confirmed catalog reference of this part
    /// version. Package SB-13 adds it defaulted, so a reviewed part stored before SB-13 stays
    /// readable; only a confirmed reference supplies authoritative catalog inputs.
    #[serde(default)]
    pub catalog: CatalogRecord,
}

impl ReviewedPart {
    /// Validate the classification, the reviewed volume and the catalog record.
    pub fn validate(&self) -> Result<()> {
        self.classification.validate("reviewed_part.classification")?;
        self.catalog.validate("reviewed_part.catalog")?;
        if let Some(volume) = &self.volume_mm3 {
            volume.validate("reviewed_part.volume_mm3")?;
            if volume.unit != Unit::MillimetreCubed {
                return Err(ContractError::invalid("reviewed_part.volume_mm3.unit", "the reviewed volume is in mm3"));
            }
            if volume.review == ReviewState::Confirmed && volume.value.is_none() {
                return Err(ContractError::invalid(
                    "reviewed_part.volume_mm3.review",
                    "a confirmed measurement needs a value; a missing volume stays missing",
                ));
            }
        }
        let confirmed = self.volume_mm3.as_ref().is_some_and(|volume| volume.review == ReviewState::Confirmed);
        match (&self.volume_confirmed_by, confirmed) {
            (Some(reviewer), true) => {
                check_text(reviewer, "reviewed_part.volume_confirmed_by")?;
                if reviewer.chars().count() > MAX_REVIEWER_CHARS {
                    return Err(ContractError::invalid("reviewed_part.volume_confirmed_by", "the reviewer name is too long"));
                }
            }
            (Some(_), false) => {
                return Err(ContractError::invalid("reviewed_part.volume_confirmed_by", "a volume reviewer is recorded only on a confirmed volume"));
            }
            (None, true) => {
                return Err(ContractError::invalid(
                    "reviewed_part.volume_confirmed_by",
                    "a confirmed volume needs the authenticated reviewer who confirmed it",
                ));
            }
            (None, false) => {}
        }
        Ok(())
    }
}

/// One reviewed component of one product revision: the engineer's decision that these source-local
/// bodies are one component. Stored separately from the original import and customer hierarchy.
///
/// `part_version_id` is the shared project part version the component was associated with. Until an
/// engineer confirms one, the association is unresolved and stays uncosted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewedComponent {
    pub component_id: Uuid,
    pub product_revision_id: Uuid,
    pub analysis_key: AnalysisKey,
    pub geometry: Vec<ReviewedGeometry>,
    pub part_version_id: Option<Uuid>,
}

impl ReviewedComponent {
    /// Validate the analysis binding and the body references.
    pub fn validate(&self) -> Result<()> {
        self.analysis_key.validate()?;
        if self.geometry.is_empty() {
            return Err(ContractError::invalid("reviewed_component.geometry", "a reviewed component needs at least one body"));
        }
        if self.geometry.len() > MAX_COMPONENT_GEOMETRY {
            return Err(ContractError::invalid("reviewed_component.geometry", "the component holds too many bodies"));
        }
        let mut seen = BTreeSet::new();
        for geometry in &self.geometry {
            geometry.validate("reviewed_component.geometry")?;
            if !seen.insert(geometry.clone()) {
                return Err(ContractError::invalid("reviewed_component.geometry", "a body appears twice in one component"));
            }
        }
        Ok(())
    }
}

/// The persisted metadata of one confirmed label. This is evidence for later curation; it does not
/// train, deploy or reclassify anything by itself.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrainingExampleMetadata {
    pub example_id: Uuid,
    pub part_version_id: Uuid,
    pub component_id: Uuid,
    pub product_revision_id: Uuid,
    pub analysis_key: AnalysisKey,
    pub geometry: Vec<ReviewedGeometry>,
    pub prediction: Option<ClassificationPrediction>,
    pub confirmed_label: ConfirmedClassification,
}

impl TrainingExampleMetadata {
    /// Validate the exact analysis binding, the bodies and the confirmed label.
    pub fn validate(&self) -> Result<()> {
        self.analysis_key.validate()?;
        if self.geometry.is_empty() {
            return Err(ContractError::invalid("training_example.geometry", "a training example needs at least one body"));
        }
        if self.geometry.len() > MAX_COMPONENT_GEOMETRY {
            return Err(ContractError::invalid("training_example.geometry", "the example holds too many bodies"));
        }
        let mut seen = BTreeSet::new();
        for geometry in &self.geometry {
            geometry.validate("training_example.geometry")?;
            if !seen.insert(geometry.clone()) {
                return Err(ContractError::invalid("training_example.geometry", "a body appears twice in one example"));
            }
        }
        if let Some(prediction) = &self.prediction {
            prediction.validate("training_example.prediction")?;
        }
        self.confirmed_label.validate("training_example.confirmed_label")
    }
}

/// One computed problem of an engineering review. Issues are never stored; a missing input is an
/// issue, not a zero, and an unresolved association stays uncosted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EngineeringIssue {
    pub code: String,
    pub field_path: String,
    pub component_id: Option<Uuid>,
    pub product_revision_id: Option<Uuid>,
    pub part_version_id: Option<Uuid>,
    pub message: String,
}

impl EngineeringIssue {
    /// A named issue. `code` is a stable contract name, not display text.
    pub fn new(code: impl Into<String>, field_path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            field_path: field_path.into(),
            component_id: None,
            product_revision_id: None,
            part_version_id: None,
            message: message.into(),
        }
    }

    /// Attach the component the issue is about.
    pub fn for_component(mut self, component_id: Uuid, product_revision_id: Uuid) -> Self {
        self.component_id = Some(component_id);
        self.product_revision_id = Some(product_revision_id);
        self
    }

    /// Attach the part version the issue is about.
    pub fn for_part(mut self, part_version_id: Uuid) -> Self {
        self.part_version_id = Some(part_version_id);
        self
    }
}

/// The stored, appended engineering record of one project. History lives in the immutable record
/// versions; this document is the current one.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EngineeringRecordDocument {
    pub schema_version: u32,
    pub project_id: Uuid,
    pub components: Vec<ReviewedComponent>,
    pub parts: Vec<ReviewedPart>,
    pub training_examples: Vec<TrainingExampleMetadata>,
}

impl EngineeringRecordDocument {
    /// The empty review of a project with no recorded decision yet.
    pub fn empty(project_id: Uuid) -> Self {
        Self {
            schema_version: ENGINEERING_DOCUMENT_SCHEMA_VERSION,
            project_id,
            components: Vec::new(),
            parts: Vec::new(),
            training_examples: Vec::new(),
        }
    }

    /// Validate a stored document before it is written and before clients read one.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != ENGINEERING_DOCUMENT_SCHEMA_VERSION {
            return Err(ContractError::SchemaVersion { found: self.schema_version, expected: ENGINEERING_DOCUMENT_SCHEMA_VERSION });
        }
        if self.components.len() > MAX_REVIEWED_COMPONENTS {
            return Err(ContractError::invalid("engineering.components", format!("a review holds at most {MAX_REVIEWED_COMPONENTS} components")));
        }
        if self.parts.len() > MAX_REVIEWED_PARTS {
            return Err(ContractError::invalid("engineering.parts", format!("a review holds at most {MAX_REVIEWED_PARTS} reviewed parts")));
        }
        if self.training_examples.len() > MAX_TRAINING_EXAMPLES {
            return Err(ContractError::invalid(
                "engineering.training_examples",
                format!("a review holds at most {MAX_TRAINING_EXAMPLES} training examples"),
            ));
        }
        let mut component_ids = BTreeSet::new();
        for component in &self.components {
            component.validate()?;
            if !component_ids.insert(component.component_id) {
                return Err(ContractError::invalid("engineering.components", "two reviewed components share one component id"));
            }
        }
        let mut part_versions = BTreeSet::new();
        for part in &self.parts {
            part.validate()?;
            if !part_versions.insert(part.part_version_id) {
                return Err(ContractError::invalid("engineering.parts", "two reviewed part records share one part version id"));
            }
        }
        let mut example_ids = BTreeSet::new();
        for example in &self.training_examples {
            example.validate()?;
            if !example_ids.insert(example.example_id) {
                return Err(ContractError::invalid("engineering.training_examples", "two training examples share one example id"));
            }
        }
        Ok(())
    }

    /// The reviewed component with `component_id`.
    pub fn component(&self, component_id: Uuid) -> Option<&ReviewedComponent> {
        self.components.iter().find(|component| component.component_id == component_id)
    }

    /// The reviewed part record of `part_version_id`.
    pub fn part(&self, part_version_id: Uuid) -> Option<&ReviewedPart> {
        self.parts.iter().find(|part| part.part_version_id == part_version_id)
    }

    /// The mutable reviewed part record of `part_version_id`, when one exists.
    pub fn part_mut(&mut self, part_version_id: Uuid) -> Option<&mut ReviewedPart> {
        self.parts.iter_mut().find(|part| part.part_version_id == part_version_id)
    }

    /// The reviewed components of one product revision and one exact analysis.
    pub fn components_of(&self, product_revision_id: Uuid, analysis_key: &AnalysisKey) -> impl Iterator<Item = &ReviewedComponent> {
        self.components
            .iter()
            .filter(move |component| component.product_revision_id == product_revision_id && component.analysis_key == *analysis_key)
    }

    /// The reviewed components associated with one shared part version, in document order.
    pub fn components_for_part(&self, part_version_id: Uuid) -> impl Iterator<Item = &ReviewedComponent> {
        self.components.iter().filter(move |component| component.part_version_id == Some(part_version_id))
    }

    /// The computed issues of this review: unresolved associations and incomplete part reviews.
    /// Grouping completeness against an analysis manifest is computed where the manifest is known,
    /// so no single-body assumption is made here.
    pub fn issues(&self) -> Vec<EngineeringIssue> {
        let mut issues = Vec::new();
        for component in &self.components {
            if component.part_version_id.is_none() {
                issues.push(
                    EngineeringIssue::new(
                        "unresolved_component_association",
                        format!("components.{}.part_version_id", component.component_id),
                        "the reviewed component has no confirmed shared part version, so it stays uncosted",
                    )
                    .for_component(component.component_id, component.product_revision_id),
                );
            }
        }
        // A part version referenced by a reviewed component is under review even before any
        // measurement or label is recorded for it; it is uncosted until both are confirmed.
        let reviewed: BTreeSet<Uuid> = self.parts.iter().map(|part| part.part_version_id).collect();
        let mut unrecorded = BTreeSet::new();
        for component in &self.components {
            if let Some(part_version_id) = component.part_version_id
                && !reviewed.contains(&part_version_id)
                && unrecorded.insert(part_version_id)
            {
                issues.push(
                    EngineeringIssue::new(
                        "pending_classification",
                        format!("parts.{part_version_id}.classification.confirmed"),
                        "the classification is a proposal until an engineer confirms a label",
                    )
                    .for_part(part_version_id),
                );
                issues.push(
                    EngineeringIssue::new(
                        "missing_reviewed_volume",
                        format!("parts.{part_version_id}.volume_mm3"),
                        "no usable volume is recorded; complete it manually before costing",
                    )
                    .for_part(part_version_id),
                );
            }
        }
        for part in &self.parts {
            if !part.classification.is_confirmed() {
                issues.push(
                    EngineeringIssue::new(
                        "pending_classification",
                        format!("parts.{}.classification.confirmed", part.part_version_id),
                        "the classification is a proposal until an engineer confirms a label",
                    )
                    .for_part(part.part_version_id),
                );
            }
            match &part.volume_mm3 {
                None => issues.push(
                    EngineeringIssue::new(
                        "missing_reviewed_volume",
                        format!("parts.{}.volume_mm3", part.part_version_id),
                        "no usable volume is recorded; complete it manually before costing",
                    )
                    .for_part(part.part_version_id),
                ),
                Some(volume) if volume.review != ReviewState::Confirmed => issues.push(
                    EngineeringIssue::new(
                        "pending_measurement_review",
                        format!("parts.{}.volume_mm3.review", part.part_version_id),
                        "the recorded volume is not confirmed by an engineer yet",
                    )
                    .for_part(part.part_version_id),
                ),
                Some(_) => {}
            }
        }
        issues
    }
}

/// One immutable review version as a client reads it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EngineeringRecordView {
    pub project_id: Uuid,
    pub version: u64,
    pub document: EngineeringRecordDocument,
    pub issues: Vec<EngineeringIssue>,
}

/// The compact receipt of one committed engineering write.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EngineeringReceipt {
    pub request_id: String,
    pub project_id: Uuid,
    pub version: u64,
}

/// One committed engineering write: its receipt and the dependency facts it emitted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EngineeringWriteOutcome {
    pub receipt: EngineeringReceipt,
    pub facts: Vec<EngineeringFact>,
}

/// One dependency fact an engineering write emits. Facts name what changed; they never rewrite
/// accepted results, and later packages own any recalculation they imply.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EngineeringFact {
    /// The engineer confirmed or changed the bodies of one component. `geometry` is the resulting
    /// body count.
    ComponentGrouped { component_id: Uuid, product_revision_id: Uuid, geometry: u64 },
    /// The engineer dissolved a component; its bodies are unassociated again.
    ComponentDissolved { component_id: Uuid },
    /// The component's shared part version association changed; `None` means unresolved again.
    ComponentPartAssociated { component_id: Uuid, part_version_id: Option<Uuid> },
    /// A measured volume was recorded for review; a manual value stays pending until confirmed.
    MeasurementRecorded { part_version_id: Uuid, source: MeasurementSource, review: ReviewState },
    /// Recognition recorded classification and/or catalog proposals. The counts are proposals;
    /// nothing is confirmed, and an unconfirmed proposal supplies no authoritative input.
    SuggestionsRecorded { classification: u64, catalog: u64 },
    /// An engineer confirmed a classification and/or the measured volume of one part version.
    PartConfirmed { part_version_id: Uuid, classification_confirmed: bool, measurement_confirmed: bool },
    /// An engineer confirmed one catalog component version for one part version.
    CatalogMatchConfirmed { part_version_id: Uuid },
    /// Confirmed labels were captured as training-example metadata. Nothing is trained or deployed.
    TrainingExamplesCaptured { part_version_id: Uuid, count: u64 },
}

/// Register the exact analysis of one product revision. The review records body references inside
/// that analysis, so the source hash, engine build and extractor version are bound once.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegisterAnalysis {
    pub request_id: String,
    pub product_revision_id: Uuid,
    pub manifest: AnalysisManifest,
}

impl RegisterAnalysis {
    /// Validate the request and the analysis before the service stores it.
    pub fn validate(&self) -> Result<()> {
        validate_request_id(&self.request_id, "register_analysis.request_id")?;
        self.manifest.validate()
    }
}

/// The registered analysis identity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnalysisReceipt {
    pub request_id: String,
    pub product_revision_id: Uuid,
    pub analysis_key: AnalysisKey,
}

impl AnalysisReceipt {
    /// Build the receipt for a stored request.
    pub fn of(request_id: impl Into<String>, product_revision_id: Uuid, manifest: &AnalysisManifest) -> Self {
        Self { request_id: request_id.into(), product_revision_id, analysis_key: manifest.key.clone() }
    }

    /// A receipt names its request, revision and analysis.
    pub fn validate(&self) -> Result<()> {
        validate_request_id(&self.request_id, "analysis_receipt.request_id")?;
        self.analysis_key.validate()
    }
}

/// Group source-local bodies into one reviewed component of one product revision, or replace the
/// grouping of an existing component. Submitting no geometry dissolves an existing component; a
/// new component needs at least one body.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GroupSolids {
    pub request_id: String,
    pub expected_version: u64,
    pub component_id: Uuid,
    pub product_revision_id: Uuid,
    pub analysis_key: AnalysisKey,
    pub geometry: Vec<ReviewedGeometry>,
    pub part_version_id: Option<Uuid>,
}

impl GroupSolids {
    /// Validate the request before the service writes it. The service also proves the bodies exist
    /// in the registered analysis and are associated once.
    pub fn validate(&self) -> Result<()> {
        validate_request_id(&self.request_id, "group_solids.request_id")?;
        self.analysis_key.validate()?;
        if self.geometry.len() > MAX_COMPONENT_GEOMETRY {
            return Err(ContractError::invalid("group_solids.geometry", "the component holds too many bodies"));
        }
        let mut seen = BTreeSet::new();
        for geometry in &self.geometry {
            geometry.validate("group_solids.geometry")?;
            if !seen.insert(geometry.clone()) {
                return Err(ContractError::invalid("group_solids.geometry", "a body appears twice in one component"));
            }
        }
        Ok(())
    }
}

/// Associate one reviewed component with the shared project part version it belongs to, or clear
/// the association when the engineer decides the component is unresolved again.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssociateComponentPart {
    pub request_id: String,
    pub expected_version: u64,
    pub component_id: Uuid,
    pub part_version_id: Option<Uuid>,
}

impl AssociateComponentPart {
    /// Validate the request before the service writes it.
    pub fn validate(&self) -> Result<()> {
        validate_request_id(&self.request_id, "associate_component_part.request_id")
    }
}

/// Complete or replace the reviewed volume of one part version. A manual value carries its unit,
/// source and reason and stays pending; only the confirmation action marks it reviewed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompleteMeasurement {
    pub request_id: String,
    pub expected_version: u64,
    pub part_version_id: Uuid,
    pub volume_mm3: Measurement,
}

impl CompleteMeasurement {
    /// Validate the request and the measurement before the service writes it.
    pub fn validate(&self) -> Result<()> {
        validate_request_id(&self.request_id, "complete_measurement.request_id")?;
        self.volume_mm3.validate("complete_measurement.volume_mm3")?;
        if self.volume_mm3.unit != Unit::MillimetreCubed {
            return Err(ContractError::invalid("complete_measurement.volume_mm3.unit", "the reviewed volume is in mm3"));
        }
        if self.volume_mm3.value.is_none() {
            return Err(ContractError::invalid("complete_measurement.volume_mm3.value", "a completed measurement needs a value"));
        }
        if self.volume_mm3.review == ReviewState::Confirmed {
            return Err(ContractError::invalid(
                "complete_measurement.volume_mm3.review",
                "a recorded measurement is reviewed by the confirmation action, not by the completion itself",
            ));
        }
        Ok(())
    }
}

/// One engineer's confirmed label for one part version. The label is copied into the reviewed part
/// and becomes one training-example metadata row per associated component.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClassificationConfirmation {
    pub name: String,
}

impl ClassificationConfirmation {
    /// Validate the confirmed label.
    pub fn validate(&self, field: &str) -> Result<()> {
        validate_label(&self.name, &format!("{field}.name"))
    }
}

/// One engineer decision inside a bulk confirmation: confirm or correct a classification, confirm
/// the recorded volume, confirm a catalog component version (with an optional project override), or
/// any combination of them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PartConfirmation {
    pub part_version_id: Uuid,
    pub classification: Option<ClassificationConfirmation>,
    pub confirm_measurement: bool,
    /// Package SB-13 adds the catalog decision. Defaulted, so a confirmation stored before SB-13
    /// stays readable; the service resolves the catalog values and the reviewer.
    #[serde(default)]
    pub catalog: Option<CatalogConfirmation>,
}

impl PartConfirmation {
    /// At least one decision is required; a confirmation that confirms nothing is refused.
    pub fn validate(&self, field: &str) -> Result<()> {
        if self.classification.is_none() && !self.confirm_measurement && self.catalog.is_none() {
            return Err(ContractError::invalid(
                field,
                "a confirmation needs a classification decision, a measurement review, a catalog decision, or a combination",
            ));
        }
        if let Some(classification) = &self.classification {
            classification.validate(&format!("{field}.classification"))?;
        }
        if let Some(catalog) = &self.catalog {
            catalog.validate(&format!("{field}.catalog"))?;
        }
        Ok(())
    }
}

/// An engineer's decision to confirm one catalog component version for one part version, with an
/// optional project override recorded beside the retained catalog reference.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CatalogConfirmation {
    /// The exact catalog component version the engineer confirms.
    pub component_version_id: Uuid,
    /// The project override of the confirmed catalog values, when the project replaces them.
    pub project_override: Option<ProjectCatalogOverride>,
}

impl CatalogConfirmation {
    /// Validate the optional override. The service resolves the referenced catalog version; the
    /// override's own rules require a reason and an authenticated reviewer.
    pub fn validate(&self, field: &str) -> Result<()> {
        if let Some(project_override) = &self.project_override {
            project_override.validate(&format!("{field}.project_override"))?;
        }
        Ok(())
    }
}

/// Bulk confirmation of engineer decisions: correct predictions can be accepted together and
/// exceptions corrected in one write. A high confidence value never confirms by itself.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConfirmParts {
    pub request_id: String,
    pub expected_version: u64,
    pub confirmations: Vec<PartConfirmation>,
}

impl ConfirmParts {
    /// Validate the request before the service writes it.
    pub fn validate(&self) -> Result<()> {
        validate_request_id(&self.request_id, "confirm_parts.request_id")?;
        if self.confirmations.is_empty() {
            return Err(ContractError::invalid("confirm_parts.confirmations", "a confirmation write names at least one part version"));
        }
        if self.confirmations.len() > MAX_CONFIRMATIONS {
            return Err(ContractError::invalid("confirm_parts.confirmations", "the confirmation write names too many part versions"));
        }
        let mut seen = BTreeSet::new();
        for (index, confirmation) in self.confirmations.iter().enumerate() {
            confirmation.validate(&format!("confirm_parts.confirmations.{index}"))?;
            if !seen.insert(confirmation.part_version_id) {
                return Err(ContractError::invalid("confirm_parts.confirmations", "a part version appears twice in one confirmation write"));
            }
        }
        Ok(())
    }
}

/// Every body of a registered analysis that the reviewed components of one product revision do not
/// cover. The service computes it from the registered analysis; it never assumes one body is one
/// component and it never invents a grouping.
pub fn unassociated_geometry(document: &EngineeringRecordDocument, product_revision_id: Uuid, manifest: &AnalysisManifest) -> Vec<ReviewedGeometry> {
    let associated: BTreeSet<ReviewedGeometry> =
        document.components_of(product_revision_id, &manifest.key).flat_map(|component| component.geometry.iter().cloned()).collect();
    let mut uncovered = Vec::new();
    let mut seen = BTreeSet::new();
    for binding in &manifest.geometry_bindings {
        let geometry = ReviewedGeometry { source_path: binding.source_path.clone(), source_body: binding.source_body };
        if seen.insert(geometry.clone()) && !associated.contains(&geometry) {
            uncovered.push(geometry);
        }
    }
    uncovered
}

/// The definition one component body reference belongs to in the exact analysis, when it exists.
pub fn geometry_definition(manifest: &AnalysisManifest, geometry: &ReviewedGeometry) -> Option<Uuid> {
    manifest
        .geometry_bindings
        .iter()
        .find(|binding| binding.source_path == geometry.source_path && binding.source_body == geometry.source_body)
        .map(|binding| binding.definition_id)
}

fn validate_label(name: &str, field: &str) -> Result<()> {
    check_text(name, field)?;
    if name.chars().count() > MAX_LABEL_CHARS {
        return Err(ContractError::invalid(field, "the label is too long"));
    }
    Ok(())
}

fn validate_request_id(request_id: &str, field: &str) -> Result<()> {
    check_text(request_id, field)?;
    if request_id.chars().count() > MAX_REQUEST_ID_CHARS {
        return Err(ContractError::invalid(field, "the request id is too long"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{DefinitionRef, GeometryBinding, Issue, OccurrenceRef, SCHEMA_VERSION};

    const IDENTITY: [[f64; 4]; 4] = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];

    fn key() -> AnalysisKey {
        AnalysisKey { source_sha256: "a".repeat(64), engine_build: "0.40.0".into(), extractor_version: "2".into() }
    }

    fn measurement(value: Option<&str>, review: ReviewState) -> Measurement {
        Measurement {
            value: value.map(|text| text.parse().unwrap_or_default()),
            unit: Unit::MillimetreCubed,
            source: MeasurementSource::Extracted,
            review,
            reason: None,
            analysis_key: key(),
        }
    }

    fn manual(value: &str) -> Measurement {
        Measurement {
            value: Some(value.parse().unwrap_or_default()),
            unit: Unit::MillimetreCubed,
            source: MeasurementSource::Manual,
            review: ReviewState::Pending,
            reason: Some("open surface; measured on the bench".into()),
            analysis_key: key(),
        }
    }

    fn geometry(source_path: Vec<u64>, source_body: u32) -> ReviewedGeometry {
        ReviewedGeometry { source_path, source_body }
    }

    fn component(parts: Option<Uuid>, geometry: Vec<ReviewedGeometry>) -> ReviewedComponent {
        ReviewedComponent {
            component_id: Uuid::from_u128(1),
            product_revision_id: Uuid::from_u128(2),
            analysis_key: key(),
            geometry,
            part_version_id: parts,
        }
    }

    fn label(name: &str) -> ConfirmedClassification {
        ConfirmedClassification { name: name.into(), confirmed_by: "engineer".into() }
    }

    fn proposed_prediction() -> ClassificationPrediction {
        ClassificationPrediction {
            name: "clip".into(),
            model_version: "recognition-1".into(),
            confidence: Some("0.5".parse().unwrap_or_default()),
            prediction_version: None,
            analysis_key: None,
            evidence: Vec::new(),
            suggested_at: None,
        }
    }

    fn manifest() -> AnalysisManifest {
        AnalysisManifest {
            schema_version: SCHEMA_VERSION,
            key: key(),
            definitions: vec![DefinitionRef {
                definition_id: Uuid::from_u128(10),
                source_key: key(),
                customer_part_number: None,
                customer_revision: None,
                volume_mm3: measurement(Some("7200"), ReviewState::Pending),
            }],
            occurrences: vec![OccurrenceRef {
                occurrence_id: Uuid::from_u128(20),
                definition_id: Uuid::from_u128(10),
                parent_occurrence_id: None,
                source_path: Vec::new(),
                transform: IDENTITY,
            }],
            geometry_bindings: vec![
                GeometryBinding { source_path: Vec::new(), source_body: 0, definition_id: Uuid::from_u128(10), occurrence_id: Uuid::from_u128(20) },
                GeometryBinding { source_path: Vec::new(), source_body: 1, definition_id: Uuid::from_u128(10), occurrence_id: Uuid::from_u128(20) },
            ],
            issues: vec![Issue::new("unresolved_grouping", "analysis.definitions", "the flat input needs a component review")],
        }
    }

    #[test]
    fn an_empty_review_has_no_components_parts_or_examples() {
        let document = EngineeringRecordDocument::empty(Uuid::from_u128(3));
        assert_eq!(document.schema_version, ENGINEERING_DOCUMENT_SCHEMA_VERSION);
        assert!(document.components.is_empty());
        assert!(document.parts.is_empty());
        assert!(document.training_examples.is_empty());
        assert!(document.validate().is_ok());
        assert!(document.issues().is_empty());
    }

    #[test]
    fn a_component_needs_bodies_inside_one_analysis_and_each_body_once() {
        let valid = component(None, vec![geometry(Vec::new(), 0), geometry(Vec::new(), 1)]);
        assert!(valid.validate().is_ok());
        assert!(component(None, Vec::new()).validate().is_err());
        assert!(component(None, vec![geometry(Vec::new(), 0), geometry(Vec::new(), 0)]).validate().is_err());
        let deep = component(None, vec![geometry((0..=MAX_SOURCE_PATH_DEPTH as u64).collect(), 0)]);
        assert!(deep.validate().is_err());
    }

    #[test]
    fn a_prediction_never_confirms_a_classification() {
        let proposed = ClassificationPrediction {
            name: "clip".into(),
            model_version: "recognition-1".into(),
            confidence: Some("0.99".parse().unwrap_or_default()),
            prediction_version: Some("1".into()),
            analysis_key: Some(key()),
            evidence: vec![SuggestionEvidence::new("geometry_rule", "an analytic rule proposed the label").unwrap()],
            suggested_at: None,
        };
        let pending = ClassificationRecord { prediction: Some(proposed.clone()), confirmed: None };
        assert!(!pending.is_confirmed(), "high confidence cannot confirm automatically");
        assert!(pending.validate("classification").is_ok());
        let confirmed = ClassificationRecord { prediction: Some(proposed), confirmed: Some(label("clip")) };
        assert!(confirmed.is_confirmed());
        assert!(confirmed.validate("classification").is_ok());
        let refused_by_service = ConfirmedClassification { name: "clip".into(), confirmed_by: " ".into() };
        assert!(refused_by_service.validate("classification.confirmed").is_err());
        let out_of_range = ClassificationPrediction { confidence: Some("1.5".parse().unwrap_or_default()), ..proposed_prediction() };
        assert!(out_of_range.validate("prediction").is_err());
        let blank = ClassificationPrediction { name: " ".into(), ..proposed_prediction() };
        assert!(blank.validate("prediction").is_err());
    }

    #[test]
    fn a_confirmed_volume_needs_a_value_and_a_manual_one_needs_its_reason() {
        let confirmed = ReviewedPart {
            part_version_id: Uuid::from_u128(4),
            classification: ClassificationRecord::pending(),
            volume_mm3: Some(measurement(Some("2400"), ReviewState::Confirmed)),
            volume_confirmed_by: Some("engineer".into()),
            catalog: CatalogRecord::pending(),
        };
        assert!(confirmed.validate().is_ok());
        let missing = ReviewedPart {
            part_version_id: Uuid::from_u128(4),
            classification: ClassificationRecord::pending(),
            volume_mm3: Some(measurement(None, ReviewState::Confirmed)),
            volume_confirmed_by: Some("engineer".into()),
            catalog: CatalogRecord::pending(),
        };
        assert!(missing.validate().is_err());
        let missing_pending = ReviewedPart {
            part_version_id: Uuid::from_u128(4),
            classification: ClassificationRecord::pending(),
            volume_mm3: Some(measurement(None, ReviewState::Pending)),
            volume_confirmed_by: None,
            catalog: CatalogRecord::pending(),
        };
        assert!(missing_pending.validate().is_ok());
        let manual_value = ReviewedPart {
            part_version_id: Uuid::from_u128(4),
            classification: ClassificationRecord::pending(),
            volume_mm3: Some(manual("2500")),
            volume_confirmed_by: None,
            catalog: CatalogRecord::pending(),
        };
        assert!(manual_value.validate().is_ok());
        assert_eq!(manual_value.volume_mm3.as_ref().map(|volume| volume.review), Some(ReviewState::Pending));
        let wrong_unit = Measurement { unit: Unit::Second, ..manual("2500") };
        assert!(ReviewedPart { volume_mm3: Some(wrong_unit), ..manual_value.clone() }.validate().is_err());
        assert!(ReviewedPart { volume_mm3: None, ..manual_value }.validate().is_ok(), "no measurement recorded yet is valid");

        // A confirmed volume keeps the authenticated reviewer; a pending or missing value has none.
        assert!(ReviewedPart { volume_confirmed_by: None, ..confirmed.clone() }.validate().is_err());
        assert!(ReviewedPart { volume_confirmed_by: Some(" ".into()), ..confirmed.clone() }.validate().is_err());
        let pending_value = ReviewedPart { volume_mm3: Some(manual("2500")), volume_confirmed_by: Some("engineer".into()), ..confirmed };
        assert!(pending_value.validate().is_err(), "a reviewer is recorded only on a confirmed volume");
    }

    #[test]
    fn issues_report_unresolved_associations_and_incomplete_reviews() {
        let mut document = EngineeringRecordDocument::empty(Uuid::from_u128(3));
        document.components.push(component(None, vec![geometry(Vec::new(), 0)]));
        document.parts.push(ReviewedPart {
            part_version_id: Uuid::from_u128(4),
            classification: ClassificationRecord { prediction: None, confirmed: None },
            volume_mm3: None,
            volume_confirmed_by: None,
            catalog: CatalogRecord::pending(),
        });
        let issues = document.issues();
        let codes: Vec<&str> = issues.iter().map(|issue| issue.code.as_str()).collect();
        assert!(codes.contains(&"unresolved_component_association"), "{codes:?}");
        assert!(codes.contains(&"pending_classification"), "{codes:?}");
        assert!(codes.contains(&"missing_reviewed_volume"), "{codes:?}");
        assert!(document.validate().is_ok());

        // An associated part version is under review even before a record exists for it.
        document.components[0].part_version_id = Some(Uuid::from_u128(4));
        document.parts.clear();
        let issues = document.issues();
        let codes: Vec<&str> = issues.iter().map(|issue| issue.code.as_str()).collect();
        assert!(codes.contains(&"pending_classification"), "{codes:?}");
        assert!(codes.contains(&"missing_reviewed_volume"), "{codes:?}");
        assert!(!codes.contains(&"unresolved_component_association"), "{codes:?}");
        document.parts.push(ReviewedPart {
            part_version_id: Uuid::from_u128(4),
            classification: ClassificationRecord::default(),
            volume_mm3: None,
            volume_confirmed_by: None,
            catalog: CatalogRecord::pending(),
        });

        let mut reviewed = document.clone();
        reviewed.components[0].part_version_id = Some(Uuid::from_u128(4));
        reviewed.parts[0].classification.confirmed = Some(label("clip"));
        reviewed.parts[0].volume_mm3 = Some(measurement(Some("2400"), ReviewState::Confirmed));
        reviewed.parts[0].volume_confirmed_by = Some("engineer".into());
        assert!(reviewed.issues().is_empty(), "{:?}", reviewed.issues());
        assert!(reviewed.validate().is_ok());

        let mut pending = reviewed;
        pending.parts[0].volume_mm3 = Some(measurement(Some("2400"), ReviewState::Pending));
        pending.parts[0].volume_confirmed_by = None;
        let issues = pending.issues();
        let codes: Vec<&str> = issues.iter().map(|issue| issue.code.as_str()).collect();
        assert_eq!(codes, vec!["pending_measurement_review"]);
    }

    #[test]
    fn a_record_refuses_duplicate_identities_and_a_newer_schema() {
        let mut document = EngineeringRecordDocument::empty(Uuid::from_u128(3));
        document.components.push(component(None, vec![geometry(Vec::new(), 0)]));
        document.components.push(component(None, vec![geometry(Vec::new(), 1)]));
        assert!(document.validate().is_err(), "two components share one id");
        document.components.pop();
        assert!(document.validate().is_ok());
        document.schema_version = ENGINEERING_DOCUMENT_SCHEMA_VERSION + 1;
        assert!(matches!(document.validate(), Err(ContractError::SchemaVersion { .. })));
    }

    #[test]
    fn a_training_example_is_self_contained_metadata() {
        let example = TrainingExampleMetadata {
            example_id: Uuid::from_u128(5),
            part_version_id: Uuid::from_u128(4),
            component_id: Uuid::from_u128(1),
            product_revision_id: Uuid::from_u128(2),
            analysis_key: key(),
            geometry: vec![geometry(Vec::new(), 0), geometry(Vec::new(), 1)],
            prediction: Some(ClassificationPrediction {
                name: "clip".into(),
                model_version: "recognition-1".into(),
                confidence: None,
                prediction_version: None,
                analysis_key: None,
                evidence: Vec::new(),
                suggested_at: None,
            }),
            confirmed_label: label("clip"),
        };
        assert!(example.validate().is_ok());
        assert!(TrainingExampleMetadata { geometry: Vec::new(), ..example.clone() }.validate().is_err());
        assert!(
            TrainingExampleMetadata { confirmed_label: ConfirmedClassification { name: " ".into(), confirmed_by: "e".into() }, ..example }
                .validate()
                .is_err()
        );
    }

    #[test]
    fn a_group_request_accepts_an_empty_dissolve_but_refuses_duplicate_bodies() {
        let valid = GroupSolids {
            request_id: "group-1".into(),
            expected_version: 0,
            component_id: Uuid::from_u128(1),
            product_revision_id: Uuid::from_u128(2),
            analysis_key: key(),
            geometry: vec![geometry(Vec::new(), 0), geometry(Vec::new(), 1)],
            part_version_id: None,
        };
        assert!(valid.validate().is_ok());
        assert!(GroupSolids { geometry: Vec::new(), ..valid.clone() }.validate().is_ok(), "an emptied grouping dissolves the component");
        assert!(GroupSolids { geometry: vec![geometry(Vec::new(), 0), geometry(Vec::new(), 0)], ..valid }.validate().is_err());
    }

    #[test]
    fn a_completion_cannot_claim_its_own_review() {
        let request = CompleteMeasurement {
            request_id: "measure-1".into(),
            expected_version: 0,
            part_version_id: Uuid::from_u128(4),
            volume_mm3: manual("2500"),
        };
        assert!(request.validate().is_ok());
        assert!(CompleteMeasurement { volume_mm3: measurement(Some("2500"), ReviewState::Confirmed), ..request.clone() }.validate().is_err());
        assert!(CompleteMeasurement { volume_mm3: measurement(None, ReviewState::Pending), ..request.clone() }.validate().is_err());
        let manual_without_reason = Measurement { reason: None, ..manual("2500") };
        assert!(CompleteMeasurement { volume_mm3: manual_without_reason, ..request }.validate().is_err());
    }

    #[test]
    fn a_bulk_confirmation_needs_a_decision_per_part_version() {
        let confirm = ConfirmParts {
            request_id: "confirm-1".into(),
            expected_version: 0,
            confirmations: vec![
                PartConfirmation {
                    part_version_id: Uuid::from_u128(4),
                    classification: Some(ClassificationConfirmation { name: "clip".into() }),
                    confirm_measurement: true,
                    catalog: None,
                },
                PartConfirmation { part_version_id: Uuid::from_u128(5), classification: None, confirm_measurement: true, catalog: None },
            ],
        };
        assert!(confirm.validate().is_ok());
        let nothing = ConfirmParts {
            confirmations: vec![PartConfirmation {
                part_version_id: Uuid::from_u128(4),
                classification: None,
                confirm_measurement: false,
                catalog: None,
            }],
            ..confirm.clone()
        };
        assert!(nothing.validate().is_err());
        assert!(ConfirmParts { confirmations: Vec::new(), ..confirm.clone() }.validate().is_err());
        let duplicate = ConfirmParts { confirmations: vec![confirm.confirmations[0].clone(), confirm.confirmations[0].clone()], ..confirm };
        assert!(duplicate.validate().is_err());
    }

    #[test]
    fn the_analysis_helpers_see_exactly_the_registered_bodies() {
        let manifest = manifest();
        let mut document = EngineeringRecordDocument::empty(Uuid::from_u128(3));
        assert_eq!(unassociated_geometry(&document, Uuid::from_u128(2), &manifest).len(), 2);
        document.components.push(component(Some(Uuid::from_u128(4)), vec![geometry(Vec::new(), 0)]));
        let uncovered = unassociated_geometry(&document, Uuid::from_u128(2), &manifest);
        assert_eq!(uncovered, vec![geometry(Vec::new(), 1)]);
        assert_eq!(geometry_definition(&manifest, &geometry(Vec::new(), 1)), Some(Uuid::from_u128(10)));
        assert_eq!(geometry_definition(&manifest, &geometry(Vec::new(), 9)), None);
        assert!(manifest.validate().is_ok());
    }

    #[test]
    fn a_registered_analysis_receipt_names_its_exact_source_identity() {
        let manifest = manifest();
        let receipt = AnalysisReceipt::of("analysis-1", Uuid::from_u128(2), &manifest);
        assert!(receipt.validate().is_ok());
        assert_eq!(receipt.analysis_key, manifest.key);
        assert!(AnalysisReceipt { request_id: " ".into(), ..receipt }.validate().is_err());
        let register = RegisterAnalysis { request_id: "analysis-1".into(), product_revision_id: Uuid::from_u128(2), manifest };
        assert!(register.validate().is_ok());
        let mut broken = register.clone();
        broken.manifest.key.source_sha256 = "A".repeat(64);
        assert!(broken.validate().is_err());
    }

    #[test]
    fn an_old_prediction_without_suggestion_provenance_stays_readable_and_valid() {
        let old = serde_json::json!({
            "name": "clip",
            "model_version": "recognition-0",
            "confidence": "0.9",
        });
        let prediction: ClassificationPrediction = serde_json::from_value(old).unwrap();
        assert!(prediction.prediction_version.is_none());
        assert!(prediction.analysis_key.is_none());
        assert!(prediction.evidence.is_empty());
        assert!(prediction.suggested_at.is_none());
        assert!(prediction.validate("prediction").is_ok(), "a stored prediction without the SB-13 fields stays valid");

        let recorded = ClassificationPrediction {
            name: "tube".into(),
            model_version: "recognition-rules-1".into(),
            confidence: None,
            prediction_version: Some("1".into()),
            analysis_key: Some(key()),
            evidence: vec![SuggestionEvidence::new("tubular_sweep", "the rule measured a swept tube").unwrap()],
            suggested_at: Some("2026-11-10T00:00:00Z".into()),
        };
        assert!(recorded.validate("prediction").is_ok());
        assert!(ClassificationPrediction { prediction_version: Some(" ".into()), ..recorded.clone() }.validate("prediction").is_err());
        assert!(ClassificationPrediction { suggested_at: Some(" ".into()), ..recorded.clone() }.validate("prediction").is_err());
        let too_much_evidence =
            (0..=crate::recognition::MAX_SUGGESTION_EVIDENCE).map(|_| SuggestionEvidence::new("tubular_sweep", "detail").unwrap()).collect();
        assert!(ClassificationPrediction { evidence: too_much_evidence, ..recorded }.validate("prediction").is_err());
    }

    #[test]
    fn an_old_reviewed_part_without_a_catalog_record_stays_readable() {
        let old = serde_json::json!({
            "part_version_id": Uuid::from_u128(4),
            "classification": { "prediction": null, "confirmed": null },
            "volume_mm3": null,
            "volume_confirmed_by": null,
        });
        let part: ReviewedPart = serde_json::from_value(old).unwrap();
        assert_eq!(part.catalog, CatalogRecord::pending());
        assert!(part.validate().is_ok());

        let with_suggestion =
            ReviewedPart { catalog: CatalogRecord { suggestions: vec![], confirmed: None, project_override: None }, ..part.clone() };
        assert!(with_suggestion.validate().is_ok());
        let override_without_confirmed = ReviewedPart {
            catalog: CatalogRecord {
                project_override: Some(ProjectCatalogOverride { reason: "because".into(), ..override_value() }),
                ..CatalogRecord::pending()
            },
            ..part
        };
        assert!(override_without_confirmed.validate().is_err(), "an override needs a confirmed catalog reference");
    }

    fn override_value() -> ProjectCatalogOverride {
        ProjectCatalogOverride {
            specification: "project stock".into(),
            price: None,
            price_unit: Unit::CurrencyPerPiece,
            currency: "CNY".into(),
            reason: "project buys a different supplier".into(),
            recorded_by: "engineer".into(),
        }
    }

    #[test]
    fn a_catalog_confirmation_is_its_own_engineer_decision() {
        let catalog_only = PartConfirmation {
            part_version_id: Uuid::from_u128(4),
            classification: None,
            confirm_measurement: false,
            catalog: Some(CatalogConfirmation { component_version_id: Uuid::from_u128(201), project_override: None }),
        };
        assert!(catalog_only.validate("part_confirmation").is_ok(), "a catalog decision alone is a decision");

        let with_override = PartConfirmation {
            catalog: Some(CatalogConfirmation { component_version_id: Uuid::from_u128(201), project_override: Some(override_value()) }),
            ..catalog_only.clone()
        };
        assert!(with_override.validate("part_confirmation").is_ok());

        let override_without_reason = PartConfirmation {
            catalog: Some(CatalogConfirmation {
                component_version_id: Uuid::from_u128(201),
                project_override: Some(ProjectCatalogOverride { reason: " ".into(), ..override_value() }),
            }),
            ..catalog_only.clone()
        };
        assert!(override_without_reason.validate("part_confirmation").is_err(), "an override needs its reason");

        let nothing = PartConfirmation { catalog: None, ..catalog_only.clone() };
        assert!(nothing.validate("part_confirmation").is_err(), "a confirmation that confirms nothing is refused");

        let old_json = serde_json::json!({
            "part_version_id": Uuid::from_u128(4),
            "classification": null,
            "confirm_measurement": true,
        });
        let old: PartConfirmation = serde_json::from_value(old_json).unwrap();
        assert!(old.catalog.is_none(), "an old confirmation keeps working");
        assert!(old.validate("part_confirmation").is_ok());
    }

    #[test]
    fn the_suggestion_facts_name_what_changed_and_nothing_is_confirmed() {
        let recorded = EngineeringFact::SuggestionsRecorded { classification: 2, catalog: 1 };
        let value = serde_json::to_value(&recorded).unwrap();
        assert_eq!(value["kind"], serde_json::json!("suggestions_recorded"));
        assert_eq!(value["classification"], serde_json::json!(2));
        assert_eq!(value["catalog"], serde_json::json!(1));

        let confirmed = EngineeringFact::CatalogMatchConfirmed { part_version_id: Uuid::from_u128(4) };
        let value = serde_json::to_value(&confirmed).unwrap();
        assert_eq!(value["kind"], serde_json::json!("catalog_match_confirmed"));
        assert_eq!(value["part_version_id"], serde_json::json!(Uuid::from_u128(4)));
    }
}
