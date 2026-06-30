# Raw Delta/Event Stream Plan

## Purpose

Hourly canonical snapshots are necessary for durable replay state, but they are
not sufficient for speed-five combat, movement, order, and control replay. The
collector preserves raw frame observations separately from hourly snapshot
selection, and finalized `.battleplan` files now carry that raw stream as a
manifest-covered optional payload for bounded `domain_delta`, `event`, and `gap`
frames.

Runtime remains save-free. Save readers stay useful as development fixtures and
oracle checks only.

## Current Boundary

- `live_capture/collector_frames.py` already reserves `domain_delta`, `event`,
  `diagnostic`, and `gap` frame types in `FRAME_TYPES`.
- `live_capture/collector_session.py` currently records only
  `collector_started` and `snapshot` frames as canonical snapshots; other frame
  types are skipped as `non_snapshot_frame`.
- `collector_frames.jsonl` is the raw preserved stream. Canonical snapshots stay
  one frame per unique game hour.
- `.battleplan` finalization embeds the stream at
  `payload/<game>/collector_raw/collector_frames.jsonl`, marks it with
  `entries[*].role == "collector_raw_stream"`, and exposes
  `manifest.raw_streams[]` metadata. Native readers can open/verify these
  containers, enumerate/read verified raw payloads, and `scrub::Timeline` now
  indexes gameplay raw frames as a sibling to the hourly snapshot spine.
- `replay/diff_engine.py::build_timeline` coalesces same-hour snapshots before
  diffing, so sub-hour observations must not be represented only as normal
  snapshots.

## Frame Model

Every raw delta/event frame should carry:

- `type`: `domain_delta`, `event`, or `gap`.
- `sequence`: producer-monotonic transport sequence.
- `session_sequence`: worker-local sequence when present.
- `tick_ms`: producer monotonic tick.
- `date`: current game date string when readable.
- `hour_index`: parsed game-hour integer when readable.
- `sample_ms`: nominal sampler interval.
- `capture_boundary`: e.g. `worker_poll`, `hook_callback`, or
  `bounded_reread`.
- `domain`: one canonical schema domain.
- `entity_key`: stable replay key, never raw heap address.
- `source`: `C` for collector-produced live data.
- `confidence`: `exact`, `sampled`, `proxy`, or `gap`.
- `payload`: domain-specific data.
- `diagnostics`: optional reject counts, validation status, or drop reason.

Raw heap addresses may appear in transient debug logs but should not enter the
durable replay schema. If body continuity is required, use an internal
generation key such as `combat_id + born_sequence + age_start` and surface only
the stable replay key.

## Event Identity

Raw events need stable dedupe identities so they can supersede interval
snapshot diffs without erasing distinct repeated changes. Use the existing diff
engine identity rules as the baseline, then add raw-stream fields only where the
same event type can occur more than once inside one hour.

Recommended identities:

- Province/state control: `(type, key, from, to)`.
- Movement lifecycle: `(type, division_uid, from, to, order_generation)`.
- Movement progress: `(type, division_uid, source, target, progress_bucket)`.
- Land combat lifecycle: `(type, combat_key, generation)`.
- Combat participant changes: `(type, combat_key, generation, side,
  participant_hash)`.
- Naval ship damage/sinking: `(type, combat_key, ship_instance_id,
  generation)`.
- Army order changes: `(type, army_id, order_key, order_generation)`.
- Air region deltas: `(type, region_id, side_or_owner, mission_mask)`.
- Gap frames: `(domain, gap_kind, start_sequence, end_sequence)`.

`generation` is collector-local continuity metadata. It should be used to avoid
collapsing id reuse, but it does not need to be exposed as a permanent gameplay
identifier in user-facing views.

## Minimal Payload Shapes

The first implementation should validate only a small common envelope plus
domain-specific payloads needed by tests. Do not try to model every reader field
up front.

`event` payload:

- `event_type`: replay event type or raw-only event type.
- `key`: entity key.
- `from`: optional previous value.
- `to`: optional new value.
- `subject`: optional TAG or entity owner.
- `detail`: optional object.

`domain_delta` payload:

- `op`: `upsert`, `delete`, or `sample`.
- `before`: optional compact previous value when available.
- `after`: compact measured value.
- `changed_fields`: list of changed field names.

`gap` payload:

- `gap_kind`: `dropped_frames`, `validation_failed`, `overload`,
  `layout_invalid`, `race_detected`, or `unsupported`.
- `domain`: affected domain.
- `start_sequence` / `end_sequence`: sequence range when known.
- `reason`: short machine-readable reason.
- `detail`: optional bounded diagnostics.

## Required Domains

### Movement

Source candidates:

- `memory_reader/division_live/division_live_reader.py`
  - `locate_division_bodies()`
  - `read_divisions_from_bodies()`
  - `_decode_body()`
- `memory_reader/division_live/division_movement_delta.py`
  - `DivisionMovementDeltaTracker`
  - bounded raw `domain_delta` payload contract and tests

Current measured constraint:

