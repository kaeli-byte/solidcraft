# SolidBOM requirements and acceptance map

This map allocates the supplied requirements to the approved delivery packages. It is a coverage and ownership record, not evidence that a feature exists or a check has passed. The implementation baseline reviewed is SolidCraft commit `871014f955a76fd308ce156e68c93e6372906005`, version `0.40.0-rc.2`. No SolidBOM application acceptance or customer-file benchmark was executed in that review.

## Authority, status, and package ownership

Read the [application specification](requirements/cost_calculator_handover/APP_SPECIFICATION.md) first, followed by the [approved prototype specification](requirements/cost_calculator_handover/DESKTOP_PROTOTYPE_SPEC.md). Later explicit owner decisions take precedence. Confirmed application requirements take precedence over compatible workbook formulas. Preserving a source document does not approve every proposal inside it.

| Status | Meaning in this map |
|---|---|
| Confirmed | Required behavior from the application specification or approved prototype checks. |
| Proposal | A source recommendation that is not automatically a confirmed product requirement. A package may select a mechanism in its reviewed technical contract without relabeling the source as confirmed. |
| Historical | Earlier analysis, workbook behavior, or recorded observations; useful evidence with a stated scope, not current acceptance. |
| Deferred | Outside the approved prototype or MVP as specified. A prototype deferral does not remove an MVP obligation. |
| Gated extension | Separately tracked work that cannot become ready until its named inputs and policy decisions are resolved. |

The first package named in an owner cell owns the behavior; later packages supply dependencies or integration. SB-08 owns integrated P01–P14 acceptance. SB-22 owns integrated MVP acceptance across all confirmed requirements and A01–A20. A package being named here does not mark it ready, implemented, integrated, or verified. The execution ledger and each package's evidence record determine those states.

| Package group | Responsibilities |
|---|---|
| SB-00 | Requirements preservation, reconciliation, references, and execution records. |
| SB-01–SB-03 | Portable contracts/integration foundations; prototype calculator; central persistence and calculation API. |
| SB-04–SB-08 | CAD mappings/measurements; immutable review sessions; desktop controller; prototype review UI; prototype acceptance. |
| SB-09–SB-11 | Shared data/permissions; projects/deliveries/part versions; engineering review and flat grouping. |
| SB-12–SB-15 | Tube/protection extraction; recognition/catalog suggestions; feasibility; sourcing/manufacturing definitions. |
| SB-16–SB-18 | Shared demand/manufacturing costs; tooling; commercial scenarios. |
| SB-19–SB-22 | Activation; dependency updates/adoption; internal exports; MVP acceptance and operating handoff. |
| SB-X01 | Gated machine hourly-rate construction and sensitivity extension. |

## Confirmed application obligations

Source section numbers below refer to the application specification. Rows group related rules without changing their authority. All rows in this section are **Confirmed**; the separate proposal register preserves unapproved details.

### Delivery, ownership, and project lifecycle

