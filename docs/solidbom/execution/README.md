# Execution record format

Each package owner edits only its own record. The first JSON block is machine-readable; the rest carries checkpoints and evidence. Keep null for facts that do not exist yet. Dates use ISO 8601. Commit fields contain actual Git commits, not placeholders or branch names.

States: `planned`, `ready`, `in_progress`, `in_review`, `ready_to_integrate`, `integrated`, `verified`, `blocked`.

Ready requires an assignment-ready brief, locked contracts and integrated prerequisite commits. In progress additionally requires an assigned agent, worktree, branch and starting commit. A blocked record preserves its previous state and states the cause, responsible input/decision and resume action.

SB-01 creates the first product contracts: it pins an independently reviewed foundation brief in `brief_review` (`reviewer`, `commit`) instead of requiring a pre-existing product contract. SB-00 needs neither product contract nor foundation brief lock. All downstream packages pin their actual integrated contract versions.

Ready to integrate requires an implementation commit, independent reviewer and reviewed commit, plus recorded acceptance evidence. Integrated requires the integration commit. Verified additionally requires post-integration checks. The coordinator validates ancestry against the destination branch before advancing dependency gates; a recorded SHA alone does not prove it landed.

The coordinator owns EXECUTION.md and transition decisions. Agents propose transitions in their records; they cannot self-certify integration or overall acceptance. Status checks validate record structure and dependency consistency; they do not execute product tests or replace review of their evidence.
