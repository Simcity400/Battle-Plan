# Master Progress - Native Battle Plan App

This is the only current progress and remaining-work document for the native
replay app. It replaces the prior separate checklist and handoff documents.

The target is a read-only Battle Plan app that can be used like the real game UI
for inspection, with no ability to mutate the game.

## Current Verdict

- Engineering DoD is met: 114 registered/menu-reachable views, scrub-safe host
  rebinding, no-fabrication guardrails, wired map/weather/render stack,
  `.battleplan` container support, and passing coverage checks.
- Product parity is not complete.
- Runtime capture architecture is changing: the replay recorder must use the
  live host `hoi4.exe` process as its only authoritative runtime source through
  an in-process collector DLL. Saves are optional development fixtures, not
  recording or replay requirements.
- The app is a replay/inspection surface, not playable HOI4. The standard is
  still one-to-one inspection: every real read-only interaction, tooltip, visual
  state, map overlay, and recorded data surface must either work faithfully or be
  explicitly badged with evidence.
- Do not split current progress into separate handoff, checklist, or status
  documents. Update this file when status changes.

## Source Of Truth

- This file: current status, remaining work, acceptance gates, and user
  dependencies.
- `coverage_audit/memory_manifest.json`: machine-readable data completeness
  ledger.
- `coverage_audit/COVERAGE_REPORT.md`: readable data coverage report and
  extraction evidence.
- `replay_ui/INTERACTION_PARITY_INVENTORY.md`: interaction inventory and
  per-view evidence.
- `replay_native/app/src/views/interaction_contract.rs`: executable interaction
  contract scaffold.
- `replay_ui/BINDING_MAP_COVERAGE.md` and `replay_ui/BINDING_MAP_*.md`: UI
  binding evidence and no-fabrication notes.
- `replay_ui/HOI4REPLAY_CONTAINER_SPEC.md`: portable recording container
  contract.
- `IN_PROCESS_COLLECTOR_PLAN.md`: live-process collector implementation plan.
- `AGENTS.md`: current workflow and safety rules.

## Current Status Snapshot

- Views: 114 registered/menu-reachable native app views, 86 templates, all
  engineering-DoD criteria met.
- Binding coverage: 115 in-game source coverage rows accounted for in
  `BINDING_MAP_COVERAGE.md`; this is the source-coverage universe, not the
  openable native registry count.
- Data manifest: 273 rows total.
- Data rows closed: 149 `closed-by-reader` plus 14 `closed-by-parser`.
- Data rows still closeable/provable: 26 `needs-wartime-save`, 5
  `open-live-extractable`, and 15 `open-computed`.
- `needs-wartime-save` is now a legacy fixture label: useful for development
  proof, but not an acceptable runtime dependency. Runtime closure requires live
  collector output or an explicit unavailable badge.
- Permanent walls: 2.
- Excluded rows: 62.
- Interaction coverage currently has a broad scaffold and many first real
  slices: 129 target rows with executable-proof notes are inventoried, while
  959 category rows remain `not_proven`. Product parity is open until every
  read-only interaction and every mutating-control disabled state is proven.

## Done Or Proven

- Native Rust/wgpu app architecture, distributed view registry, generic window
  host, scrub/rebind, `Captured<T>` no-fabrication plumbing, and host-level scrub
  integration.
- All 114 registered views are menu-reachable and scrub-safe.
- Real `.gui` ingestion and binding maps exist for the app view universe.
- Map terrain shader path, borders, snow, mud, frozen seas, season color,
  animated water/rivers, and dynamic map state are wired.
- Dynamic weather, mud/snow, and frozen-sea drivers feed the map.
- `.battleplan` writer/reader/container support exists, with selectable cadence
  modes, manual/paused one-shot capture, hour de-duplication, hour-crossing
  rejection, skipped-hour diagnostics, timing diagnostics, and manifest cadence
  metadata.
