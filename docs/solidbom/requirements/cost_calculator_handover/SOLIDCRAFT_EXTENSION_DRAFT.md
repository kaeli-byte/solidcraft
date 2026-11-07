# Building SolidBOM on SolidCraft: analysis and proposal

Date: 2026-11-06
Status: technical draft for review; no product implementation or approved scope changes.

**Direction update, 2026-11-06:** the owner subsequently chose a desktop-first SolidCraft fork with SolidBOM panels. The browser-first recommendation and delivery sequence below are historical analysis, superseded by [APP_SPECIFICATION.md](APP_SPECIFICATION.md), sections 1 and 18, and the proposed [DESKTOP_PROTOTYPE_SPEC.md](DESKTOP_PROTOTYPE_SPEC.md). Repository observations and saved probe evidence remain supporting material; this report is not the current delivery specification.

## Recommendation

Use SolidCraft as a pinned, replaceable CAD processing dependency behind SolidBOM's central backend. Build SolidBOM's engineering review, manufacturing, costing and quotation workflows as a separate browser application. Start with the headless CLI and a versioned adapter. Add a small Rust worker using `solidcraft-engine` if the command API cannot supply the topology needed for tube extraction or efficient viewer assets.

SolidCraft can save us work on STEP ingestion, assembly placement, geometric measurement and CAD processing. It does not establish that the required tube recognition and measurement accuracy are solved. Adopt it only after a focused feasibility study against SolidBOM's inputs.

This proposal follows APP_SPECIFICATION.md, particularly the confirmed central backend, background CAD jobs, preserved customer hierarchy, flat review fallback, mandatory engineer confirmation and immutable accepted costing results. It does not replace that specification with SolidCraft's own BOM semantics.

## What I inspected and verified

Repository: `/Users/hafid/Documents/projects/solidCraft`.
Local HEAD: `cc9787f80e4e0aef068d576e6c6ddaf52d909f11`, dated 2026-10-30.
Workspace version and existing CLI binary: `0.27.1`. The binary was not rebuilt, so runtime checks establish its behavior, not a verified source-to-binary correspondence. The checkout had an existing untracked `.dsh/` directory; no repository files were changed.

GitNexus lists no index for this repository. Findings come from direct source/document inspection and a synthetic CLI probe. I also reviewed this workspace's APP_SPECIFICATION.md, benchmark inventory, design-rule review and costing handover.

The probe adapted the repository's BOM test fixture and ran successfully through `solidcraft-cli run ... --results`:

| Check | Observed result |
|---|---|
| A 40 × 20 × 3 mm aluminum plate | Volume 2,400 mm³; area 1,960 mm²; mass 6.48 g |
| Three occurrences of that plate | BOM count 3; root volume 7,200 mm³; mass 19.44 g |
| Another definition with the same part number but revision C instead of B | Default BOM combined the definitions into one row, count 4, showing revision B |
| Same design, `group_by: ["definition"]` | Separate rows: revision B count 3; revision C count 1 |

Reproducible input and raw replies are saved in `solidcraft-analysis-evidence/probe.json` and `results.json` beside this report. The probe did not contact a desktop session.

I did not run a full build, test suite, browser rendering test, customer STEP import benchmark or accuracy benchmark. The supplied benchmark inventory remains an inventory. Independent reference measurements/classifications are still pending. Repository oracle counts are historical project evidence, not proof of SolidBOM acceptance.

## Existing capabilities and their limits

