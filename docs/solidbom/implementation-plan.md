# SolidBOM delivery through tracked worktree packages

Adopted from the owner-approved package plan. Baseline reviewed: `871014f95`, SolidCraft `0.40.0-rc.2`. See [package-index.json](package-index.json) for the machine-readable dependency graph and [requirements-map.md](requirements-map.md) for coverage.

## 1. Delivery model

Cover the **full confirmed MVP**, delivering the approved P01–P14 prototype first.

Each technical package has:

- One coding agent and one dedicated worktree.
- A versioned brief defining scope, interfaces, owned files, dependencies, and acceptance checks.
- A progress/handoff record containing commits, validation results, evidence, and remaining issues.
- Independent review before integration.
- An explicit completion decision based on the integrated code.

A coordinator maintains the overall execution ledger and assigns only packages whose prerequisites are satisfied. Independent packages run concurrently; dependent packages start from commits containing their completed prerequisites.

The user creates Conductor workspaces. The coordinator supplies each workspace’s package brief, starting commit, and agent instructions. Existing branches are not renamed.

Retain the reviewed architecture: Rust/egui desktop, portable contracts, backend-only authoritative costing, and a separate Rust/PostgreSQL service.

## 2. Repository-based execution tracking

Create this tracked structure:

```text
docs/solidbom/
  README.md
  EXECUTION.md
  requirements/
    cost_calculator_handover/
    machine_hourly_handover/
  contracts/
  packages/
    SB-00.md
    SB-01.md
    ...
  execution/
    SB-00.md
    SB-01.md
    ...
  reference-manifest.json
```

**Package briefs** contain:

- Package ID, objective, and requirements covered.
- Included behavior and explicit exclusions.
- Dependencies and required contract versions.
- Public types, commands, API changes, and persistence changes.
- Owned files/modules and permitted shared-file edits.
- Acceptance tests, required checks, and evidence requirements.
- Missing inputs or decisions that prevent assignment.

**Execution records** contain:

- Status, assigned agent, worktree, branch, and starting commit.
- Last completed checkpoint and exact next action.
- Implementation commits and optional PR link.
- Checks actually run, results, and evidence locations.
- Blockers, contract changes, and follow-up work.
- Reviewed commit, integrated commit, and post-integration results.

Use these states:

`planned → ready → in_progress → in_review → ready_to_integrate → integrated → verified`

`blocked` records the cause, responsible decision/input, and previous state.

Only the coordinator edits `EXECUTION.md`; each agent edits its own execution record. This avoids a shared progress-file conflict across every worktree. The coordinator reconciles records using actual commits and review evidence.

**“Verified” means integrated and checked.** A package passing tests only on its own branch remains `ready_to_integrate`.

Map every confirmed requirement and acceptance scenario to an owning package. Historical workbook rules and proposed behavior remain clearly distinguished from approved requirements.

## 3. Technical package roadmap

Dependencies below mean **integrated prerequisites**, not merely work reported complete elsewhere.

### Foundation and approved prototype

