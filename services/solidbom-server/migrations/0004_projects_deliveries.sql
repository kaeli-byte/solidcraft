-- Project, delivery and part-version records (contract version 2, package SB-10).
--
-- The project document is append-only: every accepted write appends one immutable version and
-- advances the project's current version. Delivery packages keep the original source reference
-- recorded in the document; the bytes stay in `sources`. Quotation exclusion/reinstatement and
-- part matches are ordinary document writes, so history is never rewritten.

create table project_records (
    project_id uuid primary key,
    current_version bigint not null default 0 check (current_version >= 0),
    created_at timestamptz not null default now()
);

create table project_record_versions (
    project_id uuid not null references project_records(project_id),
    version bigint not null check (version >= 1),
    document jsonb not null,
    created_at timestamptz not null default now(),
    primary key (project_id, version)
);
