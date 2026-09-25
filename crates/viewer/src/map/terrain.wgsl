// `pdxmap.shader`: the `terrain` and `underwater` effects.
//
// The vertex shader replaces the engine's instanced LOD quads with one uniform
// grid over the visible ground rectangle (R1 section 5.2 shows this is
// equivalent and strictly more accurate), so `QuadOffset_Scale_IsDetail` and
// the `vSnap` T-junction stitching have no counterpart here. Everything the
// pixel shaders do is ported literally; each departure is commented.
//
// # The scene's brightness gap was this pass's atlas, and it is closed (2026-09-15)
//
// This section used to open "this pass is where the scene's brightness gap
// lives" and tabulate a factor of three to four between the game's scene and
// this viewer's, inferred by carrying the game's 8-bit frame back through the
// colour grade. The inference was right about the size and right about the
// place. The cause is in `map::textures`, and the capture names it: the engine
// creates `atlas0.dds`, the colormap, the city lights, the mud diffuse, the
// province secondary map and the gradient-border channels with `_SRGB` DXGI
// formats, so the hardware decodes every colour sample this pass takes and this
// port was reading the bytes raw. That header carries the slot table, the
// draw-event ids and the scores.
//
// The gap is now measured against the game's own `R16G16B16A16_FLOAT` scene
// target rather than through an inverted grade — `hoi4_frame33840.rdc` dumped
// after its last terrain draw (event 2861), after its water draw (10338) and
// before its `Restore` (16280), against this viewer's `--postfx off` render at
// the camera that capture's own `vCamPos` names, (3011.74, 1570.27, 263.493).
// Medians, game over ours, before and after the colour spaces were corrected:
//
// | surface | px | before | after |
// |---|---|---|---|
// | whole frame | 2.06 M | 0.353 0.338 0.406 | 1.112 1.014 1.217 |
// | open water | 554 k | 0.384 0.428 0.530 | 1.064 1.034 1.000 |
// | bare land, canopy off | 286 k | 0.307 0.276 0.393 | 1.198 0.976 1.202 |
// | land under the canopy | 95 k | 0.421 0.316 0.481 | 1.580 1.181 1.295 |
// | `terrain_0` plains (tile 1) | 222 k | 0.305 0.285 0.394 | 1.139 0.976 1.203 |
// | `jungle_18` forest (tile 4) | 33 k | 0.323 0.239 0.366 | 1.653 1.281 1.232 |
// | `desert_mountain` hills (tile 3) | 1.6 k | 0.306 0.285 0.437 | 1.310 1.166 1.392 |
// | `forest_13` urban (tile 10) | 11 k | 0.335 0.272 0.357 | 1.109 0.903 0.982 |
//
// Those are ratios of medians over masks whose two distributions do not line up
// pixel for pixel, and per pixel the two surface rows read differently: medians
// of `game / ours` are **1.007 1.004 1.003** on the open water and
// **1.187 1.055 1.201** on the bare land. So the sea is exact and the land's
// factor of 1.2 is real. What is left of the per-tile spread sits on the two
// tiles the canopy covers: the mask can only remove *this viewer's* trees, so a
// pixel the game's canopy covers and ours does not reads game-dark, and the
// canopy row moves the same way. The 8-bit `--postfx off` dump quantises these
// medians at about 4 % a level, which is most of the width of what is left.
//
// `SCENE_BRIGHTNESS` in `map::post`, the single fitted gain this file used to
// point at, is deleted: the scene it graded is now the scene the game grades.
//
// # The land's remaining 1.2 is the gradient-border fill, and this pass is exact (2026-09-15)
//
// The 1.2 was chased into the capture itself rather than fitted: RenderDoc's
// shader debugger was run on the engine's own `PixelShaderTerrain` at chosen
// pixels, stepping all 550 instructions and recording every register, and a
// scratch build of this viewer wrote the matching intermediates to the frame at
// the same camera. The terrain draws of `hoi4_frame33840.rdc` run from event
// 2777 to 2861; `PixelHistory` names event **2819** as the one that shades all
// three land pixels below. Scripts: `C:\hoi4_scratch\replay_map\rdc\py\
// debug_pixel.py` traces, `C:\hoi4_scratch\replay_map\psdebug\` annotates
// the trace against the disassembly, picks the pixels and measures.
//
// **A bare plains pixel, screen (917, 246), world (2991.61, 9.70, 1632.94)**,
// the game's terrain id bytes `(1, 1, 101, 1)` so one tile and no blend. Term
// for term, the game's registers against this viewer's probes, in trace order:
//
// | term | game | ours |
// |---|---|---|
// | atlas sample, after the `_SRGB` view | 0.3433 0.3638 0.2240 | 0.3412 0.3647 0.2235 |
// | `_Glossiness` = its alpha | 0.2573 | 0.2549 |
// | terrain-normal alpha (`vSpec`) | 0.3962 | 0.3961 |
// | colormap tint, after its `_SRGB` view | 0.0265 0.0537 0.0072 | 0.0265 0.0539 0.0069 |
// | `GetOverlay` before the `lerp` | 0.0437 0.0933 0.0079 | - |
// | colour after the overlay | 0.1186 0.1609 0.0619 | 0.1186 0.1618 0.0618 |
// | snow and mud | unchanged | unchanged |
// | normal after the tile rotate | -0.0873 0.9962 0.0001 | -0.0902 0.9922 0.0039 |
// | **gradient-border alpha, 9 taps** | **0.9106** | **0.7098** |
// | gradient-border colour | 0.1943 0.1387 0.1094 | 0.1961 0.1412 0.1137 |
// | **colour after the fill** | **0.1817 0.1424 0.1015** | **0.1647 0.1490 0.0941** |
// | **`vBloomAlpha`** | **0.0759** | **0.3176** |
// | `GetShadowScaled` | 1.0000 | 1.0000 |
// | `N.L` | 0.5486 | 0.5451 |
// | `AmbientLight` | 0.0309 0.0309 0.0607 | 0.0314 0.0314 0.0608 |
// | specular | 0.0063 | 0.0059 |
// | `ComposeLightSnow` | 0.1116 0.0888 0.0681 | 0.1010 0.0922 0.0627 |
// | border light-removal factor | 0.7393 | 0.5459 |
// | after the light removal | 0.1634 0.1284 0.0928 | 0.1363 0.1235 0.0784 |
// | `(FogColorFactor, FogAlphaFactor)` | 0.2118, 0.1137 | 0, 0 |
// | after `ApplyFOW` | 0.1666 0.1323 0.0982 | 0.1363 0.1235 0.0784 |
// | distance-fog factor | 0.02214 | 0.02255 |
// | output | 0.1656 0.1356 0.1093 | 0.1373 0.1255 0.0902 |
//
// Every term this pass owns agrees to the 8-bit probe's own step, the overlay
// and the atlas decode included. The two that do not are the gradient-border
// layer's alpha and, through it, `vBloomAlpha` — and `vBloomAlpha` is the
// larger lever of the two, because `BORDER_LIGHT_REMOVAL_FACTOR * (1 - alpha)`
// mixes that fraction of the *unlit* albedo back over the lit colour, and the
// unlit albedo is about twice the lit one when `ambient + N.L` is 0.58.
//
// **Two pixels the game fills and this viewer does not.** Screen (993, 436),
// world (3006.07, 9.70, 1596.35), and (704, 531), world
// (2952.83, 9.70, 1578.50). At both the game's layer-2 nine-tap alpha is about
// 0.62 and this viewer's is **0.0000**, so the game blends its fill at
// `0.567 * 0.902` and 0.571 * 0.902 and this viewer blends none, and this
// viewer's `vBloomAlpha` is 1 where the game's is 0.433, so this viewer removes
// no light at all. The fill *colour* matches at both (game 0.0825 0.0762 0.0684
// against ours 0.0863 0.0784 0.0667); only the field does not. At (993, 436)
// the game reads 0.0758 0.0865 0.0542 and this viewer 0.0627 0.0824 0.0392, and
// feeding the game's post-fill albedo and `vBloomAlpha` into *this viewer's*
// lighting reproduces the game to **1.004 1.005 1.010**.
//
// **The same split over the whole frame.** Over the 286 k bare-land pixels of
// the capture's framing, medians of `game / ours`:
//
// | bare-land subset | px | game / ours |
// |---|---|---|
// | this viewer's fill at full strength (`vBloomAlpha` 0) | 7.1 k | 1.007 1.008 1.013 |
// | both fields at full strength | 9.2 k | 0.990 0.987 0.990 |
// | `vBloomAlpha` 0.1 to 0.3 | 42 k | 0.993 0.912 1.015 |
// | this viewer's field absent (`vBloomAlpha` above 0.3) | 210 k | 1.234 1.095 1.267 |
//
// Where the two fills agree the frame agrees; where this viewer has no fill the
// whole 1.2 appears. The scene path this file and [`super::common`] carry —
// the atlas decode, `GetOverlay`, the snow and mud, the normals, `AmbientLight`,
// `CalculateSunLight`, the specular, `ComposeLightSnow`, `ApplyFOW`, the
// distance fog and `DayNightWithBlend` — has no residual left in it.
//
// **What the fix is, and whose.** It is [`super::gradient`]'s distance field:
// the engine's layer-2 alpha is about 0.62 over the German interior at this
// camera where this viewer's is 0, so the game's country fill covers the
// interior and this viewer's is a rim at the border. The capture adds two
// constants for the same owner, both already recorded in [`super::frame`]:
// `vGBCamDistOverride_GBOutlineCutoff` is `(1, 0, 0.98, 0.973)` in `cb0`, not
// this viewer's `(0, 0, 0.973, 0.973)`, and the engine's channel 1 is the
// strategic-region field (a flat `(8, 13, 26)` at alpha 1 over all three pixels,
// which the `floor(vOutline)` gate then discards) while channel 2 is the country
// fill — the reverse of this viewer's layer order.
//
// **Done, and the field now matches at all three pixels.** `map::gradient`'s
// two layers are the strategic regions and the countries in that order, a
// pixel in no region of a layer holds alpha 255, and the country thickness
// defines are areas rather than lengths to square, which is what had made the
// fill a rim: at 25 map pixels rather than 5 the field carries to where the
// game's does. Measured on the capture's own `GradientBorderChannel1` at the
// three pixels above, this viewer's layer-2 alpha is now 0.9098, 0.5608 and
// 0.6235 against the game's 0.9098, 0.5608 and 0.6196, where it was 0.7098,
// 0.0000 and 0.0000. `reference/03.md` moves 15.4296 / 9.0116 / 0.6944 to
// 12.3392 / 7.0562 / 0.7262 with it.
//
// The fog of war is the one remaining difference this pass can see and it is a
// recording gap, not a port one: the game's pre-pass buffer carries
// `FogAlphaFactor` 0.1137 at (917, 246) and 0 at the other two, and the
// reference recording is format 14, which has no fog inputs, so
// `map::scene` binds an empty intel map and every `ApplyFOW` here is the
// identity. On the one pixel where the game fogs, that is 2 % of red and 5 % of
// blue.
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
//   `GridStart_InvCellSize`. The last selects tiled point lights (now ported).
//   Both retained captures select only zero-colour entries; this constant is
//   not a whole-scene multiplier.
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
// **What is left, and where to look next.** The remainder is a single
// scene-wide gain of about 0.3, with the hue rotation and the sea's residual
// offset that `map::post` and `water.wgsl` record. (It is the atlas decode, and
// the head of this file closes it; what follows is how the search got there.) No constant is
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
// refit of the scene-wide gain on this frame would want 1.7 % less of the scene
// than before, which is inside the bracket the twelve rows already put round
// it. The shadow pass is not the missing factor, and it was never a
// candidate to be — it multiplies the sun alone, and the section above is what
// rules that shape out.
//
// That is the predicted size. The whole range of the pass is a factor between
// 0.3 and 1 on the sun of the pixels a caster occludes, and at the reference
// framings the occluded pixels are a small share of the frame: 17 % at height
// 164, where the buildings and the sprawl still draw, and 0.7 % at height 303,
// where they do not and only the canopy casts.
//
// ## The albedo was the gap, and the two sections here are withdrawn (2026-09-15)
//
// Two sections stood here. The first measured the gap per atlas tile and found
// it was not one scale — 0.31 on plains against 0.21 on forest in green — and
// concluded that a cause had to raise the contrast *between* tiles by about
// 1.4. The second fitted that shape, found it was the atlas texel raised to
// about 2.2, scored `texel^2.2` against `SCENE_BRIGHTNESS` refitted to 0.45 and
// found it better on three pairs of four, and then refused to port it because
// "a `pow(2.2)` in this file would be a constant with a fit behind it and no
// mechanism, and the frame does not even ask for one number".
//
// That refusal was right and the fit was right. The mechanism exists and is not
// in this file: the engine binds `atlas0.dds` through a `BC3_UNORM_SRGB` view
// (`map::textures`, draw event 2777 of `hoi4_frame33840.rdc`), so the decode is
// the hardware's, it is the sRGB curve rather than a plain power — which is why
// red wanted 2.2, green 2.4 and blue 3.3 when a single exponent was fitted to
// three channels of a curve that is not one — and it applies to the colormap,
// the city lights and the mud diffuse as well, which a `pow` at this file's
// sampling site would have missed.
//
// What the withdrawn sections established and what still stands: the
// palette-to-tile mapping is right (`super::textures::atlas_lookup`, checked
// against `common/terrain/00_terrain.txt` and the cut atlas); the BC3 decode
// and the mip chain carry the byte domain unchanged, so no filtering choice can
// move a tile's mean; `texture_quality` selects three sizes of one content;
// `GetOverlay` is the cached shader's, operand for operand, and its gamma
// branch is on the colormap and not the atlas; the snow, the mud and the
// province secondary mask together move every per-tile factor by at most 0.004;
// and `ComposeLight` is linear in the albedo. Scripts, kept for the record:
// `C:\hoi4_scratch\replay_map\tile\` and `C:\hoi4_scratch\replay_map\atlaspow\`;
// the table above is rebuilt by `C:\hoi4_scratch\replay_map\rdc\layers.py`.
//
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
    let column = i32(fmod_loop(x, globals.map_size.x));
    let row = clamp(i32(globals.map_size.y - 1.0 - z), 0, i32(globals.map_size.y) - 1);
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
    out.uv = vec2<f32>((x + 0.5) / globals.map_size.x, (z + 0.5) / globals.map_size.y);
    out.uv2 = vec2<f32>((x + 0.5) / globals.map_size.x, (z + 0.5 - globals.map_size.y) / -globals.map_size.y);
    out.position = globals.frame.view_projection * vec4<f32>(out.prepos, 1.0);
    return out;
}

