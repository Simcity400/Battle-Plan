# A14 construction repair and exact-location RE closeout

Status: **PROMOTION READY** for `hoi4.exe 1.19.2.0.a729`, image base
`0x140000000`, SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

## Exact laws

- `CBuildingProductionLine+0x70 -> CBuilding` for type-59 building lines.
- `CBuilding+0x1D8 -> target-location object`; `target-location+0x6C` is the
  exact save `building.location`. Existing paired evidence matched 273/273
  construction rows (243 state locations, 30 province locations). Province
  targets join through the validated province-to-state topology; state targets
  are direct state ids.
- `CBuildingProductionLine+0x78` is the repair-level counter. `0` is ordinary
  construction; positive means the line is repairing a damaged building. The
  production update recomputes it as `CBuilding.level-CBuilding.healthy_level`.
- `CBuilding+0x48` is signed fixed-5 partial health for the next damaged level.
  The valid per-object range is `0..=10_000_000`. Repair adds to it; at
  `10_000_000` the engine clears it and increments `healthy_level`.
- Building-mutation reason codes are closed and exact:
  `0 none`, `1 bombed`, `2 sabotaged`, `3 collateral`, `4 repaired`,
  `5 scorched`. The repair mutator emits reason `4` on each completed repaired
  level. The reason argument is transient; current-state rows therefore emit
  exact repair state/progress and do not fabricate a historical last cause.

## Ghidra provenance

Relative artifact paths in the table resolve under
`C:\Users\Example\ghidra_agent_exports\<job-id>\`; this records the exact artifact
path without repeating the job-qualified root in every cell.

| Address / finding | Job ID | Artifact path | Manifest hash |
|---|---|---|---|
| string roots (`building_repair`, `BUILDING_REPAIR`, damaged-building terms) | `20260716T224923Z-a14-construction-repair-semantics-roots--a432f439` | `string-xrefs.txt` (SHA-256 `948589fec8c025f9143a470118a818da28ed55ab30c05244fa75164e4c2a341c`) | identity manifest `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe` |
| `0x140B58840` loads `building_bombed/sabotaged/collateral/repaired` ids; `0x140B58E10` names reason codes 1..5 | `20260716T225004Z-a14-building-repair-owner-functions-v1-e6e67673` | `evidence/func_140b58840.decompile.c`; `evidence/func_140b58e10.decompile.c`; `evidence/func_140b58e10.graph.txt` | `8463bb9d53b6fa290c7f2c0d35afcd6183c4aa3695f52a8a49c37d8dbdf863f7` |
| `0x1410C69F0` mutation dispatch; `0x141163C10` listener fan-out; `0x140B5D4C0` notification selection | `20260716T230052Z-a14-building-damage-reason-dispatch-v1-ace9a172` | `evidence/func_1410c69f0.decompile.c`; `evidence/func_141163c10.decompile.c`; `evidence/func_140b5d4c0.decompile.c` | `769d1439d6413fed0d291f19377aaedf83d3101f26d3ff9eb0ad7fc1fb224672` |
| `0x1410C5C90` repair accumulator; `0x1410C6D20` level damage; `0x1410C7000` fixed-point damage | `20260716T230239Z-a14-building-damage-repair-mutator-calle-69dfeb3b` | `evidence/func_1410c5c90.decompile.c`; `evidence/func_1410c6d20.decompile.c`; `evidence/func_1410c7000.decompile.c` | `fafa6b9108f2c468a69c65ac6bedf0dc24c82334466dbae1087387ea81588634` |
| `0x140F55590` production repair application and `+0x78` recomputation; `0x140240200` controlled health setter | `20260716T230425Z-a14-building-repair-input-producers-v1-f42e2ebb` | `evidence/func_140f55590.decompile.c`; `evidence/func_140240200.decompile.c` | `787f763843ddd9e95d2fb926daa6d2a2595c0019ff04ce04dc072426e5dd95e1` |

Incidental finding retained: `0x140B58E10` also maps reason `5` to
`BUILDING_SCORCHED`; only reasons 1..4 have the four cached notification ids at
owner offsets `+0x468/+0x470/+0x478/+0x480`.

## Production implementation

Compact `CBSD` is version 2. Construction rows append exact
`target_location_id`, typed target scope, and `repair_levels_remaining`.
Building rows append aggregate `partial_health_raw`. Unknown flags, invalid
scope/location joins, negative/out-of-range repair fields, incoherent healthy
state, and nonzero partial health on an undamaged row fail closed.

The recorder world fold exposes:

- queue: `target_location_id`, `target_scope`, `repair_levels_remaining`,
  `is_repair`;
- building: `partial_health_raw` alongside `level`, `healthy_level`, `damaged`.

## Mechanical verification

- lane: 26 library tests, 15 replay-pack tests, and 4 target-oracle tests pass;
- recorder-host: `cargo check` passes;
- recorder d48 fold: 7/7 tests pass;
- explicit production-fold assertion proves all new keys and the melt-anchored
  `partial_health_raw=8_184_501` value;
- collector: full `cargo check` passes (warnings only).
