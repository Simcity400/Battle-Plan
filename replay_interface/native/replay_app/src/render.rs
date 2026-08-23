//! old CPU `map_visual.rgbx` underlay and generated political SRV path are not

use std::collections::BTreeMap;
use std::ffi::CString;
use std::io::Write as _;
use std::mem;
use std::path::{Path, PathBuf};
use std::ptr;
use std::time::{Duration, Instant};

use map_viewer::{
    hoi4_map_camera_defines_from_bundle, load_restore_scene_bundle_session_dir, map_wrap_offsets,
    scene_average_luminance, tree_second_subdraw_verdict, AverageLuminanceReduction,
    BloomBlurKernel, BloomParams, C03CameraReplayBinding, GameFileResolver, Hoi4MapCameraDefines,
    LuminanceClampBounds, NativeD3d11PreparedProductionMap, NativeD3d11RenderOptions,
    NativeD3d11TerrainBundle, NativeD3d11TerrainRenderer, PostEffectRegionWeightManifest,
    ProductionMapCamera, ProductionMapCameraProjection, RESTORE_PASS_BUNDLE_FILE,
};
use replay_core::{
    OccupationSecondaryReplayReadbackReceipt, PoliticalTextureFormat, PoliticalTextureKind,
    ProductionRenderFrame, RenderSubmit, SnowMudReplayReadbackReceipt, SnowMudTexturePacket,
    StaticMapCommand, StaticShaderResourceSource, ViewStatus,
    WeatherProvinceSnowView, WorldFrame,
};
use sha2::{Digest, Sha256};
use winapi::shared::dxgiformat::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_B8G8R8A8_UNORM_SRGB, DXGI_FORMAT_R16G16B16A16_FLOAT,
    DXGI_FORMAT_R32G32B32A32_FLOAT, DXGI_FORMAT_R8G8B8A8_UNORM,
};
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
use crate::counter_gpu::{CounterGpuFrameAdmission, CounterGpuPass};
use crate::d3d::{hr_ok, ComPtr, Gfx};
use crate::grading_law::{
    file_derived_grade, file_derived_grade_per_frame, load_posteffect_region_weight_manifest,
    posteffect_region_winter_input_for_frame, FileDerivedGrade, GradingSelectionState,
};
use crate::Result;

