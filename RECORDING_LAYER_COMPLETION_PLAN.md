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
- The accepted production path is `hoi4.exe` -> DLL collector -> compact
  raw/event/snapshot stream -> `.battleplan` file. Python is a development and
  proof harness only.
- Python may attach the DLL, run proof gates, validate manifests/replay files,
  and perform temporary bounded preflight discovery. It must not walk memory
  hourly/sub-hourly, own the hot extraction loop, or be required to keep up with
  speed five.
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
- If a segment still needs Python preflight/root discovery, it is temporary debt
  and is not fully DLL-complete until that discovery is moved into the DLL/Rust
  host or proven to be one-time, bounded, cached, and outside recording.

## Production Completion Standard

A segment is complete only when the DLL/Rust recording path captures that segment
at hourly-or-better live cadence, with no skipped game hours, at speed five or a
measured equivalent. Save files, old Python memory readers, and Python proof
harnesses may validate the output, but they cannot be required capture inputs.

Existing Python-backed `.battleplan` packing and session wrappers remain useful
for development proof gates. They do not by themselves promote a segment to
production-complete.

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
- `live_capture/` has the Python development/proof harness: assembler, durable
  store, `.battleplan` validator/writer, collector frame validation, cadence
  diagnostics, and legacy external-memory readers. These are not the production
  hot extraction loop.
- `replay_native/collector/` has the Rust DLL proof: attach, build/profile
  sentinels, clock/date roots, country tag-table rows, named-pipe frames,
  producer sequence ids, bounded worker mode, and speed-five proof behavior.
- `replay_native/recorder_host/` has the first Rust native host for the
  production path. It writes the bounded collector control file, loads the DLL
  through the read-only hook attach route, keeps the hook alive for the worker
  duration, reads the DLL named-pipe stream, persists `collector_frames.jsonl`,
  and emits `native_session_manifest.json` cadence diagnostics without Python in
  the live loop. It does not yet pack `.battleplan`.
- `memory_reader/` has a large amount of reverse-engineering work that can be
  ported into the collector as root-anchored live readers.
- `save_oracle/` has broad parsers for development fixtures and validation.

The most important transport proof so far is the user-confirmed speed-five worker run:
10 ms collector sampling captured 12 consecutive hourly deltas with zero skipped
hours, monotonic transport sequence, and byte-verified `.battleplan` packing.
That proves the clock/identity DLL transport shape can keep up at speed five. It
does not prove the full data layer yet, and Python-assisted session packing is
not a production-completion signal.

Current recorder-facing bridge status:

- `live_capture.collector_session` wraps repeated bounded DLL workers as a
  development/proof session with `session_start`, `session_stop`,
  `session_manifest`, a durable store, and a finalized `.battleplan`. It is not
  the accepted production hot loop.
- Native-host status: the first `collector_record` live attempt wrote
  `replay/snapshots/_native_collector_session/clock_lightweight_v1/` without
  Python in the live loop and captured 912 DLL frames, 14 root frames, 456 raw
  deltas, and 442 raw events with no parse errors and no skipped observed hours.
  It did not pass the 12-hour speed-five gate: the target game process exited
  during the run and only advanced from raw hour 60812157 to 60812158. This is
  failed proof evidence, not segment completion.
- 2026-06-30 native speed-five checkpoint, no Python hot loop, no save input:
  `clock_lightweight_speed5_v3` passed after widening the DLL worker duration
  cap from 30 seconds to a production-length cap. The rebuilt DLL recorded 740
  unique game hours in a 45 second native-host run, advanced 739 hours, skipped
  zero hours, emitted 740 scheduled clock frames, and preserved raw
  `land_combats` deltas/events. This completes the clock/transport proof for
  the current build, but `.battleplan` packing still has to move into the native
  host before the whole recorder is production-complete.
- The same checkpoint proved several country domains independently through the
  DLL/Rust path at speed five. `decisions_domain_speed5_v1` advanced 191 hours,
  emitted 192 scheduled `decisions_live` frames, and skipped zero hours.
  `diplomacy_domain_speed5_v1` advanced 213 hours, emitted 214 scheduled
  `diplomacy_live` frames, and skipped zero hours. These count as DLL-complete
  live capture segments for the current build, subject to replay-container
  packing moving into the native host.
- The combined country-domain sweep and several heavy country domains are not
  complete. `country_domains_speed5_v1` verified that the DLL can read
  `focus_live`, `tech_live`, `politics_live`, `politics_residuals`,
  `decisions_live`, and `diplomacy_live` from live all-country rows, but the
  combined payload advanced 272 hours while skipping 148 hours. Individual
  `politics_live`/`politics_residuals` advanced 242 hours with 48 skipped,
  `focus_live` advanced 241 hours with 15 skipped, and `tech_live` advanced 223
  hours with 7 skipped. These roots are real, but their current snapshot shape is
  too heavy for hourly speed-five capture and needs compact delta/snapshot
  emission before completion.
- Later 2026-06-30 release/native checkpoint: filtered compact country frames
  now use DLL-side keyframe/delta emission, heavy raw domains are opt-in when a
  scheduled-domain filter is supplied, filtered worker polls emit raw clock
  without per-poll root snapshots, and the DLL accepts 1 ms proof sampling for
  speed-five gates. With the release DLL/Rust host and no Python hot loop, these
  individual country domains passed hourly-or-better speed-five proof with zero
  skipped scheduled game hours on the live pid 34388 session:
  `politics_live_compact_release_speed5_v2` (187 scheduled frames, 186 hours
  advanced, 0 skipped), `politics_residuals_compact_release_speed5_v1` (311
  scheduled frames, 310 hours advanced, 0 skipped),
  `focus_compact_release_speed5_v1` (176 scheduled frames, 175 hours advanced,
  0 skipped), `tech_compact_release_speed5_v1` (285 scheduled frames, 284 hours
  advanced, 0 skipped), `decisions_compact_release_speed5_v1` (297 scheduled
  frames, 296 hours advanced, 0 skipped),
  `diplomacy_compact_release_speed5_v1` (303 scheduled frames, 302 hours
  advanced, 0 skipped), `fuel_compact_release_speed5_v1` (271 scheduled frames,
  270 hours advanced, 0 skipped), and
  `production_compact_release_speed5_v1` (176 scheduled frames, 174 hours
  advanced, 0 skipped). These are DLL/Rust live-capture completions for the
  individual segments on the current vanilla build, still pending native
  `.battleplan` packing.
- The same checkpoint also proved the remaining combined-reader blocker:
  `country_core_compact_release_speed5_v1`, with `focus_live`, `tech_live`,
  `politics_live`, and `politics_residuals` all enabled together, advanced 244
  hours but skipped 48 scheduled hours. Combined country-core recording is
  therefore not complete. The next recorder-layer task is to avoid additive
  all-country scans at every hour, either through cheaper per-domain dirty
  detection, per-domain capture lanes merged by the native host, or a bounded
  multiplexed scheduler that preserves hourly coverage for each claimed domain.
  The static `common/ideas` and `common/characters` filter contracts used for
  politics classification are currently generated by a bounded development
  preflight; moving that static game-definition cache generation into Rust host
  startup remains temporary debt before politics can be called fully
  production-self-contained.
- `projects_live` is also not complete. A lightweight run without project
  scanning kept cadence but emitted only clock domains, so it was correctly
  rejected as false completion. The native host now exposes
  `--special-projects-scan`; enabling it caused the project scan to exceed a
  bounded live proof window, and the target process was no longer present before
  a manifest was written. Project extraction needs bounded root discovery or a
  cached DLL/Rust project contract before it can be retried as a hot segment.
- A live vanilla run on 2026-06-28 processed 2 bounded worker bursts: 359 raw
  collector observations became 10 unique hourly replay snapshots, advanced 9
  in-game hours with zero skipped hours, verified the replay container, and left
  the game process responding.
- A 3-burst live run processed 800 raw observations into 20 unique hourly replay
  snapshots, advanced 19 in-game hours with zero skipped hours, and verified the
  replay container.
- The session wrapper writes a validated `collector_frames.jsonl` sidecar with
  every accepted DLL frame, including duplicate same-hour samples, and now embeds
  that same byte stream in finalized `.battleplan` files at
  `payload/<game>/collector_raw/collector_frames.jsonl`. Canonical replay
  snapshots still dedupe to one snapshot per game hour, but the raw stream
  preserves sub-hour observations for future combat, movement, and event
  extraction.
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
- The collector-frame adapter now has a map-control contract for
  `state_ownership`, `state_control`, and `province_control` payloads. It folds
  valid state/province rows into schema snapshots, mirrors province control into
  the map block, and records raw-frame row-count summaries in the session
  manifest. Requested map-control preflights now also produce a
  `map_control_quality` summary and fail closed when scheduled frames omit the
  requested map domains. State owner/controller and province control are now
  wired through the static `CGameState+0x2B0` `CProvince*` array: state rows use
  `CProvince+0xC0` state pointers and province rows use `CProvince+0x188`
  controller tag-db ids. The older state-root and dense-array/controller
  bridges remain fallback evidence. Current live proof on pid 43412 advanced 12
  game hours with zero skipped hours, stored 13 verified snapshots, emitted 12
  scheduled frames each for `state_ownership`, `state_control`, and
  `province_control`, carried 1081 state rows and 10272 province rows per frame,
  supplied no map preflight bridges, and verified the `.battleplan` container.
  State resource deposits now ride the same scheduled state-object path through
  `state_detail`: the strict pid 43412 proof advanced 12 game hours in 44.579
  wall seconds, emitted 13 scheduled `state_detail` frames, folded 14053 state
  resource rows with 9334 resource entries, reached full 1081/1081
  resource-bearing rows in one frame, and verified the replay with zero heavy
  payloads on lightweight worker frames. Unknown resource db ids are preserved
  as `tid_0x...`; the live reader handles the verified `size=6/cap=5`
  trailing `NO_ID` sentinel shape for Piedmont/Liaotung without fabricating
  rows. This covers state deposit maps only; country resource
  produced/exported/imported totals remain open.
