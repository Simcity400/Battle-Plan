// Helpers shared by the terrain and water passes, ported from the game's
// `gfx/FX/constants.fxh`, `standardfuncsgfx.fxh` and `fow.fxh`.
//
// The port follows the `terrain` and `water` effect defines: GRADIENT_BORDERS
// and PDX_IMPROVED_BLINN_PHONG are on, LOW_END_GFX, NO_SHADER_TEXTURE_LOD and
// NO_REFRACTIONS are off, and every `*_POW2_*` is 1.0 (R1 section 3.5), so the
// POW2 multiplies are dropped. WGSL has no `inout`, so helpers that the HLSL
// writes through an argument return a struct instead.
//
// This file is concatenated ahead of `terrain.wgsl` and `water.wgsl`.

// ---------------------------------------------------------------- constants.fxh
const MAP_SIZE_X: f32 = 5632.0;
const MAP_SIZE_Y: f32 = 2048.0;

const SPECULAR_MULTIPLIER: f32 = 1.0;
const MAP_SPECULAR_WIDTH: f32 = 15.0;

const CITY_LIGHTS_TILING: f32 = 0.09103;
const CITY_LIGHTS_INTENSITY: f32 = 5.5;
const CITY_LIGHTS_BLOOM_FACTOR: f32 = 0.3;

const TERRAIN_TILE_FREQ: f32 = 128.0;
const MAP_NUM_TILES: f32 = 4.0;
const TEXELS_PER_TILE: f32 = 512.0;
const ATLAS_TEXEL_POW2_EXPONENT: f32 = 11.0;
const TERRAIN_WATER_CLIP_HEIGHT: f32 = 3.0;

const MUD_TILING: f32 = 0.09;
const MUD_NORMAL_CUTOFF: f32 = 10.982;
const MUD_STRENGHTEN: f32 = 1.0;

const SNOW_OPACITY_MIN: f32 = 0.95;
const SNOW_OPACITY_MAX: f32 = 0.2;
const SNOW_CAM_MIN: f32 = 50.0;
const SNOW_CAM_MAX: f32 = 300.0;
const MUD_CAM_MIN: f32 = 50.0;
const MUD_CAM_MAX: f32 = 300.0;
const ICE_CAM_MIN: f32 = 100.0;
const ICE_CAM_MAX: f32 = 350.0;

const SNOW_START_HEIGHT: f32 = 3.0;
const SNOW_RIDGE_START_HEIGHT: f32 = 11.0;
const SNOW_NORMAL_START: f32 = 0.7;
const SNOW_COLOR: vec3<f32> = vec3<f32>(0.46, 0.48, 0.69);
const SNOW_CLIFFS: f32 = 5.0;
const SNOW_SPEC_GLOSS_MULT: f32 = 0.2;
const SNOW_TILING: f32 = 0.05;
const SNOW_NOISE_TILING: f32 = 0.06;
const SNOW_FROST_MIN_EFFECT: f32 = 0.4;

const ICE_COLOR: vec3<f32> = vec3<f32>(0.5, 0.6, 0.9);
const ICE_NOISE_TILING: f32 = 0.1;

const WATER_COLOR_LIGHTNESS: f32 = 0.5;
const COLORMAP_OVERLAY_STRENGTH: f32 = 0.75;
const COLORMAP_MUD_OVERLAY_STRENGTH: f32 = 0.5;
const FAKE_CUBEMAP_COLOR: vec3<f32> = vec3<f32>(0.0, 0.0, 0.0);

const NIGHT_AMBIENT_BOOST: f32 = 3.0;
const DAY_AMBIENT: array<vec3<f32>, 6> = array<vec3<f32>, 6>(
    vec3<f32>(0.1, 0.1, 0.05),
    vec3<f32>(0.15, 0.15, 0.15),
    vec3<f32>(0.03, 0.03, 0.06),
    vec3<f32>(0.0, 0.0, 0.0),
    vec3<f32>(0.0502, 0.05023, 0.1023),
    vec3<f32>(0.03, 0.033, 0.033),
);
const NIGHT_AMBIENT: array<vec3<f32>, 6> = array<vec3<f32>, 6>(
    vec3<f32>(0.2, 0.2, 0.2),
    vec3<f32>(0.0, 0.0, 0.0),
    vec3<f32>(0.01, 0.01, 0.01),
    vec3<f32>(0.0, 0.0, 0.1),
    vec3<f32>(0.06, 0.1, 0.15),
    vec3<f32>(0.14, 0.14, 0.2),
);

const FOG_COLOR: vec3<f32> = vec3<f32>(0.12, 0.28, 0.6);
const FOG_BEGIN: f32 = 1.0;
const FOG_END: f32 = 150.0;
const FOG_MAX: f32 = 0.35;

const WATER_TIME_SCALE: f32 = 1.0 / 50.0;
const WATER_HEIGHT: f32 = 9.5;

