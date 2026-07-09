# UI COVERAGE MATRIX — every the game panel vs replay derivation status

**Lane: UI + map-mode INVENTORY (docs-only). Authored 2026-07-09.**
Companion residual ledger to `REPLAY_PRODUCTION_PLAN.md` Phase D and
`UI_LAYOUT_LAW.md` §6.3/§6.4. One row per shipped `.gui` panel FILE. This is the
honest, exhaustive map of what remains to derive every UI panel at full fidelity
per the 2026-07-09 user scope decision (v1 renders **every** panel + **all** map
modes, interactive controls rendered read-only).

## 0. Methodology, scope, and how to read the counts

- **Granularity = one row per `.gui` file.** The file is the enumerable, citable
  unit. Vanilla `interface/*.gui` (130 files) + integrated/standalone DLC music
  panels (3 files) = **133 panel files**. Deeper granularity would be the
  **2,009 `containerWindowType` blocks** those files declare (measured:
  `grep -c containerWindowType interface/*.gui`) — every list-row template,
  sub-window and tooltip frame. The per-file rows below each note when a file is
  a multi-window family; the 2,009 figure is the true sub-panel work surface.
- **Data-driven content is NOT a panel.** 128 event files and 88 decision files
  (`events/*.txt`, `common/decisions/*.txt`) declare **hundreds** of distinct
  event/decision instances, but they all render through a **handful of chrome
  panels** (`eventwindow.gui`, `popupwindow.gui`, `countrydecisionview.gui`).
  The replay renders the chrome once and binds instance content from recording
  tokens + the localisation join (requirements item 9). So "hundreds of events"
  = 1–3 panel rows here, not hundreds.
- **Status vocabulary:**
  - `LAW-solid` — the layout/markup/widget LAW that composes this panel is
    file-cited and gated (see `UI_LAYOUT_LAW.md`); applies to all panels via
    `resolve_element_rect` (game_ui parses all 453 `.gui`, 0 failures).
  - `chrome-composable` — skeleton rect/sprite/text-key composition is
    file-derivable NOW through the solid law, but no data is bound yet.
  - `POPULATED` — data binding from `WorldTimeline` is implemented (in progress).
  - `not-started` — neither composition proof nor binding done for this panel.
- **Interactive-only note.** A replay is read-only: every button/slider/drag
  control renders **inert** (UI_LAYOUT_LAW §6.4 gap-class 3). Panels whose whole
  purpose is a mutating control (designers, deployment drag, nudger) are still
  IN SCOPE per the user directive — rendered as a read-only snapshot of the
  state they displayed. Flagged `[interactive→read-only]` below.
- **Data dependency** column cross-refs `REPLAY_DATA_REQUIREMENTS.md` item # or
  flags `NEW` (needs a new requirements row — consolidated in §9 + LANE_REPORT).

---

## 1. Major views — full-screen country-management / tree / designer panels (28)

These are the deep panels a player opens over the map. Layout is `chrome-composable`
today (law solid); population is the bulk of Phase D beyond D1–D4. Every one needs
the localisation join (item 9) for its token→name text plus its own state stream.