- First state-control speed-five proof is now green. With cached state roots
  and 10 ms sampling, `collector_speed5_state_only_probe` advanced 12 game hours
  in about 9.9 wall seconds, stored 13 hourly snapshots, skipped zero hours,
  emitted 13 `state_ownership`/`state_control` scheduled frames covering 1081
  state rows, preserved 1171 raw collector frames, and verified the
  `.battleplan` with byte-identical raw-stream packaging. This proves the
  scheduled state map spine can keep up at speed five when preflight roots are
  warm.
- `live_capture.collector_map_bridge` now preflights and caches live state
  object roots. A cold live run resolved all 1081 state roots in about 28.5s; a
  same-process cache hit validated the roots and re-read all owner/controller
  rows in about 0.2s. This proves state ownership/control can use the same
  "expensive discovery before recording, cheap validation during recording"
  pattern as country scalars. Normal map capture has since moved to the static
  `CProvince` spine; the cache remains fallback evidence.
- The country-object preflight startup hang is now bounded out of the normal
  recorder path. Cache validation is allowed during hot session startup;
  heap-wide record recovery,
  `locate.locate`, `CountryIndexBridge.resolve`, vtable scans, and tag-table
  heap fallback are explicit `allow_cold_scan`/`--allow-country-cold-scan`
  debug paths only. If neither cache nor root rows are usable, the recorder now
  reports `cold_scan_disabled` instead of hanging before a run directory exists.
  Live proof: `collector_speed5_country_bounded_probe` ran the previously
  hanging `--preflight-country-object-bridge --preflight-state-control` path in
  about 7.5 wall seconds, advanced 4 game hours with zero skipped hours, stored
  5 verified snapshots, and packaged byte-identically. On the current Portugal
  process the country preflight still reports `cold_scan_disabled` with zero
  root rows, so this fixes startup liveness but not country-scalar coverage yet.
- Country object scalar coverage is now DLL-native without country preflight.
  The collector reads `CGameState+0x310` as a tag-db-id-indexed `CCountry*`
  table, validates each row by heap range, country vtable, object-owned
  `tag_db_id`, tag-table decode, stable name, and duplicate-object checks, then
  reuses the existing per-frame object revalidation. Live proof:
  `collector_live_gamestate_country_table_probe` supplied no country preflight
  bridge and no cold scan, advanced 2 game hours in about 5.81 wall seconds at
  speed five with zero skipped hours, verified the replay, and reported
  `country_object_identity.status = gamestate_country_table_verified` with 355
  object-backed country scalar rows. The same scheduled frames also emitted
  `fuel_live` and `production_live`, leaked no heavy map/production payloads into
  worker polls, and kept map-control rows through the static `CProvince` spine.
  Political power is now also DLL-native on this path: follow-up proof
  `collector_live_political_status_pp_probe` supplied no country preflight or
  record bridge, advanced 2 game hours with zero skipped hours, and reported
  Portugal `political_power = 28.069` from the validated `CPoliticalStatus`
  source. Scheduled country politics core is now also DLL-native: proof
  `collector_live_politics_core_probe` advanced 2 game hours with zero skipped
  hours, emitted 2 scheduled `politics_live` frames and zero worker-poll politics
  payloads, and reported Portugal `ruling_party = neutrality`, `idea_count = 12`,
  `party_count = 4`, and `low_confidence = false`.
- Raw clock deltas are now emitted as durable `domain_delta` frames from the
  DLL worker and merged into `collector_frames.jsonl` without polluting hourly
  snapshot domain claims. A speed-five smoke on 2026-06-28 advanced 4 game hours
  in an 8 second run, stored 5 hourly snapshots with zero skipped hours, and
  preserved 139 raw clock delta frames.
- Division movement now has a bounded raw-frame contract and Python state
  machine for decoded live rows. The contract uses stable replay keys
  (`entity_key = division:<div_id>:<div_type>`, payload `div_uid =
  <div_id>:<div_type>`) and rejects raw pointer/tag-db leakage. The DLL worker
  consumes a sanitized `division_body_bridge` and emits these rows on the raw
  pipe stream.
- Live division timing evidence is now clear: on the active vanilla build, a
  cold CDivision body locate took about 26.8s for 1806 divisions, while warmed
  re-reads from the pinned body map took about 22ms median across 20 samples.
  That is viable for sub-hour movement deltas at speed five, but only if the
  cold locate stays out of the hot loop.
- The old raw proof pattern was: Python preflight locates and validates
  CDivision body roots once, passes a sanitized `division_body_bridge` to the
  DLL control file, and the DLL performs bounded in-process revalidation plus
  movement delta emission on the same raw pipe stream as clock frames. That
  proves the field reads and cadence after body addresses are known; it does
  not satisfy production discovery. The compact DLL path now uses an anchored
  native country unit-wrapper root for the current vanilla build, and DLL-side
  heap/vtable scanning is negative evidence only, not a production discovery
  path.
- That pattern now has a live speed-five smoke proof. A 50 ms run bridged 1965
  division bodies, preserved 2155 raw division movement frames plus 31 clock
  frames, verified the replay, and leaked no body addresses into manifests. A
  shorter 10 ms run bridged 1971 division bodies, preserved 1816 division
  movement frames plus 39 clock frames, had zero session errors, and verified
  the replay. The remaining work is no longer "can we go below hourly"; it is
  backpressure/coalescing policy and widening the movement slice into combat,
  org/strength, and order semantics.
- The division stream now also carries bounded organisation, strength,
  `combat_ref_count`, and sampled `in_combat` telemetry. A 10 ms live run
  bridged 2042 division bodies, preserved 3229 division frames with all four
  expanded fields present, verified the replay, and recorded a raw-stream load
  of about 106 frames per collector tick with no missing or duplicate sequence
  numbers. This is still telemetry, not full combat truth; the combat replay
  layer must join it with active combat objects before claiming exact battles.
- The division stream now emits explicit lifecycle/semantic raw `event` frames
  for movement start/stop, org-strength bucket changes, and combat-ref changes
  from the same cached body samples. First-seen moving divisions emit only the
  movement-start event, not synthetic org/combat events from the initial
  all-field sample. A 2026-06-29 warm-cache live proof bridged 1564 division
  bodies, preserved 846 division deltas and 842 `division_movement_started`
  events in one game hour, and met the wall cap in 9.31 seconds.
- Air-wing telemetry now has the same bridge-gated raw-stream shape. The DLL
  accepts `air_wing_body_bridge` rows (`owner_tag_db_id:wing_id:body_ptr`),
  revalidates the wing body, pool, owner-chain country object, tag-table decode,
  and optional mission-region object every sample, and emits pointer-free raw
  `air_wings` deltas/events keyed as `air_wing:<TAG>:<wing_id>:<generation>`.
  The Python proof harness can populate the bridge with
  `--preflight-air-wings`; cache hits reread cached bodies and the expensive
  `NavalAirReader.enumerate_air_wings()` scan is gated by
  `--allow-air-wing-cold-scan`. This remains production debt until native
  discovery replaces the Python preflight. It captures count, mission mask,
  mission region, and effective active planes, not the final UI air-superiority
  percent.
- Active naval combat now has the first per-member DLL stream. The existing
  `naval_combat_body_bridge` roots `CNavalCombat` bodies; the DLL revalidates
  body, registry, side, group, and `CFEXMember` vtables, then emits bounded
  `naval_combat_members` deltas keyed by combat generation, side, and either
  ship instance id or convoy group/member/slot. Rows carry side membership,
  display identity, owner TAG, designed max-strength raw value, battle-slot
  index, state bitmask, and engagement-weight raw value. They intentionally do
  not claim current HP/org, targets, aggregate attack rows, or UI damage totals.
- Global CShip stats now have the same bridge-gated raw-stream shape. The DLL
  accepts `ship_body_bridge` rows (`instance_id:body_ptr`), revalidates the
  CShip vtable, handle type/id, and `ref100` sentinel every sample, and emits
  pointer-free raw `ships` deltas keyed as `ship:<instance_id>`. Rows carry
  definition token when readable, category enum/label, strength/org/experience
  raw fixed-point values plus decoded floats, and sunk-convoy counts. The Python
  proof harness can populate the bridge with `--preflight-ships`; cache hits
  validate cached bodies and the expensive global CShip vtable scan is gated by
  `--allow-ship-cold-scan`. This raw/oracle path has been superseded for
  production compact ship hull telemetry by the DLL-native anchored
  country/fleet/task-force/ship path; it remains historical validation evidence,
  not the hot capture loop. It captures current CShip hull HP/org/XP, not ship
  name, target, or exact sinking/damage events.
