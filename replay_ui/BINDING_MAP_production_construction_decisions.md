# BINDING MAP — PRODUCTION + CONSTRUCTION + DECISIONS (Phase-3 binding sweep)

> **What this is.** The COMPLETE, no-truncation data-binding map for three in-game the game views the
> native replay UI binds in the Phase-3 sweep: **production**
> (`game_assets/gui/production.json`, from `interface/countryproductionlineview.gui`),
> **construction** (`game_assets/gui/construction.json`, from
> `interface/countryconstructionsview.gui`), and **decisions**
> (`game_assets/gui/decisions.json`, from `interface/countrydecisionview.gui`). It realises
> GAME_UI_REPLICA_PLAN §C — the "comprehensive but mechanical binding sweep" — for these three
> array→grid surfaces. EVERY element in each `.gui` tree is accounted for, in exactly one bucket:
> dynamic (data-bound), static/decoration (no binding), or deferred (needs imperative adapter).
> Nothing is omitted; where a `.gui` element exists but our snapshot has no field for it, the
> element is listed and **badged `unavailable`** with the missing field named.
>
> **Binding-rule shape** (PLAN §C.1): `{ target (gui element name+type), source (snapshot path),
> format (formatter id), fidelity (exact|derived|proxy|unavailable) }`. **Source paths** use the
> SCHEMA dotted form `countries.<TAG>.…` (see `replay/schema.py` FIELD_SOURCES, the authoritative
> machine-readable mirror, incl. the newly-wired LIVE leaves). `<TAG>` = the player/observed
> country. Per-row sources inside a gridbox row-factory are written `…[].<field>`.
>
> **No-fabrication (SCHEMA §0.2, PLAN principle 3):** `null` renders "—", never `0`. `0`/`0.0` is a
> measured zero. A field schema-present but never fed by the current parser is badged `unavailable`
> (not silently dropped).
>
> **Grid row-factory model (PLAN §B.4 / §C.2).** `gridBoxType`/`gridboxtype` are **row factories**,
> not passive containers: the engine instantiates a named ENTRY-TEMPLATE window once per bound array
> element. In these three `.gui` files the grids and their entry-template windows are defined as
> SEPARATE top-level windows (e.g. `item_grid` is the grid; `production_military_line_entry` is the
> row template it instantiates). Each entry template is mapped once below; at runtime it is repeated
> per array element. The grid container itself carries no datum (it is the structural row factory);
> its bound CHILDREN (in the entry template) carry the data.

---

## Provenance of the fidelity calls

Grounded in **`replay/schema.py` FIELD_SOURCES + the dataclass docstrings** (the STABLE, wiring-
complete schema — authoritative for the newly-wired live domains `production_live`,
`construction_orders`, `decisions_market`), cross-checked against `replay/SCHEMA.md` §3.1/§3.6/§3.18
and the on-disk demo snapshot `replay/snapshots/replay/000017005619.snapshot.json` (GER 1941,
schema_version "1"). **Adversarially reviewed by Codex** (read-only, 2026-06-26); its verdict and
corrections are folded in (see the "Codex review" note at the end).

**Critical schema facts that drive the badges (verified in schema.py):**

| schema leaf (live id) | shape / element | source tag | consequence in these views |
|---|---|---|---|
| `countries.*.production.military_lines` / `.naval_lines` (`production_live`, M) | each `{id, active_factories, cost, speed, produced, amount}` | **M** | drives the PRODUCTION line-entry rows (exact line membership + active-factory sum; per-line scalars measured) |
| `countries.*.production.military_factories` / `.naval_factories` (M) | int = SUM(active_factories) over the line list | **M** | the operational-factory counters (numerator measured; **denominator = total owned** is a different number → `n/m` split badged derived) |
| `countries.*.production.stockpile` (M) `{variant_id: amount}` + `.stockpile_total` (M/D) | banked equipment, fixed-point | **M / M·D** | equipment-overview amounts; total is summed-from-memory (derived) |
| `countries.*.production.construction_queue` (`construction_orders`, M) | each `{line_id, template_id, active_factories, cost, produced, priority}` — **build target STATE/location is RESISTED (absent — no `location` child emitted, NOT a stored `null`)** | **M** | drives CONSTRUCTION building-line rows; **location/flag → `unavailable`** (province handle behind the template def; never fabricated). [Codex: the leaf is *absent/resisted*, not a stored null — wording tightened.] |
| `countries.*.economy.construction_queue` (save, S) | each `{template, location_state, active_factories, produced, cost, priority, created_date}` — **HAS `location_state` + `template` name** | **S** | the SAVE channel DOES carry build location + building name; usable when the snapshot is save-backed (see Construction note A) |
| `countries.*.decisions.timed_live` (`decisions_market`, M) | each `{decision, days, state, state_name}` | **M** | drives the timed-decision rows: name + days-remaining + target state (+ a resolved `state_name`) |
| `countries.*.decisions.{active,timed,missions}` (save, S) | save-shaped active/timed/missions lists | **S** | one-shot AVAILABILITY (clickable/locked, cost, scripted `visible`) is **not stored** (SCHEMA §3.6 note) → availability/progress badged unavailable |
| `countries.*.economy.factories` (S) | FIELD_SOURCES authoritative keys: `{civ, mil, dockyard, under_construction}` (S). The extra keys `civ_from_states`/`mil_from_states`/`dockyard_from_states`/`offmap` appear in SCHEMA.md (lines 176-178) **and in the demo snapshot** but are **NOT in `FIELD_SOURCES`** → schema-doc drift. | **S** | construction "available factories" + civ/mil/dock counts. **[Codex: the `*_from_states`/`offmap` sub-keys are unresolved drift — valid in SCHEMA.md + present in data, but not authoritative in FIELD_SOURCES. Bindings that use them (local/traded factory split) are badged `derived` accordingly below.]** |

**Genuinely runtime-computed (never stored anywhere) → stay `unavailable`/`derived` by nature:**
production efficiency cap / efficiency growth / efficiency loss, production & dockyard daily OUTPUT,
per-line production SPEED text & progress fraction, factory bombing defence, consumer-goods factory
count, construction daily SPEED & CIC bank rate, per-decision dynamic-modifier progress bars,
decision COST, scripted-`visible` availability. These are serialize-computed UI aggregates with no
persisted backing field (PLAN §G hard-limit class).

---

# VIEW 1 — PRODUCTION (`game_assets/gui/production.json`)

Root window `countryproductionlineview` + 30 sibling top-level windows (entry templates, sub-windows
for naval repair / deployment / equipment overview / ship-name list). The replay binds the player/
observed country `<TAG>`. Formatters: `int`, `int-triple`, `factory-split` (`n/m`), `manpower`,
`flag`/`flag-small`, `category` (id→loc/sprite), `equipment-icon` (frame of `GFX_technology_medium`),
`resource-icon` (frame of `GFX_resources_strip`), `progressbar-fraction`, `efficiency-pct` (×100).

> **Slice note.** The replay vertical slice is read-only and Political-mode-first; the naval-repair
> queue, naval deployment, ship-name editor, equipment/ship-design overview, and all
> add/subtract-factory / priority / manufacturer / convert / obsolete / variant mutation controls
> are **inert** (PLAN §D read-only verbs; mutation DISABLED). They are rendered chrome and listed in
> §1c. The data-bearing surface is the **production-line list** (`item_grid` →
> `production_military_line_entry` / `production_naval_line_entry` / their collapsed + railway-gun +
> ship-refit variants) plus the top-bar production summary in `production_win_top`.