pub(crate) const POLITICAL_COLOR_BLOCKER_BADGE: &str = "political colors BLOCKED";

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
    threshold_emissive_inv_size: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct BloomBlurConstants {
    axis_inv_size: [f32; 4],
    weight0: [f32; 4],
    weights: [f32; 4],
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
    bloom: Option<RestoreBloomLaw>,
    luminance_bounds: Option<LuminanceClampBounds>,
    middle_grey: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct RestoreBloomLaw {
    params: BloomParams,
    kernel: BloomBlurKernel,
}

struct RestoreColorCubeGpu {
    _texture: ComPtr<ID3D11Texture2D>,
    srv: ComPtr<ID3D11ShaderResourceView>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RestoreGradeKey {
    camera: [u32; 3],
    night: bool,
    winter: bool,
    region_winter_factor: Option<u32>,
    region_snow_inputs_sha256: Option<[u8; 32]>,
}

impl RestoreGradeKey {
    fn from_state(
        state: GradingSelectionState,
        region_snow_inputs_sha256: Option<[u8; 32]>,
    ) -> Self {
        Self {
            camera: state.camera.map(f32::to_bits),
            night: state.night,
            winter: state.winter,
            region_winter_factor: state.region_winter_factor.map(f32::to_bits),
            region_snow_inputs_sha256,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RestoreGradeCpuUpdate {
    constants_changed: bool,
    color_cube_changed: bool,
    bloom_changed: bool,
    luminance_bounds_changed: bool,
}

struct RestoreBloomGpu {
    half_width: u32,
    half_height: u32,
    _bright_texture: ComPtr<ID3D11Texture2D>,
    bright_rtv: ComPtr<ID3D11RenderTargetView>,
    bright_srv: ComPtr<ID3D11ShaderResourceView>,
    _scratch_texture: ComPtr<ID3D11Texture2D>,
    scratch_rtv: ComPtr<ID3D11RenderTargetView>,
    scratch_srv: ComPtr<ID3D11ShaderResourceView>,
    bright_vs: ComPtr<ID3D11VertexShader>,
    bright_ps: ComPtr<ID3D11PixelShader>,
    blur_vs: ComPtr<ID3D11VertexShader>,
    blur_ps: ComPtr<ID3D11PixelShader>,
    blur_cb_horizontal: ComPtr<ID3D11Buffer>,
    blur_cb_vertical: ComPtr<ID3D11Buffer>,
    bright_cb: ComPtr<ID3D11Buffer>,
}

const TERRAIN_BUNDLE_FILE: &str = "map_frame_bundle.json";
const WATER_BUNDLE_FILE: &str = "map_frame_water_pass_bundle.json";
const RIVERS_BUNDLE_FILE: &str = "map_frame_rivers_pass_bundle.json";
const TREES_BUNDLE_FILE: &str = "map_frame_trees_pass_bundle.json";

fn tree_layer_note(
    trees: &NativeD3d11TerrainBundle,
    game_resolver: Option<&GameFileResolver>,
) -> String {
    let Some(resolver) = game_resolver else {
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
    pub particle_stream_omitted: bool,
    pub entity_stream_omitted: bool,
    pub weather_stream_omitted: bool,
    pub water_gap: Option<String>,
    pub layer_gaps: Vec<String>,
    pub counter_gpu_draws: usize,
    pub counter_gpu_omission: Option<String>,
    pub snow_mud_h2_receipt_sha256: Option<String>,
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
    submitted_draw_resolver: GameFileResolver,
    displayed_submit: Option<RenderSubmit>,
    c03_camera_replay: Option<C03CameraReplayBinding>,
    restore_grade_resolver: Option<GameFileResolver>,
    restore_postfx_cb: Option<ComPtr<ID3D11Buffer>>,
    restore_color_cube: Option<RestoreColorCubeGpu>,
    restore_bloom: Option<RestoreBloomGpu>,
    restore_region_weights: Option<PostEffectRegionWeightManifest>,
    current_weather_province_snow: Option<WeatherProvinceSnowView>,
    counter_gpu: CounterGpuPass,
    political_cb0: map_viewer::PoliticalSceneCb0,
    recorded_political_bindings: std::collections::BTreeSet<String>,
    reported_omitted_particles: bool,
    reported_omitted_entities: bool,
    reported_omitted_weather: bool,
    last_camera: Option<CapturedCameraKey>,
    last_restore_grade_key: Option<RestoreGradeKey>,
    last_snow_mud_h2_receipt: Option<SnowMudReplayReadbackReceipt>,
    dirty: bool,
    last_render: Option<Duration>,
}

fn recorded_snow_mud_texture<'a>(
    _frame: &WorldFrame,
    render_frame: Option<&'a ProductionRenderFrame>,
) -> Result<Option<(&'a SnowMudTexturePacket, f32)>> {
    let render_frame = render_frame.ok_or_else(|| {
        "recorded SnowMudData blocked: displayed hour has no production render carrier".to_string()
    })?;
    let packet = render_frame.snow_mud_texture.as_ref().ok_or_else(|| {
        "recorded SnowMudData blocked: production render frame has no exact CPU pre-upload packet"
            .to_string()
    })?;
    let fade = f32::from_bits(packet.snow_mud_fade_f32_bits);
    if !fade.is_finite() || !(0.0..=1.0).contains(&fade) {
        return Err("recorded SnowMudData blocked: exact fade bits are not finite in [0,1]".into());
    }
    Ok(Some((packet, fade)))
}

fn required_recorded_snow_mud_texture<'a>(
    frame: &WorldFrame,
    render_frame: Option<&'a ProductionRenderFrame>,
) -> Result<(&'a SnowMudTexturePacket, f32)> {
    recorded_snow_mud_texture(frame, render_frame)?.ok_or_else(|| {
        "recorded SnowMudData blocked: this hour has no present snow/mud view; refusing the captured bundle texture"
            .into()
    })
}

fn production_display_submit(frame: &ProductionRenderFrame) -> Result<Option<&RenderSubmit>> {
    let Some(text) = frame.display_submit_index.as_deref() else {
        if frame.submits.is_empty() {
            return Ok(None);
        }
        return Err(
            "production render frame blocked: nonempty submit set has no display_submit_index"
                .into(),
        );
    };
    let submit_index = text.parse::<u64>().map_err(|_| {
        format!("production render frame blocked: invalid display_submit_index {text:?}")
    })?;
    frame.submit(submit_index).map(Some).ok_or_else(|| {
        format!(
            "production render frame blocked: display_submit_index {submit_index} is not retained"
        )
        .into()
    })
}

fn require_secondary_political_texture(submit: Option<&RenderSubmit>) -> Result<&RenderSubmit> {
    let submit = submit.ok_or_else(|| {
        "occupation stripes DATA_BLOCKED: displayed hour has no production submit carrying ProvinceSecondaryColorMap; refusing the captured bundle texture"
            .to_string()
    })?;
    let mut secondary_packets = submit.political_textures.iter().filter(|packet| {
        packet.kind == PoliticalTextureKind::Secondary
            && packet.target_binding_identity == "ProvinceSecondaryColorMap"
    });
    let packet = secondary_packets.next().ok_or_else(|| {
        "occupation stripes DATA_BLOCKED: displayed submit has no exact ProvinceSecondaryColorMap packet; refusing the captured bundle texture"
            .to_string()
    })?;
    if secondary_packets.next().is_some() {
        return Err(
            "occupation stripes DATA_BLOCKED: displayed submit has multiple ProvinceSecondaryColorMap packets; captured-pass replay cannot choose one"
                .into(),
        );
    }
    let has_same_submit_consumer = submit
        .ordered_static_map_programs
        .iter()
        .flat_map(|program| program.ordered_operations.iter())
        .filter_map(|operation| match &operation.command {
            StaticMapCommand::Draw { draw } => Some(draw),
            StaticMapCommand::ClearColor { .. } | StaticMapCommand::ClearDepthStencil { .. } => {
                None
            }
        })
        .flat_map(|draw| draw.ordered_shader_resources.iter())
        .any(|binding| {
            matches!(
                &binding.source,
                StaticShaderResourceSource::PoliticalTexture {
                    texture_kind: PoliticalTextureKind::Secondary,
                    target_binding_identity,
                    upload_order,
                    payload_sha256,
                    ..
                } if target_binding_identity == "ProvinceSecondaryColorMap"
                    && *upload_order == packet.upload_order
                    && payload_sha256 == &packet.sha256
            )
        });
    if !has_same_submit_consumer {
        return Err(
            "occupation stripes DATA_BLOCKED: ProvinceSecondaryColorMap packet has no exact D80 selected-submit shader-resource binding with the same upload ordinal and payload SHA-256"
                .into(),
        );
    }
    Ok(submit)
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
        let resolver = game_resolver.ok_or(
            "production map rendering requires a launcher-equivalent effective-VFS resolver selected by --game plus the replay mod/DLC profile; captured assets are never used against an unverified default install",
        )?;
        if !resolver.has_content_admission() {
            return Err(
                "production map rendering requires replay-validated used-content admission; unrestricted/default VFS lookup is forbidden"
                    .into(),
            );
        }
        let mut renderer = NativeD3d11TerrainRenderer::from_borrowed_raw(
            gfx.device.as_raw(),
            gfx.context.as_raw(),
        )?;
        renderer.set_water_full(water_full);
        let renderer = if water_full {
            renderer.with_underwater_resolver(resolver.clone())
        } else {
            renderer
        };
        let renderer = renderer.with_reflection_resolver(resolver.clone());
        let mut layer_gaps = Vec::new();
        let terrain_path = session_dir.join(TERRAIN_BUNDLE_FILE);
        let terrain = load_terrain_bundle_for_replay(&terrain_path, Some(resolver))?;
        let base_camera = terrain.production_camera_from_capture()?;
        let camera_defines = hoi4_map_camera_defines_from_bundle(&terrain)?;

        let water_path = session_dir.join(WATER_BUNDLE_FILE);
        let water = if water_path.exists() {
            NativeD3d11TerrainBundle::load_water_with_resolver(&water_path, resolver)?
        } else {
            return Err(format!(
                "captured map rendering requires the water pass bundle: {}",
                water_path.display()
            )
            .into());
        };
        let rivers_path = session_dir.join(RIVERS_BUNDLE_FILE);
        let rivers = if rivers_path.exists() {
            NativeD3d11TerrainBundle::load_rivers_with_resolver(&rivers_path, resolver)?
        } else {
            return Err(format!(
                "captured map rendering requires the rivers pass bundle: {}",
                rivers_path.display()
            )
            .into());
        };
        let trees_path = session_dir.join(TREES_BUNDLE_FILE);
        let trees = if trees_path.exists() {
            let trees = NativeD3d11TerrainBundle::load_trees_with_resolver(&trees_path, resolver)?;
            layer_gaps.push(tree_layer_note(&trees, Some(resolver)));
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
            "replay-driven political textures are DATA_BLOCKED on any displayed hour without exact byte-native packets; ProvinceSecondaryColorMap is never allowed to fall back to the captured PAN2 texture"
                .to_string(),
        );
        let output_srv = create_captured_output_srv(
            gfx,
            prepared.target_format(),
            prepared.output_texture_raw(),
        )?;
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
        let restore_grade_resolver = if restore_postfx.enabled {
            Some(
                game_resolver
                    .ok_or(
                        "--postfx-restore-full needs --game <the game install dir> (or BATTLE_PLAN_VANILLA) for per-frame GRADE2 LUT recomposition",
                    )?
                    .clone(),
            )
        } else {
            None
        };
        let restore_region_weights = if restore_postfx.enabled {
            Some(load_posteffect_region_weight_manifest(resolver).map_err(|error| {
                format!(
                    "--postfx-restore-full RG-E1 blocked: exact posteffect region weights failed: {error}"
                )
            })?)
        } else {
            None
        };
        let restore_postfx_cb = if restore_postfx.enabled {
            Some(create_restore_postfx_constant_buffer(
                gfx,
                restore_postfx.constants,
            )?)
        } else {
            None
        };
        let restore_color_cube = create_restore_color_cube_texture(gfx, &restore_postfx)?;
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
            submitted_draw_resolver: resolver.clone(),
            displayed_submit: None,
            c03_camera_replay: None,
            restore_grade_resolver,
            restore_postfx_cb,
            restore_color_cube,
            restore_bloom,
            restore_region_weights,
            current_weather_province_snow: None,
            counter_gpu: CounterGpuPass::new(resolver.clone(), session_dir.to_path_buf()),
            political_cb0,
            recorded_political_bindings: std::collections::BTreeSet::new(),
            reported_omitted_particles: false,
            reported_omitted_entities: false,
            reported_omitted_weather: false,
            last_camera: None,
            last_restore_grade_key: None,
            last_snow_mud_h2_receipt: None,
            dirty: true,
            last_render: None,
        })
    }

    #[allow(dead_code)]
    fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    fn update_weather_province_snow(&mut self, frame: &WorldFrame) {
        if self.restore_region_weights.is_some() {
            self.current_weather_province_snow = Some(frame.weather_province_snow().clone());
            self.mark_dirty();
        }
    }

    unsafe fn update_recorded_snow_mud(
        &mut self,
        gfx: &Gfx,
        frame: &WorldFrame,
        render_frame: &ProductionRenderFrame,
    ) -> Result<()> {
        let (packet, fade) = required_recorded_snow_mud_texture(frame, Some(render_frame))?;
        let (texture, srv) = create_immutable_texture_with_pitch(
            gfx,
            packet.width as usize,
            packet.height as usize,
            DXGI_FORMAT_B8G8R8A8_UNORM,
            packet.row_pitch as usize,
            &packet.bytes,
            "recorded SnowMudData",
        )?;
        let readback = verify_immutable_texture_readback(gfx, &texture, packet)?;
        let replacements = self.prepared.replace_snow_mud_srv(srv.as_raw())?;
        if replacements == 0 {
            return Err(
                "recorded SnowMudData blocked: no production pass sampled SnowMudTexture".into(),
            );
        }
        let h2_receipt = seal_snow_mud_h2_receipt(
            packet,
            &render_frame.selected_present,
            &readback,
            replacements,
        )?;
        let h2_receipt_path =
            persist_snow_mud_h2_receipt(&h2_receipt, &readback.tight_readback_bytes)?;
        self.political_cb0.snow_mud_fade = Some(fade);
        self.last_snow_mud_h2_receipt = Some(h2_receipt.clone());
        self.mark_dirty();
        eprintln!(
            "replay_app: bound exact SnowMudData pre-upload bytes ({} pass bindings, producer ordinal {}, payload {}, H2 receipt {}, fade {:.6})",
            replacements,
            packet.producer_ordinal,
            packet.payload_sha256,
            h2_receipt.receipt_sha256,
            fade,
        );
        if let Some(path) = h2_receipt_path {
            eprintln!(
                "replay_app: persisted SnowMudData H2 receipt {}",
                path.display()
            );
        }
        Ok(())
    }

    unsafe fn update_recorded_political_textures(
        &mut self,
        gfx: &Gfx,
        render_frame: &ProductionRenderFrame,
    ) -> Result<()> {
        let submit = require_secondary_political_texture(production_display_submit(render_frame)?)?;
        let mut packets: Vec<_> = submit.political_textures.iter().collect();
        packets.sort_by_key(|packet| packet.upload_order);
        let next_bindings: std::collections::BTreeSet<String> = packets
            .iter()
            .map(|packet| packet.target_binding_identity.clone())
            .collect();
        if !self.recorded_political_bindings.is_subset(&next_bindings) {
            return Err(
                "production political textures blocked: displayed submit omits a previously bound target; refusing stale reuse"
                    .into(),
            );
        }

        for packet in packets {
            let format = match packet.format {
                PoliticalTextureFormat::B8g8r8a8UnormSrgb => DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
            };
            let tight_pitch = packet
                .width
                .checked_mul(4)
                .ok_or("production political texture row pitch overflow")?;
            if packet.row_pitch != tight_pitch {
                return Err(format!(
                    "production political texture {} blocked: row_pitch {} is not tight width*4 ({tight_pitch})",
                    packet.target_binding_identity, packet.row_pitch
                )
                .into());
            }
            let (_texture, srv) = create_immutable_texture(
                gfx,
                packet.width as usize,
                packet.height as usize,
                format,
                &packet.bytes,
                &format!(
                    "recorded political texture {}",
                    packet.target_binding_identity
                ),
            )?;
            let replacements = self
                .prepared
                .replace_named_ps_srv(&packet.target_binding_identity, srv.as_raw())?;
            if replacements == 0 {
                return Err(format!(
                    "production political texture {:?} blocked: target binding {:?} is not sampled by the prepared map pass",
                    packet.kind, packet.target_binding_identity
                )
                .into());
            }
        }
        if !next_bindings.is_empty() {
            self.recorded_political_bindings = next_bindings;
            self.mark_dirty();
        }
        Ok(())
    }

    fn update_recorded_submitted_draws(
        &mut self,
        render_frame: &ProductionRenderFrame,
    ) -> Result<()> {
        let next = production_display_submit(render_frame)?.cloned();
        let next_camera_binding = map_viewer::selected_c03_camera_replay_binding(render_frame)?;
        if let (Some(submit), Some(binding)) = (&next, &next_camera_binding) {
            // kills: carrier.render_submit.submit_index — the binding identity is checked
            // against the receipt-derived submit identity, not a carried counter.
            let identity = render_frame
                .retained_submit_identity()
                .ok_or("displayed submit has no derived identity")?;
            if binding.submit_index != identity
                || binding.submit_global_order != submit.global_order
            {
                return Err(
                    "recorded C03 camera blocked: binding belongs to a different display submit"
                        .into(),
                );
            }
        }
        if self.displayed_submit != next || self.c03_camera_replay != next_camera_binding {
            self.displayed_submit = next;
            self.c03_camera_replay = next_camera_binding;
            self.mark_dirty();
        }
        self.reported_omitted_particles = false;
        self.reported_omitted_entities = false;
        self.reported_omitted_weather = false;
        Ok(())
    }

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
            let production_camera = match (
                self.displayed_submit
                    .as_ref()
                    .and_then(|submit| submit.camera_carrier.as_ref()),
                self.c03_camera_replay.as_ref(),
            ) {
                (Some(recorded), Some(binding)) => production_camera_from_recorded_carrier(
                    recorded, binding, gfx.width, gfx.height,
                )?,
                (None, None) => production_camera_from_app_camera(
                    self.camera_projection,
                    self.camera_defines,
                    camera,
                    gfx.width,
                )?,
                _ => return Err(
                    "recorded C03 camera blocked: carrier/consumer binding cardinality mismatch"
                        .into(),
                ),
            };
            let frame_camera_projection =
                ProductionMapCameraProjection::from_camera(production_camera)?;
            unsafe {
                self.recompose_restore_grade_for_frame(gfx, production_camera, day_night_hour)?;
            }
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
                    frame_camera_projection,
                    production_camera,
                    wrap_offset,
                )?;
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
                    cull_world_rect: shifted_cull_world_rect(cull_world_rect, wrap_offset),
                    tree_season_cb: None,
                    map_clock_time,
                };
                self.renderer
                    .render_prepared_production_map(&self.prepared, &options)?;
            }
            if let Some(submit) = self.displayed_submit.as_ref() {
                let (color_width, color_height, color_format, color_samples, color_quality) =
                    self.prepared.output_color_desc();
                let (depth_width, depth_height, depth_format, depth_samples, depth_quality) =
                    self.prepared.output_depth_desc();
                let target = crate::submitted_draw::SubmittedMainSceneTarget {
                    color_identity: None,
                    depth_identity: None,
                    color_rtv: self.prepared.output_render_target_view_raw(),
                    depth_dsv: self.prepared.output_depth_stencil_view_raw(),
                    color_width,
                    color_height,
                    color_format,
                    color_samples,
                    color_quality,
                    depth_width,
                    depth_height,
                    depth_format,
                    depth_samples,
                    depth_quality,
                };
                let stats = unsafe {
                    crate::submitted_draw::execute_submitted_draws(
                        gfx,
                        &self.submitted_draw_resolver,
                        submit,
                        &target,
                    )?
                };
                eprintln!(
                    "replay_app: executed portable submit in authoritative order ({} entity, {} particle, {} weather joins)",
                    stats.entity_draws,
                    stats.particle_draws,
                    stats.weather_joins,
                );
            }
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
        self.composite(gfx)?;
        self.counter_gpu.draw(gfx)
    }

    unsafe fn recompose_restore_grade_for_frame(
        &mut self,
        gfx: &Gfx,
        production_camera: ProductionMapCamera,
        day_night_hour: Option<f32>,
    ) -> Result<Option<RestoreGradeCpuUpdate>> {
        if !(self.postfx_enabled && self.restore_postfx.enabled) {
            return Ok(None);
        }
        let night = day_night_hour
            .map(|hour| !(0.25..0.75).contains(&hour))
            .unwrap_or(false);
        let resolver = self.restore_grade_resolver.as_ref().ok_or(
            "--postfx-restore-full GRADE2 blocked: missing game-file resolver for per-frame grade recomposition",
        )?;
        let region_winter = match self.restore_region_weights.as_ref() {
            Some(weights) => posteffect_region_winter_input_for_frame(
                resolver,
                weights,
                self.current_weather_province_snow.as_ref(),
                production_camera.position,
                night,
            )?,
            None => None,
        };
        let state = GradingSelectionState {
            camera: production_camera.position,
            night,
            winter: false,
            region_winter_factor: region_winter.as_ref().map(|input| input.factor()),
        };
        let key = RestoreGradeKey::from_state(
            state,
            region_winter.as_ref().map(|input| input.used_rows_sha256),
        );
        if self.last_restore_grade_key == Some(key) {
            return Ok(Some(RestoreGradeCpuUpdate::default()));
        }
        let grade = file_derived_grade_per_frame(resolver, state, None).map_err(|err| {
            format!(
                "--postfx-restore-full GRADE2 blocked: per-frame grade recomposition failed: {err}"
            )
        })?;
        let bloom = restore_bloom_law_for_grade(&grade)?;
        let update = apply_restore_grade_cpu(&mut self.restore_postfx, &grade, bloom);

        if update.constants_changed {
            if let Some(cb) = self.restore_postfx_cb.as_ref() {
                write_restore_postfx_constants(
                    gfx,
                    cb,
                    self.restore_postfx.constants,
                    "Map restore-postfx CB for per-frame GRADE2 constants",
                )?;
            }
        }
        if update.color_cube_changed {
            if let Some(color_cube) = self.restore_color_cube.as_ref() {
                update_restore_color_cube_texture(
                    gfx,
                    color_cube,
                    &self.restore_postfx.color_cube_rgba8,
                )?;
            }
        }
        if update.bloom_changed {
            self.restore_bloom = create_restore_bloom_gpu(
                gfx,
                &self.restore_postfx,
                self.prepared.target_width(),
                self.prepared.target_height(),
                self.prepared.target_format(),
            )?;
        }
        self.last_restore_grade_key = Some(key);
        if update.constants_changed
            || update.color_cube_changed
            || update.bloom_changed
            || update.luminance_bounds_changed
        {
            eprintln!(
                "replay_app: GRADE2 per-frame grade recomposed: {}; uploads constants={}, LUT={}, bloom={}; luminance bounds changed={}",
                grade.source_note,
                update.constants_changed,
                update.color_cube_changed,
                update.bloom_changed,
                update.luminance_bounds_changed,
            );
        }
        Ok(Some(update))
    }

    fn composite(&self, gfx: &Gfx) -> Result<()> {
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
                let srvs = [bloom.bright_srv.as_raw()];
                (*context).PSSetShaderResources(8, 1, srvs.as_ptr());
            }
            let samplers = [self.composite_sampler.as_raw()];
            (*context).PSSetSamplers(0, 1, samplers.as_ptr());
            (*context).Draw(3, 0);
            let null_srvs: [*mut ID3D11ShaderResourceView; 1] = [ptr::null_mut()];
            (*context).PSSetShaderResources(0, 1, null_srvs.as_ptr());
            if self.restore_bloom.is_some() {
                (*context).PSSetShaderResources(8, 1, null_srvs.as_ptr());
            }
        }
        Ok(())
    }

    unsafe fn run_bloom_chain(&self, gfx: &Gfx, bloom: &RestoreBloomGpu) -> Result<()> {
        let context = gfx.context.as_raw();
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

        (*context).OMSetRenderTargets(1, [bloom.bright_rtv.as_raw()].as_ptr(), ptr::null_mut());
        (*context).VSSetShader(bloom.bright_vs.as_raw(), ptr::null(), 0);
        (*context).PSSetShader(bloom.bright_ps.as_raw(), ptr::null(), 0);
        (*context).PSSetConstantBuffers(0, 1, [bloom.bright_cb.as_raw()].as_ptr());
        (*context).PSSetShaderResources(0, 1, [self.output_srv.as_raw()].as_ptr());
        (*context).Draw(3, 0);
        (*context).OMSetRenderTargets(1, null_srv.as_ptr() as *const *mut _, ptr::null_mut());
        (*context).PSSetShaderResources(0, 1, null_srv.as_ptr());

        (*context).OMSetRenderTargets(1, [bloom.scratch_rtv.as_raw()].as_ptr(), ptr::null_mut());
        (*context).VSSetShader(bloom.blur_vs.as_raw(), ptr::null(), 0);
        (*context).PSSetShader(bloom.blur_ps.as_raw(), ptr::null(), 0);
        (*context).PSSetConstantBuffers(0, 1, [bloom.blur_cb_horizontal.as_raw()].as_ptr());
        (*context).PSSetShaderResources(0, 1, [bloom.bright_srv.as_raw()].as_ptr());
        (*context).Draw(3, 0);
        (*context).OMSetRenderTargets(1, null_srv.as_ptr() as *const *mut _, ptr::null_mut());
        (*context).PSSetShaderResources(0, 1, null_srv.as_ptr());

        (*context).OMSetRenderTargets(1, [bloom.bright_rtv.as_raw()].as_ptr(), ptr::null_mut());
        (*context).PSSetConstantBuffers(0, 1, [bloom.blur_cb_vertical.as_raw()].as_ptr());
        (*context).PSSetShaderResources(0, 1, [bloom.scratch_srv.as_raw()].as_ptr());
        (*context).Draw(3, 0);
        (*context).OMSetRenderTargets(1, null_srv.as_ptr() as *const *mut _, ptr::null_mut());
        (*context).PSSetShaderResources(0, 1, null_srv.as_ptr());

        Ok(())
    }

    unsafe fn update_exposure_from_scene_luminance(
        &self,
        gfx: &Gfx,
        bounds: LuminanceClampBounds,
        cb: &ComPtr<ID3D11Buffer>,
    ) -> Result<AverageLuminanceReduction> {
        let width = self.prepared.target_width();
        let height = self.prepared.target_height();
        let format = self.prepared.target_format();
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

        let mut constants = self.restore_postfx.constants;
        constants.exposure[0] = self.restore_postfx.middle_grey;
        constants.exposure[1] = reduction.r16_quantized;
        constants.exposure[2] = 1.0;

        write_restore_postfx_constants(
            gfx,
            cb,
            constants,
            "Map restore-postfx CB to update measured exposure",
        )?;
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

    fn dump_hdr_mainscene(&self, gfx: &Gfx, path: &std::path::Path) -> Result<()> {
        let width = self.prepared.target_width();
        let height = self.prepared.target_height();
        let format = self.prepared.target_format();
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
            .map(|(x, z)| (f64::from(x), map_h - f64::from(z))))
    }

    #[allow(dead_code)]
    fn production_camera_height(&self, camera: &Camera, screen_w: f64) -> Result<f32> {
        let production_camera = production_camera_from_app_camera(
            self.camera_projection,
            self.camera_defines,
            camera,
            screen_w.max(1.0) as u32,
        )?;
        Ok(production_camera.position[1])
    }

    #[allow(dead_code)]
    fn map_to_screen_3d(
        &self,
        camera: &Camera,
        map_x: f64,
        map_y: f64,
        elevation: f32,
        screen_w: f64,
        screen_h: f64,
    ) -> Result<Option<(f64, f64)>> {
        self.map_to_screen_3d_at(
            camera, map_x, map_y, elevation, screen_w, screen_w, screen_h,
        )
    }

    #[allow(dead_code)]
    fn map_to_screen_3d_at(
        &self,
        camera: &Camera,
        map_x: f64,
        map_y: f64,
        elevation: f32,
        cam_screen_w: f64,
        out_screen_w: f64,
        out_screen_h: f64,
    ) -> Result<Option<(f64, f64)>> {
        let production_camera = production_camera_from_app_camera(
            self.camera_projection,
            self.camera_defines,
            camera,
            cam_screen_w.max(1.0) as u32,
        )?;
        Ok(project_map_point_3d(
            &production_camera,
            camera.map_h(),
            map_x,
            map_y,
            elevation,
            out_screen_w,
            out_screen_h,
        ))
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
            particle_stream_omitted: self.reported_omitted_particles,
            entity_stream_omitted: self.reported_omitted_entities,
            weather_stream_omitted: self.reported_omitted_weather,
            water_gap: self.water_gap.clone(),
            layer_gaps: self.layer_gaps.clone(),
            counter_gpu_draws: self.counter_gpu.last_outcome().draw_count,
            counter_gpu_omission: self
                .counter_gpu
                .last_outcome()
                .omission
                .as_ref()
                .map(|reason| reason.code().to_string()),
            snow_mud_h2_receipt_sha256: self
                .last_snow_mud_h2_receipt
                .as_ref()
                .map(|receipt| receipt.receipt_sha256.clone()),
        }
    }
}

