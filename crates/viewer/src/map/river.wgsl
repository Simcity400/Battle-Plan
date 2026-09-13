// `gfx/FX/river.shader`, the `river` effect, ported line for line.
//
// The ribbon is a flat strip over the terrain with the same depth trick `border.shader` and
// `strait.shader` use: the clip-space z comes from a position pushed `vCamLookAtDir * 0.05` toward
// the camera, so the strip is depth-tested against the terrain but never intersects it.
//
// Deviations from the game shader, each with the line it replaces:
//
// * `LOW_END_GFX` is off; the `river_low_gfx` effect is not ported.
// * `GetShadowScaled( SHADOW_WEIGHT_RIVER, ... )` is 1: the reference profile is `shadows=no` and
//   there is no shadow map to sample (R1 section 3.4). `SHADOW_WEIGHT_RIVER = 0.4` would scale a
//   shadow value of 1 back to 1, so the term is left out rather than written as `* 1`.
// * `CalculatePointLights` is dropped with its two maps: the lights come from combats and effects,
//   not from the map, and the replay records none (R1 section 3.4, `RENDERING.md` section 5).
// * `ProvinceSecondaryColorMap` (sampler 6) is declared by the game shader and never read, and the
//   `HeightNormal` line (sampler 7) is commented out in the game shader; neither is bound.
// * `ApplyFOW` is the identity while the viewer is omniscient, as in the other passes.
// * `world_offset_x` is added to the vertex before anything else. The engine draws three world-wrap
//   copies by translating the camera (R2 section 2.3); the ribbons never cross the x seam, because
//   the tracer clamps there, so the viewer translates the geometry instead.
// * `vTimeDirectionSeasonLerp` is a `ConstantBuffer(1, 32)` the viewer does not build. Its `.x` is
//   the animation time and its `.y` a direction, and the shader uses only their product; the pass
//   supplies that product. A headless dump runs at time 0, which is what makes it deterministic.
//
// The specification is `C:\hoi4_scratch\replay_map\research\r2\R2-borders-names-icons.md`
// section 3.2 and 3.3; section numbers below refer to it.

/// `SHADOW_WEIGHT_RIVER` (`constants.fxh:174`). Recorded for the shadow pass; see the header.
const SHADOW_WEIGHT_RIVER: f32 = 0.4;
/// `SNOW_ICE_NOISE_TILING` (`constants.fxh:84`).
const SNOW_ICE_NOISE_TILING: f32 = 0.0625;
/// `SNOW_WATER_COLOR` (`constants.fxh:79`).
const SNOW_WATER_COLOR: vec3<f32> = vec3<f32>(0.3, 0.6, 1.0);
/// `loltime` in `river.shader`'s pixel shader: the scale on every wave layer's time multiplier.
const RIVER_TIME_SCALE: f32 = 0.4;
/// The `0.05` `river.shader` folds into `vTimeDirectionSeasonLerp.x * .y` before `SampleWater`.
const RIVER_ANIMATION_SCALE: f32 = 0.05;
/// `Out.vUV.x *= 0.15f`: how often the bank strip tiles along the flow.
const RIVER_ALONG_TILING: f32 = 0.15;
/// `texCUBE( ReflectionCubeMap, ... ).rgb * 1.3`.
const RIVER_REFLECTION_GAIN: f32 = 1.3;
/// `fresnelBias` in the pixel shader.
const RIVER_FRESNEL_BIAS: f32 = 0.5;

@group(1) @binding(0) var water_color_map: texture_2d<f32>;
@group(1) @binding(1) var river_diffuse: texture_2d<f32>;
@group(1) @binding(2) var river_normal_map: texture_2d<f32>;
@group(1) @binding(3) var river_data: texture_2d<f32>;
@group(1) @binding(4) var lean_texture1: texture_2d<f32>;
@group(1) @binding(5) var lean_texture2: texture_2d<f32>;
@group(1) @binding(6) var snow_mud_texture: texture_2d<f32>;
@group(1) @binding(7) var city_lights_and_snow_noise: texture_2d<f32>;
@group(1) @binding(8) var gradient_border_channel1: texture_2d<f32>;
@group(1) @binding(9) var gradient_border_channel2: texture_2d<f32>;
@group(1) @binding(10) var reflection_cube_map: texture_cube<f32>;
@group(1) @binding(11) var linear_wrap: sampler;
@group(1) @binding(12) var linear_clamp: sampler;
/// `AddressU = Wrap, AddressV = Clamp`, which `Diffuse` and `NormalMap` declare and nothing else in
/// the map passes needs: the bank strip repeats along the flow and must not repeat across it.
@group(1) @binding(13) var river_strip: sampler;