| SolidCraft capability | SolidBOM use | Qualification |
|---|---|---|
| Layered Rust crates: geom, kernel/sketch/render, doc, io, engine, UI/MCP | Reuse CAD independently of its desktop interface | Only kernel owns truck dependencies; keep custom analysis behind a narrow boundary |
| `Session::execute(id, params)` and JSON scripts | Deterministic worker requests and replayable processing | Commands and output schemas are still being converted; pin and validate the required subset |
| STEP products, components and occurrence placement | Ingest hierarchy and repeated placements | Import can flatten unsupported/deep structures; preserve original source and detect degraded hierarchy |
| `component.list`, `component.metadata`, `assembly.bom` | Geometry/definition mapping and quantity cross-checks | Flat/top BOM exports are not the canonical preserved customer tree |
| `component.properties`, `occurrence.properties`, `inspect.measure` | Volume, area, bounds and placement-aware measurements | Open bodies omit volume/mass; assembly sums do not calculate the union of overlapping solids |
| Material assignment and document-owned material definitions | Geometry checks and optional CAD annotations | Unassigned/unknown materials fall back to a default; SolidBOM must use reviewed material master data |
| Face/edge inspection and cylinder data from `model.faces` | Inputs for analytic geometry recognition | No completed tube-analysis workflow was identified; generic face replies do not expose every surface parameter needed |
| Desktop/browser UI, GPU viewport, snapshots | Reference implementation and possible later embedded CAD experience | Browser API operates on the tab's UI thread; not the central analysis service required by the spec |
| Parameters, configurations, undo and document persistence | Later controlled engineering variants | Editing CAD is not needed for the first costing workflow |
| Plugin and web-runtime design documents | Future extension direction | Do not assume a shipped sandboxed plugin SDK or worker runtime from these documents |

The local roadmap also describes a truck-to-SMLib kernel migration. Treat that as a reason to isolate the dependency and rerun acceptance tests on upgrades, not as evidence that this checkout already uses SMLib.

## Architecture and ownership

Recommended flow:

```text
SolidBOM browser review UI
          |
Central application API + project/revision database
          |
Background job queue -> isolated CAD worker -> pinned SolidCraft
          |                       |
          |                versioned extraction result
          |                + viewer geometry + diagnostics
          v
Reviewed engineering baseline -> manufacturing/costing -> quotation snapshots
```

The API, database and queue technologies are implementation choices. PostgreSQL, object storage and a Python application backend would fit the handover; a Rust CAD worker can remain a separate process. These are proposals, not previously approved stack choices.

SolidCraft owns file decoding, CAD geometry, placements and low-level measurement. SolidBOM owns packages, persistent project identities, customer revisions, engineer-confirmed grouping, classification, sourcing, catalogs, feasibility rules, routings, forecasts, costs, scenarios, activation and exports.

Keep original STEP bytes immutable. Store analysis products against their file hash and product revision. Keep reviewed component-to-geometry associations separate from imported geometry. For flattened files, present flat review groups without creating a synthetic nested customer hierarchy.

For the browser, generate display meshes with a manifest mapping every selectable object to SolidBOM geometry and occurrence IDs. Preserve local geometry plus occurrence transforms so repeated parts can share mesh data. Prototype viewer export early: the inspected CLI export command does not list glTF output, so do not assume a ready-made web viewer export pipeline. A mesh sidecar/worker extension may be needed. Decimated display meshes must not become the source of critical dimensions.

### Proposed adapter contract

Expose application-level analysis jobs rather than forwarding arbitrary engine commands from the browser. The normalized result should contain:

- File hash, worker build, SolidCraft commit/version, adapter version and extraction algorithm version.
- Product/import status and explicit hierarchy-preservation diagnostics.
- Separate geometry definitions and occurrence paths/transforms, with source identifiers scoped to this analysis.
- Geometry associations, closed/open status, volume/area/bounds with units and measurement method.
- Tube centerline segments, OD/ID, bend radii/angles, tangent locations, end lengths and developed length when supported.
- Missing/unsupported/error states and diagnostics per component, rather than a single misleading success flag.
- Viewer artifact references and selection mappings.

SolidBOM assigns durable IDs. SolidCraft numeric IDs, body names and session revision counters are analysis-local references, not cross-delivery identity keys. File hashes and versioned results identify reproducible artifacts; reviewed matching establishes continuity across deliveries.

Run a fresh headless worker/session for each job initially. Give it a bounded temporary directory, CPU/memory/time limits and only the required input/output access. Use job IDs and deduplication to prevent retries from applying changes twice. Persist results only after validation. Do not expose the desktop control port as a backend service. A persistent stdio MCP session is available if interactive command exchange becomes useful, but deterministic jobs do not need an LLM.

## The work we still need to build

### Tube extraction is the first technical gate

