-- Costing settings and immutable baseline snapshots (package SB-16, additive to contract
-- version 2).
--
-- The project's costing settings (representative volume policy, saved manual exchange rates and
-- purchase price tiers) live in the additive `costing` field of the stored project document, so an
-- accepted settings write is an ordinary append-only project record version and migration 0009
-- adds no settings table.
--
-- The resolved baseline itself is immutable: one row per calculation, naming the exact project,
-- engineering and manufacturing record versions it read, the canonical hash and version of its
-- arithmetic and the resolved inputs and explained outputs. A recalculation appends a new row; no
-- code path updates or deletes one. The shared `requests` table keeps the idempotent request
-- receipt for both the settings write and the calculation.

create table baseline_calculations (
    calculation_id uuid primary key,
    project_id uuid not null references project_records(project_id),
    project_version bigint not null check (project_version >= 1),
    engineering_version bigint not null check (engineering_version >= 0),
    manufacturing_version bigint not null check (manufacturing_version >= 0),
    input_hash text not null check (input_hash <> ''),
    calculator_version text not null check (calculator_version <> ''),
    inputs jsonb not null check (jsonb_typeof(inputs) = 'object'),
    outputs jsonb not null check (jsonb_typeof(outputs) = 'object'),
    created_at timestamptz not null default now()
);

-- The per-project history reads newest first; the index keeps that read bounded by the project.
create index baseline_calculations_project_history on baseline_calculations (project_id, created_at desc, calculation_id desc);