- Land and naval command membership now share the same bridge-gated raw-stream
  shape. The DLL accepts `oob_command_bridge` rows (`kind:id:body_ptr`),
  revalidates bridged army/army-group/division/fleet/task-force object identity
  every sample, maps `CDivision+0xB0` parent armies, maps `CArmy+0x1B8` parent
  army groups, validates `CArmy+0x88` leader pointers, resolves
  `CTaskForce+0x330` parent fleets against the bridge, validates CShip pointers
  from `CTaskForce+0x338/+0x344`, and emits pointer-free raw `oob_command`
  deltas keyed as `oob_command:army_group:<id>`, `oob_command:army:<id>`,
  `oob_command:fleet:<id>`, and `oob_command:task_force:<id>`. The Python proof
  harness can populate the bridge with `--preflight-oob-command`; cache hits
  validate cached roots and the expensive command-object vtable scan is gated by
  `--allow-oob-command-cold-scan`. This raw/oracle path has been superseded for
  active land OOB and verified compact fleet/task-force membership on the
  `870ebf` build. It still captures historical raw membership joins, army leader
  ids, bounded army leader skill, and assigned army leader trait tokens; it does
  not yet claim names, missions, locations, theatre rows, leader portraits,
  temporary/captured trait state, naval leaders, global commander rosters, or
  save-shaped OOB nesting.

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

The session path preserves these raw samples in `collector_frames.jsonl` and
packages the same stream as a manifest-covered `.battleplan` payload. That file
is the near-term home for sub-hour data while the canonical snapshot store
remains keyed by game hour. It prevents fast transient observations from being
lost merely because the current replay snapshot key is hourly.

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
- Persist raw-frame payload summaries so large domains like states/provinces
  prove their row counts without manual frame inspection.
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

- Keep country object identity on the DLL `CGameState+0x310` resolver and
  longer-soak it at speed five.
- Keep political power on the DLL `CCountry+0xF90 -> CPoliticalStatus+0xE0`
  resolver and classify any remaining record-only fields before adding a new
  country-record root.
- Keep state/province map control on the static `CProvince` spine, with the old
  state-root cache as fallback/corroboration evidence.
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
  `appointed_advisors`, `leader_detail`, and `commanders_live` into
  collector-owned readers.
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
- Research slot count is now scheduled DLL `tech_live`; research speed/live ETA
  remain open until active-slot/progress inputs are safe.
- Focus tree id, completed focuses, current focus, focus progress, continuous
  focus, pause state, focus completion events. Core focus state is now scheduled
  DLL `focus_live`; pause state and optional exact event markers remain open.
- Active decisions, timed decisions, missions, cooldowns, category status,
  decision timeout/progress.
- One-shot decisions only when they leave persistent state, emit a marker, or
  can be inferred from an exact state diff.
- Special projects, facilities, scientists, project status, project completion,
  breakthrough/research progress where available.
- MIOs, MIO policies, scheduled variants, equipment upgrades.

Tasks:

- Port active research slot detail/progress, project progress/completion and
  definition joins, `decisions_market`, `mio`, `designs`, `designs_variants`,
  and save-proven project parsers into collector domain plans. The first
  gated DLL `projects_live` catalog slice now covers facilities, scientists,
  and currently worked project rows only; normal runs should use the
  cache-first `special_projects_bridge` path, with the cold heap sweep reserved
  for explicit preflight refreshes.
- Keep scheduled DLL `focus_live` on the validated `CCountry` path and add only
  the missing pause/event-marker proof rather than reopening the old Python
  scan model.
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
- Current compact DLL checkpoint: `ships` now has a DLL-native, no-bridge
  compact stream for the verified `870ebf` vanilla executable profile. The
  collector walks `CGameState+0x310 -> CCountry+0x278 -> CFleet*`, normalizes
  bounded task-force subobject references to validated `CTaskForce` bodies, and
  validates `CShip` vtables/type/id/ref sentinels before emitting compact ship
  rows. `compact_ships_native_870ebf_20260701_010232` advanced 39 game hours
  with 0 skipped and no source/partial/bridge/worker failures; the wider
  `compact_full_plus_ships_870ebf_20260701_010320` covered 17 compact domains,
  advanced 53 game hours with 0 skipped, verified ordered/decodable records and
  `.battleplan` packing, and kept `ship_root_source = "dll_native"` with no
  compact discovery debt.
- Current compact DLL checkpoint: `oob_command` now emits native fleet and
  task-force hierarchy rows on the same verified build. The collector reuses the
  DLL-native `CGameState+0x310 -> CCountry+0x278 -> CFleet* -> CTaskForce/CShip`
  walk, then emits kind `fleet` rows with task-force members and kind
  `task_force` rows with parent fleet plus ship members. Focused proof
  `compact_oob_ships_native_870ebf_20260701_nav_oob_v2` advanced 38 game hours
  with 0 skipped and no source/partial/bridge/worker failures; the full
  `compact_full_plus_naval_oob_870ebf_20260701_v1` covered 17 compact domains,
  advanced 53 game hours with 0 skipped, verified ordered/decodable records and
  `.battleplan` packing, and captured real naval counters: 124 fleet rows, 195
  task-force rows, 404 task-force member ids, 1,625 ship member ids, 5,441
  observed fleet rows, and 8,638 observed task-force rows. Naval leaders,
  missions, theatre/region state, active naval combat damage/sinking semantics,
  and exact naval OOB presentation remain follow-up segments.
- Current owner-attribution checkpoint: compact fleet rows now carry the owning
  explicit country object tag-db id from the country-rooted fleet walk, and
  compact task-force rows inherit that owner from their parent fleet. Focused
  proof `compact_oob_ships_owner_870ebf_20260701_v4` advanced 39 game hours with
  0 skipped, verified ordered/decodable records and `.battleplan` packing, had
  no source/partial/bridge/worker failures, and captured 924 OOB owner rows. The
  full `compact_full_plus_oob_owner_870ebf_20260701_v2` covered 17 compact
  domains, advanced 53 game hours with 0 skipped, verified full per-domain
  coverage, ordered/decodable records and `.battleplan` packing, had no
  source/partial/bridge/worker failures, and captured 964 OOB owner rows
  including 152 fleet rows, 233 task-force rows, 536 task-force member ids,
  1,661 ship member ids, 474 missing-member-count rows, 5,940 observed fleet
  rows, and 9,243 observed task-force rows. Direct per-ship owner/name fields
  remain a separate ship-row widening task.
- Current DLL checkpoint: `naval_air_combats` now has a bridge-gated raw stream
  from supplied `CNavalCombat` body pins. The collector revalidates bridged
  bodies every sample and emits side-level active-combat summaries plus
  lifecycle start/update/end events without leaking raw pointers. This does not
  yet include per-ship combat rows, damage ticks, or sinkings. The recorder now
  has an explicit `--preflight-naval-combat` producer that enumerates active
  `CNavalCombat` bodies before attach and supplies the bridge rows, but this is
  still preflight-only and will not discover battles born later in a long run.
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
- Current DLL checkpoint: `air_wings` now has a bridge-gated raw stream from
  supplied air-wing body pins. It emits `air_wing_observed`,
  `air_wing_updated`, and `air_wing_lost` events with owner, wing id,
  generation, count, mission mask, mission region, and effective active planes.
  It does not yet emit plane variant/type, airbase, reserves, ace joins,
  airbase capacity, exact air-superiority percent, air losses, or region combat
  resolution.
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

- Use the live land-combat manager stream as the template, then add naval and
  air active fixtures.
- Land combat is live-verified for a no-cold 12-game-hour speed-five slice with
  start/update/end raw events. Canonical replay timeline promotion now exists
  for `land_combat_started` -> `combat_started` and `land_combat_ended` ->
  `combat_ended`; richer tick/participant deltas and app ingestion are still
  open.
- Lifecycle events for born, participant/age update, generation split, and ended
  are now emitted; damage update remains derived from division strength/org
  deltas joined to combat participants.
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
- Current status: scheduled DLL `diplomacy_live` now covers faction group and
  faction membership, and the preflighted `war_relation_bridge` covers war
  relation rows plus per-country `at_war` and casualty folds. The current-game
  proof
  `replay/snapshots/_collector_session_diplomacy_war_gate/20260629T202751Z_pid43412`
  passed `--require-diplomacy-war-relations`: it advanced 12 game hours in
  43.984 wall seconds, verified replay output, supplied 689 bridge rows after a
  90.422s preflight, observed 13 scheduled war frames, folded 884 country
  `at_war` rows and 884 casualty rows, and observed max per-frame casualty sum
  14124532. Scheduled `diplomacy_live` without `--preflight-war-relations`
  remains faction-only and is not war/casualty proof.
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
- Keep `.battleplan` finalization preserving raw stream, gap, and quality
  metadata; native readers now expose verified raw payload enumeration/read APIs
  and `scrub::Timeline` owns a sibling gameplay raw-frame index.

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
  command power, scheduled ruling party, party popularities, active idea tokens,
  election frequency, fuel, major/capital/original tag fields.

