# Shared demand and manufacturing cost baseline contract

Version: **1** of this contract, additive to the published version 2 HTTP API · Status: **published
by SB-16** (owner instruction 2026-11-10: start implementation on issue #31; the locking commit is
recorded in [SB-16's execution record](../../execution/SB-16.md)).

This contract extends version 2 additively. It adds the project's costing settings (representative
volume policy, saved manual exchange rates and purchase price tiers), the pure shared-demand and
manufacturing-cost baseline calculator and the immutable baseline snapshots beside the project
record. The version 1 routes, the version 2 project routes, the engineering review routes, the
manufacturing routes and their meanings stay exactly as published. It does not change the stored
project document's existing fields, the stored engineering review document or the stored
manufacturing document.

Portable types: `crates/solidbom-contract/src/costing.rs` (re-exported at the crate root).
Producers: the service's costing store (`services/solidbom-server/src/costing.rs`) and the costing
routes below; the portable calculator
(`crates/solidbom-costing/src/baseline.rs`, version `solidbom-manufacturing-baseline/1`). Consumers:
the desktop costing surface (later increment), SB-17 (lifetime demand, unit costs and purchased
cutoffs), SB-18 (unit manufacturing cost for pricing), SB-19 (readiness issues) and SB-20 (draft
recalculation and change impact).

Acceptance: the contract crate's `costing` tests, the `solidbom-costing` crate's baseline arithmetic
tests, the service's `tests/costing.rs` domain tests and the costing route tests in `tests/api.rs`.

## Rules that apply to every value

The version 1 rules still apply: business amounts, rates and volumes are decimal strings; a missing
value is `null` and an explicit zero is `"0"`, never interchangeable; unknown JSON fields are
tolerated; a stored schema version this build does not read is refused, never coerced.

Additional rules for this contract:

- One costing currency per project: the project document's `currency`. Supplier prices, rates and
  catalog prices keep their own currency. A different currency converts through a saved manual rate
  expressed as **project-currency units per one source-currency unit**; the converted amount is
  `source amount × rate`. Same-currency conversion is the identity and needs no rate. No live
  exchange-rate integration exists in this contract.
- The calculator is pure and deterministic: the same `BaselineInputs` always produce byte-identical
  `BaselineOutcome` JSON. It performs no catalog lookup, no wall-clock read and no database access.
  It uses checked decimal arithmetic; an unrepresentable operation becomes a named
  `arithmetic_overflow` issue and an unavailable amount, never a panic and never an invented value.
- No intermediate display rounding. Decimal division that does not terminate is rounded by
  `rust_decimal` at its last representable digit; every approved fixture divides exactly or is a
  policy average whose documented precision is this rule.
- A missing input leaves its affected amounts `null` and produces a named issue; it never becomes
  zero. An explicit zero (zero allowance, zero price, zero cycle, zero setup) is a real value.
- The representative costing volume is a demand concept, not a production schedule. Volume never
  selects a cycle time, a rate or a cost-center assumption (only the purchase tier). Rates and
  cycles are the pinned versions the manufacturing contract stores; updating a cost center never
  changes a saved baseline.
- Every cost write names a `request_id` and an `expected_version`. The service keys the request by
  request id plus a canonical payload hash; an identical retry returns the recorded outcome before
  the version check, and a changed payload under the same request id is a conflict. Every accepted
  project costing write appends one immutable project record version. Every baseline calculation
  appends one immutable snapshot.
- A baseline snapshot binds the exact input versions it read (project record version, engineering
  review version, manufacturing version and every selected material, rate, catalog and app-config
  version) and its calculator version. A later edit or recalculation appends a new snapshot; it
  never rewrites an existing one. Editing the costing settings never rewrites saved snapshots.

## Identities

| Value | Meaning | Stability |
|---|---|---|
| `project_id` | The project whose demand and manufacturing baseline is calculated. | Business identity from the version 2 project contract. |
| `project_version` | One immutable project record version; the baseline names the one it read. | Append-only integer. |
| `engineering_version` | One immutable engineering review record version; the volume and catalog evidence the baseline read. | Append-only integer. |
| `manufacturing_version` | One immutable manufacturing record version; the sourcing, routing and attached-cost definitions the baseline read. | Append-only integer. |
| `product_id` | One product of the project; only quoted products contribute demand. | Business identity. |
| `product_revision_id` | The product revision whose resolved structure (occurrence tree) the baseline read. | Business identity. |
| `part_version_id` | One shared project part definition: customer part number with one customer revision. The demand key. | Business identity. |
| `occurrence_id` | One resolved placement of the reviewed structure. Several placements of one part version are several occurrences. | Stable inside one resolved structure. |
| `calculation_id` | One immutable baseline snapshot. | Service-generated UUID. |

## Project costing settings

The project document gains one additive optional field, `costing: ProjectCosting | null`
(`#[serde(default)]`, schema version stays 1; a document written before this contract reads as
`null`, never as an implicit default policy). `ProjectCosting`:

| Field | Meaning | Validation |
|---|---|---|
| `volume_policy` | `average`, `maximum` or `median` representative annual volume, or `null` when the project has not chosen one. | Required before a baseline can select a representative volume. |
| `exchange_rates` | Saved manual rates, each `{ source_currency, project_units_per_source_unit }`. | Three-letter upper-case currency; a strictly positive rate; one rate per source currency; a source equal to the project currency is refused as a redundant identity. |
| `purchase_prices` | Optional purchase price tiers, each `{ part_version_id, currency, tiers: [{ minimum_annual_quantity, unit_price }], source_reference }`. | One table per part version; `minimum_annual_quantity ≥ 1`; a non-negative unit price; minimum quantities strictly increasing and unique; a table for another part is refused; the part version must exist in the project. |

The tiers are the project quotation's agreed purchase prices for that part version. The single
price, when the catalog holds one, is the confirmed catalog price the engineering review records;
the tiers are retained beside it without rewriting the catalog. A purchase tier is the price source
with the greatest `minimum_annual_quantity` not exceeding the part's representative annual costing
volume; a tier source retains its selected minimum quantity. When no tier covers the volume, the
confirmed catalog single price is used; when neither is available the price is missing, never a
guessed tier.

Scenarios (SB-18) cannot override the project's volume policy. They also cannot delete these
settings; they calculate from them.

## Demand

Only **quoted** products contribute demand: a product whose `quotation_state` is `excluded` adds no
annual demand, no manufacturing cost and no setup charge, while its engineering history stays. For
each quoted product the calculator receives its effective schedule (`schedule_override` or the
project default, inclusive SOP–EOP calendar years), its positive annual forecasts and its resolved
product structure.

The resolved product structure is one occurrence tree per product revision: the root occurrences and
each occurrence's resolved `part_version_id` (or `null` when the reviewed association is still
missing) with its nested children. The service resolves it from the registered analysis manifest and
the reviewed component associations; the calculator never reads the manifest itself. An occurrence
whose part version is unresolved is an `unresolved_occurrence` issue and its branch contributes no
demand and no cost.

