# BINDING MAP — INTEL · DIPLOMACY (trade / peace / tension) · WAR / INSIGNIA

> **What this is.** The COMPLETE, no-truncation data-binding map for **eleven** in-game the game views the
> native replay UI binds in the Phase-3 sweep, covering the **intelligence**, **trade / peace / world-tension**,
> and **war / faction / insignia** clusters (Plan §C.4 rows "Intel / operatives / MIO",
> "Diplomacy / trade / peace", "War / faction / insignia"):
>
> 1. `intel_agency.json`         — `interface/countryintelligenceagencyview.gui`
> 2. `intel_ledger.json`         — `interface/countryintelledger.gui`
> 3. `operative.json`            — `interface/operative.gui`
> 4. `operative_leader.json`     — `interface/operativeleader.gui`
> 5. `operation_overview.json`   — `interface/operationoverview.gui`
> 6. `trade.json`                — `interface/countrytradeview.gui`
> 7. `peace_conference.json`     — `interface/peaceconferencewindow.gui`
> 8. `world_tension_popup.json`  — `interface/worldtensionwindowpopup.gui`
> 9. `war_overview.json`         — `interface/waroverview.gui`
> 10. `insignia_selection.json`  — `interface/insigniaselection.gui`
> 11. `ins_revolution.json`      — `interface/insrevolutionwindow.gui`
>
> It realises GAME_UI_REPLICA_PLAN §C (comprehensive but mechanical binding sweep) + §C.2 (roster
> aggregation for grid rows) + §D (read-only: every mutation verb is inert chrome). EVERY element in
> each `.gui` tree is accounted for, in one of three buckets: **dynamic-bound** (data-driven),
> **static/decoration** (no binding), or **deferred-needs-adapter** (mutation modal / future adapter).
> Where a `.gui` element exists but our snapshot has no usable field it is listed and badged
> **`unavailable`** with the missing field named. Each view ends with a provably-exhaustive
> element-count table reconciled against the parsed JSON's own `stats.element_type_counts`.
>
> **The diplomacy country card itself (`diplomacy.json` / `countrydiplomacyview.gui`) is already mapped
> in `BINDING_MAP_diplomacy_combat.md` and is NOT re-mapped here.** The views here are its siblings.
>
> **Binding-rule shape** (PLAN §C.1): `{ target (gui element name+type, sprite/text, loc key),
> source (snapshot dotted path), format (formatter id), fidelity (exact|derived|proxy|unavailable) }`.
> **Source paths** use SCHEMA dotted form: `countries.<TAG>.intel.…`, `diplomacy_world.…`,
> `world_events.threat[]`. `<SEL>` = the selected/observed country (the one the panel is open on);
> `<VIEWER>` = the player/observed root country.
>
> **No-fabrication (SCHEMA §0.2, PLAN principle 3):** `null` renders "—", never `0`. A measured `0`/`0.0`
> stays. A schema-present field the current parser never feeds is badged `unavailable` (not silently
> dropped). A runtime-computed value we don't model is `derived`/`proxy`/`unavailable` by nature. An
> operative `gfx` is a **sprite NAME**, not image data (a separate asset-extraction step renders it).

---

## Provenance of the fidelity calls

Grounded in **empirical inspection of the demo snapshot**
(`replay/snapshots/replay/000017005619.snapshot.json`, `schema_version "1"`, date `1941.4.13.11`, GER
1941 channel, 143 countries) cross-checked against `replay/SCHEMA.md` / `replay/schema.py` §3.7
(diplomacy), §3.8 (intel), §7b (diplomacy_world), §15 (strategic_operatives), §16 (raids),
§17 (world_events), §18 (world), and **adversarially reviewed by Codex** (read-only, 2026-06-26).

### DEMO-SNAPSHOT ARTIFACT vs TRUE GAP — the central calibration

The demo snapshot is **SAVE-ONLY** and was produced before several schema domains were folded into the
writer. Many `unavailable` badges below are therefore **artifacts of this demo capture**, NOT permanent
gaps. Empirically confirmed by walking the snapshot:

| schema domain | state in the demo snapshot | classification |
|---|---|---|
| top-level keys present | only `combats, countries, diplomacy_world, divisions, events, map, provinces, states` | — |
| **`world_events`** (incl. `threat[]`) | **top-level key ABSENT** (returns `None`) | **ARTIFACT** — domain not folded into this demo; §17 schema exists & is parser-fed elsewhere |
| **`strategic_operatives`** (§15 pools/networks) | **top-level key ABSENT** | **ARTIFACT** — not folded into this demo |
| **`raids`** (§16) | **top-level key ABSENT** | **ARTIFACT** (not used by these 11 views; noted for completeness) |
| **`market_live`** / `diplomacy_world.trade_contracts` | `trade_contracts` = **`null`**; `market_live` key **ABSENT** | **ARTIFACT** — equipment-market not folded; schema-present (§7b) & unfed |
| `countries.*.intel.operatives[]` | **POPULATED** — 39 operatives across 143 countries (8 GER), each `{name, skill, experience, traits[], state, nationalities[]}` | **bindable (exact)** for those 6 fields |
| operative `op_id` / `gfx` / `operation_id` / `mission` / `captured` / `captured_by` | **NOT emitted** in this demo (key union is exactly the 6 fields above; 0 captured, 0 gfx, 0 op_id) | **ARTIFACT** — §3.8 defines them; this parser build didn't emit them → portrait/ETA/mission detail unavailable |
| operative `state` | only `on_operation` (4) / `on_mission` (35); none captured | drives the status badge |
| `countries.*.intel.{agency, recruitable_operatives, operations, decryption, peace_conference}` | **`null` for ALL 143 countries** | **ARTIFACT** — §3.8 defines them; unfed in this demo |
| `countries.*.intel.intel_networks` | `[]` (empty) everywhere | empty leaf (graph lives in §15) |
| `countries.*.diplomacy.opinion` | **rich dict**, 143 targets for GER: `{attitude, opinion_sum, opinion_modifiers[], guarantee, non_aggression_pact, military_access, embargo, at_war}` | exact (values); per-modifier names need DEF lookup |
| `countries.*.diplomacy.lend_lease[]` | **`[]` (empty) for ALL countries** | **ARTIFACT** — equipment-market unfed (matches `trade_contracts=null`) |
| `countries.*.diplomacy.{guarantees, non_aggression, embargoes, volunteers, attaches, expeditionary}` | **`[]` (empty)** everywhere in this demo | mostly genuine (no such relations) / partly ARTIFACT (reserved lists) |
| `countries.*.diplomacy.wargoals[]` | **POPULATED** (GER: 6 — `{type, target, states[], expire}`) | exact |
| `diplomacy_world.{wars, factions, subjects}` | **POPULATED** — 75 wars (`{a,b,start_date,a_casualties,b_casualties,war_score_a_vs_b,war_score_b_vs_a}`), 4 factions (`{id, ideology, icon, template, members[], leader, leader_change_date}`), 5 subjects (`{overlord, subject, autonomy_state, value, start_date}`) | exact |
| `world.world_tension_inputs` (and the AGGREGATE tension scalar) | inputs only; **aggregate tension VALUE not stored anywhere** (`world` top-level itself absent in this demo) | **TRUE GAP** — COMPUTED-NOT-STORED |

> **Read-only verb note (PLAN §D).** Every button whose in-game role is to CREATE/START/SEND/LAUNCH/
> DECRYPT/CALL/REFUND/DISMISS/CHANGE-NAME/SET-COLOUR is a **mutation** verb. In a replay these render as
> real chrome but are **inert** (no datum, no action). They are listed under each view's static/deferred
> section, never bound. The OUTCOMES they would produce are surfaced via the diff-event timeline.

---

## GAP LEDGER — classification key

Per the task's required scheme, every `unavailable` element is tagged with two columns:

* **ARTIFACT** — schema domain exists & is parser-fed in the pipeline, merely **unfed in THIS demo
  snapshot**. Becomes `exact`/`derived` once the writer folds the domain (no engineering wall). This is
  a FOLD-IN/RECONCILE item, not a true gap.
* **TRUE GAP** — not obtainable from the current snapshot model. Sub-bucketed by CLOSURE:
  * **(a) LIVE-EXTRACTABLE** — a memory read could obtain it (live reader work).
  * **(b) COMPUTED-NOT-STORED** — a runtime-only value (Ghidra tier): e.g. the **AGGREGATE world-tension
    VALUE**, opinion-modifier breakdown math, operation **success-chance / outlook**, intel-level force
    **estimates**, capitulation %.
  * **(c) STATIC DEF / WORLD DATA** — parseable from game `common/` / `interface/`: e.g. the operation
    DEF catalog, insignia/emblem catalog, opinion-modifier DEF (names), casus-belli/wargoal DEF,
    resource & idea sprite catalogs.

A consolidated gap ledger is at the end of the document.

---
---

# VIEW 1 — INTEL AGENCY (`game_assets/gui/intel_agency.json`)

Root: `containerWindowType "countryintelligenceagencyview"` — the player/observed country's OWN
intelligence agency (the agency is a property of `<VIEWER>`, not a foreign target). Surfaces:
`name_logo` (agency identity + spymaster), `before_creation_panel` (pre-agency state),
`agency_branches` (upgrade tree summary), `agency_agents` (operative pool counters + progress),
`agency_operations` / `agency_crypto` (two tab bodies), plus several **runtime row-factory templates**
(`upgrade_branch_entry`, `upgrade_entry`, `operation_view_entry`, `cryptology_country_entry`,
`logo_list_entry`, `agency_upgrade_*`) and the insignia / branch-upgrade popups.

Formatters: `loc`, `int`, `signed-int`, `days` (int+" days"), `progressbar-fraction` (0..1),
`flag-small`, `category` (insignia/idea token→sprite), `percent`.

> **Whole-view fidelity ceiling in the demo:** `<VIEWER>.intel.agency` is **`null` for all 143
> countries** → the agency identity / branch / counter / progress block is **`unavailable`
> (ARTIFACT)**. `intel.operations` and `intel.decryption` are likewise `null` → the operations and
> cryptology tab bodies are `unavailable` (ARTIFACT). The view is enumerated in full below so
> completeness is provable; it lights up `exact`/`derived` once `intel.agency`/`operations`/`decryption`
> are folded.

## 1a. Intel agency — dynamic (would-bind; agency block)

