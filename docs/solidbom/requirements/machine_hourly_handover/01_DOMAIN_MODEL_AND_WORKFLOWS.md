# Domain Model and Workflows

## 1. Entity boundaries

### `Machine` or `CostCenter`

Stable master identity; contains no annually changing calculation assumptions.

| Field | Requirement | Notes |
|---|---|---|
| `id` | Required, immutable UUID/internal ID | Authoritative foreign key |
| `code` | Required, unique | Business cost-center code; workbook cells are blank, so assign during migration |
| `name` | Required | Display name, not a lookup key |
| `process_type` | Required enum/string | Extrusion, Winding, Forming, Cutting, Test, Injection, Manual work |
| `process_description` | Optional | More specific process label |
| `brand` | Optional | Descriptive only |
| `model` | Optional | Descriptive only |
| `acquisition_year` | Required for derived remaining life | Workbook `Year`, cell B10 |
| `active` | Required boolean | Controls future selection, not historical reads |

A manual workstation can use the same entity even when CAPEX, power, and utilities are zero.

### `MachineRateVersion`

Immutable calculation input for one machine, financial year, and scenario.

Suggested metadata:

```text
id
machine_id                 -> Machine.id, required
financial_year             integer, required
scenario                   enum: BASE | BUDGET | CUSTOM
version_number             integer, unique per machine/year/scenario
status                     DRAFT | VALIDATED | PUBLISHED | RETIRED
source_reference           optional workbook/document/reference
assumption_notes           optional
created_at / approved_at   audit metadata
```

Suggested calculation fields, using the workbook's B-column row numbers as traceability:

| Backend field | Cell | Unit / behavior |
|---|---:|---|
| `working_days_per_year` | B13 | days/year |
| `shifts_per_day` | B14 | shifts/day |
| `hours_per_shift` | B15 | hours/shift |
| `loading` | B17 | decimal fraction |
| `availability` | B19 | decimal fraction during loaded hours |
| `machine_purchase_capex` | B25 | CNY |
| `installation_capex` | B26 | CNY |
| `dedicated_auxiliary_capex` | B27 | CNY |
| `residual_value` | B29 | CNY; derived as 20% of installed CAPEX by default |
| `useful_life_years` | B30 | years |
| `remaining_depreciation_years` | B31 | years; derived by default, explicit override optional |
| `annual_maintenance` | B35 | CNY/year, fixed portion only |
| `annual_insurance` | B36 | CNY/year |
| `floor_space_m2` | B37 | m² |
| `annual_premises_rate_per_m2` | B38 | CNY/m²/year |
| `other_fixed_machine_overhead` | B40 | CNY/year |
| `dedicated_annual_labor` | B41 | CNY/year, fully burdened |
| `committed_operators` | B42 | people/machine, fractional allowed |
| `committed_labor_base_hourly_pay` | B43 | CNY/person-hour |
| `committed_labor_burden_rate` | B44 | decimal fraction |
| `installed_running_power_kw` | B50 | kW; measured average power may be entered with factor = 1 |
| `average_running_power_factor` | B51 | decimal fraction |
| `electricity_tariff` | B52 | CNY/kWh |
| `standby_power_kw` | B53 | kW during loaded downtime |
| `compressed_air_nm3_per_running_hour` | B54 | Nm³/running hour |
| `compressed_air_tariff` | B55 | CNY/Nm³ |
| `cooling_water_m3_per_running_hour` | B56 | m³/running hour |
| `cooling_water_tariff` | B57 | CNY/m³ |
| `other_utilities_per_running_hour` | B58 | CNY/running hour |
| `wear_parts_per_running_hour` | B59 | CNY/running hour |
| `operating_consumables_per_running_hour` | B60 | CNY/running hour |
| `run_only_operators` | B61 | people/machine, fractional allowed |
| `run_only_labor_base_hourly_pay` | B62 | CNY/person-hour |
| `run_only_labor_burden_rate` | B63 | decimal fraction |
| `other_variable_cost_per_running_hour` | B64 | CNY/running hour |

### Derivation policy

Store canonical inputs, not worksheet formulas:

```text
residual_value =
    explicit residual_value, if supplied
    otherwise 0.20 × installed_capex

remaining_depreciation_years =
    explicit override, if supplied and policy permits
    otherwise max(useful_life_years - (financial_year - acquisition_year), 0)
```

For imported auxiliary CAPEX, maintenance, or premises cells containing arithmetic such as `97256.64 + 700000` or `39 × 12`, import the numeric result as the canonical input. Optionally retain the original expression and detailed cost breakdown as provenance, but do not build an expression evaluator.

### `MachineRateResult`

Derived output; recalculable and never independently editable.

```text
scheduled_capacity_hours
loaded_scheduled_hours
loaded_downtime_hours
actual_running_hours

installed_capex
annual_depreciation
annual_premises_cost
loaded_committed_labor_rate
annual_committed_shift_labor
annual_fixed_operating_cost

running_electricity_per_hour
annual_standby_electricity
standby_electricity_per_running_hour
compressed_air_per_hour
cooling_water_per_hour
run_only_labor_per_hour
variable_cost_excluding_standby
variable_operating_rate

fixed_operating_rate
depreciation_rate
total_machine_hourly_rate
cash_operating_rate
annual_variable_operating_cost
annual_fully_absorbed_cost
annual_cost_per_loaded_scheduled_hour

diagnostics[]
```

Monetary/rate results are `null` when mathematically undefined. Zero is a real numeric result and must remain zero.

