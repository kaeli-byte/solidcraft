# Issue #69 scope discussion

The ticket's independent review asked for the issue discussion that resolves this change's scope
(display attributes only; geometry, hierarchy, history and source replacement stay refused). This
file records the discussion accurately; it is **not** an owner authorization for the narrower scope.

1. `pullfrog[bot]` (automated analysis, not the owner), issue comment 6838493416,
   2026-11-12T23:08:17Z
   (https://github.com/kaeli-byte/solidcraft/issues/69#issuecomment-6838493416):

   > What is actually refusing these actions
   >
   > The alert comes from the reviewed-source command policy in
   > `crates/engine/src/solidbom/policy.rs`. A session bound to an imported STEP is default-deny ...
   > The three reported actions resolve to commands that are not on it:
   >
   > - Show/Hide → `browser.visibility`
   > - Physical material → `material.assign`
   > - Appearance → `appearance.assign` (the dialog also calls `appearance.library` and
   >   `appearance.list`)
   >
   > ...
   >
   > These are display/document attributes, not source geometry or hierarchy. The reviewed mapping
   > is keyed by occurrence ids and child-index paths, so visibility, looks and physical material do
   > not invalidate it. The package brief (`docs/solidbom/packages/SB-05.md`) only requires denying
   > modeling, transforms/reparenting, parameter changes, undo/redo and source replacement.
   > Removing the whole policy is not safe: geometry edits would let the STEP-derived model diverge
   > from the reviewed analysis, and the mapping would stop describing it.
   >
   > ...
   >
   > **Plan**
   >
   > 1. `crates/engine/src/solidbom/policy.rs`: add explicit `ALLOWED` entries ...
   >    `browser.visibility`, `material.assign`, `material.list`, `material.inspect`,
   >    `appearance.assign`, ... and the rest of the appearance family.

2. `kaeli-byte`, repository role **OWNER**, issue comment 6838523764, 2026-11-12T23:09:45Z
   (https://github.com/kaeli-byte/solidcraft/issues/69#issuecomment-6838523764):

   > @elektrikmusik Analyze, Plan and Implement fixes

   This asks the agent to analyze, plan and implement the fixes. It does not by itself resolve the
   scope conflict: the issue's original instruction is still "Remove all read-only limitations"
   (the issue body), which is wider than the display-attribute plan.

3. `elektrikmusik` (the agent), issue comment 6840372840, 2026-11-13T00:47:23Z
   (https://github.com/kaeli-byte/solidcraft/issues/69#issuecomment-6840372840), delivery notice:

   > [PR #71](...) removes the refusals you hit: show/hide, physical material assignment and the
   > appearance family ... now work on a reviewed source ...
   >
   > It does not remove every restriction. Modelling, reparenting, moves, parameter changes,
   > undo/redo and source replacement still refuse, because the D05 prototype policy and the SB-05
   > acceptance keep the geometry read-only ...
   >
   > If you want geometry editing released as well, say so and I will handle that separately.

   This is the agent's own comment asking a question; it is not an owner answer and grants no scope
   decision.

Status: the owner has not replied to the follow-up question, so the narrower display-attribute scope
is **not owner-authorized**. The confirmed D05 prototype policy
(`docs/solidbom/requirements/cost_calculator_handover/APP_SPECIFICATION.md`: imported geometry is
read-only; geometry changes arrive through a new STEP revision) and the SB-05 package acceptance
(deny modelling, transforms/reparenting, parameter changes, undo/redo and source replacement) are
the standing product contract. Removing "all" restrictions therefore needs an explicit owner
decision and its own ticket; PR #71 implements only the display-attribute plan until that decision
is made.
