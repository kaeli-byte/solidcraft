# Parity Issues, Defects, and Test Plan

## 1. Verification status

The calculation logic was independently reconstructed from workbook formulas and checked against cached results:

- All eight machine baseline calculations reconcile.
- All 64 loading-sensitivity results reconcile.
- Sensitivity at each machine's current loading equals its base total rate.
- No cached Excel formula errors were found in the supplied state.

A numerical match does **not** mean all inputs are valid. Oven Forming is the clearest example: it produces a plausible cached rate while displaying `CHECK INPUTS`.

## 2. Defects and implementation risks

### M01 — Oven Forming has no financial year

- `Oven Forming!B7` is blank.
- Excel treats it as zero in `B31 = B30 - (B7 - B10)`, producing remaining depreciation years of `2034` instead of a valid value no greater than useful life `10`.
- `B86` displays `CHECK INPUTS`, but `B32`, `B79`, and reports still produce numbers.
- If 2026 is confirmed, derived remaining life would be `2` years. The current annual depreciation would remain one full annual amount because `min(1, remaining) = 1`, but this does not make the blank input acceptable.

**Recommended:** require financial year; do not publish the rate until confirmed.

### M02 — Invalid status does not invalidate outputs

The status cell checks only:

```text
shifts × hours > 24
useful life <= 0
residual value > installed CAPEX
remaining years > useful life
```

It does not gate hourly-rate formulas or summary/comparison outputs. Other invalid depreciation inputs can cause an empty-string depreciation result to propagate into arithmetic errors.

**Recommended:** validation is a precondition for a publishable rate. Invalid inputs return structured errors, not an apparently usable number.

### M03 — Missing values are silently treated as zero

Blank insurance, utilities, power, maintenance, and other numeric cells evaluate as zero in Excel arithmetic. This hides the difference between “not applicable,” “not yet supplied,” and a confirmed zero.

**Recommended:** require explicit numeric API input; use documented import defaults only for genuinely unused optional components.

### M04 — Cost-center identity is unpopulated

`Cost Center` (B6) is blank on every machine sheet. Summary lookups display blank results as zero in the cost-center column.

**Recommended:** assign stable machine IDs and real cost-center codes during migration; never use zero as a cost-center identifier.

### M05 — Sheet names and labels act as identifiers

`SUMMARY`, `Comparison`, and sensitivity use sheet discovery, `INDIRECT`, and descriptive-label `XLOOKUP`. Renaming sheets or labels can break reporting; order and naming affect behavior.

**Recommended:** direct relationships and explicit report projections.

### M06 — Legacy Excel discovery is not portable calculation logic

The file has no VBA, but `SheetList` uses the Excel 4 macro function `GET.WORKBOOK(1)`. Hidden compatibility names `_xleta.LARGE` and `_xlpm.names` point to `#NAME?`; formulas also use `LET`, `TEXTAFTER`, `FILTER`, and `XLOOKUP`.

**Recommended:** ignore these presentation mechanics in Python. They are not required by the rate engine.

### M07 — Residual value is a hardcoded default

Residual value is formula-driven as 20% of installed CAPEX, not an explicit policy parameter. Some sheets reverse the multiplication order but implement the same rule.

**Recommended:** expose residual value or residual-rate policy explicitly. Retain 20% only as an import/default parity choice, not a universal accounting rule.

### M08 — Remaining-life derivation permits invalid future dates

The formula clamps only the lower bound at zero. If financial year precedes acquisition year, remaining life can exceed useful life. The status notices this only when `remaining > useful life`, while calculations continue.

**Recommended:** reject financial year earlier than acquisition year unless a separate pre-acquisition planning policy exists; validate `0 <= remaining <= useful life`.

### M09 — Acquisition-year convention is implicit

Derived remaining life produces a full annual depreciation amount in the acquisition year and no acquisition-month proration. Fractional proration occurs only when remaining life itself is explicitly fractional.

**Recommended:** confirm this annual planning convention. If statutory/accounting depreciation is required, model it separately.

### M10 — Validation coverage is incomplete

Workbook validation covers many numeric ranges but not all identity/year fields and does not enforce integer years. It also allows zero loading/availability, for which rates become blank.

**Recommended:** implement API-level schema and cross-field validation; never rely on client-side validation alone.

### M11 — Guidance contains stale row references

- Power-factor guidance says to use 100% if “row 43” is measured average power; installed running power is actually row 50.
- Loaded-hour-rate guidance says to use “row 72” for running-hour costing; total machine hourly rate is actually row 79. Row 72 is variable cost excluding standby.

