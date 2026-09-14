// `pdxmap.shader`: the `terrain` and `underwater` effects.
//
// The vertex shader replaces the engine's instanced LOD quads with one uniform
// grid over the visible ground rectangle (R1 section 5.2 shows this is
// equivalent and strictly more accurate), so `QuadOffset_Scale_IsDetail` and
// the `vSnap` T-junction stitching have no counterpart here. Everything the
// pixel shaders do is ported literally; each departure is commented.
//
// # This pass is where the scene's brightness gap lives (measured 2026-09-14)
//
// `SCENE_BRIGHTNESS` in `map::post` says the scene this viewer hands the colour
// grade is about 2.7x brighter than the game's. The game's own console layer
// toggles locate it here rather than in the political fill, the trees or the
// water. `pairE_h200_nofill.png` is the reference hour with `debug_borders`,
// which removes the gradient fill and the border strips; `--debug-borders` puts
// this viewer in the same state; the camera is (3009.83, 1568.00, 163.934),
// fitted by matching bare terrain to bare terrain and independently from the
// victory-point labels. Masks are the game's own: a pixel is bare terrain when
// blanking the terrain changes it and removing the trees, the rivers and the
// city sprawl does not. Scene values, both frames carried back through the post
// chain with the winter cube the snow rule selects:
//
// | surface | game | this viewer | game / ours |
// |---|---|---|---|
// | bare terrain (809k px) | 0.060 0.076 0.052 | 0.255 0.310 0.165 | 0.24 0.25 0.32 |
// | terrain under trees | 0.089 0.104 0.097 | 0.251 0.306 0.184 | 0.35 0.34 0.53 |
// | terrain under city sprawl | 0.119 0.129 0.127 | 0.286 0.322 0.216 | 0.42 0.40 0.59 |
// | open water | 0.103 0.127 0.185 | 0.251 0.286 0.345 | 0.41 0.44 0.54 |
// | whole frame | 0.072 0.089 0.068 | 0.251 0.290 0.192 | 0.29 0.31 0.35 |
//
// So the bare lit terrain is the worst of it at a factor of four, the sea is
// only 2.3x out, and the two rows between them are where the game draws a layer
// this viewer does not: its tree canopy and its city sprawl add scene of their
// own, which is why the ratio there sits between the two. One constant fitted
// over the whole frame is therefore a compromise, and it shows in the graded
// frame at the same camera: with `SCENE_BRIGHTNESS` applied, this viewer's bare
// terrain comes out at 1.8, 1.3 and 1.1 times the game's three channels while
// its open water comes out at 1.0, 0.78 and 0.72 of them.
//
// The hue is a smaller story than the level here. Over bare terrain the three
// channels want 0.24, 0.25 and 0.32, so the blue deficit the daylight pairs show
// is only 1.3x relative to red once the political fill and the sea are out of
// the frame; the 2.45x that `map::post` records is a whole-frame number and
// belongs mostly to the sea and to the filled land.
//
// Two cautions on the sea row. The whole-frame fits at the four scored cameras
// say the sea wants the *same* per-channel correction as the land (the sea pass
// records that), where this table has it 1.7x closer to one. The masks are not
// the same thing: the sea here is the rivers, lakes and coastal shelf a camera
// 164 units up sees, under the game's weather haze, while the scored cameras
// look at the open North Sea and Baltic. And the game's `debug_water` removes
// the underwater draw along with the plane, so `pairE_05_nowater` shows lit
// terrain below the waterline; that only affects which pixels the mask calls
// water, not the comparison itself, which is between the two full frames.
//
// Not the fill: at this camera the engine's own
// `gradient_border_distance_to_alpha` gives an interior fill of exactly zero,
// and the game agrees — over inland terrain more than seven pixels from any
// border strip the median difference between `pairE_h200_fill` and
// `pairE_h200_nofill` is 1, 1 and 2 levels of 255, against this viewer's 0.
// Scripts: `C:\hoi4_scratch\replay_map\scene\pairE_compare.py`,
// `fitE_terrain.py`.
//
// ## Which effect and which defines the engine actually binds
//
// The scene brightness search asked whether the political-map terrain draw runs
// with the variant this port assumes. It does, and the evidence is now closed
// enough that the question should not be reopened.
//
// `FUN_141261100` compiles all four map-terrain effects at construction, each by
// its literal name against `"gfx/FX/pdxmap.lua"`, and stores the resulting
// effect index in its own field: `"terrain"` (the string at `0x142719110`) to
// `this+0x14`, `"terrain_low_gfx"` (`0x14298fa30`) to `this+0x18`,
// `"underwater"` (`0x14298fa40`) to `this+0x1c` and `"terrainunlit"`
// (`0x14298fa50`) to `this+0x20`. Compiling them all is not selecting one.
//
// The selection is in the draw, `FUN_141264d30(this, ctx, _, pass, ...)`, whose
// `pass` argument picks the field: 0 takes `+0x14` or `+0x18`, 1 takes `+0x1c`
// (underwater) and 2 takes `+0x20` (terrainunlit). For the main pass it reads a
// byte from the graphics-settings object `*(DAT_143317038) + 0x178 + 0x18`
// (`FUN_1401fa1c0` returns the global, `FUN_1401faa50` adds `0x178`) and takes
// `terrain_low_gfx` when that byte is zero, `terrain` otherwise. The byte is
// `high_gfx_shaders`, which the reference profile's `settings.txt` sets to
// `yes`; `draw_hires_terrain`, `shadows` and `texture_quality` are read nowhere
// on this path, and `pdxmap.shader` carries no hires terrain effect at all.
//
// The engine's own shader cache settles it without the decompile. A run writes
// the fully preprocessed HLSL it handed the compiler to
// `<userdir>/shadercache/dx11_win32/{vs,ps}_5_0/*.scache`, injected defines
// included, so the reference profile's cache is a direct record of what was
// compiled. `PixelShaderTerrain` appears there in exactly two define sets, and
// the one that matches this port
// (`ps_5_0/00080000F7A2100080E174D8.scache`, and the same body with borders on
// in `0008000089A21000815BD390.scache`) is:
//
//     PIXEL_SHADER, MAP_SIZE_X 5632, MAP_SIZE_Y 2048,
//     MAP_POW2_X 1, MAP_POW2_Y 1, FOW_POW2_X 1, FOW_POW2_Y 1,
//     GRADIENT_BORDERS, PDX_IMPROVED_BLINN_PHONG, PDX_DIRECTX_11
//
// and nothing else. The two cached bodies differ in `GRADIENT_BORDERS` and in
// nothing else, which is the console `debug_borders` toggle: the only string of
// the five searched for that the binary contains at all is `GRADIENT_BORDERS`
// (at `0x142719378`, read by the console handlers at `0x14024930b`,
// `0x1402494ff` and `0x140a4df30`, among the "Borders enabled/disabled" texts).
// `NO_SHADER_TEXTURE_LOD` exists only inside the OpenGL header block beside
// `#define PDX_OPENGL 1` and `gfx/FX/defines_glsl.fxh`, so the D3D11 path never
// sets it; `LOW_END_GFX` and `PDX_IMPROVED_BLINN_PHONG` are not in the binary at
// all and can only come from the `Defines` blocks of `pdxmap.shader`.
//
// So the assumed set is the bound set. Three further checks against the same
// cached source found no unported term either:
//
// * Every `static const` the cached terrain pixel shader carries and this port
//   also names — 67 of them, `COLORMAP_OVERLAY_STRENGTH` and `NIGHT_DARKNESS`
//   and the ambient basis and `FOG_MAX` and the rest — has the same value.
//   (`constants.fxh` carries commented-out alternatives for `SNOW_COLOR` and
//   `FOG_MAX`; the cache shows the live ones are the ones ported.)
// * The terrain vertex shader (`vs_5_0/0008000009540000B4175865.scache`) is LOD
//   arithmetic and two projections. It writes no colour, no fog and no lighting
//   term, so replacing its quads with a uniform grid cannot move brightness.
// * The pixel shader's constant buffers are `cb0`, the frame constants
//   `FUN_140ff3fc0` fills, and `cb1`/`cb2`, which hold
//   `ShadowMapTextureMatrix`, `QuadOffset_Scale_IsDetail`, `vSnap`,
//   `vBorderLookup_HeightScale_UseMultisample_SeasonLerp` and
//   `GridStart_InvCellSize`. Of those the terrain pixel shader reads none — the
//   last is declared and never used — so there is no per-draw multiplier to
//   port. The draw's own argument block in `FUN_141264d30` is a texture table,
//   not constants.
//
// The counterfactual was measured rather than argued: rendering the reference
// day frame with every `LOW_END_GFX` branch taken instead (flat `(0,1,0)` base
// normal, `vSpec` 0, `ComposeLight` without the snow ambient, no city lights, no
// distance fog) moves `reference/03.md` from mae 30.92 / delta E 20.75 to
// 31.23 / 21.28 and lowers all three SSIM levels. Over the same 809 k
// bare-terrain pixels of the pairE frame it moves the game-over-ours scene ratio
// from 0.236 0.245 0.317 to 0.216 0.227 0.333 — a few per cent, and the wrong
// way in two channels of three. It is both the wrong variant and far too small
// an effect to be the gap.
//
// What this does not explain, and what is therefore left: the table above. The
// terrain effect, its defines, its constants, its vertex stage and its constant
// buffers are all now accounted for, so the remaining factor of four has to be
// in an input to the pass rather than in the pass itself.
//
// ## The shadow term is that input, and it is not the gap either (2026-09-14)
//
// The one input the pass reads that this port never renders is `ShadowMap`, and
// every lit pass scales its sun term by it:
// `fShadowTerm = max( GetShadowScaled( SHADOW_WEIGHT_TERRAIN, Input.vScreenCoord,
// ShadowMap ), 0.1 )`, where `GetShadowScaled` (`shadow.fxh:44`) is
// `(1 - w) + w * saturate( ShadowMap.r + (1 - ShadowFadeFactor) )` with weights
// terrain 0.7, tree 0.7, border 0.7, water 0.5, river 0.4
// (`constants.fxh:170-175`). If that buffer sampled as 0 the sun would be
// scaled by `1 - w`, which is 0.3 here and 0.5 on the water — close enough to
// the measured 0.24 and 0.41 above, and with the same land/sea split, to be
// worth settling. It is settled, and the answer is no.
//
// **What the engine binds.** `ShadowMap` is not a depth map at this point in
// the frame: it is a backbuffer-sized RGBA target, `CWrapWorldShadowMap+0x1d8`,
// bound at `t12` for the terrain (`FUN_141264d30`, main branch), `t15` for the
// water and the river and `t8` for the trees, holding `(shadow, FogColorFactor,
// FogAlphaFactor, 1)` — the same texture and screen coordinate serve
// `GetShadowScaled` through `.r` and `ApplyFOW` through `.g` and `.b`. The
// `terrainunlit` and `treeunlit` effects write it (`pdxmap.shader:457-470`,
// `tree.shader:425-445`, both compiled in the reference run's cache), sampling
// the real 2048-square depth map at `+0x1d0` through `CalculateShadow`, and two
// `shadowblur` passes blur it in place. `FUN_140b481a0` gates all of that on
// `mapRenderingOptions.draw_shadows` — the byte at `settings+0x178+0x13`, which
// `FUN_1419a3970` ties to the `"shadows_checkbox"` widget — and on
// `vCamPos.y < DRAW_SHADOWS_CUTOFF`, **not** on `graphics.shadows`, whose only
// located effect is the hardware check in `FUN_142224600` clearing the device
// flag `renderer+0xf0` through `FUN_142214e00`; no function on the map path
// reads that flag. The reference profile has `draw_shadows=yes`. So the
// earlier reading of this port — `shadows=no`, therefore nothing casts,
// therefore the buffer is the all-ones clear — was wrong in its premise: below
// height 400 the game really does draw cast shadows that this viewer does not,
// and `.r` is 1 only where the sun reaches. (The clear value is
// `DAT_143098838 = (1, 0, 0, 1)`, so where nothing is drawn the term is 1 and
// the fog of war is absent, which is what the fallback would have been.)
//
// **The prediction.** Our scene is affine in the shadow term, since
// `ComposeLightSnow`, the border-light lerp, `ApplyFOW`, the distance fog and
// the day-night blend are all affine in the pass's colour. Rendering the pairE
// frame twice, once stock and once with every lit pass's `fShadowTerm` forced
// to 0, splits each layer into an ambient part `A` and a sun part `S`, and the
// hypothesis predicts `A + (1 - w) * S`. Medians over the same masks as the
// table above:
//
// | surface | game | ours | predicted | game / ours | game / predicted |
// |---|---|---|---|---|---|
// | bare terrain | 0.060 0.076 0.052 | 0.255 0.310 0.165 | 0.094 0.115 0.073 | 0.24 0.25 0.32 | 0.64 0.66 0.71 |
// | under trees | 0.089 0.104 0.097 | 0.251 0.306 0.184 | 0.093 0.117 0.084 | 0.35 0.34 0.53 | 0.95 0.89 1.15 |
// | under cities | 0.119 0.129 0.127 | 0.286 0.322 0.216 | 0.111 0.130 0.100 | 0.42 0.40 0.59 | 1.08 0.99 1.27 |
// | open water | 0.103 0.127 0.185 | 0.251 0.286 0.345 | 0.133 0.153 0.198 | 0.41 0.44 0.54 | 0.78 0.83 0.94 |
// | whole frame | 0.072 0.089 0.068 | 0.251 0.290 0.192 | 0.088 0.108 0.088 | 0.29 0.31 0.35 | 0.81 0.83 0.77 |
//
// **The measurement.** The same split inverts: the factor the game's scene
// actually demands on our sun term is `k = (game - A) / S`, and it is flat
// across the three channels, which is itself the news — the gap is a scalar on
// the sun and not a hue error. Over the 809 k bare-terrain pixels `k` is
// **0.164 0.175 0.145**, and over open water **0.380 0.405 0.462**. The
// hypothesis cannot reach either: `(1 - w) + w * saturate(...)` is bounded below
// by `1 - w`, so a weight-0.7 pass can never scale the sun below 0.3 and a
// weight-0.5 pass never below 0.5, whatever the buffer holds and whatever
// `ShadowFadeFactor` is. The land wants 0.17 where 0.30 is the floor and the sea
// wants 0.41 where 0.50 is.
//
// **And the height series closes it.** `ShadowFadeFactor` is
// `max(0, (400 - camY) / 50)` (`FUN_140ff3fc0`, byte `0x11c`; `map::frame`
// carries it), so above height 400 the saturate inside `GetShadowScaled` pins
// the term to exactly 1 and `FUN_140b481a0` does not run the shadow pass at all.
// The demanded `k` over the four fitted pairE fill-off framings, land and water
// masked by our own `--no-water` render:
//
// | height | land `k` | water `k` |
// |---|---|---|
// | 50.178 (`hmin`) | 0.156 0.169 0.127 | 0.420 0.419 0.359 |
// | 163.934 (`h200`) | 0.178 0.186 0.168 | 0.389 0.408 0.454 |
// | 627.404 (`h400`) | 0.195 0.166 0.110 | 0.324 0.313 0.304 |
// | 1589.602 (`h700`) | 0.315 0.256 0.149 | 0.270 0.282 0.275 |
//
// The two lower framings are below the cutoff, where the game draws shadows and
// this port does not; the two upper ones are far above it, where the game draws
// none either and the port is exact in this respect. The demanded factor does
// not step at the cutoff — 0.13 to 0.19 on land below it against 0.11 to 0.32
// above it, where the hypothesis needs exactly 1 — and its per-pixel
// distribution does not change shape across it either. Green `k` over land, at
// the 10th, 50th and 90th percentiles: 0.028, 0.169 and 0.385 at height 50 and
// 0.059, 0.186 and 0.337 at 164, against -0.013, 0.166 and 0.370 at 627 and
// 0.080, 0.256 and 0.482 at 1590. The shadowed low tail that the two lower
// frames should carry and the two upper ones should not is absent. Scripts:
// `C:\hoi4_scratch\replay_map\shadow\` (`probe.py` forces the term, `render.py`
// and `heights.py` render, `ratios.py`, `predict.py`, `fitk.py` and `spread.py`
// measure).
//
// So the shadow term is closed as a candidate for the brightness gap. The pass
// itself is a separate and much smaller thing, and it is drawn now; the section
// at the foot of this header measures it.
//
// ## And `k` is not what the frame says: the sun-term reading is withdrawn (2026-09-14)
//
// The paragraph that used to stand here read the table above as "the sun-term
// scalar the game demands is 0.17 on land and 0.41 on water, flat in colour",
// and made that the lead. It is withdrawn. `k = (game - A) / S` is the
// coefficient of a **one-parameter model that pins the ambient's coefficient at
// 1**, and the frame does not support that pin. Three candidates for a genuine
// multiplier on `CalculateSunLight`'s output were predicted and measured first;
// all three are dead, and then the shape itself turned out not to be the
// frame's.
//
// **1. The day factor.** `CalculateSunLight` weights the sun by
// `vDayFactor = 1 - DayNightFactor( CalcGlobeNormal( pos.xz ) )` and the moon by
// the same function at `MOON_FEATHER_MIN/MAX`. The open worry was that the
// terminator law `map::frame` fits is `DayNightWithBlend`'s, which might take a
// different feather or a different direction from this one. The game's cached
// terrain pixel shader answers it: both go through the *same* one-argument
// `DayNightFactor( vGlobeNormal )` overload, at `FEATHER_MIN = -0.01` and
// `FEATHER_MAX = 0.01`, reading the same `DayNight_Hour_SunDir.yzw`, and
// `MOON_FEATHER_MIN/MAX` are `-0.01`/`0.01` as well
// (`00080000F7A2100080E174D8.scache:700-703, 734-743, 1018-1038`); `common.wgsl`
// ports that body statement for statement. Over Berlin (3010, 1568) the globe
// normal is (0.1045, 0.9265, -0.3614) and its dot with the sun direction is
// **-0.3088** at the day hour 60828490 and **+0.4123** at the night hour
// 60828478 — 31 and 41 times the feather's own half-width — so `vDayFactor` is
// exactly 1 by day and `vNightFactor` exactly 1 by night, in the game's shader
// as in this one, and no plausible error in the declination moves a dot of 0.31
// inside a band of 0.01. Predicted `k` **1.000** against a measured 0.17.
//
// **2. The light vector.** As ported, and read out of the binary rather than
// out of R1. `FUN_141267270` writes the virtual sun as three consecutive floats
// at `this+0x5d18`: `(1 - hourFraction) * MAP_SIZE_X` with `MAP_SIZE_X` from
// `*(DAT_143320db8 + 0x40)`, then `DAT_14331e6ec` (`SUN_HEIGHT` 600), then
// `DAT_14331e768` (`SUN_LATITUDE` 848). `SUN_LATITUDE` lands in the **third**
// float, so it is a world `z` and not a pixel row — the 848-from-the-top
// reading would have put it at `z` 1199 and *raised* `N.L`, the wrong way. The
// same function derives the moon's `z` as `MAP_SIZE_Y - *(this+0x5d1c)`, that
// is `2048 - SUN_HEIGHT`, which is the odd form `map::frame` already carries
// marked UNVERIFIED; it is the decompiled form, and it is deliberate enough to
// be reproduced. `FUN_140ff3bb0` copies those into `cb0` at 144 and 160 with
// `DAT_14331e898` (`SUN_HEIGHT_WATER` 5000) and `DAT_14331eafc`
// (`MOON_HEIGHT_WATER` 550) in the `.w` that `CalculateSunDirectionWater`
// swizzles, and builds both second positions as
// `fmod( x + MAP_SIZE_X * 0.5, MAP_SIZE_X )` at `DAT_14331e7f0` / `DAT_14331ea5c`
// (`SECOND_SUN_LATITUDE` and `SECOND_MOON_LATITUDE`, both 100) — field for field
// what `map::frame` writes. At Berlin `|vWorldPos.x - vSourcePos.x|` is 275 of a
// half map width of 2816, so the wrap branch does not fire and `smoothstep(0.5,
// 1.0, 0.098)` is 0: the second sun is not engaged and cannot be the gap
// either. That gives `N.L` 0.6045 on flat land (sun elevation 37.2 degrees) and
// 0.9883 on flat water.
//
// The prediction is what kills it. A lower sun does scale land harder than
// water, as the two heights suggest — but not by enough, and not in the ratio
// the split needs. `k` 0.164 on land wants `N.L` 0.0991, which at `SUN_HEIGHT`
// 600 puts the source **6,022** world units away horizontally; `k` 0.380 on
// water wants `N.L` 0.3759, which at `SUN_HEIGHT_WATER` 5000 puts it **12,327**
// away. The two heights share one horizontal position by construction — the
// water swizzle changes only `y` — so a single virtual sun cannot be 6,022 from
// Germany for the terrain and 12,327 for the sea beside it. No map size, no
// latitude convention, no hour and no declination closes a factor of 2.05 in
// that ratio.
//
// **3. Any remaining multiplier on the sun path.** There is none.
// `00_graphics.lua` gives `SUN_INTENSITY = 1.0` (line 1061, with a
// commented-out 0.4 beside it), `SUN_SPECULAR_INTENSITY = 1.0` (1062) and
// `LIGHT_HDR_RANGE = 1.0` (772), and `FUN_140ff3bb0` is where each lands in
// `cb0`: `DAT_14331f084` into `SunDiffuseIntensity.a` at `0x17c`,
// `DAT_14331f27c` (`MOON_INTENSITY` 2.5) into `MoonDiffuseIntensity.a` at
// `0x18c`, `DAT_14331f120` into `SunSpecularIntensity` at `0x194` and
// `DAT_14331d3b0` into `HdrRange` at `0xdc`. So the sun path carries no free
// scalar at all; and `SunSpecularIntensity` and `HdrRange` are shared by the
// terrain and the water passes, so neither could split them even if it were
// wrong.
//
// **The shape itself.** `A` and `S` are not proportional across a frame: the
// terrain normal drives them apart, because `AmbientLight` weights a sideways
// normal at 0.10 to 0.15 and an upward one at 0.03
// (`DayAmbientMapPosX`/`NegX`/`PosY` in the cached shader) while
// `saturate( N.L )` runs from 0 to 0.9 over the same normals. So the game's
// scene `G` can be regressed on `A` and `S` per channel, and the coefficient of
// `A` is a measurement rather than an assumption. Three nested models over the
// same bare-terrain and open-water masks as the table above:
//
//     sun     G = A + k*S           (one parameter, alpha pinned at 1)
//     scale   G = r*(A + S)         (one parameter, alpha = beta)
//     free    G = alpha*A + beta*S  (two parameters)
//
// Least squares, green channel, over the four fill-off pairE framings and the
// two scored pairs, with the residual rms of each — the rms is the part that
// does not care how collinear `A` and `S` are, and the standard error on
// `alpha` comes from the residual variance with the sample discounted 16-fold
// for the spatial correlation of neighbouring pixels:
//
// | frame | surface | free alpha | free beta | rms free | `k` | rms sun | `r` | rms scale |
// |---|---|---|---|---|---|---|---|---|
// | `hmin` 50.178 | land | 0.516 +- 0.016 | 0.221 | 0.0339 | 0.167 | 0.0365 | 0.253 | 0.0347 |
// | `h200` 163.934 | land | 0.361 +- 0.011 | 0.265 | 0.0291 | 0.204 | 0.0372 | 0.274 | 0.0293 |
// | `h400` 627.404 | land | 0.271 +- 0.011 | 0.249 | 0.0361 | 0.175 | 0.0518 | 0.252 | 0.0361 |
// | `h700` 1589.602 | land | 0.314 +- 0.026 | 0.359 | 0.0569 | 0.301 | 0.0632 | 0.355 | 0.0569 |
// | `03.md` 302.960 | land | 0.350 +- 0.006 | 0.230 | 0.0447 | **-0.386** | 0.0848 | 0.295 | 0.0451 |
// | `02.md` 310.044 | land | 0.272 +- 0.006 | 0.300 | 0.0385 | **-0.136** | 0.0863 | 0.287 | 0.0385 |
// | `hmin` | water | 0.612 +- 0.192 | 0.481 | 0.0471 | 0.404 | 0.0498 | 0.506 | 0.0473 |
// | `h200` | water | 0.748 +- 0.048 | 0.417 | 0.0307 | 0.392 | 0.0310 | 0.448 | 0.0311 |
// | `h400` | water | 0.289 +- 0.018 | 0.371 | 0.0187 | 0.315 | 0.0233 | 0.364 | 0.0188 |
// | `h700` | water | 0.387 +- 0.032 | 0.334 | 0.0149 | 0.296 | 0.0158 | 0.337 | 0.0149 |
// | `03.md` | water | 0.626 +- 0.032 | 0.413 | 0.0261 | 0.383 | 0.0268 | 0.430 | 0.0263 |
// | `02.md` | water | 0.321 +- 0.028 | 0.433 | 0.0176 | 0.380 | 0.0199 | 0.424 | 0.0177 |
//
// The free `alpha` is **0.27 to 0.75, never within two standard errors of 1 and
// more than nineteen away on every row but the two water ones with the smallest
// samples** (30, 58, 66, 26, 108 and 121 standard errors on the six land rows).
// The one-parameter scale model — which sets `alpha = beta` and so says nothing
// about the sun in particular — reproduces the free fit's residual to within
// 2.5 % on every one of the twelve rows, while pinning `alpha = 1` costs 1 % to
// 124 % more residual, the worst of it on the two scored pairs. And on those
// two scored pairs `k` is **negative** over land in two channels of three
// (`reference/03.md` -0.312 -0.386 -0.141, `reference/02.md`
// -0.551 -0.136 +0.187), because the game's scene there is *below* our ambient
// alone: no scalar on `CalculateSunLight`'s output, of any sign the function can
// produce, reaches it. Those two frames carry the political fill, which makes
// `A` large (0.17 and 0.12 green against 0.03 on bare terrain) and so separates
// the two models far better than the fill-off frames do.
//
// Binning those two frames by `S/A` shows it without any fit at all. Green
// channel, `bins.py`, sextiles of `S/A`:
//
// | frame | `S/A` band | n | mean `A` | mean `S` | mean `G` | `G/A` | `(G - A)/S` |
// |---|---|---|---|---|---|---|---|
// | `03.md` | 0.07-0.41 | 159,726 | 0.3006 | 0.0774 | 0.1197 | 0.40 | -2.34 |
// | `03.md` | 0.97-1.34 | 399,815 | 0.1489 | 0.1714 | 0.0904 | 0.61 | -0.34 |
// | `03.md` | 16.5-37.5 | 169,734 | 0.0130 | 0.2533 | 0.1108 | 8.50 | +0.39 |
// | `02.md` | 0.05-0.57 | 163,134 | 0.2525 | 0.0920 | 0.1027 | 0.41 | -1.63 |
// | `02.md` | 1.62-2.52 | 408,689 | 0.0841 | 0.1800 | 0.0724 | 0.86 | -0.07 |
// | `02.md` | 18.5-up | 175,328 | 0.0060 | 0.1621 | 0.0726 | 12.21 | +0.41 |
//
// Where the sun contributes almost nothing the game's scene is 0.40 of our
// ambient, not 1.0 of it, on both the day and the night pair; the apparent `k`
// only climbs toward 0.4 in the bands where `S` dominates and the ambient's
// error is diluted out of it. That is a scene scale seen from two ends, and it
// is the whole of the "flat in colour" result the withdrawn paragraph reported.
//
// Two readings fit the same medians and only one fits the frame: **the game's
// whole scene, its ambient included, is about 0.25 to 0.36 of ours on land and
// about 0.34 to 0.51 on water, and the ambient is not right after all.** `k`
// came out at 0.17 on the bare-terrain frames rather than at the scale only
// because forcing `alpha` to 1 has to subtract the whole of our too-bright
// ambient out of the sun's coefficient: a scene scale `r` shows up as
// `k = r + (r - 1) * A/S`, which with `A/S` 0.12 on land at `h200` is 0.19
// against a measured 0.20. Nothing in `CalculateSunLight` is implicated, and the
// land/sea split does not need a cause of its own either — as a scene scale it
// is 0.25 against 0.51 at height 50 and 0.27 against 0.45 at 164, but 0.25
// against 0.36 at 627 and 0.36 against 0.34 at 1590, so the two surfaces
// converge as the camera rises where the earlier `k` had them crossing over.
//
// **What is left, and where to look next.** The remainder is the single
// scene-wide gain that `SCENE_BRIGHTNESS` in `map::post` already stands for —
// the twelve rows bracket its 0.37 — with the hue rotation and the sea's
// residual offset that `map::post` and `water.wgsl` record. No constant is
// refitted here, because nothing in this file changed: this is a correction to
// what the measurement means, not to what the pass computes. But it does move
// the search. A cause now has to multiply `(ambient + diffuseLight) * _Diffuse`
// as a whole, which is a much narrower shape than "something on the sun", and
// `ComposeLight` has exactly one term of that shape: **`HdrRange`**. It is
// closed too, by name rather than by fit — `FUN_14084d9c0` is the parser that
// reads `"LIGHT_HDR_RANGE"` into `DAT_14331d3b0`, `00_graphics.lua:772` gives it
// 1.0, and `FUN_140ff3bb0` writes that into `cb0 + 0xdc` unless
// `DAT_143317238` is set, in which case it writes the literal 1.0 instead, so
// the constant buffer holds 1.0 either way. (The same parser check names every
// other global quoted above: `DAT_14331f084` is `SUN_INTENSITY`,
// `DAT_14331f120` is `SUN_SPECULAR_INTENSITY`, `DAT_14331e6ec` is `SUN_HEIGHT`,
// `DAT_14331e768` is `SUN_LATITUDE` and `DAT_14331e898` is `SUN_HEIGHT_WATER`.)
// That leaves `_Diffuse` itself — the terrain albedo after the atlas blend, the
// colormap overlay, the snow and the mud — as the only remaining factor of that
// shape inside this pass, and [`super::textures`] records what has already been
// ruled out there: the decode is not a power, and the atlas tiles' own byte
// means are what this port samples. A scale on the albedo that is not a decode,
// or a stage between the scene and the grade that is not in `map::post`, is what
// the next search should be aimed at — not at `CalculateSunLight`.
//
// Scripts: `C:\hoi4_scratch\replay_map\sunterm\` (`terms.py` is the CPU mirror
// of the day factor and the light vector, `probe.py` forces the shadow term,
// `render.py` renders, `gamescene.py` inverts the grade, `fit.py` and `bins.py`
// measure).
//
// ## The pass itself is drawn now, and it moves nothing by much (2026-09-14)
//
// What was still owed to the port, and is a separate and much smaller thing,
// was the shadow pass below height 400 that the paragraph above leaves open.
// `map::shadow` draws it: the 2048-square depth map, whose casters are the
// trees and the map-object meshes and **not** the terrain — `pdxmap.shader`
// declares no shadow effect, there is no `terrainshadow` in the binary and the
// depth pass never calls the terrain renderer, so the heightfield cannot shade
// itself and the relief in the game's frames is `N.L` — the screen-space
// `treeunlit`/`terrainunlit` pre-pass, and its two blurs. `--no-shadows` turns
// it off, which is the game's own `Draw.Shadows` console toggle, so the pair is
// measurable in one binary. Over the same masks and the same pairE frame as the
// table at the head of this file, medians and game-over-ours ratios with the
// chain off and on:
//
// | surface | off | ratio | on | ratio |
// |---|---|---|---|---|
// | bare terrain (844k px) | 0.259 0.310 0.169 | 0.236 0.249 0.319 | 0.255 0.306 0.165 | 0.240 0.252 0.326 |
// | under trees (208k) | 0.255 0.306 0.184 | 0.361 0.349 0.542 | 0.247 0.298 0.180 | 0.373 0.358 0.554 |
// | under cities (154k) | 0.286 0.322 0.216 | 0.418 0.401 0.588 | 0.282 0.314 0.212 | 0.424 0.411 0.599 |
// | open water (294k) | 0.251 0.286 0.345 | 0.412 0.445 0.536 | 0.251 0.286 0.345 | 0.412 0.445 0.536 |
// | whole frame (1.72M) | 0.251 0.294 0.196 | 0.290 0.308 0.356 | 0.247 0.290 0.192 | 0.295 0.312 0.363 |
//
// The land ratio moves 0.236 to 0.240 and the tree ratio 0.361 to 0.373; open
// water does not move at all, because nothing in the reference frame casts onto
// the open sea and the term there is 1 either way. So the scene scale the
// section above settles on is untouched in kind and barely touched in size: a
// refit of `SCENE_BRIGHTNESS` on this frame would want 1.7 % less of the scene
// than before, which is inside the bracket the twelve rows already put round
// its 0.37. The shadow pass is not the missing factor, and it was never a
// candidate to be — it multiplies the sun alone, and the section above is what
// rules that shape out.
//
// That is the predicted size. The whole range of the pass is a factor between
// 0.3 and 1 on the sun of the pixels a caster occludes, and at the reference
// framings the occluded pixels are a small share of the frame: 17 % at height
// 164, where the buildings and the sprawl still draw, and 0.7 % at height 303,
// where they do not and only the canopy casts.
//
// ## The albedo is not a scale either: the gap is per atlas tile (2026-09-15)
//
// The section above leaves `_Diffuse` as the last term of the right shape, and
// the search that followed it asked whether the palette-to-tile mapping or the
// atlas layout is wrong, since a wrong tile for forest would look like a
// land-wide albedo error. **They are not.** `super::textures::atlas_lookup`
// carries the confirmation: `texture = N` of `common/terrain/00_terrain.txt` is
// the byte, `calculate_map_tex_index` cuts it into `(N mod 4, N div 4)` — the
// cached game shader's body, `MAP_NUM_TILES` 4 and all, is this file's
// expression for expression — and decoding `atlas0.dds` and cutting it into its
// sixteen 512-square tiles names each one as the terrain that claims it. The
// two warnings the loader printed on every dump were noise and now say so: the
// seven undeclared palette indices cover 53 of `terrain.bmp`'s 11,534,336
// pixels, and palette index 14, the `texture = 255` one, is painted nowhere at
// all.
//
// What the same masks do show is that the remaining gap is **not** one scale.
// Masking both frames by the terrain type each pixel stands on — `terrain.bmp`
// projected through the fitted `pairE` camera (3009.95, 1567.92, 163.934),
// keeping only texels whose whole `all_same` neighbourhood shares one tile so
// no fragment mixes two, with the game's interface and this viewer's counter
// and name layers masked out — the game's frame demands a different factor of
// each type. Scene space, the game's `pairE_04_notrees` carried back through
// the trusted grade against our `--no-trees --postfx off` dump, so the fill is
// on, the canopy off and the shadow chain on in both:
//
// | tile | terrain | px | game scene | this viewer | game / ours |
// |---|---|---|---|---|---|
// | 1 | `terrain_0` plains | 186,113 | 0.1059 0.1076 0.0943 | 0.3234 0.3529 0.2250 | 0.327 0.305 0.419 |
// | 4 | `terrain_1` forest, `jungle_18` | 50,360 | 0.0445 0.0493 0.0457 | 0.1840 0.2322 0.1461 | 0.242 0.212 0.313 |
// | 10 | `forest_13` urban | 5,611 | 0.1070 0.1000 0.0941 | 0.2936 0.2801 0.2319 | 0.364 0.357 0.406 |
// | 0 | `terrain_5`, `plains_17` plains | 4,720 | 0.1039 0.0958 0.0840 | 0.2589 0.2887 0.1854 | 0.401 0.332 0.453 |
// | 5 | `terrain_4` forest | 732 | 0.1221 0.0778 0.0745 | 0.2801 0.3616 0.1795 | 0.436 0.215 0.415 |
// | 3 | `desert_mountain` hills | 379 | 0.0442 0.0494 0.0327 | 0.2164 0.2463 0.1132 | 0.204 0.200 0.289 |
// | all | | 247,999 | 0.0933 0.0952 0.0841 | 0.2928 0.3253 0.2081 | 0.319 0.292 0.404 |
//
// The two rows with the samples — plains at 0.305 green and forest at 0.212 —
// are a factor of 1.44 apart, and the split is geographic nowhere: over the
// frame's four quadrants the plains ratio is 0.295, 0.315, 0.309, 0.305 and the
// forest ratio 0.174, 0.271, 0.212, 0.190. Read as a contrast rather than as
// two levels, the game's forest sits at **0.42, 0.51, 0.49, 0.40 of its own
// plains** where ours sits at 0.71, 0.59, 0.72, 0.64.
//
// That contrast is exactly what the two tiles hold, in the two colour spaces
// the atlas could be read in. Tile 4 against tile 1, green, over `atlas0.dds`:
// **0.622 as bytes, 0.444 decoded**. This viewer draws 0.66 and the game draws
// 0.45. The same reading for the other tiles the frame carries (byte, then
// decoded, each against tile 1): tile 0 0.905 / 0.819, tile 3 0.880 / 0.764,
// tile 5 0.816 / 0.699, tile 6 0.964 / 0.967, tile 10 0.767 / 0.602, tile 11
// 0.736 / 0.552.
//
// This does not reopen the decode, which [`super::textures`] closed end to end
// and on its own level-binned evidence, and no constant moves here. It does say
// what a cause now has to do, which is narrower than the last section left it:
// the remaining factor is **not** a scene-wide gain, because one gain moves
// every type by the same amount and this frame wants 0.31 on plains and 0.21 on
// forest. Whatever pays for it has to raise the contrast *between* atlas tiles
// by about 1.4 while leaving the frame's level roughly where the eight-framing
// fit already puts it. The graded 8-bit frames say the same thing in the
// smaller numbers the grade compresses them into — game against ours, red green
// blue, same pair: tile 1 78.0 87.7 98.5 against 90.2 103.0 97.4, tile 4
// 29.2 41.3 54.4 against 51.3 69.6 67.5, tile 10 75.2 77.3 88.1 against
// 81.6 86.8 95.0, tile 0 72.1 80.5 89.7 against 73.6 87.3 83.6.
//
// `--dump --no-trees` is the switch that makes the canopy side of this
// measurable in one binary, and it is the game's own `debug_trees`. Scripts:
// `C:\hoi4_scratch\replay_map\tile\` (`bcdec.py` and `atlas.py` decode and cut
// the atlas, `hist.py` counts the palette, `typemask.py` projects the type mask
// and tabulates the graded frames, `scene_types.py` the same in scene space,
// `stability.py` the quadrants).
//
// ## The contrast is the atlas texel squared, and no mechanism produces it (2026-09-15)
//
// The section above leaves one shape: something has to raise the contrast
// *between* atlas tiles by about 1.4 while leaving the frame's level where the
// eight-framing fit puts it. That shape was fitted, and then every mechanism
// this pass has that could carry it was measured. The fit is clean; the
// mechanism is missing; nothing is ported here.
//
// **The fit.** `game_scene = c * (atlas texel)^p` per channel, holding the
// overlay, the lighting and the grade as ported, against the six tiles of the
// table above. Taken on each tile's own byte mean over `atlas0.dds` and on the
// `game / ours` ratio, so the lighting divides out (weighted by the tile's
// pixel count; `p - 1` is the slope of `log(game/ours)` on `log(atlas)`):
//
// | fit | `p` R G B | `c` R G B |
// |---|---|---|
// | six tiles, weighted by px | 1.458 1.728 1.860 | 0.419 0.438 0.801 |
// | six tiles, unweighted | 1.208 1.541 1.521 | 0.371 0.381 0.595 |
// | the four largest, unweighted | 1.629 1.745 1.982 | 0.526 0.498 0.951 |
//
// **One `(c, p)` per channel does not explain the six together.** The weighted
// fit's own residuals — the factor each tile is still out by after it — are
// green 0.993 (`terrain_0` plains), 0.978 (`terrain_1` forest), 1.411
// (`forest_13` urban), 1.163 (`terrain_5` plains), 0.813 (`terrain_4` forest)
// and 0.717 (`desert_mountain` hills). The two tiles with the samples carry the
// fit and the other four do not follow it, and no exponent moves them: the
// hills tile stays at 0.67 to 0.79 of the fit and the urban tile at 1.15 to
// 1.46 under every exponent between 1 and 2.7, including the sRGB curve. The
// urban row has a cause that is not the albedo — the game draws city sprawl
// over it and this viewer does not, which is the same layer the header's first
// table shows at 0.42 against bare terrain's 0.24 — and the hills and the
// second forest row are 379 and 732 pixels.
//
// **Where it does hold it is a decode, and the exponent is close to 2.2.**
// Rebuilding the albedo on the CPU through the real `GetOverlay` — the atlas
// texels of each tile against the colormap values actually under that tile's
// mask, so the blend's non-linearity is evaluated pixel by pixel — the
// exponent that closes plains against forest is 1.66 2.27 2.71 with
// `c` 0.454 0.528 1.333. Rendered rather than predicted, `pow(2.2)` on the
// texel at the sampling site (before the four-tile blend, the overlay, the
// snow, the mud and the lighting) takes the plains-over-forest spread of the
// demanded factor from **1.354 1.437 1.340 to 1.026 1.076 1.168** and brings
// the hills tile back with it:
//
// | tile | terrain | game / ours, ported | game / ours, texel `^2.2` |
// |---|---|---|---|
// | 1 | `terrain_0` plains | 0.327 0.305 0.419 | 0.503 0.450 0.651 |
// | 4 | `terrain_1` forest | 0.242 0.212 0.313 | 0.491 0.419 0.557 |
// | 10 | `forest_13` urban | 0.364 0.357 0.406 | 0.435 0.431 0.474 |
// | 0 | `terrain_5` plains | 0.401 0.332 0.453 | 0.642 0.504 0.690 |
// | 5 | `terrain_4` forest | 0.436 0.215 0.415 | 0.930 0.420 0.824 |
// | 3 | `desert_mountain` hills | 0.204 0.200 0.289 | 0.445 0.429 0.725 |
// | all | | 0.319 0.292 0.404 | 0.503 0.447 0.634 |
//
// The forest-over-plains contrast the game's own frame carries is
// 0.420 0.458 0.485; this viewer draws 0.569 0.658 0.649 and with the texel
// squared draws 0.431 0.493 0.566. Green is closed to 7 %, red to 3 %, blue
// only to 17 % — blue's own closing exponent is about 3.3, which is not a
// gamma of any kind, and the per-channel exponent 1.66 2.27 2.71 fitted on the
// CPU mirror does **not** transfer to the render (it leaves 1.109 1.065 1.160,
// worse in red than a flat 2.2), because the mirror has no specular and no
// normal.
//
// **And it does not explain the level.** The whole-frame land factor only moves
// from 0.319 0.292 0.404 to 0.503 0.447 0.634, so `SCENE_BRIGHTNESS` survives —
// refitted, it goes from 0.37 to about 0.45. This is a contrast law and not a
// level law, which is the test this section was set.
//
// ### The mechanisms, each measured
//
// A decode of the atlas texel is what the numbers describe. Nothing in the
// engine performs one, and these are the candidates that were open:
//
// * **The BC3 decode and the mip chain.** A hardware decode is linear in the
//   byte domain, and the file's own mips carry the byte domain unchanged: the
//   forest-over-plains green mean of `atlas0.dds` is 0.6217 at mip 0 and
//   0.6221 at mip 6, flat to 0.5 % over all seven levels this pass can select,
//   and the other tiles' ratios move by less. So no filtering choice, and no
//   difference between our lod and the game's, can move a tile's mean.
// * **`texture_quality`.** `FUN_141249190` loads `map/terrain/atlas%d.dds` with
//   the settings byte, and the reference profile's `settings.txt` has
//   `texture_quality=0`. The three files are the same content at three sizes —
//   tile-4-over-tile-1 green is 0.622 in all three — so the suffix cannot be it
//   either, and the game ships exactly one `atlas0.dds` with no mod enabled
//   (`dlc_load.json` is empty).
// * **`GetOverlay`.** Read out of the reference run's own cached terrain pixel
//   shader rather than inferred: `ToGamma` is `pow(x, 0.45)`, `ToLinear` is
//   `pow(x, 2.2)`, the branch is per channel on **`vOverlayGamma`** — the
//   colormap, not the atlas — and the return is `lerp( vColor, res, 0.75 )`
//   (`00080000F7A2100080E174D8.scache:535-546`). `common.wgsl`'s `get_overlay`
//   is that, operand for operand. Feeding it a decoded colormap instead of a
//   decoded texel was measured and does the wrong thing: it pushes every tile
//   from the screen branch into the multiply branch, which darkens the albedo
//   by 2.5x but leaves the contrast where it was — the plains-over-forest
//   spread stays at 1.323 1.319 1.346 against the ported 1.354 1.437 1.340.
// * **The terrain colormap's own values.** This was the cheapest candidate and
//   the most likely, and it is real but already drawn. Over the visible land of
//   the `pairE` frame, `colormap_rgb_cityemissivemask_a.dds` *is* darker over
//   the forest and the hills than over the plains — tint means 0.2069 0.2596
//   0.1214 under tile 1 against 0.1857 0.2325 0.0851 under tile 4 and
//   0.1789 0.2103 0.0706 under tile 3, that is 0.90 0.90 0.70 and
//   0.87 0.81 0.58 of the plains tint. But this pass samples it and blends it
//   at 0.75 in both frames, so it is in this viewer's contrast already; it
//   explains part of why the ported contrast is 0.66 rather than the atlas's
//   own 0.62, not why the game's is 0.45. (`tile/cmap.py`.)
// * **The snow, the mud and the province secondary mask.** Neutralised all
//   three at once and re-rendered the same frame: every per-tile factor moves
//   by at most 0.004 and the plains-over-forest spread goes 1.354 1.437 1.340
//   to 1.330 1.434 1.332. The snow's per-tile gate is `1 - vSpec` with `vSpec`
//   the atlas normal's alpha, which is 0.584 over the plains tile and 0.816
//   over the forest one, so the snow term is the *wrong way round* for this gap
//   even before its size is measured: it brightens forest harder than plains.
// * **The pass itself.** `ComposeLight` and `ComposeLightSnow` are
//   `((ambient + diffuse) * _Diffuse) * HdrRange + specular`
//   (`scache:1055-1082`), linear in the albedo, so a contrast that is not in
//   the albedo cannot be in the lit result either. The one term of the cached
//   shader this port omits is `CalculatePointLights`, which only adds light.
// * **Cast shadows of the trees the game's `debug_trees` removes**, which would
//   darken exactly the forest tiles. Cropping the same forest patch out of both
//   frames at 2x shows the *same* tile texture at the same scale with no
//   tree-shaped occlusion in either — the game's is uniformly darker and bluer,
//   which is a tone difference and not an added shadow.
//
// ### Scored end to end, because the previous test conflated two textures
//
// [`super::textures`] rejects an sRGB view of the atlas, on format evidence
// that is per texture and stands, and its end-to-end table scores "atlas +
// colormap" together. The atlas **alone** was never scored, and the bullet
// above shows the colormap half is what breaks: it moves the level without
// moving the contrast, and [`super::textures`] separately records that it
// breaks the brightest sextile. So the atlas-alone variant was swept against
// `SCENE_BRIGHTNESS` refitted for it, best against best. Mean absolute error:
//
// | variant | gain | `03.md` | `02.md` | `04.md` | `05.md` |
// |---|---|---|---|---|---|
// | this head | 0.37 | 25.0852 | 21.4115 | 20.8258 | 18.2330 |
// | texel `^2.2` | 0.40 | 21.8721 | 18.8792 | 20.8255 | 20.9907 |
// | texel `^2.2` | **0.45** | **20.8161** | **18.1164** | 18.9874 | 19.1019 |
// | texel `^2.2` | 0.48 | 21.4187 | 18.9339 | **18.7297** | 18.2385 |
// | texel `^2.2` | 0.50 | 22.3652 | 19.9402 | 18.9758 | 17.8344 |
// | texel `^2.2` | 0.55 | 25.9657 | 23.3053 | 20.7508 | **17.4292** |
//
// At its own best gain of 0.45 it is better on three of the four pairs (by
// 4.27, 3.30 and 1.84 of mean absolute error) and worse on `05.md` by 0.87; its
// delta E improves on all four (16.68 to 14.13, 14.67 to 12.61, 13.09 to 11.97,
// 11.91 to 11.91) and its SSIM on three (0.6734 to 0.6843, 0.6969 to 0.7122,
// 0.6536 to 0.6628, and 0.6760 to 0.6624 on `05.md`). That is a broader
// improvement than the "atlas + colormap" row it replaces, which lost `04.md`
// by 4.3.
//
// **It is still not ported**, and the reason is the one [`super::textures`]
// gives: the shipped header is a legacy `DDPIXELFORMAT` with `DXT5`, the
// loader's enum switch has no path to `BC3_UNORM_SRGB`, the shader-resource
// view takes the same enum, and `PixelShaderTerrain` applies no `ToLinear` to
// any sample. A `pow(2.2)` in this file would be a constant with a fit behind
// it and no mechanism, and the frame does not even ask for one number — red
// wants about 2.2, green about 2.4 and blue about 3.3.
//
// **What the next attempt should start from.** The transform the game's frame
// demands is a contrast expansion of roughly the square, applied to the atlas
// texel before the overlay, the snow, the lighting and the blend, and *not* to
// the colormap; it is compatible with the level-independence
// [`super::textures`] measures, because binning by our own final level mixes
// tiles and a per-texel power does not survive that mixing. The sea is not
// governed by it: carrying the land's fitted `(c, p)` onto
// `underwater_terrain_0.dds`'s own mean (0.177 0.219 0.244) predicts a water
// factor of 0.190 0.144 0.238 against the 0.412 0.445 0.536 measured, short by
// 2.2 to 3.1x, and `colormap_water_0.dds` (0.191 0.302 0.399) gives the same
// answer — consistent with `water.wgsl`, which accounts for the sea's gap in
// the reflection term instead. So the cause to look for is one that acts on the
// land atlas and on nothing else.
//
// Scripts: `C:\hoi4_scratch\replay_map\tile\` (`cmap.py` reads the colormap
// under each tile's mask, `transfer.py` fits `(c, p)` and rebuilds the albedo
// through `GetOverlay`) and `C:\hoi4_scratch\replay_map\atlaspow\`
// (`patch.py` raises the texel to a power at the sampling site, `gain.py` sets
// the constant, `run.sh` and `sweep.sh` render and score, `tiles.py` rebuilds
// the per-tile table for any dump, `crop.py` cuts the patch comparison).

