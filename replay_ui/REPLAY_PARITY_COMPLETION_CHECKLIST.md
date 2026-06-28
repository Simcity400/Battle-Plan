# Replay Parity Completion Checklist

This checklist supersedes the narrower engineering DoD when the target is:

> A read-only Battle Plan app that can be used like the real game UI for inspection,
> with no ability to mutate the game.

The prior Definition of Done proves the architecture, registered-view coverage,
no-fabrication rules, and basic replay surfaces. It does not by itself prove
1-to-1 UI behavior, tooltip coverage, all read-only drilldowns, or hourly recorder
completeness.

## 0. Current Verdict

- [x] App engineering DoD reached: 114 registered views, menu-reachable, scrub-safe,
  no-fabrication tests green.
- [x] Native map rendering is close to the game shader path for terrain, borders,
  snow, mud, frozen seas, season color, water/rivers, and dynamic map state.
- [x] `.battleplan` recorder/container exists and was live round-trip proven.
- [ ] Product parity is not complete: every visible read-only game interaction has
  not been implemented/proven.
- [ ] Product parity is blocked until interaction parity, hourly capture,
  side-by-side visual comparison, and the remaining data rows are closed or
  proven/badged.
- [ ] Hourly replay capture is not proven for a running unpaused game: the GUI
  recorder now has selectable every-1/2/6-hour cadence modes, manual/paused
  one-shot capture, in-game-hour de-duplication, integer timing diagnostics,
  keeping-up status, safety warnings, and pre-write hour-crossing rejection, but
  live unpaused no-gap proof is still missing.
- [ ] Side-by-side game-vs-app visual and interaction comparison has not been run
  across the full view set.

## 1. Recorder And Hourly Capture

Goal: a recording captures every in-game hour needed for replay, or loudly tells
the user the current game speed/capture latency cannot keep up.

- [x] Read-only capture loop exists (`live_capture/capture.py`).
- [x] Single-writer lock exists for capture runs.
- [x] Build gate fails closed on unknown the game executable hash.
- [x] Live clock date provider exists in the GUI recorder.
- [x] `.battleplan` writer is wired and finalized atomically.
- [x] Warm-pinning optimization exists and is tested (`test_warm_pins*.py`).
- [x] Documented performance improvement: cold walk about 45 min to 18.7 min;
  warm walk about 135 s to 39 s.
- [x] Add in-game-hour de-dup scheduler: poll the live clock cheaply, capture
  the first trusted hour and every strictly later trusted hour, skip duplicate
  observations, and record skipped-hour diagnostics after durable write.
- [x] Reject hour-crossing captures before durable write: the recorder now reads
  the clock again immediately before writing, refuses snapshots whose hour changed
  during capture, leaves cadence uncommitted, and records the sentinel warning.
- [ ] Run live unpaused proof for hour-boundary behavior at intended MP speed; the
  rejection path is unit-tested, but real-speed no-gap proof is still required.
- [x] Replace the fixed hourly recorder behavior with explicit cadence modes:
  `every in-game hour`, `every 2 in-game hours`, `every 6 in-game hours`,
  and `manual/paused`.
- [x] Add recorder UI field showing the current cadence/mode.
- [x] Add selectable cadence modes instead of the fixed single hourly mode. The
  recorder still uses a 1-second wall-clock polling loop to observe the live
  game clock, then gates captures by the selected in-game cadence.
- [x] Add recorder status showing last capture duration, observed game-hour
  interval, and whether the recorder is keeping up. The GUI now reports the last
  successful capture duration, observed per-game-hour wall interval, and
  keeping-up/not-keeping-up state, and records integer timing diagnostics in the
  `.battleplan` manifest.
- [x] Add a safety warning when capture duration is greater than the observed
  in-game hour duration. The warning is surfaced in GUI status and persisted as
  `capture_slower_than_game_hour` in recorder diagnostics.
- [x] Add a replay manifest field for skipped or duplicate in-game hours.
- [x] Add tests for canonical hour de-duplication and skipped-hour reporting.
- [x] Add tests for cadence mode parsing/defaults, every-N wait/skip behavior,
  and manual/paused one-shot worker capture.
- [ ] Run a live unpaused recording test at intended MP speed and prove no hourly
  gaps in the `.battleplan` index.
- [ ] Decide whether to pursue further Stage-4 perf work on `land_combats` after
  measuring live hour-boundary capture. Current 39 s warm capture is not enough
  evidence by itself.

## 2. Read-Only UI Interaction Parity

Goal: anything a player would click/hover/open to inspect in the game works in the
replay app unless it would mutate game state; mutating controls are visibly inert
or read-only-badged.

- [x] Generic window host exists: open/close, z-order, modal layer, hit-test,
  block-map-behind-panel behavior.
- [x] Menubar can open registered views.
- [x] Scrubber rebinds open registered views.
- [x] Extracted GUI `@` position defines are preserved from file-root and
  unambiguous nested GUI blocks, and the runtime corpus test rejects unresolved
  `@` position anchors instead of silently laying them out at `0.0`. The only
  allowed raw coordinate token is the vanilla `tech_tree.json` typo `275k`.
- [x] Map pan/zoom/province picking exists.
- [x] Minimap pick/recenter primitive exists.
- [x] Combat bubbles can open a read-only combat panel.
- [x] Some graph pan/zoom interactions exist for tree views.
- [x] First-pass interaction inventory exists for all 114 registered views
  (`INTERACTION_PARITY_INVENTORY.md`).
- [x] First executable view-level interaction-contract scaffold exists for all
  114 registered views (`replay_native/app/src/views/interaction_contract.rs`).