## 1a. Production — dynamic (data-bound) elements

Interaction proof: `ProductionView` has measured production-line row selection
and wheel-scroll/thumb-drag coverage. Tests prove live `id` row identity,
save-backed `line_id` row identity, military/naval line identity preservation,
selected-row highlight, scrolled row clicks, A->B->A row rebuilds, unkeyed and
duplicate rows inert without index fallback, missing selected rows fail closed,
outside-grid wheel input is inert, thumb drags clamp and continue outside the
grid until pointer-up, and track clicks do not select underlying rows. Line
`close_production_line` buttons on visible rows are also proven
replay-disabled no-ops with unchanged selection/scroll/digest and neighboring
row-body clicks still selecting normally. Factory add/sub row buttons are now
proven replay-disabled no-ops using the preserved `@prod_*` GUI defines. Visible
priority/amount row buttons are also proven replay-disabled no-ops for military
and naval line templates. Visible line mutation controls are now also proven
replay-disabled no-ops: military `attach_manufacturer`/`change_manufacturer`,
`equipment_button`, `expand`, and `convert`; naval
`attach_manufacturer`/`change_manufacturer`, `equipment_button`, `expand`,
`carrier_air_wing_composition`, `deploy_location`, and `name_list_btn`. These
use real template positions, consume pointer-down/up with no commands or redraw,
preserve selected row/scroll/bind digest, and leave neighboring equipment/name-list
row-body clicks selectable. The six primary production category filters now
toggle replay-local row visibility from catalog-derived equipment categories.
The six secondary/status filter checkboxes now have geometry-anchored disabled
no-op proof so clicks cannot bleed into row selection; true status filtering
still needs backing per-line flags. Line detail drilldowns, scrollbar art,
hidden variant/obsolete equipment-info controls, unrendered refit-template
controls, disabled visuals, and side-by-side proof remain open.

### Top summary bar (`countryproductionlineview > military_tab > production_win_top`)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `operational_factories_value` (instantTextboxType, `"20/37"`) | `countries.<TAG>.production.military_factories` (used) / `economy.factories.mil` (total) | factory-split (`n/m`) | **derived** | Used = `military_factories` (= SUM active, **M**, exact). Total = `economy.factories.mil` (**S**, exact). The `n/m` pairing of a live numerator with a save denominator is the derived part. |
| `operational_naval_factories_value` (instantTextboxType, `"120/370"`) | `countries.<TAG>.production.naval_factories` (used) / `economy.factories.dockyard` (total) | factory-split | **derived** | As above for dockyards. |
| `operational_naval_factories_repair_value` (instantTextboxType, `"13/37"`) | dockyards-assigned-to-repair / total dockyards | factory-split | **unavailable** | The repair-assignment split is a naval-repair-queue computed value; **not stored** (no per-country repair-dockyard field). Badge unavailable. |
| `docyards_output_value` (instantTextboxType, `999`) | national daily dockyard OUTPUT (IC) | int | **unavailable** (country modifier display) / **derived** (if aggregate-from-line-speeds) | **[Codex nuance: per-line `speed` IS stored M; if this field means SUM of naval line `speed`, badge derived.]** As the in-game country OUTPUT readout it is a serialize-computed modifier display → unavailable; if the UI instead renders an aggregate of `naval_lines[].speed` it is derived. (`economy.gross_ic_by_equipment` is per-equipment GROSS IC, S/D — a different aggregate.) |
| `production_output_value` (instantTextboxType, `999`) | national daily production OUTPUT (IC) | int | **unavailable** (country display) / **derived** (sum of `military_lines[].speed`) | **[Codex nuance: `military_lines[].speed` is a stored M leaf.]** Same split as dockyard output: the country OUTPUT readout is computed (unavailable); a sum-of-line-speeds aggregate is derived. |
| `efficiency_cap_value` (instantTextboxType, `999`) | production efficiency cap % | efficiency-pct | **unavailable** | Efficiency cap is a tech/MIO-computed runtime value; not stored. |
| `efficiency_growth_value` (instantTextboxType, `999`) | efficiency growth % | efficiency-pct | **unavailable** | Runtime-computed; not stored. |
| `production_efficiency_loss_value` (instantTextboxType, `999`) | efficiency loss from switching | efficiency-pct | **unavailable** | Runtime-computed; not stored. |
| `factory_bombing_defence_value` (instantTextboxType, `999`) | factory bombing-defence % | efficiency-pct | **unavailable** | Runtime-computed; not stored. |
| `resources_grid` (gridBoxType, in `resource_strip_container`) → `resource_strip_item` rows | `countries.<TAG>.economy.resources.available_derived.{steel,oil,aluminium,tungsten,chromium,rubber,coal}` | row factory; `resource_strip_item.icon`→resource-icon, `.value`→int | **derived** | National resource strip. `available_derived` is `S/D`. The `resource_strip_item.button` (`resource_item_background`) + `energy_ratio_anim` are static chrome on the row template. |
| `factories_used_equipment` / `_armor` / `_air` / `_capital_ships` / `_screen_ships` / `_other_ships` / `_resources` / `_outdated` / `_stockpiling` / `_reinforcing` / `_upgrading` / `_refitting` (12 × instantTextboxType, `110`) | per-FILTER factory tallies (sum of `production.military_lines`/`naval_lines` active_factories grouped by category/flag) | int | **derived** | Each counter = active factories on lines matching that filter bucket. We can re-derive the per-category grouping FROM `military_lines`/`naval_lines` (active_factories) for the equipment/armor/air/capital/screen/other buckets; the **stockpiling/reinforcing/upgrading/refitting/outdated** buckets need per-line STATE flags we do not store → those four+outdated are **unavailable** within this derived set. Conservative badge: derived (computable buckets) with the flag-dependent buckets unavailable. |

### Production-line row templates (instantiated by `item_grid`, repeated per line)

These windows are the row factory output for `countries.<TAG>.production.military_lines[]` (land/air)
and `…naval_lines[]` (ships). Element shape per line: `{id, active_factories, cost, speed, produced,
amount}`. The two collapsed variants + railway-gun + ship-refit + new-line variants bind the SAME
per-line fields (a render-mode choice, not new data). Mapped once per distinct bound field.

