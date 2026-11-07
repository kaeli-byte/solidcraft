# Sensitivity, Comparison, and Cost-Calculator Integration

## 1. Loading sensitivity contract

The sensitivity sheet changes **loading only**. The following remain fixed:

- Working days, shifts, hours per shift, and scheduled capacity.
- Availability.
- CAPEX, residual value, useful life, remaining life, and annual depreciation.
- Annual maintenance, insurance, premises, other fixed overhead, and dedicated annual labor.
- Staffing per machine, pay rates, burdens, utilities, and other per-running-hour inputs.

For each scenario loading `l`, the safest implementation is to call the same engine with a copied input object where only `loading = l` changes.

The workbook's built-in scenario values are:

```text
0.20, 0.40, 0.50, 0.60, 0.70, 0.80, 0.90, 1.00
```

Do not limit the API to these values; accept any validated loading in `(0, 1]`.

## 2. Closed-form sensitivity formula

The scenario can also be expressed explicitly. Define:

```text
S = scheduled_capacity_hours
a = availability
c = committed_operators
w = loaded_committed_labor_rate
V = variable_cost_excluding_standby
p = standby_power_kw
t = electricity_tariff

F = annual_maintenance
    + annual_insurance
    + annual_premises_cost
    + other_fixed_machine_overhead
    + dedicated_annual_labor
    + annual_depreciation
```

Then, for positive scenario running hours:

```text
total_rate_at_loading(l) =
    F / (S × l × a)
    + c × w / a
    + V
    + p × t × (1 - a) / a
```

Why the last three terms do not depend on loading:

- Committed labor annual cost grows with loaded hours, and scenario running hours grow by the same factor.
- Standby annual cost also grows with loaded hours, and is divided by scenario running hours.
- `V` is already expressed per running hour.

Only the truly annual fixed allocations and depreciation are diluted across more running hours as loading increases.

### Recommended implementation

Use the full engine as the authoritative path. Keep the closed form for testing and explanation, not as a second independently maintained production implementation. Otherwise future cost components could be added to one path and omitted from the other.

Required reconciliation:

```text
sensitivity(base.loading).total_machine_hourly_rate
    == base_result.total_machine_hourly_rate
```

within the agreed decimal tolerance.

### Expected monotonic behavior

For nonnegative inputs and `F > 0`, total rate decreases as loading increases:

```text
d(total_rate) / d(loading) = -F / (S × a × loading²) < 0
```

If `F = 0`, changing loading does not change the hourly rate under this model.

## 3. Cash-rate sensitivity

The workbook displays sensitivity only for the fully absorbed total rate. If cash sensitivity is added, use the same formula but exclude annual depreciation from `F`:

```text
F_cash = F - annual_depreciation

cash_rate_at_loading(l) =
    F_cash / (S × l × a)
    + c × w / a
    + V
    + p × t × (1 - a) / a
```

This is an optional extension, not a current workbook output.

## 4. Summary and comparison projections

The backend should expose the following per machine/version; no separate calculation is needed.

| Report field | Source result |
|---|---|
| Process, type, brand, model, acquisition year | Machine identity |
| Actual running hours | `actual_running_hours` |
| Installed CAPEX | `installed_capex` |
| Annual depreciation | `annual_depreciation` |
| Committed operators | Input snapshot |
| Fixed operating cost rate | `fixed_operating_rate` |
| Variable operating cost rate | `variable_operating_rate` |
| Depreciation rate | `depreciation_rate` |
| Total machine hourly rate | `total_machine_hourly_rate` |
| Cash operating rate | `cash_operating_rate` |
| Annual fully absorbed cost | `annual_fully_absorbed_cost` |
| Annual cost per loaded scheduled hour | `annual_cost_per_loaded_scheduled_hour` |

Do not sum hourly rates across machines to create a portfolio metric. If an aggregate is required, define a production-weighted or hours-weighted denominator explicitly.

## 5. Excel discovery logic is not backend logic

The workbook discovers machine sheets with `GET.WORKBOOK`, `TEXTAFTER`, `LET`, and `FILTER`, then reads labels through `INDIRECT` and `XLOOKUP`. This has several consequences that should disappear in the backend:

