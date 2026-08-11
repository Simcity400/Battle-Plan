# naval_missions_status_live
> Active lane note: this README is the only active lane-local documentation. Historical plans, proof reports, blocker notes, and registration packets for this lane are archived under ../../../documents/archive/native_lanes/naval_missions_status_live/. Current extraction completion status lives in ../../../documents/MASTER_EXTRACTION_CONTRACT.md.


Lane-local production-shaped stack for final compact domain 56,
`naval_missions_status_live`.

## Current status

Completion status: `PRODUCTION-READY EXACT D56 SLICE` (2026-07-20).

The shared collector/recorder now emits and folds this domain. Mission,
home-base, and repair-target are direct validated fields. The exact build
870ebf mission enum is `0..=9`: HOLD, PATROL, STRIKE_FORCE, CONVOY_RAIDING,
CONVOY_ESCORT, MINES_PLANTING, MINES_SWEEPING, TRAINING, RESERVE_FLEET, and
NAVAL_INVASION_SUPPORT. Unknown values fail closed. The exact persisted
`detached_activity` enum is emitted as `mission_state` (`0..=4`: none,
repairing, moving-to-refit, refitting, reinforcing). Repair progress, a live
task-force retreat enum, and convoy-route loss identity are not D56 fields.
Their schema-v1 bit/column positions are decoder-rejected and are not projected
into replay JSON. Exact save-tick repair links/mode, province movement `path`,
and boolean `retreat` are surfaced independently by the save oracle and typed
replay. Convoy route/loss attribution belongs exclusively to D90's
pre-collapse full-EntityHandle join.

