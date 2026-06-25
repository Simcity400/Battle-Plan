# Layer 2 — Game-authentic replay UI: window → data spec

> This is the build spec for the **game-authentic replay layer**: a faithful recreation
> of the real the game interface, rendered with the game's **real assets** (`game_assets/`)
> and driven by **replay snapshots** (`replay/SCHEMA.md`). Every menu/window should look
> like the game; the user scrubs/plays across the snapshot series and watches the world
> evolve. Pairs with **Layer 1** (`replay_ui/`, the stats/graphs view) under one shell
> with a mode toggle.
>
> Two hard rules carry over from DESIGN: **no fabrication** (a `null` schema field renders
> as "not captured" or is hidden — never invented) and **graceful degradation** (a
> single-snapshot replay disables the scrubber and collapses time series to one instant).

## Inputs

- **Data:** `replay/store.py` → `SnapshotStore.load_all_ordered(game_id)` gives the ordered
  `[snapshot]`; `<game_id>/timeline.json` gives the merged event feed. Shapes = `replay/SCHEMA.md`.
- **Art:** `game_assets/sprites.json` (sprite name → PNG path + frame info), `game_assets/assets/`
  (PNGs; gitignored), `game_assets/gui/*.json` (parsed window layouts — use for geometry).
- **Map geometry:** `map_pipeline/` province bitmap + `province_to_state.json` + province centroids
  (already used by `replay_render/`) for placing combat bubbles / unit counters on the map.

> **Schema note (combats are now cracked):** `SCHEMA.md §7` still tags `combats[]` as `M*`.
> That is now stale — **land** combat is live-memory native (attacker+defender, validated
> 13/13) via `memory_reader/combat/`, plus save `combats.json` for casualties. Treat
> `combats[]` as populated (land); naval/air remain `M*` until a live sea/air battle is
> observed. The live combat reader feeds the snapshot `combats[]` in the live-capture path;
> the save oracle fills it for save snapshots.

---

## A. Global chrome

### A1. Topbar (always visible)
Recreate the game top resource/status bar. Per the selected/!player country (or a chosen TAG):
| topbar element | schema field | sprite |
|---|---|---|
| country flag + name | TAG + `countries[TAG].politics.faction`/ideology | `gfx/flags/<TAG>_<ideology>` |
| political power | `countries[TAG].politics.political_power` | PP icon (interface) |
| stability / war support | `.politics.stability` / `.war_support` (0..1 → %) | stability / war_support icons |
| command power | `.politics.command_power` | CP icon |
| manpower | `.military.manpower.available` | manpower icon |
| factories civ/mil/dock | `.economy.factories.{civ,mil,dockyard}` | the three factory icons |
| convoys | `.military.convoys[]` count / `.economy` | convoy icon |
| fuel | `.economy.fuel.current` / `.max` | fuel icon |
| resources strip | `.economy.resources.available_derived.*` | per-resource icons |
| date + speed | snapshot `date`; "speed" replaced by the **scrub/play control** | calendar/clock chrome |

The right side of the topbar = the **replay transport** (play/pause, speed, a timeline
scrubber over `hour_index`), styled into the game chrome.

### A2. Outliner / sidebar
The game's right-side outliner → list pinned/major countries with flag + a few key stats
(`division_count`, factories, PP). Clicking a country opens its National view (§C).

---

## B. Map view (central, default screen)
Reuse the `replay_render` raster pipeline as the base map; render game-style overlays:
| overlay | schema field | notes |
|---|---|---|
| owner political map | `map.owner_by_state` (+`states[*].owner`) | flag/ideology color per state |
| controller / occupation | `map.control_by_province` (+`provinces[*].controller`) | stripes where controller≠owner |
| front lines | `map.front_lines[]` (else derive from control adjacency) | |
| **combat bubbles** | `combats[]`: `location_province` → province centroid → bubble | show attacker vs defender flags, `combat_width`, `progress` (-1..1 bar), `terrain`; click → Combat view §G |
| division counters | `divisions[]` grouped by `location_province` | game-style unit counters (flag + strength); size by Σ`strength` |
| state/province click | → State view §F | |
| **map modes** | see **§B.1** | a mapmode toggle bar mirroring the game's; the selected mode persists as you scrub time |