/// The viewer's own block, in place of `ConstantBuffer( 1, 32 )`; see the header.
struct RiverConstants {
    /// World x translation of this world-wrap copy.
    world_offset_x: f32,
    /// `vTimeDirectionSeasonLerp.x * vTimeDirectionSeasonLerp.y`.
    time_direction: f32,
    pad0: vec2<f32>,
}

@group(1) @binding(14) var<uniform> river: RiverConstants;

struct RiverVarying {
    @builtin(position) position: vec4<f32>,
    /// `Out.vUV`, already swizzled: `x` along the flow and tiled, `y` across the ribbon.
    @location(0) uv: vec2<f32>,
    @location(1) tangent: vec2<f32>,
    /// `Out.vPrePos_Fade`: the world position in `xyz` and the end fade in `w`.
    @location(2) pre_pos_fade: vec4<f32>,
    @location(3) world_uv: vec2<f32>,
    @location(4) transparency: f32,
}

/// The z row of `ViewProjectionMatrix`, i.e. `GetMatrixData( ViewProjectionMatrix, 2, 0..3 )`.
fn river_clip_z_row() -> vec4<f32> {
    let m = globals.frame.view_projection;
    return vec4<f32>(m[0].z, m[1].z, m[2].z, m[3].z);
}

fn river_cam_look_at_dir() -> vec3<f32> {
    return globals.frame.cam_look_at_dir_global_time.xyz;
}

@vertex
fn river_vertex(
    @location(0) position: vec4<f32>,
    @location(1) uv_tangent: vec4<f32>,
) -> RiverVarying {
    let pos = vec4<f32>(position.x + river.world_offset_x, position.y, position.z, 1.0);

    var out: RiverVarying;
    out.transparency = position.w;
    out.pre_pos_fade = vec4<f32>(pos.xyz, saturate(1.0 - uv_tangent.y));

    let distorted = pos - vec4<f32>(river_cam_look_at_dir() * 0.05, 0.0);
    let clip = globals.frame.view_projection * pos;
    // Move z slightly closer to the camera to avoid intersections with terrain.
    out.position = vec4<f32>(clip.xy, dot(distorted, river_clip_z_row()), clip.w);

    // `Out.vUV.yx = v.vUV_Tangent.xy; Out.vUV.x *= 0.15f;` — see `rivers::RiverVertex` for why the
    // mesh already stores the across coordinate in `.x` and the along-flow one in `.y`.
    out.uv = vec2<f32>(uv_tangent.y * RIVER_ALONG_TILING, uv_tangent.x);
    out.tangent = uv_tangent.zw;

    out.world_uv = vec2<f32>(
        (out.pre_pos_fade.x + 0.5) / MAP_SIZE_X,
        (out.pre_pos_fade.z + 0.5 - MAP_SIZE_Y) / -MAP_SIZE_Y,
    );
    return out;
}

/// The four-layer `SampleWater` overload (`standardfuncsgfx.fxh:1200`), with the multipliers
/// `river.shader` passes it. Unlike the two-argument overload in `common.wgsl` this one does *not*
/// apply `WATER_TIME_SCALE`: the river hands it an already-scaled time.
fn sample_river_water(uv: vec2<f32>, time: f32) -> WaterSample {
    let lol1 = 0.6;
    let lol = 0.4;
    let uv_multipliers = array<vec2<f32>, 4>(
        vec2<f32>(0.4) * lol1,
        vec2<f32>(0.4) * lol1,
        vec2<f32>(lol, -lol),
        vec2<f32>(lol, -lol),
    );
    let time_multipliers = array<vec2<f32>, 4>(
        vec2<f32>(0.98) * RIVER_TIME_SCALE,
        vec2<f32>(1.03) * RIVER_TIME_SCALE,
        vec2<f32>(0.95) * RIVER_TIME_SCALE,
        vec2<f32>(1.02) * RIVER_TIME_SCALE,
    );
    let first = sample_blend_lean(
        0.5,
        uv * uv_multipliers[0] + time * time_multipliers[0],
        uv * uv_multipliers[1] + time * time_multipliers[1],
        lean_texture1,
        lean_texture2,
        linear_wrap,
    );
    let second = sample_blend_lean(
        0.5,
        uv * uv_multipliers[2] + time * time_multipliers[2],
        uv * uv_multipliers[3] + time * time_multipliers[3],
        lean_texture1,
        lean_texture2,
        linear_wrap,
    );
    let blended = blend_lean(0.5, first.b, first.m, second.b, second.m);
    var normal = vec3<f32>(blended.b.x, 1.0, blended.b.y);
    normal *= 1.0 / sqrt(normal.x * normal.x + normal.y * normal.y + normal.z * normal.z);
    return WaterSample(blended.b, blended.m, normal);
}