Develop a deterministic extractor for the agreed constant-diameter tube subset. Candidate steps are to recognize compatible inner/outer cylindrical regions, identify bend regions and end sections, reconstruct an ordered centerline, and derive line lengths, arc lengths, individual radii, bend count and tangent-to-tangent/end distances.

This is a proposed algorithm direction, not a demonstrated solution. Imported bends may be spline surfaces rather than clean analytic tori. Split faces, seams, multiple layers, ambiguous tube ends and incomplete shells require explicit handling. Wear/thermal protection may need distinct centerline methods. Recognition categories and unsupported geometry must be explicit.

Do not estimate developed length from a bounding box. Volume divided by annular area can provide a consistency check only when constant dimensions, component grouping and end geometry justify it; it does not establish bend count or individual radii. Preserve the ±0.1% acceptance tolerance and exact bend-count requirement. Allow manual completion with review/provenance where the spec permits it.

### Definition identity and occurrence-sensitive costing

Read component definitions and occurrence structure before deriving costing rows. Use definition grouping for extraction and cross-checks. Never merge on part number alone: the runtime probe demonstrated loss of revision distinction under default grouping.

Different customer revisions remain separate project part versions. Similar geometry is supporting evidence for a proposed match, not authority to share routings or demand. Preserve the full hierarchy when a purchased parent stops costing expansion. Apply that boundary per occurrence branch so the same child used elsewhere still contributes demand. SolidCraft's `bom_type` is useful CAD metadata but must not override SolidBOM's reviewed sourcing choice.

Measure one definition once where geometry permits, then apply occurrence quantities exactly once. Do not multiply an already expanded assembly mass/cost by its occurrence counts again. Subassembly own geometry and own operations need explicit treatment alongside children.

### Engineering review and feasibility

Build synchronized tree/viewport selection, flat grouping, classification confirmation, measurement overrides and catalog confirmation in SolidBOM. Store predictions separately from confirmed engineering values. Collect confirmed labels without automatically training or deploying models.

Implement structured feasibility checks using the supplied rule references and approved Administrator-entered limits. The existing radius tables have material/process applicability conditions. Missing spacing/end-length/compatibility limits remain unresolved; no invented defaults or automatic pass. Detect conflicting applicable limits. Retain justified exceptions and their supporting inputs.

### Manufacturing, costs and revisions

Keep the current authoritative specification's business rules in the application backend:

- Derive net tube consumption from reviewed OD/ID/developed length; use usable solid volume for other internally manufactured parts. Apply reviewed density and material allowance once.
- Keep purchase UOM, quantity and currency explicit. A CAD occurrence count is not automatically a length in metres or a purchasing quantity.
- Aggregate shared-part demand by calendar year before applying the project volume policy and selecting price breaks.
- Keep setup as a separate quotation charge, shared-part setup counted once. Deduplicate tooling investment and preserve the selected recovery basis.
- Retain separate draft/active revisions and atomic activation of affected shared-part changes. Forecast edits recalculate drafts and flag scenarios without modifying accepted snapshots.
- Keep non-CAD cost additions attached to parts/assemblies without altering the CAD tree. Produce detailed Excel and summary PDF from saved results.

SolidCraft's document metadata supports reverse-DNS objects and preserves unknown namespaced keys. It could carry a small SolidBOM association ID if interoperability needs it. The central database remains authoritative for pricing, review and history; avoid placing an entire business model into `.sc` metadata.

## Options considered

| Option | Benefit | Cost or risk | Assessment |
|---|---|---|---|
| Separate SolidBOM application with CLI/Rust CAD worker | Fits confirmed browser/backend model; isolates CAD upgrades; preserves business ownership | Adapter, web viewer assets and tube extractor still required | Recommended |
| Fork SolidCraft and add costing panels | Immediate CAD viewport, tree and interactive modeling | Large Rust/egui UI investment; central shared workflow still needs a backend; ongoing upstream merge burden | Consider later only if CAD editing becomes a core product need |
| Build on the proposed plugin runtime | Potentially smaller upstream UI changes | Host/API maturity is not established by this checkout | Do not make MVP depend on it |

A direct Rust engine embedding is a refinement of the recommended worker, not a separate product architecture. Use it where the CLI JSON surface proves insufficient. Keep any fork small and limited to generic geometry extraction/export capabilities.

