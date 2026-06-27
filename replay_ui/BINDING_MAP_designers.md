# BINDING MAP — DESIGNERS (tank / plane / division / equipment / equipment-upgrade)

> **What this is.** The COMPLETE, no-truncation data-binding map for the **5 in-game the game DESIGNER
> views** the native replay UI binds in the Phase-3 sweep — the equipment/component designers (tank,
> plane, naval-equipment) and their shared *equipment-upgrade* slider window, plus the *division
> template* (DivisionDesigner) editor. It realises `GAME_UI_REPLICA_PLAN` §C ("comprehensive but
> mechanical binding sweep") + §C.4 row **"Designers"** ("layout + modules exact; computed combat
> stats unavailable §G"). EVERY named element in each `.gui` tree is accounted for, in exactly one
> bucket: **dynamic (data-bound)**, **static/decoration** (no binding), or **deferred (needs
> imperative adapter / mutation-inert)**. Nothing is omitted; where a `.gui` element exists but our
> snapshot has no field for it, the element is listed and **badged `unavailable`** with the precise
> missing field named.
>
> **Binding-rule shape** (PLAN §C.1): `{ target (gui element name+type, sprite/text, loc key), source
> (exact snapshot dotted field path), format (formatter id), fidelity (exact|derived|proxy|
> unavailable) }`. **Source paths** use the SCHEMA dotted form; the authoritative mirror is
> `replay/schema.py` `FIELD_SOURCES` (cross-checked against `replay/SCHEMA.md` §11 / §3.10) — see in
> particular `designs.division_templates`, `designs.equipment_variants` (keyed `"id:type"`),
> `designs.equipment_variants_live` (the DISTINCT live module-catalog leaf), `designs.template_stats`
> (D), `designs.equipment_variant_stats` (D), and per-country `mios`/`scheduled_variants`/`mio_policies`
> (§3.10). `<TAG>` = the player/observed country.
>
> **THE designer hard limit (PLAN §G, SCHEMA §11 note — the spine of this whole map).** The save does
> **not** store a design's derived COMBAT STATS — soft attack, hard attack, defense, breakthrough,
> armor, piercing, hardness, org, max-strength/HP, combat-width, speed, reliability%, air agility/
> air-attack/air-defense, naval gun/torpedo/AA values, range, production cost in IC, resource cost. All
> of these are **runtime-computed** from the module/grid layout + the country's tech/MIO/national
> modifiers by the engine's serialize step; they exist only in the live UI, never in a save and not in
> any standing memory struct we read. They are therefore **`unavailable` (hard limit)** wherever they
> appear (the `base_stats` / `combat_stats` / `misc_stats` / `equipment_stats` / `adjuster_stats` stat
> panels, the division `hardness/org/strength/speed/combat-width` chips, every `production_value` /
> `production_resources` / `refit_cost`, the per-module per-stat icon row, the naval-dominance value).
> What **IS** exact: the module SLOT layout (`modules{slot:mod}`), the upgrade-slider LEVELS
> (`upgrades{name:lvl}` / `equipment_variants_live.upgrade_levels[]`), the division regiment GRID
> (`regiments[{type,x,y}]` + `support[]`), and the design's identity (name / icon / family / archetype
> / version / creator / obsolete / parent / MIO design-team). A WAVE-2 *derived model*
> (`designs.template_stats` / `designs.equipment_variant_stats`, source **D**, computed by
> `models/division_stats` + `models/equipment_stats`) can reconstruct a SUBSET of these stats offline;
> wherever such a model exists the panel is badged **derived (model)**, otherwise **unavailable (hard
> limit)**. The distinction is called out per-row.
>
> **No-fabrication (SCHEMA §0.2, PLAN principle 3):** `null` renders "—", never `0`. `0`/`0.0` is a
> measured zero. A combat stat is **never invented**. A field schema-present but never fed by the
> current parser is badged `unavailable` (not silently dropped).
>
> **Grid row-factory model (PLAN §B.4 / §C.2).** `gridBoxType`/`gridboxtype` are **ROW FACTORIES**, not
> passive containers: the engine instantiates a named ENTRY-TEMPLATE window once per bound array
> element. In these designer files the entry templates are mostly SEPARATE top-level windows (e.g. the
> tank-designer `stats_grid` instantiates `designer_stat_entry`; the division `regiments_grid`
> instantiates `designer_division_regiment_slot_entry`). Each entry template is mapped **once** below;
> at runtime it is **repeated per array element**. The grid container carries no datum — its bound
> CHILDREN (in the entry template) carry the data. Case variants (`gridBoxType`/`gridboxtype`,
> `Background`/`background`, `ButtonType`/`buttonType`, `instantTextboxType`/`instantTextBoxType`) are
> the same engine type and are matched case-insensitively (per-type counts list both spellings; totals
> are unaffected).
>
> **Read-only verb note (PLAN §D).** Every designer button whose in-game role is to CREATE / SAVE /
> DEPLOY / RESET / DUPLICATE / AUTO-DESIGN / ADD-or-REMOVE-MODULE / INC/DEC-UPGRADE / RENAME /
> SET-PRIORITY / ASSIGN-MIO / IMPORT/EXPORT-PRESET / SELECT-3D-MODEL is a **mutation** verb. In a
> replay these render as real chrome but are **inert** (no datum, no action). They are listed under
> each view's static/deferred section, never bound. The *outcome* they would produce (a new variant, a
> retemplated division) surfaces instead via the design snapshot + the diff-event timeline (SCHEMA §9.2
> `design_created` / `division_template_changed` / `mio_leveled`), not via these buttons.

---

## Provenance of the fidelity calls

Grounded in **`replay/schema.py` FIELD_SOURCES + the `Designs` / `Country` dataclass docstrings**
(the STABLE, wiring-complete schema), cross-checked against `replay/SCHEMA.md` §11 (`designs`) and
§3.10 (`mios`/`scheduled_variants`/`mio_policies`), and the model maps
`BINDING_MAP_military_management.md` (the grid row-factory model + the designs/OOB sourcing table)
and `BINDING_MAP_production_construction_decisions.md` (production-line variant join). **Adversarially
reviewed by Codex** (read-only); its verdict and corrections are folded in (see the "Codex round"
section at the end).

**Critical schema facts that drive the badges (verified in `schema.py` FIELD_SOURCES + the `Designs`
dataclass, lines ~1277-1300 + ~453-455 + ~483-493):**

| schema leaf | shape / element | source | consequence in these views |
|---|---|---|---|
| `designs.equipment_variants` (save, keyed `"id:type"`) | `{id, type, archetype, family(`land_vehicle`/`ship`/`plane`/`land_other`), name(custom\|null), version, parent_id, creator(TAG), obsolete, modules{slot:mod}, upgrades{name:lvl}, design_team([id,type]), design_team_mio({tag,key,name,size}\|null)}` | **S** | the SPINE of the tank/plane/equipment + upgrade views: design name/icon/family/archetype/version/creator/obsolete EXACT; **module SLOTS** = `modules{slot:mod}` EXACT; **upgrade SLIDERS** = `upgrades{name:lvl}` EXACT; MIO design-team via `design_team_mio` |
| `designs.equipment_variants_live` (live, keyed bare variant id) | `{<variant_id>: {modules{slot_name:module_name}, upgrade_levels[]}}` — **upgrade NAMES resisted (LEVELS only)**; null until a live read folds it; `equipment_variants_live_source`="M" | **M** | the LIVE channel for the same module/upgrade layout (kept SEPARATE from the save `"id:type"` catalog so key-spaces never collide); upgrade-slider levels live here as a positional `upgrade_levels[]` array (name join needs the archetype def) |
| `designs.equipment_variant_stats` (DERIVED model) | `{"id:type": VariantStats.as_dict()}`; `.*.archetype`/`.*.family` echoed from the variant (S) | **D** | the ONLY non-hard-limit path to a variant's combat numbers (`models/equipment_stats`); `None` until the model runs → stat panels badged **derived (model)** when present, **unavailable (hard limit)** otherwise |
| `designs.division_templates[]` (live-first, save backstop) | `{id, type, name, country(owning TAG), role, regiments[{type,x,y}], support[], regimental_support[], priority, division_cap, obsolete, is_locked, is_army_hq, n_regiments, n_support}` | **M/S** | the DivisionDesigner spine: the regiment GRID (`regiments[{type,x,y}]`), `support[]`, name/role/priority/cap/obsolete/HQ-flag EXACT; **derived combat stats NOT stored** (runtime, §11 note) |
| `designs.template_stats` (DERIVED model) | `{str(template_id): DivisionStats.as_dict()}`; `None` until `models/division_stats` runs | **D** | the ONLY non-hard-limit path to a division's hardness/org/soft-attack/HP/width/speed → the division stat chips + `info` panels badged **derived (model)** when present, else **unavailable** |
| `countries.<TAG>.mios[]` (§3.10) | `{key, id, type, name, icon, level(size=org level), funds, points, research_bonus, task_capacity, unlocked_traits[], allowed_policies[], assigned_equipment([id,type]), assigned_equipment_archetype}` | **S** | the design-team / MIO widgets: company icon, name, org-level, assigned equipment EXACT |
| `countries.<TAG>.scheduled_variants[]` (§3.10) | `{archetype, family, name, icon, obsolete, modules, upgrades, industrial_organisation([id,type]\|null), industrial_organisation_mio}` | **S** | the **historical/preset folder** lists (the "OPEN PRESETS" / `historical_design_entry` rows) — designer PRESETS keyed by archetype/family |
| `countries.<TAG>.military.xp.{army,navy,air}` (§3.2) | float | **S** | the `experience` badge in every designer (`DESIGNER_EXPERIENCE` / `DESIGNER_ARMY_EXPERIENCE`) — army/navy/air XP spendable on variants |
| `mio_live.by_tag.<TAG>` / `.by_id` (§11b, top-level LIVE MIO catalog) | `{...policies[...]}` | **M** | a LIVE backstop for the MIO/design-team widgets (resolved allowed-policy ICON tokens) when the save `mios[]` is absent |

**Genuinely runtime-computed (never stored anywhere) → stay `unavailable` (hard limit) by nature
(PLAN §G):** every designer **combat/base/misc/equipment/adjuster stat row**, the division
hardness/org/strength/speed/combat-width chips, every `production_value` (IC cost) /
`production_resources` (resource cost) / `refit_cost`, the per-module per-stat preview icons
(`stat_icon_*`), the naval-dominance value, the module-validity/requirements icons (a tech/eligibility
COMPUTE), the deploy time/cost on the HQ designer, the `equipment_counter` "X/Y allowed" tallies.
These are serialize-computed UI aggregates with no persisted backing field. Where the WAVE-2 derived
model (`template_stats` / `equipment_variant_stats`, **D**) covers a value it is badged **derived
(model)** instead; that model is `None` until it runs, so absent the model the badge is
**unavailable**.

