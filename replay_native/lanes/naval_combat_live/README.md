# naval_combat_live lane-local production stack

This lane owns compact domains assigned by `replay_native/DOMAIN_REGISTRY.md`:

- `49 = naval_combat_lifecycle_live`
- `50 = naval_combat_members_live`

The module is deliberately lane-local while another lane owns the shared
collector/recorder integration slot.

## Files

- `src/domain49_50_naval_combat_live.rs`: compact encoder/decoder/gate for
  lifecycle and member rows, including recorder-facing payload summary decoders.
- `src/extractor.rs`: collector-side extraction boundary for validated
  `CNavalCombat -> CNavalCombatant -> registry -> CFEXMember` rows plus the
  current `CFEXMember -> CShip` join.
- `src/bin/naval_combat_lane_proof.rs`: proof harness that emits a domain
  `49/50` compact stream, gate JSON, and lane-local `.battleplan`.
- `REGISTRATION_HOOK_PLAN.md`: registration-only collector/recorder patch plan
  for domains `49` and `50`.
- `LIVE_FIXTURE_RUNBOOK.md`: exact in-game fixture contract, probe commands,
  speed-five proof thresholds, and non-combat route audit.

## Current Validated Subset

- Current `CShip` route: `module+0x2941748`
- `CShip+0x08`: packed handle with type tag `51` and instance id
- `CShip+0x0C`: matching instance id alias
- `CShip+0x710`: `10000000` sentinel
- `CFEXMember+0x08` high u32 ship instance id must resolve to that current
  `CShip` route for every non-convoy ship member.

## Candidate Lifecycle/Member Roots

These are implemented as fail-closed offsets in `src/extractor.rs`, but remain
fixture-blocked until an active two-sided naval combat exists in the current
process:

- `CNavalCombat` body vtable: `module+0x29B6660`
- side vtable: `module+0x29BC908`
- registry vtable: `module+0x29B6588`
- `CFEXMember` vtable: `module+0x29F9608`
- body sides: `+0x18/+0x20`
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

See `REGISTRATION_HOOK_PLAN.md`. The shared integration must stay
registration-only until the orchestrator grants the slot: dependency entries,
domain `49/50` activation, one DLL coordinator callout, two recorder decode
dispatch arms, and proof commands. No broad shared entrypoint implementation is
part of this lane-local package.

## Proof Commands

```powershell
cargo test --manifest-path replay_native\lanes\naval_combat_live\Cargo.toml
cargo run --manifest-path replay_native\lanes\naval_combat_live\Cargo.toml --bin naval_combat_lane_proof -- --out-dir replay_native\runs\naval_combat_live_lane_stack_870ebf_20260701_v1
cargo run --manifest-path replay_native\snapshot\Cargo.toml --bin pack_replay -- verify replay_native\runs\naval_combat_live_lane_stack_870ebf_20260701_v1\native_session.battleplan
python memory_reader\combat\naval_combat_cship_route_probe.py --pid 45880 --workers 4 --out-dir replay_native\runs\naval_combat_live_lane_stack_870ebf_20260701_v1\cship_route_probe --member-bridge-file replay_native\runs\naval_combat_live_lane_stack_870ebf_20260701_v1\live_fixture_probe\member_bridge.jsonl
python memory_reader\combat\naval_combat_live_fixture_probe.py --pid 45880 --workers 4 --require-two-sided --require-cship-join --out-dir replay_native\runs\naval_combat_live_lane_stack_870ebf_20260701_v1\live_fixture_probe
```

## Remaining Blocker

The current live process has no active `CNavalCombat` body hits for
`module+0x29B6660`; see
`replay_native/runs/naval_combat_live_lane_stack_870ebf_20260701_v1/live_fixture_probe/naval_combat_live_fixture_probe.json`.
This blocks live lifecycle/member proof only, not the lane-local compact module.
Use `LIVE_FIXTURE_RUNBOOK.md` for the exact battle fixture and proof commands.
