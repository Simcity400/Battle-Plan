# Native Replay Interaction Parity Inventory

Status: first-pass inventory plus executable view-level scaffold, executable
coverage report, and a first tab-target inventory slice. This file does not
close product parity.

This inventory supports `REPLAY_PARITY_COMPLETION_CHECKLIST.md` section 2:
every registered replay view still needs an interaction contract, implementation
where the interaction is read-only, inert/disabled handling where the original
game control mutates state, and screenshot or side-by-side proof. The existing
engineering DoD proves registry coverage, real `.gui` binding, no-fabrication,
and scrub-safe rendering. It does not prove that every visible the game inspection
interaction works.

## Evidence Used

- `AGENTS.md`: current Codex-native rules; legacy Claude/local-CLI review
  workflow is explicitly retired and must not be used by Codex subagents.
- `DESIGN.md`: product target is a maximal-depth replay/review tool with no
  fabricated data.
- `replay_ui/SESSION_HANDOFF.md`: engineering DoD is green, product parity is
  still open.
- `replay_ui/REPLAY_PARITY_COMPLETION_CHECKLIST.md`: interaction parity,
  tooltips, visual parity, hourly capture, and data gaps remain blockers.
- `replay_native/app/src/host/{registry.rs,view.rs,window_host.rs,input.rs}`:
  registry, host policy, open/close, z-order, modal/blocking hit tests, input
  routing, and scrub rebind primitives.
- `replay_native/app/src/main.rs`, `scrubber.rs`, `drill.rs`, and core views:
  event-loop evidence for map pan/zoom/pick, menubar open, scrubber seek, focus
  graph input, and combat bubble selection.
- `replay_native/app/src/views/clusters/*`: distributed `register_view!`
  descriptors, cluster comments, view-specific `on_input` implementations, and
  cluster registration/scrub/accounting tests.
- `replay_native/app/src/views/interaction_contract.rs`: registry-derived
  executable scaffold with one contract per registered view, explicit statuses
  for the category set below, an executable coverage report, and first
  target-level `tab_switch`/`row`/`drill` inventory rows for known N2/tab/list
  slices.

A deterministic `--shot-view=<view_id>` app hook now opens a registered view
through the live registry before the existing `--shot` compose path.
`--shot-all-views=<dir>` now writes a registry-derived manifest of planned
per-view screenshot targets (`shot_manifest.json`) without a static view-id
list. Each registered view gets deterministic native, HOI4-reference, and diff
filenames; `core.state_panel` targets carry an explicit default state id. No
per-view image-emitting harness run or side-by-side the game comparison was run for
this inventory.

## Executable Scaffold Status

The first native scaffold is now code-backed, not just prose:

- `views::interaction_contract::interaction_contracts(&ViewRegistry)` derives
  contract rows from the registered view descriptors, with no separate static
  view-id list.
- `views::interaction_contract::interaction_coverage_report(&ViewRegistry)`
  builds an executable accounting report from the registry plus those contracts.
- The scaffold tracks `open_close`, `tab_switch`, `list_selection`,
  `text_input`, `scroll`, `hover_tooltip`, `drilldown`, `disabled_mutation`,
  `scrub_state`, and
  `screenshot_proof` for every registered view.
- Contract rows can now carry target-level inventory rows. The first slices are
  `tab_switch` rows for known N2 tab widgets with stable tab ids/labels:
  `politics.politics` (Government, National Spirits, Ideas & Laws, Autonomy),
  `military.army` (Army, Navy, Air), `designers.division` (Division, Army HQ),
  `intel.intel_agency` (Agency, Operations, Cryptology), `chrome.find`
  (States, Countries, Victory Points), and `logistics.unit_view` (Stats,
  History, Template), plus `chrome.find` `list_selection`
  rows (State/Country/Victory Point row), `chrome.find` `text_input` row
  (query edit box), `chrome.find` `scroll` row (results wheel/thumb scroll),
  `military.army` `list_selection` rows
  (Army/Navy/Air roster row), `military.army` `scroll` rows
  (Army/Navy/Air roster wheel/thumb scroll), `chrome.find` `drilldown` rows
  (state/VP row to state panel plus Find/Goto Capital map recenter),
  `core.state_panel` `scroll` row (building-grid wheel/thumb scroll) plus
  `hover_tooltip` row (owner/building/resource factual hover tooltips),
  `core.topbar` `drilldown` rows for the real topbar navigation buttons, and a
  `chrome.minimap` `drilldown` row for click/drag recenter.
  `chrome.mapmodes` now adds `list_selection` rows for the 12
  real numbered `.gui` rail buttons plus the real `mapmodes_all` popup path for
  Supply/Intel, and a `hover_tooltip` target for source/fidelity readouts.
  `logistics.unit_view` now adds `scroll` targets for division-list wheel/thumb
  scroll over measured `div_id` rows and selected-division equipment-pane
  wheel/thumb scroll.
- Focused app tests assert the registry still contains 114 views, every
  registered view has exactly one contract, and every contract has an explicit
  status for every category.
- New target tests assert that real registered views `chrome.find`,
  `military.army`, `politics.politics`, `designers.division`, and
  `intel.intel_agency` have non-empty `tab_switch` target inventories,
  `chrome.find` has explicit row/text-input/scroll/drilldown target inventories,
  `core.topbar` has explicit topbar navigation-button drilldown target
  inventories,
  `chrome.minimap` has an explicit click/drag recenter target inventory,
  `chrome.mapmodes` has explicit map-mode row and hover target inventories,
  `logistics.unit_view` has explicit stats-panel tab target inventory,
  `core.map` has an explicit factual province-hover target inventory,
  `core.state_panel` has explicit building-grid wheel/thumb scroll and factual
  owner/building/resource hover-tooltip target inventories,
  `military.theatre_selector` has explicit measured theatre-row and
  wheel-scroll/thumb-drag target inventories,
  `military.army` has explicit Army/Navy/Air roster wheel-scroll/thumb-drag
  target inventories,
  `production.production` has explicit production-line row and wheel-scroll
  target inventories,
  `production.construction` has explicit construction-queue row and
  wheel-scroll/thumb-drag target inventories,
  `production.decisions` has explicit timed-decision row and wheel-scroll/thumb-drag
  target inventories plus a disabled-mutation target for visible timed-decision
  `btn_select` actions, `leaders.army_trait_tree` and
  `leaders.navy_trait_tree` have explicit disabled-mutation targets for rendered
  `add_trait_0` through `add_trait_3` trait-pick slots, and
  `logistics.unit_view` has explicit division-list and
  equipment-pane wheel-scroll/thumb-drag target inventories. `politics.politics`
  now has explicit Government/National Spirits/Ideas & Laws/Autonomy row target
  inventories plus long National Spirits and Ideas & Laws wheel-scroll/thumb-drag
  target inventories, `research.research` has an explicit active-research-slot
  row target inventory, `research.tech_tree` has explicit tech-node click
  selection and factual tech-node hover target inventories, and
  `research.doctrine_tree` has explicit doctrine-node click selection and
  factual doctrine-node hover target inventories. `designers.tank`,
  `designers.plane`, and `designers.equipment` have explicit component
  module-slot selection targets, passive module-slot hover-inspection targets,
  and disabled-mutation targets for their shared non-slot chrome/control area.
  `core.focus_tree` has an explicit focus-node click-selection target inventory.
  Every recorded target has a stable id/label/status/note, and views without
  known tab targets remain explicitly tracked as `not_proven` instead of
  disappearing.
- New report tests lock the current executable counts: 114 registered views, 114
  contracts, 1,140 category rows (ten per view), 123 target rows, 123 target rows
  with executable-proof notes, and 965 `not_proven` category rows.
- The report intentionally exposes no implemented/proven target count, because
  the current target status model only has `open` and `not_proven`. It does now
  separately count twenty intentionally inert mutation targets for the
  ingame-menu disabled verbs, state-panel occupation-law/template and
  scorched-earth controls, topbar pause/date and speed controls, topbar
  lobby/menu/help/achievements/playlist/music controls, topbar dismissed-alert
  and faction no-op controls, displayed event-definition
  preview option actions, displayed popup action area, construction top factory
  add/sub buttons, construction visible queue priority/amount/close buttons,
  production visible add-line buttons, production visible close-line buttons,
  production visible factory add/sub buttons, production visible priority/amount
  buttons, production visible manufacturer/equipment/convert/deploy/name-list
  line controls, timed-decision `btn_select` action buttons, and army/navy
  trait-tree add-trait slot buttons, plus tank/plane/equipment
  component-designer non-slot control areas,
  without converting those proofs into broad mutation parity.
- `chrome.find` now has executable read-only tab-switch proof: tests click
  States/Countries/Victory Points, verify the digest-visible catalog changes,
  prove non-pickable tab clicks are inert, and prove selected-tab state
  survives scrub A->B->A. This tab-switch proof alone does not cover localized
  names, country/province drilldown, or the other N2 tab inventories.
