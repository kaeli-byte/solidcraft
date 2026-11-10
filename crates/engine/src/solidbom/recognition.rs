//! Analytic component recognition, conservative catalog matching and an independent-reference
//! evaluation harness.
//!
//! The rules here are small, deterministic and explainable; they claim no trained model and no
//! probability. A recognition result is a proposal for an engineer to review, never a confirmation:
//! the confirmed classification and the confirmed catalog reference stay the engineer's decisions
//! in the engineering-record contract, and nothing in this module can set them.
//!
//! # Recognition rule v1
//!
//! Rule version [`RECOGNITION_VERSION`]. Only the supported constant-diameter tube/protection
//! subset of package SB-12 is recognised. The exact geometry is probed through SB-12's extractor
//! ([`extraction::extract_definition`]); the `ExtractionCategory::Tube` argument only selects the
//! extraction view, because that extractor's measurement is category-independent, so recognition
//! never consumes a reviewed category and never infers a label from one.
//!
//! - Complete extraction of the supported subset proposes exactly the category `tube`, with
//!   evidence codes for the tubular sweep, the annular end faces, the constant diameter, the
//!   measured developed length and the measured bend count. One more evidence item,
//!   `category_shared_with_protection`, states that wear and thermal protection sleeves are the
//!   same tubular geometry: only the engineer's confirmation decides which label the part carries.
//!   The confidence is `None`; an analytic rule reports no probability, and a constant "1.0" would
//!   be an invented number.
//! - Anything else is [`RecognitionStatus::Unsupported`]: no category, a stable
//!   `recognition_unsupported` issue naming the extractor's own issue codes, one mapped
//!   `extraction_<code>` issue per extractor issue, and a `no_matching_analytic_rule` issue saying
//!   honestly that connectors, fittings and clips have no analytic rule here. Geometry is never
//!   guessed into one of those labels, and similar geometry never stands in for a different part.
//!
//! Recognition is a pure function of the exact source bytes and this rule version: the same
//! analysed geometry always gives the same category, evidence and issue order.
//!
//! # Catalog matching rule v1
//!
//! A catalog match is a text-attribute proposal, not an identity merge. It requires the
//! engineer-confirmed classification and a customer part number; a pending classification, a
//! missing part number, or mere geometric similarity never proposes a match. The reviewed
//! attributes are `category`, `customer_part_number` and, when present, `customer_revision`. Only
//! the newest published version of each logical catalog component is a candidate; a component that
//! declares no attributes is not a wildcard and never matches; a component matches only when every
//! attribute it declares has a reviewed counterpart whose value is equal after trimming and ASCII
//! case folding (an unknown declared key means no match). Exactly one candidate becomes one
//! suggestion with per-attribute evidence, none is a `no_catalog_match` issue and several is an
//! `ambiguous_catalog_match` issue with no suggestion at all - the matcher never picks one, never
//! confirms and never merges part identities.
//!
//! # Evaluation harness
//!
//! [`evaluate`] compares recognition (and, per case, catalog) outcomes with a reference dataset of
//! independent labels and reports counts and per-case outcomes. There is deliberately **no accuracy
//! ratio or percentage field**: a match count over an independent dataset is evidence once such a
//! dataset exists, not an accuracy percentage and not a deployment claim. The harness does not
//! verify that a dataset's labels are independent; the dataset's own provenance states where they
//! came from. Tests in this crate use a small synthetic dataset that is clearly marked
//! "synthetic contract fixture, not customer data".
//!
//! [`extraction::extract_definition`]: super::extraction::extract_definition

use std::collections::{BTreeMap, BTreeSet};

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use solidbom_contract::{
    AnalysisKey, AnalysisManifest, CatalogComponentVersion, CatalogMatchSuggestion, CategoryCode, ClassificationPrediction, ClassificationSuggestion,
    MAX_SUGGESTION_EVIDENCE, SuggestionEvidence,
};
use uuid::Uuid;

use super::extraction::{ExtractionCategory, ExtractionInput, ExtractionStatus, TubeExtraction, extract_definition};
use crate::{EngineError, Result, Session};

