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

- `AGENTS.md` and `CODEX_PROTOCOL.md`: current Codex-native rules; no old CLI
  review workflow.
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
- The scaffold tracks `open_close`, `tab_switch`, `list_selection`, `scroll`,
  `hover_tooltip`, `drilldown`, `disabled_mutation`, `scrub_state`, and
  `screenshot_proof` for every registered view.
- Contract rows can now carry target-level inventory rows. The first slices are
  `tab_switch` rows for known N2 tab widgets with stable tab ids/labels:
  `politics.politics` (Government, National Spirits, Ideas & Laws, Autonomy),
  `military.army` (Army, Navy, Air), `designers.division` (Division, Army HQ),
  `intel.intel_agency` (Agency, Operations, Cryptology), and `chrome.find`
  (States, Countries, Victory Points), plus `chrome.find` `list_selection`
  rows (State/Country/Victory Point row), `chrome.find` `drilldown` rows
  (state/VP row to state panel), and a `chrome.minimap` `drilldown` row for
  click recenter. `chrome.mapmodes` now adds `list_selection` rows for the 13
  rail buttons backed by existing core map-mode variants.
- Focused app tests assert the registry still contains 114 views, every
  registered view has exactly one contract, and every contract has an explicit
  status for every category.
- New target tests assert that real registered views `chrome.find`,
  `military.army`, `politics.politics`, `designers.division`, and
  `intel.intel_agency` have non-empty `tab_switch` target inventories,
  `chrome.find` has explicit row/drilldown target inventories, `chrome.minimap`
  has an explicit click-recenter target inventory, `chrome.mapmodes` has
  explicit core-backed row target inventories, every recorded target has a
  stable id/label/status/note, and views without known tab targets remain
  explicitly tracked as `not_proven` instead of disappearing.
- New report tests lock the current executable counts: 114 registered views, 114
  contracts, 1,026 category rows (nine per view), 34 target rows, 34 target rows
  with executable-proof notes, and 903 `not_proven` category rows.
- The report intentionally exposes no implemented/proven target count and no
  intentionally inert mutation count yet, because the current status model only
  has `open` and `not_proven`. This keeps broad interaction parity open instead
  of converting partial proof notes into a completed status.
- `chrome.find` now has executable read-only tab-switch proof: tests click
  States/Countries/Victory Points, verify the digest-visible catalog changes,
  prove non-pickable tab clicks are inert, and prove selected-tab state
  survives scrub A->B->A. This does not cover search text, filtering,
  recentering, country/province drilldown, or the other N2 tab inventories.
- `politics.politics` now has executable read-only tab-switch proof: tests
  click Government/National Spirits/Ideas & Laws/Autonomy, verify the
  digest-visible tab content changes, prove unavailable tab clicks are inert,
  and prove selected-tab state survives scrub A->B->A without carrying B-only
  rows. This does not cover idea/law/subject row selection, modal drilldowns,
  scrolling, hover tooltips, disabled mutation visuals, or side-by-side proof.
- `military.army` now has executable read-only tab-switch proof: tests click
  Army/Navy/Air, verify the digest-visible roster changes, prove unavailable
  tab clicks are inert, and prove selected-tab state survives scrub A->B->A
  without carrying B-only rows. This does not cover row selection, drilldowns,
  scrolling, tooltips, mutating order controls, or side-by-side proof.
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
- `chrome.find` now has a first executable row-selection/drilldown slice:
  tests prove snapshot-derived catalog row selection/highlight, outside-row
  inert clicks, selected-row scrub A->B->A behavior without carrying B-only row
  data, and read-only `OpenView(core.state_panel, state_id)` commands for
  state and Victory Point rows. Text search/filtering, map recenter, country
  drilldown, and province-level drilldown remain open.
- `chrome.minimap` now has a first executable click-recenter slice: tests prove
  inside-fitted-map clicks emit `RecenterMap`, applying the command changes the
  bounded map-camera offset, letterbox clicks are explicit no-ops, outside
  clicks fall through, and no recenter is emitted without a live camera. Drag
  recenter, minimap screenshot proof, and real-game side-by-side proof remain
  open.
