# construction_buildings_state_damage_live lane-local integration plan

Status: lane-local extraction helpers, compact encoding, decoder/gate code, live probe, fixture
contract gate, production-shaped compact stream proof, lane-local `.battleplan` pack/verify
proof, exact shared registration patch plan, and host-shaped hourly schedule proof are ready. Do not
merge into shared collector or recorder entrypoints until the orchestrator grants an integration
slot and the active shared collector/recorder edits are settled.

Domain-48 mini-stack status: construction queue rows, target state, state/province building rows,
damaged-building health rows, and state resource rows can be emitted by the lane-local collector.
Construction target location is validated at `subobject+0x1D8 -> target_location +0x6C`; state
locations map directly, and province construction rows map exact province location through the
province-to-state topology. The compact source flag
`SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE = 0x10` remains the fail-closed fallback if any
construction row cannot resolve. The gate rejects silent target-state omission, target-state
emission while unavailable-badged, and partial target-state emission.

Registry alignment: `replay_native/domain_registry.json` assigns final domain id
`48 = construction_buildings_state_damage_live`. Current shared registry helper files expose the
domain-48 descriptor, but the shared collector and recorder entrypoints do not yet call the lane
module. The registry-aware gate `construction_buildings_state_damage_domain_allocation_gate`
therefore checks two different facts: id 48 is assigned to this lane, and shared implementation
hooks remain absent until an integration slot opens.

## Lane-local proof

Commands:

```powershell
cargo test --manifest-path replay_native\lanes\construction_buildings_state_damage_live\Cargo.toml
cargo run --manifest-path replay_native\lanes\construction_buildings_state_damage_live\Cargo.toml --bin construction_buildings_state_damage_lane_proof -- replay_native\runs\construction_buildings_state_damage_live_20260701_v1
python replay_native\lanes\construction_buildings_state_damage_live\probes\live_probe_870ebf.py --out replay_native\runs\construction_buildings_state_damage_live_20260701_v2\live_probe_870ebf.json
cargo run --manifest-path replay_native\lanes\construction_buildings_state_damage_live\Cargo.toml --bin construction_buildings_state_damage_fixture_gate -- replay_native\lanes\construction_buildings_state_damage_live\fixtures\fixture_contract.example.json replay_native\runs\construction_buildings_state_damage_live_20260701_v2
cargo run --manifest-path replay_native\lanes\construction_buildings_state_damage_live\Cargo.toml --bin construction_buildings_state_damage_lane_proof -- replay_native\runs\construction_buildings_state_damage_live_20260701_v5_slot_ready_badged
cargo run --manifest-path replay_native\lanes\construction_buildings_state_damage_live\Cargo.toml --bin construction_buildings_state_damage_lane_proof -- replay_native\runs\construction_buildings_state_damage_live_20260701_v6_domain46_slot_ready
cargo run --manifest-path replay_native\lanes\construction_buildings_state_damage_live\Cargo.toml --bin construction_buildings_state_damage_domain_allocation_gate -- replay_native\runs\construction_buildings_state_damage_live_20260701_v7_domain48_allocation
cargo run --manifest-path replay_native\lanes\construction_buildings_state_damage_live\Cargo.toml --bin construction_buildings_state_damage_lane_proof -- replay_native\runs\construction_buildings_state_damage_live_20260701_v7_domain48_slot_ready
cargo run --manifest-path replay_native\lanes\construction_buildings_state_damage_live\Cargo.toml --bin construction_buildings_state_damage_domain_allocation_gate -- replay_native\runs\construction_buildings_state_damage_live_20260701_v8_domain48_current_shared_alignment
cargo run --manifest-path replay_native\lanes\construction_buildings_state_damage_live\Cargo.toml --bin construction_buildings_state_damage_target_state_fixture_blocker -- replay_native\runs\construction_buildings_state_damage_live_20260701_v4\target_candidate_focus_probe.json replay_native\runs\construction_buildings_state_damage_live_20260701_v9_target_state_fixture_blocker
cargo run --manifest-path replay_native\lanes\construction_buildings_state_damage_live\Cargo.toml --bin construction_buildings_state_damage_mini_stack_proof -- replay_native\runs\construction_buildings_state_damage_live_20260701_v10_domain48_mini_stack
cargo run --manifest-path replay_native\lanes\construction_buildings_state_damage_live\Cargo.toml --bin construction_buildings_state_damage_replay_pack_proof -- replay_native\runs\construction_buildings_state_damage_live_20260701_v11_replay_pack_proof
cargo run --manifest-path replay_native\lanes\construction_buildings_state_damage_live\Cargo.toml --bin construction_buildings_state_damage_domain_allocation_gate -- replay_native\runs\construction_buildings_state_damage_live_20260701_v12_registry_gate
python replay_native\lanes\construction_buildings_state_damage_live\probes\target_candidate_focus_probe.py --out replay_native\runs\construction_buildings_state_damage_live_20260701_v13_target_focus_refresh\target_candidate_focus_probe.json
cargo run --manifest-path replay_native\lanes\construction_buildings_state_damage_live\Cargo.toml --bin construction_buildings_state_damage_target_state_oracle_contract -- replay_native\runs\construction_buildings_state_damage_live_20260701_v13_target_focus_refresh\target_candidate_focus_probe.json replay_native\runs\construction_buildings_state_damage_live_20260701_v14_target_state_oracle_contract
```