const GB_CAM_MIN: f32 = 100.0;
const GB_CAM_MAX: f32 = 350.0;
const GB_CAM_MAX_FILLING_CLAMP: f32 = 0.8;
const GB_THRESHOLD: f32 = 0.05;
const GB_THRESHOLD2: f32 = 0.25;
const GB_OUTLINE_CUTOFF_SEA: f32 = 0.990;
const GB_OPACITY_NEAR: f32 = 1.0;
const GB_OPACITY_FAR: f32 = 0.85;
const BORDER_NIGHT_DESATURATION_MAX: f32 = 0.2;
const BORDER_FOW_REMOVAL_FACTOR: f32 = 0.8;
const BORDER_LIGHT_REMOVAL_FACTOR: f32 = 0.8;
const GB_STRENGTH_CH1: f32 = 1.0;
const GB_STRENGTH_CH2: f32 = 1.0;
const GB_FIRST_LAYER_PRIORITY: f32 = 0.4;

const SEC_MAP_TILE: f32 = 6000.0;

// -------------------------------------------------- standardfuncsgfx.fxh day/night
const GMT_OFFSET: f32 = 2793.0;
const FEATHER_MIN: f32 = -0.01;
const FEATHER_MAX: f32 = 0.01;
const MOON_FEATHER_MIN: f32 = -0.01;
const MOON_FEATHER_MAX: f32 = 0.01;
const NIGHT_OPACITY: f32 = 0.85;
const NIGHT_DARKNESS: f32 = 0.5;
const SOUTH_POLE_OFFSET: f32 = 0.17;
const NORTH_POLE_OFFSET: f32 = 0.93;

// ----------------------------------------------------- ConstantBuffer(0, 0)
struct Frame {
    view_projection: mat4x4<f32>,
    inv_view_projection: mat4x4<f32>,
    gb_cam_dist_override_outline_cutoff: vec4<f32>,
    virtual_sun_pos: vec4<f32>,
    virtual_moon_pos: vec4<f32>,
    second_virtual_sun_pos: vec4<f32>,
    second_virtual_moon_pos: vec4<f32>,
    cam_pos_hdr_range: vec4<f32>,
    cam_look_at_dir_global_time: vec4<f32>,
    fow_opacity_time_snow_mud_day_night: vec4<f32>,
    day_night_hour_sun_dir: vec4<f32>,
    ambient_pos_x_shadow_fade: vec4<f32>,
    ambient_neg_x_fow_fade: vec4<f32>,
    ambient_pos_y_min_mesh_alpha: vec4<f32>,
    ambient_neg_y_neg_fog: vec4<f32>,
    ambient_pos_z: vec4<f32>,
    ambient_neg_z_cubemap: vec4<f32>,
    sun_diffuse_intensity: vec4<f32>,
    moon_diffuse_intensity: vec4<f32>,
    gb_texture_height_sun_specular: vec4<f32>,
}

/// The frame constants plus the two viewer-only blocks: the terrain grid the
/// vertex shader walks, and `pdxwater.shader`'s `ConstantBuffer(3, 48)`.
struct Globals {
    frame: Frame,
    /// `(origin x, origin z, step, unused)` of the visible-rectangle grid.
    grid: vec4<f32>,
    /// `(columns, rows, unused, unused)` of grid vertices.
    counts: vec4<u32>,
    /// `vTime_HalfPixelOffset.xyz`.
    time_half_pixel: vec4<f32>,
}

@group(0) @binding(0) var<uniform> globals: Globals;

fn cam_pos() -> vec3<f32> { return globals.frame.cam_pos_hdr_range.xyz; }
fn hdr_range() -> f32 { return globals.frame.cam_pos_hdr_range.w; }
fn global_time() -> f32 { return globals.frame.cam_look_at_dir_global_time.w; }
fn day_night_strength() -> f32 { return globals.frame.fow_opacity_time_snow_mud_day_night.w; }
fn snow_mud_fade() -> f32 { return globals.frame.fow_opacity_time_snow_mud_day_night.z; }
fn gb_texture_height() -> f32 { return globals.frame.gb_texture_height_sun_specular.x; }
fn sun_specular_intensity() -> f32 { return globals.frame.gb_texture_height_sun_specular.y; }

// ------------------------------------------------------------------ utilities
fn to_gamma(linear: vec3<f32>) -> vec3<f32> { return pow(linear, vec3<f32>(0.45)); }
fn to_linear(gamma: vec3<f32>) -> vec3<f32> { return pow(gamma, vec3<f32>(2.2)); }

/// `fmod_loop` from `defines_hlsl_dx11.fxh`: a modulo that stays positive.
fn fmod_loop(x: f32, y: f32) -> f32 { return x - y * floor(x / y); }

fn levels1(value: f32, low: f32, high: f32) -> f32 {
    return saturate((value - low) / (high - low));
}

fn rotate_vector_by_vector(v1: vec3<f32>, v2: vec3<f32>) -> vec3<f32> {
    let zaxis = v1;
    let xaxis = normalize(cross(zaxis, vec3<f32>(0.0, 0.0, 1.0)));
    let yaxis = normalize(cross(xaxis, zaxis));
    return xaxis * v2.x + zaxis * v2.y + yaxis * v2.z;
}

fn get_overlay(color: vec3<f32>, overlay: vec3<f32>, percent: f32) -> vec3<f32> {
    let color_gamma = to_gamma(color);
    let overlay_gamma = to_gamma(overlay);
    let screened = vec3<f32>(1.0) - 2.0 * (vec3<f32>(1.0) - overlay_gamma) * (vec3<f32>(1.0) - color_gamma);
    let multiplied = 2.0 * overlay_gamma * color_gamma;
    let blended = select(screened, multiplied, overlay_gamma < vec3<f32>(0.5));
    return mix(color, to_linear(blended), percent);
}