- `chrome.mapmodes` now has a first executable rail-click slice: tests prove
  core-backed rows emit `SetMapMode`, applying that command changes the core map
  digest/mode label, unavailable/data-proxy badges remain honest, outside clicks
  are ignored, and the non-core Intel row is not aliased. Active-state echo,
  Intel core support, legend hover, screenshots, and broader map/chrome parity
  remain open.
- Current statuses are conservative scaffold accounting: `open` for the
  descriptor-derived open/close gap, `open` for known tab target inventories
  and partial proven slices, and `not_proven` where no executable target
  inventory exists. They are not full interaction-parity claims.

Current executable report totals:

| Count | Value |
|---|---:|
| Registered views | 114 |
| Interaction contracts | 114 |
| Category rows | 1,026 |
| Target rows | 34 |
| Target rows with executable-proof notes | 34 |
| Implemented/proven target rows | Not representable yet |
| Intentionally inert mutation rows | Not representable yet |
| `not_proven` category rows | 903 |

Per-category report totals:

| Category | Category `open` | Category `not_proven` | Target rows | Target `open` | Target `not_proven` | Proof-note targets |
|---|---:|---:|---:|---:|---:|---:|
| `open_close` | 114 | 0 | 0 | 0 | 0 | 0 |
| `tab_switch` | 5 | 109 | 15 | 15 | 0 | 15 |
| `list_selection` | 2 | 112 | 16 | 16 | 0 | 16 |
| `scroll` | 0 | 114 | 0 | 0 | 0 | 0 |
| `hover_tooltip` | 0 | 114 | 0 | 0 | 0 | 0 |
| `drilldown` | 2 | 112 | 3 | 3 | 0 | 3 |
| `disabled_mutation` | 0 | 114 | 0 | 0 | 0 | 0 |
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
  `intel.intel_agency`, and `chrome.find`; tests prove `chrome.find`,
  `military.army`, `politics.politics`, `designers.division`, and
  `intel.intel_agency` are real registered views with non-empty target rows.
- [x] First executable read-only tab switches: `chrome.find` clicks switch the
  selected States/Countries/Victory Points catalog, `military.army` clicks
  switch the selected Army/Navy/Air roster, `politics.politics` clicks switch
  Government/National Spirits/Ideas & Laws/Autonomy content,
  `designers.division` switches Division/Army HQ where the focused template
  allows it, and `intel.intel_agency` switches Agency/Operations/Cryptology.
  These slices have inert non-pickable or unavailable tab proof and
  selected-tab scrub A->B->A proof for their tested views.
- [ ] Tab switching contract: every N2 tab bar must specify enabled, disabled,
  unavailable, selected, click target, persistent selected tab, and scrub
  behavior. `chrome.find`, `military.army`, `politics.politics`,
  `designers.division`, and `intel.intel_agency` have proof for their slices;
  the remaining N2 tab inventories remain open.
- [ ] List/grid selection contract: row factories render rows, but each row set
  still needs selectable rows, selected-row visual state, detail-pane update rules,
  and disabled-row handling. `chrome.find` has a first catalog-row
  selection/highlight proof only.
- [ ] Scroll contract: row factories render scrollbar geometry, but drag/wheel
  scrolling with hit clipping is not generally wired in cluster `on_input`.
- [x] Map pan/zoom: wheel zoom and right-drag pan are implemented in the main
  event loop when no higher view consumes input.
- [x] Map pick: left-click on a map miss resolves a province/state and opens
  `core.state_panel`; panel block rect prevents map picks behind panels.
- [ ] Map hover/readout contract: province/state hover, map-mode legend hover,
  map icon hover, and hover-to-tooltip are not globally proven.
- [ ] Minimap pick/recenter contract: N7 minimap renders a viewport panel and
  has executable click-to-recenter proof; drag recenter, screenshot proof, and
  side-by-side proof remain open.
- [x] Graph pan/zoom for focus, tech tree, and doctrine tree: direct
  `on_input` implementations consume drag/wheel, and tests show pan changes
  digest while no-refit rebind preserves camera.
- [ ] Graph node drilldown contract: focus/tech/doctrine/trait nodes still need
  click selection, read-only detail panes, and per-node tooltip contracts beyond
  the existing focus hover tooltip.
