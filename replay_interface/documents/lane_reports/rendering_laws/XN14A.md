# XN14A — Weather runtime, trigger, timing, and shipped composition

Status: **PROVEN — PROMOTION READY**

Pinned binary: `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

Scope: current a729 binary producer law plus the exact shipped weather asset/position/define chain. RenderDoc draw-corpus correlation is owned and frozen separately by XN14B.

## Promotion result

The extractor can promote XN14A without a reconstructed or visually tuned weather model:

1. Statically extract the exact reachable chain `weather kind -> weather_effect variant -> entity/fading_entity -> particle registration -> particle definition -> subsystem -> texture/shader`, all weather positions, and all eight camera/playback defines.
2. Extract the strategic-region weather record (`0x160` stride), its active airborne code, transition expiry, and the ordered selected entity pair per weather position whenever the code changes.
3. Record the ordered final submitted particle packets. This is the exact boundary because both entity-variant selection and particle simulation are seeded from a high-resolution platform counter; recomputing either from game date and weather state would not reproduce the game.

This is the exact capture contract.

## Exact runtime law


When the code changes, the producer destroys both members of every live pair, clears the vector, then iterates the strategic region's weather positions. A `big` position uses the code's big vector when non-empty; all other positions use the small vector. A wall-clock-seeded PRNG selects an entry modulo that vector's count. Each entry is the ordered pair `[entity, fading_entity]`, instantiated at the shipped XYZ position with unit scale.

For the render/view distance scalar `d` at context `+0x184`, the exact fading-entity alpha is:

- `0` for `d < 358`
- `(d - 358) / 220` for `358 <= d < 578`
- `1` for `578 <= d <= 1100`
- `(1500 - d) / 400` for `1100 < d <= 1500`
- `0` for `d > 1500`

The fading entity is enabled iff alpha is positive and receives alpha every update. The primary entity is toggled and assigned alpha only in the far-fade branch (`d > 1100`); the near-zoom branch leaves it unchanged.


The complete machine-readable law is [runtime_contract.json](../../../re_evidence/rendering_laws/XN14A/runtime_contract.json).

## Exact shipped census

Mechanical reachability from `common/weather.txt::weather_effects` proves:

- 5 airborne kinds, 9 variants, 17 entity roles, 17 unique entities, 17 particle events, 17 registrations, and 17 particle definitions;
- 10 unique textures across the reachable definitions;
- 588 weather positions across 287 strategic regions: 452 `big`, 136 `small`;
- no heuristic inclusions: mud, arctic water, vehicle snow tracks, and unrelated cloud/dust effects are excluded from the airborne chain.

Source identities:

| Source | SHA-256 |
|---|---|
| `common/weather.txt` | `38f43f8566d9a63e44ad38edff0f37aa4c09587f9adf5f11a908f67dc765a7f4` |
| `gfx/entities/weather_entities.asset` | `d55a0161f4835c6d3ff6f608d371e69087edf1685bf2de3b12e6881972ed1ee3` |
| `map/weatherpositions.txt` | `c532e511e3158e4786527f546f4180ea16652c5fe86164cc67bd6d4b3522409a` |
| `common/defines/00_graphics.lua` | `dfdab0a5fad7319d7b5ac524d98af49d3a0c1f95772e6b7a76abce54836f3d86` |
| `gfx/FX/particle.shader` | `75ea6b51a1941a5ca0b7cfbbfb6266b0b7e23f2a935f0bd5ef02cd757b192f14` |
| XN12A registry census | `c736085bbea45ba6688cccbaef3810b11d3d4590f435ed9ea6f1768f514d0b0c` |

The full joined rows, subsystem scalars, events, registrations, definitions, texture hashes, settings, defines, and positions are in [weather_asset_census.json](../../../re_evidence/rendering_laws/XN14A/weather_asset_census.json).

## Decisive current-binary provenance

| Address | Finding | Job ID | Artifact path | Artifact SHA-256 | Manifest SHA-256 |
|---|---|---|---|---|---|
| `0x14166ec50` | visual producer and exact formulas | `20260717T012439Z-xn14a-weather-region-getter-all-callers-e6227833` | `C:\Users\Example\ghidra_agent_exports\20260717T012439Z-xn14a-weather-region-getter-all-callers-e6227833\evidence\func_14166ec50.decompile.c` | `1c0de44d8d5507c6f8349785e0e7ceef93a1e184898c60ed0da4f2db7fb1cfec` | `fd99de197bf7699319a522e0d2e68dbd63a36d9a778ded8ca368597d9679d9c8` |
| `0x14166cec0` | per-update caller | `20260717T014238Z-xn14a-weather-visual-producer-callers-11da8748` | `C:\Users\Example\ghidra_agent_exports\20260717T014238Z-xn14a-weather-visual-producer-callers-11da8748\evidence\func_14166cec0.decompile.c` | `5c0f68e3e5115fcbf1db0bb0c2093942a06825936466ccebb70b74651b626698` | `0036f0cde8f3df959014d0bc5bc30e27395521688b59567da2cc3d58832d82d3` |
| `0x141b78fb0` | editor position refresh caller | `20260717T014238Z-xn14a-weather-visual-producer-callers-11da8748` | `C:\Users\Example\ghidra_agent_exports\20260717T014238Z-xn14a-weather-visual-producer-callers-11da8748\evidence\func_141b78fb0.decompile.c` | `d5a4b658d6455e6f80d996708777e2fdc3072eed50577a99f181586d8002daaa` | `0036f0cde8f3df959014d0bc5bc30e27395521688b59567da2cc3d58832d82d3` |
| `0x140f04e70` | validated `base + id*0x160` region getter | `20260717T015007Z-xn14a-weather-region-getter-exact-28f4775c` | `C:\Users\Example\ghidra_agent_exports\20260717T015007Z-xn14a-weather-region-getter-exact-28f4775c\evidence\func_140f04e70.decompile.c` | `39528b7ea9d33654b791842cd1cff111366398c58d3c1853497ecb7bb9847cc3` | `fee10fc4c60b173eb1974acdf53178cbe6cc8127b596cea8e529a60eac476508` |
| `0x140f0bd60` | exact region transition, flags, scalar, expiry | `20260717T010100Z-xn14a-weather-state-transition-workers-b3770842` | `C:\Users\Example\ghidra_agent_exports\20260717T010100Z-xn14a-weather-state-transition-workers-b3770842\evidence\func_140f0bd60.decompile.c` | `3d4727289f1f795a185deaf0f19179ac601653f04bf9138f2ec71c13e9d267e7` | `97ee87421ce17726a486eb4529cb7422cb70d2b1942fe927bee335c9f97929e8` |
| `0x1424dbe60` | high-resolution-counter seed source | `20260717T004246Z-xn12b-particle-prng-seed-init-min-14613a5d` | `C:\Users\Example\ghidra_agent_exports\20260717T004246Z-xn12b-particle-prng-seed-init-min-14613a5d\evidence\func_1424dbe60.decompile.c` | `9d7be4ddbb3488207c5ba1fc47b31d400a3878333458eeaa1490266b4b8c179c` | `faabb041c2eaa89bdc9a93a81535073aa5189d0e09a9322f46ea613b8b9ad1d5` |
| `0x140f04660` | region flag-to-token mapping | `20260717T011615Z-xn14a-weather-runtime-xref-consumers-2bdc1e55` | `C:\Users\Example\ghidra_agent_exports\20260717T011615Z-xn14a-weather-runtime-xref-consumers-2bdc1e55\evidence\func_140f04660.decompile.c` | `e11515c872793a2fbdf6e609122e87200e288663a6a745de4465ef7091c10c71` | `256f4fa83344ba6cdfcc1b368d37ca0a1a65f6190c2531214fd3af47b48661fc` |
| `0x140f054a0` | province-mud boundary | `20260717T011615Z-xn14a-weather-runtime-xref-consumers-2bdc1e55` | `C:\Users\Example\ghidra_agent_exports\20260717T011615Z-xn14a-weather-runtime-xref-consumers-2bdc1e55\evidence\func_140f054a0.decompile.c` | `0794af8a5ab95b935e2b84930ca3d3a7a57fd03842406aa01e7bd201afe27462` | `256f4fa83344ba6cdfcc1b368d37ca0a1a65f6190c2531214fd3af47b48661fc` |

Every supporting and incidental finding—including ground-raster, province water/snow/mud, define loaders, UI-only consumers, failed historical leads, and negative string/RTTI results—has its function/address, job ID, absolute artifact path, artifact hash, and manifest hash in [ghidra_provenance.json](../../../re_evidence/rendering_laws/XN14A/ghidra_provenance.json). This keeps the report readable without discarding evidence.

## Evidence and verification

- `build_xn14a_census.py` — reproducible exact-chain builder
- `weather_asset_census.json` — joined shipped-data census
- `runtime_contract.json` — extractor/replay contract
- `ghidra_provenance.json` — complete provenance ledger
- `verification.json` — mechanical verification result
- `manifest.json` — frozen evidence hashes

XN14A has no data blocker and no remaining promotion residual. Full XN14 promotion additionally requires the independently verified XN14B draw-corpus tuple.
