# Design rule source review

Reviewed: 2026-11-06. Source folder: `design rules/`.

This is a transcription and applicability review of owner-supplied references, not confirmation that these historical documents represent the current DIN edition. Preserve their stated conditions and revisions. These sources supplement section 9 of APP_SPECIFICATION.md; no unprovided limits are inferred.

## Source inventory

| File | Content and status |
|---|---|
| [Material descriptions PDF](design%20rules/02.4_Materials_description%20_acc.%20DIN%2073378_.pdf) | Two scanned material designation tables, German and English; identifies PA6/PA11/PA12 variants. Historical DIN extract identified as 1996-02, reference updated 2007-05-14. Does not provide material density, purchase price, or bend-spacing limits. |
| [Material descriptions workbook](design%20rules/02.4_Materials_description%20_acc.%20DIN%2073378_.xlsx) | Sheet1 has document headings/text; the substantive tables are embedded images, not structured cell records. Do not treat this as a ready-to-import material database. |
| [Smooth tube dimensions/tolerances](design%20rules/03.1_smooth-tubes_dimensions+tolerances_DIN73378.pdf) | One page of nominal OD, ID, wall thickness and dimensional limits. DIN extract identified as 1996-02; update 2007-05-14. Its footnote constrains simultaneous use of dimensional extremes/coaxiality. These manufacturing tolerances are not the app's ±0.1% extraction acceptance tolerance. |
| [Smooth tube minimum bending radius](design%20rules/03.2_smooth-tubes_bending-radius.pdf) | One-page size-specific radius table, drawing reference ENG06517, update 2010-11-05. Applies to standard PA12 with supporting spring. |
| [Corrugated pipe minimum bending radius](design%20rules/04.1_Corrugated-tubes_Minimum-Bending-Radius.pdf) | One-page nominal connection-size radius table, dated 2007-05-14. Nominal size is connection-area inside diameter, not generic OD. |

Review method: extracted available PDF text, visually inspected all five PDF pages including scanned tables/footnotes, and inspected workbook sheet XML/cell content. Original sources unchanged.

## Smooth tube radius table

Units: mm. Size is OD × nominal wall thickness, not OD × ID. Wall thickness can be derived as (OD − ID) / 2 for the supported constant-diameter tube geometry, with nominal-size association separately reviewed.

| OD | Nominal wall thickness | Minimum radius R |
|---:|---:|---:|
| 4 | 1 | 15 |
| 5 | 1 | 20 |
| 5 | 1.5 | 20 |
| 6 | 1 | 20 |
| 8 | 1 | 25 |
| 8 | 2.25 | 25 |
| 9 | 1.5 | 30 |
| 9 | 3 | 30 |
| 10 | 1 | 30 |
| 12.5 | 1.25 | 35 |
| 12.5 | 2.1 | 35 |
| 15 | 1.5 | 45 |
| 15 | 2.5 | 45 |
| 18 | 1.5 | 45 |

Source conditions: standard material PA12 **and** supporting spring. Smaller radii require Engineering consultation and testing; the text notes dependence on bending angle and prefers larger radii. No angle-dependent formula is provided. Do not generalize these limits to every material or unsupported process, extrapolate missing sizes, or generate smaller angle-based limits from this table.

The app measures centerline radius by confirmed requirement. Preserve the source drawing with the imported rule; verify radius convention when validating source-to-app mapping rather than silently converting inner/outer radius conventions.

## Corrugated pipe radius table

Units: mm. Nominal size = inside diameter in the connection area.

| Nominal connection size | Minimum radius R |
|---:|---:|
| 6 | 20 |
| 8 | 20 |
| 10 | 25 |
| 12 | 25 |
| 15 | 30 |
| 19 | 35 |
| 27 | 45 |
| 33 | 60 |

This source does not by itself approve automatic corrugated-tube measurement in MVP. It must remain distinct from the agreed constant-diameter smooth-tube extraction scope; automated support requires a separately agreed geometry method and validation. The table can be retained as reference without silently expanding CAD support.

## Rule data still absent

- Minimum straight centerline length between consecutive bend tangent points.
- Required end straight length for fir-tree/rubber-barb assembly, including the distinction between insertion depth and additional assembly/tooling clearance.
- Connector-specific compatible tube OD/ID ranges.
- Numeric rules for materials/nominal sizes not covered by the supplied radius tables.
- Material density/pricing needed for geometry-to-cost conversion.

The material/dimensional references do not supply these missing manufacturing requirements. Do not derive connector compatibility solely from tube manufacturing tolerances.

## Confirmed MVP handling — 2026-11-06

Use only applicable supplied rules. For a missing numerical limit, an Administrator can enter an engineering-approved value with its source reference. Until provided, show the check as unresolved/not evaluated; do not report a pass. The already agreed exception workflow remains available with recorded justification. Retain the agreed spacing, compatibility and end-length checks in MVP rather than deferring them entirely. No general rule builder or interpolation engine is required.

**Confirmed 2026-11-06 — overlapping rules:** flag conflicting applicable rules for an Administrator to resolve. Do not silently choose a limit or introduce rule priorities/override precedence in MVP. An affected evaluation remains unresolved until the conflict is resolved or the agreed exception process is used.