### B.1 Map modes
Recreate the game's mapmode bar. The selected mode is global and **persists across the
scrub** (changing time recolors the *current* mode). Honor **no-fabrication**: a mode whose
data we don't capture renders **disabled** with a "not captured" tooltip — never faked. Group:

**Fully supported (per-snapshot data):**
| mode | shows | schema source |
|---|---|---|
| Political *(default)* | state owner | `map.owner_by_state` / `states[*].owner` (flag/ideology color) |
| Controller / occupation | province controller; stripe where controller≠owner; resistance/compliance shading | `map.control_by_province`, `states[*].occupation.{resistance,compliance,operational_status}` |
| Faction | color by faction membership | `countries[*].politics.faction` + `diplomacy_world.factions` (derive TAG→faction→color) |
| Diplomatic | from the **selected** country: allies / enemies / neutral / guaranteed / opinion gradient | `countries[SEL].diplomacy.{wars,guarantees,non_aggression,embargoes,opinion}` (+`diplomacy_world`) |
| Victory points | VP weight per state | `map.vp_by_state` / `states[*].victory_points_total` |
| Resources | deposits per state | `states[*].resources` |
| Manpower | state manpower pool | `states[*].manpower_pool` |
| Buildings / industry | infra, civ/mil/dock, forts, air/naval base levels (sub-select which) | `states[*].buildings` + `states[*].province_buildings` |
| Front lines | control-adjacency fronts | `map.front_lines[]` (else derive) |

**Partial (sparse or single-channel — render where present, grey the rest):**
| mode | shows | schema source | gap |
|---|---|---|---|
| Supply | supply level | `provinces[*].supply` (S, sparse) / `states[*].supply` (M\*) | sparse; grey unmeasured |
| Railway / infrastructure flow | rail level + usage | `provinces[*].rail_usage` / `route_usage` (S) | usage only when emitted |
| Air bases | air base coverage | `states[*].buildings.air_base` | base **levels** only — air *superiority* is not captured |
| Intel | intel level on others | `countries[SEL].intel` + targets (S, sparse) | sparse |

**Static (render, but does not change across the scrub):**
| mode | shows | source |
|---|---|---|
| Terrain | terrain type | `map_pipeline` terrain bitmap (static) |
| Strategic regions | region borders/areas | `map.strategic_regions` (S, carried once) |

**Being cracked now — dedicated memory-RE agents are LIVE on these (goal: ZERO permanently-disabled modes; until a layer lands, render disabled with a "capturing…" note, never faked):**
- **Naval** (sea-zone / strategic-region control, fleet positions) → `memory_reader/naval_air/`.
- **Air superiority** (contested air per region) → `memory_reader/naval_air/`.
- **Weather** (precip / storm / mud / frozen, dynamic; memory-only — not in saves) → `memory_reader/weather/`.
- **Supply / Railway / Intel** — promotes the "Partial" rows above to FULL per-province coverage → `memory_reader/logistics_intel/`.
- **Division movement / in-combat / encirclement** (unit arrows + counters on the map) → `memory_reader/division_live/`.

> Implementation: each mode = a pure `(snapshot) → {state_or_province_id: color}` function fed
> to the same raster recolor `replay_render` already uses; the bubble/counter overlays (§B) sit
> on top regardless of mode. Add new modes by adding a colorizer — the transport/scrub is shared.

---

## C. National view (country window) — the main per-country screen, tabbed
Open by clicking a country (map or outliner). Header = flag, name, ruling party, faction,
ideology. Tabs below mirror the game's national window:

### C1. Politics
`countries[TAG].politics`: `ruling_party`, `party_popularities`, `laws.{economy,trade,conscription}`,
`ideas_other[]` (spirits/advisors as their idea icons from `gfx/interface/ideas`), `cabinet`,
`stability`/`war_support` gauges. Government/ideology change shows on the timeline (`government_changed`).

### C2. National Focus (tree)
`countries[TAG].focus`: `focus_tree` (which tree), `completed[]` (drawn done), `current` + `progress`
(in-progress bar), `paused`. Render the focus tree using **goal icons** (`gfx/interface/goals`,
keyed by focus id). `focus_completed` events drive the per-focus completion timeline.

### C3. Research
`countries[TAG].research`: `technologies[]` (done, with `date`), `in_progress[]` (the **research
queue** — name + `points_factor`; `progress_pct` is M\*/null in save, render as indeterminate),
`research_slots`. Tech icons from interface sprites. `tech_researched` events on the timeline.

