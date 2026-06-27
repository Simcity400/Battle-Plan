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
  one-shot capture, in-game-hour de-duplication, and diagnostics, but live
  unpaused no-gap proof is still missing and exact boundary rejection is
  diagnostic-only.
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
- [ ] Prove exact hour-boundary capture/rejection under live unpaused speed; the
  current hour-crossing sentinel is post-capture diagnostic-only.
- [x] Replace the fixed hourly recorder behavior with explicit cadence modes:
  `every in-game hour`, `every 2 in-game hours`, `every 6 in-game hours`,
  and `manual/paused`.
- [x] Add recorder UI field showing the current cadence/mode.
- [x] Add selectable cadence modes instead of the fixed single hourly mode. The
  recorder still uses a 1-second wall-clock polling loop to observe the live
  game clock, then gates captures by the selected in-game cadence.
- [ ] Add recorder status showing last capture duration, observed game-hour
  interval, and whether the recorder is keeping up. Current status reports
  captured/skipped/duplicate/waiting cadence state, but not the full timing
  proof.
- [ ] Add a safety warning when capture duration is greater than the observed
  in-game hour duration.
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
- [x] Map pan/zoom/province picking exists.
- [x] Minimap pick/recenter primitive exists.
- [x] Combat bubbles can open a read-only combat panel.
- [x] Some graph pan/zoom interactions exist for tree views.
- [x] First-pass interaction inventory exists for all 114 registered views
  (`INTERACTION_PARITY_INVENTORY.md`).
- [x] First executable view-level interaction-contract scaffold exists for all
  114 registered views (`replay_native/app/src/views/interaction_contract.rs`).
- [x] Executable interaction coverage report exists and is tested. Current
  report totals: 114 registered views, 114 contracts, 1,026 category rows, 34
  target rows, 34 target rows with executable-proof notes, and 903
  `not_proven` category rows. Implemented/proven target count and intentionally
  inert mutation count are intentionally not represented by the current
  `open`/`not_proven` status model.
- [x] First executable `tab_switch` target inventory slice exists for known N2
  tab ids in `politics.politics`, `military.army`, `designers.division`,
  `intel.intel_agency`, and `chrome.find`. The completed target-accounting
  tests now cover all five registered views.
- [x] First real read-only N2 tab-switch slice exists for `chrome.find`: tests
  click States/Countries/Victory Points, verify the digest-visible catalog
  changes, prove non-pickable tab clicks are inert, and prove selected-tab
  state survives scrub A->B->A.
- [x] First real `military.army` N2 tab-switch slice exists: tests click
  Army/Navy/Air, verify the digest-visible roster changes, prove unavailable
  tab clicks are inert, and prove selected-tab state survives scrub A->B->A
  without carrying B-only rows.
- [x] First real `politics.politics` N2 tab-switch slice exists: tests click
  Government/National Spirits/Ideas & Laws/Autonomy, verify digest-visible
  content changes, prove unavailable tab clicks are inert, and prove selected
  tab state survives scrub A->B->A without carrying B-only rows.
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
- [x] First real `chrome.minimap` click-recenter slice exists: tests prove
  fitted-map clicks emit a read-only `RecenterMap` command, command application
  changes the bounded map-camera offset, letterbox clicks are explicit no-ops,
  outside clicks fall through, and no recenter is emitted without a live camera.
  Drag recenter remains open.
- [ ] Define an interaction contract for every registered view: tabs, subtabs,
  list row selection, drilldowns, dropdowns, filters, scrollbars, close buttons,
  map picks, graph pan/zoom, and hover targets.
- [ ] Expand the scaffold beyond the current `tab_switch` target slice into
  executable per-target contracts/tests for rows, scrollbars, hovers,
  drilldowns, disabled mutation controls, and proof links.
- [ ] Implement and test every read-only tab/subtab switch.
- [ ] Implement and test every read-only row selection and detail pane update.
- [ ] Implement and test every read-only drilldown:
  country flag to country view, state/province to state panel, unit/army/fleet/air
  wing to details, combat bubble to combat detail, focus/tech/doctrine node to
  details, event/decision/log row to details.