@fragment
fn terrain_main(input: TerrainVertex) -> @location(0) vec4<f32> {
    // Native terrain also draws below water (frame33840 pixel histories).
    // The sea-bed material belongs to the separate WaterRefraction target.
    let offsets = vec2<f32>(-0.5 / globals.map_size.x, -0.5 / globals.map_size.y);
    let ids = textureSample(terrain_id_map, point_clamp, wrap_u(input.uv + offsets));
    let index = calculate_map_tex_index(ids);

    var tile_repeat = input.uv2 * TERRAIN_TILE_FREQ;
    tile_repeat.x *= globals.map_size.x / globals.map_size.y;

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
            input.uv.x * globals.map_size.x - 0.5,
            input.uv.y * globals.map_size.y - 0.5,
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
    light = calculate_point_lights(properties, light, true, globals.light_grid);

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

// `PixelShaderUnderwater` (pdxmap.shader:407-455), with its plain Phong
// effect. This writes only the separate eight-bit WaterRefraction target.
// Frame33840 draws 61..145 bind underwater_terrain_0 and colormap_water_0
// as sRGB, and blend half the water tint with the lit floor by
// saturate((world_y - 5.3) / 7.7). The open floor is about y=8.9.
// The native terrain scene is a later draw, including below the waterline.
// [archived rendering notes](#)'s water section records the sampled inputs and pixel histories.
@fragment
fn underwater_main(input: TerrainVertex) -> @location(0) vec4<f32> {
    if WATER_HEIGHT - input.prepos.y + TERRAIN_WATER_CLIP_HEIGHT < 0.0 {
        discard;
    }

    let normal = normalize(textureSample(height_normal, linear_wrap, input.uv2).rbg - vec3<f32>(0.5));
    let diffuse = textureSample(
        underwater_terrain,
        linear_wrap,
        input.uv2 * vec2<f32>(globals.map_size.x / 32.0, globals.map_size.y / 32.0),
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
    var light = calculate_sun_light_phong(
        properties,
        get_shadow_scaled(SHADOW_WEIGHT_TERRAIN, input.position),
        calculate_sun_direction(input.prepos),
    );
    light = calculate_point_lights(properties, light, false, globals.light_grid);
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
