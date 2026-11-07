# Domain Model and Workflows

## 1. Modeling principles

1. Use database IDs for relationships. Excel lookup keys are presentation identifiers, not reliable foreign keys.
2. Treat product cost sheets as **calculated views**, not entities that must be created before recalculation.
3. Separate authoritative inputs from derived values. Persist derived values only as calculation-run outputs.
4. Version master data or reference immutable versions in each calculation run.
5. Do not use customer part number as the primary product key. The workbook's lookups assume uniqueness but do not enforce it.

## 2. Core entities

### 2.1 `Project`

Suggested fields:

| Field | Workbook source | Rule |
|---|---|---|
| `id` | None | UUID/primary key |
| `inquiry_number` | `ASSEMBLIES!I5` | Required; unique per business scope |
| `inquiry_type` | `ASSEMBLIES!I6` | `PROTOTYPE`, `SERIAL`, or `ENGINEERING CHANGE` |
| `customer` | `ASSEMBLIES!E5` | Required |
| `name` | `ASSEMBLIES!E6` | Required |
| `product_family` | `ASSEMBLIES!I9` | Optional classification |
| `sop_year` | `ASSEMBLIES!E9` | Start of production; integer |
| `eop_year` | `ASSEMBLIES!E10` | End of production; normally `>= sop_year` |
| `sales_owner`, `engineering_owner`, `purchasing_owner`, `process_owner`, `quality_owner`, `finance_owner` | `C5:C10` area | Optional ownership metadata |
| `material_overhead_rate` | `U14`, exposed as `CC-2026!T44` | Current workbook: `0.05` |
| `production_sga_rate` | `Y14`, exposed as `T45` | Current workbook: `0.89`; applied only to production cost |
| `sales_margin_rate` | `AA14`, exposed as `T46` | Current workbook: `0.20`; denominator-based pricing margin |
| `productivity_rate` | `AB14`, exposed as `T47` | Current workbook: `0.12`; uplifts suggested price |
| `base_cost_year` | `CC-2026!H3` | Current workbook: `2026` |
| `working_days` | `CC-2026!H4` | Current workbook: `248` |
| `capacity_oee` | `CC-2026!G3` | Current workbook: `0.90`; used in cost-center/BOM capacity formulas |
| `labor_inflation_rate` | `CC-2026!J4` | Current: `0.04` |
| `manufacturing_inflation_rate` | `K4` | Current: `0.01` |
| `depreciation_inflation_rate` | `L4` | Current: `0.01` |
| `updated_at` | `ASSEMBLIES!I10` | Set on relevant project mutations; workbook VBA updates it after any sheet change |

Workbook filename logic, if export requires it:

```text
"RFQ" + inquiry_number + "_" + product_family + "_" + inquiry_type + "_"
+ customer + "_" + project_name + "_" + format(updated_at, "yyyymmdd")
```

### 2.2 `ProductAssembly`

One record per product involved in the project; corresponds to a row in `ASSY`.

| Field | Source/rule |
|---|---|
| `id`, `project_id` | Backend relationship |
| `racn_part_number` | `ASSY[RACN P/N]`; required and unique within project |
| `customer_part_number` | `ASSY[Customer P/N]`; store as text to preserve leading zeros |
| `quote_status` | `ASSY[To be quoted]`: `YES` or `NO` |
| `description`, `size`, `special` | Product attributes |
| `yearly_volumes` | Ten non-negative integers/Decimals for `Year 1`…`Year 10`; map `Year 1` to SOP unless business says otherwise |
| `actual_sales_price` | Prefer explicit project quote input or a versioned benchmark link; see pricing document |
| `benchmark_references` | Optional initial cost, target, VOSS, CHINAUST values with source/version |

Recommended quote behavior: only `YES` products enter quotation outputs and project quoted totals. The VBA bulk generator creates sheets for every status other than exact `No`, including blanks; this should **not** be copied as an implicit quote rule.

### 2.3 `Component`

Corresponds to `COMPONENTS` columns.

| Group | Fields |
|---|---|
| Identity | `id`, `code`, `part_number`, `designation_en` |
| Costing classification | `origin`, `type`, `uom` |
| Manufacturing | `tubes_to_assemble`, `volume_cm3`, `length_mm`, `od_mm`, `id_mm` |
| Sourcing | `supplier`, `supplier_part_number`, `material_part_number`, `unit_cost_cny_per_uom` |
| Audit | `remarks`, `volume_validity`, `last_offer`, `comment`, effective dates/version |

Observed `Origin` values are `P` (purchased) and formula-supported `I` (in-house). Product process rows use `M`; blank manual BOM rows receive `[Manual input]`. Backend enum:

```text
P = purchased component
I = in-house manufactured component
M = additional manufacturing process
MANUAL = manual cost line, only if this feature is retained
```