Acceptance:

- No country-object heap scan is required during normal recording.
- Country rows remain stable across a longer speed-five run.

Current status: live `tag_db_id` identity is emitted from the country tag table
and folded into snapshots. Object-backed country scalars are now rooted inside
the DLL through the `CGameState+0x310` tag-db-id-indexed `CCountry*` table. Each
candidate row is validated by heap range, country vtable, object-owned
`tag_db_id`, tag-table decode, stable name, and duplicate checks before
per-frame scalar reads. The bounded `CountryDataBase` object-local root attempt
still exists as fallback evidence and currently fails closed as
`country_database_root_insufficient` on the live Portugal process. The
collector now reads political power without the record bridge by validating
`CCountry+0xF90 -> CPoliticalStatus`, checking the political-status vtable and
`CPoliticalStatus+0xC8` country backlink, then reading fixed-point
`CPoliticalStatus+0xE0`. Scheduled `politics_live` now follows the same
validated `CPoliticalStatus` object and emits ruling party, party popularities,
ordered active idea tokens, idea count, election frequency, party count, and
low-confidence diagnostics only on scheduled frames. It also emits active
country leader rows by validating
`CPoliticalStatus -> ruling_party -> CCountryLeader -> CCharacter`; only
singleton ruling-party leader vectors are folded into
`countries.*.commanders.leader`, while multi-entry vectors are omitted until the
active element can be proven. Scheduled
`politics_residuals` also uses the same validated `CPoliticalStatus`, decoding
`last_election` from `+0xA0` and reading `elections_allowed` from `+0xEC`.
`appointed_advisors`, `appointed_advisor_details`, and
`appointed_advisor_slots` / `appointed_advisor_ledgers` are implemented behind
a validated static advisor contract sourced from `game_defs` /
`common/characters`; live state is derived from the active-ideas vector, static
metadata supplies `{slot, ledger, character}`, missing or invalid contracts omit
the leaves, and a measured empty `[]` is real. The collector does not sweep
`CAdvisor` heap objects in the hot path. The save-free Python country bridge
remains useful for
fallback/bootstrap and for future record-only fields: it writes compact temporary
`tag_db_id:country_object_ptr` and `tag_db_id:country_record_ptr` control
contracts, keeps `country_object_scan` off, and lets the DLL revalidate object
and record pointers before emitting `country_scalars`.

The preflight now has three same-process caches keyed by PID, process creation
time, exe hash, and module base. `country_array.json` stores the located record
array token when available. `country_objects.json` is the speed-five object
bridge cache: warm recording validates each cached object pointer by vtable,
object-owned `tag_db_id`, and tag-table match before emitting
`tag_db_id:country_object_ptr` control rows. `country_records.json` stores
record pointers for fallback and future record-only fields; warm recording
validates the record cache against current object rows, checks each record
back-pointer, and merges record pointers additively so record coverage cannot
shrink the object-backed country-scalar set. Record-only rows that cannot
validate as current country objects are skipped instead of poisoning the full
cache.
Heap-wide record recovery and cold object discovery are opt-in through
`allow_cold_scan` / `--allow-country-cold-scan`; normal recording startup fails
fast with `cold_scan_disabled` when only uncached cold discovery could continue.

Live proof on the current Portugal process: `collector_live_gamestate_country_table_probe`
supplied no country preflight bridge and no cold scan, advanced 2 game hours in
about 5.81 wall seconds at speed five with zero skipped hours, verified the
replay, and reported `country_object_identity.status =
gamestate_country_table_verified` with 355 object-backed country scalar rows.
The same scheduled frames also emitted `fuel_live` and `production_live`, and
heavy map/production payloads stayed out of lightweight worker polls. Follow-up
live proof `collector_live_political_status_pp_probe` supplied no country
preflight bridge or record bridge, advanced 2 game hours with zero skipped hours,
verified the replay, emitted 355 scalar rows per scheduled frame, and reported
Portugal `political_power = 28.069` on the first scheduled frame. Historical
record-cache proof remains fallback evidence: explicit record-cache bootstrap
from the object cache resolved in 111.907s, warm preflight resolved in 0.063s,
and the first live frame had 354 country rows with 223 non-null
`political_power` values. Follow-up live proof
`collector_live_politics_core_probe` emitted scheduled `politics_live` without a
country bridge or save, advanced 2 game hours with zero skipped hours, and
reported Portugal as `neutrality` with 12 active ideas and party vector
low_confidence false. Follow-up live proof
`collector_live_politics_residuals_probe` emitted 2 scheduled
`politics_residuals` frames with zero skipped hours and zero worker-poll
politics payloads; Portugal reported `last_election = 1933.3.5.1` and
`elections_allowed = false`. The current combined proof
`collector_live_politics_decisions_probe` supplied a 3722-token static advisor
contract and a 6474-row vanilla idea contract, advanced 2 game hours at speed
five with zero skipped hours, emitted 3 scheduled `politics_live` /
`politics_residuals` / `decisions_live` frames, kept worker-poll politics
payloads at zero, and measured Portugal as `appointed_advisors = []`. The same
live path now classifies typed law buckets, broad non-law `ideas_other`, and the
narrower `national_spirits` subset from the active-ideas vector when a validated
static idea-group contract is supplied. The proof measured Portugal laws as
`civilian_economy`, `export_focus`, and `volunteer_only` with 12 active ideas
and 9 non-law `ideas_other`.
Follow-up live proof `politics_live_idea_groups_2h_proof` rebuilt the DLL
with complete active `idea_groups` and active cabinet/designer `advisor_ideas`,
then ran against PID 40464 with `hourly_requirement.met = true`,
`game_hours_advanced = 2`, `stored_snapshot_count = 3`, and
`replay_verified = true`. The raw stream carried 2 scheduled `politics_live`
frames and 2 `politics_residuals` frames, with zero heavy payloads on
worker-poll frames; canonical replay snapshot validation reported zero schema
problems. The proof folded `idea_groups` for 712 country rows and positive
`advisor_ideas` for 160 rows. Portugal carried laws
`civilian_economy` / `export_focus` / `limited_conscription`, 11
`national_spirits`, `idea_groups.industrial_concern = companhia_uniao_fabril`,
and `advisor_ideas.industrial_concern = companhia_uniao_fabril`.
Follow-up active-ruler proof
`replay/snapshots/_collector_session_politics_leader_gate/20260629T171216Z_pid43412`
advanced 12 game hours in 39.328 wall seconds, stored 13 verified snapshots,
emitted 13 scheduled `politics_live` frames, captured 4095 non-ambiguous
`commanders.leader` rows with up to 315 active rulers per scheduled frame,
verified the replay, and kept politics payloads off worker-poll frames.
Current strict politics classification proof
`replay/snapshots/_collector_session_politics_idea_gate/20260629T211245Z_pid43412`
passed `--require-politics-idea-classification`: it supplied 6474 vanilla
`common/ideas` rows, advanced 12 game hours in 45.797 wall seconds, verified
replay output, emitted 12 scheduled `politics_live` frames, folded 4248 active
idea rows, 4248 law rows, 4248 complete idea-group rows, 1104 advisor/cabinet
idea rows, 4248 broad non-law idea rows, and 4248 national-spirit rows, and
kept worker-poll frames free of politics payloads.
Current advisor-residual proof
`replay/snapshots/_collector_session_politics_advisor_gate/20260629T212028Z_pid43412`
passed `--require-politics-advisor-residuals`: it supplied 3722 vanilla
`common/characters` advisor tokens, advanced 12 game hours in 46.047 wall
seconds, verified replay output, emitted 13 scheduled `politics_residuals`
frames, and observed 4602 country rows with `appointed_advisors`,
`appointed_advisor_details`, `appointed_advisor_slots`, and
`appointed_advisor_ledgers` fields present. The current fixture measured those
lists/maps as empty, so a non-empty appointed-advisor fixture remains open.
`decisions_live` is also now DLL-native: the collector validates
`CCountry+0xFA0 -> CDecisionStatus`, checks the owner back-pointer, emits
decision-status vector counts, and decodes active `CTimedDecision` rows. That
same proof found non-empty active timed decisions for 7 countries while Portugal
had measured-empty `timed_live = []`.
Scheduled `focus_live` is now DLL-native for the core focus state: the collector
uses `CCountry+0x1370 -> CFocusTree` for `focus_tree` and
`CCountry+0x1380` for completed focuses, completed count, current national
focus, current continuous focus, and progress days. The frame adapter folds
those rows into `countries.*.focus`, protects measured-empty completed lists by
measured TAG, and rejects impossible `current` plus `current_continuous` rows.
Live proof `collector_live_focus_probe` advanced 2 game hours with zero skipped
hours, emitted 3 scheduled `focus_live` frames, emitted 1065 country focus rows,
kept worker-poll focus payloads at zero, and measured Portugal on
`portuguese_focus` with 22 completed focuses and current `POR_standardization`.
`paused` and optional exact focus-completion markers remain open.
Scheduled `tech_live` now emits completed technology count and research slot
capacity from `CCountry+0xF60 -> CTechnologyStatus`, with vector/header/backlink
validation and adapter folding into `countries.*.research.tech_count` and
`.research_slots`. It also emits raw active-research rows from the bounded
`CTechnologyStatus+0xA0` `CResearchSlot*` vector and folds them into
`countries.*.research.in_progress_live`. Rows now carry
`progress_units = progress_raw / 1000`, moving
`live_progress_raw/live_progress_units` from
`CResearchSlot+0x18 -> CTechnology+0x198`, DLL base denominator
`base_required_points_raw/base_required_points_units` from
`CTechnology+0x160 -> CTechnologyTemplate+0x3e0`, and `tech_name` when the
cache-first `tech_def_filter` preflight supplies a validated live
tech-definition handle map. Combined proof
`replay/snapshots/_collector_session_research_denominator_gate/20260629T192125Z_pid43412`
advanced 36 game hours in 48.125 wall seconds with a no-cold tech-definition
cache hit, emitted 37 scheduled `tech_live` frames, captured 15687 named active
research-slot rows and 15687 live base-denominator rows with up to 424 rows per
frame, verified replay output, and kept research payloads off worker-poll
frames. The manifest recorded 15263 persisted live-progress rows, 15263
DLL-base progress rows, one moving-progress delta frame with 346 rows, and 346
ETA rows. A current-game rerun
`replay/snapshots/_collector_session_research_denominator_gate/20260629T204418Z_pid43412`
advanced 36 game hours in 52.344 wall seconds with a no-cold tech-definition
cache hit, emitted 38 scheduled `tech_live` frames, captured 16562 named active
research-slot rows, 16126 DLL-base progress rows, two moving-progress delta
frames with 708 rows, and 708 ETA rows, verified replay output, and kept
research payloads off worker-poll frames. The static denominator audit recorded
`game_defs/out/technologies.json`, 557 tech defs, 10 no-cost defs, and base
points-per-cost 110. The older `progress_units` remains source-badged but is
not accepted as moving progress; effective research cost/speed modifiers remain
unclaimed unless separately pinned.
The DLL/adapter contract for `projects_live` now supports the first top-level
`special_projects` catalog slice: facilities, scientists, and currently worked
project rows, all source-badged `C`. The adapter folds only scheduled frames
that explicitly claim `projects_live`, rejects pointer/tag-db diagnostics, and
summarizes project row counts in manifests. The normal producer path now has a
cache-first `special_projects_bridge`: Python preflight validates cached
facility, project, and scientist roots or performs a cold discovery only when
`--allow-special-projects-cold-scan` is explicitly paired with
`--preflight-special-projects`; the DLL then revalidates those roots in-process
and emits `special_projects_identity.status = bridge_verified` when usable.
The older `special_projects_scan = true` heap sweep remains available but is
not the normal/speed-five path. Project progress/completion, owner attribution,
and resolved project definition tokens remain open.