/// Version of the analytic recognition rule. Bump it whenever the same geometry could be labelled
/// differently, so a saved proposal is never silently attached to another rule.
pub const RECOGNITION_VERSION: &str = "1";
/// Version of the catalog text-attribute matching rule.
pub const CATALOG_MATCH_VERSION: &str = "1";
/// Schema version of a reference dataset the evaluation harness reads.
pub const REFERENCE_DATASET_SCHEMA_VERSION: u32 = 1;

/// The one category rule v1 can propose.
const TUBE_CATEGORY: &str = "tube";
/// How much of one evidence detail this module writes; the contract bounds it too.
const MAX_EVIDENCE_DETAIL_CHARS: usize = 240;

// ---------------------------------------------------------------------------------------------
// Recognition
// ---------------------------------------------------------------------------------------------

/// Whether recognition proposed a category or could not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecognitionStatus {
    /// The geometry is in the supported subset and a category is proposed for review.
    Suggested,
    /// The geometry is outside the supported subset; there is no category and issues say why.
    Unsupported,
}

/// One analytic recognition result for one exact analysis and definition.
///
/// This is a proposal. It never carries a confirmed label, and [`RecognitionResult::confidence`] is
/// always `None` because an analytic rule claims no probability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecognitionResult {
    /// The rule version that produced this result.
    pub recognition_version: String,
    /// The exact analysis the geometry came from.
    pub analysis_key: AnalysisKey,
    /// The definition this result describes.
    pub definition_id: Uuid,
    pub status: RecognitionStatus,
    /// The proposed category; `None` when unsupported.
    pub category: Option<CategoryCode>,
    /// Always `None`: analytic rules report no probability.
    pub confidence: Option<Decimal>,
    /// Why the category was proposed, most specific first; empty when unsupported.
    pub evidence: Vec<SuggestionEvidence>,
    /// Stable issue codes explaining an unsupported geometry; empty when a category is proposed.
    pub issues: Vec<SuggestionEvidence>,
}

impl RecognitionResult {
    /// The recorded-proposal form of this result for one shared part version.
    ///
    /// `None` when recognition proposed no category (unsupported geometry always is): an
    /// unsupported result is not written as a proposal. The suggestion stays a prediction bound to
    /// the exact analysis; nothing here confirms it.
    pub fn suggestion(&self, part_version_id: Uuid) -> Option<ClassificationSuggestion> {
        if self.status != RecognitionStatus::Suggested {
            return None;
        }
        let category = self.category.as_ref()?;
        Some(ClassificationSuggestion {
            part_version_id,
            prediction: ClassificationPrediction {
                name: category.as_str().to_string(),
                model_version: recognition_model_version(),
                confidence: None,
                prediction_version: Some(RECOGNITION_VERSION.to_string()),
                analysis_key: Some(self.analysis_key.clone()),
                evidence: self.evidence.clone(),
                suggested_at: None,
            },
        })
    }
}

/// Recognise the component category of one definition of a reviewed analysis.
///
/// The exact geometry is measured by package SB-12's extractor. `ExtractionCategory::Tube` only
/// selects the extraction view: that extractor's measurement does not depend on the category it is
/// asked for, and recognition never reads a reviewed category, so the same geometry is probed
/// whether the engineer has classified it or not.
///
/// The manifest is validated and must describe the session's reviewed source exactly as SB-12
/// requires, so a stale or foreign manifest is refused instead of describing another source. The
/// result is deterministic: the same analysed bytes and this rule version always give the same
/// status, category, evidence and issue order.
pub fn recognise_definition(session: &Session, manifest: &AnalysisManifest, definition_id: Uuid) -> Result<RecognitionResult> {
    manifest.validate().map_err(contract_error)?;
    // The extraction view is category-independent: recognition must not depend on a reviewed label.
    let input = ExtractionInput::new(manifest.key.clone(), definition_id, ExtractionCategory::Tube);
    let extraction = extract_definition(session, manifest, &input)?;
    // The extractor's own invariants: a complete result carries every measurement, an unsupported
    // one carries at least one issue. A result that does not validate is refused, never mapped.
    extraction.validate().map_err(contract_error)?;
    let (status, category, evidence, issues) = if extraction.status == ExtractionStatus::Complete {
        let category = CategoryCode::new(TUBE_CATEGORY).map_err(contract_error)?;
        (RecognitionStatus::Suggested, Some(category), tube_evidence(&extraction), Vec::new())
    } else {
        (RecognitionStatus::Unsupported, None, Vec::new(), unsupported_issues(&extraction))
    };
    Ok(RecognitionResult {
        recognition_version: RECOGNITION_VERSION.to_string(),
        analysis_key: extraction.analysis_key,
        definition_id: extraction.definition_id,
        status,
        category,
        // Analytic rules claim no probability, so this is never a number.
        confidence: None,
        evidence,
        issues,
    })
}