- Cold CDivision body discovery is not a recording-loop operation. On the live
  2026-06-28 speed-five session it took about 26.8s to locate 1806 division
  bodies.
- Warmed re-reads from the pinned body map took about 22ms median across 20
  samples. At the observed speed-five rate, that is sub-hour and sub-minute
  granularity, but not a reason to put heap discovery in the hot path.
- The old raw/JSON runtime target used a `division_body_bridge` supplied before
  recording: Python preflight did the expensive locate, the DLL revalidated each
  bridged body in-process every sample, and raw movement deltas were emitted on
  the same pipe stream as clock deltas. The production compact path must not use
  that Python preflight or a DLL heap/vtable scan. The compact path now uses the
  anchored native root
  `CGameState+0x310 -> CCountry+0x290 unit wrappers -> wrapper+0x10 CDivision`
  for the current vanilla build.
- First live proof: on 2026-06-28, the bridge ran successfully at speed five.
  A 50 ms smoke preserved 2155 raw movement frames; a shorter 10 ms smoke
  preserved 1816 raw movement frames and 39 clock frames, wrote a verified
  replay, and had zero session errors. The manifests recorded only bridge
  counts/status, not body addresses.
- Expanded telemetry proof: the same bridge now emits bounded `organisation`,
  `strength`, `combat_ref_count`, and sampled `in_combat` fields. A 10 ms smoke
  preserved 3229 division frames with those fields present and no raw-stream
  sequence gaps. Treat `in_combat` as body-ref telemetry until joined with the
  combat object stream.
- Division lifecycle event proof: the DLL now emits explicit division raw
  `event` frames for `division_movement_started`,
  `division_movement_stopped`, `division_org_strength_delta`, and
  `division_combat_ref_changed` from the same cached body samples. First-seen
  moving divisions emit only the movement-start event, so org/strength and
  combat-ref events are not duplicated from initial all-field samples. Live
  2026-06-29 warm-cache proof bridged 1564 division bodies, advanced one game
  hour in 9.31 wall seconds, preserved 846 division deltas and 842
  `division_movement_started` events, with zero first-observation org/combat
  noise events.
- Current compact verification state: the no-bridge
  `compact_divisions_anchored_telemetry_validroot_speed5_20260630_v1` proof
  refreshed the anchored division root every compact frame and advanced 665
  game hours with 666 frame-end records, 0 skipped, 91,642 typed division rows,
  `division_root_source = "dll_native"`, no bridge/source/partial failures,
  division read avg 7.505 ms, p95 8.002 ms, max 15.187 ms,
  ordered/decodable compact stream, and an independently verified
  `native_session.battleplan`. The compact row now carries division id,
  location/source/target/progress, organisation/strength buckets,
  combat-ref/in-combat state, owner tag-db id, parent army id, raw experience,
  max/current supply, supply gain, and fuel. This proves the widened anchored
  compact division row at hourly-or-better cadence for the current vanilla
  build; it does not yet cover every possible movement mode,
  template/equipment/manpower, or full order semantics. The raw JSON event
  sidecar remains movement/org/strength/combat-ref focused until these newer
  compact fields are given typed lifecycle events.
- Current combined-load state:
  `compact_country_divisions_land_combats_map_mapdelta_speed5_20260630_v1`
  decoded, ordered, packed, and verified the widened row set inside the
  11-domain bundle after `map_control` moved to cached topology plus
  baseline/delta data. It advanced 349 game hours with 350 frame-end records, 0
  skipped, full per-domain coverage, no parse/order errors, no
  source/partial/bridge failures, avg frame 31.373 ms, p95 37.631 ms, and an
  independently verified `native_session.battleplan`. `map_control` observed
  5,072,900 full rows but emitted 15,418 changed data rows; map read p95 was
  12.583 ms. This is the current completed combined compact speed-five proof for
  the implemented domains, not for still-missing replay layers such as full
  order geometry, template/equipment/manpower, weather, and naval/air coverage.
- Follow-up compact division delta state:
  `compact_country_divisions_land_combats_map_divdelta_parallel_speed5_20260630_v1`
  kept the same 11-domain compact set and moved `divisions` to a full baseline
  followed by compact delta data records. Disappeared division ids now emit
  explicit zero-filled delete/tombstone rows, and large root sets are sampled by
  bounded internal DLL workers before deterministic ordered emission. The proof
  advanced 345 game hours with 346 frame-end records, 0 skipped, full
  per-domain coverage, no parse/order/source/partial/bridge failures, avg frame
  34.743 ms, p95 52.766 ms, division read p95 37.112 ms, 62,786,985 raw stream
  bytes, and an independently verified `native_session.battleplan`.
- Compact map-detail state:
  `compact_map_detail_buildings_cached_speed5_20260630_v1` moved
  `state_detail` and `buildings_live` into the compact DLL/Rust path as
  `map_detail`: state resource deposits plus state/province building levels,
  baseline-plus-delta records, bounded province workers, and batched
  building-vector metadata reads. The isolated proof advanced 259 game hours
  with 260 frame-end records, 0 skipped, full coverage, no parse/source/partial
  failures, map-detail read p95 23.933 ms, 473,672 raw stream bytes, and an
  independently verified `native_session.battleplan`.
