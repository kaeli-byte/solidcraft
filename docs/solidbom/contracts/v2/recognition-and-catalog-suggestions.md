# Recognition and catalog suggestions contract

Version: **1** of this contract, additive to the published version 2 HTTP API · Status:
**proposed/published by SB-13** under the owner's 2026-11-09 implementation instruction; it is
**not coordinator-locked** and the assignment gate in
[SB-13](../../packages/SB-13.md) stays open.

This contract extends the published contract additively. It adds the portable shape of recognition
suggestions and catalog match proposals beside the reviewed engineering record; the version 1 and
version 2 routes, wire names and meanings stay exactly as published, and no stored project document
changes meaning.

The coordinator-owned gate is still open, and this document deliberately does not close it:

- independent reference labels for evaluation are not yet acquired: the engine's evaluation harness
  consumes a supplied dataset and reports counts, and no dataset exists yet;
- recognition coverage and catalog-matching targets have not been agreed by the owner;
- the final contract lock has not been given by the coordinator.

This contract therefore claims **no accuracy percentage, no measured coverage and no deployment
result**. A suggestion count over a future independent dataset is evidence, not a model claim, and
nothing here trains, deploys, activates or silently reclassifies a project.

Portable types: `crates/solidbom-contract/src/recognition.rs` (re-exported at the crate root), plus
the additive fields in `engineering.rs` (`ClassificationPrediction`, `ReviewedPart`,
`PartConfirmation`, `CatalogConfirmation`, `EngineeringFact`), `types.rs`
(`CatalogAttribute`, `CatalogComponentVersion.attributes`) and `shared_data.rs`
(`CreateComponentVersion.attributes`). Producers: the service's engineering store
(`services/solidbom-server/src/engineering.rs`) and the suggestion route below; the engine's
analytic recognition and catalog matcher validate a proposal against the exact analysis before it is
recorded. Consumers: the reviewed-group selection adapters and review UI (SB-11 desktop increment),
SB-14, SB-15 and the SB-19 readiness gate. Acceptance: the contract crate's `recognition` and
`engineering` tests, the service's `tests/engineering.rs`/`tests/api.rs` and the engine's
`recognition` tests.

## Rules that apply to every value

The version 1 and version 2 rules still apply: business amounts are decimal strings; a missing value
is `null` and an explicit zero is `"0"`, never interchangeable; unknown JSON fields are tolerated; a
stored schema version this build does not read is refused, never coerced; every write names a
`request_id` and an `expected_version`, and an identical retry returns the recorded outcome before
the version check.

Additional rules for this contract:

- **A suggestion is evidence only.** Recording a prediction, a catalog proposal or a confidence
  value never sets a confirmed label, a confirmed catalog reference or a confirmed price. Only the
  engineer confirmation actions in the SB-11 contract do that.
- **Nothing confirmed is inferred.** A missing value stays missing. A proposal with confidence `1`
  is still a proposal; a part with only proposals has no authoritative catalog inputs.
- **Bounded hostile input.** Every list and text has a contract bound; a value outside it is
  `ContractError::Invalid`, never a crash and never a silent truncation.
- **English only, stable codes only.** Suggestion labels are stable category codes; evidence and
  issue codes are stable contract names, not display text.
- Evidence shown for review is stored with the proposal: a recorded suggestion carries at least one
  evidence item, and a classification proposal also carries its prediction version and the exact
  analysis it describes.

## Categories

A category is an **open, stable code** matching `^[a-z][a-z0-9_]{0,63}$`. The six categories the
prototype confirmed are listed by `INITIAL_CATEGORIES`: `tube`, `connector`, `fitting`, `clip`,
`wear_protection`, `thermal_protection`. Adding a category such as `bracket` or `custom_bracket` is
a well-formed code and needs **no schema, identity or wire change**; `CategoryCode::is_initial`
only distinguishes the confirmed initial set from an extension.

An invalid code (empty, upper-case, starting with a digit or underscore, containing spaces, `-` or
`.`, or longer than 64 characters) is refused when it is created and when it is deserialized, so a
malformed code cannot enter the wire contract. The JSON form of a category code is the plain string.

## Suggestion types