fn cam_distance(low: f32, high: f32) -> f32 {
    return (clamp(cam_pos().y, low, high) - low) / (high - low);
}

/// Sampling the whole-map data textures that index on world `xz` directly, so
/// their first row is world `z = 0` (R1 section 6.1).
fn map_uv_bottom_up(world: vec3<f32>) -> vec2<f32> {
    return vec2<f32>((world.x + 0.5) / MAP_SIZE_X, (world.z + 0.5) / MAP_SIZE_Y);
}

/// The clamped samplers (`TerrainIDMap`, the two gradient channels) would smear
/// their edge texel where the mesh runs past the seam. The engine avoids that
/// by drawing three world-wrap copies with the camera translated instead
/// (R2 section 2.4); wrapping the coordinate here draws one mesh with the same
/// result, apart from a single-texel seam exactly on the wrap.
fn wrap_u(uv: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(fmod_loop(uv.x, 1.0), uv.y);
}

// ------------------------------------------------------------------ day/night
fn calc_globe_normal(world_xz: vec2<f32>) -> vec3<f32> {
    let x = fmod_loop(
        (world_xz.x - GMT_OFFSET) / MAP_SIZE_X + globals.frame.day_night_hour_sun_dir.x,
        1.0,
    );
    var y = world_xz.y / MAP_SIZE_Y;
    y = SOUTH_POLE_OFFSET + (NORTH_POLE_OFFSET - SOUTH_POLE_OFFSET) * y;
    y = -cos(y * 3.1415);
    let xz_len = 1.0 - abs(y);
    return normalize(vec3<f32>(sin(x * 6.2831) * xz_len, y, cos(x * 6.2831) * xz_len));
}

fn day_night_factor_range(globe_normal: vec3<f32>, low: f32, high: f32) -> f32 {
    let value = dot(globe_normal, globals.frame.day_night_hour_sun_dir.yzw);
    return saturate((value - low) / (high - low)) * day_night_strength();
}

fn day_night_factor(globe_normal: vec3<f32>) -> f32 {
    return day_night_factor_range(globe_normal, FEATHER_MIN, FEATHER_MAX);
}

fn nightify_color(day_color: vec3<f32>, blend: f32) -> vec3<f32> {
    let desaturation = mix(0.0, 0.8, blend * blend * blend);
    let grey = dot(day_color, vec3<f32>(0.4, 0.3, 0.05));
    let night = saturate(mix(vec3<f32>(grey), grey * vec3<f32>(0.2, 0.7, 1.2), vec3<f32>(0.25)));
    return mix(day_color, night, vec3<f32>(desaturation)) * NIGHT_DARKNESS;
}

fn day_night_with_blend(day_color: vec3<f32>, globe_normal: vec3<f32>, blend: f32) -> vec3<f32> {
    return mix(
        day_color,
        nightify_color(day_color, blend),
        day_night_factor(globe_normal) * NIGHT_OPACITY,
    );
}

// --------------------------------------------------------------------- fog
fn calculate_distance_fog_factor(pos: vec3<f32>) -> f32 {
    let difference = cam_pos() - pos;
    let fog_factor = 1.0 - abs(normalize(difference).y);
    let square_distance = dot(difference, difference);
    let begin = FOG_BEGIN * FOG_BEGIN;
    let end = FOG_END * FOG_END;
    return saturate(min((square_distance - begin) / (end - begin), FOG_MAX)) * fog_factor;
}

fn apply_distance_fog(color: vec3<f32>, pos: vec3<f32>) -> vec3<f32> {
    return mix(color, FOG_COLOR, calculate_distance_fog_factor(pos));
}

// ----------------------------------------------------------------- fow.fxh
/// `ApplyFOW` with the factors the `terrainunlit` pre-pass would have written.
///
/// The viewer is omniscient (`PLAN.md` section 2), so it does not run that
/// pre-pass and both factors are zero, which makes this the identity. The port
/// is kept so the FOW pass only has to supply the factors.
fn apply_fow(color: vec3<f32>, color_factor: f32, alpha_factor: f32) -> vec3<f32> {
    let fog_color = mix(vec3<f32>(0.08, 0.08, 0.1), vec3<f32>(0.98, 0.93, 0.93), color_factor);
    return mix(color, fog_color, alpha_factor);
}

// --------------------------------------------------------------- snow and mud
fn get_snow(mud_snow: vec4<f32>) -> f32 {
    return mix(mud_snow.b, mud_snow.g, snow_mud_fade());
}

struct SnowResult {
    color: vec3<f32>,
    normal: vec3<f32>,
    glossiness: f32,
    snow_alpha: f32,
}

