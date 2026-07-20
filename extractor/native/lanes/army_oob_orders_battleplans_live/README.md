# army_oob_orders_battleplans_live lane
> Active lane note: this README is the only active lane-local documentation. Historical plans, proof reports, blocker notes, and registration packets for this lane are archived under ../../../documents/archive/native_lanes/army_oob_orders_battleplans_live/. Current extraction completion status lives in ../../../documents/MASTER_EXTRACTION_CONTRACT.md.

> 2026-07-19 residual RE correction: Type 5 is area defense, exact
> `+0x70/+0x78` geometry is admitted only for authenticated type-2 line/type-3
> invasion rows, and execution/activation is exact at `order+0x248`. See the
> integrated domain-54 source and
> `../army_battleplans_depth_live/A729_BATTLEPLAN_RE_REPORT.md`.

> 2026-07-20 manager/scalar closeout: exact registered-reference table code
> closes the complete army manager, and exact command writers close the u32
> execution/cohesion/leader-proximity scalars at `+0x1A4/+0x1A8/+0x1AC`.
> The fail-closed reader and pointer-free `OGS1` carrier live in
> `../army_battleplans_depth_live/src/orders_group_live.rs`; receipts and
> supersession details are in `ORDERS_MANAGER_SCALARS_RE_20260720.md`.
> Where older sections below say `CArmy` for RVA `0x293BC60`, read that as the
> army-row `COrdersGroup`; exact MSVC RTTI is `.?AVCOrdersGroup@@`.


Lane scope: land army/army-group command hierarchy, commander assignment,
formation membership, selected army order roots, battle-plan target slices, and
OOB/order events needed for replay. Division scalar and movement fields remain
owned by the division lane; this lane consumes division identity roots only as a
join/source boundary.

Registry binding: depth work uses `army_battleplans_depth_live` (`domain_id =
54`). Existing base domains `army_orders_live` (`domain_id = 27`) and
`oob_command_live` (`domain_id = 28`) stay unchanged. Older lane-local
`direct_land_oob` domain-44 slot material is canceled by the compact domain
registry and is not a production integration path.

## Current production slice

The current production-shaped slice is the compact DLL/Rust path for active
armies reachable from live divisions:

- compact domain `army_orders` (`domain_id = 27`);
- compact domain `oob_command` (`domain_id = 28`);
- one DLL collector/coordinator, compact binary stream, Rust recorder decode,
  and `.battleplan` packing;
- no Python/save hot-loop dependency;
- division roots are used only to derive active `CArmy` parents and OOB joins.

Fresh focused live proof:

- run: `extractor/native/runs/army_oob_orders_battleplans_live_870ebf_20260701_v1`;
- pid: `45880`;
- duration: 20 seconds, `sample_ms = 0`;
- scheduled domains: `army_orders`, `oob_command`;
- advanced: 24 game hours, 25 unique frame hours;
- skipped hours: 0;
- compact stream: ordered and decodable;
- `.battleplan`: `native_session.battleplan`, independently accepted by
  `pack_replay verify`;
- failures: no source-unavailable, partial-data, bridge-unavailable, or worker
  panic flags.

Direct-root/theatre proof:

- run: `extractor/native/runs/army_oob_orders_battleplans_live_direct_oob_870ebf_20260701_v1`;
- pid: `45880`;
- proof module: `direct_oob_probe.py`;
- gate: `direct_oob_proof_gate.py`;
- result: direct vtable root scan found 660 `CArmy`, 470 army-group,
  97 theatre, and 3051 division roots with zero rejected vtable hits;
- active-army comparison: 652 armies reached through live `CDivision+0xB0`
  parents, leaving 8 direct empty/inactive army roots that the active-army
  production slice cannot see;
- order mix on direct armies: 630 order rows, including 166 exact type-5
  area-defense target rows, 423 type-2 line rows, and 41 type-3 invasion rows;
- production position: validated current-build discovery evidence and a
  cold-preflight/cache-refresh candidate, not yet a hot-loop DLL production root.

## Validated current-build roots and offsets