/// The evidence of a complete extraction of the supported subset, in a fixed order.
fn tube_evidence(extraction: &TubeExtraction) -> Vec<SuggestionEvidence> {
    let developed = measurement(extraction.developed_centerline_length_mm);
    let outer = measurement(extraction.outer_diameter_mm);
    let inner = measurement(extraction.inner_diameter_mm);
    let bends = extraction.bend_count.unwrap_or_default();
    [
        ("tubular_sweep", "the exact B-rep is one closed annular sweep with a connected outer and inner wall layer".to_string()),
        (
            "annular_end_faces",
            format!("both ends are planar annular end faces square to the centerline (outer diameter {outer} mm, inner diameter {inner} mm)"),
        ),
        ("constant_diameter", "the outer and inner wall radii are constant along the whole centerline".to_string()),
        ("developed_length_measured", format!("the developed centerline length is {developed} mm")),
        ("bend_count_measured", format!("the centerline has {bends} bend(s)")),
        (
            "category_shared_with_protection",
            "wear and thermal protection sleeves share this tubular geometry; only the engineer's confirmation decides \
             whether the part is a tube or a protection sleeve, and no label is confirmed here"
                .to_string(),
        ),
    ]
    .into_iter()
    .map(|(code, detail)| suggestion_evidence(code, detail))
    .collect()
}

/// The stable issues of an unsupported extraction: one summary naming SB-12's own codes, one mapped
/// issue per code, and the honest statement that no analytic rule covers the other categories.
fn unsupported_issues(extraction: &TubeExtraction) -> Vec<SuggestionEvidence> {
    let mut issues = Vec::with_capacity(extraction.issues.len() + 2);
    let codes: Vec<&str> = extraction.issues.iter().map(|issue| issue.code.as_str()).collect();
    let named = if codes.is_empty() { "no issue code was reported".to_string() } else { codes.join(", ") };
    issues.push(suggestion_evidence(
        "recognition_unsupported",
        format!("the exact-geometry extractor supports only the annular constant-diameter tube/protection subset; it reported: {named}"),
    ));
    // The mapped issues keep SB-12's stable codes readable and never invent a label from them.
    let mapped_cap = MAX_SUGGESTION_EVIDENCE.saturating_sub(3).max(1);
    for issue in extraction.issues.iter().take(mapped_cap) {
        issues.push(suggestion_evidence(&format!("extraction_{}", evidence_code(&issue.code)), issue.message.clone()));
    }
    if extraction.issues.len() > mapped_cap {
        let omitted = extraction.issues.len().saturating_sub(mapped_cap);
        issues.push(suggestion_evidence("extraction_issues_truncated", format!("{omitted} more extractor issue(s) are not repeated here")));
    }
    issues.push(suggestion_evidence(
        "no_matching_analytic_rule",
        "analytic rule v1 recognises only the supported annular constant-diameter tube/protection subset; connectors, fittings \
         and clips have no analytic rule here, so no category is proposed from this geometry"
            .to_string(),
    ));
    issues
}

/// A measurement as exact contract text, or an honest marker when the extractor reported none. A
/// complete extraction always carries every value; this never turns a missing value into zero.
fn measurement(value: Option<Decimal>) -> String {
    value.map(|value| value.to_string()).unwrap_or_else(|| "unavailable".to_string())
}

