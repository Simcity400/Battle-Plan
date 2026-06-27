# BINDING MAP — RESEARCH + NATIONAL FOCUS + MENUBAR (Phase-3 binding sweep)

> **What this is.** The COMPLETE, no-truncation data-binding map for three in-game the game views the
> native replay UI binds in the Phase-3 sweep: the **research** side panel
> (`game_assets/gui/research.json`, from `interface/countrytechnologyview.gui`), the **national
> focus** tree (`game_assets/gui/national_focus.json`, from `interface/nationalfocusview.gui`), and
> the **menubar** (`game_assets/gui/menubar.json`, from `interface/menubar.gui`). It realises
> GAME_UI_REPLICA_PLAN §C (the comprehensive-but-mechanical binding sweep) for these three surfaces:
> EVERY element in each parsed `.gui` tree is accounted for — dynamic (data-bound), static/decoration
> (no binding), or deferred (needs an imperative adapter). Nothing is omitted; where a `.gui` element
> exists but our snapshot has no field for it, the element is listed and **badged `unavailable`** with
> the missing field named.
>
> **Binding-rule shape** (GAME_UI_REPLICA_PLAN §C.1): `{ target (gui element name+type), source
> (snapshot path), format (formatter id), fidelity (exact|derived|proxy|unavailable) }`. Source paths
> use SCHEMA dotted form: `countries.<TAG>.…` (see `replay/SCHEMA.md` §3 and `schema.py`
> `FIELD_SOURCES`). `<TAG>` = the player/observed country.
>
> **Two of these views are GRAPH-pattern trees (§B.5), not flat element lists.** Research and national
> focus are tree engines: the `.gui` supplies only the FRAME / scroll canvas / node-prototype, and the
> NODES come from data (the tech catalog / the focus script). Pure declarative binding **breaks** for
> the node graph (PLAN §C.2 "Graph + link lines" row → adapter required). So per tree this doc has
> BOTH: (a) the standard per-element table for the chrome that IS in the `.gui`, and (b) a dedicated
> **NODE-BINDING** section mapping each tech/focus node → its snapshot fields, since the nodes are not
> declarative `.gui` elements. Node geometry + link lines are flagged **adapter-fed**, not a
> declarative element bind.
>
> **No-fabrication (SCHEMA §0.2, PLAN principle 3):** `null` renders "—", never `0`. `0`/`0.0` is a
> measured zero. A field schema-present but never fed by the current parser is badged `unavailable`
> (not silently dropped).

---

## Provenance of the fidelity calls

Calls below are grounded in **empirical inspection of the real save-backed snapshot**
`replay/snapshots/replay/000017005619.snapshot.json` (`schema_version "1"`, date `1941.4.13.11`, GER
1941 save channel, 143 countries), cross-checked against `replay/SCHEMA.md` §3.4 / §3.5 and
`schema.py` `FIELD_SOURCES` + `DOMAINS`, and **adversarially reviewed by Codex** (read-only,
2026-06-26). Codex's verdict and the corrections folded in are recorded in the final section.

**Empirically confirmed PRESENT & usable in the current snapshot (drives the `exact`/`derived`
badges):**

| schema field | observed in GER (sample) | drives |
|---|---|---|
| `research.tech_count` | `122` | research-count readouts (no `.gui` slot in this side panel — see note) |
| `research.research_slots` / `original_research_slots` | `5` / `4` | the slot count → `research_slots_grid` row factory |
| `research.technologies[]` | 122 rows `{name, level, date, research_points}` | completed-tech NODE level (tech-tree adapter) |
| `research.in_progress[]` | 5 rows `{name, points_factor}` | the active-research SLOT rows (`research_slot_entry`) |
| `research.research_bonuses[]` | 20 rows `{name, categories[], bonus, kind}` | focus/research bonus readout |
| `focus.focus_tree` | `"german_focus"` | which focus tree the adapter loads |
| `focus.completed[]` / `completed_count` | 55 ids / `55` | completed-focus NODE state |
| `focus.current` | `"GER_the_swiss_gold"` | the in-progress focus NODE (glow + detail) |
| `focus.progress` | `12` (days into focus) | the active-focus progress bar |
| `focus.continuous_focus_palette` | `"generic_focus"` | continuous-focus list source |
| `politics.faction.name` | `"faction_template_axis"` | war-overview / endgame faction grouping |

**Empirically confirmed ABSENT/UNFED in the current snapshot (drives the `unavailable` badges):**

| schema field | state in snapshot | consequence |
|---|---|---|
| `research.in_progress_live` | **`None`** (save channel; live not folded) | the `eta`/live-progress fraction of an active-research row → `unavailable` (the SAVE `points_factor` is not an ETA) |
| `research.in_progress[].progress_pct` | **not present** (SCHEMA §3.4: `M*`, null in save) | research-progress BAR fraction → `unavailable` (only `points_factor` is stored) |
| `technologies[].research_points` | present BUT is the tech's point COST, not a date/ETA | `year`/`eta` on a completed tech derive from `technologies[].date`, not this |
| top-level `designs` / tech catalog geometry | not a snapshot field (static game data) | tech-node RECTS + link lines are **adapter-fed** from `countrytechtreeview.gui`, not the snapshot |
| focus node geometry (`x`/`y`/`relative_position_id`/`offset`) | not a snapshot field (lives in `common/national_focus/*.txt`) | focus-node RECTS + link lines are **adapter-fed** from the focus script, not the snapshot |
| per-focus `cost` (PP) / `available`/`bypass` triggers | not in `countries.*.focus` | focus-node cost text + availability frame → `unavailable` (focus-DEF data, not country state) |
| endgame `score` / army/navy/air/industry/province tallies | `countries.*.score.*` is schema-present but **`D` (derived) and not in this snapshot** | endgame ladder columns → `unavailable` (no `score` block folded) |
| `commanders.leader` (name/ideology/portrait) | `commanders` domain not folded in this snapshot | endgame leader/ideology/govt/party → `unavailable` |

---

# VIEW 1 — RESEARCH side panel (`game_assets/gui/research.json`, `countrytechnologyview.gui`)

**Scope clarification (load-bearing).** This `.gui` is the research **side panel** that opens on the
left (`countrytechnologyview`, x=-606): the research-slots list (the active-research rows) plus a
small **research-folder grid** (`research_groups_grid`) and a focus/speed bonus header. It is **NOT**
the full scrollable tech-tree grid — that is `countrytechtreeview.gui` (6678 lines), a separate
GRAPH-pattern view handled by the **tech adapter** (PLAN §B.5, §F Phase 2). Because the brief maps
"each tech → completed level / in-progress / slots / bonuses", the tech-tree NODE binding is captured
in section **1c (NODE-BINDING)** below; the per-element tables 1a/1b cover this side-panel `.gui`.

The two reusable entry templates this view instantiates are `technology_idea_entry` (an idea-tile in
the folder grid) and `research_slot_entry` (one active-research row). Selected/observed country =
`<TAG>`.

