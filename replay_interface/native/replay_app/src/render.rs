//! GPU map renderer for the captured the game D3D11 replay path.
//!
//! Product map rendering must enter through a validated the game D3D11 capture. The
//! old CPU `map_visual.rgbx` underlay and generated political SRV path are not
//! part of this renderer.

use std::collections::BTreeMap;
use std::ffi::CString;
use std::mem;
use std::path::{Path, PathBuf};
use std::ptr;
use std::time::{Duration, Instant};

use map_viewer::{
    hoi4_map_camera_defines_from_bundle, load_restore_scene_bundle_session_dir, map_wrap_offsets,
    resolve_bloom_params_for_volume, scene_average_luminance, tree_second_subdraw_verdict,
    AverageLuminanceReduction, BloomBlurKernel, BloomParams, GameFileResolver,
    Hoi4MapCameraDefines, LuminanceClampBounds, NativeD3d11PreparedProductionMap,
    NativeD3d11RenderOptions, NativeD3d11TerrainBundle, NativeD3d11TerrainRenderer,
    ProductionMapCamera, ProductionMapCameraProjection, RESTORE_PASS_BUNDLE_FILE,
};
use winapi::shared::dxgiformat::{DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_FORMAT_R8G8B8A8_UNORM};
use winapi::shared::dxgitype::DXGI_SAMPLE_DESC;
use winapi::shared::minwindef::UINT;
use winapi::um::d3d11::{
    ID3D11Buffer, ID3D11PixelShader, ID3D11RenderTargetView, ID3D11SamplerState,
    ID3D11ShaderResourceView, ID3D11Texture2D, ID3D11VertexShader, D3D11_BIND_CONSTANT_BUFFER,
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_BUFFER_DESC,
    D3D11_COMPARISON_ALWAYS, D3D11_CPU_ACCESS_READ, D3D11_CPU_ACCESS_WRITE,
    D3D11_FILTER_MIN_MAG_MIP_LINEAR, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ,
    D3D11_MAP_WRITE_DISCARD, D3D11_SAMPLER_DESC, D3D11_SHADER_RESOURCE_VIEW_DESC,
    D3D11_SUBRESOURCE_DATA, D3D11_TEX2D_SRV, D3D11_TEXTURE2D_DESC, D3D11_TEXTURE_ADDRESS_CLAMP,
    D3D11_USAGE_DEFAULT, D3D11_USAGE_DYNAMIC, D3D11_USAGE_IMMUTABLE, D3D11_USAGE_STAGING,
    D3D11_VIEWPORT,
};
use winapi::um::d3dcommon::{
    ID3DBlob, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, D3D11_SRV_DIMENSION_TEXTURE2D,
};
use winapi::um::d3dcompiler::D3DCompile;

use crate::bundle::{MapBundle, PRODUCT_MAP_WIDTH};
use crate::camera::Camera;
use crate::d3d::{hr_ok, ComPtr, Gfx};
use crate::grading_law::{
    file_derived_grade, load_graphics_grading_defines, GradingSelectionState,
};
use crate::Result;

pub(crate) const POLITICAL_COLOR_BLOCKER_BADGE: &str = "political colors BLOCKED";

/// Full-screen composite shader for the captured the game D3D11 scene target.
/// The input color comes from the captured engine pass, never from the removed
/// CPU `map_visual` underlay.
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
/// only when the files-first restore grade is enabled; the internal disabled
/// composer path never allocates or binds any of them.
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

const TERRAIN_BUNDLE_FILE: &str = "map_frame_bundle.json";
const WATER_BUNDLE_FILE: &str = "map_frame_water_pass_bundle.json";
const RIVERS_BUNDLE_FILE: &str = "map_frame_rivers_pass_bundle.json";
const TREES_BUNDLE_FILE: &str = "map_frame_trees_pass_bundle.json";

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
fn tree_layer_note(
    trees: &NativeD3d11TerrainBundle,
    game_resolver: Option<&GameFileResolver>,
) -> String {
    let Some(resolver) = game_resolver else {
        // No resolver: cannot classify the bytecode by recompile, so keep the
        // conservative, honest statement (identification is unavailable here).
        return "trees layer: second-tree-shader identification unavailable without a --game \
                resolver; rendering the captured full-detail tree family"
            .to_string();
    };
    match tree_second_subdraw_verdict(resolver, trees) {
        Ok(verdict) => verdict.layer_note().unwrap_or_else(|| {
            "trees layer: rendering the captured full-detail tree family".to_string()
        }),
        Err(err) => {
            format!("trees layer: second-tree-shader identification failed ({err}); rendering the captured full-detail tree family")
        }
    }
}