pub struct MapRenderer {
    captured: CapturedPassLayer,
    max_state_id: u32,
}

impl MapRenderer {
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
            let restore_postfx = restore_full_postfx_from_files(
                postfx_restore_full,
                game_resolver,
                bundle.width,
                bundle.height,
                None,
            )?;
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

    pub fn update_state_politics(
        &mut self,
        gfx: &Gfx,
        _bundle: &MapBundle,
        frame: Option<&WorldFrame>,
        _owners: &BTreeMap<u32, String>,
        controllers: &BTreeMap<u32, String>,
        country_colors: &BTreeMap<String, [u8; 3]>,
    ) -> Result<()> {
        let _ = crate::palette::build_state_color_table(
            self.max_state_id,
            controllers,
            country_colors,
        )?;
        let frame = frame.ok_or(
            "occupation stripes DATA_BLOCKED: displayed hour has no WorldFrame carrying ProvinceSecondaryColorMap; refusing the captured bundle texture",
        )?;
        let render_frame = frame.production_render().ok_or(
            "occupation stripes DATA_BLOCKED: displayed hour has no production_render carrier carrying ProvinceSecondaryColorMap; refusing the captured bundle texture",
        )?;
        self.captured.update_weather_province_snow(frame);
        unsafe {
            self.captured
                .update_recorded_snow_mud(gfx, frame, render_frame)?;
            self.captured
                .update_recorded_submitted_draws(render_frame)?;
            self.captured
                .update_recorded_political_textures(gfx, render_frame)?;
        }
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

    pub fn snow_mud_h2_receipt(&self) -> Option<&SnowMudReplayReadbackReceipt> {
        self.captured.last_snow_mud_h2_receipt.as_ref()
    }

    #[allow(dead_code)]
    pub(crate) fn update_counter_batch(
        &mut self,
        batch: map_viewer::CounterRenderBatch,
        admission: Option<CounterGpuFrameAdmission>,
    ) {
        self.captured.counter_gpu.update(batch, admission);
        self.captured.mark_dirty();
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

    #[allow(dead_code)]
    pub fn production_camera_height(&self, camera: &Camera, screen_w: f64) -> Result<f32> {
        self.captured.production_camera_height(camera, screen_w)
    }

    #[allow(dead_code)]
    pub fn map_to_screen_3d(
        &self,
        camera: &Camera,
        map_x: f64,
        map_y: f64,
        elevation: f32,
        screen_w: f64,
        screen_h: f64,
    ) -> Result<Option<(f64, f64)>> {
        self.captured
            .map_to_screen_3d(camera, map_x, map_y, elevation, screen_w, screen_h)
    }

    pub fn captured_pass_stats(&self) -> Option<CapturedPassStats> {
        Some(self.captured.stats())
    }

    pub fn dump_hdr_mainscene(&self, gfx: &Gfx, path: &std::path::Path) -> Result<()> {
        self.captured.dump_hdr_mainscene(gfx, path)
    }
}

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

pub(crate) unsafe fn create_immutable_texture(
    gfx: &Gfx,
    width: usize,
    height: usize,
    format: u32,
    bytes: &[u8],
    role: &str,
) -> Result<(ComPtr<ID3D11Texture2D>, ComPtr<ID3D11ShaderResourceView>)> {
    let row_pitch = width.checked_mul(4).ok_or("texture row pitch overflow")?;
    create_immutable_texture_with_pitch(gfx, width, height, format, row_pitch, bytes, role)
}

unsafe fn create_immutable_texture_with_pitch(
    gfx: &Gfx,
    width: usize,
    height: usize,
    format: u32,
    row_pitch: usize,
    bytes: &[u8],
    role: &str,
) -> Result<(ComPtr<ID3D11Texture2D>, ComPtr<ID3D11ShaderResourceView>)> {
    let expected = row_pitch
        .checked_mul(height)
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
        SysMemPitch: UINT::try_from(row_pitch)?,
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

struct SnowMudTextureReadbackProof {
    tight_readback_bytes: Vec<u8>,
    readback_sha256: String,
    mapped_row_pitch: u32,
    verified_row_count: u32,
    verified_tight_byte_count: u64,
}

unsafe fn verify_immutable_texture_readback(
    gfx: &Gfx,
    texture: &ComPtr<ID3D11Texture2D>,
    packet: &SnowMudTexturePacket,
) -> Result<SnowMudTextureReadbackProof> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: packet.width,
        Height: packet.height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
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
    let hr = (*gfx.device.as_raw()).CreateTexture2D(&desc, ptr::null(), &mut staging);
    hr_ok(hr, "CreateTexture2D staging for SnowMudData readback")?;
    let staging = ComPtr::from_raw(staging, "SnowMudData readback staging")?;
    let context = gfx.context.as_raw();
    (*context).CopyResource(staging.as_raw() as *mut _, texture.as_raw() as *mut _);
    gfx.check_void_device_call("CopyResource SnowMudData readback")?;
    let mut mapped: D3D11_MAPPED_SUBRESOURCE = mem::zeroed();
    let hr = (*context).Map(
        staging.as_raw() as *mut _,
        0,
        D3D11_MAP_READ,
        0,
        &mut mapped,
    );
    hr_ok(hr, "Map staging SnowMudData readback")?;
    let verification = (|| -> Result<SnowMudTextureReadbackProof> {
        let source_pitch = packet.row_pitch as usize;
        let mapped_pitch = mapped.RowPitch as usize;
        if mapped.pData.is_null() || mapped_pitch < source_pitch {
            return Err("SnowMudData readback returned a null/short mapped row".into());
        }
        let mapped_bytes = std::slice::from_raw_parts(
            mapped.pData as *const u8,
            mapped_pitch
                .checked_mul(packet.height as usize)
                .ok_or("SnowMudData mapped byte length overflow")?,
        );
        let mut readback_hasher = Sha256::new();
        let mut tight_readback_bytes = Vec::with_capacity(packet.bytes.len());
        for row in 0..packet.height as usize {
            let source_start = row * source_pitch;
            let mapped_start = row * mapped_pitch;
            if mapped_bytes[mapped_start..mapped_start + source_pitch]
                != packet.bytes[source_start..source_start + source_pitch]
            {
                return Err(format!(
                    "SnowMudData immutable upload/readback differs at row {row}; refusing H2 receipt"
                )
                .into());
            }
            let tight_row = &mapped_bytes[mapped_start..mapped_start + source_pitch];
            readback_hasher.update(tight_row);
            tight_readback_bytes.extend_from_slice(tight_row);
        }
        let readback_sha256 = format!("{:x}", readback_hasher.finalize());
        Ok(SnowMudTextureReadbackProof {
            tight_readback_bytes,
            readback_sha256,
            mapped_row_pitch: mapped.RowPitch,
            verified_row_count: packet.height,
            verified_tight_byte_count: u64::from(packet.row_pitch) * u64::from(packet.height),
        })
    })();
    (*context).Unmap(staging.as_raw() as *mut _, 0);
    verification
}

fn seal_snow_mud_h2_receipt(
    packet: &SnowMudTexturePacket,
    selected_present: &replay_core::SelectedPresentReceipt,
    readback: &SnowMudTextureReadbackProof,
    replaced_binding_count: usize,
) -> Result<SnowMudReplayReadbackReceipt> {
    let replaced_binding_count = u32::try_from(replaced_binding_count)
        .map_err(|_| "SnowMudData replacement count exceeds the portable receipt")?;
    let mut receipt = SnowMudReplayReadbackReceipt {
        schema: replay_core::SNOW_MUD_H2_READBACK_SCHEMA.into(),
        process_start_nonce: packet.process_start_nonce,
        hook_epoch_nonce: packet.hook_epoch_nonce,
        producer_ordinal: packet.producer_ordinal,
        producer_game_hour_raw: packet.producer_game_hour_raw,
        display_game_hour_raw: packet.display_game_hour_raw,
        selected_candidate_id: selected_present.selected_candidate_id,
        selected_qpc_ticks: selected_present.selected_qpc_ticks,
        selected_present_sha256: packet.selected_present_sha256.clone(),
        snow_mud_fade_f32_bits: packet.snow_mud_fade_f32_bits,
        snow_mud_fade_consumer_count: packet.snow_mud_fade_consumer_count,
        snow_mud_fade_consumer_set_sha256: packet.snow_mud_fade_consumer_set_sha256.clone(),
        observation_sha256: packet.observation_sha256.clone(),
        payload_sha256: packet.payload_sha256.clone(),
        readback_sha256: readback.readback_sha256.clone(),
        width: packet.width,
        height: packet.height,
        source_row_pitch: packet.row_pitch,
        mapped_row_pitch: readback.mapped_row_pitch,
        verified_row_count: readback.verified_row_count,
        verified_tight_byte_count: readback.verified_tight_byte_count,
        replaced_binding_count,
        receipt_sha256: String::new(),
    };
    receipt.receipt_sha256 = receipt.computed_receipt_sha256()?;
    receipt.validate_against(packet, selected_present)?;
    Ok(receipt)
}

pub(crate) fn seal_product_snow_mud_h2_receipt(
    packet: &SnowMudTexturePacket,
    selected_present: &replay_core::SelectedPresentReceipt,
    proof: &map_viewer::SnowMudD3dExecutionProof,
) -> Result<SnowMudReplayReadbackReceipt> {
    let readback = SnowMudTextureReadbackProof {
        tight_readback_bytes: proof.tight_readback_bytes.clone(),
        readback_sha256: proof.readback_sha256.clone(),
        mapped_row_pitch: proof.mapped_row_pitch,
        verified_row_count: proof.verified_row_count,
        verified_tight_byte_count: proof.verified_tight_byte_count,
    };
    seal_snow_mud_h2_receipt(
        packet,
        selected_present,
        &readback,
        proof.replaced_binding_count as usize,
    )
}

pub(crate) fn persist_snow_mud_h2_receipt(
    receipt: &SnowMudReplayReadbackReceipt,
    tight_readback_bytes: &[u8],
) -> Result<Option<PathBuf>> {
    let Some(directory) = std::env::var_os("REPLAY_SNOW_MUD_H2_RECEIPT_DIR") else {
        return Ok(None);
    };
    if directory.is_empty() {
        return Err("REPLAY_SNOW_MUD_H2_RECEIPT_DIR is empty".into());
    }
    let directory = PathBuf::from(directory);
    std::fs::create_dir_all(&directory)?;
    if tight_readback_bytes.len() as u64 != receipt.verified_tight_byte_count
        || format!("{:x}", Sha256::digest(tight_readback_bytes)) != receipt.readback_sha256
    {
        return Err("SnowMudData H2 raw readback bytes differ from the sealed receipt".into());
    }
    let stem = format!(
        "snow-mud-h2-p{}-{}",
        receipt.producer_ordinal, receipt.observation_sha256
    );
    let path = directory.join(format!("{stem}.json"));
    let payload_path = directory.join(format!("{stem}.tight-bgra8"));
    let mut bytes = serde_json::to_vec_pretty(receipt)?;
    bytes.push(b'\n');
    persist_create_new_or_equal(&payload_path, tight_readback_bytes, "raw readback payload")?;
    persist_create_new_or_equal(&path, &bytes, "receipt")?;
    Ok(Some(path))
}

pub(crate) fn seal_product_occupation_secondary_h2_receipt(
    packet: &replay_core::PoliticalTexturePacket,
    selected_present: &replay_core::SelectedPresentReceipt,
    proof: &map_viewer::OccupationSecondaryD3dExecutionProof,
) -> Result<OccupationSecondaryReplayReadbackReceipt> {
    let mut receipt = OccupationSecondaryReplayReadbackReceipt {
        schema: replay_core::OCCUPATION_SECONDARY_H2_READBACK_SCHEMA.into(),
        target_binding_identity: packet.target_binding_identity.clone(),
        upload_order: packet.upload_order,
        selected_present_sha256: selected_present.computed_sha256()?,
        payload_sha256: packet.sha256.clone(),
        readback_sha256: proof.readback_sha256.clone(),
        width: packet.width,
        height: packet.height,
        source_row_pitch: packet.row_pitch,
        mapped_row_pitch: proof.mapped_row_pitch,
        verified_row_count: proof.verified_row_count,
        verified_tight_byte_count: proof.verified_tight_byte_count,
        terrain_t6_binding_count: proof.terrain_t6_binding_count,
        water_t3_binding_count: proof.water_t3_binding_count,
        receipt_sha256: String::new(),
    };
    receipt.receipt_sha256 = receipt.computed_receipt_sha256()?;
    receipt.validate_against(packet, selected_present)?;
    Ok(receipt)
}

pub(crate) fn persist_occupation_secondary_h2_receipt(
    receipt: &OccupationSecondaryReplayReadbackReceipt,
    tight_readback_bytes: &[u8],
) -> Result<Option<PathBuf>> {
    let Some(directory) = std::env::var_os("REPLAY_OCCUPATION_H2_RECEIPT_DIR") else {
        return Ok(None);
    };
    if directory.is_empty() {
        return Err("REPLAY_OCCUPATION_H2_RECEIPT_DIR is empty".into());
    }
    let directory = PathBuf::from(directory);
    std::fs::create_dir_all(&directory)?;
    if tight_readback_bytes.len() as u64 != receipt.verified_tight_byte_count
        || format!("{:x}", Sha256::digest(tight_readback_bytes)) != receipt.readback_sha256
    {
        return Err(
            "occupation secondary H2 raw readback bytes differ from the sealed receipt".into(),
        );
    }
    let stem = format!(
        "occupation-secondary-h2-u{}-{}",
        receipt.upload_order, receipt.payload_sha256
    );
    let path = directory.join(format!("{stem}.json"));
    let payload_path = directory.join(format!("{stem}.tight-bgra8"));
    let mut bytes = serde_json::to_vec_pretty(receipt)?;
    bytes.push(b'\n');
    persist_create_new_or_equal(
        &payload_path,
        tight_readback_bytes,
        "occupation raw readback payload",
    )?;
    persist_create_new_or_equal(&path, &bytes, "occupation receipt")?;
    Ok(Some(path))
}

fn persist_create_new_or_equal(path: &Path, bytes: &[u8], label: &str) -> Result<()> {
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut file) => {
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if std::fs::read(path)? != bytes {
                return Err(format!(
                    "SnowMudData H2 {label} path already exists with different bytes: {}",
                    path.display()
                )
                .into());
            }
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

unsafe fn create_linear_clamp_sampler(gfx: &Gfx) -> Result<ComPtr<ID3D11SamplerState>> {
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
    validate_restore_color_cube_bytes(&restore.color_cube_rgba8, "RestoreScene ColorCube")?;
    let desc = D3D11_TEXTURE2D_DESC {
        Width: 1024,
        Height: 32,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let init = D3D11_SUBRESOURCE_DATA {
        pSysMem: restore.color_cube_rgba8.as_ptr() as *const _,
        SysMemPitch: 1024 * 4,
        SysMemSlicePitch: 0,
    };
    let mut texture: *mut ID3D11Texture2D = ptr::null_mut();
    let hr = (*gfx.device.as_raw()).CreateTexture2D(&desc, &init, &mut texture);
    hr_ok(hr, "CreateTexture2D for RestoreScene ColorCube")?;
    let texture = ComPtr::from_raw(texture, "RestoreScene ColorCube")?;
    let mut srv: *mut ID3D11ShaderResourceView = ptr::null_mut();
    let hr = (*gfx.device.as_raw()).CreateShaderResourceView(
        texture.as_raw() as *mut _,
        ptr::null(),
        &mut srv,
    );
    hr_ok(hr, "CreateShaderResourceView for RestoreScene ColorCube")?;
    let srv = ComPtr::from_raw(srv, "RestoreScene ColorCube")?;
    Ok(Some(RestoreColorCubeGpu {
        _texture: texture,
        srv,
    }))
}

fn validate_restore_color_cube_bytes(bytes: &[u8], role: &str) -> Result<()> {
    let expected = 1024 * 32 * 4;
    if bytes.len() != expected {
        return Err(format!("{role} has {} bytes, expected {expected}", bytes.len()).into());
    }
    Ok(())
}

unsafe fn update_restore_color_cube_texture(
    gfx: &Gfx,
    cube: &RestoreColorCubeGpu,
    bytes: &[u8],
) -> Result<()> {
    validate_restore_color_cube_bytes(bytes, "RestoreScene ColorCube update")?;
    (*gfx.context.as_raw()).UpdateSubresource(
        cube._texture.as_raw() as *mut _,
        0,
        ptr::null(),
        bytes.as_ptr() as *const _,
        1024 * 4,
        0,
    );
    gfx.check_void_device_call("UpdateSubresource RestoreScene ColorCube")
}

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

unsafe fn create_immutable_constant_buffer<T: Copy>(
    gfx: &Gfx,
    value: &T,
    role: &str,
) -> Result<ComPtr<ID3D11Buffer>> {
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

    let bright_constants = BloomBrightConstants {
        threshold_emissive_inv_size: [
            law.params.bright_threshold,
            law.params.emissive_bloom_strength,
            1.0 / source_width as f32,
            1.0 / source_height as f32,
        ],
    };
    let bright_cb = create_immutable_constant_buffer(gfx, &bright_constants, "bloom bright CB")?;

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

const RESTORE_GRADE_VIEW_CAMERA_HEIGHT: f32 = 2958.0;

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
        region_winter_factor: None,
    };
    let grade = file_derived_grade(resolver, state, None)?;
    let bloom = restore_bloom_law_for_grade(&grade)?;
    let luminance_bounds = LuminanceClampBounds {
        min_hdr_adjustment: grade.posteffect_scalars.min_hdr_adjustment,
        max_hdr_adjustment: grade.posteffect_scalars.max_hdr_adjustment,
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
        bloom.params.bloom_width,
        bloom.params.bloom_scale,
        bloom.params.bright_threshold,
        bloom.params.emissive_bloom_strength,
    );
    restore.bloom = Some(bloom);
    Ok(restore)
}

fn reject_snow_mud_env_overrides(
    snow_mud_override: Option<std::ffi::OsString>,
    snow_mud_fade: Option<std::ffi::OsString>,
) -> Result<()> {
    if snow_mud_override.is_some() {
        return Err(
            crate::blockers::reason(crate::blockers::BlockerId::SnowMudTextureOverride).into(),
        );
    }
    if snow_mud_fade.is_some() {
        return Err(
            crate::blockers::reason(crate::blockers::BlockerId::SnowMudFadeOverride).into(),
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
    let bundle = load_restore_scene_bundle_session_dir(capture_dir)?.ok_or_else(|| {
        format!(
            "no {RESTORE_PASS_BUNDLE_FILE} or retained RestoreScene manifest in {}",
            capture_dir.display()
        )
    })?;
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
        region_winter_factor: None,
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

    fn from_file_grade(middle_grey: f32, average_luminance: f32) -> Self {
        let mut constants = Self::identity();
        constants.color_balance_use_lut = [1.0, 1.0, 1.0, 1.0];
        if average_luminance.is_finite() && average_luminance > 0.0 {
            constants.exposure = [middle_grey, average_luminance, 1.0, 0.0];
        }
        constants
    }
}

fn restore_bloom_law_for_grade(grade: &FileDerivedGrade) -> Result<RestoreBloomLaw> {
    let params = BloomParams {
        bloom_width: grade.posteffect_scalars.bloom_width,
        bloom_scale: grade.posteffect_scalars.bloom_scale,
        bright_threshold: grade.posteffect_scalars.bright_threshold,
        emissive_bloom_strength: grade.emissive_bloom_strength,
    };
    params.validate()?;
    let kernel = BloomBlurKernel::from_gaussian(params.bloom_width, params.bloom_scale)?;
    Ok(RestoreBloomLaw { params, kernel })
}

fn apply_restore_grade_cpu(
    restore: &mut RestoreFullPostfx,
    grade: &FileDerivedGrade,
    bloom: RestoreBloomLaw,
) -> RestoreGradeCpuUpdate {
    let constants = RestorePostfxConstants::from_file_grade(
        grade.resolved.middle_grey,
        grade.average_luminance,
    );
    let luminance_bounds = Some(LuminanceClampBounds {
        min_hdr_adjustment: grade.posteffect_scalars.min_hdr_adjustment,
        max_hdr_adjustment: grade.posteffect_scalars.max_hdr_adjustment,
    });
    let update = RestoreGradeCpuUpdate {
        constants_changed: restore.constants != constants
            || restore.middle_grey.to_bits() != grade.resolved.middle_grey.to_bits(),
        color_cube_changed: restore.color_cube_rgba8 != grade.color_cube_rgba8,
        bloom_changed: restore.bloom != Some(bloom),
        luminance_bounds_changed: restore.luminance_bounds != luminance_bounds,
    };
    restore.constants = constants;
    restore.middle_grey = grade.resolved.middle_grey;
    restore.luminance_bounds = luminance_bounds;
    if update.color_cube_changed {
        restore.color_cube_rgba8 = grade.color_cube_rgba8.clone();
    }
    restore.bloom = Some(bloom);
    update
}

unsafe fn write_restore_postfx_constants(
    gfx: &Gfx,
    cb: &ComPtr<ID3D11Buffer>,
    constants: RestorePostfxConstants,
    role: &str,
) -> Result<()> {
    let context = gfx.context.as_raw();
    let mut mapped: D3D11_MAPPED_SUBRESOURCE = mem::zeroed();
    let hr = (*context).Map(
        cb.as_raw() as *mut _,
        0,
        D3D11_MAP_WRITE_DISCARD,
        0,
        &mut mapped,
    );
    hr_ok(hr, role)?;
    ptr::copy_nonoverlapping(
        &constants as *const RestorePostfxConstants as *const u8,
        mapped.pData as *mut u8,
        mem::size_of::<RestorePostfxConstants>(),
    );
    (*context).Unmap(cb.as_raw() as *mut _, 0);
    Ok(())
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

fn project_map_point_3d(
    production_camera: &ProductionMapCamera,
    map_h: f64,
    map_x: f64,
    map_y: f64,
    elevation: f32,
    out_screen_w: f64,
    out_screen_h: f64,
) -> Option<(f64, f64)> {
    let ground_z = map_h as f32 - map_y as f32;
    production_camera
        .world_to_screen(
            [map_x as f32, elevation, ground_z],
            out_screen_w.max(1.0) as f32,
            out_screen_h.max(1.0) as f32,
        )
        .map(|(sx, sy)| (f64::from(sx), f64::from(sy)))
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
    if let Some(viewer_state) = camera.viewer_state() {
        return viewer_state.production_camera();
    }

    let screen_w = screen_w.max(1) as f32;
    let height = projection.height_for_screen_zoom(
        camera.zoom as f32,
        screen_w,
        defines.min_height(),
        defines.max_height(),
    )?;
    let ground_z = camera.map_h() as f32 - camera.center_y as f32;
    projection.camera_for_ground_center(camera.center_x as f32, ground_z, height)
}

fn production_camera_from_recorded_carrier(
    carrier: &replay_core::AuthenticatedCameraCarrier,
    binding: &map_viewer::C03CameraReplayBinding,
    screen_w: u32,
    screen_h: u32,
) -> Result<ProductionMapCamera> {
    map_viewer::validate_c03_camera_consumer(carrier, binding)?;
    let [viewport_x, viewport_y, viewport_w, viewport_h] = carrier.viewport_i32;
    if viewport_x != 0
        || viewport_y != 0
        || viewport_w != i32::try_from(screen_w).unwrap_or(-1)
        || viewport_h != i32::try_from(screen_h).unwrap_or(-1)
    {
        return Err(format!(
            "recorded C02 camera blocked: viewport [{viewport_x},{viewport_y},{viewport_w},{viewport_h}] does not match replay target {screen_w}x{screen_h}"
        )
        .into());
    }
    let bits = carrier.view_projection_f32_bits();
    let matrix = std::array::from_fn(|row| {
        std::array::from_fn(|column| f32::from_bits(bits[row * 4 + column]))
    });
    ProductionMapCamera::new(matrix, carrier.current_eye_f32_bits.map(f32::from_bits))
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_core::{ParticleRuntimeMode, RenderAuthority, RenderCheckpoint};

    fn c03_recorded_camera_fixture() -> (
        replay_core::AuthenticatedCameraCarrier,
        map_viewer::C03CameraReplayBinding,
    ) {
        let current_eye = [10.0f32.to_bits(), 20.0f32.to_bits(), 30.0f32.to_bits()];
        let current_look = [11.0f32.to_bits(), 0.0f32.to_bits(), 31.0f32.to_bits()];
        let target_eye = [15.0f32.to_bits(), 16.0f32.to_bits(), 17.0f32.to_bits()];
        let target_look = [18.0f32.to_bits(), 0.0f32.to_bits(), 19.0f32.to_bits()];
        let mut bytes = vec![0u8; replay_core::A729_CAMERA_RENDER_VIEW_BYTES];
        for (index, value) in current_eye.into_iter().enumerate() {
            bytes[0x180 + index * 4..0x184 + index * 4].copy_from_slice(&value.to_le_bytes());
        }
        for (index, value) in current_look.into_iter().enumerate() {
            bytes[0x18c + index * 4..0x190 + index * 4].copy_from_slice(&value.to_le_bytes());
        }
        let mut embedded = [0u8; 16];
        embedded[4..8].copy_from_slice(&4i32.to_le_bytes());
        embedded[8..12].copy_from_slice(&5i32.to_le_bytes());
        let event_batch = replay_core::A729CameraEventBatch {
            producer_rva: replay_core::A729_CAMERA_UPDATE_RVA,
            owner_kind: replay_core::A729CameraOwnerKind::InGame,
            owner_vtable_rva: 0x0295_2510,
            action_source_owner_offset: 0x508,
            cursor_source_owner_offset: 0x500,
            event_list_owner_offset: 0x758,
            frame_scalar_f32_bits: Some(0.1f32.to_bits()),
            producer_flag_0_u8: 1,
            producer_flag_1_u8: 0,
            input_events: vec![replay_core::A729CameraInputEvent {
                node_ordinal: 0,
                node_kind_i32: 0,
                consumed_before_u8: 0,
                consumed_after_u8: 1,
                event_type_i32: 4,
                signed_delta_i32: 5,
                embedded_event_16_bytes: embedded,
            }],
            map_picks: vec![replay_core::A729CameraMapPick {
                call_ordinal: 0,
                mode_u8: 1,
                screen_xy_f32_bits: [320.0f32.to_bits(), 240.0f32.to_bits()],
                world_xyz_f32_bits: [100.0f32.to_bits(), 0, 200.0f32.to_bits()],
            }],
            current_eye_before_f32_bits: [1.0f32.to_bits(); 3],
            target_eye_before_f32_bits: [2.0f32.to_bits(); 3],
            target_look_at_before_f32_bits: [3.0f32.to_bits(); 3],
            current_eye_after_f32_bits: current_eye,
            target_eye_after_f32_bits: target_eye,
            target_look_at_after_f32_bits: target_look,
        };
        let mut carrier = replay_core::AuthenticatedCameraCarrier {
            schema: replay_core::A729_CAMERA_TEMPORAL_SCHEMA.into(),
            executable_sha256: replay_core::A729_CAMERA_EXECUTABLE_SHA256.into(),
            capture_boundary_rva: replay_core::A729_CAMERA_ENVELOPE_RVA,
            camera_update_rva: replay_core::A729_CAMERA_UPDATE_RVA,
            carrier_ordinal: 0,
            carrier_offset: 0x10,
            owner_kind: replay_core::A729CameraOwnerKind::InGame,
            owner_vtable_rva: 0x0295_2510,
            render_view_436_bytes: bytes.clone(),
            canonical_camera_root_436_bytes: bytes,
            previous_current_eye_f32_bits: [0; 3],
            current_eye_f32_bits: current_eye,
            current_look_at_f32_bits: current_look,
            target_eye_f32_bits: target_eye,
            target_look_at_f32_bits: target_look,
            viewport_i32: [0, 0, 640, 480],
            min_height_i32: 1,
            max_height_i32: 100,
            update_produced_u8: 1,
            special_free_state_u8: 0,
            ramp_f32_bits: 1.0f32.to_bits(),
            wrap_branch_u8: 1,
            camera_event_batches: vec![event_batch],
            payload_sha256: String::new(),
        };
        carrier.payload_sha256 = carrier.computed_payload_sha256();
        let binding = map_viewer::C03CameraReplayBinding {
            schema: map_viewer::C03_CAMERA_REPLAY_BINDING_SCHEMA.into(),
            submit_index: 7,
            submit_global_order: 3,
            carrier_payload_sha256: carrier.payload_sha256.clone(),
            event_batch_count: 1,
            raw_input_event_count: 1,
            map_pick_count: 1,
            current_eye_f32_bits: current_eye,
            target_eye_f32_bits: target_eye,
            target_look_at_f32_bits: target_look,
            mode: map_viewer::C03CameraReplayMode::RecordedEnvelopeStateOnly,
            raw_event_reapplication_count: 0,
            envelope_state_selection_count: 1,
        };
        (carrier, binding)
    }

    #[test]
    fn c03_recorded_camera_selects_final_envelope_state_without_reapplying_delta() {
        let (carrier, binding) = c03_recorded_camera_fixture();
        let camera = production_camera_from_recorded_carrier(&carrier, &binding, 640, 480)
            .expect("recorded camera");
        assert_eq!(camera.position, [10.0, 20.0, 30.0]);
        assert_eq!(binding.raw_event_reapplication_count, 0);

        let mut double_apply = binding.clone();
        double_apply.raw_event_reapplication_count = 1;
        assert!(
            production_camera_from_recorded_carrier(&carrier, &double_apply, 640, 480)
                .unwrap_err()
                .to_string()
                .contains("binding identity/policy mismatch")
        );

        let mut changed_raw_event = carrier.clone();
        changed_raw_event.camera_event_batches[0].input_events[0].signed_delta_i32 = 6;
        changed_raw_event.payload_sha256 = changed_raw_event.computed_payload_sha256();
        assert!(
            production_camera_from_recorded_carrier(&changed_raw_event, &binding, 640, 480,)
                .unwrap_err()
                .to_string()
                .contains("binding/state policy mismatch")
        );
    }

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
    fn map_to_screen_3d_round_trips_camdef1_projection() {
        let projection = ProductionMapCameraProjection::from_engine_constants(1408.0 / 512.0)
            .expect("projection");
        let camera = projection
            .camera_for_ground_center(2816.0, 1024.0, 905.0)
            .expect("production camera");
        let screen_w = 1408.0;
        let screen_h = 512.0;
        let map_h = 2048.0;
        let center = project_map_point_3d(&camera, map_h, 2816.0, 1024.0, 0.0, screen_w, screen_h)
            .expect("center projects");
        assert!(
            (center.0 - screen_w * 0.5).abs() < 1.0e-3
                && (center.1 - screen_h * 0.5).abs() < 1.0e-3,
            "ground center projects without half-pixel bias: {center:?}"
        );

        for &(map_x, map_y) in &[
            (2816.0, 1024.0),
            (2890.25, 970.5),
            (2700.75, 1110.25),
            (3000.0, 880.0),
        ] {
            let (sx, sy) =
                project_map_point_3d(&camera, map_h, map_x, map_y, 0.0, screen_w, screen_h)
                    .expect("point projects");
            let (ground_x, ground_z) = camera
                .screen_to_ground_y0(sx as f32, sy as f32, screen_w as f32, screen_h as f32)
                .expect("screen-to-ground")
                .expect("ground hit");
            let expected_z = map_h as f32 - map_y as f32;
            assert!(
                (ground_x - map_x as f32).abs() < 0.02
                    && (ground_z - expected_z).abs() < 0.02,
                "round-trip ({map_x},{map_y}) -> ({sx},{sy}) -> ({ground_x},{ground_z}), expected z {expected_z}"
            );
        }
    }

    #[test]
    fn restore_full_postfx_disabled_is_inert_validation_path() {
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
        let err = restore_full_postfx_from_files(true, None, 5632, 2048, None)
            .expect_err("enabled grade without a resolver must fail closed");
        assert!(
            err.to_string().contains("--game") || err.to_string().contains("BATTLE_PLAN_VANILLA"),
            "error must name the missing game install: {err}"
        );
    }

    fn write_color_cube_tga(path: &Path, rgb: [u8; 3]) {
        let mut tga = vec![0u8; 18];
        tga[2] = 2;
        tga[12..14].copy_from_slice(&1024u16.to_le_bytes());
        tga[14..16].copy_from_slice(&32u16.to_le_bytes());
        tga[16] = 24;
        tga[17] = 0x20;
        for _ in 0..(1024 * 32) {
            tga.push(rgb[2]);
            tga.push(rgb[1]);
            tga.push(rgb[0]);
        }
        std::fs::write(path, tga).expect("write color-cube tga");
    }

    fn write_grade2_fixture_game_dir(tag: &str) -> PathBuf {
        let dir = crate::bundle::tests::unique_temp_dir(tag);
        let gfx = dir.join("gfx");
        let world = gfx.join("world");
        let defines = dir.join("common").join("defines");
        std::fs::create_dir_all(&world).expect("create gfx/world");
        std::fs::create_dir_all(&defines).expect("create common/defines");
        std::fs::write(
            gfx.join("posteffect_volumes.txt"),
            r#"
posteffect_values = {
    name = standard
    lut = "gfx/world/colorcorrection_standard.tga"
    tonemap_middlegrey = 0.55
    BLOOM_WIDTH = 1.5
    BLOOM_SCALE = 0.9
    BRIGHT_THRESHOLD = 0.9
}
posteffect_values = {
    name = default
    inherit = standard
    lut = "gfx/world/colorcorrection_standard.tga"
}
posteffect_values = {
    name = mid_distance
    inherit = standard
    lut = "gfx/world/colorcorrection_midrange.tga"
    tonemap_middlegrey = 0.45
    hdr_min_adjustment = 0.8
    hdr_max_adjustment = 0.6
    BLOOM_WIDTH = 2.5
    BLOOM_SCALE = 1.2
    BRIGHT_THRESHOLD = 0.4
}
posteffect_values = {
    name = max_distance
    inherit = standard
    lut = "gfx/world/colorcorrection_farrange.tga"
}
posteffect_volumes = {
    posteffect_height_volume = {
        name = "Base Highest"
        posteffect_values_day = max_distance
        height = 1600
        fade_distance = 200
    }
    posteffect_height_volume = {
        name = "Base Medium"
        posteffect_values_day = mid_distance
        height = 900
        fade_distance = 200
    }
    posteffect_height_volume = {
        name = "Base"
        posteffect_values_day = default
        height = 0
        fade_distance = 0
    }
}
"#,
        )
        .expect("write posteffect volumes");
        std::fs::write(
            defines.join("00_graphics.lua"),
            r#"
TONE_MAP_MIDDLE_GREY = 0.5,
TONE_MAP_LUMINANCE_WHITE = 1.0,
MIN_HDR_ADJUSTMENT = 0.5,
MAX_HDR_ADJUSTMENT = 1.0,
HDR_ADJUSTMENT_SPEED = 15.0,
BLOOM_WIDTH = 1.5,
BLOOM_SCALE = 0.9,
BRIGHT_THRESHOLD = 0.9,
EMISSIVE_BLOOM_STRENGTH = 1.0,
POSTEFFECT_PER_PROVINCE_MIN_SNOW = 0.1,
POSTEFFECT_PER_PROVINCE_MAX_SNOW = 0.2,
POSTEFFECT_TOTAL_MIN_SNOW = 0.0,
POSTEFFECT_TOTAL_MAX_SNOW = 0.05,
POSTEFFECT_FEATHER_MIN_DISTANCE = 300.0,
POSTEFFECT_FEATHER_MAX_DISTANCE = 2000.0,
POSTEFFECT_FEATHER_AT_MIN = 0.03,
POSTEFFECT_FEATHER_AT_MAX = 0.80,
"#,
        )
        .expect("write graphics defines");
        write_color_cube_tga(&world.join("colorcorrection_standard.tga"), [16, 24, 32]);
        write_color_cube_tga(&world.join("colorcorrection_midrange.tga"), [96, 24, 32]);
        write_color_cube_tga(&world.join("colorcorrection_farrange.tga"), [176, 24, 32]);
        dir
    }

    #[test]
    fn gate_grade2_changed_input_changes_lut_and_same_input_skips_upload() {
        let game_dir = write_grade2_fixture_game_dir("grade2_render_upload");
        let resolver = GameFileResolver::vanilla(game_dir).expect("fixture resolver");
        let high_state = GradingSelectionState {
            camera: [2816.0, 2958.0, 1400.0],
            night: false,
            winter: false,
            region_winter_factor: None,
        };
        let high = file_derived_grade_per_frame(&resolver, high_state, None).expect("high grade");
        let bloom = restore_bloom_law_for_grade(&high).expect("high bloom");
        let mut restore = RestoreFullPostfx {
            enabled: true,
            evidence_found: true,
            constants: RestorePostfxConstants::identity(),
            color_cube_rgba8: restore_identity_color_cube_rgba8(),
            bloom: Some(bloom),
            luminance_bounds: None,
            middle_grey: 1.0,
        };
        let mut lut_uploads = 0u32;
        let first = apply_restore_grade_cpu(&mut restore, &high, bloom);
        if first.color_cube_changed {
            lut_uploads += 1;
        }
        assert!(
            first.color_cube_changed,
            "initial frame must upload the files-derived LUT"
        );
        let high_bytes = restore.color_cube_rgba8.clone();

        let high_again =
            file_derived_grade_per_frame(&resolver, high_state, None).expect("same high grade");
        assert_eq!(
            high_again.color_cube_rgba8, high_bytes,
            "unchanged grade input must compose byte-identical LUT bytes"
        );
        let stable = apply_restore_grade_cpu(&mut restore, &high_again, bloom);
        if stable.color_cube_changed {
            lut_uploads += 1;
        }
        assert_eq!(
            stable,
            RestoreGradeCpuUpdate::default(),
            "unchanged grade input must not request constants/LUT/bloom uploads"
        );

        let faded_state = GradingSelectionState {
            camera: [2816.0, 1700.0, 1400.0],
            night: false,
            winter: false,
            region_winter_factor: None,
        };
        let faded =
            file_derived_grade_per_frame(&resolver, faded_state, None).expect("faded grade");
        assert_ne!(
            faded.color_cube_rgba8, high_bytes,
            "changed camera height inside the file fade window must change LUT bytes"
        );
        let faded_bloom = restore_bloom_law_for_grade(&faded).expect("faded bloom");
        let changed = apply_restore_grade_cpu(&mut restore, &faded, faded_bloom);
        if changed.color_cube_changed {
            lut_uploads += 1;
        }
        assert!(
            changed.color_cube_changed
                && changed.constants_changed
                && changed.bloom_changed
                && changed.luminance_bounds_changed,
            "changed six-scalar grade input must update LUT, constants, bloom, and HDR bounds"
        );
        assert_eq!(
            lut_uploads, 2,
            "only first frame and changed-input frame may upload the LUT"
        );
    }

    #[test]
    fn restore_grade_cache_key_includes_selected_region_snow_rows() {
        let state = GradingSelectionState {
            camera: [2712.5, 300.0, 1700.0],
            night: false,
            winter: false,
            region_winter_factor: Some(0.5),
        };
        let first = RestoreGradeKey::from_state(state, Some([0x11; 32]));
        let changed_snow_same_factor = RestoreGradeKey::from_state(state, Some([0x22; 32]));
        assert_ne!(first, changed_snow_same_factor);
        assert_ne!(first, RestoreGradeKey::from_state(state, None));
    }

    #[test]
    fn snow_mud_env_overrides_are_product_path_blocked() {
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

    #[test]
    fn semantic_snow_mud_without_exact_preupload_packet_fails_closed() {
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("fixtures/positive/fx_snow_mud_accumulator.battleplan");
        let relocated_dir = crate::bundle::tests::unique_temp_dir("portable_snow_mud_relocated");
        let relocated = relocated_dir.join("shared-copy.battleplan");
        std::fs::copy(&source, &relocated).expect("relocate standalone replay file");

        let timeline = replay_core::WorldTimeline::open(
            replay_core::open_recording(&relocated).expect("open relocated replay"),
        )
        .expect("open relocated timeline");
        let hour = *timeline.hours().last().expect("recorded hour");
        let frame = timeline.frame(hour).expect("typed frame");
        let err = recorded_snow_mud_texture(&frame, None)
            .expect_err("semantic levels cannot replace exact producer bytes");
        assert!(
            err.to_string().contains("no production render carrier"),
            "{err}"
        );

        drop(frame);
        drop(timeline);
        portable_test_fs_retry::remove_dir_all(relocated_dir).expect("clean relocated replay fixture");
    }

    #[test]
    fn production_snow_mud_rejects_missing_first_hour_instead_of_captured_fallback() {
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("fixtures/positive/fx_owner_controller_flip.battleplan");
        let timeline = replay_core::WorldTimeline::open(
            replay_core::open_recording(&source).expect("open snow/mud-negative replay"),
        )
        .expect("open snow/mud-negative timeline");
        let hour = *timeline.hours().first().expect("recorded hour");
        let frame = timeline.frame(hour).expect("typed frame");
        assert_ne!(frame.snow_mud().status, ViewStatus::Present);

        let err = required_recorded_snow_mud_texture(&frame, None)
            .expect_err("missing first-hour SnowMudData must not retain PAN2's texture");
        assert!(
            err.to_string()
                .contains("displayed hour has no production render carrier"),
            "{err}"
        );
    }

    #[test]
    fn occupation_stripes_reject_missing_first_submit_secondary_texture() {
        let err = require_secondary_political_texture(None)
            .expect_err("missing first submit must not retain PAN2's secondary texture");
        assert!(err.to_string().contains("DATA_BLOCKED"), "{err}");
        assert!(
            err.to_string()
                .contains("refusing the captured bundle texture"),
            "{err}"
        );

        let submit = empty_submit(1);
        let err = require_secondary_political_texture(Some(&submit))
            .expect_err("empty first submit must not retain PAN2's secondary texture");
        assert!(
            err.to_string()
                .contains("no exact ProvinceSecondaryColorMap packet"),
            "{err}"
        );

        let mut submit = empty_submit(1);
        submit
            .political_textures
            .push(replay_core::PoliticalTexturePacket {
                kind: PoliticalTextureKind::Secondary,
                target_binding_identity: "ProvinceSecondaryColorMap".into(),
                upload_order: 7,
                width: 1,
                height: 1,
                row_pitch: 4,
                format: PoliticalTextureFormat::B8g8r8a8UnormSrgb,
                bytes: vec![1, 2, 3, 4],
                sha256: "9f64a747e1b97f131fabb6b447296c9b6f0201e79fb3c5356e6c77e89b6a806a".into(),
                occupation_observation: None,
            });
        let err = require_secondary_political_texture(Some(&submit))
            .expect_err("an unconsumed packet must not authenticate occupation stripes");
        assert!(
            err.to_string()
                .contains("no exact D80 selected-submit shader-resource binding"),
            "{err}"
        );
    }

    fn empty_submit(global_order: u32) -> RenderSubmit {
        RenderSubmit {
            global_order,
            camera_carrier_ordinal: 0,
            camera_carrier: None,
            ordered_static_map_programs: Vec::new(),
            authenticated_texture_snapshots: Vec::new(),
            s1_shadow: None,
            particles: Vec::new(),
            particle_packet_set_sha256: "5".repeat(64),
            entities: Vec::new(),
            rail_construction_arrows: Vec::new(),
            ordered_render_operations: Vec::new(),
            weather_membership: Vec::new(),
            political_textures: Vec::new(),
        }
    }
    #[test]
    fn production_render_uses_explicit_display_submit_without_first_last_guess() {
        use replay_core::{ParticleRuntimeMode, RenderAuthority, RenderCheckpoint};

        let frame = ProductionRenderFrame {
            schema: replay_core::PRODUCTION_RENDER_SCHEMA.to_string(),
            authority: RenderAuthority {
                process_id: 7,
                process_start_nonce: 8,
                observer_installed_qpc: 9,
                d3d_device_created_qpc: 10,
                first_creation_sequence: 1,
                hook_epoch_nonce: 11,
                hook_set_receipt_sha256: "a".repeat(64),
                capture_receipt_sha256: "b".repeat(64),
                launch_epoch_sha256: replay_core::compute_render_launch_epoch_sha256(
                    7,
                    8,
                    9,
                    10,
                    1,
                    11,
                    &"a".repeat(64),
                    &"b".repeat(64),
                ),
                effective_vfs_manifest_sha256: "2".repeat(64),
                effect_catalog_sha256: "3".repeat(64),
                entity_animation_catalog_sha256: "4".repeat(64),
                recorded_input_variant: None,
                recorded_input_receipt_sha256: None,
                recorded_input_observation: None,
                particle_runtime_mode: ParticleRuntimeMode::SubmittedPacketReplay,
            },
            update_index: 16_959_360,
            first_sequence: 10,
            last_sequence: 20,
            presentation_qpc_frequency_hz: 10_000_000,
            display_submit_index: Some("12".to_string()),
            selected_present: replay_core::SelectedPresentReceipt::sealed(
                2,
                "c".repeat(64),
                12,
                1,
                60_759_359,
                200,
                300,
            ),
            snow_mud_texture: None,
            submits: vec![empty_submit(2)],
            m02_combat: None,
            m03_arrows: None,
            rail_supply_arrows: None,
            o04_cities: None,
            o05_buildings: None,
            authenticated_ui_pass: None,
            checkpoint: empty_render_checkpoint(20, 16_959_360),
        };
        assert_eq!(
            // kills: carrier.render_submit.submit_index — display selection resolves through
            // the receipt-derived identity (selected_present.selected_candidate_id == 12).
            production_display_submit(&frame)
                .expect("select display submit")
                .is_some(),
            true
        );
        assert_eq!(frame.retained_submit_identity(), Some(12));
        assert_eq!(frame.selected_presentation_qpc_ticks(), Some(200));

        let mut ambiguous = frame;
        ambiguous.display_submit_index = None;
        assert!(production_display_submit(&ambiguous)
            .expect_err("nonempty submit set without a selected display submit must fail")
            .to_string()
            .contains("display_submit_index"));
    }

    fn empty_render_checkpoint(last_sequence: u64, update_index: u64) -> RenderCheckpoint {
        RenderCheckpoint {
            last_sequence,
            last_update_index: update_index,
            last_submit_index: Some("12".to_string()),
            next_stable_ids: BTreeMap::new(),
            active_effect_lifecycle: Vec::new(),
            active_light_lifecycle: Vec::new(),
            weather_state_and_ordered_membership: Vec::new(),
            active_effect_set_sha256: "6".repeat(64),
            ordered_particle_packets_sha256: "7".repeat(64),
            ordered_entity_submissions_sha256: "8".repeat(64),
            ordered_light_submissions_sha256: "a".repeat(64),
            weather_state_sha256:
                "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945".into(),
            weather_membership_sha256:
                "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945".into(),
            weather_lifecycle_sha256:
                "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945".into(),
            weather_visibility_sha256:
                "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945".into(),
            ordered_weather_packets_sha256:
                "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945".into(),
            weather_composition_sha256:
                "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945".into(),
        }
    }
}
