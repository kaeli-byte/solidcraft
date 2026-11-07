# SolidBOM desktop prototype implementation plan

> For agentic workers: use superpowers:executing-plans to implement this plan task by task when implementation is authorized. Checkboxes below describe future work, not completed work.

**Goal:** deliver the approved macOS desktop workflow from STEP import to reviewed direct costs and persistent save/reopen.

**Architecture:** extend SolidCraft's existing Rust/egui desktop with a SolidBOM panel and immutable imported-geometry session. Run geometry analysis locally in background jobs; a separate Rust service owns project data and authoritative costing. Keep portable contracts separate from native transport and service persistence.

**Tech stack (implementation choices):** existing Rust/egui CAD application; serde JSON contracts; Axum/Tokio backend; SQLx/PostgreSQL persistence; rust_decimal for costs; native HTTP transport injected by the desktop host. These choices implement the approved behavior and are not new user requirements. Pin compatible dependency versions and preserve lockfiles in task 1; no packages were installed for this plan.

**Specs:** [DESKTOP_PROTOTYPE_SPEC.md](DESKTOP_PROTOTYPE_SPEC.md), [APP_SPECIFICATION.md](APP_SPECIFICATION.md). D01–D07 are approved. All P01–P14 checks remain unexecuted.

## Constraints and inspected baseline

- macOS first, locally launched development build; no installer/updater work.
- Online-only project writes; central costing and data; retain failed-save form edits in the current session and retry explicitly.
- Imported geometry is read-only across UI, shortcuts and programmatic command paths. Preserve original STEP bytes and hierarchy. Geometry changes require a new source revision.
- Exact decimal arithmetic for business inputs; explicit units; missing values never become zero. Volume tolerance is ±0.1%; numeric fixture arithmetic tolerance is 0.0000001 CNY.
- One-second maximum navigation/selection response during the controlled five-second delay, on the declared test Mac. This target is not a benchmark result.
- Follow SolidCraft command, UI-kit, layering, localization, asset, hostile-input and wasm checks. No kernel replacement. Only kernel may use truck. No native network/database dependency below the desktop host.
- Product source: `/Users/hafid/Documents/projects/solidCraft`, HEAD `cc9787f80e4e0aef068d576e6c6ddaf52d909f11`, version 0.27.1 at inspection. Existing untracked `.dsh/` must be preserved.
- Inspected machine: arm64, macOS 26.7.1 (25G241), rustc 1.99.1, Python 3.14.7. Demonstration target is `aarch64-apple-darwin` on this machine; minimum supported OS beyond it is not established. Python is not an application dependency.
- Existing integration points: `SolveApp`, `Services`, `SolveApp::run/start` in `crates/ui-egui/src/lib.rs`; `Session::execute` in `crates/engine/src/lib.rs`; `file_load.rs`, `save_job.rs`, `selection.rs`, `workspace.rs` in UI; `cmd/file_import.rs`, `cmd/bom.rs`, `cmd/component_properties.rs`, `cmd/inspect.rs` in engine. The earlier analysis records actual CLI observations, not proof of the future implementation.

## Review focus

1. A lost reply after a committed save must not create duplicates on explicit retry (task 3).
2. Reimport/version changes must not attach old measurements to new geometry (tasks 4 and 6).
3. A command reached through MCP/control/shortcut must not bypass source immutability (task 5).
4. Open surfaces and flat multi-body files must remain incomplete where associations/measurements are missing (tasks 4 and 6).
5. A late cost response after a selection or input change must not overwrite current data or mislabel it saved (task 6).

## File and dependency map

All paths in this section are relative to the SolidCraft checkout. New paths are proposed, not existing artifacts.

| Path | Responsibility |
|---|---|
| `crates/solidbom-contract/{Cargo.toml,src/lib.rs,src/types.rs,src/error.rs}` | Portable wire types and schema version; no UI, I/O or database |
| `crates/solidbom-costing/{Cargo.toml,src/lib.rs,src/calculate.rs,src/tests.rs}` | Pure deterministic backend arithmetic; never called for authoritative desktop costs |
| `services/solidbom-server/{Cargo.toml,src/main.rs,src/lib.rs,src/routes.rs,src/store.rs,src/auth.rs}` | Separate native Rust workspace: API, database transactions and development authentication |
| `services/solidbom-server/migrations/0001_initial.sql` | Schema described below |
| `services/solidbom-server/tests/{persistence.rs,api.rs}` | PostgreSQL integration and failure cases |
| `apps/solidcraft/src/solidbom_client.rs` | Native transport, development endpoint/token configuration and asynchronous host adapter |
| `crates/engine/src/solidbom/{mod.rs,extract.rs,policy.rs,tests.rs}` | Extraction, mapping and session mutation guard |
| `crates/ui-egui/src/solidbom/{mod.rs,state.rs,panel.rs,jobs.rs,tests.rs}` | Portable panel/controller, drafts and completion handling |
| `crates/ui-egui/tests/scenarios/solidbom-review.json` | Native review workflow scenario |
| `tests/solidbom/{manifest.json,README.md}` | Synthetic STEP fixture identities, dimensions, expected mappings and provenance |
| `docs/solidbom-prototype.md` | Startup, endpoint configuration, saved-state behavior and acceptance procedure |