fn load_terrain_bundle_for_replay(
    path: impl AsRef<Path>,
    game_resolver: Option<&GameFileResolver>,
) -> Result<NativeD3d11TerrainBundle> {
    let path = path.as_ref();
    match game_resolver {
        Some(resolver) => NativeD3d11TerrainBundle::load_with_resolver(path, resolver),
        None => NativeD3d11TerrainBundle::load(path),
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

#[derive(Clone, Copy, Debug, PartialEq)]
struct CapturedCameraKey {
    center_x: f64,
    center_y: f64,
    zoom: f64,
    screen_w: u32,
    screen_h: u32,
    day_night_hour: Option<f32>,
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
    pub last_render_ms: Option<f64>,
    pub last_day_night_hour: Option<f32>,
    pub postfx_enabled: bool,
    pub postfx_restore_full: bool,
    pub restore_evidence_found: bool,
    pub water_gap: Option<String>,
    pub layer_gaps: Vec<String>,
}

struct CapturedPassLayer {
    session_dir: PathBuf,
    renderer: NativeD3d11TerrainRenderer,
    prepared: NativeD3d11PreparedProductionMap,
    camera_projection: ProductionMapCameraProjection,
    camera_defines: Hoi4MapCameraDefines,
    output_srv: ComPtr<ID3D11ShaderResourceView>,
    composite_vs: ComPtr<ID3D11VertexShader>,
    composite_ps: ComPtr<ID3D11PixelShader>,
    composite_sampler: ComPtr<ID3D11SamplerState>,
    water_gap: Option<String>,
    layer_gaps: Vec<String>,
    postfx_enabled: bool,
    restore_postfx: RestoreFullPostfx,
    restore_postfx_cb: Option<ComPtr<ID3D11Buffer>>,
    restore_color_cube: Option<RestoreColorCubeGpu>,
    restore_bloom: Option<RestoreBloomGpu>,
    /// COL3: the political-mapmode scene CB0 state (GB override/cutoffs +
    /// shadow-fade height law) written into every pass's scene CB0 on camera
    /// update. Defines are resolver-read (mod-aware); missing resolver data is
    /// a blocker rather than a baked vanilla fallback. See
    /// `map_viewer::PoliticalSceneCb0`.
    political_cb0: map_viewer::PoliticalSceneCb0,
    last_camera: Option<CapturedCameraKey>,
    dirty: bool,
    last_render: Option<Duration>,
}

impl CapturedPassLayer {
    unsafe fn new(
        gfx: &Gfx,
        session_dir: &Path,
        game_resolver: Option<&GameFileResolver>,
        postfx_enabled: bool,
        restore_postfx: RestoreFullPostfx,
        water_full: bool,
    ) -> Result<Self> {
        if water_full && game_resolver.is_none() {
            return Err(
                "--water-full refraction requires --game <the game install dir> or BATTLE_PLAN_VANILLA".into(),
            );
        }
        let mut renderer = NativeD3d11TerrainRenderer::from_borrowed_raw(
            gfx.device.as_raw(),
            gfx.context.as_raw(),
        )?;
        renderer.set_water_full(water_full);
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
        // WATER1/REFLECT: when the modset resolver is available, the
        // ReflectionCubeMap slot (slot 8) is produced by decoding + repacking the
        // mod-aware map/terrain/reflection.dds and BYTE-VERIFIED equal to the
        // captured payload (fail closed on mismatch). PROVENANCE-only: pixels are
        // provably unchanged for vanilla, mod-correct otherwise. Only affects the
        // paths that upload the real cube (--water-full / reference replay); the
        // blocked legacy mean-color approximation never reaches the decode, so
        // this is safe to thread unconditionally where a resolver exists.
        let renderer = if let Some(resolver) = game_resolver {
            renderer.with_reflection_resolver(resolver.clone())
        } else {
            renderer
        };
        let mut layer_gaps = Vec::new();
        let terrain_path = session_dir.join(TERRAIN_BUNDLE_FILE);
        let terrain = load_terrain_bundle_for_replay(&terrain_path, game_resolver)?;
        let base_camera = terrain.production_camera_from_capture()?;
        let camera_defines = hoi4_map_camera_defines_from_bundle(&terrain)?;

        let water_path = session_dir.join(WATER_BUNDLE_FILE);
        let water = if water_path.exists() {
            NativeD3d11TerrainBundle::load_water(&water_path)?
        } else {
            return Err(format!(
                "captured map rendering requires the water pass bundle: {}",
                water_path.display()
            )
            .into());
        };
        let rivers_path = session_dir.join(RIVERS_BUNDLE_FILE);
        let rivers = if rivers_path.exists() {
            NativeD3d11TerrainBundle::load_rivers(&rivers_path)?
        } else {
            return Err(format!(
                "captured map rendering requires the rivers pass bundle: {}",
                rivers_path.display()
            )
            .into());
        };
        let trees_path = session_dir.join(TREES_BUNDLE_FILE);
        let trees = if trees_path.exists() {
            // TREES2: PAN2 (and every captured session) binds only the
            // full-detail tree family. The 6b77b6df pixel shader the old
            // warning called a "second sub-draw" is gfx/FX/tree.shader:
            // tree_low_gfx — the LOW_END_GFX variant, compiled but bound to
            // no draw at the captured graphics settings. Gate the note on
            // real draw-binding evidence instead of asserting a gap.
            let trees = NativeD3d11TerrainBundle::load_trees(&trees_path)?;
            layer_gaps.push(tree_layer_note(&trees, game_resolver));
            trees
        } else {
            return Err(format!(
                "captured map rendering requires the trees pass bundle: {}",
                trees_path.display()
            )
            .into());
        };
        let prepared = renderer.prepare_production_map_with_layers(
            &terrain,
            Some(&water),
            Some(&rivers),
            None,
            Some(&trees),
            base_camera,
        )?;
        let water_gap = None;
        let camera_projection = ProductionMapCameraProjection::from_camera(base_camera)?;
        layer_gaps.push(
            "replay-driven political color substitution blocked: captured-pass rendering keeps the HOI4-captured ProvinceSecondaryColorMap and GradientBorderChannel SRVs until the engine's primary political fill and exact CGradientBorderManager generation law are ported"
                .to_string(),
        );
        let output_srv = create_captured_output_srv(
            gfx,
            prepared.target_format(),
            prepared.output_texture_raw(),
        )?;
        // Bloom is part of the postfx composite (restorescene.shader adds it
        // before Exposure/ToneMap). It rides the same internal `postfx_enabled`
        // switch as tone/LUT so validation-only disabled postfx does not produce
        // the impossible hybrid "linear scene + bloom" image.
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
        // run, or bound. The scene target must be R16G16B16A16_FLOAT (the game's
        // MainScene); fail closed if a future capture format differs.
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
            "replay_app: captured-pass session {} loaded ({}x{} format {}, water {}, rivers {}, trees {}, postfx {}, restore_full {}, restore_evidence {}, bloom {})",
            session_dir.display(),
            prepared.target_width(),
            prepared.target_height(),
            prepared.target_format(),
            if prepared.has_water_pass() { "on" } else { "off" },
            if prepared.has_rivers_pass() { "on" } else { "off" },
            if prepared.has_trees_pass() { "on" } else { "off" },
            if postfx_enabled { "on" } else { "off" },
            if restore_postfx.enabled { "on" } else { "off" },
            if restore_postfx.evidence_found { "found" } else { "none" },
            if restore_bloom.is_some() { "on" } else { "off" }
        );

        // COL3: political-mapmode scene CB0 defines, resolver-read like the GB
        // country field. This is product-visible draw state, so missing source
        // files block startup instead of falling back to baked vanilla values.
        let resolver = game_resolver.ok_or(
            "replay_app captured-pass rendering blocked: --game <the game install dir> or BATTLE_PLAN_VANILLA is required to derive political scene CB0 defines from common/defines/00_graphics.lua",
        )?;
        let political_cb0 =
            crate::political_scene::PoliticalSceneDefines::from_resolver(resolver)?.to_cb0();
        reject_snow_mud_env_overrides(
            std::env::var_os("REPLAY_SNOW_MUD_OVERRIDE"),
            std::env::var_os("REPLAY_SNOW_MUD_FADE"),
        )?;

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
            water_gap,
            layer_gaps,
            postfx_enabled,
            restore_postfx,
            restore_postfx_cb,
            restore_color_cube,
            restore_bloom,
            political_cb0,
            last_camera: None,
            dirty: true,
            last_render: None,
        })
    }

    #[allow(dead_code)]
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
        map_clock_time: Option<f32>,
    ) -> Result<()> {
        if let Some(hour) = day_night_hour {
            validate_day_night_hour(hour)?;
        }
        let key = CapturedCameraKey {
            center_x: camera.center_x,
            center_y: camera.center_y,
            zoom: camera.zoom,
            screen_w: gfx.width,
            screen_h: gfx.height,
            day_night_hour,
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
                self.renderer.update_prepared_production_map_scene(
                    &mut self.prepared,
                    copy_camera,
                    day_night_hour,
                    map_clock_time,
                    Some(self.political_cb0),
                )?;
                let options = NativeD3d11RenderOptions {
                    output: None,
                    readback: false,
                    compare_each_draw: false,
                    compare_final_reference: false,
                    parity_report: None,
                    clear_targets: copy_index == 0,
                    // The plain captured bundle ignores this cull option and
                    // redraws its captured pass per wrap copy.
                    cull_world_rect: shifted_cull_world_rect(cull_world_rect, wrap_offset),
                    tree_season_cb: None,
                    map_clock_time,
                };
                self.renderer
                    .render_prepared_production_map(&self.prepared, &options)?;
            }
            // Files-first per-frame exposure adaptation (FIDELITY_LEDGER debt
            // #2): reduce the just-rendered HDR MainScene through the ported
            // downsample_luminance.shader chain and rewrite the exposure's
            // AverageLuminance, replacing the constant MIN_HDR_ADJUSTMENT
            // floor. AvgLum depends only on the rendered scene bytes, so it is
            // recomputed exactly when the scene re-renders (this dirty branch)
            // rather than in composite() — composite runs every presented
            // frame and the readback is a full GPU sync + CPU reduction.
            // Gated so validation-only disabled postfx never reads back or
            // mutates anything.
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

    fn stats(&self) -> CapturedPassStats {
        CapturedPassStats {
            session_dir: self.session_dir.clone(),
            target_width: self.prepared.target_width(),
            target_height: self.prepared.target_height(),
            target_format: self.prepared.target_format(),
            has_water: self.prepared.has_water_pass(),
            has_rivers: self.prepared.has_rivers_pass(),
            has_trees: self.prepared.has_trees_pass(),
            last_render_ms: self
                .last_render
                .map(|duration| duration.as_secs_f64() * 1000.0),
            last_day_night_hour: self.last_camera.and_then(|key| key.day_night_hour),
            postfx_enabled: self.postfx_enabled,
            postfx_restore_full: self.restore_postfx.enabled,
            restore_evidence_found: self.restore_postfx.evidence_found,
            water_gap: self.water_gap.clone(),
            layer_gaps: self.layer_gaps.clone(),
        }
    }
}

