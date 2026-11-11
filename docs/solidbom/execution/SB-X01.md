# SB-X01 execution record

```json
{
  "package": "SB-X01",
  "status": "in_review",
  "brief_ready": true,
  "agent": "elektrikmusik (SB-X01 implementation agent)",
  "worktree": "/Users/hafid/.dsh/state/superagent/work/machine-hourly-rates-72c4f87ad707",
  "branch": "agent/machine-hourly-rates-72c4f87ad707",
  "starting_commit": "6b9add396e9412a391e609adec19b8aa99391833",
  "locked_contracts": [
    {
      "path": "docs/solidbom/contracts/v2/machine-hourly-rates.md",
      "version": "1 (additive to API version 2; amended with the parity exception at 7a47c3b and the publication-ordering rule at 69022d53)",
      "commit": "317e00979f69f53454f8434b865baf634c36c762"
    }
  ],
  "dependency_commits": {
    "SB-09": "7d7dad5aa5b8e62013b304db68438e0404871b36",
    "SB-20": "6b9add396e9412a391e609adec19b8aa99391833",
    "SB-21": "bc5418d1e89dac73d12cd294e04fc48247408763"
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
    "0a054ee1cc134cba9c0c2feeb66d7038290eca1f",
    "69022d5394185ef8185d746ad5e74cd9deff7ea2",
    "5b7ff09f95cb5d25349cddf54cf81d2f6944a490",
    "7df71b04b821c628d1fbe226da22b53dda50de39"
  ],
  "pr": "https://github.com/kaeli-byte/solidcraft/pull/66",
  "reviewer": "sbx01-verifier (fresh-context independent verifier; no part in the implementation, own /tmp harness, own disposable database) returned READY with no blockers and no should-fix findings: 1,592 independent arithmetic checks and 198 independent live-service checks, 0 failures, own Python Decimal reference from handover 02/03/04, own service fixture and disposable database, own migration-constraint probes and gate runs. The repository automated reviewer (pullfrog) review round 1 on head b131308 reported no new issues. The ticket's independent Reviewer review round 1 on head 5c86e2f8e returned CHANGES with one should-fix and one nit. R1 [should, valid, fixed at 69022d53]: publication accepted a superseded draft because it checked the selected draft's own version instead of the chain's newest; the service now locks the (machine, financial year, scenario) chain, requires the selected version to be its newest and refuses a superseded draft as 409, with reverse-order and concurrent-publication regression tests and a live-service transcript of the refusal. R2 [nit, declined]: the workspace check fails only on the pre-existing coordinator-owned ledger drift (EXECUTION.md and the SB-09 record are coordinator-owned; this record documents the lines and the coordinator reconciles them), and the reviewer's sandbox lacked PostgreSQL and GitHub so it could not run the service suite; the full 176-test suite and the live HTTP transcript were reproduced locally and by the fresh-context verifier, and are committed as evidence. Review round 2 on head 580e474be confirmed R1 resolved, lock ordering and retry behaviour consistent, and dropped the coordinator-ledger item. R2 [nit, declined]: \"independent execution remains unverified\" is a limitation of the reviewer's sandbox (no PostgreSQL or GitHub), not of the code or the delta under review; the full 176-test service suite was executed on this worktree against PostgreSQL 18.6 and independently by the fresh-context verifier on its own disposable database, the raw transcript is committed, and the exact reproduction commands and tool versions are recorded under Checks and evidence, so the next round only needs database access to rerun it. Review round 3 on head de2f5c22f found no new code defect and confirmed the publication-ordering finding closed; its only finding repeated the environment limitation (PostgreSQL returned \"Operation not permitted\" and GitHub was unreachable in the reviewer sandbox) and it explicitly requested no source change. That is the third review round and the item is reviewer-environment access, not the code, so this record is the final disposition: no further automated review round is requested. A human reviewer with database access can rerun the exact checks with the reproduction commands under Checks and evidence.",
  "reviewed_commit": "69022d5394185ef8185d746ad5e74cd9deff7ea2",
  "integrated_commit": null,
  "post_integration_checks": [],
  "blocker": null,
  "previous_state": "planned",
  "acceptance_checks": [
    {
      "command": "cargo test -p solidbom-contract --locked",
      "result": "passed",
      "commit": "0a054ee1cc134cba9c0c2feeb66d7038290eca1f",
      "evidence": "269 passed, 0 failed on the merged tree (the predecessor tests including SB-21 exports, plus 13 machine_rates tests). The new tests cover identity and text bounds, missing-versus-zero JSON round trips, outcome status/result invariants, published-versus-draft version shape, sensitivity loading bounds, request validation and the stable wire names. Raw output: docs/solidbom/execution/evidence/SB-X01-contract-engine-tests.txt."
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
      "commit": "69022d5394185ef8185d746ad5e74cd9deff7ea2",
      "evidence": "207 passed, 0 failed (one suite reports 2 ignored) on a fresh random schema per test with the real migrations 0001-0013: lib 3, machine_rates 13, api 48, activation 13, commercial 7, costing 5, engineering 16, feasibility 8, manufacturing 11, persistence 22, projects 9, tooling 6, plus the remaining predecessor suites. The machine-rate suite covers the append-only chain and idempotency (including concurrent identical creates), the exact recommended arithmetic, incomplete drafts that are invalid and never zero-filled, zero-running-hours, publication into the SB-09 chain with the frozen approval snapshot, the refused no-code publication, pure sensitivity (stored bytes and counts unchanged), corrupt-snapshot and corrupt-input codec failures, inventory/chain filters, the project-snapshot/no-double-charge preservation test, and the review round 1 regressions: a superseded draft is refused in reverse order and two concurrently published drafts append exactly one shared rate. The API route cases cover the permission matrix, create/read/list/sensitivity/publish, idempotent retry bytes, stale and duplicate conflicts, 404/422 malformed ids and filters, and the refused publications. Raw output: SB-X01-service-tests.txt."
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
      "commit": "69022d5394185ef8185d746ad5e74cd9deff7ea2",
      "evidence": "docs/solidbom/execution/evidence/SB-X01-native-service.txt records the raw status and body for: the 401/403 permission matrix; the administrator draft create with the exact components; an identical retry; read, inventory and filtered chain list; sensitivity at 0.85/0.50/0 (base outcome repeated, zero loading no_run_hours with null per-hour rates); the stored input text before and after the calls (unchanged); publication appending one SB-09 production cny_h/CNY rate row with ACTUAL_RUNNING_HOUR provenance and one cost_center change fact; the identical publication retry; the second-publish 409; the published read answering from the stored snapshot; the rate_versions and reference_changes counts 2->3 and 0->1; the seeded prototype 120 CNY/hour production row byte-identical before and after; the missing-financial-year publication 422 naming missing_financial_year with no row written; the no-cost-center-code publication 422 naming missing_machine_code; the two-draft chain whose later SB-09 rate supersedes the earlier one; and, after review round 1, the reverse-order publication of an older draft refused as 409 with the message naming the superseding chain version while the older draft stays a draft and only one shared rate exists on its cost center. Tokens are redacted; the fixture is synthetic and public-safe."
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
      "gap": "The ticket Reviewer could not execute the service suite in its own sandbox",
      "detail": "Review rounds 1-3 ran with PostgreSQL unreachable (round 3: \"Operation not permitted\") and GitHub unavailable, so the Reviewer could not independently reproduce the service checks or read hosted CI itself. It requested no source change. Independent execution evidence therefore comes from this worktree's PostgreSQL 18.6 run (176 service tests, raw transcript committed), the fresh-context verifier's own disposable database run (198 live-service checks), and the committed live HTTP transcript; the exact one-command reproduction steps are recorded under Checks and evidence, and a human reviewer with database access can rerun them after the three-round review cap."
    },
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
      "gap": "Pre-existing RustSec advisory job fails on the base too",
      "detail": "The PR's Dependency advisories job runs because the diff touches Cargo.lock/Cargo.toml, and it fails on RustSec advisories already present in main's root lockfile (for example RUSTSEC-2026-0486 and the yanked async-executor/clipboard-win entries). The same job fails on main at the integrated base 6b9add396, and this diff adds only a path-only workspace package with dependencies already in the lockfile; upgrading unrelated dependencies is outside this package's scope."
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
The increment is published for review as [PR #66](https://github.com/kaeli-byte/solidcraft/pull/66).

Review round 1 (the ticket's independent Reviewer, head `5c86e2f8e`) raised one should-fix: publication
checked the selected draft's own version instead of the chain's newest, so a superseded draft could
append obsolete assumptions as a newer shared rate. Fixed at `69022d539`: publication now takes the
same (machine, financial year, scenario) chain lock as creation, requires the selected version to be
that chain's newest, and refuses a superseded draft as `409 conflict`, with reverse-order and
concurrent-publication regression tests (machine-rate suite 11 -> 13, service suite 174 -> 176) and a
live-service transcript of the refusal. The Reviewer's nit (workspace-check drift and its blocked
PostgreSQL/GitHub access) is declined as coordinator/environment-owned: the ledger and EXECUTION.md
are coordinator-owned and the record documents the lines, while the suite and live transcript were
reproduced locally and independently.

Merge 2026-11-12: `main` advanced to `bc5418d1e` (the SB-21 internal-exports merge, PR #67) and the
pull request became conflicting. The repository ruleset `no-force-push` forbids rewriting any branch
and has no bypass actors, so the conflict could not be resolved by rebase-and-force-push (the local
rebased branch was rejected). The conflicts were therefore resolved with a merge commit on top of the
reviewed head (`5b7ff09f9`), which keeps the original reviewed commit hashes and merges the integrated
main. Conflicts were only where both packages extend the same registries: the contract crate's
module/doc list, the contract registry base sentences/portable types/rows, the service README's
package list and the service API test file; both sides were kept, with no behaviour change to either
package. Every check was re-run on the merged tree: contract 269, engine 36, service 207 (including
SB-21's export suites), clippy and formatting clean, wasm 18 crates and the repository gates green
(workspace check red only on the coordinator-ledger drift). The earlier review verdicts cover the same
code and content. The automated reviewer (pullfrog) then flagged a doubled "and" in the conflict-resolved
portable-types list in the contract registry; that documentation-only defect was fixed at `7df71b04b`
and the other conflict resolutions were re-checked for the same kind of artefact.

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

The pull request is open at the review round 1 revision. The next review round can check the
revised head; after that, leave integration to the coordinator: review and merge
[PR #66](https://github.com/kaeli-byte/solidcraft/pull/66), reconcile the execution ledger, and obtain
the desktop surface/approved data increments and the owner allocation/validation decisions named in
the contract's Open decisions section. The coordinator should re-run the relevant checks on the
integrated base before advancing the package.

## Checks and evidence

The JSON block above carries the structured acceptance checks. Raw captures:

- [SB-X01-contract-engine-tests.txt](evidence/SB-X01-contract-engine-tests.txt)
- [SB-X01-service-tests.txt](evidence/SB-X01-service-tests.txt)
- [SB-X01-repository-gates.txt](evidence/SB-X01-repository-gates.txt)
- [SB-X01-native-service.txt](evidence/SB-X01-native-service.txt)
- [SB-X01-independent-verification.txt](evidence/SB-X01-independent-verification.txt)

All five captures were re-taken at the merged head `5b7ff09f9` after the main conflict resolution;
the earlier captures were at pre-merge commits covering the same code plus the review round 1 fix.
The independent verification harness was written and run against the equivalent pre-merge code; its
findings and numbers are reproduced in SB-X01-independent-verification.txt.

How to reproduce the service verification on a machine with PostgreSQL (the reviewer's sandbox lacked
it; nothing in this package needs GitHub access to run the checks):

```sh
# PostgreSQL 18.6 and Rust 1.99.1 were used for the captured run.
createdb -h 127.0.0.1 -U hafid solidbom_sbx01
SOLIDBOM_TEST_DATABASE_URL=postgres://hafid@127.0.0.1:5432/solidbom_sbx01 \
    cargo test --manifest-path services/solidbom-server/Cargo.toml --locked
