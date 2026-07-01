use super::*;
use minhook::MinHook;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{c_void as StdVoid, CStr, CString, OsStr};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::mem;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr;
use std::slice;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use winapi::ctypes::c_void as WinVoid;
use winapi::shared::guiddef::REFIID;
use winapi::shared::minwindef::UINT;
use winapi::shared::winerror::{FAILED, SUCCEEDED};
use winapi::um::d3d11::{
    D3D11CalcSubresource, D3D11CreateDevice, ID3D11BlendState, ID3D11Buffer, ID3D11ClassInstance,
    ID3D11ClassLinkage, ID3D11DepthStencilState, ID3D11DepthStencilView, ID3D11Device,
    ID3D11DeviceContext, ID3D11InputLayout, ID3D11PixelShader, ID3D11RasterizerState,
    ID3D11RenderTargetView, ID3D11Resource, ID3D11ShaderResourceView, ID3D11Texture2D,
    ID3D11VertexShader, IID_ID3D11Texture2D, D3D11_BIND_CONSTANT_BUFFER, D3D11_BLEND_DESC,
    D3D11_BUFFER_DESC, D3D11_DEPTH_STENCILOP_DESC, D3D11_DEPTH_STENCIL_DESC,
    D3D11_DEPTH_STENCIL_VIEW_DESC, D3D11_DSV_DIMENSION_TEXTURE2D,
    D3D11_DSV_DIMENSION_TEXTURE2DARRAY, D3D11_DSV_DIMENSION_TEXTURE2DMS,
    D3D11_DSV_DIMENSION_TEXTURE2DMSARRAY, D3D11_INPUT_ELEMENT_DESC, D3D11_MAPPED_SUBRESOURCE,
    D3D11_RASTERIZER_DESC, D3D11_RECT, D3D11_RENDER_TARGET_BLEND_DESC,
    D3D11_RENDER_TARGET_VIEW_DESC, D3D11_RTV_DIMENSION_TEXTURE2D,
    D3D11_RTV_DIMENSION_TEXTURE2DARRAY, D3D11_RTV_DIMENSION_TEXTURE2DMS,
    D3D11_RTV_DIMENSION_TEXTURE2DMSARRAY, D3D11_SAMPLER_DESC, D3D11_SDK_VERSION,
    D3D11_SHADER_RESOURCE_VIEW_DESC, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING, D3D11_VIEWPORT,
};
use winapi::um::d3d11shader::{
    ID3D11ShaderReflection, IID_ID3D11ShaderReflection, D3D11_SHADER_DESC,
    D3D11_SHADER_INPUT_BIND_DESC, D3D11_SIGNATURE_PARAMETER_DESC,
};
use winapi::um::d3dcommon::{
    D3D11_SRV_DIMENSION_TEXTURE2D, D3D11_SRV_DIMENSION_TEXTURE2DARRAY,
    D3D11_SRV_DIMENSION_TEXTURECUBE, D3D11_SRV_DIMENSION_TEXTURECUBEARRAY, D3D_DRIVER_TYPE_NULL,
    D3D_FEATURE_LEVEL_11_0, D3D_SIT_CBUFFER, D3D_SIT_TEXTURE,
};
use winapi::um::unknwnbase::IUnknown;
use windows_sys::Win32::Foundation::{FreeLibrary, HMODULE};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

const CAPTURE_DIR_NAME: &str = "terrain_capture";
const CAPTURE_EVENTS_FILE: &str = "terrain_capture_events.jsonl";
const TERRAIN_RGBA_FILE: &str = "terrain_id_map.rgba";
const TERRAIN_META_FILE: &str = "terrain_id_map_capture.json";
const SNOW_MUD_RGBA_FILE: &str = "snow_mud_data.rgba";
const SNOW_MUD_META_FILE: &str = "snow_mud_data_capture.json";
const PROVINCE_SECONDARY_COLOR_MAP_SYMBOL: &str = "ProvinceSecondaryColorMap";
const PROVINCE_SECONDARY_COLOR_MAP_RGBA_FILE: &str = "province_secondary_color_map.rgba";
const PROVINCE_SECONDARY_COLOR_MAP_META_FILE: &str = "province_secondary_color_map_capture.json";
const GRADIENT_BORDER_CHANNEL1_SYMBOL: &str = "GradientBorderChannel1";
const GRADIENT_BORDER_CHANNEL1_RGBA_FILE: &str = "gradient_border_channel1.rgba";
const GRADIENT_BORDER_CHANNEL1_META_FILE: &str = "gradient_border_channel1_capture.json";
const GRADIENT_BORDER_CHANNEL2_SYMBOL: &str = "GradientBorderChannel2";
const GRADIENT_BORDER_CHANNEL2_RGBA_FILE: &str = "gradient_border_channel2.rgba";
const GRADIENT_BORDER_CHANNEL2_META_FILE: &str = "gradient_border_channel2_capture.json";
const LIGHT_INDEX_MAP_SYMBOL: &str = "LightIndexMap";
const LIGHT_INDEX_MAP_RGBA_FILE: &str = "light_index_map.rgba";
const LIGHT_INDEX_MAP_META_FILE: &str = "light_index_map_capture.json";
const LIGHT_DATA_MAP_SYMBOL: &str = "LightDataMap";
const LIGHT_DATA_MAP_RAW_FILE: &str = "light_data_map.r32g32b32a32_float";
const LIGHT_DATA_MAP_META_FILE: &str = "light_data_map_capture.json";
const TERRAIN_SCENE_CB0_SYMBOL: &str = "TerrainSceneCB0";
const TERRAIN_SCENE_CB0_FILE: &str = "terrain_scene_cb0.bin";
const TERRAIN_SCENE_CB0_META_FILE: &str = "terrain_scene_cb0_capture.json";
const TERRAIN_VERTEX_BUFFER_SYMBOL: &str = "TerrainVertexBuffer";
const TERRAIN_VERTEX_BUFFER_SAMPLE_FILE: &str = "terrain_vertex_buffer_sample.bin";
const TERRAIN_VERTEX_BUFFER_META_FILE: &str = "terrain_vertex_buffer_capture.json";
const TERRAIN_INPUT_LAYOUT_SYMBOL: &str = "TerrainInputLayout";
const TERRAIN_INPUT_LAYOUT_META_FILE: &str = "terrain_input_layout_capture.json";
const MAP_FRAME_BUNDLE_SYMBOL: &str = "MapFrameBundle";
const MAP_FRAME_BUNDLE_FILE: &str = "map_frame_bundle.json";
const WATER_MAP_FRAME_BUNDLE_FILE: &str = "map_frame_water_pass_bundle.json";
const RIVERS_MAP_FRAME_BUNDLE_FILE: &str = "map_frame_rivers_pass_bundle.json";
const BORDERS_MAP_FRAME_BUNDLE_FILE: &str = "map_frame_borders_pass_bundle.json";
const TREES_MAP_FRAME_BUNDLE_FILE: &str = "map_frame_trees_pass_bundle.json";
const MAP_FRAME_BUNDLE_SCHEMA: &str = "battle-plan-map-frame-bundle-v1";
const NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT: &str = "d3d11_mapped_subresources";
const MAP_FRAME_PRE_DRAW_RENDER_TARGET_FILE_PREFIX: &str = "map_frame_pre_draw_render_target";
const MAP_FRAME_PRE_DRAW_DEPTH_STENCIL_FILE_PREFIX: &str = "map_frame_pre_draw_depth_stencil";
const MAP_FRAME_POST_PASS_RENDER_TARGET_FILE_PREFIX: &str = "map_frame_post_pass_render_target";
const MAP_CAPTURE_SESSION_FILE: &str = "map_capture_session.json";
const MAP_CAPTURE_SESSION_SCHEMA: &str = "battle-plan-map-capture-session-v1";
const TERRAIN_CAPTURE_MOMENT: &str = "terrain_active_pass";
const WATER_CAPTURE_MOMENT: &str = "water_pass";
const RIVERS_CAPTURE_MOMENT: &str = "rivers_pass";
const BORDERS_CAPTURE_MOMENT: &str = "borders_pass";
const TREES_CAPTURE_MOMENT: &str = "trees_pass";
const TERRAIN_MAP_FRAME_FILE_PREFIX: &str = "map_frame_draw";
const WATER_MAP_FRAME_FILE_PREFIX: &str = "water_frame_draw";
const RIVERS_MAP_FRAME_FILE_PREFIX: &str = "rivers_frame_draw";
const BORDERS_MAP_FRAME_FILE_PREFIX: &str = "borders_frame_draw";
const TREES_MAP_FRAME_FILE_PREFIX: &str = "trees_frame_draw";
const INPUT_SEMANTIC_NAME_MAX_BYTES: usize = 64;
const MAP_FRAME_BINDING_SLOTS: u32 = 16;
const MAP_FRAME_MIN_TERRAIN_DRAWS: usize = 2;
const D3D11_COMMONSHADER_INPUT_RESOURCE_SLOT_COUNT: u32 = 128;
const MAP_CAPTURE_REQUIRED_PAYLOAD: [&str; 14] = [
    "shader_bytecode_reflection",
    "input_layouts",
    "vertex_buffers_all_slots",
    "index_buffers",
    "constant_buffers_all_stages",
    "shader_resource_views_textures",
    "sampler_states",
    "draw_args",
    "resource_descs",
    "output_merger_clear_provenance",
    "pre_draw_output_merger_payload",
    "post_pass_render_target_payload",
    "hashes",
    "provenance",
];
const DEFERRED_MAP_CAPTURE_MOMENTS: [&str; 6] = [
    "water_pass",
    "rivers_pass",
    "borders_pass",
    "trees_pass",
    "cities_pass",
    "overlays_pass",
];
const BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE: &str = "common/defines/00_graphics.lua";
const VANILLA_SHADER_SOURCES: [&str; 7] = [
    "gfx/FX/pdxmap.shader",
    "gfx/FX/pdxmap.fxh",
    "gfx/FX/constants.fxh",
    "gfx/FX/standardfuncsgfx.fxh",
    "gfx/FX/tiled_pointlights.fxh",
    "gfx/FX/shadow.fxh",
    "gfx/FX/fow.fxh",
];
const VANILLA_MAP_FRAME_SOURCES: [&str; 8] = [
    "gfx/FX/pdxmap.shader",
    "gfx/FX/pdxmap.fxh",
    "gfx/FX/constants.fxh",
    "gfx/FX/standardfuncsgfx.fxh",
    "gfx/FX/tiled_pointlights.fxh",
    "gfx/FX/shadow.fxh",
    "gfx/FX/fow.fxh",
    BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE,
];
const WATER_VANILLA_SHADER_SOURCES: [&str; 7] = [
    "gfx/FX/pdxwater.shader",
    "gfx/FX/pdxmap.fxh",
    "gfx/FX/constants.fxh",
    "gfx/FX/standardfuncsgfx.fxh",
    "gfx/FX/tiled_pointlights.fxh",
    "gfx/FX/shadow.fxh",
    "gfx/FX/fow.fxh",
];
const WATER_VANILLA_MAP_FRAME_SOURCES: [&str; 8] = [
    "gfx/FX/pdxwater.shader",
    "gfx/FX/pdxmap.fxh",
    "gfx/FX/constants.fxh",
    "gfx/FX/standardfuncsgfx.fxh",
    "gfx/FX/tiled_pointlights.fxh",
    "gfx/FX/shadow.fxh",
    "gfx/FX/fow.fxh",
    BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE,
];
const RIVER_VANILLA_SHADER_SOURCES: [&str; 6] = [
    "gfx/FX/river.shader",
    "gfx/FX/constants.fxh",
    "gfx/FX/standardfuncsgfx.fxh",
    "gfx/FX/tiled_pointlights.fxh",
    "gfx/FX/shadow.fxh",
    "gfx/FX/fow.fxh",
];
const RIVER_VANILLA_MAP_FRAME_SOURCES: [&str; 7] = [
    "gfx/FX/river.shader",
    "gfx/FX/constants.fxh",
    "gfx/FX/standardfuncsgfx.fxh",
    "gfx/FX/tiled_pointlights.fxh",
    "gfx/FX/shadow.fxh",
    "gfx/FX/fow.fxh",
    BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE,
];
const BORDER_VANILLA_SHADER_SOURCES: [&str; 6] = [
    "gfx/FX/border.shader",
    "gfx/FX/pdxmap.fxh",
    "gfx/FX/constants.fxh",
    "gfx/FX/standardfuncsgfx.fxh",
    "gfx/FX/shadow.fxh",
    "gfx/FX/fow.fxh",
];
const BORDER_VANILLA_MAP_FRAME_SOURCES: [&str; 7] = [
    "gfx/FX/border.shader",
    "gfx/FX/pdxmap.fxh",
    "gfx/FX/constants.fxh",
    "gfx/FX/standardfuncsgfx.fxh",
    "gfx/FX/shadow.fxh",
    "gfx/FX/fow.fxh",
    BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE,
];
const TREE_VANILLA_SHADER_SOURCES: [&str; 6] = [
    "gfx/FX/tree.shader",
    "gfx/FX/constants.fxh",
    "gfx/FX/standardfuncsgfx.fxh",
    "gfx/FX/tiled_pointlights.fxh",
    "gfx/FX/shadow.fxh",
    "gfx/FX/fow.fxh",
];
const TREE_VANILLA_MAP_FRAME_SOURCES: [&str; 7] = [
    "gfx/FX/tree.shader",
    "gfx/FX/constants.fxh",
    "gfx/FX/standardfuncsgfx.fxh",
    "gfx/FX/tiled_pointlights.fxh",
    "gfx/FX/shadow.fxh",
    "gfx/FX/fow.fxh",
    BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE,
];
pub(crate) const CONTROL_FILE: &str = "terrain_capture_control.json";

#[derive(Clone, Copy, Debug)]
struct CaptureTarget {
    symbol: &'static str,
    rgba_file: &'static str,
    meta_file: &'static str,
    output_format: &'static str,
    readback_format: CaptureReadbackFormat,
    validate_control: bool,
}

#[derive(Clone, Copy, Debug)]
enum CaptureReadbackFormat {
    Rgba8,
    R32G32B32A32Float,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MapFrameCaptureProfile {
    capture_moment: &'static str,
    bundle_file: &'static str,
    file_prefix: &'static str,
    pixel_shader_file_prefix: &'static str,
    vertex_shader_file_prefix: &'static str,
    shader_sources: &'static [&'static str],
    vanilla_sources: &'static [&'static str],
    requires_terrain_runtime_files: bool,
    requires_shader_cache_constants: bool,
    min_draws_before_clear_finalize: usize,
}

const TERRAIN_MAP_FRAME_CAPTURE_PROFILE: MapFrameCaptureProfile = MapFrameCaptureProfile {
    capture_moment: TERRAIN_CAPTURE_MOMENT,
    bundle_file: MAP_FRAME_BUNDLE_FILE,
    file_prefix: TERRAIN_MAP_FRAME_FILE_PREFIX,
    pixel_shader_file_prefix: "pdxmap_pixel_shader",
    vertex_shader_file_prefix: "pdxmap_vertex_shader",
    shader_sources: &VANILLA_SHADER_SOURCES,
    vanilla_sources: &VANILLA_MAP_FRAME_SOURCES,
    requires_terrain_runtime_files: true,
    requires_shader_cache_constants: true,
    min_draws_before_clear_finalize: MAP_FRAME_MIN_TERRAIN_DRAWS,
};

const WATER_MAP_FRAME_CAPTURE_PROFILE: MapFrameCaptureProfile = MapFrameCaptureProfile {
    capture_moment: WATER_CAPTURE_MOMENT,
    bundle_file: WATER_MAP_FRAME_BUNDLE_FILE,
    file_prefix: WATER_MAP_FRAME_FILE_PREFIX,
    pixel_shader_file_prefix: "pdxwater_pixel_shader",
    vertex_shader_file_prefix: "map_vertex_shader",
    shader_sources: &WATER_VANILLA_SHADER_SOURCES,
    vanilla_sources: &WATER_VANILLA_MAP_FRAME_SOURCES,
    requires_terrain_runtime_files: false,
    requires_shader_cache_constants: false,
    min_draws_before_clear_finalize: 1,
};

const RIVERS_MAP_FRAME_CAPTURE_PROFILE: MapFrameCaptureProfile = MapFrameCaptureProfile {
    capture_moment: RIVERS_CAPTURE_MOMENT,
    bundle_file: RIVERS_MAP_FRAME_BUNDLE_FILE,
    file_prefix: RIVERS_MAP_FRAME_FILE_PREFIX,
    pixel_shader_file_prefix: "river_pixel_shader",
    vertex_shader_file_prefix: "map_vertex_shader",
    shader_sources: &RIVER_VANILLA_SHADER_SOURCES,
    vanilla_sources: &RIVER_VANILLA_MAP_FRAME_SOURCES,
    requires_terrain_runtime_files: false,
    requires_shader_cache_constants: false,
    min_draws_before_clear_finalize: 1,
};

const BORDERS_MAP_FRAME_CAPTURE_PROFILE: MapFrameCaptureProfile = MapFrameCaptureProfile {
    capture_moment: BORDERS_CAPTURE_MOMENT,
    bundle_file: BORDERS_MAP_FRAME_BUNDLE_FILE,
    file_prefix: BORDERS_MAP_FRAME_FILE_PREFIX,
    pixel_shader_file_prefix: "border_pixel_shader",
    vertex_shader_file_prefix: "map_vertex_shader",
    shader_sources: &BORDER_VANILLA_SHADER_SOURCES,
    vanilla_sources: &BORDER_VANILLA_MAP_FRAME_SOURCES,
    requires_terrain_runtime_files: false,
    requires_shader_cache_constants: false,
    min_draws_before_clear_finalize: 1,
};

const TREES_MAP_FRAME_CAPTURE_PROFILE: MapFrameCaptureProfile = MapFrameCaptureProfile {
    capture_moment: TREES_CAPTURE_MOMENT,
    bundle_file: TREES_MAP_FRAME_BUNDLE_FILE,
    file_prefix: TREES_MAP_FRAME_FILE_PREFIX,
    pixel_shader_file_prefix: "tree_pixel_shader",
    vertex_shader_file_prefix: "map_vertex_shader",
    shader_sources: &TREE_VANILLA_SHADER_SOURCES,
    vanilla_sources: &TREE_VANILLA_MAP_FRAME_SOURCES,
    requires_terrain_runtime_files: false,
    requires_shader_cache_constants: false,
    min_draws_before_clear_finalize: 1,
};

