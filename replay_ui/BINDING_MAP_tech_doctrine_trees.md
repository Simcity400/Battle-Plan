# BINDING MAP — TECH TREE + DOCTRINE TREES (Phase-3 binding sweep)

> **What this is.** The COMPLETE, no-truncation data-binding map for the two big in-game the game GRAPH
> views the native replay UI binds in the Phase-3 sweep: the **tech tree**
> (`game_assets/gui/tech_tree.json`, from `interface/countrytechtreeview.gui`) and the **doctrine
> trees** (`game_assets/gui/doctrine_tree.json`, from `interface/countrydoctrinetreeview.gui` —
> land / naval / air / special-forces). It realises GAME_UI_REPLICA_PLAN §C.4 ("Research / doctrine"
> row: graph static-geom; tech adapter; exact levels + in-progress) and §B.5 (tree adapters) for these
> two surfaces: EVERY element in each parsed `.gui` tree is accounted for — dynamic (data-bound),
> static/decoration (no binding), or deferred (needs an imperative adapter). Nothing is omitted; where
> a `.gui` element exists but our snapshot has no field for it, the element is listed and **badged
> `unavailable`** with the missing field named.
>
> **Binding-rule shape** (GAME_UI_REPLICA_PLAN §C.1): `{ target (gui element name+type, sprite/text,
> loc key), source (snapshot dotted field path), format (formatter id),
> fidelity (exact|derived|proxy|unavailable) }`. Source paths use SCHEMA dotted form:
> `countries.<TAG>.research.…` (see `replay/SCHEMA.md` §3.4 and `schema.py` `FIELD_SOURCES`).
> `<TAG>` = the player/observed country.
>
> **Both views are GRAPH-pattern trees (§B.5), not flat element lists.** `countrytechtreeview.gui` and
> `countrydoctrinetreeview.gui` are TREE ENGINES: the `.gui` supplies only the FRAME / per-folder
> `drag_scroll` scroll canvas / the per-tech node prototypes — the NODES come from the tech/doctrine
> CATALOG (static script data in `common/technologies/*.txt`) joined with the per-country research
> state. Pure declarative binding **breaks** for the node graph (PLAN §C.4 → tech adapter required).
> So per tree this doc has BOTH: (a) the standard per-element table for the chrome that IS in the
> `.gui`, and (b) a dedicated **NODE-BINDING** section mapping each tech/doctrine node → its snapshot
> fields. Node geometry + link/XOR lines are flagged **adapter-fed**, not a declarative element bind.
>
> **No-fabrication (SCHEMA §0.2, PLAN principle 3):** `null` renders "—", never `0`. `0`/`0.0` is a
> measured zero. A field schema-present but never fed by the current parser is badged `unavailable`
> (not silently dropped). A runtime-computed value (a tech's effect/modifier breakdown, a doctrine
> bonus's combat math, a research ETA) is `derived`/`unavailable` by nature — the save never stores it.

---

## Provenance of the fidelity calls

Calls below are grounded in **empirical inspection of the real save-backed snapshot**
`replay/snapshots/replay/000017005619.snapshot.json` (`schema_version "1"`, date `1941.4.13.11`, GER
1941 save channel, 143 countries), cross-checked against `replay/SCHEMA.md` §3.4 and `schema.py`
`FIELD_SOURCES` (the `research` leaves) + the LIVE `tech_live` leaves
(`countries.*.research.in_progress_live.*.{tech_name,progress_units}`, M), and **adversarially
reviewed by Codex** (read-only, 2026-06-26). Codex's verdict and the corrections folded in are
recorded in the final section.

**Empirically confirmed PRESENT & usable (drives the `exact`/`derived` badges):**

| schema field | observed in GER (sample) | drives |
|---|---|---|
| `research.tech_count` | `122` | header tech-count readout (no slot in these tree `.gui`s; feeds folder summaries) |
| `research.technologies[]` | 122 rows `{name, level, date, research_points}` | completed-tech NODE level + completion date (tech adapter) |
| `research.in_progress[]` | 5 rows `{name, points_factor}` | which NODES render the "researching" frame |
| `research.research_slots` / `original_research_slots` | `5` / `4` | parallel-research cap (bounds `in_progress[]`) |
| `research.research_bonuses[]` | 20 rows `{name, categories[], bonus, kind}` | the per-node `bonus_icon`/`bonus` tile + folder tints |

**Empirically confirmed ABSENT/UNFED (drives the `unavailable` badges):**

| schema field | state in snapshot | consequence |
|---|---|---|
| **DOCTRINE techs in `research.technologies[]`** | **ZERO across all 143 countries** — the world's 187 distinct tech names are *all* equipment/industry/support/electronics; not one doctrine root (`mobile_warfare`, `superior_firepower`, `grand_battle_plan`, `mass_assault`, `base_strike`, `trade_interdiction`, `fleet_in_being`, `strategic_destruction`, `battlefield_support`, `operational_integrity`, …) appears | **the entire doctrine NODE binding is `unavailable` for this snapshot/channel** — the parser that fed this save channel does not extract doctrine technologies. The leaf (`technologies[]`) is generic and would carry them **exact-when-present**; it is **unfed** here, not a schema gap (see Schema-gap ledger #1, and the conditional badge in §D.2). |
| `research.in_progress_live` | **`None`** (save channel; live not folded) | the live active-research progress (`progress_units`) of an in-progress node → `unavailable` (the SAVE `points_factor` is a SPEED multiplier, not progress) |
| `research.in_progress[].progress_pct` | **not present** (SCHEMA §3.4: `M*`, null in save) | the node / detail-window research-progress BAR fraction → `unavailable` (only `points_factor` is stored) |
| `technologies[].research_points` | present BUT is the tech's point COST (sometimes `null`), not a date/ETA | the node's spent-points tooltip = `exact` (where non-null); it is NEVER an ETA/`year` source |
| tech catalog geometry + prereq/XOR graph | not a snapshot field (static `common/technologies/*.txt` + the per-folder containers in this `.gui`) | tech/doctrine-node RECTS + link/XOR lines are **adapter-fed** by the tech adapter, not the snapshot |
| per-tech effect / combat-stat rows (soft/hard attack, org, …) | **not stored by the save** — runtime-computed (SCHEMA §11 note: derived combat stats are a documented hard limit) | every detail-window stat grid / unlock-effect value → `unavailable`/`derived` (tech-DEF + engine math, not country state) |
| `scalars.research_speed.*` | `D` (derived) and **not in this snapshot** | any net research-speed readout (none in these two `.gui`s) would be `unavailable` until the scalars model runs |

---

# VIEW 1 — TECH TREE (`game_assets/gui/tech_tree.json`, `countrytechtreeview.gui`)

**This is the big scrollable tech tree (PLAN §B.5 "easiest tree, first proof").** The `.gui` is a set
of **12 per-category FOLDER windows** (`infantry_folder`, `support_folder`, `armour_folder`,
`nsb_armour_folder`, `artillery_folder`, `naval_folder`, `mtgnavalfolder`, `mtgnavalsupportfolder`,
`air_techs_folder`, `bba_air_techs_folder`, `industry_folder`, `electronics_folder`), each a
`drag_scroll` scroll canvas hosting many `gridboxtype` **"…_tree"** node hosts. The actual tech NODES
are **not** declarative elements — they are instantiated by the tech adapter from the catalog into
those grids, each using one of the **node-prototype** windows
(`techtree_<cat>_folder_item` / `…_small_item`). Plus a tech **info/detail window** pair
(`technology_info_window`, `unlock_tech_info_window`) whose stat rows are generated from
`technology_*_statlist_item_*` row templates. Selected/observed country = `<TAG>`.

Formatters: `int`, `percent`, `loc` (token→localised string), `tech-icon` (the per-tech sprite,
adapter/tech-DB lookup), `tech-state-bg` (completed/available/unavailable/in-progress → background
frame), `equipment-role-icon` (frame of `GFX_land_equipment_role_icons`), `research-date`
(Clausewitz→year), `progressbar-fraction` (0..1), `subtech-frame`.

## 1a. Tech tree — STRUCTURAL FOLDER WINDOWS + per-folder chrome (mostly static; the grids are adapter hosts)

The 12 folder windows + the root are layout/scroll chrome. The **`gridboxtype` "…_tree" hosts inside
them are the adapter's NODE containers** (the tree adapter fills each with node-prototype instances —
node binding is §1d). The folders also carry static year-axis labels and category subtitles.

