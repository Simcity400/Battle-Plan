// `pdxwater.shader`: the `water` effect, with refractions and without LOW_END_GFX.
//
// The engine draws the sea as a flat plane at WATER_HEIGHT. Its `uv` is the
// flipped, top-down one (`pdxwater.shader:200`), the same coordinate the
// terrain calls `uv2`, so every map-sized texture below is sampled top-down.

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
