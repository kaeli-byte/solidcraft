# SolidBOM desktop prototype

This is the approved macOS prototype for reviewing a synthetic STEP assembly and calculating a
direct-cost subtotal. It is **not** the full SolidBOM product: results are labelled
`Draft prototype / Direct-cost subtotal`, and tooling, SG&A, margin, pricing, taxes and other
categories are outside the calculation.

The authoritative behavior is in `APP_SPECIFICATION.md` and `DESKTOP_PROTOTYPE_SPEC.md` beside the
handover, with the implementation plan in `TECHNICAL_IMPLEMENTATION_PLAN.md`. The integrated
P01–P14 acceptance run and its evidence are recorded in
[the SB-08 execution record](solidbom/execution/SB-08.md); the earlier pre-integration run remains
in `tests/solidbom/acceptance.md`.

## What it does

- Import one STEP file into a read-only review session; the source bytes stay central.
- Review one shared part definition across its occurrences, with exact closed-body volume.
- Confirm a classification, choose manufactured sourcing and one material version, enter
  allowance, cycle, cavities and setup, and calculate on the service.
- Save the project centrally, close the desktop process, reopen it by project id and recover the
  source, mapping, reviewed inputs and saved calculation.
- An open surface has no enclosed volume until an engineer enters and confirms a manual value.
- A flat multi-body file without a usable product hierarchy stays review-required.

Imported geometry is read-only: modelling, reparenting, moves, undo/redo, parameter changes and
document replacement are refused in a review session. Selection, measurement, inspection, view
changes and a separate local CAD save remain available.

## Prerequisites

- macOS on Apple silicon, Rust 1.95 or newer with the `wasm32-unknown-unknown` target.
- PostgreSQL 18, for example with Homebrew:

```sh
brew install postgresql@18
brew services start postgresql@18
export PATH="/opt/homebrew/opt/postgresql@18/bin:$PATH"
createdb solidbom_dev
```

## Start the development service

The service is its own Cargo workspace under `services/solidbom-server`.

```sh
export SOLIDBOM_DATABASE_URL="postgres://$(whoami)@127.0.0.1:5432/solidbom_dev"
export SOLIDBOM_TOKEN="<at least 16 characters; never commit it>"
export SOLIDBOM_BIND="127.0.0.1:8099"
cargo run --manifest-path services/solidbom-server/Cargo.toml
```

The service applies the migrations and seeds two explicit test materials (2.7 g/cm³, 20 CNY/kg)
and two explicit test rates (120 CNY/h). They are test fixtures, not company defaults.

`SOLIDBOM_DELAY_MS` adds a delay to every protected request; it exists only to exercise the
one-second UI-response target during a controlled five-second delay.

## Start the desktop app

```sh
export SOLIDBOM_ENDPOINT="http://127.0.0.1:8099"
export SOLIDBOM_TOKEN="<the same token>"
cargo run --release -p solidcraft -- --control 8123
```

Without `SOLIDBOM_ENDPOINT` or `SOLIDBOM_TOKEN` the app still imports and analyses locally, and the
panel says that saving and calculating need the service.

Choose **BOM** from the workspace selector beside Design and Render. The workspace keeps the
model above the bill of materials, with a selected-part inspector on the right. Its ribbon has
Project, Engineering, Manufacturing, Cost and View tabs. Import STEP and Open Project create
separate review document tabs. Open Project accepts a central project ID. With no review open the
workspace shows an empty state with the same two actions; cancelling the file picker creates no
tab, and a failed import or open leaves the previous review in place and says what failed.

Each table row is a shared part definition; its quantity counts the mapped occurrences. The
default columns are Part, Revision, Quantity, Classification, Sourcing, Material, Volume, Review
Status and Direct Cost per Part. A value the review does not have reads **Unavailable**, never
zero. Selecting a row highlights its occurrences; selecting a part in the model or browser
selects and reveals its row. Search, sorting and the All / Needs Review / Missing Inputs / With
Issues filters preserve the selected part, and a filtered-out selection stays in the inspector.
Selection, drafts, filters, scroll positions and panel sizes belong to each review tab. A narrow
window collapses the inspector first and the table scrolls sideways rather than overlapping the
model.