Formatters: `int`, `percent`, `loc` (token→localised string), `tech-folder-frame` (frame of
`GFX_research_folder_strip`), `equipment-role-icon` (frame of `GFX_land_equipment_role_icons`),
`research-date` (Clausewitz date → year), `progressbar-fraction` (0..1), `idea-trait-frame`.

## 1a. Research panel — dynamic (data-bound) elements

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `research_slots_grid` (gridboxType, in `slots_window`) → **N × `research_slot_entry`** | row factory over `countries.<TAG>.research.in_progress[]`, padded to `research.research_slots` empty slots | row factory (see per-row children below) | **exact** (slot count) / **derived** (per-row progress) | One row per active-research slot. Filled rows = `in_progress[]` (5 in sample); empty rows = `research_slots − len(in_progress)` (here 5 slots, 5 active → 0 empty). Slot COUNT is `exact` (`research_slots=5`); per-row progress is `derived`/`unavailable` (no stored fraction — see `research_progressbar`). |
| ↳ `research_slot_entry.title` (instantTextboxType, no placeholder) | `in_progress[<i>].name` → loc | loc (tech token→name) | **exact** | The tech being researched (e.g. `"excavation3"`). Token→display name is a static loc lookup. |
| ↳ `research_slot_entry.technology` (iconType, `GFX_research_folder_strip`) | `in_progress[<i>].name` → the tech's research category/folder | tech-folder-frame | **derived** | Folder icon is chosen from the tech's category (static tech-DB lookup keyed by tech name); the name is `exact`, the frame mapping is a derive. |
| ↳ `research_slot_entry.equipment` (iconType, `GFX_land_equipment_role_icons`) | `in_progress[<i>].name` → equipment archetype role | equipment-role-icon | **derived** | Role icon for techs that unlock equipment; from the static tech→equipment map. Hidden for non-equipment techs (visibility = has-equipment). |
| ↳ `research_slot_entry.designer` (iconType, `GFX_research_folder_strip`) | `countries.<TAG>.mios[]` / `scheduled_variants[]` (designer MIO bonus on this tech) | tech-folder-frame | **unavailable** (current data) | The designer/MIO tile needs the designer-bonus join; `mios` domain is **not folded in this snapshot** (`designs`-sourced). Badge unavailable. |
| ↳ `research_slot_entry.carrier_fighter` (iconType, `GFX_airwing_carrier_icon`) | `in_progress[<i>].name` (carrier-variant tech?) | category (bool→icon) | **derived** | Carrier-fighter overlay; shown only for the specific carrier-air techs (static flag). Visibility-only. |
| ↳ `research_slot_entry.year` (instantTextboxType) | `in_progress[<i>].name` → the tech's defined year (static tech-DB) | int (year) | **unavailable** (current data) → **derived** once the tech-DB is loaded | The tech's *historical year* is a static tech-DB attribute, not a per-country field; not in `in_progress[]` (only `{name, points_factor}`). **Codex:** badge `derived` IF the adapter supplies tech defs (it's adapter/static data, not snapshot-bound), else `unavailable`; it is `unavailable` now (no tech-DB join folded). (Completed techs DO carry `technologies[].date` — that feeds the tech-tree node, §1c, not this row.) |
| ↳ `research_slot_entry.eta` (instantTextboxType, format right) | `in_progress[<i>]` remaining-days ETA | int (days) / `loc` | **unavailable** | In-game ETA = (cost − accumulated)/daily-speed. The SAVE channel stores only `points_factor` (a speed multiplier), **not** accumulated progress or an ETA; `in_progress_live` (which carries `progress_units`) is `None` here. Badge unavailable. |
| ↳ `research_slot_entry.research_progressbar` (iconType, `Gfx_research_progressbar`) | `in_progress[<i>].progress_pct` | progressbar-fraction | **unavailable** | Per-slot progress fraction is `M*` in SCHEMA §3.4 (null in save) and unfed here (`in_progress_live=None`). Render the frame inert / badged. |
| `research_groups_grid` (gridboxType, in `research_win_top`) → research-category tiles | `countries.<TAG>.research.research_bonuses[]` grouped by `categories[]` | row factory; per tile: `bonus`→percent, `categories`→folder frame | **exact** | The focus/research-bonus tiles. `research_bonuses[]` is fully present (20 rows in sample, each `{name, categories[], bonus, kind}`). Tile shows the active bonus per category. |
| `research_speed_value` (instantTextboxType, placeholder `999`) | `countries.<TAG>.scalars.research_speed.pct` (SCHEMA §3.13, `D`) | percent | **unavailable** (current data) | Net research-speed modifier. `scalars.research_speed` is schema-present but **`D` (derived) and not in this snapshot**. Badge unavailable; once the scalars model runs it becomes `derived`. |
| `limited_research_bonus_value` (instantTextboxType, placeholder `999`) | best-applicable entry of `research.research_bonuses[]` (the capped "limited" bonus) | percent | **derived** | The "limited research bonus" number is an aggregate the engine computes from the bonus stack; we have the raw `research_bonuses[]` and can derive the displayed value, but it is a compute, not a stored scalar. **Codex caveat:** `derived` ONLY for bonuses represented in `research_bonuses[]` — the category applicability / stacking / display semantics still need adapter logic (it is not a pure field read). |
| `technology_idea_entry.idea_traits` (iconType, `GFX_idea_traits_strip`) — template tile | `countries.<TAG>.politics.ideas_other[]` (research-affecting ideas/advisors) | idea-trait-frame | **unavailable** (current data) | This idea-tile template surfaces research-modifying ideas/advisors in the folder strip. `ideas_other` IS present in schema (`S`) but this snapshot's politics carries it only as a flat token list with no trait-frame mapping folded; the trait-frame index is not stored. Badge unavailable. |

## 1b. Research panel — STATIC / decoration / navigation (no data binding)

Pure chrome, tab buttons (read-only "switch sub-tab" verbs), section headers (static loc), and
positioning containers. Listed so the map is provably exhaustive.

- **Structural containers / windows:** `countrytechnologyview` (root), `tabs`, `research_slots`
  (sub-tab body), `slots_window`, `research_win_top`, `research_win_bottom`, `facilities` (empty
  sub-tab body — no children in this `.gui`), `technology_idea_entry` (template root),
  `technology_sharing_entry` (template root), `research_slot_entry` (template root).
- **Backgrounds / frames:** every `Background` (`GFX_tiled_plain_bg`, `GFX_tiled_window2_1b_border`,
  `GFX_tiled_generic_overlay_bg1`, `GFX_tiled_window_transparent`, `GFX_research_top_win`,
  `GFX_production_win_bottom`, `GFX_research_line_bg`, `GFX_tech_idea_bg`, `GFX_commonwealth_research`),
  `production_header_bg` (`GFX_header_bg`), `research_progressbar_frame_icon`
  (`GFX_research_progressbar_frame`), `empty_research_slot_glow` (cosmetic glow on an empty slot —
  visibility-only, no datum), `focus_bonuses` (`GFX_focus_bonuses_icon`), `research_speed_icon`
  (`GFX_research_speed_icon`).
- **Static label text (loc constants):** `research_title` (`RESEARCH_RESEARCH`),
  `limited_research_bonus_text` (`LIMITED_RESEARCH_BONUS`), `research_speed_text` (`RESEARCH_SPEED`).
- **Navigation / tab buttons (read-only verbs, no datum):** `research_slots_tab`
  (`RESEARCH_TAB_SLOTS`), `facilities_tab` (`RESEARCH_TAB_FACILITIES`), `close_button` (`CLOSE`).
- **Positioning anchors (positionType, no visual):** `technology_sharing_offset`,
  `technology_sharing_faction_offset`. Plus the layout `margin`/`borderSize`/`slotsize` structural
  nodes (8 × `borderSize`, 2 × `slotsize`, 1 × `margin`) — geometry only.

## 1c. Research — TECH-TREE NODE BINDING (adapter-fed; nodes are NOT in this `.gui`)

The full tech-tree GRID (each tech as a positioned node with link lines) is **not** in
`countrytechnologyview.gui` — it lives in `countrytechtreeview.gui` and is rendered by the **tech
adapter** (PLAN §B.5): the adapter parses the per-folder blocks → `{techId → rect}` and draws link
lines from prereqs (**geometry is static, adapter-fed — NOT a declarative element bind**). This
section maps each tech NODE's *data* state onto our `research` domain.

| node state (per tech) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| **completed level** (researched / leveled) | `countries.<TAG>.research.technologies[]` → match `name`, read `level` | int (level) | **exact** | 122 completed rows in sample; `{name, level, date, research_points}`. A tech present with `level≥1` renders as researched; `level` drives multi-level tech tiers (e.g. `infantry_weapons` levels). |
| **completion date** (node "year" / timeline) | `technologies[].date` | research-date (Clausewitz→year) | **exact** | e.g. `"1936.1.1.12"`. Feeds the node's done-state + the research-completion event timeline (diff `tech_researched`). |
| **research points spent** (node tooltip) | `technologies[].research_points` | int | **exact** | e.g. `165.0`. The tech's point cost as recorded. |
| **in-progress** (node currently researching) | `countries.<TAG>.research.in_progress[]` → match `name` | category (researching frame) | **exact** (membership) / **unavailable** (fraction) | A tech in `in_progress[]` renders the "researching" node frame. The PROGRESS FRACTION is `unavailable` (no stored pct; `in_progress_live=None`; SCHEMA §3.4 `progress_pct` is `M*`). |
| **not-yet-researched** (available / locked) | absence from both `technologies[]` and `in_progress[]` | category (available/locked frame) | **derived** | A tech absent from both lists is un-researched. available-vs-locked needs the prereq graph (adapter-resolved from the tech-DB) → derive; we have the completed set to test prereqs against. |
| **research slots** (how many parallel slots) | `countries.<TAG>.research.research_slots` (+ `original_research_slots`) | int | **exact** | `5` (orig `4`) in sample. Bounds how many `in_progress[]` rows can exist; drives the `research_slot_gained` diff event. |
| **research bonuses** (folder discount) | `countries.<TAG>.research.research_bonuses[]` (`{name, categories[], bonus, kind}`) | percent + category | **exact** | 20 rows. Per-folder/per-category bonus tiles; also tints the tree's folder tabs. |
| **link lines / prereq edges** | tech-DB prereq graph (static) | — | **adapter-fed** | Edges come from the tech definitions, not the snapshot. The completed set decides which edges render as "satisfied". Not a declarative element bind (PLAN §B.5). |
| **node rect / position** | `countrytechtreeview.gui` folder blocks (static) | — | **adapter-fed** | Positions are static, embedded in the tech-tree `.gui` (PLAN §B.5: "easiest tree, first proof"). Not a snapshot field. |

---

# VIEW 2 — NATIONAL FOCUS tree (`game_assets/gui/national_focus.json`, `nationalfocusview.gui`)

**This is a GRAPH-pattern tree (PLAN §B.5).** The `.gui` supplies only the **frame, search box, zoom
slider, the `drag_scroll` scrollable `tree` container, a node prototype (`national_focus_item`), a
continuous-focus item prototype, a link prototype, an exclusive-link prototype, and the detail/​
shortcut panels** — **NOT the focus nodes themselves.** Focus node positions live in
`common/national_focus/*.txt` (base `x`/`y` + `relative_position_id` + conditional `offset` blocks)
and the prereq/`mutually_exclusive` link lines come from those files too. Pure declarative binding
**breaks** here (PLAN §C.2 "Graph + link lines" → adapter required). So: tables 2a/2b cover the `.gui`
chrome + the node-PROTOTYPE's bindable children; section **2c (NODE-BINDING)** maps each focus node's
data state onto our `focus` domain; node geometry + link routing are **adapter-fed**.

Selected/observed country = `<TAG>`. Formatters: `loc`, `int`, `percent`, `progressbar-fraction`,
`flag-small`, `goal-symbol` (focus icon sprite), `category` (state→frame).

## 2a. Focus tree — dynamic (data-bound) elements (the node PROTOTYPE + detail panel)

The node prototype `national_focus_item` (and the continuous variant) is **instantiated once per
focus** by the adapter; its bindable children are mapped here (each binds the per-focus datum the
adapter supplies). The `national_focus_detail_view` binds the *selected* focus.

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `national_focus_item.symbol` (buttonType, `GFX_goal_unknown`) | per-focus icon (focus-DEF `icon`) + state from `focus.completed`/`focus.current` | goal-symbol + category(state) | **exact** (state) / **derived** (icon) | Node icon. STATE (done/active/available/unavailable) is `exact` from our `focus` domain (see 2c); the base icon sprite is a focus-DEF lookup (derive). |
| `national_focus_item.bg` (buttonType, `GFX_technology_unavailable_item_bg`) | per-focus state ← `focus.completed[]` ∋ id ? done : (`focus.current`==id ? active : avail/unavail) | category (state→bg frame) | **exact** | The node background frame encodes completed/selected/available/unavailable. Driven entirely by our `focus.{completed,current}` (2c). `unavailable` (locked) vs `available` needs the prereq graph → adapter (still derivable from `completed`). |
| `national_focus_item.name` (instantTextboxType) | per-focus id → loc | loc (focus token→name) | **exact** | Focus display name. Token from the adapter's node list; loc is static. |
| `national_focus_item.continuous_glow` (iconType, `GFX_ongoing_focus_goal`) | visible iff `focus.current` == this focus id | visibility | **exact** | The "currently working on this" glow. `focus.current="GER_the_swiss_gold"` in sample → exactly one node glows. |
| `national_focus_item.highlight_glow` (iconType, `GFX_highlight_focus_goal`) | search/selection highlight (UI state, not country data) | visibility | static/UI | Highlight from the search box; replay-local UI state, no datum. |
| `national_focus_item.overlay` (iconType, no sprite) | per-focus overlay (focus-DEF, e.g. branch icon) | category | **unavailable** (current data) | Optional per-focus overlay sprite from the focus-DEF; not a country field and no overlay map folded. Badge unavailable. |
| `national_focus_item.historical` (iconType, `GFX_own_chat`, tooltip `HISTORICAL_FOCUS`) | per-focus `will_lead_to_war`/historical flag (focus-DEF) | visibility | **unavailable** (current data) | "Historical AI focus" marker is a focus-DEF attribute, not country state. Badge unavailable. |
| `national_focus_item.viewing_flag` (iconType, `GFX_flag_small2`) | which other observed country is viewing/also-has this focus | flag-small | **unavailable** | Multi-country "who else is on this focus" overlay; needs a cross-country focus join not modeled per-node. Badge unavailable; `viewing_flag_border` is its static frame. |
| `continuous_national_focus_item.name` (instantTextboxType) | continuous-focus id → loc, from `focus.continuous_focus_palette` / `focus.current_continuous` | loc | **derived** | Continuous-focus tile name. `continuous_focus_palette="generic_focus"` gives the palette; `current_continuous` (the active one) is `M`/live-only and `null` in this save snapshot → which tile is *active* is `unavailable`, but the tile LIST is derivable from the palette. |
| `continuous_national_focus_item.focus_cost` (instantTextboxType, placeholder `2`) | per-continuous-focus PP cost (focus-DEF) | int | **unavailable** | PP cost is a focus-DEF constant, not country state. Badge unavailable. |
| `continuous_national_focus_item.continuous_glow` (iconType, `GFX_ongoing_focus_goal`) | visible iff `focus.current_continuous` == this id | visibility | **unavailable** (current data) | `current_continuous` is `M` live-only and unfed in this save snapshot (`null`). Badge unavailable. |
| `progress_focus_item.checked_icon` (iconType, `GFX_cont_focus_check_icon`) | continuous-focus weekly-progress checkmarks | category | **unavailable** | Per-week progress ticks for continuous focus; no per-week progress array in `focus`. Badge unavailable. |
| `continuous_focus_progress` (instantTextboxType, in `focus_progress_window`) | `focus.progress` for the active *continuous* focus | percent/int | **unavailable** (current data) | The continuous-focus progress readout. Our `focus.progress` (=12) is days-into the *national* `current`, not the continuous one; no separate continuous-progress field. Badge unavailable. |
| **DETAIL VIEW** `national_focus_detail_view.name` (instantTextboxType) | selected focus id → loc | loc | **exact** | The detail panel binds the *clicked* node. Name is loc of the selected focus id. |
| `national_focus_detail_view.symbol` (iconType, `GFX_goal_unknown`) | selected focus icon (focus-DEF) | goal-symbol | **derived** | Selected focus's icon sprite (focus-DEF lookup). |
| `national_focus_detail_view.status` (instantTextboxType) | selected focus state ← `focus.completed`/`focus.current`/`focus.progress` | loc (state) | **exact** | "Completed" / "In progress (N/M days)" / "Available". Derived from our `focus` domain. |
| `national_focus_detail_view.research_progressbar` (iconType, `GFX_prod_progress_bar3`) | `focus.progress` / focus duration (selected==`focus.current`) | progressbar-fraction | **derived** | Progress bar for the active focus. We store `focus.progress` in DAYS (=12); the focus DURATION (denominator, typically 70 days × cost) is a focus-DEF constant → fraction is a derive (days ÷ def-duration). Bar shown only when the selected focus is `focus.current`. |
| `national_focus_detail_view.prerequisites` (instantTextboxType) | selected focus prereq ids → loc, tinted by `focus.completed[]` | loc + category(satisfied) | **derived** | Prereq list text; which prereqs are satisfied is testable against `focus.completed[]` (exact), but the prereq SET is focus-DEF data (adapter-supplied). |
| `national_focus_detail_view.desc` / `.reward` (instantTextboxType) | selected focus description / reward effect (focus-DEF) | loc | **unavailable** (current data) | Description + reward-effect text are focus-DEF/loc, not country state. Badge unavailable (renders the loc string when the focus-DEF is loaded by the adapter, but not from the snapshot). |
| `focus_tree_shortcut_item.name` + `.focus_button` (in `shortcut_grid`) | branch shortcuts of `focus.focus_tree` (=`"german_focus"`) | loc + goal-symbol | **derived** | The bottom branch-jump shortcuts. Which tree (and thus which branches) is `exact` (`focus_tree`); the per-branch entries are derived from the focus-tree DEF (adapter). |

## 2b. Focus tree — STATIC / decoration / navigation / search (no data binding)

- **Structural containers / windows:** `nationalfocusview` (root), `find_view`, `zoom_slider_container`,
  `a_focus_background`, `tree` (the `drag_scroll` canvas — special widget, PLAN §B.5),
  `techtree_stripes`, `grid_window`, `grid` (gridbox — the node host the adapter fills),
  `focus_tree_scripted_window_container`, `continuous_focus_window`, `entries`, `search_tooltip`,
  `filter_container`, `filter_grid_main_container` / `_bg_container` / `_container`,
  `shortcut_grid_container`, and the template ROOTS (`continuous_national_focus_item`,
  `national_focus_item`, `progress_focus_item`, `national_focus_link`,
  `national_focus_exclusive_item`, `national_focus_detail_view`, `coninuous_focus_detail_view` [sic],
  `focus_tree_filter_icon_container`, `focus_tree_alternate_icon_button_container`,
  `focus_tree_shortcut_item`, `focus_tree_filter_show_rest`).
- **LINK-LINE prototypes — adapter-fed geometry, NOT a declarative bind (PLAN §B.5):**
  `national_focus_link.link` (`GFX_focus_link_up_down`) and the exclusive-link tiles
  (`national_focus_exclusive_item.{link1,link2,left,right,mid}`, all `GFX_focus_link_exclusive*`).
  These prototypes are *instantiated by the adapter per edge*; the routing comes from the focus
  script's prereq/`mutually_exclusive` lists, not from any country-snapshot field. Flagged
  **adapter-fed** (their per-edge presence is in 2c).
- **Backgrounds / frames:** every `Background` (`GFX_tiled_plain_bg2`, `GFX_tiled_focus_bg`,
  `GFX_tiled_window_thin_border2`, `GFX_tiled_paper_bg2`, `GFX_mini_tooltip`, `GFX_focus_tooltip`,
  `GFX_tiled_window_transparent`), `find_editbox_bg`, `info_top_win`, `prerequisites_bg`,
  `reward_bg`, `research_progressbar_frame`, `statsarea`.
- **Static label text (loc constants):** `national_focus_title` (`NATIONAL_FOCUS_TITLE`),
  `continuous_focus_title` (`CONTINUOUS_FOCUS_TREE_HEADER`), `continuous_focus_desc`
  (`CONTINUOUS_FOCUS_TREE_DESC`), `type_to_search_text` (`FOCUS_SEARCH_TYPE_TEXT`), `reward_label`
  (`FOCUS_REWARD`), `tooltip_text` (placeholder "Hello world!"), the detail-view `desc`/`reward`
  placeholders.
- **Search / filter / zoom / nav (read-only or replay-local UI verbs, no country datum):**
  `to_find_text` (editBox — search input), `found_text`, `open_focus`, `clear_search`, `zoom_in` /
  `zoom_out` (both containers), `filter_icon_grid` / `filter_grid` (the focus-category filter — UI
  filter state), `focus_tree_filter_icon_container.{checkbox,icon}`,
  `focus_tree_alternate_icon_button_container.{checkbox,icon}`, `focus_tree_filter_show_rest.toggle`,
  `toggle_shortcuts`, `close_button`, the detail-view `close` buttons, `start` / `stop` buttons
  (focus-START/STOP are **mutations** → inert/read-only in a replay, PLAN §D), the 4 `tutorial_glow_*`
  icons (cosmetic), `continuous_glow` on the prototype when used as decoration.
- **Positioning anchors (positionType, no visual):** `focus_spacing`, `national_focus_center`,
  `link_spacing`, `link_offsets`, `link_begin`, `link_end`, `exclusive_offset`,
  `exclusive_offset_left`, `exclusive_positioning`, and the 5 `search_tooltip` `pos_*` anchors. These
  are the anchors the focus adapter's geometry pass targets — no datum.

## 2c. Focus — NODE BINDING (adapter-fed; nodes are NOT in the `.gui`)

Each focus node's *data* state maps onto our `focus` domain. The node's RECT (position) and the link
edges are **adapter-fed** from `common/national_focus/*.txt` (PLAN §B.5), not the snapshot.

| node state (per focus) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| **completed** (node done) | `countries.<TAG>.focus.completed[]` ∋ focus id | category (done frame) | **exact** | 55 completed ids in sample (e.g. `GER_remilitarize_the_rhineland`). Membership test → the node's done frame + checkmark. Drives the `focus_completed` diff event. |
| **completed count** | `countries.<TAG>.focus.completed_count` | int | **exact** | `55`. Header/summary count; cross-checks `len(completed)`. |
| **current** (in-progress node) | `countries.<TAG>.focus.current` | category (active frame) + glow | **exact** | `"GER_the_swiss_gold"`. Exactly the node that renders the ongoing-glow + active frame. |
| **progress** (days into current) | `countries.<TAG>.focus.progress` | int (days) → progressbar-fraction | **exact** (days) / **derived** (fraction) | `12` days. The raw days value is `exact`; the BAR fraction needs the focus's total duration (focus-DEF, ~70×cost days) → `derived` (days ÷ def-duration). Only the `current` node shows a bar. |
| **available / locked** (un-started node) | absence from `completed`, ≠ `current`, prereqs ⊆ `completed[]` | category (available vs unavailable frame) | **derived** | A focus not done and not current renders available iff all its prereqs are in `completed[]` (prereq set is focus-DEF, adapter-supplied; the satisfied-test uses our exact `completed[]`). |
| **which tree** (node set selection) | `countries.<TAG>.focus.focus_tree` | loc / adapter-key | **exact** | `"german_focus"`. Selects which focus-DEF tree (and thus which node set + geometry) the adapter loads. |
| **continuous focus** (active continuous) | `countries.<TAG>.focus.current_continuous` (`M`, live-only) | category | **unavailable** (current data) | The active continuous focus's glow; `current_continuous` is unfed in this save snapshot (`null`). The palette (`continuous_focus_palette="generic_focus"`) IS present → the tile LIST is derivable, but the ACTIVE one is unavailable. |
| **paused** (focus paused) | `countries.<TAG>.focus.paused` | bool → category | **exact** | `false` in sample. Pauses the active node's progress display. |
| **link edges** (prereq + mutually-exclusive lines) | focus script prereq/`mutually_exclusive` lists (static) | — | **adapter-fed** | Edges from the focus DEF, not the snapshot. Whether a prereq edge renders "satisfied" tests against `completed[]` (exact). Not a declarative element bind (PLAN §B.5). |
| **node rect / position** | `common/national_focus/*.txt` (`x`/`y`/`relative_position_id`/`offset` triggers) | — | **adapter-fed** | Positions + conditional offsets come from the focus script via the §E offset-trigger evaluator. Not a snapshot field. |

---

# VIEW 3 — MENUBAR (`game_assets/gui/menubar.json`, `interface/menubar.gui`)

**Scope note.** `menubar.gui` is a grab-bag root: it holds the actual bottom-right **menubar** window
(nav buttons, minimap, zoom, floating map-mode numbers), plus several detachable sub-windows that
ship in the same file — the **war-overview** shield list, the **endgame** score-ladder dialog, the
**message log** + **message-settings** + **message-icons** + **chat** windows, and the
**save-game**/**province-finder** list-entry templates. Most are nav chrome or replay-inert
mod/MP/message machinery; the genuinely data-bound parts are the **endgame ladder** (a country
score table) and the **war-overview/endgame faction/country** entries. Observed country = `<TAG>`.

Formatters: `int`, `loc`, `flag` / `flag-shield` (maskedShield), `category` (faction/ideology→sprite),
`clock-date`, `score-int`.

## 3a. Menubar — dynamic (data-bound) elements

| gui element (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `minimap_pic` (iconType, `GFX_minimap`) | top-level `map.owner_by_state` / `map.control_by_province` (raster) | map-raster | **derived** | The minimap recolors from the map layers (present in the snapshot's `map` block). Region granularity → derived raster, not a per-pixel exact. |
| `endgame_country_entry.flag` (iconType, `GFX_flag_small`) | per-row `<TAG>` (ladder country) | flag-small | **exact** | One row per country in the endgame ladder; the TAG resolves the flag. |
| `endgame_country_entry.country_name` (instantTextBoxType) | per-row `<TAG>` → loc | loc (TAG→name) | **exact** | Country display name. |
| `endgame_country_entry.pol_faction_icon` (iconType, `GFX_pol_factions_strip`) | `countries.<TAG>.politics.ruling_party` (ideology) | category (ideology→frame) | **derived** | Ideology/faction-colour tile; `ruling_party` is present (`S`/`M`). Frame index is an ideology→frame lookup. |
| `endgame_country_entry.faction_name` (instantTextBoxType) | `countries.<TAG>.politics.faction.name` → loc | loc | **exact** | Faction membership name (`"faction_template_axis"` in sample). |
| `endgame_country_entry.sum` (instantTextBoxType, placeholder "muscles") | `countries.<TAG>.score.overall` | score-int | **unavailable** (current data) | Total victory score. `countries.*.score.*` is schema-present (`D`, §3.14) but **not folded in this snapshot**. Badge unavailable until the victory-score model runs. |
| `endgame_country_entry.army` (instantTextBoxType, placeholder "helmets") | `countries.<TAG>.score.army` | score-int | **unavailable** (current data) | Army score column. Same `score`-block gap. (Raw proxies like `military.division_count` exist but are not the endgame score.) |
| `endgame_country_entry.navy` (instantTextBoxType, placeholder "boats") | `countries.<TAG>.score.navy` | score-int | **unavailable** (current data) | Navy score column. Same gap. |
| `endgame_country_entry.air` (instantTextBoxType, placeholder "airplanes") | `countries.<TAG>.score.air` | score-int | **unavailable** (current data) | Air score column. Same gap. |
| `endgame_country_entry.industry` (instantTextBoxType, placeholder "factories") | `countries.<TAG>.score.industry` | score-int | **unavailable** (current data) | Industry score. `economy.factories.*` exists as a raw proxy but is not the endgame industry SCORE. Badge unavailable. |
| `endgame_country_entry.land` (instantTextBoxType, placeholder "bits of ground") | `countries.<TAG>.score.territory` | score-int | **unavailable** (current data) | Territory score. Same `score`-block gap. |
| `endgame_faction_entry.name` (instantTextBoxType) | `diplomacy_world.factions[].leader`/id → loc | loc | **derived** | Faction-row name; `diplomacy_world.factions` is the world faction list. Present-but-sparse in this snapshot → derive/unavailable per row. |
| `endgame_faction_entry.members` (instantTextBoxType) | `len(diplomacy_world.factions[].members[])` | int | **derived** | Member count per faction. |
| `endgame_faction_entry.strength` (instantTextBoxType, tooltip `AVERAGE_MEMBER_STRENGTH`) | aggregate of member `score`/strength | int | **unavailable** (current data) | Average member strength needs the per-country `score` block (unfed). Badge unavailable. |
| `endgame_dialog.country_name` (instantTextBoxType) | observed/winner `<TAG>` → loc | loc | **exact** | The featured country's name in the endgame summary. |
| `endgame_dialog.country_flag` (iconType, `GFX_shield_medium`) | observed `<TAG>` + `politics.ruling_party` | flag-shield (ideology-tint) | **exact** | Featured-country shield (TAG + ideology tint). |
| `endgame_dialog.country_score` / `country_score_pos` (instantTextBoxType) | `countries.<TAG>.score.overall` / `.rank` | score-int / int | **unavailable** (current data) | Featured score + ladder position. `score` block unfed → unavailable. |
| `endgame_dialog.leader` (instantTextBoxType) | `countries.<TAG>.commanders.leader.name` | loc | **unavailable** (current data) | Country leader name. `commanders` domain not folded in this snapshot. Badge unavailable. |
| `endgame_dialog.ideology` (instantTextBoxType) | `countries.<TAG>.politics.ruling_party` (sub-ideology) → loc | loc | **derived** | Ideology line; `ruling_party` present. The fine sub-ideology may need `party_popularities` (present `M/S`). |
| `endgame_dialog.government` / `.party` (instantTextBoxType) | `countries.<TAG>.politics.ruling_party` / `commanders.party_leaders[]` | loc | **derived** / **unavailable** | Government type derives from `ruling_party` (derived); the named ruling PARTY needs `commanders` (unfed) → unavailable. |
| `endgame_dialog.country_leader` (iconType, `GFX_leader_unknown`) | `countries.<TAG>.commanders.leader.portrait_gfx` | portrait-sprite | **unavailable** (current data) | Leader portrait; `commanders` unfed + portrait is an asset-extraction step. Badge unavailable. |
| `historical_score_item.country_shield` (guiButtonType, `GFX_shield_small`) | per-row `<TAG>` | flag-shield | **exact** | Historical-score ladder row shield. |
| `historical_score_item.country_name` (instantTextBoxType) | per-row `<TAG>` → loc | loc | **exact** | Row country name. |
| `historical_score_item.score` (instantTextBoxType) | `countries.<TAG>.score.overall` (historical series) | score-int | **unavailable** (current data) | Historical score value; `score` block unfed. Badge unavailable. |
| `waroverview_shieldItem.waroverview_icon` (iconType, `GFX_shield_medium`) + `button_war_overview_item` | per-war belligerent `<TAG>` (from `diplomacy_world.wars`) | flag-shield | **derived** | War-overview shows the wars' major belligerents; `diplomacy_world.wars` present. Which shields to show is an adapter aggregation over the war list. |
| `diplomessageicon_entry_window.diplo_flag_overlay` (guiButtonType, `GFX_flag_smallest`) | per-message sender `<TAG>` | flag-smallest | **unavailable** | Diplomatic-message icons; the message feed is an imperative alerts adapter (PLAN §C.4), out of the binding sweep. Badge unavailable (and see 3b deferred). |

## 3b. Menubar — deferred (needs imperative adapter; NOT bound in this sweep)

These render as real chrome but are **inert**; their content is an imperative feed (message log /
alerts / chat / save-list), not a declarative country-field bind (PLAN §C.4).

| gui element(s) | why deferred |
|---|---|
| `messagelog_window.messagelog` (listbox) + `message_entry` template (`message`,`text`,`messageicon_pause`,`messageicon_icon`) + `message_item` template (`new_flash`,`icon`,`flag`,`title`,`other_flag`) + `messageicon_entry_window` (`messageicon_button`,`flag`,`msgTypeIcon`) | **deferred: needs adapter** — the in-game message LOG/alerts feed is an imperative adapter (PLAN §C.4 alerts/outliner). Markers would come from `snapshot.events[]` / the diff timeline, not a per-element bind. |
| `message_icons` OverlappingElementsBoxes (`combat`,`diplo`,`government`,`trade`,`event`) + `diplomessageicons_window` + `message_window.message_list` | **deferred: needs adapter** — per-category message-icon stacks; same alerts adapter. |
| `chat_window` (+ `none`/`allies`/`enemies`/`all`/`closebutton`), `chat_list`, `chat_entry` (`text`,`text_shadow`), `logtext.messagelogbutton` | **deferred: needs adapter** — MP CHAT is live-session machinery; no replay datum. Inert. |
| `menu_message_settings` (entire window: `all`/`other` checkboxes, `filter_button_01..04`, the 10 `messagesettings_label_*`, `reset`/`close`/`disable_all_messages`) | **deferred / N-A** — message-FILTER preferences UI; pure settings, no game data. Effectively static, grouped here as the message subsystem. |
| `menu_unsync` (`disk_space_needed`,`cloudsave_unsync_label`,`games` listbox,`cancel`) | **deferred / N-A** — cloud-save-unsync dialog; SP/cloud machinery, never shown in a replay. |
| `save_game_entry` / `ingamesavegameentry` / `province_finder_entry` templates (`save_game`,`title`,`save_game_date`,`save_game_shield`,`cloud_file`,`ironman_icon`,`tutorial_icon`,`button`) | **deferred: needs adapter** — save-list + province-finder are file-browser / search adapters, not country-data binds. The province-finder COULD bind province metadata later (then `derived`); inert now. |

## 3c. Menubar — STATIC / decoration / navigation (no data binding)

- **The menubar nav window itself (`menubar`) — read-only "open view" verbs + chrome, no datum:**
  `zoom_in` / `zoom_out` (map zoom — replay-inert, the camera owns zoom), `more_button`,
  `closebutton` / `OPENbutton` (log open/close), the 4 `menubar_floating_bg_1..4`
  (`GFX_menubar_floating_bg`), and the 4 `floating_number_1..4` (placeholder `100`) — these are the
  **map-mode floating legend numbers**; they reflect the active MAP MODE's legend (a UI/render state,
  not a country field) → static/UI, not bound.
- **War-overview / endgame structural containers (no datum themselves):** `ButtonMenuWindow`,
  `ButtonMenuButton` (`GFX_countryaction_gotolocation_big`), `WarOverviewButtonWindow` +
  `WarOverviewButtonList` (Overlapping box — the adapter fills it, see 3a), `endgame_dialog` (root),
  `info` / `factions_info` / `countries_info` / `countries_info_list` containers,
  `factions_list_container` + `faction_list` (gridbox — adapter-filled faction rows),
  `country_list` (gridbox — adapter-filled ladder rows, the row template is `endgame_country_entry`
  in 3a), `endgame_faction_entry` / `endgame_country_entry` / `historical_score_item` /
  `waroverview_shieldItem` / `diplomessageicon_entry_window` / `message_item` / `message_entry`
  template ROOTS (their bound children are in 3a/3b).
- **Endgame ladder SORT buttons (read-only verbs) + static column-header labels:** `country_icon`
  (`LEDGER_COUNTRY`), `faction_icon` (`FACTION`), `score_icon`, `army_icon`, `navy_icon`, `air_icon`,
  `industry_icon`, `province_icon` (all `ENDGAME_SORT_*` tooltips), and the `info`/`factions_info`
  label texts (`SCORE`, `LEADER`, `IDEOLOGY`, `GOVERNMENT`, `RULING_PARTY`, `FACTION_LABEL`,
  `STRENGTH_LABEL`, `MEMBERS_LABEL`, `country_score_label`).
- **Backgrounds / frames / cosmetic icons:** `ladder_background` (`GFX_ladder_background`), `bg`
  (`GFX_tiled_window_transparent`), `country_leader_frame` (`GFX_leader_frame_0_75`), `flag_frame`
  (`GFX_large_flag_frame`), `messagesettings_bg`, `log_bg`, `messageicon_bg`, `messageicon_pause`,
  the message-category checkboxes (`messagecat_combat`/`_diplomacy`/`_government`/`_events`,
  `logfilter_interesting`, `msg_icons_filter_interesting`), `new_flash`.
- **Positioning anchors (positionType, no visual):** `ButtonMenu_offset_1..5`, `minimap_size`,
  `messageicons_offset`, `diplomessageicon_offset`, `diplomessageicon_start`, `chat_shield_start`,
  `chat_shield_offset`, `chat_shield_offset_long`, `war_overview_list_start` — anchors the adapters
  target; no datum. Plus the structural `hide_position` (4×), `margin`, `padding`, `borderSize` (66×),
  `slotsize` (2×) layout nodes.

---

## Notes

**A. The research `.gui` is the side panel, not the tech grid.** `countrytechnologyview.gui` is the
left-hand research panel (slots + folder-bonus header). The brief's "each tech → completed level /
in-progress / slots / bonuses" mapping is fully captured, but the per-tech NODE binding (the thing the
brief asks for) belongs to the **tech-tree** grid (`countrytechtreeview.gui`) — an adapter-fed
GRAPH-pattern view (PLAN §B.5). Section 1c maps the node DATA onto `research.{technologies,in_progress,
research_slots,research_bonuses}` exactly as required; the node RECTS/edges are adapter-fed (static
`.gui` geometry), flagged as such, not invented as declarative binds.

**B. In-progress vs completed (the trickiest fidelity call).** A tech/focus is COMPLETED via a
MEMBERSHIP test (`technologies[]` for tech, `focus.completed[]` for focus) — both `exact`. A tech/focus
is IN-PROGRESS via `in_progress[]` / `focus.current` — membership `exact`. The PROGRESS FRACTION
differs sharply between the two: focus stores `focus.progress` in DAYS (`exact` raw, `derived` as a
bar fraction since the denominator is a focus-DEF duration), whereas research stores **no** progress
fraction in this snapshot (`in_progress[].points_factor` is a SPEED multiplier, not progress;
`in_progress_live`/`progress_units` is `None`) → research progress bar = `unavailable`. This asymmetry
is the single most important correctness point and is reflected in 1a/1c (research bar `unavailable`)
vs 2a/2c (focus bar `derived`).

**C. Research slots vs bonuses.** `research_slots` (=5) is `exact` and bounds the `in_progress[]` row
count (the slot grid pads to it). `research_bonuses[]` (20 rows, `exact`) feeds the folder-bonus tiles
and the `research_groups_grid`; the displayed NET `research_speed`/`limited_research_bonus` numbers are
ENGINE AGGREGATES — `limited_research_bonus_value` is `derived` (computable from the bonus stack) while
`research_speed_value` maps to `scalars.research_speed.pct` which is `D` and unfed → `unavailable` now.

**D. `unavailable` ≠ "missing forever".** Most `unavailable` badges here are schema-present fields the
current snapshot generator does not yet feed (`in_progress_live`, `current_continuous`, the
`score`/`commanders`/`mios`/`scalars` domains) or focus/tech-DEF static data the adapter supplies
(costs, descriptions, prereq sets, node geometry). They render "—" rather than being omitted
(no-fabrication). Genuinely focus-DEF/tech-DEF values (cost, historical flag, reward text) and engine
aggregates (research-speed, ETA) stay `unavailable`/`derived` by nature for a country-snapshot bind.

---

## Element accounting (provably complete)

### RESEARCH (`research.json`) — type counts (from `.gui` stats): containerWindowType 10, background 9, iconType 11, positionType 2, margin 1, instantTextboxType 8, borderSize 8, buttonType 3, gridboxType 2, slotsize 2 (= **56** elements)

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** (incl. derived/unavailable badges) | **13** | research_slots_grid (+ slot-entry children: title, technology, equipment, designer, carrier_fighter, year, eta, research_progressbar), research_groups_grid, research_speed_value, limited_research_bonus_value, idea_traits |
| **Static / decoration / navigation** | **~43** | root + 7 structural containers, 9 backgrounds + 5 frame/glow icons, 3 label texts, 3 tab/close buttons, 2 positionTypes, 8 borderSize, 2 slotsize, margin |
| **NODE-binding (tech tree, adapter-fed — separate from the 56 `.gui` elements)** | **8 node states** | completed level, date, points, in-progress, available/locked, slots, bonuses, link-lines/geometry |

### NATIONAL FOCUS (`national_focus.json`) — type counts: containerWindowType 30, background 13, instantTextboxType 23, borderSize 19, buttonType 19, iconType 32, editBoxType 1, margin 4, drag_scroll 2, gridboxtype 4, slotsize 6, gridBoxType 2, positionType 14 (= **169** elements)

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** (incl. derived/unavailable badges) | **~22** | node-prototype children (symbol, bg, name, continuous_glow, overlay, historical, viewing_flag), continuous-item (name, focus_cost, glow), progress_focus_item, continuous_focus_progress, detail-view (name, symbol, status, progressbar, prerequisites, desc, reward), shortcut name/button |
| **Adapter-fed link prototypes (geometry, not declarative)** | **6** | national_focus_link.link, national_focus_exclusive_item.{link1,link2,left,right,mid} |
| **Static / decoration / search / nav** | **~141** | 30 containers/template-roots, 13 backgrounds, label/search/filter/zoom/close/start-stop, 14 positionTypes, 4 tutorial glows, structural margins/borders/slotsizes |
| **NODE-binding (focus tree, adapter-fed — the node DATA)** | **9 node states** | completed, completed_count, current, progress, available/locked, focus_tree, continuous, paused, link-edges/geometry |

### MENUBAR (`menubar.json`) — type counts: windowType 21, guiButtonType 27, positionType 13, iconType 22, containerWindowType 8, instantTextBoxType 62, borderSize 66, background 2, gridboxType 2, slotsize 2, ButtonType 8, margin 1, padding 1, hide_position 4, listboxType 3, checkboxType 15, OverlappingElementsBoxType 6, listBoxType 2 (= **265** elements)

| bucket | count | examples |
|---|---:|---|
| **Dynamic-bound** (incl. derived/unavailable badges) | **~24** | minimap, endgame_country_entry (flag, name, pol_faction_icon, faction_name, sum, army, navy, air, industry, land), endgame_faction_entry (name, members, strength), endgame_dialog (country_name, flag, score, score_pos, leader, ideology, government, party, leader portrait), historical_score_item (shield, name, score), waroverview shield, diplo_flag_overlay |
| **Deferred: needs adapter** | **~6 logical groups** (message log, message icons, chat, message-settings, cloud-unsync, save/province-finder lists) | messagelog, message_icons, chat_window, menu_message_settings, menu_unsync, save_game_entry/ingamesavegameentry/province_finder_entry |
| **Static / decoration / navigation** | **~235** | menubar nav window (zoom/more/close/floating bgs+numbers), endgame sort buttons + column labels, all containers/template roots, backgrounds/frames, message-cat checkboxes, 13 positionTypes, 66 borderSize, hide_positions, margins |

Every element in each of the three `.gui` trees falls into exactly one bucket above; the per-element
tables (1a/1b/1c, 2a/2b/2c, 3a/3b/3c) enumerate them by name.

### `unavailable`-badged elements (with reason)

**Research panel (7):** `designer` (mios domain unfed), `year` (static tech-DB year not joined), `eta`
(no accumulated-progress/ETA stored; `in_progress_live=None`), `research_progressbar`
(`progress_pct`=`M*`, unfed), `research_speed_value` (`scalars.research_speed` `D`, unfed),
`idea_traits` (trait-frame map not folded). *Plus the tech-NODE in-progress FRACTION (adapter-fed
section, same `in_progress_live=None` cause).*

**Focus tree (7):** `overlay` (focus-DEF overlay not folded), `historical` (focus-DEF flag),
`viewing_flag` (cross-country focus join), `focus_cost` (focus-DEF PP cost), continuous-focus
`continuous_glow` + `continuous_focus_progress` + `progress_focus_item.checked_icon`
(`current_continuous`/per-week progress unfed), `desc`/`reward` (focus-DEF loc, not country state).

**Menubar (13):** the 6 endgame score columns (`sum`,`army`,`navy`,`air`,`industry`,`land`) +
`country_score`/`country_score_pos`/`historical_score_item.score` + faction `strength` (all the
unfed `score` block), `leader`/`country_leader` portrait + `party` (unfed `commanders` domain),
`diplo_flag_overlay` (message-feed adapter).

**Total `unavailable`: 27 elements** (7 research + 7 focus + 13 menubar). All are schema-present-but-
unfed fields, separate-domain data not folded into THIS snapshot, or focus/tech-DEF static data — none
fabricated; each renders "—" per no-fabrication.

---

## Codex review

Reviewed by Codex (`gpt-5.5`, read-only Mode-A one-shot, 2026-06-26) — the six trickiest node-binding
+ fidelity claims (in-progress vs completed; focus current/progress; the research-vs-focus progress
asymmetry; research slots/bonuses; continuous focus). Verdict: **AGREE on 5, PARTIAL-REFUTE on 1**;
both nuances folded in above.

| claim | Codex verdict | folded-in correction |
|---|---|---|
| **1 — research node binding** (completed=membership+level exact; date exact; in-progress=membership exact; progress bar + ETA `unavailable` because no stored numerator) | **AGREE** | none — `points_factor`=speed, `research_points`=completed-tech cost, `in_progress_live=None` ⇒ no active-progress numerator ⇒ `unavailable` confirmed. |
| **2 — active-row `year`** (`unavailable`; static tech-DB year, not per-country) | **AGREE (wording tweak)** | clarified: badge **`derived` IF the adapter loads tech defs**, else `unavailable`; it is `unavailable` now (no tech-DB join folded). Row updated. |
| **3 — focus node binding** (completed/current exact; progress bar `derived` = days ÷ def-duration; available/locked `derived` via prereqs ⊆ completed) | **AGREE** | none — confirmed the bar fraction is renderable only when the adapter supplies the focus-duration denominator. |
| **4 — the asymmetry** (focus progress `derived` [have days numerator], research progress `unavailable` [no numerator]) | **AGREE** | none — explicitly confirmed: do NOT repurpose `points_factor` or completed `research_points` as a research numerator. |
| **5 — slots/bonuses** (`research_slots` exact + empty-row padding; `research_bonuses[]` exact; net research-speed `unavailable`; limited-bonus `derived`) | **PARTIAL REFUTE** | slots/padding + bonus-rows + research-speed badges confirmed. Caveat added: limited-research-bonus is `derived` ONLY for bonuses in `research_bonuses[]` — category applicability/stacking/display still need adapter logic (not a pure field read). Row updated. |
| **6 — continuous focus** (active glow `unavailable` because `current_continuous`=None; tile LIST `derived` from `continuous_focus_palette`) | **AGREE** | none — the unavailable(active)/derived(list) distinction confirmed. |

Net: no badge in the doc was downgraded; two were tightened with Codex's caveats (research `year`
may upgrade to `derived` once the tech-DB adapter exists; limited-research-bonus `derived` carries an
adapter-logic caveat). All node-binding correctness claims and the research-vs-focus progress
asymmetry — the highest-risk calls — were confirmed.