Current executable/profile in the existing proof family:
`870EBF1F615262FACCA48A3B947775C41F0A5338F3068C40958209F5C41C65AD`.

Production root dependency, as implemented in `extractor/native/collector/src/lib.rs`:

- validated collector game-state pointer, then `CGameState+0x310` country table;
- country count bounded by `MAX_CGAMESTATE_COUNTRIES = 2048`;
- `CCountry` vtable `MODULE+0x27D0E90`;
- country division-wrapper vector:
  - data `CCountry+0x290`;
  - capacity `CCountry+0x298`;
  - count `CCountry+0x29C`;
- division wrapper vtable `MODULE+0x2943AD0`;
- wrapper-to-body delta `+0x10`;
- `CDivision` body vtable `MODULE+0x2943CB0`;
- division type accepted as `4713` or alternate type `51`;
- division id read from the validated body and used as the join key;
- `CDivision+0xB0` supplies the parent `CArmy*` for active-army discovery.

Army order roots:

- `CArmy` vtable `MODULE+0x293BC60`;
- army order vector at `CArmy+0x98`;
- order instance vtable `MODULE+0x296FAE8`;
- order type at `order+0x30`;
- army back-reference at `order+0x38`;
- type-5 area-defense target-state vector:
  - begin `order+0xE0`;
  - size `order+0xE8`;
- max 64 orders per army;
- max 5000 target states, with compact inline target rows capped by the encoder;
- max state id `65535`;
- target geometry is exact only when `target_status = exact`. Unsupported,
  overflow, and validation-failed rows are explicitly badged.

Land OOB command roots:

- active army roots are derived from the same `CDivision+0xB0` parents;
- army-group vtable `MODULE+0x293BCF8`;
- army group parent pointer from `CArmy+0x1B8`;
- army member engine count from `CArmy+0x5C`;
- leader pointer from `CArmy+0x88`;
- leader id from leader `+0x0C`;
- leader skill/trait fields are emitted only when validated by the collector;
- owner is derived from member-division owner consensus and emitted as tag-db id.

Direct land-OOB candidate roots:

- `CArmy` vtable `MODULE+0x293BC60`;
- army-group vtable `MODULE+0x293BCF8`;
- theatre vtable `MODULE+0x295EE18`;
- `CDivision` body vtable `MODULE+0x2943CB0`;
- army and army-group handles use type `53`;
- theatre handles use type `67`;
- direct root ids were accepted only after vtable, type, id, alignment, and
  sanity-bound validation;
- current proof rejected no candidate hits for army, army group, theatre, or
  division roots.

The older Python readers (`memory_reader/construction_orders_live.py` and
`memory_reader/oob_command_live.py`) remain useful discovery lineage for the
same shapes, but their hardcoded `16022851` offsets are not production-current.
The live proof above used the Rust collector's current `870EBF` profile.

## Compact records

`army_orders` payload:

- 40-byte domain header:
  - elapsed wall ms;
  - row count;
  - changed rows;
  - status flags;
  - division root rows;
  - observed army rows;
  - observed order rows;
  - target-state child rows;
  - deleted rows.
- 48-byte order rows keyed by `(army_id, generation, slot_index)`:
  - deleted flag;
  - order type;
  - optional target-state child range;
  - optional declared target-state count;
  - target status code.
- 8-byte target-state child rows:
  - state id;
  - reserved zero.

`oob_command` payload:

- 48-byte domain header:
  - elapsed wall ms;
  - row count;
  - changed rows;
  - status flags;
  - division root rows;
  - observed army rows;
  - observed army-group rows;
  - member child rows;
  - deleted rows;
  - observed fleet rows;
  - observed task-force rows.
- 80-byte OOB rows keyed by `(kind, object_id)`:
  - kind `1 = army`, `2 = army_group`, `3 = fleet`, `4 = task_force`;
  - deleted flag;
  - optional parent id;
  - optional owner tag-db id;
  - optional leader id and skill;
  - optional member child range;
  - engine member count and missing member count when measured;
  - optional location province for task-force rows;
  - status codes for parent/member/owner/leader/leader traits.
- 8-byte member child rows:
  - member object id;
  - reserved zero.

