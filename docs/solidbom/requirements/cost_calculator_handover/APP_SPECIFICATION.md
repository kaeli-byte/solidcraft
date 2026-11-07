# Engineering BOM and Costing App — Authoritative Specification

Date: 2026-11-05

Last decision update: 2026-11-06 — desktop-first SolidCraft fork selected; prior business decisions retained. Desktop execution/persistence choices and the proposed prototype are recorded separately from confirmed requirements.

Status: consolidated design for review; not authorization to implement unresolved behavior.

## 1. Authority and purpose

This specification consolidates the owner's decisions from the product discussion. It is the source of truth for app behavior. The existing workbook handover remains a costing reference, not an overriding application specification.

Authority order:

1. Subsequent explicit owner decisions.
2. Confirmed requirements in this document.
3. Approved resolutions to the open decisions in section 15.
4. Compatible calculation rules in the handover.

Statements marked **Proposal** are design recommendations, not confirmed owner decisions. Items marked **Open** must not be silently resolved by copying Excel behavior. Acceptance examples below are specification checks, not executed software tests.

The app helps engineers turn customer STEP assemblies into a reviewed BOM, assess feasibility, define manufacturing, calculate costs, and compare internal quotation scenarios. Preserve customer structure, explain automated suggestions, retain engineering work across revisions, and make accepted results reproducible.

**Confirmed 2026-11-06 — delivery direction, revised:** build the initial SolidBOM experience in a fork of the SolidCraft desktop application, with costing and engineering-review panels connected to its existing CAD tree and viewport. Desktop delivery takes priority; a browser client is outside the initial delivery scope. This supersedes the previous browser-first interface and no-workstation-installation requirement. It does not authorize implementation of unresolved behavior.

**Retained confirmed responsibilities:** projects/shared data remain central, one organization per MVP deployment, and cost calculations run on the backend. The desktop decision does not by itself authorize standalone local projects, offline authoritative edits, local production costing, or a new synchronization model. These responsibilities remain in force unless the owner explicitly revises them.

**Confirmed 2026-11-06 — prototype execution split (D01):** STEP import, viewport, selection and geometry measurements run in the desktop application, with long-running geometry analysis in the background. Projects, original STEP files, reviewed measurements/manufacturing inputs, cost calculations and saved calculation results remain central. This resolves the prototype CAD-location choice; it does not establish automated recognition/extraction coverage or measured performance.

**Confirmed 2026-11-06 — prototype connectivity and saving (D03/D04):** project writes are online-only. On connection failure, keep unsaved form edits in the current desktop session, show the failed save and support explicit retry; no authoritative local costing or silent later synchronization. Save project centrally persists reviewed inputs and source associations. A local SolidCraft file save is separate and does not imply a saved SolidBOM project. Local geometry/cache is temporary and rebuildable from the saved source.

**Confirmed 2026-11-06 — first prototype platform (D02):** macOS first. Windows/Linux delivery is outside the first prototype; no later delivery date is implied. Target architecture and minimum macOS version remain technical planning details to establish from the development/test machine.

**Confirmed 2026-11-06 — prototype CAD-edit policy (D05):** imported geometry is read-only. Selection, measurement, annotations and engineering/costing edits remain available, but modeling/reparenting of the reviewed source is blocked across UI, shortcuts and programmatic commands. Geometry changes arrive through a new STEP revision. Annotations do not alter source geometry/customer hierarchy; this does not add full revision comparison/activation to the first prototype.

**Confirmed 2026-11-06 — prototype delivery (D06):** use a macOS development build launched locally. Signed installers, automatic updates and production distribution are deferred beyond the first prototype.

**Confirmed 2026-11-06 — prototype scope and acceptance (D07):** the owner approved the scope, numeric test fixtures/comparison tolerance and checks P01–P14 in [DESKTOP_PROTOTYPE_SPEC.md](DESKTOP_PROTOTYPE_SPEC.md). This includes a maximum one-second UI response during a controlled five-second background delay on the declared test Mac. Fixture prices/rates are test inputs, not company defaults. All checks remain unexecuted; approval establishes a planning baseline, not software acceptance or permission to implement. Artifact formats, backend write/version contracts, hosting provider, backend framework and deployment mechanics remain technical design choices.

**Confirmed 2026-11-06 — organization scope:** one organization per MVP deployment. Its users share company catalogs, feasibility rules, cost centers and app configuration under the agreed Engineer/Administrator permissions. Multi-organization isolation, company onboarding and cross-company administration are deferred. This decision does not authorize communication or sharing outside the organization.

Anti-goals: silently changing accepted costs, treating missing inputs as zero, inventing geometry measurements, double-counting shared tooling or child costs, and treating legacy spreadsheet defects as requirements.

## 2. Approaches considered

| Approach | Tradeoff | Decision |
|---|---|---|
| Extend the workbook-derived handover into the application specification | Preserves its organization but interleaves legacy formula behavior with new engineering workflows | Not selected |
| Separate authoritative app specification with the handover retained as a costing reference | Establishes a clear behavioral contract and requires explicit reconciliation of legacy formulas | Selected by owner |

### Delivery approaches, revised 2026-11-06

| Approach | Tradeoff | Decision |
|---|---|---|
| Separate browser application using SolidCraft as a CAD worker | Independent web UI; requires a separate viewport/selection integration | Earlier recommendation superseded for initial delivery |
| Fork SolidCraft desktop and add SolidBOM panels | Reuses the native CAD tree, viewport and selection; requires maintaining the fork and connecting central services | Owner-selected initial direction |
| Standalone desktop application with authoritative local data/costing | Could work without a server; introduces different data ownership and synchronization requirements | Not selected by desktop preference; requires an explicit change to retained central responsibilities |

The current folder is not a Git repository. This document is saved alongside the handover; no commit is implied.

## 3. Vocabulary and ownership

| Term | Definition and ownership |
|---|---|
| Project | A customer inquiry: name, customer, internal team, volumes, SOP, EOP, platform, application, and business details. Owns packages, product identities, shared project parts, and quotation scenarios. |
| Package | One customer delivery of product data. Belongs exclusively to one project; never shared across projects. MVP imports STEP; PDFs and tables are future scope unless separately agreed. |
| Product | A simple part or complex assembly in project scope, with its own volumes. Identity persists across customer deliveries. |
| Product revision | The version of a product delivered in one package. May be draft, active, or historical. A new delivery may revise selected products and add others. |
| CAD/BOM occurrence | A node in the customer's hierarchy, with its parent, placement, quantity, and geometry association. The BOM follows this hierarchy. |
| Project part definition | Shared component metadata, sourcing choice, and part manufacturing routing referenced by occurrences across products in that project. Different customer part revisions must remain distinguishable. |
| Catalog component | Shared reference component with versioned specifications and price. Project-specific overrides are separate. |
| Routing | Ordered manufacturing operations attached to a part, subassembly, or product root. |
| Quotation scenario | A named commercial alternative using the common project engineering/costing baseline. |
| Calculation snapshot | An immutable record of inputs, referenced versions, outputs, and issues for a calculation. |