const CAPTURE_TARGETS: [CaptureTarget; 7] = [
    CaptureTarget {
        symbol: TERRAIN_ID_RESOURCE_SYMBOL,
        rgba_file: TERRAIN_RGBA_FILE,
        meta_file: TERRAIN_META_FILE,
        output_format: "rgba8_unorm_engine_packed_terrain_ids",
        readback_format: CaptureReadbackFormat::Rgba8,
        validate_control: true,
    },
    CaptureTarget {
        symbol: SNOW_MUD_DATA_RESOURCE_SYMBOL,
        rgba_file: SNOW_MUD_RGBA_FILE,
        meta_file: SNOW_MUD_META_FILE,
        output_format: "rgba8_unorm_runtime_captured",
        readback_format: CaptureReadbackFormat::Rgba8,
        validate_control: false,
    },
    CaptureTarget {
        symbol: PROVINCE_SECONDARY_COLOR_MAP_SYMBOL,
        rgba_file: PROVINCE_SECONDARY_COLOR_MAP_RGBA_FILE,
        meta_file: PROVINCE_SECONDARY_COLOR_MAP_META_FILE,
        output_format: "rgba8_unorm_runtime_captured",
        readback_format: CaptureReadbackFormat::Rgba8,
        validate_control: false,
    },
    CaptureTarget {
        symbol: GRADIENT_BORDER_CHANNEL1_SYMBOL,
        rgba_file: GRADIENT_BORDER_CHANNEL1_RGBA_FILE,
        meta_file: GRADIENT_BORDER_CHANNEL1_META_FILE,
        output_format: "rgba8_unorm_runtime_captured",
        readback_format: CaptureReadbackFormat::Rgba8,
        validate_control: false,
    },
    CaptureTarget {
        symbol: GRADIENT_BORDER_CHANNEL2_SYMBOL,
        rgba_file: GRADIENT_BORDER_CHANNEL2_RGBA_FILE,
        meta_file: GRADIENT_BORDER_CHANNEL2_META_FILE,
        output_format: "rgba8_unorm_runtime_captured",
        readback_format: CaptureReadbackFormat::Rgba8,
        validate_control: false,
    },
    CaptureTarget {
        symbol: LIGHT_INDEX_MAP_SYMBOL,
        rgba_file: LIGHT_INDEX_MAP_RGBA_FILE,
        meta_file: LIGHT_INDEX_MAP_META_FILE,
        output_format: "rgba8_unorm_runtime_captured",
        readback_format: CaptureReadbackFormat::Rgba8,
        validate_control: false,
    },
    CaptureTarget {
        symbol: LIGHT_DATA_MAP_SYMBOL,
        rgba_file: LIGHT_DATA_MAP_RAW_FILE,
        meta_file: LIGHT_DATA_MAP_META_FILE,
        output_format: "r32g32b32a32_float_runtime_captured",
        readback_format: CaptureReadbackFormat::R32G32B32A32Float,
        validate_control: false,
    },
];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct TerrainCaptureControl {
    pub schema: String,
    #[serde(default)]
    pub capture_moment: Option<String>,
    #[serde(default)]
    pub capture_moments: Vec<String>,
    #[serde(default)]
    pub expected_width: u32,
    #[serde(default)]
    pub expected_height: u32,
    #[serde(default)]
    pub allowed_slots: Vec<u32>,
    #[serde(default)]
    pub paired_resource_slots: Vec<PairedResourceSlotControl>,
    #[serde(default)]
    pub shader_cache_signatures: Vec<TerrainShaderCacheSignatureControl>,
    pub provenance: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PairedResourceSlotControl {
    pub terrain_id_srv_slot: u32,
    pub symbol: String,
    pub shader_resource_slot: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct TerrainShaderCacheSignatureControl {
    pub file: String,
    pub map_size_x: u32,
    pub map_size_y: u32,
    pub map_pow2_x_milli: u32,
    pub map_pow2_y_milli: u32,
    pub fow_pow2_x_milli: u32,
    pub fow_pow2_y_milli: u32,
    pub terrain_id_srv_slot: u32,
}

type CreatePixelShaderFn = unsafe extern "system" fn(
    *mut ID3D11Device,
    *const WinVoid,
    usize,
    *mut ID3D11ClassLinkage,
    *mut *mut ID3D11PixelShader,
) -> i32;

type CreateVertexShaderFn = unsafe extern "system" fn(
    *mut ID3D11Device,
    *const WinVoid,
    usize,
    *mut ID3D11ClassLinkage,
    *mut *mut ID3D11VertexShader,
) -> i32;

type CreateInputLayoutFn = unsafe extern "system" fn(
    *mut ID3D11Device,
    *const D3D11_INPUT_ELEMENT_DESC,
    UINT,
    *const WinVoid,
    usize,
    *mut *mut ID3D11InputLayout,
) -> i32;

type PsSetShaderFn = unsafe extern "system" fn(
    *mut ID3D11DeviceContext,
    *mut ID3D11PixelShader,
    *const *mut ID3D11ClassInstance,
    UINT,
);

type VsSetShaderFn = unsafe extern "system" fn(
    *mut ID3D11DeviceContext,
    *mut ID3D11VertexShader,
    *const *mut ID3D11ClassInstance,
    UINT,
);

type IaSetInputLayoutFn =
    unsafe extern "system" fn(*mut ID3D11DeviceContext, *mut ID3D11InputLayout);

type PsSetShaderResourcesFn = unsafe extern "system" fn(
    *mut ID3D11DeviceContext,
    UINT,
    UINT,
    *const *mut ID3D11ShaderResourceView,
);

type DrawIndexedFn = unsafe extern "system" fn(*mut ID3D11DeviceContext, UINT, UINT, i32);

type DrawFn = unsafe extern "system" fn(*mut ID3D11DeviceContext, UINT, UINT);

type DrawIndexedInstancedFn =
    unsafe extern "system" fn(*mut ID3D11DeviceContext, UINT, UINT, UINT, i32, UINT);

type DrawInstancedFn = unsafe extern "system" fn(*mut ID3D11DeviceContext, UINT, UINT, UINT, UINT);

type ClearRenderTargetViewFn =
    unsafe extern "system" fn(*mut ID3D11DeviceContext, *mut ID3D11RenderTargetView, *const f32);

type ClearDepthStencilViewFn =
    unsafe extern "system" fn(*mut ID3D11DeviceContext, *mut ID3D11DepthStencilView, UINT, f32, u8);

type D3DReflectFn =
    unsafe extern "system" fn(*const WinVoid, usize, REFIID, *mut *mut WinVoid) -> i32;

#[derive(Clone, Copy, Debug)]
struct HookTargets {
    create_input_layout: usize,
    create_vertex_shader: usize,
    create_pixel_shader: usize,
    ia_set_input_layout: usize,
    vs_set_shader: usize,
    ps_set_shader: usize,
    ps_set_shader_resources: usize,
    draw_indexed: usize,
    draw: usize,
    draw_indexed_instanced: usize,
    draw_instanced: usize,
    clear_render_target_view: usize,
    clear_depth_stencil_view: usize,
}

#[derive(Debug)]
struct CaptureRuntime {
    state: TerrainCaptureState,
    output_dir: PathBuf,
    control: Option<TerrainCaptureControl>,
    capture_moment_sequence: Vec<String>,
    active_capture_index: usize,
    captured_symbols: BTreeSet<String>,
    completed_map_frame_moments: BTreeSet<String>,
    map_frame_draws: Vec<MapFrameDrawFile>,
    map_frame_blockers: Vec<String>,
    map_frame_rejections: BTreeSet<String>,
    latest_post_draw_output_merger: Option<MapFrameOutputMergerStateFile>,
    latest_post_draw_output_blockers: Vec<String>,
    om_render_target_clears: BTreeMap<usize, MapFrameRenderTargetClearFile>,
    om_depth_stencil_clears: BTreeMap<usize, MapFrameDepthStencilClearFile>,
    om_clear_sequence: u64,
    terrain_id_srv_slot_observed: Option<u32>,
    last_error: Option<String>,
}

static RUNTIME: OnceLock<Mutex<CaptureRuntime>> = OnceLock::new();
static HOOK_TARGETS: OnceLock<HookTargets> = OnceLock::new();
static HOOKS_INSTALLED: AtomicBool = AtomicBool::new(false);
static CAPTURED: AtomicBool = AtomicBool::new(false);
static CAPTURE_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static VERTEX_CAPTURE_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static VERTEX_CAPTURE_REJECTION_RECORDED: AtomicBool = AtomicBool::new(false);
static INPUT_LAYOUT_CAPTURE_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static INPUT_LAYOUT_CAPTURE_REJECTION_RECORDED: AtomicBool = AtomicBool::new(false);
static INPUT_LAYOUT_COPY_REJECTION_RECORDED: AtomicBool = AtomicBool::new(false);
static MAP_FRAME_CAPTURE_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

static mut ORIG_CREATE_INPUT_LAYOUT: Option<CreateInputLayoutFn> = None;
static mut ORIG_CREATE_VERTEX_SHADER: Option<CreateVertexShaderFn> = None;
static mut ORIG_CREATE_PIXEL_SHADER: Option<CreatePixelShaderFn> = None;
static mut ORIG_IA_SET_INPUT_LAYOUT: Option<IaSetInputLayoutFn> = None;
static mut ORIG_VS_SET_SHADER: Option<VsSetShaderFn> = None;
static mut ORIG_PS_SET_SHADER: Option<PsSetShaderFn> = None;
static mut ORIG_PS_SET_SHADER_RESOURCES: Option<PsSetShaderResourcesFn> = None;
static mut ORIG_DRAW_INDEXED: Option<DrawIndexedFn> = None;
static mut ORIG_DRAW: Option<DrawFn> = None;
static mut ORIG_DRAW_INDEXED_INSTANCED: Option<DrawIndexedInstancedFn> = None;
static mut ORIG_DRAW_INSTANCED: Option<DrawInstancedFn> = None;
static mut ORIG_CLEAR_RENDER_TARGET_VIEW: Option<ClearRenderTargetViewFn> = None;
static mut ORIG_CLEAR_DEPTH_STENCIL_VIEW: Option<ClearDepthStencilViewFn> = None;

pub(crate) fn reflect_shader_resource_slot(bytecode: &[u8], resource: &str) -> Option<u32> {
    unsafe { reflect_shader_resource_slot_inner(bytecode, resource) }
}

pub(crate) fn reflect_shader_texture_resource_slots(bytecode: &[u8]) -> BTreeMap<String, u32> {
    unsafe { reflect_shader_texture_resource_slots_inner(bytecode).unwrap_or_default() }
}

pub(crate) fn reflect_shader_constant_buffer_slots(bytecode: &[u8]) -> BTreeMap<String, u32> {
    unsafe { reflect_shader_constant_buffer_slots_inner(bytecode).unwrap_or_default() }
}

pub(crate) fn reflect_terrain_vertex_shader_metadata(
    bytecode: &[u8],
) -> Option<TerrainVertexShaderMetadata> {
    unsafe { reflect_terrain_vertex_shader_metadata_inner(bytecode) }
}

pub(crate) fn run_one_shot_capture(module_dir: &Path, timeout_ms: u32) {
    let output_dir = module_dir.join(CAPTURE_DIR_NAME);
    let _ = fs::create_dir_all(&output_dir);
    let control = read_control(module_dir);
    let capture_moment_sequence = capture_moment_sequence_from_control(control.as_ref());
    let _ = RUNTIME.set(Mutex::new(CaptureRuntime {
        state: TerrainCaptureState::default(),
        output_dir: output_dir.clone(),
        control: control.clone(),
        capture_moment_sequence,
        active_capture_index: 0,
        captured_symbols: BTreeSet::new(),
        completed_map_frame_moments: BTreeSet::new(),
        map_frame_draws: Vec::new(),
        map_frame_blockers: Vec::new(),
        map_frame_rejections: BTreeSet::new(),
        latest_post_draw_output_merger: None,
        latest_post_draw_output_blockers: Vec::new(),
        om_render_target_clears: BTreeMap::new(),
        om_depth_stencil_clears: BTreeMap::new(),
        om_clear_sequence: 0,
        terrain_id_srv_slot_observed: None,
        last_error: None,
    }));
    write_event(
        &output_dir,
        &format!(
            "{{\"type\":\"terrain_capture_start\",\"timeout_ms\":{},\"requires\":\"active_pdxmap_shader_declared_TerrainIDMap_and_SnowMudData_slots_plus_terrain_vertex_buffer_and_input_layout\"}}",
            timeout_ms
        ),
    );
    if let Some(control) = &control {
        write_event(
            &output_dir,
            &format!(
                "{{\"type\":\"terrain_capture_control_loaded\",\"expected_width\":{},\"expected_height\":{},\"allowed_slots\":{:?},\"paired_resource_slot_count\":{},\"provenance\":\"{}\"}}",
                control.expected_width,
                control.expected_height,
                control.allowed_slots,
                control.paired_resource_slots.len(),
                json_escape(&control.provenance)
            ),
        );
    }

    match unsafe { install_hooks(&output_dir) } {
        Ok(()) => {}
        Err(error) => {
            write_event(
                &output_dir,
                &format!(
                    "{{\"type\":\"terrain_capture_blocked\",\"stage\":\"install_hooks\",\"message\":\"{}\"}}",
                    json_escape(&error)
                ),
            );
            return;
        }
    }

    let started = unsafe { GetTickCount64() };
    while !CAPTURED.load(Ordering::SeqCst) {
        let elapsed = unsafe { GetTickCount64() }.saturating_sub(started);
        if elapsed >= u64::from(timeout_ms) {
            break;
        }
        unsafe {
            Sleep(50);
        }
    }

    let captured = CAPTURED.load(Ordering::SeqCst);
    unsafe {
        disable_hooks();
    }
    if captured {
        write_event(
            &output_dir,
            "{\"type\":\"terrain_capture_finished\",\"status\":\"captured_runtime_terrain_inputs\"}",
        );
    } else {
        let message = pending_capture_message();
        write_event(
            &output_dir,
            &format!(
                "{{\"type\":\"terrain_capture_blocked\",\"stage\":\"observe_active_shader\",\"message\":\"{}\"}}",
                json_escape(&message)
            ),
        );
        write_timeout_blocked_map_capture_session(&message);
    }
}

fn write_timeout_blocked_map_capture_session(message: &str) {
    let Some((output_dir, blockers, profile, blocked_provenance)) =
        RUNTIME.get().and_then(|runtime| {
            runtime.lock().ok().map(|runtime| {
                let profile = requested_map_frame_capture_profile_for_runtime(&runtime)
                    .unwrap_or(TERRAIN_MAP_FRAME_CAPTURE_PROFILE);
                let mut blockers = vec![message.to_string()];
                blockers.extend(runtime.map_frame_rejections.iter().cloned());
                dedupe_blockers(&mut blockers);
                let blocked_provenance = match profile.capture_moment {
                    WATER_CAPTURE_MOMENT => "timeout_before_water_pass_bundle",
                    RIVERS_CAPTURE_MOMENT => "timeout_before_rivers_pass_bundle",
                    BORDERS_CAPTURE_MOMENT => "timeout_before_borders_pass_bundle",
                    TREES_CAPTURE_MOMENT => "timeout_before_trees_pass_bundle",
                    _ => "timeout_before_terrain_active_pass_bundle",
                };
                (
                    runtime.output_dir.clone(),
                    blockers,
                    profile,
                    blocked_provenance,
                )
            })
        })
    else {
        return;
    };
    if let Err(write_error) =
        write_blocked_map_capture_session(&output_dir, blockers, blocked_provenance, profile)
    {
        write_event(
            &output_dir,
            &format!(
                "{{\"type\":\"map_capture_session_write_failed\",\"file\":\"{}\",\"message\":\"{}\"}}",
                MAP_CAPTURE_SESSION_FILE,
                json_escape(&write_error)
            ),
        );
    }
}

unsafe fn install_hooks(output_dir: &Path) -> Result<(), String> {
    if HOOKS_INSTALLED.load(Ordering::SeqCst) {
        return Ok(());
    }
    let probe = unsafe { create_probe_device()? };
    let targets = unsafe {
        HookTargets {
            create_input_layout: vtable_slot(
                probe.device as *mut WinVoid,
                DEVICE_CREATE_INPUT_LAYOUT_INDEX,
            )?,
            create_vertex_shader: vtable_slot(
                probe.device as *mut WinVoid,
                DEVICE_CREATE_VERTEX_SHADER_INDEX,
            )?,
            create_pixel_shader: vtable_slot(
                probe.device as *mut WinVoid,
                DEVICE_CREATE_PIXEL_SHADER_INDEX,
            )?,
            ia_set_input_layout: vtable_slot(
                probe.context as *mut WinVoid,
                CONTEXT_IA_SET_INPUT_LAYOUT_INDEX,
            )?,
            vs_set_shader: vtable_slot(probe.context as *mut WinVoid, CONTEXT_VS_SET_SHADER_INDEX)?,
            ps_set_shader: vtable_slot(probe.context as *mut WinVoid, CONTEXT_PS_SET_SHADER_INDEX)?,
            ps_set_shader_resources: vtable_slot(
                probe.context as *mut WinVoid,
                CONTEXT_PS_SET_SHADER_RESOURCES_INDEX,
            )?,
            draw_indexed: vtable_slot(probe.context as *mut WinVoid, CONTEXT_DRAW_INDEXED_INDEX)?,
            draw: vtable_slot(probe.context as *mut WinVoid, CONTEXT_DRAW_INDEX)?,
            draw_indexed_instanced: vtable_slot(
                probe.context as *mut WinVoid,
                CONTEXT_DRAW_INDEXED_INSTANCED_INDEX,
            )?,
            draw_instanced: vtable_slot(
                probe.context as *mut WinVoid,
                CONTEXT_DRAW_INSTANCED_INDEX,
            )?,
            clear_render_target_view: vtable_slot(
                probe.context as *mut WinVoid,
                CONTEXT_CLEAR_RENDER_TARGET_VIEW_INDEX,
            )?,
            clear_depth_stencil_view: vtable_slot(
                probe.context as *mut WinVoid,
                CONTEXT_CLEAR_DEPTH_STENCIL_VIEW_INDEX,
            )?,
        }
    };

    let create_input_layout_original = unsafe {
        MinHook::create_hook(
            targets.create_input_layout as *mut StdVoid,
            hook_create_input_layout as *mut StdVoid,
        )
        .map_err(|status| format!("CreateInputLayout hook failed: {status}"))?
    };
    let create_original = unsafe {
        MinHook::create_hook(
            targets.create_pixel_shader as *mut StdVoid,
            hook_create_pixel_shader as *mut StdVoid,
        )
        .map_err(|status| format!("CreatePixelShader hook failed: {status}"))?
    };
    let create_vertex_original = unsafe {
        MinHook::create_hook(
            targets.create_vertex_shader as *mut StdVoid,
            hook_create_vertex_shader as *mut StdVoid,
        )
        .map_err(|status| format!("CreateVertexShader hook failed: {status}"))?
    };
    let ia_set_input_layout_original = unsafe {
        MinHook::create_hook(
            targets.ia_set_input_layout as *mut StdVoid,
            hook_ia_set_input_layout as *mut StdVoid,
        )
        .map_err(|status| format!("IASetInputLayout hook failed: {status}"))?
    };
    let ps_set_original = unsafe {
        MinHook::create_hook(
            targets.ps_set_shader as *mut StdVoid,
            hook_ps_set_shader as *mut StdVoid,
        )
        .map_err(|status| format!("PSSetShader hook failed: {status}"))?
    };
    let vs_set_original = unsafe {
        MinHook::create_hook(
            targets.vs_set_shader as *mut StdVoid,
            hook_vs_set_shader as *mut StdVoid,
        )
        .map_err(|status| format!("VSSetShader hook failed: {status}"))?
    };
    let ps_resources_original = unsafe {
        MinHook::create_hook(
            targets.ps_set_shader_resources as *mut StdVoid,
            hook_ps_set_shader_resources as *mut StdVoid,
        )
        .map_err(|status| format!("PSSetShaderResources hook failed: {status}"))?
    };
    let draw_indexed_original = unsafe {
        MinHook::create_hook(
            targets.draw_indexed as *mut StdVoid,
            hook_draw_indexed as *mut StdVoid,
        )
        .map_err(|status| format!("DrawIndexed hook failed: {status}"))?
    };
    let draw_original = unsafe {
        MinHook::create_hook(targets.draw as *mut StdVoid, hook_draw as *mut StdVoid)
            .map_err(|status| format!("Draw hook failed: {status}"))?
    };
    let draw_indexed_instanced_original = unsafe {
        MinHook::create_hook(
            targets.draw_indexed_instanced as *mut StdVoid,
            hook_draw_indexed_instanced as *mut StdVoid,
        )
        .map_err(|status| format!("DrawIndexedInstanced hook failed: {status}"))?
    };
    let draw_instanced_original = unsafe {
        MinHook::create_hook(
            targets.draw_instanced as *mut StdVoid,
            hook_draw_instanced as *mut StdVoid,
        )
        .map_err(|status| format!("DrawInstanced hook failed: {status}"))?
    };
    let clear_render_target_original = unsafe {
        MinHook::create_hook(
            targets.clear_render_target_view as *mut StdVoid,
            hook_clear_render_target_view as *mut StdVoid,
        )
        .map_err(|status| format!("ClearRenderTargetView hook failed: {status}"))?
    };
    let clear_depth_stencil_original = unsafe {
        MinHook::create_hook(
            targets.clear_depth_stencil_view as *mut StdVoid,
            hook_clear_depth_stencil_view as *mut StdVoid,
        )
        .map_err(|status| format!("ClearDepthStencilView hook failed: {status}"))?
    };

    unsafe {
        ORIG_CREATE_INPUT_LAYOUT = Some(mem::transmute(create_input_layout_original));
        ORIG_CREATE_VERTEX_SHADER = Some(mem::transmute(create_vertex_original));
        ORIG_CREATE_PIXEL_SHADER = Some(mem::transmute(create_original));
        ORIG_IA_SET_INPUT_LAYOUT = Some(mem::transmute(ia_set_input_layout_original));
        ORIG_VS_SET_SHADER = Some(mem::transmute(vs_set_original));
        ORIG_PS_SET_SHADER = Some(mem::transmute(ps_set_original));
        ORIG_PS_SET_SHADER_RESOURCES = Some(mem::transmute(ps_resources_original));
        ORIG_DRAW_INDEXED = Some(mem::transmute(draw_indexed_original));
        ORIG_DRAW = Some(mem::transmute(draw_original));
        ORIG_DRAW_INDEXED_INSTANCED = Some(mem::transmute(draw_indexed_instanced_original));
        ORIG_DRAW_INSTANCED = Some(mem::transmute(draw_instanced_original));
        ORIG_CLEAR_RENDER_TARGET_VIEW = Some(mem::transmute(clear_render_target_original));
        ORIG_CLEAR_DEPTH_STENCIL_VIEW = Some(mem::transmute(clear_depth_stencil_original));
        MinHook::queue_enable_hook(targets.create_input_layout as *mut StdVoid)
            .map_err(|status| format!("queue CreateInputLayout hook failed: {status}"))?;
        MinHook::queue_enable_hook(targets.create_vertex_shader as *mut StdVoid)
            .map_err(|status| format!("queue CreateVertexShader hook failed: {status}"))?;
        MinHook::queue_enable_hook(targets.create_pixel_shader as *mut StdVoid)
            .map_err(|status| format!("queue CreatePixelShader hook failed: {status}"))?;
        MinHook::queue_enable_hook(targets.ia_set_input_layout as *mut StdVoid)
            .map_err(|status| format!("queue IASetInputLayout hook failed: {status}"))?;
        MinHook::queue_enable_hook(targets.vs_set_shader as *mut StdVoid)
            .map_err(|status| format!("queue VSSetShader hook failed: {status}"))?;
        MinHook::queue_enable_hook(targets.ps_set_shader as *mut StdVoid)
            .map_err(|status| format!("queue PSSetShader hook failed: {status}"))?;
        MinHook::queue_enable_hook(targets.ps_set_shader_resources as *mut StdVoid)
            .map_err(|status| format!("queue PSSetShaderResources hook failed: {status}"))?;
        MinHook::queue_enable_hook(targets.draw_indexed as *mut StdVoid)
            .map_err(|status| format!("queue DrawIndexed hook failed: {status}"))?;
        MinHook::queue_enable_hook(targets.draw as *mut StdVoid)
            .map_err(|status| format!("queue Draw hook failed: {status}"))?;
        MinHook::queue_enable_hook(targets.draw_indexed_instanced as *mut StdVoid)
            .map_err(|status| format!("queue DrawIndexedInstanced hook failed: {status}"))?;
        MinHook::queue_enable_hook(targets.draw_instanced as *mut StdVoid)
            .map_err(|status| format!("queue DrawInstanced hook failed: {status}"))?;
        MinHook::queue_enable_hook(targets.clear_render_target_view as *mut StdVoid)
            .map_err(|status| format!("queue ClearRenderTargetView hook failed: {status}"))?;
        MinHook::queue_enable_hook(targets.clear_depth_stencil_view as *mut StdVoid)
            .map_err(|status| format!("queue ClearDepthStencilView hook failed: {status}"))?;
        MinHook::apply_queued().map_err(|status| format!("enable D3D11 hooks failed: {status}"))?;
    }

    let _ = HOOK_TARGETS.set(targets);
    HOOKS_INSTALLED.store(true, Ordering::SeqCst);
    write_event(
        output_dir,
        &format!(
            "{{\"type\":\"terrain_capture_hooks_installed\",\"create_input_layout\":\"0x{:X}\",\"create_vertex_shader\":\"0x{:X}\",\"create_pixel_shader\":\"0x{:X}\",\"ia_set_input_layout\":\"0x{:X}\",\"vs_set_shader\":\"0x{:X}\",\"ps_set_shader\":\"0x{:X}\",\"ps_set_shader_resources\":\"0x{:X}\",\"draw_indexed\":\"0x{:X}\",\"draw\":\"0x{:X}\",\"draw_indexed_instanced\":\"0x{:X}\",\"draw_instanced\":\"0x{:X}\",\"clear_render_target_view\":\"0x{:X}\",\"clear_depth_stencil_view\":\"0x{:X}\"}}",
            targets.create_input_layout,
            targets.create_vertex_shader,
            targets.create_pixel_shader,
            targets.ia_set_input_layout,
            targets.vs_set_shader,
            targets.ps_set_shader,
            targets.ps_set_shader_resources,
            targets.draw_indexed,
            targets.draw,
            targets.draw_indexed_instanced,
            targets.draw_instanced,
            targets.clear_render_target_view,
            targets.clear_depth_stencil_view
        ),
    );
    unsafe {
        release_unknown(probe.context as *mut WinVoid);
        release_unknown(probe.device as *mut WinVoid);
    }
    Ok(())
}

unsafe fn disable_hooks() {
    let Some(targets) = HOOK_TARGETS.get().copied() else {
        return;
    };
    unsafe {
        let _ = MinHook::queue_disable_hook(targets.create_input_layout as *mut StdVoid);
        let _ = MinHook::queue_disable_hook(targets.create_vertex_shader as *mut StdVoid);
        let _ = MinHook::queue_disable_hook(targets.create_pixel_shader as *mut StdVoid);
        let _ = MinHook::queue_disable_hook(targets.ia_set_input_layout as *mut StdVoid);
        let _ = MinHook::queue_disable_hook(targets.vs_set_shader as *mut StdVoid);
        let _ = MinHook::queue_disable_hook(targets.ps_set_shader as *mut StdVoid);
        let _ = MinHook::queue_disable_hook(targets.ps_set_shader_resources as *mut StdVoid);
        let _ = MinHook::queue_disable_hook(targets.draw_indexed as *mut StdVoid);
        let _ = MinHook::queue_disable_hook(targets.draw as *mut StdVoid);
        let _ = MinHook::queue_disable_hook(targets.draw_indexed_instanced as *mut StdVoid);
        let _ = MinHook::queue_disable_hook(targets.draw_instanced as *mut StdVoid);
        let _ = MinHook::queue_disable_hook(targets.clear_render_target_view as *mut StdVoid);
        let _ = MinHook::queue_disable_hook(targets.clear_depth_stencil_view as *mut StdVoid);
        let _ = MinHook::apply_queued();
        let _ = MinHook::remove_hook(targets.create_input_layout as *mut StdVoid);
        let _ = MinHook::remove_hook(targets.create_vertex_shader as *mut StdVoid);
        let _ = MinHook::remove_hook(targets.create_pixel_shader as *mut StdVoid);
        let _ = MinHook::remove_hook(targets.ia_set_input_layout as *mut StdVoid);
        let _ = MinHook::remove_hook(targets.vs_set_shader as *mut StdVoid);
        let _ = MinHook::remove_hook(targets.ps_set_shader as *mut StdVoid);
        let _ = MinHook::remove_hook(targets.ps_set_shader_resources as *mut StdVoid);
        let _ = MinHook::remove_hook(targets.draw_indexed as *mut StdVoid);
        let _ = MinHook::remove_hook(targets.draw as *mut StdVoid);
        let _ = MinHook::remove_hook(targets.draw_indexed_instanced as *mut StdVoid);
        let _ = MinHook::remove_hook(targets.draw_instanced as *mut StdVoid);
        let _ = MinHook::remove_hook(targets.clear_render_target_view as *mut StdVoid);
        let _ = MinHook::remove_hook(targets.clear_depth_stencil_view as *mut StdVoid);
    }
    HOOKS_INSTALLED.store(false, Ordering::SeqCst);
}

unsafe extern "system" fn hook_create_input_layout(
    device: *mut ID3D11Device,
    input_elements: *const D3D11_INPUT_ELEMENT_DESC,
    num_elements: UINT,
    shader_bytecode_with_input_signature: *const WinVoid,
    bytecode_length: usize,
    input_layout: *mut *mut ID3D11InputLayout,
) -> i32 {
    let Some(original) = (unsafe { ORIG_CREATE_INPUT_LAYOUT }) else {
        return 0x80004005u32 as i32;
    };
    let copied_elements = unsafe { copy_input_element_descs(input_elements, num_elements) };
    let hr = unsafe {
        original(
            device,
            input_elements,
            num_elements,
            shader_bytecode_with_input_signature,
            bytecode_length,
            input_layout,
        )
    };
    if SUCCEEDED(hr) && !input_layout.is_null() {
        let layout = unsafe { *input_layout };
        if !layout.is_null() {
            match copied_elements {
                Ok(elements) => record_input_layout_created(layout as usize, elements),
                Err(error) => record_input_layout_copy_rejected_once(&error),
            }
        }
    }
    hr
}

unsafe extern "system" fn hook_create_pixel_shader(
    device: *mut ID3D11Device,
    bytecode: *const WinVoid,
    bytecode_len: usize,
    class_linkage: *mut ID3D11ClassLinkage,
    pixel_shader: *mut *mut ID3D11PixelShader,
) -> i32 {
    let Some(original) = (unsafe { ORIG_CREATE_PIXEL_SHADER }) else {
        return 0x80004005u32 as i32;
    };
    let hr = unsafe { original(device, bytecode, bytecode_len, class_linkage, pixel_shader) };
    if SUCCEEDED(hr) && !bytecode.is_null() && !pixel_shader.is_null() {
        let shader = unsafe { *pixel_shader };
        if !shader.is_null() {
            let bytes = unsafe { slice::from_raw_parts(bytecode.cast::<u8>(), bytecode_len) };
            if let Some(runtime) = RUNTIME.get() {
                if let Ok(mut runtime) = runtime.lock() {
                    let terrain_id_srv_slot = terrain_id_slot_from_shader_bytecode(bytes);
                    if runtime
                        .state
                        .observe_pixel_shader_created(shader as usize, bytes)
                    {
                        let shader_hash = fnv1a64_bytes(bytes);
                        let shader_file = format!("pdxmap_pixel_shader_{shader_hash}.bin");
                        if let Err(error) = fs::write(runtime.output_dir.join(&shader_file), bytes)
                        {
                            write_event(
                                &runtime.output_dir,
                                &format!(
                                    "{{\"type\":\"terrain_shader_bytecode_write_failed\",\"file\":\"{}\",\"message\":\"{}\"}}",
                                    json_escape(&shader_file),
                                    json_escape(&error.to_string())
                                ),
                            );
                        }
                        let slot = terrain_id_srv_slot.unwrap_or(TERRAIN_ID_SRV_SLOT);
                        let snow_mud_slot = shader_resource_slot_from_shader_bytecode(
                            bytes,
                            SNOW_MUD_DATA_RESOURCE_SYMBOL,
                        );
                        write_event(
                            &runtime.output_dir,
                            &format!(
                                "{{\"type\":\"terrain_shader_observed\",\"pixel_shader\":\"0x{:X}\",\"terrain_id_srv_slot\":{},\"snow_mud_data_srv_slot\":{},\"byte_len\":{},\"fnv1a64\":\"{}\",\"file\":\"{}\",\"shader_count\":{}}}",
                                shader as usize,
                                slot,
                                snow_mud_slot.map_or("null".to_string(), |slot| slot.to_string()),
                                bytes.len(),
                                shader_hash,
                                json_escape(&shader_file),
                                runtime.state.pdxmap_pixel_shader_count()
                            ),
                        );
                    }
                    if runtime
                        .state
                        .observe_water_pixel_shader_created(shader as usize, bytes)
                    {
                        let shader_hash = fnv1a64_bytes(bytes);
                        let shader_file = format!("pdxwater_pixel_shader_{shader_hash}.bin");
                        if let Err(error) = fs::write(runtime.output_dir.join(&shader_file), bytes)
                        {
                            write_event(
                                &runtime.output_dir,
                                &format!(
                                    "{{\"type\":\"water_shader_bytecode_write_failed\",\"file\":\"{}\",\"message\":\"{}\"}}",
                                    json_escape(&shader_file),
                                    json_escape(&error.to_string())
                                ),
                            );
                        }
                        write_event(
                            &runtime.output_dir,
                            &format!(
                                "{{\"type\":\"water_shader_observed\",\"pixel_shader\":\"0x{:X}\",\"byte_len\":{},\"fnv1a64\":\"{}\",\"file\":\"{}\",\"shader_count\":{}}}",
                                shader as usize,
                                bytes.len(),
                                shader_hash,
                                json_escape(&shader_file),
                                runtime.state.pdxwater_pixel_shader_count()
                            ),
                        );
                    }
                    if runtime
                        .state
                        .observe_river_pixel_shader_created(shader as usize, bytes)
                    {
                        let shader_hash = fnv1a64_bytes(bytes);
                        let shader_file = format!("river_pixel_shader_{shader_hash}.bin");
                        if let Err(error) = fs::write(runtime.output_dir.join(&shader_file), bytes)
                        {
                            write_event(
                                &runtime.output_dir,
                                &format!(
                                    "{{\"type\":\"river_shader_bytecode_write_failed\",\"file\":\"{}\",\"message\":\"{}\"}}",
                                    json_escape(&shader_file),
                                    json_escape(&error.to_string())
                                ),
                            );
                        }
                        write_event(
                            &runtime.output_dir,
                            &format!(
                                "{{\"type\":\"river_shader_observed\",\"pixel_shader\":\"0x{:X}\",\"byte_len\":{},\"fnv1a64\":\"{}\",\"file\":\"{}\",\"shader_count\":{}}}",
                                shader as usize,
                                bytes.len(),
                                shader_hash,
                                json_escape(&shader_file),
                                runtime.state.river_pixel_shader_count()
                            ),
                        );
                    }
                    if runtime
                        .state
                        .observe_border_pixel_shader_created(shader as usize, bytes)
                    {
                        let shader_hash = fnv1a64_bytes(bytes);
                        let shader_file = format!("border_pixel_shader_{shader_hash}.bin");
                        if let Err(error) = fs::write(runtime.output_dir.join(&shader_file), bytes)
                        {
                            write_event(
                                &runtime.output_dir,
                                &format!(
                                    "{{\"type\":\"border_shader_bytecode_write_failed\",\"file\":\"{}\",\"message\":\"{}\"}}",
                                    json_escape(&shader_file),
                                    json_escape(&error.to_string())
                                ),
                            );
                        }
                        write_event(
                            &runtime.output_dir,
                            &format!(
                                "{{\"type\":\"border_shader_observed\",\"pixel_shader\":\"0x{:X}\",\"byte_len\":{},\"fnv1a64\":\"{}\",\"file\":\"{}\",\"shader_count\":{}}}",
                                shader as usize,
                                bytes.len(),
                                shader_hash,
                                json_escape(&shader_file),
                                runtime.state.border_pixel_shader_count()
                            ),
                        );
                    }
                    if runtime
                        .state
                        .observe_tree_pixel_shader_created(shader as usize, bytes)
                    {
                        let shader_hash = fnv1a64_bytes(bytes);
                        let shader_file = format!("tree_pixel_shader_{shader_hash}.bin");
                        if let Err(error) = fs::write(runtime.output_dir.join(&shader_file), bytes)
                        {
                            write_event(
                                &runtime.output_dir,
                                &format!(
                                    "{{\"type\":\"tree_shader_bytecode_write_failed\",\"file\":\"{}\",\"message\":\"{}\"}}",
                                    json_escape(&shader_file),
                                    json_escape(&error.to_string())
                                ),
                            );
                        }
                        write_event(
                            &runtime.output_dir,
                            &format!(
                                "{{\"type\":\"tree_shader_observed\",\"pixel_shader\":\"0x{:X}\",\"byte_len\":{},\"fnv1a64\":\"{}\",\"file\":\"{}\",\"shader_count\":{}}}",
                                shader as usize,
                                bytes.len(),
                                shader_hash,
                                json_escape(&shader_file),
                                runtime.state.tree_pixel_shader_count()
                            ),
                        );
                    }
                }
            }
        }
    }
    hr
}

unsafe extern "system" fn hook_create_vertex_shader(
    device: *mut ID3D11Device,
    bytecode: *const WinVoid,
    bytecode_len: usize,
    class_linkage: *mut ID3D11ClassLinkage,
    vertex_shader: *mut *mut ID3D11VertexShader,
) -> i32 {
    let Some(original) = (unsafe { ORIG_CREATE_VERTEX_SHADER }) else {
        return 0x80004005u32 as i32;
    };
    let hr = unsafe { original(device, bytecode, bytecode_len, class_linkage, vertex_shader) };
    if SUCCEEDED(hr) && !bytecode.is_null() && !vertex_shader.is_null() {
        let shader = unsafe { *vertex_shader };
        if !shader.is_null() {
            let bytes = unsafe { slice::from_raw_parts(bytecode.cast::<u8>(), bytecode_len) };
            let reflection = reflect_terrain_vertex_shader_metadata(bytes);
            if let Some(runtime) = RUNTIME.get() {
                if let Ok(mut runtime) = runtime.lock() {
                    let terrain_shader_observed = runtime.state.observe_vertex_shader_created(
                        shader as usize,
                        bytes,
                        reflection.clone(),
                    );
                    if terrain_shader_observed {
                        let shader_hash = fnv1a64_bytes(bytes);
                        let shader_file = format!("pdxmap_vertex_shader_{shader_hash}.bin");
                        if let Err(error) = fs::write(runtime.output_dir.join(&shader_file), bytes)
                        {
                            write_event(
                                &runtime.output_dir,
                                &format!(
                                    "{{\"type\":\"terrain_vertex_shader_bytecode_write_failed\",\"file\":\"{}\",\"message\":\"{}\"}}",
                                    json_escape(&shader_file),
                                    json_escape(&error.to_string())
                                ),
                            );
                        }
                        let metadata = runtime
                            .state
                            .terrain_vertex_shader_metadata(shader as usize)
                            .cloned()
                            .expect("observe success records vertex shader metadata");
                        write_event(
                            &runtime.output_dir,
                            &format!(
                                "{{\"type\":\"terrain_vertex_shader_observed\",\"vertex_shader\":\"0x{:X}\",\"byte_len\":{},\"fnv1a64\":\"{}\",\"file\":\"{}\",\"input_parameter_count\":{},\"constant_buffer_count\":{},\"shader_count\":{}}}",
                                shader as usize,
                                bytes.len(),
                                shader_hash,
                                json_escape(&shader_file),
                                metadata.input_parameters.len(),
                                metadata.constant_buffers.len(),
                                runtime.state.terrain_vertex_shader_count()
                            ),
                        );
                    }
                    if !terrain_shader_observed {
                        if let Some(metadata) = runtime
                            .state
                            .map_vertex_shader_metadata(shader as usize)
                            .cloned()
                        {
                            let shader_file = format!("map_vertex_shader_{}.bin", metadata.fnv1a64);
                            if let Err(error) =
                                fs::write(runtime.output_dir.join(&shader_file), bytes)
                            {
                                write_event(
                                    &runtime.output_dir,
                                    &format!(
                                        "{{\"type\":\"map_vertex_shader_bytecode_write_failed\",\"file\":\"{}\",\"message\":\"{}\"}}",
                                        json_escape(&shader_file),
                                        json_escape(&error.to_string())
                                    ),
                                );
                            }
                            write_event(
                                &runtime.output_dir,
                                &format!(
                                    "{{\"type\":\"map_vertex_shader_observed\",\"vertex_shader\":\"0x{:X}\",\"byte_len\":{},\"fnv1a64\":\"{}\",\"file\":\"{}\",\"input_parameter_count\":{},\"constant_buffer_count\":{},\"shader_count\":{}}}",
                                    shader as usize,
                                    metadata.byte_len,
                                    metadata.fnv1a64,
                                    json_escape(&shader_file),
                                    metadata.input_parameters.len(),
                                    metadata.constant_buffers.len(),
                                    runtime.state.map_vertex_shader_count()
                                ),
                            );
                        }
                    }
                }
            }
        }
    }
    hr
}

unsafe extern "system" fn hook_ia_set_input_layout(
    context: *mut ID3D11DeviceContext,
    input_layout: *mut ID3D11InputLayout,
) {
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            runtime
                .state
                .observe_ia_set_input_layout(input_layout as usize);
        }
    }
    if let Some(original) = unsafe { ORIG_IA_SET_INPUT_LAYOUT } {
        unsafe {
            original(context, input_layout);
        }
    }
}

unsafe extern "system" fn hook_vs_set_shader(
    context: *mut ID3D11DeviceContext,
    vertex_shader: *mut ID3D11VertexShader,
    class_instances: *const *mut ID3D11ClassInstance,
    class_instance_count: UINT,
) {
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            runtime.state.observe_vs_set_shader(vertex_shader as usize);
        }
    }
    if let Some(original) = unsafe { ORIG_VS_SET_SHADER } {
        unsafe {
            original(
                context,
                vertex_shader,
                class_instances,
                class_instance_count,
            );
        }
    }
}

unsafe extern "system" fn hook_ps_set_shader(
    context: *mut ID3D11DeviceContext,
    pixel_shader: *mut ID3D11PixelShader,
    class_instances: *const *mut ID3D11ClassInstance,
    class_instance_count: UINT,
) {
    let mut finalize_map_frame = false;
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            let profile = requested_map_frame_capture_profile_for_runtime(&runtime).ok();
            let previous_map_shader = profile.and_then(|profile| {
                active_map_frame_pixel_shader_pointer_for_profile(&runtime.state, profile)
            });
            runtime.state.observe_ps_set_shader(pixel_shader as usize);
            let current_map_shader = profile.and_then(|profile| {
                active_map_frame_pixel_shader_pointer_for_profile(&runtime.state, profile)
            });
            finalize_map_frame = previous_map_shader.is_some()
                && current_map_shader.is_none()
                && !runtime.map_frame_draws.is_empty()
                && profile.is_some_and(|profile| {
                    !runtime
                        .completed_map_frame_moments
                        .contains(profile.capture_moment)
                });
        }
    }
    if finalize_map_frame {
        finalize_map_frame_bundle_pass(context, "ps_shader_transition");
    }
    if let Some(original) = unsafe { ORIG_PS_SET_SHADER } {
        unsafe {
            original(context, pixel_shader, class_instances, class_instance_count);
        }
    }
}

unsafe extern "system" fn hook_ps_set_shader_resources(
    context: *mut ID3D11DeviceContext,
    start_slot: UINT,
    num_views: UINT,
    views: *const *mut ID3D11ShaderResourceView,
) {
    if let Some(original) = unsafe { ORIG_PS_SET_SHADER_RESOURCES } {
        unsafe {
            original(context, start_slot, num_views, views);
        }
    }
    if CAPTURED.load(Ordering::SeqCst) || CAPTURE_IN_PROGRESS.load(Ordering::SeqCst) {
        return;
    }
    let srvs = if views.is_null() || num_views == 0 {
        Vec::new()
    } else {
        unsafe { slice::from_raw_parts(views, num_views as usize) }
            .iter()
            .map(|srv| *srv as usize)
            .collect::<Vec<_>>()
    };
    let candidates = candidate_resource_bindings(start_slot, &srvs);
    if candidates.is_empty() {
        return;
    }
    if CAPTURE_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        return;
    }
    let mut last_error = None;
    for candidate in candidates {
        if !candidate_requirement_met(&candidate) {
            continue;
        }
        let result = unsafe {
            capture_runtime_texture(
                context,
                candidate.target,
                candidate.binding.srv as *mut ID3D11ShaderResourceView,
                candidate.binding.slot,
                candidate.identity,
            )
        };
        match result {
            Ok(meta) => {
                record_capture_success(candidate.target, &meta);
                if candidate.target.symbol == TERRAIN_ID_RESOURCE_SYMBOL {
                    unsafe {
                        capture_paired_runtime_resources(context, candidate.binding.slot);
                        capture_scene_constant_buffer(context, 0);
                    }
                }
                if CAPTURED.load(Ordering::SeqCst) {
                    CAPTURE_IN_PROGRESS.store(false, Ordering::SeqCst);
                    return;
                }
                last_error = None;
                continue;
            }
            Err(error) => {
                last_error = Some(error.clone());
                record_candidate_rejected(
                    candidate.target,
                    candidate.binding.slot,
                    candidate.identity,
                    &error,
                );
            }
        }
    }
    if let Some(error) = last_error {
        if let Some(runtime) = RUNTIME.get() {
            if let Ok(mut runtime) = runtime.lock() {
                runtime.last_error = Some(error);
            }
        }
    }
    CAPTURE_IN_PROGRESS.store(false, Ordering::SeqCst);
}

unsafe extern "system" fn hook_draw_indexed(
    context: *mut ID3D11DeviceContext,
    index_count: UINT,
    start_index_location: UINT,
    base_vertex_location: i32,
) {
    let captured_map_frame_draw = unsafe {
        capture_terrain_input_layout_if_active(
            context,
            "DrawIndexed",
            index_count,
            start_index_location,
            base_vertex_location,
        );
        capture_terrain_vertex_buffer_if_active(
            context,
            "DrawIndexed",
            index_count,
            start_index_location,
            base_vertex_location,
        );
        capture_map_frame_bundle_if_active(
            context,
            "DrawIndexed",
            index_count,
            start_index_location,
            base_vertex_location,
            None,
            None,
            None,
        )
    };
    if let Some(original) = unsafe { ORIG_DRAW_INDEXED } {
        unsafe {
            original(
                context,
                index_count,
                start_index_location,
                base_vertex_location,
            );
        }
    }
    if captured_map_frame_draw {
        unsafe { capture_map_frame_post_draw_output_if_active(context) };
    }
}

unsafe extern "system" fn hook_draw(
    context: *mut ID3D11DeviceContext,
    vertex_count: UINT,
    start_vertex_location: UINT,
) {
    let captured_map_frame_draw = unsafe {
        capture_terrain_input_layout_if_active(
            context,
            "Draw",
            vertex_count,
            start_vertex_location,
            0,
        );
        capture_terrain_vertex_buffer_if_active(
            context,
            "Draw",
            vertex_count,
            start_vertex_location,
            0,
        );
        capture_map_frame_bundle_if_active(
            context,
            "Draw",
            vertex_count,
            start_vertex_location,
            0,
            None,
            None,
            None,
        )
    };
    if let Some(original) = unsafe { ORIG_DRAW } {
        unsafe {
            original(context, vertex_count, start_vertex_location);
        }
    }
    if captured_map_frame_draw {
        unsafe { capture_map_frame_post_draw_output_if_active(context) };
    }
}

