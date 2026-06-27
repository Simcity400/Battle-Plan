// M4 map shader — FAITHFUL 1:1 translation of the game's political map-mode composite.
//
// The terrain BASE (binding 3) is the game's `PixelShaderTerrain` output, built CPU-side
// in terrain.rs by the exact game math (atlas diffuse · GetOverlay(colormap, 0.75) · game
// sun+ambient lighting for land; the real colormap_water · WATER_COLOR_LIGHTNESS · shore/ice
// for sea), stored 8-bit sRGB (the framebuffer form). This WGSL applies the LAST stage the
// engine does on top: the POLITICAL COUNTRY COLOUR blended at the EXACT game ratio, then the
// country/province BORDERS, rivers, city emissive, and a final gamma encode.
//
// Colour-space discipline (Codex-verified): the game works in LINEAR and gamma-encodes at
// the framebuffer (ToGamma = pow(x, 0.45)). Our base is stored sRGB-encoded; we decode it
// (and the country colour) to LINEAR, do the political lerp in linear (the engine builds the
// mapmode colour in linear before it becomes TerrainColorTint), then gamma-encode the result.
//
// The political blend ratio is the game's: final_terrain = lerp(terrain, country_colour,
// 1 - MAP_MODE_TERRAIN_TRANSPARENCY). With the mod's MAP_MODE_TERRAIN_TRANSPARENCY = 0.8
// [msb_defines.lua:152] that is a 0.2 tint strength: 80% terrain, 20% country. (Codex: the
// engine bakes lerp(country, terrain, TRANSPARENCY); the country colour blend is a plain
// linear transparency, NOT a GetOverlay.)
//
// Integer textures (province ids, owner index) are R32Uint, read with textureLoad. The LUT,
// terrain base, river mask and emissive mask are read via textureLoad (no filtering across
// province boundaries — the political fill must not bleed across borders).

struct MapUniforms {
    screen: vec2<f32>,    // output target size (px)
    map_dims: vec2<f32>,  // province-id raster size (texels)
    offset: vec2<f32>,    // pan offset in map-UV (0..1)
    scale: vec2<f32>,     // zoom scale (1 = full world)
    dst: vec4<f32>,       // destination rect x,y,w,h (output px) the map fills
    // x: fidelity on (1) / off (0)
    // y: POLITICAL TINT STRENGTH = 1 - MAP_MODE_TERRAIN_TRANSPARENCY (game value 0.2)
    // z: NIGHT FACTOR (0 = day, 1 = full night) — drives city emissive [pdxmap:381,385]
    // w: zoom-derived camera distance 0..1 (1 = full-world far, 0 = max zoom near)
    flags: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: MapUniforms;
@group(0) @binding(1) var province_ids: texture_2d<u32>;
@group(0) @binding(2) var lut: texture_2d<f32>;
@group(0) @binding(3) var terrain_base: texture_2d<f32>;
@group(0) @binding(4) var river_mask: texture_2d<f32>;
@group(0) @binding(5) var owner_index: texture_2d<u32>;
@group(0) @binding(6) var emissive_mask: texture_2d<f32>;
@group(0) @binding(7) var secondary_lut: texture_2d<f32>;

// DYNAMIC (time-varying) state — the globals the scrubber drives. Mirrors
// crate::dynamic::DynamicUniforms. state = (night_factor, snow_mud_fade, global_time,
// night_tone_mul); sun_dir = toward-sun vector; cb_* = season colour-balance by latitude band.
struct DynUniforms {
    state: vec4<f32>,
    sun_dir: vec4<f32>,
    hour_sun: vec4<f32>,
    cb_north: vec4<f32>,
    cb_center: vec4<f32>,
    cb_south: vec4<f32>,
    dom_enemy: vec4<f32>,
    dom_friendly: vec4<f32>,
};
@group(0) @binding(8) var<uniform> dyn_u: DynUniforms;
// per-province dynamic LUT (R=winter_level, G=mud_level, B=frozen, A=valid).
@group(0) @binding(9) var dynamic_lut: texture_2d<f32>;
// FAITHFUL gradient-border field — a RASTER-resolution SDF height texture built CPU-side
// by crate::borders::build_border_field. R = country-border height, G = province-border
// height, B = state-border height (each in the engine's GradientBorderChannel.a alpha
// convention: 1 ON the border, ramping to 0 across the bake thickness), A = land mask.
@group(0) @binding(10) var border_field: texture_2d<f32>;

// GROUP(1): real per-pixel MATERIAL textures + the ColorCube grading LUT + the per-pixel
// gate (height/normal/land) + a linear-wrap sampler. These drive the FAITHFUL per-pixel
// ApplySnow / GetMudColor / ApplyIce material and the RestoreScene color-grading postprocess.
@group(1) @binding(0) var snow_tex: texture_2d<f32>;        // snow_normal_rgb_diffuse_a (A=diffuse)
@group(1) @binding(1) var city_snow_noise: texture_2d<f32>; // citylights_rgb_snowmask_a (RGB=city, A=noise)
@group(1) @binding(2) var mud_diffuse_tex: texture_2d<f32>; // mud_diffuse_rgb_gloss_a
@group(1) @binding(3) var mud_normal_tex: texture_2d<f32>;  // mud_normal_rgb_spec_a
@group(1) @binding(4) var ice_diffuse_tex: texture_2d<f32>; // ice_diffuse
@group(1) @binding(5) var ice_noise_tex: texture_2d<f32>;   // ice_noise_0
@group(1) @binding(6) var color_cube: texture_2d<f32>;      // colorcorrection.tga (32^3 LUT)
@group(1) @binding(7) var gate_tex: texture_2d<f32>;        // R=height/range, G=normal.y*0.5+0.5, B=land
@group(1) @binding(8) var lean1_tex: texture_2d<f32>;       // lean1.dds (LEAN wave moments)
@group(1) @binding(9) var lean2_tex: texture_2d<f32>;       // lean2.dds
@group(1) @binding(10) var water_spec_tex: texture_2d<f32>; // fow_rgb_waterspec_a (A = spec map)
@group(1) @binding(11) var mat_samp: sampler;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) px: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
    var out: VsOut;
    var xy = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let p = xy[vid];
    out.clip = vec4<f32>(p, 0.0, 1.0);
    out.px = vec2<f32>(
        (p.x * 0.5 + 0.5) * u.screen.x,
        (1.0 - (p.y * 0.5 + 0.5)) * u.screen.y,
    );
    return out;
}

// --- exact game constants (cited in terrain.rs gc::) used by the WGSL stage ----------
// Final gamma encode: ToGamma = pow(x, 0.45). [standardfuncsgfx.fxh:40]
const GAMMA_ENCODE: f32 = 0.45;
const GAMMA_DECODE: f32 = 2.2;
// City lights additive intensity at night. [constants.fxh:50]
const CITY_LIGHTS_INTENSITY: f32 = 5.5;
// City-lights texture tiling. [constants.fxh:49]
const CITY_LIGHTS_TILING: f32 = 0.09103;
// Occupation-stripe tiling (secondary colour mask). [constants.fxh:216]
const SEC_MAP_TILE: f32 = 6000.0;
const PI: f32 = 3.14159265;
// Snow colour (linear). [constants.fxh:78]
const SNOW_COLOR: vec3<f32> = vec3<f32>(0.46, 0.48, 0.69);
// Ice colour (linear). [constants.fxh:87]
const ICE_COLOR: vec3<f32> = vec3<f32>(0.5, 0.6, 0.9);
// Mud darkens/desaturates terrain; the game overlays a mud material — at political zoom this
// reads as a brown-grey darkening. Strength from COLORMAP_MUD_OVERLAY_STRENGTH. [constants.fxh:94]
const MUD_OVERLAY_STRENGTH: f32 = 0.5;
const MUD_COLOR: vec3<f32> = vec3<f32>(0.22, 0.18, 0.13);
// Day/night terminator constants [standardfuncsgfx.fxh:323-327, constants.fxh].
const GMT_OFFSET: f32 = 2793.0;     // X pixel of the Greenwich meridian
const SOUTH_POLE_OFFSET: f32 = 0.17;
const NORTH_POLE_OFFSET: f32 = 0.93;
const FEATHER_MIN: f32 = -0.01;
const FEATHER_MAX: f32 = 0.01;
const NIGHT_OPACITY: f32 = 0.85;
const NIGHT_DARKNESS: f32 = 0.5;
// Border light removal + night desaturation factors (vBloomAlpha-gated). [constants.fxh:204-206]
const BORDER_LIGHT_REMOVAL_FACTOR: f32 = 0.8;
const BORDER_NIGHT_DESATURATION_MAX: f32 = 0.2;

