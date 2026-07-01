# naval_combat_live live fixture runbook

This runbook is the remaining unblocker for domains:

- `49 = naval_combat_lifecycle_live`
- `50 = naval_combat_members_live`

Shared collector and recorder entrypoints are intentionally untouched here. Run
the production proof only after the orchestrator grants the integration slot and
the registration-only patch from `REGISTRATION_HOOK_PLAN.md` has landed.

## Required In-Game Fixture

Use the current `870ebf` `hoi4.exe` process. The probe checks the process SHA
and fails closed on mismatch.

The fixture must create a visible active naval battle with both sides present:

1. Two hostile countries must have fleets operating in the same sea zone.
2. Use fleet-vs-fleet surface or submarine combat, not a peacetime naval
   summary and not a naval OOB-only frame.
3. At least one non-convoy warship member must be present so the member row can
   prove `CFEXMember -> CShip` through the current CShip route.
4. Keep the battle active until the watcher detects it and writes non-empty
   `bridge.txt` and `member_bridge.jsonl`.
5. For lifecycle-end proof, keep recording until that same battle ends and the
   compact stream contains the corresponding lifecycle deletion/end evidence.
6. For task-force/fleet linkage and damage telemetry, run with the current
   native naval OOB/ships path available so each non-convoy combat member's
   CShip instance can be joined to task-force id, fleet id, strength, org, XP,
   and sunk-convoy counters.
7. The bridge must supply both front province/location and strategic region.
   Location is read from the validated body/front route; region is the static
   `map/strategicregions/*.txt` province-to-region join. The synthetic proof
   fixture uses province `2799`, which maps to strategic region `18` in
   `18-Channel.txt`.

## Expected Live Evidence

The fixture probe must prove this exact route:

- `CNavalCombat` body vtable: `module+0x29B6660`
- body handle: `CNavalCombat+0x08`, low u32 type `4713`, high u32 combat id
- attacker/defender sides: `body+0x18/+0x20`
- `CNavalCombatant` side vtable: `module+0x29BC908`
- side registry: `side+0x18`
- nested registry vtable: `module+0x29B6588`
- registry sides/front: `registry+0x28/+0x30/+0x38`
- side group vector: `side+0xC8` data, `side+0xD4` size
- group member vector: `group+0x08` data, `group+0x14` size
- `CFEXMember` vtable: `module+0x29F9608`
- member handle: `CFEXMember+0x08`
- front location: `body+0x28 -> CProvince+0xA4`
- strategic region: static `map/strategicregions/*.txt` province-to-region map
- current CShip route: `CFEXMember+0x08` high u32 ship instance id resolves to
  `CShip+0x08` high u32 through CShip vtable `module+0x2941748`
- current native OOB route: the same CShip instance resolves through
  `CGameState+0x310 -> CCountry+0x278 -> CFleet* -> CTaskForce/CShip`, with
  `CTaskForce+0x330` parent fleet and `CTaskForce+0x338/+0x344` ship members

Detection passes only when the probe output has:

- `root.sha256_ok == true`
- `scan.status == "scanned"`
- `status == "validated_active_fixture"`
- `body_hit_count >= 1`
- `validated_combat_count >= 1`
- `bridge_row_count >= 1`
- `member_bridge_row_count >= 2`
- attacker and defender `CFEXMember` rows for the same combat id/generation
- every lifecycle bridge row has `location_id` and `region_id`
- `non_convoy_member_count >= 1`
- `cship_unresolved_member_count == 0`
- every non-convoy member row has task-force and fleet ids
- every non-convoy member row has current CShip strength/org/XP/sunk-convoy
  telemetry
- non-empty `bridge.txt`
- non-empty `member_bridge.jsonl`

## Detection Commands

Preferred one-command lane harness:

```powershell
$hoi4Pid = (Get-Process the game | Select-Object -First 1 -ExpandProperty Id)
python replay_native\lanes\naval_combat_live\tools\active_fixture_proof.py `
  --pid $hoi4Pid `
  --workers 4 `
  --timeout-s 1800 `
  --poll-s 5 `
  --require-two-sided `
  --require-cship-join `
  --out-dir replay_native\runs\naval_combat_live_active_fixture_harness_870ebf_<fixture>
```

When a valid fixture appears, this writes `bridge.txt`,
`member_bridge.jsonl`, `cship_task_force_index.json`,
`active_fixture_input.json`, runs:

```powershell
cargo run --manifest-path replay_native\lanes\naval_combat_live\Cargo.toml --bin naval_combat_lane_proof -- --out-dir <out>\rust_fixture_proof --fixture-json <out>\active_fixture_input.json
```

and verifies `<out>\rust_fixture_proof\native_session.battleplan`.

Use this first to prove the active fixture and write bridge rows:

```powershell
$hoi4Pid = (Get-Process the game | Select-Object -First 1 -ExpandProperty Id)
$out = "replay_native\runs\naval_combat_live_active_fixture_870ebf_20260701_v3"
python memory_reader\combat\naval_combat_live_fixture_probe.py `
  --pid $hoi4Pid `
  --workers 4 `
  --require-two-sided `
  --require-cship-join `
  --out-dir $out
```