| gui element (name + type) | role | binding |
|---|---|---|
| `countrytechtreeview` (containerWindowType, root) + its `Background` (`GFX_tiled_plain_bg2`) | view root + backdrop | **static** |
| 12 × `<cat>_folder` (containerWindowType) each with `margin`, `drag_scroll`, `Background` (`GFX_tiled_window_2b_border`) | the per-category scroll canvases (one visible at a time via the folder tabs) | **static frame** (the `drag_scroll` = the §B.5 scroll widget; camera owns pan/zoom in replay) |
| `techtree_stripes` / `tiled_research_bg` / `tiled_research_bg_2` / `armour_subtitles` / `nsb_armour_subtitles` / `size_filler` / `tiled_window_transparent_transparent` / `wonderweapon_year` / `engineering_year_right` / `airtech_year_left` (containerWindowType) | cosmetic tiled backgrounds / sub-title bands inside folders | **static decoration** |
| ~**N** × `gridboxtype` named `*_tree` (e.g. `infantry_weapons_tree`, `early_destroyer_tree`, `gwtank_tree`, `atomic_research_tree`, `Juggle_1..6_tree`, …) — **123 `gridboxtype` + 1 `gridboxType` total in the file** | **ADAPTER NODE HOSTS** — each is a positioned grid the tech adapter fills with node-prototype instances for that sub-branch | **adapter-fed** (the grid itself is static geometry; its CONTENTS = §1d node binding). The `Juggle_*_tree` grids are layout spacers the adapter also targets. |
| year-axis labels: `naval_year1..4`, `airtech_year2..7` (instantTextBoxType, literal `text` = `1922`/`1933`/… ) | static decade ruler on naval/air folders | **static** (literal years baked in the `.gui`) |
| category subtitles: `naval_subtitle_*`, `airtech_subtitle_*`, `industry_subtitle*`, `electronic_eng_subtitle`, `fortification_eng_subtitle`, `experimental_rockets_subtitle`, `atomic_research_subtitle` (instantTextBoxType, loc keys `NAVAL_TITLE_*` / `AIR_TITLE_*` / `INDUSTRY_TITLE*` / `TITLE_*_SUBTITLE`) | static branch headers | **static loc constants** |
| highlight glows: `highlight_infantry_1`, `highlight_industry_1/2`, `highlight_engineering_1` (iconType, `GFX_tutorial_research_small_item_icon_glow`) | tutorial highlight glows | **static/UI** (tutorial state, no datum) |
| `naval_techtree_bg` / `mtgnavaltechtree_bg` (iconType, `GFX_naval_techtree_bg`) | naval folder backdrop art | **static** |

## 1b. Tech tree — FOLDER TABS + nav (static, read-only verbs)

- **`folder_tabs` (containerWindowType)** holding **14 × `buttonType` folder tabs** (`infantry_folder_tab`,
  `support_folder_tab`, `armour_folder_tab`, `nsb_armour_folder_tab`, `artillery_folder_tab`,
  `naval_folder_tab`, `mtgnavalfolder_tab`, `mtgnavalsupportfolder_tab`, `air_techs_folder_tab`,
  `bba_air_techs_folder_tab`, `electronics_folder_tab`, `industry_folder_tab`, each with its
  `GFX_*_folder_tab` sprite) + the 3 tab-highlight glows (`highlight_infantry_folder`,
  `highlight_engineering_folder`, `highlight_industry_folder`, `GFX_tutorial_research_folder_icon_glow`).
  These switch which folder is visible — **read-only nav verbs**, no country datum. The active tab
  could be `derived` from "which folder the user is viewing" (replay-local UI state), not a snapshot
  field. **static/UI.**
- **`close_button` (buttonType, `GFX_closebutton`)** — close verb, no datum. **static.**

## 1c. Tech tree — INFO / DETAIL WINDOW + generated stat rows

