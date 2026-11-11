# SolidBOM development service

The central persistence and calculation API for the SolidBOM desktop prototype (packages SB-03,
SB-09, SB-10, SB-11, SB-13, SB-14, SB-15, SB-16, SB-17 and SB-18). It stores the original STEP bytes, append-only project
versions, append-only calculation snapshots, the append-only shared company reference-data chains,
the append-only project record with its deliveries and part versions, the reviewed engineering
record, the sourcing and manufacturing record with its versioned routing templates, and the
feasibility rule-table chain and project feasibility record in PostgreSQL, resolves the selected
material and rate versions itself, and runs the pure direct-cost calculator from
`crates/solidbom-costing`. The desktop never calculates an authoritative cost itself and never sends
an absolute source path.

This directory is its own Cargo workspace. It is not part of the SolidCraft workspace, is not
built for WebAssembly, and its dependencies are not shipped in the desktop program.

## Prerequisites

- Rust 1.95 or newer.
- PostgreSQL 18, for example with Homebrew:

```sh
brew install postgresql@18
brew services start postgresql@18
```

## Configuration

The service reads its configuration from environment variables. The tokens are development
identities only; they are never logged, returned in a response or committed.

| Variable | Required | Default | Meaning |
|---|---|---|---|
| `SOLIDBOM_DATABASE_URL` | yes | — | PostgreSQL URL of the development database |
| `SOLIDBOM_TOKEN` | yes | — | the combined legacy development identity (`development`): Engineer and Administrator |
| `SOLIDBOM_ENGINEER_TOKEN` | no | — | an engineer-only development identity (`engineer`) |
| `SOLIDBOM_ADMIN_TOKEN` | no | — | an administrator-only development identity (`administrator`) |
| `SOLIDBOM_BIND` | no | `127.0.0.1:8080` | listen address; loopback is the prototype default |
| `SOLIDBOM_MAX_UPLOAD_BYTES` | no | `67108864` (64 MiB) | request body limit, a resource limit, not a supported-file claim |
| `SOLIDBOM_REQUEST_TIMEOUT_MS` | no | `25000` | bound on one protected request; a timeout is a retryable `internal` |
| `SOLIDBOM_DELAY_MS` | no | `0` | delay every protected request; only for controlled responsiveness measurements |

Every token is at least 16 characters and all configured tokens must be distinct; the service
refuses to start otherwise. `SOLIDBOM_TOKEN` keeps an existing single-token setup working: that one
identity holds both permission sets. Add the optional tokens to exercise the two separate sets.

A remote development endpoint needs a TLS-terminating proxy in front of the service; the service
itself speaks plain HTTP on loopback.

## Permissions

One organization per deployment. Two permission sets exist, and one identity can hold both. The
service decides every request; a hidden button in a client is never the check.

| Permission set | Allows |
|---|---|
| `engineer` | project work: sources, project versions, reference-data reads, calculations, reopening, engineering review and manufacturing definitions |
| `administrator` | shared records: material catalog, cost-center rates, catalog components, app cost configuration and routing templates |

`GET /v1/me` answers the caller's identity and permission sets for any authenticated caller. A
missing or unknown token is `401 unauthorized`; a known identity without the route's permission set
is `403 forbidden`.

## Start, migrate and seed

The service applies its migrations on start, so start is also the migration and seed command:

```sh
createdb solidbom_dev
export SOLIDBOM_DATABASE_URL="postgres://$(whoami)@127.0.0.1:5432/solidbom_dev"
export SOLIDBOM_TOKEN="<at least 16 characters; never commit it>"
export SOLIDBOM_ENGINEER_TOKEN="<a different token; optional>"
export SOLIDBOM_ADMIN_TOKEN="<another different token; optional>"
export SOLIDBOM_BIND="127.0.0.1:8099"
cargo run --manifest-path services/solidbom-server/Cargo.toml --locked
```

- `migrations/0001_initial.sql` creates `sources`, `projects`, append-only `project_versions`,
  `material_versions`, `rate_versions`, append-only `calculations` and the accepted-request table.
- `migrations/0002_reference_seed.sql` inserts the deterministic reference fixtures: two materials
  with density 2.7 g/cm³ and price 20 CNY/kg, and two rates at 120 CNY/h. They are explicit test
  fixtures, not company defaults. The migration is idempotent by id.