| gui element (name + type, in `production_military_line_entry` / `production_naval_line_entry`) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `name` (instantTextboxType) | `…production.military_lines[].id` → equipment-variant name lookup | category (id→loc) | **derived** | Line's equipment-variant id is measured (M); the display NAME is a registry lookup (variant id→loc), not a stored string. |
| `general_name` (instantTextboxType) | archetype/general name of the line's equipment | category (id→loc) | **derived** | Archetype label; lookup from the variant id. |
| `priority` (instantTextboxType, `7`) | per-line priority | int | **unavailable** | **RESISTED in `production_live`** (per-line priority/requested_factories/variant_index not offset-pinned — schema.py Production docstring). Badge unavailable. |
| `factories_used_txt` (instantTextboxType) | `…production.military_lines[].active_factories` | int | **exact** | Measured active factories on the line (line count + SUM(active) EXACT per docstring). |
| `factories_grid` (gridBoxType) → `production_military_factory_item` / `production_naval_factory_item` rows | `…military_lines[].active_factories` (count of factory pips) | row factory; pip icon per active factory | **exact** | One factory pip (`icon`=`GFX_factory_item` + `icon_overlay` half-pip) per active factory; count = `active_factories`. The pip windows are pure-icon templates (no own datum beyond the repeat count). |
| `amount` (instantTextboxType, `"INFINITY"`, naval/railway only) | `…production.{military,naval}_lines[].amount` | int / `∞` | **exact** | Target build count (`INFINITY` = no cap). `amount` is in the measured per-line shape. |
| `total_need` (instantTextboxType, `"Total need: 200"`) | per-line total equipment NEED across the army | int | **unavailable** | "Total need" is a computed demand aggregate (army requirement minus stock); not stored per line. |
| `speed` (instantTextboxType, `"--"`) | `…lines[].speed` (per-line build speed text) | category/proxy | **proxy** | `speed` IS in the measured shape, but the in-game text is a computed "units per period" with a denominator (cost/efficiency) we don't fully store → render the measured raw as a proxy, not the exact in-game string. |
| `progressbar` (iconType, `GFX_prod_progress_bar2`) | `…lines[].produced` / `…lines[].cost` | progressbar-fraction | **derived-from-exact** | **[Codex: produced + cost are exact M leaves; the FRACTION is derived from them.]** Progress to next unit = produced/cost; numerator + denominator both measured exact, the displayed ratio is computed. |
| `efficiency_progressbar` (iconType) + `efficiency_progressbar_to_max` (iconType) | per-line production EFFICIENCY (current / cap) | progressbar-fraction | **unavailable** | Per-line efficiency + efficiency cap are runtime-computed, not in the measured line shape (which carries id/active/cost/speed/produced/amount only). Badge unavailable; bars render inert. |
| `creator_flag` (iconType, `GFX_flag_small`) | the equipment variant's CREATOR tag | flag-small | **derived** | Variant designer/creator TAG; derivable from the variant catalog when the creator is recorded, else unavailable. |
| `equipment_icon` (iconType, `GFX_technology_medium`) | variant → equipment sprite frame | equipment-icon | **derived** | Sprite frame chosen from the variant archetype (icon-map lookup). |
| `sub_tech` / `equipment_role` (iconType) | variant role icon | category | **derived** | Role/sub-tech icon from variant metadata. |
| `has_mio_background` (iconType, `GFX_production_item_has_mio`) | line has an attached MIO? | visibility | **unavailable** | MIO-attachment per line is not in the measured line shape; visibility flag undeterminable → unavailable (renders default). |
| `deploy_name` (instantTextboxType, naval) | naval deploy target name | category | **unavailable** | Deploy location for a naval line is not stored (target-geometry RESISTED, like construction location). |
| `ship_name` (instantTextboxType, ship-refit entry) | refitting ship's name | category | **unavailable** | Per-ship refit identity not in the line shape. |

### Equipment / naval overview entry templates (`military_equipment_info_entry`, `naval_equipment_info_entry`)

These rows populate the equipment-picker sub-windows (`production_equipment_window*`). Out of the
political-mode core but mapped for completeness — they bind the equipment-VARIANT catalog.

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `name` (instantTextboxType, `"Maus"`) | `designs.equipment_variants` / `designs.equipment_variants_live` (variant name) | category | **derived** | Variant catalog name; `equipment_variants` save-owned (S), `_live` is M. |
| `general_name` (instantTextboxType) | variant archetype name | category | **derived** | As above. |
| `production_cost` / `naval_production_cost` (instantTextboxType, `"cost 9001"`) | variant production cost (IC) | int | **derived** | Per-variant cost is in the variant-stats catalog (`designs.equipment_variant_stats`, D). |
| `equipment_icon` (iconType) | variant → sprite | equipment-icon | **derived** | Icon-map lookup. |
| `creator_flag` (iconType) | variant creator TAG | flag-small | **derived** | From the variant catalog. |
| `resources_grid` (gridBoxType) → `production_resource_item` / `production_type_resource_item` rows | variant per-unit resource cost `{resource: amount}` | row factory; `icon`→resource-icon, `amount`→int | **derived** | Resource cost map from the variant catalog. |
| `dominance_value` (instantTextboxType, naval) | naval dominance contribution | int | **unavailable** | Computed naval-dominance stat; not stored. |
| `role_icon` / `equipment_role` / `sub_tech` / `carrier_fighter_icon` (iconType) | variant role/type icons | category | **derived** | Metadata-driven icon frames. |
| `missions` (containerWindowType) → `mission_grid` (gridboxtype) rows | air-wing mission tags for the variant | row factory; category | **unavailable** | Per-variant mission tag list not stored. |

### MIO counter

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `available_MIOs` (instantTextBoxType, `999`, in `production_MIOs`, both equipment windows) | count of available MIOs for the country | int | **derived** | Derivable from `mio_live.by_tag[<TAG>]` (M) — count of usable MIOs. The in-game number is the same aggregate (computed, not a stored scalar). |
| `outdated_units` (instantTextboxType, `"OUTDATED_EQ"`) | count of outdated equipment designs | int | **unavailable** | "Outdated" classification is a runtime comparison vs the latest variant; not stored as a count. |

## 1b. Production — deferred (needs imperative adapter / out of replay slice)

| gui element group | why deferred |
|---|---|
| **Naval-repair queue** — top window `navy_repair_window` (+ `count_container` w/ `dockyards_used_txt`, the 6 add/sub buttons, `toggle_expand_all`, `items_grid`) and its entry templates `naval_repair_queue_base_entry` (`dockyard_name`, `ship_count`, `repair_count`, `dockyard_state`/`ally_dockyard_state`) + `naval_repair_queue_ship_entry` (`ship_name`, `ship_type`, `time_left`, `progress_bar`, `bar_str`/`bar_org`, `damaged_part_report`, `flag`, `cancel_repair`, `shield`) | **deferred: needs adapter** — the naval-repair queue is a per-dockyard/per-ship assignment view fed by an OOB/ship adapter (`order_of_battle.ships`, M) joined to a repair-assignment model we don't store. Mutation controls (add/sub/cancel) inert. Out of political-mode slice. |
| **Naval deployment** — top window `production_deployment_window` (`deployments_grid`) + entry templates `production_deployment_port_item` / `_task_force_item` / `_navy_theater_item` (`name`, `icon`) | **deferred: needs adapter** — deploy-target list = an OOB/port adapter; out of slice. |
| **Ship-name list editor** — top window `ship_name_list_window` (`name_list_grid`, `new_name_box` editBox, `add_name_button`) + entry template `naval_name_list_entry` (`name`, `remove`, `assign`) | **deferred: mutation** — name-list editing is a mutation (PLAN §D, DISABLED). `editBoxType` + add/remove/assign inert. |
| **Equipment-role icon selector** — top window `equipment_role_icon_selection_window` (`box` grid, `btn_role`/`btn_close`) | **deferred: mutation** — role-icon assignment is a mutation; selector popup inert. |
| **`production_consumergoods_line_entry`** (`goods`, `factories`, `picture`) | **deferred** — consumer-goods line = a construction/economy-derived special line; its factory count is a computed consumer-goods allotment, surfaced in the Construction view's special entries (see View 2) rather than re-bound here. |

