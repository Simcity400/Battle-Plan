# air_combat_losses_live domain 42 full-event reconstruction blocker

Status: validated slice only. Do not call full air combat complete.

The accepted production slice is air-wing strength/equipment delta rows for
losses, reinforcements, deletes, mission region, and equipment amount deltas.
The full sortie/combat event reconstruction backlog remains open except for
the direct opposed event-root candidates rejected here.

## Backlog item narrowed

Rejected or narrowed candidate:

- Direct `StrategicAirManager` / airbase-owner-container / air-wing-pool object
  that holds opposed air-wing body/object references and can serve as a full
  sortie/combat/loss event root.
- Direct air-wing-body attached heap object that carries opposed air-wing refs
  or tactical event-state shape for sortie/combat/loss-cause/ace/reinforcement
  reconstruction.
- Direct land-style `CCombatSide` / combat-registry small-vector root that
  carries air-wing participants for sortie/combat/loss-cause/ace/reinforcement
  reconstruction.
- Stable rooted residual routes under `StrategicAirManager`, active
  mission-region objects, air-base records, air-wing pools/bodies, first-hop
  rooted manager objects, indirect vectors, and POD-like record arrays.
- Direct wing-body `CArmyLeader` / `CNavyLeader` attachment as flying-ace
  participation.
- Direct country-owned ace roster reference from `StrategicAirManager`, active
  wing bodies/pools, mission regions, or first-hop mission objects.
- Air-resolution hook/callback discovery narrowed from broad
  `StrategicAirManager` / air-region search to exact current-build
  RTTI/vtable/static candidates.
- Runtime RVA contract probing rejected the six TBB update wrapper RVAs as
  direct event hooks, rejected the `CAirRegionCombatData` serializer/dispatch
  RVAs as live resolution hooks, and rejected the static
  `Sortie_efficiency_candidate` as strategic-air residual evidence after RTTI
  proved it is `CFEXAir` / `CNavalCombat` carrier sortie efficiency.
- Command-dispatch decoding narrowed the ace-management call block
  `0x6FA8AC -> 0x6FA8BA -> 0xC52840` to command id `0x2FB5` in dispatch table
  `0x6F9EB0`. This proves the remaining `rdx = r14` payload work is a
  controlled command-fixture problem, not a normal hourly air-combat tick hook.
- Static payload-contract decoding of `0xC52840` rejects that function for
  sortie lifecycle, combat pairing, kill attribution, cause/source, tactical
  state, and reinforcement reason. The function consumes an OOB/ace command
  payload, branches on `payload+0x30`, and mutates the country ace roster.

This rejection narrows the search for:

- full sortie lifecycle;
- combat pairing;
- kill attribution;
- cause/source of loss;
- tactical engagement state;
- ace participation;
- rich reinforcement reason.

It does not prove those fields are impossible. It only proves these bounded
direct-reference routes are not the production path on the live 870ebf profile.

## Live proof

Proof artifact:

- `replay_native/lanes/air_combat_losses_live/proofs/direct_event_root_rejection_20260701_v1/air_combat_losses_probe_870ebf.json`

Command:

```powershell
python memory_reader/naval_air/air_combat_losses_probe_870ebf.py --pid 45880 --duration-s 30 --interval-s 0.10 --max-body-field-wings 128 --out-dir replay_native/lanes/air_combat_losses_live/proofs/direct_event_root_rejection_20260701_v1
```

Root validation:

- exe SHA-256 matched `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`;
- daily update sentinel matched at `module+0x1D4390`;
- long-string sentinel matched at `module+0x526A40`;
- `CGameState` root: `module+0x3316E90 -> CGameState*`;
- current date: `CGameState+0x468`;
- `StrategicAirManager`: `CGameState+0x690`;
- `StrategicAirManager` vtable: `module+0x2942118`;
- live `StrategicAirManager` pointer: `0x22C1E3D95D0`;
- live vtable: `0x7FF6AD932118`.

Observed air-wing topology:

- air-wing rows observed: `426`;
- rows with mission region: `109`;
- temporal samples: `194`;
- advanced game hours: `51`;
- skipped game hours: `0`;
- loss-like deltas: `8`;
- reinforcement-like deltas: `10`;
- hourly-or-lower sampling: passed.

## Rejected direct-reference search

Scanned source spans:

- `StrategicAirManager`: `0x0..0x200`, sources scanned `1`;
- `airbase_owner_container`: `0x0..0x120`, sources scanned `439`;
- `air_wing_pool`: `0x0..0x160`, sources scanned `227`.

Criterion:

- unknown-vtable target object in a bounded root field contains at least two
  direct air-wing body/object refs in the first `0x400` bytes;
- event-root upgrade requires opposition-capable refs: at least two owner tags
  with region-bearing wings;
- same-owner pool-local collections are rejected.

Result:

- unknown-vtable pointers seen from air-wing pools: `774`;
- direct-reference candidates found: `6`;
- opposed event-root candidates found: `0`;
- status: `rejected_no_direct_opposed_event_root`;
- classification: `rejected_direct_opposed_event_root`.

Rejected candidate rows:

| source | source addr | field | target | vtable RVA | reason |
| --- | ---: | ---: | ---: | ---: | --- |
| air_wing_pool | `0x22CDEC23140` | `0xd8` | `0x22CDE51B8B0` | `0x29bbe58` | owner `16` only, region `1`, wings `16:110`, `16:274`, `16:354` |
| air_wing_pool | `0x22CDEC239B0` | `0xd8` | `0x22CDE51B8B0` | `0x29bbe58` | owner `16` only, region `1`, wings `16:110`, `16:274`, `16:354` |
| air_wing_pool | `0x22CDEC25260` | `0x128` | `0x22CDE51B830` | `0x29bbe58` | owner `16` only, region `1`, wings `16:110`, `16:274`, `16:354` |
| air_wing_pool | `0x22CDEC24220` | `0xd8` | `0x22CDE51B8F0` | `0x29bbe58` | owner `16` only, region `1`, wings `16:110`, `16:274`, `16:354` |
| air_wing_pool | `0x22CDEC26B10` | `0xd8` | `0x22CDE51B770` | `0x29bbe58` | owner `16` only, region `1`, wings `16:110`, `16:274`, `16:354` |
| air_wing_pool | `0x22CDEC29090` | `0x88` | `0x22CDE51B930` | `0x29bbe58` | owner `16` only, region `1`, wings `16:110`, `16:274`, `16:354` |