- `migrations/0003_shared_data.sql` turns materials and rates into append-only version chains
  (logical id, version, `supersedes`, source/version reference, explicit units, purpose, nullable
  price/rate) and adds `catalog_component_versions`, `app_config_versions` and append-only
  `reference_changes`. The seeded rows stay valid: their logical id is their version id and they are
  version 1. No numeric company default is invented.
- `migrations/0004_projects_deliveries.sql` creates `project_records` and append-only
  `project_record_versions` for the version 2 project record (business details, products,
  deliveries, revisions, shared part versions and quotation events).
- `migrations/0005_engineering_review.sql` adds `engineering_records` and append-only
  `engineering_record_versions` for the reviewed engineering record, and `engineering_analyses`
  for the immutable exact-analysis evidence that grouping writes validate against.
- `migrations/0006_sourcing_manufacturing.sql` adds `manufacturing_records` and append-only
  `manufacturing_record_versions` for the project's sourcing and manufacturing definitions, and
  `routing_templates` with append-only `routing_template_versions` for the reusable company
  routing templates. Applications copy a template version; they never edit it.
- `migrations/0007_catalog_match_attributes.sql` adds the optional `attributes` array to
  `catalog_component_versions`: the administrator-declared values catalog matching compares
  against. It is empty for every existing row and is validated by the contract on write and read.
  Recorded suggestions and confirmed catalog references stay in the engineering record document,
  never in the shared catalog rows.
- `migrations/0008_feasibility.sql` adds the append-only Administrator-maintained
  `feasibility_rule_tables`/`feasibility_rule_versions` chain and the append-only
  `feasibility_records`/`feasibility_record_versions` project record for recorded assessments and
  justified exceptions. No rule data is seeded: every transcribed limit and its source reference
  enters through the publish route.
- `migrations/0009_costing_baseline.sql` adds the append-only `baseline_calculations` snapshots
  beside the project record: one immutable row per calculation, naming the exact project,
  engineering and manufacturing record versions it read, plus the canonical input hash, the
  calculator version and the resolved inputs and explained outputs. The project's costing settings
  (representative volume policy, saved manual exchange rates and purchase price tiers) live in the
  additive `costing` field of the project document, so an accepted settings write is an ordinary
  project record version and this migration adds no settings table.
- `migrations/0010_commercial_scenarios.sql` adds the append-only `commercial_scenarios` version
  chain: one immutable row per saved scenario version, naming the pinned baseline snapshot and
  project version, the manual prices, the tooling recovery assumptions and the tooling productivity
  treatment. The project's commercial settings (EBIT target and annual productivity rate) live in
  the additive `commercial` field of the project document, so an accepted settings write is an
  ordinary project record version and this migration adds no settings table. A comparison stores
  nothing.
- Re-running the service is idempotent: SQLx records applied migrations in `_sqlx_migrations`.
- To start from a fresh development state, drop and recreate only the explicitly designated
  development database (`dropdb solidbom_dev && createdb solidbom_dev`) while the service is
  stopped. Never run that against an unknown database.

`GET /health` answers `{"ok":true}` without a token, for readiness and restart checks. `SIGINT`
and `SIGTERM` stop the service gracefully, so a completed write is durable when the desktop
reopens the project.

## Routes

All routes below except `/health` require `Authorization: Bearer <token>`. The published contracts
are `docs/solidbom/contracts/v1/` and `docs/solidbom/contracts/v2/shared-data-and-permissions.md`.

| Method | Path | Permission | Request | Success | Body |
|---|---|---|---|---|---|
| `GET` | `/health` | public | — | `200` | `{"ok":true}` |
| `GET` | `/v1/me` | any | — | `200` | `Identity` |
| `PUT` | `/v1/sources/{sha256}?name=…` | engineer | raw STEP bytes | `200` | `SourceRef` |
| `GET` | `/v1/sources/{source_id}` | engineer | — | `200` | raw bytes, `application/step`, `x-solidbom-sha256` |
| `GET` | `/v1/reference-data` | engineer | — | `200` | `ReferenceData` |
| `POST` | `/v1/projects` | engineer | `SaveDraft` with `expected_version` 0 | `201` | `SaveReceipt` |
| `GET` | `/v1/projects/{project_id}` | engineer | — | `200` | `ProjectView` |
| `PUT` | `/v1/projects/{project_id}/draft` | engineer | `SaveDraft` | `200` | `SaveReceipt` |
| `POST` | `/v1/projects/{project_id}/calculations` | engineer | `CalculateRequest` | `201` | `CalculationSnapshot` |
| `GET` | `/v1/calculations/{calculation_id}` | engineer | — | `200` | `CalculationSnapshot` |
| `GET` | `/v1/shared` | administrator | — | `200` | `SharedDataView` |
| `POST` | `/v1/shared/materials/{material_id}/versions` | administrator | `CreateMaterialVersion` | `201` | `MaterialVersionPublished` |
| `POST` | `/v1/shared/cost-centers/{cost_center_id}/versions` | administrator | `CreateRateVersion` | `201` | `RateVersionPublished` |
| `POST` | `/v1/shared/components/{component_id}/versions` | administrator | `CreateComponentVersion` | `201` | `ComponentVersionPublished` |
| `POST` | `/v1/shared/app-config/{config_id}/versions` | administrator | `CreateAppConfigVersion` | `201` | `AppConfigVersionPublished` |
| `GET` | `/v1/feasibility-rules` | engineer | — | `200` | `FeasibilityRulesView` |
| `POST` | `/v1/shared/feasibility-rules/{rule_table_id}/versions` | administrator | `CreateFeasibilityRuleTableVersion` | `201` | `FeasibilityRuleTablePublished` |