Proof artifact:

```text
replay_native/runs/construction_buildings_state_damage_live_20260701_v1/lane_proof.json
replay_native/runs/construction_buildings_state_damage_live_20260701_v1/construction_buildings_state_damage_live.compact.bin
replay_native/runs/construction_buildings_state_damage_live_20260701_v2/live_probe_870ebf.json
replay_native/runs/construction_buildings_state_damage_live_20260701_v2/fixture_contract_gate.json
replay_native/runs/construction_buildings_state_damage_live_20260701_v2/fixture_contract.compact.bin
replay_native/runs/construction_buildings_state_damage_live_20260701_v5_slot_ready_badged/lane_proof.json
replay_native/runs/construction_buildings_state_damage_live_20260701_v5_slot_ready_badged/construction_buildings_state_damage_live.compact.bin
replay_native/runs/construction_buildings_state_damage_live_20260701_v6_domain46_slot_ready/lane_proof.json
replay_native/runs/construction_buildings_state_damage_live_20260701_v6_domain46_slot_ready/construction_buildings_state_damage_live.compact.bin
replay_native/runs/construction_buildings_state_damage_live_20260701_v7_domain48_allocation/domain_allocation_gate.json
replay_native/runs/construction_buildings_state_damage_live_20260701_v7_domain48_slot_ready/lane_proof.json
replay_native/runs/construction_buildings_state_damage_live_20260701_v7_domain48_slot_ready/construction_buildings_state_damage_live.compact.bin
replay_native/runs/construction_buildings_state_damage_live_20260701_v8_domain48_current_shared_alignment/domain_allocation_gate.json
replay_native/runs/construction_buildings_state_damage_live_20260701_v9_target_state_fixture_blocker/target_state_fixture_blocker.json
replay_native/runs/construction_buildings_state_damage_live_20260701_v10_domain48_mini_stack/mini_stack_proof.json
replay_native/runs/construction_buildings_state_damage_live_20260701_v10_domain48_mini_stack/construction_buildings_state_damage_live.domain48.compact.bin
replay_native/runs/construction_buildings_state_damage_live_20260701_v11_replay_pack_proof/replay_pack_proof.json
replay_native/runs/construction_buildings_state_damage_live_20260701_v11_replay_pack_proof/construction_buildings_state_damage_live.domain48.collector_stream.bin
replay_native/runs/construction_buildings_state_damage_live_20260701_v11_replay_pack_proof/construction_buildings_state_damage_live.domain48.battleplan
replay_native/runs/construction_buildings_state_damage_live_20260701_v12_registry_gate/domain_allocation_gate.json
replay_native/runs/construction_buildings_state_damage_live_20260701_v13_target_focus_refresh/target_candidate_focus_probe.json
replay_native/runs/construction_buildings_state_damage_live_20260701_v14_target_state_oracle_contract/target_state_oracle_contract.json
replay_native/runs/construction_buildings_state_damage_live_20260701_v14_target_state_oracle_contract/target_state_oracle_contract_gate.json
replay_native/runs/construction_buildings_state_damage_live_20260701_v29_bounded_root_search_static_topology/target_state_bounded_root_search.json
replay_native/runs/construction_buildings_state_damage_live_20260701_v31_target_location_root_inspect_secondhop/target_location_root_inspect.json
replay_native/runs/construction_buildings_state_damage_live_20260701_v33_mini_stack_target_location_root/mini_stack_proof.json
replay_native/runs/construction_buildings_state_damage_live_20260701_v33_replay_pack_target_location_root/replay_pack_proof.json
replay_native/runs/construction_buildings_state_damage_live_20260701_v33_replay_pack_target_location_root/construction_buildings_state_damage_live.domain48.battleplan
replay_native/runs/construction_buildings_state_damage_live_20260701_v34_replay_pack_hourly_schedule_target_location_root/replay_pack_proof.json
replay_native/runs/construction_buildings_state_damage_live_20260701_v34_replay_pack_hourly_schedule_target_location_root/construction_buildings_state_damage_live.domain48.collector_stream.bin
replay_native/runs/construction_buildings_state_damage_live_20260701_v34_replay_pack_hourly_schedule_target_location_root/construction_buildings_state_damage_live.domain48.battleplan
```