/// `SHADOW_WEIGHT_TERRAIN` (`constants.fxh:170`), the share of the sun the
/// screen-space shadow buffer is allowed to take: a fully shadowed pixel keeps
/// `1 - SHADOW_WEIGHT_TERRAIN` of it.
const SHADOW_WEIGHT_TERRAIN: f32 = 0.7;

@group(1) @binding(0) var terrain_diffuse: texture_2d<f32>;
@group(1) @binding(1) var height_normal: texture_2d<f32>;
@group(1) @binding(2) var terrain_color_tint: texture_2d<f32>;
@group(1) @binding(3) var snow_texture: texture_2d<f32>;
@group(1) @binding(4) var terrain_normal: texture_2d<f32>;
@group(1) @binding(5) var terrain_id_map: texture_2d<f32>;
@group(1) @binding(6) var province_secondary_color_map: texture_2d<f32>;
@group(1) @binding(7) var snow_mud_data: texture_2d<f32>;
@group(1) @binding(8) var city_lights_and_snow_noise: texture_2d<f32>;
@group(1) @binding(9) var mud_normal_spec: texture_2d<f32>;
@group(1) @binding(10) var mud_diffuse_gloss: texture_2d<f32>;
@group(1) @binding(11) var gradient_border_channel1: texture_2d<f32>;
@group(1) @binding(12) var gradient_border_channel2: texture_2d<f32>;
// The `underwater` effect rebinds slots 0 and 2 to these two textures.
@group(1) @binding(13) var underwater_terrain: texture_2d<f32>;
@group(1) @binding(14) var colormap_water: texture_2d<f32>;
// The engine uploads per-vertex heights; the grid fetches them instead.
@group(1) @binding(15) var heightmap: texture_2d<f32>;
@group(1) @binding(16) var linear_wrap: sampler;
@group(1) @binding(17) var point_clamp: sampler;
@group(1) @binding(18) var linear_clamp: sampler;

