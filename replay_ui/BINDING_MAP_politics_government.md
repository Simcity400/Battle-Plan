# BINDING MAP — POLITICS / GOVERNMENT (Phase-3 binding sweep)

> **What this is.** The COMPLETE, no-truncation data-binding map for the five in-game the game views the
> native replay UI binds for the country POLITICS / GOVERNMENT surface: **politics**
> (`game_assets/gui/politics.json`, from `interface/countrypoliticsview.gui`), **power balance**
> (`power_balance.json`, from `interface/powerbalanceview.gui`), the 1.19 unified **command/character**
> detail (`command_character.json`, from `interface/command_character_view.gui`), the **advisor-role
> picker** (`advisor_role_selection.json`, from `interface/advisorroleselectionview.gui`), and
> **occupation** (`occupation.json`, from `interface/countryoccupationview.gui`). It realises
> GAME_UI_REPLICA_PLAN §C — the "comprehensive but mechanical binding sweep" — and §C.4 row "Country
> politics / government" (grids + conditional slots; grouping adapter; some scripted slots degrade).
> EVERY element in each `.gui` tree is accounted for, in exactly one bucket: dynamic (data-bound),
> static/decoration (no binding), or deferred (needs imperative adapter / mutation modal). Nothing is
> omitted; where a `.gui` element exists but our snapshot has no usable field, the element is listed
> and **badged `unavailable`** with the missing field named.
>
> **Binding-rule shape** (PLAN §C.1): `{ target (gui element name+type, sprite/text, loc key), source
> (exact snapshot dotted field path), format (formatter id), fidelity (exact|derived|proxy|unavailable) }`.
> **Source paths** use SCHEMA dotted form `countries.<TAG>.…` / `states.<id>.…` / `world.…`
> (`replay/SCHEMA.md` §3 / §4 / §18, and the authoritative machine-readable mirror
> `replay/schema.py` `FIELD_SOURCES`, which carries the **newly-wired LIVE leaves** SCHEMA.md predates).
> `<TAG>` = the player/observed country.
>
> **No-fabrication (SCHEMA §0.2, PLAN principle 3):** `null` renders "—", never `0`. `0`/`0.0` is a
> measured zero. A field schema-present but never fed by the current parser is badged `unavailable`
> (not silently dropped). A conditional `visible={…}` scripted slot whose backing var we do NOT record
> gets a **"scripted content not captured"** badge (= a flavour of `unavailable`).
>
> **Grid row-factory model (PLAN §B.4 / §C.2).** `gridBoxType`/`gridboxtype` are **row factories**,
> not passive containers: the engine instantiates a named ENTRY-TEMPLATE window once per bound array
> element. In these `.gui` files the grids and their entry-template windows are SEPARATE top-level
> windows (e.g. `idea_categories_grid` is the grid; `country_politics_idea_category_entry` is the row
> template it instantiates, which itself nests an `ideas_grid` → `political_idea_entry` per idea). Each
> entry template is mapped once below; at runtime it repeats per array element. The grid container
> itself carries no datum.
>
> **Read-only verb note (PLAN §D).** Every `*_button` whose in-game role is to PERFORM an action
> (create/dismantle faction, change law, add/remove idea/spirit, change autonomy, set occupation law,
> release/return territory, assign a garrison template, switch/recruit a character) is a **mutation**
> verb. In a replay these render as real chrome but are **inert** (no datum, no action). They are
> listed under each view's static/deferred section, never bound.

---

## Provenance of the fidelity calls

Grounded in **`replay/schema.py` FIELD_SOURCES + the dataclass docstrings** (the STABLE, wiring-
complete schema — authoritative for the newly-wired LIVE politics/commanders domains that postdate
SCHEMA.md), cross-checked against `replay/SCHEMA.md` §3.3 / §3.9 / §3.11 / §4 / §18 and the on-disk
demo snapshot family (`replay/snapshots/*/…snapshot.json`, GER 1941, `schema_version "1"`).
**Adversarially reviewed by Codex** (read-only, 2026-06-26); its verdict and corrections are folded in
(see the "Codex round" note at the end).

**Critical schema facts that drive the badges (verified in `schema.py` FIELD_SOURCES — these UPGRADE
several SCHEMA.md `S`-only leaves to LIVE-first `M/S` EXACT):**

| schema leaf | source tag (FIELD_SOURCES) | shape / element | consequence in these views |
|---|---|---|---|
| `countries.*.politics.ruling_party` | **M/S** (live ideology-group id, 46/46 EXACT) | str | ideology label + pie + faction-tile **exact** |
| `countries.*.politics.party_popularities` | **M/S** (4-ideology vector, 46/46 EXACT) | `{ideology: fraction}` | the political pie chart + `parties_grid` rows → **exact** |
| `countries.*.politics.{stability,war_support,political_power,command_power}` | **M** | float | (no scalar widgets in politics.gui except the focus-cost/PP icons; see §1) |
| `countries.*.politics.appointed_advisors` | **M** (HIRED cabinet advisor idea-token list, 46/46 EXACT) | list[str] | the cabinet/advisor idea tiles (`ideas_grid` government category) → **derived** (tokens exact; sprite/loc via idea-DB) |
| `countries.*.politics.ideas_other` / `.ideas` (M) | S / **M** | list[str] | national-spirit grid + idea-category grids → **derived** (tokens exact; frame/loc = idea-DB lookup) |
| `countries.*.politics.laws` (M, typed triple) + `.laws.{economy,trade,conscription}` (S) | **M** / S | tokens | law ideas surface as idea tiles in the government category; the law CHANGER modal is a mutation (deferred) |
| `countries.*.politics.autonomy` | **S** | `{level, overlord}` | `autonomy_progress_view` (this-country-as-subject) → **derived** |
| `countries.*.diplomacy.{overlord,autonomy_state,autonomy_ratio}` | **M** (17/17 EXACT) | TAG / str / float | richer LIVE subject data for `autonomy_progress_view` → **exact/derived** |
| `countries.*.politics.faction` | **S** `{name, leader, is_leader}` | obj | `faction_name` + faction logo tile → **exact/derived** |
| `countries.*.commanders.{leader,party_leaders,advisors}` | **S** | obj/list | leader portrait/name + party-popularity leaders + cabinet detail (save channel) |
| `countries.*.commanders.{army_leaders,navy_leaders}` (+`.skill`,`.traits`) | **M** | list | LIVE unit-leader roster for the command/character grid (char_id/name/skills/xp; **role split + overall level RESISTED** unless leader_detail enrichment on) |
| `states.<id>.occupation.{resistance,compliance}` | **M/S** (live state_detail, EXACT) | 0..100 float | occupation list resistance/compliance bars + text → **exact** |
| `states.<id>.occupation` (rest) / `.occupation_law` | **S** | obj `{…, occupied_country_tag, operational_status}` / str | per-state occupied-country flag + law icon → **exact/derived** (save), **unavailable** if save not folded |
| `world.power_balances[]` | **S** | `{template, value, left_side, right_side, countries[]}` (`trending_side` per SCHEMA.md §18, not in the demo) | the power-balance bar view → **exact** (value+sides) / **unavailable** (`trending_side` if absent) |

**Genuinely runtime-computed / scripted-not-recorded (stay `unavailable` by nature, PLAN §G):** the
power-balance RANGE band names (`active_range_name`, `range_indicator` position) — scripted side/range
thresholds; per-occupied-country aggregate manpower/IC/resource **gains** (computed occupation yields);
garrison TEMPLATE need/strength (computed garrison requirement); the law/garrison/policy CHOICE preview
modifiers (`+10%` etc. in the select-law modal = static law-DEF values, surfaced only with the
occupation-law DB); election DATE display (`elections` text — but see the LIVE `last_election` upgrade);
character SWITCH/recruit cost; exile-government legitimacy + reward slots (exile domain not modeled).

> **Conditional-`visible` scripted slots (the §C.4 "some scripted slots degrade" rule).** Several
> politics.gui sub-windows are shown only when a scripted condition holds — `autonomy_progress_view`
> (this country is a subject), `faction`'s `no_faction`/`in_faction`/`original_in_faction` branches,
> the `industrial_org_entry` MIO tile, the `power_balance_button` (a BoP track is active),
> `collaboration_item`/`exile_item` (collaboration/exile govts exist). Where our snapshot records the
> backing state (`politics.autonomy`/`diplomacy.overlord`; `politics.faction`; `world.power_balances`;
> `extra.collaboration_present`) we resolve which branch shows and bind it. Where it does NOT (per-MIO
> politics tile; exile-legitimacy internals), the slot gets the **"scripted content not captured"**
> badge rather than a fabricated value.

---
---