Latest host-shaped schedule proof:

```powershell
cargo run --manifest-path replay_native\lanes\construction_buildings_state_damage_live\Cargo.toml --bin construction_buildings_state_damage_replay_pack_proof -- replay_native\runs\construction_buildings_state_damage_live_20260701_v34_replay_pack_hourly_schedule_target_location_root 12
```

Result: passed 12 consecutive scheduled game hours with one domain-48 perf record, one domain-48
data record, and one frame-end record per hour. The decoder saw 24 construction rows, 24 target
states, 36 building rows, 12 damaged-building rows, and 12 resource rows. The compact stream had
zero skipped hours, `.battleplan` verification had byte-identical stream bytes, and the lane-owned
collector path recorded no source-unavailable, partial-data, worker-panic, or bridge-failure
condition. This is not a DLL-only production proof because shared collector/recorder registration
is still intentionally unedited.

## Lane-owned module shape

- Collector module: `src/collector_module.rs`
  - entrypoint `collect_domain48_compact(mem, CollectorRoots)`;
  - inputs: `module_base`, `game_hour_raw`, shared country construction roots, shared state roots,
    and shared province roots;
  - output: domain-48 compact bytes plus gate/stats;
  - target-state behavior: reads exact construction location from
    `subobject+0x1D8 -> target_location +0x6C`; emits `target_state_id` on every construction row
    when state/province topology resolves all locations, otherwise clears all target states and sets
    `SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE = 0x10`.
- Recorder decoder/gate module: `src/decoder_gate.rs`
  - entrypoint `decode_domain48_compact(bytes)`;
  - validates domain id `48`, row lengths, duplicate guards, damage coherence, target-state badge
    rules, and summary counters.
- Mini-stack proof: `construction_buildings_state_damage_mini_stack_proof`
  - builds production-shaped memory for one country construction queue, one state building/resource
    root, and one province building root;
  - runs collector module -> compact encoder -> decoder/gate module;
  - proves construction/building/resource rows can pass with all-row target-state emission from the
    validated target-location root.

## Roots and offsets

- Construction queue root: `CCountry+0x0F68 -> production manager -> +0x70/+0x78/+0x7C`
  pointer vector, stride 8, type-59 construction lines.
- Construction line fields: type `+0x08`, line id `+0x0C`, active factories `+0x18`,
  cost raw `+0x28`, produced raw `+0x38`, building subobject `+0x70`, template id
  `subobject+0x08`.