/// The rule name recorded as the prediction's model version.
fn recognition_model_version() -> String {
    format!("analytic-tube-rule-v{RECOGNITION_VERSION}")
}

// ---------------------------------------------------------------------------------------------
// Catalog matching
// ---------------------------------------------------------------------------------------------

/// One shared part version as the catalog matcher sees it.
///
/// `category` is the engineer's **confirmed** classification only: a pending proposal is passed as
/// `None`, so an unconfirmed label can never source a catalog match. The customer part number is
/// part-identity evidence; geometry alone never stands in for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogMatchPart {
    pub part_version_id: Uuid,
    /// The confirmed classification; `None` while nothing is confirmed.
    pub category: Option<CategoryCode>,
    pub customer_part_number: Option<String>,
    pub customer_revision: Option<String>,
}

/// The catalog matcher's proposal: at most one suggestion, plus the stable issues that explain why
/// other candidates were not proposed. It carries no confirmed reference and merges no identities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogMatchOutcome {
    pub suggestions: Vec<CatalogMatchSuggestion>,
    pub issues: Vec<SuggestionEvidence>,
}

/// Propose a catalog match for one shared part version against the published catalog versions.
///
/// The rule is deliberately conservative and text-only; it invents no numeric tolerances. See the
/// module documentation for the rule and its refusals. The outcome is deterministic: the same part
/// and catalog (in any order) give the same suggestions and issue order, and a tie between several
/// newest components is reported as ambiguous instead of resolved.
pub fn suggest_catalog_matches(part: &CatalogMatchPart, catalog: &[CatalogComponentVersion]) -> CatalogMatchOutcome {
    let mut issues = Vec::new();
    let Some(category) = part.category.as_ref() else {
        return CatalogMatchOutcome {
            suggestions: Vec::new(),
            issues: vec![suggestion_evidence(
                "classification_not_confirmed",
                "the part version has no engineer-confirmed classification; catalog matching uses the confirmed \
                 classification only and never a prediction"
                    .to_string(),
            )],
        };
    };
    let reviewed = reviewed_attributes(part, category);
    if !reviewed.contains_key("customer_part_number") {
        return CatalogMatchOutcome {
            suggestions: Vec::new(),
            issues: vec![suggestion_evidence(
                "missing_customer_part_number",
                "the part version has no customer part number; part identity is required evidence and similar geometry alone \
                 never proposes a catalog match"
                    .to_string(),
            )],
        };
    }
    let candidates = newest_versions(catalog);
    let mut matches: Vec<&CatalogComponentVersion> = Vec::new();
    // A catalog is administrator-maintained data, but it is still bounded input: never grow the
    // issue list past the contract's item count. Two slots are kept for the truncation note and
    // the terminal outcome issue.
    let component_issue_cap = MAX_SUGGESTION_EVIDENCE.saturating_sub(2).max(1);
    let mut omitted_components = 0usize;
    for candidate in &candidates {
        if candidate.attributes.is_empty() {
            // No wildcard: a component that declares no attributes cannot be matched at all.
            if issues.len() < component_issue_cap {
                issues.push(suggestion_evidence(
                    "catalog_component_has_no_attributes",
                    format!(
                        "catalog component {} version {} declares no matching attributes; it is not treated as a wildcard",
                        candidate.component_id, candidate.version
                    ),
                ));
            } else {
                omitted_components = omitted_components.saturating_add(1);
            }
            continue;
        }
        if declares_only_reviewed_attributes(&reviewed, candidate) {
            matches.push(candidate);
        }
    }
    if omitted_components > 0 {
        issues.push(suggestion_evidence(
            "catalog_component_issues_truncated",
            format!("{omitted_components} more newest catalog component(s) declare no attributes and are not repeated here"),
        ));
    }
    let suggestions = match matches.as_slice() {
        [] => {
            issues.push(suggestion_evidence(
                "no_catalog_match",
                format!("no newest published catalog component declares only the reviewed attributes ({} candidate(s) evaluated)", candidates.len()),
            ));
            Vec::new()
        }
        [single] => vec![catalog_suggestion(part, single)],
        several => {
            let versions: Vec<String> = several.iter().map(|candidate| candidate.component_version_id.to_string()).collect();
            issues.push(suggestion_evidence(
                "ambiguous_catalog_match",
                format!("{} newest published catalog components match equally well ({}); no suggestion is made", several.len(), versions.join(", ")),
            ));
            Vec::new()
        }
    };
    CatalogMatchOutcome { suggestions, issues }
}