`compact_schema.rs` is the lane-local decoder/gate mirror for these two domains.
It intentionally has no dependency on shared recorder code so schema review can
happen without editing shared entrypoints. Its H4CR framing comes from the
dependency-free canonical module through
`#[path = "../../compact_contract/src/record_header.rs"]`; this keeps direct
`rustc` use standalone while preventing the record-header contract from drifting.

From the repository root, compile and invoke the gate directly with:

```powershell
rustc --edition=2021 .\extractor\native\lanes\army_oob_orders_battleplans_live\compact_schema.rs -o "$env:TEMP\compact_schema.exe"
& "$env:TEMP\compact_schema.exe" .\extractor\native\runs\<run>\collector_stream.bin 5
```

The older `direct_oob_compact.rs`, `direct_land_oob_slot_contract.json`, and
`direct_land_oob_slot_gate.py` domain-44 slot material is retained only as
historical lane evidence. It is not a valid production registration path because
domain 44 is assigned elsewhere. Current direct land-OOB roots are represented
as an additive direct-root section inside domain 54.

`battleplan_fixture_gate.py` is the acceptance/rejection gate for remaining
battleplan gaps. Exact rows must carry validated offsets and concrete field
values. Rejected candidates must carry candidate, reason, and evidence list, and
only pass with `--allow-rejection`.
`battleplan_fixture_gate_tests.py` covers exact acceptance, rejection gating,
passive-candidate rejection, and cached plan-value rejection without live
offsets.
`battleplan_integration_guard.py` blocks bad future integration proposals:
existing `army_orders`/`oob_command` row contracts must stay unchanged, geometry
and execution must be additive, and passive scans or save-only/cache-only fields
cannot be used as integration evidence.
`battleplan_fixture_observer.py` is the executable before/after harness for a
future redraw/toggle fixture. It converts changed observations into exact rows
and unchanged observations into exact rejection rows, while rejecting passive
scan sources.

`d54_exact_fixture_observer.py` supersedes that historical v1 observation
shape for the four narrowed a729 D54 fixtures. Its contract is
`D54_EXACT_FIXTURE_CARRIER_CONTRACT.json`. It carries the current 60-byte header
/ 88-byte row receipt, the raw 88-byte row, raw order pointer and bytes at
`+0x30`, `+0x38`, `+0x70/+0x78`, `+0x210/+0x21C`, `+0x244`, and `+0x248`, and
same-paused save receipts. Boundary arrays, paths, member arrays, and manager
walk entries remain in source order. The resulting carrier always sets every
activation field to `false`; even mechanically complete observations require a
separate reviewed promotion edit. The old observer's `+0x150` execution example
is retained as historical rejected-candidate coverage and is not an open D54
fixture.

`army_battleplans_depth_collector.rs`,
`army_battleplans_depth_compact.rs`, `army_battleplans_depth_module.rs`, and
`army_battleplans_depth_from_stream.py` are retained legacy 52-byte-header /
64-byte-row proof and corpus-mining tools. They are not the current production
wire authority. The current authority is
`../army_battleplans_depth_live/src/lib.rs`: a 60-byte header, 88-byte rows,
exact `+0x244` order identity, exact `+0x210/+0x21C` assigned-division child
IDs, exact `+0x248` active/execution state, authenticated type-2/type-3 path
children, and type-5 area-defense state children. Type 4 remains fail-closed.

The retained legacy stack covers the previously validated subset as follows:

`army_battleplans_depth_compact.rs` uses the historical canonical
dependency-free record-header include. Its old direct proof invocation was:

```powershell
rustc --edition=2021 .\extractor\native\lanes\army_oob_orders_battleplans_live\army_battleplans_depth_compact.rs -o "$env:TEMP\army_battleplans_depth_compact.exe"
& "$env:TEMP\army_battleplans_depth_compact.exe" .\extractor\native\runs\<run>\depth_stream.bin 5 --json
```

- exact type-5 area-defense target-state rows are emitted as exact depth rows;
- assigned-unit and order-linkage status are carried on every non-deleted
  domain-54 row. The lane-local converter derives assigned-unit status from the
  paired `oob_command` row for the same frame hour and fails the depth gate if a
  live row lacks exact assignment or linkage status;
