# BINDING MAP — MILITARY MANAGEMENT (army / navy / air / OOB / deployment / equipment)

> **What this is.** The COMPLETE, no-truncation data-binding map for **15 in-game the game views** the
> native replay UI binds in the Phase-3 sweep — the army / navy / air management surfaces, the
> order-of-battle (theatre / battle-plan / abilities / divisions-summary / railway-gun) chrome, the
> deployment queue, and the equipment overview. It realises GAME_UI_REPLICA_PLAN §C ("comprehensive
> but mechanical binding sweep") + §C.4 rows "Army / navy / air mgmt + plans" (OOB adapter; OOB exact;
> battle-plan ORDER geometry partial §G) and "Production/construction/deployment/equipment overview".
> EVERY named element in each `.gui` tree is accounted for, in exactly one bucket: **dynamic
> (data-bound)**, **static/decoration** (no binding), or **deferred (needs imperative adapter /
> mutation-inert)**. Nothing is omitted; where a `.gui` element exists but our snapshot has no field
> for it, the element is listed and **badged `unavailable`** with the precise missing field named.
>
> **Binding-rule shape** (PLAN §C.1): `{ target (gui element name+type, sprite/text, loc key), source
> (exact snapshot dotted field path), format (formatter id), fidelity (exact|derived|proxy|
> unavailable) }`. **Source paths** use the SCHEMA dotted form (`countries.<TAG>.…`,
> `order_of_battle.…`, `designs.…`, `divisions[]`, `strategic_air.…`); see `replay/schema.py`
> `FIELD_SOURCES` — the authoritative machine-readable mirror — cross-checked against
> `replay/SCHEMA.md` §3.1/§3.2/§6/§11/§12/§14. `<TAG>` = the player/observed country. Per-row sources
> inside a gridbox row-factory are written `…[].<field>`.
>
> **No-fabrication (SCHEMA §0.2, PLAN principle 3):** `null` renders "—", never `0`. `0`/`0.0` is a
> measured zero. A field schema-present but never fed by the current parser is badged `unavailable`
> (not silently dropped). A runtime-computed value we do not model (battle-plan path geometry, air
> mission `effective_planes` RUNTIME, combat-stat previews, terrain/fort modifier previews) is
> `derived`/`proxy`/`unavailable` by nature.
>
> **Grid row-factory model (PLAN §B.4 / §C.2).** `gridBoxType`/`gridboxtype`/`listBoxType`/
> `smoothListBoxType`/`OverlappingElementsBoxType` are **ROW FACTORIES**, not passive containers: the
> engine instantiates a named ENTRY-TEMPLATE window once per bound array element. In these `.gui`
> files the grids and their entry-template windows are mostly defined as SEPARATE top-level windows
> (e.g. `army_units > item_grid` is the grid; `armyview_army_entry` is the row template it
> instantiates). Each entry template is mapped **once** below; at runtime it is **repeated per array
> element**. The grid container itself carries no datum (it is the structural row factory); its bound
> CHILDREN (in the entry template) carry the data. Case variants (`gridBoxType`/`gridboxtype`,
> `Background`/`background`, `ButtonType`/`buttonType`, `instantTextBoxType`/`instantTextboxType`) are
> the same engine type and are matched case-insensitively (the parser preserved source casing, so the
> per-type stats counts list both spellings — totals are unaffected).

---

## Provenance of the fidelity calls

Grounded in **`replay/schema.py` FIELD_SOURCES + the dataclass docstrings** (the STABLE, wiring-
complete schema), cross-checked against `replay/SCHEMA.md` §3.1/§3.2/§6/§11/§12/§14 and the model
maps `BINDING_MAP_production_construction_decisions.md` (grid row-factory model) and
`BINDING_MAP_diplomacy_combat.md` (roster aggregation + combat hard-limits). **Adversarially reviewed
by Codex** (read-only, 2026-06-26); its verdict and corrections are folded in (see the "Codex round"
note at the end).

**Critical schema facts that drive the badges (verified in schema.py FIELD_SOURCES + dataclasses):**

| schema leaf | shape / element | source | consequence in these views |
|---|---|---|---|
| `order_of_battle.armies[]` (`oob_command` M, save backstop → M/S) | `{id, owner, name, leader(general id), members[](div `id:type`), member_count, members_resolved, orders[](battle-plan `order_instance`), field_marshal_group, cached{…}}` | **M/S** | army rows: name/owner/member-count EXACT; **member ids + leader id are RAW engine ints, not TAGs** (only `owner` is a resolved TAG) → leader NAME/skill needs `commanders` join (derived/unavailable); battle-plan order geometry partial (§G) |
| `order_of_battle.army_groups[]` | `{id, owner, name, leader(FM `id:type`), army_refs[], army_refs_resolved, cached{plan_value,our_power,enemy_power}(RUNTIME), icon}` | **M/S** | army-group shields: name/owner/member EXACT; `cached.*` plan/power **RUNTIME, non-authoritative** → proxy |
| `order_of_battle.fleets[]` | `{id, owner, name, leader(admiral id), task_forces[{id,name,location_province,owner,mission,ships[{id,definition,organisation,strength,experience,name,equipment}]}]}` | **M/S** | fleet rows + task-force rows + per-ship rows EXACT identity/org/str/xp; **leader id RAW int** → admiral name/skill needs `commanders` |
| `order_of_battle.ships[]` (LIVE per-ship, `ships` M) | `{instance_id, definition, category, strength_hp, organisation, experience, sunk_convoys}` — **owner NOT carried live** (save-join only) | **M** | live ship rows: org/str/xp/category EXACT; owner needs the save OOB join |
| `order_of_battle.navy_theaters[]` / `air_wings[]` | save-only lists | **S** | theatre selector + air management lean on these for fleet/wing membership |
| `order_of_battle.air_wings[]` | `{id, owner, name, count(planes), experience, manpower, mission/mission_type(cached), strategic_region, plane_definition, air_base(`id:type`), state_id, equipment[[id,type,amount]], effective_planes(RUNTIME)}` | **S** | air-wing rows: count/xp/manpower/mission/base/equipment EXACT (save); **`effective_planes` RUNTIME → proxy** |
| `order_of_battle.battle_plan_orders[]` (`construction_orders_live` M) | per-army `{army_id, order_type, target_states}` — **`target_states` EXACT for type-5 (offensive) ONLY; non-type-5 live geometry RESISTED (null)**; `army_id`+`target_states` are RAW engine ints | **M** | battle-plan tools: order TYPE bindable; live ARROW/PATH geometry unavailable except offensive target-state SET; save-backed `armies[].orders[]` measured geometry can render exact paths when folded |
| `order_of_battle.by_country` | `{TAG: {armies, army_groups, divisions_in_armies, fleets, task_forces, ships, air_wings, planes}}` | **S/D** | the per-country COUNT rollups (army/navy/air tallies, theatre counts) |
| `designs.division_templates[]` (`designs_live` M, save backstop → M/S) | `{id, type, name, country, role, regiments[{type,x,y}], support[], priority, division_cap, obsolete, is_locked, is_army_hq, n_regiments, n_support}` | **M/S** | deployment template rows + division-template names; **derived combat stats NOT stored** (runtime, §11 note) |
| `designs.equipment_variants` (save) / `…_live` (M) | keyed `"id:type"` / bare live id; `{archetype, family, name, version, creator, obsolete, modules, upgrades, design_team…}` | **S / M** | equipment-overview names/icons; per-variant STATS = `designs.equipment_variant_stats` (**D**) |
| `divisions[]` (§6, M/S + divisions_full body) | `{div_id, div_type, owner, location_province, template_id, template_name, template_role, organisation, strength, equipment[], experience(M\*), …}` | **M / S** | unit rows org/str/equipment EXACT; per-unit combat previews NOT stored |
| `countries.<TAG>.military.{num_ships,division_count,manpower,xp,equipment_stockpile[],convoys[],convoys_count,ships_by_hull,casualties}` | §3.2 | M / S | army/navy/air header counters; `equipment_stockpile[]` (S) drives equipment overview |
| `countries.<TAG>.military.deployment_queue` (deployment.json) | save-only conveyor + reinforcement priority; null until ingested | **S** | deployment-queue rows (the conveyor/series) — **`location` target RESISTED like construction** |
| `countries.<TAG>.military.reinforcement_priority` | int | **S** | reinforcement-priority widgets |
| `countries.<TAG>.economy.construction_queue` (S) / `production.construction_queue` (M) | save HAS `location_state`+`template`; live RESISTS location | **S / M** | deployment overlaps the construction/economy deploy state where building-vs-unit deploy share the queue |
| `strategic_air.air_bases[]` (S) | `{id, type, state, capacity, base, level, countries[{country, capacity_penalty}]}` | **S** | air-base capacity in air-selection / strategic-air |

**Genuinely runtime-computed (never stored anywhere) → stay `unavailable`/`proxy` by nature (PLAN §G
hard-limit class):** air mission `effective_planes`, air superiority/detection/efficiency per region
(`naval_air.air_superiority` is a REGION PROXY only, badged), per-unit/per-division & per-ship COMBAT
stat previews (soft/hard attack, defense, org bars in a combat preview), tactics previews, battle-plan
live ARROW/PATH geometry (only offensive `target_states` survive unless save-backed measured geometry
is folded), terrain/fort modifier previews, plane
fuel-efficiency warnings, screening efficiencies, carrier-capacity penalty numbers, ace-in-wing combat
contribution, the `cached.{plan_value,our_power,enemy_power}` army-group power estimates (cached
runtime), deployment ETA/training progressbars (computed). These are serialize-computed UI aggregates
with no persisted backing field.

> **Read-only verb note (PLAN §D).** Every `*_button` whose in-game role is to CREATE / DELETE / SPLIT
> / MERGE / REORGANIZE / ASSIGN / DEPLOY / DRAW-ORDER / RENAME / set-priority / set-mission / upgrade
> is a **mutation** verb. In a replay these render as real chrome but are **inert** (no datum, no
> action). They are listed under each view's static/deferred section, never bound. The *outcomes* they
> would produce (a new army, a moved division, a deployed unit, a drawn front) surface instead via the
> OOB snapshot + the diff-event timeline (SCHEMA §9.2 `army_formed`/`division_created`/
> `division_moved`/`commander_assigned`), not via these buttons.

---

# VIEW 1 — ARMY (`game_assets/gui/army.json`, from `interface/countryarmyview.gui`)

Root `countryarmyview` + 7 sibling top-level windows (4 small filter/count strips + 3 unit-row entry
templates). The panel is **tabbed** — `{army_header, army_units}`, `{navy_header, navy_units}`,
`{air_header, air_units}` — each `*_units` carries an `item_grid` row factory instantiating one of the
three entry templates. Binds the player/observed country `<TAG>`. Formatters: `int`, `flag`/
`flag-small`, `progressbar-fraction` (org/strength bars), `category` (unit-type id→icon), `loc`.

## 1a. Army — dynamic (data-bound)

### Top info bar (`countryarmyview > info`) + titles

| gui element (name + type, sprite/text) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `army_title` (instantTextboxType, text `ARMY_ARMY_LABEL`) | static loc | loc | **static** | View label (decoration). |
| `filter_title` (instantTextboxType, `ARMY_FILTER_LABEL`) | static loc | loc | **static** | Filter section label. |
| `unit_count` (instantTextboxType) | `countries.<TAG>.military.division_count` | int | **exact** | Count of `<TAG>` entries in `divisions[]` (M/D). |
| `battalion_count` (instantTextboxType, placeholder `"30 Battalions"`) | SUM of regiments across `<TAG>`'s division templates / divisions | int | **derived** | Battalion total derivable from `designs.division_templates[].n_regiments` × division counts; computed aggregate. |
| `special_forces_cap` (instantTextboxType, placeholder `7`) | special-forces battalion CAP vs used | int | **unavailable** | The special-forces cap is a tech/manpower-computed modifier; not stored. |
| `theatres_count` (instantTextboxType) | `order_of_battle.by_country.<TAG>` theatre tally / `order_of_battle.theatres[]` filtered by owner | int | **derived** | Count of the country's theatres (S/D rollup). |

### ARMY tab — header grids + unit list

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `army_header > sub_units_grid` (gridBoxType) → `army_view_subunit_entry` rows (`sub_units_count`, `sub_unit_icon`, `filter_button`, `highlight`) | per-sub-unit-type tallies over `<TAG>`'s `divisions[].equipment`/templates | row factory; `sub_units_count`→int, `sub_unit_icon`→category | **derived** | One chip per sub-unit type; count = grouping over the division/equipment set. `filter_button` is a UI filter (inert). |
| `army_header > modifiers_grid` (gridBoxType) → `modifier_entry` rows (`modifier_icon`, `modifier_value`, `modifier_bg`) | national ARMY modifiers (`countries.<TAG>.extra.dynamic_modifiers[]` / army-XP/terrain mods) | row factory; `modifier_value`→int/pct, `modifier_icon`→category | **unavailable** | Army-wide combat modifiers shown here are computed modifier aggregates; `dynamic_modifiers[]` exists (S) but the specific army-overview modifier set (experience/terrain bonuses) is a runtime display → unavailable. |
| `army_units > item_grid` (gridBoxType) → **`armyview_army_entry`** rows | iterate `order_of_battle.armies[]` (owner=`<TAG>`) and/or `divisions[]`(owner=`<TAG>`) | row factory | **exact (enumeration)** | One row per army/division. Member resolution = `armies[].members_resolved`→`divisions[]`. |

### `armyview_army_entry` (the ARMY/division row template, repeated per army/division)

| gui element (name + type, sprite) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `name` (instantTextboxType) | `order_of_battle.armies[].name` (or `divisions[].template_name`) | loc | **exact** | Army name (save/live string) or division template name. |
| `theatre` (instantTextboxType) | the army's theatre (`order_of_battle.theatres[]` membership) | loc | **derived** | Theatre name via theatre→army join. |
| `group` (instantTextboxType) | the army's army-group (`order_of_battle.army_groups[].army_refs`) | loc | **derived** | Army-group name via membership join. |
| `icon` (iconType, `GFX_unit_infantry_icon_medium`) | dominant `divisions[].template_role` of the army | category (role→icon) | **derived** | Unit-type icon from the template role. |
| `level` (iconType, `GFX_unit_level`) | commander/army XP level | int (frame) | **unavailable** | Army XP level needs the commander/leader join (`armies[].leader` is a RAW int → `commanders`); not in OOB scalars. |
| `expeditionary_flag` / `foreign_template_flag` (iconType) | division/army `owner` vs `<TAG>` (foreign/expeditionary) | flag | **derived** | Shown when the unit's owner TAG differs from the viewing country (expeditionary). |
| `colonial_force_flag_frame` / `exile_force_flag_frame` (iconType) | colonial/exile force status | flag-frame | **unavailable** | Colonial/exile classification not stored per army in OOB. |
| `battle_icon` (iconType, `GFX_country_army_battle`, tooltip `IN_COMBAT`) | army has a division in `combats[]` | visibility | **derived** | Cross-check `divisions[].in_combat` (M\*) / `combats[]` attacker set; in-combat flag is M\* (low-confidence). |
| `obsolete_template_warning` (iconType, tooltip `DECOMMISSIONED_TEMPLATE_WARNING_TOOLTIP`) | the army uses a `designs.division_templates[].obsolete=true` template | visibility | **derived** | Obsolete flag IS on the template (M/S); the per-army "uses obsolete" join is derivable. |
| `organization_progressbar` (iconType, `GFX_navieslist_bar_org`) | mean `divisions[].organisation` over the army's members / template max | progressbar-fraction | **derived** | Org bar = current org / template-max org; org is exact (M) but the max (denominator) is a template-derived stat (D). |
| `strength_progressbar` (iconType, `GFX_navieslist_bar_str`) | mean `divisions[].strength` over members / template max | progressbar-fraction | **derived** | Same as org bar; strength exact (M/S), max derived. |
| `unit_details_button` (buttonType) | opens unit details | — | **deferred: nav** | Read-only verb (opens the unit panel). |
| `group_marker` (buttonType, `GFX_theatre_army_shield`) | the army-group shield | category (icon) | **derived** | Group shield insignia; jump-to verb inert. |

