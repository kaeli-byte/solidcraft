//! Read-only projection of reviewed definitions for tables and programmatic inspection.

use std::collections::{BTreeMap, BTreeSet};

use rust_decimal::Decimal;
use solidbom_contract::{MaterialVersion, MeasurementSource, PartInputs, ReviewState, Sourcing};
use uuid::Uuid;

use crate::schema::solidbom::{InputsParams, PartIssue, PartRow, PartsAnalysisKey, PartsReply};
use crate::solidbom::extract::definition_occurrence_paths;
use crate::{Result, Session};

pub(super) fn project(session: &Session) -> Result<PartsReply> {
    let state = &session.solidbom;
    let Some(draft) = &state.draft else { return Ok(PartsReply::default()) };
    let paths = definition_occurrence_paths(session, &draft.analysis)?;
    let definitions: BTreeMap<_, _> = draft.analysis.definitions.iter().map(|definition| (definition.definition_id, definition)).collect();
    let mut quantities = BTreeMap::new();
    for occurrence in &draft.analysis.occurrences {
        *quantities.entry(occurrence.definition_id).or_insert(0_usize) += 1;
    }
    let costs: BTreeMap<_, _> =
        state.current.as_ref().map(|snapshot| snapshot.outputs.parts.iter().map(|cost| (cost.part_version_id, cost)).collect()).unwrap_or_default();
    let mut issues = BTreeMap::new();
    for issue in &draft.analysis.issues {
        if let Some(id) = issue.definition_id {
            issues.entry(id).or_insert_with(Vec::new).push(issue);
        }
    }
    if let Some(snapshot) = &state.current {
        for issue in snapshot.issues.iter().chain(&snapshot.outputs.issues) {
            if let Some(id) = issue.definition_id {
                let group = issues.entry(id).or_insert_with(Vec::new);
                if !group.contains(&issue) {
                    group.push(issue);
                }
            }
        }
    }
    let materials: BTreeMap<_, _> = state
        .reference
        .as_ref()
        .map(|reference| reference.materials.iter().map(|material| (material.material_version_id, material)).collect())
        .unwrap_or_default();
    let rates: BTreeSet<_> =
        state.reference.as_ref().map(|reference| reference.rates.iter().map(|rate| rate.rate_version_id).collect()).unwrap_or_default();
    let components: BTreeMap<_, _> = session.doc.components.iter().map(|component| (component.id, component.name.as_str())).collect();
    let occurrence_components: BTreeMap<_, _> = session.doc.occurrences.iter().map(|occurrence| (occurrence.id, occurrence.component)).collect();
    let result_state = super::state_reply(session).result;
    let mut rows = Vec::with_capacity(draft.parts.len());
    for part in &draft.parts {
        let definition = definitions.get(&part.definition_id);
        let occurrence_paths = paths.get(&part.definition_id).cloned().unwrap_or_default();
        let component_name = occurrence_paths
            .first()
            .and_then(|path| path.last())
            .and_then(|id| occurrence_components.get(id))
            .and_then(|component| components.get(component))
            .map(|name| (*name).to_owned());
        let cost = costs.get(&part.part_version_id);
        let mut part_issues = Vec::new();
        for issue in issues.get(&part.definition_id).into_iter().flatten().chain(issues.get(&part.part_version_id).into_iter().flatten()) {
            if !part_issues.iter().any(|item: &PartIssue| item.code == issue.code && item.field_path == issue.field_path) {
                part_issues.push(PartIssue { code: issue.code.clone(), field_path: issue.field_path.clone(), message: issue.message.clone() });
            }
        }
        let missing_inputs = missing_inputs(part, draft.material_overhead_rate.is_some(), &materials, &rates);
        let volume_reviewed = part.volume_mm3.review == ReviewState::Confirmed;
        rows.push(PartRow {
            part_version_id: part.part_version_id.to_string(),
            definition_id: part.definition_id.to_string(),
            part: definition
                .and_then(|definition| definition.customer_part_number.clone())
                .or(component_name)
                .unwrap_or_else(|| part.definition_id.to_string()),
            revision: definition.and_then(|definition| definition.customer_revision.clone()),
            quantity: quantities.get(&part.definition_id).copied().unwrap_or(0),
            classification: part.classification.clone(),
            classification_confirmed: part.classification_confirmed,
            sourcing: sourcing(part.sourcing).into(),
            material: part.material_version_id.and_then(|id| materials.get(&id)).map(|material| material.name.clone()),
            material_version_id: part.material_version_id.map(|id| id.to_string()),
            volume_mm3: part.volume_mm3.value.map(|value| value.to_string()),
            volume_source: match part.volume_mm3.source {
                MeasurementSource::Extracted => "extracted",
                MeasurementSource::Manual => "manual",
            }
            .into(),
            volume_review: if volume_reviewed { "confirmed" } else { "pending" }.into(),
            manual_reason: part.volume_mm3.reason.clone(),
            needs_review: !part.classification_confirmed || !volume_reviewed || part.volume_mm3.value.is_none() || !part_issues.is_empty(),
            missing_inputs,
            issues: part_issues,
            per_part_subtotal: cost.and_then(|cost| cost.per_part_subtotal).map(|value| value.to_string()),
            line_total: cost.and_then(|cost| cost.line_total).map(|value| value.to_string()),
            result_state: result_state.clone(),
            occurrence_paths,
            inputs: input_fields(part),
        });
    }
    Ok(PartsReply {
        rows,
        currency: Some(draft.currency.clone()),
        material_overhead_rate: draft.material_overhead_rate.map(|value| value.to_string()),
        generation: state.generation,
        analysis_key: Some(PartsAnalysisKey {
            source_sha256: draft.analysis.key.source_sha256.clone(),
            engine_build: draft.analysis.key.engine_build.clone(),
            extractor_version: draft.analysis.key.extractor_version.clone(),
        }),
    })
}