unsafe extern "system" fn hook_draw_indexed_instanced(
    context: *mut ID3D11DeviceContext,
    index_count_per_instance: UINT,
    instance_count: UINT,
    start_index_location: UINT,
    base_vertex_location: i32,
    start_instance_location: UINT,
) {
    let draw_count = index_count_per_instance.saturating_mul(instance_count);
    let captured_map_frame_draw = unsafe {
        capture_terrain_input_layout_if_active(
            context,
            "DrawIndexedInstanced",
            draw_count,
            start_index_location,
            base_vertex_location,
        );
        capture_terrain_vertex_buffer_if_active(
            context,
            "DrawIndexedInstanced",
            draw_count,
            start_index_location,
            base_vertex_location,
        );
        capture_map_frame_bundle_if_active(
            context,
            "DrawIndexedInstanced",
            draw_count,
            start_index_location,
            base_vertex_location,
            Some(index_count_per_instance),
            Some(instance_count),
            Some(start_instance_location),
        )
    };
    if let Some(original) = unsafe { ORIG_DRAW_INDEXED_INSTANCED } {
        unsafe {
            original(
                context,
                index_count_per_instance,
                instance_count,
                start_index_location,
                base_vertex_location,
                start_instance_location,
            );
        }
    }
    if captured_map_frame_draw {
        unsafe { capture_map_frame_post_draw_output_if_active(context) };
    }
}

unsafe extern "system" fn hook_draw_instanced(
    context: *mut ID3D11DeviceContext,
    vertex_count_per_instance: UINT,
    instance_count: UINT,
    start_vertex_location: UINT,
    start_instance_location: UINT,
) {
    let draw_count = vertex_count_per_instance.saturating_mul(instance_count);
    let captured_map_frame_draw = unsafe {
        capture_terrain_input_layout_if_active(
            context,
            "DrawInstanced",
            draw_count,
            start_vertex_location,
            0,
        );
        capture_terrain_vertex_buffer_if_active(
            context,
            "DrawInstanced",
            draw_count,
            start_vertex_location,
            0,
        );
        capture_map_frame_bundle_if_active(
            context,
            "DrawInstanced",
            draw_count,
            start_vertex_location,
            0,
            Some(vertex_count_per_instance),
            Some(instance_count),
            Some(start_instance_location),
        )
    };
    if let Some(original) = unsafe { ORIG_DRAW_INSTANCED } {
        unsafe {
            original(
                context,
                vertex_count_per_instance,
                instance_count,
                start_vertex_location,
                start_instance_location,
            );
        }
    }
    if captured_map_frame_draw {
        unsafe { capture_map_frame_post_draw_output_if_active(context) };
    }
}

unsafe extern "system" fn hook_clear_render_target_view(
    context: *mut ID3D11DeviceContext,
    render_target_view: *mut ID3D11RenderTargetView,
    color_rgba: *const f32,
) {
    if should_finalize_map_frame_bundle_at_render_target_clear() {
        finalize_map_frame_bundle_pass(context, "pre_clear_render_target_frame_boundary");
    }
    if let Some(original) = unsafe { ORIG_CLEAR_RENDER_TARGET_VIEW } {
        unsafe {
            original(context, render_target_view, color_rgba);
        }
    }
    if render_target_view.is_null() || color_rgba.is_null() {
        return;
    }
    let color = unsafe {
        [
            *color_rgba,
            *color_rgba.add(1),
            *color_rgba.add(2),
            *color_rgba.add(3),
        ]
    };
    let resource_pointer = unsafe { render_target_view_resource_pointer(render_target_view) };
    record_om_render_target_clear(render_target_view as usize, resource_pointer, color);
}

fn should_finalize_map_frame_bundle_at_render_target_clear() -> bool {
    if capture_symbol_observed(MAP_FRAME_BUNDLE_SYMBOL) {
        return false;
    }
    RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .and_then(|runtime| {
            requested_map_frame_capture_profile_for_runtime(&runtime)
                .ok()
                .map(|profile| {
                    runtime.map_frame_draws.len() >= profile.min_draws_before_clear_finalize
                })
        })
        .unwrap_or(false)
}

unsafe extern "system" fn hook_clear_depth_stencil_view(
    context: *mut ID3D11DeviceContext,
    depth_stencil_view: *mut ID3D11DepthStencilView,
    clear_flags: UINT,
    depth: f32,
    stencil: u8,
) {
    if let Some(original) = unsafe { ORIG_CLEAR_DEPTH_STENCIL_VIEW } {
        unsafe {
            original(context, depth_stencil_view, clear_flags, depth, stencil);
        }
    }
    if depth_stencil_view.is_null() {
        return;
    }
    let resource_pointer = unsafe { depth_stencil_view_resource_pointer(depth_stencil_view) };
    record_om_depth_stencil_clear(
        depth_stencil_view as usize,
        resource_pointer,
        clear_flags,
        depth,
        stencil,
    );
}

unsafe fn render_target_view_resource_pointer(view: *mut ID3D11RenderTargetView) -> Option<usize> {
    if view.is_null() {
        return None;
    }
    let mut resource: *mut ID3D11Resource = ptr::null_mut();
    unsafe {
        (*view).GetResource(&mut resource);
    }
    if resource.is_null() {
        return None;
    }
    let pointer = resource as usize;
    unsafe {
        release_unknown(resource as *mut WinVoid);
    }
    Some(pointer)
}

unsafe fn depth_stencil_view_resource_pointer(view: *mut ID3D11DepthStencilView) -> Option<usize> {
    if view.is_null() {
        return None;
    }
    let mut resource: *mut ID3D11Resource = ptr::null_mut();
    unsafe {
        (*view).GetResource(&mut resource);
    }
    if resource.is_null() {
        return None;
    }
    let pointer = resource as usize;
    unsafe {
        release_unknown(resource as *mut WinVoid);
    }
    Some(pointer)
}

fn record_om_render_target_clear(
    view_pointer: usize,
    resource_pointer: Option<usize>,
    color: [f32; 4],
) {
    if view_pointer == 0 {
        return;
    }
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            runtime.om_clear_sequence = runtime.om_clear_sequence.saturating_add(1);
            let clear = MapFrameRenderTargetClearFile {
                sequence: runtime.om_clear_sequence,
                color,
            };
            runtime
                .om_render_target_clears
                .insert(view_pointer, clear.clone());
            if let Some(resource_pointer) = resource_pointer.filter(|pointer| *pointer != 0) {
                runtime
                    .om_render_target_clears
                    .insert(resource_pointer, clear);
            }
        }
    }
}

fn record_om_depth_stencil_clear(
    view_pointer: usize,
    resource_pointer: Option<usize>,
    clear_flags: UINT,
    depth: f32,
    stencil: u8,
) {
    if view_pointer == 0 {
        return;
    }
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            runtime.om_clear_sequence = runtime.om_clear_sequence.saturating_add(1);
            let clear = MapFrameDepthStencilClearFile {
                sequence: runtime.om_clear_sequence,
                clear_flags,
                depth,
                stencil,
            };
            runtime
                .om_depth_stencil_clears
                .insert(view_pointer, clear.clone());
            if let Some(resource_pointer) = resource_pointer.filter(|pointer| *pointer != 0) {
                runtime
                    .om_depth_stencil_clears
                    .insert(resource_pointer, clear);
            }
        }
    }
}

unsafe fn copy_input_element_descs(
    input_elements: *const D3D11_INPUT_ELEMENT_DESC,
    num_elements: UINT,
) -> Result<Vec<TerrainInputElementDesc>, String> {
    if input_elements.is_null() {
        return Err("CreateInputLayout received a null D3D11_INPUT_ELEMENT_DESC array".to_string());
    }
    if num_elements == 0 {
        return Err("CreateInputLayout received zero input elements".to_string());
    }
    if num_elements > D3D11_IA_VERTEX_INPUT_STRUCTURE_ELEMENT_COUNT {
        return Err(format!(
            "CreateInputLayout descriptor count {num_elements} exceeds D3D11_IA_VERTEX_INPUT_STRUCTURE_ELEMENT_COUNT"
        ));
    }
    let native = unsafe { slice::from_raw_parts(input_elements, num_elements as usize) };
    native
        .iter()
        .enumerate()
        .map(|(index, element)| {
            if element.SemanticName.is_null() {
                return Err(format!(
                    "CreateInputLayout element {index} has a null SemanticName"
                ));
            }
            let semantic_name = unsafe { copy_semantic_name(element.SemanticName, index)? };
            if semantic_name.is_empty() {
                return Err(format!(
                    "CreateInputLayout element {index} has an empty SemanticName"
                ));
            }
            Ok(TerrainInputElementDesc {
                semantic_name,
                semantic_index: element.SemanticIndex,
                format: element.Format as u32,
                input_slot: element.InputSlot,
                aligned_byte_offset: element.AlignedByteOffset,
                input_slot_class: element.InputSlotClass as u32,
                instance_data_step_rate: element.InstanceDataStepRate,
            })
        })
        .collect()
}

unsafe fn copy_semantic_name(ptr: *const i8, element_index: usize) -> Result<String, String> {
    let mut bytes = Vec::new();
    for offset in 0..=INPUT_SEMANTIC_NAME_MAX_BYTES {
        let byte = unsafe { *ptr.cast::<u8>().add(offset) };
        if byte == 0 {
            return String::from_utf8(bytes).map_err(|error| {
                format!(
                    "CreateInputLayout element {element_index} SemanticName is not UTF-8: {error}"
                )
            });
        }
        bytes.push(byte);
    }
    Err(format!(
        "CreateInputLayout element {element_index} SemanticName is not null terminated within {INPUT_SEMANTIC_NAME_MAX_BYTES} bytes"
    ))
}

fn record_input_layout_created(layout_ptr: usize, elements: Vec<TerrainInputElementDesc>) {
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            if !runtime
                .state
                .observe_input_layout_created(layout_ptr, elements.clone())
            {
                return;
            }
            if validate_terrain_input_layout(&elements, u32::MAX).is_ok() {
                write_event(
                    &runtime.output_dir,
                    &format!(
                        "{{\"type\":\"terrain_input_layout_candidate_observed\",\"input_layout\":\"0x{:X}\",\"element_count\":{},\"layout_count\":{},\"shader_input_source\":\"{}\"}}",
                        layout_ptr,
                        elements.len(),
                        runtime.state.input_layout_count(),
                        json_escape(TERRAIN_INPUT_LAYOUT_SHADER_SOURCE)
                    ),
                );
            }
        }
    }
}

fn record_input_layout_copy_rejected_once(error: &str) {
    if INPUT_LAYOUT_COPY_REJECTION_RECORDED.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            runtime.last_error = Some(error.to_string());
            write_event(
                &runtime.output_dir,
                &format!(
                    "{{\"type\":\"terrain_input_layout_copy_rejected\",\"message\":\"{}\"}}",
                    json_escape(error)
                ),
            );
        }
    }
}

fn record_capture_success(target: CaptureTarget, meta: &CapturedTextureMeta) {
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            runtime.captured_symbols.insert(target.symbol.to_string());
            if target.symbol == TERRAIN_ID_RESOURCE_SYMBOL {
                runtime.terrain_id_srv_slot_observed = Some(meta.slot);
            }
            write_event(
                &runtime.output_dir,
                &format!(
                    "{{\"type\":\"runtime_texture_captured\",\"symbol\":\"{}\",\"file\":\"{}\",\"width\":{},\"height\":{},\"byte_len\":{},\"fnv1a64\":\"{}\",\"source_dxgi_format\":{},\"shader_resource_slot\":{},\"identity\":\"{}\"}}",
                    target.symbol,
                    target.rgba_file,
                    meta.width,
                    meta.height,
                    meta.byte_len,
                    meta.fnv1a64,
                    meta.source_format,
                    meta.slot,
                    json_escape(&meta.identity)
                ),
            );
            if refresh_session_capture_progress(&mut runtime) {
                CAPTURED.store(true, Ordering::SeqCst);
            }
        }
    }
}

fn capture_complete(runtime: &CaptureRuntime) -> bool {
    runtime
        .capture_moment_sequence
        .iter()
        .all(|moment| match map_frame_capture_profile(moment) {
            Ok(profile) => capture_profile_complete(runtime, profile),
            Err(_) => false,
        })
}

fn capture_profile_complete(runtime: &CaptureRuntime, profile: MapFrameCaptureProfile) -> bool {
    if !runtime
        .completed_map_frame_moments
        .contains(profile.capture_moment)
    {
        return false;
    }
    if !profile.requires_terrain_runtime_files {
        return true;
    }
    terrain_runtime_inputs_complete(runtime)
}

fn terrain_runtime_inputs_complete(runtime: &CaptureRuntime) -> bool {
    CAPTURE_TARGETS
        .iter()
        .all(|target| runtime.captured_symbols.contains(target.symbol))
        && runtime
            .captured_symbols
            .contains(TERRAIN_VERTEX_BUFFER_SYMBOL)
        && runtime
            .captured_symbols
            .contains(TERRAIN_INPUT_LAYOUT_SYMBOL)
}

fn refresh_session_capture_progress(runtime: &mut CaptureRuntime) -> bool {
    while runtime.active_capture_index < runtime.capture_moment_sequence.len() {
        let Ok(profile) = requested_map_frame_capture_profile_for_runtime(runtime) else {
            return false;
        };
        if !capture_profile_complete(runtime, profile) {
            break;
        }
        if runtime.active_capture_index + 1 >= runtime.capture_moment_sequence.len() {
            break;
        }
        let completed_moment = profile.capture_moment;
        runtime.active_capture_index += 1;
        reset_current_map_frame_capture(runtime);
        let next_profile = requested_map_frame_capture_profile_for_runtime(runtime)
            .unwrap_or(TERRAIN_MAP_FRAME_CAPTURE_PROFILE);
        write_event(
            &runtime.output_dir,
            &format!(
                "{{\"type\":\"map_capture_moment_advanced\",\"completed\":\"{}\",\"next\":\"{}\"}}",
                completed_moment, next_profile.capture_moment
            ),
        );
    }
    capture_complete(runtime)
}

fn reset_current_map_frame_capture(runtime: &mut CaptureRuntime) {
    runtime.map_frame_draws.clear();
    runtime.map_frame_blockers.clear();
    runtime.map_frame_rejections.clear();
    runtime.latest_post_draw_output_merger = None;
    runtime.latest_post_draw_output_blockers.clear();
}

fn record_candidate_rejected(
    target: CaptureTarget,
    slot: u32,
    identity: &'static str,
    error: &str,
) {
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            runtime.last_error = Some(error.to_string());
            write_event(
                &runtime.output_dir,
                &format!(
                    "{{\"type\":\"terrain_capture_candidate_rejected\",\"symbol\":\"{}\",\"slot\":{},\"identity\":\"{}\",\"message\":\"{}\"}}",
                    target.symbol,
                    slot,
                    json_escape(identity),
                    json_escape(error)
                ),
            );
        }
    }
}

unsafe fn capture_terrain_input_layout_if_active(
    context: *mut ID3D11DeviceContext,
    draw_call: &'static str,
    draw_count: UINT,
    draw_start: UINT,
    base_vertex_location: i32,
) {
    if capture_symbol_observed(TERRAIN_INPUT_LAYOUT_SYMBOL) {
        return;
    }
    let Some(shader) = active_pdxmap_shader_binding() else {
        return;
    };
    if INPUT_LAYOUT_CAPTURE_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        return;
    }
    let result = unsafe {
        capture_active_terrain_input_layout(
            context,
            draw_call,
            draw_count,
            draw_start,
            base_vertex_location,
            shader,
        )
    };
    match result {
        Ok(meta) => record_input_layout_success(&meta),
        Err(error) => record_input_layout_rejected_once(draw_call, shader, &error),
    }
    INPUT_LAYOUT_CAPTURE_IN_PROGRESS.store(false, Ordering::SeqCst);
}

#[derive(Debug)]
struct CapturedInputLayoutMeta {
    elements: Vec<TerrainInputElementDesc>,
    vertex_stride: u32,
    vertex_offset: u32,
    draw_call: &'static str,
    draw_count: u32,
    draw_start: u32,
    base_vertex_location: i32,
    terrain_id_srv_slot: u32,
}

#[derive(Clone, Debug, Serialize)]
struct MapFrameBundleFile {
    schema: &'static str,
    provenance: &'static str,
    capture_moment: &'static str,
    session_manifest: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    shader_cache_constants: Option<MapFrameShaderCacheConstantsFile>,
    active_draw: MapFrameDrawFile,
    draws: Vec<MapFrameDrawFile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    post_pass_output_merger: Option<MapFrameOutputMergerStateFile>,
    vanilla_sources: BTreeMap<String, MapFrameFileEvidence>,
    runtime_files: BTreeMap<String, MapFrameFileEvidence>,
    blockers: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
struct MapFrameShaderCacheConstantsFile {
    map_size_x: f32,
    map_size_y: f32,
    map_pow2_x: f32,
    map_pow2_y: f32,
    fow_pow2_x: f32,
    fow_pow2_y: f32,
    terrain_id_srv_slot: u32,
    cache_files: Vec<String>,
    provenance: String,
    extraction: &'static str,
}

#[derive(Clone, Debug, Serialize)]
struct MapCaptureSessionFile {
    schema: &'static str,
    provenance: &'static str,
    required_first_moment: &'static str,
    moments: Vec<MapCaptureSessionMomentFile>,
    deferred_moments: Vec<MapCaptureDeferredMomentFile>,
}

#[derive(Clone, Debug, Serialize)]
struct MapCaptureSessionMomentFile {
    name: &'static str,
    status: &'static str,
    bundle_file: &'static str,
    required_payload: Vec<&'static str>,
    blockers: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
struct MapCaptureDeferredMomentFile {
    name: &'static str,
    status: &'static str,
    required_after: &'static str,
    blocker: &'static str,
}

#[derive(Clone, Debug, Serialize)]
struct MapFrameDrawFile {
    draw_index: u32,
    draw_call: &'static str,
    draw_count: u32,
    draw_start: u32,
    base_vertex_location: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    per_instance_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    instance_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    start_instance_location: Option<u32>,
    topology: Option<u32>,
    shader: MapFrameShaderFile,
    vertex_shader: MapFrameShaderFile,
    input_layout: MapFrameInputLayoutFile,
    vertex_buffers: BTreeMap<String, MapFrameBufferBindingFile>,
    index_buffer: Option<MapFrameIndexBufferBindingFile>,
    constant_buffers: Vec<MapFrameBufferBindingFile>,
    shader_resources: Vec<MapFrameTextureBindingFile>,
    samplers: Vec<MapFrameSamplerBindingFile>,
    draw_state: Option<MapFrameDrawStateFile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    post_draw_output_merger: Option<MapFrameOutputMergerStateFile>,
}

#[derive(Clone, Debug, Serialize)]
struct MapFrameDrawStateFile {
    viewports: Vec<MapFrameViewportFile>,
    scissor_rects: Vec<MapFrameScissorRectFile>,
    rasterizer: MapFrameRasterizerStateFile,
    blend: MapFrameBlendStateFile,
    depth_stencil: MapFrameDepthStencilStateFile,
    output_merger: MapFrameOutputMergerStateFile,
}

#[derive(Clone, Debug, Serialize)]
struct MapFrameViewportFile {
    top_left_x: f32,
    top_left_y: f32,
    width: f32,
    height: f32,
    min_depth: f32,
    max_depth: f32,
}

#[derive(Clone, Debug, Serialize)]
struct MapFrameScissorRectFile {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[derive(Clone, Debug, Serialize)]
struct MapFrameRasterizerStateFile {
    default_state: bool,
    desc: Option<MapFrameRasterizerDescFile>,
}

#[derive(Clone, Debug, Serialize)]
struct MapFrameRasterizerDescFile {
    fill_mode: u32,
    cull_mode: u32,
    front_counter_clockwise: bool,
    depth_bias: i32,
    depth_bias_clamp: f32,
    slope_scaled_depth_bias: f32,
    depth_clip_enable: bool,
    scissor_enable: bool,
    multisample_enable: bool,
    antialiased_line_enable: bool,
}

#[derive(Clone, Debug, Serialize)]
struct MapFrameBlendStateFile {
    default_state: bool,
    blend_factor: [f32; 4],
    sample_mask: u32,
    desc: Option<MapFrameBlendDescFile>,
}

#[derive(Clone, Debug, Serialize)]
struct MapFrameBlendDescFile {
    alpha_to_coverage_enable: bool,
    independent_blend_enable: bool,
    render_targets: Vec<MapFrameRenderTargetBlendDescFile>,
}

#[derive(Clone, Debug, Serialize)]
struct MapFrameRenderTargetBlendDescFile {
    blend_enable: bool,
    src_blend: u32,
    dest_blend: u32,
    blend_op: u32,
    src_blend_alpha: u32,
    dest_blend_alpha: u32,
    blend_op_alpha: u32,
    render_target_write_mask: u8,
}

#[derive(Clone, Debug, Serialize)]
struct MapFrameDepthStencilStateFile {
    default_state: bool,
    stencil_ref: u32,
    desc: Option<MapFrameDepthStencilDescFile>,
}

#[derive(Clone, Debug, Serialize)]
struct MapFrameDepthStencilDescFile {
    depth_enable: bool,
    depth_write_mask: u32,
    depth_func: u32,
    stencil_enable: bool,
    stencil_read_mask: u8,
    stencil_write_mask: u8,
    front_face: MapFrameDepthStencilOpDescFile,
    back_face: MapFrameDepthStencilOpDescFile,
}

#[derive(Clone, Debug, Serialize)]
struct MapFrameDepthStencilOpDescFile {
    stencil_fail_op: u32,
    stencil_depth_fail_op: u32,
    stencil_pass_op: u32,
    stencil_func: u32,
}

#[derive(Clone, Debug, Serialize)]
struct MapFrameOutputMergerStateFile {
    render_targets: Vec<MapFrameOutputMergerTextureFile>,
    depth_stencil: Option<MapFrameOutputMergerTextureFile>,
}

#[derive(Clone, Debug, Serialize)]
struct MapFrameOutputMergerTextureFile {
    slot: Option<u32>,
    view_pointer: String,
    resource_pointer: String,
    desc: MapFrameTexture2DDescFile,
    #[serde(skip_serializing_if = "Option::is_none")]
    view_desc: Option<MapFrameOutputMergerViewDescFile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pre_draw_payload: Option<MapFrameOutputMergerTexturePayloadFile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    post_pass_payload: Option<MapFrameOutputMergerTexturePayloadFile>,
    last_render_target_clear: Option<MapFrameRenderTargetClearFile>,
    last_depth_stencil_clear: Option<MapFrameDepthStencilClearFile>,
}

#[derive(Clone, Debug, Serialize)]
struct MapFrameOutputMergerViewDescFile {
    format: u32,
    view_dimension: u32,
    mip_slice: u32,
    first_array_slice: u32,
    array_size: u32,
    flags: u32,
}

#[derive(Clone, Debug, Serialize)]
struct MapFrameOutputMergerTexturePayloadFile {
    file: String,
    byte_len: usize,
    fnv1a64: String,
    payload_format: &'static str,
    subresources: Vec<MapFrameTextureSubresourceFile>,
    provenance: &'static str,
}

#[derive(Clone, Debug, Serialize)]
struct MapFrameTexture2DDescFile {
    width: u32,
    height: u32,
    mip_levels: u32,
    array_size: u32,
    format: u32,
    sample_count: u32,
    sample_quality: u32,
    usage: u32,
    bind_flags: u32,
    cpu_access_flags: u32,
    misc_flags: u32,
}

#[derive(Clone, Debug, Serialize)]
struct MapFrameRenderTargetClearFile {
    sequence: u64,
    color: [f32; 4],
}

#[derive(Clone, Debug, Serialize)]
struct MapFrameDepthStencilClearFile {
    sequence: u64,
    clear_flags: u32,
    depth: f32,
    stencil: u8,
}

#[derive(Clone, Debug, Serialize)]
struct MapFrameShaderFile {
    stage: &'static str,
    pointer: String,
    bytecode_file: String,
    byte_len: usize,
    fnv1a64: String,
    reflection_resources: BTreeMap<String, u32>,
    input_parameters: Vec<ShaderInputParameter>,
    constant_buffers: BTreeMap<String, u32>,
    source_evidence: Vec<&'static str>,
}

#[derive(Clone, Debug, Serialize)]
struct MapFrameInputLayoutFile {
    pointer: String,
    elements: Vec<TerrainInputElementDesc>,
}

#[derive(Clone, Debug, Serialize)]
struct MapFrameBufferBindingFile {
    stage: &'static str,
    slot: u32,
    declared_name: Option<String>,
    file: Option<String>,
    byte_len: usize,
    fnv1a64: Option<String>,
    stride: Option<u32>,
    offset: Option<u32>,
    dxgi_format: Option<u32>,
    bind_flags: u32,
    usage: u32,
    cpu_access_flags: u32,
}

#[derive(Clone, Debug, Serialize)]
struct MapFrameIndexBufferBindingFile {
    file: Option<String>,
    byte_len: usize,
    fnv1a64: Option<String>,
    dxgi_format: u32,
    offset: u32,
    bind_flags: u32,
    usage: u32,
    cpu_access_flags: u32,
}

#[derive(Clone, Debug, Serialize)]
struct MapFrameTextureBindingFile {
    stage: &'static str,
    slot: u32,
    declared_name: Option<String>,
    is_null: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    view_desc: Option<MapFrameShaderResourceViewDescFile>,
    file: Option<String>,
    byte_len: usize,
    fnv1a64: Option<String>,
    dxgi_format: u32,
    width: u32,
    height: u32,
    mip_count: u32,
    array_size: u32,
    sample_count: u32,
    sample_quality: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload_format: Option<&'static str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    subresources: Vec<MapFrameTextureSubresourceFile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provenance: Option<&'static str>,
    bind_flags: u32,
    usage: u32,
    cpu_access_flags: u32,
    misc_flags: u32,
}

#[derive(Clone, Debug, Serialize)]
struct MapFrameShaderResourceViewDescFile {
    format: u32,
    view_dimension: u32,
    most_detailed_mip: u32,
    mip_levels: u32,
    first_array_slice: u32,
    array_size: u32,
    num_cubes: u32,
}

#[derive(Clone, Debug, Serialize)]
struct MapFrameTextureSubresourceFile {
    mip_slice: u32,
    array_slice: u32,
    file_offset: usize,
    byte_len: usize,
    row_pitch: u32,
    depth_pitch: u32,
    width: u32,
    height: u32,
}

#[derive(Clone, Debug, Serialize)]
struct MapFrameSamplerBindingFile {
    stage: &'static str,
    slot: u32,
    filter: u32,
    address_u: u32,
    address_v: u32,
    address_w: u32,
    mip_lod_bias: f32,
    max_anisotropy: u32,
    comparison_func: u32,
    border_color: [f32; 4],
    min_lod: f32,
    max_lod: f32,
}

#[derive(Clone, Debug, Serialize)]
struct MapFrameFileEvidence {
    file: String,
    byte_len: usize,
    fnv1a64: String,
    provenance: String,
}

unsafe fn capture_map_frame_bundle_if_active(
    context: *mut ID3D11DeviceContext,
    draw_call: &'static str,
    draw_count: UINT,
    draw_start: UINT,
    base_vertex_location: i32,
    per_instance_count: Option<UINT>,
    instance_count: Option<UINT>,
    start_instance_location: Option<UINT>,
) -> bool {
    if capture_symbol_observed(MAP_FRAME_BUNDLE_SYMBOL) {
        return false;
    }
    if MAP_FRAME_CAPTURE_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        return false;
    }
    let result = unsafe {
        capture_active_map_frame_bundle(
            context,
            draw_call,
            draw_count,
            draw_start,
            base_vertex_location,
            per_instance_count,
            instance_count,
            start_instance_location,
        )
    };
    let captured_draw = result.is_ok();
    match result {
        Ok(sequence_complete) => record_map_frame_bundle_success(sequence_complete),
        Err(error) => record_map_frame_bundle_rejected(&error),
    }
    MAP_FRAME_CAPTURE_IN_PROGRESS.store(false, Ordering::SeqCst);
    captured_draw
}

unsafe fn capture_map_frame_post_draw_output_if_active(context: *mut ID3D11DeviceContext) {
    if context.is_null() || capture_symbol_observed(MAP_FRAME_BUNDLE_SYMBOL) {
        return;
    }
    let Some((output_dir, draw_index, profile)) = RUNTIME.get().and_then(|runtime| {
        runtime.lock().ok().and_then(|runtime| {
            let profile = requested_map_frame_capture_profile_for_runtime(&runtime).ok()?;
            runtime
                .map_frame_draws
                .last()
                .map(|draw| (runtime.output_dir.clone(), draw.draw_index, profile))
        })
    }) else {
        return;
    };
    let (state, blockers) = unsafe {
        capture_map_frame_post_pass_output_merger_state(context, &output_dir, draw_index, profile)
    };
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            if let Some(state) = state {
                if let Some(draw) = runtime
                    .map_frame_draws
                    .iter_mut()
                    .find(|draw| draw.draw_index == draw_index)
                {
                    draw.post_draw_output_merger = Some(state.clone());
                }
                runtime.latest_post_draw_output_merger = Some(state);
            }
            runtime.latest_post_draw_output_blockers = blockers;
        }
    }
}

