# Issue #69 evidence — display attributes in a reviewed source

Real application windows captured through the control channel (`ui.screenshot`) on the ticket
machine, with an isolated home folder (no earlier preferences, autosave or crash reports) and no
SolidBOM service: the review imports and analyses the source locally.

All captures use the same requested 2800x800 logical viewport, captured at 2x (5600x1600 pixels),
the light theme, the BOM workspace, and the tracked synthetic fixture
`tests/solidbom/fixtures/nested_repeated.step`. Each action is captured in a fresh session: import,
expand the browser, select the `FramePlate` body, then attempt the issue's action. Before and after
therefore start from the same workspace, camera, viewport and selection.

| File | Revision | Shows |
|---|---|---|
| `before/show-hide.png` | base `14c3d7ae3` | The issue's Show/Hide step: press `V` with the reviewed `FramePlate` body selected. The toast reads `Show/Hide is not allowed: the imported source is read-only while it is reviewed` and the body stays visible. |
| `before/material.png` | base `14c3d7ae3` | The issue's physical-material step: body menu → Physical Material. The toast reads `` `material.assign` is not allowed: the imported source is read-only while it is reviewed `` and no dialog opens. |
| `before/appearance.png` | base `14c3d7ae3` | The issue's appearance step: body menu → Appearance. The toast reads `` `appearance.assign` is not allowed: the imported source is read-only while it is reviewed `` and no dialog opens. |
| `after/show-hide.png` | fix `913b9cdd9` | The same Show/Hide step: the `FramePlate` body is hidden in the viewport and no refusal is reported. |
| `after/material.png` | fix `913b9cdd9` | The same physical-material step completes: `FramePlate` was set to Steel in the dialog (`inspect.measure` reports Steel, density 7.85 g/cm³) and the reopened Physical Material dialog shows Steel and its density. |
| `after/appearance.png` | fix `913b9cdd9` | The same appearance step completes: the Wood - Honey Oak look picked in the Appearance dialog is applied to `FramePlate` (`appearance.list` reports `#be9158` with its PBR response) and the viewport shows it. |

`captures.json` records each action's reply, status, dialog, hidden bodies, measured physical
material, applied look and the step-by-step values for the before and after runs.

The change is display-only: the reviewed source bytes do not change, the reviewed mapping still
places every occurrence, and a reopened project imports the source's own visibility, looks and
materials again. Undo stays refused, so a display change is reverted by re-applying the old value;
the pull request description records that limitation.

No generated review media is part of the pull request branch.
