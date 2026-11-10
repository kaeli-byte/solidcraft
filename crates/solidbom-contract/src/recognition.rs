//! Recognition and catalog suggestions: the portable suggestion contract of package SB-13.
//!
//! The package proposes component classifications and catalog matches while engineering
//! confirmation stays authoritative. These types are the wire and storage shape of a proposal,
//! never of a decision:
//!
//! - A suggestion is evidence only. A [`ClassificationSuggestion`] records what recognition
//!   proposed for one part version; it never sets the confirmed label, whatever its confidence.
//!   A [`CatalogMatchSuggestion`] is one matcher proposal; [`CatalogRecord::effective_inputs`]
//!   deliberately returns `None` until an engineer-confirmed [`ConfirmedCatalogReference`]
//!   exists, so an unconfirmed proposal can never supply a price or a specification.
//! - Confirmed catalog values and the engineer's project override stay separate. A
//!   [`ConfirmedCatalogReference`] keeps the catalog version, name, specification, price, unit
//!   and currency frozen at confirmation time; a [`ProjectCatalogOverride`] is recorded beside
//!   it, so the original catalog value is never lost when a project replaces it.
//! - Categories are open stable codes. [`INITIAL_CATEGORIES`] lists the six categories confirmed
//!   for the prototype; any other well-formed code is a valid extension, so adding a category
//!   needs no schema, identity or wire change.
//!
//! Rules mirrored from the published version 1 and version 2 contracts: decimals serialize as
//! strings, a missing value is `None` and never an explicit zero, unknown JSON fields are
//! tolerated, and hostile input is bounded rather than trusted. A recorded suggestion is bound
//! to the exact analysis that produced it and shows at least one evidence item; a matcher never
//! confirms or merges a part identity, and this contract claims no model accuracy and no
//! deployment result.

use std::collections::{BTreeMap, BTreeSet};

use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

use crate::engineering::{ClassificationPrediction, MAX_MODEL_VERSION_CHARS, MAX_REQUEST_ID_CHARS, MAX_REVIEWER_CHARS};
use crate::error::{ContractError, Result};
use crate::shared_data::{MAX_SHARED_NAME_CHARS, MAX_SOURCE_REFERENCE_CHARS};
use crate::types::{Unit, check_text, is_stable_code};

/// The recognition contract version this build reads and writes. It extends the published
/// version 2 API additively and has its own version so the suggestion shape can grow.
pub const RECOGNITION_CONTRACT_VERSION: u32 = 1;

/// The most suggestions (classification plus catalog) one recorded write may carry.
pub const MAX_SUGGESTIONS_PER_WRITE: usize = 10_000;
/// The most catalog candidates one part version may hold after one matching run.
pub const MAX_CATALOG_SUGGESTIONS_PER_PART: usize = 16;
/// The most evidence items one suggestion may carry.
pub const MAX_SUGGESTION_EVIDENCE: usize = 32;
/// The longest category code, matching `^[a-z][a-z0-9_]{0,63}$`.
pub const MAX_CATEGORY_CHARS: usize = 64;
/// The longest stable evidence code.
pub const MAX_EVIDENCE_CODE_CHARS: usize = 64;
/// The longest human explanation one evidence item may carry.
pub const MAX_EVIDENCE_DETAIL_CHARS: usize = 512;
/// The longest recorded timestamp. The service writes RFC 3339 UTC and never trusts a client
/// timestamp as authoritative.
pub const MAX_TIMESTAMP_CHARS: usize = 64;

/// The six categories confirmed for the prototype. A category is an open stable code: an added
/// category like `bracket` or `custom_bracket` is a valid suggestion, and this list never limits
/// what a project may confirm.
pub const INITIAL_CATEGORIES: [&str; 6] = ["tube", "connector", "fitting", "clip", "wear_protection", "thermal_protection"];

/// One open, stable category code such as `tube` or a later `custom_bracket`.
///
/// The code follows `^[a-z][a-z0-9_]{0,63}$`. Because the set is open, an added category needs no
/// schema or identity change. Deserializing a code validates it, so an invalid code cannot enter
/// the wire contract; the JSON form is the plain code string.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CategoryCode(String);

impl CategoryCode {
    /// A category code from its stable wire text.
    pub fn new(code: impl Into<String>) -> Result<Self> {
        let code = code.into();
        if is_stable_code(&code, MAX_CATEGORY_CHARS) {
            Ok(Self(code))
        } else {
            Err(ContractError::invalid(
                "category",
                "expected a stable category code: a lower-case letter followed by up to 63 lower-case letters, digits or underscores",
            ))
        }
    }

    /// The stable wire text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this is one of the six categories the prototype confirmed.
    pub fn is_initial(&self) -> bool {
        INITIAL_CATEGORIES.iter().any(|code| *code == self.0)
    }
}

impl Serialize for CategoryCode {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for CategoryCode {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let code = String::deserialize(deserializer)?;
        Self::new(code).map_err(serde::de::Error::custom)
    }
}

/// One stable evidence code with its human explanation. Evidence explains a proposal; it never
/// approves one.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SuggestionEvidence {
    /// A stable contract code such as `tubular_sweep`, not display text.
    pub code: String,
    /// A human-readable explanation of the evidence, in English.
    pub detail: String,
}

impl SuggestionEvidence {
    /// A bounded evidence item with a stable code and its explanation.
    pub fn new(code: impl Into<String>, detail: impl Into<String>) -> Result<Self> {
        let evidence = Self { code: code.into(), detail: detail.into() };
        evidence.validate("suggestion_evidence")?;
        Ok(evidence)
    }