- [x] Executable interaction coverage report exists and is tested. Current
  report totals: 114 registered views, 114 contracts, 1,140 category rows, 108
  target rows, 108 target rows with executable-proof notes, fifteen intentionally
  inert mutation targets, and 975 `not_proven` category rows.
  Implemented/proven target count is intentionally not represented by the
  current `open`/`not_proven` target-status model.
- [x] First executable `tab_switch` target inventory slice exists for known N2
  tab ids in `politics.politics`, `military.army`, `designers.division`,
  `intel.intel_agency`, `chrome.find`, and `logistics.unit_view`. The completed
  target-accounting tests now cover all six registered views.
- [x] First real read-only N2 tab-switch slice exists for `chrome.find`: tests
  click States/Countries/Victory Points, verify the digest-visible catalog
  changes, prove non-pickable tab clicks are inert, and prove selected-tab
  state survives scrub A->B->A.
- [x] First real `military.army` N2 tab-switch slice exists: tests click
  Army/Navy/Air, verify the digest-visible roster changes, prove unavailable
  tab clicks are inert, and prove selected-tab state survives scrub A->B->A
  without carrying B-only rows.
- [x] First real `military.army` roster row-selection/scroll slice exists:
  tests prove keyed Army/Navy/Air row selection/highlight, wheel-scroll and
  scrollbar-thumb drag visible window changes/clamps, captured thumb drags
  continue outside the list until pointer-up, track clicks do not select
  underlying rows, scrolled clicks select the correct backing row, outside-list
  wheel input is inert, per-tab scroll state stays isolated, and selection/scroll
  survive scrub A->B->A. Detail drilldowns, exact scrollbar art, and other
  military lists remain open.
- [x] First real `politics.politics` N2 tab-switch slice exists: tests click
  Government/National Spirits/Ideas & Laws/Autonomy, verify digest-visible
  content changes, prove unavailable tab clicks are inert, and prove selected
  tab state survives scrub A->B->A without carrying B-only rows.
- [x] First real `politics.politics` active-tab row-selection/scroll slice
  exists: tests prove stable-keyed Government/National Spirits/Ideas & Laws/
  Autonomy row selection/highlight, duplicate/unkeyed row inert behavior,
  long National Spirits plus Ideas & Laws wheel-scroll and scrollbar-thumb drag
  visible window changes/clamps, captured thumb drags continue outside the list
  until pointer-up, track clicks do not select underlying rows, scrolled clicks
  select the correct backing row, per-tab scroll state stays isolated, and
  selection/scroll survive scrub A->B->A without carrying B-only rows.
  Government/Autonomy long-list scroll, modal drilldowns, exact scrollbar art,
  hover tooltips, disabled mutation visuals, and side-by-side proof remain open.
- [x] First real `designers.division` N2 tab-switch slice exists: tests click
  Division/Army HQ for HQ-capable templates, prove non-HQ Army HQ clicks are
  inert, and prove selected tab state survives scrub A->B->A without carrying
  B-only template rows.
- [x] First real `intel.intel_agency` N2 tab-switch slice exists: tests click
  Agency/Operations/Cryptology, prove unavailable tab clicks are inert, and
  prove selected tab state survives scrub A->B->A.
- [x] First real `chrome.find` row-selection/drilldown slice exists: tests prove
  snapshot-derived row selection/highlight, outside-row inert clicks, selected
  row scrub A->B->A behavior, and read-only `core.state_panel` drill commands
  for state and Victory Point rows.
- [x] First real `chrome.find` results-scroll slice exists: tests prove wheel
  scrolling and scrollbar-thumb drag change/clamp visible rows, captured thumb
  drags continue outside the list until pointer-up, track clicks do not select
  or drill underlying rows, scrolled clicks select the correct backing row, and
  scroll offset survives scrub A->B->A without carrying B-only rows. Exact
  scrollbar art remains open.
- [x] First real `chrome.find` text query/filtering slice exists: tests focus
  the real `to_find_text` edit box, type/backspace replay-local query text,
  filter over honest catalog labels/keys, reset/clamp old scroll, consume
  focused text before global shortcuts, and preserve query state across scrub
  A->B->A without carrying B-only rows. Localized names and full text-field
  parity remain open.
- [x] First real `chrome.find` map-navigation slice exists: tests prove
  `find_button` recenters the selected row or first filtered state/country/VP
  row via read-only `RecenterMap`, `goto_home_button` recenters measured
  capital VP geometry, button clicks blur focused query text, and missing
  geometry fails closed without fabricating a map location. Country detail
  panels, province-level VP drilldown, localized names, exact scrollbar art, and
  screenshot proof remain open.
- [x] First real `core.topbar` read-only navigation slice exists: tests prove
  the real extracted topbar `.gui` exposes the registered navigation button
  rects, synced topbar clicks emit read-only `OpenView` commands for
  Decisions/Intel/Research/Diplomacy/Trade/Construction/Production/Deployment/
  Logistics/Officer Corps/Army/Navy/Air, the topbar render digest is unchanged,
  and empty topbar space still falls through. Faction/alerts drilldowns, hover
  frames/tooltips, disabled read-only behavior, screenshots, and broader topbar
  parity remain open.
- [x] First real `core.topbar` disabled-mutation slice exists: tests prove the
  real extracted pause/date, speed-control, lobby/menu/help/achievements,
  playlist, and music-control buttons (`pause_button_date`,
  `pause_button_date_paused`, `button_speeddown`, `button_speedstep1` through
  `button_speedstep5`, `button_speedup`, `reopen_ingame_lobby`, `menu_button`,
  `help_button`, `achievements_button`, `playlist_button`,
  `music_pause_button`, `music_next_button`, and `musicplayer_button`) are
  consumed as replay-disabled no-op pointer-down/up clicks with no commands, no
  redraw, and unchanged topbar digest; shell tests also prove higher-z panels
  prevent hidden topbar inert targets from arming. `dismissed_alerts_button`,
  threat/faction controls, disabled visual treatment, screenshots, and broader
  topbar mutation parity remain open.
