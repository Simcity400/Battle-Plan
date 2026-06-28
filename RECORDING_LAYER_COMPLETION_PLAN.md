# Recording Layer Completion Plan

Date: 2026-06-28

## Purpose

This document is the working plan for finishing the recording layer: the part of
the project that turns a live the game multiplayer session into complete, durable,
scrubbable replay data for the app.

The target is not a stats sampler. The target is a full replay/review record:
the host machine's live `hoi4.exe` state over time, from every country
perspective, with map state, units, economy, wars, diplomacy, focuses, research,
combat, logistics, UI-relevant state, and enough provenance to know exactly what
was measured versus unavailable.

## Hard Rules

- Runtime recording has one authoritative source: the live host `hoi4.exe`
  process through the in-process collector.
- Saves are development fixtures only. They can discover layouts, validate
  fields, and build test cases, but they cannot be required to record, replay,
  bridge, or complete a live session.
- Daily data is not replay coverage. Daily rows can exist as auxiliary rollups,
  but the replay floor is hourly-or-better live frames.
- Speed five is a first-class acceptance target. Any architecture that only works
  at speed two is not done.
- No field is guessed. Missing live data is `null`, an unavailable badge, or a
  manifest gap.
- The collector must be read-only: no `WriteProcessMemory`, no game-state
  mutation, no unsafe calls into mutating game logic.
- Unknown builds, failed sentinels, torn reads, invalid roots, and transport
  failures fail closed.
- No heap-wide scans in the hot recording loop. Whole-heap discovery is research
  scaffolding only.

## Current Baseline

The project already has several important pieces.

- `DESIGN.md` defines the current architecture: a live in-process collector plus
  external recorder, with saves retained only as development validators.
- `IN_PROCESS_COLLECTOR_PLAN.md` records the DLL direction, safety contract, and
  the speed-five proof.
- `replay/schema.py` and `replay/SCHEMA.md` define the durable snapshot schema,
  provenance model, no-fabrication rules, and event diff contract.
- `coverage_audit/memory_manifest.json` is the machine-readable data universe:
  273 rows covering save-persistent state, UI/render inputs, computed rows, walls,
  and excluded internal state.
- `coverage_audit/COVERAGE_REPORT.md` explains the same coverage ledger in human
  terms.
- `live_capture/` has the Python recorder, assembler, durable store,
  `.battleplan` writer, collector frame validation, cadence diagnostics, and the
  current external-memory reader registry.
- `replay_native/collector/` has the Rust DLL proof: attach, build/profile
  sentinels, clock/date roots, country tag-table rows, named-pipe frames,
  producer sequence ids, bounded worker mode, and speed-five proof behavior.
- `memory_reader/` has a large amount of reverse-engineering work that can be
  ported into the collector as root-anchored live readers.
- `save_oracle/` has broad parsers for development fixtures and validation.

The most important proof so far is the user-confirmed speed-five worker run:
10 ms collector sampling captured 12 consecutive hourly deltas with zero skipped
hours, monotonic transport sequence, and byte-verified `.battleplan` packing.
That proves the clock/identity transport shape can keep up at speed five. It does
not prove the full data layer yet.

Current recorder-facing bridge status:

- `live_capture.collector_session` wraps repeated bounded DLL workers as one
  logical session with `session_start`, `session_stop`, `session_manifest`, a
  durable store, and a finalized `.battleplan`.
- A live vanilla run on 2026-06-28 processed 2 bounded worker bursts: 359 raw
  collector observations became 10 unique hourly replay snapshots, advanced 9
  in-game hours with zero skipped hours, verified the replay container, and left
  the game process responding.
- A 3-burst live run processed 800 raw observations into 20 unique hourly replay
  snapshots, advanced 19 in-game hours with zero skipped hours, and verified the
  replay container.
- Bounded worker sequence ids restart per worker burst. Manifests now preserve
  the global reset evidence while treating burst-local monotonic sequence as the
  recorder health signal.
- If a later bounded worker fails after earlier valid frames, the session writes
  the verified prefix replay and manifest, then fails closed with the attach
  error. This is still a bridge; the final target remains a true single
  start/stop collector session.
- The collector tag-table fallback now emits `countries.*.identity.tag_db_id`
  from live memory without a country-object heap scan. This is the safe first
  identity bridge needed by later state, province, division, and country-object
  joins. It is not yet the full country-scalar bridge.

## Definition Of Done

The recording layer is complete when all of these are true:

1. A supported vanilla build can record a live session at speed five with no
   runtime save input.
2. The recorder produces a `.battleplan` that opens in the native app and scrubs
   all captured surfaces from one global timeline.
3. The core timeline has hourly-or-better snapshots with explicit skipped-hour
   diagnostics.
4. Combat and movement are captured at the highest sustainable cadence, with
   honest confidence/gap metadata for sub-sample events.
5. Every non-excluded manifest row is either live collector fed, derived from
   live collector inputs, closed by static definitions/assets, or badged as a
   proven unavailable/wall.
6. The collector is build-gated, profile-gated, root-sentinel validated, and
   crash-contained enough for normal host use.
7. All domains carry source provenance, domain ids, collector version, executable
   hash, mod/static-data profile, capture boundary, and quality status.
