# SolidBOM desktop prototype

This is the approved macOS prototype for reviewing a synthetic STEP assembly and calculating a
direct-cost subtotal. It is **not** the full SolidBOM product: results are labelled
`Draft prototype / Direct-cost subtotal`, and tooling, SG&A, margin, pricing, taxes and other
categories are outside the calculation.

The authoritative behavior is in `APP_SPECIFICATION.md` and `DESKTOP_PROTOTYPE_SPEC.md` beside the
handover, with the implementation plan in `TECHNICAL_IMPLEMENTATION_PLAN.md`. Acceptance checks
P01–P14 and their evidence are in `tests/solidbom/acceptance.md`.

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