Remaining Phase 2 gaps:

- Audit whether any replay-critical values remain country-record-only; add a
  root-anchored or hook-derived record bridge only for those fields.
- Expand country-record-backed fields only where the live record layout proves
  data not already available through validated `CCountry` child objects.
- Add deeper cabinet/leader buckets and manager/root contracts where active idea
  token classification is not enough.
- Add research progress/ETA resolution, project progress/completion roots, and
  deeper targeted decision/cooldown roots; add focus
  pause/event-marker proof without returning to save-required capture.
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

Current status: state owner/controller and province controller are now
live-proven through scheduled DLL frames without requiring map preflight
bridges. The pid 43412 no-cold proof advanced 12 game hours in 45.734 wall
seconds, stored 13 verified snapshots, emitted 12 scheduled frames each for
`state_ownership`, `state_control`, and `province_control`, and carried 1081
state rows plus 10272 province rows per frame. Province-control cache validation
still mirrors the native scheduled reader for fallback use: it checks native
bridge shape, slot-span readability, controller-pointer coverage, heap/range/
alignment bounds, and tag-id limits before allowing cached province rows into
normal recording. Cold province-array discovery remains opt-in and is not needed
for the static-root scheduled map path. State resource deposits are also
live-proven on this same scheduled state spine: the strict pid 43412 proof
`replay/snapshots/_collector_session_state_resources_gate/20260629T223450Z_pid43412`
advanced 12 game hours in 44.579 wall seconds, emitted 13 scheduled
`state_detail` frames, folded 14053 state resource rows with 9334 resource
entries, reached full 1081/1081 resource-bearing state rows in one frame, and
verified the replay with zero heavy payloads on lightweight worker frames.
Unknown resource db ids are preserved as `tid_0x...`; the live reader handles
the verified `size=6/cap=5` trailing `NO_ID` sentinel shape for
Piedmont/Liaotung without fabricating rows. This is raw state deposit data
only, not country produced/exported/imported resource economy.

### Phase 4: Economy And Production

Deliver:

- Factories, construction queue, production lines, stockpiles, resources, fuel,
  market/trade/lend-lease basics.
- Hourly economy snapshots plus optional daily rollups.

Acceptance:

- Economy tables and graphs are live-fed at replay cadence.
- Daily rows are labelled as rollups only.

Current status: scheduled DLL economy frames already emit `production_live`
stockpile/line/factory totals and `fuel_live` current/max from validated
country-object bridge rows. Fuel gain/cost are now also DLL-emitted from the
fuel-status object, validated as bounded signed fixed-point values, and folded
into `countries.*.economy.fuel.{gain,cost}`. Live no-setup proof on pid 43412
advanced 12 game hours in 45.734 wall seconds, stored 13 verified snapshots,
observed `country_scalars`, `country_misc`, `fuel_live`, and scheduled
`production_live`, emitted 12 production frames and 12 fuel frames, and kept
heavy payloads off lightweight worker polls. The same run saw 356 production
country rows per frame, up to 20 military lines, 12 naval lines, and 146
stockpile entries. Construction queue is now also DLL-native inside scheduled
`production_live`: proof
`replay/snapshots/_collector_session_construction_gate/20260629T172526Z_pid43412`
advanced 12 game hours in 40.219 wall seconds, emitted 14 scheduled production
frames, folded 3661 queue rows with up to 262 rows per scheduled frame and 29
rows per country, confirmed replay-store `countries.*.production.construction_queue`
rows, and kept production payloads off worker polls. Construction target
state/location, market contracts, resource trade, and lend-lease remain
follow-up DLL slices.

### Phase 5: Divisions And Land OOB

Deliver:

- Division roster, location, org/strength, equipment, template, movement,
  army/army-group/theater hierarchy, leaders, battle-plan assignment.

Acceptance:

- Unit counters can scrub positions and strength/org at replay cadence.
- Movement/order deltas are captured faster than hourly when possible.

Current status: the production compact path now resolves division body roots
inside the DLL without Python, saves, a body bridge, or heap/vtable scanning.
The current vanilla build path is
`CGameState+0x310 -> CCountry+0x290 unit wrappers -> wrapper+0x10 CDivision`.
The collector refreshes that bounded root every compact frame, validates
country, wrapper, and division vtables plus division type/id bounds, then
samples the widened compact row contract from the anchored `CDivision` bodies:
division id, location/source/target/progress, organisation/strength buckets,
combat-ref/in-combat state, owner tag-db id, parent army id, raw experience,
max/current supply, supply gain, and fuel. Division compact data now uses a full
baseline followed by delta rows, with explicit zero-filled delete/tombstone rows
when a previously observed division id disappears. The current no-bridge proof
`compact_divisions_anchored_telemetry_validroot_speed5_20260630_v1` advanced
665 game hours with 666 frame-end records, 0 skipped, 91,642 typed division
rows, no bridge/source/partial failures, `division_root_source = "dll_native"`,
ordered/decodable compact stream, division read avg 7.505 ms, p95 8.002 ms, max
15.187 ms, avg frame 8.135 ms, p95 8.768 ms, max 15.776 ms, 8,197,696 raw
stream bytes, and an independently verified `native_session.battleplan`. This
promotes compact division body discovery and the widened movement/org/strength/
combat-ref/owner/army/supply/fuel compact row to DLL/Rust-complete for the
current vanilla build.

Important validator detail from the active vanilla Portugal proof: the
country-wrapper path can surface `CDivision` bodies whose handle type is `51`
rather than `4713`. The production collector now treats the `CDivision` vtable
as the primary identity guard and accepts body types `{4713, 51}` after the
vtable check. The compact stream still writes canonical replay division type
`4713`, so the host/replay schema does not branch on this engine-internal
variant. This fixed the failed weather-bundle rerun where the anchored root was
present but the DLL rejected every division row as `SOURCE_UNAVAILABLE`.