pub struct MapRenderer {
    captured: CapturedPassLayer,
    max_state_id: u32,
}

impl MapRenderer {
    /// Build the captured D3D11 replay renderer. `max_state_id` is retained only
    /// for validating replay/controller color inputs while political recoloring
    /// remains blocked.
    pub fn new(
        gfx: &Gfx,
        bundle: &MapBundle,
        max_state_id: u32,
        capture_dir: Option<&Path>,
        game_resolver: Option<&GameFileResolver>,
        postfx_enabled: bool,
        postfx_restore_full: bool,
        water_full: bool,
    ) -> Result<MapRenderer> {
        if capture_dir.is_none() {
            return Err(
                "replay_app map rendering blocked: --capture <validated the game D3D11 map-frame session> is required. The non-captured map_visual.rgbx path was removed because it was a CPU scaffold with known non-engine-complete terrain and color shortcuts (TerrainIDMap generation, mip/derivative behavior, runtime buffers, lighting, and post-processing), so it may not be shown as product output."
                    .into(),
            );
        }
        unsafe {
            // Files-first grade inputs come from shipped installed content through the
            // resolver. Missing resolver or malformed source data is a hard
            // blocker, not a downgrade to an ungraded map.
            let restore_postfx = restore_full_postfx_from_files(
                postfx_restore_full,
                game_resolver,
                bundle.width,
                bundle.height,
                None,
            )?;
            // Captured-pass construction is fail-closed: missing water evidence,
            // terrain evidence, or resolver data stops startup rather than
            // retrying with app-owned approximations.
            let capture_dir = capture_dir.expect("capture_dir checked above");
            let captured = CapturedPassLayer::new(
                gfx,
                capture_dir,
                game_resolver,
                postfx_enabled,
                restore_postfx.clone(),
                water_full,
            )?;
            Ok(MapRenderer {
                captured,
                max_state_id,
            })
        }
    }

