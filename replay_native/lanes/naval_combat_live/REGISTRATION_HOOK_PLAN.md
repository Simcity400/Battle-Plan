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
   bridge rows plus current CShip rows to
   `naval_combat_live_lane::build_capture_frame`.
4. Append the returned compact stream records only when `build_capture_frame`
   succeeds. The lane fails closed on empty or one-sided fixtures, unresolved
   non-convoy `CFEXMember -> CShip` joins, and status failure flags.
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
     `naval_combat_cship_validated_member_rows`,
     `naval_combat_convoy_member_rows`
   - promote any `parse_errors` to compact decoder errors.
6. Keep the full stream gate for proof runs:
   `naval_combat_live_lane::gate_stream` must pass for a domain `49/50`
   production fixture.

## Required Integration Proof

The integration commit may be accepted only after all of these pass:

```powershell
cargo test --manifest-path replay_native\lanes\naval_combat_live\Cargo.toml
cargo run --manifest-path replay_native\lanes\naval_combat_live\Cargo.toml --bin naval_combat_lane_proof -- --out-dir replay_native\runs\naval_combat_live_lane_stack_870ebf_20260701_v1
cargo run --manifest-path replay_native\snapshot\Cargo.toml --bin pack_replay -- verify replay_native\runs\naval_combat_live_lane_stack_870ebf_20260701_v1\native_session.battleplan
python memory_reader\combat\naval_combat_live_fixture_probe.py --pid <pid> --workers 4 --require-two-sided --require-cship-join --out-dir replay_native\runs\naval_combat_live_<fixture>\live_fixture_probe
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