- [x] First real `chrome.minimap` click/drag recenter slice exists: tests prove
  fitted-map clicks and drags emit read-only `RecenterMap` commands, command
  application changes the bounded map-camera offset, pointer-up clears drag
  capture state, letterbox clicks are explicit no-ops, outside clicks fall
  through, and no recenter is emitted without a live camera. Screenshot and
  side-by-side proof remain open.
- [x] First real `core.map` province-hover slice exists: shell hover refresh
  follows the exposed-map/host-miss rule used by map picking, and tests prove
  factual province/state/owner/raster tooltip rows while unresolved metadata
  stays unavailable. Full the game map tooltip frames, per-mode legend hover,
  map-icon hover, screenshots, and side-by-side proof remain open.
- [x] First real `core.state_panel` building-grid scroll slice exists: tests
  prove wheel-scroll and scrollbar-thumb drag over the real state-building
  grid geometry change/clamp visible building rows, track clicks are consumed
  without scroll/row/map leakage, outside-grid wheel input is inert, pointer-up
  clears drag capture, and scrub rebind clamps/clears stale drag when the
  selected state's building rows shrink. A shell-level test proves the captured
  `close_button` leaf closes the state panel through the host/shell close path.
  A resource-grid geometry test proves the six known state-resource rows fit
  without overflow in the measured resource grid, so no resource scrollbar is
  required under the current data model. Province/state drilldown links, exact
  scrollbar art, screenshots, and side-by-side proof remain open.
- [x] First real `core.state_panel` factual hover-tooltip slice exists: tests
  prove passive owner-flag, scroll-aware building-row, and resource-row tooltips
  are generated from the bound state report, clear on hover exit, and clear
  stale building-row hover on scrub rebind instead of fabricating another row.
  Full the game tooltip frames, hover chrome, screenshots, and side-by-side proof
  remain open.
- [x] First real `core.state_panel` disabled-mutation slice exists: tests prove
  visible `select_law_button`, `template_garrison_button`, and
  `scorched_earth_state_button` clicks are consumed as replay-disabled no-op
  pointer-down/up events with no commands, no redraw, and unchanged digest, while
  neighboring non-control panel space remains unclaimed by the view-specific
  handler. A guard now proves `state_shared_slot_building_entry` `remove`
  remains unrendered/unclaimed even when the GUI defines it; native shared-slot
  row-factory rendering is the next prerequisite. Disabled visual treatment,
  screenshots, and side-by-side proof remain open.
- [x] First real `core.focus_tree` focus-node click-selection slice exists:
  tests prove exact focus-id keyed selection/highlight, drag-pan conflict
  protection, missing selected focus fail-closed behavior without node fallback,
  selected-node A->B->A detail rebuild, and a factual selected-detail
  panel/digest slice for ID, state, cost, current progress days, prereq group
  count, and position fidelity with missing cost/progress unavailable instead
  of fabricated as zero. Full the game `national_focus_detail_view` parity,
  search/filter/shortcut controls, mutation controls, screenshots, and
  side-by-side proof remain open.
- [x] First real `logistics.unit_view` stats-panel tab-switch slice exists:
  tests click Stats/History/Template, prove unchanged-tab clicks consume without
  redraw, prove selected-tab state survives scrub A->B->A, and prove non-stats
  tabs hide the equipment body without fabricating rows. Template/history data
  binding and broader unit-detail parity remain open.
- [x] First real `military.theatre_selector` row-selection/scroll slice exists:
  tests prove measured theatre-id row selection/highlight, wheel-scroll visible
  window changes/clamps, scrollbar-thumb drag clamps and continues outside the
  list until pointer-up, scrollbar track clicks do not select underlying rows,
  scrolled clicks select the correct backing theatre row, duplicate/missing ids
  are unselectable without index/name fallback, missing selected rows fail
  closed, outside-list wheel input is inert, and selection/scroll survive scrub
  A->B->A. Group/member drilldowns, alerts, pin/priority UI state, scrollbar
  art, and side-by-side proof remain open.
- [x] First real `production.construction` construction-queue row-selection/
  scroll slice exists: tests prove measured live `line_id` and save
  `template`/`location_state`/`created_date` identities, wheel-scroll visible
  window changes/clamps, scrollbar-thumb drag clamps and continues outside the
  grid until pointer-up, track clicks do not select underlying rows, scrolled
  clicks select the correct backing construction row, unkeyed rows are inert
  without index fallback, missing selected rows fail closed, outside-grid wheel
  input is inert, and selection/scroll survive scrub A->B->A. Building
  drilldowns, scrollbar art, and broader construction mutation parity remain
  open.
- [x] First real `production.production` disabled-mutation slice exists: tests
  prove visible root add-line buttons
  (`infantry_button`/`tank_button`/`aircraft_button`/`naval_button`/
  `naval_repair_button`), visible `close_production_line` buttons, factory
  add/sub buttons on scrolled production rows, plus priority/amount controls
  (`increase_priority`/`decrease_priority`, military `btn_x10`/`btn_x5`, and
  naval `increase_amount`/`decrease_amount`), are consumed as replay-disabled
  no-op clicks with no emitted commands, no redraw request, and unchanged
  selected row/scroll/bind digest. Row-control tests prove neighboring row-body
  clicks still select the backing production row, and root add-line tests prove
  empty space between aircraft and naval groups remains unclaimed. The
  production GUI `@prod_*` anchors are now preserved as real defines and
  resolved before hit-testing.
  Convert/deploy/refit/obsolete controls, disabled visuals, and broader
  production mutation parity remain open.
