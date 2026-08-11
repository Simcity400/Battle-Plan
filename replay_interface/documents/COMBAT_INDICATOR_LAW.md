# Combat Indicator Law (battle bubbles)

Status: exact-a729 engine, game-file, and producer-boundary law closed. The
state-first consumer is the sole product route and fails closed for recordings
which do not contain the minimum exact carrier.

There is no second D21-only battle-bubble compositor. D21 is the authoritative
active-land census and one side of a mandatory same-hour equality join; it is
not by itself enough to derive pixels. The product adapter admits a combat only
when the complete exact XMAP14 carrier agrees with D21 on ordered
`(combat_id,generation)`, location and division-side membership. Missing,
partial or disagreeing input makes `CombatMarkers` unavailable. Windowed and
headless XMAP22 routes then refuse a complete presentation instead of drawing a
default bubble or reporting semantic rows as if they were pixels.

The selected-Present/M02 path is not replay-semantic authority. Exact controller
and placement evidence is in
`re_evidence/rendering_laws/XMAP14/PRODUCTION_EVIDENCE.md`. The smallest
non-derivable producer boundary is
`re_evidence/rendering_laws/XMAP14/PRODUCER_DATA_CONTRACT.md`.

The decoder remains the effective-VFS asset authority. FILE container geometry
is combined only with the exact `CMapIcon` projection and combat-controller
laws; it is never reinterpreted as recorder-space geometry.

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
| Combat text fonts | FILE | `combat_progress` and fleet-status counts use `hoi_16mbs`; its Japanese override selects `gfx/fonts/japanese/hoi_16mbs`. `CTinyUnitCounter.count` uses effective `Main_14`. Playback resolves the effective control font and matching language override, retaining the base color. | `interface/mapicons.gui:1199,1846-1888`, `interface/core.gfx` |
| Battle-bubble screen-space law | ENGINE + FILE | `CMapIcon` projects with the XMAP03 matrix, applies alignment `0x12` (half live GUI width/height), signed half rounding, content distance cutoff, then aligned-rectangle viewport intersection. Land midpoint and naval/result anchors come from the minimum producer carrier. | `re_evidence/rendering_laws/XMAP14/PRODUCTION_EVIDENCE.md` |
| Combat → frame/colour/variant selection | ENGINE + FILE + PRODUCER | Exact fixed5 progress, side comparison scalars/flags, naval seven-counter vectors and retained result-member primitives are producer inputs. Replay derives POV flip, strict 110% tint, air/bombardment/direction frames, naval active frame and result frame. | `re_evidence/rendering_laws/XMAP14/PRODUCER_DATA_CONTRACT.md` |
| Progress-bar fill + direction rotation | ENGINE + FILE | `progress.shader` selects texture one at `uv.x <= CurrentState` and texture two otherwise. Land direction is `atan2f(delta.x,-delta.z)` around the FILE-centred direction control. | `gfx/FX/progress.shader`; exact-a729 evidence above |

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
- `fonts`: every effective bitmapfont referenced by the combat and
  `CTinyUnitCounter` controls, including language overrides.

The legacy `placement_stub` remains a file-inventory helper only. Production
does not consume it: XMAP14's adapter derives projection, alignment, viewport
intersection and ordered child draws from the exact carrier, XMAP03 camera and
this admitted manifest.

The active-naval large variant executes both authored grid surfaces per side.
`attacker_grid`/`defender_grid` reuse the exact XMAP13 tier-zero
`CUnitCounterItem` group presentation once per complete engine counter-item,
partitioned first by the live viewer allegiance and then by item ordinal. The
viewer relation is derived from the same-hour diplomatic pair exactly as in
XMAP13: owned is `Ours`, a measured war pair is `Enemy`, and every other
measured pair is `Other`; world observers retain `WorldObserver`. First-seen
allegiance-group order and each authenticated member ordinal are preserved.
`attacker_adds_grid`/`defender_adds_grid` use the separate FILE
`GFX_mapicon_tiny_unit_counter` template: tinted background, authored transport
icon and centered effective-font task-force count. Its spotting container is
constructor-hidden and the combat helper does not activate it. Empty convoy
sides remain measured-empty and issue no grid draw.

