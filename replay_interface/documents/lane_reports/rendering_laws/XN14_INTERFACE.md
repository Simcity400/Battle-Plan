# XN14 weather interface slice

Status: **PROVEN - FROZEN**

This immutable extractor-to-replay weather interface is independent of final XN15. Its machine contract is [weather_interface_contract.json](../../../re_evidence/rendering_laws/XN14C/weather_interface_contract.json).

## Exact binding

The state row carries the six strategic-region flags, `region_value_i64_fixed_1e5`, signed absolute `transition_expiry_game_hour_i32`, `airborne_runtime_code_u8`, and `weather_effect_token_or_null`. Runtime code zero means no airborne effect: the token is null and every ordered position is `explicit_empty`, with null authored index, variant token, primary entity ID and fading entity ID, plus an empty ordered effect-ID array.

Codes 2, 3, 4, 5 and 7 map to `rain_light`, `rain_heavy`, `snow`, `blizzard` and `sandstorm` after producer priority. Each position still has its own resolution. A big position uses its big vector when nonempty and otherwise selects the small vector; a small position selects the small vector. An empty selected small vector does not fall back to big and yields explicit-empty membership without consuming a PRNG draw. Shipped sandstorm has no small entry, so every small sandstorm position is explicit-empty. Instantiated rows carry the selected authored index, resolved variant, primary entity instance, nullable fading entity instance and ordered effect instance IDs. Those IDs, not texture identity, are replay join keys.

There is no airborne intensity field and no weather-state interpolation field. Code change is destroy-clear-recreate.

## Decisive provenance

| Function address | Finding | Job ID | Artifact path | Artifact SHA-256 | Manifest SHA-256 |
|---|---|---|---|---|---|
| `0x14166ec50` | dispatch, code zero, pair lifecycle, position selection | `20260717T012439Z-xn14a-weather-region-getter-all-callers-e6227833` | `C:\Users\Example\ghidra_agent_exports\20260717T012439Z-xn14a-weather-region-getter-all-callers-e6227833\evidence\func_14166ec50.decompile.c` | `1c0de44d8d5507c6f8349785e0e7ceef93a1e184898c60ed0da4f2db7fb1cfec` | `fd99de197bf7699319a522e0d2e68dbd63a36d9a778ded8ca368597d9679d9c8` |
| `0x140f0bd60` | flags, fixed-point value, expiry hour | `20260717T010100Z-xn14a-weather-state-transition-workers-b3770842` | `C:\Users\Example\ghidra_agent_exports\20260717T010100Z-xn14a-weather-state-transition-workers-b3770842\evidence\func_140f0bd60.decompile.c` | `3d4727289f1f795a185deaf0f19179ac601653f04bf9138f2ec71c13e9d267e7` | `97ee87421ce17726a486eb4529cb7422cb70d2b1942fe927bee335c9f97929e8` |

`verify_weather_interface.py` mechanically freezes the contract, report and verifier hashes.
