# SB-X01 execution record

```json
{
  "package": "SB-X01",
  "status": "in_progress",
  "brief_ready": true,
  "agent": "elektrikmusik (SB-X01 implementation agent)",
  "worktree": "/Users/hafid/.dsh/state/superagent/work/machine-hourly-rates-72c4f87ad707",
  "branch": "agent/machine-hourly-rates-72c4f87ad707",
  "starting_commit": "6b9add396e9412a391e609adec19b8aa99391833",
  "locked_contracts": [
    {
      "path": "docs/solidbom/contracts/v2/machine-hourly-rates.md",
      "version": "1 (additive to API version 2)",
      "commit": "317e00979f69f53454f8434b865baf634c36c762"
    }
  ],
  "dependency_commits": {
    "SB-09": "7d7dad5aa5b8e62013b304db68438e0404871b36",
    "SB-20": "6b9add396e9412a391e609adec19b8aa99391833"
  },
  "shared_file_reservations": [
    "docs/solidbom/contracts/v2/machine-hourly-rates.md (new) and the version 2 registry row, base sentence and portable-type list in docs/solidbom/contracts/README.md",
    "crates/solidbom-contract/src/machine_rates.rs (new) and its lib.rs module/re-export and crate-doc line; no existing contract type changes",
    "crates/solidbom-rates/ (new pure engine crate: Cargo.toml, lib.rs, engine.rs, recommended.rs, parity.rs, fixtures.rs, calculate_tests.rs, sensitivity_tests.rs) and the root Cargo.lock entry for the new workspace member",
    "xtask/src/layers.rs: one solidbom-rates layering row",
    "services/solidbom-server/migrations/0013_machine_hourly_rates.sql (new; version 13 after SB-20's 0012)",
    "services/solidbom-server/src/machine_rates.rs (new), its lib.rs registration, the six machine-rate routes and handlers in src/api.rs, behaviour-identical pub(crate) visibility changes to i64_version/price_summary/append_change in src/store.rs, the solidbom-rates path dependency in Cargo.toml and the service Cargo.lock",
    "services/solidbom-server/tests/machine_rates.rs (new, 11 tests), the machine-rate route cases in tests/api.rs, the machine-rate fixtures in tests/support/mod.rs, and the migration/route/test tables plus package list line in services/solidbom-server/README.md",
    "docs/solidbom/packages/SB-X01.md status line, docs/solidbom/execution/SB-X01.md (this record) and the new evidence files under docs/solidbom/execution/evidence/: SB-X01-contract-engine-tests.txt, SB-X01-service-tests.txt, SB-X01-repository-gates.txt, SB-X01-native-service.txt, SB-X01-independent-verification.txt"
  ],
  "implementation_commits": [
    "317e00979f69f53454f8434b865baf634c36c762",
    "7a47c3bbb1bfccade208f3733cb26db04b757083",
    "daa06384356f1910dbffe7e2959a1ef57e122796",
    "0a054ee1cc134cba9c0c2feeb66d7038290eca1f"
  ],
  "pr": null,
  "reviewer": "sbx01-verifier (fresh-context independent verifier; no part in the implementation, own /tmp harness, own disposable database). Verdict READY: no blockers, no should-fix findings, 5 notes. It independently wrote a Python Decimal reference from handover 02/03/04 and checked the synthetic golden vector, the eight workbook baselines and Extrusion Line 1's components/sensitivity table, 72 closed-form sensitivity comparisons, all 34-field missing/negative sweeps, the 04 section 6 edge cases, parity-versus-recommended Oven Forming behaviour, overflow-without-panic, determinism and outcome validation: 1,592 checks, 0 failures. It ran the service on loopback against its own disposable database and probed the invalid-publication refusals, pure sensitivity, publication into the SB-09 chain, retry/conflict rules, snapshot reads, supersession chain, permission matrix and migration constraints: 198 checks, 0 failures. It re-ran contract 258, engine 36, service 174, clippy and fmt clean, xtask lists/layers/skills clean; the workspace check is red only on the known coordinator-ledger drift. Notes (none blocking): sensitivity of a published version recalculates the base with the current calculator while leaving the stored snapshot intact (byte-equal under calculator version solidbom-machine-rate/1; a later contract revision should state the rule); the chain filter accepts any i32 financial year and returns an empty list for out-of-range years; the record and evidence were still being written at review time; the workspace-check drift is pre-existing and coordinator-owned; the seven reconstructed machine fixtures cannot be checked against the missing workbook cells.",
  "reviewed_commit": "0a054ee1cc134cba9c0c2feeb66d7038290eca1f",
  "integrated_commit": null,
  "post_integration_checks": [],
  "blocker": null,
  "previous_state": "planned",
  "acceptance_checks": [
    {
      "command": "cargo test -p solidbom-contract --locked",
      "result": "passed",
      "commit": "0a054ee1cc134cba9c0c2feeb66d7038290eca1f",
      "evidence": "258 passed, 0 failed (the predecessor tests plus 13 machine_rates tests). The new tests cover identity and text bounds, missing-versus-zero JSON round trips, outcome status/result invariants, published-versus-draft version shape, sensitivity loading bounds, request validation and the stable wire names. Raw output: docs/solidbom/execution/evidence/SB-X01-contract-engine-tests.txt."
    },
    {
      "command": "cargo test -p solidbom-rates --locked",
      "result": "passed",
      "commit": "0a054ee1cc134cba9c0c2feeb66d7038290eca1f",
      "evidence": "36 passed, 0 failed. Covers the synthetic full-branch golden vector (960 running hours, annual depreciation 5000, annual fully absorbed cost 111800, 116.458333333... per running hour, 131.5625 at loading 0.50 and the 29000 annual full cost at loading 0), the eight workbook baselines and Extrusion Line 1's detailed component vector, the 64 built-in loading-sensitivity cases against the independent closed form plus the nine documented Extrusion Line 1 values, the 04 section 6 edge cases, the reconciliation invariants, Oven Forming's parity-versus-recommended split, all-field missing/negative sweeps, overflow without panic, the business warnings and byte-identical determinism. Raw output: SB-X01-contract-engine-tests.txt."
    },
    {
      "command": "SOLIDBOM_TEST_DATABASE_URL=postgres://hafid@127.0.0.1:5432/solidbom_sbx01 cargo test --manifest-path services/solidbom-server/Cargo.toml --locked",
      "result": "passed",
      "commit": "0a054ee1cc134cba9c0c2feeb66d7038290eca1f",
      "evidence": "174 passed, 0 failed on a fresh random schema per test with the real migrations 0001-0013: lib 3, machine_rates 11, api 47, activation 13, commercial 7, costing 5, engineering 16, feasibility 8, manufacturing 11, persistence 22, projects 9, tooling 6, plus the remaining predecessor suites. The machine-rate suite covers the append-only chain and idempotency (including concurrent identical creates), the exact recommended arithmetic, incomplete drafts that are invalid and never zero-filled, zero-running-hours, publication into the SB-09 chain with the frozen approval snapshot, the refused no-code publication, pure sensitivity (stored bytes and counts unchanged), corrupt-snapshot and corrupt-input codec failures, inventory/chain filters, and the project-snapshot/no-double-charge preservation test. The API route cases cover the permission matrix, create/read/list/sensitivity/publish, idempotent retry bytes, stale and duplicate conflicts, 404/422 malformed ids and filters, and the refused publications. Raw output: SB-X01-service-tests.txt."
    },
    {
      "command": "cargo clippy -p solidbom-contract -p solidbom-rates --all-targets --locked -- -D warnings; cargo clippy --workspace --all-targets --profile ci -- -D warnings; cargo clippy --manifest-path services/solidbom-server/Cargo.toml --all-targets --locked -- -D warnings",
      "result": "passed",
      "commit": "0a054ee1cc134cba9c0c2feeb66d7038290eca1f",
      "evidence": "All three are warning-free with all targets. Non-test engine code uses checked decimal arithmetic only and the crate denies unwrap/expect/panic/todo/unreachable and forbids unsafe; the service module keeps the same rules. Raw output: SB-X01-repository-gates.txt (workspace clippy) and the service evidence line in SB-X01-service-tests.txt."
    },
    {
      "command": "cargo fmt --all -- --check; cargo fmt --manifest-path services/solidbom-server/Cargo.toml -- --check",
      "result": "passed",
      "commit": "0a054ee1cc134cba9c0c2feeb66d7038290eca1f",
      "evidence": "Both workspaces are formatted. Raw output: SB-X01-repository-gates.txt."
    },
    {
      "command": "cargo xtask wasm",
      "result": "passed",
      "commit": "0a054ee1cc134cba9c0c2feeb66d7038290eca1f",
      "evidence": "18 crates build for wasm32-unknown-unknown (the 17 predecessors plus the new solidbom-rates layer-1 crate and its contract types). Raw output: SB-X01-repository-gates.txt."
    },
    {
      "command": "cargo xtask lists; cargo xtask layers; cargo xtask licences --check; cargo xtask skills; cargo xtask assets; cargo xtask icons; cargo xtask ui-containers; cargo xtask mesh-decisions",
      "result": "passed",
      "commit": "0a054ee1cc134cba9c0c2feeb66d7038290eca1f",
      "evidence": "Every non-test repository stage passes: sorted lists and slices; 25 crates with no layering violation; licences current; AGENTS.md and 17 skills consistent; 1084 attributed assets; 375 icon masters; outer UI hosts shared; 181 known mesh-decision sites with no new ones. Raw output: SB-X01-repository-gates.txt."
    },
    {
      "command": "python3 scripts/solidbom_workspace.py check",
      "result": "red only on pre-existing coordinator-ledger drift",
      "commit": "0a054ee1cc134cba9c0c2feeb66d7038290eca1f",
      "evidence": "No diff-caused or record-structure problem. The check reports the known coordinator-owned drift for SB-09..SB-20 (their merged rows/records were never reconciled) plus the expected SB-X01 lines: the ledger still says planned and SB-09's record does not yet say integrated. EXECUTION.md is coordinator-owned; this branch does not edit it. Raw output: SB-X01-repository-gates.txt."
    },
    {
      "command": "live service HTTP run on a disposable PostgreSQL database (built binary from this tree, loopback, dummy development tokens)",
      "result": "passed",
      "commit": "0a054ee1cc134cba9c0c2feeb66d7038290eca1f",
      "evidence": "docs/solidbom/execution/evidence/SB-X01-native-service.txt records the raw status and body for: the 401/403 permission matrix; the administrator draft create with the exact components; an identical retry; read, inventory and filtered chain list; sensitivity at 0.85/0.50/0 (base outcome repeated, zero loading no_run_hours with null per-hour rates); the stored input text before and after the calls (unchanged); publication appending one SB-09 production cny_h/CNY rate row with ACTUAL_RUNNING_HOUR provenance and one cost_center change fact; the identical publication retry; the second-publish 409; the published read answering from the stored snapshot; the rate_versions and reference_changes counts 2->3 and 0->1; the seeded prototype 120 CNY/hour production row byte-identical before and after; the missing-financial-year publication 422 naming missing_financial_year with no row written; and the no-cost-center-code publication 422 naming missing_machine_code. Tokens are redacted; the fixture is synthetic and public-safe."
    },
    {
      "command": "independent verifier: own /tmp harness (own Python Decimal reference plus a Rust harness with path dependencies) at the reviewed tree",
      "result": "passed",
      "commit": "0a054ee1cc134cba9c0c2feeb66d7038290eca1f",
      "evidence": "1,592 independent arithmetic checks and 198 independent live-service checks, 0 failures, verdict READY with no blockers and no should-fix findings. Its own 8-machine closed form, its own golden vector, its own service fixture (independent expected rate 263.16666666666666666666666667 per running hour), its own disposable database, its own migration-constraint probes and its own gate runs. Full summary: docs/solidbom/execution/evidence/SB-X01-independent-verification.txt."
    }
  ],
  "acceptance_gaps": [
    {
      "gap": "No desktop administration surface in this increment",
      "detail": "The contract and this record mark the rate editor and sensitivity table as the next increment, matching how SB-16 through SB-20 deferred their desktop surfaces. The brief allows UI evidence only if implemented, so no before/after UI evidence exists; the observable change is the HTTP API, captured in SB-X01-native-service.txt. The extension remains gated."
    },
    {
      "gap": "Workbook per-cell inputs for seven machines are not in the supplied reference package",
      "detail": "The handover publishes the eight aggregate baselines and Extrusion Line 1's components and sensitivity table only; the machine workbook is not part of docs/solidbom/reference-manifest.json. The parity fixtures reconstruct the seven undocumented per-cell splits, disclosed in crates/solidbom-rates/src/fixtures.rs, and match every documented aggregate. No parity value is seeded or published as an approved company rate."
    },
    {
      "gap": "Owner allocation and validation decisions remain open",
      "detail": "Premises versus SG&A, compressed-air/electricity, cooling-water basis, staffing/loading, residual/depreciation convention, Oven Forming year/power and real cost-center codes are recorded as business warnings or publication refusals instead of invented defaults (contract Open decisions section). The package therefore stays gated; no approved company rate is published."
    },
    {
      "gap": "Hosted heavy CI has not run",
      "detail": "The repository only runs the heavy CI jobs with the maintainer's approved-for-ci label. The local non-test gates, wasm check, clippy, format and the full contract/engine/service suites are green; the known coordinator-ledger drift keeps the workspace check red as documented for the predecessor packages."
    }
  ]
}
```

