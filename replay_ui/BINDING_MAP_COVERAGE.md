# BINDING-MAP COVERAGE — the in-game `.gui` coverage contract + consolidated schema-gap ledger

> **What this is.** The master coverage proof for the native Battle Plan-UI binding sweep. It (1)
> enumerates EVERY in-game `.gui` view in the effective mod-over-vanilla set, (2) maps each view to
> the binding-map doc that covers it, (3) consolidates every schema gap the views surfaced — split
> **demo-snapshot artifact vs TRUE GAP** and, for true gaps, classified into the **three closure
> buckets** (live-extractable / computed-not-stored / static-def-or-world-data), and (4) gives the
> final coverage table + count. Per the grand goal (replicate literally everything the game shows),
> **no in-game view is optional, skippable, deferred, or out of scope** — the only excluded group is
> the pre-game frontend/lobby/main-menu shell (a recorded replay has no lobby state to bind) plus two
> non-replay dev tools.
>
> **The 13 binding-map docs this consolidates** (all under `replay_ui/`):
> `BINDING_MAP_topbar_state.md`, `BINDING_MAP_research_focus_menubar.md`,
> `BINDING_MAP_production_construction_decisions.md`, `BINDING_MAP_diplomacy_combat.md` (the 4
> pre-existing), plus the 9 authored in this sweep: `BINDING_MAP_chrome_outliner_maps.md`,
> `BINDING_MAP_politics_government.md`, `BINDING_MAP_tech_doctrine_trees.md`,
> `BINDING_MAP_military_management.md`, `BINDING_MAP_designers.md`,
> `BINDING_MAP_leaders_officers_traits.md`, `BINDING_MAP_intel_diplomacy_trade.md`,
> `BINDING_MAP_logistics_combat_units_events.md`, `BINDING_MAP_scripted_gui_overlays.md`.

---

## 0. Method & provenance

- **View universe = the EFFECTIVE mod-over-vanilla `interface/*.gui` set.** The mod (`AMSB - DEV`,
  per `game_assets/active_modset.json`) does **not** `replace_path` `interface`, so the effective set
  = vanilla `interface/*.gui` ∪ mod `interface/*.gui` (file-level override on same stem). Vanilla
  ships **130** `.gui`; the mod adds in-game overlays (propaganda boards, host tool, VP/resource
  topbars, naval scripted GUI, focus background, a dev check). Resolution rule replicated by
  `game_assets/extract.py` (`build_file_map_layers`).
- **Parsed core set** = `game_assets/gui/_index.json` = **77 view JSONs** (the `PRIORITY_GUI` map in
  `extract.py`). One, `destroyed_ships.json`, parses to **0 windows** (a present-but-empty stub —
  documented, not dropped).
- **Schema** = `replay/SCHEMA.md` + `replay/schema.py` (`DOMAINS`, `FIELD_SOURCES`). Source paths in
  every doc use the dotted SCHEMA form (`countries.<TAG>.…`, `states.<id>.…`, `provinces.<pid>.…`,
  top-level `map.*` / `naval_air.*` / `supply.*` / `world.*` / `diplomacy_world.*` /
  `order_of_battle.*` / `designs.*` / `strategic_air.*` / `world_events.*`).
- **Demo snapshot** the children empirically inspected =
  `replay/snapshots/replay/000017005619.snapshot.json` (GER 1941, **143 countries**). **It is
  SAVE-ONLY** — several schema-present LIVE/save domains are unfed in it (the single most important
  calibration below). A field unavailable *only because this demo snapshot did not fold its domain* is
  a **demo-snapshot artifact (FOLD-IN/RECONCILE)**, not a true schema gap.
- Historical adversarial review covered each doc and this consolidation; see §5.
  Codex-native sessions must use normal sub-agent/partner review, not the old CLI workflow.

### 0.1 The demo-snapshot artifact calibration (critical — do not inflate the gap count)

