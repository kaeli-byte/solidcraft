# Reviewed inputs contract

Version: **1** · Status: **published by SB-01** (not coordinator-locked)
Base: `aadc9b31bb6a4ffb30990818e1a0624c82bf2386` · SB-00 prerequisite: `b5115cda78f629cd82ef86d1f90e67a20f2a7c69`
Publication commit: `1ab7b3b28c08b9a7afadee725c95f62465403ef7` (these artifacts); the integration commit is recorded in `docs/solidbom/execution/SB-01.md`

Portable types: `crates/solidbom-contract/src/types.rs` (`PartInputs`, `OperationInputs`,
`Sourcing`, `CycleBasis`) and `crates/solidbom-contract/src/review.rs` (`ReviewedClassification`,
`QuantityExpansion`, `Annotation`, `MaterialOverheadPolicy`, `StaleReason`).
Producers: the desktop review commands (`solidbom.inputs`, `solidbom.inputs.batch`).
Consumers: the service store, the calculator, later engineering/sourcing/feasibility packages.
Acceptance: the `types` and `review` test modules (round trips, missing vs zero, allowance/setup,
quantity expansion, annotation and overhead validation).

## `PartInputs`

One reviewed definition: `part_version_id`, `definition_id`, `classification: String`,
`classification_confirmed: bool`, `sourcing`, `volume_mm3: Measurement`,
`material_version_id: Option<Uuid>`, `allowance_rate: Option<Decimal>`, `operation:
Option<OperationInputs>`. `validate()` refuses a confirmed blank classification, a negative
allowance and an invalid operation.

- A classification is an engineer decision. A CAD suggestion is not a reviewed association; it
  stays pending until a person confirms it (`ReviewedClassification::confirmed`).
- Sourcing is `manufactured` or `purchased`, explicit per part. The prototype calculates
  manufactured parts; a purchased part still carries its reviewed record.
- `material_version_id` names a server-owned `MaterialVersion`; a suggestion is not a selection.
  Missing is `null`, never a default density.
- `allowance_rate` is a fraction (`"0.02"` = 2%), applied once to net material consumption, never
  to quantities, cycles or setup. Zero is a real allowance; missing is `null`.

## `OperationInputs`

`cycle_seconds`, `cycle_basis` (`per_piece` only in v1), `cavities`, `production_rate_version_id`,
`setup_hours`, `setup_rate_version_id`. Cycle and setup times are non-negative; cavities are at
least one when present. Production and setup keep **separate** rate versions and are never
amortised into one figure. A zero cycle is a valid explicit value; a missing cycle is missing.

## `ReviewedClassification` (projection)

`name`, `confirmed`; `From<&PartInputs>` projects the stored fields. A confirmed classification
needs a non-blank name. `validate()` is the v1 rule; no confidence score bypasses confirmation.

## `QuantityExpansion`

`part_version_id`, `definition_id`, `occurrence_ids`, `quantity`. Expansion is derived from the
analysis, never typed or read from a display row: `from_analysis` groups the manifest's
occurrences by definition and requires `quantity == occurrence_ids.len()`. The reviewed definition
is charged once per product with this expanded quantity; repeated occurrences share one reviewed
part record. `part_version_for(key, definition_id)` is the deterministic reviewed id
(`<source_sha256>:part:<definition_id>`).

## `Annotation`

`annotation_id`, optional `definition_id`, `part_version_id`, `analysis_key`, `text` (non-blank,
at most `MAX_ANNOTATION_CHARS = 4096`). Annotations are stored **separately** from the imported
geometry, the customer hierarchy and the part inputs; they never mutate any of them. Attaching the
annotation list to a saved draft is a later additive change, not a v1 field.

## Material overhead

`MaterialOverheadPolicy { policy_version, rate, basis }` with `OverheadBasis::MaterialCost`.
The policy is versioned and server-owned; the snapshot keeps the resolved rate and (from SB-03)
its policy version. `rate` is a fraction; zero is valid and a negative rate is refused. Missing
policy is `None`, never zero. Prototype note: `DraftProject.material_overhead_rate` carries the
project's resolved rate until the policy version is stored additively.

## Source/input/version binding and staleness

Every reviewed value is bound to an `AnalysisKey`; the draft binds `SourceRef.sha256` to
`AnalysisManifest.key.source_sha256`. A request carries a `JobEnvelope` with its project, analysis,
local edit generation and binding generation. `JobEnvelope::stale_reason` reports, most
fundamental first: `source_changed`, `engine_changed`, `extractor_changed`, `binding_changed`,
`inputs_changed`. A stale answer stays readable with its original inputs; it is never presented as
current and never overwrites a newer result.

## Compatibility

All new fields above are projections or standalone records; `DraftProject` and `PartInputs` keep
their v1 shape so existing consumers compile. Future fields are added as optional JSON fields.