Modify existing root `Cargo.toml`, relevant engine/UI/app manifests and sorted module declarations. Register contract at L0 and costing at L1 in `xtask/src/layers.rs`; confirm the layer checker accepts these names and edges. Engine/UI may depend on contract; the desktop must not depend on costing. The service has its own `[workspace]` and lockfile and references contract/costing by path. Keep database and HTTP server dependencies outside the CAD workspace's wasm library checks. Native transport is injected through `Services`, with no-service behavior for web/test hosts.

## Core contracts

Use UUIDs encoded as strings for persistent identities, u64 version counters, SHA-256 hex source hashes and decimal strings for amounts/measurements in JSON. Convert finite geometric f64 output once using round-trip decimal text, retain algorithm/tolerance metadata, and reject nonfinite/overflow values. Never round to displayed precision before calculation.

Define these public types in `solidbom-contract/src/types.rs`:

- `SourceRef { source_id, sha256, byte_len, original_name }`.
- `AnalysisKey { source_sha256, engine_build, extractor_version }`.
- `DefinitionRef { definition_id, source_key, customer_part_number: Option<String>, customer_revision: Option<String> }`.
- `OccurrenceRef { occurrence_id, definition_id, parent_occurrence_id: Option<Uuid>, source_path: Vec<u64>, transform: [[f64;4];4] }`.
- `Measurement { value: Option<Decimal>, unit, source: Extracted|Manual, review: Pending|Confirmed, reason: Option<String>, analysis_key }`.
- `AnalysisManifest { schema_version, key, definitions, occurrences, geometry_bindings, issues }`. A binding maps source-local body/placement identity to definition/occurrence IDs. Geometry IDs are scoped to the analysis, not globally inferred from names.
- `PartInputs { part_version_id, classification, classification_confirmed, sourcing, volume_mm3: Measurement, material_version_id: Option<Uuid>, allowance_rate: Option<Decimal>, operation: Option<OperationInputs> }`.
- `OperationInputs { cycle_seconds, cycle_basis: PerPiece, cavities, production_rate_version_id, setup_hours, setup_rate_version_id }`, numeric fields nullable until completed.
- `DraftProject { project_id, product_revision_id, version, source, analysis, parts, forecast_year, forecast_quantity, currency }`.
- `SaveDraft { request_id, expected_version, draft }` and `SaveReceipt { request_id, project_id, version }`.
- `CalculateRequest { request_id, project_id, saved_version }` and `CalculationSnapshot { calculation_id, project_id, saved_version, input_hash, calculator_version, inputs, outputs, issues }`.
- `Issue { code, field_path, definition_id: Option<Uuid>, message }`; result state `Incomplete|Complete`. “Complete” means the supported direct-cost subtotal only; UI retains the prototype label.

Material and rate values are server-owned versioned records. The API resolves selected version IDs rather than accepting an authoritative client-supplied rate. Fixtures seed their explicit values. Customer number/revision only proposes identity; same-number/different-revision definitions remain separate. Cross-product matching is deferred.

## Persistence and request semantics

Use PostgreSQL for the development service, with an explicitly configured test database. Store the small prototype's original STEP bytes in a `sources` bytea column with unique SHA-256, length and filename. This keeps source/reference commits transactional and avoids a separate object-store lifecycle in this slice. Stream with a configurable request size limit, initially 64 MiB as a prototype resource limit, not a claimed supported-file target. Do not store client absolute paths as source locations.

Tables: `sources`; `projects` (current version); append-only `project_versions` (validated draft JSON plus source FK); `material_versions`; `rate_versions`; append-only `calculations` (input snapshot, results, version/hash); `requests` (request ID, operation, payload hash and recorded response). Uniqueness on `(project_id, version)` and request ID. JSON is versioned/validated contract data, not unstructured arbitrary commands. Values requiring numeric queries can be projected later.