**Proposal:** use stable internal IDs, never display order or customer part number alone, for relationships. Customer identifiers and geometric similarity can assist matching but must not silently establish shared identity.

## 4. Scope

### Confirmed MVP

- Desktop-first SolidCraft fork with SolidBOM engineering-review and costing panels linked to the existing tree and viewport.
- Project/business-detail creation and editing; product-specific production volumes.
- STEP packages with assembly hierarchy, solid bodies, and surface representations.
- Automated recognition, geometry extraction, feasibility findings, and engineer review through a synchronized product tree and 3D viewport.
- BOM following the customer's CAD hierarchy; no engineering restructuring of that hierarchy.
- Flattened/multi-body STEP fallback: flat component review with engineer-confirmed grouping of solids/surfaces, preserving original import structure; no automatic reconstruction of a nested assembly tree.
- Shared part metadata and manufacturing routing within and across products in the same project.
- Versioned customer revisions, preserved engineering work, explicit activation, and coordinated sourcing changes.
- Component catalogs, cost centers, routing templates, and feasibility rules as shared data.
- Purchased and internally manufactured custom parts; sourcing explicitly selected by the engineer.
- Setup cost as a separate charge; no production batch planning.
- Additional non-CAD cost lines attached to a part or assembly, including packaging, adhesive and consumables; included in costing and internal exports without changing CAD/BOM hierarchy.
- Shared-demand costing, project-level volume basis and EBIT target, configurable EBIT cost categories.
- Named commercial scenarios, manual and suggested selling prices, tooling alternatives, annual productivity.
- Internal Excel and PDF exports.
- Collection of confirmed classifications as training examples.

### Deferred or unapproved

- Browser client for the initial delivery; no browser parity or delivery date is implied.
- Offline authoritative project editing, standalone local costing and synchronization are not approved by choosing desktop.
- Creo and CATIA imports; other CAD formats.
- Automatic measurement of varying-diameter tubes.
- Production batches and setup allocation through batches.
- Concurrent project editing and a separate revision approval role.
- Customer-facing quotation generation.
- NPV/payback, investment analysis, and original/improved comparisons: handover references only, not approved MVP functionality.
- Automatic model training/deployment from collected labels: not approved by collection of examples alone.
- Synthetic/non-geometric restructuring of the CAD/BOM hierarchy. Non-CAD items are supported as attached cost lines, not new CAD/BOM nodes.

## 5. User journey

| Stage | Engineer action | App behavior |
|---|---|---|
| Define inquiry | Create project, enter business details and product forecasts | Validate required data; retain incomplete drafts |
| Receive delivery | Open/import a STEP package in the desktop app and associate products | Preserve delivery ownership and original source, parse hierarchy, report per-product progress/failures; central transfer and CAD execution follow the selected technical contract |
| Review engineering | Navigate tree/viewport; review classifications, measurements and matches | Link selected geometry to node metadata; expose missing/uncertain results |
| Assess feasibility | Review findings; correct inputs or justify exceptions | Explain rule, supporting geometry/measurements, and required limit |
| Define sourcing/manufacturing | Select sourcing; apply and edit routings | Share part-level edits across affected products and occurrences; keep assembly operations at their parent |
| Calculate | Complete cost inputs, inspect results | Calculate explainable costs with completeness indicators and dependency tracking |
| Negotiate internally | Compare named scenarios and enter selling prices | Keep suggested target-EBIT prices visible; show manual-price EBIT and annual projections |
| Activate revision | Activate own reviewed work | Keep prior revision active until explicit activation; coordinate shared sourcing changes |
| Export | Export selected baseline/scenario to Excel/PDF | Generate internal results with traceable versions |

**Proposal:** others may view while one engineer edits. A project edit lease or equivalent conflict check prevents a second session overwriting work. Concurrency within a single import worker system is an implementation concern, not multi-user editing.

## 6. Packages, revisions, and activation

### Confirmed behavior

- A delivery can update selected products; omitted products remain unchanged, not deleted.
- **Confirmed 2026-11-06 — quotation exclusion:** products no longer quoted remain in the project with files, engineering work and history retained. Exclude them from quoted shared-part demand, quoted costs and revenue rather than deleting them. They may be reinstated using retained work, subject to any outstanding review/update requirements. Exclusion/reinstatement are quotation-scope edits and trigger the agreed draft dependency recalculation; accepted snapshots remain unchanged.
- Preserve previous files, engineering work, revisions, and calculation results.
- Carry previous engineering work forward and highlight what needs review again.
- Engineers explicitly activate revised products; previous revisions remain the project costing basis until activation.
- The preparing engineer may activate their own revision; no separate approver is required.
- Sourcing changes to a shared part require activation of all affected products together. The part cannot simultaneously be purchased for one active product and manufactured internally for another in the same project.
- **Confirmed 2026-11-06:** coordinated activation also applies to other shared-part definition changes, including material, dimensions, and manufacturing routing. Review and activate all affected active products together; do not let independent activation leave them using different accepted definitions of the same shared customer part version. Distinct customer revisions may coexist under the identity rules in section 8. Product-specific changes without a shared-part impact may still activate independently. This does not remove the explicit user-adoption requirement for shared master-data updates.
- Successful analysis remains available while failed products are flagged for retry/manual completion.

### Proposed operational rules

- Separate comparison outcomes: unchanged, modified, added, removed, and uncertain match.
- Preserve prior values with provenance; preservation is not automatic confirmation of validity for changed geometry.
- Require review of uncertain revision/part matches before transferring accepted decisions.
- Version shared part definitions. Active products pin accepted versions; draft edits do not mutate active snapshots.
- Coordinate sourcing activation as an all-or-nothing transaction. If any affected product is unready, none of the group activates.
- Report invalidated dependencies, including catalog matches, manual measurements, feasibility exceptions, routing drivers, and calculated costs.
- Keep draft calculations distinct from active results. For a preview, substitute selected drafts into active scope without counting both revisions.
- Track import, extraction, classification, and feasibility progress separately. Retrying failed work must not erase accepted review work.

