# Unit Counter Law

Status: C1 files-first prep only. This document records what the replay can
derive offline from shipped the game files and what remains capture-gated for
CAP6. No product renderer should draw unit counters from this draft yet.

All shipped-file reads in code must go through `GameFileResolver` keyed by the
recording modset. The C1 decoder lives in
`replay_interface/native/map_viewer/src/unit_counter_assets.rs` and exposes
resolved and unresolved asset records separately.

## Source Inventory

| Row | Class | Derived fact | Source |
|---|---|---|---|
| On-map land counter layout | FILE | Land counter container is `unit_counter`, position `{0,0}`, declared size `76x24`. It contains backing, ideology overlay, type icon, count text, org/strength bars, flag stripe, order badges, low supply, dig-in, training, action, HQ and leader-enroute controls. | `<local-path> of Iron IV\interface\mapicons.gui:17-187` |
| Zoom-in offset | FILE | `unit_counter_zoomed_in_offset` is `{x=0 y=20}`; comment says only y is used. | `<local-path> of Iron IV\interface\mapicons.gui:10-14` |
| Navy and railway-gun layouts | FILE | Navy counter declared size is `66x33`; railway gun counter declared size is `58x24`; both reuse counter/backing pieces with different child positions. | `<local-path> of Iron IV\interface\mapicons.gui:201-345`, `<local-path> of Iron IV\interface\mapicons.gui:388-485` |
| Counter backing sprites | FILE | `GFX_onmap_unit_counter` and `GFX_onmap_unit_counter_empty` use `gfx/interface/onmap_unit_counter*.dds` with `noOfFrames = 3`; DDS dimensions are `198x27`, so frame rects are `66x27`. | `<local-path> of Iron IV\interface\mapicons.gfx:80-88` |
| Overlay/selection sprites | FILE | Overlay, selected, half-selected, active ability, naval overlay, mission/ship counter, and mission type strip are named spriteTypes. | `<local-path> of Iron IV\interface\mapicons.gfx:38-121` |
| Ideology/tint sprite | FILE | `GFX_onmap_unit_counter_ideology` uses `gfx/interface/onmap_unit_counter_ideology.dds`, `noOfFrames = 4`; DDS is `112x27`, so frame rects are `28x27`. | `<local-path> of Iron IV\interface\mapicons.gfx:283-288` |
| Runtime country flag stripe | FILE / CAPTURE-GATED | The land counter has `flag` and `large_flag` controls, but the spriteType comments say the texture file is set in code. The control positions are file-derived; the runtime texture source and frame choice are capture/RE-gated. | `<local-path> of Iron IV\interface\mapicons.gui:96-110`, `<local-path> of Iron IV\interface\mapicons.gfx:297-315` |
| Org/strength bar sprites | FILE | On-map org and strength bars use `mapicon_bar_org_sprite.dds` and `mapicon_bar_str_sprite.dds`; each DDS is `3x17`; GUI rotates each by `1.5708`. | `<local-path> of Iron IV\interface\mapicons.gfx:326-334`, `<local-path> of Iron IV\interface\mapicons.gui:81-94` |
| Order/supply badge assets | FILE | `no_orders`, `area_defense`, `invalid_orders`, and `low_supply` controls are in the land layout. Low supply uses `GFX_unitcounter_supply_status_strip`; that sprite is a 3-frame `72x24` DDS, so frame rects are `24x24`. | `<local-path> of Iron IV\interface\mapicons.gui:118-142`, `<local-path> of Iron IV\interface\mapicons.gfx:442-459`, `<local-path> of Iron IV\interface\unitview.gfx:420-422` |
| Dig-in/training/action badges | FILE | Land counter has `onmap_digin_icon`, `onmap_training_icon`, and `onmap_action_icon` slots at the right edge. The slot positions are file-derived; runtime visibility is recording-gated. | `<local-path> of Iron IV\interface\mapicons.gui:144-159` |
| Active ability grid | FILE + RECORDING-GATED | `active_abilities` is a grid at `{x=55 y=-9}`, slot `25x25`; each item uses `GFX_onmap_unit_counter_active_ability`. The layout and fallback icon are file-derived; active ability kinds must come from recording state. | `<local-path> of Iron IV\interface\mapicons.gui:57-63`, `<local-path> of Iron IV\interface\mapicons.gui:189-198`, `<local-path> of Iron IV\interface\mapicons.gfx:68-72` |
| Count text font | FILE | Land counter count text uses `Main_14`, positioned `{x=36 y=6}` with `20x14` max bounds. `Main_14` maps to `gfx/fonts/garamond_14`. | `<local-path> of Iron IV\interface\mapicons.gui:72-80`, `<local-path> of Iron IV\interface\core.gfx:871-875` |
| Army badge text font | FILE | Side-panel army badge count fields use `hoi_18mbs`; the font files are `gfx/fonts/hoi_18mbs` and `gfx/fonts/hoi_18mbs_cryllic`. | `<local-path> of Iron IV\interface\unitview.gui:350-357`, `<local-path> of Iron IV\interface\unitview.gui:429-434`, `<local-path> of Iron IV\interface\core.gfx:1174-1177` |
| Land division symbol inventory | FILE | `subuniticons.gfx` marks "Small images for DIVISIONS (On-map-counters)" and defines `_medium_white` spriteTypes under `gfx/interface/counters/divisions_small/`. C1 inventory: 126 definitions, 121 unique names, 82 unique texture paths, 125 resolvable DDS files, 1 unresolved shipped reference. | `<local-path> of Iron IV\interface\subuniticons.gfx:182-231` |
| Known land symbol frame law | FILE | Infantry, medium armor, light armor, and artillery use `60x12` DDS strips with `noOfFrames = 2`; frame 0 is `[0,0,30,12]`, frame 1 is `[30,0,30,12]`. | `<local-path> of Iron IV\interface\subuniticons.gfx:199-208` |
| Unresolved shipped symbol | FILE | `GFX_unit_jungle_pioneers_support_icon_medium_white` references `gfx/interface/counters/divisions_small/onmap_unit_jungle_pioneers_support_icon_white.dds`, but the install contains `unit_jungle_pioneers_support_icon_white.dds` instead. The decoder reports this as unresolved, not silently remapped. | `<local-path> of Iron IV\interface\subuniticons.gfx:259` |
| Ship counter symbols | FILE | Ship on-map counters define 12 white and 12 black `_medium_*` spriteTypes under `gfx/interface/counters/ships_small/`, with 22 unique texture paths. | `<local-path> of Iron IV\interface\subuniticons.gfx:529-556` |
| NATO text icons | FILE | `texticons.gfx` defines `GFX_nato_unit_*_icon_small` text icons. C1 inventory: 28 definitions, 21 unique NATO text icon textures. These are text/icon assets, not proven to be the map counter symbols. | `<local-path> of Iron IV\interface\texticons.gfx:514-641`, `<local-path> of Iron IV\interface\texticons.gfx:1037-1123` |
| Template icon picker | FILE / CAPTURE-GATED | Division designer GUI says the custom icon button is code-overridden to index all possible symbols from `subuniticons.gfx`. The picker button is `84x50` with inner `76x42`, but runtime custom-icon index ordering and template-role mapping must be validated. | `<local-path> of Iron IV\interface\divisiondesignerview.gui:1716-1732` |
| AI template icon indices | FILE | Shipped AI templates use `custom_icon = N`, proving the runtime stores icon indices in some template contexts. Exact index-to-sprite ordering remains code/capture-gated. | `<local-path> of Iron IV\common\ai_templates\generic.txt:297`, `<local-path> of Iron IV\common\ai_templates\templates_JAP.txt:530` |
| Map icon priority constants | FILE | `NMapIcons` defines unit stack and unit stack group draw priorities for each map mode; default priorities are 10 and 11. | `<local-path> of Iron IV\common\defines\00_graphics.lua:134-145` |
| Counter-mode colors | FILE / CAPTURE-GATED | `COUNTER_MODE_ALLEGIANCE_*` colors are shipped constants. Which counter mode/frame/color path is active in replay context is capture-gated. | `<local-path> of Iron IV\common\defines\00_graphics.lua:724-727` |
| Gameplay thresholds for badges | FILE / RECORDING-GATED | Defines expose unit experience level thresholds, dig-in cap/speed, `LOW_SUPPLY = 0.99`, and supply attrition alert thresholds. They do not provide the full badge visibility law without per-division runtime state. | `<local-path> of Iron IV\common\defines\00_defines.lua:960-979`, `<local-path> of Iron IV\common\defines\00_defines.lua:1012`, `<local-path> of Iron IV\common\defines\00_defines.lua:4412-4418` |