// CalcGlobeNormal — reconstruct the sphere normal at a map position (the game uses it to
// place the day/night terminator). vWorldXZ is in map-pixel space; map_dims = (X,Y).
// [standardfuncsgfx.fxh:347-356]
fn calc_globe_normal(world_xz: vec2<f32>, dims: vec2<f32>, hour_phase: f32) -> vec3<f32> {
    var x = ((world_xz.x - GMT_OFFSET) / dims.x + hour_phase);
    x = x - floor(x); // fmod_loop to [0,1)
    var y = world_xz.y / dims.y;
    y = SOUTH_POLE_OFFSET + (NORTH_POLE_OFFSET - SOUTH_POLE_OFFSET) * y;
    y = -cos(y * PI);
    let xz_len = 1.0 - abs(y);
    let n = vec3<f32>(sin(x * 2.0 * PI) * xz_len, y, cos(x * 2.0 * PI) * xz_len);
    return normalize(n);
}

// Per-pixel DayNightFactor: 1 = night, 0 = day, feathered at the terminator.
// [standardfuncsgfx.fxh:358-361]
fn day_night_factor(globe_n: vec3<f32>, sun: vec3<f32>) -> f32 {
    let d = dot(globe_n, sun);
    return clamp((d - FEATHER_MIN) / (FEATHER_MAX - FEATHER_MIN), 0.0, 1.0);
}

// NightifyColor — the game's faithful night look: grey via weights (0.4,0.3,0.05), blended
// toward a blue night colour, cubic desaturation by vBlend, × NIGHT_DARKNESS.
// [standardfuncsgfx.fxh:370-381]
fn nightify_color(day_color: vec3<f32>, blend: f32) -> vec3<f32> {
    let desat = mix(0.0, 0.8, blend * blend * blend);
    let grey = dot(day_color, vec3<f32>(0.4, 0.3, 0.05));
    let night_col = clamp(mix(vec3<f32>(grey), grey * vec3<f32>(0.2, 0.7, 1.2), vec3<f32>(0.25)), vec3<f32>(0.0), vec3<f32>(1.0));
    let c = mix(day_color, night_col, vec3<f32>(desat));
    return c * NIGHT_DARKNESS;
}

// DayNightWithBlend — lerp(dayColor, NightifyColor, DayNightFactor * NIGHT_OPACITY).
// [standardfuncsgfx.fxh:384-393]
fn day_night_with_blend(day_color: vec3<f32>, night_factor: f32, blend: f32) -> vec3<f32> {
    return mix(day_color, nightify_color(day_color, blend), night_factor * NIGHT_OPACITY);
}
// Distance fog colour (linear) + max factor. [constants.fxh:152,155]
const FOG_COLOR: vec3<f32> = vec3<f32>(0.12, 0.28, 0.6);
const FOG_MAX: f32 = 0.35;
// Border colours (linear). The gradient-border country line is a dark outline; province
// borders are a faint darker line. Country line darkens its base (the game's outline edge
// `vCh * .5` darkening, [standardfuncsgfx.fxh:881]).
const NEUTRAL_TERRAIN: vec4<f32> = vec4<f32>(60.0 / 255.0, 74.0 / 255.0, 92.0 / 255.0, 1.0);

fn to_linear3(c: vec3<f32>) -> vec3<f32> { return pow(max(c, vec3<f32>(0.0)), vec3<f32>(GAMMA_DECODE)); }
fn to_gamma3(c: vec3<f32>) -> vec3<f32> { return pow(max(c, vec3<f32>(0.0)), vec3<f32>(GAMMA_ENCODE)); }

// RGBtoHSV / HSVtoRGBPost / Levels — the game's [standardfuncsgfx.fxh:112-132,102-110,172-176].
// Used by the season postprocess (restorescene.shader RestoreScene).
fn rgb_to_hsv(rgb: vec3<f32>) -> vec3<f32> {
    let cmax = max(rgb.r, max(rgb.g, rgb.b));
    let cmin = min(rgb.r, min(rgb.g, rgb.b));
    let diff = cmax - cmin;
    var h = 0.0;
    var s = 0.0;
    if (diff != 0.0) {
        s = diff / cmax;
        if (cmax == rgb.r) {
            h = (rgb.g - rgb.b) / diff + 6.0;
        } else if (cmax == rgb.g) {
            h = (rgb.b - rgb.r) / diff + 2.0;
        } else {
            h = (rgb.r - rgb.g) / diff + 4.0;
        }
    }
    return vec3<f32>(h, s, cmax);
}
// HuePost — the game's exact [standardfuncsgfx.fxh:91-100]: X = 1 - |mod(H,2)-1|, 6 sectors.
fn hue_post(h: f32) -> vec3<f32> {
    let x = 1.0 - abs((h - 2.0 * floor(h / 2.0)) - 1.0);
    if (h < 1.0) { return vec3<f32>(1.0, x, 0.0); }
    else if (h < 2.0) { return vec3<f32>(x, 1.0, 0.0); }
    else if (h < 3.0) { return vec3<f32>(0.0, 1.0, x); }
    else if (h < 4.0) { return vec3<f32>(0.0, x, 1.0); }
    else if (h < 5.0) { return vec3<f32>(x, 0.0, 1.0); }
    else { return vec3<f32>(1.0, 0.0, x); }
}
fn hsv_to_rgb_post(hsv: vec3<f32>) -> vec3<f32> {
    if (hsv.y != 0.0) {
        let c = hsv.y * hsv.z;
        return clamp(hue_post(hsv.x) * c + (hsv.z - c), vec3<f32>(0.0), vec3<f32>(1.0));
    }
    return clamp(vec3<f32>(hsv.z), vec3<f32>(0.0), vec3<f32>(1.0));
}
fn levels3(c: vec3<f32>, lo: f32, hi: f32) -> vec3<f32> {
    return clamp((clamp(c - vec3<f32>(lo), vec3<f32>(0.0), vec3<f32>(1.0))) / (hi - lo), vec3<f32>(0.0), vec3<f32>(1.0));
}

// Uncharted2 filmic tonemap [restorescene.shader:155-167]. A=0.22 B=0.30 C=0.10 D=0.20 E=0.01 F=0.30.
fn uncharted2(x: vec3<f32>) -> vec3<f32> {
    let a = 0.22; let b = 0.30; let c = 0.10; let d = 0.20; let e = 0.01; let f = 0.30;
    return ((x * (a * x + c * b) + d * e) / (x * (a * x + b) + d * f)) - e / f;
}
// The full UNCHARTED ToneMap branch [restorescene.shader:193-201]: tonemap with W=11.2,
// white-scale, then pow(1/2.2) — THIS is the gamma encode (replaces a separate to_gamma).
fn tonemap_uncharted(linear_col: vec3<f32>) -> vec3<f32> {
    let w = 11.2;
    let curr = uncharted2(linear_col); // ExposureBias = 1
    let white_scale = vec3<f32>(1.0) / uncharted2(vec3<f32>(w));
    return pow(curr * white_scale, vec3<f32>(1.0 / 2.2));
}

// SampleColorCube — trilinear 32^3 LUT lookup from colorcorrection.tga (1024x32).
// [restorescene.shader:91-108]. Input/output in display [0,1].
fn sample_color_cube(in_color: vec3<f32>) -> vec3<f32> {
    let cube_size = 32.0;
    let scale = (cube_size - 1.0) / cube_size;
    let offset = 0.5 / cube_size;
    let x = (scale * in_color.r + offset) / cube_size;
    let y = scale * in_color.g + offset;
    let z_floor = floor((scale * in_color.b + offset) * cube_size);
    let x_off1 = z_floor / cube_size;
    let x_off2 = min(cube_size - 1.0, z_floor + 1.0) / cube_size;
    let c1 = textureSampleLevel(color_cube, mat_samp, vec2<f32>(x + x_off1, y), 0.0).rgb;
    let c2 = textureSampleLevel(color_cube, mat_samp, vec2<f32>(x + x_off2, y), 0.0).rgb;
    return mix(c1, c2, scale * in_color.b * cube_size - z_floor);
}
// Scalar Levels — the game's [standardfuncsgfx.fxh:179-183]:
// saturate( (v - lo) / (hi - lo) ).
fn levels1(v: f32, lo: f32, hi: f32) -> f32 {
    return clamp((v - lo) / (hi - lo), 0.0, 1.0);
}

// ===== FAITHFUL gradient-border processing (ported from standardfuncsgfx.fxh) =====
// The engine's GradientBorderChannel.a is a normalized DISTANCE-TO-BORDER "height": high
// ON the border, ramping down into the interior. We bake that height CPU-side
// (crate::borders) into border_field's R/G/B channels. These functions reproduce the
// shader's height→fill/outline conversion 1:1.

