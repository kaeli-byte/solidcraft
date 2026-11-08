# SB-07 before/after evidence

Real application windows driven through the SolidCraft control channel (`ui.screenshot`), with
the same synthetic fixtures, window size (content 1600×872), light theme, agent cursor off and
reviewed inputs in both phases. These files are evidence only: they are not part of the package
branch.

- Before: base revision `dbe088a368a4d0b9b6fa903469ca1235577121cf` (SB-06 merged).
- After: the SB-07 branch built from that revision; the PR records the implementation commit.
- `before-*` / `after-*` pairs use the same step of the same scenario:
  - `12-stale-after-cycle-60`: after the cycle change, before recalculation (cost display at two places, history, explicit Recalculate).
  - `16-save-failed-service-down`: save with the service stopped.
  - `17-*`: explicit retry after the service returns (before: refused; after: commits the same request once).
  - `22-flat-multi-body-calculated`: flat source with `unresolved_grouping` (before: partial amount shown as cost; after: no complete cost, table cell Unavailable).
  - `11-reopen-recovered-current`: reopened project after a full process restart.
  - `15d-open-surface-manual-cost`: open surface with a reviewed manual volume.

PNGs are the originals; these files are downscaled JPEG copies for the pull-request description.
The dialog text and values in the images are the application's own.
