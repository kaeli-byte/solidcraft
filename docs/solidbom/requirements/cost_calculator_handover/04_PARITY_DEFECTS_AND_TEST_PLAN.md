# Parity Decisions, Defect Register, and Test Plan

## 1. Purpose

This file separates reliable business rules from workbook behavior that is broken, stale, ambiguous, or unexercised. A coding agent should not infer desired behavior from cached Excel errors.

Recommended default: implement corrected behavior, while retaining an explicit `excel_parity` option only where numerical compatibility is genuinely required.

## 2. Defect and ambiguity register

| ID | Severity | Area | Workbook behavior | Backend decision/recommendation |
|---|---|---|---|---|
| D01 | **Critical** | BOM identity | Item number resets after a blank product separator. `ENG0002` has six duplicate display part numbers; product `XLOOKUP` uses the first match and prices the wrong rows | Use immutable line IDs and direct foreign keys. Display sequence must not be a lookup key. Add regression T03 |
| D02 | High | Empty process list | Product `C36` contains `#REF!`; `D36` empty `FILTER` propagates `#VALUE!` through process row 36 | Return an empty process list and zero process totals when no processes exist |
| D03 | Medium | Lookup keys | Product thermoforming looks up component code in BOM part numbers; PROCESS size/special looks up customer P/N in RACN P/N | Resolve all descriptive attributes through direct product/component relationships |
| D04 | **High** | Tooling summary | `ASSY[Tooling cost]` sums `AM` unit tool cost, ignoring `AL` number of tools and `AO` total tooling cost | Recommended: sum `tooling_count × tooling_unit_cost`; strict parity is allowed only for compatibility |
| D05 | **High** | In-house CT material | Volume supports ST and CT, but density/material-rate lookup supports only ST and QC; in-house CT material cost becomes zero | Recommended: include CT in density/rate lookup; require material inputs. Strict parity reproduces zero |
| D06 | Medium | Cost-center rate | `CC+` calculates midlife inflation, but product costing uses base `CC` | Make `base` versus `midlife` an explicit rate policy. Default to base for parity |
| D07 | **High** | FX conversion | Material cost divides unit cost by the CNY column; visible exchange-rate table appears to express CNY per currency, which would require multiplication | Confirm rate convention. Recommended explicit `cny_per_unit_currency` and multiplication; current CNY data is unaffected |
| D08 | High | Missing inputs | Multiple `IFERROR(...,0)` expressions turn missing/invalid data into zero cost | Distinguish true zero from missing/invalid. Reject incomplete active cost lines |
| D09 | High | Actual price | Missing `Table2` match returns `#N/A`; turnover and project totals inherit errors | Return structured `MISSING_ACTUAL_SALES_PRICE`; keep cost results, mark price-dependent totals incomplete |
| D10 | Medium | Tool amortization | Denominator is `Year1 × (EOP-SOP)`, with no zero guard; `amortize` must be lowercase `yes` | Use boolean and validate positive volume/year span. Confirm whether year span should remain non-inclusive |
| D11 | Medium | CAPEX | `AR` is summarized but not amortized; `AS` investment-amortization years is unused; investment sheet is manually populated | Do not include CAPEX in unit cost without a new finance rule. Keep CAPEX summary separate |
| D12 | Medium | Quote status | Bulk sheet generator treats blank and any non-`No` value as quotable; turnover uses exact `Yes` | Normalize to required YES/NO enum; only YES enters quotation totals |
| D13 | Low | Layout limits | Product formulas cover 24 BOM and 19 PROCESS rows; assembly sums rows 10:100 | Backend and export queries must be dynamic; never use these as business limits |
| D14 | Medium | Synthesis | Original/improved mapping uses fixed row offsets; several business-case formulas contain `#REF!` | Use explicit `ProductComparison` relationships |
| D15 | Low | Legacy sheets | `SUMMARY` contains another project's metadata; `BigBOM`, `DATBASE`, `Correspondence`, `SINO-VOSS` have no core dependents | Exclude from authoritative calculation and migration unless separately requested |
| D16 | Low | Component aggregate | `Qty per set` sums quantities across all quoted products, so it is not normalized to one set | Rename/clarify the aggregate and expose per-product quantities separately |
| D17 | High | Process operations | Only Cutting, Protection, Assembly, Packaging have automatic cycle branches; Testing has no time and unmatched operations fall through | Validate supported operation and positive standard time; do not silently cost unknown operations as zero |
| D18 | Medium | Cost source precedence | Any nonzero component purchase price overrides in-house material cost, even for origin I | Make source policy explicit; reject ambiguous in-house lines or require deliberate purchase-price override |
| D19 | High | Master-data keys | Excel `XLOOKUP` takes the first match; uniqueness is not enforced | Enforce unique active component codes, material numbers, cost centers, and operation names per version |
| D20 | Medium | Investment analysis | Revenue/full-cost rows are not linked; current NPV/payback outputs are incomplete or erroneous | Exclude from MVP until cash-flow requirements are supplied |
| D21 | Low | Product process numbering | First process row references earlier unrelated cells for item numbering; process `C36` is broken | Generate process sequence directly from ordered process records |
| D22 | Low | Price validity | Component volume validity and last-offer fields do not affect cost selection | Add effective-date/version policy if required; do not imply the workbook already validates offers |