### NAVY tab — header grids + fleet list

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `navy_header > ships_count_grid` (gridBoxType) → `navy_view_ship_entry` rows (`ships_count`, `ship_icon`, `highlight`) | per-hull ship tallies — `countries.<TAG>.military.ships_by_hull` / `order_of_battle.by_country.<TAG>.ships` | row factory; `ships_count`→int, `ship_icon`→category | **derived** | `ships_by_hull` (S/D) gives `{hull:{count,tonnage}}`; one chip per ship class. |
| `navy_header > modifiers_grid` (gridBoxType) → `modifier_entry` | national NAVY modifiers | row factory | **unavailable** | As army modifiers — computed naval-modifier display, not stored. |
| `navy_units > item_grid` (gridBoxType) → **`armyview_navy_entry`** rows | iterate `order_of_battle.fleets[]` (owner=`<TAG>`) | row factory | **exact (enumeration)** | One row per fleet. |

### `armyview_navy_entry` (the FLEET row template, repeated per fleet)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `unit_name` (instantTextboxType) | `order_of_battle.fleets[].name` | loc | **exact** | Fleet name (save/live string). |
| `region` (instantTextboxType) | the fleet's task-force `location_province` → sea region | loc | **derived** | Region from `fleets[].task_forces[].location_province` → strategic-region name. |
| `mission_icon` (iconType, `GFX_NV_all_missions`) | `fleets[].task_forces[].mission` | category (mission→icon) | **derived** | Mission frame from the task-force mission token. |
| `unit_count` (instantTextboxType, placeholder `0`) | SUM ships across the fleet's task forces | int | **exact** | Count of `fleets[].task_forces[].ships[]`. |
| `organization_progressbar` / `strength_progressbar` (iconType) | mean ship org/strength over the fleet | progressbar-fraction | **derived** | From `ships[].organisation`/`.strength` (org exact M/S; bar fraction derived). |
| `leader_button` (ButtonType) / `leader_frame` (iconType) | `fleets[].leader` (admiral RAW int) → `commanders.admirals` | portrait/loc | **unavailable** | Leader id is a RAW engine int; admiral name/portrait needs the `commanders` join (not folded into OOB) → "—" + nav verb inert. |
| `battle_icon` (iconType) | fleet has a ship in a naval combat | visibility | **derived** | Cross-check `combats[]` naval entries (0 naval in current data → renders default). |
| `pride` (iconType, `GFX_NV_icon_pride`) | `extra.pride_of_the_fleet.{id,type}` matches a ship in this fleet | visibility | **derived** | Pride-of-the-fleet flag from `country_extra` (S). |

### AIR tab — header grids + wing list

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `air_header > count_grid` (gridBoxType) → `air_view_plane_entry` rows (`count`, `icon`, `highlight`) | per-plane-type tallies — SUM `order_of_battle.air_wings[].count` grouped by `plane_definition` | row factory; `count`→int, `icon`→category | **derived** | One chip per plane type; count = grouping over `air_wings[]` (S). |
| `air_header > modifiers_grid` (gridBoxType) → `modifier_entry` | national AIR modifiers | row factory | **unavailable** | Computed air-modifier display. |
| `air_units > item_grid` (gridBoxType) → **`armyview_air_entry`** rows | iterate `order_of_battle.air_wings[]` (owner=`<TAG>`) | row factory | **exact (enumeration)** | One row per air wing. |

### `armyview_air_entry` (the AIR WING row template, repeated per wing)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `unit_name` (instantTextboxType) | `order_of_battle.air_wings[].name` | loc | **exact** | Wing name (save). |
| `base` (instantTextboxType) | `air_wings[].air_base` (`id:type`) → state name; `air_wings[].state_id` | loc | **derived** | Air-base state resolved via `air_base`→state (`state_id` resolved, null for carrier/mobile). |
| `count` (instantTextboxType) | `air_wings[].count` | int | **exact** | Plane count (save). |
| `limit` (instantTextboxType) | wing size cap | int | **unavailable** | Per-wing operational limit is a runtime cap; not stored (only `count` is). |
| `assigned_region` (instantTextboxType) | `air_wings[].strategic_region` | loc | **exact** | Strategic region id → name. |
| `icon` (iconType) | `air_wings[].plane_definition` → equipment variant | category (variant→icon) | **derived** | Plane icon from the plane definition. |
| `bar_filled` (iconType, `GFX_navieslist_bar_str`) | wing strength = `count` / operational limit | progressbar-fraction | **derived** | Numerator exact; denominator (limit) not stored → derived/approximate. |
| `ace` (iconType, `GFX_ace_none`) | wing has an attached ace (`extra.aces[]`) | category (ace→icon) | **unavailable** | Ace-to-wing attachment not in `air_wings[]`; `aces[]` exists (S) but no wing join → unavailable. |
| `mission_icon_1..4` (iconType ×4, `GFX_strategicair_mission_types`) | `air_wings[].mission`/`mission_type` (cached) | category (mission→frame) | **derived** | Up to 4 mission frames from the cached mission token. |
| `carrier_capable_icon` / `on_carrier_icon` (iconType, tooltip `AIR_CARRIER_CAPABLE`) | wing is carrier-based (`air_wings[].state_id`=null + plane type) | visibility | **derived** | Carrier-capable flag derivable from plane definition + null base state. |
| `exile_force_flag_frame` / `foreign_wing_flag` (iconType, tooltip `GIE_AIRWING_SELECTION_ITEM_EXILE_FLAG`) | `air_wings[].owner` ≠ `<TAG>` | flag | **derived** | Foreign/exile wing flag. |

## 1b. Army — STATIC / decoration / navigation / mutation-inert (no data binding)

- **Backgrounds / chrome:** `countryarmyview > Background` (`GFX_tiled_window_1b_border`), `army_header_bg`, `info > Background`+`army_overview_top`, the three `*_units > Background`, `overview_bottom`, each entry's `btn_bg` (row-select bg), `navy_overview_ship_entry_bg`, the `highlight`/`modifier_bg` chrome on the count/modifier strips, `temp_editbox_bg`, `special_forces_icon`, `armyoverview_naval_leader_frame`.
- **Static label texts (loc):** `army_title` (`ARMY_ARMY_LABEL`), `filter_title` (`ARMY_FILTER_LABEL`), the sort button texts `ARMY_SORT_NAME`/`ARMY_SORT_THEATRE`/`ARMY_SORT_TYPE`/`ARMY_SORT_GROUP`/`ARMY_SORT_SIZE`/`ARMY_SORT_REGION`/`ARMY_SORT_BASE` (labels on the inert sort buttons).
- **Search / filter (UI-state, no datum):** `filter` (editBoxType — search box).
- **Mutation / sort / nav verbs — INERT (read-only, PLAN §D):** the 11 sort buttons (`army_sort_*`, `navy_sort_*`, `air_sort_*` — re-order lists); `filter_button` (sub-unit filter toggle); per-row `unit_details_button` / `group_marker` / `leader_button` (open/select); `close_button` (close); `naval_hq_open` / `destroyed_ships_open` / `details_open` (open other panels — naval HQ / sunk-ships / equipment overview).
- **Structural (row factories + layout):** `countryarmyview`, `info`, the 3 `*_header` + 3 `*_units` containers, all 9 `gridBoxType` (3×`item_grid`, 3×`modifiers_grid`, `sub_units_grid`, `ships_count_grid`, `count_grid` — row factories, no datum), `slotsize` (×9), `margin` (×7), `borderSize` (×7).

### Army element accounting (provably complete) — `stats.element_type_counts`

`containerWindowType` 15, `background` 7, `iconType` 42, `instantTextboxType` 21, `buttonType` 22,
`borderSize` 7, `margin` 7, `editBoxType` 1, `gridBoxType` 9, `slotsize` 9, `ButtonType` 1 = **141 total**.

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** (incl. derived/unavailable) | **~38** | unit_count, theatres_count, battalion_count, the 3 unit-row entry templates (`armyview_{army,navy,air}_entry` — name/region/base/count/org-bar/str-bar/mission-icons/flags/icon, mapped once each), the 3 count/tally grid entry templates (`navy_view_ship_entry`, `air_view_plane_entry`, `army_view_subunit_entry`), `modifier_entry` (unavailable) |
| **Static / decoration / nav / mutation-inert / structural** | **~103** | window/header/list backgrounds, 11 sort buttons, close/naval-hq/sunk-ships/equipment-overview nav, filter box, special-forces chrome, 9 grid factories + slotsizes, margins, borderSizes |

> Row-factory ↔ entry-template map: `army_units/item_grid`→`armyview_army_entry` (armies/divisions);
> `navy_units/item_grid`→`armyview_navy_entry` (fleets); `air_units/item_grid`→`armyview_air_entry`
> (air wings); `sub_units_grid`→`army_view_subunit_entry`; `ships_count_grid`→`navy_view_ship_entry`;
> `count_grid`→`air_view_plane_entry`; the three `modifiers_grid`→`modifier_entry`.

---

# VIEW 2 — NAVIES (`game_assets/gui/navies.json`, from `interface/naviesview.gui`)

Root `naviesview_window` + `target_bar_window` + `action_bar_window` + ~24 sibling top-level windows
(fleet/task-force/ship row templates, the composition editor, move-ships / home-base / refit dialogs,
ship-history + sunk-ship logs). This is a heavily **mutation-oriented** view. The data-bearing surface
is the **fleet → task-force → ship roster** (`detailed_fleet_box` → `fleet_view_header` +
`taskforce_view_entry` + `ship_entry_detailed/compact`), sourced from
`order_of_battle.fleets[].task_forces[].ships[]` (M/S) + the live `order_of_battle.ships[]` (M). Binds
`<TAG>`. Formatters: `int`, `flag`/`flag-small`, `progressbar-fraction`, `category`, `loc`, `fuel`.

## 2a. Navies — dynamic (data-bound)

### `fleet_view_header` (the FLEET card template, repeated per fleet)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `fleet_name` (editBoxType, placeholder `FLEET_NAME_HERE`) | `order_of_battle.fleets[].name` | loc | **exact** (display) | Fleet name; the editBox EDIT is a mutation (inert) — render value only. |
| `taskforce_count` (instantTextboxType, `"99/99"`) | count of `fleets[].task_forces[]` (active/total) | int | **derived** | TF count; the `/total` cap is a UI display. |
| `insigna` (iconType, `GFX_theatre_army_shield`) | the fleet's insignia/icon | category | **derived** | Insignia frame. |
| `active_in_regions` (instantTextboxType, `"Active in Regions: 4"`) | distinct sea regions over the fleet's task forces | int | **derived** | Count of distinct `task_forces[].location_province`→regions. |
| `home_base_name` (instantTextboxType, placeholder `"Home base"`) | the fleet's home-base province → state name | loc | **unavailable** | Per-fleet home base not stored in OOB; renders "—". |
| `pride_of_the_fleet_icon` (iconType, tooltip `FLEET_CONTAINS_PRIDE_OF_THE_FLEET`) | `extra.pride_of_the_fleet` ship in this fleet | visibility | **derived** | From `country_extra` (S). |
| `fuel_icon` (iconType, `GFX_fuel_extended_strip`) | fleet fuel level | fuel | **unavailable** | Per-fleet fuel not stored (country `fuel` is national, S; no per-fleet split). |
| `leader_name` / `skill_text` / `leader_stat_{attack,defense,maneuvering,coordination}_text` | `fleets[].leader` (admiral RAW int) → `commanders.admirals.*` | loc/int | **unavailable** | Leader id RAW; admiral name/skill/stats need `commanders` (not folded into OOB) → "—" + `CLICK_TO_ASSIGN` placeholder. |
| `traits_grid` (gridBoxType) → trait icons | `commanders.admirals.*.traits[]` | row factory; category | **unavailable** | Needs `commanders`. |

### `taskforce_view_entry` (the TASK FORCE card template, repeated per task force)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `unit_name` (editBoxType) | `fleets[].task_forces[].name` | loc | **exact** (display) | TF name. |
| `current_mission` (iconType, `GFX_NV_all_missions`) | `task_forces[].mission` | category (mission→icon) | **derived** | Mission frame from the TF mission token. |
| `current_activity` (iconType) / `activity_description` (instantTextboxType, `"I DO THAT"`) | TF current activity (moving/training/in-combat) | category/loc | **unavailable** | TF activity state (en-route/training) is a runtime status; not in the OOB TF shape (which carries mission only). |
| `dominance` (iconType) | naval dominance contribution | category | **unavailable** | Computed naval-dominance stat; not stored. |
| `fuel_stocks_icon` / `fuel_efficiency_warning_icon` (iconType) | TF fuel level / efficiency warning | fuel | **unavailable** | Per-TF fuel not stored. |
| `reserve_fleet_icon` (iconType, tooltip `TASK_FORCE_IS_RESERVE_FLEET`) | TF is a reserve fleet | visibility | **unavailable** | Reserve flag not in the TF shape. |
| `naval_headquarter_icon` (iconType) | TF at a naval HQ | visibility | **unavailable** | Not stored. |
| `ships_compact` / `ships_detailed` (containerWindowType) → `ship_entry_compact` / `ship_entry_detailed` rows | `task_forces[].ships[]` (+ live `order_of_battle.ships[]`) | row factory | **exact (enumeration)** | One ship row per ship; populated at runtime into these containers. |