/// The reviewed attributes a catalog component must declare a subset of: the confirmed category,
/// the customer part number and, only when present, the customer revision.
fn reviewed_attributes(part: &CatalogMatchPart, category: &CategoryCode) -> BTreeMap<String, String> {
    let mut reviewed = BTreeMap::new();
    reviewed.insert("category".to_string(), category.as_str().to_string());
    if let Some(number) = trimmed(part.customer_part_number.as_deref()) {
        reviewed.insert("customer_part_number".to_string(), number.to_string());
    }
    if let Some(revision) = trimmed(part.customer_revision.as_deref()) {
        reviewed.insert("customer_revision".to_string(), revision.to_string());
    }
    reviewed
}

/// The newest published version of every logical component, in deterministic
/// `component_version_id` order. A tie in `version` is resolved by the smaller version id, so the
/// candidate set never depends on the input order.
fn newest_versions(catalog: &[CatalogComponentVersion]) -> Vec<&CatalogComponentVersion> {
    let mut newest: BTreeMap<Uuid, &CatalogComponentVersion> = BTreeMap::new();
    for version in catalog {
        let replace = match newest.get(&version.component_id) {
            Some(current) => {
                version.version > current.version
                    || (version.version == current.version && version.component_version_id < current.component_version_id)
            }
            None => true,
        };
        if replace {
            newest.insert(version.component_id, version);
        }
    }
    let mut candidates: Vec<&CatalogComponentVersion> = newest.into_values().collect();
    candidates.sort_by_key(|candidate| candidate.component_version_id);
    candidates
}

/// Whether a candidate declares only reviewed attributes and every declared value equals the
/// reviewed value after trimming and ASCII case folding. An unknown key or a different value means
/// no match; there is no partial match and no tolerance.
fn declares_only_reviewed_attributes(reviewed: &BTreeMap<String, String>, candidate: &CatalogComponentVersion) -> bool {
    candidate
        .attributes
        .iter()
        .all(|attribute| reviewed.get(&attribute.key).is_some_and(|reviewed_value| folded(reviewed_value) == folded(&attribute.value)))
}

/// The one suggestion of a unique candidate, with one evidence item per matched attribute. The
/// evidence never exceeds the contract's item bound: the header plus at most
/// `MAX_SUGGESTION_EVIDENCE - 1` attribute items, in sorted key order, so the output is stable.
fn catalog_suggestion(part: &CatalogMatchPart, candidate: &CatalogComponentVersion) -> CatalogMatchSuggestion {
    let mut evidence = Vec::with_capacity(candidate.attributes.len().min(MAX_SUGGESTION_EVIDENCE));
    evidence.push(suggestion_evidence(
        "catalog_attributes_match",
        format!(
            "the newest published version ({}) of catalog component {} declares only the reviewed attributes",
            candidate.version, candidate.component_id
        ),
    ));
    let mut attributes: Vec<&solidbom_contract::CatalogAttribute> = candidate.attributes.iter().collect();
    attributes.sort_by(|left, right| left.key.cmp(&right.key));
    for attribute in attributes.iter().take(MAX_SUGGESTION_EVIDENCE.saturating_sub(1)) {
        evidence.push(suggestion_evidence("catalog_attribute_match", format!("{}={}", attribute.key, attribute.value)));
    }
    CatalogMatchSuggestion {
        part_version_id: part.part_version_id,
        component_id: candidate.component_id,
        component_version_id: candidate.component_version_id,
        model_version: catalog_model_version(),
        prediction_version: CATALOG_MATCH_VERSION.to_string(),
        // Text matching reports no probability; confidence is evidence only, and here it is None.
        confidence: None,
        evidence,
    }
}