Do not retry this exact bounded direct-reference route as a full event root
without a new anchor or a changed criterion. Useful next roots would need to
come from a different evidence source, such as an event hook/function xref,
combat-resolution disassembly, a proven active ace assignment frame, or a wider
root chain outside the scanned manager/container/pool fields.

## Rejected air-wing-body attachment event-root search

Probe:

- `replay_native/lanes/air_combat_losses_live/body_attachment_event_root_probe.py`

Proof artifact:

- `replay_native/lanes/air_combat_losses_live/proofs/body_attachment_event_root_rejection_20260701_v1/body_attachment_event_root_probe_870ebf.json`

Command:

```powershell
python replay_native\lanes\air_combat_losses_live\body_attachment_event_root_probe.py --pid 45880 --duration-s 30 --interval-s 0.10 --body-span 0x300 --target-span 0x600 --max-wings 160 --out-dir replay_native\lanes\air_combat_losses_live\proofs\body_attachment_event_root_rejection_20260701_v1
```

Root validation:

- exe SHA-256 matched `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`;
- daily update sentinel matched at `module+0x1D4390`;
- long-string sentinel matched at `module+0x526A40`;
- `CGameState` root: `module+0x3316E90 -> CGameState*`;
- current date: `CGameState+0x468`;
- `StrategicAirManager`: `CGameState+0x690`;
- `StrategicAirManager` vtable: `module+0x2942118`.

Observed live activity:

- air-wing rows observed: `434`;
- rows with mission region: `185`;
- temporal samples: `165`;
- advanced game hours: `54`;
- skipped game hours: `0`;
- loss-like deltas captured: `22` examples;
- reinforcement-like deltas captured: `16` examples;
- hourly-or-lower sampling: passed.

Rejected body-attachment search:

- source span: `air_wing_body+0x0..0x300`;
- target span: `target+0x0..0x600`;
- source wings scanned: `160`;
- source fields scanned: `15360`;
- heap pointer fields observed: `1629`;
- module-vtable targets observed: `277`;
- candidate body attachments classified: `115`;
- opposed event-root candidates found: `0`;
- status: `rejected_no_body_attached_opposed_event_root`;
- classification: `rejected_body_attached_opposed_event_root`.

Criterion:

- active or sampled air-wing body has a direct heap pointer field, outside
  known pool/mission-region/equipment-bank fields;
- the target object has a module-local vtable outside the already-known body,
  object, pool, and region vtables;
- event-root upgrade requires direct opposed air-wing refs, meaning at least two
  owner tags with wing refs and either a region ref or a source-wing ref in the
  target scan span.

Result:

- `0` body-attached opposed event-root candidates;
- candidates with region refs only were not accepted as combat event roots;
- candidates without direct wing/region refs were rejected;
- no compact schema change is justified from this path.

Do not retry this exact body-attachment route as a full event root without a new
structure hypothesis, a wider root chain justified by disassembly, or an active
ace/combat-resolution fixture that changes the criterion.

## Rejected land-style CCombatSide event-root search

Probe:

- `replay_native/lanes/air_combat_losses_live/ccombat_side_event_root_probe_870ebf.py`

Proof artifact:

- `replay_native/runs/air_combat_losses_live_ccombat_side_rejection_20260701_v1/ccombat_side_event_root_probe_870ebf.json`

Command:

```powershell
python replay_native\lanes\air_combat_losses_live\ccombat_side_event_root_probe_870ebf.py --pid 45880 --duration-s 20 --interval-s 0.10 --max-body-field-wings 96 --workers 6 --out-dir replay_native\runs\air_combat_losses_live_ccombat_side_rejection_20260701_v1
```

Root validation:

- exe SHA-256 matched `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`;
- daily update sentinel matched at `module+0x1D4390`;
- long-string sentinel matched at `module+0x526A40`;
- `CGameState` root: `module+0x3316E90 -> CGameState*`;
- current date: `CGameState+0x468`;
- `StrategicAirManager`: `CGameState+0x690`;
- `StrategicAirManager` vtable: `module+0x2942118`.

Observed live air activity during the same proof:

- air-wing rows observed: `465`;
- rows with mission region: `199`;
- advanced game hours: `34`;
- skipped game hours: `0`;
- loss-like deltas captured: `23`;
- reinforcement-like deltas captured: `10`;
- equipment scale mismatches: `0`;
- hourly-or-lower sampling: passed.

Rejected CCombatSide route:

- `CCombatSide` vtable: `module+0x29913C0`;
- `CCombatRegistry` vtable: `module+0x29914D8`;
- side unit vector: `+0x20 data`, `+0x28 capacity`, `+0x2C size`;
- registry handle: `+0x18`;
- registry attacker/defender/front: `+0x28/+0x30/+0x38`;
- air-wing participant identity checked as `module+0x29641B0`, type `69`.

Result:

- heap arena scanned: `0x22A80000000..0x22CFC568000`, `16401` readable regions;
- `CCombatSide` hits: `217`;
- side classifications: `136` empty, `77` exact populated, `4` bad-side-vtable;
- participant kind counts: `77` division-side hits, `0` air-wing-side hits,
  `0` naval-unit-side hits;
- combat-registry hits: `42`;
- registry handle types: `41` with type `62`, `1` with type `10325`;
- status: `rejected_no_ccombat_side_air_event_root`;
- classification: `rejected_no_ccombat_side_air_wing_participants`.

This rejects the current-build land-style `CCombatSide` / registry small-vector
route as a full air-combat event root. Do not retry this route without a new
anchor showing air-wing participants in the side vector, a different combat-side
class, or disassembly proving that air combat enters this registry path. This is
not a universal impossibility proof for tick-local POD records, index/vector-
indirect storage, region-owned known-manager containers, or state distributed
onto wing/mission/ace structures.

## Rejected stable rooted residual matrix

Probe:

- `replay_native/lanes/air_combat_losses_live/residual_route_matrix_probe_870ebf.py`

Proof artifact:

- `replay_native/runs/air_combat_losses_live_residual_matrix_20260701_v5/residual_route_matrix_probe_870ebf.json`

Command:

```powershell
python replay_native\lanes\air_combat_losses_live\residual_route_matrix_probe_870ebf.py --pid 45880 --duration-s 10 --interval-s 0.10 --max-wing-bodies 96 --max-body-field-wings 96 --max-container-sources 48 --max-pool-sources 64 --max-region-sources 96 --max-one-hop-sources 192 --max-vector-records 1024 --max-record-scan-records 128 --object-span 0x280 --out-dir replay_native\runs\air_combat_losses_live_residual_matrix_20260701_v5
```

