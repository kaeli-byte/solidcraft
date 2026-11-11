-- Dependency decisions: the append-only acknowledgement and adoption record of one project
-- (package SB-20, dependency updates and adoption contract version 1 additive to API version 2).
--
-- One row records one explicit engineer decision about one current shared-record dependency: the
-- logical shared record, the version the project resolved at decision time, the version the
-- decision names, whether the engineer only acknowledged the update or adopted it, who decided and
-- when. A decision is never updated or deleted, so the history stays readable after later publishes
-- and decisions; `sequence` is strictly increasing per project and starts at 1, which is the order
-- the decision history reads.
--
-- The adopted app-configuration decision is also the pin the SB-20 resolution refinement uses: the
-- project resolves that exact configuration version in later draft calculations instead of the
-- latest published one. No project document field changes; no existing table changes. The shared
-- `requests` table keeps the idempotent decision receipt, and a failed adoption writes neither a
-- decision row nor a record version (the whole transaction rolls back).

create table dependency_decisions (
    decision_id uuid primary key,
    project_id uuid not null references project_records(project_id) on delete cascade,
    sequence bigint not null,
    kind text not null,
    record_id uuid not null,
    from_version_id uuid,
    to_version_id uuid not null,
    action text not null check (action in ('acknowledged', 'adopted')),
    decided_by text not null,
    decided_at timestamptz not null default now(),
    unique (project_id, sequence)
);

-- The per-project decision history reads newest first; the index keeps that read bounded by the
-- project. The receipt replay and the duplicate-decision check read one project's rows too.
create index dependency_decisions_project on dependency_decisions (project_id, sequence desc);

-- The draft result records the highest adoption sequence it consumed. A stored draft is stale with
-- `newer_shared_data` when the project's current highest *adopted* decision sequence is greater than
-- this value: the ordering follows the append-only decision sequence assigned under the project row
-- lock, never a wall-clock timestamp, so an adoption transaction that started earlier but committed
-- after a calculation still marks that calculation stale. Existing snapshots read as 0, which marks
-- them stale as soon as the project records its first adoption — the truthful conservative answer,
-- because those snapshots never consumed one.
alter table baseline_calculations add column adoption_sequence bigint not null default 0;