| Type | Meaning |
|---|---|
| `CategoryCode` | One open stable category code; the confirmed label itself is still an engineer decision. |
| `SuggestionEvidence` | One stable evidence code with a bounded English explanation. Evidence never approves anything. |
| `PredictionProvenance` | The exact `analysis_key`, the model/rule version, the prediction version and the optional service-recorded timestamp of a proposal. |
| `ClassificationSuggestion` | One `part_version_id` with the proposed label (`ClassificationPrediction`) recorded for review. |
| `CatalogMatchSuggestion` | One matcher proposal naming `part_version_id`, `component_id`, `component_version_id`, model/prediction version, optional confidence and at least one evidence item. |
| `CatalogMatchCandidate` | The persisted proposal: the same identity plus the catalog version, name, specification, price, unit and currency frozen at record time, and the service timestamp. |
| `ConfirmedCatalogReference` | The engineer-confirmed catalog component version with its values and the authenticated reviewer. |
| `ProjectCatalogOverride` | The engineer's project replacement of the confirmed catalog values, with its required reason and the authenticated reviewer, stored beside (not instead of) the confirmed reference. |
| `CatalogRecord` | One part version's suggestions, optional confirmed reference and optional project override. |
| `EffectiveCatalogInputs` | The authoritative catalog inputs, and whether they came from the project override. |
| `RecordSuggestions` | One suggestion write: `request_id`, `expected_version` and the classification and catalog proposals. |

`RecordSuggestions::validate` refuses an empty write, a part version named twice among
classification proposals, a catalog component version named twice for one part version, more than
`MAX_SUGGESTIONS_PER_WRITE` proposals, more than `MAX_CATALOG_SUGGESTIONS_PER_PART` catalog
proposals for one part version, more than `MAX_SUGGESTION_EVIDENCE` evidence items per suggestion,
an out-of-range confidence, a classification proposal without `prediction_version`, without
`analysis_key` or without evidence, and a malformed `request_id`.

### The unconfirmed-suggestion guard

`CatalogRecord::effective_inputs()` returns `None` while `confirmed` is `None`. A candidate is
returned by nothing else, so **an unconfirmed suggestion can never contribute a catalog price, a
specification, a unit or a currency to costing**, whatever its confidence. The value is `Some` only
after an engineer confirms a catalog component version, and the confirmed values are the
authoritative ones.

### Confirmed reference and project override retention

A confirmed reference retains the frozen catalog `component_version_id`, `version`, `name`,
`specification`, `price`, `price_unit` and `currency`. When a project override is recorded,
`effective_inputs()` returns the override values with `override_applied: true`, while the
`confirmed` reference stays in the record with its original catalog version and value. The two are
never merged, so the catalog value an engineer confirmed remains readable beside the project's
replacement. A `ProjectCatalogOverride` is only valid together with a confirmed reference; the
contract validator refuses an override without one. `price: null` stays missing and is never turned
into a zero.

## Catalog matching attributes

`CatalogComponentVersion` (published reference data) and `CreateComponentVersion` (an
administrative publish request) carry a defaulted `attributes` array of `CatalogAttribute`
`{key, value}` objects. Each key is a stable code matching `^[a-z][a-z0-9_]{0,63}$`; each value is
trimmed non-empty English text of at most 255 characters without control characters; at most 32
attributes are declared per version and a key is declared once. The fields are additive with
`#[serde(default)]`, so a legacy record or an old writer keeps working with an empty list.

The declared attributes are what the engine's conservative matcher compares against reviewed part
attributes. A component with no declared attributes **never matches** (it is not a wildcard), and a
suggestion never confirms a match or merges two part identities. Attribute matching is text-only in
this slice; it invents no numeric tolerance.

## Engineering record additions

The reviewed engineering record gains `ReviewedPart.catalog: CatalogRecord`, defaulted so a stored
review from before SB-13 stays readable. `ClassificationPrediction` gains the defaulted
`prediction_version`, `analysis_key`, `evidence` and `suggested_at` fields; an existing stored
prediction without them stays readable and valid. `PartConfirmation` gains a defaulted
`catalog: Option<CatalogConfirmation>` decision: the engineer confirms one exact
`component_version_id` and optionally records a `ProjectCatalogOverride`. The at-least-one-decision
rule now accepts a classification decision, a measurement confirmation **or** a catalog decision;
an override's reason and authenticated reviewer are required.

