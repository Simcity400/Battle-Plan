# BINDING MAP — TOPBAR + STATE/PROVINCE panel (M2/M3/M5)

> **What this is.** The COMPLETE, no-truncation data-binding map for the two in-game the game views
> the native replay UI binds in the vertical slice: the **topbar** (`game_assets/gui/topbar.json`,
> from `interface/topbar.gui`) and the **state/province panel**
> (`game_assets/gui/province_state.json`, from `interface/countrystateview.gui`). It realises
> GAME_UI_REPLICA_PLAN §C — "comprehensive but mechanical binding sweep" — for these two surfaces:
> EVERY element in each `.gui` tree is accounted for, either as a dynamic (data-driven) binding or
> as static/decoration with no binding. Nothing is omitted; where a `.gui` element exists but our
> snapshot has no field for it, the element is listed and **badged `unavailable`** with the missing
> field named.
>
> **Binding-rule shape** (GAME_UI_REPLICA_PLAN §C.1): `{ target (gui element name+type), source
> (view-model / snapshot path), format (formatter id), fidelity (exact|derived|proxy|unavailable) }`.
> **Source paths** use SCHEMA dotted form: `countries.<TAG>.…` and `states.<id>.…` (see
> `replay/SCHEMA.md` §3 / §4). `<TAG>` = the player/observed country; `<id>` = the selected state id.
>
> **No-fabrication (SCHEMA §0.2, PLAN principle 3):** `null` renders as "—", never `0`. `0`/`0.0`
> is a measured zero. A field that is schema-present but never fed by the current parser is badged
> `unavailable` (not silently dropped).

---

## Provenance of the fidelity calls

Calls below are grounded in **empirical inspection of 3 real save-backed snapshots**
(`replay/snapshots/{replay,campaign,ger_ddr_1946}/*.snapshot.json`, `schema_version "1"`,
GER 1941 save channel) cross-checked against `replay/SCHEMA.md`, and **adversarially reviewed by
Codex** (`gpt-5.5`, read-only, 2026-06-26). Codex AGREED on items 1–6 and 8 and **refuted item 7**
(state-panel manpower → `manpower_pool.total`, not `.available`); all corrections are folded in.

**Empirically confirmed in the current data (drives the `unavailable` badges):**

| schema field | populated in current snapshots | consequence |
|---|---|---|
| `states.<id>.resources` | **0 / 1081** (always `null`) | state resource rows → `unavailable` |
| `states.<id>.victory_points_by_province` | **0 / 1081** (always `null`) | per-province VP → `unavailable` |
| `states.<id>.occupation_law` | **0 / 1081** (always `null`) | occupation-law icon → `unavailable` |
| `states.<id>.supply` | **0 / 1081** (always `null`) | (no state-panel binding needs it) |
| `states.<id>.is_core_of` / `is_claimed_by` | **0 / 1081** (`[]`) | claims/cores overlapping boxes → `unavailable` |
| `countries.<TAG>.production.*` | **key does not exist** | factories come from `economy.factories.*`, NOT `production.*` (slice-spec path is wrong) |

**Confirmed PRESENT & usable:** `economy.factories.{civ,mil,dockyard,under_construction}`,
`economy.fuel.{current,max}`, `military.manpower.available`, `military.convoys[]` (list),
`politics.{political_power,stability,war_support,command_power,ruling_party,faction}`,
`states.<id>.{owner,controller,buildings,province_buildings,victory_points_total,manpower_pool,
category,occupation,demilitarized}`.

---

# VIEW 1 — TOPBAR (`game_assets/gui/topbar.json`)

Root: `containerWindowType "topbar"`. All bindings observe the player/observed country `<TAG>`;
the clock binds the snapshot scalar. Formatters referenced: `int`, `int-triple`, `unsigned-int`,
`percent` (×100), `manpower` (compact people), `fuel` (raw float, `849K` style),
`progressbar-fraction` (0..1), `flag` / `flag-shield` (ideology-tinted maskedShield), `clock`
(Clausewitz date→`H:00 D, Month YYYY`).