**Confirmed 2026-11-06 — activation readiness:** block activation when information required for a valid cost is missing or feasibility findings remain unresolved. Required inputs must be completed; findings must be resolved or accepted with recorded justification. A manually negotiated selling price is not required to activate the engineering/costing baseline. Incomplete work remains draft, not active with a provisional cost. Apply the same gate to every member of a coordinated activation group.

**Confirmed 2026-11-06 — forecast/scope edits:** automatically recalculate affected draft costs across the project when engineers change forecasts or quoted-product scope. Recompute shared-part demand and its downstream cost dependencies as a consistent draft baseline; flag affected saved quotation scenarios for refresh. Do not mutate accepted results or historical snapshots. If inputs are incomplete, expose missing results/issues rather than reuse stale numbers as current. Shared catalog/configuration updates still require explicit user adoption before entering draft calculation inputs.

Coordinated activation and readiness follow the rules above; field-level requiredness follows applicable sourcing, operations and configured cost categories.

## 7. CAD recognition and measurement

### Confirmed extraction requirements

| Category | Critical geometry |
|---|---|
| Constant-diameter tubes | OD, ID, developed centerline length including bend arcs, exact bend count, centerline radius of every bend |
| Wear protection | Developed centerline length including curves |
| Thermal protection | Developed centerline length including curves |
| Other components | Solid/enclosed volume |

Initial recognized categories: tubes, connectors, fittings, clips, wear protection, and thermal protection. Additional trained categories must be possible without redesigning the domain.

Feasibility also needs straight tube-end lengths, straight lengths between bend tangent points, and associations between tube ends and connectors. These are necessary derived inputs even though they were not in the initial critical-measurement list.

The owner has STEP assemblies with known classifications and verified measurements. Relative measurement tolerance is **±0.1%** for length, OD, ID, bend radius, cover length, and component volume:

**Confirmed 2026-11-06 — benchmark-first acceptance:** use representative owner-supplied, verified STEP examples to establish recognition coverage, supported assembly/file sizes and processing-time targets before fixing those numerical acceptance targets. The ±0.1% measurement requirement remains unchanged. Record actual benchmark results and agree the targets from evidence; choosing this approach does not mean benchmarks have run or achieved acceptable results. The STEP dataset is located as documented below; independent reference classifications/measurements are pending from the owner. Desktop delivery, retained central responsibilities and the confirmed prototype execution/save split are recorded in section 1.

**Dataset intake 2026-11-06:** owner supplied `benchmarking CAD/`; see [BENCHMARK_REFERENCE.md](BENCHMARK_REFERENCE.md). Ten STEP files, about 3.51–19.00 MiB, were inventoried through text/entity inspection, not a CAD import or accuracy/timing benchmark. No separate verified classification/measurement sheets were found. Nine files have one PRODUCT entity, no standard assembly-occurrence entries and multiple solids, suggesting flattened/multi-body candidates. Physical component grouping needs CAD import/engineer verification; do not assume one solid equals one component.

**Owner update 2026-11-06:** the owner will provide the independent benchmark reference data. Reference-sheet creation is not requested now. Recognition coverage, benchmark results and evidence-based numerical acceptance targets remain pending; the ±0.1% dimensional tolerance is already fixed and must not be relaxed by benchmark results. Other agreed application requirements do not depend on inventing missing reference values.

**Confirmed 2026-11-06 — flattened STEP fallback:** support a flat component review rather than requiring a hierarchy-preserving re-export. Engineers confirm which solids/surfaces belong to each component. Preserve the original imported geometry and structure; store reviewed component-to-geometry associations separately under the product's flat component view. Do not invent a nested assembly hierarchy or silently treat every body as a component. This fallback applies where the supplied data lacks a usable component hierarchy; it does not authorize regrouping an existing customer assembly tree. Link the component view and viewport selections to the associated geometry.

```text
relative_error = abs(extracted_value - reference_value) / abs(reference_value)
pass when relative_error <= 0.001, for nonzero reference values
```

Bend count must match exactly. **Pending benchmark definition:** required extraction coverage and classification accuracy targets. Engineer confirmation is already mandatory; no confidence-based automatic review threshold is needed. Relative error at a zero reference needs a documented validation convention if such a reference occurs; do not use division by zero or conflate unavailable measurements with zero.

### Solids, surfaces, and manual completion

- Accept both solids and surfaces in STEP input.
- Engineers can enter missing measurements manually and continue costing when surfaces prevent reliable extraction.
- **Proposal:** derive volume only from valid solids or reliably closed/repaired shells. Never infer usable volume from an open surface without an explicit supported method.
- **Proposal:** preserve original geometry, record repairs, and distinguish unsupported geometry, extraction failure, and insufficient input.
- **Proposal:** expose manual/extracted source, units, author, time, reason, extraction/model version, and review state. Flag manual values for renewed review when relevant geometry changes.
- **Proposal:** varying-diameter tubes may be completed manually but must not be represented as successfully measured constant-diameter tubes.

### Recognition feedback

**Confirmed 2026-11-06:** automatically recognized component types require engineer confirmation; bulk confirmation is available so correct predictions can be accepted together and exceptions corrected. High model confidence does not automatically approve a classification. Treat unconfirmed classifications as pending engineering review; confirmation is part of completing that review before activation. Confirmed classifications become training examples.

**Proposal:** retain geometry reference, prediction, engineer-confirmed label, and model version; curate examples and use a held-out validation set before deploying new models. Existing projects must not be silently reclassified.

## 8. Catalogs and shared parts

- Catalog matches are suggestions until engineer confirmation. Unconfirmed catalog specifications/prices must not enter costing.
- Retain a versioned catalog reference; record project overrides separately.
- Repeated occurrences share part metadata and part manufacturing routing by default, including across products in the same project.
- Each occurrence retains its hierarchy position, placement, and applicable quantity.
- Parent assembly operations cover installation/joining/testing in that assembly context; do not duplicate them in each part's manufacturing cost.
- Engineer explicitly chooses purchased or internally manufactured for every custom part. That choice is consistent across the project.
- **Proposal:** unresolved sourcing, absent prices, and uncertain catalog compatibility produce incomplete results, not zero costs.
- **Proposal:** purchased parts require price, currency, purchase UOM, and price source. Internally manufactured parts require material/consumption and applicable routing, cost centers, and tooling inputs.
- **Confirmed 2026-11-06:** use customer part number and revision, where available, to propose identity matches across products and deliveries within the project. Geometry validates a proposed match; similar geometry alone must not automatically merge parts. Engineers confirm ambiguous cases, including missing identifiers or conflicting evidence, before work is transferred or edits shared. A changed revision can be associated with the persistent part identity without overwriting its earlier accepted version.
- **Confirmed 2026-11-06 — customer revision coexistence:** different customer revisions of the same part number may coexist as distinct project part versions, with separate metadata, manufacturing routing and demand aggregation. Share engineering edits and coordinate activation among products using the same customer version, not across genuinely distinct customer revisions. Ambiguous or conflicting matches still require engineer confirmation; do not automatically merge versions or invent a customer revision to bypass coordinated activation for an ordinary engineering edit.