The wider `compact_country_divisions_land_combats_map_telemetry_validroot_speed5_20260630_v1`
stress gate covered 11 domains with the same anchored division path and proved
the widened rows decode and pack inside the combined stream, but it is not a
completed all-domain speed-five proof: it advanced 448 game hours with 381
frame-end records and skipped 68 hours. The stream had no parse/order errors,
no bridge/source/partial failures, and verified as a `.battleplan`; the blocker
is extraction volume and scheduling under combined load, led by `map_control`
p95 43.350 ms and several country-root lanes. Template/equipment/manpower/
assignment/order details still need separate typed records before the division
replay is complete in the broader gameplay sense.

Current combined status: `compact_country_divisions_land_combats_map_mapdelta_speed5_20260630_v1`
closed that widened 11-domain bottleneck by caching immutable map topology and
emitting `map_control` as a full baseline followed by delta data records while
perf records continue to report full observed row counts. The run covered
`fuel_live`, `production_live`, `focus_live`, `tech_live`, `politics_live`,
`politics_residuals`, `decisions_live`, `diplomacy_live`, `divisions`,
`land_combats`, and `map_control`; advanced 349 game hours with 350 frame-end
records, 0 skipped, full per-domain coverage, no parse/order errors, no
worker-panic, bridge-unavailable, source-unavailable, or partial-data flags, avg
frame 31.373 ms, p95 37.631 ms, max 51.696 ms, 34,334,975 raw stream bytes, and
an independently verified `.battleplan`. `map_control` observed 5,072,900 full
rows but emitted only 15,418 changed data rows; map read p95 dropped to 12.583
ms. The largest remaining p95 lanes in that run were widened `divisions` at
33.470 ms and `fuel_live`/`production_live` at 34.625 ms. This promotes the
current implemented 11-domain compact bundle to speed-five complete for the
active vanilla process, while longer soaks and the missing replay-critical
domains remain open.

Follow-up combined status:
`compact_country_divisions_land_combats_map_divdelta_parallel_speed5_20260630_v1`
kept the same 11-domain compact set and added division baseline-plus-delta
records, explicit division delete/tombstone rows, and bounded internal division
sampling workers. The run advanced 345 game hours with 346 frame-end records, 0
skipped, full per-domain coverage, no parse/order errors, no worker-panic,
bridge-unavailable, source-unavailable, or partial-data flags, avg frame 34.743
ms, p95 52.766 ms, max 66.712 ms, 62,786,985 raw stream bytes, and an
independently verified `.battleplan`. `divisions` observed 772,920 full rows
and emitted 585,450 changed/tombstone rows; division read p95 was 37.112 ms.
This preserves the current speed-five completion status for the implemented
11-domain compact bundle while reducing division stream volume semantics from
"repeat full roster" toward replay-friendly deltas. The remaining division
gaps are still template, equipment, manpower, exact order geometry, assignment,
split/merge/deploy/destroy identity rules, and higher-frequency semantic event
records.

Map-detail compact status:
`compact_map_detail_buildings_cached_speed5_20260630_v1` promoted live state
resource deposits and state/province building levels into the DLL/Rust compact
path as `map_detail`, mapped from scheduled `state_detail` and
`buildings_live`. The collector uses the native CProvince/CState topology,
baseline-plus-delta records, bounded province workers, and batched
building-vector metadata reads. The isolated proof advanced 259 game hours with
260 frame-end records, 0 skipped, full per-domain coverage, no parse/order
errors, no worker-panic, source-unavailable, or partial-data flags, avg frame
20.292 ms, p95 24.443 ms, max 59.611 ms, 473,672 raw stream bytes, and an
independently verified `.battleplan`.

Current 12-domain combined status:
`compact_country_divisions_land_combats_map_detail_speed5_20260630_v1` added
`state_detail` and `buildings_live`/`map_detail` to the previous compact
country/division/land-combat/map bundle. The run advanced 289 game hours with
290 frame-end records, 0 skipped, full per-domain coverage, no parse/order
errors, no worker-panic, bridge-unavailable, source-unavailable, or partial-data
flags, avg frame 54.922 ms, p95 89.906 ms, max 99.708 ms, 56,044,696 raw stream
bytes, and an independently verified `.battleplan`. `map_detail` observed
4,203,260 full rows and emitted 14,528 changed rows; its read p95 was 36.421
ms. This promotes the currently implemented 12-domain compact bundle to
speed-five complete for the active vanilla process. The remaining map/economy
gaps are still weather, logistics/supply proxies, market/trade extras,
occupation/resistance/compliance depth, and visual/computed overlays that have
not yet been promoted to compact DLL records.

Economy-lane split follow-up:
`compact_country_divisions_land_combats_map_detail_split_economy_speed5_20260630_v1`
kept the same 12-domain bundle but split the former shared economy worker into
separate `fuel_live` and `production_live` compact workers. The run advanced
262 game hours with 263 frame-end records, 0 skipped, full per-domain coverage,
no parse/order errors, no worker-panic, bridge-unavailable, source-unavailable,
or partial-data flags, avg frame 61.468 ms, p95 90.135 ms, max 117.487 ms,
62,020,850 raw stream bytes, and an independently verified `.battleplan`.
The split did not change the compact file contract; it proved `fuel_live` is
cheap at p95 3.218 ms and that `production_live` is the current country-domain
headroom limiter at p95 86.877 ms.

Weather-region implementation status:
The collector now has a compact `weather_live` region stream (domain 24) that
resolves `CWeatherManager`, validates `CWeatherManager+0x40` as 305 inline
`SWeatherPerRegion` rows, and emits raw phenomenon, low-byte weather flags,
active-vector signal, intensity, active-modifier count, `next_weather_change_raw`,
and scaled region temperature inputs. Smoke proof
`compact_weather_region_decode_smoke_20260630_v1` emitted 305 rows, decoded with
no parse/order/source/partial/worker failures, read weather in 54 us, wrote
9,976 compact bytes, and verified the `.battleplan`.

Weather-region speed-five completion:
`compact_country_divisions_land_combats_map_detail_weather_region_speed5_20260630_v3`
reran `weather_live` in the full compact bundle on the advancing live game. It
covered 13 expected domains (`fuel_live`, `production_live`, `focus_live`,
`tech_live`, `politics_live`, `politics_residuals`, `decisions_live`,
`diplomacy_live`, `divisions`, `land_combats`, `map_control`, `map_detail`, and
`weather_live`), advanced 57 game hours with 58 frame-end records, skipped 0
game hours, had full per-domain coverage, no parse/order errors, no
worker-panic, bridge-unavailable, source-unavailable, or partial-data flags, avg
frame 22.567 ms, p95 24.484 ms, max 79.348 ms, 2,983,908 raw stream bytes, and
an independently verified `.battleplan`. `weather_live` observed 17,690 region
rows, emitted 394 changed rows, and read at p95 123 us. This promotes the
current weather compact row contract to DLL/Rust speed-five complete.

Weather root hardening completion:
`compact_country_divisions_land_combats_map_detail_weather_region_anchor_speed5_20260630_v2`
replaced the production weather root lookup with the anchored path
`u64[module+0x33048C0] -> CGameState`, then `u64[CGameState+0x688] ->
CWeatherManager`. The DLL still validates the manager vtable
`module+0x2950AF0`, the `+0x40` region vector size/capacity of 305, and
`SWeatherPerRegion` row vtables/ids before emitting. The run covered the same
13 compact domains, advanced 56 game hours with 57 frame-end records, skipped 0
game hours, had full per-domain coverage, no parse, order, source, partial,
bridge, or worker failures, avg frame 22.319 ms, p95 26.714 ms, max 68.701 ms,
3,084,775 raw stream bytes, and an independently verified `.battleplan`.
`weather_live` observed 17,385 region rows, emitted 396 changed rows, and read
at p95 87 us. The old CWeatherManager heap/vtable singleton discovery is now
retained only as a reverse-engineering fallback, not the production compact
recorder path. The same checkpoint also fixed and proved the `map_detail`
schedule alias so it triggers the full `state_detail` plus `buildings_live`
compact detail stream.

Long anchored compact soak:
`compact_country_divisions_land_combats_map_detail_weather_region_anchor_soak_speed5_20260630_v1`
ran the same anchored 13-domain production-shaped bundle for 120 seconds. It
advanced 222 game hours with 223 frame-end records, skipped 0 game hours, had
full per-domain coverage, no parse/order/source/partial/bridge/worker failures,
avg frame 22.113 ms, p95 24.463 ms, max 69.586 ms, 9,180,014 raw stream bytes,
and an independently verified `.battleplan`. `divisions` observed 298,132 roots
and emitted 69,187 changed rows with read p95 22.296 ms; `map_detail` observed
3,232,162 rows with read p95 22.394 ms; `weather_live` observed 68,015 region
rows with read p95 102 us.