- Static DEF parsers and coverage checker are current.
- First interaction scaffold and many first slices exist for tab switching, row
  selection, scroll, map/minimap actions, hover tooltips, read-only navigation,
  and disabled mutation no-op proof.
- War overview war-row selection and diplomacy country-row selection now have
  measured-key/TAG proof, scrub fail-closed tests, and interaction-contract
  target rows; participant/relation rows and broader diplomacy/war parity remain
  open.
- Insignia catalog buttons now have explicit replay-disabled no-op proof and an
  interaction-contract target row; broader insignia controls and visual parity
  remain open.
- Logistics overview resource-row selection and factual resource-hover tooltips
  now have stable-key/no-fabrication/fail-closed proof and interaction-contract
  target rows; materiel rows, stockpile/demand/history tooltip detail, and
  broader logistics parity remain open.
- The save-persisted `countries.*.strategic_navy` block now folds through
  `country_extra` into `countries.*.extra.strategic_navy`; live memory offsets
  for those values remain fail-closed and are not claimed.
- DLL-native scheduled `buildings_live` is proven on the live static map spine:
  the collector reads state/province `CBuilding` vectors on hourly frames,
  folds into `states.*.buildings` and `provinces.*.buildings`, and passed a
  no-cold 12-game-hour proof on pid 43412 with 13 verified scheduled building
  frames, 1081 state rows and 10272 province rows per scheduled frame, non-empty
  state/province building maps, and zero worker-poll building payloads.
- DLL-native scheduled `diplomacy_live` is proven for faction membership and
  war/casualty rows: the collector reads `CCountry+0x598` faction groups on
  hourly frames, folds into `countries.*.diplomacy.faction_group` and
  `diplomacy_live.faction_members`, and now uses a preflight
  `war_relation_bridge` so the DLL hot path can revalidate pinned war rows and
  reread casualties on scheduled frames. The faction proof passed a no-cold
  12-game-hour run on pid 43412 with 14 scheduled diplomacy frames, 4984
  diplomacy country rows, 356 countries and 9 faction groups per scheduled
  frame, and zero worker-poll diplomacy payloads. The war/casualty proof
  `replay/snapshots/_collector_session_diplomacy_war_gate/20260629T163650Z_pid43412`
  advanced 12 game hours in 43.203 wall seconds, verified replay output,
  supplied 1033 bridge rows via live tag-table identity, observed 14 scheduled
  diplomacy war frames, captured up to 1033 `war_relations` per frame, folded
  1022 country `at_war` rows and 1022 country casualty rows, observed max
  per-frame casualty sum 56475731, and kept diplomacy payloads off worker-poll
  frames.
- DLL-native active country leaders are proven through scheduled
  `politics_live`: the collector validates
  `CPoliticalStatus -> ruling_party -> CCountryLeader -> CCharacter`, accepts
  only singleton ruling-party leader vectors, and folds verified rows into
  `countries.*.commanders.leader`. The proof
  `replay/snapshots/_collector_session_politics_leader_gate/20260629T171216Z_pid43412`
  advanced 12 game hours in 39.328 wall seconds, stored 13 verified snapshots,
  emitted 13 scheduled `politics_live` frames, captured 4095 active-ruler rows
  with up to 315 per scheduled frame, and kept politics payloads off
  worker-poll frames. Multi-entry leader vectors remain intentionally omitted
  until active-element disambiguation is proven.
- DLL-native construction queue capture is proven through scheduled
  `production_live`: the collector reads the production manager general-lines
  vector at `CCountry+0xF68 -> mgr+0x70`, accepts only type-59 construction
  lines, and folds verified rows into
  `countries.*.production.construction_queue`. The proof
  `replay/snapshots/_collector_session_construction_gate/20260629T172526Z_pid43412`
  advanced 12 game hours in 40.219 wall seconds, stored 13 verified snapshots,
  emitted 14 scheduled `production_live` frames, captured 3661 construction
  queue rows with up to 262 rows per scheduled frame and 29 rows per country,
  confirmed Portugal queue rows in the replay store, and kept production
  payloads off worker-poll frames. Target state/location remains intentionally
  unavailable until the target handle is pinned.
