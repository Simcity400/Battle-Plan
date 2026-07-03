# naval_combat_live lane-local production stack

> Active lane note: this README is the lane-local orientation page. Historical
> plans, proof reports, blocker notes, and registration packets for this lane
> are archived under `../../../documents/archive/native_lanes/naval_combat_live/`.
> Current extraction completion status lives in
> `../../../documents/MASTER_EXTRACTION_CONTRACT.md`.

This lane owns compact domains from `extractor/documents/DOMAIN_REGISTRY.md`:

- `49 = naval_combat_lifecycle_live`
- `50 = naval_combat_members_live`

The shared collector and recorder integration slot is intentionally separate.
This lane may prove constants and compact behavior, but the orchestrator must
serially port validated constants into shared hot files after live validation.

## Files

- `src/domain49_50_naval_combat_live.rs`: compact encoder/decoder/gate for
  lifecycle and member rows.
- `src/extractor.rs`: lane-local extraction boundary for validated
  `CNavalCombat -> CNavalCombatant -> CFEXMember` rows plus the current
  `CFEXMember -> CShip -> CTaskForce/CFleet` join.
- `src/bin/naval_combat_lane_proof.rs`: proof harness for synthetic and
  `--fixture-json` active-fixture compact/replay verification.
- `tools/active_fixture_proof.py`: bounded live harness that waits for a valid
  fixture, preserves `front+0xA4` as an unvalidated candidate, enriches CShip
  task-force/fleet data, and invokes the Rust proof.
- `CHAIN_REVERSAL_870EBF.md`: byte-level chain reversal notes for build 870ebf.
- `LIVE_VALIDATION_RUNSHEET.md`: commands and success/failure criteria for the
  next live naval battle validation.

## 870ebf Provenance

Current constants are pinned to evidence under
`extractor/native/runs/naval_combat_fixture_20260703_biscay_v2`, especially
`CHAIN_FINDINGS.md`, `raw_body_dump.json`, `side_dump.hex`, and `side2_dump.hex`.
The fixture build SHA is
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

## Current Layout

- `CNavalCombat` vtable: `module+0x29C6F18`.
- `CNavalCombatant` vtable: `module+0x29CD298`.
- `CFEXMember` vtable: `module+0x2A0A228`.
- `CShip` vtable: `module+0x2941748`.
- `CFleet` vtable: `module+0x294BF28`.
- `CTaskForce` vtable: `module+0x294C480`.
- Body handle: `body+0x18` packed `{type=0x1269, combat_id}`.
- Body sides: `body+0x28/+0x30`.
- Body front pointer: `body+0x38`.
- Side parent back-ref: `side+0x18 -> CNavalCombat`.
- Side group allocation: `side+0x2A0/+0x2A8/+0x2B0`, with captured
  `0x80`-byte allocations in the Biscay side dumps.
- Group member vector: `group+0x08` member data and `group+0x14` member count.
- Member handle: `CFEXMember+0x08`; high u32 joins to `CShip+0x08` high u32
  instance id for non-convoy members.

`CNavalCombatSide` has no own RTTI hit on 870ebf. The old registry vtable
constant is not part of this lane's adopted 870ebf route.

## Fail-Closed Rules

- `front+0xA4` is only `location_candidate_front_0xA4`, reported with
  `validated=false`. It must not be serialized as compact `location_id` or
  `region_id`.
- Compact lifecycle payloads leave location and region absent and set
  `STATUS_LOCATION_CANDIDATE_UNVALIDATED` until manual live verification and a
  later promotion change.
- Fixture JSON accepted by `naval_combat_lane_proof --fixture-json` must carry
  `source.root.sha256_ok == true` and `source.root.sha256` equal to the pinned
  870ebf SHA. Missing or mismatched build provenance hard-fails.
- With `--require-cship-join`, any unresolved non-convoy `CFEXMember -> CShip`
  join makes `validated_combat_count == 0`, suppresses normal `bridge.txt` and
  `member_bridge.jsonl`, and exits nonzero. Rejected diagnostics may use only
  `bridge.REJECTED.txt` and `member_bridge.REJECTED.jsonl`.

## Proof Commands

```powershell
Set-Location 'C:\Users\Example\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV\mod\battle-plan\extractor\native'
cargo test -p naval_combat_live_lane
cargo test --workspace
```

Next live validation must use `LIVE_VALIDATION_RUNSHEET.md`. The domain is not
validated until that live run succeeds and the orchestrator records the manual
location candidate comparison.