Observed component categories include `CT`, `QC`, `FIT`, `SL`, `CLIP`, and `pack`; formulas additionally reference `ST` and `SHT`. Make category a governed reference table rather than hardcoding display names. The following categories have special arithmetic:

- `ST` and `CT`: annular tube volume.
- `QC`: component volume and tubes-to-assemble logic.
- `SHT`: protection cycle-time driver.
- `pack`/other purchased categories: no special volume formula.

`unit_cost_cny_per_uom = 0` is not equivalent to missing. The workbook treats zero as no usable purchase price; recommended behavior is to distinguish `None` from a true zero-price component.

### 2.4 `Material` and `ExchangeRate`

`Material` corresponds to `GRANULATES`/`MATERIALS`:

```text
material_part_number, commercial_name, density_g_cm3, iso_code,
unit_cost, currency, last_updated_cost_date, supplier,
calculation_cost_cny_kg, effective_version
```

Workbook formula:

```text
calculation_cost = unit_cost / VLOOKUP(currency, EX-RATES!A3:X26, 10, FALSE)
```

Column 10 is CNY. In recommended mode, store an explicit `cny_per_unit_currency` rate and use:

```text
calculation_cost_cny_kg = unit_cost * cny_per_unit_currency
```

The existing division is correct only for a rate defined in the inverse direction; the visible table appears to define CNY per currency. See defect `D07`.

### 2.5 `CostCenter`

Corresponds to `CC_YEAR`:

| Field | Formula/source |
|---|---|
| `area`, `code`, `description` | Direct input |
| `machine_investment` | `Invest 1 machine`; informational in core product costing |
| `shifts_per_day`, `hours_per_shift` | Direct input |
| `annual_capacity_hours` | `working_days × shifts_per_day × hours_per_shift × capacity_oee` |
| `base_labor_rate`, `base_manufacturing_rate`, `base_depreciation_rate` | `Labor2`, `Mfg3`, `Depr.4` |
| `base_hourly_rate` | Sum of the three base rates; `CC_YEAR[CC]` |
| `inflation_years` | `ROUNDUP((SOP + EOP)/2, 0) - base_cost_year`, except workbook hardcodes zero when SOP = 2016 |
| `midlife_hourly_rate` | Sum of each base rate compounded by its own inflation rate for `inflation_years`; `CC_YEAR[CC+]` |

Exact midlife components:

```text
labor_mid = base_labor × (1 + labor_inflation) ^ inflation_years
mfg_mid = base_manufacturing × (1 + manufacturing_inflation) ^ inflation_years
depr_mid = base_depreciation × (1 + depreciation_inflation) ^ inflation_years
midlife_hourly_rate = labor_mid + mfg_mid + depr_mid
```

**Core product sheets use `base_hourly_rate` (`CC`), not `midlife_hourly_rate` (`CC+`).** Keep this as an explicit cost-rate policy option rather than accidentally switching rates.

### 2.6 `OperationCycle`

Corresponds to `CYCLES`: `operation`, `category`, `uom`, `time_seconds`.

Current standards:

| Operation | Driver category | Workbook time |
|---|---|---:|
| Cutting | ST | 3.6 seconds per driver unit |
| Protection | SHT | 15 seconds per driver unit |
| Assembly | QC | 10 seconds per driver unit |
| Testing | ST | Blank; not implemented in `PROCESS` formula |
| Packaging | ST | 5 seconds per ST line count |

Require a positive configured time for every operation used in a process line. Do not silently cost an unconfigured operation as zero.

## 3. BOM model and derivation

### 3.1 `BomLine` inputs

```text
id, project_id, product_id, sequence, component_id,
quantity, length_mm, thermoforming, number_bends,
cost_center_id, cycle_time_seconds,
tool_description, cavity_count, tooling_delivery_weeks,
tooling_count, tooling_unit_cost_cny, tooling_comments,
mold_lifetime_supplier_shots
```

The workbook also carries product/customer IDs, descriptions, UOM, type, volume, and forecasts, but these should be joins/derived values rather than duplicated editable fields.

### 3.2 Derived values

| Backend value | Excel rule |
|---|---|
| `item_number` | Sequence within product, starting at 1. Excel resets whenever the previous row's product changes, including blank separator rows; backend must not reset on separators |
| `display_part_number` | `racn_part_number + "-" + item_number` |
| `business_case` | Product quote status |
| `description`, `category`, `uom`, `component_volume` | Direct component attributes |
| `forecast_year_1` | `product.year_1_volume × quantity` |
| `process_quantity` | `component.tubes_to_assemble × quantity` |
| `expected_capacity_pcs` | `(working_days × 24 × 3600 × capacity_oee / cycle_time_seconds) × cavity_count` |
| `lifetime_expected_shots` | `expected_capacity_pcs / cavity_count` |

Capacity formulas are in `BOM[Expected capacity (pcs)]` and `BOM[Lifeime expected (shots)]`. Guard missing/zero cycle time and cavity count. The capacity calculation assumes 24 hours/day and does not use the selected cost center's shifts.