unsafe fn capture_active_map_frame_bundle(
    context: *mut ID3D11DeviceContext,
    draw_call: &'static str,
    draw_count: UINT,
    draw_start: UINT,
    base_vertex_location: i32,
    per_instance_count: Option<UINT>,
    instance_count: Option<UINT>,
    start_instance_location: Option<UINT>,
) -> Result<bool, String> {
    if context.is_null() {
        return Err("null D3D11 context for map-frame capture".to_string());
    }
    let profile = requested_map_frame_capture_profile()?;
    let (
        shader,
        shader_meta,
        vertex_shader,
        vertex_shader_meta,
        layout_ptr,
        elements,
        output_dir,
        draw_index,
    ) = {
        let runtime = RUNTIME
            .get()
            .and_then(|runtime| runtime.lock().ok())
            .ok_or_else(|| "terrain capture runtime not initialized".to_string())?;
        let (shader, shader_meta, vertex_shader, vertex_shader_meta) = match profile.capture_moment
        {
            TERRAIN_CAPTURE_MOMENT => {
                let shader = runtime
                    .state
                    .pdxmap_pixel_shader()
                    .map(|shader| MapPixelShaderBinding {
                        pixel_shader: shader.pixel_shader,
                    })
                    .ok_or_else(|| {
                        "no active pdxmap pixel shader for map-frame capture".to_string()
                    })?;
                let shader_meta = runtime
                    .state
                    .pdxmap_pixel_shader_metadata(shader.pixel_shader)
                    .cloned()
                    .ok_or_else(|| "active pdxmap shader has no bytecode metadata".to_string())?;
                let vertex_shader = runtime.state.terrain_vertex_shader().ok_or_else(|| {
                    "active pdxmap draw has no tracked terrain vertex shader".to_string()
                })?;
                let vertex_shader_meta = runtime
                    .state
                    .terrain_vertex_shader_metadata(vertex_shader.vertex_shader)
                    .cloned()
                    .ok_or_else(|| {
                        "active terrain vertex shader has no bytecode metadata".to_string()
                    })?;
                (shader, shader_meta, vertex_shader, vertex_shader_meta)
            }
            WATER_CAPTURE_MOMENT => {
                let shader = runtime.state.pdxwater_pixel_shader().ok_or_else(|| {
                    "no active pdxwater pixel shader for water-pass map-frame capture".to_string()
                })?;
                let shader_meta = runtime
                    .state
                    .pdxwater_pixel_shader_metadata(shader.pixel_shader)
                    .cloned()
                    .ok_or_else(|| "active pdxwater shader has no bytecode metadata".to_string())?;
                let vertex_shader = runtime.state.map_vertex_shader().ok_or_else(|| {
                    "active pdxwater draw has no tracked reflected map vertex shader".to_string()
                })?;
                let vertex_shader_meta = runtime
                    .state
                    .map_vertex_shader_metadata(vertex_shader.vertex_shader)
                    .cloned()
                    .ok_or_else(|| {
                        "active water map vertex shader has no bytecode metadata".to_string()
                    })?;
                (shader, shader_meta, vertex_shader, vertex_shader_meta)
            }
            RIVERS_CAPTURE_MOMENT => {
                let shader = runtime.state.river_pixel_shader().ok_or_else(|| {
                    "no active river pixel shader for rivers-pass map-frame capture".to_string()
                })?;
                let shader_meta = runtime
                    .state
                    .river_pixel_shader_metadata(shader.pixel_shader)
                    .cloned()
                    .ok_or_else(|| "active river shader has no bytecode metadata".to_string())?;
                let vertex_shader = runtime.state.map_vertex_shader().ok_or_else(|| {
                    "active river draw has no tracked reflected map vertex shader".to_string()
                })?;
                let vertex_shader_meta = runtime
                    .state
                    .map_vertex_shader_metadata(vertex_shader.vertex_shader)
                    .cloned()
                    .ok_or_else(|| {
                        "active river map vertex shader has no bytecode metadata".to_string()
                    })?;
                (shader, shader_meta, vertex_shader, vertex_shader_meta)
            }
            BORDERS_CAPTURE_MOMENT => {
                let shader = runtime.state.border_pixel_shader().ok_or_else(|| {
                    "no active border pixel shader for borders-pass map-frame capture".to_string()
                })?;
                let shader_meta = runtime
                    .state
                    .border_pixel_shader_metadata(shader.pixel_shader)
                    .cloned()
                    .ok_or_else(|| "active border shader has no bytecode metadata".to_string())?;
                let vertex_shader = runtime.state.map_vertex_shader().ok_or_else(|| {
                    "active border draw has no tracked reflected map vertex shader".to_string()
                })?;
                let vertex_shader_meta = runtime
                    .state
                    .map_vertex_shader_metadata(vertex_shader.vertex_shader)
                    .cloned()
                    .ok_or_else(|| {
                        "active border map vertex shader has no bytecode metadata".to_string()
                    })?;
                (shader, shader_meta, vertex_shader, vertex_shader_meta)
            }
            TREES_CAPTURE_MOMENT => {
                let shader = runtime.state.tree_pixel_shader().ok_or_else(|| {
                    "no active tree pixel shader for trees-pass map-frame capture".to_string()
                })?;
                let shader_meta = runtime
                    .state
                    .tree_pixel_shader_metadata(shader.pixel_shader)
                    .cloned()
                    .ok_or_else(|| "active tree shader has no bytecode metadata".to_string())?;
                let vertex_shader = runtime.state.map_vertex_shader().ok_or_else(|| {
                    "active tree draw has no tracked reflected map vertex shader".to_string()
                })?;
                let vertex_shader_meta = runtime
                    .state
                    .map_vertex_shader_metadata(vertex_shader.vertex_shader)
                    .cloned()
                    .ok_or_else(|| {
                        "active tree map vertex shader has no bytecode metadata".to_string()
                    })?;
                (shader, shader_meta, vertex_shader, vertex_shader_meta)
            }
            other => return Err(format!("unsupported map-frame capture moment {other}")),
        };
        let (layout_ptr, elements) = runtime
            .state
            .current_input_layout()
            .map(|(layout, elements)| (layout, elements.to_vec()))
            .ok_or_else(|| {
                format!(
                    "active {} draw has no tracked input layout",
                    profile.capture_moment
                )
            })?;
        (
            shader,
            shader_meta,
            vertex_shader,
            vertex_shader_meta,
            layout_ptr,
            elements,
            runtime.output_dir.clone(),
            runtime.map_frame_draws.len() as u32,
        )
    };
    if profile.capture_moment == TERRAIN_CAPTURE_MOMENT {
        validate_terrain_input_layout(&elements, u32::MAX).map_err(|error| {
            format!(
                "active map-frame input layout rejected against {TERRAIN_INPUT_LAYOUT_SHADER_SOURCE}: {error:?}"
            )
        })?;
    } else if elements.is_empty() {
        return Err(format!(
            "active {} draw has an empty D3D11 input layout",
            profile.capture_moment
        ));
    }

    let mut blockers = Vec::new();
    let topology = unsafe { active_primitive_topology(context) };
    let mut vertex_buffers = BTreeMap::new();
    let input_slots = elements
        .iter()
        .map(|element| element.input_slot)
        .collect::<BTreeSet<_>>();
    for slot in input_slots {
        match unsafe {
            capture_map_frame_vertex_buffer_slot(context, slot, &output_dir, draw_index, profile)
        } {
            Ok(binding) => {
                vertex_buffers.insert(slot.to_string(), binding);
            }
            Err(error) => blockers.push(error),
        }
    }
    let index_buffer = if draw_call.contains("Indexed") {
        match unsafe { capture_map_frame_index_buffer(context, &output_dir, draw_index, profile) } {
            Ok(binding) => Some(binding),
            Err(error) => {
                blockers.push(error);
                None
            }
        }
    } else {
        None
    };
    let (shader_resources, mut srv_blockers) = unsafe {
        capture_map_frame_ps_srvs(
            context,
            &shader_meta.resource_slots,
            &output_dir,
            draw_index,
            profile,
        )
    };
    blockers.append(&mut srv_blockers);
    let (samplers, mut sampler_blockers) =
        unsafe { capture_map_frame_ps_samplers(context, profile) };
    blockers.append(&mut sampler_blockers);
    let (constant_buffers, mut constant_buffer_blockers) =
        unsafe { capture_map_frame_constant_buffers(context, draw_index, profile) };
    blockers.append(&mut constant_buffer_blockers);
    let (draw_state, mut state_blockers) =
        unsafe { capture_map_frame_draw_state(context, &output_dir, draw_index, profile) };
    blockers.append(&mut state_blockers);

    let shader_file = format!(
        "{}_{}.bin",
        profile.pixel_shader_file_prefix, shader_meta.fnv1a64
    );
    let vertex_shader_file = format!(
        "{}_{}.bin",
        profile.vertex_shader_file_prefix, vertex_shader_meta.fnv1a64
    );
    let draw = MapFrameDrawFile {
        draw_index,
        draw_call,
        draw_count,
        draw_start,
        base_vertex_location,
        per_instance_count,
        instance_count,
        start_instance_location,
        topology,
        shader: MapFrameShaderFile {
            stage: "PS",
            pointer: format!("0x{:X}", shader.pixel_shader),
            bytecode_file: shader_file,
            byte_len: shader_meta.byte_len,
            fnv1a64: shader_meta.fnv1a64.clone(),
            reflection_resources: shader_meta.resource_slots.clone(),
            input_parameters: Vec::new(),
            constant_buffers: shader_meta.constant_buffers.clone(),
            source_evidence: profile.shader_sources.to_vec(),
        },
        vertex_shader: MapFrameShaderFile {
            stage: "VS",
            pointer: format!("0x{:X}", vertex_shader.vertex_shader),
            bytecode_file: vertex_shader_file,
            byte_len: vertex_shader_meta.byte_len,
            fnv1a64: vertex_shader_meta.fnv1a64.clone(),
            reflection_resources: BTreeMap::new(),
            input_parameters: vertex_shader_meta.input_parameters.clone(),
            constant_buffers: vertex_shader_meta.constant_buffers.clone(),
            source_evidence: profile.shader_sources.to_vec(),
        },
        input_layout: MapFrameInputLayoutFile {
            pointer: format!("0x{layout_ptr:X}"),
            elements,
        },
        vertex_buffers,
        index_buffer,
        constant_buffers,
        shader_resources,
        samplers,
        draw_state,
        post_draw_output_merger: None,
    };

    let (draws, mut accumulated_blockers) = {
        let mut runtime = RUNTIME
            .get()
            .and_then(|runtime| runtime.lock().ok())
            .ok_or_else(|| "terrain capture runtime not initialized".to_string())?;
        runtime.map_frame_draws.push(draw.clone());
        runtime.map_frame_blockers.extend(blockers);
        (
            runtime.map_frame_draws.clone(),
            runtime.map_frame_blockers.clone(),
        )
    };
    accumulated_blockers.push(format!(
        "map-frame {} is still active; waiting for next render-target clear frame boundary",
        profile.capture_moment
    ));
    let in_progress_provenance = match profile.capture_moment {
        WATER_CAPTURE_MOMENT => "early_d3d11_active_pdxwater_draw_sequence_capture_in_progress",
        RIVERS_CAPTURE_MOMENT => "early_d3d11_active_river_draw_sequence_capture_in_progress",
        BORDERS_CAPTURE_MOMENT => "early_d3d11_active_border_draw_sequence_capture_in_progress",
        TREES_CAPTURE_MOMENT => "early_d3d11_active_tree_draw_sequence_capture_in_progress",
        _ => "early_d3d11_active_pdxmap_draw_sequence_capture_in_progress",
    };
    write_map_frame_bundle_snapshot(
        &output_dir,
        draws,
        accumulated_blockers,
        in_progress_provenance,
        None,
        profile,
    )?;
    Ok(false)
}

fn finalize_map_frame_bundle_pass(context: *mut ID3D11DeviceContext, reason: &'static str) {
    if capture_symbol_observed(MAP_FRAME_BUNDLE_SYMBOL) {
        return;
    }
    if MAP_FRAME_CAPTURE_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        return;
    }
    let result = write_final_map_frame_bundle_snapshot(context, reason);
    match result {
        Ok(complete_without_blockers) => record_map_frame_bundle_success(complete_without_blockers),
        Err(error) => record_map_frame_bundle_rejected(&error),
    }
    MAP_FRAME_CAPTURE_IN_PROGRESS.store(false, Ordering::SeqCst);
}

fn write_final_map_frame_bundle_snapshot(
    context: *mut ID3D11DeviceContext,
    reason: &'static str,
) -> Result<bool, String> {
    let (output_dir, draws, blockers, profile) = {
        let runtime = RUNTIME
            .get()
            .and_then(|runtime| runtime.lock().ok())
            .ok_or_else(|| "terrain capture runtime not initialized".to_string())?;
        let profile = requested_map_frame_capture_profile_for_runtime(&runtime)?;
        (
            runtime.output_dir.clone(),
            runtime.map_frame_draws.clone(),
            runtime.map_frame_blockers.clone(),
            profile,
        )
    };
    write_map_frame_bundle_snapshot(&output_dir, draws, blockers, reason, Some(context), profile)
}

fn write_map_frame_bundle_snapshot(
    output_dir: &Path,
    draws: Vec<MapFrameDrawFile>,
    mut blockers: Vec<String>,
    provenance: &'static str,
    post_pass_context: Option<*mut ID3D11DeviceContext>,
    profile: MapFrameCaptureProfile,
) -> Result<bool, String> {
    let active_draw = draws
        .last()
        .cloned()
        .ok_or_else(|| "map-frame capture produced no draw records".to_string())?;
    let post_pass_output_merger = if post_pass_context.is_some() {
        let (state, output_merger_blockers) = RUNTIME
            .get()
            .and_then(|runtime| runtime.lock().ok())
            .map(|runtime| {
                (
                    runtime.latest_post_draw_output_merger.clone(),
                    runtime.latest_post_draw_output_blockers.clone(),
                )
            })
            .unwrap_or_default();
        blockers.extend(output_merger_blockers);
        if state.is_none() {
            blockers.push(format!(
                "post-draw output-merger capture did not produce a {} render target payload",
                profile.capture_moment
            ));
        }
        state
    } else {
        None
    };
    let shader_cache_constants = if profile.requires_shader_cache_constants {
        map_frame_shader_cache_constants_for_draw(&active_draw, &mut blockers)
    } else {
        None
    };
    let mut runtime_files = BTreeMap::new();
    for target in CAPTURE_TARGETS {
        let provenance = if target.symbol == TERRAIN_ID_RESOURCE_SYMBOL {
            "runtime_texture_readback_TerrainIDMap".to_string()
        } else {
            format!("runtime_texture_readback_{}", target.symbol)
        };
        if profile.requires_terrain_runtime_files
            && (target.symbol == TERRAIN_ID_RESOURCE_SYMBOL
                || target.symbol == SNOW_MUD_DATA_RESOURCE_SYMBOL
                    && active_draw
                        .shader
                        .reflection_resources
                        .contains_key(SNOW_MUD_DATA_RESOURCE_SYMBOL))
        {
            add_required_file_evidence(
                &mut runtime_files,
                output_dir,
                target.rgba_file,
                &provenance,
                &mut blockers,
            );
        } else {
            add_file_evidence_if_exists(
                &mut runtime_files,
                output_dir,
                target.rgba_file,
                &provenance,
            );
        }
    }
    add_map_frame_shader_resource_file_evidence(
        &mut runtime_files,
        output_dir,
        &draws,
        &mut blockers,
    );
    add_map_frame_pre_draw_output_file_evidence(
        &mut runtime_files,
        output_dir,
        &draws,
        &mut blockers,
    );
    add_map_frame_post_pass_output_merger_file_evidence(
        &mut runtime_files,
        output_dir,
        post_pass_output_merger.as_ref(),
        profile,
        &mut blockers,
    );
    add_map_frame_post_draw_output_file_evidence(
        &mut runtime_files,
        output_dir,
        &draws,
        &mut blockers,
    );
    if profile.requires_terrain_runtime_files {
        add_file_evidence_if_exists(
            &mut runtime_files,
            output_dir,
            TERRAIN_SCENE_CB0_FILE,
            "runtime_constant_buffer_readback_TerrainSceneCB0",
        );
    }
    add_required_file_evidence(
        &mut runtime_files,
        output_dir,
        &active_draw.shader.bytecode_file,
        "create_pixel_shader_bytecode",
        &mut blockers,
    );
    add_required_file_evidence(
        &mut runtime_files,
        output_dir,
        &active_draw.vertex_shader.bytecode_file,
        "create_vertex_shader_bytecode",
        &mut blockers,
    );
    let mut vanilla_sources = BTreeMap::new();
    copy_required_vanilla_sources(
        &mut vanilla_sources,
        output_dir,
        profile.vanilla_sources,
        &mut blockers,
    );
    dedupe_blockers(&mut blockers);
    let frame = MapFrameBundleFile {
        schema: MAP_FRAME_BUNDLE_SCHEMA,
        provenance,
        capture_moment: profile.capture_moment,
        session_manifest: MAP_CAPTURE_SESSION_FILE,
        shader_cache_constants,
        active_draw,
        draws,
        post_pass_output_merger,
        vanilla_sources,
        runtime_files,
        blockers,
    };
    fs::create_dir_all(&output_dir).map_err(|error| error.to_string())?;
    let json = serde_json::to_string_pretty(&frame).map_err(|error| error.to_string())?;
    fs::write(output_dir.join(profile.bundle_file), format!("{json}\n"))
        .map_err(|error| error.to_string())?;
    write_map_capture_session_snapshot(output_dir, &frame, provenance, profile)?;
    Ok(frame.blockers.is_empty())
}

fn dedupe_blockers(blockers: &mut Vec<String>) {
    let mut seen = BTreeSet::new();
    blockers.retain(|blocker| seen.insert(blocker.clone()));
}

fn map_frame_shader_cache_constants_for_draw(
    active_draw: &MapFrameDrawFile,
    blockers: &mut Vec<String>,
) -> Option<MapFrameShaderCacheConstantsFile> {
    let Some(terrain_id_srv_slot) = active_draw
        .shader
        .reflection_resources
        .get(TERRAIN_ID_RESOURCE_SYMBOL)
        .copied()
    else {
        blockers.push(
            "active pdxmap shader reflection did not declare TerrainIDMap for shader-cache constants"
                .to_string(),
        );
        return None;
    };
    let Some(control) = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .and_then(|runtime| runtime.control.clone())
    else {
        blockers.push(
            "terrain capture control from --shader-cache is required to bundle MAP_POW2/FOW_POW2 constants"
                .to_string(),
        );
        return None;
    };
    let signatures = control
        .shader_cache_signatures
        .iter()
        .filter(|signature| signature.terrain_id_srv_slot == terrain_id_srv_slot)
        .collect::<Vec<_>>();
    if signatures.is_empty() {
        blockers.push(format!(
            "shader-cache control has no TerrainIDMap signature for active slot t{terrain_id_srv_slot}"
        ));
        return None;
    }
    let first = signatures[0];
    let first_constants = (
        first.map_size_x,
        first.map_size_y,
        first.map_pow2_x_milli,
        first.map_pow2_y_milli,
        first.fow_pow2_x_milli,
        first.fow_pow2_y_milli,
    );
    if let Some(mismatch) = signatures.iter().find(|signature| {
        (
            signature.map_size_x,
            signature.map_size_y,
            signature.map_pow2_x_milli,
            signature.map_pow2_y_milli,
            signature.fow_pow2_x_milli,
            signature.fow_pow2_y_milli,
        ) != first_constants
    }) {
        blockers.push(format!(
            "shader-cache TerrainIDMap signature {} disagrees with active-slot MAP/FOW constants",
            mismatch.file
        ));
        return None;
    }
    Some(MapFrameShaderCacheConstantsFile {
        map_size_x: first.map_size_x as f32,
        map_size_y: first.map_size_y as f32,
        map_pow2_x: milli_to_float(first.map_pow2_x_milli),
        map_pow2_y: milli_to_float(first.map_pow2_y_milli),
        fow_pow2_x: milli_to_float(first.fow_pow2_x_milli),
        fow_pow2_y: milli_to_float(first.fow_pow2_y_milli),
        terrain_id_srv_slot,
        cache_files: signatures
            .iter()
            .map(|signature| signature.file.clone())
            .collect(),
        provenance: control.provenance.clone(),
        extraction: "terrain_capture_attach_shader_cache_control",
    })
}

fn milli_to_float(value: u32) -> f32 {
    value as f32 / 1000.0
}

fn write_map_capture_session_snapshot(
    output_dir: &Path,
    frame: &MapFrameBundleFile,
    provenance: &'static str,
    profile: MapFrameCaptureProfile,
) -> Result<(), String> {
    let active_status = if frame
        .blockers
        .iter()
        .any(|blocker| blocker.contains("is still active"))
    {
        "capture_in_progress"
    } else if frame.blockers.is_empty() {
        "captured_pending_renderer_validation"
    } else {
        "blocked"
    };
    let (sequence, completed_moments) = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .map(|runtime| {
            (
                runtime.capture_moment_sequence.clone(),
                runtime.completed_map_frame_moments.clone(),
            )
        })
        .unwrap_or_else(|| (vec![profile.capture_moment.to_string()], BTreeSet::new()));
    let mut moments = Vec::new();
    let mut recorded = BTreeSet::new();
    for moment in sequence {
        let moment_profile = map_frame_capture_profile(&moment)?;
        let (status, blockers) = if moment_profile.capture_moment == profile.capture_moment {
            (active_status, frame.blockers.clone())
        } else if completed_moments.contains(moment_profile.capture_moment) {
            ("captured_pending_renderer_validation", Vec::new())
        } else {
            (
                "deferred_until_prior_moment_validated",
                vec![format!(
                    "{} has not reached its active capture moment in this session",
                    moment_profile.capture_moment
                )],
            )
        };
        moments.push(MapCaptureSessionMomentFile {
            name: moment_profile.capture_moment,
            status,
            bundle_file: moment_profile.bundle_file,
            required_payload: MAP_CAPTURE_REQUIRED_PAYLOAD.to_vec(),
            blockers,
        });
        recorded.insert(moment_profile.capture_moment);
    }
    let session = MapCaptureSessionFile {
        schema: MAP_CAPTURE_SESSION_SCHEMA,
        provenance,
        required_first_moment: TERRAIN_CAPTURE_MOMENT,
        moments,
        deferred_moments: DEFERRED_MAP_CAPTURE_MOMENTS
            .iter()
            .copied()
            .filter(|name| !recorded.contains(name))
            .map(|name| MapCaptureDeferredMomentFile {
                name,
                status: "deferred_until_terrain_active_pass_validated",
                required_after: TERRAIN_CAPTURE_MOMENT,
                blocker: "terrain bundle capture and renderer validation must complete before this layer is captured",
            })
            .collect(),
    };
    let json = serde_json::to_string_pretty(&session).map_err(|error| error.to_string())?;
    fs::write(
        output_dir.join(MAP_CAPTURE_SESSION_FILE),
        format!("{json}\n"),
    )
    .map_err(|error| error.to_string())
}

fn write_blocked_map_capture_session(
    output_dir: &Path,
    blockers: Vec<String>,
    provenance: &'static str,
    profile: MapFrameCaptureProfile,
) -> Result<(), String> {
    let (sequence, completed_moments) = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .map(|runtime| {
            (
                runtime.capture_moment_sequence.clone(),
                runtime.completed_map_frame_moments.clone(),
            )
        })
        .unwrap_or_else(|| {
            if profile.capture_moment == TERRAIN_CAPTURE_MOMENT {
                (vec![TERRAIN_CAPTURE_MOMENT.to_string()], BTreeSet::new())
            } else {
                (
                    vec![
                        TERRAIN_CAPTURE_MOMENT.to_string(),
                        profile.capture_moment.to_string(),
                    ],
                    BTreeSet::from([TERRAIN_CAPTURE_MOMENT.to_string()]),
                )
            }
        });
    let mut moments = Vec::new();
    let mut recorded = BTreeSet::new();
    for moment in sequence {
        let moment_profile = map_frame_capture_profile(&moment)?;
        let (status, blockers) = if moment_profile.capture_moment == profile.capture_moment {
            ("blocked", blockers.clone())
        } else if completed_moments.contains(moment_profile.capture_moment) {
            ("captured_pending_renderer_validation", Vec::new())
        } else {
            (
                "deferred_until_prior_moment_validated",
                vec![format!(
                    "{} has not reached its active capture moment in this session",
                    moment_profile.capture_moment
                )],
            )
        };
        moments.push(MapCaptureSessionMomentFile {
            name: moment_profile.capture_moment,
            status,
            bundle_file: moment_profile.bundle_file,
            required_payload: MAP_CAPTURE_REQUIRED_PAYLOAD.to_vec(),
            blockers,
        });
        recorded.insert(moment_profile.capture_moment);
    }
    let session = MapCaptureSessionFile {
        schema: MAP_CAPTURE_SESSION_SCHEMA,
        provenance,
        required_first_moment: TERRAIN_CAPTURE_MOMENT,
        moments,
        deferred_moments: DEFERRED_MAP_CAPTURE_MOMENTS
            .iter()
            .copied()
            .filter(|name| !recorded.contains(name))
            .map(|name| MapCaptureDeferredMomentFile {
                name,
                status: "deferred_until_terrain_active_pass_validated",
                required_after: TERRAIN_CAPTURE_MOMENT,
                blocker: "terrain bundle capture and renderer validation must complete before this layer is captured",
            })
            .collect(),
    };
    fs::create_dir_all(output_dir).map_err(|error| error.to_string())?;
    let json = serde_json::to_string_pretty(&session).map_err(|error| error.to_string())?;
    fs::write(
        output_dir.join(MAP_CAPTURE_SESSION_FILE),
        format!("{json}\n"),
    )
    .map_err(|error| error.to_string())
}

unsafe fn active_primitive_topology(context: *mut ID3D11DeviceContext) -> Option<u32> {
    if context.is_null() {
        return None;
    }
    let mut topology = 0u32;
    unsafe {
        (*context).IAGetPrimitiveTopology(&mut topology);
    }
    Some(topology)
}

unsafe fn capture_active_terrain_input_layout(
    context: *mut ID3D11DeviceContext,
    draw_call: &'static str,
    draw_count: UINT,
    draw_start: UINT,
    base_vertex_location: i32,
    shader: TerrainShaderBinding,
) -> Result<CapturedInputLayoutMeta, String> {
    if context.is_null() {
        return Err("null D3D11 context for terrain input layout capture".to_string());
    }
    let mut layout: *mut ID3D11InputLayout = ptr::null_mut();
    unsafe {
        (*context).IAGetInputLayout(&mut layout);
    }
    if layout.is_null() {
        return Err("active pdxmap draw has no IA input layout bound".to_string());
    }
    let layout_ptr = layout as usize;
    unsafe {
        release_unknown(layout as *mut WinVoid);
    }
    let elements = {
        let runtime = RUNTIME
            .get()
            .and_then(|runtime| runtime.lock().ok())
            .ok_or_else(|| "terrain capture runtime not initialized".to_string())?;
        runtime
            .state
            .input_layout(layout_ptr)
            .map(|elements| elements.to_vec())
            .ok_or_else(|| {
                format!(
                    "active terrain input layout 0x{layout_ptr:X} was not created after hook install; D3D11 does not expose its original D3D11_INPUT_ELEMENT_DESC array"
                )
            })?
    };
    let (vertex_stride, vertex_offset) = unsafe { active_vertex_buffer_stride_offset(context)? };
    validate_terrain_input_layout(&elements, vertex_stride).map_err(|error| {
        format!(
            "active terrain input layout rejected against {TERRAIN_INPUT_LAYOUT_SHADER_SOURCE}: {error:?}"
        )
    })?;
    let output_dir = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .map(|runtime| runtime.output_dir.clone())
        .ok_or_else(|| "terrain capture runtime not initialized".to_string())?;
    write_input_layout_capture_file(
        &output_dir,
        &elements,
        vertex_stride,
        vertex_offset,
        draw_call,
        draw_count,
        draw_start,
        base_vertex_location,
        shader,
    )
}

unsafe fn active_vertex_buffer_stride_offset(
    context: *mut ID3D11DeviceContext,
) -> Result<(u32, u32), String> {
    let mut buffer: *mut ID3D11Buffer = ptr::null_mut();
    let mut stride = 0u32;
    let mut offset = 0u32;
    unsafe {
        (*context).IAGetVertexBuffers(0, 1, &mut buffer, &mut stride, &mut offset);
    }
    if !buffer.is_null() {
        unsafe {
            release_unknown(buffer as *mut WinVoid);
        }
    }
    if buffer.is_null() {
        return Err("active pdxmap draw has no IA vertex buffer bound at slot 0".to_string());
    }
    if stride == 0 {
        return Err("active pdxmap draw reported zero vertex stride at slot 0".to_string());
    }
    Ok((stride, offset))
}

unsafe fn capture_map_frame_vertex_buffer_slot(
    context: *mut ID3D11DeviceContext,
    slot: u32,
    output_dir: &Path,
    draw_index: u32,
    profile: MapFrameCaptureProfile,
) -> Result<MapFrameBufferBindingFile, String> {
    let mut buffer: *mut ID3D11Buffer = ptr::null_mut();
    let mut stride = 0u32;
    let mut offset = 0u32;
    unsafe {
        (*context).IAGetVertexBuffers(slot, 1, &mut buffer, &mut stride, &mut offset);
    }
    if buffer.is_null() {
        return Err(format!(
            "active {} input layout references IA vertex buffer slot {slot}, but no buffer is bound",
            profile.capture_moment
        ));
    }
    let result = unsafe {
        let (source, bytes) = readback_d3d11_buffer_bytes(context, buffer)?;
        if source.bind_flags & D3D11_BIND_VERTEX_BUFFER == 0 {
            return Err(format!(
                "IA vertex buffer slot {slot} missing D3D11_BIND_VERTEX_BUFFER: BindFlags {}",
                source.bind_flags
            ));
        }
        if stride == 0 {
            return Err(format!("IA vertex buffer slot {slot} has zero stride"));
        }
        let file = format!(
            "{}{draw_index:03}_ia_vertex_buffer_slot{slot}.bin",
            profile.file_prefix
        );
        write_map_frame_bytes(output_dir, &file, &bytes)?;
        Ok(MapFrameBufferBindingFile {
            stage: "IA",
            slot,
            declared_name: None,
            file: Some(file),
            byte_len: bytes.len(),
            fnv1a64: Some(fnv1a64_bytes(&bytes)),
            stride: Some(stride),
            offset: Some(offset),
            dxgi_format: None,
            bind_flags: source.bind_flags,
            usage: source.usage,
            cpu_access_flags: source.cpu_access_flags,
        })
    };
    unsafe {
        release_unknown(buffer as *mut WinVoid);
    }
    result
}

unsafe fn capture_map_frame_index_buffer(
    context: *mut ID3D11DeviceContext,
    output_dir: &Path,
    draw_index: u32,
    profile: MapFrameCaptureProfile,
) -> Result<MapFrameIndexBufferBindingFile, String> {
    let mut buffer: *mut ID3D11Buffer = ptr::null_mut();
    let mut format = 0u32;
    let mut offset = 0u32;
    unsafe {
        (*context).IAGetIndexBuffer(&mut buffer, &mut format, &mut offset);
    }
    if buffer.is_null() {
        return Err("active indexed pdxmap draw has no IA index buffer bound".to_string());
    }
    let result = unsafe {
        let (source, bytes) = readback_d3d11_buffer_bytes(context, buffer)?;
        if source.bind_flags & 0x2 == 0 {
            return Err(format!(
                "IA index buffer missing D3D11_BIND_INDEX_BUFFER: BindFlags {}",
                source.bind_flags
            ));
        }
        let file = format!("{}{draw_index:03}_ia_index_buffer.bin", profile.file_prefix);
        write_map_frame_bytes(output_dir, &file, &bytes)?;
        Ok(MapFrameIndexBufferBindingFile {
            file: Some(file),
            byte_len: bytes.len(),
            fnv1a64: Some(fnv1a64_bytes(&bytes)),
            dxgi_format: format,
            offset,
            bind_flags: source.bind_flags,
            usage: source.usage,
            cpu_access_flags: source.cpu_access_flags,
        })
    };
    unsafe {
        release_unknown(buffer as *mut WinVoid);
    }
    result
}

unsafe fn readback_d3d11_buffer_bytes(
    context: *mut ID3D11DeviceContext,
    buffer: *mut ID3D11Buffer,
) -> Result<(BufferDesc, Vec<u8>), String> {
    if context.is_null() || buffer.is_null() {
        return Err("null D3D11 context or buffer for map-frame readback".to_string());
    }
    let mut source_desc_native: D3D11_BUFFER_DESC = unsafe { mem::zeroed() };
    unsafe {
        (*buffer).GetDesc(&mut source_desc_native);
    }
    let source = buffer_desc_from_native(&source_desc_native);
    if source.byte_width == 0 {
        return Err("map-frame buffer has zero ByteWidth".to_string());
    }

    let mut device: *mut ID3D11Device = ptr::null_mut();
    unsafe {
        (*buffer).GetDevice(&mut device);
    }
    if device.is_null() {
        return Err("map-frame buffer did not expose an ID3D11Device".to_string());
    }

    let staging_desc = D3D11_BUFFER_DESC {
        ByteWidth: source_desc_native.ByteWidth,
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let mut staging: *mut ID3D11Buffer = ptr::null_mut();
    let create_hr = unsafe { (*device).CreateBuffer(&staging_desc, ptr::null(), &mut staging) };
    unsafe {
        release_unknown(device as *mut WinVoid);
    }
    if FAILED(create_hr) || staging.is_null() {
        return Err(format!(
            "CreateBuffer map-frame staging readback failed: HRESULT 0x{:08X}",
            create_hr as u32
        ));
    }

    unsafe {
        (*context).CopyResource(
            staging as *mut ID3D11Resource,
            buffer as *mut ID3D11Resource,
        );
    }
    let mut mapped: D3D11_MAPPED_SUBRESOURCE = unsafe { mem::zeroed() };
    let map_hr = unsafe {
        (*context).Map(
            staging as *mut ID3D11Resource,
            0,
            D3D11_MAP_READ,
            0,
            &mut mapped,
        )
    };
    if FAILED(map_hr) || mapped.pData.is_null() {
        unsafe {
            release_unknown(staging as *mut WinVoid);
        }
        return Err(format!(
            "Map staging map-frame buffer failed: HRESULT 0x{:08X}",
            map_hr as u32
        ));
    }
    let bytes =
        unsafe { slice::from_raw_parts(mapped.pData.cast::<u8>(), source.byte_width as usize) }
            .to_vec();
    unsafe {
        (*context).Unmap(staging as *mut ID3D11Resource, 0);
        release_unknown(staging as *mut WinVoid);
    }
    Ok((source, bytes))
}

fn write_map_frame_bytes(output_dir: &Path, file: &str, bytes: &[u8]) -> Result<(), String> {
    fs::create_dir_all(output_dir).map_err(|error| error.to_string())?;
    fs::write(output_dir.join(file), bytes).map_err(|error| error.to_string())
}

fn write_input_layout_capture_file(
    output_dir: &Path,
    elements: &[TerrainInputElementDesc],
    vertex_stride: u32,
    vertex_offset: u32,
    draw_call: &'static str,
    draw_count: UINT,
    draw_start: UINT,
    base_vertex_location: i32,
    shader: TerrainShaderBinding,
) -> Result<CapturedInputLayoutMeta, String> {
    fs::create_dir_all(output_dir).map_err(|error| error.to_string())?;
    let meta = CapturedInputLayoutMeta {
        elements: elements.to_vec(),
        vertex_stride,
        vertex_offset,
        draw_call,
        draw_count,
        draw_start,
        base_vertex_location,
        terrain_id_srv_slot: shader.terrain_id_srv_slot,
    };
    let meta_json = format!(
        concat!(
            "{{\n",
            "  \"schema\": \"battle-plan-terrain-input-layout-capture-v1\",\n",
            "  \"symbol\": \"{}\",\n",
            "  \"element_count\": {},\n",
            "  \"elements\": {},\n",
            "  \"vertex_stride\": {},\n",
            "  \"vertex_offset\": {},\n",
            "  \"shader_input_source\": \"{}\",\n",
            "  \"draw_call\": \"{}\",\n",
            "  \"draw_count\": {},\n",
            "  \"draw_start\": {},\n",
            "  \"base_vertex_location\": {},\n",
            "  \"terrain_id_srv_slot\": {},\n",
            "  \"source\": \"create_input_layout_descriptor_bound_during_active_pdxmap_draw\"\n",
            "}}\n"
        ),
        TERRAIN_INPUT_LAYOUT_SYMBOL,
        meta.elements.len(),
        input_elements_json(&meta.elements),
        meta.vertex_stride,
        meta.vertex_offset,
        json_escape(TERRAIN_INPUT_LAYOUT_SHADER_SOURCE),
        meta.draw_call,
        meta.draw_count,
        meta.draw_start,
        meta.base_vertex_location,
        meta.terrain_id_srv_slot
    );
    fs::write(output_dir.join(TERRAIN_INPUT_LAYOUT_META_FILE), meta_json)
        .map_err(|error| error.to_string())?;
    Ok(meta)
}

fn input_elements_json(elements: &[TerrainInputElementDesc]) -> String {
    let mut out = String::from("[");
    for (index, element) in elements.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            concat!(
                "{{",
                "\"semantic_name\":\"{}\",",
                "\"semantic_index\":{},",
                "\"format\":{},",
                "\"input_slot\":{},",
                "\"aligned_byte_offset\":{},",
                "\"input_slot_class\":{},",
                "\"instance_data_step_rate\":{}",
                "}}"
            ),
            json_escape(&element.semantic_name),
            element.semantic_index,
            element.format,
            element.input_slot,
            element.aligned_byte_offset,
            element.input_slot_class,
            element.instance_data_step_rate
        ));
    }
    out.push(']');
    out
}

fn record_input_layout_success(meta: &CapturedInputLayoutMeta) {
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            runtime
                .captured_symbols
                .insert(TERRAIN_INPUT_LAYOUT_SYMBOL.to_string());
            write_event(
                &runtime.output_dir,
                &format!(
                    "{{\"type\":\"terrain_input_layout_captured\",\"symbol\":\"{}\",\"file\":\"{}\",\"element_count\":{},\"vertex_stride\":{},\"vertex_offset\":{},\"draw_call\":\"{}\",\"draw_count\":{},\"draw_start\":{},\"base_vertex_location\":{},\"terrain_id_srv_slot\":{},\"shader_input_source\":\"{}\"}}",
                    TERRAIN_INPUT_LAYOUT_SYMBOL,
                    TERRAIN_INPUT_LAYOUT_META_FILE,
                    meta.elements.len(),
                    meta.vertex_stride,
                    meta.vertex_offset,
                    meta.draw_call,
                    meta.draw_count,
                    meta.draw_start,
                    meta.base_vertex_location,
                    meta.terrain_id_srv_slot,
                    json_escape(TERRAIN_INPUT_LAYOUT_SHADER_SOURCE)
                ),
            );
            if refresh_session_capture_progress(&mut runtime) {
                CAPTURED.store(true, Ordering::SeqCst);
            }
        }
    }
}

