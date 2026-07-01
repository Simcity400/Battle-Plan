# naval_combat_live registration-only hook plan

This plan is intentionally lane-local documentation. It does not edit the
shared collector or recorder entrypoints while another lane owns the integration
slot.

## Current Registry Evidence

- `replay_native/DOMAIN_REGISTRY.md` assigns:
  - `49 = naval_combat_lifecycle_live`
  - `50 = naval_combat_members_live`
- `replay_native/domain_registry.json` assigns those same IDs and names.
- `replay_native/collector/src/domain_registry.rs` already contains reserved
  descriptors for:
  - domain `49`, wire name `naval_combat_lifecycle_live`,
    `CompactDomainStatus::Reserved`, `schedulable=false`,
    `requires_data_record=true`
  - domain `50`, wire name `naval_combat_members_live`,
    `CompactDomainStatus::Reserved`, `schedulable=false`,
    `requires_data_record=true`
- `replay_native/recorder_host/src/compact_domain_registry.rs` already contains
  the matching reserved descriptors for domains `49` and `50`.

## Collector Registration Patch

When the orchestrator grants a collector slot, the registration-only patch is:

1. Add a collector dependency:
   `naval_combat_live_lane = { path = "../lanes/naval_combat_live" }`.
2. Activate only the two existing collector registry descriptors:
   - domain `49`: `Reserved/false/true` to `ActiveIntegration/true/true`
   - domain `50`: `Reserved/false/true` to `ActiveIntegration/true/true`
   Keep IDs, canonical names, wire names, aliases, and
   `requires_data_record=true` unchanged.
3. Add one callout in the existing DLL compact coordinator/scheduler for these
   scheduled names. The coordinator supplies already validated root/member
   bridge rows plus OOB-enriched current CShip rows to
   `naval_combat_live_lane::build_capture_frame`.
   Lifecycle bridge rows must include `location_id` from
   `body+0x28 -> CProvince+0xA4` and `region_id` from the active static
   `map/strategicregions/*.txt` province-to-region map. The lane gate rejects
   lifecycle rows without both fields.
4. Append the returned compact stream records only when `build_capture_frame`
   succeeds. The lane fails closed on empty or one-sided fixtures, unresolved
   non-convoy `CFEXMember -> CShip` joins, missing task-force/fleet links,
   missing CShip strength/org/XP/sunk-convoy telemetry, and status failure
   flags.
5. Do not schedule domains `49/50` in a live run with no active two-sided
   fixture. In that state the module remains ready, but live combat proof is
   externally blocked; emitting scheduled-but-empty production domains would
   create expected-domain coverage failures.

## Recorder Registration Patch

When the orchestrator grants a recorder slot, the registration-only patch is:

1. Add a recorder dependency:
   `naval_combat_live_lane = { path = "../lanes/naval_combat_live" }`.
2. Activate only the two existing recorder registry descriptors:
   - domain `49`: `Reserved/false/true` to `ActiveIntegration/true/true`
   - domain `50`: `Reserved/false/true` to `ActiveIntegration/true/true`
   Keep IDs, canonical names, wire names, aliases, and
   `requires_data_record=true` unchanged.
3. Add narrow constants for the two domains in `recorder_host/src/main.rs`.
4. Add two decode dispatch arms:
   - domain `49` calls
     `naval_combat_live_lane::decode_lifecycle_data_payload`
   - domain `50` calls
     `naval_combat_live_lane::decode_members_data_payload`
5. Copy the returned `PayloadDecodeSummary` into the recorder `CompactSummary`:
   - OR `status_flags_or`
   - add `row_count` to the domain row total
   - add `data_units_total` keys:
     `naval_combat_lifecycle_rows`, `naval_combat_member_rows`,
     `naval_combat_lifecycle_location_rows`,
     `naval_combat_lifecycle_region_rows`,
     `naval_combat_cship_validated_member_rows`,
     `naval_combat_convoy_member_rows`,
     `naval_combat_member_deleted_rows`,
     `naval_combat_task_force_linked_member_rows`,
     `naval_combat_fleet_linked_member_rows`,
     `naval_combat_hull_telemetry_member_rows`,
     `naval_combat_sunk_convoy_telemetry_member_rows`
   - promote any `parse_errors` to compact decoder errors.