fn apply_snow(
    color_in: vec3<f32>,
    pos: vec3<f32>,
    normal_in: vec3<f32>,
    mud_snow: vec4<f32>,
    snow_texture: texture_2d<f32>,
    snow_noise: texture_2d<f32>,
    samp: sampler,
    glossiness_in: f32,
    snow_alpha_in: f32,
) -> SnowResult {
    let snow_fade = saturate(pos.y - SNOW_START_HEIGHT);
    let normal_fade = saturate(saturate(normal_in.y - SNOW_NORMAL_START) * SNOW_CLIFFS);
    let snow_sample = textureSample(snow_texture, samp, pos.xz * SNOW_TILING);
    let noise = textureSample(snow_noise, samp, pos.xz * SNOW_NOISE_TILING).a;
    let is_snow = get_snow(mud_snow);

    var transparency = noise;
    transparency += saturate(pos.y - SNOW_RIDGE_START_HEIGHT)
        * (saturate((normal_in.y - 0.9) * 1000.0) * is_snow);
    transparency = saturate(transparency);

    let snow = saturate(saturate(transparency - (1.0 - is_snow)) * 5.0);
    let frost = saturate(saturate(transparency + 0.5) - (1.0 - is_snow));

    var opacity = cam_distance(SNOW_CAM_MIN, SNOW_CAM_MAX);
    opacity = SNOW_OPACITY_MIN + opacity * (SNOW_OPACITY_MAX - SNOW_OPACITY_MIN);

    let snow_alpha = saturate(
        saturate(snow + frost) * snow_fade * normal_fade * saturate(is_snow * 2.25) * opacity,
    );
    let min_snow = smoothstep(0.0, 1.0, is_snow);
    let color = mix(
        color_in,
        snow_sample.a * SNOW_COLOR,
        snow_alpha_in * saturate(snow_alpha + SNOW_FROST_MIN_EFFECT * min_snow),
    );

    var snow_normal = normalize(snow_sample.rbg - vec3<f32>(0.5));
    snow_normal = normalize(rotate_vector_by_vector(snow_normal, normal_in));
    let normal = normalize(mix(normal_in, snow_normal, snow_alpha));

    return SnowResult(
        color,
        normal,
        glossiness_in + snow_sample.a * snow_alpha * SNOW_SPEC_GLOSS_MULT,
        snow_alpha,
    );
}

struct MudResult {
    color: vec3<f32>,
    normal: vec3<f32>,
    glossiness: f32,
    spec: f32,
}

fn get_mud_color(
    result_in: vec3<f32>,
    mud_snow: vec4<f32>,
    pos: vec3<f32>,
    normal_in: vec3<f32>,
    glossiness_in: f32,
    spec_in: f32,
    mud_diffuse_gloss: texture_2d<f32>,
    mud_normal_spec: texture_2d<f32>,
    terrain_color: vec3<f32>,
    snow_noise: texture_2d<f32>,
    samp: sampler,
) -> MudResult {
    let opacity = cam_distance(MUD_CAM_MIN, MUD_CAM_MAX);
    let noise = mix(1.0, (0.5 + textureSample(snow_noise, samp, pos.xz * 0.01).a) * 0.5, opacity);

    var mud = mix(mud_snow.r, mud_snow.a, snow_mud_fade());
    mud *= 1.0
        - saturate(saturate(normal_in.y - MUD_NORMAL_CUTOFF) * ((1.0 - MUD_NORMAL_CUTOFF) * 1000.0));
    mud = saturate(mud * MUD_STRENGHTEN * noise);

    let diffuse_gloss = textureSample(mud_diffuse_gloss, samp, pos.xz * MUD_TILING);
    let normal_spec = textureSample(mud_normal_spec, samp, pos.xz * MUD_TILING);

    var mud_normal = normalize(normal_spec.rbg - vec3<f32>(0.5));
    mud_normal = normalize(rotate_vector_by_vector(mud_normal, normal_in));

    let mix_color = get_overlay(diffuse_gloss.rgb, terrain_color, COLORMAP_MUD_OVERLAY_STRENGTH);
    return MudResult(
        mix(result_in, mix_color, mud),
        normalize(mix(normal_in, mud_normal, mud)),
        mix(glossiness_in, diffuse_gloss.a, mud),
        mix(spec_in, normal_spec.a, mud),
    );
}

// -------------------------------------------------------------------- lighting
struct LightingProperties {
    world_space_pos: vec3<f32>,
    to_camera_dir: vec3<f32>,
    normal: vec3<f32>,
    diffuse: vec3<f32>,
    specular_color: vec3<f32>,
    glossiness: f32,
    non_linear_glossiness: f32,
}

fn ambient_light(world_normal: vec3<f32>, day_factor: f32) -> vec3<f32> {
    // The `NegFogMultiplier` normal bend is a no-op while that constant is 0
    // (R1 section 3.3), so the lerp at the head of `AmbientLight` is dropped.
    let squared = world_normal * world_normal;
    let ix = select(0u, 1u, world_normal.x < 0.0);
    let iy = select(2u, 3u, world_normal.y < 0.0);
    let iz = select(4u, 5u, world_normal.z < 0.0);
    let day = DAY_AMBIENT;
    let night = NIGHT_AMBIENT;
    return squared.x * mix(day[ix], saturate(NIGHT_AMBIENT_BOOST * night[ix]), day_factor)
        + squared.y * mix(day[iy], saturate(NIGHT_AMBIENT_BOOST * night[iy]), day_factor)
        + squared.z * mix(day[iz], saturate(NIGHT_AMBIENT_BOOST * night[iz]), day_factor);
}