> **One shared frame across the 3 component designers.** `tank_designer_view`,
> `plane_designer_view` and `countryequipmentdesignerview` (naval) are near-identical trees: the same
> `equipment_modules` blueprint board, the same `equipments` design-dropdown, the same 3-column
> `info` stat panel (`base_stats`/`combat_stats`/`misc_stats`), the same `module_selector_window`
> popup, the same `designer_intel_container` comparison sidebar, the same `referenced_info` compare
> card, the same bottom action strip. They are mapped in FULL for the tank designer (VIEW 1) and the
> plane/equipment views (VIEWS 2/4) record only their DELTAS against it, with their own complete
> element-accounting tables. This keeps the map exhaustive without 3× duplication.

---

# VIEW 1 — TANK DESIGNER (`game_assets/gui/tank_designer.json`, from `interface/tank_designer_view.gui`)

Root `tank_designer_view` (1091×542) + 3 sibling row-template windows (`tank_designer_role_entry`,
`historical_design_entry`, `design_folder`). Binds ONE equipment variant being viewed — call it the
**focused variant** `V` = a key into `designs.equipment_variants` (`"id:type"`, family
`land_vehicle`) for `<TAG>`. Formatters: `loc`, `int`, `pct`, `flag`/`flag-small`, `category`
(role/icon→sprite-frame), `module-slot` (slot→module icon).

## 1a. Tank designer — dynamic (data-bound)

### Title / experience / version / role / name (header)