`technology_info_window` (clickable tech's panel) and `unlock_tech_info_window` (an unlock/effect
variant) bind the **selected/hovered** tech. Most of their content is **tech-DB + loc** (static,
adapter-fed) or **runtime-computed effect math** (unavailable) — *not* country snapshot state.

Current native status: `TechTreeView` / `DoctrineTreeView` selected nodes emit a first factual
read-only selected-detail panel/digest subset (title/ID/folder/state plus research cost or XOR-link
count). That proves selected-node detail output exists, but it is **not** full
`technology_info_window` / `unlock_tech_info_window` parity: the real game chrome, icon/stat grids,
generated effect rows, progress/ETA, MIO/design-team affordances, XP unlock/research affordances,
comparison styling, and side-by-side visual proof remain open.

| gui element (name + type) | snapshot source / origin | formatter | fidelity | notes |
|---|---|---|---|---|
| `technology_info_window` / `unlock_tech_info_window` (containerWindowType, roots) + their `Background` (`GFX_tiled_window_thin_border2`), `tech_info_top_win` (`GFX_tech_info_top_win`) | panel chrome | — | **static** | Frames. |
| `tech_info_title` (instantTextboxType) | selected tech id → loc | loc | **static / adapter-fed** | The tech's display NAME is a tech-DB + localisation lookup (the adapter supplies the selected id). *Not* a country field → not `unavailable`; it renders the loc string once the adapter loads the tech-DB. **[Codex correction]** |
| `tech_info_description` (instantTextboxType) | selected tech → loc description | loc | **static / adapter-fed** | Tech-DB description text. Same as title. |
| `tech_icon` (iconType, `GFX_tiled_window_transparent`) | selected tech icon (tech-DB) | tech-icon | **static / adapter-fed** | The tech's sprite, tech-DB lookup. |
| `research_progressbar` (iconType, `GFX_prod_progress_bar3`) + `research_progressbar_frame` (`GFX_production_progressbar_frame2`) | selected tech's research progress fraction | progressbar-fraction | **unavailable** | No stored fraction: `points_factor` is a speed multiplier, `in_progress_live=None`, `progress_pct` is `M*`. Frame renders inert / badged. |
| `researchtime_label` (instantTextboxType) | selected tech remaining-days ETA | int (days) / loc | **unavailable** | ETA = (cost − accumulated)/daily-speed; neither accumulated progress nor an ETA is stored. Badge unavailable. |
| `tech_info_special_description` (instantTextboxType, in `statsarea`) | tech special-effect text (tech-DB) | loc | **static / adapter-fed** | Tech-DB special-effect blurb. |
| `research` / `research_with_xp` / `unlock_with_xp` (buttonType, `GFX_button_*`, text `RESEARCH`/`RESEARCH_WITH_XP`) | — (start-research **mutations**) | — | **static / inert** | Start-research buttons are MUTATIONS → inert in a read-only replay (PLAN §D). No datum; render disabled. |
| `research_MIO` (containerWindowType) → `research_has_MIO` (buttonType, `GFX_government_button`, `RESEARCH`) + `MIO_icon` (iconType, `GFX_design_team_icon`) | the design-team / MIO assignment affordance for this tech | loc + mio-icon | **unavailable** | The MIO/design-team bonus join needs the `mios` domain (`countries.<TAG>.mios[]`, §3.10) — **not folded in this snapshot** (`designs`-sourced) — AND it is a mutation affordance. Badge unavailable; **not derivable from the `research` domain alone** (Codex). |
| `close` (buttonType, `GFX_closebutton`, tip `CLOSE`) | — | — | **static** | Close verb. |
| **generated stat rows** — the row templates `technology_unit_statlist_item_{equipment,module,upgrade,modifier}`, `technology_unlocks_statlist_item_{building,tactic,subunit}`, and their leaf templates `technology_unit_stat_item` (`name`/`amount`/`diff`), `technology_unit_upgrade_stat_item`, `technology_subunit_unlock_stat_item`, `technology_tech_stat_item`, `technology_unlocks_item`, `technology_tactics_item`, `technology_subunit_adjuster_entry` (terrain-mod strips), `technology_info_category_header` (`category:` header) | the tech's effect breakdown (stat name + amount + delta) | int/percent + loc | **unavailable / derived** | These rows enumerate what a tech UNLOCKS or MODIFIES (equipment stats, subunit stats, terrain modifiers, building unlocks, tactics). The **values + deltas are runtime-computed** from the tech-DB effect blocks and engine stat math — the save stores none of them (SCHEMA §11 note: derived combat stats are a hard limit). The *structure* (which unlocks exist) is tech-DB/static (`derived` once the adapter loads it); the *numbers* are `unavailable`. **[Codex: split static-structure vs runtime-numbers]** |
| `subunit_label`/`subunit_icon` (in `subunit_icon_entry`), `subunit_name`/`subunit_icon` (in `…_subunit`), `production_resources`, `equipment_icon` (`GFX_land_equipment_role_icons`), `carrier_fighter_icon` (`GFX_airwing_carrier_icon`), `unit_icon`, `design_team_icon` (`GFX_add_pol_idea_button`), the many `stats_grid`/`subunit_stats_grid`/`air_missions_grid`/`leader_bonuses_grid`/`support_category_multipliers_grid`/`terrain_modifiers_grid` (gridboxtype hosts) + their `*_header` labels | per-unlock icons + stat-grid hosts | icon/loc | **unavailable / derived** | Same as above: structure is tech-DB/static; the per-row stat numbers are runtime-computed and unstored. Grid hosts are adapter-filled. |
| `technology_air_mission_entry.mission_icon` (iconType, `GFX_strategicair_mission_types`) — *also present in doctrine view* | which air missions a tech/doctrine enables | category (mission→frame) | **derived** | Static tech/doctrine-DB attribute (which missions unlock); not a country field. |

## 1d. Tech tree — NODE BINDING (adapter-fed; nodes are NOT declarative `.gui` elements)