fn record_input_layout_rejected_once(
    draw_call: &'static str,
    shader: TerrainShaderBinding,
    error: &str,
) {
    if INPUT_LAYOUT_CAPTURE_REJECTION_RECORDED.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            runtime.last_error = Some(error.to_string());
            write_event(
                &runtime.output_dir,
                &format!(
                    "{{\"type\":\"terrain_input_layout_candidate_rejected\",\"draw_call\":\"{}\",\"terrain_id_srv_slot\":{},\"message\":\"{}\"}}",
                    draw_call,
                    shader.terrain_id_srv_slot,
                    json_escape(error)
                ),
            );
        }
    }
}

unsafe fn capture_terrain_vertex_buffer_if_active(
    context: *mut ID3D11DeviceContext,
    draw_call: &'static str,
    draw_count: UINT,
    draw_start: UINT,
    base_vertex_location: i32,
) {
    if capture_symbol_observed(TERRAIN_VERTEX_BUFFER_SYMBOL) {
        return;
    }
    let Some(shader) = active_pdxmap_shader_binding() else {
        return;
    };
    if VERTEX_CAPTURE_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        return;
    }
    let result = unsafe {
        capture_active_terrain_vertex_buffer(
            context,
            draw_call,
            draw_count,
            draw_start,
            base_vertex_location,
            shader,
        )
    };
    match result {
        Ok(meta) => record_vertex_buffer_success(&meta),
        Err(error) => record_vertex_buffer_rejected_once(draw_call, shader, &error),
    }
    VERTEX_CAPTURE_IN_PROGRESS.store(false, Ordering::SeqCst);
}

fn active_pdxmap_shader_binding() -> Option<TerrainShaderBinding> {
    let runtime = RUNTIME.get().and_then(|runtime| runtime.lock().ok())?;
    runtime.state.pdxmap_pixel_shader().or_else(|| {
        runtime
            .terrain_id_srv_slot_observed
            .map(|terrain_id_srv_slot| TerrainShaderBinding {
                pixel_shader: 0,
                terrain_id_srv_slot,
            })
    })
}

#[derive(Debug)]
struct CapturedVertexBufferMeta {
    source_byte_width: u32,
    captured_byte_len: usize,
    fnv1a64: String,
    stride: u32,
    offset: u32,
    draw_call: &'static str,
    draw_count: u32,
    draw_start: u32,
    base_vertex_location: i32,
    terrain_id_srv_slot: u32,
}

unsafe fn capture_active_terrain_vertex_buffer(
    context: *mut ID3D11DeviceContext,
    draw_call: &'static str,
    draw_count: UINT,
    draw_start: UINT,
    base_vertex_location: i32,
    shader: TerrainShaderBinding,
) -> Result<CapturedVertexBufferMeta, String> {
    if context.is_null() {
        return Err("null D3D11 context for terrain vertex buffer capture".to_string());
    }
    let mut buffer: *mut ID3D11Buffer = ptr::null_mut();
    let mut stride = 0u32;
    let mut offset = 0u32;
    unsafe {
        (*context).IAGetVertexBuffers(0, 1, &mut buffer, &mut stride, &mut offset);
    }
    if buffer.is_null() {
        return Err("active pdxmap draw has no IA vertex buffer bound at slot 0".to_string());
    }
    let result = unsafe {
        capture_terrain_vertex_buffer(
            context,
            buffer,
            stride,
            offset,
            draw_call,
            draw_count,
            draw_start,
            base_vertex_location,
            shader,
        )
    };
    unsafe {
        release_unknown(buffer as *mut WinVoid);
    }
    result
}

unsafe fn capture_terrain_vertex_buffer(
    context: *mut ID3D11DeviceContext,
    buffer: *mut ID3D11Buffer,
    stride: u32,
    offset: u32,
    draw_call: &'static str,
    draw_count: UINT,
    draw_start: UINT,
    base_vertex_location: i32,
    shader: TerrainShaderBinding,
) -> Result<CapturedVertexBufferMeta, String> {
    let mut source_desc: D3D11_BUFFER_DESC = unsafe { mem::zeroed() };
    unsafe {
        (*buffer).GetDesc(&mut source_desc);
    }
    let source = buffer_desc_from_native(&source_desc);
    let plan = terrain_vertex_buffer_readback_plan(&source, stride, offset)
        .map_err(|error| format!("terrain vertex buffer rejected: {error:?}"))?;

    let mut device: *mut ID3D11Device = ptr::null_mut();
    unsafe {
        (*buffer).GetDevice(&mut device);
    }
    if device.is_null() {
        return Err("terrain vertex buffer did not expose an ID3D11Device".to_string());
    }

    let staging_desc = buffer_desc_to_native(&plan.staging_desc);
    let mut staging: *mut ID3D11Buffer = ptr::null_mut();
    let create_hr = unsafe { (*device).CreateBuffer(&staging_desc, ptr::null(), &mut staging) };
    unsafe {
        release_unknown(device as *mut WinVoid);
    }
    if FAILED(create_hr) || staging.is_null() {
        return Err(format!(
            "CreateBuffer terrain vertex staging readback failed: HRESULT 0x{:08X}",
            create_hr as u32
        ));
    }

    unsafe {
        (*context).CopyResource(
            staging as *mut ID3D11Resource,
            buffer as *mut ID3D11Resource,
        );
    }

    let mut mapped: D3D11_MAPPED_SUBRESOURCE = unsafe { mem::zeroed() };
    let map_hr = unsafe {
        (*context).Map(
            staging as *mut ID3D11Resource,
            0,
            D3D11_MAP_READ,
            0,
            &mut mapped,
        )
    };
    if FAILED(map_hr) || mapped.pData.is_null() {
        unsafe {
            release_unknown(staging as *mut WinVoid);
        }
        return Err(format!(
            "Map staging terrain vertex buffer failed: HRESULT 0x{:08X}",
            map_hr as u32
        ));
    }

    let bytes = unsafe {
        slice::from_raw_parts(
            mapped.pData.cast::<u8>(),
            plan.captured_byte_len.min(source.byte_width as usize),
        )
    }
    .to_vec();
    unsafe {
        (*context).Unmap(staging as *mut ID3D11Resource, 0);
        release_unknown(staging as *mut WinVoid);
    }

    let output_dir = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .map(|runtime| runtime.output_dir.clone())
        .ok_or_else(|| "terrain capture runtime not initialized".to_string())?;
    write_vertex_buffer_capture_files(
        &output_dir,
        &bytes,
        &source,
        stride,
        offset,
        draw_call,
        draw_count,
        draw_start,
        base_vertex_location,
        shader,
    )
}

fn write_vertex_buffer_capture_files(
    output_dir: &Path,
    bytes: &[u8],
    source: &BufferDesc,
    stride: u32,
    offset: u32,
    draw_call: &'static str,
    draw_count: UINT,
    draw_start: UINT,
    base_vertex_location: i32,
    shader: TerrainShaderBinding,
) -> Result<CapturedVertexBufferMeta, String> {
    fs::create_dir_all(output_dir).map_err(|error| error.to_string())?;
    fs::write(output_dir.join(TERRAIN_VERTEX_BUFFER_SAMPLE_FILE), bytes)
        .map_err(|error| error.to_string())?;
    let meta = CapturedVertexBufferMeta {
        source_byte_width: source.byte_width,
        captured_byte_len: bytes.len(),
        fnv1a64: fnv1a64_bytes(bytes),
        stride,
        offset,
        draw_call,
        draw_count,
        draw_start,
        base_vertex_location,
        terrain_id_srv_slot: shader.terrain_id_srv_slot,
    };
    let meta_json = format!(
        concat!(
            "{{\n",
            "  \"schema\": \"battle-plan-terrain-vertex-buffer-capture-v1\",\n",
            "  \"symbol\": \"{}\",\n",
            "  \"file\": \"{}\",\n",
            "  \"source_byte_width\": {},\n",
            "  \"captured_byte_len\": {},\n",
            "  \"fnv1a64\": \"{}\",\n",
            "  \"stride\": {},\n",
            "  \"offset\": {},\n",
            "  \"minimum_stride_source\": \"{}\",\n",
            "  \"draw_call\": \"{}\",\n",
            "  \"draw_count\": {},\n",
            "  \"draw_start\": {},\n",
            "  \"base_vertex_location\": {},\n",
            "  \"terrain_id_srv_slot\": {},\n",
            "  \"source\": \"ia_get_vertex_buffers_slot0_during_active_pdxmap_draw\"\n",
            "}}\n"
        ),
        TERRAIN_VERTEX_BUFFER_SYMBOL,
        TERRAIN_VERTEX_BUFFER_SAMPLE_FILE,
        meta.source_byte_width,
        meta.captured_byte_len,
        meta.fnv1a64,
        meta.stride,
        meta.offset,
        json_escape(TERRAIN_INPUT_LAYOUT_SHADER_SOURCE),
        meta.draw_call,
        meta.draw_count,
        meta.draw_start,
        meta.base_vertex_location,
        meta.terrain_id_srv_slot
    );
    fs::write(output_dir.join(TERRAIN_VERTEX_BUFFER_META_FILE), meta_json)
        .map_err(|error| error.to_string())?;
    Ok(meta)
}

fn record_vertex_buffer_success(meta: &CapturedVertexBufferMeta) {
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            runtime
                .captured_symbols
                .insert(TERRAIN_VERTEX_BUFFER_SYMBOL.to_string());
            write_event(
                &runtime.output_dir,
                &format!(
                    "{{\"type\":\"terrain_vertex_buffer_captured\",\"symbol\":\"{}\",\"file\":\"{}\",\"source_byte_width\":{},\"captured_byte_len\":{},\"fnv1a64\":\"{}\",\"stride\":{},\"offset\":{},\"draw_call\":\"{}\",\"draw_count\":{},\"draw_start\":{},\"base_vertex_location\":{},\"terrain_id_srv_slot\":{}}}",
                    TERRAIN_VERTEX_BUFFER_SYMBOL,
                    TERRAIN_VERTEX_BUFFER_SAMPLE_FILE,
                    meta.source_byte_width,
                    meta.captured_byte_len,
                    meta.fnv1a64,
                    meta.stride,
                    meta.offset,
                    meta.draw_call,
                    meta.draw_count,
                    meta.draw_start,
                    meta.base_vertex_location,
                    meta.terrain_id_srv_slot
                ),
            );
            if refresh_session_capture_progress(&mut runtime) {
                CAPTURED.store(true, Ordering::SeqCst);
            }
        }
    }
}

fn record_vertex_buffer_rejected_once(
    draw_call: &'static str,
    shader: TerrainShaderBinding,
    error: &str,
) {
    if VERTEX_CAPTURE_REJECTION_RECORDED.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            runtime.last_error = Some(error.to_string());
            write_event(
                &runtime.output_dir,
                &format!(
                    "{{\"type\":\"terrain_vertex_buffer_candidate_rejected\",\"draw_call\":\"{}\",\"terrain_id_srv_slot\":{},\"message\":\"{}\"}}",
                    draw_call,
                    shader.terrain_id_srv_slot,
                    json_escape(error)
                ),
            );
        }
    }
}

fn record_map_frame_bundle_success(sequence_complete: bool) {
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            let profile = requested_map_frame_capture_profile_for_runtime(&runtime)
                .unwrap_or(TERRAIN_MAP_FRAME_CAPTURE_PROFILE);
            if sequence_complete {
                runtime
                    .completed_map_frame_moments
                    .insert(profile.capture_moment.to_string());
            }
            write_event(
                &runtime.output_dir,
                &format!(
                    "{{\"type\":\"map_frame_bundle_captured\",\"symbol\":\"{}\",\"file\":\"{}\",\"sequence_complete\":{},\"draw_count\":{}}}",
                    MAP_FRAME_BUNDLE_SYMBOL,
                    profile.bundle_file,
                    sequence_complete,
                    runtime.map_frame_draws.len()
                ),
            );
            if refresh_session_capture_progress(&mut runtime) {
                CAPTURED.store(true, Ordering::SeqCst);
            }
        }
    }
}

fn record_map_frame_bundle_rejected(error: &str) {
    let Some((output_dir, blockers, profile, blocked_provenance)) = RUNTIME.get().and_then(|runtime| {
        runtime.lock().ok().and_then(|mut runtime| {
            runtime.last_error = Some(error.to_string());
            if !runtime.map_frame_rejections.insert(error.to_string()) {
                return None;
            }
            let output_dir = runtime.output_dir.clone();
            let blockers = runtime
                .map_frame_rejections
                .iter()
                .cloned()
                .collect::<Vec<_>>();
            let profile = requested_map_frame_capture_profile_for_runtime(&runtime)
                .unwrap_or(TERRAIN_MAP_FRAME_CAPTURE_PROFILE);
            write_event(
                &runtime.output_dir,
                &format!(
                    "{{\"type\":\"map_frame_bundle_rejected\",\"symbol\":\"{}\",\"message\":\"{}\"}}",
                    MAP_FRAME_BUNDLE_SYMBOL,
                    json_escape(error)
                ),
            );
            let blocked_provenance = match profile.capture_moment {
                WATER_CAPTURE_MOMENT => "blocked_before_water_pass_bundle",
                RIVERS_CAPTURE_MOMENT => "blocked_before_rivers_pass_bundle",
                BORDERS_CAPTURE_MOMENT => "blocked_before_borders_pass_bundle",
                TREES_CAPTURE_MOMENT => "blocked_before_trees_pass_bundle",
                _ => "blocked_before_terrain_active_pass_bundle",
            };
            Some((output_dir, blockers, profile, blocked_provenance))
        })
    }) else {
        return;
    };
    if let Err(write_error) =
        write_blocked_map_capture_session(&output_dir, blockers, blocked_provenance, profile)
    {
        write_event(
            &output_dir,
            &format!(
                "{{\"type\":\"map_capture_session_write_failed\",\"file\":\"{}\",\"message\":\"{}\"}}",
                MAP_CAPTURE_SESSION_FILE,
                json_escape(&write_error)
            ),
        );
    }
}

unsafe fn capture_paired_runtime_resources(context: *mut ID3D11DeviceContext, terrain_slot: u32) {
    if context.is_null() {
        return;
    }
    let pairs = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .and_then(|runtime| runtime.control.clone())
        .map(|control| control.paired_resource_slots)
        .unwrap_or_default();
    for pair in pairs {
        if pair.terrain_id_srv_slot != terrain_slot {
            continue;
        }
        let Some(target) = capture_target_by_symbol(&pair.symbol) else {
            continue;
        };
        if capture_symbol_observed(target.symbol) {
            continue;
        }
        let mut srv: *mut ID3D11ShaderResourceView = ptr::null_mut();
        unsafe {
            (*context).PSGetShaderResources(pair.shader_resource_slot, 1, &mut srv);
        }
        if srv.is_null() {
            record_candidate_rejected(
                target,
                pair.shader_resource_slot,
                "ps_get_shader_resources_paired_terrain_slot",
                "paired shader resource slot is not currently bound",
            );
            continue;
        }
        let result = unsafe {
            capture_runtime_texture(
                context,
                target,
                srv,
                pair.shader_resource_slot,
                "ps_get_shader_resources_paired_terrain_slot",
            )
        };
        unsafe {
            release_unknown(srv as *mut WinVoid);
        }
        match result {
            Ok(meta) => record_capture_success(target, &meta),
            Err(error) => record_candidate_rejected(
                target,
                pair.shader_resource_slot,
                "ps_get_shader_resources_paired_terrain_slot",
                &error,
            ),
        }
    }
}

unsafe fn capture_scene_constant_buffer(context: *mut ID3D11DeviceContext, slot: u32) {
    if context.is_null() || capture_symbol_observed(TERRAIN_SCENE_CB0_SYMBOL) {
        return;
    }
    let mut buffer: *mut ID3D11Buffer = ptr::null_mut();
    unsafe {
        (*context).PSGetConstantBuffers(slot, 1, &mut buffer);
    }
    if buffer.is_null() {
        record_constant_buffer_rejected(slot, "PS constant buffer slot is not currently bound");
        return;
    }
    let result = unsafe { readback_constant_buffer(context, buffer, slot) };
    unsafe {
        release_unknown(buffer as *mut WinVoid);
    }
    match result {
        Ok(meta) => record_constant_buffer_success(&meta),
        Err(error) => record_constant_buffer_rejected(slot, &error),
    }
}

#[derive(Debug)]
struct CapturedConstantBufferMeta {
    byte_len: usize,
    fnv1a64: String,
    slot: u32,
}

unsafe fn readback_constant_buffer(
    context: *mut ID3D11DeviceContext,
    buffer: *mut ID3D11Buffer,
    slot: u32,
) -> Result<CapturedConstantBufferMeta, String> {
    if context.is_null() || buffer.is_null() {
        return Err("null D3D11 context or scene constant buffer".to_string());
    }
    let mut source_desc: D3D11_BUFFER_DESC = unsafe { mem::zeroed() };
    unsafe {
        (*buffer).GetDesc(&mut source_desc);
    }
    validate_scene_constant_buffer_desc(&source_desc)?;

    let mut device: *mut ID3D11Device = ptr::null_mut();
    unsafe {
        (*buffer).GetDevice(&mut device);
    }
    if device.is_null() {
        return Err("scene constant buffer did not expose an ID3D11Device".to_string());
    }

    let mut staging_desc = source_desc;
    staging_desc.Usage = D3D11_USAGE_STAGING;
    staging_desc.BindFlags = 0;
    staging_desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
    staging_desc.MiscFlags = 0;
    staging_desc.StructureByteStride = 0;

    let mut staging: *mut ID3D11Buffer = ptr::null_mut();
    let create_hr = unsafe { (*device).CreateBuffer(&staging_desc, ptr::null(), &mut staging) };
    unsafe {
        release_unknown(device as *mut WinVoid);
    }
    if FAILED(create_hr) || staging.is_null() {
        return Err(format!(
            "CreateBuffer scene constant staging readback failed: HRESULT 0x{:08X}",
            create_hr as u32
        ));
    }

    unsafe {
        (*context).CopyResource(
            staging as *mut ID3D11Resource,
            buffer as *mut ID3D11Resource,
        );
    }

    let mut mapped: D3D11_MAPPED_SUBRESOURCE = unsafe { mem::zeroed() };
    let map_hr = unsafe {
        (*context).Map(
            staging as *mut ID3D11Resource,
            0,
            D3D11_MAP_READ,
            0,
            &mut mapped,
        )
    };
    if FAILED(map_hr) || mapped.pData.is_null() {
        unsafe {
            release_unknown(staging as *mut WinVoid);
        }
        return Err(format!(
            "Map staging scene constant buffer failed: HRESULT 0x{:08X}",
            map_hr as u32
        ));
    }

    let byte_len = source_desc.ByteWidth as usize;
    let bytes = unsafe { slice::from_raw_parts(mapped.pData.cast::<u8>(), byte_len) }.to_vec();
    unsafe {
        (*context).Unmap(staging as *mut ID3D11Resource, 0);
        release_unknown(staging as *mut WinVoid);
    }

    let output_dir = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .map(|runtime| runtime.output_dir.clone())
        .ok_or_else(|| "terrain capture runtime not initialized".to_string())?;
    write_constant_buffer_capture_files(&output_dir, &bytes, slot)
}

fn validate_scene_constant_buffer_desc(desc: &D3D11_BUFFER_DESC) -> Result<(), String> {
    if desc.ByteWidth == 0 {
        return Err("scene constant buffer has zero ByteWidth".to_string());
    }
    if desc.ByteWidth % 16 != 0 {
        return Err(format!(
            "scene constant buffer ByteWidth {} is not 16-byte aligned",
            desc.ByteWidth
        ));
    }
    if desc.BindFlags & D3D11_BIND_CONSTANT_BUFFER == 0 {
        return Err(format!(
            "scene constant buffer missing D3D11_BIND_CONSTANT_BUFFER: BindFlags {}",
            desc.BindFlags
        ));
    }
    Ok(())
}

fn write_constant_buffer_capture_files(
    output_dir: &Path,
    bytes: &[u8],
    slot: u32,
) -> Result<CapturedConstantBufferMeta, String> {
    fs::create_dir_all(output_dir).map_err(|error| error.to_string())?;
    fs::write(output_dir.join(TERRAIN_SCENE_CB0_FILE), bytes).map_err(|error| error.to_string())?;
    let meta = CapturedConstantBufferMeta {
        byte_len: bytes.len(),
        fnv1a64: fnv1a64_bytes(bytes),
        slot,
    };
    let meta_json = format!(
        concat!(
            "{{\n",
            "  \"schema\": \"battle-plan-terrain-scene-constant-buffer-capture-v1\",\n",
            "  \"symbol\": \"{}\",\n",
            "  \"file\": \"{}\",\n",
            "  \"byte_len\": {},\n",
            "  \"fnv1a64\": \"{}\",\n",
            "  \"shader_constant_buffer_slot\": {},\n",
            "  \"source\": \"ps_get_constant_buffers_after_terrain_id_capture\"\n",
            "}}\n"
        ),
        TERRAIN_SCENE_CB0_SYMBOL, TERRAIN_SCENE_CB0_FILE, meta.byte_len, meta.fnv1a64, meta.slot
    );
    fs::write(output_dir.join(TERRAIN_SCENE_CB0_META_FILE), meta_json)
        .map_err(|error| error.to_string())?;
    Ok(meta)
}

fn record_constant_buffer_success(meta: &CapturedConstantBufferMeta) {
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            runtime
                .captured_symbols
                .insert(TERRAIN_SCENE_CB0_SYMBOL.to_string());
            write_event(
                &runtime.output_dir,
                &format!(
                    "{{\"type\":\"runtime_constant_buffer_captured\",\"symbol\":\"{}\",\"file\":\"{}\",\"byte_len\":{},\"fnv1a64\":\"{}\",\"shader_constant_buffer_slot\":{}}}",
                    TERRAIN_SCENE_CB0_SYMBOL,
                    TERRAIN_SCENE_CB0_FILE,
                    meta.byte_len,
                    meta.fnv1a64,
                    meta.slot
                ),
            );
        }
    }
}

fn record_constant_buffer_rejected(slot: u32, error: &str) {
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            runtime.last_error = Some(error.to_string());
            write_event(
                &runtime.output_dir,
                &format!(
                    "{{\"type\":\"terrain_capture_constant_buffer_rejected\",\"symbol\":\"{}\",\"slot\":{},\"message\":\"{}\"}}",
                    TERRAIN_SCENE_CB0_SYMBOL,
                    slot,
                    json_escape(error)
                ),
            );
        }
    }
}

#[derive(Clone, Copy)]
struct CandidateBinding {
    target: CaptureTarget,
    binding: TerrainSrvBinding,
    identity: &'static str,
    requires_captured_symbol: Option<&'static str>,
}

fn candidate_resource_bindings(start_slot: UINT, srvs: &[usize]) -> Vec<CandidateBinding> {
    let Some(runtime) = RUNTIME.get() else {
        return Vec::new();
    };
    let Ok(mut runtime) = runtime.lock() else {
        return Vec::new();
    };
    let pending_targets = CAPTURE_TARGETS
        .iter()
        .copied()
        .filter(|target| !runtime.captured_symbols.contains(target.symbol))
        .collect::<Vec<_>>();
    let mut candidates = Vec::new();
    for target in &pending_targets {
        if let Some(binding) =
            runtime
                .state
                .observe_ps_set_named_shader_resource(target.symbol, start_slot, srvs)
        {
            candidates.push(CandidateBinding {
                target: *target,
                binding,
                identity: "active_shader_declared_slot",
                requires_captured_symbol: None,
            });
        }
    }
    if !candidates.is_empty() {
        return candidates;
    }
    let Some(control) = runtime.control.clone() else {
        return Vec::new();
    };
    if pending_targets
        .iter()
        .any(|target| target.symbol == TERRAIN_ID_RESOURCE_SYMBOL)
    {
        candidates.extend(control.allowed_slots.iter().filter_map(|slot| {
            let index = slot.checked_sub(start_slot)?;
            let srv = *srvs.get(index as usize)?;
            (srv != 0).then_some(CandidateBinding {
                target: CAPTURE_TARGETS[0],
                binding: TerrainSrvBinding { srv, slot: *slot },
                identity: "late_attach_shader_cache_slot_and_texture_desc",
                requires_captured_symbol: None,
            })
        }));
    }
    for pair in &control.paired_resource_slots {
        let Some(target) = capture_target_by_symbol(&pair.symbol) else {
            continue;
        };
        if !pending_targets
            .iter()
            .any(|pending| pending.symbol == target.symbol)
        {
            continue;
        }
        let Some(terrain_index) = pair.terrain_id_srv_slot.checked_sub(start_slot) else {
            continue;
        };
        let Some(terrain_srv) = srvs.get(terrain_index as usize) else {
            continue;
        };
        if *terrain_srv == 0 {
            continue;
        }
        let Some(index) = pair.shader_resource_slot.checked_sub(start_slot) else {
            continue;
        };
        let Some(&srv) = srvs.get(index as usize) else {
            continue;
        };
        if srv == 0 {
            continue;
        }
        candidates.push(CandidateBinding {
            target,
            binding: TerrainSrvBinding {
                srv,
                slot: pair.shader_resource_slot,
            },
            identity: "late_attach_shader_cache_paired_terrain_slot",
            requires_captured_symbol: Some(TERRAIN_ID_RESOURCE_SYMBOL),
        });
    }
    candidates
}

fn capture_target_by_symbol(symbol: &str) -> Option<CaptureTarget> {
    CAPTURE_TARGETS
        .iter()
        .copied()
        .find(|target| target.symbol == symbol)
}

fn candidate_requirement_met(candidate: &CandidateBinding) -> bool {
    let Some(required) = candidate.requires_captured_symbol else {
        return true;
    };
    capture_symbol_observed(required)
}

fn capture_symbol_observed(symbol: &str) -> bool {
    RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .is_some_and(|runtime| {
            if symbol == MAP_FRAME_BUNDLE_SYMBOL {
                requested_map_frame_capture_profile_for_runtime(&runtime)
                    .map(|profile| {
                        runtime
                            .completed_map_frame_moments
                            .contains(profile.capture_moment)
                    })
                    .unwrap_or(true)
            } else {
                runtime.captured_symbols.contains(symbol)
            }
        })
}

#[derive(Debug)]
struct CapturedTextureMeta {
    width: u32,
    height: u32,
    byte_len: usize,
    fnv1a64: String,
    source_format: u32,
    slot: u32,
    identity: &'static str,
}

unsafe fn capture_runtime_texture(
    context: *mut ID3D11DeviceContext,
    target: CaptureTarget,
    srv: *mut ID3D11ShaderResourceView,
    slot: u32,
    identity: &'static str,
) -> Result<CapturedTextureMeta, String> {
    if context.is_null() || srv.is_null() {
        return Err(format!("null D3D11 context or {} SRV", target.symbol));
    }

    let mut resource: *mut ID3D11Resource = ptr::null_mut();
    unsafe {
        (*srv).GetResource(&mut resource);
    }
    if resource.is_null() {
        return Err(format!(
            "{} SRV did not expose an ID3D11Resource",
            target.symbol
        ));
    }

    let mut texture_void: *mut WinVoid = ptr::null_mut();
    let qi = unsafe {
        (*(resource as *mut IUnknown)).QueryInterface(&IID_ID3D11Texture2D, &mut texture_void)
    };
    unsafe {
        release_unknown(resource as *mut WinVoid);
    }
    if FAILED(qi) || texture_void.is_null() {
        return Err(format!(
            "{} SRV resource is not ID3D11Texture2D: HRESULT 0x{:08X}",
            target.symbol, qi as u32
        ));
    }
    let texture = texture_void as *mut ID3D11Texture2D;

    let mut source_desc_native: D3D11_TEXTURE2D_DESC = unsafe { mem::zeroed() };
    unsafe {
        (*texture).GetDesc(&mut source_desc_native);
    }
    let source_desc = texture_desc_from_native(&source_desc_native);
    if target.validate_control {
        validate_against_control(&source_desc, slot)?;
    }
    validate_capture_source_desc(target, &source_desc)
        .map_err(|error| format!("{} source texture rejected: {error:?}", target.symbol))?;

    let mut device: *mut ID3D11Device = ptr::null_mut();
    unsafe {
        (*texture).GetDevice(&mut device);
    }
    if device.is_null() {
        unsafe {
            release_unknown(texture as *mut WinVoid);
        }
        return Err(format!(
            "{} texture did not expose an ID3D11Device",
            target.symbol
        ));
    }

    let mut staging_desc = source_desc_native;
    staging_desc.MipLevels = 1;
    staging_desc.ArraySize = 1;
    staging_desc.Usage = D3D11_USAGE_STAGING;
    staging_desc.BindFlags = 0;
    staging_desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
    staging_desc.MiscFlags = 0;
    staging_desc.SampleDesc.Count = 1;
    staging_desc.SampleDesc.Quality = 0;

    let mut staging: *mut ID3D11Texture2D = ptr::null_mut();
    let create_hr = unsafe { (*device).CreateTexture2D(&staging_desc, ptr::null(), &mut staging) };
    unsafe {
        release_unknown(device as *mut WinVoid);
    }
    if FAILED(create_hr) || staging.is_null() {
        unsafe {
            release_unknown(texture as *mut WinVoid);
        }
        return Err(format!(
            "CreateTexture2D {} staging readback failed: HRESULT 0x{:08X}",
            target.symbol, create_hr as u32
        ));
    }

    unsafe {
        (*context).CopySubresourceRegion(
            staging as *mut ID3D11Resource,
            0,
            0,
            0,
            0,
            texture as *mut ID3D11Resource,
            0,
            ptr::null(),
        );
    }

    let mut mapped: D3D11_MAPPED_SUBRESOURCE = unsafe { mem::zeroed() };
    let map_hr = unsafe {
        (*context).Map(
            staging as *mut ID3D11Resource,
            0,
            D3D11_MAP_READ,
            0,
            &mut mapped,
        )
    };
    if FAILED(map_hr) || mapped.pData.is_null() {
        unsafe {
            release_unknown(staging as *mut WinVoid);
            release_unknown(texture as *mut WinVoid);
        }
        return Err(format!(
            "Map staging {} failed: HRESULT 0x{:08X}",
            target.symbol, map_hr as u32
        ));
    }

    let row_pitch = mapped.RowPitch as usize;
    let mapped_len = row_pitch
        .checked_mul(source_desc.height as usize)
        .ok_or_else(|| format!("{} mapped byte span overflowed", target.symbol))?;
    let mapped_rows = unsafe { slice::from_raw_parts(mapped.pData.cast::<u8>(), mapped_len) };
    let captured_bytes = copy_mapped_capture_rows(target, mapped_rows, row_pitch, &source_desc)
        .map_err(|error| format!("{} mapped row copy rejected: {error:?}", target.symbol))?;
    unsafe {
        (*context).Unmap(staging as *mut ID3D11Resource, 0);
        release_unknown(staging as *mut WinVoid);
        release_unknown(texture as *mut WinVoid);
    }

    let output_dir = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .map(|runtime| runtime.output_dir.clone())
        .ok_or_else(|| "terrain capture runtime not initialized".to_string())?;
    write_capture_files(
        &output_dir,
        target,
        &captured_bytes,
        &source_desc,
        slot,
        identity,
    )
}

fn validate_capture_source_desc(
    target: CaptureTarget,
    desc: &Texture2DDesc,
) -> Result<(), TerrainCaptureError> {
    match target.readback_format {
        CaptureReadbackFormat::Rgba8 => validate_rgba8_shader_resource_source_desc(desc),
        CaptureReadbackFormat::R32G32B32A32Float => {
            validate_r32g32b32a32_float_shader_resource_source_desc(desc)
        }
    }
}

fn copy_mapped_capture_rows(
    target: CaptureTarget,
    rows: &[u8],
    row_pitch: usize,
    desc: &Texture2DDesc,
) -> Result<Vec<u8>, TerrainCaptureError> {
    match target.readback_format {
        CaptureReadbackFormat::Rgba8 => {
            copy_pitched_terrain_id_rows(rows, row_pitch, desc.width, desc.height, desc.format)
        }
        CaptureReadbackFormat::R32G32B32A32Float => copy_pitched_r32g32b32a32_float_rows(
            rows,
            row_pitch,
            desc.width,
            desc.height,
            desc.format,
        ),
    }
}