- Combined map-detail state:
  `compact_country_divisions_land_combats_map_detail_speed5_20260630_v1` added
  `map_detail` to the previous compact bundle and passed with 12 expected
  domains, 289 game hours advanced, 290 frame-end records, 0 skipped, full
  coverage, no parse/source/partial/bridge failures, avg frame 54.922 ms, p95
  89.906 ms, 56,044,696 raw stream bytes, and an independently verified
  `native_session.battleplan`.
- Earlier raw-event verification state: live-verified on the restarted vanilla
  process (pid 43412). A cold bootstrap outside the hot loop populated the
  current-process division and army caches, then a no-cold 12-game-hour run used
  cache hits only, advanced 12 game hours in 52.266 wall seconds, stored 13
  verified hourly snapshots, preserved the raw stream, and emitted 6903
  `division_org_strength_delta` events plus 85
  `division_combat_ref_changed` events with no raw gap domains. This proves the
  division org/strength raw stream at hourly-or-better replay cadence on that
  live process, but it depended on prefilled body caches. A stricter
  2026-06-29 raw military gate
  `replay/snapshots/_collector_session_raw_military_gate/20260629T194359Z_pid43412`
  advanced 12 game hours with `hourly_requirement.met = true` in 62.688 wall
  seconds, verified replay output, emitted zero raw gap domains, and captured
  12002 raw division deltas, 6189 `division_org_strength_delta`, 2109
  `division_movement_started`, 38 `division_movement_stopped`, and 19
  `division_combat_ref_changed` events. That run allowed cold preflight only to
  refresh caches outside the hot loop; it is a capture-cadence proof, not an
  acceptable normal startup-time proof.

Cadence:

- Start at 10 ms during speed-five proof runs, but treat it as a load target
  requiring explicit backpressure/coalescing counters before long sessions.
- Emit deltas only on changed movement state, target, source, progress bucket,
  location, organisation bucket, strength bucket, combat ref count, or sampled
  in-combat state.
- Keep full division snapshots hourly; use deltas for animation and exact order.

Events/deltas:

- `division_movement_started`
- `movement_progress`
- `division_movement_stopped`
- `movement_arrived`
- `movement_cancelled_or_retargeted`
- `division_combat_ref_changed`
- `division_org_strength_delta`

Notes:

- `movement_source_province` is confirmed for normal movement via the move-order
  object. Retreat, strategic redeploy, naval transport, and invasion movement
  need live fixture coverage before being marked exact.

### Land Combat

Source candidates:

- `memory_reader/combat/combat_enum.py`
  - `CombatReader.enumerate_land_combats()`
- `memory_reader/combat/combat_sampler.py`
  - `BattleTracker.ingest()`

Current measured constraint:

- Active land combats are now rooted at `CGameState+0x260`. The Python proof
  defaults to the bounded manager walk and keeps the old `CLandCombat`
  heap-vtable scan only as a research cross-check. Live 2026-06-29 proof:
  22 manager rows, 22 heap-scan rows, exact same `(combat_id, body)` set.
- The collector now emits bounded `land_combats` `domain_delta` samples from
  that active-manager root:
  `entity_key = land_combat:<combat_id>:<generation>`, with `combat_id`,
  `generation`, `location`, optional `day`/`duration`, and nullable
  `attacker_divisions` / `defender_divisions`. A `null` side means validation
  failed or is unknown; an empty list means a measured empty side.
- Durable raw combat frames must not contain heap addresses, body pointers,
  tag-db ids, or pointer-like changed fields. Body continuity, if needed, stays
  internal and is represented only by the generation component.

Bridge contract now implemented for the first land-combat slice:

- The DLL validates the active manager vector, each body vtable/type/id, the
  front object, registry identity, side vtables, side counts, and CDivision
  participant references before emitting a sample.
- The collector keeps body continuity internal and emits only stable
  `land_combat:<combat_id>:<generation>` keys. Missing/disappeared combats emit
  a `delete` op with `after:null` for the last known generation.
- Invalid active-manager slots are skipped when other rows validate, matching the
  Python proof reader. If no rows validate, active-manager validation failures
  emit bounded `confidence:"gap"` samples keyed as
  `land_combats:gap:<gap_kind>` instead of silently producing no coverage marker.
- The current DLL also emits explicit lifecycle `event` frames for
  `land_combat_started`, `land_combat_updated`, and `land_combat_ended`. A
  2026-06-29 no-cold live proof against pid 43412 advanced 12 game hours in
  39.266 wall seconds with replay verification, 2485 raw domain-delta frames,
  2419 raw event frames, 42 starts, 2368 updates, 9 ends, and zero raw gap
  frames. The stricter raw military proof
  `replay/snapshots/_collector_session_raw_military_gate/20260629T194359Z_pid43412`
  also passed `--require-land-combat-raw-lifecycle`, advancing 12 game hours in
  62.688 wall seconds and observing 160 raw land-combat deltas, 8 starts, 151
  updates, 1 end, and zero raw gap domains.
