# LANE REPORT — C5 rail/supply overlay (files-first half)

Branch `lane/c5-rails`. Plan item **C5** + EXTRACTION_NEEDS **A8** context.
Scope delivered: parse + geometrically prepare the STATIC map railway/supply-hub
content from shipped installed content, with FILE/CAPTURE/UNKNOWN labels on every visual
parameter. BUILT-rail state over time stays extractor-side (A8) — not touched.

## What landed

- **New module `map_viewer/src/rail_files.rs`** — the geometry + render-law layer.
  It does NOT re-parse the files: the raw parse already existed in
  `map_static_data.rs` (`RailwaySegment{level,provinces}`, `SupplyNode{level,
  province_id}`, `railway_segments_from_resolver`, `supply_nodes_from_resolver`).
  This lane consumes those (no duplication) and adds:
  - `RailGeometryDefines` + resolver reader for the `00_graphics.lua` rail defines.
  - `compute_province_centroids` — one raster pass (reuses `load_definition_csv` +
    `load_provinces_bmp`, the same province-position source `map_furniture` uses).
  - `rail_polylines` / `supply_hub_markers` — world-space geometry, fail-closed.
  - `rail_render_law` — machine-readable FILE/CAPTURE/UNKNOWN disposition.
  - `rail_file_derived_layer_from_resolver` — full pipeline + stats gates.
  Wired into `lib.rs`; all reads go through `GameFileResolver` (mod-aware).
- **FIDELITY_LEDGER.md §2.18** rail rows rewritten (5 rows: 3 FILE + 2 UNKNOWN);
  header count `1→4 FILE`, totals `78→81 FILE`; debt #22 annotated.
- **REPLAY_PRODUCTION_PLAN.md** C5 row → FILES-FIRST DONE.
- **EXTRACTION_NEEDS.md A8** annotated (static half done; row now = built state only).
  No NEW recording dependency — built-rail state was already A8 / REPLAY_DATA_
  REQUIREMENTS §11.

## Derived format spec (verified against the shipped vanilla files)

- `map/railways.txt` — one railway per line: `<level> <province_count>
  <province_id …×province_count>`. 909 lines; column 2 equals the trailing-id count
  on every line; levels observed 1..=4. Engine model allows level up to 5
  (`RAILWAY_MAP_ARROW_THICK_LEVEL_THRESHOLD`).
- `map/supply_nodes.txt` — one hub per line: `<flag> <province_id>`. 727 lines;
  the leading column is always `1` in vanilla (an enabled flag; full domain
  engine-side, carried verbatim as `SupplyNode.level`).
- Render chain (FILE): `gfx/models/supply/railroad.shader` (Effect `railroad`)
  draws a **textured ribbon** (world-space VB, samples `RailTexture` =
  `railroad.dds` 64×320 Wrap/Wrap; `railroad_damaged.dds` variant; blend
  src_alpha/inv_src_alpha, DepthEnable=no). Cutoffs/offsets from
  `common/defines/00_graphics.lua`: `RAILWAY_Y_OFFSET=0.9`,
  `RAILWAY_CAMERA_CUTOFF=300`/`_SPEED=3`, `RAILWAY_ICON_CUTOFF=900`. Supply hub =
  sprite `GFX_supply_hub_icon_strip` (`interface/supply.gfx`; 180×36, 5 frames).

## Gate numbers (`gate_vanilla_rail_geometry_and_referential_integrity`)

| Metric | Value |
|---|---|
| railway lines | **909** |
| level histogram | **1:467, 2:406, 3:29, 4:7** (5: none) |
| supply nodes | **727** (all flag=1) |
| distinct rail provinces | **3126** |
| distinct supply provinces | **727** |
| rail polylines built / dropped | **909 / 0** |
| supply hub markers | **727** |
| provinces missing from `definition.csv` | **0** |
| provinces absent from raster (no centroid) | **0** |
| polyline points in 5632×2048 bounds | **all** |
| path continuity (points == path provinces per railway) | **holds for all 909** |

Tests: **10 rail tests pass** (2 vanilla gates + 8 unit). Full suite:
`cargo build -p map_viewer` + `cargo test -p map_viewer` green — **652 passed / 0
failed**. No existing module renamed/moved; `extractor/`, `replay_core`, fixtures
untouched. No commits made.

## UNKNOWN table (fail-closed; no invented values)

| Parameter | Class | Why UNKNOWN | Closure path |
|---|---|---|---|
| Per-province rail anchor | UNKNOWN | Polyline uses province pixel **centroid = DERIVED_APPROXIMATION**; engine's exact per-province rail entry/exit point untraced | Capture rail draw VB (plan CAP9 cities/rails) or RE the ribbon builder in `hoi4.exe` |
| Default-ribbon world width per level | UNKNOWN | `railroad.shader` width-scale branch is disabled ("doesn't look good"); ribbon width is baked into the engine-built VB, not in files. The `RAILWAY_MAP_ARROW_*_LEVEL_THRESHOLD` thin/medium/thick widths are **SUPPLY-MAP-MODE arrows**, not the default ribbon | Capture rail VB / RE ribbon builder |
| Inter-centroid curve / smoothing | UNKNOWN | Polyline emitted as straight centroid-to-centroid segments; any engine spline untraced | Capture rail VB / RE |
| RailTexture UV tiling rate along track | UNKNOWN | `vUV.x` wrap rate baked into the VB; not in files | Capture rail VB / RE |
| Supply-hub icon frame index (0..4) | UNKNOWN | Which of the 5 strip frames a hub shows is engine state (hub level/status) | RE or a labelled supply-map capture |
| Built-rail state over time (constructed/damaged/cooldown) | UNKNOWN (recording) | Game state, not files | **EXTRACTION_NEEDS A8** (extractor-side), REPLAY_DATA_REQUIREMENTS §11 — out of C5 scope |

## RE targets named for a follow-up lane (no binary RE done here, per lane rules)

- Railroad-ribbon **CPU VB builder** in `hoi4.exe` (turns the `railways.txt`
  province path into the world-space triangle-strip with per-level width + UV
  tiling + any inter-centroid curve). Closing this promotes ribbon width / UV /
  curve / per-province anchor from UNKNOWN to FILE/RE-LAW.
- Supply-hub **icon-frame selection** logic (maps hub state → one of the 5
  `GFX_supply_hub_icon_strip` frames).