/// The rule name recorded as the catalog suggestion's model version.
fn catalog_model_version() -> String {
    format!("text-attribute-rule-v{CATALOG_MATCH_VERSION}")
}

/// Trimmed text, or `None` for missing/blank text (a blank value is missing, never a value).
fn trimmed(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// The comparison form of an attribute value: trimmed and ASCII case folded. Unicode case folding
/// is deliberately not used, so the comparison is exact and locale-free.
fn folded(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

// ---------------------------------------------------------------------------------------------
// Evaluation against independent reference labels
// ---------------------------------------------------------------------------------------------

/// One reference case: the label an independent reviewer recorded for one definition, and the
/// catalog component they expect, when the catalog was evaluated for that case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceCase {
    pub case_id: String,
    pub definition_id: Uuid,
    /// `None` means the reference says the definition is unclassified.
    pub expected_category: Option<CategoryCode>,
    /// `None` means the reference does not evaluate the catalog for this case.
    pub expected_catalog_component_version_id: Option<Uuid>,
}

/// A reference dataset of independent labels.
///
/// `provenance` is required: it says where the labels came from. The harness does not verify
/// independence, and a count over the dataset is evidence only once the labels really are
/// independent of the rules under test.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceDataset {
    /// The dataset schema this build reads: [`REFERENCE_DATASET_SCHEMA_VERSION`].
    pub schema_version: u32,
    pub dataset_id: String,
    /// Where the independent labels came from; required and never inferred.
    pub provenance: String,
    pub cases: Vec<ReferenceCase>,
}

/// One evaluation input: the outcomes produced for one definition, matched to the dataset by
/// definition identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationInput {
    pub definition_id: Uuid,
    pub recognition: RecognitionResult,
    /// The catalog outcome, when the catalog matcher was asked for this definition.
    pub catalog: Option<CatalogMatchOutcome>,
}

/// How the proposed classification compared with the reference label of one case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseClassification {
    /// The proposal equals the reference label.
    Matched,
    /// The proposal differs from the reference label; either side may be unclassified.
    Mismatched { expected: Option<CategoryCode>, proposed: Option<CategoryCode> },
    /// No category could be proposed; `codes` carries the stable recognition issue codes.
    Unsupported { codes: Vec<String> },
}

/// How the catalog proposal compared with the reference catalog of one case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseCatalog {
    /// The reference does not evaluate the catalog for this case.
    NotEvaluated,
    /// The single proposal equals the reference catalog component version.
    Matched,
    /// The proposal differs from the reference; `proposed` lists every proposed version.
    Mismatched { expected: Option<Uuid>, proposed: Vec<Uuid> },
    /// No catalog proposal exists; `codes` carries the stable matcher issue codes.
    Unsupported { codes: Vec<String> },
}

/// The per-case outcome of one reference case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaseOutcome {
    pub case_id: String,
    pub definition_id: Uuid,
    pub classification: CaseClassification,
    pub catalog: CaseCatalog,
}

/// The evaluation report: counts and per-case outcomes only.
///
/// There is deliberately no accuracy ratio or percentage field. A count of matches over an
/// independent dataset is evidence once such a dataset exists; it is not an accuracy percentage and
/// no deployment claim follows from it. The `matched`, `mismatched` and `unsupported` counts are
/// classification outcomes; catalog outcomes are reported per case in [`CaseOutcome::catalog`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationReport {
    pub dataset_id: String,
    pub dataset_provenance: String,
    /// The recognition rule version (or versions) the evaluated proposals came from.
    pub recognition_version: String,
    pub matched: u32,
    pub mismatched: u32,
    pub unsupported: u32,
    /// Reference cases with no evaluation input for their definition.
    pub missing_results: u32,
    /// One outcome per reference case, in dataset order.
    pub outcomes: Vec<CaseOutcome>,
    /// The harness's own limits, stated so a reader cannot mistake the report for more than it is.
    pub limitations: Vec<String>,
}

