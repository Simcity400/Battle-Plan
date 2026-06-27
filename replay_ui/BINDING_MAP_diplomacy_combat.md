# BINDING MAP — DIPLOMACY + COMBAT (land / naval / combat-log)

> **What this is.** The COMPLETE, no-truncation data-binding map for four in-game the game views the
> native replay UI binds in the Phase-3 sweep: **diplomacy** (`game_assets/gui/diplomacy.json`, from
> `interface/countrydiplomacyview.gui`), **land combat** (`land_combat.json`, from
> `interface/landcombat.gui`), **naval combat** (`naval_combat.json`, from `interface/navalcombat.gui`),
> and the **combat log** (`combat_log.json`, from `interface/combatlogview.gui`). It realises
> GAME_UI_REPLICA_PLAN §C ("comprehensive but mechanical binding sweep") + §C.2 (combat = **roster
> aggregation**, not flat element lists) + §G (HARD LIMITS — combat battle-detail). EVERY element in
> each `.gui` tree is accounted for, either as a dynamic (data-driven) binding, static/decoration with
> no binding, or deferred-needs-adapter. Nothing is omitted; where a `.gui` element exists but our
> snapshot has no usable field, the element is listed and **badged `unavailable`** with the missing
> field named.
>
> **Binding-rule shape** (PLAN §C.1): `{ target (gui element name+type), source (snapshot path),
> format (formatter id), fidelity (exact|derived|proxy|unavailable) }`. **Source paths** use SCHEMA
> dotted form: `countries.<TAG>.…`, `combats[]`, `diplomacy_world.…`. `<SEL>` = the selected/observed
> country (the one whose diplomacy card is open); `<VIEWER>` = the player/observed root country.
>
> **No-fabrication (SCHEMA §0.2, PLAN principle 3):** `null` renders "—", never `0`. `0`/`0.0` is a
> measured zero. A schema-present field the current parser never feeds is badged `unavailable` (not
> silently dropped). A runtime-computed value we don't model is `derived`/`proxy`/`unavailable` by
> nature.

---

## Provenance of the fidelity calls