- `politics.politics` now has executable read-only tab-switch proof: tests
  click Government/National Spirits/Ideas & Laws/Autonomy, verify the
  digest-visible tab content changes, prove unavailable tab clicks are inert,
  and prove selected-tab state survives scrub A->B->A without carrying B-only
  rows. It now also has stable-keyed active-tab row selection/highlight proof
  and long National Spirits plus Ideas & Laws row wheel-scroll/thumb-drag proof;
  tests prove ideology, source-token, and autonomy-relationship row keys,
  duplicate/unkeyed row inert behavior, scrolled backing-row selection,
  track-click inertness, captured thumb drags continuing outside the list until
  pointer-up, per-tab scroll isolation, and row selection/scroll scrub A->B->A
  without carrying B-only rows. This does not cover Government/Autonomy long-list
  scroll, modal drilldowns, character/advisor/occupation subviews, exact
  scrollbar art, hover tooltips, disabled mutation visuals, or side-by-side proof.
- `military.army` now has executable read-only tab-switch proof: tests click
  Army/Navy/Air, verify the digest-visible roster changes, prove unavailable
  tab clicks are inert, and prove selected-tab state survives scrub A->B->A
  without carrying B-only rows. It now also has keyed Army/Navy/Air roster
  row-selection/highlight proof and roster wheel-scroll/thumb-drag proof; scrolled
  clicks select the correct backing row, captured thumb drags continue outside
  the list until pointer-up, track clicks do not select underlying rows,
  outside-list wheel input is inert, per-tab scroll state stays isolated, and
  row selection/scroll survive A->B->A. This
  still does not cover detail-pane drilldowns, other military list views,
  exact scrollbar art, tooltips, mutating order controls, or side-by-side proof.
- `military.theatre_selector` now has executable read-only theatre-row
  selection/highlight proof and theatre-list wheel-scroll/thumb-drag proof:
  tests select by measured theatre id, reject missing/duplicate ids without
  index/name fallback, prove scrolled clicks select the correct backing row,
  prove outside-list wheel input is inert, prove scrollbar-thumb drags clamp and
  continue outside the list until pointer-up, prove track clicks do not select
  underlying rows, and prove scroll/selection survive scrub A->B->A.
  Group/member drilldowns, alerts, pin/priority UI state, scrollbar art, and
  side-by-side proof remain open.
- `designers.division` now has executable read-only tab-switch proof: tests
  click Division/Army HQ when the focused template is an HQ template, prove
  non-HQ Army HQ clicks are inert, and prove selected-tab state survives scrub
  A->B->A without carrying B-only template rows. This does not cover module
  slot selection, template selector drilldowns, upgrade controls, tooltips, or
  side-by-side proof.
- `intel.intel_agency` now has executable read-only tab-switch proof: tests
  click Agency/Operations/Cryptology, prove unavailable tab clicks are inert,
  and prove selected-tab state survives scrub A->B->A. This does not cover
  operative/trade/war row selection, diplomacy drilldowns, disabled action
  controls, tooltips, or side-by-side proof.
- `chrome.find` now has a first executable row-selection/drilldown/navigation slice:
  tests prove snapshot-derived catalog row selection/highlight, outside-row
  inert clicks, selected-row scrub A->B->A behavior without carrying B-only row
  data, and read-only `OpenView(core.state_panel, state_id)` commands for
  state and Victory Point rows. Find/Goto Capital buttons now emit read-only
  `RecenterMap` commands from selected or first filtered state/country/VP rows
  and measured capital VP geometry. Country detail panels, province-level
  VP drilldown, localized names, and full find parity remain open.
- `core.topbar` now has a first executable read-only navigation slice: tests
  prove the real extracted topbar `.gui` exposes the registered navigation
  button rects, synced topbar button clicks emit read-only `OpenView` commands
  for Decisions/Intel/Research/Diplomacy/Trade/Construction/Production/
  Deployment/Logistics/Officer Corps/Army/Navy/Air, the topbar render digest is
  unchanged, and empty topbar space still falls through. Faction, alerts,
  hover frames/tooltips, disabled read-only behavior, screenshots, and broader
  topbar parity remain open.
- `core.topbar` now has a first disabled-mutation slice: tests prove the real
  extracted pause/date, speed-control, lobby/menu/help/achievements, playlist,
  and music-control buttons
  (`pause_button_date`, `pause_button_date_paused`, `button_speeddown`,
  `button_speedstep1` through `button_speedstep5`, `button_speedup`,
  `reopen_ingame_lobby`, `menu_button`, `help_button`, `achievements_button`,
  `playlist_button`, `music_pause_button`, `music_next_button`, and
  `musicplayer_button`) are consumed as replay-disabled no-op pointer-down/up
  clicks with no commands, no redraw, and unchanged topbar digest; shell tests
  also prove higher-z panels prevent hidden topbar inert targets from arming.
  `dismissed_alerts_button`, `threat_button`, `threat_value_button`,
  `faction_button`, disabled visual treatment, screenshots, and broader topbar
  mutation parity remain open.
- `core.state_panel` now has a first executable building-grid scroll slice:
  tests prove wheel scrolling and scrollbar-thumb drag over the real
  `state_building_entries` geometry change/clamp visible building rows,
  captured thumb drags continue until pointer-up, track clicks are consumed
  without scroll/row/map leakage, outside-grid wheel input is inert, and scrub
  rebind clamps/clears stale drag when selected-state building rows shrink.
  A resource-grid geometry test proves the six known state-resource rows fit
  without overflow in the measured resource grid, so no resource scrollbar is
  required under the current data model. Province/state drilldown links, exact
  scrollbar art, screenshots, and side-by-side proof remain open.
- `core.state_panel` now has a first factual hover-tooltip slice: tests prove
  passive owner-flag, scroll-aware building-row, and resource-row tooltips use
  the existing `StateBindReport` rows, clear when the pointer leaves, and clear
  stale building-row hover on scrub rebind instead of falling back to a fabricated
  row. Full the game tooltip frames, hover chrome, province/state drilldown links,
  screenshots, and side-by-side proof remain open.
- `core.state_panel` now has a first disabled-mutation slice: tests prove
  `select_law_button`, `template_garrison_button`, and
  `scorched_earth_state_button` are consumed as replay-disabled no-op
  pointer-down/up clicks with no commands, no redraw, and unchanged digest, while
  neighboring non-control panel space remains unclaimed by the view-specific
  handler. A separate executable guard proves `state_shared_slot_building_entry`
  `remove` remains unrendered/unclaimed even when the GUI defines that template;
  the next prerequisite is native shared-slot row-factory rendering. Disabled
  visual treatment, province/state drilldown links, screenshots, and side-by-side
  proof remain open.
- `chrome.find` now has a first executable text-input/filtering slice: tests
  focus the real `to_find_text` edit-box rect, type and backspace replay-local
  query text, filter over honest catalog labels/keys, reset/clamp scroll after
  filtering, consume focused text keys before global shortcuts, and preserve the
  query across A->B->A without carrying B-only rows. Localized names and full
  text-field parity remain open.
- `chrome.find` now has a first executable results-scroll slice: tests prove
  wheel scrolling and scrollbar-thumb drag change/clamp visible rows, captured
  thumb drags continue outside the list until pointer-up, track clicks do not
  select or drill underlying rows, scrolled clicks select the correct backing
  row, and scroll offset survives scrub A->B->A without carrying B-only rows.
  Exact scrollbar art and other scroll surfaces remain open.
- `production.construction` now has executable read-only construction-queue row
  selection/highlight proof and queue wheel-scroll/thumb-drag proof: tests select by
  measured live `line_id` or save `template`/`location_state`/`created_date`
  identity, reject unkeyed rows without index fallback, prove scrolled clicks
  select the correct backing row, prove outside-grid wheel input is inert, prove
  thumb drags clamp and continue outside the grid until pointer-up, prove track
  clicks do not select underlying rows, and prove scroll/selection survive scrub
  A->B->A. The rendered `production_building_line_entry` queue priority,
  amount, and close controls now have replay-disabled no-op proof with
  neighboring row-body click protection. Building drilldowns, scrollbar art,
  build-picker actions, market/debug controls, disabled visuals, and broader
  construction mutation parity remain open.
- `production.production` now has executable read-only production-line row
  selection/highlight proof and line-list wheel-scroll/thumb-drag proof: tests
  select by measured live `id` or save `line_id`, preserve military/naval line
  identity, reject unkeyed and duplicate ids without index fallback, prove
  scrolled clicks select the correct backing row, prove outside-grid wheel input
  is inert, prove thumb drags clamp and continue outside the grid until
  pointer-up, prove track clicks do not select underlying rows, and prove
  scroll/selection survive scrub A->B->A. Primary production category filters
  now toggle replay-local row visibility from catalog-derived categories while
  unknown categories fail open. Visible row mutation controls now have
  replay-disabled no-op proof for manufacturer attach/change, equipment
  drag/drop hotspots, military convert/expand, and naval carrier-air-wing,
  deploy-location, name-list, and expand controls, with unchanged
  selected row/scroll/bind digest and neighboring row-body click protection.
  Secondary/status filter checkboxes now have geometry-anchored disabled no-op
  proof so clicks cannot bleed into production row selection; true status
  filtering and disabled visual treatment remain open until per-line status
  flags are available. Line detail drilldowns, scrollbar art, hidden
  variant/obsolete/refit-template controls, and side-by-side proof remain open.
