# Combat Indicator Law (battle bubbles)

Status: C3 files-first prep only. This document records what the replay can
derive offline from shipped the game files for the on-map combat indicators (land +
naval "battle bubbles") and what remains capture-gated for CAP11. No product
renderer should draw combat indicators from this draft yet.

All shipped-file reads in code go through `GameFileResolver` keyed by the
recording modset. The C3 decoder lives in
`replay_interface/native/map_viewer/src/combat_indicator_assets.rs` and reuses
the C1 (`unit_counter_assets.rs`) sprite/DDS/GUI parser helpers verbatim (no
forked parser); it exposes resolved sprites, progress bars, GUI layouts and a
province-keyed placement stub separately, and reports unresolved references
rather than silently remapping them.

## Source Inventory

| Row | Class | Derived fact | Source |
|---|---|---|---|
| Land combat container | FILE | `land_combat_mapicon` container, position `{0,0}`, declared size `35x34`, `clipping=no`. Children: `battle_indicator_direction` (rotation slot `1.0`, pos `{18,18}`), `bg_btn` (`GFX_mapicon_land_combat`, pos `{-2,-2}`), `mapicon_land_combat_frame` (`frame=1`), `mapicon_land_combat_bombardment` (`frame=1`), `combat_progress` textbox (`hoi_16mbs`, maxW 30 / maxH 23, placeholder `"?"`), `air_superiority_icon` (`GFX_battle_modifier_strip`, pos `{4,8}`). | `interface/mapicons.gui:1154-1213` |
| Naval combat container | FILE | `naval_combat_mapicon` container, declared size `33x43`. Children: `bg_btn` (`GFX_mapicon_naval_combat`, center, pos `{1,-30}`), `frame_btn` (`GFX_naval_hq_combat_frame`, center, pos `{17,-9}`), `attacker_adds_grid`/`defender_adds_grid`, `progress` (`GFX_navalcombat_bar_progress_attacker_154`), `progress_small` (`GFX_land_unit_combat_progressbar`), `attacker_grid`/`defender_grid` (slot `36x36`). | `interface/mapicons.gui:1076-1151` |
| Naval combat results container | FILE | `naval_combat_results_mapicon`, declared size `19x24`; `bg_btn` = `GFX_naval_combat_result`. | `interface/mapicons.gui:1060-1073` |
| Fleet-status combat count containers | FILE | `ongoing_combat_container` (`GFX_naval_combat_icon` + `count` textbox `hoi_16mbs`) and `combat_results_container` (`GFX_naval_combat_results_icon` + count), each `28x35`. | `interface/mapicons.gui:1459-1507` |
| Land combat sprites | FILE | `GFX_mapicon_land_combat` 38x37 (1f); `GFX_mapicon_land_combat_frame` 76x37 `noOfFrames=2` → 38x37; `GFX_mapicon_land_combat_bombardment_overlay` 38x37; `GFX_mapicon_land_combat_urban` 38x37; `GFX_battle_indicator_direction` 70x70 (7 mips). | `interface/mapicons.gfx:416-440` |
| Land battle animated strip | FILE | `GFX_landbattle_strip` is a `frameAnimatedSpriteType`, 1020x44, `noOfFrames=20` → 51x44, `animation_rate_fps=10`. (Engine-selected; not wired in the on-map GUI container.) | `interface/mapicons.gfx:43-49` |
| Air-superiority modifier strip | FILE | `GFX_battle_modifier_strip` 250x25 `noOfFrames=10` → 25x25; used by the land container's `air_superiority_icon`. | `interface/landcombat.gfx:188-192` |
| Naval combat sprites | FILE | `GFX_mapicon_naval_combat` 80x33 `noOfFrames=2` → 40x33; `GFX_naval_combat_result` 105x33 3f → 35x33; `_portstrikes`/`_navalstrikes` 102x34 3f → 34x34; `_convoys` 102x33 3f → 34x33; `GFX_mapicon_naval_battle_result` 66x33 2f → 33x33 (comment: frame 2 = port strike, frame 1 = naval battle); `GFX_mapicon_naval_battle_result_text` 17x17 DXT5; `GFX_naval_combat_icon` / `GFX_naval_combat_results_icon` 25x25. | `interface/mapicons.gfx:251-508` |
| Naval HQ combat frame | FILE | `GFX_naval_hq_combat_frame` 53x43 (single frame); referenced by the naval container `frame_btn`. | `interface/building_roster/naval_leader_module.gfx:38-41` |
| Combat progress bars | FILE | `GFX_navalcombat_bar_progress_attacker_154` declared `size={154,14}`, textures `bar_progress_loose_157x14.dds` + `bar_progress_win_157x14.dds` (both 157x14), `effectFile=gfx/FX/progress.lua`, horizontal. `GFX_land_unit_combat_progressbar` declared `size={40,5}`, textures 40x5. | `interface/navalcombat.gfx:476-483`, `interface/unitview.gfx:769-777` |
| Combat text font | FILE | `combat_progress` and the fleet-status combat counts use `hoi_16mbs`; font files `gfx/fonts/hoi_16mbs` + `_cryllic`, japanese override `gfx/fonts/japanese/hoi_16mbs`. | `interface/mapicons.gui:1199`, `interface/core.gfx:1093-1119` |
| Battle-bubble screen-space law | CAPTURE-GATED | Declared container sizes (`35x34` land, `33x43` naval, `19x24` naval-results) are the file "true size" hints, not the rendered quad. Rendered quad size, DPI scaling, zoom scaling, world/province anchor, and depth ordering against terrain are unmeasured. | needs CAP11 |
| Combat → frame/colour/variant selection | CAPTURE-GATED | Files expose the frame inventory (land frame 2f, bombardment overlay, urban variant, air-superiority 10-frame strip; naval 2f battle-result, 3f result families) but not which frame/variant/colour is chosen for a given combat instance / participant allegiance / counter mode. | needs CAP11 + recording A16 |
| Progress-bar fill + direction rotation | CAPTURE-GATED | Progress-bar geometry and the `rotation` slot are FILE; the fill fraction ↔ combat `progress`, and the per-combat `battle_indicator_direction` rotation angle (derived from participant provinces) are runtime. | needs CAP11 + recording A16 `progress` |