The tech NODES are instantiated by the **tech adapter** (PLAN §B.5) into the `*_tree` `gridboxtype`
hosts (§1a), one per tech, each cloning a **node prototype**. The prototypes are:
`techtree_infantry_folder_item` / `…_small_item`, `techtree_support_folder_item` / `…_small_item`,
`techtree_armour_folder_item` / `…_small_item`, `techtree_nsb_armour_folder_item` / `…_small_item`,
`techtree_artillery_folder_item` / `…_small_item`, `techtree_naval_folder_item` / `…_small_item`,
`techtree_mtgnavalfolder_item` / `…_small_item`, `techtree_mtgnavalsupportfolder_item` / `…_small_item`,
`techtree_air_techs_folder_item` / `…_small_item`, `techtree_bba_air_techs_folder_item` / `…_small_item`,
`techtree_industry_folder_item`, `techtree_secret_weapons_folder_item`, `techtree_electronics_folder_item`.
**They share one repeating child pattern** (the "small" variants drop `Name`; the `*_item` variants add
`Name` + 0–3 `sub_technology_slot_*`):

**Per-node prototype children (the repeating pattern):**

| node child (name + type) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `Background`/`bg` (background, `GFX_technology_unavailable_item_bg`) | per-tech STATE ← `technologies[]` ∋ name ? completed : (`in_progress[]` ∋ name ? researching : avail/unavail) | tech-state-bg | **exact** (completed / in-progress membership) / **derived-gappy** (available vs locked) | The node's frame encodes the state. Completed = membership in `technologies[]` (`exact`); in-progress = membership in `in_progress[]` (`exact`); available-vs-locked is **derived/gappy** — see "available/locked" row below. |
| `Icon` (iconType, `GFX_technology_medium`) | per-tech icon (tech-DB) | tech-icon | **static / adapter-fed** | The tech's sprite is a tech-DB lookup; STATE-tinting comes from the bg. |
| `Name` (instantTextBoxType; `*_item` only, placeholder `Happy-Go-Lucky-Tank`/`lol boat`) | tech id → loc | loc | **static / adapter-fed** | Display name, loc of the adapter-supplied id. |
| `bonus_icon` (iconType, `GFX_tech_bonus`) | does a `research_bonuses[]` entry cover this tech's category? | category | **derived** | The "you have a research bonus here" tile; bonuses themselves are `exact` in `research_bonuses[]`, but **node applicability is a derive** (category/tech join + consumption rules) (Codex). |
| `bonus` (instantTextBoxType, placeholder `lol boat`) | the applicable bonus value from `research_bonuses[].bonus` | percent | **derived** | The bonus % shown on the node; same join caveat. |
| `sub_technology_slot_0..2` (containerWindowType, each `Background` `GFX_subtechnology_unavailable_item_bg` + `picture` iconType `GFX_subtech_*`) | per-sub-tech STATE (sub-techs are themselves entries in `technologies[]`) | subtech-frame | **exact** (membership) / **static** (icon) | The small sub-tech pips on a parent node (e.g. armour sub-modules). Each pip's done/available state is a membership test in `technologies[]` (`exact`); the pip sprite is tech-DB (`static`). |
| `can_assign_design_team_icon` (iconType, `GFX_design_team_icon`) | can a MIO/design-team be assigned to this tech? | category | **unavailable** | Needs the `mios` domain (unfed, §3.10) + is an interaction affordance. Not derivable from `research` alone. |
| `special_project_specialization_list` (gridboxtype) → `special_project_specialization` (`technology_specialization_tag_primary/secondary` icons) + `special_project_tech_icon_offset` (positionType) | special-project specialization tags on a tech | category | **unavailable** | Special-project specialization tags come from `extra.special_projects` (§3.11, unfed here) / tech-DB; no per-tech specialization mapping folded. Badge unavailable. |

**Per-node DATA state (the actual node binding the brief asks for):**

| node state (per tech) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| **completed level** (researched / leveled) | `countries.<TAG>.research.technologies[]` → match `name`, read `level` | int (level) | **exact** | 122 completed rows in sample; `{name, level, date, research_points}`. `level≥1` ⇒ researched; `level` drives multi-level tech tiers (e.g. `infantry_weapons` levels). |
| **completion date** (node done timeline) | `technologies[].date` | research-date (Clausewitz→year) | **exact** | e.g. `"1936.1.1.12"`. Feeds the node's done-state + the `tech_researched` diff-event timeline. |
| **research points spent** (node tooltip) | `technologies[].research_points` | int | **exact** (where non-null) | e.g. `165.0`; **sometimes `null`** (e.g. `smoke_generator`) → renders "—". This is a point COST, **never** an ETA/year. |
| **in-progress** (node currently researching) | `countries.<TAG>.research.in_progress[]` → match `name` | category (researching frame) | **exact** (membership) / **unavailable** (fraction) | A tech in `in_progress[]` renders the "researching" frame. The PROGRESS FRACTION is `unavailable` (no stored numerator; `in_progress_live=None`; `progress_pct` `M*`). Note `in_progress[].points_factor` is a SPEED multiplier, not progress. |
| **in-progress (live)** (would carry accumulated points) | `countries.<TAG>.research.in_progress_live[].{tech_name,progress_units}` | int (points) | **unavailable** (current data) | The LIVE leaf carries accumulated research POINTS (`progress_units`, NOT a completion fraction — the cost denominator is runtime-computed). It is `None` in this save snapshot → `unavailable`; would be a renderable accumulated-points readout (still not a fraction) once a live read folds it. |
| **not-yet-researched** (available vs locked) | absence from both `technologies[]` and `in_progress[]`; available iff prereqs ⊆ completed set | category (available/locked frame) | **derived-gappy** | Absence ⇒ un-researched. The available-vs-locked **frame is only an approximation from prereqs ⊆ completed**: exact clickability ALSO needs allow/potential triggers, XOR state, ahead-of-time date penalties, XP/doctrine rules, country state, DLC/game-rules, and free slots — none modeled. Badge `derived` but **gappy** (Codex). |
| **research slots** (parallel-research cap) | `countries.<TAG>.research.research_slots` (+ `original_research_slots`) | int | **exact** | `5` (orig `4`). Bounds how many `in_progress[]` rows exist; drives the `research_slot_gained` diff event. |
| **research bonuses** (folder/category discount) | `countries.<TAG>.research.research_bonuses[]` (`{name, categories[], bonus, kind}`) | percent + category | **exact** (the bonus rows) / **derived** (per-node applicability) | 20 rows. The bonus VALUES are `exact`; mapping a bonus to the specific nodes it covers (the `bonus_icon`/`bonus` tile) is a category/tech join → `derived`. |
| **link lines** (prereq edges) | tech-DB prereq graph (static, `common/technologies/*.txt`) — rendered via `techtree_line_item` (Up/Down/Left/Right/Center icons) | — | **adapter-fed** | Edges come from the tech definitions, not the snapshot. The completed set decides which edges render "satisfied". Not a declarative bind (PLAN §B.5). |
| **XOR lines** (mutually-exclusive tech pairs) | tech-DB `XOR`/`mutually_exclusive` (static) — rendered via `techtree_xor_item` (`First`/`Second` icons) | — | **adapter-fed** | Mutual-exclusion routing is tech-DB data; not a snapshot field. |
| **node rect / position** | the per-tech `gridboxtype` `*_tree` hosts + per-prototype offsets in this `.gui` (static) | — | **adapter-fed** / **approximate where disambiguated** | Positions are static, embedded in the tech-tree `.gui` folders + the catalog. Not a snapshot field. The current native graph no longer feeds raw mixed coordinates directly: folder-grid techs are laid out in a deterministic folder atlas, sub-techs use their parent plus the parser's canonical slot offset, runtime-positioned techs are parked/badged, and duplicate folder slots are disambiguated/badged so common nodes remain independently pickable. Exact per-folder host extraction and side-by-side geometry proof remain open. |