## 1c. Production — STATIC / decoration / navigation / mutation-inert (no data binding)

Listed by name so the map is provably exhaustive. None carry a datum.

- **Backgrounds / chrome / frames:** every `background` (`GFX_tiled_plain_bg`, `GFX_production_win_top`, `GFX_tiled_window2_1b_border`, `GFX_tiled_generic_overlay_bg1`, `GFX_tiled_bg_1_scrollbar`, `GFX_production_item`, `GFX_production_item_collapsed`, `GFX_naval_production_item_bg_strip`, `GFX_factory_bg`, `GFX_name_list_item_bg`, `GFX_repair_queue_*_bg`, `GFX_mini_bg`, `GFX_generic_bg2`, `GFX_tiled_window`, …), `production_header_bg`, `header_bg`, `MIO_bg`, `generic_text_bg_203`, `role_icon_bg`, `bg`, `production_line_selected`(+`_collapsed`), `priority_frame`, `efficiency_progressbar_frame`, `prod_line_glow`, `new_glow`/`tutorial_glow`/`tutorial_glow_2`/`infantry_button_glow`/`tank_button_glow`/`aircraft_button_glow`/`naval_button_glow` (animated cosmetics), `equipment_designer`/`equipment_designer_can_upgrade`/`outdated_equipment_overlay`(+collapsed variants)/`blueprint_icon`/`design_invalid`/`new_glow` overlays, `icon`/`icon_overlay`/`icon_header_bg` chrome, `need_state_icon`, `deploy_icon`.
- **Static label texts (loc, no datum):** `production_title` (`PRODUCTION_PRODUCTION_LABEL`), `title` (repair/deploy/name-list headers), `operational_factories` / `operational_naval_factories` (label text — their *values* are in §1a), `from_trade`, `goods`/`factories` placeholders on consumer-goods/static entries.
- **Static icons:** `military_factory_icon`, `naval_factory_icon`, `naval_factory_repair_icon`, `dockyard_icon`(`_with_wrench`), `production_output_icon`, `efficiency_cap_icon`, `efficiency_growth_icon`(`efficiency_icon`), `production_efficiency_loss_icon`, `factory_bombing_defence_icon`, `docyards_output_icon`, `MIO_icon`, `oil_icon`/`rubber_icon`/`steel_icon`/`aluminium_icon`/`tungsten_icon`/`chromium_icon`/`coal_icon` (resource-filter icon row), `resource_icon`, `ship_icon`, `add_name`.
- **Mutation controls — INERT in replay (read-only, PLAN §D):** the root add-line buttons `infantry_button`/`tank_button`/`aircraft_button`/`naval_button`/`naval_repair_button` have executable no-op proof from real `production.json` offsets plus measured 81x41 button frames: pointer-down/up are consumed with no commands, no redraw, unchanged selected row/scroll/visible rows/bind digest, and empty space between aircraft and naval groups remains unclaimed. `close_production_line` on visible rows has executable no-op proof: pointer-down/up are consumed with no commands, no redraw, unchanged selected row/scroll/bind digest, and neighboring row-body clicks still selecting normally. The per-line `factories_add_1x/5x/10x` and `factories_sub_1x/5x/10x` controls now use the preserved `@prod_*` GUI defines (`prod_sub_pos_x=379`, `prod_add_pos_x=453`) and also have executable no-op proof with unchanged selected row/scroll/bind digest and neighboring row-body clicks still selecting normally. The visible row priority/amount controls `increase_priority`/`decrease_priority`, military `btn_x5`/`btn_x10`, and naval `increase_amount`/`decrease_amount` also have executable no-op proof with unchanged selected row/scroll/bind digest and neighboring row-body clicks still selecting normally. Visible manufacturer/equipment/deploy/name-list controls now have executable no-op proof for `production_military_line_entry` `attach_manufacturer`/`change_manufacturer`, `equipment_button` (drag/drop), `expand`, and `convert`, plus `production_naval_line_entry` `attach_manufacturer`/`change_manufacturer`, `equipment_button`, `expand`, `carrier_air_wing_composition`, `deploy_location`, and `name_list_btn`. `variant`/`obsolete` equipment-info controls and unrendered refit-template controls remain unproven mutation surfaces. Primary filter `checkBoxType` toggles (`show_equipment`/`show_armor`/`show_air`/`show_capital_ships`/`show_screen_ships`/`show_other_ships`) are replay-local row filters, not mutations: they classify via `game_defs/out/catalogs.json` `equipment_archetypes`, follow `archetype` links, use unique live-id to save-`line_id` joins when live rows lack `archetype`, and leave unknown/ambiguous rows visible. The secondary filters (`show_resources`/`show_outdated`/`show_stockpiling`/`show_reinforcing`/`show_upgrading`/`show_refitting`) plus the 7 resource checkboxes (`oil_checkbox`…`coal_checkbox`) still need backing per-line flags or disabled visual/no-op treatment.
- **Secondary/status filter controls:** `show_resources`/`show_outdated`/`show_stockpiling`/`show_reinforcing`/`show_upgrading`/`show_refitting` now have executable disabled no-op proof from real checkbox rects: clicks are consumed with no commands, no redraw, unchanged selected row/scroll/bind digest, and no bleed into row selection. True status filtering still needs backing per-line flags or faithful recompute; the 7 detail-resource checkboxes remain tied to the unimplemented line-detail surface.
- **Navigation / window controls:** `close_button` / `close` / `btn_close` (open/close view), `minimize`/`expand` window arrows, `btn_role`, `btn_select`.
- **positionType anchors (no visual):** the 6 `positionType` slots use the preserved `@prod_*` define table from `countryproductionlineview.gui` (`prod_icon_pos_x=358`, `prod_sub_pos_x=379`, `prod_add_pos_x=453`, `prod_nof_pos_x=403`) so row-control geometry resolves before hit-testing.

---

# VIEW 2 — CONSTRUCTION (`game_assets/gui/construction.json`)

Root `countryconstructionsview` + 9 sibling top-level windows (the construction line-entry templates
+ the "start construction" / "convert IC" picker buttons + 4 static special-line entries). Binds the
player/observed country `<TAG>`. Formatters: `int`, `factory-split` (`n/m`), `building-icon` (frame
of `GFX_buildings_strip`), `resource-icon`, `progressbar-fraction`, `flag`, `category`.

> **Note A — construction location: live RESISTED, save AVAILABLE.** The live leaf
> `countries.<TAG>.production.construction_queue` (`construction_orders`, M) intentionally RESISTS
> the build target STATE/location (province handle behind the template def → needs a province→state
> map; never fabricated). **But** the SAVE leaf `countries.<TAG>.economy.construction_queue` (S) DOES
> carry `location_state` AND the building `template` name (verified in the demo snapshot:
> `{"template":"arms_factory","location_state":75,…}`). So when the snapshot is SAVE-backed the
> `location` text + building picture are **exact**; when only the LIVE channel is present they are
> **unavailable**. Rows are badged for the live channel (the Phase-3 primary), with the save fallback
> noted. The `relative_flag` (build-location owner flag) is `unavailable` either way (no per-line
> owner tag is stored; the location_state→owner join is possible but not wired).

## 2a. Construction — dynamic (data-bound) elements