fn fresnel_schlick(specular_color: vec3<f32>, e: vec3<f32>, h: vec3<f32>) -> vec3<f32> {
    return specular_color
        + (vec3<f32>(1.0) - specular_color) * pow(1.0 - saturate(dot(e, h)), 5.0);
}

fn fresnel_glossy(
    specular_color: vec3<f32>,
    e: vec3<f32>,
    n: vec3<f32>,
    smoothness: f32,
) -> vec3<f32> {
    return specular_color
        + (max(vec3<f32>(smoothness), specular_color) - specular_color)
            * pow(1.0 - saturate(dot(e, n)), 5.0);
}

fn metalness_to_diffuse(metalness: f32, diffuse: vec3<f32>) -> vec3<f32> {
    return mix(diffuse, vec3<f32>(0.0), metalness);
}

fn metalness_to_spec(metalness: f32, diffuse: vec3<f32>, spec: f32) -> vec3<f32> {
    return mix(vec3<f32>(spec), diffuse, metalness);
}

fn get_non_linear_glossiness(glossiness: f32) -> f32 {
    return exp2(11.0 * glossiness);
}

struct LightOut {
    diffuse: vec3<f32>,
    specular: vec3<f32>,
}

fn improved_blinn_phong(
    light_color: vec3<f32>,
    to_light_dir: vec3<f32>,
    properties: LightingProperties,
) -> LightOut {
    let h = normalize(properties.to_camera_dir + to_light_dir);
    let n_dot_l = saturate(dot(properties.normal, to_light_dir));
    let n_dot_h = saturate(dot(properties.normal, h));
    let normalization = (properties.non_linear_glossiness + 2.0) / 8.0;
    let spec_color = normalization
        * pow(n_dot_h, properties.non_linear_glossiness)
        * fresnel_schlick(properties.specular_color, to_light_dir, h);
    return LightOut(light_color * n_dot_l, spec_color * light_color * n_dot_l);
}

fn calculate_sun_direction_from(
    world_pos: vec3<f32>,
    sun_pos: vec3<f32>,
    second_sun_pos: vec3<f32>,
    moon_pos: vec3<f32>,
    second_moon_pos: vec3<f32>,
) -> vec3<f32> {
    let selected = day_night_factor_range(calc_globe_normal(world_pos.xz), 0.0, 0.0001);
    var source = mix(sun_pos, moon_pos, selected);
    var second_source = mix(second_sun_pos, second_moon_pos, selected);

    if world_pos.x - source.x > MAP_SIZE_X * 0.5 {
        source.x += MAP_SIZE_X;
    } else if world_pos.x - source.x < -MAP_SIZE_X * 0.5 {
        source.x -= MAP_SIZE_X;
    }
    if world_pos.x - second_source.x > MAP_SIZE_X * 0.5 {
        second_source.x += MAP_SIZE_X;
    } else if world_pos.x - second_source.x < -MAP_SIZE_X * 0.5 {
        second_source.x -= MAP_SIZE_X;
    }

    var lerp_factor = abs(world_pos.x - source.x) / (MAP_SIZE_X * 0.5);
    lerp_factor = smoothstep(0.5, 1.0, lerp_factor);
    source = mix(source, second_source, lerp_factor);
    return normalize(world_pos - source);
}

fn calculate_sun_direction(world_pos: vec3<f32>) -> vec3<f32> {
    return calculate_sun_direction_from(
        world_pos,
        globals.frame.virtual_sun_pos.xyz,
        globals.frame.second_virtual_sun_pos.xyz,
        globals.frame.virtual_moon_pos.xyz,
        globals.frame.second_virtual_moon_pos.xyz,
    );
}

/// `CalculateSunDirectionWater` swaps `*_HEIGHT_WATER` in for the `y` of every
/// virtual position, which is what the `.w` of those vectors carries.
fn calculate_sun_direction_water(world_pos: vec3<f32>) -> vec3<f32> {
    return calculate_sun_direction_from(
        world_pos,
        globals.frame.virtual_sun_pos.xwz,
        globals.frame.second_virtual_sun_pos.xwz,
        globals.frame.virtual_moon_pos.xwz,
        globals.frame.second_virtual_moon_pos.xwz,
    );
}

/// The sun and moon contribution `CalculateSunLight` mixes by the day factor.
fn sun_intensity(world_pos: vec3<f32>, shadow_term_in: f32) -> vec3<f32> {
    let globe_normal = calc_globe_normal(world_pos.xz);
    let day_factor = 1.0 - day_night_factor(globe_normal);
    let night_factor = day_night_factor_range(globe_normal, MOON_FEATHER_MIN, MOON_FEATHER_MAX);
    let shadow_term = shadow_term_in * saturate(day_factor + night_factor);
    return globals.frame.sun_diffuse_intensity.rgb
        * globals.frame.sun_diffuse_intensity.a
        * shadow_term
        * day_factor
        + globals.frame.moon_diffuse_intensity.rgb
            * globals.frame.moon_diffuse_intensity.a
            * shadow_term
            * night_factor;
}