| Package | Deliverable | Depends on | Completion evidence |
|---|---|---|---|
| **SB-00 — Requirements and execution records** | Import the 17 documents, establish authority/reconciliation notes, package briefs, tracking records, reference manifest, agent instructions, and local reference copying. | — | Hashes and links verified; documents tracked; supporting files ignored and available through the documented copy procedure. |
| **SB-01 — Contracts and integration foundations** | Portable types, package/layer registrations, typed command and host interfaces, versioned fixture references, and separate service CI configuration. | SB-00 | Serialization/schema tests, no-service behavior, dependency boundaries, wasm compatibility, and CI coverage established. |
| **SB-02 — Prototype cost engine** | Checked decimal material, allowance, overhead, production, quantity expansion, and separate setup calculations. | SB-01 | Exact fixture results, missing-versus-zero tests, unsupported-input rejection, and explained outputs. |
| **SB-03 — Central persistence and calculation API** | Source storage, immutable project versions/results, reference resolution, development authentication, concurrency checks, and idempotent writes. | SB-02 | PostgreSQL/API tests, restart persistence, lost-response retry, conflict handling, and immutable historical calculations. |
| **SB-04 — CAD source mapping and measurement** | Source-to-runtime manifest, definition/occurrence associations, selection adapters, exact volume, and incomplete geometry handling. | SB-01 | Repeated/nested occurrences, revision separation, reopen mapping, mixed open/closed geometry, and display-quality-independent measurements. |
| **SB-05 — Immutable review sessions** | Per-document review binding using the existing command-policy mechanism; UI, command, asynchronous adoption, and source-path protection. | SB-01 | Mutation refusals across entry paths; pending-work and tab-switch tests; ordinary CAD sessions remain editable. |
| **SB-06 — Desktop service controller** | Native transport, bounded background jobs, save/calculate/reopen orchestration, completion matching, and explicit retry. | SB-03, SB-04, SB-05 | Late-response, edit-during-save, backend-failure, project-switch, and no-service tests. |
| **SB-07 — Prototype review UI** | Linked review panel, classification confirmation, material/operation inputs, manual volume, annotations, explained costs, and distinct save states. | SB-06 | Complete persisted native workflow, programmatic command coverage, screenshots/recording, and clear prototype labeling. |
| **SB-08 — Prototype acceptance** | Execute P01–P14 against the integrated desktop and real development service. | SB-07 | Per-check results, restart recovery, numeric reconciliation, read-only evidence, and measured responsiveness. |

After SB-01, SB-02, SB-04, and SB-05 can proceed independently. SB-03 follows SB-02 while the CAD packages continue. Desktop integration waits for their contracts and implementations to land.

### Full MVP

| Package | Deliverable | Depends on | Completion evidence |
|---|---|---|---|
| **SB-09 — Shared data and permissions** | Engineer/Administrator enforcement; versioned component/material catalogs, cost centers, configuration, and administrative maintenance. | SB-08 | Permission matrix, immutable referenced versions, explicit prices/units/currency, and no silent project updates. |
| **SB-10 — Projects, deliveries, and part versions** | Project/product schedules, package ownership, retained sources, draft/active/historical identities, customer revision coexistence, and quotation inclusion/exclusion. | SB-08 | Partial-delivery and identity tests; omitted products preserved; distinct revisions remain separate. |
| **SB-11 — Engineering review and flat grouping** | Reviewed component-to-geometry associations, manual measurements/classifications, shared review records, and training-example capture. | SB-09, SB-10 | Preserved hierarchy, engineer-confirmed flat grouping, bulk confirmation, and source-bound manual decisions. |
| **SB-12 — Tube and protection extraction** | Exact geometry analysis for OD/ID, developed length, individual bends/radii, tangent spacing, end lengths, and protection lengths. | SB-08 | Analytic fixtures plus independently referenced customer cases; exact bend counts and ±0.1% measurement checks. |
| **SB-13 — Recognition and catalog suggestions** | Extensible classification pipeline and catalog suggestions with versioned predictions and mandatory confirmation. | SB-09, SB-11, SB-12 | Agreed coverage evaluation; unconfirmed suggestions never supply authoritative costing inputs; no automatic model deployment. |
| **SB-14 — Feasibility rules and exceptions** | Structured sourced rules, applicability, missing/conflicting-limit handling, and justified exceptions. | SB-09, SB-11, SB-12 | Radius/spacing/connector cases; unresolved checks never pass silently; changed supporting inputs trigger review. |
| **SB-15 — Sourcing and manufacturing definitions** | Explicit sourcing, versioned routing templates, editable operations, dedicated setup rates, and attached non-CAD cost lines. | SB-09, SB-10, SB-11 | Shared part versus assembly operation ownership; template isolation; explicit cycle/unit bases. |
| **SB-16 — Shared demand and manufacturing costs** | Calendar-year demand, volume policies, price breaks, manual FX, material consumption/allowance, configured categories, purchased-branch cutoff, and setup deduplication. | SB-15 | Aggregate-before-peak tests, missing forecast handling, purchased-parent exclusions, explicit FX, and reconciled cost rollups. |
| **SB-17 — Tooling investment and recovery** | Unique tooling ownership, lifetime allocation, separate/amortized treatment, recovery quantities, and under/over-recovery. | SB-16 | Shared investment counted once, assembly quantities respected, and no double recovery. |
| **SB-18 — Commercial scenarios** | Manual/suggested base prices, recurring EBIT, annual productivity, named comparisons, and scenario tooling alternatives. | SB-17 | Confirmed pricing examples, blank-price handling, project-owned targets/volume policy, and tooling/setup exclusions from recurring EBIT. |
| **SB-19 — Activation and accepted baselines** | Readiness gates and atomic activation of affected shared-part revisions while retaining prior accepted results. | SB-14, SB-16, SB-17 | All-or-nothing activation; unresolved inputs/findings block activation; negotiated selling price is not required. |
| **SB-20 — Dependency updates and adoption** | Affected-project notifications, explicit master-data adoption, forecast/scope draft recalculation, and stale-scenario handling. | SB-18, SB-19 | Only dependent records flagged; acknowledgement is not adoption; accepted history remains unchanged. |
| **SB-21 — Internal exports** | Detailed Excel and summary PDF using the selected engineering/scenario baseline. | SB-20 | Draft/incomplete/stale labels, missing-input reporting, traceable versions, and reconciled totals. |
| **SB-22 — MVP acceptance and operating handoff** | End-to-end validation of confirmed application scenarios, permissions, recovery, accuracy evidence, and operational instructions. | SB-13, SB-21 | Requirement-to-test coverage, integrated acceptance results, explicit remaining limitations, and no unsupported readiness claims. |