fn write_capture_files(
    output_dir: &Path,
    target: CaptureTarget,
    captured_bytes: &[u8],
    desc: &Texture2DDesc,
    slot: u32,
    identity: &'static str,
) -> Result<CapturedTextureMeta, String> {
    fs::create_dir_all(output_dir).map_err(|error| error.to_string())?;
    let capture_path = output_dir.join(target.rgba_file);
    fs::write(&capture_path, captured_bytes).map_err(|error| error.to_string())?;
    let fnv1a64 = fnv1a64_bytes(captured_bytes);
    let meta = CapturedTextureMeta {
        width: desc.width,
        height: desc.height,
        byte_len: captured_bytes.len(),
        fnv1a64,
        source_format: desc.format,
        slot,
        identity,
    };
    let meta_json = if target.symbol == TERRAIN_ID_RESOURCE_SYMBOL {
        format!(
            concat!(
                "{{\n",
                "  \"schema\": \"battle-plan-terrain-id-map-capture-v1\",\n",
                "  \"file\": \"{}\",\n",
                "  \"width\": {},\n",
                "  \"height\": {},\n",
                "  \"byte_len\": {},\n",
                "  \"fnv1a64\": \"{}\",\n",
                "  \"format\": \"rgba8_unorm_engine_packed_terrain_ids\",\n",
                "  \"source_dxgi_format\": {},\n",
                "  \"terrain_id_srv_slot\": {},\n",
                "  \"source\": \"{}\",\n",
                "  \"sampler\": {{\"mag_filter\":\"Point\",\"min_filter\":\"Point\",\"mip_filter\":\"None\",\"address_u\":\"Clamp\",\"address_v\":\"Clamp\"}}\n",
                "}}\n"
            ),
            target.rgba_file,
            meta.width,
            meta.height,
            meta.byte_len,
            meta.fnv1a64,
            meta.source_format,
            meta.slot,
            identity
        )
    } else {
        format!(
            concat!(
                "{{\n",
                "  \"schema\": \"battle-plan-runtime-texture-capture-v1\",\n",
                "  \"symbol\": \"{}\",\n",
                "  \"file\": \"{}\",\n",
                "  \"width\": {},\n",
                "  \"height\": {},\n",
                "  \"byte_len\": {},\n",
                "  \"fnv1a64\": \"{}\",\n",
                "  \"format\": \"{}\",\n",
                "  \"source_dxgi_format\": {},\n",
                "  \"shader_resource_slot\": {},\n",
                "  \"source\": \"{}\"\n",
                "}}\n"
            ),
            target.symbol,
            target.rgba_file,
            meta.width,
            meta.height,
            meta.byte_len,
            meta.fnv1a64,
            target.output_format,
            meta.source_format,
            meta.slot,
            identity
        )
    };
    fs::write(output_dir.join(target.meta_file), meta_json).map_err(|error| error.to_string())?;
    Ok(meta)
}

unsafe fn capture_map_frame_ps_srvs(
    context: *mut ID3D11DeviceContext,
    resource_slots: &BTreeMap<String, u32>,
    output_dir: &Path,
    draw_index: u32,
    profile: MapFrameCaptureProfile,
) -> (Vec<MapFrameTextureBindingFile>, Vec<String>) {
    let binding_slots = D3D11_COMMONSHADER_INPUT_RESOURCE_SLOT_COUNT;
    let mut srvs = vec![ptr::null_mut(); binding_slots as usize];
    unsafe {
        (*context).PSGetShaderResources(0, binding_slots, srvs.as_mut_ptr());
    }
    let mut bindings = Vec::new();
    let mut blockers = Vec::new();
    for (slot, srv) in srvs.into_iter().enumerate() {
        let slot = slot as u32;
        if srv.is_null() {
            if let Some(declared_name) = map_frame_declared_resource_name(resource_slots, slot) {
                bindings.push(MapFrameTextureBindingFile {
                    stage: "PS",
                    slot,
                    declared_name: Some(declared_name),
                    is_null: true,
                    view_desc: None,
                    file: None,
                    byte_len: 0,
                    fnv1a64: None,
                    dxgi_format: 0,
                    width: 0,
                    height: 0,
                    mip_count: 0,
                    array_size: 0,
                    sample_count: 0,
                    sample_quality: 0,
                    payload_format: None,
                    subresources: Vec::new(),
                    provenance: None,
                    bind_flags: 0,
                    usage: 0,
                    cpu_access_flags: 0,
                    misc_flags: 0,
                });
            }
            continue;
        }
        match unsafe {
            map_frame_texture_binding_from_srv(
                context,
                srv,
                slot,
                resource_slots,
                output_dir,
                draw_index,
                profile,
            )
        } {
            Ok((binding, mut binding_blockers)) => {
                bindings.push(binding);
                blockers.append(&mut binding_blockers);
            }
            Err(error) => blockers.push(error),
        }
        unsafe {
            release_unknown(srv as *mut WinVoid);
        }
    }
    (bindings, blockers)
}

unsafe fn map_frame_texture_binding_from_srv(
    context: *mut ID3D11DeviceContext,
    srv: *mut ID3D11ShaderResourceView,
    slot: u32,
    resource_slots: &BTreeMap<String, u32>,
    output_dir: &Path,
    draw_index: u32,
    profile: MapFrameCaptureProfile,
) -> Result<(MapFrameTextureBindingFile, Vec<String>), String> {
    let mut resource: *mut ID3D11Resource = ptr::null_mut();
    unsafe {
        (*srv).GetResource(&mut resource);
    }
    if resource.is_null() {
        return Err(format!("PS SRV slot {slot} did not expose a resource"));
    }
    let mut texture_void: *mut WinVoid = ptr::null_mut();
    let qi = unsafe {
        (*(resource as *mut IUnknown)).QueryInterface(&IID_ID3D11Texture2D, &mut texture_void)
    };
    unsafe {
        release_unknown(resource as *mut WinVoid);
    }
    if FAILED(qi) || texture_void.is_null() {
        return Err(format!(
            "PS SRV slot {slot} resource is not ID3D11Texture2D: HRESULT 0x{:08X}",
            qi as u32
        ));
    }
    let texture = texture_void as *mut ID3D11Texture2D;
    let result = (|| -> Result<(MapFrameTextureBindingFile, Vec<String>), String> {
        let mut desc: D3D11_TEXTURE2D_DESC = unsafe { mem::zeroed() };
        unsafe {
            (*texture).GetDesc(&mut desc);
        }
        let mut srv_desc_native: D3D11_SHADER_RESOURCE_VIEW_DESC = unsafe { mem::zeroed() };
        unsafe {
            (*srv).GetDesc(&mut srv_desc_native);
        }
        let view_desc = map_frame_srv_desc_file(&srv_desc_native, &desc, slot)?;
        let declared_name = resource_slots
            .iter()
            .find_map(|(name, resource_slot)| (*resource_slot == slot).then(|| name.clone()));
        let mut blockers = Vec::new();
        let mut file_meta: (Option<String>, usize, Option<String>);
        let mut payload_format = None;
        let mut subresources = Vec::new();
        let mut provenance = None;
        match unsafe {
            capture_map_frame_srv_texture_exact(
                context,
                srv,
                texture,
                slot,
                declared_name.as_deref(),
                &desc,
                output_dir,
                draw_index,
                profile,
            )
        } {
            Ok(payload) => {
                file_meta = (Some(payload.file), payload.byte_len, Some(payload.fnv1a64));
                payload_format = Some(NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT);
                subresources = payload.subresources;
                provenance = Some("d3d11_staging_map_readback_from_active_ps_srv");
            }
            Err(error) => {
                blockers.push(error);
                file_meta =
                    map_frame_known_runtime_texture_file(output_dir, declared_name.as_deref());
            }
        }
        if payload_format.is_none() && file_meta.0.is_none() {
            if let Some(target) = declared_name.as_deref().and_then(capture_target_by_symbol) {
                match unsafe {
                    capture_runtime_texture(
                        context,
                        target,
                        srv,
                        slot,
                        "map_frame_active_draw_declared_resource",
                    )
                } {
                    Ok(meta) => {
                        record_capture_success(target, &meta);
                        file_meta = (
                            Some(target.rgba_file.to_string()),
                            meta.byte_len,
                            Some(meta.fnv1a64),
                        );
                    }
                    Err(error) => blockers.push(format!(
                        "PS SRV slot {slot} declared {} canonical runtime readback failed: {error}",
                        target.symbol
                    )),
                }
            }
        }
        let (file, byte_len, fnv1a64) = file_meta;
        Ok((
            MapFrameTextureBindingFile {
                stage: "PS",
                slot,
                declared_name,
                is_null: false,
                view_desc: Some(view_desc),
                file,
                byte_len,
                fnv1a64,
                dxgi_format: desc.Format as u32,
                width: desc.Width,
                height: desc.Height,
                mip_count: desc.MipLevels,
                array_size: desc.ArraySize,
                sample_count: desc.SampleDesc.Count,
                sample_quality: desc.SampleDesc.Quality,
                payload_format,
                subresources,
                provenance,
                bind_flags: desc.BindFlags,
                usage: desc.Usage as u32,
                cpu_access_flags: desc.CPUAccessFlags,
                misc_flags: desc.MiscFlags,
            },
            blockers,
        ))
    })();
    unsafe {
        release_unknown(texture as *mut WinVoid);
    }
    result
}

#[derive(Debug)]
struct MapFrameExactTexturePayload {
    file: String,
    byte_len: usize,
    fnv1a64: String,
    subresources: Vec<MapFrameTextureSubresourceFile>,
}

#[derive(Clone, Copy, Debug)]
struct MapFrameSrvSubresourceRange {
    first_mip: u32,
    mip_levels: u32,
    first_array_slice: u32,
    array_size: u32,
}

struct MappedTextureSubresourceBytes {
    mip_slice: u32,
    array_slice: u32,
    width: u32,
    height: u32,
    row_pitch: u32,
    depth_pitch: u32,
    bytes: Vec<u8>,
}

unsafe fn capture_om_render_target_payload_exact(
    context: *mut ID3D11DeviceContext,
    texture: *mut ID3D11Texture2D,
    source_desc: &D3D11_TEXTURE2D_DESC,
    slot: u32,
    output_dir: &Path,
    draw_index: u32,
    profile: MapFrameCaptureProfile,
) -> Result<MapFrameOutputMergerTexturePayloadFile, String> {
    unsafe {
        capture_om_texture_payload_exact(
            context,
            texture,
            source_desc,
            slot,
            output_dir,
            map_frame_post_pass_render_target_payload_file(draw_index, slot, profile),
            "post_draw_om_render_target_resolved_readback",
        )
    }
}

unsafe fn capture_om_texture_payload_exact(
    context: *mut ID3D11DeviceContext,
    texture: *mut ID3D11Texture2D,
    source_desc: &D3D11_TEXTURE2D_DESC,
    slot: u32,
    output_dir: &Path,
    file: String,
    provenance: &'static str,
) -> Result<MapFrameOutputMergerTexturePayloadFile, String> {
    if context.is_null() || texture.is_null() {
        return Err(format!(
            "OM render target slot {slot} exact D3D11 readback got a null context or Texture2D"
        ));
    }
    if source_desc.Width == 0 || source_desc.Height == 0 || source_desc.MipLevels == 0 {
        return Err(format!(
            "OM render target slot {slot} exact D3D11 readback rejected empty Texture2D desc {}x{} mips {}",
            source_desc.Width, source_desc.Height, source_desc.MipLevels
        ));
    }
    if source_desc.MipLevels != 1 || source_desc.ArraySize != 1 {
        return Err(format!(
            "OM render target slot {slot} exact D3D11 readback requires a single mip/slice render target, got mips {} array {}",
            source_desc.MipLevels, source_desc.ArraySize
        ));
    }
    let mut device: *mut ID3D11Device = ptr::null_mut();
    unsafe {
        (*texture).GetDevice(&mut device);
    }
    if device.is_null() {
        return Err(format!(
            "OM render target slot {slot} exact D3D11 readback Texture2D did not expose an ID3D11Device"
        ));
    }
    let result = (|| -> Result<MapFrameOutputMergerTexturePayloadFile, String> {
        let mut readback_desc = *source_desc;
        let mut resolved: *mut ID3D11Texture2D = ptr::null_mut();
        let readback_texture = if source_desc.SampleDesc.Count > 1 {
            readback_desc.SampleDesc.Count = 1;
            readback_desc.SampleDesc.Quality = 0;
            readback_desc.BindFlags = 0;
            readback_desc.CPUAccessFlags = 0;
            readback_desc.MiscFlags = 0;
            unsafe {
                resolved = create_texture2d_resolve_target(device, &readback_desc, slot)?;
                (*context).ResolveSubresource(
                    resolved as *mut ID3D11Resource,
                    0,
                    texture as *mut ID3D11Resource,
                    0,
                    source_desc.Format,
                );
            }
            resolved
        } else {
            texture
        };
        let staging = unsafe { create_texture2d_exact_staging(device, &readback_desc, slot)? };
        unsafe {
            (*context).CopyResource(
                staging as *mut ID3D11Resource,
                readback_texture as *mut ID3D11Resource,
            );
        }
        let mapped = unsafe {
            readback_texture2d_subresource_exact(
                context,
                staging,
                D3D11CalcSubresource(0, 0, readback_desc.MipLevels),
                0,
                0,
                readback_desc.Width,
                readback_desc.Height,
                slot,
            )
        }?;
        write_map_frame_bytes(output_dir, &file, &mapped.bytes)?;
        let byte_len = mapped.bytes.len();
        let subresources = vec![MapFrameTextureSubresourceFile {
            mip_slice: mapped.mip_slice,
            array_slice: mapped.array_slice,
            file_offset: 0,
            byte_len,
            row_pitch: mapped.row_pitch,
            depth_pitch: mapped.depth_pitch,
            width: mapped.width,
            height: mapped.height,
        }];
        Ok(MapFrameOutputMergerTexturePayloadFile {
            file,
            byte_len,
            fnv1a64: fnv1a64_bytes(&mapped.bytes),
            payload_format: NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT,
            subresources,
            provenance,
        })
        .map(|payload| {
            unsafe {
                release_unknown(staging as *mut WinVoid);
                if !resolved.is_null() {
                    release_unknown(resolved as *mut WinVoid);
                }
            }
            payload
        })
        .map_err(|error| {
            unsafe {
                release_unknown(staging as *mut WinVoid);
                if !resolved.is_null() {
                    release_unknown(resolved as *mut WinVoid);
                }
            }
            error
        })
    })();
    unsafe {
        release_unknown(device as *mut WinVoid);
    }
    result
}

unsafe fn create_texture2d_resolve_target(
    device: *mut ID3D11Device,
    desc: &D3D11_TEXTURE2D_DESC,
    slot: u32,
) -> Result<*mut ID3D11Texture2D, String> {
    let mut resolve: *mut ID3D11Texture2D = ptr::null_mut();
    let create_hr = unsafe { (*device).CreateTexture2D(desc, ptr::null(), &mut resolve) };
    if FAILED(create_hr) || resolve.is_null() {
        return Err(format!(
            "CreateTexture2D OM render target slot {slot} single-sample resolve target failed: HRESULT 0x{:08X}",
            create_hr as u32
        ));
    }
    Ok(resolve)
}

unsafe fn capture_map_frame_srv_texture_exact(
    context: *mut ID3D11DeviceContext,
    srv: *mut ID3D11ShaderResourceView,
    texture: *mut ID3D11Texture2D,
    slot: u32,
    declared_name: Option<&str>,
    source_desc: &D3D11_TEXTURE2D_DESC,
    output_dir: &Path,
    draw_index: u32,
    profile: MapFrameCaptureProfile,
) -> Result<MapFrameExactTexturePayload, String> {
    if context.is_null() || srv.is_null() || texture.is_null() {
        return Err(format!(
            "PS SRV slot {slot} exact D3D11 readback got a null context, SRV, or Texture2D"
        ));
    }
    if source_desc.Width == 0 || source_desc.Height == 0 || source_desc.MipLevels == 0 {
        return Err(format!(
            "PS SRV slot {slot} exact D3D11 readback rejected empty Texture2D desc {}x{} mips {}",
            source_desc.Width, source_desc.Height, source_desc.MipLevels
        ));
    }
    if source_desc.ArraySize == 0 {
        return Err(format!(
            "PS SRV slot {slot} exact D3D11 readback rejected Texture2D with ArraySize 0"
        ));
    }
    if source_desc.SampleDesc.Count != 1 {
        return Err(format!(
            "PS SRV slot {slot} exact D3D11 readback rejected multisampled Texture2D sample count {}; resolve would not be exact",
            source_desc.SampleDesc.Count
        ));
    }

    let mut srv_desc: D3D11_SHADER_RESOURCE_VIEW_DESC = unsafe { mem::zeroed() };
    unsafe {
        (*srv).GetDesc(&mut srv_desc);
    }
    let range = map_frame_srv_subresource_range(&srv_desc, source_desc, slot)?;

    let mut device: *mut ID3D11Device = ptr::null_mut();
    unsafe {
        (*texture).GetDevice(&mut device);
    }
    if device.is_null() {
        return Err(format!(
            "PS SRV slot {slot} exact D3D11 readback Texture2D did not expose an ID3D11Device"
        ));
    }
    let result = (|| -> Result<MapFrameExactTexturePayload, String> {
        let staging = unsafe { create_texture2d_exact_staging(device, source_desc, slot)? };
        unsafe {
            (*context).CopyResource(
                staging as *mut ID3D11Resource,
                texture as *mut ID3D11Resource,
            );
        }
        let mut payload_bytes = Vec::new();
        let mut subresources = Vec::new();
        let end_array_slice = range
            .first_array_slice
            .checked_add(range.array_size)
            .ok_or_else(|| format!("PS SRV slot {slot} exact D3D11 array range overflowed"))?;
        let end_mip = range
            .first_mip
            .checked_add(range.mip_levels)
            .ok_or_else(|| format!("PS SRV slot {slot} exact D3D11 mip range overflowed"))?;
        for array_slice in range.first_array_slice..end_array_slice {
            for mip_slice in range.first_mip..end_mip {
                let width = mip_dimension(source_desc.Width, mip_slice);
                let height = mip_dimension(source_desc.Height, mip_slice);
                let source_subresource =
                    D3D11CalcSubresource(mip_slice, array_slice, source_desc.MipLevels);
                let mapped = unsafe {
                    readback_texture2d_subresource_exact(
                        context,
                        staging,
                        source_subresource,
                        mip_slice,
                        array_slice,
                        width,
                        height,
                        slot,
                    )
                }?;
                let file_offset = payload_bytes.len();
                payload_bytes.extend_from_slice(&mapped.bytes);
                subresources.push(MapFrameTextureSubresourceFile {
                    mip_slice: mapped.mip_slice,
                    array_slice: mapped.array_slice,
                    file_offset,
                    byte_len: mapped.bytes.len(),
                    row_pitch: mapped.row_pitch,
                    depth_pitch: mapped.depth_pitch,
                    width: mapped.width,
                    height: mapped.height,
                });
            }
        }
        let file = map_frame_srv_payload_file(draw_index, slot, declared_name, profile);
        write_map_frame_bytes(output_dir, &file, &payload_bytes)?;
        Ok(MapFrameExactTexturePayload {
            file,
            byte_len: payload_bytes.len(),
            fnv1a64: fnv1a64_bytes(&payload_bytes),
            subresources,
        })
        .map(|payload| {
            unsafe {
                release_unknown(staging as *mut WinVoid);
            }
            payload
        })
        .map_err(|error| {
            unsafe {
                release_unknown(staging as *mut WinVoid);
            }
            error
        })
    })();
    unsafe {
        release_unknown(device as *mut WinVoid);
    }
    result
}

unsafe fn create_texture2d_exact_staging(
    device: *mut ID3D11Device,
    source_desc: &D3D11_TEXTURE2D_DESC,
    slot: u32,
) -> Result<*mut ID3D11Texture2D, String> {
    let mut staging_desc = *source_desc;
    staging_desc.Usage = D3D11_USAGE_STAGING;
    staging_desc.BindFlags = 0;
    staging_desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
    staging_desc.MiscFlags = 0;

    let mut staging: *mut ID3D11Texture2D = ptr::null_mut();
    let create_hr = unsafe { (*device).CreateTexture2D(&staging_desc, ptr::null(), &mut staging) };
    if FAILED(create_hr) || staging.is_null() {
        return Err(format!(
            "CreateTexture2D PS SRV slot {slot} exact full-texture staging readback failed: HRESULT 0x{:08X}",
            create_hr as u32
        ));
    }
    Ok(staging)
}

unsafe fn readback_texture2d_subresource_exact(
    context: *mut ID3D11DeviceContext,
    staging: *mut ID3D11Texture2D,
    source_subresource: u32,
    mip_slice: u32,
    array_slice: u32,
    width: u32,
    height: u32,
    slot: u32,
) -> Result<MappedTextureSubresourceBytes, String> {
    let mut mapped: D3D11_MAPPED_SUBRESOURCE = unsafe { mem::zeroed() };
    let map_hr = unsafe {
        (*context).Map(
            staging as *mut ID3D11Resource,
            source_subresource,
            D3D11_MAP_READ,
            0,
            &mut mapped,
        )
    };
    if FAILED(map_hr) || mapped.pData.is_null() {
        return Err(format!(
            "Map PS SRV slot {slot} exact staging readback failed for mip {mip_slice}, array slice {array_slice}: HRESULT 0x{:08X}",
            map_hr as u32
        ));
    }

    let row_pitch = mapped.RowPitch;
    let depth_pitch = mapped.DepthPitch;
    let result = if row_pitch == 0 || depth_pitch == 0 {
        Err(format!(
            "PS SRV slot {slot} exact staging readback produced zero pitch for mip {mip_slice}, array slice {array_slice}: RowPitch {row_pitch}, DepthPitch {depth_pitch}"
        ))
    } else {
        let bytes =
            unsafe { slice::from_raw_parts(mapped.pData.cast::<u8>(), depth_pitch as usize) }
                .to_vec();
        Ok(MappedTextureSubresourceBytes {
            mip_slice,
            array_slice,
            width,
            height,
            row_pitch,
            depth_pitch,
            bytes,
        })
    };
    unsafe {
        (*context).Unmap(staging as *mut ID3D11Resource, source_subresource);
    }
    result
}

fn map_frame_srv_subresource_range(
    srv_desc: &D3D11_SHADER_RESOURCE_VIEW_DESC,
    texture_desc: &D3D11_TEXTURE2D_DESC,
    slot: u32,
) -> Result<MapFrameSrvSubresourceRange, String> {
    match srv_desc.ViewDimension {
        D3D11_SRV_DIMENSION_TEXTURE2D => {
            let view = unsafe { *srv_desc.u.Texture2D() };
            let mip_levels =
                resolve_srv_mip_count(view.MostDetailedMip, view.MipLevels, texture_desc, slot)?;
            Ok(MapFrameSrvSubresourceRange {
                first_mip: view.MostDetailedMip,
                mip_levels,
                first_array_slice: 0,
                array_size: 1,
            })
        }
        D3D11_SRV_DIMENSION_TEXTURE2DARRAY => {
            let view = unsafe { *srv_desc.u.Texture2DArray() };
            let mip_levels =
                resolve_srv_mip_count(view.MostDetailedMip, view.MipLevels, texture_desc, slot)?;
            let array_size = resolve_srv_array_count(
                view.FirstArraySlice,
                view.ArraySize,
                texture_desc.ArraySize,
                slot,
            )?;
            Ok(MapFrameSrvSubresourceRange {
                first_mip: view.MostDetailedMip,
                mip_levels,
                first_array_slice: view.FirstArraySlice,
                array_size,
            })
        }
        D3D11_SRV_DIMENSION_TEXTURECUBE => {
            let view = unsafe { *srv_desc.u.TextureCube() };
            let mip_levels =
                resolve_srv_mip_count(view.MostDetailedMip, view.MipLevels, texture_desc, slot)?;
            validate_srv_array_range(0, 6, texture_desc.ArraySize, slot)?;
            Ok(MapFrameSrvSubresourceRange {
                first_mip: view.MostDetailedMip,
                mip_levels,
                first_array_slice: 0,
                array_size: 6,
            })
        }
        D3D11_SRV_DIMENSION_TEXTURECUBEARRAY => {
            let view = unsafe { *srv_desc.u.TextureCubeArray() };
            let mip_levels =
                resolve_srv_mip_count(view.MostDetailedMip, view.MipLevels, texture_desc, slot)?;
            let array_size = view.NumCubes.checked_mul(6).ok_or_else(|| {
                format!(
                    "PS SRV slot {slot} TextureCubeArray face count overflowed for NumCubes {}",
                    view.NumCubes
                )
            })?;
            validate_srv_array_range(
                view.First2DArrayFace,
                array_size,
                texture_desc.ArraySize,
                slot,
            )?;
            Ok(MapFrameSrvSubresourceRange {
                first_mip: view.MostDetailedMip,
                mip_levels,
                first_array_slice: view.First2DArrayFace,
                array_size,
            })
        }
        other => Err(format!(
            "PS SRV slot {slot} exact D3D11 readback skipped unsupported SRV view dimension {other}; only non-MS Texture2D-backed views are captured exactly"
        )),
    }
}

fn map_frame_srv_desc_file(
    srv_desc: &D3D11_SHADER_RESOURCE_VIEW_DESC,
    texture_desc: &D3D11_TEXTURE2D_DESC,
    slot: u32,
) -> Result<MapFrameShaderResourceViewDescFile, String> {
    match srv_desc.ViewDimension {
        D3D11_SRV_DIMENSION_TEXTURE2D => {
            let view = unsafe { *srv_desc.u.Texture2D() };
            let mip_levels =
                resolve_srv_mip_count(view.MostDetailedMip, view.MipLevels, texture_desc, slot)?;
            Ok(MapFrameShaderResourceViewDescFile {
                format: srv_desc.Format as u32,
                view_dimension: srv_desc.ViewDimension,
                most_detailed_mip: view.MostDetailedMip,
                mip_levels,
                first_array_slice: 0,
                array_size: 1,
                num_cubes: 0,
            })
        }
        D3D11_SRV_DIMENSION_TEXTURE2DARRAY => {
            let view = unsafe { *srv_desc.u.Texture2DArray() };
            let mip_levels =
                resolve_srv_mip_count(view.MostDetailedMip, view.MipLevels, texture_desc, slot)?;
            let array_size = resolve_srv_array_count(
                view.FirstArraySlice,
                view.ArraySize,
                texture_desc.ArraySize,
                slot,
            )?;
            Ok(MapFrameShaderResourceViewDescFile {
                format: srv_desc.Format as u32,
                view_dimension: srv_desc.ViewDimension,
                most_detailed_mip: view.MostDetailedMip,
                mip_levels,
                first_array_slice: view.FirstArraySlice,
                array_size,
                num_cubes: 0,
            })
        }
        D3D11_SRV_DIMENSION_TEXTURECUBE => {
            let view = unsafe { *srv_desc.u.TextureCube() };
            let mip_levels =
                resolve_srv_mip_count(view.MostDetailedMip, view.MipLevels, texture_desc, slot)?;
            validate_srv_array_range(0, 6, texture_desc.ArraySize, slot)?;
            Ok(MapFrameShaderResourceViewDescFile {
                format: srv_desc.Format as u32,
                view_dimension: srv_desc.ViewDimension,
                most_detailed_mip: view.MostDetailedMip,
                mip_levels,
                first_array_slice: 0,
                array_size: 6,
                num_cubes: 1,
            })
        }
        D3D11_SRV_DIMENSION_TEXTURECUBEARRAY => {
            let view = unsafe { *srv_desc.u.TextureCubeArray() };
            let mip_levels =
                resolve_srv_mip_count(view.MostDetailedMip, view.MipLevels, texture_desc, slot)?;
            let array_size = view.NumCubes.checked_mul(6).ok_or_else(|| {
                format!(
                    "PS SRV slot {slot} TextureCubeArray face count overflowed for NumCubes {}",
                    view.NumCubes
                )
            })?;
            validate_srv_array_range(
                view.First2DArrayFace,
                array_size,
                texture_desc.ArraySize,
                slot,
            )?;
            Ok(MapFrameShaderResourceViewDescFile {
                format: srv_desc.Format as u32,
                view_dimension: srv_desc.ViewDimension,
                most_detailed_mip: view.MostDetailedMip,
                mip_levels,
                first_array_slice: view.First2DArrayFace,
                array_size,
                num_cubes: view.NumCubes,
            })
        }
        other => Err(format!(
            "PS SRV slot {slot} skipped unsupported SRV view dimension {other}; only Texture2D-backed views are supported by the production bundle"
        )),
    }
}

fn resolve_srv_mip_count(
    most_detailed_mip: u32,
    requested_mip_levels: u32,
    texture_desc: &D3D11_TEXTURE2D_DESC,
    slot: u32,
) -> Result<u32, String> {
    if most_detailed_mip >= texture_desc.MipLevels {
        return Err(format!(
            "PS SRV slot {slot} exact D3D11 mip range starts at {}, beyond texture mip count {}",
            most_detailed_mip, texture_desc.MipLevels
        ));
    }
    let mip_levels = if requested_mip_levels == u32::MAX {
        texture_desc.MipLevels - most_detailed_mip
    } else {
        requested_mip_levels
    };
    if mip_levels == 0 {
        return Err(format!(
            "PS SRV slot {slot} exact D3D11 mip range is empty at MostDetailedMip {most_detailed_mip}"
        ));
    }
    let end_mip = most_detailed_mip
        .checked_add(mip_levels)
        .ok_or_else(|| format!("PS SRV slot {slot} exact D3D11 mip range overflowed"))?;
    if end_mip > texture_desc.MipLevels {
        return Err(format!(
            "PS SRV slot {slot} exact D3D11 mip range {}..{} exceeds texture mip count {}",
            most_detailed_mip, end_mip, texture_desc.MipLevels
        ));
    }
    Ok(mip_levels)
}

fn resolve_srv_array_count(
    first_array_slice: u32,
    requested_array_size: u32,
    texture_array_size: u32,
    slot: u32,
) -> Result<u32, String> {
    let array_size = if requested_array_size == u32::MAX {
        texture_array_size
            .checked_sub(first_array_slice)
            .ok_or_else(|| {
                format!(
                    "PS SRV slot {slot} exact D3D11 array range starts at {first_array_slice}, beyond texture array size {texture_array_size}"
                )
            })?
    } else {
        requested_array_size
    };
    validate_srv_array_range(first_array_slice, array_size, texture_array_size, slot)?;
    Ok(array_size)
}

fn validate_srv_array_range(
    first_array_slice: u32,
    array_size: u32,
    texture_array_size: u32,
    slot: u32,
) -> Result<(), String> {
    if array_size == 0 {
        return Err(format!(
            "PS SRV slot {slot} exact D3D11 array range is empty at FirstArraySlice {first_array_slice}"
        ));
    }
    let end_array_slice = first_array_slice
        .checked_add(array_size)
        .ok_or_else(|| format!("PS SRV slot {slot} exact D3D11 array range overflowed"))?;
    if end_array_slice > texture_array_size {
        return Err(format!(
            "PS SRV slot {slot} exact D3D11 array range {}..{} exceeds texture ArraySize {}",
            first_array_slice, end_array_slice, texture_array_size
        ));
    }
    Ok(())
}

fn mip_dimension(base: u32, mip_slice: u32) -> u32 {
    base.checked_shr(mip_slice).unwrap_or(0).max(1)
}

fn map_frame_srv_payload_file(
    draw_index: u32,
    slot: u32,
    declared_name: Option<&str>,
    profile: MapFrameCaptureProfile,
) -> String {
    let name = declared_name
        .map(sanitize_map_frame_file_component)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "unnamed".to_string());
    format!(
        "{}{draw_index:03}_ps_srv_slot{slot:03}_{name}.d3d11",
        profile.file_prefix
    )
}

fn map_frame_post_pass_render_target_payload_file(
    draw_index: u32,
    slot: u32,
    profile: MapFrameCaptureProfile,
) -> String {
    format!(
        "{}{draw_index:03}_{MAP_FRAME_POST_PASS_RENDER_TARGET_FILE_PREFIX}_slot{slot:03}.d3d11",
        profile.file_prefix
    )
}

fn map_frame_pre_draw_render_target_payload_file(
    draw_index: u32,
    slot: u32,
    profile: MapFrameCaptureProfile,
) -> String {
    format!(
        "{}{draw_index:03}_{MAP_FRAME_PRE_DRAW_RENDER_TARGET_FILE_PREFIX}_slot{slot:03}.d3d11",
        profile.file_prefix
    )
}

fn map_frame_pre_draw_depth_stencil_payload_file(
    draw_index: u32,
    profile: MapFrameCaptureProfile,
) -> String {
    format!(
        "{}{draw_index:03}_{MAP_FRAME_PRE_DRAW_DEPTH_STENCIL_FILE_PREFIX}.d3d11",
        profile.file_prefix
    )
}

fn sanitize_map_frame_file_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' => ch,
            _ => '_',
        })
        .collect()
}

fn map_frame_declared_resource_name(
    resource_slots: &BTreeMap<String, u32>,
    slot: u32,
) -> Option<String> {
    resource_slots
        .iter()
        .find_map(|(name, resource_slot)| (*resource_slot == slot).then(|| name.clone()))
}

fn map_frame_known_runtime_texture_file(
    output_dir: &Path,
    declared_name: Option<&str>,
) -> (Option<String>, usize, Option<String>) {
    let file = declared_name
        .and_then(capture_target_by_symbol)
        .map(|target| target.rgba_file);
    let Some(file) = file else {
        return (None, 0, None);
    };
    let path = output_dir.join(file);
    let Ok(bytes) = fs::read(&path) else {
        return (None, 0, None);
    };
    (
        Some(file.to_string()),
        bytes.len(),
        Some(fnv1a64_bytes(&bytes)),
    )
}

