# BINDING MAP — CHROME + OUTLINER + MAPS (clock / minimap / alerts / find / right-click / map-modes / map-icons)

> **What this is.** The COMPLETE, no-truncation data-binding map for seven in-game the game "chrome / outliner /
> map" views the native replay UI binds in the Phase-3 sweep:
> **in-game clock** (`game_assets/gui/ingame_clock.json`, from `interface/ingameclock.gui`),
> **minimap** (`minimap.json`, from `interface/minimap.gui`),
> **alerts** (`alerts.json`, from `interface/alerts.gui`),
> **find view** (`find_view.json`, from `interface/findview.gui`),
> **right-click menu** (`rightclick_menu.json`, from `interface/rightclickmenu.gui`),
> **map-mode switcher** (`mapmodes.json`, from `interface/mapmodes_interface.gui`), and the
> **on-map icons** (`mapicons.json`, from `interface/mapicons.gui`).
> It realises GAME_UI_REPLICA_PLAN §C ("comprehensive but mechanical binding sweep"), §B.9 (the map engine
> + map-mode recolor table), and §C.4 (view inventory — outliner/alerts/find are imperative adapters).
> EVERY element in each parsed `.gui` tree is accounted for — dynamic (data-bound, incl. derived/unavailable),
> static/decoration (no binding), or deferred-needs-adapter. Nothing is omitted; where a `.gui` element exists
> but our snapshot has no usable field, the element is listed and **badged `unavailable`** with the missing
> field (and the precise capture gap) named. Per-view **element-count tables** at the end prove exhaustiveness.
>
> **Binding-rule shape** (PLAN §C.1): `{ target (gui element name+type, sprite/text/loc key), source (exact
> snapshot dotted field path), format (formatter id), fidelity (exact|derived|proxy|unavailable) }`. **Source
> paths** use SCHEMA dotted form: `countries.<TAG>.…`, `states.<id>.…`, `provinces.<pid>.…`, and the top-level
> blocks `map.*`, `naval_air.*`, `supply.*`, `world.*`, `diplomacy_world.*`, `raids.*`, `strategic_air.*`.
> `<TAG>` = the player/observed country; `<id>` = a state id; `<pid>` = a province id.
>
> **STATIC MAP METADATA vs PER-SNAPSHOT SCHEMA (load-bearing distinction).** The map engine consumes two kinds
> of data: (1) the **per-snapshot schema** (changes every tick — owner/controller/occupation/VP/buildings, the
> `map.*`/`supply.*`/`naval_air.*` sidecars), and (2) **static map metadata** produced once by `map_pipeline`
> (`map_pipeline/assets/{province_ids.png, province_to_state.json, state_meta.json, country_colors.json,
> strategic_region_meta.json, province_to_strategic_region.json}`) — geometry, the province→state join, state
> names/categories/VP geometry, the per-TAG map RGB, and the strategic-region geometry. Static metadata is
> **NOT a per-snapshot field**; wherever a binding uses it, it is flagged `[static map meta]`. The recolor LUT
> (PLAN §B.9) = a per-snapshot schema field joined through the static metadata into a per-state/per-province
> colour.
>
> **No-fabrication (SCHEMA §0.2, PLAN principle 3):** `null` renders "—", never `0`. `0`/`0.0` is a measured
> zero. A schema-present field the current parser never feeds is badged `unavailable` (not silently dropped).
> A runtime-computed value we don't model is `derived`/`proxy`/`unavailable` by nature.

---

## Provenance of the fidelity calls

Calls below are grounded in **the canonical schema** (`replay/SCHEMA.md` + `replay/schema.py` `FIELD_SOURCES`/
`DOMAINS`) and the map-mode fidelity table the plan already locked (**GAME_UI_REPLICA_PLAN §B.9**), cross-checked
against the same real save-backed snapshot family the sibling binding maps used
(`replay/snapshots/replay/000017005619.snapshot.json`, `schema_version "1"`, date `1941.4.13.11`, GER 1941
channel), and the `map_pipeline/assets/*.json` static products (`state_meta.json` keyed by state id →
`{name_key, provinces[], vp_total, victory_points{pid:vp}, state_category, manpower}`; `province_to_state.json`
flat `{pid: state_id}`; `country_colors.json` `{TAG:[r,g,b]}`). **Adversarially reviewed** in a
historical review round on 2026-06-26; the asked/agreed/refuted/fixed log is the final section.

**The decisive facts that drive the badges in these seven views:**

| fact | source | consequence |
|---|---|---|
| The clock/minimap/alerts/find/right-click `.gui` are **mostly template + chrome** — the *content* (alert icons, find results, right-click options, on-map counters) is **adapter-instantiated at runtime**, not declarative `.gui` elements | PLAN §C.4 (outliner/alerts = imperative adapters); the parsed JSONs (e.g. `right_click_entry` has an empty-text `button`, `results_grid_box` is an empty gridbox) | these views bind via **row/entry factories** + **deferred adapters**, mapped per template below |
| Map-mode recolor fidelity is **already locked** in §B.9 | PLAN §B.9 table | owner/control/resistance/compliance/VP/buildings/resources = **exact**; weather = exact-value/proxy-vs-game-mode; front-lines = derived; faction/wars = exact at country granularity; air/naval/supply = **proxy** (region/division-province) |
| `supply.by_province` is a **division-province PROXY**; the exact per-province supply NUMBER is a **proven wall** (not stored) | `replay/SCHEMA.md §13.2` + `memory_reader/supply_solver_offsets.draft.json` + `RESEARCH_logistics_intel.md` | on-map supply counters use `supply.by_province` proxy values; the native supply map mode reads `provinces.*.supply` when present and remains a proxy/badged value, never the exact the game tooltip `99/99` number |
| `combats[]` is the set of CURRENTLY-ACTIVE engagements, all `M*`-uncracked except enumeration; in the inspected snapshot **13 land, 0 naval**; `combats[].progress` is a large positive float, **NOT** the `-1..1` GUI bar | SCHEMA §7 + sibling combat binding map | the on-map land/naval combat icons bind enumeration + side flags (derived); the progress bars + `combat_progress` text are `unavailable` |
| `commanders`, `order_of_battle.air_wings`/`navy_theaters` are save-sourced and **not folded in the inspected snapshot**; `order_of_battle.{armies,army_groups,fleets}` + `order_of_battle.ships` ARE live-first | SCHEMA §3.9/§12 + `FIELD_SOURCES` | on-map general portraits/leaders = `unavailable` (current data); naval/air on-map counters bind from OOB where live, else `unavailable` |
| Per-state `resources` are now live-fed; `occupation_law` remains `null` in the inspected family; `manpower_pool` + `occupation.{resistance,compliance}` + `buildings` + `victory_points_*` ARE populated/live | sibling state/province binding map empirical table + `FIELD_SOURCES` | resources map mode is exact from measured state resources; resource on-map icons emit exact anchored markers for non-empty measured blocks; resistance/compliance/VP/buildings map modes + icons = exact |
| `raids`, `strategic_air`, `world.flags`/`saved_event_targets`, `diplomacy_world.{wars,factions,subjects}` ARE save-present | SCHEMA §14/§16/§18/§7b | on-map raid/air-base/faction-ping/peace icons bind where those domains are fed; intel on-map icons bind from `intel.*` |

> **Read-only verb note (PLAN §D).** Every `*_button` whose in-game role is to PING the map, MOVE the supply
> capital, TOGGLE fog/radar/night, START a decision, or DISENGAGE is a **mutation / live-session verb**. In a
> replay these render as real chrome but are **inert** (no datum, no action). They are listed under each view's
> static/deferred section, never bound.

---
---

# VIEW 1 — IN-GAME CLOCK (`game_assets/gui/ingame_clock.json`)

Root: `containerWindowType "clock_window"` (lower-right). This is the small standalone **time readout** that
sits above the minimap (distinct from the topbar `DateText`, which carries the full date — this one shows only
`HH:MM`). Formatter `clock-time` = Clausewitz date `"Y.M.D.H"` → `H:00` wall-clock (the game ticks hourly; minutes
are always `00`, so the placeholder `"13:37"` renders as `13:00`).

## 1a. Clock — dynamic (data-bound)

| gui element (name + type, text) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `time_text` (instantTextBoxType, `"13:37"`) | `snapshot.date` (top-level, country-independent) | clock-time (`H:00`) | **exact** | Clausewitz `"1941.4.13.11"` → `11:00`. The hour component drives the readout; the scrubber spine sets `snapshot.date`. Minutes are not modelled by the engine (hourly tick) → always `:00`. |

## 1b. Clock — STATIC / decoration / positioning (no data binding)

- **Backgrounds / chrome:** `background` (`GFX_ingameclock_bg`), `time_box` (`GFX_generic_box_smallest`) — frame
  chrome behind the text.
- **Positioning anchors (positionType, no visual):** `peace_conference_offset`, `minimap_offset` — layout
  anchors the shell uses to slot the clock relative to the peace-conference panel / minimap; no datum.

---

# VIEW 2 — MINIMAP (`game_assets/gui/minimap.json`)

Two top-level surfaces: ping-marker icons (`minimap_offensive_icon` / `minimap_defensive_icon`) and the
`mini_map_interface` container (the world-map thumbnail + the camera viewport quad + ping buttons). The current
native slice draws the real `.gui` chrome plus the camera viewport outline and supports read-only click/drag
recenter. The thumbnail raster itself is still **unavailable** in this registry-opened view because the live
`PoliticalMap` raster is owned by the seeded `core.map` view and is not yet shared into `chrome.minimap`; it must
not be described as implemented recolor parity. The camera quad reflects the 3-D camera's current viewport
(a render-state, not country data). Pings are a **live multiplayer verb** (one player drops a ping for teammates)
-> no replay datum.

## 2a. Minimap — dynamic (data-bound)

| gui element (name + type, sprite) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `worldmap_bg` (iconType, `GFX_minimap_world`) | blocked: the active `PoliticalMap` raster is not available to the registry-opened minimap view | unavailable badge | **unavailable** | Current native minimap draws the panel background and camera viewport outline only (`draw_minimap(..., None, ...)`). A true thumbnail recolor should reuse the same map layers/LUT as the main map, but that raster handoff remains open and must not be fabricated. |

## 2b. Minimap — STATIC / decoration / camera-state / live-verb (no data binding)