- Replay timeline promotion is now implemented for the lifecycle endpoints:
  `land_combat_started` maps to canonical `combat_started`, and
  `land_combat_ended` maps to canonical `combat_ended`. `land_combat_updated`
  remains a raw frame until the canonical combat-tick/participant-delta schema is
  widened.
- Remaining hardening before this is replay-truth quality: richer
  participant/tick canonical events, longer full-war soaks, and app ingestion
  for the `land_combats` raw stream.

Cadence:

- 10 ms raw sampling for active-combat proof mode.
- Longer intervals may be allowed only after missed-battle diagnostics show
  acceptable coverage at speed five.

Events/deltas:

- `combat_started`
- `combat_participants_changed`
- `combat_location_changed`
- `combat_tick_delta`
- `combat_ended`
- `combat_gap`

Notes:

- Combat id reuse must create a new internal generation when age regresses or a
  body continuity check fails.
- A side validation failure is `null`/unknown, not an empty participant set.
- A combat born and ended between samples remains unobservable and must stay a
  gap/confidence limitation.

### Naval Combat

Current DLL contract:

- The Rust collector accepts a `naval_combat_body_bridge` control contract:
  semicolon-separated `combat_id:body_ptr` rows produced outside the hot loop.
  It rejects malformed, duplicate, unaligned, out-of-range, or oversized bridge
  payloads before sampling.
- Each bridged `CNavalCombat` body is revalidated on every worker sample using
  the known naval-combat body, registry, side, member, and leader vtables.
  Bridge pointers stay internal; durable frames use stable
  `naval_combat:<combat_id>:<generation>` keys.
- The first emitted stream is the canonical `naval_air_combats` raw domain with
  side-level summaries: location, attacker/defender TAG sets, side ship counts,
  commander tokens when available, positioning values, side validation status,
  and lifecycle `event` frames for started/updated/ended.
- A second bounded stream now emits `naval_combat_members` domain deltas keyed
  as `naval_combat_members:naval_combat:<combat_id>:<generation>:<side>:ship:<instance_id>`
  for warships and
  `naval_combat_members:naval_combat:<combat_id>:<generation>:<side>:convoy:<group_index>:<member_index>:<slot_index>`
  for convoys. Each row is one active `CFEXMember` and carries combat key,
  side, group/member index, unit type tag, nullable ship instance id, convoy
  flag, owner TAG, display strings, designed max strength raw value,
  battle-slot index, state bitmask, and engagement-weight raw value.
- Validation exists on both sides: Rust bounds/sanitizes emitted rows and
  `live_capture.collector_frames` rejects pointer/tag-db leakage, unstable keys,
  bad TAGs, unbounded ship counts, bad positioning, and malformed gap rows.

Source candidates:

- `memory_reader/combat/combat_enum.py`
  - `CombatReader.enumerate_naval_combats()`
  - `CombatReader.enumerate_naval_ships()`
- `memory_reader/combat/naval_combat_stats.py`
  - `NavalCombatStats.read_combat_stats()`
- `memory_reader/oob_full_live.py`
  - `OobShipRegistry.build_index()`
  - `OobShipRegistry.read_ship()`

Ship/CShip telemetry:

- The Rust collector now accepts a separate `ship_body_bridge` control contract:
  semicolon-separated `instance_id:body_ptr` CShip rows produced outside the
  hot loop from the existing `OobShipRegistry` cache/preflight path.
- Each bridged CShip body is revalidated every sample with vtable
  `MODULE+0x2931998`, handle type `51`, handle high-id matching
  `instance_id`, and the `ref100_raw == 10000000` sentinel before any row is
  emitted.
- The raw `ships` domain uses stable `ship:<instance_id>` keys and carries
  pointer-free CShip stats: definition token when readable, category enum/label,
  `strength_raw` plus decoded absolute `strength_hp`, `organisation_raw` plus
  decoded `organisation`, `experience_raw` plus decoded `experience`, and
  `sunk_convoys`.
- Owner, fleet/task-force membership, ship name, refit/equipment variant,
  target, hit counters, and sinking events are intentionally not claimed from
  this row. Combat wrapper state remains in `naval_combat_members`; command
  hierarchy and names still require their own sources.
- Current `ships` verification state: live-verified on the restarted vanilla
  process (pid 43412) with the strict `--require-ship-raw-telemetry` proof
  gate. The no-cold run
  `replay/snapshots/_collector_session_air_ship_oob_gate/20260629T210118Z_pid43412`
  hit `ship_body_cache` in 0.125s, supplied 1309 bridge rows, emitted 22249 raw
  `ships` deltas / 22248 object rows, advanced 12 game hours in 57.297 wall
  seconds with `hourly_requirement.met = true`, verified the replay, and
  reported no raw gap domains. The raw stream sidecar can be large on combined
  ship/OOB runs, so long-session coalescing or lower-frequency ship sampling is
  still a recording-layer hardening item.

