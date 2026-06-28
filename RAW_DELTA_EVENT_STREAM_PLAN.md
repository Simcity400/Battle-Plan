# Raw Delta/Event Stream Plan

## Purpose

Hourly canonical snapshots are necessary for durable replay state, but they are
not sufficient for speed-five combat, movement, order, and control replay. The
collector already preserves raw frame observations separately from hourly
snapshot selection; the next layer should make that raw stream first-class for
bounded `domain_delta`, `event`, and `gap` frames.

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

Cadence:

- Start at 10 ms during speed-five proof runs.
- Emit deltas only on changed movement state, target, source, progress bucket,
  location, combat ref count, org, or strength.
- Keep full division snapshots hourly; use deltas for animation and exact order.

Events/deltas:

- `movement_started`
- `movement_progress`
- `movement_stopped`
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

Source candidates:

- `memory_reader/combat/combat_enum.py`
  - `CombatReader.enumerate_naval_combats()`
  - `CombatReader.enumerate_naval_ships()`
- `memory_reader/combat/naval_combat_stats.py`
  - `NavalCombatStats.read_combat_stats()`
- `memory_reader/oob_full_live.py`
  - `OobShipRegistry.build_index()`
  - `OobShipRegistry.read_ship()`

Cadence:

- Sample active naval combats at the high-frequency combat cadence.
- Sample global CShip stats opportunistically for ships participating in active
  combats; avoid whole-navy scans in the hot path unless a cached registry has
  been validated.

Events/deltas:

- `naval_combat_started`
- `naval_combat_participants_changed`
- `naval_ship_strength_delta`
- `naval_ship_sunk`
- `naval_combat_damage_delta`
- `naval_combat_ended`

Notes:

- Use the newer `memory_reader/combat` CNavalCombat path for active battles.
  The older `memory_reader/naval_air` naval-combat detector is useful as a
  negative/probe path, not the primary port target.
- Damage history and hit counters need unpaused mid-tick proof before they are
  treated as exact.

### Air Activity

Source candidates:

- `memory_reader/naval_air/naval_air_reader.py`
  - `enumerate_air_wings()`
  - `enumerate_air_superiority()`
  - `wing_owner_tagid()`
  - `wing_mission_region()`
- `memory_reader/naval_air/air_combat_b1_findings.draft.json`

Cadence:

- Region/wing deltas can be sampled below the combat hot path unless proof
  shows mission changes or air ticks are being missed.
- Start with raw frames at 100-250 ms for air-region deltas, plus hourly full
  wing snapshots.

Events/deltas:

- `air_wing_mission_changed`
- `air_region_planes_delta`
- `air_region_effective_planes_delta`
- `air_region_proxy_control_delta`
- `air_gap`

Notes:

- Do not model air as land-style `combat_started`/`combat_ended`. Current live
  evidence supports continuous per-region attrition/aggregation, not a discrete
  CCombatSide object with air-wing participants.
- Displayed air-superiority percent remains a proxy until the exact UI scalar or
  full scoring inputs are located.

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

Cadence:

- Orders are event/delta data, not 10 ms hot-loop data by default.
- Start at 250-1000 ms while the user is issuing orders or when army/order
  object vectors change; otherwise rely on hourly snapshots.

Events/deltas:

- `army_order_added`
- `army_order_removed`
- `army_order_target_changed`
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

- Keep `build_timeline(snapshots)` unchanged until the raw contract is proven.
- Add a sibling helper, for example `build_timeline_with_raw_events(snapshots,
  raw_events)`, that calls the existing snapshot diff path, normalizes raw
  frames into schema events or gap markers, then applies one merge/dedupe pass.
- Raw events with `source="C"` and `confidence="exact"` supersede interval
  diffs only when their identity matches and their timestamp falls inside the
  diff interval.
- Raw sampled/proxy events do not supersede exact marker events.
- Gap frames never delete inferred events. They annotate the interval so the UI
  can display "inferred with known capture gap" instead of a clean exact event.

## Bounded First Implementation Slice

The first slice should avoid live reader porting and prove the contract end to
end with synthetic frames:

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
5. Only after that, port one live domain: division movement deltas from pinned
   division bodies.

Suggested first patch set:

- `live_capture/collector_frames.py`: common envelope validation and payload
  validation for synthetic `event`, `domain_delta`, and `gap` frames.
- `live_capture/collector_session.py`: raw-frame summary counters only; do not
  convert deltas into canonical snapshots.
- `replay/diff_engine.py`: add the sibling raw merge helper while leaving
  `build_timeline()` behavior intact.
- `live_capture/tests/test_collector_frames.py`: malformed/valid raw frame
  validation cases.
- `live_capture/tests/test_collector_session.py`: manifest raw-frame summary
  cases.
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
