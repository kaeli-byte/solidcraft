# SolidBOM prototype acceptance record

Date: 2026-11-07. Checks P01–P14 from `DESKTOP_PROTOTYPE_SPEC.md`, run against the desktop app
and the development service on the machine below. This record does not promote the prototype to
full MVP readiness.

## Build and machine

- Machine: Apple M5 Pro, 24 GiB RAM, macOS 26.7.1 (25G241), arm64.
- Rust: rustc 1.99.1. CAD workspace: branch `solidbom/task-8-acceptance`, parent commit
  `98729a388` with the task 8 changes uncommitted at the time of the run.
- Service: `services/solidbom-server`, PostgreSQL 18.6, fresh database `solidbom_accept`
  migrated and seeded at service start.
- Fixtures: `tests/solidbom/fixtures/`, hashes in `tests/solidbom/manifest.json`.
- Desktop: `target/ci/solidcraft --control 8123`, `SOLIDBOM_ENDPOINT=http://127.0.0.1:8099`.
- Evidence files: `.context/acceptance/*.json` and the PNG screenshots beside them, outside the
  repository. Tokens are not recorded. Panel screenshots were captured after reopening the saved
  projects; the cost screenshot shows the cycle-60 result with the reviewed inputs.

## Results

| Check | Result | Evidence |
|---|---|---|
| P01 import and source | Pass | `run.json`: import of `three_boxes.step`; the source SHA-256 was verified on reopen and the analysis key matched. The fixture hash is unchanged. |
| P02 selection | Partial | The engine test `six_selections_resolve_to_the_same_shared_part` resolves three tree and three viewport occurrence selections to one shared definition. The panel reads the selection and falls back to the first part; six manual app selections were not repeated in this run. |
| P03 volume and units | Pass | `run.json`: 2400 mm³, one definition, 3 occurrences, per-part subtotal uses the definition once (1.1388016, not 3× volume). The panel shows mm³ and the extracted/confirmed source. |
| P04 review | Pass | `run.json`: classification confirmed and the PA12 fixture material selected; the server reports Complete only with them. The unconfirmed/missing-material service tests report Incomplete. |
| P05 arithmetic | Pass | `run.json`: 0.132192 material, 0.0066096 overhead, 1 production, 1.1388016 per part, 3.4164048 assembly, 60 separate setup. The panel shows the explained lines. |
| P06 save/reopen | Pass | `run.json` then a full process restart and `reopen.json`: same project id `cb8c39ff…`, version 3, both calculation ids, source hash verified, 2400 mm³ manual/extracted state, result current. |
| P07 invalidation | Pass | `run.json`: changing the cycle from 30 to 60 gives 2.1388016 per part and 6.4164048 assembly; the first calculation id remains in the history. The controller test covers the stale state immediately after the edit. |
| P08 missing data | Pass | `flat.json`: removing the cycle time leaves the result Incomplete with `missing_cycle_time` and no complete subtotal. |
| P09 surfaces/manual values | Pass | `surface.json`: the open surface has no volume; a manual 2400 mm³ with a reason and confirmation gives 0.13608 per part/assembly, source `manual`, review `confirmed`, with a screenshot. |
| P10 revision separation | Pass | `revisions.json` and `revisions_reopen.json`: two definitions with distinct costs (1.1388016 and 2.2776032) survive a restart and reopen. STEP did not carry the B/C labels; the fixture tests supply them from the manifest. |
| P11 flat input guard | Pass | `flat.json`: the flat file reports `unresolved_grouping`; the calculation is Incomplete with that issue, never a complete cost. |
| P12 failures | Pass | `save_fail.json`: with the service stopped, the save fails with a clear unreachable message and the inputs stay; `retry.json` after the service restarts saves the same request with version 4. The lost-reply/duplicate case is covered by the service HTTP tests. |
| P13 responsiveness | Pass | `delay.json`: with `SOLIDBOM_DELAY_MS=5000`, ten `document.inspect` round trips had a maximum of 0.0028 s; the delayed save committed version 6 while the edit made during the delay stayed dirty and the result was marked stale. |
| P14 scope honesty | Pass | `flat.json`: `solid.extrude` was refused with the read-only reason and `inspect.measure` succeeded. The panel shows `Draft prototype / Direct-cost subtotal` and the omitted categories; `solidbom-panel-cost.png`. |

## Commands used

```sh
# service, fresh database
SOLIDBOM_DATABASE_URL="postgres://$(whoami)@127.0.0.1:5432/solidbom_accept" \
  SOLIDBOM_TOKEN="<development token>" SOLIDBOM_BIND=127.0.0.1:8099 \
  services/solidbom-server/target/debug/solidbom-server

# desktop
SOLIDBOM_ENDPOINT=http://127.0.0.1:8099 SOLIDBOM_TOKEN="<development token>" \
  SOLIDCRAFT_CONTROL_TOKEN="<32 hex characters>" target/ci/solidcraft --control 8123

# drive it (scripts kept outside the repository)
python3 -I .context/accept.py run 8123 <control token> <screenshot dir> <three_boxes.step>
python3 -I .context/accept.py reopen 8123 <control token> <screenshot dir> <project> <first calc>
python3 -I .context/accept.py surface 8123 <control token> <screenshot dir> <open_surface.step>
python3 -I .context/accept_more.py revisions 8123 <control token> <revisions_bc.step>
python3 -I .context/accept_more.py flat_all 8123 <control token> <flat_multi_body.step>
python3 -I .context/accept_more.py delay 8123 <control token> <part>
```

## Limits seen in this run

- Local STEP extraction runs on the window thread; the prototype fixtures are small, and the
  measured five-second delay was a service delay on a worker. Long local analyses are future work.
- No tube recognition, full manufacturing totals, pricing, exports or activation.
- One development bearer identity; no production authentication.
- The source upload limit is 64 MiB.
