# Pricing and Project Analysis

## 1. Assembly-level pricing inputs

For each `ProductAssembly`, start with the manufacturing result defined in `02_MANUFACTURING_COST_ENGINE.md` and the project rates:

| Rate | Workbook cell | Current value | Application |
|---|---|---:|---|
| Material overhead | `ASSEMBLIES!U14` / `CC-2026!T44` | 5% | Already included in manufacturing subtotal |
| Production SG&A | `ASSEMBLIES!Y14` / `CC-2026!T45` | 89% | Applied only to total production cost |
| Sales margin | `ASSEMBLIES!AA14` / `CC-2026!T46` | 20% | Used as margin on suggested sales price |
| Productivity | `ASSEMBLIES!AB14` / `CC-2026!T47` | 12% | Uplifts suggested sales price |

The 89% SG&A value is not applied to material, full cost, or revenue. Preserve its unusual base unless finance changes the requirement.

## 2. Product pricing formulas

The following mappings correspond to `ASSY` columns.

| Output | Exact formula |
|---|---|
| `material_cost` | Sum product line material/purchase cost |
| `material_overheads` | Sum product line material overhead |
| `total_purchasing` | Sum product line material cost including overhead |
| `total_production` | Sum product line total production cost |
| `subtotal` | Sum product line total cost, including tooling amortization |
| `production_sga` | `total_production × production_sga_rate` |
| `full_cost` | `subtotal + production_sga` |
| `suggested_sales_price` | `full_cost / (1 - sales_margin_rate)` |
| `price_incl_productivity` | `suggested_sales_price × (1 + productivity_rate)` |

Validation: `sales_margin_rate < 1`. No rounding is applied by the workbook.

### Interpretation

The suggested price achieves the target margin on price:

```text
(suggested_sales_price - full_cost) / suggested_sales_price
= sales_margin_rate
```

`price_incl_productivity` is a 12% price uplift in the current workbook, not a cost reduction and not a compounded margin adjustment.

## 3. Actual sales price and benchmarks

Workbook behavior for quoted products:

```excel
Actual Sales Price = XLOOKUP(
    product.customer_part_number,
    Table2[Partnumber],
    Table2[JINAN SALES]
)
```

If the product is not quoted, actual price is blank. Other external lookups are:

| Assembly output | `Table2` source |
|---|---|
| `cost_init` | `COST 2026` |
| `target_sinotruk` | `TARGET 2026` |
| `voss` | `Column29` |
| `chinaust` | `Column31` |

Recommended backend contract:

```text
ActualPriceProvider.get_price(product, price_version) ->
    { amount_cny, source, source_version, effective_date } | MissingPrice
```

Allow an explicit project quote price to override or supplement the external benchmark, with source and audit metadata. Do not use benchmark price as an input to manufacturing cost.

### Missing price behavior

The current workbook returns `#N/A` for missing current-project prices, which propagates into turnover and project totals. Recommended behavior:

- Keep manufacturing cost and suggested prices available.
- Mark actual-price-dependent outputs as `None`/incomplete.
- Exclude incomplete products from a complete project quoted total, or return a partial total explicitly labeled with missing product IDs.
- Never convert a missing actual price to zero silently.

## 4. Gross profit, gross margin, EBIT, and EBIT rate

The workbook columns named `Gross Margin` and `Estimated EBIT` are **rates**, not currency amounts.

```text
gross_margin_rate = (actual_sales_price - subtotal) / actual_sales_price
estimated_ebit_rate = 1 - full_cost / actual_sales_price
                    = (actual_sales_price - full_cost) / actual_sales_price
```

Equivalent currency amounts, if the UI needs them:

```text
gross_profit_per_unit = actual_sales_price - subtotal
estimated_ebit_per_unit = actual_sales_price - full_cost
```

The workbook uses `Sub-Total` for gross margin, so its gross-profit base includes material overhead, production cost, and tooling amortization but excludes production SG&A. Estimated EBIT uses full cost, which includes production SG&A.

Require nonzero actual price for rates. Excel returns blank through `IFERROR`; backend should return `None` with a reason.

## 5. Tooling and CAPEX summaries

### 5.1 Recommended summaries

```text
product_total_tooling_cost = sum(number_of_tools × unit_tooling_cost)
product_capex = sum(line.investment_required_cny)
project_total_tooling_cost = sum(product_total_tooling_cost for quoted products)
project_capex = sum(product_capex for quoted products)
```

Separate these cash-outlay totals from per-unit tooling amortization already included in subtotal.

### 5.2 Workbook tooling defect

`ASSY[Tooling cost]` sums product column `AM` (`Tooling costs (CNY)`, the unit cost per tool), not `AO` (`Total tooling costs (CNY)`, count × unit cost). Strict Excel parity therefore ignores the number of tools. Recommended mode must sum total tooling cost; see defect `D04`.