struct RiverSnow {
    color: vec3<f32>,
    normal: vec3<f32>,
    spec_gloss: f32,
}

/// `ApplyRiverSnow` from `river.shader`'s pixel shader block.
fn apply_river_snow(
    color_in: vec3<f32>,
    reflect_color: vec3<f32>,
    pos: vec3<f32>,
    normal_in: vec3<f32>,
    mud_snow_color: vec4<f32>,
    water_side_alpha: f32,
) -> RiverSnow {
    let noise = textureSample(
        city_lights_and_snow_noise,
        linear_wrap,
        pos.xz * SNOW_ICE_NOISE_TILING,
    ).a;
    let is_snow = get_snow(mud_snow_color);

    var opacity = cam_distance(SNOW_CAM_MIN, SNOW_CAM_MAX);
    opacity = SNOW_OPACITY_MIN + opacity * (SNOW_OPACITY_MAX - SNOW_OPACITY_MIN);

    let snow_alpha = smoothstep(0.0, 1.0, saturate(is_snow * opacity * 1.7));
    let snowy = mix(
        saturate(SNOW_WATER_COLOR * reflect_color),
        SNOW_COLOR,
        smoothstep(0.4, 0.75, saturate(noise + water_side_alpha)),
    );
    var normal = normal_in;
    normal.y += mix(20.0, 1.0, water_side_alpha) * snow_alpha;
    return RiverSnow(
        mix(color_in, snowy, snow_alpha),
        normalize(normal),
        snow_alpha,
    );
}

