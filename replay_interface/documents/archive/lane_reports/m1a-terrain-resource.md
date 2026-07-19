# LANE REPORT — M1a: Terrain + Resource map modes (Phase E3, MAPMODE rows 6 & 9)

Worktree `C:\Users\Example\hoi4_lanes\m1a-terrain-resource`, branch
`lane/m1a-terrain-resource`. Cargo root `replay_interface/native`. Derives the
two file-static, law-known map modes from the MAPMODE coverage matrix:

- **Terrain** (toolbar button 6, `ui_mapmode_terrain` / F5).
- **Resource** (toolbar button 9, `ui_mapmode_resource` / F8).

Both fills are computed replay-side from shipped installed content, never recorded
(standing override). Delivered as one new module,
`map_viewer/src/map_mode_terrain_resource.rs`, keyed per-province / per-state
like the political path (`keying.rs` + `political_color_synthesis.rs`).

`replay_core` and `extractor/` untouched (read-only). No git commit
(orchestrator commits).

## Gate numbers (all green)

| Gate | Result |
|---|---|
| `cargo build -p map_viewer` | **Finished, exit 0**, no new warnings from this module |
| `cargo test -p map_viewer --lib` | **709 passed / 0 failed / 1 ignored** |
| module tests `map_mode_terrain_resource` | **11 passed / 0 failed** (2 vanilla parse gates executed with `BATTLE_PLAN_GAME_DIR` set, not skipped) |

## Terrain mode — law (all FILE, byte-cited)

Per-province fill = the province's terrain **category** colour.

1. **Province → category (FILE).** `map/definition.csv` column 6 (0-indexed
   `id;r;g;b;type;coastal;terrain;continent`) is each province's terrain
   category name, already parsed into `ProvinceDefinition::terrain`.
2. **Category → fill RGB (FILE).** `common/terrain/00_terrain.txt`
   `categories = { <name> = { color = { r g b } } }`. All **14** category
   colours cited verbatim (`VANILLA_TERRAIN_CATEGORY_COLORS`) and re-parsed via
   the mod-aware resolver so mods override. Only the `categories` block is read;
   the graphical `terrain = { }` palette block is deliberately ignored (the
   per-province category is the exact, complete key).

Byte-cited colour table (`00_terrain.txt`, `categories`):

| category | RGB | line | category | RGB | line |
|---|---|---|---|---|---|
| unknown | 255,0,0 | 9 | urban | 120,120,120 | 122 |
| ocean | 40,83,176 | 14 | jungle | 127,191,0 | 145 |
| lakes | 58,91,255 | 23 | marsh | 76,96,35 | 175 |
| forest | 89,199,85 | 30 | desert | 255,127,0 | 202 |
| hills | 248,255,153 | 56 | water_fjords | 75,162,198 | 224 |
| mountain | 157,192,208 | 81 | water_shallow_sea | 56,118,217 | 269 |
| plains | 255,129,66 | 108 | water_deep_ocean | 2,38,150 | 284 |

**Gate `gate_vanilla_terrain_map_mode`:** 14 categories parsed == cited constant
byte-for-byte; **13,414 / 13,414 provinces classified, 0 unclassified**; the 11
categories actually used by vanilla provinces with exact histogram —
ocean 3133, plains 3031, forest 2364, mountain 1708, hills 1352, desert 815,
jungle 477, urban 265, marsh 142, lakes 126, unknown 1 (sum = 13,414).

Fail-closed: a province whose terrain field names no defined category gets NO
fill and is collected in `TerrainMapMode::unclassified` (never invented).

## Resource mode — law (deposits FILE, per-resource fill UNKNOWN fail-closed)

1. **Resource definitions (FILE).** `common/resources/00_resources.txt`
   `resources = { <name> = { icon_frame = N } }` → 7 resources, `icon_frame`
   1..7 in file order: oil 1, aluminium 2, rubber 3, tungsten 4, steel 5,
   chromium 6, coal 7. On-map identity = strip icon `GFX_resources_strip` →
   `gfx/interface/resources_strip.dds` (`interface/general_stuff.gfx:185-186`).