- `logistics.unit_view` now has executable division-list and equipment-pane
  wheel-scroll/thumb-drag slices: tests prove measured `div_id` division rows
  and selected division equipment rows change/clamp visible windows, captured
  thumb drags continue outside the pane until pointer-up, track clicks are
  consumed without selecting underlying rows, stale drag captures clear on
  rebind when geometry disappears, equipment wheel/thumb input does not move the
  division list, equipment scroll resets on explicit division selection changes,
  and both row sets survive scrub A->B->A without carrying B-only rows. Exact
  scrollbar art and broader unit-detail parity remain open.
- `chrome.minimap` now has a first executable click/drag recenter slice: tests
  prove inside-fitted-map clicks and drags emit `RecenterMap`, applying the
  command changes the bounded map-camera offset, pointer-up clears drag capture
  state, letterbox clicks are explicit no-ops, outside clicks fall through, and
  no recenter is emitted without a live camera. Minimap screenshot proof and
  real-game side-by-side proof remain open.
- `chrome.mapmodes` now has a first executable rail-click slice: tests prove
  real laid-out `mapmode_button_*` slots emit `SetMapMode`, applying that
  command changes the core map digest/mode label, unavailable/data-proxy badges
  remain honest, outside clicks are ignored, and the active-state echo syncs
  from the seeded `core.map` view. The real `mapmodes_all` button opens the
  extracted all-modes popup and its Supply/Intel rows emit read-only
  `SetMapMode`. Custom slot configuration/defaults, legend hover, screenshots,
  and broader map/chrome parity remain open.
- `chrome.mapmodes` now has a first executable rail-hover slice: tests prove
  real row hover emits a factual tooltip with active state, fidelity/source
  text, and clear on exit; popup rows use the same factual tooltip path. Full
  the game tooltip frames, map legend hover, screenshots, and broader tooltip parity
  remain open.
- `core.map` now has a first executable province-hover slice: shell hover
  refresh uses the same exposed-map/host-miss rule as map picking, and
  `MapView` tests prove factual province/state/owner/raster tooltip rows while
  unresolved land metadata stays unavailable. Full the game map tooltip frames,
  per-mode legend hover, map-icon hover, screenshots, and side-by-side proof
  remain open.
- `logistics.unit_view` now has a first executable stats-panel tab slice:
  tests click Stats/History/Template, prove unchanged-tab clicks consume without
  redraw, prove the selected tab survives scrub A->B->A, and prove non-stats
  tabs hide the stats equipment body without fabricating rows. Template/history
  data binding and broader unit-detail parity remain open.
- `research.research` now has executable active-slot row selection proof:
  tests prove clicks select the exact in-progress tech token, empty/outside
  slots are inert, missing selected techs fail closed without row fallback, and
  selected-slot state survives scrub A->B->A without carrying B-only rows.
  Slot detail drilldowns, ETA/progress parity, mutating research controls, and
  side-by-side proof remain open.
- `research.tech_tree` now has executable factual hover-tooltip proof for tech
  nodes: tests prove ID, folder, state, and research cost rows, and missing/null
  cost is badged unavailable instead of fabricated as zero. Full the game tooltip
  frames, prerequisites, localized domain composers, and side-by-side proof
  remain open.
- `research.tech_tree` now has executable tech-node click-selection proof:
  tests prove exact catalog-id selection, drag-pan conflict protection, missing
  selected tech fail-closed behavior, selected-node A->B->A scrub rebuild, and
  independently pickable common completed/in-progress nodes after folder-grid /
  sub-tech / duplicate-slot layout disambiguation. The selected node now also
  produces a factual read-only detail panel/digest slice for ID, folder, state,
  and research cost, with missing/null cost badged unavailable instead of
  fabricated as zero. Full the game detail-window parity, exact folder-host
  geometry, mutation-control inertness, and side-by-side proof remain open.
- Current statuses are conservative scaffold accounting: `open` for the
  descriptor-derived open/close gap, `open` for known tab target inventories
  and partial proven slices, and `not_proven` where no executable target
  inventory exists. They are not full interaction-parity claims.

Current executable report totals:

| Count | Value |
|---|---:|
| Registered views | 114 |
| Interaction contracts | 114 |
| Category rows | 1,140 |
| Target rows | 123 |
| Target rows with executable-proof notes | 123 |
| Implemented/proven target rows | Not representable yet |
| Intentionally inert mutation rows | 20 |
| `not_proven` category rows | 965 |

Per-category report totals:

| Category | Category `open` | Category `not_proven` | Target rows | Target `open` | Target `not_proven` | Proof-note targets |
|---|---:|---:|---:|---:|---:|---:|
| `open_close` | 114 | 0 | 0 | 0 | 0 | 0 |
| `tab_switch` | 6 | 108 | 18 | 18 | 0 | 18 |
| `list_selection` | 18 | 96 | 39 | 39 | 0 | 39 |
| `text_input` | 1 | 113 | 1 | 1 | 0 | 1 |
| `scroll` | 9 | 105 | 13 | 13 | 0 | 13 |
| `hover_tooltip` | 12 | 102 | 12 | 12 | 0 | 12 |
| `drilldown` | 3 | 111 | 20 | 20 | 0 | 20 |
| `disabled_mutation` | 13 | 101 | 20 | 20 | 0 | 20 |
| `scrub_state` | 0 | 114 | 0 | 0 | 0 | 0 |
| `screenshot_proof` | 0 | 114 | 0 | 0 | 0 | 0 |

## Registry Surface

Registry evidence comes from the distributed `register_view!` descriptors plus
the generated scripted-overlay table. Some module headers are stale: leaders
mentions 8 surfaces including a template-only library but registers 7 views;
logistics says 12 but registers/tests 14; intel says 11 but registers 12 after
the late diplomacy view. The surface below uses the registry as source of truth.

| Cluster | Registered views | View ids |
|---|---:|---|
| Core | 5 | `core.map`, `core.topbar`, `core.state_panel`, `core.focus_tree`, `core.land_combat` |
| PoliticsGovernment | 5 | `politics.politics`, `politics.power_balance`, `politics.command_character`, `politics.advisor_role_selection`, `politics.occupation` |
| ProductionConstructionDecisions | 3 | `production.production`, `production.construction`, `production.decisions` |
| ResearchTechDoctrine | 3 | `research.tech_tree`, `research.doctrine_tree`, `research.research` |
| MilitaryManagement | 15 | `military.army`, `military.navies`, `military.air_group`, `military.air_selection`, `military.air_wing_details`, `military.air_wing_reorg`, `military.carrier_air_wing`, `military.strategic_air`, `military.battle_plan_tools`, `military.ability_list`, `military.theatre_selector`, `military.divisions_summary`, `military.railway_gun`, `military.deployment`, `military.equipment_overview` |
| Designers | 5 | `designers.tank`, `designers.plane`, `designers.division`, `designers.equipment`, `designers.equipment_upgrade` |
| LeadersOfficersTraits | 7 | `leaders.officer_corps`, `leaders.unit_leader`, `leaders.navy_leader`, `leaders.leader_groups`, `leaders.aces`, `leaders.army_trait_tree`, `leaders.navy_trait_tree` |
| IntelDiplomacyTrade | 12 | `intel.intel_agency`, `intel.intel_ledger`, `intel.operative`, `intel.operative_leader`, `intel.operation_overview`, `intel.trade`, `intel.peace_conference`, `intel.world_tension_popup`, `intel.war_overview`, `intel.insignia_selection`, `intel.ins_revolution`, `intel.diplomacy` |
| LogisticsCombatUnitsEvents | 14 | `logistics.logistics`, `logistics.garrison_log`, `logistics.naval_combat_results`, `logistics.destroyed_ships`, `logistics.naval_losses`, `logistics.naval_strategic_region`, `logistics.unit_view`, `logistics.event_window`, `logistics.popup_window`, `logistics.detailed_windows`, `logistics.core`, `logistics.ingame_menu`, `logistics.naval_combat`, `logistics.combat_log` |
| ChromeOutlinerMaps | 7 | `chrome.clock`, `chrome.minimap`, `chrome.alerts`, `chrome.find`, `chrome.rightclick`, `chrome.mapmodes`, `chrome.mapicons` |
| ScriptedGuiOverlays | 38 | `overlay.horsttool`, `overlay.topbarresources`, `overlay.topbarvp`, `overlay.vp_scripted_gui`, `overlay.naval_scripted_gui`, `overlay.SWOA_focus_background`, `overlay.game_check`, `overlay.com_propaganda`, `overlay.eng_propaganda`, `overlay.fra_propaganda`, `overlay.ger_propaganda`, `overlay.ita_propaganda`, `overlay.jap_propaganda`, `overlay.usa_propaganda`, `overlay.non_major_propaganda`, `overlay.ger_inner_circle`, `overlay.ger_inner_circle_hidden`, `overlay.ger_rk`, `overlay.ger_monroe`, `overlay.jap_imperial_influence`, `overlay.sov_paranoia`, `overlay.sov_propaganda`, `overlay.usa_congress`, `overlay.swe_rearmament`, `overlay.swe_sheltered`, `overlay.ast_cabinet_trust`, `overlay.ast_right_vs_left`, `overlay.ast_right_vs_left_empty`, `overlay.aus_antischluss`, `overlay.bul_factions`, `overlay.gre_factions`, `overlay.cze_industry`, `overlay.sia_khana_ratsadon`, `overlay.sia_movie_theater`, `overlay.raj_famine`, `overlay.raj_eic_tax_fraud`, `overlay.raj_local_leaders`, `overlay.war_escalation` |
| Total | 114 | First-pass source inventory; not interaction-complete. |

