# Unit Counter Law

Status: CAP6 PARTIAL law landed (2026-07-10). The whole-frame corpus proves
constant 1 px/GUI-unit counter geometry over h in [50.0000076, 566.1656494],
two exact close-zoom anchor bands, frame-0 close-zoom backing/symbol selection,
and two observed stack slots separated by 24 px. `counter_render.rs` exposes
resolver-gated capture-law constructors and the former FIT stack emits nothing.
The captured-map renderer now has a receipt-gated GPU seam, while the product
still passes `UnknownFailClosed` and draws zero without exact target,
tint/modulation, and complete order receipts. Engine stack identity/order,
grouping above the FILE h=90 threshold, and unsampled anchor interpolation also
remain fail-closed.

All shipped-file reads in code must go through `GameFileResolver` keyed by the
recording modset. The C1 decoder lives in
`replay_interface/native/map_viewer/src/unit_counter_assets.rs` and exposes
resolved and unresolved asset records separately.

Retained CAP6 observations are frozen and mechanically checked by
`re_evidence/rendering_laws/CAP6/verify_cap6_retained.py` (offline checks;
no game launch or build). The exact remaining discriminating re-shoots are in
`re_evidence/rendering_laws/CAP6/human_input_requirements.json`.

## Source Inventory

| Row | Class | Derived fact | Source |
|---|---|---|---|
| Land badge sprite set | FILE | C1 resolves 9 compositor badge sprites: no-orders, area-defense, invalid-orders, low-supply, dig-in, training, action, attrition, and active-ability. The active-ability item is `20x20`, centered, with FILE icon scale `0.8`. | `interface/mapicons.gfx:15-29,68-72,442-459,686-690`; `interface/unitview.gfx:419-423`; `interface/mapicons.gui:189-198` |
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
| CAP6 content signature | CAPTURE + FILE | The counter block is identified by exact uploaded texture bytes, not stop order: resource 175827 is byte-identical BGRA to `gfx/interface/onmap_unit_counter.dds`; resources 175837/175839 match the shipped org/strength bars; resource 176424 matches `onmap_unit_infantry_icon.dds`; resources 467229/467231/467233/467239 match light-tank/cavalry/mountain/motorized strips. | `CAPTURE (mined/hoi4_frame69242, chunk 1343, resource 175827 SHA-256 db601c289334ddb3250e34a7a61457645ede027a7eff5eb256cf4aed155aed08)`; `CAPTURE (mined/hoi4_frame69242, chunk 1443, resource 176424 SHA-256 71c77e622b375d2ff02f61c86ca5f675de5dc488c325e5ef3c7da74271282d)`; FILE `interface/mapicons.gfx:79-88,326-334`, `interface/subuniticons.gfx:184-231` |
| Screen quad size / zoom hookup | CAPTURE + DERIVED | The immutable backing VB is 66x27 and symbol VB is 30x12; every sampled counter GUI CB from h=50.0000076 through 566.1656494 uses orthographic diagonal `(2/1920,2/1080)`, so rendered size is exactly natural FILE pixels and `pixels_per_gui_unit=1`. No counter-scale term or camera height enters the GUI VS. Covered interval: h in [50.0000076,566.1656494]; h in (566.1656494,3000] is UNKNOWN-fail-closed because h=3000 contains no identified counter draw. | `CAPTURE (mined/hoi4_frame69242, chunk 266, VB 94765 = vertices (0,0,1,0,1),(0,27,1,0,0),(66,0,1,1/3,1),(66,27,1,1/3,0))`; `CAPTURE (mined/hoi4_frame69242, chunk 31494, CB diag bytes 89 88 88 3a / d6 b9 f2 3a = 0.00104166672 / 0.00185185182)`; extrema confirmed by `CAPTURE (mined/hoi4_frame35240, chunk 28061, same diag)` and `CAPTURE (mined/hoi4_frame37391, chunk 10240, same diag)` |
| Close-zoom world-anchor to origin | CAPTURE + FILE + DERIVED | Project FILE `unitstacks.txt` type-0 `(x,y,z)` through scene VP, round x/y to the nearest pixel, then add `[-38,+39]` in the captured h=[50.0000076,50.1151161] band or `[-38,+28]` in h=[79.7969208,79.7970505]. `-38` is half the resolver-loaded FILE container width 76. The h=79 law matches 35 backing CB origins exactly, including province 3380: projected `(1473.438789,13.838991)` -> rounded `(1473,14)` -> origin `(1435,42)`. Heights between/above these anchor bands remain UNKNOWN-fail-closed; no interpolation is fitted. | FILE `map/unitstacks.txt:3366`; `CAPTURE (mined/hoi4_frame69242, chunk 4282, scene VP row bytes decode to [1.35876012,0,0,0;0,0.9481408,-0.920898139,-0.919746995;0,2.22171664,0.393002898,0.392511636;-4169.7832,-3472.2429,-532.3483,-526.6829])`; `CAPTURE (mined/hoi4_frame69242, chunk 31494, backing CB translation 0.494791687,0.872222185 -> origin 1435,42)`; h=50 confirmation `CAPTURE (mined/hoi4_frame35240, chunk 28061, decoded offset [-38,+39])` |
| Close-zoom backing selection / allegiance modulation | CAPTURE + FILE | All h<350 observed land backings use atlas frame 0 (`Offset.x=0`, `NextOffset.x=1/3`). Unit allegiance is not a backing-frame choice: the same resource/frame receives CB Color equal to FILE `COUNTER_MODE_ALLEGIANCE_OURS`, `ENEMY`, or `OTHER`; captured counts at h=79.797 are 17/19/11. The current CPU quad lacks a modulation field, so product tint remains UNKNOWN-fail-closed rather than baking grey. | `CAPTURE (mined/hoi4_frame69242, chunk 31494, offset bytes 00 00 00 00 / ab aa aa 3e and Color 0.32,0.71,0.39,1)`; `CAPTURE (mined/hoi4_frame69242, chunk 31247, Color 0.91,0.30,0.30,1)`; FILE `common/defines/00_graphics.lua:724-727` |
| Close-zoom division-symbol selection / modulation | CAPTURE | Every identified 60x12 division strip at h<=331.732 uses frame 0 (`Offset.x=0`, `NextOffset.x=1/2`); infantry, light tank, cavalry, mountain, and motorized strips are byte-identified. Captured symbol colors are ours `[0.7,0.9,0.7,1]`, enemy `[1,0.7,0.7,1]`, other `[0.78,0.78,0.78,1]`. At h>=389.789 the identified counter block emits no 60x12 symbol draws. | `CAPTURE (mined/hoi4_frame69242, chunk 31527, offset bytes 00 00 00 00 / 00 00 00 3f and Color 0.7,0.9,0.7,1)`; `CAPTURE (mined/postfx_frame8667, chunk 25653, h=331.7320862 frame-0 backing/symbol block)`; `CAPTURE (mined/postfx_frame8766, chunk 24918, h=389.7897949 backing frame 2 and no 60x12 symbol draw)` |
| Strategic grouped backing frame | FILE + CAPTURE | FILE grouping changes at h=90/180/350. The backing remains frame 0 at h=331.732 and changes to frame 2 (`Offset.x=2/3`) at h=389.789, 432.627, 470.095, and 566.166; this brackets and confirms the shipped strategic/coarse grouping threshold 350. Product aggregation remains fail-closed because the capture does not identify replay unit->group membership/order. | FILE `common/defines/00_graphics.lua:984-992`; `CAPTURE (mined/postfx_frame8667, chunk 25653, h=331.7320862 offset bytes 00 00 00 00)`; `CAPTURE (mined/postfx_frame8766, chunk 24918, h=389.7897949 offset bytes ab aa 2a 3f = 2/3)`; `CAPTURE (mined/hoi4_frame37391, chunk 10240, h=566.1656494 offset 2/3)` |
| Close-zoom two-slot stack separation | CAPTURE | Two backing draws at the same x are separated by exactly +24 px y, matching the FILE container height: h=79.797 origins `(452,818)` and `(452,842)`; independent h=50 and h=79 frames repeat the same delta. Only offsets `[0,0]` and `[0,24]` are promoted; slot index >=2 fails closed. Stable UID ordering is deterministic DERIVED plumbing, not an engine-order claim. | `CAPTURE (mined/hoi4_frame69242, chunk 33696, CB translation -0.529166698,-0.564814806 -> origin 452,818)`; `CAPTURE (mined/hoi4_frame69242, chunk 33815, CB translation -0.529166698,-0.609259248 -> origin 452,842)`; `CAPTURE (mined/hoi4_frame35240, chunks 29202/29303, same +24 delta)`; FILE `interface/mapicons.gui:17-21` |

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