| gui element (name + type, sprite/text, loc) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `agency_name` (instantTextboxType) | `countries.<VIEWER>.intel.agency.name` | loc | **unavailable** (ARTIFACT) | `agency=null` in demo. Custom agency name. |
| `agency_logo` (buttonType, `GFX_intelligence_agency_logo_bel`) | `countries.<VIEWER>.intel.agency.icon` | category (insignia→sprite) | **unavailable** (ARTIFACT) | Agency insignia sprite name; also a name-change verb (inert). |
| `agency_defense_level` (instantTextboxType, "tees") + `counter_intelligence_icon` | `countries.<VIEWER>.intel.agency.capacity.defense` | int | **unavailable** (ARTIFACT) | Counter-intel defense level. `capacity.defense` (§3.8). |
| `total_operative_text` (instantTextboxType, "10 (-1)") + `total_operative_icon` | `len(<VIEWER>.intel.operatives)` / `<VIEWER>.intel.agency.capacity.max_operatives` | int "n/max" | **derived** | Current count is **exact** (`len(operatives)`); the limit needs `agency.capacity.max_operatives` (path per §3.8; `agency=null` → limit unavailable). |
| `arrested_operative_text` (instantTextboxType, 0) + `arrested_operative_icon` | count of `operatives[]` with `state=="captured"`/`captured==true` | int | **derived** | Derivable from operatives, but in demo 0 are captured AND `captured` field unemitted → renders `0`. |
| `dead_operative_text` (instantTextboxType, 0) + `dead_operative_icon` | dead-operative tally | int | **unavailable** (ARTIFACT) | No dead-operative tally in `intel` (operatives are live-only); §15 networks don't carry it. |
| `agency_branches_title` / `ai_factory_allocation` (instantTextboxType) | branch headers / AI factory split | loc / int | **unavailable** (ARTIFACT) | `agency.upgrades` unfed; `ai_factory_allocation` is an AI-debug readout. |
| `upgrades_list` (gridBoxType) → **`upgrade_branch_entry` / `upgrade_entry` ROWS** | `<VIEWER>.intel.agency.upgrades{branch:level}` + `current_upgrade` | row factory; `upgrade_name`→loc, `num_factories`→int, `upgrade_icon`→category, progress | **unavailable** (ARTIFACT) | The branch-upgrade catalog rows. `agency.upgrades` null in demo; sprite/loc per upgrade also needs the agency-upgrade DEF (STATIC DEF). |
| `upgrade_progressbar` (`progressbar_content/desc/days`) | `<VIEWER>.intel.agency.upgrade_progress` + `current_upgrade` | progressbar-fraction + days | **unavailable** (ARTIFACT) | Active branch-upgrade progress. |
| `operatives_progressbar` (`progressbar_content/desc/days`) | recruit-operative progress | progressbar-fraction + days | **unavailable** (ARTIFACT) | New-operative recruitment timer; no field (operatives are a snapshot list, no in-flight timer). |
| `creation_progressbar_factories` (15) / `creation_progressbar_days` ("180 days") / `creation_progressbar_content` | agency-creation progress | int / days / fraction | **unavailable** (ARTIFACT) | Pre-creation state; `agency.is_created`=false branch. |
| `spymaster_button_cost` (122) / `spymaster_title` / `spymaster_button_name` / `spymaster_flag` (`GFX_flag_small`) | spymaster status (who is faction spymaster) | int / loc / flag-small | **unavailable** | TRUE GAP (a): which faction member is spymaster is not in `intel`/`diplomacy_world` schema → would need live extract. |

## 1b. Intel agency — OPERATIONS tab (`agency_operations`) — `operation_view_entry` row factory

The operations tab `operations_grid` (gridBoxType) is a **ROW FACTORY** over running/finished operations.
Mapped once as the entry template (`operation_view_entry`); it repeats per operation. **`intel.operations`
is `null` in the demo → all `unavailable` (ARTIFACT).**

| `operation_view_entry` element (name + type, sprite) | source (per `intel.operations.running[]`/`finished[]` item) | formatter | fidelity | notes |
|---|---|---|---|---|
| `operation_name_text` (instantTextboxType) | `…operations.running[i].type` | loc (operation id→name) | **unavailable** (ARTIFACT) | Name via operation DEF (STATIC DEF) once `operations` fed. |
| `operation_icon` (iconType, no sprite = data-driven) | `…running[i].type` | category (operation→sprite) | **unavailable** (ARTIFACT) | Icon from operation DEF. |
| `btn_target_flag` (buttonType, `GFX_flag_small`) | `…running[i].target` | flag-small | **unavailable** (ARTIFACT) | Target country flag. |
| `operation_prepared_remaining_days_text` ("60 days") + `operation_prepared_icon` | `…running[i].date`+`duration` vs `save_date` | days | **unavailable** (ARTIFACT) | Prep countdown; progress derives from save_date vs date+duration (§3.8 note). |
| `operation_duration_text` (instantTextboxType) | `…running[i].duration` | days | **unavailable** (ARTIFACT) | |
| `operatives_required_text` / `network_strength_text` (instantTextboxType) | `len(running[i].operatives)` / `strategic_operatives.pools[idx].networks[].vertices` (§15) | int | **unavailable** (ARTIFACT) | Required-operative count derivable from `operations`; **network strength → §15 `strategic_operatives.pools` (danger/subversive/networks)** — top-level `strategic_operatives` key ABSENT in demo → both unfed (ARTIFACT). |
| `operation_ai_score` (instantTextboxType, "xxxxx") | AI desirability score | int | **unavailable** | TRUE GAP (b): AI-internal score, COMPUTED-NOT-STORED. |
| `container_tokens` icons (`token_airforce_icon`/`token_army_icon`/`token_civilian_icon`/`token_navy_icon`/`token_resistance_contacts_icon`) | operation requirement flags | category | **unavailable** (ARTIFACT) | Requirement tokens from operation DEF. |
| `operation_completed_icon` / `operation_completed_warning` / `clip_icon` | lifecycle state of `…finished[]` | category | **unavailable** (ARTIFACT) | Completed-operation markers. |
| `operations_not_active` (instantTextboxType, `OPERATIONS_NOT_ACTIVE`) | static empty-state | loc | **static-with-data** | Shown when `running`+`finished` empty (the demo state). |

## 1c. Intel agency — CRYPTOLOGY tab (`agency_crypto`) — `cryptology_country_entry` row factory

`country_list` (gridboxtype) is a **ROW FACTORY** over decryption targets. **`intel.decryption` is `null`
in the demo → all `unavailable` (ARTIFACT).**

| element (name + type, sprite) | source (per `intel.decryption.targets[]` item) | formatter | fidelity | notes |
|---|---|---|---|---|
| `agency_crypto_defense_level` ("Defense Level: 10") + `defense_level_icon` | `<VIEWER>.intel.agency.capacity.defense` | int | **unavailable** (ARTIFACT) | Own crypto defense level. |
| `agency_decrypt_strength` (500) | `intel.decryption.intel_source` / strength | int | **unavailable** (ARTIFACT) | Aggregate decryption strength. |
| `cryptology_country_entry.flag` (buttonType, `GFX_flag_small2`) + `flag_frame` | `…decryption.targets[i].target` | flag-small | **unavailable** (ARTIFACT) | Target country flag. |
| `cryptology_country_entry.name` (instantTextboxType, "Country Name") | `…targets[i].target` | loc | **unavailable** (ARTIFACT) | Target country name. |
| `cryptology_state` ("50%") + `decrytion_bar`/`_done`/`_bg` | `…targets[i].amount` (accumulated points) / threshold | percent / progressbar | **unavailable** (ARTIFACT) | Decryption % — `amount` accumulates toward a code threshold; the threshold is a STATIC DEF, so % would be `derived` once fed. |
| `decryption_done_icon` | `…targets[i].decryption==true` | category | **unavailable** (ARTIFACT) | Actively-decrypting marker. |
| `crypto_not_active` (instantTextboxType, `CRYPTO_NOT_ACTIVE`) | static empty-state | loc | **static-with-data** | Demo state. |

## 1d. Intel agency — STATIC / decoration / navigation / mutation (no binding)

- **Chrome / frames / titles / bgs:** `Background` (`GFX_tiled_bg`/`GFX_tiled_paper_bg`/`…_flat_bg`),
  `production_header_bg`, `intelligenceagencyview_title` (`INTELLIGENCEAGENCYVIEW_TITLE` loc),
  `name_logo`→`Background`/`agency_logo_bg`, `spymaster_icon`, `title_bg` (×2, `GFX_intel_header_bg`),
  `agency_branches_title`/`agency_agents_title` (loc), `operatives_bg`, `empty_agency_slot_glow`,
  `upgrades_before_agency`/`operatives_before_agency`→`Background`, `progressbar_days`
  (`STATUS_NO_AGENCY_NO_OPERATIVE` loc), `no_operations_header`/`crypto_header` (`GFX_crypto_header`),
  `add_new_text` (`CLICK_TO_ADD_NEW` loc), `country_list_container`→`Background`, `progressbar_frame`
  (×many), `agency_upgrade_*` (cost/anim/glow/frame/icon decals), `branch_upgrades_title`,
  `upgrade_duration_title`, `civilian_factory_icon` (×2), `repetitions_icons`/`repetition_icon`,
  the insignia-popup `header_text` (`CHOOSE_INSIGNIA`), `tiled_window_insigna` bg, `group_name_bg`.
- **Tab / navigation / mutation buttons (inert):** `close_button` (`CLOSE`), `operations_tab_button`
  (`OPERATIONS`), `crypto_tab_button` (`CRYPTOLOGY`) — view-switch verbs; `creation_button`
  (`CREATE_AGENCY_BUTTON`), `cancel_creation_button`, `spymaster_button`/`faction_button`
  (`BECOME_SPY_MASTER_BUTTON`), `upgrade_button`/`background_button`/`upgrade_icon`/`cancel_upgrade`,
  `btn_select_button` (`GFX_launch_operation_button_anim` — LAUNCH op), `start_stop_button`/
  `activate_button`/`delete_button` (cryptology), `btn_close`/`create_button`/`logo_list_entry.button`
  (insignia popup), `agency_name_entry` (editBoxType — rename) — **all mutation verbs, inert**.
- **Scrollbar (UI control, no datum):** `right_vertical_slider_intel` (+ `Slider`/`Track`/`Decrease`/
  `Increase`/`Background`).
- **Structural containers / row-template roots:** `countryintelligenceagencyview`, `name_logo`,
  `spymaster`, `before_creation_panel`, `creation_progressbar`, `agency_branches`, `agency_agents`,
  `agency_operations`, `no_operations_container`, `operations_grid_container`, `agency_crypto`,
  `active_crypto_items`, `add_new_container`, `country_list_container`, `upgrade_button`,
  `agency_upgrade_researched_icon`, `name_logo_agency_selection_window`, `agency_name`, `logo_list`,
  `logo_list_entry`, `intelligence_agency_upgrades_window`, `branches`, `upgrade_branch_entry`,
  `upgrades`, `upgrade_entry`, `agency_upgrade_repetition_icon`, `operation_view_entry` (+ sub-containers
  `container_operation_info`/`container_operatives_required`/`container_network_strength`/
  `container_tokens`/`container_launch_operation`), `cryptology_country_entry` (+ `flag_container`/
  `crypto_bar_container`/`activate_button_container`).

---
---

# VIEW 2 — INTEL LEDGER (`game_assets/gui/countryintelledger.gui`)

