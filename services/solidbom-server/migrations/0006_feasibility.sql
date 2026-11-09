-- Feasibility rules and justified exceptions (package SB-14, additive to contract version 2).
--
-- Rule tables are append-only version chains maintained by an Administrator: a version keeps its
-- structured rules, their source references and the publisher. A published version is never
-- edited, so a recorded finding always names the exact rules it used.
--
-- A project's feasibility record is append-only too: every recorded assessment keeps the exact
-- reviewed evidence it consumed and every accepted exception keeps the evidence and finding it was
-- accepted against. Supporting geometry changes are detected by comparing the stored evidence with
-- the current engineering review; no accepted exception is silently dropped or silently kept.

create table feasibility_rule_tables (
    rule_table_id uuid primary key,
    current_version bigint not null default 0 check (current_version >= 0),
    created_at timestamptz not null default now()
);

create table feasibility_rule_versions (
    rule_table_version_id uuid primary key,
    rule_table_id uuid not null references feasibility_rule_tables(rule_table_id),
    version bigint not null check (version >= 1),
    supersedes uuid references feasibility_rule_versions(rule_table_version_id),
    name text not null check (length(btrim(name)) > 0 and length(name) <= 255),
    rules jsonb not null,
    source_reference text check (source_reference is null or length(btrim(source_reference)) > 0),
    created_by text not null check (length(btrim(created_by)) > 0 and length(created_by) <= 255),
    created_at timestamptz not null default now(),
    unique (rule_table_id, version)
);

create table feasibility_records (
    project_id uuid primary key references project_records(project_id),
    current_version bigint not null default 0 check (current_version >= 0),
    created_at timestamptz not null default now()
);

create table feasibility_record_versions (
    project_id uuid not null references feasibility_records(project_id),
    version bigint not null check (version >= 1),
    document jsonb not null,
    created_at timestamptz not null default now(),
    primary key (project_id, version)
);