- [ ] Implement and test every scrollbar/drag-scroll surface with hit clipping.
- [ ] Implement and test map mode selector, legend, and hover readouts for all
  replay-supported map modes. Partial: `chrome.mapmodes` core-backed rail rows
  now emit read-only `SetMapMode`; active echo, non-core Intel, legend hover,
  screenshots, and broader parity remain open.
- [ ] Implement read-only disabled state for every mutating the game control; it must
  look like a disabled/inert game control, not a missing feature.
- [ ] Implement hover state rendering for buttons/icons/rows where the game has a
  visible hover frame/highlight.
- [ ] Implement keyboard/navigation behavior required for inspection: escape/close,
  view cycling where applicable, scrub/playback shortcuts, and focus-safe text
  entry behavior for read-only fields.
- [ ] Add per-view interaction tests, not just bind/scrub tests.
- [x] Add an interaction coverage report: registered view count, contract count,
  category-row count, interactive target count, missing/not-proven category
  count, and per-category target/status counts are executable and tested. The
  report keeps implemented/proven target count and intentionally inert mutation
  count unset until those states are first-class.

## 3. Tooltip And Localization Parity

Goal: hovering real the game UI targets produces HOI4-equivalent explanatory text for
captured replay data, with missing computed pieces explicitly badged.

- [x] Localization and markup foundations exist.
- [x] Some binding maps identify tooltip/computed gaps.
- [x] Build the first generic passive tooltip primitive and host integration:
  tooltip metadata is preserved, tooltip plans participate in bound digests, and
  tech-tree hover has a factual no-fabrication vertical slice.
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
- Authoritative row-level `status`: 142 `closed-by-reader`, 11
  `closed-by-parser`, 26 `needs-wartime-save`, 15 `open-live-extractable`,
  14 `open-computed`, 2 `wall`, 62 `excluded`.
- Legacy `gap`/coverage status: 153 `covered`, 63 `excluded-by-scope`,
  33 `GAP-save-not-live`, 14 `GAP-computed`, 5 `GAP-uncovered`,
  2 `GAP-visual`, 2 `WALL`.

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

- [ ] `difficulty_settings`
- [ ] `doctrine`
- [ ] `all_playthrough_data`
- [ ] `countries.logistics`
- [ ] `countries.strategic_navy`
- [ ] `countries.flags`
- [ ] `countries.upgrades`
- [ ] `map.resources_recolor`
- [ ] `map.weather_phenomenon_overlay`
- [ ] `render.frozen_water_ice_flag`
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
- [ ] `ui_open.vp_province_geometry`
- [ ] `ui_open.tech_doctrine_render_geometry`
- [ ] `ui_open.power_balance_def_thresholds`

### 5.3 Computed Jobs

These are not stored directly, but for a 1-to-1 inspector they still need either
a faithful recompute or a reviewed permanent badge:

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
- [ ] `research completion fraction`
- [ ] `weather temperature formula / RNG / phenomenon-name strings`
- [ ] `combat detail stat rows`
- [ ] `ui_computed.designer_combat_stats`
- [ ] `ui_computed.naval_air_per_province_exactness`
- [ ] `ui_computed.occupation_equilibrium_garrison_need`
- [ ] `ui_computed.power_balance_thresholds`
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
   beyond the current known-N2-tab slice.
3. Build generic tooltip infrastructure and wire the first complete vertical slice.
4. Finish the #26 visual overlay parity pass: feed real the game route geometry
   into the measured-polyline schema, close exact front and battleplan styling
   beyond the current semantic line-style metadata and live target-state arrow
   join, and capture side-by-side map-mode screenshots.
5. Work down non-user-blocked manifest gaps, starting with visual/map drivers and
   computed values that affect common inspection workflows.
6. When the user provides a mid-war paused save, close the 26 wartime/fixture rows.