- DLL-native active research slot identity is proven through scheduled
  `tech_live`: the collector reads the bounded `CTechnologyStatus+0xA0`
  research-slot vector and uses the cache-first live tech-definition bridge to
  resolve `tech_name` without saves. Moving live progress is now proven from
  `CResearchSlot+0x18 -> CTechnology+0x198` as
  `live_progress_raw/live_progress_units`, and base denominator is now proven
  from `CTechnology+0x160 -> CTechnologyTemplate+0x3e0` as
  `base_required_points_raw/base_required_points_units`; the older
  `progress_units` remains source-badged but is not accepted as moving
  progress. The combined proof
  `replay/snapshots/_collector_session_research_denominator_gate/20260629T192125Z_pid43412`
  advanced 36 game hours in 48.125 wall seconds with a no-cold cache hit,
  emitted 37 scheduled `tech_live` frames, captured 15687 named active
  research-slot rows and 15687 live base-denominator rows with up to 424 rows
  per frame, verified replay output, kept research payloads off worker-poll
  frames, produced 346 derived ETA rows after a positive daily progress delta,
  and recorded the static denominator audit `{tech_count=557,
  no_research_cost_count=10, base_research_points_per_cost=110}`. Effective
  cost/speed modifiers remain unclaimed until separately pinned.
- DLL-native raw `land_combats` lifecycle capture is proven at speed five for
  the raw replay stream: the 2026-06-29 pid 43412 proof
  `replay/snapshots/_collector_session_diplomacy_gate/20260629T152734Z_pid43412`
  advanced 12 game hours in 48.328 wall seconds, verified the replay container,
  passed the raw `land_combats` proof gate, and observed 56 combat-started, 6695
  combat-updated, and 11 combat-ended events without heavyweight payloads on
  lightweight frames. UI ingestion and richer battle-detail enrichment remain
  open.
- DLL-native raw `air_wings` telemetry capture is proven at speed five for the
  raw replay stream: cold preflight now live-discovers the heap arena before
  air-wing body enumeration, stored 653 rows in the cache, and the no-cold proof
  `replay/snapshots/_collector_session_air_wing_gate/20260629T164933Z_pid43412`
  advanced 12 game hours in 52.891 wall seconds, verified the replay container,
  supplied 652 cached bridge rows, passed the raw `air_wings` proof gate,
  observed 652 `air_wing_observed` and 9128 `air_wing_updated` events, and
  reported zero raw gaps or heavyweight payloads on lightweight frames. Exact
  displayed air-superiority percentage remains an open computed layer.
- Anchored land-counter stacks now compose portrait and formation-insignia
  sprites into the native fixed-footprint `UnitCounter` render plan, preserving
  four stable asset slots, sprite frame identities, and explicit missing-asset
  placeholders.
- Normal land and railway-gun counters now account for attrition as anchored
  unavailable badges, preserving real stack anchors without claiming the
  runtime attrition computation.
- Fleet and task-force insignia frame fallback now uses the native
  `GFX_theatre_task_force_shield` strip, while explicit art and missing sprite
  misses remain deterministic.
- Asset/counter and map-overlay work are partially landed, not complete.

## Remaining Work

### 1. In-Process Collector Runtime Source

- Replace the hourly attach/polling proof with a long-running in-process worker
  or reviewed hourly-safe hook.
- Keep build-gated attach/detach, heartbeat transport, recorder ingestion, and
  `.battleplan` provenance fail-closed.
- Reproduce live economy rows after the hourly timeline is stable. Daily
  MIL/CIV/NAV-style output may be useful as a rollup, but it does not satisfy
  replay cadence.