# VIEW 1 — POLITICS (`game_assets/gui/politics.json`)

Root `countrypoliticsview` + 16 sibling top-level windows: the idea-category row template
(`country_politics_idea_category_entry` → nested `ideas_grid` → `political_idea_entry`), the law/idea
PICKER (`political_ideas_window` + `idea_group` + `political_idea_entry` + the two
`political_selectable_idea_entry_*` + `industrial_org_entry`), the ruling-party row
(`political_party_info_entry`), and the subjects / exiles / occupied-territory MANAGEMENT modals
(`manage_subjects_window`+`subject_item`, `exiles_window`+`exile_item`+`collaboration_item`+
`occupier_flag_entry`, `manage_occupied_window`+`occupied_territory_entry`+`occupy_policy_entry`).
Observed country = `<TAG>`.

Formatters: `int`, `percent` (×100), `loc` (token→localised string), `flag`/`flag-small`,
`flag-shield`, `pie-chart` (`party_popularities`→arcs), `category` (ideology/idea/law/autonomy
token→loc+sprite), `idea-trait-frame` (frame of `GFX_idea_traits_strip`), `progressbar-fraction`,
`portrait` (leader sprite name → asset).

## 1a. Politics — dynamic (data-bound) elements

### Government summary (root `countrypoliticsview`)

| gui element (name + type, sprite/text, loc key) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `chart` (iconType, `GFX_political_chart`, in `political_pie_chart`) | `countries.<TAG>.politics.party_popularities` | pie-chart (ideology→arc) | **exact** | `{ideology: fraction}` → pie arcs. **M/S LIVE-first (46/46 EXACT)** — UPGRADE from SCHEMA.md's `S`-only. |
| `parties_grid` (gridBoxType, in `chart_explanation`) → `political_party_info_entry` rows | `countries.<TAG>.politics.party_popularities` (one row per ideology) | row factory (see row children) | **exact** | The pie legend: one party row per popularity entry. |
| ↳ `political_party_info_entry.name` (instantTextboxType, `"party name"`) | ideology token (+ party meta) | category/loc | **derived** | Party display name; ideology key is exact, the localised party name is an idea/party-DB lookup. |
| ↳ `political_party_info_entry.color_block` (iconType, `GFX_pol_party_colour`) | ideology token → party colour frame | category (ideology→colour) | **derived** | Colour swatch chosen from the ideology id (static colour map). `color_block_bg`/`leading_pol_party_bg` are static chrome. |
| `ideology` (instantTextboxType, `"ideology"`, in `ruling_party_info`) | `countries.<TAG>.politics.ruling_party` | category (ideology→loc) | **exact** | Ruling-ideology label. **M/S LIVE-first EXACT.** |
| `leader` (iconType, `GFX_leader_unknown`) | `countries.<TAG>.commanders.leader.portrait_gfx` | portrait (sprite name→asset) | **unavailable** (current data) | SCHEMA §3.9 carries `commanders.leader.portrait_gfx` (a sprite NAME, S); `commanders` is **not folded in the demo snapshot** + portrait rendering is a separate asset-extraction step → generic `GFX_leader_unknown`. `leader_frame2` is static chrome. |
| `leader_name` (instantTextboxType, `"leader_name"`) | `countries.<TAG>.commanders.leader.name` | loc | **unavailable** (current data) | `commanders` domain unfed in the demo snapshot → "—". Becomes `exact` once `commanders` is folded. |
| `goal_icon` (iconType, `GFX_goal_unknown`, in `active_goal`) | `countries.<TAG>.focus.current` | category (focus→sprite) | **exact** (state) / **derived** (icon) | Current focus icon; membership in `focus.current` is exact (M/S LIVE), the base sprite is a focus-DEF lookup. `null`→generic. |
| `title` (instantTextboxType, `POLITICS_ACTIVE_GOAL`, in `active_goal`) | `countries.<TAG>.focus.current` → loc | loc (focus id→name) | **exact** | Active-focus name; the static label `POLITICS_ACTIVE_GOAL` shows when no focus. |
| `progress` (iconType, `GFX_activegoal_progress`, in `active_goal`) | `countries.<TAG>.focus.progress` / focus duration | progressbar-fraction | **derived** | `focus.progress` is days (M/S EXACT); the bar denominator (focus-DEF duration ≈ 70×cost days) is adapter-supplied → fraction is a derive. `progress_frame` is static chrome. |
| `focus_cost` (instantTextboxType, `3`, in `active_goal`) | active continuous-focus PP cost (focus-DEF) | int | **unavailable** | PP cost is a focus-DEF constant, not country state; the `pol_power_icon`/`continuous*_glow` beside it are static/visibility chrome. |
| `continuous_glow` (iconType, `GFX_ongoing_focus_goal_polview`, in `active_goal`) | visible iff `focus.current_continuous` set | visibility | **derived** | `current_continuous` is a live-only leaf (M); shown when a continuous focus is active. |
| `spirit_grid` (gridboxtype, in `national_spirit_container`) → spirit tiles | `countries.<TAG>.politics.ideas_other[]` (national-spirit subset) | row factory; idea-trait-frame | **derived** | National-spirit tiles. `ideas_other` tokens are exact (S); per-idea sprite/loc is an idea-DB lookup. The over-defined variant grid `spirit_grid_over_defined` binds the same list (render-mode choice). |
| `idea_categories_grid` (gridBoxType, in `ideas`) → `country_politics_idea_category_entry` rows | grouping adapter over `countries.<TAG>.politics.{ideas_other,appointed_advisors,laws}` by idea CATEGORY (government / economy / trade / mobilisation / political advisor / high command / theorist) | row factory (see below) | **derived** | The ideas/laws/advisors grid, one HEADER row per non-empty category. Category id derivable from each idea token via the idea-DB; tokens are exact. |
| ↳ `country_politics_idea_category_entry.name` (instantTextboxType, `"Government"`) | category id → loc | category/loc | **derived** | Category header name (Government / Economy / Trade / Mobilisation / Political Advisor / …). |
| ↳ `country_politics_idea_category_entry.category_icon` (iconType, `GFX_idea_categories`) | category id → frame | category (id→frame) | **derived** | Category sprite (idea-DB lookup). `category_header` is static chrome. |
| ↳ `country_politics_idea_category_entry.ideas_grid` (gridboxtype) → `political_idea_entry` rows | the category's member idea tokens (`ideas_other` / `appointed_advisors` / law ideas filtered to this category) | row factory; per tile below | **derived** | One tile per active idea/advisor/law in the category. **`appointed_advisors` is M LIVE (46/46 EXACT)** so the CABINET advisor tiles bind; law ideas come from `politics.laws`/`ideas`. |
| ↳↳ `political_idea_entry.idea_traits` (iconType, `GFX_idea_traits_strip`) | the idea token → trait-frame | idea-trait-frame | **derived** | Idea/advisor trait frame; token exact, frame is an idea-DB index. `add_idea_button` is a mutation (inert); `idea_alert_glow` is visibility chrome. |
| `faction_name` (instantTextboxType, in `ruling_party_info`→`faction`→`original_in_faction`/`in_faction`) | `countries.<TAG>.politics.faction.name` | category (faction→loc) | **exact** (membership) / **derived** (loc) | Faction display name; `faction.name` (S) is exact, the localised string is a lookup. The branch shown (`no_faction` vs `in_faction` vs `original_in_faction`) is resolved from `faction` presence + `faction.is_leader`. |
| `faction_icon` (iconType, `GFX_faction_logo_generic_democratic`, in `in_faction`) | `countries.<TAG>.politics.faction.name` → logo sprite | category (faction→logo) | **derived** | Faction logo chosen from the faction template name (lookup). |
| `power_balance_percentage` (instantTextboxType, `"90%"`, in `power_balance_button`) | matching `world.power_balances[]` entry for `<TAG>` → `value` | percent | **derived** | The BoP summary % on the politics screen; `world.power_balances[].value` (S) is a signed −1..1 track value → mapped to a 0..100% position (a derive). The button is shown only when a BoP track is active for the country (`countries[] ∋ <TAG>`). |
| `power_balance_levels` (instantTextboxType, `"3/5"`, in `power_balance_button`) | BoP discrete level / total levels | int (`n/m`) | **unavailable** | The discrete BoP level split (current/total side ranges) is a scripted side-range count not stored (only the continuous `value` is). Badge unavailable. |

