//! Reviewed engineering records that are not part of a saved draft yet: quantity expansion,
//! annotations kept apart from source geometry, the versioned material-overhead policy and the
//! reason a saved answer is stale.
//!
//! These are version 1 projections: `DraftProject` and `PartInputs` keep their shape, and a later
//! package attaches annotations and the policy version additively.

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{ContractError, Result};
use crate::types::{AnalysisKey, AnalysisManifest, PartInputs};

/// The most characters one annotation may carry.
pub const MAX_ANNOTATION_CHARS: usize = 4096;

/// How many occurrences one reviewed definition expands to. The quantity is derived from the
/// manifest, never typed by an engineer or taken from a display row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QuantityExpansion {
    pub part_version_id: Uuid,
    pub definition_id: Uuid,
    pub occurrence_ids: Vec<Uuid>,
    pub quantity: u32,
}

impl QuantityExpansion {
    /// Group a manifest's occurrences by definition, in definition-id order and manifest order
    /// within a definition. A definition with no occurrence is omitted.
    pub fn from_analysis(manifest: &AnalysisManifest) -> Result<Vec<Self>> {
        let mut grouped: BTreeMap<Uuid, Vec<Uuid>> = BTreeMap::new();
        for occurrence in &manifest.occurrences {
            grouped.entry(occurrence.definition_id).or_default().push(occurrence.occurrence_id);
        }
        let mut expansions = Vec::with_capacity(grouped.len());
        for (definition_id, occurrence_ids) in grouped {
            let quantity = u32::try_from(occurrence_ids.len())
                .map_err(|_| ContractError::invalid("quantity.occurrence_ids", "too many occurrences for one definition"))?;
            let expansion = Self { part_version_id: part_version_for(manifest.key.clone(), definition_id), definition_id, occurrence_ids, quantity };
            expansion.validate()?;
            expansions.push(expansion);
        }
        Ok(expansions)
    }

    pub fn validate(&self) -> Result<()> {
        if self.quantity == 0 {
            return Err(ContractError::invalid("quantity.quantity", "a reviewed definition needs at least one occurrence"));
        }
        if usize::try_from(self.quantity).ok() != Some(self.occurrence_ids.len()) {
            return Err(ContractError::invalid("quantity.occurrence_ids", "the quantity must equal the occurrence count"));
        }
        if self.occurrence_ids.iter().collect::<std::collections::BTreeSet<_>>().len() != self.occurrence_ids.len() {
            return Err(ContractError::invalid("quantity.occurrence_ids", "an occurrence appears twice"));
        }
        Ok(())
    }
}

/// The stable reviewed-part id a definition expands to. Deterministic so a re-import of the same
/// source and extractor reaches the same reviewed record.
pub fn part_version_for(key: AnalysisKey, definition_id: Uuid) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_OID, format!("{}:part:{}", key.source_sha256, definition_id).as_bytes())
}

/// An engineer's note about reviewed work. Stored apart from the imported geometry and customer
/// hierarchy; it never mutates either.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Annotation {
    pub annotation_id: Uuid,
    pub part_version_id: Uuid,
    pub definition_id: Option<Uuid>,
    pub analysis_key: AnalysisKey,
    pub text: String,
}

impl Annotation {
    pub fn validate(&self) -> Result<()> {
        if self.text.trim().is_empty() {
            return Err(ContractError::invalid("annotation.text", "an annotation needs text"));
        }
        if self.text.chars().count() > MAX_ANNOTATION_CHARS {
            return Err(ContractError::invalid("annotation.text", "the annotation is longer than the contract accepts"));
        }
        self.analysis_key.validate()
    }
}

/// What a material-overhead rate is a percentage of.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OverheadBasis {
    /// The allowed material cost of the part.
    MaterialCost,
}

/// A server-owned, versioned material-overhead policy. Zero is a real rate; a missing policy is
/// `None` at the caller, never zero.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MaterialOverheadPolicy {
    pub policy_version: String,
    pub rate: Decimal,
    pub basis: OverheadBasis,
}

impl MaterialOverheadPolicy {
    pub fn validate(&self) -> Result<()> {
        if self.policy_version.trim().is_empty() {
            return Err(ContractError::invalid("overhead.policy_version", "a policy needs a version"));
        }
        if self.rate.is_sign_negative() {
            return Err(ContractError::invalid("overhead.rate", "an overhead rate cannot be negative"));
        }
        Ok(())
    }
}

/// An engineer's classification decision. A suggestion is pending until a person confirms it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewedClassification {
    pub name: String,
    pub confirmed: bool,
}

impl ReviewedClassification {
    pub fn validate(&self) -> Result<()> {
        if self.confirmed && self.name.trim().is_empty() {
            return Err(ContractError::invalid("classification.name", "a confirmed classification needs a name"));
        }
        Ok(())
    }
}

impl From<&PartInputs> for ReviewedClassification {
    fn from(part: &PartInputs) -> Self {
        Self { name: part.classification.clone(), confirmed: part.classification_confirmed }
    }
}