The shipping compositor treats those child draws as one authenticated stage:
`("xmap14.combat_indicators.execute", "combat.indicators.draw")`. Its ordinal
comes only from the shared assigned-stage schedule. The prepared frame also
authenticates the playback-language `hoi_16mbs` binding, the effective
`CTinyUnitCounter` count font, their `.fnt` descriptors and resolved atlas
pages; extensionless `fontfiles` entries in `core.gfx` are names, not files.
Imported XMAP13 naval-counter text carries that same selected language, checks
the selected descriptor/page closure against the prepared frame, and receipts
every atlas page that actually issues a draw.
`COMBAT_ICON_PROGRESS` comes from the
playback language's admitted effective localisation namespace. XMAP14 applies
the exact shipped `$VAL|0$` substitution to the viewer-relative fixed5
percentage and fails closed on a different, unproved substitution program.

Frame law: identical to C1 — for each DDS-backed sprite, `noOfFrames` splits the
texture into equal atlas frames (horizontal unless `framedirectiony=yes`); tests
cross-check GFX `noOfFrames` against DDS pixel dimensions, so non-divisible
textures fail closed. N/N resolution gate: `combat_indicator_assets_from_resolver`
resolves 21 sprites (7 land + 10 combat-naval + 4 `CTinyUnitCounter`) with 0
unresolved on vanilla; each
referenced `.dds` must exist or the read fails.

Progress controls resolve their effective GFX aliases and normalize the
Clausewitz `effectFile=gfx/FX/progress.lua` spelling to its admitted sibling
`progress.shader`. Replay requires the complete normalized program to be
token-equivalent to the engine/game-file authority, including both pixel
programs, vertex structs/program, constant buffer, samplers, blend state and
effects. Only comments, whitespace and ASCII case may vary; any added, removed
or changed live token fails closed. The admitted shader hash is bound into every
prepared progress draw and rechecked before native execution. Effective
`centerPosition` controls whether the authored progress anchor denotes its
centre or top-left; land `combat_progress` must retain its supported FILE
`format=center` alignment or admission fails closed.

## Recording dependencies

The replay-side join is:

- complete D21 active-land collection with measured `combat_id`, top-level
  `generation` independent of optional D55 detail,
  location and ordered attacker/defender division identities;
- complete D50 naval combat/member collection, including its exact side-country
  pair and authoritative measured-empty convoy-side task-force vectors;
- portable `(combat_id,generation)` identity;
- location and exact participant sides;
- effective-VFS FILE assets;
- the minimum XMAP14 carrier for exact progress, side comparison/flags, land
  anchor inputs, naval seven-counter vectors, and retained naval-result
  lifecycle/member primitives.

The exact land pixel leaves that must accompany D21 are: the province combat
base point; the first live attacker stack point selected in engine order;
virtual-`+200` fixed5 progress; both virtual-`+0x68` comparison scalars; both
side flag words plus their digest; `combat+0x88` frame selector; active-phase
witness; producer sequence/evidence identity; and the complete census/order
digests. Replay derives midpoint, rotation, viewer side, tint, bombardment,
air frame, GUI geometry, content resources, viewport culling and hit bounds.
None of those derived presentation values is accepted from the recorder.

The current `CombatRow` exposes the D21 identity/census subset, including its
top-level generation leaf. A malformed or partial row
poisons the complete frame; measured-empty is authoritative. Absent exact
producer fields produce zero combat draws for that hour and a typed unavailable
family disposition. Duration, division strength, province centroids, recorder
pixels, and prior-hour state are never substituted.

`replay_app` has no independent semantic-only fallback. The sole shipping
execution is `Xmap14Adapter -> PreparedXmap14Frame -> CombatGpuPass`; XMAP22
requires and hashes its sealed native receipt before the final target can be
advertised as complete.

Historical M02/selected-Present artifacts remain retained evidence of what one
recorder rendered. They are not portable observer gamestate and are not
promotion authority for the replay-owned compositor.
