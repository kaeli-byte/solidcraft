-- Machine hourly-rate versions (gated package SB-X01, machine hourly-rate contract version 1
-- additive to API version 2).
--
-- One row is one immutable version of one (machine, financial year, scenario) chain. A draft holds
-- the complete input snapshot and no publication; publishing it appends one SB-09 cost-center
-- `rate_versions` row and writes that approved outcome snapshot, calculator version and timestamp
-- onto the same machine rate version. A published row is never edited and a later version replaces
-- it by superseding it, exactly like the existing append-only shared chains.
--
-- `financial_year` is nullable only for an incomplete draft: the recommended mode refuses to
-- calculate without it, so a draft can record the missing value instead of inventing one. The chain
-- key coalesces a missing year to 0, which no valid planning year can be (`1900..=2200`), so the
-- uniqueness rule covers the incomplete draft too.
--
-- The `requests` table keeps the idempotent reply of a write, like every other route; `request_id`
-- here records which create request produced the version. No existing table changes.

create table machine_rate_versions (
    machine_rate_version_id uuid primary key,
    machine_id uuid not null,
    financial_year int,
    scenario text not null check (scenario in ('base', 'budget', 'custom')),
    version bigint not null check (version >= 1),
    supersedes uuid references machine_rate_versions (machine_rate_version_id),
    state text not null check (state in ('draft', 'published')),
    input jsonb not null check (jsonb_typeof(input) = 'object'),
    outcome_snapshot jsonb,
    calculator_version text check (calculator_version is null or length(calculator_version) > 0),
    published_rate_version_id uuid references rate_versions (rate_version_id),
    request_id text check (request_id is null or (length(btrim(request_id)) > 0 and length(request_id) <= 128)),
    created_by text check (created_by is null or length(btrim(created_by)) > 0),
    created_at timestamptz not null default now(),
    published_at timestamptz,
    -- A published version carries its complete publication and a draft carries none of it; the two
    -- states are never mixed by an update.
    check (
        (
            state = 'published'
            and outcome_snapshot is not null
            and calculator_version is not null
            and published_rate_version_id is not null
            and published_at is not null
        )
        or (
            state = 'draft'
            and outcome_snapshot is null
            and calculator_version is null
            and published_rate_version_id is null
            and published_at is null
        )
    )
);

-- One chain is (machine, financial year, scenario); version positions inside a chain start at 1 and
-- are unique, so two concurrent creates cannot append one number.
create unique index machine_rate_versions_chain
    on machine_rate_versions (machine_id, coalesce(financial_year, 0), scenario, version);

-- One machine rate version per published SB-09 cost-center rate version, and one machine rate
-- version per create request id.
create unique index machine_rate_versions_published_rate
    on machine_rate_versions (published_rate_version_id)
    where published_rate_version_id is not null;
create unique index machine_rate_versions_request on machine_rate_versions (request_id) where request_id is not null;
