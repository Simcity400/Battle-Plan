# Direct DLL/Rust Integration Patch Plan

Lane: `focus_research_decisions_depth_live`.

The shared integration slot for this lane uses production domain id `45` in the
collector and recorder entrypoints.

## Domain Allocation

- Production domain id: `45`.
- Domain id `44` is reserved by `supply_logistics_attrition_live`; this lane must
  not emit domain `44`.
- Other shared ids preserved by this integration include air combat losses `42`,
  market/trade `43`, projects/scientists/project progress `41`, weather `24`,
  and the existing base compact focus/tech/decision domains.
- Domain name: `focus_research_decisions_depth_live`.

## Compact Payload Schema

Use the lane crate schema from
`replay_native/lanes/focus_research_decisions_depth_live/src/lib.rs`.

Header, 48 bytes:

- `u64 elapsed_wall_ms`
- `u32 status_flags`
- `u32 focus_event_count`
- `u32 research_eta_count`
- `u32 decision_status_count`
- `u32 decision_event_count`
- `u32 observed_focus_rows`
- `u32 observed_research_slot_rows`
- `u32 observed_decision_rows`
- `u32 observed_timed_decision_rows`
- `u32 badge_count`

Known `status_flags` bits:

- bit 0: focus pause is unavailable and must be represented by a focus-pause
  badge row.
- bit 1: targeted/cooldown decision target fields are unavailable and must be
  represented by a decision-target badge row.
- bit 2: direct engine event hooks are unavailable; replay transitions are
  deterministic compact-frame derivations and must be represented by a direct
  event badge row.

For the current build, all three bits are required. The decoder must fail
closed if a payload silently omits any required fail-closed status bit.

Rows:

- Focus event row: fixed numeric prefix
  `tag_db_id, kind, focus_kind, flags, previous_completed_count,
  current_completed_count, previous_progress_raw, current_progress_raw,
  completed_delta`, then compact strings `from_token`, `to_token`,
  `completed_token`.
- Research ETA row: fixed 80 bytes:
  `tag_db_id, slot_index, flags, hour_delta, previous_tech_handle,
  current_tech_handle, previous_tech_def_index, current_tech_def_index,
  previous_live_progress_raw, current_live_progress_raw,
  delta_live_progress_raw, speed_raw_per_hour, base_required_points_raw,
  eta_hours`.
- Decision status row: fixed 28 bytes:
  `tag_db_id, active_timed_count, targeted_count, targeted_timed_count,
  decisions_taken_bucket_count, re_enable_count, timed_live_count`.
- Decision event row: fixed numeric prefix
  `tag_db_id, kind, flags, previous_days, current_days, previous_state,
  current_state`, then compact string `decision`.
- Badge row: fixed 20-byte prefix
  `kind, flags, source_domain_id, affected_count, evidence_code`, then compact
  string `detail`.

Required badge rows while current fields remain fail-closed:

- kind `1`, focus pause fail-closed:
  - flags include fail-closed and current-build-probed;
  - `source_domain_id=12`;
  - `evidence_code=1`;
  - detail token
    `focus_pause_flag_fail_closed_no_paused_true_ground_truth`.
- kind `2`, targeted/cooldown decision target fields fail-closed:
  - flags include fail-closed and current-build-probed;
  - `source_domain_id=16`;
  - `evidence_code=2`;
  - detail token
    `decision_target_fields_fail_closed_body_layout_unvalidated`.
- kind `3`, direct event hooks unavailable:
  - flags include fail-closed and deterministic-derived;
  - `source_domain_id=1000`;
  - `evidence_code=3`;
  - detail token
    `direct_event_hooks_unavailable_transitions_derived_from_compact_frames`.

## Collector Patch

Patch target: `replay_native/collector/src/lib.rs`, after an integration slot is
granted.

1. Add the final domain id/name to the compact domain registry and heartbeat
   descriptors.
2. Add `CompactFocusResearchDecisionsDepthRead`, row structs, and encoder using
   the lane crate schema above.
3. Add `CompactFrdDepthTracker` beside the compact country frame tracker. It
   stores:
   - focus previous state keyed by `tag_db_id`;
   - research previous state keyed by `(tag_db_id, slot_index)`;
   - decision previous state keyed by `tag_db_id` and active timed decision
     token.
4. Feed the tracker from already validated DLL roots, not from Python/saves:
   - focus rows from `CCountry+0x1370` and `CCountry+0x1380`;
   - for direct completed-focus closure, compact focus rows must include a
     bounded completed-token vector, or the depth tracker must receive an exact
     completed-token delta before the compact row discards it; do not rely only
     on `completed_count` plus previous-current inference;
   - research rows from `CCountry+0xF60`, research slots, live progress, and
     base required points;
   - decision rows from `CCountry+0xFA0` and active timed decisions.
5. Emit one depth domain data record per compact frame, before the frame-end
   record, with the same `game_hour_raw` as the frame.
6. Keep fail-closed behavior:
   - do not emit a focus pause boolean;
   - do not emit non-timed targeted/cooldown decision body token, target, or
     cooldown detail;
   - do not emit targeted-timed target detail until `record+0x20` or another
     target field is proven by a stronger invariant than tag-id coincidence;
   - if the capture is running with `compact_country_frame=true`, verify that
     exact completed-focus tokens are still present in the depth payload before
     claiming direct completed-focus closure.

## Rust Recorder Patch

Patch target: `replay_native/recorder_host/src/main.rs`, after the collector
slot is granted.

1. Add the final domain id to the compact decoder dispatch.
2. Port or share the lane crate's `decode_depth_payload` validation:
   - minimum 48-byte header;
   - row-count/cursor bounds;
   - fixed row length checks for research ETA and decision status;
   - bounded event kinds and flags;
   - compact string length/cursor validation;
   - known status flag mask only;
   - required fail-closed status bits present for the current schema;
   - badge rows required for every fail-closed status flag;
   - badge kind/source/evidence/detail validation;
   - exact badge detail token validation;
   - duplicate-key gates where applicable.
3. Add recorder counters for:
   - `focus_depth_event_rows`;
   - `research_eta_rows`;
   - `decision_status_rows`;
   - `decision_event_rows`;
   - depth status flags.
4. Add `.battleplan` manifest/index fields only after the decoder accepts the
   compact domain without parse errors.

## Tests

Keep lane-local tests as fixtures until the integration slot lands:

```powershell
cargo test --manifest-path replay_native\lanes\focus_research_decisions_depth_live\Cargo.toml
```

Integration tests to add with the shared patch:

- collector encoder unit test for the 48-byte header and each row kind;
- recorder decoder unit test using a lane fixture payload with all row kinds;
- fail-closed decoder tests for truncated rows, bad flags, bad compact strings,
  missing required status bits, wrong badge detail tokens, and row-count
  overflow;
- deterministic derivation test: two identical ordered base-frame sequences
  produce byte-identical depth payloads.

## Production Proof Plan

After the shared patch is granted and applied:

1. Build collector and recorder.
2. Run a compact live speed-five/current-equivalent proof with scheduled
   domains including `focus_live`, `tech_live`, `decisions_live`, and
   `focus_research_decisions_depth_live`.
3. Require:
   - DLL-only source for depth rows;
   - compact stream contains the final depth domain;
   - Rust recorder decodes the final depth domain with zero parse errors;
   - exact completed-focus closure rows are present when a focus completes, or
     the proof explicitly records the field as blocked;
   - `.battleplan` pack verifies;
   - hourly-or-lower cadence;
   - zero skipped hours;
   - no source, partial, worker, or bridge failures.

Until that proof exists, this lane remains `NOT COMPLETE`.