### `autonomy_progress_view` (shown only when THIS country is a subject — conditional `visible`)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `autonomy_country_flag` (buttonType, `GFX_flag_small2`) | `countries.<TAG>.diplomacy.overlord` (M) ∥ `politics.autonomy.overlord` (S) | flag-small | **exact** (live) / **derived** (save) | Overlord flag. `diplomacy.overlord` is M (17/17 EXACT); `politics.autonomy.overlord` is the S backstop. `country_flag_frame` is static chrome. |
| `overlord_name` (instantTextboxType) | `countries.<TAG>.diplomacy.overlord` → loc | loc (TAG→name) | **exact** (live) / **derived** | Overlord country name. |
| `current_autonomy_name` (instantTextboxType, `"autonomy"`) | `countries.<TAG>.diplomacy.autonomy_state` (M) ∥ `politics.autonomy.level` (S) | category (autonomy→loc) | **exact** (live) / **derived** | Current autonomy-level name (`"autonomy_puppet"` →loc). M 17/17 EXACT. |
| `autonomy_progress` (iconType, `GFX_subject_progressbar`) | `countries.<TAG>.diplomacy.autonomy_ratio` (M) | progressbar-fraction | **exact** | Progress toward the next autonomy level; `autonomy_ratio` is the i32/1e5 ratio (M, EXACT). |
| `current_autonomy_level_icon` (iconType, `GFX_autonomy_unknown_icon`) | `diplomacy.autonomy_state` → level frame | category (level→frame) | **derived** | Autonomy-level icon (level→sprite lookup). `prev_/next_autonomy_level_icon` show adjacent levels (derived from the level ordering). |

### Subjects / occupied / exiles MANAGEMENT modals (data rows bound; mutation buttons inert)

| gui element (name + type, in template) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `subjects_grid` (gridboxtype, `manage_subjects_window`) → `subject_item` rows | aggregate of `countries.<SUBJ>` where `countries.<SUBJ>.diplomacy.overlord==<TAG>` (M) | row factory (grouping adapter) | **derived** | One row per subject of the observed country. **Codex fix:** `diplomacy_world.subjects[]` is NOT a FIELD_SOURCES leaf — the subject set is an adapter aggregation over each country's LIVE `diplomacy.overlord` (M, 17/17 EXACT), falling back to `politics.autonomy.overlord` (S). |
| ↳ `subject_item.autonomy_country_flag` (buttonType, `GFX_player_flag`) | the `<SUBJ>` TAG (group member) | flag-small | **exact** | Subject flag. `large_flag_frame` static. |
| ↳ `subject_item.autonomy_country_name` (instantTextboxType) | `<SUBJ>` TAG → loc | loc | **exact** | Subject country name. |
| ↳ `subject_item.current_autonomy_name` (instantTextboxType) | `countries.<SUBJ>.diplomacy.autonomy_state` (M) ∥ `politics.autonomy.level` (S) → loc | category/loc | **exact** (state) / **derived** (loc) | Subject's autonomy level (M 17/17 EXACT). `autonomy_progress` bar = `countries.<SUBJ>.diplomacy.autonomy_ratio` (M) → **exact**; `prev/next` level icons derived. |
| `occupied_territories_grid` (gridboxtype, `manage_occupied_window`) → `occupied_territory_entry` rows | aggregate of `states.*` where `controller==<TAG>` and `owner≠<TAG>`, grouped by the occupied OWNER tag | row factory (grouping adapter) | **derived** | One row per occupied country. The grouping is an adapter over the states domain (controller≠owner); the per-country aggregate exists in `states.*` (exact) but is re-bucketed (derive). |
| ↳ `occupied_territory_entry.occupied_country_flag` (buttonType, `GFX_player_flag`) | the occupied OWNER tag | flag-small | **derived** | Occupied-nation flag (from the grouping key). `large_flag_frame` static. |
| ↳ `occupied_territory_entry.occupied_country_name` (instantTextboxType) | occupied OWNER tag → loc | loc | **derived** | Occupied-nation name. |
| ↳ `occupied_territory_entry.occupied_country_cost` (instantTextboxType) | occupation manpower/IC cost for that nation | int | **unavailable** | Per-occupied-nation cost is a computed garrison/occupation cost aggregate, not stored. |
| ↳ `occupy_policy_grid` (gridboxtype) → `occupy_policy_entry` rows (`occupation_policy_icon`) | the occupation-law options + the active one | category (law→frame) | **unavailable** | The per-nation occupation-policy SELECTOR shows every selectable law (law-DEF list) + highlights the active one; the active law is in `states.*.occupation_law` (S) but the full selectable-law catalog is law-DB data not in the snapshot → unavailable; the policy buttons are mutations (inert). |

## 1b. Politics — `unavailable` / scripted-not-captured (per-element reasons)

| gui element (name + type) | intended source | fidelity | reason |
|---|---|---|---|
| `leader` portrait + `leader_name` | `commanders.leader.{portrait_gfx,name}` | **unavailable** (current data) → `leader_name` **exact** once folded | **Codex:** NOT inherently unavailable — `commanders.leader` is **S**, so `leader_name` is **exact** when the save commanders domain is folded and `leader` portrait is **derived** from `portrait_gfx` (asset lookup). It is `unavailable` ONLY because the demo SAVE-only snapshot leaves `commanders` unfed. |
| `elections` (instantTextboxType, `"elections"`, in `ruling_party_info`) | next-election date | loc/date | **derived** | SCHEMA §3.3 has no `election_date`, BUT `politics.last_election` + `politics.election_frequency` are LIVE leaves (M, 46/46 EXACT) → the NEXT election date is `last_election + election_frequency months` (a derive). `elections_allowed` (M) gates "no elections". Badge **derived** (not unavailable — the live cadence leaves exist). |
| `focus_cost` (in `active_goal`) | continuous-focus PP cost | int | **unavailable** | focus-DEF constant, not country state. |
| `power_balance_levels` | discrete BoP level | int | **unavailable** | scripted side-range count not stored. |
| `industrial_org_entry.open_roster_button` (buttonType) — the MIO tile in the spirit/idea strip | per-country MIO present? | visibility | **scripted content not captured** | The politics-screen MIO roster tile is shown by a scripted `visible`; per-country MIOs ARE in `countries.<TAG>.mios[]` (S) but that domain is unfed here + the tile's politics-strip placement is scripted → badge. |
| `collaboration_item.*` (`country_name`, `surrender_text`, `progress` bar, `modifier_container`) | collaboration-government roster | — | **scripted content not captured** | `countries.<TAG>.extra.collaboration_present` (S) is only a BOOL — the per-collaborator roster + surrender % + collaboration-bar progress are not modeled; the whole exiles/collab modal is exile-domain data we don't carry. |
| `exile_item.*` (`exile_country_name`, `legitimacy`, `manpower_text`, `exile_legitimacy_progress`, the 6 `exile_*_recruitment` reward icons, `occupier_flags`) | government-in-exile roster + legitimacy | — | **scripted content not captured** | Exile-government legitimacy / reward-slot internals are not in the schema (no exile domain). Whole modal degrades. |
| `occupied_territory_entry.occupied_country_cost` | occupation cost | int | **unavailable** | computed occupation cost not stored. |
| `occupy_policy_grid` rows | selectable occupation-law catalog | category | **unavailable** | law-DB catalog not in snapshot (active law IS, via `occupation_law`). |

## 1c. Politics — deferred (mutation modals / pickers; inert in replay)

- **Law / national-spirit / idea PICKER (`political_ideas_window` + `idea_group` + `political_idea_entry`
  + `political_selectable_idea_entry_grid` + `political_selectable_idea_entry_list`).** This is the
  ADD-idea / CHANGE-law / pick-advisor modal — a **mutation** surface (PLAN §D, DISABLED). Its rows
  (`idea_entry_bg`, `icon`, `idea_traits`, `traits_container.{name,traits}`, `cost`, `stats`, the
  `idea_group.{title,limit,ideas_grid}`) are action-construction UI; in a replay the modal never
  opens. The currently-ACTIVE ideas/laws/advisors it would let you change are surfaced read-only in
  the government idea-category grid (§1a) instead. The `political_idea_entry.add_idea_button` /
  `industrial_org_entry.open_roster_button` are the inert mutation buttons.
- **Management-modal mutation buttons (inert):** `new_faction_button` / `dismantle_faction` /
  `view_factions_button` / `open_faction_button` (faction create/dismantle/open — mutations),
  `add_national_goal_button` / `drop_focus_button` / `drop_continuous_focus_button` (focus
  start/drop — mutations), `prev_/next_autonomy_level` (autonomy change — mutation),
  `show_power_balance` (opens View 2 — read-only verb), `manage_occupied_button` /
  `show_subjects_button` / `show_exiles_collaborations_button` (open the management modals —
  read-only verbs), `subject_item.prev_/next_autonomy_level`, `occupied_territory_entry.release` /
  `occupy_policy_entry.occupation_policy*` (release / set-policy — mutations),
  `exile_item.reinstate_btn` (reinstate — mutation).