8. Long-running sessions have start/stop lifecycle, segment metadata,
   backpressure counters, transport diagnostics, gap manifests, and crash
   recovery.
9. Runtime performance is stable at speed five. No reader can depend on a
   per-frame heap traversal or slow Python graph walk.
10. Save-oracle comparison remains available as a development test path, but the
    live recorder never silently fills gaps from a save.

## Cadence Model

The recording layer needs multiple cadences, not one global polling interval.

### Raw Collector Samples

Low-cost root samples used to notice clock changes and transient state. The
current speed-five proof says 10 ms sampling is enough for the present clock
worker and 25 ms was not enough for that proof. Keep 10 ms as the conservative
polling default until a hook or better measured interval replaces it.

The proof-mode `collector_control.json` path is not the final runtime contract.
Normal recording needs an explicit `CollectorSession` with start/stop lifecycle,
session-scoped transport, and manifest diagnostics. The proof control file can
remain as a test harness while the session path is built, but production
recording should not depend on proof semantics.

### Scheduled Replay Snapshots

Hourly-or-better point-in-time snapshots for durable world state. These must
include clock, country identity, map control, and every domain whose state needs
to be scrubbed at that hour. Missing a scheduled hour is either a command
failure in proof mode or an explicit manifest gap in normal mode.

### Event/Delta Streams

High-frequency streams for things that can begin and end between hourly
snapshots: combat, movement, order changes, province control flips, naval/air
engagements, and short-lived UI or diplomacy events. Event streams can be
generated by diffing frequent snapshots, exact hooks, or both.

### Daily Rollups

Daily IC/economy rows can be useful for graphs and validation, but they are
rollups only. They cannot substitute for hourly replay state.

### Static Definition Capture

Definitions, localization, icons, map geometry, terrain, state names, focus
trees, tech trees, equipment definitions, GUI geometry, and catalogs are captured
from the active game/mod files, not from the live process hot loop. The replay
must record the static profile hashes needed to decode live ids later.

## Target Data Inventory

The list below is the recording target. For every item, the work is the same:
find a live collector source or exact static source, map it into the schema,
validate it against fixtures, and give the UI an honest captured/null/unavailable
contract.

### 1. Session, Build, Mod, And Timeline

Capture:

- Game id, segment id, segment start/stop reason, campaign continuity id.
- Current game date/hour and canonical `hour_index`.
- Pause/unpause state when available.
- Game speed changes when available.
- Executable version, executable hash, image base, build profile id.
- Active mod set identity, mod load order, mod checksums/static definition
  hashes, localization profile, map/profile hashes.
- DLC/game-rule/scenario metadata when exposed live or statically.
- Collector version, ABI version, transport version, profile version.
- Producer sequence ids, recorder sequence ids, monotonic timestamps, frame
  byte counts, CRC/checksum, pipe/shared-memory token.
- Scheduled-hour coverage, skipped-hour records, duplicate snapshots, torn-read
  rejects, domain omission reasons, backpressure counters.

Tasks:

- Promote the bounded worker into normal long-running session mode.
- Add explicit start/stop/pause/resume segment records.
- Persist backpressure and transport-health counters in the manifest.
- Store only basename/profile ids where filesystem identity is needed.
- Make proof mode fail on non-monotonic time, skipped requested hours, or wall
  time overrun.

### 2. Country Identity And Player Presence

Capture:

- TAG table, active country-present rows, original tag, cosmetic tag, dynamic
  tag state, reserved dynamic-country marker.
- Country object pointer identity, tag-db id, country running index bridge when
  needed.
- Capital/original capital, major status, country exists/has capitulated where
  available.
- Human/player occupancy, observer/co-op/player persona name where available.
- Subject/overlord identity and autonomy state.

Tasks:

- Replace heap-scan country-object discovery with a root-anchored or
  hook-derived identity resolver.
- Cache country object pointers before recording and validate them cheaply on
  each frame.
- Maintain id-space bridges without requiring a save.
- Keep true Steam identity as a wall unless new evidence appears; player persona
  name is acceptable only when measured.

### 3. Country Politics, Laws, Ideas, And Leadership

Capture:

- Political power, political power gain where available.
- Stability, war support, command power.
- Ruling party, ideology, party popularities.
- Economy/conscription/trade/mobilization/occupation laws.
- National spirits, timed ideas, designers, advisors, theorists, army/navy/air
  chiefs.
- Leader, country leader traits, elections, election date/frequency, elections
  allowed.
- Appointed advisors and political residuals.
- Power-balance state, side, range, thresholds, trend markers when derivable.

Tasks:

- Port `country_scalars`, `politics_live`, `politics_residuals`,
  `leader_detail`, and `commanders_live` into collector-owned readers.
- Separate exact live fields from computed tooltip-only fields.
- Implement power-balance trend/marker math or badge it.
- Validate against existing save-oracle fixtures without making saves runtime
  inputs.

### 4. Economy, Industry, Resources, And Construction

Capture:

- Civilian, military, and dockyard factory totals, usable totals, offmap totals,
  damaged/disabled factories where available.
- Factory assignment by construction, production, consumer goods, trade, repair,
  and special projects.
- Construction queue entries: target type, state/province, priority, produced
  progress, active factories, cost, created date.