Every save locks the project row, checks expected version, validates source/material/rate references, inserts the new immutable version and receipt, then advances the current version in one transaction. Idempotency is checked before optimistic concurrency: repeating the same request ID/payload returns its prior receipt even if the current version advanced; a reused ID with different payload returns 409. New request IDs with stale versions return 409 without a partial write. Failed requests preserve local form edits.

API v1:

| Route | Contract |
|---|---|
| `GET /health` | readiness only; no credentials/data |
| `PUT /v1/sources/{sha256}` | raw STEP bytes; server verifies hash/size, returns SourceRef; repeat identical bytes is idempotent |
| `GET /v1/sources/{id}` | original bytes for reopen, hash checked by client |
| `GET /v1/reference-data` | seeded immutable material/rate versions |
| `POST /v1/projects` | request ID plus initial draft; transaction creates project/version 1 |
| `GET /v1/projects/{id}` | current draft and saved calculation references |
| `PUT /v1/projects/{id}/draft` | SaveDraft -> SaveReceipt |
| `POST /v1/projects/{id}/calculations` | CalculateRequest -> immutable CalculationSnapshot |
| `GET /v1/calculations/{id}` | historical snapshot unchanged by later edits |

Use 422 for invalid contract/unsupported inputs, 409 for conflicts, 401 for absent/invalid token, 413 for configured source size limit. Missing engineering values are saved draft issues; calculation returns Incomplete with null affected totals, not zeros. Unexpected server errors do not expose internal paths or secrets.

One explicit development bearer identity is sufficient for this prototype. Configure token and endpoint through environment variables; redact tokens from logs. Default service bind is loopback; any remote development endpoint uses TLS. This is a real independent service/database even if both run on the developer Mac. Do not add production identity or organization onboarding in this slice.

## Client state and geometry safety

Separate `Idle/Importing/Reviewing` from `Clean/Dirty/Saving/SaveFailed` and `NoResult/Calculating/Current/Stale/Incomplete`. Each job carries project/revision ID, analysis key, local edit generation and request ID. Completion is applied only to the matching record; selection is never a response destination. Capture the save generation: edits made during save remain dirty after the older save succeeds. No automatic network mutation retries.

Calculate explicitly saves current inputs first, then calculates that returned version. On failure, show issues and keep the draft. Preserve every earlier calculation ID. Source bytes are uploaded before their project reference; unreferenced source rows are harmless and may be cleaned by a later maintenance task.

For reopen, fetch source and manifest, verify hash and rebuild local geometry. Restore mapping only if the extraction manifest matches the pinned analysis contract; a mismatch requires review and disables a current cost claim. Stable server IDs are preserved for unchanged source-path mappings, not regenerated on every reopen. Missing customer revision information may be manually supplied from the fixture manifest; do not assume STEP imports populate it.

Add an engine session access mode `Editable|SolidBomReview`. It is host/session state, not serialized in customer CAD metadata. At `Session::execute`, enforce an explicit reviewed allowlist for non-geometric operations; unknown commands are denied in review mode. Undo/redo, source replacement, modeling, component transforms/reparenting and material/parameter geometry commands are denied. View, selection, inspect and local save/export operations may be allowed after checking their effects. Local file save is allowed only to a separate working path, never over the bound original source. Source opening occurs in a new scratch session through the trusted project-import path, then atomically binds review mode; normal control/MCP cannot disable it. Confirm no UI direct-document mutation path bypasses the policy. Engineering annotations/inputs live outside the CAD document.

## Execution tasks

Run each test first to establish a meaningful failing assertion, implement the smallest corresponding change, then rerun. Commands below refer to future files/targets. Do not claim they currently exist or pass. Commit scoped changes only after required checks; do not push or create a public fork as part of this plan.

### 1. Portable contracts and numeric calculator

Files: new contract/costing crates and root/layer registrations above.
Interfaces: `calculate(input: &ResolvedCalculationInput) -> CalculationOutcome`; resolved inputs include validated material/rate values, source volume, quantities and project currency. Outcomes contain line explanations, issues, per-part and assembly direct subtotals and separate setup total.

- [ ] Add contract serialization tests for decimal strings, missing versus explicit zero, nonfinite geometry rejection and schema version rejection; run `cargo test --profile ci -p solidbom-contract` to observe failures.
- [ ] Add `fixture_cost`, `missing_density`, `zero_cycle`, `allowance_once`, `setup_once` in costing tests. Assert 0.132192 material, 0.0066096 overhead, 1 production, 1.1388016 part, 3.4164048 assembly and 60 separate setup. Missing cycle is incomplete; explicit zero is valid for this prototype's manual operation and gives zero production. Reject negative cycle, nonpositive density/rates and cavity count zero. Setup zero is permitted; omitted setup remains missing until explicitly set.
- [ ] Implement checked decimal arithmetic and fixture-specific supported-category validation. No implicit company defaults. Add repeat-quantity and 60-second-cycle result 6.4164048 tests.
- [ ] Run both crate suites and `cargo xtask layers`; expected all pass. Record dependency pins and source/license entries under repository conventions.