// gradient-border engine constants [constants.fxh:197-199, 00_graphics.lua:880,883].
const GB_THRESHOLD: f32 = 0.05;
const GB_THRESHOLD2: f32 = 0.25;
const GB_CAM_MAX_FILLING_CLAMP: f32 = 0.8;
const GB_OUTLINE_CUTOFF_COUNTRY: f32 = 0.973;
const GB_OUTLINE_CUTOFF_STATE: f32 = 0.973;
// GB_OUTLINE_CUTOFF_SEA = 0.990 [constants.fxh:201] — the engine's WATER-shader
// gradient_border_apply cutoff for COASTAL / sea-side edges [pdxwater.shader:351-355].
const GB_OUTLINE_CUTOFF_SEA: f32 = 0.990;
// vFullWidth / vGradientWidth from gradient_border_process_channel [fxh:847-848].
const GB_FULL_WIDTH: f32 = 5.25 / 255.0;
const GB_GRADIENT_WIDTH: f32 = 0.5 / 255.0;
// Priority of the FIRST (country) layer over the second (state) when both are active at a
// pixel. GB_FIRST_LAYER_PRIORITY = 0.4 [constants.fxh:209]. The engine reduces Ch2's
// strength by (1 - alpha1*GB_STRENGTH_CH1*GB_FIRST_LAYER_PRIORITY) [fxh:923].
const GB_FIRST_LAYER_PRIORITY: f32 = 0.4;
// Per-channel CAMERA-DISTANCE OVERRIDE [00_graphics.lua:890-891]: country = 0 (no override,
// the full vGBCamDist ramp applies), state = 0.4. The engine blends:
//   vGBCamDistChN = saturate( vGBCamDist*int(1-override) + override )   [fxh:901-902]
// int(1-override) truncates to 0 unless override==0 → so any non-zero override REPLACES the
// camera ramp with the constant `override`, and override==0 keeps the live ramp.
const GB_CAM_OVERRIDE_COUNTRY: f32 = 0.0;
const GB_CAM_OVERRIDE_STATE: f32 = 0.4;
// Border blend OPACITY ramped by camera distance [constants.fxh:202-203]: the engine blends
// the gradient colour by (GB_OPACITY_NEAR + (1-vGBCamDist)*(GB_OPACITY_FAR-GB_OPACITY_NEAR))
// * Transp [fxh:915,919]. GB_OPACITY_NEAR=1.0 (near), GB_OPACITY_FAR=0.85 (far).
const GB_OPACITY_NEAR: f32 = 1.0;
const GB_OPACITY_FAR: f32 = 0.85;

// gradient_border_apply's per-channel camera-distance override [fxh:901-902]:
//   saturate( vGBCamDist * int(1 - override) + override ).
// WGSL has no `int()` truncation-toward-zero of a float-in-[0,1]; for override in [0,1],
// int(1-override) is 1 iff override==0 else 0 — i.e. the live ramp survives only when there
// is no override. Reproduce exactly with a select on override==0.
fn gb_cam_dist_override(cam: f32, cam_override: f32) -> f32 {
    if (cam_override == 0.0) {
        return clamp(cam, 0.0, 1.0);          // int(1-0)=1 → keep the live ramp
    }
    return clamp(cam_override, 0.0, 1.0);     // int(1-override)=0 → constant override
}
// The camera-distance OPACITY ramp that scales the gradient-colour blend [fxh:915,919]:
//   GB_OPACITY_NEAR + (1 - vGBCamDist) * (GB_OPACITY_FAR - GB_OPACITY_NEAR).
fn gb_opacity_ramp(gb_cam: f32) -> f32 {
    return GB_OPACITY_NEAR + (1.0 - gb_cam) * (GB_OPACITY_FAR - GB_OPACITY_NEAR);
}

// gradient_border_distance_to_alpha [standardfuncsgfx.fxh:818-822]:
//   vDist = 1 - vDist;  vDist *= (vCamDist - GB_THRESHOLD)/GB_THRESHOLD2;  return 1 - saturate(vDist).
// This is the "whole magic": it converts the static height to a camera-distance-dependent
// FILL — at far camera (high vCamDist) the band fills more; at near camera it narrows to the
// thin line. (gb_cam_dist below is the engine's gradient_border_camera_distance, 0..1.)
fn gradient_border_distance_to_alpha(vDist: f32, vCamDist: f32) -> f32 {
    var d = 1.0 - vDist;
    d = d * ((vCamDist - GB_THRESHOLD) / GB_THRESHOLD2);
    return 1.0 - clamp(d, 0.0, 1.0);
}

// gradient_border_camera_distance [standardfuncsgfx.fxh:813]:
//   1 - clamp( cam_distance(GB_CAM_MIN, GB_CAM_MAX), 0, GB_CAM_MAX_FILLING_CLAMP ).
// The engine's cam_distance(100,350) is 0 NEAR .. 1 FAR. Our flags.w has the SAME
// orientation — flags.w = 1 at full-world (FAR), 0 at max zoom (NEAR) [MapUniforms.flags.w
// doc] — so flags.w IS the engine's normalized cam_distance (NOT its inverse; verified vs
// flags_for: cam_dist = 1/zoom, zoom≥1 → 1 at full-world). Therefore:
//   vGBCamDist = 1 - clamp(flags.w, 0, 0.8)  →  NEAR(flags.w=0)→1.0, FAR(flags.w=1)→0.2,
// exactly the engine's NEAR→1 (thin filled line) / FAR→0.2 (more fill). This drives the
// distance_to_alpha fill that widens the visible band toward 25px (HIGH) near and narrows
// it toward 5px (LOW) far. (Codex (b): endpoints confirmed equal to the engine; the
// "inversion" Codex raised was a naming concern, not a numeric one — flags.w == cam_distance.)
fn gradient_border_camera_distance(cam01: f32) -> f32 {
    return 1.0 - clamp(cam01, 0.0, GB_CAM_MAX_FILLING_CLAMP);
}

// gradient_border_process_channel [standardfuncsgfx.fxh:840-883], reduced to the political
// map's needs (no FX pulse texture — FX_Alpha.r=0 so vStrength is unchanged, FX_Alpha.g=1).
// Produces the channel's `vGradMix` (= the engine `vCh` out-param) AND returns the channel
// coverage `max(vMaxGradient,vThick)` [fxh:883] used for the next-layer strength reduction
// and the season-protect vBloomAlpha.
//
// COLOUR RECONCILIATION (Codex-verified, finding 1): the engine has ONE `vGBDist.rgb` (the
// country's map colour) for both the broad FILL and the razor OUTLINE, and it works because
// the engine's BASE `vColor` is ALREADY the country mapmode colour — so the interior FILL
// lerp is near-identity and only the `vCh*0.5` outline darken reads. OUR base `col` is
// terrain + a deliberate 0.2 political tint (NOT the full country colour), so lerping the
// broad fill toward the full country colour FLOODS the interior. We therefore split
// `vGBDist.rgb` into two faithful targets:
//   * `fill_rgb`  = the base itself (identity broad fill → NO interior flood, matching the
//                   engine's near-identity interior), and
//   * `outline_rgb` = the FULL country colour for the razor OUTLINE/thick band, so the dark
//                   border line keeps the engine's country-colour identity (`country*0.5`)
//                   even over terrain-heavy/coastal pixels where `col` is mostly terrain.
// This reproduces the engine's NET output (dark anti-aliased country-coloured line at the
// boundary, interior untouched) without the flood the low-tint base would otherwise cause.
struct GBResult {
    grad_mix: vec3<f32>, // the channel-coloured result (engine vGradMix)
    coverage: f32,       // engine return value max(vMaxGradient,vThick)
}
fn gradient_border_process(base: vec3<f32>, fill_rgb: vec3<f32>, outline_rgb: vec3<f32>, alpha_h: f32, cam: f32, outline_cutoff: f32, strength: f32) -> GBResult {
    var vCh = base;
    // vColorOpacity = Levels(Alpha, 0, vOutlineCutoff)   [fxh:856]
    var vColorOpacity = levels1(alpha_h, 0.0, outline_cutoff);
    // vOutline = 1 - Levels(Alpha, vOutlineCutoff, 1)    [fxh:857]
    var vOutline = 1.0 - levels1(alpha_h, outline_cutoff, 1.0);
    let vOldOutline = vOutline;
    vOutline = vOutline * floor(vColorOpacity);          // [fxh:859] only inside the fill
    // (vOutlineMult = 1 on the political map; omitted.)

    // Convert "heightmap" to "fill" regarding camera distance.   [fxh:865]
    vColorOpacity = gradient_border_distance_to_alpha(vColorOpacity, cam);
    // fill XOR outline — never both > 0.   [fxh:869]
    vColorOpacity = vColorOpacity * floor(vOldOutline);

    // vThick = smoothstep(0,1, Levels(Alpha, cutoff-fullW, cutoff-fullW+gradW)).   [fxh:872]
    var vThick = smoothstep(0.0, 1.0, levels1(alpha_h, outline_cutoff - GB_FULL_WIDTH, outline_cutoff - GB_FULL_WIDTH + GB_GRADIENT_WIDTH));
    vThick = vThick * floor(vOldOutline);                // [fxh:874]

    let vMaxGradient = max(vColorOpacity, vOutline);
    let cov = max(vMaxGradient, vThick);
    // Engine: vCh = lerp(vCh, vGBDist.rgb, max(vMaxGradient,vThick)*vStrength) [fxh:878]. We
    // lerp the BROAD fill (vMaxGradient) toward `fill_rgb` (= base → identity, no flood) and
    // the razor THICK band (vThick) toward `outline_rgb` (the full country colour). Where
    // vThick>0 the outline colour wins; elsewhere the fill is identity. This is the engine's
    // single lerp specialized to the two targets (Codex finding 1 reconciliation).
    vCh = mix(vCh, fill_rgb, clamp(vMaxGradient * strength, 0.0, 1.0));
    vCh = mix(vCh, outline_rgb, clamp(vThick * strength, 0.0, 1.0));
    // Make the outline edge darker: vCh = lerp(vCh, vCh*0.5, vThick) [fxh:881]. With the thick
    // band already = country colour, this yields the engine's country*0.5 dark line.
    vCh = mix(vCh, vCh * 0.5, vThick);
    var out: GBResult;
    out.grad_mix = vCh;
    // engine returns max(vMaxGradient,vThick)*FX_Alpha.g [fxh:883]; FX_Alpha.g=1 (no FX).
    out.coverage = clamp(cov, 0.0, 1.0);
    return out;
}

