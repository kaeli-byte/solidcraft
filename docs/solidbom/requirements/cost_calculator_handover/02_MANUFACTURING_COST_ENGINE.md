# Manufacturing Cost Engine

## 1. Scope and output

Implement a pure calculation that produces one `ProductManufacturingResult` per product, with line-level explanations. The product sheets `template` and `ENG0001`–`ENG0016` are the source specification.

Each result must expose at least:

```text
product_id
currency = CNY
bom_lines[]
process_lines[]
material_cost
material_overheads
total_purchasing
total_production
tooling_amortization_per_unit
subtotal
total_tooling_cost
capex
warnings[]
```

`total_tooling_cost` should use number of tools × unit tooling cost. The workbook's assembly summary currently uses a different, defective formula; see pricing document and defect `D04`.

## 2. Product calculation inputs

The product header resolves:

| Product value | Source |
|---|---|
| Customer, inquiry, project family | Project |
| Description/customer P/N | Product |
| RACN P/N | Product |
| SOP/EOP | Project |
| Annual volume | Product Year 1 volume |
| BOM cost lines | BOM lines whose `product_id` matches and whose record type is BOM |
| Process cost lines | PROCESS lines whose `product_id` matches |

Workbook dynamic arrays:

```excel
D11 = FILTER(BOM[Partnumber], (BOM[RA P/N]=$I$5)*(BOM[Type]="BOM"))
D36 = FILTER(PROCESS[Process Item], PROCESS[RA P/N]=$I$5)
```

Use direct database filtering instead. An empty process list is valid and must return `[]`, not an error.

The Excel layout has formulas for 24 BOM rows (11–34) and 19 process rows (36–54). Do not impose those presentational limits in the backend.

## 3. BOM line resolution

For each BOM line, resolve these values directly from relationships:

| Product-sheet column | Meaning | Backend source |
|---|---|---|
| `C` | Assembly P/N | Product RACN P/N |
| `D` | Position/line key | Stable BOM line ID; display key separately |
| `E` | Annual volume | Product Year 1 volume |
| `F` | Item number | Sequence within product |
| `G` | Origin | Component origin |
| `H` | Component ID | Component code |
| `I` | Item description | Component designation |
| `J`, `K` | Material number/code | Component material and material ISO code |
| `L` | Supplier | Component supplier |
| `M` | Quantity | BOM quantity |
| `N` | UOM | Component/BOM UOM |
| `O` | Category | Component type |
| `P` | Length mm | BOM length |
| `Q`, `R` | OD/ID mm | Component dimensions |
| `S` | Thermoforming | BOM attribute; display only |
| `W`, `X` | Cost center and cycle time | BOM inputs |
| `AJ` | Cavity count | BOM tooling input |

## 4. In-house material volume and raw-material cost

### 4.1 Volume (`T`, `volume_cm3`)

For origin `I` and category `ST` or `CT`:

```text
volume_cm3 = π × (od_mm² - id_mm²) / 4 / 1000 × length_mm
```

This is an annular cylinder. OD/ID/length are mm; dividing mm³ by 1000 yields cm³.

For origin `I` and category `QC`:

```text
volume_cm3 = component.volume_cm3
```

For other origin/category combinations, there is no material volume. Excel returns `FALSE` through a nested `IF`; use `None` in the backend.

Validation: require `od_mm > id_mm >= 0` and positive length for ST/CT. Require non-negative volume for QC.

### 4.2 Density and material rate (`U`, `V`)

The exact workbook rule looks up density and raw-material calculation cost only when:

```text
origin = I AND category IN {ST, QC}
```

The material is selected through the component's material part number.

**Important:** CT gets a volume but does not get density/cost in the workbook. Recommended corrected behavior is to include CT in this lookup; strict parity excludes it and therefore produces zero in-house CT material cost. See defect `D05`.

### 4.3 In-house material cost (`AB`, `in_house_material_cost`)

```text
in_house_material_cost = volume_cm3 × density_g_cm3 × material_cost_cny_kg / 1000
```

The `/1000` converts grams to kilograms. Missing/invalid arithmetic is coerced to zero by Excel `IFERROR`; recommended behavior is to return a validation error for a line expected to have in-house material cost.

## 5. Purchase price and total material/purchase cost

