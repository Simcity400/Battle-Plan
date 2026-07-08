//! GPU map renderer (MAP_REPLICA_PLAN Phase 2 architecture).
//!
//! Big immutable textures uploaded exactly once: province ids as
//! `R32_UINT` (integer `Load`, never filtered) and the grounded
//! `map_visual.rgbx` underlay as RGBA8. Recoloring is a GPU lookup chain
//! province -> state -> owner color plus runtime gradient-border channels.
//! Scrubbing rewrites the compact state table from controllers and rebuilds
//! generated runtime fields only when their political signatures change.

use std::collections::BTreeMap;
use std::ffi::CString;
use std::fs;
use std::mem;
use std::path::{Path, PathBuf};
use std::ptr;
use std::time::{Duration, Instant};

use map_viewer::{
    hoi4_map_camera_defines_from_bundle, load_restore_scene_bundle_session_dir, map_wrap_offsets,
    resolve_bloom_params_for_volume, scene_average_luminance, seasons_table_from_source,
    tree_file_derived_layer_from_resolver, tree_season_cb_values, tree_second_subdraw_verdict,
    AverageLuminanceReduction, BloomBlurKernel, BloomParams, BuildingFileDerivedLayerStats,
    GameFileResolver, Hoi4MapCameraDefines, LuminanceClampBounds, MapLayerSynthesisInputs,
    NativeD3d11PreparedProductionMap, NativeD3d11PreparedSynthesizedProductionMap,
    NativeD3d11RenderOptions, NativeD3d11TerrainBundle, NativeD3d11TerrainChunkLibrary,
    NativeD3d11TerrainPoliticalSrvSubstitution, NativeD3d11TerrainRenderer, ProductionMapCamera,
    ProductionMapCameraProjection, SeasonsTable, RESTORE_PASS_BUNDLE_FILE,
};
use winapi::shared::dxgiformat::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_FORMAT_R32_UINT,
    DXGI_FORMAT_R8G8B8A8_UNORM,
};
use winapi::shared::dxgitype::DXGI_SAMPLE_DESC;
use winapi::shared::minwindef::UINT;
use winapi::um::d3d11::{
    ID3D11Buffer, ID3D11PixelShader, ID3D11RenderTargetView, ID3D11SamplerState,
    ID3D11ShaderResourceView, ID3D11Texture2D, ID3D11VertexShader, D3D11_BIND_CONSTANT_BUFFER,
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_BUFFER_DESC, D3D11_BUFFER_SRV,
    D3D11_COMPARISON_ALWAYS, D3D11_CPU_ACCESS_READ, D3D11_CPU_ACCESS_WRITE,
    D3D11_FILTER_MIN_MAG_MIP_LINEAR, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ,
    D3D11_MAP_WRITE_DISCARD, D3D11_SAMPLER_DESC, D3D11_SHADER_RESOURCE_VIEW_DESC,
    D3D11_SUBRESOURCE_DATA, D3D11_TEX2D_SRV, D3D11_TEXTURE2D_DESC, D3D11_TEXTURE_ADDRESS_CLAMP,
    D3D11_USAGE_DEFAULT, D3D11_USAGE_DYNAMIC, D3D11_USAGE_IMMUTABLE, D3D11_USAGE_STAGING,
    D3D11_VIEWPORT,
};
use winapi::um::d3dcommon::{
    ID3DBlob, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, D3D11_SRV_DIMENSION_BUFFER,
    D3D11_SRV_DIMENSION_TEXTURE2D,
};
use winapi::um::d3dcompiler::D3DCompile;

use crate::buildings::BuildingPreviewLayer;
use crate::bundle::{MapBundle, PRODUCT_MAP_WIDTH};
use crate::camera::Camera;
use crate::d3d::{hr_ok, ComPtr, Gfx};
use crate::gb_field::GbField;
use crate::grading_law::{
    file_derived_grade, load_graphics_grading_defines, GradingSelectionState,
};
use crate::secondary_field::SecondaryColorField;
use crate::Result;

/// Full-screen-triangle map shader. The pixel shader maps each screen
/// pixel to a map pixel from the camera constants, samples the grounded
/// `map_visual` underlay, then applies the game's active gradient-border shader
/// path using generated GradientBorderChannel1/2 textures.
const MAP_SHADER_HLSL: &str = r#"
#ifndef REPLAY_POSTFX_ENABLED
#define REPLAY_POSTFX_ENABLED 1
#endif

cbuffer CameraCB : register(b0) {
    float2 screen_size;
    float2 center_map;
    float zoom;
    float map_w;
    float map_h;
    float camera_height;
    // GRADIENT_BORDERS_OUTLINE_CUTOFF_{STRATEGIC_REGIONS,COUNTRY} from the
    // mod-aware resolver read of common/defines/00_graphics.lua:880-885
    // (vanilla 0.98 / 0.973). Fed here so Path B matches the engine + Path A
    // scene-CB0 write; see GB_OUTLINE_CUTOFF below for the channel mapping.
    float2 gb_outline_cutoff;
    float2 camera_cb_pad;
};

Texture2D<uint> ProvinceIds : register(t0);
Texture2D<float4> MapVisual : register(t1);
Buffer<uint> ProvinceToState : register(t2);
Buffer<float4> StateColors : register(t3);
Texture2D<float4> GradientBorderChannel1 : register(t4);
Texture2D<float4> GradientBorderChannel2 : register(t5);
Texture2D<float4> ProvinceSecondaryColorMap : register(t6);
SamplerState LinearClampSampler : register(s0);

// gfx/FX/constants.fxh:195-209, active "GRADIENT BORDERS" block.
static const float GB_CAM_MIN = 100.0f;
static const float GB_CAM_MAX = 350.0f;
static const float GB_CAM_MAX_FILLING_CLAMP = 0.8f;
static const float GB_THRESHOLD = 0.05f;
static const float GB_THRESHOLD2 = 0.25f;
static const float GB_OPACITY_NEAR = 1.0f;
static const float GB_OPACITY_FAR = 0.85f;
static const float GB_STRENGTH_CH1 = 1.0f;
static const float GB_STRENGTH_CH2 = 1.0f;
static const float GB_FIRST_LAYER_PRIORITY = 0.4f;
static const float BORDER_NIGHT_DESATURATION_MAX = 0.2f;
static const float BORDER_FOW_REMOVAL_FACTOR = 0.8f;
static const float BORDER_LIGHT_REMOVAL_FACTOR = 0.8f;

// GAP: replay_app has no live night/FOW/light map inputs yet. Those
// constants are transcribed from constants.fxh:201-203; the surrounding
// pdxmap lighting/FOW effects remain neutral in this gradient-border port.

// Captured 2026-07-02 pdxmap dx11_cb0 evidence:
// vGBCamDistOverride_GBOutlineCutoff at offset 128 = (0,0,0.9999,0.9999);
// shader_meta.rs records that field at offset 128 and GB_TextureHeight at 400.
static const float2 GB_CAM_DIST_OVERRIDE = float2(0.0f, 0.0f);
// GB outline cutoffs come from the CameraCB (resolver-read shipped defines),
// NOT a hardcode. The engine calls
//   gradient_border_apply(..., vOutlineCutoff = ...OutlineCutoff.zw, ...)
// (pdxmap.shader:325), where CB0 field .z = STRATEGIC, .w = COUNTRY
// (shader_meta.rs write_political_scene_fields). Inside gradient_border_apply
// the FIRST (top-half / strategic) channel uses vOutlineCutoff.x and the
// SECOND (bottom-half / country) channel uses vOutlineCutoff.y
// (standardfuncsgfx.fxh:886-928). So .x = strategic (0.98), .y = country
// (0.973): CameraCB.gb_outline_cutoff is packed (strategic, country) to match.
#define GB_OUTLINE_CUTOFF gb_outline_cutoff
static const float GB_TEXTURE_HEIGHT = 1024.0f;

// gfx/FX/constants.fxh:216.
static const float SEC_MAP_TILE = 6000.0f;

// GAP: this fallback shader has no captured Clausewitz time input; the active
// shader uses vGlobalTime only for the FX strength pulse in
// standardfuncsgfx.fxh:844-846.
static const float vGlobalTime = 0.0f;

struct VsOut {
    float4 pos : SV_Position;
};

VsOut vs_main(uint vertex_id : SV_VertexID) {
    // Full-screen triangle from the vertex id; no vertex buffer.
    VsOut output;
    float2 clip = float2(
        (vertex_id == 1) ? 3.0 : -1.0,
        (vertex_id == 2) ? -3.0 : 1.0);
    output.pos = float4(clip, 0.0, 1.0);
    return output;
}

// standardfuncsgfx.fxh:179-181.
float Levels(float value, float min_value, float max_value) {
    return saturate((value - min_value) / (max_value - min_value));
}

// standardfuncsgfx.fxh:184-186, with replay_app camera_height supplied from
// the map-pixel camera via the map_viewer zoom->height grounding.
float cam_distance(float min_value, float max_value) {
    return (clamp(camera_height, min_value, max_value) - min_value) / (max_value - min_value);
}

// standardfuncsgfx.fxh:965-976.
float CalculateOccupationMask(float2 uv) {
    float t = 3.14159f / 8.0f;
    float w = SEC_MAP_TILE;
    float stripeVal = cos((uv.x * cos(t) * w) + (uv.y * sin(t) * w));
    float camDist = cam_distance(300.0f, 1200.0f);
    stripeVal += camDist * 1.5f;
    stripeVal = smoothstep(0.0f, 1.0f, stripeVal * 1.7f) * lerp(1.0f, 0.3f, camDist);
    return stripeVal;
}

// standardfuncsgfx.fxh:979-987. The `normal` parameter is unused by the game
// function, so replay_app passes a neutral normal and ports the sampled RGBA
// blend exactly.
void secondary_color_mask(inout float3 color, float3 normal, float2 uv, inout float bloom_alpha) {
    float4 color_mask = ProvinceSecondaryColorMap.SampleLevel(LinearClampSampler, uv, 0.0f).rgba;
    float occupation_mask = CalculateOccupationMask(uv);
    occupation_mask *= color_mask.a;
    bloom_alpha = bloom_alpha * (1.0f - occupation_mask);
    color = lerp(color, color_mask.rgb, occupation_mask);
}

// standardfuncsgfx.fxh:813-815.
float gradient_border_camera_distance() {
    return 1.0f - clamp(cam_distance(GB_CAM_MIN, GB_CAM_MAX), 0.0f, GB_CAM_MAX_FILLING_CLAMP);
}

// standardfuncsgfx.fxh:818-822.
float gradient_border_distance_to_alpha(float dist_value, float cam_dist) {
    dist_value = 1.0f - dist_value;
    dist_value *= (cam_dist - GB_THRESHOLD) / GB_THRESHOLD2;
    return 1.0f - saturate(dist_value);
}

float4 sample_gb1(float2 uv) {
    return GradientBorderChannel1.SampleLevel(LinearClampSampler, uv, 0.0f);
}

float4 sample_gb2(float2 uv) {
    return GradientBorderChannel2.SampleLevel(LinearClampSampler, uv, 0.0f);
}

// standardfuncsgfx.fxh:791-808, active non-LOW_END_GFX branch.
float4 gradient_border_multisample_alpha(float4 ch, float2 uv) {
    float offset_x = -0.5f / map_w;
    float offset_y = -0.5f / map_h;
    float4 result = ch;
    result += sample_gb1(uv + float2(-offset_x, 0.0f));
    result += sample_gb1(uv + float2(0.0f, -offset_y));
    result += sample_gb1(uv + float2(offset_x, 0.0f));
    result += sample_gb1(uv + float2(0.0f, offset_y));
    result += sample_gb1(uv + float2(-offset_x, -offset_y));
    result += sample_gb1(uv + float2(offset_x, -offset_y));
    result += sample_gb1(uv + float2(offset_x, offset_y));
    result += sample_gb1(uv + float2(-offset_x, offset_y));
    result /= 9.0f;
    return result;
}

// standardfuncsgfx.fxh:840-884, active implementation. The older commented
// variant at standardfuncsgfx.fxh:930-963 is intentionally not ported.
float gradient_border_process_channel(
    out float3 channel_color,
    float3 initial_color,
    float cam_dist,
    float2 uv,
    float outline_mult,
    float outline_cutoff,
    float strength)
{
    channel_color = initial_color;
    const float PulseSpeedMult = 3.5f;
    float2 FX_Alpha = sample_gb2(uv).bg;
    strength *= lerp(lerp(0.45f, 1.0f, 1.0f - FX_Alpha.r), 1.0f, (sin(vGlobalTime * PulseSpeedMult) + 1.0f) / 2.0f);

    float full_width = 5.25f / 255.0f;
    float gradient_width = 0.5f / 255.0f;
    float4 gb_dist = gradient_border_multisample_alpha(sample_gb1(uv), uv);
    float alpha = gb_dist.a;
    float color_opacity = Levels(alpha, 0.0f, outline_cutoff);
    float outline = 1.0f - Levels(alpha, outline_cutoff, 1.0f);
    float old_outline = outline;
    outline *= floor(color_opacity);
    outline *= outline_mult;
    color_opacity = gradient_border_distance_to_alpha(color_opacity, cam_dist);
    color_opacity *= floor(old_outline);
    float thick = smoothstep(0.0f, 1.0f, Levels(alpha, outline_cutoff - full_width, outline_cutoff - full_width + gradient_width));
    thick *= floor(old_outline);
    float max_gradient = max(color_opacity, outline);
    channel_color = lerp(channel_color, gb_dist.rgb, max(max_gradient, thick) * strength);
    channel_color = lerp(channel_color, channel_color * 0.5f, thick);
    return max(max_gradient, thick) * FX_Alpha.g;
}

// standardfuncsgfx.fxh:886-928, active implementation.
void gradient_border_apply(inout float3 color, float2 uv, inout float bloom_alpha) {
    float gb_cam_dist = gradient_border_camera_distance();
    float gb_cam_dist_ch1 = saturate((gb_cam_dist * int(1.0f - GB_CAM_DIST_OVERRIDE.x)) + GB_CAM_DIST_OVERRIDE.x);
    float gb_cam_dist_ch2 = saturate((gb_cam_dist * int(1.0f - GB_CAM_DIST_OVERRIDE.y)) + GB_CAM_DIST_OVERRIDE.y);

    float half_pix = 0.5f / GB_TEXTURE_HEIGHT;
    uv.y *= 0.5f - half_pix;
    float2 uv2 = float2(uv.x, uv.y + 0.5f);

    float3 grad_mix;
    float alpha1 = gradient_border_process_channel(grad_mix, color, gb_cam_dist_ch1, uv, 1.0f, GB_OUTLINE_CUTOFF.x, GB_STRENGTH_CH1);
    float transp_a = sample_gb2(uv).g;
    float opacity = GB_OPACITY_NEAR + (1.0f - gb_cam_dist) * (GB_OPACITY_FAR - GB_OPACITY_NEAR);
    color = lerp(color, grad_mix, opacity * transp_a);

    float alpha2 = gradient_border_process_channel(
        grad_mix,
        color,
        gb_cam_dist_ch2,
        uv2,
        1.0f,
        GB_OUTLINE_CUTOFF.y,
        (1.0f - alpha1 * GB_STRENGTH_CH1 * GB_FIRST_LAYER_PRIORITY) * GB_STRENGTH_CH2);
    float transp_b = sample_gb2(uv2).g;
    color = lerp(color, grad_mix, opacity * transp_b);
    bloom_alpha = 1.0f - max(alpha1, alpha2);
}

// gfx/FX/restorescene.shader:155-164.
float3 uncharted2_tonemap(float3 x) {
    float A = 0.22f;
    float B = 0.30f;
    float C = 0.10f;
    float D = 0.20f;
    float E = 0.01f;
    float F = 0.30f;
    return ((x * (A * x + C * B) + D * E) / (x * (A * x + B) + D * F)) - E / F;
}

// Ported scene restore tone curve:
// - restorescene.shader:194-202 defines the active UNCHARTED tone map with
//   ExposureBias = 1.0, W = 11.2, whiteScale, and pow(color, 1/2.2).
// - restorescene.shader:223 saturates before RestoreScene.
//
// GAME POST GAPS: restorescene.shader:137-139 samples AverageLuminanceTexture
// for adjusted exposure; replay_app has no runtime adaptation texture, so the
// Exposure() multiplier is not guessed. restorescene.shader:210-213 composes a
// RestoreBloom texture produced by bloom.shader:94-101/109-116; replay_app has
// no bright-pass/blur buffer, so bloom is skipped. restorescene.shader:114-123
// uses COLOR_LUT/ColorCube plus HSV/ColorBalance, while posteffect_volumes.txt:
// 21-28 shows volume-dependent shipped LUT selection; the active runtime volume
// is not captured here, so LUT/color-grading is a named gap rather than a
// guessed texture.
float3 hoi4_scene_restore_tone_map(float3 inColor) {
    float ExposureBias = 1.0f;
    float W = 11.2f;
    float3 curr = uncharted2_tonemap(ExposureBias * inColor);
    float3 whiteScale = 1.0f / uncharted2_tonemap(float3(W, W, W));
    float3 color = curr * whiteScale;
    color = pow(max(color, 0.0f), float3(1.0f / 2.2f, 1.0f / 2.2f, 1.0f / 2.2f));
    return saturate(color);
}

float4 ps_main(float4 pos : SV_Position) : SV_Target {
    // SV_Position.xy is the pixel center; invert the camera transform.
    float2 map_pos = center_map + (pos.xy - screen_size * 0.5) / zoom;
    if (map_pos.y < 0.0 || map_pos.y >= map_h) {
        // PROXY: letterbox color outside the vertical map bounds is not
        // game-file-derived. Real parity requires the game camera pan/zoom
        // limits from common/defines so off-map space is never visible.
        return float4(0.0, 0.0, 0.0, 1.0);
    }
    // Horizontal wrap: the game map wraps east-west.
    float wrapped_x = fmod(map_pos.x, map_w);
    if (wrapped_x < 0.0) {
        wrapped_x += map_w;
    }
    int2 texel = int2(
        clamp(int(wrapped_x), 0, int(map_w) - 1),
        clamp(int(map_pos.y), 0, int(map_h) - 1));
    // Sentinel: no recorded owner; generated channels have zero masks there,
    // so the grounded map_visual underlay remains unchanged.
    float4 underlay = MapVisual.Load(int3(texel, 0));
    float3 color = underlay.rgb;
    float2 uv = (float2(texel) + 0.5f) / float2(map_w, map_h);

    // `pdxmap.shader:323-328` applies gradient borders before the secondary
    // color mask.
    float bloom_alpha = 0.0f;
    gradient_border_apply(color, uv, bloom_alpha);
    secondary_color_mask(color, float3(0.0f, 0.0f, 1.0f), uv, bloom_alpha);
#if REPLAY_POSTFX_ENABLED
    color = hoi4_scene_restore_tone_map(color);
#endif
    return float4(color, 1.0);
}
"#;

const CAPTURED_PASS_COMPOSITE_HLSL: &str = r#"
#ifndef REPLAY_POSTFX_ENABLED
#define REPLAY_POSTFX_ENABLED 1
#endif
#ifndef REPLAY_BLOOM_ENABLED
#define REPLAY_BLOOM_ENABLED 0
#endif

Texture2D<float4> CapturedMapColor : register(t0);
SamplerState LinearClampSampler : register(s0);

// restorescene.shader:20-28 RestoreBloom sampler (Linear/Clamp). Files-first:
// this is the CPU-mirrored bloom chain's output computed from the replay's own
// rendered MainScene, never a captured texture. Bound only when bloom is on.
Texture2D<float4> RestoreBloom : register(t8);

// posteffect_base.fxh:8 BloomToScreenScale (captured [1,1]); half-res bloom
// still uses 0..1 UVs over its own extent, so this is [1,1] here.
static const float2 BloomToScreenScale = float2(1.0f, 1.0f);

struct VsOut {
    float4 pos : SV_Position;
    float2 uv : TEXCOORD0;
};

VsOut vs_main(uint vertex_id : SV_VertexID) {
    float2 clip = float2(
        (vertex_id == 1) ? 3.0 : -1.0,
        (vertex_id == 2) ? -3.0 : 1.0);
    VsOut output;
    output.pos = float4(clip, 0.0, 1.0);
    output.uv = float2((clip.x + 1.0) * 0.5, (1.0 - clip.y) * 0.5);
    return output;
}

float3 uncharted2_tonemap(float3 x) {
    float A = 0.22f;
    float B = 0.30f;
    float C = 0.10f;
    float D = 0.20f;
    float E = 0.01f;
    float F = 0.30f;
    return ((x * (A * x + C * B) + D * E) / (x * (A * x + B) + D * F)) - E / F;
}