// NOTE on gradient_border_multisample_alpha [standardfuncsgfx.fxh:791-810]: the engine 9-taps
// its baked GradientBorderChannel texture at ±half-texel and averages to anti-alias the
// border ALPHA — because that channel texture is a quantized/aliased raster. OUR border field
// is a true Euclidean SDF: the height is ALREADY sub-pixel-smooth (linear in distance), so a
// box average would only WIDEN and soften the razor cutoff band (Codex finding 3). We
// therefore do NOT multisample — sampling the raw SDF height IS the faithful, already-AA'd
// equivalent of the engine's 9-tap output. (Verified: averaging an exact distance field over
// a 3x3 neighbourhood is strictly worse than the field itself near a straight edge.)

// Map a map-UV (0..1) to the CLAMPED integer id-raster texel — the ONE shared
// id-coordinate computation used by the fill and every border tap.
fn id_coord(map_uv: vec2<f32>) -> vec2<i32> {
    let dimsf = u.map_dims;
    return clamp(
        vec2<i32>(floor(map_uv * dimsf)),
        vec2<i32>(0, 0),
        vec2<i32>(i32(dimsf.x) - 1, i32(dimsf.y) - 1),
    );
}

fn load_pid(coord: vec2<i32>) -> u32 {
    return textureLoad(province_ids, coord, 0).r;
}
fn owner_of_pid(pid: u32) -> u32 {
    let w = textureDimensions(owner_index).x;
    if (pid >= w) {
        return 0u;
    }
    return textureLoad(owner_index, vec2<i32>(i32(pid), 0), 0).r;
}

// The game's CalculateOccupationMask — diagonal cosine stripes at SEC_MAP_TILE freq.
// [standardfuncsgfx.fxh:965-977]. `cam` is the camera-distance factor (0 near .. 1 far),
// our equivalent of cam_distance(300,1200). The stripes thin and fade as you zoom out.
fn occupation_mask(uv: vec2<f32>, cam: f32) -> f32 {
    let t = PI / 8.0;
    let w = SEC_MAP_TILE;
    var s = cos(uv.x * cos(t) * w + uv.y * sin(t) * w);
    s = s + cam * 1.5;
    s = smoothstep(0.0, 1.0, s * 1.7) * mix(1.0, 0.3, cam);
    return clamp(s, 0.0, 1.0);
}

// --- per-pixel material constants (the game's, [constants.fxh]) ----------------------
const SNOW_START_HEIGHT: f32 = 3.0;       // [75]
const SNOW_RIDGE_START_HEIGHT: f32 = 11.0;// [76]
const SNOW_NORMAL_START: f32 = 0.7;       // [77]
const SNOW_CLIFFS: f32 = 5.0;             // [80]
const SNOW_TILING: f32 = 0.05;            // [82]
const SNOW_NOISE_TILING: f32 = 0.06;      // [83]
const SNOW_OPACITY_MIN: f32 = 0.95;       // [65]
const SNOW_OPACITY_MAX: f32 = 0.2;        // [66]
const SNOW_FROST_MIN_EFFECT: f32 = 0.4;   // [85]
const MUD_TILING: f32 = 0.09;             // [61]
const ICE_NOISE_TILING: f32 = 0.1;        // [88]
const GATE_HEIGHT_RANGE: f32 = 25.3;      // matches terrain::GATE_HEIGHT_RANGE

// RotateVectorByVector — compose a tangent-space material normal onto the world normal
// (Frisvad basis from the up vector). Mirrors standardfuncsgfx.fxh:59 / terrain.rs.
fn rotate_vec_by_vec(v1: vec3<f32>, v2: vec3<f32>) -> vec3<f32> {
    let up = v2;
    var q = vec3<f32>(0.0, 0.0, 1.0);
    if (abs(up.z) >= 0.999) { q = vec3<f32>(1.0, 0.0, 0.0); }
    let t = normalize(cross(q, up));
    let b = cross(up, t);
    return v1.x * t + v1.y * up + v1.z * b;
}

// GetOverlay — the gamma-space photoshop overlay (mud uses it). [standardfuncsgfx.fxh:159-169]
fn get_overlay(base: vec3<f32>, overlay: vec3<f32>, pct: f32) -> vec3<f32> {
    let cg = to_gamma3(base);
    let og = to_gamma3(overlay);
    var res: vec3<f32>;
    res.r = select(1.0 - 2.0 * (1.0 - og.r) * (1.0 - cg.r), 2.0 * og.r * cg.r, og.r < 0.5);
    res.g = select(1.0 - 2.0 * (1.0 - og.g) * (1.0 - cg.g), 2.0 * og.g * cg.g, og.g < 0.5);
    res.b = select(1.0 - 2.0 * (1.0 - og.b) * (1.0 - cg.b), 2.0 * og.b * cg.b, og.b < 0.5);
    let lin = to_linear3(res);
    return mix(base, lin, pct);
}

// ApplySnow (per-pixel, [standardfuncsgfx.fxh:264-301]): blend the REAL snow material
// (snow_tex.a · SNOW_COLOR) by the height/normal/noise/winter gates + camera opacity. Returns
// the new colour. `cam` = flags.w (1 far .. 0 near) ~ cam_distance(SNOW_CAM_MIN,MAX).
fn apply_snow(color: vec3<f32>, world_pos: vec3<f32>, normal_y: f32, winter: f32, cam: f32) -> vec3<f32> {
    let v_snow_fade = clamp(world_pos.y - SNOW_START_HEIGHT, 0.0, 1.0);
    let v_normal_fade = clamp(clamp(normal_y - SNOW_NORMAL_START, 0.0, 1.0) * SNOW_CLIFFS, 0.0, 1.0);
    let snow_diffuse = textureSampleLevel(snow_tex, mat_samp, world_pos.xz * SNOW_TILING, 0.0).a;
    let v_noise = textureSampleLevel(city_snow_noise, mat_samp, world_pos.xz * SNOW_NOISE_TILING, 0.0).a;
    let v_is_snow = winter;
    var v_transp = v_noise;
    v_transp = v_transp + clamp(world_pos.y - SNOW_RIDGE_START_HEIGHT, 0.0, 1.0) * (clamp((normal_y - 0.9) * 1000.0, 0.0, 1.0) * v_is_snow);
    v_transp = clamp(v_transp, 0.0, 1.0);
    let v_snow = clamp(clamp(v_transp - (1.0 - v_is_snow), 0.0, 1.0) * 5.0, 0.0, 1.0);
    let v_frost = clamp(clamp(v_transp + 0.5, 0.0, 1.0) - (1.0 - v_is_snow), 0.0, 1.0);
    // camera opacity ramp: SNOW_OPACITY_MIN at near (cam 0) .. MAX at far (cam 1).
    let v_opacity = SNOW_OPACITY_MIN + cam * (SNOW_OPACITY_MAX - SNOW_OPACITY_MIN);
    let v_snow_alpha = clamp(clamp(v_snow + v_frost, 0.0, 1.0) * v_snow_fade * v_normal_fade * clamp(v_is_snow * 2.25, 0.0, 1.0) * v_opacity, 0.0, 1.0);
    let v_min_snow = smoothstep(0.0, 1.0, v_is_snow);
    // vSnowAlphaOut starts at 1 (1 - vSpec for fully-diffuse terrain); use 1.0.
    return mix(color, snow_diffuse * SNOW_COLOR, clamp(v_snow_alpha + SNOW_FROST_MIN_EFFECT * v_min_snow, 0.0, 1.0));
}