- Construction target location root: `subobject+0x1D8 -> target_location +0x6C`.
  Bounded live/save proof v29 accepted 273 construction rows and matched exact save
  `building.location` on all rows. State-location rows use the location id as target state;
  province-location rows derive target state by joining the exact province id through
  province-to-state topology.
- Construction target-state pointer detail: `target_location+0x150` is a matching `CState` for
  state-location rows. For province-location rows, v31 scanned 0x300 bytes from `target_location`
  and `target_location+0x150` and rejected a direct target-state second-hop candidate set with 30
  province rows, 0 all-row candidates, 0 partial candidates, and 0 failures.
- State building root: state object `+0x158` vector of `CBuilding*`.
- Province building root: `CProvince+0x1C8` vector of `CBuilding*`.
- Current `870ebf` building identity: `CBuilding` vtable RVA `0x2973B88`, type id
  `+0x08`, level `+0x40`.
- Damage candidate: `healthy_level` at `CBuilding+0x42`. It is carried behind a
  presence flag and must remain fail-closed until a damaged-frame proof validates it on
  the current build.
- Construction target-state candidate: lane-local `probes/live_probe_870ebf.py` scans current
  construction line, subobject, and `subobject+0x10` template-def candidate ranges and joins any
  state/province-looking values through live `CProvince`/state roots. This is evidence inventory
  only until the fixture gate is run with an expected target state from a live/save-aligned
  construction fixture.
- Target-state candidate reduction on current `870ebf`: `target_candidate_analysis.py` rejected
  `subobject+0x60` as a per-line target-state root under `raw_state_id`, `raw_province_id`,
  `u64_low_state_id`, and `u64_low_province_id` interpretations. In the live proof it was
  invariant (`raw=357`, mapping to state `357` or province `357 -> state 253`) across 398
  construction lines spanning 84 owner tag ids and 18 template ids. Later v20/v23 proofs reject
  the surviving `subobject+0xC0` and `subobject+0xC8` candidates for state-building target rows;
  do not retry those offsets as the target-state or exact-location root.
- Historical target-state fixture blocker, tightened by `target_candidate_focus_probe.py`: on
  current `870ebf`, `subobject+0xC0` existed as a state id on 391/391 live construction lines
  but had only 11 unique state values and only 5 owner/controller matches to the line owner.
  `subobject+0xC8` existed as a state id on 391/391 lines but had only 3 unique state values and
  only 1 owner/controller match. This is strong suspicious evidence but not a conclusive rejection
  because foreign/subject/occupied construction cannot be ruled out from this lane alone. This
  blocker is superseded by the v23 stable-line rejection. The v4 focus proof emitted concrete rows:
  `replay_native/runs/construction_buildings_state_damage_live_20260701_v4/target_candidate_focus_probe.json`.
- Target-state unavailable badge: this is now a fail-closed fallback, not the expected accepted
  production state. Integrated packets should emit target state for every construction row from the
  validated target-location root. If shared state/province roots are incomplete and any row cannot
  resolve, the lane module clears all target states and sets
  `SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE = 0x10`.
- Historical exact target-state fixture blocker: the executable blocker proof
  `construction_buildings_state_damage_target_state_fixture_blocker` consumes the v4 focus proof,
  verifies domain id `48` from `replay_native/domain_registry.json`, inventories lane fixtures, and
  emits `target_state_fixture_gate.ok=false`. This remains useful as a fixture-shape contract, but
  the `+0xC0` / `+0xC8` root candidates themselves are now rejected by v23.
- Historical refreshed target-state fixture blocker: the v13 focus proof on the still-running current-build
  `870ebf` process observed 229 construction lines across 94 tag ids. `subobject+0xC0` was a valid
  state id for 229/229 rows but had only 9 unique states and 22 owner/controller matches.
  `subobject+0xC8` was a valid state id for 229/229 rows but had only 3 unique states and 16
  owner/controller matches. The later v23 stable-line proof supplies the oracle rejection that
  this owner/controller heuristic could not.
