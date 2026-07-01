# construction_buildings_state_damage_live / domain 48 status - 2026-07-01

Completion status: `NOT COMPLETE`.

Backlog reduced: yes. This pass removed the damaged-building / repair-health
proof gap for the current `870ebf` build, fixed a lane-local construction queue
sanity cap that would have skipped observed valid rows, rejected
`subobject+0xC0` / `subobject+0xC8`, and then validated the actual construction
target-location root used to emit target state lane-locally. The latest pass
also removed the schedule-shape ambiguity by producing an exact shared
registration patch plan and a bounded 12-hour host-shaped compact stream /
`.battleplan` proof for target-state capture.

Exact backlog item removed or narrowed:

- Removed: `CBuilding+0x42` is no longer an undamaged-frame-only candidate. A
  current live frame contained `healthy_level_at_0x42 < level_at_0x40` for
  damaged buildings, with no `healthy > level` invalids in the scanned frame.
- Removed: state resource rows needed by the domain-48 compact fixture gate are
  live-readable from `CState+0x30 -> state_def +0xF8/+0x100/+0x104`, stride
  `0x10`, in the same live proof path.
- Removed: `subobject+0xC0` and `subobject+0xC8` are no longer viable
  target-state roots for state-building construction rows. Stable-line
  autosave proof `v23_stable_line_state_rejection` accepted 8 type-59
  construction rows, rejected the offsets as target-state interpretations on 5
  state-building rows across 5 distinct target states, and rejected both
  offsets as exact `building.location` roots on all 8 stable rows.
- Removed: the actual exact target-location root is validated as
  `subobject+0x1D8 -> target_location +0x6C`. Bounded live/save proof
  `v29_bounded_root_search_static_topology` accepted 273 construction rows
  (243 state-location rows, 30 province-location rows) and found this exact
  save `building.location` candidate on every accepted row.
- Narrowed: no separate all-row target-state scalar exists in the bounded set.
  For state-location rows, `target_location+0x150` points to the matching
  `CState`. For province-location rows, v31 second-hop inspection rejected the
  bounded 0x300-byte `target_location` / `target_location+0x150` candidate set
  with 30 province rows, 0 all-row candidates, 0 partial candidates, and 0
  failures. Production target state for province construction must derive from
  exact province location through the shared/static province-to-state topology.
- Narrowed: production integration remains shared-slot blocked. Registry id 48
  is assigned and helper descriptors exist, but shared collector/recorder
  implementation hooks are absent by design.
- Narrowed: the exact minimal shared registration patch plan is now written in
  `REGISTRATION_PATCH_PLAN.md`. It specifies the registry descriptor flip,
  collector dependency/imports, process-memory adapter, root adapter from
  existing country/topology roots, domain perf/data append path, recorder
  dependency, domain-48 decode helper, and dispatch arm. No target-state root
  discovery or major lane implementation remains for the orchestrator.
- Narrowed: a host-shaped hourly schedule proof shows the lane-owned collector
  path can emit one domain-48 perf record, one domain-48 data record, and one
  frame-end record for every scheduled hour after registration. Proof v34 ran
  12 consecutive game hours, emitted target state on 24/24 construction rows
  across state-location and province-location fixtures, verified the compact
  stream inside `.battleplan`, and recorded zero skipped hours with no
  source/partial/worker/bridge failures.

Files changed:

- `replay_native/lanes/construction_buildings_state_damage_live/src/lib.rs`
- `replay_native/lanes/construction_buildings_state_damage_live/src/collector_module.rs`
- `replay_native/lanes/construction_buildings_state_damage_live/src/bin/domain_allocation_gate.rs`
- `replay_native/lanes/construction_buildings_state_damage_live/src/bin/mini_stack_proof.rs`
- `replay_native/lanes/construction_buildings_state_damage_live/src/bin/replay_pack_proof.rs`
- `replay_native/lanes/construction_buildings_state_damage_live/probes/live_probe_870ebf.py`
- `replay_native/lanes/construction_buildings_state_damage_live/probes/build_damage_fixture_from_live_probe.py`
- `replay_native/lanes/construction_buildings_state_damage_live/probes/target_state_stable_line_rejection.py`
- `replay_native/lanes/construction_buildings_state_damage_live/probes/target_state_bounded_root_search.py`
- `replay_native/lanes/construction_buildings_state_damage_live/probes/target_location_root_inspect.py`
- `replay_native/lanes/construction_buildings_state_damage_live/REGISTRATION_PATCH_PLAN.md`
- `replay_native/lanes/construction_buildings_state_damage_live/INTEGRATION_PLAN.md`
- `replay_native/lanes/construction_buildings_state_damage_live/LANE_STATUS_20260701.md`

Roots, offsets, and functions validated or rejected:

- Validated construction queue root remains
  `CCountry+0x0F68 -> production manager -> +0x70/+0x78/+0x7C`, with type-59
  construction lines and subobject template id at `subobject+0x08`.
- Fixed lane-local `MAX_PRODUCTION_LINE_FIXED_RAW` from `1_000_000_000` to
  `2_000_000_000`; live focused proof observed valid rows at `cost_raw =
  1_450_000_000`.
- Validated current-frame building damage / repair-health signal:
  `CBuilding+0x40` level and `CBuilding+0x42` healthy level. Live proof
  `v16_live_probe_resources` scanned 8,317 building entries, read healthy level
  for all 8,317, found 97 rows with `healthy < level`, 8,220 rows with
  `healthy == level`, 0 rows with `healthy > level`, and 0 missing healthy
  values.
- Validated state resources for the lane gate: live proof
  `v16_live_probe_resources` scanned 1,081 states, observed 467 states with
  resources, emitted 718 resource rows, and saw 0 invalid resource vectors or
  invalid rows.
- Rejected: `subobject+0xC0` and `subobject+0xC8` target-state candidates for
  state-building construction rows. The v23 stable-line proof used the v20
  focused live rows plus v21 autosave row identity. It accepted only rows where
  `line_id` matched, save `line_type == 59`, `cost_raw` matched, and save
  `building.location` was numeric. On state-building save templates
  `industrial_complex` and `infrastructure`, save locations
  `{131, 558, 776, 886, 1014}` differed from every emitted direct raw-state and
  raw-province-mapped-state interpretation of `+0xC0` / `+0xC8`.
- Rejected: `subobject+0xC0` and `subobject+0xC8` as exact
  `building.location` roots for all 8 stable rows. Save locations
  `{131, 558, 776, 886, 1014, 6039, 12003, 12975}` differed from the raw
  candidate values on the accepted rows.
- Validated: `subobject+0x1D8` points to a target-location object and
  `target_location+0x6C` stores save `building.location`. In v29 this matched
  273/273 accepted live/save construction rows across state and province
  construction.
- Validated: for 243 state-location rows, `target_location+0x150` points to the
  matching `CState`.
- Rejected: for 30 province-location rows, bounded second-hop inspection found
  no direct target-state candidate in the 0x300-byte scan of `target_location`
  and `target_location+0x150`. Province construction target state is derived by
  joining exact province `building.location` through province-to-state topology.

Compact schema/record changes:

- No wire-schema layout change.
- Domain id remains `48`.
- Existing construction rows still require either all `target_state_id` values
  or source flag `SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE = 0x10`.
  The lane-local collector now fills target state from the validated
  target-location root and clears the unavailable flag only when every
  construction row resolves.
- Existing building rows carry `healthy_level` behind the row presence flag; a
  row is damaged when `healthy_level < level`.
- Existing state resource rows remain `{state_id, resource_id, amount_raw}`.

Rust decoder/gate changes:

