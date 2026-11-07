# Desktop SolidBOM prototype specification

Date: 2026-11-06
Status: D01–D07 confirmed by owner on 2026-11-06. Prototype scope and acceptance checks P01–P14 are approved as the planning baseline. Technical implementation design remains to be prepared. No implementation is authorized by this approval alone; no acceptance check below has been executed.

Authority: [APP_SPECIFICATION.md](APP_SPECIFICATION.md), including the desktop reconciliation in sections 1, 2, 15, 17 and 18. Confirmed business rules take precedence over prototype conveniences. The earlier repository analysis supplies evidence, not approved product behavior.

## 1. Goal and completion boundary

Demonstrate one complete workflow inside the SolidCraft desktop fork:

1. Create or open one draft SolidBOM project/product revision.
2. Import a synthetic STEP assembly without changing its hierarchy.
3. Select a repeated part in the CAD tree or viewport and see the same engineering record.
4. Review its measured volume, confirm a classification and choose internally manufactured sourcing.
5. Select an explicit material version, allowance and one manufacturing operation.
6. Calculate an explained direct-cost subtotal and a separate setup charge using the backend.
7. Save, close the app, reopen the project and recover the same source associations, decisions and saved calculation.

**Confirmed 2026-11-06 — D07:** the owner approved the prototype scope, numeric test fixtures/comparison tolerance and acceptance checks P01–P14, including the one-second UI-response target during a controlled five-second background delay on the declared test Mac. Prices/rates are test inputs, not company defaults; targets are requirements to verify, not measured results.

This is an engineering/costing prototype, not a throwaway screen mockup. It is also smaller than MVP. It does not demonstrate complete manufacturing totals, engineering activation, quotation readiness or automated tube recognition. Label its results `Draft prototype / Direct-cost subtotal`; display excluded categories explicitly. Do not call an incomplete subtotal an accepted full manufacturing cost.

## 2. Confirmed prototype operating model and proposed details

**Confirmed 2026-11-06 — D02:** target macOS first for the prototype. This does not commit to Windows/Linux delivery dates or universal-binary support.

**Confirmed 2026-11-06 — D06:** deliver a macOS development build launched locally. Signed installers, automatic updates and production distribution are outside the first prototype.

Use the existing SolidCraft desktop viewport, component tree and selection path. Add a SolidBOM workspace panel with three sections: engineering review, manufacturing inputs and cost explanation. Exact layout is a later design choice; no replacement viewport or browser shell is needed.

Retain central projects/shared data and backend costing as required by the application specification. **Confirmed 2026-11-06:** the prototype uses an online desktop client with central project writes and backend cost calculations. A real development service/persistent store and seeded material/rate records remain proposed implementation details; an in-memory server or mocked cost response cannot demonstrate persistent save/reopen.

**Confirmed 2026-11-06 — D01:** STEP import, viewport, selection and geometry measurements run on the desktop. Long-running geometry analysis runs in the background. Authoritative business data and cost calculations stay on the backend. Bind analysis results to their exact source/input version; technical scheduling and engine integration are left to the design. This decision does not claim that automated recognition or tube extraction is implemented.

**Confirmed 2026-11-06 — D04:** store projects, original STEP files, reviewed measurements, manufacturing inputs and saved calculation results centrally. Save project persists reviewed inputs and their source associations. A local SolidCraft file save is a separate action and does not imply that the SolidBOM project is saved. Local geometry/cache is temporary and rebuildable from the saved source; it is not an independently authoritative project. The precise CAD artifact/cache format and write/version contracts are left to technical design.

**Confirmed 2026-11-06 — D03:** project writes are online-only for the prototype. On connection failure, retain unsaved form edits in the current session, clearly report the failed save and support explicit retry. Do not calculate authoritative costs locally or silently synchronize later. No offline write queue or synchronization protocol is included. This does not promise recovery of unsaved edits after app exit/crash. Cached geometry may remain visible if already loaded; that does not imply offline project support.

## 3. Included and excluded work

