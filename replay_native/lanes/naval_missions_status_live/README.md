# naval_missions_status_live

Lane-local production-shaped stack for final compact domain 56,
`naval_missions_status_live`.

## Current status

Completion status: `NOT COMPLETE`.

Backlog reduced: yes. This lane adds the final domain-56 compact schema and
decoder/gate, proves the current live naval roots for task-force identity,
fleet identity, owner, current province, derived region where mapped, and
aggregate sunk-convoy counters, and separates the remaining mission/home-base/
repair/retreat/convoy-route fields behind exact fixture requirements.

No shared collector, recorder, or master status files were edited.

## Files

- `domain56_compact.rs`: final domain-56 wire schema, payload encoder, compact
  stream gate, and tests.
- `naval_missions_status_probe.py`: bounded live proof probe against `hoi4.exe`.
- `REGISTRATION_PATCH_PLAN.md`: exact shared registration plan for the
  orchestrator.

Proof output:

- `replay_native/runs/naval_missions_status_live_870ebf_20260701_v1/naval_missions_status_probe.json`

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

Naval missions: `FIXTURE_BLOCKED`.

The bounded CTaskForce scalar scans produce small-u32 candidates, but a single
snapshot cannot assign mission enum/state semantics. Required fixture:
controlled before/after task-force mission change with stable `task_force_id`
and `fleet_id`.

Home base/repair/retreat: `FIXTURE_BLOCKED`.

`CTaskForce+0x18` remains validated as current location, not home base. Home
base needs a controlled port/base-change fixture. Repair and retreat need a
damaged/repairing or retreating task-force fixture before any scalar or
province candidate can be promoted.

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
| Naval missions | `FIXTURE_BLOCKED` | `naval_missions_status_live` / 56 lane proof | Controlled task-force mission before/after fixture to select exact mission enum/state offsets; domain-56 compact schema and validated task-force/location roots are lane-local. |
| Home base/repair/retreat | `FIXTURE_BLOCKED` | `naval_missions_status_live` / 56 lane proof | Controlled port-change, damaged-repairing, and retreating task-force fixtures; do not promote `CTaskForce+0x18` because it is current location, not home base. |
| Convoy combat/losses | `FIXTURE_BLOCKED` | `naval_missions_status_live` / 56 lane proof plus `ships_live` sunk-convoy counter | Active convoy battle/loss route fixture to link losses to route/mission context; `CShip+0x3C` remains only aggregate sunk-convoy count. |

## Proof commands

```powershell
python -m py_compile replay_native/lanes/naval_missions_status_live/naval_missions_status_probe.py
rustc --edition=2021 --test replay_native/lanes/naval_missions_status_live/domain56_compact.rs -o replay_native/lanes/naval_missions_status_live/domain56_compact_tests.exe
replay_native/lanes/naval_missions_status_live/domain56_compact_tests.exe
python replay_native/lanes/naval_missions_status_live/naval_missions_status_probe.py --out replay_native/runs/naval_missions_status_live_870ebf_20260701_v1/naval_missions_status_probe.json --snapshot-label domain56_live_probe_v1
```

Results:

- Python compile: pass.
- Rust tests: pass, 4 tests.
- Live probe: pass, proof JSON written.

## Required report fields

- Completion status: `NOT COMPLETE`
- Backlog reduced: yes
- Exact backlog item removed or narrowed: domain-56 compact schema/gate is
  implemented; mission/home-base/repair/retreat fixture boundaries are exact;
  convoy route context is narrowed to aggregate `CShip+0x3C` only without an
  active convoy fixture.
- Exact files changed: lane-local files in
  `replay_native/lanes/naval_missions_status_live/`.
- Exact roots/offsets/functions validated or rejected: listed above.
- Compact schema/record changes: final domain 56, 44-byte header, 64-byte
  task-force rows, badged fixture-required fields.
- Rust decoder/gate changes: `domain56_compact.rs`.
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