fn sourcing(value: Sourcing) -> &'static str {
    match value {
        Sourcing::Manufactured => "manufactured",
        Sourcing::Purchased => "purchased",
    }
}

fn input_fields(part: &PartInputs) -> InputsParams {
    let operation = part.operation.as_ref();
    InputsParams {
        part_version_id: part.part_version_id.to_string(),
        classification: Some(part.classification.clone()),
        classification_confirmed: Some(part.classification_confirmed),
        sourcing: Some(sourcing(part.sourcing).into()),
        volume_mm3: part.volume_mm3.value.map(|value| value.to_string()),
        manual_reason: part.volume_mm3.reason.clone(),
        volume_confirmed: Some(part.volume_mm3.review == ReviewState::Confirmed),
        material_version_id: part.material_version_id.map(|id| id.to_string()),
        allowance_rate: part.allowance_rate.map(|value| value.to_string()),
        cycle_seconds: operation.and_then(|operation| operation.cycle_seconds).map(|value| value.to_string()),
        cavities: operation.and_then(|operation| operation.cavities),
        production_rate_version_id: operation.and_then(|operation| operation.production_rate_version_id).map(|id| id.to_string()),
        setup_hours: operation.and_then(|operation| operation.setup_hours).map(|value| value.to_string()),
        setup_rate_version_id: operation.and_then(|operation| operation.setup_rate_version_id).map(|id| id.to_string()),
        material_overhead_rate: None,
    }
}

fn missing_inputs(part: &PartInputs, overhead_present: bool, materials: &BTreeMap<Uuid, &MaterialVersion>, rates: &BTreeSet<Uuid>) -> Vec<String> {
    let mut missing = Vec::new();
    let mut require = |field: &str, present: bool| {
        if !present {
            missing.push(field.to_owned());
        }
    };
    require("classification", !part.classification.trim().is_empty());
    require("classification_confirmed", part.classification_confirmed);
    require("volume_mm3", part.volume_mm3.value.is_some());
    require("volume_confirmed", part.volume_mm3.review == ReviewState::Confirmed);
    require("material_version_id", part.material_version_id.and_then(|id| materials.get(&id)).is_some());
    require("allowance_rate", part.allowance_rate.is_some());
    let operation = part.operation.as_ref();
    let cycle = operation.and_then(|operation| operation.cycle_seconds);
    require("cycle_seconds", cycle.is_some());
    if cycle != Some(Decimal::ZERO) {
        require("cavities", operation.and_then(|operation| operation.cavities).is_some());
        require(
            "production_rate_version_id",
            operation.and_then(|operation| operation.production_rate_version_id).is_some_and(|id| rates.contains(&id)),
        );
    }
    let setup = operation.and_then(|operation| operation.setup_hours);
    require("setup_hours", setup.is_some());
    if setup.is_some_and(|hours| hours > Decimal::ZERO) {
        require("setup_rate_version_id", operation.and_then(|operation| operation.setup_rate_version_id).is_some_and(|id| rates.contains(&id)));
    }
    require("material_overhead_rate", overhead_present);
    missing
}