Cadence:

- Sample active naval combats at the high-frequency combat cadence.
- Sample global CShip stats from the preflight/cache bridge at a bounded
  sub-second cadence; avoid whole-navy scans in the hot path.

Events/deltas:

- `naval_combat_started`
- `naval_combat_updated`
- `naval_combat_ended`
- Later per-ship streams: `naval_combat_participants_changed`,
  `naval_ship_sunk`, exact damage/hit counters, and
  `naval_combat_damage_delta`.

Notes:

- Use the newer `memory_reader/combat` CNavalCombat path for active battles.
  The older `memory_reader/naval_air` naval-combat detector is useful as a
  negative/probe path, not the primary port target.
- The current DLL slice is bridge-gated. The remaining promotion step is a
  root-anchored active-combat discoverer; the current recorder producer
  `live_capture.collector_naval_combat_bridge` performs explicit preflight
  vtable enumeration, encodes active `combat_id:body_ptr` rows, and records only
  sanitized counts/status in manifests. It can bridge combats already active at
  attach time, but battles born after preflight need a later root/hook-derived
  discovery path.
- Current `naval_air_combats` / `naval_combat_members` verification state:
  live-verified for active-at-attach battles on pid 43412. A no-cold
  12-game-hour proof resolved 3 active naval combats during preflight in
  26.031s outside the hot loop, emitted both raw domains, advanced 12 game
  hours in 46.953 wall seconds, verified replay output, observed 3
  `naval_combat_started` and 360 `naval_combat_updated` events, and reported
  zero raw gap frames. `naval_combat_ended` remains fixture-dependent until a
  bridged battle ends inside a capture window.
- Side summaries are intentionally not per-ship combat rows. Per-ship rows need
  a separate bounded stream keyed by ship/combat so large battle arrays do not
  exceed the raw JSON frame envelope. That bounded stream now exists as
  `naval_combat_members`.
- The member stream deliberately does not claim current HP, organisation,
  target, aggregate attack rows, or UI damage totals. Current ship strength/org
  now come from the separate `ships` CShip bridge; damage history, hit counters,
  target state, and sinking semantics still need unpaused mid-tick proof before
  they are treated as exact.

Command-hierarchy membership:

- The first command-hierarchy DLL slice now emits raw `oob_command`
  domain-delta rows from a cache-first `oob_command_bridge` contract. The
  temporary control payload is semicolon-separated `kind:id:body_ptr` rows from
  `live_capture.collector_oob_command_bridge`; manifests store only sanitized
  counts/status.
- The current emitted rows are deliberately narrow but now span land and navy:
  `oob_command:army_group:<army_group_id>` rows expose bounded
  `member_army_ids`, `member_army_count`, and `member_army_status`;
  `oob_command:army:<army_id>` rows expose `army_group_id`, `parent_status`,
  bounded `member_division_ids`, `member_division_count`,
  `member_division_status`, `leader_id`, `leader_status`,
  `leader_skill`, `leader_traits`, `leader_trait_status`,
  `member_count_engine`, and `missing_member_count`;
  `oob_command:fleet:<fleet_id>` rows expose bounded `task_force_ids`,
  `task_force_count`, and `task_force_status`; and
  `oob_command:task_force:<task_force_id>` rows expose `fleet_id`,
  `parent_status`, bounded `ship_ids`, `ship_count`, and `ship_status`.
- Worker sampling does not sweep the heap. It revalidates CFleet/CTaskForce
  vtable + handle identity, resolves `CTaskForce+0x330` against bridged fleet
  bodies, and validates each inlined CShip pointer from
  `CTaskForce+0x338/+0x344` before emitting ship ids. Land rows revalidate
  army/army-group/division roots, map `CDivision+0xB0` parent armies, map
  `CArmy+0x1B8` parent army groups, and validate `CArmy+0x88` leader pointers
  before emitting leader ids, bounded skill values, and assigned trait tokens.
- This stream does not yet claim owner, command names, missions, locations,
  theatre membership, leader portraits, temporary/captured trait state, naval
  leaders, global commander rosters, or save-shaped nested OOB records.
  Invalid bridge/layout reads emit `oob_command:gap:<kind>` rows instead of
  guessed hierarchy.
- Current `oob_command` verification state: live-verified on the restarted
  vanilla process (pid 43412). A cold bootstrap outside the hot loop found 4897
  command roots and stored `oob_command_roots.json`; the 12-game-hour no-cold
  proof resolved 4704 still-live roots from cache in 0.109s, emitted raw
  `oob_command` deltas together with `ships`, advanced 12 game hours, verified
  replay output, and reported no raw gap domains. The same proof established
  that OOB discovery stays out of the worker loop (`hot_loop_heap_scan = false`)
  but also exposed the high raw-frame volume that needs coalescing before full
  game-length capture. The stricter raw military gate later resolved 7117
  command roots after an intentionally allowed 135.703s cold preflight, then
  captured 17705 raw `oob_command` deltas across 12 game hours with zero raw gap
  domains and verified replay output. The cold discovery timing is not
  acceptable for the normal path; it confirms cache/root refresh work is still
  required before full-game capture.
