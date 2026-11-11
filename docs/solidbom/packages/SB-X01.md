# SB-X01 — Machine hourly-rate construction and sensitivity

Status: in progress; separately gated extension (outside the approved prototype and not an MVP
completion dependency). The owner instructed implementation on issue
[#38](https://github.com/kaeli-byte/solidcraft/issues/38) on 2026-11-11; the machine hourly-rate
contract is locked at `317e00979`, and the pure engine and service increment are in progress. The
desktop administration surface and the owner allocation/validation decisions below remain open.

## Objective

Construct versioned upstream cost-center rates from approved machine assumptions, with explained components and nonmutating loading sensitivity.

## Requirements

Read all five [machine hourly-rate handover](../requirements/machine_hourly_handover/00_HANDOVER.md) documents, especially its defects/decisions, and [APP_SPECIFICATION](../requirements/cost_calculator_handover/APP_SPECIFICATION.md), sections 11 and 14. Workbook recommendations do not expand approved product scope or establish company defaults.

## Dependencies

Depends on: SB-09

Require integrated reference/rate-version contracts plus explicit extension authorization and allocation/validation resolutions below.

## Scope

One deterministic engine handles capacity, loading/availability, depreciation, fixed/variable/standby/labor components and annual totals. Publish immutable approved fully absorbed rates through SB-09. Sensitivity copies input assumptions and changes loading without changing the active version. Machine investment only constructs the upstream rate: no extra project CAPEX, availability multiplier or depreciation charge. Project forecasts cannot derive loading or auto-select new annual rates; adoption stays explicit. Keep the prototype's 120 CNY/hour fixture unchanged.

## Interfaces

Lock machine identity, immutable input/result versions, validation diagnostics and publish/sensitivity commands. Rate publication includes cost-center/rate-version IDs, currency, `ACTUAL_RUNNING_HOUR`, fully absorbed basis and provenance. Undefined hourly rates are null and cannot be published as usable production rates. Cash/loaded-hour outputs are distinct analysis fields, never substituted silently for the selected production rate.

## Ownership

Own proposed machine-rate engine, extension service/admin adapters and area tests. SB-09 retains shared reference publication and authorization. Reserve shared rate contracts, migrations, dependencies and admin registration before work. Own `execution/SB-X01.md` only; no concurrent edits to downstream costing.

## Acceptance

- Supplied [synthetic golden vector](../requirements/machine_hourly_handover/04_PARITY_DEFECTS_AND_TEST_PLAN.md) gives 960 running hours, 111,800 annual full cost and 116.458333333… per running hour; at loading 0.50 the rate is 131.5625. Verify component and annual reconciliations with locked tolerances.
- Independently reproduce eight workbook baselines and 64 sensitivity cases as arithmetic references, not approved company rates. Oven Forming's missing financial year prevents publication; missing IDs and blank authoritative inputs cannot silently become zero.
- Sensitivity at base loading equals the base rate and leaves inputs unchanged. Project snapshots preserve adopted versions; no automatic inflation, repeated availability/depreciation or added project CAPEX. Capture extension UI evidence if implemented.

## Assignment gate

Require explicit extension scope authorization, integrated SB-09 and locked contracts/ownership. Resolve premises versus SG&A, air/electricity and water basis, staffing/loading assumptions, residual/depreciation convention, Oven year/power and missing cost-center codes. Reconcile zero-running-hours scenario versus publication validation, future-acquisition-year rules and loaded-hour reporting semantics. Do not assume 20% residual or illustrative workbook inputs are approved defaults; unresolved allocation/validation decisions keep this package planned/blocked rather than ready.

## Handoff

Record owner resolutions, approved versus parity-only inputs, formula/version and rate-publication contracts, independent results and actual checks/evidence. Link adopted-rate compatibility to SB-09/SB-20 without making this extension a prerequisite for SB-22.