## Checkpoint

Assigned on the owner's 2026-11-11 instruction on issue
[#38](https://github.com/kaeli-byte/solidcraft/issues/38) ("start implementing"). The workspace is a
clean branch from the integrated main `6b9add396` (the merge of SB-20).

Contract version 1 is locked at `317e00979`: `docs/solidbom/contracts/v2/machine-hourly-rates.md`, the
portable types in `crates/solidbom-contract/src/machine_rates.rs` and the version 2 registry row.
The pure engine crate `crates/solidbom-rates` (`solidbom-machine-rate/1`) is implemented at
`7a47c3bbb`: one deterministic arithmetic path for both calculation modes, strict recommended-mode
validation, the documented Excel-parity defects, non-mutating loading sensitivity and publication
identity checks. The service increment is implemented at `daa063843`: migration 0013 with the
append-only `machine_rate_versions` chain, the store module and six administrator routes, publication
into the SB-09 cost-center rate chain with its reference-change fact, the frozen approval snapshot and
the read rule for published versions. The test-only fixture-table slice edit is `0a054ee1`. The
independent verifier reviewed the working tree equivalent to `0a054ee1` and returned READY; its
findings and evidence are in `docs/solidbom/execution/evidence/SB-X01-independent-verification.txt`.

## Available context and known gaps

- The supplied handover publishes the eight workbook baselines and Extrusion Line 1's component
  vector and sensitivity table, but not the workbook's per-cell inputs for the other seven machines.
  The machine workbook itself is not part of the imported reference package
  (`docs/solidbom/reference-manifest.json`). The parity fixtures are disclosed reconstructions whose
  documented aggregates match exactly; the independent verifier checked the engine arithmetic against
  the documented aggregates and its own closed form, but the reconstruction itself cannot be checked
  against the missing cells.
- The environment's owner data is not read or committed. All fixtures and evidence are public-safe
  synthetic or documented values.
- The extension remains gated: this increment adds the engine, validation, publication service and
  parity evidence, not approved company rates and not the desktop administration surface.

## Exact next action

Open the pull request for the branch, record its link and set this record to `in_review`, then leave
integration to the coordinator: reconcile the execution ledger, obtain the desktop surface/approved
data increments and the owner allocation/validation decisions named in the contract's Open decisions
section. The coordinator should re-run the relevant checks on the integrated base before advancing
the package.

## Checks and evidence

The JSON block above carries the structured acceptance checks. Raw captures:

- [SB-X01-contract-engine-tests.txt](evidence/SB-X01-contract-engine-tests.txt)
- [SB-X01-service-tests.txt](evidence/SB-X01-service-tests.txt)
- [SB-X01-repository-gates.txt](evidence/SB-X01-repository-gates.txt)
- [SB-X01-native-service.txt](evidence/SB-X01-native-service.txt)
- [SB-X01-independent-verification.txt](evidence/SB-X01-independent-verification.txt)

All captures were taken at `0a054ee1` on this branch's worktree (the native-service transcript was
captured at `daa063843`, whose production code is identical to `0a054ee1`; the later commit changes
test fixture tables only).

## Contract changes and follow-up

The contract is additive only: a new contract module, a new table and six new routes, no existing
type, route, status or error code changed. The only edits to existing service files are the
behaviour-identical `pub(crate)` visibility of three store helpers and the route/handler additions.
Consumers: the SB-09 published cost-center rate chain and its reference-change facts (input to SB-20's
adoption surface), and the later desktop administration surface. No other package edits are needed.

Follow-up noted by the independent verifier for a later contract revision: sensitivity of a published
version recalculates the base outcome with the current calculator while leaving the stored snapshot
intact (byte-equal under `solidbom-machine-rate/1`). A future calculator version must decide whether
the sensitivity base is the stored approval snapshot; this increment does not change it. The chain
filter accepts any `i32` financial year and returns an empty list for out-of-range values.

## Review and integration

Independent review is recorded in the JSON block (`reviewer`, `reviewed_commit`). Integration is not
performed here: the coordinator owns the ledger transition, the affected-consumer reconciliation and
the final integrated-base checks.