| Panel | source .gui | category | status | data dependency | beyond-layout need |
|---|---|---|---|---|---|
| Politics (ideas/laws/party/advisors) | countrypoliticsview.gui | major-view | chrome-composable | 7b + **NEW: politics detail** (active ideas/spirits, party popularities, advisors, laws) | binding + computed tooltips |
| National focus tree | nationalfocusview.gui | major-view | chrome-composable | **NEW: focus state** (current/completed/progress/available) | tree layout law + focus icons |
| Research (list) | countrytechnologyview.gui | major-view | chrome-composable | **NEW: research state** (done/in-progress/ahead penalty) | binding |
| Tech tree | countrytechtreeview.gui | major-view | chrome-composable | **NEW: research state** | tree line/prereq layout |
| Doctrine tree | countrydoctrinetreeview.gui | major-view | chrome-composable | **NEW: research state (doctrine subset)** | tree layout |
| Production lines | countryproductionlineview.gui | major-view | chrome-composable | 10 (construction ext.) + **NEW: production queue** (equipment lines, factories, progress) | binding + listbox rows |
| Construction | countryconstructionsview.gui | major-view | chrome-composable | 10 (buildings + construction lines) | binding |
| Equipment overview / stockpile | equipmentoverview.gui | major-view | chrome-composable | **NEW: equipment stockpile** (per-archetype counts, variants) | binding |
| Trade | countrytradeview.gui | major-view | chrome-composable | **NEW: trade state** (resource balance, trade deals) | binding |
| Deployment | countrydeploymentview.gui | major-view `[interactive→read-only]` | chrome-composable | 3 (divisions in training/deploy) + templates | read-only snapshot |
| Army management | countryarmyview.gui | major-view | chrome-composable | 3 (divisions, army groups) | binding + hierarchy |
| Navies overview | naviesview.gui | major-view | chrome-composable | 8 (naval task forces) | binding |
| Strategic air | strategicairview.gui | major-view | chrome-composable | 8 (air) | binding |
| Air group view | airgroupview.gui | major-view | chrome-composable | 8 (air wings) | binding |
| Logistics | countrylogisticsview.gui | major-view | chrome-composable | 11 (rail/supply) | binding |
| Occupation | countryoccupationview.gui | major-view | chrome-composable | **NEW: occupation state** (resistance/compliance/laws/garrison) | binding (also feeds resistance/compliance map modes) |
| Intelligence agency | countryintelligenceagencyview.gui | major-view | chrome-composable | **NEW: intel/operatives state** | binding |
| Intel ledger | countryintelledger.gui | major-view | chrome-composable | **NEW: intel state** | binding |
| Operation overview | operationoverview.gui | major-view | chrome-composable | **NEW: intel operations** | binding |
| Officer corps | countryofficercorpview.gui | major-view | chrome-composable | **NEW: character/leader roster** | binding |
| Diplomacy | countrydiplomacyview.gui | major-view | **POPULATED (D4)** | 8b (relations/factions/wars) | relation_strip_view composes; finish rows |
| State info | countrystateview.gui | major-view/popup | chrome-composable | 2 + 10 (owner/control/buildings) | binding |
| Decisions | countrydecisionview.gui | major-view | **DEF-MAPPED (U2)** — layout + decision-def→card mapping done; state pending | **NEW: decision state** (available/active/timers/costs) + 9b alert counts | `decision_chrome.rs`: layout gate + 4125 decisions/591 cats mapped (name 3839/4125, icon 3518/3595); binding fail-closed until decision-state stream (A21) |
| Division designer | divisiondesignerview.gui | major-view `[interactive→read-only]` | chrome-composable | **NEW: division templates** | read-only snapshot |
| Equipment designer | equipmentdesignerview.gui | major-view `[interactive→read-only]` | chrome-composable | **NEW: equipment variants** | read-only snapshot |
| Tank designer (NSB) | tank_designer_view.gui | major-view `[interactive→read-only]` DLC | chrome-composable | **NEW: equipment variants** | read-only, DLC-gated |
| Plane designer (BBA) | plane_designer_view.gui | major-view `[interactive→read-only]` DLC | chrome-composable | **NEW: equipment variants** | read-only, DLC-gated |
| Equipment upgrade designer | equipmentupgradedesignerwindow.gui | major-view `[interactive→read-only]` | chrome-composable | **NEW: equipment variants** | read-only |
| Power balance (BBA) | powerbalanceview.gui | major-view DLC | chrome-composable | **NEW: power-balance state** | binding, DLC-gated |
| Peace conference | peaceconferencewindow.gui | major-view | chrome-composable | **NEW: peace-conference state** (rare; descope candidate) | binding |
| Naval strategic region | naval_strategic_region_view.gui | major-view/popup | chrome-composable | 5 + 8 (supremacy/naval) | binding |

Count: **31 rows** (28 distinct major panels; designers grouped). Note: peace
conference is a v1 descope candidate (rare, deferred with item 12 territory).

---

## 2. Persistent HUD chrome (on-screen every frame) (24)