Use this for a bounded wait while the user creates the battle:

```powershell
$hoi4Pid = (Get-Process the game | Select-Object -First 1 -ExpandProperty Id)
$out = "replay_native\runs\naval_combat_live_active_fixture_watch_870ebf_20260701_v3"
python memory_reader\combat\naval_combat_live_watch_gate.py `
  --pid $hoi4Pid `
  --workers 4 `
  --timeout-s 1800 `
  --poll-s 5 `
  --require-two-sided `
  --require-cship-join `
  --out-dir $out `
  --record-duration-s 180 `
  --sample-ms 10 `
  --until-game-hours 12 `
  --max-wall-s 360
```

When `bridge.txt` is non-empty, run the lane-local bridge proof:

```powershell
python memory_reader\combat\naval_combat_compact_proof.py `
  --bridge-file $out\bridge.txt `
  --out-dir replay_native\runs\naval_combat_live_detected_bridge_compact_870ebf_20260701_v3
```

The lane-local Rust/synthetic replay proof remains:

```powershell
cargo test --manifest-path replay_native\lanes\naval_combat_live\Cargo.toml
cargo run --manifest-path replay_native\lanes\naval_combat_live\Cargo.toml --bin naval_combat_lane_proof -- --out-dir replay_native\runs\naval_combat_live_lane_stack_870ebf_20260701_v4
cargo run --manifest-path replay_native\snapshot\Cargo.toml --bin pack_replay -- verify replay_native\runs\naval_combat_live_lane_stack_870ebf_20260701_v4\native_session.battleplan
```

## Production Proof After Registration

After the collector/recorder registration-only patch lands, run the integrated
compact proof while the battle is active:

```powershell
replay_native\target\release\collector_record.exe `
  --pid $pid `
  --dll replay_native\target\release\collector.dll `
  --out-dir replay_native\runs\naval_combat_live_integrated_870ebf_20260701_v1 `
  --duration-s 180 `
  --sample-ms 10 `
  --min-game-hours 12 `
  --scheduled-domain naval_combat_lifecycle_live `
  --scheduled-domain naval_combat_members_live `
  --stream-format compact `
  --extraction-perf-gate `
  --lightweight-root-frame
```

Then verify the run:

```powershell
cargo run --manifest-path replay_native\lanes\compact_replay_verification_gates\Cargo.toml -- `
  --run-dir replay_native\runs\naval_combat_live_integrated_870ebf_20260701_v1 `
  --require-domain naval_combat_lifecycle_live `
  --require-domain naval_combat_members_live `
  --min-game-hours 12 `
  --write-report replay_native\runs\naval_combat_live_integrated_870ebf_20260701_v1\compact_gate_report.json
