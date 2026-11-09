# SolidBOM development service

The central persistence and calculation API for the SolidBOM desktop prototype (package SB-03)
and the multi-product project record (contract version 2, package SB-10). It stores the original
STEP bytes, append-only project versions and calculation snapshots, the append-only project record
with its deliveries and part versions in PostgreSQL, resolves the selected material and rate
versions itself, and runs the pure direct-cost calculator from `crates/solidbom-costing`. The desktop never calculates an
authoritative cost itself and never sends an absolute source path.

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

The service reads its configuration from environment variables. The token is a development
identity only; it is never logged, returned in a response or committed.

| Variable | Required | Default | Meaning |
|---|---|---|---|
| `SOLIDBOM_DATABASE_URL` | yes | — | PostgreSQL URL of the development database |
| `SOLIDBOM_TOKEN` | yes | — | the one development bearer token, at least 16 characters |
| `SOLIDBOM_BIND` | no | `127.0.0.1:8080` | listen address; loopback is the prototype default |
| `SOLIDBOM_MAX_UPLOAD_BYTES` | no | `67108864` (64 MiB) | request body limit, a resource limit, not a supported-file claim |
| `SOLIDBOM_REQUEST_TIMEOUT_MS` | no | `25000` | bound on one protected request; a timeout is a retryable `internal` |
| `SOLIDBOM_DELAY_MS` | no | `0` | delay every protected request; only for controlled responsiveness measurements |

A remote development endpoint needs a TLS-terminating proxy in front of the service; the service
itself speaks plain HTTP on loopback and provides only the single development identity.

## Start, migrate and seed

The service applies its migrations on start, so start is also the migration and seed command:

```sh
createdb solidbom_dev
export SOLIDBOM_DATABASE_URL="postgres://$(whoami)@127.0.0.1:5432/solidbom_dev"
export SOLIDBOM_TOKEN="<at least 16 characters; never commit it>"
export SOLIDBOM_BIND="127.0.0.1:8099"
cargo run --manifest-path services/solidbom-server/Cargo.toml --locked
```

- `migrations/0001_initial.sql` creates `sources`, `projects`, append-only `project_versions`,
  `material_versions`, `rate_versions`, append-only `calculations` and the accepted-request table.
- `migrations/0003_projects_deliveries.sql` creates `project_records` and append-only
  `project_record_versions` for the version 2 project record (business details, products,
  deliveries, revisions, shared part versions and quotation events).
- `migrations/0002_reference_seed.sql` inserts the deterministic reference fixtures: two materials
  with density 2.7 g/cm³ and price 20 CNY/kg, and two rates at 120 CNY/h. They are explicit test
  fixtures, not company defaults. The migration is idempotent by id.
- Re-running the service is idempotent: SQLx records applied migrations in `_sqlx_migrations`.
- To start from a fresh development state, drop and recreate only the explicitly designated
  development database (`dropdb solidbom_dev && createdb solidbom_dev`) while the service is
  stopped. Never run that against an unknown database.

`GET /health` answers `{"ok":true}` without a token, for readiness and restart checks. `SIGINT`
and `SIGTERM` stop the service gracefully, so a completed write is durable when the desktop
reopens the project.

## Routes

All routes below except `/health` require `Authorization: Bearer <token>`. The published version 1
contract is `docs/solidbom/contracts/v1/http-api.md`.

| Method | Path | Request | Success | Body |
|---|---|---|---|---|
| `GET` | `/health` | — | `200` | `{"ok":true}` |
| `PUT` | `/v1/sources/{sha256}?name=…` | raw STEP bytes | `200` | `SourceRef` |
| `GET` | `/v1/sources/{source_id}` | — | `200` | raw bytes, `application/step`, `x-solidbom-sha256` |
| `GET` | `/v1/reference-data` | — | `200` | `ReferenceData` |
| `POST` | `/v1/projects` | `SaveDraft` with `expected_version` 0 | `201` | `SaveReceipt` |
| `GET` | `/v1/projects/{project_id}` | — | `200` | `ProjectView` |
| `PUT` | `/v1/projects/{project_id}/draft` | `SaveDraft` | `200` | `SaveReceipt` |
| `POST` | `/v1/projects/{project_id}/calculations` | `CalculateRequest` | `201` | `CalculationSnapshot` |
| `GET` | `/v1/calculations/{calculation_id}` | — | `200` | `CalculationSnapshot` |

