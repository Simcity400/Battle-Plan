// `gfx/FX/maparrow.shader`, the `MapArrowDefault` and `MapArrowNoHead`
// effects. One pipeline carries both: `has_head` picks which
// of the two shipped pixel shaders' bodies runs, and `uv_head_variant.z` picks
// the head quad inside the first of them.
//
// Kept as the shader has it: the packed variant page and `vTextureVariantScale`,
// `vBodyUvScale`, `fCullingOffset`, the `ANIM_TEXTURE` texture scroll, the mask
// channel weighting, the HSV saturation and value boost (1.5/1.0 for the head
// shader, 2.0/1.5 for the no-head one), the 50 % lighting blend, the distance
// fog and the day/night blend at 0.2, and both fade laws --- uv fractions for
// the head shader, absolute map units for the no-head one.
//
// Dropped, with the reason:
//
// * Generated mip levels: not verified on pair H.
//   Authored texture mip levels are retained.
// * The `#ifndef NO_SHADER_TEXTURE_LOD` half of `CalculateTerrainNormal`, the
//   terrain-atlas and water normals. The shipped shader has an explicit
//   `#ifdef NO_SHADER_TEXTURE_LOD return normal;` early out that returns the
//   heightmap normal alone, and that branch is what this port takes. The
//   consequence is `vWaterValue = 0`, so `MAP_ARROW_NORMALS_STR_WATER` never
//   applies and an arrow over water does not get the water's uv push.
//
// Two inputs are fed zero rather than dropped, because a replay has neither:
// `vTime_IsSelected_FadeInOut.y`, the selection blink, and
// `vOfsX_vOfsTex_vOfsFx.z`, the `FxMask` execution stripe. Both terms are
// written out below and reduce to the identity at zero.

/// `gfx/FX/constants.fxh:225`.
const MAP_ARROW_NORMALS_STR_TERR: f32 = 0.0125;
/// `:226`. Reached only through `vWaterValue`, which this port leaves at zero.
const MAP_ARROW_NORMALS_STR_WATER: f32 = 0.08;
/// `:223`, `:224`.
const MAP_ARROW_SEL_BLINK_SPEED: f32 = 5.5;
const MAP_ARROW_SEL_BLINK_RANGE: f32 = 0.7;

@group(1) @binding(0) var arrow_mask_texture: texture_2d<f32>;
@group(1) @binding(1) var arrow_pattern_texture: texture_2d<f32>;
@group(1) @binding(2) var height_normal_texture: texture_2d<f32>;
@group(1) @binding(3) var linear_wrap: sampler;
@group(1) @binding(4) var linear_clamp: sampler;
@group(1) @binding(5) var point_wrap: sampler;

/// `ConstantBuffer(3, 32)` reduced to what the port reads, plus the viewer's own
/// world-wrap offset in the slot the engine already keeps it in:
/// `vOfsX_vOfsTex_vOfsFx.x` is the engine's own map-wrap x offset.
struct ArrowSlot {
    arrow_mask: vec4<f32>,
    arrow_color: vec4<f32>,
    arrow_secondary_color: vec4<f32>,
    time_selected_fade: vec4<f32>,
    ofs_x_tex_fx: vec4<f32>,
    body_uv_scale: f32,
    secondary_body_uv_scale: f32,
    texture_variant_scale: f32,
    culling_offset: f32,
    normal_map_factor: f32,
    total_length: f32,
    uv_to_absolute_length: f32,
    /// 1 for an arrow type whose technique carries an arrowhead.
    has_head: f32,
}

@group(2) @binding(0) var<uniform> arrow: ArrowSlot;

struct ArrowOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv_head_variant: vec4<f32>,
    @location(1) uv_terrain: vec2<f32>,
    @location(2) prepos: vec3<f32>,
}

@vertex
fn arrow_vertex(
    @location(0) position: vec3<f32>,
    @location(1) uv_head_variant: vec4<f32>,
) -> ArrowOut {
    var out: ArrowOut;
    var pos = position;
    pos.x += arrow.ofs_x_tex_fx.x;
    out.prepos = pos;
    out.clip = globals.frame.view_projection * vec4<f32>(pos, 1.0);
    var uv = uv_head_variant;
    // `#ifdef ANIM_TEXTURE`, which every `MapArrowNoHead` effect declares and
    // `MapArrowDefault` does not. A zero `texture_anim_speed` makes it the
    // identity, which is what the arrows that do not scroll carry.
    uv.x -= arrow.time_selected_fade.x * arrow.ofs_x_tex_fx.y;
    if arrow.culling_offset > 0.0 {
        uv.y = arrow.culling_offset - uv.y;
    }
    out.uv_head_variant = uv;
    // `MAP_POW2_*` are 1.0 on this profile (R1 section 3).
    out.uv_terrain = vec2<f32>(
        (pos.x + 0.5) / MAP_SIZE_X,
        (pos.z + 0.5 - MAP_SIZE_Y) / -MAP_SIZE_Y,
    );
    return out;
}

