# Project save and calculation contract

Version: **1** · Status: **published by SB-01** (not coordinator-locked)
Base: `aadc9b31bb6a4ffb30990818e1a0624c82bf2386` · SB-00 prerequisite: `b5115cda78f629cd82ef86d1f90e67a20f2a7c69`
Publication commit: `1ab7b3b28c08b9a7afadee725c95f62465403ef7` (these artifacts); the integration commit is recorded in `docs/solidbom/execution/SB-01.md`

Portable types: `crates/solidbom-contract/src/types.rs` and `crates/solidbom-contract/src/api.rs`
(`ProjectVersionRef`, `CalculationReceipt`).
Producers: the desktop controller (`crates/engine/src/solidbom/state.rs`); the service store.
Consumers: the desktop, the calculator, reopen and later export packages.
Acceptance: the `types` and `api` test modules; the P05/P06/P07 representability round trips below.

## Draft and saved versions

`DraftProject { project_id, product_revision_id, version, source, analysis,
material_overhead_rate, parts, forecast_year, forecast_quantity, currency }`. `version` is `0` for
a project that has never been saved. `validate()` binds the source hash to the analysis key,
requires a four-digit calendar year, a positive forecast and the prototype currency (CNY), unique
part/definition ids and measurements bound to the analysis. A saved version is immutable: a save
appends a version and moves the project's current version; it never rewrites history.

## Save request and receipt

`SaveDraft { request_id, expected_version, draft }`; `validate()` requires a non-blank request id
and `draft.version == expected_version`.

- **Idempotency:** the service keys the request by `request_id` plus a canonical payload hash
  (operation + validated value). An identical retry returns the original `SaveReceipt` **before**
  the current-version check. A changed payload under the same `request_id` is a conflict (`409`).
- **Optimistic concurrency:** `expected_version` must equal the stored current version; otherwise
  the save is a conflict and no version is written. A lost response is retried with the same
  request id and payload, never a new one.
- `SaveReceipt { request_id, project_id, version }` is the committed version. A project save is a
  central write; it is separate from a local CAD file save and never enters CAD undo.

## Calculation request and receipt

`CalculateRequest { request_id, project_id, saved_version }` names a **persisted** version;
`saved_version` is at least 1. The service resolves reference data on its side, runs the calculator
and appends a `CalculationSnapshot`. `CalculationReceipt::of(request_id, &snapshot)` is the compact
identity (`request_id`, `calculation_id`, `project_id`, `saved_version`) for history rows and
idempotent retries; the HTTP response itself is the snapshot.

`ProjectVersionRef { project_id, version }` (`version > 0`) names one immutable saved version.

## Calculation inputs, output and snapshot

- `CalculationInputs { currency, material_overhead_rate, parts }` with `ResolvedPartInput` values:
  `part_version_id` and `quantity` (both required, non-optional), then `volume_mm3`,
  `density_g_cm3`, `material_price_per_kg`, `allowance_rate`, `cycle_seconds`, `cavities`,
  `production_rate_per_hour`, `setup_hours`, `setup_rate_per_hour`. Versions have become the values
  they name; every optional value that is missing stays `None`.
- `CalculationOutcome { state, parts, assembly_subtotal, setup_total, issues }` where
  `state = incomplete | complete`; `PartCost` carries `net_mass_kg`, `allowed_material_cost`,
  `material_overhead`, `production_cost`, `per_part_subtotal`, `line_total` and `explanations:
  Vec<CostLine>`. Setup is a separate `setup_total`, never folded into a per-part subtotal.
- `CalculationSnapshot { calculation_id, project_id, saved_version, input_hash,
  calculator_version, inputs, outputs, issues }` is append-only. It retains the resolved material,
  rate and policy values, explanations, issues, input hash and calculator version, so a historical
  result remains readable and reproducible. A later edit or recalculation appends a new snapshot;
  it never replaces an existing one.

## Stale results

A result is bound to its saved version, input hash, calculator version and (from SB-03) the
material/rate/policy versions. It is marked stale — not current — when the project version moves,
the reviewed inputs change, the analysis key changes or the session binds another source. A stale
snapshot remains readable with its original inputs. The client's `JobEnvelope` generations carry
the analysis and local-edit information for an asynchronous answer, but `stale_reason` alone does
not detect a project-version change: consumers must also compare `snapshot.saved_version` with the
current draft `version` (the controller does) and never present an older calculation as current.

## Representability of the prototype scenarios

| Check | Fields that carry it |
|---|---|
| P01 import/source | `SourceRef.sha256`/`byte_len`/`original_name`, `AnalysisKey`, `AnalysisManifest` definitions/occurrences/bindings |
| P02 selection | `OccurrenceRef.occurrence_id`/`definition_id`/`source_path`, `GeometryBinding`, shared `part_version_id` |
| P03 volume/units | `Measurement.value`/`unit`/`source`/`review`, `QuantityExpansion` |
| P04 review | `PartInputs.classification_confirmed`, `Sourcing`, `ReviewedClassification`, `MaterialVersion` selection |
| P05 arithmetic | `CalculationInputs`/`ResolvedPartInput` values, `PartCost` lines, `assembly_subtotal`, `setup_total`, `CostLine.basis` |
| P06 save/reopen | `DraftProject` + `SourceRef` + `AnalysisManifest` + `SaveReceipt` + `ProjectView`/`CalculationSummary` + `CalculationSnapshot` |
| P07 invalidation | `saved_version` + changed `OperationInputs.cycle_seconds` + `JobEnvelope.local_edit_generation`/`StaleReason` + both snapshots |
| P08 missing data | every input is `Option`; a missing value is `null`; `Issue { code, field_path }` names the missing input |
| P09 surfaces/manual | `Measurement.source = manual`, `reason`, `review`; volume `null` until reviewed |
| P10 revision separation | distinct `definition_id`/`part_version_id` per source definition; `customer_revision` is metadata, not identity |
| P11 flat input | `AnalysisManifest.issues` with `unresolved_grouping`; no invented component identities |
| P12 failures | `ClientFailure`/`HostError` classes; explicit retry only; `SaveState::SaveFailed`, no “saved” state |
| P13 responsiveness | `SolidBomHost` submit/poll off the window thread; `JobCompletion` bound to its envelope |
| P14 scope honesty | command contract in `commands.md`; no activation/export command is published by v1 |

`ProjectView { draft, calculations }` is what reopen returns; `CalculationSummary` is the compact
history row. The full snapshot is loaded by `calculation_id`.

## Compatibility

Adding an optional field is compatible. Changing `saved_version`, `input_hash`, the calculator
version meaning or the snapshot's immutability is breaking and needs coordinator approval.