Root validation:

- exe SHA-256 matched `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`;
- `CGameState`: `module+0x3316E90`;
- `StrategicAirManager`: `CGameState+0x690`, vtable `module+0x2942118`;
- air-base state table: `StrategicAirManager+0x90`;
- active mission-region objects: `module+0x29569A8` / `module+0x29569F8`, id `+0x58`;
- air-wing body: `module+0x29641B0`, type `69`, id `+0x0C`, pool `+0x28`, mission region `+0xB0`.

Observed live activity:

- air-wing rows observed: `573`;
- rows with mission region: `250`;
- advanced game hours: `17`;
- skipped game hours: `0`;
- loss-like deltas captured: `21`;
- reinforcement-like deltas captured: `11`;
- hourly-or-lower sampling: passed.

Scanned roots:

- root sources: `597`;
- one-hop rooted sources: `192`;
- active pool sources available: `130`;
- country container sources used: `48` of `439`;
- pool sources used: `64` of `279`, active-prioritized;
- mission-region sources used: `20` of `20`;
- wing-body sources used: `96`;
- air-base record pointers seen: `2164`, valid air-base records: `368`.

Result:

- accepted vector headers inspected: `4553`;
- matrix candidates classified: `1301`;
- direct rooted opposed event candidates: `0`;
- status: `rejected_no_rooted_opposed_indirect_vector_or_pod_event_state`;
- verdict: `rejected_stable_rooted_residual_routes_blocked_tick_local_state`.

Rejected / separated false routes:

- `47` `air_base_record` direct wing-object collections were rejected as basing
  collections, not combat roots. They are rooted at `StrategicAirManager+0x90`
  and contain co-located wing object pointers, including idle/no-region wings.
- `453` POD owner/wing-id apparent matches were rejected as collision-prone
  without direct opposed wing refs.
- `16` unanchored record-stride apparent opposed refs were rejected as synthetic
  reinterpretations of pointer/vector storage.
- same-owner or non-region collections: `701`;
- non-opposed or ambiguous identity collections: `84`.

This rejects the stable production-readable residual roots currently reachable
from the native air manager/region/wing graph. It does not prove that
tick-local/function-local air-resolution records never exist; those are blocked
without a controlled tick-synchronous fixture or an air-resolution hook.

Exact fixture required to unblock tick-local/function-local state:

- controlled before/during/after capture around a known active air-combat tick;
- two opposing countries with exactly one identified fighter wing each in the
  same strategic air region;
- one wing with a known assigned ace and one without;
- reinforcements disabled or manually triggered separately;
- game paused immediately after a visible air loss;
- UI/save anchors for both wing ids, owner tag-db-ids, region id, ace id/name,
  loss count, and reinforcement source;
- three read-only heap/root snapshots diffing `StrategicAirManager`, active
  mission-region objects, both wing bodies/pools, country ace containers, and
  first-hop manager objects.

If that diff still finds no stable root, the production route must be an
in-DLL read-only hook/probe at the air-resolution function identified by
disassembly or by the tick-synchronous diff.

## Rejected ace participation routes

### Direct wing-body leader attachment

Probe:

- `replay_native/lanes/air_combat_losses_live/wing_leader_attachment_probe_870ebf.py`

Proof artifact:

- `replay_native/runs/air_combat_losses_live_wing_leader_attachment_20260701_v2/wing_leader_attachment_probe_870ebf.json`

Command:

```powershell
python replay_native\lanes\air_combat_losses_live\wing_leader_attachment_probe_870ebf.py --pid 45880 --duration-s 10 --interval-s 0.10 --max-wings 512 --max-body-field-wings 128 --out-dir replay_native\runs\air_combat_losses_live_wing_leader_attachment_20260701_v2
```

Result:

- air-wing rows observed: `574`;
- rows with mission region: `257`;
- advanced game hours: `17`;
- skipped game hours: `0`;
- loss-like deltas captured: `18`;
- reinforcement-like deltas captured: `1`;
- exact leader attachments found: `39`;
- attachment fields: `+0x290` (`19`), `+0x2B0` (`19`), `+0x2F0` (`1`);
- classification: `rejected_direct_wing_leader_attachment_as_ace_participation`.

Rejection evidence:

- attached objects are exact `CArmyLeader` / `CNavyLeader`-shaped objects, not
  proven flying-ace records;
- owners `33`, `36`, and `51` had exact attachments despite country ace count
  `0`;
- shared leader targets appeared across unrelated owner/wing keys;
- named samples include regular army/navy leaders such as
  `Konstantin Rokossovsky`, `Tamon Yamaguchi`, `Harold Rainsford Stark`,
  `Johannes Blaskowitz`, and `Erwin von Witzleben`.

Do not use these wing-body leader fields as ace participation without a new
controlled ace assignment/deassignment fixture that proves the specific field
tracks a flying ace.

### Country-owned ace roster direct reference

Probe:

- `replay_native/lanes/air_combat_losses_live/ace_roster_route_probe_870ebf.py`

Proof artifact:

- `replay_native/runs/air_combat_losses_live_ace_roster_route_20260701_v2/ace_roster_route_probe_870ebf.json`

Command:

```powershell
python replay_native\lanes\air_combat_losses_live\ace_roster_route_probe_870ebf.py --pid 45880 --duration-s 10 --interval-s 0.10 --country-span 0x3000 --max-vector-cap 512 --max-wings 192 --max-body-field-wings 96 --out-dir replay_native\runs\air_combat_losses_live_ace_roster_route_20260701_v2
```

Validated ace-roster root:

- country table: `CGameState+0x310` begin, `CGameState+0x31C` count;
- `CCountry` vtable: `module+0x27D0E90`;
- country tag-db-id: `CCountry+0x08`;
- ace count: `CCountry+0x12E4`;
- likely ace roster: `CCountry+0x12D8` vector;
- likely ace entry vtable: `module+0x27C7DD0`.

Result:

- nonzero-ace countries: `14`;
- matching ace-count-sized vectors found during broad country scan: `109`;
- pinned likely ace-roster entry pointers: `154`;
- direct refs from air roots to pinned roster entries: `0`;
- air-root source count scanned: `833`;
- first-hop air-root sources scanned: `256`;
- air-wing rows observed: `576`;
- rows with mission region: `237`;
- advanced game hours: `16`;
- skipped game hours: `0`;
- loss-like deltas captured: `7`;
- reinforcement-like deltas captured: `16`;
- classification: `rejected_no_direct_ace_roster_ref_from_air_roots`.

This validates a country-owned ace roster root, but rejects a direct
wing/mission/region participation path from that roster under the bounded
current-build roots. Ace participation remains blocked on a controlled ace
assignment/loss fixture or an air-resolution hook.

## Static air-resolution hook candidates narrowed

Probe scripts:

- `replay_native/lanes/air_combat_losses_live/air_resolution_static_hook_probe_870ebf.py`
- `replay_native/lanes/air_combat_losses_live/air_resolution_target_disasm_870ebf.py`

Proof artifacts:

- `replay_native/runs/air_combat_losses_live_static_hooks_20260701_v3/air_resolution_static_hook_probe_870ebf.json`
- `replay_native/runs/air_combat_losses_live_static_hooks_20260701_v3/air_resolution_target_disasm_870ebf.json`

Commands:

```powershell
python replay_native\lanes\air_combat_losses_live\air_resolution_static_hook_probe_870ebf.py --exe "<local-path> of Iron IV\hoi4.exe" --out-dir replay_native\runs\air_combat_losses_live_static_hooks_20260701_v3 --max-strings 900 --max-function-bytes 0x20000 --max-candidates 120 --snippet-instructions 16
python replay_native\lanes\air_combat_losses_live\air_resolution_target_disasm_870ebf.py --exe "<local-path> of Iron IV\hoi4.exe" --out-dir replay_native\runs\air_combat_losses_live_static_hooks_20260701_v3 --window-bytes 0x700 --max-instructions 90 --max-disasm-instructions 300 --max-evidence 100
```

Static root validation:

- exe SHA-256 matched `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`;
- image base: `0x140000000`;
- RTTI/update/combat type names located: `25`;
- static candidates found, not validated as a production hook:
  `111` air-resolution/air-wing-update-shaped functions, `1`
  ace-roster-access-shaped function.

Exact RTTI/vtable candidates:

| RTTI type | vtable RVA | slot | function RVA | result |
| --- | ---: | ---: | ---: | --- |
| `DailyUpdate@CStrategicAirManager` TBB body | `0x29436C0` | `1` | `0xC57A50` | wrapper calls `0xC350D0` |
| `HourlyUpdateThreadedInternal@CStrategicAirManager` lambda 1 | `0x29436E0` | `1` | `0xC57960` | wrapper calls `0xC34C80` |
| `HourlyUpdateThreadedInternal@CStrategicAirManager` lambda 2 | `0x2943700` | `1` | `0xC57AA0` | wrapper calls `0xC35240` |
| `HourlyUpdateThreadedInternal@CStrategicAirManager` lambda 3 | `0x2943720` | `1` | `0xC57B40` | wrapper calls `0xC35520` |
| `HourlyUpdateThreadedInternal@CStrategicAirManager` lambda 4 | `0x2943740` | `1` | `0xC57BE0` | wrapper calls `0xC35800` |
| `HourlyUpdateThreadedInternal@CStrategicAirManager` lambda 5 | `0x2943760` | `1` | `0xC57C30` | wrapper calls `0xC35970` |
| `CAirRegionCombatData` | `0x2942188` | `2` | `0x1950F80` | serializer-like, not a live hook by itself |
| `CAirRegionCombatData` | `0x2942188` | `4` | `0x1950A30` | dispatch-like, not a live hook by itself |

Bounded disassembly evidence:

- the six `DailyUpdate` / `HourlyUpdateThreadedInternal` wrapper functions are
  exact `.pdata` ranges of `0x4E` bytes each and make direct calls to the phase
  bodies above;
- phase/body chunks reached from those wrappers include direct jump targets
  `0xC36380`, `0xC36610`, `0xC36D00`, `0xC36F90`, `0xC37380`,
  `0xC37760`, `0xC379F0`, `0xC37CE0`, and `0xC37F70`;
- those chunks show air-update container/region activity, including
  `air_wing_body.mission_region`, `air_region.region_id`,
  air-wing pool/vector fields, and calls such as `0x140C3B330`,
  `0x140C3AFB0`, `0x140C3ABB0`, `0x140C3CE30`, `0x140C57EF0`,
  `0x140C58100`, `0x140C58290`, and `0x140C58400`;
- `0xC52840` is a separate `strategicair.cpp` ace-management candidate with
  `country.ace_roster_vector` / `country.ace_count` hits and strings such as
  `. name set without airwing first` and
  `. start_experience_factor set without airwing first`;
- `0x1963700` is a sortie-efficiency candidate anchored by
  `pAirWingPool != nullptr && "Could not find an air wing pool to compute sortie efficiency"`,
  but the static proof does not show combat pairing, kill attribution, loss
  cause/source, tactical state, ace participation, or reinforcement reason.

Rejected tooling route:

- Ghidra `Hoi4RootHunt` was attempted against the exact candidate VAs with
  `--max=4` and again against only the six named update-body VAs with
  `--max=0`;
- both runs had to be terminated because Ghidra's current project function
  boundaries collapsed the targets into the giant containing function around
  `0x140082CB0` and spawned long-running decompiler workers, with warnings
  including `out of address spaces` and unreadable `0x20ccccccc` varnodes;
- for this route, `.pdata`-bounded Capstone disassembly is the validated static
  evidence source, while Ghidra decompilation is rejected unless the project is
  repaired with correct function boundaries.

This narrows the direct air-resolution hook search to exact current-build
candidate RVAs, but does not validate any candidate as a DLL-only production
event hook. The remaining unblocked forward path is bounded instrumentation of
these RVAs, or a controlled tick-synchronous fixture that proves which candidate
carries sortie lifecycle, combat pairing, attribution, cause/source, tactical
state, ace participation, and rich reinforcement reason.

## Runtime RVA contract candidates reduced

Probe scripts:

- `replay_native/lanes/air_combat_losses_live/air_resolution_rva_contract_probe_870ebf.py`
- `replay_native/lanes/air_combat_losses_live/air_resolution_observed_vtable_rtti_870ebf.py`
- `replay_native/lanes/air_combat_losses_live/air_resolution_target_disasm_870ebf.py`
- `replay_native/lanes/air_combat_losses_live/air_resolution_callsite_probe_870ebf.py`
- `replay_native/lanes/air_combat_losses_live/air_resolution_dispatch_xref_probe_870ebf.py`
- `replay_native/lanes/air_combat_losses_live/air_resolution_dispatch_table_probe_870ebf.py`
- `replay_native/lanes/air_combat_losses_live/air_resolution_ace_payload_static_probe_870ebf.py`

