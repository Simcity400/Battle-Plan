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
- current CShip route: `CFEXMember+0x08` high u32 ship instance id resolves to
  `CShip+0x08` high u32 through CShip vtable `module+0x2941748`

Detection passes only when the probe output has:

- `root.sha256_ok == true`
- `scan.status == "scanned"`
- `status == "validated_active_fixture"`
- `body_hit_count >= 1`
- `validated_combat_count >= 1`
- `bridge_row_count >= 1`
- `member_bridge_row_count >= 2`
- attacker and defender `CFEXMember` rows for the same combat id/generation
- `non_convoy_member_count >= 1`
- `cship_unresolved_member_count == 0`
- non-empty `bridge.txt`
- non-empty `member_bridge.jsonl`

## Detection Commands

Use this first to prove the active fixture and write bridge rows:

```powershell
$pid = (Get-Process the game | Select-Object -First 1 -ExpandProperty Id)
$out = "replay_native\runs\naval_combat_live_active_fixture_870ebf_20260701_v1"
python memory_reader\combat\naval_combat_live_fixture_probe.py `
  --pid $pid `
  --workers 4 `
  --require-two-sided `
  --require-cship-join `
  --out-dir $out
```

Use this for a bounded wait while the user creates the battle:

```powershell
$pid = (Get-Process the game | Select-Object -First 1 -ExpandProperty Id)
$out = "replay_native\runs\naval_combat_live_active_fixture_watch_870ebf_20260701_v1"
python memory_reader\combat\naval_combat_live_watch_gate.py `
  --pid $pid `
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
  --out-dir replay_native\runs\naval_combat_live_detected_bridge_compact_870ebf_20260701_v1
```

The lane-local Rust/synthetic replay proof remains:

```powershell
cargo test --manifest-path replay_native\lanes\naval_combat_live\Cargo.toml
cargo run --manifest-path replay_native\lanes\naval_combat_live\Cargo.toml --bin naval_combat_lane_proof -- --out-dir replay_native\runs\naval_combat_live_lane_stack_870ebf_20260701_v1
cargo run --manifest-path replay_native\snapshot\Cargo.toml --bin pack_replay -- verify replay_native\runs\naval_combat_live_lane_stack_870ebf_20260701_v1\native_session.battleplan
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
- Domain `49` has at least one active lifecycle row.
- Domain `50` has attacker and defender member rows for the same
  combat id/generation.
- At least one non-convoy member row has a validated current CShip join.
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
- A non-convoy `CFEXMember` row has a ship instance id that cannot resolve
  through current CShip `module+0x2941748`.
- The integrated compact run emits domain `49/50` rows that fail the lane gate
  or compact replay gate.

Treat this as an external fixture blocker, not candidate rejection:

- The watcher times out with `fixture_absent`, `body_hit_count=0`,
  `bridge_row_count=0`, and `member_bridge_row_count=0`.

## Non-Combat Route Audit

There is no remaining non-combat naval route in this lane that can complete
domains `49/50` without an active battle fixture.

- Current CShip resolver is already validated live:
  `module+0x2941748`, `current_validated_count=1772`, and stale
  `module+0x2931998` is rejected with `old_hit_count=0`.
- Current live fixture probe still has no combat/member rows:
  PID `45880`, SHA
  `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`,
  `body_hit_count=0`, `validated_combat_count=0`, `bridge_row_count=0`,
  `member_bridge_row_count=0`.
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
