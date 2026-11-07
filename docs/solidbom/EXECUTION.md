# SolidBOM execution ledger

Coordinator-owned. Read [the plan](implementation-plan.md), [package instructions](packages/README.md) and [record format](execution/README.md) before assigning work. Status is reconciled against actual commits, reviews and checks; the dependency graph alone never certifies readiness.

## Milestone readiness

| Milestone | State | Evidence |
|---|---|---|
| Prototype accepted | Pending | SB-08 and P01–P14 have not run. |
| MVP engineering/costing complete | Pending | Full engineering, manufacturing, activation and update packages have not run. |
| MVP accepted | Pending | SB-22 has not run. |

## Packages

| Package | Deliverable | Dependencies | Status | Owner |
|---|---|---|---|---|
| [SB-00](packages/SB-00.md) | Requirements and execution records | — | integrated | root (coordinator) |
| [SB-01](packages/SB-01.md) | Contracts and integration foundations | SB-00 | planned | Unassigned |
| [SB-02](packages/SB-02.md) | Prototype cost engine | SB-01 | planned | Unassigned |
| [SB-03](packages/SB-03.md) | Central persistence and calculation API | SB-02 | planned | Unassigned |
| [SB-04](packages/SB-04.md) | CAD source mapping and measurement | SB-01 | planned | Unassigned |
| [SB-05](packages/SB-05.md) | Immutable review sessions | SB-01 | planned | Unassigned |
| [SB-06](packages/SB-06.md) | Desktop service controller | SB-03, SB-04, SB-05 | planned | Unassigned |
| [SB-07](packages/SB-07.md) | Prototype review UI | SB-06 | planned | Unassigned |
| [SB-08](packages/SB-08.md) | Prototype acceptance | SB-07 | planned | Unassigned |
| [SB-09](packages/SB-09.md) | Shared data and permissions | SB-08 | planned | Unassigned |
| [SB-10](packages/SB-10.md) | Projects, deliveries, and part versions | SB-08 | planned | Unassigned |
| [SB-11](packages/SB-11.md) | Engineering review and flat grouping | SB-09, SB-10 | planned | Unassigned |
| [SB-12](packages/SB-12.md) | Tube and protection extraction | SB-08 | planned | Unassigned |
| [SB-13](packages/SB-13.md) | Recognition and catalog suggestions | SB-09, SB-11, SB-12 | planned | Unassigned |
| [SB-14](packages/SB-14.md) | Feasibility rules and exceptions | SB-09, SB-11, SB-12 | planned | Unassigned |
| [SB-15](packages/SB-15.md) | Sourcing and manufacturing definitions | SB-09, SB-10, SB-11 | planned | Unassigned |
| [SB-16](packages/SB-16.md) | Shared demand and manufacturing costs | SB-15 | planned | Unassigned |
| [SB-17](packages/SB-17.md) | Tooling investment and recovery | SB-16 | planned | Unassigned |
| [SB-18](packages/SB-18.md) | Commercial scenarios | SB-17 | planned | Unassigned |
| [SB-19](packages/SB-19.md) | Activation and accepted baselines | SB-14, SB-16, SB-17 | planned | Unassigned |
| [SB-20](packages/SB-20.md) | Dependency updates and adoption | SB-18, SB-19 | planned | Unassigned |
| [SB-21](packages/SB-21.md) | Internal exports | SB-20 | planned | Unassigned |
| [SB-22](packages/SB-22.md) | MVP acceptance and operating handoff | SB-13, SB-21 | planned | Unassigned |
| [SB-X01](packages/SB-X01.md) | Machine hourly-rate construction and sensitivity | SB-09 | planned | Unassigned |

## Current assignment and next dispatch

- SB-00: existing Conductor workspace `kaeli-byte/goals-gaps-review`, starting commit `871014f955a76fd308ce156e68c93e6372906005`. Implementation `b5115cda78f629cd82ef86d1f90e67a20f2a7c69` is independently reviewed and integrated; the owner accepted the documented baseline test failures. Source import and execution infrastructure only. Record: [SB-00](execution/SB-00.md).
- Next candidate: SB-01. Its dependency gate is satisfied; the coordinator must still review and lock its brief, reserve shared files and create the workspace before assignment.
- No product package is assigned or Ready. SB-12/SB-13 customer-data acceptance needs independent references and agreed coverage targets. SB-X01 additionally needs owner allocation/validation decisions.

## Shared-file reservations

| Package | Reserved edits | Release condition |
|---|---|---|
| SB-00 | AGENTS.md pointer, .gitignore, .worktreeinclude, scripts/solidbom_workspace.py, scripts/test_solidbom_workspace.py, execution documents | SB-00 integrated; later agents reserve specific edits before touching these files. |
| SB-01 (future) | Root manifests/layer registrations, protocol exports, host/command scaffolding, service CI | Must be assigned and exact edits pinned before implementation. |

No other reservation is granted by this ledger. Per-package Ownership sections propose future ownership, not concurrent permission to edit shared files.

## Validation and integration log

- Source inspection baseline is recorded above. Imported historical probe results are not current acceptance evidence.
- 2026-11-07: SB-00 implements and documents the import. All 70 manifest files match the owner source; `check` and `check --require-references` pass with 24 records and 53 local references; the main checkout holds 53/53 matching references and a repeat hydrate copies 0.
- 2026-11-07: `scripts/test_solidbom_workspace.py` passes 13 tests; `cargo xtask skills`, `lists` and `licences --check` pass.
- 2026-11-07: Independent review of `b5115cda78f629cd82ef86d1f90e67a20f2a7c69` approved after fixes; findings and evidence are in the SB-00 record.
- 2026-11-07: `cargo xtask ci` fails only in test binaries of crates SB-00 does not change (11 of 89 binaries) on this host; every non-test gate passes with owner references detached. The gate stays unsatisfied and is recorded in the SB-00 record.
- SB-00 integration is recorded at `b5115cda78f629cd82ef86d1f90e67a20f2a7c69`; the owner accepted the documented baseline test failures and the red `cargo xtask ci` gate. The package is `integrated`, not `verified`; post-integration checks are not recorded. No public PR or remote push has been made.
- 2026-11-07: the SB-00 package PR is held by owner decision. The available fork is public and the tracked requirement documents contain customer identifiers, so the branch stays local until a private base repository is chosen. The same PR-per-package rule (E11) applies once that base exists.

## Coordinator reconciliation

Run `python3 scripts/solidbom_workspace.py check --require-references` before closing SB-00. Run `status` for dependency eligibility; verify actual destination-branch ancestry, contract locks and the relevant evidence before changing a package to Ready. Update this summary whenever a record transition is accepted.
