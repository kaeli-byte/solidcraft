# HTTP API contract

Version: **1** (`API_VERSION`) · Status: **published by SB-01** (not coordinator-locked)
Base: `aadc9b31bb6a4ffb30990818e1a0624c82bf2386` · SB-00 prerequisite: `b5115cda78f629cd82ef86d1f90e67a20f2a7c69`
Publication commit: recorded in `docs/solidbom/execution/SB-01.md`

Portable types: `crates/solidbom-contract/src/api.rs` (`API_VERSION`, `ApiErrorBody`,
`ApiErrorCode`, `CalculationReceipt`).
Implementations: `services/solidbom-server/src/api.rs` (routes), `store.rs` (idempotency,
concurrency), `apps/solidcraft/src/solidbom_client.rs` (native client).
Acceptance: the server tests (`tests/api.rs`, `tests/persistence.rs`) and the client's error
mapping; the contract crate's error-body test.

## Transport

- Base path `/v1`; `/health` needs no token.
- `Authorization: Bearer <token>` on every `/v1` route. The development token is configured
  outside the repository and is never logged or returned.
- JSON bodies are the contract types, with decimals as strings. The request body limit is the
  service's configured upload limit (64 MiB by default). The client refuses larger response bodies.
- Errors never contain a database error, filesystem path or token.

## Routes

| Method | Path | Request | Success | Body |
|---|---|---|---|---|
| `GET` | `/health` | — | `200` | `{"ok":true}` |
| `PUT` | `/v1/sources/{sha256}?name=…` | raw STEP bytes | `200` | `SourceRef` |
| `GET` | `/v1/sources/{source_id}` | — | `200` | raw bytes, `Content-Type: application/step`, `x-solidbom-sha256` |
| `GET` | `/v1/reference-data` | — | `200` | `ReferenceData` |
| `POST` | `/v1/projects` | `SaveDraft` (`expected_version` 0) | `201` | `SaveReceipt` |
| `GET` | `/v1/projects/{project_id}` | — | `200` | `ProjectView` |
| `PUT` | `/v1/projects/{project_id}/draft` | `SaveDraft` | `200` | `SaveReceipt` |
| `POST` | `/v1/projects/{project_id}/calculations` | `CalculateRequest` | `201` | `CalculationSnapshot` |
| `GET` | `/v1/calculations/{calculation_id}` | — | `200` | `CalculationSnapshot` |

A source upload is idempotent by SHA-256: the service verifies the digest and returns the existing
row. A draft path whose `project_id` differs from the body, or a calculation body whose project
differs from the path, is `422 invalid`.

## Errors

The exact JSON shape is:

```json
{"error": {"code": "conflict", "message": "the project changed"}}
```

| Status | `code` | Meaning | Retry without a person? |
|---|---|---|---|
| 401 | `unauthorized` | missing or wrong bearer token | no |
| 404 | `not_found` | project/source/calculation not found | no |
| 409 | `conflict` | version conflict, or a changed payload under a reused request id | no — reopen or change the request |
| 413 | — (axum body limit) | upload larger than the service accepts | no |
| 422 | `invalid` | contract/field validation failed | no — correct the inputs |
| 500 | `internal` | the service could not complete the request | yes |

`ApiErrorCode::retryable` is `internal` only. The native client maps 401/404/409/413/422/other onto
`ClientFailure::{Unauthorized, NotFound, Conflict, Invalid, Server}` and network failures onto
`ClientFailure::Network`; only `Network` and `Server` are offered as an explicit retry.

## Idempotency and concurrency

- A write is keyed by its body `request_id` plus a canonical payload hash. An identical retry
  returns the original receipt/snapshot before the current-version check. A changed payload under
  the same id is `409`.
- A draft write compares `expected_version` with the stored current version under a row lock and
  appends one immutable version in the same transaction.
- A calculation names an exact `saved_version`; it never recalculates a moving current version.

## Compatibility

Changing a route, status, error code, field name or the idempotency key is a breaking change and
needs the affected-consumer list and coordinator approval. Adding a route or an optional field is
compatible.