- [x] First real `production.construction` disabled-mutation slice exists:
  tests prove the top `count_container` factory add/sub buttons
  (`factories_sub_1x/5x/10x`, `factories_add_1x/5x/10x`) are consumed as
  replay-disabled no-op clicks with no emitted commands, no redraw request, and
  unchanged selected row/scroll/bind digest, while neighboring count-container
  clicks remain unclaimed. The rendered `production_building_line_entry` queue
  row controls (`increase_priority`/`decrease_priority`,
  `increase_amount`/`decrease_amount`, `close_production_line`) are also
  consumed as replay-disabled no-op clicks with unchanged selected
  row/scroll/bind digest, while neighboring row-body clicks still select the
  backing construction row. Build-picker actions, market/debug controls,
  disabled visuals, and broader construction mutation parity remain open.
- [x] First real `production.decisions` timed-decision row-selection/scroll
  slice exists: tests prove measured decision/state identities, wheel-scroll
  visible window changes/clamps, scrollbar-thumb drag clamps and continues
  outside the grid until pointer-up, track clicks do not select underlying rows,
  scrolled clicks select the correct backing decision row, unkeyed rows are
  inert without index fallback, missing selected rows fail closed, outside-grid
  wheel input is inert, and selection/scroll survive scrub A->B->A. Decision
  detail drilldowns, one-shot availability, scrollbar art, and broader mutating
  decision buttons remain open.
- [x] First real `production.decisions` disabled-mutation slice exists: visible
  timed-decision `btn_select` action buttons are consumed as replay-disabled
  no-op pointer-down/up clicks with no commands, no redraw, and unchanged
  selected row/scroll/bind digest. Tests also prove neighboring row-body clicks
  still select the backing timed decision and scrolled visible button hits do
  not select the row. One-shot decision actions, disabled visual treatment, and
  broader decisions mutation parity remain open.
- [x] First real `production.production` production-line row-selection/scroll
  slice exists: tests prove measured live `id` and save `line_id` identities,
  preserve military/naval line identity, wheel-scroll and scrollbar-thumb drag
  visible window changes/clamps, captured thumb drags keep scrolling outside the
  grid until pointer-up, track clicks do not select underlying rows, scrolled
  clicks select the correct backing production row, unkeyed and duplicate ids are
  inert without index fallback, missing selected rows fail closed, outside-grid
  wheel input is inert, and selection/scroll survive scrub A->B->A. Primary
  category filters (`show_equipment`/`show_armor`/`show_air`/
  `show_capital_ships`/`show_screen_ships`/`show_other_ships`) now toggle
  replay-local row visibility from catalog-derived categories, including unique
  live-id to save-`line_id` joins for live rows without `archetype`, while
  unknown/ambiguous rows fail open. Secondary/status filter checkboxes
  (`show_resources`/`show_outdated`/`show_stockpiling`/`show_reinforcing`/
  `show_upgrading`/`show_refitting`) now have geometry-anchored disabled no-op
  proof so clicks cannot bleed into row selection; true status filtering and
  disabled visual treatment remain open until per-line status flags are
  available. Line detail drilldowns, scrollbar art, mutating line controls, and
  side-by-side proof remain open.
- [x] First real `research.research` active-slot row-selection slice exists:
  tests prove exact tech-token keyed slot selection/highlight, empty/outside
  slot inert behavior, missing selected tech fail-closed behavior without row
  fallback, and selected-slot scrub A->B->A without carrying B-only rows. Slot
  detail drilldowns, ETA/progress parity, mutating research controls, and
  side-by-side proof remain open.
- [x] First real `research.tech_tree` factual hover-tooltip slice exists:
  tests prove tech-node ID, folder, state, and research cost rows, with
  missing/null cost badged unavailable instead of fabricated as zero. Full the game
  tooltip frames, prerequisites, localized domain composers, and side-by-side
  proof remain open.
- [x] First real `research.doctrine_tree` factual hover-tooltip slice exists:
  tests prove doctrine-node ID, folder, exact-when-fed state, unfed-badged
  parser-unfed state, and full static XOR-link count rows without fabricating
  absence as unresearched. Full the game tooltip frames, prerequisites/effects,
  localized domain composers, and side-by-side proof remain open.
- [x] First real `research.tech_tree` tech-node click-selection slice exists:
  tests prove exact catalog-id keyed node selection, drag-pan conflict
  protection, missing selected tech fail-closed behavior, selected-node A->B->A
  scrub rebuild, and independently pickable common completed/in-progress nodes
  after folder-grid/sub-tech/duplicate-slot layout disambiguation. Selected
  nodes now emit a factual read-only detail panel/digest slice for title, ID,
  folder, state, and research cost; missing/null cost stays unavailable instead
  of fabricated as zero. Full the game detail-window parity, exact per-folder host
  geometry, mutation-control inertness, and side-by-side proof remain open.
- [x] First real `research.doctrine_tree` doctrine-node click-selection slice
  exists: tests prove exact catalog-id keyed node selection, drag-pan conflict
  protection, missing selected doctrine fail-closed behavior without node
  fallback, and selected-node A->B->A scrub rebuild while keeping parser-unfed
  doctrine state unavailable unless exact doctrine IDs are fed. Selected nodes
  now emit a factual read-only detail panel/digest slice for title, ID, folder,
  exact-when-fed/unfed-badged state, and XOR-link count without fabricating
  parser-unfed doctrine state as unresearched. Full the game detail-window parity,
  doctrine tooltip parity, mutation-control inertness, and side-by-side proof
  remain open.
