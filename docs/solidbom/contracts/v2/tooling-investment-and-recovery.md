# Tooling investment and recovery contract

Version: **1** of this contract, additive to the published version 2 HTTP API · Status: **published
by SB-17** (owner instruction 2026-11-11: start implementation on issue #32; the locking commit is
recorded in [SB-17's execution record](../../execution/SB-17.md)).

This contract extends version 2 additively. It adds the project's tooling definitions (the default
commercial treatment and the tool entries owned by one part version or one product), the pure
tooling investment/allocation/recovery calculator and one read-only evaluation route that applies
scenario recovery assumptions to an immutable SB-16 baseline snapshot. The version 1 routes, the
version 2 project routes, the engineering, manufacturing, feasibility and costing routes and their
meanings stay exactly as published. It does not change the stored project document's existing
fields, the stored engineering/manufacturing/feasibility documents or a stored baseline snapshot.

Portable types: `crates/solidbom-contract/src/tooling.rs` (re-exported at the crate root).
Producers: the service's tooling adapter (`services/solidbom-server/src/tooling.rs`) and the routes
below; the pure calculator (`crates/solidbom-costing/src/tooling.rs`, version
`solidbom-tooling-recovery/1`). Consumers: SB-18 (scenario recovery assumptions and commercial
pricing), SB-19 (cost completeness), SB-20 (change impact) and the desktop tooling surface (later
increment of this package).

Acceptance: the contract crate's `tooling` tests, the `solidbom-costing` crate's tooling arithmetic
tests, the service's `tests/tooling.rs` domain tests and the tooling route tests in `tests/api.rs`.

## Rules that apply to every value

The version 1 and version 2 rules still apply: business amounts, rates and quantities are decimal
strings; a missing value is `null` and an explicit zero is `"0"`, never interchangeable; unknown
JSON fields are tolerated; a stored schema version this build does not read is refused, never
coerced.

Additional rules for this contract:

- The calculator is pure and deterministic: the same `ToolingInputs` always produce byte-identical
  `ToolingOutcome` JSON. It performs no catalog lookup, no wall-clock read and no database access.
  It uses checked decimal arithmetic; an unrepresentable operation becomes a named
  `arithmetic_overflow` issue and an unavailable amount, never a panic and never an invented value.
- No intermediate display rounding. Decimal division that does not terminate is rounded by
  `rust_decimal` at its last representable digit. Every approved fixture divides exactly.
- A missing input leaves its affected amounts `null` and produces a named issue; it never becomes
  zero. An explicit zero (zero tool cost, zero cost per tool) is a real value.
- Investment is **never** multiplied by demand, occurrence counts or allocation. It is counted once
  per tool entry. Allocation explains product shares of that one investment; it does not create
  additional investment.
- Investment, amortized recovery revenue and the separate charge are distinct values. An
  evaluation selects exactly one treatment, so one investment is never recovered through both a
  full separate charge and full amortization.
- The project tooling settings live in the append-only project document, so an accepted tooling
  write appends one immutable project record version. Editing the settings never rewrites an
  earlier version and never changes an earlier baseline snapshot.
- A tooling recovery evaluation is a pure read of an immutable SB-16 baseline snapshot plus the
  tooling definitions of that snapshot's exact project record version and the supplied scenario
  assumptions. It stores nothing and rewrites nothing. A scenario result is reproducible from its
  baseline id, project version and recovery inputs.
- The evaluation reads the demand the baseline already resolved. It never re-derives demand from a
  raw manifest, never expands a product's whole project demand per product, and never exceeds the
  purchased-branch cutoff the baseline applied.

## Identities

| Value | Meaning | Stability |
|---|---|---|
| `project_id` | The project whose tooling is evaluated. | Business identity from the version 2 project contract. |
| `project_version` | One immutable project record version; the evaluated tooling definitions are the ones that version holds. | Append-only integer. |
| `baseline_calculation_id` | One immutable SB-16 baseline snapshot; the demand and purchased cutoffs the evaluation reads. | Service-generated UUID. |
| `tool_id` | One tooling entry: one physical investment owned by one part version or one product. | Business identity, stable across project versions. |
| `part_version_id` | One shared project part definition. A part-version owner covers both a component and an assembly. | Business identity. |
| `product_id` | One quoted product; a product-root tool is owned by it. | Business identity. |
| `tool_owner` | The single part version or product that owns the tool entry; the investment is never split across definitions in MVP. | Business identity, fixed by the entry. |
| `recovery_quantity` | The positive quantity a tool's investment is divided by: the owner's combined lifetime demand or a manually entered positive quantity. | Scenario assumption, retained with the scenario by SB-18. |

## Project tooling settings

The project document gains one additive optional field, `tooling: ProjectTooling | null`
(`#[serde(default)]`, schema version stays 1; a document written before this contract reads as
`null`, never as an implicit treatment). `ProjectTooling`:

| Field | Meaning | Validation |
|---|---|---|
| `default_treatment` | The project's default commercial treatment (`separate` or `amortized`), or `null` when the project has not chosen one. A scenario may override it. | Enum value only; `null` is allowed and is not an implicit default. |
| `entries` | The project's tool entries, at most 10,000. | Unique `tool_id`; each entry validated as below. |

`ToolingEntry`:

| Field | Meaning | Validation |
|---|---|---|
| `tool_id` | Stable business identity of the entry. | Non-nil UUID; unique inside the project. |
| `owner` | `{ "kind": "part_version", "part_version_id": … }` or `{ "kind": "product", "product_id": … }`. | The named part version or product must exist in the same project document; one entry names exactly one owner. |
| `name` | Optional explanation label. | Non-blank when present, at most 200 characters. |
| `tool_count` | How many physical tools this entry buys. | At least 1. |
| `cost_per_tool` | The cost of one tool, in `currency`. | Non-negative; an explicit zero is a real value. |
| `currency` | The three-letter upper-case currency of `cost_per_tool`. | Three upper-case letters; converts to the project currency through a saved rate when it differs. |
| `source_reference` | Optional provenance text. | Non-blank when present, at most 200 characters. |

An entry is the sole statement of the investment: `tool_count × cost_per_tool` is the whole
investment of that tool. Two entries of 10,000 are 20,000 of project investment counted once. No
occurrence of a part version, no repeated use of an assembly and no second product multiplies it.

`apply_tooling` replaces the whole settings value and keeps every product, package, part version
and recorded event. Replacing the settings appends one project record version.

## Demand the calculator consumes

The service resolves `ToolingDemand` from one immutable SB-16 baseline snapshot. It contains only
**quoted** products (the baseline's product lines), and the expanded `part_quantities` are the
baseline's non-suppressed occurrence counts per finished product; a purchased-branch descendant
counts zero there, and an unresolved occurrence contributes nothing.

| Type | Fields |
|---|---|
| `ToolingDemand` | `products: ToolingProductDemand[]`, ordered by `product_id`. |
| `ToolingProductDemand` | `product_id`, `product_revision_id: Uuid | null`, `lifetime_demand: u64 | null` (the baseline product line's summed positive forecast), `forecasts: AnnualForecast[]` (the same stored positive forecasts), `part_quantities: ToolingPartQuantity[]` (ordered by part version id). |
| `ToolingPartQuantity` | `part_version_id`, `quantity: u64` (the expanded non-suppressed count in one finished product; at least 1). |

For one tool `t` owned by part version `c` and one quoted product `p`, `q[t,p]` is the expanded
quantity of `c` in one `p` (zero when absent). A product-root tool of `p` uses `q[t,p] = 1` for its
own product and zero for every other product.

```text
product_lifetime[p] = lifetime_demand[p]
owner_lifetime[t]   = Σ over quoted p of product_lifetime[p] × q[t,p]        (part-version owner)
owner_lifetime[t]   = product_lifetime[p]                                    (product owner p)
owner_annual[t,y]   = Σ over quoted p with a forecast for y of forecast[p,y] × q[t,p]
```

The product owner's `q` is one for its own product. `owner_annual` is only produced when
`owner_lifetime` is available; a product whose baseline `lifetime_demand` is `null` makes every
owner that demands it unavailable (`missing_product_demand`), never zero.

## Investment

```text
source_investment[t] = tool_count[t] × cost_per_tool[t]              # entry currency
investment[t]        = source_investment[t] × rate(t.currency)       # project currency
project_investment   = Σ over entries of investment[t]
```

`rate` is the saved manual rate expressed as project-currency units per one source-currency unit;
same-currency conversion is the identity and needs no rate. The multiplication is checked decimal
arithmetic with no intermediate rounding. An entry whose currency differs and has no saved rate
leaves its investment `null` with `missing_exchange_rate`/`invalid_exchange_rate`; the project
total is then unavailable, never a partial sum.

The investment depends only on the project tooling settings. It is identical under the separate and
the amortized treatment and is always reported once.

## Allocation by lifetime product demand

For every tool and quoted product, with `D[t,p] = product_lifetime[p] × q[t,p]`:

```text
combined_demand[t]      = Σ over quoted p of D[t,p]
allocation_share[t,p]   = D[t,p] / combined_demand[t]                       # combined demand > 0
product_allocated_investment[p] = Σ over tools of allocation_share[t,p] × investment[t]
```

- Allocation uses **lifetime** demand shares, not one product's forecast, not the representative
  costing volume and not the scenario recovery quantity.
- A missing product lifetime demand makes `D` and `combined_demand` unavailable
  (`missing_product_demand`). A combined demand of zero (the owner is demanded by no quoted product,
  for example because it sits only inside a purchased branch) leaves the shares unavailable with
  `zero_combined_demand`; no percentage is invented.
- Allocation explains each product's share of the single project investment. It never multiplies the
  project investment and never replaces the selected recovery quantity in the contribution below.

## Recovery

The evaluation treatment is the scenario's explicit choice, otherwise the project's
`default_treatment`. When neither exists the evaluation is incomplete with `missing_treatment`.

The recovery basis is selected per tool in the scenario assumptions; a tool without an explicit
choice uses the owner's combined lifetime demand:

| Basis | `recovery_quantity[t]` | Requirement |
|---|---|---|
| `lifetime_demand` (default) | `combined_demand[t]` | Available and strictly positive; otherwise `zero_combined_demand`/`missing_product_demand`. |
| `manual_quantity` | The entered quantity | Entered and strictly positive; a missing entry is `missing_recovery_quantity`, a non-positive one `invalid_recovery_quantity`. |

### Amortized treatment

```text
contribution_per_owner_unit[t] = investment[t] / recovery_quantity[t]
product_contribution[t,p]      = contribution_per_owner_unit[t] × q[t,p]
product_amortized_contribution[p] = Σ over tools of product_contribution[t,p]

recovered[t,y]        = contribution_per_owner_unit[t] × owner_annual[t,y]
projected_recovery[t] = contribution_per_owner_unit[t] × owner_lifetime[t]
under_over_recovery[t] = projected_recovery[t] − investment[t]
```

- The initial component contribution is the tool's whole investment divided by the selected
  positive recovery quantity; this is the formula the requirement source labels a Proposal, locked
  here with the confirmed acceptance example: a 10,000 investment with recovery quantity 5,000
  contributes 2 per owner unit, and 2 per finished product when the owner occurs once.
- A finished product's amortized contribution expands the owner contribution with the applicable
  part quantity: an assembly occurring twice per finished product contributes twice its owner-unit
  contribution. A product-root tool contributes its owner-unit contribution once per finished
  product.
- `projected_recovery` uses the entered forecasts: the owner's combined lifetime demand is the
  projected number of owner units over the production period and `recovered[t,y]` is the same
  projection per calendar year. The contribution stays at its initial value in every production
  year; reaching the recovery quantity never stops, caps or reduces it.
- `under_over_recovery` is positive when the projection recovers more than the investment
  (over-recovery) and negative when it recovers less (under-recovery). The under/over state never
  changes the quoted contribution; a revised contribution needs a revised quotation scenario
  (SB-18).
- `projected_recovery` is `investment × owner_lifetime / recovery_quantity`. When the basis is
  lifetime demand the projected recovery follows the investment, and on every approved fixture it
  equals the investment exactly; a manual basis shows the projected under/over. A division that
  does not terminate in `rust_decimal`'s last representable digit is rounded by the shared precision
  rule above, so at magnitudes whose quotient is unrepresentable the projected recovery can differ
  from the investment by a last-digit rounding residual, which the under/over recovery then
  reports. That is the documented precision rule, not an invented value.
- An owner contribution that cannot be divided exactly is rounded only at `rust_decimal`'s last
  representable digit, as documented above.

The recovery quantity is resolved for every tool regardless of the selected treatment, because it
is a stored scenario assumption. A manual basis with no entered quantity is
`missing_recovery_quantity` even under the separate treatment, and the evaluation is incomplete: the
stored assumption would make a later amortized view unusable. Under the separate treatment the
contribution, projection and annual series stay `null` and are not themselves missing values.

### Separate treatment

```text
separate_charge[t] = investment[t]
```

- Under the separate treatment the full investment is one separate charge; it is outside the unit
  price and the amortized contribution. `contribution_per_owner_unit`, `product_contribution`,
  `projected_recovery`, `under_over_recovery` and the annual recovery series are `null` because
  they do not apply to this treatment, not because an input is missing. They are never treated as
  an incompleteness.
- The allocation still reports each product's share of the separate charge, and the project
  separate charge is the project investment counted once. A missing projected recovery under the
  separate treatment is not an incompleteness: the treatment's applicable amounts are its
  investment, allocation and separate charge.
- Switching the treatment preserves the investment exactly and changes only which recovery channel
  is populated. One evaluation can never report both a full separate charge and an amortized
  contribution, so the same investment is never recovered twice.

## Outputs

`ToolingOutcome` carries the whole answer, ordered by `tool_id` then `product_id` then ascending
year:

| Value | Meaning |
|---|---|
| `state` | `complete` only when the treatment is known, every applicable amount is available and no issue was raised. A project with no tool entries is a complete zero-investment answer even when no treatment is chosen, because no recovery channel applies. |
| `treatment` | The selected treatment, or `null` when unknown. |
| `tools` | One `ToolingToolLine` per entry: owner, investment and its conversion, recovery quantity, contribution per owner unit, per-product allocation lines (one per quoted product with a positive expanded owner quantity; a product that does not use the owner is omitted because its share is zero), the annual recovery series (amortized only), projected recovery, under/over recovery and the separate charge (separate only), with explanation lines. |
| `products` | One `ToolingProductLine` per quoted product: lifetime demand, allocated investment (the share of the project investment, read as its share of the separate charge under that treatment), amortized contribution per finished product (amortized only) and explanation lines. |
| `project_investment` | `Σ investment[t]` counted once; `"0"` when the project has no tool entries, and each product line then reports an allocated investment of `"0"` (and an amortized contribution of `"0"` in the amortized treatment). |
| `project_amortized_recovered` | `Σ projected_recovery[t]` (amortized only). |
| `project_under_over_recovery` | `Σ under_over_recovery[t]` (amortized only). |
| `project_separate_charge` | `Σ separate_charge[t]` (separate only). |
| `annual_recovery` | The summed `recovered[t,y]` per calendar year (amortized only, ascending), present only when every tool line's annual series is available; empty under the separate treatment. |
| `explanations` | Project-level `CostLine` lines naming the investment total, the selected treatment/total recovery or the separate charge. |
| `issues` | The named `ToolingIssue`s, ordered by `(code, field_path)`. |

Every business decimal is normalised (trailing zeros stripped) at the output boundary so repeated
runs on identical inputs are byte-identical. A project with no tooling settings and no entries is a
complete zero-investment answer (`project_investment = "0"`), not a missing value.

## Incomplete results and issues

A missing or unusable input makes its affected amount `null` and the result `incomplete`. An issue
is computed on read and never stored as an inferred value.

| Code | Field | Meaning |
|---|---|---|
| `missing_treatment` | `recovery.treatment` | Neither the scenario nor the project selects a treatment. |
| `missing_product_demand` | `demand.products[i].lifetime_demand` | A quoted product that demands the owner has no baseline lifetime demand; allocation and projection are unavailable. |
| `zero_combined_demand` | `tools[i].allocation` | The owner is demanded by no quoted product; no allocation percentage or lifetime recovery quantity is invented. |
| `missing_recovery_quantity` | `recovery.recovery[j].basis.quantity` | The manual basis is selected but no quantity is entered. |
| `invalid_recovery_quantity` | `recovery.recovery[j].basis.quantity` | The manual quantity is not strictly positive. |
| `missing_exchange_rate` / `invalid_exchange_rate` | `exchange_rates` | A tool currency differs from the project currency with no saved rate, or a non-positive one. |
| `invalid_tool_count` / `invalid_cost_per_tool` | `tools[i].tool_count` / `tools[i].cost_per_tool` | A hostile value that the input validation would refuse; the calculator reports it instead of panicking. |
| `arithmetic_overflow` | the affected line | The checked decimal operation exceeded the supported range. |

A malformed structure (empty ids, a duplicate tool id, a duplicate product, an empty part version
in the demand, a non-positive expanded quantity, a non-positive manual recovery quantity) is
refused by `ToolingInputs::validate` before a calculation; it never reaches the calculator as a
stored result. A recovery choice naming a tool the evaluated settings no longer hold is ignored.

## Service routes

Base path `/v2`; a bearer identity with the Engineer permission set from the version 2 permissions
contract, bounded requests exactly as version 1. Errors use the version 1 `ApiErrorBody` shape and
codes. A stale `expected_version` is `409 conflict`; a malformed or refused payload is `422 invalid`;
an unknown project, baseline calculation or record version is `404 not_found`; a missing permission
is `403`.

| Method | Path | Request | Success | Body |
|---|---|---|---|---|
| `PUT` | `/v2/projects/{project_id}/tooling` | `UpdateProjectTooling { request_id, expected_version, tooling }` | `200` | `ProjectWriteOutcome` |
| `POST` | `/v2/projects/{project_id}/tooling-recovery` | `EvaluateToolingRecovery { baseline_calculation_id, recovery }` | `200` | `ToolingOutcome` |

The tooling settings write is an ordinary append-only project record write: the same `request_id`
with an identical payload returns the recording outcome before the version check, a changed payload
under the same id is a conflict, and the tooling value is validated against the project currency,
part versions and products by `ProjectDocument::validate`.

The recovery route is a **read-only calculation**: it stores no row and appends no project version,
so it takes no `request_id` and an identical request simply returns the same deterministic
`ToolingOutcome`. It reads the baseline snapshot named by `baseline_calculation_id`, uses the
tooling settings of the project record version that snapshot recorded, resolves `ToolingInputs` and
runs the pure calculator. A baseline snapshot whose project version no longer exists is `404`. It
does not create, activate or price a saved scenario; SB-18 owns scenario persistence and will call
the same portable calculation with the scenario's recovery assumptions.

## Persistence

No new table and no new migration. The tooling settings are the additive `tooling` field of the
existing append-only project document, so the version 2 project record storage already keeps every
version. The recovery evaluation stores nothing. The SB-16 `baseline_calculations` table and every
other table are unchanged.

## Validation the input refuses

`ProjectTooling::validate(project_currency, part_version_ids, product_ids)`:
at most 10,000 entries; a non-nil unique `tool_id`; a valid owner that names a part version or a
product of the same project; at most 200 characters for a present `name`/`source_reference`;
`tool_count ≥ 1`; a non-negative `cost_per_tool`; a three-letter upper-case `currency`.

`ToolingInputs::validate()`: a three-letter upper-case `project_currency`; the existing exchange
rate rules; every entry validated structurally; at most 1,000 demand products and 10,000 part
quantities per product; a non-nil unique `product_id` with the products in ascending id order; a
non-nil optional revision id; a non-positive product `lifetime_demand` is refused; a unique part
version per product, with quantity at least 1 and ascending part version order; at most 10,000
recovery choices, each naming a non-nil `tool_id` once, and a manual quantity that, when present,
is strictly positive. The ordering rule keeps an issue's `demand.products[i]` path unambiguous:
the calculator canonicalises the order internally, so a non-conforming caller is refused instead
of receiving a path that names a different element of its own input.

The calculator still re-checks business values (a non-positive manual quantity, a negative cost)
and reports the named issue instead of panicking, so a hostile stored record can never crash the
service.

## Documented limitations and decisions

These decisions preserve a confirmed rule where the sources are silent or where a neighbouring
contract cannot express the requested value. Each is a deliberate choice, not a silent
reinterpretation.

- **Locked contribution formula.** The source marks the component contribution
  (`assigned recovery amount / selected positive recovery quantity`) as a Proposal. This contract
  locks it for the MVP as the tool's whole investment divided by the selected positive recovery
  quantity, because the acceptance example 10,000 / 5,000 = 2 and the no-double-recovery rule fix
  the arithmetic unambiguously. An "assigned recovery amount" smaller than the investment would
  contradict the confirmed "investment is counted once" and "do not recover the full investment
  twice" combination; the allocation share stays explanatory and never divides the contribution.
- **Treatment is scenario-wide.** The project holds one default treatment and one scenario selects
  one treatment for all of its tools. Per-tool treatment mixing is not confirmed and is not
  modelled. The recovery **basis** may be chosen per tool because lifetime and manual recovery for
  assembly tools are explicitly selectable.
- **Product-root tool ownership.** A tool owned by a product uses that product's lifetime demand as
  its combined demand, so its allocation share is 100% for that product. A product-root tool cannot
  be shared by several products; a shared assembly tool is owned by the assembly's part version and
  allocated by the expanded assembly demand instead.
- **Purchased branches.** The baseline's expanded quantities already stop at a purchased branch, so
  a descendant tool has no demand and reports `zero_combined_demand`. A tool owned by the purchased
  part itself is evaluated against that part's own non-suppressed demand, matching the manufactured
  cost rule that a purchased assembly's own operations still apply while its descendants do not.
- **One owner per entry.** Multi-part/family-tool investment splitting and one entry spanning
  several definitions are excluded; the entry names exactly one owner and must not be duplicated to
  simulate a split.
- **No tooling snapshot.** This contract deliberately stores no immutable recovery result; the
  result is a deterministic function of an immutable baseline snapshot, the immutable project record
  version it names and the scenario assumptions. SB-18 persists the scenario, and SB-19/SB-20 read
  the same values without negotiating a selling price.
- **No desktop surface in this increment.** The typed engine commands, controller state and tooling
  editing/explanation UI are a later increment of this package; they depend on the still-open
  SB-10/SB-15/SB-16 desktop wiring. This contract and its service surface come first.

## Compatibility

- Version 1, version 2, the SB-11 engineering routes, the SB-13 recognition routes, the SB-14
  feasibility routes, the SB-15 manufacturing routes and the SB-16 costing routes are unchanged;
  this contract is additive.
- Adding the optional `tooling` field to the stored project document is compatible: a document
  written earlier has no field, reads as `null`, validates under schema version 1 and never gains
  an implicit treatment. A build before this contract tolerates the unknown field.
- Adding the two routes is compatible. Changing a field meaning, a wire name, a unit, a route, a
  status, an error code, the idempotency rule or a validation rule that previously accepted a write
  is breaking and needs a new contract version, the affected-consumer list and coordinator approval.
- Historical project record versions stay readable under the schema version that wrote them.

## Handoff to later packages

- **SB-18 (commercial scenarios):** consumes `ToolingRecoveryInputs`, the pure recovery functions
  and the `ToolingOutcome` values (initial contribution, product allocation, projected under/over
  recovery). It owns scenario persistence, the manual/suggested base price, the EBIT target and the
  productivity treatment. It must not duplicate the investment arithmetic or mutate a baseline.
  Productivity is applied by SB-18 on the initial contribution; this contract reports the
  un-productivity-reduced contribution and its forecast recovery.
- **SB-19 (activation):** treats an incomplete tooling evaluation (a missing treatment, a missing
  product demand, a zero combined demand or a missing recovery quantity) as a readiness input. A
  negotiated selling price and a completed scenario are never prerequisites here.
- **SB-20 (dependency updates):** recalculates a draft tooling evaluation when forecasts, scope,
  deliveries or tooling definitions change; it flags saved scenarios and appends new project record
  versions rather than rewriting history.
- **Desktop increment:** the typed engine commands, controller state and tooling editing/explanation
  surface are a later increment of this package.
