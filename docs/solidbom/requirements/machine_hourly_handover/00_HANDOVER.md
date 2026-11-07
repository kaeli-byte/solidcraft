# Machine Hourly-Rate Workbook — Backend Handover

## 1. Purpose and scope

Source analyzed: `NOT RELEASED_Machine_Hourly_CNY.xlsm`.

This workbook calculates a **fully absorbed cost-center rate in CNY per actual running hour**. It is master-data/rate-building logic, not product costing logic. Its output should feed the process operation rates used by the manufacturing cost calculator documented separately.

The workbook covers eight configured resources:

1. Extrusion Line 1
2. Extrusion Line 2
3. ABC Winding
4. Oven Forming
5. Pipe Cutting
6. Leakage Testing
7. Overmolding
8. Manual operation

All eight use the same formulas and row layout. Implement **one parameterized engine**, not eight machine-specific calculators.

### Deliverable map

- [01_DOMAIN_MODEL_AND_WORKFLOWS.md](01_DOMAIN_MODEL_AND_WORKFLOWS.md): entities, fields, versioning, API boundaries, validation, and workflows.
- [02_HOURLY_RATE_ENGINE.md](02_HOURLY_RATE_ENGINE.md): exact formulas and reference pseudocode.
- [03_SENSITIVITY_COMPARISON_AND_INTEGRATION.md](03_SENSITIVITY_COMPARISON_AND_INTEGRATION.md): loading scenarios, comparison outputs, and process-cost integration.
- [04_PARITY_DEFECTS_AND_TEST_PLAN.md](04_PARITY_DEFECTS_AND_TEST_PLAN.md): defects, business decisions, baseline values, golden tests, and invariants.

## 2. Workbook responsibility map

| Sheet | Role | Backend treatment |
|---|---|---|
| `Language` | EN/CN labels and navigation | Presentation only; do not put in engine |
| `Start here` | Business definitions and cost boundary | Convert to validation rules and user guidance |
| `SUMMARY` | Machine inventory and headline outputs | Query/projection over rate versions |
| `Comparison` | Running hours, rate components, annual cost, CAPEX | Report/DTO over the same results |
| `Loading sensitivity` | Recalculates total rate at alternative loading values | Pure scenario function using an immutable rate version |
| Eight machine sheets | Inputs and all core calculations | One common `MachineRateVersion` schema and calculator |

There is **no VBA project** despite the `.xlsm` extension. No macro workflow needs to be reproduced.

## 3. Calculation dependency graph

```text
Machine identity + financial year + acquisition year
                       |
Working days × shifts/day × hours/shift
                       |
               scheduled capacity
                       |
                  × loading
                       |
             loaded scheduled hours
                 /             \
        × availability       × (1 - availability)
             /                       \
    actual running hours         loaded downtime
          |    |                       |
          |    |                       × standby kW × electricity tariff
          |    |                              |
          |    |                       annual standby electricity
          |    |
          |    +---- denominator for fixed, variable, depreciation and total rates
          |
Machine + installation + dedicated auxiliary CAPEX
                       |
              installed CAPEX
                       |
        (installed CAPEX - residual value) / useful life
                       |
          × min(1, remaining depreciation years)
                       |
               annual depreciation

Annual maintenance + insurance + premises + other fixed overhead
+ dedicated annual labor + committed shift labor
                       |
             annual fixed operating cost

Running electricity + air + cooling water + other utilities
+ wear + consumables + run-only labor + other variable costs
                       |
          variable cost excluding standby, CNY/running h

(fixed operating + depreciation + standby) / running hours
+ variable cost excluding standby
                       |
             TOTAL CNY / RUNNING HOUR
```

## 4. Units and cost boundary

### Required units

| Quantity | Unit |
|---|---|
| Currency | CNY, excluding recoverable VAT |
| Capacity and downtime | hours/year |
| Machine rate | CNY/actual running hour |
| Annual cost | CNY/year |
| CAPEX and residual value | CNY |
| Labor pay | CNY/person-hour |
| Staffing | people/machine; fractional values allowed |
| Power | kW |
| Electricity tariff | CNY/kWh |
| Compressed air | Nm³/running hour and CNY/Nm³ |
| Cooling water | m³/running hour and CNY/m³ |
| Other utilities, wear, consumables, other variable | CNY/running hour |
| Floor area and premises rate | m² and CNY/m²/year |
| Loading, availability, power factor, labor burden | decimal fractions, not percentages |