- The domain allocation gate now accepts the current registry status strings:
  `lane_ready_shared_adapter_registration_blocked_remaining_matrix` for domain
  47 and
  `target_state_same_frame_observer_ready_external_save_ui_action_blocked` for
  domain 48.
- Added lane-local target-state extraction from `subobject+0x1D8 ->
  target_location +0x6C`, with province template ids mapped through
  `ProvinceRoot`. Decoder/gate all-or-none semantics are unchanged:
  target-state omission is rejected unless the unavailable source flag is
  present, partial target-state emission is rejected, and target-state emission
  while unavailable-badged is rejected.

Proof commands and results:

```powershell
cargo test --manifest-path replay_native\lanes\construction_buildings_state_damage_live\Cargo.toml
```

Result: 13 library tests passed and 4 target-state oracle-contract tests passed.

```powershell
python replay_native\lanes\construction_buildings_state_damage_live\probes\live_probe_870ebf.py --out replay_native\runs\construction_buildings_state_damage_live_20260701_v16_live_probe_resources\live_probe_870ebf.json
```

Result: 293 construction lines observed; building damage proof passed with 97
damaged rows; state resources proof passed with 718 rows.

```powershell
python replay_native\lanes\construction_buildings_state_damage_live\probes\build_damage_fixture_from_live_probe.py --live-probe replay_native\runs\construction_buildings_state_damage_live_20260701_v16_live_probe_resources\live_probe_870ebf.json --out replay_native\runs\construction_buildings_state_damage_live_20260701_v16_damage_fixture\damage_fixture_from_live_probe.json
cargo run --manifest-path replay_native\lanes\construction_buildings_state_damage_live\Cargo.toml --bin construction_buildings_state_damage_fixture_gate -- replay_native\runs\construction_buildings_state_damage_live_20260701_v16_damage_fixture\damage_fixture_from_live_probe.json replay_native\runs\construction_buildings_state_damage_live_20260701_v16_damage_fixture_gate
```

Result: `gate_ok = true`, 9 building rows, 8 damaged rows, 1 resource row,
0 construction target-state rows, source flag `0x10` set, status flags `0`.

```powershell
cargo run --manifest-path replay_native\lanes\construction_buildings_state_damage_live\Cargo.toml --bin construction_buildings_state_damage_mini_stack_proof -- replay_native\runs\construction_buildings_state_damage_live_20260701_v17_mini_stack_after_cap_fix
cargo run --manifest-path replay_native\lanes\construction_buildings_state_damage_live\Cargo.toml --bin construction_buildings_state_damage_replay_pack_proof -- replay_native\runs\construction_buildings_state_damage_live_20260701_v17_replay_pack_after_cap_fix
```

Result: lane collector -> compact -> Rust decode gate passed; replay-pack proof
produced and verified
`construction_buildings_state_damage_live.domain48.battleplan`; compact stream
had 1 domain-data record, 1 frame-end record, zero skipped hours in the fixture
stream, byte-identical replay stream, and `container_verify_all = true`.

```powershell
python replay_native\lanes\construction_buildings_state_damage_live\probes\target_state_same_frame_observer.py --pid 45880 --timeout-s 5 --poll-s 1 --stable-s 1 --out-dir replay_native\runs\construction_buildings_state_damage_live_20260701_v20_same_frame_observer_short
cargo run --manifest-path replay_native\lanes\construction_buildings_state_damage_live\Cargo.toml --bin construction_buildings_state_damage_target_state_oracle_contract -- replay_native\runs\construction_buildings_state_damage_live_20260701_v20_same_frame_observer_short\target_candidate_focus_probe.json replay_native\runs\construction_buildings_state_damage_live_20260701_v20_same_frame_observer_short\target_state_oracle_contract replay_native\runs\construction_buildings_state_damage_live_20260701_v20_same_frame_observer_short\filled_target_state_oracle.json
```