**Machine-rate extension, SB-X01:** preserve a separately gated package for hourly-rate construction and sensitivity. It publishes immutable fully absorbed rates into SB-09; it does not add another project CAPEX charge or recompute rates from quotation volumes. Allocation policy, incomplete workbook inputs, and inconsistent zero-hours/year validation must be resolved before this package becomes ready. It is not added to the approved prototype.

SB-12/SB-13 customer-data acceptance remains gated by independent references and agreed coverage targets. Missing feasibility limits are handled as unresolved data under SB-14, rather than invented defaults.

## 4. Worktree ownership and integration rules

Before assigning a package, the coordinator:

1. Completes its technical brief and locks its interfaces.
2. Confirms dependency commits are present in the starting base.
3. Assigns one owner and reserves its shared-file edits.
4. Records the assignment before dispatching another agent.

Each package owns dedicated subsystem modules and its execution record. Root manifests, layer tables, shared contracts, command registries, host wiring, and CI files require explicit ownership. Agents must not independently redesign these shared interfaces.

Contract changes are proposed in the execution record with affected consumers. The coordinator resolves them before dependent work continues; agents do not silently change wire formats or maintain incompatible local variants.

Each agent starts by reading `AGENTS.md`, the package brief, applicable requirements, contracts, and predecessor handoffs. Each checkpoint records the actual commit, checks, blockers, and next action so another agent can resume without chat history.

Integration requires:

- Scope and interface review.
- Package acceptance checks and required repository gates.
- Real evidence for observable behavior.
- Revalidation against the integrated base.
- A recorded integration commit and verification result.

Public PR creation or publishing is performed only when authorized. PR links supplement Git records; they are not required for local progress tracking.

## 5. Technical invariants and completion gates

Every package inherits the reviewed corrections:

- Source identities are separate from runtime occurrence IDs.
- Business measurements use authoritative geometry paths.
- Missing inputs and incomplete geometry never become zero.
- The existing command-policy hook is composed with per-document binding and asynchronous adoption checks.
- Long analysis/service work remains nonmodal.
- Material overhead, annotations, and geometry-to-part joins are explicit persisted contracts.
- Programmatic calls use typed commands and never open dialogs.
- Backend writes cannot be represented as CAD-undo transactions.
- Separate backend CI covers PostgreSQL tests, formatting, clippy, dependency/license checks, and shared-contract changes.
- Machine depreciation and availability are not charged twice.

The execution ledger reports milestone readiness separately from package progress: **prototype accepted**, **MVP engineering/costing complete**, and **MVP accepted** require their own integrated evidence.

This is the active execution plan. Current assignments and evidence are recorded in [EXECUTION.md](EXECUTION.md). SB-00 establishes this execution system; later software packages require integrated dependencies and dedicated worktree assignments.