**Recommended:** use field names in the backend and user interface, not fragile row references.

### M12 — Scope text and copied notes are stale

`Start here` describes automatic/semi-automatic assembly and several injection-molding tonnages that are not present as sheets. Copied notes such as `was 260`, `was 8`, `was 2`, gas, and tooling wear also appear where they may not apply. Pipe Cutting contains injection-molding comments about tonnage, molds, dryers, and molding cycles.

**Recommended:** do not infer calculation branches from these texts. Clean migrated guidance by process type.

### M13 — Premises/SG&A boundary is unresolved

A comment on Extrusion Line 1 says premises cost is already included in SG&A, while adjacent notes ask whether it is included. Premises cost is nevertheless added to fixed operating cost.

**Recommended:** finance must decide whether premises belongs in the machine rate, downstream SG&A, or another allocation. Add a double-counting warning; do not silently remove it from parity calculations.

### M14 — Compressed-air/electricity boundary is unresolved

Comments on both extrusion sheets say compressed air is accounted in electricity, yet the formula separately adds air consumption × air tariff. The guidance also warns against counting compressor electricity twice.

**Recommended:** confirm whether machine power includes compressor supply and whether air tariff includes generation electricity. Retain the formula but flag current assumptions for review.

### M15 — Cooling-water basis needs confirmation

Comments describe a closed loop, while nonzero consumption and tariff are used on extrusion sheets. Circulation volume and net makeup volume have very different cost meanings.

**Recommended:** record the measurement basis with consumption and tariff; ensure both use the same basis.

### M16 — Oven Forming power is explicitly unconfirmed

A comment asks to double-check energy consumption and expects it to be higher than extrusion. Current Oven Forming installed power is 90 kW versus 150 kW on Extrusion Line 1.

**Recommended:** treat the rate as an illustrative/unapproved assumption until measurement or documentation is supplied.

### M17 — Extrusion staffing assumption needs review

A comment on Extrusion Line 1 refers to checking the average workload of two people, while committed operators per machine is `0.1`.

**Recommended:** confirm whether the value represents shared allocation, attendance, or an unfinished assumption. Fractional staffing is valid mathematically but must have a documented basis.

### M18 — Illustrative inputs lack governance metadata

The workbook itself says initial values are illustrative, not supplier quotes or industry benchmarks. There is no structured source, measurement date, approver, or confidence status per assumption.

**Recommended:** add provenance and approval fields; do not migrate “cached and numeric” as “validated.”

### M19 — “Fixed” cost contains loading-flexed labor

Annual fixed operating cost includes committed shift labor calculated from loaded scheduled hours. It is fixed in the annual cost classification but changes with loading in sensitivity.

**Recommended:** preserve this behavior and expose the committed-labor component separately so users are not surprised.

### M20 — Standby boundary is an assumption, not a universal law

Standby power applies only during loaded downtime; unloaded hours assume equipment is off.

**Recommended:** retain for parity, but use a different scenario/model if equipment remains powered during unloaded scheduled time.

### M21 — No direct integration with manufacturing cost centers

This workbook does not automatically update the manufacturing workbook's cost-center table. Copying the total rate manually can lose version, year, denominator, and depreciation-boundary information.

**Recommended:** integrate through stable cost-center/rate-version IDs and snapshots.

## 3. Business decisions required before production use

1. Approve or correct every imported machine assumption; current values are illustrative.
2. Confirm Oven Forming financial year and power.
3. Confirm residual-value and annual depreciation conventions.
4. Confirm premises allocation versus production SG&A.
5. Confirm compressed-air and cooling-water supply-cost boundaries.
6. Confirm whether committed labor truly flexes with loading; otherwise use dedicated annual labor or scenario staffing.
7. Decide which product-cost mode uses fully absorbed versus cash rates.
8. Ensure machine depreciation and product/project CAPEX amortization are not charged twice.
9. Define whether loading/availability are standards, forecasts, or measured historical values and how often they are revised.

## 4. Workbook baseline golden tests

All machines have scheduled capacity `4000`, loading `0.85`, loaded hours `3400`, availability `0.90`, loaded downtime `340`, and actual running hours `3060` in the supplied state.

Values below are shown rounded for readability. Tests should recompute from full-precision imported inputs and compare with an explicit tolerance, not compare these display strings alone.

