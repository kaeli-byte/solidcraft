# Internal exports contract

Version: **1** (additive to API version 2) · Status: **implemented by SB-21** (owner instruction
2026-11-11: start implementation on issue #36; the lock commit is recorded in
[SB-21's execution record](../../execution/SB-21.md)).

This contract is the version 1 internal-export vocabulary of package SB-21, additive to API version
2. The project, engineering, manufacturing, costing, tooling, commercial, adoption and
accepted-baseline contracts stay exactly as published; this contract reads them and adds one
read-only route, one request/reply vocabulary and the two rendered documents. It introduces no
customer-facing output and no activation prerequisite: an unfinished draft result is exportable.

Portable types: `crates/solidbom-contract/src/exports.rs` (re-exported at the crate root), plus the
`ClientRequest::Export` / `ClientReply::Export` host seam in `host.rs`.
Producers: the service export projection and renderers
(`services/solidbom-server/src/export/`) and the export route below.
Consumers: the native export command and controller (`crates/engine`, `apps/solidcraft`) and SB-22's
acceptance evidence.

Acceptance: the contract crate's `exports` tests, the service's `tests/export.rs` domain tests and
the export route cases in `tests/api.rs`; the package record's real rendered XLSX/PDF evidence.

## Rules that apply to every value

The version 1 and version 2 rules still apply: money, rates and measurements are decimal strings; a
missing value is `null` and an explicit zero is `"0"`, never interchangeable; unknown JSON fields
are tolerated; a stored schema version this build does not read is refused, never coerced; a value
received from another program is validated before it is used.

Additional rules for this contract:

- An export is a **pure read**. It appends no project version, record version, snapshot, accepted
  baseline, decision or request receipt, and it stores no rendered artifact: the service renders
  the document and returns it with the request. Rendering the same selected result twice produces
  the same cost and price facts.
- An export names **one exact saved result**. The request carries either `accepted_baseline_id` plus
  its sequence, or the current draft result's `calculation_id`. There is no "export whatever is
  newest" request: the client echoes the identity it read, so a renamed or moved project can never
  change which result the document describes.
- A **draft export names the project's current draft result**: the newest immutable SB-16 baseline
  calculation snapshot that no accepted baseline pins. A superseded snapshot is refused with a
  conflict (`409`); export the current draft result or an accepted baseline instead. Selected
  identities are preserved: a reader can compare the document with the stored snapshot.
- An export of an **accepted baseline** is accepted history. Its project, engineering,
  manufacturing and feasibility record versions are the ones activation pinned and committed, and
  its stored cost values are the accepted values; nothing in the document is re-resolved from
  current master data. A newer accepted baseline is labelled, not silently substituted.
- The selected snapshot's pinned identifiers are printed: the project, engineering, manufacturing
  and feasibility record versions, the costing calculation id, the calculator version and the
  canonical input hash. A draft result's feasibility record is not part of the costing snapshot; the
  metadata records the version read and states that it was read at export time.
- A draft result carries the SB-20 stale statement: the named reasons (`newer_project_version`,
  `newer_engineering_version`, `newer_manufacturing_version`, `newer_shared_data`) are copied from
  the dependency report of the same project state. A stale result is labelled `outdated_costs` and
  its costs are never presented as current.
- **Freshness labels are stable contract names, not display text**, in a canonical order:
  `draft_result`, `incomplete_result`, `missing_inputs`, `unresolved_findings`, `outdated_costs`,
  `superseded_accepted_baseline`, `stale_scenarios`, `unpinned_feasibility_record`. The renderer owns
  the words it prints. No activation prerequisite, readiness claim or customer-facing label is
  introduced; an incomplete or stale result is exported with its defects listed.
- The documents list what is missing instead of inventing values: missing inputs, unavailable
  amounts and unresolved or failed findings are printed with their owning contract's stable issue
  code. The PDF is a summary: costs, annual prices/EBIT, tooling/setup charges and the unresolved
  issues. The workbook is the detail: metadata, BOM, routings, feasibility, cost details and the
  commercial scenarios. Exact page styling and sheet layout are implementation choices of this
  contract version, recorded below.
- Both renderers are **deterministic pure functions of the resolved model**. They perform no
  business arithmetic, no database access and no wall-clock read. The model is resolved once from
  stored records; the renderers only format, wrap and paginate its values.
- The rendered bytes travel in one JSON reply as base64; `media_type`, `byte_len` and `sha256`
  describe them, so a client can verify what it received. The service's suggested `file_name` is one
  ASCII base name with the format's extension; a client may save under another path.

## Identities and selections

| Value | Meaning | Stability |
|---|---|---|
| `project_id` | The project whose saved result is exported. The request carries it and the route path must match. | Business identity from version 2. |
| `accepted_baseline_id` | One append-only accepted project state (SB-19). The pair with its `sequence` is validated against the stored row. | Service-generated UUID plus a strictly increasing sequence. |
| `calculation_id` | One immutable SB-16 baseline calculation snapshot. For a draft export it must be the project's current draft result. | Service-generated UUID. |
| `project_version`, `engineering_version`, `manufacturing_version`, `feasibility_version` | The exact record versions the exported snapshot pinned; `0` is the empty record. | Append-only record versions. |
| `calculator_version`, `input_hash` | The arithmetic version and canonical hash of the stored snapshot inputs. | Immutable snapshot fields. |
| `scenario_id` | One saved commercial scenario version (SB-18) included in the comparison, ascending order. | Business identity; the compared `version` is saved. |
| `format` | `xlsx` (detailed workbook) or `pdf` (summary document). | Contract name. |

## Request, result and route

`POST /v2/projects/{project_id}/exports` (Engineer permission) takes
`ExportRequest { project_id, baseline, scenarios, format }` and answers `200` with `ExportBundle`.
The request validates: a named project, one named saved result, at most 32 unique scenario ids in
ascending order, and a known format. A body whose `project_id` differs from the path is invalid
(`422`), like the other invalid bodies of this API. An unknown project or accepted baseline is
`404`; a selected draft calculation that is not the project's current draft result is a conflict
(`409`).
`baseline` is `{ "accepted_baseline": { "accepted_baseline_id": …, "sequence": n } }` or
`{ "draft_calculation": { "calculation_id": … } }`. `scenarios` is `[]` when the documents carry no
scenario section.

`ExportBundle` is `{ metadata, file_name, media_type, byte_len, sha256, bytes }`; `bytes` is base64
on the JSON wire. `ExportMetadata` carries the project identity and currency, the export timestamp,
the format, the selected baseline (with its sequence), the record versions (and whether the
feasibility record was pinned), the costing calculation identity, the sorted labels and the stale
statement.

The document layouts of version 1 are:

| Workbook sheet | Content |
|---|---|
| Summary | Project, customer, currency, selected result, record versions, calculation identity, result state, labels and stale reasons, issue count |
| Products | Costed product rows: identity, revision state, quotation scope, schedule, lifetime demand, cost values, annual forecasts |
| BOM | One row per product × part version: part identity and classification, sourcing, quantity in one finished product, lifetime and annual demand, unit cost |
| Routings | Part-root and product-root operations: sequence, cycle, cavities, pinned production rate, setup and pinned setup rate |
| Feasibility | Findings with state, required limit, measurements, applied rules and the recorded exception disposition |
| Costs | Part cost lines, setup lines and categories, annual project manufacturing cost, lifetime totals |
| Tooling | Project investment, recovery and separate charge, per-tool and per-product allocation and annual recovery |
| Scenarios | Summary per scenario plus per-product/per-year prices, revenue, EBIT and tooling contribution |
| Issues | Every listed issue with its source, stable code, scope and stored message |

The summary PDF carries a status band and the metadata block, the cost summary and annual
manufacturing-cost table, the scenario prices and EBIT, the tooling and setup charges, and the
unresolved issues and missing inputs, paginated with repeated headers and a footer naming the
selected result and the draft/incomplete state. The PDF uses the standard PDF fonts (Helvetica,
WinAnsi); characters outside that set are replaced with `?`. The workbook carries the exact Unicode
text, so no customer text is lost from the detailed export. Both documents are internal review
material and never a customer quotation.

## Rendering dependencies and determinism

No new dependency is added by this contract. The workbook is a deterministic Office Open XML
package written by a service-local ZIP container writer (stored entries, fixed timestamp, CRC-32);
the summary document is a deterministic PDF written by a service-local writer (standard fonts,
uncompressed content streams, explicit cross-reference table). Both are pure Rust and part of the
service workspace, which is not built for the web and is not shipped in the desktop program. The
`sha256` of the rendered bytes lets a client check the transport.

## Compatibility policy

Adding a format, a label, a stale reason or a metadata field is compatible. Changing a route, a
format name, a label name, the meaning of an identity, the selected-snapshot rule or the freshness
rules is breaking and needs the affected-consumer list and coordinator approval in `EXECUTION.md`.
`API_VERSION` stays 2 and `SCHEMA_VERSION` stays 1: this contract is additive. Exports are not
stored, so there is no stored-document migration.
