# Issue #69 scope authorization

The ticket's independent review asked for the issue discussion that authorizes this change's scope
(display attributes only; geometry, hierarchy, history and source replacement stay refused). The
source discussion, in order:

1. `pullfrog[bot]`, issue comment 6838493416, 2026-11-12T23:08:17Z
   (https://github.com/kaeli-byte/solidcraft/issues/69#issuecomment-6838493416), the analysis of
   why the three actions refuse and the plan for the fix:

   > What is actually refusing these actions
   >
   > The alert comes from the reviewed-source command policy in
   > `crates/engine/src/solidbom/policy.rs`. A session bound to an imported STEP is default-deny:
   > `ALLOWED` admits selection, view/inspect, renames and the SolidBOM business commands. The three
   > reported actions resolve to commands that are not on it:
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
   > 1. `crates/engine/src/solidbom/policy.rs`: add explicit `ALLOWED` entries (SB-05 says a prefix
   >    is not proof of safety), grouped with a comment: `browser.visibility`, `material.assign`,
   >    `material.list`, `material.inspect`, `appearance.assign`, ... and the rest of the appearance
   >    family.
   > 2. Regression in `crates/engine/src/solidbom/tests.rs` ...
   > 3. UI regression in `crates/ui-egui/src/solidbom/tests.rs` ...

2. `kaeli-byte`, repository role **OWNER**, issue comment 6838523764, 2026-11-12T23:09:45Z, 88
   seconds after the analysis above
   (https://github.com/kaeli-byte/solidcraft/issues/69#issuecomment-6838523764):

   > @elektrikmusik Analyze, Plan and Implement fixes

   The owner instructed the agent to analyze, plan and implement after that analysis had been
   posted, so the display-attribute plan above is the authorized scope for issue #69.

3. `elektrikmusik`, agent delivery comment 6840372840, 2026-11-13T00:47:23Z
   (https://github.com/kaeli-byte/solidcraft/issues/69#issuecomment-6840372840), restates the
   narrower scope to the owner:

   > [PR #71](...) removes the refusals you hit: show/hide, physical material assignment and the
   > appearance family ... now work on a reviewed source ...
   >
   > It does not remove every restriction. Modelling, reparenting, moves, parameter changes,
   > undo/redo and source replacement still refuse, because the D05 prototype policy and the SB-05
   > acceptance keep the geometry read-only ...
   >
   > If you want geometry editing released as well, say so and I will handle that separately.

No later owner message disputes that scope. The confirmed D05 prototype policy
(`docs/solidbom/requirements/cost_calculator_handover/APP_SPECIFICATION.md`: imported geometry is
read-only; geometry changes arrive through a new STEP revision) and the SB-05 package acceptance
(deny modelling, transforms/reparenting, parameter changes, undo/redo and source replacement) are
the standing product contract, so removing "all" restrictions is not authorized by the issue
discussion; releasing geometry editing needs its own owner decision and ticket.