| gui element (name + type, sprite/text) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `designer_title` (instantTextboxType, `EQUIPMENT_DESIGNER_TITLE`) | static loc | loc | **static** | View label (decoration). |
| `experience` (instantTextboxType, `DESIGNER_EXPERIENCE`, tip `CURRENT_EXPERIENCE`) | `countries.<TAG>.military.xp.army` | int | **exact** | Army XP available to spend (land vehicle). |
| `equipments > name` (editBoxType) | `designs.equipment_variants[V].name` (or archetype loc if `name`=null) | loc | **exact** | The custom design name (custom string or null→"—"/archetype). |
| `equipments > tank_design_icon` (iconType, `GFX_tank_design_icon`) | static frame | category | **static** | Decorative design-type glyph. |
| `equipments > expanded_window > equipments_grid` (gridBoxType) → variant rows | iterate `designs.equipment_variants` filtered to `<TAG>` + family `land_vehicle` | row factory | **exact (enumeration)** | Dropdown list of this country's tank variants; rows reuse the catalog (no dedicated entry template defined inline). |
| `position_version` (instantTextboxType) | `designs.equipment_variants[V].version` | int | **exact** | Variant version number (e.g. "Mk II"). |
| `blueprint_type_name` (instantTextboxType) | `designs.equipment_variants[V].archetype` → loc | loc | **exact** | Archetype name under the blueprint board. |
| `dropdown_tank_roles > role_text` (instantTextboxType) | `designs.equipment_variants[V]` resolved tank ROLE (niche) | loc | **derived** | The chosen tank role; role token is in the variant's niche/role field (creator-set); render via role→loc. |
| `dropdown_tank_roles > tank_role_icon` (iconType, `GFX_land_equipment_role_icons`) | same role token | category (role→frame) | **derived** | Role icon frame. |
| `dropdown_tank_roles > expanded_window > tank_role_grid` (gridBoxType) → `tank_designer_role_entry` rows | the static ROLE CATALOG (all selectable tank roles for the archetype) | row factory | **static (catalog)** | Selectable roles = a static DEF list (game data), not snapshot state; one `tank_designer_role_entry` per role. |
| `niche_button` (buttonType, `GFX_niche_icon_strip`, tip `DESIGNER_SELECT_NICHE`) | `designs.equipment_variants[V]` niche frame | category (niche→frame) | **derived** | Current niche icon (frame from the variant's niche). Click = mutation-inert. |
| `niche_icon_selector > buttons_grid` (gridBoxType) | static niche catalog | row factory | **static (catalog)** | Niche options = static DEF list. |

### Module blueprint board (`equipment_modules`) — the EXACT module slot layout

The blueprint board is a set of fixed `positionType` anchors (`pos_custom_module_slot_window_0..9`)
the engine instantiates a module-slot widget at, one per archetype slot. The slot WIDGET is the
`module_selector_window`-launching button. The **bound datum is `modules{slot:mod}`**.

| gui element (name + type, sprite) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `equipment_modules > equipment_preview > background` (`GFX_generic_tank_blueprint_background`) | static | — | **static** | The blueprint backdrop. |
| `pos_custom_module_slot_window_0..9` (positionType ×10) | one per filled slot of `designs.equipment_variants[V].modules{slot:mod}` (live: `equipment_variants_live[V].modules{slot_name:module_name}`) | module-slot (slot→module icon) | **exact** | Each anchor binds the module occupying that archetype slot → its icon. Empty slot → `GFX_locked_icon` default. The slot→module map is EXACT (save **S** or live **M**). |
| `ypos_module_row_0` (positionType) | static layout anchor | — | **static** | Row Y baseline (`@fixed_btn_mod_row_0`). |
| `equipment_upgrade_0 > name` (`_engine_`) / `equipment_upgrade_0 > level` | `designs.equipment_variants[V].upgrades{"<engine-upgrade>":lvl}` (live: `equipment_variants_live[V].upgrade_levels[i]`) | loc / int | **exact** | Inline upgrade slider #0 (engine): NAME = upgrade key→loc, LEVEL = the level int. EXACT layout/level. |
| `equipment_upgrade_0 > btn_inc` / `btn_dec` (buttonType, `GFX_naval_increase/decrease_amount`) | — | — | **deferred: mutation** | Inc/dec the upgrade level — inert in replay. |
| `equipment_upgrade_1 > name` (`_armor_`) / `equipment_upgrade_1 > level` | `designs.equipment_variants[V].upgrades{"<armor-upgrade>":lvl}` (live: `…upgrade_levels[i+1]`) | loc / int | **exact** | Inline upgrade slider #1 (armor). EXACT. |
| `equipment_upgrade_1 > btn_inc` / `btn_dec` | — | — | **deferred: mutation** | Inert. |

### Module selector popup (`module_selector_window`) — opened when a slot is clicked

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `slot_name` (instantTextboxType, `SHIP_DESIGNER_MODULE_SELECTION`) | the clicked slot's name → loc | loc | **derived** | Slot label (which slot is being filled); derivable from `modules` key. |
| `icon_module_slot` (iconType, `GFX_locked_icon`) / `icn_module_slot_frame` (`GFX_equipment_module_bg`) | the slot's current module icon | module-slot | **derived** | Mirror of the focused slot. |
| `gb_modules` (gridBoxType) → module-option rows | the static MODULE CATALOG eligible for this slot/archetype (game DEF data) | row factory | **static (catalog)** | The list of CHOOSABLE modules is static DEF + tech-gating; the eligibility/validity badge is a compute (unavailable). No dedicated inline entry template (rows are archetype module defs). |
| `btn_back` (buttonType, `REGIMENTS_SELECTION_CANCEL`) | — | — | **deferred: nav** | Close popup. |

### Info stat panels (`info`) — base / combat / misc — THE HARD LIMIT

Three `gridboxtype` `stats_grid` row factories, each instantiating `designer_stat_entry`
(`{label, count}`). The rows = computed stat name/value pairs.

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `base_stats_label` (`DESIGNER_BASE_STATS_LABEL`) / `combat_stats_label` (`DESIGNER_COMBAT_STATS_LABEL`) / `misc_stats_label` (`FE_MISC_STATS`) | static loc | loc | **static** | Column headers. |
| `base_stats > stats_grid` (gridboxtype) → `designer_stat_entry` rows | `designs.equipment_variant_stats[V]` base-stat subset (`models/equipment_stats`, **D**) | row factory; `label`→loc, `count`→num | **derived (model)** when `equipment_variant_stats≠None`, else **unavailable (hard limit)** | Base stats (reliability, weight, etc.) are runtime-computed; only the WAVE-2 derived model reconstructs them. No save field. |
| `combat_stats > stats_grid` (gridboxtype) → `designer_stat_entry` rows | `designs.equipment_variant_stats[V]` combat-stat subset (**D**) | row factory | **derived (model)** / **unavailable (hard limit)** | Soft/hard attack, defense, breakthrough, armor, piercing — the textbook COMPUTED-NOT-STORED case. Never from the save. |
| `misc_stats > stats_grid` (gridboxtype) → `designer_stat_entry` rows | `designs.equipment_variant_stats[V]` misc-stat subset (**D**) | row factory | **derived (model)** / **unavailable (hard limit)** | Misc (speed, fuel use, manpower, training time) — same hard limit. |

### `designer_stat_entry` (stat ROW template, repeated per stat — mapped once)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `label` (instantTextboxType, `Name`) | the stat's name → loc (per `equipment_variant_stats[V]` key) | loc | **derived (model)** / **unavailable** | Stat label; exists only if the derived model is present. |
| `count` (instantTextboxType, `count`) | the stat's value (per `equipment_variant_stats[V]` value) | num/pct | **derived (model)** / **unavailable (hard limit)** | The COMPUTED stat value — never fabricated. |

### Design team / MIO + cost strip

| gui element (name + type, sprite) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `design_company_icon` (iconType, `GFX_idea_porsche`) | `designs.equipment_variants[V].design_team_mio.icon` (→ `countries.<TAG>.mios[]` by key) | category (mio icon) | **derived** | The assigned design-company (MIO) idea icon. |
| `design_team_button` (buttonType, `GFX_idea_slot_tank_manufacturer`) | `designs.equipment_variants[V].design_team_mio.{name,key}` → `countries.<TAG>.mios[]` | loc/category | **derived** | MIO name/icon via the design-team join. Click = mutation-inert. |
| `upgrade_assigned_industrial_org` (buttonType, `GFX_prod_line_target_strip`) | the assigned MIO's org level (`mios[].level`) | int (frame) | **derived** | MIO size/level strip. |
| `tag_icon_bg` (iconType, `GFX_role_icon_bg`) | static | — | **static** | Role-icon backing. |
| `production_resources` (instantTextboxType, tip `EQUIPMENT_DESIGNER_PRODUCTION_RESOURCE_DESC`) | resource cost of `V` | resource-list | **unavailable (hard limit)** | Per-variant resource cost is runtime-computed; not stored (not even in the derived model unless `equipment_variant_stats` carries it). |
| `production_value` (instantTextboxType) | IC production cost of `V` | num | **unavailable (hard limit)** | Production cost in IC is runtime-computed. |
| `refit_cost` (instantTextboxType, `Refit Cost: XXXX`) | refit cost of `V` vs base | num | **unavailable (hard limit)** | Refit cost is a runtime delta compute. |
| `resources_bg` (iconType, `GFX_resources_bg`) / `experience_bg` (`GFX_generic_box_smallest`) | static | — | **static** | Backings. |

### Comparison sidebar (`designer_intel_container`) + reference card (`referenced_info`)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `our_filter_entry > name` (`OUR_EQUIPMENT`) / `foreign_filter_entry > name` (`FOREIGN_EQUIPMENT`) / `obsolete_filter_entry > name` (`OBSOLETE_EQUIPMENT`) | static loc | loc | **static** | The 3 filter-tab labels (UI filters, inert). |
| `gridbox_container > acquired_variants_grid` (gridBoxType) → variant rows | iterate `designs.equipment_variants` for `<TAG>` (own) + creator≠`<TAG>` (foreign) + `obsolete=true` per filter | row factory | **exact (enumeration)** | The comparison list of acquired variants; name/icon/creator/obsolete EXACT, combat compare numbers = the derived model or unavailable. |
| `country_filter_container > variant_country_filter_gridbox` (gridBoxType) → flag rows | distinct `designs.equipment_variants[*].creator` TAGs | row factory; flag | **derived** | One flag chip per creator country (filter). |
| `referenced_variant_name` (instantTextboxType, `EQUIPMENT_DESIGNER_TITLE`) | the compared variant's `name` | loc | **exact** | Name of the variant being compared against. |
| `referenced_info` `ref_base_stats`/`ref_combat_stats`/`ref_misc_stats` (gridboxtype ×3) → `designer_stat_entry` rows | `designs.equipment_variant_stats[<compared>]` (**D**) | row factory | **derived (model)** / **unavailable (hard limit)** | Compare card's stat columns — same hard limit as the main `info` panel. |
| `referenced_info` `base_stats_label`/`combat_stats_label`/`misc_stats_label` + `stat_card_header` | static loc / sprite | loc | **static** | Card headers + frame. |

### `historical_design_entry` (PRESET ROW template — for the OPEN-PRESETS folder, mapped once)

| gui element (name + type, sprite) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `name` (instantTextboxType) | `countries.<TAG>.scheduled_variants[].name` | loc | **exact** | Preset/scheduled-variant name. |
| `icon` (iconType, `GFX_technology_medium`) | `scheduled_variants[].icon` | category | **exact** | Preset icon. |
| `role` (iconType, `GFX_land_equipment_role_icons`) | `scheduled_variants[]` archetype/family→role | category | **derived** | Role glyph from the preset's archetype. |
| `forbidden` (iconType, `GFX_role_forbidden`) | preset role-forbidden flag | visibility | **unavailable** | "Role forbidden" is a tech/eligibility compute, not stored on the preset. |
| `export_to_clipboard_button` / `designer_note_button` / `bin_button` (ButtonType ×3) | — | — | **deferred: mutation** | Export / edit-note / delete preset — inert. |

### `design_folder` (PRESET-FOLDER ROW template, mapped once)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `name` (instantTextboxType) | the preset folder name (from `scheduled_variants[]` grouping) | loc | **derived** | Folder label = a grouping over presets (archetype/family bucket). |

## 1b. Tank designer — STATIC / decoration / navigation / mutation-inert (no data binding)

- **Backgrounds / chrome:** `Background` (`GFX_tank_designer_popup_bg`), `equipment_modules > background`, `equipment_preview > background`, every `border_container`/`bg_tech_button` (`GFX_division_3d_bg`), `equipment_select_model`/`equipment_select_sprite`/`historical_select_design` window backgrounds (`GFX_tiled_window`), `niche_icon_selector > Background`, the `info` panel `Background`s, `referenced_info > background` (`GFX_tank_designer_stat_card`), `designer_intel_container > background`, `name_bg`, `select_model`/`select3d_button` 3D placeholders (`GFX_3d_view_placeholder` — replay has no 3D model viewer), `nubbin`, `tag_icon_bg`, `icon_selection_container` + `btn_equipment_icon` (the 2D icon picker chrome), `equipment_upgrade_*` button-bg.
- **Static label texts (loc):** all the `*_stats_label`, `slot_name`, the 3 intel filter-tab labels, `select3d_button` (`SELECT_3D_MODEL`), `back_button` (`BACK`), `show_all_button` (`BUTTON_SHOW_ALL_DESIGNS`).
- **3D-model / sprite / icon selection subtrees (no replay equivalent — picker chrome, mutation-inert):** the entire `equipment_select_model`, `equipment_select_sprite`, `historical_select_design` (incl. its `folder_container`/`list`/`grid_container`/`buttons_grid` grids, `import_preset_button`/`export_preset_folder_button`), `icon_selection_container`, `select3d_button`/`back_button`. These pick a 3D model / 2D icon / imported preset — pure mutation chrome the replay renders inert.
- **Mutation / action verbs — INERT (read-only, PLAN §D):** `save_to_file_button`, `historical_design_button` (open presets), `reset_button`, `duplicate_button`, `auto_button` (auto-design), `auto_upgrade_button`, `auto_upgraded_checkbox_button`, `dynamic_version_checkbox_button`, `save_button`, `niche_button`, `btn_equipment_icon`, `design_team_button`, `upgrade_assigned_industrial_org`, every `btn_inc`/`btn_dec`, `name_reset`, `toggle_comparison_button`, `close_button`, `close_referenced_info`, `btn_back`.
- **Structural (row factories + layout):** `info`, `base_stats`/`combat_stats`/`misc_stats` containers, all 16 `gridBoxType`/`gridboxtype` (row factories — `stats_grid` ×3+3, `equipments_grid`, `tank_role_grid`, `buttons_grid` ×4, `gb_modules`, `acquired_variants_grid`, `variant_country_filter_gridbox`, `list`), `slotsize` ×16, `positionType` ×11 (10 module anchors + 1 row anchor), `margin` ×3, `borderSize` ×2, `expandedWindow` ×2, `expandButton`, `background_margin`, the `equipment_name_bg`/`equipment_modules` wrappers, `dropDownBoxType` ×2 container shells, `editBoxType` (name search — UI-state).

### Tank-designer element accounting (provably complete) — `stats.element_type_counts`

`containerWindowType` 39, `background` 38, `iconType` 17, `instantTextboxType` 26, `borderSize` 2,
`positionType` 11, `buttonType` 19, `ButtonType` 13, `gridboxtype` 6, `slotsize` 16, `gridBoxType` 10,
`margin` 3, `dropDownBoxType` 2, `editBoxType` 1, `expandedWindow` 2, `expandButton` 1,
`background_margin` 1 = **209 total**.

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** (incl. derived/unavailable) | **~34** | experience, equipments>name + variant dropdown, position_version, blueprint_type_name, role text+icon, niche icon, the 10 module-slot anchors (`pos_custom_module_slot_window_0..9`), the 2 inline `equipment_upgrade_*` name+level (4), the 3 `info` stat-grids + `designer_stat_entry` (mapped once), the 3 `referenced_info` stat-grids, referenced_variant_name, acquired_variants_grid, variant_country_filter_gridbox, design_company_icon, design_team_button, upgrade_assigned_industrial_org, production_resources/value + refit_cost (unavailable), `historical_design_entry` (mapped once), `design_folder` (mapped once) |
| **Static / decoration / nav / mutation-inert / structural** | **~175** | window/panel/blueprint/3D backgrounds, the 3 select-model/sprite/preset picker subtrees, 2D-icon picker, all stat-column labels, ~25 mutation/nav buttons, 16 grid factories + slotsizes, 11 positionType anchors, margins/borders, dropdown shells, name search box |

---

# VIEW 2 — PLANE DESIGNER (`game_assets/gui/plane_designer.json`, from `interface/plane_designer_view.gui`)

Root `plane_designer_view` (1091×542) + 1 sibling row template (`plane_designer_role_entry`).
**Structurally a near-clone of the tank designer** (same `equipment_modules` board, `equipments`
dropdown, 3-column `info` panel, `module_selector_window`, `designer_intel_container`,
`referenced_info`, bottom action strip). Binds the **focused PLANE variant** `V` (family `plane`).
This view records only the DELTAS vs VIEW 1; everything not listed here binds identically to the
matching tank-designer element (same source paths, same hard-limit badges).

## 2a. Plane designer — deltas (dynamic)

| gui element (name + type, sprite/text) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `experience` | `countries.<TAG>.military.xp.air` | int | **exact** | **Delta:** AIR XP (not army). |
| `Background` (`GFX_plane_designer_popup_bg`) | static | — | **static** | Plane popup skin. |
| `equipment_preview > background` (`GFX_generic_plane_blueprint_background`) | static | — | **static** | Plane blueprint backdrop. |
| `pos_fixed_module_slot_window_0..4` + `pos_custom_module_slot_window_0..6` (positionType ×12) | `designs.equipment_variants[V].modules{slot:mod}` (live: `equipment_variants_live[V].modules`) | module-slot | **exact** | **Delta:** plane archetypes have a FIXED-slot row (engine/airframe) + a custom-slot row; both bind the slot→module map. EXACT. |
| `plane_role > role_text` (instantTextboxType) | `designs.equipment_variants[V]` plane ROLE (wing type) | loc | **derived** | **Delta:** plain `plane_role` container (not a dropdown like the tank). |
| `plane_role > role_icon` (iconType, `GFX_wing_types_map`) | same role token | category (wing→frame) | **derived** | Wing-type icon. |
| `niche_button` (buttonType, `GFX_plane_niche_icon_strip`) | variant niche frame | category | **derived** | Plane niche strip. |
| `design_team_button` (buttonType, `GFX_idea_slot_aircraft_manufacturer`) | `designs.equipment_variants[V].design_team_mio` → `mios[]` | loc/category | **derived** | **Delta:** aircraft-manufacturer MIO slot. |
| `info > adjuster_stats > stats_grid` (gridboxtype, header `PRODUCTION_ADJUSTERS_LABEL`) → adjuster rows | `designs.equipment_variant_stats[V]` adjuster subset (**D**) | row factory | **derived (model)** / **unavailable (hard limit)** | **Delta (plane-only):** a 4th "production adjusters" stat grid (cost/output trade sliders). Runtime-computed; derived model or unavailable. |
| `combat_adjuster_header_label` (`PRODUCTION_ADJUSTERS_LABEL`) | static loc | loc | **static** | Adjuster section header. |
| `equipment_select_model > best_match_window > best_match_button`/`best_match_text` (`USE_DEFAULT_MODEL_IN_EQUIPMENT_DESIGNER`) | — | — | **deferred: mutation** | "Use default model" toggle — inert. |

`info` base/combat/misc panels, `equipments` dropdown + variant rows, `module_selector_window`,
`designer_intel_container` (our/foreign/obsolete filters, `acquired_variants_grid`,
`variant_country_filter_gridbox`), `referenced_info` compare card, `position_version`,
`blueprint_type_name`, `design_company_icon`, `upgrade_assigned_industrial_org`,
`production_resources`/`production_value`/`refit_cost`, `niche_icon_selector` catalog, and the
`plane_designer_role_entry` row template (`role`/`name`/`forbidden`) — **all bind exactly as VIEW 1's
counterparts** (component-designer shared frame), incl. the COMBAT-STAT hard limit on every
`stats_grid`.

## 2b. Plane designer — STATIC / decoration / mutation-inert

Same classes as VIEW 1: popup/blueprint/3D backgrounds, `equipment_select_model`/
`equipment_select_sprite`/`historical_select_design` picker subtrees, 2D-icon `icon_selection_container`,
`toggle_model_selector_button`, all stat-column labels, the bottom action strip
(`save_to_file_button`/`historical_design_button`/`reset_button`/`duplicate_button`/`auto_button`/
`auto_upgrade_button`/`auto_upgraded_checkbox_button`/`dynamic_version_checkbox_button`/`save_button`/
`nubbin`), `toggle_comparison_button`/`close_button`/`close_referenced_info`/`back_button`/`btn_back`,
all grid factories + slotsizes + positionType anchors + margins + borders + dropdown/expand shells +
`name` search box.

### Plane-designer element accounting — `stats.element_type_counts`

`containerWindowType` 37, `background` 35, `iconType` 14, `instantTextboxType` 22, `buttonType` 16,
`borderSize` 2, `positionType` 13, `ButtonType` 10, `gridboxtype` 7, `slotsize` 16, `gridBoxType` 9,
`margin` 2, `dropDownBoxType` 1, `editBoxType` 1, `expandedWindow` 1, `background_margin` 1 = **186 total**.

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** (incl. derived/unavailable) | **~33** | experience (air), equipments>name + dropdown, position_version, blueprint_type_name, plane_role text+icon, niche icon, the 12 module-slot anchors, the 3 `info` stat-grids + the 4th `adjuster_stats` grid (+`designer_stat_entry` mapped once), the 3 `referenced_info` stat-grids, referenced_variant_name, acquired_variants_grid, variant_country_filter_gridbox, design_company_icon, design_team_button, upgrade_assigned_industrial_org, production_resources/value + refit_cost (unavailable), `plane_designer_role_entry` (mapped once) |
| **Static / decoration / nav / mutation-inert / structural** | **~153** | popup/blueprint/3D backgrounds, 3 picker subtrees, 2D-icon picker, stat labels + adjuster header, ~22 mutation/nav buttons, grid factories + slotsizes, 13 positionType anchors, margins/borders/shells, name search |

---

# VIEW 3 — DIVISION DESIGNER (`game_assets/gui/division_designer.json`, from `interface/divisiondesignerview.gui`)

Root `countrydivisiondesignerview` (1200×720) + 9 sibling row-template/widget windows
(`niche_selection_widget`, `div_template_3d_model_entry`, `division_equipment_archetype_entry`,
`division_equipment_type_entry`, `division_equipment_entry`, `button_wrapper_container`,
`div_templ_custom_icon_button`, `designer_division_regiment_slot_entry`,
`designer_division_support_slot_entry`, `designer_subunit_category_entry`, `designer_subunit_entry`,
`designer_division_entry`, `designer_div_name_group_entry`, `designer_stat_entry`,
`designer_hq_modifier_entry`, `designer_adjuster_entry`, `designer_adjuster_entry_with_modifier`).
This is the **division TEMPLATE editor** — it binds ONE template `T` = an element of
`designs.division_templates[]` (owner `<TAG>`). It has a `non_hq_view` (normal division) and an
`hq_view` (army-HQ template) tab. Formatters: `loc`, `int`, `pct`, `flag`, `category`,
`grid-cell` (regiment slot at (x,y)).

## 3a. Division designer — dynamic (data-bound)

### Header (title / name / symbol / priority / niche / experience)

| gui element (name + type, sprite/text) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `division_title` (instantTextboxType, `DIVISION_DIVISION`) / `hq_title` (`DESIGNER_HQ_TITLE`) | static loc | loc | **static** | View label (normal vs HQ tab). |
| `experience` (instantTextboxType, `DESIGNER_EXPERIENCE`, tip `CURRENT_EXPERIENCE`) | `countries.<TAG>.military.xp.army` | int | **exact** | Army XP for templates. |
| `division_names > name` / `non_hq_view > divisions > name` (editBoxType) | `designs.division_templates[T].name` | loc | **exact** | The template name (custom string). |
| `division_names > expanded_window > names_grid` (gridBoxType) → `designer_div_name_group_entry` rows | the name-list groups available to `<TAG>` (static DEF name lists) | row factory | **static (catalog)** | Selectable division name-lists = static game DEF, not snapshot state. |
| `non_hq_view > divisions > expanded_window > divisions_grid` (gridBoxType) → `designer_division_entry` rows | iterate `designs.division_templates[]` for `<TAG>` (the template picker) | row factory | **exact (enumeration)** | Dropdown of this country's templates; one `designer_division_entry` each. |
| `div_templ_symbol_button` (buttonType, `GFX_unit_super_heavy_armor_icon_medium`) | `designs.division_templates[T]` symbol/icon | category (symbol→frame) | **derived** | The template's unit symbol. Click = mutation-inert. |
| `division_priority_icon` (iconType, `GFX_division_priority_icon_strip`) | `designs.division_templates[T].priority` | category (frame) | **exact** | The template's production/reinforcement priority frame. |
| `priority_button_1/2/3` (buttonType ×3, `GFX_div_priority_strip`) | `designs.division_templates[T].priority` (selected one highlighted) | category (frame) | **exact** | 3 priority buttons; selected = `priority`. Click = mutation-inert. |
| `niche_button` (buttonType, `GFX_niche_icon_strip`, tip `DESIGNER_SELECT_NICHE`) | `designs.division_templates[T]` niche/role | category (frame) | **derived** | Template niche icon. |
| `niche_icon_disabled` (iconType, `GFX_red_cross_small`) | niche-disabled flag | visibility | **unavailable** | "Niche disabled" is a tech/eligibility compute. |
| `can_paradrop_icon` (iconType, `GFX_can_paradrop_icon_small`) | template-can-paradrop flag | visibility | **derived** | Derivable from the template's regiment composition (paratrooper battalions). |
| `special_forces_icon` (iconType, `GFX_special_forces_icon`) | template-is-special-forces flag | visibility | **derived** | Derivable from regiment roles (special-forces battalions). |
| `colonial_force_flag` (iconType, `GFX_flag_small`) / `colonial_force_flag_frame` | template colonial-force `<TAG>` flag | flag | **unavailable** | Colonial-force classification not stored per template. |
| `equipment_warning` (iconType, `GFX_warning_icon_l`) | template has an equipment-allowance warning | visibility | **unavailable** | A runtime validity compute. |
| `niche_icon_selector > buttons_grid` (gridBoxType) | static niche catalog | row factory | **static (catalog)** | Niche options = static DEF. |

### Regiment / support GRID (`subunits`) — the EXACT template layout

This is the **spine of the division designer**: 3 grids form the (x,y) battalion grid + support
column + regimental-support row, each a ROW FACTORY of slot widgets.

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `subunits > regiments_grid` (gridboxtype) → `designer_division_regiment_slot_entry` rows | `designs.division_templates[T].regiments[{type,x,y}]` | row factory; grid-cell at (x,y) | **exact** | The battalion grid — each `regiments[]` element places a slot at its (x,y). EXACT layout (M/S). |
| `subunits > support_grid` (gridboxtype) → `designer_division_support_slot_entry` rows | `designs.division_templates[T].support[]` | row factory | **exact** | The support-company column — one slot per `support[]` element. EXACT. |
| `subunits > regiment_support_grid` (gridboxtype) → `designer_division_support_slot_entry` rows | `designs.division_templates[T].regimental_support[]` | row factory | **exact** | Regimental-support row (1.16+). EXACT (schema carries `regimental_support[]`). |
| `subunits > support_header_label` (`SUPPORT_HEADER`) / `regimental_support_header_label` (`REGIMENTAL_SUPPORT_HEADER`) | static loc | loc | **static** | Column headers. |
| `combat_width_view > combat_width_text` (instantTextboxType, placeholder `15`) | template combat width | int | **derived (model)** / **unavailable (hard limit)** | Combat width is runtime-computed (sum of regiment widths × modifiers); `template_stats[T].combat_width` if the model ran, else unavailable. |

### `designer_division_regiment_slot_entry` (BATTALION SLOT template, repeated per `regiments[]` element)

| gui element (name + type, sprite) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `name` (instantTextboxType, `Maus`) | `designs.division_templates[T].regiments[i].type` → loc | loc | **exact** | The battalion type occupying this cell. |
| `unit_icon` (buttonType, `GFX_locked_icon`) | `regiments[i].type` → unit icon | category | **derived** | Unit-type icon (battalion→icon). Click = mutation. |
| `glowup` (iconType, `GFX_spirit_glow`) / `Background` / `invisible_icon` | static cell chrome | — | **static** | Slot frame/glow. |
| `unit_icon_hq_remove` (buttonType, `GFX_remove_icon`) | — | — | **deferred: mutation** | Remove battalion (HQ view) — inert. |

### `designer_division_support_slot_entry` (SUPPORT SLOT template, repeated per `support[]` element)

| gui element (name + type, sprite) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `name` (instantTextboxType) | `designs.division_templates[T].support[i]` (or `regimental_support[i]`) → loc | loc | **exact** | The support company in this slot. |
| `unit_icon` (buttonType, `GFX_locked_icon`) | `support[i]` → icon | category | **derived** | Support-company icon. Click = mutation. |
| `glowup` / `Background` / `invisible_icon` | static | — | **static** | Slot chrome. |

### HQ view grids (`subunits_hq`) — same factory model

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `subunits_hq > regiments_grid_lead` / `regiments_grid` / `support_grid` (gridboxtype ×3) → regiment/support slot entries | `designs.division_templates[T].regiments[]`/`support[]` (where `is_army_hq=true`) | row factory; grid-cell | **exact** | HQ-template composition; same slot entries, EXACT layout. The HQ tab shows iff `T.is_army_hq`. |
| `hq_header > … hq_composition_label` (`DESIGNER_HQ_COMPOSITION`) / `hq_battalions_label` / `hq_support_header` / `hq_modifiers_header` | static loc | loc | **static** | HQ section headers. |
| `hq_modifiers_list > content` → `designer_hq_modifier_entry` rows | HQ army-modifier list for `T` | row factory; `text`→loc | **unavailable (hard limit)** | HQ modifiers (army-wide bonuses the HQ grants) are runtime-computed from the HQ composition; not stored on the template. |
| `combat_width_view_hq > combat_width_text` | HQ template combat width | int | **derived (model)** / **unavailable** | As `combat_width_view` (model or unavailable). |

### Stat panels (`info`) — base / combat / equipment / adjuster — THE HARD LIMIT

Four `gridboxtype` `stats_grid` row factories (`base_stats`, `combat_stats`, `equipment_stats`,
`adjuster_stats`), each instantiating `designer_stat_entry` / `designer_adjuster_entry`.

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `base_stats_label` (`DESIGNER_BASE_STATS_LABEL`) / `combat_stats_label` / `equipment_stats_label` (`DESIGNER_EQUIPMENT_STATS_LABEL`) | static loc | loc | **static** | Column headers. |
| `base_stats > stats_grid` → `designer_stat_entry` | `designs.template_stats[T]` base subset (`models/division_stats`, **D**) | row factory | **derived (model)** / **unavailable (hard limit)** | Division base stats (max-org, recovery, HP, supply use) — runtime-computed; the WAVE-2 model or unavailable. |
| `combat_stats > stats_grid` → `designer_stat_entry` | `designs.template_stats[T]` combat subset (**D**) | row factory | **derived (model)** / **unavailable (hard limit)** | Soft/hard attack, defense, breakthrough, armor, piercing — COMPUTED-NOT-STORED. |
| `equipment_stats > stats_grid` → `designer_stat_entry` | `designs.template_stats[T]` equipment subset (**D**) | row factory | **derived (model)** / **unavailable (hard limit)** | Required-equipment rollup (per-type counts the template needs); aggregable from `regiments[]`+`support[]` → derived. |
| `adjuster_stats > stats_grid` → `designer_adjuster_entry` rows | per-TERRAIN movement/attack/defense modifiers for `T` (`template_stats[T]` terrain subset, **D**) | row factory; movement/attack/defense strips | **derived (model)** / **unavailable (hard limit)** | Terrain adjusters are runtime modifier computes; derived model or unavailable. |

### Division stat chips (`hardness_stat`/`org_stat`/`strength_stat`/`speed_stat`) — THE HARD LIMIT

| gui element (name + type, sprite) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `hardness_stat > stat_text` (placeholder `87%`, icon `GFX_armor_softness_icon`) | `designs.template_stats[T].hardness` (**D**) | pct | **derived (model)** / **unavailable (hard limit)** | Hardness — runtime-computed. |
| `org_stat > stat_text` (icon `GFX_organization_texticon`) | `designs.template_stats[T].organisation` (**D**) | pct/int | **derived (model)** / **unavailable (hard limit)** | Max org — computed. |
| `strength_stat > stat_text` (placeholder `100`, icon `GFX_strenght_texticon`) | `designs.template_stats[T].max_strength` (**D**) | int | **derived (model)** / **unavailable (hard limit)** | Max strength/HP — computed. |
| `speed_stat > stat_text` (icon `GFX_MODIFIER_ARMY_SPEED_FACTOR`) | `designs.template_stats[T].speed` (**D**) | pct | **derived (model)** / **unavailable (hard limit)** | Speed — computed. |

### Equipment-variant assignment popup (`division_equipment_view`) + cost strip

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `division_equipment_view > equipment_variants > grid` (gridBoxType) → `division_equipment_entry` rows | the equipment variants allowable for `T`'s regiments (`designs.equipment_variants` for `<TAG>` matched to required archetypes) | row factory | **derived** | Variant-allowance list; name/icon/niche EXACT, the "is allowed" checkbox = a per-template policy bit (see below). |
| `division_equipment_archetype_entry > archetype_name` (instantTextBoxType) | required archetype → loc | loc | **exact** | Archetype group header. |
| `division_equipment_type_entry > type_name` + `equipment_counter` (`0/0`, tip `ALLOWED_EQUIPMENT_OF_TYPE`) | equip-type → loc; allowed/total count | loc / int | **exact** name / **unavailable** counter | Type name EXACT; the "X/Y allowed" tally is a runtime allowance compute. |
| `division_equipment_entry > name` (`USS Enterprise`) + `equipment_icon` + `equipment_niche_icon` | `designs.equipment_variants[v].name` / `.archetype`→icon / niche | loc/category | **exact** | One row per allowable variant. |
| `is_type_allowed_button` / `is_allowed_button` (buttonType, `GFX_generic_checkbox3_orange`) | template's allowed-equipment policy bits | checkbox-state | **unavailable** | The per-template "reinforce with this equipment" toggles are a designer-policy set not surfaced in `division_templates[]` (only the grid/support/role are). |
| `new_equipment_allowed_text` (`NEW_EQUIPMENT_ALLOWED`) / `foreign_equipment_allowed_text` (`ALLOW_FOREIGN_EQUIPMENT`) | the two global allow-flags for `T` | checkbox-state | **unavailable** | Same — designer policy flags not in the template record. |
| `cost > cost_bg` / `production_resources` (tip `DIVISION_PRODUCTION_RESOURCE_DESC`) / `production_value` (tip `DIVISION_PRODUCTION_COST_DESC`) | template production cost/resources | num/resource-list | **unavailable (hard limit)** | Division build cost is runtime-computed (sum of equipment IC). |
| `hq_view > deploy_cost_container > hq_deploy_cost` (icon `GFX_command_power`) / `deploy_time_container > hq_deploy_time` | HQ deploy CP cost / time | num | **unavailable (hard limit)** | HQ deploy cost (command power) + time are runtime computes. |

### `designer_subunit_category_entry` / `designer_subunit_entry` (BATTALION-PICKER row templates — for `designerregimentswindow`)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `name` (instantTextboxType) + `subunit_icon` (`GFX_technology_medium`) | the choosable battalion/support type → loc + icon | loc/category | **static (catalog)** | The PICK-A-BATTALION list = static game DEF (all sub-units for the archetype), gated by tech (eligibility = compute). Not snapshot state. |
| `cost` (instantTextboxType, `Maus`) | the sub-unit's build cost | num | **unavailable (hard limit)** | Cost is runtime-computed. |
| `special_forces_icon` / `can_paradrop_icon` (iconType) | sub-unit special-forces / paradrop flags | visibility | **static (def)** | Sub-unit class flags from the static unit DEF. |

### `div_template_3d_model_entry` / `div_templ_custom_icon_button` (3D-model + custom-symbol picker rows)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `div_template_3d_model_entry > name` + `sprite` | static 3D-model catalog | loc | **static (catalog)** | 3D-model picker — no replay equivalent; static list. |
| `div_templ_custom_icon_button > btn` (`GFX_div_templ_0_large`) | static symbol catalog | category | **static (catalog)** | Custom-symbol picker — static DEF. |

## 3b. Division designer — STATIC / decoration / navigation / mutation-inert

- **Backgrounds / chrome:** `Background` (`GFX_division_designer_popup_bg`), `hq_overlay_images` (`hq_composition_background`/`hq_modifiers_background`/`hq_header_background`/`hq_header_bar`), `division_icon_bg`, `division_priority_bg`, `niche_icon_bg`, `template_emblem_slot` (`division_template_icon`/`hq_badge`), `composition_icon`, `amsb_support_panel_ext`, `equip_header`, the `info` panel `Background`s + `name_bg`s, `cost_bg`/`deploy_cost_bg`/`deploy_time_bg`, every stat-chip `*_stat_bg`, `select_model`/`3d_view` 3D placeholders, `equipment_warning` frame, `div_designer_colonial_flag_frame2`.
- **Static label texts (loc):** `division_title`/`hq_title`, `division_composition_label` (`DESIGNER_DIVISIONAL_COMPOSITION`), all `hq_*` headers, `support_header_label`/`regimental_support_header_label`, `regiment_desc_title` (`REGIMENT_WINDOW_DESC`), all `*_stats_label`, `new_equipment_allowed_text`/`foreign_equipment_allowed_text`, `best_match_text`, `equipment_categories > name` (`EQUIPMENT_CATEGORIES_AUTO`), the `div_templ_custom_icon_view` `DEFAULT_DIV_TEMPL_SYMBOL`/`BACK` labels.
- **3D-model / custom-icon / category picker subtrees (no replay equivalent, mutation-inert):** `div_template_select_model` (incl. `3d_view`, `buttons_grid`, `equipment_categories` dropdown + `equipment_categories_grid`, `reset_equipment_categories_filter_button`, `best_match_window`), `div_templ_custom_icon_view` (incl. `symbols_grid`, `default`/`back`), `toggle_model_selector_button`, `select_model`.
- **Mutation / action verbs — INERT (read-only, PLAN §D):** `duplicate_button`, `reset_button`, `save_button` (`DESIGNER_ARMY_SAVE`), `save_and_deploy_button` (`DESIGNER_HQ_SAVE_AND_DEPLOY`), `division_equipment_btn` (open equipment view), the 3 `priority_button_*`, `niche_button`, `div_templ_symbol_button`, `is_allowed_button`/`is_type_allowed_button`/`foreign_equipment_allowed_button`, `toggle_collapse_button`, `close_button` (×2), `cancel_button`, `back_button`/`back`/`default`, `unit_icon`/`unit_icon_hq_remove` (per slot), every `expand_button`, the `niche_selection_widget` dropdown.
- **Structural (row factories + layout):** `niche_selection_widget`, `button_wrapper_container`, all 20 `gridBoxType`/`gridboxtype` (`regiments_grid` ×2, `support_grid` ×2, `regiment_support_grid`, `regiments_grid_lead`, the 4 `info` `stats_grid`, `names_grid`, `divisions_grid` ×2, `designer_regiments_grid`, `symbols_grid`, `buttons_grid`, `equipment_categories_grid`, `niches_grid`, `equipment_variants > grid`, `equipment_info`-style), `slotsize` ×20, `borderSize` ×16, `margin` ×6, `padding`, `positionType` (`row_padding`), the `divisions`/`division_names`/`equipment_categories` dropdown + `expandedWindow` ×5 + `expandButton` ×5 shells, `editBoxType` ×2 (name boxes — UI-state), `designerregimentswindow` (battalion-picker popup shell), `div_templ_custom_icon_view`/`division_equipment_view` container shells, the 2 `designer_adjuster_entry*` (mapped once as adjuster rows).

### Division-designer element accounting — `stats.element_type_counts`

`containerWindowType` 55, `iconType` 54, `dropDownBoxType` 5, `expandButton` 5, `expandedWindow` 5,
`background` 38, `gridBoxType` 10, `slotsize` 20, `buttonType` 32, `instantTextboxType` 43,
`Background` 2, `instantTextBoxType` 3, `borderSize` 16, `gridboxtype` 10, `padding` 1, `ButtonType` 5,
`margin` 6, `editBoxType` 2, `positionType` 1 = **313 total**.

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** (incl. derived/unavailable) | **~55** | experience, template name + dropdown (`designer_division_entry`), div symbol, priority icon + 3 priority buttons, niche icon, can-paradrop/special-forces flags, the 3 regiment/support grids + `designer_division_regiment_slot_entry`/`designer_division_support_slot_entry` (mapped once each), the 3 HQ grids, combat-width text (×2), the 4 `info` stat-grids + `designer_stat_entry`/`designer_adjuster_entry` (mapped once), the 4 stat chips (hardness/org/strength/speed), `division_equipment_entry` + archetype/type entries + equipment_counter, allow-checkboxes (unavailable), `designer_hq_modifier_entry` (unavailable), production cost/resources + deploy cost/time (unavailable), name-group entry |
| **Static / decoration / nav / mutation-inert / structural** | **~258** | popup/HQ/panel/3D backgrounds, the 3D-model + custom-icon + category picker subtrees, the battalion-picker catalog (`designer_subunit*_entry`), all section/stat labels, ~30 mutation/nav buttons, 20 grid factories + slotsizes, dropdown/expand shells, borders/margins, name boxes |

---

# VIEW 4 — EQUIPMENT (NAVAL) DESIGNER (`game_assets/gui/equipment_designer.json`, from `interface/equipmentdesignerview.gui`)

Root `countryequipmentdesignerview` (1097×542) + 9 sibling row-template windows
(`designer_equipment_entry`, `designer_creator_entry`, `equipment_designer_name_group_entry`,
`equipment_module_category_entry`, `equipment_module_entry`, `equipment_designer_3d_model_entry`,
`equipment_designer_sprite_entry`, `equipment_designer_niche_icon_entry`,
`equipment_designer_module_slot_entry`). This is the **naval/ship equipment designer** — same
component-designer frame as VIEW 1 (`equipment_modules` board, `info` 3-column panel +
`adjuster_stats`, `module_selector_window`, `designer_intel_container`, `referenced_info`). Binds the
**focused SHIP variant** `V` (family `ship`). Records DELTAS vs VIEW 1 + its distinct row templates.

## 4a. Equipment designer — deltas (dynamic)

| gui element (name + type, sprite/text) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `experience` | `countries.<TAG>.military.xp.navy` | int | **exact** | **Delta:** NAVY XP. |
| `Background` (`GFX_ship_designer_popup_bg`) | static | — | **static** | Ship designer skin. |
| `equipment_preview > background` (`GFX_ship_view_bg`) | static | — | **static** | Ship blueprint backdrop. |
| `pos_custom_module_slot_window_0..6` + `pos_fixed_module_slot_window_0..6` (positionType ×14) | `designs.equipment_variants[V].modules{slot:mod}` (live: `equipment_variants_live[V].modules`) | module-slot | **exact** | **Delta:** ships have a 2-row board (custom + fixed). EXACT slot→module. |
| `ship_category_icon` (iconType, `GFX_ship_category_icon_small`) | `designs.equipment_variants[V].archetype` → ship category | category | **derived** | Ship hull category icon. |
| `niche_button` (buttonType, `GFX_naval_equipment_role_icons`) | variant niche/role | category | **derived** | Naval role icon. |
| `design_team_button` (buttonType, `GFX_idea_slot_naval_manufacturer`) | `designs.equipment_variants[V].design_team_mio` → `mios[]` | loc/category | **derived** | **Delta:** naval-manufacturer MIO slot. |
| `equipment_names > name` (instantTextboxType, dropdown) | the ship NAME-LIST chosen for the variant | loc | **derived** | **Delta:** a second dropdown (ship name lists) beside the design dropdown. |
| `equipment_names > names_grid` (gridBoxType) → name rows | static ship name-list catalog | row factory | **static (catalog)** | Static DEF. |
| `equipments > name` (editBoxType) + `equipment_template_icon` (`GFX_division_template_icon`) | `designs.equipment_variants[V].name` | loc | **exact** | The design name. |
| `info > ship_designer_naval_dominance_value` (`SHIP_DESIGNER_NAVAL_DOMINANCE_VALUE`, icon `GFX_naval_dominance_texticon`) | the variant's naval-dominance contribution | num | **unavailable (hard limit)** | **Delta (naval-only):** naval dominance is a runtime-computed value. |
| `ship_designer_naval_dominance_name` (`SHIP_DESIGNER_NAVAL_DOMINANCE_NAME`) | static loc | loc | **static** | Label. |
| `info > adjuster_stats > stats_grid` (header `PRODUCTION_ADJUSTERS_LABEL`) → adjuster rows | `designs.equipment_variant_stats[V]` adjuster subset (**D**) | row factory | **derived (model)** / **unavailable (hard limit)** | **Delta:** production-adjuster grid (as plane VIEW 2). |

`info` base/combat/misc panels, `equipments` dropdown + variant rows, `module_selector_window`,
`designer_intel_container` (our/foreign/obsolete + `acquired_variants_grid` +
`variant_country_filter_gridbox`), `referenced_info` compare card, `position_version`,
`design_company_icon`, `upgrade_assigned_industrial_org`, `production_resources`/`production_value`/
`refit_cost`, `niche_icon_selector` catalog — **bind exactly as VIEW 1**, incl. the COMBAT-STAT hard
limit on every `stats_grid`.

### Distinct row templates (mapped once each)

| entry template → fields | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `designer_equipment_entry > name`/`equipment`(icon)/`creator_flag` | `designs.equipment_variants[v].name` / `.archetype`→icon / `.creator`→flag | loc/category/flag | **exact** | Acquired-variant row (the `equipments` dropdown / acquired list). |
| `designer_equipment_entry > missions > mission_grid` (gridboxtype) → mission chips | the variant's allowed missions | category | **static (def)** | Allowed-mission set = static archetype DEF. |
| `designer_creator_entry > creator_flag`/`flag_frame` | `designs.equipment_variants[v].creator` → flag | flag | **exact** | Country-filter flag chip (the `variant_country_filter_gridbox`). |
| `equipment_designer_name_group_entry > name` | the variant name-group | loc | **derived** | Name-list group header. |
| `equipment_module_category_entry > name`/`equipment_icon`/`validity_icon` (`GFX_module_requirements`) | the choosable module category → loc+icon | loc/category | **static (catalog)** name+icon / **unavailable** validity | Module CATEGORY in the picker = static DEF; the validity/requirements icon = a tech-eligibility compute. |
| `equipment_module_entry > name`/`equipment_icon`/`cost`/`allow_role_text`/`forbid_role_text`/`validity_icon`/`stat_icons.stat_icon_*` | the choosable module → loc+icon | loc/category | **static (catalog)** name+icon / **unavailable** cost+stat-icons+validity | Module pick row: name/icon = static DEF; the `cost` + the 5 `stat_icon_*` (torpedo/light-gun/heavy-gun/AA/anti-sub) per-module stat previews + validity = COMPUTED (hard limit). |
| `equipment_designer_3d_model_entry > name`/`sprite`/`priority` | static 3D-model catalog | loc | **static (catalog)** | 3D-model picker. |
| `equipment_designer_sprite_entry > name`/`icon`/`priority` | static sprite catalog | loc/category | **static (catalog)** | Sprite picker. |
| `equipment_designer_niche_icon_entry > icon`/`selection` | static niche catalog | category | **static (catalog)** | Niche selector chips. |
| `equipment_designer_module_slot_entry > btn_module_slot`/`icn_module_slot_frame`/`icn_requirements`/`icn_warning` | the focused variant's module at this board slot (`modules{slot:mod}`) | module-slot | **exact** slot / **unavailable** requirements/warning | The actual blueprint-board slot widget instantiated per slot; module icon EXACT, the requirements/invalid-order warning icons = compute. |

## 4b. Equipment designer — STATIC / decoration / mutation-inert

Same classes as VIEW 1 + the naval-specific picker subtrees: popup/blueprint/3D backgrounds,
`equipment_select_model`/`equipment_select_sprite`/`historical_select_design` subtrees (incl.
`best_match_window`, `folder_container`/`list`, `import/export_preset_*`), `icon_selection_container`,
`toggle_model_selector_button`, all stat-column labels + naval-dominance name, the bottom action strip
(`save_to_file_button`/`historical_design_button`/`reset_button`/`duplicate_button`/`auto_button`/
`auto_upgrade_button`/`auto_upgraded_checkbox_button`/`dynamic_version_checkbox_button`/`save_button`/
`nubbin`), `niche_button`/`toggle_comparison_button`/`close_button`/`close_referenced_info`/
`back_button`/`btn_back`/`show_all_button`, all grid factories + slotsizes + 15 positionType anchors +
margins + borders + dropdown/expand shells + `name` search boxes (×2).

### Equipment-designer element accounting — `stats.element_type_counts`

`containerWindowType` 46, `background` 41, `iconType` 34, `instantTextboxType` 33, `buttonType` 18,
`borderSize` 6, `positionType` 15, `ButtonType` 10, `gridboxtype` 8, `slotsize` 17, `gridBoxType` 9,
`margin` 3, `dropDownBoxType` 2, `expandButton` 1, `expandedWindow` 2, `editBoxType` 1, `max_slots` 1
= **300 total**.

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** (incl. derived/unavailable) | **~40** | experience (navy), equipments>name + dropdown, equipment_names dropdown, position_version, ship_category_icon, niche icon, the 14 module-slot anchors + `equipment_designer_module_slot_entry`, the 3 `info` stat-grids + `adjuster_stats` (+`designer_stat_entry` mapped once), the 3 `referenced_info` stat-grids, naval-dominance value (unavailable), referenced_variant_name, acquired_variants_grid + `designer_equipment_entry`, variant_country_filter_gridbox + `designer_creator_entry`, design_company_icon, design_team_button, upgrade_assigned_industrial_org, production cost/resources + refit (unavailable), `equipment_module_category_entry`/`equipment_module_entry` (mapped once; name/icon static, cost/stats unavailable), name-group entry |
| **Static / decoration / nav / mutation-inert / structural** | **~260** | popup/blueprint/3D backgrounds, 3 picker subtrees + 2D-icon picker, 3D/sprite/niche picker entries, stat labels + dominance name, ~22 mutation/nav buttons, grid factories + slotsizes, 15 positionType anchors, margins/borders/shells, name search boxes |

---

# VIEW 5 — EQUIPMENT UPGRADE DESIGNER (`game_assets/gui/equipment_upgrade_designer.json`, from `interface/equipmentupgradedesignerwindow.gui`)

Root `equipment_upgrade_designer_window` (545×416) + 3 sibling family-specific subtree windows
(`land_equipment_designer_view`, `air_equipment_designer_view`, `naval_equipment_designer_view`) + 1
stat-row template (`equipment_designer_stat_entry`). This is the **archetype upgrade-slider window**
(the simple "create variant" panel for equipment with named upgrade tracks — NOT the module-based
designers). It binds ONE variant `V` being upgraded; the visible family subtree
(land/air/naval) is chosen by `designs.equipment_variants[V].family`. Each upgrade-track container is
a fixed `{Background, label, decrease_level, level, increase_level}` widget bound to **one named
upgrade** in `upgrades{name:lvl}`.

## 5a. Equipment-upgrade designer — dynamic (data-bound)

### Window header

| gui element (name + type, sprite/text) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `equipment_variant_label` (instantTextboxType, `DESIGNER_CREATE_VARIANT`) | static loc | loc | **static** | Window title. |
| `experience` (instantTextboxType, `DESIGNER_ARMY_EXPERIENCE`, tip `CURRENT_EXPERIENCE`) | `countries.<TAG>.military.xp.{army/navy/air}` (by family) | int | **exact** | XP available; channel chosen by `V.family`. |
| `variant_name` (editBoxType, `variant`) | `designs.equipment_variants[V].name` | loc | **exact** | The variant's custom name. |
| `equipment_info > equipment_info_grid` (gridBoxType) → `equipment_designer_stat_entry` rows | `designs.equipment_variant_stats[V]` (**D**) | row factory; `label`→loc, `count`→num | **derived (model)** / **unavailable (hard limit)** | The variant's resulting stat list (combat numbers) — runtime-computed; the WAVE-2 model or unavailable. |
| `change_design_team > change_design_team_label` (`INDUSTRIAL_ORG_LIST_WINDOW_TITLE`) | static loc | loc | **static** | "Design team" section label. |
| `change_design_team > open_roster` (buttonType, `GFX_idea_slot_materiel_manufacturer`) | `designs.equipment_variants[V].design_team_mio` → `mios[]` | category | **derived** | The assigned design-company icon (materiel manufacturer). Click = mutation. |

### Upgrade-track containers — THE EXACT upgrade-level layout

Each family subtree holds a set of named upgrade-track containers. Every container has the SAME shape:
`{Background(GFX_variant_entry_bg), label, line0x(connector), decrease_level, level, increase_level}`.
The **bound datum is `upgrades{name:lvl}`** (save **S**) / `equipment_variants_live[V].upgrade_levels[]`
(live **M**, positional). This is the textbook EXACT upgrade-slider mapping — LEVELS are stored, the
RESULTING combat stat is not.

| upgrade-track container (and its `label`/`level`) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| **land subtree** (`land_equipment_designer_view`): `tank_armor_upgrade`, `tank_reliability_upgrade`, `tank_gun_upgrade`, `mech_cost_upgrade`, `tank_art_upgrade`, `tank_at_upgrade`, `tank_aa_upgrade`, `tank_engine_upgrade`, `ac_engine_upgrade` — each `> label` / `> level` | `label` ← the upgrade's name → loc; `level` ← `designs.equipment_variants[V].upgrades["<this-upgrade>"]` (live: `equipment_variants_live[V].upgrade_levels[i]`) | loc / int | **exact** | Each track's NAME (label) + LEVEL are EXACT. Only the tracks valid for `V.archetype` render (the engine hides the rest). |
| **air subtree** (`air_equipment_designer_view`): `plane_reliability_upgrade`, `plane_range_upgrade`, `cv_plane_range_upgrade`, `plane_gun_upgrade`, `plane_armor_upgrade`, `cv_plane_gun_upgrade`, `plane_naval_upgrade`, `cv_plane_naval_upgrade`, `plane_bomb_upgrade`, `plane_tac_bomb_upgrade`, `plane_cas_upgrade`, `plane_engine_upgrade` — each `> label` / `> level` | as above, `upgrades{...}` / `upgrade_levels[]` | loc / int | **exact** | Air upgrade tracks; name+level EXACT. |
| **naval subtree** (`naval_equipment_designer_view`): `ship_reliability_upgrade`, `ship_ASW_upgrade`, `sub_stealth_upgrade`, `ship_armor_upgrade`, `carrier_armor_upgrade`, `ship_anti_air_upgrade`, `torpedo_ship_reliability_upgrade`, `ship_gun_upgrade`, `ship_deckspace_upgrade`, `ship_torpedo_upgrade`, `sub_torpedo_upgrade`, `ship_engine_upgrade`, `destroyer_engine_upgrade`, `sub_engine_upgrade` — each `> label` / `> level` | as above, `upgrades{...}` / `upgrade_levels[]` | loc / int | **exact** | Naval upgrade tracks; name+level EXACT. |
| every track's `decrease_level` / `increase_level` (buttonType, `GFX_naval_decrease/increase_amount`) | — | — | **deferred: mutation** | Inc/dec the level — inert in replay. |
| every track's `archetype` (iconType, `GFX_equipment_item_large`) | `designs.equipment_variants[V].archetype` → equipment icon | category | **exact** | The big archetype glyph at the centre of each family subtree. |

### `equipment_designer_stat_entry` (stat ROW template — for `equipment_info_grid`, mapped once)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `label` (instantTextboxType, `Name`) | the stat's name → loc (per `equipment_variant_stats[V]` key) | loc | **derived (model)** / **unavailable** | Stat label; only if the derived model is present. |
| `count` (instantTextboxType, `count`) | the stat's value (per `equipment_variant_stats[V]` value) | num | **derived (model)** / **unavailable (hard limit)** | The COMPUTED stat value — never fabricated. |

## 5b. Equipment-upgrade designer — STATIC / decoration / mutation-inert

- **Backgrounds / chrome:** `Background` (`GFX_equipment_upgrade_designer_bg`), every track `Background` (`GFX_variant_entry_bg`), `experience_bg`, `change_design_team > Background`, `equipment_select_model > Background`/`border_container`/`bg_tech_button`/`3d_view` (the 3D picker — no replay equivalent), every track's `line01`/`line02`/`line03`/`line04` connector glyphs (`GFX_unit_details_line_*`/`GFX_unit_navy_details_line_*` — purely decorative tree-connector lines).
- **Static label texts (loc):** `equipment_variant_label` (`DESIGNER_CREATE_VARIANT`), `change_design_team_label` (`INDUSTRIAL_ORG_LIST_WINDOW_TITLE`), `select_model_button` (`SELECT_3D_MODEL`), `save_button` (`DESIGNER_ARMY_SAVE`).
- **3D-model picker subtree (mutation-inert):** the whole `equipment_select_model` (incl. `grid_container > buttons_grid`), `select_model_button`.
- **Mutation / action verbs — INERT:** `save_button`, `close_button`, `select_model_button`, `change_design_team > upgrade_assigned_industrial_org`, `open_roster`, every track `decrease_level`/`increase_level` (~26 buttons).
- **Structural (row factories + layout):** `active_equipment_view` (empty placeholder container), `equipment_info` + `equipment_info_grid` (the stat row factory), `equipment_select_model > grid_container > buttons_grid`, `slotsize` ×2, `max_slots`, `borderSize` ×3, the 3 family-subtree container shells (`land/air/naval_equipment_designer_view` — only one renders at a time per `V.family`).

### Equipment-upgrade-designer element accounting — `stats.element_type_counts`

`containerWindowType` 46, `background` 5, `instantTextboxType` 75, `iconType` 75, `gridBoxType` 2,
`slotsize` 2, `max_slots` 1, `buttonType` 74, `editBoxType` 1, `borderSize` 3, `ButtonType` 1
= **285 total**.

> Note: the 35 upgrade-track containers (9 land + 12 air + 14 naval) each carry a `label` + `level`
> instantTextbox (≈70 of the 75) + an `archetype` icon (3, one per family) + `Background`/connector
> icons (the bulk of the 75 iconType) + 2 buttons (`decrease_level`/`increase_level`, ≈70 of the 74
> buttonType). The mapping is **once per track** (the table above); at runtime only the tracks valid
> for `V.archetype` render.

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** (incl. derived/unavailable) | **~75** | experience, variant_name, the 35 upgrade-track `label`+`level` pairs (≈70 bound textboxes — name+level EXACT), the 3 `archetype` icons, `equipment_info_grid` + `equipment_designer_stat_entry` (mapped once, derived/unavailable), open_roster (design-team icon, derived) |
| **Static / decoration / nav / mutation-inert / structural** | **~210** | window/track/3D backgrounds + connector lines (the bulk of the 75 iconType), the 3D-model picker, ~28 mutation buttons (decrease/increase × tracks + save/close), section labels, the 3 family-subtree shells + grid factory + slotsizes |

---

## Cross-view summary — total bindable elements

| view | total elements | dynamic-bound (incl. derived/unavailable) | static/decoration/nav/mutation-inert/structural |
|---|---:|---:|---:|
| 1. Tank designer | 209 | ~34 | ~175 |
| 2. Plane designer | 186 | ~33 | ~153 |
| 3. Division designer | 313 | ~55 | ~258 |
| 4. Equipment (naval) designer | 300 | ~40 | ~260 |
| 5. Equipment-upgrade designer | 285 | ~75 | ~210 |
| **TOTAL** | **1293** | **~237** | **~1056** |

> The high static/inert share is expected: designers are EDITORS. Most of every tree is mutation
> chrome (save/deploy/reset/duplicate/auto/inc-dec/add-module), 3D-model + 2D-icon + preset + module
> PICKER subtrees (no replay equivalent — static catalogs at best), and decorative blueprint/connector
> art. The replay binds the design's IDENTITY + LAYOUT (module slots, regiment grid, upgrade levels,
> name/icon/MIO) exactly; every COMPUTED combat stat is the documented §G hard limit.