    /// Validate the stable code and the bounded explanation.
    pub fn validate(&self, field: &str) -> Result<()> {
        let code_field = format!("{field}.code");
        check_bounded_text(&self.code, &code_field, MAX_EVIDENCE_CODE_CHARS)?;
        if !is_stable_code(&self.code, MAX_EVIDENCE_CODE_CHARS) {
            return Err(ContractError::invalid(
                code_field.as_str(),
                "expected a stable evidence code: a lower-case letter followed by lower-case letters, digits or underscores",
            ));
        }
        check_bounded_text(&self.detail, &format!("{field}.detail"), MAX_EVIDENCE_DETAIL_CHARS)
    }
}

/// One recorded classification suggestion for one shared reviewed part version.
///
/// The prediction is stored in `ReviewedPart.classification.prediction` and never confirms
/// anything: only the separate engineer confirmation sets a confirmed label.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClassificationSuggestion {
    /// The reviewed part version the proposal is for.
    pub part_version_id: Uuid,
    /// The proposed label with its provenance. Recording requires a prediction version, the exact
    /// analysis key and at least one evidence item.
    pub prediction: ClassificationPrediction,
}

/// One catalog match proposal produced by a matcher.
///
/// The service resolves and stores the catalog values at record time, so the persisted candidate
/// carries authoritative catalog reference values; the proposal itself is still only evidence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CatalogMatchSuggestion {
    /// The reviewed part version this proposal is for.
    pub part_version_id: Uuid,
    /// The logical catalog component the proposal names.
    pub component_id: Uuid,
    /// The exact catalog component version the proposal names.
    pub component_version_id: Uuid,
    /// The matcher/model that produced the proposal.
    pub model_version: String,
    /// The version of the matching algorithm or schema generation.
    pub prediction_version: String,
    /// The matcher's confidence in `0..=1`, when it reports one. Evidence only; it never confirms.
    pub confidence: Option<Decimal>,
    /// At least one stable code with its explanation.
    pub evidence: Vec<SuggestionEvidence>,
}

impl CatalogMatchSuggestion {
    /// Validate the proposal identity, provenance, confidence and evidence.
    pub fn validate(&self, field: &str) -> Result<()> {
        check_bounded_text(&self.model_version, &format!("{field}.model_version"), MAX_MODEL_VERSION_CHARS)?;
        check_bounded_text(&self.prediction_version, &format!("{field}.prediction_version"), MAX_MODEL_VERSION_CHARS)?;
        check_confidence(self.confidence, &format!("{field}.confidence"))?;
        if self.evidence.is_empty() {
            return Err(ContractError::invalid(format!("{field}.evidence"), "a recorded suggestion shows at least one evidence item"));
        }
        validate_evidence(&self.evidence, &format!("{field}.evidence"))
    }
}

/// One persisted catalog match proposal, with the catalog version values frozen at record time.
///
/// A candidate is still a proposal: it never supplies [`CatalogRecord::effective_inputs`] until an
/// engineer confirms the catalog version.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CatalogMatchCandidate {
    /// The deterministic UUIDv5 identity the service assigns over the suggestion identity.
    pub suggestion_id: Uuid,
    /// The reviewed part version this proposal is for.
    pub part_version_id: Uuid,
    /// The logical catalog component the proposal names.
    pub component_id: Uuid,
    /// The exact catalog component version the proposal names.
    pub component_version_id: Uuid,
    /// The published catalog version the values were resolved from.
    pub catalog_version: u64,
    /// The catalog component name, as published at record time.
    pub catalog_name: String,
    /// The catalog specification, as published at record time.
    pub specification: String,
    /// The catalog price, as published at record time. `None` is missing, never zero.
    pub price: Option<Decimal>,
    pub price_unit: Unit,
    pub currency: String,
    /// The matcher/model that produced the proposal.
    pub model_version: String,
    /// The version of the matching algorithm or schema generation.
    pub prediction_version: String,
    /// The matcher's confidence in `0..=1`, when it reports one. Evidence only.
    pub confidence: Option<Decimal>,
    /// At least one stable code with its explanation.
    pub evidence: Vec<SuggestionEvidence>,
    /// The RFC 3339 timestamp the service recorded, when it recorded one.
    pub suggested_at: Option<String>,
}

impl CatalogMatchCandidate {
    /// Validate the frozen catalog reference values and the proposal evidence.
    pub fn validate(&self, field: &str) -> Result<()> {
        check_catalog_version(self.catalog_version, &format!("{field}.catalog_version"))?;
        check_catalog_name(&self.catalog_name, &format!("{field}.catalog_name"))?;
        check_catalog_specification(&self.specification, &format!("{field}.specification"))?;
        check_catalog_price(self.price, &format!("{field}.price"))?;
        check_catalog_price_unit(self.price_unit, &format!("{field}.price_unit"))?;
        check_currency(&self.currency, &format!("{field}.currency"))?;
        check_bounded_text(&self.model_version, &format!("{field}.model_version"), MAX_MODEL_VERSION_CHARS)?;
        check_bounded_text(&self.prediction_version, &format!("{field}.prediction_version"), MAX_MODEL_VERSION_CHARS)?;
        check_confidence(self.confidence, &format!("{field}.confidence"))?;
        if self.evidence.is_empty() {
            return Err(ContractError::invalid(format!("{field}.evidence"), "a recorded suggestion shows at least one evidence item"));
        }
        validate_evidence(&self.evidence, &format!("{field}.evidence"))?;
        check_optional_timestamp(&self.suggested_at, &format!("{field}.suggested_at"))
    }
}