struct TerrainVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) uv2: vec2<f32>,
    @location(2) prepos: vec3<f32>,
}

/// `world y = heightmap byte * 0.1` (R1 section 1.3). The heightmap texture is
/// stored with row 0 at the north edge, so a world `z` reads row `2047 - z`.
fn terrain_height(x: f32, z: f32) -> f32 {
    let column = i32(fmod_loop(x, MAP_SIZE_X));
    let row = clamp(i32(MAP_SIZE_Y - 1.0 - z), 0, i32(MAP_SIZE_Y) - 1);
    return textureLoad(heightmap, vec2<i32>(column, row), 0).r * 255.0 * 0.1;
}

@vertex
fn vertex_main(@builtin(vertex_index) index: u32) -> TerrainVertex {
    var corners = array<vec2<u32>, 6>(
        vec2<u32>(0u, 0u),
        vec2<u32>(1u, 0u),
        vec2<u32>(0u, 1u),
        vec2<u32>(1u, 0u),
        vec2<u32>(1u, 1u),
        vec2<u32>(0u, 1u),
    );
    let columns = max(globals.counts.x, 2u) - 1u;
    let quad = index / 6u;
    let corner = corners[index % 6u];
    let x = globals.grid.x + f32(quad % columns + corner.x) * globals.grid.z;
    let z = globals.grid.y + f32(quad / columns + corner.y) * globals.grid.z;

    var out: TerrainVertex;
    out.prepos = vec3<f32>(x, terrain_height(x, z), z);
    // `pdxmap.shader:198-201`, with every `*_POW2_*` at 1.0.
    out.uv = vec2<f32>((x + 0.5) / MAP_SIZE_X, (z + 0.5) / MAP_SIZE_Y);
    out.uv2 = vec2<f32>((x + 0.5) / MAP_SIZE_X, (z + 0.5 - MAP_SIZE_Y) / -MAP_SIZE_Y);
    out.position = globals.frame.view_projection * vec4<f32>(out.prepos, 1.0);
    return out;
}