| ID | Obligation and boundary | Source | Owning packages |
|---|---|---|---|
| R-DEL-01 | Deliver SolidBOM review/costing in the SolidCraft desktop fork, linked to its existing CAD tree and viewport. Initial browser delivery is deferred. | §§1, 4, 18 | SB-07; SB-11, SB-18 |
| R-DEL-02 | Keep projects, shared data, original STEP sources, reviewed inputs, source associations, calculations, and saved results central. Authoritative costing runs on the backend. | §§1, 18; D01/D04 | SB-03; SB-06, SB-09 |
| R-DEL-03 | Run prototype import, viewport, selection, and measurement locally; run long analysis in the background. Local geometry/cache is rebuildable from saved sources. | §§1, 18; D01/D04 | SB-04; SB-06 |
| R-DEL-04 | Project writes are online-only. A failed save retains form edits in the current session, reports failure, and supports explicit retry. No silent synchronization or authoritative local costing. Local CAD save is distinct from central project save. | §1; D03/D04 | SB-06; SB-03, SB-07 |
| R-DEL-05 | First prototype is a locally launched macOS development build; signed distribution, updates, and Windows/Linux prototype delivery are deferred. Establish the actual test architecture and OS in evidence. | §1; D02/D06 | SB-08 |
| R-DEL-06 | Reviewed prototype source geometry/hierarchy is immutable across UI, shortcuts, and programmatic paths. Selection, measurement, annotations, and business edits remain available. Geometry changes require a new STEP revision. | §§1, 18; D05 | SB-05; SB-07 |
| R-DEL-07 | One organization per MVP deployment shares catalogs, rules, cost centers, and configuration; no multi-organization administration is implied. | §§1, 13 | SB-09 |
| R-DEL-08 | Engineer handles project work, self-activation, and justified exceptions. Administrator maintains shared data/configuration. One user may hold both permission sets; no separate approval role. | §13 | SB-09; SB-14, SB-19 |
| R-PRJ-01 | Create/edit inquiry business details, customer/internal team, platform/application, products, schedules, and product forecasts; retain incomplete drafts. Products may be simple parts or assemblies. | §§3–5 | SB-10 |
| R-PRJ-02 | Each customer package belongs to one project, preserves original STEP source, and associates delivered product revisions. Product identity persists across deliveries. | §§3, 5–6 | SB-10; SB-03 |
| R-PRJ-03 | Partial deliveries may revise selected products and add new products; omitted products remain unchanged. Keep prior files, engineering work, revisions, and calculation results. | §6 | SB-10 |
| R-PRJ-04 | Carry engineering work forward and highlight affected review work. Preserve successful analysis when another product fails; support retry/manual completion without discarding success. | §§5–6 | SB-11; SB-10, SB-06 |
| R-PRJ-05 | Exclude/reinstate quoted products without deleting engineering history. Exclusion removes their quoted demand, costs, and revenue; reinstatement respects outstanding review requirements. | §6, O13 | SB-10; SB-16, SB-18, SB-20 |
| R-PRJ-06 | Previous product revisions remain the active costing basis until explicit activation. Draft, active, and historical results remain distinguishable. | §§3, 6 | SB-19; SB-10 |
| R-PRJ-07 | Coordinate activation of all affected active products for shared sourcing, material, dimension, or routing changes within the same customer part version. Product-only changes may activate independently; distinct customer revisions may coexist. | §§6, 8, O05/O06 | SB-19; SB-10, SB-15 |
| R-PRJ-08 | Block activation for missing valid-cost inputs or unresolved findings unless findings receive justified disposition. Apply readiness to every coordinated member; negotiated selling price is not required. | §6 | SB-19; SB-14, SB-16 |
| R-PRJ-09 | Forecast/quoted-scope edits recalculate affected project draft demand/costs and flag saved scenarios; accepted/history results remain unchanged. Incomplete inputs yield issues/missing results, not stale numbers labelled current. | §6 | SB-20; SB-16, SB-18 |

### CAD, identity, review, and feasibility

| ID | Obligation and boundary | Source | Owning packages |
|---|---|---|---|
| R-CAD-01 | Accept STEP assemblies, solids, and surface representations; preserve customer hierarchy, placements, quantities, and geometry associations. Metadata review must not restructure a usable customer hierarchy. | §§3–5, 7 | SB-04; SB-11 |
| R-CAD-02 | Keep tree/viewport selection synchronized with the reviewed definition/occurrence, including repeated placements. Expose missing or uncertain analysis. | §§4–5, 7 | SB-11; SB-04, SB-07 |
| R-CAD-03 | For flattened inputs, engineers confirm flat component-to-solid/surface associations separately from the import. Do not invent nested assemblies or equate one body with one component. | §7 | SB-11; SB-04 |
| R-CAD-04 | Extract constant-diameter tube OD/ID, developed centerline length including arcs, exact bend count, and each centerline bend radius. Extract wear/thermal protection developed lengths including curves. Other components require solid/enclosed volume. | §7 | SB-12; SB-04 |
| R-CAD-05 | Derive straight tube-end lengths, straight tangent-to-tangent lengths between bends, and tube-end/connector associations needed by feasibility. | §§7, 9 | SB-12; SB-14 |
| R-CAD-06 | Meet ±0.1% for each required nonzero dimensional/volume reference; bend count is exact and each radius is checked individually. Do not relax tolerance to fit benchmark outcomes. | §7 | SB-12; SB-22 |
| R-CAD-07 | Establish coverage, supported file/assembly sizes, and processing-time targets from representative independently verified owner examples. Inventory is not a benchmark; zero-reference validation remains an explicit unresolved acceptance detail. | §§7, 15 O07 | SB-12; SB-13, SB-22 |
| R-CAD-08 | Allow manually completed missing measurements so surfaces can proceed to costing when other requirements are met. Unavailable volume is missing, never zero or invented geometry. | §§7, 11; P09 | SB-11; SB-04, SB-07, SB-16 |
| R-REV-01 | Recognize tubes, connectors, fittings, clips, wear protection, and thermal protection; additional trained categories must not require a domain redesign. | §7 | SB-13 |
| R-REV-02 | Require engineer confirmation of every recognized classification, support bulk confirmation/correction, and keep unconfirmed labels pending regardless of confidence. Collect confirmed labels as training examples. | §§4, 7 | SB-11; SB-13 |
| R-ID-01 | Share part metadata and manufacturing routing across repeated occurrences/products within a project while retaining occurrence hierarchy, placement, and quantity. | §§3, 8 | SB-10; SB-11, SB-15 |
| R-ID-02 | Customer number/revision proposes identity and geometry validates it. Confirm ambiguous/missing/conflicting evidence before sharing edits or transferring work; similar geometry alone never merges parts. | §8 | SB-10; SB-11 |
| R-ID-03 | Different customer revisions may coexist with separate definitions, routing, and demand. Do not overwrite earlier accepted versions or invent a revision to evade coordinated activation. | §8 | SB-10; SB-16, SB-19 |
| R-CAT-01 | Catalog matches remain suggestions until confirmed; unconfirmed specifications/prices cannot enter costs. Preserve versioned catalog references and separate project overrides. | §8 | SB-13; SB-09, SB-16 |
| R-FEA-01 | Feasibility is independent of manufacturing cost-center selection. Check each bend radius, tangent spacing, tube/connector compatibility, and available end assembly length. | §9 | SB-14; SB-12 |
| R-FEA-02 | Connector requirements come from confirmed catalog specifications, with configurable rules for custom connectors. Explain rule, supporting geometry/measurement, and required limit. | §§5, 9 | SB-14; SB-09, SB-13 |
| R-FEA-03 | Use structured applicability/limit tables, not a general formula builder. Enter approved engineering sources; apply supplied radius tables only within their material/geometry conditions. Do not invent limits or interpolate silently. | §9 | SB-14 |
| R-FEA-04 | Missing required limits are unresolved/not evaluated. Conflicting applicable limits require Administrator resolution; no hidden priority. A justified exception remains available. | §9 | SB-14; SB-09 |
| R-FEA-05 | Engineers may accept feasibility exceptions with recorded justification; unresolved findings otherwise prevent activation. | §9; §6 readiness | SB-14; SB-19 |