## Decoder Contract

`unit_counter_assets_from_resolver(resolver)` returns:

- `on_map_division_symbols`: resolved land symbol DDS assets with source line,
  texture path, DDS dimensions, `noOfFrames`, pixel frame rects, and UV rects.
- `unresolved_on_map_division_symbols`: shipped sprite definitions whose
  texture path cannot be resolved through the active resolver.
- `on_map_ship_symbols` and `nato_text_icons`: resolved ship and NATO text icon
  inventories.
- `map_counter_sprites`: backing, ideology, overlay, bar, badge, and type-strip
  sprites from `mapicons.gfx`.
- `map_counter_code_textured_sprites`: flag stripe spriteTypes whose texture is
  explicitly code-set by the engine.
- `unitview_sprites` and `army_badge_progress_bars`: supply strip, unit level,
  army badge medians, and side-panel progress bars.
- `gui_layouts`: land, navy, railway-gun, and air-wing counter containers plus
  named child controls.
- `division_template_layout`: custom icon picker geometry.
- `fonts`: `Main_14` and `hoi_18mbs` bindings from `core.gfx`.

Frame law: for each DDS-backed sprite, `noOfFrames` splits the texture into
equal atlas frames. The default split is horizontal; `framedirectiony = yes`
switches to vertical. Tests cross-check GFX `noOfFrames` against DDS pixel
dimensions, so non-divisible textures fail closed.