---

# VIEW 2 — DOCTRINE TREES (`game_assets/gui/doctrine_tree.json`, `countrydoctrinetreeview.gui`)

**This is the land / naval / air / special-forces doctrine view — and it is the SAME tree engine as
View 1.** Doctrines ARE technologies (same `research` domain, same `countrytechtreeview` rendering
machinery). The `.gui` here is small (78 elements, 3 windows): the root `countrydoctrineview` holds
the 4 doctrine FOLDERS, the folder tabs, and a close button. **Critically, the `land_doctrine_folder`,
`naval_doctrine_folder`, and `air_doctrine_folder` are EMPTY containers** (no declared children) — the
doctrine NODES are filled entirely by the adapter (reusing the tech-tree node prototypes from View 1,
since doctrines render as techs). Only `special_forces_doctrine_folder*` carries declared background
label sub-windows, and its node prototype `techtree_special_forces_doctrine_folder_item` is the one
doctrine-specific prototype in THIS file. Selected/observed country = `<TAG>`.

Formatters: `loc`, `int`, `percent`, `tech-state-bg`, `tech-icon`, `category` (mission→frame).

## 2a. Doctrine tree — STRUCTURAL FOLDER WINDOWS (the land/naval/air folders are bare adapter hosts)

| gui element (name + type) | role | binding |
|---|---|---|
| `countrydoctrineview` (containerWindowType, root) + its `margin` | view root | **static** |
| `land_doctrine_folder` (containerWindowType, **0 children**) | the LAND doctrine canvas — **entirely adapter-filled** (no declared nodes) | **adapter-fed host** (geometry static; CONTENTS = §2c node binding) |
| `naval_doctrine_folder` (containerWindowType, **0 children**) | the NAVAL doctrine canvas — adapter-filled | **adapter-fed host** |
| `air_doctrine_folder` (containerWindowType, **0 children**) | the AIR doctrine canvas — adapter-filled | **adapter-fed host** |
| `special_forces_doctrine_folder_no_aat` (containerWindowType) + `margin`, `drag_scroll`, `Background` (`GFX_tiled_window_2b_border`) | special-forces folder (no-After-Action-Theatre variant) — Rangers branch only | **static frame + adapter host** |
| `special_forces_doctrine_folder` (containerWindowType) + `margin`, `drag_scroll`, `Background`, `size_filler` | the full special-forces folder (mountaineers + marines + paratroopers + rangers) | **static frame + adapter host** |
| `special_forces_<branch>_background_window` (containerWindowType ×4: `_rangers_`, `_mountaineers_`, `_marines_`, `_paratroopers_`) — each holds a `<branch>_bg` iconType (`GFX_rangers_bg`/`GFX_mountaineers_bg`/`GFX_marine_bg`/`GFX_paratroopers_bg`) + `<branch>_title` (instantTextBoxType, loc `*_DOCTRINE_LABEL`) + `<branch>_desc` (instantTextBoxType, loc `*_DOCTRINE_DESC`) | the static branch art + heading + description band for each special-forces doctrine | **static loc constants + art** |
| `special_forces_<branch>_tree` (gridboxtype ×5: `_rangers_tree` ×2 [one per SF folder], `_mountaineers_tree`, `_marines_tree`, `_paratroopers_tree`) each + `slotsize` + `Background` (`GFX_fullborder_tiled`) | **ADAPTER NODE HOSTS** — the grids the adapter fills with special-forces doctrine node-prototype instances | **adapter-fed** (grid geometry static; contents = §2c) |

## 2b. Doctrine tree — FOLDER TABS + nav (static, read-only verbs)

- **`folder_tabs` (containerWindowType)** with **4 × `buttonType`** doctrine tabs
  (`land_doctrine_folder_tab` `GFX_landdoctrine_tab_large`, `naval_doctrine_folder_tab`
  `GFX_navaldoctrine_tab_large`, `air_doctrine_folder_tab` `GFX_airdoctrine_tab_large`,
  `special_forces_doctrine_folder_tab` `GFX_specialforcesdoctrine_tab_large`) + their static title
  labels (`army_doctrine_folder_title` `land_doctrine`, `navy_doctrine_folder_title` `naval_doctrine`,
  `air_doctrine_folder_title` `air_doctrine`, `special_forces_doctrine_folder_title` `special_forces`)
  + the highlight glow `highlight_land_doctrine_folder` (`GFX_tutorial_research_folder_icon_glow`).
  Read-only nav verbs; active tab = replay-local UI state. **static/UI.**
- **`close_button` (buttonType, `GFX_closebutton`, tip `CLOSE`)** — close verb. **static.**