### 2. Transactional project storage

Files: server manifest/workspace, migration, store, persistence tests.
Interfaces: `Store::create_draft`, `Store::save_draft(SaveDraft) -> Result<SaveReceipt>`, `Store::load_project(Uuid) -> Result<DraftProject>`; async database methods.

- [ ] Write PostgreSQL integration tests for close/reconnect persistence, FK rejection, same-number/different-revision separation and stale expected-version rejection.
- [ ] Implement schema/migrations and source hashing/persistence, with append-only project history and explicit current-version transaction.
- [ ] Run `cargo test --manifest-path services/solidbom-server/Cargo.toml --test persistence` against an isolated test database; assert failed transactions leave all prior rows/current versions unchanged. Tests must never reset an arbitrary DATABASE_URL database.

### 3. API, authoritative calculation and lost replies

Files: server routes/auth/lib/main, API tests.
Interfaces: route table above; `Store::record_calculation(CalculateRequest) -> Result<CalculationSnapshot>` calls task 1 with server-resolved inputs.

- [ ] Add HTTP tests for unauthenticated requests, invalid decimal/unit input, oversized/hash-mismatched upload and missing reference versions.
- [ ] Implement routes and development token configuration. Calculate only persisted versions; include input hash and calculator version in immutable results.
- [ ] Test committed-save/lost-response retry returns the identical receipt, changed-payload request-ID reuse fails, and a later calculation leaves the earlier snapshot unchanged. A calculation on a historical version remains labeled historical when the project advanced.
- [ ] Run `cargo test --manifest-path services/solidbom-server/Cargo.toml --test api`; require all cases and seeded numeric fixtures pass through HTTP, not just pure functions.

### 4. Local extraction and persistent mappings

Files: engine solidbom extract module/tests; synthetic STEP fixtures/manifest.
Interfaces: `extract(session: &Session, key: AnalysisKey) -> Result<AnalysisManifest>`; `resolve_pick(manifest: &AnalysisManifest, selection: &Sel) -> Result<OccurrenceRef>` with source-local mapping supplied by import.

- [ ] Create original analytic STEP fixtures and independently authored manifests for three placed boxes, open surface, revisions B/C and flat multi-body input. Document generation/provenance; do not use imported output as expected truth.
- [ ] Implement extraction using existing component/occurrence and physical-property APIs. Use definition grouping; preserve placements, explicitly detect ambiguous/flattened mappings and missing volume. Do not rely on default BOM rows as identity or fallback material density as reviewed data.
- [ ] Test six tree/viewport selection bindings, volume 2400 ±2.4 mm³, quantity three, revision separation, open-volume None, unresolved flat grouping and changed-analysis-key refusal.
- [ ] Run `cargo test --profile ci -p solidcraft-engine solidbom`; require fixture source hashes unchanged and mapping restored deterministically on second import.

### 5. Enforce read-only reviewed source

Files: engine policy module, `Session` fields/execute/scratch/replacement paths; UI command/interactive gates; existing relevant control tests.
Interfaces: `ReviewPolicy::check(command_id: &str) -> Result<()>`; policy bound by trusted import only and retained on session copies.

- [ ] Add denial tests for modeling, reparenting, move, undo/redo, parameter mutation and doc.open through execute/control; assert document/manifest hash unchanged.
- [ ] Implement default-deny policy, explicit permitted read/view/selection and safe local-save operations. Audit interactive tools/direct document writes and disable their start while bound. Do not use `.noundo()` as a proxy for read-only.
- [ ] Test permitted selection/measurement and separate local CAD save, plus denial of overwriting the source. Ensure project-bound session replacement cannot escape the guard. Test ordinary unbound SolidCraft sessions retain modeling behavior.
- [ ] Run engine solidbom tests and corresponding UI/control tests; record every entry path tested for P14.

### 6. Native service adapter and versioned UI controller

Files: native client, Services extension, UI state/jobs/tests, app host wiring.
Interfaces: portable `SolidBomHost::submit(request: ClientRequest) -> JobId` and `poll() -> Vec<JobCompletion>`; request enum wraps source upload/get, project create/load/save, reference data and calculation contracts. Native host handles network/threading; UI polls without blocking.