## 1d. Politics — STATIC / decoration / navigation (no data binding)

- **Backgrounds / frames / chrome:** `Background` (×19 — `GFX_tiled_plain_bg`, `GFX_tiled_window_1b_border`,
  `GFX_tiled_window2_1b_border`, `GFX_tiled_generic_overlay_bg1_small`, `GFX_tiled_window_transparent`,
  `GFX_pol_goal_bg`, `GFX_subjects_entry_bg`, `GFX_collaboration_entry_bg`,
  `GFX_government_in_exile_entry_bg`, `GFX_occupied_territory`, `GFX_mini_bg`), `production_header_bg`
  (`GFX_header_bg`), `pol_view_bg` (`GFX_pol_view_bg` + the piechart overlay `GFX_pol_piechart_overlay_63x63`),
  `pol_faction_icon` (`GFX_pol_factions_strip` — the static faction-strip frame behind the legend),
  `nat_spirit_glow_overlay`, `continuous_small_glow`, `show_power_balance_glow`, `power_balance_ICON`
  (`GFX_placeholder_bordered`), `prev_/next_level_glow`, `politics_bottom` (`GFX_main_screens_bottom`),
  `rules` (`GFX_rules_sum_icon`), `tutorial_glow`, `leading_pol_party_bg`/`color_block_bg`,
  `large_flag_frame`/`small_flag_frame_thin`/`flag_frame`/`country_flag_frame`, `surrender_icon`,
  `exile_legitimacy_progress_bg`, `manage_occupied_header_bg`/`title_bg`, `backgroundwrapper`.
- **Static label texts (loc constants):** `political_title` (`POLITICAL_POLITICAL`),
  `national_spirit.title` (`POLITICS_NATIONAL_SPIRIT`), `active_goal.title` (`POLITICS_ACTIVE_GOAL`
  placeholder), `view_factions_text` (`CREATE_FACTION`), `manage_occupied_button_text`
  (`MANAGE_OCCUPIED_TERRITORIES`), `show_subjects_button_text` (`MANAGE_SUBJECTS`),
  `manage_exiled_collaborations_governments` (`MANAGE_EXILED_GOVERNMENTS`), the modal titles
  (`MANAGE_SUBJECTS_HEADER`, `MANAGE_EXILES_HEADER`, `MANAGE_OCCUPIED_TERRITORIES_HEADER`),
  `political_ideas_window.{title,switch}` (placeholders), `idea_group.{title,limit}`,
  `occupation_policy` (`OCCUPATION_POLICY`), `subject_item`/`exile_item`/`collaboration_item` label
  texts, `player_icon` (`GIE_PLAYER_CONTROLLED` tooltip).
- **Static icons:** `pol_power_icon`, `goal_icon` glow set, `icon_manage_subjects`,
  `icon_exiled_collaborations_governments`, `icon_occupied_territories`, `create_faction_icon`,
  `view_faction_icon`, `category_header`, `manpower_icon` (exile manpower), the exile reward-icon set
  (when not bound), `occupation_policy_overlay`.
- **Positioning anchors (positionType, no visual):** `faction_name_position`.
- **Navigation / window controls:** `close_button` / `close` (×several — close the view/modals).
- **Structural containers (positioning only):** `countrypoliticsview`, `chart_explanation`,
  `political_pie_chart`, `ruling_party_info`, `faction` (+ `no_faction`/`in_faction`/
  `original_in_faction`), `power_balance_button`, `active_goal`, `national_spirit`(+`_container`),
  `ideas`, `subjects_button_container`, the template ROOTS (`country_politics_idea_category_entry`,
  `political_ideas_window`, `idea_group`, `political_idea_entry`, `industrial_org_entry`,
  `political_selectable_idea_entry_grid`/`_list`, `political_party_info_entry`, `manage_subjects_window`,
  `subject_item`, `exiles_window`, `collaboration_item`, `exile_item`, `occupier_flag_entry`,
  `manage_occupied_window`, `occupied_territory_entry`, `occupy_policy_entry`) when considered as
  containers (their bound children are in 1a). The `idea_group.box_list`/`box_grid` /
  `political_ideas_window.grid` are picker grids (deferred §1c).

---
---

# VIEW 2 — POWER BALANCE (`game_assets/gui/power_balance.json`)

Root `powerbalanceview` + two top-level prototype icons (`range_bar`, `range_indicator`). A
single balance-of-power TRACK (e.g. a country's own internal BoP, like the French or Spanish
balance) rendered as a needle on a frame. Source = the `world.power_balances[]` entry whose
`countries[]` contains `<TAG>` (or the selected BoP track). Formatters: `loc`, `percent`,
`progressbar-fraction` (needle position), `category` (side/range token→loc).

## 2a. Power balance — dynamic (data-bound) elements

| gui element (name + type, sprite/text) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `power_balance_value` (iconType, no sprite — the FILL) | `world.power_balances[<i>].value` | progressbar-fraction (−1..1 → bar) | **exact** | The continuous BoP value (`value`, S; e.g. `−0.35`). Drives the bar fill. |
| `position_marker` (iconType, `GFX_bop_needle`) | `world.power_balances[<i>].value` | progressbar-position (−1..1) | **exact** | The needle position = the same `value` mapped onto the bar (left↔right). |
| `range_indicator` (iconType, `GFX_bop_indicator`) | active RANGE band of `value` (which side-range it sits in) | category (range→position) | **unavailable** | Which scripted side-RANGE the value currently falls in (and thus where the range marker sits) needs the per-track range thresholds (scripted side-DEF data); only the raw `value` is stored. Badge unavailable. |
| `power_balance_name` (instantTextboxType, `"Left Power vs. Right Power"`) | `world.power_balances[<i>].{left_side,right_side}` → loc | loc | **exact** (sides) / **derived** (loc) | The "X vs Y" header; `left_side`/`right_side` (S) are side tokens (`"DEN_welfare_side"`/`"…warfare_side"`); the localised "vs" string is a lookup. |
| `active_range_name` (instantTextboxType, `"We Are Very Powerful"`) | active side-range token → loc | category (range→loc) | **unavailable** | The named CURRENT range ("We Are Very Powerful") is the scripted range the value falls in — same gap as `range_indicator` (range thresholds not stored). Badge unavailable. |
| `left_power_icon` (iconType, no sprite) | `world.power_balances[<i>].left_side` → side icon | category (side→sprite) | **derived** | Left-side emblem from the side token (side-DEF icon lookup). |
| `right_power_icon` (iconType, no sprite) | `world.power_balances[<i>].right_side` → side icon | category (side→sprite) | **derived** | Right-side emblem. |
| `decision_grid` (gridboxtype, in `decision_container`) → BoP-decision tiles | the track's side decisions (BoP decision-DEF) | row factory; category | **unavailable** | The per-range BoP DECISIONS (the clickable side-effects) are decision-DEF + scripted availability not stored (the active timed ones could come from `decisions.timed_live` but BoP decisions are one-shot side picks) → grid empty; the tiles are mutations (inert). |

> **`trending_side`.** SCHEMA.md §18 documents `world.power_balances[].trending_side`; it is **absent
> in the demo snapshot** (only `template`/`value`/`left_side`/`right_side`/`countries` present). There
> is no dedicated `.gui` element for a trend arrow in this parsed view, so it is not a missed binding —
> noted for completeness. Were a trend arrow added it would be `exact` once `trending_side` is fed.

## 2b. Power balance — STATIC / decoration / navigation (no data binding)

- **Chrome / frames:** `range_bar` (`GFX_bop_splitter` — the prototype tick-splitter), `background`
  (`GFX_tiled_plain_bg`), `bop_bg` (`GFX_bop_background`), `power_balance_header_bg` (`GFX_header_bg`),
  `power_balance_frame` (`GFX_bop_bar_frame` — the bar frame around `power_balance_value`),
  `Background` (`GFX_tiled_window2_1b_border` + `GFX_tiled_decisions_bg_small`).
- **Static label text (loc / placeholder):** `title` (`"Balance of Power"`).
- **Navigation:** `close` (close the view — read-only verb).
- **Structural containers:** `powerbalanceview`, `decision_container`.

---
---

# VIEW 3 — COMMAND / CHARACTER detail (`game_assets/gui/command_character.json`)

The 1.19 unified character/leader detail (`command_character_view`). Root
`command_character_view_window` + two row templates (`command_character_view_item`,
`command_character_view_item_purge`). A grid of CHARACTERS (the country's commanders / advisors /
council) with name, traits, cost, stats. Observed country = `<TAG>`. Maps onto the **commanders**
domain (§3.9): the LIVE `commanders.{army_leaders,navy_leaders}` (M) + the save
`commanders.{leader,generals,field_marshals,admirals,advisors}` (S). Formatters: `loc`, `int`,
`category` (trait token→loc/icon), `idea-trait-frame`, `portrait`.

