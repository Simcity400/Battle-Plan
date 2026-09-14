// `pdxwater.shader`: the `water` effect, with refractions and without LOW_END_GFX.
//
// The engine draws the sea as a flat plane at WATER_HEIGHT. Its `uv` is the
// flipped, top-down one (`pdxwater.shader:200`), the same coordinate the
// terrain calls `uv2`, so every map-sized texture below is sampled top-down.
//
// # What the sea is made of, term by term
//
// Every term below was written straight to the frame in a scratch build
// (`C:\hoi4_scratch\replay_map\sea\probe_build.patch`, driven by `run_probes.py`)
// and read back over the open sea — every water pixel at least twelve from a
// shore — of four frames: the day pair `reference/03.md` (hour 60828490), the
// night pair `reference/02.md` (60828478), and the two water-heavy pans of the
// same day hour, `pairD_pan_med.png` (camera 2908.65,1225.79,164.229) and
// `pairD_pan_atlantic.png` (2606.77,1225.85,163.873). Linear means, before the
// colour grade:
//
// | term | day | night | med | atlantic |
// |---|---|---|---|---|
// | underwater `TerrainColorTint` x `WATER_COLOR_LIGHTNESS` | 0.169 0.269 0.391 | 0.169 0.269 0.391 | 0.172 0.267 0.351 | 0.090 0.140 0.229 |
// | underwater `TerrainDiffuse` | 0.178 0.216 0.243 | 0.178 0.216 0.243 | 0.178 0.216 0.243 | 0.178 0.216 0.243 |
// | underwater `vWaterAlpha` | 0.4666 | 0.4666 | 0.4664 | 0.4667 |
// | underwater `ComposeLight` | 0.107 0.129 0.152 | 0.100 0.291 0.575 | | |
// | what the underwater pass writes | 0.140 0.204 0.279 | 0.137 0.280 0.477 | 0.165 0.229 0.287 | 0.106 0.144 0.203 |
// | `WaterRefraction` as this shader samples it | 0.140 0.204 0.279 | 0.137 0.280 0.477 | 0.165 0.229 0.287 | 0.106 0.144 0.203 |
// | `ReflectionCubeMap` | 0.463 0.475 0.496 | 0.463 0.475 0.495 | | |
// | `fresnel` | 0.502 | 0.502 | 0.502 | 0.502 |
// | after the fresnel mix | 0.302 0.340 0.388 | 0.300 0.377 0.486 | 0.298 0.337 0.380 | 0.260 0.287 0.329 |
// | after the gradient borders, the 0.2 darkness offset and the secondary mask, that is `_Diffuse` | 0.241 0.272 0.310 | 0.240 0.302 0.389 | 0.238 0.270 0.304 | 0.208 0.229 0.263 |
// | `ComposeLight` | 0.248 0.278 0.327 | 0.112 0.345 0.789 | 0.246 0.279 0.323 | 0.218 0.239 0.281 |
// | `ApplyDistanceFog` then `DayNightWithBlend` | 0.241 0.278 0.340 | 0.077 0.139 0.251 | 0.239 0.279 0.339 | 0.214 0.240 0.292 |
//
// ## The refraction is not the water tint, and the earlier solve is withdrawn
//
// This file used to record that "the refraction really is `colormap_water_0.dds`
// times `WATER_COLOR_LIGHTNESS`" and, from that, that the sea would land where
// the game has it "if the underwater pass hands over the water tint undimmed".
// Both are wrong, and the second only looked plausible because the first was.
//
// `map/heightmap.bmp` carries the byte **89** over the whole sea floor — 7.47 M
// of its 11.5 M pixels are that one value and every open-sea sample is it — so
// `prepos.y` under water is 8.9 everywhere, never the 5.3 `vMin` or anything
// near it. `PixelShaderUnderwater`'s `vWaterAlpha = saturate((y - 5.3) / 7.7)`
// is therefore a *constant* 0.4675 over every ocean on the map, which the probe
// reads back as 0.4666 at all four cameras. The refraction the sea shows is
// 53 % water tint and 47 % lit sea bed, not the tint alone; the "depth fog" the
// shader's own comment describes has no depth to work with, because `vMax` of
// 13.0 is above `WATER_HEIGHT` and the floor never drops below 8.9. Undimming
// the tint would not move the refraction to where that solve wanted it either:
// the same pixels wanted 0.274 0.450 0.784 and the undimmed tint is
// 0.338 0.538 0.782 *before* the sea bed drags 47 % of it toward
// 0.178 0.216 0.243.
//
// ## The refraction copy, the cube and the fresnel are exact
//
// `WaterRefraction` is the underwater pass's own output, at full resolution, in
// the same format, copied between the terrain draw and this one: the two rows
// of the table above agree to the last digit an 8-bit probe can carry, on all
// four frames. `map::scene::Targets` makes both the scene and the copy
// `SCENE_FORMAT` at the frame size and copies one into the other after
// `terrain.draw`, which is the only ordering that can produce that. The copy
// follows the scene's format rather than the output's, so the sea bed reaches
// this shader with the range the terrain pass gave it; the probe those two
// rows were read with was 8-bit, which is why they agree only to its step.
//
// The cube sample is exact as well. Writing `reflect(vEyeDir, normal)` to the
// frame and looking the same direction up in a CPU decode of `reflection.dds`
// (`C:\hoi4_scratch\replay_map\sea\texcheck.py`, `night_water/cube.py`) agrees
// to 0.004, which is the 8-bit probe's own step, and every open-sea pixel lands
// on the `+Y` face. That face really is near-grey: its mip-0 mean is
// 0.355 0.374 0.410 and no face of the cube is more saturated. The fresnel is
// 0.502 because `saturate(0.5 + 0.5 * pow(1 - dot * 0.5, 10))` has 0.5 as its
// floor at normal incidence and only rises toward 1 at grazing angles.
//
// The two underwater textures were cross-checked the same way: the shader's
// `waterColorTint` matches a CPU decode of `colormap_water_0.dds` at the probed
// `uv2` to 0.008 0.010 0.013 over twelve open-sea points — nearest-neighbour on
// the CPU against a bilinear sample of a 2816x1024 texture, so that is the
// quantisation and not a disagreement, and it rules out a flipped `v` — and the
// sampled `TerrainDiffuse` of 0.178 0.216 0.243 is `underwater_terrain_0.dds`'s
// own whole-file mean of 0.1781 0.2171 0.2434. That texture's spatial standard
// deviation is only 0.013, so the flat sea bed the probe reads is the file.
//
// ## What that pins, and what it rules out
//
// With the cube and the fresnel fixed, this shader's output has a floor. The
// reflection alone contributes `0.502 * cube * 0.8 * (ambient + sun)`, about
// 0.190 0.195 0.204 of the daylight scene before any scene gain, and the
// refraction adds only 0.056 0.081 0.111 on top. Inverting the day frame's
// grade — the CPU mirror reproduces this viewer's own graded frame to 0.50 of
// 255, so the inverse is trustworthy — the game's open sea implies a scene of
// 0.067 0.119 0.195. At `SCENE_BRIGHTNESS = 0.32` the cube term alone is
// 0.061 0.062 0.065, which is 91 % of the game's *whole* red and 33 % of its
// blue, so reaching the game's daylight sea by changing the refraction would
// need it to be 0.05 0.33 1.00: more saturated than any texture in the water
// path and at the ceiling in blue. **No value of the refraction, and therefore
// nothing in the underwater pass, can close the daylight sea's colour gap.**
// That is the clearest thing the measurements say, and it is why this pass
// changes nothing.
//
// ## The sun glint, which checks the specular and the sun position directly
//
// `pairD_pan_atlantic.png` has the sun's glint on open water. The brightest
// 31-pixel block of the game's sea is at screen (1210, 953) and ours at
// (1215, 949) — five pixels, on a 2048x1152 frame — and the crop shows the same
// size, the same elongation and the same wave texture inside it. So
// `CalculateSunDirectionWater`, the LEAN moments and the Gaussian are right.
//
// The level was not, and the cause was named, bounded and has since been fixed
// in the pass that owned it. This shader computes up to 3.8 at the glint, and
// the scene target used to be `Rgba8Unorm` where the engine's is
// `R16G16B16A16_FLOAT`, so everything above 1.0 was gone before the grade ever
// ran. It showed as a flat ceiling rather than as a dim glint: the graded core
// read 194.6 of 255 with not one pixel of the whole frame above 200, which no
// dim scene produces. `map::scene::SCENE_FORMAT` is now the engine's float
// format and the graded core reaches 255 like the game's — 1,433 pixels clipped
// white against the game's 3,538, with the centroid still 4.8 px from the
// game's. What remains is extent, not level: at a threshold of 180 the game's
// glint covers 6,639 pixels and ours 3,320, and the game's reaches about 100 px
// further west. That is a width, so it belongs to the Gaussian and the LEAN
// moments in this file or to the sea's own colour gap below, and it is not a
// range problem any more.
//
// ## Sea against land: the split is smaller than a power fit makes it look
//
// The reading this file used to carry — that the sea prefers `p = 2.2` where
// the land prefers `p = 1`, so "the sea is not only dim but flat" — reproduces
// (`powersplit.py`: day sea 25.92 at p=1 to 20.49 at p=2.2, day land 25.38 to
// 25.51; atlantic sea 11.47 to 7.37, atlantic land 31.96 to 45.47). But a power
// is the wrong instrument for it. Our sea has `B > G > R` and our land
// `R > G > B`, so a power raises blue over red at sea and red over blue on
// land, and both surfaces in fact want the *same* rotation toward blue. Fitting
// a per-channel gain on the scene instead, forward through the CPU mirror so
// that nothing depends on inverting the cube (`gainfit.py`, `jointfit.py`, mean
// absolute error):
//
// | frame | sea's own gain | sea | land's own gain | land | one shared gain | sea | land |
// |---|---|---|---|---|---|---|---|
// | day | 0.259 0.412 0.583 | 19.07 | 0.206 0.259 0.412 | 22.40 | 0.231 0.346 0.550 | 21.15 | 26.26 |
// | night | 0.437 0.412 0.346 | 11.10 | 0.327 0.275 0.275 | 20.54 | 0.309 0.327 0.346 | 12.32 | 22.44 |
// | med | 0.367 0.437 0.519 | 13.69 | 0.437 0.367 0.291 | 22.34 | 0.389 0.412 0.389 | 16.97 | 23.96 |
// | atlantic | 0.346 0.367 0.437 | 10.59 | 0.412 0.389 0.583 | 31.28 | 0.346 0.389 0.437 | 10.61 | 33.29 |
//
// One gain for both surfaces costs the sea 0.0 to 3.3 and the land 1.6 to 3.9,
// and the day frame's shared gain of 0.231 0.346 0.550 is the same
// blue-over-red rotation `map::post`'s `SCENE_BRIGHTNESS` already records as
// 0.22 0.32 0.54 for the whole frame. What is left of the split is a level
// difference, not a different correction: the best *uniform* gain is 0.407 for
// the day sea against 0.275 for the day land, 0.381 against 0.294 at night and
// 0.434 against 0.357 on the med pan, so our sea is if anything too *dark*
// beside our land rather than too bright, which is the opposite of what the
// earlier reading said. The atlantic pan is the one frame where the land wants
// the higher gain, 0.434 against 0.381, and its land is 74 k coastal pixels
// against 1.7 M of sea.
//
// The contrast gap is not the sea's either. Over 16-pixel patches of open sea
// the game carries 1.4x to 3.3x our relative standard deviation
// (`structure_sea.py`: day 3.3/2.5/2.6, night 3.1/1.4/1.6, med 1.9/2.1/3.3,
// atlantic 2.1/2.2/2.5), which is the same factor `map::post` records for
// country interiors on land. One scene-wide cause, seen twice.
//
// ## The game's own layer toggles, and what they do and do not settle
//
// `captures\map_ref_20260914\screens\pairE_*` are the same hour
// captured with the game's console toggles (`pairE_README.md`). Their camera is
// not in the manifest and the VP-label fit cannot take it — the counters hide
// most of the icons — so it was fitted from the coastline instead
// (`fitcoast4.py`): unprojecting a screen pixel onto the water plane and reading
// `map/heightmap.bmp` there gives a land/sea mask for any candidate camera with
// no render at all, and `debug_water` supplies the ground truth to fit it
// against, because the pixels that differ between `pairE_03_noicons.png` and
// `pairE_05_nowater.png` are exactly where the water plane drew. That gives
// **centre (3010.15, 1567.85), height 165.32**. The method recovers
// `reference/03.md`'s independently known camera to within one world unit
// (3011.10, 1567.40, 303.375 against 3010.09, 1568.21, 302.960), a render-side
// height sweep at the same centre has its mean-absolute-error minimum at 165
// (34.14 at 80, 30.81 at 140, **28.82 at 165**, 32.04 at 200, 34.88 at 310) and
// a checkerboard of the two frames carries rivers, coasts and borders across
// every block edge. `--dump --no-water` renders our side of it.
//
// **`debug_water` removes the underwater draw as well as the water plane, so
// that frame does not isolate the underwater pass and cannot settle
// `WATER_COLOR_LIGHTNESS`.** Two measurements say so. The sea in
// `pairE_05_nowater.png` is a pale, snow-textured surface, and a scratch build
// that draws `PixelShaderTerrain` below water instead of the underwater pass
// reproduces exactly that character where our underwater pass is flat dark navy
// — which also confirms that `pdxmap.shader:232`'s commented-out clip really is
// off and the engine draws lit terrain under the sea, something this port
// discards. And `debug_terrain`, which blacks the terrain, leaves the open sea
// at 0.91 0.95 1.00 of its normal value; if the water plane refracted the
// terrain the sea would have collapsed to its reflection term. So the
// refraction is the underwater pass, drawn over that terrain, which is the
// arrangement this port already has.
//
// The sea also carries no political fill: over the sea `pairE_h200_fill.png`
// and `pairE_h200_nofill.png` differ by 0.6 0.9 0.8 of a level (mean absolute
// difference 2.8, all of it the border strips at province edges) where the land
// differs by 10.6. That confirms `gradient_border_apply`'s measured no-op at
// sea from the game's side.
//
// What the frames do settle is where the sea's level difference from the land
// enters. At this one camera and this one grade, inverting the game's frame and
// comparing with our own scene (`shoreband2.py`, `pairE_land.py`):
//
// | surface | game / ours |
// |---|---|
// | land | 0.185 0.239 0.363 |
// | open sea, heightmap byte 89 | 0.307 0.455 0.595 |
// | shore band, byte 94, where `1 - waterShore` leaves the plane only 21 % opaque | 0.308 0.393 0.488 |
//
// The shore band is four fifths the underwater pass shown directly, and it
// already carries the sea's whole level difference from the land: the water
// plane does not add it. So whatever makes our sea sit at about 1.6x the land's
// gain is upstream of this file, in the underwater pass's own inputs — and
// every one of those has been checked against the engine's files above. The
// tint, the sea bed, the depth and the lighting are what the engine's shader
// asks for, and they still hand the surface a sea the game grades 1.6x lower
// than it grades the land beside it.
//
// ## UNVERIFIED
//
// * The slot rebinding itself. `pdxmap.shader` gives the `underwater` effect
//   `TerrainDiffuse` at sampler 0 and `TerrainColorTint` at sampler 2, and this
//   port binds `underwater_terrain_0.dds` and `colormap_water_0.dds` there
//   because those are the two files `FUN_141249190` loads for no other pass and
//   because the values they produce are the ones the probe reads back. The call
//   that binds a texture to a sampler slot for the underwater draw is not in
//   the indexed decompile, so the mapping is inferred rather than read. A frame
//   capture of the engine's underwater draw would settle it.
//   `pairE_05_nowater.png` cannot: `debug_water` takes the underwater draw
//   with the plane (above). A console toggle that removes only the plane,
//   or a frame capture, would.
// * Why the daylight scene wants a blue-over-red rotation at all. It is not in
//   this pass and not in the underwater pass, both shown above, and `map::post`
//   shows it is not in the grade. It is the same rotation on land and at sea.
// * The engine's own LOD-quad classification, which decides where it runs
//   `terrain` and where `underwater`; `RENDERING.md` section 3 already records
//   that R1 section 5.3 leaves it open. `debug_water` narrows it: the engine
//   draws lit terrain below water as well, and the underwater draw covers it.
//   Where the two meet — `PixelShaderUnderwater` clips at `WATER_HEIGHT + 3`,
//   so both are live in the three world units above the waterline — is still
//   open, and this port splits them at `WATER_HEIGHT` exactly. It is a coastal
//   band only; every measurement here is on open sea, where the readings agree.
//
// Scripts for all of the above: `C:\hoi4_scratch\replay_map\sea\`
// (`probe.sh`, `run_probes.py`, `implied_sea.py`, `gainfit.py`, `jointfit.py`,
// `powersplit.py`, `structure_sea.py`, `texcheck.py`, `fitcoast4.py`,
// `pairE_compare.py`, `pairE_land.py`, `shoreband2.py`, `score.sh`, `cmp.sh`).