### Included

- Annual fixed maintenance and insurance.
- Allocated premises cost.
- Other fixed machine overhead.
- Dedicated annual labor allocation.
- Committed shift labor, including loaded downtime.
- Running electricity and standby electricity during loaded downtime.
- Compressed air, cooling water, other utilities.
- Run-only labor, wear parts, operating consumables, other variable costs.
- Straight-line depreciation on machine, installation, and dedicated auxiliary CAPEX.

### Excluded unless explicitly entered into an input

- Product resin/material and purchased product parts.
- Scrap material losses.
- Profit and financing.
- Corporate overhead.
- Yield/performance losses beyond the stated loading and availability assumptions.

Do not add another availability, yield, or performance factor when applying the resulting rate unless the costing architecture deliberately defines a separate loss. Avoid counting the same loss twice.

## 5. Recommended backend architecture

```text
Machine / CostCenter (stable identity)
        |
        | 1..n
        v
MachineRateVersion (immutable assumptions for a financial year/scenario)
        |
        | calculated, never user-edited
        v
MachineRateResult (component rates, annual costs, diagnostics)
        |
        | referenced by ID + version
        v
ProcessOperation -> ManufacturingCostEngine
```

Key rules:

1. **Separate identity from assumptions.** Renaming a machine or changing a tariff must not rewrite historical calculations.
2. **One rate version per financial year and planning scenario.** Do not mutate a version already used by a project calculation.
3. **Pure deterministic calculation.** The engine accepts one validated input object and returns all components; no Excel, database, clock, or locale dependency.
4. **Store snapshots with calculation runs.** A product quote must retain the exact machine-rate version and resulting rate used.
5. **Use stable foreign keys.** Never resolve a resource by sheet name, process description, brand, or model.
6. **Use `Decimal`.** No intermediate rounding; round only for display or an explicitly defined posting rule.
7. **Return `null` for undefined rates.** Do not reproduce Excel's empty string as a numeric value.
8. **Calculate sensitivity from the original input snapshot.** It must not temporarily mutate the active machine version.

## 6. Excel parity policy

Support two explicit behaviors where necessary:

- `excel_parity`: reproduce arithmetic and default derivation formulas for valid inputs, including the 20% residual-value default and acquisition-year remaining-life derivation.
- `recommended`: reject invalid or incomplete inputs before calculation, preserve missing-versus-zero semantics, and expose structured diagnostics.

Do not silently reproduce defects that produce a plausible number from invalid data. In particular, Oven Forming currently returns a numerical rate while its own status says `CHECK INPUTS`; the recommended backend must not treat that rate as approved master data.

## 7. Implementation sequence

### Phase 1 — Core rate service

- Implement common schema and strict validation.
- Implement capacity, depreciation, fixed cost, variable cost, hourly rates, and annual totals.
- Import the eight machines as separate versioned resources.
- Pass all baseline and synthetic golden tests.

### Phase 2 — Planning outputs

- Implement loading sensitivity as a pure scenario calculation.
- Implement summary/comparison projections.
- Add approval and publication workflow for rate versions.
- Link published rates to process operations by stable cost-center ID.

### Phase 3 — Governance and analysis

- Add cost-boundary checks and double-counting warnings.
- Add assumption provenance: source, measurement date, approver, and comment.
- Add multi-year rate comparisons and scenario reporting if required.

## 8. Non-goals

- Reproducing workbook navigation, colors, translations, dynamic arrays, `INDIRECT`, or `GET.WORKBOOK` in Python.
- Generating one physical worksheet per machine.
- Treating workbook example assumptions as supplier quotations or validated industry benchmarks.
- Automatically inflating rates to future years; this workbook contains no inflation formula.
- Calculating product cycle time or throughput; those belong to the process/manufacturing engine.