- Run a longer live unpaused collector-backed recorder proof at intended
  multiplayer speed for the hourly cadence.
- Prove either no scheduled in-game hours are missed, or that skipped scheduled
  hours are explicitly recorded in the `.battleplan` manifest.
- Reopen the resulting `.battleplan` in the native app.
- Promote the proven raw `land_combats` stream into app ingestion and only add
  Stage-4 performance work if longer soaks show recorder pressure.

### 2. Interaction Parity

- Expand `INTERACTION_PARITY_INVENTORY.md` and
  `replay_native/app/src/views/interaction_contract.rs` beyond the current
  partial target slices.
- Implement and test every read-only tab, subtab, row selection, drilldown,
  detail-pane update, scrollbar, drag-scroll surface, text filter, map mode
  selector, legend, and hover readout.
- Prove every mutating the game control is inert or disabled in replay mode, with a
  visible/badged state and unchanged replay data.
- Add per-view interaction tests, not only bind/scrub tests.

### 3. Tooltip And Localization Parity

- Expand the passive tooltip resolver to game-style frames, timing, anchoring,
  localization, and markup behavior.
- Build per-domain tooltip composers for topbar, map, politics, production,
  construction, research, military, diplomacy, intel, logistics, state panel,
  counters, alerts, and event/popup surfaces.
- Parse or evaluate the non-mutating trigger/effect subset needed for faithful
  tooltip lines.
- Badge omitted computed tooltip lines instead of silently dropping them.
- Add stable tooltip fixtures and side-by-side game comparison cases.

### 4. Visual And Screenshot Parity

- Extend the screenshot harness beyond registry manifest plumbing to actual
  captures for every registered view, representative tabs, rows, tooltips, and
  map modes.
- Capture matching the game reference screenshots for the same states/windows.
- Add visual diff thresholds and a reviewed exception list.
- Verify fonts, icon frames, disabled states, hover frames, scrollbars, clipping,
  modal layers, counters, map modes, and tooltip framing.

### 5. Map Overlay Parity (#26)

- Finish real the game route/front extraction coverage.
- Finish exact front, battleplan, trade-route, weather-phenomenon, label, and
  vector-arrow rendering semantics.
- Prove exact Air/Naval per-province parity beyond the current strategic-region
  proxy.
- Capture side-by-side map-mode screenshots and track exceptions.

Current partial state: native labels/vector arrows/trade planning exist; semantic
line metadata exists; measured route/front/battle-plan polylines can ingest as
exact; malformed or missing anchored geometry fails closed; and live
`oob_command.member_div_ids` can anchor type-5 battle-plan arrows.

### 6. Asset And Counter Parity (#50)

- Finish portrait and insignia/emblem extraction and rendering.
- Render full GUI unit-counter chrome/art.
- Add commander selection and complete navy, air, rail, transport, HQ, supply,
  attrition, formation, and strength/org counter indicators.
- Reject placeholders and account sprite misses deterministically.
- Capture side-by-side asset/counter visual proof.

Current partial state: portrait/insignia resolution, explicit sprite-miss
accounting, selected-formation insignia binding, land counter logical entities,
fixed-slot native `UnitCounter` plans for anchored land stacks, anchored
unavailable attrition slots, task-force shield-frame binding for fleets/task
forces, and several owner/anchor-gated joins exist.

### 7. Open Live/Static Data Rows

These are not user-blocked and should be closed or proven unavailable:

- `all_playthrough_data`
- `countries.*.logistics`
- `countries.*.upgrades`
- `mapicon.general_portrait_on_counter`
- `ui_open.insignia_emblem_assets`

Current blockers:

- `all_playthrough_data`: root-blocked; an instance was value-anchored, but no
  portable gamestate/root chain is proven.
- `countries.*.logistics`: root row is distinct from save-parsed
  `countries.*.economy.logistics`; current evidence says the host-only
  serializer behavior cannot be emitted without fabricating player identity.