- State and province building levels: infrastructure, factories, dockyards,
  airbases, naval bases, supply hubs, railways, forts, coastal forts, AA, radar,
  rocket sites, nuclear reactors, synthetic refineries, special-project
  facilities.
- Resource production, exports, imports, consumed resources, shortages, and
  available resources for steel/oil/aluminium/tungsten/chromium/rubber/coal
  where the game exposes it.
- Trade deals, trade factories, convoy use, market contracts, lend-lease
  transfers.
- Fuel current, max, gain/usage where available, storage capacity.
- Equipment stockpiles by equipment type and variant.
- Production lines: equipment/variant, assigned factories/dockyards, efficiency,
  efficiency cap, daily output, resource deficits, progress, priority, and
  produced deltas.
- Equipment market stockpiles, automation settings, price bands, subsidies.

Tasks:

- Start with IC-Graphs-class economy roots, but sample hourly-or-better.
- Port `production_live`, `fuel_live`, `construction_orders`,
  `buildings_live`, `state_fields`, `market`, `market_extra`, and resource
  folds into collector frames.
- Define whether each economy value is an instantaneous state, hourly delta, or
  daily rollup.
- Add per-line stable ids so production line changes produce clean events.
- Add static definition joins for equipment names, resource icons, building
  names, and costs.

### 5. Research, Focuses, Decisions, Missions, And Projects

Capture:

- Completed technologies, doctrines, special technologies, and completion times
  derived from diffs.
- Active research slots, tech id, progress units, progress fraction when
  available, assigned bonuses, ahead-of-time modifiers when reconstructable.
- Research slot count and research speed/live ETA where derivable.
- Focus tree id, completed focuses, current focus, focus progress, continuous
  focus, pause state, focus completion events.
- Active decisions, timed decisions, missions, cooldowns, category status,
  decision timeout/progress.
- One-shot decisions only when they leave persistent state, emit a marker, or
  can be inferred from an exact state diff.
- Special projects, facilities, scientists, project status, project completion,
  breakthrough/research progress where available.
- MIOs, MIO policies, scheduled variants, equipment upgrades.

Tasks:

- Port `tech_live`, `focus_live`, `decisions_market`, `projects_live`, `mio`,
  `designs`, `designs_variants`, and save-proven project parsers into collector
  domain plans.
- Implement research ETA/live-speed from live inputs or badge it.
- Add optional exact event markers for focus completion and decision/event
  fire times where the in-game mod can cheaply log them.
- Keep static focus/tech/project definitions in the static profile, not in
  every frame.

### 6. States, Provinces, Map Control, And Occupation

Capture:

- State owner, controller, cores, claims, previous owner where available.
- Province controller, explicit overrides, fallback source, state id mapping.
- Occupation law, compliance, resistance, garrison state, occupation modifiers.
- State buildings and province buildings.
- State resources, manpower, category, victory points, VP-by-province.
- Dynamic state modifiers, targeted modifiers, active state modifiers.
- Strategic bombing damage, last bombing state, border conflicts.
- Province terrain, weather, building icons, supply node/railway/naval-base
  state, route-use inputs.
- Map mirrors: owner by state, control by province, resource overlays, building
  overlays, occupation overlays.

Tasks:

- Port `state_control`, `province_control`, `state_detail`, `state_vp`,
  `state_fields`, `buildings_live`, and `occupation` into collector-owned roots.
- Keep static terrain/state/province/map geometry in the static definition
  profile.
- Add stable per-state/province validation counts and bounds.
- Make control flips event-derived from collector snapshots, not saves.

### 7. Land Divisions, Templates, And Army OOB

Capture:

- Division id/type, owner, template id/name, name where available.
- Current province, state, destination province, movement progress, strategic
  redeployment, retreat state, transport state.
- Organisation, strength, manpower, experience, training, entrenchment/dig-in,
  acclimatization, supply status, max supply, supply gain, fuel state where
  available.
- Equipment inventory by equipment variant and amount.
- Division template structure, battalions, support companies, combat width,
  manpower/equipment requirements, derived combat stats.
- Killed/lost manpower, casualties, captured/encircled/lost state where
  available.
- Army, army group, theater assignment, commander, field marshal, order
  assignment, front/battle-plan membership.
- Battle plans, front lines, offensive lines, fallback lines, spearheads,
  garrison orders, naval invasion orders, paradrop orders where available.
- Railway guns and special land units.

Tasks:

- Port `divisions`, `division_equipment`, `oob_command`, `construction_orders`,
  `designs`, and `commanders_live` into root-anchored collector reads.
- Replace any division-body heap scanning with cached object pools and cheap
  liveness checks.
- Add high-frequency movement/order deltas separate from hourly full snapshots.
- Build stable identity rules for divisions that split, merge, deploy, or die.
- Add template and stat static joins so the app can show authentic counters and
  designer stats without guessing.

### 8. Navy, Ships, Fleets, Task Forces, And Naval Combat

Capture:

- Ship roster: ship id, owner, hull/type, variant/design, name, task force,
  fleet, theater, location/sea region, HP/strength, org, experience, repair
  state, mission state where available.