- **Camera viewport quad (render-state, not country data):** `camera_quad_wrap`, `camera_quad_wrap_top`,
  `camera_quad_wrap_bottom`, `camera_quad_left`, `camera_quad_right`, `camera_quad_top`, `camera_quad_bottom`
  (all `GFX_minimap_quad_pixel`) — the white rectangle showing where the 3-D camera is looking. In the replay
  the camera owns this; it tracks the view, not a snapshot field → static/render-state, not bound.
- **Frames / chrome:** `worldmap_border` (`GFX_minimap_border`), `circle` (`GFX_minimap_ping_circle` — ping
  target reticle).
- **Ping markers + buttons (live MP verb — inert in replay):** `minimap_offensive_icon`
  (`GFX_minimap_ping_offensive`), `minimap_defensive_icon` (`GFX_minimap_ping_defensive`), `offensive_ping`
  (buttonType), `defensive_ping` (buttonType), `handle` / `handle_pinged` (buttonTypes — the drag handle, with a
  "someone pinged" alt frame). Map-pings are a live session action with no persisted state and no log marker
  (SCHEMA §10 sub-sample-transient) → **inert/unbindable** in a replay.
- **Structural containers:** `mini_map_interface`, `border_container`.

---

# VIEW 3 — ALERTS (`game_assets/gui/alerts.json`)