@group(1) @binding(0) var height_texture: texture_2d<f32>;
@group(1) @binding(1) var lean_texture1: texture_2d<f32>;
@group(1) @binding(2) var lean_texture2: texture_2d<f32>;
@group(1) @binding(3) var province_secondary_color_map: texture_2d<f32>;
@group(1) @binding(4) var specular_map: texture_2d<f32>;
@group(1) @binding(5) var water_refraction: texture_2d<f32>;
@group(1) @binding(6) var ice_diffuse: texture_2d<f32>;
@group(1) @binding(7) var ice_noise: texture_2d<f32>;
@group(1) @binding(8) var reflection_cube_map: texture_cube<f32>;
@group(1) @binding(9) var snow_mud_texture: texture_2d<f32>;
@group(1) @binding(10) var gradient_border_channel1: texture_2d<f32>;
@group(1) @binding(11) var gradient_border_channel2: texture_2d<f32>;
@group(1) @binding(12) var linear_wrap: sampler;
@group(1) @binding(13) var linear_clamp: sampler;

struct WaterVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    // Perspective-correct interpolation of a clip position returns the clip
    // position, which is what the engine's TEXCOORD does.
    @location(2) screen_pos: vec4<f32>,
    @location(3) uv_ice: vec2<f32>,
}