- Fleet/task-force hierarchy, admiral, commander traits/skill, mission, region
  assignments, engagement/split-off/repair settings where available.
- Naval invasions and transport convoy movement where available.
- Naval region control, convoy routes, naval supremacy/proxy data, sea-zone
  mission coverage.
- Active naval combats: combat id, region/location, attackers, defenders,
  ships engaged, damage, hits, sinkings, retreat/end state.
- Sunk ship history and convoy-sunk ledger.

Tasks:

- Port `ships`, `oob_command`, `naval_air`, `naval_air_combats`, and
  `naval_air_combat_stats` into collector frames.
- Build a live fixture with active naval combat; peacetime empty captures are
  not enough.
- Treat region-level naval/air values as badged proxies until per-province or
  per-unit exactness is proven.
- Reconstruct sinkings and combat outcomes from combat lifecycle plus roster
  deltas.

### 9. Air Wings, Air Regions, Aces, And Air Combat

Capture:

- Air wings: id, owner, plane type/variant, count, reserves, mission, airbase,
  region assignment, strength, experience, ace assignment, reinforcement state.
- Airbase capacity/use and per-country capacity penalties.
- Strategic air region state, air superiority, detection, mission efficiency,
  bombing, interception, CAS, logistics strike, naval strike where available.
- Aces, ace gained/lost events, ace country/wing joins.
- Active air engagements and air losses where available.
- Air theater/group indices.

Tasks:

- Promote `strategic_air`, `naval_air`, `abilities`, `country_extra`, and OOB
  air-wing rows into collector-sourced domains or exact derived rows.
- Build an active air-combat fixture for validation.
- Resolve `airwing_effective_planes_denom` or badge it.
- Decide which air-superiority values are exact live state and which are
  computed/render proxies.

### 10. Land, Naval, And Air Combat Event Layer

Capture:

- Combat id, kind, location province/region, start/end time, age/duration.
- Attackers and defenders with unit ids, owner tags, commander where available.
- Terrain, weather, tactic, phase, combat width, reinforce chance, modifiers,
  air support/CAS, forts/river/amphibious/paradrop indicators.
- Per-side org/strength/equipment/manpower deltas.
- Damage rows, equipment loss rows, casualties, ship sinkings, plane losses.
- Combat outcome, retreat/breakthrough/overrun/capitulation linkage where
  inferable.
- Confidence metadata: observed start, observed end, sampled-in-progress,
  possible sub-sample miss, merged/skipped due to overload.

Tasks:

- Keep land combat exact participant work as the template, then add naval and
  air active fixtures.
- Move combat enumeration into the collector with a high-frequency cadence or
  safe combat-boundary hooks.
- Emit lifecycle events: born, participant update, damage update, ended.
- Derive battle logs from lifecycle plus division/ship/wing deltas.
- Accept that a combat that starts and ends between two samples can be missed;
  count that as a confidence gap, not a fake event.

### 11. Diplomacy, Wars, Factions, Subjects, And Peace

Capture:

- Wars, attackers/defenders, war goals, war score, participants, start/end,
  casualties, capitulation state, peace conference state.
- Factions, faction leader, members, faction changes.
- Guarantees, non-aggression pacts, military access, docking rights, embargoes,
  truce state, volunteers, expeditionaries, attaches.
- Bilateral opinion, relation flags, autonomy, overlord/subject graph.
- Incoming diplomatic actions, delayed diplomatic actions, wargoals.
- Collaboration governments, governments in exile, civil war target, dynamic
  revolution tags.
- Lend-lease and trade contracts.

Tasks:

- Port `diplomacy_live`, `diplo_relations`, `market`, `deployment`, and
  relevant `country_extra`/`world_events` rows into collector domains.
- Build wartime fixtures for peace conferences, incoming actions, governments in
  exile, collaboration governments, and civil-war-only rows.
- Add exact event diffs for war declaration, faction join/leave, capitulation,
  peace conference start/end, lend-lease start/end, and subject/autonomy changes.

### 12. Intel, Operatives, Agencies, And Operations

Capture:

- Intelligence agency exists/name/icon, upgrades, upgrade progress.
- Operatives, operative owner, assignment, target, skill, XP, code name where
  available, risk/capture state where available.
- Intel networks, network vertices, subversive activity, danger.
- Cryptology/decryption state.
- Operations, operation target, chance bands, outcome/progress where available.
- Intel ledger/pairwise observed values where derivable.

Tasks:

- Port `intel_live`, `intel_deep`, `operative_xp`, `strategic_operatives`, and
  `intel_ledger` into collector-sourced or derived rows.
- Resolve operation chance/intel bands/capitulation math or badge it.
- Build eventful fixtures for operative code name and active operations.

### 13. Logistics, Supply, Convoys, And Attrition

Capture:

- Per-country logistics: supply nodes, motorization, trucks, trains, convoys,
  convoy losses, lost rails/trains/trucks ring buffers.
- Supply node state, rail connection state, capital supply, foreign home-base
  nodes, disrupted supply, bottlenecks.
- Per-province supply proxy and solver inputs: capacity, draw, efficiency,
  divisions present.
- Unit supply status, attrition status, equipment deficits, reinforcement
  needs.
- Convoy use for trade, supply, invasions, lend-lease, troop transport.