@fragment
fn terrain_main(input: TerrainVertex) -> @location(0) vec4<f32> {
    // `pdxmap.shader:232` carries this clip commented out, yet the sea bed the
    // `underwater` effect draws would never be visible without it: the water
    // plane is fully transparent wherever the heightmap byte is above 95
    // (`pdxwater.shader:281-282`), so an unclipped lit terrain shows through as
    // land-coloured sea. The engine classifies its LOD quads on the CPU
    // instead, which the shader files do not show and which R1 section 5.3
    // leaves UNVERIFIED. Splitting the two passes exactly at WATER_HEIGHT is
    // the only division consistent with that shore alpha.
    if input.prepos.y < WATER_HEIGHT {
        discard;
    }
    let offsets = vec2<f32>(-0.5 / MAP_SIZE_X, -0.5 / MAP_SIZE_Y);
    let ids = textureSample(terrain_id_map, point_clamp, wrap_u(input.uv + offsets));
    let index = calculate_map_tex_index(ids);

    var tile_repeat = input.uv2 * TERRAIN_TILE_FREQ;
    tile_repeat.x *= MAP_SIZE_X / MAP_SIZE_Y;

    let lod = clamp(mipmap_level(tile_repeat) - 0.5, 0.0, 6.0);
    let mip_texels = pow(2.0, ATLAS_TEXEL_POW2_EXPONENT - lod);

    // `pdxmap.shader:247`, verbatim. The map's world z runs opposite to the
    // bitmap's rows, so the `.g` channel that lands in world z could have needed
    // a sign flip; it does not. Negating it was measured on both reference pairs
    // and is much worse (daylight mae 16.74 to 23.06, night 36.05 to 58.49), and
    // the engine applies no negation either.
    var normal = normalize(textureSample(height_normal, linear_wrap, input.uv2).rbg - vec3<f32>(0.5));

    let uv_w = sample_terrain_uv(index.index_u.w, index.index_v.w, tile_repeat, mip_texels);
    var diffuse = textureSampleLevel(terrain_diffuse, linear_wrap, uv_w, lod);
    var glossiness = diffuse.a;
    var terrain_normal_raw = textureSampleLevel(terrain_normal, linear_wrap, uv_w, lod);

    if index.all_same < 1.0 {
        let uv_x = sample_terrain_uv(index.index_u.x, index.index_v.x, tile_repeat, mip_texels);
        let uv_y = sample_terrain_uv(index.index_u.y, index.index_v.y, tile_repeat, mip_texels);
        let uv_z = sample_terrain_uv(index.index_u.z, index.index_v.z, tile_repeat, mip_texels);
        let color_rd = textureSampleLevel(terrain_diffuse, linear_wrap, uv_x, lod);
        let color_lu = textureSampleLevel(terrain_diffuse, linear_wrap, uv_y, lod);
        let color_ru = textureSampleLevel(terrain_diffuse, linear_wrap, uv_z, lod);
        let normal_rd = textureSampleLevel(terrain_normal, linear_wrap, uv_x, lod);
        let normal_lu = textureSampleLevel(terrain_normal, linear_wrap, uv_y, lod);
        let normal_ru = textureSampleLevel(terrain_normal, linear_wrap, uv_z, lod);

        let frac_uv = fract(vec2<f32>(
            input.uv.x * MAP_SIZE_X - 0.5,
            input.uv.y * MAP_SIZE_Y - 0.5,
        ));
        diffuse = mix(
            mix(color_ru, color_lu, frac_uv.x),
            mix(color_rd, diffuse, frac_uv.x),
            frac_uv.y,
        );
        terrain_normal_raw = mix(
            mix(normal_ru, normal_lu, frac_uv.x),
            mix(normal_rd, terrain_normal_raw, frac_uv.x),
            frac_uv.y,
        );
    }

    let spec_in = terrain_normal_raw.a;
    glossiness = diffuse.a;
    let tile_normal = normalize(terrain_normal_raw.rbg - vec3<f32>(0.5));
    normal = normalize(rotate_vector_by_vector(normal, tile_normal));

    let terrain_color = textureSample(terrain_color_tint, linear_wrap, input.uv2);
    let city_lights_mask = terrain_color.a;

    var snow_alpha = 1.0 - spec_in;
    var color = get_overlay(diffuse.rgb, terrain_color.rgb, COLORMAP_OVERLAY_STRENGTH);

    let mud_snow = textureSample(snow_mud_data, linear_wrap, map_uv_bottom_up(input.prepos));
    let snowed = apply_snow(
        color,
        input.prepos,
        normal,
        mud_snow,
        snow_texture,
        city_lights_and_snow_noise,
        linear_wrap,
        glossiness,
        snow_alpha,
    );
    color = snowed.color;
    normal = snowed.normal;
    glossiness = snowed.glossiness;
    snow_alpha = snowed.snow_alpha;

    let muddy = get_mud_color(
        color,
        mud_snow,
        input.prepos,
        normal,
        glossiness,
        spec_in,
        mud_diffuse_gloss,
        mud_normal_spec,
        terrain_color.rgb,
        city_lights_and_snow_noise,
        linear_wrap,
    );
    color = muddy.color;
    normal = muddy.normal;
    glossiness = muddy.glossiness;
    let spec = muddy.spec;

    let gradient = gradient_border_apply(
        color,
        input.uv2,
        gradient_border_channel1,
        gradient_border_channel2,
        linear_clamp,
        1.0,
        globals.frame.gb_cam_dist_override_outline_cutoff.zw,
        globals.frame.gb_cam_dist_override_outline_cutoff.xy,
    );
    let secondary = secondary_color_mask(
        gradient.color,
        input.uv2,
        province_secondary_color_map,
        linear_wrap,
        gradient.bloom_alpha,
    );
    color = secondary.color;
    let bloom_alpha = secondary.bloom_alpha;

    // PDX_IMPROVED_BLINN_PHONG, with NO_SHADER_TEXTURE_LOD off.
    let spec_remapped = spec * spec * 0.4;
    let metalness = 0.0;
    var properties: LightingProperties;
    properties.world_space_pos = input.prepos;
    properties.to_camera_dir = normalize(cam_pos() - input.prepos);
    properties.normal = normal;
    properties.diffuse = metalness_to_diffuse(metalness, color);
    properties.glossiness = glossiness;
    properties.specular_color = metalness_to_spec(metalness, color, spec_remapped);
    properties.non_linear_glossiness = get_non_linear_glossiness(glossiness);

    // `max( GetShadowScaled( SHADOW_WEIGHT_TERRAIN, Input.vScreenCoord,
    // ShadowMap ), 0.1 )`. `map::shadow` fills the buffer below camera height
    // 400 and binds its all-ones clear value above it, where `ShadowFadeFactor`
    // is 0 and the saturate inside `GetShadowScaled` pins the term to 1 anyway.
    let shadow_term = max(get_shadow_scaled(SHADOW_WEIGHT_TERRAIN, input.position), 0.1);
    var light = calculate_sun_light(properties, shadow_term, calculate_sun_direction(input.prepos));
    // No point lights: they come from combats and effects, not from the map
    // itself, and the reference frame has none (R1 section 3.4).

    let eye_dir = normalize(input.prepos - cam_pos());
    // FAKE_CUBEMAP_COLOR is black, so the Fresnel term adds nothing; kept for
    // the same reason the game keeps it.
    light.specular += FAKE_CUBEMAP_COLOR
        * fresnel_glossy(properties.specular_color, -eye_dir, properties.normal, properties.glossiness);

    var out = compose_light_snow(properties, light.diffuse, light.specular, snow_alpha);
    out = mix(out, color, BORDER_LIGHT_REMOVAL_FACTOR * (1.0 - bloom_alpha));

    let globe_normal = calc_globe_normal(input.prepos.xz);
    let night_factor = day_night_factor(globe_normal);

    let city_lights = textureSample(
        city_lights_and_snow_noise,
        linear_wrap,
        input.prepos.xz * CITY_LIGHTS_TILING,
    ).rgb;
    out += city_lights * CITY_LIGHTS_INTENSITY * city_lights_mask * night_factor;

    // `ApplyFOW( vOut, ShadowMap, Input.vScreenCoord )`, the two factors the
    // pre-pass wrote into this pixel's green and blue.
    let fogged = apply_fow_buffer(out, input.position);
    out = mix(fogged, out, BORDER_FOW_REMOVAL_FACTOR * (1.0 - bloom_alpha));
    out = apply_distance_fog(out, input.prepos);
    out = day_night_with_blend(
        out,
        globe_normal,
        mix(BORDER_NIGHT_DESATURATION_MAX, 1.0, bloom_alpha),
    );

    return vec4<f32>(out, saturate(city_lights_mask * night_factor * CITY_LIGHTS_BLOOM_FACTOR));
}