unsafe fn capture_map_frame_ps_samplers(
    context: *mut ID3D11DeviceContext,
    profile: MapFrameCaptureProfile,
) -> (Vec<MapFrameSamplerBindingFile>, Vec<String>) {
    let mut samplers = vec![ptr::null_mut(); MAP_FRAME_BINDING_SLOTS as usize];
    unsafe {
        (*context).PSGetSamplers(0, MAP_FRAME_BINDING_SLOTS, samplers.as_mut_ptr());
    }
    let mut bindings = Vec::new();
    let mut blockers = Vec::new();
    for (slot, sampler) in samplers.into_iter().enumerate() {
        if sampler.is_null() {
            continue;
        }
        let mut desc: D3D11_SAMPLER_DESC = unsafe { mem::zeroed() };
        unsafe {
            (*sampler).GetDesc(&mut desc);
            release_unknown(sampler as *mut WinVoid);
        }
        bindings.push(MapFrameSamplerBindingFile {
            stage: "PS",
            slot: slot as u32,
            filter: desc.Filter as u32,
            address_u: desc.AddressU as u32,
            address_v: desc.AddressV as u32,
            address_w: desc.AddressW as u32,
            mip_lod_bias: desc.MipLODBias,
            max_anisotropy: desc.MaxAnisotropy,
            comparison_func: desc.ComparisonFunc as u32,
            border_color: desc.BorderColor,
            min_lod: desc.MinLOD,
            max_lod: desc.MaxLOD,
        });
    }
    if bindings.is_empty() {
        blockers.push(format!(
            "active {} draw exposed no PS sampler-state bindings",
            profile.capture_moment
        ));
    }
    (bindings, blockers)
}

unsafe fn capture_map_frame_draw_state(
    context: *mut ID3D11DeviceContext,
    output_dir: &Path,
    draw_index: u32,
    profile: MapFrameCaptureProfile,
) -> (Option<MapFrameDrawStateFile>, Vec<String>) {
    if context.is_null() {
        return (
            None,
            vec!["null D3D11 context for map-frame draw-state capture".to_string()],
        );
    }
    let mut blockers = Vec::new();
    let viewports = unsafe { capture_map_frame_viewports(context) };
    if viewports.is_empty() {
        blockers.push("map-frame draw-state capture found no RS viewports".to_string());
    }
    let scissor_rects = unsafe { capture_map_frame_scissor_rects(context) };
    let rasterizer = unsafe { capture_map_frame_rasterizer_state(context) };
    let blend = unsafe { capture_map_frame_blend_state(context) };
    let depth_stencil = unsafe { capture_map_frame_depth_stencil_state(context) };
    let (output_merger, mut output_merger_blockers) =
        unsafe { capture_map_frame_output_merger_state(context, output_dir, draw_index, profile) };
    blockers.append(&mut output_merger_blockers);
    (
        Some(MapFrameDrawStateFile {
            viewports,
            scissor_rects,
            rasterizer,
            blend,
            depth_stencil,
            output_merger,
        }),
        blockers,
    )
}

unsafe fn capture_map_frame_viewports(
    context: *mut ID3D11DeviceContext,
) -> Vec<MapFrameViewportFile> {
    let mut count: UINT = 16;
    let mut viewports: [D3D11_VIEWPORT; 16] = unsafe { mem::zeroed() };
    unsafe {
        (*context).RSGetViewports(&mut count, viewports.as_mut_ptr());
    }
    viewports
        .iter()
        .take(count.min(16) as usize)
        .map(|viewport| MapFrameViewportFile {
            top_left_x: viewport.TopLeftX,
            top_left_y: viewport.TopLeftY,
            width: viewport.Width,
            height: viewport.Height,
            min_depth: viewport.MinDepth,
            max_depth: viewport.MaxDepth,
        })
        .collect()
}

unsafe fn capture_map_frame_scissor_rects(
    context: *mut ID3D11DeviceContext,
) -> Vec<MapFrameScissorRectFile> {
    let mut count: UINT = 16;
    let mut rects: [D3D11_RECT; 16] = unsafe { mem::zeroed() };
    unsafe {
        (*context).RSGetScissorRects(&mut count, rects.as_mut_ptr());
    }
    rects
        .iter()
        .take(count.min(16) as usize)
        .map(|rect| MapFrameScissorRectFile {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        })
        .collect()
}

unsafe fn capture_map_frame_rasterizer_state(
    context: *mut ID3D11DeviceContext,
) -> MapFrameRasterizerStateFile {
    let mut state: *mut ID3D11RasterizerState = ptr::null_mut();
    unsafe {
        (*context).RSGetState(&mut state);
    }
    if state.is_null() {
        return MapFrameRasterizerStateFile {
            default_state: true,
            desc: None,
        };
    }
    let mut desc: D3D11_RASTERIZER_DESC = unsafe { mem::zeroed() };
    unsafe {
        (*state).GetDesc(&mut desc);
        release_unknown(state as *mut WinVoid);
    }
    MapFrameRasterizerStateFile {
        default_state: false,
        desc: Some(rasterizer_desc_file(&desc)),
    }
}

unsafe fn capture_map_frame_blend_state(
    context: *mut ID3D11DeviceContext,
) -> MapFrameBlendStateFile {
    let mut state: *mut ID3D11BlendState = ptr::null_mut();
    let mut blend_factor = [0.0f32; 4];
    let mut sample_mask: UINT = 0;
    unsafe {
        (*context).OMGetBlendState(&mut state, &mut blend_factor, &mut sample_mask);
    }
    if state.is_null() {
        return MapFrameBlendStateFile {
            default_state: true,
            blend_factor,
            sample_mask,
            desc: None,
        };
    }
    let mut desc: D3D11_BLEND_DESC = unsafe { mem::zeroed() };
    unsafe {
        (*state).GetDesc(&mut desc);
        release_unknown(state as *mut WinVoid);
    }
    MapFrameBlendStateFile {
        default_state: false,
        blend_factor,
        sample_mask,
        desc: Some(blend_desc_file(&desc)),
    }
}

unsafe fn capture_map_frame_depth_stencil_state(
    context: *mut ID3D11DeviceContext,
) -> MapFrameDepthStencilStateFile {
    let mut state: *mut ID3D11DepthStencilState = ptr::null_mut();
    let mut stencil_ref: UINT = 0;
    unsafe {
        (*context).OMGetDepthStencilState(&mut state, &mut stencil_ref);
    }
    if state.is_null() {
        return MapFrameDepthStencilStateFile {
            default_state: true,
            stencil_ref,
            desc: None,
        };
    }
    let mut desc: D3D11_DEPTH_STENCIL_DESC = unsafe { mem::zeroed() };
    unsafe {
        (*state).GetDesc(&mut desc);
        release_unknown(state as *mut WinVoid);
    }
    MapFrameDepthStencilStateFile {
        default_state: false,
        stencil_ref,
        desc: Some(depth_stencil_desc_file(&desc)),
    }
}

unsafe fn capture_map_frame_output_merger_state(
    context: *mut ID3D11DeviceContext,
    output_dir: &Path,
    draw_index: u32,
    profile: MapFrameCaptureProfile,
) -> (MapFrameOutputMergerStateFile, Vec<String>) {
    let mut blockers = Vec::new();
    let (render_target_clears, depth_stencil_clears) = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .map(|runtime| {
            (
                runtime.om_render_target_clears.clone(),
                runtime.om_depth_stencil_clears.clone(),
            )
        })
        .unwrap_or_default();
    let mut render_targets: [*mut ID3D11RenderTargetView; 8] = [ptr::null_mut(); 8];
    let mut depth_stencil: *mut ID3D11DepthStencilView = ptr::null_mut();
    unsafe {
        (*context).OMGetRenderTargets(8, render_targets.as_mut_ptr(), &mut depth_stencil);
    }
    let mut captured_render_targets = Vec::new();
    for (slot, rtv) in render_targets.iter().copied().enumerate() {
        if rtv.is_null() {
            continue;
        }
        match unsafe {
            capture_om_render_target_texture(
                context,
                rtv,
                slot as u32,
                &render_target_clears,
                output_dir,
                draw_index,
                profile,
            )
        } {
            Ok(texture) => captured_render_targets.push(texture),
            Err(error) => blockers.push(format!("OM render target slot {slot}: {error}")),
        }
        unsafe {
            release_unknown(rtv as *mut WinVoid);
        }
    }
    let captured_depth_stencil = if depth_stencil.is_null() {
        None
    } else {
        let captured = match unsafe {
            capture_om_depth_stencil_texture(
                context,
                depth_stencil,
                &depth_stencil_clears,
                output_dir,
                draw_index,
                profile,
            )
        } {
            Ok(texture) => Some(texture),
            Err(error) => {
                blockers.push(format!("OM depth-stencil view: {error}"));
                None
            }
        };
        unsafe {
            release_unknown(depth_stencil as *mut WinVoid);
        }
        captured
    };
    (
        MapFrameOutputMergerStateFile {
            render_targets: captured_render_targets,
            depth_stencil: captured_depth_stencil,
        },
        blockers,
    )
}

unsafe fn capture_map_frame_post_pass_output_merger_state(
    context: *mut ID3D11DeviceContext,
    output_dir: &Path,
    draw_index: u32,
    profile: MapFrameCaptureProfile,
) -> (Option<MapFrameOutputMergerStateFile>, Vec<String>) {
    let mut blockers = Vec::new();
    if context.is_null() {
        blockers.push("post-pass output-merger capture got a null D3D11 context".to_string());
        return (None, blockers);
    }
    let (render_target_clears, depth_stencil_clears) = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .map(|runtime| {
            (
                runtime.om_render_target_clears.clone(),
                runtime.om_depth_stencil_clears.clone(),
            )
        })
        .unwrap_or_default();
    let mut render_targets: [*mut ID3D11RenderTargetView; 8] = [ptr::null_mut(); 8];
    let mut depth_stencil: *mut ID3D11DepthStencilView = ptr::null_mut();
    unsafe {
        (*context).OMGetRenderTargets(8, render_targets.as_mut_ptr(), &mut depth_stencil);
    }
    let mut captured_render_targets = Vec::new();
    for (slot, rtv) in render_targets.iter().copied().enumerate() {
        if rtv.is_null() {
            continue;
        }
        match unsafe {
            capture_om_render_target_texture_with_post_pass_payload(
                context,
                rtv,
                slot as u32,
                &render_target_clears,
                output_dir,
                draw_index,
                profile,
            )
        } {
            Ok(texture) => captured_render_targets.push(texture),
            Err(error) => blockers.push(format!("post-pass OM render target slot {slot}: {error}")),
        }
        unsafe {
            release_unknown(rtv as *mut WinVoid);
        }
    }
    if captured_render_targets.is_empty() {
        blockers.push(format!(
            "post-pass output-merger capture found no bound {} render target",
            profile.capture_moment
        ));
    }
    let captured_depth_stencil = if depth_stencil.is_null() {
        None
    } else {
        let captured = match unsafe {
            capture_om_depth_stencil_texture_desc(depth_stencil, &depth_stencil_clears)
        } {
            Ok(texture) => Some(texture),
            Err(error) => {
                blockers.push(format!("post-pass OM depth-stencil view: {error}"));
                None
            }
        };
        unsafe {
            release_unknown(depth_stencil as *mut WinVoid);
        }
        captured
    };
    let state = MapFrameOutputMergerStateFile {
        render_targets: captured_render_targets,
        depth_stencil: captured_depth_stencil,
    };
    (Some(state), blockers)
}

unsafe fn capture_om_render_target_texture(
    context: *mut ID3D11DeviceContext,
    view: *mut ID3D11RenderTargetView,
    slot: u32,
    clears: &BTreeMap<usize, MapFrameRenderTargetClearFile>,
    output_dir: &Path,
    draw_index: u32,
    profile: MapFrameCaptureProfile,
) -> Result<MapFrameOutputMergerTextureFile, String> {
    if context.is_null() {
        return Err("null D3D11 context for pre-draw render target readback".to_string());
    }
    if view.is_null() {
        return Err("null OM render target view".to_string());
    }
    let mut resource: *mut ID3D11Resource = ptr::null_mut();
    unsafe {
        (*view).GetResource(&mut resource);
    }
    let last_render_target_clear = clears
        .get(&(view as usize))
        .or_else(|| clears.get(&(resource as usize)))
        .cloned();
    let texture = match unsafe { texture2d_from_resource(resource) } {
        Ok(texture) => texture,
        Err(error) => {
            unsafe {
                release_unknown(resource as *mut WinVoid);
            }
            return Err(error);
        }
    };
    let result = (|| -> Result<MapFrameOutputMergerTextureFile, String> {
        let mut desc_native: D3D11_TEXTURE2D_DESC = unsafe { mem::zeroed() };
        unsafe {
            (*texture).GetDesc(&mut desc_native);
        }
        if desc_native.SampleDesc.Count > 1 {
            return Err(format!(
                "pre-draw OM render target slot {slot} is {}x MSAA; exact replay requires unresolved per-sample payload, not a resolved readback",
                desc_native.SampleDesc.Count
            ));
        }
        let view_desc = unsafe { render_target_view_desc_file(view) }?;
        let pre_draw_payload = unsafe {
            capture_om_texture_payload_exact(
                context,
                texture,
                &desc_native,
                slot,
                output_dir,
                map_frame_pre_draw_render_target_payload_file(draw_index, slot, profile),
                "pre_draw_om_render_target_resolved_readback",
            )
        }?;
        Ok(MapFrameOutputMergerTextureFile {
            slot: Some(slot),
            view_pointer: format!("0x{:X}", view as usize),
            resource_pointer: format!("0x{:X}", resource as usize),
            desc: texture2d_desc_file(&texture_desc_from_native(&desc_native)),
            view_desc: Some(view_desc),
            pre_draw_payload: Some(pre_draw_payload),
            post_pass_payload: None,
            last_render_target_clear,
            last_depth_stencil_clear: None,
        })
    })();
    unsafe {
        release_unknown(texture as *mut WinVoid);
        release_unknown(resource as *mut WinVoid);
    }
    result
}

unsafe fn capture_om_render_target_texture_with_post_pass_payload(
    context: *mut ID3D11DeviceContext,
    view: *mut ID3D11RenderTargetView,
    slot: u32,
    clears: &BTreeMap<usize, MapFrameRenderTargetClearFile>,
    output_dir: &Path,
    draw_index: u32,
    profile: MapFrameCaptureProfile,
) -> Result<MapFrameOutputMergerTextureFile, String> {
    if context.is_null() {
        return Err("null D3D11 context for post-pass render target readback".to_string());
    }
    if view.is_null() {
        return Err("null OM render target view".to_string());
    }
    let mut resource: *mut ID3D11Resource = ptr::null_mut();
    unsafe {
        (*view).GetResource(&mut resource);
    }
    if resource.is_null() {
        return Err("OM render target view has no resource".to_string());
    }
    let last_render_target_clear = clears
        .get(&(view as usize))
        .or_else(|| clears.get(&(resource as usize)))
        .cloned();
    let texture = match unsafe { texture2d_from_resource(resource) } {
        Ok(texture) => texture,
        Err(error) => {
            unsafe {
                release_unknown(resource as *mut WinVoid);
            }
            return Err(error);
        }
    };
    let result = (|| -> Result<MapFrameOutputMergerTextureFile, String> {
        let mut desc_native: D3D11_TEXTURE2D_DESC = unsafe { mem::zeroed() };
        unsafe {
            (*texture).GetDesc(&mut desc_native);
        }
        let view_desc = unsafe { render_target_view_desc_file(view) }?;
        let post_pass_payload = unsafe {
            capture_om_render_target_payload_exact(
                context,
                texture,
                &desc_native,
                slot,
                output_dir,
                draw_index,
                profile,
            )
        }?;
        Ok(MapFrameOutputMergerTextureFile {
            slot: Some(slot),
            view_pointer: format!("0x{:X}", view as usize),
            resource_pointer: format!("0x{:X}", resource as usize),
            desc: texture2d_desc_file(&texture_desc_from_native(&desc_native)),
            view_desc: Some(view_desc),
            pre_draw_payload: None,
            post_pass_payload: Some(post_pass_payload),
            last_render_target_clear,
            last_depth_stencil_clear: None,
        })
    })();
    unsafe {
        release_unknown(texture as *mut WinVoid);
        release_unknown(resource as *mut WinVoid);
    }
    result
}

unsafe fn capture_om_depth_stencil_texture(
    context: *mut ID3D11DeviceContext,
    view: *mut ID3D11DepthStencilView,
    clears: &BTreeMap<usize, MapFrameDepthStencilClearFile>,
    output_dir: &Path,
    draw_index: u32,
    profile: MapFrameCaptureProfile,
) -> Result<MapFrameOutputMergerTextureFile, String> {
    if context.is_null() {
        return Err("null D3D11 context for pre-draw depth-stencil readback".to_string());
    }
    if view.is_null() {
        return Err("null OM depth-stencil view".to_string());
    }
    let mut resource: *mut ID3D11Resource = ptr::null_mut();
    unsafe {
        (*view).GetResource(&mut resource);
    }
    let last_depth_stencil_clear = clears
        .get(&(view as usize))
        .or_else(|| clears.get(&(resource as usize)))
        .cloned();
    let texture = match unsafe { texture2d_from_resource(resource) } {
        Ok(texture) => texture,
        Err(error) => {
            unsafe {
                release_unknown(resource as *mut WinVoid);
            }
            return Err(error);
        }
    };
    let result = (|| -> Result<MapFrameOutputMergerTextureFile, String> {
        let mut desc_native: D3D11_TEXTURE2D_DESC = unsafe { mem::zeroed() };
        unsafe {
            (*texture).GetDesc(&mut desc_native);
        }
        if desc_native.SampleDesc.Count > 1 {
            return Err(format!(
                "pre-draw OM depth-stencil target is {}x MSAA; exact replay requires unresolved per-sample payload, not a resolved readback",
                desc_native.SampleDesc.Count
            ));
        }
        let view_desc = unsafe { depth_stencil_view_desc_file(view) }?;
        let pre_draw_payload = unsafe {
            capture_om_texture_payload_exact(
                context,
                texture,
                &desc_native,
                0,
                output_dir,
                map_frame_pre_draw_depth_stencil_payload_file(draw_index, profile),
                "pre_draw_om_depth_stencil_resolved_readback",
            )
        }?;
        Ok(MapFrameOutputMergerTextureFile {
            slot: None,
            view_pointer: format!("0x{:X}", view as usize),
            resource_pointer: format!("0x{:X}", resource as usize),
            desc: texture2d_desc_file(&texture_desc_from_native(&desc_native)),
            view_desc: Some(view_desc),
            pre_draw_payload: Some(pre_draw_payload),
            post_pass_payload: None,
            last_render_target_clear: None,
            last_depth_stencil_clear,
        })
    })();
    unsafe {
        release_unknown(texture as *mut WinVoid);
        release_unknown(resource as *mut WinVoid);
    }
    result
}

unsafe fn capture_om_depth_stencil_texture_desc(
    view: *mut ID3D11DepthStencilView,
    clears: &BTreeMap<usize, MapFrameDepthStencilClearFile>,
) -> Result<MapFrameOutputMergerTextureFile, String> {
    if view.is_null() {
        return Err("null OM depth-stencil view".to_string());
    }
    let mut resource: *mut ID3D11Resource = ptr::null_mut();
    unsafe {
        (*view).GetResource(&mut resource);
    }
    let last_depth_stencil_clear = clears
        .get(&(view as usize))
        .or_else(|| clears.get(&(resource as usize)))
        .cloned();
    let view_desc = unsafe { depth_stencil_view_desc_file(view) }?;
    unsafe {
        capture_om_texture_from_resource(
            view as *mut WinVoid,
            None,
            resource,
            Some(view_desc),
            None,
            last_depth_stencil_clear,
        )
    }
}

unsafe fn capture_om_texture_from_resource(
    view: *mut WinVoid,
    slot: Option<u32>,
    resource: *mut ID3D11Resource,
    view_desc: Option<MapFrameOutputMergerViewDescFile>,
    last_render_target_clear: Option<MapFrameRenderTargetClearFile>,
    last_depth_stencil_clear: Option<MapFrameDepthStencilClearFile>,
) -> Result<MapFrameOutputMergerTextureFile, String> {
    if view.is_null() {
        return Err("null OM view".to_string());
    }
    if resource.is_null() {
        return Err("OM view has no resource".to_string());
    }
    let texture = match unsafe { texture2d_from_resource(resource) } {
        Ok(texture) => texture,
        Err(error) => {
            unsafe {
                release_unknown(resource as *mut WinVoid);
            }
            return Err(error);
        }
    };
    let mut desc_native: D3D11_TEXTURE2D_DESC = unsafe { mem::zeroed() };
    unsafe {
        (*texture).GetDesc(&mut desc_native);
    }
    let out = MapFrameOutputMergerTextureFile {
        slot,
        view_pointer: format!("0x{:X}", view as usize),
        resource_pointer: format!("0x{:X}", resource as usize),
        desc: texture2d_desc_file(&texture_desc_from_native(&desc_native)),
        view_desc,
        pre_draw_payload: None,
        post_pass_payload: None,
        last_render_target_clear,
        last_depth_stencil_clear,
    };
    unsafe {
        release_unknown(texture as *mut WinVoid);
        release_unknown(resource as *mut WinVoid);
    }
    Ok(out)
}

unsafe fn texture2d_from_resource(
    resource: *mut ID3D11Resource,
) -> Result<*mut ID3D11Texture2D, String> {
    let mut texture_void: *mut WinVoid = ptr::null_mut();
    let hr = unsafe {
        (*(resource as *mut IUnknown)).QueryInterface(&IID_ID3D11Texture2D, &mut texture_void)
    };
    if FAILED(hr) || texture_void.is_null() {
        return Err(format!(
            "OM resource is not ID3D11Texture2D: HRESULT 0x{:08X}",
            hr as u32
        ));
    }
    Ok(texture_void as *mut ID3D11Texture2D)
}

unsafe fn render_target_view_desc_file(
    view: *mut ID3D11RenderTargetView,
) -> Result<MapFrameOutputMergerViewDescFile, String> {
    if view.is_null() {
        return Err("null render target view descriptor source".to_string());
    }
    let mut desc: D3D11_RENDER_TARGET_VIEW_DESC = unsafe { mem::zeroed() };
    unsafe {
        (*view).GetDesc(&mut desc);
    }
    match desc.ViewDimension {
        D3D11_RTV_DIMENSION_TEXTURE2D => {
            let view = unsafe { *desc.u.Texture2D() };
            Ok(MapFrameOutputMergerViewDescFile {
                format: desc.Format as u32,
                view_dimension: desc.ViewDimension,
                mip_slice: view.MipSlice,
                first_array_slice: 0,
                array_size: 1,
                flags: 0,
            })
        }
        D3D11_RTV_DIMENSION_TEXTURE2DARRAY => {
            let view = unsafe { *desc.u.Texture2DArray() };
            Ok(MapFrameOutputMergerViewDescFile {
                format: desc.Format as u32,
                view_dimension: desc.ViewDimension,
                mip_slice: view.MipSlice,
                first_array_slice: view.FirstArraySlice,
                array_size: view.ArraySize,
                flags: 0,
            })
        }
        D3D11_RTV_DIMENSION_TEXTURE2DMS => Ok(MapFrameOutputMergerViewDescFile {
            format: desc.Format as u32,
            view_dimension: desc.ViewDimension,
            mip_slice: 0,
            first_array_slice: 0,
            array_size: 1,
            flags: 0,
        }),
        D3D11_RTV_DIMENSION_TEXTURE2DMSARRAY => {
            let view = unsafe { *desc.u.Texture2DMSArray() };
            Ok(MapFrameOutputMergerViewDescFile {
                format: desc.Format as u32,
                view_dimension: desc.ViewDimension,
                mip_slice: 0,
                first_array_slice: view.FirstArraySlice,
                array_size: view.ArraySize,
                flags: 0,
            })
        }
        other => Err(format!(
            "unsupported OM render target view dimension {other}"
        )),
    }
}

unsafe fn depth_stencil_view_desc_file(
    view: *mut ID3D11DepthStencilView,
) -> Result<MapFrameOutputMergerViewDescFile, String> {
    if view.is_null() {
        return Err("null depth-stencil view descriptor source".to_string());
    }
    let mut desc: D3D11_DEPTH_STENCIL_VIEW_DESC = unsafe { mem::zeroed() };
    unsafe {
        (*view).GetDesc(&mut desc);
    }
    match desc.ViewDimension {
        D3D11_DSV_DIMENSION_TEXTURE2D => {
            let view = unsafe { *desc.u.Texture2D() };
            Ok(MapFrameOutputMergerViewDescFile {
                format: desc.Format as u32,
                view_dimension: desc.ViewDimension,
                mip_slice: view.MipSlice,
                first_array_slice: 0,
                array_size: 1,
                flags: desc.Flags,
            })
        }
        D3D11_DSV_DIMENSION_TEXTURE2DARRAY => {
            let view = unsafe { *desc.u.Texture2DArray() };
            Ok(MapFrameOutputMergerViewDescFile {
                format: desc.Format as u32,
                view_dimension: desc.ViewDimension,
                mip_slice: view.MipSlice,
                first_array_slice: view.FirstArraySlice,
                array_size: view.ArraySize,
                flags: desc.Flags,
            })
        }
        D3D11_DSV_DIMENSION_TEXTURE2DMS => Ok(MapFrameOutputMergerViewDescFile {
            format: desc.Format as u32,
            view_dimension: desc.ViewDimension,
            mip_slice: 0,
            first_array_slice: 0,
            array_size: 1,
            flags: desc.Flags,
        }),
        D3D11_DSV_DIMENSION_TEXTURE2DMSARRAY => {
            let view = unsafe { *desc.u.Texture2DMSArray() };
            Ok(MapFrameOutputMergerViewDescFile {
                format: desc.Format as u32,
                view_dimension: desc.ViewDimension,
                mip_slice: 0,
                first_array_slice: view.FirstArraySlice,
                array_size: view.ArraySize,
                flags: desc.Flags,
            })
        }
        other => Err(format!(
            "unsupported OM depth-stencil view dimension {other}"
        )),
    }
}

fn rasterizer_desc_file(desc: &D3D11_RASTERIZER_DESC) -> MapFrameRasterizerDescFile {
    MapFrameRasterizerDescFile {
        fill_mode: desc.FillMode as u32,
        cull_mode: desc.CullMode as u32,
        front_counter_clockwise: d3d_bool(desc.FrontCounterClockwise),
        depth_bias: desc.DepthBias,
        depth_bias_clamp: desc.DepthBiasClamp,
        slope_scaled_depth_bias: desc.SlopeScaledDepthBias,
        depth_clip_enable: d3d_bool(desc.DepthClipEnable),
        scissor_enable: d3d_bool(desc.ScissorEnable),
        multisample_enable: d3d_bool(desc.MultisampleEnable),
        antialiased_line_enable: d3d_bool(desc.AntialiasedLineEnable),
    }
}

fn blend_desc_file(desc: &D3D11_BLEND_DESC) -> MapFrameBlendDescFile {
    MapFrameBlendDescFile {
        alpha_to_coverage_enable: d3d_bool(desc.AlphaToCoverageEnable),
        independent_blend_enable: d3d_bool(desc.IndependentBlendEnable),
        render_targets: desc
            .RenderTarget
            .iter()
            .map(render_target_blend_desc_file)
            .collect(),
    }
}

fn render_target_blend_desc_file(
    desc: &D3D11_RENDER_TARGET_BLEND_DESC,
) -> MapFrameRenderTargetBlendDescFile {
    MapFrameRenderTargetBlendDescFile {
        blend_enable: d3d_bool(desc.BlendEnable),
        src_blend: desc.SrcBlend as u32,
        dest_blend: desc.DestBlend as u32,
        blend_op: desc.BlendOp as u32,
        src_blend_alpha: desc.SrcBlendAlpha as u32,
        dest_blend_alpha: desc.DestBlendAlpha as u32,
        blend_op_alpha: desc.BlendOpAlpha as u32,
        render_target_write_mask: desc.RenderTargetWriteMask,
    }
}

fn depth_stencil_desc_file(desc: &D3D11_DEPTH_STENCIL_DESC) -> MapFrameDepthStencilDescFile {
    MapFrameDepthStencilDescFile {
        depth_enable: d3d_bool(desc.DepthEnable),
        depth_write_mask: desc.DepthWriteMask as u32,
        depth_func: desc.DepthFunc as u32,
        stencil_enable: d3d_bool(desc.StencilEnable),
        stencil_read_mask: desc.StencilReadMask,
        stencil_write_mask: desc.StencilWriteMask,
        front_face: depth_stencil_op_desc_file(&desc.FrontFace),
        back_face: depth_stencil_op_desc_file(&desc.BackFace),
    }
}

fn depth_stencil_op_desc_file(desc: &D3D11_DEPTH_STENCILOP_DESC) -> MapFrameDepthStencilOpDescFile {
    MapFrameDepthStencilOpDescFile {
        stencil_fail_op: desc.StencilFailOp as u32,
        stencil_depth_fail_op: desc.StencilDepthFailOp as u32,
        stencil_pass_op: desc.StencilPassOp as u32,
        stencil_func: desc.StencilFunc as u32,
    }
}

fn texture2d_desc_file(desc: &Texture2DDesc) -> MapFrameTexture2DDescFile {
    MapFrameTexture2DDescFile {
        width: desc.width,
        height: desc.height,
        mip_levels: desc.mip_levels,
        array_size: desc.array_size,
        format: desc.format,
        sample_count: desc.sample_count,
        sample_quality: desc.sample_quality,
        usage: desc.usage,
        bind_flags: desc.bind_flags,
        cpu_access_flags: desc.cpu_access_flags,
        misc_flags: desc.misc_flags,
    }
}

fn d3d_bool(value: i32) -> bool {
    value != 0
}

unsafe fn capture_map_frame_constant_buffers(
    context: *mut ID3D11DeviceContext,
    draw_index: u32,
    profile: MapFrameCaptureProfile,
) -> (Vec<MapFrameBufferBindingFile>, Vec<String>) {
    let mut out = Vec::new();
    let mut blockers = Vec::new();
    unsafe {
        capture_map_frame_constant_buffer_stage(
            context,
            "VS",
            draw_index,
            profile,
            &mut out,
            &mut blockers,
        );
        capture_map_frame_constant_buffer_stage(
            context,
            "PS",
            draw_index,
            profile,
            &mut out,
            &mut blockers,
        );
    }
    (out, blockers)
}

unsafe fn capture_map_frame_constant_buffer_stage(
    context: *mut ID3D11DeviceContext,
    stage: &'static str,
    draw_index: u32,
    profile: MapFrameCaptureProfile,
    out: &mut Vec<MapFrameBufferBindingFile>,
    blockers: &mut Vec<String>,
) {
    for slot in 0..MAP_FRAME_BINDING_SLOTS {
        let mut buffer: *mut ID3D11Buffer = ptr::null_mut();
        unsafe {
            if stage == "VS" {
                (*context).VSGetConstantBuffers(slot, 1, &mut buffer);
            } else {
                (*context).PSGetConstantBuffers(slot, 1, &mut buffer);
            }
        }
        if buffer.is_null() {
            continue;
        }
        let result = unsafe {
            capture_map_frame_constant_buffer_slot(
                context, stage, slot, draw_index, profile, buffer,
            )
        };
        unsafe {
            release_unknown(buffer as *mut WinVoid);
        }
        match result {
            Ok(binding) => out.push(binding),
            Err(error) => blockers.push(error),
        }
    }
}

unsafe fn capture_map_frame_constant_buffer_slot(
    context: *mut ID3D11DeviceContext,
    stage: &'static str,
    slot: u32,
    draw_index: u32,
    profile: MapFrameCaptureProfile,
    buffer: *mut ID3D11Buffer,
) -> Result<MapFrameBufferBindingFile, String> {
    let output_dir = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .map(|runtime| runtime.output_dir.clone())
        .ok_or_else(|| "terrain capture runtime not initialized".to_string())?;
    let (source, bytes) = unsafe { readback_d3d11_buffer_bytes(context, buffer)? };
    if source.bind_flags & D3D11_BIND_CONSTANT_BUFFER == 0 {
        return Err(format!(
            "{stage} constant buffer slot {slot} missing D3D11_BIND_CONSTANT_BUFFER: BindFlags {}",
            source.bind_flags
        ));
    }
    let file = format!(
        "{}{draw_index:03}_{}_constant_buffer_slot{}.bin",
        profile.file_prefix,
        stage.to_ascii_lowercase(),
        slot
    );
    write_map_frame_bytes(&output_dir, &file, &bytes)?;
    Ok(MapFrameBufferBindingFile {
        stage,
        slot,
        declared_name: map_frame_constant_buffer_declared_name(stage, slot, profile),
        file: Some(file),
        byte_len: bytes.len(),
        fnv1a64: Some(fnv1a64_bytes(&bytes)),
        stride: None,
        offset: None,
        dxgi_format: None,
        bind_flags: source.bind_flags,
        usage: source.usage,
        cpu_access_flags: source.cpu_access_flags,
    })
}

fn map_frame_constant_buffer_declared_name(
    stage: &str,
    slot: u32,
    profile: MapFrameCaptureProfile,
) -> Option<String> {
    let reflected = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .and_then(|runtime| match stage {
            "PS" => runtime
                .state
                .current_pixel_shader_constant_buffer_name(slot),
            "VS" => runtime
                .state
                .current_vertex_shader_constant_buffer_name(slot),
            _ => None,
        });
    if reflected.is_some() {
        return reflected;
    }
    if profile.capture_moment == TERRAIN_CAPTURE_MOMENT && matches!(stage, "VS" | "PS") && slot == 0
    {
        Some(TERRAIN_SCENE_CB0_SYMBOL.to_string())
    } else {
        None
    }
}

fn add_file_evidence_if_exists(
    files: &mut BTreeMap<String, MapFrameFileEvidence>,
    output_dir: &Path,
    file: &str,
    provenance: &str,
) {
    let path = output_dir.join(file);
    let Ok(bytes) = fs::read(&path) else {
        return;
    };
    files.insert(
        file.to_string(),
        MapFrameFileEvidence {
            file: file.to_string(),
            byte_len: bytes.len(),
            fnv1a64: fnv1a64_bytes(&bytes),
            provenance: provenance.to_string(),
        },
    );
}

