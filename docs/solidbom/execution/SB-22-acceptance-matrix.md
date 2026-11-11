# SB-22 acceptance matrix — pass / fail / unrun

Requirement-to-test/evidence matrix for the confirmed MVP obligations, application scenarios A01–A20,
prototype checks P01–P14 and the SB-22 scope/exclusion checks. Compiled from the evidence run on
2026-11-12 (Asia/Shanghai) at branch `agent/mvp-acceptance-handoff-262f3c774bf5`, integrated code
`4a8a811fb` (SB-09–SB-21 and SB-X01 merged), evidence recorded at commit `eb710e99b` (the contract
commit is `8d9cd9296`).

This matrix is a record of what was observed, not a readiness claim. The MVP acceptance gate remains
**Pending**: see [Verdict](#verdict-and-remaining-prerequisites). Result semantics and the evidence
rules are in [the acceptance contract](../contracts/v2/mvp-acceptance-and-operating-handoff.md) §2–§3.

Environment: macOS 26.7.1 (25G241), Apple M5 Pro 15-core, 24 GiB, arm64, Metal; rustc/cargo 1.99.1;
PostgreSQL 18.6 on loopback; service migrations 0001–0013 applied at start; service binary SHA-256
`1aa18cdc8a7631cf55f97af371ed8354c6b837ea85d55860800f979adc14fef2`; desktop binary `target/debug/solidcraft`
SHA-256 `53747397063175a23912e7aac8e92cdb3ac8772f2f37824ed884455a94f47fe6`
(`SolidCraft 0.40.0-rc.2 (8d9cd9296)`); fixtures `tests/solidbom/fixtures/*.step` match
`tests/solidbom/manifest.json`. Deployment boundary: contract §1.

Result column: **Pass** = confirmed observation produced against the integrated build; **Partial** =
a passed observation plus a named unrun/limited remainder; **Unrun** = not executed, blocking
prerequisite named. Evidence column points at the tracked evidence file; the detail and raw
observations are there.

## Summary

| Group | Pass | Partial | Unrun | Total |
|---|---|---|---|---|
| Application scenarios A01–A20 | 19 | 0 | 1 | 20 |
| Prototype checks P01–P14 | 12 | 2 | 0 | 14 |
| Confirmed obligations R-* | 58 | 6 | 5 | 69 |
| Recovery, permissions and restart (contract §4.3) | 12 | 0 | 0 | 12 |
| Required checks (contract §4.6) | 2 | 2 | 0 | 4 |
| Independent dimensional/recognition evidence (contract §4.4) | 0 | 0 | 1 | 1 |
| Scope and exclusion checks (D-01–D-03) | 3 | 0 | 0 | 3 |
| Operating handoff procedures (contract §5) | 22 | 0 | 3 | 25 |

The two desktop prototype partials are the SB-08 gaps unchanged by integration (P10 part-number/
revision metadata path; P13 analysis-delay selection/navigation). The unrun dimensional/recognition
rows are the missing owner reference inputs, not an implementation result.

## Application scenarios A01–A20

| ID | Result | Level | Observation | Evidence |
|---|---|---|---|---|
| A01 Partial delivery | Pass | service HTTP | Package 2 revised B and added D; A and C stayed byte-identical; B stayed on its old active revision until explicit activation; D is a draft. | `SB-22-service-lifecycle.txt` |
| A02 Partial analysis failure | Pass | service HTTP | A's accepted review byte-identical after B's failed analysis registration (422, no version appended); retry succeeded and B was manually completed; desktop extraction-failure presentation is the unrun remainder (SB-06/SB-11 desktop increment). | `SB-22-service-engineering.txt` |
| A03 Preserved review | Pass | service HTTP | Prior engineering/manufacturing/exceptions retained; changed geometry required part-match confirmation; consumption review pending and superseded feasibility decisions became `review_required`. No geometry-derived routing flag exists (SB-15 deferral). | `SB-22-service-engineering.txt` |
| A04 Shared part | Pass | service HTTP | 3 subassembly occurrences x 2 part occurrences = 6; one shared part line with annual demand/costing volume 600; one routing edit reached all six occurrences (project total 10594.8 → 11194.8, recomputed). | `SB-22-service-engineering.txt` |
| A05 Sourcing consistency | Pass | service HTTP | A-only activation after the shared material change refused `409 coordinated_activation_required` naming B and wrote nothing; coordinated A+B activation changed both in one transaction. | `SB-22-service-lifecycle.txt` |
| A06 CAD fidelity | Pass | desktop capture | Tree and viewport selections resolve the three occurrences to the shared record before and after a metadata edit; hierarchy fingerprint unchanged; flat import preserved. | `SB-22-desktop-acceptance.txt`, `SB-22-a06-*.png` |
| A07 Measurement tolerance | **Unrun** | — | Needs independently verified owner STEP references, exact bend counts and per-radius references; `check --require-references` exits 1 with 53 missing reference files and the benchmarking folder is absent. Analytic fixture rows exist but are not independent customer references. | `SB-22-service-engineering.txt`; `SB-22-operating-handoff.txt` |
| A08 Surface fallback | Pass | service HTTP | Volume `null` with `missing_reviewed_volume`; incomplete with `missing_consumption`; confirmed manual 4321 mm³ (`manual`, `mm3`, reason, reviewer) made costing complete (material 0.23800068, unit 1.65800068, project total 16.5800068, recomputed). | `SB-22-service-engineering.txt` |
| A09 Catalog confirmation | Pass | service HTTP | Confidence-1 candidate still produced `missing_purchase_price`; confirmation costed 0.42; override 0.55 preserved separately with confirmed version 1/0.42 also after catalog v2 0.30. Open defect D1: the override does not enter costing (SB-16). | `SB-22-service-engineering.txt` |
| A10 Feasibility | Pass | service HTTP | Spacing uses `tangent_straight_lengths_mm[0]=18` vs required 20; each radius is its own finding; custom-connector checks unresolved without a limit; six justified exceptions active; changed geometry made them `review_required`. | `SB-22-service-engineering.txt` |
| A11 Demand order | Pass | service HTTP | Annual demand 300/300, peak 300 (not 400), lifetime 600, costing volume 300. | `SB-22-service-commercial.txt` |
| A12 Missing forecast | Pass | service HTTP | Missing 2028 forecast stays null/incomplete (`missing_forecast`); a scenario payload carrying volume policy/EBIT stores neither and the project policy still applies. | `SB-22-service-commercial.txt` |
| A13 Setup | Pass | service HTTP | 0.5 h x 120/h = 60 separate setup, counted once for a routing shared by two products; excluded from unit cost 6.508 and product total 64.108. | `SB-22-service-commercial.txt` |
| A14 Productivity | Pass | service HTTP | C=80, target 0.2, rate 0.03: 100/97/94.09 from 2027; recurring cost 80 unchanged. | `SB-22-service-commercial.txt` |
| A15 Tooling alternatives | Pass | service HTTP | 10,000 investment / 5,000 recovery → contribution 2; separate and amortized views retain the single 10,000 investment and no double recovery. | `SB-22-service-commercial.txt` |
| A16 Tooling productivity | Pass | service HTTP | Year 1 102; year 2 99 exempt vs 98.94 applied to both (contribution series 2/2/2 vs 2/1.94/1.8818). | `SB-22-service-commercial.txt` |
| A17 Price visibility | Pass | service HTTP | Suggested 100 visible beside manual 125 (EBIT 45 / rate 0.36); blank manual price → null EBIT/rate and null revenue, not zero. Desktop rendering not built (later SB-18 increment). | `SB-22-service-commercial.txt` |
| A18 Shared update | Pass | service HTTP | Material/catalog/rule publishes flagged only dependent P1; all stored snapshots byte-identical until explicit adoption/recalculation; accepted history unchanged. | `SB-22-service-lifecycle.txt` |
| A19 Scenario integrity | Pass | service HTTP | Comparison reports shared average/0.2/0.03 for five scenarios; forecast edit flags `scenario_baseline_outdated`/`scenario_project_outdated`; newer-draft comparison adds `scenario_baseline_differs`. | `SB-22-service-commercial.txt` |
| A20 Export traceability | Pass | service HTTP | Accepted-baseline XLSX+PDF bytes preserved pinned material price 20 (allowed cost 0.60588 recomputed) instead of current master 24 (0.727056), and named scope/versions/calculation id/input hash; draft and incomplete exports labelled. | `SB-22-service-lifecycle.txt` |

## Prototype checks P01–P14 (rerun on the integrated build)

| ID | Result | Observation | Capture |
|---|---|---|---|
| P01 Import and source | Pass | Source SHA-256 unchanged; 1 definition / 3 occurrences match manifest quantities and transforms; part-version formula equal; hierarchy preserved. | `SB-22-p01-1-after-import.png` |
| P02 Selection | Pass | Six real selections (3 tree, 3 viewport) resolve to the correct occurrence and one shared part/version; one edit shows at all three; hierarchy unchanged. | `SB-22-p02-1-tree-selection.png`, `-2-viewport-selection.png` |
| P03 Volume and units | Pass | 2400 mm³ within ±2.4, source `extracted`, one definition with quantity 3, not charged three times. | `SB-22-p03-1-volume-and-units.png` |
| P04 Review | Pass | Classification pending until confirmed; explicit manufactured sourcing and material; ten named missing inputs, incomplete, no subtotal, no default-mass substitution. | `SB-22-p04-1-review-incomplete.png` |
| P05 Arithmetic | Pass | 1.1388016 / 3.4164048 / 60.0 with seven explained lines; independent Decimal difference 0 within 1e-7 on the actual imported measurement. | `SB-22-p05-1-cost-explanation.png` |
| P06 Save/reopen | Pass | Saved version 3; complete process quit/restart; reopen by project id recovered 14/14 facts; recalculation is a new immutable result and the earlier snapshot is unchanged. | `SB-22-p06-1-reopened-current.png` |
| P07 Invalidation | Pass | 30→60 s marks the prior result stale; recalculated 2.1388016 / 6.4164048 within 1e-7; earlier result readable with its original inputs. | `SB-22-p07-1-stale-after-edit.png`, `-2-earlier-result-readable.png` |
| P08 Missing data | Pass | Named `missing_cycle_time`/`missing_density`/`missing_material_price` with no complete subtotal; explicit zero cycle is current with production 0 and 0.1388016 per part, distinct from missing. | `SB-22-p08-1-zero-vs-missing.png` |
| P09 Surfaces/manual values | Pass (display residual) | Volume null (never zero) with `open_body`/`missing_volume`; incomplete without a manual volume; confirmed manual 1200 mm³ labelled manual; current 1.069400800 / 60.0 with independent arithmetic. BOM review-status residual recorded for SB-07. | `SB-22-p09-1-manual-volume-reviewed.png` |
| P10 Revision separation | **Partial** | Distinct definitions/ids with independent B/C inputs and save/reopen preservation, but the same-part-number/revision premise is Unrun: the STEP carries PRODUCT names only and app rows have `revision: null`. Prerequisite: metadata-carrying import path; then rerun (SB-04 area). | `SB-22-p10-1-revision-b.png` |
| P11 Flat input guard | Pass (backend residual) | Original 3-body import structure preserved; `unresolved_grouping` review required; no complete cost and no invented component identity. Incomplete snapshot residual recorded for SB-07/SB-16. | `SB-22-p11-1-flat-no-complete-cost.png` |
| P12 Failures | Pass | Service stopped → `save_failed`, `retryable: true`, edits retained, no DB write, no silent sync; retry with the same request id committed exactly one new version; conflict refused; restart+reopen loads the authoritative version. | `SB-22-p12-1-save-failed.png` |
| P13 Responsiveness | **Partial** | Service-delay half verified under `SOLIDBOM_DELAY_MS=5000`: rendered selection 0.2493 s, camera 0.3316 s, control max 0.0088 s, late answer stale and bound to original inputs. Analysis-delay selection/navigation Unrun: the importing workspace has no model (0 triangles), repaint/input only. Prerequisite: navigable view during analysis (SB-07 area) or an explicit scope decision. | `SB-22-p13-1-selection-rendered.png`, `-2-importing-no-viewport.png` |
| P14 Scope honesty | Pass | Prototype label and omitted categories visible; read-only enforcement across command, shortcut and UI paths; permitted selection/measurement/local save work; no activation/quotation command claims MVP readiness. | `SB-22-p14-1-readonly-refusal.png` |

## Confirmed obligations R-*

Grouped as in [the requirements map](../requirements-map.md). "Level" is the highest level actually
exercised.

### Delivery, ownership and project lifecycle

| ID | Result | Level | Observation | Evidence |
|---|---|---|---|---|
| R-DEL-01 | Pass | desktop | SolidBOM review/costing runs in the desktop fork with linked tree/viewport (P02, A06, P14); no browser delivery is claimed. | desktop, engineering |
| R-DEL-02 | Pass | desktop + service | Projects, sources, reviewed inputs, calculations and accepted results are central in PostgreSQL; restart, dump/restore and desktop reopen recover them. | lifecycle, recovery, desktop, ops |
| R-DEL-03 | Partial | desktop + service | Import, viewport, selection and measurement run locally with background analysis; the P13 analysis-delay selection/navigation remainder is Unrun. | desktop (P01–P13) |
| R-DEL-04 | Pass | desktop + service | Online-only writes: failed save reported, unsaved edits retained, explicit retry after service return commits once; local CAD save remains separate. | desktop P12, ops |
| R-DEL-05 | Pass | desktop | Locally launched macOS development build; architecture, OS and build recorded. | desktop env, ops |
| R-DEL-06 | Pass | desktop | Reviewed source geometry/hierarchy immutable across command, shortcut and UI paths; selection/measurement/annotations and business edits remain usable. | desktop P14, A06 |
| R-DEL-07 | Pass | service | One organization per deployment; the service identity model exposes no multi-organization behavior. | service README/tests, recovery |
| R-DEL-08 | Partial | service + desktop | Service enforces Engineer vs Administrator and the combined identity (401/403, both directions); desktop administrative surface is exercised read-only (`solidbom.shared`), no desktop admin-write UI is built. | recovery, ops evidence |
| R-PRJ-01 | Partial | service | Project record with business details, products, schedules and forecasts exists and is exercised; the desktop project editor is an open SB-10 desktop increment. | lifecycle, commercial |
| R-PRJ-02 | Pass | service | Each package belongs to one project and retains its original source; part identity/versions persist across packages. | lifecycle |
| R-PRJ-03 | Pass | service | Partial delivery revises selected products and adds new ones while omitted products stay intact. | lifecycle A01 |
| R-PRJ-04 | Pass | service | Engineering work is carried forward, affected review is surfaced, and a failed analysis does not erase a successful one; desktop failure presentation is the named remainder. | engineering A02/A03 |
| R-PRJ-05 | Pass | service | Exclude/reinstate retains files, revisions and quotation history and leaves accepted baselines byte-identical. | lifecycle |
| R-PRJ-06 | Pass | service | Draft/active/historical states distinguishable; prior revisions stay the costing basis until activation. | lifecycle A01 |
| R-PRJ-07 | Pass | service | Shared sourcing/material changes require coordinated activation within the customer version. | lifecycle A05 |
| R-PRJ-08 | Pass | service | Activation readiness blocks missing valid-cost inputs and unresolved findings; the gate applies to every group member. | lifecycle A05, engineering R-FEA-05 |
| R-PRJ-09 | Pass | service | Forecast/scope edits publish recalculated drafts and flag saved scenarios; accepted baselines unchanged; incomplete inputs stay named issues. | lifecycle |

### CAD, identity, review and feasibility

| ID | Result | Level | Observation | Evidence |
|---|---|---|---|---|
| R-CAD-01 | Pass | desktop + service | STEP solids and surfaces accepted; hierarchy/placement/quantity preserved; flat and mixed inputs handled without invented nesting (P01, P09, P11). | desktop, engineering |
| R-CAD-02 | Pass | desktop | Tree/viewport selection stays synchronized with the reviewed occurrence, including repeated placements (P02, A06). | desktop |
| R-CAD-03 | Partial | service | Flat grouping requires engineer association and refuses invented identities; the desktop grouping editor is deferred to the SB-11 increment. | desktop P11, engineering |
| R-CAD-04 | **Unrun** | — | Exact-B-rep extractor exists in the engine (tube/wear/thermal, version 1) but no integrated service/desktop acceptance path runs it; owner references and coverage targets absent. | engineering |
| R-CAD-05 | **Unrun** | — | Same extraction boundary; the service boundary consumed tangent/end lengths as portable evidence, which proves consumption, not extraction. | engineering A10 |
| R-CAD-06 | **Unrun** | — | ±0.1%/exact-bend-count claim needs independently verified owner references and agreed targets; 53 reference files absent. Analytic fixture expectations are not independent customer references. | engineering, ops |
| R-CAD-07 | **Unrun** | — | Coverage, supported file sizes and processing-time targets cannot be agreed without owner benchmark labels/measurements; benchmarking folder absent. | engineering, ops |
| R-CAD-08 | Pass | service | Manual completion of a missing measurement with unit/source/review enables costing when other inputs are valid; unavailable volume stays null, never zero (A08). | engineering |
| R-REV-01 | **Unrun** | — | The open-category model is live (prediction "clip", correction "bracket") but the analytic rule v1 proposes only `tube`; connector/fitting/clip/protection recognition coverage needs independent labels and agreed targets. | engineering |
| R-REV-02 | Pass | service | Every classification and catalog match stayed pending until engineer confirmation; bulk confirmation recorded the reviewer and two training-example records; confidence 1 never self-confirmed. | engineering |
| R-ID-01 | Pass | service | Six occurrences share one reviewed part record and one routing; hierarchy, placements and quantity retained. | engineering A04 |
| R-ID-02 | Pass | service | Changed geometry under the same customer number/revision stayed pending with `conflicting_geometry_match`; different part numbers stay distinct. | engineering |
| R-ID-03 | Pass | service | Distinct customer revisions coexist with separate sourcing, routing and demand/cost lines; accepted-version preservation on activation is SB-19 evidence. | engineering |
| R-CAT-01 | Pass | service | Unconfirmed candidates supply no cost; versioned confirmed references preserved; overrides stored separately (open defect D1: override not consumed by costing, SB-16). | engineering A09 |
| R-FEA-01 | Pass | service | Feasibility computed with no manufacturing/cost-center record; per-bend, per-end, spacing, compatibility and end-length checks present; no cost field in rule tables/findings. | engineering |
| R-FEA-02 | Pass | service | Findings carry rule/version/source, measured values and required limit; custom connectors with no data stay unresolved instead of an invented limit. | engineering |
| R-FEA-03 | Pass | service | Typed structured rule tables with applicability; no formula builder; inapplicable limits never applied; no rule → unresolved. | engineering |
| R-FEA-04 | Pass | service | Missing limit → unresolved; overlapping limits → conflict listing both with no priority; Administrator resolved by a new rule version; justified exception available. | engineering |
| R-FEA-05 | Pass | service | Open failed/unresolved findings blocked readiness; six justified exceptions cleared them; changed geometry made them `review_required` again. | engineering |
| R-MFG-01 | Pass | service | Every custom part has an explicit sourcing record; a missing record produces a `missing_sourcing` readiness blocker; costing reads stored sourcing. | engineering |
| R-MFG-02 | Partial | service | Part and subassembly routings apply and edit independently, propagate to all occurrences; product-root routing and routing-template adoption were not exercised in this fixture (routes exist; SB-15 contract). | engineering |
| R-MFG-03 | Pass | service | Subassembly operations are counted per subassembly occurrence (3), not in the six child units; child unit cost excludes them. | engineering |
| R-MFG-04 | Pass | service | Setup = 0.5 h x 120/h = 60 separate charge, once per shared routing, no batch allocation, outside per-piece totals. | commercial A13 |
| R-MFG-05 | Pass | service | Attached packaging line (2 pcs x 0.5, owner part version) costs 1/part-unit and 6/product without adding a CAD node. | engineering |

### Demand, costs, tooling and commercial scenarios

| ID | Result | Level | Observation | Evidence |
|---|---|---|---|---|
| R-DEM-01 | Pass | service | Project owns average/maximum/median policy; scenarios cannot override it; annual aggregation precedes the policy (peak 300, not 400). | commercial A11/A12 |
| R-DEM-02 | Pass | service | Project SOP/EOP defaults with product overrides; inclusive calendar years; missing forecast stays incomplete with no zero padding or monthly proration. | commercial |
| R-DEM-03 | Pass | service | Actual annual demand, costing volume and lifetime demand are distinct fields; actual forecasts drive the annual cost series. | commercial |
| R-COST-01 | Pass | service | Greatest minimum ≤ costing volume selects the tier; same-currency single price supported; uncovered demand → `missing_purchase_price`, never a guessed tier. | commercial |
| R-COST-02 | **Partial** | service | Pinned 120 CNY/h versions survive a published 200 CNY/h version 2 and recalculation with the earlier snapshot byte-identical; the explicit adoption action for a rate version was not exercised (A18 covered material/catalog/rule; owner SB-20). | commercial |
| R-COST-03 | Pass | service | One project currency; saved USD rate 7.2 converted 8 USD to 57.6 CNY; same-currency identity; earlier snapshot keeps its saved rate. | commercial |
| R-COST-04 | Pass | service | Geometry net consumption → density → mass → price/kg with allowance; manual override with unit/source; missing consumption incomplete, never zero. | engineering |
| R-COST-05 | Pass | service | Allowance applied once to material only (5.4 → 5.508); demand, production and setup unchanged. | engineering |
| R-COST-06 | Pass | service | Child costs rolled once per occurrence (6 x 5.508) and parent operations once; shared unit cost reused with occurrence quantity, not project demand. | engineering |
| R-COST-07 | Pass | service | Purchased assembly price covers descendants: child demand only from a direct occurrence elsewhere; nested-only part no demand/cost; same part elsewhere contributes. | commercial |
| R-COST-08 | Pass | service | Direct/fixed/percentage categories with defined bases; circular basis and duplicate code refused 422; app configuration selects EBIT categories. | commercial |
| R-COST-09 | Pass | scope check | Machine depreciation is reflected through the selected cost-center rates; no project equipment CAPEX/amortization line exists or is added; SB-X01 rate construction is outside this gate. | commercial, contract §1 |
| R-COST-10 | Pass | service | Immutable snapshots keep inputs/versions/results; incomplete results carry named issues; accepted results are never silently replaced. | lifecycle, recovery |
| R-TOOL-01 | Pass | service | Investment = count x cost/tool, one entry owned by one definition, counted once across products. | commercial |
| R-TOOL-02 | Pass | service | Manual and lifetime-demand recovery bases, occurrence expansion, product-root lifetime demand and purchased-branch exclusion all observed. | commercial |
| R-TOOL-03 | Pass | service | Shared investment allocated by lifetime demand shares (2/3 + 1/3 = 1,000, not 2,000); missing/zero combined demand unavailable. | commercial |
| R-TOOL-04 | Pass | service | Separate and amortized alternatives read from the same stored inputs, same investment, no double recovery; switching changes only the view. | commercial |
| R-TOOL-05 | Pass | service | Contribution continues after the recovery quantity (annual revenue 10,000/20,000/30,000, over-recovery 50,000); under/over shown. | commercial |
| R-COM-01 | Pass | service | Scenarios share project EBIT target and volume method; scenarios hold prices, recovery assumptions and tooling treatment only. | commercial |
| R-COM-02 | Pass | service | Suggested base price always beside manual price and its EBIT; blank manual price gives null EBIT/rate and null revenue. | commercial A17 |
| R-COM-03 | Pass | service | EBIT excludes tooling/setup; 80/(1−0.2)=100; manual EBIT 45/0.36; zero price and 100% target refused 422; first production year basis. | commercial |
| R-COM-04 | Pass | service | Productivity compounds on selling price 100/97/94.09; manufacturing cost unchanged. | commercial A14 |
| R-COM-05 | Pass | service | Scenario selects tooling productivity treatment; base + amortized shown as total; separate tooling outside unit price. | commercial A16 |
| R-COM-06 | Pass | service | Five named immutable scenarios saved and compared on the common baseline; the desktop comparison surface is a deferred SB-18 increment and is not claimed. | commercial |
| R-UPD-01 | Pass | service | Shared publishes flag only dependent projects and explain impact; adoption/recalculation explicit; snapshots and history unchanged. | lifecycle A18 |
| R-EXP-01 | Pass | service | Detailed Excel and summary PDF rendered from real service bytes; both independently validated. | lifecycle A20 |
| R-EXP-02 | Pass | service | Draft/incomplete/outdated exports labelled with missing inputs and unresolved findings; activation is not a prerequisite. | lifecycle A20 |
| R-ACC-01 | Pass | process | Prototype P01–P14 were rerun and recorded (P10/P13 gaps explicit); no prototype or MVP readiness is claimed from them. Confirmed MVP obligations and A01–A20 are covered above with explicit Unrun rows. | this matrix |

## Recovery, permissions and restart (contract §4.3)

| Procedure | Result | Observation | Evidence |
|---|---|---|---|
| Missing/unknown token | Pass | 401 unauthorized for both. | recovery |
| Engineer-only on an administrator route | Pass | 403 forbidden; administrator and combined identities 200. | recovery |
| Administrator-only / combined identities | Pass | Each identity holds exactly its sets; engineer routes refused for administrator-only. | recovery, ops |
| Lost reply: identical retry | Pass | Identical activation and project-create retries returned the recorded receipt with no duplicate row/version. | recovery |
| Lost reply: changed payload same request id | Pass | 409 conflict and no write. | recovery |
| Expected-version conflict | Pass | Delivery, activation project version and accepted-baseline head conflicts refused with no state change. | recovery |
| Stale result labelled | Pass | Draft result marked stale (`newer_shared_data`, later engineering/manufacturing versions) and export labelled `outdated_costs`; never served as current. | recovery |
| Service process restart | Pass | SIGTERM + restart recovered projects/sources/engineering/manufacturing/feasibility/3 accepted baselines/snapshots byte-identically; 25 table counts unchanged. | recovery |
| PostgreSQL dump + restore | Pass | Dump → fresh database → restore → service on second port reproduced identical row counts, identifiers, accepted results and values. | recovery, ops |
| Desktop save failure / disconnect / retry | Pass | Service stopped → `save_failed` (connection refused, `retryable: true`), version unchanged, edits retained; explicit retry after restart committed exactly one version. | desktop P12, ops |
| Desktop process restart + reopen | Pass | Full process quit and relaunch; `solidbom.reopen` by project id recovered 14/14 facts and the authoritative version. | desktop P06, ops |
| Startup failures documented | Pass | Occupied port, bad database URL, token configuration and migration-checksum failures produced the recorded symptoms and recovery actions. | ops |

## Required checks (contract §4.6)

| Check | Result | Observation | Evidence |
|---|---|---|---|
| Root repository gates | Pass | fmt, lists, skills, layers, mesh-decisions, licences, assets, icons, ui-containers, workspace clippy `-D warnings`, wasm, schema tests and design-format gate all exit 0 at the acceptance HEAD. | `SB-22-repository-gates.txt` |
| Aggregated `cargo xtask ci` | Partial | Not run as one command: its native test stage is the documented host baseline. Two documented red entries were reproduced directly (`kernel --test boolean_props`: 13/2/1; `solidcraft-doc --lib`: 637/2/2); no new failure, no SolidBOM crate involved. Full native stage remains inherited from prior records, not re-derived here. | `SB-22-repository-gates.txt` |
| Service workspace | Pass | fmt exit 0; clippy `--all-targets -D warnings` exit 0, no diagnostics; PostgreSQL-backed suite 207 passed / 0 failed / 2 ignored (on-demand evidence writers) with a fresh migrated schema per test. | `SB-22-service-tests.txt` |
| Workspace bookkeeping check | Partial | `python3 scripts/solidbom_workspace.py check` exits 1 on this integrated branch: the coordinator ledger (`EXECUTION.md`) and the dependency records still describe SB-09–SB-22 as unintegrated/in-progress. Pre-existing coordinator-owned drift, not changed by this package; must be reconciled by the coordinator. | ops evidence; `SB-22-service-engineering.txt` |

## Independent dimensional/recognition evidence (contract §4.4)

| Row | Result | Blocking input |
|---|---|---|
| A07, R-CAD-04/05/06/07, R-REV-01 coverage | **Unrun** | Owner-supplied independently verified STEP examples, reference labels/measurements, agreed coverage/file-size/processing-time targets and the zero-reference validation convention. `scripts/solidbom_workspace.py check --require-references` exits 1: 53 local reference files missing and 5 missing link targets; the benchmarking CAD folder is absent. The analytic fixture manifest rows (2400/4800/7200 mm³, ±0.1%) can be independently recomputed but are synthetic, not owner references. |

## Scope and exclusion checks (D-01–D-03)

| ID | Result | Observation |
|---|---|---|
| D-01 | Pass | The branch changes documents/evidence only; no browser client, installer, updater, offline authoritative editing/local costing, or Windows/Linux delivery was added or claimed. |
| D-02 | Pass | No other CAD formats, varying-diameter automatic measurement, batch/setup allocation, concurrent editing, separate revision approver, customer quotation, NPV/payback/investment analysis, general formula builder, multi-part tools or automatic model training/deployment was added or claimed. |
| D-03 | Pass | No synthetic CAD hierarchy restructuring: non-CAD items stay attached cost lines; no destructive project deletion, broad archival, live FX, actual-production recovery tracking, separate project CAPEX or supplier/purchasing workflows were added. |

## Defects and residuals for owning packages

| ID | Owner | Observation | Status |
|---|---|---|---|
| D1 | SB-16 (cross-reference SB-13 contract) | A project catalog override is retained and separated (A09 passes) but never reaches costing: `services/solidbom-server/src/costing.rs` `resolve_purchase` reads only `CatalogRecord.confirmed`, so the baseline costed 0.42 CNY before and after the 0.55 CNY override while the recognition contract calls `effective_inputs()` override values authoritative. Not patched (acceptance does not own feature modules). | Open — blocks a correct override price in costing |
| D2 | SB-04 area (STEP import) | P10 premise: import carries PRODUCT names only, no part-number/revision property parsing; app rows carry `revision: null`. Needs a metadata-carrying import path or fixture, then P10 rerun. | Open — P10 Partial |
| D3 | SB-07 area (review/import UI) | P13 analysis-delay selection/navigation cannot be observed because the importing workspace shows no model (0 triangles). Needs a navigable/selectable view during analysis or an explicit owner/coordinator scope decision. | Open — P13 Partial |
| L1 | SB-15 | Geometry-driven routing calculation is deferred, so a changed tube length has no geometry-derived routing input to invalidate. | Documented deferral |
| L2 | SB-14/SB-19 | Activation readiness for a new revision does not attribute the project's stale superseded feasibility assessment; assessment completeness remains a desktop client obligation. | Documented limitation |
| L3 | SB-07 | After a confirmed manual volume the BOM row still reads Pending review and keeps analysis issues; the incomplete calculation issue list omits `open_body`. | Display residual, P09 |
| L4 | SB-07/SB-16 | The incomplete flat result carries a partial per-part/assembly amount and duplicate issue entries in the raw snapshot/state while the UI shows Unavailable. | Residual, P11 |
| L5 | service startup | Startup logs only the listen address; the applied migration version must be read from `_sqlx_migrations` (documented in the handoff). | Non-defect observation |
| L6 | desktop shared-data command | `solidbom.shared` is asynchronous: the first control call after start returns `pending:["shared_data"]` with empty data; the next carries the loaded identity/records. | Non-defect observation |

## Verdict and remaining prerequisites

**The MVP acceptance gate is not passed and must stay Pending.** No row above is a readiness claim.

What passed: every executable confirmed obligation and application scenario at the service/storage
level; the desktop prototype rerun P01–P09, P11, P12 and P14 with real captures; recovery,
permissions and restart/restore; required repository and service checks (with the documented red
aggregate baseline reproduced, not hidden); the operating handoff procedures.

What blocks MVP acceptance:

1. **Owner inputs for dimensional/recognition evidence (owner action).** Independently verified STEP
   references, labels/measurements, agreed coverage/file-size/processing-time targets and the
   zero-reference convention. Until supplied, A07/R-CAD-04/05/06/07/R-REV-01 stay Unrun. This is also
   the SB-22 assignment gate's missing "independent references and agreed coverage/performance
   targets" item.
2. **Desktop increments owned by feature packages.** The integrated build exposes the service-level
   behavior, but the desktop surfaces for projects/deliveries (SB-10), flat grouping (SB-11),
   recognition suggestions (SB-13), feasibility (SB-14), manufacturing edits (SB-15), costing
   (SB-16), tooling (SB-17), commercial comparison (SB-18), activation (SB-19) and adoption (SB-20)
   are explicitly deferred by those packages' contracts. Their rows are recorded Pass at the service
   level with the desktop remainder named; MVP acceptance cannot be claimed without them.
3. **Open defects D1–D4 above** (catalog override consumption; P10 metadata path; P13 analysis-delay
   viewport; P09/P11 display residuals) — each needs its owning package.
4. **Coordinator reconciliation.** `EXECUTION.md` and the dependency records still show SB-09–SB-22
   as not integrated; the workspace check consequently exits 1. The coordinator must reconcile the
   ledger before any package readiness transition (this package must not edit `EXECUTION.md`).
5. **Release build unrun.** Only the development (`dev`) profile was built and launched, which is the
   agreed deployment boundary; a release-profile build was not executed.

Resume condition: owner supplies the reference/coverage inputs and the coordinating packages deliver
their desktop increments and defect fixes; then the affected rows are rerun on the integrated base and
this matrix is updated. No action is required from the wider team beyond those owners and the
coordinator.