- [x] First real `leaders.army_trait_tree` / `leaders.navy_trait_tree`
  owned-trait-node click-selection slice exists: tests prove exact trait-token
  keyed selection/highlight for the currently surfaced N3 owned trait nodes,
  inert no-leader/no-traits unavailable glyphs, drag movement canceling click
  selection, missing selected trait fail-closed behavior without node fallback,
  selected-node A->B->A scrub rebuild, and factual selected-detail panel/digest
  rows for trait id, owned state, branch, leader name, leader key, and derived
  static DEF catalog metadata rows for scope/cost/prereq/mutual-exclusion/
  modifier/source values with null DEF fields badged unavailable. Full static
  trait catalog graph parity, disabled/pickable states, trait icons, effect-text
  expansion, leader-roster driven selection, full the game tooltip parity,
  mutation-control inertness, and side-by-side proof remain open.
- [x] First real `leaders.army_trait_tree` / `leaders.navy_trait_tree`
  owned-trait-node hover-tooltip slice exists: tests prove factual passive
  tooltip rows for trait id, owned state, branch, leader name, leader key, and
  derived static DEF catalog metadata rows, no-leader glyph hover inertness, and
  hover-clear behavior. Full the game trait tooltip frames, localized trait
  names/icons/effect-text expansion, prerequisite text, disabled/pickable
  states, and side-by-side proof remain open.
- [x] First real `leaders.army_trait_tree` / `leaders.navy_trait_tree`
  disabled-mutation slice exists: tests prove rendered `add_trait_0` through
  `add_trait_3` trait-pick slots resolve from the real bound `.gui` roots and
  are consumed as replay-disabled no-op pointer-down/up clicks with no commands,
  no redraw, and unchanged selected trait/detail bind digest. Trait-pick
  availability, disabled visuals, tactic/wiki/tab controls, static catalog
  pickable/disabled nodes, and broader trait-tree mutation parity remain open.
- [x] First disabled-mutation slices exist: `logistics.ingame_menu`
  save/load/resign/quit verb clicks are consumed as replay-modal no-ops with no
  commands, no redraw, and unchanged digest; `logistics.event_window` displayed
  static event-definition preview option actions are consumed as replay-disabled
  no-ops with no commands, no redraw, and unchanged digest; inside-root
  outside-option chrome clicks are consumed as modal no-ops with no commands,
  no redraw, and unchanged digest while off-root clicks remain host-modal-policy
  blocking; `logistics.popup_window` displayed generic
  popup-library action clicks are consumed as replay-modal disabled no-ops with
  no commands, no redraw, and unchanged digest; `core.state_panel` visible
  occupation-law/template and scorched-earth buttons are consumed as
  replay-disabled no-ops with unchanged digest; `core.topbar` pause/date and
  speed controls plus lobby/menu/help/achievements/playlist/music controls are
  consumed as replay-disabled no-ops with unchanged digest and hidden-target
  arming protection under higher-z panels; `production.production`
  visible root add-line, close-line, factory add/sub, and priority/amount
  buttons, plus `production.construction` top count-container factory add/sub
  buttons and visible queue priority/amount/close buttons, plus
  `production.decisions` visible timed-decision `btn_select` buttons are consumed
  as replay-disabled no-ops with unchanged digests; `leaders.army_trait_tree`
  and `leaders.navy_trait_tree` rendered `add_trait_0` through `add_trait_3`
  slots are consumed as replay-disabled no-ops with unchanged selected
  trait/detail digests. Per-button leaf routing,
  disabled visual treatment, fired-event historical choice parity, full event
  complete event modal behavior, per-root popup buttons, native shared-slot
  row-factory rendering plus building remove buttons, topbar
  dismissed-alerts/threat/faction controls, one-shot decision actions, and
  broader mutation parity remain open.
- [ ] Define an interaction contract for every registered view: tabs, subtabs,
  list row selection, drilldowns, dropdowns, filters, scrollbars, close buttons,
  map picks, graph pan/zoom, and hover targets.
- [ ] Expand the scaffold beyond the current `tab_switch` target slice into
  executable per-target contracts/tests for rows, text inputs, scrollbars, hovers,
  drilldowns, disabled mutation controls, and proof links.
- [ ] Implement and test every read-only tab/subtab switch.
  Partial: `chrome.find`, `military.army`, `politics.politics`,
  `designers.division`, `intel.intel_agency`, and `logistics.unit_view` have
  executable tab-switch slices with scrub-safe selected-state proof.