### `ship_entry_detailed` / `ship_entry_compact` (the SHIP row templates, repeated per ship)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `ship_name` (instantTextboxType) | `task_forces[].ships[].name` | loc | **exact** | Ship name (save). |
| `ship_type` (instantTextboxType, placeholder `"test"`) | `ships[].definition` → ship-class name; live `ships[].category` | category (def→loc) | **derived** | Ship class from the definition / live category. |
| `ship_button` / `ship_icon` (buttonType, `GFX_navalcombat_ship_icon_unknown`) | `ships[].definition`/`category` → hull icon | category | **derived** | Hull icon from class. |
| `bar_str` (iconType, `GFX_navieslist_bar_str`) | save: `fleets[].task_forces[].ships[].strength` / max; live: `order_of_battle.ships[].strength_hp` / max HP | progressbar-fraction | **derived** | **[Codex: the two channels carry DISTINCT leaves — don't conflate.]** Save TF-ships carry `strength` (fraction); live per-ship records carry `strength_hp` (absolute HP). Numerator exact in its channel; max (denominator) derived from class. |
| `bar_org` (iconType, `GFX_navieslist_bar_org`) | `fleets[].task_forces[].ships[].organisation` (save) / live `order_of_battle.ships[].organisation` / max org | progressbar-fraction | **derived** | Org bar; org exact in either channel; max derived. |
| `bar_planes` / `plane_icon` (iconType) | carrier ship plane load | progressbar | **unavailable** | Carrier air complement not on the ship shape; needs the carrier-air-wing join (View 7). |
| `xp_bar` (iconType, `GFX_unit_xp_bar`) | `ships[].experience` | progressbar-fraction | **derived** | XP exact (M/S); bar fraction derived. |
| `ship_level` (iconType, `GFX_unit_level`) | ship XP level | int (frame) | **derived** | From `experience`. |
| `pride` (iconType, `GFX_NV_icon_pride`) | ship == `extra.pride_of_the_fleet` (`{id,type}`) | visibility | **derived** | Pride flag. |
| `repair_symbol` / `damaged_part_report` (iconType) | ship is under repair / has critical damage | visibility | **unavailable** | Repair state + critical-damage report are runtime; not stored. |
| `exile_force_flag_frame` / `foreign_ship_flag` (iconType) | ship owner ≠ `<TAG>` (live ships carry no owner) | flag | **unavailable** (live) / **derived** (save) | Live `ships[]` carry NO owner; owner needs the save OOB join. |

### Mission/target action bars (`action_bar_window`, `target_bar_window`)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `taskforces_selected` (instantTextboxType, `"Missions - All taskforces"`) | the currently-selected task-force set | loc | **unavailable** | Selection is a UI-interaction state, not snapshot data. |
| `btn_mission0..9`, `btn_target_*`, `btn_remove_regions` (buttonType) | mission/target assignment verbs | — | **deferred: mutation** | All assign a mission/target → inert in replay. The *current* mission is shown per-TF (above), not via these toggles. |

## 2b. Navies — deferred (needs imperative adapter / mutation)

| gui element group | why deferred |
|---|---|
| **Task-force toolbar** (`tool_bar_window`: `btn_delete`, `btn_create_new_from_selected`, `btn_split_in_half`, `btn_select_half`, `btn_exclude`, `btn_merge`, `btn_taskforce_o_matic`, `btn_base_underway_replenishment`, `btn_upgrade`, `btn_repair_now`, `btn_base_reinforce`, `btn_base_toggle_split_for_repair`, the 3 dropdowns `defensive_stance_dropdown`/`repair_priority_dropdown`/`aggressiveness_dropdown` + their `*_item` row templates) | **mutation** — every button creates/splits/merges/upgrades/repairs/sets-policy a task force. The dropdown OPTIONS (`defensive_stand_item`/`repair_priority_item`/`aggressiveness_item`) are static option sets, not country data; the SELECTED policy per TF is not stored → inert. |
| **Composition editor** (`task_force_composition_editor_window` + `…_entry`/`…_new_entry`/`…_sub_unit_*`/`…_role_icon_entry`/`task_force_template_entry`, `ddb_templates`) | **mutation** — designs/edits a task-force composition template (add/remove/+/−/save/delete/role-pick). Saved templates are not snapshot data. |
| **Move-ships dialog** (`move_ships_window` → `ships_grid` → `move_ship_entry`) | **mutation** — moves ships between fleets. The ship LIST it shows binds the same `ships[]` source, but the dialog is a move action → deferred. |
| **Home-base picker** (`home_base_selection_window` → `home_base_selection_grid` → `naval_base_selection_item`, the 3 base-filter `checkboxType`) | **mutation** — set fleet home base; base list is a port adapter (province/naval-base catalog) we do not fold. |
| **Refit/upgrade picker** (`ship_refit_equipment_view` → `equipments_grid`, `outdated_equipment_checkbox`) | **mutation** — ship refit; equipment list = the variant catalog (View 15) in a refit context. |
| **Ship-history / sunk-ship logs** (`ship_history_entry` + `medal_dropdown`/`medals_grid`; `sunk_ship_entry`) | **deferred: needs adapter** — per-ship kill/medal HISTORY is a running ledger reconstructable from the diff timeline (`convoy_sunk`/combat events), not a stored per-ship log. Medal assignment is a mutation. |
| **Navy theater chips** (`navy_theater_fleet_item_small`, `compact_fleet_view_entry`) | **derived (deferrable)** — per-theatre fleet-strength chips; bindable from `order_of_battle.navy_theaters[]` + `fleets[]`, but rendered in the theatre selector (View 11) — mapped there. |

## 2c. Navies — STATIC / decoration / navigation (no binding)

- **Backgrounds / chrome / frames:** every `Background`/`background` (`GFX_tiled_window_transparent`, `GFX_tiled_window_thin_border2`, `GFX_fleet_bg`(+`_hl`), `GFX_task_force_bg`(+`_hl`), `GFX_naval_unit_ship_entry`(+`_highlight`), `GFX_task_force_actions_bg`, `GFX_air_priorities_tiled_bg`, `GFX_theatre_naval_control_panel`, `GFX_mini_tooltip`, `GFX_deploy_port_fleet_bg`, scroll-bar bgs, dialog `GFX_tiled_window*_border`s), `navy_box_background`, `current_mission_bg`, `naval_unit_leader_frame`, `skill_icon`/`land_battle_leader_skill_bg`, the `leader_stat_*_icon` set, all `hint_highlight_mission_*` / `btn_mission*_warn` / `naval_dominance_texticon*` chrome, `paper_stain`, the `sunk_ships_entry_divider*`, `small_flag_frame`.
- **Static label texts (loc):** `MOVE_SHIPS_TITLE`, `TASK_FORCE_COMPOSITION_EDITOR_TITLE`, `NAVAL_SUB_UNIT_DEF_SELECTION_WINDOW_TITLE`, `HOMEBASE_WINDOW_TITLE`, `SUBJECT/FACTION/ACCESSIBLE_NAVAL_BASES`, `OUTDATED_EQ`, `REFIT_DESCRIPTION`, `REFIT_NO_SHARED_MATCHES`, `SELECT`/`SELECT_ALL`/`CLICK_TO_ASSIGN`/`FE_CANCEL`/`OK`, `LEADER_DESC`.
- **Navigation / nav-verb buttons (inert):** `btn_close`/`deselect_btn`/`btn_cancel`/`close`, `air_base_btn`/`air_base` (jump to base), `ship_button`/`navy_item_bg`/`leader_button`/`btn_trait_tree`/`btn_advisor_tree`/`history_report_button`/`btn_composition_editor`/`select_btn`/`select_all_btn`, the scrollbar `extendedScrollbarType` parts (Slider/Track/Decrease/Increase ×2), the `dropDownBoxType`/`expandButton`/`expandedWindow` shells, `positionType` anchors, `padding`.
- **Mutation-inert (read-only):** all mission/target/toolbar/editor/move/refit/medal/checkbox/rename buttons enumerated in §2b; `available_medals`, `btn_repair_now`, `btn_refit`.

### Navies element accounting — `stats.element_type_counts`

`containerWindowType` 40, `background` 38, `buttonType` 77, `iconType` 117, `instantTextboxType` 39,
`OverlappingElementsBoxType` 1, `margin` 12, `gridBoxType` 10, `slotsize` 12, `dropDownBoxType` 5,
`expandButton` 5, `expandedWindow` 5, `extendedScrollbarType` 2, `slider` 2, `track` 2,
`decreaseButton` 2, `increaseButton` 2, `editBoxType` 3, `borderSize` 12, `instantTextBoxType` 2,
`positionType` 4, `checkboxType` 3, `gridboxtype` 2, `padding` 1 = **398 total**.

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** (incl. derived/unavailable) | **~40** | `fleet_view_header` (name/insignia/tf-count/regions/pride + leader-fields unavailable), `taskforce_view_entry` (name/mission + activity/fuel/dominance unavailable), `ship_entry_detailed`+`ship_entry_compact` (name/type/icon/str-bar/org-bar/xp/level/pride + repair/foreign unavailable) |
| **Deferred: needs adapter / mutation** | **~8 groups (~210 elements)** | TF toolbar (+3 dropdowns + items), composition editor (+entries), move-ships dialog, home-base picker, refit picker, ship-history/sunk-ship logs, mission/target action bars |
| **Static / decoration / navigation / structural** | **~148** | 38 backgrounds, 117 icons (mission/dominance/fuel/frame chrome), nav buttons, 10+2 grid factories + 12 slotsizes, 12 margins, 2 scrollbars, dropdown shells |

> Row-factory ↔ entry-template map: `detailed_fleet_box`→`fleet_view_header`(+`taskforce_view_entry`,
> fleets/task forces); TF `ships_compact`/`ships_detailed`→`ship_entry_{compact,detailed}` (ships);
> `traits_grid`→admiral trait cells; the 3 toolbar dropdown `button_grid`→`{defensive_stand,
> repair_priority,aggressiveness}_item`; `move_ships_window/ships_grid`→`move_ship_entry`;
> editor `ships_grid`→`task_force_composition_editor_entry`; `templates_grid`→`task_force_template_entry`;
> `sub_unit_box`→`task_force_composition_editor_sub_unit_entry_window`; `home_base_selection_grid`→
> `naval_base_selection_item`; `equipments_grid`→refit equipment cells; `navy_box`
> (OverlappingElementsBoxType)→selected-TF chips.

---

# VIEW 3 — AIR GROUP (`game_assets/gui/air_group.json`, from `interface/airgroupview.gui`)

Small view: a scrollable list of AIR GROUPS, each group expanding to a compact wing strip. Root list
`air_groups_view` (`air_group_view_entries` smoothListBoxType) instantiates `air_group_view_entry` per
group; each group's `wings_compact` grid instantiates `wing_entry_compact` per member wing. Source =
the country's air groups (an OOB grouping over `order_of_battle.air_wings[]`, owner=`<TAG>`).

## 3a. Air group — dynamic (data-bound)

### `air_group_view_entry` (the AIR GROUP row template, repeated per group)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `unit_name` (editBoxType, placeholder `"Unnamed Air Group"`) | the air group's name | loc | **unavailable** | Air-GROUP names are not a stored OOB leaf (`air_wings[]` carry wing names, not a group name) → "—". The grouping itself is derivable; its name is not. |
| `insigna` (buttonType, `GFX_theatre_army_shield`) | group insignia | category | **derived** | Insignia frame (grouping marker). |
| `wings_compact` (gridBoxType) → `wing_entry_compact` rows | iterate the group's member `air_wings[]` | row factory | **exact (enumeration)** | One compact wing chip per member wing. |
| `fuel_stocks_icon` (iconType, `GFX_fuel_extended_strip`) | group fuel | fuel | **unavailable** | Per-group fuel not stored. |
| `fuel_efficiency_warning_icon` (iconType) | fuel-efficiency warning | visibility | **unavailable** | Runtime warning; not stored. |
| `select_btn` (buttonType, text `SELECT`) / `deselect_btn` (buttonType, tooltip `DESELECT_UNIT`) | select/deselect group | — | **deferred: mutation** | Selection verbs (inert). |

### `wing_entry_compact` (the compact WING chip template, repeated per wing)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `wing_button` (buttonType, `GFX_wing_types_map`) | `air_wings[].plane_definition` → wing-type icon | category | **derived** | Wing-type sprite. |
| `planes_count` (instantTextboxType, placeholder `99`) | `air_wings[].count` | int | **exact** | Plane count (save). |

### `air_selection_grid_item` (a selectable group cell on the map; `background_btn` buttonType) — selection chrome; binds no datum (the group identity is positional). **deferred: mutation/nav.**

## 3b. Air group — STATIC / decoration / navigation (no binding)

- **Chrome:** `air_group_view_entry > Background` + `background_sprite` (`GFX_interact_with_air_group`), `wing_entry_compact > background`, `air_groups_view` (list container, no datum).
- **Structural:** `air_groups_view`, `air_group_view_entries` (smoothListBoxType row factory), `wings_compact` (gridBoxType row factory), `wings_compact_number_of_elements` (positionType), `slotsize`.
- **Mutation/nav-inert:** `background_btn`, `insigna`, `select_btn`, `deselect_btn` (select/deselect/jump).

### Air group element accounting — `stats.element_type_counts`

`containerWindowType` 4, `buttonType` 5, `smoothListBoxType` 1, `background` 2, `instantTextboxType` 1,
`iconType` 3, `editBoxType` 1, `gridBoxType` 1, `slotsize` 1, `positionType` 1 = **20 total**.

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** | **~5** | `wing_entry_compact` (wing_button/planes_count), `air_group_view_entry` insignia + wings list; group name unavailable, fuel unavailable |
| **Static / decoration / nav-inert / structural** | **~15** | backgrounds, list/grid factories, select/deselect/insignia verbs, positionType |

> Row-factory ↔ entry-template: `air_group_view_entries` (smoothListBoxType)→`air_group_view_entry`
> (air groups); `wings_compact` (gridBoxType)→`wing_entry_compact` (member wings).

---

# VIEW 4 — AIR SELECTION (`game_assets/gui/air_selection.json`, from `interface/airselectionview.gui`)

The big air management surface — a heterogeneous selection list (`air_selection` smoothListBoxType)
that instantiates one of four row templates per selected unit: **`airbase_selection_item`** (per air
base), **`airwing_selection_item`** (per wing), **`gun_emplacement_selection_item`** (per AA/gun),
**`rocket_selection_item`** (per rocket/missile). Plus a mission toolbar (`missions_grid` →
`air_mission_entry_*`), strategic-bombing priorities, an air-wing action bar, a quick-deploy selector,
and a rocket picker. Binds `<TAG>`. Sources: `order_of_battle.air_wings[]` (S), `strategic_air.air_bases[]`
(S), `extra.special_projects`/rockets, `designs.equipment_variants` (plane/rocket variants).

## 4a. Air selection — dynamic (data-bound)

### `airbase_selection_item` (the AIR BASE row template, repeated per base)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `base_name` (instantTextboxType, placeholder `"Super Maga Wing"`) | the base's state → name (`strategic_air.air_bases[].state`) | loc | **derived** | Base state id → state name. |
| `capacity` (instantTextboxType, placeholder `4444`) | `strategic_air.air_bases[].capacity` (+ `base`/`level`) | int | **exact** | Air-base capacity (save). |
| `owner_flag` (iconType, `GFX_flag_small`) | the base state's owner TAG | flag-small | **derived** | State→owner join. |
| `airbase_friendship_status` (iconType) | base friendship status (own/ally/access) | category | **derived** | Derivable from owner vs `<TAG>` faction/access. |
| `fuel_icon` (iconType, `GFX_fuel_strip`) | base fuel level | fuel | **unavailable** | Per-base fuel not stored. |
| `permission_wings_count` (instantTextboxType, `"100/200"`, tooltip `AIR_VOLUNTEERS_LIMIT`) | volunteer-wing slots used/max at this base | int | **unavailable** | Volunteer-air slot accounting is a runtime cap; not stored. |
| `quick_buttons_container` (gridBoxType) → `quick_wing_deployment_item` rows | quick-deploy plane-type buttons | row factory | **deferred: mutation** | Deploy buttons (inert). |

### `airwing_selection_item` (the AIR WING row template, repeated per wing)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `unit_name` (instantTextboxType, placeholder `"Super Maga Wing 1"`) | `air_wings[].name` | loc | **exact** | Wing name (save). |
| `count` (instantTextboxType) | `air_wings[].count` | int | **exact** | Plane count. |
| `equipment_icon` (iconType, `GFX_technology_medium`) | `air_wings[].plane_definition` → variant icon | category | **derived** | Plane icon. |
| `mission_state` (instantTextboxType, `"Standing by"`) | `air_wings[].mission`/`mission_type` | category/loc | **derived** | Mission state label from the cached mission token. |
| `mission_grid` (gridboxtype) → `air_mission_entry_*` rows | `air_wings[].mission` (active mission) | row factory; category | **derived** | The active-mission chips; the chip SET reflects the cached mission, the toggle is a mutation (inert). |
| `air_mission_efficiency_strip` (iconType) | `order_of_battle.air_wings[].effective_planes` (RUNTIME-cached) vs `count` | progressbar | **proxy** | **[Codex correction: unavailable→proxy.]** `effective_planes` IS a captured field (RUNTIME-cached snapshot of the engine value, S) — render it badged proxy (non-authoritative, like `cached.*`); the displayed mission-EFFICIENCY % (effective/count, or its own denominator) is derived-from-proxy. Not a hard gap. |
| `xp_bar` (iconType, `GFX_unit_xp_bar`) / `xp_level` (iconType) | `air_wings[].experience` | progressbar/int | **derived** | XP exact (S); bar/level derived. |
| `bar_str` (iconType, `GFX_navieslist_bar_str`) | wing strength = `count`/limit | progressbar-fraction | **derived** | Numerator exact; limit not stored. |
| `armies_count_box` (instantTextBoxType, placeholder `28`, tooltip `ATTACHED_AIR_WING_DESC`) | armies the wing is attached to | int | **unavailable** | Wing→army attachment count not in `air_wings[]`. |
| `ace_btn` (buttonType, `GFX_ace_none`) / `ace_kia` (iconType) | wing ace (`extra.aces[]`) | category | **unavailable** | Ace-to-wing join not stored. |
| `carrier_capable_icon` (iconType, tooltip `AIR_CARRIER_CAPABLE`) | carrier-capable plane | visibility | **derived** | From plane definition. |
| `exile_flag` / `exile_flag_frame` (iconType, tooltip `GIE_AIRWING_SELECTION_ITEM_EXILE_FLAG`) | `air_wings[].owner` ≠ `<TAG>` | flag | **derived** | Foreign/exile wing. |
| `air_volunteer_icon` (iconType) | wing is a volunteer wing | visibility | **derived** | Volunteer status from owner relation. |

### `gun_emplacement_selection_item` / `rocket_selection_item` (gun / rocket row templates)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `unit_name` (instantTextboxType) | gun/rocket-wing name (`air_wings[].name` if modeled as a wing, else rocket record) | loc | **exact / unavailable** | Gun emplacements may appear as wings (exact); rocket/missile programs are `extra.special_projects`/nuke records → name **unavailable** (no per-rocket name stored). |
| `count` (instantTextboxType, rocket) | rocket/missile stock | int | **unavailable** | Rocket stock = `extra.nukes_total`/special-project output; not a per-rocket-wing count. |
| `mission_state` (instantTextboxType) | gun/rocket mission | category | **derived / unavailable** | Gun mission derivable if wing-modeled; rocket targeting not stored. |
| `equipment_icon` (iconType) | gun/rocket equipment variant | category | **derived** | Variant icon. |
| `nuclear_button` (buttonType) / `select_missile_button` (buttonType) | arm nuclear / select missile | — | **deferred: mutation** | Arm/select verbs (inert). |

## 4b. Air selection — deferred (needs adapter / mutation)

| gui element group | why deferred |
|---|---|
| **Mission toolbar** (`mission_toolbar > missions_grid` → `air_mission_entry_0..15` + `air_mission_entry_3_target_btn` + `mission_icon_item`) | **mutation** — assign-mission toolbar; one chip per mission-type id. The chips' icons are static mission sprites; assigning is a mutation. The *current* mission per wing is shown in the wing row (4a). |
| **Strategic-bombing priorities** (`strategic_air_priorities_window`/`strategic_gun_emplacement_priorities_window`/`strategic_rocket_priorities_window` → `bombing_priorities_grid` → `bombing_priority_item`) | **mutation** — toggling bombing-target priorities. Targets exist in `raids.targets[]` (S, §16) but the per-country bombing-PRIORITY toggle state is not stored → inert. |
| **Air-wing action bar** (`air_wing_toolbar`: `btn_delete`, `btn_create_air_group`, `split_wing_btn`, `merge_wings_btn`, `merge_groups_btn`, `select_all_wings_btn`, `btn_select_half`, `btn_hold`, `btn_unassign`, `btn_close`, `reinf_btn` + the 4 dropdowns `reinforcement_preference`/`niche_selection`/`aggressiveness`/`day_night` + their `button_grid` row factories) | **mutation** — create/delete/split/merge/hold/unassign/reinforce/set-policy. Dropdown OPTION sets are static; selected policy per wing not stored. |
| **Quick-deploy selector** (`quick_wing_deployment_selection_window` → `plane_selection_items_list` → `quick_wing_deploy_plane_type_entry_item`; `quick_wing_deployment_item`) | **mutation** — deploy new wings; plane-type list from the variant catalog + stockpile. |
| **Rocket picker** (`rocket_selection_window` → `equipment_grid` → `equipment_entry`) | **mutation** — pick a rocket/missile equipment to fire. |
| **Unassigned-wings list** (`unassigned_airwings_view` → `air_wings_list` smoothListBoxType → `airwing_selection_item`) | **derived (bindable)** — unassigned wings = `air_wings[]` with no army/theatre attachment; binds the same wing template (4a). `divisions_count` (`UNASSIGNED_UNIT_DESCRIPTION`) is a count label. |
| **Dropdown option row** (`airwing_dropdown_selection_item`) | static option set (one per dropdown choice); no country datum. |

## 4c. Air selection — STATIC / decoration / navigation (no binding)

- **Backgrounds / chrome:** all `Background`/`background` (`GFX_tiled_window_w_close`, `GFX_tiled_window_1b_thin_border`, `GFX_tiled_plain_bg_small`, `GFX_air_priorities_tiled_bg`, `GFX_mini_bg`, `GFX_mini_tooltip`, `GFX_AV_base_entry_bg`, `GFX_AV_wing_entry_bg`, `GFX_unitlist_unassigned_bg`, dropdown `button_grid` bgs), `btn_bg` row bgs, `create_new_wing_glow`, `selected_air_priority_icon`/`selected_mission_overlay`, `mission_background`, `count_selected`, `niche_icon_bg`, `reinforce_icon`/`priority`/`reinforcement_preference` icon chrome, `stockpille_icon`, `training_will_stop_at_max_icon`.
- **Static label texts (loc):** `QUICK_DEPLOY_SELECTION_WINDOW_DESC`, `AIR_STRATEGIC_PRIORITIES`, `SELECT`/`SELECT_ROCKET`, `UNASSIGNED_UNIT_DESCRIPTION`, `"X divisions"` (placeholder), `"0/3"`/`"100/200"` (placeholders).
- **Navigation (inert):** `close_button`/`btn_close` (`CLOSE`/`DESELECT_UNIT`), `unitinfo_button` (open unit info), `selection_window_button` (open quick-deploy — `QUICK_DEPLOY_SELECTION_WINDOW_OPEN_DESC`), `mission_targets` (open target), dropdown `expandButton`/`expandedWindow` shells, `positionType`, `offset`, `max_slots`, `slotsize`, `margin`.
- **Mutation-inert:** every button enumerated in §4b plus `new_btn`, `sticky_mission_area_btn`, `reinf_btn`, `building_button`, `equipment_button`, `checkbox`, `deploy`.

### Air selection element accounting — `stats.element_type_counts`

`containerWindowType` 36, `iconType` 54, `buttonType` 52, `instantTextboxType` 25, `background` 16,
`listBoxType` 1, `offset` 1, `gridBoxType` 12, `slotsize` 13, `instantTextBoxType` 1, `gridboxtype` 1,
`max_slots` 1, `dropDownBoxType` 6, `expandButton` 6, `expandedWindow` 6, `margin` 7,
`smoothListBoxType` 2, `positionType` 1 = **241 total**.

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** (incl. derived/unavailable) | **~30** | `airbase_selection_item` (name/capacity/owner-flag/friendship + fuel/permission unavailable), `airwing_selection_item` (name/count/icon/mission-state/xp/str + efficiency/armies/ace unavailable), `gun_emplacement`/`rocket_selection_item` |
| **Deferred: needs adapter / mutation** | **~7 groups (~130 elements)** | mission toolbar, 3× bombing-priority windows, action bar (+4 dropdowns), quick-deploy, rocket picker, unassigned-wings list |
| **Static / decoration / navigation / structural** | **~81** | backgrounds, mission/priority chrome, nav buttons, list/grid factories + slotsizes, dropdown shells |

> Row-factory ↔ entry-template: `air_selection` (smoothListBoxType)→`{airbase,airwing,gun_emplacement,
> rocket}_selection_item`; `air_wings_list` (smoothListBoxType)→`airwing_selection_item` (unassigned);
> `missions_grid`→`air_mission_entry_*`; `bombing_priorities_grid` (×3)→`bombing_priority_item`;
> `plane_selection_items_list` (listBoxType)→`quick_wing_deploy_plane_type_entry_item`;
> `quick_buttons_container`→`quick_wing_deployment_item`; `equipment_grid`→`equipment_entry`; each
> dropdown `button_grid`→`airwing_dropdown_selection_item`.

---

# VIEW 5 — AIR WING DETAILS (`game_assets/gui/air_wing_details.json`, from `interface/airwingdetails.gui`)

Single popup `air_wing_stats_window` for ONE air wing: name, type, manpower, equipment ratio, mission
chips, ace, three stat grids (base/combat/misc), and an equipment list. Plus the row template
`air_wing_details_equipment_entry`. Source = one `order_of_battle.air_wings[]` element +
`designs.equipment_variants` for the plane variant; stat grids = `designs.equipment_variant_stats` (D).

## 5a. Air wing details — dynamic (data-bound)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `ship_title` (instantTextboxType, text `AIR_WING_STATS`) | static loc | loc | **static** | Panel title. |
| `air_wing_name` (editBoxType) | `air_wings[].name` | loc | **exact** (display) | Wing name (edit is a mutation, inert). |
| `type_icon` (iconType, `GFX_technology_medium`) | `air_wings[].plane_definition` → variant icon | category | **derived** | Plane-type icon. |
| `carrier_capable_icon` (iconType, tooltip `AIR_CARRIER_CAPABLE`) | carrier-capable plane | visibility | **derived** | From plane definition. |
| `current_manpower` (instantTextboxType, placeholder `"Stuff"`) | `air_wings[].manpower` | manpower | **exact** | Wing manpower (save). |
| `equipment_ratio` (instantTextboxType, placeholder `"Stuff"`) | `air_wings[].count` vs operational target | int/ratio | **derived** | Equipment ratio = on-strength / target; numerator exact, target not stored. |
| `ace_name` (instantTextboxType) / `ace_icon` (iconType, `GFX_ace_none`) | the wing's ace (`extra.aces[]`) | loc/category | **unavailable** | Ace-to-wing join not stored. |
| `mission_grid` (gridboxtype) → mission cells | `air_wings[].mission`/`mission_type` | row factory; category | **derived** | Active-mission chips. |
| `base_stats > stats_grid` / `combat_stats > stats_grid` / `misc_stats > stats_grid` (3× gridboxtype) → stat rows | `designs.equipment_variant_stats["id:type"]` (the wing's plane variant) | row factory; int/float | **derived** | Per-variant base/combat/misc stats are DERIVED (`equipment_variant_stats` is **D**, computed from modules). Labels (`DESIGNER_BASE_STATS_LABEL`/`DESIGNER_COMBAT_STATS_LABEL`/`FE_MISC_STATS`) are static. |
| `equipment > list` (gridboxtype) → `air_wing_details_equipment_entry` rows | `air_wings[].equipment[[id,type,amount]]` | row factory | **exact (enumeration)** | One row per equipment line in the wing. |

### `air_wing_details_equipment_entry` (equipment row template, repeated per equipment line)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `count` (instantTextboxType) | `air_wings[].equipment[].amount` | int | **exact** | Equipment amount. |
| `name` (instantTextboxType) | `air_wings[].equipment[].(id,type)` → `designs.equipment_variants` name | category | **derived** | Variant name lookup. |
| `type` (instantTextboxType) | the equipment archetype/role | category | **derived** | Archetype label. |
| `equipment_icon` (iconType, `GFX_technology_medium`) | variant → sprite | category | **derived** | Icon-map lookup. |
| `carrier_capable_icon` (iconType, tooltip `AIR_CARRIER_CAPABLE`) | carrier-capable | visibility | **derived** | From variant. |

### `stat_equipment_entry_large` (`equipment_icon`/`carrier_capable_icon`/`equipment_name`) — a featured-equipment header card; binds `air_wings[].equipment[0]` → variant name/icon. **derived.**

## 5b. Air wing details — STATIC / decoration / navigation (no binding)

- **Chrome:** `air_wing_stats_bg` (`GFX_air_unit_stats_bg`), `bg` (`GFX_air_unit_stats_equip_entry`), `manpower_icon`, `equipment_icon` header, the three stat-section labels.
- **Navigation / mutation-inert:** `btn_close` (`CLOSE`), `btn_delete` (`AIRWING_DELETE` — disband, mutation), `niche_selection_container`/`niche_selection_position` (positionType anchor for a role picker — mutation).
- **Structural:** `air_wing_stats_window`, `info`, `base_stats`/`combat_stats`/`misc_stats`, `missions`, `equipment`, the 5 `gridboxtype` row factories + `slotsize` (×5) + `max_slots`, `borderSize` (×3).

### Air wing details element accounting — `stats.element_type_counts`

`containerWindowType` 10, `Background` 1, `buttonType` 2, `instantTextboxType` 10, `borderSize` 3,
`iconType` 9, `editBoxType` 1, `gridboxtype` 5, `slotsize` 5, `max_slots` 1, `positionType` 1,
`instantTextBoxType` 1, `background` 1 = **50 total**.

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** (incl. derived/unavailable) | **~18** | air_wing_name, type_icon, current_manpower, equipment_ratio, 3 stat grids (derived), equipment list + `air_wing_details_equipment_entry` (count/name/type/icon), `stat_equipment_entry_large`; ace unavailable |
| **Static / decoration / nav-inert / structural** | **~32** | bg chrome, 3 stat labels, close/delete verbs, 5 grid factories + slotsizes, niche anchor |

> Row-factory ↔ entry-template: `equipment/list`→`air_wing_details_equipment_entry` (equipment lines);
> the 3 `stats_grid`→per-variant stat rows (`designs.equipment_variant_stats`); `mission_grid`→mission
> chips.

---

# VIEW 6 — AIR WING REORGANIZATION (`game_assets/gui/air_wing_reorg.json`, from `interface/airwingreorganization.gui`)

A dual-list MOVE/REORGANIZE dialog (`air_wing_reorganization`): a LEFT source list and a RIGHT
destination list of equipment entries, plus a plane-type filter strip, a split dialog, an exile
manpower-country dropdown, and reinforce-preference strips. This is fundamentally a **mutation** view
(move planes between wings). The data it RENDERS binds the same `air_wings[].equipment` + variant
catalog; the move itself is inert.

## 6a. Air wing reorg — dynamic (data-bound, render-only)

### `air_wing_reorganization_equipment_entry_left` / `_right` (the dual equipment row templates)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `name` (instantTextboxType, placeholder `"fighter"`) | `air_wings[].equipment[].(id,type)` → `designs.equipment_variants` name | category | **derived** | Variant name. |
| `type` (instantTextboxType, placeholder `"Bf109"`) | variant model name | category | **derived** | Model/archetype label. |
| `count` (instantTextboxType, placeholder `10`) | `air_wings[].equipment[].amount` | int | **exact** | Equipment amount. |
| `equipment_icon` (iconType, `GFX_technology_medium`) | variant → sprite | category | **derived** | Icon lookup. |
| `design_team_icon` (iconType) | variant `design_team_mio` | category | **derived** | MIO/design-team icon from the variant catalog. |
| `carrier_capable_icon` (iconType, tooltip `AIR_CARRIER_CAPABLE`) | carrier-capable | visibility | **derived** | From variant. |
| `niche_icon` (iconType) | plane niche/role | category | **derived** | Niche icon from variant role. |
| `missions > mission_grid` (gridboxtype) → `mission_grid_item` rows | the wing's active missions | row factory; category | **derived** | Mission chips. |
| `bg` (buttonType) | select/move this equipment | — | **deferred: mutation** | Move verb (inert). |
| `reinforcement_preference_dropdown` (right only) | per-wing reinforce preference | — | **unavailable** | Reinforce preference per wing not stored. |

### Filter + manpower header

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `plane_type_grid` (gridBoxType) → `airwing_filter_item` rows (`airwing_count`, `airwing_icon`) | per-plane-type tallies over the wings being reorganized | row factory; int + category | **derived** | One filter chip per plane type. |
| `current_manpower_left` / `current_manpower_right` (instantTextboxType, placeholder `"Stuff"`) | source/destination wing manpower (`air_wings[].manpower`) | manpower | **exact** | Per-wing manpower (save). |
| `current_plane_count` (instantTextboxType, placeholder `"WIP"`) | combined plane count | int | **derived** | Sum of the entries' `count`. |
| `manpower_dropdown` → `countries_grid` → `airwing_manpower_item` rows (`flag`, `manpower_text`) | exile manpower-source countries | row factory; flag + manpower | **derived** | Country list for exile manpower; flags + a per-country manpower figure. |

## 6b. Air wing reorg — STATIC / decoration / navigation / mutation (no binding)

- **Chrome:** `Background`s (`GFX_tiled_plain_bg`, `GFX_tiled_window2_1b_border`, `GFX_mini_bg`, `GFX_mini_tooltip`, `GFX_tiled_window_1b_thin_border`), `filter_background`/`prio_background`, `current_plane_coun_background`/`generic_w_box_right`, `manpower_icon_left`/`_right`, `current_plane_icon`, `highlight`.
- **Static label texts (loc):** `BUTTON_OK` (confirm), the reinforce-pref tooltips (`AIR_REINFORCE_OBSOLETE_ONLY`/`_ANY`/`_ONLY_NEW`/`_ONLY_SAME`).
- **Mutation / nav-inert:** `confirm_btn` (`BUTTON_OK` — confirm reorg), `close_button`/`close_btn` (`CLOSE`), split dialog (`strategicair_split_wnd`: `confirm_btn`, `split_slider` scrollbar + `landslider_*` guiButtons, `add_planes_1/2`, `remove_planes_1/2`), `reinforce_preferences` strip (`reinforce_1..4`), all `bg` move buttons, dropdown `expandButton`/`expandedWindow` shells.
- **Structural:** `air_wing_reorganization`, `plane_filters`, `prio_container`, `list_left`/`list_right` (+ their `list` gridBoxType row factories), `reinforcement_preferences_position` (positionType), `slotsize`, `margin`, `max_slots`, `borderSize`.

### Air wing reorg element accounting — `stats.element_type_counts`

`containerWindowType` 14, `buttonType` 14, `background` 10, `instantTextboxType` 15, `iconType` 18,
`scrollbarType` 1, `borderSize` 2, `guiButtonType` 4, `gridBoxType` 4, `slotsize` 6, `positionType` 2,
`margin` 4, `dropDownBoxType` 2, `expandButton` 2, `expandedWindow` 2, `gridboxtype` 2, `max_slots` 2 =
**104 total**.

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** (render-only, incl. derived) | **~20** | the dual `…_equipment_entry_left/right` (name/type/count/icon/design-team/niche/missions), `airwing_filter_item`, manpower header (left/right/plane-count), `airwing_manpower_item` |
| **Static / decoration / nav / mutation-inert / structural** | **~84** | chrome, split dialog (scrollbar + add/remove planes), reinforce-pref strips, confirm/close, list factories, dropdown shells |

> Row-factory ↔ entry-template: `list_left/list`→`…_equipment_entry_left`; `list_right/list`→
> `…_equipment_entry_right`; `plane_type_grid`→`airwing_filter_item`; `countries_grid`→
> `airwing_manpower_item`; the two `mission_grid`→`mission_grid_item`.

---

# VIEW 7 — CARRIER AIR WING (`game_assets/gui/carrier_air_wing.json`, from `interface/carrierairwingcompositionwindow.gui`)

Small dialog `carrier_air_wing_composition_window`: a list of plane-type rows with +/− amount editors
and a capacity summary. **Mutation** view (sets a carrier's air complement). Renders the carrier's
plane-type allocation; the allocation source would be the carrier ship's air complement, which is **not
stored** (the per-ship shape carries no air-wing composition).

## 7a. Carrier air wing — dynamic (data-bound, render-only)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `title` (instantTextboxType) | the carrier's name (`order_of_battle.…ships[].name`) | loc | **derived** | Title "<carrier> air wing" from the ship name. |
| `summary` (instantTextboxType, placeholder `"0/100"`) | planes assigned / carrier capacity | int (`n/m`) | **unavailable** | Carrier air capacity + assigned count is a runtime composition; not stored per ship. |
| `carrier_capacity_icon` (iconType, `GFX_MODIFIER_CARRIER_CAPACITY_PENALTY_REDUCTION`) | carrier capacity-penalty modifier | category | **unavailable** | Capacity-penalty modifier is runtime; not stored. |
| `list` (gridBoxType) → `carrier_air_wing_composition_entry` rows | per-plane-type composition slots | row factory | **unavailable** | The carrier's per-plane-type allocation is not stored (no ship→air-complement leaf) → rows render "—". |

### `carrier_air_wing_composition_entry` (plane-type row template)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `equipment_name` (instantTextboxType, placeholder `"Bf109"`) | the plane-type variant name | category | **unavailable** | Composition is not stored; the variant catalog has names but there is no carrier-allocation to bind them to. |
| `equipment_icon` (iconType, `GFX_wing_types_map`) | plane-type icon | category | **unavailable** | As above. |
| `number_box` (editBoxType, placeholder `0`) | assigned count of this plane type | int | **unavailable** | Per-carrier allocation not stored. |
| `increase_amount` / `decrease_amount` (buttonType, tooltip `CARRIER_AIR_WING_INCREASE`/`_DECREASE`) | +/− the allocation | — | **deferred: mutation** | Allocation editors (inert). |

## 7b. Carrier air wing — STATIC / decoration / navigation (no binding)

- **Chrome:** `Background` (`GFX_tiled_window_w_close`), `entries > Background`, `summary_bg` (`GFX_generic_w_box`), `bg` (`GFX_reorganize_airwings_new_wing_entry`), `number_box_bg`.
- **Navigation / mutation-inert:** `close_button` (close), `confirm` (`OK` — apply, mutation), `increase_amount`/`decrease_amount`.
- **Structural:** `carrier_air_wing_composition_window`, `entries`, `list` (gridBoxType row factory), `slotsize`, `borderSize`.

### Carrier air wing element accounting — `stats.element_type_counts`

`containerWindowType` 3, `background` 2, `instantTextboxType` 3, `buttonType` 5, `gridBoxType` 1,
`slotsize` 1, `iconType` 4, `editBoxType` 1, `borderSize` 1 = **21 total**.

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** (mostly unavailable) | **~6** | title (derived); summary/capacity-icon/composition rows (`carrier_air_wing_composition_entry`) all **unavailable** — carrier air complement not stored |
| **Static / decoration / nav-inert / structural** | **~15** | chrome, close/confirm/+/− verbs, grid factory + slotsize |

> Row-factory ↔ entry-template: `list` (gridBoxType)→`carrier_air_wing_composition_entry` — but the
> per-carrier composition array has no schema home, so the rows are `unavailable` (no fabrication).

---

# VIEW 8 — STRATEGIC AIR (`game_assets/gui/strategic_air.json`, from `interface/strategicairview.gui`)

A per-strategic-region air-dominance readout `strategicairview`: weather backdrop, our/enemy wing-type
strength bars, air-superiority progress, detection/efficiency, a line-chart details panel, and an
air-combat damage breakdown. This is a **READ-ONLY display** view (the only mutations are chart toggles
+ mission-category filters). The CORE air-superiority data is `naval_air.air_superiority`
(`{strategic_region_id: {controller, …}}`, **M, a REGION PROXY** — badged `proxy`, never `exact`).

## 8a. Strategic air — dynamic (data-bound)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `region_name` (instantTextboxType) | the selected strategic-region id → name | loc | **derived** | Region name (static meta). |
| `weather_*` (iconType ×11: none/light_rain/heavy_rain/light_snow/heavy_snow + `_night` variants + `air_weather_frame`) | the region's current weather state | category (weather→sprite) | **unavailable** | Per-region weather is a computed/uncracked value (FULL_GAME_COVERAGE weather wall); not in the snapshot → default sprite. |
| `progress` / `progress_disabled` (iconType, `GFX_strategicair_progress`) | air-superiority progress for the region | progressbar | **proxy** | `naval_air.air_superiority[region]` is a REGION-LEVEL PROXY (M); the superiority value drives the bar, badged proxy (never exact). |
| `wing_type_left_1..4` (iconType, `GFX_wing_types_own`) + `wing_type_left_text_1..4` (instantTextboxType, tooltips `AIR_SUPERIORITY_TYPE_1..4`) | OUR planes per type in the region — SUM `order_of_battle.air_wings[].count` (owner=`<TAG>`) grouped by `plane_definition` where `strategic_region`==region | int | **derived** | **[Codex correction: proxy→derived.]** OUR per-type counts are an aggregate over `air_wings[]` (exact `count` + `strategic_region`, S) — NOT sourced from `naval_air.air_superiority`. The grouping/region-membership is the derived step. |
| `wing_type_friendly_left_1..4` (iconType, `GFX_airbase_friendship_status`) | friendly-presence status per type | category | **derived** | Friendly-presence from the same `air_wings[]` aggregate (owner/faction). |
| `wing_type_right_1..4` (iconType, `GFX_wing_types_enemy`) + `wing_type_right_text_1..4` (instantTextboxType, tooltips `AIR_SUPERIORITY_ENEMY_TYPE_1..4`) | ENEMY planes per type in the region | int | **derived** (proxy if fogged) | ENEMY per-type counts also aggregate over `air_wings[]` (all tags) by region — derived. (In a fog-of-war replay the enemy view could be PROXY/intel-scaled, but the underlying counts are derivable; the snapshot stores REAL counts.) |
| `detection_left_text` / `detection_right_text` (instantTextboxType) | our/enemy air detection in region | int/pct | **unavailable** | Air detection is a runtime per-region value; not stored. |
| `effeciency_left_text` / `effeciency_right_text` (instantTextboxType, `"100%"`) | our/enemy air mission efficiency | pct | **unavailable** | Mission efficiency is RUNTIME (§G). |
| `crypto_broken_icon` (iconType, `GFX_crypto_icon_done_small`) | enemy region intel decrypted | visibility | **derived** | From `intel.decryption` (S) targeting the region's enemy. |
| `our_fighters_count` / `enemy_fighters_count` / `our_bombers_count` / `enemy_bombers_count` (instantTextboxType) | air-combat participant counts | int | **unavailable** | Per-region live air-combat participant counts are runtime (air combat = continuous attrition, no discrete object — MEMORY note); not stored. |
| `our_fighters_damage_list` / `enemy_fighters_damage_list` / `our_bombers_damage_list` / `enemy_bombers_damage_list` (gridboxtype ×4) → `air_combat_damage_received` rows (`damage_type`, `count_txt`) | per-damage-type air-combat losses | row factory; category + int | **unavailable** | Air-combat damage breakdown is a runtime combat ledger; not stored → rows render "—". |
| `our_fighter_equipment_icon*` / `enemy_*` / `our_bomber_*` / `enemy_bomber_*` (iconType, `GFX_technology_medium`) | the combatant plane variants | category | **unavailable** | Needs the live air-combat participant set (not stored). |
| `linechart` (iconType, `GFX_strategicair_linechart`) + `max_value`/`avg_value`/`min_value`/`min_x_value`/`max_x_value` (instantTextboxType) | the region air-history line chart | chart | **unavailable** | The per-region air-superiority HISTORY series is a runtime ledger; reconstructable from the diff timeline only (not a stored chart). |

## 8b. Strategic air — STATIC / decoration / navigation / mutation (no binding)

- **Chrome:** `strategic_air_bg`, `our_label` (`AIR_STRATEGIC_OUR_TEXT`), `enemy_label` (`AIR_STRATEGIC_ENEMY_TEXT`), `air_progress_frame`, all `*_text_bg`/`generic_mini_bg`, `efficiency_left`/`_right` strips, `detection_*_icon`, `linechart_bg`/`linechart_tiling_bg`, `x_axis_type` label, the 24 `channel_*` chart-series toggle windows (each `btn_channel_N` label + `channel_checkbox` + `color_block` — STATIC, hand-declared, not factory).
- **Navigation / mutation-inert:** `close_button` (`CLOSE`), `details_open` (`DETAILS` — open chart), `btn_days`/`btn_months`/`btn_years` (chart granularity), `our_missions_btn`/`enemy_missions` (toggle), `mission_btn_1/2/3` (filter by mission category — mutation), the `channel_checkbox`es (toggle series visibility).
- **Structural:** `strategicairview`, `details_wnd`, `air_combat_wnd`, `mission_btn_win`, `airbase_margin` (positionType), the 4 `gridboxtype` damage factories + `slotsize` (×4).

### Strategic air element accounting — `stats.element_type_counts`

`containerWindowType` 30, `iconType` 75, `instantTextboxType` 28, `background` 3, `buttonType` 29,
`ButtonType` 29, `gridboxtype` 4, `slotsize` 4, `positionType` 1 = **203 total**.

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** (proxy/derived/unavailable) | **~40** | region_name (derived), air-superiority progress (proxy), our/enemy wing-type counts ×8+8 (proxy), detection/efficiency (unavailable), crypto (derived), air-combat counts + 4 damage lists (unavailable), line-chart values (unavailable) |
| **Static / decoration / navigation / structural** | **~163** | weather backdrop, labels, 24 chart-channel toggles, chart chrome, details/granularity/mission-filter buttons, 4 damage factories |

> **Proxy note (refined per Codex).** The air-SUPERIORITY/DOMINANCE bar (`progress`) is the value from
> `naval_air.air_superiority` — REGION-GRANULARITY LIVE (M), `confidence` ≠ `"exact"` (PLAN §13.1) →
> **proxy**. But the per-type PLANE COUNTS (`wing_type_*_text`) are **derived** aggregates over
> `order_of_battle.air_wings[]` (`count`+`strategic_region`, S) — NOT sourced from `air_superiority`.
> The air-COMBAT breakdown (counts + damage lists + line chart) is genuinely runtime (continuous
> attrition, no discrete combat object) → `unavailable`. Row-factory: the 4 `*_damage_list`
> (gridboxtype)→`air_combat_damage_received`.

---

# VIEW 9 — BATTLE PLAN TOOLS (`game_assets/gui/battle_plan_tools.json`, from `interface/battleplantools.gui`)

A fixed ORDER-DRAWING toolbar `battleplantools_window` (no row factory). Draw-order buttons, execution/
cohesion/leader-proximity config popouts, and a garrison/area-defense config window. Per PLAN §G the
battle-plan ORDER **geometry is partial**: live `order_of_battle.battle_plan_orders[].target_states`
for **type-5 (offensive)** orders is EXACT; other live order arrows/paths are RESISTED unless a
save-backed `armies[].orders[]` measured geometry channel is folded in.

## 9a. Battle plan tools — dynamic (data-bound)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `operation_name` (editBoxType, empty) | the active battle plan's operation name | loc | **derived** | `armies[].orders[].operation` carries an operation name when present; render-only (edit is a mutation). |
| `title` (instantTextboxType, text `BATTLE_PLANS`) | static loc | loc | **static** | Toolbar title. |
| `icon_execute_type` (iconType, `GFX_BPT_execute_strip`) | selected order's execution setting | category (frame) | **unavailable** | Per-order execution/cohesion/proximity SETTINGS are not stored (the live leaf is `{army_id, order_type, target_states}` only). |
| `selected_order_overlay` (iconType, `GFX_selected_order_overlay`) | the currently-selected order — `order_of_battle.battle_plan_orders[].order_type` (live) / `armies[].orders[]` `order_instance` (save) | category | **derived** | Order identity + type. The SAVE `armies[].orders[]` is the richer source (see geometry note). |

> **Order geometry (PLAN §G, badged) — TWO channels.** The draw-order buttons CREATE orders whose
> live ARROW/PATH geometry is the resisted data. **(1) Live** `order_of_battle.battle_plan_orders[]`
> (`construction_orders_live`, M) gives `order_type` (EXACT, all orders) + `target_states` (**EXACT for
> type-5 offensive ONLY**, 107/108 vs save; **null/RESISTED** for blitz/defence/fallback/naval-invasion/
> paradrop/area-defense). So an offensive order's TARGET STATE SET is bindable (shade target states on
> the map) — **derived**; other order geometry **unavailable** in the live channel. **(2) Save**
> `order_of_battle.armies[].orders[]` (S, the `order_instance`) carries the FULLER shape — `type`,
> `dates`, `operation`, `path[]`, `states[]`, `scheduled_members[]`, `root_front` — so when the snapshot
> is SAVE-backed, the order's `path`/`states`/`root_front` ARE available (**derived**, becomes the
> primary order-geometry source). **[Codex (a): made the save `armies[].orders[]` channel explicit.]**
> Native map overlays join live `battle_plan_orders[].target_states` to live `oob_command`
> armies via `member_div_ids` for type-5 offensive target-state arrows. Those arrows are
> still **derived** centroid-to-state geometry, not exact the game order paths.
> If save-backed orders carry measured `order_geometry`, `path_geometry`, or measured path fields
> with real map points, native overlays render that order path as **exact** geometry. Malformed
> exact/measured order geometry fails closed with an unavailable marker and is not replaced by the
> derived save `path[]` or target-state fallback.
> `army_id`/`target_states` (live) are RAW engine ints (not TAGs).

## 9b. Battle plan tools — deferred (mutation; no row factory)

| gui element group | why deferred |
|---|---|
| **Draw-order buttons** (`btn_order_draw_offence`, `btn_order_draw_blitz`, `btn_order_draw_defence`, `btn_order_draw_fallback`, `btn_order_draw_naval_inv`, `btn_order_draw_naval_inv_with_harbor`, `btn_order_draw_paradrop`, `btn_area_defense`, `btn_order_reshape`, `btn_order_delete`, `btn_assign_units`, `btn_unassign_units`) | **mutation** — each draws/assigns/reshapes/deletes a battle-plan order. The order's existence + (for offensive) target-state set surface via `battle_plan_orders[]` (9a); the DRAW action is inert. Only `btn_order_draw_offence` corresponds to a type-5 order whose geometry we can read. |
| **Execution / cohesion / leader-proximity popouts** (`planning_speed_window`: `btn_execute_0/1/2`; `cohesion_window`: `btn_cohesion_0..3`; `leader_proximity_window`: `btn_proximity_0..4`; plus `btn_priority`/`btn_cohesion`/`btn_leader_proximity`/`btn_exercise` toggles) | **mutation** — set the selected order's execution speed / cohesion / leader proximity / exercise. None of these per-order settings are stored → inert. |
| **Garrison/area-defense config** (`battleplantools_area_defense_config_window`: `btn_setting_victorypoints/ports/coastlines/airbases/borders/forts/rail/facility`) | **mutation** — configure a garrison order's target categories. Not stored → inert. |

## 9c. Battle plan tools — STATIC / decoration (no binding)

- **Chrome:** `Background` (`GFX_BPT_bg`), the popout `Background`s (`GFX_BPT_areadef_conf_bg`, `GFX_battle_plan_speed_bg`, `GFX_battle_plan_cohesion_bg`, `GFX_hq_proximity_settings_bg`), `btn_exercise_anim`, `icn_exercise_stop_at_max`, the tutorial glows (`draw_defence_tutorial_glow`/`draw_offense_tutorial_glow`).
- **Static label texts (loc):** `BATTLE_PLANS` (title), `hint` (`?`), the popout titles `EXECUTION`/`COHESION`/`LEADER_PROXIMITY`/`GARRISON_ORDER_CONF`, and the per-button tooltip keys (`PLAN_EXECUTE_*`/`PLAN_COHESION_*`/`LEADER_PROXIMITY_*`/`GARRISON_CONF_*`).
- **Structural:** `battleplantools_window`, `content`, `ability_list_container` (empty — populated by code, no grid), `leader_proximity_button_group`, the 4 popout containers.

### Battle plan tools element accounting — `stats.element_type_counts`

`containerWindowType` 8, `background` 5, `editBoxType` 1, `buttonType` 37, `iconType` 5,
`instantTextboxType` 6 = **62 total**.

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** (incl. unavailable) | **~4** | operation_name (derived), selected_order_overlay (derived order identity), icon_execute_type (unavailable); offensive target_states bindable from `battle_plan_orders[]`; measured save order geometry exact when folded |
| **Deferred: mutation** | **~30** | all draw-order buttons, exec/cohesion/proximity popout buttons, garrison config buttons |
| **Static / decoration / structural** | **~28** | bgs, tutorial glows, title/hint, popout titles, empty ability container, containers |

> **No row factory** — fixed toolbar. The order data this toolbar produces lives in
> `order_of_battle.battle_plan_orders[]` (type-5 target_states EXACT, rest live geometry RESISTED) +
> `armies[].orders[]` (save battle-plan `order_instance`: type/dates/operation/path/states/root_front,
> plus exact measured geometry when folded).

---

# VIEW 10 — ABILITY LIST (`game_assets/gui/ability_list.json`, from `interface/abilitylist.gui`)

A tiny floating ability bar (the on-map command-ability popup: `abilitylist_button_container` +
`abilitylist_window`). Country command abilities (e.g. "Force Attack", commander abilities) are
**activated** abilities with cooldowns — a runtime UI state not stored in the snapshot.

## 10a. Ability list — dynamic / unavailable

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `abilitylist_button` (buttonType, `GFX_ability_icon_default`) | the available command ability | category (ability→icon) | **unavailable** | Activatable command abilities + their active/cooldown state are not stored (no `abilities` snapshot leaf for command abilities; `countries.*.abilities.officer_corps.*` is a different officer-corps domain). |
| `abilitylist_active_bg` / `abilitylist_cooldown_bg` (iconType) | ability active / on-cooldown state | visibility | **unavailable** | Active/cooldown is runtime UI state; not stored. |

## 10b. Ability list — STATIC / decoration

- **Chrome:** `abilitylist_bg_mid` (`GFX_ability_list_mid`), `bg_begin` (`GFX_ability_list_begin`), `bg_end` (`GFX_ability_list_end`).
- **Structural:** `abilitylist_button_container`, `abilitylist_window`.

### Ability list element accounting — `stats.element_type_counts`

`iconType` 5, `containerWindowType` 2, `buttonType` 1 = **8 total**.

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** (all unavailable) | **~3** | `abilitylist_button` + active/cooldown bgs — command-ability state not stored |
| **Static / decoration / structural** | **~5** | begin/mid/end bg chrome, 2 containers |

> No row factory. The entire view is `unavailable` from snapshot data — command-ability activation is a
> sub-sample-transient UI action (SCHEMA §10 class); it renders as inert chrome. **[Codex (a) note: the
> command-POWER resource these abilities spend IS feedable — `countries.<TAG>.military.command_power`
> (M) / `politics.command_power` (M) — but this `.gui` has no command-power textbox; that readout lives
> in the topbar (see `BINDING_MAP_topbar_state.md`), not here. The ABILITIES themselves remain
> unavailable.]**

---

# VIEW 11 — THEATRE SELECTOR (`game_assets/gui/theatre_selector.json`, from `interface/theatreselector.gui`)

The army/navy/air THEATRE outliner. The list container `theatre_selection_window > elements_list`
(gridBoxType) instantiates `theatre_selection_entry` per theatre/strategic region; theatre-group rows
(`theater_group_item` / `navy_theater_group_item` / `air_theatre_item` / reserve / new) carry nested
army/fleet/wing shields via `OverlappingElementsBoxType` overlap-boxes and inner `gridBoxType` row
factories. Sources: `order_of_battle.theatres[]`, `armies[]`, `army_groups[]`, `fleets[]`,
`navy_theaters[]`, `air_wings[]`, all (M/S or S), filtered by owner=`<TAG>`.

## 11a. Theatre selector — dynamic (data-bound)

### `theatre_selection_entry` (the THEATRE/region row template, repeated per theatre)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `name` (instantTextBoxType, placeholder `100%`) | the theatre/region name | loc | **derived** | Theatre name (`theatres[]`) or strategic-region name (static meta). |
| `unit_indicator` (guiButtonType, text `5`) | unit count in the theatre | int | **derived** | **[Codex: `theatres[]` exposes `n_armies`/`n_army_groups`/`area_states[]` — those are the EXACT theatre fields; broader per-tag tallies are in `by_country`.]** Count via `theatres[].n_armies` (+ `by_country.<TAG>` for the army/fleet/wing rollup). |
| `alerts` (OverlappingElementsBoxType) → `theatre_selection_alert_entry` rows | active alerts for the theatre | row factory; category | **unavailable** | Per-theatre alerts (low supply, no orders, combat) are runtime UI alerts; not stored. |
| `priority_1/2/3` (guiButtonType, `GFX_deploy_priority`, tooltips `LOW/MEDIUM/HIGH_PRIORITY`) | theatre supply/reinforce priority | int (frame) | **unavailable** | Per-theatre priority is a UI setting; not stored. |
| `pinned` (checkBoxType, tooltip `THEATRE_PIN_AREA`) | theatre is pinned | bool | **unavailable** | Pin state is UI-state; not stored. |

### Theatre-GROUP row templates (`theater_group_item` army / `navy_theater_group_item` / `air_theatre_item` / reserve / new)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `name_box` / `theater_name_box` / `theatre_name_box` (editBoxType, `NEW_THEATER_GROUP`) | the theatre group's name | loc | **derived** | Group name (`army_groups[].name`/`navy_theaters[].name`); edit is a mutation. |
| `armies_count_box` / `ship_count_box` / `air_wing_count_box` (instantTextBoxType) | units in the group | int | **exact** | From `army_groups[].army_refs` / `navy_theaters[].fleet_refs` / `air_wings[]` membership. |
| `orders_group_box` (army) / `fleets_box` (navy) / `air_groups_box` (air) (OverlappingElementsBoxType) → `orders_group_item` / `navy_theater_fleet_item` / `air_theatre_wing_item` rows | the group's member armies / fleets / wings | row factory | **exact (enumeration)** | One shield per member. |
| `fleets_row_box` (navy, gridBoxType) → `fleet_row_item_box` / `reserve_fleet_row_item_box` rows | fleet rows in the navy theatre group | row factory | **exact (enumeration)** | One row per fleet. |
| `air_groups_row_box` (air, gridBoxType) → `air_theatre_group_row_item` → `air_theatre_group_item` rows | air-group rows | row factory | **exact (enumeration)** | One row per air group. |
| `attack_combat_count` / `defence_combat_count` (instantTextBoxType, placeholder `10`) | active attack/defence combats in the group | int | **derived** | Count of `combats[]` involving the group's units (land combats present; M\* low-confidence). |

### Member-shield row templates (`orders_group_item` army, `navy_theater_fleet_item` fleet, `air_theatre_wing_item`/`air_theatre_group_item` air)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `armies_count_box` / `ship_count_box` / `air_wing_count_box` (instantTextBoxType) | divisions/ships/planes in the member | int | **exact** | From `armies[].member_count` / `fleets[].task_forces[].ships[]` / `air_wings[].count`. |
| `army_item_bg` / `navy_item_bg` / `background` (buttonType, `GFX_theatre_army_shield`/`GFX_wing_types_map`) | the member insignia | category | **derived** | Shield/wing icon. |
| `orders_group_item_training` / `training_icon` (iconType) | member is exercising/training | visibility | **unavailable** | Training/exercise state not stored. |
| `in_combat_icon` / `orders_group_item_border_conflict` / `moving_to_combat_icon` (iconType) | member in combat / border war / moving | visibility | **derived** | Cross-check `combats[]` (in-combat derivable; M\*). |
| `orders_group_no_order` / `generic_mission_icon` (iconType) | member has no order / current mission | visibility/category | **derived** (mission) / **unavailable** (no-order) | Fleet mission from `task_forces[].mission` (derived); "no order" flag is runtime. |

### `strategic_region_item` (`name`, `pinned`, `naval_access_rule`, `alerts`) — a strategic-region selection row; `name` derived (region meta), `pinned`/`naval_access_rule` UI-state (**unavailable**), `alerts` runtime (**unavailable**).

## 11b. Theatre selector — deferred / mutation (no binding)

| gui element group | why deferred |
|---|---|
| **Theatre-group settings** (`theater_group_settings_view`: `name` rename, `btn_delete`, `priority_button_1/2/3` reinforcement-priority, `reinforcement_priority` label) | **mutation** — rename/delete/set-priority a theatre group. |
| **Assign / split / reserve verbs** (`assign_button`/`unmoveable_assign_button`/`new_theater_group_button`, `split_off_btn`, `reserve_fleet_btn`, `settings_button`, `naval_access_rule`, `combat_logs_btn`/`attack_combat_btn`/`defence_combat_btn`) | **mutation/nav** — assign units to a group, split fleets, reserve, open settings/combat-log. |
| **`new_theater_group_item`** (the "+ new theatre group" drop target) | **mutation** — create a theatre group. |

## 11c. Theatre selector — STATIC / decoration (no binding)

- **Chrome:** `theatre_selection_window > Background` (`GFX_tiled_window_transparent`), all `background`/`unmoveable_background` (`GFX_theater_group_bg`, `GFX_theatre_settings_bg`, `GFX_theatre_selection_background`), `selected_theatre_frame`/`frame`/`army_item_frame`/`navy_item_frame`/`selected_group_frame`, `theater_supply_priority`, `theatre_priority_bg`, the alert glows (`theatre_alert_red/green/yellow_glow`, `GFX_warning_icon_l`, `category`), the dropdown bgs (`army_group_box_middle_bg`/`_bottom_bg`).
- **Static label texts (loc):** `REINFORCEMENT_PRIORITY`, `UNASSIGNED_AIR_WINGS`, `RESERVE_NAVY_THEATER_GROUP_NAME`, the priority tooltips (`LOW/MEDIUM/HIGH_PRIORITY`), `THEATRE_PIN_AREA`, `RESERVE_FLEETS_FOR_THIS_THEATER`, `UNITS_VIEW_EXERCISING`.
- **Structural / row factories:** `elements_list` (gridBoxType), `fleets_row_box`/`air_groups_row_box` (gridBoxType), the 14 `OverlappingElementsBoxType` overlap-iterators (armies_box/navy_box/air_box/fleets_box/air_groups_box/orders_group_box/alerts), `slotsize` (×3), the `army_group_box`/`fleet_row_item_box`/`air_theatre_group_row_item` dropdown-body wrappers, `windowType` entry roots, `guiButtonType` priority/indicator buttons.

### Theatre selector element accounting — `stats.element_type_counts`

`containerWindowType` 18, `iconType` 30, `buttonType` 39, `editBoxType` 4, `instantTextBoxType` 18,
`ButtonType` 4, `OverlappingElementsBoxType` 14, `gridBoxType` 3, `slotsize` 3, `background` 1,
`windowType` 2, `guiButtonType` 6, `checkBoxType` 1 = **143 total**.

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** (incl. derived/unavailable) | **~30** | `theatre_selection_entry` (name/unit_indicator + alerts/priority/pin unavailable), the 5 theatre-group templates (name/count/member overlap-boxes + row factories), the 4 member-shield templates (count/insignia + training unavailable, in-combat derived) |
| **Deferred: mutation/nav** | **~3 groups (~30 elements)** | settings popup, assign/split/reserve/combat-log verbs, new-group target |
| **Static / decoration / structural** | **~83** | group/selection bgs, frames, alert glows, 14 overlap-iterators, 3 grid factories, dropdown bodies |

> Row-factory ↔ entry-template: `elements_list`→`theatre_selection_entry` (theatres/regions);
> `fleets_row_box`→`fleet_row_item_box`/`reserve_fleet_row_item_box`; `air_groups_row_box`→
> `air_theatre_group_row_item`; the overlap-boxes→`orders_group_item`/`navy_theater_fleet_item`/
> `air_theatre_wing_item`/`air_theatre_group_item`/`strategic_region_alert_entry`.

---

# VIEW 12 — DIVISIONS SUMMARY (`game_assets/gui/divisions_summary.json`, from `interface/divisions_summary.gui`)

A single ENTRY TEMPLATE `divisions_summary_item` — one unit-type summary chip (icon + count + selection
marker), instantiated once per unit type by a parent grid (the grid lives in the unit-list view, not
this file). Source = per-unit-type tallies over `<TAG>`'s `divisions[]` (grouped by `template_role`).

## 12a. Divisions summary — dynamic (data-bound)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `icon` (iconType) | the unit-type → icon (`divisions[].template_role`) | category (role→icon) | **derived** | Unit-type sprite from the grouped role. |
| `count` (instantTextBoxType) | count of `<TAG>`'s `divisions[]` of this unit type | int | **exact** | Per-type division count (grouping over `divisions[]`, M/S). |
| `selection_marker` (iconType, `GFX_army_badge_division_summary_item_selected`) | this unit-type filter selected | visibility | **unavailable** | Selection is a UI-filter state; not snapshot data (renders default). |
| `button` (buttonType, `GFX_tiled_window_transparent`) | toggle the unit-type filter | — | **deferred: mutation/UI-state** | Filter toggle (inert). |

### Divisions summary element accounting — `stats.element_type_counts`

`containerWindowType` 1, `buttonType` 1, `iconType` 2, `instantTextBoxType` 1 = **5 total**.

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** | **~2** | `icon` (derived role icon), `count` (exact per-type division count) |
| **Static / decoration / mutation-inert / structural** | **~3** | `divisions_summary_item` (container), `selection_marker` (UI-state unavailable), `button` (filter toggle) |

> This file IS a single entry template (the row factory that instantiates it is in the parent unit-list
> view). It repeats once per unit type. `count` exact; `icon` derived; selection UI-state unavailable.

---

# VIEW 13 — RAILWAY GUN (`game_assets/gui/railway_gun.json`, from `interface/railway_gun.gui`)

A railway-gun unit list (`railway_gun_list_view`) with a per-unit row template (`railway_gun_list_entry`)
and a unit-stats popup (`railway_gun_stats_view`) with base/combat/misc stat grids + an equipment grid.
Railway guns are divisions; source = `divisions[]` filtered to `template_role=="railway_gun"` (owner
`<TAG>`) + `designs.division_templates`/`equipment_variant_stats`.

## 13a. Railway gun — dynamic (data-bound)

### `railway_gun_list_entry` (the railway-gun unit row template, repeated per unit)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `unit_name` (instantTextboxType) | `divisions[].template_name` / `division_name` (resolved) | loc | **exact** | Unit name. |
| `unit_icon` (buttonType, `GFX_unit_railway_gun_icon_medium`) | unit type icon | category | **derived** | Railway-gun icon (fixed role). |
| `bar_str` (iconType, `GFX_navieslist_bar_str`) | `divisions[].strength` / template max | progressbar-fraction | **derived** | Strength bar; strength exact (M/S), max derived. |
| `armies_count_box` (instantTextBoxType, placeholder `28`, tooltip `RAILWAY_GUN_ASSIGNED`) | the army the unit is attached to | int | **derived** | Army membership via `armies[].members_resolved`. |
| `army_attachment_button` (iconType, `GFX_theatre_army_shield`) | attached army shield | category | **derived** | Army insignia. |
| `no_orders` / `invalid_orders` (iconType, `GFX_mapicon_unit_no_orders`) | unit has no / invalid orders | visibility | **unavailable** | Order-validity state is runtime; not stored. |
| `low_supply` (iconType, `GFX_unitcounter_supply_status_strip`) | unit low on supply | category | **proxy** | From `supply.by_province` PROXY (M, division-province proxy) — badged proxy. |
| `unitlist_movement_icon` (iconType) | unit moving | visibility | **derived** | `divisions[].moving` (M\*). |
| `unitlist_exiled_icon` (iconType) | exiled unit | visibility | **derived** | Owner ≠ host / exile. |
| `flag` (iconType, `GFX_mapicon_unit_flag`) | `divisions[].owner` | flag | **derived** | Owner flag (foreign/expeditionary railway gun). |
| `unit_land_combat_grid` (gridboxtype) → land-combat indicator rows | the unit's active `combats[]` | row factory; category | **derived** | Land-combat indicators; in-combat from `divisions[].in_combat`/`combats[]` (M\*, low-confidence). |

### `railway_gun_stats_view` (the unit-stats popup)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `railway_gun_title` (instantTextboxType, text `RAILWAY_GUN_UNIT_STATS`) | static loc | loc | **static** | Popup title. |
| `railway_gun_name` (editBoxType) | `divisions[].template_name`/`division_name` | loc | **exact** (display) | Unit name (edit is a mutation). |
| `current_manpower` (instantTextboxType, placeholder `"Stuff"`) | `divisions[].army_manpower.value` | manpower | **exact** | Unit manpower (divisions_full, S). |
| `base_stats > stats_grid` / `combat_stats > stats_grid` / `misc_stats > stats_grid` (3× gridboxtype) → stat rows | `designs.template_stats[template_id]` / `equipment_variant_stats` | row factory; int/float | **derived** | Per-template/variant stats are DERIVED (`template_stats`/`equipment_variant_stats` are **D**). |
| `equipment > equipment_grid` (gridboxtype) → equipment rows | `divisions[].equipment[[archetype,id,type,amount]]` | row factory | **exact (enumeration)** | One row per equipment line. |
| `forts_modifier` (terrain_mod_*_strip iconType ×5) | terrain attack/move/defense modifiers | category (frame) | **unavailable** | Per-unit terrain/fort modifier preview is a runtime-computed stat (§G); not stored. |

## 13b. Railway gun — STATIC / decoration / navigation / mutation (no binding)

- **Chrome:** `Background` (`GFX_tiled_window_1b_thin_border`, `GFX_naval_unit_stats_bg`), `army_group_header`, `convoys_icon`(+`_bg`), `type_icon`, `manpower_icon`, the 5 `terrain_mod_*` strips + bg/frame, the 3 stat-section labels (`DESIGNER_BASE_STATS_LABEL`/`DESIGNER_COMBAT_STATS_LABEL`/`FE_MISC_STATS`), `btn_bg` row bg.
- **Static label texts (loc):** `RAILWAY_GUN_UNIT_STATS`, `UNASSIGNED_RAILWAY_GUNS_DESCRIPTION`, the tooltips (`DISBAND_ALL_UNIT`/`CONFIRMUNASSIGN_RAILWAY_GUN`/`HALT_UNIT`/`DESELECT_UNIT`/`CLOSE`/`RAILWAY_GUN_ASSIGNED`).
- **Navigation / mutation-inert:** `btn_delete` (disband all), `btn_unassign` (unassign), `btn_hold` (halt), `btn_select_half`/`btn_close`/`btn_deselect` (select/deselect), `unit_icon` (select), `railway_gun_name` (rename), the `extendedScrollbarType` `slider` (+ Slider/Track/Decrease/Increase), `transport_weight` (placeholder display) + `selected_division_count` (selection count — UI-state).
- **Structural:** `railway_gun_list_view`, `content_window`, `divisions_window`, `bottom_window`, `forts_modifier`, `info`, `base_stats`/`combat_stats`/`misc_stats`, `equipment`, the 5 `gridboxtype` row factories + `slotsize` (×5), `borderSize` (×2).

### Railway gun element accounting — `stats.element_type_counts`

`containerWindowType` 12, `background` 2, `iconType` 18, `buttonType` 10, `instantTextboxType` 9,
`extendedScrollbarType` 1, `slider` 1, `track` 1, `decreaseButton` 1, `increaseButton` 1,
`instantTextBoxType` 1, `gridboxtype` 5, `slotsize` 5, `Background` 1, `borderSize` 2, `editBoxType` 1
= **71 total**.

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** (incl. derived/unavailable/proxy) | **~18** | `railway_gun_list_entry` (name/icon/str-bar/army-attachment/flag/movement + land-combat grid; low_supply proxy; no_orders unavailable), stats popup (name/manpower + 3 stat grids derived + equipment grid exact; forts unavailable) |
| **Static / decoration / nav / mutation-inert / structural** | **~53** | header/stats bgs, terrain-mod strips, 3 stat labels, disband/unassign/hold/select verbs, scrollbar, 5 grid factories + slotsizes |

> Row-factory ↔ entry-template: the railway-gun list (in `divisions_window`/`content_window`, populated
> at runtime)→`railway_gun_list_entry`; `unit_land_combat_grid`→land-combat indicators; the 3
> `stats_grid`→template/variant stat rows; `equipment_grid`→`divisions[].equipment` rows.

---

# VIEW 14 — DEPLOYMENT (`game_assets/gui/deployment.json`, from `interface/countrydeploymentview.gui`)

The unit DEPLOYMENT QUEUE + division-template sidebar. Main panel `countrydeploymentview`
(`deployments_list_gridbox` → `deploy_entry`/`supply_deploy_entry`/conveyor rows) + a template sidebar
`templatedeploymentwindow` (`templates_grid` → `named_division_template_entry`; a foreign-templates
dropdown; a custom-icon picker). Sources: `countries.<TAG>.military.deployment_queue` (S),
`designs.division_templates[]` (M/S), `economy.construction_queue` for the shared deploy state. Binds
`<TAG>`. Formatters: `int`, `flag`, `category` (template→icon), `progressbar-fraction`.

> **Note — deployment location: RESISTED (like construction).** The live/save deployment queue carries
> the conveyor + amount + priority, but the build-target STATE/location of a deploying unit is RESISTED
> (same province-handle wall as construction; SCHEMA `production.construction_queue` note). The
> `deploy_location_button` shows a target the snapshot does not carry → `unavailable`.

## 14a. Deployment — dynamic (data-bound)

### `named_division_template_entry` (the division-TEMPLATE row template, repeated per template)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `name` (instantTextboxType) | `designs.division_templates[].name` (country=`<TAG>`) | loc | **exact** | Template name (M/S). |
| `counter` (buttonType, `GFX_technology_medium`) | template icon | category | **derived** | Template/role icon. |
| `template_cap` (instantTextboxType, placeholder `10/10`) | `designs.division_templates[].division_cap` (used/cap) | int (`n/m`) | **derived** | Cap from the template (exact); used count = grouping over `divisions[]` (derived). |
| `obsolete_template_warning` (iconType) / `obsolete` (buttonType, `GFX_toggle_obsolete_button`) | `division_templates[].obsolete` | visibility | **exact** | Obsolete flag (M/S); toggle is a mutation. |
| `special_forces_icon` / `allow_as_hq_icon` / `can_paradrop_icon` (iconType) | template flags (`is_army_hq`, special-forces, paradrop-capable) | visibility | **derived** | `is_army_hq` exact (M/S); special-forces/paradrop derivable from the regiment grid. |
| `division_priority_icon` (iconType, `GFX_div_deploy_priority_icon_strip`) | `division_templates[].priority` | int (frame) | **exact** | Template priority. |
| `foreign_template_flag` / `colonial_force_flag_frame` / `exile_force_flag_frame` (iconType) | template `country` ≠ `<TAG>` | flag | **derived** | Foreign/colonial/exile template. |

### `deploy_entry` / `supply_deploy_entry` (the deploy-queue row templates, repeated per queued deployment)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `name` (instantTextboxType, placeholder `"Maus"`) | `deployment_queue[].(template)` → template name | loc | **derived** | Deploying unit name from the queued template (deployment_queue is S; resolve to template name). |
| `army_equipment` (iconType, `GFX_equipment_icon`) | the deploying unit's equipment shortfall icon | category | **derived** | Equipment-state icon. |
| `equipment_progressbar` (iconType, `GFX_deployment_progressbar`) | deployment readiness (equipment filled) | progressbar-fraction | **unavailable** | Per-deployment equipment-fill % + ETA is a runtime computation; not stored. |
| `reinforcements_icon` (iconType) | reinforcement vs new-unit deploy | visibility | **derived** | From the queue entry kind. |
| `low_prio` / `medium_prio` / `high_prio` (buttonType, `GFX_deploy_priority`) | the deployment's priority | int (frame) | **derived** | From `military.reinforcement_priority` / queue entry priority (S). |

### Conveyor rows (`military_deployment_conveyor_view`, `military_deployment_line_view`, `military_deployment_conveyor_summary_view`)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `template_icon` (iconType, `GFX_technology_medium`) | the conveyor's division template | category | **derived** | Template icon. |
| `amount` (instantTextboxType, placeholder `INFINITY`) | conveyor target count (`deployment_queue[].amount`) | int / `∞` | **derived** | Conveyor amount (S); `INFINITY` = uncapped. |
| `generic_name_edit` (editBoxType) | the series name | loc | **derived** | Series name (edit is a mutation). |
| `division_name_dropdown > grid` (gridBoxType) → `division_name_dropdown_entry` rows | available division names | row factory | **unavailable** | The deploy name-list pool is not stored (name-list = a static script asset + used-names state we don't fold). |
| `series_number` / `lines_count` / `add_limit` (instantTextboxType, tooltips `CONVEYOR_*`) | conveyor series counters | int | **derived** | Counts over the conveyor's lines (deployment_queue). |
| `army_training` / `training_progressbar` (iconType) | training progress of the deploying unit | progressbar-fraction | **unavailable** | Per-line training progress is runtime; not stored. |

### `foreign_template_item` (foreign-template dropdown row: `owner_flag`, `name`, `current_autonomy_level_icon`)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `name` (instantTextboxType) | a foreign/subject country's `division_templates[].name` | loc | **derived** | Foreign template name (subject/expeditionary). |
| `owner_flag` (iconType, `GFX_flag_small2`) | the template's owner TAG | flag-small | **derived** | Owner flag. |
| `current_autonomy_level_icon` (iconType) | the subject's autonomy level | category | **derived** | From `diplomacy_world.subjects` / `politics.autonomy`. |

## 14b. Deployment — deferred / mutation (no binding)

| gui element group | why deferred |
|---|---|
| **Add / deploy / cancel verbs** (`add_template_button` `DEPLOYMENT_ADD_TEMPLATE`, `edit_button` `EDIT`, `delete_button`, `open_designer_button` `DEPLOYMENT_DESIGNER`, `add_button` `ADD_CONVEYOR_LINE_BUTTON_LABEL`, `deploy_line_button`, `cancel_line_button`, `deploy_conveyor_button`, `cancel_conveyor_button`, `increase_amount`/`decrease_amount`, `priority_up_btn`/`priority_down_btn`, `deploy_location_button`, `deploy_group_button`) | **mutation** — add a template to the queue, deploy/cancel a line/conveyor, set amount/priority/location/group, open the designer. |
| **Custom-icon picker** (`div_templ_custom_icon_view` → `symbols_grid`, `back`/`default`) | **mutation** — choose a division symbol (mutates the template). `symbols_grid` iterates symbol assets (no country datum). |
| **Show-decommissioned toggle** (`show_decommissioned_templates_window`: `checkbox` + `SHOW_DECOMMISSIONED_TEMPLATES`) | **UI-state** — filter toggle. |

## 14c. Deployment — STATIC / decoration / navigation (no binding)

- **Chrome:** `Background` (`GFX_tiled_window`, `GFX_tiled_window2_1b_border`, `GFX_tiled_generic_overlay_bg1`, `GFX_tiled_window_1b_thin_border`, `GFX_deploy_reinforcements_entry`, `GFX_deployment_named_division_bg`(+`_obsolete_bg`), `GFX_mini_tooltip`, conveyor bgs), `production_header_bg`, `deployment_binding`, `template_header_bg`, `deploy_icon_tiled_bg`, `deploy_priority_title_bg`/`deploy_priority_equipment_meter_bg`, `add_limit_bg`, `special_forces_icon` header, `deploy_group_button_icon`, tutorial highlights (`tutorial_highlight_train`/`tutorial_highlight_add_unit`).
- **Static label texts (loc):** `DEPLOYMENT_DEPLOYMENT_LABEL` (title), `DEPLOYMENT_TEMPLATE_LABEL`, `DEPLOYMENT_DESIGNER`/`EDIT`/`BACK`/`DEFAULT_DIV_TEMPL_SYMBOL`, `SHOW_DECOMMISSIONED_TEMPLATES`, `DEPLOYMENT_LINE_OUT_OF_NAMES`, the `CONVEYOR_*` + `DEPLOYMENT_{LOW,MEDIUM,HIGH}_PRIORITY_DESC` tooltips, `special_forces_cap` placeholder `THINGS`.
- **Navigation / mutation-inert:** `close_button` (`CLOSE`), `conveyor_collapse_btn` (collapse), the `dropDownBoxType`/`expandButton`/`expandedWindow` shells (foreign-templates + division-name), all verbs in §14b.
- **Structural / row factories:** `military_tab`, `divisions`, `templates`, the 2 `gridboxtype` (`deployments_list_gridbox`, `templates_grid`) + 3 `gridBoxType` (`foreign_templates_grid`, `symbols_grid`, name-dropdown `grid`) + `slotsize` (×5), `margin` (×5), `borderSize` (×12), `military_deployment_conveyor_end_view` (spacer template).

### Deployment element accounting — `stats.element_type_counts`

`containerWindowType` 16, `background` 23, `iconType` 39, `instantTextboxType` 15, `borderSize` 12,
`margin` 5, `gridboxtype` 2, `slotsize` 5, `buttonType` 31, `dropDownBoxType` 2, `expandButton` 2,
`expandedWindow` 2, `gridBoxType` 3, `editBoxType` 2 = **159 total**.

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** (incl. derived/unavailable) | **~28** | `named_division_template_entry` (name/icon/cap/obsolete/priority/flags), `deploy_entry`+`supply_deploy_entry` (name/equipment/priority + progress unavailable), conveyor rows (template-icon/amount/counters + training/name-pool unavailable), `foreign_template_item` |
| **Deferred: mutation / UI-state** | **~3 groups (~40 elements)** | add/deploy/cancel verbs, custom-icon picker, show-decommissioned toggle |
| **Static / decoration / navigation / structural** | **~91** | 23 backgrounds, header/priority chrome, title labels, 5 grid factories + slotsizes, dropdown shells, tutorial highlights |

> Row-factory ↔ entry-template: `deployments_list_gridbox`→`deploy_entry`/`supply_deploy_entry` +
> conveyor views (deployment queue); `templates_grid`→`named_division_template_entry` (templates);
> `foreign_templates_grid`→`foreign_template_item`; name-dropdown `grid`→`division_name_dropdown_entry`;
> `symbols_grid`→symbol assets (no datum).

---

# VIEW 15 — EQUIPMENT OVERVIEW (`game_assets/gui/equipment_overview.json`, from `interface/equipmentoverview.gui`)

A compact panel `equipment_overview_window`: a header (manpower + equipment ratio) and a grid
(`equipment_list`) of archetype headers + equipment rows. Sources:
`countries.<TAG>.military.equipment_stockpile[]` (S, each `{equip_id, equip_type, archetype, amount}`)
+ `designs.equipment_variants` (S) for names/icons. Binds `<TAG>`.

## 15a. Equipment overview — dynamic (data-bound)

### Header (`equipment_overview_window`)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `template_title` (instantTextboxType, text `EQUIPMENT_IN_USE_HEADER`) | static loc | loc | **static** | "Equipment in use" header. |
| `template_title_garrison` (instantTextboxType, `EQUIPMENT_IN_GARRISON_HEADER`) | static loc | loc | **static** | Garrison-mode header. |
| `current_manpower` (instantTextboxType, placeholder `"Stuff"`) | `countries.<TAG>.military.manpower.available` (+ deployed) | manpower | **exact** | Manpower readout (S/M\*). |
| `equipment_ratio` (instantTextboxType, placeholder `"Stuff"`) | equipment on-hand vs needed | ratio | **derived** | On-hand (`equipment_stockpile[].amount`, exact) vs total NEED (army requirement — computed, not stored) → ratio derived/approximate. |

### `overview_archetype_header_list_entry` (the ARCHETYPE header row template, repeated per archetype)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `archetype_name` (instantTextBoxType) | `equipment_stockpile[].archetype` → loc | category (archetype→loc) | **derived** | Archetype group label. |
| `archetype_amount` (instantTextBoxType) | SUM `equipment_stockpile[].amount` over the archetype | int | **exact** | Sum of variant amounts in the archetype (measured zero stays 0). |

### `overview_equipment_entry` (the EQUIPMENT row template, repeated per variant in the archetype)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `equipment_name` (instantTextBoxType) | `equipment_stockpile[].(equip_id,equip_type)` → `designs.equipment_variants["id:type"].name` | category (variant→loc) | **derived** | Variant name (custom or archetype name) via catalog lookup. |
| `equipment_amount` (instantTextBoxType) | `equipment_stockpile[].amount` | int | **exact** | Banked amount of this variant (S). |
| `equipment_icon` (iconType, `GFX_equipment_item`) | variant → sprite frame | category (variant→icon) | **derived** | Icon-map lookup from the variant archetype. |

## 15b. Equipment overview — STATIC / decoration (no binding)

- **Chrome:** `unit_stats_list_bg` (×2, `GFX_tiled_window2_1b_border`/`GFX_tiled_window_transparent`), `template_header_bg`, `manpower_icon`, `equipment_icon` (header), `unit_stats_list_entry_bg` (`GFX_unit_stats_list_entry_bg2`/`GFX_mini_tooltip`).
- **Structural:** `equipment_overview_window`, `equipment` (container), `equipment_list` (gridBoxType row factory), `slotsize`, `margin`, `borderSize` (×5).

### Equipment overview element accounting — `stats.element_type_counts`

`containerWindowType` 4, `Background` 4, `iconType` 4, `instantTextboxType` 4, `borderSize` 5,
`margin` 1, `gridBoxType` 1, `slotsize` 1, `instantTextBoxType` 4 = **28 total**.

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** (incl. derived) | **~9** | current_manpower (exact), equipment_ratio (derived), `overview_archetype_header_list_entry` (archetype name derived + amount exact), `overview_equipment_entry` (name derived + amount exact + icon derived) |
| **Static / decoration / structural** | **~19** | 2 titles (static loc), bg chrome, manpower/equipment header icons, grid factory + slotsize/margin/borderSizes |

> Row-factory ↔ entry-template: `equipment_list` (gridBoxType)→`overview_archetype_header_list_entry`
> (per archetype) + `overview_equipment_entry` (per variant). The grid emits a HEADER row per archetype
> then a VARIANT row per `equipment_stockpile[]` element grouped under it. Amounts exact (S); names/icons
> derived (catalog lookup).

---

# Element accounting (provably complete) — all 15 views

Counts are the SUM of each view's `stats.element_type_counts` (the parser's own per-type tally over the
full `.gui` tree, including layout primitives `slotsize`/`margin`/`borderSize`/`positionType`/`offset`/
`max_slots`/`padding`/`slider`/`track`/`*Button` — counted under "static/structural"). Every named
element falls in exactly one bucket.

| # | view (`.gui`) | total elements | dynamic-bound (≈) | deferred (≈) | static/structural (≈) |
|---|---|---:|---:|---:|---:|
| 1 | `army.json` (countryarmyview) | **141** | 38 | — | 103 |
| 2 | `navies.json` (naviesview) | **398** | 40 | 210 | 148 |
| 3 | `air_group.json` (airgroupview) | **20** | 5 | — | 15 |
| 4 | `air_selection.json` (airselectionview) | **241** | 30 | 130 | 81 |
| 5 | `air_wing_details.json` (airwingdetails) | **50** | 18 | — | 32 |
| 6 | `air_wing_reorg.json` (airwingreorganization) | **104** | 20 | — | 84 |
| 7 | `carrier_air_wing.json` (carrierairwingcompositionwindow) | **21** | 6 | — | 15 |
| 8 | `strategic_air.json` (strategicairview) | **203** | 40 | — | 163 |
| 9 | `battle_plan_tools.json` (battleplantools) | **62** | 4 | 30 | 28 |
| 10 | `ability_list.json` (abilitylist) | **8** | 3 | — | 5 |
| 11 | `theatre_selector.json` (theatreselector) | **143** | 30 | 30 | 83 |
| 12 | `divisions_summary.json` (divisions_summary) | **5** | 2 | — | 3 |
| 13 | `railway_gun.json` (railway_gun) | **71** | 18 | — | 53 |
| 14 | `deployment.json` (countrydeploymentview) | **159** | 28 | 40 | 91 |
| 15 | `equipment_overview.json` (equipmentoverview) | **28** | 9 | — | 19 |
| | **TOTAL** | **1654** | **~291** | **~470** | **~923** |

(The "deferred" column folds mutation-modal/adapter groups; many of their constituent buttons are also
counted in static/structural per the parser tally — the column is the logical group count's element
span, not a disjoint partition of the type tally. The totals column is the authoritative per-view
element count from `stats`.)

**Total bindable (dynamic-bound) elements across the 15 views: ~291** (the surfaces a replay actually
feeds — unit/fleet/wing/ship/template/equipment rows, OOB/theatre membership, counts, manpower, stockpile,
air-superiority proxies, battle-plan order identity).

---

## Schema-gap ledger — every needed-but-absent field (no-fabrication)

Each renders "—" per SCHEMA §0.2. Class: **(P) parser-unfed** (becomes exact/derived when the named
domain/join is wired) · **(R) RESISTED** (deliberately not pinned — province/geometry handle) · **(H)
hard limit** (genuinely runtime-computed / sub-sample-transient — SCHEMA §10 / PLAN §G, stays
unavailable by nature).

| view(s) | missing field / readout | class | precise gap |
|---|---|---|---|
| army, navies | army/fleet **leader name / skill / portrait** (`level`, `leader_name`, `skill_text`, `leader_stat_*`) | **P** | `armies[].leader`/`fleets[].leader` are RAW engine ints; the `commanders` domain (§3.9) is not folded into OOB → no name/skill/portrait. Becomes derived once `commanders` is joined by `unit_leader_id`. |
| army, navies | admiral/general **traits** (`traits_grid`) | **P** | needs `commanders.{generals,admirals}.traits[]`. |
| army | **special-forces cap** (`special_forces_cap`) | **H** | tech/manpower-computed cap; not stored. |
| army, navies | army/fleet/wing **modifier strips** (`modifiers_grid`→`modifier_entry`) | **H** | computed combat-modifier aggregates (experience/terrain bonuses); `dynamic_modifiers[]` (S) exists but the army-overview modifier set is a runtime display. |
| army, navies | **colonial / exile / reserve** classification flags | **P** | not stored per army/TF in OOB; the owner-TAG-derived expeditionary flag IS derivable, but colonial/exile/reserve sub-class is not. |
| navies | per-fleet/TF **home base**, **fuel**, **activity** (`home_base_name`, `fuel_icon`, `activity_description`, `dominance`) | **P/H** | home base not in OOB; per-fleet fuel not split from national `fuel`; TF activity/dominance are runtime. |
| navies | per-ship **repair / critical-damage** state (`repair_symbol`, `damaged_part_report`) | **H** | runtime repair state; not stored. |
| navies | live ship **owner** (`foreign_ship_flag` on live `ships[]`) | **P** | live `order_of_battle.ships[]` carry NO owner (save-join only). |
| air (3/4/5), army | air-wing **operational limit / efficiency / effective_planes** (`limit`, `air_mission_efficiency_strip`, `equipment_ratio`) | **H** | `effective_planes` is RUNTIME (§12 note); the operational LIMIT denominator is not stored (only `count`). |
| air (3/4/5) | wing **ace** attachment (`ace`, `ace_btn`, `ace_name`) | **P** | `extra.aces[]` exists (S) but no wing→ace join. |
| air (3/4) | air-GROUP **name**, wing→army **attachment count** (`unit_name` on group, `armies_count_box`) | **P** | air-groups are a derived grouping with no stored group name; wing→army attachment count not in `air_wings[]`. |
| air (4) | base **volunteer-slot** accounting, base **fuel** (`permission_wings_count`, `fuel_icon`) | **H/P** | volunteer-slot cap is runtime; per-base fuel not stored. |
| air (7) | **carrier air-wing composition** (entire `carrier_air_wing_composition_entry` grid + `summary`) | **P** | no ship→air-complement leaf in the per-ship shape; the carrier's plane allocation has no schema home. |
| strategic_air | per-region **weather** (`weather_*`) | **H** | weather temp/phenomenon is a documented compute wall (FULL_GAME_COVERAGE). |
| strategic_air | air **detection / efficiency**, air-COMBAT **counts + damage lists + line chart** | **H** | air superiority is a REGION PROXY (`naval_air.air_superiority`, M); detection/efficiency + the combat breakdown are runtime (air combat = continuous attrition, no discrete object). |
| battle_plan_tools, theatre | battle-plan order **arrow/path GEOMETRY**; per-order execution/cohesion/proximity settings | **R/H** | Live `battle_plan_orders[].target_states` exact set for type-5 (offensive) only; native map renders derived anchor-to-target arrows. Save `armies[].orders[]` measured geometry is exact when folded with real map points. Remaining live order geometry and per-order settings are resisted/not stored. |
| battle_plan_tools | `armies[].orders[]` save **`order_instance`** detail (path/states/scheduled_members) | **P** | save OOB carries it; not folded into the live-primary path — usable when save-backed. |
| ability_list | **command abilities** + active/cooldown state | **H** | activatable command abilities are sub-sample-transient UI actions (SCHEMA §10); no snapshot leaf. |
| theatre | per-theatre **alerts, pin, priority**, member **training/no-order** state | **H** | runtime UI alerts + UI-state toggles; not stored. |
| divisions_summary, others | unit-type **filter SELECTION** state (`selection_marker`) | **H** | UI-interaction state; not snapshot data. |
| railway_gun, units | per-unit **order-validity** (`no_orders`/`invalid_orders`), **terrain/fort modifier** preview, **supply** | **H/proxy** | order-validity + terrain-mod previews are runtime; supply is a division-province PROXY (`supply.by_province`, M, badged). |
| deployment | unit **deploy LOCATION** (`deploy_location_button`), **equipment-fill % / ETA / training progress**, deploy **name-pool** | **R/H** | deploy target location RESISTED (province handle, like construction); fill %/ETA/training are runtime; the name-list pool is a static script asset + used-names state not folded. |
| equipment_overview, navies | equipment **total NEED** (drives `equipment_ratio`, `total_need`) | **H** | army equipment requirement minus stock is a computed demand aggregate; not stored. On-hand amounts ARE exact. |

> **`unavailable` ≠ "missing forever".** Class-P gaps (commanders/order_instance/ship-owner/ace joins,
> carrier composition) become exact/derived once the relevant parser/join is wired. Class-R gaps
> (battle-plan geometry, deploy location) are deliberately resisted (province/geometry handle wall;
> never fabricated). Class-H gaps (effective_planes, weather, air-combat breakdown, modifier
> aggregates, command abilities, equipment need, UI-state) are genuinely runtime-computed or
> sub-sample-transient (PLAN §G / SCHEMA §10) and stay unavailable by nature. Region air-superiority +
> division-province supply are LIVE but **PROXY** (badged, never `exact`). None are fabricated; each
> renders "—" rather than being omitted, so every view's `.gui` tree is provably 100% accounted for.

---

## Cross-cutting fidelity notes

**A. OOB is the spine.** Army/navy/air management + theatre selector are all roster aggregations over
`order_of_battle.*` (M/S live-first for armies/army_groups/fleets/theatres; save-only for
navy_theaters/air_wings; live per-ship `ships[]` M with no owner). The *enumeration* (who/where/how
many) is **exact**; per-element NAMES are exact (save strings) or **derived** (catalog/loc lookups);
LEADERS need the unfolded `commanders` join → **unavailable**; combat/efficiency/modifier overlays are
**runtime → unavailable/proxy**.

**B. Battle-plan partial (PLAN §G).** The single most consequential `unavailable` here is live
battle-plan ORDER geometry: only an OFFENSIVE order's `target_states` survives (type-5, EXACT,
bindable to shade target states); blitz/defence/fallback/naval-invasion/paradrop/area-defense arrows
are RESISTED in the live channel. The save `armies[].orders[]` `order_instance`
(path/states/root_front) is richer and usable when the snapshot is save-backed; measured save
geometry renders exact paths when folded with real map points.