Two icon-window TEMPLATES the **alerts adapter** instantiates per active alert: `global_alerticon_window` (a
global alert badge — low supply, encirclement, focus available, etc.) and `global_diplorequesticon_window` (an
incoming diplomatic-request badge with the requester's flag). Per PLAN §C.4 the alerts feed is an **imperative
adapter**, not a declarative country-field bind: alerts are a runtime, fog-of-war, per-player computation
(many alert types have no persisted snapshot field). The replay surfaces alert-equivalent information from the
**diff-event timeline** (SCHEMA §9) where possible, NOT from a stored "active alerts" domain.

## 3a. Alerts — dynamic / derivable (per-template)

| gui element (name + type, sprite) | source | formatter | fidelity | notes |
|---|---|---|---|---|
| `global_diplorequesticon_window.country_flag` (iconType, `GFX_flag_smallest`) | the requesting country `<TAG>` of a diplomatic-request alert | flag-smallest | **derived** | IF the alerts adapter reconstructs a diplomatic request (e.g. from a `faction_joined`/`subject_created`/lend-lease diff event, SCHEMA §9.2), the requester TAG resolves the flag. The flag itself is exact-from-TAG; whether the alert EXISTS is adapter-reconstructed → derived. |
| `global_alerticon_window.icon` (buttonType, `GFX_global_alert_icons`) | alert category → frame of `GFX_global_alert_icons` | category (alert-type→frame) | **unavailable** | The alert TYPE (which frame) is a live per-player computation; there is **no stored "active alerts" domain** in the schema (capture gap: *no `alerts[]`/per-country alert ledger is captured — alerts are recomputed live and never persisted*). A future adapter could surface a SUBSET (low-supply from `supply.by_province` proxy; focus-available from `focus`; war from `diplomacy_world`) as `derived`/`proxy`, but the general alert badge is `unavailable`. |

## 3b. Alerts — deferred (needs imperative adapter; NOT bound in this sweep)

| gui element(s) | why deferred |
|---|---|
| `global_alerticon_window` (whole window: `icon`, `red_alert_glow`, `yellow_alert_glow`) + the 4 layout anchors (`alerticon_startposition`, `alerticon_startposition_extended`, `alerticon_offset`, `alerticon_endposition`) | **deferred: needs adapter** — the global-alert STRIP is an imperative alerts feed (PLAN §C.4). The adapter would lay out N alert badges between `alerticon_startposition` and `alerticon_endposition` along `alerticon_offset`; the badge COUNT + each badge's type/glow are a live computation, not a per-element snapshot bind. The two glows (`red_alert_glow`/`yellow_alert_glow`) encode urgency (red=critical, yellow=warning) — a severity the adapter would assign, not a stored field. |

## 3c. Alerts — STATIC / decoration (no data binding)

- **Glows (cosmetic, adapter-driven visibility):** `red_alert_glow` (`GFX_red_alert_glow`), `yellow_alert_glow`
  (`GFX_yellow_alert_glow`) — listed under 3b as part of the deferred badge; pure visibility/severity overlays,
  no datum of their own.
- **Positioning anchors (positionType, no visual):** `alerticon_startposition`,
  `alerticon_startposition_extended`, `alerticon_offset`, `alerticon_endposition` — the strip-layout anchors.

> **Schema-gap (capture work-item):** *no `alerts[]` / per-country active-alert ledger* is captured. Alerts are
> recomputed live by the engine per player and never persisted (not in the save, not pinned in memory). Until
> an alerts adapter reconstructs a documented SUBSET from existing domains, the alert badge type/glow is
> `unavailable`.

---

# VIEW 4 — FIND VIEW (`game_assets/gui/find_view.json`)

A **search panel**: an edit box (`to_find_text`), a results gridbox (`results_grid_box`) populated with
`find_view_item` rows, a Find button, a "go to capital" button, and a close button. The current native slice is a
browsable read-only catalog over snapshot entities: States, Countries, and Victory Points tabs produce
adapter rows from captured state/country fields, the list is uncapped and wheel-scrollable, and state/VP row clicks
open the read-only state panel. The `to_find_text` edit box now supports replay-local typed filtering over the
honest displayed row labels/keys. The `find_button` and `goto_home_button` now emit read-only map recenter
commands from selected/first filtered catalog rows and measured capital VP geometry.
The results list also has scrollbar-thumb drag with captured drags continuing outside the list until pointer-up;
track clicks are inert and do not select or drill underlying rows. Localized names, country detail panels,
province-level VP drilldown, exact scrollbar art, and screenshot proof remain
open.

## 4a. Find view — dynamic / derivable

| gui element (name + type, text) | source | formatter | fidelity | notes |
|---|---|---|---|---|
| `results_grid_box` (gridboxtype) → **N × `find_view_item`** | browse/query over captured `states.<id>.owner`, `states.<id>.victory_points_total`, and observed-country identity fields | row factory | **derived** | Native exposes a browse catalog for States/Countries/Victory Points and filters it with replay-local `to_find_text` input over the displayed token/key strings. It labels rows with honest tokens such as `#64 (GER)`, `#64: 50 VP`, or `GER` plus measured identity enrichments; no localized static-map name is synthesized. The row catalog is uncapped and wheel/thumb-scrollable; Find/Goto Capital can recenter selected or first filtered results using state/capital geometry. Country detail panels, province-level VP drilldown, localized names, exact scrollbar art, and live-game extras (find-by-decision, find-by-template) remain open. |
| ↳ `find_view_item.name` (instantTextboxType, `"invasion"`) | the catalog row token described above | row text | **derived** | One row per result; the row label is the snapshot-derived token, not a localized entity name yet. `find_view_item` is a runtime ROW template (the row is not a declarative element). |

## 4b. Find view — STATIC / decoration / navigation / UI-input (no data binding)

- **Backgrounds / chrome:** `find_view_item.Background` (`GFX_sort_button_202x29_button`),
  `find_view.Background` (`GFX_tiled_window`), `find_editbox_bg` (`GFX_find_editbox`), `results_window.Background`
  (`GFX_tiled_plain_bg`), `GFX_find_screen_bottom` (iconType — note: the element NAME equals the sprite name, a
  vanilla quirk; bottom chrome).
- **Search input / navigation buttons (replay-local UI input + read-only verbs):** `to_find_text` (editBoxType —
  the search box; replay-local input, no country datum; filters the current catalog labels/keys), `find_button`
  (buttonText `FIND` — recenters the selected row or first filtered state/country/VP row when measured geometry is
  available), `goto_home_button` (buttonText `GOTO_CAPITAL` — recenters camera on
  `countries.<TAG>.politics.capital_state` VP geometry when measured), `back_button`
  (`GFX_close_button_small` — close).
- **Structural containers / layout:** `find_view_item` (row-template root), `find_view` (root),
  `results_window`, plus the `margin` / `slotsize` layout nodes in the results gridbox (geometry only).

---

# VIEW 5 — RIGHT-CLICK MENU (`game_assets/gui/rightclick_menu.json`)

A **context-menu** scaffold: three row TEMPLATES (`right_click_entry` = a labelled action; `right_click_entry_2`
= an action with an extra `open_button`; `empty_right_click_entry` = a spacer) plus the menu shell
(`rightClickMenu` → a `listboxType` `options`). The menu CONTENTS are entirely **context-dependent and
adapter-built** at right-click time (what you right-clicked on → which actions apply); the parsed `.gui` carries
no fixed options (the buttons have empty `buttonText`). Almost every right-click action is a **mutation** (move
army, set rally point, assign order, release nation) → **inert in a replay**. The menu is therefore a
**deferred adapter** surface; in a read-only replay it would most plausibly not appear at all (or show only
read-only "open X" verbs).

## 5a. Right-click menu — dynamic / derivable

| gui element (name + type, text) | source | formatter | fidelity | notes |
|---|---|---|---|---|
| `right_click_entry.button` (guiButtonType, empty `buttonText`) | per-context action label → loc | loc | **derived** (label) / **unavailable** (action) | IF the replay offers any read-only right-click verb (e.g. "open this state", "go to capital"), the adapter sets this row's label. The LABEL is derivable; the ACTION behind it is a mutation → inert (5b). The option SET is fully adapter-computed from what was clicked (no snapshot field enumerates context options). |
| `right_click_entry_2.button` + `open_button` (guiButtonType, `"Open"`) | as above, with a secondary "Open <target> view" read-only verb | loc | **derived** (label) | The `open_button` ("Open") is the one plausibly-live verb in a replay: open the clicked entity's view (read-only). Label derived; the rest inert. |

## 5b. Right-click menu — deferred (mutation context-menu; inert in replay)

| gui element(s) | why deferred |
|---|---|
| `rightClickMenu` (root) + `options` (listboxType, `SandboxFloaterBackground`) + the row templates `right_click_entry`, `right_click_entry_2`, `empty_right_click_entry` | **deferred: needs adapter / mutations** — the context menu is built imperatively per right-click target, and its options are overwhelmingly MUTATIONS (move/order/assign/release/build). Read-only replay shows no action menu (or only the "Open" read-only verbs in 5a). No declarative bind; the `options` listbox is adapter-filled. |

## 5c. Right-click menu — STATIC / decoration (no data binding)

- **Chrome:** `options` `sprite` `SandboxFloaterBackground` (the menu's floating background), the row buttons'
  `button_type_4` / `button_type_9` sprites (`button` / `open_button`).
- **Structural / layout:** the three windowType template roots, `borderSize` (listbox geometry).

---
---

# VIEW 6 — MAP-MODE SWITCHER (`game_assets/gui/mapmodes.json`)

Root: `containerWindowType "MapmodesInterface_Ingame"` (lower-right). This is the **map-mode button rail** plus
the social-view widget, the configurable-mapmode list windows, and the bottom toggle row (day/night, player
counters, counter colour, allied plans, fog, radar). The mode buttons are GENERIC slots (`mapmode_button_1..12`,
each with a `_s` selected-state twin + a `_bg`/`_s_bg`); the engine assigns each numbered slot to a concrete map
mode at runtime, and the slot's **tooltip/legend** describes the mode. **The binding here is NOT per-button — it
is per-MAP-MODE**: each mode is a **recolor function** over the province-id raster (`[static map meta]`
`province_ids.png` → state via `province_to_state.json`), driven by the snapshot domain that mode visualises, and
carrying that domain's fidelity badge. Selecting a slot swaps the active recolor LUT (View 2's minimap + the main
map both rebind to it). Because the slot→mode assignment is configurable and not in this `.gui`, the table below
binds each **map MODE** (the abstraction every `mapmode_button_*` instantiates) to its driving domain, exactly
per PLAN §B.9.

## 6a. Map MODES — the recolor bindings (each mode = one `mapmode_button_*` slot at runtime)

| map mode (mode button instance) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| **Political (owner)** | `map.owner_by_state` (mirror of `states.<id>.owner`) → `[static map meta]` `country_colors.json` | state-recolor LUT | **exact** | The default mode. Each state painted its owner TAG's RGB. `owner` is `M/S` (live-first). |
| **Control (controller + contested hatch)** | `map.control_by_province` (mirror of `provinces.<pid>.controller`) + `states.<id>.controller` | province-recolor LUT + hatch where controller≠owner | **exact** | Province-level control is cracked (`provinces.<pid>.controller` `M/S/D`); contested states hatch owner-vs-controller. |
| **Resistance** | `states.<id>.occupation.resistance` (0..100) | state-recolor heat LUT (0..100→colour ramp) | **exact** | `occupation.resistance` is `M/S` (live state_detail). Occupied states only; un-occupied render neutral. |
| **Compliance** | `states.<id>.occupation.compliance` (0..100) | state-recolor heat LUT | **exact** | `occupation.compliance` `M/S`. |
| **Victory points** | `map.vp_by_state` / `states.<id>.victory_points_total` (+ per-province via `[static map meta]` `state_meta.json` `victory_points{pid:vp}`) | state/province VP gradient LUT | **exact** | VP is `M/S` (live state_vp) + static geometry. Per-province VP markers come from `state_meta.json`. |
| **Buildings (factories/infra/forts/airbase/…)** | `states.<id>.buildings.{infrastructure,industrial_complex,arms_factory,air_base,…}` (+ `province_buildings` for naval base/forts) | state-recolor level LUT (per selected building) | **exact** | `buildings` is `M/S` (live buildings_live). One sub-mode per building type. |
| **Resources** | `states.<id>.resources.{steel,oil,aluminium,tungsten,chromium,rubber,coal}` | state-recolor deposit LUT | **exact** | Per-state raw deposits are now live-fed. `MapMode::Resources` consumes `states.<id>.resources` directly; measured empty `{}` renders as real 0 and absent data stays grey. Do NOT substitute country-level `economy.resources.produced` (national aggregate != per-state deposit). |
| **Weather (temp / precip / snow)** | `weather.provinces.<pid>.{temp_c,precip,snow}` (M) / `weather.regions.<rid>.{phenom,phenom_name,active}` (M) | province/region weather LUT | **proxy** | §B.9: **exact VALUES, proxy vs the game's exact visual weather mode**. `weather.provinces.*` is `M` (live per-province); `weather.regions.*.temp_c` is a retracted reader (`null`). The recolor reproduces the values, not the game's exact pixel shader → proxy. |
| **Front lines** | explicit `map.front_lines[]` measured geometry, else controller-boundary derivation from province/state control | front-line overlay | **exact / derived / unavailable** | §B.9: measured `map.front_lines[]` geometry ingests as exact when present. Otherwise the native overlay can derive controller-boundary front segments from measured control data. Missing or malformed explicit geometry badges unavailable; this is still not a side-by-side-proven reproduction of the game's battleplan front rendering. |
| **Faction / wars / subjects** | `diplomacy_world.{factions,wars,subjects}` | country-granular faction/war LUT | **exact** (country granularity) | §B.9. `diplomacy_world` populated (75 wars / 4 factions / 5 subjects in sample). Colour by faction membership / belligerent side / overlord. |
| **Air superiority** | `naval_air.air_superiority` (`{strategic_region_id: {controller, …}}`) → parsed strategic-region province join | region-recolor LUT (badged) | **proxy** | §B.9: **strategic-region granularity**. Native `MapMode::Air` now colors joined region provinces from the measured controller TAG where present. Missing join/region, `controller:null`, or unknown country color stays neutral; no province-level superiority is fabricated. `confidence` is never `"exact"`. The optional `naval_air.air_superiority_model.*` is a `D` display ESTIMATE and is not used as a controller fallback. |
| **Naval (sea-zone dominance)** | `naval_air.region_control` (`{strategic_region_id: [{controller,…}]}`) → parsed strategic-region province join + sea-province filter | sea-region-recolor LUT (badged) | **proxy** | §B.9: region only. Native `MapMode::Naval` now colors only joined sea provinces from `region_control[region][0].controller`; land in the same region, missing/unmapped regions, `controller:null`, and unknown colors remain neutral. `naval_air.region_control` is `M`, region-proxy confidence. |
| **Supply** | `provinces.<pid>.supply` when present | province supply LUT (badged) | **proxy** | §B.9: province supply is a proxy and is usually absent because the **exact per-province supply NUMBER is a proven wall** (`replay/SCHEMA.md §13.2`, `memory_reader/supply_solver_offsets.draft.json` — not stored). Native `MapMode::Supply` only colors numeric `provinces.*.supply`; absent data stays grey/unavailable. |
| **Intel / counter-intel** (intel map mode) | `countries.<TAG>.intel.agency.capacity.{defense,usable_slots,max_operatives}` + `countries.<TAG>.intel.{operatives,operatives_live}` | intel-recolor LUT | **derived** | Intel map mode shades owner-country provinces only from measured agency capacity or operative-count signals. Native `MapMode::Intel` badges unavailable when those measured fields are absent, and remains derived because the real game has richer fog-of-war semantics than this country-granularity readout. |
| **(other configurable modes — radar coverage, manpower, etc.)** | the specific driving domain (e.g. `states.<id>.manpower_pool` for a manpower mode) | per-mode LUT | **exact/derived/proxy** by the driving domain | Any configurable mode binds the domain it visualises with that domain's badge. Modes with **no captured domain** (see gap ledger) badge `unavailable`. |

## 6b. Map-mode rail — STATIC / decoration / navigation / toggle (no data binding)

- **The mode-button SLOTS themselves (chrome + selection state, the DATUM is the active map mode, bound in 6a):**
  `mapmode_button_1..12` (each `GFX_mapmode_buttons_deselected_big`/`_small`) + their selected twins
  `mapmode_button_1_s..12_s` (`GFX_mapmode_buttons_selected_*`) + per-button backgrounds `mapmode_button_1_bg`,
  `mapmode_button_2_bg`, `mapmode_button_3_bg`, `mapmode_button_4_bg`, `mapmode_button_4_s_bg`,
  `mapmode_button_5_s_bg` … `mapmode_button_12_s_bg` (`GFX_mapmode_button_bg` / `GFX_mapmode_button_depressed`).
  These exist in BOTH `3_button_mapmode` and `4_button_mapmode` layout containers (the engine shows the variant
  matching the active mode-group's submode count). The button is a **read-only "switch map mode" verb**; the
  selected/deselected sprite swap is **UI state** (which mode is active), not a country field; native now syncs
  that active echo from the seeded `core.map` mode and badges it only in registry-only contexts. Native rail
  hover now emits a factual source/fidelity tooltip for each row; full the game legend tooltip frame/text parity
  remains open. The extracted `.gui` exposes 12 numbered rail slots here; the renderer supports 14 logical
  replay map modes, and native now uses the real `mapmodes_all` button + `WindowWithAllMapModes` popup to
  expose the non-rail Supply/Intel logical modes without pretending they are numbered rail slots.
  Each button's
  `pdx_tooltip_anchor` child anchors the mode's legend tooltip (the tooltip TEXT is the mode's loc — a
  per-mode loc constant, not a datum). `mapmodes_all` (`GFX_mapmode_button_all`, tooltip `MAPMODE_ALL`) toggles
  the all-modes window; custom slot configuration/defaults remain open.
- **Bottom toggle row (live render-state toggles — inert in replay):** `day_night_toggle`
  (`GFX_day_night_toggle`, tooltip `TOGGLE_NIGHT_ENABLED`), `player_counters_toggle`
  (`TOGGLE_PLAYER_COUNTERS_ONLY`), `button_counters_color_mode` (`TOGGLE_UNIT_COUNTER_MODE`),
  `button_allied_plans`, `button_fog` (`TOGGLE_FOG_OF_WAR`), `button_radar` (`TOGGLE_RADAR`), each with its
  `*_bg` (`GFX_mapmode_button_depressed`) and `pdx_tooltip_anchor`. These toggle RENDER state (night shading,
  counter style, fog, radar overlay), not country data — the replay's renderer owns them; inert/UI-state.
- **Social-view widget (live MP social panel — inert in replay):** `social_view_interface_window` (container) +
  `toggle_button` (`GFX_sort_button_83x29`), `button_toggle_socialview_icon` (`GFX_socialview_icon`),
  `friends_online` (instantTextBoxType `0` — Steam-friends count, live MP only), `new_message`
  (`GFX_new_message_icon`, tooltip `SOCIAL_NEW_MESSAGE`), `new_friend` (`GFX_new_friend_icon`,
  `SOCIAL_NEW_FRIEND`). Steam social state is a **hard wall** (SCHEMA §10 — no Steam/player real data) → inert.
- **Find-view launcher + rail chrome:** `open_find_view_button` (`GFX_find_screen_button`, tooltip `SEARCH` —
  opens View 4, read-only verb), `map_modes_bg` (`GFX_map_modes_bg`), `map_modes_bottom_bg`
  (`GFX_map_modes_bottom_bg`), the tutorial highlights/glows `tutorial_highlight_default_mapmode`
  (`GFX_tutorial_default_mapmode_highlight`), `logistics_tutorial_glow_1`/`_2`/`logistics_tutorial_glow`
  (`GFX_logistics_tutorial_glow`) — cosmetic tutorial overlays.
- **Configurable-mapmode list windows (mode PICKER chrome — read-only verbs):** `ConfigurableMapModeSlot` (`bg`
  + `name`), `ConfigurableMapMode` (`bg`, `name`, `shortcut`, `icon_selected`, `icon_deselected`),
  `WindowWithMapModeSlots` (`Background`, `slots` gridbox, `default` button tooltip `CONFIG_MAPMODE_DEFAULT`),
  `WindowWithAllMapModes` (`Background`, `slots`), `WindowWithAllMapModes_Standalone` (`Background`, `slots`).
  The `name`/`shortcut` text of a configurable-mode row is the mode's loc + keybind (UI metadata, not country
  data); the `icon_selected`/`icon_deselected` reflect the active-mode UI state. The `slots` gridboxes are
  adapter-filled with the available modes (a fixed UI catalog, not snapshot data). These are listed static (the
  ACTIVE mode each row would select binds in 6a).