- Executable target-state oracle contract:
  `construction_buildings_state_damage_target_state_oracle_contract` consumes a focused live proof
  and writes `target_state_oracle_contract.json` plus
  `target_state_oracle_contract_gate.json`. The contract requires at least four filled
  `expected_target_state_id` rows from a same-frame save/UI oracle, matched by `tag_db_id`,
  `line_id`, `priority`, `template_id`, `active_factories`, `cost_raw`, and `produced_raw`. It
  terminally validates `+0xC0`, validates `+0xC8`, validates a province-id-to-state
  interpretation of either offset, rejects every interpretation, or reports
  `status = missing_oracle`. The v14 gate currently reports `missing_oracle` with 8 placeholder
  rows and keeps target-state omitted with source flag `0x10`.
- Refreshed target-state interpretation proof on 2026-07-01: the focused live proof now emits both
  direct raw-state candidates and raw-province-id mapped-state candidates for `subobject+0xC0` and
  `subobject+0xC8`. In the current 250-line proof, `+0xC0` was a direct state id on 250/250 rows
  with 6 unique direct states, and a valid province id on 76/250 rows mapping to 4 unique states.
  `+0xC8` was a direct state id on 250/250 rows with 2 unique direct states, and a valid province
  id on 250/250 rows mapping to states `213` or `400`. The later v23 proof checked both direct
  raw-state and raw-province-mapped-state interpretations for stable state-building rows and
  rejected them as target-state candidates.
- Target-state candidate rejection on 2026-07-01: stable-line proof
  `replay_native/runs/construction_buildings_state_damage_live_20260701_v23_stable_line_state_rejection/stable_line_rejection.json`
  accepted 8 type-59 construction rows from the v20 focused live proof and v21 autosave row oracle.
  It accepted only rows with matching `line_id`, `line_type = 59`, equal `cost_raw`, and numeric
  save `building.location`. It rejected `subobject+0xC0` and `subobject+0xC8` as target-state
  roots on 5 state-building rows with save state targets `{131, 558, 776, 886, 1014}` and rejected
  them as exact `building.location` roots on all 8 stable rows. Province-building rows remain
  separated as province-location evidence; they are not counted as mapped target-state proof.
- Runnable same-frame target-state observer: lane-local
  `probes/target_state_same_frame_observer.py` now captures the focused live proof from an explicit
  `hoi4.exe` PID, waits for a save whose mtime is after the live proof, parses
  `countries.*.production.general_lines.building.*.building.location`, and writes
  `filled_target_state_oracle.json` in the shape consumed by
  `construction_buildings_state_damage_target_state_oracle_contract`.
  Proof command on PID `45880`:
  `python replay_native\lanes\construction_buildings_state_damage_live\probes\target_state_same_frame_observer.py --pid 45880 --timeout-s 5 --poll-s 1 --stable-s 1 --out-dir replay_native\lanes\construction_buildings_state_damage_live\target\codex_domain48_same_frame_observer\pid45880_wait_short_v2`.
  Result: attached to requested PID `45880`, read `300` live construction lines, emitted `8`
  oracle rows, filled `0`, and marked each row `observer_failure =
  no_save_after_live_focus_proof` because no same-frame save was created after the live proof.
  Gate proof command:
  `cargo run --manifest-path replay_native\lanes\construction_buildings_state_damage_live\Cargo.toml --bin construction_buildings_state_damage_target_state_oracle_contract -- replay_native\lanes\construction_buildings_state_damage_live\target\codex_domain48_same_frame_observer\pid45880_wait_short_v2\target_candidate_focus_probe.json replay_native\lanes\construction_buildings_state_damage_live\target\codex_domain48_same_frame_observer\pid45880_wait_short_v2\target_state_oracle_contract replay_native\lanes\construction_buildings_state_damage_live\target\codex_domain48_same_frame_observer\pid45880_wait_short_v2\filled_target_state_oracle.json`.
  Gate result: `status = missing_oracle`, `blocker_confirmed = true`, `filled_rows = 0`,
  `placeholder_rows = 8`, `terminal_candidate_result = false`, source flag remains `0x10`.
  This path is superseded for production extraction by the v29/v31 target-location root proof, but
  remains useful if a future build invalidates `subobject+0x1D8`.