Tasks:

- Port `supply`, `supply_inputs`, `supply_logistics`, `countries.*.logistics`,
  and convoy/misc readers into collector-friendly roots.
- Keep exact per-province supply tooltip number as a wall unless the actual
  stored value or solver is proven.
- Implement attrition flag from live terrain/supply/weather/unit inputs or keep
  the anchored unavailable badge.
- Separate exact values from badged proxies in schema and UI.

### 14. Weather, Time, And World State

Capture:

- Date/hour, day/night, season.
- Realized province temperature, water/precipitation, snow.
- Region temperature, next weather change, weather phenomenon flags
  (rain/snow/blizzard/fog/mud/sandstorm/arctic water where present).
- Weather map-mode overlay driver and frozen-water state.
- World scenario data, start date, current date live, global flags and
  variables where in scope.
- World tension contributor ledger and aggregate world tension when derivable.
- Tech-sharing groups, power-balance world data, faction influence.
- Raids, strategic bombing/naval raid target tables.

Tasks:

- Move weather readers into a true collector/static profile path or keep them
  absent until cheap and reliable.
- Implement weather phenomenon overlay semantics and side-by-side visual proof.
- Derive day/night/season from clock.
- Implement world tension aggregate from threat contributors or badge it.
- Keep global script flags/variables as opaque causal state when captured.

### 15. Events, Scripted State, Alerts, And UI-Relevant Ledgers

Capture:

- Fired events, pending events, event ids, news events, selected options where
  persistent or marker-backed.
- Script flags and variables at world/country/state scope when in replay scope.
- Delayed events and event targets only when persistent and useful for causal
  replay.
- Alerts ledger: alert type, country, severity, target, active/inactive where
  live-exposed.
- War participant casualties and computed war participant score.
- Selection groups, player settings, cooperative game state where available.
- Sunk convoy/ship ledgers and history rows.

Tasks:

- Use static event definitions for title/description/picture/options.
- Add optional exact markers for cheap script `on_actions`.
- Promote `world_events`, `events`, `country_extra`, and wartime rows to live
  collector sources or explicit unavailable badges.
- Build wartime/eventful fixtures for alert ledger, combat log, combat data
  entry, selection groups, sunk ships, and player settings.

### 16. Static Definitions, Assets, And Visual Drivers

Capture:

- Map geometry, province/state adjacency, terrain, strategic regions, supply
  areas, naval regions, air regions, victory-point positions.
- State/province names, country names, cosmetic tags, localization.
- Focus tree definitions, tech/doctrine definitions, research costs, XORs,
  prerequisites, GUI geometry.
- Equipment archetypes, modules, variants, MIO definitions, designer stats.
- Ideas, advisors, laws, traits, tactics, operations, occupation laws, wargoals,
  opinion modifiers, power-balance definitions.
- Unit counters, portraits, insignia, emblem assets, sprite frames, icon atlases,
  GUI chrome, tooltip frames.
- Map-mode render inputs: owner, controller, compliance, resistance, buildings,
  resources, weather, supply, air/naval, front lines, battle plans, trade routes,
  labels, vector arrows.

Tasks:

- Keep static parsers out of the hot collector path, but bind their hashes into
  replay provenance.
- Finish general portrait-on-counter and insignia/emblem asset extraction.
- Finish front-line/battle-plan/trade-route/weather overlay parity.
- Add visual proof harnesses for every map mode and counter family.
- Badge proxy overlays and unavailable visual details explicitly.

### 17. Multiplayer Continuity And Rehost Handling

Capture:

- Segment start/stop, rehost, crash, rollback, desync, hotjoin, and observer
  changes where detectable.
- Campaign identity across segments.
- Player country assignment changes and co-op/human occupancy.
- Time jumps, duplicate hours, negative deltas, rollbacks, and branch/segment
  selection.
- Host game speed/pause behavior when available.

Tasks:

- Extend `live_capture/continuity.py` into the collector session model.
- Make time rollback create a new segment rather than corrupting the index.
- Add replay-library metadata for multiple segments per campaign.
- Add UI warnings for gaps, rollback spans, and unsupported domains.

## Engineering Work Packages

### A. Collector Core

- Replace proof-only bounded worker mode with normal long-running recorder mode.
- Add explicit session control: start, stop, graceful detach, crash detection,
  stale worker cleanup, and run-token isolation.
- Add explicit frame types: `session_start`, `heartbeat`, `root_probe`,
  `snapshot`, `domain_delta`, `event`, `diagnostic`, `gap`, and
  `session_stop`.
- Move frame construction away from ad hoc JSON string building toward a typed
  frame encoder.
- Keep hook callbacks tiny. Hook code should enqueue descriptors only.
- Add a collector-owned worker thread for traversal and transport writes.
- Add domain enable/disable flags so a bad domain can fail closed without
  inventing data.
- Add memory safety wrappers for pointer validation, range checks, vtable/sentinel
  checks, MSVC string reads, small-vector/vector/list traversal, and object
  liveness.

### B. Build Profiles And Root Discovery

- Define the profile file as the single contract for supported builds:
  executable hash, version label, module ranges, hook RVAs/patterns, root
  offsets, sentinels, vtables, container layouts, static profile ids, and enabled
  domains.