- Army leader-detail verification is stricter than generic OOB membership. The
  proof gate `--require-oob-command-leader-detail` requires exact army rows
  where leader id/status, skill, trait list, and exact trait status are
  co-present. The no-cold proof
  `replay/snapshots/_collector_session_oob_leader_gate/20260629T215306Z_pid43412`
  advanced 12 game hours in 54.110 wall seconds, verified replay output,
  emitted 19842 raw `oob_command` deltas, reported zero raw OOB gap domains,
  and captured 7485 exact army leader-detail rows with 15911 trait tokens. This
  proves raw army leader skill and assigned trait tokens only; save-shaped OOB
  nesting and wider commander detail remain follow-up segments.

### Air Activity

Source candidates:

- `memory_reader/naval_air/naval_air_reader.py`
  - `enumerate_air_wings()`
  - `enumerate_air_superiority()`
  - `wing_owner_tagid()`
  - `wing_mission_region()`
- `memory_reader/naval_air/air_combat_b1_findings.draft.json`

Cadence:

- The first DLL slice samples bridged air-wing bodies at a 250 ms floor even
  when the worker itself is running at 10 ms. This keeps speed-five recording
  bounded while still catching sub-hour wing mission/count transitions.
- Region aggregates can be derived from the wing stream by grouping
  `(mission_region, owner, mission_mask)` and summing `effective_planes`, then
  promoted into a separate region stream if the UI needs precomputed rows.

Events/deltas:

- Current raw domain: `air_wings`.
- Current durable key: `air_wing:<TAG>:<wing_id>:<generation>`. The generation
  is assigned by body continuity and prevents owner/wing-id reuse from
  overwriting an earlier logical wing without leaking the pointer.
- Current row fields: `air_wing_key`, `owner`, `wing_id`, `generation`,
  `count`, `mission_mask`, `mission_region`, and `effective_planes`.
- Current events: `air_wing_observed`, `air_wing_updated`, and
  `air_wing_lost`.
- Current gap key: `air_wings:gap:<kind>`.
- Later derived region events: `air_region_planes_delta`,
  `air_region_effective_planes_delta`, and `air_region_proxy_control_delta`.

Notes:

- The Rust collector accepts an `air_wing_body_bridge` control contract:
  `owner_tag_db_id:wing_id:body_ptr`. It rejects malformed, duplicate,
  unaligned, out-of-range, or oversized bridge rows before sampling.
- `live_capture.collector_air_wing_bridge` is cache-first and requires
  `--allow-air-wing-cold-scan` before running the expensive
  `NavalAirReader.enumerate_air_wings()` heap/vtable scan. Cache hits reread
  each cached body and require the wing vtable, type `69`, wing id, and owner
  chain to match before the DLL receives a bridge row.
- Each DLL sample revalidates the wing vtable, type, wing id, pool vtable,
  owner-chain country vtable, owner tag id, tag-table decode, optional mission
  region vtables, and sane count/effective-plane bounds before emitting. The
  replay file never receives `body_ptr` or `tag_db_id`.
- Current `air_wings` verification state on pid 43412: live-proven on the
  current game with the cache-backed no-cold run
  `replay/snapshots/_collector_session_air_wing_gate/20260629T205254Z_pid43412`.
  The run advanced 12 game hours in 55.875 wall seconds, verified replay output,
  hit `air_wing_body_cache` in 0.094s, supplied 585 bridge rows, emitted 11115
  raw `air_wings` deltas, observed 585 `air_wing_observed` and 10530
  `air_wing_updated` events, and reported zero raw gaps. Earlier zero-row
  fixtures remain useful negative evidence only for games without discoverable
  active wings.
- Do not model air as land-style `combat_started`/`combat_ended`. Current live
  evidence supports continuous per-region attrition/aggregation, not a discrete
  CCombatSide object with air-wing participants.
- Displayed air-superiority percent remains a proxy until the exact UI scalar or
  full scoring inputs are located. The emitted `effective_planes` value is the
  live active/effective plane count and a stable aggregation input, not the
  final UI air-superiority percentage.

### Orders And Fronts

Source candidates:

- `memory_reader/oob_command_live.py`
  - `enumerate_command_objs()`
  - `validate_command_objs()`
  - `read_command_tree_from_objs()`
- `memory_reader/construction_orders_live.py`
  - `enumerate_armies()`
  - `validate_armies()`
  - `read_orders_from_armies()`
- `memory_reader/construction_orders_offsets.draft.json`

Current DLL contract:

- The Rust collector accepts an `army_body_bridge` control contract:
  semicolon-separated `army_id:body_ptr` rows. The recorder producer
  `live_capture.collector_army_order_bridge` is cache-first and requires
  `--allow-army-cold-scan` before running the expensive army vtable sweep.
- The worker revalidates every supplied `CArmy` body by vtable, type, and id
  before reading `CArmy+0x98` active order pointers. It does not scan the heap in
  the worker loop.