## Interaction Contract Categories

Every view-level contract should name each interactive target, its source
element or widget id, current behavior, replay-expected behavior, state carried
across scrub, and proof.

- [x] Open from menubar for routed views: `Menubar` builds from
  `ViewRegistry::menubar()` and calls `WindowHost::open_view`.
- [x] Open from core shortcuts for seeded core surfaces: `F` focus, `C` combat,
  `M` or brackets for map mode, space/arrow keys for transport.
- [x] Generic singleton open/focus and z-order are implemented by
  `WindowHost::open_instance`, `sort_z`, and modal/front-closeable handling.
- [x] Generic close exists for `Esc` on front closeable windows and for captured
  leaf names `close_button`, `close`, `exit`, and `X`.
- [ ] Per-view close contract: verify every registered view has either a real
  clickable close element, `Esc` close, or intentional always-on/no-close status.
- [x] First executable tab target inventory: known N2 tab ids/labels are
  recorded for `politics.politics`, `military.army`, `designers.division`,
  `intel.intel_agency`, `chrome.find`, and `logistics.unit_view`; tests prove `chrome.find`,
  `military.army`, `politics.politics`, `designers.division`, and
  `intel.intel_agency` are real registered views with non-empty target rows,
  plus `logistics.unit_view` for the stats-panel tab row.
- [x] First executable read-only tab switches: `chrome.find` clicks switch the
  selected States/Countries/Victory Points catalog, `military.army` clicks
  switch the selected Army/Navy/Air roster, `politics.politics` clicks switch
  Government/National Spirits/Ideas & Laws/Autonomy content,
  `designers.division` switches Division/Army HQ where the focused template
  allows it, `intel.intel_agency` switches Agency/Operations/Cryptology, and
  `logistics.unit_view` switches Stats/History/Template on the unit stats panel.
  These slices have inert non-pickable or unavailable tab proof and
  selected-tab scrub A->B->A proof for their tested views.
- [ ] Tab switching contract: every N2 tab bar must specify enabled, disabled,
  unavailable, selected, click target, persistent selected tab, and scrub
  behavior. `chrome.find`, `military.army`, `politics.politics`,
  `designers.division`, `intel.intel_agency`, and `logistics.unit_view` have
  proof for their slices; the remaining N2 tab inventories remain open.
- [ ] List/grid selection contract: row factories render rows, but each row set
  still needs selectable rows, selected-row visual state, detail-pane update rules,
  and disabled-row handling. `chrome.find` has a first catalog-row
  selection/highlight proof, `military.army` has keyed Army/Navy/Air roster
  row selection/highlight proof, `politics.politics` has stable-keyed active-tab
  row selection/highlight proof, and `research.research` has active-slot row
  selection proof.
- [ ] Text-input contract: text fields/edit boxes are not generally wired or
  focus-safe. `chrome.find` has the first slice: the real `to_find_text` rect
  focuses, typed text/backspace filter honest catalog labels/keys, old scroll is
  reset/clamped, focused text consumes global shortcuts, and query state survives
  scrub A->B->A.
- [ ] Scroll contract: row factories render scrollbar geometry, but drag/wheel
  scrolling with hit clipping is not generally wired in cluster `on_input`.
  `chrome.find` has wheel-scroll/thumb-drag for its tested row catalog, while
  `military.army` has Army/Navy/Air wheel-scroll/thumb-drag and
  `politics.politics` has long National Spirits plus Ideas & Laws row
  wheel-scroll/thumb-drag, and
  `logistics.unit_view` has wheel-scroll/thumb-drag for both the division list
  and selected-division equipment pane. Exact scrollbar art and other scroll
  surfaces remain open.
- [x] Map pan/zoom: wheel zoom and right-drag pan are implemented in the main
  event loop when no higher view consumes input.
- [x] Map pick: left-click on a map miss resolves a province/state and opens
  `core.state_panel`; panel block rect prevents map picks behind panels.
- [ ] Map hover/readout contract: province/state hover, map-mode legend hover,
  map icon hover, and hover-to-tooltip are not globally proven.
- [ ] Minimap pick/recenter contract: N7 minimap renders a viewport panel and
  has executable click-and-drag recenter proof; screenshot proof and
  side-by-side proof remain open.
- [x] Graph pan/zoom for focus, tech tree, and doctrine tree: direct
  `on_input` implementations consume drag/wheel, and tests show pan changes
  digest while no-refit rebind preserves camera.
- [ ] Graph node drilldown contract: focus/tech/doctrine nodes and the current
  leaders army/navy N3 owned-trait-token nodes now have click selection and
  first factual selected-detail digest/panel proof; tech/doctrine and surfaced
  army/navy owned-trait-token nodes also have factual hover target proof.
  Trait-node proof is intentionally limited to the surfaced owned tokens from
  the selected leader; full static trait catalog graph, disabled/pickable
  states, trait icons/effect text, and full the game detail-window/tooltip parity
  remain open.
- [x] Combat bubble drilldown: `core.land_combat` bubble click opens the
  read-only combat panel; panel close and `Esc` close the panel state.
- [ ] General read-only drilldown contract: country flag, state/province, unit,
  army, fleet, air wing, combat row, focus, tech, doctrine, event, decision, war,
  trade, and logistics row drilldowns are not globally implemented/proven.
  `chrome.find` state and Victory Point rows have a first read-only
  `core.state_panel` drill command proof, and Find/Goto Capital buttons have a
  first read-only `RecenterMap` proof for selected or first filtered catalog
  rows and measured capital VP geometry.
- [x] Mutating controls are often classified as inert in cluster docs and generic
  captured non-close controls do nothing.
- [ ] Disabled mutation contract: every mutating the game control needs a target
  inventory, visual disabled/inert state, click-no-op test, and side-by-side proof
  that it is visibly read-only rather than absent or misleading.
- [x] Scrub rebind: `WindowHost::rebind_all` rebinds every open view on seek;
  clusters have A-B-A digest tests for render state, and a host-level multiple
  window scrub test exists.
- [ ] Scrub interaction-state contract: once tabs, row selections, scroll offsets,
  and drilldown selections exist, tests must prove scrub changes data without
  dropping or fabricating local UI state.
- [ ] Screenshot proof: run the `--shot-view=<view_id>` app hook across every
  registered view plus key tabs, rows, tooltips, and map modes.
- [x] Screenshot harness plumbing: `--shot-all-views=<dir>` writes a
  registry-derived manifest of planned native/HOI4/diff per-view screenshot
  targets and native shot args. It does not emit images or prove visual parity.
- [ ] Side-by-side proof: capture matching the game screenshots/interactions and
  review differences before any product parity claim.

## Cluster Inventory

### Core

Views: `core.map`, `core.topbar`, `core.state_panel`, `core.focus_tree`,
`core.land_combat`.

Implemented interactions:

- [x] `core.map`: pan by right-drag, zoom by wheel, map-mode cycle by key,
  province/state pick on click miss, factual exposed-province hover tooltip,
  and dynamic scrub-driven map recolor.
- [x] `core.state_panel`: opens from map pick with selected state id; blocks map
  clicks inside panel; close leaf names are handled generically; building-grid
  wheel/thumb scroll is tested with clamp, track-click-inert,
  outside-grid-inert, drag-clear, and scrub-clamp proof. A shell-level test now
  proves the captured `close_button` leaf closes the state panel through the
  real host/shell close path. A geometry test proves the resource grid does not
  need scroll for the six known state resources. Passive factual owner-flag,
  scroll-aware building-row, and resource-row hover tooltips are digest-visible
  and fail closed when the hovered backing row disappears. Occupation
  law/template and scorched-earth state buttons have replay-disabled no-op proof.
