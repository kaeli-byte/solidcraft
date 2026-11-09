# Sourcing and manufacturing definitions contract

Version: **1** of this contract, additive to the published version 2 HTTP API · Status:
**published by SB-15** (owner instruction 2026-11-09: start implementation; the locking commit is
recorded in [SB-15's execution record](../../execution/SB-15.md)).

This contract extends version 2 additively. It adds the explicit sourcing choice, the reusable
routing template records, the applied routings and their operations, the material consumption and
allowance of a made part, and the attached non-CAD cost lines of a project. The version 1 routes,
the version 2 project routes, the engineering review routes and their meanings stay exactly as
published. No stored project, delivery or engineering document changes shape.

Portable types: `crates/solidbom-contract/src/manufacturing.rs` (re-exported at the crate root),
plus `CycleBasis::PerCycle` in `types.rs`. Producers: the service's manufacturing store
(`services/solidbom-server/src/manufacturing.rs`) and the template/manufacturing routes below; the
desktop engine validates a draft before it writes. Consumers: cost evaluation (SB-16), activation
readiness (SB-19), dependency adoption (SB-20) and the later desktop manufacturing editor, which
this increment does not wire.

Acceptance: the contract crate's `manufacturing` tests, the service's
`tests/manufacturing.rs` domain tests and the route tests in `tests/api.rs`.

## Rules that apply to every value

The version 1 rules still apply: business amounts and volumes are decimal strings; a missing value
is `null` and an explicit zero is `"0"`, never interchangeable; unknown JSON fields are tolerated; a
stored schema version this build does not read is refused, never coerced.

Additional rules for this contract:

- Every project write names a `request_id` and an `expected_version`. The service keys the request
  by request id plus a canonical payload hash; an identical retry returns the recorded outcome
  before the version check, and a changed payload under the same request id is a conflict.
- Every accepted project write appends one immutable manufacturing record version. Earlier versions
  stay readable through `GET /v2/projects/{project_id}/manufacturing/versions/{version}`, so an
  accepted snapshot never changes.
- A published template version is immutable. Publishing the next version appends it; nothing edits
  an older version in place.
- Missing data stays missing and becomes a computed issue. Nothing in this contract turns a missing
  cycle time, cavity count, rate version, consumption or unit cost into zero or into an implied
  figure.
- The contract stores draft definitions. It publishes no activation, readiness, cost or
  recalculation decision; those belong to SB-16, SB-19 and SB-20.

## Identities

| Value | Meaning | Stability |
|---|---|---|
| `project_id` | The project whose manufacturing definitions are recorded. | Business identity from the version 2 project contract. |
| `part_version_id` | The shared project part version that owns a sourcing record or applied routing. | Business identity from the version 2 project contract. |
| `product_id` | The project product whose root owns a routing or attached cost line. | Business identity from the version 2 project contract. |
| `routing_id` | One applied routing instance. | Client-generated stable UUID, created at the host boundary. |
| `operation_id` | One operation inside one routing or template version. | Stable UUID inside its owner. |
| `line_id` | One attached non-CAD cost line. | Stable UUID inside the project. |
| `template_id` | The logical reusable routing template. | Stable across its published versions. |
| `template_version_id` | One published immutable template version. | Service-generated; recorded in `supersedes` chains. |

`part_version_id` is the customer part number with one customer revision. Different customer
revisions are different part versions with separate sourcing and routing records; editing one never
changes the other. Every occurrence and every product that references the same part version reads
the same record, while its placements stay in the immutable import.

## Sourcing

A `SourcedPart` fixes the explicit purchased or internally manufactured choice of one shared project
part version:

- The choice is required for every custom part version. The service read computes
  `missing_sourcing` for every project part version without a record; nothing is assumed
  manufactured.
- A purchased part records no manufacturing material consumption. Its price, currency, purchase
  unit and descendant cutoff belong to SB-16.
- A manufactured part carries its `MaterialConsumption`: the selected immutable material version
  from the version 2 shared-data contract, the net required consumption with an explicit unit, the
  measurement source, its review state and reviewer, and the simple allowance rate. An explicit
  zero allowance is `"0"`; a missing allowance is `null` and an issue.
- The review follows the reviewed-value convention of the version 2 engineering contract: a
  confirmed consumption needs a value, `confirmed_by` is filled by the service from the
  authenticated identity and never trusted from the payload, and an extracted consumption names the
  exact `analysis_key` it was derived from. A manual override names its reason instead and does not
  claim an analysis.
- Consumption is recorded as a volume (`mm3` or `cm3`) before the allowance and before any costing
  evaluation; SB-16 owns the mass conversion, the allowance formula and the material cost. A
  workshop figure entered in mass units is converted by the costing boundary, which keeps the
  entered source and unit; this contract stores the volume it is evaluated from.
- The version 2 manufacturing record is authoritative for version 2 project costing and readiness.
  The version 1 reviewed input (`PartInputs.sourcing`, its material version, allowance and single
  operation) stays the prototype-only path with its published behavior and is not merged with this
  record; the costing package reads exactly one of them.

## Routing templates

A routing template is a reusable company routing. Publishing is an administrative action:

- `POST /v1/routing-templates` publishes the next version of one logical template. `expected_version`
  0 creates version 1; a later publish names the current version and appends the next one. The
  service records `version`, `template_version_id` and the `supersedes` link.
- A published version is complete and immutable: it carries its schema version, its name, its
  `applies_to` levels (at least one of part, subassembly, product root) and at least one ordered
  operation. A published template is never edited by applying or editing a copy. A stored template
  document that names a schema version this build does not read is refused, never coerced.
- The contract does not alter the SB-09 shared-record and cost-center version lifecycle. It reuses
  the same append-only discipline and the existing Administrator permission set; no machine rate is
  built or published here. This version emits no SB-09 `ReferenceChange` for a routing template:
  extending that shared-record vocabulary is a coordinator decision recorded with SB-20, so the
  template versions themselves are the retained change evidence.
- Engineers read the current template versions to apply one. Listing and reading are available to
  an identity holding either permission set; publishing requires Administrator.

## Applied routings

An `AppliedRouting` is the local routing of one owner: one shared project part version (a part or a
subassembly) or one product root. One owner holds at most one applied routing.

- **Adoption copies; it never links live.** Adopting a `TemplateRef` copies that exact published
  version's operations into the applied routing and retains the source template identity and
  version. A later template version is adopted only by naming it explicitly, and only that
  adoption changes the link.
- **Editing a copy never edits the template.** `EditRouting` replaces the operations of one applied
  routing and keeps its owner, level and source link. The template and every other applied routing
  that was copied from it are unchanged. Shared part edits are visible at the part version's
  occurrences; separate applied routings stay separate.
- A routing entered without a template is `manual` and keeps no template link.
- The source link is immutable in kind: a template-sourced routing is never silently converted to a
  manual one. Replacing the template version is the explicit adoption write.
- One owner holds one applied routing. Adopting for an owner that already has the same routing id
  with a different template version (newer or older, named explicitly) replaces that routing's
  operations and link in place; a write that would create a second routing for an owner, or a
  `manual` write over a template-sourced routing, is refused (`422 invalid`). Adoption creates the
  routing when its id is new; an `edit` of a non-existent routing id is refused.

## Operations, cycle basis and cavity conversion

A `RoutingOperation` records its place in the routing (`sequence`, the first one exactly 1 and then
strictly increasing; the service stores the list in sequence order), its name, its optional cost
center reference, its optional cycle, its optional setup and its optional tooling reference. The
pinned rate versions carry the authoritative cost-center linkage; the service validates that a
pinned version belongs to the operation's cost center when both are named.

An applied routing may exist as an empty draft while an engineer prepares its operations; that
state is the `missing_routing_operations` issue, not a zero cost.

The cycle basis is explicit and fixes the cavity conversion:

| Basis | The seconds figure means | Cavity conversion |
|---|---|---|
| `per_piece` | The time for one finished piece. | None. A cavity count above one is refused, because a per-piece figure is not divided again. |
| `per_cycle` | The time for one machine cycle shared by its cavities. | Per-piece seconds = cycle seconds ÷ the declared cavity count; a missing cavity count is `missing_cycle_cavities` until entered. |

`OperationCycle::per_piece_seconds()` returns `None` while the figure needed by the declared basis is
missing; it never substitutes a zero or an implied cavity count. An explicit zero cycle is a value
and stays distinct from a missing one.

The production rate version is pinned in the cycle: a selected version id is kept until an explicit
change, so a later cost-center rate version does not change this routing. No volume, forecast or
quotation quantity selects or changes a cycle figure or rate.

A calculated cycle names the formula/version that produced it in `source_reference`; a manual figure
does not need one. Geometry-driven calculation itself is later package work.

## Setup

Setup is a separate charge: a duration in hours and the dedicated setup cost-center rate version
that prices it, kept apart from the production cycle and rate. An operation without setup records
none; an operation with a setup record but a missing duration or rate is incomplete and reported as
an issue. The contract does not amortise setup over a batch and adds no batch-frequency assumption.
SB-16 evaluates setup and counts a shared part routing's setup once per project quotation.

## Attached non-CAD cost lines

An `AttachedCostLine` attaches packaging, adhesive, consumables or other items to a project part
version or product root with an explicit category, quantity, unit basis and unit cost. It creates no
CAD node and does not restructure the imported BOM: the line owns a part version or product root,
never a component, occurrence or definition.

- The category and the unit basis are explicit text; the quantity and unit cost stay optional until
  entered, and a missing one is an issue rather than a zero.
- The unit cost is stated in the project currency. Supplier currencies, exchange rates and price
  breaks are SB-16's cost-evaluation boundary.
- The contract makes no one-time-versus-per-unit assumption: the explicit quantity/unit basis names
  what one unit of the owner consumes, and SB-16 derives the contribution and the export lines.
- Removing a line from the draft appends a new record version; earlier record versions keep it.

## Assembly ownership

Installation, joining and testing operations belong to the assembly context that performs them.
They are recorded on the assembly's own applied routing (its part version or product root) and are
never copied into the child parts' routings or into every child unit's cost. A purchased assembly
keeps its CAD hierarchy and its own operations; its descendant cost/demand cutoff belongs to SB-16.

## Issues

Issues are computed on read, never stored. A missing value is an issue, not a zero.

| Code | Meaning |
|---|---|
| `missing_sourcing` | A project part version has no explicit purchased/manufactured choice. |
| `missing_material` | A manufactured part has no material consumption record. |
| `missing_material_version` | The consumption record names no material version. |
| `missing_consumption` | The net material consumption is not recorded. |
| `pending_consumption_review` | The recorded consumption is not confirmed by an engineer yet. |
| `missing_allowance` | The material allowance is not entered; zero must be explicit. |
| `missing_routing` | A manufactured part has no applied routing. |
| `missing_routing_operations` | The applied routing has no operations yet. |
| `missing_cycle` | The operation records no cycle basis. |
| `missing_cycle_time` | The cycle records no time. |
| `missing_cycle_cavities` | A per-cycle time has no cavity count to convert per piece. |
| `missing_production_rate` | The cycle pins no production rate version. |
| `missing_setup_hours` | A recorded setup has no duration. |
| `missing_setup_rate` | A recorded setup pins no dedicated setup rate version. |
| `missing_attached_quantity` | An attached cost line has no quantity. |
| `missing_attached_unit_cost` | An attached cost line has no unit cost. |

## Typed writes and affected references

The three project write routes are deliberately typed, and every one names the shared part versions
and products it affects. The service validates those references against the current project record
and refuses a reference the project does not hold.

- `SetSourcing` writes complete sourcing records. The affected part versions must name exactly the
  written part versions.
- `RoutingWrite` is one tagged payload: `adopt_template` copies a published template version,
  `edit` replaces the operations of an existing applied routing, and `manual` creates a routing
  without a template. Each names its routing, its owner and level (adoption and manual entry) and
  its affected references, which must include the routing's owner — the contract itself refuses a
  payload whose owner is absent. The action tag keeps an edit from silently adopting a newer
  template version. An `edit` payload does not carry the owner: the service validates its affected
  references against the stored routing's owner.
- `EditAttachedCosts` upserts lines by stable line id and removes line ids from the draft. Upserted
  and removed ids are disjoint, and the affected references name the owners of the upserted lines;
  the service checks the owners of removed lines against the stored document. Removing a line id the
  draft does not hold is `422 invalid`; an upsert may move a line's owner when the new owner is
  named as affected.
- The committed write result echoes the validated affected part versions and products
  (`ManufacturingWriteOutcome.affected_part_versions` / `affected_products`), so a readiness gate
  reads them from the write instead of searching the project record. The request log retains each
  write's response.

## HTTP routes

Template routes extend `/v1`; the manufacturing record routes extend `/v2`. The version 1 error body,
request-id idempotency, bounded request time and immutable-version rules still apply. A stale
`expected_version` is `409 conflict`; a malformed or refused payload, including an unknown part
version, product, template version, level mismatch, pinned rate version whose purpose does not match
the field or whose cost center does not match the operation, or a selected material/rate version the
reference data does not hold, is `422 invalid`; an unknown project or record version is
`404 not_found`. Publishing a template answers `201`; reads and project writes answer
`200`.

| Method | Path | Permission | Request | Success | Body |
|---|---|---|---|---|---|
| `POST` | `/v1/routing-templates` | administrator | `PublishRoutingTemplate` | `201` | `RoutingTemplateReceipt` |
| `GET` | `/v1/routing-templates` | engineer or administrator | — | `200` | `RoutingTemplateList` |
| `GET` | `/v1/routing-templates/{template_id}/versions/{version}` | engineer or administrator | — | `200` | `RoutingTemplateVersion` |
| `GET` | `/v2/projects/{project_id}/manufacturing` | engineer | — | `200` | `ManufacturingRecordView` |
| `GET` | `/v2/projects/{project_id}/manufacturing/versions/{version}` | engineer | — | `200` | `ManufacturingRecordView` |
| `POST` | `/v2/projects/{project_id}/manufacturing/sourcing` | engineer | `SetSourcing` | `200` | `ManufacturingWriteOutcome` |
| `POST` | `/v2/projects/{project_id}/manufacturing/routings` | engineer | `RoutingWrite` | `200` | `ManufacturingWriteOutcome` |
| `POST` | `/v2/projects/{project_id}/manufacturing/attached-costs` | engineer | `EditAttachedCosts` | `200` | `ManufacturingWriteOutcome` |

There is deliberately no route that activates a revision, evaluates a cost, adopts a newer template
without an explicit write, or edits a published template version in place.

## Persistence

Migration `services/solidbom-server/migrations/0006_sourcing_manufacturing.sql` adds
`manufacturing_records` (project identity and current version), append-only
`manufacturing_record_versions` (document JSON per version), `routing_templates` (template identity
and current version) and append-only `routing_template_versions` (document JSON per version, with
the stored `schema_version` in the document). The shared `requests` table keeps idempotent request
receipts and each write's affected-reference response. The project, delivery, reference-data and
engineering tables are unchanged.

## Compatibility

- Version 1, version 2 and the SB-11 engineering routes, wire names and meanings are unchanged; this
  contract is additive.
- `CycleBasis` gains the `per_cycle` wire value for the manufacturing routing operation only. The
  version 1 `OperationInputs` refuses that value (`422 invalid`): it keeps its published per-piece
  behavior and is not reinterpreted, and its shipped calculator is unchanged. Affected consumers:
  the version 1 reviewed-input command and calculator keep `per_piece`; SB-16 evaluates routing
  operations with this contract's basis rule. A build before this contract refuses the value rather
  than coercing it.
- The version 1 reviewed-input record and this manufacturing record are separate paths, as stated
  under Sourcing; this contract does not merge them and the costing package reads exactly one.
- Adding an optional field or a route is compatible. Changing a field meaning, a wire name, a unit, a
  route, a status, an error code, an idempotency rule, the stored document schema or a validation
  rule that previously accepted a write is breaking and needs a new contract version, the
  affected-consumer list and coordinator approval.
- Historical manufacturing record versions stay readable under the schema version that wrote them.

## Handoff to later packages

- **SB-16:** consumes the sourced parts with their material consumption and allowance, the applied
  routings with their pinned rate versions and explicit cycle basis, the setup records and the
  attached non-CAD lines. It owns the mass conversion (including a mass-based manual override kept
  with its entered unit), allowance, cavity-adjusted cost, setup charge, **the purchased part's
  price/catalog source, purchase unit, price breaks, FX** and the purchased-branch cutoff; it must
  keep a missing value incomplete rather than zero. A manufacturing record that is `is_complete`
  for a purchased part therefore does **not** imply the purchased price is available: that price
  selection is SB-16's increment and is expected to attach to the same `part_version_id`.
- **SB-19:** treats any open manufacturing issue as a readiness input for coordinated activation;
  this contract publishes no activation decision.
- **SB-20:** template versions and manufacturing record versions are the change evidence for
  affected-project notices and explicit adoption; this increment emits write facts but no project
  notification. A routing-template publish emits no SB-09 `ReferenceChange` in this version; adding
  that shared-record kind is a coordinator decision to record with SB-20.
- **Desktop increment:** the typed engine commands, controller state and the manufacturing editor
  are the next increment of this package; they depend on the SB-10 desktop wiring of project part
  version selection, which is still open.