## 1a. Dynamic (data-bound) topbar elements

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `player_flag` (buttonType, `GFX_shield_medium`) | `<TAG>` + `countries.<TAG>.politics.ruling_party` | flag-shield (ideology-tint) | **exact** | Shield sprite resolved from TAG; ruling_party (`"fascism"`) drives the ideology tint frame. Click = drill to country view (read-only verb). |
| `pol_power` (instantTextBoxType, placeholder `999`) | `countries.<TAG>.politics.political_power` | unsigned-int | **exact** | Stored stock value (GER=247.37→`247`). **Codex:** formatter is unsigned/rounded **int, not signed** — daily ± delta is not stored. |
| `pol_power_icon` (iconType, `GFX_pol_power_icon`) | — (static icon) | — | static | Always-on PP icon; decoration for `pol_power`. |
| `pol_power_value_bg` (iconType, `GFX_generic_box_smallest`) | — | — | static | Box chrome behind PP. |
| `stability` (instantTextBoxType, placeholder `"100%"`) | `countries.<TAG>.politics.stability` | percent (×100) | **exact** | 0..1 float (0.75→`75%`). **No progressbar element exists** for stability in this topbar.gui (see note A). |
| `stability_icon` (iconType, `GFX_stability_icon`) | — | — | static | Stability icon. |
| `sability_value_bg` (iconType, `GFX_generic_box_smallest`) | — | — | static | Box chrome (sic: "sability" — vanilla typo). |
| `war_support` (instantTextBoxType, placeholder `"100%"`) | `countries.<TAG>.politics.war_support` | percent (×100) | **exact** | 0..1 float (0.85→`85%`). No progressbar element (note A). |
| `war_support_icon` (iconType, `GFX_war_support_icon`) | — | — | static | WS icon. |
| `war_support_value_bg` (iconType, `GFX_generic_box_smallest`) | — | — | static | Box chrome. |
| `manpower` (instantTextBoxType, placeholder `9999`) | `countries.<TAG>.military.manpower.available` | manpower (compact) | **exact** | Raw people int (525000→`525.00K`). Deployable reserve. |
| `manpower_icon` (iconType, `GFX_manpower_icon`) | — | — | static | |
| `manpower_value_bg` (iconType, `GFX_generic_box_96`) | — | — | static | |
| `industrial_capacity` (instantTextBoxType, placeholder `"12/12/12"`) | `countries.<TAG>.economy.factories.civ` + `.mil` + `.dockyard` | int-triple (`civ/mil/dock`) | **exact** | Three measured save totals (110/131/16). **Slice-spec path `production.{civilian,military,dockyard}_factories` is WRONG — no `production` key exists**; correct path is `economy.factories.*`. Totals include offmap per SCHEMA §3.1. |
| `industrial_capacity_icon` (iconType, `GFX_industrial_capacity_icon`) | — | — | static | |
| `industrial_capacity_bg` (iconType, `GFX_generic_box_smallest`) | — | — | static | |
| `industrial_ratio_bar` (iconType, `GFX_topbar_fuel_bar`) | `countries.<TAG>.economy.factories.under_construction` vs total slots | progressbar-fraction | **derived** | In-game = consumer-vs-available IC ratio. We have `under_construction`=12 and total factories but **not** the consumer/free split → render as a derived approximation or badge approximate. `under_construction` itself is **exact**. |
| `fuel_value` (instantTextBoxType, placeholder `"999d"`) | `countries.<TAG>.economy.fuel.current` | fuel (raw float, `849K`) | **exact** (amount) | Stored fuel AMOUNT (849000.0). **Codex:** value is sane decoded units (NOT `/1000`). The in-game `Nd` **days-remaining** variant is **derived/unavailable** (needs a consumption rate we don't store). |
| `fuel_icon` (iconType, `GFX_fuel_state`) | — | — | static | |
| `fuel_value_bg` (iconType, `GFX_generic_box_smallest`) | — | — | static | |
| `fuel_ratio_bar` (iconType, `GFX_topbar_fuel_bar`) | `economy.fuel.current` / `economy.fuel.max` | progressbar-fraction | **exact** | current/max (849000/849000 = full). |
| `convoys_count` (instantTextBoxType, placeholder `0`) | sum of `countries.<TAG>.military.convoys[].amount` | int | **derived** | `military.convoys` is a **list** of equipment-stockpile entries (`{archetype:"convoy_1", amount:1147.0}, …`), not a scalar — we sum `.amount`. The in-game number is the same aggregate but it is computed, not stored. |
| `convoys_icon` (iconType, `GFX_topbar_convoys`) | — | — | static | |
| `convoy_text_bg` (iconType, `GFX_generic_box_smallest`) | — | — | static | |
| `convoys_ratio_bar` (iconType, `GFX_topbar_fuel_bar`) | (free vs assigned convoys) | progressbar-fraction | **unavailable** | We store total convoys only; **assigned/free split not stored** → no fraction. Render bar inert/badged. |
| `DateText` (instantTextBoxType, placeholder `"7:00 29, September 1939"`) | `snapshot.date` (`countries`-independent; top-level) | clock | **exact** | Clausewitz `"1941.4.13.11"` → `11:00 13, April 1941`. Drives the scrubber spine. |
| `supply_value` (instantTextBoxType, placeholder `"999d"`) | (national equipment/supply scalar) | int/days | **unavailable** | This topbar slot (`GFX_equipment_icon`) is the equipment/supply topbar readout; **no matching national supply scalar is stored** (`supply.by_province` is a region proxy, not this number). Badge unavailable. |
| `supply_icon` (iconType, `GFX_equipment_icon`) | — | — | static | |
| `supply_value_bg` (iconType, `GFX_generic_box_smallest`) | — | — | static | |
| `supply_ratio_bar` (iconType, `GFX_topbar_fuel_bar`) | (supply fraction) | progressbar-fraction | **unavailable** | Same as `supply_value`. |
| `experience_value` (instantTextBoxType, in `land`) | `countries.<TAG>.military.xp.army` | int | **unavailable** (current data) | SCHEMA §3.2 has `xp.army` (S); but `military.xp` is **`{}` in all 3 snapshots** → null/unavailable now. Schema-present, parser-unfed. |
| `experience_icon` (iconType, `GFX_army_experience`, in `land`) | — | — | static | |
| `navy_experience_value` (instantTextBoxType, in `navy`) | `countries.<TAG>.military.xp.navy` | int | **unavailable** (current data) | As above; `xp.navy` unfed. |
| `navy_experience_icon` (iconType, `GFX_navy_experience`) | — | — | static | |
| `air_experience_value` (instantTextBoxType, in `air`) | `countries.<TAG>.military.xp.air` | int | **unavailable** (current data) | As above; `xp.air` unfed. |
| `air_experience_icon` (iconType, `GFX_air_experience`) | — | — | static | |
| `nukes_value` (instantTextBoxType, in `nukes`, no placeholder) | `countries.<TAG>.extra.nukes_total` (SCHEMA §3.11) | int | **unavailable** (current data) | `extra` domain is schema-present but the country object here has **no `extra` key** (save `country_extra.json` not folded into these snapshots). |
| `nuke_icon` (iconType, `GFX_topbar_nuke`) | — | — | static | |
| `command_power_value` (instantTextBoxType, in `cp`, placeholder `999`) | `countries.<TAG>.politics.command_power` (= `military.command_power`) | unsigned-int | **exact** | 182.36→`182`. Stored stock. |
| `command_power_icon` (iconType, `GFX_command_power`) | — | — | static | |
| `threat_value` (instantTextBoxType, placeholder `"50%"`) | world tension (`world_events.threat[]` aggregate / `world.world_tension_inputs`) | percent | **unavailable** | World-tension **aggregate scalar is not stored** (SCHEMA §17/§18 explicitly: only the contributor ledger + scaling inputs). No `world_events`/`world` block in these snapshots either. Badge unavailable. |
| `threat_button` (buttonType, `GFX_world_tension_icon_big_strip`) | (tension level → frame) | category-frame | **unavailable** | Frame would encode tension band; same gap as `threat_value`. |
| `threat_value_button` (buttonType) | — | — | static | Clickable hotspot behind `threat_value`. |
| `threat_icon_glow` (iconType, `GFX_WT_anim_strip`) | (tension animation) | — | static/cosmetic | Animated glow; cosmetic, not bound. |
| `initiative_text` (instantTextBoxType, in `faction_button_container`, placeholder `999`) | faction initiative (`world.faction_influence[].initiative`, SCHEMA §18) | int | **unavailable** | Faction-system initiative not in these snapshots (no `world` block). Country `politics.faction` has `{name,is_leader}` only. |
| `faction_icon` (iconType, `GFX_faction_logo_generic_democratic_miniature`) | `countries.<TAG>.politics.faction.name` | category (faction→logo sprite) | **derived** | We have `faction.name` (`"faction_template_axis"`); the logo sprite is chosen from it (a lookup, not a stored sprite id). |
| `faction_button` (buttonType, `GFX_topbar_faction_empty`) | `politics.faction.name` (membership present?) | flag/visibility | **derived** | Visible/active when the country is in a faction; drill to faction view (read-only). |

## 1b. Topbar — deferred (needs imperative adapter; NOT bound in this slice)

Per the native replay contract and PLAN §C.2 (outliner = imperative adapter),
outliner, alerts, and speed controls render as real chrome but are **inert**; no
binding is invented.

| gui element (name + type) | why deferred |
|---|---|
| `dismissed_alerts_button` (buttonType) | **deferred: needs adapter** — alerts feed is an imperative adapter (PLAN §C.4 alerts/outliner), out of slice. |
| `decisionview_button` + `decision_view_alert_glow` + `decisionview_amount_timeout_bg` + `decisionview_amount_timeout_items` + `decisionview_amount_to_take_bg` + `decisionview_amount_to_take_items` | **deferred: needs adapter** — decision-alert counters require the decisions-grouping adapter. Placeholders (`999`) render inert. |
| `intel_agency_button` + `intelview_ops_prepared_or_completed_bg` + `intelview_ops_prepared_or_completed_items` + `intelview_ops_available_bg` + `intelview_ops_available_items` | **deferred: needs adapter** — intel ops counters need the intel adapter. |
| `prototype_rewards_aquired` (+ `_bg`) | **deferred: needs adapter** — tech-reward counter (research adapter). |
| `non_delivering_contract_amount` (+ `non_delivering_contract_amount_bg`, container `non_delivering_contracts`) | **deferred: needs adapter** — trade-contract counters (diplomacy/trade adapter). |
| `ongoing_contract_amount` (+ `ongoing_contract_amount_bg`, container `delivering_contracts`) | **deferred: needs adapter** — as above. |

## 1c. Topbar — STATIC / decoration / navigation (no data binding)

These are pure chrome, navigation buttons (read-only verbs, no datum), or animated cosmetics.
Listed for completeness so the map is provably exhaustive.

- **Backgrounds / chrome:** `background` (icon), `armyoverview_buttons_bg`, `observer_flag_overlay`,
  `toggle_music_commands_win` (+ children `Background`, `music_pause_button`, `music_next_button`,
  `musicplayer_button`), `cp_bg`/`land_xp_bg`/`navy_xp_bg`/`air_xp_bg`/`nukes_bg` (container
  backgrounds), `pause_button_date`/`pause_button_date_paused` (clock chrome — the *paused* frame
  is a replay/scrubber state, not country data), the 5 speed steps `button_speedstep1` /
  `button_speedstep2` / `button_speedstep3` / `button_speedstep4` / `button_speedstep5` +
  `button_speedup` / `button_speeddown` (sim-speed control — **inert in a replay**, the scrubber owns time),
  `decisionview_amount_*_bg` / `intelview_*_bg` / `*_contract_amount_bg` / `prototype_rewards_aquired_bg`
  (alert-count backgrounds, paired with their deferred counters above).
- **Navigation buttons (wired to read-only "open view" verbs, no datum bound):** `reopen_ingame_lobby`
  (+ `reopen_ingame_lobby_label` "LOBBY" text — replay: inert), `menu_button`, `help_button`, `achievements_button`, `playlist_button`,
  `army_button`, `navy_button`, `air_button`, `technology_button`, `diplomacy_button`,
  `trade_button`, `construction_button`, `production_button` (+ `production_button_glow`),
  `deployment_button`, `logistics_button`, `officer_corp_button`, plus the structural containers
  `country_menu_window`, `faction_button_container`, `after_faction_button`, `after_intel_agency_button`,
  `time_control_window`, `land`/`navy`/`air`/`nukes`/`cp` (positioning containers).

---

# VIEW 2 — STATE / PROVINCE panel (`game_assets/gui/province_state.json`)

Root windows: `countrystateview` (→ `state_info_window` + `province_info_window`) plus reusable
ENTRY TEMPLATES instantiated by the gridboxes (`state_building_entry`,
`state_shared_slot_building_entry`, `province_building_entry`, `province_building_special_icon_entry`,
`state_resource_entry`, `state_compliance_status_container`, `state_resistance_status_container`,
`state_view_dynamic_modifier_entry`, `province_view_strategic_location_entry`,
`state_stakeholder_entry`). Selected state id = `<id>`.

Formatters: `int`, `level-int`, `percent`, `manpower`, `flag` / `flag-small`, `category`
(category id→loc/sprite), `building-icon` (frame of `GFX_buildings_strip`), `resource-icon`
(frame of `GFX_resources_strip`), `progressbar-fraction`, `damage-bar-fraction`.

## 2a. State panel — dynamic (data-bound) elements

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `state_name` (instantTextboxType, placeholder `"STATE_NAME_TEST"`) | `state_meta`[`<id>`].name (map artifact) + `<id>` | category/loc | **exact** | State display name from `map_pipeline/state_meta.json` keyed by id (not in the snapshot — it's static map metadata). |
| `owner` (instantTextboxType label) | (loc label "Owner") | static-loc | static | Label text; the flag is `state_owners`/`controller_flag`. |
| `state_owners` (OverlappingElementsBoxType) | `states.<id>.owner` | flag-small | **exact** | Owner TAG → small flag (ITA in sample). Overlapping box because owner+controller can coincide; the `state_owners_size_when_owner_is(_not)_controller` positionTypes pick width by whether owner==controller. |
| `controller_flag` (buttonType, `GFX_flag_small2`) | `states.<id>.controller` | flag-small | **exact** | Controller TAG; shown when controller≠owner. Click = drill to controller country. |
| `controller_flag_border` (iconType, `GFX_diplo_countrylist_flag_frame`) | — | — | static | Frame around controller flag. |
| `claims` (instantTextboxType label) | (loc label "Claims") | static-loc | static | Label only. |
| `state_claims` (OverlappingElementsBoxType) | `states.<id>.is_claimed_by[]` | flag-small (rows) | **unavailable** (current data) | Schema-present (`is_claimed_by`) but **always `[]` (0/1081)** in current snapshots → no claim flags. Cores (`is_core_of`) likewise unfed. |
| `victory_points` (instantTextboxType) | `states.<id>.victory_points_total` | int | **exact** | Sum of state's province VP (sample=1; populated 865/1081). |
| `victory_points_bg` (iconType, `GFX_victorypoint_stateview_bg`) | — | — | static | VP icon background. |
| `manpower` (instantTextboxType, in `state_info_window`) | `states.<id>.manpower_pool.total` | manpower | **exact** | **Codex correction:** state-view population shows the **total** state manpower pool (sample total=322900), NOT `.available` (=667, a separate recruitable concept). `manpower_pool` populated 1081/1081. |
| `population_icon` (iconType, `GFX_population_icon`) | — | — | static | Population icon for the manpower field. |
| `state_building_entries` (gridBoxType) → `state_building_entry` rows | `states.<id>.buildings.{infrastructure,industrial_complex,arms_factory,air_base,anti_air_building,radar_station,fuel_silo,synthetic_refinery}` | row factory; per row: `level`→level-int, `building_picture`→building-icon | **exact** | One row per present state-scoped building. Levels are measured ints (populated 951/1081). Building→sprite frame is a static icon map. `dockyard` also appears here when present. |
| `state_shared_slot_building_entries` (gridBoxType) → `state_shared_slot_building_entry` rows | `states.<id>.buildings.*` (shared-slot subset: `dockyard`, large slots) | row factory; `building_picture`→building-icon, `damage_bar`→damage-bar-fraction | **exact** (levels) / **unavailable** (damage) | Shared building slots. Levels exact; **per-building damage is not stored** → `damage_bar` and `building_status_overlay` badged unavailable. `remove` button inert (read-only). |
| `shared_slot_count` (instantTextboxType, placeholder `"SLOTS"`) | derived: used vs max shared building slots | int (`n/m`) | **derived** | In-game = used/total shared slots; we count `buildings.*` shared entries but **max-slots is a state-category/tech computed value** we don't store → derived/approximate. |
| `state_resources_entries` (gridBoxType, in `state_resources`) → `state_resource_entry` rows | `states.<id>.resources.{steel,oil,aluminium,tungsten,chromium,rubber,coal}` | row factory; `resource`→resource-icon, `amount`→int | **unavailable** (current data) | Schema-present (`states.<id>.resources`, "raw deposits") but **always `null` (0/1081)** in current snapshots. **Codex:** do NOT substitute country-level `economy.resources.produced` (national aggregate ≠ per-state deposit). Badge unavailable. |
| `state_resistance_status_container.progress` (iconType bar) | `states.<id>.occupation.resistance` / 100 | progressbar-fraction | **exact** | Resistance is a **0..100 percent** (sample 4.525). Bar fraction = value/100. Occupation present 400/1081. |
| `state_resistance_status_container.progress_text` (instantTextBoxType, placeholder `"100%"`) | `states.<id>.occupation.resistance` | percent | **exact** | Shown as `4.5%`. |
| `state_compliance_status_container.progress` (iconType bar) | `states.<id>.occupation.compliance` / 100 | progressbar-fraction | **exact** | Compliance 0..100 (sample 97.07). |
| `state_compliance_status_container.progress_text` (instantTextBoxType, placeholder `"100%"`) | `states.<id>.occupation.compliance` | percent | **exact** | `97%`. |
| `state_resistance_icon` (iconType, `GFX_resistance_icon`) | visible iff `states.<id>.occupation` present | visibility | **derived** | Icon shown only for occupied states; visibility = `occupation != null`. |
| `state_compliance_icon` (iconType, `GFX_compliance_icon`) | visible iff `occupation` present | visibility | **derived** | As above. |
| `select_law_icon` (iconType, `GFX_occupation_policy_icon_strip`) | `states.<id>.occupation_law` | category (law→frame) | **unavailable** (current data) | Schema-present but **always `null` (0/1081)** → occupation-law frame can't be chosen. Badge unavailable; `select_law_button` inert (read-only). |
| `energy_text` (instantTextboxType, in `state_info_window`) | per-state energy/fuel consumption | int | **unavailable** | No per-state energy-consumption field in schema (`states.<id>` has no energy). Badge unavailable; `energy_icon`/`energy_consumption_bg` are static chrome. |
| `province` (instantTextboxType, in `province_info_window`) | `provinces.<pid>` name (province meta) | category/loc | **derived** | Province name from province metadata + selected province id; the snapshot `provinces.<pid>` carries `controller`/`state_id`/`supply` but not a name → name from static map meta. |
| `province_building_entries` (gridBoxType) → `province_building_entry` rows | `states.<id>.province_buildings.{naval_base,coastal_bunker,bunker,rail_way,supply_node,naval_supply_hub,air_facility,…}` | row factory; `level`→level-int, `building_picture`→building-icon | **exact** (levels) | Province-scoped buildings folded to the state (SCHEMA §4 `province_buildings`, populated 846/1081). Levels exact; per-building `damage_bar` unavailable (not stored). 33 distinct keys incl. landmarks. |
| `province_custom_icon_building_entries` (gridBoxType) → `province_building_special_icon_entry` rows | `states.<id>.province_buildings.{landmark_*, canal_kiel, canal_panama, dam, nuclear_facility, naval_headquarters, …}` | row factory; `building_button`→building-icon | **exact** (presence) | Special/landmark province buildings (e.g. `landmark_eiffel_tower`, `canal_panama`). Presence exact; `damage_bar` (`GFX_province_landmark_damage_bar`) unavailable. |
| `state_stakeholder_entry` (windowType template) → `stakeholder_country_flag` (guibuttonType) | (state stakeholders / claimants list) | flag-small | **unavailable** | Template for a stakeholder-flag row; backing data (stakeholder tags) not in schema for states. Its frame icons `flag_border_thin` (`GFX_small_flag_frame_thin`), `flag_border_thick` (`GFX_diplo_countrylist_flag_frame`), and `flag_exclamation_mark` (`GFX_alerts_category`, alert overlay) are static chrome on the template, also unbound. |
| `state_view_dynamic_modifier_entry` (template) → `icon` / `flag` | per-state dynamic modifiers | category (modifier→icon) | **unavailable** | `dynamic_modifiers_grid` instantiates these from per-state modifier list; no per-state modifier array in schema (`country_extra.dynamic_modifiers` is country-level, not state). Badge unavailable. |
| `province_view_strategic_location_entry` (template) → `icon` | `strategic_locations_grid` source | category | **unavailable** | Strategic-location icons per province; no backing field in schema. |

## 2b. State panel — STATIC / decoration / weather / navigation (no data binding)

- **Backgrounds / frames / chrome:** `Background` (in `state_info_window`, `GFX_ww_stateview_bg`),
  `Resources_bg` (`GFX_tiled_window_transparent_transparent`), `level_bg` (entry templates),
  `build_slot_bg`, `energy_consumption_bg` / `energy_icon` (static; the `energy_text` value is
  unavailable per 2a), the entry-template `Background`s, the resistance/compliance container `bg`
  icons (`GFX_occupation_state_resistance_bar_bg` / `..._compliance_bar_bg`).
- **Weather / climate / terrain decoration (province_info_window):** `terrain_picture`
  (`GFX_terrain_default`), `modifiers_mud_icon`, `modifiers_snow_icons`, `modifiers_weather_icons`,
  `temperature_icons`, `impassability_icon`, `flooded_icon`, `unplanned_offensive_icon`,
  `the_great_wall_icon`, `building_modifiers_icon`, `province_required_rule`, `state_modifiers`
  (placeholder `"STATE_MODIFIERS"`). These are **per-province weather/modifier icons** — the
  original vertical slice was Political-mode only; weather/modifier state for a
  province was **out of that slice** and not bound here. (If later bound, they would be `proxy`/
  `unavailable` — weather is per-province, schema carries no province weather in these snapshots.)
- **Positioning anchors (positionType, no visual):** `countrystateview_weathericons_position`,
  `countrystateview_weathericons_offset`, `state_owners_size_when_owner_is_controller`,
  `state_owners_size_when_owner_is_not_controller`, `resistance_status_pos`, `compliance_status_pos`,
  `facility_building_button_scale` — named slots/anchors the adapter targets; no datum.
- **Garrison / template (occupation sub-panel) — inert in replay:** `template_garrison_button`,
  `template_garrison` (`GFX_locked_icon`), `template_need_text` (placeholder `42`),
  `law_and_template_container`, `select_law_button` — garrison-template assignment is a **mutation**
  (PLAN §D, DISABLED); rendered inert. `template_need_text` would be a derived garrison requirement
  we don't store → treat as unavailable if ever shown.
- **Navigation / window controls:** `close_button` (`GFX_closebutton_small`), `scorched_earth_state_button`
  (mutation → inert/read-only), the structural containers `countrystateview`, `state_info_window`,
  `province_info_window`, `state_resources`, `resistance_container`, `custom_icon_container`,
  `claim_entry` (+ `claim_flag`), and the reusable building/resource entry **template roots**
  (`state_building_entry`, `state_shared_slot_building_entry`, `province_building_entry`,
  `province_building_special_icon_entry`, `state_resource_entry`) when considered as containers
  (their *bound children* are in 2a).
- **Damage/status overlays (entry templates), not stored → effectively static here:**
  `damage_bar` (all 4 entry templates), `building_status_overlay`, `remove` (shared-slot, mutation
  → inert). Per-building damage/status is not in schema → these render at a default (no fraction).

---

## Notes

**A. No stability/war-support progressbar in this topbar.gui.** The early task brief said
"stability (+ its progressbar)" / "war_support (+ bar)", but the parsed
`topbar.json` contains **no `progressbartype`** for stability or war support — only an icon
(`stability_icon`/`war_support_icon`), a value box (`*_value_bg`), and the text. The only
bar-like icons in the topbar are `industrial_ratio_bar`, `fuel_ratio_bar`, `convoys_ratio_bar`,
`supply_ratio_bar` (all `GFX_topbar_fuel_bar` icons, mapped above). The stability/WS "bar" is the
in-game *tooltip* breakdown, not a topbar element. Codex confirmed: **the stability/WS progressbar
claim is unfounded for this view.** (Fuel/IC/convoy/supply ratio bars ARE present and mapped.)

**B. `unavailable` ≠ "missing forever".** Several `unavailable` badges (state `resources`,
`victory_points_by_province`, `occupation_law`, `is_core_of`/`is_claimed_by`, country `military.xp`,
`extra.nukes_total`, the `world`/`world_events` topbar gaps) are **schema-present fields the current
snapshot generator does not yet feed** (parser-unfed), or save domains not folded into THESE
snapshots — not hard-impossible (SCHEMA §10) limits. They become `exact` once their parser/oracle
domain is wired. They are badged `unavailable` (render "—") rather than omitted, per the
no-fabrication rule. Genuinely runtime-computed gaps (fuel days-remaining, IC consumer split,
per-building damage, garrison need, world-tension aggregate) stay `derived`/`unavailable` by nature.

---

## Element accounting (provably complete)

### TOPBAR (`topbar.json`) — type counts: containerWindowType 14, iconType 39, buttonType 35, instantTextBoxType 24, background 6 (= **118** elements)

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** (incl. derived/unavailable badges) | **31** | flag, pol_power, stability, war_support, manpower, industrial_capacity(+bar), fuel(+bar), convoys(+bar), DateText, command_power, supply(+bar), the 3 xp values, nukes, threat(×2), initiative, faction icon/button, + their measured-vs-unavailable mix |
| **Deferred: needs adapter** | **6 logical** (12 elements w/ bgs) | dismissed_alerts, decisionview counters, intel ops counters, prototype rewards, 2 contract counters |
| **Static / decoration / navigation** | **~75** | background, music win (4), 16 nav buttons, speed steps (7), clock chrome, all `*_icon`/`*_bg`/`*_value_bg` chrome, alert-count bgs, positioning containers |

Every one of the 118 topbar elements falls in exactly one bucket above (the per-element tables 1a/1b/1c enumerate them by name).

### STATE PANEL (`province_state.json`) — type counts: containerWindowType 17, buttonType 11, positionType 7, background 5, instantTextboxType 12, gridBoxType 7, slotsize 7, iconType 39, OverlappingElementsBoxType 2, instantTextBoxType 3, windowType 1, guibuttonType 1 (= **112** elements)

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** (incl. derived/unavailable badges) | **~28** (incl. 7 gridbox row-factories + their bound row children) | state_name, state_owners, controller_flag, victory_points, manpower, building grids (×4 factories), resource grid, resistance/compliance bars+text (×4), occupation icons, select_law_icon, province, claims/stakeholder/modifier templates |
| **Static / decoration / weather / nav** | **~84** | 7 positionTypes, weather/terrain/modifier icons (12), backgrounds/frames, damage/status overlays, garrison-template (inert), close/scorched buttons, container roots, label texts |

Every one of the 112 state-panel elements is enumerated by name in tables 2a/2b.

### `unavailable`-badged elements (with reason)

**Topbar (9):** `convoys_ratio_bar` (assigned/free split not stored), `supply_value` + `supply_ratio_bar` (no national supply scalar), `experience_value`/`navy_experience_value`/`air_experience_value` (`military.xp` = `{}` unfed), `nukes_value` (no `extra` domain), `threat_value` + `threat_button` (world-tension aggregate not stored), `initiative_text` (faction-system unfed).

**State panel (8):** `state_claims` (`is_claimed_by` always `[]`), `state_resources_entries` rows (`resources` always null), `select_law_icon` (`occupation_law` always null), `energy_text` (no per-state energy field), `state_stakeholder_entry` flags (no stakeholder data), `state_view_dynamic_modifier_entry` (no per-state modifier array), `province_view_strategic_location_entry` (no strategic-location data), shared/province building `damage_bar`/`building_status_overlay` (per-building damage not stored).

**Total `unavailable`: 17 elements.** All are schema-present-but-unfed or runtime-computed values — none fabricated; each renders "—" per no-fabrication.