| Panel | source .gui | status | data dependency | beyond-layout need |
|---|---|---|---|---|
| Topbar | topbar.gui | **POPULATED (D1/D2)** | 9b scalars + 7/7b identity | binding.rs done; ledger §4 quad gate |
| In-game clock | ingameclock.gui | chrome-composable (D6 partial) | 4 (date/speed) | replay-native clock |
| Outliner | theatreselector.gui | **POPULATED (D3)** | 3/8 (unit lists) | listbox row population in progress |
| Minimap | minimap.gui | chrome-composable | 2 (owner colors) | minimap render + mode |
| Map-mode bar | mapmodes_interface.gui | chrome-composable | active mode = replay-selected (not recorded) | see MAPMODE_COVERAGE_MATRIX |
| Menubar | menubar.gui | chrome-composable `[interactive→read-only]` | none (inert) | static chrome |
| Alerts / notifications | alerts.gui + topbar.gui | **POPULATED (U1)** — 4 topbar count badges | 9b alert counts (initiative/decisions/intel/market_contracts) + **NEW A18** (badge sub-counts + fired-alert feed) | `game_ui::alerts` binds the 4 counts; feed (`global_alerticon_window`) is recording-Absent → fail-closed; layout gate green |
| Map icons (VP/counters furniture) | mapicons.gui | **counter CONTENT POPULATED (U1)** | 2 + 3 + 8b (VP/units/relation cols) | FURN-REL law partly done; unit-counter CONTENT bound (`game_ui::binding_counters`: name/str-org bars/NATO key/owner-colour/A15 badges, fail-closed) — counter GEOMETRY/stacking/bar-render still Phase C1/CAP6 |
| Selected-unit panel | unitview.gui | **POPULATED (U1)** | 3/8 (selected div/fleet/wing) | `game_ui::unitview` binds land/naval/air detail panels; layout-gated + fixture-gated. VIEW GAPS: div/task-force display name + land experience (see LANE_REPORT) |
| Battle-plan tools | battleplantools.gui | chrome-composable `[interactive→read-only]` | 12 (plans, deferred) | inert; arrows deferred |
| Right-click context menu | rightclickmenu.gui | chrome-composable `[interactive→read-only]` | none | inert |
| Ability bar | abilitylist.gui | chrome-composable | **NEW: active command abilities** | binding |
| Divisions summary | divisions_summary.gui | chrome-composable | 3 (divisions) | binding |
| Unit leader window | unitleaderwindow.gui | chrome-composable | **NEW: leader roster** | binding |
| Find/search | findview.gui | chrome-composable `[interactive→read-only]` | 2/3 (searchable entities) | inert or read-only list |
| Air selection | airselectionview.gui | chrome-composable `[interactive→read-only]` | 8 (air) | read-only |
| Command character (leader) | command_character_view.gui | chrome-composable | **NEW: leader roster** | binding |
| Ingame menu (esc) | ingamemenu.gui | chrome-composable `[interactive→read-only]` | none | inert |
| Chat (MP) | chat.gui | chrome-composable | **NEW: chat log (optional session trivia)** | descope candidate |
| Social view | social_view.gui | chrome-composable | none (account) | inert / descope |
| Music player | musicplayer.gui | chrome-composable `[interactive→read-only]` | none | inert |
| Music station base | music_station_base.gui | chrome-composable | none | inert |
| Music station classic | music_station_classic.gui | chrome-composable | none | inert |
| Core widgets + tooltip frame | core.gui | LAW-solid | — (base templates + `ToolTip`) | tooltip lines = computed-not-stored (§4) |

Count: **24 rows.** `core.gui` is the base-widget + tooltip-frame library that
every other panel references (multi-window; LAW-solid via markup/widget law).

---

## 3. Popups — combat / detail / info windows (23)

Fired during a war replay; high value (Phase D wave 2). All chrome-composable.

