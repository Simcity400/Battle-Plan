# naval_oob_fleets_ships_live
> Active lane note: this README is the only active lane-local documentation. Historical plans, proof reports, blocker notes, and registration packets for this lane are archived under ../../../documents/archive/native_lanes/naval_oob_fleets_ships_live/. Current extraction completion status lives in ../../../documents/MASTER_EXTRACTION_CONTRACT.md.


Lane-local proof stack for naval OOB, inventory, and continuity compact domains.
This lane owns fleet/task-force/ship OOB state. It does not own active naval
combat lifecycle/member work.

## Current production roots

Current shared collector code for the verified 870ebf proof uses a DLL-native
root path:

- `CGameState+0x310` country table, count at `CGameState+0x31C`;
- each valid `CCountry` has vtable RVA `0x27D0E90` and tag-db id at `+0x08`;
- `CCountry+0x278/+0x280/+0x284` is the fleet pointer vector;
- `CFleet` vtable RVA `0x294BF28`, handle `{type=61,id}` at `+0x08`;
- task-force roots are validated from bounded fleet scans and vectors, including
  subobject vtable RVA `0x294C480` normalized by deltas `[-0x780,0x10,0x7A0,0xF30]`;
- `CTaskForce` vtable RVA `0x294C660`, handle `{type=61,id}` at `+0x08`;
- `CTaskForce+0x330` is parent fleet, `+0x18` is current province/location,
  `+0x338/+0x344` is the direct `CShip*` member vector;
- `CShip` vtable RVA `0x2941748`, handle `{type=51,instance_id}` at `+0x08`,
  sunk convoys `+0x3C`, definition string `+0x98`, category enum `+0xE0`,
  strength `+0x6F8`, organisation `+0x700`, experience `+0x708`,
  `ref100` sentinel `+0x710 == 10000000`.

These roots fail closed on vtable, handle type/id, pointer shape, vector bounds,
duplicate ship ids, and CShip sentinel validation.

## Compact records

- Domain 28 `oob_command`: fleet rows, task-force rows, parent fleet, owner,
  task-force location, member task-force ids, member ship ids, engine/logical
  member counts, delete rows.
- Domain 29 `ships`: ship instance id, owner, task-force id, definition/class,
  category enum, strength, organisation, experience, sunk convoy count, delete
  rows.

Ship names are not yet in the compact `ships` production row. Persistent CShip
name was not located in prior OOB work; combat-only `CFEXMember+0x48` names
belong to the naval combat lane, not this OOB stream.

## Lane-local gate

`domain28_29_compact.rs` is a standalone Rust decoder/gate for the compact stream
wire contract used by domains 28 and 29. It validates the `H4CR` record stream,
record ordering, payload lengths, row flags, counters, member ranges, fixed-point
scale, owner/task-force ids, definition strings, and failure status bits.

Proof commands:

```powershell
rustc --edition=2021 --test extractor/native/lanes/naval_oob_fleets_ships_live/domain28_29_compact.rs -o extractor/native/lanes/naval_oob_fleets_ships_live/domain28_29_compact_tests.exe
extractor/native/lanes/naval_oob_fleets_ships_live/domain28_29_compact_tests.exe
rustc --edition=2021 extractor/native/lanes/naval_oob_fleets_ships_live/domain28_29_compact.rs -o extractor/native/lanes/naval_oob_fleets_ships_live/naval_oob_gate.exe
extractor/native/lanes/naval_oob_fleets_ships_live/naval_oob_gate.exe extractor/native/runs/compact_full_plus_naval_oob_870ebf_20260701_v1/collector_stream.bin
```

Current verified live proof source:

- `extractor/native/runs/compact_full_plus_naval_oob_870ebf_20260701_v1/collector_stream.bin`
- `extractor/native/runs/compact_full_plus_naval_oob_870ebf_20260701_v1/native_session.battleplan`
- `extractor/native/runs/compact_full_plus_naval_oob_870ebf_20260701_v1/native_session_manifest.json`

Lane-local gate outputs:

- `extractor/native/runs/naval_oob_fleets_ships_live_870ebf_20260701_lane_gate_v1/naval_oob_gate_summary.json`
  validates the requested seed run: 54 game hours, 54 domain-28 records, 54
  domain-29 records, 124 fleet rows, 195 task-force rows, 1,625 task-force
  member ship ids, 1,732 ship definition rows, no parse/order/failure flags.
- `extractor/native/runs/naval_oob_fleets_ships_live_870ebf_20260701_lane_gate_v1/ship_owner_gate_summary.json`
  validates the later ship-owner proof: 54 game hours, 2,080 owner-attributed
  ship rows, no parse/order/failure flags.
- `extractor/native/runs/naval_oob_fleets_ships_live_870ebf_20260701_lane_gate_v1/tf_location_gate_summary.json`
  validates the later task-force-location proof: 53 game hours, 203 task-force
  location rows, 2,462 owner-attributed ship rows, no parse/order/failure flags.