## 3. Current workbook data limitations

- All populated BOM component origins are purchased (`P`).
- There are no authoritative PROCESS input rows.
- Current product production, tooling, and CAPEX totals are zero.
- Current actual-price lookups do not match the ZERON customer part numbers.
- Cached formula errors exist in ASSEMBLIES, product process rows, Synthesis, SUMMARY, and Investment Analysis.
- ENG0002 is affected by duplicate BOM display IDs.

Therefore, current cached values are not sufficient to validate the full engine. Use both current-data golden tests and synthetic branch tests.

## 4. Golden test vectors

### T01 — Purchased-material product: ENG0001

Independent recomputation from BOM quantities and component prices gives:

| Output | Expected |
|---|---:|
| Material cost | 14.0055500000 |
| Material overhead at 5% | 0.7002775000 |
| Total purchasing | 14.7058275000 |
| Total production | 0 |
| Subtotal | 14.7058275000 |
| Production SG&A | 0 |
| Full cost | 14.7058275000 |
| Suggested price at 20% margin | 18.3822843750 |
| Price incl. 12% productivity | 20.5881585000 |

This matches the workbook's cached manufacturing values within floating-point tolerance.

### T02 — Mixed material, process, tooling, and pricing

Inputs:

```text
Purchased BOM line: quantity 2, price CNY 3.8/unit
Process line: cycle 12 s, cavities 2, base cost-center rate CNY 99.72/h
Process tooling: 2 tools × CNY 10,000; amortize = true
Year 1 volume: 2,000; SOP/EOP span: 3 years
Material overhead: 5%; production SG&A: 89%
Sales margin: 20%; productivity: 12%; actual sales price: CNY 20
```

Expected intermediate and final values:

| Output | Expected |
|---|---:|
| Material cost | 7.600000000000 |
| Material overhead | 0.380000000000 |
| Total purchasing | 7.980000000000 |
| Cavity-adjusted cycle | 6 s/piece |
| Total production | 0.166200000000 |
| Total tooling cost | 20,000 |
| Tool amortization/unit | 3.333333333333… |
| Subtotal | 11.479533333333… |
| Production SG&A | 0.147918000000 |
| Full cost | 11.627451333333… |
| Suggested sales price | 14.534314166666… |
| Price incl. productivity | 16.278431866666… |
| Gross margin rate at actual 20 | 0.426023333333… |
| Estimated EBIT rate at actual 20 | 0.418627433333… |
| Assembly tooling, current defective Excel rule | 10,000 |
| Assembly tooling, recommended rule | 20,000 |

### T03 — Duplicate BOM IDs: ENG0002 regression

Cause: blank BOM row 29 resets the item sequence at row 30. The following display IDs each occur twice:

```text
ENG0002-1, ENG0002-2, ENG0002-3,
ENG0002-4, ENG0002-5, ENG0002-6
```

The product sheet returns those IDs from `FILTER` and then resolves each with first-match `XLOOKUP`, so later distinct lines are ignored and earlier lines are costed again.

| Measure | Value |
|---|---:|
| Correct raw material sum across all 17 actual BOM lines | 202.7529600000 |
| Correct material overhead at 5% | 10.1376480000 |
| Correct subtotal/full cost with no production | 212.8906080000 |
| Correct suggested price | 266.1132600000 |
| Correct price incl. productivity | 298.0468512000 |
| Current defective cached material cost | 354.4148000000 |
| Current defective cached subtotal/full cost | 372.1355400000 |

Acceptance: backend corrected mode must calculate from the 17 stable line IDs, not reproduce the defective cached total. A strict Excel importer should flag the source as invalid rather than silently choosing a match.

### T04 — In-house annular tube material

Inputs:

```text
Origin I, category ST, OD 20 mm, ID 16 mm, length 100 mm,
density 1.02 g/cm³, material cost CNY 50.1/kg, quantity 1
```

Expected:

```text
volume = π × (20² - 16²) / 4 / 1000 × 100
       = 11.309733552923255 cm³ (approximately)
in-house material cost = volume × 1.02 × 50.1 / 1000
                       = 0.5779500040214842 CNY (approximately)
```

Run the same geometry as CT. Recommended mode must produce the same material-cost behavior; strict parity yields zero because CT density/rate is not selected.

### T05 — BOM capacity helper