Division template-id scalar completion:
`compact_country_divisions_template_id_speed5_20260630_v1` widened the compact
`divisions` row with `template_id` from the validated
`CDivision+0x3A8 -> CDivisionTemplate+0x0C` path, guarded by template vtable
`module+0x29291D0`, type tag `52`, and a bounded nonzero id before emission.
It covered the full 13-domain compact bundle, advanced 56 game hours with 57
frame-end records, skipped 0 game hours, had full coverage, no parse/order/
source/partial/bridge/worker failures, avg frame 22.930 ms, p95 25.829 ms, max
72.687 ms, 3,025,600 raw stream bytes, and an independently verified
`.battleplan`. A byte-level compact-stream audit found 19,222 division rows with
the template flag and nonzero template ids, with 0 flag/value mismatches. This
completes the scalar `template_id` part of division replay state on the current
build; template catalog rows, per-division held-equipment inventory, exact
manpower, assignment, and order geometry remain separate segments. Exact
division manpower is not yet pinned and must not be called DLL-complete until a
live CDivision manpower offset or a clearly labeled derived contract is proven.

Earlier no-cold proof on the restarted vanilla process resolved 1898 division
bodies and 417 still-live army bodies from cache, advanced 12 game hours in
52.266 wall seconds, stored 13 verified hourly snapshots, and emitted raw
`clock`, `divisions`, `army_orders`, and `land_combats` streams with no raw
gaps. A
stricter raw military proof
`replay/snapshots/_collector_session_raw_military_gate/20260629T194359Z_pid43412`
advanced 12 game hours with `hourly_requirement.met = true` in 62.688 wall
seconds, verified replay output, and emitted zero raw gap domains while proving
`divisions`, `army_orders`, `land_combats`, and `oob_command` together. That run
emitted 12002 raw division deltas, 6189 `division_org_strength_delta`, 2109
`division_movement_started`, 38 `division_movement_stopped`, and 19
`division_combat_ref_changed` events. Its cold preflight was intentionally
allowed only to refresh caches outside the hot loop: division discovery took
26.531s, army discovery took 27.516s, and OOB command discovery took 135.703s,
so this is a capture-cadence proof, not an acceptable normal startup path.
The first no-bridge compact division-root proof attempt on 2026-06-30 exposed a
collector lifetime bug and then proved blind heap/vtable scanning is the wrong
production strategy: after restart, the scan-backed run advanced 101 game hours
with 0 skipped but emitted 0 division rows and reported source unavailable.
That scan code has been removed from the compact path; it remains negative
evidence only.
The DLL now also accepts an `army_body_bridge` populated by
`--preflight-army-orders` and emits raw `army_orders` deltas/events from
revalidated army bodies at a 250 ms sampling floor. The first slice captures
order type, slot index, and exact type-5 target state sets; non-type-5 geometry
is explicitly marked unsupported until those layouts are pinned. The army
preflight now salvages individually valid cached army rows instead of dropping
the whole bridge when a few cached army objects go stale. `army_order_updated`
is unit-tested in the Rust tracker for same-order target mutation, but remains
unverified in the live game because the live proof did not edit an existing
sampled order; it needs a fixture where an offensive order is
redrawn/resized/retyped after the collector has taken its baseline sample. A
later cache-backed fixture run
`replay/snapshots/_collector_session_raw_military_gate/20260629T200009Z_pid43412`
advanced 36 game hours in 45.266 wall seconds, resolved 580 army bodies from
cache in 0.078s, verified replay output, emitted 519 army-order events, observed
one `army_order_removed`, and reported zero raw gap domains. Raw-stream
inspection found no same-army/same-slot replacement near that removal, so this
evidence proves removals but not hidden replacement-as-update behavior. A
follow-up cache-backed speed-five run
`replay/snapshots/_collector_session_raw_military_gate/20260629T201212Z_pid43412`
advanced 48 game hours in 55.031 wall seconds, resolved 496 army bodies from
cache in 0.078s, verified replay output, emitted 463 adds and 13 removals with
zero raw gap domains, and still failed `--require-army-order-raw-updates`. Do
not move this segment to done until the live fixture edits an already-sampled
type-5 offensive order and the strict update gate passes. A second fixture
window
`replay/snapshots/_collector_session_raw_military_gate/20260629T201839Z_pid43412`
advanced 49 game hours in 54.188 wall seconds, resolved 425 army bodies from
cache in 0.078s, verified replay output, emitted 390 adds and one removal with
zero raw gap domains, and also failed the update gate.

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

Current status: raw `divisions` movement deltas and movement/org-strength/
combat-ref event frames are live-proven through cached division body pins. Land
combat now has a pinned active manager root at
`CGameState+0x260` and both Python and Rust paths use bounded manager reads
instead of heap discovery in the hot loop. The Rust collector emits raw
`land_combats` `domain_delta` samples with stable
`land_combat:<combat_id>:<generation>` identities, province/age scalars,
nullable attacker/defender division lists, `delete` ops with `after:null` for
disappeared combats, bounded gap markers for manager/layout validation failures,
explicit lifecycle `event` frames for started/updated/ended combats, generation
split end reasons, and pointer/tag-db leak rejection. Invalid manager slots are
now skipped when other rows validate, matching the Python combat reader. A
2026-06-29 no-cold live proof against pid 43412 advanced 12 game hours in
39.266 wall seconds with replay verification, emitted 2485 raw domain-delta
frames, 2419 raw event frames, 42 `land_combat_started`, 2368
`land_combat_updated`, and 9 `land_combat_ended` events, and reported zero raw
gap frames. The stricter raw military proof
`replay/snapshots/_collector_session_raw_military_gate/20260629T194359Z_pid43412`
also passed the new `--require-land-combat-raw-lifecycle` gate at speed five,
with 160 raw `land_combats` deltas, 8 starts, 151 updates, 1 end, and zero raw
gap domains across 12 game hours. The remaining land-combat unlock is not "find
the objects"; it is richer participant/tick canonical events, longer full-war
soaks, and timeline/app ingestion. Naval combat now has the same raw-frame shape for
the canonical `naval_air_combats` domain when the recorder supplies
`naval_combat_body_bridge` rows: the DLL revalidates active `CNavalCombat`
bodies, emits side-level summaries and started/updated/ended lifecycle events,
and produces bounded gap rows when the bridge/layout fails closed. The same
bridge now also feeds `naval_combat_members`, a pointer-free per-`CFEXMember`
stream for ship/convoy identity, side membership, designed max-strength raw
value, battle-slot index, state bitmask, and engagement-weight raw value.
Current HP/org still require a separate `CShip` registry bridge; targets,
aggregate attack rows, damage ticks, hit counters, sinking events, and UI
damage totals remain resisted until they have live proof. The recorder-side
`--preflight-naval-combat` producer can populate that bridge from active
combats already present at attach time. A 2026-06-29 no-cold live proof against
pid 43412 resolved 3 active naval combats during preflight in 26.031s outside
the hot loop, advanced 12 game hours in 46.953 wall seconds, verified replay
output, emitted raw `naval_air_combats` and `naval_combat_members`, observed 3
`naval_combat_started` and 360 `naval_combat_updated` events, and reported zero
raw gap frames. `naval_combat_ended` still needs a fixture where a bridged
naval battle ends during capture. The next naval unlocks are a root/hook-derived
discoverer for newly born combats, CShip enrichment into active combat members,
exact end/delete fixture proof, damage/sinking deltas, and longer soaks. Compact
ship hull telemetry and basic fleet/task-force hierarchy are no longer
bridge-gated for the verified `870ebf` build: `compact_ships_native_870ebf_20260701_010232`,
`compact_full_plus_ships_870ebf_20260701_010320`,
`compact_oob_ships_native_870ebf_20260701_nav_oob_v2`, and
`compact_full_plus_naval_oob_870ebf_20260701_v1` prove the DLL-native
country/fleet/task-force/ship path at speed five/current equivalent with zero
skipped game hours and no source/partial/bridge/worker failures. The older raw
OOB/ship proof
`replay/snapshots/_collector_session_air_ship_oob_gate/20260629T210118Z_pid43412`
remains useful historical/oracle evidence only; it hit `ship_body_cache` in
0.125s, supplied 1309 bridge rows, emitted 22249 raw `ships` deltas / 22248
object rows, advanced 12 game hours in 57.297 wall seconds, stored verified
hourly snapshots, and reported zero raw gap domains. The same historical proof
also captured 4704 OOB command roots and 17705 `oob_command` deltas over 12 game
hours after an intentionally cold 135.703s discovery; that bridge evidence is
now an oracle, not the compact production path for fleet/task-force membership.
Army leader detail now has a stricter
same-row proof: `--require-oob-command-leader-detail` passed on the no-cold run
`replay/snapshots/_collector_session_oob_leader_gate/20260629T215306Z_pid43412`,
which advanced 12 game hours in 54.110 wall seconds, verified replay output,
emitted 19842 raw `oob_command` deltas with zero raw OOB gaps, and captured
7485 exact army leader-detail rows with 15911 trait tokens. This proves raw
army leader id/status, skill, and assigned trait tokens only; save-shaped OOB
nesting, portraits, names, temporary/captured traits, naval leaders, and global
commander rosters remain open. Army orders now have a raw `army_orders`
stream from `army_body_bridge`, with
`army_order:<army_id>:<generation>` identities, add/remove lifecycle proof,
bounded bridge gaps, exact type-5 target states, and unsupported status for
unvalidated non-type-5 geometry. The next army-order unlocks are a live
`army_order_updated` fixture while issuing/editing orders, semantic event
splits, non-type-5 geometry, and timeline/app ingestion. Air wings now have a
raw `air_wings`
stream from `air_wing_body_bridge`, with
`air_wing:<TAG>:<wing_id>:<generation>` identities, observed/update/lost
events, bounded bridge gaps, and 250 ms sampling of count, mission mask,
mission region, and effective active planes. Current pid 43412 proof
`replay/snapshots/_collector_session_air_wing_gate/20260629T205254Z_pid43412`
passed the strict `--require-air-wing-raw-telemetry` gate: it advanced 12 game
hours in 55.875 wall seconds, verified replay output, hit `air_wing_body_cache`
in 0.094s, supplied 585 bridge rows, emitted 11115 raw `air_wings` deltas,
observed 585 `air_wing_observed` and 10530 `air_wing_updated` events, and
reported zero raw gaps. The next air unlocks are
exact airbase/variant joins, region aggregate rows, air-loss/attrition proof,
and a decision on which air-superiority values are exact state versus computed
UI proxies.