## 3a. Command/character — dynamic (data-bound) elements

| gui element (name + type, sprite/text) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `box_grid` (gridBoxType, in `ideas`→`grid`) → `command_character_view_item` rows | `countries.<TAG>.commanders.{army_leaders,navy_leaders}` (M) ∥ `commanders.{generals,field_marshals,admirals,advisors}` (S) | row factory (see row children) | **derived** (live) / **unavailable** (current data) | One tile per character. **LIVE `army_leaders`/`navy_leaders` (M) carry char_id/name/skills/xp** so the roster binds when the live commanders reader ran; in the demo SAVE-only snapshot the `commanders` domain is unfed → unavailable. **Codex precision:** only the **role split** (general vs field-marshal) is RESISTED in the live channel; the overall **skill/level + traits ARE schema-present** as `army_leaders.*.{skill,traits}` / `navy_leaders.*.{skill,traits}` (M) — **exact when the `leader_detail` enrichment is folded, unavailable when it isn't** (not globally resisted). |
| ↳ `command_character_view_item.name` (instantTextboxType, in `traits_container`) | per-row leader `name` | loc | **derived** | Character name (live `army_leaders[].name`, exact-raw; the displayed string is the name as measured). |
| ↳ `command_character_view_item.traits` (instantTextboxType, in `traits_container`) | per-row leader `traits[]` (`army_leaders.*.traits`, M, leader_detail enrichment) → loc | category (trait→loc) | **exact** (tokens, enrichment on) / **unavailable** (enrichment off) | Trait list; the trait TOKEN set is an OPT-IN M enrichment (953/953 EXACT when on) — **exact when folded**, not merely derived; absent → unavailable. Token→loc display is a static lookup. Save fallback `commanders.*.traits` (S). |
| ↳ `command_character_view_item.idea_traits` (iconType, `GFX_idea_traits_strip`) | per-row leader trait → trait-frame | idea-trait-frame | **derived** | Trait-icon frame (idea/trait-DB index). |
| ↳ `command_character_view_item.icon` (iconType, `GFX_placeholder_bordered`) | per-row leader `portrait_gfx` | portrait (sprite name→asset) | **unavailable** (current data) | Portrait sprite NAME exists on the save commander record (`portrait_gfx`, S) but is unfed here + portrait rendering is an asset step → placeholder. |
| ↳ `command_character_view_item.stats` (instantTextboxType) | per-row leader `skill` (M, leader_detail) — overall; per-stat attack/defense/planning/logistics (army) or maneuvering/coordination (navy) on SAVE `commanders.*` | int | **exact** (overall skill, enrichment on) / **unavailable** (per-stat split / enrichment off) | `army_leaders.*.skill` / `navy_leaders.*.skill` (M, leader_detail, 953/953 EXACT) gives the overall skill — **exact when folded**; the per-stat army/navy split is SAVE-only (`commanders.*.{attack,defense,…}`, S, unfed in demo). |
| ↳ `command_character_view_item.cost` (instantTextboxType) | character recruit/appoint cost (CP/PP) | int | **unavailable** | Recruitment cost is a character-DEF + runtime value, not stored on the commander record. |
| `command_character_view_item_purge` row | same as `command_character_view_item` (the PURGE variant) | — | **derived/unavailable** | The purge-mode tile binds the same character fields; the purge ACTION is a mutation (inert). It has no `cost` element. |
| `filter_grid` (gridBoxType, in `header`→`filters`) → filter tiles | character-type filter set (static UI categories) | row factory; category | **static-set** | The character-type filter buttons (commanders / advisors / council) are a fixed UI set, not data; counts (if shown) would derive from the roster partition. |

## 3b. Command/character — STATIC / decoration / navigation / mutation (no binding)

- **Chrome / frames:** `Background` (×6 — `GFX_tiled_plain_bg`, `GFX_tiled_window2_1b_border`,
  `GFX_tiled_generic_overlay_bg1_small`, `GFX_overview_top_strip_small`), the row template
  `Background`s + `button` (`GFX_idea_entry_bg_2` — the tile background button, a read-only select
  verb).
- **Static label texts (placeholders):** `title` (`"Characters Place Holder"`), `switch`
  (`"Switch out Character Place Holder"`).
- **Navigation / mutation (inert):** `close` (close view), `action_one` / `action_two` /
  `action_center_place_holder` (the bottom action buttons — SWITCH / recruit / purge = mutations,
  inert), the row `button` (open-character read-only verb).
- **Structural containers:** `command_character_view_window`, `ideas`, `grid`, `header`, `filters`,
  `header_background`, `traits_container`, and the row-template ROOTS
  (`command_character_view_item`, `command_character_view_item_purge`).

---
---

# VIEW 4 — ADVISOR-ROLE SELECTION (`game_assets/gui/advisor_role_selection.json`)

Root `advisorroleselectionview` + the row template `advisor_role_entry`. A small modal: pick which
advisor ROLE/slot a chosen character fills. The header shows the selected CHARACTER; the grid lists
the applicable advisor roles. Maps onto the **commanders** domain (the chosen leader) + the country's
advisor SLOTS. Formatters: `loc`, `int`, `category` (role/idea token→loc+icon), `portrait`.

## 4a. Advisor-role selection — dynamic (data-bound) elements

| gui element (name + type, sprite/text, loc key) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `leader.leader_icon` (iconType, `GFX_leader_unknown`) | the selected character's `portrait_gfx` (`commanders.*` / `advisors[].portrait_gfx`) | portrait (sprite name→asset) | **unavailable** (current data) | The chosen-character portrait; `commanders`/advisor portrait_gfx is a sprite NAME (S) unfed here + asset step → placeholder. `clip_icon`/`skill_tooltip_icon` are static chrome. |
| `leader.name` (instantTextboxType, `"name"`) | selected character `name` | loc | **derived** (live) / **unavailable** | The character's name (live `army_leaders[].name` when this is a unit-leader being appointed; else the save commander record). |
| `leader.title` (instantTextboxType, `"Mr."`) | character honorific/title | loc | **unavailable** | The honorific prefix is a character-DEF string not on the commander record. |
| `leader.skill` (instantTextboxType, `"skill"`) | selected character `skill`/level | int | **derived** (live, leader_detail) / **unavailable** | `army_leaders.*.skill` (M, leader_detail) gives skill; else unavailable. |
| `leader.divisions_count` (instantTextboxType) | divisions this leader commands | int | **unavailable** | The commanded-division count needs the `order_of_battle`→leader join (OOB domain unfed here). Badge unavailable. |
| `roles_grid` (gridBoxType, in `roles`) → `advisor_role_entry` rows | the country's advisor ROLE/slot set (`countries.<TAG>.commanders.advisors[].slot`, S) + `politics.cabinet` slot occupancy (S) + `politics.appointed_advisors` (M, filled) | row factory (see below) | **derived** (known role tokens) / **unavailable** (per-character eligibility) | **Codex fix:** NOT all unfeedable — the advisor ROLE rows can be PARTIALLY bound from `commanders.advisors[].slot` (S, known slot/role tokens) + the cabinet occupancy. What stays **unavailable** is the per-character ELIGIBILITY/selectability (character traits × scripted `can_be_X`) — a runtime computation — and the `select_button` ACTION (mutation). |
| ↳ `advisor_role_entry.select_button` (buttonType, `GFX_button_261x34`) | the role token → display name (`commanders.advisors[].slot`/ idea_token → loc) | loc | **derived** (role name) / **unavailable** (selectability) | Role name derivable from the advisor slot/idea token (S, advisor-DB lookup); the button itself is a mutation (assign role — inert). |
| ↳ `advisor_role_entry.idea_icon` (iconType, `GFX_idea_slot_high_command`) | the role's slot-category → frame (`commanders.advisors[].slot` / idea-DB) | category (slot→frame) | **derived** | Slot icon from the slot/idea token (advisor-DB index). |

## 4b. Advisor-role selection — STATIC / decoration / navigation / mutation (no binding)

- **Chrome / frames:** `Background` (×3 — `GFX_leader_selection_bg`, `GFX_leader_selection_entry_bg`,
  `GFX_tiled_window_transparent`), `leader_icon`/`clip_icon`/`skill_tooltip_icon` chrome.
- **Static label texts (loc):** `header_text` (`UNIT_LEADER_CHOOSE_ADVISOR_ROLE`), `roles_text`
  (`UNIT_LEADER_APPLICABLE_ADVISOR_ROLES`).