    /// Scrub step: validate controller colors while visible replay-driven
    /// political colors remain blocked.
    pub fn update_state_politics(
        &mut self,
        _gfx: &Gfx,
        _owners: &BTreeMap<u32, String>,
        controllers: &BTreeMap<u32, String>,
        country_colors: &BTreeMap<String, [u8; 3]>,
    ) -> Result<()> {
        let _ = crate::palette::build_state_color_table(
            self.max_state_id,
            controllers,
            country_colors,
        )?;
        Ok(())
    }

    pub fn render(
        &mut self,
        gfx: &Gfx,
        camera: &Camera,
        day_night_hour: Option<f32>,
        map_clock_time: Option<f32>,
    ) -> Result<()> {
        self.captured
            .render(gfx, camera, day_night_hour, map_clock_time)
    }

    pub fn screen_to_map(
        &self,
        camera: &Camera,
        screen_x: f64,
        screen_y: f64,
        screen_w: f64,
        screen_h: f64,
    ) -> Result<Option<(f64, f64)>> {
        self.captured
            .screen_to_map(camera, screen_x, screen_y, screen_w, screen_h)
    }

    pub fn captured_pass_stats(&self) -> Option<CapturedPassStats> {
        Some(self.captured.stats())
    }

    /// The resolver/bundle-read the game camera defines (00_graphics.lua) this
    /// renderer's production camera obeys.
    pub fn camera_defines(&self) -> Hoi4MapCameraDefines {
        self.captured.camera_defines
    }