Empirically verified against the demo snapshot, these schema-present domains are **entirely unfed**
in it (every country's country object carries only
`{decisions,diplomacy,economy,focus,intel,military,politics,research}`):

| domain (schema §) | unfed in demo? | consequence |
|---|---|---|
| `commanders` (§3.9) | **yes** (None on all 143) | leaders/generals/admirals/advisors, command-character grid, unit-leader detail, trait-tree node state → badged **FOLD-IN**, NOT true gap |
| `extra` (§3.11) | **yes** | aces, nukes, dynamic-modifiers, cosmetic-tag, special-projects → **FOLD-IN** |
| `order_of_battle` (§12) | **yes** | army/navy/air formations, theatres, leader assignment join → **FOLD-IN** |
| `intel.agency/operations/decryption/recruitable/peace_conference` | **yes** (operatives[] partially fed) | intel agency, operation overview, decryption → **FOLD-IN** |
| top-level `world` / `world_events` / `strategic_operatives` / `raids` / `strategic_air` | **yes** (absent keys) | world-tension contributor ledger, scripted vars/flags, fired-events, sunk-convoys, air-base table, intel-network graph → **FOLD-IN** |
| `economy.logistics` (§3.1) | **yes** (0/143) | logistics view supply-system-2 rows → **FOLD-IN** |
| `supply` / `naval_air` sidecars (§13) | **absent keys** | logistics/naval-region/map-mode supply+dominance → **FOLD-IN** (but ALSO carry a permanent PROXY badge by nature) |
| `divisions_full` save body (§6) | **yes** (memory `divisions[]` IS fed) | per-division dig-in/acclimatization/killed/max-supply → **FOLD-IN** |
| `designs` (§11) `equipment_variants`/`division_templates` | partially | designer module-slot layouts → mostly **FOLD-IN** |

**Populated & truly bindable in the demo:** `countries.*.{economy.factories/fuel/resources,
military.manpower/convoys/equipment_stockpile, politics.*, research.*, focus.*}`, top-level
`divisions[]` (1654, memory channel: owner/template/org/strength/equipment/location),
`states.*.{owner,controller,buildings,province_buildings,victory_points_total,manpower_pool,
occupation(resistance/compliance)}`, `diplomacy_world.{wars,factions,subjects}`,
`countries.*.diplomacy.{opinion,wargoals}`.