- Add a profile generator/checker pipeline for new the game builds.
- Add pattern scans or symbol/string anchors where RVAs are too brittle.
- Add per-domain sentinels and invariants: expected counts, tag table shape,
  state/province bounds, country counts, object vtables, date sanity, and
  cross-domain joins.
- Require profile preflight immediately before recording.
- Record profile id and profile hash in every segment manifest.

### C. Transport And Recorder Adapter

- Keep named pipes for the next phase, but design the frame contract so shared
  memory/ring buffer can replace it if needed.
- Include sequence id, monotonic timestamp, game date/hour, boundary, domain,
  byte count, checksum, and quality status on every frame.
- Every runtime frame should carry or inherit executable hash, mod/static profile
  id, profile hash, collector version, ABI version, capture boundary, domain
  list or domain id, and transport metadata.
- Add backpressure policy: diagnostics first, optional high-frequency deltas
  second, scheduled snapshots last; never block the game thread.
- Add manifest counters for dropped diagnostics, dropped optional deltas,
  dropped scheduled frames, queue high-water mark, invalid frames, duplicate
  sequence ids, and CRC failures.
- Make `.battleplan` finalization preserve all gap/quality metadata.

### D. Schema And Provenance

- Keep schema additions additive and optional.
- Add collector-specific provenance fields where needed, but do not rename
  existing snapshot fields in place.
- Promote current sidecars into first-class schema homes when the app needs
  stable consumption.
- For every field, define one of: collector-measured, static profile, derived
  from collector/static inputs, unavailable, wall, excluded.
- Keep measured zero distinct from unavailable/null.
- Add domain-level quality: complete, partial, proxy, stale, torn-rejected,
  unsupported, failed-sentinel, backpressure-gap.

### E. Python/Rust Split

- Rust owns the hot path: injected collector, root validation, domain reads,
  high-frequency streams, transport, and eventually replay packing where
  performance matters.
- Python remains the research/oracle/harness layer while the runtime matures:
  profile checks, save fixture comparison, schema validation, coverage audits,
  and command-line orchestration.
- Reuse Python assembler logic initially to keep schema behavior stable.
- Gradually move recorder hot-path pieces to Rust only when measurements show
  Python is the bottleneck.
- Do not rewrite everything into Rust just for cleanliness. Move what must keep
  up with the game.

### F. Domain Promotion Pipeline

For every current memory/save domain:

1. Identify the schema home and field ownership.
2. Identify current evidence: live reader, save parser, static parser, computed
   model, visual binding, or wall.
3. Identify the live collector root/hook/boundary.
4. Define pointer/container layout and sentinels.
5. Implement the collector reader with no heap-wide hot-loop scans.
6. Emit typed frame payloads.
7. Fold into snapshots with no-fabrication behavior.
8. Diff into events where useful.
9. Compare with save fixtures in dev only.
10. Add speed-five cadence/performance proof.
11. Add UI contract: exact, proxy, unavailable, or wall.

### G. Event Reconstruction

- Extend the diff engine for every domain where the replay needs a timeline:
  state capture, province capture, focus complete, tech complete, production
  line change, division deployed/destroyed/moved, combat start/end, war started,
  faction changed, capitulation, peace conference, law/idea/advisor change,
  market/trade/lend-lease change, operative operation, nuke/raid/ace events.
- Use exact collector events when available and snapshot diffs as the fallback.
- Add event confidence and source: exact hook, exact snapshot diff,
  high-frequency sampled, inferred, proxy.
- Do not emit events from null-to-null or unavailable-to-unavailable changes.

### H. Validation And Test Harness

- Keep unit tests for schema validation, frame validation, recorder cadence,
  manifest diagnostics, and `.battleplan` packing.
- Add integration proof commands for:
  - unsupported build refuses attach;
  - supported build emits heartbeat/root probe;
  - speed-five hourly proof catches requested hours;
  - long-running capture records gap-free core snapshots;
  - transport detects corrupt/duplicate/out-of-order frames;
  - native app opens the output container.
- Add domain fixture comparisons using save oracle output only in test mode.
- Add targeted live fixtures: land combat, naval combat, air combat, wartime
  diplomacy, peace conference, collaboration/government-in-exile, active
  operations, market trades, weather phenomena, special projects, invasions.
- Add performance budgets and fail tests when a domain exceeds them.

### I. UI Consumption Contract

- For every view, specify which schema fields are required, optional, proxy, or
  unavailable.
- Add badges for proxy supply, region-level air/naval, computed gaps, missing
  combat detail, missing portraits/assets, and unsupported domains.
- Make manifest gaps visible on the replay timeline.
- Prevent the UI from silently treating null as zero.
- Keep all mutating the game controls disabled/inert in replay mode.

## Migration Order

### Phase 1: Long-Running Core Recorder

Deliver:

- Long-running collector session mode.
- Session start/stop records.
- Separate profile preflight before recording.
- Single attach per recording session, not repeated proof attaches.
- Per-run tokenized transport.
- Backpressure counters.
- Gap manifest records.
- `.battleplan` finalization from collector frames.
- Native app open proof.

Acceptance:

- A live speed-five run records clock and country identity hourly-or-better with
  zero missed requested hours.