@fragment
fn river_fragment(input: RiverVarying) -> @location(0) vec4<f32> {
    var new_uv = input.uv;

    // The ribbon is symmetric about `v = 0.5`; `vFlip` negates the bank normal's z on the mirrored
    // side, and only the outer quarter of each side maps onto the bank profile.
    var flip = -1.0;
    if input.uv.y < 0.5 {
        new_uv.y = 1.0 - new_uv.y;
        flip = 1.0;
    }
    new_uv.y = saturate(new_uv.y * 4.0 - 3.0);

    var water_color = textureSample(water_color_map, linear_wrap, input.world_uv).rgb;
    let diffuse_color = textureSample(
        river_diffuse,
        river_strip,
        vec2<f32>(new_uv.x, 1.0 - new_uv.y),
    );
    // `RiverData`: blue is the bank blend and red the opacity (section 3.2).
    var water_side_alpha = textureSample(river_data, linear_clamp, new_uv).br;
    if abs(input.uv.y - 0.5) <= 0.25 {
        water_side_alpha = vec2<f32>(0.0, 1.0);
    }

    let water = sample_river_water(input.uv, river.time_direction * RIVER_ANIMATION_SCALE);

    let sun_dir_water = calculate_sun_direction_water(input.pre_pos_fade.xyz);
    let h = normalize(
        normalize(cam_pos() - input.pre_pos_fade.xyz).xzy + -sun_dir_water.xzy,
    );
    let h_wave = h.xy / h.z - water.b;

    let sigma = water.m - vec3<f32>(water.b * water.b, water.b.x * water.b.y);
    let det = sigma.x * sigma.y - sigma.z * sigma.z;
    let e = h_wave.x * h_wave.x * sigma.y + h_wave.y * h_wave.y * sigma.x
        - 2.0 * h_wave.x * h_wave.y * sigma.z;
    var spec = 0.0;
    if det > 0.0 {
        spec = exp(-0.5 * e / det) / sqrt(det);
    }

    // The canal normal: the bank strip's normal map rotated into the flow's own frame.
    let canal_normal_raw = textureSample(
        river_normal_map,
        river_strip,
        vec2<f32>(new_uv.x, 1.0 - new_uv.y),
    ).rbg - vec3<f32>(0.5);
    var canal_normal = normalize(canal_normal_raw);
    canal_normal.z *= flip;
    let tangent = normalize(vec3<f32>(input.tangent.x, 0.0, input.tangent.y));
    let bitangent = normalize(vec3<f32>(input.tangent.y, 0.0, -input.tangent.x));
    let tmp_normal = normalize(cross(tangent, bitangent));
    // `mul( vCanalNormal, Create3x3( vTangent, vTmpNormal, vBitangent ) )`: a row vector times a
    // matrix built from rows, which is the weighted sum of those rows.
    let side_normal = normalize(
        canal_normal.x * tangent + canal_normal.y * tmp_normal + canal_normal.z * bitangent,
    );
    var normal = normalize(mix(water.normal, side_normal, water_side_alpha.x));

    // Gradient borders, applied twice: once to the bank diffuse and once to a copy of the water
    // colour, which is then blended back by the camera-distance factor (section 3.2).
    let gradient_border_factor = 1.0 - gradient_border_camera_distance();
    let cutoff = globals.frame.gb_cam_dist_override_outline_cutoff.zw;
    let override_distance = globals.frame.gb_cam_dist_override_outline_cutoff.xy;
    let bank = gradient_border_apply(
        diffuse_color.rgb,
        input.world_uv,
        gradient_border_channel1,
        gradient_border_channel2,
        linear_clamp,
        1.0,
        cutoff,
        override_distance,
    );
    let bordered_water = gradient_border_apply(
        water_color,
        input.world_uv,
        gradient_border_channel1,
        gradient_border_channel2,
        linear_clamp,
        1.0,
        cutoff,
        override_distance,
    );
    // `vBloomAlpha` is an `inout` both calls write, so the second one's value is the one that
    // reaches `DayNightWithBlend`.
    let bloom_alpha = bordered_water.bloom_alpha;
    water_color = mix(water_color, bordered_water.color, gradient_border_factor);

    let eye_dir = normalize(input.pre_pos_fade.xyz - cam_pos());
    let reflective_color =
        textureSample(reflection_cube_map, linear_clamp, reflect(eye_dir, normal)).rgb
            * RIVER_REFLECTION_GAIN;

    var fresnel = saturate(dot(-eye_dir, normal)) * 0.5;
    fresnel = saturate(RIVER_FRESNEL_BIAS + (1.0 - RIVER_FRESNEL_BIAS) * pow(1.0 - fresnel, 10.0));
    water_color = water_color * (1.0 - fresnel) + reflective_color * fresnel;

    var diffuse = mix(water_color, bank.color, water_side_alpha.x);

    let mud_snow = textureSample(
        snow_mud_texture,
        linear_wrap,
        map_uv_bottom_up(input.pre_pos_fade.xyz),
    );
    let snow = apply_river_snow(
        diffuse,
        reflective_color,
        input.pre_pos_fade.xyz,
        normal,
        mud_snow,
        water_side_alpha.x,
    );
    diffuse = snow.color;
    normal = snow.normal;

    var specular_intensity = mix(0.085, 0.051, snow.spec_gloss);
    specular_intensity = mix(specular_intensity, diffuse_color.a, water_side_alpha.x)
        * (1.0 - gradient_border_factor * 0.5);
    let glossiness = mix(spec / 0.5, 5.0, water_side_alpha.x)
        + snow.spec_gloss * SNOW_SPEC_GLOSS_MULT;

    var properties: LightingProperties;
    properties.world_space_pos = input.pre_pos_fade.xyz;
    properties.to_camera_dir = normalize(cam_pos() - input.pre_pos_fade.xyz);
    properties.diffuse = diffuse;
    properties.normal = normal;
    properties.glossiness = glossiness;
    properties.specular_color = vec3<f32>(specular_intensity);
    // `_NonLinearGlossiness` is `GetNonLinearGlossiness( vGlossiness )` in the game shader, which is
    // `exp2( 11 * g )`; the `river` effect declares no defines, so `CalculateSunLight` takes the
    // plain Phong branch and never reads it. It is left at zero rather than computed, because
    // `spec` is an unbounded Gaussian highlight and `exp2` of it overflows to infinity.
    properties.non_linear_glossiness = 0.0;

    // `fShadowTerm` is 1 (see the header). The light direction is `CalculateSunDirection`, not the
    // `CalculateSunDirectionWater` the wave highlight above uses: the game shader calls the
    // two-argument `CalculateSunLight` overload, which takes the sun direction from
    // `vVirtualSunPos.xyz` rather than the `*_HEIGHT_WATER` swizzle.
    let light = calculate_sun_light_phong(
        properties,
        1.0,
        calculate_sun_direction(input.pre_pos_fade.xyz),
    );
    var out = compose_light(properties, light.diffuse, light.specular);

    out = apply_fow(out, 0.0, 0.0);
    out = apply_distance_fog(out, input.pre_pos_fade.xyz);
    out = day_night_with_blend(
        out,
        calc_globe_normal(input.pre_pos_fade.xz),
        mix(BORDER_NIGHT_DESATURATION_MAX, 1.0, bloom_alpha),
    );

    let fade_value = 1.0 - input.pre_pos_fade.w;
    var fast_fade = fade_value * fade_value * fade_value * fade_value;
    fast_fade = fast_fade * fast_fade;
    // "fade slower if water, faster if land (help river crossings)".
    let desired_fade = mix(fade_value * 2.0, 0.0, saturate(water_side_alpha.x * 4.0));
    let alpha_multiplier = saturate(mix(desired_fade, 1.0, fast_fade));

    return vec4<f32>(out, water_side_alpha.y * alpha_multiplier * input.transparency);
}
