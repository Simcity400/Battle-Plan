# A4/A21 Air Combat Promotion Report

Status: **PROMOTION READY** for
`hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

This report closes the reverse-engineering questions in A4 and A21. The engine
stores exact country/region/date/mission combat history and typed damage causes.
It does not store a persistent wing-versus-wing combat pairing or a
reinforcement-source enum in this root. Production must preserve the exact
stored facts and must not synthesize either absent field.

## Promotable root

The existing validated root remains:

`CGameState+0x690 -> StrategicAirManager`

`StrategicAirManager+0x30` is the country-container pointer vector and `+0x3C`
is its count. Each validated country container has:

- owner tag-db-id at `+0x90`;
- indexed `CAirRegionCombatData*` table at `+0x170`, capacity at `+0x178`, and
  count at `+0x17C`;
- null slots for regions without retained statistics.

`CAirRegionCombatData` has vtable `module+0x2942188`, extent `0x40`, and owner
tag-db-id at `+0x38`. It contains two inline vectors of
`SAirWingCombatData` records:

- vector 0 header at `+0x08`, count at `+0x14`;
- vector 1 header at `+0x20`, count at `+0x2C`.

The accessor `0x140C407E0` indexes the country container's `+0x170` table by
region id, lazily allocates a `0x40`-byte region object, initializes both
vectors, and copies the country tag from container `+0x90` to region `+0x38`.
The serializer path at `0x140C51F00` independently proves the indexed-vector
law: the parallel `+0x158` table uses capacity/count at `+0x160/+0x164`, and the
combat-table branch dispatches through `0x140C407E0`.

## `SAirWingCombatData` exact layout

Vtable: `module+0x2A099E0`. Record stride/extent: `0x98`.

| Offset | Exact stored meaning |
|---:|---|
| `+0x08` | exact wing id: writer wrapper `+0x1C` equals the validated wing body at wrapper `+0x10`, body wing-id field `+0x0C` |
| `+0x0C` | `CGameDate` raw value; record matching uses current `CGameState+0x468` |
| `+0x10` | maximum plane-count/denominator snapshot copied from producer `+0x7C` |
| `+0x14` | accumulated mission mask, OR-ed from producer `+0xA0` |
| `+0x18` | selection flag used by the two UI aggregate destinations |
| `+0x19` | classification flag; part of record identity and UI filtering |
| `+0x20` | `SDamageEntry` vector for damage types `0,1,2,6,7`; count `+0x2C` |
| `+0x38` | `SDamageEntry` vector for types `3,4,5,9,10`; count `+0x44` |
| `+0x50` | attributed country tag-db-id returned by `0x140F45950` |
| `+0x58` | serialized statistics/mission-efficiency block |
| `+0x60` | copied producer vector, `0x18` stride, refreshed when the plane-count maximum advances |
| `+0x78` | copied producer vector, `0x10` stride, refreshed on the same condition |
| `+0x90` | copied producer boolean from `+0x210` |

Important correction: the archived hypothesis that record `+0x10/+0x14` formed
a country pair is false. Writer `0x14194F590` proves `+0x10` is a plane-count
snapshot and `+0x14` is the mission mask.

`0x141950340` selects vector 0 or 1 by comparing the writer's country tag with
`CAirRegionCombatData+0x38`, then matches/creates records by current date,
wing id, and classification flag. `0x14194FC90` scans newest to oldest,
breaks at the requested date window, filters mission mask/classification, and
aggregates damage. `0x14194FEC0` retains the maximum `+0x10` record for each
distinct wing id and uses `+0x18` to select the result bucket. This is the
exact persistent lifecycle law behind the UI's last-sortie and last-day views.

It is a retained region/country/date/mission aggregate, not a tactical event
object. No member is a direct opponent-wing pointer or wing-versus-wing pair.

## Damage and loss cause enum

`SDamageEntry` has vtable `module+0x2A09990`, stride `0x18`:

- `+0x08`: cause type (`u32`);
- `+0x10`: amount (`i64`, fixed `100000` per plane/count where applicable).

Writer `0x14194F590` and UI label selectors `0x14186C4E0` / `0x1418698B0`
prove the complete mapping:

| Type | Exact UI meaning |
|---:|---|
| 0 | buildings bombed |
| 1 | ships bombed |
| 2 | damage to divisions |
| 3 | disrupted planes |
| 4 | enemy planes shot down / friendly planes lost to enemy planes |
| 5 | planes downed by anti-air / friendly planes lost to anti-air |
| 6 | logistics bombed |
| 7 | railway guns bombed |
| 8 | intentionally not stored or displayed by this writer switch |
| 9 | planes shot down en route to target |
| 10 | planes downed by armored trains / lost to enemy armored trains |

For type 3 the writer clamps the amount to `plane_count * 100000`. The English
labels are independently present in game file
`localisation/english/air_l_english.yml` at lines 356-379.

## Pairing, killer, and reinforcement conclusions

- Exact persistent pairing is country-container + region id + side vector +
  date + wing id + classification + mission mask. The root contains no
  killer-wing object identity. Production therefore exposes the stored keys and
  does not invent wing-versus-wing pairings.
- Exact cause-class attribution is stored through types 4, 5, 9, and 10.
- Country-level dealer/victim attribution remains available through the already
  proven `NCombatLog::CStatsObserver+0x38` `SDamageDone` rows. Ace-death killer
  identity remains separately proven at `CAceEntry+0x164/+0x168`.
- Reinforcement reason is absent as a stored enum. The exact production facts
  are wing identity, count/equipment delta, variant, mission/region, and
  country production/stockpile context. Labels such as “from production” or
  “from transfer” are not exact and must not be emitted.

This absence is a closed schema disposition, not an open RE residual: the
promoted contract represents every stored fact relevant to the requested
question and explicitly omits a field the engine does not retain.

## Convoy/sunk variant

A21's convoy variant is already statically closed by the naval root:

- `CGameState+0x5A8` pointer vector, count `+0x5B4`;
- each `CSunkConvoyInfo*`, vtable `0x14295D008`, extent `0x18`;
- `+0x08` month, `+0x0C` convoy count, `+0x10` killer tag handle/id,
  `+0x14` owner tag handle/id;
- serializer `0x1401F2A20`, field writes around
  `0x1401F490E..0x1401F4951`.

This is a global sunk-convoy history root and is distinct from the existing
per-ship cumulative `CShip+0x3C` counter. It belongs in the naval production
domain rather than being relabelled as an air-region record.

Production implements that ownership decision as dedicated domain 70
`sunk_convoy_history_live`, not a d49 adjunct. Its versioned `SCH1` full-snapshot
payload carries exact `{month, convoys, killer_country_tag_db_id,
owner_tag_db_id}` rows. The collector double-reads the vector header and each
row, gates every object on vtable `module+0x295D008`, requires exact bounded
values, sorts by month/killer/owner, and rejects duplicate identities. The
recorder folds the rows into `world_events.sunk_convoys` with country-id
resolution. Domain 70 is registered as a production-profile `validated_slice`.

## Production admission law

The d42 reader validates all roots and vectors before emission:

- pinned manager, country-container, region-object, record, and damage-entry
  vtables;
- vector `count <= capacity`, bounded table/record/damage counts, heap pointers
  for non-empty vectors;
- region owner equals country-container owner;
- boolean fields are `0/1`, plane counts and amounts are bounded, cause type is
  `0..10`.

Any failure rejects the regional extension rather than emitting guessed or
partially trusted attribution. A versioned `A4R1` trailer follows the legacy
d42 wing-delta rows and contains full regional state plus flattened damage
entries.

Mechanical verification on 2026-07-17:

- `cargo check -p collector -p recorder_host`: passed;
- full `compact_contract` suite: passed (117/117), including registry/profile
  synchronization and `SCH1` round-trip/corruption gates (3/3);
- `sunk_convoy_history_summary_accepts_exact_sch1_payload`: passed (1/1);
- `chunk_folds_exact_sunk_convoy_history_into_world_events`: passed (1/1);
- `d42_summary_accepts_exact_air_region_cause_trailer`: passed (1/1);
- `chunk2_folds_d42_exact_air_region_combat_causes`: passed (1/1);
- legacy compatibility `chunk2_folds_air_sections_and_keeps_d42_out_of_events`:
  passed (1/1).

The collector recovery merge is complete. The final localized collector changes
correct d42 record `+0x08` from the rejected country-tag interpretation to exact
`wing_id` validation and add only the dedicated d70 vector walk/scheduler. The
recovered d57 and building-registry overlays were preserved; the combined
collector compiles cleanly and the building-registry contract tests pass.

## Ghidra evidence ledger

Every function claim above is pinned to the executable identity at the top.

| Function/address | Job ID | Artifact path | Manifest/artifact SHA-256 |
|---|---|---|---|
| `0x140C407E0`, `0x140C40870` | `20260716T230508Z-d42-airregionstatistics-root-accessors-v-293a7598` | `evidence/func_140c407e0.decompile.c`, `evidence/_FUNCTION_EVIDENCE_MANIFEST.json` | manifest `cc37cbd969dac08822427b3dd28750f20d766a9d356b2fe01e152a6212ff1fe3` |
| `0x140C51F00` | `20260716T230508Z-d42-airregionstatistics-root-accessors-v-293a7598` | `evidence/func_140c51f00.decompile.c` | manifest `cc37cbd969dac08822427b3dd28750f20d766a9d356b2fe01e152a6212ff1fe3` |
| `0x141950340`, `0x14194F590` | `20260716T225947Z-d42-airregionstatistics-enum-and-damagee-adfdde5a` | `evidence/func_141950340.decompile.c`, `evidence/func_14194f590.decompile.c` | manifest `9e085a96163dfea76f644978faafe552bc7301e4bc064aa0aadd63b93320969b` |
| `0x14194FC90`, `0x14194FEC0` | `20260716T230700Z-d42-airregionstatistics-ui-aggregation-v-73ee5b05` | `evidence/func_14194fc90.decompile.c`, `evidence/func_14194fec0.decompile.c` | manifest `e2938dcecb969b4098e12fab62ca21d473c9985025703f4a8db7fcd71f9a5377` |
| `0x14194F3F0`, `0x141951050`, `0x141950B60` | `20260716T225313Z-d42-sairwingcombatdata-vmethods-v1-7f2222eb` | `evidence/func_14194f3f0.decompile.c`, `evidence/func_141951050.decompile.c`, `evidence/func_141950b60.decompile.c` | manifest `21c14fcd1fbcdb8ff36079265d1bc71e4998c24a85ed41468686c9ed7e12656f` |
| `SAirWingCombatData` / `SDamageEntry` vtables | `20260716T225206Z-d42-sairwingcombatdata-vtable-neighborho-8aed8a8a` | `datdump.txt` | artifact `6a1e03f4edcec36dde9d4318193a0e03c0eb754d7c9e6c8137d787c42e418cb9`; identity manifest `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe` |
| `0x14186C4E0`, `0x1418698B0` | `20260716T230340Z-d42-airview-stat-enum-label-map-v1-afd406db` | `evidence/func_14186c4e0.decompile.c`, `evidence/func_1418698b0.decompile.c` | manifest `f1e7c0bcadbd9dc9a0c1634958d3c458a15bfa26a93ab3c5d761868dd6d57bd6` |
| `0x1401F2A20`, `0x1401CAE90`, `0x1401D7480`, `0x1401D09B0` | `20260717T000231Z-d42-a21-sunk-convoy-history-root-v1-aa5d3fab` | `evidence/func_1401f2a20.decompile.c`, `evidence/func_1401cae90.decompile.c`, `evidence/func_1401d7480.decompile.c`, `evidence/func_1401d09b0.decompile.c`, `evidence/_FUNCTION_EVIDENCE_MANIFEST.json` | manifest `236a3635de7b081f71fe03b83628613fd9b6fdca277510abcff1364838d450ab` |