/// Why an answer describes an earlier record. Reported with the stale result so a person sees
/// what changed, not only that it changed.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StaleReason {
    /// The project now describes another source file.
    SourceChanged,
    /// The engine build that extracted the source changed.
    EngineChanged,
    /// The extractor version that produced the analysis changed.
    ExtractorChanged,
    /// The session bound another reviewed source.
    BindingChanged,
    /// The reviewed inputs changed after the request was submitted.
    InputsChanged,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{GeometryBinding, OccurrenceRef, SCHEMA_VERSION};

    fn key() -> AnalysisKey {
        AnalysisKey { source_sha256: "a".repeat(64), engine_build: "0.40.0".into(), extractor_version: "1".into() }
    }

    fn occurrence(id: u128, definition: u128) -> OccurrenceRef {
        OccurrenceRef {
            occurrence_id: Uuid::from_u128(id),
            definition_id: Uuid::from_u128(definition),
            parent_occurrence_id: None,
            source_path: vec![id as u64],
            transform: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]],
        }
    }

    fn manifest() -> AnalysisManifest {
        AnalysisManifest {
            schema_version: SCHEMA_VERSION,
            key: key(),
            definitions: Vec::new(),
            occurrences: vec![occurrence(1, 10), occurrence(2, 10), occurrence(3, 20)],
            geometry_bindings: vec![GeometryBinding {
                source_path: vec![1],
                source_body: 0,
                definition_id: Uuid::from_u128(10),
                occurrence_id: Uuid::from_u128(1),
            }],
            issues: Vec::new(),
        }
    }

    #[test]
    fn an_expansion_counts_the_occurrences_of_each_definition() {
        let expansions = QuantityExpansion::from_analysis(&manifest()).unwrap_or_default();
        assert_eq!(expansions.len(), 2);
        assert_eq!(expansions[0].definition_id, Uuid::from_u128(10));
        assert_eq!(expansions[0].quantity, 2);
        assert_eq!(expansions[1].definition_id, Uuid::from_u128(20));
        assert_eq!(expansions[1].quantity, 1);
        for expansion in &expansions {
            assert!(expansion.validate().is_ok());
        }
    }

    #[test]
    fn an_expansion_quantity_must_match_its_occurrences() {
        let expansion =
            QuantityExpansion { part_version_id: Uuid::nil(), definition_id: Uuid::nil(), occurrence_ids: vec![Uuid::nil()], quantity: 2 };
        assert!(expansion.validate().is_err());
        let empty = QuantityExpansion { quantity: 0, ..expansion };
        assert!(empty.validate().is_err());
    }

    #[test]
    fn an_annotation_needs_text_and_an_analysis_binding() {
        let note = Annotation {
            annotation_id: Uuid::nil(),
            part_version_id: Uuid::nil(),
            definition_id: None,
            analysis_key: key(),
            text: "measure this one on the bench".into(),
        };
        assert!(note.validate().is_ok());
        assert!(Annotation { text: "  ".into(), ..note.clone() }.validate().is_err());
        assert!(Annotation { text: "x".repeat(MAX_ANNOTATION_CHARS + 1), ..note }.validate().is_err());
    }

    #[test]
    fn a_zero_overhead_is_a_real_rate_but_a_negative_one_is_not() {
        let zero = MaterialOverheadPolicy { policy_version: "policy-1".into(), rate: Decimal::ZERO, basis: OverheadBasis::MaterialCost };
        assert!(zero.validate().is_ok());
        assert!(MaterialOverheadPolicy { rate: "-0.05".parse().unwrap_or_default(), ..zero.clone() }.validate().is_err());
        assert!(MaterialOverheadPolicy { policy_version: " ".into(), ..zero }.validate().is_err());
    }

    #[test]
    fn a_confirmed_classification_needs_a_name() {
        assert!(ReviewedClassification { name: "clip".into(), confirmed: true }.validate().is_ok());
        assert!(ReviewedClassification { name: " ".into(), confirmed: true }.validate().is_err());
        assert!(ReviewedClassification { name: " ".into(), confirmed: false }.validate().is_ok());
    }

    #[test]
    fn a_part_projects_its_reviewed_classification() {
        let part = PartInputs {
            part_version_id: Uuid::nil(),
            definition_id: Uuid::nil(),
            classification: "clip".into(),
            classification_confirmed: true,
            sourcing: crate::types::Sourcing::Manufactured,
            volume_mm3: crate::types::Measurement {
                value: Some("2400".parse().unwrap_or_default()),
                unit: crate::types::Unit::MillimetreCubed,
                source: crate::types::MeasurementSource::Extracted,
                review: crate::types::ReviewState::Confirmed,
                reason: None,
                analysis_key: key(),
            },
            material_version_id: None,
            allowance_rate: None,
            operation: None,
        };
        assert_eq!(ReviewedClassification::from(&part), ReviewedClassification { name: "clip".into(), confirmed: true });
    }
}
