# Shared data and permissions — contract version 2

Status: proposed by package SB-09; pinned by the SB-09 execution record. Extends the version 1
registry, whose rules continue to apply. A breaking change to this document needs the
affected-consumer list, a migration/compatibility decision and coordinator approval in
`EXECUTION.md` before a dependent agent continues.

Base: `3c355556b2bafe19ced67e785aa8bee8fff16079` (the integrated SB-08 handoff). Portable types:
`crates/solidbom-contract/src/shared_data.rs` and the reference records in `types.rs`. Wire
vocabulary: `host.rs` requests/replies and `api.rs` error codes. Implementation: package SB-09 owns
the service `shared_data` and `authorization` modules, the desktop client/controller and the
administrative UI.

## Scope

One organization per deployment. Its users share company reference data under two combinable
permission sets:

| Permission set | Allows |
|---|---|
| `engineer` | Project work: sources, project versions, reviewed inputs, calculations, reopening and activation. |
| `administrator` | Shared records: versioned material catalogs, cost-center rates, catalog components, app cost configuration, feasibility rules and routing templates in later packages. |

One identity may hold both sets. The service decides every request; a hidden button in a client is
not enforcement.

## Identity and authorization

Every route except `GET /health` needs `Authorization: Bearer <token>`. A token maps to exactly one
`Identity`:

```json
{ "identity_id": "…", "name": "development", "permissions": ["engineer", "administrator"] }
```

- Unknown or missing token: `401` with code `unauthorized`.
- Known identity without the permission set the route requires: `403` with code `forbidden`.
- `GET /v1/me` answers the caller's identity for any authenticated caller.
- Development configuration: `SOLIDBOM_TOKEN` is the combined legacy identity (both sets), so an
  existing single-token prototype setup keeps working. Optional `SOLIDBOM_ENGINEER_TOKEN` and
  `SOLIDBOM_ADMIN_TOKEN` add an engineer-only and an administrator-only identity. Tokens are at
  least 16 characters and must be distinct; a duplicate is a startup failure. No production
  authentication, no multi-organization isolation and no onboarding in MVP.

## Routes

Route permissions are grouped, not per handler:

| Method | Path | Permission | Request → success body |
|---|---|---|---|
| `GET` | `/health` | public | — → `{"ok":true}` |
| `GET` | `/v1/me` | any | — → `Identity` |
| `PUT` | `/v1/sources/{sha256}?name=…` | engineer | raw STEP bytes → `SourceRef` |
| `GET` | `/v1/sources/{source_id}` | engineer | — → raw bytes, `application/step`, `x-solidbom-sha256` |
| `GET` | `/v1/reference-data` | engineer | — → `ReferenceData` |
| `POST` | `/v1/projects` | engineer | `SaveDraft` → `SaveReceipt` |
| `GET` | `/v1/projects/{project_id}` | engineer | — → `ProjectView` |
| `PUT` | `/v1/projects/{project_id}/draft` | engineer | `SaveDraft` → `SaveReceipt` |
| `POST` | `/v1/projects/{project_id}/calculations` | engineer | `CalculateRequest` → `CalculationSnapshot` |
| `GET` | `/v1/calculations/{calculation_id}` | engineer | — → `CalculationSnapshot` |
| `GET` | `/v1/shared` | administrator | — → `SharedDataView` |
| `POST` | `/v1/shared/materials/{material_id}/versions` | administrator | `CreateMaterialVersion` → `MaterialVersionPublished` |
| `POST` | `/v1/shared/cost-centers/{cost_center_id}/versions` | administrator | `CreateRateVersion` → `RateVersionPublished` |
| `POST` | `/v1/shared/components/{component_id}/versions` | administrator | `CreateComponentVersion` → `ComponentVersionPublished` |
| `POST` | `/v1/shared/app-config/{config_id}/versions` | administrator | `CreateAppConfigVersion` → `AppConfigVersionPublished` |

The version 1 HTTP rules still hold: exact error body, bounded request ids, canonical payload
hashes, idempotent retries, immutable project versions and snapshots. `API_VERSION` is 2.

## Versioned shared records

