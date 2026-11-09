# Engineering review and flat grouping contract

Version: **1** of this contract, additive to the published version 2 HTTP API · Status:
**published by SB-11** (owner instruction 2026-11-09: start implementation; the locking commit is
recorded in [SB-11's execution record](../../execution/SB-11.md)).

This contract extends version 2 additively. It adds reviewed engineering records beside the project
record; the version 1 routes, the version 2 project routes and their meanings stay exactly as
published. It does not change the stored project document.

Portable types: `crates/solidbom-contract/src/engineering.rs` (re-exported at the crate root).
Producers: the service's engineering store (`services/solidbom-server/src/engineering.rs`) and the
engineering routes below; the desktop engine validates body references against the exact analysis
before it writes. Consumers: the reviewed-group selection adapters and review UI (SB-11 desktop
increment), recognition suggestions (SB-13), feasibility (SB-14), sourcing/manufacturing
definitions (SB-15) and the activation readiness gate (SB-19).

Acceptance: the contract crate's `engineering` tests, the service's `tests/engineering.rs` domain
tests and the engineering route tests in `tests/api.rs`.

## Rules that apply to every value

The version 1 rules still apply: business amounts and volumes are decimal strings; a missing value
is `null` and an explicit zero is `"0"`, never interchangeable; unknown JSON fields are tolerated; a
stored schema version this build does not read is refused, never coerced.

Additional rules for this contract:

- Every write names a `request_id` and an `expected_version`. The service keys the request by
  request id plus a canonical payload hash; an identical retry returns the recorded outcome before
  the version check, and a changed payload under the same request id is a conflict.
- Every accepted review write appends one immutable engineering record version. Earlier review
  versions stay readable through `GET /v2/projects/{project_id}/engineering/versions/{version}`.
  The original STEP bytes in `sources` are never modified, and neither is the project record.
- The imported geometry and the customer hierarchy are never rewritten by a review. Associations,
  measurements and confirmed labels are stored separately from the import.
- Missing values stay missing: a review never writes a zero volume for an unavailable one, and
  nothing in this contract turns a proposal into a confirmed decision.

## Identities

| Value | Meaning | Stability |
|---|---|---|
| `project_id` | The project the review belongs to. | Business identity, UUID. |
| `product_revision_id` | The project product revision whose delivered geometry is reviewed. | Business identity from the version 2 project contract. |
| `part_version_id` | The shared project part version whose engineering metadata is reviewed. | Business identity from the version 2 project contract. |
| `component_id` | One reviewed component association inside one product revision. | Client-generated stable UUID, created at the host boundary. |
| `analysis_key` | `source_sha256`, `engine_build`, `extractor_version` of the exact analysis. | Immutable evidence. |
| `example_id` | One persisted training-example metadata record. | Deterministic UUIDv5 over the part version, component, confirmed label and prediction identity. |

`component_id`, `product_revision_id` and `part_version_id` are separate: a component is one
reviewed association inside one revision, while the same project part version can be referenced by
several components in the same or different revisions. Repeated occurrences of one definition
share the reviewed metadata of their part version and keep their own placements.

## Registered analysis

A review is only meaningful against the exact bytes and extractor that produced it, so the first
step records the analysis:

- `POST /v2/projects/{project_id}/engineering/analyses` stores one analysis manifest under its
  `AnalysisKey`.
- The product revision must exist in the project, and the manifest's `source_sha256` must equal the
  SHA-256 of the source its delivery package retained. The source hash is therefore preserved by
  construction: a review can never be attached to different bytes than the delivered package.
- A registration is immutable. The same request id is idempotent; a different manifest that claims
  the same source/engine/extractor identity for the same revision is a `409 conflict`. A changed
  extractor version is a new analysis identity, not a rewrite of the old one.
- Analysis registration does not change the review version; the registered analysis is evidence
  that grouping writes validate against and reads report about.

## Reviewed components and flat grouping

A `ReviewedComponent` is one engineer-confirmed association of source-local bodies to one component
of one product revision. Each body reference is the imported tree path and the body index inside its
definition, exactly as the analysis manifest spells them.

Validation the service performs on every grouping write:

- The component names an existing product revision and, when set, an existing project part version.
  The component's analysis must be registered for that revision.
- Every body must exist in that registered analysis. A body that is not in the manifest is invalid.
- All bodies of one component must belong to **one** definition. Several solids from a flattened
  definition may be confirmed as one component; the review never merges different definitions into
  one component and never invents a nested customer hierarchy.
- A body may belong to at most one reviewed component of the same revision and exact analysis. A
  write that would take a body owned by another component is refused; the owning component is
  rewritten instead, so a body is never silently duplicated.
- A component is bound to one exact analysis key. A changed source, engine or extractor needs a new
  component; an existing component cannot be silently rebound.
- A component cannot move to another product revision.

A grouping write replaces the component's body set. Submitting no geometry dissolves the component
and its bodies become unassociated again; a new component needs at least one body.

For flattened inputs (an analysis whose manifest reports the extractor's `unresolved_grouping`
guard) the review is complete only when every body of that analysis belongs to a reviewed
component. Bodies that remain unassociated are reported as `unresolved_grouping` issues and stay
uncosted: the service never treats one body as one component and never manufactures component
identities from a body count. For preserved customer trees the associations are reviewed definition
by definition; completeness is not imposed beyond the bodies the engineer associated.

A component with no confirmed shared part version is `unresolved_component_association` and stays
uncosted. Associating or clearing the part version is a separate recorded decision. That
association is always an explicit engineer decision: the service proves the project holds the part
version, and it never infers an association from similar geometry, a previous revision, or a body
count. Distinct customer revisions are never merged by a review.

## Reviewed part metadata

A `ReviewedPart` holds the shared engineering metadata of one project part version:

- `classification`: an optional `ClassificationPrediction` and an optional
  `ConfirmedClassification`. A prediction carries its label, model version and optional confidence
  in `0..=1`. **A prediction never confirms a label**: nothing in this contract sets a confirmed
  classification from a prediction or its confidence, so high model confidence cannot approve
  anything. Only an engineer's confirmation does.
- `volume_mm3`: the reviewed volume once one is recorded, else `null`. A manual completion carries
  `source: manual`, its explicit unit, a reason (required by the version 1 measurement rules) and
  stays `review: pending`; the completion action refuses a measurement that claims its own
  confirmation. The recorded value must name the exact analysis of at least one reviewed component
  that references this part version: the service refuses a volume whose analysis does not describe
  the component geometry, so a value from unrelated source bytes cannot be recorded or confirmed.
  A separately recorded bulk confirmation marks the volume reviewed. An open surface has no volume
  until that manual completion and review happen; it is never given a zero or a repaired value
  implicitly. Replacing a recorded value clears its review again.
- The reviewer of a confirmed classification or volume is the authenticated identity recorded by
  the service, never a payload field. A confirmed volume keeps that identity in
  `volume_confirmed_by`; a recorded but unconfirmed value has none, and the identity stays readable
  in every retained review version.

Occurrences and product revisions that reference the same part version share this record; their
placements stay in the immutable import and in the reviewed component associations.

## Bulk confirmation and training-example metadata

`POST /v2/projects/{project_id}/engineering/confirmations` records engineer decisions for one or
more part versions in one write:

- Each decision may confirm or correct a classification label, confirm the recorded volume, or both.
  At least one decision is required per part version and a part version appears at most once per
  write. This is how correct predictions are accepted together and exceptions corrected.
- A volume confirmation requires a recorded value; a missing volume cannot be confirmed.
- Confirming a label captures one `TrainingExampleMetadata` per reviewed component that references
  the part version: the exact analysis, the component's bodies, the prediction when one exists, the
  confirmed label and the reviewer. The record is self-contained; later grouping edits do not
  rewrite it, and the deterministic example id prevents duplicates from retries or repeated
  confirmations of the same label.

The persisted examples are metadata only. This contract claims no automatic training, no model
deployment and no retroactive reclassification: existing projects are never reclassified in the
background, and curated examples and held-out validation remain later, separately approved work.

## Issues

Issues are computed on read, never stored. A missing value is an issue, not a zero.

| Code | Meaning |
|---|---|
| `unresolved_component_association` | A reviewed component has no confirmed shared part version; it stays uncosted. |
| `unresolved_grouping` | A flattened analysis still has bodies no reviewed component associates; they stay uncosted. |
| `pending_classification` | The part version has no engineer-confirmed classification. |
| `missing_reviewed_volume` | No reviewed volume is recorded; an open surface needs manual completion. |
| `pending_measurement_review` | A volume is recorded but not confirmed by an engineer. |

## Dependency facts

Writes return facts. Facts name what changed; they never rewrite accepted results, and the consumers
below own their workflows.

| Fact | Meaning | Consumer |
|---|---|---|
| `component_grouped` | A component's reviewed body set was confirmed or replaced. | SB-11 desktop adapters, SB-13 |
| `component_dissolved` | A component was dissolved; its bodies are unassociated again. | SB-11 desktop adapters, SB-13 |
| `component_part_associated` | A component's shared part version association changed. | SB-14, SB-15 |
| `measurement_recorded` | A volume was recorded for review. | SB-14, SB-15 |
| `part_confirmed` | An engineer confirmed a classification and/or volume. | SB-14, SB-15, SB-19 |
| `training_examples_captured` | Confirmed-label metadata was persisted. | later model-curation work |

## HTTP routes

Base path `/v2`; a bearer identity with the Engineer permission set from the version 2 permissions
contract, and the same bounded request time as version 1. Errors use the version 1 `ApiErrorBody`
shape and codes. A stale `expected_version` is `409 conflict`; a malformed or refused payload,
including an unknown product revision, part version or analysis, is `422 invalid`; an unknown
project or review version is `404 not_found`. Registering an analysis answers `201`; review reads
and writes answer `200`.

| Method | Path | Request | Success | Body |
|---|---|---|---|---|
| `GET` | `/v2/projects/{project_id}/engineering` | — | `200` | `EngineeringRecordView` |
| `GET` | `/v2/projects/{project_id}/engineering/versions/{version}` | — | `200` | `EngineeringRecordView` |
| `POST` | `/v2/projects/{project_id}/engineering/analyses` | `RegisterAnalysis` | `201` | `AnalysisReceipt` |
| `POST` | `/v2/projects/{project_id}/engineering/components` | `GroupSolids` | `200` | `EngineeringWriteOutcome` |
| `POST` | `/v2/projects/{project_id}/engineering/component-parts` | `AssociateComponentPart` | `200` | `EngineeringWriteOutcome` |
| `POST` | `/v2/projects/{project_id}/engineering/measurements` | `CompleteMeasurement` | `200` | `EngineeringWriteOutcome` |
| `POST` | `/v2/projects/{project_id}/engineering/confirmations` | `ConfirmParts` | `200` | `EngineeringWriteOutcome` |

There is deliberately no route that confirms a classification from a prediction, publishes a model,
trains, activates a revision or recalculates a cost. Those belong to SB-13, SB-19 and SB-20.

## Persistence

Migration `services/solidbom-server/migrations/0005_engineering_review.sql` adds
`engineering_records` (project review identity and current version), `engineering_record_versions`
(append-only document JSON per version) and `engineering_analyses` (immutable manifest per project,
product revision and analysis identity). The shared `requests` table keeps idempotent request
receipts. The project record tables are unchanged.

## Compatibility

- Version 1 and version 2 routes, wire names and meanings are unchanged; this contract is additive.
- Adding an optional field is compatible. Changing a field meaning, a wire name, a unit, a route, a
  status, an error code, an idempotency rule, the stored document schema or a validation rule that
  previously accepted a write is breaking and needs a new contract version, the affected-consumer
  list and coordinator approval.
- Historical review record versions stay readable under the schema version that wrote them.
- Identity validation reads the current project record. Project identities are append-only, so a
  review decision stays valid as later deliveries, revisions and part versions are added.

## Handoff to later packages

- **SB-13:** writes classification predictions into `ReviewedPart.classification.prediction` and
  never a confirmed label; the review UI confirms through `confirm_parts`. Recognition coverage,
  held-out validation and model deployment are its own approval boundary.
- **SB-14:** consumes confirmed component geometry and reviewed measurements; a changed supporting
  analysis needs renewed review rather than a silently reused value.
- **SB-15:** consumes the component-to-part-version associations and confirmed labels; sourcing and
  routing metadata attaches to `part_version_id`, not to a component or an occurrence.
- **SB-19:** the readiness gate treats any open engineering issue, unresolved component association
  or unconfirmed measurement as blocking; this contract publishes no activation decision.
- **SB-20:** facts name what changed; adoption and recalculation remain its workflows.
