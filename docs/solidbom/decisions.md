# SolidBOM execution decisions

This log records execution decisions and later owner decisions. It does not change imported requirements implicitly. New product decisions require an explicit owner decision and the affected requirement/package references.

| ID | Decision | Authority and consequence |
|---|---|---|
| E01 | Preserve the complete full-MVP roadmap; accept the approved prototype first. | Owner selected full MVP package coverage in this session. Prototype completion does not establish MVP readiness. |
| E02 | Track requirements, package briefs and execution records in Git; keep owner reference data local and copied by Conductor. | Owner selected this storage policy. No LFS or automatic OneDrive dependency. |
| E03 | One product package per user-created Conductor worktree; dependencies mean integrated commits. | Owner-approved package plan. The coordinator does not create or rename Conductor workspaces. |
| E04 | SB-00 implements the execution system in the existing workspace. | Later product work waits for SB-00 integration and dedicated workspace assignment. Documentation drafting/review within SB-00 may be delegated without claiming product packages. |
| E05 | Use current checkout 871014f955a76fd308ce156e68c93e6372906005, SolidCraft 0.40.0-rc.2, as the review baseline. | Source inspection; imported plan/probes describe 0.27.1 and remain historical. Each assignment must record its actual newer base. |
| E06 | Preserve imports byte-for-byte and put reconciliation in active documents. | Avoid rewriting historical workbook evidence or presenting proposals as approved. See requirements-map.md and implementation-plan.md. |
| E07 | Machine-rate construction is SB-X01, separate from prototype/MVP acceptance. | Rate consumption is part of the MVP; rate building still needs allocation and validation decisions. Never add depreciation/availability twice. |
| E08 | SB-00 may hydrate the main checkout's ignored reference files and local copy settings without editing its tracked source. | Implements the approved cross-worktree local-copy behavior. Existing settings and differing files must be preserved; no automatic refresh into existing workspaces. |
| E09 | Run the source CI gate with manifest-verified ignored owner references temporarily detached and restored afterward. | The asset scanner visits ignored CAD files. Keep private owner data out of collected source assets without inventing licenses or weakening the gate. Data-dependent acceptance runs remain separate. |
| E10 | Replace the upstream draft-PR claim rule in `AGENTS.md` with coordinator-assigned SolidBOM packages. | Fork process decision recorded with SB-00. The upstream contributing skill remains general guidance; this fork assigns one package per worktree through the coordinator. |
| E11 | Deliver each package as its own pull request against the fork's `main`, carrying that package's record and evidence. | Owner decision: one PR per package keeps review and integration traceable. These PRs are fork work, not upstream contributions. |
| E12 | Keep the fork's `CI` workflow active but run only its light checks on GitHub-hosted runners; heavy jobs keep Blacksmith labels and skip in the fork. | Owner decision, 2026-11-07: the fork has no Blacksmith runners. Light jobs rehosted to `ubuntu-latest`: select checks, format/repository checks, dependency advisories, clippy, doc tests, reference tests, wasm, RISC-V manual and the aggregate `ci`. Heavy jobs are guarded by `github.repository == 'tangency-labs/solidcraft'` and are covered by local `cargo xtask ci` evidence. |