## 9. Feasibility

Rules are independent of the selected manufacturing cost center.

| Rule | Required interpretation |
|---|---|
| Minimum bend radius | Compare each centerline radius with the applicable limit |
| Bend spacing | Straight centerline distance between consecutive bend tangent points |
| Tube/connector compatibility | Compare tube dimensions with connector requirements |
| End assembly length | Straight tube length available for fir-tree or rubber-barb connector assembly |

End assembly requirements come from confirmed connector catalog specifications, with configurable rules for custom connectors.

An authorized engineer may accept an exception with a recorded justification. **Proposal:** retain failed-rule status, measurements, rule version, author and timestamp; exception acceptance resolves disposition, not geometric compliance. Reopen exceptions if their supporting inputs change.

**Rule-table mapping recommendation:** select applicable rows using material, OD, wall thickness and connector attributes as required by the engineering source. Distinguish insertion depth from additional assembly clearance if the source requires both. Applicability/limits are engineering data to populate, not permission to invent rules. Missing-limit and conflicting-rule behavior is confirmed below; no hidden precedence remains to be designed.

**Confirmed 2026-11-06:** configure MVP feasibility rules through structured tables with defined parameters, applicability ranges, and limits, rather than a general formula/conditional rule builder. Supported examples include minimum bend radius expressed as a multiple of OD, minimum straight length between bends, and required connector assembly length. Numerical thresholds must come from engineering data; do not invent defaults. New rule types can be added when a concrete need is identified.

**Confirmed 2026-11-06 — initial rule source:** translate the owner's existing engineering standards/tables into the structured rule tables. Do not create a speculative starter rule set. Supplied files have been reviewed as documented below; additional limits absent from those files remain engineering data prerequisites. Retain source/revision references when importing the rules.

**Source review 2026-11-06:** the owner supplied `design rules/`. See [DESIGN_RULES_REFERENCE.md](DESIGN_RULES_REFERENCE.md) for the reviewed inventory, transcribed radius tables and applicability conditions. Smooth-tube radii are limited to standard PA12 with supporting spring; corrugated nominal sizes refer to connection-area ID. Do not generalize either table to unsupported geometry/materials. Material designation and tube manufacturing tolerance references do not replace extraction accuracy criteria or provide connector assembly/spacing limits. Initial numerical bend-radius data is available; bend-spacing and connector compatibility/end-length data are still absent from this folder.

**Confirmed 2026-11-06 — missing numerical limits:** retain the agreed checks in MVP. Administrators can enter engineering-approved limits in structured rule tables with source references. Until a required limit is supplied, mark its check unresolved/not evaluated, never passed. The recorded-justification exception workflow remains available. Missing source data does not authorize invented default limits or automatic interpolation.

**Confirmed 2026-11-06 — rule conflicts:** when overlapping applicable rules specify conflicting limits, flag the conflict for an Administrator to resolve. Do not silently select a limit. No rule-priority/override system in MVP. Keep the affected check unresolved unless the conflict is resolved or the agreed justified-exception process is used.

## 10. Manufacturing and setup

- Routing templates can apply at part, subassembly and product-root levels.
- Applied routings can be edited independently without changing the shared template.
- **Proposal:** instantiate versioned template content as a local routing with source/template version retained. Shared-template updates are offered for adoption, never applied silently.
- Part routing edits propagate to that shared part's occurrences/products, subject to active revision preservation.
- **Proposal:** operations record sequence, cost center, calculated/manual cycle time, explicit cycle basis, applicable cavities/tooling and input provenance. Avoid legacy implicit UOM-dependent multiplication.
- Setup uses duration and the cost center dedicated to setup, separately from production cycle time/cost center:

```text
setup_charge = setup_duration_hours * setup_cost_center_hourly_rate
```

- Report setup as a separate charge alongside unit manufacturing cost. Do not amortize it over production batches in MVP.
- **Confirmed 2026-11-06:** count each shared part routing's setup once per project quotation, even when several products use that part. Sum the setup costs of its applicable operations once. Assembly-specific setups are counted separately at their applicable assembly level. This is a quotation assumption, not a production schedule or a batch-frequency calculation. Product-level display allocation, if needed, must not duplicate the project charge.

## 11. Demand and manufacturing cost baseline

### Project volume policy

The project selects the representative costing volume method: average annual lifetime volume, maximum annual volume, median annual volume, or later defined alternatives. Quotation scenarios cannot override this method.

For shared part c and calendar year y:

```text
part_demand[c,y] = sum(product_volume[p,y] * expanded_quantity[c,p])
costing_volume[c] = project_volume_method(part_demand[c, applicable_years])
lifetime_demand[c] = sum(part_demand[c, applicable_years])
```

Use quoted products, expand quantities through all assembly levels, and sum repeated occurrences. Aggregate by calendar year **before** applying average, maximum or median. Do not sum individual product peaks. Actual annual forecasts remain separate from representative costing volume.

**Confirmed 2026-11-06 — forecast years:** every calendar year from a product's SOP through EOP inclusive requires a positive forecast. Zero-volume production years are not allowed in MVP; missing required values remain incomplete. Exclude years outside the product's schedule rather than padding them with zero. Build each shared part's annual demand from products whose production schedules cover that year; apply average/maximum/median to years with applicable demand. Do not introduce zero rows in gaps where no applicable product is in production. Forecast horizon follows the declared product schedules, not the workbook's fixed ten-year limit.

**Confirmed 2026-11-06 — purchase price breaks:** support optional price-break tables with minimum annual quantities and unit prices. Use the part's calculated representative annual costing volume to select the applicable price: the row with the greatest minimum quantity not exceeding that volume. Support a single price where no breaks exist. If no row covers the calculated volume and no applicable single price is available, flag missing pricing rather than selecting an unsupported tier. Retain the selected price source/version and engineer-confirmed catalog association where applicable. This does not add supplier integration or purchasing workflows.

Costing volume is not the lifetime tooling recovery denominator. **Confirmed 2026-11-06:** keep the selected routing's cycle times and hourly rates fixed; volume does not automatically select different manufacturing assumptions. Engineers may explicitly revise routing assumptions. No manufacturing cost-volume curves or volume-selected cycle-time rules in MVP. Purchase price breaks and tooling recovery retain their agreed volume behavior. Geometry-driven cycle formulas may still change when their geometric inputs change; this decision excludes volume-driven switching, not geometry-driven calculation.