// `PixelShaderUnderwater` (`pdxmap.shader:407-455`), checked against the source
// line by line and measured term by term on four frames; `water.wgsl` carries
// the numbers and the scripts, and this is what the check settled.
//
// The `underwater` effect (`pdxmap.shader:495-499`) names no defines, where
// `terrain` names `PDX_IMPROVED_BLINN_PHONG` and `terrain_low_gfx` names
// `LOW_END_GFX`; the engine compiles it by that name and no other
// (`FUN_141261100` calls the effect loader with `"gfx/FX/pdxmap.lua"` and the
// literal `"underwater"`, exactly as it does for `terrain`, `terrain_low_gfx`
// and `terrainunlit`, and the only other arguments are a vertex declaration and
// an out-list). So this pass takes the plain Phong branch of `CalculateSunLight`
// and `_Glossiness` is the specular exponent directly, which is why
// `MAP_SPECULAR_WIDTH` of 15 is passed to `calculate_sun_light_phong` and not
// through `GetNonLinearGlossiness`.
//
// Two facts the measurements added, both about the sea floor:
//
// * `map/heightmap.bmp` is the byte 89 over every ocean — 7.47 M of its 11.5 M
//   pixels — so `prepos.y` here is 8.9 and `vWaterAlpha` is a constant 0.4675
//   over open water. The `vMin` 5.3 and `vMax` 13.0 the shader calls a "depth
//   fog" never vary: `vMax` is above `WATER_HEIGHT` and the floor never drops
//   below 8.9. Every open-sea refraction the water plane samples is therefore
//   53 % water tint and 47 % lit sea bed, not the tint alone.
// * The sea bed is lit at about 0.60 of its albedo by day where the water
//   surface beside it is lit at 1.02, and that is the engine's own doing rather
//   than a porting slip: this pass calls `CalculateSunDirection`, which uses the
//   virtual sun's real `y`, while `pdxwater.shader` calls
//   `CalculateSunDirectionWater`, which swaps in `*_HEIGHT_WATER` through the
//   `.xwz` swizzle and puts the source nearly overhead.
//
// The game's own `debug_water` toggle takes this draw away together with the
// water plane, and what it leaves is lit terrain: `pdxmap.shader:232`'s
// commented-out clip really is off and the engine draws `PixelShaderTerrain`
// below water too, under this pass. `terrain_main` above discards it instead,
// which is invisible while the underwater draw covers it and is what
// `map/water.wgsl` records against the `pairE` frames.
//
// The two textures were cross-checked against CPU decodes at the probed `uv2`:
// the tint is `colormap_water_0.dds` (matching to within the 8-bit probe's own
// step, which also rules out a flipped `v`) and the diffuse is
// `underwater_terrain_0.dds`, whose whole-file mean 0.1781 0.2171 0.2434 is
// what the shader reads because the file's spatial standard deviation is only
// 0.013. Which sampler slot the engine binds each of them to is inferred from
// the asset set rather than read from the decompile; `water.wgsl` records that
// as the one UNVERIFIED item here.
@fragment
fn underwater_main(input: TerrainVertex) -> @location(0) vec4<f32> {
    if WATER_HEIGHT - input.prepos.y + TERRAIN_WATER_CLIP_HEIGHT < 0.0 {
        discard;
    }

    let normal = normalize(textureSample(height_normal, linear_wrap, input.uv2).rbg - vec3<f32>(0.5));
    let diffuse = textureSample(
        underwater_terrain,
        linear_wrap,
        input.uv2 * vec2<f32>(MAP_SIZE_X / 32.0, MAP_SIZE_Y / 32.0),
    ).rgb;
    let water_color_tint =
        textureSample(colormap_water, linear_wrap, input.uv2).rgb * WATER_COLOR_LIGHTNESS;

    // The SnowTexture slot the shader used to read here is dead in 1.19.2.
    let water_alpha = saturate((input.prepos.y - 5.3) / (13.0 - 5.3));
    let glossiness = MAP_SPECULAR_WIDTH;

    var properties: LightingProperties;
    properties.world_space_pos = input.prepos;
    properties.to_camera_dir = normalize(cam_pos() - input.prepos);
    properties.normal = normal;
    properties.diffuse = diffuse;
    properties.glossiness = glossiness;
    properties.specular_color = globals.frame.sun_diffuse_intensity.rgb * 0.1;
    properties.non_linear_glossiness = get_non_linear_glossiness(glossiness);

    // `GetShadowScaled( SHADOW_WEIGHT_TERRAIN, ... )` without the lit pass's
    // 0.1 floor (`pdxmap.shader:444`).
    let light = calculate_sun_light_phong(
        properties,
        get_shadow_scaled(SHADOW_WEIGHT_TERRAIN, input.position),
        calculate_sun_direction(input.prepos),
    );
    let lit = compose_light(properties, light.diffuse, light.specular);
    return vec4<f32>(mix(water_color_tint, lit, water_alpha), 1.0);
}

// `pdxmap.shader:457-470`: the `terrainunlit` effect, the terrain half of the
// screen-space shadow pre-pass. It shares `VertexShader` with the lit effect,
// so this is the same grid drawn again through the camera, writing what the lit
// passes will read back at their own pixel.
//
// `GetFogFactors` fills `.g` and `.b` with the fog-of-war the recorded
// perspective sees; `common.wgsl`'s `get_fog_factors` is the whole of it, and
// `terrainunlit` passes an `ExtraHeight` of zero where `treeunlit` passes the
// canopy's.
@fragment
fn terrain_unlit_main(input: TerrainVertex) -> @location(0) vec4<f32> {
    let fog = get_fog_factors(input.prepos, 0.0);
    return vec4<f32>(calculate_shadow(shadow_proj(input.prepos)), fog.x, fog.y, 1.0);
}
