# Machine hourly-rate construction and sensitivity contract

Version: **1** of this contract, additive to the published version 2 API · Status: **proposed by
SB-X01** under the owner's 2026-11-11 implementation instruction on issue
[#38](https://github.com/kaeli-byte/solidcraft/issues/38); the locking commit is recorded in
[SB-X01's execution record](../../execution/SB-X01.md).

This contract adds the gated machine hourly-rate extension: stable machine identity, append-only
machine rate versions, one deterministic fully absorbed rate engine, its structured validation, the
non-mutating loading sensitivity and publication into the SB-09 shared cost-center rate chain. The
version 1 and version 2 routes and meanings stay exactly as published. This contract changes no
stored project, engineering, manufacturing, costing or commercial document.

Portable types: `crates/solidbom-contract/src/machine_rates.rs` (re-exported at the crate root).
Producers: the pure rate engine (`crates/solidbom-rates`, calculator version
`solidbom-machine-rate/1`), the service's machine-rate store (`services/solidbom-server/src/machine_rates.rs`)
and the routes below. Consumers: the administrator surface (later increment) and every process
operation that selects a published cost-center rate through the SB-09 contract. The extension does
not add a second rate, a second depreciation charge or a second availability factor anywhere in the
MVP.

Acceptance: the contract crate's `machine_rates` tests, the `solidbom-rates` engine tests (including
the documented parity references) and the service's `tests/machine_rates.rs` domain and route tests.

## Rules that apply to every value

The version 1 rules still apply: money, rates and fractions are decimal strings; a missing value is
`null` and an explicit zero is `"0"`, never interchangeable; unknown JSON fields are tolerated; a
stored schema version this build does not read is refused, never coerced. No value is a percentage:
loading, availability, power factor and burden rates are decimals (`0.85`, not `85`).

Additional rules for this contract:

- The engine is pure and deterministic: one `MachineRateInput` and one `CalculationMode` always
  produce byte-identical `MachineRateOutcome` JSON. It reads no clock, database, locale or file. It
  uses checked decimal arithmetic; an unrepresentable operation becomes an `arithmetic_overflow`
  diagnostic and an invalid outcome, never a panic and never an invented value.
- No intermediate display rounding. `rust_decimal` division is the only rounding, at its last
  representable digit; every documented reference divides exactly or states its tolerance.
- **Missing is not zero.** The recommended mode calculates only from fully supplied authoritative
  inputs. A missing assumption is a `missing_input` error and leaves no result. Excel parity may
  coerce a workbook blank to zero, but only inside its explicit parity mode, and a parity outcome is
  never publishable.
- One machine rate version is immutable input. A revised rate is a new version of the same
  (machine, financial year, scenario) chain. A published version and its result snapshot are never
  edited; later versions supersede it and reference it.
- Loading sensitivity is a pure scenario calculation. It copies the input snapshot, changes only
  `loading`, and never mutates a draft, a published version or the SB-09 reference data.
- A published machine rate is an SB-09 cost-center `RateVersion` with `purpose = production`,
  `rate_unit = cny_h`, `currency = CNY`, `ACTUAL_RUNNING_HOUR` as the denominator and the fully
  absorbed basis stated in its provenance. Undefined rates are absent, never zero. The cash and
  loaded-hour figures are analysis fields; neither is ever published in place of the production
  rate.

## Machine identity

`MachineIdentity` is stable master data; it holds no annually changing assumption.

| Field | Meaning | Validation |
|---|---|---|
| `machine_id` | Stable machine/cost-center id; also the published SB-09 cost-center id. | Non-nil UUID. |
| `code` | Business cost-center code. The supplied workbook leaves it blank on every sheet; publication needs a real code. | Optional, non-empty, at most 64 characters, no control characters. |
| `name` | Display name; never a lookup key. | Non-empty, at most 255 characters. |
| `process_type` | `extrusion`, `winding`, `forming`, `cutting`, `testing`, `injection`, `manual_work`. | Required enum. |
| `process_description`, `brand`, `model` | Descriptive only. | Optional, bounded, no control characters. |
| `acquisition_year` | Acquisition year for a derived remaining life. | Optional planning year between 1900 and 2200; required only to derive the remaining life. |
| `active` | Whether future versions may select the machine. History stays readable. | Required boolean. |

Renaming or re-coding a machine never rewrites a historical rate version: every version embeds the
identity snapshot it was calculated from.

## Rate version input

`MachineRateInput` is the complete, immutable input snapshot of one version: the machine identity, the
financial year, the scenario and the machine-sheet assumptions. Every assumption is an optional
decimal on the wire so a missing value is representable; the recommended mode then refuses to
calculate without the authoritative ones.

| Group | Fields (workbook rows use the handover's `B` column) |
|---|---|
| Planning | `financial_year` (required by the recommended mode), `scenario` = `base`/`budget`/`custom` |
| Capacity | `working_days_per_year` (B13), `shifts_per_day` (B14), `hours_per_shift` (B15), `loading` (B17), `availability` (B19) |
| CAPEX | `machine_purchase_capex` (B25), `installation_capex` (B26), `dedicated_auxiliary_capex` (B27), `residual_value` (B29), `useful_life_years` (B30), `remaining_depreciation_years` (B31, optional override) |
| Fixed | `annual_maintenance` (B35), `annual_insurance` (B36), `floor_space_m2` (B37), `annual_premises_rate_per_m2` (B38), `other_fixed_machine_overhead` (B40), `dedicated_annual_labor` (B41), `committed_operators` (B42), `committed_labor_base_hourly_pay` (B43), `committed_labor_burden_rate` (B44) |
| Variable | `installed_running_power_kw` (B50), `average_running_power_factor` (B51), `electricity_tariff` (B52), `standby_power_kw` (B53), `compressed_air_nm3_per_running_hour` (B54), `compressed_air_tariff` (B55), `cooling_water_m3_per_running_hour` (B56), `cooling_water_tariff` (B57), `other_utilities_per_running_hour` (B58), `wear_parts_per_running_hour` (B59), `operating_consumables_per_running_hour` (B60), `run_only_operators` (B61), `run_only_labor_base_hourly_pay` (B62), `run_only_labor_burden_rate` (B63), `other_variable_cost_per_running_hour` (B64) |
| Provenance | `source_reference`, `assumption_notes` |

The recommended mode requires every numeric field above except `remaining_depreciation_years`, which
is derived when absent. Requirements for a first publication: a stable `machine_id`, a non-empty
`code` and a `financial_year`. `acquisition_year` is required only when the remaining life is
derived. `residual_value` is always explicit in the recommended mode: the 20 % workbook default is a
parity reference, not an approved accounting policy.

Derivation inside the engine:

```text
residual_value        = the explicit residual value (recommended)
                      | installed_capex × 0.20 when absent (parity only)

remaining_years       = the explicit override
                      | max(useful_life_years - (financial_year - acquisition_year), 0)
```

## Calculation

The engine mirrors the handover's reference pseudocode. `computed` values are exact decimals;
per-hour outputs exist only when `actual_running_hours > 0`.

```text
scheduled_capacity_hours = working_days_per_year × shifts_per_day × hours_per_shift
loaded_scheduled_hours   = scheduled_capacity_hours × loading
loaded_downtime_hours    = loaded_scheduled_hours × (1 - availability)
actual_running_hours     = loaded_scheduled_hours × availability

installed_capex          = machine_purchase_capex + installation_capex + dedicated_auxiliary_capex
annual_depreciation      = (installed_capex - residual_value) / useful_life_years × min(1, remaining_years)
annual_premises_cost     = floor_space_m2 × annual_premises_rate_per_m2
loaded_committed_labor_rate = committed_labor_base_hourly_pay × (1 + committed_labor_burden_rate)
annual_committed_shift_labor = committed_operators × loaded_committed_labor_rate × loaded_scheduled_hours
annual_fixed_operating_cost  = annual_maintenance + annual_insurance + annual_premises_cost
                             + other_fixed_machine_overhead + dedicated_annual_labor
                             + annual_committed_shift_labor
running_electricity_per_hour = installed_running_power_kw × average_running_power_factor × electricity_tariff
annual_standby_electricity   = standby_power_kw × electricity_tariff × loaded_downtime_hours
compressed_air_per_hour      = compressed_air_nm3_per_running_hour × compressed_air_tariff
cooling_water_per_hour       = cooling_water_m3_per_running_hour × cooling_water_tariff
run_only_labor_per_hour      = run_only_operators × run_only_labor_base_hourly_pay × (1 + run_only_labor_burden_rate)
variable_cost_excluding_standby = running_electricity_per_hour + compressed_air_per_hour
                             + cooling_water_per_hour + other_utilities_per_running_hour
                             + wear_parts_per_running_hour + operating_consumables_per_running_hour
                             + run_only_labor_per_hour + other_variable_cost_per_running_hour
annual_variable_operating_cost = variable_cost_excluding_standby × actual_running_hours + annual_standby_electricity
annual_fully_absorbed_cost   = annual_fixed_operating_cost + annual_depreciation + annual_variable_operating_cost
```

For `actual_running_hours > 0`:

```text
standby_electricity_per_running_hour = annual_standby_electricity / actual_running_hours
fixed_operating_rate      = annual_fixed_operating_cost / actual_running_hours
depreciation_rate         = annual_depreciation / actual_running_hours
variable_operating_rate   = variable_cost_excluding_standby + standby_electricity_per_running_hour
total_machine_hourly_rate = (annual_fixed_operating_cost + annual_depreciation + annual_standby_electricity)
                            / actual_running_hours + variable_cost_excluding_standby
cash_operating_rate       = total_machine_hourly_rate - depreciation_rate
annual_cost_per_loaded_scheduled_hour = annual_fully_absorbed_cost / loaded_scheduled_hours
```

Reconciliation invariants the engine guarantees:

```text
loaded_downtime_hours + actual_running_hours == loaded_scheduled_hours
total_machine_hourly_rate == fixed_operating_rate + variable_operating_rate + depreciation_rate
cash_operating_rate == total_machine_hourly_rate - depreciation_rate
annual_fully_absorbed_cost == total_machine_hourly_rate × actual_running_hours
annual_variable_operating_cost == variable_operating_rate × actual_running_hours
```

The engine never applies SG&A, profit, margin, material cost, tooling amortization, product CAPEX
amortization, cycle time or a productivity uplift. It does not charge an availability or depreciation
loss twice and does not inflate a rate to a future year.

## Validation and statuses

`MachineRateOutcome.status` is one of:

| Status | Meaning | Result |
|---|---|---|
| `ok` | Valid arithmetic with positive running hours. | Full result. |
| `ok_with_warnings` | Valid arithmetic plus business-review warnings. | Full result. |
| `no_run_hours` | Inputs otherwise valid, but zero running hours make every per-hour rate undefined. | Annual costs are still returned; all seven per-hour fields are `null`. |
| `invalid` | At least one hard error. | No result at all. |

Hard errors in the recommended mode: a missing financial year; a missing acquisition year while the
remaining life is derived; a financial year earlier than the acquisition year; a negative or
out-of-range planning year; a missing authoritative numeric input; a negative numeric input;
`working_days_per_year` above 366 or not positive; `shifts_per_day` not in `(0, 4]`;
`hours_per_shift` not in `(0, 24]`; `shifts_per_day × hours_per_shift > 24`; `loading < 0` or `> 1`;
`availability < 0` or `> 1`; `useful_life_years <= 0`; an explicit remaining life below 0 or above
the useful life; a derived remaining life above the useful life; `residual_value > installed_capex`;
a power factor or labor burden outside `[0, 1]`; arithmetic overflow. Zero loading or zero
availability is a valid scenario: the outcome is `no_run_hours`, never a division by zero.

Business warnings (they change no arithmetic): `residual_value` equal to positive installed CAPEX;
positive power with a zero electricity tariff or the reverse; positive air/water consumption with a
zero tariff or the reverse; committed staffing and dedicated annual labor both non-zero; a non-zero
premises cost (confirm the boundary with production SG&A); a non-zero compressed-air cost while
machine power is also charged (confirm whether the compressor supply is counted twice); a non-zero
cooling-water cost (confirm circulation versus net-makeup basis).

Excel parity (`CalculationMode::ExcelParity`) reproduces the supplied workbook, including its
documented defects: blank numeric cells become zero (`blank_treated_as_zero`), a missing residual
value becomes 20 % of installed CAPEX (`residual_defaulted_to_twenty_percent`), and a missing
financial year becomes year zero for the derived remaining life (`remaining_life_exceeds_useful_life`
is a warning there). The one exception is a non-positive useful life: the workbook divides by zero
and has no number to preserve, so parity returns `invalid` with `invalid_useful_life` rather than
inventing one. Otherwise a parity outcome keeps its numbers so the documented references stay
reproducible. A parity outcome is a reference calculation only: the service calculates drafts only
in the recommended mode and never publishes a parity outcome.

## Rate versions and publication

`MachineRateVersion` is one append-only version of one (machine, financial year, scenario) chain.
`CreateMachineRateVersion { request_id, expected_version, input }` creates the next draft:
`expected_version = 0` starts a new chain, otherwise it must equal the chain's current latest version
and the new version supersedes it. A stale expectation is `409 conflict`; an identical retry returns
the recorded outcome; a changed payload under the same request id is `409 conflict`.

`POST /v1/machine-rate-versions/{id}/publish` with `{ request_id, expected_version }` publishes a
draft, where `expected_version` is the machine rate version the caller believes current:

1. Lock the (machine, financial year, scenario) chain and check the selected version is its newest.
   A draft that a newer version superseded, or an `expected_version` that is not the version being
   published, is `409 conflict`; no shared rate is written. The chain lock is the same one the create
   path takes, so a concurrent create or publication cannot make the check stale, and it is taken
   before the version row lock so the two paths cannot deadlock.
2. Recalculate in the recommended mode. An invalid outcome is `422 unprocessable_entity` with a
   client-safe message naming the blocking diagnostic codes; the draft stays a draft.
3. Require a stable machine identity and a non-empty cost-center `code`; otherwise
   `missing_machine_identity` / `missing_machine_code` is `422` and nothing is written.
4. Lock the machine's cost-center chain, append the next SB-09 cost-center `RateVersion`
   (`rate_per_hour` = `total_machine_hourly_rate`, purpose `production`, unit `cny_h`, currency
   `CNY`, source reference naming the machine rate version and its fully absorbed basis), append its
   `cost_center` reference-change fact, write the approved outcome snapshot and calculator version
   onto the machine rate version and mark it `published`. One transaction.
5. The publication response carries the published machine rate version, its stored outcome snapshot,
   the SB-09 `RateVersion` and the change fact. A published version is never edited; a later version
   replaces it by superseding it and appending a newer SB-09 rate version.

Publication never writes a project CAPEX field, never changes the prototype's 120 CNY/hour fixture
and never inflates a rate. Project snapshots that selected an earlier rate keep the version id and
value they recorded (SB-20's adoption rules apply unchanged).

## Loading sensitivity

`POST /v1/machine-rate-versions/{id}/sensitivity` with `{ loadings: [...] }` changes **only**
`loading`. Scheduled capacity, availability, CAPEX, depreciation, annual fixed costs, staffing, pay,
burdens, utilities and every other per-running-hour input stay fixed. Loadings are within `[0, 1]`,
at most 64, and an empty list is refused.

- The response carries the base outcome and one outcome per loading, newest input order preserved.
- A loading of zero is a valid scenario with `no_run_hours` and undefined per-hour rates.
- Sensitivity at the input's base loading equals the base outcome exactly.
- The request is a pure read: the stored draft or published record, its result snapshot and the SB-09
  reference data are byte-identical before and after.
- The closed form `F / (S × l × a) + c × w / a + V + p × t × (1 - a) / a` is the independent
  arithmetic reference for tests (`F` excludes committed labor). The engine remains the only
  production implementation.

## HTTP routes

All routes need a bearer token and the `administrator` permission set; a missing/unknown token is
`401`, a known identity without the permission is `403`. Bodies are `application/json`; errors are
the version 2 `ApiErrorBody` envelope with the stable codes `unauthorized`, `forbidden`, `not_found`,
`conflict`, `invalid` and `internal`.

| Route | Body / query | Success |
|---|---|---|
| `POST /v1/machine-rate-versions` | `CreateMachineRateVersion` | `201 MachineRateVersionView` |
| `GET /v1/machines` | none; latest version per machine, ordered by machine id | `200 MachineRateVersionList` |
| `GET /v1/machines/{machine_id}/rate-versions` | optional `financial_year`, `scenario`; newest first | `200 MachineRateVersionList` |
| `GET /v1/machine-rate-versions/{machine_rate_version_id}` | none | `200 MachineRateVersionView` |
| `POST /v1/machine-rate-versions/{machine_rate_version_id}/sensitivity` | `MachineRateSensitivityRequest` | `200 MachineRateSensitivity` |
| `POST /v1/machine-rate-versions/{machine_rate_version_id}/publish` | `PublishMachineRateVersion` | `201 MachineRateVersionPublished` |

A missing version is `404`. Reads are side-effect free. A draft's view is calculated fresh with the
current engine; a published version's view answers from its stored approval snapshot so a later
calculator change cannot rewrite an approved result. The list routes are bounded to 500 versions.

Coordinates inside the version chain: `machine_id` from the input snapshot, `financial_year` and
`scenario` from the input snapshot. A chain is (machine, financial year, scenario); two scenarios of
one machine are two chains and two rate versions.

## Parity references (not approved company rates)

The engine tests reproduce the handover's arithmetic references, not company-approved defaults:

- The synthetic full-branch vector: 960 running hours, installed CAPEX 120 000, annual
  depreciation 5 000, annual fully absorbed cost 111 800 and 116.458333333… per running hour; at
  loading 0.50 the rate is 131.5625.
- Eight workbook baselines and the 64 built-in loading-sensitivity cases
  (0.20/0.40/0.50/0.60/0.70/0.80/0.90/1.00 × eight machines) as arithmetic references. The supplied
  handover documents publish the eight aggregate baselines and Extrusion Line 1's components and
  sensitivity table, but not the workbook's per-cell inputs for the other seven machines; those
  fixtures are reconstructions that match every published aggregate exactly and are labelled as
  parity references in the test fixture file.
- Oven Forming's missing financial year produces a parity rate but `missing_financial_year` under the
  recommended mode, so the service never publishes it and blank authoritative inputs never become
  zero.
- Loading sensitivity at the base loading equals the base rate, and inputs are unchanged.

## Compatibility

Additive only: new types, new routes and a new migration. A field's meaning, unit, wire name, route,
status, error code or the envelope semantics is a breaking change. A later contract version may add
optional fields; a stored schema version this build does not read is refused, never coerced.

The engine's calculator version is `solidbom-machine-rate/1`. A published version records it beside
its approval snapshot; a future calculator version may add fields to `MachineRateOutcome` but never
change the meaning of an existing one, and never rewrites a published snapshot.

## Open decisions recorded by this contract

These are owner/business decisions the implementation deliberately does not invent. The engine makes
them visible as diagnostics or keeps them out of publication instead of choosing silently:

1. Approved machine assumptions and cost-center codes: the workbook values are illustrative; none are
   seeded as approved company rates.
2. Premises allocation versus production SG&A, and the compressed-air/electricity and cooling-water
   supply boundaries: parity arithmetic is retained and each is a business warning until confirmed.
3. Residual value and annual depreciation conventions: the recommended mode requires an explicit
   residual value; 20 % remains only the workbook parity default.
4. Oven Forming's financial year and power: the recommended mode refuses a missing financial year; the
   power stays an illustrative parity input until measured.
5. Committed labor flexing, fully absorbed versus cash rate selection, and the loaded-hour reporting
   denominator: expressed as separate fields and rate types, never silently substituted.
6. Whether machine depreciation and product/project CAPEX amortization could be charged twice: this
   extension charges depreciation once and adds no project CAPEX.