- **Structural containers / layout:** `MapmodesInterface_Ingame` (root), `3_button_mapmode`, `4_button_mapmode`,
  and the `background`/`gridboxtype`/`slotsize` layout nodes inside the configurable windows.

> **Schema-gap (capture work-items) for map modes:** (1) *real the game front-line extraction/styling*
> is not proven (the native renderer now has measured `map.front_lines[]` ingestion plus a derived
> controller-boundary fallback); (2) *exact per-province supply* is a proven wall (supply mode permanently
> `proxy`); (3)
> *air-superiority / naval region control* are region-proxy by nature (permanently `proxy`); (4) any
> configurable mode visualising a domain we don't capture (e.g. *radar coverage* — no per-state radar-range
> field beyond `buildings.radar_station` level) badges `unavailable`.

---
---

# VIEW 7 — ON-MAP ICONS (`game_assets/gui/mapicons.json`)

The largest view: ~36 on-map icon TEMPLATES the **map adapter** instantiates per entity at the entity's map
position — unit counters (land/navy/railway-gun/air), VP/capital/resource/state-modifier counters, building
mapicons (air base / rocket site / gun emplacement / naval HQ / naval base / supply node / railway), combat
mapicons (land/naval), mission mapicons (air/naval), intel mapicons (operations/operatives/cryptology/
counter-intel/ledger), supply/peace/raid/faction/ping/decision/resistance-compliance icons. **None of these are
declarative elements** — every one is a runtime template placed by the adapter at a province/state/unit map
coordinate. The mapping below binds each TEMPLATE's bindable children to the snapshot fields that drive it; the
template's PLACEMENT (where on the map) comes from the entity's `location_province` / state geometry
(`[static map meta]`). Per PLAN §C.2 (combat = roster aggregation) + §G (combat battle-detail HARD LIMIT), the
combat/mission detail values are `unavailable`; the enumeration/identity/strength/org are bound.

> **Observed-country filter.** On-map counters are normally drawn for the player's own + spotted enemy units. In
> a read-only replay observing `<TAG>`, the adapter draws `divisions[]` / `order_of_battle.*` for the observed
> country (and, fog permitting, others). Division identity/org/strength come from `divisions[]` (`M`-rich);
> ship/wing rosters from `order_of_battle.*`.

## 7a. UNIT COUNTERS (land / navy / railway-gun / air) — dynamic (data-bound)

| gui element (name + type, sprite) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `unit_counter.count_txt` (instantTextboxType) | count of `divisions[]` in this stack (filtered by `location_province` + owner) | int | **derived** | The number on the counter = how many divisions are stacked here; the adapter groups `divisions[]` by province. |
| `unit_counter.bar_org` (iconType, `GFX_mapicon_unit_bar_org`) | aggregate `divisions[].organisation` of the stack | progressbar-fraction | **exact** | `organisation` is `M` (live `/100000`). Stack org bar = mean/sum org fraction. |
| `unit_counter.bar_str` (iconType, `GFX_mapicon_unit_bar_str`) | aggregate `divisions[].strength` of the stack | progressbar-fraction | **exact / unavailable** | Exact only when every measured stack strength is a fractional save-style value in `0..=1`. Memory-channel strength can be manpower-scaled, so values outside `0..=1` badge unavailable at the real counter anchor instead of fabricating a progressbar percent. |
| `unit_counter.flag` / `large_flag` (iconType, `GFX_mapicon_unit_flag_stripe` / `_large_flag_stripe`) | the stack owner `divisions[].owner` `<TAG>` | flag-stripe | **exact** | Owner TAG → ideology-striped flag. `large_flag` for the zoomed/HQ counter. |
| `unit_counter.counter_ideology` (iconType, `GFX_onmap_unit_counter_ideology`) | `countries.<owner>.politics.ruling_party` (ideology) | category (ideology→frame) | **derived** | Ideology tint of the counter. |
| `unit_counter.type` (iconType, `GFX_tiled_window_transparent` placeholder) | dominant `divisions[].template_role` of the stack (NATO symbol) | category (role→NATO sprite) | **derived** | The unit-type symbol; `template_role` (`infantry`/`armor`/…) is `S`, resolved via the unit-icon map. |
| `unit_counter.general_portrait` (iconType, `GFX_portrait_GER_oswald_lutz` placeholder) | the commanding general of the stack ← `order_of_battle.armies[].leader` → `commanders.generals[].portrait_gfx` | portrait-sprite | **partial** | Native mapicons now join OOB/division anchors to commander portraits and emit exact art or explicit sprite misses only when a real province anchor exists; unanchored data badges unavailable. Full stack commander selection/visual parity remains open. |
| `unit_counter.formation_insignia` (logical native mapicon sprite entity) | `order_of_battle.armies[]` / `army_groups[]` insignia fields or one-based `icon` frame + OOB/division province anchor | sprite / frame | **exact / derived / unavailable** | Explicit `insignia_gfx`/`emblem_gfx` is exact; one-based army/army-group shield frame is derived; missing/generic/out-of-range art stays an explicit miss. No anchor → unavailable badge, not a province-less shield. |
| `unit_counter.low_supply` (iconType, `GFX_unitcounter_supply_status_strip`) | `divisions[].supply_status` (M*) / `supply.by_province[<pid>]` proxy | category (supply band→frame) | **proxy** | `supply_status` is `M*` (unfed); falls back to the `supply.by_province` division-province PROXY → proxy, never exact (exact per-province supply is a wall). |
| `unit_counter.attrition` (iconType, `GFX_unit_attrition`) | division in attrition terrain (derived from `location_province` terrain + supply) | visibility | **unavailable** | No stored per-division attrition flag (`divisions[]` has no attrition leaf); terrain attrition is a runtime computation. |
| `unit_counter.onmap_digin_icon` (iconType, `GFX_onmap_digin_icon`) | `divisions[].dig_in` (>0) | visibility | **exact** | `dig_in` is `S` (divisions_full). Shown when entrenched. |
| `unit_counter.onmap_training_icon` (iconType, `GFX_onmap_training_icon`) | division training state | visibility | **unavailable** | No stored training-state flag on `divisions[]` (training is a deployment-conveyor state; `military.deployment_queue` is `S` but not per-onmap-division). |
| `unit_counter.hq_badge` / `leader_enroute` (iconType, `GFX_hq_badge_bg` / `GFX_leader_en_route`) | the stack is an HQ (template `is_army_hq`) / leader travelling | visibility | **derived** / **unavailable** | HQ badge derives from `designs.division_templates[].is_army_hq` join; leader-enroute (a moving commander) is not a stored field → unavailable. |

Native status: land `unit_counter.count_txt`, `flag`, `bar_org`, `bar_str`, `type`, and `onmap_digin_icon`
now emit logical entities only at real owned division counter anchors. Count/type are derived groupings; owner,
org, fractional save-style strength, and positive dig-in are exact when the source fields are measured. Missing org,
missing strength, or memory-scaled strength outside `0..=1` badges at the anchored counter; role/dig-in gaps do the
same. Foreign-owner and target-only/no-anchor divisions do not place counter visuals.