A publish route's path id must equal the logical id in its body. `GET /v1/shared` lists every
published material, rate, component and app-configuration version plus the newest reference-change
facts: its `app_config_versions` field is the complete configuration chain, oldest version first,
while `app_config` is the latest configuration. Example (no secret is printed):

The version 2 project-record routes are the service side of
`docs/solidbom/contracts/v2/projects-deliveries.md`. They are additive: the version 1 routes above
keep their meaning, and version 2 publishes no activation route (activation belongs to SB-19).

The engineering review and flat grouping routes are the service side of
`docs/solidbom/contracts/v2/engineering-review-and-grouping.md`. They are additive to version 2:
reviewed component associations are bound to one registered exact analysis, reviewed volumes,
classifications and confirmed-label metadata are stored beside the project record, and no route
trains a model, confirms from a prediction, activates a revision or recalculates a cost.

The suggestions route is the service side of the SB-13 recognition and catalog suggestion contract.
A recorded suggestion is a proposal: a classification prediction is stored beside the confirmed
label without changing it, and a catalog candidate carries the catalog's resolved values but never
supplies a costing input. Only the confirmation route records the engineer's confirmed reference,
and an optional project override is kept separately with its own reason and reviewer.

The project-record routes require the Engineer permission set introduced by the version 2
permissions contract; an administrator-only identity is refused with `403 forbidden`.

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

The sourcing and manufacturing routes are the service side of
`docs/solidbom/contracts/v2/sourcing-and-manufacturing.md`. The template routes extend `/v1`: an
administrator publishes an immutable template version, while an engineer and an administrator can
both list the current template versions and read one exact version. The project manufacturing
routes extend `/v2` and require the Engineer permission set. Applying a template copies its
operations into the project routing; editing the copy, or editing the template, never changes the
other. There is no route that activates a revision or evaluates a cost.

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

The engineering review routes also require the Engineer permission set:

| Method | Path | Request | Success | Body |
|---|---|---|---|---|
| `GET` | `/v2/projects/{project_id}/engineering` | — | `200` | `EngineeringRecordView` |
| `GET` | `/v2/projects/{project_id}/engineering/versions/{version}` | — | `200` | `EngineeringRecordView` |
| `POST` | `/v2/projects/{project_id}/engineering/analyses` | `RegisterAnalysis` | `201` | `AnalysisReceipt` |
| `POST` | `/v2/projects/{project_id}/engineering/components` | `GroupSolids` | `200` | `EngineeringWriteOutcome` |
| `POST` | `/v2/projects/{project_id}/engineering/component-parts` | `AssociateComponentPart` | `200` | `EngineeringWriteOutcome` |
| `POST` | `/v2/projects/{project_id}/engineering/measurements` | `CompleteMeasurement` | `200` | `EngineeringWriteOutcome` |
| `POST` | `/v2/projects/{project_id}/engineering/suggestions` | `RecordSuggestions` | `200` | `EngineeringWriteOutcome` |
| `POST` | `/v2/projects/{project_id}/engineering/confirmations` | `ConfirmParts` | `200` | `EngineeringWriteOutcome` |

