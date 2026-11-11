-- Accepted baselines: the append-only accepted state of one project (package SB-19, activation
-- contract version 1 additive to API version 2).
--
-- One row records one committed activation: the project record version it appended, the exact
-- engineering, manufacturing and feasibility record versions it read, the immutable SB-16 costing
-- snapshot it wrote in the same transaction, the accepted members and their actions, the resulting
-- active revision of every product, and the full accepted shared-definition set. The accepted
-- baseline with the largest sequence is the project's head; `(project_id, sequence)` and
-- `(project_id, project_version)` are unique, so the accepted-baseline head and the project-version
-- head stay single-valued concurrency tokens.
--
-- The table is immutable: no route and no code path updates or deletes a row. The accepted-baseline
-- state deliberately lives here instead of the stored project document, so the project document
-- schema stays version 1 and no later write can rewrite accepted history. The shared `requests`
-- table keeps the idempotent activation receipt; the SB-16 `baseline_calculations` table keeps the
-- pinned snapshot (its columns are unchanged).

create table accepted_baselines (
    accepted_baseline_id uuid primary key,
    project_id uuid not null references project_records(project_id),
    sequence bigint not null check (sequence >= 1),
    request_id text not null check (length(btrim(request_id)) > 0 and length(request_id) <= 128),
    project_version bigint not null check (project_version >= 1),
    engineering_version bigint not null check (engineering_version >= 0),
    manufacturing_version bigint not null check (manufacturing_version >= 0),
    feasibility_version bigint not null check (feasibility_version >= 0),
    costing_calculation_id uuid not null references baseline_calculations(calculation_id),
    costing_input_hash text not null check (costing_input_hash ~ '^[0-9a-f]{64}$'),
    calculator_version text not null check (length(btrim(calculator_version)) > 0),
    costing_state text not null check (costing_state in ('complete', 'incomplete')),
    members jsonb not null check (jsonb_typeof(members) = 'array'),
    active_revisions jsonb not null check (jsonb_typeof(active_revisions) = 'array'),
    shared_definitions jsonb not null check (jsonb_typeof(shared_definitions) = 'array'),
    created_at timestamptz not null default now(),
    unique (project_id, sequence),
    unique (project_id, project_version),
    unique (request_id)
);

-- The accepted-baseline history reads newest first; the index keeps that read bounded by the
-- project. The unique constraints above already index the two concurrency tokens.
create index accepted_baselines_project_history on accepted_baselines (project_id, sequence desc);