/// `HuePost` (`standardfuncsgfx.fxh:91`), a hue in sixths rather than degrees.
/// `map/post.wgsl` carries its own copy; the two are the same function and each
/// pass compiles alone.
fn arrow_hue_post(h: f32) -> vec3<f32> {
    let x = 1.0 - abs(h - 2.0 * floor(h * 0.5) - 1.0);
    if h < 1.0 { return vec3<f32>(1.0, x, 0.0); }
    if h < 2.0 { return vec3<f32>(x, 1.0, 0.0); }
    if h < 3.0 { return vec3<f32>(0.0, 1.0, x); }
    if h < 4.0 { return vec3<f32>(0.0, x, 1.0); }
    if h < 5.0 { return vec3<f32>(x, 0.0, 1.0); }
    return vec3<f32>(1.0, 0.0, x);
}

fn arrow_hsv_to_rgb(hsv: vec3<f32>) -> vec3<f32> {
    if hsv.y != 0.0 {
        let c = hsv.y * hsv.z;
        return clamp(arrow_hue_post(hsv.x) * c + (hsv.z - c), vec3<f32>(0.0), vec3<f32>(1.0));
    }
    return clamp(vec3<f32>(hsv.z), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn arrow_rgb_to_hsv(rgb: vec3<f32>) -> vec3<f32> {
    let c_max = max(rgb.r, max(rgb.g, rgb.b));
    let c_min = min(rgb.r, min(rgb.g, rgb.b));
    let diff = c_max - c_min;
    var h = 0.0;
    var s = 0.0;
    if diff != 0.0 {
        s = diff / c_max;
        if c_max == rgb.r {
            h = (rgb.g - rgb.b) / diff + 6.0;
        } else if c_max == rgb.g {
            h = (rgb.b - rgb.r) / diff + 2.0;
        } else {
            h = (rgb.r - rgb.g) / diff + 4.0;
        }
    }
    return vec3<f32>(h, s, c_max);
}

/// `CalculateTerrainNormal` under `NO_SHADER_TEXTURE_LOD`: the heightmap normal
/// alone, with the shipped `.rbg` swizzle and the 0.5 bias.
fn arrow_terrain_normal(uv_terrain: vec2<f32>) -> vec3<f32> {
    let sampled = textureSample(height_normal_texture, linear_clamp, wrap_u(uv_terrain));
    return normalize(sampled.rbg - 0.5);
}

/// Plain Phong: maparrow declares no PDX_IMPROVED_BLINN_PHONG.
/// It shares the terrain shadow weight and accumulates tiled point lights.
fn arrow_lighting(prepos: vec3<f32>, normal: vec3<f32>, color: vec4<f32>, shadow_term: f32) -> vec3<f32> {
    var properties: LightingProperties;
    properties.world_space_pos = prepos;
    properties.to_camera_dir = normalize(cam_pos() - prepos);
    properties.normal = normal;
    properties.diffuse = color.rgb;
    properties.glossiness = 0.05;
    properties.specular_color = vec3<f32>(color.a);
    properties.non_linear_glossiness = get_non_linear_glossiness(color.a);
    var light = calculate_sun_light_phong(properties, shadow_term, calculate_sun_direction(prepos));
    light = calculate_point_lights(properties, light, false, globals.light_grid);
    return compose_light(properties, light.diffuse, light.specular);
}

@fragment
fn arrow_fragment(input: ArrowOut) -> @location(0) vec4<f32> {
    let heads = arrow.has_head >= 0.5;
    let is_head = select(0.0, 1.0, heads && input.uv_head_variant.z >= 0.5);
    // `1.0f - step( Input.uv_isHead_variant.w, 0.0f )`: a variant word at or
    // below zero selects the first page.
    let variant = 1.0 - select(0.0, 1.0, 0.0 >= input.uv_head_variant.w);
    let variant_body_uv_scale = mix(
        arrow.body_uv_scale,
        arrow.secondary_body_uv_scale,
        variant,
    );

    let body_v = ((input.uv_head_variant.y - 0.5) * variant_body_uv_scale + 0.5 + variant)
        * arrow.texture_variant_scale;
    var uv: vec2<f32>;
    if heads {
        // `ArrowPixelShader`: the body is the mask's left half and the head its
        // right one, which is why both pages are square and twice as wide.
        uv = vec2<f32>(
            input.uv_head_variant.x * 0.5 + is_head * 0.5,
            mix(body_v, input.uv_head_variant.y, is_head),
        );
    } else {
        // `ArrowPixelShaderNoHead`: the looping u the vertex laid down.
        uv = vec2<f32>(input.uv_head_variant.x, body_v);
    }

    let normal = arrow.normal_map_factor * arrow_terrain_normal(input.uv_terrain);
    let water_value = 0.0;
    uv += normal.xz * MAP_ARROW_NORMALS_STR_TERR * select(normal.y, 1.0, heads);
    uv -= normal.xz * (water_value * MAP_ARROW_NORMALS_STR_WATER);
    if heads {
        uv = clamp(uv, vec2<f32>(0.001), vec2<f32>(0.998));
    }

    var mask = textureSample(arrow_mask_texture, point_wrap, uv);
    // The selection blink, at `vTime_IsSelected_FadeInOut.y = 0` in a replay.
    mask -= vec4<f32>(
        (sin(arrow.time_selected_fade.x * MAP_ARROW_SEL_BLINK_SPEED) * MAP_ARROW_SEL_BLINK_RANGE
            + 1.0
            - MAP_ARROW_SEL_BLINK_RANGE * 0.5)
            * 0.5
            * arrow.time_selected_fade.y,
    );
    mask = saturate(mask);
    if heads && mask.a <= 0.0 {
        // `clip( vMask.a <= 0 ? -1 : 1 )`, which only `ArrowPixelShader` has.
        discard;
    }
    let weighted = mask.rgb * arrow.arrow_mask.rgb * mask.a;
    var mask_value = saturate(weighted.r + weighted.g + weighted.b);

    if heads {
        // The fades are fractions of the body's own uv, and the head is never
        // faded --- the `+ vIsHead` inside each `saturate`.
        if arrow.time_selected_fade.z > 0.0 {
            mask_value *= saturate(
                levels1(input.uv_head_variant.x, 0.0, arrow.time_selected_fade.z) + is_head,
            );
        }
        if arrow.time_selected_fade.w > 0.0 {
            mask_value *= saturate(
                levels1(1.0 - input.uv_head_variant.x, 0.0, arrow.time_selected_fade.w) + is_head,
            );
        }
    } else {
        // The fades are in map units along the curve, recovered by undoing the
        // texture scroll and scaling back up.
        let epsilon = 0.0001;
        let along_uv = input.uv_head_variant.x
            + arrow.time_selected_fade.x * arrow.ofs_x_tex_fx.y;
        let absolute = along_uv * arrow.uv_to_absolute_length;
        mask_value *= saturate((absolute + epsilon) / max(arrow.time_selected_fade.z, epsilon));
        mask_value *= saturate(
            (arrow.total_length - absolute + epsilon) / max(arrow.time_selected_fade.w, epsilon),
        );
    }
    if mask_value <= 0.0 {
        discard;
    }

    let pattern = textureSampleBias(arrow_pattern_texture, linear_wrap, uv, -0.5);
    var tint = mix(arrow.arrow_color, arrow.arrow_secondary_color, variant);
    // The shipped `#if 1` branch of both pixel shaders.
    var hsv = arrow_rgb_to_hsv(tint.rgb);
    hsv.x = fmod_loop(hsv.x, 6.0);
    hsv.y *= select(2.0, 1.5, heads);
    hsv.z *= select(1.5, 1.0, heads);
    tint = vec4<f32>(arrow_hsv_to_rgb(hsv), tint.a);

    var color = saturate(pattern * tint);
    let lit = arrow_lighting(input.prepos, normal, color, get_shadow_scaled(0.7, input.clip));
    var rgb = mix(color.rgb, lit, 0.5);
    rgb = apply_distance_fog(rgb, input.prepos);
    rgb = day_night_with_blend(rgb, calc_globe_normal(input.prepos.xz), 0.2);
    return vec4<f32>(rgb, color.a * mask_value);
}