## Decoder Contract

`combat_indicator_assets_from_resolver(resolver)` returns:

- `land_combat_sprites`: resolved land bubble sprites (backing, frame,
  bombardment overlay, urban variant, direction indicator, animated battle
  strip, air-superiority modifier strip) with source line, texture path, DDS
  dimensions, format, `noOfFrames`, pixel frame rects and UV rects.
- `naval_combat_sprites`: resolved naval bubble sprites (combat icon, result
  variants, battle-result frames, HQ combat frame, fleet-status combat/result
  icons).
- `unresolved_sprites`: shipped combat sprite references whose texture path
  cannot be resolved through the active resolver (empty on clean vanilla).
- `progress_bars`: the naval attacker progress bar + the reused land unit
  combat progress bar (`ProgressBarAsset` with declared size + both textures).
- `gui_layouts`: the five on-map combat containers plus their named child
  controls (`CombatIndicatorGuiLayouts`).
- `fonts`: the `hoi_16mbs` binding used by the combat textboxes.

`CombatIndicatorAssetManifest::placement_stub(province_id, kind)` returns a
FILE-only `CombatIndicatorPlacement`: the declared container size is exposed;
`screen_quad_size` stays `None` and `anchor_law_known` stays `false` — those are
capture-gated (CAP11).

Frame law: identical to C1 — for each DDS-backed sprite, `noOfFrames` splits the
texture into equal atlas frames (horizontal unless `framedirectiony=yes`); tests
cross-check GFX `noOfFrames` against DDS pixel dimensions, so non-divisible
textures fail closed. N/N resolution gate: `combat_indicator_assets_from_resolver`
resolves 17 sprites (7 land + 10 naval) with 0 unresolved on vanilla; each
referenced `.dds` must exist or the read fails.

## Recording Dependencies (added to REPLAY_DATA_REQUIREMENTS / EXTRACTION_NEEDS)

The bubbles display combat state that the recording must carry:

- **Land** (REPLAY_DATA_REQUIREMENTS item 3): per active land combat `progress`
  (0..1), `bombardment_active` (bool), `air_superiority_state` (0..9 → modifier
  strip frame). Urban variant is province-terrain derived (static), not a
  recording field.
- **Naval** (REPLAY_DATA_REQUIREMENTS item 8): active naval combats
  `{naval_combat_id, location sea-province, attacker/defender task_force ids,
  result_type, progress}`.
- Tracked as EXTRACTION_NEEDS **A16**. Missing fields degrade to bubble-absent /
  default frame.

## CAP11 Capture Ask

Close-zoom, no-FoW war frame with an active **land battle bubble**
(`land_combat_mapicon`) on screen — ideally one land combat with a progress bar
mid-fill, plus if easy a shore-bombarded combat and an air-superiority modifier;
and one **naval combat bubble** at a sea zone. Batches with CAP6 (counters) —
same close-zoom war session.

Questions CAP11 must answer:

| Question | Why files are insufficient |
|---|---|
| Screen-space size / anchor | Container declares `35x34` (land) / `33x43` (naval); the rendered quad size, zoom scaling, and world/province anchor are not in files. |
| Frame / colour / variant selection | Files expose land frame (2f), bombardment overlay, urban variant, air-superiority strip (10f), naval battle-result (2f, port-strike vs naval-battle), and naval result families (3f) — but not which is chosen for a given combat/allegiance/counter-mode. |
| Progress-bar fill mapping | Progress-bar geometry is FILE; the fraction-to-fill mapping and which bar (naval 154-wide vs land 40-wide) is used at which zoom is runtime. |
| Direction-indicator rotation | `battle_indicator_direction` has a `rotation` slot but the per-combat angle (from participant provinces) is runtime. |
| Depth / overlap vs counters & terrain | Files give no draw priority for the bubble against unit counters or terrain. |
