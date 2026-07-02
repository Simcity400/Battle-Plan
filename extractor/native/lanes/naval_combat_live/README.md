# naval_combat_live lane-local production stack
> Active lane note: this README is the only active lane-local documentation. Historical plans, proof reports, blocker notes, and registration packets for this lane are archived under ../../../documents/archive/native_lanes/naval_combat_live/. Current extraction completion status lives in ../../../documents/MASTER_EXTRACTION_CONTRACT.md.


This lane owns compact domains assigned by `extractor/documents/DOMAIN_REGISTRY.md`:

- `49 = naval_combat_lifecycle_live`
- `50 = naval_combat_members_live`

The module is deliberately lane-local while another lane owns the shared
collector/recorder integration slot.

## Files

- `src/domain49_50_naval_combat_live.rs`: compact encoder/decoder/gate for
  lifecycle and member rows, including recorder-facing payload summary decoders
  for lifecycle start/update/end rows, location/region rows, member deletion
  rows, task-force/fleet links, and CShip hull telemetry.
- `src/extractor.rs`: collector-side extraction boundary for validated
  `CNavalCombat -> CNavalCombatant -> registry -> CFEXMember` rows plus the
  current `CFEXMember -> CShip -> CTaskForce/CFleet` join.
- `src/bin/naval_combat_lane_proof.rs`: proof harness that emits a domain
  `49/50` compact start/update/end stream, gate JSON, and lane-local
  `.battleplan`. It also accepts `--fixture-json` from the active fixture
  harness to compact-decode and replay-verify a real detected fixture.
- `tools/active_fixture_proof.py`: bounded live watcher that attaches to a PID,
  captures the first valid `CNavalCombat` fixture, enriches it with static
  region and OOB CShip task-force/fleet joins, and invokes the Rust
  `--fixture-json` proof.
- `tools/fixture_independent_roots_audit.py`: PID-attached audit for non-combat
  CShip damage/counter roots, CShip task-force/fleet linkage, task-force
  location/region roots, mission/order candidates, home-base candidates, and
  direct theatre pointer rejection.
- `../../../documents/archive/native_lanes/naval_combat_live/REGISTRATION_HOOK_PLAN.md`: registration-only collector/recorder patch plan
  for domains `49` and `50`.
- `../../../documents/archive/native_lanes/naval_combat_live/LIVE_FIXTURE_RUNBOOK.md`: exact in-game fixture contract, probe commands,
  speed-five proof thresholds, and non-combat route audit.

## Current Validated Subset

- Current `CShip` route: `module+0x2941748`
- `CShip+0x08`: packed handle with type tag `51` and instance id
- `CShip+0x0C`: matching instance id alias
- `CShip+0x710`: `10000000` sentinel
- `CFEXMember+0x08` high u32 ship instance id must resolve to that current
  `CShip` route for every non-convoy ship member.
- Non-convoy combat members must also carry the OOB link supplied by the
  current native naval OOB path: task-force id, fleet id, and current CShip
  strength/org/XP/sunk-convoy telemetry. The lane gate fails closed if any
  CShip-validated member lacks this enrichment.
- Lifecycle rows must carry both the live front location id and the static
  strategic region id. Current proof fixture uses `location_id=2799` and
  `region_id=18`, matching local game data in
  `map/strategicregions/18-Channel.txt` (`STRATEGICREGION_18` contains
  province `2799`).
- Lifecycle end emits deleted member rows for every previous combat member so
  ship departure/sinking/end semantics are not represented only by a body
  deletion marker.

## Candidate Lifecycle/Member Roots

These are implemented as fail-closed offsets in `src/extractor.rs`, but remain
fixture-blocked until an active two-sided naval combat exists in the current
process:

- `CNavalCombat` body vtable: `module+0x29B6660`
- side vtable: `module+0x29BC908`
- registry vtable: `module+0x29B6588`
- `CFEXMember` vtable: `module+0x29F9608`
- body sides: `+0x18/+0x20`
- body/front location: `body+0x28 -> CProvince+0xA4`
- region: static `map/strategicregions/*.txt` province-to-region join, not a
  guessed live memory field
- side registry: `+0x18`
- side group vector: `+0xC8/+0xD4`
- group member vector: `+0x08/+0x14`
- member handle: `+0x08`
- member owner tag: `+0x20`
- member max strength: `+0x30`
- member state: `+0xE0`
- member engagement weight: `+0xF8`
- member battle slot: `+0x120`

## Integration Patch Plan

See `../../../documents/archive/native_lanes/naval_combat_live/REGISTRATION_HOOK_PLAN.md`. The shared integration must stay
registration-only until the orchestrator grants the slot: dependency entries,
domain `49/50` activation, one DLL coordinator callout, two recorder decode
dispatch arms, and proof commands. No broad shared entrypoint implementation is
part of this lane-local package.

## Proof Commands