### Calculation principles

**Confirmed 2026-11-06 — currencies:** one costing currency per project. Retain supplier prices in their original currencies and convert using manually maintained, saved project exchange rates. Define each rate explicitly as project-currency units per one source-currency unit; converted amount = source amount * rate. Require a positive rate for a different currency; same-currency conversion is identity. Calculation snapshots retain the rates used. No live exchange-rate integration in MVP, and editing rates must not rewrite saved results.

**Confirmed 2026-11-06 — cost-center rate policy:** use the selected versioned hourly rate without automatic annual inflation or midlife adjustment. Keep that rate in the cost baseline until the user explicitly adopts updated rates and recalculates. This applies to production and dedicated setup cost-center rates. Annual selling-price productivity does not change these rates.

**Confirmed 2026-11-06 — machine investment:** reflect machine depreciation through cost-center hourly rates where applicable. Separate project machine/equipment CAPEX calculation, allocation and amortization are outside MVP. Do not additionally charge the same depreciation through a project investment line. This exclusion does not remove the separately agreed tooling investment and recovery functionality.

**Confirmed 2026-11-06 — material allowance:** each internally manufactured part supports a simple material allowance percentage, including zero where unnecessary. Costed material consumption = net required consumption * (1 + allowance rate); 2% gives a multiplier of 1.02. This increases material cost only, not BOM quantities, finished-part demand, production cycle time, setup, or tooling recovery quantity. It is an allowance on net consumption, not an operation yield/rejection model. Detailed losses by operation are outside MVP.

**Proposal:** deterministic calculation functions, explicit units/currency/UOM conversion, Decimal money and rates, no intermediate display rounding, versioned inputs and line-level explanations. Preserve true zero separately from missing/not applicable.

Roll child cost into parent once, then add that parent's own operations and applicable categories once. Shared part unit cost is reused with occurrence quantity; do not charge its entire project demand to each product. Tool investments require unique identities and project-level deduplication.

**Confirmed 2026-11-06 — non-CAD cost additions:** engineers may attach additional cost lines to a part or assembly for packaging, adhesive, consumables or other items absent from CAD. Include them in applicable costing and internal exports, without adding or restructuring CAD/BOM nodes. Identify these lines separately from geometry-derived components. Their cost category and explicit quantity/unit basis determine their contribution; do not assume a separate one-time charge when an item is consumed per part or assembly.

**Confirmed 2026-11-06 (O14):** a purchased assembly's price covers its children. Preserve the complete CAD hierarchy, but stop costing/demand expansion at the purchased assembly: descendants in that branch generate no additional material/manufacturing costs, shared-part demand, or tooling recovery. Own installation/testing operations may still be added at the applicable assembly level. The same part used elsewhere outside a purchased branch still contributes demand/cost normally; exclusion is occurrence-context-specific, not a global deletion of the part.

**Confirmed 2026-11-06 — net material consumption:** for internally manufactured parts, use geometry by default: tube OD/ID and developed centerline length determine annular material volume; other components use their extracted solid/enclosed volume. Convert volume to mass using the selected material density, then apply price per kg with explicit unit conversion. Allow the engineer to override net consumption when needed, retaining its source/unit. Apply the material allowance afterward, once. Missing usable geometry may be completed manually as already agreed; never substitute zero consumption.

Retain compatible cavity/time and operation formulas from the handover subject to section 14. The material formula is not restricted to the workbook's legacy QC category. More specialized consumption models, such as multi-material breakdowns, require separate scope rather than being silently inferred.

## 12. Pricing and quotation scenarios

### Settings ownership

| Level | Settings |
|---|---|
| App | Configurable cost categories used to calculate EBIT; versioned configuration |
| Project | EBIT target, costing volume policy, default tooling charge/amortization mode, productivity requirement |
| Scenario | Named commercial alternative, manual product prices, tooling recovery assumptions, whether productivity applies to tooling recovery |

The project EBIT target is shared by every scenario. Scenarios cannot override the volume method. **Confirmed 2026-11-06:** each scenario starts from the project's default tooling mode and may independently select separate charging or unit-price amortization. Saved scenarios can compare those alternatives side by side.

### Always-visible prices and EBIT

Every product shows suggested selling price based on the project EBIT target, manually entered selling price, and EBIT calculated from that manual price. Suggested price remains visible during negotiation. Blank manual price means uncalculated manual-price EBIT.

**Confirmed 2026-11-06:** EBIT beside the unit selling price represents recurring product profitability. Its revenue basis is the base unit selling price; its cost basis is the app-configured operating-cost categories. Exclude tooling recovery revenue/investment and separate setup revenue/cost from this recurring unit EBIT and target-price calculation. Show tooling recovery and setup charges separately; do not spread them over production to create a combined EBIT in MVP. Configurable operating categories must respect this boundary.

**Confirmed 2026-11-06:** the manually entered price is the base unit selling price, excluding tooling. In amortized mode, the app adds the tooling contribution and displays the total customer unit price alongside the base price. Separately charged tooling remains outside the unit price. Apply annual productivity separately to the base and tooling contributions according to the scenario's agreed treatment.

**Confirmed 2026-11-06 — category calculations:** MVP cost categories use defined calculation types: calculated direct cost from supported engines, a fixed amount per unit, or a percentage of a defined cost subtotal. App configuration selects the operating categories included in EBIT. No arbitrary custom formulas or formula language. Percentage bases are cost subtotals, not selling price; they must be explicitly defined and must not create circular or duplicate inclusion.

**Confirmed 2026-11-06 — target period:** the suggested starting base selling price achieves the project EBIT target in the product's first production year. Later-year projections show the EBIT effect of annual productivity reductions; do not solve for a lifetime volume-weighted target in MVP.

Calculation contract: with included recurring unit cost C and target EBIT rate t (margin on base selling price), suggested first-year base price = C / (1-t); manual-price EBIT amount = manual base price - C, and rate = (manual base price - C) / manual base price. Display amount and rate distinctly. Require a positive manual base price to calculate the rate and a target rate below 100%. The defined cost types above do not require a general price solver. Tooling/setup remain excluded as agreed.

### Annual productivity

Productivity applies only to selling price, not assumed manufacturing cost savings. It starts after each product's own first production year and compounds on the previous year's selling price:

```text
base_selling_price[p,n] = first_year_base_price[p] * (1-rate) ** (n-1)
```

Example: rate 3%, first-year base price 100 gives 100, 97, 94.09. Use actual annual product forecasts for revenue, not representative costing volume.

