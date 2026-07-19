# Lane C3 — Battle bubbles / combat indicators (files-first)

Branch `lane/c3-bubbles`. Plan item C3 (REPLAY_PRODUCTION_PLAN.md Phase C).
Same treatment C1 gave unit counters: decode the on-map combat-indicator
asset/layout chain from shipped files with N/N resolution gates; the
screen-space visual law stays capture-gated.

## What shipped

- `replay_interface/native/map_viewer/src/combat_indicator_assets.rs` — new
  module decoding the land + naval on-map combat bubbles. **Reuses** the C1
  parser/DDS/GUI helpers (promoted 4 fns in `unit_counter_assets.rs` to
  `pub(crate)`: `sprites_from_resolver`, `sprite_inventory_from_resolver`,
  `container_layout_from_text`, `parse_progress_bar_assets`, `parse_bitmap_fonts`)
  — no forked parser.
- `replay_interface/documents/COMBAT_INDICATOR_LAW.md` — FILE vs CAPTURE-GATED
  classification + source inventory + CAP11 ask (parallel to UNIT_COUNTER_LAW.md).
- Requirements additions (REPLAY_DATA_REQUIREMENTS item 3 + item 8; EXTRACTION_NEEDS A16).
- FIDELITY_LEDGER.md combat-indicator rows (summary + §2.18 detail; totals 78→81 FILE / 24→27 CAPTURE).
- Plan §4 CAP11 row + C3 status row updated to FILES-FIRST DONE.
- `map_viewer/src/lib.rs` — module declared + public API re-exported.

## Asset chain + resolution gates (all vs vanilla install, test-gated)

| Group | Sprites | Source | Gate |
|---|---|---|---|
| Land combat (mapicons.gfx) | mapicon_land_combat 38x37·1f; _frame 76x37·2f→38x37; _bombardment_overlay 38x37; _urban 38x37; battle_indicator_direction 70x70·7mip; landbattle_strip 1020x44·20f→51x44 (10fps) | `mapicons.gfx` | 6/6 resolve, frame math verified |
| Land combat (landcombat.gfx) | battle_modifier_strip 250x25·10f→25x25 (air-superiority) | `landcombat.gfx` | 1/1 |
| Naval combat (mapicons.gfx) | mapicon_naval_combat 80x33·2f→40x33; naval_combat_result 105x33·3f→35x33; _portstrikes/_navalstrikes 102x34·3f→34x34; _convoys 102x33·3f→34x33; mapicon_naval_battle_result 66x33·2f→33x33; _text 17x17 DXT5; naval_combat_icon/results_icon 25x25 | `mapicons.gfx` | 9/9 |
| Naval combat (naval_leader_module.gfx) | naval_hq_combat_frame 53x43 | `building_roster/naval_leader_module.gfx` | 1/1 |
| Progress bars | navalcombat_bar_progress_attacker_154 (154x14, 157x14 tex); land_unit_combat_progressbar (40x5) | `navalcombat.gfx`, `unitview.gfx` | 2/2 |
| GUI containers | land_combat_mapicon (35x34); naval_combat_mapicon (33x43); naval_combat_results_mapicon (19x24); ongoing_combat_container; combat_results_container | `mapicons.gui` | 5 containers + child controls parsed |
| Font | hoi_16mbs → gfx/fonts/hoi_16mbs | `core.gfx` | resolved |

**Totals: 17 sprites resolved N/N (0 unresolved), 12 unique textures, 2 progress
bars, 5 GUI containers, 1 font.** 6 tests pass (5 vanilla-install-gated + 1
mod-resolver-override); full `map_viewer` suite: 645 lib + all integration
green, 0 failed. `cargo build -p map_viewer` clean.

## Requirements additions (combat state the bubbles display)

Item 3 (land) had the combat skeleton (`combat_id`, `location_province`,
participant ids) but not the bubble display fields. Added:
- **REPLAY_DATA_REQUIREMENTS item 3 (land):** `progress` (formalized),
  `bombardment_active`, `air_superiority_state`. Urban variant = province-terrain
  derived (static), noted as NOT a recording field.
- **REPLAY_DATA_REQUIREMENTS item 8 (naval):** active naval combats
  `{naval_combat_id, location sea-province, attacker/defender task_force ids,
  result_type, progress}` — no prior coverage (item 8 was fleet presence only).
- **EXTRACTION_NEEDS A16** — one row covering both, degrade-to-absent.

## Capture ask

**CAP11** (plan §4): close-zoom no-FoW war frame with an active land battle
bubble (progress bar mid-fill; shore-bombarded + air-superiority modifier if
easy) + one naval combat bubble. Batches with CAP6 (counters) — same session.
Answers: screen-space size/anchor/zoom-scale, combat→frame/colour/variant
selection, progress-bar fill mapping, direction-indicator rotation, depth order.

## Ledger rows added

- Summary: `Combat indicators (battle bubbles, C3) | 3 FILE | 3 CAPTURE | 0 | 0 | COMBAT_INDICATOR_LAW.md`.
- §2.18 detail: 3 FILE rows (sprite chain / layout tree / progress-bars+font),
  3 CAPTURE rows (screen-space law / frame-selection / fill+rotation).
- Totals updated 78/24/38/46 → 81/27/38/46.

## Scope honesty

FILE = asset/layout chain, frame splits, declared sizes, font binding — all
byte-verified against the vanilla install. CAPTURE = every screen-space render
parameter (no eyeballing, no fits). No product renderer draws bubbles yet; this
is the FILE half of C3.
