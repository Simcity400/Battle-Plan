# BINDING MAP — LOGISTICS · NAVAL-COMBAT-RESULTS · UNITS · EVENTS / POPUPS

> **What this is.** The COMPLETE, no-truncation data-binding map for **twelve** in-game the game views the
> native replay UI binds in the Phase-3 sweep, covering the **logistics / supply**, **naval combat-result**,
> **unit / division detail**, and **events / popups / notifications** clusters of GAME_UI_REPLICA_PLAN §C.4:
>
> 1. `game_assets/gui/logistics.json` — `interface/countrylogisticsview.gui`
> 2. `game_assets/gui/garrison_log.json` — `interface/garrisonlogview.gui`
> 3. `game_assets/gui/naval_combat_results.json` — `interface/navalcombatresultswindow.gui`
> 4. `game_assets/gui/destroyed_ships.json` — `interface/destroyedshipswindow.gui` **(present-but-empty; documented in full)**
> 4b. `game_assets/gui/naval_losses.json` — `interface/naval_losses.gui`
> 5. `game_assets/gui/naval_strategic_region.json` — `interface/naval_strategic_region_view.gui`
> 6. `game_assets/gui/unit_view.json` — `interface/unitview.gui` (the big division/ship detail panel)
> 7. `game_assets/gui/event_window.json` — `interface/eventwindow.gui`
> 8. `game_assets/gui/popup_window.json` — `interface/popupwindow.gui` (the 32-type generic popup library)
> 9. `game_assets/gui/detailed_windows.json` — `interface/detailedwindows.gui`
> 10. `game_assets/gui/core.json` — `interface/core.gui` (shared prototype chrome library)
> 11. `game_assets/gui/ingame_menu.json` — `interface/ingamemenu.gui` (pause / escape menu)
>
> The land/naval/combat-log **battle** views (`landcombat`, `navalcombat`, `combatlogview`) are mapped in
> **BINDING_MAP_diplomacy_combat.md** — NOT re-mapped here. `naval_combat_results` / `destroyed_ships` /
> `naval_losses` are the **combat-RESULT siblings** of that map; they share its hard limits (battle-detail
> `unavailable` §G) and its roster-aggregation model, and they bind to the same combat-loss schema sources
> (`world_events.sunk_convoys`, `convoy_losses_by_*`, `military.ships_by_hull`, `military.casualties`).
>
> **Binding-rule shape** (PLAN §C.1): `{ target (gui element name+type, sprite/text, loc key), source
> (exact SCHEMA dotted field path), format (formatter id), fidelity (exact|derived|proxy|unavailable) }`.
> EVERY element of each `.gui` is accounted for in one of three buckets: **dynamic-bound** (data-driven),
> **static/decoration** (no datum), or **deferred-needs-adapter** (mutation verb / runtime-instanced shell).
> `gridBox`/`gridboxtype`/`OverlappingElementsBoxType` are **ROW FACTORIES** — the per-row template
> (a sibling top-level `containerWindowType`) is mapped ONCE and the mapping repeats per row.
>
> **No-fabrication (SCHEMA §0.2):** `null` → "—", never `0`. A genuine `0`/`0.0` is a measured zero. A
> schema-present field the current parser never feeds is badged `unavailable` (not silently dropped).

---

## Provenance of the fidelity calls — what the DEMO snapshot actually carries

Grounded in **empirical inspection of the one shipped snapshot**
(`replay/snapshots/replay/000017005619.snapshot.json`, `schema_version "1"`, date `1941.4.13.11`, GER 1941
channel) cross-checked against `replay/SCHEMA.md` / `replay/schema.py` §3.1/§3.2/§6/§13/§17/§9.

**This snapshot is SAVE-ONLY** — its top-level keys are exactly
`[combats, countries, date, diplomacy_world, divisions, events, game_id, hour_index, map, provenance,
provinces, save_version, schema_version, states]`. The live / sidecar domains relevant to THIS map —
`supply`, `naval_air`, `world_events` — are **not even present as keys** (not `null`-present; absent),
and `provenance` is `null`. This is the **FOLD-IN artifact** described in the task: the demo was built
from a melted save without the live PROXY plug or the `events.json` / `supply.json` domains folded.

| schema field | state in the inspected snapshot | consequence for the badges below |
|---|---|---|
| `countries.<TAG>.economy.logistics` (supply_system_2) | **0 / 143 countries** populated. `economy` union of keys = `{factories, fuel, resources, construction_queue, gross_ic_by_equipment, _production_lines}` only | logistics network readouts → bind paths are correct & schema-present, but **unfed in this demo (ARTIFACT)**. Closure: fold `supply.json`. |
| top-level `supply.by_province` | **key absent** | per-province supply PROXY → unfed ARTIFACT; AND carries a PERMANENT PROXY badge (the exact tooltip supply NUMBER is a documented WALL — see `replay/SCHEMA.md §13.2`, `memory_reader/supply_solver_offsets.draft.json`, and `RESEARCH_logistics_intel.md`). |
| top-level `naval_air.{region_control,air_superiority}` | **key absent** | sea-zone dominance / air-superiority PROXY → unfed ARTIFACT; PERMANENT PROXY/region-granularity badge. |
| top-level `world_events.*` (`fired_events`, `sunk_convoys`, `convoy_losses_by_*`) | **key absent** | event-fired ids + convoy-loss ledgers → unfed ARTIFACT. Closure: fold `events.json`. |
| `snapshot.events[]` | **`[]`** (0) | marker-event channel empty (expected; mod log not wired). |
| `divisions[]` | **1654 entries**, MEMORY-channel body present: `{div_id, div_type, owner, location_province, state_id, template_id, template_type, template_name, template_role, organisation, strength, equipment[], experience(null), moving, move_target_province, move_progress, in_combat, encircled, supply_status(null)}` | unit_view division rows → org/strength/equipment/template **EXACT**; the **divisions_full save body** (`dig_in`, `max_supply`, `army_manpower`, `acclimatization`, `killed`, `division_name`, `motorization_level`, …) is **absent** → those cells `unavailable` (current data; closure = fold `divisions_full.json`). |
| `countries.<TAG>.military.ships_by_hull` | **0 / 143** (null) | ship-archetype breakdown (detailed_windows ships) → `unavailable` (current data). |
| `countries.<TAG>.military.casualties.{killed,captured}` | **0 / 143** (null) | garrison/manpower-loss totals → `unavailable` (current data). |
| `combats[]` | **13 entries, all `kind:"land"`, 0 naval** | naval combat-RESULT views have no live engagement to bind in this demo (and battle-detail is a hard limit regardless — §G). |

> **Two distinct causes are folded under `unavailable` and differ in prognosis** (SCHEMA §10 vs parser-unfed):
> **(a) ARTIFACT / parser-unfed** — becomes `exact`/`proxy` once its domain is folded (`logistics`, `supply`,
> `naval_air`, `world_events`, `divisions_full`, `ships_by_hull`, `casualties`). **(b) TRUE GAP** — stays
> `unavailable` by nature (combat battle-detail §G; the EXACT per-province supply NUMBER §B2 wall; per-battle
> casualty ledger; event TITLE/DESC/OPTION text which needs an event-file parse + loc). Both render "—".

**Read-only verb note (PLAN §D).** Every `*_button` / `accept_button` / `decline_button` / `send_button`
whose in-game role is to PERFORM an action (lend-lease, release country, accept diplomacy, resync OOS,
save/load, resign) is a **mutation** verb → rendered as real chrome but **inert** (no datum, no action) in
a replay. These are listed under each view's static/deferred section, never bound.

**Formatters used:** `flag`/`flag-small`/`flag-shield`/`flag-smallest`, `int`, `signed-int`, `float`,
`percent` (×100), `bignum` (`2.44K`), `loc` (localisation key → text), `clock`/`date` (Clausewitz date),
`progressbar-fraction` (0..1), `category` (token → loc + sprite), `equipment-icon` (archetype → `GFX_*`),
`unit-icon` (template_role → sprite).

---
---

# VIEW 1 — LOGISTICS (`game_assets/gui/logistics.json`)

Source `interface/countrylogisticsview.gui`. Three top-level windows: `countrylogisticsview` (the main
materiel + fuel-prioritization + factory-usage panel), `logistics_info_window` (the per-equipment
production line-chart detail), `logistics_fuel_window` (the fuel line-chart detail), plus **8 runtime ROW
templates** (`logistics_overview_*_equipment_entry`, `logistics_overview_*_other_entry`,
`logistics_overview_resource_item`, `logistics_entry_resource_item`, `logistics_info_fuel_line_entry`,
`logistics_info_equipment_entry`).

> **Nature of this view (important).** `countrylogisticsview` is the **MATERIEL / equipment-balance +
> fuel-prioritization** screen, NOT the supply-NETWORK map. Its grid rows are per-equipment-type
> {efficiency, status, need, balance, in-stock, weekly-production} — sourced from `economy` (stockpile,
> production lines, gross-IC) + `military.equipment_stockpile`. The supply_system_2 NETWORK data
> (`economy.logistics`) feeds the fuel-prioritization (priority/buffer/truck) side and is the home for the
> per-province supply PROXY (`supply.by_province`). **In this demo `economy.logistics`, `supply.by_province`,
> and `military.equipment_stockpile` deltas needed for the materiel grid are unfed → the bindings are
> schema-correct but render "—" (ARTIFACT).**

