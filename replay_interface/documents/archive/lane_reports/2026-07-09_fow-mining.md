# FoW Mining Lane Report - C8

Worktree: `C:\Users\Example\hoi4_lanes\fow-mining`
Capture pair:

- FoW ON POV: `C:\Users\Example\hoi4_capture_corpus_preserved\session_WAVE1_CAP4_rounds\fow_on_pov_front`
- FoW OFF observer: `C:\Users\Example\hoi4_capture_corpus_preserved\session_WAVE1_CAP4_rounds\fow_off_observer_front`
- Counter/close reference checked: `C:\Users\Example\hoi4_capture_corpus_preserved\session_WAVE1_CAP4_rounds\counters_close`

## Carrier Identification

Terrain draw-set diff:

- `map_frame_bundle.json` has 15 terrain draws in each round.
- All 15 draws have the same draw signature by index: `DrawIndexed`, count `24576`, start `0`, PS FNV `ce5e5185d46e214b`, VS FNV `e5a0acbee3ae07a2`.
- No terrain draw signature is present only in FoW ON or only in FoW OFF.
- Every shared terrain draw has the same non-ShadowMap SRV hashes.
- Every shared terrain draw changes `ShadowMap` at PS slot 12.
- Every shared terrain draw has 3 constant-buffer hash deltas; scene CB0 itself differs only in animation-clock words plus one sub-ulp float, so it is not the FoW visual carrier for this pair.

Carrier:

| Field | FoW ON | FoW OFF |
|---|---:|---:|
| SRV name / slot | `ShadowMap` / PS slot 12 | `ShadowMap` / PS slot 12 |
| Dimensions | 1920x1080 | 1920x1080 |
| DXGI format | 87 (`B8G8R8A8_UNORM`) | 87 (`B8G8R8A8_UNORM`) |
| byte_len | 8,294,400 | 8,294,400 |
| FNV-1a64 | `35fa63fe96ea5217` | `4929cd2b85441701` |

DXGI 87 staging bytes are BGRA in memory, while HLSL samples RGBA. Therefore:

- byte 2 -> shader `.r` -> shadow factor
- byte 1 -> shader `.g` -> FoW color factor
- byte 0 -> shader `.b` -> FoW alpha factor
- byte 3 -> shader `.a` -> constant 1 in this pair

Channel diffs over 2,073,600 pixels:

| Shader channel | Captured byte | Meaning | Diff pixels |
|---|---:|---|---:|
| `.r` | byte 2 | shadow factor | 10,813 |
| `.g` | byte 1 | FoW color factor | 2,018,110 |
| `.b` | byte 0 | FoW alpha factor | 547,006 |
| `.a` | byte 3 | producer alpha | 0 |

FoW alpha nonzero pixels:

- FoW ON: 928,268
- FoW OFF: 925,461

The differing FoW texture is not `TerrainIDMap`, `ProvinceSecondaryColorMap`, `SnowMudData`, light maps, or gradient-border channels. For the terrain pass, the observed pair delta is the screen-space `ShadowMap` SRV.

## Shader Citations

Consumer path:

- `gfx/FX/pdxmap.shader:122-130` declares `ShadowMap` at index 12, linear/wrap, type `Shadow`.
- `gfx/FX/shadow.fxh:44-52` samples `ShadowMap` `.r` for scaled shadows.
- `gfx/FX/pdxmap.shader:359` uses `GetShadowScaled(..., ShadowMap)` for terrain shadowing.
- `gfx/FX/pdxmap.shader:388-389` samples `ApplyFOW(vOut, ShadowMap, Input.vScreenCoord)`, then blends back toward `vOut` by `BORDER_FOW_REMOVAL_FACTOR * (1 - vBloomAlpha)`.
- `gfx/FX/fow.fxh:85-88` samples the FoW texture and passes `FogValues.y` and `FogValues.z` to the closed-form `ApplyFOW`.

Producer path:

- `gfx/FX/pdxmap.shader:459-467` writes the screen-space payload as `float4(fShadowTerm, FogColorFactor, FogAlphaFactor, 1.0f)`.
- `gfx/FX/fow.fxh:63-76` computes `FogColorFactor` and `FogAlphaFactor`.
- `gfx/FX/fow.fxh:25-33` samples the visibility/intel and extra-fog inputs by world position.
- `gfx/FX/fow.fxh:43-60` computes animated noise transparency/color.

Actual `pdxmap.shader:466` producer bindings are `GetFogFactors(..., TerrainDiffuse, HeightNormal, TerrainColorTint)`. The function parameter names in `fow.fxh` are generic (`NoiseTex`, `ExtraFOW`, `IntelMapSampler`); the bound producer resources are those terrain SRVs.

## Law Statement

For each terrain pixel after terrain lighting and before post-FoW border removal:

Inputs:

