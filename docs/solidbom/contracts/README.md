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

## Version 2 registry (SB-09, SB-10, SB-11, SB-13, SB-14, SB-15, SB-16, SB-17, SB-18, SB-19 and SB-20)

Version 1 stays published and readable. Version 2 adds the shared-data/permissions contract
(SB-09), the project/delivery/part-version contract (SB-10), the engineering review and flat
grouping contract (SB-11), the sourcing and manufacturing definitions contract (SB-15), the
recognition and catalog suggestions contract (SB-13) and the feasibility rules and exceptions
contract (SB-14); SB-11, SB-13, SB-14 and SB-15 are each contract version 1 of their own, additive
to API version 2, and `SCHEMA_VERSION` remains 1 while `API_VERSION` is 2. All contracts are served
by the same service; the shared-data routes extend `/v1`, the project-record, engineering-review,
manufacturing and feasibility project routes are nested under `/v2`, and the feasibility rule
routes live under `/v1`.

Bases: SB-09 at `3c355556b2bafe19ced67e785aa8bee8fff16079`; SB-10 at
`e175e73827252159ebf120d0d201161120274ce3`, reconciled onto the integrated main at `a10b44b9f`;
SB-11 at the integrated main `71b9f91fb818ba241a595f9a1b34d79c52207137`, which carries SB-09
(`7d7dad5aa`) and SB-10 (`4dae1a3c8`); its contract lock commit is recorded in SB-11's record.
SB-15 starts on the integrated main `7fa5a0ab2`, which carries SB-09, SB-10 and SB-11; its contract
lock commit is recorded in SB-15's record. SB-13 builds on that same base under the owner's
2026-11-09 implementation instruction; its status is recorded in its contract document and it is not
coordinator-locked yet. SB-14 starts on the same integrated main
`7fa5a0ab2dbf725d2fef4e77575e7da788beb584`, which carries SB-09 (`7d7dad5aa`), SB-10
(`4dae1a3c8`), SB-11 (`306eee219`) and SB-12 (`c034e7fa2`); its contract lock commit is recorded in
SB-14's record. SB-16 starts on the integrated main `10d79722b`, which carries SB-09 through SB-15
and the feasibility package; its contract lock commit is recorded in SB-16's record. SB-17 starts on
the integrated main `ab52a01a9`, which carries SB-16, and its contract lock commit is recorded in
SB-17's record. SB-18 starts on the integrated main `802d52de1`, the merge of SB-17 (PR #62), and its
contract lock commit is recorded in SB-18's record. SB-19 starts on the same integrated main
`802d52de1`, which carries SB-14 (#60), SB-16 (#61) and SB-17 (#62), and is rebased onto the SB-18
merge (PR #63) before its own integration; its contract lock commit is recorded in SB-19's record.
SB-20 starts on the integrated main `acc7c3582`, the merge of SB-18 (PR #63) and SB-19 (PR #64), and
its contract lock commit is recorded in SB-20's record.
Portable types: `crates/solidbom-contract/src/shared_data.rs`, the versioned reference records in
`types.rs`, `crates/solidbom-contract/src/projects.rs`, `crates/solidbom-contract/src/engineering.rs`,
`crates/solidbom-contract/src/manufacturing.rs`, `crates/solidbom-contract/src/recognition.rs`,
`crates/solidbom-contract/src/feasibility.rs`, `crates/solidbom-contract/src/costing.rs`,
`crates/solidbom-contract/src/tooling.rs`, `crates/solidbom-contract/src/commercial.rs`,
`crates/solidbom-contract/src/activation.rs` and `crates/solidbom-contract/src/adoption.rs`.

| Contract | Version | Document | Portable types | Producers → consumers | Acceptance |
|---|---|---|---|---|---|
| Shared data and permissions | 2 | [v2/shared-data-and-permissions.md](v2/shared-data-and-permissions.md) | `shared_data.rs` (`Identity`, `Permission`, `SharedDataView`, `ReferenceChange`, `CatalogComponentVersion`, `AppConfigVersion`, `Create*`), versioned `MaterialVersion`/`RateVersion`, `ApiErrorCode::Forbidden` | service shared-data/authorization modules → desktop client, engine state, administrative UI, SB-20 change facts | contract tests; service permission/immutability tests; desktop command/UI tests; recorded API/UI evidence |
| Projects, deliveries and part versions | 2 | [v2/projects-deliveries.md](v2/projects-deliveries.md) | `projects.rs` | service store (`services/solidbom-server/src/projects.rs`) → desktop controller, SB-11, SB-15, SB-19 | contract `projects` tests; service `tests/projects.rs`; version 2 route tests |
| Engineering review and flat grouping | 1 (additive to API 2) | [v2/engineering-review-and-grouping.md](v2/engineering-review-and-grouping.md) | `engineering.rs` (`RegisterAnalysis`, `GroupSolids`, `AssociateComponentPart`, `CompleteMeasurement`, `ConfirmParts`, `ReviewedComponent`, `ReviewedPart`, `TrainingExampleMetadata`) | service engineering store and routes → desktop review/selection adapters, SB-13, SB-14, SB-15, SB-19 | contract `engineering` tests; service `tests/engineering.rs`; engineering route tests |
| Sourcing and manufacturing definitions | 1 (additive to API 2) | [v2/sourcing-and-manufacturing.md](v2/sourcing-and-manufacturing.md) | `manufacturing.rs` (`SourcedPart`, `RoutingTemplateVersion`, `PublishRoutingTemplate`, `AppliedRouting`, `RoutingOperation`, `RoutingWrite`, `AttachedCostLine`, `EditAttachedCosts`, `ManufacturingRecordDocument`, `ManufacturingWriteOutcome`, `CycleBasis::PerCycle`) | service manufacturing store and routes → SB-16 cost evaluation, SB-19 readiness, SB-20 adoption, desktop manufacturing editor (next increment) | contract `manufacturing` tests; service `tests/manufacturing.rs`; manufacturing route tests |
| Recognition and catalog suggestions | 1 (additive to API 2) | [v2/recognition-and-catalog-suggestions.md](v2/recognition-and-catalog-suggestions.md) | `recognition.rs` (`CategoryCode`, `SuggestionEvidence`, `ClassificationSuggestion`, `CatalogMatchSuggestion`, `CatalogMatchCandidate`, `ConfirmedCatalogReference`, `ProjectCatalogOverride`, `CatalogRecord`, `RecordSuggestions`), additive `engineering.rs`/`types.rs`/`shared_data.rs` fields; proposed by SB-13 under the owner's 2026-11-09 instruction and not coordinator-locked | engine analytic recognition/matcher → service engineering store and suggestion route → desktop review adapters, SB-14, SB-15, SB-19 | contract `recognition`/`engineering` tests; service `tests/engineering.rs`; engine `recognition` tests |
| Shared demand and manufacturing costs | 1 (additive to API 2) | [v2/shared-demand-and-manufacturing-costs.md](v2/shared-demand-and-manufacturing-costs.md) | `costing.rs` (`ProjectCosting`, `CostingVolumePolicy`, `ExchangeRate`, `PurchasePriceTable`, `UpdateProjectCosting`, `BaselineInputs`, `BaselinePartInput`, `BaselineProductInput`, `BaselineOutcome`, `BaselineSnapshot`, `BaselineCalculationList`, `CalculateBaseline`, `ProductRevisionSelection`) | service costing store (`services/solidbom-server/src/costing.rs`) and the costing routes plus the pure calculator (`crates/solidbom-costing/src/baseline.rs`) → SB-17, SB-18, SB-19, SB-20 and the desktop costing surface (later increment) | contract `costing` tests; `solidbom-costing` baseline arithmetic tests; service `tests/costing.rs`; costing route tests |
| Feasibility rules and exceptions | 1 (additive to API 2) | [v2/feasibility-rules-and-exceptions.md](v2/feasibility-rules-and-exceptions.md) | `feasibility.rs` (`FeasibilityRuleTableVersion`, `CreateFeasibilityRuleTableVersion`, `RecordFeasibilityAssessment`, `FeasibilityComponentEvidence`, `FeasibilityFinding`, `AcceptFeasibilityException`, `FeasibilityRecordView`, `evaluate_feasibility`) | service feasibility store and routes → desktop feasibility/readiness surface, SB-19, SB-20 | contract `feasibility` tests; service `tests/feasibility.rs`; feasibility route test |
| Tooling investment and recovery | 1 (additive to API 2) | [v2/tooling-investment-and-recovery.md](v2/tooling-investment-and-recovery.md) | `tooling.rs` (`ProjectTooling`, `ToolingEntry`, `ToolingOwner`, `ToolingTreatment`, `ToolingRecoveryInputs`, `ToolingRecoveryChoice`, `ToolingRecoveryBasis`, `ToolingDemand`, `ToolingInputs`, `ToolingOutcome`, `UpdateProjectTooling`, `EvaluateToolingRecovery`) | service tooling adapter (`services/solidbom-server/src/tooling.rs`) and the tooling routes plus the pure calculator (`crates/solidbom-costing/src/tooling.rs`) → SB-18, SB-19, SB-20 and the desktop tooling surface (later increment) | contract `tooling` tests; `solidbom-costing` tooling arithmetic tests; service `tests/tooling.rs`; tooling route tests |
| Commercial scenarios | 1 (additive to API 2) | [v2/commercial-scenarios.md](v2/commercial-scenarios.md) | `commercial.rs` (`ProjectCommercial`, `ScenarioManualPrice`, `CommercialScenarioContent`, `CommercialScenario`, `CreateCommercialScenario`, `UpdateCommercialScenario`, `CommercialScenarioWriteOutcome`, `CommercialScenarioList`, `CompareCommercialScenarios`, `CommercialScenarioComparison`, `CommercialInputs`, `CommercialOutcome`, `UpdateProjectCommercial`) | service commercial adapter (`services/solidbom-server/src/commercial.rs`) and the commercial routes plus the pure calculator (`crates/solidbom-costing/src/commercial.rs`) → SB-19, SB-20, internal exports and the desktop comparison surface (later increment) | contract `commercial` tests; `solidbom-costing` commercial arithmetic tests; service `tests/commercial.rs`; commercial route tests |
| Activation and accepted baselines | 1 (additive to API 2) | [v2/activation-and-accepted-baselines.md](v2/activation-and-accepted-baselines.md) | `activation.rs` (`ActivationReadinessRequest`, `ActivationReadinessReport`, `MemberReadiness`, `ReadinessBlocker`, `CoordinationReport`, `ChangedSharedDefinition`, `AcceptedSharedDefinition`, `ActivateRevisions`, `ActivationReceipt`, `ActivationMember`, `ActivationFact`, `AcceptedBaseline`, `AcceptedBaselineSummary`, `AcceptedBaselineList`, `ActivationRefusal`) | service activation store and routes → desktop readiness/activation surface (later increment), SB-20 changed-baseline facts, SB-21 exports | contract `activation` tests; service `tests/activation.rs`; activation route tests |
| Dependency updates and adoption | 1 (additive to API 2) | [v2/dependency-updates-and-adoption.md](v2/dependency-updates-and-adoption.md) | `adoption.rs` (`SharedUpdateKind`, `SharedUpdate`, `SharedUpdateList`, `ProjectDependency`, `DependencyDecisionAction`, `DependencyDecision`, `ProjectDependencyReport`, `DraftStaleReason`, `DraftResultStatus`, `FlaggedScenario`, `ImpactedProject`, `DependencyProjectList`, `DependencyDecisionItem`, `RecordDependencyDecisions`, `DependencyDecisionReceipt`, `DependencyFact`, `RecalculateDraft`, `DraftPublication`, `DependencyRefusal`), the additive `ProjectWriteOutcome.draft` field in `projects.rs` | service adoption store (`services/solidbom-server/src/adoption.rs`), the project write path and the version 2 adoption routes → desktop update/adoption surface (later increment), SB-21 exports, SB-22 update recovery | contract `adoption` tests; service `tests/adoption.rs`; adoption route cases in `tests/api.rs` |

The version 2 routes and the `forbidden` code extend the HTTP API row above; the version 1 error
body, request-id idempotency and immutable-version rules still apply. Reference records gain
logical ids, version chains, explicit units and optional money, so a build before SB-09 must not
read an SB-09 service. The project record is additive: the version 1 project draft/version routes
stay unchanged. The activation, readiness and accepted-baseline routes are published by the
activation contract above; the shared-update, dependency-decision and draft-recalculation routes are
published by the dependency-updates contract above. The dependency-updates contract refines the
SB-16 app-configuration resolution order for a project that adopted an app-configuration version
(the latest adopted decision, then the newest snapshot's recorded version, then the latest published
version); a project with neither keeps the published latest-version behaviour, and no wire type,
route or error code changes.
Later packages extend this registry only through coordinator review of the affected consumers.

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
