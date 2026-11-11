# Activation and accepted baselines contract

Version: **1** (additive to API version 2) · Status: **locked by SB-19** (coordinator
assignment of 2026-11-11; the lock commit is recorded in
[SB-19's execution record](../../execution/SB-19.md)).

This contract is the version 1 activation vocabulary of package SB-19, additive to API version 2.
The project record (SB-10) already stores and validates `draft`/`active`/`historical` product
revisions, but it publishes no activation decision. This contract publishes the readiness gate, the
coordinated shared-definition activation and the append-only accepted baselines that decision needs.
The version 1 and version 2 routes, types and stored documents stay exactly as published in the
[registry](../README.md); nothing here rewrites them.

Portable types: `crates/solidbom-contract/src/activation.rs` (re-exported at the crate root).
Producers: the service activation store and the version 2 activation routes (module
`services/solidbom-server/src/activation.rs`, migration `0010_accepted_baselines.sql`).
Consumers: the desktop readiness/activation surface (later increment), SB-20 (changed-baseline
facts) and SB-21 (exports of the selected accepted baseline). Acceptance: the contract crate's
`activation` tests, the service's `tests/activation.rs` domain tests and the route cases in
`tests/api.rs`.

## Rules that apply to every value

The version 1 and version 2 rules still apply: business amounts are decimal strings; a missing value
is `null` and an explicit zero is `"0"`, never interchangeable; unknown JSON fields are tolerated; a
newer stored schema version is refused, never coerced; money, rates and measurements are decimally
exact; a value received from another program is validated before it is stored.

Additional rules for this contract:

- Every activation names a `request_id` and an `expected_project_version`. The service keys the
  request by request id plus a canonical payload hash; an identical retry returns the recorded
  receipt before any version check, and a changed payload under the same request id is a `409
  conflict`. A readiness report is a pure read: it stores no row and takes no request id.
- A write whose `expected_project_version` is not the current project record version is a `409
  conflict` and writes nothing. So is an `expected_accepted_baseline_id` that is not the current
  accepted-baseline head.
- Every accepted activation appends, in one transaction, exactly one immutable project record
  version, one immutable SB-16 baseline calculation snapshot and one immutable accepted baseline
  row, or none of them.
- The service assigns `accepted_baseline_id` and the snapshot's `calculation_id`; clients keep the
  project, product, product-revision and part-version identities they already use.
- Sorted lists stay sorted: members, activated revisions, affected products, pins, facts and
  blockers have the canonical order stated per value.

## Identities and states

| Value | Meaning | Stability |
|---|---|---|
| project record version | One immutable appended project document (SB-10). Activation appends version `N+1` and names it in the accepted baseline. | Append-only integer; 1-based. |
| product revision state | `draft`, `active` or `historical`. Activation turns one draft `active` and the previously active revision `historical`. | Stored in the project document. |
| activation member | One `{product_id, product_revision_id}` a request accepts: either the product's current `draft` (the newest draft revision appended to the product; activate it) or its current `active` revision (re-pin it). | Request and receipt identity. |
| accepted baseline | One append-only record of an accepted project state. | Service-generated UUID, plus a per-project `sequence`. |
| accepted-baseline head | The accepted baseline with the largest `sequence`; `None` before the first activation. | `sequence` is strictly increasing from 1. |
| accepted definition | The shared-part definition pin carried by the accepted-baseline head for one part version. | Part version id is the key. |
| resulting active revision | The revision a product has after the request: the selected draft for an activated member, otherwise its current active revision. | Computed, never stored as a new identity. |
| accepted state scope | The quotation-included products and the shared part versions their resulting active structures use. | Computed from the resulting project version. |

A member is a revision the activation can accept now. The accepted revision is the product's current
`draft` (the activation activates it) or its current `active` revision (the activation re-pins the
accepted shared definitions without a revision state transition).

**The current draft is the newest draft revision appended to the product**, in the project
document's revision order. Older drafts are superseded: a revision that a later delivery replaced is
not the product's current draft even though it is still stored, and selecting it is
`revision_not_activatable` (`409`) on both the readiness and activation routes. The contract needs no
timestamp for this; the document order is the authority. A superseded draft can never be activated
later; a further delivery appends a new draft instead. A `historical` or foreign revision is refused
with the same code.

## Readiness

`ActivationReadinessRequest { project_version, selections }` names the exact project record version
to read (it must be the current one) and one current draft (the newest draft revision) or current
active revision per product.
The answer is an `ActivationReadinessReport`:

| Field | Meaning |
|---|---|
| `project_id`, `project_version` | The project and the exact project record version read. |
| `accepted_baseline_id` | The current accepted-baseline head, or `null` before the first activation. The client echoes it in the activation request. |
| `members` | One `MemberReadiness` per selection, sorted by product id: `product_id`, `product_revision_id`, `state` (`ready`/`blocked`) and its typed `blockers`. |
| `coordination` | The project-wide pending changed definitions from the accepted-baseline head's whole pin set and every required quotation-included product for this selection's resulting structures. A selected member can have required products outside the selection (see "Coordinated activation"). |

Issues are computed on read, never stored. A missing value is a blocker, never a zero and never a
pass. Readiness appends no project version, snapshot or accepted baseline.

### Readiness inputs

Readiness resolves one exact state: the current project record version, the current engineering,
manufacturing and feasibility record versions, the registered analyses, the published reference
data, the project costing and tooling settings, and the accepted-baseline head. It blocks when
information required for a valid cost of the selected revision is missing, when a feasibility finding
is unresolved or its exception needs review, when a classification or measurement is unconfirmed,
when a pending delivered-part match is unconfirmed, or when a required cost or tooling input is
missing. The same gate applies to every member, including a re-pinned active member.

A blocker is attributed to the member it blocks:

- a product-scoped blocker blocks that product;
- a part-scoped blocker blocks every member whose resulting structure uses that part version;
- a component- or revision-scoped blocker blocks the member whose resulting revision holds it;
- a blocker scoped to no product or part is a project setting and blocks every member.

The owners are the packages that own the issue, not display text:

| Owner | Source and scoping |
|---|---|
| `project` (SB-10) | The project record's computed issues: `missing_forecast` for the member product, and the revision state (`revision_not_activatable` on the readiness route is a refusal, not a report member). |
| `engineering` (SB-11) | The engineering record's computed issues for the member's resulting revision and its part versions: `pending_classification`, `missing_reviewed_volume`, `pending_measurement_review`, `unresolved_component_association`, and the pending delivered-part-match issues `missing_part_identity`, `missing_geometry_for_match`, `conflicting_geometry_match` (an unconfirmed match is review work before anything is shared). |
| `feasibility` (SB-14) | The feasibility record's readiness issues, from the SB-14 contract: `feasibility_not_assessed` (project setting, blocks every member), `assessment_stale` and `exception_review_required` (scoped by the assessed component's product revision), `open_finding_failed`, `open_finding_conflict`, `open_finding_unresolved` (scoped by the finding's product revision). A closed exception is history and blocks nothing; an active accepted exception resolves disposition. |
| `manufacturing` (SB-15) | The manufacturing record's computed issues for the member's part versions and product: missing sourcing/material/consumption/allowance/routing, missing or incomplete routing operations, pending consumption review, missing attached quantities and unit costs. A routing or line issue is scoped through its owner (a part version or the product root). |
| `costing` (SB-16) | The project-wide baseline evaluation's issues. A product-scoped issue blocks that product only; a part-scoped issue blocks every member whose resulting structure uses that part version; a project-scope issue (missing volume policy, missing app cost configuration, missing or invalid exchange rate, invalid purchase tier, project-scope arithmetic overflow) blocks every member. Another product's product-scoped issue does not block a member, and `no_quoted_products` does not block a quotation-excluded member. |
| `tooling` (SB-17) | The tooling evaluation of the project's tool settings against the same accepted state, with the project default treatment and lifetime-demand recovery basis: `missing_treatment` (project setting when tool entries exist), `missing_product_demand` (product-scoped), `zero_combined_demand`, `missing_recovery_quantity`/`invalid_recovery_quantity` (tool-scoped, blocked by the tool's owner), missing or invalid exchange rate (project setting), and invalid or overflowing tool values. A manual recovery quantity is a scenario assumption, so readiness evaluates the lifetime basis and never blocks on an unentered manual quantity. |

Not readiness inputs, deliberately: a negotiated selling price, any saved commercial scenario, a
scenario's tooling treatment/recovery quantities, EBIT targets, exports, and shared-data adoption.
Those belong to SB-18, SB-20 and SB-21. A fully priced, feasible part with an unconfirmed
classification is blocked; a missing negotiated price is not.

### Readiness is a point-in-time report

The required set depends on the requested selection's resulting structures: a selected draft can stop
using a changed part version, and a required product that is not selected keeps its current active
revision for the computation. One readiness read is therefore a point-in-time answer, not a proof
that the group has converged. The activation route recomputes the pending changes, the required set
and the gate from the final request and refuses with the updated omitted set. A client refreshes the
readiness report after changing its selection; tests must not assume one call stabilises the
membership.

## Coordinated activation

### What a shared definition is

A project part version (customer part number plus customer revision) carries one accepted shared
definition with four aspects, matching the confirmed coordinated-activation rule in
APP_SPECIFICATION section 6:

| Aspect | Accepted value | Source |
|---|---|---|
| `sourcing` | The explicit `purchased`/`manufactured` choice, or `null` when undefined. | SB-15 `SourcedPart.sourcing` |
| `material` | The selected material version, the net consumption with its explicit unit, and the material allowance; `null` when no material record. | SB-15 `SourcedPart.material` |
| `dimensions` | The reviewed volume the cost basis uses, in mm³; `null` when no reviewed volume. | SB-11 `ReviewedPart.volume_mm3.value` |
| `routing` | The part-owned routing's operations in sequence order, each with its cost center, cycle seconds/basis/cavities, setup hours and tooling reference. | SB-15 `AppliedRouting` for the part version |

The comparison is semantic, not a record-version comparison and not a byte comparison of stored
documents. Excluded from the definition: provenance and review fields (source, reviewer, reason,
analysis key, timestamps), routing names/descriptions/operation ids/routing id, pinned
production/setup rate versions, purchase price tables, exchange rates, attached non-CAD cost lines
and every other costing setting. Those are cost inputs and shared-data adoption, not shared-part
definitions: re-saving an identical definition, confirming an unchanged value, adopting a rate or
editing an attached cost line never forces coordinated activation. They are recorded in the accepted
state's immutable costing snapshot and are SB-20's change/adoption surface.

The portable helpers in `activation.rs` are the one definition of this mapping:
`accepted_shared_definition`, `shared_material_definition`, `shared_dimension_definition`,
`shared_routing_definition`, `shared_definition_changes` and `pending_shared_definition_changes`.
The service must use them instead of re-deriving the shape.

### The accepted definition set

The accepted state is the project's **full** accepted definition set: one pin for every shared part
version used by the resulting active structures of all quotation-included products, computed from
the resulting project record version and the exact engineering/manufacturing records read. The pins
are taken from the same resolved structure the pinned SB-16 snapshot costs, so the accepted
definition set and the accepted cost evidence cannot disagree. Each accepted baseline carries the
whole set, not only the parts its members touched.

That full set is also the **changed-candidate set**: a pending change is detected against the head
pin set, so it is project-wide and cannot be hidden by an unrelated activation (see "The grouping
rule").

Carrying the whole set is required: if a baseline dropped a pin for a part an activation did not
touch, a later change to that part would find no accepted definition to differ from and one product
could be activated alone while another kept the old definition. The full-set rule also defines which
part versions are coordinate-able at all: a part version that no quotation-included product has ever
used in its accepted state has no accepted definition, so a first use is "newly accepted" and is not
by itself a coordinated change. Distinct customer revisions are distinct part versions with separate
pins, so they are independent groups by the confirmed O05 rule. The contract cannot prove that a
customer revision is genuine; establishing identity is SB-11 review (O05), and an activation can
never mint a revision, so a fabricated revision cannot be introduced through this route.

The accepted set is pinned for the costed structure. A descendant under a purchased parent is not a
costed occurrence (the SB-16 purchased-branch cutoff), so it is outside the accepted set while it is
suppressed; a part version that appears only in a draft's delivered list but in no costed structure
is not part of the accepted state.

### The grouping rule

For one activation request:

1. Compute every product's resulting revision: the selected revision for a member, otherwise the
   product's current active revision. The resulting structures are what decides which products use a
   part version; a product with no active revision and no selection has no resulting structure.
2. The changed candidates are the accepted-baseline head's **whole pin set**, never only the part
   versions the request's members use. Compare every head pin with its current definition (helpers
   above); a pin with any differing aspect is a **pending changed shared definition** with the
   differing aspects. A part version with no head pin is newly used and is not a pending change.
3. For every pending change, every quotation-included product whose resulting active structure costs
   that part version is an **affected product**. The required product set is the sorted union over
   the pending changes. A pending change that no resulting structure uses has no affected product:
   it needs no group and is not listed as a changed definition (nothing to coordinate), and the next
   activation simply refreshes its pin. A product that no resulting structure uses is not required.
4. The request is complete only when it names every required product. An omitted required product
   refuses the activation with `coordinated_activation_required` (`409`), naming every required
   product the request omits. A required product joins by selecting its current draft (activate it)
   or its current active revision (re-pin it).
5. While a pending change exists, **every** activation must include its group, even when the
   request's own members have nothing to do with the changed part version. Every accepted baseline
   refreshes the full accepted set from the current records, so accepting any baseline would
   otherwise silently accept the pending change. This deliberately blocks unrelated activations
   until the project's next activation includes the group or the records are reverted to the accepted
   definition. A selected product without a shared-definition impact is still an independent,
   product-only activation and is accepted atomically together with the group.

Accepting the pending change recomputes the full accepted set from the current records, so the new
accepted baseline pins the changed definition and the change stops being pending. The pins and the
pinned snapshot come from one evaluation of the same resulting state, so they cannot disagree.

The quotation-excluded products are deliberately outside the coordinated group. They contribute no
quoted demand, cost or accepted result, and exclusion leaves accepted snapshots unchanged (the SB-10
and SB-16 contracts); reinstatement carries any still-open review/update requirements to the later
packages that own them. A part version used only by excluded products therefore never forces
coordination while they are excluded.

The confirmed product-only independence therefore holds in the steady state, when the accepted set has
no pending change. A pending change is project-wide until its group accepts it or the records are
reverted; that is the deliberate cost of never accepting a definition silently.

## Activation request

`ActivateRevisions { request_id, expected_project_version, expected_accepted_baseline_id,
product_revisions }`:

| Field | Rule |
|---|---|
| `request_id` | Non-blank, at most 128 characters, no control characters; the idempotency key. |
| `expected_project_version` | The current project record version the caller read; `409 stale_project_version` otherwise. |
| `expected_accepted_baseline_id` | The current accepted-baseline head the caller read. `null` or absent means "the project has no accepted baseline yet"; the current head must be `null` too, else `409 stale_accepted_baseline`. |
| `product_revisions` | One current draft (the product's newest draft revision; older drafts are superseded) or current active revision per product, sorted or not, unique by product, at most 1,000; a selection that is not an activatable revision is `409 revision_not_activatable`. The request must also name every required product of every pending shared-definition change, even when the member does not use the changed part version. |

Refusal precedence is fixed so a failure is deterministic: request validation and unknown
identities (`422 invalid_request`, including an unknown product, an unknown revision id, an empty or
duplicate selection, a malformed request id, a project-path mismatch), then `409` in this order:
stale project version, stale accepted baseline, revision not activatable, coordinated activation
required, member not ready. A refused activation writes nothing.

### Transaction invariants

The service runs one database transaction:

1. Take the request-id advisory lock and return a recorded receipt for an identical retry, before
   any version check. A changed payload under the same request id is a conflict.
2. Lock the project record row so no other project write interleaves, and the engineering,
   manufacturing and feasibility record rows read so their versions cannot be superseded inside the
   transaction.
3. Compare the expected project version and expected accepted-baseline head; a mismatch refuses.
4. Validate every selection against the stored document (`422`), then check each selected revision
   is the product's current draft (its newest draft revision) or current active revision (`409`).
5. Compute the pending changed definitions from the accepted-baseline head's whole pin set and the
   current records, and the required product set from the final request's resulting structures; a
   missing required product refuses with the computed omitted set.
6. Apply the member actions to the in-memory document: an activated draft becomes `active`, its
   previous active revision becomes `historical` and `active_revision_id` points at it; a re-pinned
   member keeps its active revision and state. Validate the resulting project document.
7. Resolve the SB-16 baseline inputs ONCE from the resulting project state (all products, the full
   resulting revision selection, the exact engineering/manufacturing records and reference data
   read). This one resolution yields the costing and tooling evaluation, the accepted definition
   pins and the input hash/calculator version; the service must not evaluate the state twice.
8. Run the authoritative readiness gate on the final member set with that resolution (plus the owner
   issue lists read under the same locks). Any member with a blocker refuses with `member_not_ready`,
   naming the blockers; this recheck is authoritative even when an earlier readiness report was
   ready.
9. Append project record version `N+1` and advance the project head. A pure re-pin activation still
   appends version `N+1` with unchanged content, so every accepted baseline names a distinct project
   record version and the two version heads stay single-valued.
10. Insert one immutable `baseline_calculations` snapshot from that same resolution, recording
    project version `N+1`, its own `calculation_id`, the input hash and the calculator version. It is
    written by the same resolution code as the SB-16 route and records no second request receipt.
11. Insert one immutable accepted-baseline row: the new sequence, project version `N+1`, the pinned
    record versions, the snapshot id and its input hash/calculator version/state, the members, the
    resulting active revisions and the full accepted definition set taken from the same resolution.
12. Record the activation receipt as the request's recorded reply and commit.

Any failure at any step rolls the whole transaction back: no project version, no snapshot, no
accepted baseline and no receipt are visible, so the previous active revisions, accepted history and
project version are exactly as before. The locks taken in step 2 cover the whole transaction, so the
readiness gate, the pins and the snapshot are one evaluation of one state.

### Members and dependency facts

The receipt carries `members`: one `ActivationMember` per accepted product, sorted by product id,
with `action` (`activated` or `repinned`) and the `previous_active_revision_id` that became
historical for an activated member. An activated member without a previous active revision is the
product's first activation.

Facts for SB-20, in order with the baseline fact first:

| Fact | Meaning | Consumer |
|---|---|---|
| `baseline_accepted` | A new accepted baseline heads the project; names the id, the previous head and the project version. | SB-20 |
| `revision_activated` | One draft became the active revision; names the previous active revision. | SB-20 |
| `revision_repinned` | One active revision was re-accepted without a state transition. | SB-20 |
| `shared_definition_changed` | One part version was accepted with the aspects that differ from the previous accepted definition. | SB-20 |

The facts are one canonical list: exactly one `baseline_accepted` fact first, then the revision facts
sorted by product id (exactly one per member, in the receipt's member order), then the
`shared_definition_changed` facts sorted by part version id, with no duplicate baseline, product or
part version. The receipt validation enforces that order and that the baseline fact agrees with the
receipt's accepted baseline id and project version, and that the revision facts name exactly the
receipt's members.

Facts name what changed; they never rewrite an accepted result.

## Receipt and accepted baseline

`ActivationReceipt` carries: `request_id`, `project_id`, `pinned` (the new project version and the
exact engineering/manufacturing/feasibility versions read), `accepted_baseline_id`,
`costing_calculation_id`, `costing_input_hash`, `calculator_version`, `costing_state`, `members` and
`facts`.

`AcceptedBaseline` is the append-only accepted record:

| Field | Meaning |
|---|---|
| `accepted_baseline_id` | The record identity. |
| `project_id`, `sequence` | The project and the strictly increasing per-project order, starting at 1. The largest sequence is the head. |
| `request_id` | The activation request that accepted it. |
| `pinned` | Project version `N+1` and the engineering/manufacturing/feasibility record versions read. |
| `costing_calculation_id` | The immutable SB-16 snapshot written in the same transaction. The snapshot names every record and reference version the resolved inputs read. |
| `costing_input_hash`, `calculator_version`, `costing_state` | The snapshot's canonical input hash, arithmetic version and resulting completeness state. |
| `members` | The accepted members sorted by product id, with their actions. |
| `active_revisions` | The resulting active revision of every product that has one, sorted by product id. |
| `shared_definitions` | The full accepted definition set (see above), sorted by part version id; empty only when the accepted state uses no shared part version. |
| `created_at` | The RFC 3339 UTC timestamp the service recorded. |

The accepted state is `complete` for the members the gate accepted. The pinned snapshot is
project-wide, so its `costing_state` can be `incomplete` because an unrelated product has a
pre-existing open input; the receipt exposes that honestly instead of hiding it. The activation
refuses only on a member blocker, never on another product's own issue.

An accepted baseline is immutable: it is never updated, reordered or deleted, and its sequence never
changes. The expected-baseline head is the only concurrency token; a project with no accepted
baseline has head `None`, the first accepted activation writes `sequence = 1`, and every later one
writes the next integer.

## Activation refusals

The version 1 `ApiErrorBody` shape is unchanged and the version 2 error-code set stays closed. The
stable refusal reason is the prefix of the client-safe message (`<code>: <detail>`); the structured
affected product set, blockers and readiness states are read from the readiness report, never from
the error body.

The `ApiErrorBody` applies to refusals the handlers produce. A request body that the shared JSON
extractor cannot deserialize is rejected **before** any handler runs: HTTP `400` with a plain-text
body, not an `ApiErrorBody`, exactly as on every other version 2 route. This contract does not change
the extractor or the version 1/2 transport rules.

| Code | Status | When | Message names |
|---|---|---|---|
| `invalid_request` | `422 invalid` | A request the handler refuses as malformed or unrepresentable: unknown product or revision id, empty or duplicate selection, invalid identity syntax. A non-deserializable body is the extractor's HTTP `400` plain-text rejection above, not this body. | The offending value or rule. |
| `stale_project_version` | `409 conflict` | The expected project record version is not current. | Expected and current version. |
| `stale_accepted_baseline` | `409 conflict` | The expected accepted-baseline head is not current (including expected `null` when a head exists). | Expected and current head, or `none`. |
| `revision_not_activatable` | `409 conflict` | A selection is neither the product's current draft (its newest draft revision) nor its current active revision: a historical revision, a superseded older draft or a foreign revision. | Product and revision. |
| `coordinated_activation_required` | `409 conflict` | A pin in the accepted-baseline head differs from its current definition and the request omits a required product of the pending change, including an unrelated activation that would otherwise refresh the pin silently. | Every required product id the request omits, sorted. |
| `member_not_ready` | `409 conflict` | A member has open readiness blockers at the authoritative recheck. | Product, revision and each blocker code with its field path (first eight, then a count). |
| permission | `403 forbidden` | The identity lacks the Engineer permission set. | No data. |
| unknown project | `404 not_found` | The project does not exist. | No data. |

## HTTP routes

Base path `/v2`; a bearer identity with the Engineer permission set from the version 2 permissions
contract, bounded requests exactly as version 1. Handler refusals use the version 1 `ApiErrorBody`
shape and codes as mapped above; a body the shared JSON extractor cannot deserialize is rejected
before the route with HTTP `400` plain text, as on every version 2 route.

| Method | Path | Request | Success | Body |
|---|---|---|---|---|
| `POST` | `/v2/projects/{project_id}/activation-readiness` | `ActivationReadinessRequest` | `200` | `ActivationReadinessReport` |
| `POST` | `/v2/projects/{project_id}/activations` | `ActivateRevisions` | `201` | `ActivationReceipt` |
| `GET` | `/v2/projects/{project_id}/accepted-baselines` | — | `200` | `AcceptedBaselineList` |
| `GET` | `/v2/projects/{project_id}/accepted-baselines/{accepted_baseline_id}` | — | `200` | `AcceptedBaseline` |

The readiness route is a read-only calculation: it appends nothing, stores nothing and takes no
request id, like the tooling-recovery evaluation. It reads one transaction's consistent view of the
project version, the records it names and the accepted-baseline head, and reports the pending
changes project-wide. A stale `project_version` in the readiness request is
`409 stale_project_version`; a non-activatable selection is `409 revision_not_activatable`; an
unknown project or revision id is `404`/`422` as above.

The activation route returns `201` on both a fresh activation and an identical replay of a recorded
request id, exactly as the other append routes do. The list is newest first and bounded to the newest
100 accepted baselines. The historical read returns the immutable row by id even after later
activation, record edits or reference updates; an unknown baseline id is `404 not_found`.

## Persistence

Migration `services/solidbom-server/migrations/0010_accepted_baselines.sql` (reserved by the SB-19
assignment, next number after `0009_costing_baseline.sql`) adds one append-only table:

- `accepted_baselines` (`accepted_baseline_id` uuid primary key, `project_id` uuid not null
  referencing `project_records`, `sequence` bigint not null, `request_id` text not null,
  `project_version` bigint not null, `engineering_version` bigint not null,
  `manufacturing_version` bigint not null, `feasibility_version` bigint not null,
  `costing_calculation_id` uuid not null referencing `baseline_calculations`, `costing_input_hash`
  text not null, `calculator_version` text not null, `costing_state` text not null,
  `members` jsonb not null, `active_revisions` jsonb not null, `shared_definitions` jsonb not null,
  `created_at` timestamptz not null), with `unique (project_id, sequence)` and
  `unique (project_id, project_version)`.

The table is immutable: no route and no code path updates or deletes a row. The shared `requests`
table keeps the idempotent activation receipt. The SB-16 `baseline_calculations` table stores the
pinned snapshot written by the same transaction; its columns are unchanged. The project record,
engineering, manufacturing, feasibility, reference-data, costing-settings and tooling tables are
unchanged, and the stored project document schema stays version 1: accepted-baseline state lives in
its own append-only table, so the project document never grows a hidden history, never changes its
schema version and never carries an accepted result a later build could rewrite.

## Validation the input refuses

`ActivationReadinessRequest::validate`, `ActivateRevisions::validate` and the record validators
refuse: a zero version or sequence; a nil or duplicate member product or revision; an empty or
oversized member list; a request id that is blank, too long or contains a control character; a
`costing_input_hash` that is not 64 lower-case hexadecimal characters; an unknown enum wire name; an
unsorted or duplicated member, active-revision, affected-product or pin list; a blocker code that is
not a stable code; a blank or over-long field path, identity, message, calculator version or tooling
reference; a negative amount or cycle/setup value; a non-volume material unit; a cycle basis whose
cavity count it cannot use; a routing with no operations or a non-increasing sequence; a purchased
part pin carrying manufacturing material; a member blocker naming another product or revision; a
ready member carrying a blocker or a blocked member carrying none; a coordination report whose
required set is not exactly the union of the affected products; a receipt whose facts are not exactly
one baseline fact first, then revision facts naming its members in product id order, then changed
definitions sorted by part version id and without duplicates or disagreement with the receipt's own
baseline id and project version; a pending-change list that is unsorted or names no differing aspect;
and an `invalid_request` refusal whose reason is blank.

A selection that is stored but is not the product's current draft or current active revision passes
the structural validation and is refused by the service as `revision_not_activatable` (`409`); the
document order decides whether a draft is current.

## Compatibility

- This contract is additive to API version 2. The version 1 and version 2 routes, wire names,
  meanings, statuses and the stored project/engineering/manufacturing/feasibility documents are
  unchanged; `SCHEMA_VERSION` stays 1 and `API_VERSION` stays 2.
- No stored project document changes. Accepted-baseline state is an append-only event log with
  project-wide pins and snapshot ids; storing it in the project document would make every project
  version carry derived state, require a document schema version bump and allow a later write to
  rewrite accepted history. It therefore lives in its own table.
- Adding an optional field or a route is compatible. Changing a field meaning, a wire name, a unit,
  a route, a status, an error code or idempotency rule, the meaning of "shared definition", the
  full-set pin rule, the pending-change rule (accepted head pin candidates and the blocking of
  unrelated activations), the atomicity guarantee or a validation rule that previously accepted a
  write is breaking and needs a new contract version, the affected-consumer list and coordinator
  approval.
- Historical project record versions, SB-16 snapshots, feasibility/manufacturing/engineering record
  versions and accepted baselines stay readable under the version that wrote them. An accepted
  baseline is never reinterpreted against newer definitions; it names the exact versions it read.

## Documented limitations and decisions

These decisions preserve a confirmed rule where the sources are silent, or record a deliberate
technical choice. Each is a limitation, not a silently promoted proposal.

- **Four coordinated aspects only.** The confirmed rule names sourcing, material, dimensions and
  manufacturing routing; this contract makes exactly those four the accepted definition. Attached
  non-CAD cost lines, cost-center rate versions, purchase price tables, exchange rates, app cost
  configuration and material prices are cost inputs, not shared-part definitions: a change to them
  recalculates or is adopted through SB-16/SB-20 and never forces coordinated activation. A source
  list that later includes an attached line or a price as a coordinated definition is a contract
  change, not an interpretation.
- **Dimensions are the reviewed volume.** The shared dimension is the reviewed part volume the cost
  basis consumes. Tube measurements are SB-14 evidence and re-open feasibility findings through their
  own contract; they do not become a second coordination trigger.
- **Quotation-excluded products are outside the group.** An excluded product contributes no quoted
  demand, cost or accepted result and exclusion leaves accepted snapshots unchanged; reinstatement
  carries any outstanding review/update requirements to the later packages. This is the deliberate
  reading of "affected active products" as the accepted costing scope.
- **No new draft is required to join.** A required product joins by its current draft (activate) or
  its current active revision (re-pin, no revision state transition). This keeps
  "complete coordinated activation changes both or neither" possible when an affected product has no
  new delivery, without inventing a revision.
- **Superseded drafts never activate.** The current draft is the newest draft revision appended to
  the product in the project document. An older draft is `409 revision_not_activatable` on readiness
  and activation; the contract offers no reactivation path for it, and a further delivery appends a
  new draft instead. No timestamp is used or needed.
- **Pure re-pin activations still append a project version.** Every accepted baseline names a
  distinct project record version, so the project-version head and the accepted-baseline head stay
  single-valued concurrency tokens. A pure re-pin appends an unchanged document.
- **A pending shared-definition change blocks unrelated activations (deliberate).** The changed
  candidates are the accepted-baseline head's whole pin set. While any pin differs from its current
  definition, every activation must include every affected quotation-included product, even when the
  request's own members do not use the part version: any accepted baseline refreshes the full set
  from the current records, so accepting one without the group would silently accept the change
  (that bypass was the reason this rule was tightened after verification). The pending change is
  accepted only by an activation that includes its group, or it stops being pending when the records
  are reverted to the accepted definition. SB-20 reports it meanwhile. Product-only independence
  holds in the steady state, when no pin is pending.
- **First accepted baseline on a project without one.** Before the first activation there is no
  accepted definition to differ from, so the first activation has no coordination trigger and
  records the full accepted set. This also covers a project whose active revisions predate this
  contract (the internal SB-10 transition): its accepted state is undefined until it adopts a first
  accepted baseline. Historical claims before that are not retroactively certified.
- **Genuine versus fabricated customer revisions.** Distinct customer part revisions are distinct
  part versions and independent groups by the confirmed O05 rule. This contract cannot prove that a
  customer revision is genuine; identity review is SB-11's, and an activation can never mint a
  revision, so coordination cannot be bypassed through the activation route.
- **Purchased branches.** The accepted set and group membership follow the costed structure; a
  descendant under a purchased parent is suppressed by the SB-16 cutoff and is not a costed
  occurrence. When the branch becomes manufactured, the part version enters the accepted set and
  coordination applies from then on.
- **Feasibility assessment coverage.** The SB-14 contract treats a missing assessment as a project
  readiness issue and assessment completeness as a client obligation; this contract keeps that
  consequence, so a project with no recorded feasibility assessment blocks every member until one is
  recorded. The empty issue list is never proof that every tube was assessed.
- **Costing snapshot is project-wide.** Readiness and the accepted baseline evaluate the whole
  project baseline because shared demand is project-wide. Another product's own open issue does not
  block a member but is visible in the pinned snapshot's state and issues. A build that made
  activation depend on the whole snapshot being complete would block independent product activations
  and contradict the confirmed independence rule. This is distinct from a pending shared-definition
  change, which deliberately does block activations until its group accepts it.
- **Tooling readiness is scenario-free.** Readiness evaluates the project default treatment and the
  lifetime-demand recovery basis. A manual recovery quantity, a scenario treatment, a selling price
  and a completed scenario are never prerequisites. `missing_treatment` blocks only when tool entries
  exist.
- **No desktop surface in this increment.** The typed engine commands, controller state and the
  readiness/activation UI are a later increment of SB-19; they depend on the still-open SB-10 desktop
  project-record wiring. This contract and its service surface come first.

## Handoff to later packages

- **SB-20 (dependency updates and adoption):** consumes `ActivationFact`, the accepted-baseline
  history and head, the full accepted definition set and the pinned record versions. It flags only
  dependent records, treats acknowledgement as not adoption and never rewrites an accepted baseline
  or snapshot. A pending changed definition is visible by comparing current records with the accepted
  pins it reads, and it blocks every activation until its group accepts it.
- **SB-21 (internal exports):** exports a selected accepted baseline by reading `AcceptedBaseline`
  and its `costing_calculation_id` snapshot, not by recomputing today's state; draft/incomplete/stale
  labels stay its own contract.
- **SB-18 (commercial scenarios):** never a readiness input. Scenarios reference an accepted baseline
  and never change activation state.
- **Desktop increment:** the readiness/activation surface reads `ActivationReadinessReport`, echoes
  `accepted_baseline_id` and `project_version` into `ActivateRevisions`, and shows a
  `coordinated_activation_required` refusal by adding the named required products as re-pin members.
  It is a later increment after the SB-10 desktop wiring lands.
- **Independent verification:** re-run the contract crate's `activation` tests and the service
  `tests/activation.rs` domain and route cases against the committed tree; verify the transaction
  atomicity and rollback cases, the expected-version and idempotency rules, the full-set pin rule,
  the pending-change rule (including the unrelated-activation bypass) and the required-product
  computation with an independent fixture before accepting the package.