// GetMudColor (per-pixel, [standardfuncsgfx.fxh:237-256]): overlay the REAL mud material
// (GetOverlay(mud_diffuse, colormap, 0.5)) by the mud level (gated by noise). TerrainColor is
// the local colormap base (we use the current terrain colour as the overlay reference).
fn apply_mud(color: vec3<f32>, world_pos: vec3<f32>, mud_level: f32, terrain_col: vec3<f32>, cam: f32) -> vec3<f32> {
    // vNoise = lerp(1.0, (0.5 + SnowNoise.a)*0.5, cam_distance(MUD_CAM_MIN,MAX)). cam here is
    // flags.w (1 far .. 0 near); the game's cam_distance(50,300) is 0 near .. 1 far → same. [fxh:240-241]
    let noise_a = textureSampleLevel(city_snow_noise, mat_samp, world_pos.xz * 0.01, 0.0).a;
    let v_noise = mix(1.0, (0.5 + noise_a) * 0.5, cam);
    let v_mud_current = clamp(mud_level * v_noise, 0.0, 1.0); // MUD_STRENGHTEN=1
    let mud_dg = textureSampleLevel(mud_diffuse_tex, mat_samp, world_pos.xz * MUD_TILING, 0.0).rgb;
    let mud_mix = get_overlay(mud_dg, terrain_col, 0.5); // COLORMAP_MUD_OVERLAY_STRENGTH
    return mix(color, mud_mix, v_mud_current);
}

// ApplyIce (per-pixel, [pdxwater.shader:231-253]): frozen sea → blend the REAL ice diffuse
// tinted by ICE_COLOR, broken up by ice noise + the north map-limit fade. `frozen` 0..1.
fn apply_ice(color: vec3<f32>, world_pos: vec3<f32>, map_v: f32, frozen: f32, cam: f32) -> vec3<f32> {
    // EXACT ApplyIce [pdxwater.shader:231-253]: vIceFade = GetSnow*8 · IceNoise(vPos·tiling) ·
    // (1 - cam_distance(100,350)) · vMapLimitFade, then (fade-0.3)*10. The game uses the snow
    // (winter) level (== our `frozen` driver, the SnowMudData.g winter channel).
    let v_ice_noise = textureSampleLevel(ice_noise_tex, mat_samp, (world_pos.xz + 0.5) * ICE_NOISE_TILING, 0.0).r;
    var v_ice_fade = clamp(frozen, 0.0, 1.0) * 8.0 * v_ice_noise;
    // ice opacity = 1 - cam_distance(ICE_CAM_MIN=100, ICE_CAM_MAX=350): high near, 0 far.
    v_ice_fade = v_ice_fade * (1.0 - cam);
    // map-limit fade: NORTH only — saturate((map_v - 0.74)*800) (map_v grows downward = south,
    // but the game's y grows the SAME way as map_v here; the engine intends the far latitude).
    let v_map_limit = clamp((map_v - 0.74) * 800.0, 0.0, 1.0);
    v_ice_fade = v_ice_fade * v_map_limit;
    v_ice_fade = clamp((v_ice_fade - 0.3) * 10.0, 0.0, 1.0);
    let ice_d = textureSampleLevel(ice_diffuse_tex, mat_samp, world_pos.xz * 0.05, 0.0);
    // vColor = lerp(vColor, ICE_COLOR * vIceDiffuse.a, vIceFade) [pdxwater:252-253].
    let ice_color = ICE_COLOR * ice_d.a;
    return mix(color, ice_color, v_ice_fade);
}

// --- WATER SURFACE (the animated LEAN wave shimmer + spec map) the game draws on the sea.
// For the TOP-DOWN political view (Codex-verified): the cubemap reflection collapses to a
// constant +Y sky and fresnel ≈ 0.5, so the deep-water base is a near-constant; the surviving
// VISIBLE animation is the LEAN wave normal (lean1/lean2 by global_time) flattened by the
// water spec map, producing a sun-spec glint. We add that glint to the water base.
// [pdxwater.shader:287-322; standardfuncsgfx.fxh:1165-1240]

const WATER_TIME_SCALE: f32 = 0.02; // 1/50 [constants.fxh:132]
// the +Y sky-face mean of reflection.dds (Codex-decoded), for the constant reflection.
const WATER_SKY: vec3<f32> = vec3<f32>(0.3553, 0.3741, 0.4104);

// BlendLEAN [standardfuncsgfx.fxh:1165-1177].
fn blend_lean(t: f32, b1: vec2<f32>, m1: vec3<f32>, b2: vec2<f32>, m2: vec3<f32>) -> array<vec4<f32>, 2> {
    let u = t;
    let tt = 1.0 - t;
    let t2 = tt * tt;
    let u2 = u * u;
    let bout = b1 * tt + b2 * u;
    let mout = vec3<f32>(
        m1.x * t2 + m2.x * u2 + 2.0 * tt * u * b1.x * b2.x,
        m1.y * t2 + m2.y * u2 + 2.0 * tt * u * b1.y * b2.y,
        m1.z * t2 + m2.z * u2 + tt * u * b1.x * b2.y + tt * u * b1.y * b2.x,
    );
    return array<vec4<f32>, 2>(vec4<f32>(bout, 0.0, 0.0), vec4<f32>(mout, 0.0));
}
// one LEAN tap → (B, M) per standardfuncsgfx.fxh:1179-1187.
fn lean_tap(uv: vec2<f32>) -> array<vec4<f32>, 2> {
    let l1 = textureSampleLevel(lean1_tex, mat_samp, uv, 0.0);
    let l2 = textureSampleLevel(lean2_tex, mat_samp, uv, 0.0);
    let v_scale = 1.7;
    let b = (2.0 * l2.xy - 1.0) * v_scale;
    let m = vec3<f32>(l2.z, l2.w, (2.0 * l1.w - 1.0) * 0.5) * v_scale * v_scale;
    return array<vec4<f32>, 2>(vec4<f32>(b, 0.0, 0.0), vec4<f32>(m, 0.0));
}
fn blend_two(t: f32, uv1: vec2<f32>, uv2: vec2<f32>) -> array<vec4<f32>, 2> {
    let a = lean_tap(uv1);
    let b = lean_tap(uv2);
    return blend_lean(t, a[0].xy, a[1].xyz, b[0].xy, b[1].xyz);
}

