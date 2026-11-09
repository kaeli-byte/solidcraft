# SB-09 evidence

Real application windows captured through the control channel (`ui.screenshot`) on a machine with
the development service on loopback and a disposable PostgreSQL database.

All captures use the same 2800x1600 viewport, the light theme, and an opened synthetic fixture
(`tests/solidbom/fixtures/nested_repeated.step`).

| File | Revision | Identity | Shows |
|---|---|---|---|
| `before-bom-project-review.png` | base `3c355556b` | combined development token | BOM workspace and review inspector only; no shared-data administration surface exists. |
| `after-admin-administration.png` | publish/record branch, review-round-1 head | combined (Engineer+Administrator) | Administration surface: material and cost-center version chains with `Missing price` and explicit `Price: 0 CNY`, app configuration history, recent reference changes, publish forms. |
| `after-engineer-bom.png` | publish/record branch, review-round-1 head | Engineer only | The same reviewed project and workspace; no administration entry and no publish controls. |
| `after-admin-only-administration.png` | this branch | Administrator only | Administration surface opens without a project, because shared data is organization-level. |

`api-evidence.json` is the recorded service response for the permission matrix, the version
publishing/retry/conflict run, explicit-zero versus missing prices, and the refused app cost
configurations. It contains no token, no database path and no customer data.

The review-round-1 head refreshed the two after captures: the material selector now labels every
version with its number and price, and a publish form keeps its draft until the service confirms
the request. The round-1 selector and form-retention behaviour is covered by UI tests rather than a
static capture.

No generated review media is part of the SB-09 pull request branch.