---

## Schema-gap ledger (with ARTIFACT-vs-TRUE-GAP + CLOSURE-BUCKET classification)

For every `unavailable`/gap below: **ARTIFACT** = the field is live/save-extractable, just absent from
the current SAVE-ONLY demo snapshot → "FOLD-IN/RECONCILE"; **TRUE GAP** = a genuine schema/data limit.
For TRUE GAPs, the **CLOSURE BUCKET** is one of: **(a) LIVE-EXTRACTABLE** (in memory, reader exists/
addable); **(b) COMPUTED-NOT-STORED** (needs cracking / live recompute — Ghidra tier); **(c) STATIC
DEF / WORLD DATA** (parseable from game `common/` files, mod-aware).

| # | gap (gui element → missing field) | views | artifact vs TRUE GAP | closure bucket | note |
|---|---|---|---|---|---|
| 1 | designer **combat/base/misc/equipment stat panels** (`base_stats`/`combat_stats`/`misc_stats`/`equipment_stats` `stats_grid` → soft/hard atk, defense, breakthrough, armor, piercing, org, HP, reliability, agility, etc.) | 1,2,3,4,5 | **TRUE GAP** | **(b) COMPUTED-NOT-STORED** | The textbook designer hard limit (§G). Not in save, not in any standing memory struct. PARTIAL closure via the WAVE-2 derived model (`designs.equipment_variant_stats` / `template_stats`, **D**) which recomputes a subset offline — when present, badge **derived (model)**; the FULL set (all engine modifiers) stays (b). |
| 2 | division **stat chips** `hardness_stat`/`org_stat`/`strength_stat`/`speed_stat` → hardness/org/max-strength/speed | 3 | **TRUE GAP** | **(b) COMPUTED-NOT-STORED** | Same as #1 (division flavour). `template_stats[T]` (D) covers these if the model runs; else unavailable. |
| 3 | division `combat_width_view` / `combat_width_view_hq` → combat width | 3 | **TRUE GAP** | **(b) COMPUTED-NOT-STORED** | Runtime sum of regiment widths × modifiers. `template_stats[T].combat_width` (D) or unavailable. |
| 4 | every `production_value` (IC cost) / `production_resources` (resource cost) / `refit_cost` | 1,2,3,4 | **TRUE GAP** | **(b) COMPUTED-NOT-STORED** | Build/refit cost is a runtime serialize compute. Not stored. (A future model could derive IC from module/equipment base costs → would become (c)+(b) hybrid.) |
| 5 | per-module **stat preview icons** `stat_icon_torpedo`/`_light_gun`/`_heavy_gun`/`_anti_air`/`_anti_sub` (`equipment_module_entry`) + module `cost` | 4 | **TRUE GAP** | **(b) COMPUTED** stat values, **(c) STATIC DEF** for the module's base contribution | The per-module stat *contribution* is in the module DEF (`common/units/equipment/modules/`, mod-aware → (c)); the *realised* value shown (after tech) is a compute (b). |
| 6 | module **validity/requirements** icons (`validity_icon`/`icn_requirements`/`icn_warning`/`equipment_warning`) | 1,3,4 | **TRUE GAP** | **(b) COMPUTED** (eligibility) + **(c) STATIC DEF** (the requirement rule) | "Is this module allowed" = evaluate the module's requirement trigger against the country's tech (compute); the rule itself is static DEF. |
| 7 | naval **dominance value** `ship_designer_naval_dominance_value` | 4 | **TRUE GAP** | **(b) COMPUTED-NOT-STORED** | Naval-dominance contribution is runtime-computed. |
| 8 | plane/naval **production-adjuster** grid `adjuster_stats > stats_grid` | 2,4 | **TRUE GAP** | **(b) COMPUTED-NOT-STORED** | Cost/output trade-off sliders' resulting numbers are runtime; the chosen adjuster LEVELS would be (a) if they live in the variant record (currently not surfaced separately from `upgrades`). |
| 9 | division terrain **adjuster** rows `adjuster_stats` (`designer_adjuster_entry` movement/attack/defense) | 3 | **TRUE GAP** | **(b) COMPUTED-NOT-STORED** | Per-terrain modifiers for the template = runtime compute. `template_stats[T]` terrain subset (D) or unavailable. |
| 10 | division **allowed-equipment policy** bits `is_allowed_button`/`is_type_allowed_button`/`new_equipment_allowed_text`/`foreign_equipment_allowed_text` + `equipment_counter` "X/Y" | 3 | **TRUE GAP** | **(a) LIVE-EXTRACTABLE** (the policy flags) + **(b)** (the X/Y tally) | The per-template "reinforce-with" / allow-foreign toggles are a stored designer-policy set NOT currently in `division_templates[]`; a reader/parser could add them → (a). The "X/Y allowed" count is a derived tally (b). |
| 11 | HQ **deploy cost/time** `hq_deploy_cost`/`hq_deploy_time` + division `cost` | 3 | **TRUE GAP** | **(b) COMPUTED-NOT-STORED** | HQ command-power deploy cost + time = runtime. |
| 12 | HQ **modifiers list** `designer_hq_modifier_entry` | 3 | **TRUE GAP** | **(b) COMPUTED-NOT-STORED** | Army-wide bonuses the HQ grants are computed from its composition. |
| 13 | upgrade-track **NAME→loc join** (`equipment_variants_live.upgrade_levels[]` is POSITIONAL, names resisted) | 1,5 | **TRUE GAP** (live channel only) | **(c) STATIC DEF** | The LIVE leaf stores upgrade LEVELS positionally; the upgrade NAMES come from the archetype DEF (`common/units/equipment/upgrades/`, mod-aware → (c)). The SAVE channel `upgrades{name:lvl}` already carries names (no gap there). |
| 14 | module/role/niche **selector catalogs** (`gb_modules`, `tank_role_grid`, `niche_icon_selector`, battalion picker `designer_subunit*_entry`, 3D-model + 2D-icon + symbol pickers) | 1,2,3,4 | **TRUE GAP** | **(c) STATIC DEF / WORLD DATA** | The CHOOSABLE options (modules/roles/niches/battalions/symbols/3D-models) are static game DEF, not snapshot state. Parseable mod-aware. The replay shows the CHOSEN value (exact), not the full picker. |
| 15 | `forbidden`/`niche_icon_disabled`/`forbid_role_text`/`allow_role_text` (role-eligibility flags) | 1,3,4 | **TRUE GAP** | **(b) COMPUTED** + **(c) STATIC DEF** | "Role forbidden/allowed for this design" = evaluate the role rule (static DEF) against the design (compute). |
| 16 | per-variant **niche/role token** on `equipment_variants` (driving `dropdown_tank_roles`/`plane_role`/`niche_button`) | 1,2,4 | **FOLD-IN/RECONCILE** (likely) | **(a) LIVE-EXTRACTABLE** / parse from save | The variant's chosen role/niche is a creator-set field; SCHEMA §11 lists `modules`/`upgrades`/`name`/etc. but not an explicit `role` on a variant — if the parser already captures it elsewhere this is an artifact; if not, it is an (a) add. Flagged for reconcile against `parse_designs` output. |
| 17 | 3D-model / 2D-icon **selection** (`select_model`/`3d_view`/`btn_equipment_icon` + the model entries) | 1,2,3,4,5 | **TRUE GAP** (by design) | **n/a (out of scope)** | The replay has NO 3D model/icon viewer; these are PICKER chrome with no data binding. Listed inert, not a closable gap. |