- type-2 line and type-3 invasion rows with a validated live
  `order+0x70/+0x78` path vector are emitted as exact path geometry rows with an
  8-byte path-province child table; all other types are fail-closed and do not
  dereference that vector;
- active/execution state is exact from `order+0x248`;
- stance and aggressiveness are emitted as explicit unknown statuses until a
  live current-build offset is validated or rejected;
- direct `CArmy`, army-group, and theatre roots can be carried in the legacy
  domain-54 direct-root section with 64-byte rows and 8-byte member child rows.
  Those proof rows are explicitly `STATUS_COLD_PREFLIGHT_ONLY` because the direct
  root enumerator is validated discovery/preflight evidence, not yet the
  DLL-only hot-loop production source;
- non-exact type-5 area-defense rows are omitted fail-closed rather than guessed.
- the historical production-module boundary accepts live order observations and emits a
  domain-54 payload directly to a compact sink. It also accepts prepared direct
  root rows for a future bounded preflight/cache-refresh hook and no longer
  depends on a domain-27 proof stream conversion for the future DLL hook.

Proof run:
`extractor/native/runs/army_battleplans_depth_live_870ebf_20260701_v1`.

Replay/package proof run:
`extractor/native/runs/army_battleplans_depth_live_module_replay_870ebf_20260701_v2`.
It wraps the exact domain-54 compact bytes in a `.battleplan`, runs
`pack_replay verify`, and checks the replay manifest entry hash against the
source compact stream.

Assignment/linkage proof run:
`extractor/native/runs/army_battleplans_depth_live_assignment_status_870ebf_20260701_v3`.
It rebuilds the domain-54 stream from the same live DLL compact proof, joins
domain 27 order rows to same-hour domain 28 OOB rows, and gates:

The execution and order-label counts below describe that historical pre-RE
artifact only; they are not the current a729 semantic contract.

- 25 depth data records over 25 frame hours;
- 24 game hours advanced with zero skipped hours;
- 564 domain-54 rows, including 552 non-deleted rows;
- 132 exact type-5 target rows and 550 target-state child rows;
- 552 exact assigned-unit status rows;
- 552 exact order-linkage status rows;
- 420 geometry fixture-required rows in the historical artifact;
- 552 execution fixture-required rows in the historical artifact;
- 552 stance unknown rows;
- 552 aggressiveness unknown rows;
- `.battleplan` byte-identity verification and compact-entry hash check.

Path-geometry compact proof run:
`extractor/native/runs/army_battleplans_depth_live_path_geometry_compact_870ebf_20260701_v1`.
It reruns the save/live geometry proof with full compact-row retention, then
materializes the exact type-2/type-3 `order+0x70/+0x78` rows into domain-54
compact bytes:

- save/live geometry gate passed:
  `VALIDATED_PATH70_AND_REJECTED_TYPE3_C0_GEOMETRY`;
- type-5 pairing control: 137/171 exact rows at `order+0xE0/+0xE8`;
- path candidate: 95/111 exact type-2/type-3 rows at `order+0x70/+0x78`;
- rejected candidate: 0/17 exact type-3 rows at `order+0xC0/+0xC8`;
- compact stream: 95 domain-54 rows and 324 path-province child rows;
- Rust gate: ordered, decodable, hourly-or-lower, one frame and one depth record
  for the materialized proof hour, zero skipped hours, and failure status flags
  `0`;
- `.battleplan` byte-identity verification and compact-entry hash check passed
  for stream SHA-256
  `5e800a3e16dde18a889661a656a80d87986ba6aa40905dc5bff401e6a3003d0c`.

Direct-root cold-preflight proof run:
`extractor/native/runs/army_battleplans_depth_live_direct_roots_870ebf_20260701_v4`.
It rebuilds the domain-54 stream from the same live DLL compact proof, joins the
v3 assignment/linkage data, and appends direct-root rows from the validated
current-build direct-OOB proof:

- 25 depth data records over 25 frame hours;
- 24 game hours advanced with zero skipped hours;
- 564 domain-54 order/depth rows, including 552 non-deleted rows;
- 1227 unique direct root rows per frame: 660 armies, 470 army groups, and
  97 theatres;
- 30675 direct root rows and 4150 direct member child rows across the proof
  stream;
- all direct-root rows are badged `STATUS_COLD_PREFLIGHT_ONLY`;
- Rust gate: ordered, decodable, hourly-or-lower, one frame and one depth record
  per hour, zero skipped hours, and failure status flags `0`;
- `.battleplan` byte-identity verification and compact-entry hash check passed
  for stream SHA-256
  `73310c148d0a47f3e845c95f68a6f0dab56a845cdd5d394ba30512b64e04836f`.

Direct-root warm-cache rejection proof run:
`extractor/native/runs/army_battleplans_depth_live_direct_warm_cache_870ebf_20260701_v5`.
It tested a pure preflight-address cache as a candidate path for direct
army/group/theatre hot-loop capture:

- preflight scan found 477 direct armies, 259 army groups, 100 theatres, and
  2877 divisions with zero rejected vtable hits;
- preflight proved 12 empty/inactive direct armies beyond the 465 active armies
  reachable through division parents;
- five warm iterations performed zero heap sweeps and used only cached-address
  vtable/handle/id liveness checks;
- every warm iteration rejected 20 cached roots: 12 army vtable mismatches,
  3 army-group vtable mismatches, 4 army-group id mismatches, and 1 division
  vtable mismatch;
- direct armies dropped from 477 to 465 and army groups from 259 to 252 in the
  warm reads; empty/inactive direct-army coverage dropped from 12 to 0 for the
  first four warm iterations;
- strict gate failed, and the rejection gate passed with
  `candidate_status = REJECTED_PURE_PREFLIGHT_CACHE`.

Do not integrate direct roots by performing one slow preflight scan and then
trusting cached root addresses across scheduled frames. A production path needs
a current-build root manager, a bounded refresh-on-mismatch contract, or
fail-closed omission/badging when direct roots go stale.

Direct-root targeted refresh rejection proof run:
`extractor/native/runs/army_battleplans_depth_live_direct_targeted_refresh_870ebf_20260701_v3`.
It tested a cached-root plus targeted handle-scan refresh-on-mismatch candidate
with a current-build synthetic stale fixture:

- preflight scan found 513 direct armies, 274 army groups, 101 theatres, and
  3063 divisions in the current live state;
- the current live state had no empty/inactive direct armies, so this proof does
  not close empty-army production coverage;
- the fixture deliberately invalidated one cached direct root per kind
  (`army`, `army_group`, `theatre`) by changing the cached address after
  validated current-build discovery;
- the targeted refresh recovered all 3 stale direct roots by scanning for their
  `(type,id)` handle values and revalidating vtable/type/id;
- recovery required reading 9,303,945,216 bytes in 13.094 seconds for a
  three-row mismatch, exceeding the 512 MiB bounded hot-loop threshold;
- the gate passes with
  `candidate_status = REJECTED_TARGETED_HANDLE_REFRESH_ON_MISMATCH`.

Do not integrate direct roots by repairing stale cached rows with an arena-wide
targeted handle scan. This narrows the remaining refresh-on-mismatch option to a
true bounded DLL-native manager/container/helper path; otherwise direct roots
must stay fail-closed/badged.

Direct-root table candidate proof run:
`extractor/native/runs/army_battleplans_depth_live_direct_table_870ebf_20260701_v2`.
It searched the current-build heap for qword reference runs to validated direct
army, army-group, and theatre roots after the same direct-OOB vtable preflight:

- preflight scan found 487 direct armies, 255 army groups, 100 theatres, and
  2956 divisions with 8 empty/inactive direct armies beyond the 479 active
  armies reachable through division parents;
- the qword-reference scan read 9,268,875,264 bytes and found 22,712 direct-root
  references with no hit-cap overflow and no membership false positives;
- every direct root is referenced somewhere in memory (`487/487` armies,
  `255/255` army groups, `100/100` theatres), but most references are not a
  bounded manager/vector table;
