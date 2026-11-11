# MVP acceptance and operating handoff contract

Version: 1 (acceptance/deployment contract; additive to API version 2, no wire change).

Owner: SB-22 (MVP acceptance and operating handoff).

Status: locked for the owner-instructed implementation start on issue #37, 2026-11-12. The package
brief's assignment gate is only partly met: the owner instructed the work to start, the acceptance and
deployment boundaries are fixed here, and independent customer references plus agreed coverage and
performance targets are still missing from the owner. The package record states that limitation; this
document does not promote the MVP to accepted.

Authority order for this contract:

1. Later explicit owner decisions.
2. Confirmed requirements in `docs/solidbom/requirements/cost_calculator_handover/APP_SPECIFICATION.md`
   (sections 1–15 and acceptance scenarios A01–A20).
3. `docs/solidbom/requirements/cost_calculator_handover/DESKTOP_PROTOTYPE_SPEC.md` checks P01–P14.
4. `docs/solidbom/requirements-map.md` obligations R-*, examples A01–A20, prototype checks P01–P14 and
   the proposal/historical/exclusion registers.
5. This contract, the package brief and the imported handover arithmetic.

Nothing here replaces those sources or converts a Proposal into a confirmed requirement. A defect found
during acceptance is recorded and handed to the responsible package; SB-22 does not repair feature
modules, shared contracts or CI while acting as acceptance owner.

## 1. Deployment boundary being accepted

The only deployment in scope is the one the confirmed prototype decisions describe:

- A locally launched macOS development build of the SolidCraft desktop fork, built from the same commit
  as the service, with the SolidBOM panels linked to the existing CAD tree and viewport.
- The `services/solidbom-server` development service on loopback, with PostgreSQL 18 storage on the
  same development machine. The service is its own Cargo workspace and applies its migrations at start.
- Development tokens of at least 16 characters created for a run only. Tokens, connection strings and
  any owner data never enter tracked files or pull-request text.
- Local geometry cache is temporary and rebuildable; project data, original STEP sources, reviewed
  inputs, calculations and accepted results are central and survive a desktop process restart.

Out of scope for this gate, per decisions D-01/D-02/D-03 and decision E07: browser delivery, signed
installers and automatic updates, Windows/Linux delivery, offline authoritative editing or local
costing, multi-organization isolation, NPV/payback analysis, customer-facing quotations, and using
SB-X01 machine-rate construction as a readiness requirement. Published machine-rate versions are
consumed through SB-09 exactly as any other cost-center rate.

The operating handoff documents this deployment and its recovery, not production operations.

## 2. Evidence scope and retention

An acceptance row is evidence only when it contains all of the following:

- The requirement or scenario ID being checked.
- The exact integrated commit (`git rev-parse HEAD`) and a clean tracked working tree.
- The environment: macOS version, CPU/RAM, Rust version, PostgreSQL version, service and desktop
  binary SHA-256 values, fixture SHA-256 values and, for desktop rows, the control-channel method and
  viewport/theme used.
- The exact commands or scripted steps that produced the observation, and the raw status/body or
  screenshot that was observed. A summary of a test name is not an observation.
- For numeric claims, the value the implementation returned and the independently recomputed value,
  with the stated tolerance and the unrounded inputs. Imported measurements are never their own
  expected value.
- For desktop-visible claims, a real capture of the running app. Mockups, regenerated images or another
  build's screenshots are not evidence.

Runs are recorded in `docs/solidbom/execution/evidence/SB-22-*`: text evidence for service/storage
runs, PNG screenshots for desktop claims, and the rendered export files where an export is checked. A
script that regenerates a run may stay untracked when it embeds a token or a machine path; its command
line and fixture hashes are recorded instead. Public-safe synthetic fixtures are used; owner references
stay local and are never copied into the repository, a screenshot or a pull request.

A row cannot be marked passed by: package inventory, the presence of unit tests, a branch status, the
tracker state, source inspection alone, or another package's summary. Unit and integration suites run
here support the claim; they do not replace the run.

## 3. Result semantics

Every requirement row gets exactly one result:

| Result | Meaning |
|---|---|
| Pass | The confirmed observation was produced against the integrated build, with the evidence above. |
| Fail | The observation was produced and violates the requirement. The responsible package and the observed difference are recorded. |
| Unrun | The check was not executed. The prerequisite that blocks it and the owner of that prerequisite are named. An unrun row blocks any readiness claim that depends on it. |
| Not applicable | Only where the confirmed scope genuinely does not contain the behavior. The scope decision is cited. |

A partial result is written as the specific passed observations plus the explicit unrun remainder; a
row is never upgraded to Pass because part of it passed. Skipped or failed checks remain visible in
the matrix and in the execution record. The MVP acceptance gate stays `Pending` unless every confirmed
obligation and all A01–A20 have Pass, and this contract's remaining prerequisites are supplied.

## 4. Required execution set

### 4.1 Confirmed application scenarios A01–A20