Proof artifacts:

- `replay_native/runs/air_combat_losses_live_rva_contract_20260701_v1_wrappers/air_resolution_rva_contract_probe_870ebf.json`
- `replay_native/runs/air_combat_losses_live_rva_contract_20260701_v1_chunks_ace/air_resolution_rva_contract_probe_870ebf.json`
- `replay_native/runs/air_combat_losses_live_rva_contract_20260701_v1_ace_sortie_long/air_resolution_rva_contract_probe_870ebf.json`
- `replay_native/runs/air_combat_losses_live_rva_contract_20260701_v1_rtti/air_resolution_observed_vtable_rtti_870ebf.json`
- `replay_native/runs/air_combat_losses_live_rva_contract_20260701_v1_internal_calls/air_resolution_rva_contract_probe_870ebf.json`
- `replay_native/runs/air_combat_losses_live_rva_contract_20260701_v1_internal_calls_rtti/air_resolution_observed_vtable_rtti_870ebf.json`
- `replay_native/runs/air_combat_losses_live_rva_contract_20260701_v2_deep_calls_air_fields/air_resolution_rva_contract_probe_870ebf.json`
- `replay_native/runs/air_combat_losses_live_rva_contract_20260701_v2_deep_calls_air_fields_rtti/air_resolution_observed_vtable_rtti_870ebf.json`
- `replay_native/runs/air_combat_losses_live_rva_contract_20260701_v1_interior_calls/air_resolution_rva_contract_probe_870ebf.json`
- `replay_native/runs/air_combat_losses_live_rva_contract_20260701_v1_interior_calls_rtti/air_resolution_observed_vtable_rtti_870ebf.json`
- `replay_native/runs/air_combat_losses_live_rva_contract_20260701_v2_pair_focus/air_resolution_rva_contract_probe_870ebf.json`
- `replay_native/runs/air_combat_losses_live_target_disasm_20260701_v4_pair_inner/air_resolution_target_disasm_870ebf.json`
- `replay_native/runs/air_combat_losses_live_target_disasm_20260701_v5_pair_inner_unbounded/air_resolution_target_disasm_870ebf.json`
- `replay_native/runs/air_combat_losses_live_callsite_probe_20260701_v2_pair_ace/air_resolution_callsite_probe_870ebf.json`
- `replay_native/runs/air_combat_losses_live_target_disasm_20260701_v7_interior_all_pdata/air_resolution_target_disasm_870ebf.json`
- `replay_native/runs/air_combat_losses_live_target_disasm_20260701_v8_interior_all_unbounded/air_resolution_target_disasm_870ebf.json`
- `replay_native/runs/air_combat_losses_live_target_disasm_20260701_v9_ace_callsite/air_resolution_target_disasm_870ebf.json`
- `replay_native/runs/air_combat_losses_live_callsite_probe_20260701_v4_ace_function_context/air_resolution_callsite_probe_870ebf.json`
- `replay_native/runs/air_combat_losses_live_callsite_probe_20260701_v5_ace_branch_context/air_resolution_callsite_probe_870ebf.json`
- `replay_native/runs/air_combat_losses_live_dispatch_xref_20260701_v1_ace_block/air_resolution_dispatch_xref_probe_870ebf.json`
- `replay_native/runs/air_combat_losses_live_dispatch_xref_20260701_v2_ace_block_xref_context/air_resolution_dispatch_xref_probe_870ebf.json`
- `replay_native/runs/air_combat_losses_live_target_disasm_20260701_v10_ace_caller_full/air_resolution_target_disasm_870ebf.json`
- `replay_native/runs/air_combat_losses_live_dispatch_table_20260701_v1_ace_block/air_resolution_dispatch_table_probe_870ebf.json`
- `replay_native/runs/air_combat_losses_live_ace_payload_static_20260701_v1/air_resolution_ace_payload_static_probe_870ebf.json`

Commands:

```powershell
python replay_native\lanes\air_combat_losses_live\air_resolution_rva_contract_probe_870ebf.py --pid 45880 --duration-s 12 --max-hits-per-target 6 --target-group wrappers --target-group phases --out-dir replay_native\runs\air_combat_losses_live_rva_contract_20260701_v1_wrappers
python replay_native\lanes\air_combat_losses_live\air_resolution_rva_contract_probe_870ebf.py --pid 45880 --duration-s 12 --max-hits-per-target 6 --target-group chunks --target-group ace_sortie --target-group combat_data --out-dir replay_native\runs\air_combat_losses_live_rva_contract_20260701_v1_chunks_ace
python replay_native\lanes\air_combat_losses_live\air_resolution_rva_contract_probe_870ebf.py --pid 45880 --duration-s 45 --max-hits-per-target 12 --target-group ace_sortie --out-dir replay_native\runs\air_combat_losses_live_rva_contract_20260701_v1_ace_sortie_long
python replay_native\lanes\air_combat_losses_live\air_resolution_observed_vtable_rtti_870ebf.py --contract-json replay_native\runs\air_combat_losses_live_rva_contract_20260701_v1_wrappers\air_resolution_rva_contract_probe_870ebf.json --contract-json replay_native\runs\air_combat_losses_live_rva_contract_20260701_v1_chunks_ace\air_resolution_rva_contract_probe_870ebf.json --contract-json replay_native\runs\air_combat_losses_live_rva_contract_20260701_v1_ace_sortie_long\air_resolution_rva_contract_probe_870ebf.json --out-dir replay_native\runs\air_combat_losses_live_rva_contract_20260701_v1_rtti
python replay_native\lanes\air_combat_losses_live\air_resolution_rva_contract_probe_870ebf.py --pid 45880 --duration-s 15 --max-hits-per-target 8 --target-group internal_calls --out-dir replay_native\runs\air_combat_losses_live_rva_contract_20260701_v1_internal_calls
python replay_native\lanes\air_combat_losses_live\air_resolution_observed_vtable_rtti_870ebf.py --contract-json replay_native\runs\air_combat_losses_live_rva_contract_20260701_v1_internal_calls\air_resolution_rva_contract_probe_870ebf.json --out-dir replay_native\runs\air_combat_losses_live_rva_contract_20260701_v1_internal_calls_rtti
python replay_native\lanes\air_combat_losses_live\air_resolution_rva_contract_probe_870ebf.py --pid 45880 --duration-s 15 --max-hits-per-target 8 --target-group deep_calls --out-dir replay_native\runs\air_combat_losses_live_rva_contract_20260701_v2_deep_calls_air_fields
python replay_native\lanes\air_combat_losses_live\air_resolution_observed_vtable_rtti_870ebf.py --contract-json replay_native\runs\air_combat_losses_live_rva_contract_20260701_v2_deep_calls_air_fields\air_resolution_rva_contract_probe_870ebf.json --out-dir replay_native\runs\air_combat_losses_live_rva_contract_20260701_v2_deep_calls_air_fields_rtti
python replay_native\lanes\air_combat_losses_live\air_resolution_target_disasm_870ebf.py --out-dir replay_native\runs\air_combat_losses_live_target_disasm_20260701_v2_deep_calls --window-bytes 0x900 --max-instructions 120 --max-disasm-instructions 420 --max-evidence 120
python replay_native\lanes\air_combat_losses_live\air_resolution_rva_contract_probe_870ebf.py --pid 45880 --duration-s 15 --max-hits-per-target 6 --target-group interior_calls --out-dir replay_native\runs\air_combat_losses_live_rva_contract_20260701_v1_interior_calls
python replay_native\lanes\air_combat_losses_live\air_resolution_observed_vtable_rtti_870ebf.py --contract-json replay_native\runs\air_combat_losses_live_rva_contract_20260701_v1_interior_calls\air_resolution_rva_contract_probe_870ebf.json --out-dir replay_native\runs\air_combat_losses_live_rva_contract_20260701_v1_interior_calls_rtti
python replay_native\lanes\air_combat_losses_live\air_resolution_rva_contract_probe_870ebf.py --pid 45880 --duration-s 20 --max-hits-per-target 40 --label Interior_air_call_F46A10 --label Interior_air_call_F6B7B0 --out-dir replay_native\runs\air_combat_losses_live_rva_contract_20260701_v2_pair_focus
python replay_native\lanes\air_combat_losses_live\air_resolution_target_disasm_870ebf.py --out-dir replay_native\runs\air_combat_losses_live_target_disasm_20260701_v4_pair_inner --window-bytes 0x900 --max-instructions 160 --max-disasm-instructions 520 --max-evidence 180 --respect-pdata-all
python replay_native\lanes\air_combat_losses_live\air_resolution_target_disasm_870ebf.py --out-dir replay_native\runs\air_combat_losses_live_target_disasm_20260701_v5_pair_inner_unbounded --window-bytes 0x300 --max-instructions 120 --max-disasm-instructions 180 --max-evidence 120 --no-respect-pdata-all
python replay_native\lanes\air_combat_losses_live\air_resolution_callsite_probe_870ebf.py --out-dir replay_native\runs\air_combat_losses_live_callsite_probe_20260701_v2_pair_ace --preceding 16 --context-bytes 128
python replay_native\lanes\air_combat_losses_live\air_resolution_target_disasm_870ebf.py --out-dir replay_native\runs\air_combat_losses_live_target_disasm_20260701_v7_interior_all_pdata --window-bytes 0x900 --max-instructions 160 --max-disasm-instructions 520 --max-evidence 220 --respect-pdata-all
python replay_native\lanes\air_combat_losses_live\air_resolution_target_disasm_870ebf.py --out-dir replay_native\runs\air_combat_losses_live_target_disasm_20260701_v8_interior_all_unbounded --window-bytes 0x700 --max-instructions 180 --max-disasm-instructions 220 --max-evidence 160 --no-respect-pdata-all
python replay_native\lanes\air_combat_losses_live\air_resolution_target_disasm_870ebf.py --out-dir replay_native\runs\air_combat_losses_live_target_disasm_20260701_v9_ace_callsite --window-bytes 0x1200 --max-instructions 220 --max-disasm-instructions 700 --max-evidence 240 --respect-pdata-all
python replay_native\lanes\air_combat_losses_live\air_resolution_callsite_probe_870ebf.py --out-dir replay_native\runs\air_combat_losses_live_callsite_probe_20260701_v4_ace_function_context --target-rva 0xC52840 --preceding 120 --context-from-function
python replay_native\lanes\air_combat_losses_live\air_resolution_callsite_probe_870ebf.py --out-dir replay_native\runs\air_combat_losses_live_callsite_probe_20260701_v5_ace_branch_context --target-rva 0xC52840 --preceding 120 --context-from-function --branch-window 0x80 --max-incoming-branches 64
python replay_native\lanes\air_combat_losses_live\air_resolution_dispatch_xref_probe_870ebf.py --out-dir replay_native\runs\air_combat_losses_live_dispatch_xref_20260701_v1_ace_block --target-rva 0x6FA8AC --context-bytes 0x220 --after-bytes 0x40
python replay_native\lanes\air_combat_losses_live\air_resolution_dispatch_xref_probe_870ebf.py --out-dir replay_native\runs\air_combat_losses_live_dispatch_xref_20260701_v2_ace_block_xref_context --target-rva 0x6FA8AC --context-bytes 0x220 --after-bytes 0x40 --xref-context-bytes 0x120 --xref-after-bytes 0x80
python replay_native\lanes\air_combat_losses_live\air_resolution_target_disasm_870ebf.py --out-dir replay_native\runs\air_combat_losses_live_target_disasm_20260701_v10_ace_caller_full --window-bytes 0x5200 --max-instructions 260 --max-disasm-instructions 6500 --max-evidence 2500 --respect-pdata-all
python replay_native\lanes\air_combat_losses_live\air_resolution_dispatch_table_probe_870ebf.py --out-dir replay_native\runs\air_combat_losses_live_dispatch_table_20260701_v1_ace_block --target-rva 0x6FA8AC --target-rva 0x6FA821 --target-rva 0x6FA8C4
python replay_native\lanes\air_combat_losses_live\air_resolution_ace_payload_static_probe_870ebf.py --out-dir replay_native\runs\air_combat_losses_live_ace_payload_static_20260701_v1
```

Runtime validation:

- all runs matched exe SHA-256
  `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`;
- wrapper/phase probe advanced `21` game hours, recorded `72` hits, matched
  `6` wrapper-to-phase contracts, and had `0` Frida/script errors;
