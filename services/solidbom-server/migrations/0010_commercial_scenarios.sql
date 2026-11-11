-- Commercial scenarios (package SB-18, additive to contract version 2).
--
-- The project's EBIT target and annual selling-price productivity rate live in the additive
-- `commercial` field of the stored project document, so an accepted settings write is an ordinary
-- append-only project record version and this migration adds no settings table.
--
-- A named commercial scenario is its own append-only version chain beside the project record: every
-- write inserts one immutable version naming its pinned baseline snapshot and project version; no
-- code path updates or deletes one. The shared `requests` table keeps the idempotent request
-- receipt for the create and edit routes; the comparison route stores nothing.

create table commercial_scenarios (
    scenario_id uuid not null,
    project_id uuid not null references project_records(project_id),
    version bigint not null check (version >= 1),
    name text not null check (name <> ''),
    baseline_calculation_id uuid not null references baseline_calculations(calculation_id),
    project_version bigint not null check (project_version >= 1),
    manual_prices jsonb not null check (jsonb_typeof(manual_prices) = 'array'),
    tooling jsonb not null check (jsonb_typeof(tooling) = 'object'),
    productivity_applies_to_tooling boolean not null,
    created_at timestamptz not null default now(),
    primary key (scenario_id, version)
);

-- The current-version read and the per-project list both order one scenario's chain newest first.
create index commercial_scenarios_project_history on commercial_scenarios (project_id, scenario_id, version desc);
-- An update locks the chain and reads its latest version; the same index serves that read.
