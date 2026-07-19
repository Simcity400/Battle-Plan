# A22 air-loss channel closeout

Status: **PROVEN / IMPLEMENTED / PROMOTION-READY**

Pinned executable: `hoi4.exe` 1.19.2.0.a729, image base `0x140000000`, SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

## Ghidra evidence

| Function address | Data address | Job ID | Artifact path | Manifest hash | Artifact hash |
|---|---|---|---|---|---|
| N/A (static data) | `0x1429f8940` | `20260716T225128Z-a22-air-loss-channel-table-v1-4ae5c588` | `C:\Users\Example\ghidra_agent_exports\20260716T225128Z-a22-air-loss-channel-table-v1-4ae5c588\datdump.txt` | `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe` | `2b4183394c1cb9246a40fd6641cb5ace529f7a71762adbda05a5a4770e9afcb0` |
| `0x1418738e0` | `0x1429f8940` | `20260716T225144Z-a22-air-loss-ui-builder-v1-504d5b30` | `C:\Users\Example\ghidra_agent_exports\20260716T225144Z-a22-air-loss-ui-builder-v1-504d5b30\evidence\func_1418738e0.decompile.c` | `b7dd54a001522d85208e4d87f87eb07ef337d37b75ab25023ffaa50591d21bf6` | `b6faba2489c834d295c48940118dc1d6b06e219ac134b9ec4acbb3c8d3c9e1fc` |
| `0x141874160` | `0x1429f8940` | `20260716T225144Z-a22-air-loss-ui-builder-v1-504d5b30` | `C:\Users\Example\ghidra_agent_exports\20260716T225144Z-a22-air-loss-ui-builder-v1-504d5b30\evidence\func_141874160.decompile.c` | `b7dd54a001522d85208e4d87f87eb07ef337d37b75ab25023ffaa50591d21bf6` | `9d148b692ae7b5d5786552a245d1afa04010e09974ee1be449bcbea805216630` |

Function-evidence manifest:
`C:\Users\Example\ghidra_agent_exports\20260716T225144Z-a22-air-loss-ui-builder-v1-504d5b30\evidence\_FUNCTION_EVIDENCE_MANIFEST.json`.

The static `u32[24]` table is exactly:

```text
0, 1, 2, 3, 10, 17, 24, 39, 36, 37, 38, 40,
41, 42, 43, 44, 51, 58, 65, 80, 77, 78, 79, 81
```

The enemy half is the exact +41 mirror of the friendly half. All selected fields are below the
alias-prone aggregate tail at fields 82..87.

## Game-file semantic proof

The pinned installed game executable was independently hash-checked at
`<local-path> of Iron IV\hoi4.exe`. English localisation
`<local-path> of Iron IV\localisation\english\core_l_english.yml:722`
through line 745 (SHA-256
`2bdc0a8f6db927e5db88b6e9ccd6cdf56bf5224a7ff8da0a1de92935e15d5e77`) defines
`AIRVIEW_DETAILS_CHANNEL_1..24` in the same order. Combining that
ordered key set with the engine table yields:

| UI channel | Field | Label | Display divisor |
|---:|---:|---|---:|
| 1 | 0 | `our_buildings_bombed` | 1,000 |
| 2 | 1 | `our_troops_bombed` | 100,000 |
| 3 | 2 | `our_ships_bombed` | 100,000 |
| 4 | 3 | `our_fighters_lost` | 100,000 |
| 5 | 10 | `our_bombers_lost` | 100,000 |
| 6 | 17 | `our_cas_lost` | 100,000 |
| 7 | 24 | `our_transports_lost` | 100,000 |
| 8 | 39 | `our_bombers_disrupted` | 100,000 |
| 9 | 36 | `our_trains_lost` | 100,000 |
| 10 | 37 | `our_trucks_lost` | 100,000 |
| 11 | 38 | `our_railway_guns_lost` | 100,000 |
| 12 | 40 | `our_missiles_lost` | 100,000 |
| 13 | 41 | `enemy_buildings_bombed` | 1,000 |
| 14 | 42 | `enemy_troops_bombed` | 100,000 |
| 15 | 43 | `enemy_ships_sunk` | 100,000 |
| 16 | 44 | `enemy_fighters_lost` | 100,000 |
| 17 | 51 | `enemy_bombers_lost` | 100,000 |
| 18 | 58 | `enemy_cas_lost` | 100,000 |
| 19 | 65 | `enemy_transports_lost` | 100,000 |
| 20 | 80 | `enemy_bombers_disrupted` | 100,000 |
| 21 | 77 | `enemy_trains_lost` | 100,000 |
| 22 | 78 | `enemy_trucks_lost` | 100,000 |
| 23 | 79 | `enemy_railway_guns_lost` | 100,000 |
| 24 | 81 | `enemy_missiles_lost` | 100,000 |

The `R0/R1/G0/G1` format suffixes corroborate the displayed precision. The engine builder proves
that buildings fields 0 and 41 use divisor 1,000; all other selected fields use 100,000. Raw
values remain untouched in the replay payload and recorder output.

Useful side finding: several non-English localisation files contain stale/mistranslated channel
10/11 text, while their ordered keys and formatting flags remain intact. Stable extractor labels
therefore come from the pinned engine order plus the English canonical key meanings, not from an
arbitrary translated string.

## Production changes

- Domain-61 payload schema bumped from v1 to v2.
- Row widened from six channels / 64 bytes to 24 channels / 208 bytes.
- Both live readers now fail the whole row closed if any of the 24 reads is unavailable,
  negative, or outside the existing raw bound.
- Strict decoder, gate totals, recorder world fold, and summary decoder consume all 24 channels.
- Recorder output emits `channels`, exact `raw`, and per-label `display_divisors` maps.

## Mechanical verification

```text
cargo test --manifest-path extractor\native\lanes\air_loss_stats_live\Cargo.toml
18 passed; 0 failed

cargo test --manifest-path extractor\native\recorder_host\Cargo.toml air_loss_stats -- --nocapture
3 passed; 0 failed (2 world-fold tests, 1 summary-decoder test)

cargo test --manifest-path extractor\native\collector\Cargo.toml air_loss_stats -- --nocapture
collector and dependencies compiled successfully; 0 matching tests, 0 failures
```

No A22 semantic or implementation residual remains.