    /// Engine-defines wheel zoom: map the app camera's screen zoom to the
    /// production camera height (the same `height_for_screen_zoom` law the
    /// renderer itself applies), step it by `notches` wheel steps of
    /// `CAMERA_ZOOM_SPEED` scaled toward `CAMERA_ZOOM_SPEED_DISTANCE_MULT`
    /// with height (00_graphics.lua:1187-1189), clamp to
    /// `CAMERA_MIN/MAX_HEIGHT` (:1495-1496), and return the multiplicative
    /// screen-zoom factor that realizes the new height. Replaces the old
    /// fixed 1.2^notches interaction proxy with the shipped defines.
    pub fn wheel_zoom_screen_factor(&self, zoom: f64, screen_w: u32, notches: f64) -> Result<f64> {
        let projection = self.captured.camera_projection;
        let defines = self.captured.camera_defines;
        let screen_w = screen_w.max(1) as f32;
        let height = projection.height_for_screen_zoom(
            zoom as f32,
            screen_w,
            defines.min_height(),
            defines.max_height(),
        )?;
        let stepped = defines.wheel_zoom_stepped_height(height, notches as f32);
        let new_zoom = projection.screen_zoom_for_height(stepped, screen_w)?;
        if !(new_zoom.is_finite() && new_zoom > 0.0) {
            return Err(format!(
                "wheel zoom produced an invalid screen zoom {new_zoom} at height {stepped}"
            )
            .into());
        }
        let factor = f64::from(new_zoom) / zoom.max(f64::MIN_POSITIVE);
        // Direction guard: when the app camera sits OUTSIDE the engine zoom
        // range (e.g. the whole-map fit view is farther out than
        // CAMERA_MAX_HEIGHT allows), a further zoom-out must be a no-op, not a
        // snap back into range; symmetric for zoom-in at CAMERA_MIN_HEIGHT.
        Ok(if notches >= 0.0 {
            factor.max(1.0)
        } else {
            factor.min(1.0)
        })
    }