**Confirmed 2026-11-06:** products inherit project SOP/EOP defaults, and engineers may override either schedule at product level. SOP/EOP use calendar years only in MVP. Enter production volumes by calendar year, including actual forecast quantities for partial launch/end years, and align shared-component demand on those calendar years. Do not automatically prorate quantities or prices by month. Each product's first production calendar year is Year 1 for pricing; its compounded annual productivity reduction starts in the next production year.

### Tooling alternatives and recovery

- Always make separate-charge and amortized-unit-price alternatives accessible for comparison; switching must promptly update the commercial view without changing tooling inputs.
- Show investment, recovery basis/quantity, and amortized contribution at component level, with product/project rollups.
- Recovery basis supports combined lifetime demand and manually entered recovery quantity. Negotiation scenarios retain those assumptions.
- **Confirmed 2026-11-06 — assembly tooling:** use the same recovery options for assembly-level tools: lifetime demand or a manually entered recovery quantity, selectable per scenario. Expand subassembly occurrences through the BOM; two occurrences per finished product contribute twice that product's lifetime volume. Product-root tooling uses the product's lifetime volume. Shared-assembly investment allocation follows its lifetime-demand shares just like shared-component tooling. Respect purchased-assembly costing boundaries.
- Tooling investment = tool count * cost per tool. Count a shared tool investment once.
- **Confirmed 2026-11-06 — tooling ownership:** each tooling entry belongs to one part or assembly definition and is shared across products using that definition. One entry cannot span several different part definitions in MVP. Multi-part/family-tool investment splitting is deferred; do not duplicate one physical investment as several full-value tooling entries to simulate that behavior.
- **Confirmed 2026-11-06:** allocate shared-component tooling investment to quoted products in proportion to their lifetime demand for that component. Product allocation share = product lifetime component demand / combined lifetime component demand. Use quantities expanded through the BOM and respect purchased-assembly exclusions. This allocation explains product investment shares; it does not multiply the project investment or replace the scenario's selected recovery quantity for calculating the amortized contribution. If combined lifetime demand is missing or zero, mark allocation incomplete rather than inventing percentages.
- Selected quotation treatment must avoid recovering the same investment twice through both a full separate charge and full amortization.
- Scenario selects whether annual productivity applies to the tooling contribution:

```text
tooling_contribution[n] = initial_contribution * (1-rate) ** (n-1)  # applies
tooling_contribution[n] = initial_contribution                       # exempt
unit_selling_price[n] = base_selling_price[n] + tooling_contribution[n]
```

- **Proposal:** initial component contribution = assigned tooling recovery amount / selected positive recovery quantity. Convert it to product contribution using applicable part quantities.
- **Confirmed 2026-11-06:** retain the quoted tooling contribution throughout the planned production period, subject to the scenario's productivity treatment. Reaching the recovery quantity does not automatically stop or cap the contribution. Show projected under/over-recovery against investment using the entered forecasts; changing the quoted contribution requires a revised quotation scenario. Actual production/sales tracking is not required for this behavior in MVP.
- Suggested price is a base unit price, comparable with the manually entered base price; show any amortized tooling contribution separately and add it to display the total. Assembly tooling uses the confirmed recovery rules above. Multi-part tooling ownership is excluded from MVP. Recurring EBIT excludes tooling/setup as confirmed above.

### Saved comparisons

Allow multiple named scenarios and side-by-side comparison. **Proposal:** compare yearly selling prices, EBIT, revenue, tooling investment/recovery and separate charges; reference a common baseline version and flag stale scenarios. Do not present comparisons from different baselines as equivalent without explicit labeling.

## 13. Shared updates, audit, and exports

**Confirmed 2026-11-06 — permissions:** two simple permission sets in MVP: Engineer for project work, and Administrator for shared catalogs, cost centers, feasibility rules, routing templates and app configuration. One user can hold both. Engineers can activate their own prepared revisions and accept feasibility exceptions with recorded justification; no extra approval workflow is introduced. Shared-data maintenance does not bypass project notification/explicit-adoption rules.

Changes to app configuration, catalog prices/specifications, cost-center rates, feasibility rules and routing templates flag affected existing projects. Explain the impact; do not automatically adopt changes or recalculate. User decides whether to update. Preserve historical versions/results.

**Proposal:** dependency tracking records both old and available versions, affected parts/products/scenarios, acknowledgement and adoption status. Reading a notification is not adopting an update. Explicit recalculation creates a new snapshot; it does not rewrite history.

Internal-only Excel and PDF exports are required. **Proposal:** include selected scope, CAD BOM, reviewed measurements, sourcing/catalog provenance, routings, feasibility exceptions, cost breakdowns, annual prices/EBIT, scenario comparisons, unresolved issues, revision IDs and calculation/configuration versions. Mark partial or stale exports clearly. Customer quotation generation is excluded.

**Confirmed 2026-11-06:** allow exports of unfinished work for internal review, clearly labelled Draft / Incomplete. Include missing inputs and unresolved findings. Outdated costs must be identified as outdated, not presented as current. Export need not wait for activation. Exact visual templates remain a presentation design choice; this does not authorize customer-facing outputs.

**Confirmed 2026-11-06 — export profiles:** Excel carries detailed BOM, metadata, routings, feasibility, cost details and quotation scenarios. PDF is an internal summary of costs, annual prices/EBIT, tooling/setup charges and unresolved issues, rather than a repetition of all engineering tables. Both follow the agreed draft/incomplete and stale-result labeling rules. Exact page styling and workbook sheet layout are implementation/design choices, not additional product-policy blockers.

## 14. Relationship to the existing costing handover

| Reference | Retained use | Superseded or restricted behavior |
|---|---|---|
| [00_HANDOVER.md](00_HANDOVER.md) | Formula map, units, deterministic calculation concepts | Its MVP list and service outline do not define the complete app scope |
| [01_DOMAIN_MODEL_AND_WORKFLOWS.md](01_DOMAIN_MODEL_AND_WORKFLOWS.md) | Materials, cost centers, operation standards, stable identities | Product ownership includes package revisions, CAD hierarchy, shared parts and snapshots; forecast horizon follows SOP/EOP, not a fixed ten years |
| [02_MANUFACTURING_COST_ENGINE.md](02_MANUFACTURING_COST_ENGINE.md) | Annular tube material formula, density/price conversion, cycle/hourly rate arithmetic, count-times-unit tooling cost | Do not copy Year1*(EOP-SOP) tooling denominator, implicit purchase-price precedence, setup omission, or automatic tooling inclusion in manufacturing cost |
| [03_PRICING_AND_PROJECT_ANALYSIS.md](03_PRICING_AND_PROJECT_ANALYSIS.md) | Explainable price/EBIT concepts and compatible arithmetic | One-time productivity uplift is replaced by annual selling-price reductions; EBIT categories configurable; manual selling price always present; flat lifetime selling price is superseded |
| [04_PARITY_DEFECTS_AND_TEST_PLAN.md](04_PARITY_DEFECTS_AND_TEST_PLAN.md) | Defect evidence, compatible golden vectors, validation examples | Golden expectations involving superseded pricing/tooling rules need revised vectors; parity mode is not a confirmed app requirement |

