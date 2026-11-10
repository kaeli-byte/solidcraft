-- Sourcing, routing and attached-cost records (package SB-15, additive to contract version 2).
--
-- The manufacturing definition of one project is an append-only document: the explicit sourcing
-- choice of each shared project part version, the applied routings with their operations and the
-- attached non-CAD cost lines. Every accepted write appends one immutable version and advances the
-- project's manufacturing version; the imported geometry, the project record and the engineering
-- review are never touched.
--
-- A routing template is a company-scoped append-only chain: one logical template with immutable
-- published versions. Applying a template copies its operations into a project routing, so editing
-- the copy never edits the template.

create table manufacturing_records (
    project_id uuid primary key references project_records(project_id),
    current_version bigint not null default 0 check (current_version >= 0),
    created_at timestamptz not null default now()
);

create table manufacturing_record_versions (
    project_id uuid not null references manufacturing_records(project_id),
    version bigint not null check (version >= 1),
    document jsonb not null check (jsonb_typeof(document) = 'object'),
    created_at timestamptz not null default now(),
    primary key (project_id, version)
);

create table routing_templates (
    template_id uuid primary key,
    current_version bigint not null default 0 check (current_version >= 0),
    created_at timestamptz not null default now()
);

create table routing_template_versions (
    template_id uuid not null references routing_templates(template_id),
    version bigint not null check (version >= 1),
    template_version_id uuid not null unique,
    document jsonb not null check (jsonb_typeof(document) = 'object'),
    created_at timestamptz not null default now(),
    primary key (template_id, version)
);