### 3.3 Identifier invariant

`display_part_number` may be recreated for presentation, but `BomLine.id` must be immutable. Product cost calculations must resolve `BomLine.id` directly. This prevents the current `ENG0002` duplicate-ID miscosting described in defect `D01`.

## 4. PROCESS model and derivation

### 4.1 `ProcessLine` inputs

```text
id, project_id, product_id, sequence, operation,
cost_center_id, cycle_time_seconds_override (optional),
tool_description, cavity_count, tooling_delivery_weeks,
tooling_count, tooling_unit_cost_cny
```

Suggested display ID: `racn_part_number + "-P-" + sequence`.

### 4.2 Automatic cycle-time rules

The workbook computes process cycle time from the product's whole BOM when no manual override concept exists:

| `operation` | Exact driver |
|---|---|
| Cutting | `sum(BOM.quantity where product = current and category = "ST") × cycle_standard("Cutting")` |
| Protection | `sum(BOM.quantity where product = current and category = "SHT") × cycle_standard("Protection")` |
| Assembly | `sum(BOM.process_quantity where product = current and category = "QC") × cycle_standard("Assembly")` |
| Packaging | `count(BOM lines where product = current and category = "ST") × cycle_standard("Packaging")` |

Notes:

- Packaging counts ST **lines**, not quantities.
- Assembly uses `process_quantity = tubes_to_assemble × quantity`, not plain quantity.
- Testing has no branch in the workbook.
- Unmatched descriptions fall through to Excel `FALSE`; recommended behavior is a validation error.
- Preserve an optional explicit override only as a backend enhancement, with source marked `MANUAL` versus `DERIVED`.

Workbook `PROCESS` size/special formulas look up `CUSTOMER P/N` in `ASSY[RACN P/N]`, which is a key mismatch. Resolve both directly from `product_id`.

## 5. Component project aggregates

For each component code, the workbook calculates:

```text
qty_per_set = sum(BOM.quantity)
    where BOM.business_case <> "No" and BOM.component = current_component

total_volume = sum(BOM.forecast_year_1)
    where BOM.business_case <> "No" and BOM.component = current_component
```

Recommended behavior: include only products with `quote_status = YES`. `qty_per_set` is a project-wide sum across products, not a normalized quantity for one finished set; rename it `project_quantity_per_product_set_sum` or clarify its semantics in the UI.

## 6. Operational workflows

### 6.1 Create/update a project

1. Validate project metadata, SOP/EOP, rates, and master-data versions.
2. Upsert products with stable IDs and unique RACN part numbers.
3. Store ten yearly volumes per product.
4. Mark products `YES`/`NO` for quotation.
5. Touch `updated_at` after relevant changes.

### 6.2 Maintain BOM and process

1. Validate product and component/process relationships.
2. Accept ordered lines; do not use blank rows as control characters.
3. Derive display item numbers independently of persistence IDs.
4. Recalculate process cycle-time drivers after BOM changes.
5. Return validation issues before running costing.

### 6.3 Generate product calculations

Workbook VBA offers:

- `CreateSheet`: selected `ASSEMBLIES` row → copy `template` → name it with RACN P/N → write RACN P/N to `I5`.
- `CreateSheetsForRowsToQuote`: repeat for every `ASSY` row whose status is not `No`, skipping existing sheet names.

Backend replacement:

```text
for product in project.products where quote_status = YES:
    validate product inputs
    result = costing_engine.calculate_product(product, input_snapshot)
    store result in calculation_run
```

No physical generation step is required. An export can later render one sheet per result.

### 6.4 Recalculate after input changes

Use a single calculation run per project snapshot:

1. Lock/reference all input versions.
2. Run every quoted product.
3. Run assembly pricing only for successfully costed products.
4. Return project totals together with incomplete-product errors.
5. Do not present partial totals as complete without an explicit completeness flag.

## 7. Validation invariants

### Project/product

- RACN part number is nonblank and unique within project.
- Customer part numbers are text.
- Volumes are non-negative; Year 1 must be positive if tooling amortization is enabled.
- Sales margin must be `< 1`; normally all rates are `>= 0`.
- For amortization, workbook semantics require `eop_year > sop_year`.

### BOM/process

- Quantity is non-negative and component exists.
- UOM is compatible with price and cost drivers.
- In-house tube calculations require OD > ID >= 0 and positive length.
- In-house QC requires component volume, material density, and material cost.
- A costed process requires operation, positive cycle time, and valid cost center.
- Cavity count, when used for cycle/capacity adjustment, must be positive.
- Tooling count and unit cost must both be present when tool description/costing is supplied.
- Amortization requires positive total tooling cost, positive Year 1 volume, and positive year span.

### Lookup integrity

Never emulate Excel's silent first-match behavior for authoritative relationships. Reject duplicate component codes, material part numbers, cost-center codes, and operation names within the active version.