| Panel | source .gui | status | data dependency | note |
|---|---|---|---|---|
| Event window | eventwindow.gui | **chrome-DERIVED (U2 2026-07-09)** | 9 (event id→loc) + **NEW: fired-event stream (item 14 / A18, FILED)** | 4 window variants + option-row template layout-gated; **6580/6757 visible events (97.4%) resolve to a fully renderable window** (title+desc+picture+option loc/gfx joined); content fail-closed until the fired-event stream lands |
| Generic popup | popupwindow.gui | chrome-composable | 9 + event stream | shared popup chrome (diplomacy-request/lend-lease popups — distinct binding surface from the fired-event window, not U2) |
| World-tension popup | worldtensionwindowpopup.gui | chrome-composable | 9b (world_tension) | binding |
| Land combat | landcombat.gui | **POPULATED (U1)** | 3 (combats) + **A16 (display detail, OPEN)** | `combat_binding.rs`: identity + participant division counts bound from `replay_core::CombatRow`; progress/bombardment/air-superiority/combat-width/leaders are recording-Absent (A16) → fail-closed |
| Naval combat | navalcombat.gui | chrome-composable | 8 + **NEW: naval combat detail** | binding |
| Naval combat results | navalcombatresultswindow.gui | chrome-composable | **NEW: naval combat results** | binding |
| Naval losses | naval_losses.gui | chrome-composable | **NEW: naval losses** | binding |
| Destroyed ships | destroyedshipswindow.gui | chrome-composable | **NEW: naval losses** | binding |
| Combat log | combatlogview.gui | chrome-composable | 3/8 (combat events) | binding |
| War overview | waroverview.gui | **POPULATED (U1)** | 8b (wars/participants) + **NEW: war-participant stats (OPEN)** | `waroverview_binding.rs`: belligerent-pair casualties (from `DiplomacyView::war_relations`) + belligerent/POV-belligerent sets bound; war-object grouping (attacker/defender sides, front) is a `replay_core` view gap; warscore/divisions/IC/surrender-progress are a NEW recording dep → fail-closed |
| Insurrection/civil-war window | insrevolutionwindow.gui | chrome-composable | 7/7b (splinter/ideology) | binding |
| Garrison log | garrisonlogview.gui | chrome-composable | **NEW: occupation/garrison state** | binding |
| Air wing details | airwingdetails.gui | chrome-composable | 8 (air) | binding |
| Carrier air-wing composition | carrierairwingcompositionwindow.gui | chrome-composable | 8 (naval/air) | binding |
| Aces view | acesview.gui | chrome-composable | **NEW: aces roster** | binding |
| Army trait tree | armytraittreewindow.gui | chrome-composable | **NEW: leader traits** | binding |
| Navy trait tree | navytraittreewindow.gui | chrome-composable | **NEW: leader traits** | binding |
| Trait tree common | traittreewindow_common.gui | LAW-solid (shared template) | **NEW: leader traits** | shared chrome |
| Navy leader view | navyleaderview.gui | chrome-composable | **NEW: leader roster** | binding |
| Leader groups | leadergroups.gui | chrome-composable | **NEW: leader roster** | binding |
| Operative | operative.gui | chrome-composable DLC | **NEW: intel/operatives** | binding, LaR-gated |
| Operative leader | operativeleader.gui | chrome-composable DLC | **NEW: intel/operatives** | binding, LaR-gated |
| Detailed windows | detailedwindows.gui | chrome-composable | various (shared detail chrome) | shared templates |
| Railway gun (NSB) | railway_gun.gui | chrome-composable DLC | 3 (railway-gun unit) | binding, NSB-gated |
| Advisor role selection | advisorroleselectionview.gui | chrome-composable `[interactive→read-only]` | politics detail | read-only |
| Insignia selection | insigniaselection.gui | chrome-composable `[interactive→read-only]` | none | inert |
| Airwing reorganization | airwingreorganization.gui | chrome-composable `[interactive→read-only]` | 8 (air) | read-only (widget gate uses this file) |

Count: **27 rows.**

---

## 4. Country scripted-GUI — national-mechanic panels (23)

`interface/*_scripted_gui.gui` + a few `*_decision_ui.gui`. Each shows a
country-specific mechanic driven by scripted variables/effects the engine
evaluates at runtime. **These are the hardest population class:** their content
is `computed-not-stored` script state (UI_LAYOUT_LAW §6.4 gap-class 2) — a replay
needs the underlying scripted variables per country per hour to populate them.

| Panel | source .gui | country/mechanic | data dependency |
|---|---|---|---|
| Cabinet trust | AST_cabinet_trust_scripted_gui.gui | Australia | **NEW: scripted-GUI variable state** |
| Industry focus | CZE_industry_focus_scripted_gui.gui | Czechoslovakia | NEW: scripted-var state |
| Inner circle | GER_inner_circle_scripted_gui.gui | Germany | NEW |
| Inner circle (hidden) | GER_inner_circle_hidden_scripted_gui.gui | Germany | NEW |
| Reichskommissariat | GER_rk_scripted_gui.gui | Germany | NEW |
| Imperial influence | JAP_imperial_influence_scripted_gui.gui | Japan | NEW |
| EIC tax fraud | RAJ_eic_tax_fraud.gui | Raj | NEW |
| Famine | RAJ_famine.gui | Raj | NEW |
| Local leaders investments | raj_local_leaders_investments_decision_ui.gui | Raj (decision-dialog) | NEW |
| Khana Ratsadon | SIA_khana_ratsadon_scripted_gui.gui | Siam | NEW |
| Movie-theater campaigns | sia_movie_theater_campaigns_scripted_gui.gui | Siam | NEW |
| Right-vs-left campaign | ast_right_vs_left_campaign_scripted_gui.gui | Australia | NEW |
| Right-vs-left (empty) | ast_right_vs_left_campaign_empty_scripted_gui.gui | Australia | NEW |
| Anti-Anschluss measures | aus_antischluss_measures_scripted_gui.gui | Austria | NEW |
| Internal factions | bul_internal_factions_scripted_gui.gui | Bulgaria | NEW |
| Internal factions | gre_internal_factions_scripted_gui.gui | Greece | NEW |
| Monroe doctrine | ger_monroe_doctrine_scripted_gui.gui | Germany | NEW |
| Paranoia system | sov_paranoia_system_scripted_gui.gui | Soviet | NEW |
| Propaganda campaigns | sov_propaganda_campaigns_scripted_gui.gui | Soviet | NEW |
| Rearmament system | swe_rearnament_system_scripted_gui.gui | Sweden | NEW |
| Train the sheltered | swe_train_the_sheltered_gui.gui | Sweden | NEW |
| Congress | usa_congress_scripted_gui.gui | USA | NEW |
| War escalation | war_escalation_scripted_gui.gui | global | NEW |