- Sheet order affects report order.
- Sheet names become implicit identifiers.
- Blank lookup results can appear as numeric zero.
- Descriptive labels are lookup keys.
- Modern Excel dynamic-array support is required.

Replace all of this with explicit database queries ordered by stable machine code/name and direct foreign keys.

## 6. Applying the rate to a process operation

The manufacturing workbook calculates operation production cost from cycle seconds and a CNY/hour cost-center rate. With this workbook's fully absorbed rate:

```text
operation_production_cost =
    cycle_seconds_per_piece
    × total_machine_hourly_rate
    / 3600
```

If cavities are handled by the manufacturing engine:

```text
cycle_seconds_per_piece =
    cycle_time_seconds / cavity_count
```

when cavity count is greater than one, under that engine's existing parity rule. Cavity logic does not belong to the machine-rate engine.

For a throughput-based process:

```text
conversion_cost_per_piece =
    total_machine_hourly_rate / actual_pieces_per_running_hour

conversion_cost_per_kg =
    total_machine_hourly_rate / actual_kg_per_running_hour

conversion_cost_per_meter =
    total_machine_hourly_rate / actual_meters_per_running_hour
```

Use actual throughput measured on the same running-time basis as the rate. Do not additionally apply availability to a throughput already measured as output per actual running hour.

## 7. Choose the correct rate for the cost objective

| Rate | Appropriate use |
|---|---|
| `total_machine_hourly_rate` | Fully absorbed standard/product manufacturing cost |
| `cash_operating_rate` | Cash operating view excluding depreciation |
| `fixed_operating_rate` | Cost breakdown only; not a complete operation rate |
| `variable_operating_rate` | Variable/standby operating view; excludes fixed cost and depreciation |
| `annual_cost_per_loaded_scheduled_hour` | Alternative capacity reporting denominator, not normal running-hour product costing |

If the product engine adds CAPEX amortization separately for the same machine, using the fully absorbed rate would double count depreciation. Decide whether an investment is machine-level or product/project-level and charge it once.

## 8. Integration with production SG&A and other uplifts

The manufacturing calculator currently applies production SG&A to total production cost and later calculates sales pricing. Keep those steps downstream; they are not part of this workbook.

However, cost-boundary review is mandatory before combining the models:

- Machine rates include allocated premises cost.
- A workbook comment says premises are already included in SG&A, while another note asks whether they are included.
- The manufacturing calculator applies a production SG&A percentage to production cost.
- Compressed-air comments say air is accounted in electricity, while air consumption × tariff is also added.
- Cooling-water comments describe a closed loop; consumption and tariff must use a consistent circulation or net-makeup basis.

These are not reasons to delete components automatically. They require a documented allocation policy so that machine costing and downstream SG&A/utility charges do not absorb the same cost twice.

## 9. Rate versioning and project snapshots

For every product calculation run, persist:

```text
process_operation_id
machine_id / cost_center_id
machine_rate_version_id
financial_year and scenario
rate_type = FULLY_ABSORBED or CASH
rate_snapshot_cny_per_running_hour
cycle_time_snapshot_seconds
cavity_count_snapshot
calculated_operation_cost
```

This workbook has no inflation-adjusted rate. If the other cost model distinguishes base and future/midlife cost-center rates, create separate explicitly versioned annual rates rather than multiplying by an undocumented factor inside the process engine.

## 10. Capacity interpretation

The model allocates all annual fixed costs to expected running hours at the entered loading and availability. Therefore:

- Low loading increases the hourly rate through under-absorption.
- Low availability increases the hourly rate and also affects committed labor and standby per running hour.
- Unloaded scheduled hours carry no standby electricity in this model.
- Committed labor is flexed with loading in sensitivity; if staffing cannot actually flex, put that annual cost in dedicated annual labor or use a revised staffing scenario.

Do not interpret the resulting rate as a market price, supplier quote, or cash-only marginal cost. It is a planning allocation based on the supplied assumptions.