- the gate passes with
  `candidate_status = VALIDATED_PARTIAL_DIRECT_ROOT_POINTER_TABLE`;
- the accepted table-shaped candidate is a stride-8 theatre vector at
  `0x22D05419560`, length `73`, covering `73/100` current-build theatre roots;
- best table-shaped army coverage is `45/487` (`9.24%`) and best army-group
  coverage is `18/255` (`7.06%`), so army and army-group roots still need a
  real manager container, bounded refresh-on-mismatch, or fail-closed badging.

Direct-root table rejection proof run:
`extractor/native/runs/army_battleplans_depth_live_direct_table_870ebf_20260701_v3`.
It extends the table probe with explicit active and empty/inactive army coverage
and rejects the complete plain contiguous/fixed-stride pointer-table path:

- preflight scan found 477 direct armies, 242 army groups, 101 theatres, and
  2953 divisions;
- 13 direct armies were empty/inactive relative to the 464 active armies reached
  through division parents;
- normal gate passes with
  `candidate_status = REJECTED_PLAIN_DIRECT_ROOT_POINTER_TABLE`;
- strict empty-army table-resolution gate fails because empty/inactive armies
  are not fully covered by accepted table runs;
- all 13 empty/inactive armies had at least one memory reference, but only
  11/13 appeared in any stride 8/16/24/32 table-shaped run and 0/13 appeared in
  an accepted table run;
- best single-run coverage stayed below production thresholds: army `46/477`
  (`9.64%`), army group `18/242` (`7.44%`), theatre `49/101` (`48.51%`);
- union coverage across all table-shaped runs was high but incomplete
  (`471/477` armies, `238/242` army groups, `91/101` theatres), so this proves
  references exist but does not provide a bounded root manager/table source.

`../../../documents/archive/native_lanes/army_oob_orders_battleplans_live/ARMY_BATTLEPLANS_DEPTH_REGISTRATION_PLAN.md` is the exact registration-only
hook plan for the collector registry/coordinator and Rust recorder. It is not
applied while another lane owns the shared integration slot.

Current candidate probe:
`extractor/native/runs/army_battleplans_depth_live_candidate_probe_870ebf_20260701_v2`.
`battleplan_candidate_probe.py` validated 350 current-build order objects,
passed the type-5 `order+0xE0/+0xE8` positive control, rejected `order+0x150`
as `is_executing: u8`, and narrowed `order+0x70/+0x78` plus
`order+0xC0/+0xC8` to redraw-fixture-required geometry candidates.
`battleplan_candidate_probe_gate.py` locks that proof shape so failed positive
controls or passive vector-shaped rows cannot be cited as production roots.

Save/live geometry proof run:
`extractor/native/runs/army_battleplans_depth_live_save_geometry_870ebf_20260701_v1`.
It parses the melted autosave's `orders_group.order_instance` rows, pairs them
to live order-vector slots only when army id, order count, and order type
sequence match, and uses exact type-5 state vectors as the pairing control:

- 448 save armies with orders and 449 live armies with orders;
- 281 armies had the same order count and type sequence, producing 282 paired
  order rows;
- type-5 positive control passed: 137/171 paired type-5 rows exactly matched
  save `states` at live `order+0xE0/+0xE8`;
- validated geometry slice: 95/111 paired type-2/type-3 rows exactly matched save
  `path` at live `order+0x70/+0x78`;
- rejected geometry candidate: 0/17 paired type-3 rows matched save `path` at
  live `order+0xC0/+0xC8`, so the old type-3 secondary vector candidate should
  not be retried as the primary path geometry vector;
- gate passed with
  `candidate_status = VALIDATED_PATH70_AND_REJECTED_TYPE3_C0_GEOMETRY`.

This validates the current-build type-2/type-3 path-vector offset for exact
save/live paired rows. It does not close the master `Battleplan geometry` row:
domain 54 now has a lane-local compact path-geometry child table and proof, but
it still needs shared end-to-end proof and an authenticated same-instance
type-2 redraw/delete fixture before the master row can close.

