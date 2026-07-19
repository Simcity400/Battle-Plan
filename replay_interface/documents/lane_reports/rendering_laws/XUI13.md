# XUI13 - templates and variants

Status: **REPLAY DATA/CATALOG PROMOTION READY - PRODUCT DESIGNER SURFACE AUTHORITY REQUIRED**

Function address: **N/A (no Ghidra operation was needed or used)**  
Job ID: **N/A**  
Artifact path: `replay_interface/re_evidence/rendering_laws/XUI13/`  
Manifest SHA-256: **2525e8da946ebc7487b4b425d988f18ced57a6467c16af3e4c545609b52c765e**

## Result

The production replay seam is implemented in `game_ui::design_catalog` for division templates plus ship, tank, aircraft, and general equipment variants. It preserves exact producer order for designs, module slots, stat rows, and upgrades. Admission requires explicit definition, owner, lifecycle, module, stat, upgrade, and display-order completeness; malformed or incomplete inputs reject.

The focused suite covers all five families and negative provenance, identity, completeness, duplicate, non-finite, negative-upgrade, and non-permutation cases.

Final canonical gate: XUI13 focused **10/10** plus the canonical A27 packaging/relocation gate **1/1**. The full shared-suite counts are recorded in the production integration report after the settled-tree double pass.

## Exact static evidence

The exact a729 executable and current vanilla parsers mechanically produce:

- 804 division templates, 256 unique names, 157 sub-unit definitions, 274 source files.
- 308 equipment archetypes, 313 effective modules, two last-wins module collisions, 37 upgrades.
- 80 ship hulls and 116 ship modules; 45/45 equipment files and 3/3 module files parse; zero warnings or selected ship ID collisions.

The authored corpus also contains 1,440 `create_equipment_variant` occurrences across 125 files and 436 `create_ship` occurrences across 21 files. These are recorded as mutation potential only and are never treated as current runtime catalog evidence.

These values are definition evidence, not live-catalog or display-order evidence.

## Production handoff and remaining boundary

A27's canonical recorder handoff is now closed. The recorder seals the pointer-free catalog at `static/design_catalog_live/a27_live_catalog.json` under the dedicated `design_catalog_live` role; portable relocation and replay admission succeed after the source artifact is deleted, and post-seal mutation rejects. The focused end-to-end gate `recorder_seals_relocates_and_admits_a27_xui13_catalog` passes 1/1. The full frozen 20,734,645-byte A27 source hashes to `6fa4386b9bd688df47a613b6268ccb0f125fcd2fe75edd9c484154ff38d5628b`; its projection produces exactly 6,407 records and excludes `pointer_identity`, `subunit_definition_pointer`, `game_state`, and `module_base`. Domain 78 supplies the authenticated current display order, and unknown IDs reject.

`admit_design_catalog` admits only that complete authenticated A27 catalog plus the d78 order. Token order, map order, numeric ID order, localisation order, static source order, and the shallow legacy `replay_core::DesignsView` remain forbidden substitutes.

The remaining product-pixel boundary is not catalog data: a recording must authenticate designer surface activation and the effective-VFS semantic GUI-root/dependency closure for that surface. Until then, the production app omits the designer canvas as a unit and does not parse a default GUI path.

## Evidence

- `static_census.json`
- `promotion_contract.json`
- `verification.json`
- `evidence_manifest.json`
- `ALL_FINDINGS.md`
- `extractor/documents/dossiers/A27_DESIGN_CATALOG_INPUTS.json`
