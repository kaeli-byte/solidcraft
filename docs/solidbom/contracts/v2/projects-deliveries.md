# Projects, deliveries and part versions contract

Version: **2** · Status: **published by SB-10** (owner instruction 2026-11-09: lock the
project/delivery contract and start implementation; the locking commit is recorded in
[SB-10's execution record](../../execution/SB-10.md)).

This contract extends version 1 additively: the version 1 routes, types and the prototype
draft/calculate path stay exactly as published in the [`../v1/`](../README.md) documents. Version 2 adds the
multi-product project record described here.

Portable types: `crates/solidbom-contract/src/projects.rs` (re-exported at the crate root).
Producers: the service store (`services/solidbom-server/src/projects.rs`) and the version 2 project
routes. Consumers: the desktop controller (to be wired), and the later packages SB-11 (engineering
review per product revision), SB-15 (sourcing/routing per shared part version) and SB-19
(activation and accepted baselines).
Acceptance: the contract crate's `projects` tests, the service's `tests/projects.rs` domain tests
and the version 2 route tests in `tests/api.rs`.

## Rules that apply to every value

The version 1 rules still apply: business amounts are decimal strings; a missing value is `null`
and an explicit zero is `"0"`, never interchangeable; unknown JSON fields are tolerated; a newer
stored schema version is refused, never coerced.

Additional rules for this contract:

- Every write names a `request_id` and an `expected_version`. The service keys the request by
  request id plus a canonical payload hash; an identical retry returns the recorded outcome before
  the version check, and a changed payload under the same request id is a conflict.
- A write whose `expected_version` is not the current project record version is a conflict and
  writes nothing.
- Every accepted write appends one immutable project record version. Earlier documents,
  deliveries, revisions, part versions and quotation events stay readable through
  `GET /v2/projects/{project_id}/versions/{version}`.
- The service assigns `product_id`, `product_revision_id` and `part_version_id` when it creates an
  identity. Clients keep `project_id` and `package_id`.

## Identities

| Value | Meaning | Stability |
|---|---|---|
| `project_id` | One customer inquiry: business details, products, deliveries and part versions. | Business identity, UUID. |
| `package_id` | One customer delivery of product data; belongs to exactly one project. | Business identity, UUID. |
| `product_id` | One simple part or assembly in the project; persists across deliveries. | Business identity, UUID. |
| `product_revision_id` | One revision of a product, delivered in one package. | Business identity, UUID. |
| `part_version_id` | One shared project part definition: a customer part number with one customer revision. | Business identity, UUID. |
| `source_id` | The stored original STEP file of a package. | Assigned by the store; the original file is never modified. |

A customer part number and revision propose identity; they are never used alone as an identity.
Different customer revisions of one number are different part versions and are never merged.
Similar geometry alone never merges parts, and no customer revision is invented to make the
existing project fit.

## Project document

The stored document (`ProjectDocument`, `schema_version` 1 inside contract version 2):

- Business details (`ProjectDetails`): `name`, `customer`, optional `internal_team`, `platform`
  and `application`, `currency`, and the project default `default_schedule` (inclusive
  `sop_year`/`eop_year`, four-digit calendar years, at most 200 production years).
- `products: [Product]`: persistent product identity, `name`, `kind` (`part` or `assembly`),
  optional customer part number, optional `schedule_override`, per-calendar-year `forecasts`,
  `revisions`, `active_revision_id` and `quotation_state`.
- `packages: [DeliveryPackage]`: one delivery, its `name` and the retained original `SourceRef`.
- `part_versions: [PartVersion]`: shared project part definitions with the customer number and
  revision, the validated `volume_mm3` when geometry was available, and the creating package.
- `quotation_events: [QuotationEvent]`: append-only record of exclusion/reinstatement scope
  changes, with a strictly increasing `sequence`.

`ProductRevision` carries `product_revision_id`, `package_id`, `state`
(`draft`/`active`/`historical`) and the delivered parts. A `DeliveredPart` carries the client's
stable `delivered_key`, optional customer number/revision, optional `volume_mm3`, and the
associated `part_version_id`; `null` means the match is pending engineer confirmation.

## Forecasts and schedules

- A product uses its `schedule_override` when present and the project `default_schedule`
  otherwise. Years outside that range are invalid, not padded.
- Every production year from SOP through EOP inclusive needs a positive forecast. A zero
  quantity is invalid; a missing year stays an `incomplete` issue (`missing_forecast`) and is never
  converted to zero.
- Forecasts are calendar years only; no monthly proration exists in this contract.

## Deliveries and revisions

- `POST /v2/projects/{project_id}/deliveries` records one package and the product revisions it
  delivers, as one transaction with an expected-version check. The package source must already be
  stored (`PUT /v1/sources/{sha256}`).
- A delivered product is matched to a project product by an explicit `product_id` when given,
  else by unique customer part number, else by unique product name. A product name is unique
  within a project, and an ambiguous proposal is refused instead of sharing another product's
  work; the client must name the product explicitly.
- A delivered existing product gains a new `draft` revision. Omitted products are unchanged: their
  revisions, part versions and active revision stay exactly as they were.
- A product that has no project identity yet is created with its first `draft` revision, no
  forecasts, quotation included. It stays incomplete until its forecasts are entered; this is how
  a later delivery may add a product.
- A part is associated with a shared `part_version` when its customer number and revision match
  one existing part version and geometry validates the match within the contract's relative
  tolerance `0.001` (0.1 %). When the identity is new, the service creates the part version. When
  geometry is missing or disagrees, or the identifiers are incomplete, the delivered part stays
  unassociated and an issue asks for engineer confirmation; `POST
  /v2/projects/{project_id}/part-matches` confirms the association explicitly.
- A failed delivery (ambiguous product, unknown source, stale version) writes no record version:
  products from earlier successful deliveries are untouched.

## Draft, active and historical revisions

- The contract stores and validates the three states. A product has at most one `active` revision,
  and `active_revision_id` must name it. A previously active revision becomes `historical` when a
  new revision is activated.
- **No activation route is published in contract version 2.** Activation readiness, coordinated
  shared-part activation and accepted baselines belong to SB-19. The service exposes the internal
  transition `Store::record_revision_activation` (used by the acceptance tests) for SB-19 to
  drive; a client cannot activate through HTTP in this contract.

## Quotation exclusion and reinstatement

- `POST /v2/projects/{project_id}/exclusions` and `/reinstatements` set the quotation scope of one
  or more products and append a `QuotationEvent` in the same record version.
- Exclusion never deletes packages, files, revisions, part versions, forecasts or recorded events.
  It emits a `quotation_scope_changed` fact and leaves accepted results untouched; dependent
  draft recalculation belongs to SB-20.
- Reinstatement uses the retained work and emits the same fact with `included`. Any review or
  update requirements that still apply are resolved by the later packages that own them.

## Issues

Issues are computed on read, never stored. A missing value is an issue, not a zero.

| Code | Meaning |
|---|---|
| `missing_forecast` | One required production year has no positive forecast. |
| `missing_part_identity` | A delivered part has no customer number or no revision to propose identity. |
| `missing_geometry_for_match` | Geometry is unavailable, so a proposed identity match needs confirmation. |
| `conflicting_geometry_match` | Geometry disagrees with the existing part identity; an engineer confirms or keeps it separate. |

## Dependency facts

Writes return facts. Facts name what changed; they never rewrite accepted results, and the
consumers below own their workflows.

| Fact | Meaning | Consumer |
|---|---|---|
| `delivery_recorded` | A package appended revisions; names the product ids and the count of pending part matches. | SB-11, SB-19 |
| `quotation_scope_changed` | Products entered or left the quotation scope. | SB-20 |
| `part_match_confirmed` | An engineer confirmed one pending delivered-part match. | SB-11, SB-19 |

## HTTP routes

Base path `/v2`; a bearer identity with the Engineer permission set from the version 2 permissions contract, and bounded request time exactly as version 1. Errors use the
version 1 `ApiErrorBody` shape and codes. A stale `expected_version` is `409 conflict`; a malformed
or refused payload is `422 invalid`; an unknown project is `404 not_found`.

| Method | Path | Request | Success | Body |
|---|---|---|---|---|
| `POST` | `/v2/projects` | `CreateProject` | `201` | `ProjectWriteOutcome` |
| `GET` | `/v2/projects/{project_id}` | — | `200` | `ProjectRecordView` |
| `PUT` | `/v2/projects/{project_id}` | `UpdateProject` | `200` | `ProjectWriteOutcome` |
| `PUT` | `/v2/projects/{project_id}/products/{product_id}` | `UpdateProduct` | `200` | `ProjectWriteOutcome` |
| `POST` | `/v2/projects/{project_id}/deliveries` | `RecordDelivery` | `201` | `ProjectWriteOutcome` |
| `POST` | `/v2/projects/{project_id}/part-matches` | `ConfirmPartMatch` | `200` | `ProjectWriteOutcome` |
| `POST` | `/v2/projects/{project_id}/exclusions` | `ExcludeProducts` | `200` | `ProjectWriteOutcome` |
| `POST` | `/v2/projects/{project_id}/reinstatements` | `ReinstateProducts` | `200` | `ProjectWriteOutcome` |
| `GET` | `/v2/projects/{project_id}/versions/{version}` | — | `200` | `ProjectRecordView` |

There is deliberately no activation, readiness or recalculation route in version 2; those belong
to SB-19 and SB-20.

## Persistence

Migration `services/solidbom-server/migrations/0004_projects_deliveries.sql` adds
`project_records` (business identity and current version) and `project_record_versions`
(append-only document JSON per version). The shared `requests` table keeps idempotent request
receipts; the shared `sources` table keeps the original bytes each package names.

## Compatibility

- Version 2 is additive. Version 1 routes, wire names and meanings are unchanged.
- Adding an optional field is compatible. Changing a field meaning, a wire name, a unit, a route,
  a status, an error code, an idempotency rule or the stored document schema is breaking and needs
  a new contract version, the affected-consumer list and coordinator approval.
- Historical project record versions stay readable under the schema version that wrote them.
- The v1 prototype project draft/version routes remain available for the prototype path; they are
  not the multi-product project record.

## Handoff to later packages

- **SB-19:** publish the activation route, readiness gate and coordinated shared-part activation on
  top of the stored states; it drives `record_revision_activation` and must not change historical
  revisions.
- **SB-11/SB-15:** attach engineering review and sourcing to `product_revision_id` and
  `part_version_id` identities; share edits only within the customer version that was confirmed.
- **SB-20:** consume `quotation_scope_changed` and delivery facts for draft recalculation and
  scenario refresh; accepted snapshots stay unchanged.
- **SB-16/SB-18:** add volume-policy, EBIT-target and exchange-rate settings additively to the
  project document; this contract does not invent them.
