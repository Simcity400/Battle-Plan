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
// So the shadow term is closed as a candidate for the brightness gap, and the
// sun-term scalar it would have to be is now measured: **0.17 on land and 0.41
// on water, flat in colour, and within half of itself over a thirtyfold change
// of camera height with no step at the cutoff**.
//
// ## The pass itself is ported, and it moves the gap by a per cent (2026-09-14)
//
// What was still owed to the port, and is a separate and much smaller thing,
// was the shadow pass below height 400. `map::shadow` now draws it: the
// 2048-square depth map with the trees and the map-object meshes as its casters
// — **not** the terrain, which declares no shadow effect and so cannot shade
// itself — the screen-space `terrainunlit`/`treeunlit` pre-pass, and its two
// blurs. `--no-shadows` turns it off, which is the game's own `Draw.Shadows`
// console toggle, so the pair is measurable in one binary. Over the same masks
// and the same pairE frame as the table above, medians and game-over-ours
// ratios with the chain off and on:
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
// the open sea and the term there is 1 either way. So the remaining sun-term
// gap is untouched in kind and barely touched in size: a refit of
// `SCENE_BRIGHTNESS` on this frame would want 1.7 % less of the scene than
// before, and the 0.17 land and 0.41 water scalars stand.
//
// That is the predicted size. The whole range of the pass is a factor between
// 0.3 and 1 on the sun of the pixels a caster occludes, and at the reference
// framings the occluded pixels are a small share of the frame: 17 % at height
// 164, where the buildings and the sprawl still draw, and 0.7 % at height 303,
// where they do not and only the canopy casts.

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

    // ApplyFOW with the neutral factors an omniscient viewer produces.
    let fogged = apply_fow(out, 0.0, 0.0);
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
// The engine's `GetFogFactors` fills `.g` and `.b`. Both are zero for this
// viewer and for the reference profile: `FogAlphaFactor` is the intel map's
// alpha times `FOWFadeFactor`, and `PLAN.md` section 2 makes the viewer
// omniscient while `FUN_140ff3fc0` writes `FOWFadeFactor` as a literal zero
// outside the five fog-of-war map modes. `common.wgsl`'s `apply_fow` records
// the same reading at the other end of the buffer.
@fragment
fn terrain_unlit_main(input: TerrainVertex) -> @location(0) vec4<f32> {
    return vec4<f32>(calculate_shadow(shadow_proj(input.prepos)), 0.0, 0.0, 1.0);
}
