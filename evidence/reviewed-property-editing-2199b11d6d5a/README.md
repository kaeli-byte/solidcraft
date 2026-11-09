# Issue #50 evidence — renaming a body or component in a reviewed source

Real application windows captured through the control channel (`ui.screenshot`) on the ticket
machine, with an isolated home folder (no earlier preferences, autosave or crash reports) and no
SolidBOM service: the review imports and analyses the source locally.

All captures use the same requested 2800x800 logical viewport, captured at 2x (5600x1600 pixels),
the light theme, the BOM workspace, and the tracked synthetic fixture
`tests/solidbom/fixtures/nested_repeated.step`. The browser tree is expanded and scrolled the same
way in every capture, so before and after show the same rows. The steps are the issue's steps:
right-click the `Frame` component (or its `FramePlate` body) and choose Rename.

| File | Revision | Shows |
|---|---|---|
| `before-component.png` | base `a10b44b9f` | Rename on the `Frame` component: `Rename Component is not allowed: the imported source is read-only while it is reviewed`; the tree still lists `Frame:1`. |
| `before-body.png` | base `a10b44b9f` | Rename on the `FramePlate` body: `Rename Body is not allowed: the imported source is read-only while it is reviewed`; the body keeps its name. |
| `after-component.png` | fix `80618bab4` | The same rename now works: the tree lists `Reviewed part:1`. |
| `after-body.png` | fix `80618bab4` | The same rename now works: the tree lists `Reviewed body` under `Reviewed part:1`. |

`captures.json` records the review phase, the rename targets, the refusal messages on the base
build, the names after each menu action, and the capture sizes.

The rename is display-only: the reviewed source bytes do not change, the reviewed mapping still
places every occurrence, and a reopened project imports the source's own names again. The pull
request description records that limitation.

No generated review media is part of the pull request branch.