## CAP6 Capture Ask

CAP6 should capture a close-zoom, no-FoW land frame with several counters in
view, including at least: one infantry division stack, one armor stack, one
moving unit, one selected or highlighted unit if possible, one unit with low
supply or no orders if easy, and one active ability if easy.

Questions CAP6 must answer:

| Question | Why files are insufficient |
|---|---|
| Screen-space size law | The land GUI container says `76x24`, while `GFX_onmap_unit_counter` frame geometry is `66x27`. The exact rendered quad size, DPI scaling, and zoom scaling must be measured. |
| Province anchor law | Files give child offsets and a zoomed-in y offset, but not the world/province anchor, depth ordering against terrain, or hq/zoom offset activation. |
| Stack aggregation law | Files expose `count_txt`, `group_marker`, and unit-stack priorities, but not when multiple divisions merge into one counter or how this changes by zoom. |
| Runtime icon selection | Files expose all symbol sprites and `custom_icon` indices, but not the engine's exact template-role/custom-icon/index selection for an observed division. |
| Counter frame/color selection | Files expose backing frames, ideology frames, allegiance colors, flags, and white/black ship variants, but not which frame/color path is chosen for owner, player, ally, enemy, selection, or counter mode. |
| Badge visibility and frame selection | Files expose low-supply, orders, dig-in, training, action, attrition, and active-ability slots; recording state and capture must validate thresholds, frame indices, and overlap priority. |
| FoW/no-intel behavior | Files include no-intel/fake-intel symbol assets, but the visibility/intel level that triggers them belongs to runtime FoW state. |

Recording update from C1: add `active_ability_kinds` to the per-division
counter-badge state. `REPLAY_DATA_REQUIREMENTS.md` item 3 and
`EXTRACTION_NEEDS.md` A15 track this; missing values degrade to no
active-ability badge.