## CPU Composition Contract

`counter_render_assets_from_resolver(resolver)` reuses the decoder through the
active `GameFileResolver` and returns the land frame, bar sprites, land/NATO
atlas, 9 badge sprites, unresolved symbol keys, and land GUI layout.

`build_counter_instances` groups visible land inputs by province, sorts by
stable unit id, and emits frame/NATO/org/strength/badge quads in painter order.
Every quad carries texture source provenance, atlas frame, four screen corners,
and four UVs. Bar fractions outside `[0,1]`, absent assets/data, unsupported
naval/air domains, and missing runtime selections omit that layer explicitly.

`CounterScreenLaw::from_capture_close_zoom(assets,height)` now returns a
CAPTURE law only for the repeated h=50 and h=79.797 bands and only when the
active resolver supplies the captured vanilla 76x24 layout, `{0,20}` zoomed-in
offset, and 66x27 backing frame. It rounds the projected world anchor, applies
the band-specific offset, uses one pixel per GUI unit, and supplies the observed
frame-0 backing/symbol selection. Every unsampled height or mod-layout mismatch
returns `UnknownFailClosed`.

`CounterStackingLaw::from_capture_close_zoom` supplies exactly `[0,0]` and
`[0,24]`; a third slot is unavailable. The legacy
`FitVerticalByDeclaredContainerHeight` spelling remains for source compatibility
but now returns no offset and emits no instance. This removes the FIT from the
render path without pretending the capture recovered engine stack identity or
group membership.