## 1a. Logistics — dynamic (data-bound)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `viewing_flag` (iconType, `GFX_flag_small2`) | `<VIEWER>` (observed country) | flag-small | **exact** | The country whose logistics is open. |
| `resources_grid` (gridBoxType, in `resource_strip_container`) → `logistics_overview_resource_item` rows | `countries.<VIEWER>.economy.resources.produced.*` (+ `available_derived`) | row factory | **exact** | One row per resource (steel/oil/aluminium/tungsten/chromium/rubber/coal). Row maps below (1b). |
| `materiel_grid` (gridboxtype, in `materiel`) → `logistics_overview_{land,naval,air}_equipment_entry` + `_other_entry` rows | per-equipment-type aggregate over `military.equipment_stockpile[]` + `economy.gross_ic_by_equipment` + production lines | row factory | **derived** | One row per equipment archetype in use, grouped land/naval/air. Per-row stock/need/balance below (1b). Needs the production-delta + stockpile join → derived; **unfed in demo (ARTIFACT)** → "—". |
| `nuke_count` (instantTextBoxType, in `nuke`) | `countries.<VIEWER>.extra.nukes_total` (SCHEMA §3.11) | int | **unavailable** (current data) | `extra` domain (country_extra.json) absent in this snapshot → "—". Closure: fold `country_extra.json` (LIVE-EXTRACTABLE). |
| `general_info` → `fulfilled` (iconType, `GFX_logistics_fuelbar`) — the AGGREGATE fuel bar | `economy.fuel.{current,max}` | progressbar-fraction | **derived** | **Codex flag (folded):** `fuel.current/max` IS present in this demo → the aggregate fuel fill binds NOW (`current/max`), do NOT badge unavailable. |
| `army_info`/`navy_info`/`air_info` → `fulfilled` (iconType, `GFX_logistics_fuelbar`) — per-BRANCH fuel | `economy.logistics.priority` (per-branch fuel prioritization) + `economy.fuel.{current,max}` | progressbar-fraction | **unavailable** (current data) | The per-branch SPLIT needs `logistics.priority` (`economy.logistics` unfed — ARTIFACT). The aggregate bar above is the bindable one. |
| `army_info`/`navy_info`/`air_info` → `low_prio`/`medium_prio`/`high_prio` (buttonType, `GFX_deploy_priority`) | `economy.logistics.priority` (selected fuel-priority tier per branch) | category (which tier active) | **unavailable** (current data) | Display-of-selected-tier is bindable from `logistics.priority`; unfed (ARTIFACT). The buttons themselves are mutation verbs (set priority) → inert. |
| `military_factories` → `military_factories_usage` (iconType, `GFX_logistics_fuelbar`) | `countries.<VIEWER>.economy.factories.mil` (+ under_construction usage) | progressbar-fraction | **derived** | `factories.mil` present → bar fill `derived` from used-vs-total; the exact "usage" fraction is computed. |
| `naval_factories` → `naval_factories_usage` (iconType, `GFX_logistics_fuelbar`) | `countries.<VIEWER>.economy.factories.dockyard` | progressbar-fraction | **derived** | As above (dockyards). |
| `logistics_info_window` line-chart readouts: `max_value`/`avg_value`/`min_value`/`min_x_value`/`max_x_value` (instantTextboxType) | per-equipment production history series | float / int | **unavailable** | A 30-day production-history time-series per equipment variant — not stored per-snapshot (the replay reconstructs cross-snapshot series, but this view's own chart needs the in-game running buffer). "—". |
| `logistics_info_window` → `variants_grid` (gridBoxType) → `logistics_info_equipment_entry` rows | per-variant {in_stock, weekly_production, request} | row factory | **derived** | Per-equipment-variant detail; from stockpile + production lines + lend-lease request. Unfed in demo (ARTIFACT). |
| `logistics_fuel_window` → `lines_grid` (gridBoxType) → `logistics_info_fuel_line_entry` rows | `economy.fuel` consumption/storage series | row factory | **unavailable** | Fuel stored/consumed history series — not stored as a per-snapshot buffer. "—". |

## 1b. Logistics — ROW templates (mapped once; repeat per grid row)

**`logistics_overview_{land,naval,air}_equipment_entry`** (materiel grid rows — identical shape, 3 variants):

| row element (name + type) | source (per equipment archetype) | formatter | fidelity |
|---|---|---|---|
| `equipment_type` (instantTextboxType, `txt=name`) | archetype name | loc | **derived** (name via equipment-DB; **ARTIFACT** value) |
| `equipment_icon` (iconType, `GFX_equipment_item`) | archetype → sprite | equipment-icon | **derived** |
| `efficiency_progressbar` (iconType, `GFX_unit_xp_bar`) | production efficiency of the line | progressbar-fraction | **unavailable** (line efficiency not folded; closure = production lines) |
| `status_progressbar` + `status_progress_marker` (iconType) | need-vs-have status | progressbar-fraction | **unavailable** (ARTIFACT — needs stockpile vs need) |
| `needs` (instantTextboxType, `txt=amount`) | total equipment NEED across deployed divisions | int | **unavailable** (need not computed in schema) |
| `produced` (instantTextboxType) | `economy.gross_ic_by_equipment[archetype]` (weekly) | int | **derived** (ARTIFACT — `gross_ic_by_equipment` null-until-delta-pass) |
| `balance` (instantTextboxType) | produced − consumed | signed-int | **derived** |
| `in_stock` (instantTextboxType) | `military.equipment_stockpile[archetype].amount` | int | **derived** (ARTIFACT) |
| `resources_grid` (gridBoxType) → `logistics_entry_resource_item` (iconType `icon`) | per-equipment resource cost icons | equipment/resource-icon | **derived** (static equipment-DB cost lookup) |

**`logistics_overview_{land,naval,air}_other_entry`** — `name` (loc label `LOGISTICS_LAND/NAVAL/AIR_OTHER_LABEL`)
+ `resources_grid`→`logistics_entry_resource_item`: the "other / uncategorised" catch-all row. **derived** (residual aggregate; ARTIFACT in demo).

**`logistics_overview_resource_item`** — `button` (`resource_item_background`), `icon` (`GFX_resources_strip`,
resource→frame), `value` (instantTextboxType, `txt=15`): one row per resource. `value` ← `economy.resources.produced.<res>` → **exact**; `icon` frame = resource id → **exact**.

**`logistics_info_equipment_entry`** — `equipment_variant` (loc name), `in_stock` / `weekly_production` /
`request` (int), `delete_button` (mutation→inert): per-variant production detail; **derived** (ARTIFACT). The `delete_button` is a mutation verb.

## 1c. Logistics — STATIC / decoration / navigation (no data binding)

- **Chrome / frames / headers:** `Background` (×many: `GFX_tiled_window_1b_border`, `GFX_tiled_window2_1b_border`,
  `GFX_tiled_generic_overlay_bg1`, `GFX_logistics_win_bottom`, `GFX_logistics_*_entry_bg`), `logistics_header_bg`,
  `viewing_flag_border`, `resources_bg`, `material_header_bg`, `trade_filter_bg`, `fuel_header`, all `*_bg`/`*_background`
  in `military_factories`/`naval_factories`/`nuke`, `linechart_tiling_bg`/`linechart_bg`/`linechart`/`linechart_frame`
  (chart canvases — cosmetic), `header_bg`, the icons `efficiency_icon`/`producing_icon`/`need_icon`/`balance_icon`/
  `in_stock_icon`/`military_factory_icon`/`naval_factory_icon`/`nuke_icon`/`fuel_logisctics_icon`/`details_button`.
- **Loc HEADER labels (static text):** `logistics_title` (`LOGISTICS_LOGISTICS_LABEL`), `equipment_type_label`,
  `status_label`, `resources_label`, `LOGISTICS_FUEL_PRIORITIZATION` `label`, `type_text` (`TOPBAR_ARMY`/`_NAVY`/`_AIR`/
  `PRODUCTION_OUTPUT`), `military_factories_label`, `naval_factories_label`, `nuke_label` (`NUCLEAR_BOMB`),
  the `logistics_info_window`/`fuel_window` header labels (`LOGISTICS_EQUIPMENT_VARIANT_LABEL`/`_IN_STOCK_LABEL`/
  `_NEEDS_LABEL`/`_WEEKLY_PRODUCTION_LABEL`, `x_axis_type` `LINECHART_DETAILS_DAYS`).
- **Navigation / mutation (inert):** `close_button` (×3), `expand_button` (general_info), `btn_years`/`btn_months`/
  `btn_days` (chart-level toggles — UI control), `btn_stored`/`btn_consumed` (fuel chart toggles), `icon_checkbox`
  (fuel-line toggle in `logistics_info_fuel_line_entry`), the prio buttons (1a).
- **Structural containers (positioning only):** `countrylogisticsview`, `resource_strip_container`, `materiel`,
  `headers`, `fuel_info`, `army_info`/`navy_info`/`air_info`/`general_info`, `production_win_bottom`,
  `military_factories`/`naval_factories`/`nuke`, `logistics_info_window`, `logistics_fuel_window`,
  `linechart_tiling_bg`, `variants`, `lines`, and the 8 row-template roots.

---
---

# VIEW 2 — GARRISON LOG (`game_assets/gui/garrison_log.json`)

Source `interface/garrisonlogview.gui`. Root `garrison_log_view` (header + month slider + 2 tabs:
`resistance_activities_tab`, `equipment_tab`), plus 3 runtime ROW templates (`countryflag_garrison_entry`,
`resistance_attack_item`, and the equipment-list rows reuse combat-log loss rows).

> **Nature of this view.** A **historical per-country ledger** of garrison resistance attacks +
> equipment lost to resistance — the same stored-ledger class as the combat-log (see
> BINDING_MAP_diplomacy_combat.md §4). Our snapshots do not carry this ledger. The replay can
> reconstruct resistance EVENTS from the diff timeline (`resistance_changed` / `compliance_changed`,
> SCHEMA §9.2) but NOT the per-attack manpower/equipment loss tally (that is an in-game running buffer,
> not stored). Most value cells → `unavailable` (TRUE GAP — not stored as a ledger).

## 2a. Garrison log — dynamic / derivable

| gui element (name + type) | source | formatter | fidelity | notes |
|---|---|---|---|---|
| `flags` (OverlappingElementsBoxType) → `countryflag_garrison_entry` rows (`flag` buttonType `GFX_flag_small2` + `flag_frame`) | the set of occupier/occupied TAGs in the window | flag-small | **derived** | Belligerent flags for the filtered period — derivable from the resistance-event set. |
| `activities_list` (gridBoxType, in `resistance_activities_tab`) → `resistance_attack_item` rows | diff timeline `resistance_changed` (per state) | row factory | **derived** | Reconstructable from the event timeline; until the adapter is wired → `empty_msg` (`COMBAT_LOG_EMPTY_COMBATS`). Row mapped in 2b. |
| `period_desc` / `min_log_months` (`1`) / `max_log_months` (`12`) (instantTextbox) | month-window of the log slider | int/loc | **derived** | UI control over the reconstructed history range. |
| `total_equipment_lost` (instantTextBoxType, in `equipment_tab`) | aggregate equipment lost to resistance over window | int | **unavailable** | Garrison equipment-loss ledger not stored → "—". |
| `equipments_list` (smoothListBoxType, in `equipment_tab.subwindow`) → equipment-loss rows | per-equipment garrison loss tally | row factory | **unavailable** | Running ledger not in snapshot → `empty_msg`. |

## 2b. Garrison log — ROW templates

**`resistance_attack_item`** — `location` (buttonType, province name → loc, **derived**), `date`
(instantTextBox → clock, **derived**), `attack_blocked` (text, **unavailable** — outcome not stored),
`garrison_losses` → `lost_manpower` / `lost_equipment` (instantTextBox, **unavailable** — per-attack loss
ledger not stored), `flag` (buttonType `GFX_flag_small2` → the attacker/garrison TAG, **derived**). One row
per resistance attack.

**`countryflag_garrison_entry`** — `flag` (buttonType, TAG → flag-small, **derived**) + `flag_frame`
(static). One row per belligerent flag in the overlapping-flags box.

## 2c. Garrison log — STATIC / decoration / navigation

- **Chrome / frames / titles:** `Background` (×7: `GFX_tiled_plain_bg`, `GFX_tiled_paper_bg2`,
  `GFX_combat_log_info_bg`, `GFX_combat_log_info_bg_overlay`, `GFX_garrison_log_entry`), `battle_log_top`,
  `manpower_icon` (`GFX_casuality_icon`), `equipment_icon` (`GFX_equipment_icon`), `combat_log_info_header`,
  loc title labels (`title` `GARRISON_LOG`, `title_losses_in_manpower`, `title_losses_in_equipment`,
  `title_equipment_lost` `COMBAT_LOG_EQ_LOST`), `empty_msg` (×2 loc placeholders).
- **Navigation / mutation (inert):** `close_button`, `resistance_activities_tab_button`,
  `equipment_tab_button` (tab switch verbs), `filter_button` (`COMBAT_LOG_FILTER_BTN`).
- **Scrollbar (UI control):** `log_month_slider` (+ `Slider`/`Track`/`Decrease`/`Increase`/`tileSize`),
  the `smoothListBoxType` scroll state.
- **Structural containers:** `garrison_log_view`, `resistance_activities_tab`, `equipment_tab`,
  `subwindow`, `garrison_losses`, and the row-template roots.

---
---

# COMBAT-RESULT VIEWS (3, 4, 4b) — roster-aggregation result siblings (PLAN §C.2 + §G)

Per PLAN §C.4 these are the **combat-RESULT** siblings of the live-combat views in
BINDING_MAP_diplomacy_combat.md. They share that map's two hard facts: **(i)** the demo carries **0 naval
combats** and no `order_of_battle` → there is no live naval engagement to bind; **(ii)** even when fed,
the per-ship in-battle DETAIL (positioning/screening/escape) is `unavailable` (§G). What CAN bind is the
**aggregate loss/kill ledger** for a sea zone or a country — sourced from `world_events.sunk_convoys` +
`convoy_losses_by_{killer,owner}` (SCHEMA §17) and `military.ships_by_hull` / `military.casualties`
(§3.2) — but **all three of those are unfed in this demo (ARTIFACT)**.

---

# VIEW 3 — NAVAL COMBAT RESULTS (`game_assets/gui/naval_combat_results.json`)

Source `interface/navalcombatresultswindow.gui`. Root `navalcombatresultswindow` (a post-battle report:
attacker/defender leader + HQ-leader headers, attacking/defending side survivor+lost grids, an outcome
label, details panels), plus **16 runtime ROW templates** (survivor / lost-line / lost-ship / air-lost
entries, attacker & defender variants).

> **In this demo: 0 naval combats + `world_events` unfed → the whole view binds nothing (all "—"), but
> every element is enumerated so completeness is provable.** This is a **transient runtime report window**
> (it pops after a naval battle resolves); the replay would instantiate it from a reconstructed
> `combat_ended` (kind=naval) event + the convoy/ship loss ledger.

## 3a. Naval combat results — dynamic (would-bind)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `attacker_leader.flag` / `defender_leader.flag` (buttonType, `GFX_flag_small2`) | the two belligerent TAGs of the naval `combat_ended` event | flag-small | **unavailable** (current data) | 0 naval combats + `world_events` unfed → "—". Would be `derived`. |
| `attacker_leader.leader_portrait` / `defender_leader.leader_portrait` (iconType, `GFX_leader_unknown`) | `commanders.admirals.*` of the battle | portrait | **unavailable** (current data) | `commanders` domain absent. |
| `name` (instantTextboxType, `txt=name`) | admiral name | loc | **unavailable** (current data) | As above. |
| `outcome` (instantTextboxType, `txt=WINNER`) | which side won | loc (`WINNER`/`LOSER`) | **unavailable** | Battle outcome not stored (combat is reconstructed as inferred, §9; outcome flag not modelled). |
| `attacking_side.survivors_grid` / `defending_side.survivors_grid` (gridBoxType) → `naval_combat_results_{attacker,defender}_survivor_entry` rows | per-hull surviving ship counts of each side | row factory | **unavailable** (current data) | Needs the per-battle ship roster (`order_of_battle` absent + 0 battles). Row in 3b. |
| `attacking_side.lost_line_grid` / `defending_side.lost_line_grid` (gridBoxType) → `naval_combat_results_{attacker,defender}_lost_line_entry` rows | per-hull SHIPS LOST this battle | row factory | **unavailable** (current data) | Closest schema home is `military.ships_by_hull` deltas + `world_events.sunk_convoys` (for convoy hulls) — both unfed (ARTIFACT). Row in 3b. |
| `attacker_details.text` / `defender_details.text` (instantTextboxType) | per-side detail summary | loc | **unavailable** | Battle-detail (§G). |

## 3b. Naval combat results — ROW templates (mapped once; ×attacker/defender variants identical)

| row template | row elements | source | fidelity |
|---|---|---|---|
| `naval_combat_results_{attacker,defender}_survivor_entry` | `fleet` (iconType, hull-type sprite), `country_flag_{left,right,center}` (iconType, `GFX_mapicon_unit_flag_stripe`), `number` (instantTextboxType, `txt=256`) | surviving ship count by hull, per owner TAG | **unavailable** (current data) — needs per-battle ship roster |
| `naval_combat_results_{attacker,defender}_lost_line_entry` | `equipment` (iconType, ship-class sprite), `pride` (iconType, `GFX_NV_icon_pride` — flagship), `lost_ships_grid` (gridBox)→`*_lost_ship_entry` | ships lost this battle by hull | **unavailable** (current data) — `ships_by_hull` deltas / `sunk_convoys` unfed |
| `naval_combat_results_{attacker,defender}_lost_ship_entry` | `country_flag` (iconType, `GFX_flag_smallest`), `number` (instantTextboxType) | per-owner lost-ship count | **unavailable** (current data) |
| `naval_combat_results_{attacker,defender}_air_lost_line_entry` + `*_lost_air_entry` | `equipment` (iconType), `lost_air_grid`→`country_flag`+`number` | carrier-air losses this battle | **unavailable** — air-loss-per-battle not stored (§G) |
| `naval_combat_results_{attacker,defender}_air_entry` | `plane` (iconType, airframe sprite), `country_flag_{left,right,center}`, `number` | carrier-air committed | **unavailable** — same |

## 3c. Naval combat results — STATIC / decoration / navigation / mutation

- **Chrome / frames:** `Background` (`GFX_naval_battle_report_bg`, `GFX_tiled_window_transparent` ×many),
  `leader_frame`/`hq_leader_frame` (×attacker/defender), `flag_frame` (`GFX_diplo_countrylist_flag_frame`),
  `hq_icon` (`GFX_buildings_strip`), `attacker_hq_leader`/`defender_hq_leader` portrait + frame, the
  `attacker_details`/`defender_details` `Background` (`GFX_mini_tooltip`), `traits_grid` (gridBox, leader
  traits → **unavailable**, needs commanders), `margin`.
- **Navigation / mutation (inert):** `open_details` (toggle), `close_button`, `dismiss_button`
  (`GFX_button_reset_123x34` — dismiss the report).
- **Structural containers:** `navalcombatresultswindow`, `attacker_leader`/`defender_leader`,
  `attacker_hq_leader`/`defender_hq_leader`, `attacking_side`/`defending_side`, `survivors`/`lost`,
  and the 16 row-template roots.

---

# VIEW 4 — DESTROYED SHIPS (`game_assets/gui/destroyed_ships.json`) — PRESENT-BUT-EMPTY

**This parsed JSON is a STUB:** `{"view":"destroyed_ships", "source":"interface/destroyedshipswindow.gui",
"windows":[], "stats":{"element_type_counts":{}}}` — **0 windows, 0 elements.**

**WHY it is empty (not skipped):** the effective-load-set parse produced **no `containerWindowType`** for
this view. The most likely causes (both consistent with the game's runtime model):
1. **Runtime-instanced shell.** `destroyedshipswindow.gui` in the mod's effective load set defines no
   static container — the "destroyed ships" list is a **runtime-instanced window** built by the engine from
   a shared prototype (the row template lives elsewhere, e.g. `naval_losses.gui`'s
   `naval_losses_sunk_ship_entry`, or the report rows in `navalcombatresultswindow.gui`). The `.gui` file is
   effectively a forward-declaration / empty placeholder; nothing to bind statically.
2. **Override-stub.** The mod (or a vanilla 1.19 reshuffle) replaced/emptied this file, folding its content
   into `naval_losses.gui` (VIEW 4b), which DOES carry `naval_losses_sunk_ship_entry` + the sunk-ship
   roster. (`naval_losses.gui` is the modern "Naval Losses" window; `destroyedshipswindow.gui` is the
   legacy name.)

**What it WOULD bind (when/if instanced):** a per-ship "destroyed ships" roster — same shape as
`naval_losses_sunk_ship_entry` (4b): `{ship_owner_flag, name, icon (hull), killed_icon, ship_killer_flag}`.
**Sources:** `world_events.sunk_convoys[]` (for convoy hulls) + `world_events.convoy_losses_by_{killer,owner}`
(SCHEMA §17) for the convoy side, and `military.ships_by_hull` deltas (§3.2) for warship hulls.
**Fidelity if instanced:** **unavailable (current data)** — both `world_events` and `ships_by_hull` are
unfed in this demo (ARTIFACT); the per-ship NAME + exact killer attribution for warships is a **TRUE GAP**
(only the per-month convoy ledger is stored, not a per-warship destruction log).

> **Codex correction (folded).** The empty stub is itself only an ARTIFACT/placeholder — it justifies the
> unavailable-ARTIFACT badge for the naval-loss surface, but it does **NOT** by itself bind `sunk_convoys`
> or `ships_by_hull`. The binding materialises only once `world_events` / `military.ships_by_hull` are
> actually fed AND the window is instanced (likely as VIEW 4b). "Empty `.gui`" ≠ "no losses occurred" —
> it is a parse/load-set artifact, not a data statement.

**Element accounting:** 0 static elements → 0 bindable, 0 static, 0 deferred. Documented as present-but-empty;
the binding intent is carried by VIEW 4b.

---

# VIEW 4b — NAVAL LOSSES (`game_assets/gui/naval_losses.json`)

Source `interface/naval_losses.gui`. Root `navallosseswindow` (a ship-loss browser: totals header,
country/faction filter dropdowns, a `ships_grid`), plus 7 runtime ROW templates
(`equipment_archetype_entry`, `accident_equipment_archetype_entry`, `naval_losses_sunk_ship_entry`,
`naval_losses_header_entry`, `navallosses_dropdown_item`, `navallosses_name_header`).

> This is the **modern home of the destroyed-ships roster** (see VIEW 4). It is a historical ledger of
> ships lost (in combat, by accident/training, by mines) + kills. The schema homes are
> `world_events.sunk_convoys` + `convoy_losses_by_{killer,owner}` (convoy side) and `military.ships_by_hull`
> deltas (warship side). **All unfed in this demo → "—" (ARTIFACT).** The exact per-ship NAME and the
> accident/mine breakdown are a **TRUE GAP** (not stored at per-ship granularity).

## 4b-a. Naval losses — dynamic (would-bind)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `total_ships` (instantTextboxType, in `navallosseswindow`) | sum of own ships lost (window scope) | int | **unavailable** (current data) | `ships_by_hull` deltas / convoy ledger unfed (ARTIFACT). |
| `total_ships_sunk` (instantTextboxType) | sum of enemy ships sunk | int | **unavailable** (current data) | `world_events.convoy_losses_by_killer[<VIEWER>]` (convoys) + warship-kill ledger (warship kills not stored = TRUE GAP). |
| `ships_grid` (gridBoxType, in `ships`) → `naval_losses_sunk_ship_entry` / `equipment_archetype_entry` / `accident_equipment_archetype_entry` / `naval_losses_header_entry` rows | the loss roster | row factory | **unavailable** (current data) | Rows mapped in 4b-b. Reconstructable from `world_events` once folded (convoys exact; warships partial). |
| `countries_filter_dropdown` → `countries` grid → `navallosses_dropdown_item` rows | filterable TAG set | row factory; flag + name | **derived** | The filter set = TAGs with losses; bindable from the ledger (ARTIFACT in demo). The dropdown itself is a UI control. |
| `factions_filter_dropdown` → `factions` grid | filterable faction set | row factory | **derived** | `diplomacy_world.factions` (present!) → faction list is `exact`; loss-by-faction values unfed (ARTIFACT). |

## 4b-b. Naval losses — ROW templates

| row template | row elements | source | fidelity |
|---|---|---|---|
| `naval_losses_sunk_ship_entry` | `ship_owner_flag` (iconType `GFX_flag_small`), `name` (instantTextbox, e.g. `USS Enterprise`), `icon` (hull), `killed_icon`, `ship_killer_flag` (iconType) | one sunk ship: owner TAG + name + killer TAG | **unavailable** (current data) — convoy hulls from `sunk_convoys` (owner/killer exact, **no per-ship NAME** = TRUE GAP); warships from `ships_by_hull` delta (unfed) |
| `equipment_archetype_entry` | `equipment_icon` (naval), `name`, `lost`, `first_kills`, `second_kills`, `other_kills` (int) | per-hull-archetype lost + kill tallies | **unavailable** (current data) — `ships_by_hull` aggregate unfed (ARTIFACT) |
| `accident_equipment_archetype_entry` | `name`, `sunk_by_training`, `sunk_by_mines`, `damage_by_training`, `damage_by_mines` (int) + training/mines icons | non-combat losses | **unavailable** — accident/mine loss attribution not stored (TRUE GAP) |
| `naval_losses_header_entry` | `text` (loc), `our_losses` / `our_kills` (int) + icons | section header w/ summary | **unavailable** (current data) — same ledger |
| `navallosses_dropdown_item` / `navallosses_name_header` | `text` (loc/TAG name) + bg | filter-row chrome | **derived** (TAG/faction name) |

## 4b-c. Naval losses — STATIC / decoration / navigation / mutation

- **Chrome / frames / titles:** `Background` (`GFX_tiled_plain_bg`, `GFX_tiled_window2_1b_border`,
  `GFX_mini_bg`, `GFX_naval_losses_ship_bg`, `GFX_generic_background`, `GFX_tiled_window_1b_thin_border`),
  `template_title` (`NAVAL_LOSSES`), `back_from_ship_list_bg`/`header_ship_list_bg` (`GFX_naval_fleet_entry`),
  `naval_ship_icon_loss`/`total_ships_sunk_icon` (`GFX_naval_ship_icon_win`), `our_losses_icon`/`our_kills_icon`,
  `bg`/`country_name_bg` (dropdown frames), `expand_button` (×2), `equipment_icon` (`GFX_equipment_naval_item`),
  `training_icon`/`mines_icon`, the `name` labels (`COMBAT_HEADER`/`ALL_FACTIONS`).
- **Navigation / mutation (inert):** `close`, `back_from_ship_list`, `naval_losses_battle_bg` (button →
  open battle).
- **Structural containers:** `navallosseswindow`, `ships`, `countries_filter_dropdown`/`factions_filter_dropdown`
  (+ `*_filter_bg` / `expanded_window` / `countries` / `factions`), and the row-template roots.

---

# VIEW 5 — NAVAL STRATEGIC REGION (`game_assets/gui/naval_strategic_region.json`)

Source `interface/naval_strategic_region_view.gui`. Root `navalregionview` (a sea-zone info panel: region
name, own vs enemy ship/convoy/plane/mine readouts, a dominance progress bar + flag, naval-access rules),
plus a `naval_region_modifier_entry` ROW template.

> **This is the BEST schema fit for `naval_air.region_control` (SCHEMA §13.1) — the LIVE PROXY sidecar.**
> It carries a PERMANENT PROXY / region-granularity badge: the values are sea-zone-level dominance reads,
> NOT per-province/per-ship ground truth, and `naval_air.confidence` is **never** `"exact"`. In this demo
> `naval_air` is **unfed (ARTIFACT)** → "—". Convoy/mine/enemy counts are a deeper proxy still.

## 5a. Naval strategic region — dynamic (would-bind, PROXY)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `region_name` (instantTextboxType) | strategic_region id → name | loc | **derived** | `map.strategic_regions` (static) / region-DB. Name `derived`; **value unfed in demo (ARTIFACT)**. |
| `dominance_flag.flag` (buttonType, `GFX_flag_small`) | `naval_air.region_control[<region>][0].controller` (dominant TAG) | flag-small | **proxy** (PERMANENT) + **unavailable** (current data) | Region-granularity dominance PROXY (§13.1); unfed in demo. Carries PROXY badge even when fed. |
| `progress` / `progress_enemy` / `neutral_progress` / `progress_disabled` (iconType bars) | own-vs-enemy naval dominance fraction in the region | progressbar (−1..1) | **proxy** + **unavailable** (current data) | From `region_control` controller list; PROXY (region-level, not the in-game continuous value). |
| `progress_frame_window.dominance_marker` (iconType, `GFX_vertical_marker`) | dominance bar marker position | progressbar-fraction | **proxy** + **unavailable** (current data) | Same source. |
| `ship_text` (instantTextboxType, `txt=123`) | own ship count in the region | int | **proxy** + **unavailable** | Region ship presence — deeper proxy (count not in `region_control` leaf; needs naval OOB). |
| `convoys_text` (`txt=0`) | own convoys in region | int | **unavailable** | Convoy-by-region not in schema. |
| `plane_text` (`txt=20%`) | own air superiority % in region | percent | **proxy** + **unavailable** (current data) | `naval_air.air_superiority[<region>].controller` PROXY (§13.1); unfed. |
| `mine_text` (`txt=15%`) | own mine coverage % | percent | **unavailable** | Mine coverage not in schema (TRUE GAP). |
| `enemy_ship_text` (`txt=24-100`) / `enemy_convoys_text` / `enemy_plane_text` / `enemy_mine_text` (4×) | enemy equivalents | int / int-range / percent | **proxy** / **unavailable** | Enemy ship range is a fog-of-war estimate (TRUE GAP — like diplomacy intel estimates); enemy air from `air_superiority`; enemy mine = TRUE GAP. |
| `modifier_grid` (gridboxType) → `naval_region_modifier_entry` rows (`icon` `GFX_temperature_icons`) | region weather/terrain modifiers | category icons | **unavailable** | Sea-zone weather/modifiers = COMPUTED-NOT-STORED (§G / weather tier). |
| `terrain_picture` (iconType, `GFX_terrain_default`) | region terrain | category | **unavailable** | Sea-zone terrain art — battle-detail-class (§G). |

## 5b. Naval strategic region — STATIC / decoration / navigation / mutation

- **Chrome / frames / icons:** `Background` (`GFX_naval_region_window_bg`), `Header`, `dominanceheader`,
  `dominance_icon`, `progress_frame`, and all the label ICONS (`ship_icon`/`convoys_icon`/`plane_icon`/
  `mine_icon`/`enemy_*_icon` ×8 — `GFX_naval_region_window_*_icon`).
- **Navigation / mutation (inert):** `close_button`, `sunk_ships.logbook` (open the sunk-ships log →
  navigation), `naval_access` → `naval_access_rule_allowed`/`_avoid`/`_blocked` (buttonType — these SET a
  naval-access rule = mutation verbs; `naval_access_text` `NAVAL_ACCESS` loc + `naval_access_rule_highlight`
  cosmetic). The currently-selected rule is bindable-in-principle (region access state) but not in schema →
  the rule-display state is `unavailable`.
- **Structural containers:** `navalregionview`, `sunk_ships`, `dominance_flag`, `progress_frame_window`,
  `naval_access`, `modifier_grid`, `naval_region_modifier_entry` (row root).

---
---

# VIEW 6 — UNIT VIEW (`game_assets/gui/unit_view.json`)

Source `interface/unitview.gui` — the BIG division/ship/army detail panel (539 elements, 17 distinct
top-level windows). This is the **richest EXACT-binding view in this map**: the snapshot's `divisions[]`
(1654 entries, memory-channel body) feeds the per-division org / strength / equipment / template / movement
rows DIRECTLY and **exactly**. The save-only `divisions_full` body (dig-in, supply, acclimatization,
manpower, kills) is **absent in this demo → those specific cells `unavailable` (ARTIFACT)**; ship/air
detail needs `order_of_battle` (absent).

> **Repeated-row strategy.** This file is a library of ~13 runtime ROW templates (sibling top-level
> `containerWindowType`s). I map each template ONCE; it instantiates per division / per ship / per
> equipment / per army. The four big surfaces are: the **army badge** (`army_badge_view`), the **division
> list** (`army_division_list_view` + `army_division_view` row), the **division stats panel**
> (`army_division_stats_view` + `template_tab` / `equipment` / `info` stat grids), and the **ship stats
> panel** (`ship_stats_window`).

## 6a. Unit view — army_division_view ROW (the per-division list row) — the EXACT core

One `army_division_view` is instantiated per division. Source = a `divisions[]` entry for `<VIEWER>`.

| row element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `unit_name` (instantTextboxType) | `divisions[i].division_name` (resolved) ELSE `template_name` | loc | **derived** | `division_name` (divisions_full) absent in demo → falls back to `template_name` (present, exact). |
| `unit_icon` (buttonType, `GFX_unit_infantry_icon_medium`) | `divisions[i].template_role` → sprite | unit-icon | **exact** | `template_role` present (`motorized`/`armor`/…). |
| `unit_level` (iconType, `GFX_unit_level`) | `divisions[i].experience` → veterancy tier | category | **unavailable** (current data) | `experience` key present but **null** in demo → "—". |
| `xp_bar` (iconType, `GFX_unit_xp_bar`) | `divisions[i].experience` (0..max within tier) | progressbar-fraction | **unavailable** (current data) | null in demo. |
| `bar_org` (iconType, `GFX_navieslist_bar_org`) | `divisions[i].organisation` | progressbar-fraction | **exact** | `organisation` present (memory `/100000`, e.g. `99.879`). Diff only within one channel (SCHEMA §6 note). |
| `bar_str` (iconType, `GFX_navieslist_bar_str`) | `divisions[i].strength` | progressbar-fraction | **exact** | `strength` present (memory-scaled; channel-comparable only). |
| `bar_prepare` (iconType, `GFX_unitlist_bar_prepare`) | division planning/preparedness | progressbar-fraction | **unavailable** | Planning bonus not in `divisions[]` schema. |
| `unitlist_dig_in_icon` + `dig_in_bonus` (iconType) | `divisions[i].dig_in` / `dig_in_cap` | progressbar-fraction | **unavailable** (current data) | divisions_full fields — absent in demo (ARTIFACT). |
| `unitlist_fuel_icon` (iconType, `GFX_fuel_strip`) | division fuel state | category | **unavailable** | Per-division fuel not in `divisions[]` schema. |
| `unitlist_acclimatization` (iconType) | `divisions[i].acclimatization.{hot,cold}` | category | **unavailable** (current data) | divisions_full — absent (ARTIFACT). |
| `unitlist_movement_icon` (iconType) | `divisions[i].moving` (+ `move_target_province`) | category (moving/static) | **exact** | `moving` present. |
| `low_supply` (iconType, `GFX_unitcounter_supply_status_strip`) | `divisions[i].supply_status` | category | **unavailable** (current data) | `supply_status` key present but **null** in demo. |
| `attrition` (iconType, `GFX_unit_attrition`) | division attrition state | category | **unavailable** | Not in schema. |
| `flag` (iconType, `GFX_mapicon_unit_flag`) | `divisions[i].owner` | flag | **exact** | `owner` present (TAG). |
| `unit_land_combat_grid` (gridboxtype) → `unit_land_combat_entry` rows (`combat_icon` + `combat_progressbar`) | `divisions[i].in_combat` → the combat it is in | progressbar | **unavailable** | `in_combat` (bool) present, but the combat-progress bar value is battle-detail (§G). The boolean lights the icon (`derived`); the BAR is `unavailable`. |
| `no_orders` / `invalid_orders` / `obsolete_template` / `unitlist_priority_icon` / `foreign_template_flag` / `exile/colonial flag frames` | order state / template-obsolescence / priority / foreign-template | category | **unavailable** | Order-validity + obsolescence + priority not in `divisions[]` schema. |

## 6b. Unit view — army_division_stats_view: template + equipment + stat grids

Instantiated for a selected division. Tabs: `stats`, `template`, `history`.

| element | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `unit_title` (`UNIT_STATS`) / `experience` (instantTextbox) | `divisions[i].experience` | float | **unavailable** (current data) | null in demo. |
| `commanding_officer` / `commanding_officer_title_text` | division officer (`has_officer`) → `commanders` | loc | **unavailable** (current data) | `has_officer` (divisions_full) absent + `commanders` domain absent. |
| `current_manpower` (instantTextbox) | `divisions[i].army_manpower.value` | int | **unavailable** (current data) | divisions_full — absent (ARTIFACT). |
| `equipment_ratio` (instantTextbox) | filled-vs-needed equipment | percent | **derived** | From `equipment[]` amount vs template need; **unavailable** in demo (need not modelled). |
| `equipment` → `equipment_list` (gridBoxType) → `stat_equipment_entry` rows | `divisions[i].equipment[]` | row factory | **exact** | Each entry `{archetype, equip_id, equip_type, amount}` present → rows EXACT. Row in 6c. |
| `template_tab.subunits` → `regiments_grid` / `support_grid` / `regiment_support_grid` (gridboxtype) → `unit_stats_view_subunit_entry` / `_support_entry` rows | division template composition (`designs.division_templates[]`, SCHEMA §11) | row factory | **derived** | Template structure from the design catalog joined on `template_id`. `combat_width_text` per template → **derived** (static template stat), NOT the live combat width. |
| `info.base_stats` / `combat_stats` / `misc_stats` → `stats_grid` (gridboxtype) → `stat_entry` rows (`label`+`value`) | per-division computed combat stats (soft/hard attack, defense, breakthrough, …) | row factory | **unavailable** | Computed combat stat-rows = COMPUTED-NOT-STORED (§G — same class as designer combat stats). The TEMPLATE base stats are `derived` (design catalog); the EFFECTIVE (equipment-scaled) values are `unavailable`. |
| `history_tab` → `history_entry_grid` (listboxType) + `history_kill_count` / `history_experience_count` | per-division combat history (kills, xp gained, last combat) | int / loc | **unavailable** (current data) | `last_combat_date` (divisions_full) + kill ledger absent; per-division kill history not stored. |
| `adjuster_container.stats_grid` → `stat_entry` rows | production-adjuster previews | row factory | **unavailable** | Runtime-computed adjuster math (§G). |
| `softness` / `composition_icon` / `armor_softness_icon` / `inf_softness_icon` (iconType) | division hardness/composition ratios | percent | **derived** | Derivable from `equipment[]` archetype mix; **unavailable** in demo (hardness math not modelled). |

## 6c. Unit view — equipment / template ROW templates (mapped once)

| row template | row elements | source | fidelity |
|---|---|---|---|
| `stat_equipment_entry` | `equipment_icon` (archetype sprite), `creator_flag` (iconType `GFX_flag_small`), `equipment_name` (loc), `equipment_amount` (int) | `divisions[i].equipment[j]` `{archetype, amount}` (+ creator from `equip_type`→variant→`designs`) | **exact** (amount + archetype) / **derived** (name + creator flag) |
| `stat_archetype_header_list_entry` | `archetype_name` (loc) + `archetype_amount` (int) | grouped equipment by archetype | **exact** (amount) / **derived** (name) |
| `stat_equipment_entry_naval` | `equipment_icon`, `equipment_name`, `role_icon` (`GFX_naval_equipment_role_icons`) | ship-equipment of a fleet | **unavailable** (current data) — needs `order_of_battle` |
| `stat_equipment_entry_railway_gun` | `equipment_icon`, `equipment_name` | railway-gun attachment | **derived** (from equipment list) |
| `unit_stats_view_subunit_entry` / `_support_entry` | `unit_icon` (`GFX_locked_icon`) | template battalion / support slot | **derived** (design catalog, §11) |
| `unit_template_entry` | `division` (iconType), `name` (loc), `cp_cost_text` (int) + priority/foreign flags | a deployable template | **derived** (`designs.division_templates[]`) |

## 6d. Unit view — ship_stats_window (the ship detail panel)

Instantiated for a selected ship. **All `unavailable` (current data)** — needs the `order_of_battle`
domain (absent in this snapshot); when fed, ship identity/org/str/equipment become `exact`, the computed
combat stat grids stay `unavailable` (§G).

| element | intended source | fidelity |
|---|---|---|
| `ship_name` (editBox), `ship_title` (`SHIP_UNIT_STATS`), `experience`, `unit_level` | per-ship name/xp/level | **unavailable** (current data) — `order_of_battle.fleets[].task_forces[].ships[]` |
| `dominance_value` / `support_value` (instantTextbox) | ship dominance/support contribution | **unavailable** — computed (§G) |
| `current_manpower` / `fuel_consumption` (instantTextbox) | ship crew / fuel | **unavailable** (current data) |
| `captain` / `captain_title_text` | ship captain → `commanders.admirals` | **unavailable** (current data) — commanders absent |
| `equipment` → `equipment_grid`; `air_wings.list` → `ship_stats_air_wing_entry` (`count`/`name`/`ace_icon`); `terrain.terrain_grid`; `info.{base,combat,misc}_stats` grids; `history.{medals_grid, history_grid}` | ship modules / carrier air wings / terrain mods / computed stats / medals & history | **unavailable** (current data) / **unavailable** (§G for computed stats) |

## 6e. Unit view — army_badge_view (the army/army-group badge) — leader + aggregate org/str

Instantiated per army/army-group. Source = the army's leader + aggregate of its divisions.

| element | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `divisions_count` / `armies_count_box` / `army_group_count_box` (instantTextbox) | count of divisions in the army | int | **derived** | Derivable by grouping `divisions[]` by army; army-membership not directly in `divisions[]` schema → **unavailable** in demo (no army-join). |
| `leader_name` / `leader_skill` / `leader_stat_{attack,defense,planning,logistics}` (instantTextbox) | army commander → `commanders.{generals,field_marshals}` | loc / int | **unavailable** (current data) | `commanders` domain absent. |
| `traits_grid` (gridBox) → leader trait rows | `commanders.*.traits[]` | row factory | **unavailable** (current data) | As above. |
| `army_org_str_indicator.bar_org` / `bar_str` (+ medians) | aggregate org/strength of the army's divisions | progressbar | **derived** | Aggregable from `divisions[].{organisation,strength}` IF army membership were known (not in `divisions[]`) → **unavailable** in demo. |
| `supply_icon` / `fuel_icon` (iconType) | army supply/fuel state | category | **unavailable** | Per-army supply not in schema (per-province PROXY is the closest, §13.2). |

## 6f. Unit view — STATIC / decoration / navigation / mutation (no binding)

- **Color picker (`color_picker_root`/`color_picker` + `color_picker_preset_entry`):** entire HSV army-color
  picker — a **mutation** (set army color) → inert. All sliders (`hue_slider`/`sat_slider`/`val_slider` +
  their scrollbar guiButtons), `presets` grid, `group_marker`, `change_group_marker_up/down`,
  `orders_group_icon` → deferred/inert.
- **Expeditionary-force UI (`expeditionary_force_container` + `_country_entry` + `expeditionary_force_button`):**
  request-expeditionaries dialog = **mutation**; `country_name`/`num_units`/`flag` would bind to a target TAG
  but the +/− buttons (`units_add/sub_*`) and `send_button` are mutation verbs → entire dialog deferred/inert.
- **Division-list chrome / sort / actions:** `army_division_list_view` `column_heads`
  (`organisation_strength`/`warning`/`template`/`experience`/`preparedness`/`status`/`name`/`fuel`/`army` — sort
  buttons, UI controls), `hq_window` (`deploy`/`undeploy`/`deploy_dropdown`/`templates_grid` — deploy = mutation),
  `redeployment_type_button`/`btn_consolidate_selection`/`btn_select_half`/`btn_hold`/`btn_unassign`/`btn_close`/
  `btn_delete` (mutation verbs), `convoys_icon`/`transport_weight` (transport readout — `derived` if naval
  transport modelled, else static), `divisions` dropdown + `expand_button`/`expanded_window`/`divisions_grid`
  (structural), `bottom_window.divisions_count` (`UNASSIGNED_UNIT_DESCRIPTION` loc).
- **Stat-panel chrome / mutation:** tab buttons (`stats_tab_button`/`template_tab_button`/`history_tab_button`/
  `tab_stats`/`tab_history`/`tab_design`), `btn_pride`/`promote_officer`/`promote_captain`/`portraittime`/
  `change_template_icon`/`template_change_icon`/`division_name_dropdown` (rename = mutation),
  `medal_dropdown`/`medal_button_entry`/`medal_instance_entry`/`army_history_entry` (assign medal = mutation),
  `unit_hq_modifier_entry`, all `Background`/`*_bg`/`*_frame`/`niche`/`idea_traits`/`chevrons`/`creator_flag frames`.
- **Army-badge chrome / mutation:** `army_bg`/`army_group_bg`/`army_item_bg`/`div_count_bg`/`leader_portrait_frame`/
  `skill_bg`/`leader_stat_*_bg`, `leader_button`/`leader_details_button`/`assign_staff_office`/`select_all_button`/
  `army_motorization_priority_button`(+overlay)/`army_railway_gun_attachment_button`/`toggle_armies_divisions_button`/
  `select_all_wings_button`/`army_color_button`/`unassign_army_button`/`group_name` editBox/`change_group_marker`
  (all mutation/nav verbs), `army_badges_window` slider, `divisions_summary` (OverlappingElementsBox — structural),
  `leader_enroute`/`unit_leader_tutorial_glow`/`click_to_assign` (cosmetic/empty-state).
- **Structural containers / positionTypes:** `armies_view`, `army_division_view`, `army_division_stats_view`,
  `ship_stats_window`, `info`/`base_stats`/`combat_stats`/`misc_stats`, `subunits`/`subunits_hq`/`hq_modifiers_list`,
  `equipment`/`adjuster_container`/`adjuster_stats`, `captain_window`/`air_wings`/`terrain`/`history`, all
  `positionType` anchors (`select_all_button_army_group_position`, `carrier_air_wings_list_position`,
  `taog_carrier_air_wings_list_position`, `divisions_summary_max_item_count`, `row_padding`, …), and every
  row-template root.

---
---

# EVENTS / POPUPS / NOTIFICATIONS (Views 7–11)

> **The defining limit (PLAN §C.4 "Events / popups").** The replay's event timeline (SCHEMA §9.2) carries
> **fired-event IDs exactly** (`event_fired` `key`=`to`= scripted-event id, derived from
> `world_events.fired_events`). But the in-game event WINDOW renders the event's **TITLE, DESCRIPTION,
> PICTURE, and per-OPTION text** — none of which are in the snapshot. Those come from a **STATIC
> DEF/WORLD-DATA parse** of the mod's `events/*.txt` (the `title`/`desc`/`picture`/`option` keys) +
> the localisation `.yml` for the loc keys. That parse is a **separate adapter (gap closure bucket (c)
> STATIC DEF)** not yet built. So: event ENUMERATION (which event fired, when, to whom) = **exact/derived**;
> event TEXT/options = **unavailable** until the event-file+loc adapter exists. And in THIS demo
> `world_events` itself is unfed (ARTIFACT) → even the fired-id list is "—".

---

# VIEW 7 — EVENT WINDOW (`game_assets/gui/event_window.json`)

Source `interface/eventwindow.gui`. Four event-window variants (`EventWindow`, `EventWindow_Operative`,
`EventWindow_leader`, `EventWindow_News`) sharing the same shape, + one ROW template (`event_option_entry`).

## 7a. Event window — dynamic (would-bind)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `Title` (instantTextBoxType, all 4 variants) | event def `title` loc key (keyed by fired-event id from `world_events.fired_events`) | loc | **unavailable** (TRUE GAP + ARTIFACT) | Title TEXT needs event-file parse + loc (bucket (c)); fired-id list also unfed in demo. |
| `Description` (instantTextBoxType, all 4) | event def `desc` loc key | loc | **unavailable** (TRUE GAP + ARTIFACT) | As above. |
| `event_picture` (iconType, `GFX_report_event_001` / `GFX_news_event_001`) | event def `picture` → `GFX_*` | category (event→sprite) | **unavailable** (TRUE GAP) | Picture key from the event def. |
| `options_grid` (gridBoxType, all 4) → `event_option_entry` rows (`Name` instantTextBox) | event def `option[].name` loc keys | row factory; loc | **unavailable** (TRUE GAP) | Per-option button text from the event def. The option buttons are mutation verbs anyway (pick option) → in a replay they'd show the historical CHOICE (derivable from the consequent diff) but the option TEXT needs the def parse. |
| `EventWindow_Operative` → `leader` (button), `leader_fullname` / `leader_codename` (instantTextBox), `agency_insignia` (iconType) | the operative the event is about → `strategic_operatives` / `intel` | loc + portrait | **unavailable** (current data) | Operative identity needs the intel/operatives domain (absent); + the event def. |
| `EventWindow_leader` → `leader` (button, `GFX_leader_unknown`) | event-subject leader portrait | portrait | **unavailable** (current data + TRUE GAP) | Portrait asset + which leader (from the event def's scope). |

## 7b. Event window — STATIC / decoration / navigation / mutation

- **Chrome / frames:** all `Background` (`GFX_event_report_top_win`/`_midsection`/`_bottom_win(_end)`,
  `GFX_event_operative_background`, `GFX_event_news_bg`, `GFX_tiled_window_transparent`), `event_pic_clip`,
  `event_picture_overlay`, `paper_clip_2`, `leader_frame`, `event_option_background`.
- **Navigation / mutation (inert):** `btn_minimize` (×4), the `options_grid` option buttons (pick = mutation),
  `leader` portrait buttons (open leader = nav).
- **Structural containers:** `EventWindow`/`_Operative`/`_leader`/`_News`, `top_Window`/`midsection`/
  `bottom_Window`/`bottom_window_end`, `leader_picture_container`/`leader_details_container`,
  `event_option_entry` (row root).

---

# VIEW 8 — POPUP WINDOW (`game_assets/gui/popup_window.json`)

Source `interface/popupwindow.gui` — the **generic popup LIBRARY**: 32 distinct top-level
`containerWindowType` popup types + a few embedded ROW templates. These are **transient runtime
notifications/dialogs**. The replay surfaces the *outcomes* these announce via the diff-event timeline
(SCHEMA §9.2), so most popups are best rendered as **derived from an event**, not bound to a stored popup
object. Each popup's TITLE/DESCRIPTION/MESSAGE is generic scripted text (loc key supplied at fire time) →
**unavailable** unless the firing event/decision is reconstructed. I group the 32 by binding pattern.

## 8a. Popup types — derivable-from-event (the notification class)

These announce a world EVENT the diff engine already derives. The popup's flags/title can be `derived`
from the corresponding event; the body MESSAGE text is the scripted string (loc) → `unavailable`.

| popup (containerWindowType) | maps to event (SCHEMA §9.2) | bindable sub-elements | fidelity |
|---|---|---|---|
| `surrendered_country_popup` | `capitulated` / `country_removed` | `sender_flag`/`receiver_flag` (the two TAGs), `equipment_items` (seized equipment OverlappingBox → `equipment_special_item` rows) | flags **derived**; seized-equipment list **unavailable** (per-capitulation seizure not stored); `message`/`title` **unavailable** (scripted loc) |
| `focus_finished_popup_window` | `focus_completed` | `symbol` (`GFX_goal_unknown` → focus icon), `focus_name` (loc) | `derived` (focus id → icon/name from focus-DB); `title` **unavailable** |
| `research_finished_popup_window` | `tech_researched` | `tech_icon`, `tech_name` (loc) | `derived` (tech id → icon/name); the design-team/facility/carrier icons **unavailable** |
| `facility_captured_popup_window` | `state_controller_change` (w/ facility) | `from_country_icon` (flag), `facility_icon` (building → sprite), `description` | flags/building **derived**; `description` **unavailable** (scripted) |
| `new_exiled_hosted_popup` / `_small` / `exile_country_popup` | `subject_created` (exile) / `country_removed` | `sender_flag`/`receiver_flag`, `equipment_items` | flags **derived**; equipment/message **unavailable** |
| `political_goal_popup_window` | `focus_completed` (political) / decision | `goal_icon`, `title`/`description`/`reward` (loc) | icon **derived**; text **unavailable** (scripted) |
| `project_research_completed` / `unlock_industrial_org_trait_popup` | `project_completed` / `mio_leveled` | `icon`, `title`/`description` (loc) | icon **derived**; text **unavailable** |
| `peace_summary_popup_window` | `peace` | `title`/`description` (loc) | **unavailable** (scripted peace-summary text) |

**Embedded ROW templates** used by the above: `equipment_special_item` / `equipment_item` (seized/normal
equipment: `equipment_name` loc + `equipment_icon` + `equipment_amount` int) → **unavailable** (per-event
seizure list not stored). `equipment_lend_lease_entry` / `popup_lend_lease_info_item` (lend-lease deal:
`name`/`count`/`deal_description`/`deal_state`) → maps to `diplomacy_world.trade_contracts` (SCHEMA §7b) /
`lend_lease_started` event; **unavailable** in demo (`trade_contracts` absent — ARTIFACT).

## 8b. Popup types — DEFERRED (mutation dialogs / session control — inert in replay)

Every popup whose buttons PERFORM an action or whose existence is a live-session artifact is inert. The
title/message would be scripted loc (unavailable) even if shown.

| popup (containerWindowType) | why deferred / inert |
|---|---|
| `diplomacy_standard_popup_window`, `diplomacy_foreign_manpower_popup_window` (+ `manpower_slider`), `diplomacy_incoming_lend_lease_popup_window` (+ `entries.list`), `popup_lend_lease_request_item` (+ `value_box`/`equipment_variants`), `diplomacy_scripted_popup_window` / `send_diplomacy_scripted_popup_window` (+ `scripted_gui_container`) | **diplomatic-action MODALS** → `accept_button`/`decline_button`/`confirm_button`/`send` all mutation verbs. (Lend-lease deals surface via `trade_contracts` + `lend_lease_*` events instead.) |
| `confirmation_popup_window`, `default_confirmation_popup`, `default_info_popup_window`, `release_country_dialog` (+ puppet/play-as/retain-cores checkboxes), `aicontroller_popup_window`, `dismiss_operative_popup`, `air_wing_assignment_popup` | **confirm/info/action MODALS** → mutation verbs (release country, dismiss operative, assign air wing, take AI control). |
| `oos_popup_window` (+ `resync_button`), `connection_lost_popup_window` (+ `save_button`) | **MULTIPLAYER-SESSION control** (out-of-sync resync, connection lost) — meaningless in a replay; inert. |
| `custom_naval_invasion_warning` (+ `show`/`disable`), `custom_paradrop_warning` (+ `disable`) | **live alert popups** — `sender_flag`/`receiver_flag` would be `derived` from a combat/landing event, but the `ok`/`show`/`disable` buttons are live-UI verbs; in a replay the underlying landing surfaces via combat/province-control events. |

## 8c. Popup window — STATIC / decoration

- **Chrome / frames (shared across the 32):** all `Background` (`GFX_tiled_bg`, `GFX_pol_goal_popup_bg`,
  `GFX_popup_capitulation_bg`, `GFX_popup_new_exile_hosted(_small)_bg`, `GFX_tiled_window_pol_goal`,
  `GFX_event_news_bg`, `GFX_agency_popup_bg`, `SandboxFloaterBackground`, `GFX_lendlease_*_bg`,
  `GFX_mini_tooltip`, `GFX_generic_text_bg_60`), `diplo_popup_header_bg` (+ `_large`), `diplo_popup_bottom`
  (+ `_large`), `diplo_war_large_icon`(×2), all `*_flag_frame` (`GFX_large_flag_frame`/
  `GFX_diplo_countrylist_flag_frame`), `*_leader_frame`, `pc_decoration_top_left/right`, `focus_popup_bg`,
  `paper_background`.
- **Loc labels (static):** `release_puppet_label`/`play_as_label`/`retain_cores_label`/`update_variants_description`,
  `slider_description`, `RELEASE_AS_PUPPET`/`PLAY_AS`/`RETAIN_CORES`/`NAVAL_ACCESS` etc.
- **Structural containers:** `manpower_slider_container`, `scripted_gui_container`/`bottom_container`,
  `paper_background`, `entries`, `equipment_variants` (+ `expanded_window`), the embedded row roots.

---

# VIEW 9 — DETAILED WINDOWS (`game_assets/gui/detailed_windows.json`)

Source `interface/detailedwindows.gui`. Two windows — `shipswindow` (equipment-in-use ship breakdown) and
`planeswindow` (equipment-in-use plane breakdown) — + 4 ROW templates (ship/plane archetype headers +
equipment entries).

> These are "equipment in use" drill-downs (the `EQUIPMENT_IN_USE_HEADER`). **Ship** breakdown maps to
> `military.ships_by_hull` (SCHEMA §3.2) — **unfed in demo (ARTIFACT)**. **Plane** breakdown maps to the
> air-equipment subset of `military.equipment_stockpile[]` + air-wing assignment (needs `order_of_battle`
> air wings, absent).

## 9a. Detailed windows — dynamic (would-bind)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `shipswindow.total_ships` (instantTextbox) | `military.num_ships` | int | **unavailable** (current data) | `num_ships` present in schema; **0/143 in demo** → "—". |
| `shipswindow.current_manpower` | ship crew manpower | int | **unavailable** | Not split per-domain in schema. |
| `shipswindow.ships_grid` (gridBox) → `ship_archetype_header_entry` + `equipment_ship_entry` rows | `military.ships_by_hull` `{hull:{count,tonnage}}` | row factory | **unavailable** (current data) | `ships_by_hull` null in demo (ARTIFACT). Row: `archetype_name`(loc)+`archetype_amount`(int) header; `equipment_ship_entry` `name`+`amount`. |
| `planeswindow.total_planes` (instantTextbox) | sum of air equipment in use | int | **derived** | From `military.equipment_stockpile[]` air archetypes; **unavailable** in demo (stockpile not folded). |
| `planeswindow.equipment_ratio` | filled-vs-needed air equipment | percent | **unavailable** | Need not modelled. |
| `planeswindow.planes_grid` (gridBox) → `plane_archetype_header_entry` + `equipment_plane_entry` rows | air subset of `military.equipment_stockpile[]` | row factory | **derived** | Per-archetype plane counts; **unavailable** in demo. |

## 9b. Detailed windows — STATIC / decoration

- **Chrome / icons / labels:** `Background` (`GFX_tiled_window2_1b_border`, `GFX_unit_stats_list_entry_bg2`,
  `GFX_mini_tooltip`), `template_header_bg`, `total_ships_icon`/`total_planes_icon`/`manpower_icon`/
  `equipment_icon`/`airwing_carrier_icon`, `template_title` (`EQUIPMENT_IN_USE_HEADER` loc), `equipment_icon`
  (`GFX_equipment_naval_item`).
- **Structural containers:** `shipswindow`/`planeswindow`, `ships`/`planes`, and the 4 row-template roots.

---

# VIEW 10 — CORE (`game_assets/gui/core.json`) — SHARED PROTOTYPE LIBRARY

Source `interface/core.gui`. This is the **shared widget / scrollbar / tooltip PROTOTYPE library** — the
base templates other views `inherit`/reference. It defines almost no data-bound surface; it is the chrome
toolkit. **Documented as the shared prototype set; near-zero data binding.**

## 10a. Core — the prototype set (static chrome — NO data binding)

- **Scrollbar prototypes (the reusable scroll widgets):** `standardlistbox_slider`,
  `standardlistbox_horizontal_slider`, `right_vertical_slider(no_bg)`, `right_cabinet_vertical_slider`,
  `left_vertical_slider`, `bottom_horizontal_slider`, `career_profile_vertical_slider`, `standardtext_slider`,
  `standardlistbox_noslider` (each with `slider`/`track`/`Decrease`/`Increase` or
  `listboxSliderButton`/`listboxTrackButton`/`upButton`/`downButton`). Pure UI controls — instantiated into
  other views; no datum.
- **Console / debug prototypes:** `console_wnd` (`console_list` + `console_edit`), `console_entry_wnd`
  (`console_text`), `FPS_Counter`, `error_log_win` (`this_is_dog` + `error_log_button`), `slow_interface_win`
  (`slow_text` `SLOW INTERFACE`) — developer overlays, never part of the replay UI.
- **Shared widget prototypes:** `ChatMemberText`, `ToolTip` (the base tooltip text box — referenced by the
  tooltip engine, PLAN §B.8), `countryflag_entry` (the reusable `flag` + `small_flag_frame_thin` row — this
  IS a row factory used by many views; when instantiated its `flag` binds to a TAG → `derived`, but the
  prototype itself carries no datum), `dark_overlay` (`GFX_in_game_menu_dark_overlay` — the modal dimmer).

## 10b. Core — the ONE conditionally-bindable prototype

| element | source | fidelity | notes |
|---|---|---|---|
| `countryflag_entry.flag` (buttonType, `GFX_flag_small2`) | a TAG supplied by the host view at instantiation | flag-small | **derived** (when instantiated) | The shared flag-row prototype. Standalone (in `core.gui`) it has no datum; each consuming view supplies the TAG. Listed so the prototype isn't mistaken for dead. |

**Everything else in `core.json` is static prototype chrome** (no dynamic-bound, no deferred-mutation —
these are templates, not live dialogs). Element accounting: 1 conditionally-derived prototype, 82 static
prototype/chrome.

---

# VIEW 11 — INGAME MENU (`game_assets/gui/ingame_menu.json`)

Source `interface/ingamemenu.gui` — the **pause / escape menu** + save/load sub-screens. In a replay this
is **scrubber-relevant chrome but almost entirely inert/navigation**: the replay has its own
playback/scrubber UI; the in-game pause menu's verbs (save/load/resign/quit/settings) are meaningless.

## 11a. Ingame menu — dynamic (the only data-bearing surface: the save-list)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `savegame_item` ROW → `player_shield` (iconType, `GFX_flag_small2_not_masked`) | the save's player TAG | flag-shield | **derived** | One row per save file in `games_grid`. In a replay this list is the host filesystem's saves, NOT snapshot data → effectively **out-of-scope** (real OS save dir). Listed for completeness. |
| `savegame_item` → `name` / `date` (instantTextBox) | save file name / save date | loc / date | **derived** | From the save-file metadata (filesystem), not the snapshot. |
| `disk_quota` / `saves_dirs` (instantTextBox) | storage usage / save directory | loc | **derived** (OS) | Filesystem, not snapshot. |

> All of these read the **host filesystem**, not the replay snapshot — so within the replay-binding model
> they are **deferred/out-of-scope** (the replay would either hide this menu or repurpose it as a "load
> different recording" picker). No snapshot field binds here.

## 11b. Ingame menu — STATIC / decoration / navigation / mutation (everything else)

- **Menu buttons (all mutation/nav verbs — inert in replay):** `menu_load_button`, `menu_save_button`,
  `menu_gamesettings_button`, `menu_playthrough_stats_button` (+ `..._star`), `menu_wiki_button`,
  `menu_resign_button`, `menu_quit_button`, `menu_close_button`, `achievements_button`, and the social-link
  buttons `homepage`/`forum`/`discord`/`privacy`/`bluesky`/`x`/`facebook`/`instagram`/`youtube` (open URL).
- **Save/load sub-screens (`ingamemenu_load` / `ingamemenu_save`):** `localgames_tab`/`remotegames_tab`
  (checkbox tabs), `load`/`save`/`cancel`/`delete` (mutation verbs), `game_name` editBox, `localgames_label`/
  `cloudgames_label`/`filename_label` (loc), `menu`/`menu_bottom` chrome, `cloud_file`/`ironman_icon`/
  `tutorial_icon`/`missing_dlc_sign` (per-save status icons → derived from save meta, OS-scope).
- **Chrome / structural:** `Background` (`GFX_tiled_window`, `GFX_no_frame_special_bg`),
  `player_shield_overlay`/`player_shield_frame`, containers `ingamemenu`/`first_row`/`second_row`/
  `ingamemenu_load`/`ingamemenu_save`/`games`/`savegame_item`, `games_grid` (row factory root).

---
---

## Element accounting (provably complete, per view)

| # | view | total elements (tree-walk) | dynamic-bound | static/decoration | deferred (mutation/runtime/OS) |
|---:|---|---:|---:|---:|---:|
| 1 | `logistics.json` | 208 | ~22 (materiel/resource/fuel/factory rows + nuke) | ~150 | ~36 (close/chart-toggles/prio/delete buttons) |
| 2 | `garrison_log.json` | 57 | ~9 (flags, activities rows, period, eq totals) | ~38 | ~10 (tabs/filter/close/slider) |
| 3 | `naval_combat_results.json` | 115 | ~20 (leader flags, outcome, survivor/lost rows ×16 templates) | ~80 | ~5 (open_details/close/dismiss) |
| 4 | `destroyed_ships.json` | **0** (present-but-empty stub) | 0 | 0 | 0 (runtime-instanced shell — see VIEW 4) |
| 4b | `naval_losses.json` | 78 | ~18 (totals, ship/archetype rows ×7 templates, filter rows) | ~52 | ~8 (close/back/dropdowns/battle-btn) |
| 5 | `naval_strategic_region.json` | 45 | ~16 (region name, own/enemy ship/convoy/plane/mine ×8, dominance bar+flag, modifiers) | ~20 | ~9 (close/logbook/access-rule mutations) |
| 6 | `unit_view.json` | 539 | ~55 (division row, equipment rows, template/stat grids, army badge, ship panel) | ~330 | ~150 (color picker, expeditionary, sort/deploy/rename/medal mutations) |
| 7 | `event_window.json` | 78 | ~14 (title/desc/picture/options ×4 variants, operative/leader) | ~50 | ~14 (minimize ×4, option/leader buttons) |
| 8 | `popup_window.json` | 387 | ~40 (event-derived flags/icons/titles across the notification-class popups + lend-lease/equipment rows) | ~210 | ~137 (mutation modals, MP-session control, +/− sliders, accept/decline) |
| 9 | `detailed_windows.json` | 53 | ~14 (ship/plane totals + archetype/equipment rows ×4 templates) | ~39 | 0 |
| 10 | `core.json` | 83 | 1 (countryflag_entry.flag prototype) | 82 | 0 |
| 11 | `ingame_menu.json` | 77 | ~6 (save-list rows — OS-scope) | ~25 | ~46 (menu/save/load/social verbs) |

**Total bindable (dynamic-bound) elements across the 11 non-empty views: ~214** (the remainder are static
chrome or deferred mutation/runtime/OS verbs). Of those ~214, in the **current demo snapshot** only the
**unit_view division/equipment rows (~30 cells) bind EXACT/derived with real data** (org/strength/equipment/
template/owner/moving) + a handful of logistics resource cells (`economy.resources` exact); everything else
is "—" pending a domain fold (ARTIFACT) or is a permanent PROXY/GAP.

---

## SCHEMA-GAP LEDGER (ARTIFACT vs TRUE GAP + closure bucket)

> Buckets: **(a) LIVE-EXTRACTABLE** (a live read fills it), **(b) COMPUTED-NOT-STORED** (Ghidra tier —
> stays a wall), **(c) STATIC DEF/WORLD DATA** (parseable from game `common/` / `events/` / `interface/` + loc).

### ARTIFACTS (unfed in THIS demo; become bound once the domain is folded)

| gap (view) | schema field (correct path) | closure | once folded → |
|---|---|---|---|
| logistics fuel-priority / truck / network (V1) | `countries.<TAG>.economy.logistics.*` (supply_system_2) | fold `supply.json` (a) | **exact** (priority/buffer/truck/node counts) |
| materiel grid stock/produced/balance (V1) | `military.equipment_stockpile[]` + `economy.gross_ic_by_equipment` | production-delta + stockpile fold (a) | **derived** |
| nuke count (V1) | `extra.nukes_total` | fold `country_extra.json` (a) | **exact** |
| per-province supply (V1/proxy) | `supply.by_province{current,capacity,gain,divisions}` | fold live PROXY (a) — **but PROXY badge stays** | **proxy** |
| sea-zone dominance / air-sup (V5) | `naval_air.{region_control,air_superiority}` | fold live PROXY (a) — **PROXY badge stays** | **proxy** |
| convoy/ship-loss ledgers (V3/4/4b) | `world_events.{sunk_convoys,convoy_losses_by_killer,convoy_losses_by_owner}` | fold `events.json` (a) | **exact** (convoy side; owner/killer/count) |
| ship-hull breakdown (V4b/V9) | `military.ships_by_hull{hull:{count,tonnage}}` | parse fold (a) | **derived** |
| garrison/manpower totals (V2) | `military.casualties.{killed,captured}` | compute/fold (a) | **derived** |
| fired-event id list (V7/V8) | `world_events.fired_events[]` | fold `events.json` (a) | **exact** (the ID enumeration) |
| division dig-in/supply/manpower/accl/kills (V6) | `divisions[].{dig_in,dig_in_cap,max_supply,army_current_supply_ratio,supply_gain,army_manpower,acclimatization,killed,division_name,motorization_level,has_officer,last_combat_date}` (divisions_full) | fold `divisions_full.json` (a) | **exact** (literal save fields) |
| lend-lease deal rows (V8) | `diplomacy_world.trade_contracts[]` | fold `market.json` (a) | **exact** |
| ship/air OOB detail (V6 ship panel, V9 air wings) | `order_of_battle.fleets[].task_forces[].ships[]` (+ air wings) | fold OOB domain (a) | **exact** (identity/org/str) |

### TRUE GAPs (stay `unavailable` regardless of folding)

| gap (view) | why | bucket |
|---|---|---|
| **EXACT per-province supply NUMBER** (V1/V5 tooltip value) | the `supply.by_province` PROXY gives where-troops-are current/capacity, but the **exact tooltip supply figure is COMPUTED-NOT-STORED** — the documented wall (`replay/SCHEMA.md §13.2`, `memory_reader/supply_solver_offsets.draft.json`, `RESEARCH_logistics_intel.md`) | **(b)** |
| combat progress bar / combat width / terrain / tactics / modifiers (V3 outcome detail, V6 unit_land_combat bar) | runtime battle-detail (§G) | **(b)** |
| per-division EFFECTIVE combat stats (soft/hard/defense, V6 info grids) | runtime-computed from equipment+modifiers (§G) — template BASE stats are derivable (c), effective are not | **(b)** |
| naval region weather / mine coverage / enemy-ship range estimate (V5) | sea-zone weather + fog-of-war estimates — computed/intel-scaled, not stored | **(b)** |
| per-battle casualty/loss ledger, battle OUTCOME flag (V3 outcome, V2 per-attack loss) | not stored as a ledger (combat reconstructed as `inferred`, §9) | **(b)** |
| per-WARSHIP destruction log + per-ship NAME + accident/mine attribution (V4/V4b) | only the per-month CONVOY ledger is stored; warship sinkings + ship names + training/mine cause are not stored at per-ship granularity | **(b)** |
| event TITLE / DESCRIPTION / PICTURE / OPTION text (V7, V8 popup bodies) | the snapshot has the fired-event ID, not the rendered text | **(c)** — parse `events/*.txt` `title`/`desc`/`picture`/`option` + loc `.yml` |
| equipment / resource / building / template NAMES + icons (V1/V4b/V6/V9 loc labels) | id→name/sprite is a static catalog lookup | **(c)** — equipment/resource/building DB + loc |
| garrison-template / occupation-law catalog (V2) | static def | **(c)** |

---

## Historical review round (adversarial review — 2026-06-26)

Historical note: this records a completed adversarial review from the old
Claude-era workflow. Codex-native sessions must use normal sub-agents/partner
review and must not run the old local CLI command. Asked: (a) the empty
`destroyed_ships` handling, (b) field-path correctness, (c) false `unavailable` (over/under-pessimism),
(d) artifact-vs-gap split + the supply-exact-number-is-a-wall claim, (e) division EXACT badge under
memory-channel-only comparability. The first call (full prompt, file-reading allowed) **timed out at 5min**
while the review was still grepping the schema/doc; the retry (lean prompt, no-file-read, all facts supplied
inline) returned a verdict in-budget.

| ask | Codex verdict | action |
|---|---|---|
| (a) destroyed_ships present-but-empty + would-bind | **CONFIRM-WITH-CORRECTION** — the empty stub is itself an ARTIFACT/placeholder; it justifies the unavailable badge but does **not** by itself bind `sunk_convoys`/`ships_by_hull` until those domains are fed. "Empty `.gui`" ≠ "no losses". | **Folded** — added an explicit note to VIEW 4 sharpening that the binding materialises only on fold + instancing. |
| (b) field paths | **CONFIRM** — paths internally consistent; reiterated `naval_air.region_control`/`air_superiority` are PROXY, not exact controls. | No change (already PROXY-badged). |
| (c) over/under-pessimism | **CONFIRM-WITH-CORRECTION** — division badges right (present-non-null = exact; present-null = unavailable; absent divisions_full = unavailable-ARTIFACT); domain ARTIFACT badges right. **Do NOT mark unavailable:** `economy.fuel.current/max` (fuel bar binds now), `economy.resources` (resource rows exact), `diplomacy_world.factions` (faction list exact). | **Folded** — the doc already binds resources `exact` (V1 1a/1b) and faction list `exact` (V4b 4b-a); **fixed** the V1 aggregate fuel bar from unavailable → `derived` (`general_info.fulfilled` ← `fuel.current/max`). |
| (d) supply ARTIFACT-when-absent + PERMANENT proxy, exact tooltip number = separate TRUE GAP (B2 wall) | **CONFIRM** — good split. | No change. |
| (e) division value EXACT despite channel-only comparability | **CONFIRM** — badge the displayed reading EXACT (real current value); caveat that cross-channel diffs/deltas are unsafe (monotone within one channel only). | No change (doc already carries the channel caveat on `bar_org`/`bar_str`). |
| naval_strategic_region host for region_control | **CONFIRM** — plausible/right host. | No change. |
| mutation dialogs / static-vs-bound | **CONFIRM** — don't bind mutation actions as replay state unless they DISPLAY read-only state; keep decorative labels/icons/frames/tabs static unless backed by a real schema field. | No change (matches the read-only-verb policy applied throughout). |

**Net: 1 substantive fix** (V1 aggregate fuel bar derived-not-unavailable) + 1 sharpening note (V4 empty-stub
semantics). No path errors, no false gaps, no mis-bound mutation dialogs found.
