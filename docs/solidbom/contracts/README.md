# SolidBOM contract registry

No product contract is locked or implemented yet. SB-01 must publish the first prototype contracts here before SB-02, SB-04 or SB-05 can be assigned. Later packages extend the registry only through coordinator review of affected consumers.

For each contract record its version, owning package, implementing module, producers, consumers, wire/schema rules, compatibility policy and acceptance tests. A package assignment pins contract versions and an integrated base commit; a prose roadmap alone is not a wire contract.

## Prototype decisions SB-01 must encode

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

SB-01 owns prototype contract scaffolding and initial command/host seams. Future packages own dedicated contract modules for their subsystem. Root exports, protocol versions, command registries and shared enums require a coordinator reservation; never race modifications in independent worktrees.

Breaking changes need an affected-consumer list, a migration/compatibility decision and explicit approval in EXECUTION.md before dependent agents continue. Historical saved snapshots remain readable under their recorded schema/calculator versions.