- [x] Combat bubble drilldown: `core.land_combat` bubble click opens the
  read-only combat panel; panel close and `Esc` close the panel state.
- [ ] General read-only drilldown contract: country flag, state/province, unit,
  army, fleet, air wing, combat row, focus, tech, doctrine, event, decision, war,
  trade, and logistics row drilldowns are not globally implemented/proven.
  `chrome.find` state and Victory Point rows have a first read-only
  `core.state_panel` drill command proof only.
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
  province/state pick on click miss, and dynamic scrub-driven map recolor.
- [x] `core.state_panel`: opens from map pick with selected state id; blocks map
  clicks inside panel; close leaf names are handled generically.
- [x] `core.focus_tree`: `F` toggles modal overlay; drag pan, wheel zoom, hover
  tooltip, and `Esc` close are implemented.
- [x] `core.land_combat`: `C` toggles overlay; bubble click opens combat panel;
  panel close and `Esc` close panel state; click misses fall through to map.
- [x] `core.topbar`: always-on chrome is registered and scrub-bound.

Likely missing interactions:

- [ ] `core.map`: hover readout, map-mode selector clicking, legend hover, map
  icon hover/selection, and side-by-side map interaction proof.
- [ ] `core.state_panel`: building row scroll, province/state drilldown links,
  tooltip coverage, and close-button proof for the real `.gui` leaf.
- [ ] `core.topbar`: clickable topbar drilldowns, hover frames, topbar tooltips,
  and disabled read-only behavior for any game-action buttons.

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
- [x] Data-bound rows and widgets render: politics tabbed subwindows, power
  balance track, character roster, advisor rows, occupied country/state rows.
- [x] Mutating verbs are documented as inert: create/switch faction, change law,
  assign garrison, switch character.

Likely missing interactions:

- [ ] Character/advisor/law/idea/occupied-row selection and detail-pane updates.
- [ ] Scrollbar drag/wheel for long idea, character, advisor, and occupation lists.
- [ ] Hover tooltips for parties, ideas, laws, power-balance markers, character
  traits, advisor roles, resistance/compliance, and garrison/law gaps.
- [ ] Explicit disabled-state inventory for every mutating government control.

Tests/proof needed:

- [ ] Per-view interaction tests for tab clicks, row clicks, scroll clipping, and
  inert mutating controls.
- [ ] Screenshots for selected rows, disabled laws/advisors, hover frames, and
  unavailable badges.

### ProductionConstructionDecisions

Views: `production.production`, `production.construction`,
`production.decisions`.

Implemented interactions:

- [x] Menubar open and generic close path.
- [x] Production, construction, and timed-decision row factories render with
  no-fabrication display atoms and scrollbar geometry.

Likely missing interactions:

- [ ] Row selection for production lines, construction queue entries, and timed
  decisions.
- [ ] Scrollbar drag/wheel for long production/queue/decision lists.
- [ ] Read-only drilldown from rows to equipment/building/decision details.
- [ ] Disabled/inert proof for line management, factory assignment, building
  queue edits, and decision-click actions.
- [ ] Tooltips for resources, efficiency, factory allocation, queue progress,
  decision timers, and unavailable decision-status gaps.

Tests/proof needed:

- [ ] List selection and scroll tests with clipped rows.
- [ ] Click-no-op tests for all mutating production/construction/decision buttons.
- [ ] Side-by-side screenshots for a long production list and active timed
  decision.

### ResearchTechDoctrine

Views: `research.tech_tree`, `research.doctrine_tree`, `research.research`.

Implemented interactions:

- [x] `research.tech_tree`: graph drag pan, wheel zoom, `Esc` close, camera
  preservation on no-refit scrub.
- [x] `research.doctrine_tree`: graph drag pan, wheel zoom, `Esc` close, camera
  preservation on no-refit scrub.
- [x] `research.research`: active research slot panel renders real `.gui` chrome
  and slot rows.

Likely missing interactions:

- [ ] Tech/doctrine node click selection and read-only detail panes.
- [ ] Tech/doctrine node hover tooltips with localized cost, prerequisites,
  ahead-of-time, bonuses, and unavailable computed lines.