float3 hoi4_scene_restore_tone_map(float3 inColor) {
    float W = 11.2f;
    float3 curr = uncharted2_tonemap(inColor);
    float3 whiteScale = 1.0f / uncharted2_tonemap(float3(W, W, W));
    float3 color = curr * whiteScale;
    color = pow(max(color, 0.0f), float3(1.0f / 2.2f, 1.0f / 2.2f, 1.0f / 2.2f));
    return saturate(color);
}

float4 ps_main(VsOut input) : SV_Target {
    float3 color = CapturedMapColor.SampleLevel(LinearClampSampler, input.uv, 0.0f).rgb;
#if REPLAY_BLOOM_ENABLED
    // restorescene.shader:210-213: bloom is ADDED to the linear MainScene
    // BEFORE Exposure/ToneMap/LUT. Additive, no extra scale (BLOOM_SCALE is
    // already baked into the blur), sampled at uv * BloomToScreenScale.
    float3 bloom = RestoreBloom.SampleLevel(LinearClampSampler, input.uv * BloomToScreenScale, 0.0f).rgb;
    color = bloom + color;
#endif
#if REPLAY_POSTFX_ENABLED
    color = hoi4_scene_restore_tone_map(color);
#endif
    return float4(color, 1.0f);
}
"#;

const RESTORE_FULL_POSTFX_HLSL: &str = r#"
Texture2D<float4> RestoreColorCube : register(t7);

cbuffer RestorePostfxCB : register(b1) {
    float4 RestoreHSV;
    float4 RestoreColorBalanceUseLut;
    float4 RestoreExposure;
};

float restore_shader_mod(float value, float modulus) {
    return value - modulus * floor(value / modulus);
}

float3 restore_hue_post(float H) {
    float X = 1.0f - abs(restore_shader_mod(H, 2.0f) - 1.0f);
    if (H < 1.0f) return float3(1.0f, X, 0.0f);
    else if (H < 2.0f) return float3(X, 1.0f, 0.0f);
    else if (H < 3.0f) return float3(0.0f, 1.0f, X);
    else if (H < 4.0f) return float3(0.0f, X, 1.0f);
    else if (H < 5.0f) return float3(X, 0.0f, 1.0f);
    else return float3(1.0f, 0.0f, X);
}

float3 restore_hsv_to_rgb_post(float3 hsv) {
    if (hsv.y != 0.0f) {
        float C = hsv.y * hsv.z;
        return clamp(restore_hue_post(hsv.x) * C + (hsv.z - C), 0.0f, 1.0f);
    }
    return saturate(hsv.zzz);
}

float3 restore_rgb_to_hsv(float3 RGB) {
    float Cmax = max(RGB.r, max(RGB.g, RGB.b));
    float Cmin = min(RGB.r, min(RGB.g, RGB.b));
    float diff = Cmax - Cmin;
    float H = 0.0f;
    float S = 0.0f;
    if (diff != 0.0f) {
        S = diff / Cmax;
        if (Cmax == RGB.r) H = (RGB.g - RGB.b) / diff + 6.0f;
        else if (Cmax == RGB.g) H = (RGB.b - RGB.r) / diff + 2.0f;
        else H = (RGB.r - RGB.g) / diff + 4.0f;
    }
    return float3(H, S, Cmax);
}

float3 restore_levels(float3 color, float min_input, float max_input) {
    float3 ret = saturate(color - min_input);
    ret /= max_input - min_input;
    return saturate(ret);
}

float3 restore_sample_color_cube(float3 color) {
    color = saturate(color);
    const float cube_size = 32.0f;
    const float scale = (cube_size - 1.0f) / cube_size;
    const float offset = 0.5f / cube_size;
    float x = (scale * color.r + offset) / cube_size;
    float y = scale * color.g + offset;
    float z_floor = floor((scale * color.b + offset) * cube_size);
    float x_offset1 = z_floor / cube_size;
    float x_offset2 = min(cube_size - 1.0f, z_floor + 1.0f) / cube_size;
    float3 color1 = RestoreColorCube.SampleLevel(LinearClampSampler, float2(x + x_offset1, y), 0.0f).rgb;
    float3 color2 = RestoreColorCube.SampleLevel(LinearClampSampler, float2(x + x_offset2, y), 0.0f).rgb;
    // The colorcorrection LUT (.tga) parser BGR->RGB-swaps each texel VALUE
    // (color_lut.rs: "TGA stores BGR(A)"), but the engine samples the cube in the
    // TGA's STORED byte order — so the graded output must be swapped back here.
    // Without this, the near-neutral LUT turns the engine's warm brown ocean
    // (R>G>B) into a cold punchy blue (B>R) = the user's "way too saturated".
    // Verified vs the engine's post-grade swapchain frame (res64, B8G8R8A8):
    // no-swap reproduces the engine final at mean|err| ~0.0095 (world) vs ~0.081
    // swapped. Render-path scoped (Option C) so the parser + capture-fit paired-
    // swap tests are untouched.
    return lerp(color1, color2, scale * color.b * cube_size - z_floor).bgr;
}

float3 hoi4_scene_restore_full(float3 inColor) {
    if (RestoreExposure.z > 0.5f) {
        inColor *= RestoreExposure.x / RestoreExposure.y;
    }
    float3 color = hoi4_scene_restore_tone_map(inColor);
    if (RestoreColorBalanceUseLut.w > 0.5f) {
        color = restore_sample_color_cube(color);
        float3 hsv = restore_rgb_to_hsv(color);
        hsv.yz *= RestoreHSV.yz;
        hsv.x += RestoreHSV.x;
        hsv.x = restore_shader_mod(hsv.x, 6.0f);
        color = restore_hsv_to_rgb_post(hsv);
        color = saturate(color * RestoreColorBalanceUseLut.xyz);
        color = restore_levels(color, 0.04f, 0.8f);
    }
    return color;
}
"#;

/// GPU port of `map_viewer::bloom` (bright pass + separable blur). Every stage
/// mirrors the CPU law, which itself is cited from the shipped shaders:
///   * bright pass / 2x box downsample + knee: `gfx/FX/downsample.shader:62-73`
///     (and `bloom.rs::bright_pass_downsample`).
///   * separable linear-sampled Gaussian: `gfx/FX/bloom.shader` PixelShader
///     (center tap + 4 symmetric tap pairs; `bloom.rs::blur_pass`).
/// The kernel weights/offsets and knee constants come from the files-first
/// resolver (`BloomBlurKernel::from_gaussian` + `BloomParams`), uploaded here.
const RESTORE_BLOOM_HLSL: &str = r#"
Texture2D<float4> BloomSource : register(t0);
SamplerState BloomSampler : register(s0);

// gfx/FX/downsample.shader knee constants for the bright pass.
cbuffer BloomBrightCB : register(b0) {
    // x = BRIGHT_THRESHOLD, y = EMISSIVE_BLOOM_STRENGTH, zw = InvSourceSize
    // (1 / MainScene size, for the 2x2 box tap centers).
    float4 BrightThresholdEmissiveInvSize;
};

// constants.fxh:119 LUMINANCE_VECTOR (log-luminance in the bright-pass alpha).
static const float3 BloomLuminanceVector = float3(0.2125f, 0.7154f, 0.0721f);

struct BloomVsOut {
    float4 pos : SV_Position;
    float2 uv : TEXCOORD0;
};

BloomVsOut bloom_vs(uint vertex_id : SV_VertexID) {
    float2 clip = float2(
        (vertex_id == 1) ? 3.0f : -1.0f,
        (vertex_id == 2) ? -3.0f : 1.0f);
    BloomVsOut output;
    output.pos = float4(clip, 0.0f, 1.0f);
    output.uv = float2((clip.x + 1.0f) * 0.5f, (1.0f - clip.y) * 0.5f);
    return output;
}

// gfx/FX/downsample.shader:62-73 mirrored from bloom.rs::bright_pass_downsample.
// The output is half-res; each output texel is the 2x2 box average of the four
// MainScene texels it covers, then the bright-pass knee, with log-luminance in
// alpha.
float4 bloom_bright_ps(BloomVsOut input) : SV_Target {
    float bright_threshold = BrightThresholdEmissiveInvSize.x;
    float emissive_strength = BrightThresholdEmissiveInvSize.y;
    float2 inv_source = BrightThresholdEmissiveInvSize.zw;
    // The half-res output texel center is `input.uv`; the four source texels it
    // covers are at +/- 0.5 source-texel around that center. Sampling exactly at
    // each source texel center with a linear sampler returns that texel, so this
    // is the 2x2 box average.
    float2 off = 0.5f * inv_source;
    float4 c00 = BloomSource.SampleLevel(BloomSampler, input.uv + float2(-off.x, -off.y), 0.0f);
    float4 c10 = BloomSource.SampleLevel(BloomSampler, input.uv + float2( off.x, -off.y), 0.0f);
    float4 c01 = BloomSource.SampleLevel(BloomSampler, input.uv + float2(-off.x,  off.y), 0.0f);
    float4 c11 = BloomSource.SampleLevel(BloomSampler, input.uv + float2( off.x,  off.y), 0.0f);
    float4 avg = 0.25f * (c00 + c10 + c01 + c11);
    float r = avg.r;
    float g = avg.g;
    float b = avg.b;
    float a = avg.a;
    // vMax = max(0, max(max(r,g),b) - BrightThreshold); vMax /= (0.5 + vMax);
    float vmax = max(max(max(r, g), b) - bright_threshold, 0.0f);
    vmax = vmax / (0.5f + vmax);
    vmax = vmax + a * emissive_strength;
    float3 color = float3(r, g, b) * vmax;
    // logLuminance = log(max(0, dot(rgb, LUMINANCE_VECTOR)) + 1e-4).
    float lum = max(dot(avg.rgb, BloomLuminanceVector), 0.0f);
    float log_luminance = log(lum + 0.0001f);
    return float4(color, log_luminance);
}

// gfx/FX/bloom.shader PixelShader mirrored from bloom.rs::blur_pass. Axis picks
// the blur direction: (1,0) horizontal, (0,1) vertical. Weights/offsets are the
// files-first linear-sampled Gaussian.
cbuffer BloomBlurCB : register(b0) {
    float4 AxisInvSize;   // xy = axis (1,0) or (0,1); zw = 1/blur-buffer size.
    float4 Weight0;       // x = center-tap weight.
    float4 Weights;       // per-pair combined weights.
    float4 Offsets;       // per-pair fractional sample offsets (in texels).
};