| **`unit_counter_navy.*`** — `count_txt` / `capital_count_txt` (`"99-99"`), `bar_org`, `bar_str`, `flag`/`large_flag`, `counter_ideology`, `capital_ship` (`GFX_capital_ship_icon`), `mission_type` (`GFX_onmap_mission_unit_counter_types`), `pride_of_the_fleet_icon` | the fleet/task-force at this sea province ← `order_of_battle.fleets[].task_forces[].ships[]` (count, org, strength, owner) + `countries.<owner>.extra.pride_of_the_fleet` | int / progressbar / flag / category | **exact** (ship count/org/str/owner — where OOB fed) / **unavailable** (mission_type, pride when `extra` unfed) | `order_of_battle.fleets` is `M/S` (live-first); per-ship org/str exact. `capital_count_txt`=capital-vs-screen split. `mission_type` (the task-force's current mission) is a `cached` runtime field (SCHEMA §12, non-authoritative) → unavailable/proxy. `pride_of_the_fleet` needs `extra` (unfed). |
| `unit_counter_navy.spotting_bar_fg` (iconType, `GFX_spotting_bar`) | enemy-fleet spotting progress | progressbar-fraction | **unavailable** | Spotting/detection is a live fog-of-war computation, not stored. |
| `unit_counter_navy.air_base_icon` (buttonType) + `wings_box` (gridbox) → wing rows | carrier air wings ← `order_of_battle.air_wings[]` on this carrier | count | **unavailable** (current data) | `air_wings` is `S` and not folded in the inspected snapshot → "—"; binds when fed. |
| **`unit_counter_railway_gun.*`** — `count_txt`, `bar_str`, `flag`/`large_flag`, `counter_ideology`, `type`, `attrition` | the railway-gun unit ← `divisions[]` (railway-gun template) at this province | int / progressbar / flag | **exact** (count/str/owner) / **derived** (type) | Same as `unit_counter` but no org bar (railway guns have no org bar slot). |
| **`air_wing_counter.*`** — `count_txt`, `bar_str`, `flag`, `wing_type` (`GFX_wing_types_map`), `training`, `no_mission`/`no_region` alerts | the air wing at this air base ← `order_of_battle.air_wings[]` (count=planes, owner, plane type, mission) | int / progressbar / flag / category | **unavailable** (current data) | `air_wings` is `S`, unfed in the inspected snapshot. Would be `exact` for count/owner/plane-type, `derived` for `wing_type`, `unavailable`/`proxy` for the cached `mission`/`no_region` flags. |
| **`GFX_mapicon_tiny_unit_counter.*`** — `icon` (`GFX_unit_transport_icon_medium_white`), `count`, `spotting_bar_fg` | a transport/convoy stack at sea ← `divisions[]` being naval-transported / `military.convoys` | int | **derived** (count) / **unavailable** (spotting) | Transported-unit counter. Count derivable from divisions-in-transit; spotting unavailable. |

## 7b. STATE / TERRITORY counters (VP / capital / resource / state-modifier / construction) — dynamic

| gui element (name + type, sprite/text) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `victory_point_mapicon.text` (instantTextboxType) + `icon` (`GFX_onmap_victorypoints_strip`) | `states.<id>.victory_points_total` / per-province `[static map meta]` `state_meta.json` `victory_points{<pid>:vp}` | int + frame | **exact** | On-map VP number at the VP province. `victory_points_total` is `M/S`; per-province geometry static. |
| `capital_mapicon.text` + `icon` (`GFX_onmap_victorypoints_strip`) | the capital state's VP ← `countries.<TAG>.politics.capital_state` → `states.<id>.victory_points_total` | int + frame | **exact** | Capital marker (capital is a VP location). `capital_state` is `S`. |
| `resource_mapicon_item.desc` + `flag` (`GFX_flag_smallest`) ; `resource_mapicon.resources_list` (listbox) -> items | `states.<id>.resources.{...}` + state owner `<TAG>` | int + flag | **partial exact** | Per-state resource deposits are live-fed. Native map icons currently emit an exact anchored resource-type count for non-empty measured resource blocks, measured empty `{}` emits no icon, and absent data emits no fabricated marker. Full per-resource list/chrome parity remains a visual/detail task. |
| `construction_info_mapicon.construction_info_window.text` + `flag` + `subject_flag` + `infrastructure_effect` (`"+25%"`) | the building under construction at this state ← `countries.<TAG>.economy.construction_queue[]` / `production.construction_queue` (template + location_state) | loc + int + flag | **derived** | Construction-progress label at the build site. `economy.construction_queue` is `S` (location_state + active_factories + produced). `infrastructure_effect` (the infra speed bonus) is a derived modifier. `subject_flag` = the building country if a subject builds in an overlord's state. |
| `construction_info_mapicon.conversion_info_window.*` — `industry_btn`/`arms_btn` (+ `_disabled`), `industry_count`/`arms_count` | factory-conversion counts at this state ← `states.<id>.buildings.{industrial_complex,arms_factory}` (convertible) | int | **derived** | Civ↔mil factory conversion UI; the COUNTS derive from `buildings` levels, the buttons are mutations (inert). |
| `state_modifier_mapicon.modifiers_list` (gridbox) → modifier icons | per-state dynamic modifiers | category (modifier→icon) | **unavailable** | No per-state modifier ARRAY in the schema (`country_extra.dynamic_modifiers` is country-level, not per-state). Capture gap: *no `states.<id>.dynamic_modifiers[]`*. |
| `resistance_compliance_map_icon.text` (`"100%"`) + `modifier_grid_list` → `resistance_compliance_map_icon_modifier_entry.icon` (`GFX_occupation_compliance_modifier_strip`) | `states.<id>.occupation.{resistance,compliance}` (+ the active occupation modifiers) | percent + category | **exact** (resistance/compliance %) / **unavailable** (per-modifier icons) | The on-map resistance/compliance readout. The % is `M/S` (exact); the modifier-effect icons need an occupation-modifier list not stored per-state → unavailable. |

## 7c. BUILDING / FACILITY map-icons (air base / rocket / gun / naval HQ / naval base / supply node / railway / radar) — dynamic

| gui element (name + type, sprite/text) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `air_base_mapicon.wings_count` + `permission_wings_count` (`"100/2000"`) + `wings_box` → wing rows ; `bg_btn`/`bg_strat_btn` | the air base at this state ← `states.<id>.buildings.air_base` (level) + `strategic_air.air_bases[]` (capacity) + `order_of_battle.air_wings[]` (stationed) | int (`used/cap`) | **exact** (level/capacity) / **unavailable** (stationed wings) | `buildings.air_base` is `M/S`; `strategic_air.air_bases[].capacity` is `S`. `wings_count`=stationed planes (needs `air_wings`, unfed → "—"). `permission_wings_count`=volunteer cap (from `strategic_air` per-country capacity). |
| `naval_base_mapicon.txt_capacity` (`"99/99"`) + `box_ships` → ship rows ; `bg_btn`/`bg_strat_btn` | the naval base ← `states.<id>.province_buildings.naval_base` (level) + docked `order_of_battle.fleets[]` | int (`docked/cap`) | **exact** (level) / **unavailable** (docked ships when OOB unfed) | `province_buildings.naval_base` is `S`. Docked-ships list needs `order_of_battle.fleets` (live-first; "—" if not folded). |
| `supply_node_map_icon.supply` (`"20/20"`) + `supply_hub_overlay_*` (icon/motorization/naval) + `under_construction` + `supply_hub_disconnected` | the supply hub ← `states.<id>.province_buildings.supply_node` (level) + `supply.by_province[<pid>]` (current/capacity) + `countries.<TAG>.economy.logistics.node_*` | int (`cur/cap`, badged) | **proxy** | The `20/20` supply readout is the **division-province PROXY** (`supply.by_province` `M`, confidence "division_province_proxy") — the EXACT hub throughput is a **wall**. `node_motorization`/`disrupted`/`foreign_homebase` come from `economy.logistics` (`S`). `under_construction` from the build queue. Hub LEVEL exact; the supply NUMBER proxy. |
| `railway_map_icon.text` (`5`) + `under_construction` + `cooldown` | the railway level on this connection ← `states.<id>.province_buildings.rail_way` / `provinces.<pid>.buildings.rail_way` | int (level) | **exact** | `province_buildings.rail_way` is `S` (+ live `provinces.*.buildings` `M`). The number is the rail level. |
| `rocket_site_mapicon` / `gun_emplacement_mapicon` / `special_facility_mapicon` / `naval_headquarter_mapicon` (`bg_btn`/`bg_strat_btn`, `rockets_box`) | the special facility ← `countries.<TAG>.extra.special_projects.active_programs[]` / `projects.*` (building/province) | presence | **unavailable** (current data) | Special-project facilities (rocket site, mega-gun, naval HQ). `extra.special_projects` (`S`) / `projects` (`S`) carry the building+province but are **not folded in the inspected snapshot** → "—"; binds when fed. The on-map LARGE vs small variants are zoom states. |
| `radar_mapicon` (`GFX_radar_efficiency_icon`) | radar coverage at this state ← `states.<id>.buildings.radar_station` (level) | category (efficiency→frame) | **derived** | Radar level is exact (`buildings.radar_station` `M/S`); the EFFICIENCY/coverage frame is a derived computation over the level + adjacency. |
| `adjacency_rule_mapicon.province_required_rule` (`GFX_strait_canal_onmap_icon`) | a strait/canal adjacency rule (e.g. Kiel/Panama controller) ← `states.<id>.province_buildings.{canal_kiel,canal_panama}` + controller | category | **derived** | Whether the canal is open depends on the controller; presence from `province_buildings`, openness derived from control. |

## 7d. COMBAT / MISSION map-icons (land / naval combat + air/naval missions) — dynamic + unavailable

| gui element (name + type, sprite/text) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `land_combat_mapicon.bg_btn` (`GFX_mapicon_land_combat`) + `mapicon_land_combat_frame` + `battle_indicator_direction` | one icon per active land combat ← `combats[]` (`kind:"land"`, `location_province`) | enumeration | **exact** (the bubble exists) | 13 land combats in the inspected snapshot; one map bubble each at `location_province`. Clicking → land-combat panel (sibling map). |
| `land_combat_mapicon.combat_progress` (instantTextboxType, `"?"`) | `combats[].progress` | percent | **unavailable** | `progress` is a large positive float, **NOT** the displayable bar/percent (sibling combat map, Codex). The literal `"?"` placeholder is honest → `unavailable`. |
| `land_combat_mapicon.air_superiority_icon` (`GFX_battle_modifier_strip`) / `mapicon_land_combat_bombardment` | air support / naval bombardment modifying the battle | category | **unavailable** | Battle-detail modifiers not in `combats[]` (§G HARD LIMIT). |
| `naval_combat_mapicon.bg_btn` (`GFX_mapicon_naval_combat`) + `frame_btn` + `attacker_grid`/`defender_grid`/`attacker_adds_grid`/`defender_adds_grid` (gridboxes) → side counters | one icon per active naval combat ← `combats[]` (`kind:"naval"`); side rosters from `order_of_battle.fleets[]` | enumeration + flags | **unavailable** (current data) | **0 naval combats** in the inspected snapshot + `order_of_battle` ship rosters not folded → "—". Becomes enumeration-`exact` + side-`derived` when a naval battle is active and OOB is fed. |
| `naval_combat_mapicon.progress` / `progress_small` (iconType bars) | `combats[].progress` (naval) | progressbar | **unavailable** | §G + no naval combat + progress-scale mismatch. |
| `naval_combat_results_mapicon.bg_btn` (`GFX_naval_combat_result`) | a recently-resolved naval combat at this sea province ← diff-event `combat_ended` (kind naval) | enumeration | **derived** | Reconstructable from the combat-lifecycle event timeline (SCHEMA §9), not a stored field. |
| `air_mission_mapicon.missions_list` (OverlappingBox) → `air_mission_mapicon_entry` rows (`button` `GFX_strategicair_mission_types`, `count`, `supply_count` `"555/555"`, `alert`) | air missions over this region ← `order_of_battle.air_wings[]` grouped by `mission`/`strategic_region` | int (`assigned`) | **unavailable** (current data) | Air-mission stacks (CAS/AS/strat-bomb/air-supply) per region. Needs `air_wings` (`S`, unfed) → "—". `supply_count` (air-supply throughput) would be a derived/proxy supply number. |
| `naval_mission_mapicon` (content) — `unit_grid`/`enemy_unit_grid`/`enemy_convoy_grid` (gridboxes), `mine_status_container` (`progress_ours`/`progress_enemies`), `ongoing_combat_container.count`, `combat_results_container.count`, `accidents_container.count` ; `naval_mission_mapicon_entry` (`button` `GFX_mapicon_naval_mission_patrol`, `count` `"1/2"`, `efficiency` `"100%"`, `pride`) | naval missions in this sea region ← `order_of_battle.fleets[].task_forces[]` (mission, ships) + `countries.<TAG>.misc.accidents_score` + `naval_air.region_control` | int / percent | **unavailable** (current data) / **proxy** (region) | Naval-mission stacks (patrol/convoy-raid/escort) + mine progress + ongoing-combat/accidents counts. Needs `order_of_battle.fleets` (live-first) for the rosters; mine status + efficiency are runtime fog-of-war → unavailable. `accidents_score` is `M` (live country_misc). |
| `naval_mission_ships` (`naval_missions_grid`) → `naval_mission_ships_entry` (`text`, `pride`, `btn`) ; `intel_ledger_naval_mission_entry` / `intel_ledger_air_mission_entry` (`icon`, `count`) | the expanded fleet roster at a naval-mission stack ← `order_of_battle.fleets[].task_forces[].ships[]` (name) | loc + int | **unavailable** (current data) | Per-ship list rows. Bind from `order_of_battle.fleets` when fed (ship NAME exact); `pride` from `extra.pride_of_the_fleet`. |
| `naval_mines_map_icon` (`icon`, `progress_good`/`progress_bad`) | naval mine density in this sea zone | progressbar | **unavailable** | No per-sea-zone mine-level field in the schema. Capture gap: *no naval-mine domain*. |

## 7e. INTEL map-icons (operations / operatives / cryptology / counter-intel / ledger) — dynamic + unavailable

| gui element (name + type, sprite/text) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `operation_map_icon_entry.*` — `operation_icon` (`GFX_operative_mission_icons_small`), `phase_circular_progress_icon`, `operation_progress_text` (`"100%"`), `operation_completed_overlay`/`_warning` | a running operation targeting this state ← `countries.<TAG>.intel.operations.running[]` (`type`, `date`, `duration`, `target`, `phases[]`) | category + percent | **derived** | `intel.operations` is `S`. Progress derives from `save_date` vs `date`+`duration` (SCHEMA §3.8). Operation TYPE icon exact; the % is a derive (elapsed/duration). |
| `operative_map_icon_entry.*` — `portrait_icn` (`GFX_mapicon_operative_unknown`), `mission_icon` (`GFX_operative_mission_icons`), `mission_progress_text` (`"100%"`), `cartouche_icn`, `mission_status_icn` | an operative on-mission at this state ← `countries.<TAG>.intel.operatives[]` (`state`, `mission`, `op_id`) / `intel.operatives_live[]` | category + percent | **derived** / **unavailable** (portrait) | `intel.operatives` is `S` (+ live `operatives_live` `M`). On-mission state + mission type drive the icon; the operative PORTRAIT (`gfx`) needs asset extraction → `unavailable`. Mission progress derived. |
| `cryptology_map_icon.*` — `large_flag`, `decrytion_bar` (`GFX_phase_circular_progress_pie_chart_on_map`), `percent` (`"10%"`), `state_icon` | active decryption vs a target ← `countries.<TAG>.intel.decryption.targets[]` (`target`, `amount`, `decryption`) / `intel.cryptology_live` | flag + percent | **derived** | `intel.decryption` is `S` (+ live `cryptology_live` `M`: `{TGT:{amount,decrypting}}`). The target flag exact; the % is the accumulated decryption `amount` toward the cipher cost → derived (cost not stored per-target). |
| `counterintelligence_map_icon_entry.rating_txt` (`999`) | the counter-intel rating of this country ← `countries.<TAG>.intel.agency.capacity.defense` | int | **exact** | `intel.agency.capacity.defense` is `M/S`. |
| `intel_ledger_map_icon.civilian_ledger_view.*` — `civilian_factory_count`/`military_factory_count`/`dockyard_count`/`infra_count`/`refinery_count`/`silo_count` (all `999`) | the intel-ESTIMATE of a target's industry ← target `countries.<TAG>.economy.factories.{civ,mil,dockyard}` + `buildings` aggregates | int | **unavailable** | These are **fog-of-war intel ESTIMATES** (intel-level-scaled obfuscation of the target's real values). We store the REAL `economy.factories.*` but not the intel-scaled estimate → showing real values misrepresents the intel view; badge `unavailable` (consistent with the diplomacy `estimated_*` precedent). |
| `intel_ledger_map_icon.air_and_navy_ledger_view.missions_grid` → mission rows | intel estimate of the target's air/naval missions | int | **unavailable** | Same intel-estimate gap. |
| `intel_map_icon_more.more_icon` / `intel_map_mode_map_icon.entries` (OverlappingBox) | "more" expander / the intel-icon stack container | — | static/derived | The expander is UI; `entries` is adapter-filled with the operation/operative/crypto icons above. |