**C. Render-mode duplicate templates.** Several views define collapsed/compact/left-right/reserve/
supply variants of a row entry (`ship_entry_compact`↔`_detailed`; `deploy_entry`↔`supply_deploy_entry`;
the dual `air_wing_reorganization_equipment_entry_left/right`; `reserve_*` theatre rows). These bind
the SAME per-element fields as their canonical template (a render-state choice, not new data) and are
NOT double-counted as distinct bindings.

**D. Proxy badges travel with the data.** `naval_air.air_superiority` (region) and `supply.by_province`
(division-province) are LIVE (M) but carry a `confidence` that is **never `"exact"`** (SCHEMA §13). The
strategic-air superiority readouts and the unit low-supply icons inherit that PROXY badge — the UI must
render them as estimates.

---

## Historical review round (adversarial review)

Historical note: this records a completed adversarial review from the old
Claude-era workflow. Codex-native sessions must use normal sub-agents/partner
review and must not run the old local CLI command.

**Asked:** (a) any MISSED
elements / row-factories across the 15 views; (b) FIELD-PATH correctness vs `replay/schema.py`
FIELD_SOURCES + `SCHEMA.md` (esp. `order_of_battle.{armies,fleets,air_wings,ships,battle_plan_orders}`,
`military.{equipment_stockpile,deployment_queue}`, `designs.{division_templates,equipment_variants,
equipment_variant_stats}`, `strategic_air.air_bases`, `naval_air.air_superiority`); (c) any FALSE
`unavailable` that is actually feedable (a real gap vs a parser-unfed-but-derivable leaf).

