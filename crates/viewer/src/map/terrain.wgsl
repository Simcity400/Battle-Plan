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

    // `shadows=no` leaves nothing to cast and the screen-space shadow buffer the
    // `terrainunlit` pre-pass would fill is all ones, so `GetShadowScaled` and
    // the `max(..., 0.1)` around it collapse to 1 (R1 section 3.4).
    let shadow_term = 1.0;
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

    // Same shadow reasoning as the lit pass, without the 0.1 floor.
    let light = calculate_sun_light_phong(properties, 1.0, calculate_sun_direction(input.prepos));
    let lit = compose_light(properties, light.diffuse, light.specular);
    return vec4<f32>(mix(water_color_tint, lit, water_alpha), 1.0);
}