- No save is read.
- The output packs, verifies, and opens.
- The normal session path, not the bounded proof command, produces the replay.

Current status: session lifecycle, repeated bounded-worker wrapping, prefix
artifact preservation, hourly dedupe, replay finalization, and transport
diagnostics are implemented in the Python bridge. The remaining Phase 1 gap is
replacing repeated bounded attaches with a true long-running collector session
and adding richer backpressure counters.

### Phase 2: Root-Anchored Identity And Country Scalars

Deliver:

- Root-anchored country object identity.
- Cached pointer validation.
- Country identity, static identity, stability, war support, political power,
  command power, fuel, major/capital/original tag fields.

Acceptance:

- No country-object heap scan is required during normal recording.
- Country rows remain stable across a longer speed-five run.

Current status: live `tag_db_id` identity is emitted from the country tag table
and folded into snapshots. The recorder now has a save-free country-object
preflight bridge: `live_capture.collector_country_bridge` locates the live
country array before recording, reads each country object's own `tag_db_id`,
writes compact temporary `tag_db_id:country_object_ptr` and
`tag_db_id:country_record_ptr` control contracts, and keeps
`country_object_scan` off. The older `CountryIndexBridge` anchor scan remains a
fallback, not the primary path. The DLL accepts only bounded, aligned,
duplicate-free bridge rows, validates each object pointer against the expected
country vtable and `tag_db_id`, validates each record pointer by checking that it
still points at the same country object, then re-reads object-backed scalars plus
record-backed political power per frame before emitting `country_scalars`.

The preflight now has a same-process cache keyed by PID, process creation time,
exe hash, and module base. Cache hits validate the cached country-array token
with `locate.resolve_cached`, read the tag table directly from the
`CountryDataBase` root, and avoid both heap scans. The live Portugal process
resolved cached preflight in about 0.05 seconds. Cold preflight for a fresh
process still falls back to structural country-array discovery.

Remaining Phase 2 gaps:

- Make the preflight bridge automatic for the normal session path after a
  speed-five proof with row counts and zero skipped hours.
- Replace cold preflight country-array discovery with a root-anchored or
  hook-derived bridge so first session start is also fast on every supported
  build.
- Expand country-record-backed fields beyond political power where the live
  record layout proves them.
- Add longer-run pointer freshness diagnostics: accepted row count, stale row
  drops, and domain omission reasons.

### Phase 3: Map State Closure

Deliver:

- State owner/controller.
- Province controller.
- State/province buildings.
- State resources, VP, manpower/category, occupation/resistance/compliance.
- Map mirrors and map-control events.

Acceptance:

- The replay map can scrub owner/control/buildings/resources/occupation changes
  from live collector data only.

### Phase 4: Economy And Production

Deliver:

- Factories, construction queue, production lines, stockpiles, resources, fuel,
  market/trade/lend-lease basics.
- Hourly economy snapshots plus optional daily rollups.

Acceptance:

- Economy tables and graphs are live-fed at replay cadence.
- Daily rows are labelled as rollups only.

### Phase 5: Divisions And Land OOB

Deliver:

- Division roster, location, org/strength, equipment, template, movement,
  army/army-group/theater hierarchy, leaders, battle-plan assignment.

Acceptance:

- Unit counters can scrub positions and strength/org at replay cadence.
- Movement/order deltas are captured faster than hourly when possible.

### Phase 6: Combat And Movement High-Frequency Layer

Deliver:

- Land combat lifecycle.
- Naval combat lifecycle.
- Air combat lifecycle.
- Movement/order event stream.
- Combat confidence/gap model.

Acceptance:

- Battles that persist across at least one collector boundary are captured.
- Sub-boundary misses are counted as possible misses, not fabricated.

### Phase 7: Focus, Tech, Decisions, Projects, Politics

Deliver:

- Focus/research/decision/project/politics domains promoted to collector or
  exact derived sources.
- Focus/tech/decision event timelines.

Acceptance:

- Country deep-dive views can scrub political/research/focus/project state from
  live data.

### Phase 8: Diplomacy, Intel, Market, Logistics, Weather

Deliver:

- Wars/factions/subjects/opinion/relations.
- Intel/operatives/cryptology/operations.
- Market/trade/lend-lease extras.
- Supply/logistics proxies and solver inputs.
- Weather fields and visual drivers.

Acceptance:

- Diplomacy, intel, logistics, and weather views are live-fed or explicitly
  badged, with no hidden save dependency.

### Phase 9: Computed And Visual Parity Rows

Deliver:

- World tension aggregate or badge.
- Front-line/battle-plan/trade-route rendering parity.
- Weather phenomenon overlay parity.
- Attrition flag or badge.
- Designer combat stats.
- Air/naval per-province exactness decision.
- Operation chance bands.
- War participant score.
- Combat battle-detail rows.
- Portrait/insignia/emblem asset parity.

Acceptance:

- Every computed/visual manifest row is implemented, proven proxy, or badged.

### Phase 10: Hardening, Patch Workflow, And Release

Deliver:

- Repeatable new-build profile workflow.
- Smoke test matrix.
- Crash/unsupported-build behavior.
- Full speed-five soak test.
- Full app-open and scrub proof.
- Documentation updated when status changes.