| Included in the prototype | Outside this prototype, retained where required by MVP |
|---|---|
| One project, one product, one draft revision, one synthetic assembly | Full customer delivery comparison and coordinated activation |
| Repeated definitions/occurrences and safe revision identity | Cross-product shared demand, purchased-subtree costing and sourcing changes |
| Closed-solid volume review and manual missing-volume completion | Flat component-grouping editor, automated category recognition, complete tube/cover extraction and feasibility evaluation |
| Explicit manufactured sourcing; one material and one production operation | Catalog matching UI, routing templates, additional cost lines and administration screens |
| Explicit material allowance, material overhead and separate setup | Tooling investment/recovery, further cost categories, price breaks and FX scenarios |
| Backend arithmetic, explanations and immutable draft results | Suggested/manual selling prices, EBIT and annual productivity views |
| Save/reopen, incomplete/stale states and bounded failure handling | Production installers/updaters, browser delivery and internal Excel/PDF exports |

Full MVP continues to require every confirmed item in APP_SPECIFICATION.md. Prototype fixtures use one currency and explicit per-piece cycle basis. Unsupported routes/categories must be rejected or visibly excluded; they cannot silently receive zero cost or appear fully costed. Store a valid one-year positive forecast in the fixture, but do not claim to implement shared-demand policy from it.

**Confirmed 2026-11-06 — D05:** imported geometry is read-only in the prototype. Allow selection, measurement, annotations and engineering/costing edits, while preventing modeling or reparenting of geometry bound to the reviewed SolidBOM source. Geometry changes arrive through a new STEP revision; the complete revision comparison/activation workflow remains outside this prototype. Original CAD files remain unchanged. Enforce the restriction across UI, shortcuts and programmatic command paths, rather than hiding a panel alone. Annotations must not mutate source geometry or customer hierarchy. Any later editable variant requires a separate decision and revision/invalidation/save design.

## 4. Identity, commands and stored state

The behavior required by the approved acceptance checks is binding for this prototype. The implementation mechanisms below remain technical design recommendations, not a completed API/database implementation plan:

- Distinguish project, product revision, part definition/version, geometry definition and occurrence path. Repeated occurrences share one reviewed part record while preserving individual placements.
- Never use a displayed row index, body name or customer part number alone as persistent identity. Same-number/different-revision definitions must remain separate. Use SolidCraft's definition grouping for extraction where appropriate; its default BOM grouping can merge revisions.
- Bind every analysis and calculation result to exact input versions. Selecting another component while a request is pending must not redirect its response to that component. Discard or label obsolete results.
- Store classifications and material choices as engineer decisions. A material suggested by CAD is not a reviewed catalog association. Do not use SolidCraft's fallback density when no usable material was selected.
- Store extracted versus manually entered measurements, unit, source, review status and the input version they describe. Measurement source/unit labeling and the persistence needed by P06/P09 are approved for this prototype. Additional provenance fields remain design recommendations; this approval does not promote every provenance proposal in the full application specification to a confirmed MVP requirement.
- Run UI actions through commands/application services, following SolidCraft's thin UI and crate-layer rules. Cost arithmetic must be independent of egui and executable in backend tests. Only the existing kernel boundary may use truck.
- Keep project-save status separate from CAD session/file-save status. A backend failure must not show the project as saved. Successful persistence records consistent source, engineering input and calculation references; rejected writes retain the last saved version.
- SolidCraft undo does not automatically undo a central project save. Propose ordinary form editing until Save, then versioned backend writes; do not advertise unified CAD/backend undo. Define this contract before implementation.

## 5. Numeric fixture and independent expected results

Use original synthetic geometry and a STEP export with independently known dimensions. Verify the STEP-import path; constructing a box directly in the active session is insufficient. Do not use the imported measurements themselves as expected values.

The primary assembly contains three occurrences of one 40 × 20 × 3 mm rectangular solid. Assign a synthetic test classification such as clip manually; this is a test label, not evidence of recognition. All price/rate values below are explicit test inputs, not company defaults or engineering recommendations.

| Input or expected result | Value |
|---|---|
| One-part volume | 2,400 mm³ = 2.4 cm³ |
| Density selected in SolidBOM | 2.7 g/cm³ |
| Net mass per part | 6.48 g = 0.00648 kg |
| Test material price | 20 CNY/kg |
| Material allowance | 2%, applied once |
| Allowed material cost per part | 0.00648 × 1.02 × 20 = 0.132192 CNY |
| Material overhead | 5% of material cost = 0.0066096 CNY |
| Production cycle and rate | 30 seconds per part, 120 CNY/hour, cavity count 1 |
| Production cost per part | 30 / 3,600 × 120 = 1 CNY |
| Direct-cost subtotal per part | 0.132192 + 0.0066096 + 1 = 1.1388016 CNY |
| Assembly direct-cost subtotal | 3 × 1.1388016 = 3.4164048 CNY |
| Setup duration and dedicated setup rate | 0.5 hours × 120 CNY/hour = 60 CNY, separate charge once |

