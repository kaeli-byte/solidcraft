-- SB-09: shared company reference data and the Engineer/Administrator permission model.
--
-- Reference records become append-only version chains: a stable logical id plus one immutable row
-- per published version. Existing seeded rows stay valid with no invented values: their logical id
-- is their own version id, they are version 1, and their existing price and rate are kept.

-- Material versions: logical material, chain position, optional price and explicit units.
alter table material_versions
    add column material_id uuid,
    add column version bigint not null default 1,
    add column supersedes uuid,
    add column source_reference text,
    add column density_unit text not null default 'g_cm3',
    add column price_unit text not null default 'cny_kg';

update material_versions set material_id = material_version_id where material_id is null;
alter table material_versions alter column material_id set not null;
-- Missing money is null, never zero; the existing non-negative check still holds for a value.
alter table material_versions alter column price_per_kg drop not null;

alter table material_versions
    add constraint material_versions_version_check check (version >= 1),
    add constraint material_versions_supersedes_fkey foreign key (supersedes) references material_versions (material_version_id),
    add constraint material_versions_name_check check (length(btrim(name)) > 0 and length(name) <= 255),
    add constraint material_versions_density_unit_check check (density_unit = 'g_cm3'),
    add constraint material_versions_price_unit_check check (price_unit = 'cny_kg'),
    add constraint material_versions_source_reference_check
        check (source_reference is null or (length(btrim(source_reference)) > 0 and length(source_reference) <= 1024));

create unique index material_versions_logical_version on material_versions (material_id, version);

-- Rate versions: logical cost center, chain position, optional rate, usage purpose and units.
alter table rate_versions
    add column cost_center_id uuid,
    add column version bigint not null default 1,
    add column supersedes uuid,
    add column source_reference text,
    add column rate_unit text not null default 'cny_h',
    add column purpose text not null default 'production';

update rate_versions set cost_center_id = rate_version_id where cost_center_id is null;
-- The seeded setup fixture is the one existing row that is explicitly a setup cost center.
update rate_versions set purpose = 'setup' where rate_version_id = '00000000-0000-4000-8000-000000000102';
alter table rate_versions alter column cost_center_id set not null;
alter table rate_versions alter column rate_per_hour drop not null;
-- An explicit zero rate is a stored value, distinct from a missing rate.
alter table rate_versions drop constraint rate_versions_rate_per_hour_check;
alter table rate_versions add constraint rate_versions_rate_per_hour_check check (rate_per_hour >= 0);

alter table rate_versions
    add constraint rate_versions_version_check check (version >= 1),
    add constraint rate_versions_supersedes_fkey foreign key (supersedes) references rate_versions (rate_version_id),
    add constraint rate_versions_name_check check (length(btrim(name)) > 0 and length(name) <= 255),
    add constraint rate_versions_purpose_check check (purpose in ('production', 'setup')),
    add constraint rate_versions_rate_unit_check check (rate_unit = 'cny_h'),
    add constraint rate_versions_source_reference_check
        check (source_reference is null or (length(btrim(source_reference)) > 0 and length(source_reference) <= 1024));

create unique index rate_versions_logical_version on rate_versions (cost_center_id, version);

-- Catalog component versions: shared reference specifications with an optional per-piece price.
create table catalog_component_versions (
    component_version_id uuid primary key,
    component_id uuid not null,
    version bigint not null check (version >= 1),
    supersedes uuid references catalog_component_versions (component_version_id),
    name text not null check (length(btrim(name)) > 0 and length(name) <= 255),
    specification text not null check (length(btrim(specification)) > 0 and length(specification) <= 1024),
    price numeric check (price >= 0),
    price_unit text not null default 'cny_piece' check (price_unit = 'cny_piece'),
    currency text not null check (currency = 'CNY'),
    source_reference text
        check (source_reference is null or (length(btrim(source_reference)) > 0 and length(source_reference) <= 1024)),
    created_at timestamptz not null default now(),
    unique (component_id, version)
);

-- App cost configuration versions: explicit category list and currency.
create table app_config_versions (
    config_version_id uuid primary key,
    config_id uuid not null,
    version bigint not null check (version >= 1),
    supersedes uuid references app_config_versions (config_version_id),
    currency text not null check (currency ~ '^[A-Z]{3}$'),
    categories jsonb not null,
    source_reference text
        check (source_reference is null or (length(btrim(source_reference)) > 0 and length(source_reference) <= 1024)),
    created_at timestamptz not null default now(),
    unique (config_id, version)
);

-- Append-only reference-change facts: one row per published version, for later packages (SB-20).
-- The service inserts facts and never updates or deletes them; they adopt nothing into a project.
create table reference_changes (
    change_id uuid primary key,
    kind text not null check (kind in ('material', 'cost_center', 'component', 'app_config')),
    record_id uuid not null,
    version_id uuid not null,
    version bigint not null check (version >= 1),
    previous_version_id uuid,
    summary text not null check (length(btrim(summary)) > 0),
    changed_by text not null check (length(btrim(changed_by)) > 0),
    changed_at timestamptz not null default now()
);

create index reference_changes_newest on reference_changes (changed_at desc, change_id desc);