The version 2 project-record routes are the service side of
`docs/solidbom/contracts/v2/projects-deliveries.md`. They are additive: the version 1 routes above
keep their meaning, and version 2 publishes no activation route (activation belongs to SB-19).

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

Example (no secret is printed):

```sh
export SOLIDBOM_ENDPOINT="http://127.0.0.1:8099"
export SOLIDBOM_TOKEN="<the same token>"
curl -fsS "$SOLIDBOM_ENDPOINT/health"
curl -fsS -H "Authorization: Bearer $SOLIDBOM_TOKEN" "$SOLIDBOM_ENDPOINT/v1/reference-data"
```

## Writes, retries and conflicts

- Every write is keyed by its body `request_id` plus a canonical payload hash. An identical retry
  returns the recorded receipt or snapshot before the current-version check, so a committed write
  whose reply was lost is safe to retry. A reused request id with a different payload is `409`.
- Request ids are bounded to 128 characters. A source name is bounded to 255 characters.
- A draft save locks the project row, checks `expected_version` against the stored current version,
  validates the source reference and every selected material and rate version, then appends one
  immutable version and its receipt in the same transaction. A stale version is `409`; an unknown
  reference, a malformed contract or an oversized request id is `422`; neither writes anything.
- A calculation names one persisted `saved_version`; it never recalculates a moving current
  version. A later edit or recalculation appends a snapshot; it never rewrites an earlier one, so
  a historical result stays readable with its original inputs, input hash and calculator version.
- The calculator names a part by its resolved part version. The service maps that id to the
  reviewed definition id the client shows, in `Issue.definition_id`.
- One protected request is bounded by `SOLIDBOM_REQUEST_TIMEOUT_MS`. The client's 30-second
  transport timeout is above it, so a slow answer is an explicit retryable failure. The write may
  have committed; retry the same request id and payload.

Error responses are exactly `{"error":{"code":"…","message":"…"}}` with `unauthorized` (401),
`not_found` (404), `conflict` (409), `invalid` (422) or `internal` (500, the only retryable code).
A body over the configured limit is `413`; malformed JSON, a missing required field or a wrong
content type is rejected by the HTTP extractors before any handler runs (plain-text `400`, `422`
and `415`). Errors and logs never contain a database error, a filesystem path or the token.

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
concurrent identical creates), reused-id conflicts and historical snapshots. The HTTP suite covers
the routes, authentication, upload bounds and hash verification, malformed bodies, request
timeouts, the SB-02 fixture vectors (including two cavities and a reviewed manual volume), named
incomplete results, source/input/result references after reopening, and the version 2 project
routes (create, read, delivery, stale conflict, authentication and the absent activation route).
The project suite covers partial deliveries, revision separation and geometry confirmation,
exclusion retention, missing forecasts, version/idempotency checks and failed-delivery retention.

Server-only gates, run separately from the root `cargo xtask ci`:

```sh
cargo fmt --manifest-path services/solidbom-server/Cargo.toml -- --check
cargo clippy --manifest-path services/solidbom-server/Cargo.toml --all-targets --locked -- -D warnings
cargo build --manifest-path services/solidbom-server/Cargo.toml --locked
```

## Versions and known limits

- HTTP API version 1 (`solidbom_contract::API_VERSION`) plus the version 2 project-record routes,
  with the version 1 and version 2 contract documents under `docs/solidbom/contracts/`.
- Database schema version is the migration set above; `_sqlx_migrations` records what a database
  has applied. A build refuses a database state it does not understand rather than coercing it.
- Calculator version `solidbom-direct-cost/1` (`solidbom_costing::calculator_version()`), written
  into every snapshot.
- One development bearer identity; no production authentication or organizations.
- The prototype is online-only. There is no client write queue and no local authoritative cost.
- Annotations and a server-owned material-overhead policy version are deferred: version 1
  deliberately does not carry them on the wire or in a draft, so adding them needs a compatible
  contract extension (see the SB-03 execution record).
- The service has no separate repository CI workflow yet; the commands above are the acceptance
  run. Wiring PostgreSQL tests into GitHub Actions is a coordinator-reserved root-CI change.