Count: **23 rows.** All share ONE consolidated new dependency (scripted-GUI
variable state); mod support multiplies this class arbitrarily (mods add their
own `*_scripted_gui.gui`), so the derivation must be a **generic scripted-GUI
composer** driven by recorded script variables, not 23 bespoke panels.

---

## 5. Frontend / meta / dev — OUT OF SCOPE for in-game replay (enumerated for completeness) (24)

A replay opens an in-game moment; these never appear during playback. Enumerated
so the count is honest and nothing is silently dropped. Status = `not-started`
and recommended **v1 descope** (except `load_screen` if a load chrome is wanted).

| Panel | source .gui | class | disposition |
|---|---|---|---|
| Frontend main | frontendmainview.gui | frontend | descope |
| Frontend game setup | frontendgamesetupview.gui | frontend | descope |
| Frontend multiplayer | frontendmultiplayerview.gui | frontend | descope |
| Frontend friends | frontend_friends_view.gui | frontend | descope |
| Frontend credits | frontendcreditview.gui | frontend | descope |
| Credits | credits.gui | meta | descope |
| Load screen | load_screen.gui | meta | optional (replay load chrome) |
| Tutorial screen | tutorialscreen.gui | meta | descope |
| Achievements | achievements.gui | meta | descope (widget gate uses this file) |
| Backend (account) | backend.gui | meta | descope (widget gate uses this file) |
| Browser | browser.gui | meta | descope |
| Pops login | pops_login.gui | account | descope |
| Subscription message | subscription_message_view.gui | meta | descope |
| Settings | settings.gui | settings `[interactive]` | descope |
| Message settings | messagesettings.gui | settings `[interactive]` | descope |
| Message filters | msg_filters.gui | settings `[interactive]` | descope |
| Saved game rules | savedgamerules.gui | frontend | descope |
| Player AI prefs | playeraiprefs.gui | frontend | descope |
| Ingame lobby | ingamelobby.gui | MP | descope |
| Player lobby | playerlobby.gui | MP | descope |
| Hotjoin | hotjoin.gui | MP | descope |
| Matchmaking | matchmaking.gui | MP | descope |
| Matchmaking chat | matchmaking_chat.gui | MP | descope |
| Nudge (map editor) | nudge.gui | dev/editor | descope |
| AI trace | ai_trace.gui | dev/debug | descope |

Count: **25 rows.**

---

## 6. DLC-directory panels (3)

Only music panels ship as separate DLC `interface/*.gui`; all other DLC UI
(operative, designers, power balance, railway gun, RK, etc.) ships in **base**
`interface/` and is DLC-gated at runtime — already rowed in §1–§4 with a `DLC`
tag. Enumerated: `integrated_dlc/dlc023_man_the_guns/interface/mtg_music.gui`,
`dlc/dlc013_sabaton/interface/sabaton_music.gui`,
`dlc/dlc019_sabaton_vol2/interface/sabaton_music.gui`. Status `not-started`,
disposition **descope** (music UI, inert). Count: **3 rows.**

---

## 7. Tooltips (cross-cutting, not a file class)

Tooltips are not separate `.gui` files — every element carries `pdx_tooltip` /
`pdx_tooltip_anchor` keys and renders through the `core.gui` `ToolTip` frame
(§4). Tooltip **content** is `computed-not-stored` (UI_LAYOUT_LAW §6.4 gap-class
2): a replay can render static loc-key tooltips file-derived, but any tooltip
with `[GetXxx]` data functions (combat/supply/production breakdowns) needs either
a bounded evaluator or an "unavailable" badge. Status: LAW-solid for the frame +
static keys; `not-started` for dynamic-breakdown tooltips. Tracked as ONE
cross-cutting residual, not per-panel.