Do not inherit workbook sample values (5% overhead, 89% production SG&A, 20% margin, 12% uplift) as confirmed app defaults. The rate policy is now confirmed: selected versioned rate, without automatic inflation/midlife adjustment. Machine depreciation is covered by applicable cost-center rates; separate project CAPEX calculation is excluded from MVP. Currency conversion uses saved manual project rates expressed as project-currency units per source-currency unit, not the ambiguous workbook division. New requirements take precedence over strict workbook parity.

## 15. Decision status and pending inputs

Audit as of 2026-11-06, revised for desktop delivery: the confirmed business behavior in O01–O06, O08–O11 and O13–O14 is retained. O07 retains evidence-dependent acceptance details, deliberately deferred until independent references and real benchmarks are available. O12 now also records desktop-first delivery, the confirmed prototype execution/save split and remaining desktop operation choices. This is not evidence that CAD accuracy/performance has been demonstrated. Missing numerical feasibility limits are an engineering-data prerequisite under the agreed entry/exception workflow, not an unresolved application policy.

Technical details still labelled Proposal remain recommendations for the consolidated design review; the decision table does not silently approve every proposal. Most concern implementation/audit mechanics, not new business alternatives. In particular, numerical interaction targets (including tooling mode switching) and any zero-reference validation convention still need to be specified in the technical acceptance plan. No need to invent additional business workflows to settle them.

| ID | Decision needed | Why it matters |
|---|---|---|
| O01 | Resolved: recurring EBIT excludes tooling/setup; defined cost types and bases; suggested starting base price achieves project EBIT-rate target in each product's first production year | Defines suggested-price and EBIT arithmetic without a custom formula language |
| O02 | Resolved: entered selling price excludes tooling; contribution continues without automatic cap/stop, with projected under/over-recovery shown; each scenario may override the project default tooling mode | Prevents double recovery and ambiguous annual prices |
| O03 | Resolved: one part/assembly owns each tooling entry; allocate investment by lifetime demand shares; lifetime or manual recovery quantity also applies to assembly tools; multi-part tools deferred | Determines component/product totals without duplicated investment |
| O04 | Resolved for MVP: shared part routing setup counted once per project quotation; assembly-specific setups counted separately; no batch-frequency model | Prevents duplicate setup charges across products |
| O05 | Resolved: customer number/revision proposes identity, geometry validates, engineer confirms ambiguity; distinct customer revisions may coexist with separate definitions/demand, sharing and coordinated activation within each version | Determines safe reuse across products and customer deliveries |
| O06 | Resolved: coordinated shared-part activation; complete costs/resolved or justified findings required; selling price not required; forecast/scope edits automatically recalculate dependent project draft costs and flag scenarios, preserving accepted results | Preserves coherent active results when dependencies change |
| O07 | Partially resolved: engineer confirmation; benchmark-first with ±0.1% tolerance; ten files inventoried; flattened inputs use flat engineer-confirmed component grouping while preserving original structure. Remaining: independent reference labels/measurements, benchmark evidence/targets and zero-reference handling | Makes accurate recognition/extraction objectively testable |
| O08 | Resolved behavior: supplied radius tables plus Administrator-entered approved limits with sources; missing/conflicting limits remain unresolved; Administrator resolves conflicts, no hidden precedence; justified exceptions allowed. Remaining numerical data entry is a data prerequisite, not an undefined policy | Defines actual engineering checks rather than their headings |
| O09 | Resolved: project SOP/EOP defaults with product overrides; calendar years only; positive forecast for each SOP–EOP year inclusive; no zero padding or monthly proration; horizon follows schedule | Defines annual aggregation and productivity timing |
| O10 | Resolved: fixed versioned rates/cycle assumptions without volume-driven switching; depreciation through rates, separate CAPEX excluded; purchase price breaks; saved manual FX; geometry-based consumption with manual override, then material allowance | Completes deterministic costing beyond inherited workbook examples |
| O11 | Resolved: detailed internal Excel and summary PDF; unfinished exports allowed with Draft / Incomplete labels, missing inputs/findings and stale-cost identification; visual layout left to design | Defines reviewable Excel/PDF output |
| O12 | Revised: desktop-first SolidCraft fork; single-organization central project/shared storage, backend costing and Engineer/Administrator permissions retained. Prototype local CAD import/measurements/background analysis, online-only writes, current-session retention of unsaved edits and separate central project/local CAD saves are confirmed. The first prototype targets macOS. Read-only imported geometry is confirmed for the prototype, with geometry changes through new STEP revisions. A locally launched macOS development build is confirmed; signed installers, automatic updates and production distribution are deferred. Prototype scope and P01–P14 are approved as detailed in DESKTOP_PROTOTYPE_SPEC.md; architecture/minimum OS version remain technical planning details. Broader MVP benchmark evidence/targets remain pending; the prototype one-second UI-response target is approved but unmeasured. Prior ten-file inventory is historical evidence only | Establishes desktop delivery without silently choosing standalone storage, local costing or unverified CAD performance |
| O13 | Resolved for MVP product scope: attached non-CAD cost lines; combinable Engineer/Administrator permissions; exclude/reinstate quotation products while retaining all engineering history. Destructive deletion and broader project archival are not required by this scope | Completes lifecycle without violating CAD hierarchy |
| O14 | Resolved: purchased parent price includes descendants; retain CAD tree but stop descendant cost, demand and tooling expansion in that branch | Prevents charging purchased parent price plus the same children again |

## 16. Acceptance scenarios

These examples inherit the confirmation status of the requirements they exercise. Clauses testing a **Proposal** remain proposed acceptance criteria until owner approval; they do not promote that proposal to a confirmed requirement. In particular: provenance in 8, reopening exceptions in 10, the contribution formula in 15, baseline metadata in 19, and specific export metadata in 20 are proposed details. Formulas are illustrative where an open decision is identified.