// Apply the animated water surface to a SEA pixel. Returns the water colour with the LEAN
// wave shimmer + spec-map flatten + sun-spec glint added. `global_time` drives the waves.
fn apply_water_surface(color: vec3<f32>, uv: vec2<f32>, world_pos: vec3<f32>, sun_dir: vec3<f32>, global_time: f32) -> vec3<f32> {
    let vt = global_time * WATER_TIME_SCALE;
    // the 4 UV/time multipliers [standardfuncsgfx.fxh:1228-1238].
    let r1 = blend_two(0.5, uv * 1000.0 * vec2<f32>(0.9, 1.0) + vt * vec2<f32>(1.0, 0.1),
                            uv * 700.0 * vec2<f32>(0.95, 1.05) + vt * vec2<f32>(0.1, 2.0));
    let r2 = blend_two(0.5, uv * 534.0 * vec2<f32>(1.05, 0.95) + vt * vec2<f32>(-0.2, -2.0),
                            uv * 300.0 * vec2<f32>(1.0, 1.0) + vt * vec2<f32>(-1.0, -0.1));
    let blended = blend_lean(0.5, r1[0].xy, r1[1].xyz, r2[0].xy, r2[1].xyz);
    var b = blended[0].xy;
    let m = blended[1].xyz;
    var normal = normalize(vec3<f32>(b.x, 1.0, b.y));

    // water spec map flatten [pdxwater.shader:289-296].
    let v_spec = textureSampleLevel(water_spec_tex, mat_samp, uv, 0.0).a;
    normal.y += (1.0 - v_spec);
    normal = vec3<f32>(normal.x * v_spec, normal.y, normal.z * v_spec);
    normal = normalize(normal);
    b = b * v_spec;
    let mm = m * v_spec * v_spec;

    // Ward anisotropic spec [pdxwater.shader:303-309]. View ≈ top-down (cam above).
    let view = vec3<f32>(0.0, 1.0, 0.0);
    let h = normalize(view.xzy + -sun_dir.xzy);
    let h_wave = h.xy / h.z - b;
    let sigma = mm - vec3<f32>(b.x * b.x, b.y * b.y, b.x * b.y);
    let det = sigma.x * sigma.y - sigma.z * sigma.z;
    let e = h_wave.x * h_wave.x * sigma.y + h_wave.y * h_wave.y * sigma.x - 2.0 * h_wave.x * h_wave.y * sigma.z;
    var spec = 0.0;
    if (det > 0.0) {
        spec = exp(-0.5 * e / det) / sqrt(det);
    }
    // the visible water shimmer: a sun-coloured glint scaled by the Ward spec × (1-specMap)
    // (glossiness = (spec/9)*(1-vSpecMap) [pdxwater:322]); add it to the water base.
    let glint = clamp(spec / 9.0, 0.0, 4.0) * (1.0 - v_spec);
    // constant reflection sheen toward the +Y sky at fresnel≈0.5 (Codex): a gentle lift.
    var out = color + WATER_SKY * glint * 0.5;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // 1) clip to the destination rect.
    let rel = in.px - u.dst.xy;
    if (rel.x < 0.0 || rel.y < 0.0 || rel.x >= u.dst.z || rel.y >= u.dst.w) {
        discard;
    }
    // 2) destination UV -> map UV (pan/zoom).
    let out_uv = vec2<f32>(rel.x / u.dst.z, rel.y / u.dst.w);
    let map_uv = out_uv / u.scale + u.offset;
    if (map_uv.x < 0.0 || map_uv.y < 0.0 || map_uv.x >= 1.0 || map_uv.y >= 1.0) {
        return NEUTRAL_TERRAIN;
    }

    let coord = id_coord(map_uv);
    let pid = load_pid(coord);

    // --- mode tint (the DATA layer; alpha = validity flag) -----------------------
    let lut_w = textureDimensions(lut).x;
    var tint = NEUTRAL_TERRAIN;
    var has_tint = false;
    if (pid < lut_w) {
        tint = textureLoad(lut, vec2<i32>(i32(pid), 0), 0);
        has_tint = tint.a > 0.5; // alpha 255 = real tinted land; 0 = sea/no-data
    }

    // Fidelity OFF -> the original flat opaque behaviour (BEFORE image).
    if (u.flags.x < 0.5) {
        if (has_tint) {
            return vec4<f32>(tint.rgb, 1.0);
        }
        return NEUTRAL_TERRAIN;
    }

    // --- terrain underlay (game PixelShaderTerrain output, stored sRGB) -----------
    let tdim = textureDimensions(terrain_base);
    let tcoord = clamp(coord, vec2<i32>(0, 0), vec2<i32>(i32(tdim.x) - 1, i32(tdim.y) - 1));
    let base_srgb = textureLoad(terrain_base, tcoord, 0).rgb;
    var base_lin = to_linear3(base_srgb);

    // --- DYNAMIC terrain branches (snow / mud / ice) applied to the terrain BASE, the way
    // the game's ApplySnow / GetMudColor / ApplyIce modify diffuse before the mapmode tint.
    // Driven by the per-province dynamic LUT (winter/mud/frozen) the scrubber uploads.
    let dyn_w = textureDimensions(dynamic_lut).x;
    var winter = 0.0;
    var mud = 0.0;
    var frozen = 0.0;
    if (pid < dyn_w) {
        let dv = textureLoad(dynamic_lut, vec2<i32>(i32(pid), 0), 0);
        // dv already holds the RESOLVED SnowMud crossfade lerp(old,current,SnowMudFade) (done
        // CPU-side in pack_province_dynamic_lut, = the engine's resolved SnowMudData texture).
        winter = dv.r;
        mud = dv.g;
        frozen = dv.b;
    }
    // PER-PIXEL MATERIAL inputs: world position (xz from map_uv, y from the gate height) and
    // the gate's normal.y. The gate texture is the raster-resolution height/normal/land map.
    let gate = textureLoad(gate_tex, clamp(coord, vec2<i32>(0, 0), vec2<i32>(i32(textureDimensions(gate_tex).x) - 1, i32(textureDimensions(gate_tex).y) - 1)), 0);
    let world_y = gate.r * GATE_HEIGHT_RANGE;        // recovered world height
    let world_normal_y = gate.g * 2.0 - 1.0;          // normal.y in [-1,1]
    // world xz = map_uv * map_dims (the game's prepos.xz in map-pixel space).
    let world_pos = vec3<f32>(map_uv.x * u.map_dims.x, world_y, map_uv.y * u.map_dims.y);

    // SNOW: the REAL ApplySnow material (snow_tex.a · SNOW_COLOR), height/normal/noise gated,
    // camera-opacity ramped. LAND only. [standardfuncsgfx.fxh:264-301]
    if (has_tint && winter > 0.001) {
        base_lin = apply_snow(base_lin, world_pos, world_normal_y, clamp(winter, 0.0, 1.0), u.flags.w);
    }
    // MUD: the REAL GetMudColor material overlay. LAND only. [standardfuncsgfx.fxh:237-256]
    if (has_tint && mud > 0.001) {
        base_lin = apply_mud(base_lin, world_pos, clamp(mud, 0.0, 1.0), base_lin, u.flags.w);
    }
    // WATER SURFACE: the animated LEAN wave shimmer + spec-map + sun-spec glint on SEA pixels
    // (the surviving visible water animation at top-down view). Driven by global_time. Applied
    // BEFORE ice (frozen water is covered by the ice tint). [pdxwater.shader:287-322]
    if (!has_tint && frozen <= 0.5) {
        base_lin = apply_water_surface(base_lin, map_uv, world_pos, dyn_u.sun_dir.xyz, dyn_u.state.z);
    }
    // ICE: the REAL ApplyIce material on frozen sea/lake. SEA only. [pdxwater:231-253]
    if (!has_tint && frozen > 0.5) {
        base_lin = apply_ice(base_lin, world_pos, map_uv.y, clamp(frozen, 0.0, 1.0), u.flags.w);
    }

    // --- POLITICAL country colour: lerp(terrain, country, 1 - TERRAIN_TRANSPARENCY) -
    // Done in LINEAR (the engine builds the mapmode colour in linear). Only real land.
    var col = base_lin;
    if (has_tint) {
        let country_lin = to_linear3(tint.rgb);
        col = mix(base_lin, country_lin, u.flags.y); // strength = 0.2 (game)
    }
    // sea / no-data: terrain (water) shows through untinted (col already = base_lin).

    // --- rivers: the real RiverSurface colour over the terrain, with an animated FLOW
    // shimmer. The game animates the river surface waves by vTimeDirectionSeasonLerp.x
    // (= global time) via SampleWater [river.shader:309]; at political-map scale a river is a
    // thin line, so the faithful reduction is a brightness shimmer scrolling ALONG the river
    // by global_time (dyn_u.state.z), giving the sense of flowing water.
    let rdim = textureDimensions(river_mask);
    let rcoord = clamp(coord, vec2<i32>(0, 0), vec2<i32>(i32(rdim.x) - 1, i32(rdim.y) - 1));
    let river = textureLoad(river_mask, rcoord, 0);
    let here_owner = owner_of_pid(pid);
    if (river.a > 0.5) {
        // The REAL RiverSurface diffuse colour as a near-static line. (Codex-confirmed: at the
        // top-down political/strategic view a river is a 1-2px line, so the game's animated
        // lean-wave / specular surface collapses to sub-pixel variation and is NOT a readable
        // animated surface — the faithful political-map result is the static tinted river line.
        // The lean1/lean2 + Ward specular only matter at close 3D zoom. [river.shader:277-398])
        col = mix(col, to_linear3(river.rgb), 0.75);
    }

    // --- NAVAL DOMINANCE (dominance_fx_apply): contested SEA provinces get the friendly/enemy
    // overlay. dominance −1..1 is in the dynamic LUT alpha (128 = none). The faithful base
    // overlay [standardfuncsgfx.fxh:1020,1061] is:
    //   OverlayColor = abs(Control) * lerp(EnemyColor, FriendlyColor, (Control+1)/2)
    //   applied at Opacity = lerp(0.2, 0.23, (sin(vGlobalTime*1.5)+1)/2)  (× OuterFade).
    // (Codex FIX-4: the 0.8..1.5 ContestedIntensity pulse is ONLY for contested-region
    // blending, NOT the base overlay — so it is NOT applied here.) Colours are the REAL
    // naval_dominance_fx.dds px0 (enemy) / px1 (friendly).
    // Guard: only when the REAL per-province dynamic LUT is bound (width > 1; the 1×1
    // placeholder has alpha 0 which would falsely read as full enemy dominance everywhere).
    if (!has_tint && dyn_w > 1u && pid < dyn_w) {
        let dom_raw = textureLoad(dynamic_lut, vec2<i32>(i32(pid), 0), 0).a;
        let control = dom_raw * 2.0 - 1.0; // −1 enemy .. +1 friendly
        if (abs(control) > 0.04) {
            let t = dyn_u.state.z;
            let opacity = mix(0.2, 0.23, (sin(t * 1.5) + 1.0) * 0.5);
            // base overlay = |Control| · lerp(enemy, friendly, (Control+1)/2); applied at Opacity.
            let dom_color = abs(control) * mix(dyn_u.dom_enemy.rgb, dyn_u.dom_friendly.rgb, (control + 1.0) * 0.5);
            col = mix(col, dom_color, clamp(opacity, 0.0, 0.6));
        }
    }

    // --- city emissive (night): the REAL CityLights texture. vOut += CityLights *
    // CITY_LIGHTS_INTENSITY * CityLightsMask * nightFactor, where CityLights =
    // CityLightsAndSnowNoise.rgb (the citylights_rgb_snowmask_a texture) at prepos.xz *
    // CITY_LIGHTS_TILING, and CityLightsMask = colormap alpha. [pdxmap.shader:384-385]
    let edim = textureDimensions(emissive_mask);
    let ecoord = clamp(coord, vec2<i32>(0, 0), vec2<i32>(i32(edim.x) - 1, i32(edim.y) - 1));
    let city_mask = textureLoad(emissive_mask, ecoord, 0).r;
    // PER-PIXEL day/night: the terminator sweeps across the world by longitude at this hour,
    // so part of the map is day and part night (the faithful DayNightFactor, not a global one).
    // A zero sun vector = day/night DISABLED (the static/neutral default → full day).
    var night_factor = 0.0;
    if (dot(dyn_u.hour_sun.yzw, dyn_u.hour_sun.yzw) > 0.0001) {
        let world_xz = vec2<f32>(map_uv.x * u.map_dims.x, map_uv.y * u.map_dims.y);
        let globe_n = calc_globe_normal(world_xz, u.map_dims, dyn_u.hour_sun.x);
        night_factor = day_night_factor(globe_n, dyn_u.hour_sun.yzw);
    }
    if (night_factor > 0.001 && city_mask > 0.001) {
        let city_xz = vec2<f32>(map_uv.x * u.map_dims.x, map_uv.y * u.map_dims.y);
        let city_lights = textureSampleLevel(city_snow_noise, mat_samp, city_xz * CITY_LIGHTS_TILING, 0.0).rgb;
        col += city_lights * CITY_LIGHTS_INTENSITY * city_mask * night_factor;
    }

    // --- occupation stripes (secondary_color_mask): striped political borders over land
    // controlled by a country other than its owner. [pdxmap:328; standardfuncsgfx:979-987]
    // secondary_lut is province-indexed: rgb = controller colour, a = 1 where occupied.
    let sec_w = textureDimensions(secondary_lut).x;
    if (has_tint && pid < sec_w) {
        let sec = textureLoad(secondary_lut, vec2<i32>(i32(pid), 0), 0);
        if (sec.a > 0.5) {
            let occ = occupation_mask(map_uv, u.flags.w) * sec.a;
            col = mix(col, to_linear3(sec.rgb), occ); // game: lerp(color, mask.rgb, occMask)
        }
    }

    // --- FAITHFUL gradient borders (gradient_border_apply, ported) ----------------
    // Replaces the old 4-neighbour owner-edge darkening with the engine's anti-aliased
    // GradientBorderChannel ramp. border_field is a RASTER-resolution SDF whose channels
    // hold the engine's GradientBorderChannel.a "height" (1 ON border, ramping to 0 over
    // the bake thickness): R=country, G=province, B=state. A=land mask.
    //
    // We run gradient_border_process (the ported gradient_border_process_channel) per
    // layer in the engine's order: COUNTRY (Ch1) FIRST, STATE (Ch2) second. The visible band
    // WIDTH scales with camera distance through vGBCamDist → gradient_border_distance_to_alpha
    // — that IS the THICKNESS_COUNTRY_LOW(5)..HIGH(25) ramp (00_graphics.lua:867,869): far
    // camera fills more of the baked 25px envelope, near camera narrows it to the thin line.
    // [standardfuncsgfx.fxh:886-928]
    // sample the SDF at the id-raster texel (the border field is raster-resolution; clamp
    // to ITS dims, not the terrain's, so the two never have to be the same size).
    let bdim = vec2<i32>(textureDimensions(border_field));
    let bcoord = clamp(coord, vec2<i32>(0, 0), bdim - vec2<i32>(1, 1));
    // raw SDF heights — already sub-pixel-smooth, no multisample needed (see note above /
    // Codex finding 3). .a = land mask. .r=country, .g=province, .b=state height.
    let bf = textureLoad(border_field, bcoord, 0);
    // engine vGBCamDist (0 near .. up to 0.8 far), from our flags.w (1 far .. 0 near).
    let gb_cam = gradient_border_camera_distance(u.flags.w);
    // border_coverage accumulates max(alpha1, alpha2) for the season-protect (vBloomAlpha,
    // finding 4) — declared here so it survives past the if-block.
    var border_coverage = 0.0;

    // The engine's gradient_border_apply processes COUNTRY (Ch1) FIRST, then STATE (Ch2)
    // with its strength reduced by (1 - alpha1*GB_STRENGTH_CH1*GB_FIRST_LAYER_PRIORITY)
    // [fxh:917-923] — so the country line WINS at a shared country+state edge and the two
    // layers don't double-stack. We follow that order. Province lines have NO dedicated GB
    // channel in the engine (province fade), so we lay them FIRST as the faint bottom layer
    // (under country/state) where they can't over-darken the higher layers.
    // Land gate = owned land (here_owner != 0u); the A channel no longer carries a land mask
    // (it is the coastal height now), so the land gate is purely the owner key.
    if (here_owner != 0u) {
        // The gradient-border COLOUR split (Codex finding 1 reconciliation, see
        // gradient_border_process):
        //   * OUTLINE target = the full country political colour `country_lin` so the dark
        //     border LINE keeps the engine's country-colour identity (`country*0.5`, finding
        //     B: the game has NO defined base country-border colour — the line IS the country's
        //     own GradientBorderChannel.rgb darkened by the *0.5 outline edge; the SELECTION
        //     border colours at 00_graphics.lua:799+ are hover/peace highlights, not this).
        //   * FILL target = `mix(col, country_lin, flags.y)` (Codex Candidate C): pulls the
        //     mid-fill region toward the country colour in the engine's DIRECTION (the engine
        //     fills toward country colour) but converges to a BOUNDED value at full fill
        //     (≈0.36 effective tint at flags.y=0.2) — so the interior gains a faithful slight
        //     country emphasis at borders WITHOUT the flood the full-country fill would cause
        //     over our deliberately low-tint (0.2) base.
        let country_lin = to_linear3(tint.rgb);

        // (0) PROVINCE — faint thin inner line, bottom layer. A country/state border is
        // ALWAYS also a province border (provinces are the atomic unit), so at those texels
        // bf.g is high too; drawing the province line there would pre-darken the base that
        // country/state then darken again. SUBSUME it: gate the province height by
        // (1 - max(country,state) height) so the province line only appears on INTERIOR
        // province edges (not under a country/state line). The country/state line is the
        // visible border there, exactly as the engine subsumes the province edge.
        let prov_h = bf.g * (1.0 - max(bf.r, bf.b));
        let prov_cam = gb_cam_dist_override(gb_cam, GB_CAM_OVERRIDE_STATE); // province ~ state detail band
        let pr = gradient_border_process(col, mix(col, country_lin, u.flags.y), country_lin, prov_h, prov_cam, GB_OUTLINE_CUTOFF_STATE, 0.6);
        // OUTER gradient_border_apply mix: lerp(base, vGradMix, opacity_ramp * Transp).
        // Transp (GradientBorderChannel2.g) is the per-pixel border transparency; our SDF has
        // no separate transparency texture → Transp = 1 where the border exists. The opacity
        // ramp GB_OPACITY_NEAR..FAR (1.0..0.85) by camera distance [fxh:915,919] applies.
        let prov_op = gb_opacity_ramp(gb_cam);
        col = mix(col, pr.grad_mix, clamp(prov_op, 0.0, 1.0));

        // (1) COUNTRY = Ch1 — vCamDistOverride=0 → the live camera ramp [00_graphics.lua:890].
        let country_cam = gb_cam_dist_override(gb_cam, GB_CAM_OVERRIDE_COUNTRY);
        let cr = gradient_border_process(col, mix(col, country_lin, u.flags.y), country_lin, bf.r, country_cam, GB_OUTLINE_CUTOFF_COUNTRY, 1.0);
        let country_op = gb_opacity_ramp(gb_cam);
        col = mix(col, cr.grad_mix, clamp(country_op, 0.0, 1.0));

        // (2) STATE = Ch2 — vCamDistOverride=0.4 [00_graphics.lua:891]; strength reduced where
        // the country layer is active: (1 - alpha1 * GB_FIRST_LAYER_PRIORITY) [fxh:923].
        let state_cam = gb_cam_dist_override(gb_cam, GB_CAM_OVERRIDE_STATE);
        let state_strength = 1.0 - cr.coverage * GB_FIRST_LAYER_PRIORITY;
        let sr = gradient_border_process(col, mix(col, country_lin, u.flags.y), country_lin, bf.b, state_cam, GB_OUTLINE_CUTOFF_STATE, state_strength);
        let state_op = gb_opacity_ramp(gb_cam);
        col = mix(col, sr.grad_mix, clamp(state_op, 0.0, 1.0));

        // vBloomAlpha = 1 - max(vAlpha1, vAlpha2) [fxh:927]; we carry the COVERAGE (= 1 -
        // vBloomAlpha) to the season postprocess to protect border pixels (finding 4).
        border_coverage = max(cr.coverage, sr.coverage * state_strength);
    } else if (!has_tint && bf.a > 0.003) {
        // SEA-SIDE COASTAL border (item A): the engine runs gradient_border_apply in the WATER
        // shader too [pdxwater.shader:351-355], so coastlines carry a border line on the SEA
        // side with GB_OUTLINE_CUTOFF_SEA=0.990 [constants.fxh:201]. `bf.a` is the coastal SDF
        // height (1 ON the coast, ramping to 0 out to sea). The line colour is the engine's
        // coastal gradient — a dark coastline against the water; we use the local water `col`
        // darkened via the same outline `*0.5`, processed at the SEA cutoff so it is a hairline.
        let coast_dark = col * 0.5; // dark coastline (the outline edge darken reads dark on water)
        let coast_cam = gb_cam_dist_override(gb_cam, GB_CAM_OVERRIDE_STATE);
        let cor = gradient_border_process(col, col, coast_dark, bf.a, coast_cam, GB_OUTLINE_CUTOFF_SEA, 1.0);
        let coast_op = gb_opacity_ramp(gb_cam);
        col = mix(col, cor.grad_mix, clamp(coast_op, 0.0, 1.0));
        border_coverage = cor.coverage;
    }

    // --- SEASONAL colour-balance: per-channel multiply by the latitude-banded season
    // balance (seasons.txt colorbalance, north..center..south by map-V). [seasons.txt:63-123]
    // --- DISTANCE FOG (ApplyDistanceFog): CalculateDistanceFogFactor =
    // saturate(min((dist²-begin²)/(end²-begin²), FOG_MAX)) · (1 - |viewDir.y|). For a TRUE
    // TOP-DOWN political camera viewDir = (0,-1,0), so |viewDir.y| = 1 and the (1 - |viewDir.y|)
    // angle term is EXACTLY 0 → distance fog is ZERO at the strategic political view. The game
    // shows distance fog only on the OBLIQUE in-game camera, never the flat political map.
    // (Codex-confirmed: the exact formula yields ~0 straight down.) So we apply NO distance
    // fog here — that IS the faithful result, not a drop. [standardfuncsgfx.fxh:203-218; pdxmap:392]
    let fog_factor = 0.0;
    col = mix(col, FOG_COLOR, fog_factor);

    // --- BORDER LIGHT REMOVAL (vBloomAlpha): the game flattens the lighting toward the raw
    // diffuse on border pixels: vOut = lerp(vOut, diffuse, BORDER_LIGHT_REMOVAL_FACTOR=0.8 *
    // (1 - vBloomAlpha)), where vBloomAlpha = 1 - border_coverage. [pdxmap.shader:378]. Our
    // `col` already IS the (un-lit-flattened) terrain diffuse composite, so this is a gentle
    // pull toward the un-tinted terrain base on border pixels — keeping borders crisp under the
    // tint. We lerp toward the political-pre-tint terrain (base_lin) by that factor.
    let v_bloom_alpha = 1.0 - clamp(border_coverage, 0.0, 1.0);
    col = mix(col, base_lin, BORDER_LIGHT_REMOVAL_FACTOR * (1.0 - v_bloom_alpha));

    // --- DAY/NIGHT per-pixel tone shift — the FAITHFUL DayNightWithBlend (NightifyColor:
    // grey-weights + blue-night + cubic desaturation × NIGHT_DARKNESS). The blend factor is
    // vBloomAlpha-gated EXACTLY like the game: lerp(BORDER_NIGHT_DESATURATION_MAX=0.2, 1.0,
    // vBloomAlpha) — so border pixels desaturate LESS at night (stay readable).
    // [pdxmap:395; standardfuncsgfx.fxh:384-393]
    let night_blend = mix(BORDER_NIGHT_DESATURATION_MAX, 1.0, v_bloom_alpha);
    col = day_night_with_blend(col, night_factor, night_blend);

    // --- FINAL POSTPROCESS = the game's HDR tonemap + restorescene.shader RestoreScene, the
    // FAITHFUL full chain (Codex-verified): fixed exposure (1.0; the runtime auto-exposure
    // settles to mid-grey for a static map) → Uncharted2 tonemap (which gamma-encodes via
    // pow(1/2.2)) → SampleColorCube (the real colorcorrection.tga 32^3 LUT) → season HSV
    // (RGB→HSV, HSV.yz *= season.yz, HSV.x += season.x mod 6, HSVtoRGBPost) → *= ColorBalance
    // → Levels(0.04, 0.8). [restorescene.shader:112-223]
    // `col` is still LINEAR here (day/night + fog already applied in linear, as the game does
    // in pdxmap BEFORE the restorescene postprocess).
    var disp = tonemap_uncharted(col);   // exposure 1.0 folded in; this IS the gamma encode
    disp = saturate3(disp);
    disp = sample_color_cube(disp);      // the graded ColorCube LUT (required for parity)

    // season latitude-banded HSV + ColorBalance (RestoreScene COLOR_LUT branch).
    let v = map_uv.y;
    var cb = dyn_u.cb_center.rgb;
    var season_s = dyn_u.cb_center.w;
    if (v < 0.5) {
        let t = v / 0.5;
        cb = mix(dyn_u.cb_north.rgb, dyn_u.cb_center.rgb, t);
        season_s = mix(dyn_u.cb_north.w, dyn_u.cb_center.w, t);
    } else {
        let t = (v - 0.5) / 0.5;
        cb = mix(dyn_u.cb_center.rgb, dyn_u.cb_south.rgb, t);
        season_s = mix(dyn_u.cb_center.w, dyn_u.cb_south.w, t);
    }
    // vBloomAlpha border-protect (finding 4): the engine returns vBloomAlpha = 1 -
    // max(alpha1,alpha2) [fxh:927] "so the postprocess will ignore gradient borders when
    // applying season coloring overlay". So the SEASON colour overlay (HSV-sat + ColorBalance)
    // must NOT recolour border pixels. We compute the season-graded result, then lerp it back
    // toward the UN-season-graded `disp` by `border_coverage` (= 1 - vBloomAlpha). The
    // ColorCube + Levels tone curve are the scene-wide HDR grade the engine applies to
    // everything (borders included) — only the SEASON overlay is guarded by vBloomAlpha.
    let pre_season = disp;
    // HSV season adjust — the EXACT RestoreScene path [restorescene.shader:116-119]:
    // RGBtoHSV, HSV.y *= season S (and .z *= V=1, .x += H=0 in vanilla), HSVtoRGBPost. The
    // engine's HSVtoRGBPost is its own (lossy) "Photoshop filter" reconstruction — we use it
    // verbatim. When the season is exactly neutral (S == 1) the adjust is the identity, so we
    // skip the round-trip (avoids any reconstruction drift on the bulk of the map); the
    // non-neutral bands (e.g. vanilla north S=0.1 [seasons.txt:67]) run the exact HSV path.
    if (abs(season_s - 1.0) >= 0.001) {
        var hsv = rgb_to_hsv(disp);
        hsv.y = hsv.y * season_s;
        disp = hsv_to_rgb_post(hsv);
    }
    disp = saturate3(disp * cb);          // *= ColorBalance
    // Protect borders from the SEASON overlay: where border_coverage is high, keep the
    // pre-season grade (vBloomAlpha = 1 - border_coverage). [fxh:927]
    disp = mix(disp, pre_season, clamp(border_coverage, 0.0, 1.0));
    disp = levels3(disp, 0.04, 0.8);      // the fixed RestoreScene Levels tone curve

    return vec4<f32>(disp, 1.0);
}

fn saturate3(c: vec3<f32>) -> vec3<f32> { return clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)); }