    /// COLOR-lane debug readback (`--dump-hdr`): see
    /// [`CapturedPassLayer::dump_hdr_mainscene`]. Fails closed when no
    /// captured pass is active (there is no HDR MainScene without one).
    pub fn dump_hdr_mainscene(&self, gfx: &Gfx, path: &std::path::Path) -> Result<()> {
        self.captured.dump_hdr_mainscene(gfx, path)
    }
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
fn reject_snow_mud_env_overrides(
    snow_mud_override: Option<std::ffi::OsString>,
    snow_mud_fade: Option<std::ffi::OsString>,
) -> Result<()> {
    if snow_mud_override.is_some() || snow_mud_fade.is_some() {
        return Err(
            "replay_app snow/mud environment overrides are blocked: SnowMudTexture and SnowMudFade must come from the captured engine pass or a documented game-file law, not REPLAY_SNOW_MUD_OVERRIDE / REPLAY_SNOW_MUD_FADE"
                .into(),
        );
    }
    Ok(())
}

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
    fn restore_full_postfx_disabled_is_inert_validation_path() {
        // The product CLI blocks `--no-postfx-restore-full`; this disabled
        // constructor path is retained only to prove the composer is inert when
        // validation code asks for it directly.
        let disabled = restore_full_postfx_from_files(false, None, 5632, 2048, None)
            .expect("disabled grade never errors");
        assert!(!disabled.enabled, "disabled grade must remain disabled");
        assert!(
            !disabled.evidence_found,
            "disabled grade carries no files-first evidence"
        );
        assert!(
            disabled.bloom.is_none() && disabled.luminance_bounds.is_none(),
            "disabled grade has no files-first bloom/luminance recompose"
        );
    }

    #[test]
    fn restore_full_postfx_enabled_without_resolver_fails_closed() {
        // With the grade enabled but no resolver, the composer errors and the
        // renderer propagates that blocker. The message names the missing game
        // install so the user knows which evidence source is absent.
        let err = restore_full_postfx_from_files(true, None, 5632, 2048, None)
            .expect_err("enabled grade without a resolver must fail closed");
        assert!(
            err.to_string().contains("--game") || err.to_string().contains("BATTLE_PLAN_VANILLA"),
            "error must name the missing game install: {err}"
        );
    }

    #[test]
    fn snow_mud_env_overrides_are_product_blocked() {
        reject_snow_mud_env_overrides(None, None).expect("absent overrides are fine");

        let err = reject_snow_mud_env_overrides(Some(std::ffi::OsString::from("snow.rgba")), None)
            .expect_err("texture override must be blocked");
        assert!(
            err.to_string().contains("REPLAY_SNOW_MUD_OVERRIDE"),
            "{err}"
        );

        let err = reject_snow_mud_env_overrides(None, Some(std::ffi::OsString::from("0.75")))
            .expect_err("fade override must be blocked");
        assert!(err.to_string().contains("REPLAY_SNOW_MUD_FADE"), "{err}");
    }
}