Result: observer attached to PID `45880`, captured 337 construction rows, emitted
8 required oracle rows, filled 0 rows because no same-frame save appeared after
the live proof. Rust oracle gate status is `missing_oracle` with
`blocker_confirmed = true`.

```powershell
python replay_native\lanes\construction_buildings_state_damage_live\probes\target_state_same_frame_observer.py --focus-proof replay_native\runs\construction_buildings_state_damage_live_20260701_v20_same_frame_observer_short\target_candidate_focus_probe.json --save "C:\Users\Example\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV\save games\autosave.hoi4" --out-dir replay_native\runs\construction_buildings_state_damage_live_20260701_v21_autosave_row_oracle
python replay_native\lanes\construction_buildings_state_damage_live\probes\target_state_stable_line_rejection.py --focus-proof replay_native\runs\construction_buildings_state_damage_live_20260701_v20_same_frame_observer_short\target_candidate_focus_probe.json --oracle replay_native\runs\construction_buildings_state_damage_live_20260701_v21_autosave_row_oracle\filled_target_state_oracle.json --out replay_native\runs\construction_buildings_state_damage_live_20260701_v23_stable_line_state_rejection\stable_line_rejection.json
```

Result: stable-line rejection status
`rejected_state_building_target_candidates_and_exact_location_candidates`;
accepted 8 rows, rejected target-state interpretations on 5 state-building rows
with unique save state targets `[131, 558, 776, 886, 1014]`, and rejected exact
location roots on all 8 accepted rows.

```powershell
python replay_native\lanes\construction_buildings_state_damage_live\probes\target_state_bounded_root_search.py --pid 45880 --save "C:\Users\Example\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV\save games\autosave.hoi4" --out replay_native\runs\construction_buildings_state_damage_live_20260701_v29_bounded_root_search_static_topology\target_state_bounded_root_search.json
python replay_native\lanes\construction_buildings_state_damage_live\probes\target_location_root_inspect.py --pid 45880 --bounded-proof replay_native\runs\construction_buildings_state_damage_live_20260701_v29_bounded_root_search_static_topology\target_state_bounded_root_search.json --out replay_native\runs\construction_buildings_state_damage_live_20260701_v31_target_location_root_inspect_secondhop\target_location_root_inspect.json
```

Result: v29 accepted 273 live/save construction rows and found
`subobject+0x1D8->deref+0x6C` as an all-row exact `building.location` root.
There were 0 all-row target-state scalar candidates across the same row set.
v31 validated the root, showed `target_location+0x150` as `CState` for the 243
state-location rows, and rejected the bounded province second-hop candidate set
for 30 province-location rows.

```powershell
cargo run --manifest-path replay_native\lanes\construction_buildings_state_damage_live\Cargo.toml --bin construction_buildings_state_damage_mini_stack_proof -- replay_native\runs\construction_buildings_state_damage_live_20260701_v33_mini_stack_target_location_root
cargo run --manifest-path replay_native\lanes\construction_buildings_state_damage_live\Cargo.toml --bin construction_buildings_state_damage_mini_stack_proof -- replay_native\runs\construction_buildings_state_damage_live_20260701_v36_mini_stack_after_registration_plan
cargo run --manifest-path replay_native\lanes\construction_buildings_state_damage_live\Cargo.toml --bin construction_buildings_state_damage_replay_pack_proof -- replay_native\runs\construction_buildings_state_damage_live_20260701_v33_replay_pack_target_location_root
```

Result: lane-local collector -> compact -> Rust decode gate now emits target
state on all construction rows in the proof fixture. The replay-pack proof
produced and verified
`construction_buildings_state_damage_live.domain48.battleplan`;
`construction_rows_with_target_state = construction_rows = 1`,
`target_state_unavailable_badged = false`, compact stream had zero skipped
hours, byte-identical replay stream, and `container_verify_all = true`.