- exact runtime wrapper contract: wrapper entry `rcx` is the TBB body object,
  phase entry receives `rcx = wrapper + 0x28`, `rdx = wrapper`, and
  `r8 = wrapper + 0x08`;
- rejected direct hook endpoints:
  `0xC57A50`, `0xC57960`, `0xC57AA0`, `0xC57B40`, `0xC57BE0`,
  and `0xC57C30` are TBB scheduler wrappers, not sortie/combat event hooks;
- chunk/ace/combat-data probe advanced `21` game hours, recorded `54` hits,
  sampled all nine worker chunks, and had `0` Frida/script errors;
- chunk entries also use scheduler/work-slice arguments at entry:
  `rcx = wrapper + 0x28`, `rdx = wrapper object`, `r8 = wrapper + 0x08`;
- `CAirRegionCombatData` serializer/dispatch RVAs `0x1950F80` and
  `0x1950A30` had `0` hits during the live air-worker window and remain
  rejected as serialization endpoints, not live resolution hooks;
- long ace/sortie probe advanced `77` game hours, recorded `12` hits at
  `0x1963700`, and `0` hits at `0xC52840`;
- RTTI resolution of observed `0x1963700` arguments:
  `rcx` vtable `0x2A0A588` is `.?AVCFEXAir@@`, and `r8` vtable
  `0x29C6F18` is `.?AVCNavalCombat@@`.
- internal-call probe advanced `23` game hours, recorded `120` hits across
  all `15` direct worker-call targets, and had `0` Frida/script errors;
- internal-call RTTI resolved entry arguments to `CAirBase` (`0x2941FA0`),
  `CStrategicAir` (`0x2941FF8`), `CAirMission` (`0x2965CA0`),
  `CCountrySupplySystem` (`0x298BBF8`), and TBB scheduler body vtables
  `0x2943700`, `0x29437C0`, `0x29437E0`, `0x2943800`;
- the internal-call entry snapshots contained no observed `AirWingBody`
  (`0x29641B0`), `AirWingPool` (`0x2964208`), `AirWingRegion`
  (`0x29569A8` / `0x29569F8`), or ace-entry (`0x27C7DD0`) vtable argument.
- deep-call probe advanced `25` game hours, recorded `104` hits, sampled `13`
  of `16` worker-interior callees, and had `0` Frida/script errors;
- deep-call RTTI/field snapshots validated live `CAirWing` entry arguments at
  `0xF4C430`, `0xF51060`, `0xF49710`, and `0xF4B6E0`, and live
  `CStrategicRegion` arguments at `0xF652E0` and `0xF625B0`. The decoded
  single-wing fields were wing id, wing type, pool pointer/vtable, plane count,
  effective planes, mission mask, and mission-region id/vtable;
- deep-call entries did not expose opposed wing pairs, loss attribution,
  cause/source, tactical engagement state, ace participation, or reinforcement
  reason at function entry. No compact event record was emitted from these
  scalar/single-wing fields;
- second-level interior probe advanced `25` game hours, recorded `168` hits,
  sampled all `28` selected interior callees, and had `0` Frida/script errors;
- interior RTTI resolved ordinary gameplay object arguments:
  `CAirWing`, `CAirWingPool`, `CAirMission`, `CStrategicRegion`, `CAirBase`,
  `CStrategicAir`, `CEquipmentVariantPool`, `CSubUnitDefinition`,
  `CEquipmentVariant`, `CCountry`, `CWeatherManager`, and `CArmy`;
- `0xF46A10` initially looked pair-shaped in runtime samples, but `.pdata`
  bounded disassembly rejects it as a combat-pair event hook: the function body
  ignores the incoming `rdx`, keeps only `rcx` in `rbx`, checks
  `CAirWing+0x38`, `CAirWing+0xA0`, and `CAirWing+0xA2C`, calls
  `0xF4E0E0`, and returns small state constants `2`, `4`, `3`, or `5`;
- `0xF4E0E0`, the only domain-specific callee from `0xF46A10`, is a
  `27`-instruction boolean helper that walks through `rcx+0x38` and never
  receives or resolves an enemy/paired wing;
- `0xF6B7B0` initially looked pair-shaped because stale `r8` sometimes pointed
  to `CAirMission`, but `.pdata` bounded disassembly rejects it as a pair event
  hook: the function consumes only `rcx`, derives local `rdx/r8`, calls
  `0xF49E80`, and returns a scalar comparison;
- callsite byte scan found `108` direct call/jump xrefs across the narrowed
  pair/ace RVAs. `0xC52840` has exactly one direct caller, at callsite
  `0x6FA8BA` in runtime function `0x6F9170..0x6FE079`; the immediate call
  contract is `rcx = return(0x6C2D80(r15))`, `rdx = r14`.
- the focused runtime pair proof for `0xF46A10`/`0xF6B7B0` recorded `0` hits
  because the game clock did not advance (`0` game hours); this run is fixture
  evidence only and is not a candidate rejection.
- full interior disassembly covered the `28` live second-level interior
  callees with `.pdata` clipping plus first-return unbounded slices where
  function table extents were misleading;
- rejected region scalar helpers:
  `0xC44FA0` is a five-instruction `CStrategicAir` region-table accessor
  (`rax = [rcx+0x98] + CStrategicRegion+0x58 * 24`);
  `0xC448F0` is a `CAirBase` scalar resolver that writes one `dword` to the
  caller-provided output pointer; `0xE0CD40`, `0xE0CF00`, and `0xF71050` loop
  region-owned vectors and write aggregate ratios/sums to output pointers.
  None of these bodies constructs or receives opposed participants, loss cause,
  ace entry, or tactical event records;
- rejected subunit/equipment helper route under `0xF51060`:
  `0xF51A60` initializes a bitmask-to-byte-index helper buffer from `edx`;
  `0x1004BF0`, `0x1004C40`, `0x1004C50`, `0x1004C80`, and `0x1004E70` are
  `CSubUnitDefinition` / equipment variant helper wrappers or reset/copy
  routines. The earlier `CAirWing+0x2F0`/`+0x968` path is not ace
  participation and does not carry loss attribution or reinforcement reason;
- ace arg-factory disassembly resolved `0x6C2D80`: it reads
  `GameState+0x690` (`StrategicAirManager`), checks container count at
  `+0x3C`, indexes country containers from `+0x30`, and returns the selected
  country strategic-air container. Therefore `0xC52840.rcx` is not an air
  combat event object; it is the country strategic-air container selected by
  the caller;
