# Commercial scenarios contract

Version: **1** of this contract, additive to the published version 2 HTTP API · Status: **published
by SB-18** (owner instruction 2026-11-11: start implementation on issue #33; the locking commit is
recorded in [SB-18's execution record](../../execution/SB-18.md)).

This contract extends version 2 additively. It adds the project's commercial settings, the named
commercial scenarios as their own append-only version chain, and the pure per-product/per-year
price, EBIT and revenue answer that compares the scenarios against one immutable SB-16 baseline
snapshot. It consumes the SB-17 tooling recovery answer and never repeats the investment,
allocation or recovery arithmetic. The version 1 routes, the version 2 project, engineering,
manufacturing, feasibility and costing routes, and the SB-17 tooling routes and their meanings stay
exactly as published. It does not change the stored project document's existing fields, the stored
engineering/manufacturing/feasibility documents, the stored baseline snapshots or the tooling
settings.

Portable types: `crates/solidbom-contract/src/commercial.rs` (re-exported at the crate root).
Producers: the service's commercial adapter (`services/solidbom-server/src/commercial.rs`) and the
routes below; the pure calculator (`crates/solidbom-costing/src/commercial.rs`, version
`solidbom-commercial-scenarios/1`). Consumers: SB-19 (readiness), SB-20 (recalculation and stale
scenario flagging), the internal exports package and the desktop comparison surface (later
increment of this package).

Acceptance: the contract crate's `commercial` tests, the `solidbom-costing` crate's commercial
arithmetic tests, the service's `tests/commercial.rs` domain tests and the commercial route tests in
`tests/api.rs`.

## Rules that apply to every value

The version 1 and version 2 rules still apply: business amounts, rates, prices and quantities are
decimal strings; a missing value is `null` and an explicit zero is `"0"`, never interchangeable;
unknown JSON fields are tolerated; a stored schema version this build does not read is refused,
never coerced.

Additional rules for this contract:

- The calculator is pure and deterministic: the same `CommercialInputs` always produce
  byte-identical `CommercialOutcome` JSON. It performs no catalog lookup, no wall-clock read and no
  database access. It uses checked decimal arithmetic; an unrepresentable operation becomes a named
  `arithmetic_overflow` issue and an unavailable amount, never a panic and never an invented value.
- No intermediate display rounding. Decimal division that does not terminate is rounded by
  `rust_decimal` at its last representable digit. Every approved fixture divides exactly.
- A missing input leaves its affected amounts `null` and produces a named issue; it never becomes
  zero. An explicit zero is a real value: a zero productivity rate, a zero EBIT target and a zero
  recurring cost are real values.
- A **blank manual price** is the deliberate absence of a quoted price. It is not zero revenue, not
  a valid manual-price EBIT and not an error. The suggested price stays visible beside it, and the
  scenario remains a valid suggestion-only view.
- The annual productivity reduction applies to selling prices only. The recurring manufacturing
  cost `C` the baseline resolved stays fixed across the production years; it is never reduced by the
  selling-price productivity. No manufacturing cost-volume curve is introduced.
- Tooling recovery revenue and the separate setup charge stay outside the recurring EBIT. The
  separate tooling charge stays outside the unit price; the amortized contribution is added to the
  unit price.
- One evaluation selects one tooling treatment, so the same investment is never recovered through
  both a full separate charge and full amortization. The calculator consumes the SB-17 answer and
  never recomputes the investment.
- A scenario cannot express an EBIT target or a costing volume policy, so it cannot override the
  project's values. A comparison evaluates every scenario against one named immutable baseline and
  labels a scenario saved against another baseline, project version or settings as stale.

## Identities and settings ownership

| Level | Settings | Owner |
|---|---|---|
| Project | EBIT target rate, annual selling-price productivity rate | Additive `commercial` field of the stored project document; versioned by the append-only project record |
| Project | Costing volume policy, default tooling treatment | Already owned by SB-16 and SB-17; unchanged by this contract |
| Scenario | Name, manual base unit prices, tooling recovery assumptions, whether productivity applies to tooling | One append-only scenario version chain under the project |
| Baseline | The immutable SB-16 baseline snapshot a scenario version is saved against | The SB-16 contract; unchanged by this contract |

| Value | Meaning | Stability |
|---|---|---|
| `project_id` | The project whose commercial settings and scenarios are used. | Business identity from the version 2 project contract. |
| `scenario_id` | One named commercial alternative. | Business identity, stable across its saved versions. |
| `scenario version` | One immutable saved scenario; an edit appends `version + 1`. | Append-only integer starting at 1. |
| `baseline_calculation_id` | One immutable SB-16 baseline snapshot; the demand, recurring cost, forecasts, volume policy and setup charge the comparison reads. | Service-generated UUID. |
| `project_version` | The project record version the pinned baseline snapshot recorded; it supplies the EBIT target, productivity rate and tooling settings. | Append-only integer. |

## Project commercial settings

The project document gains one additive optional field, `commercial: ProjectCommercial | null`
(`#[serde(default)]`, schema version stays 1; a document written before this contract reads as
`null`, never as an implicit EBIT target). `ProjectCommercial`:

| Field | Meaning | Validation |
|---|---|---|
| `ebit_target_rate` | The project EBIT target `t`: a margin on the base selling price. | A fraction, `0 ≤ t < 1`. |
| `annual_productivity_rate` | The annual selling-price productivity reduction. | A fraction, `0 ≤ rate < 1`; 100% is refused because it would zero every later price. |

Replacing the settings appends one project record version through the ordinary project write path;
it never rewrites an earlier version and never changes an earlier baseline snapshot. A comparison
reads the settings of the exact project version its named baseline snapshot recorded.

## Named scenarios

A scenario is its own append-only version chain beside the project record. One scenario version
carries:

| Field | Meaning | Validation |
|---|---|---|
| `scenario_id`, `project_id` | Stable identities. | Non-nil UUIDs. |
| `version` | Append-only version, starting at 1. | At least 1. |
| `name` | The scenario's explanation label. | Non-blank, at most 200 characters. |
| `baseline_calculation_id` | The immutable baseline snapshot this version was saved against. | A baseline of the same project. |
| `project_version` | The project record version that baseline recorded, retained for the stale labels. | At least 1. |
| `manual_prices` | Entered base unit selling prices, each excluding tooling. A product without an entry has a blank price. | At most 1,000; unique product id in ascending order; each price strictly positive. |
| `tooling` | The scenario's `ToolingRecoveryInputs` from the SB-17 contract: a treatment override and per-tool recovery bases. | The SB-17 validation. |
| `productivity_applies_to_tooling` | Whether the annual reduction also applies to the amortized contribution. | Boolean. |
| `created_at` | The service timestamp of that saved version. | Recorded by the service. |

An accepted create writes version 1; an accepted edit checks the expected version under a lock and
appends exactly one new version in the same transaction as its request receipt. No code path
updates or deletes a saved scenario version. A later baseline is never adopted silently: the
scenario keeps its pinned baseline until a revised scenario version names a new one.

## Requests and routes

Base path `/v2`; a bearer identity with the Engineer permission set from the version 2 permissions
contract, bounded requests exactly as version 1. Errors use the version 1 `ApiErrorBody` shape and
codes. A stale `expected_version` is `409 conflict`; a malformed or refused payload is `422 invalid`;
an unknown project, scenario or baseline calculation is `404 not_found`; a missing permission is
`403`.

| Method | Path | Request | Success | Body |
|---|---|---|---|---|
| `PUT` | `/v2/projects/{project_id}/commercial` | `UpdateProjectCommercial` | `200` | `ProjectWriteOutcome` |
| `PUT` | `/v2/projects/{project_id}/commercial-scenarios` | `CreateCommercialScenario` | `200` | `CommercialScenarioWriteOutcome` |
| `PUT` | `/v2/projects/{project_id}/commercial-scenarios/{scenario_id}` | `UpdateCommercialScenario` | `200` | `CommercialScenarioWriteOutcome` |
| `GET` | `/v2/projects/{project_id}/commercial-scenarios` | — | `200` | `CommercialScenarioList` |
| `POST` | `/v2/projects/{project_id}/commercial-comparisons` | `CompareCommercialScenarios` | `200` | `CommercialScenarioComparison` |

The settings write, the create and the edit are ordinary append-only writes: the same `request_id`
with an identical payload returns the recorded outcome before the version check, and a changed
payload under the same id is a conflict. The create request carries a client-generated `scenario_id`; an existing id is a
conflict. The edit request's `expected_version` must be the current version of that scenario for the
project.

The comparison route is a **read-only calculation**: it stores no row and appends no scenario or
project version, so it takes no `request_id` and an identical request returns the same deterministic
answer. It reads the named immutable baseline snapshot, the project record version that snapshot
recorded and the current version of every named scenario.

## The calculator input

The service resolves `CommercialInputs` from three immutable sources: the compared baseline
snapshot, the project record version it recorded, and the saved scenario version.

| Input field | Source |
|---|---|
| `project_currency` | The exact project record version. |
| `ebit_target_rate`, `annual_productivity_rate` | That project version's `commercial` settings; `null` when the document has none. |
| `productivity_applies_to_tooling` | The saved scenario version. |
| `manual_prices` | The saved scenario version. |
| `tooling` | The SB-17 `ToolingOutcome` for the same baseline snapshot with the scenario's `ToolingRecoveryInputs`; the calculator consumes it and never recomputes it. |
| `products[].product_revision_id`, `products[].recurring_unit_cost` | The baseline snapshot's product lines. |
| `products[].schedule`, `products[].forecasts` | The same product inputs the baseline snapshot recorded; the product is priced from its own SOP to its own EOP. |
| `setup_total` | The baseline snapshot's separate setup total. |

For one quoted product `p` and production year `n` counting from that product's first production
year (`n = 1` at SOP):

```text
C[p]                     = recurring unit cost of one finished product
                           (material + production + attached + configured EBIT categories;
                           tooling and setup excluded)
suggested_first_year[p]  = C[p] / (1 − t)                               # t is the project EBIT target
factor[p,n]              = (1 − r) ^ (n − 1)                            # r is the project productivity rate
suggested_base[p,n]      = suggested_first_year[p] × factor[p,n]
manual_base[p,n]         = manual_price[p] × factor[p,n]                # blank manual price: null
```

## Recurring EBIT and quoted revenue

The recurring EBIT is the quoted-price profitability. It excludes the tooling recovery revenue, the
tooling investment and the separate setup charge, exactly as the requirements confirm:

```text
manual_unit_ebit_amount[p,n] = manual_base[p,n] − C[p]
manual_unit_ebit_rate[p,n]   = (manual_base[p,n] − C[p]) / manual_base[p,n]
revenue[p,n]                 = manual_base[p,n] × forecast[p,n]
ebit_amount[p,n]             = (manual_base[p,n] − C[p]) × forecast[p,n]

suggested_unit_ebit_amount[p,n] = suggested_base[p,n] − C[p]
suggested_unit_ebit_rate[p,n]   = (suggested_base[p,n] − C[p]) / suggested_base[p,n]
```

- `C[p]` stays fixed in every production year: the productivity reduction lowers the price and the
  EBIT, never the assumed manufacturing cost.
- The suggested first-year base price achieves the project EBIT target in the product's first
  production year. There is no lifetime-target solver and no customer quotation.
- A blank manual price leaves `manual_base`, the manual EBIT and `revenue`/`ebit_amount` `null`.
  This is a valid state, not an issue: the suggested price and its EBIT stay visible beside it.
- An EBIT rate is unavailable when its base price is zero; a zero price has no rate. An EBIT amount
  may legitimately be negative (a price below the recurring cost).
- The revenue basis is the base unit selling price; tooling recovery revenue is reported separately
  and never enters the recurring EBIT.

## Tooling in the commercial view

The calculator consumes the SB-17 `ToolingOutcome` of the same baseline:

- **Amortized treatment:** the SB-17 answer gives each product's initial amortized contribution per
  finished product. That contribution is added to the unit price:
  `tooling_contribution[p,n] = contribution[p] × factor[p,n]` when the scenario's productivity
  applies to tooling, otherwise `contribution[p]` in every year. The total customer unit price is
  `base price + contribution`, and the tooling recovery revenue is
  `contribution[p,n] × forecast[p,n]`, reported separately from the recurring revenue. Reaching the
  recovery quantity never stops, caps or reduces the quoted contribution; the projected under/over
  recovery is the SB-17 answer.
- **Separate treatment:** the unit price carries no contribution and the whole investment stays one
  separate charge outside the unit price. The product's allocated investment is still reported as
  the explanation of its share. The tooling contribution, its annual series and the tooling recovery
  revenue are `null` because they do not apply to this treatment, not because an input is missing.
- An incomplete SB-17 answer makes the commercial answer incomplete with a `tooling_incomplete`
  issue; the calculator never invents a contribution the SB-17 answer could not compute.

The separate setup charge (`setup_total`, from the baseline) is carried through unchanged as a
separate charge outside the recurring EBIT and outside the unit price.

## Project totals

- `project.years` is the ascending union of every quoted product's schedule years. For each year,
  `revenue`, `ebit_amount` and `tooling_recovery_revenue` sum the product lines that produce in that
  year. A product outside its schedule contributes nothing; a product inside its schedule whose
  value is unavailable makes that yearly total `null`. No partial total is ever reported.
- `lifetime_revenue` and `lifetime_ebit_amount` sum the yearly totals; an unavailable year makes the
  lifetime total `null`.
- A blank manual price makes the quoted yearly and lifetime revenue/EBIT totals `null` without an
  issue, because no quoted price was entered. This is different from a missing forecast, which is a
  named issue.
- `tooling_investment`, `tooling_separate_charge`, `tooling_projected_recovery` and
  `tooling_under_over_recovery` are copied from the SB-17 answer, counted once.
- `setup_total` is reported once at the project level; it is never spread over production to create
  a combined EBIT.

## Outputs

`CommercialOutcome` carries the whole answer, ordered by product id and ascending year:

| Value | Meaning |
|---|---|
| `state` | `complete` only when no issue was raised. |
| `ebit_target_rate`, `annual_productivity_rate`, `productivity_applies_to_tooling` | The settings the answer used. |
| `products` | One `CommercialProductLine` per quoted product: the manual price, the suggested first-year price, the first-year manual EBIT amount/rate, the initial tooling contribution, the allocated investment and one `CommercialYearLine` per production year. |
| `project` | The yearly and lifetime quoted revenue/EBIT, the SB-17 tooling totals and the separate setup charge. |
| `tooling` | The SB-17 `ToolingOutcome` the answer consumed, unchanged. |
| `setup_total` | The baseline's separate setup charge. |
| `explanations` | Project-level `CostLine` lines naming the investment, the separate charge, the setup charge, the lifetime revenue and the lifetime EBIT. |
| `issues` | The named `CommercialIssue`s, ordered by `(code, field_path)`. |

`CommercialYearLine` carries `year`, `forecast`, `recurring_unit_cost`, `suggested_base_price`,
`manual_base_price`, `tooling_contribution`, `suggested_total_price`, `manual_total_price`,
`suggested_unit_ebit_amount`/`rate`, `manual_unit_ebit_amount`/`rate`, `revenue`, `ebit_amount` and
`tooling_recovery_revenue`.

Every business decimal is normalised (trailing zeros stripped) at the output boundary so repeated
runs on identical inputs are byte-identical.

## Comparisons and stale labels

`CommercialScenarioComparison` names the common baseline, the project version it recorded, the
shared volume policy, EBIT target and productivity rate, and one line per compared scenario. Every
scenario is evaluated against the common baseline, so the comparison never presents results from
different baselines as equivalent without labelling:

| Code | Meaning |
|---|---|
| `scenario_baseline_differs` | The saved scenario pinned another baseline; it was evaluated against the comparison's baseline. |
| `scenario_project_version_differs` | The saved baseline's project version differs from the comparison's. |
| `scenario_settings_differ` | The saved baseline's project version has different commercial settings (EBIT target or productivity) from the comparison's. |
| `scenario_baseline_outdated` | A newer baseline snapshot for the project exists than the one the scenario pinned. |
| `scenario_project_outdated` | The project record version the scenario pinned is older than the current project version. |

`stale` is true exactly when at least one reason is present. The requested scenario order is
ascending by id; every named scenario must be the current version of a scenario of that project or
the request is `404`.

## Incomplete results and issues

A missing or unusable input makes its affected amounts `null` and the result `incomplete`. An issue
is computed on read and never stored as an inferred value.

| Code | Field | Meaning |
|---|---|---|
| `missing_ebit_target` / `invalid_ebit_target` | `ebit_target_rate` | The project has no EBIT target, or a rate outside `[0, 1)`. |
| `missing_productivity` / `invalid_productivity` | `annual_productivity_rate` | The project has no productivity rate, or a rate outside `[0, 1)`. |
| `missing_recurring_unit_cost` / `invalid_recurring_unit_cost` | `products[i].recurring_unit_cost` | The baseline has no recurring unit cost for a quoted product, or a negative one. |
| `missing_forecast` / `invalid_forecast` | `products[i].years[j].forecast` | The baseline has no forecast for a schedule year, or a zero one. |
| `invalid_manual_price` | `manual_prices[i].price` | A stored manual price that is not strictly positive; the calculator treats it as blank and names it. |
| `tooling_incomplete` | `tooling` | The SB-17 answer is incomplete; the contribution it could not compute is not invented. |
| `missing_setup_charge` | `setup_total` | The baseline has no separate setup total. |
| `arithmetic_overflow` | the affected line | The checked decimal operation exceeded the supported range. |

A blank manual price is **not** an issue. A malformed structure (two products, two manual prices for
one product, unordered manual prices, a manual price for a product the calculation does not price, a
nil baseline id) is refused by `CommercialInputs::validate` or `CommercialScenarioContent::validate`
before a calculation; it never reaches the calculator as a stored result.

## Persistence

One new table and one new migration, `0010_commercial_scenarios.sql`:

- `commercial_scenarios (scenario_id, project_id, version, name, baseline_calculation_id,
  project_version, manual_prices, tooling, productivity_applies_to_tooling, created_at)` with the
  primary key `(scenario_id, version)`; every write inserts one row and no code path updates or
  deletes one. The idempotent request receipt lives in the existing shared `requests` table.
- A read of the current scenario list selects the greatest version per scenario id; a historical
  version is never rewritten.
- The project commercial settings are the additive `commercial` field of the existing append-only
  project document; the SB-16 `baseline_calculations` table and every other table are unchanged.

## Validation the input refuses

`ProjectCommercial::validate`: both rates are fractions in `[0, 1)`.

`UpdateProjectCommercial::validate`: a bounded request id, an expected project version of at least 1
and valid settings.

`CommercialScenarioContent::validate`: a non-blank name of at most 200 characters; a non-nil
baseline id; at most 1,000 manual prices, each naming a non-nil product id once in ascending order
with a strictly positive price; the SB-17 `ToolingRecoveryInputs::validate`.

`CommercialScenario::validate`: the content rules plus non-nil identities and an append-only version
of at least 1.

`CreateCommercialScenario::validate`, `UpdateCommercialScenario::validate` and
`CompareCommercialScenarios::validate`: a bounded request id (writes), a non-nil scenario id, an
expected version of at least 1 (edits), a non-nil baseline id and 1–50 unique scenario ids in
ascending order (comparisons).

`CommercialInputs::validate`: a three-letter upper-case project currency; both optional rates
validated when present; a non-negative setup total; at most 1,000 products ordered by ascending id
with a valid schedule, unique forecast years inside the schedule and a non-negative recurring cost;
the manual-price rules above, with every priced product present.

The calculator still re-checks the business values a stored record could hold (a rate outside
`[0, 1)`, a non-positive manual price, a negative recurring cost, a zero forecast) and reports the
named issue instead of panicking, so a hostile stored record can never crash the service.

## Acceptance vectors

These are the exact vectors asserted by the contract and calculator tests; the service tests assert
the same arithmetic over a real baseline's recurring unit cost, forecasts and setup charge.

1. **Productivity and fixed cost.** `C = 80`, project EBIT target `0.2`, productivity `0.03` gives a
   suggested first-year base price `80 / 0.8 = 100`, then `97`, then `94.09`. The recurring cost
   stays `80` in every year.
2. **Tooling productivity.** An entered base price `100` with an SB-17 amortized contribution `2`
   gives a first-year total `102`. Year two is `99` when the contribution is exempt from
   productivity (`97 + 2`) and `98.94` when it applies to both (`97 + 2 × 0.97 = 97 + 1.94`).
3. **Manual price and EBIT.** A blank manual price has no manual-price EBIT amount or rate, no quoted
   revenue and no quoted EBIT, while the suggested price stays visible; the scenario remains
   complete. An entered `100` with `C = 80` shows the amount `20` and the rate `0.2` distinctly, and
   the quoted revenue/EBIT are the price/cost difference times the forecast.
4. **Target below 100%.** `C = 80` with target `0.75` gives `80 / 0.25 = 320` as the first-year
   suggested price; a target of `100%` or more is refused.
5. **Each product's own first year.** Two products with SOPs in different calendar years each start
   their compounded reduction at their own first production year; the project year series is their
   union.
6. **Separate tooling.** The separate treatment keeps the unit price at the base price, reports the
   whole investment as one separate charge and leaves the amortized contribution and its series
   `null`; the same investment is never recovered twice.
7. **Scenario integrity.** Two scenarios saved against the same baseline share the project volume
   policy and EBIT target; evaluating one against another baseline labels it with the stale reasons
   above instead of presenting the results as equivalent.
8. **Missing and hostile values.** A missing project rate, recurring cost, forecast or setup total
   is a named issue and an unavailable amount, never zero; an unrepresentable multiplication is an
   `arithmetic_overflow` issue, never a panic; the same inputs serialize byte-identically.

## Documented limitations and decisions

These decisions preserve a confirmed rule where the sources are silent or where a neighbouring
contract cannot express the requested value. Each is a deliberate choice, not a silent
reinterpretation.

- **The first-year suggested price is `C / (1 − t)`.** The requirement fixes this arithmetic and the
  first production year; later years show the productivity effect and never solve a lifetime
  target. `C` is the baseline's `recurring_unit_cost`, which already excludes tooling and setup.
- **Manual prices are first-year base prices.** The entered price is the base unit selling price
  (excluding tooling), so the productivity series starts from it exactly like the suggested series.
  The manual-price EBIT uses that same series; the first-year amount/rate are reported explicitly
  because that is the entered year.
- **A blank manual price is not an error.** The requirement keeps the suggested price visible beside
  an unentered manual price and says a blank manual price has no valid manual-price EBIT; it does
  not make the rest of the scenario incomplete. The quoted project totals are `null` in that case
  because no quoted price exists.
- **The comparison common baseline is required.** A comparison names one immutable snapshot; every
  scenario is evaluated against it. This is the explicit labelling the requirements ask for: a
  scenario saved against another baseline is still shown, marked stale, and never silently mixed.
- **Scenario productivity treatment.** The project owns the productivity rate; the scenario owns
  only whether it applies to the tooling contribution, exactly as the settings-ownership table
  states. Applying it to the contribution compounds the contribution independently of the base
  price.
- **Quoted revenue excludes tooling recovery.** The recurring EBIT's revenue basis is the base unit
  selling price. The tooling recovery revenue is a separate series, and the separate tooling charge
  and setup charge are separate project charges.
- **No customer quotation or lifetime-target solver.** This contract ends at the internal
  comparison; customer-facing quotation generation and a lifetime-volume-weighted target are
  outside the MVP.
- **No desktop surface in this increment.** The typed engine commands, controller state and the
  comparison UI are a later increment of this package; they depend on the still-open SB-10,
  SB-15, SB-16 and SB-17 desktop wiring. This contract and its service surface come first, so no UI
  evidence is claimed here.

## Compatibility

- Version 1, version 2, the SB-11 engineering routes, the SB-13 recognition routes, the SB-14
  feasibility routes, the SB-15 manufacturing routes, the SB-16 costing routes and the SB-17
  tooling routes are unchanged; this contract is additive.
- Adding the optional `commercial` field to the stored project document is compatible: a document
  written earlier has no field, reads as `null`, validates under schema version 1 and never gains an
  implicit EBIT target. A build before this contract tolerates the unknown field.
- Adding one append-only scenario table, one migration and four routes is compatible. Changing a
  field meaning, a wire name, a unit, a route, a status, an error code, the idempotency rule or a
  validation rule that previously accepted a write is breaking and needs a new contract version, the
  affected-consumer list and coordinator approval.
- Historical project record versions, baseline snapshots and scenario versions stay readable under
  the versions that wrote them.

## Handoff to later packages

- **SB-19 (activation and accepted baselines):** reads a saved scenario's state and the comparison's
  stale labels as readiness/dependency inputs; a scenario is never a prerequisite for readiness.
- **SB-20 (dependency updates):** recalculates a draft comparison when forecasts, scope, baselines
  or project settings change, appends new scenario versions rather than rewriting history, and
  surfaces the stale labels.
- **Internal exports (SB-21 scope):** the annual prices, EBIT, revenue and the separate tooling and
  setup charges are the values to export; the result object is already deterministic and stable.
- **Desktop increment:** the typed engine commands, controller state and the comparison UI are a
  later increment of this package.
