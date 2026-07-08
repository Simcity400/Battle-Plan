# WATER_LAW — water reflection / refraction / glitter (WATER1)

Mechanism-correct laws for the game water surface, derived **files-first** from the
shipped installed content (via the mod-aware `GameFileResolver`) and validated against
captured evidence. `--water-full` is now **DEFAULT ON** in `replay_app` (GRAD1
graduated it): the real reflection cube + the mechanism-correct underwater
refraction pass render by default.

**Default policy (GRAD1).** The old capture-anchored water default (mean-color
reflection flatten + the `TERRAIN_COLOR_SNAPSHOT_HLSL` approximation refraction)
is reachable with the explicit opt-out `--no-water-full` (byte-identical to the
pre-GRAD1 default; see FIDELITY_LEDGER equivalence gate B). Fail-open vs
fail-closed (the reflection cube needs no resolver; the underwater refraction
pass reconstructs `pdxmap` `underwater` through the resolver, so the resolver is
the gate):

- **Explicit `--water-full`** — a missing `--game`/resolver is a HARD ERROR
  (fail closed; no silent fallback to the approximation). Unchanged from the
  pre-GRAD1 flag.
- **Default (no flag)** — if no resolver is available, the renderer logs a
  one-line downgrade NOTICE and falls back to the old capture-anchored water
  path (both cube and refraction revert, keeping the render byte-identical to
  the old default rather than erroring). The decision lives in
  `render.rs::CapturedPassLayer::new`: `water_full` is forced false iff
  `water_full && game_resolver.is_none() && !water_full_explicit`, before
  `set_water_full` / `with_underwater_resolver`.
- **`--no-water-full`** — old default, unconditionally (approximation, never
  errors on a missing resolver).