The repository declares MIT OR Apache-2.0 and includes vendored components/assets with their own attribution records. Carry the applicable notices and review the actual shipped dependency/asset set. Follow its clean-room and contribution rules for upstream work. Those rules prohibit commercial-CAD-derived reference fixtures for SolidCraft contributions; keep owner data and product-specific benchmark artifacts out of upstream changes. No dependency or distribution audit was performed here.

## Proposed delivery sequence and acceptance gates

| Stage | Deliverable | Evidence required to continue |
|---|---|---|
| 1. Feasibility study | Pinned worker, normalized import/measurement output, tube extraction trial and viewer manifest prototype | Synthetic hierarchy/surface/revision cases; real-data assessment when permitted; independent references for accuracy claims; exact bend counts and ±0.1% critical dimensions |
| 2. Review foundation | Package/product revisions, preserved import tree, flat grouping and linked viewport, classifications/manual completion | Round-trip selection mapping; no invented component hierarchy; missing geometry remains missing; retry preserves review work |
| 3. Engineering and manufacturing | Versioned part identity, catalogs, sourcing, structured rules and routings | Distinct customer revisions; purchased-branch boundaries; shared edits and exception invalidation behave as specified |
| 4. Costing baseline | Material/purchase/production costs, shared demand, setup/tooling, activation and snapshots | Reconciled handover vectors plus application acceptance scenarios; no duplicate costs or silently changed accepted results |
| 5. Commercial views and exports | Named scenarios, target/manual prices, EBIT, productivity, Excel/PDF | Traceable saved baseline; incomplete/stale results labeled; project investment/setup totals reconcile |

The feasibility study should cover all ten supplied STEP files for import behavior and record per-file failures, preserved hierarchy, solids/surfaces, memory and processing time. Accuracy remains pending until independent reference labels/measurements arrive. Respect local contribution restrictions when choosing the execution environment and fixtures; no customer benchmark was run for this analysis.

Add synthetic controls for nested repeated assemblies, same part number with different revisions, a purchased subtree, open surfaces, mixed geometry groups, missing density and known analytic tubes. Validate viewer picking and unit conversion as well as geometric results. Agree file-size/time/coverage targets from measured evidence rather than inventing estimates now.

If SolidCraft cannot preserve required topology or expose sufficient tube geometry, replace the worker implementation behind the same adapter before expanding UI investment. The recommendation is conditional on this gate; neither the README's feature breadth nor the synthetic plate probe establishes production readiness.

## Source map

Paths below are relative to the inspected SolidCraft repository unless noted.

| Evidence | Location |
|---|---|
| Version and crate dependencies | `Cargo.toml` |
| Layering and command ownership | `docs/architecture.md`, `skills/solidcraft-commands/SKILL.md` |
| Shared command entry point and rollback | `crates/engine/src/lib.rs`, `Session::execute` around line 1259 |
| BOM contract, grouping key and row merge | `crates/engine/src/cmd/bom.rs`, lines 25–47 and 342–382 |
| Repeated/nested component tests | `crates/engine/src/cmd/bom_tests.rs` |
| STEP hierarchy import and flattening fallback | `crates/engine/src/cmd/file_import.rs`, beginning and `Plan::walk` |
| Open-body measurement behavior and face data | `crates/engine/src/cmd/inspect.rs`, `measure_json`, `measure`, `model_faces` |
| Definition/occurrence physical measurements | `crates/engine/src/cmd/component_properties.rs` |
| Material fallback | `crates/doc/src/document.rs`, `body_material` and `density` around lines 3268–3283 |
| External metadata preservation and limits | `crates/doc/src/metadata.rs` |
| Browser execution constraints | `docs/browser-api.md`, `docs/web-runtime.md` |
| Plugin maturity and migration direction | `docs/plugins.md`, `ROADMAP.md`, workspace dependencies |
| Authoritative business behavior | This workspace's `APP_SPECIFICATION.md`, sections 1, 6–15 |
| Benchmark/reference limitations | This workspace's `BENCHMARK_REFERENCE.md`, `DESIGN_RULES_REFERENCE.md` |