- [x] `core.focus_tree`: `F` toggles modal overlay; drag pan, wheel zoom, hover
  tooltip, and `Esc` close are implemented.
- [x] `core.focus_tree`: focus-node click selection is executable; tests prove
  exact focus-id keyed selection/highlight, drag-pan conflict protection,
  missing selected focus fail-closed behavior without node fallback,
  selected-node A->B->A detail rebuild, and a factual selected-detail
  panel/digest slice for ID, state, cost, current progress days, prereq group
  count, and position fidelity with missing cost/progress unavailable rather
  than fabricated as zero.
- [x] `core.land_combat`: `C` toggles overlay; bubble click opens combat panel;
  panel close and `Esc` close panel state; the open panel blocks map hover/pick
  behind it, while off-panel click misses fall through to map.
- [x] `core.topbar`: always-on chrome is registered and scrub-bound; real
  topbar navigation buttons now emit read-only `OpenView` commands for the
  existing replay views while empty chrome still falls through. Pause/date and
  speed-control buttons have replay-disabled no-op proof and hidden-target
  arming protection under higher-z panels.

Likely missing interactions:

- [ ] `core.map`: full the game map tooltip frame parity, map-mode selector
  clicking, legend hover, map icon hover/selection, and side-by-side map
  interaction proof.
- [ ] `core.state_panel`: province/state drilldown links, full the game tooltip
  frame/hover chrome coverage beyond the factual first slice, exact scrollbar
  art, native shared-slot row-factory rendering plus remove-button inert proof,
  disabled visual treatment, and side-by-side proof.
- [ ] `core.topbar`: faction/alerts/threat drilldowns, hover frames, topbar
  tooltips, disabled read-only behavior for remaining game-action buttons,
  disabled visual treatment, screenshots, and side-by-side proof.

Tests/proof needed:

- [ ] Headless input tests for map pick vs panel block rect using current host.
- [ ] Screenshot cases for every map mode and state-panel hover/close state.
- [ ] Real the game side-by-side for map pick, state panel, focus hover, and combat
  bubble panel.

### PoliticsGovernment

Views: `politics.politics`, `politics.power_balance`,
`politics.command_character`, `politics.advisor_role_selection`,
`politics.occupation`.

Implemented interactions:

- [x] Menubar open and generic close path.
- [x] `politics.politics` read-only subtabs switch via click for Government,
  National Spirits, Ideas & Laws, and Autonomy; tests prove digest-visible
  content changes, unavailable tab clicks are inert, and selected tab state
  survives scrub A->B->A without carrying B-only rows.
- [x] `politics.politics` active-tab row selection and scroll are executable:
  tests prove stable-keyed Government/National Spirits/Ideas & Laws/Autonomy
  row selection/highlight, duplicate/unkeyed row inert behavior, scrolled
  backing-row selection, and long National Spirits plus Ideas & Laws
  wheel-scroll and scrollbar-thumb drag clamping. They also prove captured thumb
  drags continuing outside the list until pointer-up, track-click inertness,
  per-tab scroll isolation, and scrub A->B->A without carrying B-only rows.
- [x] Data-bound rows and widgets render: politics tabbed subwindows, power
  balance track, character roster, advisor rows, occupied country/state rows.
- [x] Mutating verbs are documented as inert: create/switch faction, change law,
  assign garrison, switch character.

Likely missing interactions:

- [ ] Character/advisor/law/idea/occupied-row drilldowns and detail-pane updates
  outside the active `politics.politics` tab rows.
- [ ] Exact scrollbar art plus scroll coverage for character, advisor, modal,
  and occupation lists outside the proven `politics.politics` active-tab rows.
- [ ] Hover tooltips for parties, ideas, laws, power-balance markers, character
  traits, advisor roles, resistance/compliance, and garrison/law gaps.
- [ ] Explicit disabled-state inventory for every mutating government control.

Tests/proof needed:

- [ ] Per-view interaction tests for remaining row clicks, scroll clipping, and
  inert mutating controls outside the proven `politics.politics` slice.
- [ ] Screenshots for selected rows, disabled laws/advisors, hover frames, and
  unavailable badges.

### ProductionConstructionDecisions

Views: `production.production`, `production.construction`,
`production.decisions`.

Implemented interactions:

- [x] Menubar open and generic close path.
- [x] Production, construction, and timed-decision row factories render with
  no-fabrication display atoms and scrollbar geometry.
- [x] `production.production` production-line row click selection is executable
  for measured line identities: live rows use `id`, save-backed rows use
  `line_id`, military/naval line identity is preserved, selected rows rebuild
  across scrub, rows without measured identity or with duplicate ids are inert,
  and missing selected rows fail closed.
- [x] `production.production` production-line wheel scroll and scrollbar-thumb
  drag are executable: visible rows change and clamp, captured thumb drags keep
  scrolling outside the grid until pointer-up, track clicks do not select
  underlying rows, scrolled clicks select the correct backing production key,
  scroll+selection survive A->B->A without carrying B-only rows, and outside-grid
  wheel input is inert.
- [x] `production.production` primary category filters are executable:
  `show_equipment`/`show_armor`/`show_air`/`show_capital_ships`/
  `show_screen_ships`/`show_other_ships` toggle replay-local row visibility with
  no commands. Tests prove categories resolve through `game_defs/out/catalogs`
  `equipment_archetypes`, follow `archetype` links, use a unique
  live-id-to-save-`line_id` join when live rows lack `archetype`, keep unknown or
  duplicate-join rows visible, and keep filtered row clicks aligned with the
  visible backing rows after rebind.
- [x] `production.production` visible line close buttons are replay-disabled
  no-ops: tests prove `close_production_line` pointer-down/up on a visible
  scrolled row consumes with no commands, no redraw, and unchanged selected
  row/scroll/bind digest, while neighboring row-body clicks still select the
  backing production row.
- [x] `production.production` visible factory add/sub buttons are
  replay-disabled no-ops: the extracted production GUI now preserves the
  `@prod_*` define table, tests prove the row `factories_sub_1x/5x/10x` and
  `factories_add_1x/5x/10x` hit areas consume pointer-down/up with no commands,
  no redraw, and unchanged selected row/scroll/bind digest, while neighboring
  row-body clicks still select the backing production row.
- [x] `production.production` visible priority/amount buttons are
  replay-disabled no-ops: tests prove `increase_priority`/`decrease_priority`,
  military `btn_x10`/`btn_x5`, and naval `increase_amount`/`decrease_amount`
  consume pointer-down/up with no commands, no redraw, and unchanged selected
  row/scroll/bind digest, while neighboring row-body clicks still select the
  backing production row.
- [x] `production.production` visible root add-line buttons are replay-disabled
  no-ops: tests prove `infantry_button`/`tank_button`/`aircraft_button`/
  `naval_button`/`naval_repair_button` consume pointer-down/up with no commands,
  no redraw, and unchanged selected row/scroll/visible rows/bind digest, while
  empty space between aircraft and naval groups remains unclaimed.
- [x] `production.decisions` timed-decision row click selection is executable for
  measured decision/state rows: duplicate decision ids with different target
  states are safe, selected rows rebuild across scrub, rows without measured
  decision id are inert, and missing selected rows fail closed.
- [x] `production.decisions` timed-decision list wheel scroll and
  scrollbar-thumb drag are executable: visible rows change and clamp, captured
  thumb drags keep scrolling outside the grid until pointer-up, track clicks do
  not select underlying rows, scrolled clicks select the correct backing
  decision key, scroll+selection survive A->B->A without carrying B-only rows,
  and outside-grid wheel input is inert.
- [x] `production.decisions` visible timed-decision `btn_select` actions are
  replay-disabled no-ops: tests prove pointer-down/up consumes with no commands,
  no redraw, and unchanged selected row/scroll/bind digest, while neighboring
  row-body clicks still select the backing timed decision and scrolled visible
  button hits resolve without selecting that row.
- [x] `production.construction` construction-queue row click selection is
  executable for measured queue identities: live rows use `line_id`, save-backed
  rows use `template`/`location_state`/`created_date`, selected rows rebuild
  across scrub, rows without measured identity are inert, and missing selected
  rows fail closed.
- [x] `production.construction` construction-queue wheel scroll and
  scrollbar-thumb drag are executable: visible rows change and clamp, captured
  thumb drags keep scrolling outside the grid until pointer-up, track clicks do
  not select underlying rows, scrolled clicks select the correct backing
  construction key, scroll+selection survive A->B->A without carrying B-only
  rows, and outside-grid wheel input is inert.
- [x] `production.construction` top count-container factory add/sub buttons are
  replay-disabled no-ops: tests prove the six real GUI variants
  (`factories_sub_1x/5x/10x`, `factories_add_1x/5x/10x`) collapse to two
  physical hit areas that consume pointer-down/up with no commands, no redraw,
  and unchanged selected row/scroll/bind digest, while neighboring
  count-container clicks remain unclaimed.