## 2c. Doctrine tree — NODE BINDING (adapter-fed; nodes are NOT declarative `.gui` elements — and UNFED this channel)

Doctrine NODES are instantiated by the **tech adapter** into the doctrine-folder grids (§2a). The
land/naval/air folders reuse the **View-1 tech node prototypes** (doctrines are techs); the
special-forces folder uses `techtree_special_forces_doctrine_folder_item` (the lone doctrine-specific
prototype here — children: `Background` `GFX_technology_unavailable_item_bg`, `Icon`
`GFX_technology_medium`, `bonus_icon` `GFX_tech_bonus`, `bonus` instantTextBoxType — a stripped
folder-item with no `Name`/sub-slots). The node DATA binds onto the SAME `research` fields as View-1
techs — **but with one decisive caveat for this snapshot/channel:**

> **DOCTRINE TECHS ARE ABSENT FROM THE SNAPSHOT.** Empirically (this sample, all 143 countries): the
> 187 distinct tech names in `research.technologies[]` contain **zero** doctrine roots. The parser that
> fed this save channel does **not** extract doctrine technologies. Therefore every "completed
> level / date / in-progress" doctrine node is **`unavailable` (unfed) for this snapshot** — even
> though the binding TARGET (`research.technologies[]`) is generic and would carry doctrines
> **exact-when-present**. This is a parser/channel gap, **not** a schema gap, and **not** "not
> researched" — absence here means *unknown*, not *zero* (no-fabrication). Use the conditional badge
> **EXACT-when-present; UNAVAILABLE/UNFED for this snapshot/channel** (Codex-confirmed framing, §D.2).

| node state (per doctrine tech) | snapshot source path | formatter | fidelity | notes |
|---|---|---|---|---|
| `Background` STATE (completed / researching / available) | `research.technologies[]` / `in_progress[]` membership (as View 1) | tech-state-bg | **exact-when-present; UNAVAILABLE this channel** | The frame logic is identical to a tech node — but with zero doctrine rows fed, every doctrine node currently resolves to the un-researched frame *by absence*, which would be **fabrication** if rendered as "not researched". Badge `unavailable` until doctrines are parsed. |
| `Icon` (`GFX_technology_medium`) | doctrine icon (doctrine-DB) | tech-icon | **static / adapter-fed** | Doctrine sprite, DB lookup; independent of the data gap. |
| `bonus_icon` / `bonus` | `research_bonuses[]` category-join (doctrine categories, e.g. `SUBDOCTRINE_*` entries seen in `research_bonuses[]`) | category + percent | **derived** | `research_bonuses[]` DOES carry sub-doctrine bonus rows (e.g. `SUBDOCTRINE_MOBILE_INFANTRY` observed) → the bonus tile is derivable even though the doctrine TECH rows are absent. |
| **completed level** | `research.technologies[]` → doctrine name → `level` | int | **exact-when-present; UNAVAILABLE this channel** | Doctrine progression is leveled (e.g. mobile-warfare tier). Schema-eligible; unfed. |
| **completion date** | `technologies[].date` for the doctrine | research-date | **exact-when-present; UNAVAILABLE this channel** | Unfed. |
| **in-progress** | `in_progress[]` ∋ doctrine name | category | **exact-when-present (membership); UNAVAILABLE this channel** | Unfed; fraction also unavailable (same numerator gap as techs). |
| **mutual exclusion** (the XOR between doctrine PATHS — the defining doctrine UI feature) | doctrine-DB `XOR`/`path` (static) — rendered via the View-1 `techtree_xor_item` prototype the adapter reuses | — | **adapter-fed** | The "pick one doctrine path" exclusivity is doctrine-DB data, not a country field. The CHOSEN side is implied by which doctrine techs are in `technologies[]` (→ also `unavailable` here, since none are fed). |
| **link lines** (doctrine prereq edges) | doctrine-DB prereq graph (static) — View-1 `techtree_line_item` | — | **adapter-fed** | Edges from the doctrine DB. |
| **node rect / position** | the doctrine-folder grids + catalog offsets (static) | — | **adapter-fed** | Positions static; the land/naval/air folders are empty → 100% adapter-placed. |
| `technology_air_mission_entry.mission_icon` (`GFX_strategicair_mission_types`) | which air missions an air-doctrine enables (doctrine-DB) | category | **derived** | Static doctrine-DB attribute. |

---

## Notes

**A. Both views are ONE engine.** `countrydoctrineview.gui` is a thin shell whose land/naval/air
folders are *empty* and whose nodes are the same tech-node prototypes from `countrytechtreeview.gui`
(doctrines are technologies). The tech adapter (PLAN §B.5) drives both: parse the catalog →
`{techId → rect}` per folder, clone a node prototype per tech/doctrine, draw `techtree_line_item` /
`techtree_xor_item` edges from the prereq/XOR graph. Geometry + edges are **adapter-fed** in both;
only the per-node DATA binds to the snapshot.

**B. The single most important fidelity asymmetry: techs vs doctrines.** TECH nodes bind **exact**
(122 completed rows present, `{name, level, date}`). DOCTRINE nodes bind to the *same* fields but are
**`unavailable`/unfed** in this snapshot because the parser feeds **zero** doctrine techs (verified
across all 143 countries — 187 distinct names, none a doctrine root). This is the load-bearing
correctness point of this map: do not render an absent doctrine as "not researched" (that is
fabrication); render "—" and badge unavailable. The fix is a parser change (extract doctrine
technologies into `research.technologies[]`), after which doctrine nodes become **exact** with no
schema change.

**C. Progress fraction is unavailable for BOTH (same cause as the research map).** A tech/doctrine is
COMPLETED via membership in `technologies[]` (`exact`) and IN-PROGRESS via `in_progress[]` (`exact`).
The PROGRESS FRACTION is `unavailable`: `in_progress[].points_factor` is a SPEED multiplier (not
progress), `in_progress_live.progress_units` is accumulated POINTS (not a fraction — denominator is
runtime-computed) and is `None` in this save snapshot, and `progress_pct` is `M*`/null. So every
research-progress bar (`research_progressbar`) and ETA (`researchtime_label`) is `unavailable`.