### 5.3 CAPEX limitations

- CAPEX `AR` is manual per product cost line.
- `Invest amortization [years]` (`AS`) is not used in core calculations.
- CAPEX does not affect product full cost.
- The separate `Investment Analysis` sheet does not automatically consume `ASSY[CAPEX]`; its machine investment is manually entered.

## 6. Turnover and target comparisons

For a quoted product:

```text
turnover = sum(product.yearly_volumes[1:10]) × actual_sales_price
cost_difference = full_cost - cost_init
target_portfolio = target_sinotruk × year_1_volume
```

Turnover is the undiscounted revenue over all ten entered volumes at one constant actual sales price. It is not a yearly series and does not apply escalation, productivity, or discounting.

Project cells:

```text
possible_revenues = sum(turnover where quote_status = YES)
possible_target_portfolio = sum(target_portfolio where quote_status = YES)
```

Recommended API should also expose yearly turnover:

```text
turnover_year_n = volume_year_n × actual_sales_price
```

This is needed for analysis and avoids relying only on the workbook's aggregate.

## 7. Worked current-data example: ENG0001

Current workbook inputs produce no production cost, tooling amortization, or CAPEX for ENG0001:

```text
material_cost                 = 14.0055500000
material_overheads            =  0.7002775000
total_purchasing / subtotal   = 14.7058275000
total_production              =  0
production_sga                =  0
full_cost                     = 14.7058275000
suggested_sales_price         = 14.7058275 / 0.8
                              = 18.3822843750
price_incl_productivity       = 18.382284375 × 1.12
                              = 20.5881585000
```

The current workbook lacks a matching actual sales price, so margin and turnover outputs are not valid golden values.

## 8. Quotation result contract

Suggested product result:

```json
{
  "product_id": "...",
  "racn_part_number": "ENG0001",
  "currency": "CNY",
  "manufacturing": {
    "material_cost": "14.00555",
    "material_overheads": "0.7002775",
    "total_production": "0",
    "subtotal": "14.7058275",
    "production_sga": "0",
    "full_cost": "14.7058275",
    "total_tooling_cost": "0",
    "capex": "0"
  },
  "pricing": {
    "suggested_sales_price": "18.382284375",
    "price_incl_productivity": "20.5881585",
    "actual_sales_price": null,
    "actual_price_source": null,
    "gross_margin_rate": null,
    "estimated_ebit_rate": null
  },
  "completeness": {
    "costing_complete": true,
    "actual_price_complete": false,
    "issues": ["MISSING_ACTUAL_SALES_PRICE"]
  }
}
```

Return values as decimal strings at JSON boundaries if exact decimal preservation is required.

## 9. Optional original/improved comparison (`Synthesis`)

### Intended logic

Each comparison row maps:

- `Original`: a product from the first assembly block.
- `Improved`: a second product intended as the automated/alternative design.
- `Business case`: enables the comparison when exactly `yes`.
- `Year 1`…`Year 10`: original product volumes.
- `Manual`: original full cost.
- `Automatic`: improved full cost.
- `Sales prices`: original actual sales price.

For enabled rows and years 1–10:

```text
turnover_year_n = volume_year_n × original_actual_sales_price
savings_year_n = volume_year_n × (original_full_cost - improved_full_cost)
```

### Required redesign

Do not copy the fixed row-offset mapping (`Original = ASSEMBLIES!E17:E32`, `Improved = E33:E48`) or broken `#REF!` business-case formulas. Use an explicit entity:

```text
ProductComparison
  id, project_id, original_product_id, improved_product_id,
  enabled, business_case_label
```

Validate that both products belong to the project and have compatible volumes/price scenarios.

## 10. Optional investment analysis

The workbook's investment sheet is incomplete, but its main arithmetic is:

```text
capex_t = machines_t + tools_t + fixtures_t + customer_contribution_t
          + hardware_software_t + others_t

payments_in_t = sales_revenue_t + other_revenues_t + cost_savings_t
payments_out_t = production_full_costs_t + other_payments_out_t
net_cash_flow_t = payments_in_t + payments_out_t + capex_t
cumulative_cash_flow_t = sum(net_cash_flow_0:t)
discount_factor_t = (1 + discount_rate) ^ (-t)
discounted_cash_flow_t = net_cash_flow_t × discount_factor_t
NPV = sum(discounted_cash_flow_0:10)
```

Workbook sign convention: payments out and capital expenditure are negative; customer contributions and payments in are positive. Current discount rate is 14%.

The sheet pulls quantities and savings from `SYNTHESIS`, but sales-revenue and full-production-cost input rows are not populated by formulas. Payback currently returns errors. Treat NPV/payback as a separate requirements exercise, not as a validated part of the manufacturing-cost MVP.