Historical army-level scalar candidate proof run:
`extractor/native/runs/army_battleplans_depth_live_army_scalar_870ebf_20260701_v2`.
It used the current autosave's `orders_group` scalar fields as a candidate
oracle, then scanned validated live `CArmy` roots over `+0x00..+0x300` as
`u8/u32`:

- matched 463 save armies to 498 live direct army roots;
- near-match gate passed with
  `candidate_status = NARROWED_ARMY_SCALAR_NEAR_MATCH`;
- strict gate failed because all three fields are near-match only, not exact;
- narrowed candidates:
  - `execution_type`: `CArmy+0x1A4`, `447/463` matches (`96.54%`);
  - `cohesion_type`: `CArmy+0x1A8`, `462/463` matches (`99.78%`);
  - `proximity_type`: `CArmy+0x1AC`, `455/463` matches (`98.27%`);
- `u8` and `u32` reads at the same offsets both matched the low-value
  histograms, so this older artifact alone did not resolve width. It is now

## Remaining backlog

Not completed by this lane-local slice:

- DLL-native hot-loop or bounded preflight integration of theatres and land
  command roots not emitted by the active-army compact slice. The same domain-54
  direct-root section now carries theatre ids as cold-preflight rows. The v2
  table probe validates a partial theatre vector covering `73/100` theatre
  roots, but v3 shows that plain table coverage is not stable/complete
  (`49/101` best theatre run in the later frame);
- line/invasion geometry beyond the exact type-2/type-3 path slice, plus type-4
  paradrop geometry. `order+0x70/+0x78` is validated for type 2 and type 3 and
  has a lane-local domain-54 compact child-table proof. `order+0xC0/+0xC8` is
  rejected as the type-3 primary path vector. Redraw/delete lifecycle and type-4
  promotion still require the minimal controlled discriminator fixture;
- plan-value/power runtime caches. Active/execution state is exact at
  `order+0x248`; `order+0x150` is a rejected execution candidate for current
  build `870ebf` and must not be retried;
- order-row stance/aggressiveness live fields remain separate and unknown.
  They must not be inferred from the now-exact army-level execution, cohesion,
  or leader-proximity values;
- UI-level semantic reconstruction of front lines from order geometry.

## Production integration

The shared integration slot was granted on 2026-07-20. The production path now:

1. Keep `domain_id = 27` and `domain_id = 28` stable.
2. Keeps `domain_id = 54` schedulable as
   `army_battleplans_depth_live` and calls its lane-owned codec.
3. Promote any new battleplan row fields as additive flags/reserved-field
   expansions, never by changing existing row widths without a version bump.
4. Adapts collector memory reads to the exact type-53 source in
   `../army_battleplans_depth_live/src/orders_group_live.rs`; append its sorted,
   pointer-free `OGS1` carrier and fail the domain closed if the source rejects
   a snapshot.
5. Add theatre rows as direct-root `kind = 5` inside domain 54 only after the
   shared recorder slot is granted; until then, keep the proof as lane-local
   evidence and do not mutate `oob_command` row widths.
6. Keep type-4 geometry and same-instance redraw/delete admission behind the
   activation-false carrier in `D54_EXACT_FIXTURE_CARRIER_CONTRACT.json`.
   `d54_exact_fixture_observer.py` requires the current 60/88 D54 wire and raw
   identity/order receipts. Type-4 promotion additionally requires its raw
   geometry candidates and a same-paused save order; it does not preselect a
   geometry vector. The current save/live proof already validates
   `order+0x70/+0x78` for exact type-2/type-3 rows and rejects
   `order+0xC0/+0xC8` as the type-3 primary path vector. Active/execution state
   is already exact at `order+0x248` and is not part of the remaining fixture.
7. Preserve the existing join boundary with division lane: this lane may consume
   `(owner_tag_db_id, div_id, div_type/body identity, CArmy parent)` but must not
   emit division movement/scalar state.
8. Gate every widened proof with:
   - compact stream ordered/decodable;
   - one data record per scheduled domain per frame hour;
   - `.battleplan` verification;
   - zero skipped game hours;
   - no source-unavailable, partial-data, bridge-unavailable, or worker-panic
     flags.