**D. Detail-window stat math is runtime-computed (a documented hard limit).** The tech info window's
effect rows (equipment/subunit stats + deltas, terrain modifiers, doctrine bonus math) are computed by
the engine from the tech-DB effect blocks — the save stores none of them (SCHEMA §11: derived combat
stats are a hard limit). The *structure* (which unlocks/effects a tech has) is tech-DB static
(`derived` once the adapter loads it); the *numbers* are `unavailable`. Tech NAME/description/icon are
`static`/adapter-fed (tech-DB + loc), **not** `unavailable` (Codex correction).

**E. `unavailable` ≠ "missing forever".** Most `unavailable` badges are (i) the doctrine data gap
(parser change), (ii) schema-present fields the current snapshot generator does not yet feed
(`in_progress_live`, the `mios`/`extra.special_projects`/`scalars` domains), or (iii) runtime-computed
tech effect math that the save fundamentally never stores. All render "—" per no-fabrication.

---

## Element accounting (provably complete)

### TECH TREE (`tech_tree.json`) — type counts (walk == `.gui` stats, both **1055**): positionType 4, containerWindowType 106, iconType 137, instantTextBoxType 210, borderSize 216, background 68, margin 14, drag_scroll 12, gridboxtype 123, gridboxType 1, slotsize 101, buttonType 21, instantTextboxType 42

| bucket | count | examples |
|---|---:|---|
| **Adapter NODE HOSTS** (the `*_tree` grids the adapter fills) | **124** (`gridboxtype` 123 + `gridboxType` 1) | `infantry_weapons_tree`, `early_destroyer_tree`, `gwtank_tree`, `atomic_research_tree`, `Juggle_1..6_tree`, … |
| **Node-prototype windows** (13 `*_folder_item`/`_small_item` roots + their repeating children: `Background`/`bg`, `Icon`, `Name`, `bonus_icon`, `bonus`, `sub_technology_slot_*`, `can_assign_design_team_icon`, `special_project_specialization_list`) | **27 prototype windows** (instanced per-tech by the adapter) | `techtree_infantry_folder_item`, `techtree_naval_folder_small_item`, … |
| **Graph-edge prototypes (adapter-fed geometry, not declarative)** | **2** | `techtree_line_item` (Up/Down/Left/Right/Center), `techtree_xor_item` (First/Second) |
| **Info / detail window + generated stat rows** (2 window roots + 15 statlist/leaf row templates) | **17 window/template roots** | `technology_info_window`, `unlock_tech_info_window`, `technology_unit_statlist_item_equipment`, `technology_unit_stat_item`, `technology_subunit_adjuster_entry`, `technology_info_category_header`, … |
| **Dynamic-bound** (within the above; incl. derived/unavailable) | **~14 distinct binds** | node bg-state, Icon, Name, bonus_icon, bonus, sub_technology_slot, can_assign_design_team_icon; info-window title/desc/icon, research_progressbar, researchtime_label, MIO icon, mission_icon |
| **Static / decoration / nav** | **~860** | root + 12 folder windows + their margins/drag_scrolls/backgrounds, 14 folder tabs + 3 glows + close, year-axis labels, category subtitles, all `borderSize`/`slotsize`/`margin` layout nodes, naval/air backdrop art, info-window frames + RESEARCH buttons (inert) + the static-structure stat labels |

### DOCTRINE TREES (`doctrine_tree.json`) — type counts (walk == `.gui` stats, both **78**): containerWindowType 15, margin 3, drag_scroll 2, background 8, iconType 9, instantTextBoxType 15, borderSize 11, gridboxtype 5, slotsize 5, buttonType 5

| bucket | count | examples |
|---|---:|---|
| **Adapter NODE HOSTS** | **5** (`gridboxtype`) | `special_forces_rangers_tree` (×2), `_mountaineers_tree`, `_marines_tree`, `_paratroopers_tree`; **+ the 3 EMPTY land/naval/air folders are 100%-adapter hosts** |
| **Empty adapter folders (land/naval/air — bare containers, 0 declared children)** | **3** | `land_doctrine_folder`, `naval_doctrine_folder`, `air_doctrine_folder` |
| **Node-prototype window** (doctrine-specific) | **1** | `techtree_special_forces_doctrine_folder_item` (+ children Background/Icon/bonus_icon/bonus) — land/naval/air reuse View-1 prototypes |
| **Air-mission entry prototype** | **1** | `technology_air_mission_entry.mission_icon` |
| **Dynamic-bound** (incl. derived/unavailable) | **~6 distinct binds** | node bg-state (unfed), Icon, bonus_icon, bonus, mission_icon; (completed level/date/in-progress all UNFED this channel) |
| **Static / decoration / nav** | **~62** | root + margins, the 2 SF folder frames + backgrounds, 4 special-forces branch label/desc/art windows, 4 folder tabs + 4 titles + glow + close, all `borderSize`/`slotsize` layout nodes, `GFX_fullborder_tiled` grid backgrounds |

Every element in each of the two `.gui` trees falls into exactly one bucket above; the per-element
tables (1a/1b/1c/1d, 2a/2b/2c) enumerate them by name or by repeating-pattern with counts.

### `unavailable`-badged binds (with reason)

**Tech tree (7 distinct):** `research_progressbar` + node in-progress FRACTION (no stored numerator;
`in_progress_live=None`, `progress_pct` `M*`), `researchtime_label` (ETA not stored), `research_MIO` /
`MIO_icon` / `can_assign_design_team_icon` (mios domain unfed + mutation affordance),
`special_project_specialization*` (special-projects/tech-DB specialization unfed), the detail-window
stat NUMBERS (runtime-computed, unstored), `in_progress_live.progress_units` (live leaf None this
channel).

**Doctrine tree (the whole node-DATA layer):** every doctrine **completed level / date / in-progress**
node bind — `unavailable`/UNFED because the parser extracts **zero** doctrine techs into
`research.technologies[]` (conditional badge: EXACT-when-present). Plus the same progress-fraction /
ETA / MIO gaps inherited from the shared tech engine.

All are schema-present-but-unfed fields, separate-domain data not folded into THIS snapshot, or
tech/doctrine-DEF static data / runtime-computed effect math — none fabricated; each renders "—" per
no-fabrication.

---