1. **Partial delivery:** Package 1 delivers A/B/C; Package 2 revises B and adds D. A/C stay active and unchanged. B stays on its old active revision until explicit activation. D is a new draft.
2. **Partial analysis failure:** A succeeds and B fails extraction. A can be reviewed; B can be retried or manually completed without losing A's work.
3. **Preserved review:** a changed tube length retains prior engineering work but highlights affected consumption/routing/feasibility inputs for review.
4. **Shared part:** a part appears twice inside a subassembly used three times. Expanded demand is six parts per finished product, and shared metadata/routing are edited once.
5. **Sourcing consistency:** a sourcing change affects A and B. Activating only A is rejected; a valid coordinated activation changes both or neither.
6. **CAD fidelity:** tree/viewport selection stays linked; metadata edits do not regroup or reparent an existing customer hierarchy. For flattened inputs, reviewed flat component-to-geometry associations preserve the original import and do not invent nested assemblies.
7. **Measurement:** reference length 1,000 mm accepts 999–1,001 mm inclusive; bend count must be exact; each radius is tested individually.
8. **Surface fallback:** volume unavailable on open surfaces is missing, not zero; an engineer-entered measurement with provenance enables continued costing when other requirements are met.
9. **Catalog confirmation:** a proposed connector match cannot provide costing price before confirmation. A project override preserves the original catalog version/value.
10. **Feasibility:** spacing is measured tangent-to-tangent. A justified exception remains visible and becomes review-required if supporting geometry changes.
11. **Demand order:** product A demands 100/200 and B demands 200/100 in the same years. Combined annual demand is 300/300; peak costing demand is 300, not 400.
12. **Missing forecast:** absent forecast is not silently converted to zero. Volume policy is project-owned and cannot be changed in a scenario.
13. **Setup:** 30 minutes at 120 currency units/hour produces a separate 60-unit setup charge, without batch assumptions or addition to per-piece manufacturing cost.
14. **Productivity:** 100 at 3% gives 100/97/94.09 from the product's first production year. Manufacturing cost does not fall by that rule.
15. **Tooling alternatives:** one 10,000-unit investment with recovery quantity 5,000 contributes 2 per component unit before any scenario productivity. Separate/amortized views retain the same investment and do not double-count it.
16. **Tooling productivity:** with entered base price 100 and tooling contribution 2, the first-year total is 102. Year 2 at 3% is 99 when tooling is exempt, or 98.94 when productivity applies to both. Manual input excludes tooling, as confirmed on 2026-11-06.
17. **Price visibility:** target-EBIT suggestion remains visible beside manual price and its calculated EBIT; blank manual price does not imply zero revenue or a valid EBIT rate.
18. **Shared update:** changing a catalog price or rule flags only dependent projects; no current snapshot changes until user adoption/recalculation.
19. **Scenario integrity:** named scenarios share volume policy and EBIT target; saved comparisons identify their baseline and outdated dependencies.
20. **Export traceability:** Excel/PDF identify active/draft scope, scenario, and calculation versions and preserve the selected snapshot rather than silently using current master data.

## 17. Implementation handoff boundary

This is a behavioral design, not a runnable implementation or a completed API/database contract. Resolve blocking decisions for each workstream before coding that behavior; record resolutions in this document rather than in agent-only assumptions.

**Proposal — boundaries for planning:** project/package revision management; STEP ingestion and geometry jobs; measurement/classification review; shared part/catalog management; feasibility rules; routing/master data; pure costing; commercial scenarios; version/dependency tracking; exports.

Use the SolidCraft desktop fork as the selected client base, including its existing Rust/egui integration conventions. Leave backend framework, database schema, job technology, API naming and deployment mechanics to the technical design and implementation plan. A change of CAD kernel requires validation rather than an assumption that all required extraction exists. The handover suggests Python for the backend; that backend stack choice is not newly confirmed by this specification.

After owner review of this specification, prepare a technical plan with entity/version contracts, state transitions, API/job contracts, dependency/invalidation behavior, calculation formulas and acceptance tests. Do not treat open decisions or proposals as already approved.


## 18. Desktop reconciliation and prototype boundary

The desktop preference changes delivery, interaction and integration requirements. It does not change confirmed costing or engineering policy.

| Affected area | Reconciled requirement | Status |
|---|---|---|
| Entry point and installation | SolidCraft desktop fork with linked SolidBOM panels; browser-first/no-workstation-installation requirement superseded | Confirmed direction |
| STEP intake | Native file open/import participates in package/product revision ownership and source preservation | Confirmed behavior; transfer mechanics open |
| Display and selection | Reuse the existing CAD tree/viewport; maintain mapping to reviewed parts and occurrences | Confirmed direction; technical contract proposed |
| CAD analysis | Prototype import/measurements run locally; long-running analysis runs in the background, with source/version binding | Confirmed prototype split; accuracy rules unchanged |
| Business data and costing | Central project/shared data and backend calculations continue | Retained confirmed requirements |
| Save/reopen and failures | Central project save is distinct from local CAD-file save; online-only writes; retain unsaved form edits in-session after failure and allow explicit retry; no silent synchronization | Confirmed prototype behavior; storage/write mechanics remain technical design |
| CAD editing | Prototype imported geometry is read-only across command paths; selection, measurement, annotations and engineering/costing edits remain available; geometry changes arrive through a new STEP revision | Confirmed prototype policy; existing business constraints retained |
| Platform and distribution | macOS development build launched locally; signed installers, automatic updates and production distribution deferred; architecture/minimum OS version require technical planning | Confirmed prototype delivery; production delivery remains future planning |
| Browser | Defer the browser client from initial delivery | Confirmed priority |

Sections 6–14 retain the confirmed rules for packages/revisions, mandatory engineer confirmation, ±0.1% measurements and exact bend counts, manual completion, hierarchy and flat grouping, part identity, sourcing, feasibility, routings, setup, shared demand, currencies, material allowance, tooling, EBIT, annual productivity, explicit adoption and internal exports. Existing acceptance scenarios in section 16 remain applicable. A prototype omission is not a removal from MVP scope.

[DESKTOP_PROTOTYPE_SPEC.md](DESKTOP_PROTOTYPE_SPEC.md) records the owner-approved prototype workflow, scope and measurable checks P01–P14. D01–D07 are resolved for the prototype. Fixture values are test inputs, not company defaults or changes to confirmed business rules. Prototype completion would not establish full MVP readiness, tube-recognition coverage or customer-file accuracy. The next artifact is a technical implementation plan; remaining implementation mechanisms require design, and no prototype check has run. The instruction not to implement remains in effect.

The earlier [SOLIDCRAFT_EXTENSION_DRAFT.md](SOLIDCRAFT_EXTENSION_DRAFT.md) remains evidence from the repository analysis. Its browser-first recommendation is superseded by this section.