/// The engineer-confirmed catalog version of one part version.
///
/// The catalog values are frozen at confirmation time and retained even when a project override
/// replaces them for this project; the two are always stored separately.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfirmedCatalogReference {
    /// The logical catalog component the engineer confirmed.
    pub component_id: Uuid,
    /// The exact catalog component version the engineer confirmed.
    pub component_version_id: Uuid,
    /// The published catalog version the values were resolved from.
    pub version: u64,
    /// The confirmed catalog component name.
    pub name: String,
    /// The confirmed catalog specification.
    pub specification: String,
    /// The confirmed catalog price. `None` is missing, never zero.
    pub price: Option<Decimal>,
    pub price_unit: Unit,
    pub currency: String,
    /// The authenticated reviewer. The service fills it from the caller, never from the payload.
    pub confirmed_by: String,
}

impl ConfirmedCatalogReference {
    /// Validate the confirmed catalog values and the authenticated reviewer.
    pub fn validate(&self, field: &str) -> Result<()> {
        check_catalog_version(self.version, &format!("{field}.version"))?;
        check_catalog_name(&self.name, &format!("{field}.name"))?;
        check_catalog_specification(&self.specification, &format!("{field}.specification"))?;
        check_catalog_price(self.price, &format!("{field}.price"))?;
        check_catalog_price_unit(self.price_unit, &format!("{field}.price_unit"))?;
        check_currency(&self.currency, &format!("{field}.currency"))?;
        check_reviewer(&self.confirmed_by, &format!("{field}.confirmed_by"))
    }
}

/// The engineer's project override of the confirmed catalog values, recorded separately from the
/// confirmed catalog reference so the original catalog version and value stay readable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectCatalogOverride {
    /// The specification this project uses instead of the catalog one.
    pub specification: String,
    /// The price this project uses. `None` is missing, never zero.
    pub price: Option<Decimal>,
    pub price_unit: Unit,
    pub currency: String,
    /// Why the project overrides the catalog value; required.
    pub reason: String,
    /// The authenticated reviewer. The service fills it from the caller.
    pub recorded_by: String,
}

impl ProjectCatalogOverride {
    /// Validate the override values, the required reason and the authenticated reviewer.
    pub fn validate(&self, field: &str) -> Result<()> {
        check_catalog_specification(&self.specification, &format!("{field}.specification"))?;
        check_catalog_price(self.price, &format!("{field}.price"))?;
        check_catalog_price_unit(self.price_unit, &format!("{field}.price_unit"))?;
        check_currency(&self.currency, &format!("{field}.currency"))?;
        check_bounded_text(&self.reason, &format!("{field}.reason"), MAX_SOURCE_REFERENCE_CHARS)?;
        check_reviewer(&self.recorded_by, &format!("{field}.recorded_by"))
    }
}

/// The catalog proposal and decision state of one reviewed part version.
///
/// `suggestions` are proposals only. `confirmed` is the authoritative engineer-confirmed catalog
/// version and `project_override` the separately recorded project replacement. A record with only
/// suggestions has no authoritative catalog inputs.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CatalogRecord {
    /// Persisted matcher proposals. They never supply price or specification.
    #[serde(default)]
    pub suggestions: Vec<CatalogMatchCandidate>,
    /// The engineer-confirmed catalog version, when one is confirmed.
    #[serde(default)]
    pub confirmed: Option<ConfirmedCatalogReference>,
    /// The engineer's project override of the confirmed values, when one is recorded.
    #[serde(default)]
    pub project_override: Option<ProjectCatalogOverride>,
}

impl CatalogRecord {
    /// No suggestion and no confirmed catalog reference yet.
    pub fn pending() -> Self {
        Self::default()
    }

    /// Validate the stored proposals and the confirmed/override pair. An override is only valid
    /// together with a confirmed catalog version.
    pub fn validate(&self, field: &str) -> Result<()> {
        if self.suggestions.len() > MAX_CATALOG_SUGGESTIONS_PER_PART {
            return Err(ContractError::invalid(
                format!("{field}.suggestions"),
                format!("a part version holds at most {MAX_CATALOG_SUGGESTIONS_PER_PART} catalog suggestions"),
            ));
        }
        let mut suggestion_ids = BTreeSet::new();
        let mut component_versions = BTreeSet::new();
        let mut part_versions = BTreeSet::new();
        for (index, suggestion) in self.suggestions.iter().enumerate() {
            suggestion.validate(&format!("{field}.suggestions.{index}"))?;
            if !suggestion_ids.insert(suggestion.suggestion_id) {
                return Err(ContractError::invalid(format!("{field}.suggestions"), "two catalog suggestions share one suggestion id"));
            }
            if !component_versions.insert(suggestion.component_version_id) {
                return Err(ContractError::invalid(
                    format!("{field}.suggestions"),
                    "one part version lists the same catalog component version twice",
                ));
            }
            part_versions.insert(suggestion.part_version_id);
        }
        if part_versions.len() > 1 {
            return Err(ContractError::invalid(format!("{field}.suggestions"), "one catalog record holds the suggestions of one part version only"));
        }
        if let Some(confirmed) = &self.confirmed {
            confirmed.validate(&format!("{field}.confirmed"))?;
        }
        if let Some(project_override) = &self.project_override {
            project_override.validate(&format!("{field}.project_override"))?;
            if self.confirmed.is_none() {
                return Err(ContractError::invalid(
                    format!("{field}.project_override"),
                    "a project override needs the engineer-confirmed catalog version it replaces",
                ));
            }
        }
        Ok(())
    }