**Closure-bucket roll-up:** the overwhelming majority of designer gaps are **(b) COMPUTED-NOT-STORED**
(every combat/cost/width/dominance/modifier value — the §G hard limit; PARTIALLY closable by the
WAVE-2 derived `*_stats` models, never by the save). A second cluster is **(c) STATIC DEF** (the
module/role/niche/battalion CATALOGS + the per-module base-stat contributions + upgrade NAMES on the
live channel — all parseable from mod-aware `common/`). Only #10 (allowed-equipment policy bits) and
#16 (variant role token) are plausibly **(a) LIVE-EXTRACTABLE / FOLD-IN** adds. There are **no
demo-snapshot artifacts in the stat-panel core** — those are genuine hard limits, not missing-from-demo.

---

## Historical review round

Historical note: this records a completed adversarial review from the old
Claude-era workflow. Codex-native sessions must use normal sub-agents/partner
review and must not run the old local CLI command.

**Asked:** (a) any missed
elements per view; (b) field-path correctness vs `schema.py` (esp. `designs.equipment_variants`
`modules{slot:mod}`/`upgrades{name:lvl}`, `equipment_variants_live.upgrade_levels[]`,
`division_templates[].regiments[{type,x,y}]`/`support[]`/`regimental_support[]`,
`template_stats`/`equipment_variant_stats` source **D**, `mios[]`/`scheduled_variants[]`,
`military.xp.{army,navy,air}`); (c) is any "unavailable" actually a field I overlooked vs a true §G
hard limit; (d) is the artifact-vs-true-gap + closure-bucket classification correct (esp. that all
designer combat stats are (b) COMPUTED-NOT-STORED, module/archetype base stats are (c) STATIC DEF).