Apply commits the selected part's inputs. Revert restores its last applied values. Unapplied
inputs remain with each part when selection, document tabs or workspaces change. Save Project
and Calculate validate and apply all pending inputs atomically before contacting the service.
Material overhead is a project setting, separate from the selected part. Retry appears only for a
request that can succeed unchanged (a network or service failure); a version conflict asks for the
project to be reopened.

A review's tab indicator and close/quit question include central unsaved inputs and unapplied
editor changes. Save Project waits for the central save to complete. A failed save keeps the
review open. A separate CAD Save As does not save the central project.

The preference **Open SolidBOM review in a window** (Preferences › Layout) retains the original
review window. Both presentations share the same editor state and commands. The native assembly
bill-of-materials tool (`assembly.bom`) remains separate.

## Automation handles

The BOM workspace is reachable from the control channel and from scripts:

```sh
{"method": "ui.workspace", "params": {"workspace": "bom"}}
{"method": "ui.inspect", "params": {}}
```

`ui.inspect` returns `workspace` (`design`, `render` or `bom`) and a `bom` object with the active
ribbon tab, selected part, inspector section, filter and query, sort column and direction, pane
visibility, table share and whether the editor has unapplied changes. The presentation commands
`view.workspace`, `view.bom.tab`, `view.bom.section`, `view.bom.filter`, `view.bom.panels` and the
`view.bom.apply` / `view.bom.revert` / pane and section actions echo their request and are
dialog-free. Scenario handles include `bom:table`, `bom:inspector` and `bom:part:<id>`.

## Save and reopen

`solidbom.save` writes one immutable project version. `solidbom.calculate` saves unsaved inputs
first, then calculates the returned version. Calculations are append-only: a later edit marks the
old result stale but keeps it readable.

To reopen, start a fresh app (or a new tab) and run:

```sh
{"command": "solidbom.reopen", "params": {"project_id": "<uuid>"}}
```

The app downloads the source, verifies its SHA-256, opens it in a scratch session, and restores
the reviewed mapping only when the saved analysis key matches the current extractor. A mismatch
keeps the source read-only and asks for review again.

A local `file.save_as` writes a CAD file to a separate working path; it does not save the SolidBOM
project, and the project save does not write a CAD file.

## Integrated prototype acceptance (SB-08)