At two decimal places, displayed subtotals are 1.14 CNY per part and 3.42 CNY per assembly. Derive the assembly total from unrounded amounts, not 3 × the displayed 1.14. The server comparison uses exact decimal fixture inputs and an absolute arithmetic tolerance of 0.0000001 CNY. Geometry-dependent checks use the fixed ±0.1% relative tolerance, not the money tolerance. For the ideal-volume fixture, all numeric results above are the expected arithmetic values; an imported-volume variation within ±0.1% must flow through the formula and be compared against independently computed costs for that measured input.

Additional fixtures: an open surface with unavailable enclosed volume, a same-part-number/different-customer-revision definition, and a flattened multi-body STEP without a usable component hierarchy. No automatic one-body/one-component assumption is allowed. A flat grouping editor is deferred from this prototype. Such input must remain review-required and uncosted where component associations are unresolved; do not silently cost a body as a component.

## 6. Approved acceptance checks; execution pending

All checks are pending implementation. Save inputs, outputs, build identifiers and a native-app recording/screenshots for review. Synthetic arithmetic checks establish behavior only on these fixtures.

| ID | Action and measurable acceptance |
|---|---|
| P01 Import and source | Open the primary STEP from a file path. Original source SHA-256 is unchanged after review/save. Imported definition, occurrence quantities and transforms match the independently authored fixture manifest. Report degraded hierarchy rather than silently treating it as preserved. |
| P02 Selection | Select each of the three occurrences from the tree, then the viewport. Six selections resolve to the correct occurrence path and the same shared part/version record. Editing that record is reflected at all three occurrences without changing CAD placement or hierarchy. |
| P03 Volume and units | Extract one-part volume within 2,400 ± 2.4 mm³. Show units and source. The reviewed volume is one definition's volume; expanded quantity is 3. Do not charge the expanded assembly volume three times. |
| P04 Review | Imported/suggested classification is pending until the engineer confirms it. Explicitly choose manufactured sourcing and the fixture material version. With no selected usable material/density, calculation is incomplete even if SolidCraft displays a default mass. |
| P05 Arithmetic | Run backend tests against the ideal numeric inputs in section 5 within 0.0000001 CNY. In the UI, explain volume-to-mass conversion, allowance, overhead, seconds-to-hours, quantity expansion and setup separately. For the actual imported measurement, independently recompute the expected formula result and compare it with the server response. |
| P06 Save/reopen | Save to the development backend, close the desktop process completely, restart, and reopen by project ID. Recover the same source hash, occurrence mapping, part/version IDs, reviewed inputs and saved calculation ID/results. A recalculation is a new result, not a replacement of the saved result. |
| P07 Invalidation | After a successful result, change the operation time from 30 to 60 seconds. Mark the prior result outdated until recalculation. New ideal per-part subtotal is 2.1388016 and assembly subtotal 6.4164048 CNY; the earlier result remains readable with its original inputs. |
| P08 Missing data | Remove required density or cycle time. Return a named missing-input issue and no current complete subtotal. Explicit zero cycle time, if supported for an operation, must remain distinguishable from a missing value; its allowedness is defined by operation validation. |
| P09 Surfaces/manual values | On the open-surface fixture, enclosed volume is unavailable, never an inferred zero. An engineer can enter and review a manual volume with units/source. The calculator uses that reviewed value and labels it manual. Without it, the relevant material subtotal is incomplete. |
| P10 Revision separation | Import a fixture with identical part number and revisions B and C. The review records stay separate across save/reopen; changing B's material or routing does not change C. A default BOM row merge must not establish shared identity. |
| P11 Flat input guard | Open a flattened fixture lacking usable component associations. Preserve its original geometry/import structure, show grouping review as required and do not produce a complete cost for unresolved components. Do not manufacture component identities from body count. The full grouping editor remains required for MVP and outside this prototype. |
| P12 Failures | Inject backend unavailability, rejected version/write and a late response for an obsolete input. Do not mark failed writes saved, apply stale costs, or corrupt another selected component. Retain unsaved form edits in the current session and the previous central state; allow explicit retry without duplicate records. Do not queue silent synchronization or calculate authoritative costs locally. |
| P13 Desktop responsiveness | During a controlled five-second analysis/service delay, the window still repaints and accepts selection/navigation; the pending state is visible and a result remains bound to its original inputs. Require a maximum one-second response to these UI actions on the declared test Mac. Record OS/CPU/GPU/RAM and observed latency; this approved prototype target has not been measured and is not a customer-file performance claim. |
| P14 Scope honesty | The view shows Draft prototype / Direct-cost subtotal and lists omitted categories. No activation or production quotation action claims this prototype result satisfies full MVP readiness. Geometry modeling/reparenting is blocked under the confirmed read-only policy, including shortcuts and programmatic command paths; permitted selection, measurement, annotations and engineering/costing edits remain usable. |

