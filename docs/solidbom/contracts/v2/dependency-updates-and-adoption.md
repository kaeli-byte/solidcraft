# Dependency updates and adoption contract

Version: **1** (additive to API version 2) · Status: **locked by SB-20** (coordinator assignment of
2026-11-11; the lock commit is recorded in
[SB-20's execution record](../../execution/SB-20.md)). Base: integrated `main` `acc7c3582`, the merge
of SB-18 (PR #63) and SB-19 (PR #64).

This contract is the version 1 dependency-update and adoption vocabulary of package SB-20, additive
to API version 2. The shared reference data (materials, cost-center rates, catalog components,
routing templates, feasibility rule tables and the app cost configuration) is already versioned and
append-only (SB-09, SB-13, SB-14, SB-15, SB-16). This contract publishes what a project does when a
newer version of a shared logical record is published: it makes the pending update visible, requires
an explicit decision before the update enters a draft calculation, and re-publishes a draft
calculation when forecasts or quotation scope change. The version 1 and version 2 routes, types and
stored documents stay exactly as published in the [registry](../README.md); nothing here rewrites
them.

Portable types: `crates/solidbom-contract/src/adoption.rs` (re-exported at the crate root), plus the
additive `#[serde(default)] draft: Option<DraftPublication>` field on `ProjectWriteOutcome` in
`crates/solidbom-contract/src/projects.rs`. Producers: the service adoption store and the version 2
adoption routes (module `services/solidbom-server/src/adoption.rs`, migration
`0012_dependency_decisions.sql`), and the project write path that publishes a draft automatically.
Consumers: the desktop update/adoption surface (later increment), SB-21 (internal exports) and SB-22
(update recovery). Acceptance: the contract crate's `adoption` tests, the service's
`tests/adoption.rs` domain tests and the adoption route cases in `tests/api.rs`.

## Rules that apply to every value

The version 1 and version 2 rules still apply: a missing value is `null` and an explicit zero is a
real value, never interchangeable; unknown JSON fields are tolerated; a newer stored schema version
is refused, never coerced; identities are UUIDs and versions are integers; a value received from
another program is validated before it is stored or returned.

Additional rules for this contract:

- **Viewing is free.** The shared-update catalog, the project dependency report and the global
  project list are pure reads in one transaction. They append no project version, record version,
  snapshot, decision row or request receipt. Reading a notification is not adopting an update.
- **Acknowledgement is not adoption.** An `acknowledged` decision records that the engineer saw the
  update; nothing else changes and no draft calculation uses it. An `adopted` decision pins the exact
  newly published version; it never recalculates by itself.
- **Adoption and recalculation are separate.** A recalculation is the explicit
  `POST /v2/projects/{project_id}/draft-recalculations` command. Adoption plus recalculation creates
  a new immutable result; earlier results, accepted baselines and historical snapshots stay
  byte-identical.
- **A publication is bound to the exact generation it read.** A decision names the current project
  record version; a publication names the current project, engineering and manufacturing record
  versions. A request whose expected generation moved is a `409` conflict and writes nothing.
- **No silent selection.** A newer published version is never selected automatically for a project
  that already resolved an older one (the app configuration included). A shared update enters draft
  calculation only after explicit adoption.
- **No partial write.** A failed adoption commits nothing: no record version, no decision row, no
  request receipt. A failed automatic publication rolls the whole edit back.
- **A missing input stays missing.** A publication without the inputs a complete cost needs is an
  `incomplete` draft result with the calculator's named issues; a stale current-looking number is
  never substituted for it.
- **Dependencies are identified.** A catalog change flags only the projects that resolve the changed
  logical record, with the affected part versions and products named.
- **Sorted lists stay sorted.** Every list this contract returns has the canonical order stated per
  value; `validate()` refuses an unsorted or duplicated list instead of silently trusting it. Every
  project version, decision sequence and published-record version is at least 1; an engineering or
  manufacturing record generation may be 0, which means the project has no such record yet. Every
  identity is non-nil, and every text is bounded and free of control characters.

## Portable values

`sc` below is `crates/solidbom-contract/src/adoption.rs`; every type derives `Debug, Clone,
Serialize, Deserialize, PartialEq, Eq` unless noted, and none carries a floating-point value.

| Value | Shape | Meaning |
|---|---|---|
| `SharedUpdateKind` | `material`, `cost_center_rate`, `catalog_component`, `routing_template`, `feasibility_rule_table`, `app_config` | The kind of shared logical record a published version belongs to. `is_adoptable()` is true for the first three and `app_config`, false for the two flagged-only kinds. |
| `SharedUpdate` | `kind`, `record_id`, `version_id`, `version`, `previous_version_id?`, `published_at?` | One newest published version of one shared logical record (a version-change fact). |
| `SharedUpdateList` | `updates: Vec<SharedUpdate>` | Read-only catalog, sorted by `(kind, record_id, version)`, bounded by `MAX_SHARED_UPDATES`. |
| `ProjectDependency` | `kind`, `record_id`, `referenced_version_id`, `referenced_version`, `available_version_id`, `available_version`, `part_version_ids`, `product_ids`, `acknowledged` | One outdated dependency: the version the project resolves vs the newest published version of the same logical record, with its affected scope. |
| `DependencyDecisionAction` | `acknowledged`, `adopted` | Whether a decision saw an update or pinned it. |
| `DependencyDecision` | `decision_id`, `project_id`, `sequence`, `kind`, `record_id`, `from_version_id?`, `to_version_id`, `action`, `decided_by`, `decided_at` | One append-only decision row; `sequence` is strictly increasing per project from 1. |
| `ProjectDependencyReport` | `project_id`, `project_version`, `engineering_version`, `manufacturing_version`, `dependencies`, `decisions`, `draft?`, `flagged_scenarios` | The read-only point-in-time impact answer for one project. |
| `DraftStaleReason` | `newer_project_version`, `newer_engineering_version`, `newer_manufacturing_version`, `newer_shared_data` | One named reason a stored draft result is no longer current. |
| `DraftResultStatus` | `calculation_id`, `project_version`, `engineering_version`, `manufacturing_version`, `state`, `input_hash`, `calculator_version`, `stale`, `stale_reasons`, `created_at?` | A stored draft result and its stale markers; `stale` is exactly true when `stale_reasons` is not empty. |
| `FlaggedScenario` | `scenario_id`, `version`, `name`, `baseline_calculation_id`, `pinned_project_version`, `stale_reasons` | A saved commercial scenario that pinned an older result; the reasons reuse the SB-18 code strings (`scenario_baseline_outdated`, `scenario_project_outdated`). |
| `ImpactedProject` | `project_id`, `project_version`, `pending_count`, `kinds` | One project a shared update flags; `pending_count` is at least 1 and `kinds` is sorted and duplicate-free. |
| `DependencyProjectList` | `projects: Vec<ImpactedProject>` | The global flagged-project list, sorted by project id, bounded by `MAX_AFFECTED_PROJECTS`. |
| `DependencyDecisionItem` | `kind`, `record_id`, `version_id` | One explicit decision request item; `version_id` must be the newest published version of that logical record. |
| `RecordDependencyDecisions` | `request_id`, `expected_version`, `action`, `decisions` | One explicit decision request, unique by `(kind, record_id)`, in request order. |
| `DependencyDecisionReceipt` | `request_id`, `project_id`, `project_version`, `decisions`, `facts` | The committed answer; decisions are in canonical `(kind, record_id)` order with one matching fact each. |
| `DependencyFact` | `dependency_acknowledged` / `dependency_adopted` (tagged by `fact`) | What an explicit decision changed; a fact never rewrites an accepted result. |
| `RecalculateDraft` | `request_id`, `expected_project_version`, `expected_engineering_version`, `expected_manufacturing_version`, `product_revisions` | Explicit draft publication; empty `product_revisions` means the default draft rule. |
| `DraftPublication` | `calculation`, `dependencies`, `flagged_scenarios` | The result of an explicit or automatic publication: the new immutable snapshot summary (SB-16 `BaselineCalculationSummary`), the dependencies still pending and the scenarios the new result flags. |
| `DependencyRefusal` | `unknown_update`, `stale_update`, `unsupported_adoption`, `no_pending_dependency`, `duplicate_decision`, `stale_project_version`, `stale_draft_generation` (tagged by `refusal`) | The stable refusal reasons; `code()`, `message()`, `error_code()` and `validate()`. |

Constants, all in `adoption.rs`: `ADOPTION_CONTRACT_VERSION = 1`, `MAX_SHARED_UPDATES = 500`,
`MAX_PROJECT_DEPENDENCIES = 200`, `MAX_DEPENDENCY_TARGETS = 500`, `MAX_DEPENDENCY_DECISIONS = 100`,
`MAX_DECISION_HISTORY = 100`, `MAX_AFFECTED_PROJECTS = 200`, `MAX_FLAGGED_SCENARIOS = 100`,
`MAX_DEPENDENCY_REQUEST_ID_CHARS = 128`.

`ProjectWriteOutcome` gains one additive field, `draft: Option<DraftPublication>`, `#[serde(default)]`.
It is `Some` only when the write changed the draft demand and a draft result was published in the same
transaction; it is `null` for every other write and for an outcome written before this contract.

## Dependency kinds and resolution

A dependency exists when the version a project resolves for one shared logical record differs from
the newest published version of that same logical record. Per kind, the project resolves it from:

| Kind | Where the project resolves the referenced version | Logical record | Affected scope |
|---|---|---|---|
| `material` | Each manufacturing `SourcedPart` with `Sourcing::Manufactured` and a `material.material_version_id`. | The material version's `material_id`. | The part versions; the products using them. |
| `cost_center_rate` | Each applied routing operation's `cycle.production_rate_version_id` and `setup.setup_rate_version_id`. | The rate version's `cost_center_id`. | The part versions owning the routing; for a product-root routing the product ids instead of part versions. |
| `catalog_component` | Each engineering `ReviewedPart.catalog.confirmed.component_id`; a suggestion never counts, only a confirmed reference. | The catalog component version's component id. | The part versions; the products using them. |
| `routing_template` | Each applied routing whose `source` is `RoutingSource::Template(TemplateRef { template_id, version })`; the exact `template_version_id` is resolved from the stored `routing_template_versions` row of that logical id and version. | The template version's `template_id`. | The part versions / products owning the routing. |
| `feasibility_rule_table` | Each assessment in the current feasibility record, by its `rule_table_version_id`. | The rule-table version's `rule_table_id`. | The assessed components' product revisions, then their products. |
| `app_config` | Resolved by the refinement below; flagged only in the report. | The configuration record. | No targets; the dependency is project-wide. |

Resolution rules:

- One project referencing several versions of one logical record is reported as **one** dependency per
  `(kind, record_id)`: the oldest referenced version is the `referenced_version` and the scope is the
  union of the referencing part versions and products.
- `part_version_ids` and `product_ids` are sorted, duplicate-free and bounded by
  `MAX_DEPENDENCY_TARGETS`; a dependency names at least one target unless it is the project-wide
  `app_config` kind.
- A dependency whose referenced version is **newer** than the newest published version is corrupt
  state: the read is refused as invalid (`422`), and the contract never invents a downgrade or a
  negative dependency.
- The earlier version a `SharedUpdate` names (`previous_version_id`) is read from the version
  row's `supersedes` where the chain stores it. The `routing_template_versions` table has no
  `supersedes` column; there the previous version is the stored template version document's own
  `supersedes` field, and it may be absent.
- A published material, cost-center rate, catalog component, app configuration, routing template and
  feasibility rule-table version is adoptable through this contract only when it is the newest
  published version of its logical record.
- `acknowledged` is true when an acknowledgement decision for `available_version_id` is recorded.
  Adoption removes the dependency from the list instead of raising this flag.
- The global list never includes a project with zero pending dependencies. The optional `kind` and
  `record_id` filter pair must be given together; one without the other is `422`, and unknown filter
  values are `422`, not an empty list that looks current. A project's optional `kind` filter narrows
  the scan; the scan is bounded (the service scans at most 1,000 projects and returns at most
  `MAX_AFFECTED_PROJECTS` rows).

### App-configuration resolution refinement (owned by SB-20)

The app cost configuration is not stored per project, so SB-16 published "resolve the latest published
configuration". SB-20 refines that resolution order for the draft calculation, intentionally and
without changing any SB-16 wire type:

1. the version named by the project's latest **adopted** `app_config` decision, when one exists;
2. otherwise the configuration version the newest baseline snapshot of the project recorded;
3. otherwise the latest published configuration.

A decision that names an unreadable configuration version is an error, never a silent fallback to the
latest one. A project with no adopted decision and no snapshot keeps the published latest-version
behaviour exactly, so `calculate_baseline` and every old project behave as before. The refinement is
recorded in the compatibility section below because it narrows an SB-16 published rule on purpose; it
is this contract's decision, not an accident of the service.

## Project impact report

`ProjectDependencyReport` is a point-in-time answer computed from the current heads in one
transaction: the current project record version, the current engineering, manufacturing and
feasibility record versions, the published reference data and the append-only decision table. It
writes nothing. Its fields:

| Field | Meaning |
|---|---|
| `project_id`, `project_version` | The project and the exact project record version read. |
| `engineering_version`, `manufacturing_version` | The current engineering and manufacturing record versions read; 0 when the project has no such record yet. |
| `dependencies` | Current outdated dependencies, sorted by `(kind, record_id)`, bounded by `MAX_PROJECT_DEPENDENCIES`. |
| `decisions` | The decision history, newest first, bounded by `MAX_DECISION_HISTORY`, with unique sequences. |
| `draft` | The project's newest draft result with its stale markers, or `null` when the project has no snapshot. |
| `flagged_scenarios` | Saved scenarios whose pinned result or project version is older, sorted by scenario id, bounded by `MAX_FLAGGED_SCENARIOS`. |

The newest draft result is the newest row by creation order of the append-only SB-16
`baseline_calculations` table. Its stale reasons are: `newer_project_version`,
`newer_engineering_version` or `newer_manufacturing_version` when the result's recorded version is
not the current head, and `newer_shared_data` when at least one dependency is pending. The reasons are
sorted by their wire name and `stale` is exactly true when the reasons are not empty; a result with no
markers is never labelled stale.

A saved commercial scenario is flagged when its pinned baseline is not the newest draft result
(`scenario_baseline_outdated`, using the SB-18 vocabulary and message) or its pinned project version
is not current (`scenario_project_outdated`). Only the latest saved version per `scenario_id` is
considered, the reasons are sorted by code without duplicates, and a flagged scenario always names at
least one reason.

The report's flags never contradict its own decision history: a listed dependency cannot name a
version that an adopted decision already records, and a dependency with an acknowledgement decision
in the history is flagged `acknowledged` (this is what `ProjectDependencyReport::validate` refuses).

## Explicit decisions: acknowledgement vs adoption

`POST /v2/projects/{project_id}/dependency-decisions` takes a `RecordDependencyDecisions`:

| Field | Meaning |
|---|---|
| `request_id` | The idempotency key: bounded, non-blank, no control characters. |
| `expected_version` | The current project record version the request was prepared against; at least 1. |
| `action` | `acknowledged` or `adopted`, applying to every item in the request. |
| `decisions` | At least one and at most `MAX_DEPENDENCY_DECISIONS` items, unique by `(kind, record_id)`, in request order. |

The service keys the request by request id plus a canonical payload hash in the shared `requests`
table, like every other version 2 write. An identical retry returns the recorded receipt before any
version check; a changed payload under the same request id is a `409 conflict`. One request is one
transaction:

1. Validate the request and lock the project row. `expected_version` must be the current project
   record version, or the answer is `409 stale_project_version` and nothing is written.
2. Lock the current engineering and manufacturing record heads, so the pins a decision reads are the
   pins it writes.
3. Resolve the newest published version of each named logical record. An unknown record or version is
   `404 unknown_update`; a named version that is not the newest is `409 stale_update`.
4. Resolve each item's currently referenced version with the same rules as the report. A project
   that already resolves the named version is `409 no_pending_dependency`. An identical decision row
   (same kind, record, to-version and action) is `409 duplicate_decision`.
5. For `acknowledged`: append one decision row per item and change nothing else. No record version,
   no snapshot, and a later recalculation still uses the old version.
6. For `adopted`: first refuse a flagged-only kind (`routing_template`, `feasibility_rule_table`) with
   `422 unsupported_adoption`. Otherwise repin the project:
   - `material`: replace `material_version_id` on every manufactured part whose current material
     version belongs to the named logical record;
   - `cost_center_rate`: replace every routing operation cycle/setup rate id whose rate version
     belongs to the named logical record;
   - `catalog_component`: replace `catalog.confirmed` with a `ConfirmedCatalogReference` resolved
     from the exact newly published component version (component id, version id and number, name,
     specification, price, unit, currency) and set `confirmed_by` to the authenticated caller name;
     a saved project override stays untouched and no price is ever invented;
   - `app_config`: no record repin; the decision row is the pin the resolution refinement uses.
   Append one manufacturing record version and/or one engineering record version when those documents
   changed (both validated with their owning contract's `validate()`), append the decision rows and
   record the receipt. Either every record version and decision row commits, or nothing does.
7. Answer with a `DependencyDecisionReceipt`: `request_id`, `project_id`, the project record version
   the decisions were checked at, the new `decisions` in canonical `(kind, record_id)` order, and one
   `DependencyFact` per decision in the same order (`dependency_acknowledged` or
   `dependency_adopted`, with the previous version when there was one). A receipt with a missing,
   extra, reordered or disagreeing fact is refused by `validate()`.

The project record version is unchanged by decisions; the engineering and/or manufacturing record
version is appended only by an adoption that actually changed that document.

## Draft publication

### Explicit recalculation

`POST /v2/projects/{project_id}/draft-recalculations` takes a `RecalculateDraft`:

| Field | Meaning |
|---|---|
| `request_id` | The idempotency key, bounded and free of control characters. |
| `expected_project_version`, `expected_engineering_version`, `expected_manufacturing_version` | The exact generation the client observed; all three mandatory. `expected_project_version` is at least 1; the engineering and manufacturing generations may be 0 when the project has no such record yet. |
| `product_revisions` | One explicit `{product_id, product_revision_id}` per product, ascending by product id and unique; empty means the default draft rule. |

The default draft rule (the "preview draft in active scope" mechanic) uses, for every product, its
newest `draft` revision when it has one, else its `active` revision, else none (which the calculator
reports as a missing selection). It never substitutes a second revision and never counts both.

One transaction: validate and replay the request id; lock the project row; the current project
version must equal `expected_project_version` and the current engineering/manufacturing versions must
equal their expected values (locked `FOR SHARE`). Any mismatch is `409 stale_draft_generation` naming
the reasons in the version 1 error body, and nothing is written. The baseline is then resolved with
the SB-16 resolution code (app configuration per the refinement above) and one immutable snapshot is
appended. Missing inputs stay `incomplete` with the calculator's named issues. The answer is the
`DraftPublication`: the new snapshot summary, the dependencies still pending and the scenarios the
new result flags. A publication appends **no** project or record version.

### Automatic publication

Two project writes changed the draft demand, and they publish in the same transaction as the edit:

- `Store::update_product` publishes after appending the new project version, but only when the stored
  `forecasts` or `schedule_override` actually changed; other product fields do not publish.
- `Store::exclude_products` and `Store::reinstate_products` always publish after the scope change.

The publication is bound to the exact project/engineering/manufacturing versions the edit committed
(the engineering and manufacturing generations are 0 on a project that has no such record yet),
uses the default draft revision rule, locks the engineering/manufacturing record rows `FOR SHARE`,
and is returned in `ProjectWriteOutcome.draft`. An edit whose draft cannot be resolved writes nothing:
the whole transaction rolls back. Missing inputs are issues on an `incomplete` result, not errors.

No other write publishes automatically. Engineering, manufacturing, costing, reference-data and
commercial writes leave the previous draft result in place, and the report marks it stale. A
publication never mutates an accepted baseline, an earlier snapshot or a saved scenario; a saved
scenario that pinned an older result is flagged instead.

## Refusals

The version 1 `ApiErrorBody` shape is unchanged and the version 2 error-code set stays closed. A
`DependencyRefusal` maps to exactly one version 1 `ApiErrorCode`; the stable refusal code is the
prefix of the client-safe message (`<code>: <detail>`). `error_code()` is authoritative:

| Code | HTTP status / error code | Refusal | When | Message names |
|---|---|---|---|---|
| `unknown_update` | `404` `not_found` | `UnknownUpdate` | The named logical record or the named published version does not exist. | Nothing: the version 1 not-found body carries its generic message, like every other missing resource. |
| `stale_update` | `409` `conflict` | `StaleUpdate` | The named version is no longer the newest published version of its logical record. | Kind, record, named and current version. |
| `unsupported_adoption` | `422` `invalid` | `UnsupportedAdoption` | A routing-template or feasibility-rule-table update is requested with action `adopted`. | Kind; the owning package performs the change. |
| `no_pending_dependency` | `409` `conflict` | `NoPendingDependency` | The project already resolves the named version, so there is nothing to decide. | Kind, record, version. |
| `duplicate_decision` | `409` `conflict` | `DuplicateDecision` | An identical decision row is already recorded. | Action, kind, record, version. |
| `stale_project_version` | `409` `conflict` | `StaleProjectVersion` | `expected_version` is not the current project record version. | Expected and current version. |
| `stale_draft_generation` | `409` `conflict` | `StaleDraftGeneration` | A publication's expected project/engineering/manufacturing generation moved. | The stale reasons. |
| permission | `403` `forbidden` | (service) | The identity lacks the Engineer permission set. | No data. |
| unknown project | `404` `not_found` | (service) | The project does not exist. | No data. |
| unauthenticated | `401` `unauthorized` | (service) | No bearer token. | No data. |

A request body that the shared JSON extractor cannot deserialize is rejected **before** any handler
runs: HTTP `400` with a plain-text body, not an `ApiErrorBody`, exactly as on every other version 2
route. This contract does not change the extractor or the version 1/2 transport rules.

## HTTP routes

Base path `/v2`; a bearer identity with the Engineer permission set from the version 2 permissions
contract, bounded requests exactly as version 1. Handler refusals use the version 1 `ApiErrorBody`
shape and codes as mapped above.

| Method | Path | Request | Success | Body |
|---|---|---|---|---|
| `GET` | `/v2/shared-updates` | — | `200` | `SharedUpdateList` |
| `GET` | `/v2/dependency-projects?kind=&record_id=` | — (optional filter pair, both or neither) | `200` | `DependencyProjectList` |
| `GET` | `/v2/projects/{project_id}/dependency-report` | — | `200` | `ProjectDependencyReport` |
| `POST` | `/v2/projects/{project_id}/dependency-decisions` | `RecordDependencyDecisions` | `201` on a fresh write and on an identical replay of a recorded request id | `DependencyDecisionReceipt` |
| `POST` | `/v2/projects/{project_id}/draft-recalculations` | `RecalculateDraft` | `201` on a fresh write and on an identical replay | `DraftPublication` |

The two `GET` routes are pure reads and append no row. The `201`-for-both convention matches the
other append routes (for example the SB-19 activation route): the receipt is the recorded answer and
a replay never writes twice. Unknown dependency-project filter values are `422`; an unknown project
id is `404`.

## Persistence

One new migration, `services/solidbom-server/migrations/0012_dependency_decisions.sql` (version 12,
the next number after SB-19's `0011_accepted_baselines.sql`), adds one append-only table:

- `dependency_decisions` (`decision_id` uuid primary key, `project_id` uuid not null referencing
  `project_records (project_id)` on delete cascade (the version 2 project record; the version 1
  `projects` table is a different, prototype-only table), `sequence` bigint not null, `kind` text not null,
  `record_id` uuid not null, `from_version_id` uuid, `to_version_id` uuid not null, `action` text not
  null checked in (`acknowledged`, `adopted`), `decided_by` text not null, `decided_at` timestamptz
  not null default now()), with `unique (project_id, sequence)` and the index
  `dependency_decisions_project` on `(project_id, sequence desc)`.

No code path updates or deletes a row: the table is append-only and `sequence` is strictly increasing
per project. The idempotent receipt lives in the existing shared `requests` table. No project
document field changes, and no existing table changes: app-configuration adoption is this table plus
the resolution refinement above, not a new project field. The project record, engineering,
manufacturing, feasibility, reference-data, costing, tooling and commercial tables are unchanged.

## Validation the input refuses

`validate()` refuses the following before a value reaches a store or a client (the contract tests
assert each one):

- a zero project version, decision sequence, published-record version or pending count, a nil
  identity, or a decision `from_version_id` equal to its `to_version_id` (a zero engineering or
  manufacturing record generation is legal and means the project has no such record yet);
- a `SharedUpdate.previous_version_id` equal to its own `version_id`;
- an unsorted, duplicated or oversized catalog, dependency, target-id, project, flagged-scenario or
  kind list;
- a dependency whose `referenced_version` is not strictly older than its `available_version`;
- a report whose decisions are not newest first with unique sequences, or that names another project;
- a draft status whose `stale` flag disagrees with its reasons, an `input_hash` that is not 64
  lower-case hexadecimal characters, or stale reasons that are duplicated or unsorted;
- a flagged scenario with no stale reason, an unstable reason code, a blank or over-long reason
  message, or reasons that are duplicated or unsorted;
- a blank, over-long or control-character request id; an empty, over-bounded or duplicated decision
  item (`(kind, record_id)`); a decision receipt without a decision, with duplicate sequences, out of
  canonical order or whose facts do not match its decisions one-for-one;
- an adopted decision or adopted fact naming a flagged-only kind;
- a recalculation with a zero expected generation, a nil selection id, or selections that are not
  ascending by product id and unique;
- a publication with a nil snapshot id, a zero snapshot version, a blank calculator version or a
  non-canonical dependency/scenario list;
- a refusal whose ids are nil, whose named and current versions are equal, whose
  `unsupported_adoption` kind is adoptable, or whose stale-generation reasons are empty or unsorted.

## Compatibility

- This contract is additive to API version 2. The version 1 and version 2 routes, wire names,
  meanings, statuses, error codes, idempotency rules and stored documents are unchanged;
  `SCHEMA_VERSION` stays 1 and `API_VERSION` stays 2.
- `ProjectWriteOutcome` gains one optional `#[serde(default)] draft` field. An outcome written before
  this contract has no field and reads as `None`; an older client tolerates the unknown field. No
  existing project write changes meaning.
- One append-only table and one migration are added; the shared reference-data, project, engineering,
  manufacturing, feasibility, costing, tooling and commercial tables are unchanged. Historical
  versions, snapshots, accepted baselines and scenarios stay readable under the version that wrote
  them.
- **SB-16 app-configuration resolution refinement.** SB-16 published that a baseline resolves the
  latest published app configuration. This contract narrows that rule only for a project that has
  adopted an app-configuration version: the latest adopted decision is the pin, then the newest
  snapshot's recorded configuration, then the latest published configuration. A project with no
  adopted decision and no snapshot keeps the published latest-version behaviour, and every SB-16 wire
  type, route and error code is unchanged. This is an intentional refinement owned by SB-20, recorded
  here and in the registry; changing it later is a contract change with the affected-consumer list.
- Adding an optional field or a route is compatible. Changing a field meaning, a wire name, a kind, a
  decision action, a route, a status, an error code, the acknowledgement-is-not-adoption rule, the
  adoption-does-not-recalculate rule, the publication generation binding, the automatic-publication
  trigger set or a validation rule that previously accepted a write is breaking and needs a new
  contract version, the affected-consumer list and coordinator approval.
- **Increment boundary.** This increment publishes the contract and its service surface only. The
  desktop engine commands, controller state and any UI are a later increment of SB-20; this contract
  claims no UI string, locale file or screenshot. Adoption of a routing-template or
  feasibility-rule-table update never happens here: those kinds are flagged only, the owning package
  performs the change (SB-15 routing write, SB-14 re-recorded assessment) and the dependency
  disappears once the record pins the new version.

## What is intentionally not in this increment

- Desktop surface: no typed engine commands, no controller state, no egui panel. The package stays
  `in_progress` after this increment.
- Silent background mutation, offline sync, customer quotation generation and export (SB-21 owns
  export; SB-22 owns update recovery).
- Routing-template and feasibility-rule-table adoption through this route; the owning packages keep
  their own write/re-record paths.
- Any change to an existing contract's types, routes, meanings or statuses, and any change to the
  SB-16 arithmetic.
- Concurrent-editing leases or multi-user adoption coordination beyond the expected-version check.

## Handoff to later packages

- **SB-21 (internal exports):** a draft publication and a draft result name the exact project,
  engineering and manufacturing versions they read and the immutable SB-16 `calculation_id` /
  `input_hash` / `calculator_version`; a flagged scenario names the baseline and project version it
  pinned and its stale reasons. Exports must label a draft, incomplete or stale result as such and
  never present an outdated cost as current or silently use today's shared data.
- **SB-22 (update recovery):** the typed refusals and the acknowledgement-vs-adoption split are the
  update-recovery vocabulary. A failed adoption writes nothing, an obsolete job completion or a late
  response cannot overwrite a newer state because every write names its expected generation, and a
  dependency stays visible (and acknowledged, but pending) until it is adopted.
- **Desktop increment:** the update/adoption surface reads `SharedUpdateList`,
  `ProjectDependencyReport` and `DependencyProjectList`, sends `RecordDependencyDecisions` with the
  current `expected_version`, and sends `RecalculateDraft` with all three expected versions; it shows
  the acknowledgement/adoption difference, the stale markers and the `ProjectWriteOutcome.draft`
  publication of a forecast or scope edit.
- **Independent verification:** re-run the contract crate's `adoption` tests and the service
  `tests/adoption.rs` domain and route cases against the committed tree; verify that reading changes
  no row, that a failed adoption and a stale generation write nothing, that adoption plus
  recalculation creates a new result with the old one byte-identical, and that no app-configuration
  update is used before it is adopted.