- [ ] Implement and test every read-only row selection and detail pane update.
  Partial: `chrome.find` catalog rows and `military.army` keyed Army/Navy/Air
  roster rows are implemented and tested; `logistics.unit_view` measured
  `div_id` division-row selection updates the equipment pane, survives scrub by
  rebuilding current-hour equipment, rejects missing row identity, and fails
  closed when the selected division is missing or foreign-owned;
  `production.decisions` timed-decision row selection and wheel/thumb scroll are
  keyed by measured decision/state identity, survive scrub, reject unkeyed rows,
  select the correct backing row after scroll, clamp visible rows, and fail
  closed when the selected decision row disappears; `military.theatre_selector`
  measured theatre-id row selection survives scrub, rejects missing/duplicate
  ids without fallback, and fails closed when the selected theatre row disappears;
  `politics.politics` active-tab row selection survives scrub, rejects
  duplicate/unkeyed rows without fallback, selects the correct backing row after
  scroll, and isolates selection by tab; `research.research` active-slot row
  selection is keyed by exact in-progress tech token, keeps empty/outside slots
  inert, and fails closed when the selected tech disappears;
  `research.tech_tree` node selection is keyed by exact catalog tech id, protects
  drag-pan from accidental selection, fails closed when the selected tech
  disappears, and rebuilds selected-node state plus the factual selected-detail
  digest across A->B->A scrub;
  `research.doctrine_tree` node selection is keyed by exact catalog doctrine id,
  protects drag-pan from accidental selection, fails closed when the selected
  doctrine disappears, and rebuilds selected-node state plus the factual
  selected-detail digest across A->B->A scrub;
  `leaders.army_trait_tree` and `leaders.navy_trait_tree` owned-trait node
  selection are keyed by exact surfaced trait tokens, keep no-leader/no-traits
  glyphs inert, cancel selection on drag movement, fail closed when the selected
  trait disappears, and rebuild factual selected-detail rows across A->B->A
  scrub; owned trait-node hover tooltips now expose factual trait/leader rows
  and clear off-node without inventing unavailable glyph tooltips;
  `production.construction` construction-queue row selection survives scrub,
  rejects unkeyed live/save rows without fallback, supports both live line ids
  and save-backed template/state/created identities, and fails closed when the
  selected queue row disappears; `production.production` production-line row
  selection survives scrub, rejects unkeyed/duplicate live/save rows without
  fallback, supports live ids and save-backed line ids, and fails closed when the
  selected production line disappears.
- [ ] Implement and test every read-only drilldown:
  country flag to country view, state/province to state panel, unit/army/fleet/air
  wing to details, combat bubble to combat detail, focus/tech/doctrine node to
  details, event/decision/log row to details. Partial: `core.topbar`
  navigation buttons now open existing replay views read-only; faction/alerts
  and most row-to-detail drilldowns remain open.
- [ ] Implement and test every scrollbar/drag-scroll surface with hit clipping.
  Partial: `core.state_panel` building-grid wheel-scroll and scrollbar-thumb
  drag are implemented and tested with clamp/outside-grid-inert/
  track-click-inert/scrub-clamp proof; `chrome.find` results wheel-scroll and
  scrollbar-thumb drag plus
  `military.army` Army/Navy/Air roster wheel-scroll and scrollbar-thumb drag are
  implemented and tested;
  `politics.politics` long National Spirits plus Ideas & Laws row wheel-scroll
  and scrollbar-thumb drag are implemented and tested with clamp/scrolled-click/
  outside-list-inert/track-click-inert/per-tab-isolation proof;
  `logistics.unit_view`
  division-list wheel-scroll and scrollbar-thumb drag are implemented and tested
  over measured `div_id` rows with scrub-safe scrolled selection, stale-capture
  clearing, and inert track clicks, and selected-division equipment-pane
  wheel-scroll and scrollbar-thumb drag are implemented and tested with
  clamp/reset/missing-selected fail-closed proof; `military.theatre_selector`
  theatre-list wheel-scroll and
  scrollbar-thumb drag are implemented and tested with clamp/scrolled-click/
  outside-list-inert/track-click-inert proof;
  `production.production` production-line wheel-scroll and scrollbar-thumb drag
  are implemented and tested with clamp/scrolled-click/outside-grid-inert/
  track-click-inert proof;
  `production.construction` construction-queue wheel-scroll and
  scrollbar-thumb drag are implemented and tested with clamp/scrolled-click/
  outside-grid-inert/track-click-inert proof; `production.decisions`
  timed-decision wheel-scroll and scrollbar-thumb drag are implemented and
  tested with clamp/scrolled-click/outside-grid-inert/track-click-inert proof.
- [ ] Implement and test every read-only text input/filter field with focus-safe
  keyboard behavior. Partial: `chrome.find` query/filtering is implemented and
  tested over existing honest catalog labels/keys.
- [ ] Implement and test map mode selector, legend, and hover readouts for all
  replay-supported map modes. Partial: real laid-out `chrome.mapmodes`
  numbered rail slots now emit read-only `SetMapMode`; the active rail echo is
  synced from the seeded map view and tested, rail-row hover emits a factual
  source/fidelity tooltip, exposed `core.map` province hover emits factual
  province/state/owner/raster tooltip rows, Air/Naval map modes now render
  visible `naval_air` strategic-region proxy colours through the parsed region
  join with missing/null/unmapped regions neutral, and the real `mapmodes_all`
  popup exposes Supply/Intel without pretending they are numbered rail slots. Custom
  slot configuration/defaults, full the game tooltip frames, per-mode legend hover,
  screenshots, and broader parity remain open.
- [ ] Implement read-only disabled state for every mutating the game control; it must
  look like a disabled/inert game control, not a missing feature.
- [ ] Implement hover state rendering for buttons/icons/rows where the game has a
  visible hover frame/highlight.
- [ ] Implement keyboard/navigation behavior required for inspection: escape/close,
  view cycling where applicable, scrub/playback shortcuts, and focus-safe text
  entry behavior for read-only fields. Partial: focused `chrome.find` text input
  consumes typed keys before global shortcuts.
- [ ] Add per-view interaction tests, not just bind/scrub tests.
- [x] Add an interaction coverage report: registered view count, contract count,
  category-row count, interactive target count, missing/not-proven category
  count, and per-category target/status counts are executable and tested. The
  report keeps implemented/proven target count unset, and separately counts the
  fifteen first-class intentionally inert mutation targets without treating them as
  broad mutation parity.

## 3. Tooltip And Localization Parity

Goal: hovering real the game UI targets produces HOI4-equivalent explanatory text for
captured replay data, with missing computed pieces explicitly badged.

- [x] Localization and markup foundations exist.
- [x] Some binding maps identify tooltip/computed gaps.
- [x] Build the first generic passive tooltip primitive and host integration:
  tooltip metadata is preserved, tooltip plans participate in bound digests, and
  tech-tree hover has a factual no-fabrication vertical slice for ID, folder,
  state, research cost, and missing/null-cost badging. State-panel owner,
  building-row, and resource-row hovers now add a factual bound-report slice.
