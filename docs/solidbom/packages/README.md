# Package assignment and agent handoff

The [package index](../package-index.json) is the dependency graph. Each package has a brief here and a separate [execution record](../execution/README.md). EXECUTION.md is the coordinator's current summary; reconcile it with Git and the records before dispatching work.

## Assignment gate

1. Verify the package is unassigned and its dependencies are integrated, with actual commits recorded. A branch-local test pass does not satisfy this gate.
2. Complete and lock its package-specific interfaces under contracts/. SB-01 establishes the prototype contract; later roadmap briefs require refinement against their integrated predecessors before becoming Ready.
3. Reserve every shared-file edit in EXECUTION.md. Dedicated modules are preferred, but module registrations, manifests, contract exports, migrations, command routing and CI changes still need an owner.
4. Have the user create the Conductor workspace from the exact recorded starting commit. Keep the supplied branch name.
5. Record owner, workspace, branch, base and contract versions before dispatch. One active owner per package; do not assign dependent packages speculatively.

## Copyable agent prompt

```text
Implement package <ID> from docs/solidbom/packages/<ID>.md.
Read AGENTS.md, docs/solidbom/README.md, EXECUTION.md, the pinned contracts,
your execution record and predecessor handoffs before editing.
Starting commit: <full SHA>. Contract versions: <versions>.
Reserved shared-file edits: <explicit paths and purpose>.
Work only on this package in this user-created Conductor workspace.
Keep its execution record current at each checkpoint. Never update EXECUTION.md.
Stop dependent work on an unapproved contract change and record its consumers.
Run acceptance checks and repository gates. Report actual results and evidence.
Do not claim integrated/verified status or publish publicly without authorization.
```

## Checkpoints, review and integration

Record the exact completed action and next runnable action before a handoff or context reset. Include commit IDs, failed/unrun checks and the reason, not just a completion percentage. Use public-safe evidence; private inputs, secrets and customer screenshots remain outside tracked documentation.

An independent reviewer records findings and the reviewed commit. The coordinator integrates only a reviewed package with its required checks satisfied, resolves conflicts without discarding other work, and runs the relevant checks again on the integrated base. Only then record the integration commit and eventually Verified.

Run `cargo xtask ci` before every commit, including documentation commits. New backend packages also need their separate service gates. If a required check cannot run, record the exact failure and leave the gate unsatisfied; do not invent a successful result. Observe the repository's generated-file and test-placement rules.

No PR, remote branch push, public issue or published evidence is created merely by assigning a package. PR links are optional tracking references when those actions are authorized.
