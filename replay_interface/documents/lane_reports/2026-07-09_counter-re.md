# Lane Report: C1 Unit Counter Rendering - Game-Files-First Prep

## Summary

C1 stays tooling + docs only. No product renderer path now draws unit counters.

Implemented a new map_viewer decoder module:
`replay_interface/native/map_viewer/src/unit_counter_assets.rs`, exported from
`replay_interface/native/map_viewer/src/lib.rs`.

All shipped-file reads in the decoder go through `GameFileResolver`. Tests pin
the vanilla install only in test code and include a synthetic mod override
fixture proving a modded counter DDS/source wins over vanilla.

## Asset Inventory

| Asset class | Files decoded | Offline result |
|---|---|---|
| On-map counter layout | `<local-path> of Iron IV\interface\mapicons.gui` | Land `unit_counter` size `76x24`; navy `66x33`; railway gun `58x24`; zoomed-in y offset `20`; child controls for backing, ideology, symbol, text, bars, flag, order, supply, dig-in, training, action, active abilities, HQ/leader. |
| Counter backing/overlay sprites | `<local-path> of Iron IV\interface\mapicons.gfx`; DDS under `gfx/interface/` and `gfx/interface/mapicons/` | `GFX_onmap_unit_counter` is `198x27` with `noOfFrames=3` -> `66x27` frames. Ideology strip is `112x27` with 4 frames -> `28x27`. Unit type strip is `360x12` with 12 frames -> `30x12`. |
| Land division symbols | `<local-path> of Iron IV\interface\subuniticons.gfx`; DDS under `gfx/interface/counters/divisions_small/` | 126 `_medium_white` on-map definitions; 125 resolvable DDS files; 1 unresolved shipped reference; 121 unique names; 82 unique texture paths. Known infantry/armor/artillery symbols are `60x12`, `noOfFrames=2`, `30x12` frames. |
| Unresolved shipped ref | `interface/subuniticons.gfx:259` | `GFX_unit_jungle_pioneers_support_icon_medium_white` points at missing `gfx/interface/counters/divisions_small/onmap_unit_jungle_pioneers_support_icon_white.dds`; install contains `unit_jungle_pioneers_support_icon_white.dds`. Decoder reports unresolved; it does not guess-remap. |
| Ship symbols | `interface/subuniticons.gfx`; DDS under `gfx/interface/counters/ships_small/` | 12 white + 12 black on-map ship definitions; 22 unique ship texture paths. |
| NATO text icons | `interface/texticons.gfx`; DDS under `gfx/texticons/` | 28 `GFX_nato_unit_*` text icon definitions; 21 unique textures. Not proven to be on-map counter symbols. |
| Bars/badges | `interface/mapicons.gfx`, `interface/mapicons.gui`, `interface/unitview.gfx` | Org/strength bars are `3x17` sprites rotated by GUI; low-supply strip is `72x24`, 3 frames -> `24x24`; active ability grid is file-proven. |
| Fonts | `interface/core.gfx`, `interface/mapicons.gui`, `interface/unitview.gui` | On-map counter count uses `Main_14` -> `gfx/fonts/garamond_14`; side army badge text uses `hoi_18mbs`. |
| Template icon picker | `interface/divisiondesignerview.gui`, `common/ai_templates/*.txt` | Picker geometry is `84x50` outer, `76x42` inner; GUI comment says code indexes all possible symbols from `subuniticons.gfx`; AI templates prove `custom_icon = N` exists. Exact runtime index-to-sprite law remains capture/RE-gated. |
| Defines | `common/defines/00_graphics.lua`, `common/defines/00_defines.lua` | Map icon stack priorities, counter-mode allegiance colors, unit XP levels, dig-in constants, `LOW_SUPPLY = 0.99`, and supply attrition alert thresholds are file-derived. |

## Decoder API

Primary entrypoint:
`unit_counter_assets_from_resolver(&GameFileResolver) -> UnitCounterAssetManifest`.

Key public helpers:
- `on_map_division_counter_inventory_from_resolver`
- `on_map_division_counter_sprites_from_resolver`
- `on_map_ship_counter_sprites_from_resolver`
- `nato_text_icon_sprites_from_resolver`
- `map_counter_sprites_from_resolver`
- `unit_counter_gui_layouts_from_resolver`
- `division_template_counter_layout_from_resolver`
- `counter_fonts_from_resolver`

The manifest separates `on_map_division_symbols` from
`unresolved_on_map_division_symbols`, and every resolved sprite carries source
file/line, texture path, DDS dimensions, `noOfFrames`, pixel frame rects, and UV
rects. Frame validation cross-checks GFX frame math against DDS pixel
dimensions and fails closed on non-divisible frame strips.

## Law Doc

Added `replay_interface/documents/UNIT_COUNTER_LAW.md`.

FILE rows now cover:
- land/navy/railway counter GUI containers and child slots,
- backing/overlay/ideology/type-strip DDS frame geometry,
- land/ship/NATO inventories,
- text fonts,
- org/strength/supply/order/dig-in/training/action/active-ability slots,
- map icon priorities and shipped threshold/color constants.

CAPTURE-GATED rows remain:
- screen-space counter size and zoom scaling,
- province anchor and zoomed-in offset activation,
- stacking/aggregation by zoom,
- runtime template/custom-icon selection,
- counter frame/color/flag selection,
- badge visibility/frame priority,
- no-intel/fake-intel behavior under FoW.

Recording docs updated because `active_abilities` is an on-map counter slot:
`REPLAY_DATA_REQUIREMENTS.md` item 3 now asks for `active_ability_kinds`;
`EXTRACTION_NEEDS.md` has new A15 for division counter badge state.

## Tests

Command run from `replay_interface/native`:

`cargo test -p map_viewer`

Exact counts from the final run:
- lib suite: `614 passed; 0 failed; 1 ignored`
- integration tests: `13 passed; 0 failed`
- doc-tests: `0 passed; 0 failed`

The command completed green. Cargo emitted existing dead-code warnings in
other modules and `warn: could not canonicalize path <local-path>

## CAP6 Ask

Capture a close-zoom, no-FoW land frame with several visible counters:
infantry stack, armor stack, moving unit, selected/highlighted unit if easy,
low-supply or no-orders unit if easy, and active ability if easy.

CAP6 must answer:
- rendered counter pixel size versus GUI `76x24` and backing `66x27`,
- world/province anchor and zoomed-in offset activation,
- stack aggregation/count text/group marker law by zoom,
- actual template-role/custom-icon-to-symbol selection,
- backing/ideology/allegiance/flag/frame selection,
- badge visibility/frame/overlap priority,
- FoW no-intel/fake-intel behavior.

## Notes

`git diff --check` is clean aside from line-ending warnings. `git status` may
show `replay_interface/native/map_viewer/src/bin/dump_map_furniture.rs` as
modified in this linked worktree, but `git diff --` for that file is empty;
the index could not refresh because the external worktree index lock is not
writable in this sandbox.