- [ ] Research slot row selection, slot detail, cancel/research mutation buttons
  rendered disabled/inert.
- [ ] Tree category/folder/tab switching if present in the game surface.

Tests/proof needed:

- [ ] Node pick tests, tooltip snapshot tests, and detail-pane scrub tests.
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
- [x] Fleet, task force, air group, air selection, deployment, divisions,
  railway gun, and equipment overview rows render through registered templates.
- [x] Mutating verbs such as create army, deploy, draw order, reorg, and ability
  use are documented as inert/unavailable.

Likely missing interactions:

- [ ] Row selection and drilldown for army, fleet, task force, ship, air wing,
  division, railway gun, deployment, and equipment overview rows.
- [ ] Scrollbar drag/wheel for long OOB, deployment, and equipment lists.
- [ ] Battle-plan tools must be visibly disabled/inert and must not imply orders
  can be drawn in replay.
- [ ] Air-wing reorg, carrier air wing, theatre selector, and ability list need
  explicit read-only/mutation target inventories.
- [ ] Hover tooltips for units, missions, equipment, templates, commanders,
  supply/attrition, orders, and unavailable computed fields.

Tests/proof needed:

- [ ] Per-view row pick/scroll tests for all 15 views.
- [ ] Mutation click-no-op tests for order/deploy/reorg/ability controls.
- [ ] Side-by-side proof for selected army/fleet/air-wing/detail workflows.

### Designers

Views: `designers.tank`, `designers.plane`, `designers.division`,
`designers.equipment`, `designers.equipment_upgrade`.

Implemented interactions:

- [x] Menubar open and generic close path.
- [x] N4 module-slot layout renders equipment/variant/template data.
- [x] Division designer renders the N2 normal/HQ tab bar visually.
- [x] Division designer normal/HQ tabs switch via read-only click when the
  focused template is an Army HQ template; non-HQ Army HQ clicks are inert, and
  selected-tab state has scrub A->B->A proof.
- [x] Computed combat stats, costs, and other hard-limit panels are badged
  unavailable rather than fabricated.

Likely missing interactions:

- [ ] Module-slot hover tooltips and click selection for read-only inspection.
- [ ] Upgrade slider interaction must be disabled/inert but still visually
  accounted for.
- [ ] Variant/template selector row selection and drilldown are not proven.
- [ ] Tooltips for modules, stat contributions, upgrade tracks, unavailable
  computed stats, and production-resource gaps.

Tests/proof needed:

- [ ] Slot pick/hover tests and selected-slot screenshot cases.
- [ ] Click-no-op tests for upgrade sliders, module add/remove buttons, save,
  duplicate, and production controls.
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

Likely missing interactions:

- [ ] Leader row selection must update selected leader detail and trait tree.
- [ ] Trait node hover and read-only drilldown details are not proven.
- [ ] Trait pick buttons must be disabled/inert, not silently clickable.
- [ ] Scrollbar drag/wheel for large leader, advisor, ace, and trait lists.
- [ ] Assignment/command group/portrait interactions need read-only drilldown or
  disabled-control classification.

Tests/proof needed:

- [ ] Row-to-detail tests for generals, admirals, aces, and command groups.
- [ ] Trait hover/disabled-pick tests and tooltip snapshots.
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
- [x] Ingame menu save/load/resign/quit verbs and popup/event mutation verbs are
  documented inert/unavailable in replay.

Likely missing interactions:

- [ ] Unit/division row selection and detail panes.
- [ ] Combat log row selection into combat details; naval combat/result row
  drilldown into ship/fleet details.
- [ ] Event option and popup button hover/disabled/inert behavior.
- [ ] Scrollbar drag/wheel for combat logs, losses, logistics, unit rows, and
  popup/event lists.
- [ ] Tooltips for resources, fuel, supply, losses, event options, popups,
  naval region data, and unavailable computed combat stats.
- [ ] `logistics.destroyed_ships` is an empty stub; its interaction contract must
  stay honest about no rows and no drilldown.

Tests/proof needed:

- [ ] Row selection and scroll tests across logistics, unit, combat, and event
  views.