## 7f. DECISION / RAID / FACTION / PEACE / PING map-icons — dynamic + unavailable

| gui element (name + type, sprite/text) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `decision_mapicon_item.*` — `decision_icon` (`GFX_decision_unknown`), `progress_good`/`progress_bad`, `cost_text`, `target_flag`, `timer_icon` ; `decisions_gridbox` (in `decision_mapicon_container`) | a targeted decision available at this state ← `countries.<TAG>.decisions.{active,timed}[]` (+ target state) | category + int | **unavailable** (current data) | `decisions` is `S`. The DECISION-ON-MAP requires the decision's target-state geometry (the decision's `target_array`/`state` scope), which the snapshot `decisions` domain does not carry per-target → "—". `cost_text` (PP cost) is decision-DEF data. Becomes `derived` if a decision-target adapter is wired. |
| `raid_map_icon.*` — `raid_map_icon_button` (`GFX_raid_target_icon_generic`), `raid_map_icon_frame`, `plus_icon` | a strategic-bombing/naval-raid target here ← `raids.targets[]` (`target_province`, `target_building`, `valid`) | category | **derived** | `raids.targets` is `S` (target_province + target_building). The icon marks the target province; `plus_icon` = multiple targets here. Whether it renders for the observed country uses `raids.countries[].priority`. |
| `faction_ping_map_icon.*` — `faction_icon`, `count` (`99`), `button` (`GFX_mapicon_faction_theatre`) | a faction theatre/rally ping ← `diplomacy_world.factions[]` (members at a theatre) | int | **derived** / **unavailable** | Faction theatre marker. `diplomacy_world.factions` is `S` (members). The COUNT of members staged at a theatre is a derive; the theatre LOCATION needs `order_of_battle.theatres`/`navy_theaters` (partly fed). |
| `peace_mapicon.*` — `negotiator_flags` (OverlappingBox), `cost` (`9999`), `resources_grid` → `peace_resource_item` (`resource_icon` `GFX_resources_strip`, `amount`) ; `peace_cost_mapicon.cost` | a peace-conference claim on this state ← `countries.<TAG>.intel.peace_conference` / a peace-conference domain | flag + int | **unavailable** (current data) | Peace-conference on-map cost/negotiator markers. `intel.peace_conference` is `S` but `{active}` only (empty/inactive in most saves). The per-state claim COST + contested resources are peace-conference runtime state not modelled → "—". |
| `ping_mapicon.*` — `circle`, `ping_mapicon_offensive`, `ping_mapicon_defensive` | a player map-ping at this location | — | **unavailable** | Map-pings are a live MP verb with no persisted state (SCHEMA §10) → inert/unavailable (same as the minimap pings). |
| `supply_mapicon.*` — `supported` / `infra_wnd.{value,capacity,air_mission_icon,add_btn}` / `port_wnd.{value,capacity}` ; `bg_btn`/`bg_red_btn` | the supply situation at this hub ← `supply.by_province[<pid>]` (current/capacity) + `economy.logistics` (infra/port motorization) | int (badged) | **proxy** | The big supply mapicon (supported-divisions + infra + port throughput). All `supply.by_province` PROXY values (the exact numbers are a wall). `add_btn` (build infra) is a mutation → inert. `bg_red_btn` = under-supplied (a derived band). |

## 7g. On-map icons — STATIC / decoration / cosmetic / mutation / structural (no data binding)

- **Counter chrome / frames / selection / cosmetic overlays:** every `count_bg` (`GFX_onmap_unit_counter` /
  `_naval_unit_counter`), `counter_overlay` (`GFX_onmap_unit_counter_overlay`/`_navy`), `count_selected`
  (`GFX_onmap_unit_counter_selected` — selection UI state), `group_marker` (`GFX_UV_group_marker` /
  `GFX_group_marker_rhs_9x32`), `active_ability` (`GFX_onmap_unit_counter_active_ability`), `no_orders` /
  `invalid_orders` / `defending_area` (`GFX_mapicon_unit_no_orders` / `_invalid_orders` / `_area_defense` —
  order-state overlays, a live battle-plan computation, not stored → unbindable cosmetic), `onmap_action_icon`
  (`GFX_onmap_action_icon`/`_air_action_icon`), `training`/`training_will_stop_at_max_icon`/`no_mission`/
  `no_region` (air-wing order overlays), `spotting_bar_bg` (`GFX_spotting_bar_bg`/`_small`), the
  `spotting_bar_container`/`spotting_bar_fg_container` structural windows.
  The native replay also emits anchored logical land-counter entities for `count_txt`, `flag`, `bar_org`,
  `bar_str`, `type`, `onmap_digin_icon`, and `formation_insignia`; that does not close extraction/render parity
  for the full GUI counter chrome listed here.