Two dependency facts are added:

| Fact | Meaning | Consumer |
|---|---|---|
| `suggestions_recorded` | Recognition recorded classification and/or catalog proposals; the counts are proposals. | review UI adapters, SB-14, SB-15 |
| `catalog_match_confirmed` | An engineer confirmed one catalog component version for one part version. | SB-14, SB-15, SB-19 |

The engineering document schema version stays 1: the new fields are additive and defaulted.

## HTTP routes

The engineering routes of the SB-11 contract are unchanged except for the additive
`PartConfirmation.catalog` field. One route is added; it uses the same `/v2` base, bearer identity
with the Engineer permission set, the version 1 `ApiErrorBody` shape and codes, the same bounded
request time, and the same idempotency and immutable-version rules. A stale `expected_version` is
`409 conflict`; a malformed payload, an unknown part version, an unknown analysis binding or an
unknown catalog component version is `422 invalid`; an unknown project is `404 not_found`.

| Method | Path | Request | Success | Body |
|---|---|---|---|---|
| `POST` | `/v2/projects/{project_id}/engineering/suggestions` | `RecordSuggestions` | `200` | `EngineeringWriteOutcome` |

Recording suggestions appends one immutable engineering record version. For a classification
proposal the service proves the named `analysis_key` is a registered analysis describing a reviewed
component that references that part version (the same binding rule as `CompleteMeasurement`) so a
prediction cannot be attached to unrelated source bytes, fills `suggested_at` when absent and
stores the prediction in `ReviewedPart.classification.prediction`. It **never sets or clears the
confirmed label**: an existing confirmed classification survives a new prediction. For a catalog
proposal the service resolves the exact catalog component version from the shared catalog and stores
a `CatalogMatchCandidate` with the authoritative catalog values and a deterministic UUIDv5
suggestion id; re-recording a part's proposals replaces that part's suggestion list (a fresh
matching run) while preserving the confirmed reference and any project override.

Confirmation stays on the existing SB-11 route: `POST /v2/projects/{project_id}/engineering/confirmations`
answers with `EngineeringWriteOutcome`, and an engineer confirms a catalog component version through
`PartConfirmation.catalog`. There is deliberately **no route that confirms a label or a catalog
value from a prediction or its confidence**, no training route and no deployment route.

## Persistence

The service stores the catalog record inside the immutable engineering record document (the SB-11
tables, no new document schema). Migration
`services/solidbom-server/migrations/0006_catalog_match_attributes.sql` adds the
`catalog_component_versions.attributes` `jsonb` column defaulted to `'[]'::jsonb` with an
array-type check. Review versions and the original source bytes stay immutable: recording a
suggestion or confirming a catalog reference never rewrites the import or the project record.

## Compatibility

- Version 1 and version 2 routes, wire names, statuses, error codes, units and meanings are
  unchanged; this contract is additive.
- Adding a defaulted field is compatible, and unknown JSON fields are tolerated by both directions.
  Old JSON without the new fields parses to the documented defaults: a reviewed part has an empty
  `CatalogRecord`, a part confirmation has no catalog decision, a prediction has no SB-13
  provenance, and a catalog component version has no declared attributes.
- Adding an open category code is compatible and needs no new contract version. Changing the
  category code shape, the meaning of a confirmed reference or of an override, the unconfirmed
  guard, a route, a status or an error code is breaking and needs a new contract version, the
  affected-consumer list and coordinator approval.
- Historical engineering record versions stay readable under the document schema version that wrote
  them.

## Handoff to later packages

- **SB-11** retains confirmation and training-example storage; SB-13 only writes proposals into the
  shared reviewed record and never a confirmed label.
- **SB-14/SB-15** consume a confirmed catalog reference (`effective_inputs()`) as authoritative and
  treat an unconfirmed candidate as evidence only.
- **SB-19** may treat a missing confirmed catalog reference as an open review issue; this contract
  publishes no readiness or activation decision.
- **SB-22** owns the category-extension boundary: the open code set here allows new categories
  without a schema change, while category meaning, display names and any catalogue taxonomy beyond
  stable codes stay later, separately approved work.