**Result:** The review returned a concrete verdict and did not time out.

**AGREED / CONFIRMED:** carrier-air-wing composition is **truly absent** from schema (no ship→
air-complement leaf; the only theoretical path — a reverse join `air_wings[].air_base`→carrier ship id —
does not point at carrier instances) → the composition grid stays `unavailable`; ship-owner gap on the
LIVE `ships[]` (owner save-join only) confirmed; ability rows have no snapshot leaf; deploy-location
RESISTED; equipment total-NEED is a computed demand (unavailable); the OOB enumeration is the spine and
NAMES are exact (save) / leaders need the unfolded `commanders` join.

**REFUTED / corrected (all folded into the tables above, tagged `[Codex …]`):**
1. **Strategic-air per-type plane counts: proxy → derived.** The `wing_type_*_text` counts aggregate
   over `order_of_battle.air_wings[]` (`count`+`strategic_region`, S) — NOT `naval_air.air_superiority`.
   Only the air-superiority/dominance BAR (`progress`) stays proxy. (View 8 rows + proxy note fixed.)
2. **Air-wing `effective_planes`: unavailable → proxy.** It is an explicitly captured RUNTIME-cached
   field (like `cached.*`), so the mission-efficiency strip renders as a proxy (derived-from-proxy for
   the %), not a hard gap. (View 4 row fixed; schema-gap ledger keeps the EFFICIENCY-denominator note.)