SOLIDBOM_TEST_DATABASE_URL=postgres://hafid@127.0.0.1:5432/solidbom_sbx01 \
    cargo clippy --manifest-path services/solidbom-server/Cargo.toml --all-targets --locked -- -D warnings
cargo fmt --manifest-path services/solidbom-server/Cargo.toml -- --check
```

Each test creates and drops its own random schema, so a disposable database is enough and no hosted
service is needed. For the live HTTP transcript, run the built
`services/solidbom-server/target/debug/solidbom-server` with `SOLIDBOM_DATABASE_URL` pointing at a
disposable database, `SOLIDBOM_ADMIN_TOKEN`/`SOLIDBOM_ENGINEER_TOKEN` set to distinct dummy strings of
at least 16 characters, and `SOLIDBOM_BIND=127.0.0.1:<port>`; the transcript in
`SB-X01-native-service.txt` records every request and status.

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

Independent review is recorded in the JSON block (`reviewer`, `reviewed_commit`): a fresh-context
independent verifier returned READY with no blockers, the repository's automated reviewer (pullfrog)
reported no new issues on head `b131308`, and the ticket's independent Reviewer review round 1 on
head `5c86e2f8e` returned CHANGES with one should-fix and one nit. The should-fix is addressed at
`69022d539` and covered by regression tests and live evidence; the nit is declined because the
workspace-check lines are pre-existing coordinator-owned ledger drift (EXECUTION.md and the SB-09
record are not this package's files) and the reviewer's own environment could not run PostgreSQL or
GitHub, while this worktree and the fresh-context verifier both reproduced the full suite and live
transcript. Review round 2 confirmed the fix and the lock ordering and left only the same environment
nit, which the reproduction commands under Checks and evidence answer for any reviewer with database
access. Review round 3 found no new code defect and repeated only the same environment limitation with
no source change requested; that is the third round, so no further automated review round is asked
for and the limitation is stated in `acceptance_gaps` for a human reviewer with database access. No
review finding is open for code. Integration is not performed here: the coordinator
owns the ledger transition, the affected-consumer reconciliation and the final integrated-base
checks; the next review round can check the revised head.