| Machine | Installed CAPEX | Annual depreciation | Fixed rate | Variable rate incl. standby | Depreciation rate | Total rate | Annual full cost |
|---|---:|---:|---:|---:|---:|---:|---:|
| Extrusion Line 1 | 3755065.50 | 0.0000 | 50.9494771242 | 150.7433333333 | 0.0000000000 | 201.6928104575 | 617180.0000 |
| Extrusion Line 2 | 4796212.78 | 383697.0224 | 68.5857516340 | 180.4933333333 | 125.3911837908 | 374.4702687582 | 1145879.0224 |
| ABC Winding | 323600.00 | 0.0000 | 41.1283006536 | 57.8500000000 | 0.0000000000 | 98.9783006536 | 302873.6000 |
| Oven Forming* | 119362.83 | 9549.0264 | 22.8627450980 | 81.9883333333 | 3.1205968627 | 107.9716752941 | 330393.3264 |
| Pipe Cutting | 189495.58 | 15159.6464 | 25.2124183007 | 3.9000000000 | 4.9541328105 | 34.0665511111 | 104243.6464 |
| Leakage Testing | 73982.16 | 0.0000 | 41.8352941176 | 6.1515000000 | 0.0000000000 | 47.9867941176 | 146839.5900 |
| Overmolding | 362777.61 | 0.0000 | 25.6352941176 | 33.5640000000 | 0.0000000000 | 59.1992941176 | 181149.8400 |
| Manual operation | 0.00 | 0.0000 | 40.9627450980 | 0.0000000000 | 0.0000000000 | 40.9627450980 | 125346.0000 |

`*` Oven Forming is an arithmetic parity vector only; recommended validation rejects its missing financial year.

### Detailed Extrusion Line 1 vector

```text
scheduled capacity                  = 4000
loaded scheduled hours             = 3400
loaded downtime                    = 340
actual running hours               = 3060
installed CAPEX                    = 3755065.50
residual value                     = 751013.10
remaining depreciation years       = 0
annual depreciation                = 0
annual premises cost               = 121680
loaded committed labor rate        = 36.66
annual committed shift labor       = 12464.40
annual fixed operating cost        = 155905.40
running electricity/hour           = 127.50
annual standby electricity         = 867
standby/running hour               = 0.283333333333…
variable cost excluding standby    = 150.46
variable operating rate            = 150.743333333333…
fixed operating rate               = 50.949477124183…
depreciation rate                  = 0
total machine hourly rate          = 201.692810457516…
cash operating rate                = 201.692810457516…
annual variable operating cost     = 461274.60
annual fully absorbed cost         = 617180
annual cost/loaded scheduled hour  = 181.523529411765…
```

### Loading sensitivity — Extrusion Line 1

| Loading | Expected total CNY/running hour |
|---:|---:|
| 0.20 | 354.040277777778… |
| 0.40 | 254.428472222222… |
| 0.50 | 234.506111111111… |
| 0.60 | 221.224537037037… |
| 0.70 | 211.737698412698… |
| 0.80 | 204.622569444444… |
| 0.85 | 201.692810457516… |
| 0.90 | 199.088580246914… |
| 1.00 | 194.661388888889… |

Also run all eight built-in scenario values against all eight machines, yielding 64 parity comparisons.

## 5. Synthetic full-branch golden test

This case exercises depreciation proration, premises, committed labor during downtime, run-only labor, every utility category, and annual reconciliation.

### Inputs

```text
working days = 100; shifts/day = 2; hours/shift = 8
loading = 0.75; availability = 0.80

machine CAPEX = 100000; installation = 10000; auxiliary = 10000
residual value = 20000; useful life = 10; remaining years = 0.5

annual maintenance = 6000; insurance = 1000
floor space = 100; annual premises rate/m² = 120
other fixed overhead = 2000; dedicated annual labor = 3000
committed operators = 0.5; base pay = 40; burden = 0.25

installed running power = 50; power factor = 0.60
electricity tariff = 0.80; standby power = 5
air = 10 Nm³/run h; air tariff = 0.20
water = 4 m³/run h; water tariff = 0.50
other utilities = 3; wear = 4; consumables = 5
run-only operators = 0.25; base pay = 40; burden = 0.25
other variable cost = 1.50
```

### Expected results