- [ ] Expand the generic tooltip resolver to full game-style tooltip frames:
  hovered node to tooltip key, bound datum, localization expansion, and
  HOI4-equivalent frame rendering.
- [ ] Build per-domain tooltip composers for topbar, map, politics, production,
  research, military, designers, leaders, intel, diplomacy, logistics, combat,
  events, and scripted GUI overlays.
- [ ] Parse/evaluate the non-mutating subset of trigger/effect conditions needed
  for tooltip visibility and locked/available explanations.
- [ ] Badge omitted computed tooltip lines instead of silently dropping them.
- [ ] Add snapshot fixtures for tooltip text stability.
- [ ] Add side-by-side tooltip comparison cases from real the game screenshots or
  recorded expected text.

## 4. Visual And Pixel Parity

Goal: the app is not merely "HOI4-like"; it is checked against the real game for
visible replay surfaces.

- [x] Real `.gui` layout data is used where available.
- [x] Real sprites/fonts/assets are routed through the asset resolver.
- [x] Map shader path is close to the game shader implementation.
- [x] Dynamic weather/snow/mud/frozen-sea visuals are wired.
- [ ] Replace the older "unmistakably the game" acceptance wording with a stricter
  side-by-side parity target for all replay-inspection surfaces.
- [x] Add registry-derived screenshot target manifest plumbing:
  `--shot-all-views=<dir>` writes `shot_manifest.json` with deterministic
  `native__`, `hoi4__`, and `diff__` targets per registered view, plus
  fail-closed `core.state_panel` shot-state handling.
- [ ] Extend the deterministic screenshot harness so it captures each
  registered view, selected tabs, representative rows, tooltips, and map modes.
- [ ] Capture matching the game screenshots for the same states/windows.
- [ ] Add visual diff thresholds and a reviewed exception list for legitimate
  read-only or wall-badged differences.
- [ ] Verify all fonts, icon frames, disabled states, hover frames, scrollbars,
  clipping, row spacing, and modal layering against game screenshots.
- [ ] Verify the native map against real game screenshots for terrain, political,
  supply, resources, weather, fronts, unit counters, trade, battle plans, and naval
  route overlays.

## 5. Data Completeness Gaps

Source of truth: `coverage_audit/memory_manifest.json`.

Current manifest summary:

- 272 total rows.
- Authoritative row-level `status`: 147 `closed-by-reader`, 14
  `closed-by-parser`, 26 `needs-wartime-save`, 6 `open-live-extractable`,
  15 `open-computed`, 2 `wall`, 62 `excluded`.
- Legacy `gap`/coverage status: 161 `covered`, 62 `excluded-by-scope`,
  30 `GAP-save-not-live`, 14 `GAP-computed`, 2 `GAP-uncovered`,
  1 `GAP-visual`, 2 `WALL`.
- Closure headline: 161 rows are closed (147 reader/save + 14 parser/static),
  47 closeable/provable rows remain open (26 fixture-gated + 6 live/static +
  15 computed/visual), and 2 rows are permanent walls. This is not a product
  parity claim.

### 5.1 Needs User Fixture Or Wartime State

These cannot be honestly closed from the current USA-1936 peacetime frame:

- [ ] `peace_conference`
- [ ] `history.sunk_ship`
- [ ] `ships_built`
- [ ] `incoming_diplomatic_action`
- [ ] `civil_war_target`
- [ ] `expeditionaries_sent`
- [ ] `invasion_report`
- [ ] `dirty_controlled_states`
- [ ] `removed_controlled_province`
- [ ] `dynamic_revolution_tag`
- [ ] `states.active_targeted_modifier`
- [ ] `states.previous_owner`
- [ ] `states.last_strategic_bombing`
- [ ] `states.is_border_conflict`
- [ ] `combat_log`
- [ ] `combat_data_entry`
- [ ] `selection_groups`
- [ ] `government_in_exile`
- [ ] `collaboration_government`
- [ ] `player_settings`
- [ ] `strategic_region_data`
- [ ] `ui_live.alerts_ledger`
- [ ] `ui_live.war_participant_casualties`
- [ ] `ui_live.operative_code_name`
- [ ] `ui_live.admiral_fleet_composition`
- [ ] `ui_live.ship_owner_wing_ace_join`

User help needed: load and pause one or more mid-war/eventful saves that contain
these mechanics. The extraction agent must content-match by live date and ROOT tag,
not filename.

### 5.2 Open But Not User-Blocked

These should be worked down before calling product parity complete:

- [x] `difficulty_settings` (closed save-side by `parse_world` as explicit
  `world.scenario.difficulty_settings_empty`; no hidden difficulty payload is
  invented, and selected difficulty remains `world.scenario.difficulty`)
- [ ] `all_playthrough_data`
- [ ] `countries.*.logistics`
- [ ] `countries.*.strategic_navy`
- [ ] `countries.*.upgrades`
- [ ] `mapicon.general_portrait_on_counter` (partial: anchored OOB/commander
  portrait joins, anchored land-counter count/owner/org/fractional-strength/type/dig-in
  logical entities, memory-scaled strength progressbars fail closed at the anchor,
  and anchored army/army-group formation insignia sprite placement now fail closed;
  full GUI counter chrome/art, commander selection,
  navy/air/rail/transport counters, HQ/supply/attrition, and visual parity still open)