The feasibility routes are the service side of
`docs/solidbom/contracts/v2/feasibility-rules-and-exceptions.md`. They are additive to version 2:
Administrator-published structured rule tables are evaluated by the pure contract evaluator against
the exact reviewed tube evidence, and an Engineer may accept a justified exception for a recorded
failing, conflicting or missing-limit finding. No route invents a limit, resolves a conflict by
precedence, activates a revision or recalculates a cost. The rule table routes require the Engineer
permission for reads and the Administrator permission for publishes; the project routes require the
Engineer permission set.

| Method | Path | Request | Success | Body |
|---|---|---|---|---|
| `GET` | `/v2/projects/{project_id}/feasibility` | — | `200` | `FeasibilityRecordView` |
| `GET` | `/v2/projects/{project_id}/feasibility/versions/{version}` | — | `200` | `FeasibilityRecordView` |
| `POST` | `/v2/projects/{project_id}/feasibility/assessments` | `RecordFeasibilityAssessment` | `201` | `FeasibilityAssessmentOutcome` |
| `POST` | `/v2/projects/{project_id}/feasibility/exceptions` | `AcceptFeasibilityException` | `200` | `FeasibilityWriteOutcome` |
| `POST` | `/v2/projects/{project_id}/feasibility/exceptions/{exception_id}/closures` | `CloseFeasibilityException` | `200` | `FeasibilityWriteOutcome` |

The costing and baseline routes are the service side of
`docs/solidbom/contracts/v2/shared-demand-and-manufacturing-costs.md`. They are additive to version
2 and require the Engineer permission set: the settings route appends one project record version,
the calculation route resolves the exact project, engineering and manufacturing record versions and
the published reference data into the portable calculator inputs and appends one immutable snapshot,
and the two reads return the snapshot and its bounded newest-first history. An identical retry
returns the recorded outcome or snapshot; a changed payload under the same request id is a conflict.
No route rewrites a snapshot, activates a revision, accepts a baseline or prices a scenario.

| Method | Path | Request | Success | Body |
|---|---|---|---|---|
| `PUT` | `/v2/projects/{project_id}/costing` | `UpdateProjectCosting` | `200` | `ProjectWriteOutcome` |
| `POST` | `/v2/projects/{project_id}/baseline-calculations` | `CalculateBaseline` | `201` | `BaselineSnapshot` |
| `GET` | `/v2/projects/{project_id}/baseline-calculations` | — | `200` | `BaselineCalculationList` |
| `GET` | `/v2/projects/{project_id}/baseline-calculations/{calculation_id}` | — | `200` | `BaselineSnapshot` |

The tooling settings and recovery routes are the service side of
`docs/solidbom/contracts/v2/tooling-investment-and-recovery.md`. They are additive to version 2 and
require the Engineer permission set: the settings route appends one project record version, and the
recovery route is a read-only calculation over one immutable baseline snapshot and the tooling
settings of the exact project record version that snapshot names, so it stores nothing and appends
no project version. An identical settings retry returns the recorded outcome; a changed payload
under the same request id is a conflict. Tooling adds no migration and no table: the settings are
the additive `tooling` field of the project document, and the recovery evaluation is read-only.

| Method | Path | Request | Success | Body |
|---|---|---|---|---|
| `PUT` | `/v2/projects/{project_id}/tooling` | `UpdateProjectTooling` | `200` | `ProjectWriteOutcome` |
| `POST` | `/v2/projects/{project_id}/tooling-recovery` | `EvaluateToolingRecovery` | `200` | `ToolingOutcome` |

The commercial settings, scenario and comparison routes are the service side of
`docs/solidbom/contracts/v2/commercial-scenarios.md`. They are additive to version 2 and require the
Engineer permission set: the settings route appends one project record version, the create and edit
routes append one immutable scenario version (the edit checks the expected version under an advisory
lock), the list returns only the current version of every scenario, and the comparison route is a
read-only calculation over one immutable baseline snapshot and the project record version it
recorded, so it stores nothing. An identical write retry returns the recorded outcome; a changed
payload under the same request id and a stale expected version are conflicts.

| Method | Path | Request | Success | Body |
|---|---|---|---|---|
| `PUT` | `/v2/projects/{project_id}/commercial` | `UpdateProjectCommercial` | `200` | `ProjectWriteOutcome` |
| `PUT` | `/v2/projects/{project_id}/commercial-scenarios` | `CreateCommercialScenario` | `200` | `CommercialScenarioWriteOutcome` |
| `PUT` | `/v2/projects/{project_id}/commercial-scenarios/{scenario_id}` | `UpdateCommercialScenario` | `200` | `CommercialScenarioWriteOutcome` |
| `GET` | `/v2/projects/{project_id}/commercial-scenarios` | — | `200` | `CommercialScenarioList` |
| `POST` | `/v2/projects/{project_id}/commercial-comparisons` | `CompareCommercialScenarios` | `200` | `CommercialScenarioComparison` |