```text
scheduled capacity                  = 1600
loaded scheduled hours             = 1200
loaded downtime                    = 240
actual running hours               = 960
installed CAPEX                    = 120000
annual depreciation                = 5000
annual premises cost               = 12000
loaded committed labor rate        = 50
annual committed shift labor       = 30000
annual fixed operating cost        = 54000
running electricity/hour           = 24
annual standby electricity         = 960
compressed air/hour                = 2
cooling water/hour                 = 2
run-only labor/hour                = 12.50
variable cost excluding standby    = 54
annual variable operating cost     = 52800
annual fully absorbed cost         = 111800
fixed operating rate               = 56.25
variable operating rate            = 55
depreciation rate                  = 5.208333333333…
total machine hourly rate          = 116.458333333333…
cash operating rate                = 111.25
annual cost/loaded scheduled hour  = 93.166666666667…
```

For the same inputs with scenario loading `0.50`:

```text
loaded scheduled hours       = 800
actual running hours         = 640
annual committed labor       = 20000
annual fixed operating cost  = 44000
annual standby electricity   = 640
annual full cost             = 84200
total machine hourly rate    = 131.5625
```

## 6. Edge-case tests

| Case | Expected recommended behavior |
|---|---|
| Loading = 0 | `NO_RUN_HOURS`; all hourly rates `null`; annual scenario totals may be returned |
| Availability = 0 | Same no-run-hours behavior; do not divide by zero |
| Fully depreciated, remaining = 0 | Depreciation and depreciation rate = 0 |
| Remaining = 0.5 | Half of one full annual depreciation amount |
| Remaining > useful life | Hard validation error |
| Financial year < acquisition year | Hard validation error under derived-life policy |
| Useful life = 0 or negative | Hard validation error |
| Residual > installed CAPEX | Hard validation error |
| Residual = installed CAPEX | Valid with warning if desired; depreciation = 0 |
| Shifts × hours > 24 | Hard validation error |
| Missing financial year | Hard validation error, never numeric zero coercion |
| Missing optional unused cost component | Explicit zero only under documented import policy |
| Committed operators = 0 | No committed labor; other fixed costs remain |
| Run-only operators > 0 | Labor added per actual running hour, not loaded hour |
| Standby power = 0 | Standby annual and per-hour cost = 0 |
| Power factor = 1 with measured average kW | Running electricity = measured kW × tariff |
| Fractional operators | Supported without rounding |
| Manual operation with zero CAPEX/utilities | Same engine; rate consists of applicable fixed/labor costs |

For the synthetic case with loading changed to zero, annual full cost is `29000` (annual fixed allocations `24000` plus depreciation `5000`), while all hourly rates are undefined.

## 7. Property and reconciliation tests

For valid inputs with positive running hours:

```text
loaded_downtime + actual_running_hours == loaded_scheduled_hours

total_machine_hourly_rate
    == fixed_operating_rate
     + variable_operating_rate
     + depreciation_rate

cash_operating_rate
    == total_machine_hourly_rate - depreciation_rate

annual_fully_absorbed_cost
    == total_machine_hourly_rate × actual_running_hours

annual_variable_operating_cost
    == variable_operating_rate × actual_running_hours

annual_cost_per_loaded_scheduled_hour × loaded_scheduled_hours
    == annual_fully_absorbed_cost

sensitivity(base.loading).total_rate == base.total_rate
```

Additional properties:

- Increasing a tariff cannot decrease total cost/rate when all other inputs are unchanged.
- Increasing running hours through loading cannot increase total rate for nonnegative inputs.
- Setting a cost component to zero removes exactly that component, with no unrelated change.
- Doubling committed operators doubles committed labor but does not change run-only labor.
- Doubling run-only operators doubles run-only labor but does not change committed labor.
- Depreciation is zero when remaining life is zero, independent of loading.
- Changing language, display name, brand, or model does not change numeric results.

Use a stated tolerance for decimal comparisons, for example relative `1e-12` in Python tests; workbook cached floats may require a looser import-parity tolerance such as relative `1e-10`.

## 8. Regression and acceptance checklist

- [ ] One common engine handles all eight resources.
- [ ] All baseline values reconcile from imported full-precision inputs.
- [ ] All 64 sensitivity values reconcile.
- [ ] Synthetic full-branch values reconcile.
- [ ] Oven Forming cannot be published with missing financial year.
- [ ] Invalid inputs cannot return an apparently approved rate.
- [ ] No blank numeric API field silently becomes zero.
- [ ] Rate versions are immutable after publication/use.
- [ ] Process operations reference stable IDs and store rate snapshots.
- [ ] Fully absorbed and cash rates are not mixed or added together.
- [ ] Machine depreciation is not repeated in product CAPEX amortization.
- [ ] Premises and utilities have documented double-counting decisions.
- [ ] No SG&A, margin, profit, material, or cycle-time logic has leaked into this engine.