```

The compact replay gate must be updated in that integration slot with semantic
counters for domains `49/50` before this command can become the final merge
gate.

## Speed-Five Acceptance Threshold

The production proof can count only if all of these are true:

- The game is running at speed five or the current equivalent fastest live
  cadence.
- `game_hours_advanced >= 12`, unless the same combat naturally ends earlier
  and an explicit lifecycle end/deletion row is captured.
- `hourly_requirement.met == true`.
- The largest observed game-hour delta is hourly-or-lower.
- `skipped_game_hours == 0`.
- The compact stream includes typed domain data records for both domains `49`
  and `50` during the active combat window.
- Domain `49` has start/update/end lifecycle rows for the same combat, and
  every lifecycle row has both `location_id` and `region_id`.
- Domain `50` has attacker and defender member rows for the same
  combat id/generation.
- Domain `50` emits deleted member rows when a combat ends, and when a member
  disappears from an otherwise active combat.
- At least one non-convoy member row has a validated current CShip join.
- Every CShip-validated member row has task-force/fleet linkage and current
  CShip hull telemetry.
- No `source_unavailable`, `partial_data`, `bridge_unavailable`, or
  `worker_panic` status is present.
- `.battleplan` verification passes and the embedded compact stream byte-matches
  `collector_stream.bin`.

Peacetime no-combat summaries, OOB-only ship rows, and saved stale combat
blocks do not satisfy this threshold.

## Fail And Reject Criteria

Treat these as candidate rejection evidence:

- `body_hit_count > 0` under a visible active battle, but no body passes
  vtable, handle, side, registry, and front validation.
- Body/side/registry validation passes, but either side has zero
  `CFEXMember` rows under a visible fleet-vs-fleet fixture.
- Body validation passes, but `body+0x28 -> CProvince+0xA4` cannot provide a
  valid location id for the lifecycle row.
- Location validation passes, but static strategic-region data cannot map that
  province to a region id.
- A non-convoy `CFEXMember` row has a ship instance id that cannot resolve
  through current CShip `module+0x2941748`.
- A non-convoy CShip-validated member lacks task-force/fleet linkage from the
  current native OOB path.
- The integrated compact run emits domain `49/50` rows that fail the lane gate
  or compact replay gate.

Treat this as an external fixture blocker, not candidate rejection:

- The watcher times out with `fixture_absent`, `body_hit_count=0`,
  `bridge_row_count=0`, and `member_bridge_row_count=0`.

## Non-Combat Route Audit

There is no remaining non-combat naval route in this lane that can complete
domains `49/50` without an active battle fixture.

- Current CShip resolver is already validated live:
  `module+0x2941748`, `current_validated_count=1617`, and stale
  `module+0x2931998` is rejected with `old_hit_count=0`. Refreshed route proof:
  `replay_native/runs/naval_combat_live_cship_route_870ebf_20260701_v3/naval_combat_cship_route_probe.json`.
- Current OOB task-force/fleet linkage is validated without an active combat:
  `replay_native/runs/naval_combat_live_oob_task_fleet_870ebf_20260701_v1/naval_oob_gap_probe.json`
  reports `CGameState module+0x3316E90`, `CGameState+0x310` country table,
  `CFleet module+0x294BF28`, `CTaskForce module+0x294C660`, `CShip
  module+0x2941748`, `CTaskForce+0x338/+0x344` ship vector, 432 countries, 139
  fleets, 163 task forces, 1288 ships, and 163/163 task forces with derived
  strategic regions.
- Current fixture-independent root audit:
  `replay_native/runs/naval_combat_live_fixture_independent_roots_870ebf_20260701_v1/fixture_independent_roots_audit.json`
  reports 432 countries, 142 fleets, 163 task forces, and 1303 ships. All 1303
  ships have task-force/fleet linkage plus `CShip+0x6F8` strength,
  `CShip+0x700` organisation, `CShip+0x708` experience, and `CShip+0x3C`
  sunk-convoy counters; 158 ships have nonzero sunk-convoy counters. It also
  validates `CTaskForce+0x18` current location and 163/163 task forces with
  derived strategic regions.
- Current fixture-independent rejections/blockers:
  `theatre_linkage.status=rejected_direct_pointer_scan` with zero direct
  `CTheatre` pointer offsets in `CFleet[0x0..0x1800]` and
  `CTaskForce[0x0..0x800]`; `mission_order_candidates.status=blocked_fixture_required`
  with 40/40 single-snapshot candidate offsets all zero; and
  `home_base_port_candidates.status=blocked_fixture_required` with province
  pointer/id candidates requiring a before/after port or home-base change.
- Current live fixture probe still has no combat/member rows:
  PID `45880`, SHA
  `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`,
  `body_hit_count=0`, `validated_combat_count=0`, `bridge_row_count=0`,
  `member_bridge_row_count=0`. Refreshed probe:
  `replay_native/runs/naval_combat_live_current_fixture_probe_870ebf_20260701_v3/naval_combat_live_fixture_probe.json`
  reports `status=fixture_absent`, `scan.status=scanned`, `sha256_ok=true`,
  `scan.elapsed_s=6.0`, and zero body, bridge, member, resolved, or unresolved
  CShip rows.
- Current active harness proof:
  `replay_native/runs/naval_combat_live_active_fixture_harness_870ebf_20260701_v4/active_fixture_harness_summary.json`
  reports five bounded polls against PID `45880`, `status=timeout_no_validated_fixture`,
  `final_fixture_status=fixture_absent`, `scan.status=scanned`,
  `sha256_ok=true`, and every poll at `body_hit_count=0`,
  `bridge_row_count=0`, and `member_bridge_row_count=0`.
- Current fixture-json mode proof:
  `replay_native/runs/naval_combat_live_fixture_json_mode_870ebf_20260701_v1/rust_fixture_proof/naval_combat_live_lane_stack_proof.json`
  passes the same Rust path the active harness calls after detection, with 1
  lifecycle row, 2 member rows, 1 location row, 1 region row, 1
  CShip/task-force/fleet linked member, byte-matched compact stream, and
  verified `.battleplan`.
- Static strategic-region mapping is validated for the lane fixture:
  `<local-path> of Iron IV\map\strategicregions\18-Channel.txt`
  has `id=18` and contains province `2799`.
- The long watcher already ran `58` polls and ended
  `timeout_no_validated_fixture` with first and last `body_hit_count=0`.
- Prior current-build direct scans also found no active combat-family bodies:
  `CFEXMember_vtable module+0x29F9608 hit_count=0`,
  `CNavalCombat_vtable module+0x29B6660 hit_count=0`, and
  `CNavalCombatant_side_vtable module+0x29BC908 hit_count=0`.
- `naval_combat_stats.py` depends on active `enumerate_naval_combats` output.
  It has no independent non-combat root.
- `naval_air_reader.py` documents that the land `CCombatSide` route is not
  reused for naval/air combat; following it would switch lanes and repeat a
  previously rejected path.
- Naval OOB ship/fleet captures prove persistent ships and task forces, not
  `CNavalCombat -> CNavalCombatant -> registry -> CFEXMember` lifecycle/member
  production extraction.