- **Navigation / mutation (inert):** `close_button`, `advisor_role_entry.select_button` (assign — mutation).
- **Structural containers:** `advisorroleselectionview`, `leader`, `roles`, `advisor_role_entry`
  (row-template root).

> **Whole-view note.** This is a per-character ASSIGNMENT picker = a mutation surface. In a replay the
> modal never opens. Per the Codex fix the role rows are **partially derivable** (role name + slot icon
> from `commanders.advisors[].slot`, S) — NOT wholly unavailable; what is `unavailable` is the
> per-character ELIGIBILITY filter (which roles THIS character may take = a runtime computation) and
> the `select_button` action (mutation). The header character fields are `derived`/`unavailable` per the
> commanders-domain state (unfed in the demo snapshot). Mapped in full for completeness.

---
---

# VIEW 5 — OCCUPATION (`game_assets/gui/occupation.json`)

Root `countryoccupationview` + the occupied-list row templates
(`occupied_territory_country_entry` → nested `state_grid` → `occupied_territory_state_entry` /
`occupied_territory_state_entry_no_resistance`), the resistance/compliance status containers (4 size
variants), the modifier-tile template (`occupation_modifier_entry`), the law-SELECT modal
(`select_occupation_setting_container` + `select_law_entry_container`), and the garrison-template
SELECT entry (`select_garrison_entry_container`). Observed (occupier) country = `<TAG>`; per occupied
state id = `<sid>`. Maps onto `states.<sid>.occupation.*` + `.occupation_law` (§4). Formatters:
`int`, `percent`, `manpower` (compact), `flag`/`flag-small`, `category` (law→frame),
`progressbar-fraction`.

## 5a. Occupation — dynamic (data-bound) elements

### Country-group rows (`occupied_territory_country_entry`, one per occupied nation)

| gui element (name + type, sprite/text) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `occupation_territories_list` (gridboxtype, root) → `occupied_territory_country_entry` rows | grouping adapter over `states.*` where `controller==<TAG>` ∧ `owner≠<TAG>`, bucketed by `states.<sid>.occupation.occupied_country_tag` (∥ `owner`) | row factory | **derived** | One row per occupied nation. The grouping is an adapter over the states domain; membership is exact, the per-nation bucket is a derive. |
| ↳ `flag` (buttonType, `GFX_flag_small2`, in `flag_container`) | the occupied nation tag (group key) | flag-small | **exact** | Occupied-nation flag. `flag_frame` static. |
| ↳ `country_name` (instantTextboxType, `"Country Name"`) | occupied nation tag → loc | loc | **exact** | Occupied-nation name. |
| ↳ `occupied_state_count` (instantTextBoxType, `"10/20"`) | count of occupied states / total states owned by that nation | int (`n/m`) | **derived** | Occupied-vs-total state count for the nation; both counts derivable from `states.*` (controller vs owner). |
| ↳ `manpower` (instantTextBoxType, `"+150K"`, in `state_info_container`) | aggregate occupation MANPOWER gain from the nation's occupied states | manpower | **unavailable** | The manpower YIELD from occupation is a computed value (state manpower × occupation-law factor × compliance); not stored as an occupation aggregate. `states.*.manpower_pool` exists but the occupation-scaled gain is computed. Badge unavailable. |
| ↳ `industrial_capacity` (instantTextBoxType, `"10/25"`) | occupied IC available / total for the nation | int (`n/m`) | **unavailable** | Occupied-IC yield is a computed factory-control split (building level × compliance × law); not stored. |
| ↳ `resources` (instantTextBoxType, `"50%"`) | occupied resource-access fraction | percent | **unavailable** | Occupation resource-access % is computed; not stored. |
| ↳ `selected_law` (instantTextboxType) + `select_law_icon` (iconType, `GFX_occupation_policy_icon_strip`) | the nation-level default `states.<sid>.occupation_law` (S) | category (law→loc/frame) | **exact** (token) / **derived** (frame) | The country-default occupation law; `occupation_law` (S) token is exact, the frame/loc is a law-DB lookup. **`invalid_default_law`** (`GFX_mapicon_unit_no_orders`) shows when no uniform default → visibility-derived. |

### Per-state rows (`occupied_territory_state_entry`, nested under each country, one per occupied state)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `state_grid` (gridboxtype, in `occupied_territory_country_entry`) → `occupied_territory_state_entry` rows | the nation's occupied state ids (`states.<sid>` with `controller==<TAG>`, `owner==`nation) | row factory | **exact** | One row per occupied state. The `_no_resistance` variant is used when the state has zero resistance (a render-mode choice on the same `occupation` data). |
| ↳ `state_name` (instantTextboxType, `"State Name"`) | `<sid>` → state name (static state meta) | category/loc | **exact** | State display name from `map_pipeline/state_meta` keyed by id (static map metadata, not in the snapshot). |
| ↳ `select_law_icon` (iconType, `GFX_occupation_policy_icon_strip`) | `states.<sid>.occupation_law` | category (law→frame) | **exact** (token) / **derived** (frame) | Per-state occupation-law frame; `occupation_law` (S) is exact, the frame is a law-DB lookup. **Unavailable when the save channel isn't folded** (live channel does not carry the law). |
| ↳ `occupied_territory_state_entry_no_resistance.state_info` (instantTextboxType, `"There is no resistance…"`) | visible iff `states.<sid>.occupation.resistance==0` | static-loc + visibility | **derived** | The "no resistance" label shows for zero-resistance states (resistance is exact M/S). |