    /// The authoritative catalog inputs of this part version.
    ///
    /// Returns `None` while no engineer-confirmed catalog reference exists: an unconfirmed
    /// suggestion never supplies a price or a specification, whatever its confidence. When a
    /// project override is recorded, the override values are returned with `override_applied:
    /// true`; the confirmed catalog values stay retained in this record unchanged.
    pub fn effective_inputs(&self) -> Option<EffectiveCatalogInputs> {
        let confirmed = self.confirmed.as_ref()?;
        match &self.project_override {
            Some(project_override) => Some(EffectiveCatalogInputs {
                component_version_id: confirmed.component_version_id,
                specification: project_override.specification.clone(),
                price: project_override.price,
                price_unit: project_override.price_unit,
                currency: project_override.currency.clone(),
                override_applied: true,
            }),
            None => Some(EffectiveCatalogInputs {
                component_version_id: confirmed.component_version_id,
                specification: confirmed.specification.clone(),
                price: confirmed.price,
                price_unit: confirmed.price_unit,
                currency: confirmed.currency.clone(),
                override_applied: false,
            }),
        }
    }
}

/// The authoritative catalog inputs of one part version: the confirmed catalog values, or the
/// project override values when one is recorded.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EffectiveCatalogInputs {
    /// The engineer-confirmed catalog component version.
    pub component_version_id: Uuid,
    /// The specification in effect for this project.
    pub specification: String,
    /// The price in effect. `None` is missing, never zero.
    pub price: Option<Decimal>,
    pub price_unit: Unit,
    pub currency: String,
    /// True when the values come from the project override rather than the catalog reference.
    pub override_applied: bool,
}

/// One recorded suggestion batch for a project: classification proposals, catalog match
/// proposals, and/or the catalog evaluated-part set, appended as one engineering review write.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecordSuggestions {
    /// The idempotency key of the write.
    pub request_id: String,
    /// The engineering record version the write expects.
    pub expected_version: u64,
    /// Classification proposals for one or more reviewed part versions.
    #[serde(default)]
    pub classification: Vec<ClassificationSuggestion>,
    /// Catalog match proposals for one or more reviewed part versions.
    #[serde(default)]
    pub catalog: Vec<CatalogMatchSuggestion>,
    /// Every part version this catalog matching run evaluated. Each named part version gets its
    /// stored candidate list replaced by the candidates this write carries for it, so a no-match or
    /// ambiguous run clears the previous proposals by naming the part with no candidate. A part
    /// version named only by a candidate is evaluated implicitly, which keeps a payload that predates
    /// this field working. The engineer-confirmed reference and the project override are never
    /// changed by a matching run.
    #[serde(default)]
    pub catalog_evaluated_part_versions: Vec<Uuid>,
}

impl RecordSuggestions {
    /// Validate a suggestion write before the service appends it.
    ///
    /// A write names at least one classification suggestion, catalog suggestion or evaluated part
    /// version and stays within the per-write bounds. A recorded classification proposal carries the
    /// exact analysis, the prediction version and at least one evidence item: a proposal not bound to
    /// a source is not a reviewable suggestion. A catalog proposal additionally names one exact
    /// catalog component version once per part version. An evaluated part version is named once.
    pub fn validate(&self) -> Result<()> {
        check_request_id(&self.request_id, "record_suggestions.request_id")?;
        if self.classification.is_empty() && self.catalog.is_empty() && self.catalog_evaluated_part_versions.is_empty() {
            return Err(ContractError::invalid(
                "record_suggestions",
                "a suggestion write names at least one classification suggestion, catalog suggestion or evaluated part version",
            ));
        }
        if self.classification.len().saturating_add(self.catalog.len()) > MAX_SUGGESTIONS_PER_WRITE {
            return Err(ContractError::invalid(
                "record_suggestions",
                format!("a suggestion write carries at most {MAX_SUGGESTIONS_PER_WRITE} suggestions"),
            ));
        }
        if self.catalog_evaluated_part_versions.len() > MAX_SUGGESTIONS_PER_WRITE {
            return Err(ContractError::invalid(
                "record_suggestions.catalog_evaluated_part_versions",
                format!("a suggestion write names at most {MAX_SUGGESTIONS_PER_WRITE} evaluated part versions"),
            ));
        }
        let mut evaluated_parts = BTreeSet::new();
        for (index, part_version_id) in self.catalog_evaluated_part_versions.iter().enumerate() {
            let field = format!("record_suggestions.catalog_evaluated_part_versions.{index}");
            if part_version_id.is_nil() {
                return Err(ContractError::invalid(field, "an evaluated part version needs a business identity"));
            }
            if !evaluated_parts.insert(*part_version_id) {
                return Err(ContractError::invalid(field, "an evaluated part version appears twice in one suggestion write"));
            }
        }

        let mut classified_parts = BTreeSet::new();
        for (index, suggestion) in self.classification.iter().enumerate() {
            let field = format!("record_suggestions.classification.{index}");
            if !classified_parts.insert(suggestion.part_version_id) {
                return Err(ContractError::invalid(format!("{field}.part_version_id"), "a part version appears twice in one suggestion write"));
            }
            validate_recorded_prediction(&suggestion.prediction, &format!("{field}.prediction"))?;
        }

        let mut named_pairs = BTreeSet::new();
        let mut per_part: BTreeMap<Uuid, usize> = BTreeMap::new();
        for (index, suggestion) in self.catalog.iter().enumerate() {
            let field = format!("record_suggestions.catalog.{index}");
            suggestion.validate(&field)?;
            if !named_pairs.insert((suggestion.part_version_id, suggestion.component_version_id)) {
                return Err(ContractError::invalid(
                    format!("{field}.component_version_id"),
                    "one part version names the same catalog component version twice in one suggestion write",
                ));
            }
            let count = per_part.entry(suggestion.part_version_id).or_insert(0);
            *count += 1;
            if *count > MAX_CATALOG_SUGGESTIONS_PER_PART {
                return Err(ContractError::invalid(
                    format!("{field}.part_version_id"),
                    format!("one part version holds at most {MAX_CATALOG_SUGGESTIONS_PER_PART} catalog suggestions"),
                ));
            }
        }
        Ok(())
    }
}