### Manufacturing and recurring cost baseline

| ID | Obligation and boundary | Source | Owning packages |
|---|---|---|---|
| R-MFG-01 | Engineers explicitly choose purchased or internally manufactured for each custom part, consistently within a shared project part version. | §§8, 11 | SB-15; SB-19 |
| R-MFG-02 | Routings/templates apply at part, subassembly, and product root. Applied routings are independently editable; shared part edits propagate to its occurrences subject to accepted-revision preservation. | §§8, 10 | SB-15; SB-19 |
| R-MFG-03 | Parent installation/joining/testing operations belong to that assembly context and are not repeated in every child unit cost. | §8 | SB-15; SB-16 |
| R-MFG-04 | Setup is duration in hours × its dedicated setup hourly rate, shown as a separate charge, with no batch allocation. Count each shared part routing's applicable setup once per quotation; assembly setups count at their own level without duplicated display allocation. | §10 | SB-16; SB-15; prototype SB-02 |
| R-MFG-05 | Attach packaging, adhesive, consumables, and other non-CAD cost lines to parts/assemblies without adding CAD nodes. Explicit category, quantity, and unit basis determine consumption/charges and exports. | §11 | SB-15; SB-16, SB-21 |
| R-DEM-01 | Project owns average, maximum, or median annual costing-volume policy; scenarios cannot override it. Expand nested/repeated quantities and aggregate shared-part demand by calendar year before applying the policy. | §11 | SB-16 |
| R-DEM-02 | Project SOP/EOP defaults permit product overrides; schedules use calendar years inclusive with a positive forecast for every production year. Missing is incomplete; do not zero-pad outside schedules/gaps or prorate by month. | §§11–12, O09 | SB-10; SB-16 |
| R-DEM-03 | Keep actual annual demand, representative annual costing volume, and lifetime demand distinct. Use actual forecasts for revenue and lifetime demand for the applicable tooling basis. | §§11–12 | SB-16; SB-17, SB-18 |
| R-COST-01 | Purchase tiers use the greatest minimum annual quantity not exceeding representative costing volume. Support a single price; uncovered demand without a valid single price is missing pricing. Retain price source/version and confirmed match. | §11 | SB-16; SB-09 |
| R-COST-02 | Selected versioned cycle/rate assumptions remain fixed without automatic volume-driven switching, inflation, or midlife adjustment. Geometry-driven cycles may change with their geometry; rate updates require explicit adoption. | §§11, 14 | SB-15; SB-16, SB-20 |
| R-COST-03 | One project currency; retain source prices/currencies and saved manual FX expressed as project units per source unit. Multiply by a positive cross-currency rate; same currency is identity. Preserve historical rates/results; no live FX. | §11 | SB-16; SB-03 |
| R-COST-04 | For manufactured parts, geometry supplies net consumption: annular tube volume or other enclosed volume, density to mass, explicit price/kg conversion. Permit sourced/unit-labelled net-consumption override; no zero substitution. | §11 | SB-16; SB-11, SB-12 |
| R-COST-05 | Apply material allowance once to net consumption: net × (1 + rate), including explicit zero allowance. It changes material cost only, not demand, cycles, setup, or recovery quantity. | §11 | SB-16; prototype SB-02 |
| R-COST-06 | Roll children into parents once and add parent operations/categories once. Reuse shared unit costs with occurrence quantities, not entire project demand per product. | §11 | SB-16 |
| R-COST-07 | Purchased assembly price covers descendants: retain CAD hierarchy but stop cost, demand, and tooling expansion in that occurrence branch. Own assembly operations may apply; the same part elsewhere still contributes. | §11, O14 | SB-16; SB-17 |
| R-COST-08 | Cost categories are supported direct costs, fixed per-unit amounts, or percentages of explicit cost subtotals; prevent circular/duplicate bases. App configuration selects recurring EBIT categories; no custom formula language. | §12 | SB-09; SB-16, SB-18 |
| R-COST-09 | Machine depreciation is included through applicable cost-center rates. Do not add project equipment CAPEX/amortization; tooling remains separate approved functionality. | §§11, 14 | SB-16; SB-X01 integration boundary |
| R-COST-10 | Preserve reproducible saved calculation inputs/versions/results and expose cost completeness. Do not silently replace accepted results or treat required missing inputs as zero. | §§1, 3, 5–6, 13 | SB-03; SB-16, SB-19, SB-20 |

