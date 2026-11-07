# Cost Calculator Backend — Workbook Handover

> **Application requirements:** start with [APP_SPECIFICATION.md](APP_SPECIFICATION.md).
> It consolidates the owner's application decisions and takes precedence over this
> workbook-derived handover wherever behavior differs. Files `00`–`04` remain the
> costing/formula reference; their legacy behavior and suggested MVP scope are not
> automatically application requirements. See the specification's reference map
> and open decisions before implementation.

## 1. Purpose and source

This handover translates the business logic in `NOT RELEASED_ENG-2026-0007_FP - COOLANT_SERIAL_ZERON_T Platform_20261030.xlsm` into implementation guidance for a Python backend.

The intended system is a project-level manufacturing cost and quotation calculator:

1. Create a project and populate product assemblies.
2. Maintain components, materials, cost centers, and operation-cycle master data.
3. Enter BOM and manufacturing-process lines for each product.
4. Calculate one manufacturing-cost result per product.
5. Aggregate product results into assembly-level pricing, margins, tooling, and CAPEX.
6. Optionally compare original/improved products and perform project investment analysis.

The workbook contains 16 generated product sheets (`ENG0001`–`ENG0016`). The current data exercises purchased-material costing, but does **not** provide real examples of in-house production, valid process rows, tooling amortization, or CAPEX. Those branches are documented from formulas and require synthetic tests.

## 2. Document map

- [01_DOMAIN_MODEL_AND_WORKFLOWS.md](01_DOMAIN_MODEL_AND_WORKFLOWS.md): entities, identifiers, input derivation, workflows, and validation.
- [02_MANUFACTURING_COST_ENGINE.md](02_MANUFACTURING_COST_ENGINE.md): exact product-line and product-total calculations.
- [03_PRICING_AND_PROJECT_ANALYSIS.md](03_PRICING_AND_PROJECT_ANALYSIS.md): sales prices, margin rates, turnover, tooling/CAPEX summaries, and optional investment analysis.
- [04_PARITY_DEFECTS_AND_TEST_PLAN.md](04_PARITY_DEFECTS_AND_TEST_PLAN.md): workbook defects, parity decisions, golden vectors, and acceptance tests.

## 3. Workbook responsibility map

| Workbook object | Role | Backend treatment |
|---|---|---|
| `ASSEMBLIES`, table `ASSY` | Project metadata, product list, 10-year volumes, quote status, pricing, margins, tooling, CAPEX, turnover | Core project and product aggregate model |
| `COMPONENTS`, table `COMPONENTS` | Component master and project-level quantity/forecast aggregation | Core master data; aggregates should be calculated, not stored as authoritative inputs |
| `BOM`, table `BOM` | Product-component relationships, quantities, tooling and capacity inputs | Core transactional input |
| `PROCESS`, table `PROCESS` | Product-operation relationships, cycle/tooling inputs | Core transactional input |
| `template` and `ENG0001`–`ENG0016` | Per-product manufacturing-cost calculations | Implement as a calculation engine; a physical sheet per product is unnecessary |
| `CC-2026`, tables `CC_YEAR`, `CYCLES` | Cost-center rates/capacity, operation cycle standards, global rates | Versioned project assumptions/master data |
| `GRANULATES`, table `MATERIALS` | Raw-material density and calculation cost | Versioned material master |
| `EX-RATES` | Currency conversion matrix | Versioned exchange-rate source |
| `总表`, table `Table2` | External actual/target/benchmark prices | Optional quotation benchmark provider, not a costing input |
| `Synthesis`, table `SYNTHESIS` | Original/improved product comparison, turnover and savings | Phase 2; redesign the broken mapping |
| `Investment Analysis` | Cash flow, NPV, and payback | Phase 2; currently incomplete and not automatically linked to assembly CAPEX |
| `SUMMARY` | Legacy quotation summary containing another project's data | Do not use as source of truth |
| `BigBOM`, `Correspondence`, `SINO-VOSS`, `DATBASE` | Legacy/reference data with no core formula dependents | Exclude from core calculation unless separately required |
| `Quality cost template`, `Technical Description` | Quality/documentation functionality | Outside costing scope |

## 4. Calculation dependency graph