**Result:** The review **confirmed the classification on all four points** with no material
refutation. (A first run that used the longer prompt began exploring the real game `.gfx`/`.gui`
module-icon files and timed out under the older review runner; the re-run with a tighter no-file-read
prompt returned a clean verdict.) Codex's exact conclusions:
- **(c)** "designer combat stats are true computed-not-stored per §11. Do not mark as extractable from
  save/memory unless a runtime memory snapshot explicitly captured them. `*_stats (D)` can make them
  derived-model for covered subset only; otherwise unavailable — artifact \[of no model], not a hidden
  save path." → matches the map.
- **(d)** "Closure taxonomy is right: designer combat stats/costs/naval dominance/HQ computed outputs
  = (b) COMPUTED-NOT-STORED. Module/archetype base contributions, catalogs, legal choices, names/
  icons/models = (c) STATIC-DEF. Live upgrade names need archetype/static ordering, so values exact,
  labels (c)." → matches gaps #1-#15.
- **(b)** field paths "look correct" — modules/division-grid/XP exact; MIO is a **derived JOIN**
  (`design_team_mio` → `mios[]`), "not exact single-field UI" → the map already badges every MIO
  widget **derived**, consistent.
- **(a)** the classes Codex flagged as possibly-missed (comparison/`referenced_info` cards, family/
  archetype subtree labels, action-strip affordances, preset/3D/icon catalogs) are **all already
  mapped** above (referenced_info stat-grids = derived-model/unavailable; subtree catalogs = (c)
  static; action strip = mutation-inert; pickers = (c) static).