fn calculate_light(
    normal: vec3<f32>,
    light_direction: vec3<f32>,
    light_intensity: vec3<f32>,
) -> vec3<f32> {
    return max(dot(normal, -light_direction), 0.0) * light_intensity;
}

fn calculate_pbr_specular_power(
    pos: vec3<f32>,
    normal: vec3<f32>,
    material_specular_color: vec3<f32>,
    specular_power: f32,
    light_color: vec3<f32>,
    light_dir_in: vec3<f32>,
) -> vec3<f32> {
    let h = normalize(normalize(cam_pos() - pos) + -light_dir_in);
    let n_dot_h = saturate(dot(h, normal));
    let n_dot_l = saturate(dot(-light_dir_in, normal));
    // The first assignment in `CalculatePBRSpecularPower` is overwritten by the
    // second without being read, so only the second is ported.
    return fresnel_schlick(material_specular_color * SPECULAR_MULTIPLIER, -light_dir_in, h)
        * ((specular_power + 2.0) / 8.0)
        * saturate(pow(n_dot_h, specular_power))
        * n_dot_l
        * light_color;
}

/// `CalculateSunLight` with PDX_IMPROVED_BLINN_PHONG, which only the `terrain`
/// effect defines (`pdxmap.shader:488-493`).
fn calculate_sun_light(
    properties: LightingProperties,
    shadow_term_in: f32,
    light_source_direction: vec3<f32>,
) -> LightOut {
    let intensity = sun_intensity(properties.world_space_pos, shadow_term_in);
    var light = improved_blinn_phong(intensity, -light_source_direction, properties);
    light.specular *= sun_specular_intensity();
    return light;
}

/// `CalculateSunLight` without that define: the `underwater` and `water`
/// effects declare no defines at all, so they take the plain Phong branch, in
/// which `_Glossiness` is the specular exponent directly rather than through
/// `GetNonLinearGlossiness`. Feeding the underwater pass's `_Glossiness` of 15
/// to `exp2(11 * g)` would overflow to infinity and poison the frame.
fn calculate_sun_light_phong(
    properties: LightingProperties,
    shadow_term_in: f32,
    light_source_direction: vec3<f32>,
) -> LightOut {
    let intensity = sun_intensity(properties.world_space_pos, shadow_term_in);
    var light: LightOut;
    light.diffuse = calculate_light(properties.normal, light_source_direction, intensity);
    light.specular = calculate_pbr_specular_power(
        properties.world_space_pos,
        properties.normal,
        properties.specular_color,
        properties.glossiness,
        intensity,
        light_source_direction,
    ) * sun_specular_intensity();
    return light;
}

fn calc_snow_ambient(snow_factor: f32) -> vec3<f32> {
    return vec3<f32>(0.2, 0.7, 1.0) * 0.07 * smoothstep(0.0, 0.1, snow_factor);
}

fn compose_light(
    properties: LightingProperties,
    diffuse_light: vec3<f32>,
    specular_light: vec3<f32>,
) -> vec3<f32> {
    let day_night = day_night_factor(calc_globe_normal(properties.world_space_pos.xz));
    let ambient = ambient_light(properties.normal, day_night);
    return ((ambient + diffuse_light) * properties.diffuse) * hdr_range() + specular_light;
}

fn compose_light_snow(
    properties: LightingProperties,
    diffuse_light: vec3<f32>,
    specular_light: vec3<f32>,
    snow_factor: f32,
) -> vec3<f32> {
    let day_night = day_night_factor(calc_globe_normal(properties.world_space_pos.xz));
    let ambient = ambient_light(properties.normal, day_night);
    let snow_ambient = calc_snow_ambient(snow_factor);
    return ((snow_ambient + ambient + diffuse_light) * properties.diffuse) * hdr_range()
        + specular_light;
}

// ------------------------------------------------------------ gradient borders
fn gradient_border_multisample_alpha(
    first: vec4<f32>,
    tex: texture_2d<f32>,
    samp: sampler,
    uv: vec2<f32>,
) -> vec4<f32> {
    let offset_x = -0.5 / MAP_SIZE_X;
    let offset_y = -0.5 / MAP_SIZE_Y;
    var result = first;
    result += textureSample(tex, samp, uv + vec2<f32>(-offset_x, 0.0));
    result += textureSample(tex, samp, uv + vec2<f32>(0.0, -offset_y));
    result += textureSample(tex, samp, uv + vec2<f32>(offset_x, 0.0));
    result += textureSample(tex, samp, uv + vec2<f32>(0.0, offset_y));
    result += textureSample(tex, samp, uv + vec2<f32>(-offset_x, -offset_y));
    result += textureSample(tex, samp, uv + vec2<f32>(offset_x, -offset_y));
    result += textureSample(tex, samp, uv + vec2<f32>(offset_x, offset_y));
    result += textureSample(tex, samp, uv + vec2<f32>(-offset_x, offset_y));
    return result / 9.0;
}

fn gradient_border_camera_distance() -> f32 {
    return 1.0 - clamp(cam_distance(GB_CAM_MIN, GB_CAM_MAX), 0.0, GB_CAM_MAX_FILLING_CLAMP);
}

fn gradient_border_distance_to_alpha(distance_in: f32, cam_dist: f32) -> f32 {
    var distance = 1.0 - distance_in;
    distance *= (cam_dist - GB_THRESHOLD) / GB_THRESHOLD2;
    return 1.0 - saturate(distance);
}