A full LIVE capture (the project's live-first mandate) folds the artifact domains → those badges flip
to `exact`/`derived`. The TRUE-GAP ledger (§4) is what remains after removing artifacts.

---

## 1. Full in-game view inventory (the coverage contract)

### 1a. Parsed CORE in-game views (77) → owning binding-map doc

| # | view (game_assets/gui/) | source `.gui` | binding-map doc |
|--:|---|---|---|
| 1 | topbar | topbar.gui | topbar_state |
| 2 | province_state | countrystateview.gui | topbar_state |
| 3 | research | countrytechnologyview.gui | research_focus_menubar |
| 4 | national_focus | nationalfocusview.gui | research_focus_menubar |
| 5 | menubar | menubar.gui | research_focus_menubar |
| 6 | production | countryproductionlineview.gui | production_construction_decisions |
| 7 | construction | countryconstructionsview.gui | production_construction_decisions |
| 8 | decisions | countrydecisionview.gui | production_construction_decisions |
| 9 | diplomacy | countrydiplomacyview.gui | diplomacy_combat |
| 10 | land_combat | landcombat.gui | diplomacy_combat |
| 11 | naval_combat | navalcombat.gui | diplomacy_combat |
| 12 | combat_log | combatlogview.gui | diplomacy_combat |
| 13 | ingame_clock | ingameclock.gui | chrome_outliner_maps |
| 14 | minimap | minimap.gui | chrome_outliner_maps |
| 15 | alerts | alerts.gui | chrome_outliner_maps |
| 16 | find_view | findview.gui | chrome_outliner_maps |
| 17 | rightclick_menu | rightclickmenu.gui | chrome_outliner_maps |
| 18 | mapmodes | mapmodes_interface.gui | chrome_outliner_maps |
| 19 | mapicons | mapicons.gui | chrome_outliner_maps |
| 20 | politics | countrypoliticsview.gui | politics_government |
| 21 | power_balance | powerbalanceview.gui | politics_government |
| 22 | command_character | command_character_view.gui | politics_government |
| 23 | advisor_role_selection | advisorroleselectionview.gui | politics_government |
| 24 | occupation | countryoccupationview.gui | politics_government |
| 25 | tech_tree | countrytechtreeview.gui | tech_doctrine_trees |
| 26 | doctrine_tree | countrydoctrinetreeview.gui | tech_doctrine_trees |
| 27 | army | countryarmyview.gui | military_management |
| 28 | navies | naviesview.gui | military_management |
| 29 | air_group | airgroupview.gui | military_management |
| 30 | air_selection | airselectionview.gui | military_management |
| 31 | air_wing_details | airwingdetails.gui | military_management |
| 32 | air_wing_reorg | airwingreorganization.gui | military_management |
| 33 | carrier_air_wing | carrierairwingcompositionwindow.gui | military_management |
| 34 | strategic_air | strategicairview.gui | military_management |
| 35 | battle_plan_tools | battleplantools.gui | military_management |
| 36 | ability_list | abilitylist.gui | military_management |
| 37 | theatre_selector | theatreselector.gui | military_management |
| 38 | divisions_summary | divisions_summary.gui | military_management |
| 39 | railway_gun | railway_gun.gui | military_management |
| 40 | deployment | countrydeploymentview.gui | military_management |
| 41 | equipment_overview | equipmentoverview.gui | military_management |
| 42 | tank_designer | tank_designer_view.gui | designers |
| 43 | plane_designer | plane_designer_view.gui | designers |
| 44 | division_designer | divisiondesignerview.gui | designers |
| 45 | equipment_designer | equipmentdesignerview.gui | designers |
| 46 | equipment_upgrade_designer | equipmentupgradedesignerwindow.gui | designers |
| 47 | officer_corps | countryofficercorpview.gui | leaders_officers_traits |
| 48 | unit_leader | unitleaderwindow.gui | leaders_officers_traits |
| 49 | navy_leader | navyleaderview.gui | leaders_officers_traits |
| 50 | leader_groups | leadergroups.gui | leaders_officers_traits |
| 51 | aces | acesview.gui | leaders_officers_traits |
| 52 | army_trait_tree | armytraittreewindow.gui | leaders_officers_traits |
| 53 | navy_trait_tree | navytraittreewindow.gui | leaders_officers_traits |
| 54 | trait_tree_common | traittreewindow_common.gui | leaders_officers_traits |
| 55 | intel_agency | countryintelligenceagencyview.gui | intel_diplomacy_trade |
| 56 | intel_ledger | countryintelledger.gui | intel_diplomacy_trade |
| 57 | operative | operative.gui | intel_diplomacy_trade |
| 58 | operative_leader | operativeleader.gui | intel_diplomacy_trade |
| 59 | operation_overview | operationoverview.gui | intel_diplomacy_trade |
| 60 | trade | countrytradeview.gui | intel_diplomacy_trade |
| 61 | peace_conference | peaceconferencewindow.gui | intel_diplomacy_trade |
| 62 | world_tension_popup | worldtensionwindowpopup.gui | intel_diplomacy_trade |
| 63 | war_overview | waroverview.gui | intel_diplomacy_trade |
| 64 | insignia_selection | insigniaselection.gui | intel_diplomacy_trade |
| 65 | ins_revolution | insrevolutionwindow.gui | intel_diplomacy_trade |
| 66 | logistics | countrylogisticsview.gui | logistics_combat_units_events |
| 67 | garrison_log | garrisonlogview.gui | logistics_combat_units_events |
| 68 | naval_combat_results | navalcombatresultswindow.gui | logistics_combat_units_events |
| 69 | destroyed_ships | destroyedshipswindow.gui | logistics_combat_units_events (present-but-empty stub) |
| 70 | naval_losses | naval_losses.gui | logistics_combat_units_events |
| 71 | naval_strategic_region | naval_strategic_region_view.gui | logistics_combat_units_events |
| 72 | unit_view | unitview.gui | logistics_combat_units_events |
| 73 | event_window | eventwindow.gui | logistics_combat_units_events |
| 74 | popup_window | popupwindow.gui | logistics_combat_units_events |
| 75 | detailed_windows | detailedwindows.gui | logistics_combat_units_events |
| 76 | core | core.gui | logistics_combat_units_events |
| 77 | ingame_menu | ingamemenu.gui | logistics_combat_units_events |

### 1b. Group-1 scripted_gui / mod overlays (38) → `BINDING_MAP_scripted_gui_overlays.md`

These are loaded **in-game** (not frontend) so they are part of the contract. Verdict per overlay:
**3 partially-bindable** (`topbarresources`→`economy.resources.available_derived.*`;
`topbarvp`+`vp_scripted_gui`→`world.variables["allies_score"/"axis_score"]` + `map.vp_by_state`),
**32 chrome-only** (`naval_scripted_gui`, the 8 propaganda boards, all 23 vanilla nation-systems —
backed by **country-scoped** script vars the schema's global-only `world.variables`/`world.flags`
does not carry), **1 static decoration** (`SWOA_focus_background`), **2 dev-tool/inert**
(`horsttool` host tool — mutation buttons inert, 4 occupation/owner readouts bind; `game_check` —
mod dev). Full list & per-overlay verdict in the doc.

### 1c. OUT OF SCOPE — frontend / lobby / main-menu shell + non-replay dev tools

Per the user decision (a recorded replay has no lobby/main-menu state to bind) — the **only**
excluded group. ~31 files: `frontendmainview`, `frontendgamesetupview`, `frontendmultiplayerview`,
`frontend_friends_view`, `frontendcreditview`, `ingamelobby`, `playerlobby`, `playeraiprefs`,
`matchmaking`, `matchmaking_chat`, `chat`, `hotjoin`, `pops_login`, `social_view`,
`subscription_message_view`, `achievements`, `credits`, `settings`, `savedgamerules`,
`messagesettings`, `msg_filters`, `tutorialscreen`, `load_screen`, `browser`, `backend`,
`musicplayer`, `music_station_base`, `music_station_classic`, plus the dev tools `ai_trace`, `nudge`,
and the mod's `frontendgamesetupview` override. (The lone in-game sliver of frontend data — the
player roster — surfaces in-game via `countries.*.human_name`, schema §3.12, bound on country
surfaces, not here.)