## Integration plan

No shared entrypoint change is needed for this lane-local gate. The current
integration point is the existing shared compact production path for scheduled
domains `oob_command` and `ships`. Next production additions should be additive
compact row fields or a new typed naval OOB domain for fields not currently in
the row: persistent ship names, naval leader/admiral linkage, missions, home
base/port linkage, and theatre/region rollups.

## Remaining-gap probe

`naval_oob_gap_probe.py` is a lane-local read-only live probe for the remaining
naval OOB fields. It mirrors the current compact native roots above and scans
only bounded `CFleet`, `CTaskForce`, and `CShip` bodies or one bounded CShip
indirect pointer layer for candidate evidence.

Current proof output:

- `extractor/native/runs/naval_oob_fleets_ships_live_870ebf_20260701_gap_probe_v1/naval_oob_gap_probe_summary.json`

Live proof summary from that output:

- roots validated: `module+0x3316E90 -> CGameState`, `CGameState+0x310`
  country table, `CCountry`, `CFleet`, `CTaskForce`, `CShip`;
- observed rows: 433 countries, 159 fleets, 184 task forces, 1,443 ships;
- persistent ship names: rejected on current roots. `CShip[0x40..0x800]`
  direct string scan and one bounded indirect layer found no plausible
  persistent-name candidate. `CShip+0x98` and `CShip+0xE8` resolved definition
  tokens only; combat-member offsets `+0x48/+0x68/+0x88` read 0/128 on CShip;
- missions: blocked by executable fixture contract. Single-snapshot small-u32
  candidates are emitted, but mission enum semantics need a before/after task
  force mission change;
- home base / port: blocked by executable fixture contract. `CTaskForce+0x18`
  remains the validated current location; candidate province pointers/ids
  exist, including task-force pointer offset `+0x1E0`, but home-base semantics
  need a controlled port-change fixture;
- naval leaders: blocked by executable fixture contract, with direct candidates
  narrowed. `CNavyLeader` vtable
  `module+0x293F650`, type `4713`, id `+0x0C`, skill
  `leader+0xE60 -> +0x1B8`, and trait size/cap `+0xDD4/+0xDD0` validate on
  `CFleet` candidates. Top observed fleet offsets include `+0x14A8` (5 hits),
  `+0x1728` (4 hits), `+0x6E8` (4 hits), `+0x968` (4 hits). `CTaskForce`
  direct leader scan had no hits. Assigned-admiral semantics need an
  assignment/removal fixture before a production offset is promoted;
- strategic-region rollups: validated as derived data. All 184 task forces map
  from validated `CTaskForce+0x18` province through
  `memory_reader/weather/prov2region_mod.json` (13,413 entries), with no
  missing task-force region rows;
- theatre linkage: blocked by fixture contract. Direct `CTheatre` pointer
  candidates exist in `CFleet[0x0..0x1800]` at
  `+0x1288/+0x12B0/+0x13C8/+0x13F0` for two fleets, vtable
  `module+0x295EE18`, type `67`; no `CTaskForce` theatre candidates. Semantics
  need UI/save theatre assignment evidence before promotion.

`gap_extension_compact.rs` is a standalone provisional compact schema and
decoder/gate for the additive naval OOB extension domain. It explicitly supports
badged/rejected/fixture-required states instead of inventing values for fields
that are not yet production-proven.

Proof commands:

```powershell
python -m py_compile extractor/native/lanes/naval_oob_fleets_ships_live/naval_oob_gap_probe.py
rustc --edition=2021 --test extractor/native/lanes/naval_oob_fleets_ships_live/gap_extension_compact.rs -o extractor/native/lanes/naval_oob_fleets_ships_live/gap_extension_compact_tests.exe
extractor/native/lanes/naval_oob_fleets_ships_live/gap_extension_compact_tests.exe
python extractor/native/lanes/naval_oob_fleets_ships_live/naval_oob_gap_probe.py --out extractor/native/runs/naval_oob_fleets_ships_live_870ebf_20260701_gap_probe_v1/naval_oob_gap_probe_summary.json --snapshot-label gap_probe_v1
```

## Residual integration plan

1. Promote naval leader support into the existing domain-28 OOB rows only after
   selecting the correct `CFleet` leader offset from the validated candidate set
   with an admiral assignment fixture. The row already has `leader_id`,
   `leader_skill`, and `leader_trait_count` fields.
2. Add a new compact naval OOB extension domain using
   `gap_extension_compact.rs` for task-force mission, home base/port, theatre,
   derived strategic-region buckets, and persistent ship-name status.
3. Keep region rollups derived from `CTaskForce+0x18` plus the province-region
   map. Fail closed if a province has no map entry.
4. Keep persistent ship names badged/rejected for current roots until a separate
   identity registry or UI/save instance-id fixture validates a production path.
5. Do not source persistent OOB names from `CFEXMember+0x48`; that remains
   combat-member-only and belongs to `naval_combat_live`.