Interaction proof: `ConstructionView` has measured construction-queue row
selection and wheel-scroll/thumb-drag coverage. Tests prove live `line_id` row
identity, save-backed `template`/`location_state`/`created_date` row identity,
selected-row highlight, scrolled row clicks, A->B->A row rebuilds, unkeyed rows
inert without index fallback, missing selected rows fail closed, outside-grid
wheel input is inert, thumb drags clamp and continue outside the grid until
pointer-up, and track clicks do not select underlying rows. The top
`count_container` factory add/sub buttons are also proven replay-disabled
no-ops with unchanged selection/scroll/digest and neighboring clicks unclaimed.
The rendered queue row priority/amount/close buttons are also proven
replay-disabled no-ops with unchanged selection/scroll/digest and neighboring
row-body clicks still selecting normally. Building drilldowns, scrollbar art,
build-picker actions, market/debug controls, disabled visuals, and broader
construction mutation parity remain open.

### Top summary (`countryconstructionsview > production_win_top` + `count_container`)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `available_factories_value` (instantTextboxType, `"20/37"`) | `countries.<TAG>.economy.factories.civ` (free / total civ for construction) | factory-split | **derived** | Total civ = `economy.factories.civ` (**S**, exact). The free-vs-assigned split (numerator) = total minus civ factories assigned to construction/trade/consumer-goods → a computed split; render derived. |
| `local_factories_value` (instantTextboxType, `99`) | `countries.<TAG>.economy.factories.civ_from_states` | int | **derived** | **[Codex: `civ_from_states` is SCHEMA.md/data drift, not in FIELD_SOURCES → badge derived, not exact.]** Locally-owned civ factories; the value IS present in the snapshot (`civ_from_states`=108 in demo) but the leaf is not authoritative in FIELD_SOURCES. |
| `traded_factories_value` (instantTextboxType, `99`) | civ factories received via trade (`economy.factories.civ` − `civ_from_states`, or lend-lease) | int | **derived** | Traded-in civ = total − local (or the lend-lease civ count). Derivable from `economy.factories.*`. |
| `cic_bank_value` (instantTextboxType, `0`) | banked civilian IC (construction bank) | int | **unavailable** | The CIC bank balance is a runtime-accumulated construction-economy scalar; not stored. |
| `construction_speed_value` (instantTextboxType, `99`) | national construction-speed modifier % | int | **unavailable** | Construction-speed modifier is a computed modifier aggregate; not stored. |
| `factories_used_txt` (instantTextboxType, `42`, in `count_container`) | civ factories currently building (= SUM `production.construction_queue[].active_factories`) | int | **derived** | Sum of active factories across the construction queue (each line's `active_factories` is M-measured); the aggregate is computed. |
| `resources_grid` (gridBoxType, in `speed_container > resource_strip_container`) → `resource_strip_item` rows | `countries.<TAG>.economy.resources.available_derived.{…}` | row factory; resource-icon + int | **derived** | National resource strip (same as production); `available_derived` is S/D. |

### Construction line-entry row templates (instantiated by `production_lines > item_grid`)

The live queue `countries.<TAG>.production.construction_queue[]` (each `{line_id, template_id,
active_factories, cost, produced, priority}`) feeds these rows; the save queue
`economy.construction_queue[]` (each `{template, location_state, active_factories, produced, cost,
priority, created_date}`) is the richer fallback (Note A). Templates: `production_building_line_entry`
(standard) + `production_railway_gun_repair_entry` (railway-gun repair, same fields).

| gui element (name + type, in `production_building_line_entry`) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `picture` (iconType, `GFX_buildings_strip`) | `…construction_queue[].template_id` → building sprite frame (live) OR `economy.construction_queue[].template` (save name) | building-icon | **derived** (live + save) | Live: frame from `template_id` (M) via building icon-map (derived). Save: the `template` is an **exact schema TOKEN** (`"arms_factory"`), but the building-sprite-frame choice from it is a token→icon lookup, so the rendered icon is derived. **[Codex: `template` is a token, not a localized name — the icon lookup is the derived step.]** |
| `progress` (iconType, `GFX_construction_progressbar`) | `…construction_queue[].produced` / `.cost` | progressbar-fraction | **exact** | Build progress = produced/cost; both measured (M live; also S in save queue). |
| `factories` (instantTextboxType, `"-"`) | `…construction_queue[].active_factories` | int | **exact** | Factories assigned to this build line (M-measured; demo save shows 15). |
| `queued` (instantTextboxType, `"-"`) | queued building COUNT for this line | int | **unavailable** | The "queued count" (how many copies of this building are queued at this location) is not in the measured live shape (which carries one line's scalars); not separately stored. Badge unavailable. |
| `location` (instantTextboxType, `"Maus"`) | `economy.construction_queue[].location_state` → state name (SAVE) | category (state→loc) | **unavailable** (live) / **derived** (save) | **Live: RESISTED → unavailable** (Note A). Save: from `location_state` id joined to state meta (derived — id is exact, name is a static-meta lookup). |
| `relative_flag` (buttonType, `GFX_construction_flag`) | build-location owner TAG | flag | **unavailable** | No per-line owner tag stored; the `location_state`→owner join is not wired. |
| `priority` (— there is no per-line priority TEXT element; priority is `increase_priority`/`decrease_priority` buttons, inert) | `…construction_queue[].priority` | — | **exact** (data present, no display element) | The `priority` int IS measured (M/S) but this entry template renders priority only via the inert up/down buttons + queue ORDER, not a text box. Datum present, bound to row ORDER. |

### Static / special construction line entries (separate top-level windows, repeated by the grids)

These four `static_*` windows + `production_consumergoods_line_entry` are special non-buildable lines
the construction grid emits for fixed factory-allotment categories.

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `static_consumer_goods_entry.factories` (instantTextboxType, `"-"`) | civ factories allotted to consumer goods | int | **unavailable** | Consumer-goods factory count = a computed allotment (econ law × stability); not stored. `name` (`PRODUCTION_CONSUMER_GOODS`) + `picture` static. |
| `static_traded_factories_entry.factories` (instantTextboxType) | civ factories given away via trade | int | **derived** | Derivable from `economy.factories.civ` − `civ_from_states` (traded-out) or lend-lease; computed. `name`/`picture` static. |
| `static_project_factories_entry.factories` (instantTextboxType) | civ factories on special projects | int | **unavailable** | Special-project factory allotment not stored at country level. `name`/`picture` static. |
| `static_contract_payment_entry.factories` (instantTextboxType) | civ factories paying a market contract | int | **unavailable** | Market-contract factory payment is a computed deduction; not stored. `name`/`picture`/`open_market_view_btn` static/inert. |

## 2b. Construction — deferred (needs imperative adapter)

| gui element group | why deferred |
|---|---|
| **Buildable-building picker** — `possible_constructions` (+ sub-panels `possible_state_constructions` / `possible_shared_constructions` / `possible_province_constructions` / `possible_facility_constructions`, each with a header icon + label + `*_item_grid`) and the picker entry templates `start_construction_entry` (`start_construction_button` `GFX_buildings_strip` + `start_construction_overlay`) + `convert_ic_entry` (`convert_ic_button` + `convert_ic_overlay`) | **deferred: mutation + scripted availability** — the "what can I build here" picker is a mutation surface (start construction / convert IC = PLAN §D DISABLED) whose entries depend on the selected state's category, tech, and scripted build conditions (recorded `visible`, §C.3) we do not evaluate in the slice. The grids render their headers (static, §2c) but instantiate no rows. |

## 2c. Construction — STATIC / decoration / navigation / mutation-inert (no data binding)

- **Backgrounds / chrome:** `Background` (`GFX_tiled_window_thin_border`), `construction_screen_top_bg`, `production_header_bg` (`GFX_construction_header_bg`), `bg` (`GFX_generic_box_80`), `Background`(`GFX_tiled_window2_1b_border` ×2 + `GFX_tiled_generic_overlay_bg1_small`), `Construction_Background` (×2), the 4 `*_buildings_background` (`GFX_diplo_details_header`), entry-template `Background`s (`GFX_construction_entry_bg`, `GFX_construction_special_entry_bg`), `new_glow`, `tutorial_glow`, `ongoing_contruction_anim` (animated), `overlay` (`GFX_building_status_overlay`), `repair` (`GFX_building_repair`), `scorched_icon`, `start_construction_overlay`/`convert_ic_overlay`.
- **Static label texts (loc):** `construction_title` (`CONSTRUCTION_VIEW_LABLE`), `location` (`CONSTRUCTION_LOCATION_LABEL` — column header), `task` (`CONSTRUCTION_TASK_LABEL`), `factories_amount` (`CONSTRUCTION_FACTORIES_LABEL`), `priority` (`CONSTRUCTION_PRIORITY_LABEL` — column header), `from_trade`, `local_factories` (`owned`), `state_constructions_text` (`STATE_BUILDINGS`), `shared_constructions_text` (`SHARED_BUILDINGS`), `province_constructions_text` (`PROVINCE_BUILDINGS`), `facility_constructions_text` (`FACILITY_BUILDINGS`), the `static_*` entry `name` labels. (Note: the column-header `location`/`priority` are distinct from the per-row `location` value bound in §2a.)
- **Static icons:** `civilian_factory_icon` (`GFX_non_available_factory_icon`), `factories` (`GFX_non_available_factory_icon`), `factories_used` (`GFX_factory_icon_with_wrench`), `cic_bank_icon`, `construction_speed_icon`, `state_constructions`/`shared_constructions`/`province_constructions`/`facility_constructions` header icons, `picture` (on static entries).
- **Mutation controls — INERT (read-only):** `factories_add_1x/5x/10x`, `factories_sub_1x/5x/10x` (top count_container) have executable no-op proof: pointer-down/up are consumed with no commands, no redraw, unchanged selected row/scroll/bind digest, and neighboring count-container clicks unclaimed. The rendered `production_building_line_entry` row controls `increase_priority`/`decrease_priority`, `increase_amount`/`decrease_amount`, and `close_production_line` also have executable no-op proof with unchanged selected row/scroll/bind digest and neighboring row-body clicks still selecting normally. `temp` (backhoe debug button), `open_market_view_btn`, build-picker actions, and non-rendered queue templates remain unproven mutation surfaces.
- **Navigation / window controls:** `minimize`, `expand`, `close_button`.
- **positionType anchors:** the 1 `positionType` slot.

---

# VIEW 3 — DECISIONS (`game_assets/gui/decisions.json`)

Root `countrydecisionview` (→ `decision_grid_container > decision_grid`, the row factory) + 10 sibling
entry-template windows (category headers, decision items, event items, timed-decision items, on-map
locators). Binds the player/observed country `<TAG>`. The decision list is an **array→grid with
conditional CATEGORY HEADERS → grouping adapter** (PLAN §C.2/§C.4). Formatters: `int`, `category`
(category/decision id→loc + icon frame), `flag`/`flag-small`, `progressbar-fraction`, `days`.

> **What the replay can bind (PLAN §C.4 verdict: "active/timed exact; one-shot availability not
> stored").** The LIVE leaf `countries.<TAG>.decisions.timed_live` (`decisions_market`, M) carries
> the country's **currently-active TIMED decisions**, each `{decision, days, state, state_name}`. So
> the **`timed_decision_item`** row (and the timed fields of `targeted_decision_item`) bind exact.
> The full catalog of AVAILABLE/clickable/locked one-shot decisions, their COST, their scripted
> `visible`/`available` conditions, and the category membership of inactive decisions are **NOT
> stored** (SCHEMA §3.6 note: one-shot clicks not recoverable) → those rows are badged unavailable.
> The decisions GROUPING (which category a timed decision sits under) is the imperative grouping
> adapter (§C.2); category id is derivable from the decision id via a static decisions DB map.

Interaction proof: `production.decisions` now has executable timed-decision row selection and
wheel-scroll/thumb-drag coverage for the active `timed_live[]` rows. Selection keys use measured
`decision` plus measured `state` when present, scrolled clicks select the backing row rather
than a visible index, scroll/selection scrub A->B->A rebuilds current-hour rows, unkeyed rows
are inert, missing selected rows fail closed, outside-grid wheel input is inert, thumb drags
clamp and continue outside the grid until pointer-up, and track clicks do not select underlying
rows. This is not a claim for one-shot decision availability/click mutation, category collapse,
tracking checkboxes, or scrollbar art.

## 3a. Decisions — dynamic (data-bound) elements

### `timed_decision_item` (the row the replay actually binds, repeated per `timed_live[]` element)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `name_text` (instantTextboxType) | `…decisions.timed_live[].decision` → decision loc | category (id→loc) | **derived** | **[Codex correction: exact→derived.]** The decision *id* is measured EXACT (M) and the timed-decision SET membership is exact, but the displayed NAME is a static loc lookup (id→loc), so the bound text value is derived (not a stored string). |
| `timer_text` (instantTextboxType, `999`) | `…decisions.timed_live[].days` | days (int) | **exact** | Days remaining on the timed decision (measured). |
| `cost_and_timer_text` (instantTextboxType, `999`) | `…decisions.timed_live[].days` (timer) | days | **exact** (timer) / **unavailable** (cost) | This box shows timer-or-cost; the TIMER half is `days` (exact). The recurring-COST half (PP/command power per tick) is not stored → cost portion unavailable. |
| `target_flag` (iconType, `GFX_flag_small2_not_masked`) | `…decisions.timed_live[].state` → owner TAG, or the decision's target country | flag-small | **derived** | For state-targeted timed decisions the target is `state` (id, measured); the FLAG needs a state→owner or decision→target-country join (derivable, not a stored tag). `target_flag_frame` is static chrome. |
| `icon` (iconType, `GFX_decision_unknown`) | `…timed_live[].decision` → decision category/icon frame | category (id→frame) | **derived** | Decision icon from the decision-DB metadata (id→sprite lookup). |
| `timer_icon` (iconType, `GFX_decision_timer_icon_strip`) | visible iff timed (always for this row) | category-frame | **derived** | Timer-state frame; the row is always timed so visible; frame band could encode urgency (derived from `days`). |
| `btn_progress_good` / `btn_progress_bad` (iconType) | timed-decision tick direction (good/bad) | progressbar-fraction | **unavailable** | The "progressing good/bad" overlay needs the decision's dynamic-modifier progress + sign, not stored. Badge unavailable. |

### Per-state-target / on-map decision rows (bound only where `timed_live[].state` is present)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `targeted_decision_item.name_text` / `.timer_text` / `.cost_and_timer_text` | as `timed_decision_item` above (when the active timed decision is target-scoped) | category / days | **derived** (name, per Codex) / **exact** (timer = `days`) / **unavailable** (cost) | Same binding as the timed row; this template is the targeted variant. `name_text` = loc lookup (derived); `timer_text`/timer half of `cost_and_timer_text` = `days` (exact). Non-timed targeted decisions (availability-gated) are unavailable. |
| `targeted_decision_item.target_flag` (iconType, `GFX_flag_small`) | `…timed_live[].state` → state owner | flag-small | **derived** | Target state's flag (state→owner join). |
| `on_map_decision_locator_item.name_text` | active on-map decision name (`…timed_live[].decision`) | category | **derived** | On-map locator label; bound only for active on-map-scoped timed decisions; `decision_icon`/`zoom_icon` are static + the zoom verb is read-only. |

### Category headers + description (grouping adapter output, §C.2)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `category_header.name_text` (instantTextboxType) | decision CATEGORY id → loc (grouping adapter groups `timed_live[]` by category) | category (id→loc) | **derived** | The adapter buckets active timed decisions by their category (decision id→category via static DB), emitting one header per non-empty category. Category id is derivable; the header only appears for categories that HAVE an active timed decision (inactive categories not surfaced — see §3 note). |
| `category_header.icon` (iconType, `GFX_decision_category_generic`) | category id → category-icon frame | category (id→frame) | **derived** | Category sprite from the decisions DB (id→icon lookup). |
| `decision_category_desc.full_text` / `.short_text` (instantTextboxType) | category description loc | category | **unavailable** | Category long/short description = static loc tied to the category; surfaced only if the category-DB is loaded. Without the decisions-DB join it is unavailable; with it, derived. Conservative: unavailable (not in snapshot). `picture` static. |
| `decision_category_desc.custom_icons` (OverlappingElementsBoxType) → `custom_icon_item` rows (`value` text + `target_flag`) | per-category custom counter icons (e.g. resource/target counts) | row factory; int + flag-small | **unavailable** | Category custom-icon counters are scripted dynamic values evaluated at runtime; not stored. |

## 3b. Decisions — deferred (needs imperative adapter / not stored)

| gui element group | why deferred / unavailable |
|---|---|
| **Full available-decision catalog** — `decision_item` (standard clickable one-shot: `name_text`, `cost_and_timer_text`, `icon`, `target_flag`(+`_frame`), `btn_progress_good`/`_bad`, `btn_select`, `track_decision_checkbox`) and the non-timed branch of `targeted_decision_item` | **unavailable: not stored** — the catalog of AVAILABLE one-shot decisions, their clickable/locked state, COST, and scripted `visible`/`available` conditions are not recoverable (SCHEMA §3.6: one-shot clicks not stored; PLAN §C.4: "one-shot availability not stored"). The grouping adapter can only surface decisions that left persistent state (the timed_live set). These rows render only for active timed decisions; the broader catalog is badged unavailable. |
| **Event feed** — `event_header` (+ the 3 popup checkbox windows `minor_event_checkbox_window` / `event_checkbox_window` / `news_checkbox_window` w/ their `popup_*_checkbox` + text) and entry template `event_item` (`name_text`, `timer_text`, `event_type_icon`, `btn_bg`) | **deferred: needs adapter** — the pending/fired EVENT feed is an alerts/event adapter (PLAN §C.4); `world_events.{fired_events,pending_events}` (S) exists but is world-scoped and not folded into these per-country snapshots → no per-country pending-event list to bind. The popup-preference checkboxes are UI-state toggles (no datum). |
| **`category_collapse_btn` / `track_decisions_checkbox` / `track_decision_checkbox` / `btn_select`** | **inert: mutation/UI-state** — collapse, track, and select are interaction verbs (track = a UI preference mutation; select = the decision-click mutation, DISABLED §D). |

## 3c. Decisions — STATIC / decoration (no data binding)

- **Backgrounds / chrome / frames:** `Background` (`GFX_tiled_window2_1b_border`), `production_header_bg` (`GFX_header_bg`), `Background` (`GFX_tiled_window_transparent`), `icon_header_bg` (category + event), `btn_bg` (decision/event items), `btn_progress_good`/`btn_progress_bad` (when not bound — overlay chrome), `target_flag_frame` (`GFX_small_flag_frame_thin2`), `icon_end_bg` (`category_end`), `decision_category_desc.Background` (`GFX_tiled_decisions_bg_small`), `decision_category_desc.picture`.
- **Static label texts (loc):** `decisionview_title` (`DECISIONVIEW_TITLE`), `event_header.name_text` (`DECISIONSVIEW_EVENT_HEADER`), `minor_events_popup_text` (`POPUP_MINOR_EVENTS`), `events_popup_text` (`POPUP_EVENTS`), `news_popup_text` (`POPUP_NEWS`).
- **Structural containers (row factory + grouping):** `countrydecisionview`, `decision_grid_container`, `decision_grid` (the gridbox row factory — carries no datum; its rows are the entry templates above), `category_end` (a spacer-row template = pure chrome).
- **Navigation:** `close_button` (close view).

---

## Notes

**A. Construction location — see View 2 Note A.** The single most consequential `unavailable` in this
sweep is the construction build-LOCATION: RESISTED in the live channel, but AVAILABLE in the save
channel (`economy.construction_queue[].location_state`). Phase-3 binds the live channel primary, so
the live row is badged `unavailable` for location with the save fallback explicitly noted. This is a
"schema-present in the OTHER channel" gap, not a hard limit.

**B. `unavailable` ≠ "missing forever".** Many `unavailable` badges here are schema-present-but-
parser-unfed in THIS channel (construction location via save queue; decision availability; MIO
attachment per line) or world-scoped domains not folded into these per-country snapshots (event feed
← `world_events`). They render "—" per the no-fabrication rule and become `exact`/`derived` once the
relevant join/parser is wired. Genuinely runtime-computed gaps (efficiency cap/growth/loss, daily
output, CIC bank, construction speed, total-need, consumer-goods allotment, decision cost,
progress-good/bad overlays) stay `unavailable`/`derived` by nature (PLAN §G hard-limit class).

**C. Render-mode duplicate templates.** Production defines collapsed / new-line / railway-gun / ship-
refit variants of each line entry; construction defines a railway-gun-repair variant. These bind the
SAME per-line fields as their canonical template (a render-state choice, not new data) and are NOT
double-counted as distinct bindings — they are listed under their canonical mapping.

---

## Element accounting (provably complete)

Counts below are over the parsed `.gui` trees (named elements; layout primitives `borderSize`,
`margin`, `slotsize`, `max_slots`, `padding`, `positionType` are the structural grid/text plumbing,
counted under "static/structural"). Every named element falls in exactly one bucket.

### PRODUCTION (`production.json`) — 663 total parsed nodes; 31 top-level windows

| bucket | count (named elements) | examples |
|---|---:|---|
| **Dynamic-bound** (incl. derived/unavailable badges) | **~46** | operational/naval factory splits (×3), the 4 unavailable output/efficiency values, resources_grid rows, 12 filter tallies, line-entry `name`/`general_name`/`factories_used_txt`/`amount`/`speed`/`progressbar`/`factories_grid`/`creator_flag`/`equipment_icon` (×2 land+naval templates, mapped once), equipment-info `name`/`cost`/`resources_grid`, `available_MIOs`, `outdated_units` |
| **Deferred: needs adapter / mutation** | **~5 groups (~45 elements)** | naval-repair queue (window + 2 entry templates), naval deployment (+3 item templates), ship-name editor (+entry), role-icon selector, consumer-goods line |
| **Static / decoration / nav / mutation-inert / structural** | **~570** | 40 backgrounds, 170 icons (factory/resource/efficiency/MIO chrome), 159 buttons (add/sub/priority/manufacturer/deploy/close — inert), 13+7 checkboxes, glows/overlays, label texts, 53 borderSize, 25 slotsize, 8 margin, 7 max_slots, 6 positionType |

### CONSTRUCTION (`construction.json`) — 149 total parsed nodes; 10 top-level windows

| bucket | count (named elements) | examples |
|---|---:|---|
| **Dynamic-bound** (incl. derived/unavailable badges) | **~17** | available/local/traded factory values, cic_bank, construction_speed, count_container `factories_used_txt`, resources_grid rows, line-entry `picture`/`progress`/`factories`/`queued`/`location`/`relative_flag` (×2 templates, mapped once), 4 `static_*` entry factory counts |
| **Deferred: needs adapter / mutation** | **2 groups (~14 elements)** | buildable-building picker (4 sub-panels + 2 picker entry templates) |
| **Static / decoration / nav / mutation-inert / structural** | **~118** | 15 backgrounds, header/factory/speed icons, column-header + section labels, add/sub/priority/amount/minimize/expand/close/temp/market buttons (inert), 15 borderSize, 6 slotsize, 1 max_slots, 1 margin, 1 positionType |

### DECISIONS (`decisions.json`) — 103 total parsed nodes; 11 top-level windows

| bucket | count (named elements) | examples |
|---|---:|---|
| **Dynamic-bound** (incl. derived/unavailable badges) | **~16** | `timed_decision_item` (name/timer/cost-or-timer/target_flag/icon/timer_icon/progress ×7 fields), `targeted_decision_item` timed fields, `on_map_decision_locator_item.name_text`, `category_header.name_text`/`.icon`, `decision_category_desc.full_text`/`.short_text`/`custom_icons` rows |
| **Deferred: needs adapter / not stored** | **3 groups (~25 elements)** | full one-shot `decision_item` catalog, event feed (`event_header` + 3 popup windows + `event_item`), track/collapse/select verbs |
| **Static / decoration / structural** | **~62** | 3 backgrounds, 29 icons (item/category/event/timer/progress chrome + frames), title/event-header/popup labels, `decision_grid`/`category_end` structural, `close_button`, 20 borderSize, 1 slotsize |

### `unavailable`-badged elements (with reason) — the no-fabrication ledger

**Production (≈14 logical):** `operational_naval_factories_repair_value` (repair-dockyard split not stored), `docyards_output_value` + `production_output_value` (daily output computed-not-stored), `efficiency_cap_value` + `efficiency_growth_value` + `production_efficiency_loss_value` + `factory_bombing_defence_value` (efficiency aggregates computed), line `priority` (RESISTED in production_live), `total_need` (computed demand), line `efficiency_progressbar`(+`_to_max`) (per-line efficiency not in line shape), `has_mio_background` (per-line MIO flag absent), `deploy_name`/`ship_name` (deploy/refit target RESISTED), `outdated_units` (computed), naval `dominance_value` + variant `missions` rows (computed/absent), the stockpiling/reinforcing/upgrading/refitting/outdated filter buckets (per-line state flags absent).

**Construction (≈6 logical):** `cic_bank_value` (bank balance not stored), `construction_speed_value` (modifier computed), line `location` + `relative_flag` (LIVE channel RESISTED — save fallback available, Note A), line `queued` (queued-count absent), `static_consumer_goods_entry.factories` / `static_project_factories_entry.factories` / `static_contract_payment_entry.factories` (computed factory allotments).

**Decisions (≈6 logical):** `cost_and_timer_text` cost half (recurring cost not stored), `btn_progress_good`/`btn_progress_bad` (dynamic-modifier tick not stored), `decision_category_desc.full_text`/`.short_text` (category desc not in snapshot), `custom_icons` rows (scripted counters not stored), the entire one-shot `decision_item` catalog + non-timed `targeted_decision_item` (availability not stored — SCHEMA §3.6), event feed (`event_item`/`event_header` — world-scoped `world_events` not folded per-country).

**Net:** all `unavailable` elements are schema-present-but-unfed-in-this-channel, world-scoped-not-folded, or genuinely runtime-computed. None are fabricated; each renders "—" per the no-fabrication rule rather than being omitted, so each view's `.gui` tree is provably 100% accounted for.

---

## Historical review

Historical note: this records a completed adversarial review from the old
Claude-era workflow. Codex-native sessions must use normal sub-agents/partner
review and must not run the old local CLI command. The review checked `replay/schema.py` FIELD_SOURCES +
dataclass docstrings + `replay/SCHEMA.md`. Verdict: **"Mostly agree, with real refutes."** It
independently confirmed the live production-line reader shape against
`memory_reader/production_live.py` (line shape `{id, active_factories, cost, speed, produced, amount}`;
per-line priority/requested_factories/variant_index explicitly RESISTED — confirms the unavailable
badge).

**AGREED:** all source paths (`production.{military,naval}_lines`, `military/naval_factories`,
`stockpile`/`stockpile_total`, live `production.construction_queue`, `decisions.timed_live`
`{decision,days,state,state_name}`, save `economy.construction_queue` HAS `location_state`+`template`);
the operational-factory `n/m` derived badge; efficiency cap/growth/loss + bombing-defence unavailable;
per-line production priority unavailable (and construction-queue priority IS stored — kept distinct);
construction `location` unavailable (live) / available (save); `cic_bank`/`construction_speed`
unavailable; one-shot decision catalog + availability + cost unavailable (SCHEMA §3.6 + hard-limit
lines 793-801); event feed unavailable (world-scoped `world_events`, not folded per-country).

**REFUTED / corrected (all folded into the tables above, tagged `[Codex …]`):**
1. **Decision `name_text`: exact → derived** — `timed_live[].decision` *id* is exact, but the
   displayed name is a static loc lookup. (Also applied to `targeted_decision_item.name_text`.)
2. **Live construction location wording** — it is *absent/RESISTED*, NOT a stored `null` child; the
   row shape simply omits a location key. Tightened the "RESISTED (null)" phrasing.
3. **`economy.factories.civ_from_states`/`offmap`** — present in SCHEMA.md + the demo snapshot but
   **not in authoritative `FIELD_SOURCES`** (schema-doc drift). Bindings using them
   (`local_factories_value`, traded split) re-badged **derived** and the drift is flagged.
4. **`production_output_value`/`docyards_output_value`** — `military_lines[].speed`/`naval_lines[].speed`
   ARE stored M leaves; if the UI renders a sum-of-line-speeds aggregate the badge is **derived**, not
   unavailable. Split the badge (country-modifier display = unavailable; line-speed aggregate =
   derived).
5. **Progress bars** (`produced/cost`) — re-badged **derived-from-exact** (the two leaves are exact M;
   the displayed fraction is computed from them).