The recorder also performs the exact cross-domain activity join that is now
possible without another hook. The a729 formatter tests combat membership
before every mission/movement/detachment branch. A same-hour successful D49
active `(combat_id,generation)` set plus a D50 `task_force_id`/side member in
exact `FEX_STATE_FIGHTING` state 2 therefore emits
`TASK_FORCE_ACTIVITY_DESC_IN_COMBAT` and pointer-free
`active_combat_epochs`. D49/D50 failure produces `null`; measured no membership
produces `[]`; non-fighting membership retains its epochs but no activity key.
The other formatter branches remain unavailable because their
full target/area/accessibility/efficiency inputs are not carried, so the fold
does not turn `mission_state` into a guessed display label. Fresh exact-20
semantics and the production disposition are specified in
`../../../../replay_interface/documents/REPLAY_SCHEMA.md`. The pinned read-only
20-shard export ran on `shard-06` as job
`20260720T050110Z-w4-d56-activity-member-epoch-authority-772412cd`; its job JSON
SHA-256 is
`1a2357da51b46cae9a03659d9872793646bf90bab4327cf1ebc9ae4eb2ef713f`
and evidence-manifest SHA-256 is
`bd6628f69f397e1f9ba3c40b9629731a5704c41191471da230ba24bd5ed3995e`.
The `export_functions` receipt is pinned to image base `0x140000000` and
executable SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`;
its external manifest path is
`<local-path>`.
The export seeds were `0x140D555A0`, `0x140D55F90`, `0x140D5A690`,
`0x140D60C40`, and `0x141E18460`.

The existing shared collector/recorder integration is retained; the recorder
projection adds exact mission/detached-state names and the D49/D50 epoch join.
The replay typed fleet view and selected-naval-unit binding preserve all four
activity leaves independently. No master
status file is edited by this lane.

## Files

- `src/domain56_compact.rs`: final domain-56 wire schema, payload encoder, compact
  stream gate, and tests.
- `naval_missions_status_probe.py`: bounded live proof probe against `hoi4.exe`.
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

Home base, repair target, and detached activity: `PROVEN`. Progress/retreat:
`MECHANICALLY REJECTED/ABSENT`.

Home base is `CFleet+0xD8 -> CProvince+0xA4`; nullable repair target is
`CTaskForce+0x4B0 -> CProvince+0xA4`. The localized task-force activity label is
derived from combat state, mission, location, target, detachment mode, and
mission efficiency, but its exact detached-activity input is stored at task-force
body `+0x4A8`. The formatter receives the authenticated subobject at body
`-0x10`, making formatter `+0x4B8` the same field. Serializer body
`+0x4B8/+0x4C0` are nullable object pointers, not scalar repair-progress or
retreat fields, so those compact columns remain unflagged.

Convoy combat/loss route context: `NOT A D56 CARRIER; RETIRED`.

The current roots expose aggregate sunk-convoy counts only. Live proof found 30
task forces with sunk-convoy counters and 1598 total observed sunk convoys via
`CShip+0x3C`. D90 owns route/loss context through the full client EntityHandle
`{id,type_tag=4713}` and typed source/destination endpoints. D56's former route
column is reserved and rejected; movement path, current location, and mission
are never treated as loss-route identity.

Strategic region rollups: validated slice with a mapping gap.

The probe derives regions from `CTaskForce+0x18` plus
`extractor/memory_reader/weather/prov2region_mod.json` with 13413 entries. One live task
force had `location_province_id = 0`, so region rows must be badged/omitted for
unmapped or zero-location task forces.

## Master row change proposals

Do not edit `MASTER_EXTRACTION_CONTRACT.md` directly from this lane. Proposed
orchestrator-owned row updates:

| Row | Proposed status | Proposed evidence / domain | Proposed remaining closure condition |
| --- | --- | --- | --- |
| Home base / repair target | `PROVEN` | `CFleet+0xD8` and `CTaskForce+0x4B0`, both joined through `CProvince+0xA4` | Preserve nullable repair-target absence. |
| Detached activity | `PROVEN` | exact-ten formatter/subobject/body join plus persisted `detached_activity` serializer | Emit `mission_state` 0..4; fail closed above 4. |
| Repair progress / retreat | `MECHANICALLY REJECTED/ABSENT` | serializer body `+0x4B8/+0x4C0` are nullable object pointers; save `retreat=yes` and D50 member states are separate carriers | No scalar inference or cross-carrier alias. |
| Convoy combat/losses | `OWNED BY D90` | D56 retains only aggregate `CShip+0x3C`; D90 owns the full client EntityHandle and typed route endpoints | No D56 closure condition; never infer a loss route from movement path, current location, or mission. |

## Proof commands

Implementation-lane and authority validation runs through the caller's
worktree-local `extractor/native/scripts/Invoke-CentralRustValidation.ps1`.
Agents do not invoke Cargo or `rustc` directly. The historical live probe
remains evidence reproduction tooling, not a production-schema authority.

## Required report fields

- Completion status: `PRODUCTION-READY EXACT D56 SLICE`
- Backlog reduced: yes
- Exact backlog item removed or narrowed: the domain-56 compact schema/gate is
  implemented; mission/home-base/repair-target/detached-state semantics are
  exact; unproven repair-progress, retreat-state, and route-context positions
  are retired; D90 exclusively owns convoy route/loss attribution.
- Exact files changed: lane-local files in
  `extractor/native/lanes/naval_missions_status_live/`.
- Exact roots/offsets/functions validated or rejected: listed above.
- Compact schema/record changes: the 44-byte header and 64-byte physical row
  remain stable; retired fixture, repair-progress, retreat-state, and
  route-context positions fail closed.
- Rust decoder/gate changes: `src/domain56_compact.rs`.
- Proof commands/results: listed above.
- DLL integration: active in the shared collector; domain 56 uses the existing
  coordinator, root cache, scheduler, and sequence-owned compact writer.
- Compact stream: schema/gate plus shared domain-56 emission are wired.
- Rust decoded: yes; recorder authority accounting and the world fold consume the
  lane-owned payload fail-closed.
- `.battleplan`: the shared raw-stream/container and fold path are wired; this lane
  does not retain an authenticated live replay receipt of its own.
- Integrated performance/freshness runs are validation evidence, not remaining
  reverse engineering, and run through the central final battery.
- Human RE input required by D56: none.