### Tooling, commercial scenarios, shared updates, and exports

| ID | Obligation and boundary | Source | Owning packages |
|---|---|---|---|
| R-TOOL-01 | Tool investment is count × cost/tool; each tooling entry belongs to one part/assembly definition and is counted once across products. Multi-part/family tools are deferred; do not duplicate investment to simulate them. | §12 | SB-17 |
| R-TOOL-02 | Support lifetime-demand or manual positive recovery quantity for parts and assemblies; expand subassembly occurrences and use product lifetime demand for root tooling. Respect purchased-branch boundaries. | §12 | SB-17; SB-16 |
| R-TOOL-03 | Allocate shared investment to products by lifetime component-demand shares without multiplying project investment or changing recovery quantity. Missing/zero combined demand leaves allocation incomplete. | §12 | SB-17 |
| R-TOOL-04 | Keep separate-charge/amortized alternatives accessible; switching updates the commercial view without changing tooling inputs. Show investment, basis, quantity, contribution, and rollups; do not recover the full investment twice. | §12 | SB-17; SB-18 |
| R-TOOL-05 | Continue quoted contribution through the production period without automatic cap/stop at recovery quantity. Show forecast under/over-recovery; changes require a revised scenario. Actual-sales tracking is outside scope. | §12 | SB-17; SB-18 |
| R-COM-01 | Named scenarios share project EBIT target and volume method. Project owns default tooling mode and productivity requirement; scenarios may override tooling mode and hold manual prices/recovery assumptions/tooling productivity treatment. | §12 | SB-18; SB-17 |
| R-COM-02 | Always show suggested base price alongside manually entered base price and its EBIT. Manual price excludes tooling; blank price means uncalculated manual EBIT, not zero revenue or a valid rate. | §12 | SB-18 |
| R-COM-03 | Recurring EBIT excludes tooling recovery/investment and separate setup revenue/cost. Use included recurring unit cost C and target t: suggestion C/(1−t); manual EBIT amount P−C and rate (P−C)/P. Require P>0 for rate and t<100%. Target the first production year, not lifetime weighted EBIT. | §12 | SB-18; SB-16 |
| R-COM-04 | Annual productivity compounds base selling price after each product's own first year: P₁×(1−r)^(n−1). Manufacturing costs do not fall by this rule; use actual annual forecasts for revenue. | §12 | SB-18; SB-16 |
| R-COM-05 | Scenario selects whether productivity reduces tooling contribution or leaves it constant. Display base plus amortized tooling as total unit price; separate tooling stays outside unit price. | §12 | SB-18; SB-17 |
| R-COM-06 | Save multiple named commercial alternatives and allow side-by-side comparison on the common engineering/costing baseline. | §§3, 12 | SB-18 |
| R-UPD-01 | Catalog/specification/price, rate, feasibility-rule, routing-template, and app-configuration changes flag affected projects and explain impact. Adoption/recalculation is explicit; shared maintenance never silently rewrites project inputs/history. | §13 | SB-20; SB-09 |
| R-EXP-01 | Export internal Excel with detailed BOM, metadata, routings, feasibility, costs, and scenarios; export PDF as a cost/annual price/EBIT/tooling/setup/issues summary. Customer quotation generation is excluded. | §13 | SB-21 |
| R-EXP-02 | Permit unfinished exports before activation. Label Draft / Incomplete, list missing inputs/unresolved findings, and identify outdated costs. Preserve selected results rather than claiming stale numbers are current. | §13 | SB-21; SB-20 |
| R-ACC-01 | Deliver and record approved prototype P01–P14 before claiming prototype acceptance; subsequently verify confirmed MVP obligations and application scenarios. Prototype completion does not establish full costs, activation, quotation readiness, recognition coverage, or customer accuracy. | §§1, 16, 18; prototype §§1, 3, 6 | SB-08; SB-22 |

