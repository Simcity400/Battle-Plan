# MAPMODE COVERAGE MATRIX — every the game map mode vs replay fill-law status

**Lane: UI + map-mode INVENTORY (docs-only). Authored 2026-07-09.**
Companion residual ledger to `REPLAY_PRODUCTION_PLAN.md` Phase E3 and
`UI_LAYOUT_LAW.md` §6.2 (map-mode manager). One row per selectable map mode. Per
the 2026-07-09 user scope decision, v1 renders **ALL** map modes, not just
political. Map-mode colors are ALWAYS computed replay-side from recorded inputs —
never recorded (standing override, requirements item 9b note).

## 0. Where the mode list comes from (file evidence)

Three file sources, all cited:

1. **Toolbar buttons** — `interface/mapmodes_interface.gui` window
   `MapmodesInterface_Ingame` declares 12 mode buttons (`mapmode_button_1..12`,
   each `_s`/`_bg` variant) in two layouts (`3_button_mapmode`, `4_button_mapmode`
   — LaR/DLC). Each button's identity is file-visible via its `clicksound`
   (`ui_mapmode_*`) + `shortcut`. Button→mode binding is "selected in code"
   (comment line 96), so the CLICKSOUND is the file-derivable identity.
2. **Engine hard-coded layer types** — `common/map_modes/documentation.md` +
   `mapmodemanager.cpp` RTTI (UI_LAYOUT_LAW §6.2): `game_map_mode_country`,
   `_states`, `_diplomacy`, `_players`, `_factions`, `_ideology`. These are the
   fill/border compute primitives; the "all map modes" list
   (`mapmodes_all` button, `ctrl+F1` → `WindowWithAllMapModes`) exposes them.
3. **Scripted modes** — `common/map_modes/*.txt` `scripted_map_modes = {…}`
   loaded via `TGameItemDatabase<CScriptedMapModeDatabase>`. **Vanilla ships
   NONE** (only `documentation.md`), so the vanilla mode set = toolbar + engine
   hard-coded list. Mods add scripted modes → replay needs the generic layer
   composer (type/color/text/thickness), not per-mode code.

**Honesty caveat:** the exact sub-mode *cycling* inside each strategic toolbar
button (e.g. the naval button steps naval→naval-supremacy→sub-detection) is
engine-code runtime state, NOT fully file-enumerable — the `.gui` gives the
button family (via clicksound) and the engine gives the 6 layer primitives. The
rows below are the file-derivable canonical modes; per-button sub-cycles are
noted where known but flagged runtime.

## 1. Toolbar-selectable modes (12, by clicksound identity)

| # | Mode | selector (button / clicksound / shortcut) | what it colors (fill law source) | data dependency | status | capture/RE need |
|---|---|---|---|---|---|---|
| 1 | **Political** (strategic land / default) | `mapmode_button_1` / `ui_mapmode_strategic_land` / F1 | province/state fill = owner country color (HSV-modified raws) — **COL3 law CLOSED** (`col3-political-color-law`) | item 2 (owner/control) | **done** (fill law + borders) | none |
| 2 | **Naval** (strategic navy) | `mapmode_button_2` / `ui_mapmode_strategic_navy` / F2 | sea-zone supremacy tint = controller color, alpha over `NAVAL_REGION_TRANSPARENCY_*` — **C6 law DONE** (`naval_supremacy_tint`) | item 5 (naval supremacy) | **law-known** (fixture-gated; supremacy unit capture-pending) | supremacy magnitude unit (CAP-gated) |
| 3 | **Air** (strategic air) | `mapmode_button_3` / `ui_mapmode_strategic_air` / F3 | strategic-region air-activity tint | item 8 (air region activity, LANDED extractor-side) | not-started | air-mode tint RE + region fill |
| 4 | **Operatives / intelligence** | `mapmode_button_4` / `ui_mapmode_operatives` | intel-network / operative-presence coloring | **NEW: intel/operatives state** | not-started | intel-mode RE (LaR-gated) |
| 5 | **Supply** | `mapmode_button_5` / `ui_mapmode_supply` / F4 | supply-area coloring (hub range / supply value) | item 11 (rail/supply, deferred) | not-started | supply-mode RE + supply-graph render |
| 6 | **Terrain** | `mapmode_button_6` / `ui_mapmode_terrain` / F5 | province terrain-type fill (static files, no recording) | none (files: `map/terrain`, `00_terrain.txt`) | **law-known** (static terrain colors file-derivable) | terrain-mode color table cite |
| 7 | **Resistance** | `mapmode_button_7` / `ui_mapmode_resistance` / F6 | per-state resistance level gradient | **NEW: occupation/resistance state** | not-started | resistance-mode gradient RE (LaR-gated) |
| 8 | **Compliance** | `mapmode_button_8` / `ui_mapmode_compliance` / F7 | per-state compliance level gradient | **NEW: occupation/compliance state** | not-started | compliance-mode gradient RE (LaR-gated) |
| 9 | **Resource** | `mapmode_button_9` / `ui_mapmode_resource` / F8 | state resource-deposit coloring (static deposits + control) | item 2 (control) + files (`map/resources`, `00_resources.txt`) | **law-known** (deposits file-derivable) | resource-mode color/threshold cite |
| 10 | **Infrastructure** | `mapmode_button_10` / `ui_mapmode_infrastructure` / F9 | per-state infrastructure level gradient | item 10 (buildings — infrastructure level) | not-started | infra-mode gradient RE |
| 11 | **Factions** | `mapmode_button_11` / `ui_mapmode_factions` / F10 | faction-membership coloring — engine `game_map_mode_factions` layer | item 8b (faction membership) | **law-known** (hard-coded layer; color = faction/leader) | faction-mode color rule cite |
| 12 | **Raids** (naval/air raids) | `mapmode_button_12` / `ui_mapmode_raids` | raid-target / naval-raid coloring | **NEW: raid state** (or item 8 naval) | not-started | raid-mode RE (BBA/MtG-gated) |