Tube extraction is a separate technical gate. Preserve the ±0.1% requirement for OD/ID, developed length, each radius and cover length, and require exact bend counts. The supplied dataset inventory and earlier plate probe establish neither tube accuracy nor recognition coverage. Independent owner reference data and actual import/recognition benchmarks remain prerequisites for those claims. No reference-sheet creation is requested by this prototype.

## 7. Confirmed decisions and remaining technical design work

Owner confirmation on 2026-11-06 resolves D01–D07 for the prototype. No listed prototype decision remains open. Architecture/minimum macOS version, artifact formats, backend contracts, scheduling, enforcement mechanisms and exact panel layout remain technical planning work. Broader MVP benchmark/reference gaps and production delivery choices are not resolved by prototype approval.

| ID | Decision | Confirmed behavior and design consequences | Status / remaining work |
|---|---|---|---|
| D01 | Where CAD import/measurement runs | Confirmed for prototype: desktop import, viewport, selection and measurements; long-running geometry analysis in the background. Business data and costing remain central. Automated recognition scope remains outside this prototype. | Resolved; scheduling/integration details go into technical design |
| D02 | First supported desktop OS | Confirmed: macOS first for the prototype. Windows/Linux delivery is outside the first prototype. Determine architecture, minimum macOS version and build prerequisites from the target development/test machine during technical planning; do not assume universal-binary support. | Platform priority resolved; compatibility details remain technical planning work |
| D03 | Desktop connectivity/offline expectations | Confirmed: online-only project writes; keep unsaved form edits in the current session after failure, show failed save and allow explicit retry. No silent later sync or authoritative local costing. | Resolved for prototype |
| D04 | CAD and engineering artifact/save contract | Confirmed: central original STEP, reviewed measurements/inputs, source associations and saved results; Save project is separate from a local CAD-file save. Local geometry/cache is rebuildable. Artifact format, write/version mechanics and proposed undo behavior remain technical design details. | Ownership/save distinction resolved; design contracts still required |
| D05 | Modeling changes in a costing project | Confirmed: imported geometry is read-only; selection, measurement, annotations and engineering/costing edits remain available. Geometry changes arrive through a new STEP revision. Enforce across UI, shortcuts and programmatic command paths. | Resolved for prototype; enforcement details go into technical design |
| D06 | First distribution and update method | Confirmed: a macOS development build launched locally for the prototype. Signed installers, automatic updates and production distribution are deferred. Production delivery and fork/upstream update cadence remain future planning work. | Resolved for prototype |
| D07 | Prototype scope, money comparison and responsiveness targets | Confirmed: scope in section 3, numeric fixtures and comparison tolerance in section 5, and P01–P14 in section 6. Fixture prices/rates are test values, not company defaults; all acceptance checks remain unexecuted. | Resolved; implementation design and acceptance execution pending |

Central shared data, backend costing, one-organization scope and Engineer/Administrator permissions remain confirmed. A request to replace them is a separate decision, not a routine choice within this table. The prototype can use one development engineer account; that does not implement or waive full production authorization requirements.

## 8. Next planning artifact

With D01–D07 resolved, the next artifact is a technical implementation plan covering client panel/command boundaries, backend request/result contracts, stable identity and version mapping, save/reopen behavior, nonblocking job execution, source-edit policy and tests P01–P14. Inspect the actual fork state and target Mac before naming files or assigning estimates. Preparing that plan is separate from implementing the prototype; the original instruction not to implement remains in effect.

Do not scaffold code, install dependencies, create/push a public fork, or build prototype features as part of this specification task. No implementation plan, implementation evidence or customer accuracy result is claimed here.
