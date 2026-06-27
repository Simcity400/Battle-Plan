# BINDING MAP — LEADERS / OFFICER CORPS / COMMANDER TRAITS (Phase-3 binding sweep)

> **What this is.** The COMPLETE, no-truncation data-binding map for eight in-game the game views the
> native replay UI binds in the Phase-3 sweep — the country **officer corps**
> (`game_assets/gui/officer_corps.json`, `interface/countryofficercorpview.gui`), the
> **unit-leader** roster/detail (`unit_leader.json`, `unitleaderwindow.gui`), the **navy-leader**
> fleet-assignment strip (`navy_leader.json`, `navyleaderview.gui`), the **leader-groups** map
> portraits (`leader_groups.json`, `leadergroups.gui`), the **aces** view (`aces.json`,
> `acesview.gui`), and the three **commander-trait-tree** surfaces — army
> (`army_trait_tree.json`, `armytraittreewindow.gui`), navy (`navy_trait_tree.json`,
> `navytraittreewindow.gui`), and the shared trait-tree prototype library (`trait_tree_common.json`,
> `traittreewindow_common.gui`). It realises GAME_UI_REPLICA_PLAN §C.4 ("Officer corps / high command
> / leaders / traits" row: grids + trait trees; thin adapter + trait-tree adapter; the exact 953/953
> leaders). EVERY element in each parsed `.gui` tree is accounted for — dynamic (data-bound),
> static/decoration (no binding), or deferred (needs an imperative adapter). Nothing is omitted; where
> a `.gui` element exists but our snapshot has no field for it, the element is listed and **badged
> `unavailable`** with the missing field named.
>
> **Binding-rule shape** (GAME_UI_REPLICA_PLAN §C.1): `{ target (gui element name+type, sprite/text,
> loc key), source (snapshot dotted field path), format (formatter id),
> fidelity (exact|derived|proxy|unavailable) }`. Source paths use SCHEMA dotted form:
> `countries.<TAG>.commanders.…` (see `replay/SCHEMA.md` §3.9/§3.11/§12 and `schema.py`
> `FIELD_SOURCES`). `<TAG>` = the player/observed country.
>
> **The three trait-tree views are GRAPH-pattern trees (§B.5), like the tech/focus trees.** The
> `.gui` supplies only the FRAME / the selected-leader header / tabs / the `drag_scroll` scroll
> canvases (`regular_traits`, `terrain_traits`, `advisor_traits`, the `lines` container) — **NOT the
> trait nodes themselves.** Trait nodes are instantiated by the **trait-tree adapter** from the
> trait CATALOG (`common/unit_leader/*.txt`: each trait's `parent`, `mutually_exclusive`, `prerequisites`,
> `gui_row`/`gui_column` layout) using the **shared prototypes** in `trait_tree_common.json`
> (`unit_leader_trait`, `disabled_trait`, `pickable_trait`, `picked_trait`, `advisor_trait`,
> `active_trait` + the `trait_tree_line_*` / `*_exclusive_*` connection tiles). Pure declarative
> binding **breaks** for the node graph (PLAN §C.2 "Graph + link lines" → adapter required). So per
> trait tree this doc has BOTH: (a) the standard per-element table for the chrome that IS in the
> `.gui`, and (b) a dedicated **NODE-BINDING** section mapping each trait node → the leader's
> `traits[]`. Node geometry + connection lines are flagged **adapter-fed**, not a declarative element
> bind.
>
> **No-fabrication (SCHEMA §0.2, PLAN principle 3):** `null` renders "—", never `0`. `0`/`0.0` is a
> measured zero. A field schema-present but never fed by the current parser is badged `unavailable`
> (not silently dropped). `portrait_gfx` is a sprite NAME (an **asset-extraction** step — the adapter
> resolves the named sprite from the game's `.gfx`/DDS; the snapshot never carries image data).

---

## Provenance of the fidelity calls — and the DEMO-SNAPSHOT ARTIFACT

Calls below are grounded in **empirical inspection of the real save-backed snapshot**
`replay/snapshots/replay/000017005619.snapshot.json` (`schema_version "1"`, date `1941.4.13.11`, GER
1941 save channel, 143 countries), cross-checked against `replay/SCHEMA.md` §3.9 / §3.11 / §12 and
`schema.py` `FIELD_SOURCES` + `DOMAINS`, and **adversarially reviewed by Codex** (read-only,
2026-06-26). Codex's verdict and the corrections folded in are recorded in the final section.

> **⚠ THE COMMANDERS DOMAIN IS ABSENT FROM THIS DEMO SNAPSHOT — and that is a DEMO ARTIFACT, not a
> schema gap.** Empirically verified: for the observed country GER (and every country) the snapshot's
> per-country keys are exactly `['decisions','diplomacy','economy','focus','intel','military',
> 'politics','research']`. The **`commanders` (§3.9), `extra` (§3.11), `abilities` (§3.15), and
> `order_of_battle` (§12)** keys are **not present at all** (`country.get('commanders')` ⇒ `None`).
> The demo snapshot was produced by a **SAVE-ONLY** channel that does not yet fold those domains. Every
> leader name / portrait / skill / stat / trait / assignment / ace below is therefore badged
> **`FOLD-IN/RECONCILE`** — schema-present and bind-ready, but **parser-unfed in THIS demo snapshot**.
> When a FULL LIVE snapshot folds the `commanders`/`extra`/`order_of_battle` domains (the live readers
> `commanders_live`/`country_extra`/`oob` already exist per `schema.py` DOMAINS), these become
> **exact/derived** with **no schema change**. This is the single most important framing of this map:
> the absences here are **NOT** true schema gaps (see the gap ledger's ARTIFACT column).

**Schema fields that WILL drive the binds once the domain is folded (from `replay/SCHEMA.md` §3.9 /
§3.11 / §12):**

| schema field | shape (per SCHEMA) | drives |
|---|---|---|
| `commanders.leader` | `{name, token, ideology, traits[], expire, portrait_gfx, char_id}` | country-leader portrait/name (officer corps header context; not the main element set here) |
| `commanders.generals[]` / `field_marshals[]` | each `{name, token, role, level(=skill), experience, attack, defense, planning, logistics, traits[], preferred_tactic, penalty, captured, unit_leader_id, assignment, portrait_gfx}` | the `armyleaderentry` rows + the army/FM detail header (`unit_leader.json`, `army_trait_tree.json`) |
| `commanders.admirals[]` | each `{… maneuvering, coordination …}` (navy stats) | the `navyleaderentry` rows + admiral detail (`navy_leader.json`, `navy_trait_tree.json`) |
| `commanders.advisors[]` | each `{name, token, slot, ledger, idea_token, political_power, traits[], portrait_gfx}` | the officer-corps **advisor/chief/theorist** tiles (`officer_corps.json`) |
| `commanders.counts` | `{generals, field_marshals, admirals, advisors, roster_size}` | roster summaries / sort headers |
| each leader's `assignment` | `{kind: army_group/army/fleet, name}` or `null` (reserve) | the leader→formation join (the `leader_groups.json` map portrait + the unit-leader "divisions_count"/assigned readouts); resolved from `unit_leader_id` (type 4713) via §12 `order_of_battle` |
| each leader's `traits[]` | token list | the trait-tree NODE state (picked vs pickable) in all three trait-tree views |
| each leader's `preferred_tactic` | token | the `preferred_tactic` panel in `officer_corps.json` + `army_trait_tree.json` |
| `extra.aces[]` | each `{name, surname, callsign, modifier, alive, kill_type, killer_country}` | the `acesview_entry` rows (`aces.json`) |
| `extra.ace_count` | int | aces count / header |
| `order_of_battle.{army_groups,armies,fleets}[].leader` | FM/general/admiral `id:type` ref → §3.9 | the assignment JOIN (which formation a leader commands) — feeds `leader_groups.json` |

**Static DEF / world data the trait-tree adapter must supply (NOT country state, NOT in any snapshot):**
the trait CATALOG (`common/unit_leader/*.txt`): each trait's display name (loc), icon sprite, `parent`/
`prerequisites`/`mutually_exclusive` edges, `gui_row`/`gui_column` geometry, and the
modifier/effect block (the "buff text"). These are flagged **STATIC DEF / adapter-fed** throughout.

---

# VIEW 1 — OFFICER CORPS (`officer_corps.json`, `countryofficercorpview.gui`)

**Scope.** The officer-corps view is the high-command / advisor / doctrine-spirit hub: per-branch
(army/navy/air/special-forces) **chief & theorist advisor tiles**, the **officer-corps spirit grids**
(army/navy/air spirit `gridboxtype` hosts + their per-spirit entry windows), the **doctrine buttons**
(opens the doctrine tree, with a track-status grid), the **preferred-tactic** selector, an
**army-HQ-templates** sub-window, and the **captured-generals** counter. The leader-tile templates
(`*_entry`, `*_spirit_window_item_grid`) are **row-factory prototypes** the adapter instantiates per
advisor/spirit. Observed country = `<TAG>`.

Formatters: `loc`, `int`, `percent`, `portrait-sprite` (named `portrait_gfx` → asset-extracted
sprite), `idea-trait-frame` (frame of `GFX_idea_traits_strip`), `advisor-type-frame` (frame of
`GFX_chief_and_theorist_strip`), `spirit-icon` (frame of `GFX_officer_corp_spirit_icon_strip`),
`category` (state→frame).

## 1a. Officer corps — dynamic (data-bound) elements

The advisor entries (`country_view_advisor_entry`, `high_command_entry`, `army_chief_entry`,
`navy_chief_entry`, `air_chief_entry`, `theorist_entry`) and the spirit-window item grids are
**ROW-FACTORY templates** — mapped once here; the adapter clones one per advisor / spirit. The
per-branch chief grids (`army_chief_advisors`, `navy_chief_advisors`, `air_chief_advisors`,
`high_command_advisors`, `theorist_advisors`) are `gridBoxType` hosts filled from
`commanders.advisors[]` (filtered by `slot`/`ledger`).

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `high_command_advisors` / `theorist_advisors` / `army_chief_advisors` / `navy_chief_advisors` / `air_chief_advisors` (gridBoxType) → **N × advisor entry** | row factory over `countries.<TAG>.commanders.advisors[]`, partitioned by `slot`/`ledger` (army/navy/air/high-command/theorist) | row factory (per-tile children below) | **FOLD-IN/RECONCILE** (would be **exact** count / **derived** partition) | One tile per cabinet advisor in that ledger. `advisors[]` schema-present (S); **unfed in this demo snapshot** (`commanders=None`). |
| ↳ `*_entry.button` (buttonType, `GFX_add_pol_idea_button` / `GFX_group_add_leader_bg`) | per-advisor `advisors[<i>].portrait_gfx` | portrait-sprite | **FOLD-IN/RECONCILE** | Advisor portrait. `portrait_gfx` is a sprite NAME (asset-extraction). |
| ↳ `*_entry.advisor_type_icon` (iconType, `GFX_chief_and_theorist_strip`) | per-advisor `advisors[<i>].slot` → frame | advisor-type-frame | **FOLD-IN/RECONCILE** | Chief/theorist category frame from the advisor's slot. |
| ↳ `*_entry.idea_traits` (iconType, `GFX_idea_traits_strip`) | per-advisor `advisors[<i>].traits[]` → frame | idea-trait-frame | **FOLD-IN/RECONCILE** | The advisor's trait icon strip; trait→frame is a static idea-DB map. |
| ↳ `*_entry.idea_alert_glow` (iconType, `GFX_ongoing_generic_glow_yellow`) | visible iff an empty hireable slot (no advisor in `advisors[]` for this slot) | visibility | **FOLD-IN/RECONCILE** (derived) | "You can hire here" glow; derivable from the advisor partition once folded. |
| ↳ `army_chief_entry.leader_text` / `navy_chief_entry.leader_text` / `air_chief_entry.leader_text` (instantTextboxType, placeholder `Name PlaceHolder`) | per-advisor `advisors[<i>].name` → loc | loc | **FOLD-IN/RECONCILE** | The chief's name. |
| `*_spirit_window_item_grid` (×9 branch templates: army/navy/air × command/force/academy + division_command) → `.icon` (iconType, `GFX_placeholder_bordered`) | per officer-corps spirit token, from `countries.<TAG>.politics.ideas_other[]` filtered to the officer-corps ledger → spirit sprite | spirit-icon | **derived** | Officer-corps spirits are IDEAS. **`politics.ideas_other[]` (S) IS PRESENT in this demo snapshot** (politics is one of the 8 folded keys) — the ORDERED active-ideas/spirits set. So these tiles are **derivable NOW** (not a commanders-domain artifact): filter `ideas_other` to the officer-corps spirit ledger + map token→spirit-strip frame via the spirit catalog (**STATIC DEF**). The ledger partition is the derive. |
| ↳ `*_spirit_window_item_grid.idea_traits` (iconType, `GFX_idea_traits_strip`) | per-spirit modifier-trait frame | idea-trait-frame | **FOLD-IN/RECONCILE** | Spirit's trait-strip frame. |
| ↳ `*_spirit_window_item_grid.traits_container.name` (instantTextboxType) | per-spirit token (from `politics.ideas_other[]`) → loc | loc | **derived** | Spirit name (loc of the active-spirit token; `ideas_other` present). |
| ↳ `*_spirit_window_item_grid.traits_container.traits` (instantTextboxType, maxHeight 100%%) | per-spirit modifier text (idea-DEF) | loc | **STATIC DEF / adapter-fed** | The spirit's modifier blurb is idea-DEF loc, not country state. |
| ↳ `*_spirit_window_item_grid.cost` (instantTextboxType, format right) | per-spirit PP/command-power cost (idea-DEF) | int | **STATIC DEF / adapter-fed** | Spirit cost is an idea-DEF constant. |
| ↳ `*_spirit_window_item_grid.stats` (instantTextboxType) | per-spirit aggregate stat (runtime/idea-DEF) | loc | **STATIC DEF / adapter-fed** | DEF/computed; not country state. |
| `*_spirit_entry.spirit_icon` (×9 small spirit-pip templates: `division_command_spirit_entry`, `army_spirit_entry`, `academy_spirit_entry`, `naval_command_spirit_entry`, `navy_spirit_entry`, `naval_academy_spirit_entry`, `air_force_command_spirit_entry`, `air_force_spirit_entry`, `air_force_academy_spirit_entry`) (iconType, `GFX_officer_corp_spirit_icon_strip`) | per-active-spirit token (`politics.ideas_other[]`, officer-corps ledger) → strip frame | spirit-icon | **derived** | The small active-spirit pips on each banner; `ideas_other` present → derivable now (ledger partition is the derive). |
| ↳ `*_spirit_entry.*_glow` (iconType, `GFX_spirit_glow`) | visible iff spirit active (∈ `politics.ideas_other[]`) | visibility | **derived** | Active-spirit glow. |
| `preferred_tactic_container.icon_prefered_tactic_type` (iconType, `GFX_attacker_damage_icon`) + `army_header…button_prefered_tactic_selection` (buttonType) | `commanders.generals[].preferred_tactic` (country-level chosen tactic) / per-leader | category (tactic→icon) | **FOLD-IN/RECONCILE** | The selected combat tactic icon. `preferred_tactic` is per-leader (§3.9); this header surfaces the country/army-chief choice. Unfed here. The tactic CATALOG (icon/effect) is **STATIC DEF**. |
| `modifiers_grid` (gridBoxType, in army/navy/air/special_forces headers) → per-branch officer-corps modifier tiles | branch aggregate officer-corps modifiers (idea/spirit stack) | percent + category | **FOLD-IN/RECONCILE** (derived) | The branch doctrine-modifier readout; an aggregate over the active spirits/advisors (unfed). |
| `<branch>_doctrine_button.number_container` + `track_status_grid` (gridBoxType) | doctrine-track progress for that branch | int / category | **FOLD-IN/RECONCILE** (derived) | The doctrine button shows how many doctrine steps are taken; doctrines are techs → `research.technologies[]` (note: doctrine techs are themselves **unfed** in this save channel per the tech/doctrine map). |
| `hq_ribbon.deployed_counter` (instantTextboxType) | count of deployed HQ units (`order_of_battle` HQ divisions) | int | **FOLD-IN/RECONCILE** | Deployed-HQ count; from the OOB HQ join (§12, unfed here). |
| `captured_generals_container.captured_generals_amount` (instantTextBoxType) | `countries.<TAG>.abilities.officer_corps.own_generals_captured` (D) / `captured_enemy_generals` | int | **FOLD-IN/RECONCILE** | Captured-generals counter. `abilities.officer_corps.*_captured` is `D` (§3.15); the whole `abilities` domain is unfed here. |
| `named_hq_template_entry.name` / `.deploy_time` / `.cp_value` / `counter` (instantTextboxType + buttonType) — HQ-template row factory | per-named-HQ-template (army-designer data) | loc / int | **unavailable** | Named army-HQ templates are division-designer/template data, not a `commanders` field; no per-template snapshot model. **TRUE GAP** (see ledger). |
| `track_progress_item.status_text` (instantTextBoxType) + `full_mastery_icon` (iconType, `GFX_neutral_experience`) | doctrine-track step status | loc / category | **FOLD-IN/RECONCILE** (derived) | Per-step doctrine-track status; derivable from the doctrine `technologies[]` (unfed). |

## 1b. Officer corps — STATIC / decoration / navigation (no data binding)

- **Structural containers / windows:** `countryofficercorpview` (root), `high_command`, `theorist`,
  `preferred_tactic_container`, `hq_ribbon`, `officer_core_view`, the four `<branch>_header`
  (`army_header`/`navy_header`/`air_header`/`special_forces_header`), the four `<branch>_chief`
  containers, the four `<branch>_doctrine_button` containers, `army_spirits_view`/`navy_spirits_view`/
  `air_spirits_view`, `prefered_tactics`, `spirit_window` + `ideas`/`grid`, `armyhqtemplatewindow` +
  `templates`/`div_templ_custom_icon_view`, and all the `*_entry` / `*_spirit_window_item_grid` /
  `*_spirit_entry` **template ROOTS** (their bound children are in 1a).
- **Backgrounds / frames / banners (static art):** every `Background` (`GFX_tiled_bg`,
  `GFX_tiled_window2_1b_border`, `GFX_tiled_generic_overlay_bg1`/`_small`, `GFX_army_tiled_marble_bg`,
  `GFX_navy_tiled_marble_bg`, `GFX_air_tiled_marble_bg`, `GFX_tiled_plain_bg`,
  `GFX_army_hq_template_bg`, `GFX_tiled_window_1b_thin_border`), `army_header_bg` (`GFX_header_bg`),
  `overview_bottom` (`GFX_trade_header`), `advisor_theorist_entry`, the per-branch `icon_banner`/
  `icon_army`/`icon_navy`/`icon_air` chief banners, `deployed_hq_icon` (`GFX_hq_badge_bg`), the
  doctrine-button `icon` + `number_container` art, the spirit `icon_background`/`icon_background_*`
  banner decorations, `template_header_bg`.
- **Static label text (loc constants):** `army_title` (`OFFICER_CORPS_LABEL`), each `<branch>_header.text`
  (`OFFICER_CORP_ARMY_DOCTRINE` / `_NAVY_` / `_AIR_` / `_SPECIAL_FORCES_DOCTRINE`),
  `captured_generals_text` (`CAPTURED_GENERALS`), `spirit_window.title` (`ARMY_ARMY_LABEL`),
  `template_title`/`templates_grid` headers (`ARMY_HQ_TEMPLATES`), `commander_roster_*` button labels
  (`COMMANDER_ROSTER_SHIP_BUTTON` etc.), `army_hq_templates_toggle` (`ARMY_HQ_TEMPLATES`).
- **Navigation / mutation buttons (read-only or inert verbs):** `close_button` / `close` (`CLOSE`),
  `button_army_hq_templates_toggle`, `button_captured_generals`, the `commander_roster_*` filter
  buttons, `button_prefered_tactic_selection` (tactic-PICK is a **mutation** → inert in a replay),
  `<branch>_doctrine_button.button` (opens doctrine tree — nav verb), `spirit_window.switch` /
  `close`, `armyhqtemplatewindow` `open_designer_button` / `edit_button` / `delete_button` / `back` /
  `default` (designer mutations — inert), the spirit-entry `button` backgrounds.
- **Positioning anchors (positionType, no visual):** `spirit_row_offset`, `no_taog_offset`. Plus the
  structural `margin` (8×), `slotsize` (19×), `max_slots` (3×) layout nodes.

---

# VIEW 2 — UNIT-LEADER ROSTER + DETAIL (`unit_leader.json`, `unitleaderwindow.gui`)

**Scope.** This file is the **leader-selection roster** (the "choose a leader" picker) for FIVE
contexts — army (`armyleaderwindow`/`armyleaderentry`), captured generals
(`capturedarmyleaderwindow`/`capturedarmyleaderentry`), navy (`navyleaderwindow`/`navyleaderentry`),
operative (`operativeleaderwindow`/`operativeleaderentry`), divisional commander
(`divisioncommanderwindow`/`divisionleaderentry`), and faction-commander
(`factioncommanderwindow`) — plus a few stray templates (`unit_leader_trait_entry`,
`unit_leader_exile_trait_entry`, `division_medal_entry`, `supreme_commander_type_item`). The
brief assigns this view to **general / field-marshal detail** (`commanders.generals[]` /
`field_marshals[]`); the army/captured roster ENTRIES are the load-bearing binds. The `*_entry`
windows are **ROW-FACTORY templates** (one per leader). Observed country = `<TAG>`.

Formatters: `loc`, `int`, `percent`, `portrait-sprite`, `skill-int` (the big skill number),
`stat-int` (attack/defense/planning/logistics), `flag-small`, `flag-smallest`, `trait-frame`
(frame of the trait strip), `category`.

## 2a. Unit-leader roster — dynamic (data-bound) elements (the ENTRY row factories)

The army-leader entry (`armyleaderentry`) is the canonical row; the captured / navy / operative /
divisional / faction entries repeat the same pattern with context-specific extras (mapped below by
delta). Each binds one element of `commanders.generals[]` / `field_marshals[]` (army),
`commanders.admirals[]` (navy), or the operative/division pools.

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `armyleaderwindow.leaders_grid` (gridBoxType) → **N × `armyleaderentry`** | row factory over `commanders.generals[] + field_marshals[]` (army roster) | row factory (children below) | **FOLD-IN/RECONCILE** (would be **exact** — the 953/953-leaders target) | One row per army leader. `generals[]`/`field_marshals[]` schema-present (S); unfed in this demo snapshot. |
| ↳ `armyleaderentry.leader_button` (buttonType, `GFX_leader_unknown`) | `generals[<i>].portrait_gfx` | portrait-sprite | **FOLD-IN/RECONCILE** | Leader portrait; sprite NAME (asset-extraction). `GFX_leader_unknown` is the placeholder. |
| ↳ `armyleaderentry.name` (instantTextboxType, placeholder `name`) | `generals[<i>].name` | loc/literal | **FOLD-IN/RECONCILE** | Leader name (already a display string in §3.9). |
| ↳ `armyleaderentry.title` (instantTextboxType, placeholder `Mr.`) | `generals[<i>].role` → loc (General / Field Marshal) | loc | **FOLD-IN/RECONCILE** | Rank title; `role` distinguishes general vs field-marshal. |
| ↳ `armyleaderentry.skill` (instantTextboxType, `hoi_30header`) | `generals[<i>].level` (=`skill`) | skill-int | **FOLD-IN/RECONCILE** | The big skill number. SCHEMA notes `level` IS the skill. |
| ↳ `armyleaderentry.skill_container.leader_stat_attack` (instantTextboxType) | `generals[<i>].attack` | stat-int | **FOLD-IN/RECONCILE** | Attack stat. |
| ↳ `armyleaderentry.skill_container.leader_stat_defense` (instantTextboxType) | `generals[<i>].defense` | stat-int | **FOLD-IN/RECONCILE** | Defense stat. |
| ↳ `armyleaderentry.skill_container.leader_stat_planning` (instantTextboxType) | `generals[<i>].planning` | stat-int | **FOLD-IN/RECONCILE** | Planning stat. |
| ↳ `armyleaderentry.skill_container.leader_stat_logistics` (instantTextboxType) | `generals[<i>].logistics` | stat-int | **FOLD-IN/RECONCILE** | Logistics stat (army). |
| ↳ `armyleaderentry.traits_grid` (gridBoxType) → trait pips | `generals[<i>].traits[]` → trait-strip frames | trait-frame | **FOLD-IN/RECONCILE** | The leader's trait icons. Trait→frame is a STATIC trait-DB map; the SET is `generals[].traits[]` (unfed). |
| ↳ `armyleaderentry.divisions_count` (instantTextboxType) | count of divisions under this leader (via `unit_leader_id` → §12 `armies[].members`) | int | **FOLD-IN/RECONCILE** | Divisions commanded; the OOB join (§12, unfed). |
| ↳ `armyleaderentry.country_flag` (buttonType, `GFX_flag_small2`) | observed `<TAG>` (the leader's country) | flag-small | **FOLD-IN/RECONCILE** | Country flag on the tile. |
| ↳ `armyleaderentry.army_hq_icon` (iconType, `GFX_hq_badge_bg`) + `clip_icon` + `leader_enroute` (`GFX_leader_en_route`) | per-leader assignment state (`generals[<i>].assignment`) | category / visibility | **FOLD-IN/RECONCILE** | HQ badge / en-route marker; from the leader's `assignment` (army-group/army/null). |
| ↳ `armyleaderentry.advisor_role_icon` (`GFX_empty_paper`) + `advisor_role_traits_icon` (`GFX_idea_traits_strip`) | per-leader assigned staff-office advisor role | category / idea-trait-frame | **FOLD-IN/RECONCILE** | The leader's high-command/advisor role overlay (officer-corps staff office). |
| ↳ `armyleaderentry.faction_commander` block: `mastery_value` / `supply_value` / `supply_navy_value` / `assigned_countries` / `assigned_regions` (instantTextBoxType, placeholders `0%`/`0/0`) | faction-commander mastery + supply + capacity (faction-leader feature) | percent / int | **FOLD-IN/RECONCILE** (derived/unavailable) | Faction-commander readouts (doctrine mastery %, supply-consumption, assigned countries/regions). Mastery/supply are **runtime aggregates**; assigned counts come from the faction-command join (not a per-leader §3.9 field) → `derived`/`unavailable` even once folded. **partial TRUE GAP** (mastery math). |
| **`capturedarmyleaderentry`** delta (vs army): `captured_overlay` (`GFX_captured_general_bars`), `captor_flag` (`GFX_flag_smallest`), `rescue_raid_*` | `generals[<i>].captured` (bool) + captor TAG | category / flag-smallest | **FOLD-IN/RECONCILE** | Captured-general bars + the captor's flag. `captured` is a §3.9 field; the captor TAG needs the capture join (OOB/intel). |
| **`navyleaderentry`** delta: `leader_stat_maneuvering` / `leader_stat_coordination` (instantTextboxType) | `admirals[<i>].maneuvering` / `.coordination` | stat-int | **FOLD-IN/RECONCILE** | Navy stats replace planning/logistics (per §3.9 navy shape). |
| **`operativeleaderentry`** delta: `mission_icon` (`GFX_operative_mission_icons_small`), `operation_icon`, `nationalities_grid`, `skill` | operative pool (intel) — `intel`/`commanders` operative records | category / int | **FOLD-IN/RECONCILE / partial GAP** | Operatives are an intel-domain pool, not §3.9 generals; the operative roster needs the operative join. `intel` IS present in this snapshot (top-level key) but the operative-character detail (name/skill/nationalities/mission) is a separate reach → mixed FOLD-IN + GAP. |
| **`divisionleaderentry`** delta: `division_name` / `gained_xp` / `unit_level` / `template_button` (`GFX_unit_infantry_icon_medium`) / `ship_icon_button` / `medal_dropdown`→`medals_grid` | divisional-commander roster (per-division leader + medals) | loc / int / category | **unavailable / partial GAP** | Divisional commanders + division medals are a per-division leader layer not modeled in §3.9 (which is the country high-command roster). **TRUE GAP** (divisional-commander roster). |

## 2b. Unit-leader roster — STATIC / decoration / navigation / sort (no data binding)

- **Structural windows / template roots:** the six window roots (`armyleaderwindow`,
  `capturedarmyleaderwindow`, `navyleaderwindow`, `operativeleaderwindow`, `divisioncommanderwindow`,
  `factioncommanderwindow`), the six entry-template roots (`armyleaderentry`, `capturedarmyleaderentry`,
  `navyleaderentry`, `operativeleaderentry`, `divisionleaderentry` + the bound children in 2a), the
  `leaders` / `leaders_grid` containers (gridbox hosts — adapter fills with entries), the stray
  templates `unit_leader_trait_entry`, `unit_leader_exile_trait_entry`, `division_medal_entry`,
  `supreme_commander_type_item`, `country_filter`/`type_dropdown`/`type_expanded_window` (faction
  country-filter), the `medal_dropdown`/`expanded_window` (division medals).
- **Backgrounds / frames:** every `Background` (`GFX_leader_selection_bg`,
  `GFX_leader_selection_entry_bg`, `GFX_naval_leader_selection_entry_bg`, `GFX_marshal_captured_bg`,
  `GFX_tiled_window_transparent`, `GFX_tiled_window2_1b_border`, `GFX_tiled_window_1b_border`/`_thin_`,
  `GFX_mini_tooltip`, `GFX_trait_unknown`), `country_flag_frame` (`GFX_diplo_countrylist_flag_frame`),
  the stat-bg icons (`GFX_trait_attack`/`_defense`/`_planning`/`_logistics`/`_maneuvering`/
  `_coordination`), `skill_tooltip_icon`, `medal_collection`, `portrait_overlay_bg`.
- **Static label text (loc constants):** the `header_text` of each window (`CHOOSE_LEADER`,
  `CAPTURED_GENERALS`, `CHOOSE_DIVISIONAL_LEADER`), `show_assigned_text` (`SHOW_ACTIVE`),
  `resume_mission_txt`, the tab labels (`CAPTURED_GENERALS_OURS`/`_THEIRS`), button labels (`UNASSIGN`,
  `UNIT_LEADER_NEW_LEADER`, `UNIT_LEADER_PROMOTE`, `UNIT_LEADER_TRAITS`, `DIVISION_DETAILS`, etc.).
- **Sort buttons (read-only verbs) + search (UI state):** every `sort_*` ButtonType
  (`sort_name`, `sort_skills`, `sort_advisor`, `sort_capacity`, `sort_attack_skill`, `sort_defense_skill`,
  `sort_planning_skill`, `sort_supply_skill`, `sort_maneuvering_skill`, `sort_coordination_skill`,
  `sort_experience`, `sort_unit_type`, `sort_medals`, `sort_total_exp`, `sort_faction_*` ×6),
  `leader_view_search` (editBox — search input), `show_assigned` / `resume_mission_cb` (checkboxType —
  filter UI), the `unit_level_sort`/`total_exp_sort` sort-icon decorations.
- **Mutation / nav buttons (inert in replay):** `no_leader_button` (UNASSIGN), `new_leader_button`,
  `show_divisional_commanders_button`, `show_ship_captains_button`, `promote_btn`, `traits_btn`
  (opens trait tree — nav), `generate_advisor_button`, `details_btn`, `rescue_raid_button`,
  `btn_army`, `template_button`/`ship_icon_button`, `expand_button`/`type_button`, `close_button`.
- **Positioning anchors (positionType):** `pos_roster_overview_orientation_top_left` (×5),
  `operative_portrait_pos`, `rescue_raid_text_pos_default`/`_eligible`. Plus the structural `margin`
  (8×), `borderSize` (14×), `slotsize` (14×) layout nodes.

---

# VIEW 3 — NAVY-LEADER FLEET STRIP (`navy_leader.json`, `navyleaderview.gui`)

**Scope.** A small (25-element) map-overlay strip: the **new-fleet portrait** picker
(`navy_leader_window` → `new_fleet_window`), the **admiral portrait window**
(`navy_leader_portrait_window`) with disband / assign-staff / assign-trait affordances and a
task-force counter, and the **reserve** portrait window (`navy_reserve_portrait_window`). Binds
`commanders.admirals[]` + the §12 `fleets[].leader` assignment join. Observed country = `<TAG>`.

Formatters: `portrait-sprite`, `int`, `category`, `visibility`.

## 3a. Navy-leader strip — dynamic (data-bound) elements

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `new_fleet_window.portrait_btn` (buttonType, `GFX_group_add_leader_bg`) | per-fleet admiral `admirals[<i>].portrait_gfx` (joined via §12 `fleets[].leader`) | portrait-sprite | **FOLD-IN/RECONCILE** | The admiral portrait for a fleet (the `fleets_box` grid factory iterates fleets). |
| `fleets_box` (gridBoxType) → per-fleet portrait tiles | row factory over `order_of_battle.fleets[]` (owner==`<TAG>`), each → its `leader` admiral | row factory | **FOLD-IN/RECONCILE** | One tile per fleet; `fleets[]` (§12) unfed in this demo snapshot. |
| `navy_leader_portrait_window.portrait_btn` (buttonType, `GFX_leader_unknown`) | selected admiral `admirals[<sel>].portrait_gfx` | portrait-sprite | **FOLD-IN/RECONCILE** | Selected-admiral portrait. |
| `navy_leader_portrait_window.taskforce_count` (instantTextboxType) | `len(fleets[<sel>].task_forces[])` (§12) | int | **FOLD-IN/RECONCILE** | Task-force count under this admiral. |
| `navy_leader_portrait_window.leader_enroute` (iconType, `GFX_leader_en_route`) | admiral `assignment` en-route state | visibility | **FOLD-IN/RECONCILE** | En-route marker. |
| `navy_leader_portrait_window.group_marker` (iconType, `GFX_group_marker2`) | fleet group marker id | category | **FOLD-IN/RECONCILE** | Map-group marker; from the fleet record. |
| `navy_reserve_portrait_window.taskforce_count` (instantTextboxType) | count of reserve (unassigned) admirals (`admirals[]` with `assignment==null`) | int | **FOLD-IN/RECONCILE** | Reserve-pool count; derivable from `admirals[].assignment==null`. |

## 3b. Navy-leader strip — STATIC / decoration / mutation (no data binding)

- **Structural windows:** `navy_leader_window`, `centering_box`, `new_fleet_window`,
  `navy_leader_portrait_window`, `navy_reserve_portrait_window`.
- **Frames / decoration:** `portrait_frame` (`GFX_leader_group_frame_unselected` / `GFX_navy_group_frame` /
  `GFX_reserve_frame`), `portrait_frame_hightlight` [sic], `portrait_highlighted_btn`
  (`GFX_LG_select_group_btn`), `navy_reserve_portrait_window.portrait_btn` (`GFX_reserve_portrait`).
- **Nav / mutation (inert in replay):** `btn_scroll_left` / `btn_scroll_right` (`GFX_floating_arrows` —
  scroll the fleet strip), `btn_disband_navy` (ButtonType, disband — mutation),
  `navy_assign_staff_office` / `navy_assign_trait` (assign-role/trait — mutations → inert).
- **Layout nodes:** `slotsize` (1×).

---

# VIEW 4 — LEADER-GROUPS MAP PORTRAITS (`leader_groups.json`, `leadergroups.gui`)

**Scope.** The **map-overlay command portraits** that float over the map next to selected
army/army-group leaders: the general portrait window (`leader_groups_portrait_window`) and the
army-group/field-marshal portrait window (`army_group_portrait_window`), plus the attached-unit
sub-icons (`group_wing_type_box`, `group_railway_guns_box`) and the battle-plan execute/training
machinery. Binds the §12 `order_of_battle.armies[]`/`army_groups[]` `leader` join +
`commanders.generals[]`/`field_marshals[]`. Observed country = `<TAG>`.

Formatters: `portrait-sprite`, `int`, `category`, `progressbar-fraction`, `visibility`.

## 4a. Leader-groups — dynamic (data-bound) elements

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `leader_groups_portrait_window.leader_icon` (iconType, `GFX_leader_unknown`) | per-army general `generals[<i>].portrait_gfx` (via §12 `armies[].leader`) | portrait-sprite | **FOLD-IN/RECONCILE** | The army's commanding general's portrait. |
| `leader_groups_portrait_window.pending_leader_icon` (iconType, `GFX_leader_unknown`) | a leader en-route to assume command | portrait-sprite / visibility | **FOLD-IN/RECONCILE** | Pending/incoming leader. |
| `leader_groups_portrait_window.divisions_count` (instantTextboxType, placeholder `-`) | `armies[<i>].member_count` (§12) | int | **FOLD-IN/RECONCILE** | Divisions in this army. |
| `leader_groups_portrait_window.leader_enroute` (`GFX_leader_en_route`) / `leader_coming` (`GFX_opinion_arrow_right`) | leader `assignment` transit state | visibility | **FOLD-IN/RECONCILE** | En-route / incoming markers. |
| `leader_groups_portrait_window.leader_frame_selected` (iconType, `GFX_leader_group_frame_selected`) | UI selection state | visibility | **static/UI** | Selection highlight — replay-local UI state. |
| `leader_groups_portrait_window.hq_alert` (iconType, `GFX_hq_alert`) | army HQ-attention alert | visibility | **FOLD-IN/RECONCILE** (derived) | HQ alert; an army-state derive. |
| `leader_groups_portrait_window.plan_evaluation` (iconType, `GFX_plan_value_strip`) + `plan_unit_power_bar` / `plan_preparation_bar` (`GFX_plan_unit_power`) | `armies[<i>].cached.plan_value` / `our_power` (§12, RUNTIME cached) | category / progressbar-fraction | **FOLD-IN/RECONCILE** (proxy) | Battle-plan power/eval bars. §12 marks `cached.*` as **RUNTIME, non-authoritative** → a **proxy** even once folded. |
| `attached_units` (gridBoxType) → attached-unit sub-icons | per-army attached units (railway guns / air wings) — §12 join | row factory / category | **FOLD-IN/RECONCILE** | The small attached-unit pips under the portrait. |
| `group_wing_type_box.wing_count_box` (instantTextboxType) | count of attached air wings (`air_wings[]` assigned to this army) | int | **FOLD-IN/RECONCILE** | Attached-wing count. |
| `group_railway_guns_box.railway_gun_count_box` (instantTextboxType) | count of attached railway guns | int | **FOLD-IN/RECONCILE** | Attached railway-gun count. |
| `naval_invasion_progress_frame` (iconType) / `plan_preparation_bar` | naval-invasion / plan-prep progress | progressbar-fraction | **FOLD-IN/RECONCILE** (proxy) | Prep bars; runtime-cached plan state (proxy). |
| **`army_group_portrait_window`** delta: `leader_icon` (`GFX_leader_marshal_unknown`), `divisions_count`, `group_marker` (`GFX_group_marker4`), same plan bars | per-army-group field-marshal `field_marshals[<i>].portrait_gfx` (via §12 `army_groups[].leader`) + member count | portrait-sprite / int | **FOLD-IN/RECONCILE** | Same pattern at the army-group/FM level. |

## 4b. Leader-groups — STATIC / decoration / mutation (no data binding)

- **Structural windows:** `leader_groups_window` + `bg_container`, `group_wing_type_box`,
  `group_railway_guns_box`, `leader_groups_portrait_window_bg`, `leader_groups_portrait_window`,
  `army_group_window`, `army_group_portrait_window`.
- **Frames / decoration / animation icons:** `army_group_bg`/`army_mid_bg`/`army_end_bg`
  (`GFX_LG_*`), `leader_frame` (`GFX_LG_group_frame` / `GFX_LG_army_group_frame`), `leader_frame_selected`,
  `group_marker` (`GFX_group_marker2`/`4`), `small_button_bg` / `small_button_border_conflict_bg`,
  `btn_execute_anim` (`GFX_active_battleplan_anim`), `execute_battle_plan_tutorial_glow`,
  `plan_frame`, `training_marker` (`GFX_exercise_icon_anim_strip`), `icn_exercise_stop_at_max`,
  `defending_marker` (`GFX_area_defense_active`), `icon_execute_point_out`, `expeditionary_group_icon`,
  `hq_badge`, `wing_type`/`railway_gun_icon` decoration icons.
- **Mutation buttons (inert in replay — these are battle-plan/command verbs):** `btn_execute` /
  `btn_stop` / `btn_stop_training` (execute/stop battle plan), `expand` (expand group),
  `create_expedionary_group_button` [sic], `assign_staff_office`, `assign_trait`,
  `disband_leader_btn`, `portrait_btn` / `portrait_highlighted_btn`, `wing_select_type`,
  `button_assign_railway_gun`.
- **Positioning anchors (positionType):** `hq_badge_shown_enroute_offset`, `pending_leader_shown_offset`.
  Plus `slotsize` (1×).

---

# VIEW 5 — ACES (`aces.json`, `acesview.gui`)

**Scope.** A small (19-element) grid of **flying aces**: the `acesview` window with a `grid`
(`gridBoxType`) host filled with `acesview_entry` row tiles. Binds `extra.aces[]` (§3.11). Observed
country = `<TAG>`.

Formatters: `loc`, `category` (`kill_type`→icon, ace alive/KIA→frame), `tech-icon` (current-wing
equipment), `visibility`.

## 5a. Aces — dynamic (data-bound) elements (the `acesview_entry` row factory)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `acesview.content.grid` (gridBoxType) → **N × `acesview_entry`** | row factory over `countries.<TAG>.extra.aces[]` | row factory (children below) | **FOLD-IN/RECONCILE** (would be **exact**) | One tile per ace. `extra.aces[]` schema-present (S, §3.11); `extra` domain **unfed in this demo snapshot**. Count cross-checks `extra.ace_count`. |
| ↳ `acesview_entry.name` (instantTextboxType) | `aces[<i>].name` + `aces[<i>].surname` + `aces[<i>].callsign` | loc | **FOLD-IN/RECONCILE** | Ace name + nickname (callsign). |
| ↳ `acesview_entry.modifiers` (instantTextboxType) | `aces[<i>].modifier` | loc/percent | **FOLD-IN/RECONCILE** | The ace's bonus modifier. The modifier→display string mapping uses the ace-bonus DEF (**STATIC DEF** for the label; the value is from `modifier`). |
| ↳ `acesview_entry.ace_btn` (buttonType, `GFX_ace_none`) | `aces[<i>].kill_type` → ace-type frame | category | **FOLD-IN/RECONCILE** | The ace portrait/type sprite, keyed by `kill_type`. |
| ↳ `acesview_entry.ace_kia` (iconType, `GFX_ace_kia`) | visible iff `aces[<i>].alive == false` | visibility | **FOLD-IN/RECONCILE** | KIA overlay. `alive` is the §3.11 field; post-death fields (`killer_country`) populate when dead. |
| ↳ `acesview_entry.current_wing_equipment_icon` (iconType, `GFX_technology_medium`) | ace's current wing plane archetype (OOB `air_wings[]` join) | tech-icon | **FOLD-IN/RECONCILE / partial GAP** | The plane the ace currently flies; needs the ace→wing join (not a direct §3.11 ace field). |
| ↳ `acesview_entry.current_wing_name` (instantTextboxType) | ace's current wing name (OOB `air_wings[].name`) | loc | **FOLD-IN/RECONCILE / partial GAP** | Current wing name; same wing join. |
| ↳ `acesview_entry.eligible_planetypes_1..3` (iconType, `GFX_wing_types_own`) | which plane types the ace can fly (ace-DEF / kill_type) | category | **STATIC DEF / adapter-fed** | Eligible-plane-type icons; an ace-DEF/`kill_type` attribute, not country state. |

## 5b. Aces — STATIC / decoration (no data binding)

- **Structural windows:** `acesview_entry` (template root), `acesview` (window root), `content`.
- **Backgrounds:** the two `Background` (`GFX_tiled_window_w_close`, `GFX_tiled_window_transparent`),
  `acesview_entry.bg` (buttonType, `GFX_aces_view_bg`).
- **Static label text:** `acesview.title` (empty placeholder text; loc set at runtime).
- **Nav:** `close_button` (`GFX_closebutton`).
- **Layout nodes:** `slotsize` (1×).

---

# TRAIT TREES — GRAPH PATTERN (Views 6, 7, 8)

The three trait-tree files form ONE engine, exactly parallel to the tech/focus trees (PLAN §B.5):

- **`trait_tree_common.json`** (View 8) is the **shared PROTOTYPE LIBRARY** — the per-node prototypes
  (`unit_leader_trait`, `disabled_trait`, `pickable_trait`, `picked_trait`, `advisor_trait`,
  `active_trait`, `exile_trait`, `active_advisor_trait`) and the **connection-line prototypes**
  (`trait_tree_line_{left,right,top,bottom,mid}`, the `_dotted` variants, `skill_tree_exclusive_*`,
  `trait_tree_exclusive`, `skill_tree_line_mid`). These are **instantiated by the trait-tree adapter
  per node / per edge** — they are NOT rendered standalone.
- **`army_trait_tree.json`** (View 6) and **`navy_trait_tree.json`** (View 7) are the **trait-tree
  WINDOWS** — they supply the selected-leader **header** (portrait + skill + the four stats + XP bar),
  the **tabs** (leader-traits vs advisor-promotion-traits), the **preferred-tactic** panel (army
  only), the **active-traits** column, and the `drag_scroll` **tree canvases** (`regular_traits`,
  `terrain_traits`, `advisor_traits` + the `lines` container) — but **NOT the trait nodes**, which the
  adapter fills using the View-8 prototypes from the trait CATALOG.

The leader-header binds the **selected** general/FM (army) or admiral (navy) from
`commanders.generals[]`/`field_marshals[]`/`admirals[]`. The trait NODES bind the selected leader's
`traits[]` (the §9 NODE-BINDING section). All node geometry + connection lines are **adapter-fed**.

---

# VIEW 6 — ARMY TRAIT TREE (`army_trait_tree.json`, `armytraittreewindow.gui`)

Formatters: `loc`, `int`, `portrait-sprite`, `skill-int`, `stat-int`, `progressbar-fraction`,
`trait-state` (picked/pickable/disabled → prototype + frame), `category`.

## 6a. Army trait tree — dynamic (data-bound) chrome (the selected-leader HEADER + tactic + advisor list)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `unit_leader` (iconType, `GFX_leader_unknown`) | selected `generals[<sel>].portrait_gfx` | portrait-sprite | **FOLD-IN/RECONCILE** | Selected leader's portrait. |
| `title` (instantTextboxType, placeholder `General`) | selected `generals[<sel>].role` → loc | loc | **FOLD-IN/RECONCILE** | Rank title (General / Field Marshal). |
| `name` (instantTextboxType, placeholder `Engin Mercan`) | selected `generals[<sel>].name` | loc | **FOLD-IN/RECONCILE** | Leader name. |
| `skill` (instantTextboxType, `hoi_30header`) | selected `generals[<sel>].level` (=skill) | skill-int | **FOLD-IN/RECONCILE** | Skill number. |
| `leader_stat_attack` (instantTextboxType) | `generals[<sel>].attack` | stat-int | **FOLD-IN/RECONCILE** | Attack. |
| `leader_stat_defense` (instantTextboxType) | `generals[<sel>].defense` | stat-int | **FOLD-IN/RECONCILE** | Defense. |
| `leader_stat_planning` (instantTextboxType) | `generals[<sel>].planning` | stat-int | **FOLD-IN/RECONCILE** | Planning. |
| `leader_stat_logistics` (instantTextboxType) | `generals[<sel>].logistics` | stat-int | **FOLD-IN/RECONCILE** | Logistics. |
| `leader_xp_progressbar_position` (positionType) + the XP bar it anchors | selected `generals[<sel>].experience` | progressbar-fraction | **FOLD-IN/RECONCILE** (derived) | Leader XP toward next skill level; `experience` is a §3.9 field; the level-up denominator is a rules-DEF constant → derive. |
| `advisor_traits_window.high_command_traits` / `advisor_traits` (smoothListBoxType) | `commanders.advisors[]` (the leader's assignable advisor-promotion traits) | row factory | **FOLD-IN/RECONCILE** | The advisor-promotion-traits tab list. |
| `preferred_tactic_tiling_container.tactics_list_entry.*_value` (`defender_damage_value`, `combat_width_change_value`, `attacker_damage_value`, `attacker_speed_value`) (instantTextboxType, placeholder `TEEST`) | selected leader `preferred_tactic` → tactic-DEF effect values | stat | **STATIC DEF / adapter-fed** | Which tactic is chosen = `generals[<sel>].preferred_tactic` (FOLD-IN); the per-tactic effect NUMBERS are tactic-DEF (static). |
| `add_trait_0..3.title` (instantTextboxType, `PICK_TRAIT`) + `new` glow | open trait slots (count = leader's remaining trait picks) | category / visibility | **FOLD-IN/RECONCILE** (derived) | "Pick a trait" slots; the count derives from skill level − assigned traits. Picking is a **mutation** → the slot is shown but inert in a replay. |

## 6b. Army trait tree — STATIC / decoration / nav (no data binding)

- **Structural windows / tree canvases:** `unitleader_trait_tree_window` (root), `traits_tabs`,
  `advisor_traits_window` (+ `advisor_high_command_header_container`, `advisor_slot_header_container`),
  `active_traits` (+ `personal_traits_title`, `officer_corps_title`, `add_advisor_role`, `traits_title`,
  `lines`), `preferred_tactic_tiling_container` (+ `tactics_list_entry`), `regular_traits_window`,
  `regular_traits`/`tree` (the `drag_scroll` node canvas — **adapter host**), `terrain_traits`/`tree`
  (terrain-trait canvas — **adapter host**).
- **Backgrounds / frames / separators:** `Background` / `Background_icon` (`GFX_trait_tree_bg`),
  `unit_leader_portrait_frame`, `skill_bg` (`GFX_land_battle_leader_skill_bg`), the four
  `leader_stat_*_bg` (`GFX_trait_attack`/`_defense`/`_planning`/`_logistics`), the `*_title_bg`
  separators (`GFX_trait_separator`), the `new` icons (`GFX_trait_new`), `highlight_land_doctrine_folder`
  (tutorial glow).
- **Static label text (loc constants):** `header_text` (`LEADER_DETAILS_TITLE`), tab labels
  (`regular_trait_tab_title`, `advisor_trait_tab_title`), the section headers (`background`,
  `officer_corps`, `traits`, `basic_traits`, `field_marshal_traits`, `assignable_traits`,
  `terrain_traits`, `pick_advisor_role`, `advisor_slot_title`), `PREFERRED_TACTIC`,
  `NO_PREFERRED_TACTIC_SELECTED`, `skill too low`, the tactic-entry placeholder `TEEST` labels.
- **Nav / mutation buttons (inert in replay):** `leader_traits_tab` / `advisor_promotion_traits_tab`
  (tab switch — UI state), `wiki_button` (`GFX_trait_wiki`), `button_prefered_tactic_selection`
  (tactic pick — mutation), `close_button`.
- **Layout nodes / anchors:** `drag_scroll` (3×, the scroll canvases), `borderSize` (1×),
  `positionType` (1× — the XP-bar anchor, bound in 6a).

## 6c. Army trait tree — NODE BINDING (adapter-fed; nodes are NOT in this `.gui`)

Each trait NODE is instantiated by the **trait-tree adapter** into the `regular_traits.tree` /
`terrain_traits.tree` `drag_scroll` canvases, cloning a **View-8 prototype** per trait. The node's
RECT comes from the trait DEF's `gui_row`/`gui_column` (`common/unit_leader/*.txt`) and the
connection lines from the trait `parent`/`prerequisites`/`mutually_exclusive` edges — both
**adapter-fed**, NOT a declarative element bind. The node's STATE binds the selected leader's
`traits[]`.

| node state (per trait) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| **picked** (leader HAS this trait) | `generals[<sel>].traits[]` ∋ trait token → use `picked_trait` prototype (`GFX_trait_picked`) | trait-state | **FOLD-IN/RECONCILE** (would be **exact** membership) | Membership test selects the picked prototype + checkmark. `traits[]` is the §3.9 field; unfed in this demo snapshot. |
| **pickable** (eligible but not taken) | NOT in `traits[]` AND prereqs ⊆ `traits[]` AND skill high enough → `pickable_trait` (`GFX_trait_pickable`) | trait-state | **FOLD-IN/RECONCILE** (derived) | Pickable iff prereqs satisfied (prereq SET is trait-DEF/adapter; the satisfied-test uses `traits[]`). The skill/slot gate adds a derive. |
| **disabled** (locked / mutually-excluded) | NOT pickable (prereqs unmet OR a `mutually_exclusive` sibling is picked) → `disabled_trait` (`GFX_trait_disabled`) | trait-state | **FOLD-IN/RECONCILE** (derived) | The locked prototype. XOR state needs the trait-DEF `mutually_exclusive` graph (adapter) tested against `traits[]`. |
| **trait name** | trait token → loc | loc | **STATIC DEF / adapter-fed** | Trait display name is a loc lookup on the adapter-supplied token. |
| **trait icon** | trait DEF `icon` sprite | category (`GFX_trait_unknown` placeholder) | **STATIC DEF / adapter-fed** | The trait's sprite from the trait DEF. |
| **trait buff text** (effect blurb on `active_trait`) | trait DEF modifier block | loc | **STATIC DEF / adapter-fed** | The "buff text" is the trait's modifier/effect (trait-DEF), not country state. |
| **connection lines** (parent / prereq edges) | trait DEF `parent`/`prerequisites` (static) → `trait_tree_line_*` prototypes | — | **adapter-fed** | Edges from the trait DEF; satisfied-vs-not tests against `traits[]`. Not a declarative bind. |
| **exclusive lines** (mutually-exclusive pairs) | trait DEF `mutually_exclusive` (static) → `skill_tree_exclusive_*` / `trait_tree_exclusive` prototypes | — | **adapter-fed** | XOR routing from the trait DEF. |
| **node rect / position** | trait DEF `gui_row`/`gui_column` (static) | — | **adapter-fed** | Positions from the trait script; not a snapshot field. |

---

# VIEW 7 — NAVY TRAIT TREE (`navy_trait_tree.json`, `navytraittreewindow.gui`)

**Identical engine to View 6**, for admirals. The only deltas: the header uses **navy stats**
(`leader_stat_maneuvering` / `leader_stat_coordination` replace planning/logistics), there is **no
preferred-tactic panel**, and the regular-traits window is laid out slightly differently
(`assignable_traits_text` = `ASSIGNABLE_ADMIRAL_TRAITS`). The node binding is the SAME pattern as 6c
against `commanders.admirals[]`.

## 7a. Navy trait tree — dynamic (data-bound) chrome (HEADER + advisor list)

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `unit_leader` (iconType, `GFX_leader_unknown`) | selected `admirals[<sel>].portrait_gfx` | portrait-sprite | **FOLD-IN/RECONCILE** | Selected admiral portrait. |
| `title` (instantTextboxType, placeholder `General`) | selected `admirals[<sel>].role` → loc | loc | **FOLD-IN/RECONCILE** | Rank title (Admiral). |
| `name` (instantTextboxType) | selected `admirals[<sel>].name` | loc | **FOLD-IN/RECONCILE** | Admiral name. |
| `skill` (instantTextboxType) | selected `admirals[<sel>].level` (=skill) | skill-int | **FOLD-IN/RECONCILE** | Skill. |
| `leader_stat_attack` (instantTextboxType) | `admirals[<sel>].attack` | stat-int | **FOLD-IN/RECONCILE** | Attack. |
| `leader_stat_defense` (instantTextboxType) | `admirals[<sel>].defense` | stat-int | **FOLD-IN/RECONCILE** | Defense. |
| `leader_stat_maneuvering` (instantTextboxType) | `admirals[<sel>].maneuvering` | stat-int | **FOLD-IN/RECONCILE** | Maneuvering (navy-specific). |
| `leader_stat_coordination` (instantTextboxType) | `admirals[<sel>].coordination` | stat-int | **FOLD-IN/RECONCILE** | Coordination (navy-specific). |
| `leader_xp_progressbar_position` (positionType) + XP bar | `admirals[<sel>].experience` | progressbar-fraction | **FOLD-IN/RECONCILE** (derived) | Admiral XP; level-up denominator is rules-DEF → derive. |
| `advisor_traits_window.high_command_traits` / `advisor_traits` (smoothListBoxType) | `commanders.advisors[]` (navy advisor-promotion traits) | row factory | **FOLD-IN/RECONCILE** | Advisor-promotion list. |
| `add_trait_0..3.title` (`PICK_TRAIT`) + `new` glow | open admiral trait slots (skill − assigned) | category / visibility | **FOLD-IN/RECONCILE** (derived) | Pick-trait slots; mutation → inert. |

## 7b. Navy trait tree — STATIC / decoration / nav (no data binding)

- **Structural windows / tree canvases:** `navyleader_trait_tree_window` (root), `traits_tabs`,
  `advisor_traits_window` (+ `advisor_slot_header_container`, `advisor_high_command_header_container`),
  `active_traits` (+ `officer_corps_title`, `add_advisor_role`, `personal_traits_title`, `traits_title`,
  `lines`), `regular_traits_window`, `regular_traits`/`tree`, `terrain_traits`/`tree` (the `drag_scroll`
  **adapter host** canvases).
- **Backgrounds / frames / separators:** `Background` / `Background_icon` (`GFX_trait_tree_bg`),
  `unit_leader_portrait_frame`, `skill_bg`, the four `leader_stat_*_bg` (attack/defense/maneuvering/
  coordination), the `*_title_bg` separators (`GFX_trait_separator`), the `new` icons.
- **Static label text (loc constants):** `header_text` (`LEADER_DETAILS_TITLE`), tab labels,
  section headers (`officer_corps`, `pick_advisor_role`, `background`, `traits`, `basic_traits`,
  `ASSIGNABLE_ADMIRAL_TRAITS`, `terrain_traits`, `advisor_slot_title`).
- **Nav / mutation buttons (inert):** `leader_traits_tab` / `advisor_promotion_traits_tab`,
  `wiki_button`, `close_button`.
- **Layout nodes / anchors:** `drag_scroll` (4×), `borderSize` (1×), `positionType` (1×, XP-bar anchor).

## 7c. Navy trait tree — NODE BINDING (adapter-fed)

**Identical to 6c**, binding `admirals[<sel>].traits[]` instead of generals. Picked/pickable/disabled
prototypes from View 8; node geometry + connection/exclusive lines **adapter-fed** from the navy-trait
DEF; trait name/icon/buff **STATIC DEF**. (Table omitted — see 6c; substitute `admirals[<sel>].traits[]`.)

| node state (per navy trait) | snapshot source path | fidelity |
|---|---|---|
| picked / pickable / disabled | `admirals[<sel>].traits[]` membership + trait-DEF prereqs/XOR | **FOLD-IN/RECONCILE** (membership exact when folded; gating derived) |
| name / icon / buff | trait DEF (loc + sprite + modifier) | **STATIC DEF / adapter-fed** |
| connection / exclusive lines / node rect | trait DEF `parent`/`prerequisites`/`mutually_exclusive`/`gui_row`/`gui_column` | **adapter-fed** |

---

# VIEW 8 — TRAIT-TREE COMMON PROTOTYPES (`trait_tree_common.json`, `traittreewindow_common.gui`)

**Scope.** This file declares NO standalone window — it is the **shared prototype library** the two
trait-tree adapters (Views 6/7) clone per node and per edge. Every element here is either a **node
prototype** (mapped to a trait state) or a **connection-line prototype** (an adapter-fed edge). There
is no per-element country binding in THIS file; the binding happens when the adapter instantiates a
prototype with a specific trait token (the §6c/§7c NODE-BINDING tables). Listed here for completeness
and to record which prototype each trait STATE selects.

## 8a. Trait-tree common — NODE PROTOTYPES (instanced per trait by the adapter)

| prototype (name + type) | selected when | binds (at instantiation) | fidelity |
|---|---|---|---|
| `unit_leader_trait` (containerWindowType) — has `disabled_bg`/`pickable_bg`/`picked_bg` sub-buttons + `icon` + `name` | a regular trait node (the adapter shows the bg matching the state) | `traits[]` membership → which bg; trait DEF → icon + name | **FOLD-IN/RECONCILE** (state) / **STATIC DEF** (icon+name) |
| `picked_trait` (containerWindowType, `GFX_trait_picked`) | trait ∈ leader `traits[]` | trait DEF icon + name | **FOLD-IN/RECONCILE** |
| `pickable_trait` (containerWindowType, `GFX_trait_pickable`) | eligible, prereqs ⊆ `traits[]`, not taken | trait DEF | **FOLD-IN/RECONCILE** (derived) |
| `disabled_trait` (containerWindowType, `GFX_trait_disabled`) | locked / XOR-excluded | trait DEF | **FOLD-IN/RECONCILE** (derived) |
| `advisor_trait` / `active_advisor_trait` (containerWindowType, `GFX_trait_adviser_pickable` / `GFX_empty_paper` + `idea_traits`) | an advisor-promotion trait | `commanders.advisors[]` token → idea-trait frame; advisor-DEF buff text | **FOLD-IN/RECONCILE** (token) / **STATIC DEF** (buff) |
| `active_trait` (containerWindowType, `GFX_trait_new` + `icon` + `name` + `buff`) | a newly-active trait in the active-traits column | `traits[]` membership; trait DEF name + buff | **FOLD-IN/RECONCILE** (state) / **STATIC DEF** (buff) |
| `exile_trait` (containerWindowType, `GFX_flag_small` + `exile_flag_frame` + `name` + `buff`) | an exile-government trait | exile-source TAG → flag; trait DEF name+buff | **FOLD-IN/RECONCILE** (flag) / **STATIC DEF** (buff) |

## 8b. Trait-tree common — CONNECTION-LINE / EXCLUSIVE prototypes (adapter-fed edges, not declarative)

These are the **graph-edge tiles** the adapter instantiates per edge; the routing comes from the trait
DEF (`parent`/`prerequisites`/`mutually_exclusive`), NOT from any country field. **adapter-fed**:

- `trait_tree_line_{left,right,top,bottom,mid}` (iconType, `GFX_trait_tree_line_*`) — solid prereq-edge
  segments (satisfied edges).
- `trait_tree_line_{left,right,top,bottom}_dotted` (iconType, `GFX_trait_tree_line_*_dotted`) —
  dotted (unsatisfied / optional) edge segments.
- `skill_tree_line_mid` (iconType) — mid-segment connector.
- `trait_tree_exclusive` / `skill_tree_exclusive_horizontal` / `skill_tree_exclusive_vertical` /
  `unit_leader_trait_tree_exclusive_vertical` (iconType, `GFX_trait_tree_exclusive_*`) — mutual-exclusion
  routing tiles.
- `skill_tree_empty_window_for_line` (containerWindowType + `background`) — a spacer the adapter uses
  for line layout.
- `unit_leader_trait_tree_connection_offset_orientation_horizontal` (positionType) — a connection
  layout anchor.

## 8c. Trait-tree common — STATIC

The only non-prototype static is the `skill_tree_empty_window_for_line.background` (decoration) and the
connection anchors above. No country binding in this file.

---

## Notes

**A. The whole leader/officer/trait stack is a DEMO-SNAPSHOT ARTIFACT in this snapshot, not a schema
gap.** The `commanders` (§3.9), `extra` (§3.11), `abilities` (§3.15) and `order_of_battle` (§12)
domains are bind-ready and have live readers (`commanders_live`, `country_extra`, `oob` in
`schema.py` DOMAINS), but the SAVE-ONLY demo channel that produced this snapshot folds none of them
(verified: GER's only country keys are decisions/diplomacy/economy/focus/intel/military/politics/
research). So every leader name/portrait/skill/stat/trait/assignment/ace badge is
**FOLD-IN/RECONCILE** — it becomes exact/derived the moment a full live snapshot is used, with NO
schema change. Do **not** count these as true schema gaps.

**B. The trait trees are the GRAPH-pattern (PLAN §B.5), same as tech/focus.** Chrome (the
selected-leader header, tabs, tactic panel, tree canvases) is IN the army/navy `.gui`; the trait NODES
are adapter-instantiated from `trait_tree_common.json` prototypes, positioned from the trait DEF's
`gui_row`/`gui_column`, with edges from `parent`/`prerequisites`/`mutually_exclusive`. Node GEOMETRY +
LINES are adapter-fed (STATIC DEF / world data); node STATE (picked/pickable/disabled) binds the
selected leader's `traits[]` (FOLD-IN). Trait NAME/ICON/BUFF are STATIC DEF (the trait catalog), not
country state — never badge them `unavailable` (they render once the adapter loads the catalog).

**C. `portrait_gfx` is a sprite NAME, not image data.** Every portrait bind resolves the named sprite
(`commanders.*.portrait_gfx`, e.g. `GFX_portrait_GER_…`) from the game's `.gfx`/DDS at render — an
**asset-extraction** step. The snapshot never carries pixels (no-fabrication).

**D. Mutations are inert in a replay (PLAN §D).** Hire-advisor, promote, pick-trait, pick-tactic,
disband, execute/stop battle plan, assign-role/staff-office, new-leader, rescue-raid, edit/delete-HQ-
template — all rendered as real chrome but **inert** (read-only replay). They carry no datum and are
listed under STATIC/nav.

**E. RUNTIME-cached plan values are PROXY even once folded.** The `leader_groups.json` battle-plan
power/eval/prep bars bind §12 `armies[].cached.{plan_value,our_power,enemy_power}`, which SCHEMA marks
**RUNTIME, non-authoritative** — render as a **proxy**, never authoritative, even after the OOB domain
folds.

---

## Element accounting (provably complete)

| view (file) | TOTAL elements (== `.gui` walk) | dynamic-bound | static / decoration / nav / mutation | adapter-fed (node hosts + edge prototypes) |
|---|---:|---:|---|---:|
| **officer_corps.json** | **333** | ~24 (advisor-entry children ×6 templates, spirit-item children ×9, chief grids ×5, doctrine/track/HQ/captured readouts, modifiers/preferred-tactic) | ~309 (root + branch headers/containers, all backgrounds/banners, label text, sort/nav/mutation buttons, margins/slotsizes/max_slots) | the 5 chief `gridBoxType` + 3 `spirit_grid` (`gridboxtype`) + `templates_grid` node hosts |
| **unit_leader.json** | **305** | ~22 distinct (the `armyleaderentry` row children + the captured/navy/operative/division/faction deltas) | ~283 (6 window roots + 6 entry roots, backgrounds/frames, header/label text, all `sort_*`/search/checkbox, mutation/nav buttons, positionTypes/margins/borderSizes/slotsizes) | the 6 `leaders_grid` (`gridBoxType`) row-factory hosts + `traits_grid`/`nationalities_grid`/`medals_grid`/`button_grid` |
| **navy_leader.json** | **25** | 7 (new-fleet portrait, fleets_box factory, selected-admiral portrait, taskforce_count ×2, enroute, group_marker) | ~17 (5 windows, frames/decoration, scroll/disband/assign mutations) | the `fleets_box` (`gridBoxType`) host |
| **leader_groups.json** | **82** | ~16 (general + army-group portraits, divisions_count ×2, plan bars, attached-unit counts, enroute/coming/alert) | ~65 (8 windows, all `GFX_LG_*`/marker/anim/glow decoration, execute/stop/expand/assign/disband mutations, positionTypes) | the `attached_units` (`gridBoxType`) host |
| **aces.json** | **19** | 9 (the `acesview_entry` children: name, modifiers, ace_btn, ace_kia, current_wing_equipment_icon, current_wing_name, eligible_planetypes ×3) | ~9 (acesview window + content + backgrounds, title, close) | the `grid` (`gridBoxType`) row-factory host |
| **army_trait_tree.json** | **90** | ~13 chrome (header portrait/skill/4 stats/XP, advisor lists ×2, tactic values, add_trait slots) + **9 node states (6c)** | ~68 (root/tabs/active-traits/regular-/terrain-traits windows, backgrounds/separators, label text, wiki/tab/close, drag_scrolls) | the `regular_traits.tree` + `terrain_traits.tree` + `lines` (`drag_scroll`) node-canvas hosts |
| **navy_trait_tree.json** | **74** | ~11 chrome (header portrait/skill/4 navy stats/XP, advisor lists ×2, add_trait slots) + **3 node states (7c)** | ~60 (same chrome set minus tactic panel, backgrounds/separators, label text, wiki/tab/close, drag_scrolls) | the `regular_traits.tree` + `terrain_traits.tree` + `lines` (`drag_scroll`) hosts |
| **trait_tree_common.json** | **56** | 0 standalone (binds happen at adapter instantiation) | 1 (`skill_tree_empty_window_for_line.background`) | **7 node prototypes (8a)** + **~16 connection/exclusive line prototypes (8b)** + the connection anchor |
| **TOTAL** | **984** | ~90 distinct dynamic chrome binds + **12 trait-node states (6c/7c)** + 7 node prototypes | ~810 | ~30 adapter hosts/prototypes |

**Trait-node coverage:** the trait NODES themselves are NOT declarative `.gui` elements — they are
adapter-instantiated (one per trait in `common/unit_leader/*.txt`) using the 7 View-8 prototypes; the
NODE-BINDING tables (6c army, 7c navy) map each node STATE → the selected leader's `traits[]`
(FOLD-IN/RECONCILE) with geometry/edges adapter-fed (STATIC DEF). Across all 953/953 leaders (the
PLAN §C.4 target), each leader's trait set drives its own node-state pass.

### Badge tally (no `unavailable`-by-fabrication)

- **FOLD-IN/RECONCILE (demo-snapshot artifact — commanders/extra/oob domain schema-present,
  parser-unfed in THIS demo snapshot):** the overwhelming majority — every leader/advisor/ace
  name·portrait·skill·stat·trait·assignment·count bind across all 8 views.
- **DERIVED-NOW (bindable from a domain already folded in THIS demo snapshot):** the officer-corps
  **spirit tiles/pips** — `politics.ideas_other[]` (the ordered active-ideas/spirits set) IS present
  (politics is one of the 8 folded keys); the work is the officer-corps-ledger partition + the
  spirit-catalog token→frame map. Distinct from the commanders-domain FOLD-IN binds.
- **STATIC DEF / adapter-fed (not country state, parseable from game `common/`):** trait
  catalog (name/icon/buff/edges/geometry), tactic catalog (effect values), spirit catalog (cost/
  modifier), ace eligible-plane-types, ace bonus DEF label.
- **PROXY (runtime-cached, non-authoritative even once folded):** §12 `cached.*` battle-plan
  power/eval bars in `leader_groups.json`.
- **TRUE GAP (no schema home today):** named army-HQ templates (`named_hq_template_entry`),
  divisional-commander roster + division medals (`divisionleaderentry`/`division_medal_entry`/
  `medals_grid`), and the faction-commander mastery MATH (`mastery_value`). See the ledger.

No element is badged `unavailable` because of fabrication; every absent datum renders "—".

---

## Schema-gap ledger (needed-but-absent or unfed fields)

| # | needed for | field / data | ARTIFACT vs TRUE GAP | closure bucket |
|---|---|---|---|---|
| 1 | **all leader names/portraits/skills/stats/traits/assignment** (officer corps, unit-leader roster, navy-leader, leader-groups, both trait-tree headers + node states) | `countries.<TAG>.commanders.{generals,field_marshals,admirals,advisors,leader,counts}[]` (§3.9) + each leader's `traits[]`/`assignment` | **ARTIFACT** — schema-present (S), live reader `commanders_live` exists; **parser-unfed in THIS demo snapshot** (`commanders=None` for every country). FOLD-IN/RECONCILE | n/a (artifact) — closes when a full live/`characters.json` snapshot folds the `commanders` domain; then **exact/derived**, no schema change |
| 2 | **aces** (the aces view) | `countries.<TAG>.extra.aces[]` + `extra.ace_count` (§3.11) | **ARTIFACT** — schema-present (S), `country_extra` domain; unfed here (`extra=None`). FOLD-IN/RECONCILE | n/a (artifact) — closes when `country_extra.json` folds; then **exact** |
| 3 | **leader→formation assignment** (leader-groups portraits, navy fleets, divisions_count) | `order_of_battle.{army_groups,armies,fleets}[].leader` + `member_count`/`task_forces` (§12) joined by `unit_leader_id` (type 4713) | **ARTIFACT** — schema-present (S), `oob` domain; unfed here (`order_of_battle=None`). FOLD-IN/RECONCILE | n/a (artifact) — closes when `oob.json` folds; then **exact** (assignment join) |
| 4 | **captured-generals counter** (officer corps) | `countries.<TAG>.abilities.officer_corps.{own_generals_captured,captured_enemy_generals}` (§3.15, D) | **ARTIFACT** — schema-present, `abilities` domain unfed here. FOLD-IN/RECONCILE | n/a (artifact) — closes when `abilities.json` folds |
| 5 | **trait CATALOG** — node name/icon/buff text + the `parent`/`prerequisites`/`mutually_exclusive` edges + `gui_row`/`gui_column` geometry (all three trait-tree views) | trait DEF in `common/unit_leader/*.txt` | **TRUE GAP** (for the snapshot — it is world/DEF data, never per-country) | **(c) STATIC DEF / WORLD DATA** — the trait-tree adapter parses `common/unit_leader/` (catalog + layout + effects); identical to the focus/tech adapters |
| 6 | **preferred-tactic effect VALUES** + **spirit cost/modifier** + **ace eligible-plane-types / ace-bonus label** | tactic DEF (`common/combat_tactics`/`combat_tactic_*`), idea/spirit DEF (`common/ideas`, `common/units/*` officer-corps spirits), ace-bonus DEF | **TRUE GAP** (DEF/world data) | **(c) STATIC DEF / WORLD DATA** — adapter parses the relevant `common/` catalogs |
| 7 | **faction-commander MASTERY % + supply-consumption aggregates** (`mastery_value`, `supply_value`, `supply_navy_value`) | runtime doctrine-mastery / supply-consumption math for the faction commander | **TRUE GAP** — runtime-computed, not stored | **(b) COMPUTED-NOT-STORED (Ghidra tier)** — the engine computes mastery/supply modifiers; the save stores none; the assigned-country/region COUNTS (`assigned_countries`/`assigned_regions`) are **(a) LIVE-EXTRACTABLE** from the faction-command join |
| 8 | **named army-HQ templates** (`named_hq_template_entry`: name/deploy_time/cp_value/counter) | a per-country named-HQ-template list | **TRUE GAP** — no schema field for named HQ templates (it is division-designer/template data) | **(a) LIVE-EXTRACTABLE** (army-HQ-template records exist in the save/memory; a new parser leaf would be needed) |
| 9 | **divisional-commander roster + division medals** (`divisionleaderentry`, `division_medal_entry`, `medals_grid`) | per-division leader (name/xp/unit_level/template) + medals | **TRUE GAP** — §3.9 models the country HIGH-COMMAND roster, not per-division commanders | **(a) LIVE-EXTRACTABLE** (divisional commanders + medals are in the OOB/division records; needs a new parser leaf) |
| 10 | **operative-character detail in the operative roster** (`operativeleaderentry`: name/skill/nationalities/mission) | operative-character records (name, skill, nationalities, current mission) | **partial TRUE GAP** — the `intel` domain IS present (top-level key) but the operative-character roster detail is a separate reach | **(a) LIVE-EXTRACTABLE** (operative pool exists in §15 `strategic_operatives` + intel; the per-operative character detail needs the character→operative join) |
| 11 | **doctrine-track progress on the doctrine buttons** (`number_container`, `track_status_grid`, `track_progress_item`) | doctrine `research.technologies[]` membership/level for that branch | **ARTIFACT (inherited)** — doctrine techs are unfed in this SAVE CHANNEL (see the tech/doctrine binding map ledger #1: 0 of 187 world tech names are doctrines) | n/a (artifact) — closes with the doctrine-tech parser change (no schema change) |

---

## Historical review

**Historical review round — TIMED OUT (×2), proceeded with self-audit.** This records
a completed review attempt from the old Claude-era workflow. Codex-native
sessions must use normal sub-agents/partner review and must not run the old local
CLI command. Each attempt was given the full schema facts, the empirical demo-snapshot finding, all 8 views' element counts, and the
five highest-risk binding claims, asking the four mandated questions: (a) missed elements,
(b) field-path correctness, (c) false `unavailable`/wrong fidelity, (d) artifact-vs-gap classification.
**Attempt 1** exceeded the 300 s bash deadline with no `-o` written; **attempt 2** (background, longer
budget) was still mid-exploration of the repo (it had read the `.gui` walks, `schema.py`, the diff
event list, and the assembler/snapshot code) past the ~12-minute mark without emitting its reply. The doc shipped
without a folded external verdict and relies on the self-audit below.

**Self-audit of the highest-risk claims (in lieu of the Codex pass), each grounded in a re-read of
`SCHEMA.md`/`schema.py`/the parsed `.gui`:**

| claim | independent verification | result |
|---|---|---|
| `commanders`/`extra`/`abilities`/`order_of_battle` ABSENT in the demo snapshot ⇒ classify as ARTIFACT not schema gap | Loaded the snapshot in Python: every country's keys are exactly `[decisions,diplomacy,economy,focus,intel,military,politics,research]`; `country.get('commanders') is None`. `schema.py` DOMAINS lists `commanders`/`commanders_live`/`country_extra`/`oob` as real domains with `FIELD_SOURCES` `S`/`M`. | **CONFIRMED** — absence is parser-unfed-in-demo (ARTIFACT), not a schema gap. |
| army/navy leader field paths (`generals[].{name,role,level,attack,defense,planning,logistics,traits,portrait_gfx}`; admirals use `maneuvering`/`coordination`) | Matched against `SCHEMA.md` §3.9 table verbatim (`level`(=`skill`), the army-vs-navy stat split, `assignment` from `unit_leader_id` type 4713 via §12). | **CONFIRMED.** |
| aces paths (`extra.aces[].{name,surname,callsign,modifier,alive,kill_type,killer_country}`, `extra.ace_count`) | Matched §3.11 verbatim. (Note: `abilities.aces` (§3.15) is a parallel home; the view binds the §3.11 `extra.aces[]` shape the brief specifies.) | **CONFIRMED.** |
| officer-corps SPIRIT tiles are DERIVED-NOW (not a commanders artifact) | `politics.ideas_other` is `S` in `FIELD_SOURCES` and `politics` IS one of the 8 folded keys in the demo snapshot; `schema.py` comments it as the ordered active-ideas/spirits set. | **CONFIRMED + FIXED** — re-badged those rows `derived` (was FOLD-IN), added a DERIVED-NOW tally bucket. |
| plan bars in `leader_groups.json` = PROXY even once folded | §12 explicitly marks `cached.{plan_value,our_power,enemy_power}` "RUNTIME, non-authoritative". | **CONFIRMED.** |
| TRUE GAPs (named HQ templates; divisional-commander roster + medals; faction-commander mastery math) are NOT in §3.9–§3.15 | §3.9 is the country high-command roster (no per-division commander leaf); no named-HQ-template leaf exists; mastery/supply are runtime modifier math (matches the SCHEMA §11 "derived combat stats are a hard limit" pattern). | **CONFIRMED** — closure buckets (a LIVE-EXTRACTABLE for HQ-templates/divisional/operative; b COMPUTED-NOT-STORED for mastery math; c STATIC DEF for the trait/tactic/spirit catalogs) stand. |
