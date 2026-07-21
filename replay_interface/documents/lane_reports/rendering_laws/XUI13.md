# XUI13 - templates and variants

Status: **TEMPORAL REPLAY DATA WIRED - ROOT VALIDATION, H-A27, AND PRODUCT DESIGNER SURFACE AUTHORITY REQUIRED**

Function address: **N/A (no Ghidra operation was needed or used)**  
Job ID: **N/A**  
Artifact path: `replay_interface/re_evidence/rendering_laws/XUI13/`  
Manifest SHA-256: **2525e8da946ebc7487b4b425d988f18ced57a6467c16af3e4c545609b52c765e**

## Result

The replay seam is implemented in `game_ui::design_catalog` for division templates plus ship, tank, aircraft, and general equipment variants. Portable schema v3 carries the first complete native A27 payload after removing relocation-only fields, followed by authenticated hourly full-row upserts, absence-derived tombstones, and exact order. The smaller UI records remain a projection mechanically regenerated from native rows. Admission requires explicit definition, owner, lifecycle, module, stat, upgrade, and display-order completeness; malformed, incomplete, missing-hour, or cross-hour inputs reject.

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

A27's temporal recorder handoff is wired. The recorder starts one hidden repeated-census worker before capture, stops it after the compact pipe closes, and refuses publication unless consecutive whole-sweep-sealed catalog hours exactly cover the replay spine. Portable relocation and replay admission succeed after the source timeline is deleted, while post-seal mutation rejects. Portable v1 is retired because it discarded native template composition/scalars, creator/origin and occurrence provenance, UTF-8 byte evidence, handles, and raw stat/upgrade values while labelling the projection complete. Portable v3 retains those fields and temporal mutation semantics. The full frozen 20,734,645-byte source hash and 6,407-record count remain historical single-hour evidence. Domain 78 supplies authenticated display order only for an hour reconstructed from A27; unknown IDs and missing/cross-hour joins reject.

`admit_design_catalog` admits only that complete authenticated A27 catalog plus the d78 order. Token order, map order, numeric ID order, localisation order, static source order, and the shallow legacy `replay_core::DesignsView` remain forbidden substitutes.

Two gates remain. Root-owned central validation must settle the temporal implementation, then H-A27 must prove three consecutive advancing hours with a real design mutation. If its external reader cannot keep pace, the retained machine failure becomes the precise requirement for an in-process mutation hook. Separately, a recording must authenticate designer surface activation and the effective-VFS semantic GUI-root/dependency closure for that surface. Until both are closed, historical designer canvases remain unavailable rather than borrowing final catalog state.

## Evidence

- `static_census.json`
- `promotion_contract.json`
- `verification.json`
- `evidence_manifest.json`
- `ALL_FINDINGS.md`
- `extractor/documents/dossiers/A27_DESIGN_CATALOG_INPUTS.json`