## Application acceptance scenarios A01–A20

These local IDs correspond one-to-one to the numbered examples in [application §16](requirements/cost_calculator_handover/APP_SPECIFICATION.md#16-acceptance-scenarios). The source explicitly preserves proposal status for some clauses. Every row is **pending integrated execution**; a reusable CAD primitive or workbook calculation does not pass the scenario.

| ID | Scenario and required observation | Requirement links | Owning packages | Authority qualification |
|---|---|---|---|---|
| A01 | Partial delivery changes B/adds D while A/C and old active B remain intact until activation. | R-PRJ-02/03/06 | SB-10; SB-19 | Confirmed. |
| A02 | A successful analysis remains reviewable while failed B can retry or be manually completed. | R-PRJ-04 | SB-11; SB-06, SB-10 | Confirmed. |
| A03 | Changed tube length retains prior work and identifies affected consumption/routing/feasibility review. | R-PRJ-04, R-CAD-04 | SB-11; SB-12, SB-15, SB-20 | Confirmed preservation/review behavior; exact invalidation metadata is a technical design. |
| A04 | Two part occurrences in a subassembly used three times yield quantity six; shared metadata/routing edited once. | R-ID-01, R-DEM-01 | SB-16; SB-10, SB-15 | Confirmed. |
| A05 | Shared sourcing change across A/B cannot activate only A; coordinated change succeeds for both or neither. | R-PRJ-07/08 | SB-19; SB-15 | Confirmed coordination/readiness; transaction mechanism remains implementation detail. |
| A06 | Tree/viewport link survives metadata edits; hierarchy remains unchanged; flat grouping preserves original import without invented nesting. | R-CAD-01/02/03 | SB-11; SB-04 | Confirmed. |
| A07 | 1,000 mm reference accepts 999–1,001 inclusive; bend count exact and radii checked separately. | R-CAD-04/06/07 | SB-12; SB-22 | Confirmed; independent references/coverage targets remain prerequisites. |
| A08 | Open-surface volume stays missing; reviewed manual measurement enables otherwise valid costing. | R-CAD-08, R-COST-04 | SB-11; SB-04, SB-16 | Confirmed fallback; full MVP provenance fields are Proposal. Prototype P09 separately approves units/source/review persistence. |
| A09 | Unconfirmed connector match cannot supply costing price; override retains original catalog version/value. | R-CAT-01 | SB-13; SB-09, SB-16 | Confirmed. |
| A10 | Bend spacing is tangent-to-tangent; justified exception remains visible; changed supporting geometry prompts review. | R-CAD-05, R-FEA-01/05 | SB-14; SB-12, SB-20 | Geometry/justification confirmed. Automatic exception reopening is Proposal in the source; record any chosen design explicitly. |
| A11 | Product forecasts 100/200 and 200/100 aggregate to 300/300; peak is 300, not 400. | R-DEM-01 | SB-16 | Confirmed. |
| A12 | Missing forecast remains incomplete and scenarios cannot change project volume policy. | R-DEM-01/02 | SB-16; SB-10, SB-18 | Confirmed. |
| A13 | Setup 0.5 h ×120/h yields separate 60, without per-piece/batch amortization. | R-MFG-04 | SB-16; SB-15; prototype SB-02 | Confirmed. |
| A14 | 100 at 3% gives 100/97/94.09 from product Year 1; manufacturing cost unchanged. | R-COM-04 | SB-18 | Confirmed. |
| A15 | Investment 10,000 and recovery quantity 5,000 illustrate contribution 2; alternatives preserve investment without double counting. | R-TOOL-01/02/04 | SB-17; SB-18 | Investment/recovery alternatives confirmed; the precise initial contribution formula is marked Proposal in the source and needs the package contract's explicit resolution. |
| A16 | Base 100 plus tooling 2 gives 102; Year 2 is 99 if tooling exempt or 98.94 if both reduce by 3%. | R-COM-02/04/05 | SB-18; SB-17 | Confirmed treatment for the given contribution; does not independently approve A15's derivation. |
| A17 | Suggested target price stays visible beside manual price/EBIT; blank manual price is not zero revenue. | R-COM-02/03 | SB-18 | Confirmed. |
| A18 | Shared price/rule update flags dependent projects only; updated inputs require explicit adoption/recalculation, and historical snapshots remain unchanged. | R-UPD-01 | SB-20; SB-09 | Confirmed. |
| A19 | Named comparisons share project target/volume policy and expose outdated dependencies. | R-COM-01/06, R-PRJ-09 | SB-18; SB-20 | Shared settings and forecast/scope staleness confirmed; common-baseline metadata/comparison fields are Proposal. |
| A20 | Internal exports preserve selected results and identify relevant scope/scenario/versions. | R-EXP-01/02 | SB-21 | Export profiles and draft/stale truthfulness confirmed; exact active/draft/scenario/calculation metadata fields are Proposal. |

## Approved prototype checks P01–P14

All checks come from [prototype §6](requirements/cost_calculator_handover/DESKTOP_PROTOTYPE_SPEC.md#6-approved-acceptance-checks-execution-pending) and are **Confirmed, pending execution**. SB-08 records integrated evidence for every row; SB-22 must not treat this limited prototype as full MVP acceptance.

| ID | Check and acceptance observation | Implementing owners | Evidence required |
|---|---|---|---|
| P01 | Import primary synthetic STEP; preserve source hash, definition/occurrence quantities and transforms; expose degraded hierarchy. | SB-04; SB-05, SB-07 | Independent fixture manifest, original/final hash, imported mapping. |
| P02 | Six selections across tree/viewport resolve three occurrences to one shared reviewed part; edits propagate without geometry changes. | SB-04; SB-07 | Selection assertions and native recording/screenshots. |
| P03 | One definition measures 2,400 ±2.4 mm³ with units/source; quantity three remains separate. | SB-04; SB-02, SB-07 | Exact-geometry measurement and independent expected dimensions. |
| P04 | Classification requires confirmation; manufactured sourcing and material selection explicit; no fallback CAD density can complete costs. | SB-07; SB-02, SB-06 | Review-state and missing-material tests plus UI evidence. |
| P05 | Fixture arithmetic: material 0.132192, overhead 0.0066096, production 1, part 1.1388016, assembly 3.4164048, separate setup 60 CNY; tolerance 0.0000001 CNY. | SB-02; SB-03, SB-07 | Pure/backend tests and independent recomputation for actual measured volume, without intermediate display rounding. |
| P06 | Save, terminate desktop, restart, reopen by project ID; same source/mappings/reviewed inputs/calculation recovered; recalculation creates new result. | SB-03; SB-04, SB-06, SB-07 | Real PostgreSQL persistence and full process-restart evidence. |
| P07 | Change cycle 30→60 s; stale prior result retained; ideal new part 2.1388016 and assembly 6.4164048 CNY. | SB-06; SB-02, SB-03, SB-07 | Invalidation, historical result, and exact arithmetic assertions. |
| P08 | Missing density/cycle yields named issues and no complete current subtotal; distinguish missing from explicitly supported zero. | SB-02; SB-03, SB-07 | Validated input contract and missing/zero cases. |
| P09 | Open surface has unavailable volume; reviewed manual value with units/source can enable costing and survives persistence. | SB-04; SB-02, SB-03, SB-07 | Surface fixture, manual source/review record, and cost test. |
| P10 | Identical customer number with B/C revisions remains separate through save/reopen and edits. | SB-04; SB-03, SB-07 | Identity/mapping and persistent edit-isolation tests. |
| P11 | Flat unresolved component associations preserve import, require review, and cannot yield complete costs; grouping editor remains outside prototype. | SB-04; SB-02, SB-07 | Flat fixture with unresolved associations and no body-count identity inference. |
| P12 | Backend failure, rejected version/write, late response, and explicit retry preserve drafts/central state without duplicate records or stale/wrong-selection adoption. | SB-06; SB-03, SB-07 | Fault injection, idempotency, edit-generation, and retained-edit tests. |
| P13 | During controlled five-second analysis/service delay, repaint/selection/navigation respond within one second on declared test Mac. | SB-06; SB-07 | Recorded OS/CPU/GPU/RAM and measured latency; separate from deterministic tests and customer-file performance claims. |
| P14 | Label Draft prototype / Direct-cost subtotal with omitted categories; no activation/production quotation claim. Block source modeling/reparenting through all entry paths while allowed review actions work. | SB-05; SB-07 | UI/shortcut/control/command mutation tests, ordinary editable-session regression, native labels/evidence. |

## Proposals, historical rules, and exclusions

This register prevents source recommendations from silently becoming mandatory product policy. Package owners must record adopted technical decisions and unresolved questions in their brief/contracts, not rewrite the source's confirmation status.

| ID | Item and authority boundary | Owning packages |
|---|---|---|
| Q-01 | Internal IDs, version pinning, transactions, and comparison-state mechanics are source Proposals or technical choices. Required identity separation, preserved history, coordinated activation, and prototype persistence are confirmed outcomes. | SB-01, SB-03, SB-10, SB-19 |
| Q-02 | Single-editor leases and simultaneous viewing are Proposal; concurrent project editing is deferred. Expected-version conflict checks are the selected technical design, not a new collaboration feature. | SB-03, SB-10 |
| Q-03 | Full measurement/repair provenance, repair-method policy, and varying-diameter completion representation are Proposal. Missing-versus-zero and manual fallback are confirmed; P09 confirms its narrower units/source/review contract. | SB-04, SB-11, SB-12 |
| Q-04 | Model-version retention, curated/held-out label mechanics, and avoiding silent reclassification are proposed recognition safeguards. Confirmed training-example collection does not authorize automatic training/deployment. | SB-11, SB-13 |
| Q-05 | Exact required purchasing fields, cycle-basis/tooling provenance, and routing-template instantiation/version mechanics are proposed details to resolve in typed contracts. Confirmed price/currency/sourcing/routing outcomes still apply. | SB-09, SB-15, SB-16 |
| Q-06 | Full exception audit fields and automatic reopening on changed inputs are Proposal; the accepted roadmap assigns supporting-input review to SB-14/SB-20 as a technical design to document. | SB-14, SB-20 |
| Q-07 | Deterministic functions, Decimal representation, explicit conversion, and delayed display rounding are proposed MVP mechanisms, selected by the active technical design; prototype P05 fixes numeric behavior/tolerance. | SB-01, SB-02, SB-16 |
| Q-08 | Initial tooling contribution derivation, exact comparison/baseline metadata, dependency acknowledgement/adoption fields, and export metadata/layout are Proposal/design details; confirmed ownership, no-double-recovery, stale labeling, and explicit adoption constrain them. | SB-17, SB-18, SB-20, SB-21 |
| H-01 | Workbook ten-year horizons, implicit sourcing/price precedence, Year1×duration recovery, tooling inside recurring cost, one-time productivity uplift, ambiguous FX division, and sample rates are superseded or restricted by application §14. Compatible unit/cycle/material arithmetic remains reference material. | SB-02, SB-16, SB-17, SB-18 |
| H-02 | Earlier browser-first recommendation is superseded. The supplied technical plan targets an older checkout; its execution order/contracts are reconciled by the tracked package roadmap. Old CLI probes establish only their recorded cases. | SB-00, SB-01, SB-04 |
| H-03 | Historical ten-STEP inventory is not the current package inventory: imported references contain 16 STEP files and 28 CSVs. File presence does not establish CSV provenance, part associations, independence, or measurement accuracy. | SB-00, SB-12, SB-13, SB-22 |
| D-01 | Outside initial delivery/prototype: browser client, offline authoritative edits/local costing/sync, production installers/updates, and Windows/Linux prototype delivery. No date or parity promise. | SB-08, SB-22 scope checks |
| D-02 | Outside MVP: other CAD formats including Creo/CATIA, automatic varying-diameter tube measurement, production batches/setup batch allocation, concurrent editing, separate revision approver, customer-facing quotations, NPV/payback/original-improved investment analysis, general formula builder, multi-part tools, and automatic training/deployment. | SB-22 scope checks |
| D-03 | No synthetic CAD hierarchy restructuring; non-CAD items are attached costs. Destructive project/product deletion, broad archival, live FX, actual-production recovery tracking, separate project machine CAPEX, and supplier/purchasing workflows are not added by these requirements. | SB-10, SB-15–SB-18, SB-22 |

## Machine hourly-rate extension SB-X01

The [machine handover](requirements/machine_hourly_handover/00_HANDOVER.md) describes a separate rate-building subsystem. The accepted roadmap preserves it as a **gated extension**, not part of P01–P14 or an implicit expansion of confirmed MVP behavior. Its referenced machine workbook is not present in the imported package; the cost-calculator workbook is a different file. Documented historical reconciliations are not executed tests in this checkout.

| ID | Extension responsibility or readiness gate | Owner / integration |
|---|---|---|
| X01-01 | One parameterized rate engine for resources, explicit stable machine/cost-center identities, assumptions, financial year, and immutable versions; no worksheet-name relationships. | SB-X01; publishes to SB-09 |
| X01-02 | Compute capacity, loading, availability, running hours, loaded downtime, fixed/variable/standby costs, labor, depreciation, fully absorbed and cash rates with explicit denominator and units. | SB-X01 |
| X01-03 | Sensitivity changes loading only and uses the same engine; reconcile base-loading result and analytical monotonic behavior. Reporting must not sum hourly rates as a meaningful portfolio cost. | SB-X01 |
| X01-04 | Publish immutable fully absorbed currency-per-actual-running-hour versions for explicit project adoption. Operation cost uses rate×seconds/3600; do not apply availability twice, select loading from quotation volume, or add machine depreciation again. | SB-X01; SB-09, SB-15, SB-16, SB-20 |
| X01-05 | Resolve premises/SG&A, compressor electricity/air, cooling-water basis, staffing/loading, and depreciation/residual conventions before production rates are publishable. | SB-X01; required owner/engineering/finance inputs |
| X01-06 | Resolve missing financial year/cost-center IDs, illustrative/unconfirmed machine assumptions, and inconsistent zero-hours/year validation. Missing inputs cannot become publishable zero values; document blocked/unpublishable results. | SB-X01 readiness gate |
| X01-07 | Preserve the handover's eight baseline/64 sensitivity expectations as historical reference until source availability and assumptions permit independently reproducible validation. Do not treat cached numeric outputs as approved rates. | SB-X01 verification gate |

## Code evidence and coverage limits

The following evidence explains reuse and gaps at the reviewed commit. It is source inspection, not a runtime test result. Relative source links intentionally follow this repository across worktrees; line references in the descriptions apply to the baseline above.

| Evidence | Consequence for package planning |
|---|---|
| [STEP import planning](../../crates/engine/src/cmd/file_import.rs), lines 95–139, preserves supported components/occurrences but can flatten unsupported/deep/large mappings. | SB-04 must report degraded mappings and preserve source identities separately from runtime IDs. |
| [BOM implementation](../../crates/engine/src/cmd/bom.rs), lines 353–387, groups by part number without revision in its default key. | Reuse quantity traversal, not default rows as business identity; SB-04/SB-10 preserve customer versions. |
| [Body inspection](../../crates/engine/src/cmd/inspect.rs), lines 153–170, distinguishes closed volume and measurement provenance; [selection measurement](../../crates/engine/src/cmd/measure_sel.rs), lines 381–392, usually uses display-mesh volume. | SB-04 must select the authoritative geometry path; display measurements do not prove ±0.1% acceptance. |
| [Session policy and file-read-only behavior](../../crates/engine/src/lib.rs), lines 493–495 and 939 onward, already expose distinct file protection and a parameter-aware command-policy hook. | SB-05 composes per-document review policy with the existing hook; file read-only alone does not protect geometry. |
| [File loading](../../crates/ui-egui/src/file_load.rs) runs work off-thread but uses a loading interaction state. | SB-06 must demonstrate nonmodal analysis/service work for P13 rather than infer responsiveness from background execution. |
| [Local save](../../crates/engine/src/cmd/file.rs), lines 820–852, and [control server](../../apps/solidcraft/src/control_server.rs), lines 58–65, implement local files and loopback automation. | Neither supplies the required central project database, role model, or costing API; SB-03/SB-09 are new business subsystems. |
| [Layer registry](../../xtask/src/layers.rs), [root CI/wasm](../../xtask/src/main.rs), and [license roots](../../xtask/src/licences.rs) cover the existing CAD workspace and shipped app roots. | SB-01 must register portable crates and explicitly cover the separate backend's checks/licenses/lockfile in CI. |

Coverage allocation is complete for the confirmed application obligations grouped above, all 20 application examples, and all 14 prototype checks. This does not settle proposal-only mechanisms or certify the supplied references. SB-12/SB-13/SB-22 still require independent customer reference associations and agreed coverage/performance targets; zero-reference validation and commercial switching latency need explicit acceptance definitions. SB-14 must retain unresolved missing/conflicting engineering limits. SB-X01 remains gated by its policy/source-input issues. No acceptance row may be marked passed solely because it has an owner or a source-code link.