All twenty examples in application section 16 are attempted against the integrated build and real
storage. Each row records the level actually exercised (service HTTP route, desktop control channel,
or both) and what the integrated build cannot yet expose. Service-level observation is sufficient only
for behavior the service owns; a row whose requirement is a desktop interaction needs a real desktop
capture.

### 4.2 Confirmed obligations R-*

The confirmed obligation groups in `docs/solidbom/requirements-map.md` are covered by the A-row
evidence where the same behavior is exercised, plus targeted checks for the obligations without a
scenario row: permissions, request idempotency, version conflicts, restart persistence, source
retention, export labeling and scope exclusions.

### 4.3 Recovery, permissions and restart

- Missing or unknown token: `401`; known identity without the route's permission set: `403`.
  Engineer versus Administrator sets are both exercised.
- Lost reply: an identical retried write returns the original receipt and does not duplicate a record;
  a changed payload under the same request id is refused.
- Stale versions/results: an expected-version conflict is refused without overwriting accepted state; a
  result computed from superseded inputs is labelled stale and never presented as current.
- Disconnect/retry: a failed desktop save reports the failure, keeps unsaved form edits in the current
  session and can be explicitly retried after the service returns.
- Process restart and restore: the service and the desktop process are stopped and restarted; the
  original source, reviewed inputs, accepted calculations and accepted baselines are recovered
  unchanged. A PostgreSQL dump/restore of the development database followed by a service start must
  reproduce the same stored identifiers and accepted results.

### 4.4 Independent dimensional and recognition evidence — blocked prerequisite

A07 and the R-CAD-06/R-CAD-07 accuracy claims need independently verified owner references, benchmark
metadata and agreed coverage/processing-time targets. Those owner inputs were not supplied at the time
of this run, and the zero-reference validation convention is still an explicit unresolved acceptance
detail. The matrix records the analytic-fixture measurements that can be independently checked
(`tests/solidbom/manifest.json`) and marks the owner-reference rows Unrun with the missing input, the
requested action and the affected requirements named. The MVP gate cannot pass while they are Unrun.

### 4.5 Prototype regression rerun P01–P14

The approved prototype checks are rerun on the integrated build from the tracked synthetic fixtures and
compared with SB-08's recorded results (`docs/solidbom/execution/SB-08.md`). Rows the integrated build
still cannot expose keep the SB-08 gap reason (P10's part-number metadata path, P13's analysis-delay
selection/navigation) and are Unrun or partial exactly as observed, not as previously recorded.

### 4.6 Required checks

- Service workspace: format, clippy with `-D warnings` and the PostgreSQL-backed test suite on a
  disposable database; every failure is recorded with its command and output.
- Repository gates: the fork's root checks that can run on this host (`cargo xtask` format, clippy,
  lists, skills, layers, mesh decisions, assets/icons, license check, schema checks), run individually
  where the aggregate `ci` command cannot run. The documented red host baseline for desktop test
  binaries is reproduced rather than hidden; any new failure is recorded.
- A changed default, message catalog, generated file or design format is not in scope: an acceptance
  run does not change behavior, so those gates only need to confirm the branch does not violate them.

## 5. Operating handoff required content

`docs/solidbom/operating-handoff.md` records instructions that were actually executed for this
deployment:

- Build and launch: exact commands for the desktop app and the service, required environment
  variables, loopback binding, and how to confirm both are healthy.
- Database and references: creating a database, migration-on-start behavior, how to read the applied
  migration version, how development/reference data is seeded and how owner reference files are
  managed without entering the repository.
- Backup and restore: a PostgreSQL dump/restore procedure verified by restoring into a fresh database,
  starting the service against it and recovering the same project/source/calculation identifiers.
- Failure recovery: service unavailable during save, process restart, startup failure and migration
  failure, with the observed symptom and the documented recovery action.
- Known limits: unsupported deployment, missing desktop surfaces, unavailable owner references,
  unresolved performance targets and every requirement this run marks Unrun or Fail.

The handoff does not install packages, create services or change production deployment; those remain
deferred.

## 6. Deliverables

- This contract.
- `docs/solidbom/execution/evidence/SB-22-*` evidence files and desktop captures.
- The requirement-to-test/evidence matrix (pass/fail/unrun per confirmed obligation, A01–A20, P01–P14
  and scope/exclusion rows): `docs/solidbom/execution/SB-22-acceptance-matrix.md`.
- `docs/solidbom/operating-handoff.md`.
- `docs/solidbom/execution/SB-22.md` with the environment, build/version manifest, checks, evidence
  index, explicit limitations, review status and next action.
- The package pull request against `main` carrying only these acceptance/handoff records.

## 7. Compatibility and change policy

This contract changes no wire type, route, command, migration, preference or default. It selects no
product behavior. A run that identifies a required behavior missing from the integrated build records a
failure and its responsible package; it does not extend scope or patch that package here. If a later
run needs a different evidence scope or deployment boundary, this document is amended in the same pull
request and the change is re-reviewed.
