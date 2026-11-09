# SolidBOM development service

The central persistence and calculation API for the SolidBOM desktop prototype (packages SB-03,
SB-09, SB-10 and SB-11). It stores the original STEP bytes, append-only project versions, append-only calculation
snapshots, the append-only shared company reference-data chains and the append-only project record
with its deliveries and part versions in PostgreSQL, resolves the
selected material and rate versions itself, and runs the pure direct-cost calculator from
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
| `engineer` | project work: sources, project versions, reference-data reads, calculations and reopening |
| `administrator` | shared records: material catalog, cost-center rates, catalog components and app cost configuration |

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

The engineering review routes also require the Engineer permission set:

| Method | Path | Request | Success | Body |
|---|---|---|---|---|
| `GET` | `/v2/projects/{project_id}/engineering` | — | `200` | `EngineeringRecordView` |
| `GET` | `/v2/projects/{project_id}/engineering/versions/{version}` | — | `200` | `EngineeringRecordView` |
| `POST` | `/v2/projects/{project_id}/engineering/analyses` | `RegisterAnalysis` | `201` | `AnalysisReceipt` |
| `POST` | `/v2/projects/{project_id}/engineering/components` | `GroupSolids` | `200` | `EngineeringWriteOutcome` |
| `POST` | `/v2/projects/{project_id}/engineering/component-parts` | `AssociateComponentPart` | `200` | `EngineeringWriteOutcome` |
| `POST` | `/v2/projects/{project_id}/engineering/measurements` | `CompleteMeasurement` | `200` | `EngineeringWriteOutcome` |
| `POST` | `/v2/projects/{project_id}/engineering/confirmations` | `ConfirmParts` | `200` | `EngineeringWriteOutcome` |


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
version/idempotency checks and failed-delivery retention.

Server-only gates, run separately from the root `cargo xtask ci`:

```sh
cargo fmt --manifest-path services/solidbom-server/Cargo.toml -- --check
cargo clippy --manifest-path services/solidbom-server/Cargo.toml --all-targets --locked -- -D warnings
cargo build --manifest-path services/solidbom-server/Cargo.toml --locked
```

## Versions and known limits

- HTTP API version 2 (`solidbom_contract::API_VERSION`), with the version 1 and version 2 contract
  documents published by packages SB-01, SB-09 and SB-10, and the version 2 project-record routes
  served under `/v2`.
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
- The service has no separate repository CI workflow yet; the commands above are the acceptance
  run. Wiring PostgreSQL tests into GitHub Actions is a coordinator-reserved root-CI change.
