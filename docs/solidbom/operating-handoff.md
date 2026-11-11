# Operating handoff: local SolidCraft desktop, loopback SolidBOM service, PostgreSQL 18

Status: written for the SB-22 acceptance run on 2026-11-12 (Asia/Shanghai). Every command and
observation below was executed in that run against the integrated build; the raw outputs are in
[SB-22-operating-handoff.txt](execution/evidence/SB-22-operating-handoff.txt). The MVP gate itself
stays **Pending** (see [known limits](#known-limits-and-unrun-requirements)).

Paths under `/tmp/solidbom-run` are placeholders for a temporary directory **outside the
repository**; the acceptance run used `/tmp/sb22-ops`. The database names and loopback ports are the
values that run used; supply your own run-only tokens for a new run and keep all of them out of
tracked files.

- Branch: `agent/mvp-acceptance-handoff-262f3c774bf5`
- Integrated base commit: `4a8a811fb`; HEAD of this run: `8d9cd9296` (the SB-22 contract commit)
- Service binary: `services/solidbom-server/target/debug/solidbom-server`, SHA-256
  `1aa18cdc8a7631cf55f97af371ed8354c6b837ea85d55860800f979adc14fef2`
- Desktop binary: `target/debug/solidcraft`, SHA-256
  `53747397063175a23912e7aac8e92cdb3ac8772f2f37824ed884455a94f47fe6`
- Host of the run: macOS 26.7.1 on Apple M5 Pro (15 cores, 24 GiB), Rust 1.99.1, PostgreSQL 18.6
  on 127.0.0.1:5432

## 1. Deployment being operated

This handoff describes exactly one deployment, the one the confirmed prototype decisions accept
([contract v2 §1](contracts/v2/mvp-acceptance-and-operating-handoff.md)):

- a locally launched macOS **development build** of the SolidCraft desktop fork, built from the
  same commit as the service;
- the `services/solidbom-server` development service on **loopback (127.0.0.1)**, plain HTTP;
- **PostgreSQL 18** storage on the same development machine; the service applies its migrations
  on start.

It does not install packages, register a service manager, or describe production operations.
Development identities are created for one run only. Tokens and connection strings are supplied
through environment variables and are never written into tracked files, screenshots or pull
request text.

## 2. Prerequisites

- macOS on Apple silicon (the run used macOS 26.7.1 / Apple M5 Pro).
- Rust 1.95 or newer (the run used 1.99.1). The repository toolchain builds both workspaces.
- PostgreSQL 18 on the development machine, reachable on `127.0.0.1:5432` (the run used
  PostgreSQL 18.6; PostgreSQL was already installed and running, so the service README's Homebrew
  installation steps below were **not** re-run in the acceptance run):
  ```sh
  brew install postgresql@18
  brew services start postgresql@18
  export PATH="/opt/homebrew/opt/postgresql@18/bin:$PATH"
  ```
- `curl` and `psql`/`pg_dump`/`pg_restore` on the path. `psql` should be the PostgreSQL 18 client so
  the custom-format dump/restore versions match.

## 3. Build the service and the desktop app

Run from the repository root:

```sh
cargo build --manifest-path services/solidbom-server/Cargo.toml --locked
cargo build -p solidcraft --locked
```

Observed on this run: the service build finished with exit 0 in 0.29 s and produced
`services/solidbom-server/target/debug/solidbom-server` with the SHA-256 above; the desktop build
finished with exit 0 in 1 m 39 s and produced `target/debug/solidcraft`
(`./target/debug/solidcraft --version` prints `solidcraft 0.40.0-rc.2`).

The two binaries come from the same commit. Building the desktop app does not require the service
workspace to be running, and the service does not need the desktop build.

## 4. Start the SolidBOM service

### 4.1 Configuration

The service reads its configuration from environment variables (`services/solidbom-server/README.md`
is the reference for all of them). This deployment needs:

| Variable | Required | Value used here | Meaning |
|---|---|---|---|
| `SOLIDBOM_DATABASE_URL` | yes | `postgres://$(whoami)@127.0.0.1:5432/solidbom_sb22_ops` | development database |
| `SOLIDBOM_TOKEN` | yes | a run-only token, well over the 16-character minimum, **not tracked** | combined development identity (`development`): engineer + administrator |
| `SOLIDBOM_ENGINEER_TOKEN` | optional | a different run-only token, ≥16 characters | engineer-only identity (`engineer`) |
| `SOLIDBOM_ADMIN_TOKEN` | optional | a different run-only token, ≥16 characters | administrator-only identity (`administrator`) |
| `SOLIDBOM_BIND` | optional (default `127.0.0.1:8080`) | `127.0.0.1:8396` | loopback listen address |

Every token must be at least 16 characters and all configured tokens must be distinct; the service
refuses to start otherwise (observed: `set SOLIDBOM_TOKEN`, `development token 1 must be at least
16 characters`, `the development tokens must be distinct`, exit code 1). Only one organization is
served per deployment and there are at most three development identities.

### 4.2 Create the database, start, confirm health

```sh
export PATH="/opt/homebrew/opt/postgresql@18/bin:$PATH"
createdb -h 127.0.0.1 -p 5432 solidbom_sb22_ops

mkdir -p /tmp/solidbom-run
export SOLIDBOM_DATABASE_URL="postgres://$(whoami)@127.0.0.1:5432/solidbom_sb22_ops"
export SOLIDBOM_TOKEN="<run-only combined token, at least 16 characters>"
export SOLIDBOM_ENGINEER_TOKEN="<a different run-only token>"
export SOLIDBOM_ADMIN_TOKEN="<another different run-only token>"
export SOLIDBOM_BIND="127.0.0.1:8396"
nohup services/solidbom-server/target/debug/solidbom-server \
  > /tmp/solidbom-run/service.log 2>&1 &
echo $! > /tmp/solidbom-run/service.pid

curl -fsS http://127.0.0.1:8396/health
```

Expected output:

```
solidbom-server: listening on 127.0.0.1:8396        (in /tmp/solidbom-run/service.log)
{"ok":true}                                        (curl /health, HTTP 200)
```

`GET /health` is public (no token) and is the service readiness/restart check. `lsof -nP
-iTCP:8396 -sTCP:LISTEN` shows the listener only on `127.0.0.1:8396`; the service is not exposed
off the machine. Identity and permission check with the tokens (the run used a combined token, an
engineer-only token and an administrator-only token; the service decides every request):

```sh
curl -fsS -H "Authorization: Bearer $SOLIDBOM_TOKEN" http://127.0.0.1:8396/v1/me
# {"identity_id":"5b090000-0000-4000-8000-000000000001","name":"development","permissions":["administrator","engineer"]}

curl -sS -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8396/v1/shared
# 401  {"error":{"code":"unauthorized","message":"a bearer token is required"}}
curl -sS -o /dev/null -w '%{http_code}\n' \
  -H "Authorization: Bearer $SOLIDBOM_ENGINEER_TOKEN" http://127.0.0.1:8396/v1/shared
# 403  {"error":{"code":"forbidden","message":"this identity does not hold the permission this route needs"}}
curl -sS -o /dev/null -w '%{http_code}\n' \
  -H "Authorization: Bearer $SOLIDBOM_ADMIN_TOKEN" http://127.0.0.1:8396/v1/shared
# 200  (administrator shared records)
```

### 4.3 Stop, restart, migrate

- Stop gracefully with `kill -TERM $(cat /tmp/solidbom-run/service.pid)` (observed: the process
  exits and releases the port); the service also handles `Ctrl-C`/SIGINT and stops gracefully per
  its README. Committed writes are durable.
- Start is also the migration command: start the service again with the same environment and it
  validates/applies migrations before listening. Re-running is idempotent.
- A start can fail before listening; see [failure recovery](#7-failure-recovery).

## 5. Start the SolidCraft desktop app

### 5.1 Configuration

The desktop reads its SolidBOM connection from two environment variables
(`apps/solidcraft/src/solidbom_client.rs`); without them the desktop still starts, and per the
approved prototype description it imports and analyses locally while saving and calculating report
that the service is needed (that unconfigured mode was not exercised in this run; the run always
configured both variables).

| Variable | Required | Value used here | Meaning |
|---|---|---|---|
| `SOLIDBOM_ENDPOINT` | yes, for central work | `http://127.0.0.1:8396` | service base URL (loopback, no trailing path) |
| `SOLIDBOM_TOKEN` | yes, for central work | the **combined** development token, ≥16 characters | the single identity the desktop uses; use the combined identity when the administrator surfaces are needed |
| `SOLIDCRAFT_CONTROL_TOKEN` | optional | a run-only 32-byte hex token | fixes the control-channel token for scripted checks; the app otherwise generates one |
| `SOLIDCRAFT_SETTINGS_DIR` | optional | `/tmp/solidbom-run/desktop-settings` | isolated settings directory, keeps a run from touching the user's preferences |
| `SOLIDCRAFT_CONTROL_DIR` | optional | `/tmp/solidbom-run/control` | isolated control token/discovery directory |

The control channel is on by default and listens on loopback only. `--control PORT` (or
`SOLIDCRAFT_CONTROL_PORT`) pins the port; without it the app uses 27878 or the next free port
(docs/control-protocol.md). The first control request on a connection must carry the token as a
top-level `"token"` member; later requests on the same connection may omit it.

### 5.2 Launch, confirm health, quit

```sh
export SOLIDBOM_ENDPOINT="http://127.0.0.1:8396"
export SOLIDBOM_TOKEN="<the same combined development token>"
export SOLIDCRAFT_CONTROL_TOKEN="<run-only 32-byte hex token, e.g. openssl rand -hex 32>"
export SOLIDCRAFT_SETTINGS_DIR=/tmp/solidbom-run/desktop-settings
export SOLIDCRAFT_CONTROL_DIR=/tmp/solidbom-run/control
nohup ./target/debug/solidcraft --control 8398 > /tmp/solidbom-run/desktop.log 2>&1 &
echo $! > /tmp/solidbom-run/desktop.pid
```

Startup output observed:

```
SolidCraft graphics: Metal, MSAA 4x (requested 4x)
solidcraft: control server listening on 127.0.0.1:8398 (token in /tmp/solidbom-run/control/8398.token)
SolidCraft graphics: GPU tessellation ready; bodies are evaluated on the GPU from now on
```

**Health of the desktop pair.** Send JSON lines to the control port, first line with the token.
The acceptance run used this small Python client (the exact shape it executed); save it outside the
repository and run it:

```python
# /tmp/control_check.py  ->  python3 /tmp/control_check.py
import json, os, socket
port = 8398
token = os.environ["SOLIDCRAFT_CONTROL_TOKEN"]
sock = socket.create_connection(("127.0.0.1", port), timeout=10)
sock.settimeout(120)
stream = sock.makefile("rwb", buffering=0)
lines = [
    {"id": 1, "method": "ui.inspect", "params": {}, "token": token},
    {"id": 2, "method": "engine.execute", "params": {"command": "solidbom.shared", "params": {}}},
    {"id": 3, "method": "engine.execute", "params": {"command": "solidbom.shared", "params": {}}},
]
for line in lines:
    stream.write((json.dumps(line) + "\n").encode())
for _ in lines:
    print(stream.readline().decode().strip())
```

What it must answer:

```
id 1 -> {"ok":true,"result":{"workspace":"design","renderer":"gpu","adapter":"Metal · Apple M5 Pro",
         "documents":1,...}}                  # the app and its window/GPU are alive
id 2 -> {"ok":true,"result":{"pending":["shared_data"],"identity_name":null,...}}
id 3 -> {"ok":true,"result":{"identity_name":"development","permissions":["administrator","engineer"],
         "administrator":true,"materials":[...],"cost_centers":[...],...}}
         # the app read the identity and stored reference data through the service
```

`solidbom.shared` is asynchronous: the first call reports `pending:["shared_data"]` with empty
data, so poll or call it twice; the reply with a non-null `identity_name` proves the desktop
reached the service on `SOLIDBOM_ENDPOINT`.

If `SOLIDBOM_ENDPOINT`/`SOLIDBOM_TOKEN` are missing or the token is shorter than 16 characters, the
desktop still starts but cannot perform central work: per the desktop implementation
(`apps/solidcraft/src/solidbom_client.rs`) no SolidBOM host is installed, so `solidbom.state` reports
`needs_service: true` once a review needs the service and saving/calculating report that the service
is needed (behaviour documented by that code; this run always configured both variables). Use the
combined development token in `SOLIDBOM_TOKEN` when the administrator (shared reference data)
surface is needed; the desktop reads a single token.

Quit through the control channel (or close the window; `force` skips the unsaved-design prompt):

```python
# same client shape as above
{"id": 1, "method": "app.quit", "params": {"force": True}, "token": token}
# -> {"id":1,"ok":true,"result":{"asking":false,"quitting":true}}
```

Observed: the process exits, the control port closes and the per-port token/pid/discovery files are
removed.

### 5.3 Desktop workflow commands used by this handoff

The same control channel drives the reviewed workflow (all were executed in this run; use
`engine.execute` with the command name):

| Command | Purpose |
|---|---|
| `solidbom.import` `{"path": "<absolute path to .step>"}` | import an original STEP source for review; binds the review and creates/saves version 1 through the service |
| `solidbom.state` | review state: `phase`, `version`, `save`, `retryable`, `error`, `notice`, `needs_service` |
| `solidbom.parts` | reviewed part rows, ids, quantities and reviewed inputs |
| `solidbom.inputs.batch` `{"parts":[{...}], "material_overhead_rate":"0.05"}` | apply reviewed edits (leaves `save: "dirty"`) |
| `solidbom.save` | save the reviewed draft centrally |
| `solidbom.retry` | resubmit the exact failed request (same request id and payload) after a retryable failure |
| `solidbom.reopen` `{"project_id": "<uuid>"}` | reopen a saved project by id and verify its source |
| `solidbom.shared` | load the administrator shared records through the service |
| `app.quit` `{"force": true}` | quit |

Before quitting the app, save or explicitly discard pending edits: the desktop keeps unsaved edits
in the open session and offers an explicit Retry after a transient failure, but it does not persist
them across a process exit.

## 6. Database, migrations, seeding and owner references

### 6.1 Create the database and read the applied migration version

```sh
createdb -h 127.0.0.1 -p 5432 solidbom_sb22_ops

# start the service once (section 4) so the migrations apply
psql -h 127.0.0.1 -p 5432 -d solidbom_sb22_ops \
  -c "select version, description, success, installed_on from _sqlx_migrations order by version;"
psql -h 127.0.0.1 -p 5432 -d solidbom_sb22_ops \
  -Atc "select max(version) from _sqlx_migrations where success;"
```

Observed after the first start: 13 rows, all `success = t`, descriptions `initial`, `reference
seed`, `shared data`, `projects deliveries`, `engineering review`, `sourcing manufacturing`,
`catalog match attributes`, `feasibility`, `costing baseline`, `commercial scenarios`, `accepted
baselines`, `dependency decisions`, `machine hourly rates`; `max(version)` = **13**, matching the
embedded migration set whose last file is `0013_machine_hourly_rates.sql`.

- Migrations apply inside the service start, in order, once; re-running the service is idempotent
  because SQLx records them in `_sqlx_migrations`.
- A database whose migration bookkeeping does not match this build is refused, not coerced:
  observed message `solidbom-server: migrations failed: migration 1 was previously applied but has
  been modified` with exit code 1. Recovery is the migration-failure row in section 7 and the
  restore procedure in section 8.
- To reset a *disposable development database* while the service is stopped, drop and recreate only
  that explicitly named database and start again (`dropdb solidbom_sb22_ops && createdb
  solidbom_sb22_ops`). Never run that against a database you have not identified.

### 6.2 Development and reference data

The start seeds the explicit reference fixtures from `migrations/0002_reference_seed.sql`: two
materials (density 2.7 g/cm³, price 20 CNY/kg) and two rates (120 CNY/h), all at version 1. They
are test fixtures, not company defaults. Verify with:

```sh
curl -fsS -H "Authorization: Bearer $SOLIDBOM_ENGINEER_TOKEN" \
  http://127.0.0.1:8396/v1/reference-data
```

Further reference data is seeded through the administrator routes (never by editing the database or
the migration files). The two commands were exercised in this run:

```sh
# a new material version (expected_version 0 starts a new logical record)
curl -fsS -X POST -H "Authorization: Bearer $SOLIDBOM_ADMIN_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"request_id":"sb22-ops-material-1","expected_version":0,
       "material_id":"9b220000-0000-4000-8000-0000000000e1",
       "name":"SB-22 ops synthetic PA12 fixture","density_g_cm3":"1.05",
       "price_per_kg":"38.50","currency":"CNY",
       "source_reference":"SB-22 operating handoff synthetic reference (public-safe)"}' \
  http://127.0.0.1:8396/v1/shared/materials/9b220000-0000-4000-8000-0000000000e1/versions

# a new cost-center rate version
curl -fsS -X POST -H "Authorization: Bearer $SOLIDBOM_ADMIN_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"request_id":"sb22-ops-rate-1","expected_version":0,
       "cost_center_id":"9b220000-0000-4000-8000-0000000000e2",
       "name":"SB-22 ops synthetic machining center","purpose":"production",
       "rate_per_hour":"150","currency":"CNY",
       "source_reference":"SB-22 operating handoff synthetic reference (public-safe)"}' \
  http://127.0.0.1:8396/v1/shared/cost-centers/9b220000-0000-4000-8000-0000000000e2/versions
```

Both answered `201` and appeared in the engineer reference-data read. Catalog components,
app-configuration versions and feasibility rule tables are published through the same pattern; the
exact routes and payloads are the tables in `services/solidbom-server/README.md`. Feasibility rule
data is deliberately not seeded: transcribed limits enter through the administrator publish route.

Project data is seeded as real user data, through the desktop (section 5) or the real routes:

| Step | Route |
|---|---|
| upload the original bytes once, by SHA-256 | `PUT /v1/sources/{sha256}?name=<original name>` (raw STEP bytes, `application/step`) |
| create a draft (`expected_version` 0) | `POST /v1/projects` with `SaveDraft` |
| save a reviewed update | `PUT /v1/projects/{project_id}/draft` with `SaveDraft` |
| calculate a saved version | `POST /v1/projects/{project_id}/calculations` with `CalculateRequest` |

The verified run used the tracked public-safe fixture `tests/solidbom/fixtures/three_boxes.step`
(SHA-256 `58927a45f74d0cced3f16a6999937a6f8e3cf3b83ca276cb9d37c17e17161e9e`, 11193 bytes), saved it
as project `9b220000-0000-4000-8000-000000000001` version 2 and calculated the immutable snapshot
`5f6077fd-4f3d-490b-9d7f-99d9bc577d46`. Use public-safe synthetic fixtures for acceptance work;
never put customer data into tracked fixtures, screenshots or pull-request text.

### 6.3 Owner references (local only, never in Git)

The owner reference files (CAD, CSV, workbook, PDF, analysis JSON) are **not tracked**:
`docs/solidbom/reference-manifest.json` lists all 70 supplied files with storage class and hashes,
`.gitignore` ignores `/docs/solidbom/requirements/**/*.{FCStd,csv,json,pdf,stp,xlsm,xlsx}`, and only
the 17 Markdown requirement documents are in Git. Tokens and connection strings are likewise
environment-only.

To populate them on a new machine, from the repository root and with the owner's supplied source
package available:

```sh
python3 scripts/solidbom_workspace.py hydrate --source '/path/to/solidBOM IDEA'
python3 scripts/solidbom_workspace.py check --require-references   # validates every file hash
```

Do not edit the source snapshot in place; a deliberate new snapshot needs a reviewed manifest
update. The asset gate must run with the references detached:
`python3 scripts/solidbom_workspace.py source-ci` (see `docs/solidbom/README.md`).

**Not verified in this run:** the owner source package is absent on this host, so `hydrate` and
`source-ci` were not executed, and `check --require-references` reported the local reference files
as missing. This is recorded as an unrun prerequisite, not as done.

## 7. Failure recovery

All symptoms and recovery actions below were produced in the acceptance run (raw output in the
evidence file); each action was then shown to work unless marked otherwise.

| Situation | Symptom observed | Recovery action |
|---|---|---|
| Service unavailable during a save | Desktop state `save: "save_failed"`, `retryable: true`, error `the SolidBOM service is unreachable: io: Connection refused (os error 61)`; version does not advance; the review and unsaved edits stay open. Scripted check: `curl` exits 7 / HTTP 000. | Do not close the app. Start the service again (section 4), confirm `/health` is 200, then run `solidbom.retry` (the same request id and payload is resubmitted; no duplicate is created). Observed: version 2 committed with the unsaved edit. |
| Service process restart | Existing committed state is durable; the process exits on SIGTERM/SIGINT and releases the port. After restart, `/health` is 200 and saved versions/calculations read back unchanged. | Start it again with the same environment. Retry any failed request; a committed write whose reply was lost is safe to retry unchanged. |
| Service replaced on another port / second instance | The service always answers on its `SOLIDBOM_BIND` loopback port; a second instance on the same port exits with `Address already in use (os error 48)`. | Stop the process that owns the port (`lsof -nP -iTCP:8396 -sTCP:LISTEN`, then SIGTERM), or point both programs at a different free port (`SOLIDBOM_BIND` and `SOLIDBOM_ENDPOINT`). The running instance is unaffected by the failed start. |
| Startup failure: bad `SOLIDBOM_DATABASE_URL` | Nonexistent database: `solidbom-server: database error`, exit code 1. Unreachable database host: no output for about 30 s (connection pool retry) and then the same `database error`, exit 1. | Confirm PostgreSQL is running on 127.0.0.1:5432, the database exists (`createdb`), and the URL names `$(whoami)@127.0.0.1:5432/<database>`. Start again. The `/health` of an already running instance is unaffected. |
| Startup failure: token configuration | `set SOLIDBOM_TOKEN`, `development token 1 must be at least 16 characters`, or `the development tokens must be distinct`, exit code 1 before listening. | Supply `SOLIDBOM_TOKEN` (≥16 characters) and make every configured token distinct. |
| Migration failure (database built by a different migration set) | `solidbom-server: migrations failed: migration 1 was previously applied but has been modified`, exit 1. | Do not hand-edit `_sqlx_migrations`. Use the build that matches the database, or restore a verified dump into a freshly created database (section 8). Verified: recreate + `pg_restore` + start recovered the same project/source/calculation and `/health` returned 200. For a disposable development database with no needed data, drop and recreate it while the service is stopped and let the migrations apply again. |
| Desktop process restart | The killed/closed app loses the current window session. | Relaunch (section 5) and reopen the project by id (`solidbom.reopen {"project_id": …}`). Verified: version 2 and the reviewed inputs (cycle 31, volume 2400) were recovered and the source was verified. Unsaved edits are not recovered across a crash; save before quitting. |

A timeout is an explicit retryable failure: `SOLIDBOM_REQUEST_TIMEOUT_MS` bounds one protected
request, the desktop transport timeout is above it, and a slow answer must be retried with the same
request id. Only `internal` (500) is retryable by the client; permission, validation and conflict
answers are not.

## 8. Backup and restore (verified procedure)

Use a PostgreSQL custom-format dump. This run took the dump while the service was running (pg_dump
uses a consistent snapshot). For a maintenance backup, stop the service first; the command and
format are identical (that stopped variant was not needed here).

### 8.1 Back up

```sh
pg_dump -h 127.0.0.1 -p 5432 --format=custom \
  --file=/tmp/solidbom-run/solidbom_sb22_ops.dump solidbom_sb22_ops

pg_restore --list /tmp/solidbom-run/solidbom_sb22_ops.dump | head
shasum -a 256 /tmp/solidbom-run/solidbom_sb22_ops.dump
```

Observed: exit 0; 75487-byte custom-format archive, SHA-256
`1ff2ae456526303f624f7fb1abc2db60cadfdac019a8ee5b73bbcac54867e5b5`, 138 TOC entries, gzip
compression, dumped from/to PostgreSQL 18.6. Do not store a dump in the repository.

### 8.2 Restore into a fresh database and verify

```sh
createdb -h 127.0.0.1 -p 5432 solidbom_sb22_ops_restore
pg_restore -h 127.0.0.1 -p 5432 --dbname=solidbom_sb22_ops_restore \
  --exit-on-error /tmp/solidbom-run/solidbom_sb22_ops.dump
```

Observed: exit 0; the row counts and all 13 `_sqlx_migrations` rows matched the source database.

Start the service against the restored database on a second port and read the data back:

```sh
SOLIDBOM_DATABASE_URL="postgres://$(whoami)@127.0.0.1:5432/solidbom_sb22_ops_restore" \
SOLIDBOM_BIND="127.0.0.1:8397" \
nohup services/solidbom-server/target/debug/solidbom-server \
  > /tmp/solidbom-run/service-restored.log 2>&1 &

curl -fsS http://127.0.0.1:8397/health                 # {"ok":true}

PROJECT=9b220000-0000-4000-8000-000000000001
curl -fsS -H "Authorization: Bearer $SOLIDBOM_ENGINEER_TOKEN" \
  "http://127.0.0.1:8397/v1/projects/$PROJECT"
curl -fsS -H "Authorization: Bearer $SOLIDBOM_ENGINEER_TOKEN" \
  "http://127.0.0.1:8397/v1/calculations/5f6077fd-4f3d-490b-9d7f-99d9bc577d46"
curl -fsS -H "Authorization: Bearer $SOLIDBOM_ENGINEER_TOKEN" \
  "http://127.0.0.1:8397/v1/sources/eee9d978-e9d2-41bd-ab44-630c158edfbc" \
  -o /tmp/solidbom-run/recovered.step
shasum -a 256 /tmp/solidbom-run/recovered.step
```

Observed: the project view and calculation snapshot compared byte-identical (canonical `jq -S`
output, `diff` empty) with the original service on 8396; the recovered source bytes compared
byte-identical (`cmp`) and hash to `58927a45f74d0cced3f16a6999937a6f8e3cf3b83ca276cb9d37c17e17161e9e`;
the recovered project kept project id, source id/SHA-256/byte length, saved version 2, calculation
id, input hash `23f3dcd1cb74f7e9834949bf1a7e55d60a1fa5cfd5d7dd7a7b0206c907ecd306`, calculator
version `solidbom-direct-cost/1` and outputs (complete, subtotal `3.416404800`, setup total `60.0`).

To replace an existing database with a backup: stop the service, `dropdb --if-exists` only the
explicitly named target database, `createdb`, `pg_restore --exit-on-error`, then start the service
and repeat the verification reads. The migration-failure recovery (section 7) used exactly this
path.

## 9. Known limits and unrun requirements

The MVP gate is **Pending**. This handoff supports the agreed deployment and recovery only; it is
not a readiness claim. The requirement-by-requirement pass/fail/unrun matrix is maintained in the
SB-22 execution record and evidence index; this handoff does not replace it.

Rows this operating run leaves **Unrun**: owner-reference hydration and the gated `source-ci`
command (owner source package absent), the release-profile desktop build, the complete P01–P14
desktop sweep with real captures (owned by the desktop acceptance evidence), A07 and
R-CAD-06/R-CAD-07 (independently verified owner references and agreed targets missing), and every
confirmed A01–A20 or R-* row that the service/desktop evidence files do not cover. None of those
rows is upgraded to Pass by this handoff.

### 9.1 Unsupported deployment and explicit exclusions

Per contract §1 and decisions D-01/D-02/D-03 and E07, these are **out of scope** for this gate and
excluded from this handoff: browser delivery, signed/notarized installers and automatic updates,
Windows and Linux delivery, offline authoritative editing or local costing, multi-organization
isolation, NPV/payback analysis, customer-facing quotations, and SB-X01 machine-rate construction as
a readiness requirement. There is no production authentication, onboarding or TLS termination: the
service speaks plain HTTP on loopback, and a remote development endpoint would need a
TLS-terminating proxy (not provided here). Published machine-rate versions are consumed through
SB-09 like any other cost-center rate.

### 9.2 Missing or unverified desktop surfaces

The launch, control channel, desktop→service connection, import/auto-save, reviewed edit, failed
save, explicit retry, process restart and reopen-by-id were exercised in this run. Not exercised by
this handoff (owned by the desktop acceptance evidence): the full prototype surface sweep P01–P14
and its real screenshots, including P10's part-number metadata path and P13's analysis-delay
selection/navigation, the feasibility/readiness desktop surface, exports from the desktop and the
complete manual measurement UI. The desktop needs the combined development token for the
administrator shared-data surface; an engineer-only token is refused there by the service.

### 9.3 Unavailable owner references

The 70 owner reference files are local-only and are not present in this workspace: `check
--require-references` reported 53 local-only files missing (the five PDFs linked from
`DESIGN_RULES_REFERENCE.md` among them). `hydrate` and `source-ci` were not run because the owner
source package is absent. A07 and the R-CAD-06/R-CAD-07 accuracy claims remain **Unrun** because
the independently verified owner references, benchmark metadata and the agreed zero-reference
validation convention are still missing (contract §4.4). The MVP gate cannot pass while they are
Unrun.

### 9.4 Unresolved performance targets

No agreed coverage or processing-time targets exist from the owner, and the prototype's
responsiveness expectation (P13: UI responsive during a controlled 5 s service delay) was not
measured in this run. `SOLIDBOM_DELAY_MS` exists only to exercise that measurement; it must stay
unset (`0`) for normal operation. Treat performance as unverified until the desktop acceptance
evidence records a measurement and the owner agrees targets.

### 9.5 Development-service limits carried from the service README

One development organization and at most three development identities; CNY-only material/rate
records; no production authentication or multi-organization isolation; online-only prototype with
no client write queue and no local authoritative cost; annotations and a server-owned
material-overhead policy version are deferred; feasibility rule data is not seeded (transcribed
limits enter only through the administrator publish route); feasibility evidence currently accepts
extracted tube geometry only; app-config categories are stored/validated but the direct-cost
calculator does not consume the category list yet; reference-change facts are emitted but project
adoption is a separate workflow; machine hourly rates are the gated SB-X01 extension. The service
has no separate repository CI workflow yet; the commands in `services/solidbom-server/README.md`
are the acceptance run.

### 9.6 Operating limits of this handoff

The backup is a manual `pg_dump` procedure; no scheduled backup, retention policy, monitoring,
service manager/launchd unit or log rotation is installed. The desktop is a development build
(`dev` profile was built, launched and verified; a release-profile build was not executed in this
run). Unsaved desktop edits are not recovered across a crash; save before quitting. A single
PostgreSQL instance on 127.0.0.1:5432 is assumed.

## 10. Evidence

Raw run: [SB-22-operating-handoff.txt](execution/evidence/SB-22-operating-handoff.txt) (environment,
exact commands, raw outputs, result table). The run used `solidbom_sb22_ops` /
`solidbom_sb22_ops_restore` and loopback ports 8396/8397 for the service and 8398 for the desktop
control channel; temporary scripts and the dump stayed under `/tmp`, and no token, connection
string or owner reference is recorded in this document.