```text
Project + ProductAssembly + yearly volumes
                  |
Component <-------+--------> Material + ExchangeRate
                  |
             BOM lines -------+
                  |           |
                  |           +--> line material/purchase cost
                  |           +--> in-house material volume and cost
                  |           +--> BOM tooling and capacity
                  |
CostCenter <--- PROCESS lines --+--> cycle time and production cost
OperationCycle <----------------+--> process tooling
                                  |
                                  v
                       Product manufacturing result
               material + material overhead + production
                    + tooling amortization = subtotal
                                  |
                 production SG&A -> full cost
                 sales margin ----> suggested price
                 productivity -----> price incl. productivity
                                  |
 Actual/benchmark price ----------+--> gross margin rate, EBIT rate,
                                      turnover, project summaries
```

### Required calculation order

1. Resolve project assumptions and master-data versions.
2. Resolve direct product references for every BOM/process line.
3. Calculate BOM derived values and process cycle times.
4. Calculate each product's cost lines.
5. Sum line results into product manufacturing totals.
6. Calculate product pricing and margin rates.
7. Calculate project-level totals and optional comparisons.

Do not calculate assembly totals from worksheet addresses or product names. Use explicit foreign keys and immutable calculation-run snapshots.

## 5. Units and numeric conventions

| Quantity | Workbook unit | Backend rule |
|---|---|---|
| Money | CNY, generally per finished product unless explicitly a total | Use `Decimal`; keep currency explicit |
| Cost-center rate | CNY/hour | Convert seconds to hours with `/3600` |
| Cycle time | Seconds; `Y` is seconds per piece after cavity adjustment | Do not mix batch and per-piece times |
| Tube OD/ID and length | mm | Volume formula converts mm³ to cm³ with `/1000` |
| Material volume | cm³ | — |
| Density | g/cm³ | — |
| Raw-material cost | CNY/kg | Volume × density gives grams; divide by 1000 for kg |
| Rates | Decimal fractions: `0.05` = 5% | Validate ranges; do not accept ambiguous whole-percent values implicitly |
| Years | Calendar years and relative `Year 1`…`Year 10` | Map relative years to calendar years using SOP |
| Volumes | Pieces per year | Must be non-negative |

Preserve full precision through the calculation. Round only at presentation/export boundaries. The workbook does not specify an intermediate rounding policy.

## 6. Recommended backend boundaries

```text
project_service
  Owns project metadata, product assemblies, quote status, yearly volumes.

master_data_service
  Owns components, materials, exchange rates, cost centers, cycle standards,
  and their effective versions.

bom_process_service
  Owns BOM/process lines, stable IDs, ordering, validation, and derived inputs.

costing_engine
  Pure deterministic functions. No HTTP, ORM, Excel, or worksheet references.
  Produces line-level explainable results and product manufacturing totals.

pricing_engine
  Produces suggested/actual-price scenarios, margin rates, turnover,
  tooling and CAPEX summaries.

calculation_run_service
  Freezes input versions, executes costing/pricing, stores results and warnings,
  and supports reproducibility/audit.

export_service (optional)
  Generates Excel or other outputs from calculation results; exports are not
  required for backend calculation.
```

## 7. Parity policy

Support two explicit behaviors where the workbook is defective or ambiguous:

- **`excel_parity`**: reproduce the documented arithmetic where compatibility is required, but still reject corrupt identifiers and unusable inputs rather than generating `#N/A`, `#REF!`, or `#VALUE!` equivalents.
- **`recommended`**: apply the corrections in `04_PARITY_DEFECTS_AND_TEST_PLAN.md`, especially stable line IDs, total tooling summarization, safe empty-process handling, and explicit currency conversion.

Do not hide mode differences. Return the selected mode and applied warnings in each calculation run.

## 8. Suggested delivery phases

### MVP

- Projects, products, components, materials, cost centers, operation cycles.
- BOM and process CRUD with stable IDs.
- Purchased and in-house material costing.
- Process costing, tooling amortization, and product totals.
- Suggested price, productivity-adjusted price, actual-price input/lookup.
- Gross margin and EBIT **rates**, tooling and CAPEX summaries.
- Calculation runs, validation, and golden tests.

### Phase 2

- Benchmark providers and quotation exports.
- Original/improved comparisons.
- Fully specified cash-flow/NPV analysis.
- Excel template export with dynamic ranges rather than fixed row limits.
