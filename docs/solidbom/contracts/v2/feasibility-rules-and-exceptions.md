# Feasibility rules and justified exceptions contract

Version: **1** of this contract, additive to the published version 2 HTTP API · Status:
**published by SB-14** (owner instruction 2026-11-09: start implementation; the locking commit is
recorded in [SB-14's execution record](../../execution/SB-14.md)).

This contract extends version 2 additively. It adds Administrator-maintained feasibility rule tables,
recorded feasibility assessments and justified engineer exceptions beside the project record. The
version 1 routes, the version 2 project routes, the engineering review routes and their meanings stay
exactly as published. It does not change the stored project document or the stored engineering
review document.

Portable types: `crates/solidbom-contract/src/feasibility.rs` (re-exported at the crate root).
Producers: the service's feasibility store (`services/solidbom-server/src/feasibility.rs`) and the
feasibility routes below; the portable evaluator is pure, so the desktop may run the same function for
a preview. Consumers: the desktop feasibility and readiness surfaces, SB-19 (activation readiness) and
SB-20 (rule-change impact facts).

Acceptance: the contract crate's `feasibility` tests, the service's `tests/feasibility.rs` domain tests
and the feasibility route tests in `tests/api.rs`.

## Rules that apply to every value

The version 1 rules still apply: business amounts and volumes are decimal strings; a missing value is
`null` and an explicit zero is `"0"`, never interchangeable; unknown JSON fields are tolerated; a
stored schema version this build does not read is refused, never coerced.

Additional rules for this contract:

- Lengths are millimetres. Angles elsewhere in the contract remain degrees.
- Every feasibility write names a `request_id` and an `expected_version`. The service keys the request
  by request id plus a canonical payload hash; an identical retry returns the recorded outcome before
  the version check, and a changed payload under the same request id is a conflict.
- Every accepted write appends one immutable record version. Earlier record versions stay readable
  through `GET /v2/projects/{project_id}/feasibility/versions/{version}`.
- Feasibility is independent of the selected manufacturing cost center. No value in this contract
  names a cost center, a rate or a selected cost line.
- A missing measurement, a missing connector association or a missing limit is `Unresolved`, never a
  pass. No rule is interpolated, extrapolated or given a hidden priority.

## Identities

| Value | Meaning | Stability |
|---|---|---|
| `project_id` | The project the feasibility record belongs to. | Business identity, UUID. |
| `rule_table_id` | One logical feasibility rule table. | Business identity, UUID. |
| `rule_table_version_id` | One immutable published version of a rule table. | Business identity, UUID. |
| `rule_id` | One rule inside a rule table version. | Unique inside its version. |
| `component_id` | One reviewed component association from the engineering review contract. | Business identity from SB-11. |
| `product_revision_id` | The product revision the reviewed component belongs to. | Business identity from the version 2 project contract. |
| `analysis_key` | `source_sha256`, `engine_build`, `extractor_version` of the exact analysis the measurements came from. | Immutable evidence. |
| `finding_key` | Component, revision, rule kind and check identity of one computed finding. | Stable inside one assessment. |
| `exception_id` | One accepted justified exception. | Service-generated UUID. |

`component_id`, `product_revision_id` and the part version are separate. A reviewed component is one
association inside one revision; the assessment records the binding it verified, and the exception
keeps the binding it was accepted against.

## Rule tables

A feasibility rule table is an append-only chain of Administrator-published versions. Each version
carries a name, an optional table-level source reference and a bounded list of rules. A published
version is never edited, so a recorded finding always names the exact rules it used.

Every rule carries:

- a `kind`: `minimum_bend_radius`, `bend_spacing`, `tube_connector_compatibility` or
  `end_assembly_length`;
- a structured `applicability`, all conditions of which must be **proven** by the reviewed evidence
  for the rule to apply;
- one typed `limit` that must match the kind;
- a required `source_reference` naming the engineering source and revision the limit was transcribed
  from. An administrator-entered limit without its source is refused at publish time.

Applicability conditions are explicit evidence, never inference:

| Condition | Applies only when |
|---|---|
| `tube_kind` | The reviewed evidence names that tube kind. |
| `material_name` | The reviewed evidence names exactly that material (no case folding or normalising). |
| `supporting_spring_required` | The reviewed specification records a supporting spring. |
| `nominal_outer_diameter_mm` | The reviewed nominal-size association is exactly that value. |
| `nominal_wall_thickness_mm` | The reviewed nominal-size association is exactly that value. |
| `measured_outer_diameter_mm` | The measured outer diameter is inside the inclusive range. |
| `measured_wall_thickness_mm` | The measured wall thickness `(OD − ID) / 2` is inside the inclusive range. |
| `connector_component_id` | The end's confirmed connector association names that catalog component. It is refused on a radius or spacing rule. |

An inapplicable rule never makes a check pass. If no applicable rule supplies a usable limit, the check
is `Unresolved` with reason `no_applicable_limit`. An applicability condition that needs a measurement
which is missing cannot be proven, so that rule does not apply.

Limits are structured, not a formula language:

| Limit | Meaning |
|---|---|
| `MinimumRadiusMm { radius_mm }` | A minimum centerline bend radius in millimetres. |
| `MinimumRadiusMultipleOfOuterDiameter { multiple }` | The same minimum expressed as a multiple of the measured outer diameter, as the owner's smooth-tube source does. It resolves to millimetres once, from the measured outer diameter. |
| `MinimumLengthMm { length_mm }` | A minimum straight length in millimetres, for bend spacing or end assembly length. |
| `CompatibleTubeRange { outer_diameter_mm, inner_diameter_mm }` | The connector's compatible tube outer- and inner-diameter ranges; both are required. |

There is deliberately no priority field, override field, weight or interpolation. When two applicable
rules of one kind resolve to different limits, the check is a `Conflict` that lists all of them and
chooses none. An Administrator resolves the conflict by publishing a new rule-table version (for
example by narrowing an applicability range); no rule-priority system is introduced in MVP.

The owner's supplied radius tables are transcribed limits, not a starter rule set: this contract does
not seed any rule data into a deployment, and every transcribed rule enters through the publish route
with its source and applicability. The smooth-tube table applies only to standard PA12 with a
supporting spring; the corrugated table is keyed by nominal connection size and its automated
measurement is not part of the SB-12 extraction scope.

## Reviewed feasibility evidence

An assessment names every reviewed tube component of the project and the exact evidence for it:

- The reviewed attributes: tube kind, material name, supporting spring and nominal sizes, each
  optional because the review may not have proved it. The extractor never rounds a measured size into a
  nominal one.
- The exact measurements: outer and inner diameter, every bend's centerline radius, the straight
  centerline lengths between consecutive bend tangent points, and both end straight lengths. Each bend
  radius is a separate value and is checked independently. Bend spacing is **tangent to tangent**; the
  measurement is the SB-12 `tangent_straight_lengths_mm` value, never a chord, a nominal length or a
  display length. A straight tube reports its whole length at both ends and has no bend or spacing
  findings.
- The extraction algorithm version. A consumer comparing a saved result with a fresh analysis must
  require the exact analysis key and the extractor's own current version in addition to it.
- The reviewed ends, start first, and the confirmed connector association of each end: the catalog
  component's logical id and the exact published catalog version. A missing association is evidence
  that the requirement is unknown, so both connector checks stay `Unresolved`; they never pass from
  inference.
- A component with no exact measurements carries `measurements: null` and no ends. The assessment then
  records one `Unresolved` `WholeTube` finding per kept check kind with reason `missing_measurement`; it
  never invents a bend count.

The service verifies each component against the current engineering review: the reviewed component
must exist with the same product revision and the same exact `analysis_key`. The service records the
reviewed body set itself, never from the payload, and compares it later to detect changed supporting
geometry. A component that is not in the review, or evidence for a different analysis, is `422
invalid`.

Assessment completeness is a client obligation: the service cannot infer which reviewed components are
tubes, because the reviewed classification label is free text. The desktop increment must name every
reviewed tube component; an assessment that omits one leaves its checks unassessed. This limitation is
recorded for the desktop handoff and SB-19.

## Findings

Findings are computed, never stored as passes. Every finding names its `finding_key`, its state, the
measured values it compared, the applicable rules with their source references and limits, and the
agreed required limit.

| State | Meaning | Exception route |
|---|---|---|
| `pass` | An applicable limit exists and the measured value meets it. | Not available; a pass needs no exception. |
| `fail` | An applicable limit exists and the measured value does not meet it. | Available. |
| `conflict` | Two or more applicable rules resolve to different limits. | Available; an Administrator normally resolves the rules instead. |
| `unresolved` | A required measurement, connector association or applicable limit is missing. | Available only for reason `no_applicable_limit` (the documented missing-numerical-limit workflow). A missing measurement or connector association is review work and cannot be excepted. |

Every bend radius is one finding with its own `bend_index`; every tangent-to-tangent spacing is one
finding with its own `first_bend_index`; each end has a `connector_compatibility` finding and an
`end_assembly_length` finding. A conflict lists all overlapping rules; a fail lists the agreed rules.
The geometric state of a finding stays visible after an exception; the exception changes only the
disposition.

## Justified exceptions

An authorized Engineer may accept a justified exception for one recorded non-passing finding. The
exception records:

- the finding key and the exact finding snapshot accepted;
- the justification text (required, bounded);
- the authenticated reviewer identity, filled by the service from the bearer identity and never from
  the payload;
- the service timestamp;
- the assessment version, the exact rule-table version and the assessed component with its reviewed
  body set, exact analysis and part association.

Acceptance resolves the disposition, not the geometric result. A pass cannot be excepted, a missing
measurement or connector association cannot be excepted, and the same request id is idempotent. A later
accepted exception for the same finding key replaces the current decision; the earlier decision stays
readable in the earlier record version.

**Locked invalidation mechanics (this contract's explicit decision, not a silently promoted proposal):**

- An accepted exception becomes `review_required` when its supporting geometry changes: the current
  engineering review no longer has the same component, reviewed body set, exact analysis or part
  association. It also becomes `review_required` when the current assessment changes the evidence, the
  rule-table version or the computed finding for its key.
- `review_required` is visible with the reasons that caused it. The decision is never silently dropped
  and never silently kept accepted; a new accepted decision is required from an Engineer.
- There is no automatic re-acceptance, no time-based expiry and no automatic reopening beyond the
  comparison above. Exception audit fields are exactly the ones listed in this section; no extra
  "approval workflow" fields are introduced.

## Readiness issues

Issues are computed on read, never stored. A missing input or open finding is an issue, never a pass.
SB-19 treats a non-empty issue list as blocking activation.

| Code | Meaning |
|---|---|
| `feasibility_not_assessed` | No assessment is recorded for the project. |
| `assessment_stale` | A reviewed component binding changed after the assessment; assess the current review again. |
| `open_finding_failed` | A failing finding has no accepted exception. |
| `open_finding_conflict` | A finding is a conflict and has no accepted exception; an Administrator must resolve the rules. |
| `open_finding_unresolved` | A finding is unresolved and has no accepted exception. |
| `exception_review_required` | An accepted exception's supporting inputs changed; review it again. |

## Dependency facts

Writes return facts. Facts name what changed; they never rewrite an accepted result, and the consumers
below own their workflows.

| Fact | Meaning | Consumer |
|---|---|---|
| `assessment_recorded` | One assessment was recorded with its finding counts. | SB-14 desktop surface, SB-19 |
| `exception_accepted` | One justified exception was accepted; `replaced` names the decision it replaced. | SB-14 desktop surface, SB-19 |
| rule-table version chain and `changes` in the rules view | A rule version was published with its publisher and timestamp. | SB-20 (rule-change impact and adoption) |

## HTTP routes

Base path `/v1` (rules) and `/v2` (project records); a bearer identity is required. Reading rules and
all project routes need the Engineer permission set; publishing a rule version needs the Administrator
permission set from the version 2 permissions contract. Errors use the version 1 `ApiErrorBody` shape
and codes. A stale `expected_version` is `409 conflict`; a malformed or refused payload, including an
unknown rule version or a component that is not in the review, is `422 invalid`; an unknown project or
record version is `404 not_found`. Publishing answers `201`; assessment recording answers `201`; an
exception acceptance and a record read answer `200`.

| Method | Path | Permission | Request | Success | Body |
|---|---|---|---|---|---|
| `GET` | `/v1/feasibility-rules` | engineer | — | `200` | `FeasibilityRulesView` |
| `POST` | `/v1/shared/feasibility-rules/{rule_table_id}/versions` | administrator | `CreateFeasibilityRuleTableVersion` | `201` | `FeasibilityRuleTablePublished` |
| `GET` | `/v2/projects/{project_id}/feasibility` | engineer | — | `200` | `FeasibilityRecordView` |
| `GET` | `/v2/projects/{project_id}/feasibility/versions/{version}` | engineer | — | `200` | `FeasibilityRecordView` |
| `POST` | `/v2/projects/{project_id}/feasibility/assessments` | engineer | `RecordFeasibilityAssessment` | `201` | `FeasibilityAssessmentOutcome` |
| `POST` | `/v2/projects/{project_id}/feasibility/exceptions` | engineer | `AcceptFeasibilityException` | `200` | `FeasibilityWriteOutcome` |

There is deliberately no route that invents a limit, resolves a conflict by precedence, accepts an
exception without a reviewer, activates a revision or recalculates a cost. Those belong to this
contract's rules route, SB-19 and SB-20 respectively.

## Persistence

Migration `services/solidbom-server/migrations/0006_feasibility.sql` adds `feasibility_rule_tables`
(logical table and current version), `feasibility_rule_versions` (immutable published rules, source
reference and publisher), `feasibility_records` (project feasibility identity and current version) and
`feasibility_record_versions` (append-only document with the assessments and current exceptions). The
shared `requests` table keeps idempotent request receipts. The project, engineering and shared-data
tables are unchanged.

## Compatibility

- Version 1 and version 2 routes, wire names and meanings are unchanged; this contract is additive.
- Adding an optional field is compatible. Changing a field meaning, a wire name, a unit, a route, a
  status, an error code, an idempotency rule, a validation rule that previously accepted a write or the
  evaluator's result for the same evidence and rule version is breaking and needs a new contract
  version, the affected-consumer list and coordinator approval.
- Historical record versions and published rule versions stay readable under the schema version that
  wrote them. The stored document keeps the findings computed when it was recorded; a newer evaluator
  never rewrites them.

## Handoff to later packages

- **SB-12 / desktop increment:** map the exact `TubeExtraction` result into `FeasibilityComponentEvidence`
  (outer and inner diameter, every `bends[].centerline_radius_mm`, every `tangent_straight_lengths_mm`,
  the two `straight_end_lengths_mm`, and the confirmed end/connector associations), keep every radius
  independent and require the extractor's own current version. Unsupported extraction is submitted as
  `measurements: null`, never as zero lengths. Manual tube measurement completion and its review remain
  a separate supporting-input prerequisite (SB-11 owns measurement review): this increment accepts
  exact extracted evidence only.
- **SB-19:** the readiness gate consumes this contract's readiness issues. Any issue blocks activation
  unless an Engineer accepted an active justified exception; an accepted exception on an unresolved
  missing-limit check is the documented engine-data route. This contract publishes no activation
  decision.
- **SB-20:** a new rule-table version is the change fact: `FeasibilityRulesView.changes` names the
  previous version, the publisher and the timestamp. Adoption and project notification remain SB-20's
  workflow; this contract never adopts a new rule version into a project by itself.
- **SB-15 / sourcing:** material and nominal-size evidence used by rule applicability comes from the
  reviewed sourcing data; until it is reviewed, a material-conditioned rule is inapplicable and its
  check stays unresolved rather than assuming a material.
