# SolidBOM contract registry

**Version 1 is published by SB-01** (see `v1/` and `docs/solidbom/execution/SB-01.md`). It is not
coordinator-locked: SB-02, SB-04 and SB-05 may work against it, and a breaking change still needs
the affected-consumer list, a migration/compatibility decision and coordinator approval in
`EXECUTION.md` before dependent agents continue. Later packages extend the registry only through
coordinator review of affected consumers.

For each contract record its version, owning package, implementing module, producers, consumers,
wire/schema rules, compatibility policy and acceptance tests. A package assignment pins contract
versions and an integrated base commit; a prose roadmap alone is not a wire contract.

## Version 1 registry

Base: `aadc9b31bb6a4ffb30990818e1a0624c82bf2386`; SB-00 prerequisite:
`b5115cda78f629cd82ef86d1f90e67a20f2a7c69`. Publication commit: `1ab7b3b28c08b9a7afadee725c95f62465403ef7`; the integration commit is recorded in `docs/solidbom/execution/SB-01.md`.

| Contract | Version | Document | Portable types | Producers → consumers | Acceptance |
|---|---|---|---|---|---|
| Identity and analysis | 1 | [v1/identity-and-analysis.md](v1/identity-and-analysis.md) | `crates/solidbom-contract/src/types.rs`, `error.rs` | desktop extractor, service store → controller, calculator, later engineering packages | `types`/`error` tests |
| Reviewed inputs | 1 | [v1/reviewed-inputs.md](v1/reviewed-inputs.md) | `types.rs` (`PartInputs`, `OperationInputs`), `review.rs` | `solidbom.inputs*` commands → store, calculator, later sourcing/feasibility packages | `types`/`review` tests |
| Project save and calculation | 1 | [v1/project-save-calculation.md](v1/project-save-calculation.md) | `types.rs`, `api.rs` (`ProjectVersionRef`, `CalculationReceipt`) | controller and service store → desktop, reopen, exports | `types`/`api` tests; P05–P07 round trips |
| HTTP API | 1 | [v1/http-api.md](v1/http-api.md) | `api.rs` (`API_VERSION`, `ApiErrorBody`, `ApiErrorCode`) | service API/store → native client and control channel | server tests; client mapping |
| Commands | 1 | [v1/commands.md](v1/commands.md) | `crates/engine/src/cmd/solidbom.rs`, `crates/engine/src/schema/solidbom.rs` | UI, control channel and MCP → engine handlers | `schema/solidbom_tests.rs`, controller tests |
| Host | 1 | [v1/host.md](v1/host.md) | `crates/solidbom-contract/src/host.rs`; engine re-export | application host (`apps/solidcraft`) → session controller | `host` tests, controller tests, native budget tests |

Compatibility policy: adding an optional field, a defaulted trait method or a route is compatible.
Changing a field meaning, wire name, unit, route, status, error code, idempotency key, command id or
the envelope semantics is breaking. Historical snapshots remain readable under the version that
wrote them; a schema version this build does not read is refused, never coerced.

## Prototype decisions SB-01 encodes

- UUIDs are persistent business identities; native generation stays at the host/service boundary. Runtime component/occurrence IDs are not source or server identities.
- An analysis key binds source hash, importer/engine build and extractor version. The manifest explicitly joins source-local geometry, reviewed part versions and runtime selections.
- JSON business amounts are decimal strings. Finite geometry values are converted once with units and source/review metadata. Missing values remain optional, never zero.
- Reviewed part inputs include explicit definition-to-part-version associations, classifications/confirmation, sourcing, material version, reviewed volume, allowance, operation inputs and annotations.
- Rate versions include currency, unit/time basis and separate production/setup use. The prototype uses CNY, per-piece seconds, cavity count one and one positive annual forecast.
- A versioned server-owned cost policy supplies the material-overhead rate and its material-cost basis. The fixture's 5% and 120 CNY/hour are seeded test inputs, not defaults.
- Save requests bind request ID and expected project version; identical retries return the original receipt before checking current-version conflicts. A changed payload with the same request ID is rejected.
- Calculations reference a persisted project version and retain resolved material/rate/policy values, explanations, issues, input hash and calculator version in immutable snapshots.
- Host requests/completions include project, analysis and local edit/binding generations. Jobs return typed handles/status and cannot mutate UI state directly.
- Application commands share one typed dispatcher across UI/control/MCP; no-service hosts report capability errors. Backend writes do not participate in CAD undo transactions.

## Shared ownership

SB-01 owns prototype contract scaffolding and initial command/host seams. Future packages own
dedicated contract modules for their subsystem. Root exports, protocol versions, command
registries and shared enums require a coordinator reservation; never race modifications in
independent worktrees.

The shared files SB-01 edited are listed in `docs/solidbom/execution/SB-01.md`. Breaking changes
need an affected-consumer list, a migration/compatibility decision and explicit approval in
EXECUTION.md before dependent agents continue. Historical saved snapshots remain readable under
their recorded schema/calculator versions.