### Product wiring note (w10-counter-wire, 2026-07-10)

`replay_app::render` now uses the two public capture constructors from the same
active production-camera f32 height and reprojects/rebuilds counter geometry
whenever that camera changes. This is **CAPTURE + FILE + DERIVED geometry-only
wiring**: `CAPTURE (mined/hoi4_frame69242, chunk 31494, camera h=79.7970505)`
drives the in-band headless fixture to deterministic `2/2/6`
inputs/instances/quads, while the unsampled h=60 case is
**UNKNOWN-fail-closed** at `2/0/0`. `dump_frame` prints those same three stats.
The covered bands remain h=[50.0000076,50.1151161] and
h=[79.7969208,79.7970505], supported by
`CAPTURE (mined/hoi4_frame35240, chunk 28061, decoded [-38,+39])`,
`CAPTURE (mined/hoi4_frame61841, chunk 14602, decoded [-38,+39])`,
`CAPTURE (mined/hoi4_frame76017, chunk 24055, decoded [-38,+28])`, and
`CAPTURE (mined/hoi4_frame69242, chunk 31494, decoded [-38,+28])`.

This lane makes **no GPU or pixel-parity claim**. At mandated HEAD the only
counter handoff, `map_viewer::update_prepared_production_map_counter_batch`,
forwards to a pass-graph update that unconditionally creates a `Blocked`,
`draw_enabled=false` CPU summary; no counter texture upload/draw API exists in
the permitted replay-app files. The owning `map_viewer`/`pass_graph.rs` surfaces
were explicitly forbidden, so visible counters remain **UNKNOWN-fail-closed**
pending that API plus tint/modulation storage. The W9 stack/group/interpolation
re-shoot and A15 org/badge asks remain unchanged.

### Adversarial correction (w10-counter-wire fix round, 2026-07-10)

This section **supersedes the preceding W10 product-wiring note**. Review found
that its `2/2/6` gate was unsound: it used a 1408x512 target despite the law
being captured only at 1920x1080, combined an identity VP with one copied
camera-height component, promoted a synthesized `template_role` NATO key, and
never put two divisions in one province. It also allowed fail-closed outputs to
retain projected anchors and left the narrower replay-app truth invisible next
to the stale owning pass-graph blocker.

The corrected replay-app policy is:

- **CAPTURE:** target dimensions must be exactly 1920x1080 before projection.
  Other dimensions receive an explicit `CapturedViewportUnavailable` omission,
  zero instances/quads, and `screen_anchor_px=None` for every input.
- **CAPTURE:** the same active f32 camera height selects both public laws before
  projection. An unavailable law receives `CapturedScreenLawUnavailable`, zero
  instances/quads, and no projected anchors.
- **UNKNOWN-fail-closed:** every province/domain group with more than one
  visible unit is wholly omitted. Each unit receives map-viewer's
  `StackOffsetUnknown` plus replay-app's explicit
  `MultiUnitStackSlotIdentityUnknown`; stable UID order selects no slot and no
  ordering-derived placement carries CAPTURE honesty. Only a one-element group
  may consume captured slot 0.
- **UNKNOWN-fail-closed:** product inputs clear the synthesized NATO key and
  frame. Each eligible singleton receives `NatoSymbolSelectionUnknown`; no NATO
  quad is emitted until the FILE `template_role -> map_icon_category` mapping
  is threaded.
- **DERIVED diagnostics:** captured-pass stats now print the exact narrow
  provenance and the current GPU/NATO/stack/viewport blockers. They also name
  the residual that forbidden owning `pass_graph.rs` still says all CAP6 screen
  geometry is unknown/zero and remains draw-disabled.

The non-vacuous headless gate uses the full VP and camera position from
`CAPTURE (mined/hoi4_frame69242, chunk 4282, raw first 64 VP bytes; camera
position [3068.8146973,79.7970505,1528.8105469])`, target 1920x1080, and FILE
`map/unitstacks.txt:3366` point `(3105.00,9.80,1599.00)`. Two singleton fixture
provinces produce deterministic `2/2/4` inputs/instances/quads (frame backing +
measured strength bar only); GER produces `1/1/2`. Forcing both divisions into
one province produces `2/0/0`; h=60 and 1408x512 each produce `2/0/0` with all
anchors absent. These are geometry-only assertions, not pixel parity.

