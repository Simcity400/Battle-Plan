# XN15A Air-Combat and Sunk-Convoy Semantic Trigger Mapping

Status: **PROVEN / PROMOTION READY** for `hoi4.exe 1.19.2.0.a729`, image
base `0x140000000`, SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

## Exact disposition

The A4R1 air-region combat statistics and SCH1 sunk-convoy history are exact
semantic/accounting streams. Neither is an effect-spawn stream. Production may
use their snapshot-to-snapshot deltas as annotations and validation facts, but
**must not create, choose, position, count, or time a visual effect from either
stream**.

This is a closed negative law, not an open RE residual. The engine records no
effect registration token, effect-instance id, lifecycle sequence, authored
event index, parent entity/node, world transform, RNG state, animation phase, or
original effect tick in either producer. The XN15 direct lifecycle/presentation
record remains the sole authority for visual replay.

## A4R1 air semantic producer

A4R1 is a full retained snapshot. Its stable row identity is:

`{owner_tag_db_id, region_id, side_vector, wing_id, date_raw,
classification_flag}`.

The row value carries `selection_flag`, `copied_flag`, `plane_count`,
`mission_mask`, `attributed_country_tag_db_id`, and damage entries keyed by
`{channel, cause_type}` with signed `amount_raw`. A replay extractor may emit an
`air_combat_stat_delta` by comparing consecutive validated full snapshots. The
delta's timestamp is the observation/update boundary; it is not the original
combat or particle-spawn tick.

Cause types 4, 5, 9, and 10 exactly distinguish planes lost to enemy planes,
anti-air, en-route interception, and armored trains. Types 0, 1, 2, 3, 6, and
7 are aggregate damage/disruption facts; type 8 is unused. These enums do not
select a particle token. In particular:

- do not emit one effect per `amount_raw / 100000`;
- do not place an effect at a strategic-region centroid;
- do not treat `wing_id` or `attributed_country_tag_db_id` as an effect parent;
- do not map a loss cause to a carrier-airplane animation event.

## SCH1 sunk-convoy semantic producer

SCH1 domain 70 is a full global ledger. Its stable row identity is
`{month, killer_country_tag_db_id, owner_tag_db_id}` and its value is
`convoys`. A replay extractor may emit a `sunk_convoy_ledger_delta` from
consecutive validated snapshots. That delta records only a newly observed
change in the retained monthly total.

SCH1 contains no sea zone, province, world position, source convoy/ship entity,
loss timestamp within the month, effect token, effect instance, animation
state, or RNG. Production must therefore not infer one explosion per convoy,
place an effect in an arbitrary sea zone, or reuse a raid result effect for a
sunk-convoy row.

## Installed game-file findings

The installed effective files contain real visual candidates, but no link from
either retained statistics root to those candidates:

| Finding | Exact evidence | Disposition |
|---|---|---|
| Naval raid outcome effects | `gfx/entities/particles.gfx` lines 946-961 registers `raid_naval_limited_success_effect`, `raid_naval_success_effect`, and `raid_naval_critical_success_effect` to `raid_naval_file` at scales 1/2/3. `gfx/entities/particle_entities.asset` lines 297-328 binds them to the three `air_raids_naval_*_success_entity` attack states at authored time 0. | These are air-raid result entities, not SCH1 monthly sunk-convoy rows. No exact mapping is present. |
| Carrier airplane death | `gfx/entities/particles.gfx` lines 332-341 registers `ship_carrier_airplane_death_particle` and its water-impact companion. `gfx/entities/units_ships.asset` lines 414-524 binds them to authored carrier animation times and locator nodes. | This is entity-animation presentation on carrier models, not an A4R1 strategic air-loss trigger. |
| Ship explosion | `gfx/entities/particles.gfx` lines 272-276 registers `ship_explosion_particle`; `gfx/entities/units_ships.asset` contains authored ship-state events at exact nodes/times. | This is ship entity presentation, not a derivable SCH1 event. |

File identities:

- `gfx/entities/particles.gfx`: SHA-256
  `af261d416accb89c4fd222b41e2846bee8dc1d20def37b7b232c67ddb8d50d35`;
- `gfx/entities/particle_entities.asset`: SHA-256
  `4c0bc57d3f23c5e04f0f6e98952a386ebeec8a512503f12fee7243ac19a25c56`;
- `gfx/entities/units_ships.asset`: SHA-256
  `48614cccf562d1b2597283bc223ec5c0cd9507e545f0251858c73dfccad66920`;
- `gfx/particles/environment/raid_naval.asset`: SHA-256
  `89bec60f7ad0563bb491a3385adc27a4ad5ad6b254f4192a9bb58e29d0780152`;
- `gfx/particles/ship_carrier_airplane_death.asset`: SHA-256
  `db426b45f6283ba93fbd3fcbe31e85a6435f8eabb336c093792ae29093884e86`.

An installed-tree search over `.asset`, `.gfx`, `.gui`, `.txt`, `.lua`,
`.shader`, `.effect`, `.json`, and `.yml` files found the three `raid_naval_*`
tokens only in their registrations and the three raid presentation entities.
It found the carrier-airplane token only in its registration and authored ship
entity timelines, including DLC variants. This closes the static game-file
side without claiming that the effects themselves do not exist.

## Production mapping

1. Preserve A4R1 and SCH1 as exact full snapshots.
2. If desired, derive the two named semantic deltas using stable keys and
   before/after values. Never reinterpret the observation boundary as event
   time.
3. Spawn or advance a visual only from an XN15 `effect_lifecycle_event` or
   `entity_presentation_event` carrying its exact sequence/update, effect or
   entity instance, registration/definition identity, authored event index,
   anchor/transform, phase, and RNG/runtime state.
4. Attach an A4R1/SCH1 delta as metadata only when an independent recorded
   correlation key exists. Temporal proximity alone is not a correlation key.
5. Fail closed when the lifecycle/presentation record is absent. Do not fall
   back to region centers, month boundaries, candidate particle names, or
   quantity-derived effect counts.

The machine-readable contract is
`replay_interface/re_evidence/rendering_laws/XN15A/semantic_producer_map.json`.

## Ghidra evidence ledger

Every executable-derived claim above is pinned to the binary identity at the
top. Artifact paths are relative to the named broker job.

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

## Mechanical verification

`verify_xn15a.py` requires the exact binary identity, producer keys, no-inference
rules, direct-XN15 authority fields, installed-file hashes, and every Ghidra
job/manifest above. Its frozen result is `verification.json`.