- widened `0xC52840` callsite context confirms the sole callsite still passes
  `rdx = r14` as the command/event object and `rcx = 0x6C2D80(r15)`. The
  dispatch into `0x6FA8AC` appears to come through the caller's command
  dispatch arm rather than the normal hourly worker path.
- bounded xref proof rejects direct branch/root xrefs to dispatch block
  `0x6FA8AC`: the only rel32-looking xref is a false positive inside
  `mov qword ptr [rax+0x10], r12`, and there are `0` direct branch xrefs and
  `0` absolute xrefs to `0x6FA8AC` / `0x6FA8BA`;
- full caller disassembly of `0x6F9170..0x6FE079` found six `jmp rcx`
  command dispatch tables. Table `dispatch_jmp_6F9EB0` maps command id
  `0x2FB5` (`cmd_base 0x2F59`, index `0x5C`, selector `0x5`) to block
  `0x6FA8AC`, the block that calls `0xC52840`. The same table maps nearby
  command id `0x2FB9` to `0x6FA8C4`.
- static ace-payload proof decoded the `0xC52840` body:
  `rdx` is copied to `r15`; the command discriminator is `dword [r15+0x30]`
  with branch values `0x31E2`, `0x199`, `0x1B`, and `0x34EC`; payload data
  fields read are `r15+0x28`, `+0x34`, `+0x38`, and `+0x44`; country ace-roster
  fields read/written are `r13+0x12D8`, `+0x12E0`, `+0x12E4`, and `+0x12E8`;
- the same proof resolved nearby diagnostic strings from
  `strategicair.cpp`: `Error scripting OOB`, `. The Ace '`,
  `Ace ignored.`, `. name set without airwing first`, and
  `. start_experience_factor set without airwing first`.

Rejections/narrowing:

- do not hook the six wrapper RVAs for event payload extraction; hook phase or
  interior worker/callback targets instead;
- do not retry `0x1950F80` / `0x1950A30` as live air combat resolution hooks;
- do not treat `0x1963700` as strategic-air sortie lifecycle, combat pairing,
  attribution, cause/source, tactical state, ace participation, or
  reinforcement reason. It is carrier/naval sortie efficiency (`CFEXAir` with
  `CNavalCombat`) and belongs outside domain 42 residual extraction;
- do not treat the direct worker-call entry contracts as already carrying full
  event payloads. They are live air-update helpers, but their entry arguments
  are air-base/strategic-air/mission/supply/scheduler objects rather than
  participant-pair or loss-attribution records;
- do not treat `0xF46A10` or its inner helper `0xF4E0E0` as sortie/combat
  participant-pair hooks. Runtime register snapshots suggested
  `CAirWing`/`CAirWing`, but static body evidence proves the second argument is
  not consumed and the function is a single-wing state/category helper;
- do not treat `0xF6B7B0` or its inner helper `0xF49E80` as mission-pair or
  combat-pair hooks. Runtime register snapshots suggested
  `CAirMission`/`CAirMission`, but static body evidence proves only the first
  mission object is consumed for scalar computation;
- do not use `0xC44FA0`, `0xC448F0`, `0xE0CD40`, `0xE0CF00`, or `0xF71050`
  for tactical engagement state or loss attribution. They are region/base
  table accessors or scalar aggregate writers;
- do not use the `0xF51060` subpath through `0xF51A60` and the `0x1004B*` /
  `0x1004C*` / `0x1004E70` helpers for ace participation, rich reinforcement
  reason, or loss attribution. Static body and RTTI evidence make them
  subunit/equipment helper routines;
- `0xC52840` remains an ace-management candidate, but it was not hit over the
  `77`-hour live window and the call block is reached through command id
  `0x2FB5`, so it is not the normal air-combat tick hook. Its remaining
  validation path is now exact: trigger command `0x2FB5` through dispatch block
  `0x6FA8AC` / callsite `0x6FA8BA` with an ace assignment/loss fixture and
  verify whether `rdx = r14` carries the command payload for ace assignment,
  ace loss, or air-combat participation. `rcx` is already resolved as the
  selected country strategic-air container returned by `0x6C2D80`.
- do not use `0xC52840` for sortie lifecycle, combat pairing, kill
  attribution, cause/source, tactical engagement state, or rich reinforcement
  reason. Its bounded body is an OOB/ace command handler and country ace-roster
  mutator, not an opposed-wing combat event or loss/reinforcement attribution
  path.

Remaining unblocked hook work:

- do not retry the phase bodies or worker chunks as direct event-payload
  hooks. They were reduced to worker-interior single-wing/region/mission
  helpers and scheduler/work-slice entries;
- ace-management `0xC52840` only under a controlled command `0x2FB5` ace
  assignment/loss fixture that hits dispatch block `0x6FA8AC` and callsite
  `0x6FA8BA`, then proves whether `r14` carries ace assignment, ace loss, or
  air-combat participation context;
- deeper interior/return instrumentation below the validated single-wing and
  region helpers remains open only for fields not rejected above. No compact
  sortie/combat/pairing/attribution record should be emitted from `0xF46A10`,
  `0xF4E0E0`, `0xF6B7B0`, `0xF49E80`, `0xC44FA0`, `0xC448F0`, `0xE0CD40`,
  `0xE0CF00`, `0xF71050`, `0xF51A60`, or the `0x1004B*`/`0x1004C*`/
  `0x1004E70` helper group.

## Current validated slice, not full completion

The integrated compact delta slice already has production-shaped proof in:

- `replay_native/runs/air_combat_losses_live_integrated_20260701_v1/native_session_manifest.json`;
- `replay_native/runs/air_combat_losses_live_integrated_20260701_v1/native_session.battleplan`;
- `replay_native/runs/air_combat_losses_live_integrated_20260701_v1/collector_stream.bin`.

That run scheduled `air_combat_losses_live`, advanced `96` game hours, saw `0`
skipped hours, emitted `97` domain data records and `97` domain perf records,
decoded an ordered compact stream, saw no source-unavailable, partial-data,
bridge-unavailable, or worker-panic flags, and packed a `.battleplan`.

This proof accepts only the strength/equipment delta slice. It does not close
sortie lifecycle, combat pairing, kill attribution, cause/source of loss,
tactical engagement state, ace participation, or rich reinforcement reason.