- **Building/facility frames + mutation buttons (inert):** `bg_btn`/`bg_strat_btn`/`background_btn` frames; the
  supply-node interaction buttons `move_capital_supply_node`, `toggle_enable_node_for_allies`,
  `upgrade_railways_to_capital`, `set_hub_motorization_priority` + `add_btn` (all supply MUTATIONS → inert);
  `industry_btn`/`arms_btn` (+ `_disabled`) factory-conversion mutations; `allied_setting_overlay`,
  `supply_textbox`/`cap_bg`/`cost_bg` chrome, `under_construction`/`cooldown` overlays (visibility from the
  build queue, listed with their value rows above), `current_mission_bg`/`efficiency_background`/`pride`/
  `frame_icon_small`/`frame_btn`/`frame_select_hl_icn`/`frame_warn_hl_icn`/`cartouche_icn`/`flag_frame`/
  `shield_icn` intel/mission frames, `more_ground_crews_bg`/`_icon`.
- **Combat frames / direction / bombardment overlays:** `mapicon_land_combat_frame`,
  `battle_indicator_direction`, `mapicon_land_combat_bombardment`, `naval_hq_combat_frame`,
  `progress_small`/`progress` frames (the bars themselves are `unavailable` in 7d), the naval
  `mine_status_container.bg`, `ongoing_combat_container.bg`/`combat_results_container.bg`/`accidents_container.bg`.
- **Backgrounds:** all `Background` / `bg` / `mini_bg` (`GFX_mini_bg`, `GFX_construction_province_background`,
  `GFX_tiled_window_transparent`, `GFX_mapicon_air_mission_bg`, `GFX_naval_mine_icon_bg`, etc.),
  `cost_bg`, `permission_wings_bg` (`GFX_generic_box_smallest`, tooltip `AIR_VOLUNTEERS_LIMIT`).
- **Mutation buttons (inert read-only/verb):** `select_all` (`SELECT_ALL` — selects a fleet, mutation/UI),
  `add_btn` (build), the supply-hub interaction set (above).
- **Positioning anchors (positionType, no visual):** `unit_counter_zoomed_in_offset`,
  `unit_counter_hq_x_offset`, `cost_text_horizontal_margin` — adapter layout anchors; no datum.
- **Structural container / template roots (placement-only, their bound children are in 7a–7f):**
  `mapicons_container` (root), `unit_mapicon`/`units_box`, `scripted_map_icon_container`,
  `strategic_location_mapicon` (`bg_btn` `GFX_NV_icon_pride` — a generic placeholder), and every template's
  outer container + its gridbox/listbox/overlapping-box hosts (`wings_box`, `box_ships`, `rockets_box`,
  `units_box`, `missions_list`, `resources_grid`, `modifiers_list`, `modifier_grid_list`, `decisions_gridbox`,
  `naval_missions_grid`, `missions_grid`, `resources_list`, `entries`) — the GRID HOSTS are adapter-filled; the
  rows they host are the bound entries above.

> **Schema-gap (capture work-items) surfaced by the on-map icons:** (1) *no per-division order-state*
> (no_orders / invalid_orders / defending_area / training / hq are battle-plan/deployment runtime state, not
> stored); (2) *`order_of_battle.air_wings` + `navy_theaters`* save-sourced but not folded in the inspected
> snapshot (air-wing + carrier + air-mission counters `unavailable` until fed); (3) *`extra.special_projects` /
> `projects`* not folded (rocket/gun/naval-HQ facility icons `unavailable`); (4) *no per-state
> `dynamic_modifiers[]`* (state-modifier on-map icons `unavailable`); (5) *no naval-mine domain* (mine icons
> `unavailable`); (6) *decision-target geometry* not carried in `decisions` (on-map decision icons
> `unavailable`); (7) *intel-scaled industry ESTIMATE* not modelled (intel-ledger on-map counts `unavailable` —
> real values would misrepresent fog-of-war); (8) *spotting/detection progress* is live fog-of-war (spotting
> bars `unavailable`); (9) *exact per-province supply* is a proven wall (all supply on-map values `proxy`).

---
---

## Element accounting (provably complete)

Each view's `stats.element_type_counts` (from the parsed JSON) gives the authoritative total; every element is
placed in exactly one bucket by the per-element tables above.

### 1 — IN-GAME CLOCK (`ingame_clock.json`) — type counts: containerWindowType 1, positionType 2, background 2, instantTextBoxType 1 (= **6** elements)

| bucket | count | elements |
|---|---:|---|
| **Dynamic-bound** | **1** | `time_text` |
| **Static / decoration / positioning** | **5** | `clock_window` (root), `background`, `time_box`, `peace_conference_offset`, `minimap_offset` |

### 2 — MINIMAP (`minimap.json`) — type counts: iconType 12, containerWindowType 2, buttonType 4 (= **18** elements)

| bucket | count | elements |
|---|---:|---|
| **Dynamic-bound** (derived raster) | **1** | `worldmap_bg` |
| **Static / camera-state / live-verb** | **17** | 2 ping icons, 7 camera-quad icons, `worldmap_border`, `circle`, `offensive_ping`/`defensive_ping`/`handle`/`handle_pinged`, `mini_map_interface`/`border_container` |

### 3 — ALERTS (`alerts.json`) — type counts: containerWindowType 2, buttonType 2, iconType 3, positionType 4 (= **11** elements)

| bucket | count | elements |
|---|---:|---|
| **Dynamic / derivable** | **2** | `global_diplorequesticon_window.country_flag` (derived), `global_alerticon_window.icon` (unavailable badge) |
| **Deferred: needs adapter** | **1 logical** (window + glows) | `global_alerticon_window` (+ `red_alert_glow`/`yellow_alert_glow`) |
| **Static / positioning** | **6** | the 2 container roots, 2 glows (listed deferred), `global_diplorequesticon_window.icon`, 4 position anchors |

(Counts overlap by design: `global_alerticon_window` window + its `icon`/glows are enumerated once each across
3a/3b/3c; the 11 named elements are each placed.)

### 4 — FIND VIEW (`find_view.json`) — type counts: containerWindowType 3, background 3, instantTextboxType 1, iconType 2, editBoxType 1, margin 1, gridboxtype 1, slotsize 1, buttonType 3 (= **16** elements)

| bucket | count | elements |
|---|---:|---|
| **Dynamic / derivable** | **2** | `results_grid_box` (row factory), `find_view_item.name` |
| **Static / decoration / nav / UI-input** | **14** | 2 roots + `results_window`, 3 backgrounds, `find_editbox_bg`, `GFX_find_screen_bottom`, `to_find_text` (input), `find_button`/`goto_home_button`/`back_button`, `margin`, `slotsize` |

### 5 — RIGHT-CLICK MENU (`rightclick_menu.json`) — type counts: windowType 4, guiButtonType 3, listboxType 1, borderSize 1 (= **9** elements)

| bucket | count | elements |
|---|---:|---|
| **Dynamic / derivable** (label only) | **2** | `right_click_entry.button`, `right_click_entry_2.button`+`open_button` (logical 2 rows) |
| **Deferred: mutation context-menu** | **1 logical** | `rightClickMenu`+`options` + 3 template roots |
| **Static / decoration** | balance | `empty_right_click_entry`, `options` sprite, `borderSize` |

(9 named elements: 4 windowType roots, 3 guiButtonType, 1 listbox, 1 borderSize — each placed across 5a/5b/5c.)

### 6 — MAP-MODE SWITCHER (`mapmodes.json`) — type counts: containerWindowType 9, buttonType 58, iconType 43, instantTextBoxType 4, pdx_tooltip_anchor 55, background 3, gridboxtype 3, slotsize 3 (= **178** elements)

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** (per-MAP-MODE recolor, not per-button) | **14 logical modes; 12 real numbered rail slots + all-modes popup access proven** | political/control/resistance/compliance/VP/buildings/resources/weather/front-lines/faction/air/naval are backed by the extracted numbered `mapmode_button_*` rail slots; supply/intel exist as native logical modes and are reached through the real `mapmodes_all` / `WindowWithAllMapModes` path |
| **Static / decoration / navigation / toggle** | **~164** | the 12 mode-button slots ×(deselect+select+bg) ≈ 50 buttons + 43 icons (bgs/glows/tutorial), 55 `pdx_tooltip_anchor`, the 6 bottom toggles, social-view widget (6), `open_find_view_button`, configurable-mode windows (3 + their bg/grid/slot/name/shortcut/icons), 9 containers, `mapmodes_all` |

(The 58 buttonType = 12 mode slots + 12 selected twins + 6 small-mode pairs already counted + 6 bottom toggles +
social toggle + find button + configurable `bg`/`default` buttons; all are "switch mode / toggle render-state"
read-only verbs whose DATUM is the active map mode, bound once in 6a.)

### 7 — ON-MAP ICONS (`mapicons.json`) — type counts: containerWindowType 76, positionType 3, buttonType 47, iconType 155, instantTextboxType 42, gridboxtype 9, slotsize 18, listboxType 1, borderSize 3, background 7, gridBoxType 9, OverlappingElementsBoxType 3, overlappingElementsBoxType 1, instantTextBoxType 9 (= **383** elements)

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** (incl. derived/proxy/unavailable badges) | **~70** | unit/navy/railway/air counters (count/org/str/flag/ideology/type/dig-in/supply ≈ 25), VP/capital/resource/construction/state-modifier/resistance-compliance (≈ 12), building/facility icons (air-base/naval-base/supply-node/railway/rocket/gun/radar/canal ≈ 15), combat/mission icons (land/naval/air/naval-mission/results ≈ 12), intel icons (operation/operative/crypto/counterintel/ledger ≈ 12), decision/raid/faction/peace/ping/supply (≈ 8) |
| **Static / decoration / cosmetic / mutation / structural** | **~313** | all `count_bg`/`counter_overlay`/`count_selected`/`group_marker`/order-state overlays, frames, mine/combat/intel/mission chrome, supply-hub mutation buttons, backgrounds, position anchors, the ~36 template-container roots + their gridbox/listbox/overlapping-box HOSTS |

Every element in each of the seven `.gui` trees falls into exactly one bucket; the per-element tables
(1a/1b, 2a/2b, 3a/3b/3c, 4a/4b, 5a/5b/5c, 6a/6b, 7a–7g) enumerate them by name.

