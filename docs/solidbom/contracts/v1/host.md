# Host contract

Version: **1** · Status: **published by SB-01** (not coordinator-locked)
Base: `aadc9b31bb6a4ffb30990818e1a0624c82bf2386` · SB-00 prerequisite: `b5115cda78f629cd82ef86d1f90e67a20f2a7c69`
Publication commit: `1ab7b3b28c08b9a7afadee725c95f62465403ef7` (these artifacts); the integration commit is recorded in `docs/solidbom/execution/SB-01.md`

Portable types: `crates/solidbom-contract/src/host.rs` (`SolidBomHost`, `JobRequest`,
`JobEnvelope`, `JobCompletion`, `HostCapabilities`, `HostError`, `CancelOutcome`,
`MAX_PENDING_JOBS`). Engine re-export: `crates/engine/src/solidbom/host.rs`.
Native implementation: `apps/solidcraft/src/solidbom_client.rs`; the browser and tests install no
host.
Acceptance: the contract `host` tests, the engine controller tests (queue full, no service, wrong
contract version, cancellation, envelope echo) and the native budget/cancel tests.

## Interface

```rust
pub trait SolidBomHost: Send + Sync {
    fn submit(&self, job: JobRequest) -> Result<JobId, HostError>;
    fn poll(&self) -> Vec<JobCompletion>;
    fn cancel(&self, job_id: JobId) -> CancelOutcome { CancelOutcome::Unsupported }
    fn capabilities(&self) -> HostCapabilities { HostCapabilities::default() }
}
```

`JobRequest { envelope, request }`; `JobCompletion { job_id, envelope, result }`. The host echoes
the envelope exactly; the controller reports a mismatch and leaves the job pending rather than
applying an answer to the wrong record.

`JobEnvelope` carries `request_id`, `project_id`, `analysis_key` (all optional before a project
exists), `local_edit_generation` and `binding_generation`. `stale_reason` compares it with the
current record and names the most fundamental change (`source_changed`, `engine_changed`,
`extractor_changed`, `binding_changed`, `inputs_changed`). `JobStatus` is the transport's lifecycle
vocabulary; version 1 reports only completions through `poll`.

`JobId` is host-local and not a business id. Two hosts may reuse ids; the application routes a
completion by host identity and job id.

## Submission and polling

- `submit` never blocks for the duration of the work. The native host starts one request thread.
- `poll` is non-blocking and returns every finished request. It is the **only wakeup path** in
  version 1: the application polls once per frame (and repaints about every 100 ms while any
  session has pending work). A host never calls UI code, never takes a lock the UI holds, and never
  mutates a document.
- The host never retries a mutation on its own. An explicit `solidbom.retry` resubmits the exact
  request (same request id and payload).

## Ownership

`Session.solidbom_host` is an `Option<Arc<dyn SolidBomHost>>`; the application injects it through
`Services.solidbom`. Applications share one `Arc` across tabs and poll each distinct host once,
routing a completion to the session whose host is pointer-equal and whose pending map contains the
job id. Closing a tab calls `Session::cancel_solidbom` before the session goes away. An unknown
job id is ignored, never applied to the active tab.

## Bounded queues and cancellation

- `HostCapabilities::max_pending_jobs` is `MAX_PENDING_JOBS = 32` for the native host. Submission
  past the bound returns `HostError::QueueFull { capacity }`; slots are freed when the worker
  finishes, including on a spawn failure or an early return.
- `cancel` is best effort. The native host flips a per-job flag that its worker checks once before
  queueing the answer; a request already on the wire may still deliver its real result. A cancelled
  answer arrives as `ClientFailure::Cancelled`: the save/result clears, the project stays dirty and
  no retry is offered. `CancelOutcome` reports `Cancelled`, `Unknown` (finished or never submitted
  here) or `Unsupported`.
- Cancellation does **not** roll back a save already on the wire: the central write may commit even
  though the worker reports `ClientFailure::Cancelled` instead of the success reply. A caller must
  reconcile the project state (reopen it, or refresh the saved version) before attempting another
  save; a second save carrying the old `expected_version` is refused as a conflict, never silently
  written over the committed version.
- `ClientFailure::Cancelled` is not retryable, like a conflict, an invalid request or a missing or
  unauthorized project.

## Capabilities and no service

`HostCapabilities { service, contract_version, max_pending_jobs, cancellation, upload_bytes }`.
`upload_bytes: None` means "not advertised"; zero would mean "no uploads". `none()` is the absent
service. The controller refuses a host whose `service` is false or whose `contract_version` differs
from `SCHEMA_VERSION` before submitting, with `EngineError::Capability`; the command toolbar shows
`Disabled` with `SERVICE_MISSING`. The browser and headless sessions have no host and take the same
capability path without a dialog.

## Compatibility

The trait, the envelope fields and the bounded-queue/no-retry/capability rules are the v1 contract.
A new method must have a default implementation; changing a signature or the envelope semantics is
a breaking change needing coordinator approval.