### Resistance / compliance status containers (4 size variants — bound by the occupation values)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `{large,medium}_resistance_status_container.progress` (iconType bar) | `states.<sid>.occupation.resistance` / 100 | progressbar-fraction | **exact** | Resistance is a 0..100 percent (M/S LIVE, EXACT vs save). Bar fraction = value/100. The size variant (large/medium) is a layout choice. `bg` is static chrome. |
| `{large,medium}_resistance_status_container.progress_text` (instantTextBoxType, `"100%"`) | `states.<sid>.occupation.resistance` | percent | **exact** | e.g. `4.5%`. |
| `{large,tiny}_compliance_status_container.progress` (iconType bar) | `states.<sid>.occupation.compliance` / 100 | progressbar-fraction | **exact** | Compliance 0..100 (M/S EXACT). |
| `{large,tiny}_compliance_status_container.progress_text` (instantTextBoxType, `"100%"`) | `states.<sid>.occupation.compliance` | percent | **exact** | e.g. `97%`. |
| `medium_resistance_status_container.resistance_target` (iconType, `GFX_occupation_resistance_target_line`) | resistance TARGET (equilibrium) for the state | progressbar-position | **unavailable** | The resistance-target line = a computed equilibrium (resistance_target modifier stack); `occupation.resistance_speed` (S) exists but the target VALUE is a computed aggregate not stored. Badge unavailable. |
| `{…}_status_container.modifier_container` (gridboxtype) → `occupation_modifier_entry` rows (`icon`) | per-state resistance/compliance MODIFIER icons | category (modifier→icon) | **unavailable** | The per-state occupation-modifier list (what's pushing resistance/compliance) is not in the schema (`occupation` carries only the scalars + speeds + law). Badge unavailable. |

## 5b. Occupation — `unavailable` / deferred-mutation (per-element reasons)

| gui element (name + type) | intended source | fidelity | reason |
|---|---|---|---|
| country-row `manpower` / `industrial_capacity` / `resources` | occupation yields | manpower/int/percent | **unavailable** | computed occupation yields (manpower × law × compliance, etc.) not stored. |
| `foreign_support` block (`foreign_support_amount` `"+29.4K"`, `manpower_icon`, `name_bg`) | foreign garrison support manpower | manpower | **unavailable** | Foreign/collaborator garrison-support manpower is a computed pool not stored. |
| `resistance_target` line / `modifier_container` rows | resistance equilibrium / modifier list | — | **unavailable** | computed equilibrium + modifier list not stored. |
| `garrison` block everywhere (`garrison_template_button`, `garrison_template_icon`, `change_template_icon`, `bar_str`, `template_need_text` `42`) | assigned garrison template + need/strength | int / bar | **unavailable** + **mutation** | The garrison TEMPLATE assignment is a mutation (PLAN §D, DISABLED → inert); `template_need_text`/`bar_str` are computed garrison requirement/fill (not stored). Per the brief: garrison templates **mutation→inert**. |
| `select_law_button` / `selected_law` / `select_law_icon` (when law unfed) | `states.<sid>.occupation_law` | category | **unavailable** (when save channel absent) | The law TOKEN is `occupation_law` (S) — exact when the save channel is folded, **unavailable** in a live-only snapshot. The select_law BUTTON is a mutation (inert). |
| `select_occupation_setting_container` modal (`setting_list` → `select_law_entry_container` rows: `selected_law`, `modifier_resistance_target` `+10%`, `modifier_compliance_gain` `+10%`, `modifier_required_garrison_factor` `+10%`) | selectable occupation-law catalog + per-law modifiers | category/percent | **unavailable** | The law-CHOICE modal lists every selectable law + its resistance/compliance/garrison modifiers = law-DEF/law-DB data not in the snapshot; the whole modal is a mutation surface (set law). |
| `select_garrison_entry_container` modal (`select_garrison_name`, `manpower_value` `24.0`, `IC_value` `24.0`, `softness_value` `24.0`) | selectable garrison templates + their costs | int | **unavailable** + **mutation** | Garrison-template SELECT list = template-DEF + computed costs not stored; mutation surface. |

## 5c. Occupation — STATIC / decoration / navigation / mutation (no binding)

- **Chrome / frames / bgs:** `Background` (×9 — `GFX_tiled_bg`, `GFX_tiled_window_transparent`,
  `GFX_occupation_country_entry_bg_*`, `GFX_occupation_state_entry_bg(_no_resistance)`,
  `GFX_generic_background`), `header_bg` (`GFX_header_bg` + `GFX_occupation_header_bg` +
  `GFX_occupation_select_law_header`), the entry `top_bg`/`top_bg_locked`/`bottom_bg`/`mid_bg*`
  containers, the status-bar `bg` icons (`GFX_occupation_*_resistance/compliance_bar_bg`),
  `occupation_icon` (the `GFX_occupation_occupation_icon` state markers), `manpower_icon` /
  `industrial_capacity_icon` / `resources_icon` (label icons for the country-row counters),
  `occupation_modifier_entry.bg`.
- **Static label texts (loc):** `title` (`COUNTRY_OCCUPATION_TITLE`), `garrison_template_text`
  (`MAIN_GARRISON_TEMPLATE`), `foreign_support_text` (`FOREIGN_SUPPORT`),
  `show_non_resisting_label` (`OCCUPATION_SHOW_NO_RESISTING_COUNTRIES`), the 5 `sort_button_*`
  texts (`OCCUPATION_SORT_*`), the select-modal `title` (`COUNTRY_OCCUPATION_TITLE`),
  `select_law_entry_container` modifier icons (`resistance_icon`/`compliance_icon`/`manpower_icon`).
- **Sort / filter / nav (read-only or UI-state verbs, no datum):** the 5 `sort_button_0..4`
  (sort the list — UI control), `resistance_tab_button` / `compliance_tab_button` (switch the
  status column — UI control), `show_non_resisting_container_checkbox` (filter toggle — UI state),
  `garrison_log_open` / `equipment_open` (open sub-views — read-only verbs), `expand` (expand a
  country row — UI control), `close_button` (×several).
- **Mutation buttons (inert):** `garrison_template_button` (everywhere), `select_law_button`
  (everywhere), `release_button` / `return_territory_button` (release/return territory — mutations),
  `clear_override` (clear law override — mutation), `select_garrison_button` (pick garrison —
  mutation), `occupation_policy*` selectors.
- **Positioning anchors (positionType, no visual):** `return_territory_button_pos_for_non_resistance`,
  `resistance_status_pos` / `compliance_status_pos` (×rows — where the adapter mounts the status
  containers), `state_grid_size_add`, `resistance_target_bounds`, `size_when_clear_button_visible`.
- **Structural containers:** `countryoccupationview`, `bg_cointainer`, `header`, `foreign_support`,
  `garrison`, `sort_buttons`, `show_non_resisting_container`,
  `occupation_territories_list_container(_bottom_margin)`, `state_info_container`, the status-container
  ROOTS (`large/medium_resistance_status_container`, `large/tiny_compliance_status_container`),
  `occupation_modifier_entry`, `select_occupation_setting_container`, `list_container`, and the row
  template ROOTS (`occupied_territory_country_entry`, `occupied_territory_state_entry(_no_resistance)`,
  `select_law_entry_container`, `select_garrison_entry_container`).

---
---

## Notes

**A. The LIVE upgrade (most consequential correctness point).** `schema.py` FIELD_SOURCES — which
postdates SCHEMA.md — promotes several politics leaves the OTHER binding maps treated as `S`-only to
**LIVE-first `M/S` EXACT**: `ruling_party`, `party_popularities` (→ the political pie chart + party
legend are **exact**, not derived-from-save), and the occupation `resistance`/`compliance` (→ the
occupation bars are **exact**). Three live-only leaves further close gaps SCHEMA.md left open:
`politics.{last_election, election_frequency, elections_allowed}` make the `elections` text **derived**
(next-election = last + frequency) rather than `unavailable`; `politics.appointed_advisors` (M) makes
the CABINET advisor tiles bindable; `diplomacy.{overlord, autonomy_state, autonomy_ratio}` (M) make the
`autonomy_progress_view` **exact**. The command/character grid likewise binds `commanders.{army_leaders,
navy_leaders}` (M) — but the **role split + overall level are RESISTED** unless the opt-in
`leader_detail` enrichment ran, and the whole `commanders` domain is **unfed in the demo SAVE-only
snapshot**, so the live-bindable rows show `unavailable` in current data.

**B. `unavailable` ≠ "missing forever".** Most `unavailable` badges here are (a) parser-unfed in THIS
channel — the `commanders` domain (leader portrait/name, command/character grid, advisor picker
character), `occupation_law` in a live-only snapshot — which become `exact`/`derived` once the domain
is folded; or (b) world-/scripted-DEF data not in a country snapshot (occupation-law catalog +
per-law modifiers, garrison-template catalog + costs, BoP side-range thresholds, focus-DEF cost, exile
legitimacy). Genuinely runtime-computed gaps (occupation manpower/IC/resource yields, resistance-target
equilibrium, garrison need/strength, occupation cost, foreign-support manpower, advisor eligibility)
stay `unavailable` by nature (PLAN §G). Every one renders "—" per no-fabrication, never `0`.

**C. Conditional `visible` / scripted-content badging.** Per the §C.4 "some scripted slots degrade"
directive, slots gated by a scripted condition we DON'T record (the politics MIO tile, the
collaboration/exile modals) get the explicit **"scripted content not captured"** badge — distinct from
a plain field gap — so the renderer shows the slot chrome with a badge rather than fabricating content.
Slots gated by a condition we DO record (`autonomy_progress_view` ← subject state; `faction` branch ←
faction presence; `power_balance_button` ← BoP track; `_no_resistance` state row ← resistance==0) are
resolved and bound.

---

## Element accounting (provably complete)

Counts below are over each parsed `.gui` tree's own `stats.element_type_counts` (layout primitives
`borderSize`, `margin`, `slotsize`, `max_slots`, `positionType` are the structural grid/text plumbing,
counted under "static/structural"). Every named element falls in exactly one bucket; the per-element
tables (1a–1d, 2a–2b, 3a–3b, 4a–4b, 5a–5c) enumerate them by name.

### POLITICS (`politics.json`) — type counts: containerWindowType 44, background 19, iconType 74, instantTextboxType 45, borderSize 13, gridBoxType 5, slotsize 13, buttonType 33, positionType 1, gridboxtype 8, max_slots 2, margin 5, OverlappingElementsBoxType 1 (= **263** elements)

| bucket | count (logical) | examples |
|---|---:|---|
| **Dynamic-bound** (incl. derived/unavailable badges) | **~30** | pie `chart` + `parties_grid` rows, `ideology`, `goal_icon`/active-goal title/progress, spirit grid, idea-category grid (+ category/idea row children), `faction_name`/`faction_icon`, BoP %, `autonomy_progress_view` (flag/overlord/level/bar), subjects grid rows, occupied-territories grid rows, `leader`/`leader_name` (unavailable) |
| **`unavailable` / scripted-not-captured** | **~10** | leader portrait+name, `focus_cost`, `power_balance_levels`, MIO tile (scripted), collaboration/exile modals (scripted), occupied cost, occupy-policy catalog |
| **Deferred: mutation pickers / modals** | **2 groups (~20 elements)** | law/idea/spirit picker (`political_ideas_window` + entries), management-modal mutation buttons |
| **Static / decoration / navigation / structural** | **~200** | 19 bgs, header/faction/spirit chrome, label texts, nav/close, 16 template/container roots, 13 borderSize, 13 slotsize, 5 margin, positionType |