Inputs: 248 working days, OEE 0.9, cycle 10 seconds, 2 cavities.

```text
expected_capacity = 248 × 24 × 3600 × 0.9 / 10 × 2
                  = 3,856,896 pieces
lifetime_expected = expected_capacity / 2
                  = 1,928,448
```

### T06 — Cost-center rate policy

For base labor/manufacturing/depreciation rates `36 + 50.04 + 13.68 = 99.72`, inflation years 3, and inflation rates 4%/1%/1%:

```text
base_hourly_rate = 99.72
midlife_hourly_rate = 106.14588372
```

Core Excel parity uses 99.72. Test both policies explicitly.

### T07 — Automatic process cycle drivers

Given one product with:

```text
ST lines: quantities 1.5 and 1.0 (2 lines)
SHT quantity: 4
QC line 1: quantity 3, tubes_to_assemble 1
QC line 2: quantity 2, tubes_to_assemble 2
```

Expected cycle times:

| Operation | Calculation | Expected seconds |
|---|---|---:|
| Cutting | `(1.5 + 1.0) × 3.6` | 9 |
| Protection | `4 × 15` | 60 |
| Assembly | `(3×1 + 2×2) × 10` | 70 |
| Packaging | `2 ST lines × 5` | 10 |

## 5. Edge-case tests

At minimum, automate:

1. Product with no BOM and no process: reject as incomplete or return explicitly empty, according to API policy.
2. Product with BOM but no process: valid; production cost zero, no spreadsheet-style error.
3. Missing component/material/cost center: structured validation error.
4. Duplicate active master-data key: reject before calculation.
5. Zero-price component versus missing price: distinct behavior.
6. Actual price missing or zero: suggested price remains available; margin rates are `None` with reason.
7. Sales margin = 1: validation error.
8. Amortization with Year 1 = 0 or EOP = SOP: validation error.
9. Cavity count 0 with cycle/capacity use: validation error.
10. Tool count 2 with unit cost 10,000: total tooling 20,000 in recommended mode.
11. CAPEX present: included in CAPEX summary only, not unit cost.
12. Non-CNY material: test the selected explicit FX convention.
13. Unknown process operation or missing standard time: validation error.
14. More than 24 BOM or 19 process lines: all lines included.
15. Product quote status NO: excluded from quoted project totals.
16. Reordered BOM lines: unchanged costs; display sequence may change, stable IDs may not.
17. Rerun with identical input versions: byte-equivalent numeric results and same warning codes.
18. Master-data change after a run: old calculation run remains reproducible.

## 6. Test layers

### Unit tests

Test pure functions for volume, material cost, overhead, cavity adjustment, production scaling, tooling, amortization, pricing, and margin rates.

### Integration tests

Test BOM/process edits, product recalculation, actual-price provider behavior, project totals, and calculation-run snapshots.

### Excel parity tests

Use T01 and explicitly selected parity-only rules. Do not use defective cached ENG0002 or error-containing Synthesis/Investment results as proof of correctness.

### Property-based tests

Useful invariants:

```text
total_purchasing = material_cost + material_overheads
subtotal = total_purchasing + total_production + tooling_amortization
full_cost = subtotal + production_sga
suggested_price × (1 - sales_margin_rate) = full_cost
gross_profit_per_unit = actual_price × gross_margin_rate
estimated_ebit_per_unit = actual_price × estimated_ebit_rate
```

All quantities, volumes, costs, and capacities should be finite and non-negative when valid nonnegative inputs are supplied.

## 7. Numeric acceptance

- Rational money/cost calculations: compare exact `Decimal` values or use tolerance no greater than `1e-12` in unit tests.
- Geometric formulas involving π: use a documented tolerance such as `1e-9` absolute for unit-cost examples.
- Apply no intermediate rounding.
- JSON snapshots should serialize Decimals consistently, preferably as strings.

## 8. Business decisions still requiring confirmation

1. Should in-house CT use the same density/material-cost lookup as ST? Recommended: yes.
2. Is base cost-center rate intentional, or should midlife inflated `CC+` drive product cost? Current: base.
3. Is tooling amortization over `EOP-SOP` correct, or should the period include both endpoint years? Current: non-inclusive difference.
4. Should assembly tooling summary be count × unit cost? Recommended: yes.
5. What exact FX quote direction should the backend store? Recommended: CNY per unit of source currency.
6. Can an in-house component deliberately have a purchase-price override? Current arithmetic allows it implicitly.
7. Should actual sales price be manually entered, benchmark-derived, or selected from versioned scenarios?
8. Should unknown/missing inputs block the entire project run or only mark affected products incomplete? Recommended: product-level errors with explicitly partial project totals.
9. Is CAPEX amortization ever required in unit full cost? It is absent from the workbook core.
10. Are NPV/payback and original/improved comparisons in scope for the backend release? Current workbook implementations are incomplete.