## 2. Engine hard-coded layer modes exposed via the "all map modes" list (4 additional)

`mapmodes_all` (`ctrl+F1`) opens `WindowWithAllMapModes` listing every mode incl.
the engine primitives not already on the toolbar. `country`=political (#1) and
`factions` (#11) are already above; the remaining 4 are distinct selectable modes:

| # | Mode | engine layer type | what it colors | data dependency | status |
|---|---|---|---|---|---|
| 13 | **States** | `game_map_mode_states` | state borders/fill (administrative) — owner fill with state outlines | item 2 (owner) + static state map | **law-known** (owner fill + state borders, both file-derivable) |
| 14 | **Diplomacy** | `game_map_mode_diplomacy` | pair-relation coloring vs viewed country (ally/enemy/neutral) | item 8b (relations) — FURN-REL column law partly ported (`vp_icon_relation_column`) | **law-known** (relation→color rule ported for VP icons; extend to fill) |
| 15 | **Players** | `game_map_mode_players` | one color per human player (MP) | item 4 (player↔country mapping) | **law-known** (player map file-trivial; color assignment RE) |
| 16 | **Ideology** | `game_map_mode_ideology` | ruling-ideology coloring (fascism/communism/democratic/neutrality) | item 7b (ruling party) | **law-known** (4 ideology colors file-derivable from `00_*` ideology defs) |

## 3. Display toggles / overlays (6 — NOT modes, but map-display state)

`mapmodes_interface.gui` also declares 6 toggle buttons that modify the current
mode's display rather than selecting a mode. Enumerated for completeness; each is
a replay render option, several already have law.

| Toggle | button / clicksound | effect | data dependency | status |
|---|---|---|---|---|
| Day/night | `day_night_toggle` / `n` | night lighting overlay | none (replay camera/time) | law-known (LIGHT/night law; capture residual CAP3) |
| Player counters only | `player_counters_toggle` / `m` | filter counters to human players | item 4 | not-started (Phase C counters) |
| Counter color mode | `button_counters_color_mode` / `,` | NATO vs sprite counters | none (render option) | not-started (Phase C1) |
| Allied plans | `button_allied_plans` / `.` | show allied plan arrows | item 12 (plans, deferred) | descope (deferred) |
| Fog of war | `button_fog` | toggle FoW overlay | item 13 (visibility) — **C8 FoW law CRACKED** | law-known (fow_law.rs); tied to observer POV E6 |
| Radar | `button_radar` | radar-coverage overlay | **NEW: radar coverage** (or buildings item 10) | not-started |

## 4. Counts + status summary

- **Selectable map modes (file-derivable) = 16**: 12 toolbar (clicksound-identified)
  + 4 additional engine hard-coded layer modes (states, diplomacy, players,
  ideology). `country`/`factions` counted once (on toolbar).
- **Display toggles/overlays = 6** (day/night, player-counters, counter-color,
  allied-plans, fog, radar) — not modes.
- **Status of the 16 modes:**
  - `done` = **1** (political — COL3 fill law + borders).
  - `law-known` = **8** (naval, terrain, resource, factions, states, diplomacy,
    players, ideology — fill law file-derivable or ported, gating/binding remains).
  - `not-started` = **7** (air, operatives, supply, resistance, compliance,
    infrastructure, raids — need mode-specific RE + a new/deferred data stream).
- **New data dependencies for map modes** (fold into REPLAY_DATA_REQUIREMENTS):
  intel/operatives state (mode 4), occupation resistance/compliance (modes 7/8) —
  **shared with UI_COVERAGE_MATRIX §9 items 6 & 8** (no duplicate stream), raid
  state (mode 12), radar coverage (toggle). Supply (mode 5) = item 11; infra
  (mode 10) = item 10; air (mode 3) = item 8 — all already listed.

**Mode grand total = 16 selectable map modes + 6 display toggles.** Vanilla ships
0 scripted modes; the mod path needs the generic scripted-layer composer
(type/color/text/thickness per `documentation.md`), tracked as one RE item.