6. Keep the full stream gate for proof runs:
   `naval_combat_live_lane::gate_stream` must pass for a domain `49/50`
   production fixture.

## Required Integration Proof

The integration commit may be accepted only after all of these pass:

```powershell
cargo test --manifest-path replay_native\lanes\naval_combat_live\Cargo.toml
cargo run --manifest-path replay_native\lanes\naval_combat_live\Cargo.toml --bin naval_combat_lane_proof -- --out-dir replay_native\runs\naval_combat_live_lane_stack_870ebf_20260701_v4
cargo run --manifest-path replay_native\snapshot\Cargo.toml --bin pack_replay -- verify replay_native\runs\naval_combat_live_lane_stack_870ebf_20260701_v4\native_session.battleplan
python memory_reader\combat\naval_combat_live_fixture_probe.py --pid <pid> --workers 4 --require-two-sided --require-cship-join --out-dir replay_native\runs\naval_combat_live_<fixture>\live_fixture_probe
python replay_native\lanes\naval_combat_live\tools\active_fixture_proof.py --pid <pid> --workers 4 --timeout-s 1800 --poll-s 5 --out-dir replay_native\runs\naval_combat_live_active_fixture_harness_<fixture>
python replay_native\lanes\naval_combat_live\tools\fixture_independent_roots_audit.py --pid <pid> --out-dir replay_native\runs\naval_combat_live_fixture_independent_roots_<build>_<date>
```

For production completion, the live proof must also show non-empty
`bridge.txt`, non-empty `member_bridge.jsonl`, compact DLL capture, Rust decode,
`.battleplan` verification, speed-five/current-equivalent hourly-or-lower
cadence, zero skipped hours, and no partial/source/worker/bridge failures.

## Current Fixture Boundary

The current live blocker is combat-only. The same 870ebf process validates the
non-combat CShip resolver route, but the active naval-combat body/member root
cannot be proven until an in-game two-sided naval battle creates `CFEXMember`
rows for both sides.

The lane-local v4 proof already exercises the post-registration payload shape:
1 lifecycle start, 1 update, 1 end, 3 location rows, 3 region rows, 6 member
rows, 2 deleted member rows, 3 CShip-validated members, 3 task-force/fleet
linked members, 3 hull telemetry members, and byte-matched `.battleplan`
verification. It is synthetic and does not replace the live speed-five fixture
proof.

The active fixture harness is lane-owned and registration-safe:
`tools/active_fixture_proof.py` polls the current PID, writes `bridge.txt` and
`member_bridge.jsonl` only after `status=validated_active_fixture`, derives
`region_id` from the installed game's `map/strategicregions`, collects the
fixture-independent `CShip -> CTaskForce/CFleet` index, writes
`active_fixture_input.json`, and invokes `naval_combat_lane_proof
--fixture-json` so the same Rust compact decoder and `.battleplan` verification
gate the detected fixture.

Current no-fixture harness proof:
`replay_native/runs/naval_combat_live_active_fixture_harness_870ebf_20260701_v4`
stopped at `timeout_no_validated_fixture` with five scanned polls, `sha256_ok=true`,
and every poll at `body_hit_count=0`, `bridge_row_count=0`, and
`member_bridge_row_count=0`.
Current fixture-json handoff proof:
`replay_native/runs/naval_combat_live_fixture_json_mode_870ebf_20260701_v1`
passed `naval_combat_lane_proof --fixture-json` with 1 lifecycle row, 2 member
rows, 1 CShip/task-force/fleet linked member, byte-matched compact stream, and
verified `.battleplan`.
Current fixture-independent OOB linkage proof:
`replay_native/runs/naval_combat_live_oob_task_fleet_870ebf_20260701_v1`
validated 139 fleets, 163 task forces, 1288 ships, `CTaskForce+0x338/+0x344`
ship membership, and 163/163 task forces with derived strategic regions.
Current fixture-independent roots audit:
`replay_native/runs/naval_combat_live_fixture_independent_roots_870ebf_20260701_v1`
validated 1303/1303 ships with task-force/fleet linkage and CShip strength,
organisation, experience, and sunk-convoy counters. It rejected direct
`CTheatre` pointers in fleet/task-force bodies, and narrowed task-force
mission/repair/retreat/order plus home-base/port semantics to controlled
before/after fixtures.