**Folded correction (the one refinement):** Codex noted the action-strip "can-afford"/enablement is
itself a COMPUTE even though the XP INPUT (`military.xp`) is exact. The map already treats every
action-strip button as **mutation-inert** (no datum), so the enablement compute never surfaces — no
change needed, but recorded here for completeness.

**Agreed (folded before review, high-confidence from schema; Codex concurred):**
- All designer combat/base/misc/equipment/adjuster stat panels + the division stat chips + combat
  width + production/refit cost + naval dominance + HQ deploy cost/time + HQ modifiers are **(b)
  COMPUTED-NOT-STORED** (§G), PARTIALLY closable only by the derived `template_stats`/
  `equipment_variant_stats` models (badged **derived (model)** when present). Confirmed against the
  `Designs` dataclass note ("save does NOT store derived combat stats").
- Module SLOT layout = `equipment_variants[V].modules{slot:mod}` (S) / `equipment_variants_live[V].
  modules{slot_name:module_name}` (M) — EXACT. Upgrade-slider LEVELS = `upgrades{name:lvl}` (S) /
  `upgrade_levels[]` (M, positional, names resisted → #13 (c)) — EXACT.
- Division regiment GRID = `division_templates[T].regiments[{type,x,y}]` + `support[]` +
  `regimental_support[]` — EXACT (M/S), the spine of VIEW 3.
- The selector CATALOGS (modules/roles/niches/battalions/3D-models) are static DEF (c), not snapshot
  state; the replay shows the chosen value, not the picker.

**Flagged for reconcile (not blocking):**
- #16 (per-variant role/niche token) — verify `parse_designs` actually emits a role on each variant;
  if it does, the `dropdown_tank_roles`/`plane_role`/`niche_button` bindings upgrade from **derived**
  to **exact** and the gap is an artifact, not a TRUE GAP.
- #10 (allowed-equipment policy bits) — confirm whether the save's template carries the "reinforce
  with" / allow-foreign flags; if a parser add surfaces them, they move from unavailable to **(a)
  exact**.

**Refuted / corrected:** none material — the schema is unambiguous that designer combat stats are
runtime-computed (hard limit), so no "unavailable" in the stat panels is a mislabelled overlook.

> If the Codex CLI run returned a non-empty reply, its specific corrections are merged into the tables
> above and itemised here; if it timed out twice, this section records **"Codex attempted, timed
> out"** and the map rests on the direct schema verification.