### 5.1 Buy price (`AA`, `buy_price`)

The workbook reads `COMPONENTS[CNY/uom]` and treats nonzero as a usable purchase price. A zero price becomes blank.

Recommended model:

```text
buy_price = component.unit_cost_cny_per_uom  # may be None; preserve true zero explicitly
```

### 5.2 Total purchase/material cost (`AC`, `material_cost`)

Exact arithmetic:

```text
if buy_price is present:
    material_cost = quantity × buy_price
else:
    material_cost = quantity × in_house_material_cost
```

The presence of a buy price takes precedence even when an in-house volume exists. Origin does not directly select the branch.

Recommended validation:

- Purchased (`P`) lines require a purchase price unless intentionally zero-priced.
- In-house (`I`) lines should use in-house material cost. If both purchase price and in-house inputs exist, either reject the ambiguity or record an explicit source policy.
- Do not silently convert missing cost data to zero for an active cost line.

`material_cost` is the value aggregated into assembly `Material cost`, despite the column heading `Total purchase price (CNY)`.

### 5.3 Material overhead (`AD`) and purchasing total (`AE`)

```text
material_overhead = material_cost × project.material_overhead_rate
material_cost_incl_overhead = material_cost + material_overhead
```

With the current project rate, material overhead is 5%.

## 6. Production cost

### 6.1 Cost-center hourly rate (`Z`)

```text
if cost_center is missing or code = 0:
    hourly_rate = 0                        # strict Excel behavior
else:
    hourly_rate = cost_center.base_hourly_rate
```

Recommended behavior: missing cost center on a costed process is an error, not a zero-cost process.

The workbook uses `CC_YEAR[CC]` (base rate), not the inflation-adjusted `CC_YEAR[CC+]`.

### 6.2 Cavity-adjusted cycle time (`Y`)

```text
if cavity_count > 1:
    cycle_seconds_per_piece = cycle_time_seconds / cavity_count
else:
    cycle_seconds_per_piece = cycle_time_seconds
```

For BOM lines, cavity count is populated only for in-house QC tooling. For process lines, it comes directly from the process tooling fields.

### 6.3 Operation production cost (`AF`)

```text
operation_production_cost = cycle_seconds_per_piece × hourly_rate / 3600
```

### 6.4 Total production cost for the finished product (`AG`)

Exact workbook rule:

```text
if origin in {M, MANUAL}:
    total_production_cost = operation_production_cost
elif uom = "m":
    total_production_cost = operation_production_cost
else:
    total_production_cost = operation_production_cost × quantity
```

This distinction is significant:

- Process rows (`M`) treat cycle time as already attributable to one finished product.
- BOM lines measured in meters do not multiply production cost by quantity again.
- Other BOM lines multiply by component quantity.

Keep this rule unless process engineering supplies a more explicit cycle-time basis. If refactoring, add a `cycle_basis` field rather than inferring ambiguously from UOM/origin.

## 7. Tooling and amortization

### 7.1 Tooling fields by line type

For BOM lines, tooling values are pulled only when `origin = I` and `category = QC`.

For PROCESS lines, tooling values are pulled for every valid process line.

| Value | Meaning |
|---|---|
| `tool_description` | Tool identity/description |
| `cavity_count` | Also affects cycle time and capacity |
| `tooling_delivery_weeks` | Informational lead time |
| `tooling_count` | Number of tools |
| `tooling_unit_cost` | Cost per tool, CNY |
| `tooling_comments` | Free text |
| `amortize` | Manual yes/no flag |

### 7.2 Total tooling cost (`AO`)

BOM formula:

```text
if tool_description is nonblank:
    total_tooling_cost = tooling_count × tooling_unit_cost
else:
    total_tooling_cost = blank
```

Process formula always multiplies count × unit cost. Backend should use one consistent rule and validate required tooling fields.

### 7.3 Tooling amortization per finished unit (`AQ`)

```text
if amortize is exactly "yes":
    tooling_amortization = total_tooling_cost
                           / product.year_1_volume
                           / (project.eop_year - project.sop_year)
else:
    tooling_amortization = 0
```

The year span is `EOP - SOP`, **not** `EOP - SOP + 1`. Example: SOP 2027 and EOP 2030 produce a denominator of 3 years.

Recommended backend:

- Use a boolean `amortize`; do not reproduce case-sensitive text matching.
- Require positive Year 1 volume and `EOP > SOP` when amortization is enabled.
- Confirm with finance whether the non-inclusive year span is intentional before changing it.

### 7.4 Line total cost (`AH`)

```text
line_total_cost = material_cost_incl_overhead
                  + total_production_cost
                  + tooling_amortization
```

Tooling amortization is included in unit cost; total tooling cash outlay is not itself added to unit cost.

## 8. CAPEX and investment amortization fields

Product rows also contain:

- `AR` — `Investment required (CNY)`: manual CAPEX input, summed by the assembly sheet.
- `AS` — `Invest amortization [years]`: manual field, not referenced by core formulas.

Core product unit cost does **not** amortize AR CAPEX. Only tooling amortization `AQ` enters `AH`. Do not add CAPEX amortization to full cost unless a separate business requirement defines it.

## 9. Product aggregation

The assembly sheet sums product-sheet rows 10:100. Equivalent backend aggregation over all cost lines:

```text
material_cost = sum(line.material_cost)
material_overheads = sum(line.material_overhead)
total_purchasing = sum(line.material_cost_incl_overhead)
total_production = sum(line.total_production_cost)
tooling_amortization_per_unit = sum(line.tooling_amortization)
subtotal = sum(line.line_total_cost)
total_tooling_cost = sum(line.total_tooling_cost)
capex = sum(line.investment_required)
```

Identity checks:

```text
total_purchasing = material_cost + material_overheads
subtotal = total_purchasing + total_production + tooling_amortization_per_unit
```

The workbook does not expose tooling amortization as a separate assembly column; it is already embedded in `Sub-Total`.

## 10. Reference pseudocode

```python
def calculate_product(ctx, product):
    lines = []

    for bom in ctx.bom_lines.for_product(product.id):
        component = ctx.components.required(bom.component_id)
        volume = material_volume(component, bom)       # ST/CT annulus; QC component volume
        density, material_rate = material_inputs(ctx, component, volume)
        in_house = safe_in_house_cost(volume, density, material_rate)
        material = resolve_material_cost(component, bom.quantity, in_house)
        overhead = material * ctx.project.material_overhead_rate
        hourly = ctx.cost_centers.base_rate(bom.cost_center_id)
        cycle_per_piece = adjust_for_cavities(bom.cycle_time_seconds, bom.cavity_count)
        operation_cost = cycle_per_piece * hourly / 3600
        production = scale_bom_production(operation_cost, component.uom, bom.quantity)
        tooling_total = resolve_bom_tooling(component, bom)
        amortization = amortize_tooling(tooling_total, bom.amortize, ctx.project, product)
        lines.append(make_bom_result(...))

    for process in ctx.process_lines.for_product(product.id):
        cycle = process.cycle_time_seconds
        cycle_per_piece = adjust_for_cavities(cycle, process.cavity_count)
        hourly = ctx.cost_centers.base_rate(process.cost_center_id)
        production = cycle_per_piece * hourly / 3600  # origin M: no quantity multiplier
        tooling_total = process.tooling_count * process.tooling_unit_cost
        amortization = amortize_tooling(tooling_total, process.amortize, ctx.project, product)
        lines.append(make_process_result(...))

    return aggregate(product, lines)
```

The implementation should return detailed intermediate values so every total can be explained and tested.

## 11. Capacity helper calculations

These values do not affect current product cost, but are part of BOM logic:

```text
expected_capacity_pcs = (
    project.working_days × 24 × 3600 × project.capacity_oee
    / cycle_time_seconds
) × cavity_count

lifetime_expected_shots = expected_capacity_pcs / cavity_count
```

The number of tools is manual; it is not inferred from forecast, mold lifetime, or expected capacity.

## 12. Error and precision policy

- Use `Decimal` for money, rates, quantities, and times. Use a sufficiently precise numeric representation for π or a documented float-to-Decimal conversion for geometric volume.
- Do not round intermediate line values.
- Distinguish `not_applicable`, `missing_required_input`, and true numeric zero.
- Do not reproduce `IFERROR(..., 0)` for missing authoritative cost inputs; emit validation errors with product/line/field paths.
- Return all line results and warnings in the calculation run to support audit and export.