---

## 2. Coverage proof (count)

| bucket | count | status |
|---|--:|---|
| Parsed core in-game views (PRIORITY_GUI) | **77** | **100% mapped or gap-listed** (12 pre-existing + 65 this sweep; incl. `destroyed_ships` empty-stub documented) |
| Group-1 scripted_gui / mod overlays (in-game) | **38** | **100% accounted** (3 partial-bind + 32 chrome-only + 1 static + 2 dev-tool) |
| **Total in-game views accounted for** | **115** | **100%** |
| Out-of-scope frontend/lobby shell + 2 dev tools | ~31 | excluded by user decision (recorded replay has no lobby state) |

**Every in-game view is accounted for: mapped (binding rules written) or gap-listed (its missing
field named as a capture work-item). Count of in-game views accounted for = 115 (77 core + 38
overlays). Zero in-game views silently dropped.**

> Count reconciliation: this **115** is the source-coverage universe
> (`.gui` core files + scripted/mod overlay coverage rows). It is not the native
> app registry count. The current native registry/product checklist count is
> **114 registered openable views**, because registry descriptors count
> menu/openable runtime surfaces rather than every source coverage row. Compare
> product parity against the 114-view registry and use this file to ensure no
> in-game source view was silently omitted from binding analysis.

> Reconciliation of the "13 already-parsed slice views" framing: the 4 pre-existing docs cover **12
> distinct parsed views** (the topbar doc binds two — topbar + province_state). The often-cited "13"
> counts the GPU political **map** (M4 slice milestone) as a 13th surface; the map is a render layer,
> not a `.gui` view, and its map-mode chrome is now in `chrome_outliner_maps`. Either way the parsed
> `.gui` math is exact: **77 parsed = 12 documented earlier + 65 documented now.**