### Phase 7: Focus, Tech, Decisions, Projects, Politics

Deliver:

- Focus pause/event markers plus remaining research/decision/project/politics
  domains promoted to collector or exact derived sources.
- Focus/tech/decision event timelines.

Acceptance:

- Country deep-dive views can scrub political/research/focus/project state from
  live data.

Current status: the DLL now emits a bounded `country_misc` slice from validated
country-object rows: `convoys_destroyed`, `accidents_score`, `ace_count`, and
nullable `cosmetic_tag`. It also emits scheduled `politics_live` rows from
`CPoliticalStatus`: ruling party, party popularities, ordered active idea
tokens, typed law buckets, broad non-law `ideas_other`, narrower
`national_spirits`, complete `idea_groups`, active cabinet/designer
`advisor_ideas`, idea count, election frequency, party count, and
low-confidence diagnostics, active-ruler rows folded into
`countries.*.commanders.leader`, plus scheduled `politics_residuals` rows for
`last_election`, `elections_allowed`, `appointed_advisors`,
`appointed_advisor_details`, `appointed_advisor_slots`, and
`appointed_advisor_ledgers` when validated static contracts are supplied. The
adapter validates and folds these into
`countries.*.misc` and `countries.*.politics`; live proof observed
`country_misc`, `politics_live`, `politics_residuals`, scheduled advisor
leaves, typed laws, `idea_groups`, `advisor_ideas`, `ideas_other`, and
`national_spirits` with no session errors and no worker-poll politics payloads.
Current strict politics classification proof
`replay/snapshots/_collector_session_politics_idea_gate/20260629T211245Z_pid43412`
emitted 12 scheduled `politics_live` frames across 12 game hours, supplied a
6474-row vanilla `common/ideas` contract, folded 4248 active-idea rows, 4248
law rows, 4248 complete idea-group rows, 1104 advisor/cabinet idea rows, 4248
broad non-law idea rows, and 4248 national-spirit rows, verified replay output,
and kept heavy payloads off lightweight worker polls.
The adjacent advisor-residual gate
`replay/snapshots/_collector_session_politics_advisor_gate/20260629T212028Z_pid43412`
emitted 13 scheduled `politics_residuals` frames across 12 game hours, supplied
a 3722-token vanilla `common/characters` advisor contract, observed 4602 country
rows with all appointed-advisor leaves present, verified replay output, and
kept residual payloads off lightweight worker polls. The same proof measured
zero populated appointed-advisor rows, so a non-empty appointment fixture is
still required.
Follow-up active-ruler proof
`replay/snapshots/_collector_session_politics_leader_gate/20260629T171216Z_pid43412`
emitted 13 scheduled `politics_live` frames across 12 game hours, folded 4095
`commanders.leader` rows into the replay store, and kept multi-entry leader
vectors omitted rather than guessing.
The DLL also emits scheduled
`decisions_live` status counts and active timed-decision rows, plus scheduled
`focus_live` core focus state from validated `CCountry` child roots; live proof
emitted 3 scheduled focus frames and 1065 country focus rows with no worker-poll
focus payloads. Scheduled `tech_live` now folds completed-tech count and
research slot capacity plus raw active-research rows from `CTechnologyStatus`
and its bounded `CResearchSlot*` vector into the replay file. Research
token-name/progress/ETA resolution, project progress/definition joins, focus
pause/event markers, deeper targeted decision/cooldown internals, and deeper
political/cabinet state still need dedicated manager/root contracts before
promotion.

2026-06-29 proof-gate update for the focus/decision/project root slice:
the recorder manifest now exposes scheduled `decisions_live` frame and payload
counts alongside the existing focus/project counters, and
`live_capture.collector_proof_gate` can require scheduled collector frames for
`focus_live`, `decisions_live`, and `projects_live`. Unit tests are executable
scaffolding only; a segment is live-verified only after a `collector_session`
run against the real the game process with
`hourly_requirement.met = true`, `replay_verified = true`, zero heavy payloads
on worker-poll frames, and:

```powershell
python -m live_capture.collector_proof_gate replay/snapshots/_collector_session_project_roots_bootstrap/<run-dir>/session_manifest.json --min-game-hours 12 --require-scheduled-domain focus_live --require-scheduled-domain decisions_live --require-scheduled-domain projects_live --require-projects-live-payload
```

If `--preflight-special-projects` cannot supply a bridge from cache, classify
`projects_live` as blocked/unverified for this proof. A cold-scan run may
populate the cache, but the acceptance proof should be rerun without
`--allow-special-projects-cold-scan`. The no-cold acceptance run should keep
`--max-wall-s` larger than `--duration-s`; otherwise the session wall-budget
guard can close before the bounded worker starts.

Live proof completed on 2026-06-29 against running PID 43412 with the pinned
16022851 executable hash. A read-only special-project cold bootstrap resolved
and cached 21,533 bridge rows in 79.313s outside the collector hot loop. The
acceptance rerun used that cache without `--allow-special-projects-cold-scan`;
the preflight reported `status = resolved`, `source = special_projects_cache`,
`row_count = 21533`, `elapsed_s = 0.391`, and `hot_loop_heap_scan = false`.
`collector_proof_gate` passed with `game_hours_advanced = 12`,
`stored_snapshot_count = 13`, `hourly_requirement.met = true`,
`replay_verified = true`, `heavy_payload_on_lightweight_frames = 0`, and
scheduled collector-frame counts `focus_live = 12`, `decisions_live = 12`,
`projects_live = 12`. Payload summary: 240 facility rows, 204 worked-project
rows, and 0 measured scientist rows across the scheduled snapshots in that
active game state. This segment is therefore live-verified at hourly-or-better
cadence only for cache-backed, no-cold recorder runs; any cache miss or disabled
bridge reverts `projects_live` to blocked/unverified until a fresh bootstrap is
rerun and followed by another no-cold proof. A current-game rerun refreshed that
proof after a cache miss: the cold bootstrap
`replay/snapshots/_collector_session_project_roots_bootstrap/20260629T203510Z_pid43412`
resolved 21,549 bridge rows in 81.859s, and the no-cold acceptance rerun
`replay/snapshots/_collector_session_project_roots_bootstrap/20260629T203922Z_pid43412`
reused the same `out-root` cache with `status = resolved`,
`source = special_projects_cache`, `cache.status = hit`, `row_count = 21548`,
and `elapsed_s = 0.375`. The gate passed after 12 game hours in 38.969 wall
seconds with replay verification, 14 scheduled frames each for `focus_live`,
`decisions_live`, and `projects_live`, 4956 focus country rows, 4956 decision
country rows, 392 facility rows, 392 worked-project rows, 0 scientist rows, and
zero heavy/project payloads on worker-poll frames. A stricter rerun then proved
the payload gate itself:
`replay/snapshots/_collector_session_project_roots_bootstrap/20260629T224333Z_pid43412`
hit the scoped no-cold cache, supplied 21496 bridge rows, passed
`--require-projects-live-payload`, advanced 12 game hours in 40.781 wall
seconds with replay verification, emitted 12 scheduled frames each for
`focus_live`, `decisions_live`, and `projects_live`, folded 4236 focus country
rows, 4236 decision country rows, 288 facility rows, 348 worked-project rows,
0 scientist rows, and kept project payloads off lightweight frames. This keeps
the focus/decision/project root segment classified as hourly-or-better verified
only for cache-backed no-cold recorder runs.

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
3. Replace the remaining country-record heap discovery with root-anchored or
   hook-derived record identity; keep country cold scans as debug/cache-populate
   tools only.
4. Promote country identity/scalars into speed-five-safe collector frames using
   bounded cache/root preflight.
5. Add the first non-clock domain tranche. The practical first three are light
   country economy/scalars, state/province map state, and divisions/OOB. The
   next implementation slice is root-anchored country identity plus country
   scalars; do not extend the heap-scanning `country_object_scan` path into the
   hot loop.
6. Use the native app's raw-frame diagnostics as the entry point for exact
   movement/combat/event overlays.
7. Run a longer speed-five proof and reopen the `.battleplan` in the native app.
8. Continue domain promotion in this order: first-three tranche, combat/movement,
   focus/research/decisions/project depth, diplomacy/intel/logistics/weather,
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