/// Compare recognition and catalog outcomes with an independent reference dataset.
///
/// The dataset must be a current schema ([`REFERENCE_DATASET_SCHEMA_VERSION`]) with a non-empty id,
/// provenance and unique case ids; anything else is refused rather than evaluated as if it made
/// sense. Each case without an input is counted in `missing_results` and reported as
/// `Unsupported { codes: ["missing_result"] }`, never inferred. Cases are reported in dataset order,
/// and the same dataset and inputs always give the same report.
pub fn evaluate(dataset: &ReferenceDataset, inputs: &[EvaluationInput]) -> Result<EvaluationReport> {
    validate_dataset(dataset)?;
    let mut by_definition: BTreeMap<Uuid, &EvaluationInput> = BTreeMap::new();
    for input in inputs {
        if input.recognition.definition_id != input.definition_id {
            return Err(EngineError::Other("an evaluation input's recognition result describes another definition".into()));
        }
        if by_definition.insert(input.definition_id, input).is_some() {
            return Err(EngineError::Other(format!("the definition {} has more than one evaluation input", input.definition_id)));
        }
    }

    let mut matched = 0u32;
    let mut mismatched = 0u32;
    let mut unsupported = 0u32;
    let mut missing_results = 0u32;
    let mut versions = BTreeSet::new();
    let mut outcomes = Vec::with_capacity(dataset.cases.len());
    for case in &dataset.cases {
        let Some(input) = by_definition.get(&case.definition_id).copied() else {
            missing_results = missing_results.saturating_add(1);
            outcomes.push(CaseOutcome {
                case_id: case.case_id.clone(),
                definition_id: case.definition_id,
                classification: CaseClassification::Unsupported { codes: vec!["missing_result".to_string()] },
                catalog: CaseCatalog::NotEvaluated,
            });
            continue;
        };
        versions.insert(input.recognition.recognition_version.clone());
        let classification = classify(&input.recognition, case.expected_category.as_ref());
        match &classification {
            CaseClassification::Matched => matched = matched.saturating_add(1),
            CaseClassification::Mismatched { .. } => mismatched = mismatched.saturating_add(1),
            CaseClassification::Unsupported { .. } => unsupported = unsupported.saturating_add(1),
        }
        let catalog = evaluate_catalog(case, input.catalog.as_ref());
        outcomes.push(CaseOutcome { case_id: case.case_id.clone(), definition_id: case.definition_id, classification, catalog });
    }

    Ok(EvaluationReport {
        dataset_id: dataset.dataset_id.clone(),
        dataset_provenance: dataset.provenance.clone(),
        recognition_version: join_versions(&versions),
        matched,
        mismatched,
        unsupported,
        missing_results,
        outcomes,
        limitations: vec![
            "this report counts outcomes over the given reference dataset; a match count is evidence once the dataset is \
             independent of the rules under test, and it is not an accuracy percentage and not a deployment claim"
                .to_string(),
            "the harness does not verify that the reference labels are independent; the dataset provenance states where they \
             came from"
                .to_string(),
            "catalog outcomes are reported per case and are only compared where the reference names an expected catalog \
             component version"
                .to_string(),
            "a reference case without an evaluation input is counted in missing_results and never inferred".to_string(),
        ],
    })
}

/// One case's classification outcome. A recognition result that says "suggested" without a category
/// cannot be compared and is reported as unsupported with a code, never silently skipped.
fn classify(recognition: &RecognitionResult, expected: Option<&CategoryCode>) -> CaseClassification {
    if recognition.status == RecognitionStatus::Unsupported {
        return CaseClassification::Unsupported { codes: recognition.issues.iter().map(|issue| issue.code.clone()).collect() };
    }
    let Some(proposed) = recognition.category.as_ref() else {
        return CaseClassification::Unsupported { codes: vec!["suggested_without_category".to_string()] };
    };
    if expected == Some(proposed) {
        CaseClassification::Matched
    } else {
        CaseClassification::Mismatched { expected: expected.cloned(), proposed: Some(proposed.clone()) }
    }
}

