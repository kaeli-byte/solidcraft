# Hourly-Rate Calculation Engine

## 1. Calculation conventions

- Use decimal arithmetic.
- Inputs ending in `_rate` that represent fractions use `0.85`, not `85`.
- Do not round intermediate values.
- The denominator for the production rate is **actual running hours**, not scheduled or loaded hours.
- Annual depreciation is separate from annual fixed operating cost.
- Standby electricity is included in the variable operating rate, although its annual amount depends on loaded downtime.
- Return `null` for undefined per-hour results in the backend; Excel displays an empty string.

The formulas below reproduce the common machine-sheet logic for valid inputs.

## 2. Capacity and running hours

Workbook cells: B13–B22.

```text
scheduled_capacity_hours =
    working_days_per_year
    × shifts_per_day
    × hours_per_shift

loaded_scheduled_hours =
    scheduled_capacity_hours × loading

loaded_downtime_hours =
    loaded_scheduled_hours × (1 - availability)

actual_running_hours =
    loaded_scheduled_hours × availability
```

Equivalent identity:

```text
actual_running_hours =
    working_days_per_year × shifts_per_day × hours_per_shift
    × loading × availability
```

Loading represents demand occupation of scheduled capacity. Availability represents time losses within loaded scheduled hours. They are not interchangeable and must not describe the same loss twice.

## 3. CAPEX and depreciation

Workbook cells: B25–B32.

```text
installed_capex =
    machine_purchase_capex
    + installation_capex
    + dedicated_auxiliary_capex
```

Workbook default residual value:

```text
residual_value = installed_capex × 0.20
```

Workbook default remaining depreciation years:

```text
remaining_depreciation_years =
    max(useful_life_years - (financial_year - acquisition_year), 0)
```

Annual straight-line depreciation for the planning year:

```text
annual_depreciation =
    (installed_capex - residual_value)
    / useful_life_years
    × min(1, remaining_depreciation_years)
```

Subject to workbook guards `useful_life_years > 0` and `residual_value <= installed_capex`; the recommended backend additionally enforces all validation rules in the domain document.

Consequences:

- Remaining life `0` means fully depreciated: annual depreciation is zero.
- Remaining life between `0` and `1` prorates one annual depreciation amount.
- Remaining life `>= 1` produces one full annual depreciation amount.
- The derived formula gives a full annual amount in the acquisition year; it does not prorate by acquisition month.
- The formula does not itself cap remaining life at useful life. Treat `remaining > useful_life` as invalid instead of silently capping it.

## 4. Fixed operating costs, excluding depreciation

Workbook cells: B35–B47.

### Premises

```text
annual_premises_cost =
    floor_space_m2 × annual_premises_rate_per_m2
```

### Committed shift labor

```text
loaded_committed_labor_rate =
    committed_labor_base_hourly_pay
    × (1 + committed_labor_burden_rate)

annual_committed_shift_labor =
    committed_operators
    × loaded_committed_labor_rate
    × loaded_scheduled_hours
```

Committed labor is paid during actual running time and loaded downtime. This is why loaded scheduled hours, not running hours, determine its annual cost.

### Annual fixed operating cost

```text
annual_fixed_operating_cost =
    annual_maintenance
    + annual_insurance
    + annual_premises_cost
    + other_fixed_machine_overhead
    + dedicated_annual_labor
    + annual_committed_shift_labor
```

`dedicated_annual_labor` is already a fully burdened annual amount. Do not apply either hourly burden rate to it.

## 5. Utilities and other variable costs

Workbook cells: B50–B73.

### Per-running-hour components

```text
running_electricity_per_hour =
    installed_running_power_kw
    × average_running_power_factor
    × electricity_tariff

compressed_air_per_hour =
    compressed_air_nm3_per_running_hour
    × compressed_air_tariff

cooling_water_per_hour =
    cooling_water_m3_per_running_hour
    × cooling_water_tariff

run_only_labor_per_hour =
    run_only_operators
    × run_only_labor_base_hourly_pay
    × (1 + run_only_labor_burden_rate)
```

If measured average running kW is entered as installed running power, set the power factor to `1`; otherwise use `measured average kW / installed kW` consistently.

### Standby electricity

```text
annual_standby_electricity =
    standby_power_kw
    × electricity_tariff
    × loaded_downtime_hours
```

For positive running hours:

```text
standby_electricity_per_running_hour =
    annual_standby_electricity / actual_running_hours
```

The workbook assumes equipment is off during unloaded hours. Do not apply standby power to the entire scheduled-capacity downtime outside loaded hours.

### Variable cost excluding standby

```text
variable_cost_excluding_standby =
    running_electricity_per_hour
    + compressed_air_per_hour
    + cooling_water_per_hour
    + other_utilities_per_running_hour
    + wear_parts_per_running_hour
    + operating_consumables_per_running_hour
    + run_only_labor_per_hour
    + other_variable_cost_per_running_hour
```

### Total variable operating rate

For positive running hours:

```text
variable_operating_rate =
    variable_cost_excluding_standby
    + annual_standby_electricity / actual_running_hours
```

## 6. Hourly rates

Workbook cells: B76–B80. All require `actual_running_hours > 0`.

```text
fixed_operating_rate =
    annual_fixed_operating_cost / actual_running_hours

depreciation_rate =
    annual_depreciation / actual_running_hours

total_machine_hourly_rate =
    (annual_fixed_operating_cost
     + annual_depreciation
     + annual_standby_electricity)
    / actual_running_hours
    + variable_cost_excluding_standby

cash_operating_rate =
    (annual_fixed_operating_cost
     + annual_standby_electricity)
    / actual_running_hours
    + variable_cost_excluding_standby
```