Shipped sources of truth (game root
`<local-path> of Iron IV\`):
`gfx/FX/pdxwater.shader`, `gfx/FX/pdxmap.shader`,
`gfx/FX/standardfuncsgfx.fxh`, `gfx/FX/constants.fxh`,
`common/defines/00_graphics.lua`, `map/terrain/reflection.dds`.

---

## 1. Reflection cube — SHIPPED FILE (not engine-generated)

**Shader:** `pdxwater.shader` declares `ReflectionCubeMap` at sampler slot 8,
`Type = "Cube"` (lines 86–95), and samples it as
`reflectiveColor = texCUBE( ReflectionCubeMap, reflection ).rgb` (line 326),
where `reflection = reflect( vEyeDir, normal )` (line 319). Fresnel then blends
reflection over refraction (lines 334–337).

**Verdict: the reflection cube is a STATIC, SHIPPED, MOD-RESOLVABLE file —
`map/terrain/reflection.dds`.** This overturns the previous premise (in the old
`d3d11_device.rs` comment) that "the game re-renders the reflection cube around
the live camera every frame."

Proof (measured by WATER1, `live_PAN2` capture):

| property | captured `ReflectionCubeMap` | shipped `map/terrain/reflection.dds` |
|---|---|---|
| dims | 256×256 | 256×256 |
| faces | 6 (array_size 6, `MiscFlags` `TEXTURECUBE`) | 6 (`DDSCAPS2_CUBEMAP`, all faces) |
| mips | 9 | 9 |
| format | DXGI 91 = `B8G8R8A8_UNORM_SRGB` | uncompressed BGRA8 (pf flags `0x41`) |
| payload | 2,112,768 B (D3D11 row-aligned) | 2,097,144 B (tightly packed) + 128 B DDS header |

- **All 54 subresources (6 faces × 9 mips) are byte-identical** between the
  captured cube and the shipped DDS (compared per-subresource, stripping the
  D3D11 row-pitch padding; face 0 mip 0 matches with zero mean-abs-diff and no
  channel swap). The only difference is memory row-alignment (D3D11 pads each
  mip's row pitch; the DDS is tightly packed).
- `constants.fxh:96` `FAKE_CUBEMAP_COLOR = float3(0,0,0)` is only a black
  fallback constant used by other shader paths — **not** evidence that no
  cubemap file exists.
- `bundle_gen.rs` **already** resolves `map/terrain/reflection.dds` via the
  mod-aware resolver (role `water_reflection_cubemap`, sampler
  `ReflectionCubeMap`) and copies it into the map bundle, so a mod that ships its
  own `reflection.dds` is honored automatically.

**Implementation (`--water-full`):** the world-synthesis water pass uploads the
REAL 256²×6×9-mip cube via the generic, validated texture path
(`native_d3d11_texture_upload` + `native_d3d11_validate_texture_payload_shape`,
which fails closed on any dims/mips/faces/coverage mismatch) instead of the
1×1×6 mean-color flatten. Because the captured cube payload is byte-identical to
the resolver-resolved `reflection.dds`, uploading the captured payload is
files-first-equivalent to loading the shipped file.

**Tracked follow-up (orthodox files-first):** decode `reflection.dds` through the
resolver directly into the cube payload and byte-verify it against the captured
hash (the same `substitute_synthesized_terrain_id_map` pattern already used for
`TerrainIDMap`). This is a pure refinement — the pixels are proven identical — and
is deferred only because it needs the resolver threaded into the water-pass prep.
It is **not** a capture dependency: the shipped file is the source of truth.

---

## 2. Refraction target — SEPARATE UNDERWATER PASS (tracked gap)

**Shader:** `pdxwater.shader` declares `WaterRefraction` at slot 5 (lines 59–67),
built at projected screen UV and sampled with normal distortion:

```
refractiveUV = (screen_pos.xy / screen_pos.w) * 0.5 + 0.5; refractiveUV.y = 1 - refractiveUV.y;   // line 311-312
refractiveUV += vTime_HalfPixelOffset.gb;                                                          // line 313
vRefractionDistortion = normal.xz * vRefractionScale * 1.80;                                        // line 316
refractiveColor = tex2D( WaterRefraction, refractiveUV.xy - vRefractionDistortion ).rgb;           // line 331
```

**Mechanism (measured + shipped-shader-confirmed):** the game's `WaterRefraction`
buffer is a **separately rendered seafloor pass**, produced by
`pdxmap.shader` `Effect underwater` / `PixelShaderUnderwater` (lines 407–454):

- clips to below `WATER_HEIGHT` (renders only the seafloor),
- lights the seafloor terrain diffuse with the full sun/point lighting,
- blends toward `TerrainColorTint * WATER_COLOR_LIGHTNESS` (0.5) by a depth-fog
  factor `vWaterAlpha = saturate((prepos.y - 5.3) / (13.0 - 5.3))` — deeper water
  ⇒ more dark-blue water tint, shallow ⇒ more lit seafloor.

Captured evidence (`live_PAN2`): `WaterRefraction` is a **1920×1080 LDR
`B8G8R8A8_UNORM` (DXGI 87, NON-sRGB)** buffer, mean RGB ≈ (7, 12, 21)/255,
strongly blue, max ≈ (55, 58, 72). It is **not** a copy/tonemap of the pre-water
HDR scene RT (`R16G16B16A16_FLOAT`); it correlates only ≈ 0.42–0.56 with the
pre-water scene, because it is a *different render* (seafloor + depth tint), not a
function of the final lit scene. A per-channel linear fit gives
`WaterRefraction ≈ (0.13, 0.20, 0.42) · scene_linear + (0.012, 0.017, 0.027)`.

### 2a. WATER2 — the underwater pass is now RENDERED (rides `--water-full`)

WATER2 makes refraction mechanism-correct: under `--water-full`, the world-
synthesis water pass binds a **real seafloor render** as `WaterRefraction`,
reconstructed files-first from `pdxmap.shader` `Effect underwater`. Flag-OFF the
calibrated approximation stands, byte-identically.

**How it is produced** (`map_viewer/src/underwater_pass.rs`): the terrain draws
are re-issued with the pixel shader swapped to `PixelShaderUnderwater` into an
offscreen `B8G8R8A8_UNORM` target (backbuffer res), cleared to `(0,0,0,0)`, with
depth. Everything else is identical to the terrain pass (same VS, same VS/PS
constant buffers, same terrain SRVs + sampler states, same viewport/raster/blend/
depth state). This is SHADER-exact because `Effect underwater`
(pdxmap.shader:495–499) **shares the terrain `VertexShader`** (line 183) and
reuses the terrain CB/sampler layout, differing only in the pixel shader and
carrying **no `Defines`** (in particular NOT terrain's
`PDX_IMPROVED_BLINN_PHONG`, pdxmap.shader:488–493). Engine-level exactness
additionally depends on matching the engine's underwater draw coverage /
frustum culling / state, which is NOT proven from the shaders — see the named
gaps in 2b. Blend state: the terrain draws' captured no-blend RGBA-writing
state is the right one (pdxmap.shader's own `BlendState` at 473–477 disables
blending); pdxwater's blending RGB-only state (pdxwater.shader:420–426) must
NOT be inherited, and is not — the underwater re-render reuses the TERRAIN
draws' state, not the water draw's.

Empirically confirmed by compiling `pdxmap.shader:underwater` PS through the OFF-3
byte-exact machinery (`build_stage_hlsl` + `d3d_compile_stage`, FXC flags 0): the
underwater PS binds exactly **cb0 (scene, 416 B) + cb2 (pointlights, 16 B)** and
samples exactly **TerrainDiffuse(0), HeightNormal(1), TerrainColorTint(2),
LightIndexMap(10), LightDataMap(11), ShadowMap(12)** — a strict subset of the
terrain draw's bindings. The VS binds cb0 + cb1 (QuadOffset). So re-rendering the
terrain draws with the PS swapped reproduces the underwater pass.

**Constant list (all shipped, cited; encoded + unit-tested in
`map_viewer/src/water_law.rs::underwater_defines`):**

| constant | value | source | role |
|---|---|---|---|
| `WATER_HEIGHT` | 9.5 | constants.fxh:133 | sea level (clip) |
| `TERRAIN_WATER_CLIP_HEIGHT` | 3.0 | constants.fxh:57 | clip headroom |
| **clip threshold** | **12.5** | 9.5+3.0 | `clip(WATER_HEIGHT − prepos.y + TERRAIN_WATER_CLIP_HEIGHT)` keeps `prepos.y ≤ 12.5`; the seafloor is KEPT, land above 12.5 discarded (pdxmap.shader:411) |
| `WATER_COLOR_LIGHTNESS` | 0.5 | constants.fxh:90 | `waterColorTint = TerrainColorTint·0.5` (line 421–422) |
| depth-fog `vMin`/`vMax` | 5.3 / 13.0 | pdxmap.shader:424–425 (inline literals) | `vWaterAlpha = saturate((prepos.y−5.3)/(13.0−5.3))` |
| `MAP_SPECULAR_WIDTH` | 15.0 | constants.fxh:44 | seafloor glossiness (line 427) |
| `SHADOW_WEIGHT_TERRAIN` | 0.7 | constants.fxh:170 | `GetShadowScaled` weight (line 444) |
| **RT clear** | `(0,0,0,0)` | measured | the NO-DRAW background: high land clip-discarded above 12.5 plus frustum/coverage areas where no geometry was drawn. NOT "clipped deep ocean" — the deep seafloor is low-y and is KEPT by the clip. Measured on the captured payload: exactly 150 959 px are `(0,0,0,0)`; ALL other 1 922 641 px have alpha 255 (`float4(vOut, 1.0)`). The shader cannot prove the API clear call; this is the correct observable reconstruction clear |

`vOut = lerp(waterColorTint, litSeafloor, vWaterAlpha)` (line 451): at the sea
bottom `prepos.y` is low ⇒ `vWaterAlpha` low ⇒ dark-blue tint dominates; a rising
floor TRENDS toward the lit seafloor but never reaches fully-lit — the clip at
12.5 caps `vWaterAlpha` at `(12.5−5.3)/7.7 ≈ 0.935 < 1.0`. Returns
`float4(vOut, 1.0)`.

**Fail-closed:** under `--water-full`, a missing shader source / define / compile
error, a terrain draw that does not bind one of the six required SRV slots, or an
incompatible target descriptor is a hard error (`underwater_pass.rs`); there is NO
silent fallback to the approximation. The renderer requires a `GameFileResolver`
(threaded from the CLI `--game`/`--mod`); `--water-full` without one errors.

**The old approximation (flag-OFF only):** the app-owned path
(`TERRAIN_COLOR_SNAPSHOT_HLSL` in `d3d11_device.rs`) still approximates refraction
as `terrain · (0.0128, 0.0225, 0.0655) + tiny_bias` over the tonemapped terrain
RT. It is retained for byte-identical flag-OFF behaviour only; it ignores seafloor
geometry and depth-fog and is **not** mechanism-correct.

### 2a-bis. WATER3 — the underwater pass now rides the chunk-SYNTHESIS path too

WATER2 rendered the underwater seafloor only on the **captured-terrain**
production path (`render_prepared_production_map`). The product world renderer,
however, runs the **chunk-synthesis** path (`--synthesize-terrain-chunks`,
`render_prepared_synthesized_production_map`), which previously kept the flat
approximation even under `--water-full` — so shallow-water halos never appeared
in the shipping product view. WATER3 closes that gap.

Under `--water-full`, the synthesis path now re-renders the **synthesized tiled
terrain** with `PixelShaderUnderwater` into the same offscreen `B8G8R8A8_UNORM`
`WaterRefraction` target and binds it to the water/rivers passes, reusing the
existing `native_d3d11_render_underwater_synthesis` machinery (the same tiled
re-render the WATER2 oracle validation already used). The water/rivers refraction
source is selected by the pure law `world_synthesis_water_refraction_source`
(`UnderwaterPass` under `--water-full`, `AppOwnedTerrainColorSnapshot` flag-OFF),
which is unit-tested to match the captured-terrain path's non-reference selection
and to keep flag-OFF byte-identical. It is **fail-closed**: `--water-full` on the
synthesis path errors without a `GameFileResolver` (no silent fallback).

**CB1 ordering (safety).** `native_d3d11_render_underwater_synthesis` rewrites the
shared synthesis `template_draw` CB1 (QuadOffset/detail-edge) per tile over the
**whole** plan, whereas the terrain color render is frustum-culled and leaves CB1
at its last *visible* tile. The synthesis renderer therefore issues the underwater
re-render **before** the terrain color render, so the terrain color pass runs last
and leaves CB1 in exactly the flag-OFF terminal state the `--borders-from-files`
overlay (which borrows `template_draw` CB1) expects. The `--water-full
--borders-from-files --synthesize-terrain-chunks` combination is verified to render
without corruption and deterministically.

**Still an oracle, not byte-exact.** Like the WATER2 captured-path oracle, the
synthesis-path seafloor uses the *synthesized tiled* terrain geometry (whole-world
tiling, not the engine's frustum-culled underwater rasterization), so it is a
mechanism-correct reconstruction, not an engine byte-exact proof — see 2b.

### 2a-ter. WATER3 — the "checkerboard" root cause + the underwater light absorption

The WATER2 oracle's documented "checkerboard" (a regular grid of alternating
brighter/darker squares over the whole ocean, carried into the product view when
the underwater pass first rode the synthesis path) was root-caused by input
bisection (debug PS variants rendering each input in isolation):

- **Eliminated by direct measurement:** per-tile CB1 QuadOffset/`vSnap` (the
  production plan already zeroes the LOD-snap flags; forcing them to 0 changed
  nothing), slot-1 vertex offsets (the pool's linear world mapping is validated,
  and the sea heights are genuinely flat — the shipped `map/heightmap.bmp` puts
  the ENTIRE sea floor at raw 89 ⇒ y = 8.9, so the depth-fog is constant over
  open ocean), `uv2` continuity (calibrated ramp debug: `uv2 = (pos+0.5)/5632`
  exactly), VS↔PS interpolator signatures (our FXC PS's ISGN packs
  `TEXCOORD0/1` into `v1.xy/zw` exactly like the captured engine VS's OSGN),
  MAP_SIZE defines (5632×2048, matching the session's `.scache` constants).
- **Named:** the grid is the shipped shader's own **32-world-px `TerrainDiffuse`
  detail repeat** — `tex2D(TerrainDiffuse, uv2 * float2(MAP_SIZE_X/32,
  MAP_SIZE_Y/32))` (pdxmap.shader:414) samples the whole terrain material atlas
  once per 32 world px, and the atlas's internal material sub-tile grid reads as
  alternating squares. The pattern was measured at exactly 32 world px
  (autocorrelation 0.96) and is ABSENT at that amplitude in the engine's
  captured buffer.
- **Why the engine's buffer is dim-blue (WATER5, CORRECTED 2026-07-07):** the
  engine's `WaterRefraction` seafloor is ~5× dimmer and strongly BLUE. The
  WATER3 explanation was that the engine *swaps a water-absorption light state
  into the pass's CB0 at render time* — this is now **REFUTED by direct
  capture.** `session_CAPFIX2_GAPS_4` captured the engine's OWN underwater pass
  (10 `DrawIndexed` draws, underwater PS `c1bfddff00056313`). Decoding the scene
  CB0 (slot 0, 416 B) bound to those engine draws shows a WHITE sun
  (`SunDiffuseIntensity=(1,1,1,1)`), the shipped moon color
  (`(0.2176,0.5468,1,2.5)`), and the day-night gate OFF (`vFoWOpacity_….w = 0`).
  Byte-diffing it against the SAME session's terrain scene CB0
  (`cb0_samples.jsonl`) leaves only camera/time/fade lanes different; EVERY
  light-color field is byte-identical. **The engine binds the scene light rig
  UNCHANGED to the underwater pass — it applies NO CB0 light absorption.** The
  dim blue is produced by the shipped PS itself: over deep ocean the shipped
  `heightmap.bmp` puts the floor at y=8.9 ⇒ `vWaterAlpha = 0.4675` (constant), so
  `vOut = lerp(waterColorTint, lit, 0.4675)` is dominated (~53%) by the
  `waterColorTint = TerrainColorTint.rgb × WATER_COLOR_LIGHTNESS(0.5)` arm — a
  dark bluish seafloor TINT, not a light-color absorption. Decode:
  `map_viewer` bin `decode_underwater_state`; test
  `water_law::engine_underwater_cb0_carries_unabsorbed_white_sun`.

**The correction (`water_law.rs::UNDERWATER_LIGHT_ABSORPTION = (0.133, 0.234,
0.570)`) — reframed as an output-matching fallback, NOT an engine light law:**
originally the per-channel mean ratio of the engine's lit seafloor arm to ours
over ~27k deep-ocean samples (`vOut = lerp(tint, lit, 0.4675)` at y=8.9); the
old WATER_LAW scene-linear fit `(0.13, 0.20, 0.42)` shows the same blue shape.
Because the engine does NOT absorb light (above), this constant is now
understood as an EMPIRICAL correction that darkens the app's over-bright lit
arm — most plausibly a proxy for the app weighting the lit arm higher than the
engine (a higher effective `vWaterAlpha`, or a brighter/synthesized
`TerrainColorTint`) rather than any light absorption. It is KEPT because it
improved the oracle (below) and `--water-full` is DEFAULT ON, and this lane
cannot run the live D3D11 oracle to prove a replacement; the default render
path is left byte-identical. Under `--water-full` the underwater re-render
(both the captured-draw and the synthesis tile loops) binds a pass-owned clone
of the live scene CB0 whose `SunDiffuseIntensity.rgb` and
`MoonDiffuseIntensity.rgb` are multiplied by this correction
(`apply_underwater_light_absorption`; the `.a` intensity lanes and all other
CB0 fields are preserved, and the live CB0 shared with the terrain/water passes
is never written). The CB0 ambient cube is deliberately NOT patched:
`PixelShaderUnderwater`'s `ComposeLight` uses the compiled-in static
`DayAmbientMap*` constants (standardfuncsgfx.fxh:475-494,683), so a CB0 ambient
patch is inert for this pass (Codex-caught). The moon correction is an
extrapolation of the same shape (the day-gated capture cannot prove it).
Fail-closed on a missing slot-0 CB or a short CB0.

**RESIDUAL / underdetermination (honest):** the CB0 comparison refutes CB0
light absorption but does NOT prove seafloor-texture parity. The remaining
candidate for the dim-blue is the engine's `TerrainColorTint` content/state (or
the app's effective `vWaterAlpha`) vs the app's — a session that renders the
engine's own `TerrainColorTint`/`ShadowMap` through this pass and diffs against
the captured `WaterRefraction` would close it and let the correction be removed.
The capture is also day-gated (`FoW.w=0`), so it cannot distinguish a DAY vs
NIGHT underwater law; both would carry a white, day-gated CB0 here.

**Effect (measured):** WATER2 oracle vs `live_PAN2`: exact channels 23.91 % →
26.32 %, within-1/255 25.29 % → 32.59 %, mean abs err 20.84 → **12.40**,
seafloor-region mean err 33.2 → **21.4**. The fixed buffer's residual 32-px
pattern amplitude (row std 4.1–4.6, autocorr 0.24–0.27) is now BELOW the
engine's own captured buffer (std 6.1–8.4, autocorr 0.34) — the engine's
`WaterRefraction` genuinely carries the repeat at low amplitude; ours no longer
exceeds it. Product dumps show organic ocean with shore halos and no
over-bright grid. Modset-bound: recovered from vanilla `live_PAN2`; re-validate
per modset (MOD_COMPATIBILITY.md).

### 2b. WATER2 validation status + remaining deltas

Validation harness: `map_viewer` bin `dump_underwater_validation` renders the
underwater pass and compares to a session's captured `WaterRefraction` (exact-%,
per-channel deltas, diff heatmap, region map, per-region localization: clear /
seafloor / clip-edge). Determinism verified (two runs byte-identical,
fnv1a64 `b6eb5f727f909d36` on `live_PAN2`).

**Byte-exact against `restore_gate_1` is BLOCKED by genuinely-missing inputs.**
`PixelShaderUnderwater` samples `TerrainDiffuse`, `HeightNormal`,
`TerrainColorTint` (and `ShadowMap`). In `restore_gate_1`'s terrain frame those
four are **descriptor-only with no payload** (and `ShadowMap` is a null SRV), and
the session has no `terrain_chunk_library.json` and no `terrain_runtime` file. The
seafloor inputs are simply absent; the captured `WaterRefraction` cannot
reconstruct them. This is the honest, precisely-named gap (Codex-confirmed).

**Oracle validation against `live_PAN2`** (which DOES carry the resolved terrain
textures + the real captured `ShadowMap` in its `terrain_chunk_library.json`): the
underwater pass runs with all inputs. This is an **oracle / reconstruction test,
not a definitive engine byte-exact proof**, because the chunk-library geometry is
the *synthesized tiled* terrain (culled/LOD-selected), not the engine's exact
underwater rasterization, and the synthesis tiles the whole covered world rect
rather than the camera frustum the engine's underwater pass drew. Measured
(1920×1080, `chunk_library_oracle`, WITH the WATER3 light absorption of 2a-ter):
exact channels 26.3 %, exact pixels 0.92 %, within 1/255 32.6 %, mean abs err
12.4/255 (pre-absorption: 23.9 % / 20.8). Localization: the `clear/background`
region (150 959 px, captured `(0,0,0,0)`) diverges most (mean 225) — those are
pixels the engine's frustum-culled underwater pass left clear but the
whole-world synthesis fills; `seafloor` (1.92 M px) is off ~21/255 mean
(pre-absorption ~33; the remainder is the scene-CB0/camera substitution — the
oracle compares against a buffer captured at a different pan position). Diff
heatmap + region map are written beside the stats JSON.

**Remaining deltas / named gaps (all documented, none hidden):**
- **Synthesized vs engine geometry** — the only capture shape that carries the
  seafloor textures is the chunk library, whose geometry is synthesized; a
  byte-exact engine proof needs a session with the engine's exact underwater draw
  stream AND the seafloor textures (none exists yet). Recapture of `restore_gate_1`
  with the underwater-pass SRVs would close this.
- **Camera-frustum vs whole-world coverage** — the oracle synthesis renders the
  whole world rect; the engine's underwater pass is frustum-culled. This inflates
  the `clear` region divergence.
- **Scene CB0 alignment** — the oracle run substitutes the library's canonical
  scene CB0; exact-camera alignment with the water pass's CB0 is a follow-up.
- **`ShadowMap` null vs real** — where a terrain frame's `ShadowMap` is null, the
  direct-lit term diverges; on the deep dark seafloor the movement is usually
  small but not provably negligible.
- Resolution/sRGB/offset are correct: the target is `B8G8R8A8_UNORM` at backbuffer
  res (linear, not sRGB-encoded); the ×1.80 distortion and `vTime_HalfPixelOffset`
  are applied in `pdxwater`, not the underwater pass, so they are already right.

---

## 3. Virtual-sun / glitter law (validated against captured CB0)

**Shader:** water glitter is a LEAN spec lobe (`pdxwater.shader` 287–322) driven by
the sun direction `CalculateSunDirectionWater(pos)` (`standardfuncsgfx.fxh:634`),
which differs from terrain by a swizzle:

```
CalculateSunDirection( pos, vVirtualSunPos.xyz, ... )   // terrain, fxh:631
CalculateSunDirection( pos, vVirtualSunPos.xwz, ... )   // WATER,   fxh:636
```

`CalcGlobeNormal`/day-night selection (fxh:598–626) then returns
`normalize(worldPos − sourcePos)`. The `.xwz` swizzle uses the **`.w`** component
in place of `.y` — so the water light *height* is the packed `.w`.

**CB0 layout** (`standardfuncsgfx.fxh` `ConstantBuffer(0,0)`): `vVirtualSunPos`
float4 @ byte 144, `vVirtualMoonPos` @ 160, `vSecondVirtualSunPos` @ 176,
`vSecondVirtualMoonPos` @ 192.

**Packing law (files-derived):** each packed float4 is
`(x = longitude, y = terrain height, z = latitude, w = WATER height)`, where the
heights/latitudes are `00_graphics.lua` defines:
`SUN_HEIGHT/MOON_HEIGHT = 600` (`.y`), `SUN_HEIGHT_WATER = 5000` /
`MOON_HEIGHT_WATER = 550` (`.w`), `SUN_LATITUDE = 848` (sun `.z`),
`SECOND_SUN_LATITUDE = SECOND_MOON_LATITUDE = 100` (second `.z`).

**Validation vs captured scene CB0** (identical in `restore_gate_1` **and**
`live_PAN2`):

| field | captured `(x,y,z,w)` | law: `(x*, SUN/MOON_HEIGHT, LATITUDE, *_WATER)` | residual |
|---|---|---|---|
| `vVirtualSunPos` | (2816, 600, 848, 5000) | (·, 600, 848, 5000) | 0 |
| `vVirtualMoonPos` | (0, 600, 1448, 550) | (·, 600, **1448†**, 550) | 0 on y/w |
| `vSecondVirtualSunPos` | (0, 600, 100, 5000) | (·, 600, 100, 5000) | 0 |
| `vSecondVirtualMoonPos` | (2816, 600, 100, 550) | (·, 600, 100, 550) | 0 |

All `.y` height, `.w` water-height, and (sun/second) `.z` latitude residuals are
`< 1e-3` — the packing law reproduces the captured vectors exactly. Encoded and
unit-tested in `map_viewer/src/water_law.rs`
(`packing_law_reproduces_captured_cb0_vectors`).

**Consequence for glitter:** the water sun source position is
`vVirtualSunPos.xwz = (2816, 5000, 848)` — height 5000 vs terrain's 600, i.e. a
near-vertical sun, which produces the tight LEAN specular glitter lobe. Confirmed
by the `.xwz` swizzle in `water_source_position()`.

**Tracked gaps (engine-runtime, not in shipped files):**
- `†` The primary moon's `.z = 1448` and every light's `.x` (longitude) are
  computed by the engine's C++ orbit code from the time-of-day
  (`DayNight_Hour_SunDir`, CB0 @ 256) and appear in no shipped shader/define.
  They are read from the captured CB0 (modset-bound). WATER1 owns and validates
  the *packing/swizzle law* (which define feeds which component) — that is what
  selects the glitter geometry.
- Implementing a per-hour CB0 patch for the virtual-sun longitude in the replay
  is NOT done here: the water pass reuses one canonical captured scene CB0
  (matching the byte-exact parity gates), and re-deriving the engine orbit is a
  separate follow-up. It does not fit the existing CB0 patching machinery cleanly.

### 3a. WATER3 — the LEAN specular overflow analysis (glint blowout)

The glint is a LEAN (normalized-Gaussian) specular lobe. The shipped `Effect
water` PS — which the app REPLAYS as compiled DXBC, not re-authored — computes
`spec = exp(-0.5·e/det)/sqrt(det)` (pdxwater.shader:309) and
`vGlossiness = (spec/9)·(1−vSpecMap)` (pdxwater.shader:322), feeding
`CalculatePBRSpecularPower`'s **unbounded** `(vSpecularPower+2)/8` term
(standardfuncsgfx.fxh:528; water has no `PDX_IMPROVED_BLINN_PHONG`).

**Overflow (files-derived):** the NDF PEAK is `spec ≈ 1/sqrt(det)`, which
diverges as the slope covariance `det → 0+` (glassy water). In the HDR
`R16G16B16A16_FLOAT` scene target this overflows to `+Inf → NaN` downstream
(bloom/tonemap), reading as the reported blown-white disc with red/garbage
pixels. **There is no clamp in the shipped shader to mirror.**

**The "clamp the shader implies" is the input, not a number:** the shipped
pipeline is tamed entirely by `SpecularMap.a` (`= vSpecMap`) plus the shore
alpha — (1) `vFlatten = vSpecMap` scales `B` and `M *= vSpecMap²`
(pdxwater.shader:294-296), shrinking `sigma` toward flat where the map is small;
(2) `(1−vSpecMap)` collapses the glossiness to ~0 over open sea where
`vSpecMap → 1`; (3) `1−waterShore` alpha (pdxwater.shader:282,414) with the
captured `src_alpha/inv_src_alpha` blend fades near-shore bright fragments out.
These are encoded + unit-tested in `water_law.rs` (`lean_specular_peak`,
`lean_specular_peak_glossiness`, `lean_flattened_det`).

**Conclusion (Codex-refuted): no app-specific glint overflow was confirmed on
the replay path.** The app binds the REAL captured `SpecularMap` (slot 4, its own
`.a`; verified not mean-flattened, format preserved), replays the shipped PS with
the captured alpha-blend, and thus reproduces the game's taming — it cannot push
`det → 0` where the game would not. A static-dump reproduction of a blown-white
glint disc over open Atlantic was **not** obtained (postfx clamps the HDR to
white; the "red" pixels found under `--no-postfx` were sandy COAST terrain, not a
specular NaN). The one app-synthesized, non-captured input is the product-only
light-rig `.x` longitude shift (`write_virtual_light_rig_fields`,
shader_meta.rs), which invents a longitude the shipped files do not define; if a
live-only glint reproduces **only** at moved product cameras, the files-faithful
remedy is to A/B `delta_x = 0` or bound `.x` to captured/orbit evidence — **not**
a glossiness clamp (which would deviate from the shipped shader). This is left as
a **tracked open gap** pending a live reproduction with the exact camera/mod.

---

## `--water-full` end-to-end in replay_app (GRADE2 hook LANDED; GRAD1 graduated)

`--water-full` is fully implemented end-to-end in `replay_app`: the
`NativeD3d11TerrainRenderer` carries a `water_full` flag
(`set_water_full` / `with_water_full`), the world-synthesis water pass honors it
(real cube vs mean-color), and the map_viewer CLI
`render-production-map-d3d11 --water-full --map-clock-time <t>` exercises the real
cube end-to-end (proven: flag-on differs from flag-off; deterministic across runs;
change confined to water pixels).

The GRADE2 hook has **landed**: `render.rs::MapRenderer::new` →
`CapturedPassLayer::new` threads `water_full` (and the GRAD1 `water_full_explicit`
opt-out marker) into `NativeD3d11TerrainRenderer::from_borrowed_raw(...)`,
calling `set_water_full(...)` and, when the underwater refraction pass is active,
`with_underwater_resolver(resolver.clone())`. `main.rs` → `app.rs` /
`headless.rs` plumb the flag and the resolver.

**GRAD1 graduation:** `--water-full` is now **DEFAULT ON** (see the default-policy
block at the top of this file). An **explicit** `--water-full` without a resolver
still fails closed; the **default** without a resolver downgrades to the old
approximation with a one-line NOTICE (both cube and refraction revert together so
the render stays byte-identical to the old default); `--no-water-full` selects the
old approximation directly. The reflection cube (WATER1) needs no resolver; the
underwater refraction pass (WATER2/WATER3) reconstructs `pdxmap.shader` `Effect
underwater` and needs the modset `GameFileResolver`, so the resolver is the
fail-open/fail-closed gate.