struct ChannelResult {
    color: vec3<f32>,
    alpha: f32,
}

fn gradient_border_process_channel(
    init: vec3<f32>,
    cam_dist: f32,
    uv: vec2<f32>,
    gb_tex: texture_2d<f32>,
    gb_tex2: texture_2d<f32>,
    samp: sampler,
    outline_mult: f32,
    outline_cutoff: f32,
    strength_in: f32,
) -> ChannelResult {
    var color = init;
    let pulse_speed_mult = 3.5;
    let fx_alpha = textureSample(gb_tex2, samp, uv).bg;
    let strength = strength_in
        * mix(
            mix(0.45, 1.0, 1.0 - fx_alpha.r),
            1.0,
            (sin(global_time() * pulse_speed_mult) + 1.0) / 2.0,
        );

    let full_width = 5.25 / 255.0;
    let gradient_width = 0.5 / 255.0;

    let gb_dist = gradient_border_multisample_alpha(
        textureSample(gb_tex, samp, uv),
        gb_tex,
        samp,
        uv,
    );
    let alpha = gb_dist.a;

    var color_opacity = levels1(alpha, 0.0, outline_cutoff);
    var outline = 1.0 - levels1(alpha, outline_cutoff, 1.0);
    let old_outline = outline;
    outline *= floor(color_opacity);
    outline *= outline_mult;

    color_opacity = gradient_border_distance_to_alpha(color_opacity, cam_dist);
    color_opacity *= floor(old_outline);

    var thick = smoothstep(
        0.0,
        1.0,
        levels1(alpha, outline_cutoff - full_width, outline_cutoff - full_width + gradient_width),
    );
    thick *= floor(old_outline);

    let max_gradient = max(color_opacity, outline);
    color = mix(color, gb_dist.rgb, max(max_gradient, thick) * strength);
    color = mix(color, color * 0.5, thick);
    return ChannelResult(color, max(max_gradient, thick) * fx_alpha.g);
}

struct GradientResult {
    color: vec3<f32>,
    bloom_alpha: f32,
}

fn gradient_border_apply(
    color_in: vec3<f32>,
    uv_in: vec2<f32>,
    tex1: texture_2d<f32>,
    tex2: texture_2d<f32>,
    samp: sampler,
    outline_mult: f32,
    outline_cutoff: vec2<f32>,
    cam_dist_override: vec2<f32>,
) -> GradientResult {
    let gb_cam_dist = gradient_border_camera_distance();
    // The `int()` cast makes the override a switch, not a blend (R2 section 1.10).
    let cam_dist_ch1 = saturate(gb_cam_dist * f32(i32(1.0 - cam_dist_override.x)) + cam_dist_override.x);
    let cam_dist_ch2 = saturate(gb_cam_dist * f32(i32(1.0 - cam_dist_override.y)) + cam_dist_override.y);

    let half_pixel = 0.5 / gb_texture_height();
    var uv = uv_in;
    uv.y *= 0.5 - half_pixel;
    let uv2 = vec2<f32>(uv.x, uv.y + 0.5);

    let opacity = GB_OPACITY_NEAR + (1.0 - gb_cam_dist) * (GB_OPACITY_FAR - GB_OPACITY_NEAR);

    var color = color_in;
    let first = gradient_border_process_channel(
        color, cam_dist_ch1, uv, tex1, tex2, samp, outline_mult, outline_cutoff.x, GB_STRENGTH_CH1,
    );
    let transparency_a = textureSample(tex2, samp, uv).g;
    color = mix(color, first.color, opacity * transparency_a);

    let second = gradient_border_process_channel(
        color,
        cam_dist_ch2,
        uv2,
        tex1,
        tex2,
        samp,
        outline_mult,
        outline_cutoff.y,
        (1.0 - first.alpha * GB_STRENGTH_CH1 * GB_FIRST_LAYER_PRIORITY) * GB_STRENGTH_CH2,
    );
    let transparency_b = textureSample(tex2, samp, uv2).g;
    color = mix(color, second.color, opacity * transparency_b);

    return GradientResult(color, 1.0 - max(first.alpha, second.alpha));
}

fn calculate_occupation_mask(uv: vec2<f32>) -> f32 {
    let t = 3.14159 / 8.0;
    let w = SEC_MAP_TILE;
    var stripe = cos((uv.x * cos(t) * w) + (uv.y * sin(t) * w));
    let cam_dist = cam_distance(300.0, 1200.0);
    stripe += cam_dist * 1.5;
    return smoothstep(0.0, 1.0, stripe * 1.7) * mix(1.0, 0.3, cam_dist);
}

fn secondary_color_mask(
    color_in: vec3<f32>,
    uv: vec2<f32>,
    mask_texture: texture_2d<f32>,
    samp: sampler,
    bloom_alpha_in: f32,
) -> GradientResult {
    let color_mask = textureSample(mask_texture, samp, uv);
    let occupation = calculate_occupation_mask(uv) * color_mask.a;
    return GradientResult(
        mix(color_in, color_mask.rgb, occupation),
        bloom_alpha_in * (1.0 - occupation),
    );
}