Equivalent decomposition:

```text
total_machine_hourly_rate =
    fixed_operating_rate
    + variable_operating_rate
    + depreciation_rate

cash_operating_rate =
    total_machine_hourly_rate - depreciation_rate
```

Use the direct total formula in the engine to minimize accumulated rounding differences; the decomposition is a reconciliation invariant.

## 7. Annual totals

Workbook cells: B82–B84.

```text
annual_variable_operating_cost =
    variable_cost_excluding_standby × actual_running_hours
    + annual_standby_electricity

annual_fully_absorbed_cost =
    annual_fixed_operating_cost
    + annual_depreciation
    + annual_variable_operating_cost
```

For positive running hours:

```text
annual_cost_per_loaded_scheduled_hour =
    annual_fully_absorbed_cost / loaded_scheduled_hours
```

Reconciliation identity, within numerical tolerance:

```text
annual_fully_absorbed_cost =
    total_machine_hourly_rate × actual_running_hours
```

The loaded-hour rate is an alternative reporting denominator. It is not the rate to multiply by actual process running hours.

## 8. Reference pseudocode

```python
from decimal import Decimal
from dataclasses import dataclass

D = Decimal
ZERO = D("0")
ONE = D("1")


@dataclass(frozen=True)
class MachineRateResult:
    # Populate every field listed in 01_DOMAIN_MODEL_AND_WORKFLOWS.md.
    # Numeric fields use Decimal; undefined hourly values use None.
    pass


def calculate_machine_rate(i) -> MachineRateResult:
    # Public entry point must first run strict validation.
    # `i` is an immutable MachineRateVersion input snapshot.

    scheduled = (
        i.working_days_per_year
        * i.shifts_per_day
        * i.hours_per_shift
    )
    loaded = scheduled * i.loading
    downtime = loaded * (ONE - i.availability)
    running = loaded * i.availability

    installed_capex = (
        i.machine_purchase_capex
        + i.installation_capex
        + i.dedicated_auxiliary_capex
    )

    residual = (
        i.residual_value
        if i.residual_value is not None
        else installed_capex * D("0.20")
    )
    remaining = (
        i.remaining_depreciation_years
        if i.remaining_depreciation_years is not None
        else max(
            i.useful_life_years
            - D(i.financial_year - i.acquisition_year),
            ZERO,
        )
    )
    annual_depreciation = (
        (installed_capex - residual)
        / i.useful_life_years
        * min(ONE, remaining)
    )

    annual_premises = (
        i.floor_space_m2 * i.annual_premises_rate_per_m2
    )
    committed_rate = (
        i.committed_labor_base_hourly_pay
        * (ONE + i.committed_labor_burden_rate)
    )
    annual_committed_labor = (
        i.committed_operators * committed_rate * loaded
    )
    annual_fixed = (
        i.annual_maintenance
        + i.annual_insurance
        + annual_premises
        + i.other_fixed_machine_overhead
        + i.dedicated_annual_labor
        + annual_committed_labor
    )

    running_electricity = (
        i.installed_running_power_kw
        * i.average_running_power_factor
        * i.electricity_tariff
    )
    annual_standby = (
        i.standby_power_kw * i.electricity_tariff * downtime
    )
    air = (
        i.compressed_air_nm3_per_running_hour
        * i.compressed_air_tariff
    )
    water = (
        i.cooling_water_m3_per_running_hour
        * i.cooling_water_tariff
    )
    run_only_labor = (
        i.run_only_operators
        * i.run_only_labor_base_hourly_pay
        * (ONE + i.run_only_labor_burden_rate)
    )
    variable_ex_standby = (
        running_electricity
        + air
        + water
        + i.other_utilities_per_running_hour
        + i.wear_parts_per_running_hour
        + i.operating_consumables_per_running_hour
        + run_only_labor
        + i.other_variable_cost_per_running_hour
    )

    annual_variable = variable_ex_standby * running + annual_standby
    annual_full = annual_fixed + annual_depreciation + annual_variable

    if running <= ZERO:
        fixed_rate = None
        variable_rate = None
        depreciation_rate = None
        total_rate = None
        cash_rate = None
        loaded_hour_rate = None
        standby_per_hour = None
    else:
        standby_per_hour = annual_standby / running
        fixed_rate = annual_fixed / running
        variable_rate = variable_ex_standby + standby_per_hour
        depreciation_rate = annual_depreciation / running
        total_rate = (
            (annual_fixed + annual_depreciation + annual_standby) / running
            + variable_ex_standby
        )
        cash_rate = (
            (annual_fixed + annual_standby) / running
            + variable_ex_standby
        )
        loaded_hour_rate = annual_full / loaded

    # Return every intermediate and output listed in MachineRateResult.
    # Attach structured diagnostics; never silently repair invalid inputs.
    ...
```

## 9. What the engine must not do

- Do not apply production SG&A, profit, sales margin, or productivity uplift here.
- Do not divide by cavities or multiply by product quantity here.
- Do not derive cycle time from machine power or CAPEX.
- Do not add material cost or tooling amortization.
- Do not depreciate CAPEX again in the product engine after using the fully absorbed rate.
- Do not use `cash_operating_rate` and `depreciation_rate` together with `total_machine_hourly_rate`; that would duplicate components.
- Do not hardcode current example values as universal defaults.