```powershell
cargo test --manifest-path extractor\native\lanes\naval_combat_live\Cargo.toml
cargo run --manifest-path extractor\native\lanes\naval_combat_live\Cargo.toml --bin naval_combat_lane_proof -- --out-dir extractor\native\runs\naval_combat_live_lane_stack_870ebf_20260701_v4
cargo run --manifest-path extractor\native\snapshot\Cargo.toml --bin pack_replay -- verify extractor\native\runs\naval_combat_live_lane_stack_870ebf_20260701_v4\native_session.battleplan
python memory_reader\combat\naval_combat_cship_route_probe.py --pid 45880 --workers 4 --out-dir extractor\native\runs\naval_combat_live_cship_route_870ebf_20260701_v3 --member-bridge-file extractor\native\runs\naval_combat_live_current_fixture_probe_870ebf_20260701_v3\member_bridge.jsonl
python memory_reader\combat\naval_combat_live_fixture_probe.py --pid 45880 --workers 4 --require-two-sided --require-cship-join --out-dir extractor\native\runs\naval_combat_live_current_fixture_probe_870ebf_20260701_v3
$hoi4Pid = (Get-Process hoi4 | Select-Object -First 1 -ExpandProperty Id)
python extractor\native\lanes\naval_combat_live\tools\active_fixture_proof.py --pid $hoi4Pid --workers 4 --timeout-s 1800 --poll-s 5 --out-dir extractor\native\runs\naval_combat_live_active_fixture_harness_870ebf_<fixture>
python extractor\native\lanes\naval_oob_fleets_ships_live\naval_oob_gap_probe.py --out extractor\native\runs\naval_combat_live_oob_task_fleet_870ebf_20260701_v1\naval_oob_gap_probe.json --snapshot-label naval_combat_live_task_fleet_linkage_refresh
python extractor\native\lanes\naval_combat_live\tools\fixture_independent_roots_audit.py --pid $hoi4Pid --out-dir extractor\native\runs\naval_combat_live_fixture_independent_roots_870ebf_20260701_v1
```

Current lane-local proof `naval_combat_live_lane_stack_870ebf_20260701_v4`
passed with 3 lifecycle rows (1 start, 1 update, 1 end), 3 location rows, 3
region rows, 6 member rows, 2 deleted member rows, 3 CShip-validated members,
3 task-force/fleet linked members, 3 hull telemetry members, and byte-matched
`.battleplan` verification.

Current active-harness proof
`naval_combat_live_active_fixture_harness_870ebf_20260701_v4` ran five bounded
polls against PID `45880` and correctly stopped at `timeout_no_validated_fixture`
with `sha256_ok=true`, `scan.status=scanned`, and every poll at
`body_hit_count=0`, `bridge_row_count=0`, and `member_bridge_row_count=0`.

Current fixture-json mode proof
`naval_combat_live_fixture_json_mode_870ebf_20260701_v1` passed through the
same Rust `--fixture-json` path the active harness will call after detection:
1 lifecycle row, 2 member rows, 1 location row, 1 region row, 1
CShip/task-force/fleet linked member, byte-matched compact stream, and
`.battleplan` verification.

Current OOB linkage refresh
`naval_combat_live_oob_task_fleet_870ebf_20260701_v1` validates the
fixture-independent route for `CGameState`, `CFleet`, `CTaskForce`, and
`CShip`: 432 countries, 139 fleets, 163 task forces, 1288 ships, and 163/163
task forces with derived strategic regions.

Current fixture-independent root audit
`naval_combat_live_fixture_independent_roots_870ebf_20260701_v1` validates the
current PID route with 432 countries, 142 fleets, 163 task forces, and 1303
ships. All 1303 ships have task-force/fleet linkage plus `CShip+0x6F8`
strength, `+0x700` organisation, `+0x708` experience, and `+0x3C`
sunk-convoy counters; 158 ships have nonzero sunk-convoy counters. Direct
`CTheatre` pointers are rejected in `CFleet[0x0..0x1800]` and
`CTaskForce[0x0..0x800]`. Mission/repair/retreat/order semantics and
home-base/port semantics require controlled before/after fixtures, not an
active combat fixture.

## Remaining Blocker

The current live process has no active `CNavalCombat` body hits for
`module+0x29B6660`; see
`extractor/native/runs/naval_combat_live_current_fixture_probe_870ebf_20260701_v3/naval_combat_live_fixture_probe.json`.
The same process validates the current CShip route at
`extractor/native/runs/naval_combat_live_cship_route_870ebf_20260701_v3/naval_combat_cship_route_probe.json`
with `current_validated_count=1617` and rejects the stale CShip RVA with
`old_hit_count=0`. This blocks live lifecycle/member proof only, not the
lane-local compact module. Use `../../../documents/archive/native_lanes/naval_combat_live/LIVE_FIXTURE_RUNBOOK.md` for the exact battle
fixture and proof commands.