/// A recorded proposal must name the exact analysis it describes, the prediction version that
/// produced it and at least one evidence item.
fn validate_recorded_prediction(prediction: &ClassificationPrediction, field: &str) -> Result<()> {
    prediction.validate(field)?;
    if prediction.prediction_version.is_none() {
        return Err(ContractError::invalid(
            format!("{field}.prediction_version"),
            "a recorded classification suggestion names the prediction version that produced it",
        ));
    }
    if prediction.analysis_key.is_none() {
        return Err(ContractError::invalid(
            format!("{field}.analysis_key"),
            "a recorded classification suggestion binds the exact analysis it describes",
        ));
    }
    if prediction.evidence.is_empty() {
        return Err(ContractError::invalid(format!("{field}.evidence"), "a recorded classification suggestion shows at least one evidence item"));
    }
    Ok(())
}

pub(crate) fn validate_evidence(evidence: &[SuggestionEvidence], field: &str) -> Result<()> {
    if evidence.len() > MAX_SUGGESTION_EVIDENCE {
        return Err(ContractError::invalid(field, format!("one suggestion carries at most {MAX_SUGGESTION_EVIDENCE} evidence items")));
    }
    for (index, item) in evidence.iter().enumerate() {
        item.validate(&format!("{field}.{index}"))?;
    }
    Ok(())
}

fn check_request_id(request_id: &str, field: &str) -> Result<()> {
    check_text(request_id, field)?;
    if request_id.chars().count() > MAX_REQUEST_ID_CHARS {
        return Err(ContractError::invalid(field, "the request id is too long"));
    }
    Ok(())
}

fn check_reviewer(reviewer: &str, field: &str) -> Result<()> {
    check_bounded_text(reviewer, field, MAX_REVIEWER_CHARS)
}

fn check_catalog_version(version: u64, field: &str) -> Result<()> {
    if version == 0 {
        return Err(ContractError::invalid(field, "a published catalog version starts at 1"));
    }
    Ok(())
}

/// A catalog name follows the publication rule exactly: non-empty text bounded by the shared-name
/// limit. A published value must always be freezable into a candidate or confirmation, so this
/// validator cannot be stricter than `CreateComponentVersion`/`check_shared_name`.
fn check_catalog_name(name: &str, field: &str) -> Result<()> {
    check_text(name, field)?;
    if name.chars().count() > MAX_SHARED_NAME_CHARS {
        return Err(ContractError::invalid(field, format!("the text is longer than {MAX_SHARED_NAME_CHARS} characters")));
    }
    Ok(())
}

/// A catalog specification follows the publication rule exactly: non-empty text bounded by the
/// long-reference limit, so a published multi-line specification can be frozen and confirmed.
fn check_catalog_specification(specification: &str, field: &str) -> Result<()> {
    check_text(specification, field)?;
    if specification.chars().count() > MAX_SOURCE_REFERENCE_CHARS {
        return Err(ContractError::invalid(field, format!("the text is longer than {MAX_SOURCE_REFERENCE_CHARS} characters")));
    }
    Ok(())
}

fn check_catalog_price(price: Option<Decimal>, field: &str) -> Result<()> {
    if price.is_some_and(|price| price.is_sign_negative()) {
        return Err(ContractError::invalid(field, "a price cannot be negative"));
    }
    Ok(())
}

fn check_catalog_price_unit(unit: Unit, field: &str) -> Result<()> {
    if unit != Unit::CurrencyPerPiece {
        return Err(ContractError::invalid(field, "a catalog component price is recorded per piece"));
    }
    Ok(())
}

fn check_currency(currency: &str, field: &str) -> Result<()> {
    if currency.len() != 3 || !currency.bytes().all(|byte| byte.is_ascii_uppercase()) {
        return Err(ContractError::invalid(field, "expected a three-letter upper-case currency code"));
    }
    Ok(())
}

fn check_confidence(confidence: Option<Decimal>, field: &str) -> Result<()> {
    if let Some(confidence) = confidence
        && (confidence.is_sign_negative() || confidence > Decimal::ONE)
    {
        return Err(ContractError::invalid(field, "a confidence is between 0 and 1"));
    }
    Ok(())
}

/// A non-empty text bounded in characters and free of control characters.
pub(crate) fn check_bounded_text(text: &str, field: &str, max_chars: usize) -> Result<()> {
    check_text(text, field)?;
    if text.chars().count() > max_chars {
        return Err(ContractError::invalid(field, format!("the text is longer than {max_chars} characters")));
    }
    if text.chars().any(char::is_control) {
        return Err(ContractError::invalid(field, "the text contains a control character"));
    }
    Ok(())
}