```sh
export SOLIDBOM_ENDPOINT="http://127.0.0.1:8099"
export SOLIDBOM_TOKEN="<the same token>"
export SOLIDBOM_ADMIN_TOKEN="<an administrator token>"
curl -fsS -H "Authorization: Bearer $SOLIDBOM_TOKEN" "$SOLIDBOM_ENDPOINT/v1/me"
curl -fsS -H "Authorization: Bearer $SOLIDBOM_TOKEN" "$SOLIDBOM_ENDPOINT/v1/reference-data"
curl -fsS -H "Authorization: Bearer $SOLIDBOM_ADMIN_TOKEN" "$SOLIDBOM_ENDPOINT/v1/shared"
```

## Writes, retries and conflicts

- Every write is keyed by its body `request_id` plus a canonical payload hash. An identical retry
  returns the recorded receipt, snapshot or published version before the current-version check, so a
  committed write whose reply was lost is safe to retry. A reused request id with a different
  payload is `409`.
- Request ids are bounded to 128 characters. A source name is bounded to 255 characters.
- A draft save locks the project row, checks `expected_version` against the stored current version,
  validates the source reference and every selected material and rate version, then appends one
  immutable version and its receipt in the same transaction. A stale version is `409`; an unknown
  reference, a malformed contract or an oversized request id is `422`; neither writes anything.
- A publish names the logical record and `expected_version`. `0` creates version 1 of a new logical
  record; any other value must equal the record's current latest version, otherwise the write is
  `409`. The published version's `supersedes` is the current latest version id, so a concurrent
  publish cannot append two versions with one number. The new version and its `ReferenceChange` fact
  commit in one transaction; `GET /v1/shared` lists the newest 100 facts, newest first.
- A newer shared version never edits, recalculates or reinterprets an existing project version or
  calculation snapshot: both store the exact version ids and the resolved values they used.
  `GET /v1/reference-data` returns every published material, rate and component version, so a
  project may select any immutable version and a historical version stays readable after a newer one
  is published; `GET /v1/shared` adds the complete app-configuration chain. Only the latest app
  configuration is part of `ReferenceData`, because a project has not selected a configuration
  version yet.
- Missing money is `null`, never zero; an explicit `0` is the decimal string `"0"`. Both are stored
  and returned distinctly. A missing price or rate reaches the calculator as `None`, so a
  calculation names the missing input (`missing_material_price`, `missing_production_rate`) instead
  of subtracting zero.
- An app configuration is validated before anything is written: an ambiguous percentage base, a
  base that names the category itself or a circular chain, an unknown or repeated basis category, a
  duplicated category code, the same direct-cost subtotal included twice, a negative fixed amount
  or rate, and a direct-cost category selected for recurring EBIT are all `422 invalid` with a
  message naming the category.
- A calculation names one persisted `saved_version`; it never recalculates a moving current
  version. A later edit or recalculation appends a snapshot; it never rewrites an earlier one, so
  a historical result stays readable with its original inputs, input hash and calculator version.
- The calculator names a part by its resolved part version. The service maps that id to the
  reviewed definition id the client shows, in `Issue.definition_id`.
- A feasibility assessment names the exact reviewed tube components and one exact published
  rule-table version. The service verifies each component against the current engineering review
  (same revision, same analysis) and records the reviewed body set itself. Finding states are
  computed by the contract evaluator and retained with the assessment; a missing measurement,
  connector association or limit is `unresolved`, a conflicting overlapping limit is `conflict`, and
  neither is ever stored as a pass. A missing measurement or connector association cannot be
  excepted; a fail, conflict or missing-limit finding can carry one Engineer justification. An
  accepted exception becomes `review_required` when the supporting review or the current assessment
  changes; an Engineer closes an obsolete exception with a recorded reason (refused while its
  finding still fails), and readiness issues are computed on every read.
- One protected request is bounded by `SOLIDBOM_REQUEST_TIMEOUT_MS`. The client's 30-second
  transport timeout is above it, so a slow answer is an explicit retryable failure. The write may
  have committed; retry the same request id and payload.