Acceptance:

- A the game patch becomes a profile/update task, not a redesign.

## Current Open Manifest Rows To Close Or Badge

The coverage manifest currently names 48 open-or-wall rows:

- 26 eventful/wartime rows: `peace_conference`, `history.sunk_ship`,
  `ships_built`, incoming diplomatic actions, civil-war target,
  expeditionaries, invasion reports, dirty/removed controlled state/province
  rows, dynamic revolution tags, active targeted state modifiers, previous owner,
  last strategic bombing, border conflicts, combat log/data entry, selection
  groups, governments in exile, collaboration governments, player settings,
  strategic region data, alerts ledger, war participant casualties, operative
  code name, admiral fleet composition, and ship-owner/wing/ace joins.
- 5 open live/static rows: `all_playthrough_data`, `countries.*.logistics`,
  `countries.*.upgrades`, `mapicon.general_portrait_on_counter`,
  `ui_open.insignia_emblem_assets`.
- 15 computed/visual rows: weather phenomenon overlay, front lines overlay,
  attrition flag, world tension, weather formula/RNG/phenomenon names, combat
  detail stat rows, research ETA/live speed, designer combat stats, naval/air
  per-province exactness, occupation equilibrium garrison need, power-balance
  trend marker, operation chance/intel bands/capitulation, air-wing effective
  planes denominator, war participant score, and combat battle detail.
- 2 walls: exact per-province supply tooltip number and true Steam/player real
  names.

For the new recording layer, the bigger task is stricter than this row count:
even rows already closed by external memory readers or save parsers must be
promoted to collector output, derived from collector output, or badged as
unavailable for runtime replay.

## Speed-Five Performance Policy

- Clock/root sampling default remains 10 ms until a better hook or measured
  interval is proven.
- Scheduled hourly snapshots must not be skipped at speed five.
- Keep the current proof floor as the regression baseline: 12 requested in-game
  hourly deltas, 13 unique hourly snapshots, zero skipped hours, every positive
  delta exactly 1, monotonic producer sequence inside the active worker, no
  duplicate sequence ids inside a worker burst, completion under the configured
  wall cap, and byte-verified `.battleplan` packing. Multi-burst bridge runs
  must report sequence resets as burst-local transport, not as corruption.
- Hook callback p99 target: 100 us or lower. Hook callbacks must not traverse
  heaps, write files, block on transport, or serialize large payloads.
- Clock/country frame assembly p99 target: 5 ms or lower.
- Hourly light-snapshot p99 target: 250 ms or lower. A 500 ms or slower
  light-snapshot path at the observed speed-five pacing should be treated as a
  hard diagnostic/fail threshold until proven safe.
- Heavy domains must be incremental, cached, or distributed across safe
  boundaries. They cannot scan the whole heap per frame.
- Hook callbacks must avoid disk I/O, blocking pipe writes, compression, large
  allocation, or long traversal.
- Domain readers must expose elapsed time and object counts.
- Backpressure must prefer dropping optional diagnostics/deltas over scheduled
  snapshots.
- If scheduled snapshots are dropped, the replay manifest must say so.
- If a domain cannot keep up, disable or coarsen that domain explicitly rather
  than stalling the game or inventing values.

## What We Should Not Capture

These stay out of the replay unless the product scope changes:

- Allocator metadata, raw heap internals, container spare capacity, scratch AI
  weights, transient buffers, name-generator cursors, and engine internals that
  are neither persistent state nor player-visible.
- True Steam account identity/real names. Persona names are usable only when
  measured.
- Exact per-province supply tooltip number, unless new evidence proves an
  extractable stored value or faithful solver.
- Sub-sample events that begin and end between collector observations. These
  are possible misses, not fake events.

## Immediate Next Actions

1. Replace the repeated bounded-worker bridge with normal long-running collector
   recorder mode once the DLL can hold an explicit start/stop session.
2. Add manifest-level backpressure/gap counters for the long-running session.
3. Replace country-object heap discovery with root-anchored or hook-derived
   identity resolution.
4. Promote country identity/scalars into speed-five-safe collector frames.
5. Add the first non-clock domain tranche. The practical first three are light
   country economy/scalars, state/province map state, and divisions/OOB. The
   next implementation slice is root-anchored country identity plus country
   scalars; do not extend the heap-scanning `country_object_scan` path into the
   hot loop.
6. Run a longer speed-five proof and reopen the `.battleplan` in the native app.
7. Continue domain promotion in this order: first-three tranche, combat/movement,
   focus/research/decisions/projects, diplomacy/intel/logistics/weather,
   computed/visual rows.

## Completion Ledger To Maintain

For every row/domain as work proceeds, keep a small ledger with:

- Domain id.
- Schema path(s).
- Capture source: collector, static profile, derived, unavailable, wall,
  excluded.
- Cadence: raw, hourly, high-frequency, daily rollup, static.
- Current implementation owner/file.
- Root/hook/profile dependency.
- Validation fixture.
- Speed-five proof status.
- UI badge/consumption status.
- Failure mode.

That ledger should eventually be machine-readable and reconciled with
`coverage_audit/memory_manifest.json`, because the manifest is the closest thing
the repo has to a rigorous definition of "everything."