float4 bloom_blur_ps(BloomVsOut input) : SV_Target {
    float2 axis = AxisInvSize.xy;
    float2 texel = AxisInvSize.zw;
    float2 step = axis * texel;
    float4 color = BloomSource.SampleLevel(BloomSampler, input.uv, 0.0f) * Weight0.x;
    [unroll]
    for (int i = 0; i < 4; i++) {
        float2 d = step * Offsets[i];
        color += Weights[i] * (
            BloomSource.SampleLevel(BloomSampler, input.uv + d, 0.0f) +
            BloomSource.SampleLevel(BloomSampler, input.uv - d, 0.0f));
    }
    // bloom.shader returns float4(color, 1.0); the composite reads .rgb only.
    return float4(color.rgb, 1.0f);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct BloomBrightConstants {
    /// x = BRIGHT_THRESHOLD, y = EMISSIVE_BLOOM_STRENGTH, z/w = 1 / MainScene
    /// width/height (source-texel size for the 2x2 box tap centers).
    threshold_emissive_inv_size: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct BloomBlurConstants {
    /// xy = axis (1,0 horizontal / 0,1 vertical); zw = 1 / blur-buffer size.
    axis_inv_size: [f32; 4],
    /// x = center-tap weight (Weight0); yzw unused.
    weight0: [f32; 4],
    /// Per-pair combined weights (Weights[0..3]).
    weights: [f32; 4],
    /// Per-pair fractional sample offsets in texels (Offsets[0..3]).
    offsets: [f32; 4],
}

#[repr(C)]
struct CameraConstants {
    screen_size: [f32; 2],
    center_map: [f32; 2],
    zoom: f32,
    map_w: f32,
    map_h: f32,
    camera_height: f32,
    /// GB outline cutoffs, packed `(strategic, country)` to match the shader's
    /// `gb_outline_cutoff` and the engine's `vOutlineCutoff.x/.y` convention
    /// (see the `GB_OUTLINE_CUTOFF` note in `MAP_SHADER_HLSL`). Resolver-read
    /// from `00_graphics.lua` (mod-aware) with the vanilla 0.98/0.973 fallback.
    gb_outline_cutoff: [f32; 2],
    /// Pad to a 16-byte-multiple `ByteWidth` (D3D11 constant-buffer rule).
    camera_cb_pad: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct RestorePostfxConstants {
    hsv: [f32; 4],
    color_balance_use_lut: [f32; 4],
    exposure: [f32; 4],
}

#[derive(Clone, Debug, PartialEq)]
struct RestoreFullPostfx {
    enabled: bool,
    evidence_found: bool,
    constants: RestorePostfxConstants,
    color_cube_rgba8: Vec<u8>,
    /// Files-first bloom law resolved for the SAME posteffect volume the grade
    /// selected (so bloom and LUT never disagree about the active volume).
    /// `Some` iff `enabled`; the render composites `bloom.rgb + scene` before
    /// exposure/tonemap/LUT per restorescene.shader:208-223.
    bloom: Option<RestoreBloomLaw>,
    /// `MinHdr_MaxHdr` clamp bounds (`MIN_HDR_ADJUSTMENT` / `MAX_HDR_ADJUSTMENT`
    /// from `00_graphics.lua`, resolver-loaded). `Some` iff `enabled`; drives
    /// the per-frame luminance-adaptation reduction that recomputes exposure
    /// from the rendered scene (`downsample_luminance.shader` chain, ported in
    /// `map_viewer::luminance_law`). When present the composite pass replaces
    /// the file-grade's steady-state-floor AvgLum with the measured value.
    luminance_bounds: Option<LuminanceClampBounds>,
    /// The files-derived `MiddleGrey` the exposure divides by. Kept so the
    /// per-frame recompose can rebuild `exposure = [MiddleGrey, measured_avglum,
    /// gate]` without re-reading the file grade.
    middle_grey: f32,
}

/// The resolved, files-first bloom parameters + derived separable-blur kernel.
/// Both come from the installed content through the mod-aware resolver
/// (`resolve_bloom_params_for_volume`); no captured texture is a runtime input.
#[derive(Clone, Copy, Debug, PartialEq)]
struct RestoreBloomLaw {
    params: BloomParams,
    kernel: BloomBlurKernel,
}

struct RestoreColorCubeGpu {
    _texture: ComPtr<ID3D11Texture2D>,
    srv: ComPtr<ID3D11ShaderResourceView>,
}

/// GPU resources for the files-first bloom chain: a half-resolution bright-pass
/// target and a ping-pong pair for the two separable blur passes, plus the
/// bright-pass / blur shaders and the kernel constant buffer. All are created
/// only when `--postfx-restore-full` is active; the flag-OFF path never
/// allocates or binds any of them, so its output is byte-identical.
struct RestoreBloomGpu {
    half_width: u32,
    half_height: u32,
    /// Bright-pass output (MainScene/2). Also the vertical-blur output that the
    /// composite samples as RestoreBloom. Held to keep the texture alive; the
    /// RTV/SRV are the live handles.
    _bright_texture: ComPtr<ID3D11Texture2D>,
    bright_rtv: ComPtr<ID3D11RenderTargetView>,
    bright_srv: ComPtr<ID3D11ShaderResourceView>,
    /// Ping-pong scratch: horizontal-blur output, then vertical-blur input.
    _scratch_texture: ComPtr<ID3D11Texture2D>,
    scratch_rtv: ComPtr<ID3D11RenderTargetView>,
    scratch_srv: ComPtr<ID3D11ShaderResourceView>,
    bright_vs: ComPtr<ID3D11VertexShader>,
    bright_ps: ComPtr<ID3D11PixelShader>,
    blur_vs: ComPtr<ID3D11VertexShader>,
    blur_ps: ComPtr<ID3D11PixelShader>,
    /// Horizontal-pass constants (Axis=1) and vertical-pass constants (Axis=0).
    blur_cb_horizontal: ComPtr<ID3D11Buffer>,
    blur_cb_vertical: ComPtr<ID3D11Buffer>,
    /// Bright-pass knee/threshold/emissive constants.
    bright_cb: ComPtr<ID3D11Buffer>,
}

// common/defines/00_graphics.lua:1495-1496.
const BATTLE_PLAN_CAMERA_MIN_HEIGHT: f32 = 50.0;
const BATTLE_PLAN_CAMERA_MAX_HEIGHT: f32 = 3000.0;
// Captured 2026-07-02 `map_frame_draw020_ps_constant_buffer_slot0.bin`
// vCamPos.y at dx11_cb0 offset 208 was 567.25. map_viewer/src/cpu_view.rs:
// 207-209 grounds the zoom relation as `base.position[1] / zoom`, clamped by
// the game camera min/max defines.
const REPLAY_GB_BASE_CAMERA_HEIGHT: f32 = 567.25;
const TERRAIN_BUNDLE_FILE: &str = "map_frame_bundle.json";
const WATER_BUNDLE_FILE: &str = "map_frame_water_pass_bundle.json";
const RIVERS_BUNDLE_FILE: &str = "map_frame_rivers_pass_bundle.json";
const TREES_BUNDLE_FILE: &str = "map_frame_trees_pass_bundle.json";
const TERRAIN_CHUNK_LIBRARY_FILE: &str = "terrain_chunk_library.json";
const SYNTHESIZED_TERRAIN_TILE_SIZE: u32 = 128;

/// TREES2: emit the trees-layer note for a loaded trees bundle.
///
/// The historical warning unconditionally claimed the trees pass "captured the
/// first tree sub-draw only; missing second sub-draw pixel shader
/// 6b77b6df9ba8f03d remains unrendered". The OFF-3 byte-exact recompiler proves
/// 6b77b6df is `gfx/FX/tree.shader:tree_low_gfx` — the LOW_END_GFX variant of
/// the SAME lit tree PS, mutually exclusive with the full `tree` shader by the
/// graphics setting. Every captured session binds only the full-detail family;
/// the low-gfx shader is compiled-but-unused, so there is no unrendered second
/// sub-draw. This gates the note on real draw-binding evidence: it only calls
/// the state a seam when a bundle genuinely binds the low-gfx PS to a draw that
/// replay skipped (no captured session does today).
///
/// `--no-trees-second-subdraw` (`disable_second_subdraw`) forces the old
/// behaviour of NOT rendering a low-gfx draw even if one were ever bound; the
/// render path is identical regardless today (no bundle binds it), so it is the
/// byte-identity anchor. Falls back to the historical message when no resolver
/// is available (the verdict needs game-file access to classify the bytecode).
fn tree_layer_note(
    trees: &NativeD3d11TerrainBundle,
    game_resolver: Option<&GameFileResolver>,
    disable_second_subdraw: bool,
) -> String {
    let Some(resolver) = game_resolver else {
        // No resolver: cannot classify the bytecode by recompile, so keep the
        // conservative, honest statement (identification is unavailable here).
        return "trees layer: second-tree-shader identification unavailable without a --game \
                resolver; rendering the captured full-detail tree family"
            .to_string();
    };
    match tree_second_subdraw_verdict(resolver, trees) {
        Ok(verdict) => {
            let base = verdict.layer_note().unwrap_or_else(|| {
                "trees layer: rendering the captured full-detail tree family".to_string()
            });
            if disable_second_subdraw && verdict.low_gfx_bound_unrendered {
                format!("{base} (--no-trees-second-subdraw: low-gfx draw left unrendered)")
            } else {
                base
            }
        }
        Err(err) => {
            format!("trees layer: second-tree-shader identification failed ({err}); rendering the captured full-detail tree family")
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapturedLayerConfig {
    pub water: bool,
    pub rivers: bool,
    pub trees: bool,
}

impl Default for CapturedLayerConfig {
    fn default() -> Self {
        Self {
            water: true,
            rivers: true,
            trees: true,
        }
    }
}

pub fn playback_day_night_hour_from_timeline_hour(hour: i64) -> f32 {
    (hour.rem_euclid(24) as f32) / 24.0
}

fn shader_source_with_postfx(source: &str, postfx_enabled: bool) -> String {
    format!(
        "#define REPLAY_POSTFX_ENABLED {}\n{}",
        if postfx_enabled { 1 } else { 0 },
        source
    )
}

fn shader_source_with_restore_postfx(
    source: &str,
    postfx_enabled: bool,
    restore_full_enabled: bool,
    bloom_enabled: bool,
) -> String {
    if !restore_full_enabled {
        return shader_source_with_postfx(source, postfx_enabled);
    }
    // The bloom `#define` only affects the composite shader (which carries the
    // `REPLAY_BLOOM_ENABLED` guard); the map shader ignores it. When bloom is
    // off, the define is 0 (its default), so the composite compiles to the exact
    // pre-bloom code and the flag-ON-no-bloom output is unchanged.
    let bloom_define = format!(
        "#define REPLAY_BLOOM_ENABLED {}\n",
        if bloom_enabled { 1 } else { 0 }
    );
    let source = shader_source_with_postfx(source, postfx_enabled);
    let source = source.replace(
        "color = hoi4_scene_restore_tone_map(color);",
        "color = hoi4_scene_restore_full(color);",
    );
    if let Some(pos) = source.find("float4 ps_main") {
        let mut out = String::with_capacity(
            source.len() + RESTORE_FULL_POSTFX_HLSL.len() + bloom_define.len() + 1,
        );
        out.push_str(&bloom_define);
        out.push_str(&source[..pos]);
        out.push_str(RESTORE_FULL_POSTFX_HLSL);
        out.push('\n');
        out.push_str(&source[pos..]);
        out
    } else {
        source
    }
}

pub fn validate_day_night_hour(hour: f32) -> Result<()> {
    if hour.is_finite() && (0.0..1.0).contains(&hour) {
        Ok(())
    } else {
        Err(format!("day/night hour must be finite in [0,1), got {hour}").into())
    }
}

fn load_tree_seasons_table_from_bundle(bundle_dir: &Path) -> Result<SeasonsTable> {
    let manifest_path = bundle_dir.join("manifest.json");
    let manifest: serde_json::Value = serde_json::from_slice(&fs::read(&manifest_path)?)
        .map_err(|err| format!("read {} for seasons source: {err}", manifest_path.display()))?;
    let source_name = manifest
        .get("files")
        .and_then(|files| files.get("map_sources"))
        .and_then(|sources| sources.get("map/seasons.txt"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("source_seasons.txt");
    let path = bundle_dir.join(source_name);
    let bytes = fs::read(&path)
        .map_err(|err| format!("read preserved map/seasons.txt {}: {err}", path.display()))?;
    let source = bytes
        .iter()
        .map(|byte| char::from(*byte))
        .collect::<String>();
    seasons_table_from_source(&source)
        .map_err(|err| format!("parse preserved map/seasons.txt {}: {err}", path.display()).into())
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CapturedCameraKey {
    center_x: f64,
    center_y: f64,
    zoom: f64,
    screen_w: u32,
    screen_h: u32,
    day_night_hour: Option<f32>,
    tree_season_cb: Option<(f32, f32)>,
    map_clock_time: Option<f32>,
}

#[derive(Clone, Debug)]
pub struct CapturedPassStats {
    pub session_dir: PathBuf,
    pub target_width: u32,
    pub target_height: u32,
    pub target_format: u32,
    pub has_water: bool,
    pub has_rivers: bool,
    pub has_trees: bool,
    pub substituted_secondary: usize,
    pub substituted_gb1: usize,
    pub substituted_gb2: usize,
    pub synthesized_tile_size: Option<u32>,
    pub synthesized_tile_count: Option<usize>,
    pub last_render_ms: Option<f64>,
    pub last_day_night_hour: Option<f32>,
    pub postfx_enabled: bool,
    pub postfx_restore_full: bool,
    pub restore_evidence_found: bool,
    pub trees_from_files: bool,
    pub water_gap: Option<String>,
    pub layer_gaps: Vec<String>,
}

enum CapturedPassPrepared {
    Captured(NativeD3d11PreparedProductionMap),
    Synthesized(NativeD3d11PreparedSynthesizedProductionMap),
}

struct CapturedPassLayer {
    session_dir: PathBuf,
    renderer: NativeD3d11TerrainRenderer,
    prepared: CapturedPassPrepared,
    camera_projection: ProductionMapCameraProjection,
    camera_defines: Hoi4MapCameraDefines,
    output_srv: ComPtr<ID3D11ShaderResourceView>,
    composite_vs: ComPtr<ID3D11VertexShader>,
    composite_ps: ComPtr<ID3D11PixelShader>,
    composite_sampler: ComPtr<ID3D11SamplerState>,
    substitution: NativeD3d11TerrainPoliticalSrvSubstitution,
    water_gap: Option<String>,
    layer_gaps: Vec<String>,
    postfx_enabled: bool,
    restore_postfx: RestoreFullPostfx,
    restore_postfx_cb: Option<ComPtr<ID3D11Buffer>>,
    restore_color_cube: Option<RestoreColorCubeGpu>,
    restore_bloom: Option<RestoreBloomGpu>,
    tree_seasons_by_date: Option<SeasonsTable>,
    trees_from_files: bool,
    /// COL3: the political-mapmode scene CB0 state (GB override/cutoffs +
    /// shadow-fade height law) written into every pass's scene CB0 on camera
    /// update. Defines resolver-read (mod-aware) with the vanilla fallback;
    /// the replay always renders the political map, so this is always Some
    /// on the product path. See `map_viewer::PoliticalSceneCb0`.
    political_cb0: map_viewer::PoliticalSceneCb0,
    /// LABHDR1: lazily-created RTV over the HDR MainScene so label/furniture
    /// overlays render INTO the pre-tonemap scene like the engine's
    /// mapname/mapicons passes (see `bind_scene_target`).
    scene_rtv: Option<ComPtr<ID3D11RenderTargetView>>,
    /// SEASON1 validation-only override (env `REPLAY_SNOW_MUD_OVERRIDE`):
    /// keeps the substituted SnowMudTexture alive for the layer's lifetime.
    _snow_mud_override: Option<(ComPtr<ID3D11Texture2D>, ComPtr<ID3D11ShaderResourceView>)>,
    last_camera: Option<CapturedCameraKey>,
    dirty: bool,
    last_render: Option<Duration>,
}

impl CapturedPassPrepared {
    fn target_width(&self) -> u32 {
        match self {
            Self::Captured(prepared) => prepared.target_width(),
            Self::Synthesized(prepared) => prepared.target_width(),
        }
    }

    fn target_height(&self) -> u32 {
        match self {
            Self::Captured(prepared) => prepared.target_height(),
            Self::Synthesized(prepared) => prepared.target_height(),
        }
    }

    fn target_format(&self) -> u32 {
        match self {
            Self::Captured(prepared) => prepared.target_format(),
            Self::Synthesized(prepared) => prepared.target_format(),
        }
    }

    fn has_water_pass(&self) -> bool {
        match self {
            Self::Captured(prepared) => prepared.has_water_pass(),
            Self::Synthesized(prepared) => prepared.has_water_pass(),
        }
    }

    fn has_rivers_pass(&self) -> bool {
        match self {
            Self::Captured(prepared) => prepared.has_rivers_pass(),
            Self::Synthesized(prepared) => prepared.has_rivers_pass(),
        }
    }

    fn has_trees_pass(&self) -> bool {
        match self {
            Self::Captured(prepared) => prepared.has_trees_pass(),
            Self::Synthesized(prepared) => prepared.has_trees_pass(),
        }
    }

    fn synthesized_tile_size(&self) -> Option<u32> {
        match self {
            Self::Captured(_) => None,
            Self::Synthesized(prepared) => Some(prepared.terrain().target_tile_size()),
        }
    }

    fn synthesized_tile_count(&self) -> Option<usize> {
        match self {
            Self::Captured(_) => None,
            Self::Synthesized(prepared) => Some(prepared.terrain().tile_count()),
        }
    }

    unsafe fn output_texture_raw(&self) -> *mut ID3D11Texture2D {
        match self {
            Self::Captured(prepared) => prepared.output_texture_raw(),
            Self::Synthesized(prepared) => prepared.output_texture_raw(),
        }
    }

    unsafe fn replace_terrain_political_srvs(
        &mut self,
        province_secondary_color_map: *mut ID3D11ShaderResourceView,
        gradient_border_channel1: *mut ID3D11ShaderResourceView,
        gradient_border_channel2: *mut ID3D11ShaderResourceView,
    ) -> Result<NativeD3d11TerrainPoliticalSrvSubstitution> {
        match self {
            Self::Captured(prepared) => prepared.replace_terrain_political_srvs(
                province_secondary_color_map,
                gradient_border_channel1,
                gradient_border_channel2,
            ),
            Self::Synthesized(prepared) => prepared.replace_terrain_political_srvs(
                province_secondary_color_map,
                gradient_border_channel1,
                gradient_border_channel2,
            ),
        }
    }
    /// SEASON1 (validation): SnowMudTexture substitution across all passes.
    unsafe fn replace_snow_mud_srv(
        &mut self,
        snow_mud: *mut ID3D11ShaderResourceView,
    ) -> Result<usize> {
        match self {
            Self::Captured(prepared) => prepared.replace_snow_mud_srv(snow_mud),
            Self::Synthesized(prepared) => prepared.replace_snow_mud_srv(snow_mud),
        }
    }
}

impl CapturedPassLayer {
    unsafe fn new(
        gfx: &Gfx,
        bundle_dir: &Path,
        session_dir: &Path,
        game_resolver: Option<&GameFileResolver>,
        map_width: usize,
        map_height: usize,
        secondary_srv: *mut ID3D11ShaderResourceView,
        gradient1_srv: *mut ID3D11ShaderResourceView,
        gradient2_srv: *mut ID3D11ShaderResourceView,
        synthesize_terrain_chunks: bool,
        layer_config: CapturedLayerConfig,
        postfx_enabled: bool,
        restore_postfx: RestoreFullPostfx,
        tree_seasons_by_date: bool,
        trees_from_files: bool,
        no_trees_second_subdraw: bool,
        water_full: bool,
        water_full_explicit: bool,
        borders_from_files: bool,
        borders_from_files_explicit: bool,
        chunks_from_files: bool,
    ) -> Result<Self> {
        // GRAD1 fail-open policy for water: `--water-full` is DEFAULT ON. The
        // reflection cube (WATER1) needs no resolver, but the mechanism-correct
        // underwater refraction pass (WATER2) reconstructs the shipped pdxmap
        // `underwater` PS through the modset resolver. When water-full was
        // EXPLICITLY requested, a missing resolver is a HARD ERROR (fail closed).
        // When it is the default (no flag) and no resolver is available,
        // downgrade the WHOLE water path to the old approximation (both cube and
        // refraction) with a one-line notice, so the render stays byte-identical
        // to the old default rather than erroring. `--no-water-full` sets
        // `water_full = false` and never reaches this branch.
        let water_full = if water_full && game_resolver.is_none() {
            if water_full_explicit {
                return Err(
                    "--water-full refraction requires --game <the game install dir> or BATTLE_PLAN_VANILLA"
                        .into(),
                );
            }
            eprintln!(
                "replay_app: NOTICE: --water-full is default-ON but no --game resolver is available for the underwater refraction pass; downgrading to the old capture-anchored water default (mean-color reflection + approximation refraction). Pass --water-full to require it (fail closed) or --no-water-full to silence this."
            );
            false
        } else {
            water_full
        };
        // BORD5 fail-open policy for borders: `--borders-from-files` is DEFAULT
        // ON (the geometry law is captured-engine-derived and gate-tested
        // against the drawn Brittany|Loire strip). The pass reconstructs the
        // border mesh + shipped gfx/FX/border.shader + border_*_0.dds through
        // the modset resolver, so with no resolver: an EXPLICIT request fails
        // closed; the default downgrades to borders-off with a one-line notice.
        let borders_from_files = if borders_from_files && game_resolver.is_none() {
            if borders_from_files_explicit {
                return Err(
                    "--borders-from-files requires --game <the game install dir> or BATTLE_PLAN_VANILLA"
                        .into(),
                );
            }
            eprintln!(
                "replay_app: NOTICE: --borders-from-files is default-ON but no --game resolver is available for the border mesh/shader; rendering without thin borders. Pass --borders-from-files to require it (fail closed) or --no-borders-from-files to silence this."
            );
            false
        } else {
            borders_from_files
        };
        let mut renderer = NativeD3d11TerrainRenderer::from_borrowed_raw(
            gfx.device.as_raw(),
            gfx.context.as_raw(),
        )?;
        renderer.set_water_full(water_full);
        renderer.set_borders_from_files(borders_from_files);
        renderer.set_chunks_from_files(chunks_from_files);
        // WATER2: bind the modset resolver for the underwater refraction pass.
        // By construction (above) `water_full` is only true here when a resolver
        // exists, so this `.ok_or` can never trip on the default path.
        let renderer = if water_full {
            let resolver = game_resolver.ok_or(
                "--water-full refraction requires --game <the game install dir> or BATTLE_PLAN_VANILLA",
            )?;
            renderer.with_underwater_resolver(resolver.clone())
        } else {
            renderer
        };
        // BORD2: the synthesized border pass reconstructs the border mesh and
        // the shipped gfx/FX/border.shader + map/terrain/border_*.dds through
        // the modset resolver; --borders-from-files without a resolver must
        // fail closed rather than silently rendering nothing.
        let renderer = if borders_from_files {
            let resolver = game_resolver
                .ok_or("--borders-from-files requires --game <the game install dir> or BATTLE_PLAN_VANILLA")?;
            renderer.with_border_resolver(resolver.clone())
        } else {
            renderer
        };
        // CHUNK1: --chunks-from-files regenerates the terrain-chunk mesh (VB/IB/
        // CB1) from map/heightmap.bmp + the shipped vertex law via the modset
        // resolver; without a resolver it must fail closed rather than falling
        // back silently. Requires --synthesize-terrain-chunks (the synthesis
        // path this augments); enforced at the CLI layer.
        let renderer = if chunks_from_files {
            let resolver = game_resolver
                .ok_or("--chunks-from-files requires --game <the game install dir> or BATTLE_PLAN_VANILLA")?;
            renderer.with_chunks_from_files_resolver(resolver.clone())
        } else {
            renderer
        };
        // WATER1/REFLECT: when the modset resolver is available, the
        // ReflectionCubeMap slot (slot 8) is produced by decoding + repacking the
        // mod-aware map/terrain/reflection.dds and BYTE-VERIFIED equal to the
        // captured payload (fail closed on mismatch). PROVENANCE-only: pixels are
        // provably unchanged for vanilla, mod-correct otherwise. Only affects the
        // paths that upload the real cube (--water-full / reference replay); the
        // flag-OFF mean-color flatten never reaches the decode, so this is safe
        // to thread unconditionally where a resolver exists.
        let renderer = if let Some(resolver) = game_resolver {
            renderer.with_reflection_resolver(resolver.clone())
        } else {
            renderer
        };
        let tree_seasons_by_date = if tree_seasons_by_date {
            Some(load_tree_seasons_table_from_bundle(bundle_dir)?)
        } else {
            None
        };
        let patch_tree_season_cb1 = tree_seasons_by_date.is_some();
        let mut layer_gaps = Vec::new();
        let tree_file_layer = if trees_from_files {
            let resolver = game_resolver
                .ok_or("--trees-from-files requires --game <the game install dir> or BATTLE_PLAN_VANILLA")?;
            let layer = tree_file_derived_layer_from_resolver(resolver, map_width, map_height)?;
            layer_gaps.push(format!(
                "trees layer active: --trees-from-files selected {} {}:{} mesh {} with {} file-derived instances",
                layer.binding.name,
                layer.binding.source,
                layer.binding.name_line,
                layer.binding.mesh,
                layer.instances.stats.instance_count
            ));
            Some(layer)
        } else {
            None
        };
        let (base_camera, camera_defines, mut prepared, water_gap) = if synthesize_terrain_chunks {
            let chunk_library_path = session_dir.join(TERRAIN_CHUNK_LIBRARY_FILE);
            let chunk_library = NativeD3d11TerrainChunkLibrary::load(&chunk_library_path)?;
            let base_camera = chunk_library.production_camera_from_capture()?;
            let camera_defines = chunk_library.camera_defines()?;

            // Chunk templates were captured across different frames of the
            // session (sun direction, time-of-day, and snow fade drift between
            // them), so every synthesized tile must share one canonical scene
            // CB0. The terrain map frame bundle is that frame of record — the
            // same daytime frame the byte-exact parity gates replay.
            let scene_cb0 = NativeD3d11TerrainBundle::load(session_dir.join(TERRAIN_BUNDLE_FILE))?
                .scene_cb0_bytes()?;

            let water_path = session_dir.join(WATER_BUNDLE_FILE);
            let water = if layer_config.water && water_path.exists() {
                Some(NativeD3d11TerrainBundle::load_water(&water_path)?)
            } else if water_path.exists() {
                layer_gaps.push("water layer disabled by config".to_string());
                None
            } else {
                None
            };
            let rivers_path = session_dir.join(RIVERS_BUNDLE_FILE);
            let rivers = if water.is_some() && layer_config.rivers && rivers_path.exists() {
                Some(NativeD3d11TerrainBundle::load_rivers(&rivers_path)?)
            } else if rivers_path.exists() && !layer_config.rivers {
                layer_gaps.push("rivers layer disabled by config".to_string());
                None
            } else if rivers_path.exists() && water.is_none() {
                layer_gaps.push(
                    "rivers layer skipped because the water dependency is unavailable".to_string(),
                );
                None
            } else {
                None
            };
            let trees_path = session_dir.join(TREES_BUNDLE_FILE);
            let want_trees = trees_from_files || layer_config.trees;
            let trees = if rivers.is_some() && want_trees && trees_path.exists() {
                let trees = NativeD3d11TerrainBundle::load_trees(&trees_path)?;
                layer_gaps.push(tree_layer_note(
                    &trees,
                    game_resolver,
                    no_trees_second_subdraw,
                ));
                Some(trees)
            } else if trees_path.exists() && !layer_config.trees && !trees_from_files {
                layer_gaps.push("trees layer disabled by config".to_string());
                None
            } else if trees_path.exists() && rivers.is_none() {
                layer_gaps.push(
                    "trees layer skipped because the rivers dependency is unavailable".to_string(),
                );
                None
            } else if trees_from_files {
                layer_gaps.push(
                    "--trees-from-files skipped because the capture session has no tree pass template bundle".to_string(),
                );
                None
            } else {
                None
            };

            (
                base_camera,
                camera_defines,
                CapturedPassPrepared::Synthesized(
                    renderer.prepare_terrain_chunk_synthesis_with_generated_layers_tree_files_and_tree_season_patch(
                        &chunk_library,
                        water.as_ref(),
                        rivers.as_ref(),
                        None,
                        trees.as_ref(),
                        base_camera,
                        SYNTHESIZED_TERRAIN_TILE_SIZE,
                        chunk_library.world_rect(),
                        Some(&MapLayerSynthesisInputs::from_bundle_dir(bundle_dir)?),
                        tree_file_layer.as_ref(),
                        Some(&scene_cb0),
                        patch_tree_season_cb1,
                    )?,
                ),
                None,
            )
        } else {
            let terrain_path = session_dir.join(TERRAIN_BUNDLE_FILE);
            let terrain = NativeD3d11TerrainBundle::load(&terrain_path)?;
            let base_camera = terrain.production_camera_from_capture()?;
            let camera_defines = hoi4_map_camera_defines_from_bundle(&terrain)?;

            let water_path = session_dir.join(WATER_BUNDLE_FILE);
            let water = if layer_config.water && water_path.exists() {
                Some(NativeD3d11TerrainBundle::load_water(&water_path)?)
            } else if water_path.exists() {
                layer_gaps.push("water layer disabled by config".to_string());
                None
            } else {
                None
            };
            let rivers_path = session_dir.join(RIVERS_BUNDLE_FILE);
            let rivers = if water.is_some() && layer_config.rivers && rivers_path.exists() {
                Some(NativeD3d11TerrainBundle::load_rivers(&rivers_path)?)
            } else if rivers_path.exists() && !layer_config.rivers {
                layer_gaps.push("rivers layer disabled by config".to_string());
                None
            } else if rivers_path.exists() && water.is_none() {
                layer_gaps.push(
                    "rivers layer skipped because the water dependency is unavailable".to_string(),
                );
                None
            } else {
                None
            };
            let trees_path = session_dir.join(TREES_BUNDLE_FILE);
            let want_trees = trees_from_files || layer_config.trees;
            let trees = if rivers.is_some() && want_trees && trees_path.exists() {
                // TREES2: PAN2 (and every captured session) binds only the
                // full-detail tree family. The 6b77b6df pixel shader the old
                // warning called a "second sub-draw" is gfx/FX/tree.shader:
                // tree_low_gfx — the LOW_END_GFX variant, compiled but bound to
                // no draw at the captured graphics settings. Gate the note on
                // real draw-binding evidence instead of asserting a gap.
                let trees = NativeD3d11TerrainBundle::load_trees(&trees_path)?;
                layer_gaps.push(tree_layer_note(
                    &trees,
                    game_resolver,
                    no_trees_second_subdraw,
                ));
                Some(trees)
            } else if trees_path.exists() && !layer_config.trees && !trees_from_files {
                layer_gaps.push("trees layer disabled by config".to_string());
                None
            } else if trees_path.exists() && rivers.is_none() {
                layer_gaps.push(
                    "trees layer skipped because the rivers dependency is unavailable".to_string(),
                );
                None
            } else if trees_from_files {
                layer_gaps.push(
                    "--trees-from-files skipped because the capture session has no tree pass template bundle".to_string(),
                );
                None
            } else {
                None
            };
            let prepared_result = if trees_from_files {
                renderer.prepare_production_map_with_layers_tree_files_and_tree_season_patch(
                    &terrain,
                    water.as_ref(),
                    rivers.as_ref(),
                    None,
                    trees.as_ref(),
                    base_camera,
                    tree_file_layer.as_ref().filter(|_| trees.is_some()),
                    patch_tree_season_cb1,
                )
            } else if patch_tree_season_cb1 {
                renderer.prepare_production_map_with_layers_and_tree_season_patch(
                    &terrain,
                    water.as_ref(),
                    rivers.as_ref(),
                    None,
                    trees.as_ref(),
                    base_camera,
                )
            } else {
                renderer.prepare_production_map_with_layers(
                    &terrain,
                    water.as_ref(),
                    rivers.as_ref(),
                    None,
                    trees.as_ref(),
                    base_camera,
                )
            };
            let prepared = match prepared_result {
                Ok(prepared) => prepared,
                Err(err) if trees_from_files && trees.is_some() => {
                    layer_gaps.push(format!(
                        "--trees-from-files skipped because the file-derived tree patch did not prepare: {err}"
                    ));
                    renderer.prepare_production_map_with_layers(
                        &terrain,
                        water.as_ref(),
                        rivers.as_ref(),
                        None,
                        None,
                        base_camera,
                    )?
                }
                Err(err) if trees.is_some() => {
                    layer_gaps.push(format!(
                        "trees layer skipped because the captured first sub-draw is not D3D11-replayable yet: {err}"
                    ));
                    renderer.prepare_production_map_with_layers(
                        &terrain,
                        water.as_ref(),
                        rivers.as_ref(),
                        None,
                        None,
                        base_camera,
                    )?
                }
                Err(err) => return Err(err),
            };
            (
                base_camera,
                camera_defines,
                CapturedPassPrepared::Captured(prepared),
                None,
            )
        };
        let camera_projection = ProductionMapCameraProjection::from_camera(base_camera)?;
        // BORD2: surface the prepared border-overlay footprint (VRAM + class
        // ranges + prepare time) alongside the other layer diagnostics.
        if let Some(stats) = match &prepared {
            CapturedPassPrepared::Captured(prepared) => prepared.borders_files_stats(),
            CapturedPassPrepared::Synthesized(prepared) => prepared.borders_files_stats(),
        } {
            layer_gaps.push(format!(
                "borders layer active: --borders-from-files uploaded {} vertices ({:.1} MiB) + \
                 {} indices ({:.1} MiB) once; classes {}; prepare {:.0} ms",
                stats.vertex_count,
                stats.vertex_buffer_bytes as f64 / (1024.0 * 1024.0),
                stats.index_count,
                stats.index_buffer_bytes as f64 / (1024.0 * 1024.0),
                stats
                    .class_ranges
                    .iter()
                    .map(|(material, count)| format!("{material}={}", count / 6))
                    .collect::<Vec<_>>()
                    .join(" "),
                stats.prepare_ms,
            ));
        }
        let substitution =
            prepared.replace_terrain_political_srvs(secondary_srv, gradient1_srv, gradient2_srv)?;
        let output_srv = create_captured_output_srv(
            gfx,
            prepared.target_format(),
            prepared.output_texture_raw(),
        )?;
        // Bloom is part of the postfx composite (restorescene.shader adds it
        // before Exposure/ToneMap). So it must ride the SAME `postfx_enabled`
        // switch as tone/LUT: with `--no-postfx` the whole restore block is off
        // and bloom must be off too, or the output would be "linear scene +
        // bloom" instead of the byte-identical raw captured-RT look.
        let bloom_enabled =
            postfx_enabled && restore_postfx.enabled && restore_postfx.bloom.is_some();
        let composite_shader_source = shader_source_with_restore_postfx(
            CAPTURED_PASS_COMPOSITE_HLSL,
            postfx_enabled,
            restore_postfx.enabled,
            bloom_enabled,
        );
        let composite_vs_blob = compile_shader(&composite_shader_source, "vs_main", "vs_4_0")?;
        let composite_ps_blob = compile_shader(&composite_shader_source, "ps_main", "ps_4_0")?;
        let mut composite_vs: *mut ID3D11VertexShader = ptr::null_mut();
        let hr = (*gfx.device.as_raw()).CreateVertexShader(
            (*composite_vs_blob.as_raw()).GetBufferPointer(),
            (*composite_vs_blob.as_raw()).GetBufferSize(),
            ptr::null_mut(),
            &mut composite_vs,
        );
        hr_ok(hr, "CreateVertexShader for captured-pass compositor")?;
        let composite_vs = ComPtr::from_raw(composite_vs, "captured-pass composite vertex shader")?;
        let mut composite_ps: *mut ID3D11PixelShader = ptr::null_mut();
        let hr = (*gfx.device.as_raw()).CreatePixelShader(
            (*composite_ps_blob.as_raw()).GetBufferPointer(),
            (*composite_ps_blob.as_raw()).GetBufferSize(),
            ptr::null_mut(),
            &mut composite_ps,
        );
        hr_ok(hr, "CreatePixelShader for captured-pass compositor")?;
        let composite_ps = ComPtr::from_raw(composite_ps, "captured-pass composite pixel shader")?;
        let composite_sampler = create_linear_clamp_sampler(gfx)?;
        let restore_postfx_cb = if restore_postfx.enabled {
            Some(create_restore_postfx_constant_buffer(
                gfx,
                restore_postfx.constants,
            )?)
        } else {
            None
        };
        let restore_color_cube = create_restore_color_cube_texture(gfx, &restore_postfx)?;
        // Files-first bloom GPU chain, sized to the HDR MainScene the compositor
        // grades. Created only when the compositor will actually sample it
        // (`bloom_enabled`, i.e. postfx on + restore-full on + a resolved bloom
        // law). Otherwise this stays `None` and no extra target is allocated,
        // run, or bound — so flag-OFF and `--no-postfx` output is unchanged. The
        // scene target must be R16G16B16A16_FLOAT (the game's MainScene); fail
        // closed if a future capture format differs.
        let restore_bloom = if bloom_enabled {
            create_restore_bloom_gpu(
                gfx,
                &restore_postfx,
                prepared.target_width(),
                prepared.target_height(),
                prepared.target_format(),
            )?
        } else {
            None
        };

        eprintln!(
            "replay_app: captured-pass session {} loaded ({}x{} format {}, water {}, rivers {}, trees {}, synthesis {}, postfx {}, restore_full {}, restore_evidence {}, bloom {}, tree_seasons_by_date {}, SRV substitutions secondary={} gb1={} gb2={})",
            session_dir.display(),
            prepared.target_width(),
            prepared.target_height(),
            prepared.target_format(),
            if prepared.has_water_pass() { "on" } else { "off" },
            if prepared.has_rivers_pass() { "on" } else { "off" },
            if prepared.has_trees_pass() { "on" } else { "off" },
            if synthesize_terrain_chunks { "on" } else { "off" },
            if postfx_enabled { "on" } else { "off" },
            if restore_postfx.enabled { "on" } else { "off" },
            if restore_postfx.evidence_found { "found" } else { "none" },
            if restore_bloom.is_some() { "on" } else { "off" },
            if tree_seasons_by_date.is_some() { "on" } else { "off" },
            substitution.province_secondary_color_map,
            substitution.gradient_border_channel1,
            substitution.gradient_border_channel2
        );

        // COL3: political-mapmode scene CB0 defines (resolver-read like the
        // GB country field; vanilla fallback without a --game resolver).
        let mut political_cb0 = match game_resolver {
            Some(resolver) => crate::gb_field::PoliticalSceneDefines::from_resolver(resolver)?,
            None => crate::gb_field::PoliticalSceneDefines::vanilla(),
        }
        .to_cb0();
        // SEASON1 VALIDATION-ONLY env knobs (never a product default; used by
        // the RDOC parity harness to substitute an engine-extracted January
        // SnowMudTexture + its SnowMudFade and quantify the winter residual):
        //   REPLAY_SNOW_MUD_OVERRIDE=<raw 1408x512 BGRA8 file>
        //   REPLAY_SNOW_MUD_FADE=<0..1>
        let snow_mud_override = match std::env::var("REPLAY_SNOW_MUD_OVERRIDE") {
            Ok(path) if !path.trim().is_empty() => {
                let bytes = std::fs::read(&path)
                    .map_err(|err| format!("REPLAY_SNOW_MUD_OVERRIDE read {path}: {err}"))?;
                const W: usize = 1408;
                const H: usize = 512;
                if bytes.len() != W * H * 4 {
                    return Err(format!(
                        "REPLAY_SNOW_MUD_OVERRIDE {path}: {} bytes, expected {} (1408x512 BGRA8)",
                        bytes.len(),
                        W * H * 4
                    )
                    .into());
                }
                let desc = D3D11_TEXTURE2D_DESC {
                    Width: W as u32,
                    Height: H as u32,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_IMMUTABLE,
                    BindFlags: D3D11_BIND_SHADER_RESOURCE,
                    CPUAccessFlags: 0,
                    MiscFlags: 0,
                };
                let init = D3D11_SUBRESOURCE_DATA {
                    pSysMem: bytes.as_ptr() as *const _,
                    SysMemPitch: (W * 4) as u32,
                    SysMemSlicePitch: 0,
                };
                let mut tex: *mut ID3D11Texture2D = ptr::null_mut();
                let hr = (*gfx.device.as_raw()).CreateTexture2D(&desc, &init, &mut tex);
                hr_ok(hr, "CreateTexture2D SnowMud override")?;
                let tex = ComPtr::from_raw(tex, "SnowMud override texture")?;
                let mut srv: *mut ID3D11ShaderResourceView = ptr::null_mut();
                let hr = (*gfx.device.as_raw()).CreateShaderResourceView(
                    tex.as_raw() as *mut _,
                    ptr::null(),
                    &mut srv,
                );
                hr_ok(hr, "CreateShaderResourceView SnowMud override")?;
                let srv = ComPtr::from_raw(srv, "SnowMud override SRV")?;
                let count = prepared.replace_snow_mud_srv(srv.as_raw())?;
                eprintln!(
                    "replay_app: SEASON1 validation: SnowMudTexture override from {path} substituted in {count} bindings"
                );
                Some((tex, srv))
            }
            _ => None,
        };
        if let Ok(fade) = std::env::var("REPLAY_SNOW_MUD_FADE") {
            if let Ok(fade) = fade.parse::<f32>() {
                political_cb0.snow_mud_fade = Some(fade);
                eprintln!("replay_app: SEASON1 validation: SnowMudFade override {fade}");
            }
        }

        Ok(Self {
            session_dir: session_dir.to_path_buf(),
            renderer,
            prepared,
            camera_projection,
            camera_defines,
            output_srv,
            composite_vs,
            composite_ps,
            composite_sampler,
            substitution,
            water_gap,
            layer_gaps,
            postfx_enabled,
            restore_postfx,
            restore_postfx_cb,
            restore_color_cube,
            restore_bloom,
            tree_seasons_by_date,
            trees_from_files,
            political_cb0,
            scene_rtv: None,
            _snow_mud_override: snow_mud_override,
            last_camera: None,
            dirty: true,
            last_render: None,
        })
    }

    fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// `day_night_hour = None` keeps the captured CB0 day/night state (the
    /// game's default: terminator shading off). `Some(hour)` opts into the
    /// game's day/night toggle at that hour.
    fn render(
        &mut self,
        gfx: &Gfx,
        camera: &Camera,
        day_night_hour: Option<f32>,
        tree_season_date: Option<(u32, u32)>,
        map_clock_time: Option<f32>,
    ) -> Result<()> {
        if let Some(hour) = day_night_hour {
            validate_day_night_hour(hour)?;
        }
        let tree_season_cb =
            captured_tree_season_cb_from_date(self.tree_seasons_by_date.as_ref(), tree_season_date);
        let key = CapturedCameraKey {
            center_x: camera.center_x,
            center_y: camera.center_y,
            zoom: camera.zoom,
            screen_w: gfx.width,
            screen_h: gfx.height,
            day_night_hour,
            tree_season_cb,
            map_clock_time,
        };
        if self.dirty || self.last_camera != Some(key) {
            let production_camera = production_camera_from_app_camera(
                self.camera_projection,
                self.camera_defines,
                camera,
                gfx.width,
            )?;
            let cull_world_rect =
                camera_ground_cull_rect(&production_camera, gfx.width, gfx.height);
            let wrap_offsets = if let Some(rect) = cull_world_rect {
                let offsets = map_wrap_offsets(rect[0], rect[2], PRODUCT_MAP_WIDTH as f32);
                if offsets.is_empty() {
                    return Err(
                        "production map renderer blocked: camera wrap cull span is invalid".into(),
                    );
                }
                offsets
            } else {
                vec![0.0]
            };
            let start = Instant::now();
            for (copy_index, wrap_offset) in wrap_offsets.iter().copied().enumerate() {
                let copy_camera = production_camera_for_wrap_copy(
                    self.camera_projection,
                    production_camera,
                    wrap_offset,
                )?;
                // The virtual light rig is driven by the day-cycle law at
                // `day_night_hour`, not by the camera (the camera-follow FIT is
                // removed). `day_night_hour = None` keeps the captured rig
                // verbatim. See `shader_meta::write_virtual_light_rig_fields`.
                match &mut self.prepared {
                    CapturedPassPrepared::Captured(prepared) => {
                        self.renderer.update_prepared_production_map_scene(
                            prepared,
                            copy_camera,
                            day_night_hour,
                            map_clock_time,
                            Some(self.political_cb0),
                        )?
                    }
                    CapturedPassPrepared::Synthesized(prepared) => self
                        .renderer
                        .update_prepared_synthesized_production_map_scene(
                            prepared,
                            copy_camera,
                            day_night_hour,
                            map_clock_time,
                            Some(self.political_cb0),
                        )?,
                }
                let options = NativeD3d11RenderOptions {
                    output: None,
                    readback: false,
                    compare_each_draw: false,
                    compare_final_reference: false,
                    parity_report: None,
                    clear_targets: copy_index == 0,
                    // Only draw synthesized world tiles the camera can
                    // actually see; the plain captured bundle ignores this
                    // cull option and redraws its captured pass per copy.
                    cull_world_rect: shifted_cull_world_rect(cull_world_rect, wrap_offset),
                    tree_season_cb,
                    map_clock_time,
                };
                match &self.prepared {
                    CapturedPassPrepared::Captured(prepared) => {
                        self.renderer
                            .render_prepared_production_map(prepared, &options)?;
                    }
                    CapturedPassPrepared::Synthesized(prepared) => {
                        self.renderer
                            .render_prepared_synthesized_production_map(prepared, &options)?;
                    }
                }
            }
            // Files-first per-frame exposure adaptation (FIDELITY_LEDGER debt
            // #2): reduce the just-rendered HDR MainScene through the ported
            // downsample_luminance.shader chain and rewrite the exposure's
            // AverageLuminance, replacing the constant MIN_HDR_ADJUSTMENT
            // floor. AvgLum depends only on the rendered scene bytes, so it is
            // recomputed exactly when the scene re-renders (this dirty branch)
            // rather than in composite() — composite runs every presented
            // frame and the readback is a full GPU sync + CPU reduction.
            // Gated so the flag-OFF default path never reads back or mutates
            // anything and stays byte-identical.
            if self.postfx_enabled && self.restore_postfx.enabled {
                if let (Some(bounds), Some(cb)) = (
                    self.restore_postfx.luminance_bounds,
                    self.restore_postfx_cb.as_ref(),
                ) {
                    unsafe { self.update_exposure_from_scene_luminance(gfx, bounds, cb)? };
                }
            }
            self.last_render = Some(start.elapsed());
            self.last_camera = Some(key);
            self.dirty = false;
        }
        self.composite(gfx)
    }

    /// LABHDR1: scene-only render (no composite). Used by the split draw
    /// path that inserts label/furniture draws INTO the HDR MainScene —
    /// matching the engine, whose mapname/mapicons passes render into the
    /// pre-tonemap scene target so names are graded with the map.
    fn render_scene_only(
        &mut self,
        gfx: &Gfx,
        camera: &Camera,
        day_night_hour: Option<f32>,
        tree_season_date: Option<(u32, u32)>,
        map_clock_time: Option<f32>,
    ) -> Result<()> {
        // render() with the composite skipped: reuse render() but defer its
        // trailing composite by masking it. The dirty-tracking logic is
        // identical; composite() is idempotent per frame and is invoked by
        // `composite_only` afterwards.
        if let Some(hour) = day_night_hour {
            validate_day_night_hour(hour)?;
        }
        let tree_season_cb =
            captured_tree_season_cb_from_date(self.tree_seasons_by_date.as_ref(), tree_season_date);
        let key = CapturedCameraKey {
            center_x: camera.center_x,
            center_y: camera.center_y,
            zoom: camera.zoom,
            screen_w: gfx.width,
            screen_h: gfx.height,
            day_night_hour,
            tree_season_cb,
            map_clock_time,
        };
        if self.dirty || self.last_camera != Some(key) {
            // Reuse render() (scene update + one composite). The composite it
            // runs is premature — overlays land in the HDR scene afterwards —
            // so the split-draw caller re-composites via `composite_only`
            // once overlays are in. Cost: one redundant fullscreen triangle;
            // benefit: zero duplication of the ~90-line scene-update body.
            self.render(gfx, camera, day_night_hour, tree_season_date, map_clock_time)?;
        }
        Ok(())
    }

    /// LABHDR1: composite the (possibly overlay-augmented) HDR MainScene to
    /// the final target.
    fn composite_only(&self, gfx: &Gfx) -> Result<()> {
        self.composite(gfx)
    }

    /// LABHDR1: the HDR MainScene as an overlay render target (lazily
    /// created RTV over the captured output texture) + its dimensions. The
    /// text pipeline binds it per draw via its target override; the RTV
    /// stays owned here and outlives the overlay calls.
    unsafe fn scene_overlay_target(
        &mut self,
        gfx: &Gfx,
    ) -> Result<(*mut ID3D11RenderTargetView, u32, u32)> {
        if self.scene_rtv.is_none() {
            let mut rtv: *mut ID3D11RenderTargetView = ptr::null_mut();
            let hr = (*gfx.device.as_raw()).CreateRenderTargetView(
                self.prepared.output_texture_raw() as *mut _,
                ptr::null(),
                &mut rtv,
            );
            hr_ok(hr, "CreateRenderTargetView over HDR MainScene for overlays")?;
            self.scene_rtv = Some(ComPtr::from_raw(rtv, "HDR MainScene overlay RTV")?);
        }
        Ok((
            self.scene_rtv.as_ref().expect("just created").as_raw(),
            self.prepared.target_width(),
            self.prepared.target_height(),
        ))
    }

    fn composite(&self, gfx: &Gfx) -> Result<()> {
        // Files-first bloom: build the RestoreBloom half-res buffer from the
        // replay's own HDR MainScene (output_srv) BEFORE the final composite,
        // so the compositor can add it to the linear scene per
        // restorescene.shader:210-213. No-op unless bloom is enabled.
        if let Some(bloom) = self.restore_bloom.as_ref() {
            unsafe { self.run_bloom_chain(gfx, bloom)? };
        }
        unsafe {
            let context = gfx.context.as_raw();
            let rtv = gfx.rtv()?.as_raw();
            let clear = [0.0f32, 0.0, 0.0, 1.0];
            (*context).ClearRenderTargetView(rtv, &clear);
            let rtvs = [rtv];
            (*context).OMSetRenderTargets(1, rtvs.as_ptr(), ptr::null_mut());
            let viewport = D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: gfx.width as f32,
                Height: gfx.height as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            };
            (*context).RSSetViewports(1, &viewport);
            (*context).RSSetState(ptr::null_mut());
            let blend_factor = [0.0f32, 0.0, 0.0, 0.0];
            (*context).OMSetBlendState(ptr::null_mut(), &blend_factor, u32::MAX);
            (*context).OMSetDepthStencilState(ptr::null_mut(), 0);
            (*context).IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            (*context).IASetInputLayout(ptr::null_mut());
            (*context).VSSetShader(self.composite_vs.as_raw(), ptr::null(), 0);
            (*context).PSSetShader(self.composite_ps.as_raw(), ptr::null(), 0);
            if let Some(constants) = self.restore_postfx_cb.as_ref() {
                let constant_buffers = [constants.as_raw()];
                (*context).PSSetConstantBuffers(1, 1, constant_buffers.as_ptr());
            }
            let srvs = [self.output_srv.as_raw()];
            (*context).PSSetShaderResources(0, 1, srvs.as_ptr());
            if let Some(color_cube) = self.restore_color_cube.as_ref() {
                let srvs = [color_cube.srv.as_raw()];
                (*context).PSSetShaderResources(7, 1, srvs.as_ptr());
            }
            if let Some(bloom) = self.restore_bloom.as_ref() {
                // RestoreBloom (t8) is the vertical-blur output.
                let srvs = [bloom.bright_srv.as_raw()];
                (*context).PSSetShaderResources(8, 1, srvs.as_ptr());
            }
            let samplers = [self.composite_sampler.as_raw()];
            (*context).PSSetSamplers(0, 1, samplers.as_ptr());
            (*context).Draw(3, 0);
            // Unbind t0 (the scene) and t8 (bloom) so those textures can be
            // render targets again next frame — the bloom chain re-renders into
            // the same half-res target, and D3D11 forbids simultaneous SRV+RTV
            // binding of one resource.
            if self.restore_bloom.is_some() {
                let null_srvs: [*mut ID3D11ShaderResourceView; 1] = [ptr::null_mut()];
                (*context).PSSetShaderResources(0, 1, null_srvs.as_ptr());
                (*context).PSSetShaderResources(8, 1, null_srvs.as_ptr());
            }
        }
        Ok(())
    }

    /// Run the files-first bloom chain into `bloom.bright_srv`:
    ///   1. bright pass / 2x downsample  (output_srv -> bright_texture)
    ///   2. horizontal blur (Axis=1)      (bright_texture -> scratch_texture)
    ///   3. vertical blur (Axis=0)        (scratch_texture -> bright_texture)
    /// Every stage unbinds its source SRV before that resource is next used as a
    /// render target, so no texture is ever bound as SRV and RTV at once.
    ///
    /// # Safety
    /// `bloom`'s D3D11 resources and `self.output_srv` must be valid and belong
    /// to `gfx`'s device; the caller invokes this before the final composite.
    unsafe fn run_bloom_chain(&self, gfx: &Gfx, bloom: &RestoreBloomGpu) -> Result<()> {
        let context = gfx.context.as_raw();
        // Half-res viewport for all three passes.
        let half_viewport = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: bloom.half_width as f32,
            Height: bloom.half_height as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        let blend_factor = [0.0f32, 0.0, 0.0, 0.0];
        (*context).RSSetState(ptr::null_mut());
        (*context).OMSetBlendState(ptr::null_mut(), &blend_factor, u32::MAX);
        (*context).OMSetDepthStencilState(ptr::null_mut(), 0);
        (*context).IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        (*context).IASetInputLayout(ptr::null_mut());
        (*context).RSSetViewports(1, &half_viewport);
        let samplers = [self.composite_sampler.as_raw()];
        (*context).PSSetSamplers(0, 1, samplers.as_ptr());
        let null_srv: [*mut ID3D11ShaderResourceView; 1] = [ptr::null_mut()];

        // Pass 1: bright pass + 2x downsample. Source = full-res MainScene.
        (*context).OMSetRenderTargets(1, [bloom.bright_rtv.as_raw()].as_ptr(), ptr::null_mut());
        (*context).VSSetShader(bloom.bright_vs.as_raw(), ptr::null(), 0);
        (*context).PSSetShader(bloom.bright_ps.as_raw(), ptr::null(), 0);
        (*context).PSSetConstantBuffers(0, 1, [bloom.bright_cb.as_raw()].as_ptr());
        (*context).PSSetShaderResources(0, 1, [self.output_srv.as_raw()].as_ptr());
        (*context).Draw(3, 0);
        // Unbind the RT and its SRV source before reuse.
        (*context).OMSetRenderTargets(1, null_srv.as_ptr() as *const *mut _, ptr::null_mut());
        (*context).PSSetShaderResources(0, 1, null_srv.as_ptr());

        // Pass 2: horizontal blur (Axis=1). bright_texture -> scratch_texture.
        (*context).OMSetRenderTargets(1, [bloom.scratch_rtv.as_raw()].as_ptr(), ptr::null_mut());
        (*context).VSSetShader(bloom.blur_vs.as_raw(), ptr::null(), 0);
        (*context).PSSetShader(bloom.blur_ps.as_raw(), ptr::null(), 0);
        (*context).PSSetConstantBuffers(0, 1, [bloom.blur_cb_horizontal.as_raw()].as_ptr());
        (*context).PSSetShaderResources(0, 1, [bloom.bright_srv.as_raw()].as_ptr());
        (*context).Draw(3, 0);
        (*context).OMSetRenderTargets(1, null_srv.as_ptr() as *const *mut _, ptr::null_mut());
        (*context).PSSetShaderResources(0, 1, null_srv.as_ptr());

        // Pass 3: vertical blur (Axis=0). scratch_texture -> bright_texture
        // (reused as the final RestoreBloom buffer the compositor reads).
        (*context).OMSetRenderTargets(1, [bloom.bright_rtv.as_raw()].as_ptr(), ptr::null_mut());
        (*context).PSSetConstantBuffers(0, 1, [bloom.blur_cb_vertical.as_raw()].as_ptr());
        (*context).PSSetShaderResources(0, 1, [bloom.scratch_srv.as_raw()].as_ptr());
        (*context).Draw(3, 0);
        (*context).OMSetRenderTargets(1, null_srv.as_ptr() as *const *mut _, ptr::null_mut());
        (*context).PSSetShaderResources(0, 1, null_srv.as_ptr());

        Ok(())
    }

    /// FILES-FIRST per-frame exposure adaptation. Read back the rendered HDR
    /// MainScene, reduce it through the ported `downsample_luminance.shader`
    /// chain (`map_viewer::luminance_law::scene_average_luminance`) to a single
    /// `AverageLuminance`, and rewrite the exposure constant's AvgLum field with
    /// it — so the exposure scale `MiddleGrey / AverageLuminance` adapts to THIS
    /// frame's scene instead of the constant MIN_HDR_ADJUSTMENT floor.
    ///
    /// MiddleGrey (`exposure.x`), the LUT-block gate (`exposure.z`), and every
    /// other grade constant are preserved from the file grade; only `exposure.y`
    /// changes. The measured value is R16-quantized inside the reduction to
    /// match the engine's 1x1 `R16_FLOAT` `AverageLuminanceTexture` precision.
    ///
    /// # Safety
    /// `cb` must be the DYNAMIC `RestorePostfxConstants` buffer bound at PS slot
    /// 1; `self.prepared`'s output texture must be a valid RGBA16F MainScene on
    /// `gfx`'s device.
    unsafe fn update_exposure_from_scene_luminance(
        &self,
        gfx: &Gfx,
        bounds: LuminanceClampBounds,
        cb: &ComPtr<ID3D11Buffer>,
    ) -> Result<AverageLuminanceReduction> {
        let width = self.prepared.target_width();
        let height = self.prepared.target_height();
        let format = self.prepared.target_format();
        // Byte-for-byte staging copy of the rendered MainScene, mapped for read.
        let source = self.prepared.output_texture_raw();
        let staging_desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: format,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ,
            MiscFlags: 0,
        };
        let mut staging: *mut ID3D11Texture2D = ptr::null_mut();
        let hr = (*gfx.device.as_raw()).CreateTexture2D(&staging_desc, ptr::null(), &mut staging);
        hr_ok(hr, "CreateTexture2D staging for scene luminance readback")?;
        let staging = ComPtr::from_raw(staging, "scene luminance readback staging")?;

        let context = gfx.context.as_raw();
        (*context).CopyResource(staging.as_raw() as *mut _, source as *mut _);
        let mut mapped: D3D11_MAPPED_SUBRESOURCE = mem::zeroed();
        let hr = (*context).Map(
            staging.as_raw() as *mut _,
            0,
            D3D11_MAP_READ,
            0,
            &mut mapped,
        );
        hr_ok(hr, "Map staging scene for luminance readback")?;
        let row_pitch = mapped.RowPitch as usize;
        let byte_len = row_pitch
            .checked_mul(height as usize)
            .ok_or("scene luminance readback size overflow")?;
        let raw = std::slice::from_raw_parts(mapped.pData as *const u8, byte_len);
        // Reduce through the ported chain; copy out of the mapped memory before
        // Unmap so no borrow of GPU memory outlives the mapping.
        let reduction = scene_average_luminance(
            raw,
            width as usize,
            height as usize,
            row_pitch,
            format,
            bounds,
        );
        (*context).Unmap(staging.as_raw() as *mut _, 0);
        let reduction = reduction?;

        // Rebuild the exposure vector: keep MiddleGrey (.x) and the LUT/exposure
        // gate (.z), swap in the measured, R16-quantized AverageLuminance (.y).
        let mut constants = self.restore_postfx.constants;
        constants.exposure[0] = self.restore_postfx.middle_grey;
        constants.exposure[1] = reduction.r16_quantized;
        constants.exposure[2] = 1.0;

        let mut mapped: D3D11_MAPPED_SUBRESOURCE = mem::zeroed();
        let hr = (*context).Map(
            cb.as_raw() as *mut _,
            0,
            D3D11_MAP_WRITE_DISCARD,
            0,
            &mut mapped,
        );
        hr_ok(hr, "Map restore-postfx CB to update measured exposure")?;
        ptr::copy_nonoverlapping(
            &constants as *const RestorePostfxConstants as *const u8,
            mapped.pData as *mut u8,
            mem::size_of::<RestorePostfxConstants>(),
        );
        (*context).Unmap(cb.as_raw() as *mut _, 0);
        eprintln!(
            "replay_app: per-frame exposure adaptation: scene geo-mean luminance {:.6}, clamp[{:.3},{:.3}] -> AvgLum {:.6} (R16 {:#06x}, {}); exposure scale MiddleGrey/AvgLum = {:.4}",
            reduction.geometric_mean_luminance,
            bounds.min_hdr_adjustment,
            bounds.max_hdr_adjustment,
            reduction.r16_quantized,
            reduction.r16_bits,
            if reduction.clamped_to_bound {
                "clamped"
            } else {
                "adapting (unclamped)"
            },
            self.restore_postfx.middle_grey / reduction.r16_quantized,
        );
        Ok(reduction)
    }

    /// COLOR-lane debug readback (`--dump-hdr`): write the rendered HDR
    /// MainScene — the captured-pass output target (RGBA16F at the capture's
    /// native dimensions, BEFORE the composite/postfx) — verbatim to `path`,
    /// with a `<path>.json` sidecar recording width/height/format/row bytes.
    /// This is the byte-level artifact the engine post-pass RT referees
    /// (e.g. session_POLCOLOR_1's borders/mapname post-pass render targets,
    /// same 1920x1080 RGBA16F) diff against. Never a render input.
    fn dump_hdr_mainscene(&self, gfx: &Gfx, path: &std::path::Path) -> Result<()> {
        let width = self.prepared.target_width();
        let height = self.prepared.target_height();
        let format = self.prepared.target_format();
        // The engine MainScene is R16G16B16A16_FLOAT (DXGI format 10). Fail
        // closed on anything else so the sidecar's byte layout can never lie.
        const DXGI_FORMAT_R16G16B16A16_FLOAT: u32 = 10;
        if format != DXGI_FORMAT_R16G16B16A16_FLOAT {
            return Err(format!(
                "--dump-hdr: captured-pass output format {format} is not R16G16B16A16_FLOAT (10)"
            )
            .into());
        }
        let row_bytes = width as usize * 8;
        let mut tight = vec![0u8; row_bytes * height as usize];
        unsafe {
            let staging_desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: format,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ,
                MiscFlags: 0,
            };
            let mut staging: *mut ID3D11Texture2D = ptr::null_mut();
            let hr =
                (*gfx.device.as_raw()).CreateTexture2D(&staging_desc, ptr::null(), &mut staging);
            hr_ok(hr, "CreateTexture2D staging for HDR MainScene dump")?;
            let staging = ComPtr::from_raw(staging, "HDR MainScene dump staging")?;
            let context = gfx.context.as_raw();
            (*context).CopyResource(
                staging.as_raw() as *mut _,
                self.prepared.output_texture_raw() as *mut _,
            );
            let mut mapped: D3D11_MAPPED_SUBRESOURCE = mem::zeroed();
            let hr = (*context).Map(
                staging.as_raw() as *mut _,
                0,
                D3D11_MAP_READ,
                0,
                &mut mapped,
            );
            hr_ok(hr, "Map staging for HDR MainScene dump")?;
            let row_pitch = mapped.RowPitch as usize;
            if row_pitch < row_bytes {
                (*context).Unmap(staging.as_raw() as *mut _, 0);
                return Err(format!(
                    "--dump-hdr: mapped row pitch {row_pitch} is smaller than row bytes {row_bytes}"
                )
                .into());
            }
            let raw =
                std::slice::from_raw_parts(mapped.pData as *const u8, row_pitch * height as usize);
            for row in 0..height as usize {
                tight[row * row_bytes..(row + 1) * row_bytes]
                    .copy_from_slice(&raw[row * row_pitch..row * row_pitch + row_bytes]);
            }
            (*context).Unmap(staging.as_raw() as *mut _, 0);
        }
        std::fs::write(path, &tight)
            .map_err(|err| format!("--dump-hdr: write {}: {err}", path.display()))?;
        let sidecar = format!(
            "{{\"width\":{width},\"height\":{height},\"dxgi_format\":{format},\"row_bytes\":{row_bytes}}}"
        );
        let sidecar_path = path.with_extension(format!(
            "{}.json",
            path.extension()
                .map(|ext| ext.to_string_lossy().into_owned())
                .unwrap_or_default()
        ));
        std::fs::write(&sidecar_path, sidecar)
            .map_err(|err| format!("--dump-hdr: write {}: {err}", sidecar_path.display()))?;
        println!(
            "wrote HDR MainScene dump: {} ({width}x{height} RGBA16F, {} bytes) + sidecar {}",
            path.display(),
            tight.len(),
            sidecar_path.display()
        );
        Ok(())
    }

    fn screen_to_map(
        &self,
        camera: &Camera,
        screen_x: f64,
        screen_y: f64,
        screen_w: f64,
        screen_h: f64,
    ) -> Result<Option<(f64, f64)>> {
        let production_camera = production_camera_from_app_camera(
            self.camera_projection,
            self.camera_defines,
            camera,
            screen_w.max(1.0) as u32,
        )?;
        let map_h = camera.map_h();
        Ok(production_camera
            .screen_to_ground_y0(
                screen_x as f32,
                screen_y as f32,
                screen_w.max(1.0) as f32,
                screen_h.max(1.0) as f32,
            )?
            // Ground z (north-up) -> bmp map pixel y (south-down), the space
            // provinces.bmp picking indexes.
            .map(|(x, z)| (f64::from(x), map_h - f64::from(z))))
    }

    /// The production 3D camera's world HEIGHT for this view — the real
    /// engine-convention camera distance the shipped zoom-cutoff defines
    /// compare against (clamped into `CAMERA_MIN/MAX_HEIGHT` by the same
    /// `height_for_screen_zoom` law the terrain render uses). Replaces the
    /// visible-world-height stand-in, which underestimates the distance by
    /// the projection's FOV factor.
    fn production_camera_height(&self, camera: &Camera, screen_w: f64) -> Result<f32> {
        let production_camera = production_camera_from_app_camera(
            self.camera_projection,
            self.camera_defines,
            camera,
            screen_w.max(1.0) as u32,
        )?;
        Ok(production_camera.position[1])
    }

    /// Forward projection through the SAME production camera the 3D terrain
    /// renders with: map pixel (x, y-down) at `elevation` world units ->
    /// window pixels. The inverse of [`Self::screen_to_map`] extended with
    /// elevation, so overlay elements projected here sit ON the rendered
    /// terrain (the tilted perspective camera displaces elevated geometry vs
    /// the flat 2D map mapping — visibly at close zooms).
    fn map_to_screen_3d(
        &self,
        camera: &Camera,
        map_x: f64,
        map_y: f64,
        elevation: f32,
        screen_w: f64,
        screen_h: f64,
    ) -> Result<Option<(f64, f64)>> {
        self.map_to_screen_3d_at(camera, map_x, map_y, elevation, screen_w, screen_w, screen_h)
    }

    /// LABHDR1 projection split: the production camera MUST derive from the
    /// same `cam_screen_w` the scene render used (the app window width — the
    /// `height_for_screen_zoom` law input), while overlays drawn INTO the
    /// internal HDR MainScene project at the scene target's own dimensions.
    /// The captured VP is 16:9 and the scene target is 16:9, so projecting
    /// the same clip coords at the target dims is exact.
    fn map_to_screen_3d_at(
        &self,
        camera: &Camera,
        map_x: f64,
        map_y: f64,
        elevation: f32,
        cam_screen_w: f64,
        out_w: f64,
        out_h: f64,
    ) -> Result<Option<(f64, f64)>> {
        let production_camera = production_camera_from_app_camera(
            self.camera_projection,
            self.camera_defines,
            camera,
            cam_screen_w.max(1.0) as u32,
        )?;
        // bmp map pixel y (south-down) -> ground z (north-up), matching the
        // terrain world space (same flip as production_camera_from_app_camera).
        let ground_z = camera.map_h() as f32 - map_y as f32;
        Ok(production_camera
            .world_to_screen(
                [map_x as f32, elevation, ground_z],
                out_w.max(1.0) as f32,
                out_h.max(1.0) as f32,
            )
            .map(|(sx, sy)| (f64::from(sx), f64::from(sy))))
    }

    fn stats(&self) -> CapturedPassStats {
        CapturedPassStats {
            session_dir: self.session_dir.clone(),
            target_width: self.prepared.target_width(),
            target_height: self.prepared.target_height(),
            target_format: self.prepared.target_format(),
            has_water: self.prepared.has_water_pass(),
            has_rivers: self.prepared.has_rivers_pass(),
            has_trees: self.prepared.has_trees_pass(),
            substituted_secondary: self.substitution.province_secondary_color_map,
            substituted_gb1: self.substitution.gradient_border_channel1,
            substituted_gb2: self.substitution.gradient_border_channel2,
            synthesized_tile_size: self.prepared.synthesized_tile_size(),
            synthesized_tile_count: self.prepared.synthesized_tile_count(),
            last_render_ms: self
                .last_render
                .map(|duration| duration.as_secs_f64() * 1000.0),
            last_day_night_hour: self.last_camera.and_then(|key| key.day_night_hour),
            postfx_enabled: self.postfx_enabled,
            postfx_restore_full: self.restore_postfx.enabled,
            restore_evidence_found: self.restore_postfx.evidence_found,
            trees_from_files: self.trees_from_files,
            water_gap: self.water_gap.clone(),
            layer_gaps: self.layer_gaps.clone(),
        }
    }
}

pub struct MapRenderer {
    vs: ComPtr<ID3D11VertexShader>,
    ps: ComPtr<ID3D11PixelShader>,
    _province_texture: ComPtr<ID3D11Texture2D>,
    province_srv: ComPtr<ID3D11ShaderResourceView>,
    _visual_texture: ComPtr<ID3D11Texture2D>,
    visual_srv: ComPtr<ID3D11ShaderResourceView>,
    _province_state_buffer: ComPtr<ID3D11Buffer>,
    province_state_srv: ComPtr<ID3D11ShaderResourceView>,
    state_color_buffer: ComPtr<ID3D11Buffer>,
    state_color_srv: ComPtr<ID3D11ShaderResourceView>,
    gb_field: GbField,
    secondary_field: SecondaryColorField,
    captured: Option<CapturedPassLayer>,
    buildings: Option<BuildingPreviewLayer>,
    gb_sampler: ComPtr<ID3D11SamplerState>,
    camera_cb: ComPtr<ID3D11Buffer>,
    restore_postfx_cb: Option<ComPtr<ID3D11Buffer>>,
    restore_color_cube: Option<RestoreColorCubeGpu>,
    state_color_len: usize,
    map_w: f32,
    map_h: f32,
    /// COL3 political scene defines (resolver-read, mod-aware; vanilla
    /// fallback without a `--game` resolver). Only the two GB outline cutoffs
    /// are consumed on the Path B synthesized shader — fed into the CameraCB so
    /// the border outline cutoff matches the engine + the Path A CB-write path
    /// instead of the old hardcoded 0.9999.
    political_defines: crate::gb_field::PoliticalSceneDefines,
}

impl MapRenderer {
    /// Upload the static map data once and compile the shaders.
    /// `max_state_id` must cover every state id any timeline hour can
    /// reference so scrub updates never need to reallocate the table.
    pub fn new(
        gfx: &Gfx,
        bundle_dir: &Path,
        bundle: &MapBundle,
        max_state_id: u32,
        capture_dir: Option<&Path>,
        game_resolver: Option<&GameFileResolver>,
        synthesize_terrain_chunks: bool,
        capture_layers: CapturedLayerConfig,
        postfx_enabled: bool,
        postfx_restore_full: bool,
        postfx_restore_full_explicit: bool,
        tree_seasons_by_date: bool,
        trees_from_files: bool,
        no_trees_second_subdraw: bool,
        buildings_from_files: bool,
        buildings_preview_all: bool,
        water_full: bool,
        water_full_explicit: bool,
        borders_from_files: bool,
        borders_from_files_explicit: bool,
        chunks_from_files: bool,
    ) -> Result<MapRenderer> {
        unsafe {
            // Files-first: the grade LUT + constants are composed from the
            // shipped installed content via the resolver (mod-aware). The captured
            // bundle, if present, is consumed only by the
            // `--postfx-validate-capture` regression path, never as a render
            // input. The full-map dump is a day, above-all-volumes view.
            //
            // GRAD1 fail-open policy: the files-first grade is DEFAULT ON.
            //
            // The ONLY missing-evidence condition the default path downgrades on
            // is a MISSING RESOLVER (no `--game`/BATTLE_PLAN_VANILLA) — the "resolver
            // failure" case the spec names. The files-first grade reads only
            // installed content through the resolver (never the capture bundle), so an
            // absent resolver is the sole "inputs are missing" signal.
            // Deliberately NARROW (Codex-refuted over-breadth): if a resolver IS
            // present but the composition then fails (a missing / corrupt
            // `colorcorrection_*.tga`, a malformed `posteffect_volumes.txt`, a
            // bad bloom/define key), that is a broken game/mod install — a REAL
            // fault that MUST surface on BOTH the explicit and the default path
            // (user honesty mandate: never silently render a wrong grade). Only
            // the resolver-absent case is a "recording without a game install"
            // that must still render; a resolver-present composition error is
            // not fail-open.
            //
            // - Explicit `--postfx-restore-full`: any error (incl. no resolver)
            //   is a HARD ERROR (fail closed).
            // - Default (no flag), no resolver: downgrade to the old
            //   capture-anchored path (identity grade) with a one-line notice.
            // - Default (no flag), resolver present but composition fails: HARD
            //   ERROR (a real data fault, surfaced honestly).
            // - `--no-postfx-restore-full`: `postfx_restore_full == false`, so
            //   `restore_full_postfx_from_files` returns identity and never errors.
            let restore_postfx = if postfx_restore_full
                && !postfx_restore_full_explicit
                && game_resolver.is_none()
            {
                // Default-ON without a resolver: fail open to the old default.
                eprintln!(
                    "replay_app: NOTICE: --postfx-restore-full is default-ON but no --game resolver is available to compose the files-first grade; downgrading to the old capture-anchored default (no files-first grade). Pass --postfx-restore-full to require it (fail closed) or --no-postfx-restore-full to silence this."
                );
                restore_full_postfx_from_files(
                    false,
                    game_resolver,
                    bundle.width,
                    bundle.height,
                    None,
                )?
            } else {
                // Explicit request, resolver present, or grade disabled: compose
                // normally and let any composition error surface (fail closed).
                restore_full_postfx_from_files(
                    postfx_restore_full,
                    game_resolver,
                    bundle.width,
                    bundle.height,
                    None,
                )?
            };
            let map_shader_source = shader_source_with_restore_postfx(
                MAP_SHADER_HLSL,
                postfx_enabled,
                restore_postfx.enabled,
                // The map shader has no bloom composite hook (bloom rides only
                // the captured-pass compositor that samples the HDR MainScene).
                false,
            );
            let vs_blob = compile_shader(&map_shader_source, "vs_main", "vs_4_0")?;
            let ps_blob = compile_shader(&map_shader_source, "ps_main", "ps_4_0")?;

            let mut vs: *mut ID3D11VertexShader = ptr::null_mut();
            let hr = (*gfx.device.as_raw()).CreateVertexShader(
                (*vs_blob.as_raw()).GetBufferPointer(),
                (*vs_blob.as_raw()).GetBufferSize(),
                ptr::null_mut(),
                &mut vs,
            );
            hr_ok(hr, "CreateVertexShader for the map shader")?;
            let vs = ComPtr::from_raw(vs, "map vertex shader")?;

            let mut ps: *mut ID3D11PixelShader = ptr::null_mut();
            let hr = (*gfx.device.as_raw()).CreatePixelShader(
                (*ps_blob.as_raw()).GetBufferPointer(),
                (*ps_blob.as_raw()).GetBufferSize(),
                ptr::null_mut(),
                &mut ps,
            );
            hr_ok(hr, "CreatePixelShader for the map shader")?;
            let ps = ComPtr::from_raw(ps, "map pixel shader")?;

            // Province ids: R32_UINT, uploaded once, integer Load only.
            let (province_texture, province_srv) = create_immutable_texture(
                gfx,
                bundle.width,
                bundle.height,
                DXGI_FORMAT_R32_UINT,
                province_plane_bytes(&bundle.province_ids).as_slice(),
                "province id texture",
            )?;

            // Grounded underlay: map_visual.rgbx 0x00RRGGBB -> RGBA8.
            let (visual_texture, visual_srv) = create_immutable_texture(
                gfx,
                bundle.width,
                bundle.height,
                DXGI_FORMAT_R8G8B8A8_UNORM,
                visual_rgba_bytes(&bundle.map_visual).as_slice(),
                "map visual underlay texture",
            )?;

            // Province -> state: static typed buffer indexed by province id.
            // Index 0 and unmapped provinces stay 0 = "no state" sentinel.
            let mut province_state = vec![0u32; bundle.max_mapped_province_id() as usize + 1];
            for (province_id, state_id) in &bundle.province_to_state {
                province_state[*province_id as usize] = *state_id;
            }
            let (province_state_buffer, province_state_srv) = create_immutable_typed_buffer(
                gfx,
                &province_state,
                DXGI_FORMAT_R32_UINT,
                "province->state lookup buffer",
            )?;

            // State -> RGBA color: compact table kept for picking parity and
            // future overlays; gradient channels own the visible political fill.
            let state_color_len = max_state_id as usize + 1;
            let initial_colors = vec![crate::palette::NO_OWNER_SENTINEL_RGBA; state_color_len];
            let (state_color_buffer, state_color_srv) = create_dynamic_typed_buffer(
                gfx,
                &initial_colors,
                DXGI_FORMAT_R8G8B8A8_UNORM,
                "state->color lookup buffer",
            )?;

            // All gradient-border input content is generated from vanilla
            // bundle data (land mask + country layer); the sea-zone layer
            // stays inert like the game's default political view. The country
            // layer's per-country thickness comes from the mod-resolved
            // 00_graphics.lua GRADIENT_BORDERS_*_COUNTRY_* defines when a game
            // resolver is present (files-first / mod-aware); vanilla defines
            // otherwise.
            let gb_field = GbField::new(gfx, bundle, game_resolver)?;
            // COL3: the political GB outline cutoffs, resolver-read (mod-aware)
            // with the vanilla 0.98/0.973 fallback — the SAME source and
            // fail-open policy as the Path A CB-write path (see line ~1504) and
            // GbField's own defines. Fed into the Path B CameraCB so the border
            // outline cutoff is the shipped define, not the stale 0.9999.
            let political_defines = match game_resolver {
                Some(resolver) => crate::gb_field::PoliticalSceneDefines::from_resolver(resolver)?,
                None => crate::gb_field::PoliticalSceneDefines::vanilla(),
            };
            let secondary_field = SecondaryColorField::new(gfx, bundle)?;
            // GRAD1 water fail-open (Codex-refuted completeness): the resolver
            // check inside CapturedPassLayer::new only covers a MISSING resolver.
            // A recording captured WITHOUT the underwater-pass inputs (e.g. the
            // terrain draws do not bind the PixelShaderUnderwater SRV slots, as
            // in restore_gate_1) makes the default-ON underwater pass hard-error
            // deeper in the prepare — that is the water analogue of "restore
            // bundle absent" the spec's fail-open covers. So on the DEFAULT (no
            // flag) path, if constructing the captured pass with water_full=true
            // fails, retry ONCE with water_full=false. A water-INDEPENDENT fault
            // (terrain load, D3D, secondary/GB inputs) reproduces on the retry
            // because none of that work depends on water_full, so it re-surfaces
            // honestly; only a water-specific failure is cured. An EXPLICIT
            // --water-full never retries (fail closed).
            let build_captured = |capture_dir: &Path, water_full: bool| {
                CapturedPassLayer::new(
                    gfx,
                    bundle_dir,
                    capture_dir,
                    game_resolver,
                    bundle.width,
                    bundle.height,
                    secondary_field.srv(),
                    gb_field.channel1_srv(),
                    gb_field.channel2_srv(),
                    synthesize_terrain_chunks,
                    capture_layers,
                    postfx_enabled,
                    restore_postfx.clone(),
                    tree_seasons_by_date,
                    trees_from_files,
                    no_trees_second_subdraw,
                    water_full,
                    water_full_explicit,
                    borders_from_files,
                    borders_from_files_explicit,
                    chunks_from_files,
                )
            };
            let captured = match capture_dir {
                None => None,
                Some(capture_dir) => match build_captured(capture_dir, water_full) {
                    Ok(layer) => Some(layer),
                    Err(err) if water_full && !water_full_explicit => {
                        eprintln!(
                            "replay_app: NOTICE: --water-full is default-ON but the captured pass could not be built with the underwater refraction pass ({err}); retrying with the old capture-anchored water default (approximation). Pass --water-full to require it (fail closed) or --no-water-full to silence this."
                        );
                        // Retry with water off; a water-independent fault
                        // reproduces here and surfaces via `?`.
                        Some(build_captured(capture_dir, false)?)
                    }
                    Err(err) => return Err(err),
                },
            };
            let buildings = if buildings_from_files {
                let resolver = game_resolver.ok_or(
                    "--buildings-from-files requires --game <the game install dir> or BATTLE_PLAN_VANILLA",
                )?;
                Some(BuildingPreviewLayer::new(
                    gfx,
                    resolver,
                    buildings_preview_all,
                    bundle.width,
                    bundle.height,
                )?)
            } else {
                None
            };
            let gb_sampler = create_linear_clamp_sampler(gfx)?;
            let camera_cb = create_camera_constant_buffer(gfx)?;
            let restore_postfx_cb = if restore_postfx.enabled {
                Some(create_restore_postfx_constant_buffer(
                    gfx,
                    restore_postfx.constants,
                )?)
            } else {
                None
            };
            let restore_color_cube = create_restore_color_cube_texture(gfx, &restore_postfx)?;

            Ok(MapRenderer {
                vs,
                ps,
                _province_texture: province_texture,
                province_srv,
                _visual_texture: visual_texture,
                visual_srv,
                _province_state_buffer: province_state_buffer,
                province_state_srv,
                state_color_buffer,
                state_color_srv,
                gb_field,
                secondary_field,
                captured,
                buildings,
                gb_sampler,
                camera_cb,
                restore_postfx_cb,
                restore_color_cube,
                state_color_len,
                map_w: bundle.width as f32,
                map_h: bundle.height as f32,
                political_defines,
            })
        }
    }

    /// Scrub step: rewrite the compact state -> color table from controllers,
    /// rebuild controller-colored gradient borders, and rebuild owner-side
    /// occupation stripes only when each generated texture's signature changes.
    pub fn update_state_politics(
        &mut self,
        gfx: &Gfx,
        owners: &BTreeMap<u32, String>,
        controllers: &BTreeMap<u32, String>,
        country_colors: &BTreeMap<String, [u8; 3]>,
    ) -> Result<()> {
        // SEMANTIC-ASSUMPTION: the political base color follows controller.
        // If a live occupied-save shader capture contradicts this, shader
        // evidence wins and this provider wiring must change.
        let table = crate::palette::build_state_color_table(
            (self.state_color_len - 1) as u32,
            controllers,
            country_colors,
        );
        self.update_state_color_table(gfx, &table)?;
        let stats = self.gb_field.update(gfx, controllers, country_colors)?;
        if stats.rebuilt {
            if let Some(captured) = self.captured.as_mut() {
                captured.mark_dirty();
            }
            eprintln!(
                "replay_app: rebuilt gradient-border country layer in {:.2} ms",
                stats.elapsed.as_secs_f64() * 1000.0
            );
        }
        let secondary_stats =
            self.secondary_field
                .update(gfx, owners, controllers, country_colors)?;
        if secondary_stats.rebuilt {
            if let Some(captured) = self.captured.as_mut() {
                captured.mark_dirty();
            }
            eprintln!(
                "replay_app: rebuilt ProvinceSecondaryColorMap in {:.2} ms ({} occupied half-res cells)",
                secondary_stats.elapsed.as_secs_f64() * 1000.0,
                secondary_stats.occupied_cells
            );
        }
        Ok(())
    }

    fn update_state_color_table(&self, gfx: &Gfx, table: &[u32]) -> Result<()> {
        if table.len() != self.state_color_len {
            return Err(format!(
                "state color table has {} entries, renderer allocated {}",
                table.len(),
                self.state_color_len
            )
            .into());
        }
        unsafe {
            let mut mapped: D3D11_MAPPED_SUBRESOURCE = mem::zeroed();
            let hr = (*gfx.context.as_raw()).Map(
                self.state_color_buffer.as_raw() as *mut _,
                0,
                D3D11_MAP_WRITE_DISCARD,
                0,
                &mut mapped,
            );
            hr_ok(hr, "Map state->color buffer for scrub update")?;
            ptr::copy_nonoverlapping(
                table.as_ptr() as *const u8,
                mapped.pData as *mut u8,
                table.len() * 4,
            );
            (*gfx.context.as_raw()).Unmap(self.state_color_buffer.as_raw() as *mut _, 0);
        }
        Ok(())
    }

    /// Draw the map for the current camera into the backbuffer.
    /// `day_night_hour = None` (the default view) keeps the captured
    /// day/night state: terminator shading off, like the game.
    pub fn draw(
        &mut self,
        gfx: &Gfx,
        camera: &Camera,
        day_night_hour: Option<f32>,
        tree_season_date: Option<(u32, u32)>,
        map_clock_time: Option<f32>,
    ) -> Result<()> {
        if let Some(hour) = day_night_hour {
            validate_day_night_hour(hour)?;
        }
        if let Some(captured) = self.captured.as_mut() {
            captured.render(
                gfx,
                camera,
                day_night_hour,
                tree_season_date,
                map_clock_time,
            )?;
            if let Some(buildings) = &self.buildings {
                buildings.draw(gfx, camera)?;
            }
            return Ok(());
        }
        unsafe {
            let constants = CameraConstants {
                screen_size: [gfx.width as f32, gfx.height as f32],
                center_map: [camera.center_x as f32, camera.center_y as f32],
                zoom: camera.zoom as f32,
                map_w: self.map_w,
                map_h: self.map_h,
                camera_height: gradient_border_camera_height(
                    camera.zoom,
                    f64::from(gfx.width),
                    f64::from(gfx.height),
                    f64::from(self.map_w),
                    f64::from(self.map_h),
                ),
                // Packed (strategic, country) to match the shader's
                // gb_outline_cutoff and the engine's vOutlineCutoff.x/.y
                // (pdxmap.shader:325 passes CB0 .zw = (strategic, country)).
                gb_outline_cutoff: [
                    self.political_defines.outline_cutoff_strategic,
                    self.political_defines.outline_cutoff_country,
                ],
                camera_cb_pad: [0.0, 0.0],
            };
            let mut mapped: D3D11_MAPPED_SUBRESOURCE = mem::zeroed();
            let hr = (*gfx.context.as_raw()).Map(
                self.camera_cb.as_raw() as *mut _,
                0,
                D3D11_MAP_WRITE_DISCARD,
                0,
                &mut mapped,
            );
            hr_ok(hr, "Map camera constant buffer")?;
            ptr::copy_nonoverlapping(
                &constants as *const CameraConstants as *const u8,
                mapped.pData as *mut u8,
                mem::size_of::<CameraConstants>(),
            );
            (*gfx.context.as_raw()).Unmap(self.camera_cb.as_raw() as *mut _, 0);

            let context = gfx.context.as_raw();
            let rtv = gfx.rtv()?.as_raw();
            // PROXY: clear color is not game-file-derived; it is only
            // visible in the letterbox outside the map (see shader note).
            let clear = [0.0f32, 0.0, 0.0, 1.0];
            (*context).ClearRenderTargetView(rtv, &clear);
            let rtvs = [rtv];
            (*context).OMSetRenderTargets(1, rtvs.as_ptr(), ptr::null_mut());
            let viewport = D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: gfx.width as f32,
                Height: gfx.height as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            };
            (*context).RSSetViewports(1, &viewport);
            (*context).IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            (*context).IASetInputLayout(ptr::null_mut());
            (*context).VSSetShader(self.vs.as_raw(), ptr::null(), 0);
            (*context).PSSetShader(self.ps.as_raw(), ptr::null(), 0);
            let constant_buffers = [self.camera_cb.as_raw()];
            (*context).PSSetConstantBuffers(0, 1, constant_buffers.as_ptr());
            if let Some(constants) = self.restore_postfx_cb.as_ref() {
                let constant_buffers = [constants.as_raw()];
                (*context).PSSetConstantBuffers(1, 1, constant_buffers.as_ptr());
            }
            let srvs = [
                self.province_srv.as_raw(),
                self.visual_srv.as_raw(),
                self.province_state_srv.as_raw(),
                self.state_color_srv.as_raw(),
                self.gb_field.channel1_srv(),
                self.gb_field.channel2_srv(),
                self.secondary_field.srv(),
            ];
            (*context).PSSetShaderResources(0, srvs.len() as UINT, srvs.as_ptr());
            if let Some(color_cube) = self.restore_color_cube.as_ref() {
                let srvs = [color_cube.srv.as_raw()];
                (*context).PSSetShaderResources(7, 1, srvs.as_ptr());
            }
            let samplers = [self.gb_sampler.as_raw()];
            (*context).PSSetSamplers(0, 1, samplers.as_ptr());
            (*context).Draw(3, 0);
        }
        if let Some(buildings) = &self.buildings {
            buildings.draw(gfx, camera)?;
        }
        Ok(())
    }

    pub fn screen_to_map(
        &self,
        camera: &Camera,
        screen_x: f64,
        screen_y: f64,
        screen_w: f64,
        screen_h: f64,
    ) -> Result<Option<(f64, f64)>> {
        if let Some(captured) = self.captured.as_ref() {
            captured.screen_to_map(camera, screen_x, screen_y, screen_w, screen_h)
        } else {
            Ok(Some(
                camera.screen_to_map(screen_x, screen_y, screen_w, screen_h),
            ))
        }
    }

    /// The production 3D camera height for this view (the engine-convention
    /// camera distance the shipped zoom cutoffs compare against), or `None`
    /// without a captured 3D pass — callers fall back to the visible-height
    /// stand-in.
    pub fn production_camera_height(&self, camera: &Camera, screen_w: f64) -> Result<Option<f32>> {
        match self.captured.as_ref() {
            Some(captured) => captured
                .production_camera_height(camera, screen_w)
                .map(Some),
            None => Ok(None),
        }
    }

    /// Forward-project a map point at `elevation` through the production 3D
    /// camera the captured terrain pass renders with (window pixels, y-down).
    /// `Ok(None)` when there is no captured 3D pass (plain 2D bundle render —
    /// there the flat `Camera::map_to_screen` IS the render mapping) or the
    /// point is behind the camera; callers fall back to the 2D mapping.
    pub fn map_to_screen_3d(
        &self,
        camera: &Camera,
        map_x: f64,
        map_y: f64,
        elevation: f32,
        screen_w: f64,
        screen_h: f64,
    ) -> Result<Option<(f64, f64)>> {
        match self.captured.as_ref() {
            Some(captured) => {
                captured.map_to_screen_3d(camera, map_x, map_y, elevation, screen_w, screen_h)
            }
            None => Ok(None),
        }
    }

    /// LABHDR1 projection split (see `CapturedPassLayer::map_to_screen_3d_at`):
    /// camera derived at `cam_screen_w` (the scene render's window width),
    /// projected at the internal HDR scene target's dimensions.
    pub fn map_to_screen_3d_at(
        &self,
        camera: &Camera,
        map_x: f64,
        map_y: f64,
        elevation: f32,
        cam_screen_w: f64,
        out_w: f64,
        out_h: f64,
    ) -> Result<Option<(f64, f64)>> {
        match self.captured.as_ref() {
            Some(captured) => captured
                .map_to_screen_3d_at(camera, map_x, map_y, elevation, cam_screen_w, out_w, out_h),
            None => Ok(None),
        }
    }

    /// LABHDR1: scene-phase draw (no final composite). Returns `true` when a
    /// captured/synthesized HDR scene exists (overlays may be drawn into it
    /// via [`Self::bind_scene_target`], then [`Self::composite_scene`]);
    /// `false` = plain 2D path where `draw()` semantics are unchanged and
    /// overlays belong on the final target.
    pub fn draw_scene(
        &mut self,
        gfx: &Gfx,
        camera: &Camera,
        day_night_hour: Option<f32>,
        tree_season_date: Option<(u32, u32)>,
        map_clock_time: Option<f32>,
    ) -> Result<bool> {
        if let Some(hour) = day_night_hour {
            validate_day_night_hour(hour)?;
        }
        if let Some(captured) = self.captured.as_mut() {
            captured.render_scene_only(gfx, camera, day_night_hour, tree_season_date, map_clock_time)?;
            return Ok(true);
        }
        self.draw(gfx, camera, day_night_hour, tree_season_date, map_clock_time)?;
        Ok(false)
    }

    /// LABHDR1: composite the overlay-augmented HDR MainScene to the final
    /// target, then run the post-composite layers `draw()` would have run.
    pub fn composite_scene(&mut self, gfx: &Gfx, camera: &Camera) -> Result<()> {
        if let Some(captured) = self.captured.as_ref() {
            captured.composite_only(gfx)?;
            if let Some(buildings) = &self.buildings {
                buildings.draw(gfx, camera)?;
            }
        }
        Ok(())
    }

    /// LABHDR1: the HDR MainScene overlay target (raw RTV + dimensions) for
    /// in-scene overlays. Caller passes it to the label renderer, then calls
    /// [`Self::composite_scene`].
    pub fn scene_overlay_target(
        &mut self,
        gfx: &Gfx,
    ) -> Result<(*mut ID3D11RenderTargetView, u32, u32)> {
        match self.captured.as_mut() {
            Some(captured) => unsafe { captured.scene_overlay_target(gfx) },
            None => Err("scene_overlay_target requires the captured/synthesized pass".into()),
        }
    }

    pub fn captured_pass_stats(&self) -> Option<CapturedPassStats> {
        self.captured.as_ref().map(CapturedPassLayer::stats)
    }

    /// COLOR-lane debug readback (`--dump-hdr`): see
    /// [`CapturedPassLayer::dump_hdr_mainscene`]. Fails closed when no
    /// captured/synthesized pass is active (there is no HDR MainScene without
    /// one).
    pub fn dump_hdr_mainscene(&self, gfx: &Gfx, path: &std::path::Path) -> Result<()> {
        match self.captured.as_ref() {
            Some(captured) => captured.dump_hdr_mainscene(gfx, path),
            None => Err(
                "--dump-hdr requires the captured/synthesized pass (--capture <session dir>)"
                    .into(),
            ),
        }
    }

    pub fn building_preview_stats(&self) -> Option<&BuildingFileDerivedLayerStats> {
        self.buildings.as_ref().map(BuildingPreviewLayer::stats)
    }
}

fn province_plane_bytes(province_ids: &[u32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(province_ids.len() * 4);
    for id in province_ids {
        bytes.extend_from_slice(&id.to_le_bytes());
    }
    bytes
}

fn visual_rgba_bytes(visual: &[u32]) -> Vec<u8> {
    // map_visual.rgbx stores 0x00RRGGBB; expand to RGBA8 (opaque).
    let mut bytes = Vec::with_capacity(visual.len() * 4);
    for pixel in visual {
        bytes.push((pixel >> 16) as u8);
        bytes.push((pixel >> 8) as u8);
        bytes.push(*pixel as u8);
        bytes.push(0xFF);
    }
    bytes
}

/// Shared with text.rs (the overlay pipeline follows the same patterns).
pub(crate) unsafe fn compile_shader(
    source: &str,
    entry: &str,
    target: &str,
) -> Result<ComPtr<ID3DBlob>> {
    let entry_c = CString::new(entry).map_err(|_| "shader entry point contains NUL")?;
    let target_c = CString::new(target).map_err(|_| "shader target contains NUL")?;
    let name_c = CString::new("replay_app_map_shader.hlsl")
        .map_err(|_| "shader source name contains NUL")?;
    let mut code: *mut ID3DBlob = ptr::null_mut();
    let mut errors: *mut ID3DBlob = ptr::null_mut();
    let hr = D3DCompile(
        source.as_ptr() as *const _,
        source.len(),
        name_c.as_ptr(),
        ptr::null(),
        ptr::null_mut(),
        entry_c.as_ptr(),
        target_c.as_ptr(),
        0,
        0,
        &mut code,
        &mut errors,
    );
    let error_text = if errors.is_null() {
        String::new()
    } else {
        let blob = ComPtr::from_raw(errors, "shader error blob")?;
        let data = (*blob.as_raw()).GetBufferPointer() as *const u8;
        let len = (*blob.as_raw()).GetBufferSize();
        String::from_utf8_lossy(std::slice::from_raw_parts(data, len)).into_owned()
    };
    if hr < 0 {
        return Err(format!(
            "D3DCompile({entry}, {target}) failed with HRESULT 0x{:08X}: {}",
            hr as u32,
            error_text.trim()
        )
        .into());
    }
    ComPtr::from_raw(code, "compiled shader blob")
}

/// Shared with text.rs (glyph atlas pages use the same upload path).
pub(crate) unsafe fn create_immutable_texture(
    gfx: &Gfx,
    width: usize,
    height: usize,
    format: u32,
    bytes: &[u8],
    role: &str,
) -> Result<(ComPtr<ID3D11Texture2D>, ComPtr<ID3D11ShaderResourceView>)> {
    let expected = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or("texture byte length overflow")?;
    if bytes.len() != expected {
        return Err(format!(
            "{role} has {} bytes, expected {expected} for {width}x{height}",
            bytes.len()
        )
        .into());
    }
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width as UINT,
        Height: height as UINT,
        MipLevels: 1,
        ArraySize: 1,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_IMMUTABLE,
        BindFlags: D3D11_BIND_SHADER_RESOURCE,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let init = D3D11_SUBRESOURCE_DATA {
        pSysMem: bytes.as_ptr() as *const _,
        SysMemPitch: (width * 4) as UINT,
        SysMemSlicePitch: 0,
    };
    let mut texture: *mut ID3D11Texture2D = ptr::null_mut();
    let hr = (*gfx.device.as_raw()).CreateTexture2D(&desc, &init, &mut texture);
    hr_ok(hr, &format!("CreateTexture2D for {role}"))?;
    let texture = ComPtr::from_raw(texture, role)?;
    let mut srv: *mut ID3D11ShaderResourceView = ptr::null_mut();
    let hr = (*gfx.device.as_raw()).CreateShaderResourceView(
        texture.as_raw() as *mut _,
        ptr::null(),
        &mut srv,
    );
    hr_ok(hr, &format!("CreateShaderResourceView for {role}"))?;
    let srv = ComPtr::from_raw(srv, role)?;
    Ok((texture, srv))
}

unsafe fn create_typed_buffer_srv(
    gfx: &Gfx,
    buffer: &ComPtr<ID3D11Buffer>,
    format: u32,
    element_count: usize,
    role: &str,
) -> Result<ComPtr<ID3D11ShaderResourceView>> {
    let mut buffer_srv: D3D11_BUFFER_SRV = mem::zeroed();
    *buffer_srv.u1.FirstElement_mut() = 0;
    *buffer_srv.u2.NumElements_mut() = element_count as UINT;
    let mut desc: D3D11_SHADER_RESOURCE_VIEW_DESC = mem::zeroed();
    desc.Format = format;
    desc.ViewDimension = D3D11_SRV_DIMENSION_BUFFER;
    *desc.u.Buffer_mut() = buffer_srv;
    let mut srv: *mut ID3D11ShaderResourceView = ptr::null_mut();
    let hr =
        (*gfx.device.as_raw()).CreateShaderResourceView(buffer.as_raw() as *mut _, &desc, &mut srv);
    hr_ok(hr, &format!("CreateShaderResourceView for {role}"))?;
    ComPtr::from_raw(srv, role)
}

unsafe fn create_immutable_typed_buffer(
    gfx: &Gfx,
    values: &[u32],
    format: u32,
    role: &str,
) -> Result<(ComPtr<ID3D11Buffer>, ComPtr<ID3D11ShaderResourceView>)> {
    if values.is_empty() {
        return Err(format!("{role} cannot be empty").into());
    }
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: (values.len() * 4) as UINT,
        Usage: D3D11_USAGE_IMMUTABLE,
        BindFlags: D3D11_BIND_SHADER_RESOURCE,
        CPUAccessFlags: 0,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let init = D3D11_SUBRESOURCE_DATA {
        pSysMem: values.as_ptr() as *const _,
        SysMemPitch: 0,
        SysMemSlicePitch: 0,
    };
    let mut buffer: *mut ID3D11Buffer = ptr::null_mut();
    let hr = (*gfx.device.as_raw()).CreateBuffer(&desc, &init, &mut buffer);
    hr_ok(hr, &format!("CreateBuffer for {role}"))?;
    let buffer = ComPtr::from_raw(buffer, role)?;
    let srv = create_typed_buffer_srv(gfx, &buffer, format, values.len(), role)?;
    Ok((buffer, srv))
}

unsafe fn create_dynamic_typed_buffer(
    gfx: &Gfx,
    values: &[u32],
    format: u32,
    role: &str,
) -> Result<(ComPtr<ID3D11Buffer>, ComPtr<ID3D11ShaderResourceView>)> {
    if values.is_empty() {
        return Err(format!("{role} cannot be empty").into());
    }
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: (values.len() * 4) as UINT,
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: D3D11_BIND_SHADER_RESOURCE,
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let init = D3D11_SUBRESOURCE_DATA {
        pSysMem: values.as_ptr() as *const _,
        SysMemPitch: 0,
        SysMemSlicePitch: 0,
    };
    let mut buffer: *mut ID3D11Buffer = ptr::null_mut();
    let hr = (*gfx.device.as_raw()).CreateBuffer(&desc, &init, &mut buffer);
    hr_ok(hr, &format!("CreateBuffer for {role}"))?;
    let buffer = ComPtr::from_raw(buffer, role)?;
    let srv = create_typed_buffer_srv(gfx, &buffer, format, values.len(), role)?;
    Ok((buffer, srv))
}

unsafe fn create_linear_clamp_sampler(gfx: &Gfx) -> Result<ComPtr<ID3D11SamplerState>> {
    // `pdxmap.shader:141-157` declares both GradientBorderChannel samplers as
    // Linear/Linear/Linear with Clamp address modes.
    let desc = D3D11_SAMPLER_DESC {
        Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
        AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
        MipLODBias: 0.0,
        MaxAnisotropy: 1,
        ComparisonFunc: D3D11_COMPARISON_ALWAYS,
        BorderColor: [0.0, 0.0, 0.0, 0.0],
        MinLOD: 0.0,
        MaxLOD: f32::MAX,
    };
    let mut sampler: *mut ID3D11SamplerState = ptr::null_mut();
    let hr = (*gfx.device.as_raw()).CreateSamplerState(&desc, &mut sampler);
    hr_ok(hr, "CreateSamplerState for gradient-border channels")?;
    ComPtr::from_raw(sampler, "gradient-border linear clamp sampler")
}

unsafe fn create_camera_constant_buffer(gfx: &Gfx) -> Result<ComPtr<ID3D11Buffer>> {
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: mem::size_of::<CameraConstants>() as UINT,
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: D3D11_BIND_CONSTANT_BUFFER,
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let mut buffer: *mut ID3D11Buffer = ptr::null_mut();
    let hr = (*gfx.device.as_raw()).CreateBuffer(&desc, ptr::null(), &mut buffer);
    hr_ok(hr, "CreateBuffer for camera constants")?;
    ComPtr::from_raw(buffer, "camera constant buffer")
}

unsafe fn create_restore_postfx_constant_buffer(
    gfx: &Gfx,
    constants: RestorePostfxConstants,
) -> Result<ComPtr<ID3D11Buffer>> {
    // DYNAMIC (not IMMUTABLE): the per-frame luminance-adaptation pass rewrites
    // the exposure `AverageLuminance` field each frame (Map WRITE_DISCARD) after
    // reducing the rendered scene through the ported downsample_luminance chain.
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: mem::size_of::<RestorePostfxConstants>() as UINT,
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: D3D11_BIND_CONSTANT_BUFFER,
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let init = D3D11_SUBRESOURCE_DATA {
        pSysMem: &constants as *const RestorePostfxConstants as *const _,
        SysMemPitch: 0,
        SysMemSlicePitch: 0,
    };
    let mut buffer: *mut ID3D11Buffer = ptr::null_mut();
    let hr = (*gfx.device.as_raw()).CreateBuffer(&desc, &init, &mut buffer);
    hr_ok(hr, "CreateBuffer for RestoreScene full-postfx constants")?;
    ComPtr::from_raw(buffer, "RestoreScene full-postfx constant buffer")
}

unsafe fn create_restore_color_cube_texture(
    gfx: &Gfx,
    restore: &RestoreFullPostfx,
) -> Result<Option<RestoreColorCubeGpu>> {
    if !restore.enabled {
        return Ok(None);
    }
    let (texture, srv) = create_immutable_texture(
        gfx,
        1024,
        32,
        DXGI_FORMAT_R8G8B8A8_UNORM,
        &restore.color_cube_rgba8,
        "RestoreScene ColorCube",
    )?;
    Ok(Some(RestoreColorCubeGpu {
        _texture: texture,
        srv,
    }))
}

/// Create a half-res `R16G16B16A16_FLOAT` render target with an RTV and SRV,
/// mirroring the engine's bloom intermediates (`bloom.shader` half-float pool).
unsafe fn create_bloom_render_target(
    gfx: &Gfx,
    width: u32,
    height: u32,
    role: &str,
) -> Result<(
    ComPtr<ID3D11Texture2D>,
    ComPtr<ID3D11RenderTargetView>,
    ComPtr<ID3D11ShaderResourceView>,
)> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_R16G16B16A16_FLOAT,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut texture: *mut ID3D11Texture2D = ptr::null_mut();
    let hr = (*gfx.device.as_raw()).CreateTexture2D(&desc, ptr::null(), &mut texture);
    hr_ok(hr, &format!("CreateTexture2D for {role}"))?;
    let texture = ComPtr::from_raw(texture, role)?;
    let mut rtv: *mut ID3D11RenderTargetView = ptr::null_mut();
    let hr = (*gfx.device.as_raw()).CreateRenderTargetView(
        texture.as_raw() as *mut _,
        ptr::null(),
        &mut rtv,
    );
    hr_ok(hr, &format!("CreateRenderTargetView for {role}"))?;
    let rtv = ComPtr::from_raw(rtv, role)?;
    let mut srv: *mut ID3D11ShaderResourceView = ptr::null_mut();
    let hr = (*gfx.device.as_raw()).CreateShaderResourceView(
        texture.as_raw() as *mut _,
        ptr::null(),
        &mut srv,
    );
    hr_ok(hr, &format!("CreateShaderResourceView for {role}"))?;
    let srv = ComPtr::from_raw(srv, role)?;
    Ok((texture, rtv, srv))
}

/// Upload an immutable constant buffer of a `#[repr(C)]` value.
unsafe fn create_immutable_constant_buffer<T: Copy>(
    gfx: &Gfx,
    value: &T,
    role: &str,
) -> Result<ComPtr<ID3D11Buffer>> {
    // Constant-buffer byte width must be a multiple of 16; our layouts already
    // are (float4-aligned), but assert to catch future edits.
    let size = mem::size_of::<T>();
    if size % 16 != 0 {
        return Err(format!("{role} constant buffer size {size} is not 16-byte aligned").into());
    }
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: size as UINT,
        Usage: D3D11_USAGE_IMMUTABLE,
        BindFlags: D3D11_BIND_CONSTANT_BUFFER,
        CPUAccessFlags: 0,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let init = D3D11_SUBRESOURCE_DATA {
        pSysMem: value as *const T as *const _,
        SysMemPitch: 0,
        SysMemSlicePitch: 0,
    };
    let mut buffer: *mut ID3D11Buffer = ptr::null_mut();
    let hr = (*gfx.device.as_raw()).CreateBuffer(&desc, &init, &mut buffer);
    hr_ok(hr, &format!("CreateBuffer for {role}"))?;
    ComPtr::from_raw(buffer, role)
}

/// Compile a `(vs, ps)` pair from one HLSL source with the given entry points.
unsafe fn create_vs_ps(
    gfx: &Gfx,
    source: &str,
    vs_entry: &str,
    ps_entry: &str,
    role: &str,
) -> Result<(ComPtr<ID3D11VertexShader>, ComPtr<ID3D11PixelShader>)> {
    let vs_blob = compile_shader(source, vs_entry, "vs_4_0")?;
    let ps_blob = compile_shader(source, ps_entry, "ps_4_0")?;
    let mut vs: *mut ID3D11VertexShader = ptr::null_mut();
    let hr = (*gfx.device.as_raw()).CreateVertexShader(
        (*vs_blob.as_raw()).GetBufferPointer(),
        (*vs_blob.as_raw()).GetBufferSize(),
        ptr::null_mut(),
        &mut vs,
    );
    hr_ok(hr, &format!("CreateVertexShader for {role}"))?;
    let vs = ComPtr::from_raw(vs, role)?;
    let mut ps: *mut ID3D11PixelShader = ptr::null_mut();
    let hr = (*gfx.device.as_raw()).CreatePixelShader(
        (*ps_blob.as_raw()).GetBufferPointer(),
        (*ps_blob.as_raw()).GetBufferSize(),
        ptr::null_mut(),
        &mut ps,
    );
    hr_ok(hr, &format!("CreatePixelShader for {role}"))?;
    let ps = ComPtr::from_raw(ps, role)?;
    Ok((vs, ps))
}

/// Build the files-first GPU bloom chain resources for a MainScene of
/// `source_width x source_height`. Returns `None` (no allocation) unless a
/// resolved bloom law is present, which happens only under `--postfx-restore-full`.
unsafe fn create_restore_bloom_gpu(
    gfx: &Gfx,
    restore: &RestoreFullPostfx,
    source_width: u32,
    source_height: u32,
    source_format: u32,
) -> Result<Option<RestoreBloomGpu>> {
    let Some(law) = restore.bloom else {
        return Ok(None);
    };
    // The bloom chain mirrors the engine's half-float MainScene->bloom pool. If
    // the replay's scene target is ever not R16G16B16A16_FLOAT (DXGI 10), the
    // bright-pass / blur precision assumptions no longer hold — fail closed
    // under the flag rather than silently blooming a different format.
    if source_format != DXGI_FORMAT_R16G16B16A16_FLOAT {
        return Err(format!(
            "bloom chain blocked: MainScene format is DXGI {source_format}, expected R16G16B16A16_FLOAT (10); the bloom chain is only validated for the game's half-float scene target"
        )
        .into());
    }
    if source_width < 2 || source_height < 2 {
        return Err(format!(
            "bloom chain blocked: MainScene {source_width}x{source_height} is too small to downsample"
        )
        .into());
    }
    // The engine's downsample is an exact 2x reduction; the CPU law rejects odd
    // sizes for the same reason. Fail closed rather than silently rounding.
    if source_width % 2 != 0 || source_height % 2 != 0 {
        return Err(format!(
            "bloom chain blocked: MainScene {source_width}x{source_height} is not an even size for the 2x downsample"
        )
        .into());
    }
    let half_width = source_width / 2;
    let half_height = source_height / 2;

    let (bright_texture, bright_rtv, bright_srv) =
        create_bloom_render_target(gfx, half_width, half_height, "bloom bright/vertical target")?;
    let (scratch_texture, scratch_rtv, scratch_srv) = create_bloom_render_target(
        gfx,
        half_width,
        half_height,
        "bloom horizontal scratch target",
    )?;

    let (bright_vs, bright_ps) = create_vs_ps(
        gfx,
        RESTORE_BLOOM_HLSL,
        "bloom_vs",
        "bloom_bright_ps",
        "bloom bright pass",
    )?;
    let (blur_vs, blur_ps) = create_vs_ps(
        gfx,
        RESTORE_BLOOM_HLSL,
        "bloom_vs",
        "bloom_blur_ps",
        "bloom blur pass",
    )?;

    // Bright-pass knee constants; inv-source-size drives the 2x2 box tap centers
    // over the FULL-res MainScene.
    let bright_constants = BloomBrightConstants {
        threshold_emissive_inv_size: [
            law.params.bright_threshold,
            law.params.emissive_bloom_strength,
            1.0 / source_width as f32,
            1.0 / source_height as f32,
        ],
    };
    let bright_cb = create_immutable_constant_buffer(gfx, &bright_constants, "bloom bright CB")?;

    // Blur constants: one per axis. Offsets are in blur-buffer texels; the
    // inv-size is the HALF-res buffer size (bloom.shader samples its own extent).
    let inv_half = [1.0 / half_width as f32, 1.0 / half_height as f32];
    let make_blur_cb = |axis: [f32; 2], role: &str| -> Result<ComPtr<ID3D11Buffer>> {
        let constants = BloomBlurConstants {
            axis_inv_size: [axis[0], axis[1], inv_half[0], inv_half[1]],
            weight0: [law.kernel.weight0, 0.0, 0.0, 0.0],
            weights: law.kernel.weights,
            offsets: law.kernel.offsets,
        };
        create_immutable_constant_buffer(gfx, &constants, role)
    };
    // bloom.shader: horizontal pass Axis=1, then vertical pass Axis=0.
    let blur_cb_horizontal = make_blur_cb([1.0, 0.0], "bloom horizontal blur CB")?;
    let blur_cb_vertical = make_blur_cb([0.0, 1.0], "bloom vertical blur CB")?;

    Ok(Some(RestoreBloomGpu {
        half_width,
        half_height,
        _bright_texture: bright_texture,
        bright_rtv,
        bright_srv,
        _scratch_texture: scratch_texture,
        scratch_rtv,
        scratch_srv,
        bright_vs,
        bright_ps,
        blur_vs,
        blur_ps,
        blur_cb_horizontal,
        blur_cb_vertical,
        bright_cb,
    }))
}

// A wide full-map orthographic dump corresponds to the game's highest-altitude
// view: above every region posteffect_volume box (their tops sit below ~700), so
// the `Base Highest` height volume selects the grade. Using the captured
// vCamPos.y=2958 keeps the replay's selection identical to the live frame.
const RESTORE_GRADE_VIEW_CAMERA_HEIGHT: f32 = 2958.0;

/// FILES-FIRST grade source. The installed content are the source of truth so mods
/// work: the LUT is composed from resolver-loaded `colorcorrection_*.tga` via the
/// posteffect_volumes.txt selection law, and MiddleGrey / adaptation come from
/// resolver-loaded posteffect_volumes.txt + defines. Fails closed (naming the
/// gap) if the modset lacks a required file or selection key. The captured
/// bundle is NEVER read here — see `restore_full_postfx_validate_capture`.
fn restore_full_postfx_from_files(
    enabled: bool,
    game_resolver: Option<&GameFileResolver>,
    map_w: usize,
    map_h: usize,
    day_night_hour: Option<f32>,
) -> Result<RestoreFullPostfx> {
    let mut restore = RestoreFullPostfx {
        enabled,
        evidence_found: false,
        constants: RestorePostfxConstants::identity(),
        color_cube_rgba8: restore_identity_color_cube_rgba8(),
        bloom: None,
        luminance_bounds: None,
        middle_grey: 1.0,
    };
    if !enabled {
        return Ok(restore);
    }
    let resolver = game_resolver.ok_or(
        "--postfx-restore-full needs --game <the game install dir> (or BATTLE_PLAN_VANILLA): the grading LUT and constants are composed from the shipped installed content so mods work; no silent identity fallback",
    )?;
    // The full-map dump is an above-all-volumes view; day/night comes from the
    // playback hour when supplied (a night hour flips to the `_night` values),
    // otherwise the default day view.
    let night = day_night_hour
        .map(|hour| !(0.25..0.75).contains(&hour))
        .unwrap_or(false);
    let state = GradingSelectionState {
        camera: [
            map_w as f32 * 0.5,
            RESTORE_GRADE_VIEW_CAMERA_HEIGHT,
            map_h as f32 * 0.5,
        ],
        night,
        winter: false,
    };
    // The replay has no runtime average-luminance readback, so the adaptation
    // law's documented steady-state floor (MIN_HDR_ADJUSTMENT) is used.
    let grade = file_derived_grade(resolver, state, None)?;
    // Files-first bloom, resolved for the SAME volume the grade selected so the
    // active volume never disagrees between bloom and LUT (restorescene.shader
    // composites both from the one PostEffectBase state). Fails closed under the
    // flag if the modset omits a bloom key (never a silent no-bloom).
    let bloom_params =
        resolve_bloom_params_for_volume(resolver, &grade.resolved.value_name).map_err(|err| {
            format!(
                "--postfx-restore-full bloom blocked: could not resolve files-first bloom params for volume '{}': {err}",
                grade.resolved.value_name
            )
        })?;
    let bloom_kernel =
        BloomBlurKernel::from_gaussian(bloom_params.bloom_width, bloom_params.bloom_scale)?;
    restore.bloom = Some(RestoreBloomLaw {
        params: bloom_params,
        kernel: bloom_kernel,
    });
    // Files-first per-frame exposure adaptation (FIDELITY_LEDGER debt #2): the
    // `MinHdr_MaxHdr` clamp bounds come from the resolver-loaded defines and let
    // the composite pass recompute AvgLum from the rendered scene each frame via
    // the ported `downsample_luminance.shader` chain, instead of the
    // MIN_HDR_ADJUSTMENT steady-state floor `file_derived_grade` used above.
    let defines = load_graphics_grading_defines(resolver)?;
    let luminance_bounds = LuminanceClampBounds {
        min_hdr_adjustment: defines.min_hdr_adjustment,
        max_hdr_adjustment: defines.max_hdr_adjustment,
    };
    restore.evidence_found = true;
    restore.middle_grey = grade.resolved.middle_grey;
    restore.luminance_bounds = Some(luminance_bounds);
    restore.constants = RestorePostfxConstants::from_file_grade(
        grade.resolved.middle_grey,
        grade.average_luminance,
    );
    restore.color_cube_rgba8 = grade.color_cube_rgba8;
    eprintln!(
        "replay_app: --postfx-restore-full (files-first) {}; per-frame exposure adaptation on (MinHdr_MaxHdr {} / {}, downsample_luminance.shader chain); bloom (volume '{}'): BLOOM_WIDTH {} BLOOM_SCALE {} BRIGHT_THRESHOLD {} EMISSIVE_BLOOM_STRENGTH {}",
        grade.source_note,
        luminance_bounds.min_hdr_adjustment,
        luminance_bounds.max_hdr_adjustment,
        grade.resolved.value_name,
        bloom_params.bloom_width,
        bloom_params.bloom_scale,
        bloom_params.bright_threshold,
        bloom_params.emissive_bloom_strength,
    );
    Ok(restore)
}

/// VALIDATION-ONLY path: load the captured restore bundle and report file-law
/// vs captured deltas (LUT max per-channel delta, per-constant diffs). Never a
/// render input — this is the regression gate for the files-first law.
pub fn restore_full_postfx_validate_capture(
    game_resolver: Option<&GameFileResolver>,
    capture_dir: &Path,
    map_w: usize,
    map_h: usize,
    day_night_hour: Option<f32>,
) -> Result<RestoreCaptureValidation> {
    let resolver = game_resolver.ok_or(
        "--postfx-validate-capture needs --game <the game install dir> to compose the file-law grade",
    )?;
    let bundle = load_restore_scene_bundle_session_dir(capture_dir)?
        .ok_or_else(|| format!("no {RESTORE_PASS_BUNDLE_FILE} in {}", capture_dir.display()))?;
    let night = day_night_hour
        .map(|hour| !(0.25..0.75).contains(&hour))
        .unwrap_or(false);
    let state = GradingSelectionState {
        camera: [
            map_w as f32 * 0.5,
            RESTORE_GRADE_VIEW_CAMERA_HEIGHT,
            map_h as f32 * 0.5,
        ],
        night,
        winter: false,
    };
    let grade = file_derived_grade(resolver, state, None)?;

    let captured = &bundle.color_cube.rgba8;
    let composed = &grade.color_cube_rgba8;
    if captured.len() != composed.len() {
        return Err(format!(
            "captured ColorCube is {} bytes, composed is {} bytes",
            captured.len(),
            composed.len()
        )
        .into());
    }
    // Compare RGB only (alpha is forced opaque on both sides).
    let mut lut_max_delta = 0u8;
    for (cap_texel, file_texel) in captured.chunks_exact(4).zip(composed.chunks_exact(4)) {
        for channel in 0..3 {
            let delta = cap_texel[channel].abs_diff(file_texel[channel]);
            lut_max_delta = lut_max_delta.max(delta);
        }
    }

    let middle_grey_delta = (grade.resolved.middle_grey - bundle.posteffect_cb.middle_grey).abs();
    let average_luminance_delta = (grade.average_luminance - bundle.average_luminance.value).abs();

    Ok(RestoreCaptureValidation {
        selected_value: grade.resolved.value_name,
        selection_reason: grade.resolved.selection_reason,
        composed_lut_rel_path: grade.resolved.lut_rel_path,
        lut_max_delta,
        file_middle_grey: grade.resolved.middle_grey,
        captured_middle_grey: bundle.posteffect_cb.middle_grey,
        middle_grey_delta,
        file_average_luminance: grade.average_luminance,
        captured_average_luminance: bundle.average_luminance.value,
        average_luminance_delta,
        captured_bundle_file: bundle.bundle_file,
    })
}

/// Report of files-first-law grade vs captured evidence.
#[derive(Clone, Debug, PartialEq)]
pub struct RestoreCaptureValidation {
    pub selected_value: String,
    pub selection_reason: String,
    pub composed_lut_rel_path: String,
    pub lut_max_delta: u8,
    pub file_middle_grey: f32,
    pub captured_middle_grey: f32,
    pub middle_grey_delta: f32,
    pub file_average_luminance: f32,
    pub captured_average_luminance: f32,
    pub average_luminance_delta: f32,
    pub captured_bundle_file: String,
}

fn restore_identity_color_cube_rgba8() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(1024 * 32 * 4);
    for b in 0..32u32 {
        for g in 0..32u32 {
            for r in 0..32u32 {
                bytes.push(((r * 255 + 15) / 31) as u8);
                bytes.push(((g * 255 + 15) / 31) as u8);
                bytes.push(((b * 255 + 15) / 31) as u8);
                bytes.push(255);
            }
        }
    }
    bytes
}

fn captured_tree_season_cb_from_date(
    seasons: Option<&SeasonsTable>,
    tree_season_date: Option<(u32, u32)>,
) -> Option<(f32, f32)> {
    let seasons = seasons?;
    let (month, day) = tree_season_date?;
    tree_season_cb_values(seasons, month, day)
}

impl RestorePostfxConstants {
    fn identity() -> Self {
        Self {
            hsv: [0.0, 1.0, 1.0, 0.0],
            color_balance_use_lut: [1.0, 1.0, 1.0, 0.0],
            exposure: [1.0, 1.0, 0.0, 0.0],
        }
    }

    /// FILES-FIRST constructor: the base posteffect_volumes.txt grade values
    /// carry no HSV / ColorBalance block, so those stay neutral (HSV shift 0,
    /// sat/val x1, balance x1); the LUT block is enabled and exposure uses the
    /// files-derived MiddleGrey over the adaptation-law average luminance.
    fn from_file_grade(middle_grey: f32, average_luminance: f32) -> Self {
        let mut constants = Self::identity();
        constants.color_balance_use_lut = [1.0, 1.0, 1.0, 1.0];
        if average_luminance.is_finite() && average_luminance > 0.0 {
            constants.exposure = [middle_grey, average_luminance, 1.0, 0.0];
        }
        constants
    }
}

fn gradient_border_camera_height(
    zoom: f64,
    screen_w: f64,
    screen_h: f64,
    map_w: f64,
    map_h: f64,
) -> f32 {
    let fit_zoom = (screen_w.max(1.0) / map_w.max(1.0)).min(screen_h.max(1.0) / map_h.max(1.0));
    let relative_zoom = (zoom / fit_zoom.max(f64::MIN_POSITIVE)).max(f64::MIN_POSITIVE);
    (f64::from(REPLAY_GB_BASE_CAMERA_HEIGHT) / relative_zoom).clamp(
        f64::from(BATTLE_PLAN_CAMERA_MIN_HEIGHT),
        f64::from(BATTLE_PLAN_CAMERA_MAX_HEIGHT),
    ) as f32
}

unsafe fn create_captured_output_srv(
    gfx: &Gfx,
    target_format: u32,
    texture: *mut ID3D11Texture2D,
) -> Result<ComPtr<ID3D11ShaderResourceView>> {
    let mut desc: D3D11_SHADER_RESOURCE_VIEW_DESC = mem::zeroed();
    desc.Format = target_format;
    desc.ViewDimension = D3D11_SRV_DIMENSION_TEXTURE2D;
    *desc.u.Texture2D_mut() = D3D11_TEX2D_SRV {
        MostDetailedMip: 0,
        MipLevels: 1,
    };
    let mut srv: *mut ID3D11ShaderResourceView = ptr::null_mut();
    let hr = (*gfx.device.as_raw()).CreateShaderResourceView(texture as *mut _, &desc, &mut srv);
    hr_ok(hr, "CreateShaderResourceView for captured-pass output")?;
    ComPtr::from_raw(srv, "captured-pass output SRV")
}

/// Ground rect `[min_x, min_z, max_x, max_z]` visible from the camera,
/// padded by one coarse tile so partially-visible edges and displaced
/// terrain still draw. `None` (no culling) when any screen corner ray
/// misses the ground plane.
fn camera_ground_cull_rect(
    camera: &ProductionMapCamera,
    screen_w: u32,
    screen_h: u32,
) -> Option<[f32; 4]> {
    const PAD: f32 = 256.0;
    let w = screen_w.max(1) as f32;
    let h = screen_h.max(1) as f32;
    let mut min_x = f32::INFINITY;
    let mut min_z = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_z = f32::NEG_INFINITY;
    for (sx, sy) in [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)] {
        let (x, z) = camera.screen_to_ground_y0(sx, sy, w, h).ok()??;
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_z = min_z.min(z);
        max_z = max_z.max(z);
    }
    Some([min_x - PAD, min_z - PAD, max_x + PAD, max_z + PAD])
}

fn shifted_cull_world_rect(rect: Option<[f32; 4]>, wrap_offset: f32) -> Option<[f32; 4]> {
    rect.map(|[min_x, min_z, max_x, max_z]| {
        [min_x - wrap_offset, min_z, max_x - wrap_offset, max_z]
    })
}

fn production_camera_for_wrap_copy(
    projection: ProductionMapCameraProjection,
    camera: ProductionMapCamera,
    wrap_offset: f32,
) -> Result<ProductionMapCamera> {
    if wrap_offset == 0.0 {
        return Ok(camera);
    }
    let mut position = camera.position;
    position[0] -= wrap_offset;
    projection.camera_from_position(position)
}

fn production_camera_from_app_camera(
    projection: ProductionMapCameraProjection,
    defines: Hoi4MapCameraDefines,
    camera: &Camera,
    screen_w: u32,
) -> Result<ProductionMapCamera> {
    let screen_w = screen_w.max(1) as f32;
    let height = projection.height_for_screen_zoom(
        camera.zoom as f32,
        screen_w,
        defines.min_height(),
        defines.max_height(),
    )?;
    // The app camera is in bmp-style map pixels (y grows south/down, like
    // provinces.bmp rows); the renderer's ground plane z grows north.
    // Flipping here (and unflipping in screen_to_map) keeps every
    // interaction path - key pan, drag, wheel anchor, picking - in one
    // consistent y-down space.
    let ground_z = camera.map_h() as f32 - camera.center_y as f32;
    projection.camera_for_ground_center(camera.center_x as f32, ground_z, height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use map_viewer::{SeasonBlock, SeasonColorGrade, SeasonDate, SeasonDateRange, SeasonId};

    #[test]
    fn cull_world_rect_shifts_into_wrap_copy_base_domain() {
        assert_eq!(
            shifted_cull_world_rect(Some([-128.0, 10.0, 512.0, 900.0]), -5632.0),
            Some([5504.0, 10.0, 6144.0, 900.0])
        );
        assert_eq!(
            shifted_cull_world_rect(Some([5400.0, 0.0, 5800.0, 2048.0]), 5632.0),
            Some([-232.0, 0.0, 168.0, 2048.0])
        );
        assert_eq!(shifted_cull_world_rect(None, 5632.0), None);
    }

    #[test]
    fn restore_full_postfx_disabled_is_identity_old_default() {
        // GRAD1: the flag-off / opt-out (`--no-postfx-restore-full`) path is the
        // old capture-anchored default: a disabled, identity RestoreFullPostfx
        // with no files-first evidence. This is exactly what the default-ON
        // downgrade branch falls back to when evidence is missing.
        let disabled = restore_full_postfx_from_files(false, None, 5632, 2048, None)
            .expect("disabled grade never errors");
        assert!(!disabled.enabled, "flag-off grade must be disabled");
        assert!(
            !disabled.evidence_found,
            "flag-off grade carries no files-first evidence"
        );
        assert!(
            disabled.bloom.is_none() && disabled.luminance_bounds.is_none(),
            "flag-off grade has no files-first bloom/luminance recompose"
        );
    }

    #[test]
    fn restore_full_postfx_enabled_without_resolver_fails_closed() {
        // GRAD1: with the grade ENABLED but no resolver, the pure composer
        // errors (naming the missing --game). `MapRenderer::new` turns this into
        // a hard error only for an explicit request; the default-ON path catches
        // it and downgrades to the disabled grade proven identical above. The
        // error message must name the missing game install so the downgrade
        // notice / fail-closed error is actionable.
        let err = restore_full_postfx_from_files(true, None, 5632, 2048, None)
            .expect_err("enabled grade without a resolver must fail closed");
        assert!(
            err.to_string().contains("--game") || err.to_string().contains("BATTLE_PLAN_VANILLA"),
            "error must name the missing game install: {err}"
        );
    }

    #[test]
    fn tree_season_cb_is_wired_from_optional_date() {
        let seasons = test_seasons_table();
        assert_eq!(captured_tree_season_cb_from_date(None, Some((5, 1))), None);
        assert_eq!(
            captured_tree_season_cb_from_date(Some(&seasons), None),
            None
        );
        assert_eq!(
            captured_tree_season_cb_from_date(Some(&seasons), Some((5, 1))),
            Some((0.0, 4.0))
        );
    }

    fn test_seasons_table() -> SeasonsTable {
        let block = |id, start_month, end_month| SeasonBlock {
            id,
            date_range: SeasonDateRange {
                start: SeasonDate {
                    month: start_month,
                    day: 1,
                },
                end: SeasonDate {
                    month: end_month,
                    day: 28,
                },
            },
            north: test_grade(),
            center: test_grade(),
            south: test_grade(),
        };
        SeasonsTable {
            winter: block(SeasonId::Winter, 1, 2),
            spring: block(SeasonId::Spring, 3, 5),
            summer: block(SeasonId::Summer, 6, 8),
            autumn: block(SeasonId::Autumn, 9, 11),
            tree_winter: test_range(1),
            tree_winter2: test_range(2),
            tree_spring: test_range(3),
            tree_spring2: test_range(4),
            tree_summer: test_range(5),
            tree_summer2: test_range(6),
            tree_autumn: test_range(7),
            tree_autumn2: test_range(8),
        }
    }

    fn test_grade() -> SeasonColorGrade {
        SeasonColorGrade {
            hsv: [0.0, 1.0, 1.0],
            color_balance: [1.0, 1.0, 1.0],
        }
    }

    fn test_range(month: u8) -> SeasonDateRange {
        SeasonDateRange {
            start: SeasonDate { month, day: 1 },
            end: SeasonDate { month, day: 28 },
        }
    }
}