For one quoted product `p`, one part version `c` and one calendar year `y`:

```text
expanded_quantity[c,p] = number of non-suppressed occurrences of c in p's resolved structure
part_demand[c,y]       = sum over quoted p whose schedule covers y of volume[p,y] × expanded_quantity[c,p]
lifetime_demand[c]     = sum of part_demand[c,y] over that part's applicable years
costing_volume[c]      = volume_policy(part_demand[c, applicable years])
```

- **Aggregate by calendar year before applying the policy.** The policy is applied to the summed
  annual part demand, never to individual products' yearly figures and never to their individual
  peaks: products A demanding 100/200 and B demanding 200/100 give 300/300 and a peak of 300.
- **Applicable years** are the years at least one quoted product that demands `c` is in production
  (`expanded_quantity[c,p] > 0` and `y` inside that product's inclusive schedule). Years in which no
  demanding quoted product is in production are omitted: they add no zero row and no zero padding.
  An applicable year whose demanding product has no positive forecast is incomplete
  (`missing_forecast`), and that year's demand is `null`, never zero.
- `volume_policy`:
  - `average` = `lifetime_demand / count(applicable years)`; the division may be fractional.
  - `maximum` = the greatest annual demand.
  - `median` = the middle annual demand when the count is odd, and the arithmetic mean of the two
    middle values when the count is even.
  A missing policy, a missing or incomplete applicable year or an empty applicable-year set leaves
  `costing_volume` unavailable (and incomplete where a value was required).
- **Purchased branches suppress descendants, not themselves.** An occurrence is suppressed when one
  of its ancestors is an occurrence of a purchased part version. A purchased assembly's confirmed
  price covers its children: descendants in that branch create no demand, no material cost, no
  production cost, no attached cost and no setup charge (tooling recovery is SB-17's consumer of the
  same rule). The purchased assembly's own occurrence still creates demand for the purchased part,
  and the same part version used in another occurrence outside a purchased branch still counts.

## Manufacturing unit cost

Every shared part version the quoted structure uses needs its own `SourcedPart` definition from the
manufacturing contract. A missing sourcing choice is `missing_sourcing`; the part's cost is
unavailable and the result is incomplete. Sourcing is explicit and never inferred from catalog data.

### Internally manufactured parts

```text
net_mass_kg[c]        = net_consumption_volume_mm3 × density_g_cm3 / 1_000_000
allowed_mass_kg[c]    = net_mass_kg × (1 + allowance_rate)
net_material_cost[c]  = allowed_mass_kg × material_price_per_kg (converted to project currency)
production_cost[c]    = Σ over the part routing's operations of
                        per_piece_seconds × production_rate_per_hour (converted) / 3_600
attached_cost[c]      = Σ over attached lines owned by c of quantity × unit_cost
own_unit_cost[c]      = net_material_cost + production_cost + attached_cost
```

The configured categories are applied once on the finished product (see [Configured cost
categories](#configured-cost-categories)), so a part's `unit_cost` never contains a fixed category
amount multiplied by its occurrence quantity. The [rollup](#rollups-and-explained-outputs) later
adds a part's own subtotals to its parents' subtotals exactly once.

- Net consumption comes from the manufacturing record's `MaterialConsumption`: an explicit volume in
  `mm3` or `cm3`, either extracted from the reviewed geometry or entered manually with its reason.
  Its unit is converted once to `mm3`. A missing net consumption, a missing density or a missing
  material price is an issue, never a zero; an explicit zero consumption stays a zero value. A made
  part with no material record at all (for example an assembly whose content cost comes from its
  children) carries no own material cost; the missing record is not itself an issue, but a material
  record that names a material yet lacks its consumption, density or price is incomplete.
- The selected immutable `MaterialVersion` supplies the density and the price per kilogram; its
  currency converts to the project currency through a saved rate when it differs.
- The allowance is applied once, to net consumption only. It changes material cost only: it never
  changes demand, finished-part quantities, production cycle time, setup or tooling recovery.
- `per_piece_seconds` follows the manufacturing contract's cycle basis: a `per_cycle` basis divides
  the cycle by its declared cavity count, a `per_piece` basis never divides. A missing cycle, a
  missing cavity count for a `per_cycle` operation or a missing/zero production rate withholds the
  affected production cost. A missing setup duration only affects the setup charge, never the unit
  production cost.
- A part routing contributes the operations it owns at that part version. A part version with no
  routing has no production cost; that is not an issue by itself (a raw or cut part may only carry
  material and attached costs). Operations owned by a different part version never join it.
- Attached non-CAD lines owned by the part version add `quantity × unit_cost` once per part unit.
  Their unit cost is already in the project currency, as the manufacturing contract states; a
  missing quantity or unit cost withholds that line and the attached subtotal.

### Purchased parts

```text
purchase_price[c] = tier_price(costing_volume[c]) or the confirmed catalog single price
own_unit_cost[c]  = purchase_price (converted to project currency)
                    + production_cost[c] + attached_cost[c]
```

The tier table and the confirmed catalog price keep their own currencies. The calculator converts
only the source it actually selects; a part whose tier table and catalog price are stated in
different currencies is valid and neither amount is mislabelled. The MVP purchase unit is one piece
per occurrence (`pcs`); any other recorded purchase unit makes the price unavailable with an
`unsupported_purchase_unit` issue rather than an invented conversion factor.

- A purchased part carries no manufacturing material. Its confirmed catalog association supplies the
  single price and its currency; the project's tiers may instead select a tier price. A missing
  confirmed association and no covering tier is `missing_purchase_price`; the price is never zero.
- The purchased part's own operations and attached lines still apply: an assembly-level install or
  test operation can be costed at the purchased assembly.
- Price selection happens only for a part version with a positive lifetime demand. A part version
  that is only ever suppressed by a purchased branch needs no price and reports none, without an
  issue: its cost is not used.
- A tier price is selected from the representative annual costing volume, not from lifetime demand
  and not from one product's forecast.

### Configured cost categories

The latest published app cost configuration (`AppConfigVersion`) supplies the categories. They are
evaluated **once on the finished product unit** (and once on the project setup total), never once per
part occurrence: a fixed amount is a per-finished-unit amount, and applying it per part unit would
multiply it by the BOM occurrence count. Percentage categories are distributive for a uniform rate,
so their total is unchanged by where they are evaluated.

Each category is calculated in one of two scopes:

- **Unit scope** — `Direct { material }`, `Direct { production }`, `Fixed { amount }` and
  `Percentage { basis, rate }` whose basis names only unit-scope terms. A unit-scope category
  contributes to the finished product's unit cost.
- **Setup scope** — `Direct { setup }` and a percentage category whose basis names only setup-scope
  terms. A setup-scope category contributes to the separate setup section.

For one finished product, with `material[p]` and `production[p]` the rolled material and production
subtotals from the [rollup](#rollups-and-explained-outputs):

- `Direct { material }` is a **view** of `material[p]`; `Direct { production }` is a view of
  `production[p]`. A direct category reports a subtotal that is already part of the unit cost and
  never adds it a second time.
- `Fixed { amount }` is `amount` converted from the configuration currency to the project currency
  through a saved rate when it differs. One finished product carries it once.
- `Percentage { basis, rate }` is `rate × basis`, where the `direct_cost` term is
  `material[p] + production[p]` (the engine direct-cost subtotal, material and production only,
  never setup) and each named category contributes its own amount. Categories are evaluated in
  dependency order; the configuration validation guarantees an acyclic, non-duplicated basis.
- `Direct { setup }` is a view of the separate project setup total and appears only in the setup
  section. A percentage category whose basis mixes unit-scope and setup-scope terms is
  `ambiguous_category_basis` and withholds its own amount.
- `unit_total[p] = material[p] + production[p] + attached[p] + Σ fixed/percentage unit category
  amounts`. `recurring_unit_cost[p]` is the EBIT cost basis: material + production + attached plus
  only the fixed/percentage categories flagged `included_in_ebit`. Direct categories are already
  inside material/production, so they are always part of it; setup is outside both.
- A category amount that names a category whose own amount is unavailable is unavailable, never
  zero. A missing configuration is `missing_cost_config`; direct material and production are still
  reported, the category lines are unavailable and the result is incomplete.

## Setup

Setup is a separate charge. It is never amortized over a batch and never folded into a unit
manufacturing cost.

```text
setup_charge[routing] = Σ over its operations of setup_hours × setup_rate_per_hour (converted)
setup_base            = Σ over every applicable routing of setup_charge[routing]
setup_total           = setup_base + Σ additional setup-scope percentage category amounts
```

- Each shared part routing's setup is counted **once per project quotation**, even when several
  products use that part version and even when it occurs several times. Its applicable operations
  are summed once.
- A subassembly routing's setup is counted at its own level: once per project quotation, not once
  per occurrence.
- A product-root routing's setup is counted once for that quoted product (its owner is the product).
- A routing whose part version has no non-suppressed occurrence in a quoted product is not
  applicable and contributes no setup.
- A purchased part's own routing still contributes its setup when the purchased part is demanded.
- The setup rate is the operation's dedicated setup rate version and its currency converts through a
  saved rate. A missing setup duration with a recorded setup, or a missing setup rate with a
  positive setup duration, withholds that setup charge and the setup total. A `some(0)` setup
  duration is an explicit zero charge.
- Setup stays outside the per-unit subtotal and outside every percentage category whose basis uses
  `direct_cost`; it is reported beside the unit cost. A `Direct { setup }` category is a view of
  `setup_base` and never adds itself again; a setup-scope percentage category is an additional
  configured setup cost and is added to `setup_total` once.

## Rollups and explained outputs

The calculator reports one line per shared part version, one line per quoted product, the separate
setup section, an actual calendar-year project cost series and one project total.

- **Own subtotals.** Every part version has an own material subtotal (the allowed material cost, or
  the selected purchase price for a purchased part), an own production subtotal (its applied
  routing's operations) and an own attached subtotal (its attached lines).
- **Rolled subtotals.** One occurrence of a non-purchased part version carries its own subtotals plus
  the rolled subtotals of each child occurrence exactly once:
  `material[c] = own_material[c] + Σ_i material[i] × q_i`, and the same for production and attached,
  where `q_i` is the number of child `i` occurrences inside one `c` occurrence. A purchased part's
  rolled subtotals are its purchase price plus its own operations and attached lines; its children
  are never added. The composition of one shared part version is taken from its occurrence in the
  resolved structure; if two occurrences of the same shared part version disagree about their child
  composition the result is incomplete with `inconsistent_part_structure`, and a structure that
  would make a part contain itself is `part_structure_cycle`.
- **Reuse, not whole-project charging.** A shared part's rolled unit cost is calculated once and
  reused with each occurrence. A part appearing twice inside a subassembly used three times
  contributes six units, and its content is charged once per occurrence, never once per product
  using the whole part's project demand.
- **Product unit.** A finished product's material, production and attached subtotals sum its
  product-root own subtotals (its product-root routing and attached lines) and the rolled subtotals
  of its root occurrences. Its `unit_total` then adds the unit-scope fixed and percentage categories
  described above.
- **Project cost series.** For each calendar year with actual quoted demand:
  `project_cost[y] = Σ quoted products unit_total[p] × forecast[p,y]`. The project manufacturing
  total is `Σ_y project_cost[y]` plus the separate setup total. This uses actual annual forecasts,
  not the representative costing volume, so revenue-like annual series stay separate from the
  demand policy. Reaching the exact same lifetime figure through `Σ quoted lifetime demand ×
  unit_total` is equivalent; the annual series is the authoritative explanation.
- **Explanations and determinism.** Every part, product and project value carries explanation lines
  (`CostLine`) naming the amount, its basis and the selected source: demand per year, costing
  volume, peak and lifetime demand; net volume, net mass, material price, net cost, allowance factor
  and allowed cost; per-operation per-piece seconds and production cost; purchase tier and price,
  FX rate and converted amount; attached lines; rollup material/production/attached; category
  amounts; product unit total and recurring unit cost; setup charge per routing and setup total.
  Selected material, production-rate, setup-rate, catalog, tier and app-config versions are
  retained in the snapshot inputs. Decimal values are normalised (trailing zeros stripped) at the output boundary so repeated
  runs on identical inputs are byte-identical; parts are ordered by part version id, products by
  product id, demand years ascending, categories in configuration order and issues by
  `(code, field_path)`.

## Incomplete results and issues

Issues are computed on read and never stored as an inferred value. A missing input makes its
affected amount `null` and the result `incomplete`; a result is `complete` only when every part,
product and project amount is available and no issue was raised.

| Code | Field | Meaning |
|---|---|---|
| `missing_volume_policy` | `project.costing.volume_policy` | The project never selected a representative volume policy. |
| `missing_cost_config` | `category_config` | No published app cost configuration exists. |
| `missing_forecast` | `products[i].forecasts` | A demanding quoted product has no positive forecast for an applicable year. |
| `missing_revision_selection` | `products[i].product_revision_id` | A quoted product has no selected revision for this calculation. |
| `missing_product_structure` | `products[i].occurrences` | A quoted product's selected revision has no resolved structure. |
| `unresolved_occurrence` | `products[i].occurrences` | One occurrence's reviewed part association is unresolved; its branch is uncosted. |
| `missing_sourcing` | `parts[i].sourcing` | The part version has no explicit purchased/manufactured choice. |
| `missing_consumption` / `invalid_consumption` | `parts[i].material.quantity` | A made part has no net consumption, or a negative one. |
| `missing_density` / `invalid_density` | `parts[i].material.density_g_cm3` | No selected material density, or a non-positive one. |
| `missing_material_price` / `invalid_material_price` | `parts[i].material.price_per_kg` | The material has no price, or a negative one. |
| `missing_allowance` / `invalid_allowance` | `parts[i].material.allowance_rate` | No allowance was entered, or a negative one. |
| `missing_cycle` / `invalid_cycle` | `parts[i].operations[j].cycle.seconds` | A routed operation has no cycle record/cycle time, or a negative one. |
| `missing_cavities` / `invalid_cavities` | `parts[i].operations[j].cycle.cavities` | A `per_cycle` operation has no cavity count, or a zero one. |
| `missing_production_rate` / `invalid_production_rate` | `parts[i].operations[j].cycle.rate_per_hour` | No pinned production rate, or a non-positive one. |
| `missing_setup_hours` / `invalid_setup_hours` | `parts[i].operations[j].setup.hours` | A recorded setup has no duration, or a negative one. |
| `missing_setup_rate` / `invalid_setup_rate` | `parts[i].operations[j].setup.rate_per_hour` | A positive setup duration has no positive dedicated rate. |
| `missing_attached_quantity` | `parts[i].attached_costs[j].quantity` | An attached line has no quantity. |
| `missing_attached_unit_cost` | `parts[i].attached_costs[j].unit_cost` | An attached line has no unit cost. |
| `missing_purchase_price` | `parts[i].purchase` | A demanded purchased part has no covering tier and no confirmed catalog single price. |
| `unsupported_purchase_unit` | `parts[i].purchase.price_unit` | The purchase price unit is not one piece per occurrence. |
| `inconsistent_part_structure` | `parts[i]` | Two occurrences of one shared part version disagree about their child composition. |
| `part_structure_cycle` | `parts[i]` | The resolved structure would make one shared part version contain itself. |
| `invalid_structure_depth` | `products[i].occurrences` | The resolved product structure is deeper than the supported limit. |
| `missing_exchange_rate` / `invalid_exchange_rate` | `exchange_rates` | A different currency has no saved rate, or a non-positive one. |
| `ambiguous_category_basis` | `categories.{code}` | A percentage basis mixes unit-scope and setup-scope terms. |
| `invalid_purchase_tier` | `project.costing.purchase_prices` | A purchase tier table breaks its validation rule. |
| `invalid_quantity` / `arithmetic_overflow` | the affected line | A quantity is zero or the checked arithmetic exceeded the supported range. |
| `no_quoted_products` | `products` | No quoted product contributes to the baseline; totals stay zero and the result is incomplete. |

## Baseline calculation

`CalculateBaseline { request_id, project_version, engineering_version?, manufacturing_version?,
product_revisions }` names the exact project record version to cost and one explicit revision
selection per product; the optional record versions default to the current engineering and
manufacturing versions and are recorded in the snapshot either way. A product without an entry
uses its active revision; a quoted product with neither an entry nor an active revision is
`missing_revision_selection` and contributes no demand, never a silently chosen latest draft. The
calculation reads the versioned records, resolves the portable inputs on the service side, runs the
pure calculator and appends one immutable `BaselineSnapshot`:

| Field | Meaning |
|---|---|
| `calculation_id` | The snapshot identity. |
| `project_id`, `project_version` | The project and the exact project record version costed. |
| `engineering_version`, `manufacturing_version` | The review and sourcing versions read. |
| `input_hash` | Canonical hash of the resolved `BaselineInputs`. |
| `calculator_version` | `solidbom-manufacturing-baseline/1`. |
| `inputs` | The resolved portable inputs, including every selected product revision id, version id, currency, quantity and unit. |
| `outputs` | The `BaselineOutcome` with its state, lines, issues and explanations. |

An identical retry returns the recorded snapshot. A changed payload under the same request id is a
conflict. A snapshot is never rewritten; recalculation appends. A historical snapshot stays readable
after the project, reference data or app configuration changes.

## HTTP routes

Base path `/v2`; a bearer identity with the Engineer permission set from the version 2 permissions
contract, bounded requests exactly as version 1. Errors use the version 1 `ApiErrorBody` shape and
codes. A stale `expected_version` is `409 conflict`; a malformed or refused payload is `422 invalid`;
an unknown project, record version or calculation is `404 not_found`; a missing permission is `403`.

| Method | Path | Request | Success | Body |
|---|---|---|---|---|
| `PUT` | `/v2/projects/{project_id}/costing` | `UpdateProjectCosting` | `200` | `ProjectWriteOutcome` |
| `POST` | `/v2/projects/{project_id}/baseline-calculations` | `CalculateBaseline` | `201` | `BaselineSnapshot` |
| `GET` | `/v2/projects/{project_id}/baseline-calculations` | — | `200` | `BaselineCalculationList` |
| `GET` | `/v2/projects/{project_id}/baseline-calculations/{calculation_id}` | — | `200` | `BaselineSnapshot` |

There is deliberately no route in this contract that activates a revision, accepts a baseline,
adopts updated shared data or prices a scenario; those belong to SB-19, SB-20 and SB-18.

## Persistence

The costing settings live in the additive project document field, so migration
`services/solidbom-server/migrations/0009_costing_baseline.sql` adds only the append-only snapshots:

- `baseline_calculations` (`calculation_id` primary key, `project_id`, `project_version`,
  `engineering_version`, `manufacturing_version`, `input_hash`, `calculator_version`, `inputs`,
  `outputs`, `created_at`), immutable and read-only.

The shared `requests` table keeps idempotent request receipts for the costing write and the baseline
calculation. The project record, engineering record, manufacturing record, reference-data and
feasibility tables are unchanged.

## Documented limitations and decisions

These decisions preserve a confirmed rule where the sources are silent or where a published
neighbouring contract cannot express the requested value. Each is a deliberate deferral, not a
silent reinterpretation.

- **Mass-based manual consumption override (deferred).** The sourcing contract stores a made part's
  net consumption as a volume in `mm3` or `cm3`; it has no mass unit. This contract therefore
  supports the volume path plus the selected material density, which covers the confirmed net
  consumption policy (geometry → volume → mass → price). A mass-based manual override needs an
  additive extension of the sourcing contract; it is not invented here, and a volume is never
  reinterpreted as a mass.
- **`missing_material` stays the sourcing contract's issue.** A made part with no material record at
  all carries no own material cost in this baseline (an assembly's content cost comes from its
  children). SB-16 does not re-derive the sourcing contract's `missing_material` readiness issue;
  SB-19 readiness reads both issue lists and must not treat the baseline's complete state as the
  whole readiness decision.
- **Explicit zero cycle or setup with no pinned rate.** The arithmetic reports an exact zero: a zero
  cycle second costs no production, and zero setup hours cost no setup, without requiring a rate.
  The sourcing contract still publishes `missing_production_rate`/`missing_setup_rate` for a record
  that carries no pinned rate, and SB-19 keeps that issue; this baseline does not overwrite it.
- **One purchase unit.** The MVP purchase price is per one piece occurrence (`pcs`). Any other
  recorded purchase unit is `unsupported_purchase_unit`, never an implicit factor.
- **Reference data currencies.** Published materials, rates and catalog prices keep the currency the
  shared-data contract records (CNY in the current deployment). A project currency other than that
  converts through a saved rate; extending the shared-data currency restriction is a separate
  contract change and not assumed here.
- **Analysis identity.** A snapshot retains the resolved occurrence tree, the selected product
  revision ids and the engineering record version; the registered analysis key itself is not a
  separate snapshot field because the exact tree it produced is already in the inputs. A later
  re-registration of an analysis for the same revision is a new calculation, not a rewrite.

## Compatibility

- Version 1, version 2, the SB-11 engineering routes, the SB-13 recognition routes, the SB-14
  feasibility routes and the SB-15 manufacturing routes are unchanged; this contract is additive.
- Adding the optional `costing` field to the stored project document is compatible: a document
  written earlier has no field, reads as `null`, validates under schema version 1 and never gains an
  implicit policy. A build before this contract tolerates the unknown field.
- Adding the routes above is compatible. Changing a field meaning, a wire name, a unit, a route, a
  status, an error code, an idempotency rule, the snapshot immutability or a validation rule that
  previously accepted a write is breaking and needs a new contract version, the affected-consumer
  list and coordinator approval.
- Historical snapshots stay readable under the schema version that wrote them.

## Handoff to later packages

- **SB-17 (tooling):** consumes the annual part demand, lifetime demand, purchased-branch
  suppression and part unit costs. It must not re-derive demand from raw manifests or multiply a
  shared part's whole project demand per product.
- **SB-18 (commercial scenarios):** consumes the product unit manufacturing cost and the separate
  setup charge; scenario pricing must keep setup and tooling outside recurring EBIT and must not
  override the volume policy or the saved rates. EBIT target and productivity settings remain its
  own additive project fields.
- **SB-19 (activation):** treats any open baseline issue as a readiness input; this contract
  publishes no activation decision and no accepted-baseline record.
- **SB-20 (dependency updates):** recalculates affected draft baselines when forecasts, scope,
  deliveries, engineering review, manufacturing definitions, reference data or app configuration
  change; it appends new snapshots and flags saved scenarios. It does not mutate a snapshot. The
  snapshot's recorded record/version ids are the change evidence for its impact reports.
- **Desktop increment:** the typed engine commands, controller state and the costing surface are a
  later increment of this package; they depend on the still-open SB-10 desktop wiring.