---

## `unavailable` / `proxy` summary (with reason)

**`unavailable` (schema-present-but-unfed, or no field):**
- **Alerts:** `global_alerticon_window.icon` type/glow (no `alerts[]` ledger — alerts recomputed live, never
  persisted).
- **Map modes:** Front-lines (`map.front_lines` `D`, compute
  pass not run), any configurable mode over an uncaptured domain (e.g. radar coverage).
- **On-map icons:** general portrait + leader-enroute (`commanders` unfed + portrait asset step); attrition /
  training / order-state overlays (no per-division order/attrition/training field); air-wing + carrier +
  air-mission counters (`order_of_battle.air_wings`/`navy_theaters` unfed); rocket/gun/naval-HQ facility icons
  (`extra.special_projects`/`projects` unfed); state-modifier icons (no per-state `dynamic_modifiers[]`); naval
  mines (no mine domain); on-map decision icons (no decision-target geometry in `decisions`); intel-ledger
  industry counts (intel-scaled ESTIMATE not modelled — real values would misrepresent fog-of-war);
  spotting bars (live fog-of-war); naval-combat icon (0 naval combats + OOB ship rosters unfed); `combat_progress`
  text + combat progress bars (`combats[].progress` is not the displayable bar — §G HARD LIMIT);
  peace-conference on-map markers (only `{active}` stored).

**`proxy` (region/division-province PROXY by nature — never `exact`):**
- **Map modes:** Air superiority + Naval (visible strategic-region proxy colouring from `naval_air.*`,
  still not province-exact), Supply (province
  proxy, `provinces.*.supply` when present; exact per-province supply is a **proven wall**), Weather (exact values, proxy vs
  the game's exact visual mode).
- **On-map icons:** every supply value (`supply_node_map_icon.supply`, `supply_mapicon.*`,
  `unit_counter.low_supply`, air/naval mission supply counts) — all the `supply.by_province` PROXY.

**`derived` (computed / joined, but honest):** minimap raster, unit-stack counts/type/ideology, construction
labels, radar efficiency, raid markers, faction pings, naval-combat-results (event-reconstructed), intel
operation/operative/crypto progress, find-results, right-click labels, alert diplo-flag.

**Total dynamic-bound across the 7 views: ~90 elements/modes** (1 clock + 1 minimap + 2 alerts + 2 find + 2
right-click + ~14 map modes + ~70 on-map icon bindings). None fabricated; every `unavailable`/`proxy` renders
"—"/a badged estimate per no-fabrication.

---

## Schema-gap ledger (the required-capture backlog)

Every snapshot field these seven views need that the schema does **not** provide (or does not feed in the
inspected snapshot family). Each is a capture work-item, never a silent hole.

- **`alerts[]` / per-country active-alert ledger** — alerts are recomputed live per player and never persisted
  (not in save, not pinned in memory). Needed for the alerts strip badge type + severity glow. *No schema field
  exists; would need a live alerts adapter or an alerts memory crack.*
- **Front-line geometry** — the native renderer now has a derived controller-boundary fallback and exact
  `map.front_lines[]` measured-geometry ingestion path. Product parity still needs real the game front extraction
  coverage and side-by-side styling proof.
- **Exact per-province supply NUMBER** — a **proven wall** (`replay/SCHEMA.md §13.2`, `memory_reader/supply_solver_offsets.draft.json`; transient routing-solver
  output, not stored). Forces supply icons (`supply.by_province`) and the native supply map mode
  (`provinces.*.supply` when present) to remain proxy/badged rather than exact the game tooltip values. *Hard
  limit, not a TODO.*
- **`naval_air.{air_superiority,region_control}` are region-proxy** by nature (strategic-region granularity).
  Native Air/Naval modes now draw the joined proxy regions where controller data exists, but still force
  the modes to `proxy`; missing/null/unmapped regions stay neutral. *Hard limit.*
- **`weather.regions.<rid>.temp_c` is a retracted reader (`null`)** — the per-region temperature is unpinned;
  per-province `weather.provinces.*` is `M` (fed) but the weather map mode is `proxy` vs the game's exact visual.
- **`commanders` domain not folded in the inspected snapshot** — needed for on-map general portraits + combat
  leaders. Schema-present (`S`), parser-unfed THIS snapshot.
- **`order_of_battle.air_wings` + `navy_theaters` not folded** — needed for air-wing / carrier-wing / air-mission
  on-map counters. Schema-present (`S`), unfed THIS snapshot. (`armies`/`fleets`/`ships` ARE live-first.)
- **`countries.<TAG>.extra.special_projects` + `countries.<TAG>.projects` not folded** — needed for rocket-site /
  gun-emplacement / naval-HQ / special-facility on-map icons. Schema-present (`S`), unfed THIS snapshot.
- **No per-state `dynamic_modifiers[]`** — `country_extra.dynamic_modifiers` is country-level only; the
  `state_modifier_mapicon` + `state_view_dynamic_modifier_entry` need a per-state modifier array. *No schema
  field exists.*
- **No naval-mine domain** — `naval_mines_map_icon` / `mine_status_container` need per-sea-zone mine levels.
  *No schema field exists.*
- **Decision-target geometry not carried in `countries.<TAG>.decisions`** — on-map `decision_mapicon` needs each
  decision's target state/province scope; `decisions` carries `{active,timed,missions}` without per-target map
  geometry. *Parser/decision-DEF gap.*
- **Intel-scaled industry/force ESTIMATE not modelled** — `intel_ledger_map_icon` (and the diplomacy
  `estimated_*` readouts) need the intel-level-obfuscated estimate of a target's values; we store the REAL
  values only. *By design — showing real values misrepresents fog-of-war; a separate estimate model is the
  work-item.*
- **Spotting / detection progress not stored** — `spotting_bar` (unit/navy counters) is live fog-of-war.
  *No schema field; likely a memory crack.*
- **Per-division order-state / attrition / training flags not stored** — `no_orders`/`invalid_orders`/
  `defending_area`/`attrition`/`onmap_training_icon`/`leader_enroute` are battle-plan/deployment runtime state.
  *No `divisions[].order_state`; combat/battle-plan-detail HARD LIMIT (§G).*
- **`combats[].progress` is not the displayable −1..1 bar** (large positive float, M\*-uncracked) — forces
  `land_combat_mapicon.combat_progress` + naval combat progress bars to `unavailable` (§G battle-detail wall).
- **No per-state radar COVERAGE** beyond `buildings.radar_station` LEVEL — `radar_mapicon` efficiency frame is a
  derive over level+adjacency, not a stored coverage field.
- **Steam social state (friends online / messages)** — `social_view_interface_window` needs Steam data; a
  **hard wall** (SCHEMA §10 — no Steam/player real data).

---

## Historical review round (adversarial review, 2026-06-26)

Historical note: this records a completed adversarial review from the old
Claude-era workflow. Codex-native sessions must use normal sub-agents/partner
review and must not run the old local CLI command. Two earlier attempts timed out
while running an element-name cross-check of every `.gui` against the doc; a
third, tighter attempt restricted to the seven highest-risk binding claims
returned cleanly. **The review asked to refute three things:** (a) any
`.gui` element MISSED (cross-check each view's `stats.element_type_counts`); (b) field-path bindings + M/S/D
source tags vs `replay/schema.py` `FIELD_SOURCES`; (c) any "unavailable" badge that is actually a fed field.

**Codex verdict — all 7 verified claims OK, no field-path or fidelity refutations.** Each confirmed against a
cited `schema.py` line:

| claim Codex checked | verdict | `schema.py` citation |
|---|---|---|
| `states.*.resources` live-fed -> Resources map mode exact; `resource_mapicon` partially exact | **UPDATED** | `states.*.resources` now closes the Resources map mode; native map icons emit exact anchored resource-count markers for non-empty measured blocks. |
| `map.front_lines` = `D` plus derived controller-boundary fallback | **OK with implementation update** | line 721 (`D`, null until computed). Native overlay now renders exact measured `map.front_lines[]` geometry when fed, or derived controller-boundary segments from measured control data; real the game extraction/styling proof remains open. |
| `order_of_battle.fleets` = `M/S` + `ships` = `M` (live-first → bind) vs `air_wings` = `S` (save-only → unfed/`unavailable`) | **OK** | lines 503 / 509 / 506 — the live-first-vs-save-unfed split is correct |
| `divisions.*.organisation` = `M`, `.strength` = `M/S`; `.dig_in` = `S` | **OK with channel caveat** | lines 589 / 590 / 607. Org bars are exact; strength bars are exact only for fractional save-style values. Memory-scaled strength is not converted into a fake progressbar percent. |
| `supply.by_province` = `M`; exact per-province supply is a **wall** → supply icons proxy from `supply.by_province`; the native supply map mode proxies `provinces.*.supply` when present | **OK with implementation update** | line 630 + solver-input note 631-636 + §13.2 wall 1382-1385 |
| `intel.agency.capacity.defense` = `M/S` → counter-intel rating `exact` | **OK** | line 366 |
| binding intel-ledger industry counts to real `economy.factories` is **WRONG** → must be `unavailable` (intel ESTIMATE) | **OK (refute-the-alternative)** | real factories `S` 194-196; `intel_ledger` has only derived confidence/cipher fields 681-687, no intel-scaled industry counts |

**Self-caught fix (independent of Codex, during my own `stats` cross-check for (a)):** the on-map-icons
element-count total was mis-added as **373**; the parsed `mapicons.json` `stats` sums to **383**
(76+3+47+155+42+9+18+1+3+7+9+3+1+9). Corrected the total + the static-bucket count (303→313). The other six view
totals (6/18/11/16/9/178) match the parsed `stats` exactly.

**Net:** Codex confirmed every high-risk binding path + fidelity badge against `FIELD_SOURCES` with no
refutation; the one error (mapicons 383, not 373) was caught and fixed via my own exhaustiveness check. All
node/template bindings use exact `FIELD_SOURCES` dotted paths; every `unavailable`/`proxy` is either a named
schema gap (ledger above) or a §B.9/§G/§10 hard limit. The front-lines exact-ingestion/derived-fallback split, the
supply level-exact/number-proxy split, and the live-first (`fleets`/`ships`) vs save-unfed (`air_wings`/
`commanders`) OOB distinction are all corroborated.