fn add_required_file_evidence(
    files: &mut BTreeMap<String, MapFrameFileEvidence>,
    output_dir: &Path,
    file: &str,
    provenance: &str,
    blockers: &mut Vec<String>,
) {
    let path = output_dir.join(file);
    match fs::read(&path) {
        Ok(bytes) => {
            files.insert(
                file.to_string(),
                MapFrameFileEvidence {
                    file: file.to_string(),
                    byte_len: bytes.len(),
                    fnv1a64: fnv1a64_bytes(&bytes),
                    provenance: provenance.to_string(),
                },
            );
        }
        Err(error) => blockers.push(format!(
            "required map-frame evidence file {file} is missing or unreadable: {error}"
        )),
    }
}

fn add_map_frame_shader_resource_file_evidence(
    files: &mut BTreeMap<String, MapFrameFileEvidence>,
    output_dir: &Path,
    draws: &[MapFrameDrawFile],
    blockers: &mut Vec<String>,
) {
    for draw in draws {
        for resource in &draw.shader_resources {
            let Some(file) = resource.file.as_deref() else {
                continue;
            };
            if files.contains_key(file) {
                continue;
            }
            let provenance = resource
                .provenance
                .unwrap_or("map_frame_shader_resource_binding_file");
            add_required_file_evidence(files, output_dir, file, provenance, blockers);
        }
    }
}

fn add_map_frame_post_pass_output_merger_file_evidence(
    files: &mut BTreeMap<String, MapFrameFileEvidence>,
    output_dir: &Path,
    output_merger: Option<&MapFrameOutputMergerStateFile>,
    profile: MapFrameCaptureProfile,
    blockers: &mut Vec<String>,
) {
    let Some(output_merger) = output_merger else {
        blockers.push(format!(
            "required post-pass output-merger {} render target payload is missing",
            profile.capture_moment
        ));
        return;
    };
    if output_merger.render_targets.is_empty() {
        blockers.push(format!(
            "required post-pass output-merger {} render target payload has no render targets",
            profile.capture_moment
        ));
        return;
    }
    for target in &output_merger.render_targets {
        let Some(payload) = target.post_pass_payload.as_ref() else {
            blockers.push(format!(
                "post-pass output-merger render target slot {:?} has no payload file",
                target.slot
            ));
            continue;
        };
        add_required_file_evidence(
            files,
            output_dir,
            &payload.file,
            payload.provenance,
            blockers,
        );
    }
}

fn add_map_frame_pre_draw_output_file_evidence(
    files: &mut BTreeMap<String, MapFrameFileEvidence>,
    output_dir: &Path,
    draws: &[MapFrameDrawFile],
    blockers: &mut Vec<String>,
) {
    for draw in draws {
        let Some(draw_state) = draw.draw_state.as_ref() else {
            blockers.push(format!(
                "draw {} has no draw-state for pre-draw output-merger payload evidence",
                draw.draw_index
            ));
            continue;
        };
        for target in &draw_state.output_merger.render_targets {
            let Some(payload) = target.pre_draw_payload.as_ref() else {
                blockers.push(format!(
                    "draw {} pre-draw output-merger render target slot {:?} has no payload file",
                    draw.draw_index, target.slot
                ));
                continue;
            };
            if !files.contains_key(&payload.file) {
                add_required_file_evidence(
                    files,
                    output_dir,
                    &payload.file,
                    payload.provenance,
                    blockers,
                );
            }
        }
        if let Some(depth) = draw_state.output_merger.depth_stencil.as_ref() {
            let Some(payload) = depth.pre_draw_payload.as_ref() else {
                blockers.push(format!(
                    "draw {} pre-draw output-merger depth-stencil has no payload file",
                    draw.draw_index
                ));
                continue;
            };
            if !files.contains_key(&payload.file) {
                add_required_file_evidence(
                    files,
                    output_dir,
                    &payload.file,
                    payload.provenance,
                    blockers,
                );
            }
        }
    }
}

fn add_map_frame_post_draw_output_file_evidence(
    files: &mut BTreeMap<String, MapFrameFileEvidence>,
    output_dir: &Path,
    draws: &[MapFrameDrawFile],
    blockers: &mut Vec<String>,
) {
    for draw in draws {
        let Some(output_merger) = draw.post_draw_output_merger.as_ref() else {
            continue;
        };
        for target in &output_merger.render_targets {
            let Some(payload) = target.post_pass_payload.as_ref() else {
                blockers.push(format!(
                    "draw {} post-draw output-merger render target slot {:?} has no payload file",
                    draw.draw_index, target.slot
                ));
                continue;
            };
            if files.contains_key(&payload.file) {
                continue;
            }
            add_required_file_evidence(
                files,
                output_dir,
                &payload.file,
                payload.provenance,
                blockers,
            );
        }
    }
}

fn copy_required_vanilla_sources(
    files: &mut BTreeMap<String, MapFrameFileEvidence>,
    output_dir: &Path,
    sources: &[&str],
    blockers: &mut Vec<String>,
) {
    let Ok(game_dir) = std::env::current_dir() else {
        blockers.push(
            "could not resolve the game working directory for vanilla source evidence".to_string(),
        );
        return;
    };
    for source in sources {
        let source_path = source
            .split('/')
            .fold(game_dir.clone(), |path, part| path.join(part));
        let copied_file = format!("vanilla/{}", source);
        match fs::read(&source_path) {
            Ok(bytes) => {
                let dest_path = copied_file
                    .split('/')
                    .fold(output_dir.to_path_buf(), |path, part| path.join(part));
                if let Some(parent) = dest_path.parent() {
                    if let Err(error) = fs::create_dir_all(parent) {
                        blockers.push(format!(
                            "could not create vanilla source evidence directory for {source}: {error}"
                        ));
                        continue;
                    }
                }
                if let Err(error) = fs::write(&dest_path, &bytes) {
                    blockers.push(format!(
                        "could not copy vanilla source evidence {source}: {error}"
                    ));
                    continue;
                }
                files.insert(
                    source.to_string(),
                    MapFrameFileEvidence {
                        file: copied_file,
                        byte_len: bytes.len(),
                        fnv1a64: fnv1a64_bytes(&bytes),
                        provenance: "copied_from_hoi4_working_directory".to_string(),
                    },
                );
            }
            Err(error) => blockers.push(format!(
                "required vanilla source {source} is missing or unreadable from the game working directory: {error}"
            )),
        }
    }
}

fn validate_against_control(desc: &Texture2DDesc, slot: u32) -> Result<(), String> {
    let Some(control) = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .and_then(|runtime| runtime.control.clone())
    else {
        return Ok(());
    };
    if control.expected_width == 0
        || control.expected_height == 0
        || control.allowed_slots.is_empty()
    {
        return Ok(());
    }
    if desc.width != control.expected_width || desc.height != control.expected_height {
        return Err(format!(
            "candidate SRV dimensions {}x{} do not match shader-cache MAP_SIZE {}x{}",
            desc.width, desc.height, control.expected_width, control.expected_height
        ));
    }
    if !control.allowed_slots.contains(&slot) {
        return Err(format!(
            "candidate SRV slot {slot} is not in shader-cache TerrainIDMap slots {:?}",
            control.allowed_slots
        ));
    }
    Ok(())
}

fn read_control(module_dir: &Path) -> Option<TerrainCaptureControl> {
    let path = module_dir.join(CONTROL_FILE);
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn requested_map_frame_capture_profile() -> Result<MapFrameCaptureProfile, String> {
    let runtime = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .ok_or_else(|| "terrain capture runtime not initialized".to_string())?;
    requested_map_frame_capture_profile_for_runtime(&runtime)
}

fn requested_map_frame_capture_profile_for_runtime(
    runtime: &CaptureRuntime,
) -> Result<MapFrameCaptureProfile, String> {
    let capture_moment = runtime
        .capture_moment_sequence
        .get(runtime.active_capture_index)
        .map(String::as_str)
        .unwrap_or(TERRAIN_CAPTURE_MOMENT);
    map_frame_capture_profile(capture_moment)
}

fn capture_moment_sequence_from_control(control: Option<&TerrainCaptureControl>) -> Vec<String> {
    let mut moments = Vec::new();
    if let Some(control) = control {
        moments.extend(
            control
                .capture_moments
                .iter()
                .filter(|moment| !moment.trim().is_empty())
                .cloned(),
        );
        if moments.is_empty() {
            if let Some(moment) = control.capture_moment.as_ref() {
                if !moment.trim().is_empty() {
                    moments.push(moment.clone());
                }
            }
        }
    }
    if moments.is_empty() {
        moments.push(TERRAIN_CAPTURE_MOMENT.to_string());
    }
    moments
}

fn map_frame_capture_profile(capture_moment: &str) -> Result<MapFrameCaptureProfile, String> {
    match capture_moment {
        TERRAIN_CAPTURE_MOMENT => Ok(TERRAIN_MAP_FRAME_CAPTURE_PROFILE),
        WATER_CAPTURE_MOMENT => Ok(WATER_MAP_FRAME_CAPTURE_PROFILE),
        RIVERS_CAPTURE_MOMENT => Ok(RIVERS_MAP_FRAME_CAPTURE_PROFILE),
        BORDERS_CAPTURE_MOMENT => Ok(BORDERS_MAP_FRAME_CAPTURE_PROFILE),
        TREES_CAPTURE_MOMENT => Ok(TREES_MAP_FRAME_CAPTURE_PROFILE),
        other => Err(format!("unsupported map-frame capture moment {other}")),
    }
}

fn active_map_frame_pixel_shader_pointer_for_profile(
    state: &TerrainCaptureState,
    profile: MapFrameCaptureProfile,
) -> Option<usize> {
    match profile.capture_moment {
        TERRAIN_CAPTURE_MOMENT => state
            .pdxmap_pixel_shader()
            .map(|shader| shader.pixel_shader),
        WATER_CAPTURE_MOMENT => state
            .pdxwater_pixel_shader()
            .map(|shader| shader.pixel_shader),
        RIVERS_CAPTURE_MOMENT => state.river_pixel_shader().map(|shader| shader.pixel_shader),
        BORDERS_CAPTURE_MOMENT => state
            .border_pixel_shader()
            .map(|shader| shader.pixel_shader),
        TREES_CAPTURE_MOMENT => state.tree_pixel_shader().map(|shader| shader.pixel_shader),
        _ => None,
    }
}

fn pending_capture_message() -> String {
    let Some(runtime) = RUNTIME.get().and_then(|runtime| runtime.lock().ok()) else {
        return "no active pdxmap terrain shader/SRV binding observed before timeout".to_string();
    };
    let Ok(profile) = requested_map_frame_capture_profile_for_runtime(&runtime) else {
        return "unsupported map-frame capture moment before timeout".to_string();
    };
    let mut pending = Vec::new();
    if profile.requires_terrain_runtime_files {
        pending.extend(
            CAPTURE_TARGETS
                .iter()
                .filter(|target| !runtime.captured_symbols.contains(target.symbol))
                .map(|target| target.symbol),
        );
        if !runtime
            .captured_symbols
            .contains(TERRAIN_VERTEX_BUFFER_SYMBOL)
        {
            pending.push(TERRAIN_VERTEX_BUFFER_SYMBOL);
        }
        if !runtime
            .captured_symbols
            .contains(TERRAIN_INPUT_LAYOUT_SYMBOL)
        {
            pending.push(TERRAIN_INPUT_LAYOUT_SYMBOL);
        }
    }
    if !runtime
        .completed_map_frame_moments
        .contains(profile.capture_moment)
    {
        pending.push(MAP_FRAME_BUNDLE_SYMBOL);
    }
    if pending.is_empty() {
        format!(
            "{} capture reached completion but timeout fired before shutdown",
            profile.capture_moment
        )
    } else if pending.len() == CAPTURE_TARGETS.len() + 3
        && profile.capture_moment == TERRAIN_CAPTURE_MOMENT
    {
        "no active pdxmap terrain shader/SRV binding observed before timeout".to_string()
    } else if profile.capture_moment == WATER_CAPTURE_MOMENT
        && pending.len() == 1
        && pending[0] == MAP_FRAME_BUNDLE_SYMBOL
    {
        "no active pdxwater shader draw observed before timeout".to_string()
    } else if profile.capture_moment == RIVERS_CAPTURE_MOMENT
        && pending.len() == 1
        && pending[0] == MAP_FRAME_BUNDLE_SYMBOL
    {
        "no active river shader draw observed before timeout".to_string()
    } else if profile.capture_moment == BORDERS_CAPTURE_MOMENT
        && pending.len() == 1
        && pending[0] == MAP_FRAME_BUNDLE_SYMBOL
    {
        "no active border shader draw observed before timeout".to_string()
    } else if profile.capture_moment == TREES_CAPTURE_MOMENT
        && pending.len() == 1
        && pending[0] == MAP_FRAME_BUNDLE_SYMBOL
    {
        "no active tree shader draw observed before timeout".to_string()
    } else {
        format!(
            "pending runtime {} capture targets not observed before timeout: {}",
            profile.capture_moment,
            pending.join(",")
        )
    }
}

struct ProbeDevice {
    device: *mut ID3D11Device,
    context: *mut ID3D11DeviceContext,
}

unsafe fn create_probe_device() -> Result<ProbeDevice, String> {
    let feature_levels = [D3D_FEATURE_LEVEL_11_0];
    let mut device: *mut ID3D11Device = ptr::null_mut();
    let mut context: *mut ID3D11DeviceContext = ptr::null_mut();
    let mut actual_feature_level = D3D_FEATURE_LEVEL_11_0;
    let hr = unsafe {
        D3D11CreateDevice(
            ptr::null_mut(),
            D3D_DRIVER_TYPE_NULL,
            ptr::null_mut(),
            0,
            feature_levels.as_ptr(),
            feature_levels.len() as UINT,
            D3D11_SDK_VERSION,
            &mut device,
            &mut actual_feature_level,
            &mut context,
        )
    };
    if FAILED(hr) || device.is_null() || context.is_null() {
        return Err(format!(
            "D3D11CreateDevice probe failed: HRESULT 0x{:08X}",
            hr as u32
        ));
    }
    Ok(ProbeDevice { device, context })
}

unsafe fn vtable_slot(object: *mut WinVoid, index: usize) -> Result<usize, String> {
    if object.is_null() {
        return Err("null COM object for vtable lookup".to_string());
    }
    let vtable = unsafe { *(object as *mut *mut *mut WinVoid) };
    if vtable.is_null() {
        return Err("null COM vtable".to_string());
    }
    let slot = unsafe { *vtable.add(index) };
    if slot.is_null() {
        Err(format!("null COM vtable slot {index}"))
    } else {
        Ok(slot as usize)
    }
}

unsafe fn reflect_shader_resource_slot_inner(bytecode: &[u8], resource: &str) -> Option<u32> {
    let library = unsafe { LoadLibraryW(wide_null("d3dcompiler_47.dll").as_ptr()) };
    if library.is_null() {
        return None;
    }
    let proc_name = CString::new("D3DReflect").ok()?;
    let proc = unsafe { GetProcAddress(library, proc_name.as_ptr().cast()) }?;
    let d3d_reflect: D3DReflectFn = unsafe { mem::transmute(proc) };
    let mut reflector: *mut WinVoid = ptr::null_mut();
    let hr = unsafe {
        d3d_reflect(
            bytecode.as_ptr().cast(),
            bytecode.len(),
            &IID_ID3D11ShaderReflection,
            &mut reflector,
        )
    };
    if FAILED(hr) || reflector.is_null() {
        unsafe {
            FreeLibrary(library as HMODULE);
        }
        return None;
    }
    let reflector = reflector as *mut ID3D11ShaderReflection;
    let name = CString::new(format!("{resource}_Texture")).ok()?;
    let mut desc: D3D11_SHADER_INPUT_BIND_DESC = unsafe { mem::zeroed() };
    let binding_hr = unsafe { (*reflector).GetResourceBindingDescByName(name.as_ptr(), &mut desc) };
    unsafe {
        (*(reflector as *mut IUnknown)).Release();
        FreeLibrary(library as HMODULE);
    }
    if SUCCEEDED(binding_hr) && desc.Type == D3D_SIT_TEXTURE {
        Some(desc.BindPoint)
    } else {
        None
    }
}

unsafe fn reflect_shader_texture_resource_slots_inner(
    bytecode: &[u8],
) -> Option<BTreeMap<String, u32>> {
    let (library, reflector) = unsafe { reflect_shader(bytecode)? };
    let mut shader_desc: D3D11_SHADER_DESC = unsafe { mem::zeroed() };
    let desc_hr = unsafe { (*reflector).GetDesc(&mut shader_desc) };
    if FAILED(desc_hr) {
        unsafe {
            (*(reflector as *mut IUnknown)).Release();
            FreeLibrary(library as HMODULE);
        }
        return None;
    }
    let mut out = BTreeMap::new();
    for index in 0..shader_desc.BoundResources {
        let mut desc: D3D11_SHADER_INPUT_BIND_DESC = unsafe { mem::zeroed() };
        let hr = unsafe { (*reflector).GetResourceBindingDesc(index, &mut desc) };
        if FAILED(hr) || desc.Name.is_null() {
            unsafe {
                (*(reflector as *mut IUnknown)).Release();
                FreeLibrary(library as HMODULE);
            }
            return None;
        }
        if desc.Type != D3D_SIT_TEXTURE {
            continue;
        }
        let name = unsafe { CStr::from_ptr(desc.Name) }
            .to_string_lossy()
            .into_owned();
        out.insert(normalize_shader_resource_name(&name), desc.BindPoint);
    }
    unsafe {
        (*(reflector as *mut IUnknown)).Release();
        FreeLibrary(library as HMODULE);
    }
    Some(out)
}

unsafe fn reflect_shader_constant_buffer_slots_inner(
    bytecode: &[u8],
) -> Option<BTreeMap<String, u32>> {
    let (library, reflector) = unsafe { reflect_shader(bytecode)? };
    let mut shader_desc: D3D11_SHADER_DESC = unsafe { mem::zeroed() };
    let desc_hr = unsafe { (*reflector).GetDesc(&mut shader_desc) };
    if FAILED(desc_hr) {
        unsafe {
            (*(reflector as *mut IUnknown)).Release();
            FreeLibrary(library as HMODULE);
        }
        return None;
    }
    let mut out = BTreeMap::new();
    for index in 0..shader_desc.BoundResources {
        let mut desc: D3D11_SHADER_INPUT_BIND_DESC = unsafe { mem::zeroed() };
        let hr = unsafe { (*reflector).GetResourceBindingDesc(index, &mut desc) };
        if FAILED(hr) || desc.Name.is_null() {
            unsafe {
                (*(reflector as *mut IUnknown)).Release();
                FreeLibrary(library as HMODULE);
            }
            return None;
        }
        if desc.Type != D3D_SIT_CBUFFER {
            continue;
        }
        let name = unsafe { CStr::from_ptr(desc.Name) }
            .to_string_lossy()
            .into_owned();
        out.insert(name, desc.BindPoint);
    }
    unsafe {
        (*(reflector as *mut IUnknown)).Release();
        FreeLibrary(library as HMODULE);
    }
    Some(out)
}

unsafe fn reflect_terrain_vertex_shader_metadata_inner(
    bytecode: &[u8],
) -> Option<TerrainVertexShaderMetadata> {
    let (library, reflector) = unsafe { reflect_shader(bytecode)? };
    let mut shader_desc: D3D11_SHADER_DESC = unsafe { mem::zeroed() };
    let desc_hr = unsafe { (*reflector).GetDesc(&mut shader_desc) };
    if FAILED(desc_hr) {
        unsafe {
            (*(reflector as *mut IUnknown)).Release();
            FreeLibrary(library as HMODULE);
        }
        return None;
    }
    let mut input_parameters = Vec::new();
    for index in 0..shader_desc.InputParameters {
        let mut desc: D3D11_SIGNATURE_PARAMETER_DESC = unsafe { mem::zeroed() };
        let hr = unsafe { (*reflector).GetInputParameterDesc(index, &mut desc) };
        if FAILED(hr) || desc.SemanticName.is_null() {
            unsafe {
                (*(reflector as *mut IUnknown)).Release();
                FreeLibrary(library as HMODULE);
            }
            return None;
        }
        let semantic_name = unsafe { CStr::from_ptr(desc.SemanticName) }
            .to_string_lossy()
            .into_owned();
        input_parameters.push(ShaderInputParameter {
            semantic_name,
            semantic_index: desc.SemanticIndex,
            register: desc.Register,
            component_type: desc.ComponentType as u32,
            mask: desc.Mask,
            read_write_mask: desc.ReadWriteMask,
        });
    }
    let mut constant_buffers = BTreeMap::new();
    for index in 0..shader_desc.BoundResources {
        let mut desc: D3D11_SHADER_INPUT_BIND_DESC = unsafe { mem::zeroed() };
        let hr = unsafe { (*reflector).GetResourceBindingDesc(index, &mut desc) };
        if FAILED(hr) || desc.Name.is_null() {
            unsafe {
                (*(reflector as *mut IUnknown)).Release();
                FreeLibrary(library as HMODULE);
            }
            return None;
        }
        if desc.Type != D3D_SIT_CBUFFER {
            continue;
        }
        let name = unsafe { CStr::from_ptr(desc.Name) }
            .to_string_lossy()
            .into_owned();
        constant_buffers.insert(name, desc.BindPoint);
    }
    unsafe {
        (*(reflector as *mut IUnknown)).Release();
        FreeLibrary(library as HMODULE);
    }
    Some(TerrainVertexShaderMetadata {
        input_parameters,
        constant_buffers,
        byte_len: bytecode.len(),
        fnv1a64: fnv1a64_bytes(bytecode),
    })
}

unsafe fn reflect_shader(bytecode: &[u8]) -> Option<(HMODULE, *mut ID3D11ShaderReflection)> {
    let library = unsafe { LoadLibraryW(wide_null("d3dcompiler_47.dll").as_ptr()) };
    if library.is_null() {
        return None;
    }
    let proc_name = CString::new("D3DReflect").ok()?;
    let proc = unsafe { GetProcAddress(library, proc_name.as_ptr().cast()) };
    let Some(proc) = proc else {
        unsafe {
            FreeLibrary(library as HMODULE);
        }
        return None;
    };
    let d3d_reflect: D3DReflectFn = unsafe { mem::transmute(proc) };
    let mut reflector: *mut WinVoid = ptr::null_mut();
    let hr = unsafe {
        d3d_reflect(
            bytecode.as_ptr().cast(),
            bytecode.len(),
            &IID_ID3D11ShaderReflection,
            &mut reflector,
        )
    };
    if FAILED(hr) || reflector.is_null() {
        unsafe {
            FreeLibrary(library as HMODULE);
        }
        return None;
    }
    Some((library as HMODULE, reflector as *mut ID3D11ShaderReflection))
}

fn texture_desc_from_native(desc: &D3D11_TEXTURE2D_DESC) -> Texture2DDesc {
    Texture2DDesc {
        width: desc.Width,
        height: desc.Height,
        mip_levels: desc.MipLevels,
        array_size: desc.ArraySize,
        format: desc.Format as u32,
        sample_count: desc.SampleDesc.Count,
        sample_quality: desc.SampleDesc.Quality,
        usage: desc.Usage as u32,
        bind_flags: desc.BindFlags,
        cpu_access_flags: desc.CPUAccessFlags,
        misc_flags: desc.MiscFlags,
    }
}

fn buffer_desc_from_native(desc: &D3D11_BUFFER_DESC) -> BufferDesc {
    BufferDesc {
        byte_width: desc.ByteWidth,
        usage: desc.Usage as u32,
        bind_flags: desc.BindFlags,
        cpu_access_flags: desc.CPUAccessFlags,
        misc_flags: desc.MiscFlags,
        structure_byte_stride: desc.StructureByteStride,
    }
}

fn buffer_desc_to_native(desc: &BufferDesc) -> D3D11_BUFFER_DESC {
    D3D11_BUFFER_DESC {
        ByteWidth: desc.byte_width,
        Usage: desc.usage,
        BindFlags: desc.bind_flags,
        CPUAccessFlags: desc.cpu_access_flags,
        MiscFlags: desc.misc_flags,
        StructureByteStride: desc.structure_byte_stride,
    }
}

unsafe fn release_unknown(object: *mut WinVoid) {
    if !object.is_null() {
        unsafe {
            (*(object as *mut IUnknown)).Release();
        }
    }
}

fn write_event(output_dir: &Path, line: &str) {
    let path = output_dir.join(CAPTURE_EVENTS_FILE);
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let _ = writeln!(file, "{line}");
}

fn fnv1a64_bytes(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn json_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            ch if ch.is_control() => escaped.push(' '),
            ch => escaped.push(ch),
        }
    }
    escaped
}

fn wide_null(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain([0]).collect()
}

#[link(name = "kernel32")]
extern "system" {
    fn GetTickCount64() -> u64;
    fn Sleep(milliseconds: u32);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_capture_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "hoi4_terrain_capture_{}_{}",
            std::process::id(),
            name
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create temp capture dir");
        path
    }

    #[test]
    fn blocked_session_manifest_records_terrain_moment_without_fake_bundle() {
        let root = temp_capture_dir("blocked_session_manifest");
        write_blocked_map_capture_session(
            &root,
            vec!["no active pdxmap pixel shader for map-frame capture".to_string()],
            "unit_test_blocked",
            TERRAIN_MAP_FRAME_CAPTURE_PROFILE,
        )
        .expect("write blocked session manifest");

        assert!(!root.join(MAP_FRAME_BUNDLE_FILE).exists());
        let session: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join(MAP_CAPTURE_SESSION_FILE)).unwrap())
                .expect("parse session");
        assert_eq!(session["schema"], MAP_CAPTURE_SESSION_SCHEMA);
        assert_eq!(session["required_first_moment"], TERRAIN_CAPTURE_MOMENT);
        assert_eq!(session["moments"][0]["name"], TERRAIN_CAPTURE_MOMENT);
        assert_eq!(session["moments"][0]["status"], "blocked");
        assert_eq!(
            session["moments"][0]["blockers"][0],
            "no active pdxmap pixel shader for map-frame capture"
        );
        assert_eq!(
            session["deferred_moments"][0]["required_after"],
            TERRAIN_CAPTURE_MOMENT
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn blocked_session_manifest_records_water_moment_after_terrain_moment() {
        let root = temp_capture_dir("blocked_water_session_manifest");
        write_blocked_map_capture_session(
            &root,
            vec!["no active pdxwater pixel shader for water-pass map-frame capture".to_string()],
            "unit_test_water_blocked",
            WATER_MAP_FRAME_CAPTURE_PROFILE,
        )
        .expect("write blocked water session manifest");

        let session: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join(MAP_CAPTURE_SESSION_FILE)).unwrap())
                .expect("parse session");
        assert_eq!(session["schema"], MAP_CAPTURE_SESSION_SCHEMA);
        assert_eq!(session["required_first_moment"], TERRAIN_CAPTURE_MOMENT);
        assert_eq!(session["moments"][0]["name"], TERRAIN_CAPTURE_MOMENT);
        assert_eq!(
            session["moments"][0]["status"],
            "captured_pending_renderer_validation"
        );
        assert_eq!(session["moments"][1]["name"], WATER_CAPTURE_MOMENT);
        assert_eq!(
            session["moments"][1]["bundle_file"],
            WATER_MAP_FRAME_BUNDLE_FILE
        );
        assert_eq!(session["moments"][1]["status"], "blocked");
        assert_eq!(
            session["moments"][1]["blockers"][0],
            "no active pdxwater pixel shader for water-pass map-frame capture"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn map_frame_capture_profiles_cover_only_grounded_map_passes() {
        assert_eq!(
            map_frame_capture_profile(RIVERS_CAPTURE_MOMENT)
                .expect("rivers profile")
                .bundle_file,
            RIVERS_MAP_FRAME_BUNDLE_FILE
        );
        assert_eq!(
            map_frame_capture_profile(BORDERS_CAPTURE_MOMENT)
                .expect("borders profile")
                .pixel_shader_file_prefix,
            "border_pixel_shader"
        );
        assert_eq!(
            map_frame_capture_profile(TREES_CAPTURE_MOMENT)
                .expect("trees profile")
                .shader_sources[0],
            "gfx/FX/tree.shader"
        );
        assert!(map_frame_capture_profile("cities_pass").is_err());
        assert!(map_frame_capture_profile("overlays_pass").is_err());
    }

    fn runtime_for_capture_moments(root: &Path, moments: &[&str]) -> CaptureRuntime {
        let control = TerrainCaptureControl {
            schema: "battle-plan-terrain-capture-control-v1".to_string(),
            capture_moment: moments.first().map(|moment| (*moment).to_string()),
            capture_moments: moments.iter().map(|moment| (*moment).to_string()).collect(),
            expected_width: 0,
            expected_height: 0,
            allowed_slots: Vec::new(),
            paired_resource_slots: Vec::new(),
            shader_cache_signatures: Vec::new(),
            provenance: "unit_test".to_string(),
        };
        CaptureRuntime {
            state: TerrainCaptureState::default(),
            output_dir: root.to_path_buf(),
            capture_moment_sequence: capture_moment_sequence_from_control(Some(&control)),
            control: Some(control),
            active_capture_index: 0,
            captured_symbols: BTreeSet::new(),
            completed_map_frame_moments: BTreeSet::new(),
            map_frame_draws: Vec::new(),
            map_frame_blockers: Vec::new(),
            map_frame_rejections: BTreeSet::new(),
            latest_post_draw_output_merger: None,
            latest_post_draw_output_blockers: Vec::new(),
            om_render_target_clears: BTreeMap::new(),
            om_depth_stencil_clears: BTreeMap::new(),
            om_clear_sequence: 0,
            terrain_id_srv_slot_observed: None,
            last_error: None,
        }
    }

    #[test]
    fn session_capture_advances_after_terrain_runtime_inputs_complete() {
        let root = temp_capture_dir("session_capture_advance");
        let mut runtime =
            runtime_for_capture_moments(&root, &[TERRAIN_CAPTURE_MOMENT, WATER_CAPTURE_MOMENT]);
        assert_eq!(
            requested_map_frame_capture_profile_for_runtime(&runtime)
                .expect("terrain profile")
                .capture_moment,
            TERRAIN_CAPTURE_MOMENT
        );

        runtime
            .completed_map_frame_moments
            .insert(TERRAIN_CAPTURE_MOMENT.to_string());
        assert!(!refresh_session_capture_progress(&mut runtime));
        assert_eq!(
            requested_map_frame_capture_profile_for_runtime(&runtime)
                .expect("terrain still active")
                .capture_moment,
            TERRAIN_CAPTURE_MOMENT
        );

        for target in CAPTURE_TARGETS {
            runtime.captured_symbols.insert(target.symbol.to_string());
        }
        runtime
            .captured_symbols
            .insert(TERRAIN_VERTEX_BUFFER_SYMBOL.to_string());
        runtime
            .captured_symbols
            .insert(TERRAIN_INPUT_LAYOUT_SYMBOL.to_string());
        runtime
            .map_frame_blockers
            .push("unit test stale terrain blocker".to_string());

        assert!(!refresh_session_capture_progress(&mut runtime));
        assert_eq!(
            requested_map_frame_capture_profile_for_runtime(&runtime)
                .expect("water profile")
                .capture_moment,
            WATER_CAPTURE_MOMENT
        );
        assert!(runtime.map_frame_blockers.is_empty());

        runtime
            .completed_map_frame_moments
            .insert(WATER_CAPTURE_MOMENT.to_string());
        assert!(refresh_session_capture_progress(&mut runtime));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn map_frame_payload_files_are_capture_moment_scoped() {
        assert_eq!(
            map_frame_srv_payload_file(
                7,
                3,
                Some("TerrainIDMap"),
                TERRAIN_MAP_FRAME_CAPTURE_PROFILE
            ),
            "map_frame_draw007_ps_srv_slot003_TerrainIDMap.d3d11"
        );
        assert_eq!(
            map_frame_srv_payload_file(
                7,
                3,
                Some("HeightTexture"),
                WATER_MAP_FRAME_CAPTURE_PROFILE
            ),
            "water_frame_draw007_ps_srv_slot003_HeightTexture.d3d11"
        );
        assert_eq!(
            map_frame_srv_payload_file(7, 3, Some("RiverData"), RIVERS_MAP_FRAME_CAPTURE_PROFILE),
            "rivers_frame_draw007_ps_srv_slot003_RiverData.d3d11"
        );
        assert_eq!(
            map_frame_srv_payload_file(
                7,
                0,
                Some("BorderDiffuse"),
                BORDERS_MAP_FRAME_CAPTURE_PROFILE
            ),
            "borders_frame_draw007_ps_srv_slot000_BorderDiffuse.d3d11"
        );
        assert_eq!(
            map_frame_srv_payload_file(
                7,
                14,
                Some("TreeMaskTexture"),
                TREES_MAP_FRAME_CAPTURE_PROFILE
            ),
            "trees_frame_draw007_ps_srv_slot014_TreeMaskTexture.d3d11"
        );
        assert_eq!(
            map_frame_post_pass_render_target_payload_file(7, 0, WATER_MAP_FRAME_CAPTURE_PROFILE),
            "water_frame_draw007_map_frame_post_pass_render_target_slot000.d3d11"
        );
        assert_eq!(
            map_frame_pre_draw_render_target_payload_file(7, 0, WATER_MAP_FRAME_CAPTURE_PROFILE),
            "water_frame_draw007_map_frame_pre_draw_render_target_slot000.d3d11"
        );
        assert_eq!(
            map_frame_pre_draw_depth_stencil_payload_file(7, WATER_MAP_FRAME_CAPTURE_PROFILE),
            "water_frame_draw007_map_frame_pre_draw_depth_stencil.d3d11"
        );
    }
}