Root: `country_intel_ledger_view` — the **fog-of-war intel ESTIMATE ledger** on a FOREIGN `<SEL>`
country (economy / army / navy / air tabs). Every value here is an in-game **intel ESTIMATE** (a min–max
band scaled by intel level vs the target's real stats), shown as `"123-512"` / `"45-120"` strings, not
the true value. Many `*_entry` containers at the top are **runtime ROW-FACTORY templates**
(`ledger_resource_entry`, `ledger_idea_entry`, `ledger_tech_entry`, `ledger_template_entry`,
`ledger_equipment_stockpile_entry`, `ledger_ship_*_entry`, `ledger_airwing_*_entry`,
`intel_ledger_plane_count_detail`).

> **Whole-view fidelity ceiling — the defining call.** These are **fog-of-war ESTIMATES** the in-game
> intel system computes from intel level vs the target's real `economy.*`/`military.*`. We store the
> *real* values but NOT the *estimate* (the intel-level-scaled min–max obfuscation). Binding the real
> value here would **misrepresent** the intel view (it would leak perfect information). Therefore every
> estimate cell is badged **`unavailable` — TRUE GAP (b) COMPUTED-NOT-STORED** (the estimate band is a
> runtime function of intel level, not stored). This mirrors the diplomacy card's 8× `estimated_*`
> readouts (`BINDING_MAP_diplomacy_combat.md` §1b). The empty-state (`NO_INTEL`) is the honest default.

## 2a. Intel ledger — economy / army / navy / air estimate cells (all `unavailable`)

| gui element group (name + type) | intended source | fidelity | reason |
|---|---|---|---|
| **economy tab** `resource_entries` grid → `ledger_resource_entry` rows (`resource` icon, `amount` "123-421", `chart`, `mini_piechart_overlay`, `trade_icon`) | intel estimate of `<SEL>.states[*].resources` / market | **unavailable** | TRUE GAP (b): resource estimate band. (Underlying real resources are in `states[*].resources`; the *band* is computed.) |
| `industrial_capacity`/`manpower`/`convoys_count`/`trucks_count`/`trains_count`/`supply_count`/`fuel_value` (`first_container`, 7× instantTextBoxType, 999) | intel estimate of `<SEL>.economy.*` | **unavailable** | TRUE GAP (b) for the estimate; real values exist (`economy`) but binding them misrepresents intel. |
| `estimated_civil_industry_size`/`estimated_military_industry_size`/`estimated_naval_industry_size` ("45-120") + `*_damaged_rate` ("35%") | intel estimate of `<SEL>` industry split + damage | **unavailable** | TRUE GAP (b). Same family as diplomacy `estimated_*`. |
| `factory_trade_text` ("45-120") | traded-factories estimate | **unavailable** | TRUE GAP (b). |
| `equipment_container` (`eq_text`/`armor_text`/`air_text`/`capital_text`/`screen_text`/`other_ship_text`, 6× "42-120") | intel estimate of stockpile by category | **unavailable** | TRUE GAP (b). |
| civilian-production **line chart** (`chart_line`, `max_y_value`/`min_y_value`/`min_x_value`/`max_x_value`, `no_info_container_text` `NO_INTEL`) | per-target production history estimate | **unavailable** | TRUE GAP (b): a time-series intel estimate, not stored. Renders `NO_INTEL`. |
| economy `ideas_grid` → `ledger_idea_entry` rows (`icon` `GFX_idea_unknown`, `trait`); `technology_entries` → `ledger_tech_entry` rows | intel estimate of `<SEL>.politics.ideas_other` / researched tech | **unavailable** | TRUE GAP (b) for the *intel-gated* subset; real ideas/tech exist but the ledger shows only intel-revealed ones. Sprites need idea/tech DEF (STATIC DEF). |
| **army tab** `army_count`/`special_forces_count`/`manpower_in_field`/`equipment_fulfilled` ("123-512"/"12-32"/"123K") | intel estimate of `<SEL>` land forces | **unavailable** | TRUE GAP (b). |
| army `templates_grid` → `ledger_template_entry` rows (`name`, `count`); `stockpiles_grid` → `ledger_equipment_stockpile_entry` rows (`icon`, `name`, `count`) | intel estimate of `<SEL>` division templates / stockpile | **unavailable** | TRUE GAP (b) (template/stockpile estimate); the empty `no_info_container` `NO_INTEL` is the honest default. |
| army/navy/air `footer` → `ideas_grid` / `doctrine_tech` / `technology_entries` | intel estimate of staff/research/doctrine | **unavailable** | TRUE GAP (b). |
| **navy tab** `fleet_count`/`task_force_count`/`manpower_in_field`/`ship_count` (estimate strings) | intel estimate of `<SEL>` fleets | **unavailable** | TRUE GAP (b). |
| navy `ship_type_grid` → `ledger_ship_type_entry`/`_header_entry`/`_entry` rows (`ship_icon`, `ship_name`, `ship_count`); `ships` grid | intel estimate of ship roster by class | **unavailable** | TRUE GAP (b). |
| **air tab** `airwing_count`/`manpower_in_field`; `planes_counts_container` → `ledger_airwing_*_entry` rows + `intel_ledger_plane_count_detail` (`count`); `airwings` grid | intel estimate of `<SEL>` air forces | **unavailable** | TRUE GAP (b). |

## 2b. Intel ledger — `country_info` header

| gui element (name + type) | source | formatter | fidelity | notes |
|---|---|---|---|---|
| `country_info` (containerWindowType, empty stub) | the shared diplomacy country-header subview (flag/name/leader) | — | **derived** | This re-includes the diplomacy country card header (mapped in `BINDING_MAP_diplomacy_combat.md`); flag/name bind from `<SEL>` meta, leader from `commanders` (absent → "—"). |
| `economy_tab_frame`/`army_tab_frame`/`navy_tab_frame`/`air_tab_frame` → `intel` (instantTextboxType, "Germanian") | `<SEL>` display name (tab label echoes target name) | loc | **exact** | The per-tab label shows the observed country's name. |

## 2c. Intel ledger — STATIC / decoration / navigation (no binding)

- **Chrome / frames / titles:** `background` (`GFX_tiled_window2_1b_border`), `Background`/`Background_2`
  (`GFX_mini_bg`/`_grey`), all `*_bg`/`*_icon` decorations (`industrial_bg`, `manpower_bg`, `convoys_bg`,
  `trucks_bg`, `trains_bg`, `supply_bg`, `fuel_bg`, the `estimated_*_bg`/`_icon`, `*_damaged_bg`/`_icon`,
  `eq_bg`/`armor_bg`/`air_bg`/`capital_bg`/`screen_bg`/`other_ship_bg` + icons, `bg`/`army_icon`/`icon`
  family), `no_intel_bg` (`GFX_no_intel`), `no_info_container_text` (`NO_INTEL` loc, ×many),
  `military_research_header`/`military_research_title` (`ARMY_LEDGER_*` loc), `tech_icon_deco`,
  `linechart_bg`/`chart_grid_bg`, tab-frame icons (`civilian_economy`/`army_overview`/`naval_overview`/
  `air_overview`, `economy_tab`/`army_tab`/`navy_tab`/`air_tab`).
- **Navigation / filter buttons (read-only view-switch verbs):** `armies_tab_button`/
  `equipment_tab_button` (`ARMY_LEDGER_*`), `tab_button` (×4 tab frames), the line-chart type filters
  (`chart_type_industry`/`_convoy`/`_bomber`/`_truck`/`_train` + icons) and ranges (`chart_size_3_years`/
  `_1_year`/`_3_months`), `ledger_tech_entry.button` — UI controls over an unavailable estimate.
- **Structural containers / positions / row-template roots:** `country_intel_ledger_view`,
  `tabbed_panels_container`, `economy_panel`/`army_panel`/`navy_panel`/`air_panel`,
  `resource_container`/`first_container`/`industry_container`/`industry_factory_container`/
  `equipment_container`/`civilian_production_container`/`linechart_filter_button_container`/
  `filter_button_container`/`bg_container`/`chart_container`/`chart_fg_container`/`chart_info`/
  `graph_no_info_container`, `footer` (×4), `ideas`/`doctrine`/`no_info_container` (×many),
  `army_count_panel`/`army_count_container`/`special_forces_container`/`manpower_in_field_container`/
  `equipment_fulfilled_container`, `templates`/`stockpiles`, `navy_count_panel`/`ship_type_container`/
  `ship_type_no_info`/`ship_count_container`/`ships_container`, `air_count_panel`/`airwing_type_container`/
  `airwings_container`, `min_footer_pos` (positionType ×4), and every `ledger_*_entry` /
  `intel_ledger_plane_count_detail` row-template root.

---
---

# VIEW 3 — OPERATIVE (`game_assets/gui/operative.gui`)

Root contains a SINGLE window: `boost_party_popularity_mission_window` — the **"boost party popularity"
operative-mission confirm modal** (title / description / `additionals_grid` / send / cancel). Despite the
filename, this `.gui` does NOT hold the operative roster (that is `operativeleader.gui`, View 4); it is an
operative-MISSION action dialog.

## 3a. Operative — dynamic / deferred

| gui element (name + type, sprite/text) | source | formatter | fidelity | notes |
|---|---|---|---|---|
| `title` (instantTextboxType, "Maus") | operation/mission name | loc | **deferred** (mutation modal) | This is the boost-party mission dialog header. |
| `description` (instantTextboxType, "Maus") | mission description | loc | **deferred** | |
| `additionals_grid` (gridboxtype) → mission-option rows | mission parameter options | row factory | **deferred** | Action-construction UI. |
| `send_button` (buttonType, `DIPLOMACY_SEND`) / `cancel_button` (`BACK`) | — | — | **deferred** (mutation) | START the operative mission → inert in replay. |

## 3b. Operative — STATIC / structural

- `Background` (`GFX_diplo_action_boost_party_bg`), `additionals`→`Background` (`GFX_tiled_paper_bg2`),
  `margin`. Structural: `boost_party_popularity_mission_window`, `additionals`.

> **Whole view is a deferred mutation modal** (PLAN §D): in a replay it is never shown; the boost-party
> OUTCOME (popularity shift) is surfaced via `politics.party_popularities` deltas + the diff timeline,
> not this dialog.

---
---

# VIEW 4 — OPERATIVE LEADER (`game_assets/gui/operativeleader.gui`)

Root windows: `operative_leader_window` (the scrollable operative strip + `operatives_box` gridbox),
`operative_empty_slot_window`, three portrait windows (`operative_leader_portrait_window` /
`_small` / `_medium`), `operative_order_bar` (the 8-mission picker), `operative_badge_view` (the selected
operative's detail card), `operative_nationality_entry` (row template), `operative_status_window`
(status/ETA bar), and `operative_view` (anchor). The portrait windows + `operative_nationality_entry`
are **runtime ROW FACTORIES** (one per operative / per nationality).

> **This is where operatives actually bind.** `<SEL or VIEWER>.intel.operatives[]` is **populated** in the
> demo (39 operatives, fields `{name, skill, experience, traits[], state, nationalities[]}`). So name /
> skill / traits / nationalities / state bind **exact**. But the demo omits operative `gfx`, `op_id`,
> `operation_id`, `mission`, `captured`, `captured_by` → portrait sprite, ETA, mission-icon detail, and
> captured state are **`unavailable` (ARTIFACT)**.

## 4a. Operative leader — `operatives_box` ROW FACTORY → portrait windows (one per `operatives[i]`)

| gui element (name + type, sprite) | source (per `intel.operatives[i]`) | formatter | fidelity | notes |
|---|---|---|---|---|
| `operative_leader_portrait_window.portrait_btn` (buttonType, `GFX_operative_unknown`) | `operatives[i].gfx` | category (gfx sprite NAME) | **unavailable** (ARTIFACT) | Portrait sprite NAME, not image data; `gfx` unemitted in demo → falls back to `GFX_operative_unknown`. |
| `operative_skill_level` (instantTextboxType) | `operatives[i].skill` | int | **exact** | Skill (1 in demo sample). |
| `mission_icon` (iconType, `GFX_operative_mission_icons_small`) + `on_mission_window` | `operatives[i].state=="on_mission"` + `mission` | category (mission→sprite) | **derived** | State is exact (drives which sub-window shows); the specific mission-icon needs `operatives[i].mission` (unemitted) → generic icon. |
| `operation_icon` (iconType, data-driven) + `on_operation_window` | `operatives[i].state=="on_operation"` + `operation_id`→`operations.running` | category | **derived** | State exact; operation icon needs `operation_id` join (unemitted) → generic. |
| `leader_captured` (iconType, `GFX_leader_portrait_captured`) | `operatives[i].captured` (fallback: `operatives[i].state`) | bool→show | **derived** | **Codex fix:** in the demo, derive **false** from `state` (no captured states present) → the overlay correctly hides. Binds `operatives[i].captured` exact once that field is emitted (ARTIFACT for the explicit flag). |
| `leader_enroute` (iconType, `GFX_leader_en_route`) | `operatives[i].state` en-route substate | bool→show | **unavailable** (ARTIFACT) | No en-route substate field. |
| `operative_active_mission_icn` / `operative_active_operation_icn` (anim) | `operatives[i].state` | category | **derived** | Active-activity animation, keyed off state. |
| `operative_empty_slot_window.total_operative_text` (instantTextboxType, "X/X") | `len(operatives)` / `agency.capacity.max_operatives` | int "n/max" | **derived** | Count exact; cap needs `agency` (unavailable). |

## 4b. Operative leader — `operative_badge_view` (selected operative detail card)

| gui element (name + type, sprite) | source (selected `operatives[i]`) | formatter | fidelity | notes |
|---|---|---|---|---|
| `operative_name` (instantTextBoxType) | `operatives[i].name` | loc | **exact** | "Johann Hein". |
| `operative_code_name` (editBoxType, "IHaveACoolName") | operative custom code name | text | **unavailable** | **Codex fix:** a separate code-name is NOT in SCHEMA §3.8 → **TRUE GAP (a) LIVE-EXTRACTABLE** (needs a new field), not an ARTIFACT. (Also an edit verb — inert.) `operatives[i].name` already binds the display name in the badge. |
| `operative_skill_level` (instantTextboxType, 2) + `skill_bg` | `operatives[i].skill` | int | **exact** | |
| `operative_title` (instantTextboxType) | operative rank/title | loc | **derived** | Title derived from `skill` band (STATIC DEF). |
| `traits_grid` (gridBoxType) → trait rows | `operatives[i].traits[]` | row factory; category (trait→sprite/loc) | **exact** (ids) / **derived** (sprite) | Trait IDs exact (`operative_safe_cracker`,…); icon/loc via operative-trait DEF (STATIC DEF). |
| `nationalities_grid` (gridBoxType) → `operative_nationality_entry` rows (`flag_icn`, `GFX_flag_small`) | `operatives[i].nationalities[]` | row factory; flag-small | **exact** | Nationality flags (`["GER"]`). |
| `agency_insignia` (iconType, `GFX_intelligence_agency_logo_prc`) | `<owner>.intel.agency.icon` | category | **unavailable** (ARTIFACT) | Owning agency insignia; `agency` null. |
| `operative_status_window.status_text` (instantTextBoxType) | `operatives[i].state` (+ `mission`/`operation_id`) | loc | **derived** | State→status text exact; the detailed "doing X at Y" needs `mission`/op join (unavailable). |
| `operative_status_window.eta_text` (instantTextBoxType) | `operatives[i].mission`/operation ETA | days | **unavailable** (ARTIFACT) | ETA needs `mission`/`operation` timing (unemitted). |
| `activity_progressbar_*` (mission/operation/disabled/captured) | `operatives[i].state` + activity progress | progressbar | **derived** (state) / **unavailable** (progress) | Which bar shows is exact (state); the fill % needs mission/operation progress (unavailable). |

## 4c. Operative leader — STATIC / decoration / navigation / mutation (no binding)

- **Chrome / frames:** `bg`, `top_status_bg`, `portrait_frame` (×several, `GFX_operative_frame`/
  `GFX_unit_leader_portrait_frame_badge`/`GFX_leader_group_frame_unselected`), `portrait_frame_hightlight`,
  `group_marker` (`GFX_group_marker2`), `skill_bg`, `background` (`GFX_operative_leader_badge`/
  `GFX_operative_missionbar_bg`), `portrait_btn_anim`/`portrait_anim_icon`, `activity_captured_alert`.
- **Mission picker (`operative_order_bar`) — 8 mission buttons + icons (`btn_mission_1..8`,
  `icn_mission_1..8`, `icn_mission_active_1..8`):** these set the operative's mission → **mutation verbs,
  inert**. The active-mission highlight WOULD key off `operatives[i].mission` (unavailable), so even the
  highlight is `unavailable`.
- **Navigation / mutation buttons (inert):** `btn_scroll_left`/`btn_scroll_right` (strip scroll),
  `portrait_btn` (open operative — read-only verb), `dismiss_btn`/`portrait_dismiss_btn` (dismiss op →
  mutation), `deselect_button` (`DESELECT_UNIT`), `portrait_btn` in empty slot (`GFX_group_add_operative_bg`
  — recruit → mutation).
- **Structural containers / positions / row-template roots:** `operative_leader_window`, `centering_box`,
  `operative_empty_slot_window`, `operative_leader_portrait_window`(+`_small`/`_medium`),
  `on_mission_window`/`on_operation_window`, `operative_order_bar`, `operative_badge_view`,
  `operative_nationality_entry`, `operative_status_window`, `operative_view`, `operatives_box`
  (gridBox row factory), `operative_portrait_pos`/`operative_status_bar_pos`/`entry_padding` (positionType).

---
---

# VIEW 5 — OPERATION OVERVIEW (`game_assets/gui/operationoverview.gui`)

Root: `operation_overview_window` — the **detail window for ONE operation** (`intel.operations.running[i]`):
header, target info column (`container_target_info`: flag, resources cost grid, duration / outlook bars,
operatives grid), phases clipboard (`phases_container`: phase-titles grid + phases grid + outcome text),
bottom action bar, reward. Several `*_view_entry` are **runtime ROW FACTORIES**
(`operation_phases_view_entry`, `operation_operative_view_entry`, `operation_phase_titles_view_entry`,
`operation_resources_view_entry`, `operation_operative_leader_portrait_window`).

> **Whole-view ceiling in the demo:** `intel.operations` is **`null` for all countries → the entire view
> is `unavailable` (ARTIFACT)**. Enumerated in full; lights up once `operations` is folded.

## 5a. Operation overview — dynamic (would-bind; one operation)

| gui element (name + type, sprite/text) | source (`intel.operations.running[i]`) | formatter | fidelity | notes |
|---|---|---|---|---|
| `operation_name_text` (instantTextboxType) | `…running[i].type` | loc (operation→name) | **unavailable** (ARTIFACT) | Name via operation DEF. |
| `operation_icon` (iconType, `GFX_operation_icon`) | `…running[i].type` | category | **unavailable** (ARTIFACT) | |
| `btn_target_flag` (buttonType, `GFX_flag_small2`) + `flag_name` (instantTextboxType) | `…running[i].target` | flag-small + loc | **unavailable** (ARTIFACT) | Target country flag + name. |
| `operation_progress_text` (instantTextboxType) + `duration_progressbar_container` | `…running[i].date`+`duration` vs `save_date` | progressbar-fraction | **unavailable** (ARTIFACT) | Progress = save_date vs date+duration (§3.8). |
| `operation_duration_text` (instantTextboxType) | `…running[i].duration` | days | **unavailable** (ARTIFACT) | |
| `operation_outlook_text` ("TEST") / `operation_outlook_label` + `outlook_container` progressbar | operation success outlook | percent | **unavailable** | TRUE GAP (b): success outlook is COMPUTED-NOT-STORED (runtime chance). |
| `resources_grid` → `operation_resources_view_entry` rows (`resource_icon`, `resource_amount`, `resource_progress`) | `…running[i]` resource/IC cost + accrual | row factory; int + progressbar | **unavailable** (ARTIFACT) | Per-resource cost rows; resource sprite via resource DEF (STATIC DEF). |
| `operatives_grid` → `operation_operative_view_entry` rows (portrait, `mission_icon`, `operation_icon`) | `…running[i].operatives[]` (→ join `intel.operatives[].op_id`) | row factory; category | **unavailable** (ARTIFACT) | Assigned-operative portraits; needs `operatives` join + `gfx` (both unavailable in demo). |
| `phase_titles_grid` → `operation_phase_titles_view_entry` rows (`phase_title_picture`, `phase_title_name`) | `…running[i].phases[]` | row factory; loc + category | **unavailable** (ARTIFACT) | Phase headers; picture via operation-phase DEF (STATIC DEF). |
| `phases_grid` → `operation_phases_view_entry` rows (`phase_progress_icon`, `phase_entry_icon`, `phase_entry_desc`) | `…running[i].phases[]` | row factory; progressbar + loc | **unavailable** (ARTIFACT) | Per-phase progress + description. |
| `operation_outcome_text` (instantTextboxType) | operation outcome narrative | loc | **unavailable** (ARTIFACT) | For finished ops. |
| `operation_reward_text` (instantTextboxType) + `operation_reward_icon`/`_bonus` | operation reward | loc + category | **unavailable** (ARTIFACT) | Reward via operation DEF (STATIC DEF). |
| `operation_complete_icon` / `operation_completed_warning` | lifecycle (`…finished[]`) | category | **unavailable** (ARTIFACT) | Completed markers. |

## 5b. Operation overview — STATIC / decoration / navigation / mutation (no binding)

- **Chrome / frames / bgs:** `Background` (`GFX_tiled_paper_w_frame_bg`), `operation_header_bg`
  (`GFX_tiled_header`), `operation_bg`/`container_target_info_bg`, `clip_icon`, `flag_frame`,
  `entry_bg` (`GFX_list_entry_bg`), `progressbar_frame` (×several), `top_Window`/`midsection`/
  `bottom_Window` clipboard bgs (`GFX_operations_clipboard_*`), `Background` (`GFX_operation_bottom_bg`),
  `reward_bg`, `resource_bg`/`resource_icon_bg`/`resource_progress_frame`, `portrait_overlay_bg`,
  `pin` (`GFX_phase_pin`), `phase_title_background`, `auto_commence_text`/`auto_repeat_text` (loc).
- **Navigation / mutation buttons + checkboxes (inert):** `btn_close` (`CLOSE`), `btn_location`
  (open location — read-only verb), `btn_start_button` (`GFX_button_221x34_anim` — START op → mutation),
  `btn_collect`/`btn_refund` (collect/refund → mutation), `cb_auto_commence`/`cb_auto_repeat`
  (checkBoxType — mutation toggles), `resume_mission_icon` decal.
- **Structural containers / positions / row-template roots:** `operation_overview_window`,
  `operation_header`, `container_target_info`, `operation_icon_container`, `flag_container`,
  `list_anchor_container`, `resources_grid_container`, `duration_container`/
  `duration_progressbar_container`, `outlook_container`, `operatives_container`, `phases_container`,
  `top_Window`/`midsection`/`bottom_Window`, `container_bottom`, `button_launch_container`,
  `reward_container`, `operation_operative_leader_portrait_window`, `operation_phases_view_entry`,
  `operation_operative_view_entry`, `operation_phase_titles_view_entry` (+ `phase_title_title`),
  `operation_resources_view_entry` (+ `resource_progress_container`), `operative_portrait_pos`
  (positionType).

---
---

# VIEW 6 — TRADE (`game_assets/gui/countrytradeview.gui`)

Root: `countrytradeview` — the classic **resource trade** view (buy/sell raw resources by civilian
factory): a resource balance strip (`resources` grid of `resources_info_entry`), resource & country
filters, and the **resource-trade roster** (`trade_grid` of `country_trade_entry` rows: per partner,
available-export / exported / convoys). Plus the `tradeoffer_window` modal (set up a buy), and the runtime
ROW FACTORIES `resources_info_entry`, `resources_filter_entry`, `filter_entry`, `country_trade_entry`.

> **Codex correction — this is RESOURCE trade, NOT the equipment market.** `countrytradeview.gui` is the
> raw-resource trade screen; the game 1.19 equipment-market / lend-lease (`diplomacy_world.trade_contracts`,
> `countries.*.diplomacy.lend_lease`) is a SEPARATE system surfaced elsewhere. So `country_trade_entry`
> rows do **NOT** bind to `trade_contracts` — there is **no resource-trade ledger domain in the current
> SCHEMA** (resource imports/exports per partner are not parsed). Therefore the resource-trade roster +
> the import/export/convoy legs are **TRUE GAP — (a) LIVE-EXTRACTABLE** (or a new save/parse domain), NOT
> an ARTIFACT of the unfed equipment market.
>
> **Whole-view ceiling in the demo:** the per-partner resource-trade rows + import/export/needed legs are
> `unavailable` (TRUE GAP a). The resource-balance strip's `produced` leg derives from controlled/owned
> `states[*].resources` (raw deposits → extraction-modified); the `balance`/`needed` legs additionally need
> demand + the (un-parsed) import/export, so they are derived/unavailable.

## 6a. Trade — resource balance strip (`resources` → `resources_info_entry` ROW FACTORY)

| `resources_info_entry` element (name + type, sprite) | source (per resource) | formatter | fidelity | notes |
|---|---|---|---|---|
| `resources_icon` (buttonType, `GFX_resources_strip`) | resource id (frame) | category | **derived** | Resource sprite frame from resource DEF (STATIC DEF). |
| `balance` (instantTextboxType) | net per-resource balance | signed-int | **derived** | Producible from `states[*].resources` aggregate minus need; needs a resource-balance adapter. |
| `produced` (instantTextboxType) | aggregate `states[*].resources` over `<SEL>`-controlled states | int | **derived** | From raw deposits (`states[*].resources`) with extraction modifiers applied → derived. |
| `imported` / `exported` (instantTextboxType) | resource import/export legs | int | **unavailable** | **Codex fix:** no resource-trade ledger in SCHEMA → **TRUE GAP (a) LIVE-EXTRACTABLE**, not an unfed-equipment-market ARTIFACT. |
| `special_project` / `production_line` (instantTextboxType) | resource consumed by special projects / lines | int | **unavailable** | Needs a production-line resource-consumption ledger (absent from `economy` in this demo) → TRUE GAP (a)/(b). |
| `needed` / `neededType` (instantTextboxType) | resource surplus/deficit | signed-int | **derived** | Surplus = produced+imported−consumed; the import/consumption legs are unavailable → partial. |

## 6b. Trade — header totals + contract grid

| gui element (name + type, text/loc) | source | formatter | fidelity | notes |
|---|---|---|---|---|
| `factories_total` / `factories_used` / `factories_available` (instantTextboxType) | `<SEL>.economy` civilian-factory split (trade allocation) | int | **derived** | Total civ factories exist in `economy`; the *traded* split needs the market (partly unavailable). |
| `trade_grid` (gridBoxType) → **`country_trade_entry` ROWS**: `country_flag` (`GFX_flag_small2`)/`country_flag_frame`, `country` (name), `available_export`, `exported`, `convoys` | per-partner RESOURCE-trade ledger (NOT `trade_contracts`) | row factory; flag-small + loc + int | **unavailable** | **Codex fix:** this is the resource-trade roster, and there is **no resource-trade ledger in SCHEMA** → **TRUE GAP (a) LIVE-EXTRACTABLE** (or a new parse domain), NOT the equipment-market ARTIFACT. The partner FLAG/NAME (`country_flag`/`country`) could bind if the resource-trade partners were read; the export/convoy legs need the (unparsed) resource market. |

## 6c. Trade — `tradeoffer_window` modal (deferred mutation) + filters

| gui element (name + type) | source | fidelity | notes |
|---|---|---|---|
| `tradeoffer_window` (player_flag/country/trade/convoys_description/factories_description/flags_grid/buy_slider/…) | a NEW buy being constructed | **deferred** (mutation) | Build & send a trade offer → inert in replay. `flags_grid` (traded-nations) would bind from `trade_contracts` if shown; the modal isn't shown in replay. |
| `resources_filter_entry` (`icon`)/`resources_grid` (filters) | the static resource set | static-set | Resource filter chips (fixed set, not data). |
| `filter_entry` (`name`)/`country_filters_grid` | country-filter categories | static-set | Filter chips. |
| `sort_country`/`sort_export`/`sort_request`/`sort_convoys` (buttonType) | — | **static** (sort verbs) | Re-sort the (unavailable) grid; UI control. |

## 6d. Trade — STATIC / decoration / navigation (no binding)

- **Chrome / frames / titles:** `Background` (`GFX_tiled_window_1b_border`), `production_header_bg`,
  `trade_title` (`TRADE_TRADE` loc), `trade_back`→`trade_top`/`Background`, `trade_header`,
  `civilian_factory_icon` (×several), `civ_factories` (`TRADE_CIV_FACTORIES` loc), `resources_bg`,
  `trade_icon_and_bg`→`trade_icon_bg`/`trade_icon`, the column title labels (`produced_title`/
  `imported_title`/`exported_title`/`special_project_title`/`production_line_title`/`needed_title` =
  `TRADEVIEW_RES_*` loc), `Background` (`GFX_tiled_window_transparent`), entry bgs
  (`exported_background`/`available_export_background`/`convoys_background`→`GFX_tiled_generic_bg_1`),
  the tradeoffer modal chrome (`player_flag` `GFX_shield_medium`, `large_flag_frame`, `resources_icon`,
  `convoys_icon`, `warning_icon`, `convoys_icon_bg`, `traded_nations_label`, `trade_reset`,
  `min_amount`/`max_amount`).
- **Navigation / mutation buttons (inert):** `close_button` (`CLOSE`), `buy_needed_amount_button`,
  `cancel_imports`, `buy_slider` (extendedScrollbar — amount), `send_button` (`SEND`)/`cancel_button`
  (`CLOSE`) in the modal — mutation verbs.
- **Structural containers / row-template roots:** `countrytradeview`, `trade_back`, `resources`,
  `trade_icon_and_bg`, `filters`, `country_filters`, `trade`, `tradeoffer_window`, `flags`,
  `resources_info_entry`, `resources_filter_entry`, `filter_entry`, `country_trade_entry`
  (+ `exported_background`/`available_export_background`/`convoys_background`).

---
---

# VIEW 7 — PEACE CONFERENCE (`game_assets/gui/peace_conference.json` / `peaceconferencewindow.gui`)

> **Parse note.** `peace_conference.json` is one of the assigned views. The peace-conference state is
> **mostly inactive in saves** — `countries.*.intel.peace_conference` is **`null` for all 143 countries**
> in the demo (and the `.gui` source `peaceconferencewindow.gui` is a large modal). Per the task spec this
> view is handled as a **BADGE**: the entire peace-conference window is **`unavailable` (ARTIFACT for the
> active-state trigger; mostly genuinely inactive)** — a peace conference is a transient, fired-only state
> that is empty in virtually every snapshot.

## 7a. Peace conference — binding posture (badge)

| logical surface (peaceconferencewindow.gui) | intended source | fidelity | notes |
|---|---|---|---|
| **window active / visibility GATE** | `countries.<SEL>.intel.peace_conference.active` | bool→show | **derived** (ARTIFACT) | **Codex add:** the active/visibility gate DOES map to `peace_conference.active` (§3.8 stores `{active}`). `null` everywhere in the demo → window never shown. This single bind is real; everything below it is gap. |
| participants / agenda / votes / pot / per-state claim grid (all value cells) | `countries.<SEL>.intel.peace_conference` (beyond `active`) + `diplomacy_world` | **unavailable** | The full participant/agenda/vote/claim model is **TRUE GAP (b) COMPUTED-NOT-STORED** — a peace conference is a live, interactive sub-game whose per-tick state (votes, pot, contested states) is runtime-only and not persisted beyond the `active` flag. |
| every action button (take-state / vote / pass / accept) | — | **deferred** (mutation) | The conference is an interactive mutation sub-game → inert in replay. |

> All elements of `peaceconferencewindow.gui` fall into one of: (badge) `unavailable` value cells, or
> (deferred) mutation verbs. No element binds in the demo. The replay surfaces peace OUTCOMES (annexations,
> released nations, war end) via state-owner deltas + the diff timeline, not this window.

---
---

# VIEW 8 — WORLD TENSION POPUP (`game_assets/gui/worldtensionwindowpopup.gui`)

Root: `world_tension_popup` — the **world-tension contributor list** + a wars list. Two runtime ROW
FACTORIES: `world_tension_entry` (one per threat contributor) and `world_tension_war_entry` (one per war).

> **The headline call (task-specified).** The AGGREGATE world-tension SCALAR (the single "World Tension:
> NN%" number) is **NOT stored anywhere** in the save/schema — only the per-action **contributor ledger**
> `world_events.threat[]` (`{threat, final_threat, daily, tag, date, label}`) and the scaling INPUTS
> `world.world_tension_inputs` (`{average_major_ic, tension_scaling_base_country}`) exist (SCHEMA §17/§18).
> So the contributor LIST binds (once `world_events` is fed), but any aggregate-tension BADGE is a
> **TRUE GAP (b) COMPUTED-NOT-STORED**. **Confirmed:** in this demo the `world_events` top-level key is
> **ABSENT** (→ even the contributor list is `unavailable` ARTIFACT here), and `world` is absent too.

## 8a. World tension — `list` → `world_tension_entry` ROW FACTORY (threat contributors)

| `world_tension_entry` element (name + type, sprite) | source (per `world_events.threat[i]`) | formatter | fidelity | notes |
|---|---|---|---|---|
| `country_flag` (iconType, `GFX_flag_small2`) + `country_flag_frame` | `world_events.threat[i].tag` | flag-small | **unavailable** (ARTIFACT) | Contributor country. `world_events` absent in demo. |
| `country_name` (instantTextboxType) | `world_events.threat[i].tag` | loc | **unavailable** (ARTIFACT) | |
| `date` (instantTextboxType) | `world_events.threat[i].date` | date | **unavailable** (ARTIFACT) | When the threat action fired. |
| `value` (instantTextboxType) | `world_events.threat[i].final_threat` (or `threat`/`daily`) | signed-int / percent | **unavailable** (ARTIFACT) | The contributor's tension delta. |
| `reason` (instantTextboxType) | `world_events.threat[i].label` | loc | **unavailable** (ARTIFACT) | Threat-action label. |
| `tracking_icon` (iconType, `GFX_leader_en_route`) | ongoing-contributor marker | category | **unavailable** (ARTIFACT) | |
| `no_surces_info` (instantTextboxType, `NO_THREAT_SOURCES_WINDOW`) | static empty-state | loc | **static-with-data** | Shown when `threat[]` empty — the demo state. |
| **AGGREGATE world-tension VALUE** (not a discrete element; the popup's implied total / the topbar number) | — | — | **unavailable** | **TRUE GAP (b) COMPUTED-NOT-STORED.** Only the contributor ledger + scaling inputs are stored; the aggregate scalar is runtime-computed (Ghidra tier). |

## 8b. World tension — `list` → `world_tension_war_entry` ROW FACTORY (wars list)

| `world_tension_war_entry` element (name + type, sprite) | source (per `diplomacy_world.wars[w]`) | formatter | fidelity | notes |
|---|---|---|---|---|
| `war_name` (instantTextboxType) | `diplomacy_world.wars[w].{a,b}` (+ wargoal) | loc ("a–b war") | **derived** | **Codex tighten:** the belligerent TAGs are exact, but the war NAME string is composed/loc'd from them (and wargoal) → derived, not a stored string. |
| `warscore_progressbar` (iconType, `GFX_warscore_progressbar_small`) | `wars[w].war_score_a_vs_b` / `…b_vs_a` | progressbar | **exact** | War-score bar from the stored scores (the two side scores ARE direct). |
| `attacker_country_flag` / `defender_country_flag` (iconType, `GFX_flag_small2`) + borders | `wars[w].a` / `wars[w].b` | flag-small | **exact** | Belligerent TAGs are direct. |
| `tension_war_list_major` (iconType, `WAR_HAS_MAJOR` tt) | whether war involves a major | category | **derived** | Major-ness derived from belligerent major status (country meta). |

> **Reconciliation note:** §8a (threat contributors) is `unavailable` in THIS demo only because
> `world_events` is unfed (ARTIFACT) — it becomes `exact`/`derived` once folded. §8b (wars list) binds
> from the fed `diplomacy_world.wars`: belligerent TAGs + the two war-scores are **exact**, the war NAME
> and major-marker are **derived** (composed/loc'd, not stored — Codex). The aggregate-tension scalar
> stays `unavailable` (TRUE GAP b) regardless.

## 8c. World tension — STATIC / decoration / navigation (no binding)

- **Chrome / titles:** `Background` (`GFX_world_tension_bg`/`GFX_tiled_plain_bg`/`GFX_current_wars_entry`/
  `GFX_world_tension_entry`), `window_name` (`THREAT_WINDOW_NAME` loc), `margin`.
- **Navigation / sort buttons (read-only verbs):** `close_button` (`DIPLOMACY_CLOSE`), `sort_date_button`/
  `sort_country_button`/`sort_value_button` (re-sort the list), `show_wars_button` (toggle wars list) —
  UI controls.
- **Structural containers / row-template roots:** `world_tension_popup`, `tension_entries_wnd`, `list`
  (gridBox row factory), `world_tension_war_entry`, `world_tension_entry`.

---
---

# VIEW 9 — WAR OVERVIEW (`game_assets/gui/waroverview.gui`)

Root: `waroverview_window` — a selected WAR's two-sided roster (attacker side vs defender side), with
per-side aggregate header stats and per-participant rows. The war is chosen from `waroverview_wars_box`
(tab strip of `waroverview_war_button`). Runtime ROW FACTORIES: `war_faction_entry` (a faction group
header within a side), `war_ally_entry` (a participant in the war), `ally_entry` (a callable-but-not-in
ally). One row per participant (PLAN §C.2 roster aggregation).

> **Fidelity in the demo:** `diplomacy_world.wars` (75) + `factions` (4) + `subjects` (5) are
> **populated**, and each war carries `{a, b, start_date, a_casualties, b_casualties, war_score_a_vs_b,
> war_score_b_vs_a}`; per-country `diplomacy.wargoals[]` is populated. So the war's belligerents, casualty
> totals, war scores, faction grouping, and wargoals bind **exact/derived**. The per-participant
> DIVISIONS / IC / capitulation-% columns are runtime aggregates → `derived`/`unavailable`.

## 9a. War overview — per-side header aggregates

| gui element (name + type) | source | formatter | fidelity | notes |
|---|---|---|---|---|
| `header_desc` (instantTextboxType) | selected `wars[w]` ("the war about X") | loc | **derived** | War title from belligerents/wargoal. |
| `warscore_progressbar` (+ `_frame`/`_disabled`) | `wars[w].war_score_a_vs_b` vs `…b_vs_a` | progressbar | **exact** | Aggregate war-score bar from stored scores. |
| `attacker_fielded_man` / `defender_fielded_man` (instantTextboxType) | sum of `divisions[].strength` over each side's belligerent owners (top-level `divisions[]`) | int | **derived** | **Codex fix:** top-level `divisions[]` (1654, each `{owner, strength, organisation, …}`) IS present → per-side fielded strength is derivable by filtering on the side's TAGs. "Fielded MANPOWER" exactly is a proxy (use `strength`, the manpower-equivalent); no per-division `manpower` field. |
| `attacker_losses` / `defender_losses` (instantTextboxType) | `wars[w].a_casualties` / `b_casualties` | int | **exact** | Casualty totals ARE stored per war. |
| `waroverview_wars_box` (gridboxtype) → `waroverview_war_button` rows | iterate `diplomacy_world.wars` | row factory | **exact** | One tab per war (75). |

## 9b. War overview — `war_ally_entry` / `war_faction_entry` / `ally_entry` ROW FACTORY (participants)

| element (name + type, sprite) | source (per participant in `wars[w]`, grouped via `diplomacy_world.factions`) | formatter | fidelity | notes |
|---|---|---|---|---|
| `war_ally_entry.country_flag` (buttonType, `GFX_flag_small2`) + `country_flag_frame` + `major_country_overlay` | participant TAG | flag-small | **exact** | Belligerent flag; major-overlay from country meta. |
| `war_ally_entry.country_name` (instantTextboxType) | participant TAG | loc | **exact** | |
| `war_ally_entry.warscore` (instantTextboxType) | per-participant war score | int | **unavailable** | **Codex fix:** `wars[w].war_score_a_vs_b`/`b_vs_a` are SIDE-level, NOT a per-participant share → reconstructing a participant's score is **TRUE GAP (b) COMPUTED-NOT-STORED**. Do not present a fabricated split. (The two SIDE scores bind exact at the header, §9a.) |
| `war_ally_entry.wargoal` (iconType, `GFX_wargoal_icon`) | `countries.<participant>.diplomacy.wargoals[]` (`type`,`target`) | category (wargoal→sprite) | **derived** | Wargoals ARE populated; icon via casus-belli/wargoal DEF (STATIC DEF). |
| `war_ally_entry.divisions` (instantTextboxType) | count of `divisions[]` with `owner==participant` (top-level `divisions[]`) | int | **derived** | **Codex fix:** top-level `divisions[]` IS present in the demo → per-participant division COUNT is derivable directly (no OOB needed). |
| `war_ally_entry.ic` (instantTextboxType) | participant industrial capacity | int | **derived** | From `economy` (civ+mil factories) — bindable; the *war-attributed* IC is derived. |
| `war_ally_entry.casualties` (instantTextboxType, "-") | per-participant casualties | int | **unavailable** | **Codex fix:** war stores only SIDE casualties (`a_casualties`/`b_casualties`); per-participant casualties are NOT in `diplomacy_world.wars` → **TRUE GAP (a) LIVE-EXTRACTABLE** (a live per-country casualty read), not merely unfed. Renders "-". |
| `war_ally_entry.surrender_progressbar` / `surrender_flag` | participant capitulation % | progressbar | **unavailable** | TRUE GAP (b): capitulation progress is COMPUTED-NOT-STORED (runtime). |
| `war_faction_entry.faction_name` (instantTextboxType) + `divisions`/`ic`/`casualties` | `diplomacy_world.factions[f].{template,members}` | loc + int | **derived** | Faction group header; name from `factions[f].template`; aggregate stats summed over members (div/IC from economy/OOB, casualties side-level). |
| `ally_entry.country_flag`/`country_name` (callable, not-in-war ally) | `factions[f].members` AND `diplomacy_world.subjects[]` (overlord's subjects) minus current belligerents | flag-small + loc | **derived** | "Callable allies" = faction members **plus callable SUBJECTS** (**Codex add:** include `diplomacy_world.subjects[]` where `<VIEWER>` is overlord, not only faction members) minus current belligerents. |

## 9c. War overview — STATIC / decoration / navigation / mutation (no binding)

- **Chrome / frames / column-header buttons (icons-as-labels):** `waroverview_bg`
  (`GFX_tiled_waroverview_bg`), `attacker_window`/`defender_window`→`Background`
  (`GFX_tiled_window_transparent`), `war_*_entry.Background` (`GFX_war_faction_entry`/
  `GFX_war_participant_entry`/`GFX_not_in_war_ally_entry`), `expanded_icon`, `surrender_progress_frame`,
  the column-header icon buttons `attacker_participants`/`attacker_wargoal`/`attacker_capitulation`/
  `attacker_warscore`/`attacker_divisions`/`attacker_ic`/`attacker_casualty` + the `defender_*` mirror
  (these are loc-tooltip HEADERS for the columns whose VALUE cells are in §9b — header chrome, not data).
- **Navigation / filter / mutation buttons (inert):** `close_button` (`CLOSE`), `filter_minors`/
  `filter_capitulated`/`filter_callable` (filter toggles), `call`/`call_all` (`CALL`/`CALL_ALL` — call
  ally to war → mutation), `waroverview_war_button.button` (select war — read-only verb).
- **Structural containers / row-template roots:** `waroverview_window`, `attacker_window`/
  `defender_window`, `waroverview_wars_box`, `war_faction_entry`, `war_ally_entry`, `ally_entry`,
  `waroverview_war_button`, `attacker_grid`/`defender_grid` (gridbox row factories).

---
---

# VIEW 10 — INSIGNIA SELECTION (`game_assets/gui/insigniaselection.gui`)

Root: `insignia_selection_window` — a **picker modal** (choose an army/fleet insignia + colour for a
formation): `insignia_list` grid of `insignia_list_entry`, an HSV colour picker
(`color_selection_container`: hue/sat/val sliders + `presets` grid of `color_picker_preset_entry2`), a
group name editbox, and a fleet-colour checkbox.

> **Nature:** this is a **cosmetic formation-editor MUTATION modal** (PLAN §D). It edits a unit/fleet's
> insignia + colour — a property of a formation, NOT a per-snapshot datum, and a mutation. In a replay it
> is never shown. The insignia *catalog* it lists is **STATIC DEF / WORLD DATA (c)** — parseable from the
> game's insignia/emblem GFX. No snapshot field drives it.

## 10a. Insignia selection — binding posture

| gui element (name + type, sprite) | intended source | fidelity | notes |
|---|---|---|---|
| `insignia_list.item_grid` (gridBoxType) → `insignia_list_entry.button` (`GFX_theatre_army_shield`) rows | the insignia CATALOG | **deferred / STATIC DEF (c)** | Catalog of selectable shields — STATIC game data, not a snapshot datum; the picker itself is a mutation. |
| `group_icon` (iconType, `GFX_theatre_army_shield`) / `group_name` (editBoxType) | the formation being edited — its CURRENT insignia + name, from `order_of_battle.{army_groups,armies,fleets}[].name`/insignia | category + text | **derived** (initial value) / **deferred** (edit) | **Codex refine:** the picker's INITIAL value (current insignia + group name) WOULD bind from the selected formation's `order_of_battle.*` entry — but `order_of_battle` is ABSENT in this demo (ARTIFACT) → unavailable here; the EDIT action is a mutation (inert). |
| `presets` (gridBoxType) → `color_picker_preset_entry2.button` rows; `hue_slider`/`sat_slider`/`val_slider` | colour selection | **deferred** (mutation) | HSV colour edit. |
| `fleet_color_checkbox` (checkBoxType) + `fleet_color_label` (`USE_FLEET_COLOR`) | use-fleet-colour toggle | **deferred** (mutation) | |

## 10b. Insignia selection — STATIC / structural

- **Chrome / titles:** `Background` (`GFX_army_insignia_selection_bg`/`GFX_group_name_bg`/`GFX_parch_bg`),
  `header_text` (`CHOOSE_INSIGNIA` loc), the slider backgrounds (`hue_slider_background`/
  `sat_slider_background`/`val_slider_background`), every `guiButtonType` slider part
  (`listboxSliderButton`/`listboxTrackButton`/`upButton`/`downButton`), `borderSize` (×4).
- **Navigation / mutation buttons (inert):** `btn_close` (`DESELECT_UNIT`), `insignia_list_entry.button`,
  `color_picker_preset_entry2.button`.
- **Structural containers / row-template roots:** `insignia_selection_window`, `group_detail`,
  `insignia_list`, `color_selection_container`, `insignia_list_entry`, `color_picker_preset_entry2`.

---
---

# VIEW 11 — INS REVOLUTION (`game_assets/gui/insrevolutionwindow.gui`)

Root: `INS_revolution_ui_window` — a small **mod-specific** strip (`INS_` = Indonesia / a mod tag) showing
a "war escalation level" indicator: one label + 10 escalation-level icons (`lon_we_1_icon` …
`lon_we_10_icon`, sprites `GFX_war_escalation_level_1..10`, each `@`-positioned).

> **Nature:** this is a **scripted-mod state indicator** driven by a country/global **script variable**
> (`war_escalation` / `INS_war_escalation`-style). It is NOT a vanilla domain in SCHEMA §3.8/§7b/§17. The
> level value lives (if anywhere) in `world.variables` / a country variable / a dynamic modifier — none of
> which are in this demo's top-level keys (`world` absent), and `extra.dynamic_modifiers` is not on these
> country objects in the demo.

## 11a. Ins revolution — binding posture

| gui element (name + type, sprite/loc) | intended source | formatter | fidelity | notes |
|---|---|---|---|---|
| `INS_we_level_text` (instantTextBoxType, `INS_war_escalation_text`, tt `INS_war_escalation_level_tt`) | the war-escalation level variable | loc + int | **unavailable** (ARTIFACT) | Mod script variable. Would bind from `world.variables.<name>` or a country variable / `extra.dynamic_modifiers` once those are folded — LIVE-EXTRACTABLE (a) for the live value; not in this demo. |
| `lon_we_1_icon` … `lon_we_10_icon` (iconType, `GFX_war_escalation_level_1..10`, tt `INS_war_escalation_N_tt`) | current escalation level → which icon is lit | int→active-frame | **unavailable** (ARTIFACT) | The 10 icons are a static scale; the *active* one is selected by the level variable (unavailable in demo). |

## 11b. Ins revolution — STATIC / structural

- The 10 `lon_we_*_icon` sprites are a **fixed decorative scale** (chrome); only which one is highlighted
  is data-driven (and unavailable). Structural: `INS_revolution_ui_window`.

---
---

## Element accounting (provably complete)

Each table's element total is reconciled against the parsed JSON's own `stats.element_type_counts`
(sum of all type counts = total nodes in the tree, INCLUDING structural/positional helper nodes like
`slotsize`/`margin`/`borderSize`/`positionType`/`max_slots`/`background_margin`/`tileSize` and scrollbar
parts). The bucket columns assign every NAMED element; the helper nodes fall under static/structural.

### VIEW 1 — intel_agency.json (sum = 233)
counts: containerWindowType 40, background 16, iconType 51, instantTextboxType 41, borderSize 4,
buttonType 25, margin 9, gridBoxType 4, slotsize 7, gridboxtype 3, editBoxType 1, positionType 1,
OverlappingElementsBoxType 1, extendedScrollbarType 1, slider 1, track 1, decreaseButton 1, increaseButton 1.

| bucket | count (logical) | examples |
|---|---:|---|
| **Dynamic-bound** (would-bind, ~all `unavailable` ARTIFACT in demo) | **~26** | agency name/logo/defense, operative counters (3, partly derived), branch/upgrade rows, operation rows (name/icon/flag/days/tokens), cryptology rows (flag/name/%/done) |
| **`unavailable`-badged** | **~24** | agency block (10), operations entry (10), cryptology (6) — overlaps the above; spymaster (TRUE GAP a) |
| **Deferred: mutation** | **~14 buttons** | create/cancel agency, spymaster/faction, upgrade/cancel, launch op, start/stop/activate/delete decrypt, insignia create/rename |
| **Static / decoration / navigation / structural** | **~155** | all bgs/frames/titles, tab/nav buttons, scrollbar, ~40 container roots + slotsize/margin/border helpers |

### VIEW 2 — intel_ledger.json (sum = 351)
counts: containerWindowType 80, background 33, iconType 96, instantTextboxType 60, buttonType 12,
margin 5, gridBoxType 18, slotsize 16, instantTextBoxType 7, ButtonType 3, positionType 4,
background_margin 8, borderSize 3.

| bucket | count | examples |
|---|---:|---|
| **`unavailable`-badged** (intel ESTIMATES → TRUE GAP b) | **~55** | every economy/army/navy/air estimate cell + estimate-grid row factories (resource/idea/tech/template/stockpile/ship/airwing/plane) + line chart |
| **Dynamic-bound** (header) | **~5** | `country_info` header (derived), 4× tab `intel` name labels (exact) |
| **Static / decoration / navigation / structural** | **~291** | all bgs/icons/`NO_INTEL` placeholders/titles, 4 tab buttons + tab frames + chart filters, ~80 containers + helpers |

### VIEW 3 — operative.json (sum = 11)
counts: containerWindowType 2, background 2, instantTextboxType 2, margin 1, gridboxtype 1, slotsize 1, buttonType 2.

| bucket | count | examples |
|---|---:|---|
| **Deferred: mutation modal** (whole view) | **~6** | title/description/additionals_grid/send/cancel — boost-party mission dialog |
| **Static / structural** | **~5** | Background ×2, margin, 2 container roots, slotsize |

### VIEW 4 — operative_leader.json (sum = 92)
counts: containerWindowType 13, buttonType 17, gridBoxType 3, slotsize 3, iconType 41,
instantTextboxType 4, background 3, ButtonType 1, editBoxType 1, instantTextBoxType 3, positionType 3.

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** | **~14** | portrait-window (skill exact; gfx/captured/enroute unavailable; state-driven mission/operation icons derived), badge (name/skill exact, title/status/traits/nationalities, eta), counters |
| **`unavailable`-badged** | **~7** | portrait gfx, captured, enroute, agency_insignia, eta, code_name, activity progress fill (all ARTIFACT) |
| **Deferred: mutation** | **~12** | 8 mission buttons + dismiss/recruit/deselect/scroll |
| **Static / decoration / structural** | **~59** | frames/bgs/anim decals, ~13 containers + portrait-frame icons + positions |

### VIEW 5 — operation_overview.json (sum = 97)
counts: containerWindowType 25, background 12, instantTextboxType 13, buttonType 7, iconType 29,
gridBoxType 4, slotsize 4, checkBoxType 2, positionType 1.

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** (would-bind, all `unavailable` ARTIFACT in demo) | **~14** | name/icon/target flag+name, progress/duration, resources rows, operatives rows, phase-titles + phases rows, outcome/reward; `outlook` = TRUE GAP b |
| **Deferred: mutation** | **~6** | start/collect/refund + 2 checkboxes + close/location |
| **Static / decoration / structural** | **~77** | clipboard bgs/frames/anchors, ~25 containers + 5 row-template roots + helpers |

### VIEW 6 — trade.json (sum = 95)
counts: containerWindowType 16, background 14, iconType 14, instantTextboxType 32, borderSize 1,
gridBoxType 5, slotsize 5, max_slots 1, buttonType 12, margin 1, extendedScrollbarType 1, tileSize 1,
slider 1, track 1, decreaseButton 1, increaseButton 1.

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** | **~13** | resources_info_entry (balance/produced derived; imported/exported/needed unavailable), factory totals (derived), trade_grid country rows (unavailable ARTIFACT) |
| **`unavailable`-badged** | **~8** | trade contract rows + imported/exported/special_project/production_line legs (market unfed) |
| **Deferred: mutation** (tradeoffer modal + sorts) | **~10** | tradeoffer window + send/cancel/buy_slider/buy_needed/cancel_imports; 4 sort verbs |
| **Static / decoration / structural** | **~64** | bgs/titles/`TRADEVIEW_RES_*` labels, filters, modal chrome, ~16 containers + 4 row-template roots + scrollbar parts |

### VIEW 7 — peace_conference.json (badge — peaceconferencewindow.gui)
| bucket | count | examples |
|---|---:|---|
| **`unavailable` (badge)** | whole view | `peace_conference=null` everywhere; only `{active}` stored (TRUE GAP b for the full model) |
| **Deferred: mutation** | all action buttons | the conference is an interactive mutation sub-game |

> The parsed `peace_conference.json` was not opened element-by-element here because the demo provides no
> binding surface (the domain is `null` across all 143 countries) and the task explicitly scopes this view
> to a BADGE. Every element resolves to (badge) `unavailable` or (deferred) mutation. Flagged for a
> follow-up element-level pass IF a save with an ACTIVE conference is ever captured.

### VIEW 8 — world_tension_popup.json (sum = 32)
counts: containerWindowType 4, background 4, instantTextboxType 7, buttonType 5, margin 1, gridBoxType 1,
slotsize 1, iconType 9.

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** | **~11** | `world_tension_entry` (flag/name/date/value/reason — unavailable ARTIFACT in demo), `world_tension_war_entry` (war_name/warscore/flags/major — EXACT) |
| **`unavailable`-badged** | **~6** | threat-contributor cells (ARTIFACT) + the AGGREGATE tension scalar (TRUE GAP b) |
| **Static / decoration / navigation** | **~15** | bgs/title, close + 4 sort/show buttons, 4 containers + row-template roots + helpers |

### VIEW 9 — war_overview.json (sum = 74)
counts: containerWindowType 7, Background 3, instantTextboxType 15, iconType 14, gridboxtype 3, slotsize 3,
buttonType 25, margin 2, background 2.

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** | **~17** | header (warscore exact, losses exact, fielded_man unavailable, header_desc derived), war tabs (exact), participant rows (flag/name/warscore/wargoal/divisions/ic exact-derived; casualties/surrender unavailable), faction + callable-ally rows |
| **`unavailable`-badged** | **~5** | per-participant casualties, surrender/capitulation % (TRUE GAP b), fielded manpower, divisions (OOB unfed in demo) |
| **Deferred: mutation** | **~5** | call/call_all, filter ×3 |
| **Static / decoration / structural** | **~47** | bg/frames, 12 column-header icon buttons (chrome), close, ~7 containers + 4 row-template roots + helpers |

### VIEW 10 — insignia_selection.json (sum = 47)
counts: containerWindowType 6, buttonType 3, background 3, instantTextboxType 2, borderSize 4, iconType 4,
editBoxType 1, margin 1, gridBoxType 2, slotsize 2, scrollbarType 3, guiButtonType 12, checkBoxType 1.

| bucket | count | examples |
|---|---:|---|
| **Deferred: mutation modal + STATIC DEF catalog** | **~6** | insignia catalog grid (STATIC DEF c), colour picker (hue/sat/val + presets), group name edit, fleet-colour checkbox — all mutation |
| **Static / decoration / structural** | **~41** | bgs/title/slider backgrounds, 12 guiButton slider parts, borders, 6 containers + 2 row-template roots |

### VIEW 11 — ins_revolution.json (sum = 12)
counts: containerWindowType 1, instantTextBoxType 1, iconType 10.

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** (would-bind, `unavailable` ARTIFACT in demo) | **~11** | level text + 10 escalation-level icons (active-frame selected by the mod variable) |
| **Static / structural** | **1** | `INS_revolution_ui_window` root |

---

## Consolidated GAP LEDGER (ARTIFACT vs TRUE GAP + closure bucket)

| # | gap (view · element group) | ARTIFACT or TRUE GAP | closure bucket | note |
|---|---|---|---|---|
| 1 | Intel agency block: name/logo/defense/branches/upgrades/progress (V1) | **ARTIFACT** | fold `intel.agency` | `agency=null` across all 143 in demo; §3.8 defines it |
| 2 | Operations tab + operation overview (V1, V5): names/flags/days/phases/resources/operatives | **ARTIFACT** | fold `intel.operations` | `operations=null` everywhere; join needs `operatives[].op_id` (also unemitted) |
| 3 | Cryptology tab (V1): targets/%/strength | **ARTIFACT** | fold `intel.decryption` | `decryption=null` everywhere |
| 4 | Operative portrait `gfx` (V4) | **ARTIFACT** | re-emit operative `gfx` | sprite NAME; demo omits it → `GFX_operative_unknown` |
| 5 | Operative `captured`/`operation_id`/`mission`/`op_id` (V4) | **ARTIFACT** | re-emit operative fields | demo key union is 6 fields only; ETA / mission detail / captured depend on these |
| 6 | Intel ledger ALL estimate cells (V2) | **TRUE GAP** | **(b) COMPUTED-NOT-STORED** | the intel-level min–max ESTIMATE band is runtime; binding the real value would leak perfect info |
| 7 | RESOURCE-trade roster + import/export/convoy legs (V6) | **TRUE GAP** (Codex re-class) | **(a) LIVE-EXTRACTABLE** / new parse domain | **Codex:** `countrytradeview` is RESOURCE trade, not the equipment market; **no resource-trade ledger in SCHEMA** → NOT the unfed `trade_contracts` ARTIFACT |
| 8 | Resource produced/balance (V6) | **derived** | resource adapter over `states[*].resources` | raw deposits exist; extraction/demand math = derived; import leg = gap (row 7) |
| 9 | Peace conference full model (V7) | **TRUE GAP** (active trigger ARTIFACT) | **(b) COMPUTED-NOT-STORED** | only `{active}` stored; votes/pot/claims runtime; `peace_conference=null` in demo |
| 10 | World-tension contributor list (V8 §8a) | **ARTIFACT** | fold `world_events.threat[]` | `world_events` top-level key ABSENT in demo |
| 11 | **AGGREGATE world-tension SCALAR** (V8) | **TRUE GAP** | **(b) COMPUTED-NOT-STORED** | only `threat[]` ledger + `world.world_tension_inputs` stored; the scalar is runtime (Ghidra tier) — **task-confirmed** |
| 12 | War overview per-participant **war-score** + **capitulation %** (V9) | **TRUE GAP** | **(b) COMPUTED-NOT-STORED** | `war_score_*` are SIDE-level not per-participant (Codex); capitulation % is runtime |
| 12b | War overview per-participant **casualties** (V9) | **TRUE GAP** (Codex) | **(a) LIVE-EXTRACTABLE** | war stores only SIDE casualties (`a_/b_casualties`); per-country needs a live read |
| 13 | War overview divisions/fielded-strength per participant (V9) | **derived** (Codex fix) | from top-level `divisions[]` (`owner`/`strength`) | `divisions[]` IS present → derivable now; "fielded manpower" exactly = proxy via `strength` |
| 13b | Operative `code_name` (V4) | **TRUE GAP** (Codex) | **(a) LIVE-EXTRACTABLE** | not in §3.8; needs a new field (distinct from `operatives[].name`) |
| 14 | Insignia catalog (V10) | **TRUE GAP** (also a mutation modal) | **(c) STATIC DEF / WORLD DATA** | parse insignia/emblem GFX; the picker itself is inert |
| 15 | Operation/operative-trait/idea/tech/resource/wargoal SPRITES & NAMES (V1/V2/V4/V5/V9) | **TRUE GAP** | **(c) STATIC DEF / WORLD DATA** | icon+loc come from game `common/`+`interface/` DEFs (a DEF-catalog adapter) |
| 16 | Opinion-modifier breakdown names (diplomacy card; not a value cell in these 11 views) | **TRUE GAP** | **(c) STATIC DEF** for names | **Codex clarify:** the opinion TOTAL (`diplomacy.opinion.<other>.opinion_sum`) is **exact/populated** — only the per-modifier NAMES (DEF) + reconstructing the math from modifiers is a gap |
| 17 | Spymaster identity (V1) | **TRUE GAP** | **(a) LIVE-EXTRACTABLE** | which faction member is spymaster not in schema |
| 18 | INS war-escalation level (V11) | **ARTIFACT** | fold `world.variables` / country var / `extra.dynamic_modifiers` | mod script variable; `world` absent in demo → **(a) LIVE-EXTRACTABLE** for the live value |
| 19 | AI operation score (V1 `operation_ai_score`) | **TRUE GAP** | **(b) COMPUTED-NOT-STORED** | AI-internal desirability |
| 20 | Operation outlook / success chance (V5) | **TRUE GAP** | **(b) COMPUTED-NOT-STORED** | runtime probability |

> **`unavailable` ≠ "missing forever".** The single badge folds two very different prognoses, kept
> distinct above: **ARTIFACT** rows (1–5, 7–8, 10, 13, 18) become `exact`/`derived` the moment the writer
> folds the already-defined domain into the live snapshot (a RECONCILE, not a wall); **TRUE GAP** rows
> (6, 9, 11, 12, 14–17, 19–20) need real new work — (a) a live read, (b) a Ghidra-tier formula, or (c) a
> game-DEF catalog parser. Both render "—" per no-fabrication; neither is faked.

### What IS populated & bindable in the demo (the green list)
`countries.*.intel.operatives[]` (name/skill/experience/traits/state/nationalities — V4 binds **exact**);
`countries.*.diplomacy.{opinion(rich, incl. `opinion_sum` total exact), wargoals}` (V9 wargoals derived);
`diplomacy_world.{wars(75), factions(4), subjects(5)}` — wars carry **SIDE-level** casualties
(`a_/b_casualties`) + the two war-scores (V8 §8b wars list belligerents+scores **exact**; war NAME/major
= derived; V9 header losses/warscore **exact**, per-PARTICIPANT score/casualties = gap); plus the
**top-level `divisions[]` (1654, with `owner`/`strength`)** → per-side/per-participant division COUNT and
fielded STRENGTH are **derivable** (V9 §9a/§9b — Codex). Everything else in these 11 views is
ARTIFACT-unfed or a TRUE GAP per the ledger.

---

## Historical review round (adversarial review, 2026-06-26)

Historical note: this records a completed adversarial review from the old
Claude-era workflow. Codex-native sessions must use normal sub-agents/partner
review and must not run the old local CLI command. The review was given the full
demo-snapshot facts, the relevant SCHEMA, and every per-view binding decision,
and asked the 4 questions: (a) missed elements, (b) field-path correctness, (c) false
`unavailable` vs real gap, (d) artifact-vs-gap classification + the world-tension-aggregate claim.

**ASKED / AGREED:**
- **(d) world-tension aggregate claim — AGREED, confirmed.** "The aggregate world-tension scalar claim is
  correct against the supplied schema: no stored `world.world_tension` value; only `world_events.threat[]`
  contributor ledger plus `world.world_tension_inputs`." (If the engine formula is later validated it
  *could* become `derived` from `sum(world_events.threat[].final_threat)` + clamp/round, but it is **not
  stored** — stays TRUE GAP (b).)
- **(d) classification — AGREED with caveat:** "Mostly sound, but overused ARTIFACT where the schema path
  is wrong." → drove the (c) re-classifications below. ARTIFACT validly applies to `intel.agency`/
  `operations`/`decryption`, `world_events.threat[]`, `world.variables`; NOT to resource trade, operative
  `code_name`, per-participant war score/casualties, or the full peace model.

**REFUTED / FIXED (folded in):**
1. **`country_trade_entry` → `trade_contracts` was WRONG.** `countrytradeview.gui` is the RESOURCE-trade
   screen, not the equipment market; there is no resource-trade ledger in SCHEMA. Re-classed the
   trade-roster + import/export/convoy legs from **ARTIFACT → TRUE GAP (a) LIVE-EXTRACTABLE** (View 6 §6a/
   §6b, ledger row 7). Kept `produced` derived from `states[*].resources`.
2. **`war_ally_entry.warscore` was over-stated `derived`.** Side-level `war_score_*` is not a participant
   share → flipped to **TRUE GAP (b) COMPUTED-NOT-STORED** (§9b, ledger row 12).
3. **`war_ally_entry.casualties`** re-classed unavailable → **TRUE GAP (a) LIVE-EXTRACTABLE** (war stores
   only SIDE casualties) (§9b, ledger row 12b).
4. **`attacker_fielded_man`/`defender_fielded_man` and `war_ally_entry.divisions` were WRONGLY
   `unavailable`/OOB-dependent.** Top-level `divisions[]` (1654, each `{owner, strength, …}`) IS present →
   flipped to **derived** from `divisions[]` (verified by walking the snapshot) (§9a/§9b, ledger row 13).
5. **`operative_leader.captured`** flipped **unavailable → derived** (derive `false` from `state` in the
   demo; binds the flag once emitted) (§4a).
6. **`operative_code_name`** re-classed ARTIFACT → **TRUE GAP (a)** (not in §3.8) (§4b, ledger row 13b).
7. **`ally_entry` callable allies** now include **`diplomacy_world.subjects[]`** (callable subjects), not
   only faction members (§9b, Codex add).
8. **`peace_conference.active` gate** added as a real **derived** bind (the active/visibility gate maps to
   `intel.peace_conference.active`); the rest of the model stays TRUE GAP (b) (§7a).
9. **`network_strength_text`** now cites **§15 `strategic_operatives.pools`** explicitly (§1b).
10. **`world_tension_war_entry.war_name`/major-marker** tightened **exact → derived** (composed/loc'd, not
    stored) (§8b).
11. **`opinion_sum`** clarified **exact/populated** — only the per-modifier names (DEF) + modifier math is
    a gap (ledger row 16).
12. **`insignia_selection` initial values** (current insignia + group name) noted as **derived** from
    `order_of_battle.*` (absent in demo) rather than purely deferred-mutation (§10a).
13. **`total_operative_text` math** corrected to `len(operatives) / capacity.max_operatives` (current/limit,
    not a sum) (§1a).

**NOT CHANGED (Codex point addressed but my path was already right):**
- `extra.dynamic_modifiers` (View 11) — Codex flagged it as "not in the supplied schema", but it IS
  defined in SCHEMA **§3.11** (verified by reading the schema); Codex simply wasn't given §3.11 in the
  prompt. Path retained; classification (ARTIFACT/LIVE-EXTRACTABLE for the INS variable) unchanged.
- `agency_insignia` already sourced from `<owner>.intel.agency.icon` (not operative row data) — matches
  Codex's correction; no change needed.

No timeout (single call, exit 0). The intel-ledger ESTIMATE-band TRUE GAP (b) call and the operative-roster
exact binds were not challenged by Codex.
