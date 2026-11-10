-- SB-13: catalog component matching attributes (additive to contract version 2).
--
-- A published catalog component version carries the administrator-declared attributes that
-- catalog matching compares against. The column is optional and empty for every legacy record, so
-- an earlier version stays readable and an old writer keeps working; an empty array means "no
-- declared attributes". The contract validates every entry on write and read; the array check
-- rejects a non-array document before it can be read back.
--
-- Recorded suggestions and engineer-confirmed catalog references are not stored here: they live in
-- the append-only engineering record document of `0005_engineering_review.sql`, so a suggestion can
-- never edit shared reference data and a confirmed reference stays separate from the suggestion.

alter table catalog_component_versions
    add column attributes jsonb not null default '[]'::jsonb;

alter table catalog_component_versions
    add constraint catalog_component_versions_attributes_array_check check (jsonb_typeof(attributes) = 'array');