/// An optional recorded timestamp stays a bounded piece of text; the service owns its meaning.
pub(crate) fn check_optional_timestamp(suggested_at: &Option<String>, field: &str) -> Result<()> {
    match suggested_at {
        Some(timestamp) => check_bounded_text(timestamp, field, MAX_TIMESTAMP_CHARS),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::AnalysisKey;

    fn key() -> AnalysisKey {
        AnalysisKey { source_sha256: "a".repeat(64), engine_build: "0.40.0".into(), extractor_version: "2".into() }
    }

    fn evidence(code: &str) -> SuggestionEvidence {
        SuggestionEvidence::new(code, "the rule measured the constant diameter").unwrap()
    }

    fn recorded_prediction() -> ClassificationPrediction {
        ClassificationPrediction {
            name: "tube".into(),
            model_version: "recognition-rules-1".into(),
            confidence: None,
            prediction_version: Some("1".into()),
            analysis_key: Some(key()),
            evidence: vec![evidence("tubular_sweep")],
            suggested_at: None,
        }
    }

    fn catalog_suggestion(part_version_id: Uuid, component_version_id: Uuid) -> CatalogMatchSuggestion {
        CatalogMatchSuggestion {
            part_version_id,
            component_id: Uuid::from_u128(200),
            component_version_id,
            model_version: "catalog-text-match-1".into(),
            prediction_version: "1".into(),
            confidence: Some("0.5".parse().unwrap()),
            evidence: vec![evidence("catalog_attributes_match")],
        }
    }

    fn confirmed() -> ConfirmedCatalogReference {
        ConfirmedCatalogReference {
            component_id: Uuid::from_u128(200),
            component_version_id: Uuid::from_u128(201),
            version: 3,
            name: "Hose clip 12-20".into(),
            specification: "stainless steel, 12-20 mm".into(),
            price: Some("0.42".parse().unwrap()),
            price_unit: Unit::CurrencyPerPiece,
            currency: "CNY".into(),
            confirmed_by: "engineer".into(),
        }
    }

    fn candidate(part_version_id: Uuid, component_version_id: Uuid) -> CatalogMatchCandidate {
        CatalogMatchCandidate {
            suggestion_id: Uuid::from_u128(300),
            part_version_id,
            component_id: Uuid::from_u128(200),
            component_version_id,
            catalog_version: 3,
            catalog_name: "Hose clip 12-20".into(),
            specification: "stainless steel, 12-20 mm".into(),
            price: Some("0.42".parse().unwrap()),
            price_unit: Unit::CurrencyPerPiece,
            currency: "CNY".into(),
            model_version: "catalog-text-match-1".into(),
            prediction_version: "1".into(),
            confidence: Some(Decimal::ONE),
            evidence: vec![evidence("catalog_attributes_match")],
            suggested_at: None,
        }
    }

    fn project_override() -> ProjectCatalogOverride {
        ProjectCatalogOverride {
            specification: "project stock: stainless steel, 12-20 mm".into(),
            price: Some("0.55".parse().unwrap()),
            price_unit: Unit::CurrencyPerPiece,
            currency: "CNY".into(),
            reason: "project buys a different supplier".into(),
            recorded_by: "engineer".into(),
        }
    }

    #[test]
    fn a_published_multiline_catalog_specification_can_be_frozen_and_confirmed() {
        // The catalog publication path accepts a multi-line specification (it follows the shared
        // version 2 text rule), so a frozen candidate, the confirmed reference and the project
        // override must accept the same value instead of rejecting its newline as a control
        // character. Otherwise a valid published catalog version could never be suggested.
        let multiline = "steel\n12-20 mm";
        let mut proposed = candidate(Uuid::from_u128(100), Uuid::from_u128(201));
        proposed.specification = multiline.into();
        assert!(proposed.validate("candidate").is_ok(), "a published multi-line specification is freezable");

        let mut reference = confirmed();
        reference.specification = multiline.into();
        assert!(reference.validate("confirmed").is_ok(), "the confirmed reference keeps the same value");

        let mut project_override = project_override();
        project_override.specification = multiline.into();
        assert!(project_override.validate("override").is_ok(), "a project override may replace it with the same text shape");

        // The publication bound still applies to the frozen value.
        let mut too_long = candidate(Uuid::from_u128(100), Uuid::from_u128(201));
        too_long.specification = "x".repeat(crate::shared_data::MAX_SOURCE_REFERENCE_CHARS + 1);
        assert!(too_long.validate("candidate").is_err(), "the published text bound still applies");
    }

    #[test]
    fn the_initial_categories_are_confirmed_and_open_codes_extend_them() {
        for code in INITIAL_CATEGORIES {
            let category = CategoryCode::new(code).unwrap();
            assert!(category.is_initial(), "{code}");
            assert_eq!(serde_json::to_value(&category).unwrap(), serde_json::json!(code));
        }
        let extended = CategoryCode::new("custom_bracket").unwrap();
        assert!(!extended.is_initial());
        assert_eq!(extended.as_str(), "custom_bracket");
        let round_tripped: CategoryCode = serde_json::from_value(serde_json::json!("custom_bracket")).unwrap();
        assert_eq!(round_tripped, extended);
        // Adding a category needs no schema or identity change: any well-formed code round-trips.
        assert!(CategoryCode::new("bracket").is_ok());
        assert!(CategoryCode::new("a").is_ok());
        assert!(CategoryCode::new("a".repeat(MAX_CATEGORY_CHARS)).is_ok());
    }

    #[test]
    fn an_invalid_category_code_is_refused() {
        for code in ["", "Tube", "1tube", "_tube", "tube-1", "tube 1", "tube.mount", "tübe", &"a".repeat(MAX_CATEGORY_CHARS + 1)] {
            assert!(CategoryCode::new(code).is_err(), "{code:?}");
            assert!(serde_json::from_value::<CategoryCode>(serde_json::json!(code)).is_err(), "{code:?}");
        }
    }

    #[test]
    fn evidence_needs_a_stable_bounded_code_and_explanation() {
        let valid = SuggestionEvidence::new("tubular_sweep", "the body is one swept tube").unwrap();
        assert!(valid.validate("evidence").is_ok());
        assert!(SuggestionEvidence::new("TubularSweep", "detail").is_err());
        assert!(SuggestionEvidence::new("", "detail").is_err());
        assert!(SuggestionEvidence::new("tubular_sweep", " ").is_err());
        assert!(SuggestionEvidence::new("tubular_sweep", "x".repeat(MAX_EVIDENCE_DETAIL_CHARS + 1)).is_err());
        assert!(SuggestionEvidence::new("tubular_sweep", "line\nbreak").is_err());
    }

    #[test]
    fn a_suggestion_write_needs_a_bound_source_evidence_and_unique_identities() {
        let part = Uuid::from_u128(100);
        let valid = RecordSuggestions {
            request_id: "suggest-1".into(),
            expected_version: 0,
            classification: vec![ClassificationSuggestion { part_version_id: part, prediction: recorded_prediction() }],
            catalog: vec![catalog_suggestion(part, Uuid::from_u128(201))],
            catalog_evaluated_part_versions: Vec::new(),
        };
        assert!(valid.validate().is_ok());

        let evaluated_only =
            RecordSuggestions { classification: Vec::new(), catalog: Vec::new(), catalog_evaluated_part_versions: vec![part], ..valid.clone() };
        assert!(evaluated_only.validate().is_ok(), "a no-match matching run is representable");
        let duplicate_evaluated =
            RecordSuggestions { classification: Vec::new(), catalog: Vec::new(), catalog_evaluated_part_versions: vec![part, part], ..valid.clone() };
        assert!(duplicate_evaluated.validate().is_err(), "an evaluated part version is named once");

        let empty = RecordSuggestions { classification: Vec::new(), catalog: Vec::new(), ..valid.clone() };
        assert!(empty.validate().is_err(), "an empty write is refused");

        let no_prediction_version = RecordSuggestions {
            classification: vec![ClassificationSuggestion {
                part_version_id: part,
                prediction: ClassificationPrediction { prediction_version: None, ..recorded_prediction() },
            }],
            catalog: Vec::new(),
            ..valid.clone()
        };
        assert!(no_prediction_version.validate().is_err(), "a recorded suggestion names its prediction version");

        let no_analysis = RecordSuggestions {
            classification: vec![ClassificationSuggestion {
                part_version_id: part,
                prediction: ClassificationPrediction { analysis_key: None, ..recorded_prediction() },
            }],
            catalog: Vec::new(),
            ..valid.clone()
        };
        assert!(no_analysis.validate().is_err(), "a recorded suggestion binds its exact analysis");

        let no_evidence = RecordSuggestions {
            classification: vec![ClassificationSuggestion {
                part_version_id: part,
                prediction: ClassificationPrediction { evidence: Vec::new(), ..recorded_prediction() },
            }],
            catalog: Vec::new(),
            ..valid.clone()
        };
        assert!(no_evidence.validate().is_err(), "a recorded suggestion shows evidence");

        let duplicate_part = RecordSuggestions {
            classification: vec![
                ClassificationSuggestion { part_version_id: part, prediction: recorded_prediction() },
                ClassificationSuggestion { part_version_id: part, prediction: recorded_prediction() },
            ],
            catalog: Vec::new(),
            ..valid.clone()
        };
        assert!(duplicate_part.validate().is_err());

        let duplicate_catalog_pair = RecordSuggestions {
            classification: Vec::new(),
            catalog: vec![catalog_suggestion(part, Uuid::from_u128(201)), catalog_suggestion(part, Uuid::from_u128(201))],
            ..valid.clone()
        };
        assert!(duplicate_catalog_pair.validate().is_err());

        let no_catalog_evidence = RecordSuggestions {
            classification: Vec::new(),
            catalog: vec![CatalogMatchSuggestion { evidence: Vec::new(), ..catalog_suggestion(part, Uuid::from_u128(201)) }],
            ..valid.clone()
        };
        assert!(no_catalog_evidence.validate().is_err());

        let out_of_range_confidence = RecordSuggestions {
            classification: Vec::new(),
            catalog: vec![CatalogMatchSuggestion { confidence: Some("1.5".parse().unwrap()), ..catalog_suggestion(part, Uuid::from_u128(201)) }],
            ..valid.clone()
        };
        assert!(out_of_range_confidence.validate().is_err());

        let mut too_many_candidates = valid.clone();
        too_many_candidates.classification.clear();
        too_many_candidates.catalog =
            (0..=MAX_CATALOG_SUGGESTIONS_PER_PART).map(|index| catalog_suggestion(part, Uuid::from_u128(400 + index as u128))).collect();
        assert!(too_many_candidates.validate().is_err(), "one part holds a bounded number of candidates");

        let mut too_many_classifications = valid.clone();
        too_many_classifications.catalog.clear();
        too_many_classifications.classification = (0..=MAX_SUGGESTIONS_PER_WRITE)
            .map(|index| ClassificationSuggestion { part_version_id: Uuid::from_u128(1 + index as u128), prediction: recorded_prediction() })
            .collect();
        assert!(too_many_classifications.validate().is_err(), "a write is bounded");

        assert!(RecordSuggestions { request_id: " ".into(), ..valid.clone() }.validate().is_err());
        assert!(RecordSuggestions { request_id: "x".repeat(MAX_REQUEST_ID_CHARS + 1), ..valid }.validate().is_err());
    }

    #[test]
    fn a_pending_suggestion_never_supplies_authoritative_inputs() {
        let part = Uuid::from_u128(100);
        let mut record = CatalogRecord::pending();
        assert!(record.effective_inputs().is_none());
        assert!(record.validate("catalog").is_ok());

        // Even a candidate at confidence 1 is evidence only.
        record.suggestions.push(candidate(part, Uuid::from_u128(201)));
        assert!(record.validate("catalog").is_ok());
        assert!(record.effective_inputs().is_none(), "an unconfirmed suggestion never supplies price or specification");
    }

    #[test]
    fn confirmed_catalog_values_are_returned_and_a_project_override_stays_separate() {
        let mut record = CatalogRecord { confirmed: Some(confirmed()), ..CatalogRecord::pending() };
        assert!(record.validate("catalog").is_ok());
        let inputs = record.effective_inputs().expect("a confirmed reference supplies inputs");
        assert_eq!(inputs.component_version_id, Uuid::from_u128(201));
        assert_eq!(inputs.specification, "stainless steel, 12-20 mm");
        assert_eq!(inputs.price, Some("0.42".parse().unwrap()));
        assert!(!inputs.override_applied);

        record.project_override = Some(project_override());
        assert!(record.validate("catalog").is_ok());
        let inputs = record.effective_inputs().expect("a confirmed reference supplies inputs");
        assert_eq!(inputs.specification, "project stock: stainless steel, 12-20 mm");
        assert_eq!(inputs.price, Some("0.55".parse().unwrap()));
        assert!(inputs.override_applied);

        // The confirmed reference keeps the original catalog version and value beside the override.
        let confirmed = record.confirmed.as_ref().expect("the confirmed reference is retained");
        assert_eq!(confirmed.version, 3);
        assert_eq!(confirmed.specification, "stainless steel, 12-20 mm");
        assert_eq!(confirmed.price, Some("0.42".parse().unwrap()));
        assert_eq!(record.project_override.as_ref().expect("the override is recorded").price, Some("0.55".parse().unwrap()));
    }

    #[test]
    fn an_override_without_a_confirmed_reference_is_refused() {
        let part = Uuid::from_u128(100);
        let record = CatalogRecord {
            suggestions: vec![candidate(part, Uuid::from_u128(201))],
            project_override: Some(project_override()),
            ..CatalogRecord::pending()
        };
        assert!(record.validate("catalog").is_err());
        let mut without_override = record;
        without_override.project_override = None;
        assert!(without_override.validate("catalog").is_ok());
    }

    #[test]
    fn a_catalog_record_is_bounded_and_refuses_duplicate_proposal_identities() {
        let part = Uuid::from_u128(100);
        let mut record = CatalogRecord { suggestions: vec![candidate(part, Uuid::from_u128(201))], ..CatalogRecord::pending() };
        assert!(record.validate("catalog").is_ok());

        record.suggestions.push(candidate(part, Uuid::from_u128(201)));
        assert!(record.validate("catalog").is_err(), "a catalog version is proposed once");

        record.suggestions = (0..=MAX_CATALOG_SUGGESTIONS_PER_PART)
            .map(|index| CatalogMatchCandidate {
                suggestion_id: Uuid::from_u128(500 + index as u128),
                component_version_id: Uuid::from_u128(600 + index as u128),
                ..candidate(part, Uuid::from_u128(201))
            })
            .collect();
        assert!(record.validate("catalog").is_err(), "the candidate list is bounded");

        record.suggestions = vec![
            CatalogMatchCandidate { suggestion_id: Uuid::from_u128(500), ..candidate(part, Uuid::from_u128(201)) },
            CatalogMatchCandidate { suggestion_id: Uuid::from_u128(500), ..candidate(part, Uuid::from_u128(202)) },
        ];
        assert!(record.validate("catalog").is_err(), "suggestion ids are unique");

        record.suggestions = vec![
            CatalogMatchCandidate { suggestion_id: Uuid::from_u128(701), ..candidate(part, Uuid::from_u128(201)) },
            CatalogMatchCandidate { suggestion_id: Uuid::from_u128(702), ..candidate(Uuid::from_u128(101), Uuid::from_u128(202)) },
        ];
        assert!(record.validate("catalog").is_err(), "one record holds one part version's suggestions");
    }

    #[test]
    fn a_catalog_record_round_trips_and_old_json_still_parses() {
        let record = CatalogRecord { confirmed: Some(confirmed()), project_override: Some(project_override()), ..CatalogRecord::pending() };
        let value = serde_json::to_value(&record).unwrap();
        assert_eq!(serde_json::from_value::<CatalogRecord>(value).unwrap(), record);

        let old = serde_json::json!({
            "suggestions": [],
            "confirmed": null,
            "project_override": null,
        });
        assert_eq!(serde_json::from_value::<CatalogRecord>(old).unwrap(), CatalogRecord::pending());
        assert_eq!(serde_json::from_value::<CatalogRecord>(serde_json::json!({})).unwrap(), CatalogRecord::pending());
    }

    #[test]
    fn a_held_catalog_reference_prices_are_missing_not_zero() {
        let mut reference = confirmed();
        reference.price = None;
        let record = CatalogRecord { confirmed: Some(reference), ..CatalogRecord::pending() };
        assert_eq!(serde_json::to_value(&record).unwrap()["confirmed"]["price"], serde_json::Value::Null);
        assert!(record.effective_inputs().unwrap().price.is_none());

        let mut zero = confirmed();
        zero.price = Some(Decimal::ZERO);
        let record = CatalogRecord { confirmed: Some(zero), ..CatalogRecord::pending() };
        assert_eq!(serde_json::to_value(&record).unwrap()["confirmed"]["price"], serde_json::json!("0"));
    }

    #[test]
    fn confirmed_reference_and_override_reject_malformed_values() {
        let mut reference = confirmed();
        reference.version = 0;
        assert!(reference.validate("confirmed").is_err());
        reference.version = 3;
        reference.price = Some("-1".parse().unwrap());
        assert!(reference.validate("confirmed").is_err());
        reference.price = Some(Decimal::ONE);
        reference.price_unit = Unit::CurrencyPerHour;
        assert!(reference.validate("confirmed").is_err());
        reference.price_unit = Unit::CurrencyPerPiece;
        reference.currency = "cny".into();
        assert!(reference.validate("confirmed").is_err());
        reference.currency = "CNY".into();
        reference.confirmed_by = " ".into();
        assert!(reference.validate("confirmed").is_err());

        let mut project_override = project_override();
        project_override.reason = " ".into();
        assert!(project_override.validate("project_override").is_err());
        project_override.reason = "because".into();
        project_override.specification = " ".into();
        assert!(project_override.validate("project_override").is_err());
    }
}