The approved P01–P14 checks were executed on the integrated prototype: the desktop app and the
development service built from `main` at `3ac17d757d62e0ce5bfa3c6c3af02056aaba9a2f` (after PRs
#41–#47), against a fresh PostgreSQL database and native desktop windows, with the tracked
synthetic fixtures. The full matrix, per-check evidence and reproduction commands are in
[the SB-08 execution record](solidbom/execution/SB-08.md). This section is the handoff summary, and
the handoff is **incomplete**: P10 is blocked and two required repository gates are red (details
below). It does not declare the prototype accepted.

### Build and environment under test

| Item | Value |
|---|---|
| Integrated commit | `3ac17d757d62e0ce5bfa3c6c3af02056aaba9a2f` on `main` (SB-07 merge after SB-01–SB-06) |
| Desktop | `cargo build --profile ci -p solidcraft`, `SolidCraft 0.40.0-rc.2`, build commit `3ac17d757`, binary SHA-256 `432ebac3fd196e13…` |
| Service | `services/solidbom-server` debug build, binary SHA-256 `59b7cc74461b921e…`, PostgreSQL 18.6, migrations 1–2 applied at start |
| Contracts | version 1, published at `1ab7b3b28c08b9a7afadee725c95f62465403ef7`; the integrated files carry compatible additive updates through SB-06/SB-07 (`solidbom.calculation`, `importing` phase, batch read-only parts/state) |
| Fixtures | `tests/solidbom/manifest.json`: six tracked STEP fixtures; the measured SHA-256 of every fixture equals the manifest |
| Machine | Mac17,9 / Apple M5 Pro (15 CPUs), 24 GiB RAM, macOS 26.7.1 (25G241), arm64, Metal GPU (`Apple M5 Pro`) |
| Database | fresh `solidbom_accept08`; seeds are two test materials (2.7 g/cm³, 20 CNY/kg) and two test rates (120 CNY/h), explicit test fixtures |

### Results

| Check | Result | Observed acceptance evidence |
|---|---|---|
| P01 import and source | Pass | One definition, three occurrences and the three manifest transforms imported from `three_boxes.step`; source SHA-256 `58927a45…` and the stored service row unchanged; analysis key `source_sha256`/`engine_build 0.40.0-rc.2`/`extractor_version 2`. |
| P02 selection | Pass | Three tree picks and three viewport picks resolve to distinct occurrence selections and the same shared record `f711a93d…`; editing the shared record changes all three occurrences and leaves the placement/hierarchy fingerprint unchanged. |
| P03 volume and units | Pass | Extracted 2400 mm³ against the analytic 2400 ± 2.4 mm³ (±0.1%), one definition with expanded quantity 3, source `extracted`, units mm³ shown; the reviewed volume is charged once per part, not three times. |
| P04 review | Pass | Classification and volume stay pending until confirmed; a calculation without a selected material is Incomplete with named issues (`missing_density`, `missing_material_price`, `unreviewed_volume`, missing operation inputs) and no complete subtotal, despite SolidCraft showing its own mass. |
| P05 arithmetic | Pass | Service snapshot: 0.1321920 material, 0.006609600 overhead, 1 production, 1.138801600 per part, 3.416404800 for quantity 3, 60.0 separate setup; an independent Python `Decimal` recomputation from the resolved inputs agrees within 0.0000001 CNY (difference 0), and the seven explanation lines carry the volume-to-mass, allowance, overhead, cycle, quantity and setup bases. UI shows 1.14 / 3.42 / 60.00. |
| P06 save/reopen | Pass | Save then complete desktop-process quit and restart (new process), reopen by project id `cb8c39ff…`: source hash, analysis key, definition and part/version ids, occurrence mapping, reviewed inputs, material selection, volume and saved calculation are identical, with result Current. A recalculation appends a new calculation id; the saved snapshot stays unchanged and readable. |
| P07 invalidation | Pass | Changing the cycle from 30 to 60 marks the existing result stale before recalculation; the recalculation gives 2.1388016 per part and 6.4164048 for quantity 3 within tolerance, and the earlier cycle-30 snapshot stays unchanged and readable as history, not current. |
| P08 missing data | Pass | Removing the cycle returns Incomplete with the named `missing_cycle_time` issue and no complete subtotal; an explicit zero cycle is complete and distinct (0 production, 0.1388016 per part); removing the material yields `missing_density` and `missing_material_price` and no complete subtotal. |
| P09 open surface | Pass | The open surface has no enclosed volume (null, never zero): Incomplete with named `open_body`/`missing_volume` issues and no complete subtotal. A manual volume with reason and confirmation is then used by the calculator and labelled `manual` (1.069400800 per part, 60.0 setup), with the service snapshot carrying the reviewed 1200 mm³ input. |
| P10 revision separation | **Blocked** | The native import path cannot carry the fixture's authored customer part number/revision labels, so the same-part-number B/C premise is not established. The tracked STEP contains the products ClipB/ClipC only; `crates/step` parses PRODUCT names without part-number/revision properties, and `solidbom.import` analyses the file bytes directly while the reviewed-session policy forbids applying component metadata afterwards, so the app rows show `revision: null`. The engine-level extractor test applies the labels to an in-memory design before extraction, which the desktop prototype cannot reproduce. Prerequisite for P10: a metadata-carrying import/fixture path (owning package), then rerun. |
| P11 flat input guard | Pass | The flattened input keeps its imported structure (structure fingerprint unchanged by review), reports `unresolved_grouping` as review-required, and the result is Incomplete with no current complete cost; no component identity is invented from the body count. Residual: the backend snapshot/state replies still carry the partial per-part amount (1.416404800) for this incomplete result while the UI suppresses it; it is never presented as a current cost. |
| P12 failures | Pass | With the service stopped a save fails clearly, is never shown as saved, keeps the unsaved edit and the previous central version, and writes no new project version or request; after the service returns no silent sync happens, and the explicit retry reuses the same request id, committing exactly one version and one request row. A lost response after a committed write was injected separately by a local proxy that forwarded the draft save (the service committed version 12) and dropped its 200 response: the app showed `save_failed` ("the SolidBOM service is unreachable: io: Peer disconnected"), kept version 11 visible with retry offered, and the retry resent the same request id with exactly one new version and one request row and no duplicate. An out-of-band version bump gives a save conflict (`the project is at version 4, not 3`), not saved and not retryable; retry is refused with "reopen the project or correct the inputs", the central state and previous calculation stay unchanged, and restarting/reopening loads the authoritative version and discards the local unsaved edit. A late/obsolete calculation answer is never current (P13 evidence). |
| P13 desktop responsiveness | Pass | Both controlled delays were measured. **Service delay:** with `SOLIDBOM_DELAY_MS=5000`, during a delayed calculation the review state shows Calculating, a viewport selection visibly completed in 0.017 s and a camera move in 0.002 s, and 15 UI actions had a maximum round trip of 0.0174 s (session-D/session-G); the late answer for superseded inputs is stale and keeps its original cycle-30 inputs. **Analysis delay:** a locally generated STEP with 7500 closed boxes (43.8 MB, generated outside the repository) took 5.12 s of background analysis; during the `importing` phase the window repainted, every control call answered in under 1 ms, and a camera move visibly completed (session-F). Both are below the one-second target on the recorded Apple M5 Pro machine. |
| P14 scope honesty | Pass | The cost panel shows `Draft prototype / Direct-cost subtotal` and `Not included: tooling, SG&A, margin, pricing and other categories.`; modelling, move/reparent and undo are refused through commands, the E shortcut and the UI button without changing the document; import/reopen and writes over the reviewed source are refused; selection, measurement, a separate local CAD save and engineering/costing edits still work; no activation, quotation or Excel/PDF export command exists. |

All P01–P14 rows above are recorded from the integrated run, and **the handoff is incomplete**:
P01–P09 and P11–P14 pass, P10 is blocked by the missing metadata-carrying import path, and the
required repository gate is red (the eleven documented SB-00 host-baseline test binaries plus a
reproducible `solidbom_review` scenario isolation failure in the review-UI test suite) while
workspace validation is red on coordinator ledger/record drift. The recorded evidence covers
P01–P09 and P11–P14; P10 is blocked. The prototype is not declared accepted until the P10
prerequisite exists, the owning packages resolve the scenario test isolation defect, the
coordinator reconciles the ledger and re-runs the affected checks, and an independent review
confirms the rerun. It is not production deployment, customer-file accuracy, tube recognition or
full MVP readiness.

### Evidence and reproduction

The acceptance evidence is local scratch under `.context/acceptance/` (kept outside the
repository, as the package brief requires): `environment.json`, `session-A.json` (P01–P05, P14),
`session-B.json` (P06), `session-C.json` (P07–P08), `session-D.json` and
`session-G-service-delay-visible.json` (P13 service delay, including visibly completed navigation),
`session-E-lost-response.json` (P12 lost-response injection), `session-F-analysis-delay.json`
(P13 analysis delay), `sessions/session-P09.json` … `sessions/session-P12.json` and
`sessions/NOTES.md` (P09–P12), the gates evidence under `gates/`, and the native `shots/*.png`
captures. The controlled analysis-delay STEP used by `session-F` is generated locally by
`.context/acceptance/fixtures/analysis_delay_5s_v2.json` and is not a tracked fixture. Each SB-08 record row cites its JSON and screenshot files. The driver scripts are local
scratch too; the reproducible interface is the published version 1 command and HTTP contracts
plus the startup commands above.

### Gates for this acceptance

- Root: formatting, workspace clippy, and the SolidBOM contract (35), costing (36), engine (78 + 2
  ignored) and UI (51) test suites pass, as does the `solidbom_review` scenario when run filtered.
  Every non-test `cargo xtask ci` gate and both side jobs (wasm check, doc tests) pass. The full
  `cargo xtask ci` test phase is **red**: 12 of 92 test binaries fail, the eleven documented SB-00
  baseline binaries on this host plus `ui-egui test scenarios`, whose only failure is the
  `solidbom_review` scenario (`step 38: no widget 'Apply inputs' on screen`). That scenario passes
  when run filtered but fails reproducibly in the full scenarios binary (parallel and serial), so
  it is a test-isolation defect in the review-UI package's scenario suite, not a P05 application
  failure; the exact failure set and reproduction are in the SB-08 record. The gate stays
  unsatisfied.
- Service: its independent workspace formatting, clippy and integration tests run with a
  disposable PostgreSQL database and pass.
- `python3 scripts/test_solidbom_workspace.py` passes (14 tests), but
  `python3 scripts/solidbom_workspace.py check` is red with ten coordinator ledger/prerequisite
  lines: the pre-existing SB-02–SB-07 drift plus the SB-08 row and its SB-07 prerequisite (this
  record is `in_review` while the stale ledger still lists the package as planned, and SB-07 has no
  recorded integration). `check --require-references` additionally needs the detached owner
  reference files absent from this worktree. Both are recorded as red / not run; reconciling the
  ledger and records is coordinator work, and the handoff stays incomplete until it is done.


## Limits of this prototype

- Open Project uses the existing project-ID workflow; there is no searchable central project
  browser.
- Controls, labels and help are translated; messages produced by the engine or the service (the
  save version notice, request errors) remain English.
- Local STEP extraction runs on the window thread for the synthetic fixtures; the service calls
  run on worker threads. Long-analysis scheduling remains future work.
- No tube recognition, ±0.1% tube measurements, cover lengths or bend counts.
- No tooling, SG&A, margin, suggested prices, EBIT, exports or activation.
- One development bearer identity; no production authentication or organizations.
- One configured source limit of 64 MiB.
- macOS only; no installers or updates.
- No offline writes or silent retries; a failed save keeps the inputs and is retried explicitly.

### Acceptance residuals and coordination items

- P09 display residual: after a manual volume is confirmed the panel and `solidbom.state` show the
  reviewed manual source, but the BOM table's Review status column still reads Pending review
  because the part projection keeps the analysis-level open-body/missing-volume issues. The
  calculator behavior is unaffected; recorded for the review-UI package.
- P10 metadata limit: the tracked same-part-number fixture carries the product names ClipB/ClipC
  but not its authored revision labels into the app rows (`revision: null`). Record separation is
  carried by distinct definition/part-version ids, volumes and reviewed inputs, which is identity
  under the contract; the labels are a known fixture/extraction limit, not a row merge.
- P11 backend residual: the incomplete flat result still carries a partial per-part amount and
  duplicate `unresolved_grouping` entries in the raw snapshot/state replies while the result is
  incomplete, the BOM cell reads Unavailable and no complete direct-cost subtotal is presented.
- Review-UI scenario test isolation: `solidbom_review` passes when run filtered but fails at step 38
  ("no widget 'Apply inputs' on screen") inside the full `scenarios` test binary, reproducibly in
  parallel and serial runs (3/3). It is a test-harness order/state interaction for the owning review
  UI package to fix, not a failure of the same flow in the real desktop app (P05/P06/P07 evidence),
  and it is why the full repository gate stays red.
- `python3 scripts/solidbom_workspace.py check` is red because the coordinator execution ledger and
  several predecessor records (SB-02–SB-07) have not been reconciled with their merged pull
  requests; this is documentation drift, not a prototype acceptance result.
  `check --require-references` additionally needs the detached owner reference files, which this
  public worktree does not carry.