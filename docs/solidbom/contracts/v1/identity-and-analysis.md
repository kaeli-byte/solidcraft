# Identity and analysis contract

Version: **1** · Status: **published by SB-01** (not coordinator-locked)
Base: `aadc9b31bb6a4ffb30990818e1a0624c82bf2386` · SB-00 prerequisite: `b5115cda78f629cd82ef86d1f90e67a20f2a7c69`
Publication commit: recorded in `docs/solidbom/execution/SB-01.md`

Portable types: `crates/solidbom-contract/src/types.rs`, `crates/solidbom-contract/src/error.rs`.
Producers: the desktop importer/analyser (`crates/engine/src/solidbom/extract.rs`), the service store.
Consumers: the desktop controller, the service, the calculator, later engineering-review packages.
Acceptance: the `types` and `error` test modules (decimal strings, missing vs zero, units, versions,
non-finite/overflow conversion, manifest referential integrity).

## Rules that apply to every contract value

- Business amounts, rates and measurements are **decimal strings** (`rust_decimal::Decimal` with
  `serde-str`). A geometry `f64` becomes a decimal exactly once through
  `decimal_from_f64(value, field)`, through its shortest round-trip text.
- A missing value is `null` (`None`); an explicit zero is `"0"` (`Some(Decimal::ZERO)`). They are
  never interchangeable.
- Non-finite geometry is `ContractError::NonFinite`; a finite value outside the decimal range is
  `ContractError::Overflow`. Both name the field.
- Unknown JSON fields are tolerated; a newer `schema_version`/contract version is refused.
- Changing a field meaning, a wire name or a unit is a breaking change. Adding an optional field is
  compatible and needs no version bump. Historical snapshots keep the version that wrote them.

## Identities

| Value | Meaning | Stability |
|---|---|---|
| `project_id` | One customer inquiry. | Business identity, UUID. |
| `product_revision_id` | One delivered revision of a product. | Business identity, UUID. |
| `part_version_id` | The reviewed record shared by a definition's occurrences. | Derived from the source hash and definition id (`sha256:part:<definition_id>`). |
| `source_id` | The stored original STEP file. | UUID chosen by the store; the desktop's pre-upload value is replaced by the service's. |
| `definition_id` | One shared part definition in an analysis. | Derived from the source hash and child-index path; stable for one source/extractor. |
| `occurrence_id` | One placement of a definition. | Derived from the source hash and child-index path. |
| `calculation_id` | One immutable calculation snapshot. | Assigned by the service. |
| `annotation_id` | One engineer note. | Assigned by the writer. |

Runtime CAD component/body/face ids and display row indices are **not** in this contract and never
stand in for a source or server identity. A re-import of the same source reaches the same
definition/occurrence ids; a different source or extractor reaches different ones.

## `SourceRef`

A stored original file: `source_id: Uuid`, `sha256: String` (64 lower-case hex), `byte_len: u64`
(> 0), `original_name: String` (non-blank). `validate()` refuses any other shape. The SHA-256 is
the stable source identity; the original file is never modified by review, save or reopen.

## `AnalysisKey`

`source_sha256`, `engine_build`, `extractor_version`. Every analysis and every `Measurement`
carries the key that produced it. Bump `extractor_version` whenever extraction changes and
`engine_build` follows the engine crate version, so a saved measurement is never silently attached
to a different algorithm. `analysis_matches` compares the whole key.

## Definition, occurrence and geometry bindings

- `DefinitionRef`: `definition_id`, `source_key`, optional `customer_part_number` and
  `customer_revision`, and the definition's own `volume_mm3: Measurement`.
- `OccurrenceRef`: `occurrence_id`, `definition_id`, optional `parent_occurrence_id`, the
  child-index `source_path: Vec<u64>` (at most `MAX_SOURCE_PATH_DEPTH = 32`) and a **column-major**
  `transform: [[f64; 4]; 4]` (`transform[column][row]`). A row-major reading would transpose it.
- `GeometryBinding`: `source_path`, `source_body: u32`, `definition_id`, `occurrence_id`. One body
  belongs to exactly one occurrence binding.

`AnalysisManifest { schema_version, key, definitions, occurrences, geometry_bindings, issues }`
validates: the version; the caps (`MAX_DEFINITIONS`, `MAX_OCCURRENCES`, `MAX_GEOMETRY_BINDINGS`,
`MAX_ANALYSIS_ISSUES`); unique ids; every definition/measurement bound to `key`; unique
`(definition_id, source_path)`; finite transforms; parents and bindings resolve; issues are named.

## `Measurement`

`value: Option<Decimal>`, `unit: Unit`, `source: MeasurementSource`, `review: ReviewState`,
`reason: Option<String>` (required when manual), `analysis_key`. Negative values are refused.

| `Unit` wire name | Meaning |
|---|---|
| `mm3` | cubic millimetre |
| `cm3` | cubic centimetre |
| `g_cm3` | gram per cubic centimetre |
| `cny_kg` | CNY per kilogram |
| `s` | second |
| `h` | hour |
| `cny_h` | CNY per hour |
| `cny` | CNY |

`MeasurementSource` is `extracted` or `manual`; `ReviewState` is `pending` or `confirmed`. Missing
geometry is `value: null`, never zero; an engineer's manual value needs a reason. Later packages
add provenance fields additively.

## `Issue`

`code`, `field_path`, optional `definition_id`, `message`; all non-blank. `code` is a stable name
(`missing_volume`, `open_body`, `unresolved_grouping`, `missing_cycle_time`, …), not display text.
A missing value is an issue, never a zero.

## Compatibility

Version 1 is published for SB-02, SB-04 and SB-05. A breaking change needs an affected-consumer
list, a migration/compatibility decision and coordinator approval before dependent work continues.