- [x] `production.construction` rendered construction-queue priority/amount/close
  buttons are replay-disabled no-ops: tests prove
  `production_building_line_entry` `increase_priority`/`decrease_priority`,
  `increase_amount`/`decrease_amount`, and `close_production_line` consume
  pointer-down/up with no commands, no redraw, and unchanged selected
  row/scroll/bind digest, while neighboring row-body clicks still select the
  backing construction row. `production_railway_gun_repair_entry`,
  `static_contract_payment_entry.open_market_view_btn`, and root/debug buttons
  are not rendered by the current queue adapter and remain separate mutation
  surfaces.

Likely missing interactions:

- [ ] Broader construction building drilldowns, production-line detail
  drilldowns, decision detail drilldowns, and one-shot availability remain open.
- [ ] Scrollbar art for production lines, timed decisions, and construction
  queue.
- [ ] Read-only drilldown from rows to equipment/building/decision details.
- [x] Secondary production filter hit-testing now consumes the real
  `show_resources`/`show_outdated`/`show_stockpiling`/`show_reinforcing`/
  `show_upgrading`/`show_refitting` checkbox rects as replay-disabled no-ops
  with unchanged selected row/scroll/bind digest.
- [ ] True secondary production filtering still needs backing per-line resource
  shortage/outdated/stockpiling/reinforcing/upgrading/refitting flags or a
  faithful recompute, plus disabled visual treatment.
- [ ] Disabled/inert proof for remaining hidden production variant/obsolete,
  unrendered refit-template controls, construction building queue edits,
  build-picker actions,
  market/debug controls, one-shot decision actions, and broader decision
  mutation controls beyond timed-decision `btn_select`.
- [ ] Tooltips for resources, efficiency, factory allocation, queue progress,
  decision timers, and unavailable decision-status gaps.

Tests/proof needed:

- [ ] List selection and scroll tests with clipped rows.
- [ ] Click-no-op tests for remaining mutating production/construction/decision
  buttons beyond the proven timed-decision `btn_select` path.
- [ ] Side-by-side screenshots for a long production list and active timed
  decision.

### ResearchTechDoctrine

Views: `research.tech_tree`, `research.doctrine_tree`, `research.research`.

Implemented interactions:

- [x] `research.tech_tree`: graph drag pan, wheel zoom, `Esc` close, camera
  preservation on no-refit scrub.
- [x] `research.tech_tree`: tech-node click selection is executable; tests
  prove exact catalog-id keyed selection/highlight, drag-pan conflict
  protection, missing selected tech fail-closed behavior, selected-node
  A->B->A scrub rebuild, and independently pickable common completed/in-progress
  nodes after folder-grid/sub-tech/duplicate-slot layout disambiguation.
- [x] `research.tech_tree`: selected tech nodes emit a factual read-only
  selected-detail panel/digest slice for ID, folder, state, and research cost;
  tests prove missing/null cost remains unavailable rather than fabricated as
  zero.
- [x] `research.tech_tree`: factual tech-node hover tooltip emits ID, folder,
  state, and research-cost rows, with missing/null cost badged unavailable.
- [x] `research.doctrine_tree`: graph drag pan, wheel zoom, `Esc` close, camera
  preservation on no-refit scrub.
- [x] `research.doctrine_tree`: doctrine-node click selection is executable;
  tests prove exact catalog-id keyed selection/highlight, drag-pan conflict
  protection, missing selected doctrine fail-closed behavior without node
  fallback, and selected-node A->B->A scrub rebuild while still treating
  parser-unfed doctrine state as unavailable unless exact doctrine IDs are fed.
- [x] `research.doctrine_tree`: selected doctrine nodes emit a factual
  read-only selected-detail panel/digest slice for ID, folder,
  exact-when-fed/unfed-badged state, and full static XOR-link count; tests prove
  parser-unfed doctrine state is not fabricated as unresearched.
- [x] `research.doctrine_tree`: factual doctrine-node hover tooltip emits ID,
  folder, exact-when-fed state, unfed-badged parser-unfed state, and full static
  XOR-link count rows without fabricating absence as unresearched.
- [x] `research.research`: active research slot panel renders real `.gui` chrome
  and slot rows.
- [x] `research.research`: active research slot row selection is executable;
  tests prove exact tech-token keyed selection/highlight, empty/outside slot
  inert behavior, missing selected tech fail-closed behavior without row
  fallback, and scrub A->B->A without carrying B-only rows.

Likely missing interactions:

- [ ] Full the game tech/doctrine detail-window parity: game chrome, icon/stat
  grids, effects, prerequisites, MIO/design-team affordances, XP unlock/research
  affordances, progress/ETA, and comparison styling.
- [ ] Full tech/doctrine node hover tooltip parity with localized cost/effects,
  prerequisites, ahead-of-time, bonuses, unavailable computed lines, and game
  tooltip frame/comparison styling.
- [ ] Research slot detail, cancel/research mutation buttons rendered
  disabled/inert, and exact ETA/progress parity.
- [ ] Tree category/folder/tab switching if present in the game surface.

Tests/proof needed:

- [ ] Broader tooltip snapshot tests and full detail-window scrub/screenshot
  tests beyond the first selected-detail digest slice.
- [ ] Screenshot proof for pan/zoom, hovered node, selected node, and research
  slot rows against HOI4.

### MilitaryManagement

Views: `military.army`, `military.navies`, `military.air_group`,
`military.air_selection`, `military.air_wing_details`,
`military.air_wing_reorg`, `military.carrier_air_wing`,
`military.strategic_air`, `military.battle_plan_tools`,
`military.ability_list`, `military.theatre_selector`,
`military.divisions_summary`, `military.railway_gun`,
`military.deployment`, `military.equipment_overview`.

Implemented interactions:

- [x] Menubar open and generic close path for all 15.
- [x] Army view renders N2 army/navy/air tabs and row factories for OOB lists.
- [x] `military.army` has executable read-only Army/Navy/Air tab switching:
  left-click changes the selected roster and digest-visible rows, unavailable
  tabs are explicit inert no-ops, and selected-tab state survives scrub A->B->A
  without carrying B-only rows.
- [x] `military.army` has executable keyed Army/Navy/Air roster row selection:
  row clicks select the measured backing row, the selected row renders a
  read-only highlight, scrolled clicks target the correct backing row, and
  selected-row state survives scrub A->B->A without carrying B-only rows.
- [x] `military.army` has executable Army/Navy/Air roster wheel-scroll and
  scrollbar-thumb drag: wheel input and thumb drags change/clamp the visible row
  window, captured drags continue outside the list until pointer-up, track clicks
  do not select underlying rows, outside-list wheel input is inert, and per-tab
  scroll state survives scrub A->B->A without carrying B-only rows.
- [x] `military.theatre_selector` row click selection, wheel scroll, and
  scrollbar-thumb drag are executable for measured theatre ids: tests prove
  selection/highlight, scrolled row clicks, A->B->A row rebuilds,
  missing/duplicate id fail-closed behavior, missing selected-row fail-closed
  behavior, outside-list wheel inertness, captured thumb drag, and inert track
  clicks.
- [x] Fleet, task force, air group, air selection, deployment, divisions,
  railway gun, and equipment overview rows render through registered templates.
- [x] Mutating verbs such as create army, deploy, draw order, reorg, and ability
  use are documented as inert/unavailable.

Likely missing interactions:

- [ ] Detail-pane drilldowns for selected army/fleet/air-wing rows and row
  selection in the other military views.
- [ ] Scrollbar drag/art plus scroll coverage for other military lists,
  deployment, and equipment lists.
- [ ] Battle-plan tools must be visibly disabled/inert and must not imply orders
  can be drawn in replay.
- [ ] Theatre group/member drilldowns, alerts, pin/priority UI state, scrollbar
  art, air-wing reorg, carrier air wing, and ability list need explicit
  read-only/mutation target inventories.
- [ ] Hover tooltips for units, missions, equipment, templates, commanders,
  supply/attrition, orders, and unavailable computed fields.

Tests/proof needed:

- [x] `military.army` roster row pick/scroll tests for the Army/Navy/Air slice.
- [ ] Per-view row pick/scroll tests for the remaining military views.
- [ ] Mutation click-no-op tests for order/deploy/reorg/ability controls.
- [ ] Side-by-side proof for selected army/fleet/air-wing/detail workflows.

### Designers

Views: `designers.tank`, `designers.plane`, `designers.division`,
`designers.equipment`, `designers.equipment_upgrade`.

Implemented interactions:

- [x] Menubar open and generic close path.
- [x] N4 module-slot layout renders equipment/variant/template data.
- [x] Tank, plane, and equipment designer module slots now have executable
  read-only click-selection proof and passive hover-inspection proof over the
  shared component slot grid. Tests prove no commands are emitted, selected or
  hovered slot state is digest-visible, repeated slot clicks/hover avoid
  unnecessary redraw, and selected slot state survives save->missing->save scrub
  without falling back to another slot.