/// One case's catalog outcome, compared only where the reference expects a catalog version.
fn evaluate_catalog(case: &ReferenceCase, catalog: Option<&CatalogMatchOutcome>) -> CaseCatalog {
    let Some(expected) = case.expected_catalog_component_version_id else {
        return CaseCatalog::NotEvaluated;
    };
    let Some(outcome) = catalog else {
        return CaseCatalog::Unsupported { codes: vec!["missing_catalog_result".to_string()] };
    };
    match outcome.suggestions.as_slice() {
        [] => CaseCatalog::Unsupported { codes: outcome.issues.iter().map(|issue| issue.code.clone()).collect() },
        [single] if single.component_version_id == expected => CaseCatalog::Matched,
        [single] => CaseCatalog::Mismatched { expected: Some(expected), proposed: vec![single.component_version_id] },
        several => {
            CaseCatalog::Mismatched { expected: Some(expected), proposed: several.iter().map(|suggestion| suggestion.component_version_id).collect() }
        }
    }
}

/// The dataset must be something the harness can evaluate honestly: a current schema, an identity,
/// a stated provenance and unique case ids.
fn validate_dataset(dataset: &ReferenceDataset) -> Result<()> {
    if dataset.schema_version != REFERENCE_DATASET_SCHEMA_VERSION {
        return Err(EngineError::Other(format!(
            "the reference dataset schema version is {}, this build reads {REFERENCE_DATASET_SCHEMA_VERSION}",
            dataset.schema_version
        )));
    }
    if dataset.dataset_id.trim().is_empty() {
        return Err(EngineError::Other("a reference dataset needs an id".into()));
    }
    if dataset.provenance.trim().is_empty() {
        return Err(EngineError::Other("a reference dataset needs its provenance: where the independent labels came from".into()));
    }
    let mut case_ids = BTreeSet::new();
    for case in &dataset.cases {
        if case.case_id.trim().is_empty() {
            return Err(EngineError::Other("a reference case needs an id".into()));
        }
        if !case_ids.insert(case.case_id.as_str()) {
            return Err(EngineError::Other(format!("the reference case id `{}` is used more than once", case.case_id)));
        }
    }
    Ok(())
}

/// The rule version(s) the report covers: one version, `"none"` when nothing was evaluated, or the
/// distinct versions in sorted order.
fn join_versions(versions: &BTreeSet<String>) -> String {
    if versions.is_empty() {
        return "none".to_string();
    }
    versions.iter().cloned().collect::<Vec<_>>().join(", ")
}

// ---------------------------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------------------------

/// Build one evidence item from this module's own stable code and engine-authored text.
///
/// Both are bounded and sanitised here, so the contract's validation cannot fail for a value this
/// module builds; the fallback keeps a structurally valid item instead of panicking if it ever
/// does (for example a future contract that bounds detail text even further).
fn suggestion_evidence(code: &str, detail: impl Into<String>) -> SuggestionEvidence {
    let detail = bounded_detail(detail.into());
    match SuggestionEvidence::new(code, detail.clone()) {
        Ok(evidence) => evidence,
        Err(_) => SuggestionEvidence { code: code.to_string(), detail },
    }
}

/// Bound one evidence detail: control characters become spaces and the text is cut to a fixed
/// length, so detail text from geometry or catalog data can never break the item.
fn bounded_detail(detail: String) -> String {
    let cleaned: String =
        detail.chars().map(|character| if character.is_control() { ' ' } else { character }).take(MAX_EVIDENCE_DETAIL_CHARS).collect();
    if cleaned.trim().is_empty() { "no further detail".to_string() } else { cleaned }
}

/// A stable evidence code fragment from a stable engine issue code: lower-case ASCII letters,
/// digits and underscores. This keeps mapped codes inside the contract's code shape without
/// changing SB-12's own meaning.
fn evidence_code(code: &str) -> String {
    let mut fragment = String::with_capacity(code.len());
    for character in code.chars() {
        if character.is_ascii_alphanumeric() || character == '_' {
            fragment.push(character.to_ascii_lowercase());
        } else {
            fragment.push('_');
        }
    }
    if fragment.is_empty() { "unknown".to_string() } else { fragment }
}

/// Map a contract validation error into the engine's error type.
fn contract_error(error: impl std::fmt::Display) -> EngineError {
    EngineError::Other(error.to_string())
}