- [ ] Click-no-op tests for ingame menu and popup/event mutation controls.
- [ ] Side-by-side screenshots for unit detail, combat log, naval result, and
  representative popup/event cards.

### ChromeOutlinerMaps

Views: `chrome.clock`, `chrome.minimap`, `chrome.alerts`, `chrome.find`,
`chrome.rightclick`, `chrome.mapmodes`, `chrome.mapicons`.

Implemented interactions:

- [x] Menubar open and generic close path.
- [x] Clock renders exact hour from `ctx.date`.
- [x] Minimap renders chrome plus viewport outline from the current map camera.
- [x] Minimap click-to-recenter emits a read-only `RecenterMap` command from
  fitted-map clicks, ignores outside clicks, consumes letterbox no-ops, and has
  bounded camera-offset tests.
- [x] Find renders N2 category tab/list visually with catalog rows.
- [x] Find States/Countries/Victory Points tabs switch the bound read-only
  catalog and preserve selected tab across scrub A->B->A.
- [x] Find catalog rows support read-only selection/highlight, ignore outside-row
  clicks, preserve valid selected-row state across scrub A->B->A, and state/VP
  rows emit `core.state_panel` drill commands.
- [x] Map modes rail renders all mode slots and badges active-mode data
  unavailable in this cluster view.
- [x] Map-mode rail rows backed by existing core `render::modes::MapMode`
  variants emit read-only `SetMapMode` commands; unavailable/data-proxy badges
  remain honest and the non-core Intel row is not aliased.
- [x] Map icons render placed exact/proxy icons where anchored and badge the rest.

Likely missing interactions:

- [ ] Minimap drag recenter and side-by-side click/drag proof.
- [ ] Find text entry, filtering, country drilldown, province-level VP drilldown,
  and map recenter to selected state/country/VP.
- [ ] Map-mode rail active-state echo, non-core Intel mode support, legend hover,
  screenshot proof, and broader core map-mode parity.
- [ ] Alerts drilldown and alert hover tooltips.
- [ ] Right-click menu target-specific options and click-no-op proof for mutating
  orders/actions.
- [ ] Map icon hover/read-only drilldown for VP, capital, resistance, air base,
  combat, unit counter, and unavailable icon families.

Tests/proof needed:

- [x] Minimap click-recenter input tests after wiring.
- [ ] Minimap drag-recenter input tests.
- [ ] Find query/row-selection/recenter tests and screenshot proof.
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
  descriptor route context, nine category statuses, and registry coverage tests.
- [x] Build the first target-level contract slice: registered view id,
  target category, stable tab target id, label, status, and note for known N2
  `tab_switch` targets.
- [ ] Expand the interaction target inventory beyond the current tab slice:
  `.gui` leaf/widget target id, read-only behavior, mutation status,
  implemented status, and proof link for tabs, rows, scrollbars, hover targets,
  drilldowns, and disabled mutation controls.
- [x] Add a registry-level interaction coverage report: registered view count,
  contract count, category-row count, target count, missing/not-proven category
  count, and per-category status/target counts are executable and tested. The
  implemented/proven target count and intentionally inert mutation count remain
  explicitly not representable until the status model grows those closed states.
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
   That means tabs, rows, scrollbars, and drilldowns should be treated as missing
   until implemented or proven through generic host behavior.
2. N2 tabs are now partially executable as target inventory rows, but click
   switching is not generally wired or tested.
3. Row factories render large lists and scrollbar art, but interactive scrolling,
   row selection, selected visual state, and detail-pane updates are not generally
   wired.
4. Tooltip parity is almost entirely open. Focus has a direct hover tooltip;
   generic HOI4-style tooltip resolution/composition remains a checklist blocker.
5. Mutating controls are often documented as inert, but there is no per-control
   disabled-state inventory or click-no-op proof across the 114-view surface.
6. Map and chrome parity still lacks minimap drag recenter, map-mode rail
   clicks, hover readouts, map icon interactions, screenshot harness, and
   side-by-side real the game proof.
7. Scripted overlays are explicitly synthetic render plans, so their visual and
   interaction parity risk is higher than parsed `.gui` windows until compared
   side by side.