- [x] Tank, plane, and equipment designer shared non-slot chrome/control areas
  have executable replay-disabled no-op proof: pointer-down/up clicks are
  consumed with no commands, no redraw, and unchanged bind digest.
- [x] Division designer renders the N2 normal/HQ tab bar visually.
- [x] Division designer normal/HQ tabs switch via read-only click when the
  focused template is an Army HQ template; non-HQ Army HQ clicks are inert, and
  selected-tab state has scrub A->B->A proof.
- [x] Computed combat stats, costs, and other hard-limit panels are badged
  unavailable rather than fabricated.

Likely missing interactions:

- [ ] Full module-slot detail/tooltip parity is still open: current proof covers
  factual selected/hover inspection state, not the game tooltip frames,
  localized/effect-expanded text, stat-contribution expansion, or screenshots.
- [ ] Upgrade slider interaction must be disabled/inert but still visually
  accounted for.
- [ ] Variant/template selector row selection and drilldown are not proven.
- [ ] Module add/remove, save, duplicate, production, and upgrade-track controls
  outside the shared non-slot area still need per-control disabled visual/no-op
  accounting.
- [ ] Tooltips for modules, stat contributions, upgrade tracks, unavailable
  computed stats, and production-resource gaps beyond the factual slot
  inspection digest remain open.

Tests/proof needed:

- [x] Slot pick/hover tests for the shared tank/plane/equipment component-slot
  slice.
- [ ] Selected-slot screenshot cases.
- [ ] Per-control click-no-op tests for upgrade sliders, module add/remove
  buttons, save, duplicate, and production controls.
- [ ] the game side-by-side for each designer type.

### LeadersOfficersTraits

Views: `leaders.officer_corps`, `leaders.unit_leader`,
`leaders.navy_leader`, `leaders.leader_groups`, `leaders.aces`,
`leaders.army_trait_tree`, `leaders.navy_trait_tree`.

Implemented interactions:

- [x] Menubar open and generic close path.
- [x] Officer corps, leader rosters, leader groups, aces, and trait-tree windows
  render real `.gui` chrome and row/template data.
- [x] Trait tree widgets render selected/no-selected-leader state honestly.
- [x] Army/navy trait-tree owned trait-token nodes have read-only click
  selection/highlight proof, inert no-leader/no-traits glyph proof,
  drag-cancel proof, missing-selected fail-closed proof, A->B->A selected-detail
  rebuild proof, and factual selected-detail digest/panel rows for trait id,
  owned state, branch, leader name, leader key, plus derived static DEF rows for
  trait scope, cost, prereq count, mutual-exclusion count, modifier-row count,
  and source file; null DEF fields remain unavailable instead of fabricated.
- [x] Army trait-tree owned trait-token nodes have factual hover tooltip proof
  for trait id, owned state, branch, leader name, leader key, and the same
  derived static DEF rows, plus no-leader glyph hover inertness and hover-clear
  proof; navy owned trait nodes share the same factual tooltip path.
- [x] Army/navy trait-tree rendered `add_trait_0` through `add_trait_3`
  trait-pick slots have executable disabled-mutation proof: tests resolve the
  hit rectangles from the real rendered `.gui` roots and prove pointer-down/up
  clicks are consumed with no commands, no redraw, and unchanged selected
  trait/detail bind digest.

Likely missing interactions:

- [ ] Leader row selection must update selected leader detail and trait tree.
- [ ] Full the game trait tooltip/read-only drilldown details are not proven;
  current click/detail/hover proof covers only surfaced owned trait-token nodes
  and factual static DEF metadata rows, not localized/effect-expanded game
  tooltip frames.
- [ ] Full static trait catalog graph parity remains open: disabled/pickable
  states, real trait icons, effect text expansion, prereq/XOR lines, and DEF
  geometry.
- [ ] Remaining trait-pick parity is still open: availability/disabled visuals,
  localized/effect-expanded picker content, and non-add-trait controls such as
  tactic/wiki/tab buttons are not proven.
- [ ] Scrollbar drag/wheel for large leader, advisor, ace, and trait lists.
- [ ] Assignment/command group/portrait interactions need read-only drilldown or
  disabled-control classification.

Tests/proof needed:

- [ ] Row-to-detail tests for generals, admirals, aces, and command groups.
- [ ] Remaining trait picker availability/disabled-visual tests and tooltip
  snapshots.
- [ ] Side-by-side screenshots for selected leader and selected trait state.

### IntelDiplomacyTrade

Views: `intel.intel_agency`, `intel.intel_ledger`, `intel.operative`,
`intel.operative_leader`, `intel.operation_overview`, `intel.trade`,
`intel.peace_conference`, `intel.world_tension_popup`,
`intel.war_overview`, `intel.insignia_selection`,
`intel.ins_revolution`, `intel.diplomacy`.

Implemented interactions:

- [x] Menubar open and generic close path for all 12.
- [x] Intel agency renders N2 tab bar visually; operative, trade, war, world
  tension, and diplomacy rows render through row factories.
- [x] Intel agency Agency/Operations/Cryptology tabs switch via read-only click,
  unavailable tabs are inert, and selected-tab state has scrub A->B->A proof.
- [x] Insignia selection has tests proving menu-open does not bind an arbitrary
  formation and selected-formation binding is explicit when supplied.
- [x] Mutation verbs are documented as inert: start mission, launch operation,
  send trade, vote, pick insignia, and diplomatic actions.

Likely missing interactions:

- [ ] Operative roster selection and detail-card updates.
- [ ] Trade partner/resource row selection and country drilldowns.
- [ ] War row/participant selection, world tension contributor drilldowns, and
  diplomacy country flag/action drilldowns.
- [ ] Diplomatic action modal buttons must be visually disabled/inert and tested.
- [ ] Tooltip coverage for intel estimates, operations, trade, war score,
  casualties, world tension, diplomatic actions, and unavailable computed gaps.

Tests/proof needed:

- [ ] Per-row selection tests for operative, trade, war, and diplomacy rows.
- [ ] Click-no-op tests for trade/diplomacy/operation/peace/insignia mutation
  controls.
- [ ] Side-by-side screenshots for diplomacy and war overview selected states.

### LogisticsCombatUnitsEvents

Views: `logistics.logistics`, `logistics.garrison_log`,
`logistics.naval_combat_results`, `logistics.destroyed_ships`,
`logistics.naval_losses`, `logistics.naval_strategic_region`,
`logistics.unit_view`, `logistics.event_window`,
`logistics.popup_window`, `logistics.detailed_windows`,
`logistics.core`, `logistics.ingame_menu`, `logistics.naval_combat`,
`logistics.combat_log`.

Implemented interactions:

- [x] Menubar open and generic close path for all 14.
- [x] Logistics/resource rows, naval result widgets, unit view, event cards,
  popup cards, ingame menu, naval combat, and combat log surfaces render as
  registered views.
- [x] Ingame menu save/load/resign/quit verbs have executable disabled-mutation
  proof: clicks are consumed as replay-modal no-ops, emit no commands, request
  no redraw, and leave the bound digest unchanged. Popup mutation verbs and
  broader event-window mutation/modal behavior remain documented
  inert/unavailable but still need the same executable proof.
- [x] Event window displayed static event-definition preview option actions have
  executable disabled-mutation proof: clicks on the displayed N5 option action
  are consumed as replay-disabled no-ops, emit no commands, request no redraw,
  and leave the bound digest unchanged. Inside-root outside-option chrome clicks
  are now consumed as modal no-op input with no commands, no redraw, and unchanged
  digest; off-root clicks are not claimed by EventWindowView-specific routing and
  remain governed by the host modal policy. Fired-event historical choices and
  complete event modal parity remain open.
- [x] Popup window displayed generic popup-library action area has executable
  disabled-mutation proof: clicks on the first displayed popup root's real accept
  button area are consumed as replay-modal disabled no-ops, emit no commands,
  request no redraw, and leave the bound digest unchanged. Per-root popup button
  leaf routing, disabled visual treatment, scripted popup text, and broader
  popup mutation parity remain open.
- [x] `logistics.unit_view` division-row click selection is executable for
  measured `div_id` rows: the read-only equipment pane rebuilds from the selected
  current-hour division, duplicate template names are safe, missing/foreign
  selected divisions fail closed, and rows without measured `div_id` are inert.
- [x] `logistics.unit_view` division-list wheel-scroll and scrollbar-thumb drag
  are executable for measured `div_id` rows: visible rows change/clamp, captured
  thumb drags continue outside the list until pointer-up, track clicks do not
  select underlying rows, stale drag captures clear on rebind, scrolled clicks
  select the correct backing division, selection plus scroll survive A->B->A
  without carrying B-only rows, and wheel input outside the list is inert.