---

## 3. Per-doc binding tallies (from each doc's own element-accounting)

| binding-map doc | views | total `.gui` elements accounted | bindable (dynamic) |
|---|--:|--:|--:|
| topbar_state | 2 | 230 (topbar 118 + state 112) | ~59 |
| research_focus_menubar | 3 | (per doc) | trees: node-binding + chrome |
| production_construction_decisions | 3 | (per doc) | array→grid binds |
| diplomacy_combat | 4 | (per doc) | grids + roster |
| chrome_outliner_maps | 7 | (per doc) | scalar + map-mode LUT binds |
| politics_government | 5 | 512 (263+21+42+24+162) | grids + conditional slots |
| tech_doctrine_trees | 2 | 1133 (tech 1055 + doctrine 78) | ~20 chrome + node-binding |
| military_management | 15 | 1654 | ~291 |
| designers | 5 | 1293 | ~237 |
| leaders_officers_traits | 8 | 984 | ~90 + 12 trait-node states |
| intel_diplomacy_trade | 11 | ~1044 | grids + operative/op rows |
| logistics_combat_units_events | 12 | ~1741 | ~214 |
| scripted_gui_overlays | 38 overlays | per-overlay verdict | 3 partial-bind |

Each doc carries its own provably-exhaustive per-view element-count table (walk == the `.gui`'s own
`stats.element_type_counts`), a dynamic/static/deferred bucketing, and a Codex round.

---

## 4. CONSOLIDATED schema-gap ledger (TRUE GAPs only — artifacts excluded)

> Demo-snapshot artifacts (§0.1) are **NOT** listed here — they bind once a full live snapshot folds
> the domain. Below is the **required-capture backlog**: fields no current schema domain provides,
> classified by closure bucket. The fix path differs per bucket, so each is tagged:
> **(a) LIVE-EXTRACTABLE** (in memory; a reader exists or is addable) · **(b) COMPUTED-NOT-STORED**
> (runtime-only; needs cracking / live recompute — Ghidra tier; often a documented hard limit) ·
> **(c) STATIC DEF / WORLD DATA** (parseable from the game's `common/` + def DB mod-aware, like the
> focus-tree parser — NOT memory at all).

### 4a. Bucket (a) — LIVE-EXTRACTABLE (record the field / add the reader)

| gap | needed by | note |
|---|---|---|
| `alerts[]` per-country active-alert ledger (type/severity/target) | alerts strip, topbar alert counters | recomputed live per player; needs a live alerts adapter or memory crack |
| real the game front-line geometry/styling coverage | front-lines map mode, theatre selector | native overlay has derived controller-boundary fallback plus exact `map.front_lines[]` geometry ingestion; real extraction coverage and side-by-side proof remain open |
| `states.<id>.resources` (per-state raw deposits; null 0/1081) | Resources map mode, state resource rows, topbarresources | `S`, parser-unfed; do NOT substitute country `economy.resources` |
| per-participant **war casualties** & **war score** split | war_overview, diplomacy war cards | `diplomacy_world.wars` carries aggregate; per-side split live-extractable |
| operative `code_name` / spymaster id | operative, intel_agency | not in `intel.operatives[]` shape; live/save addable |
| INS revolution **escalation level** (country-scoped var) | ins_revolution | per-country scripted var (see bucket-c note) |
| per-country **script-variable bag** (`script_variables`) | ~32 chrome-only scripted_gui overlays, war_escalation, usa_congress | the mass closure for nation scripted GUIs; `world.variables` is GLOBAL-only today |
| live per-admiral **fleet composition** for `naval_scripted_gui` / carrier wing | naval_scripted_gui, carrier_air_wing | needs live per-admiral fleet model (temp-var/script-function case) |
| live-ship **owner** / wing→**ace** join | navies, air wings | join addable from live OOB |

