# Command contract

Version: **1** · Status: **published by SB-01** (not coordinator-locked)
Base: `aadc9b31bb6a4ffb30990818e1a0624c82bf2386` · SB-00 prerequisite: `b5115cda78f629cd82ef86d1f90e67a20f2a7c69`
Publication commit: `1ab7b3b28c08b9a7afadee725c95f62465403ef7` (these artifacts); the integration commit is recorded in `docs/solidbom/execution/SB-01.md`

Implementations: `crates/engine/src/cmd/solidbom.rs` (handlers/registry),
`crates/engine/src/schema/solidbom.rs` (input/output schemas).
Acceptance: `crates/engine/src/schema/solidbom_tests.rs` (discovery, schema/key agreement, strict
unknown keys, absent service) and the controller tests in `crates/engine/src/solidbom/tests.rs`.

## Published commands

All ten are ordinary registered commands: they take JSON, never open a dialog, and carry both an
input contract and an output schema. All are `.noundo()`: a successful call never pushes a CAD undo
step and never mutates the reviewed geometry.

| Id | Input contract | Output schema | Available when |
|---|---|---|---|
| `solidbom.import` | `ImportParams { path }` | `ImportReply` | always (local analysis works without a service) |
| `solidbom.inputs` | `InputsParams` | `StateReply` | always |
| `solidbom.inputs.batch` | `BatchInputsParams { parts, material_overhead_rate? }` | `StateReply` | always (validates every patch on a clone, commits once) |
| `solidbom.parts` | `EmptyParams` | `PartsReply` | always |
| `solidbom.save` | `EmptyParams` | `JobReply` | review open, service present, saved version, no save in flight |
| `solidbom.calculate` | `EmptyParams` | `JobReply` | as save, no calculation in flight, reference data loaded |
| `solidbom.retry` | `EmptyParams` | `JobReply` | a failed request that is retryable |
| `solidbom.reopen` | `ReopenParams { project_id }` | `JobReply` | no review bound, not already opening, service present |
| `solidbom.state` | `EmptyParams` | `StateReply` | always |
| `solidbom.review` | `EmptyParams` | `StateReply` | always (the host opens the panel; headless returns state) |

In `ParamCheck::Strict`, an unknown top-level key is refused before the handler runs. In the
default `Off` mode the handler's own reader decides, as for every other command.

## Dispatch path

```
UI panel / command palette ─► SolveApp::run ─► Session::execute ─► checked! contract ─► handler
desktop control / bridged MCP ─► control::execute ─► SolveApp::run ─► (same)
MCP headless / CLI ─► Session::execute ─► (same)
host completion ─► documents::advance_solidbom ─► Session::apply_solidbom_completion  (bypasses Session::execute)
```

- `SolveApp::run` (`crates/ui-egui/src/lib.rs`) routes application-layout commands first, then calls
  `Session::execute`; the SolidBOM panel calls `app.run` for its commands.
- The control channel's `engine.execute` reaches the same `SolveApp::run`; `engine.batch` refuses
  any `solidbom.*` step (see below).
- Headless MCP and the CLI call `Session::execute` directly. With no host installed, the service
  commands return a capability error, never a dialog.
- **Host-dispatched completions do not pass through `Session::execute`.** The application polls a
  shared host once (`documents::advance_solidbom`), routes each `JobCompletion` by job id and host
  identity, and calls `Session::apply_solidbom_completion`. That path validates the echoed
  `JobEnvelope`, applies the result only to the record it describes, and may submit follow-up jobs
  (upload → create → reference data; save → calculate). It never writes the CAD document.
- `solidbom.import` and `solidbom.reopen` open a reviewed document through `doc.open` while the
  session is not yet bound; after binding, the review policy refuses modelling commands.

## No CAD undo, no mixed transactional batches

- A service command is `.noundo()` and does not clone or mutate `Session.doc`; central writes are
  not CAD undo steps. A failed or cancelled request leaves the design untouched.
- `solidbom.inputs.batch` applies all part patches to a clone, validates the whole draft, then
  commits once — never a partial mixed edit.
- `engine.batch` refuses every `solidbom.*` step before the first step runs:
  `engine.batch unavailable: step i (id) uses the SolidBOM service; send it on its own`. A batch's
  `edit.restore_checkpoint` can restore the design but not a central write, and `solidbom.import`
  replaces the document. `solidbom.calculate` already saves first, so one step suffices.
- An absent service is a capability error: the toolbar shows the command disabled with
  `the SolidBOM service is not configured` (`EngineError::Disabled`), and a programmatic submit
  returns `EngineError::Capability`. No code path opens a dialog.

## Compatibility

The ids, their input/output schema types and the no-undo/no-batch rules are the v1 contract.
Adding a command is compatible; renaming or removing one, or making it undoable, is breaking and
needs coordinator approval.
