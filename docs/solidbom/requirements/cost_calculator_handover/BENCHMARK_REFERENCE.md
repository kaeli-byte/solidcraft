# CAD benchmark source inventory

Reviewed: 2026-11-06. Owner-supplied folder: `benchmarking CAD/`.

## What was inspected

Ten STEP text files. Inspection covered file sizes, declared schema and lexical counts of selected STEP entity types. This is not a CAD-kernel import, geometric validation, recognition benchmark, or timing benchmark. Entity counts are not physical component counts or verified BOM quantities. Multiple bodies may represent layers, supplementary geometry or several bodies of one component.

| File | Size (MiB) | Declared schema family | PRODUCT entries | NEXT_ASSEMBLY_USAGE_OCCURRENCE entries | MANIFOLD_SOLID_BREP entries | SHELL_BASED_SURFACE_MODEL entries |
|---|---:|---|---:|---:|---:|---:|
| 0727 8201 004.stp | 3.51 | CONFIG_CONTROL_DESIGN | 4 | 3 | 6 | 0 |
| 201130300463.stp | 14.73 | AP242 | 1 | 0 | 15 | 1 |
| 201130300467.stp | 4.36 | AP242 | 1 | 0 | 3 | 2 |
| 201130300468.stp | 4.22 | AP242 | 1 | 0 | 3 | 1 |
| 201130300469.stp | 6.08 | AP242 | 1 | 0 | 6 | 2 |
| 201130300470.stp | 5.95 | AP242 | 1 | 0 | 6 | 2 |
| 201130300471.stp | 4.56 | AP242 | 1 | 0 | 3 | 2 |
| 201130300475.stp | 9.67 | AP242 | 1 | 0 | 21 | 6 |
| 201130300476.stp | 19.00 | AP242 | 1 | 0 | 14 | 0 |
| 201130300477.stp | 18.82 | AP242 | 1 | 0 | 14 | 0 |

All files inspected declare SI millimetre length units. AP242 here abbreviates the full declared `AP242_MANAGED_MODEL_BASED_3D_ENGINEERING_MIM_LF` schema. Headers identify CATIA STEP exchange; this remains STEP input and does not require native CATIA support.

## Material implications

- `0727 8201 004.stp` provides evidence of explicit assembly structure and multiple solid representations, useful for hierarchy-preservation testing.
- Nine AP242 files have one PRODUCT entry, no NEXT_ASSEMBLY_USAGE_OCCURRENCE entries and multiple solids; seven also contain surface-model entries. They are candidates for flattened/multi-body testing, not proof of one body per component. A real CAD importer must confirm usable grouping/structure before deciding the component mapping.
- Names on bodies/presentation items may help engineer review but are not verified component identities or independent measurement references.
- The supplied set spans solids and surfaces and modest file sizes. It does not establish support for larger files, pathological topology, every recognition category, or deeper assembly nesting.

## Still needed for an accuracy benchmark

No separate ground-truth classifications, dimensional reference sheets, or expected component mappings were found in this folder. STEP geometric parameters alone are not independent engineering verification of the extraction algorithm being tested.

**Owner update 2026-11-06:** the owner will provide independent reference data. Keep benchmark accuracy/coverage results and numerical processing-time acceptance targets pending until the references are available and the actual benchmark is run. Do not fabricate expected values or treat the supplied STEP geometry as its own verification. No reference-sheet creation is requested at this stage.

For a reviewed subset, record expected component grouping/identity, category, applicable critical measurements, units and the source/person verifying them. Multiple tube bodies/layers need an explicit expected component mapping. Record intentionally unavailable surface measurements separately from expected successfully extractable measurements.

Do not label these examples as training/held-out validation splits until related products/revisions and repeated component leakage have been checked. Do not report recognition accuracy or ±0.1% compliance from this inventory.

## Confirmed flattened-file handling — 2026-11-06

The MVP supports a flat component review for files without a usable component hierarchy. Engineers confirm which solids/surfaces belong to each component, with reviewed associations kept separately from the preserved original geometry/structure. Do not require a hierarchy-preserving re-export, silently invent nested assemblies, or identify one component per solid. Existing customer assembly trees still follow the hierarchy-preservation rule; this fallback does not authorize restructuring them.