- Durable raw frames use synthetic `army_order:<army_id>:<generation>` keys.
  Order pointers stay internal and are used only for per-session continuity.
- The first raw domain is `army_orders`: `army_id`, `generation`,
  `slot_index`, `order_type`, nullable `target_states`, `target_state_count`,
  and `target_status`. Type-5 target-state vectors up to the bounded inline cap
  are exact; larger valid vectors are `target_status:"overflow"` with only the
  count; non-type-5 geometry is `target_status:"unsupported"`.
- Current verification state: live-verified for raw order presence and
  add/remove lifecycle, still unverified for in-place order edits. The Rust
  tracker splits same-address different-army reuse into `army_order_removed` for
  the old `army_order:<army_id>:<generation>` and `army_order_added` for a fresh
  generation, with regression coverage and no pointer leakage; it also has
  regression coverage that same-address/same-army target-state mutation emits
  `army_order_updated` rather than add/remove. A 2026-06-29 cold bootstrap on
  pid 43412 found 456 army bodies outside the hot loop. The first no-cold run
  exposed an all-or-nothing stale-cache failure; the preflight now validates
  cached army bodies row-by-row and salvages still-live rows, mirroring the
  division bridge behavior. The stricter raw military proof resolved 636 army
  bodies after an intentionally allowed 27.516s cold preflight, advanced 12 game
  hours with `hourly_requirement.met = true` in 62.688 wall seconds, emitted 547
  raw `army_orders` deltas and 547 `army_order_added` events, verified the
  replay, and reported zero raw gap domains. A later cache-backed fixture run
  `replay/snapshots/_collector_session_raw_military_gate/20260629T200009Z_pid43412`
  advanced 36 game hours in 45.266 wall seconds, verified replay output,
  resolved 580 army bodies from cache in 0.078s, emitted 519 army-order events,
  observed one `army_order_removed`, and reported zero raw gap domains. No
  `army_order_updated` event appeared; raw-stream inspection showed the removal
  had no same-army/same-slot replacement nearby, so in-place order edits remain
  a live fixture gap rather than a hidden replacement case in that evidence. A
  later cache-backed speed-five run
  `replay/snapshots/_collector_session_raw_military_gate/20260629T201212Z_pid43412`
  advanced 48 game hours in 55.031 wall seconds, resolved 496 army bodies from
  cache in 0.078s, verified replay output, emitted 463 adds and 13 removals with
  zero raw gap domains, and still failed `--require-army-order-raw-updates`.
  Promotion requires a live fixture where an already-sampled type-5 offensive
  order is redrawn/resized during capture and the strict update gate passes. A
  second fixture window
  `replay/snapshots/_collector_session_raw_military_gate/20260629T201839Z_pid43412`
  advanced 49 game hours in 54.188 wall seconds, resolved 425 army bodies from
  cache in 0.078s, verified replay output, emitted 390 adds and one removal with
  zero raw gap domains, and also failed the update gate.

Cadence:

- Orders are event/delta data, not 10 ms hot-loop data by default.
- The current DLL stream samples at a 250 ms floor even when the worker cadence
  is 10 ms for combat/movement.

Events/deltas:

- `army_order_added`
- `army_order_removed`
- `army_order_updated`
- Later semantic splits: `army_order_target_changed`,
  `army_order_reordered`, `army_order_type_changed`.
- `army_membership_changed`
- `army_group_assignment_changed`
- `fleet_task_force_assignment_changed`

Notes:

- Type-5 offensive/front target state sets are the first supported order
  geometry.
- Non-type-5 province/front geometry is explicitly not fully validated. Emit the
  order type and `target_states: null` rather than fabricating geometry.
- `map.front_lines[]` remains derived from control adjacency. Exact player order
  overlays come from order-instance geometry, not from the derived front-line
  map field.
- Exact proof command needed after a current-process cache is available, or
  after explicitly allowing the slow cold bootstrap scans:

  ```powershell
  python -m live_capture.collector_session --pid <hoi4-pid> --dll replay_native\target\debug\collector.dll --out-root replay\snapshots\_collector_session_order_org_gate --game-id order_org_strength_live --duration-s 8 --sample-ms 10 --until-game-hours 1 --max-wall-s 60 --preflight-division-movement --preflight-army-orders
  python -m live_capture.collector_proof_gate <run-dir>\session_manifest.json --min-game-hours 1 --require-raw-domain divisions --require-raw-domain army_orders --require-raw-event-type division_org_strength_delta --require-raw-event-type army_order_updated
  ```

  If the cache is missing and a bootstrap run is intentionally allowed, add
  `--allow-division-cold-scan --allow-army-cold-scan` to the session command.
  The order event type should match the live action being tested:
  `army_order_updated` for target/type/reorder edits or `army_order_removed`
  for deletion. Treat `army_order_added` as first-observation scaffolding unless
  the run already had an earlier army-order sample before the order was created.
  For the broad raw-domain/org proof, the currently proven 12-hour gate is:

  ```powershell
  python -m live_capture.collector_proof_gate replay\snapshots\_collector_session_order_org_gate\20260629T134805Z_pid43412\session_manifest.json --min-game-hours 12 --require-raw-domain divisions --require-raw-domain army_orders --require-raw-event-type division_org_strength_delta
  ```