### 4b. Bucket (b) — COMPUTED-NOT-STORED (runtime-only; crack or live-recompute; many are hard limits)

| gap | needed by | note |
|---|---|---|
| **Exact per-province supply NUMBER** | logistics, supply map modes, unit supply | **proven wall** (`replay/SCHEMA.md §13.2`, `memory_reader/supply_solver_offsets.draft.json`, `RESEARCH_logistics_intel.md`); transient routing-solver output. `supply.by_province` is a PROXY. Hard limit. |
| **Aggregate world-tension scalar** | world_tension_popup, topbar threat | NOT stored — only `world_events.threat[]` contributor ledger + `world.world_tension_inputs`. Hard limit. |
| Designer **derived combat stats** (soft/hard attack, org, HP, defense, breakthrough, armor, piercing, speed, combat width) | all 5 designers, division/template stat panels | runtime-computed (SCHEMA §11 note, §G). Partial recovery only via the `template_stats`/`equipment_variant_stats` derived models (source **D**). Hard limit on the exact game value. |
| Combat **battle-detail** (progress bar, combat width, terrain, tactics, modifiers) | land/naval combat, combat results | §G hard limit; `combats[]` domain is M\* (uncracked) |
| **Weather** per-region temperature / phenomenon visuals | weather map mode, province panel weather icons | per-region temp reader retracted (`null`); per-province `weather.provinces.*` is M but proxy vs the game's exact visual |
| `naval_air.{air_superiority,region_control}` exactness | Air/Naval map modes, naval_strategic_region | region-proxy granularity by nature. Hard limit on per-province. |
| Occupation **resistance-equilibrium / garrison need / occupation yields** | occupation view, state panel garrison | runtime-computed; resistance/compliance VALUES are fed, the targets/needs are not |
| Power-balance **side thresholds / trend math** | power_balance | `world.power_balances[]` carries value/sides; threshold breakpoints computed |
| Operation **success-chance / outlook**, intel-ledger **estimate bands**, capitulation % | operation_overview, intel_ledger, war_overview | AI/score computes |
| Air-wing `effective_planes` efficiency denominators | strategic_air, air views | a captured RUNTIME-cached field (proxy), not the true efficiency model |
| Research **progress fraction / ETA** | research, tech/doctrine trees | `points_factor`=speed; the fraction denominator + ETA are runtime-computed |

### 4c. Bucket (c) — STATIC DEF / WORLD DATA (parse `common/` + def DB, mod-aware)

| gap | needed by | closure |
|---|---|---|
| **Event TITLE / DESC / PICTURE / OPTION text** (+ loc) | event_window, popup_window, alerts | parse `events/*.txt` + loc — the diff engine has fired-event IDs, not text |
| `common/scripted_guis/*.txt` element→variable map | all 38 scripted_gui overlays | the structural closure for binding scripted GUIs (which var feeds which element) |
| **Occupation-law catalog** + per-law modifiers | occupation, state panel `select_law_icon` | parse `common/occupation_laws/` |
| **Garrison-template catalog** + costs | occupation, garrison_log, state panel | parse `common/units/` templates |
| **Trait catalog** (leader/unit traits) + prereqs/effects, **tactic** catalog, **ace bonus** def | trait trees, officer corps, aces, unit/navy leader | parse `common/unit_leader/`, `common/combat_tactics`, `common/aces` |
| **Tech-tree & doctrine node GEOMETRY** + link/XOR lines, per-tech XP cost, tech **effect** rows, doctrine mutual-exclusion | tech_tree, doctrine_tree | node positions live in the `.gui` containers (adapter reads them); effects in `common/technologies/` + loc |
| **Focus PP cost / availability** def | national_focus (already mapped; degrade noted) | parse `common/national_focus/` |
| **Advisor eligibility** def, **idea/spirit** catalog | advisor_role_selection, politics ideas grid | parse `common/ideas/`, `common/scripted_effects` |
| **Module / equipment-archetype DEF stats & costs** (what each module contributes) | all designers | parse `common/units/equipment/` modules — distinct from the computed (b) total |
| **Operation / casus-belli / opinion-modifier / insignia / emblem** catalogs | operation_overview, diplomacy, war_overview, insignia_selection | parse `common/operations`, `common/opinion_modifiers`, `gfx` insignia |
| **State / province NAMES**, terrain, strategic-location, VP-by-province static | state/province panel, map | `map_pipeline` metadata + `history/states` — static map data, not the snapshot |