2. **Deposit locations + magnitudes (FILE).** Per-state
   `resources = { <name> = <amount> }` block in `history/states/*.txt`
   (block-scoped, comment-stripped; only defined resources counted; dated-block
   / out-of-block keys excluded).
3. **Control keying (FILE).** Which country a deposit belongs to over the replay
   is the recorded state controller — the existing keying (requirements item 2 /
   `KeyingTables`); no new data dependency. State fill in resource mode reuses
   the political control fill (law-known, done).
4. **Per-resource map-mode FILL colour / threshold — UNKNOWN, fail-closed.**
   NO shipped file defines a per-resource map-mode fill colour or a display
   threshold. `resource_state_fill()` returns `None` for every state (no guessed
   colour); the FILE deliverable is the deposit set + icon frame
   (`resource_state_overlay`). Cracking = engine RE of the resource mapmode
   layer + an F8 capture.

**Gate `gate_vanilla_resource_deposit_set`:** 7 resource defs (frames 1..7,
file order); **467 states with ≥1 deposit, 717 total (state,resource) entries.**
Per-resource state counts + summed amounts (all cited):

| resource | states | total | resource | states | total |
|---|---|---|---|---|---|
| steel | 166 | 2591 | oil | 83 | 1209 |
| coal | 153 | 1463 | tungsten | 83 | 1346 |
| aluminium | 95 | 1201 | rubber | 44 | 1032 |
| chromium | 93 | 1345 | | | |

Sum of per-resource state counts = 717 = total entries (consistency gate).

## Public API (exposed via `map_viewer` lib)

Terrain: `parse_terrain_category_colors`, `terrain_category_colors_from_resolver`,
`province_terrain_fill`, `classify_terrain_map_mode`,
`terrain_map_mode_from_resolver`, `TerrainMapMode`,
`VANILLA_TERRAIN_CATEGORY_COLORS`, `TERRAIN_CATEGORIES_SOURCE`,
`TERRAIN_DEFINITION_CSV_SOURCE`.

Resource: `parse_resource_definitions`, `resource_definitions_from_resolver`,
`parse_state_resource_deposits`, `classify_resource_deposits`,
`resource_deposit_set_from_resolver`, `resource_state_overlay`,
`resource_state_fill` (→ `None`, UNKNOWN), `ResourceDefinition`,
`ResourceDepositSet`, `ResourceStateOverlay`, `VANILLA_RESOURCE_DEFINITIONS`,
`RESOURCE_DEFINITIONS_SOURCE`, `RESOURCE_DEPOSITS_STATES_DIR`,
`RESOURCE_ICON_STRIP_SOURCE`.

## Doc / matrix updates

- `MAPMODE_COVERAGE_MATRIX.md`: row 6 (Terrain) → **DONE (data law)**; row 9
  (Resource) → **deposits DONE, fill UNKNOWN fail-closed**.
- `FIDELITY_LEDGER.md` §2.20 (new): terrain+resource mapmode rows — 4 FILE,
  1 UNKNOWN (per-resource map fill). §1 aggregate recount left to the recount
  lane / orchestrator.
- `REPLAY_PRODUCTION_PLAN.md` E3 aggregate row: left to the orchestrator (E3 is
  the shared map-mode row edited by all mapmode lanes; per-mode status of record
  is the matrix, updated above).

## Files

- New: `replay_interface/native/map_viewer/src/map_mode_terrain_resource.rs`
  (data laws + 11 tests, 2 vanilla-gated).
- Edited (single lines / own rows): `map_viewer/src/lib.rs` (`mod` + `pub use`),
  `MAPMODE_COVERAGE_MATRIX.md` (rows 6 & 9), `FIDELITY_LEDGER.md` (§2.20).
- Untouched: `replay_core/*`, `extractor/*`, all other modules.

## Honesty / residuals

- Terrain data law is CLOSED (0 unclassified). Rasterising the per-province fill
  into the live gradient-border field is capture-gated — shared with every other
  map mode, not a terrain-specific gap.
- Resource per-resource map-mode FILL colour + amount threshold is the one
  UNKNOWN: engine-hardcoded, no shipped-file source, fail-closed to `None`.