## Schema-gap ledger (needed-but-absent or unfed fields)

| # | needed for | field / data | status | classification |
|---|---|---|---|---|
| 1 | **doctrine node completed-level / date / in-progress** | `research.technologies[]` / `in_progress[]` rows **for doctrine techs** | **schema-present leaf, PARSER-UNFED** — the save-channel parser extracts zero doctrine technologies (verified: 0 of 187 world tech names are doctrines) | **parser gap** (not schema) — fix = extract doctrine techs; then `exact` with no schema change |
| 2 | research-progress BAR fraction (node + detail window) | a stored progress fraction / numerator for an in-progress tech | `in_progress[].progress_pct` is `M*` (null in save); `in_progress_live.progress_units` is accumulated POINTS (not a fraction) and `None` this channel | **schema-present, unfed** + **runtime denominator** (the cost-to-complete is engine-computed) |
| 3 | research ETA (`researchtime_label`) | remaining-days-to-complete for an active tech | not stored; needs accumulated-progress + daily-speed | **runtime-computed** (never in the save) |
| 4 | per-tech / per-doctrine effect & combat-stat rows (detail window stat grids) | soft/hard attack, org, HP, width, terrain modifiers, doctrine bonus math, per-row deltas | not stored — SCHEMA §11: derived combat stats are a documented hard limit | **runtime-computed** (structure is tech-DB static; numbers unavailable) |
| 5 | per-tech XP cost / "research with XP" / "unlock with XP" | the army/navy/air XP price of a doctrine/tech step | not a `research` field; tech-DB constant | **tech-DEF static** (adapter-supplied, not country state) |
| 6 | available-vs-locked node frame (exact clickability) | allow/potential triggers, XOR state, ahead-of-time date penalty, free-slot count, DLC/game-rule gates | only prereqs ⊆ completed is testable; the rest is unmodeled | **derived-gappy** (prereq test exact; full gating needs the trigger evaluator + tech-DB) |
| 7 | node MIO / design-team assignment (`research_MIO`, `can_assign_design_team_icon`) | which MIO/design-team bonus applies to a tech | `countries.<TAG>.mios[]` (§3.10) not folded in this snapshot | **separate-domain, unfed** (designs/mios channel) |
| 8 | doctrine mutual-exclusion DISPLAY (which path is chosen / which XOR side is locked) | doctrine-DB `XOR`/`path` graph + the chosen side | XOR graph is doctrine-DB static (adapter); the chosen side is implied by `technologies[]` membership → also unfed (#1) | **tech-DEF static** (graph) + **parser-unfed** (chosen side) |
| 9 | special-project specialization tags on a tech | per-tech specialization mapping | `extra.special_projects` (§3.11) not folded; tech-DB mapping | **separate-domain, unfed** + tech-DEF static |
| 10 | net research-speed readout (not in these `.gui`s but adjacent) | `scalars.research_speed.{pct,multiplier,factor_total}` | `D` (derived) and not in this snapshot | **schema-present, unfed** (scalars model) |

---

## Historical review

Historical note: this records a completed adversarial review from the old
Claude-era workflow. Codex-native sessions must use normal sub-agents/partner
review and must not run the old local CLI command. The review checked the local `.gui` files. Verdict:
**"Your core mappings are correct."** All node→field mappings were confirmed; four badge refinements
were folded in.

| claim asked | Codex verdict | folded-in correction |
|---|---|---|
| **(a) missed distinct elements/node-types** | flagged 5 groups to classify explicitly | folded: (1) all large/small `techtree_*_folder_item` variants + their `Name`/`Icon`/`bonus`/`sub_technology_slot_*`/`special_project_specialization_list`/`can_assign_design_team_icon` children (§1d); (2) `techtree_line_item` + `techtree_xor_item` graph decorations (§1d); (3) the detail-window generated stat-list rows (§1c); (4) doctrine chrome — empty land/naval/air folders, SF backgrounds, tabs, `techtree_special_forces_doctrine_folder_item`, `technology_air_mission_entry` (§2); (5) the MIO/design-team affordances `research_MIO`/`research_has_MIO`/`can_assign_design_team_icon` — "not derivable from the `research` domain alone" → badged `unavailable` (§1c/§1d). |
| **(b) node→field correctness vs schema** | **AGREE on all 8 core mappings** | none — confirmed: `technologies[].level/date` exact; `research_points` = COST not ETA; `in_progress[].points_factor` = speed not progress; `in_progress_live.progress_units` = accumulated points not a fraction; `progress_pct`/`in_progress_live=None` ⇒ fraction unavailable. |
| **(c) false unavailable vs real gap** | flagged 3 over-/mis-badges | folded: `tech_info_title`/`description` → **STATIC/ADAPTER-FED** (tech-DB + loc), NOT `unavailable`; detail-window unlock/effect rows → split **STATIC/DERIVED structure** vs **runtime-derived/unavailable numbers**; `bonus_icon`/`bonus` → bonuses `exact` in `research_bonuses[]` but node applicability `derived` (category/tech join). |
| **available-vs-locked badge** | tighten to **DERIVED/GAPPY** | folded (§1d "not-yet-researched"): prereqs ⊆ completed is only one factor — allow/potential triggers, XOR, date penalties, XP, DLC/rules, slots also gate exact clickability. |
| **(d) doctrine framing** | **AGREE** — "doctrines are technologies → bind to `research.technologies[]`, **exact when present**, but **unavailable/unfed** for this sampled save channel because the parser feeds zero doctrine techs" is the right framing; use a **conditional badge: EXACT when present; UNAVAILABLE/UNFED for this snapshot/channel**; do not interpret absence as "not researched" | folded throughout §2c + Notes B + ledger #1: conditional badge applied to every doctrine node-DATA bind; absence framed as *unknown* not *zero* (no-fabrication). |

Net: no tech node-binding badge was downgraded (techs remain `exact`); four chrome/detail badges were
tightened (tech name/desc → static-not-unavailable; effect rows → static-structure-vs-runtime-numbers;
bonus applicability → derived; available/locked → derived-gappy); and the doctrine layer's conditional
**EXACT-when-present / UNAVAILABLE-this-channel** badge — the highest-risk call — was confirmed.
