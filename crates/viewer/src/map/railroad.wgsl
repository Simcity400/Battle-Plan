// `gfx/models/supply/railroad.shader`, the `railroad` effect, ported literally.
//
// The effect declares `PDX_SNOW` and `PDX_GRADIENT_BORDERS`, so the strip takes
// snow and carries the country borders and occupation stripes the same way the
// terrain does. It is not a lit surface: there is no normal, no light and no
// `ComposeLight*` anywhere in the shader. The colour is the texture, snowed,
// bordered, then faded by `vRailwayAlpha`. Distance fog and day-night grading
// appear in none of the effect's code either, so the strip skips them the same
// way the terrain does not: rails stay full-bright at distance and at night,
// as the game draws them.
//
 // Dropped, with the reason:
//
// * `LeanTexture1/2`, `RiverData`, `HeightNormal`, `CityLightsAndSnowNoise`,
//   `LightIndexMap` and `LightDataMap` are declared as samplers by the effect
//   and are read by none of its code.

/// The five 64 x 64 bands of `railroad.dds`, one per rail level.
const RAIL_LEVELS: f32 = 5.0;

@group(1) @binding(0) var rail_texture: texture_2d<f32>;
@group(1) @binding(1) var snow_mud_texture: texture_2d<f32>;
@group(1) @binding(2) var gradient_border_channel1: texture_2d<f32>;
@group(1) @binding(3) var gradient_border_channel2: texture_2d<f32>;
@group(1) @binding(4) var province_secondary_color_map: texture_2d<f32>;
@group(1) @binding(5) var linear_wrap: sampler;
@group(1) @binding(6) var linear_clamp: sampler;

/// `ConstantBuffer(1, 32)` reduced to the one value the ported code reads, plus
/// the viewer's own world-wrap offset. `ShadowMapTextureMatrix` is still
/// unused: the ported screen-space fog read below takes `shadow_map` and the
/// fragment position directly. `vTimeGlobal` is read by nothing.
struct RailSlot {
    world_offset_x: f32,
    railway_alpha: f32,
    padding: vec2<f32>,
}

@group(1) @binding(7) var<uniform> rail: RailSlot;

struct RailOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) world: vec3<f32>,
}

@vertex
fn rail_vertex(
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
) -> RailOut {
    var out: RailOut;
    let world = position + vec3<f32>(rail.world_offset_x, 0.0, 0.0);
    out.world = world;
    out.clip = globals.frame.view_projection * vec4<f32>(world, 1.0);
    out.uv = uv;
    return out;
}

/// `ApplySnowMesh`, which `railroad.shader` carries its own copy of. The strip's
/// normal is the literal `float3(0, 1, 0)` there, so the fade reduces to the
/// snow amount alone and both `saturate` guards on `vNormal.y` pass.
struct RailSnow {
    color: vec3<f32>,
    alpha: f32,
}

fn apply_snow_rail(color: vec3<f32>, mud_snow: vec4<f32>) -> RailSnow {
    let normal_y = 1.0;
    let is_snow = get_snow(mud_snow) * 0.25;
    let fade = saturate(
        saturate(normal_y - saturate(1.0 - is_snow))
            * is_snow
            * 5.5
            * saturate((normal_y - 0.5) * 1000.0),
    );
    var opacity = cam_distance(SNOW_CAM_MIN, SNOW_CAM_MAX);
    opacity = SNOW_OPACITY_MIN + opacity * (SNOW_OPACITY_MAX - SNOW_OPACITY_MIN);
    var out: RailSnow;
    out.color = mix(color, SNOW_COLOR, fade * opacity);
    out.alpha = saturate(is_snow * 1.5) * opacity;
    return out;
}

@fragment
fn rail_fragment(input: RailOut) -> @location(0) vec4<f32> {
    // `if ( Input.vUV.y < 0.0 || Input.vUV.y >= 1.0 ) discard;` -- the strip's
    // own edge, which is why the geometry can be wider than the band it draws.
    if input.uv.y < 0.0 || input.uv.y >= 1.0 {
        discard;
    }
    var sampled = textureSample(rail_texture, linear_wrap, input.uv);
    var color = sampled.rgb;
    let pos = input.world;

    // `PDX_SNOW`.
    let mud_snow = textureSample(snow_mud_texture, linear_wrap, map_uv_bottom_up(pos));
    color = apply_snow_rail(color, mud_snow).color;

    // `PDX_GRADIENT_BORDERS`, with the same map uv the terrain uses. `wrap_u`
    // sends a wrap copy back onto the map it was copied from, as the tree and
    // river ports do.
    let map_uv = wrap_u(vec2<f32>(
        (pos.x + 0.5) / MAP_SIZE_X,
        (pos.z + 0.5 - MAP_SIZE_Y) / -MAP_SIZE_Y,
    ));
    let border = gradient_border_apply(
        color,
        map_uv,
        gradient_border_channel1,
        gradient_border_channel2,
        linear_clamp,
        1.0,
        globals.frame.gb_cam_dist_override_outline_cutoff.zw,
        globals.frame.gb_cam_dist_override_outline_cutoff.xy,
    );
    let secondary = secondary_color_mask(
        border.color,
        map_uv,
        province_secondary_color_map,
        linear_wrap,
        border.bloom_alpha,
    );
    color = secondary.color;

    // `Color.a *= vRailwayAlpha;`
    // `float3 FOWColor = ApplyFOW( Color.rgb, ShadowMap, Input.vScreenCoord );`
    // the screen-space fog buffer the unlit pre-pass filled, as the borders read it.
    color = apply_fow_buffer(color, input.clip);
    return vec4<f32>(color, sampled.a * rail.railway_alpha);
}
