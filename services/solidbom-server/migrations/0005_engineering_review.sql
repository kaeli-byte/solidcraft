-- Engineering review and flat grouping records (package SB-11, additive to contract version 2).
--
-- The review is separate from the imported geometry and the customer hierarchy: reviewed
-- component associations, the shared engineering metadata of each project part version and the
-- captured confirmed-label metadata are an append-only document. Every accepted write appends one
-- immutable version and advances the project's engineering review version; the original STEP
-- bytes in `sources` are never touched.
--
-- A registered analysis is immutable evidence of the exact source hash, engine build and extractor
-- version the review was made against. Grouping writes validate their body references against it,
-- so a source or extractor change cannot silently reuse an earlier grouping.

create table engineering_records (
    project_id uuid primary key references project_records(project_id),
    current_version bigint not null default 0 check (current_version >= 0),
    created_at timestamptz not null default now()
);

create table engineering_record_versions (
    project_id uuid not null references engineering_records(project_id),
    version bigint not null check (version >= 1),
    document jsonb not null,
    created_at timestamptz not null default now(),
    primary key (project_id, version)
);

create table engineering_analyses (
    project_id uuid not null references project_records(project_id),
    product_revision_id uuid not null,
    source_sha256 text not null check (source_sha256 ~ '^[0-9a-f]{64}$'),
    engine_build text not null check (length(btrim(engine_build)) > 0 and length(engine_build) <= 255),
    extractor_version text not null check (length(btrim(extractor_version)) > 0 and length(extractor_version) <= 255),
    manifest jsonb not null,
    registered_at timestamptz not null default now(),
    primary key (project_id, product_revision_id, source_sha256, engine_build, extractor_version)
);