- [ ] Write controller tests for selection changes during work, edit-during-save, obsolete response, failed save retaining fields, conflict with explicit retry and disconnect without queued writes.
- [ ] Implement native transport with bounded timeouts and no implicit mutation retry; inject into Services. Build background analysis from a scratch session and publish only when source/generation still match. Never fall back to synchronous window-thread work if spawning fails.
- [ ] Implement save-then-calculate chain, reopen/hash verification, immutable historical results and distinct local-CAD/project saved labels. Annotation text persists with engineering inputs and does not modify source geometry.
- [ ] Run `cargo test --profile ci -p solidcraft-ui-egui solidbom`; use controlled channels/barriers rather than timing-sensitive sleeps. Add a no-service host test and run `cargo xtask wasm`.

### 7. Review panel and complete persisted workflow

Files: panel module, workspace integration, localization pending file, native scenario.
Interfaces: panel reads task 6 state and dispatches controller actions; no direct HTTP or business arithmetic in rendering.

- [ ] Add native scenario selecting three occurrences in tree and viewport, confirming manual classification, selecting explicit material/rates, entering allowance/cycle/setup and invoking Calculate/Save.
- [ ] Implement the three panel sections with existing `ui_kit::Container` and theme tokens. Show units, measured/manual source, pending review, missing fields, quantity basis, breakdown, separate setup, stale results and `Draft prototype / Direct-cost subtotal`. Permit manual volume completion with reason; label ungrouped flat input review-required. List omitted cost categories.
- [ ] Run native workflow against a real development service/database, close desktop completely, restart and reopen by project ID. Assert recovered IDs, source hash, engineering values and original calculation ID; capture genuine screenshots/recording.
- [ ] Exercise P07 cycle edit and P09 manual surface completion; verify historical results remain readable and no accepted production-state action is exposed.

### 8. Acceptance run and handoff

Files: `docs/solidbom-prototype.md`, acceptance evidence record beside fixtures (exclude private media/data).

- [ ] Document fresh checkout/service migration/reference seeding/start commands, environment variables without secrets, target Mac and pinned builds. Verify another fresh database can run the workflow.
- [ ] Run all P01–P14 in the coverage map below; capture actual pass/fail evidence. Inject five-second analysis/service delays and measure UI navigation/selection latency, requiring ≤1 second; keep timing evidence separate from deterministic tests.
- [ ] Run `cargo fmt --all -- --check`, relevant clippy/tests and `cargo xtask ci` for the CAD workspace. Separately run server fmt/clippy/tests because it is an independent workspace. Address failures before claiming completion; report any unrun checks explicitly.
- [ ] Review diff for source mutation paths, leaked tokens, business defaults and generated-file policy. Commit only authorized implementation changes after checks. Deliver prototype evidence and remaining full-MVP gaps; do not claim tube recognition or customer-file performance.

## Acceptance coverage and dependency order

Tasks execute 1 → 2 → 3 → 4 → 5 → 6 → 7 → 8. Revalidate source checkout and repository rules before execution. No worktree, fork, database or implementation was created during planning.

| Criteria | Owning tasks |
|---|---|
| P01 source/import; P02 selection; P03 volume | 4, 7 |
| P04 classification/material review | 1, 6, 7 |
| P05 arithmetic | 1, 3, 7 |
| P06 save/reopen | 2, 3, 6, 7 |
| P07 invalidation | 3, 6, 7 |
| P08 missing inputs | 1, 3, 7 |
| P09 surface/manual values | 4, 6, 7 |
| P10 revision separation | 2, 4, 7 |
| P11 unresolved flat input | 4, 7 |
| P12 failures/retries | 2, 3, 6 |
| P13 responsiveness | 6, 8 |
| P14 scope and read-only enforcement | 5, 7, 8 |

## Self-review and remaining limits

Checked against the approved scope: all P01–P14 have owners. Clarified lost-reply idempotency before version checking, per-analysis identity, edit-during-save, read-only bypass paths, nonfinite measurements and separate service checks. Restricted native dependencies to host/service boundaries. No automatic matching, CAD editing, offline sync, browser delivery, tooling/quotation features or production authentication were added.

The plan selects implementation mechanisms without changing business decisions. Dependency compatibility, database availability and actual build/test results must be established in task 1–3. This machine inspection is not a compatibility guarantee for other Macs. No duration estimate or geometry success rate is asserted. Full MVP still needs the independent tube references and benchmarks already recorded in the specifications.

Dependency documentation consulted: [Axum routing](https://docs.rs/axum/latest/axum/routing/) and [rust_decimal string serialization](https://docs.rs/rust_decimal/latest/rust_decimal/serde/str/index.html). These support the chosen routing and decimal wire representation; compatible versions must be locked during implementation.