Every shared record is a logical record plus an append-only chain of published versions. A version
carries the logical id, its version number, the version it supersedes, explicit money and units,
its optional source/version reference and the service's `created_at`.

| Record | Logical id | Version id | Money | Units |
|---|---|---|---|---|
| Material | `material_id` | `material_version_id` | `price_per_kg` optional | `density_unit` g/cm3, `price_unit` cny/kg, `currency` |
| Cost center | `cost_center_id` | `rate_version_id` | `rate_per_hour` optional | `rate_unit` cny/h, `purpose` production or setup, `currency` |
| Catalog component | `component_id` | `component_version_id` | `price` optional | `price_unit` cny/piece, `currency` |
| App config | `config_id` | `config_version_id` | category amounts | `currency`, explicit category bases |

Rules:

- A material publish request carries the version's own positive density; a later version never
  inherits a value silently.
- A publish request names the logical id and `expected_version`: `0` creates version 1 of a new
  logical record; otherwise it must equal the current latest version, or the write is `409`.
  The published version's `supersedes` is the current latest version id.
- Every write is keyed by `request_id` plus a canonical payload hash. An identical retry returns
  the recorded reply; a reused id with a different payload is `409`. The version and its
  `ReferenceChange` fact commit in one transaction.
- Missing money is `null`, never zero; an explicit `0` is a decimal string `"0"`. Both are
  preserved and distinguishable in reference data, calculation inputs and snapshots. A missing
  price or rate yields an incomplete calculation with a named issue, not a zero cost.
- A project version and its calculation snapshot store the exact version ids and the resolved
  values they used. Publishing a newer version never edits, recalculates or reinterprets them.
  Historical versions stay readable after a newer version is published.
- `GET /v1/reference-data` returns every published version so a project may select any immutable
  version; `GET /v1/shared` returns the same records for administration plus the newest reference
  changes. The model does not delete or rewrite a published version in MVP.

## App cost configuration and category bases

The app configuration is a versioned list of cost categories. There is no formula language:

- `direct` categories name one supported engine subtotal (`material`, `production` or `setup`).
- `fixed` categories are a fixed amount per unit in the configuration currency.
- `percentage` categories are a rate of an explicitly declared `CategoryBasis`: the direct-cost
  subtotal and/or named category codes. Percentage bases are cost subtotals, never selling price.

Validation refuses, with a `422 invalid` and a message naming the category:

- an ambiguous percentage base (no declared base term),
- a base that names the category itself,
- a circular base chain,
- an unknown basis category, a category listed twice in a base, a duplicated category code,
- the same direct-cost subtotal included twice,
- a negative fixed amount or percentage rate,
- a direct-cost category selected for recurring EBIT (direct cost is already inside the recurring
  unit cost; the setup subtotal stays outside recurring EBIT).

## Reference-change facts

Every published version appends a `ReferenceChange` fact: kind, logical record id, version id,
version number, previous version id, a short summary, the publishing identity's name and the
service timestamp. The facts are listed newest first (at most 100) in `SharedDataView.changes`.
They are facts for later packages (SB-20) to consume; they are not project notifications, do not
adopt anything into a project and never recalculate a project.

## Compatibility

- The contract crate's `SCHEMA_VERSION` stays 1: stored analysis manifests, project versions and
  calculation snapshots keep their version and remain readable.
- `API_VERSION` is 2. Version 2 adds the routes, the `forbidden` code, the versioned reference
  records and optional money/units. A version 1 client must not read a version 2 service; the
  desktop and service ship together in this development deployment.
- Adding an optional field, a defaulted request or a route is compatible. Changing a field
  meaning, wire name, unit, route, status, error code, idempotency key or the version-chain rules
  is breaking.

## Acceptance

- Engineer-only, administrator-only and combined identities through the service API and the
  desktop command/UI paths; refusal comes from the service, not a hidden control.
- Revising a price or rate leaves an existing project version and calculation snapshot unchanged;
  explicit zero and missing stay distinguishable.
- Ambiguous, circular and duplicate category inclusion is refused; genuine administration UI
  evidence and the root/service gates are recorded in the SB-09 execution record.
