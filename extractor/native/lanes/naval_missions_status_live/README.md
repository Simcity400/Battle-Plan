# naval_missions_status_live
> Active lane note: this README is the only active lane-local documentation. Historical plans, proof reports, blocker notes, and registration packets for this lane are archived under ../../../documents/archive/native_lanes/naval_missions_status_live/. Current extraction completion status lives in ../../../documents/MASTER_EXTRACTION_CONTRACT.md.


Lane-local production-shaped stack for final compact domain 56,
`naval_missions_status_live`.

## Current status

Completion status: `PRODUCTION-SAFE CORE; EXACT RESIDUAL FIXTURES FROZEN` (2026-07-19).

The shared collector/recorder now emits and folds this domain. Mission,
home-base, and repair-target are direct validated fields. The exact build
870ebf mission enum is `0..=9`: HOLD, PATROL, STRIKE_FORCE, CONVOY_RAIDING,
CONVOY_ESCORT, MINES_PLANTING, MINES_SWEEPING, TRAINING, RESERVE_FLEET, and
NAVAL_INVASION_SUPPORT. Unknown values fail closed. Mission activity,
task-force repair progress, retreat classification, and convoy-route loss
identity remain absent rather than guessed; their exact capture requirements
are frozen in the naval-combat lane's `A8_A9_B6_HUMAN_INPUT_REQUIREMENTS.json`.

The existing shared collector/recorder integration is retained; the recorder
projection now adds the exact `mission_name` beside the raw enum. No master
status file is edited by this lane.

## Files

- `src/domain56_compact.rs`: final domain-56 wire schema, payload encoder, compact
  stream gate, and tests.
- `naval_missions_status_probe.py`: bounded live proof probe against `hoi4.exe`.
- `../../../documents/archive/native_lanes/naval_missions_status_live/REGISTRATION_PATCH_PLAN.md`: exact shared registration plan for the
  orchestrator.

Proof output:

- `extractor/native/runs/naval_missions_status_live_870ebf_20260701_v1/naval_missions_status_probe.json`

## Live proof summary

Process:

- `hoi4.exe` pid `45880`
- image SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`

Observed roots and rows:

- countries: 432
- fleets: 143
- task forces: 168
- ships: 1376

Validated or reused roots:

- `module+0x3316E90 -> CGameState`
- `CGameState+0x310` country table, count at `CGameState+0x31C`
- `CCountry+0x278/+0x280/+0x284` fleet vector
- `CFleet` vtable `module+0x294BF28`
- `CTaskForce` vtable `module+0x294C660`
- `CTaskForce+0x18` u32 current province/location
- `CTaskForce+0x338/+0x344` ship vector
- `CShip+0x3C` u32 aggregate sunk-convoy count

## Field results

Naval mission enum: `PROVEN`.

`CTaskForce+0x364` is the validated raw enum. Ghidra function
outside that range. `src/lib.rs` carries this exact map and the compact gate
now shares the same maximum rather than accepting speculative wire headroom.

Home base and repair target: `PROVEN`. Activity/progress/retreat: `ABSENT,
EXACT FIXTURE REQUIRED`.

Home base is `CFleet+0xD8 -> CProvince+0xA4`; nullable repair target is
`CTaskForce+0x4B0 -> CProvince+0xA4`. The engine's task-force activity label is
mission, location, target, detachment mode, and mission efficiency. The
task-force serializer has no proved scalar repair-progress or retreat field,
so those compact columns remain unflagged.

Convoy combat/loss route context: `FIXTURE_BLOCKED`, narrowed.

The current roots expose aggregate sunk-convoy counts only. Live proof found 30
task forces with sunk-convoy counters and 1598 total observed sunk convoys via
`CShip+0x3C`. Route/loss context was not reachable from fixture-independent
fleet/task-force/ship roots. Required fixture: active convoy battle/loss with
known route/mission context.

Strategic region rollups: validated slice with a mapping gap.

The probe derives regions from `CTaskForce+0x18` plus
`memory_reader/weather/prov2region_mod.json` with 13413 entries. One live task
force had `location_province_id = 0`, so region rows must be badged/omitted for
unmapped or zero-location task forces.

## Master row change proposals

Do not edit `MASTER_EXTRACTION_CONTRACT.md` directly from this lane. Proposed
orchestrator-owned row updates:

| Row | Proposed status | Proposed evidence / domain | Proposed remaining closure condition |
| --- | --- | --- | --- |
| Home base / repair target | `PROVEN` | `CFleet+0xD8` and `CTaskForce+0x4B0`, both joined through `CProvince+0xA4` | Preserve nullable repair-target absence. |
| Activity / repair progress / retreat | `EXACT FIXTURE REQUIRED` | engine activity formatter and task-force serializer negative evidence | Controlled transition fixtures in `A8_A9_B6_HUMAN_INPUT_REQUIREMENTS.json`; no scalar inference. |
| Convoy combat/losses | `FIXTURE_BLOCKED` | `naval_missions_status_live` / 56 lane proof plus `ships_live` sunk-convoy counter | Active convoy battle/loss route fixture to link losses to route/mission context; `CShip+0x3C` remains only aggregate sunk-convoy count. |

## Proof commands

```powershell
python -m py_compile extractor/native/lanes/naval_missions_status_live/naval_missions_status_probe.py
cargo test --manifest-path extractor/native/lanes/naval_missions_status_live/Cargo.toml
python extractor/native/lanes/naval_missions_status_live/naval_missions_status_probe.py --out extractor/native/runs/naval_missions_status_live_870ebf_20260701_v1/naval_missions_status_probe.json --snapshot-label domain56_live_probe_v1
```

The Cargo invocation is the supported Rust validation path. The active module
is `src/domain56_compact.rs` (there is no lane-root `domain56_compact.rs`), and
the crate depends on the shared `compact_contract`, so a bare `rustc --test`
command does not resolve the production dependency graph.

Results:

- Python compile: pass.
- Rust tests: rerun the Cargo command above after source changes; the historical
  bare-rustc result predates the shared-contract dependency.
- Live probe: pass, proof JSON written.

## Required report fields

- Completion status: `NOT COMPLETE`
- Backlog reduced: yes
- Exact backlog item removed or narrowed: domain-56 compact schema/gate is
  implemented; mission/home-base/repair/retreat fixture boundaries are exact;
  convoy route context is narrowed to aggregate `CShip+0x3C` only without an
  active convoy fixture.
- Exact files changed: lane-local files in
  `extractor/native/lanes/naval_missions_status_live/`.
- Exact roots/offsets/functions validated or rejected: listed above.
- Compact schema/record changes: final domain 56, 44-byte header, 64-byte
  task-force rows, badged fixture-required fields.
- Rust decoder/gate changes: `src/domain56_compact.rs`.
- Proof commands/results: listed above.
- DLL-only: no shared DLL integration yet; lane-local proof uses bounded live
  validation, not hot extraction.
- Compact stream: schema/gate implemented; no shared domain-56 compact stream
  emitted yet.
- Rust decoded: yes for synthetic domain-56 compact stream tests.
- `.battleplan` verified: no.
- Speed-five/current-equivalent hourly-or-lower: no, not integrated into shared
  hourly scheduler.
- Zero skipped hours: no shared run for domain 56 yet.
- Any partial/source/worker/bridge failures: no lane-local probe failures; shared
  production extraction remains unregistered.