### CAP6 GPU handoff update (2026-07-19)

This section supersedes the earlier statement that no counter GPU upload/draw
API exists. The captured production-map renderer now owns a narrow D3D11 pass
that consumes `CounterRenderBatch` only when paired with a complete
event-scoped `CounterGpuFrameAdmission`. It executes after the RestoreScene
composite because the retained `hoi4_frame69242.rdc` joins RestoreScene chunk
29204 and counter backing chunk 31499 to the same 1920x1080 resource 64, in
that order. The admitted state is the captured four-vertex triangle strip,
linear/linear/point sampler with clamp/clamp/wrap and -0.8 mip bias,
SrcAlpha/InvSrcAlpha blending, disabled depth, back-face culling, and full-target
viewport/scissor. Textures resolve only through the active admission-bound
effective VFS.

This is a **fail-closed engineering seam, not a pixel-parity promotion**:

- the current replay-app target is `B8G8R8A8_UNORM`, while the retained
  RestoreScene/counter target is `R8G8B8A8_UNORM`; the mismatch yields zero
  draws and does not allocate the CAP6 D3D runtime;
- every admitted CPU quad must have exactly one ordered receipt carrying the
  matching texture/frame, exact decoded RGBA dimensions/SHA-256, and non-FIT,
  non-DERIVED runtime modulation; missing, undecodable, duplicate, partial, or
  mismatched receipts reject the entire batch as a feature-local zero draw;
- the receipt must name the active captured-session directory and is consumed
  after one render attempt, preventing stale screen-space geometry from being
  reused across camera/hour transitions;
- no production caller currently supplies the required ordered frame receipt,
  so the default state is zero-draw;
- the authenticated portable map/UI operation route is intentionally unchanged.
  No portable counter carrier, role table, or draw order was invented from the
  semantic batch.

The remaining engineering/evidence boundary is therefore an authenticated,
portable, pointer-free carrier for exact counter content, runtime modulation,
complete draw order, and a target-format/order receipt compatible with the
product render target. `gpu_draw_pipeline.json` freezes the retained event
pipeline and records the present zero-draw admission mismatch.

## CAP6 Residual / Re-shoot Ask

The 2026-07-10 corpus closes natural size, close-zoom frame index, two sampled
anchor bands, and one two-slot offset. It does not close continuous anchor
interpolation, runtime modulation in the product quad, or grouping membership.
Re-shoot the same labelled stack without panning at h=50,60,70,79.797,85,89,90,
100,130,179,180,349,350,351,390 and 566; retain unit/template ids, province,
selection state and exact CPU world position for every counter. Include one
province with 1,2,3,4 and >12 selected divisions, one moving division, and ours/
allied/enemy/other counters. This distinguishes interpolation from threshold
steps and maps stack slot/order/group membership. Capture the GUI CB Color and
all sprite VBs at every stop; keep the same 1920x1080 DPI/window mode.

Questions CAP6 must answer:

| Question | Why files are insufficient |
|---|---|
| Screen-space size law | CLOSED for the captured interval h in [50.0000076,566.1656494] at 1920x1080: natural FILE pixels, scale 1. DPI/window-mode variation remains unshot. |
| Province anchor law | PARTIAL: rounding and offsets close only the h=50 and h=79.797 capture bands. Unsampled interpolation and moving-unit exact world positions remain UNKNOWN-fail-closed. |
| Stack aggregation law | PARTIAL: two close-zoom slots are `[0,0]` and `[0,24]`; slot identity/order, slot >=2, selected-split behavior, and FILE h=90/180/350 group membership remain UNKNOWN-fail-closed. |
| Runtime icon selection | PARTIAL: every captured close/mid symbol uses frame 0; texture key still comes from template role. Player-counter toggle/custom-icon and grouped no-symbol behavior need a labelled state join. |
| Counter frame/color selection | PARTIAL: close/mid backing frame 0 and strategic frame 2 are proven; allegiance is CB modulation, not a frame choice. CPU/product tint storage and allied-state validation remain open. |
| Badge visibility and frame selection | Files expose low-supply, orders, dig-in, training, action, attrition, and active-ability slots; recording state and capture must validate thresholds, frame indices, and overlap priority. |
| FoW/no-intel behavior | Files include no-intel/fake-intel symbol assets, but the visibility/intel level that triggers them belongs to runtime FoW state. |

Recording update from C1: add `active_ability_kinds` to the per-division
counter-badge state. `REPLAY_DATA_REQUIREMENTS.md` item 3 and
`EXTRACTION_NEEDS.md` A15 track this; missing values degrade to no
active-ability badge.
