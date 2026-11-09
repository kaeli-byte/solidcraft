# SolidBOM execution ledger

Coordinator-owned. Read [the plan](implementation-plan.md), [package instructions](packages/README.md) and [record format](execution/README.md) before assigning work. Status is reconciled against actual commits, reviews and checks; the dependency graph alone never certifies readiness.

## Acceptance gates

These three gates are the ledger's own readiness claims. A delivery milestone below is not accepted because a tracker says so.

| Gate | State | Evidence |
|---|---|---|
| Prototype accepted | Pending | SB-08 ran P01–P14 on the integrated prototype: P10 is blocked, P13 is partial and the repository gate is red; the gate stays pending. |
| MVP engineering/costing complete | Pending | Full engineering, manufacturing, activation and update packages have not run. |
| MVP accepted | Pending | SB-22 has not run. |

## Delivery milestones

Grouping mirrored from the Linear `solidCraft` project milestones and the GitHub milestones. The state column follows the package table below and the integrated evidence, not the tracker's own status.

| Milestone | State | Packages | Tracking |
|---|---|---|---|
| 1. Execution and contract foundations | In progress | SB-00, SB-01 | [KAE-11](https://linear.app/kaeli-byte/issue/KAE-11/establish-execution-and-contract-foundations) · [milestone 1](https://github.com/kaeli-byte/solidcraft/milestone/1) |
| 2. Approved prototype accepted | In progress | SB-02–SB-08 | [KAE-12](https://linear.app/kaeli-byte/issue/KAE-12/deliver-and-accept-the-approved-prototype) · [milestone 2](https://github.com/kaeli-byte/solidcraft/milestone/2) |
| 3. Shared engineering data and geometry | Not started | SB-09–SB-12 | [KAE-13](https://linear.app/kaeli-byte/issue/KAE-13/deliver-shared-engineering-data-and-geometry) · [milestone 3](https://github.com/kaeli-byte/solidcraft/milestone/3) |
| 4. Recognition, feasibility, and manufacturing definitions | Not started | SB-13–SB-15 | [KAE-14](https://linear.app/kaeli-byte/issue/KAE-14/deliver-recognition-feasibility-and-manufacturing-definitions) · [milestone 4](https://github.com/kaeli-byte/solidcraft/milestone/4) |
| 5. MVP engineering and costing complete | Not started | SB-16–SB-19 | [KAE-15](https://linear.app/kaeli-byte/issue/KAE-15/complete-mvp-engineering-and-costing) · [milestone 5](https://github.com/kaeli-byte/solidcraft/milestone/5) |
| 6. Dependency adoption and internal exports | Not started | SB-20, SB-21 | [KAE-16](https://linear.app/kaeli-byte/issue/KAE-16/deliver-dependency-adoption-and-internal-exports) · [milestone 6](https://github.com/kaeli-byte/solidcraft/milestone/6) |
| 7. MVP accepted and operating handoff | Not started | SB-22 | [KAE-17](https://linear.app/kaeli-byte/issue/KAE-17/accept-the-mvp-and-complete-operating-handoff) · [milestone 7](https://github.com/kaeli-byte/solidcraft/milestone/7) |
| 8. Gated extension: Machine hourly rates | Gated | SB-X01 | [KAE-18](https://linear.app/kaeli-byte/issue/KAE-18/deliver-the-gated-machine-hourly-rate-extension) · [milestone 8](https://github.com/kaeli-byte/solidcraft/milestone/8) |

## Packages

| Package | Deliverable | Dependencies | Status | Owner | Tracking |
|---|---|---|---|---|---|
| [SB-00](packages/SB-00.md) | Requirements and execution records | — | verified | root (coordinator) | [KAE-19](https://linear.app/kaeli-byte/issue/KAE-19/sb-00-requirements-and-execution-records) · [#15](https://github.com/kaeli-byte/solidcraft/issues/15) |
| [SB-01](packages/SB-01.md) | Contracts and integration foundations | SB-00 | verified | Claude (SB-01 implementation agent) | [KAE-20](https://linear.app/kaeli-byte/issue/KAE-20/sb-01-contracts-and-integration-foundations) · [#16](https://github.com/kaeli-byte/solidcraft/issues/16) |
| [SB-02](packages/SB-02.md) | Prototype cost engine | SB-01 | integrated | Claude (SB-02 implementation agent) | [KAE-21](https://linear.app/kaeli-byte/issue/KAE-21/sb-02-prototype-cost-engine) · [#17](https://github.com/kaeli-byte/solidcraft/issues/17) |
| [SB-03](packages/SB-03.md) | Central persistence and calculation API | SB-02 | verified | elektrikmusik (SB-03 implementation and verification agent) | [KAE-22](https://linear.app/kaeli-byte/issue/KAE-22/sb-03-central-persistence-and-calculation-api) · [#18](https://github.com/kaeli-byte/solidcraft/issues/18) |
| [SB-04](packages/SB-04.md) | CAD source mapping and measurement | SB-01 | integrated | deepseek-flash (SB-04 implementation agent) | [KAE-23](https://linear.app/kaeli-byte/issue/KAE-23/sb-04-cad-source-mapping-and-measurement) · [#19](https://github.com/kaeli-byte/solidcraft/issues/19) |
| [SB-05](packages/SB-05.md) | Immutable review sessions | SB-01 | integrated | DeepSeek Harness (SB-05 implementation agent) | [KAE-24](https://linear.app/kaeli-byte/issue/KAE-24/sb-05-immutable-review-sessions) · [#20](https://github.com/kaeli-byte/solidcraft/issues/20) |
| [SB-06](packages/SB-06.md) | Desktop service controller | SB-03, SB-04, SB-05 | integrated | elektrikmusik (SB-06 implementation agent) | [KAE-25](https://linear.app/kaeli-byte/issue/KAE-25/sb-06-desktop-service-controller) · [#21](https://github.com/kaeli-byte/solidcraft/issues/21) |
| [SB-07](packages/SB-07.md) | Prototype review UI | SB-06 | integrated | elektrikmusik (SB-07 implementation agent) | [KAE-26](https://linear.app/kaeli-byte/issue/KAE-26/sb-07-prototype-review-ui) · [#22](https://github.com/kaeli-byte/solidcraft/issues/22) |
| [SB-08](packages/SB-08.md) | Prototype acceptance | SB-07 | integrated | elektrikmusik (SB-08 acceptance agent) | [KAE-27](https://linear.app/kaeli-byte/issue/KAE-27/sb-08-prototype-acceptance) · [#23](https://github.com/kaeli-byte/solidcraft/issues/23) |
| [SB-09](packages/SB-09.md) | Shared data and permissions | SB-08 | planned | Unassigned | [KAE-28](https://linear.app/kaeli-byte/issue/KAE-28/sb-09-shared-data-and-permissions) · [#24](https://github.com/kaeli-byte/solidcraft/issues/24) |
| [SB-10](packages/SB-10.md) | Projects, deliveries, and part versions | SB-08 | in_progress | elektrikmusik (SB-10 implementation agent) | [KAE-29](https://linear.app/kaeli-byte/issue/KAE-29/sb-10-projects-deliveries-and-part-versions) · [#25](https://github.com/kaeli-byte/solidcraft/issues/25) |
| [SB-11](packages/SB-11.md) | Engineering review and flat grouping | SB-09, SB-10 | planned | Unassigned | [KAE-30](https://linear.app/kaeli-byte/issue/KAE-30/sb-11-engineering-review-and-flat-grouping) · [#26](https://github.com/kaeli-byte/solidcraft/issues/26) |
| [SB-12](packages/SB-12.md) | Tube and protection extraction | SB-08 | planned | Unassigned | [KAE-31](https://linear.app/kaeli-byte/issue/KAE-31/sb-12-tube-and-protection-extraction) · [#27](https://github.com/kaeli-byte/solidcraft/issues/27) |
| [SB-13](packages/SB-13.md) | Recognition and catalog suggestions | SB-09, SB-11, SB-12 | planned | Unassigned | [KAE-42](https://linear.app/kaeli-byte/issue/KAE-42/sb-13-recognition-and-catalog-suggestions) · [#28](https://github.com/kaeli-byte/solidcraft/issues/28) |
| [SB-14](packages/SB-14.md) | Feasibility rules and exceptions | SB-09, SB-11, SB-12 | planned | Unassigned | [KAE-32](https://linear.app/kaeli-byte/issue/KAE-32/sb-14-feasibility-rules-and-exceptions) · [#29](https://github.com/kaeli-byte/solidcraft/issues/29) |
| [SB-15](packages/SB-15.md) | Sourcing and manufacturing definitions | SB-09, SB-10, SB-11 | planned | Unassigned | [KAE-33](https://linear.app/kaeli-byte/issue/KAE-33/sb-15-sourcing-and-manufacturing-definitions) · [#30](https://github.com/kaeli-byte/solidcraft/issues/30) |
| [SB-16](packages/SB-16.md) | Shared demand and manufacturing costs | SB-15 | planned | Unassigned | [KAE-34](https://linear.app/kaeli-byte/issue/KAE-34/sb-16-shared-demand-and-manufacturing-costs) · [#31](https://github.com/kaeli-byte/solidcraft/issues/31) |
| [SB-17](packages/SB-17.md) | Tooling investment and recovery | SB-16 | planned | Unassigned | [KAE-35](https://linear.app/kaeli-byte/issue/KAE-35/sb-17-tooling-investment-and-recovery) · [#32](https://github.com/kaeli-byte/solidcraft/issues/32) |
| [SB-18](packages/SB-18.md) | Commercial scenarios | SB-17 | planned | Unassigned | [KAE-36](https://linear.app/kaeli-byte/issue/KAE-36/sb-18-commercial-scenarios) · [#33](https://github.com/kaeli-byte/solidcraft/issues/33) |
| [SB-19](packages/SB-19.md) | Activation and accepted baselines | SB-14, SB-16, SB-17 | planned | Unassigned | [KAE-37](https://linear.app/kaeli-byte/issue/KAE-37/sb-19-activation-and-accepted-baselines) · [#34](https://github.com/kaeli-byte/solidcraft/issues/34) |
| [SB-20](packages/SB-20.md) | Dependency updates and adoption | SB-18, SB-19 | planned | Unassigned | [KAE-38](https://linear.app/kaeli-byte/issue/KAE-38/sb-20-dependency-updates-and-adoption) · [#35](https://github.com/kaeli-byte/solidcraft/issues/35) |
| [SB-21](packages/SB-21.md) | Internal exports | SB-20 | planned | Unassigned | [KAE-39](https://linear.app/kaeli-byte/issue/KAE-39/sb-21-internal-exports) · [#36](https://github.com/kaeli-byte/solidcraft/issues/36) |
| [SB-22](packages/SB-22.md) | MVP acceptance and operating handoff | SB-13, SB-21 | planned | Unassigned | [KAE-40](https://linear.app/kaeli-byte/issue/KAE-40/sb-22-mvp-acceptance-and-operating-handoff) · [#37](https://github.com/kaeli-byte/solidcraft/issues/37) |
| [SB-X01](packages/SB-X01.md) | Machine hourly-rate construction and sensitivity | SB-09 | planned | Unassigned | [KAE-41](https://linear.app/kaeli-byte/issue/KAE-41/sb-x01-machine-hourly-rate-construction-and-sensitivity) · [#38](https://github.com/kaeli-byte/solidcraft/issues/38) |

## Current assignment and next dispatch

- SB-00: existing Conductor workspace `kaeli-byte/goals-gaps-review`, starting commit `871014f955a76fd308ce156e68c93e6372906005`. Implementation `b5115cda78f629cd82ef86d1f90e67a20f2a7c69` is independently reviewed and merged as [PR #10](https://github.com/kaeli-byte/solidcraft/pull/10) at `aadc9b31bb6a4ffb30990818e1a0624c82bf2386`; post-integration checks pass and the package is `verified`. The owner accepted the documented baseline test failures. Source import and execution infrastructure only. Record: [SB-00](execution/SB-00.md).
- SB-01: verified at `8ff52d0454df27fa32176176fc950de7d479ccbc` (PR #14); version 1 contracts published at `1ab7b3b28c08b9a7afadee725c95f62465403ef7`; record [SB-01](execution/SB-01.md).
- SB-02–SB-08: implemented and integrated through PRs #13 and #41–#48; each record carries its integration commit, reviewed commit and review evidence, and the known gaps are in decision E13. SB-08's P10 is blocked, P13 is partial and the repository gate is red on the documented host baseline; only SB-03 carries verification evidence, and SB-12/SB-13 customer-data acceptance still needs independent references and agreed coverage targets.
- SB-10: in progress. Workspace and branch `agent/projects-deliveries-versions-ba667f5ec436`, starting commit `3c355556b2bafe19ced67e785aa8bee8fff16079`. The owner instructed the worker to lock the contracts and start implementation: version 2 of the project/delivery contract is locked at `8693b01ac`, the append-only project store, migration 0003 and the `/v2/projects` routes are committed at `1a3f16952` and `4236d9519`, and the implementation pull request is [#53](https://github.com/kaeli-byte/solidcraft/pull/53); the review-round-1 fixes are at `55001a6f4`. The desktop commands, controller, client and UI wiring and the native workflow evidence remain open; record [SB-10](execution/SB-10.md). SB-09 (#24) and SB-12 (#27) were dispatched by the owner on 2026-11-08 without recorded assignment metadata.
- Next: finish the SB-10 desktop wiring and capture the native partial-delivery, revision and exclusion evidence, then independent review. SB-09 and SB-12 must reserve their shared edits before touching the command registry, contract exports or migrations. SB-X01 additionally needs owner allocation/validation decisions.

## Shared-file reservations

| Package | Reserved edits | Release condition |
|---|---|---|
| SB-00 | AGENTS.md pointer, .gitignore, .worktreeinclude, scripts/solidbom_workspace.py, scripts/test_solidbom_workspace.py, execution documents | SB-00 verified; later agents reserve specific edits before touching these files. |
| SB-01 | Contract, host and command scaffolding; the exact files are listed in [its record](execution/SB-01.md) | Released: SB-01 is verified. |
| SB-10 | New contract `docs/solidbom/contracts/v2/projects-deliveries.md` and its registry row in `docs/solidbom/contracts/README.md`; the new `crates/solidbom-contract/src/projects.rs` module and its export in `crates/solidbom-contract/src/lib.rs`, plus additive changes in `crates/solidbom-contract/src/types.rs`; new migration `services/solidbom-server/migrations/0003_projects_deliveries.sql` (version 3 reserved); new `services/solidbom-server/src/projects.rs` and `deliveries.rs`, their registrations in `src/lib.rs`, `src/api.rs` and `src/store.rs`, and additions to `tests/api.rs` and `tests/persistence.rs`; new `crates/engine/src/solidbom/projects.rs` and `deliveries.rs`, controller additions in `crates/engine/src/solidbom/state.rs`, command registration in `crates/engine/src/cmd/solidbom.rs` and `crates/engine/src/schema/solidbom.rs`, and `crates/engine/src/solidbom/tests.rs`; `apps/solidcraft/src/solidbom_client.rs`; `crates/ui-egui/src/solidbom/panel.rs`, `workspace.rs`, `jobs.rs`, `tests.rs` and one new `assets/locales/pending/` string file; `docs/solidbom/execution/SB-10.md`. | SB-10 integrated. SB-09 and SB-12 were dispatched before this reservation: they must not edit these paths without a new coordinator decision, and the coordinator serializes migration numbers, the contract registry and the command registry. |

SB-09 and SB-12 have no shared-file reservation yet; each must reserve its shared edits before changing the command registry, contract exports or migrations. Per-package Ownership sections propose future ownership, not concurrent permission to edit shared files.

## Validation and integration log

- Source inspection baseline is recorded above. Imported historical probe results are not current acceptance evidence.
- 2026-11-07: SB-00 implements and documents the import. All 70 manifest files match the owner source; `check` and `check --require-references` pass with 24 records and 53 local references; the main checkout holds 53/53 matching references and a repeat hydrate copies 0.
- 2026-11-07: `scripts/test_solidbom_workspace.py` passes 13 tests; `cargo xtask skills`, `lists` and `licences --check` pass.
- 2026-11-07: Independent review of `b5115cda78f629cd82ef86d1f90e67a20f2a7c69` approved after fixes; findings and evidence are in the SB-00 record.
- 2026-11-07: `cargo xtask ci` fails only in test binaries of crates SB-00 does not change (11 of 89 binaries) on this host; every non-test gate passes with owner references detached. The gate stays unsatisfied and is recorded in the SB-00 record.
- SB-00 merged as [PR #10](https://github.com/kaeli-byte/solidcraft/pull/10) at `aadc9b31bb6a4ffb30990818e1a0624c82bf2386`; the owner accepted the documented baseline test failures and the red `cargo xtask ci` gate. Post-integration checks pass on the integrated base (13/13 tool tests, `check --require-references`, skills/lists/licences), so the package is `verified`. The gate failure stays recorded as unsatisfied.
- 2026-11-07: the owner authorised publishing the fork's public repository with the tracked requirement snapshot. The same PR-per-package rule (E11) applies to later packages.
- 2026-11-08: Tracker reconciliation. Every package now has a GitHub issue in its delivery milestone (#15–#38; milestone 8 added for the gated extension), and this ledger carries the milestone and tracking map. #15 is closed on the verified SB-00 evidence; #16 stays open because SB-01 is `ready_to_integrate`, not verified. In Linear the duplicate issues created by the GitHub integration were archived (KAE-43–KAE-44 and KAE-45–KAE-66) and blocked-by relations now mirror the dependency graph. The GitHub→Linear integration imports every GitHub issue, so new GitHub issues reappear as Linear duplicates until its filter is changed; Linear remains the milestone/parent roadmap and GitHub the public feature-request surface.
- 2026-11-08: SB-01 integrated as [PR #14](https://github.com/kaeli-byte/solidcraft/pull/14) at `8ff52d0454df27fa32176176fc950de7d479ccbc`; post-integration checks pass on that base (14 tool tests, `check --require-references`, skills/lists/licences, the contract/engine/schema/client suites and the fork CI), so the package is `verified`. The local `cargo xtask ci` gate stays red only in the accepted baseline binaries.
- 2026-11-09: Coordinator reconciliation from the merged prototype pull requests (decision E13). SB-02–SB-08 records now carry integration commits, reviewed commits, review evidence and the known gaps; `python3 scripts/solidbom_workspace.py check` passes on this reconciliation, and `scripts/test_solidbom_workspace.py` passes. SB-10 shared-file reservations are recorded and its workspace, branch and base commit are in its record; the project/delivery contract lock is still open.
- 2026-11-09: SB-10 locks the version 2 project/delivery contract at `8693b01ac` and commits the service implementation at `1a3f16952`, `4236d9519` and the review-round-1 fixes at `55001a6f4` ([PR #53](https://github.com/kaeli-byte/solidcraft/pull/53)). Contract tests 53 passed after the review fixes; service api 29, persistence 15 and projects 9 passed on a disposable database; clippy and fmt clean on both crates; `python3 scripts/solidbom_workspace.py check` passes. The desktop wiring and native evidence remain open.

## Coordinator reconciliation

Run `python3 scripts/solidbom_workspace.py check --require-references` before closing a package. Run `status` for dependency eligibility; verify actual destination-branch ancestry, contract locks and the relevant evidence before changing a package to Ready. Update this summary whenever a record transition is accepted. The Linear and GitHub items mirror this ledger: change the package state here first, then reconcile the tracker. Tracker states are never acceptance evidence, and a GitHub issue does not close on merge, only when this ledger records integration and its checks.