Grounded in **empirical inspection of a real save+memory-backed snapshot**
(`replay/snapshots/replay/000017005619.snapshot.json`, `schema_version "1"`, date `1941.4.13.11`, GER
1941 channel) cross-checked against `replay/SCHEMA.md` / `replay/schema.py`, and **adversarially
reviewed by Codex** (read-only, 2026-06-26). Codex **confirmed** the diplomacy opinion/trade source
paths and the combat battle-detail `unavailable` badging, and **corrected two calls** (both folded in):
(A) the attacker/defender **division rosters** and per-division stat rows are `unavailable` — only the
**leader ref pairs** are exact/derived (no division-join exists in `combats[]`); and (B) the
opinion **direction** — `our_opinion = countries.<VIEWER>.diplomacy.opinion.<SEL>.opinion_sum` (the
owning country's dict holds **its** opinion of the keyed target), not the reciprocal.

**Empirically confirmed in the current snapshot (drives the badges):**

| schema field | state in the inspected snapshot | consequence |
|---|---|---|
| `combats[]` | **13 entries, all `kind:"land"`, 0 naval** | naval-combat roster has no live battle to bind; land roster binds leader refs only |
| `combats[].combat_width` | **`null`** | combat-width readouts → `unavailable` |
| `combats[].terrain` / `.progress` / `.modifiers` | present (`"hills"`, `14.55…`, `{day,duration}`) **but domain is `M*`-uncracked (SCHEMA §7)**; `progress` is a large positive float, **NOT** the `-1..1` GUI bar | terrain/progress-bar/tactics/modifiers → `unavailable`; only `modifiers.duration` → `derived` (§G allows duration) |
| `combats[].attackers` / `.defenders` | `[[unit_leader_id, 4713], …]` — **commander leader refs, not division ids** | flags/leaders → exact/derived; **committed/reserve division lists + per-division stat rows → `unavailable`** |
| `order_of_battle` (top-level) | **key absent** in this snapshot | naval per-ship rows (`order_of_battle.ships`) → `unavailable` (current data) |
| `market_live` (top-level) / `diplomacy_world.trade_contracts` | **both absent** in this snapshot | diplomacy trade grid → `unavailable` (current data) |
| `countries.<TAG>.diplomacy.opinion` | **rich dict** `{TGT:{attitude, opinion_sum, opinion_modifiers[], guarantee, non_aggression_pact, military_access, embargo, at_war}}` | opinion values + relation flags → exact; per-target modifier rows = numeric `token_id`+`value` only (names `unavailable`) |
| `countries.<TAG>.diplomacy.opinion_modifiers` (live-only leaf) | **`null`** in this snapshot | use the nested `opinion[TGT].opinion_modifiers` instead |
| `diplomacy_world.{wars,factions,subjects}` | **populated** (75 wars, 4 factions, 5 subjects) | war/faction/subject rosters → exact |

**Confirmed PRESENT & usable:** `politics.{stability, war_support, ruling_party, party_popularities,
faction}`, `focus.{current, progress, completed}`, `diplomacy.{opinion, wars, guarantees,
non_aggression, embargoes, lend_lease(per-country), volunteers, attaches, expeditionary}`,
`combats[].{combat_id, kind, location_province, attackers, defenders, modifiers.duration}`,
`diplomacy_world.{wars, factions, subjects}`.

> **Read-only verb note (PLAN §D).** Every `*_button` whose in-game role is to OPEN a diplomatic
> action / disengage / edit a template is a **mutation** verb. In a replay these render as real chrome
> but are **inert** (no datum, no action). They are listed under each view's static/deferred section,
> never bound.

---
---

# VIEW 1 — DIPLOMACY (`game_assets/gui/diplomacy.json`)

Root: `containerWindowType "countrydiplomacyview"`. Two top-level surfaces: the **country card**
(`country_info`, shown when a country is selected) and the **country list** (`country_list`, the
browse/filter grid). The remaining ~17 top-level `containerWindowType`s (`declare_war`, `add_wargoal`,
`stage_coup`, `guarantee`, `naval_blockade`, `send_attache`, `lend_lease`, `incoming_lend_lease`,
`boost_party_popularity`, `call_allies`, `join_allies`, `send_volunteers`, `send_expeditionary_force`,
`request_expeditionary_forces`, `ask_for_state_control`, `give_state_control`,
`request_licensed_production`, + the `standard_diplomacy_controller` / `invite_faction_diplomacy_controller`
reply shells) are **diplomatic-action MODAL dialogs** = mutation verbs → entirely **deferred/inert** in a
read-only replay (§1d).

Formatters: `flag`/`flag-small`/`flag-shield`, `signed-int` (opinion `+50`/`-100`), `percent` (×100),
`category` (ideology/faction/party token→loc+sprite), `pie-chart` (party_popularities→arc fractions),
`progressbar-fraction` (focus progress 0..1), `loc` (localisation key→text), `relation-icon`
(relation token→`GFX_relation_*` frame).

## 1a. Diplomacy — country card: dynamic (data-bound)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `diplo_country_flag` (iconType, `GFX_shield_medium`) | `<SEL>` + `countries.<SEL>.politics.ruling_party` | flag-shield (ideology-tint) | **exact** | Selected country's shield, ideology-tinted by `ruling_party`. |
| `ideology_icon` (iconType, `GFX_ideology_unknown`) | `countries.<SEL>.politics.ruling_party` | category (ideology→sprite) | **exact** | Ruling-ideology emblem. |
| `country_name` (instantTextboxType, `"Germanian"`) | `<SEL>` + country-meta name | loc | **exact** | Display name from country meta keyed by TAG (cosmetic tag applied if `extra.cosmetic_tag` set; `extra` absent here → base name). |
| `faction_name` (instantTextboxType, in `faction`) | `countries.<SEL>.politics.faction.name` | category (faction→loc) | **exact** | `faction.name` (`"faction_template_axis"`); `null`→"—" (no faction). |
| `leader_name` (instantTextboxType, `"Rudolf Kittler"`) | `countries.<SEL>.commanders.leader.name` | loc | **unavailable** (current data) | SCHEMA §3.9 `commanders.leader.name`; **`commanders` domain absent** in this snapshot (country has only economy/military/politics/research/focus/decisions/diplomacy/intel) → "—". |
| `our_opinion_value` (instantTextboxType, `"+50"`) | `countries.<VIEWER>.diplomacy.opinion.<SEL>.opinion_sum` | signed-int | **exact** | **Codex direction fix:** the OWNING country's opinion dict holds ITS opinion of the keyed target → "our" (viewer→selected) = `<VIEWER>.opinion[<SEL>]`. Cached net sum (`opinion_sum`); do NOT recompute by summing modifiers. |
| `our_opinion_left_flag` (iconType, `GFX_flag_small`) | `<VIEWER>` | flag-small | **exact** | Viewer's flag on the opinion widget. |
| `their_opinion_value` (instantTextboxType, `-100`) | `countries.<SEL>.diplomacy.opinion.<VIEWER>.opinion_sum` | signed-int | **exact** | Reciprocal: selected→viewer. |
| `their_opinion_left_flag` (iconType, `GFX_flag_small`) | `<SEL>` | flag-small | **exact** | |
| `stability_value` (instantTextboxType, `"100%"`) | `countries.<SEL>.politics.stability` | percent (×100) | **exact** | 0..1 float. |
| `war_support_value` (instantTextboxType, `"100%"`) | `countries.<SEL>.politics.war_support` | percent (×100) | **exact** | 0..1 float. |
| `chart` (iconType, `GFX_political_chart_big`, in `political_pie_chart`) | `countries.<SEL>.politics.party_popularities` | pie-chart (ideology→arc) | **exact** | `{ideology: fraction}` → pie arcs. |
| `party_name` (instantTextboxType, in `ruling_party_info`) | `countries.<SEL>.politics.ruling_party` (+ party meta) | category/loc | **exact** | Ruling party display name. |
| `ideology_name` (instantTextboxType) | `countries.<SEL>.politics.ruling_party` | category (ideology→loc) | **exact** | |
| `elections` (instantTextboxType) | next-election date | loc/date | **unavailable** | No election-date field in schema (`politics` has no `election_date`). "—". |
| `goal_icon` (iconType, `GFX_goal_unknown`, in `active_national_focus_info`) | `countries.<SEL>.focus.current` | category (focus→sprite) | **exact** | Current focus icon; `null`→generic/"—". |
| `national_focus_label` (instantTextboxType, `POLITICS_ACTIVE_GOAL`) | `countries.<SEL>.focus.current` | loc (focus id→name) | **exact** | |
| `progress` (iconType, `GFX_activegoal_progress`) | `countries.<SEL>.focus.progress` | progressbar-fraction | **derived** | Focus progress; `progress` is raw points (S/M\*) → fraction needs the focus's total cost (not stored per-focus) → derived/approximate. |
| `relations_grid` (gridboxtype, in `relations_info`) → `relation_strip_view` / `subject_relation_strip_view` rows | `countries.<SEL>.diplomacy.{guarantees,non_aggression,embargoes}` + `diplomacy_world.subjects` (where `<SEL>` is overlord/subject) | row factory; `relation_icon`→relation-icon, `relation_flags`→flag-small | **exact** | Per §C.2 roster aggregation: one row per active relation. Relation token → `GFX_relation_*` frame; counterpart TAGs → flags. Subject rows from `diplomacy_world.subjects`. |
| `our_opinion_icon` / `their_opinion_icon` (iconType, `GFX_opinion_arrow_*`, in country-list `filters`) | direction markers | static-with-data | **exact** | Arrow direction is static; flags beside them (`opinion_flag1/2`) bind to the two compared TAGs. |
| `opinion_flag1` / `opinion_flag2` (buttonType, `GFX_flag_small`, in `filters`) | `<VIEWER>` / `<SEL or hovered>` | flag-small | **exact** | The two flags the opinion arrows compare. |

### Country-LIST grid (browse panel)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `countries_grid` (gridboxtype, in `country_list`→`countries`) → per-country rows | iterate `countries.*` (filtered) | row factory; flag + name + our/their opinion | **exact** | One row per country. Each row's opinion pair uses the same `opinion_sum` paths as the card. Row template instantiated at runtime (not in the `.gui`). |
| `filters_grid` (gridboxtype, in `top_filter_window`) → `filter_header_entry` rows | filter categories (static set) | row factory; `name`→loc | static-set | Filter buttons are a fixed UI set, not data; their COUNTS (if shown) would derive from `countries.*` partition. |

## 1b. Diplomacy — country card: `unavailable` (schema-present-but-unfed / no field)

| gui element (name + type) | intended source | fidelity | reason |
|---|---|---|---|
| `leader_name` (see 1a) | `commanders.leader.name` | **unavailable** | `commanders` domain not folded into this snapshot. |
| `elections` | election date | **unavailable** | no schema field. |
| `trade_grid` (gridboxtype, in `trade_info`) → `diplomacy_trade_entry` rows (`icon`/`chart`/`mini_piechart_overlay`) | `diplomacy_world.trade_contracts` / `market_live.contracts` | **unavailable** | **Codex-confirmed.** Both top-level trade sources **absent** in this snapshot (schema-present, unfed). Resource-trade pie rows can't be built → grid empty/"—". Becomes `exact` once `market_live`/`trade_contracts` is fed. |
| `estimated_military_forces` / `estimated_navy_forces` / `estimated_airforce` / `estimated_civil_industry_size` / `estimated_military_industry_size` / `estimated_naval_industry_size` / `estimated_convoy_size` / `estimated_manpower_size` (8× instantTextboxType, `"?"`, in `estimated_enemy_force_info`) | intel-estimate of `<SEL>`'s forces | **unavailable** | These are **fog-of-war ESTIMATES** the in-game intel system computes from intel level vs the target's real `military.*`/`economy.*`. We store the *real* values but not the *estimate* (intel-level-scaled obfuscation) → the literal `"?"` placeholder is honest here; badge `unavailable`. (A future variant could show the real value badged `proxy`, but that misrepresents the intel view.) |
| `ideas_grid` (gridBoxType, in `diplomacy_idea_category_entry`/`..._noheader_entry`) → idea rows | `countries.<SEL>.politics.ideas_other[]` | **derived** | National-spirit/idea icons. `ideas_other` IS present (spirits/advisors/traits) → rows bind; the category split + per-idea sprite/loc is a static idea-DB lookup, so derived. *(Moved here as a usable-but-derived note; not strictly unavailable.)* |
| `national_spirit_ideas_grid` / `nat_spirit_ideas_grid_over_defined` (gridboxtype, in `national_spirit_container`) → spirit rows | `countries.<SEL>.politics.ideas_other[]` (national-spirit subset) | **derived** | As above — bindable from `ideas_other`; sprite/loc via idea-DB. |

## 1c. Diplomacy — STATIC / decoration / navigation (no data binding)

- **Card chrome / frames:** `Background` (×many), `diplo_upper_win_bg`, `diplo_flag_frame`,
  `stability_bg`/`war_support_bg` (`GFX_diplo_unity_bg`), `stability_icon`/`war_support_icon`,
  `opinion_info`→`Background` (`GFX_diplo_opinion_bg`), `tabs_background`, `top_bg`,
  `pol_view_bg` (pie overlay), `diplo_leader_frame`, `leader_portrait` (portrait sprite — needs
  asset extraction; cosmetic), `progress_frame`, `show_national_goal_button` (read-only verb →
  open focus tree), `diplo_nat_spirits_bg`, `spirit_title` (loc label), `diplo_details_header`,
  `window_header_text` (loc), all 8 `estimated_*_bg` + `estimated_*_icon` (icons/labels for the
  unavailable estimates), `diplo_filter_area_bg`, `diplo_countrylist_binding`.
- **Tab / navigation buttons (read-only verbs, no datum):** `relations_tab_button`, `info_tab_button`,
  `sort_alphabetical`, `logistics_button` (`OPEN_COUNTRY_LOGISTICS`), `back_button`
  (`OPEN_COUNTRY_LIST`), `close_button`, `intel_ledger_container` (structural).
- **Title / player chrome:** `diplomacy_title` (`DIPLOMACY_DIPLOMACY` loc), `production_header_bg`,
  `player_info_box`→`player_name_bg`/`player_icon`/`player_name` (the player-name box = the human's
  lobby name; `human_name` SCHEMA §3.12 exists but is **not** on these country objects → if bound it
  would be `unavailable`; as a header it's effectively the viewer's display name = derived).
- **Structural containers (positioning only):** `countrydiplomacyview`, `country_info`, `faction`,
  `opinion_info`, `diplomacy_tab_top`, `ruling_party_info`, `active_national_focus_info`,
  `relations_info`, `diplomatic_actions`, `national_spirit_info`, `national_spirit_container`,
  `ideas_info`, `trade_info`, `estimated_enemy_force_info`, `country_list`, `top_filter_window`,
  `filters`, `countries`, `actions_grid` (+ `win_header_short`), `diplomacy_bottom`.

## 1d. Diplomacy — DEFERRED (mutation modals; inert in replay)

All listed as **deferred: read-only replay shows no diplomatic-action dialogs** (PLAN §D mutations
DISABLED). Each is a full modal that, in-game, *performs* an action; in a replay none can fire. The
WAR/faction *outcomes* they would produce are instead surfaced via `diplomacy_world` + the diff-event
timeline (SCHEMA §9.2 `war_declared`/`faction_joined`/`subject_created`/…), not via these dialogs.

| dialog (top-level containerWindowType) | why deferred |
|---|---|
| `standard_diplomacy_controller`, `invite_faction_diplomacy_controller` (reply shells: `title`/`description`/`diplo_reply`/`accept_icon`/`send_button`/`cancel_button`) | generic action confirm/reply → mutation. |
| `diplomacy_action_info_window` → `declare_war` (+ `threat`/`threat_value`, `wargoals_grid`, `flags_grid`, `wargoal_name`, `call_allies_checkbox`), `add_wargoal`, `generate_wargoal` (+ `threat_value`, `modifier_icon`), `stage_coup` (+ `states_grid`/`parties_grid`), `guarantee`, `naval_blockade`, `send_attache`, `lend_lease` (+ `lend_leases_grid`, `num_convoys`, `autonomy_score_gain`), `incoming_lend_lease`, `boost_party_popularity`, `call_allies`/`join_allies` (+ `wars_grid`), `send_volunteers`/`send_expeditionary_force` (+ `group_grid`), `request_expeditionary_forces` (+ slider), `ask_for_state_control`/`give_state_control` (+ `states_grid`), `request_licensed_production` (+ `equipments_grid`) | every one is a **mutation verb** dialog (declare war, add wargoal, coup, guarantee, blockade, lend-lease, boost party, call/join allies, send volunteers/expeditionary, request/give state control, license production). Rendered chrome only; all `send_button`/`cancel_button`/`accept_icon` inert; `threat_value` world-tension aggregate also **unavailable** (SCHEMA §17: only contributor ledger stored, not the aggregate). |

> The embedded `threat_value` (`"50%"`) appears in `declare_war` and `generate_wargoal`. Even if these
> modals were shown, the value would be **`unavailable`** — the world-tension AGGREGATE scalar is not
> stored (SCHEMA §17/§18: only `world_events.threat[]` contributor ledger + `world.world_tension_inputs`).

---
---

# COMBAT VIEWS — roster-aggregation model (PLAN §C.2 + §G)

Per PLAN §C.2 the combat panels are **roster aggregation**, not flat element lists: a `combats[]`
entry → an attacker roster + a defender roster, and (for naval) per-ship rows from
`order_of_battle.ships`. Per PLAN §G **HARD LIMIT**, the battle-DETAIL (combat_width / terrain /
tactics / progress −1..1 / modifiers / reinforcement / air support) is **`unavailable`** — the
*enumeration* (who is fighting, where, duration, lifecycle) is exact but the *detail* is not cracked.
The `.gui` `division_combat_*_entry` / `navalcombat_ship_entry` / `tactics_list_entry` templates are
**runtime row factories** (the rows are not in the `.gui` tree), so the roster-aggregation section
below maps the ROW SHAPES the combat adapter instantiates.

### Combat adapter — `combats[]` → roster rows (the imperative view-model, PLAN §C.2)

`combats[]` entry shape (empirical): `{ combat_id:"5457:62", kind:"land"|"naval"|"air",
location_province, attackers:[[unit_leader_id, 4713], …], defenders:[…], combat_width:null,
terrain:str, progress:float, modifiers:{day,duration} }`.

| roster element produced by the adapter | source | formatter | fidelity | notes |
|---|---|---|---|---|
| **battle enumeration** (one panel/bubble per active combat) | `combats[]` (filter `kind`) | — | **exact** | 13 land combats in the inspected snapshot; lifecycle (appear→persist→disappear) tracked by `combat_id` → SCHEMA §9 `combat_started`/`combat_ended`. |
| **attacker / defender side identity** (the two flags) | `combats[].attackers[0]` / `.defenders[0]` → leader ref → owner TAG | flag-small | **derived** | The ref pair `[unit_leader_id, 4713]` resolves (via `commanders`/`order_of_battle`) to a leader→country; `commanders` absent here → flag falls back to the combat's province controller/owner. Side membership itself is **exact**; the resolved TAG is derived. |
| **attacker / defender LEADER** (portrait + name + skill) | `combats[].attackers[0][0]` (`unit_leader_id`) → `commanders.{generals,field_marshals}` | loc + int | **unavailable** (current data) | Leader REF is exact, but name/skill/portrait need the `commanders` domain (absent in this snapshot) → "—". |
| **location** | `combats[].location_province` | int (id) / loc (name) | **exact** (id) / **derived** (name) | Province name from static province meta. |
| **duration** | `combats[].modifiers.duration` (days) | int/days | **derived** | §G explicitly permits duration. `modifiers.duration`==`day` (44/143/48 in samples). Low-confidence (M\* domain) but allowed. |
| **committed / reserve DIVISION rosters** (per-unit rows) | — (would need `combats[]`→division join) | — | **unavailable** | **Codex correction.** `attackers`/`defenders` are **leader** refs, not division ids; there is NO division roster in `combats[]`. The committed/reserve unit lists + every per-division stat row are `unavailable`. |
| **combat_width** | `combats[].combat_width` | int | **unavailable** | `null` in the snapshot. |
| **terrain** | `combats[].terrain` | category | **unavailable** | Present (`"hills"`) but `M*`-uncracked battle-detail (§G). |
| **progress bar (−1..1)** | `combats[].progress` | progressbar (−1..1) | **unavailable** | Present (`14.55…`) but **NOT the −1..1 GUI bar** (large positive float, different scale) → must not bind to the bar (Codex). |
| **combat modifiers** (per-unit + global icons) | `combats[].modifiers` | category icons | **unavailable** | Only `{day,duration}` present — no real combat-modifier vector. |
| **tactics** (current/countered/list, chance) | — | — | **unavailable** | Tactics not in `combats[]` (§G). |
| **air support** (counts/text) | — | — | **unavailable** | Air-support to a land/naval battle not in `combats[]` (§G). |

> **Naval rosters specifically:** per-ship rows come from `order_of_battle.fleets[].task_forces[].ships`
> (SCHEMA §12). `order_of_battle` is **absent** in this snapshot AND there are **0 naval combats** →
> the entire naval per-ship roster is **`unavailable`** in current data (becomes `exact` for ship
> identity/org/str once `order_of_battle` is fed AND a naval battle is active; the naval battle-DETAIL
> — positioning/screening/stats — stays `unavailable` per §G).

---

# VIEW 2 — LAND COMBAT (`game_assets/gui/land_combat.json`)

Root windows: `landcombatview` (the panel), `tactics_list_view` + `tactics_list_entry` (tactics
picker — mutation), `reserve_combat_entry` / `division_combat_attacker_entry` /
`division_combat_defender_entry` (runtime ROW templates), `global_modifier_icon_entry` /
`modifier_icon_entry` (modifier-icon row templates).

## 2a. Land combat — dynamic (data-bound) — roster identity only

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `attacker_flag` / `defender_flag` (buttonType, `GFX_flag_small2`) | `combats[].attackers[0]`/`.defenders[0]` → side owner TAG | flag-small | **derived** | Side identity exact; TAG derived via leader-ref/province-controller fallback (see adapter). |
| `attacker_leader` / `defender_leader` (iconType portrait) | `combats[].{attackers,defenders}[0][0]` → `commanders` | portrait | **unavailable** (current data) | `commanders` absent → generic `GFX_no_combat_leader`. |
| `attacker_leader_name` / `defender_leader_name` (instantTextboxType) | `commanders.*.name` | loc | **unavailable** (current data) | As above. |
| `attacker_leader_skill` / `defender_leader_skill` (instantTextboxType) | `commanders.*.level` | int | **unavailable** (current data) | As above. |
| `window_title` (instantTextboxType, `BATTLE_BATTLE`) | static loc + `combats[].location_province` (name) | loc | **derived** | Title "Battle of <province>" — province from `location_province`. |
| `attacker_committed_label` / `defender_committed_label` (`"4 Divisions"`) | committed division count | int | **unavailable** | Needs the committed-division roster (not in `combats[]`). |
| `attacker_reserves_label` / `defender_reserves_label` (`"4 Reserves"`) | reserve division count | int | **unavailable** | As above. |

## 2b. Land combat — battle-DETAIL → `unavailable` (PLAN §G HARD LIMIT)

| gui element (name + type) | intended source | fidelity | reason |
|---|---|---|---|
| `combat_progress` + `land_battle_progress_frame` + `combat_heading` (iconType bars) | `combats[].progress` (−1..1 bar) | **unavailable** | `progress` is not the −1..1 bar (Codex). |
| `combat_width` (instantTextboxType, `99`) | `combats[].combat_width` | **unavailable** | `null`. |
| `attacker_army_width` / `defender_army_width` (instantTextboxType) | per-side used width | **unavailable** | Not in `combats[]`. |
| `terrain_picture` (iconType, `GFX_terrain_default`) | `combats[].terrain` | **unavailable** | `M*`-uncracked battle-detail (§G); present but not bound. |
| `attacker_tactic` / `defender_tactic` (+ `*_countered_tactic_overlay`) | combat tactics | **unavailable** | Tactics not cracked (§G). |
| `attacker_traits_grid` / `defender_traits_grid` (gridBoxType) → leader-trait rows | `commanders.*.traits[]` | **unavailable** (current data) | Needs `commanders` (absent). |
| `attacker_air_count` / `attacker_air_text` / `defender_air_count` / `defender_air_text` (+ `*_air_icon`) | air-support to the battle | **unavailable** | Air support not in `combats[]` (§G). |
| `modifier_icons_container.list` (gridBoxType, attacker/defender) → `global_modifier_icon_entry` / `modifier_icon_entry` rows (`modifier_icon`) | `combats[].modifiers` (combat-modifier vector) | **unavailable** | Only `{day,duration}` present, no modifier vector (§G). |
| **`division_combat_attacker_entry` / `division_combat_defender_entry` ROWS** — `name`, `unit_icon`, `org_bar`, `str_bar`, `soft_attack`, `hard_attack`, `defense`, per-row `modifier_icons_container.list` | committed-division roster + per-division combat stats | **unavailable** | **Codex correction.** No division roster in `combats[]`; per-division org/str/soft/hard/defense are runtime-computed combat stats (§G). The entire committed grid is `unavailable`. |
| **`reserve_combat_entry` ROWS** — `name`, `unit_icon`, `org_bar`, `str_bar` | reserve-division roster | **unavailable** | As above. |

## 2c. Land combat — STATIC / decoration / navigation / mutation (no binding)

- **Chrome / frames:** `Background` (`GFX_land_battle_bg`), `land_battle_leader_frame`/`..._frame2`,
  `attacker_leader_initiative_overlay_glow`/`defender_..._glow`, `attacker_leader_army_hq_icon`/
  `defender_...`, `attacker_leader_skill_bg`/`defender_...`, `attacker_army_width_icon`/`defender_...`,
  `small_flag_frame_thin`/`..._thin2`, the attacker/defender `committed_window`/`reserves_window`
  container `Background`s + `margin`s, `attacker_window`/`defender_window` `Background`s.
- **Navigation / mutation (inert):** `close`, `tactics_list_button` (opens tactics picker → mutation),
  `unit_button` (in row templates — opens unit → read-only verb, no datum).
- **TACTICS PICKER (`tactics_list_view` + `tactics_list_entry`) — entire window deferred/inert:**
  `filter_all`/`filter_attack`/`filter_defend`, `close`, `window_title`, `current_phase`,
  `preferred_tactic_desc_text`, `tactics_list_gridbox` (→ `tactics_list_entry` rows:
  `tactics_icon`/`tactics_name`/`tactics_chance`/`phase`/`attacker_damage_value`/`defender_damage_value`/
  `combat_width_change_value`/`attacker_speed_value`/`attacker_org_modifier_value`/
  `defender_org_modifier_value`/`command_power_cost`/`locked_icon`/`button_background`). The picker is a
  **mutation** (set preferred tactic) AND its per-tactic preview stats are runtime-computed → if ever
  shown, every value is `unavailable`. Listed for completeness.
- **Structural containers:** `landcombatview`, `attacker_window`, `defender_window`,
  `committed_window`, `reserves_window`, `modifier_icons_container`, row-template roots
  (`reserve_combat_entry`, `division_combat_attacker_entry`, `division_combat_defender_entry`,
  `global_modifier_icon_entry`, `modifier_icon_entry`, `tactics_list_entry`).

---

# VIEW 3 — NAVAL COMBAT (`game_assets/gui/naval_combat.json`)

Root: `navalcombatview` (+ `header_container`, `battle_window` with carrier/battle/screen/sub column
gridboxes, `bottom_wnd`), `navalcombat_ship_entry` (runtime ship-ROW template), plus `positionType`
anchors and aim-line icons. **In current data: 0 naval combats + `order_of_battle` absent → the whole
view binds nothing (all `unavailable`), but every element is enumerated below so completeness is
provable.**

## 3a. Naval combat — dynamic (would-bind) — roster identity only

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `flag_attacker` / `flag_defender` (buttonType, `GFX_flag_small2`) | naval `combats[].{attackers,defenders}[0]` → owner TAG | flag-small | **unavailable** (current data) | No naval combat present → "—". Would be `derived` (side exact, TAG via ref/region). |
| `naval_battle_title` (instantTextboxType, `"Naval skirmish"`) | static loc + `combats[].location_province` | loc | **unavailable** (current data) | No naval battle. |
| `leader_attacker_name` / `leader_defender_name` | `commanders.admirals.*.name` | loc | **unavailable** (current data) | No naval combat + `commanders` absent. |
| `leader_attacker_ships_count` / `leader_defender_ships_count` | per-side ship count | int | **unavailable** | Needs the per-ship roster (`order_of_battle.ships`, absent). |

## 3b. Naval combat — battle-DETAIL + per-ship rows → `unavailable` (§G + absent data)

| gui element (name + type) | intended source | fidelity | reason |
|---|---|---|---|
| `progress_attacker` / `progress_defender` (iconType bars) | naval `combats[].progress` | **unavailable** | §G; + no naval combat. |
| `terrain_background` (iconType, `GFX_navalcombat_bg_weather_none`) | sea-zone weather/terrain | **unavailable** | §G battle-detail + no province weather in schema. |
| `modifier_grid` (gridboxType) → modifier rows | naval combat modifiers | **unavailable** | §G. |
| `carrier_penalty_attacker` / `carrier_penalty_defender` (iconType) | carrier-penalty modifier | **unavailable** | §G battle-detail. |
| `leader_stat_attack_text` / `..._defense_text` / `..._maneuvering_text` / `..._coordination_text` (attacker_stats + defender_stats, 8×) | `commanders.admirals.*.{attack,defense,maneuvering,coordination}` | **unavailable** (current data) | Needs `commanders` (absent) + a live battle. |
| `traits_grid_attacker` / `traits_grid_defender` (gridBoxType) → trait rows | `commanders.admirals.*.traits[]` | **unavailable** (current data) | As above. |
| `left_wnd`/`right_wnd` **stat readouts** — `stat_depth_charges`, `stat_anti_air`, `stat_torpedo`, `stat_heavy_gun`, `stat_light_gun`, `stat_positioning` (12× across both sides) | aggregate fleet combat stats | **unavailable** | Runtime-computed combat stats (§G). |
| `*_capital_screening_efficiency` / `*_carrier_screening_efficiency` (4× instantTextboxType, `"100%"`) | screening efficiency | **unavailable** | Runtime-computed battle-detail (§G). |
| **`navalcombat_ship_entry` ROWS** — `icon` (ship type), `bar_str`, `bar_org`, `desc_left`, `desc_right`, `icon_hidden`, `escape_progress`/`escape_icon_*`, `pride` | `order_of_battle.fleets[].task_forces[].ships[]` (per-ship: definition/org/strength/name) | **unavailable** (current data) | `order_of_battle` absent + 0 naval combats. Ship identity/org/str would be `exact` once `order_of_battle` is fed; the in-battle escape/hidden/positioning state stays `unavailable` (§G). |
| carrier/battle/screen/sub **column gridboxes** (`grid_carrier`/`grid_battle`/`grid_screen`/`grid_hidden`/`grid_visible`, ×both sides + `bottom_wnd` grids) → ship rows | per-ship roster sorted into role columns | **unavailable** (current data) | Same as ship rows; the role-column sort needs ship class + in-battle flags. |

## 3c. Naval combat — STATIC / decoration / navigation / mutation (no binding)

- **Chrome / frames:** `Background` (`GFX_naval_battlemap_bg`), `attacker_flag_frame`/`defender_flag_frame`,
  `leader_attacker_frame`/`leader_defender_frame`, all `stat_*_bg` (12×), `*_screening_efficiency_bg`
  (4×), `naval_battle_front`→`test` overlay, `tooltip_background`s (column tooltips:
  `SCREENING_BOX_TOOLTIP`/`CAPITAL_BOX_TOOLTIP`/`CARRIER_BOX_TOOLTIP`/`SUBMARINE_BOX_TOOLTIP`),
  `bottom_wnd` `Background`s, `margin`.
- **Aim-line / anchors (positionType + cosmetic icons):** `subwnd_offset`,
  `navalcombat_column_back_field_left`/`..._right`, `navalcombat_column_combat_left`/`..._right`,
  `navalcombat_aim_line` (`GFX_aim_body`), `navalcombat_aim_head` (`GFX_aim_head`) — firing-animation
  cosmetics, not bound.
- **Navigation / mutation (inert):** `close`, `disengage` (`DISENGAGE_FROM_COMBAT` → mutation).
- **Structural containers:** `navalcombatview`, `header_container`, `attacker_stats`, `defender_stats`,
  `battle_window`, `outside_wnd`, `top_wnd`, `middle_wnd`, `left_wnd`/`right_wnd` (×levels),
  `column_screen`/`column_battle`/`column_carrier`/`box_subs`, `bottom_wnd`,
  `navalcombat_ship_entry` (row-template root).

---

# VIEW 4 — COMBAT LOG (`game_assets/gui/combat_log.json`)

Root: `combat_log_view` (frame), `combat_log_page` (header + month slider + 3 tabs), three tab bodies
(`combat_log_combat_tab`, `combat_log_equipment_tab`, `combat_log_template_tab`), and four runtime ROW
templates (`combat_log_combat_item`, `combat_log_loss_item`, `combat_log_enemy_loss_item`,
`combat_log_div_template_item`).

> **Nature of this view (important).** The combat log is a **HISTORICAL per-country ledger** the engine
> accumulates (past battles, equipment lost/recovered/attrition, template win/total counts). Our
> snapshots do **not** carry this ledger — `combats[]` is the set of CURRENTLY-ACTIVE engagements, not
> a history. The replay reconstructs combat HISTORY from the **diff-event timeline** (SCHEMA §9.2
> `combat_started`/`combat_ended`, casualties `inferred`), NOT from a stored combat-log domain.
> Therefore most value cells here are **`unavailable`** from snapshot data; a future combat-log
> ADAPTER fed by the diff timeline could populate the combat-list tab `derived`. Each cell is mapped
> below with that reasoning.

## 4a. Combat log — dynamic / derivable

| gui element (name + type) | source | formatter | fidelity | notes |
|---|---|---|---|---|
| `combats_list` (gridBoxType, in `combat_log_combat_tab`) → `combat_log_combat_item` rows | diff-event timeline `combat_started`/`combat_ended` (per `<VIEWER>`) | row factory | **derived** | Reconstructable from the event timeline (not a stored ledger). Until the combat-log adapter is wired → `empty_msg` (`COMBAT_LOG_EMPTY_COMBATS`). |
| `combat_log_combat_item.date` (instantTextBoxType) | event `date`/`hour_index` of the battle | clock/date | **derived** | From the reconstructed event. |
| `combat_log_combat_item.location` (buttonType text) | event `combats[].location_province` (name) | loc | **derived** | Province name. |
| `combat_log_combat_item.atk_flag` / `def_flag` (buttonType, `GFX_flag_small2`) | the two belligerent TAGs of the event | flag-small | **derived** | Side TAGs from the reconstructed combat. |
| `period_desc` / `min_log_months` / `max_log_months` (instantTextbox) | month-window of the log slider | int/loc | **derived** | The 1..12-month window is a UI control over the (reconstructed) history range. |

## 4b. Combat log — `unavailable` (no stored ledger; runtime/aggregate)

| gui element (name + type) | intended source | fidelity | reason |
|---|---|---|---|
| `combat_log_combat_item.atk_lost_manpower` / `def_lost_manpower` / `atk_lost_armor_eq` / `def_lost_armor_eq` / `atk_lost_air_eq` / `def_lost_air_eq` / `atk_lost_other_eq` / `def_lost_other_eq` (8×) | per-battle casualty/equipment-loss ledger | **unavailable** | Per-battle losses are `inferred` at best (SCHEMA §9 `combat_ended.detail`) and not stored as a ledger → "—". |
| `combat_log_combat_item.icon_attack`/`icon_attack_overrun`/`icon_defence`/`icon_defence_overrun` | per-battle attack/defence/overrun outcome | **unavailable** | Outcome flags not stored. |
| `combat_log_combat_item.terrain_picture` (+ overlay) | battle terrain | **unavailable** | Battle-detail (§G); also not in the historical ledger. |
| `combat_log_combat_item.leader_attacker`/`leader_defender` (buttonType portrait) | `commanders.*` of the historical battle | **unavailable** (current data) | Needs `commanders`. |
| `atk_modifiers_window.gridbox`/`def_modifiers_window.gridbox` → modifier rows | per-battle modifier vector | **unavailable** | §G + not stored. |
| `title_losses_in_manpower` / `num_combats_won_factor` (instantTextBox) | aggregate casualty + win-rate over window | **unavailable** | Aggregate ledger not stored (`casualties.killed`/`.captured` exist per-country in SCHEMA §3.2 but are `S/D`, null-until-computed, and not folded here). |
| **EQUIPMENT tab** — `num_equipment_lost_in_combat` / `num_equipment_recovered_in_combat` / `num_equipment_lost_by_attrition` / `num_enemy_equipment_lost`; `equipments_list` (smoothListBox) → `combat_log_loss_item` rows (`eq_name`/`num_loss_in_combat`/`num_loss_recovered`/`num_loss_by_attrition`/`equipment_icon`) + `combat_log_enemy_loss_item` rows | per-country equipment loss/recovery/attrition ledger | **unavailable** | This running ledger is not in the snapshot → `empty_msg` (`COMBAT_LOG_EMPTY_EQ`/`COMBAT_LOG_EMPTY_ENEMY_EQ`). |
| **TEMPLATE tab** — `div_templates_list` (smoothListBox) → `combat_log_div_template_item` rows (`template_name`/`num_won`/`num_fought_in`/`counter`) | per-template win/fought-in tally | **unavailable** | Template names exist (`designs.division_templates[]`, SCHEMA §11) but the per-template combat-win/total TALLY is a running ledger not stored → `empty_msg` (`COMBAT_LOG_EMPTY_TEMPLATES`). |

## 4c. Combat log — STATIC / decoration / navigation (no binding)

- **Chrome / frames / titles:** `Background` (×16), `battle_log_top`, `combat_log_info_header`/`..._header3`,
  `combat_log_info_bg`/`..._bg_overlay`, all column-title labels (`title` `COMBAT_LOG`,
  `title_losses_in_manpower`-label, `title_equipment_lost`, `title_lost_in_combat`, `title_recovered`,
  `title_lost_by_attrition`, `title_enemy_equipment_lost`, `title_templates`, `title_combats_won`,
  `title_combats_total` — these are loc HEADERS, static; the VALUE cells under them are in 4b),
  `manpower_icon` (`GFX_casuality_icon`), `equipment_icon`/`counter` (row icons → asset),
  `empty_msg` (×4 loc placeholders), `groups` (OverlappingElementsBox — period chrome).
- **Tab / navigation buttons (read-only view-switch verbs):** `close_button`, `combat_tab_button`,
  `equipment_tab_button`, `template_tab_button`, `filter_button`.
- **Scrollbars (UI control, no datum):** `log_month_slider` (+ `Slider`/`Track`/`Decrease`/`Increase`/
  `tileSize`), `combats_slider` (+ children), the two `smoothListBoxType` scroll states.
- **Structural containers:** `combat_log_view`, `page_left`, `combat_log_page`,
  `combat_log_combat_tab`, `combat_log_equipment_tab` (+ `subwindow`/`upper`/`lower`),
  `combat_log_template_tab`, and the four row-template roots.

---
---

## Element accounting (provably complete)

### DIPLOMACY (`diplomacy.json`)
The country CARD + country LIST hold all bindable datums. The ~17 action MODALS + 2 reply shells are
deferred mutations (§1d) — every element inside them falls under "deferred" (their grids/labels/buttons
are action-construction UI, inert in replay).

| bucket | count (logical) | examples |
|---|---:|---|
| **Dynamic-bound** (incl. derived/unavailable) | **~24** | flag, ideology_icon, country_name, faction_name, our/their_opinion (×value+flag), stability, war_support, pie chart, party/ideology name, focus goal+label+progress, relations_grid, countries_grid, opinion arrows+flags, ideas/spirit grids |
| **`unavailable`-badged** | **11** (leader_name, elections, trade_grid, 8× estimated_* readouts) | grouped: commanders-unfed (1), no-field (1), trade-unfed (1), intel-estimate (8) |
| **Deferred: mutation modals** | **~19 dialogs** | declare_war, add_wargoal, generate_wargoal, stage_coup, guarantee, naval_blockade, send_attache, lend_lease, incoming_lend_lease, boost_party, call/join_allies, send_volunteers, send_expeditionary, request_expeditionary, ask/give_state_control, request_licensed_production, + 2 reply shells |
| **Static / decoration / navigation** | **~60** | card chrome, frames, tab/nav buttons, title, player box, structural containers |

### LAND COMBAT (`land_combat.json`) — type counts (from file `stats`): containerWindowType 19, background 14, buttonType 12, iconType 53, instantTextboxType 28, instantTextBoxType 10, gridBoxType 11, slotsize 11, borderSize 3, margin 5 (= **166** elements)

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** (roster identity) | **~6** | attacker/defender flag, window_title (loc+province) — side identity derived |
| **`unavailable`-badged** (battle-detail + rosters) | **~30** | progress bar, combat_width, army_width ×2, terrain, tactics ×2, traits grids ×2, air counts/text ×4, modifier grids, division rows (attacker/defender entry: name/icon/org/str/soft/hard/defense), reserve rows, leader portrait/name/skill ×6 |
| **Deferred: mutation** (tactics picker) | **1 window** (~16 elements) | `tactics_list_view` + `tactics_list_entry` |
| **Static / decoration / navigation** | **~75** | frames, glows, hq icons, skill bgs, container roots, close, unit_button verbs |

### NAVAL COMBAT (`naval_combat.json`) — type counts: positionType 5, containerWindowType 29, background 11, buttonType 4, iconType 46, instantTextboxType 15, instantTextBoxType 16, gridboxType 1, gridBoxType 16, slotsize 17, margin 1 (= **161** elements)

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** (would-bind, all `unavailable` in current data) | **~4** | flag_attacker/defender, naval_battle_title, leader names — all "—" (0 naval combats) |
| **`unavailable`-badged** | **~40** | progress ×2, terrain_bg, modifier_grid, carrier_penalty ×2, leader stats ×8, trait grids ×2, stat readouts ×12, screening ×4, ship rows + role-column grids |
| **Static / decoration / navigation** | **~115** | aim-line/anchors, frames, stat/screening bgs, tooltip bgs, column containers, close/disengage |

### COMBAT LOG (`combat_log.json`) — type counts: containerWindowType 15, background 16, iconType 15, buttonType 11, instantTextBoxType 38, instantTextboxType 2, OverlappingElementsBoxType 1, extendedScrollbarType 2, gridBoxType 3, slotsize 3, smoothListBoxType 3, borderSize 4, tileSize 1, slider/track/decrease/increase 2 each, margin 1 (= **128** elements)

| bucket | count | examples |
|---|---:|---|
| **Derivable** (from diff-event timeline, not stored ledger) | **~9** | combats_list rows, date, location, atk/def flags, period/month controls |
| **`unavailable`-badged** | **~24** | 8× per-battle loss cells, outcome icons ×4, terrain, leader portraits ×2, modifier grids ×2, aggregate manpower/win-factor, EQUIPMENT tab (4 totals + 2 row types), TEMPLATE tab (template rows) |
| **Static / decoration / navigation** | **~95** | titles/labels, chrome, tab/filter buttons, 2 scrollbars, container roots, empty_msg ×4 |

---

## `unavailable`-badged summary (with reason)

**Diplomacy (11+):** `leader_name` (commanders unfed), `elections` (no field), `trade_grid` rows
(`market_live`/`trade_contracts` unfed), 8× `estimated_*` forces/industry/convoy/manpower (intel-level
ESTIMATE not stored — real values would misrepresent the fog-of-war view); the `threat_value` inside
the deferred war/wargoal modals (world-tension aggregate not stored, SCHEMA §17).

**Combat (all three views):** combat_width (`null`), terrain (M\*-uncracked §G), progress −1..1 bar
(`progress` is a different scale), tactics, combat modifiers, air support, carrier-penalty/screening,
all per-division and per-ship STAT rows (runtime-computed §G), committed/reserve DIVISION rosters (no
division-join in `combats[]` — Codex), naval per-ship rows + role columns (`order_of_battle` absent +
0 naval combats), combat leaders/traits (commanders absent). **Exact/derived in combat:** the
enumeration, side identity, `location_province`, and `modifiers.duration` only.

**Combat log:** the entire stored-ledger model (per-battle losses, equipment lost/recovered/attrition,
template win tallies, aggregate manpower/win-factor) is not in the snapshot — combat HISTORY is
reconstructed from the diff-event timeline (combat-list tab `derived`), value cells `unavailable`.

> **`unavailable` ≠ "missing forever" (SCHEMA §10 vs parser-unfed).** Two distinct causes are folded
> together under the badge but differ in prognosis: **(a) parser-unfed** (becomes `exact` when its
> domain is wired) — `commanders`, `order_of_battle`, `market_live`/`trade_contracts`, `combats[]`
> naval entries; **(b) genuine hard limit** (§G, runtime-computed / uncracked, stays `unavailable`) —
> combat battle-detail (width/terrain/tactics/progress-bar/modifiers/air/screening/per-unit combat
> stats), intel force ESTIMATES, world-tension aggregate. Both render "—" per no-fabrication; neither
> is faked.

---

## Historical review verdict (adversarial review, 2026-06-26)

Historical note: this records a completed adversarial review from the old
Claude-era workflow. Codex-native sessions must use normal sub-agents/partner
review and must not run the old local CLI command. **Claim A (combat battle-detail badging): CONFIRMED with one correction** —
terrain/progress/combat_width/modifiers correctly `unavailable` (the raw `terrain`/`progress` are
present but `progress` must NOT bind the −1..1 bar; the M\* domain is uncracked); `modifiers.duration`
correctly `derived` (§G allows duration); `location_province` exact (id)/derived (name). **Correction
folded in:** the attacker/defender **rosters are NOT division rosters** — `attackers`/`defenders` are
`[unit_leader_id, 4713]` LEADER refs only, so committed/reserve division lists + every per-division
stat row are `unavailable`, and only the leader ref pairs are exact-raw / derived-resolved. **Claim B
(diplomacy opinion/trade): CONFIRMED with one correction** — `opinion_sum` is the right field for the
signed opinion number (do not re-sum modifiers); nested `opinion[TGT].opinion_modifiers` = numeric
`token_id`+`value`, names `unavailable`; trade grid `unavailable` (unfed); wars/factions/subjects
populated. **Correction folded in:** the opinion DIRECTION — the owning country's dict holds ITS opinion
of the keyed target, so `our_opinion = countries.<VIEWER>.diplomacy.opinion.<SEL>.opinion_sum` and
`their_opinion = countries.<SEL>.diplomacy.opinion.<VIEWER>.opinion_sum` (my initial draft had these
reversed).