## 2. Validation contract

### Hard errors in recommended mode

Reject calculation/publication when any of the following holds:

- `financial_year` is missing or not a valid integer year.
- `acquisition_year` is required for derived life but is missing.
- Any numeric input is non-numeric, NaN, infinite, or negative.
- `working_days_per_year > 366` (or the organization's stricter planning limit).
- `shifts_per_day <= 0` or exceeds the configured maximum; workbook upper bound is 4.
- `hours_per_shift <= 0` or `> 24`.
- `shifts_per_day × hours_per_shift > 24`.
- `loading <= 0` or `> 1` for an hourly-rate calculation.
- `availability <= 0` or `> 1` for an hourly-rate calculation.
- `useful_life_years <= 0`.
- `remaining_depreciation_years < 0` or `> useful_life_years`.
- `residual_value > installed_capex`.
- `average_running_power_factor`, committed burden, or run-only burden is outside `[0, 1]` under workbook parity; use a documented wider limit only if the business permits burdens above 100%.
- A published version has no stable `machine_id` and cost-center `code`.

A zero-running-hours scenario may still be used for annual scenario analysis, but all per-hour outputs must be `null` and status `NO_RUN_HOURS`, not a valid published production rate.

### Business warnings, not automatic arithmetic changes

- CAPEX is positive while residual value equals installed CAPEX, resulting in zero depreciation.
- Installed power is positive but electricity tariff is zero, or vice versa.
- Air/water consumption and tariff have inconsistent zero/nonzero pairs.
- Committed staffing and dedicated annual labor are both nonzero; confirm no duplicate labor.
- Premises cost is nonzero and the cost calculator also applies an SG&A uplift; confirm cost boundaries.
- Air/water tariffs may already include supply electricity also present in machine power.
- Machine power is unverified or based only on nameplate data.
- Financial year is earlier than acquisition year.

### Excel parity versus recommended handling

Excel arithmetic treats blank numeric inputs as zero. The importer should:

1. Distinguish structural required fields from optional cost components.
2. Never infer a missing financial year, acquisition year, capacity, loading, availability, life, or identity field as zero.
3. For genuinely unused optional cost components, map blank to explicit zero only under a documented migration policy and record a warning if confirmation is required.
4. Never use blank-to-zero coercion in the public calculation API.

## 3. Status model

Do not reduce validity to the workbook's single display string. Return structured diagnostics:

```json
{
  "status": "INVALID",
  "diagnostics": [
    {
      "code": "MISSING_FINANCIAL_YEAR",
      "severity": "ERROR",
      "field": "financial_year",
      "message": "Financial year is required to derive remaining depreciation years."
    }
  ]
}
```

Recommended status values:

- `OK`: valid and running hours > 0.
- `NO_RUN_HOURS`: inputs otherwise valid but per-hour rates undefined.
- `INVALID`: at least one hard error; no publishable rate.
- `OK_WITH_WARNINGS`: valid arithmetic plus business-review warnings.

## 4. Core workflows

### Create and publish a rate

1. Select stable machine/cost-center ID.
2. Create a draft for financial year and scenario.
3. Enter or import all assumptions.
4. Derive residual value and remaining life only when not explicitly supplied.
5. Validate strictly.
6. Calculate all components using the pure engine.
7. Review business warnings and provenance.
8. Publish an immutable version.
9. Make the published version available to process operations and project calculations.

### Revise a rate

1. Clone the prior version into a new draft.
2. Change assumptions; never alter a published version used by historical runs.
3. Recalculate, compare against the prior version, validate, and publish.
4. Existing project snapshots remain unchanged; new projects select the new version according to policy.

### Import this workbook

1. Create eight stable machines and assign real cost-center codes; workbook B6 is blank throughout.
2. Map each sheet to one 2026 draft version, except do not assume Oven Forming's missing financial year is valid merely because other sheets use 2026.
3. Flatten arithmetic input expressions to values and optionally preserve their text as source notes.
4. Preserve default residual value and derived remaining-life rules for parity.
5. Flag Oven Forming as invalid until financial year/remaining life is confirmed.
6. Record unresolved comments and utility/premises review items.
7. Recompute all outputs and compare with baseline values in the test plan.

### Run loading sensitivity

1. Load an immutable validated rate version.
2. Accept a list of alternative loading values.
3. For each value, recalculate loaded hours, downtime, committed labor, standby, and denominator; keep availability and all other inputs fixed.
4. Return scenario results without modifying the base version.

## 5. API boundary suggestion

The exact routes are flexible; keep these operations distinct:

```text
POST   /machines
GET    /machines/{machine_id}/rate-versions
POST   /machines/{machine_id}/rate-versions
POST   /rate-versions/{version_id}/calculate
POST   /rate-versions/{version_id}/validate
POST   /rate-versions/{version_id}/publish
POST   /rate-versions/{version_id}/sensitivity
GET    /cost-centers/rates?financial_year=YYYY&scenario=BASE
```

`calculate` and `sensitivity` should be deterministic and side-effect free. Publishing is a separate state transition and must require valid inputs.

## 6. Integration data contract

A process operation should reference:

```text
machine_id or cost_center_id
rate_version_id
rate_snapshot_cny_per_running_hour
rate_currency = CNY
rate_denominator = ACTUAL_RUNNING_HOUR
```

Do not look up rates using process description, sheet name, brand, or model. Do not allow a project calculation to read a mutable “current rate” without recording the selected version and value.