---

## 5. Historical review round (consolidation-level)

**Historical note:** this records completed reviews from the old Claude-era
workflow. Codex-native sessions must use normal sub-agents/partner review and
must not run the old local CLI command.

**Per-doc reviews (folded into each doc's "Codex round"):** notable corrections Codex caught and the
children fixed — strategic-air per-type plane counts re-badged proxy→**derived** (aggregate
`air_wings[]`, not `air_superiority`); air-wing `effective_planes` unavailable→**proxy** (captured
runtime field); `countrytradeview` is **resource** trade, not the equipment market (reclassed);
fielded-manpower/division counts **derivable** from top-level `divisions[]` (unavailable→derived);
designer combat stats confirmed **(b) computed-not-stored** per §11; doctrine techs confirmed absent
from `research.technologies[]` in the demo (a parser-unfed channel, **not** "not researched");
scripted-GUI VP system confirmed the **one** globally-bindable case (uses
`world.variables.allies_score/axis_score`), the rest country-scoped → chrome-only. Two child Codex
runs timed out at 5 min and proceeded with an independent self-audit per protocol.

**This consolidation** was Codex-reviewed for inventory completeness (`xhigh`/`priority`, exit 0).
Verdict: **structural in-game `.gui` coverage passes; no hidden missing view found.** Specifics: the
old `abilitylist.gui` omission is closed (now `ability_list.json`, in the military map);
`destroyed_ships` documented-present-but-empty is acceptable for `.gui` coverage (`windows: 0` is a
real stub, not a layout hole — kept badged/deferred for runtime-instanced loss-roster data). **One
reviewer-sensitive edge Codex raised:** MP **`chat`** — scoped out as frontend/lobby (and the
embedded menubar chat/message widgets are deferred/inert). A recorded replay carries no live
chat-comms state to bind, so `chat.gui` is correctly in the frontend-shell exclusion (§1c), **not** a
silent drop; were the contract ever widened to "capture all in-match communications," a chat
**transcript capture** would become a distinct DATA work-item (a logging/policy decision, not a `.gui`
binding gap) — recorded here so it is not a silent hole either.

---

## 6. What "100%" means here (honesty note)

"100% of in-game views accounted for" = every in-game `.gui` is either **mapped** (its elements bound
to snapshot fields with formatter + fidelity) or **gap-listed** (its missing field named in §4 as a
capture work-item with a closure bucket). It does **not** mean every field renders with live data
today — the demo snapshot is save-only and many domains are unfed artifacts (§0.1), and the bucket
**(b)** items are genuine runtime-computed walls. The contract is: **no in-game view is skipped, and
no missing field is a silent hole** — each is a named, classified backlog item. The grand-goal path to
literal full fidelity is: fold a full LIVE snapshot (clears all §0.1 artifacts) → add the bucket-(a)
readers → add the bucket-(c) `common/`-parsers (mod-aware) → accept the bucket-(b) hard limits as
badged proxies/derived, never fabricated.