### Province And State Control

Source candidates:

- `live_capture/collector_frames.py`
  - `_fold_state_rows()`
  - `_fold_province_rows()`
- `replay/diff_engine.py`
  - `_diff_states()`
  - `_diff_provinces()`

Cadence:

- Full province control scans are too heavy for the 10 ms combat hot path.
- Keep hourly full map state.
- Emit targeted `province_controller_change` deltas when map bridge/state roots
  can cheaply reread changed values, or when a scheduled map sample detects a
  change.

Events/deltas:

- `state_controller_change`
- `province_controller_change`
- `state_owner_change`
- `control_gap`

Notes:

- The existing diff engine already has snapshot-pair events for control
  changes. The raw stream should sharpen timestamps without changing the event
  meaning.

## Replay Merge Behavior

Raw events must merge with hourly snapshot-derived events by timestamp and
identity:

1. Exact raw collector events win over interval snapshot diffs for the same
   event identity and interval.
2. Multiple raw deltas inside one game hour are ordered by `(hour_index,
   tick_ms, sequence)`.
3. Hourly snapshot diffs remain a fallback when no exact raw event covers the
   interval.
4. If a raw stream contains an explicit `gap`, the UI should show degraded
   confidence rather than silently relying on a later inferred diff.
5. Same-hour snapshot coalescing must not consume raw deltas. The delta/event
   stream is a sibling timeline input, not another snapshot list.

Merge entry point:

- Keep `build_timeline(snapshots)` unchanged for snapshot-only callers.
- Use the sibling helper `build_timeline_with_raw_events(snapshots,
  raw_events)`, which calls the existing snapshot diff path, normalizes raw
  frames into schema events or gap markers, then applies one merge/dedupe pass.
- Land-combat raw lifecycle endpoints are the first real collector-domain
  promotion in this path: started/ended frames become canonical combat events,
  update frames remain raw-only, and pointer-like diagnostics are rejected rather
  than written into the replay timeline.
- Raw events with `source="C"` and `confidence="exact"` supersede interval
  diffs only when their identity matches and their timestamp falls inside the
  diff interval.
- Raw sampled/proxy events do not supersede exact marker events.
- Gap frames never delete inferred events. They annotate the interval so the UI
  can display "inferred with known capture gap" instead of a clean exact event.

## Bounded First Implementation Slice

The first slice is implemented for the raw-frame contract and has now been
widened to one live combat domain:

1. Extend collector frame validation for typed `domain_delta`, `event`, and
   `gap` payloads.
2. Preserve raw non-snapshot frames in the session manifest with counts by type,
   domain, and gap reason.
3. Add a replay timeline helper that accepts `snapshots + raw_events` and merges
   raw exact collector events with snapshot diffs.
4. Add tests for:
   - sub-hour movement events surviving same-hour snapshot coalescing;
   - exact raw province-control event superseding an interval diff;
   - explicit combat gap surfacing in the merged timeline;
   - malformed delta/event frames failing closed.
5. Port live domains incrementally. Division movement proved the raw delta path;
   land combat now proves canonical lifecycle event promotion from a live DLL
   stream.

Suggested first patch set:

- `live_capture/collector_frames.py`: common envelope validation and payload
  validation for synthetic `event`, `domain_delta`, and `gap` frames.
- `live_capture/collector_session.py`: raw-frame summary counters and
  `.battleplan` raw-stream packaging; do not convert deltas into canonical
  snapshots.
- `replay/diff_engine.py`: add the sibling raw merge helper while leaving
  `build_timeline()` behavior intact.
- `live_capture/tests/test_collector_frames.py`: malformed/valid raw frame
  validation cases.
- `live_capture/tests/test_collector_session.py`: manifest raw-frame summary and
  replay-container raw-stream packaging cases.
- `replay/tests/test_diff_engine.py`: raw exact province event supersedes
  interval diff, raw movement survives same-hour coalescing, and combat gaps are
  retained.

Files currently outside the first slice:

- `replay_native/collector/src/lib.rs`: no live DLL payload emission until the
  Python-side contract and tests are green.
- `memory_reader/*`: no reader port until synthetic raw-frame storage and merge
  behavior are stable.
- `memory_reader/fuel_live.py`: unrelated scheduled fuel workstream.

## Blockers And Risks

- Hot-loop arena scans are not acceptable at speed five. Use cold discovery,
  pinned roots, validation, and bounded rereads.
- Existing warm-pin proofs often assume paused-frame stability. Speed-five
  running-game proofs must add race detection and explicit gaps.
- Combat and order identities can reuse ids. Use generation bookkeeping.
- Air battle objects should not be invented; air remains region/wing deltas.
- Full province-map scans cannot run at combat cadence.
- Save data must not fill runtime gaps. Unsupported live layouts should be
  omitted or badged.