@vertex
fn vertex_main(@builtin(vertex_index) index: u32) -> WaterVertex {
    // The plane is flat, so the visible rectangle needs only its two triangles:
    // `uv` is affine in world `xz` and the camera has no roll, so the quad
    // covers exactly the same pixels a subdivided grid would.
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );
    let corner = corners[index % 6u];
    let span = vec2<f32>(
        f32(max(globals.counts.x, 2u) - 1u) * globals.grid.z,
        f32(max(globals.counts.y, 2u) - 1u) * globals.grid.z,
    );
    let x = globals.grid.x + corner.x * span.x;
    let z = globals.grid.y + corner.y * span.y;

    var out: WaterVertex;
    out.pos = vec3<f32>(x, WATER_HEIGHT, z);
    out.position = globals.frame.view_projection * vec4<f32>(out.pos, 1.0);
    out.screen_pos = out.position;
    out.uv = vec2<f32>((x + 0.5) / MAP_SIZE_X, (z + 0.5 - MAP_SIZE_Y) / -MAP_SIZE_Y);
    out.uv_ice = out.uv * vec2<f32>(MAP_SIZE_X, MAP_SIZE_Y) * 0.1;
    return out;
}

fn multi_sample_tex_x(tex: texture_2d<f32>, samp: sampler, uv: vec2<f32>) -> f32 {
    let offset_x = -0.5 / MAP_SIZE_X;
    let offset_y = -0.5 / MAP_SIZE_Y;
    var result = textureSample(tex, samp, uv).x;
    result += textureSample(tex, samp, uv + vec2<f32>(-offset_x, 0.0)).x;
    result += textureSample(tex, samp, uv + vec2<f32>(0.0, -offset_y)).x;
    result += textureSample(tex, samp, uv + vec2<f32>(offset_x, 0.0)).x;
    result += textureSample(tex, samp, uv + vec2<f32>(0.0, offset_y)).x;
    result += textureSample(tex, samp, uv + vec2<f32>(-offset_x, -offset_y)).x;
    result += textureSample(tex, samp, uv + vec2<f32>(offset_x, -offset_y)).x;
    result += textureSample(tex, samp, uv + vec2<f32>(offset_x, offset_y)).x;
    result += textureSample(tex, samp, uv + vec2<f32>(-offset_x, offset_y)).x;
    return result / 9.0;
}

