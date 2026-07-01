# land_combat_detail_events_live registration patch plan

Do not edit shared entrypoints until the orchestrator accepts this lane.

Required registration-only changes:

1. Add domain registry entry:
   - id `55`
   - name `land_combat_detail_events_live`
   - wire name `land_combat_detail_events_live`
   - status `Reserved` or `ValidatedSliceOpenGaps` until the shared speed-five
     proof passes.

2. Collector hook:
   - add `land_combat_detail_events_live_lane` as a path dependency of
     `extractor/native/collector`;
   - extend the existing domain-21 `LandCombatTracker` transition point to pass
     previous/current `LandCombatSnapshot` values into
     `land_combat_detail_events_live_lane::build_transition_capture_frame`;
   - append the returned compact record only from the one injected DLL
     coordinator.

3. Recorder hook:
   - add `land_combat_detail_events_live_lane` as a path dependency of
     `extractor/native/recorder_host`;
   - map domain id `55` to name `land_combat_detail_events_live`;
   - call the lane decoder/gate for data records and expose units:
     `land_combat_detail_phase_started_rows`,
     `land_combat_detail_phase_updated_rows`,
     `land_combat_detail_phase_ended_rows`,
     `land_combat_detail_participant_joined_rows`,
     `land_combat_detail_participant_left_unclassified_rows`.

4. Shared proof gate:
   - schedule `land_combats_live` and `land_combat_detail_events_live` together;
   - require hourly-or-better speed-five/current-equivalent frames, zero skipped
     hours, no source/partial/worker/bridge failures, Rust decode, and
     `.battleplan` verification.

This does not close casualty, equipment loss, retreat/overrun classification,
tactics, reserves, width, terrain, or modifiers. Those need additional exact
roots/functions or proof-backed rejections.