Error responses are exactly `{"error":{"code":"…","message":"…"}}` with `unauthorized` (401),
`forbidden` (403), `not_found` (404), `conflict` (409), `invalid` (422) or `internal` (500, the only
retryable code). A body over the configured limit is `413`; malformed JSON, a missing required field
or a wrong content type is rejected by the HTTP extractors before any handler runs (plain-text `400`,
`422` and `415`). Errors and logs never contain a database error, a filesystem path or the token.

## Isolated tests

The integration tests need a disposable PostgreSQL database. Create one and pass it through
`SOLIDBOM_TEST_DATABASE_URL`:

```sh
createdb solidbom_test
export SOLIDBOM_TEST_DATABASE_URL="postgres://$(whoami)@127.0.0.1:5432/solidbom_test"
cargo test --manifest-path services/solidbom-server/Cargo.toml --locked
```

Each test creates its own random PostgreSQL schema in that database, runs the real migrations in
it, and drops only that schema when it finishes. Tests never reset or truncate the database they
connect to, and they never touch the development database. The persistence suite covers commit and
reconnect, stale-version and invalid-reference rejection, lost-reply retries (including two
concurrent identical creates), reused-id conflicts, historical snapshots, the shared version chains
and the reference-change facts, and the version 2 project record routes. The HTTP suite covers the
routes, the three development identities and their permission sets, upload bounds and hash
verification, malformed bodies, request timeouts, the SB-02 fixture vectors (including two cavities
and a reviewed manual volume), explicit zero versus missing money, source/input/result references
after reopening, and the version 2 project routes (create, read, delivery, stale conflict,
authentication and the absent activation route). The project suite covers partial deliveries,
revision separation and geometry confirmation, exclusion retention, missing forecasts,
version/idempotency checks and failed-delivery retention. The manufacturing suite covers the
shared-version sourcing rule, template adoption and copy isolation, the explicit cycle basis with
cavity conversion, missing-versus-zero cycle/rate/cavity/consumption values, assembly operation
ownership, removable attached cost lines and the idempotent/version-checked writes.

Server-only gates, run separately from the root `cargo xtask ci`:

```sh
cargo fmt --manifest-path services/solidbom-server/Cargo.toml -- --check
cargo clippy --manifest-path services/solidbom-server/Cargo.toml --all-targets --locked -- -D warnings
cargo build --manifest-path services/solidbom-server/Cargo.toml --locked
```

## Versions and known limits

- HTTP API version 2 (`solidbom_contract::API_VERSION`), with the version 1 and version 2 contract
  documents published by packages SB-01, SB-09, SB-10, SB-11 and SB-15, and the version 2
  project-record, engineering-review and manufacturing routes served under `/v2`.
- Database schema version is the migration set above; `_sqlx_migrations` records what a database
  has applied. A build refuses a database state it does not understand rather than coercing it.
- Calculator version `solidbom-direct-cost/1` (`solidbom_costing::calculator_version()`), written
  into every snapshot.
- One development organization and up to three development identities; no production
  authentication, multi-organization isolation or onboarding.
- Materials, rates and catalog components are recorded in CNY only and a version in another
  currency is refused. The app configuration carries its own three-letter currency code.
- An app-config chain is versioned; `GET /v1/shared` exposes every published configuration version
  (`app_config_versions`, oldest version first) and the latest one (`app_config`), so history stays
  readable after a newer version is published. The direct-cost calculator does not consume the
  category list yet: that is later-package scope (project EBIT and quotation scenarios). The
  configuration is exposed and validated here so a later package can read it.
- Reference-change facts are emitted, not consumed: SB-20 adds the project adoption/notification
  workflow. A published version never updates a project by itself.
- The prototype is online-only. There is no client write queue and no local authoritative cost.
- Annotations and a server-owned material-overhead policy version are deferred: version 1
  deliberately does not carry them on the wire or in a draft, so adding them needs a compatible
  contract extension (see the SB-03 execution record).
- Feasibility rule data is not seeded. The owner's supplied radius tables are transcribed limits with
  source references and applicability conditions; an Administrator publishes them, so no
  speculative starter rule set enters a deployment.
- Feasibility evidence is exact extracted tube geometry (SB-12). Manual tube-measurement completion
  and its review remain a separate supporting-input prerequisite; this increment accepts extracted
  evidence only. The desktop mapping from `TubeExtraction` into the portable evidence, the
  feasibility/readiness UI and its native evidence are the follow-on desktop increment.
- The service has no separate repository CI workflow yet; the commands above are the acceptance
  run. Wiring PostgreSQL tests into GitHub Actions is a coordinator-reserved root-CI change.