### C4. Production
`countries[TAG].economy`: `construction_queue[]` (the **construction tab** — each `template`,
`location_state`, `active_factories`, `produced`, `cost`, `priority`) and `military`
equipment lines (`equipment_stockpile[]` + `gross_ic_by_equipment`). Equipment icons from sprites.
`production_milestone`/`factory_milestone` events.

### C5. Logistics & trade
`economy.resources.{produced,exported,available_derived}`, `economy.fuel`, `military.convoys[]`.

### C6. Military / recruit & deploy
`military`: `division_count`, `manpower.*`, `xp.*`, `num_ships`, `ships_by_hull`,
`casualties.{killed,captured}`. Template list from the divisions' `template_name`/`role`.

### C7. Diplomacy
`countries[TAG].diplomacy` + world `diplomacy_world`: `wars[]`, `factions`, `subjects`,
`guarantees[]`/`non_aggression[]`/`embargoes[]`, `opinion`. War/faction/subject events on timeline.

### C8. Decisions
`countries[TAG].decisions`: `active[]`, `timed[]` (+ cooldowns), `missions[]`. (One-shot clicks are
not stored — surface only what persists, plus decision-derived events.)

### C9. Intel
`countries[TAG].intel`: `operatives[]`, `intel_networks[]`, `decryption`, `peace_conference`.

---

## D. State / province view
Click a state/province → game-style state window: `states[id]` `owner`/`controller`,
`buildings` + `province_buildings` (with the game's building icons), `victory_points_total`,
`resources`, `manpower_pool`, `category`, `occupation` (resistance/compliance), `demilitarized`.
Provinces: `provinces[id].controller`/`supply`.

## E. Division view
Click a unit counter → `divisions[]` entry: `template_name`/`role`, `organisation`, `strength`,
`equipment[]` (with equipment icons), `location_province`/`state_id`, `in_combat`, movement
(`moving`/`move_target_province`/`move_progress` — M\*, may be null).

## F. (covered in D)

## G. Combat view (the combat tab + bubble detail)
From a map combat bubble or a combats list: `combats[]` entry — `kind`, `location_province`,
`attackers[]` / `defenders[]` (resolve div_ids → `divisions[]` for org/strength/template; flags
per owner), `combat_width`, `terrain`, `progress`, `modifiers`. Lifecycle (`combat_started`/
`combat_ended` + casualties) from the timeline. **Naval/air**: schema-present but `M*` — render
"not captured this snapshot" rather than blank/fake.

## H. Event ticker / log
The game's event/alert feed → render `timeline.json` chronologically, with **source + confidence
badges** (M/S/D/marker; exact/interval/inferred). Filter by type and country. Clicking an event
jumps the scrubber to its `hour_index` and focuses the relevant country/state.

---

## I. Sprite lookup convention
1. Resolve a logical need (e.g. "Germany fascist flag", "infantry equipment icon", "stability
   icon", focus id `GER_xyz`) to a **game sprite name** (or a direct flag path `<TAG>_<ideology>`).
2. Look it up in `game_assets/sprites.json` → PNG path (+ frame index for multi-frame strips).
3. Fall back to a neutral placeholder (and `log` the miss) if a sprite is absent — never block.

## J. Mode toggle / shell integration
- One shell, two modes: **Stats** (`replay_ui/`, Layer 1) and **Replay** (this layer). A top-level
  switch flips between them; both read the same `SnapshotStore` + timeline, share the scrubber's
  current `hour_index`.
- Self-contained offline (static HTML/CSS/JS + the gitignored asset PNGs; optional stdlib
  `http.server` launcher mirroring the dashboard's `.bat`). Reuse Layer 1's data-loading where
  possible; do not duplicate the store reader.

## K. Build order (suggested for the Layer-2 agent)
1. Asset/sprite loader + a tiny "sprite by name → <img>" helper over `sprites.json`.
2. Topbar + map base + the scrub/play transport (the always-on chrome) — the "it looks like the game"
   moment, on real data.
3. National view shell + tabs C1–C3 (politics, focus tree, research) — highest visual payoff.
4. Production (C4), military (C6), diplomacy (C7); state/division/combat views (D/E/G) + combat bubbles.
5. Event ticker (H), decisions/intel (C8/C9), map modes, polish.
6. Mode toggle wiring (J). Codex-review the data-binding + no-fabrication handling each phase.