- `countries.*.upgrades`: do not close from intelligence-agency or
  equipment/design upgrade maps; the root country container still needs a real
  allocated-container proof.
- `mapicon.general_portrait_on_counter`: partial anchored portrait binding and
  fixed-slot native counter-plan composition exist; full unit-counter placement
  and side-by-side visual parity remain open.
- `ui_open.insignia_emblem_assets`: selected-formation binding/fail-closed
  behavior and fleet/task-force shield-base correctness exist; complete gfx
  asset extraction/render parity remains open.

### 8. Computed/Visual Rows

These need faithful recomputation, visual proof, or a reviewed permanent badge:

- `map.weather_phenomenon_overlay`
- `map.front_lines_overlay`
- `mapicon.attrition_flag`
- `world_tension`
- `research ETA / live speed`
- `weather temperature formula / RNG / phenomenon-name strings`
- `combat detail stat rows`
- `ui_computed.designer_combat_stats`
- `ui_computed.naval_air_per_province_exactness`
- `ui_computed.occupation_equilibrium_garrison_need`
- `ui_computed.power_balance_trend_marker`
- `ui_computed.operation_chance_intel_bands_capit`
- `ui_computed.airwing_effective_planes_denom`
- `ui_computed.war_participant_score`
- `ui_computed.combat_battle_detail`

Current partial state: `mapicon.attrition_flag` is accounted in normal land and
railway-gun counter rows as an anchored unavailable badge only; faithful
terrain/supply/weather recomputation or a proven memory field is still open.

### 9. Needs User Fixture Or Wartime State

These rows are empty in the current peacetime USA-1936 fixture and need a
mid-war/eventful paused save. Content-match by live date and ROOT tag, not
filename or mtime.

- `peace_conference`
- `history.sunk_ship`
- `ships_built`
- `countries.*.incoming_diplomatic_action`
- `countries.*.civil_war_target`
- `countries.*.expeditionaries_sent`
- `countries.*.invasion_report`
- `countries.*.dirty_controlled_states`
- `countries.*.removed_controlled_province`
- `countries.*.dynamic_revolution_tag`
- `states.*.active_targeted_modifier`
- `states.*.previous_owner`
- `states.*.last_strategic_bombing`
- `states.*.is_border_conflict`
- `combat_log`
- `combat_data_entry`
- `selection_groups`
- `countries.*.government_in_exile`
- `countries.*.collaboration_government`
- `countries.*.player_settings`
- `countries.*.strategic_region_data`
- `ui_live.alerts_ledger`
- `ui_live.war_participant_casualties`
- `ui_live.operative_code_name`
- `ui_live.admiral_fleet_composition`
- `ui_live.ship_owner_wing_ace_join`

### 10. Hard Walls

Keep these honestly badged unless new evidence changes the classification:

- Exact per-province supply tooltip number.
- True Steam real names.

## User Help Needed

- A longer eventful live session when the hourly worker/hook and economy readers
  are ready to test.
- Optional: a mid-war/eventful paused save remains useful as a development
  comparison fixture, but it is no longer a runtime requirement.

## Acceptance Gates

The app is product-complete only when all applicable gates pass:

- Full Rust workspace tests pass.
- Full Python pytest suite passes.
- `coverage_audit/check_coverage.py` passes.
- In-process collector attaches only to supported builds, emits heartbeat and
  live data frames, and detaches cleanly.
- Collector-backed `.battleplan` recorder captures a live session without save
  input, finalizes, and reopens in the native app. Narrow clock/root-probe proof
  passed on 2026-06-28 with four monotonic live snapshots and byte-identity
  verification; country-identity proof also passed with 439 live country rows
  per snapshot. Transport proof also passed with monotonic producer sequence,
  CRC32/byte metadata, zero duplicate sequences, and byte-identity verification.