3. **Battle-plan geometry: the SAVE `armies[].orders[]` channel made explicit.** `battle_plan_orders[]`
   (live) gives only type-5 `target_states`; but the save `order_instance` carries `path[]`/`states[]`/
   `root_front`/`operation`/`dates`, and native overlays now accept exact measured order geometry
   when folded — so save-backed snapshots DO have fuller order geometry (derived or exact measured).
   (View 9 geometry note rewritten to a two-channel form.)
4. **Navy ship strength: don't conflate `strength` (save TF-ship) with `strength_hp` (live per-ship).**
   They are distinct leaves in distinct channels. (View 2 ship-row str/org bars tightened.)
5. **Theatre `theatres[]` exact fields = `n_armies`/`n_army_groups`/`area_states[]`; broader tallies in
   `by_country`.** (View 11 `unit_indicator` note tightened.)
6. **`command_power` is feedable** (`military.command_power`/`politics.command_power`, M) — noted in
   View 10, but it has no textbox in `abilitylist.gui` (that readout is in the topbar map); the
   ABILITIES stay unavailable.

**Already-correct (Codex flagged, verified present):** `air_wings[].equipment[]` rows ARE mapped in
View 5 (`air_wing_details_equipment_entry`) and View 6 (dual `…_equipment_entry_left/right`);
`divisions_summary` IS centered on `divisions[]` grouped by `template_role` (View 12);
`fleets[].task_forces[]` ARE first-class rows (`taskforce_view_entry`, View 2); `deployment_queue` (S)
IS named as the deploy-queue source (View 14). The army org/str-bar **template-max denominator** is, per
Codex, better attributed to `designs.template_stats` (D) than to `division_templates[]` — the bars stay
**derived** either way (the max is a derived stat, not a stored leaf); noted here without re-badging.