### POWER BALANCE (`power_balance.json`) — type counts: iconType 9, containerWindowType 2, background 3, margin 1, gridboxtype 1, slotsize 1, buttonType 1, instantTextboxType 3 (= **21** elements)

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** (incl. unavailable) | **~7** | `power_balance_value`, `position_marker`, `power_balance_name`, `left_/right_power_icon`, `range_indicator`/`active_range_name` (unavailable), `decision_grid` (unavailable) |
| **Static / decoration / navigation** | **~14** | `range_bar`, `background`, `bop_bg`, `power_balance_frame`, header bg, `title`, `close`, 2 containers, margin/slotsize |

### COMMAND / CHARACTER (`command_character.json`) — type counts: containerWindowType 10, background 6, instantTextboxType 9, margin 3, gridBoxType 2, slotsize 2, buttonType 6, iconType 4 (= **42** elements)

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** (derived/unavailable in current data) | **~8** | `box_grid` rows (name, traits, idea_traits, icon, stats, cost ×2 row templates mapped once), `filter_grid` (static-set) |
| **Static / decoration / nav / mutation** | **~34** | 6 bgs, title/switch labels, close + 3 action buttons (inert), tile `button` verbs, containers/template roots, 3 margin, 2 slotsize |

### ADVISOR-ROLE SELECTION (`advisor_role_selection.json`) — type counts: containerWindowType 4, background 3, instantTextboxType 6, borderSize 2, buttonType 2, iconType 4, margin 1, gridBoxType 1, slotsize 1 (= **24** elements)

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** (incl. derived/unavailable) | **~7** | `leader.{leader_icon,name,title,skill,divisions_count}` (commanders-dependent), `roles_grid` rows (`select_button` role-name + `idea_icon`) — **partially derived** from `commanders.advisors[].slot` (S); per-character eligibility + select stay unavailable/mutation |
| **Static / decoration / nav / mutation** | **~17** | 3 bgs, `header_text`/`roles_text` labels, `close_button`, `select_button` (mutation), containers/template root, borderSize, margin, slotsize |

### OCCUPATION (`occupation.json`) — type counts: containerWindowType 27, background 9, iconType 45, instantTextboxType 13, borderSize 2, buttonType 25, margin 4, instantTextBoxType 17, gridboxtype 6, slotsize 6, positionType 8 (= **162** elements)

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** (incl. derived/unavailable badges) | **~22** | country-row (flag/name/state-count/law) + state-row (name/law) + 4 status-container progress+text + `occupation_territories_list`/`state_grid` factories + `_no_resistance` state_info |
| **`unavailable` / mutation-inert** | **~16** | country-row manpower/IC/resources, foreign_support, resistance_target, modifier_container rows, garrison block, law-select + garrison-select modals |
| **Static / decoration / nav / mutation / structural** | **~124** | 9 bgs, status-bar bgs, label texts, 5 sort + 2 tab + filter buttons, mutation buttons (garrison/release/return/clear/policy), 8 positionType, 4 margin, 6 slotsize, container/template roots |

### `unavailable` / scripted-not-captured ledger (with reason)

**Politics (~10):** `leader` portrait + `leader_name` (commanders unfed), `focus_cost` (focus-DEF),
`power_balance_levels` (scripted range count), `industrial_org_entry` MIO tile (scripted +
mios unfed), `collaboration_item`/`exile_item` (exile domain not modeled — scripted-not-captured),
`occupied_country_cost` (computed occupation cost), `occupy_policy_grid` catalog (law-DB).
*(`elections` is **derived** via the live `last_election`+`election_frequency` leaves — NOT
unavailable.)*

**Power balance (3):** `range_indicator` + `active_range_name` (scripted side-range thresholds not
stored), `decision_grid` (BoP decision-DEF + one-shot availability not stored).

**Command/character (~3):** `icon` portrait (asset + commanders unfed), `cost` (recruit cost not
stored), and the per-stat army/navy split (SAVE `commanders.*`, unfed in demo). *(`name`, overall
`stats` skill, and `traits` are **exact when the `leader_detail` enrichment is folded** — only the
role split is RESISTED; in the SAVE-only demo the whole `commanders` domain is unfed → unavailable
THERE, not by nature.)*

**Advisor-role (~3 hard):** `leader.title` (character-DEF honorific), `divisions_count` (needs OOB
join), and the per-character eligibility filter on `roles_grid` (runtime) + `select_button` action
(mutation). *(The role NAME + slot icon are **derived** from `commanders.advisors[].slot` (S); the
character header name/skill/portrait are commanders-dependent, unavailable only in the demo snapshot.)*

**Occupation (~16):** country-row manpower/IC/resources yields (computed), `foreign_support` manpower
(computed pool), `resistance_target` line (computed equilibrium), `modifier_container` rows (per-state
modifier list absent), the garrison block (mutation + computed need/strength), `occupation_law` in a
live-only snapshot (save-only leaf), the law-select + garrison-select modals (law-DB / template-DB +
mutation).

**Net:** all `unavailable`/scripted-not-captured elements are parser-unfed-in-this-channel,
world-/DEF-data not in a country snapshot, runtime-computed, or mutation surfaces. None are fabricated;
each renders "—" (or a "scripted content not captured" badge) per the no-fabrication rule, so every
view's `.gui` tree is provably 100% accounted for.

---

## Historical review round (adversarial review, 2026-06-26)

_Historical note: this records a completed review from the old Claude-era workflow.
Codex-native sessions must use normal sub-agents/partner review and must not run
the old local CLI command. The review attacked (a) missed elements, (b) field-path
correctness vs `schema.py` FIELD_SOURCES, (c) false `unavailable` vs real gap._

**Missed elements:** Codex found **no omitted named element** — it confirmed every named widget in the
five inventories is bucketed exactly once; the real defects were wrong aggregate paths and over-broad
`unavailable`/`RESISTED` badges (below).

**AGREED (8 core claims, no change):** `politics.party_popularities = M/S` → pie `chart` + `parties_grid`
data **exact** (names/colours derived lookups); `politics.ruling_party = M/S` exact; `politics.appointed_advisors = M`
→ cabinet advisor tiles derived from exact tokens; `elections` defensibly **derived** from `last_election +
election_frequency` (must branch on `elections_allowed` — already noted); `autonomy_progress_view` →
`diplomacy.{overlord,autonomy_state,autonomy_ratio}` (M) exact with `politics.autonomy` (S) fallback;
command/character grid live-bindable from `commanders.{army_leaders,navy_leaders}`; `states.*.occupation.{resistance,compliance} = M/S`
→ bars/text **exact**; `states.*.occupation_law = S` → exact token when save folded, unavailable live-only;
`world.power_balances` = `S` with `value`→needle exact and range names/markers unavailable (sub-keys are
keys INSIDE the sourced dict, not separate FIELD_SOURCES leaves).

**REFUTED → fixed (4):**

| # | Codex refute | fix folded in |
|---|---|---|
| 1 | `diplomacy_world.subjects[]` is **not** a FIELD_SOURCES leaf — the subjects/`subject_item` rows used a non-sourced path. | Re-sourced `subjects_grid` to an **adapter aggregation** over `countries.<SUBJ>.diplomacy.overlord==<TAG>` (M, 17/17 EXACT) with `politics.autonomy` (S) fallback; per-row autonomy now `diplomacy.autonomy_state`/`autonomy_ratio` (M, exact). Badge grid **derived** (was "exact"). |
| 2 | Commander overall **level/skill + traits are NOT globally "RESISTED"** — only the general-vs-FM **role split** is; `commanders.{army_leaders,navy_leaders}.*.{skill,traits}` are **M** (exact when `leader_detail` enrichment folded). | `box_grid` note + the `traits`/`stats` rows re-badged **exact when enrichment present / unavailable when absent**; "RESISTED" now scoped to the role split only. |
| 3 | politics `leader`/`leader_name` are **not inherently unavailable** — `commanders.leader` is **S**, exact when folded; unavailable only in the demo snapshot. | `leader_name` re-badged **exact once folded** (portrait derived from `portrait_gfx`); the `unavailable` is now explicitly "current data" (demo SAVE-only), not by nature. |
| 4 | Advisor `roles_grid`/`idea_icon` are **partially feedable** from `commanders.advisors[].slot` (S) for known role tokens — not all unavailable. | Role-name + slot-icon rows re-badged **derived** from `commanders.advisors[].slot`; only per-character **eligibility** + the `select_button` action stay unavailable/mutation. Whole-view note + ledger updated. |

**Net:** no badge was loosened without a schema path to back it; four were **tightened toward `exact`/`derived`**
where Codex showed a sourced leaf exists (subjects via live overlord; commander skill/traits via leader_detail;
save leader; advisor slots), and one wrong aggregate path (`diplomacy_world.subjects[]`) was corrected to a real
adapter aggregation. The highest-risk correctness calls — the LIVE `M/S` upgrades (pie chart, occupation bars,
ruling_party) and the occupation/autonomy paths — were all confirmed.