- Damaged-building contract: `construction_buildings_state_damage_fixture_gate` requires at least
  one expected damaged row where `level_at_0x40 > healthy_level_at_0x42`, validates undamaged
  controls, then encodes and decodes the compact payload. Use it with real live+save damaged-frame
  observations before promoting `+0x42` semantics.
- State resource root: `state+0x30 -> state_def`, then resources `+0xF8/+0x100/+0x104`,
  stride `0x10`.

## Registration-only patch plan

Detailed patch plan: `REGISTRATION_PATCH_PLAN.md`. The file now contains exact minimal hunks for:
registry descriptor status/schedulability, collector dependency/imports, the process-memory
adapter, the root adapter from shared country and topology roots, the domain perf/data append path,
recorder dependency, recorder decode helper, and domain-48 dispatch arm.

1. Use the registry assignment `48 = construction_buildings_state_damage_live`; do not perform
   lane-local id discovery.
2. Add the lane crate as a shared collector/recorder dependency, or move the lane module under the
   orchestrator-provided lane module interface.
3. Verify the registry helper descriptor `construction_buildings_state_damage_live -> 48`; current
   shared registry helper files already expose it, so the remaining work is collector/recorder
   implementation registration.
4. Collector registration hook only: pass existing shared country roots for construction and
   existing map topology roots into
   `construction_buildings_state_damage_live::collector_module::collect_domain48_compact`.
5. Recorder registration hook only: dispatch domain-48 compact payloads into
   `construction_buildings_state_damage_live::decoder_gate::decode_domain48_compact`.
6. Reuse existing shared country roots for construction and existing map topology roots for
   state/province building/resource rows. Do not duplicate owner, controller, or occupation
   semantics beyond state/province join keys.
7. Pass complete state roots and province-to-state province roots so the lane can emit target state
   for every construction row from `subobject+0x1D8 -> target_location +0x6C`. Do not infer target
   state from owner/controller/occupation. If any row cannot resolve, the lane deliberately clears
   all target states and sets `SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE`; the lane gate
   rejects partial target-state emission.
8. Keep `healthy_level` behind a row flag. If `healthy_level > level`, missing, or otherwise
   incoherent, omit it or set partial data rather than emitting guessed damage. If the live probe
   sees only `healthy_level == level`, treat that as undamaged-frame coherence, not damaged
   semantics proof.
9. Reject or skip construction rows with invalid active factories, negative progress, bad line
   type, or invalid template ids. This directly guards the invalid construction rows seen in
   the older combined manifest.
10. Add host summary units for construction rows, state building rows, province building rows,
   resource rows, and damaged building rows.
11. After registration, the shared scheduler should match the v34 host-shaped cadence: one
    domain-48 perf record, one domain-48 data record, and one frame-end record for each scheduled
    game hour, with no target-state unavailable badge on accepted captures.

## Required production proof after integration slot

Run a no-bridge compact proof with the current collector/recorder:

```powershell
cargo build -p collector -p recorder_host --release --manifest-path replay_native\Cargo.toml
replay_native\target\release\collector_record.exe `
  --pid <hoi4-pid> `
  --dll replay_native\target\release\collector.dll `
  --out-dir replay_native\runs\compact_construction_buildings_state_damage_<date> `
  --duration-s 30 `
  --sample-ms 0 `
  --min-game-hours 10 `
  --scheduled-domain construction_buildings_state_damage_live `
  --stream-format compact `
  --extraction-perf-gate
```

Acceptance:

- Compact stream decodes with no parse errors.
- `source_unavailable`, `partial_data`, `worker_panic`, and bridge-failure flags are all false.
- `.battleplan` is produced and verified.
- Speed-five/current-equivalent cadence is hourly-or-better with zero skipped hours.
- Rows include construction queues when present, state/province building rows, and state resource
  rows.
- Construction rows include target state for every row and do not set the unavailable badge.
- Damage rows use the validated `healthy_level` semantics and remain fail-closed on incoherent rows.