---

## 8. Category counts + grand total

| Category | rows | dominant status |
|---|---|---|
| Major views (§1) | 31 | chrome-composable; 1 POPULATED (diplomacy) |
| HUD chrome (§2) | 24 | chrome-composable; 5 POPULATED (topbar, outliner, alerts, selected-unit, map-icon counter content) |
| Popups (§3) | 27 | chrome-composable; 2 POPULATED (land combat, war overview — U1) |
| Country scripted-GUI (§4) | 23 | chrome-composable shell / population not-started |
| Frontend/meta/dev (§5) | 25 | not-started / descope |
| DLC music (§6) | 3 | not-started / descope |
| **Panel-file total** | **133** | — |
| Tooltip frame (§7) | cross-cutting | LAW-solid frame, dynamic content not-started |

**Grand total = 133 panel files** (130 base `interface/*.gui` + 3 DLC music).
True sub-panel surface = **2,009 `containerWindowType` window definitions**.
POPULATED today: **8 panels** (topbar D1/D2, outliner D3, diplomacy D4, alerts
strip U1, selected-unit panel U1, map-icon counter content U1, land combat U1,
war overview U1) + the observer-POV shell (E6, a `replay_app` mode, not a
panel). Everything else is `chrome-composable` (layout law solid, binding
pending) or descoped.
`chrome-composable` (layout law solid, binding pending) or descoped.

In-scope for a replay (excludes §5 frontend/meta + §6 music) = **105 panels**:
28 major views + 24 HUD + 27 popups + 23 scripted-GUI + 3 designers already
counted in majors. Of those, **102 need data-binding work beyond the layout law**.

---

## 9. New recording-data dependencies surfaced by this enumeration

Panels above reference **14 new streams** not yet rows in
`REPLAY_DATA_REQUIREMENTS.md` (consolidated for the orchestrator to fold in). All
are country-scoped, hourly/on-change, and degrade to unavailable badges:

1. **Research state** — techs completed / in-progress / ahead-of-time penalty
   (countrytechnologyview, techtree, doctrinetree).
2. **National-focus state** — current focus, completed set, progress, available
   (nationalfocusview).
3. **Politics detail** — active ideas/national spirits, party popularities,
   advisors/ministers, conscription/economy/trade laws (countrypoliticsview).
4. **Production queue** — equipment lines, factories assigned, progress,
   efficiency (countryproductionlineview) + **equipment stockpile** per archetype.
5. **Division / ship / plane templates & equipment variants** (all designer views;
   read-only snapshot).
6. **Intelligence / operatives state** — agency, operatives, operations, ciphers,
   intel levels (intelligenceagencyview, intelledger, operationoverview, operative).
7. **Trade state** — resource import/export balance, trade deals (countrytradeview).
8. **Occupation state** — resistance/compliance per state, occupation laws,
   garrison assignment (countryoccupationview, garrisonlogview) — **also feeds the
   resistance + compliance map modes** (see MAPMODE_COVERAGE_MATRIX).
9. **Character/leader roster** — officers, high command, unit leaders, admirals,
   traits, aces (officercorpview, unitleaderwindow, leadergroups, acesview,
   trait trees).
10. **Power-balance state** — BBA power-balance sliders/positions (powerbalanceview).
11. **Decision state** — available/active decisions, timers, costs (countrydecisionview;
    partly overlaps 9b alert counts).
12. **Naval/air combat detail + losses** — combat participants/results, ships/planes
    lost (navalcombat, navalcombatresultswindow, naval_losses, destroyedshipswindow,
    combatlogview).
13. **Fired-event stream** — which events fired, to whom, when + chosen option
    (eventwindow, popupwindow); pairs with localisation join (item 9). **FILED
    (U2 2026-07-09): REPLAY_DATA_REQUIREMENTS §14 + EXTRACTION_NEEDS §A A20.**
    Chrome layout + the event-def→window mapping are DONE and gated
    (`game_ui::event_chrome`); only the per-moment fired content waits on this
    stream.
14. **Scripted-GUI variable state** — per-country scripted dynamic variables the 23
    national-mechanic panels display; must be a GENERIC recorded var map (mod-extensible),
    not 23 bespoke fields.

Plus one panel-side extension to an existing item: **active command abilities**
(abilitylist.gui) — fold into item 3/9b.