- Hourly cadence proof passed on 2026-06-28: seven consecutive live in-game
  hours, all deltas exactly `1`, zero skipped hours, 439 country rows per
  snapshot, packed and byte-verified as `.battleplan`.
- Hourly worker proof passed on 2026-06-28: one bounded in-process worker attach
  streamed multiple frames over one named pipe, stored seven consecutive hourly
  snapshots, skipped zero hours, and byte-verified the `.battleplan`.
- Guarded worker proof passed after restricting worker mode to `hoi4.exe`: four
  consecutive hourly snapshots, zero skipped hours, byte-identity verification.
- Collector profile preflight passes against the live observed build: exact
  executable hash, profile label, root-probe sentinels, expected addresses,
  domains, 439 country rows, and transport metadata.
- Recorder proves no missing in-game hours for the configured cadence, or
  records explicit skipped-hour diagnostics. For proof runs requested with
  `--until-game-hours`, missing hourly coverage is command failure unless the
  run is explicitly marked diagnostic-only.
- Scheduled `buildings_live` proof passed on 2026-06-29:
  `replay/snapshots/_collector_session_buildings_gate/20260629T150353Z_pid43412`
  advanced 12 game hours in 45.734 wall seconds, stored 13 verified snapshots,
  observed `buildings_live` on all 13 scheduled frames, captured non-empty state
  and province building maps, and reported zero heavy payloads on lightweight
  frames.
- Scheduled `diplomacy_live` faction proof passed on 2026-06-29:
  `replay/snapshots/_collector_session_diplomacy_gate/20260629T152734Z_pid43412`
  advanced 12 game hours in 48.328 wall seconds, stored 13 verified snapshots,
  observed `diplomacy_live` on 14 scheduled frames, captured 4984 diplomacy
  country rows, measured up to 356 countries and 9 faction groups per scheduled
  frame, verified the replay container, and reported zero heavy payloads on
  lightweight frames.
- Scheduled `diplomacy_live` war/casualty proof passed on 2026-06-29:
  `replay/snapshots/_collector_session_diplomacy_war_gate/20260629T163650Z_pid43412`
  advanced 12 game hours in 43.203 wall seconds, stored 13 verified snapshots,
  supplied 1033 live tag-table bridge rows, observed 14 scheduled diplomacy war
  frames, measured max `war_relations = 1033`, folded 1022 country `at_war`
  rows and 1022 country casualty rows, verified the replay container, and
  reported zero diplomacy payloads on lightweight frames.
- Raw `land_combats` lifecycle proof passed on 2026-06-29:
  `replay/snapshots/_collector_session_diplomacy_gate/20260629T152734Z_pid43412`
  advanced 12 game hours in 48.328 wall seconds, verified the replay container,
  saw raw `land_combats`, and observed 56 `land_combat_started`, 6695
  `land_combat_updated`, and 11 `land_combat_ended` events.
- Raw `air_wings` telemetry proof passed on 2026-06-29:
  `replay/snapshots/_collector_session_air_wing_gate/20260629T164933Z_pid43412`
  advanced 12 game hours in 52.891 wall seconds, verified the replay container,
  supplied 652 cached bridge rows, saw raw `air_wings`, observed 652
  `air_wing_observed` and 9128 `air_wing_updated` events, and reported zero raw
  gap frames.
- Every registered view opens from the in-app menu.
- Every registered view has an interaction contract.
- Every read-only interaction in each contract is implemented and tested.
- Every mutating game interaction is disabled/inert and visually accounted for.
- Every major hover target has a tooltip or a tracked tooltip gap.
- Screenshot harness covers every view, key tabs, tooltips, and map modes.
- Side-by-side real the game comparison is reviewed for the full first-pass view set.
- All manifest rows are closed, fixture-blocked, or proven/badged with evidence.
- The user's real in-game test mismatches are fixed or recorded here with a
  concrete owner and next action.