- `lit_rgb`: terrain color after diffuse/specular/snow/city-light composition.
- `ShadowMap` sample at projected screen coordinate.
- `fog_color_factor = ShadowMap.g`.
- `fog_alpha_factor = ShadowMap.b`.
- `vBloomAlpha`: modified by `gradient_border_apply` and `secondary_color_mask`.
- `BORDER_FOW_REMOVAL_FACTOR = 0.8` from existing file-derived constants.

Captured DXGI 87 decode:

```text
shadow_factor    = ShadowMap_byte2 / 255
fog_color_factor = ShadowMap_byte1 / 255
fog_alpha_factor = ShadowMap_byte0 / 255
```

Final terrain FoW blend:

```text
dark_fow_rgb   = (0.08, 0.08, 0.10)     // fow.fxh:21
bright_fow_rgb = (0.98, 0.93, 0.93)     // fow.fxh:20
fog_rgb        = lerp(dark_fow_rgb, bright_fow_rgb, fog_color_factor)
fow_rgb        = lerp(lit_rgb, fog_rgb, fog_alpha_factor)
out_rgb        = lerp(fow_rgb, lit_rgb, 0.8 * (1 - vBloomAlpha))
```

There is no separate HSV/saturation curve in the cited shader path. The apparent darkening/desaturation is this RGB blend away from `lit_rgb` and toward `fog_rgb`.

Equivalent effective-alpha form:

```text
out_rgb = lerp(lit_rgb, fog_rgb, fog_alpha_factor * (1 - 0.8 * (1 - vBloomAlpha)))
```

Producer factor law from `fow.fxh`:

```text
intel       = IntelMapSampler.a(world_uv)
extra       = ExtraFOW.g(world_uv_flipped) * 2 - 1
noise_a     = smoothstep(0, 1.6, product_of_three_animated_noise_alpha_samples)
transparency= lerp(0.3, 0.9, noise_a)
noise_g     = animated_noise_green_sample
fow_color   = lerp(0.2, 1.0, noise_g)
color_factor= fow_color * transparency
alpha_factor= saturate(ToGamma(saturate(height_fog * transparency)) + extra) * intel * FOWFadeFactor
```

Residual: the capture pair contains the final `ShadowMap` payload, not enough evidence to regenerate the world-space visibility/intel producer input. That producer input must come from recording item 13 / extraction need A14.

## Numeric Gate

Implemented tooling-only module: `replay_interface/native/map_viewer/src/fow_law.rs`.

Focused gate:

```text
cargo test -p map_viewer gate_wave1_cap4_fow_pair_carrier_and_parity -- --nocapture
```

Result: 1 passed.

Gate output:

| Mask | Pixels | Half RGB exact | <=1 ULP | mean abs | max abs | p95 | p99 |
|---|---:|---:|---:|---:|---:|---:|---:|
| all pixels | 2,073,600 | 60.7626% | 67.6165% | 0.0010899061 | 0.21459961 | 0.005126953 | 0.013183594 |
| clear low-alpha pixels | 1,227,046 | 94.797% | 97.503% | 0.00037328096 | 0.21459961 | 0.000061035156 | 0.00048828125 |
| changed/fogged pixels | 108,518 | 2.109% | 6.476% | 0.0058150315 | 0.03918457 | 0.016601563 | 0.022949219 |
| adversarial forced full-fog on clear pixels | 1,227,046 | 0.0000815% | 0.000353% | 0.14353476 | 0.31762695 | 0.16503906 | 0.17315674 |

Interpretation:

- Clear low-alpha pixels are near-byte-stable when the law is applied through the captured ShadowMap factors.
- Forced fog on clear pixels fails hard, so the law discriminates rather than matching everything.
- Changed/fogged pixels are not byte-claimed. Missing evidence is the exact pre-FoW base color, per-pixel `vBloomAlpha`, and same-clock producer inputs. No fitted scalar was adopted.

## Hidden Unit / Counter Evidence

Result: UNKNOWN, not proven from this capture pair.

Evidence:

- `fow_on_pov_front` and `fow_off_observer_front` round dirs contain terrain/water/rivers/trees bundles only.
- Terrain draw sets contain no added/removed draw signatures.
- Root `ui_pass_bundle_*.json` files in `<local-path>` predate the fow_on/off round markers in the session event log.
- `counters_close` is a partial/budget-stopped round; its `map_capture_session.json` records that active terrain capture exceeded budget mid-round, and it does not provide a labelled paired counter/UI draw sequence for FoW ON vs OFF.

Required follow-up:

- Same-camera/same-clock FoW ON vs observer OFF capture with a labelled unit/counter/UI pass and hidden enemy units in view.

## Files Changed

- `replay_interface/native/map_viewer/src/fow_law.rs`
- `replay_interface/native/map_viewer/src/lib.rs`
- `replay_interface/documents/FIDELITY_LEDGER.md`
- `replay_interface/documents/REPLAY_PRODUCTION_PLAN.md`
- `replay_interface/documents/REPLAY_DATA_REQUIREMENTS.md`
- `replay_interface/documents/EXTRACTION_NEEDS.md`

No extractor files or preserved-corpus files were modified.
