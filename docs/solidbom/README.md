# SolidBOM implementation workspace

This directory carries the requirements, package assignments and execution evidence for the SolidBOM work in SolidCraft. Start with [EXECUTION.md](EXECUTION.md), then the assigned [package brief](packages/README.md), its execution record and applicable requirements. Only SB-00 is being implemented in this workspace; product packages have no implementation evidence yet.

## Authority and reading order

1. Explicit later owner decisions recorded in [decisions.md](decisions.md).
2. Confirmed behavior in [APP_SPECIFICATION.md](requirements/cost_calculator_handover/APP_SPECIFICATION.md).
3. Approved first-delivery scope and P01–P14 in [DESKTOP_PROTOTYPE_SPEC.md](requirements/cost_calculator_handover/DESKTOP_PROTOTYPE_SPEC.md).
4. The active [implementation plan](implementation-plan.md), [contracts](contracts/README.md), and assigned package brief.
5. Compatible arithmetic in costing handover 00–04 and the separate machine hourly-rate handover.

The imported 17 Markdown documents are an unchanged source snapshot. Their dated statements about missing plans, old source versions or unavailable data describe their original inspection. The active plan supersedes the imported technical implementation plan for execution. It does not promote statements marked Proposal into confirmed requirements. The browser-first extension draft and old probe results are historical evidence.

Use the [requirements map](requirements-map.md) to find owning packages and acceptance criteria. Workbook parity modes, a fixed ten-year horizon, workbook sample defaults, project machine CAPEX, NPV/payback and browser delivery are not silently added to the MVP. Machine-rate construction is the separately gated SB-X01 extension.

## Requirements and local reference data

Markdown requirements are tracked in Git. CAD, CSV, workbook, PDF and analysis JSON references are local-only, retained beside their documents so relative links continue to work. [reference-manifest.json](reference-manifest.json) records all 70 supplied files, storage classes, byte sizes and SHA-256 hashes. Do not modify the source snapshot in place; record decisions and revised technical contracts separately. A deliberate new source snapshot needs a reviewed manifest update.

Run from the repository root:

```sh
python3 scripts/solidbom_workspace.py check
python3 scripts/solidbom_workspace.py check --require-references
python3 scripts/solidbom_workspace.py status
```

The first check works in a fresh clone without owner data. The second also requires and hashes every local reference. A source folder containing both handover directories can restore the references:

```sh
python3 scripts/solidbom_workspace.py hydrate --source '/path/to/solidBOM IDEA'
```

Hydration validates every source hash and all destination conflicts before copying only local references. It never overwrites differing files or imports new files outside the manifest. Python is repository tooling, not an application dependency.

For Conductor, populate the same relative reference paths in its main repository checkout. `.worktreeinclude` selects the gitignored references for new local workspaces and retains `.env*` copying. Tracked documents and configuration follow the starting Git commit; integrate SB-00 into the workspace base before relying on them. Existing workspaces need that commit and a separate reference refresh. Reference copying is not synchronization; verify hashes before any customer-data work. A clone on another machine needs the source package separately. While `.worktreeinclude` is still untracked, the main checkout's copy is only a local seed: delete it before checking out or merging a commit that tracks `.worktreeinclude`, otherwise Git refuses to overwrite the untracked file. The tracked file replaces the seed after that commit lands.

The inventory contains 16 STEP files and 28 CSVs, beyond the older ten-file inventory in BENCHMARK_REFERENCE.md. Their presence does not establish provenance, component grouping, independent reference accuracy or successful CAD import. Do not read GPL application source or use customer files as upstream analytic fixtures. Public-safe synthetic fixtures remain the first acceptance inputs.

The repository's asset gate scans ignored files too. Run the unchanged source gate with local owner references temporarily detached:

```sh
python3 scripts/solidbom_workspace.py source-ci
```

This checks every existing reference against the manifest, refuses tracked/changed/symlinked files, moves the ignored references into a temporary directory under target/, runs `cargo xtask ci`, and restores them even when the gate fails. Do not edit or hydrate references concurrently. If the process is forcibly killed, recover the files using the printed target directory's recovery.json; backups are retained on any restoration conflict. This does not exempt shipped assets or change their attribution requirements. Product checks needing owner data run separately with the references present.

## Worktree handoff

The user creates each Conductor workspace. The coordinator supplies one package ID and a starting commit containing its integrated prerequisites. Use [the agent handoff instructions](packages/README.md); do not claim a package by editing another agent's record. Each agent owns one execution record. The coordinator alone updates EXECUTION.md and package assignments.

Before assignment, package-specific contracts must be locked and ownership reservations resolved. A roadmap brief marked Planned is not permission to invent its missing API or business policy. Each package records its exact next action, checks and commit IDs so another session can continue without chat history.