- [ ] `ui_open.insignia_emblem_assets` (partial: shared asset resolver now covers
  explicit formation insignia/emblem fields plus one-based army/army-group shield
  frames with explicit miss accounting; `military.army` formation rows now consume
  that shared resolver and prove explicit metadata exact, one-based frame derived,
  and missing-base/generic/out-of-range unavailable digest accounting. Mapicon
  unit-counter formation-insignia entities now consume the same resolver only at
  real OOB/division counter anchors. Selected-formation binding exists, but
  complete extraction/render parity and side-by-side visual proof remain open)
- [x] `ui_open.vp_province_geometry` (closed by `map_pipeline` static map data
  plus native centroid projection: explicit VP province placement is used, and
  tests prove no state-id/representative fallback when an explicit VP province is
  absent)
- [x] `ui_open.tech_doctrine_render_geometry` (closed by current `parse_tech`
  + `parse_doctrines` catalogs and native `research_tech_doctrine` graph tests:
  every tree-visible tech/doctrine node is positioned, hidden/non-tree techs are
  classified, and edges/XOR groups are accounted)
- [x] `doctrine` (save-cadence closed by `parse_doctrine`: top-level
  `doctrine.countries` is parsed into TAG-keyed rows under
  `countries.*.research.doctrine`; no live doctrine-manager root is claimed)
- [x] `countries.*.flags` (save-backed under
  `countries.*.extra.script_flags`; live CFlagManager payload decoding remains
  fail-closed, not guessed)

### 5.3 Computed Jobs

These are not stored directly, but for a 1-to-1 inspector they still need either
a faithful recompute or a reviewed permanent badge:

- [ ] `map.weather_phenomenon_overlay` (data is live-captured as
  `weather.regions.*.phenomenon`; exact the game weather map-mode overlay semantics
  and side-by-side rendering remain open)
- [ ] `map.front_lines_overlay` (partial: derived controller-boundary overlay
  exists; route/front/battleplan/trade lines now carry deterministic semantic
  style metadata separate from derived/exact fidelity, explicit measured route
  and save-backed battle-plan map-point polylines ingest as exact, explicit
  `map.front_lines[]` measured geometry ingests as exact, and malformed/missing
  anchored geometry renders unavailable diagnostic markers instead of falling
  back to a fabricated line. Exact/measured route or front geometry without
  points now fails closed instead of falling back to derived province/controller
  paths. Live `oob_command` `member_div_ids` now anchor live type-5
  `battle_plan_orders[].target_states` into derived battle-plan arrows instead
  of silently losing the join. Real the game route/front extraction coverage, exact
  battleplan semantics, and side-by-side styling parity still open)
- [ ] `mapicon.attrition_flag`
- [ ] `world_tension`
- [ ] `research completion fraction / ETA` (static research costs are
  parser-backed; the progress fraction and ETA remain derived/fail-closed
  because runtime progress/speed math is not fully captured)
- [ ] `weather temperature formula / RNG / phenomenon-name strings`
- [ ] `combat detail stat rows`
- [ ] `ui_computed.designer_combat_stats`
- [ ] `ui_computed.naval_air_per_province_exactness` (native Air/Naval now draw
  joined `naval_air` strategic-region proxy colours; exact per-province the game
  parity remains intentionally unclaimed)
- [ ] `ui_computed.occupation_equilibrium_garrison_need`
- [ ] `ui_computed.power_balance_trend_marker`
- [ ] `ui_computed.operation_chance_intel_bands_capit`
- [ ] `ui_computed.airwing_effective_planes_denom`
- [ ] `ui_computed.war_participant_score`
- [ ] `ui_computed.combat_battle_detail`

### 5.4 Hard Walls

These should stay badged unless new evidence proves otherwise:

- [ ] Exact per-province supply tooltip number.
- [ ] True Steam real names.

## 6. Product Acceptance Tests

The app is product-complete only when these pass:

- [ ] Full Rust workspace tests pass.
- [ ] Full Python pytest suite passes.
- [ ] `coverage_audit/check_coverage.py` passes.
- [ ] `.battleplan` recorder captures a live session, finalizes, and reopens in
  the native app.
- [ ] Recorder proves no missing in-game hours for the configured cadence, or
  records explicit skipped-hour diagnostics.
- [ ] Every registered view opens from the in-app menu.
- [ ] Every registered view has an interaction contract.
- [ ] Every read-only interaction in each contract is implemented and tested.
- [ ] Every mutating game interaction is disabled/inert and visually accounted for.
- [ ] Every major hover target has a tooltip or a tracked tooltip gap.
- [x] App screenshot harness has registry-derived per-view manifest plumbing.
  The manifest now includes native/HOI4/diff filenames and native shot args for
  all 114 registered views.
- [ ] App screenshot harness covers every view, key tabs, tooltips, and map modes.
- [ ] Side-by-side real the game comparison is reviewed for the full first-pass view set.
- [ ] User completes a real in-game test and any observed mismatch is either fixed,
  moved into this checklist, or classified as a wall with evidence.

## 7. Immediate Next Work

1. Run the live unpaused recorder proof at intended MP speed for the selected
   cadence modes and close any
   skipped-hour/cadence failures it exposes.
2. Expand `INTERACTION_PARITY_INVENTORY.md` into executable target contracts
   beyond the current partial tab/row/text/scroll/hover slices.
3. Build generic tooltip infrastructure and wire the first complete vertical slice.
4. Finish the #26 visual overlay parity pass: feed real the game route geometry
   into the measured-polyline schema, close exact front and battleplan styling
   beyond the current semantic line-style metadata and live target-state arrow
   join, and capture side-by-side map-mode screenshots.
5. Work down non-user-blocked manifest gaps, starting with visual/map drivers and
   computed values that affect common inspection workflows.
6. When the user provides a mid-war paused save, close the 26 wartime/fixture rows.