- [x] `logistics.unit_view` selected-division equipment-pane wheel-scroll and
  scrollbar-thumb drag are executable: visible equipment rows change/clamp,
  captured thumb drags continue outside the pane until pointer-up, track clicks
  are inert, the division list does not move, explicit division selection resets
  equipment scroll to the top, missing selected divisions fail closed with an
  empty equipment pane, and scroll survives A->B->A without carrying B-only rows.
- [x] `logistics.unit_view` stats-panel tabs are executable: Stats/History/
  Template clicks switch the digest-visible body, unchanged-tab clicks are
  consumed without redraw, selected-tab state survives A->B->A, and non-stats
  tabs hide the equipment body without fabricating rows.

Likely missing interactions:

- [ ] Remaining unit/detail interactions: ship rows, column sorts, exact
  scrollbar art, full division stat panes, unit-detail drilldowns, and broader row
  visual parity.
- [ ] Combat log row selection into combat details; naval combat/result row
  drilldown into ship/fleet details.
- [ ] Event option hover/disabled visual treatment, fired-event historical choice
  parity, complete event modal behavior, and per-root popup button hover/disabled/
  inert behavior.
- [ ] Scrollbar drag/wheel for combat logs, losses, logistics overview rows,
  non-division unit rows, and popup/event lists.
- [ ] Tooltips for resources, fuel, supply, losses, event options, popups,
  naval region data, and unavailable computed combat stats.
- [ ] `logistics.destroyed_ships` is an empty stub; its interaction contract must
  stay honest about no rows and no drilldown.

Tests/proof needed:

- [ ] Row selection and scroll tests across logistics, unit, combat, and event
  views.
- [ ] Click-no-op tests for remaining popup mutation controls plus per-button
  ingame-menu, event-option, and popup leaf routing and disabled visual
  treatment.
- [ ] Side-by-side screenshots for unit detail, combat log, naval result, and
  representative popup/event cards.

### ChromeOutlinerMaps

Views: `chrome.clock`, `chrome.minimap`, `chrome.alerts`, `chrome.find`,
`chrome.rightclick`, `chrome.mapmodes`, `chrome.mapicons`.

Implemented interactions:

- [x] Menubar open and generic close path.
- [x] Clock renders exact hour from `ctx.date`.
- [x] Minimap renders chrome plus viewport outline from the current map camera.
- [x] Minimap click-and-drag recenter emits read-only `RecenterMap` commands
  from fitted-map clicks/drags, clears drag capture on pointer-up, ignores
  outside clicks, consumes letterbox no-ops, and has bounded camera-offset
  tests.
- [x] Find renders N2 category tab/list visually with catalog rows.
- [x] Find States/Countries/Victory Points tabs switch the bound read-only
  catalog and preserve selected tab across scrub A->B->A.
- [x] Find catalog rows support read-only selection/highlight, ignore outside-row
  clicks, preserve valid selected-row state across scrub A->B->A, and state/VP
  rows emit `core.state_panel` drill commands.
- [x] Find query text input focuses the real `to_find_text` edit box, accepts
  replay-local typing/backspace, filters over honest catalog labels/keys, resets
  and clamps old scroll after filtering, and preserves query state across scrub
  A->B->A without carrying B-only rows.
- [x] Map modes rail renders all mode slots, badges per-mode data honesty, and
  echoes the active mode once synced from the seeded `core.map` view.
- [x] Real laid-out map-mode rail slots emit read-only `SetMapMode` commands;
  unavailable/data-proxy badges remain honest.
- [x] Map-mode rail hover emits a factual source/fidelity tooltip and clears
  when the pointer leaves the panel.
- [x] Map icons render placed exact/proxy icons where anchored and badge the rest.

Likely missing interactions:

- [ ] Minimap screenshot proof and side-by-side click/drag proof.
- [ ] Find localized names, country detail panels, province-level VP drilldown,
  exact scrollbar art, and screenshot proof.
- [ ] Custom map-mode slot configuration/defaults, full the game tooltip frame
  parity, legend hover, screenshot proof, and broader core map-mode parity.
- [ ] Alerts drilldown and alert hover tooltips.
- [ ] Right-click menu target-specific options and click-no-op proof for mutating
  orders/actions.
- [ ] Map icon hover/read-only drilldown for VP, capital, resistance, air base,
  combat, unit counter, and unavailable icon families.

Tests/proof needed:

- [x] Minimap click/drag recenter input tests after wiring.
- [x] Find row-selection/drilldown, query/filtering, results wheel/thumb-scroll, and
  Find/Goto Capital map-recenter tests.
- [ ] Find exact scrollbar art and screenshot proof.
- [x] Mapmode button click tests against `SetMapMode` command emission and core
  map digest/mode-label application.
- [ ] Side-by-side screenshots for minimap, map modes, right-click menu, and map
  icons.

### ScriptedGuiOverlays

Views: 38 `overlay.*` scripted GUI surfaces listed in the registry table.

Implemented interactions:

- [x] Menubar open and generic `Esc` close path.
- [x] Synthetic panel roots block the map so overlay interiors do not leak to
  province picking.
- [x] Overlay verdicts classify partially bindable, chrome-only, static
  decoration, and dev-tool/inert surfaces.
- [x] Value slots are either captured data or unavailable badges; no fabricated
  values are introduced.

Likely missing interactions:

- [ ] On-screen close controls for synthetic overlays, if expected by the real
  the game surface.
- [ ] Horsttool selected-state open path from a map/state context; menu-open has
  no selected state and should remain badged.
- [ ] Button-level disabled/inert inventory for dev-tool and mutation overlays.
- [ ] Hover tooltip and hover-frame coverage for overlay controls/slots.
- [ ] Side-by-side visual proof because these are synthetic render plans rather
  than faithful parsed `.gui` reconstructions.

Tests/proof needed:

- [ ] Per-overlay interaction contracts generated from `OverlaySpec` slots and
  any button/control metadata.
- [ ] Click-no-op tests for dev-tool/mutation buttons.
- [ ] Screenshot matrix for representative partial, chrome-only, static, and
  dev-tool overlays against real HOI4.

## Cross-Cutting Product-Parity Gaps

- [x] Build a first view-level interaction contract scaffold: registered view id,
  descriptor route context, ten category statuses, and registry coverage tests.
- [x] Build the first target-level contract slice: registered view id,
  target category, stable tab target id, label, status, and note for known N2
  `tab_switch` targets.
- [ ] Expand the interaction target inventory beyond the current partial slices:
  `.gui` leaf/widget target id, read-only behavior, mutation status,
  implemented status, and proof link for tabs, rows, text inputs, scrollbars,
  hover targets, drilldowns, and disabled mutation controls.
- [x] Add a registry-level interaction coverage report: registered view count,
  contract count, category-row count, target count, missing/not-proven category
  count, and per-category status/target counts are executable and tested. The
  implemented/proven target count remains explicitly not representable until the
  status model grows closed states; intentionally inert mutation targets are
  counted separately so click-no-op proof does not overclaim broad parity.
- [ ] Implement generic N2 tab hit testing/state before hand-writing per-view
  tab logic.
- [ ] Implement generic list/grid scroll and row selection primitives that row
  factories can reuse.
- [ ] Implement generic tooltip resolver/composer before per-domain tooltip
  expansion.
- [ ] Add explicit disabled visual state helpers for mutating controls so
  read-only replay behavior is visible, not just a no-op.
- [ ] Extend scrub tests to include persistent interaction state once tabs,
  selections, scroll offsets, and drilldowns exist.
- [x] Add registry-derived screenshot target manifest plumbing with
  `--shot-all-views=<dir>`; the manifest now plans native/HOI4/diff files and
  native shot args for all 114 registered views.
- [ ] Extend the screenshot harness beyond the manifest so it captures each
  registered view, selected tabs, representative rows, tooltips, map modes, and
  overlay states.
- [ ] Run side-by-side the game comparison for the first-pass view set and attach
  exceptions or wall classifications.

## Highest-Risk First-Pass Findings

1. Most cluster views are visually bound but have no view-specific `on_input`.
   That means tabs, rows, scrollbars, and most drilldowns should be treated as
   missing until implemented or proven through generic host behavior.
2. N2 tabs are now partially executable as target inventory rows, but click
   switching is not generally wired or tested.
3. Row factories render large lists and scrollbar art, but interactive scrolling,
   row selection, selected visual state, and detail-pane updates are not generally
   wired.
4. Tooltip parity is mostly open. Focus has a direct hover tooltip and
   mapmode rail rows have a factual source/fidelity tooltip slice; generic
   HOI4-style tooltip resolution/composition remains a checklist blocker.
5. Mutating controls are often documented as inert, but there is no per-control
   disabled-state inventory or click-no-op proof across the 114-view surface.
6. Map and chrome parity still lacks minimap screenshot/side-by-side proof,
   custom map-mode slot configuration/defaults, legend hover, map icon
   interactions, full screenshot harness coverage, and side-by-side real the game
   proof.
7. Scripted overlays are explicitly synthetic render plans, so their visual and
   interaction parity risk is higher than parsed `.gui` windows until compared
   side by side.
