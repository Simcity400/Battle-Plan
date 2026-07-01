# construction_buildings_state_damage_live lane-local integration plan

Status: lane-local extraction helpers, compact encoding, decoder/gate code, live probe, fixture
contract gate, production-shaped compact stream proof, and lane-local `.battleplan` pack/verify
proof are ready. Do not merge into shared collector or recorder entrypoints until the orchestrator
grants an integration slot and the active shared collector/recorder edits are settled.

Domain-48 mini-stack status: construction queue rows, state/province building rows,
damaged-building health rows, and state resource rows can be emitted now with construction target
state explicitly badged unavailable. The compact source flag for that badge is
`SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE = 0x10`. The gate rejects any production packet
that silently omits `target_state_id` from non-empty construction rows without this flag, and also
rejects packets that set the unavailable flag while emitting target states.

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
```

## Lane-owned module shape

- Collector module: `src/collector_module.rs`
  - entrypoint `collect_domain48_compact(mem, CollectorRoots)`;
  - inputs: `module_base`, `game_hour_raw`, shared country construction roots, shared state roots,
    and shared province roots;
  - output: domain-48 compact bytes plus gate/stats;
  - target-state behavior: every construction row has `target_state_id = None` and the frame sets
    `SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE = 0x10`.
- Recorder decoder/gate module: `src/decoder_gate.rs`
  - entrypoint `decode_domain48_compact(bytes)`;
  - validates domain id `48`, row lengths, duplicate guards, damage coherence, target-state badge
    rules, and summary counters.
- Mini-stack proof: `construction_buildings_state_damage_mini_stack_proof`
  - builds production-shaped memory for one country construction queue, one state building/resource
    root, and one province building root;
  - runs collector module -> compact encoder -> decoder/gate module;
  - proves construction/building/resource rows can pass without target-state semantics as long as
    the unavailable badge is present.

## Roots and offsets

- Construction queue root: `CCountry+0x0F68 -> production manager -> +0x70/+0x78/+0x7C`
  pointer vector, stride 8, type-59 construction lines.
- Construction line fields: type `+0x08`, line id `+0x0C`, active factories `+0x18`,
  cost raw `+0x28`, produced raw `+0x38`, building subobject `+0x70`, template id
  `subobject+0x08`.
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
  construction lines spanning 84 owner tag ids and 18 template ids. Remaining candidate offsets
  are `subobject+0xC0` and `subobject+0xC8` (raw/u64-low state interpretations only); these need
  a live/save-aligned fixture with expected target states before they can be promoted or rejected.
- Remaining target-state fixture blocker, tightened by `target_candidate_focus_probe.py`: on
  current `870ebf`, `subobject+0xC0` existed as a state id on 391/391 live construction lines
  but had only 11 unique state values and only 5 owner/controller matches to the line owner.
  `subobject+0xC8` existed as a state id on 391/391 lines but had only 3 unique state values and
  only 1 owner/controller match. This is strong suspicious evidence but not a conclusive rejection
  because foreign/subject/occupied construction cannot be ruled out from this lane alone. The
  v4 focus proof emits concrete rows that must be filled from a time-aligned save/UI oracle:
  `replay_native/runs/construction_buildings_state_damage_live_20260701_v4/target_candidate_focus_probe.json`.
- Target-state unavailable badge: until the v4 fixture request is settled, integrated packets must
  leave construction `target_state_id` absent and set
  `SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE = 0x10`. This keeps the field fail-closed and
  prevents replay consumers from interpreting a missing target as a decoder bug or a valid null.
- Exact target-state fixture blocker: the executable blocker proof
  `construction_buildings_state_damage_target_state_fixture_blocker` consumes the v4 focus proof,
  verifies domain id `48` from `replay_native/domain_registry.json`, inventories lane fixtures, and
  emits `target_state_fixture_gate.ok=false` until a time-aligned save/UI oracle fills numeric
  `expected_target_state_id` rows for `subobject+0xC0` or `subobject+0xC8`.
- Refreshed target-state fixture blocker: the v13 focus proof on the still-running current-build
  `870ebf` process observed 229 construction lines across 94 tag ids. `subobject+0xC0` was a valid
  state id for 229/229 rows but had only 9 unique states and 22 owner/controller matches.
  `subobject+0xC8` was a valid state id for 229/229 rows but had only 3 unique states and 16
  owner/controller matches. Both remain suspicious but not conclusively rejected because
  owner/controller mismatch is not an oracle for foreign/subject/occupied construction targets.
- Executable target-state oracle contract:
  `construction_buildings_state_damage_target_state_oracle_contract` consumes a focused live proof
  and writes `target_state_oracle_contract.json` plus
  `target_state_oracle_contract_gate.json`. The contract requires at least four filled
  `expected_target_state_id` rows from a same-frame save/UI oracle, matched by `tag_db_id`,
  `line_id`, `priority`, `template_id`, `active_factories`, `cost_raw`, and `produced_raw`. It
  terminally validates `+0xC0`, validates `+0xC8`, rejects both, or reports
  `status = missing_oracle`. The v14 gate currently reports `missing_oracle` with 8 placeholder
  rows and keeps target-state omitted with source flag `0x10`.
- Damaged-building contract: `construction_buildings_state_damage_fixture_gate` requires at least
  one expected damaged row where `level_at_0x40 > healthy_level_at_0x42`, validates undamaged
  controls, then encodes and decodes the compact payload. Use it with real live+save damaged-frame
  observations before promoting `+0x42` semantics.
- State resource root: `state+0x30 -> state_def`, then resources `+0xF8/+0x100/+0x104`,
  stride `0x10`.

## Registration-only patch plan

Detailed patch plan: `REGISTRATION_PATCH_PLAN.md`.

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
7. Keep construction target-state absent and set
   `SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE` until a live/save-aligned fixture validates
   the exact root. Do not infer it from template id or province/state ownership. When a root is
   validated, clear the unavailable flag and emit `target_state_id` for every construction row; the
   lane gate rejects partial target-state emission.
8. Keep `healthy_level` behind a row flag. If `healthy_level > level`, missing, or otherwise
   incoherent, omit it or set partial data rather than emitting guessed damage. If the live probe
   sees only `healthy_level == level`, treat that as undamaged-frame coherence, not damaged
   semantics proof.
9. Reject or skip construction rows with invalid active factories, negative progress, bad line
   type, or invalid template ids. This directly guards the invalid construction rows seen in
   the older combined manifest.
10. Add host summary units for construction rows, state building rows, province building rows,
   resource rows, and damaged building rows.

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
- Damage rows are accepted only after a damaged-frame proof validates `healthy_level` semantics.