```powershell
cargo run --manifest-path replay_native\lanes\construction_buildings_state_damage_live\Cargo.toml --bin construction_buildings_state_damage_replay_pack_proof -- replay_native\runs\construction_buildings_state_damage_live_20260701_v34_replay_pack_hourly_schedule_target_location_root 12
```

Result: host-shaped hourly schedule proof passed for 12 consecutive game hours
`123..134`. The stream contains 36 records: 12 domain-48 perf records, 12
domain-48 data records, and 12 frame-end records. The decoder accepted all 12
frames with `construction_rows = 24`,
`construction_rows_with_target_state = 24`, `building_rows = 36`,
`damaged_building_rows = 12`, `resource_rows = 12`, and
`target_state_unavailable_badged_any = false`. The compact stream reports
`zero_skipped_hours = true`; `.battleplan` verification reports
`stream_bytes_equal = true` and `container_verify_all = true`. No
source-unavailable, partial-data, worker-panic, or bridge-failure condition was
present in the proof. Collection timing for the lane-owned path was
`min/avg/max/p95 = 40/50/145/145 us`.

```powershell
cargo run --manifest-path replay_native\lanes\construction_buildings_state_damage_live\Cargo.toml --bin construction_buildings_state_damage_domain_allocation_gate -- replay_native\runs\construction_buildings_state_damage_live_20260701_v19_registry_gate_current_status
cargo run --manifest-path replay_native\lanes\construction_buildings_state_damage_live\Cargo.toml --bin construction_buildings_state_damage_domain_allocation_gate -- replay_native\runs\construction_buildings_state_damage_live_20260701_v35_registry_gate_after_registration_plan
```

Result: `gate_ok = true`; registry id 48 is assigned to this lane, collector and
recorder registry helpers have descriptors, and shared collector/recorder
implementation hooks are absent.

Residual coverage matrix:

- Construction queue: lane-local collector reads country production manager
  queue, line id, priority/order, template id, factories, cost/progress, and
  target-location-derived target state. State-location and province-location
  construction rows are both covered by v34.
- Building levels: state and province `CBuilding` vectors emit levels from
  `CBuilding+0x40`.
- State/province damage and repair-health: `healthy_level` from
  `CBuilding+0x42` is emitted behind a row flag and interpreted as damaged when
  `healthy_level < level`; v16 live proof observed damaged rows and zero
  `healthy > level` invalids.
- State resources and map-visible static state production context:
  state-definition resources from `CState+0x30 -> state_def +0xF8/+0x100/+0x104`
  are covered by v16/v34 lane gates.
- Remaining residual: no unblocked lane-local construction/building/damage
  extractor residual is known after v34. The remaining gap is shared production
  registration and a real DLL live run through the shared recorder schedule.

Acceptance checklist:

- DLL-only: no, blocked on shared collector integration slot.
- Compact stream: yes, lane-local compact stream and replay-pack proof with
  target-state emission.
- Rust decoded: yes, lane-local decoder/gate.
- `.battleplan` verified: yes, lane-local replay-pack proof.
- Speed-five/current-equivalent hourly-or-lower: yes for the bounded host-shaped
  lane proof after registration shape; no for shared DLL production because the
  shared collector/recorder hooks are still absent.
- Zero skipped hours: yes for the lane-local host-shaped compact stream; not
  proven for production live DLL schedule.
- Any partial/source/worker/bridge failures: no in lane-local gates and no in
  the v34 host-shaped proof; production live DLL path is not schedulable yet.

Final blockers:

1. Shared integration slot: collector and recorder entrypoints do not yet hook
   domain 48. This lane must not edit those shared entrypoints without an
   orchestrator-granted slot.
2. Production DLL proof: the target-state root, state/province topology adapter,
   compact schema, decoder gate, and hourly host-shaped schedule are ready
   lane-locally. The final production proof still requires the orchestrator to
   apply the registration patch, run the single shared DLL collector schedule,
   and verify domain 48 through the shared recorder.