// ------------------------------------------------------------- terrain atlas
struct MapTexIndex {
    index_u: vec4<f32>,
    index_v: vec4<f32>,
    all_same: f32,
}

fn calculate_map_tex_index(ids_in: vec4<f32>) -> MapTexIndex {
    var ids = ids_in * 255.0;
    let all_same = saturate(ids.z - 98.0);
    ids.z -= all_same * 100.0;
    let index_v = trunc((ids + vec4<f32>(0.5)) / MAP_NUM_TILES);
    let index_u = trunc(ids - index_v * MAP_NUM_TILES + vec4<f32>(0.5));
    return MapTexIndex(index_u, index_v, all_same);
}

fn mipmap_level(uv: vec2<f32>) -> f32 {
    let dx = dpdx(uv * TEXELS_PER_TILE);
    let dy = dpdy(uv * TEXELS_PER_TILE);
    let d = max(dot(dx, dx), dot(dy, dy));
    return 0.5 * log2(d);
}

fn sample_terrain_uv(
    index_u: f32,
    index_v: f32,
    tile_repeat_in: vec2<f32>,
    mip_texels: f32,
) -> vec2<f32> {
    var tile_repeat = fract(tile_repeat_in);
    let texels_per_tile = mip_texels / MAP_NUM_TILES;
    tile_repeat *= (texels_per_tile - 1.0) / texels_per_tile;
    return (vec2<f32>(index_u, index_v) + tile_repeat) / MAP_NUM_TILES + 0.5 / mip_texels;
}

// --------------------------------------------------------------------- water
struct Lean {
    b: vec2<f32>,
    m: vec3<f32>,
}

fn blend_lean(t_in: f32, b1: vec2<f32>, m1: vec3<f32>, b2: vec2<f32>, m2: vec3<f32>) -> Lean {
    let u = t_in;
    let t = 1.0 - t_in;
    let t2 = t * t;
    let u2 = u * u;
    let b = b1 * t + b2 * u;
    let m = vec3<f32>(
        m1.x * t2 + m2.x * u2 + 2.0 * t * u * b1.x * b2.x,
        m1.y * t2 + m2.y * u2 + 2.0 * t * u * b1.y * b2.y,
        m1.z * t2 + m2.z * u2 + t * u * b1.x * b2.y + t * u * b1.y * b2.x,
    );
    return Lean(b, m);
}

fn sample_lean(
    uv: vec2<f32>,
    lean_tex1: texture_2d<f32>,
    lean_tex2: texture_2d<f32>,
    samp: sampler,
) -> Lean {
    let lean1 = textureSample(lean_tex1, samp, uv);
    let lean2 = textureSample(lean_tex2, samp, uv);
    let scale = 1.7;
    return Lean(
        (2.0 * lean2.xy - vec2<f32>(1.0)) * scale,
        vec3<f32>(lean2.z, lean2.w, (2.0 * lean1.w - 1.0) * 0.5) * scale * scale,
    );
}

fn sample_blend_lean(
    t: f32,
    uv1: vec2<f32>,
    uv2: vec2<f32>,
    lean_tex1: texture_2d<f32>,
    lean_tex2: texture_2d<f32>,
    samp: sampler,
) -> Lean {
    let a = sample_lean(uv1, lean_tex1, lean_tex2, samp);
    let b = sample_lean(uv2, lean_tex1, lean_tex2, samp);
    return blend_lean(t, a.b, a.m, b.b, b.m);
}

struct WaterSample {
    b: vec2<f32>,
    m: vec3<f32>,
    normal: vec3<f32>,
}

fn sample_water(
    uv: vec2<f32>,
    time_in: f32,
    lean_tex1: texture_2d<f32>,
    lean_tex2: texture_2d<f32>,
    samp: sampler,
) -> WaterSample {
    let uv_multipliers = array<vec2<f32>, 4>(
        1000.0 * vec2<f32>(0.9, 1.0),
        700.0 * vec2<f32>(0.95, 1.05),
        534.0 * vec2<f32>(1.05, 0.95),
        300.0 * vec2<f32>(1.0, 1.0),
    );
    let time_multipliers = array<vec2<f32>, 4>(
        vec2<f32>(1.0, 0.1),
        vec2<f32>(0.1, 2.0),
        vec2<f32>(-0.2, -2.0),
        vec2<f32>(-1.0, -0.1),
    );
    let time = time_in * WATER_TIME_SCALE;
    let first = sample_blend_lean(
        0.5,
        uv * uv_multipliers[0] + time * time_multipliers[0],
        uv * uv_multipliers[1] + time * time_multipliers[1],
        lean_tex1,
        lean_tex2,
        samp,
    );
    let second = sample_blend_lean(
        0.5,
        uv * uv_multipliers[2] + time * time_multipliers[2],
        uv * uv_multipliers[3] + time * time_multipliers[3],
        lean_tex1,
        lean_tex2,
        samp,
    );
    let blended = blend_lean(0.5, first.b, first.m, second.b, second.m);
    var normal = vec3<f32>(blended.b.x, 1.0, blended.b.y);
    normal *= 1.0 / sqrt(normal.x * normal.x + normal.y * normal.y + normal.z * normal.z);
    return WaterSample(blended.b, blended.m, normal);
}