struct IceResult {
    color: vec3<f32>,
    normal: vec3<f32>,
    fade: f32,
}

fn apply_ice(
    color_in: vec3<f32>,
    pos_xz: vec2<f32>,
    normal_in: vec3<f32>,
    mud_snow: vec4<f32>,
    ice_uv: vec2<f32>,
) -> IceResult {
    let diffuse = textureSample(ice_diffuse, linear_wrap, ice_uv);
    let noise = textureSample(ice_noise, linear_wrap, (pos_xz + vec2<f32>(0.5)) * ICE_NOISE_TILING).r;
    let snow = saturate(get_snow(mud_snow));

    var fade = snow * 8.0 * noise * (1.0 - cam_distance(ICE_CAM_MIN, ICE_CAM_MAX));
    // `pos_xz.y` is world z: ice only exists north of 0.74 of the map height.
    fade *= saturate(saturate((pos_xz.y / MAP_SIZE_Y) - 0.74) * 800.0);
    fade = saturate((fade - 0.3) * 10.0);

    return IceResult(
        mix(color_in, ICE_COLOR * diffuse.a, fade),
        normalize(mix(normal_in, normalize(diffuse.rbg - vec3<f32>(0.5)), fade)),
        fade,
    );
}

@fragment
fn water_main(input: WaterVertex) -> @location(0) vec4<f32> {
    let water_height = multi_sample_tex_x(height_texture, linear_wrap, input.uv) / (95.7 / 255.0);
    let water_shore = saturate((water_height - 0.954) * 25.0);

    let water = sample_water(
        input.uv,
        globals.time_half_pixel.x,
        lean_texture1,
        lean_texture2,
        linear_wrap,
    );
    let spec_map = textureSample(specular_map, linear_wrap, input.uv).a;
    var normal = water.normal;
    normal.y += 1.0 - spec_map;
    normal = vec3<f32>(normal.x * spec_map, normal.y, normal.z * spec_map);
    normal = normalize(normal);

    let flatten = spec_map;
    let b = water.b * flatten;
    let m = water.m * flatten * flatten;

    let sun_dir_water = calculate_sun_direction_water(input.pos);
    let h = normalize(normalize(cam_pos() - input.pos).xzy + (-sun_dir_water).xzy);
    let h_wave = h.xy / h.z - b;

    let sigma = m - vec3<f32>(b.x * b.x, b.y * b.y, b.x * b.y);
    let det = sigma.x * sigma.y - sigma.z * sigma.z;
    let e = h_wave.x * h_wave.x * sigma.y + h_wave.y * h_wave.y * sigma.x
        - 2.0 * h_wave.x * h_wave.y * sigma.z;
    var spec = 0.0;
    if det > 0.0 {
        spec = exp(-0.5 * e / det) / sqrt(det);
    }
    // Re-derived against `pdxwater.shader:302-309` and `SampleWater`/`BlendLEAN`
    // (`standardfuncsgfx.fxh:1165-1241`) while hunting the sea's brightness gap,
    // and measured: over the open North Sea at the daylight reference frame the
    // whole specular term reaches only 1/255 of the frame, because `vGlossiness`
    // = spec/9 * (1 - specMap) is a fraction and the `water` effect declares no
    // defines, so `_Glossiness` is the Phong exponent directly and
    // `pow(NdotH, <1)` never concentrates a highlight. The sea's shortfall is
    // not here.
    //
    // Measured again with the intermediates written straight to the frame, at
    // the open North Sea of the night pair: `spec` is exactly 0 there, and it is
    // 0 for a reason rather than by a porting slip. The LEAN moments come back
    // `B = (0.0155, -0.0084)`, `M = (0.0222, 0.0241, 0.0006)`, so the wave-slope
    // covariance is `sigma = (0.0219, 0.0241, 0.0007)`, an rms slope of 0.15;
    // `vSpecMap` is 0.18 out there and the flatten shrinks that by `specMap^2`,
    // to `sigma = (7.7e-4, 8.0e-4, 2e-5)`, `det = 6.2e-7`. The half-vector's
    // slope `HWave` is (0.5, 0.2) with the moon that low, so `e/det` is about
    // 1270 and `exp(-635)` underflows. The engine computes the same zero: this
    // is the Gaussian saying the moon glint is nowhere near this pixel, and only
    // becomes visible where the sun or moon is close to the mirror direction.

    var refractive_uv = (input.screen_pos.xy / input.screen_pos.w) * 0.5 + vec2<f32>(0.5);
    refractive_uv.y = 1.0 - refractive_uv.y;
    // `vTime_HalfPixelOffset.gb`. DX11 and wgpu both sample at texel centres,
    // so the DX9-era half-pixel shift the engine still carries is zero here.
    refractive_uv += globals.time_half_pixel.yz;
    let refraction_scale = saturate(5.0 - (input.screen_pos.z / input.screen_pos.w) * 5.0);
    var refraction_distortion = normal.xz * refraction_scale * 1.80;

    let eye_dir = normalize(input.pos - cam_pos());
    let reflection = reflect(eye_dir, normal);

    var specular_intensity = 0.010;
    var glossiness = (spec / 9.0) * (1.0 - spec_map);

    let reflective_color = textureSample(reflection_cube_map, linear_clamp, reflection).rgb;
    var refractive_color =
        textureSample(water_refraction, linear_clamp, refractive_uv - refraction_distortion).rgb;

    let fresnel_bias = 0.5;
    var fresnel = saturate(dot(-eye_dir, normal)) * 0.5;
    fresnel = saturate(fresnel_bias + (1.0 - fresnel_bias) * pow(1.0 - fresnel, 10.0));
    refractive_color = refractive_color * (1.0 - fresnel) + reflective_color * fresnel;

    let mud_snow = textureSample(snow_mud_texture, linear_wrap, map_uv_bottom_up(input.pos));
    let ice = apply_ice(refractive_color, input.pos.xz, normal, mud_snow, input.uv_ice);
    refractive_color = ice.color;
    normal = ice.normal;
    refraction_distortion *= 1.0 - ice.fade;
    specular_intensity += ice.fade * 0.07;
    glossiness += ice.fade * 20.0;

    // Sea provinces carry no country colour, and `vOutlineMult` is 0 so a layer
    // that does paint one shows with no outline (R2 section 1.9).
    let gradient = gradient_border_apply(
        refractive_color,
        input.uv + refraction_distortion * 0.0075,
        gradient_border_channel1,
        gradient_border_channel2,
        linear_clamp,
        0.0,
        globals.frame.gb_cam_dist_override_outline_cutoff.zw * GB_OUTLINE_CUTOFF_SEA,
        globals.frame.gb_cam_dist_override_outline_cutoff.xy,
    );
    // The engine's deliberate darkness offset: without it the sea is about a
    // quarter too bright (`pdxwater.shader:357-361`, R2 section 1.9).
    refractive_color = mix(gradient.color, vec3<f32>(0.0), 0.2);

    let secondary = secondary_color_mask(
        refractive_color,
        input.uv - refraction_distortion * 0.001,
        province_secondary_color_map,
        linear_wrap,
        gradient.bloom_alpha,
    );
    refractive_color = secondary.color;
    let bloom_alpha = secondary.bloom_alpha;

    var properties: LightingProperties;
    properties.world_space_pos = input.pos;
    properties.to_camera_dir = normalize(cam_pos() - input.pos);
    properties.normal = normal;
    properties.diffuse = refractive_color;
    properties.glossiness = glossiness;
    properties.specular_color = vec3<f32>(specular_intensity);
    properties.non_linear_glossiness = get_non_linear_glossiness(glossiness);

    // Same shadow reasoning as the terrain pass (R1 section 3.4).
    // The `water` effect declares no defines either, so it is the Phong branch.
    let light = calculate_sun_light_phong(properties, 1.0, sun_dir_water);
    var out = compose_light(properties, light.diffuse, light.specular);

    out = apply_fow(out, 0.0, 0.0);
    out = apply_distance_fog(out, input.pos);
    out = day_night_with_blend(
        out,
        calc_globe_normal(input.pos.xz),
        mix(BORDER_NIGHT_DESATURATION_MAX, 1.0, bloom_alpha),
    );
    // `dominance_fx_apply` is skipped: no naval-dominance data is recorded, so
    // its `Texture3` lookup has nothing to read (`RENDERING.md` section 5).

    return vec4<f32>(out, 1.0 - water_shore);
}
